//! Draws the wheel with Direct2D into a DirectComposition swap chain, which
//! gives the window per-pixel transparency without layered-window blits.

use swisswheel_core::geometry::Hit;
use swisswheel_core::geometry::{CANCEL_RADIUS, OUTER_RADIUS, RING_CAPACITY, sector};
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::core::{HSTRING, Interface, Result, w};
use windows_numerics::Vector2;

use super::WheelView;
use super::window::HOVER_GROW;

const OPEN_SCALE_FROM: f32 = 0.92; // wheel scales from 92% to 100% while opening

struct Palette {
    background: D2D1_COLOR_F,
    sector: D2D1_COLOR_F,
    sector_alt: D2D1_COLOR_F,
    text: D2D1_COLOR_F,
    accent: D2D1_COLOR_F,
    accent_text: D2D1_COLOR_F,
}

fn rgb(hex: u32, alpha: f32) -> D2D1_COLOR_F {
    let c = |shift: u32| ((hex >> shift) & 0xFF) as f32 / 255.0;
    D2D1_COLOR_F {
        r: c(16),
        g: c(8),
        b: c(0),
        a: alpha,
    }
}

fn palette(light: bool, alpha: f32) -> Palette {
    let accent = rgb(0xE03A3E, alpha);
    let accent_text = rgb(0xFFFFFF, alpha);
    if light {
        Palette {
            background: rgb(0xFFFFFF, 0.97 * alpha),
            sector: rgb(0xF4F4F4, alpha),
            sector_alt: rgb(0xEAEAEA, alpha),
            text: rgb(0x1F1F1F, alpha),
            accent,
            accent_text,
        }
    } else {
        Palette {
            background: rgb(0x1E1E1E, 0.97 * alpha),
            sector: rgb(0x2C2C2C, alpha),
            sector_alt: rgb(0x343434, alpha),
            text: rgb(0xF2F2F2, alpha),
            accent,
            accent_text,
        }
    }
}

/// `AppsUseLightTheme` from the user's personalization settings (default light).
fn light_theme() -> bool {
    let mut value = 1u32;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(std::ptr::from_mut(&mut value).cast()),
            Some(&mut size),
        )
    };
    status.is_err() || value != 0
}

pub struct Renderer {
    _device: ID3D11Device,
    factory: ID2D1Factory1,
    ctx: ID2D1DeviceContext,
    swap: IDXGISwapChain1,
    dcomp: IDCompositionDevice,
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
    dwrite: IDWriteFactory,
    size: u32,
}

