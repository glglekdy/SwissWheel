//! The progress card above the tray (spec 2.4): one row per running or
//! queued file, glass like the wheel, click-through, never focused, and
//! hidden when nothing runs.

use swisswheel_core::summary::{more_files, progress_title, progress_value};
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{CompositionRoundedRectangleGeometry, Compositor, SpriteVisual};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D11::ID3D11Device;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dwm::{DWMWA_USE_HOSTBACKDROPBRUSH, DwmSetWindowAttribute};
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::WinRT::Composition::{ICompositorDesktopInterop, ICompositorInterop};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, HSTRING, Interface, Result, w};
use windows_numerics::Vector2;

use crate::jobs::ProgressRow;
use crate::wheel::render::{
    Palette, create_d3d_device, light_theme, palette, transparency_enabled,
};

/// Card layout in DIPs.
const WIDTH: f32 = 300.0;
const HEADER: f32 = 32.0;
const ROW: f32 = 42.0;
const MORE: f32 = 22.0;
const PAD: f32 = 14.0;
const BOTTOM: f32 = 8.0;
const CORNER: f32 = 10.0;
const BAR: f32 = 4.0;
/// Gap between the card and the edge of the work area.
const MARGIN: f32 = 12.0;
/// Rows listed before the rest collapse into "외 N개".
const MAX_ROWS: usize = 5;

pub struct ProgressWindow {
    hwnd: HWND,
    glass_supported: bool,
    surface: Option<Surface>,
    scale: f32,
    visible: bool,
}

impl ProgressWindow {
    /// `owner` is the app's host window, as for the wheel.
    pub fn new(owner: HWND) -> Result<ProgressWindow> {
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let class = w!("SwissWheelProgress");
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(wnd_proc),
                hInstance: instance.into(),
                lpszClassName: class,
                ..Default::default()
            });
            // LAYERED + TRANSPARENT: mouse input goes to whatever is beneath.
            let hwnd = CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP
                    | WS_EX_TOPMOST
                    | WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE
                    | WS_EX_LAYERED
                    | WS_EX_TRANSPARENT,
                class,
                w!("SwissWheel progress"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                Some(owner),
                None,
                Some(instance.into()),
                None,
            )?;
            SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA)?;
            let enable = BOOL::from(true);
            let glass_supported = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_HOSTBACKDROPBRUSH,
                std::ptr::from_ref(&enable).cast(),
                size_of::<BOOL>() as u32,
            )
            .is_ok();
            Ok(ProgressWindow {
                hwnd,
                glass_supported,
                surface: None,
                scale: 1.0,
                visible: false,
            })
        }
    }

    /// Shows `rows`, or hides the card when there are none.
    pub fn update(&mut self, rows: &[ProgressRow]) {
        if rows.is_empty() {
            self.hide();
            return;
        }
        if !self.visible {
            self.place();
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            }
            self.visible = true;
        }
        let (scale, glass) = (self.scale, self.glass_supported);
        if let Some(surface) = &mut self.surface
            && let Err(e) = surface.draw(rows, scale, glass)
        {
            log_error!("진행 창 그리기 실패: {e}");
            self.surface = None; // rebuilt on the next show
        }
    }

    /// Hides the card and frees its GPU resources (it is idle most of the time).
    pub fn hide(&mut self) {
        if self.visible {
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
            self.visible = false;
        }
        self.surface = None;
    }

    /// Bottom-right of the primary monitor's work area (where the tray is by
    /// default), at that monitor's DPI.
    fn place(&mut self) {
        unsafe {
            let monitor = MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY);
            let (mut dpi_x, mut dpi_y) = (96, 96);
            let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
            self.scale = dpi_x as f32 / 96.0;
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let _ = GetMonitorInfoW(monitor, &mut info);
            let wa = info.rcWork;
            let width = (WIDTH * self.scale).ceil() as i32;
            let height = (card_height(MAX_ROWS, true) * self.scale).ceil() as i32;
            let margin = (MARGIN * self.scale).round() as i32;
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                wa.right - margin - width,
                wa.bottom - margin - height,
                width,
                height,
                SWP_NOACTIVATE,
            );
            if self.surface.is_none() {
                match Surface::new(self.hwnd, width as u32, height as u32) {
                    Ok(s) => self.surface = Some(s),
                    Err(e) => log_error!("진행 창을 만들 수 없습니다: {e}"),
                }
            }
        }
    }
}

/// The card's height in DIPs for `rows` rows, plus the "외 N개" line.
fn card_height(rows: usize, more: bool) -> f32 {
    HEADER + rows as f32 * ROW + if more { MORE } else { 0.0 } + BOTTOM
}

