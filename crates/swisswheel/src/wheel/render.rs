//! Draws the wheel. A Windows.UI.Composition visual tree gives the window
//! per-pixel transparency: at the bottom a host-backdrop (acrylic blur) disc,
//! on top a Direct2D swap chain with the tint, sectors, and labels.

use swisswheel_core::geometry::Hit;
use swisswheel_core::geometry::{
    CANCEL_RADIUS, OUTER_RADIUS, PANEL_HEADER, PANEL_PAD, PANEL_ROW, RING_CAPACITY, panel_rect,
    sector,
};
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{
    CompositionEllipseGeometry, CompositionRoundedRectangleGeometry, Compositor, SpriteVisual,
};
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::System::WinRT::Composition::{ICompositorDesktopInterop, ICompositorInterop};
use windows::core::{BOOL, HSTRING, Interface, PCWSTR, Result, w};
use windows_numerics::Vector2;

use super::glyph::glyph;
use super::window::HOVER_GROW;
use super::{PanelView, WheelView};

const OPEN_SCALE_FROM: f32 = 0.92; // wheel scales from 92% to 100% while opening

/// Gap between neighboring tiles and the rounding of their corners (DIPs).
const TILE_GAP: f32 = 4.0;
const TILE_CORNER: f32 = 6.0;
/// The hub (center circle) sits this far inside the cancel radius.
const HUB_INSET: f32 = 6.0;
/// Rounding of the option panel and of its rows (DIPs).
pub(super) const PANEL_CORNER: f32 = 10.0;
const ROW_CORNER: f32 = 8.0;

pub(crate) struct Palette {
    /// Glass: a light translucent tint over the blur. Solid: an opaque disc.
    pub(crate) disc: D2D1_COLOR_F,
    /// 1 px highlight just inside the disc edge, faint line just outside it.
    pub(crate) rim: D2D1_COLOR_F,
    pub(crate) outline: D2D1_COLOR_F,
    /// Tiles shade from top to bottom and cast a 1 px drop shadow.
    pub(crate) tile_top: D2D1_COLOR_F,
    pub(crate) tile_bottom: D2D1_COLOR_F,
    pub(crate) shadow: D2D1_COLOR_F,
    pub(crate) hub: D2D1_COLOR_F,
    pub(crate) hub_ring: D2D1_COLOR_F,
    pub(crate) text: D2D1_COLOR_F,
    pub(crate) subtext: D2D1_COLOR_F,
    pub(crate) accent_top: D2D1_COLOR_F,
    pub(crate) accent_bottom: D2D1_COLOR_F,
    pub(crate) accent_text: D2D1_COLOR_F,
}

pub(crate) fn rgb(hex: u32, alpha: f32) -> D2D1_COLOR_F {
    let c = |shift: u32| ((hex >> shift) & 0xFF) as f32 / 255.0;
    D2D1_COLOR_F {
        r: c(16),
        g: c(8),
        b: c(0),
        a: alpha,
    }
}

/// `alpha` fades everything in during the open animation.
pub(crate) fn palette(light: bool, glass: bool, alpha: f32) -> Palette {
    let a = |v: f32| v * alpha;
    let accent_top = rgb(0xEA4A4E, alpha);
    let accent_bottom = rgb(0xD2343A, alpha);
    let accent_text = rgb(0xFFFFFF, alpha);
    if light {
        Palette {
            disc: if glass {
                rgb(0xECECEF, a(0.62))
            } else {
                rgb(0xE6E6EA, alpha)
            },
            rim: rgb(0xFFFFFF, a(0.90)),
            outline: rgb(0x000000, a(0.10)),
            tile_top: rgb(0xFFFFFF, a(0.96)),
            tile_bottom: rgb(0xEFEFF2, a(0.96)),
            shadow: rgb(0x000000, a(0.10)),
            hub: rgb(0xFFFFFF, alpha),
            hub_ring: rgb(0xD6D6DB, alpha),
            text: rgb(0x2B2B2E, alpha),
            subtext: rgb(0x7A7A80, alpha),
            accent_top,
            accent_bottom,
            accent_text,
        }
    } else {
        Palette {
            disc: if glass {
                rgb(0x2A2A2E, a(0.58))
            } else {
                rgb(0x242428, alpha)
            },
            rim: rgb(0xFFFFFF, a(0.14)),
            outline: rgb(0x000000, a(0.35)),
            tile_top: rgb(0x4A4A50, a(0.92)),
            tile_bottom: rgb(0x3C3C42, a(0.92)),
            shadow: rgb(0x000000, a(0.35)),
            hub: rgb(0x3E3E44, alpha),
            hub_ring: rgb(0x55555C, alpha),
            text: rgb(0xF0F0F2, alpha),
            subtext: rgb(0xA8A8AE, alpha),
            accent_top,
            accent_bottom,
            accent_text,
        }
    }
}