impl Renderer {
    pub fn new(hwnd: HWND, size: u32) -> Result<Renderer> {
        unsafe {
            let device = create_d3d_device()?;
            let dxgi_device: IDXGIDevice = device.cast()?;
            let factory: ID2D1Factory1 =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let ctx = factory
                .CreateDevice(&dxgi_device)?
                .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            ctx.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE); // ClearType needs an opaque target

            let dxgi_factory: IDXGIFactory2 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: size,
                Height: size,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                ..Default::default()
            };
            let swap = dxgi_factory.CreateSwapChainForComposition(&device, &desc, None)?;
            let surface: IDXGISurface = swap.GetBuffer(0)?;
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                ..Default::default()
            };
            let bitmap = ctx.CreateBitmapFromDxgiSurface(&surface, Some(&props))?;
            ctx.SetTarget(&bitmap);

            let dcomp: IDCompositionDevice = DCompositionCreateDevice(&dxgi_device)?;
            let target = dcomp.CreateTargetForHwnd(hwnd, true)?;
            let visual = dcomp.CreateVisual()?;
            visual.SetContent(&swap)?;
            target.SetRoot(&visual)?;
            dcomp.Commit()?;

            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            Ok(Renderer {
                _device: device,
                factory,
                ctx,
                swap,
                dcomp,
                _target: target,
                _visual: visual,
                dwrite,
                size,
            })
        }
    }

    pub fn size(&self) -> u32 {
        self.size
    }

    pub fn draw(&mut self, view: &WheelView, scale: f32) -> Result<()> {
        let eased = 1.0 - (1.0 - view.progress.clamp(0.0, 1.0)).powi(3);
        let grow = OPEN_SCALE_FROM + (1.0 - OPEN_SCALE_FROM) * eased;
        let colors = palette(light_theme(), eased);
        let c = self.size as f32 / 2.0;
        let n = view.items.len();
        unsafe {
            let ctx = &self.ctx;
            ctx.BeginDraw();
            ctx.Clear(Some(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }));
            let brush = |color: &D2D1_COLOR_F| ctx.CreateSolidColorBrush(color, None);

            let bg = brush(&colors.background)?;
            ctx.FillEllipse(&ellipse(c, OUTER_RADIUS * scale * grow), &bg);
            if view.loading {
                // Placeholder ring while the dragged files are still unknown.
                ctx.FillEllipse(
                    &ellipse(c, (OUTER_RADIUS - 4.0) * scale * grow),
                    &brush(&colors.sector)?,
                );
            }

            let label_size = if n > RING_CAPACITY { 11.0 } else { 13.0 } * scale;
            let label_format = self.text_format(label_size, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
            for (i, op) in view.items.iter().enumerate() {
                let s = sector(n, i, scale);
                let hovered = view.hovered == Hit::Item(i);
                let inner = s.inner_radius * grow;
                let outer =
                    (s.outer_radius + if hovered { HOVER_GROW * scale } else { 0.0 }) * grow;
                let path = self.annular_sector(c, inner, outer, s.start_deg, s.end_deg)?;
                let fill = if hovered {
                    &colors.accent
                } else if i % 2 == 0 {
                    &colors.sector
                } else {
                    &colors.sector_alt
                };
                ctx.FillGeometry(&path, &brush(fill)?, None);
                ctx.DrawGeometry(&path, &bg, 2.0 * scale, None); // gap between sectors

                let mid = (s.start_deg + s.end_deg) / 2.0;
                let (x, y) = polar(c, (inner + outer) / 2.0, mid);
                let text = if hovered {
                    &colors.accent_text
                } else {
                    &colors.text
                };
                self.draw_text(op.label(), &label_format, x, y, 76.0 * scale, &brush(text)?);
            }

            let accent = brush(&colors.accent)?;
            ctx.FillEllipse(&ellipse(c, (CANCEL_RADIUS - 2.0) * scale * grow), &accent);
            let title_format = self.text_format(13.0 * scale, DWRITE_FONT_WEIGHT_BOLD)?;
            self.draw_text(
                view.title,
                &title_format,
                c,
                c,
                84.0 * scale,
                &brush(&colors.accent_text)?,
            );

            ctx.EndDraw(None, None)?;
            self.swap.Present(1, DXGI_PRESENT(0)).ok()?;
            self.dcomp.Commit()?;
        }
        Ok(())
    }

    fn text_format(&self, size: f32, weight: DWRITE_FONT_WEIGHT) -> Result<IDWriteTextFormat> {
        unsafe {
            // Hangul falls back to the system's Korean UI font automatically.
            let format = self.dwrite.CreateTextFormat(
                w!("Segoe UI Variable Text"),
                None,
                weight,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                w!("ko-kr"),
            )?;
            format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            Ok(format)
        }
    }

    fn draw_text(
        &self,
        text: &str,
        format: &IDWriteTextFormat,
        x: f32,
        y: f32,
        width: f32,
        brush: &ID2D1SolidColorBrush,
    ) {
        let wide = HSTRING::from(text);
        let half_h = 14.0 * (width / 76.0);
        let rect = D2D_RECT_F {
            left: x - width / 2.0,
            top: y - half_h,
            right: x + width / 2.0,
            bottom: y + half_h,
        };
        unsafe {
            self.ctx.DrawText(
                &wide,
                format,
                &rect,
                brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    /// A ring segment between two radii and two angles (0° = 12 o'clock, clockwise).
    fn annular_sector(
        &self,
        c: f32,
        inner: f32,
        outer: f32,
        start: f32,
        end: f32,
    ) -> Result<ID2D1PathGeometry1> {
        unsafe {
            let path = self.factory.CreatePathGeometry()?;
            let sink = path.Open()?;
            sink.BeginFigure(point(polar(c, outer, start)), D2D1_FIGURE_BEGIN_FILLED);
            arc_to(&sink, c, outer, start, end, D2D1_SWEEP_DIRECTION_CLOCKWISE);
            sink.AddLine(point(polar(c, inner, end)));
            arc_to(
                &sink,
                c,
                inner,
                end,
                start,
                D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
            );
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            sink.Close()?;
            Ok(path)
        }
    }
}

/// Arcs from `from` to `to` degrees, split so no piece exceeds 180° (a full
/// circle has identical end points, which a single arc can't express).
fn arc_to(sink: &ID2D1GeometrySink, c: f32, r: f32, from: f32, to: f32, dir: D2D1_SWEEP_DIRECTION) {
    let pieces = ((to - from).abs() / 180.0).ceil().max(1.0) as usize;
    for k in 1..=pieces {
        let angle = from + (to - from) * k as f32 / pieces as f32;
        let segment = D2D1_ARC_SEGMENT {
            point: point(polar(c, r, angle)),
            size: D2D_SIZE_F {
                width: r,
                height: r,
            },
            rotationAngle: 0.0,
            sweepDirection: dir,
            arcSize: D2D1_ARC_SIZE_SMALL,
        };
        unsafe { sink.AddArc(&segment) };
    }
}

fn polar(c: f32, r: f32, deg: f32) -> (f32, f32) {
    let rad = deg.to_radians();
    (c + r * rad.sin(), c - r * rad.cos())
}

/// Direct2D points are `windows_numerics::Vector2` in windows 0.61.
fn point((x, y): (f32, f32)) -> Vector2 {
    Vector2 { X: x, Y: y }
}

fn ellipse(c: f32, r: f32) -> D2D1_ELLIPSE {
    D2D1_ELLIPSE {
        point: point((c, c)),
        radiusX: r,
        radiusY: r,
    }
}

fn create_d3d_device() -> Result<ID3D11Device> {
    let mut device = None;
    let mut attempt = |driver: D3D_DRIVER_TYPE| unsafe {
        D3D11CreateDevice(
            None,
            driver,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )
    };
    // Fall back to the software rasterizer (e.g. in a VM without a GPU).
    attempt(D3D_DRIVER_TYPE_HARDWARE).or_else(|_| attempt(D3D_DRIVER_TYPE_WARP))?;
    device.ok_or_else(|| windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL))
}