/// A Direct2D swap chain shown through Windows.UI.Composition over a
/// host-backdrop blur clipped to the card, like the wheel's.
struct Surface {
    _device: ID3D11Device,
    ctx: ID2D1DeviceContext,
    swap: IDXGISwapChain1,
    _compositor: Compositor,
    _target: DesktopWindowTarget,
    backdrop: SpriteVisual,
    clip: CompositionRoundedRectangleGeometry,
    _content: SpriteVisual,
    dwrite: IDWriteFactory,
    size: (u32, u32),
}

impl Surface {
    fn new(hwnd: HWND, width: u32, height: u32) -> Result<Surface> {
        unsafe {
            let device = create_d3d_device()?;
            let dxgi_device: IDXGIDevice = device.cast()?;
            let factory: ID2D1Factory1 =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let ctx = factory
                .CreateDevice(&dxgi_device)?
                .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            ctx.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let dxgi_factory: IDXGIFactory2 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: width,
                Height: height,
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
            let buffer: IDXGISurface = swap.GetBuffer(0)?;
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
            ctx.SetTarget(&ctx.CreateBitmapFromDxgiSurface(&buffer, Some(&props))?);

            let compositor = Compositor::new()?;
            let target = compositor
                .cast::<ICompositorDesktopInterop>()?
                .CreateDesktopWindowTarget(hwnd, true)?;
            let full = Vector2 {
                X: width as f32,
                Y: height as f32,
            };
            let root = compositor.CreateContainerVisual()?;
            root.SetSize(full)?;
            target.SetRoot(&root)?;
            let backdrop = compositor.CreateSpriteVisual()?;
            backdrop.SetSize(full)?;
            backdrop.SetBrush(&compositor.CreateHostBackdropBrush()?)?;
            let clip = compositor.CreateRoundedRectangleGeometry()?;
            backdrop.SetClip(&compositor.CreateGeometricClipWithGeometry(&clip)?)?;
            backdrop.SetIsVisible(false)?;
            let content = compositor.CreateSpriteVisual()?;
            content.SetSize(full)?;
            let surface = compositor
                .cast::<ICompositorInterop>()?
                .CreateCompositionSurfaceForSwapChain(&swap)?;
            content.SetBrush(&compositor.CreateSurfaceBrushWithSurface(&surface)?)?;
            let children = root.Children()?;
            children.InsertAtBottom(&backdrop)?;
            children.InsertAtTop(&content)?;
            Ok(Surface {
                _device: device,
                ctx,
                swap,
                _compositor: compositor,
                _target: target,
                backdrop,
                clip,
                _content: content,
                dwrite: DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?,
                size: (width, height),
            })
        }
    }