/// A DWORD under the user's personalization settings; `default` if unset.
fn personalize_flag(name: PCWSTR, default: bool) -> bool {
    let mut value = u32::from(default);
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            name,
            RRF_RT_REG_DWORD,
            None,
            Some(std::ptr::from_mut(&mut value).cast()),
            Some(&mut size),
        )
    };
    if status.is_err() { default } else { value != 0 }
}

pub(crate) fn light_theme() -> bool {
    personalize_flag(w!("AppsUseLightTheme"), true)
}

/// Settings → Personalization → Colors → Transparency effects.
pub(crate) fn transparency_enabled() -> bool {
    personalize_flag(w!("EnableTransparency"), true)
}

pub struct Renderer {
    _device: ID3D11Device,
    factory: ID2D1Factory1,
    ctx: ID2D1DeviceContext,
    swap: IDXGISwapChain1,
    _compositor: Compositor,
    _target: DesktopWindowTarget,
    /// Acrylic blur of whatever is behind the window, clipped to `backdrop_clip`.
    backdrop: SpriteVisual,
    backdrop_clip: CompositionEllipseGeometry,
    /// The same blur behind the option panel.
    panel_backdrop: SpriteVisual,
    panel_clip: CompositionRoundedRectangleGeometry,
    _content: SpriteVisual,
    dwrite: IDWriteFactory,
    /// Family of the system icon font (see `glyph`).
    icon_font: PCWSTR,
    /// Round line joins: stroking a tile with its own fill rounds its corners.
    round: ID2D1StrokeStyle1,
    size: (u32, u32),
}

