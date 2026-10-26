//! Draws the wheel. A Windows.UI.Composition visual tree gives the window
//! per-pixel transparency: at the bottom a host-backdrop (acrylic blur) disc,
//! on top a Direct2D swap chain with the tint, sectors, and labels.

use swisswheel_core::appearance::Accent;
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
use windows::Win32::System::WinRT::Composition::{ICompositorDesktopInterop, ICompositorInterop};
use windows::core::{BOOL, HSTRING, Interface, PCWSTR, Result, w};
use windows_numerics::Vector2;

use super::glyph::glyph;
use super::window::HOVER_GROW;
use super::{PanelView, WheelView};
use crate::fonts;
use crate::theme::{Piece, Theme, glass_layer, glass_piece, rounded_shape};

const OPEN_SCALE_FROM: f32 = 0.92; // wheel scales from 92% to 100% while opening

/// Gap between neighboring tiles and the rounding of their corners (DIPs).
const TILE_GAP: f32 = 4.0;
const TILE_CORNER: f32 = 9.0;
/// The hub (center circle) sits this far inside the cancel radius.
const HUB_INSET: f32 = 6.0;
/// Rounding of the option panel and of its rows (DIPs).
pub(super) const PANEL_CORNER: f32 = 16.0;
const ROW_CORNER: f32 = 11.0;
/// Marks the default choice in the option panel.
const BADGE: &str = "기본";

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
    /// Family of the system icon font (see `glyph`).
    icon_font: PCWSTR,
    /// Round line joins: widening a tile's core by its corner rounds them.
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

            let icon_font = icon_font(&fonts::factory());
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
        // so the theme falls back to an opaque disc.
        let theme = Theme::current(glass_supported, view.accent, eased);
        let n = view.items.len();
        let disc = OUTER_RADIUS * scale * grow;
        self.backdrop.SetIsVisible(theme.glass)?;
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

            glass_layer(
                ctx,
                &self.circle(cx, cy, disc)?,
                square(cx, cy, disc),
                &theme,
                scale,
            )?;

            // Icon above, small label below; two rings make for shorter tiles.
            let (icon_size, label_size, icon_dy, label_dy) = if n > RING_CAPACITY {
                (16.0, 9.0, -6.0, 9.0)
            } else {
                (22.0, 10.5, -7.0, 13.0)
            };
            let icon_format = fonts::icon_format(self.icon_font, icon_size * scale)?;
            let label_format = fonts::text_format(
                label_size * scale,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_TEXT_ALIGNMENT_CENTER,
            )?;
            let text = brush(&theme.text)?;
            let on_accent = brush(&theme.on_accent)?;
            // The hovered tile last: it grows over its neighbors' edges.
            let order = (0..n)
                .filter(|&i| view.hovered != Hit::Item(i))
                .chain((0..n).filter(|&i| view.hovered == Hit::Item(i)));
            for i in order {
                let op = &view.items[i];
                let s = sector(n, i, scale);
                let hovered = view.hovered == Hit::Item(i);
                let inner = s.inner_radius * grow;
                let outer =
                    (s.outer_radius + if hovered { HOVER_GROW * scale } else { 0.0 }) * grow;
                let tile = Tile::new(inner, outer, s.start_deg, s.end_deg, scale);
                let shape = self.tile_shape(cx, cy, &tile)?;
                let piece = if hovered { Piece::Accent } else { Piece::Plain };
                glass_piece(ctx, &shape, shape.GetBounds(None)?, &theme, piece, scale)?;

                let mid = (s.start_deg + s.end_deg) / 2.0;
                let (x, y) = polar_at(cx, cy, (inner + outer) / 2.0, mid);
                let text = if hovered { &on_accent } else { &text };
                self.draw_text(
                    glyph(op).encode_utf8(&mut [0; 4]),
                    &icon_format,
                    x,
                    y + icon_dy * scale,
                    (icon_size + 8.0) * scale,
                    (icon_size + 8.0) * scale,
                    text,
                );
                self.draw_text(
                    op.label(),
                    &label_format,
                    x,
                    y + label_dy * scale,
                    76.0 * scale,
                    (label_size + 6.0) * scale,
                    text,
                );
            }

            // Hub: a glass piece with what is being dragged and the wheel's name.
            let hub = (CANCEL_RADIUS - HUB_INSET) * scale * grow;
            glass_piece(
                ctx,
                &self.circle(cx, cy, hub)?,
                square(cx, cy, hub),
                &theme,
                Piece::Plain,
                scale,
            )?;
            let detail_format = fonts::text_format(
                14.0 * scale,
                DWRITE_FONT_WEIGHT_BOLD,
                DWRITE_TEXT_ALIGNMENT_CENTER,
            )?;
            let title_format = fonts::text_format(
                10.5 * scale,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_TEXT_ALIGNMENT_CENTER,
            )?;
            let width = 2.0 * hub - 8.0 * scale;
            if view.detail.is_empty() {
                self.draw_text(
                    view.title,
                    &detail_format,
                    cx,
                    cy,
                    width,
                    22.0 * scale,
                    &text,
                );
            } else {
                self.draw_text(
                    view.detail,
                    &detail_format,
                    cx,
                    cy - 6.0 * scale,
                    width,
                    20.0 * scale,
                    &text,
                );
                self.draw_text(
                    view.title,
                    &title_format,
                    cx,
                    cy + 11.0 * scale,
                    width,
                    16.0 * scale,
                    &brush(&theme.subtext)?,
                );
            }

            match &view.panel {
                Some(panel) => self.draw_panel(panel, scale, &theme, view.accent, (cx, cy))?,
                None => self.panel_backdrop.SetIsVisible(false)?,
            }

            ctx.EndDraw(None, None)?;
            self.swap.Present(1, DXGI_PRESENT(0)).ok()?;
        }
        Ok(())
    }

    /// The option panel beside the wheel (spec 2.5): a glass layer with a
    /// title and one row per choice, the hovered row an accent piece.
    fn draw_panel(
        &self,
        panel: &PanelView,
        scale: f32,
        wheel: &Theme,
        accent: Accent,
        (cx, cy): (f32, f32),
    ) -> Result<()> {
        let fade = (1.0 - (1.0 - panel.progress.clamp(0.0, 1.0)).powi(3)).min(wheel.alpha);
        let theme = Theme::new(wheel.light, wheel.glass, accent, fade);
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
        self.panel_backdrop.SetIsVisible(theme.glass)?;
        self.panel_backdrop.SetOpacity(fade)?;

        let ctx = &self.ctx;
        let brush = |color: &D2D1_COLOR_F| unsafe { ctx.CreateSolidColorBrush(color, None) };
        glass_layer(ctx, &rounded_shape(ctx, rect, corner)?, rect, &theme, scale)?;

        let pad = 12.0 * scale;
        let title_format = fonts::text_format(
            10.5 * scale,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_TEXT_ALIGNMENT_LEADING,
        )?;
        let label_format = fonts::text_format(
            13.0 * scale,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_TEXT_ALIGNMENT_LEADING,
        )?;
        let detail_format = fonts::text_format(
            10.5 * scale,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_TEXT_ALIGNMENT_TRAILING,
        )?;
        let badge_format = fonts::text_format(
            10.0 * scale,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_TEXT_ALIGNMENT_CENTER,
        )?;
        let width = rect.right - rect.left - 2.0 * pad;
        let mid_x = (rect.left + rect.right) / 2.0;
        let (text, subtext) = (brush(&theme.text)?, brush(&theme.subtext)?);
        let on_accent = brush(&theme.on_accent)?;
        self.draw_text(
            panel.title,
            &title_format,
            mid_x,
            rect.top + PANEL_HEADER * scale / 2.0 + 2.0 * scale,
            width,
            PANEL_HEADER * scale,
            &subtext,
        );

        for (i, choice) in panel.choices.iter().enumerate() {
            let top = rect.top + (PANEL_HEADER + i as f32 * PANEL_ROW) * scale;
            let row = D2D_RECT_F {
                left: rect.left + PANEL_PAD * scale,
                top: top + 2.0 * scale,
                right: rect.right - PANEL_PAD * scale,
                bottom: top + (PANEL_ROW - 2.0) * scale,
            };
            let hovered = panel.hovered == Some(i);
            if hovered {
                let shape = rounded_shape(ctx, row, ROW_CORNER * scale)?;
                glass_piece(ctx, &shape, row, &theme, Piece::Accent, scale)?;
            }
            let (text, subtext) = if hovered {
                (&on_accent, &on_accent)
            } else {
                (&text, &subtext)
            };
            let row_mid_y = (row.top + row.bottom) / 2.0;
            let inner = 10.0 * scale;
            let row_width = row.right - row.left - 2.0 * inner;
            self.draw_text(
                choice.label,
                &label_format,
                mid_x,
                row_mid_y,
                row_width,
                row.bottom - row.top,
                text,
            );
            self.draw_text(
                choice.detail,
                &detail_format,
                mid_x,
                row_mid_y,
                row_width,
                row.bottom - row.top,
                subtext,
            );
            if i == panel.default {
                // A small pill right after the label.
                let (label_width, _) = fonts::text_size(choice.label, &label_format)?;
                let (badge_width, badge_height) = fonts::text_size(BADGE, &badge_format)?;
                let (w, h) = (badge_width + 12.0 * scale, badge_height + 3.0 * scale);
                let left = row.left + inner + label_width + 6.0 * scale;
                let pill = D2D_RECT_F {
                    left,
                    top: row_mid_y - h / 2.0,
                    right: left + w,
                    bottom: row_mid_y + h / 2.0,
                };
                let shape = rounded_shape(ctx, pill, h / 2.0)?;
                glass_piece(ctx, &shape, pill, &theme, Piece::Plain, scale)?;
                self.draw_text(
                    BADGE,
                    &badge_format,
                    left + w / 2.0,
                    row_mid_y,
                    w,
                    h,
                    subtext,
                );
            }
        }
        Ok(())
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

    fn circle(&self, cx: f32, cy: f32, r: f32) -> Result<ID2D1Geometry> {
        unsafe {
            self.factory
                .CreateEllipseGeometry(&ellipse_at(cx, cy, r))?
                .cast()
        }
    }

    /// The finished tile: its core path grown by `corner` with round joins
    /// (the core plus its widened outline), so it has rounded corners.
    fn tile_shape(&self, cx: f32, cy: f32, tile: &Tile) -> Result<ID2D1Geometry> {
        let core = self.tile_path(cx, cy, tile)?;
        if tile.corner <= 0.0 {
            return core.cast();
        }
        unsafe {
            let outline = self.factory.CreatePathGeometry()?;
            let sink = outline.Open()?;
            core.Widen(
                2.0 * tile.corner,
                &self.round,
                None,
                D2D1_DEFAULT_FLATTENING_TOLERANCE,
                &sink,
            )?;
            sink.Close()?;
            let shape = self.factory.CreatePathGeometry()?;
            let sink = shape.Open()?;
            core.CombineWithGeometry(
                &outline,
                D2D1_COMBINE_MODE_UNION,
                None,
                D2D1_DEFAULT_FLATTENING_TOLERANCE,
                &sink,
            )?;
            sink.Close()?;
            shape.cast()
        }
    }

    /// The tile's core shape centered at (`cx`, `cy`); see `tile_shape`.
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
/// shrunk by `TILE_GAP / 2 + corner` on every side; widening it with round
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

/// The bounds of the circle of radius `r` around (`cx`, `cy`).
fn square(cx: f32, cy: f32, r: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left: cx - r,
        top: cy - r,
        right: cx + r,
        bottom: cy + r,
    }
}

fn ellipse_at(cx: f32, cy: f32, r: f32) -> D2D1_ELLIPSE {
    D2D1_ELLIPSE {
        point: point((cx, cy)),
        radiusX: r,
        radiusY: r,
    }
}

pub(crate) fn create_d3d_device() -> Result<ID3D11Device> {
    create_d3d_device_with(D3D11_CREATE_DEVICE_BGRA_SUPPORT)
}

pub(crate) fn create_d3d_device_with(flags: D3D11_CREATE_DEVICE_FLAG) -> Result<ID3D11Device> {
    let mut device = None;
    let mut attempt = |driver: D3D_DRIVER_TYPE| unsafe {
        D3D11CreateDevice(
            None,
            driver,
            HMODULE::default(),
            flags,
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