    fn draw(&mut self, rows: &[ProgressRow], scale: f32, glass_supported: bool) -> Result<()> {
        let glass = glass_supported && transparency_enabled();
        let colors = palette(light_theme(), glass, 1.0);
        let shown = rows.len().min(MAX_ROWS);
        let hidden = rows.len() - shown;
        let (w, h) = (self.size.0 as f32, self.size.1 as f32);
        // Anchored to the bottom so the card grows upward from the tray.
        let card = D2D_RECT_F {
            left: 0.0,
            top: h - card_height(shown, hidden > 0) * scale,
            right: w,
            bottom: h,
        };
        let corner = CORNER * scale;
        self.clip.SetOffset(Vector2 {
            X: card.left,
            Y: card.top,
        })?;
        self.clip.SetSize(Vector2 {
            X: card.right - card.left,
            Y: card.bottom - card.top,
        })?;
        self.clip.SetCornerRadius(Vector2 {
            X: corner,
            Y: corner,
        })?;
        self.backdrop.SetIsVisible(glass)?;

        let leading = self.text_format(12.5 * scale, DWRITE_FONT_WEIGHT_SEMI_BOLD, true)?;
        let small_leading = self.text_format(11.0 * scale, DWRITE_FONT_WEIGHT_SEMI_BOLD, true)?;
        let trailing = self.text_format(11.0 * scale, DWRITE_FONT_WEIGHT_NORMAL, false)?;
        let ctx = &self.ctx;
        unsafe {
            ctx.BeginDraw();
            ctx.Clear(Some(&D2D1_COLOR_F::default()));
            let brush = |color: &D2D1_COLOR_F| ctx.CreateSolidColorBrush(color, None);
            ctx.FillRoundedRectangle(&rounded(card, corner), &brush(&colors.disc)?);
            ctx.DrawRoundedRectangle(
                &rounded(inset(card, 0.5 * scale), corner),
                &brush(&colors.rim)?,
                scale,
                None,
            );

            let pad = PAD * scale;
            let text_box = |top: f32, height: f32| D2D_RECT_F {
                left: card.left + pad,
                top,
                right: card.right - pad,
                bottom: top + height,
            };
            let header = format!("변환 중 · {}개", rows.len());
            draw(
                ctx,
                &header,
                &small_leading,
                text_box(card.top + 4.0 * scale, HEADER * scale - 4.0 * scale),
                &brush(&colors.subtext)?,
            );

            let track = brush(&track_color(&colors))?;
            for (i, row) in rows.iter().take(shown).enumerate() {
                let top = card.top + (HEADER + i as f32 * ROW) * scale;
                let line = text_box(top, 20.0 * scale);
                draw(
                    ctx,
                    &progress_title(&row.file, &row.op),
                    &leading,
                    D2D_RECT_F {
                        right: line.right - 44.0 * scale, // room for "100%"
                        ..line
                    },
                    &brush(&colors.text)?,
                );
                draw(
                    ctx,
                    &progress_value(row.fraction),
                    &trailing,
                    line,
                    &brush(&colors.subtext)?,
                );
                let bar = D2D_RECT_F {
                    left: line.left,
                    top: top + 26.0 * scale,
                    right: line.right,
                    bottom: top + (26.0 + BAR) * scale,
                };
                let radius = BAR * scale / 2.0;
                ctx.FillRoundedRectangle(&rounded(bar, radius), &track);
                if let Some(p) = row.fraction {
                    let filled = D2D_RECT_F {
                        right: bar.left + (bar.right - bar.left) * p.clamp(0.0, 1.0),
                        ..bar
                    };
                    ctx.FillRoundedRectangle(&rounded(filled, radius), &brush(&colors.accent_top)?);
                }
            }
            if hidden > 0 {
                let top = card.top + (HEADER + shown as f32 * ROW) * scale;
                draw(
                    ctx,
                    &more_files(hidden),
                    &small_leading,
                    text_box(top, MORE * scale),
                    &brush(&colors.subtext)?,
                );
            }
            ctx.EndDraw(None, None)?;
            self.swap.Present(1, DXGI_PRESENT(0)).ok()?;
        }
        Ok(())
    }

    fn text_format(
        &self,
        size: f32,
        weight: DWRITE_FONT_WEIGHT,
        leading: bool,
    ) -> Result<IDWriteTextFormat> {
        unsafe {
            let format = self.dwrite.CreateTextFormat(
                w!("Segoe UI Variable Text"),
                None,
                weight,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                w!("ko-kr"),
            )?;
            format.SetTextAlignment(if leading {
                DWRITE_TEXT_ALIGNMENT_LEADING
            } else {
                DWRITE_TEXT_ALIGNMENT_TRAILING
            })?;
            format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            // Long file names end in "…" instead of running under the percentage.
            let sign = self.dwrite.CreateEllipsisTrimmingSign(&format)?;
            format.SetTrimming(
                &DWRITE_TRIMMING {
                    granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                    delimiter: 0,
                    delimiterCount: 0,
                },
                &sign,
            )?;
            Ok(format)
        }
    }
}

/// The empty part of a progress bar: the text color, faint.
fn track_color(colors: &Palette) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        a: colors.text.a * 0.15,
        ..colors.text
    }
}

fn rounded(rect: D2D_RECT_F, radius: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT {
        rect,
        radiusX: radius,
        radiusY: radius,
    }
}

fn inset(rect: D2D_RECT_F, d: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left: rect.left + d,
        top: rect.top + d,
        right: rect.right - d,
        bottom: rect.bottom - d,
    }
}

fn draw(
    ctx: &ID2D1DeviceContext,
    text: &str,
    format: &IDWriteTextFormat,
    rect: D2D_RECT_F,
    brush: &ID2D1SolidColorBrush,
) {
    unsafe {
        ctx.DrawText(
            &HSTRING::from(text),
            format,
            &rect,
            brush,
            D2D1_DRAW_TEXT_OPTIONS_CLIP,
            DWRITE_MEASURING_MODE_NATURAL,
        );
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_CLOSE => {
            // Hand taskkill's WM_CLOSE to the host window, like the wheel does.
            if let Ok(owner) = unsafe { GetWindow(hwnd, GW_OWNER) } {
                unsafe {
                    let _ = PostMessageW(Some(owner), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