impl Renderer {
    pub fn new(hwnd: HWND, width: u32, height: u32) -> Result<Renderer> {
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

            // Needs a DispatcherQueue on this thread (created in app::run);
            // the compositor commits on its own as that queue is pumped.
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
            let backdrop_clip = compositor.CreateEllipseGeometry()?;
            backdrop.SetClip(&compositor.CreateGeometricClipWithGeometry(&backdrop_clip)?)?;
            backdrop.SetIsVisible(false)?;
            let panel_backdrop = compositor.CreateSpriteVisual()?;
            panel_backdrop.SetSize(full)?;
            panel_backdrop.SetBrush(&compositor.CreateHostBackdropBrush()?)?;
            let panel_clip = compositor.CreateRoundedRectangleGeometry()?;
            panel_backdrop.SetClip(&compositor.CreateGeometricClipWithGeometry(&panel_clip)?)?;
            panel_backdrop.SetIsVisible(false)?;

            let surface = compositor
                .cast::<ICompositorInterop>()?
                .CreateCompositionSurfaceForSwapChain(&swap)?;
            let content = compositor.CreateSpriteVisual()?;
            content.SetSize(full)?;
            content.SetBrush(&compositor.CreateSurfaceBrushWithSurface(&surface)?)?;

            let children = root.Children()?;
            children.InsertAtBottom(&backdrop)?;
            children.InsertAtBottom(&panel_backdrop)?;
            children.InsertAtTop(&content)?;

            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let icon_font = icon_font(&dwrite);
            let round = factory.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES1 {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    dashCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    miterLimit: 10.0,
                    dashStyle: D2D1_DASH_STYLE_SOLID,
                    dashOffset: 0.0,
                    transformType: D2D1_STROKE_TRANSFORM_TYPE_NORMAL,
                },
                None,
            )?;
            Ok(Renderer {
                _device: device,
                factory,
                ctx,
                swap,
                _compositor: compositor,
                _target: target,
                backdrop,
                backdrop_clip,
                panel_backdrop,
                panel_clip,
                _content: content,
                dwrite,
                icon_font,
                round,
                size: (width, height),
            })
        }
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Presents a fully transparent frame (the window stays hit-testable).
    pub fn clear(&mut self) -> Result<()> {
        unsafe {
            self.ctx.BeginDraw();
            self.ctx.Clear(Some(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }));
            self.ctx.EndDraw(None, None)?;
            self.swap.Present(1, DXGI_PRESENT(0)).ok()?;
        }
        self.panel_backdrop.SetIsVisible(false)?;
        self.backdrop.SetIsVisible(false)
    }

    /// `origin` is the wheel center in the surface (the panel side is wider).
    pub fn draw(
        &mut self,
        view: &WheelView,
        scale: f32,
        glass_supported: bool,
        (cx, cy): (f32, f32),
    ) -> Result<()> {
        let eased = 1.0 - (1.0 - view.progress.clamp(0.0, 1.0)).powi(3);
        let grow = OPEN_SCALE_FROM + (1.0 - OPEN_SCALE_FROM) * eased;
        // With transparency effects off the host backdrop draws nothing useful,
        // so fall back to an opaque disc.
        let glass = glass_supported && transparency_enabled();
        let light = light_theme();
        let colors = palette(light, glass, eased);
        let n = view.items.len();
        let disc = OUTER_RADIUS * scale * grow;
        self.backdrop.SetIsVisible(glass)?;
        self.backdrop.SetOpacity(eased)?;
        self.backdrop_clip.SetCenter(Vector2 { X: cx, Y: cy })?;
        self.backdrop_clip.SetRadius(Vector2 { X: disc, Y: disc })?;
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

            // Base disc with a light edge inside and a faint line outside.
            ctx.FillEllipse(&ellipse_at(cx, cy, disc), &brush(&colors.disc)?);
            ctx.DrawEllipse(
                &ellipse_at(cx, cy, disc + 0.5 * scale),
                &brush(&colors.outline)?,
                scale,
                None,
            );
            ctx.DrawEllipse(
                &ellipse_at(cx, cy, disc - 0.5 * scale),
                &brush(&colors.rim)?,
                scale,
                None,
            );

            // Icon above, small label below; two rings make for shorter tiles.
            let (icon_size, label_size, icon_dy, label_dy) = if n > RING_CAPACITY {
                (16.0, 9.0, -6.0, 9.0)
            } else {
                (22.0, 10.5, -7.0, 13.0)
            };
            let icon_format =
                self.font_format(self.icon_font, icon_size * scale, DWRITE_FONT_WEIGHT_NORMAL)?;
            let label_format =
                self.text_format(label_size * scale, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
            let shadow = brush(&colors.shadow)?;
            for (i, op) in view.items.iter().enumerate() {
                let s = sector(n, i, scale);
                let hovered = view.hovered == Hit::Item(i);
                let inner = s.inner_radius * grow;
                let outer =
                    (s.outer_radius + if hovered { HOVER_GROW * scale } else { 0.0 }) * grow;
                let tile = Tile::new(inner, outer, s.start_deg, s.end_deg, scale);

                // Drop shadow: the same tile, 1 px lower.
                self.fill_tile(
                    &self.tile_path(cx, cy + 1.2 * scale, &tile)?,
                    &tile,
                    &shadow,
                )?;
                let (top, bottom) = if hovered {
                    (&colors.accent_top, &colors.accent_bottom)
                } else {
                    (&colors.tile_top, &colors.tile_bottom)
                };
                let (y0, y1) = tile.vertical_extent(cy);
                let fill = self.vertical_gradient(cx, y0, y1, top, bottom)?;
                self.fill_tile(&self.tile_path(cx, cy, &tile)?, &tile, &fill)?;

                let mid = (s.start_deg + s.end_deg) / 2.0;
                let (x, y) = polar_at(cx, cy, (inner + outer) / 2.0, mid);
                let text = brush(if hovered {
                    &colors.accent_text
                } else {
                    &colors.text
                })?;
                self.draw_text(
                    glyph(op).encode_utf8(&mut [0; 4]),
                    &icon_format,
                    x,
                    y + icon_dy * scale,
                    (icon_size + 8.0) * scale,
                    (icon_size + 8.0) * scale,
                    &text,
                );
                self.draw_text(
                    op.label(),
                    &label_format,
                    x,
                    y + label_dy * scale,
                    76.0 * scale,
                    (label_size + 6.0) * scale,
                    &text,
                );
            }

            // Hub: a raised circle with what is being dragged and the wheel's name.
            let hub = (CANCEL_RADIUS - HUB_INSET) * scale * grow;
            ctx.FillEllipse(&ellipse_at(cx, cy + 1.2 * scale, hub), &shadow);
            ctx.FillEllipse(&ellipse_at(cx, cy, hub), &brush(&colors.hub)?);
            ctx.DrawEllipse(
                &ellipse_at(cx, cy, hub - 0.5 * scale),
                &brush(&colors.hub_ring)?,
                scale,
                None,
            );
            let detail_format = self.text_format(13.0 * scale, DWRITE_FONT_WEIGHT_BOLD)?;
            let title_format = self.text_format(10.5 * scale, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
            let width = 2.0 * hub - 8.0 * scale;
            if view.detail.is_empty() {
                self.draw_text(
                    view.title,
                    &detail_format,
                    cx,
                    cy,
                    width,
                    22.0 * scale,
                    &brush(&colors.text)?,
                );
            } else {
                self.draw_text(
                    view.detail,
                    &detail_format,
                    cx,
                    cy - 6.0 * scale,
                    width,
                    20.0 * scale,
                    &brush(&colors.text)?,
                );
                self.draw_text(
                    view.title,
                    &title_format,
                    cx,
                    cy + 11.0 * scale,
                    width,
                    16.0 * scale,
                    &brush(&colors.subtext)?,
                );
            }

            match &view.panel {
                Some(panel) => self.draw_panel(panel, scale, light, glass, eased, (cx, cy))?,
                None => self.panel_backdrop.SetIsVisible(false)?,
            }

            ctx.EndDraw(None, None)?;
            self.swap.Present(1, DXGI_PRESENT(0)).ok()?;
        }
        Ok(())
    }

    /// The option panel beside the wheel (spec 2.5): a glass card with a
    /// title and one rounded row per choice, the hovered row in the accent.
    fn draw_panel(
        &self,
        panel: &PanelView,
        scale: f32,
        light: bool,
        glass: bool,
        wheel_fade: f32,
        (cx, cy): (f32, f32),
    ) -> Result<()> {
        let fade = (1.0 - (1.0 - panel.progress.clamp(0.0, 1.0)).powi(3)).min(wheel_fade);
        let colors = palette(light, glass, fade);
        let r = panel_rect(panel.side, panel.choices.len(), scale);
        let rect = D2D_RECT_F {
            left: cx + r.left,
            top: cy + r.top,
            right: cx + r.right,
            bottom: cy + r.bottom,
        };
        let corner = PANEL_CORNER * scale;
        self.panel_clip.SetOffset(point((rect.left, rect.top)))?;
        self.panel_clip.SetSize(Vector2 {
            X: rect.right - rect.left,
            Y: rect.bottom - rect.top,
        })?;
        self.panel_clip.SetCornerRadius(Vector2 {
            X: corner,
            Y: corner,
        })?;
        self.panel_backdrop.SetIsVisible(glass)?;
        self.panel_backdrop.SetOpacity(fade)?;

        let ctx = &self.ctx;
        let brush = |color: &D2D1_COLOR_F| unsafe { ctx.CreateSolidColorBrush(color, None) };
        let rounded = |rect: D2D_RECT_F, radius: f32| D2D1_ROUNDED_RECT {
            rect,
            radiusX: radius,
            radiusY: radius,
        };
        let inset = |rect: D2D_RECT_F, d: f32| D2D_RECT_F {
            left: rect.left + d,
            top: rect.top + d,
            right: rect.right - d,
            bottom: rect.bottom - d,
        };
        unsafe {
            ctx.FillRoundedRectangle(&rounded(rect, corner), &brush(&colors.disc)?);
            ctx.DrawRoundedRectangle(
                &rounded(inset(rect, -0.5 * scale), corner),
                &brush(&colors.outline)?,
                scale,
                None,
            );
            ctx.DrawRoundedRectangle(
                &rounded(inset(rect, 0.5 * scale), corner),
                &brush(&colors.rim)?,
                scale,
                None,
            );
        }

        let pad = 12.0 * scale;
        let title_format = self.aligned_format(
            10.5 * scale,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_TEXT_ALIGNMENT_LEADING,
        )?;
        let label_format = self.aligned_format(
            13.0 * scale,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_TEXT_ALIGNMENT_LEADING,
        )?;
        let detail_format = self.aligned_format(
            10.5 * scale,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_TEXT_ALIGNMENT_TRAILING,
        )?;
        let width = rect.right - rect.left - 2.0 * pad;
        let mid_x = (rect.left + rect.right) / 2.0;
        self.draw_text(
            panel.title,
            &title_format,
            mid_x,
            rect.top + PANEL_HEADER * scale / 2.0 + 2.0 * scale,
            width,
            PANEL_HEADER * scale,
            &brush(&colors.subtext)?,
        );

        let shadow = brush(&colors.shadow)?;
        for (i, choice) in panel.choices.iter().enumerate() {
            let top = rect.top + (PANEL_HEADER + i as f32 * PANEL_ROW) * scale;
            let row = D2D_RECT_F {
                left: rect.left + PANEL_PAD * scale,
                top: top + 2.0 * scale,
                right: rect.right - PANEL_PAD * scale,
                bottom: top + (PANEL_ROW - 2.0) * scale,
            };
            let hovered = panel.hovered == Some(i);
            let (fill_top, fill_bottom, text, subtext) = if hovered {
                (
                    &colors.accent_top,
                    &colors.accent_bottom,
                    &colors.accent_text,
                    &colors.accent_text,
                )
            } else {
                (
                    &colors.tile_top,
                    &colors.tile_bottom,
                    &colors.text,
                    &colors.subtext,
                )
            };
            let lowered = D2D_RECT_F {
                top: row.top + 1.2 * scale,
                bottom: row.bottom + 1.2 * scale,
                ..row
            };
            let fill = self.vertical_gradient(mid_x, row.top, row.bottom, fill_top, fill_bottom)?;
            unsafe {
                ctx.FillRoundedRectangle(&rounded(lowered, ROW_CORNER * scale), &shadow);
                ctx.FillRoundedRectangle(&rounded(row, ROW_CORNER * scale), &fill);
            }
            let row_mid_y = (row.top + row.bottom) / 2.0;
            let row_width = row.right - row.left - 2.0 * 10.0 * scale;
            self.draw_text(
                choice.label,
                &label_format,
                mid_x,
                row_mid_y,
                row_width,
                row.bottom - row.top,
                &brush(text)?,
            );
            let detail = if i == panel.default {
                format!("{} · 기본", choice.detail)
            } else {
                choice.detail.to_string()
            };
            self.draw_text(
                &detail,
                &detail_format,
                mid_x,
                row_mid_y,
                row_width,
                row.bottom - row.top,
                &brush(subtext)?,
            );
        }
        Ok(())
    }

    /// Fills a tile path and strokes it with the same brush using round joins;
    /// the stroke's half-width is exactly the inset `Tile` left for it, so the
    /// result is the tile with rounded corners.
    fn fill_tile(&self, path: &ID2D1PathGeometry1, tile: &Tile, brush: &ID2D1Brush) -> Result<()> {
        unsafe {
            self.ctx.FillGeometry(path, brush, None);
            if tile.corner > 0.0 {
                self.ctx
                    .DrawGeometry(path, brush, 2.0 * tile.corner, &self.round);
            }
        }
        Ok(())
    }

    fn vertical_gradient(
        &self,
        x: f32,
        y0: f32,
        y1: f32,
        top: &D2D1_COLOR_F,
        bottom: &D2D1_COLOR_F,
    ) -> Result<ID2D1Brush> {
        unsafe {
            let stops = [
                D2D1_GRADIENT_STOP {
                    position: 0.0,
                    color: *top,
                },
                D2D1_GRADIENT_STOP {
                    position: 1.0,
                    color: *bottom,
                },
            ];
            // The device context's own overload takes color spaces; use the
            // classic render-target one.
            let rt: &ID2D1RenderTarget = &self.ctx;
            let collection =
                rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?;
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                startPoint: point((x, y0)),
                endPoint: point((x, y1)),
            };
            self.ctx
                .CreateLinearGradientBrush(&props, None, &collection)?
                .cast()
        }
    }

    fn text_format(&self, size: f32, weight: DWRITE_FONT_WEIGHT) -> Result<IDWriteTextFormat> {
        // Hangul falls back to the system's Korean UI font automatically.
        self.font_format(w!("Segoe UI Variable Text"), size, weight)
    }

    /// `text_format` aligned to one side of its box instead of centered.
    fn aligned_format(
        &self,
        size: f32,
        weight: DWRITE_FONT_WEIGHT,
        alignment: DWRITE_TEXT_ALIGNMENT,
    ) -> Result<IDWriteTextFormat> {
        let format = self.text_format(size, weight)?;
        unsafe { format.SetTextAlignment(alignment)? };
        Ok(format)
    }

    fn font_format(
        &self,
        family: PCWSTR,
        size: f32,
        weight: DWRITE_FONT_WEIGHT,
    ) -> Result<IDWriteTextFormat> {
        unsafe {
            let format = self.dwrite.CreateTextFormat(
                family,
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

    /// Draws `text` centered on (`x`, `y`) in a `width` × `height` box.
    #[allow(clippy::too_many_arguments)]
    fn draw_text(
        &self,
        text: &str,
        format: &IDWriteTextFormat,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        brush: &ID2D1SolidColorBrush,
    ) {
        let wide = HSTRING::from(text);
        let rect = D2D_RECT_F {
            left: x - width / 2.0,
            top: y - height / 2.0,
            right: x + width / 2.0,
            bottom: y + height / 2.0,
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

    /// The tile's core shape centered at (`cx`, `cy`); see `fill_tile`.
    fn tile_path(&self, cx: f32, cy: f32, tile: &Tile) -> Result<ID2D1PathGeometry1> {
        unsafe {
            let path = self.factory.CreatePathGeometry()?;
            let sink = path.Open()?;
            if tile.full_ring {
                // A single item: a whole ring, drawn as two circles (alternate fill).
                sink.SetFillMode(D2D1_FILL_MODE_ALTERNATE);
                for r in [tile.outer, tile.inner] {
                    sink.BeginFigure(point(polar_at(cx, cy, r, 0.0)), D2D1_FIGURE_BEGIN_FILLED);
                    arc_to(&sink, cx, cy, r, 0.0, 360.0, D2D1_SWEEP_DIRECTION_CLOCKWISE);
                    sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                }
            } else {
                let (os, oe) = tile.edge_angles(tile.outer);
                let (is, ie) = tile.edge_angles(tile.inner);
                sink.BeginFigure(
                    point(polar_at(cx, cy, tile.outer, os)),
                    D2D1_FIGURE_BEGIN_FILLED,
                );
                arc_to(
                    &sink,
                    cx,
                    cy,
                    tile.outer,
                    os,
                    oe,
                    D2D1_SWEEP_DIRECTION_CLOCKWISE,
                );
                sink.AddLine(point(polar_at(cx, cy, tile.inner, ie)));
                arc_to(
                    &sink,
                    cx,
                    cy,
                    tile.inner,
                    ie,
                    is,
                    D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
                );
                sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            }
            sink.Close()?;
            Ok(path)
        }
    }
}

/// One sector drawn as a separate rounded tile. The core shape is the sector
/// shrunk by `TILE_GAP / 2 + corner` on every side; stroking it with round
/// joins at half-width `corner` grows it back to a `TILE_GAP / 2` margin with
/// rounded corners.
struct Tile {
    inner: f32,
    outer: f32,
    start: f32,
    end: f32,
    /// Distance between the core's straight edges and the sector's edges.
    offset: f32,
    corner: f32,
    full_ring: bool,
}

impl Tile {
    fn new(inner: f32, outer: f32, start: f32, end: f32, scale: f32) -> Tile {
        let half_gap = TILE_GAP / 2.0 * scale;
        // Thin tiles (two rings, high item counts) get smaller corners.
        let corner = (TILE_CORNER * scale).min(((outer - inner) / 2.0 - half_gap - 0.5).max(0.0));
        let offset = half_gap + corner;
        Tile {
            inner: inner + offset,
            outer: outer - offset,
            start,
            end,
            offset,
            corner,
            full_ring: end - start >= 359.9,
        }
    }

    /// The core's start and end angles at radius `r`, pulled in so the straight
    /// edges stay `offset` from the sector's edges (more degrees near the hub).
    fn edge_angles(&self, r: f32) -> (f32, f32) {
        let inset = (self.offset / r.max(1.0)).min(0.99).asin().to_degrees();
        let (s, e) = (self.start + inset, self.end - inset);
        if s < e {
            (s, e)
        } else {
            let mid = (self.start + self.end) / 2.0; // too narrow: collapse to a point
            (mid, mid)
        }
    }

    /// Top and bottom y of the finished tile around a center at `cy`, for its gradient.
    fn vertical_extent(&self, cy: f32) -> (f32, f32) {
        let (mut y0, mut y1) = (f32::MAX, f32::MIN);
        for k in 0..=8 {
            let deg = self.start + (self.end - self.start) * k as f32 / 8.0;
            for r in [self.outer + self.corner, self.inner - self.corner] {
                let y = polar_at(0.0, cy, r, deg).1;
                y0 = y0.min(y);
                y1 = y1.max(y);
            }
        }
        (y0, y1)
    }
}

/// Segoe Fluent Icons ships with Windows 11; Windows 10 has the older Segoe
/// MDL2 Assets, which has the same code points for the icons we use.
pub(crate) fn icon_font(dwrite: &IDWriteFactory) -> PCWSTR {
    let fluent = w!("Segoe Fluent Icons");
    let mut fonts = None;
    let (mut index, mut exists) = (0, BOOL::default());
    let found = unsafe {
        dwrite.GetSystemFontCollection(&mut fonts, false).is_ok()
            && fonts.is_some_and(|f| {
                f.FindFamilyName(fluent, &mut index, &mut exists).is_ok() && exists.as_bool()
            })
    };
    if found {
        fluent
    } else {
        w!("Segoe MDL2 Assets")
    }
}

/// Arcs from `from` to `to` degrees around (`cx`, `cy`), split so no piece
/// exceeds 180° (a full circle has identical end points, which a single arc
/// can't express).
fn arc_to(
    sink: &ID2D1GeometrySink,
    cx: f32,
    cy: f32,
    r: f32,
    from: f32,
    to: f32,
    dir: D2D1_SWEEP_DIRECTION,
) {
    let pieces = ((to - from).abs() / 180.0).ceil().max(1.0) as usize;
    for k in 1..=pieces {
        let angle = from + (to - from) * k as f32 / pieces as f32;
        let segment = D2D1_ARC_SEGMENT {
            point: point(polar_at(cx, cy, r, angle)),
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

/// The point at radius `r` and `deg` degrees (0° = 12 o'clock, clockwise).
fn polar_at(cx: f32, cy: f32, r: f32, deg: f32) -> (f32, f32) {
    let rad = deg.to_radians();
    (cx + r * rad.sin(), cy - r * rad.cos())
}

/// Direct2D points are `windows_numerics::Vector2` in windows 0.61.
fn point((x, y): (f32, f32)) -> Vector2 {
    Vector2 { X: x, Y: y }
}

fn ellipse_at(cx: f32, cy: f32, r: f32) -> D2D1_ELLIPSE {
    D2D1_ELLIPSE {
        point: point((cx, cy)),
        radiusX: r,
        radiusY: r,
    }
}

pub(crate) fn create_d3d_device() -> Result<ID3D11Device> {
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
