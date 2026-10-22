//! Color tokens and the shared liquid-glass drawing routines.

use std::mem::ManuallyDrop;

use swisswheel_core::appearance::{Accent, FAILURE_RGB};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::core::{Interface, PCWSTR, Result, w};
use windows_numerics::{Matrix3x2, Vector2};

/// Rim stop alphas (top-left, middle, bottom-right) in the light theme.
const RIM_LIGHT: [f32; 3] = [0.95, 0.10, 0.70];
/// The dark theme's rim is this fraction of the light one.
const RIM_DARK: f32 = 0.4;

pub(crate) fn rgb(hex: u32, alpha: f32) -> D2D1_COLOR_F {
    let c = |shift: u32| ((hex >> shift) & 0xFF) as f32 / 255.0;
    D2D1_COLOR_F {
        r: c(16),
        g: c(8),
        b: c(0),
        a: alpha,
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

/// The taskbar and tray are light ("Windows mode" in Settings → Colors),
/// which is set apart from the apps' mode.
pub(crate) fn taskbar_light_theme() -> bool {
    personalize_flag(w!("SystemUsesLightTheme"), false)
}

/// Settings → Personalization → Colors → Transparency effects.
pub(crate) fn transparency_enabled() -> bool {
    personalize_flag(w!("EnableTransparency"), true)
}

/// The resolved colors for one frame. Every color, and `rim`, is already
/// multiplied by `alpha` (which fades everything in while opening).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    pub light: bool,
    /// Drawn over the system blur; otherwise over the `opaque` plate.
    pub glass: bool,
    pub alpha: f32,
    pub accent: D2D1_COLOR_F,
    pub failure: D2D1_COLOR_F,
    /// Text and icons on an accent piece.
    pub on_accent: D2D1_COLOR_F,
    /// Base layer tint, top to bottom.
    pub tint_top: D2D1_COLOR_F,
    pub tint_bottom: D2D1_COLOR_F,
    /// Base layer fill when transparency effects are off.
    pub opaque: D2D1_COLOR_F,
    pub piece: D2D1_COLOR_F,
    pub piece_selected: D2D1_COLOR_F,
    /// Inner glow along the edge.
    pub glow: D2D1_COLOR_F,
    /// Highlight over the top half, fading to nothing.
    pub sheen: D2D1_COLOR_F,
    pub text: D2D1_COLOR_F,
    pub subtext: D2D1_COLOR_F,
    /// Separators.
    pub line: D2D1_COLOR_F,
    /// Progress bar background.
    pub track: D2D1_COLOR_F,
    /// White rim alphas at the top-left, middle, and bottom-right.
    pub rim: [f32; 3],
}

impl Theme {
    pub fn new(light: bool, glass: bool, accent: Accent, alpha: f32) -> Theme {
        let c = |hex: u32, a: f32| rgb(hex, a * alpha);
        let accent = c(accent.rgb(), if glass { 0.85 } else { 1.0 });
        let failure = c(FAILURE_RGB, 1.0);
        if light {
            Theme {
                light,
                glass,
                alpha,
                accent,
                failure,
                on_accent: c(0xFFFFFF, 1.0),
                tint_top: c(0xFFFFFF, 0.32),
                tint_bottom: c(0xFFFFFF, 0.18),
                opaque: c(0xF2F2F5, 1.0),
                piece: c(0xFFFFFF, 0.30),
                piece_selected: c(0xFFFFFF, 0.50),
                glow: c(0xFFFFFF, 0.35),
                sheen: c(0xFFFFFF, 0.35),
                text: c(0x1D1D1F, 1.0),
                subtext: c(0x5B5B61, 1.0),
                line: c(0x000000, 0.06),
                track: c(0x000000, 0.10),
                rim: RIM_LIGHT.map(|r| r * alpha),
            }
        } else {
            Theme {
                light,
                glass,
                alpha,
                accent,
                failure,
                on_accent: c(0xFFFFFF, 1.0),
                tint_top: c(0x2A2A2E, 0.55),
                tint_bottom: c(0x1C1C1E, 0.45),
                opaque: c(0x1F1F22, 1.0),
                piece: c(0xFFFFFF, 0.08),
                piece_selected: c(0xFFFFFF, 0.14),
                glow: c(0xFFFFFF, 0.12),
                sheen: c(0xFFFFFF, 0.10),
                text: c(0xF5F5F7, 1.0),
                subtext: c(0xA1A1A8, 1.0),
                line: c(0xFFFFFF, 0.08),
                track: c(0xFFFFFF, 0.12),
                rim: RIM_LIGHT.map(|r| r * RIM_DARK * alpha),
            }
        }
    }

    /// The system's app theme; glass only where supported and transparency
    /// effects are on.
    pub fn current(glass_supported: bool, accent: Accent, alpha: f32) -> Theme {
        Theme::new(
            light_theme(),
            glass_supported && transparency_enabled(),
            accent,
            alpha,
        )
    }
}

/// What a glass piece is filled with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Piece {
    Plain,
    Selected,
    Accent,
    Keycap,
    /// The selected tab of a segmented control.
    Tab,
}

/// A base glass layer (wheel disc, menu bar, panels, cards): tint, inner
/// glow, sheen, rim. `scale` converts DIPs to the target's pixels.
pub fn glass_layer(
    ctx: &ID2D1DeviceContext,
    shape: &ID2D1Geometry,
    bounds: D2D_RECT_F,
    theme: &Theme,
    scale: f32,
) -> Result<()> {
    let fill = if theme.glass {
        vertical_gradient(
            ctx,
            bounds.top,
            bounds.bottom,
            theme.tint_top,
            theme.tint_bottom,
        )?
    } else {
        solid(ctx, theme.opaque)?
    };
    unsafe { ctx.FillGeometry(shape, &fill, None) };
    finish(ctx, shape, bounds, theme, theme.glow, scale)
}

/// A glass piece on top of a layer (tiles, hub, rows, tabs, keycaps): a
/// short drop shadow, the fill, then glow, sheen, and rim like a layer.
pub fn glass_piece(
    ctx: &ID2D1DeviceContext,
    shape: &ID2D1Geometry,
    bounds: D2D_RECT_F,
    theme: &Theme,
    piece: Piece,
    scale: f32,
) -> Result<()> {
    let (body, edge) = if theme.light {
        (0.08, 0.04)
    } else {
        (0.30, 0.15)
    };
    let body = solid(ctx, rgb(0x000000, body * theme.alpha))?;
    let edge = solid(ctx, rgb(0x000000, edge * theme.alpha))?;
    // Onto the base only: under the translucent piece it would dim its tint.
    masked(ctx, &outside(shape)?, || {
        offset(ctx, 3.0 * scale, || unsafe {
            ctx.FillGeometry(shape, &body, None);
            ctx.DrawGeometry(shape, &edge, 6.0 * scale, None);
        });
    });

    let fill = match piece {
        Piece::Plain => theme.piece,
        Piece::Selected => theme.piece_selected,
        Piece::Accent => theme.accent,
        Piece::Keycap => rgb(
            0xFFFFFF,
            if theme.light { 0.80 } else { 0.12 } * theme.alpha,
        ),
        Piece::Tab => rgb(
            0xFFFFFF,
            if theme.light { 0.85 } else { 0.18 } * theme.alpha,
        ),
    };
    let glow = if piece == Piece::Accent {
        // Its colored shadow, under the fill.
        let shadow = D2D1_COLOR_F {
            a: 0.45 * theme.alpha,
            ..theme.accent
        };
        unsafe { ctx.DrawGeometry(shape, &solid(ctx, shadow)?, 4.0 * scale, None) };
        let toward_white = |v: f32| v + (1.0 - v) * 0.6;
        D2D1_COLOR_F {
            r: toward_white(theme.accent.r),
            g: toward_white(theme.accent.g),
            b: toward_white(theme.accent.b),
            a: theme.glow.a,
        }
    } else {
        theme.glow
    };
    unsafe { ctx.FillGeometry(shape, &solid(ctx, fill)?, None) };
    finish(ctx, shape, bounds, theme, glow, scale)
}

/// A soft shadow around a floating window's shape (menu, cards), offset
/// downward. Masked to the outside of `shape` so it never darkens the glass.
pub fn outer_shadow(
    ctx: &ID2D1DeviceContext,
    shape: &ID2D1Geometry,
    theme: &Theme,
    scale: f32,
) -> Result<()> {
    let brush = solid(ctx, rgb(0x000000, 0.04 * theme.alpha))?;
    masked(ctx, &outside(shape)?, || {
        offset(ctx, 4.0 * scale, || {
            for width in [16.0, 10.0, 4.0] {
                unsafe { ctx.DrawGeometry(shape, &brush, width * scale, None) };
            }
        });
    });
    Ok(())
}

/// A rounded rectangle to draw glass with.
pub fn rounded_shape(
    ctx: &ID2D1DeviceContext,
    rect: D2D_RECT_F,
    corner: f32,
) -> Result<ID2D1Geometry> {
    unsafe {
        ctx.GetFactory()?
            .CreateRoundedRectangleGeometry(&D2D1_ROUNDED_RECT {
                rect,
                radiusX: corner,
                radiusY: corner,
            })?
            .cast()
    }
}

/// Inner glow, sheen, and rim, shared by layers and pieces.
fn finish(
    ctx: &ID2D1DeviceContext,
    shape: &ID2D1Geometry,
    bounds: D2D_RECT_F,
    theme: &Theme,
    glow: D2D1_COLOR_F,
    scale: f32,
) -> Result<()> {
    // Strokes centered on the edge, clipped to the shape: only their inner
    // halves show, stacking up toward the edge.
    let glow = solid(
        ctx,
        D2D1_COLOR_F {
            a: glow.a / 3.0,
            ..glow
        },
    )?;
    masked(ctx, shape, || {
        for width in [7.0, 4.0, 2.0] {
            unsafe { ctx.DrawGeometry(shape, &glow, width * scale, None) };
        }
    });

    let mid = (bounds.top + bounds.bottom) / 2.0;
    let clear = D2D1_COLOR_F {
        a: 0.0,
        ..theme.sheen
    };
    let sheen = vertical_gradient(ctx, bounds.top, mid, theme.sheen, clear)?;
    unsafe { ctx.FillGeometry(shape, &sheen, None) };

    let white = |a: f32| rgb(0xFFFFFF, a);
    let rim = gradient(
        ctx,
        (bounds.left, bounds.top),
        (bounds.right, bounds.bottom),
        &[
            (0.0, white(theme.rim[0])),
            (0.45, white(theme.rim[1])),
            (1.0, white(theme.rim[2])),
        ],
    )?;
    unsafe { ctx.DrawGeometry(shape, &rim, 1.3 * scale, None) };
    Ok(())
}

/// Runs `draw` clipped to `mask`.
fn masked(ctx: &ID2D1DeviceContext, mask: &ID2D1Geometry, draw: impl FnOnce()) {
    let params = D2D1_LAYER_PARAMETERS1 {
        // D2D1::InfiniteRect().
        contentBounds: D2D_RECT_F {
            left: -f32::MAX,
            top: -f32::MAX,
            right: f32::MAX,
            bottom: f32::MAX,
        },
        geometricMask: ManuallyDrop::new(Some(mask.clone())),
        maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
        maskTransform: Matrix3x2::identity(),
        opacity: 1.0,
        opacityBrush: ManuallyDrop::new(None),
        layerOptions: D2D1_LAYER_OPTIONS1_NONE,
    };
    unsafe { ctx.PushLayer(&params, None) };
    // The layer holds its own reference to the mask.
    drop(ManuallyDrop::into_inner(params.geometricMask));
    draw();
    unsafe { ctx.PopLayer() };
}

/// Everything but `shape`: a huge rectangle with the shape as its hole.
fn outside(shape: &ID2D1Geometry) -> Result<ID2D1Geometry> {
    unsafe {
        let factory = shape.GetFactory()?;
        let far = 1.0e6;
        let everything = factory.CreateRectangleGeometry(&D2D_RECT_F {
            left: -far,
            top: -far,
            right: far,
            bottom: far,
        })?;
        factory
            .CreateGeometryGroup(
                D2D1_FILL_MODE_ALTERNATE,
                &[Some(everything.cast()?), Some(shape.clone())],
            )?
            .cast()
    }
}

/// Runs `draw` shifted down by `dy` in the shape's own coordinates.
fn offset(ctx: &ID2D1DeviceContext, dy: f32, draw: impl FnOnce()) {
    let mut old = Matrix3x2::identity();
    unsafe {
        ctx.GetTransform(&mut old);
        ctx.SetTransform(&(Matrix3x2::translation(0.0, dy) * old));
    }
    draw();
    unsafe { ctx.SetTransform(&old) };
}

fn solid(ctx: &ID2D1DeviceContext, color: D2D1_COLOR_F) -> Result<ID2D1Brush> {
    unsafe { ctx.CreateSolidColorBrush(&color, None)?.cast() }
}

fn vertical_gradient(
    ctx: &ID2D1DeviceContext,
    y0: f32,
    y1: f32,
    top: D2D1_COLOR_F,
    bottom: D2D1_COLOR_F,
) -> Result<ID2D1Brush> {
    gradient(ctx, (0.0, y0), (0.0, y1), &[(0.0, top), (1.0, bottom)])
}

/// A linear gradient from `start` to `end` through `(position, color)` stops.
fn gradient(
    ctx: &ID2D1DeviceContext,
    start: (f32, f32),
    end: (f32, f32),
    stops: &[(f32, D2D1_COLOR_F)],
) -> Result<ID2D1Brush> {
    let stops: Vec<_> = stops
        .iter()
        .map(|&(position, color)| D2D1_GRADIENT_STOP { position, color })
        .collect();
    let point = |(x, y): (f32, f32)| Vector2 { X: x, Y: y };
    unsafe {
        // The device context's own overload takes color spaces; use the
        // classic render-target one.
        let rt: &ID2D1RenderTarget = ctx;
        let collection =
            rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?;
        let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
            startPoint: point(start),
            endPoint: point(end),
        };
        ctx.CreateLinearGradientBrush(&props, None, &collection)?
            .cast()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use swisswheel_core::appearance::Accent;

    /// `(0xRRGGBB, alpha rounded to 2 places)`.
    fn hex(c: D2D1_COLOR_F) -> (u32, f32) {
        let byte = |v: f32| (v * 255.0).round() as u32;
        (
            byte(c.r) << 16 | byte(c.g) << 8 | byte(c.b),
            (c.a * 100.0).round() / 100.0,
        )
    }

    #[test]
    fn light_and_dark_tokens() {
        let l = Theme::new(true, true, Accent::Blue, 1.0);
        assert_eq!(hex(l.tint_top), (0xFFFFFF, 0.32));
        assert_eq!(hex(l.tint_bottom), (0xFFFFFF, 0.18));
        assert_eq!(hex(l.text), (0x1D1D1F, 1.0));
        assert_eq!(hex(l.accent), (0x0A84FF, 0.85));
        assert_eq!(l.rim, [0.95, 0.10, 0.70]);
        let d = Theme::new(false, true, Accent::Green, 1.0);
        assert_eq!(hex(d.tint_top), (0x2A2A2E, 0.55));
        assert_eq!(hex(d.text), (0xF5F5F7, 1.0));
        assert_eq!(hex(d.accent), (0x1E9E50, 0.85));
        assert_eq!(d.rim.map(|r| (r * 100.0).round()), [38.0, 4.0, 28.0]);
    }

    #[test]
    fn opaque_without_glass_and_alpha_fades_everything() {
        let t = Theme::new(true, false, Accent::Red, 1.0);
        assert_eq!(hex(t.opaque), (0xF2F2F5, 1.0));
        assert_eq!(hex(t.accent).1, 1.0);
        let half = Theme::new(true, true, Accent::Blue, 0.5);
        assert_eq!(hex(half.text).1, 0.5);
        assert_eq!(hex(half.failure), (0xE5484D, 0.5));
    }

    /// A device context drawing to a `width`×`height` bitmap.
    fn canvas(
        width: u32,
        height: u32,
    ) -> Result<(ID2D1Factory1, ID2D1DeviceContext, ID2D1Bitmap1)> {
        use windows::Win32::Graphics::Dxgi::IDXGIDevice;
        unsafe {
            let device: IDXGIDevice = crate::wheel::render::create_d3d_device()?.cast()?;
            let factory: ID2D1Factory1 =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let ctx = factory
                .CreateDevice(&device)?
                .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            let target = ctx.CreateBitmap(
                D2D_SIZE_U { width, height },
                None,
                0,
                &bitmap_props(D2D1_BITMAP_OPTIONS_TARGET),
            )?;
            ctx.SetTarget(&target);
            Ok((factory, ctx, target))
        }
    }

    fn bitmap_props(options: D2D1_BITMAP_OPTIONS) -> D2D1_BITMAP_PROPERTIES1 {
        use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
        D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: options,
            ..Default::default()
        }
    }

    /// The red channel of the pixel at `(x, y)`, after `EndDraw`.
    fn red_at(ctx: &ID2D1DeviceContext, target: &ID2D1Bitmap1, x: u32, y: u32) -> Result<u8> {
        unsafe {
            let read = ctx.CreateBitmap(
                target.GetPixelSize(),
                None,
                0,
                &bitmap_props(D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW),
            )?;
            read.CopyFromBitmap(None, target, None)?;
            let map = read.Map(D2D1_MAP_OPTIONS_READ)?;
            let red = *map.bits.add((y * map.pitch + x * 4 + 2) as usize); // BGRA
            read.Unmap()?;
            Ok(red)
        }
    }

    /// Every routine on a real device context; D2D reports misused layers or
    /// brushes from `EndDraw`.
    #[test]
    fn draws_on_a_device_context() -> Result<()> {
        let (_factory, ctx, _target) = canvas(200, 120)?;
        let rect = D2D_RECT_F {
            left: 20.0,
            top: 20.0,
            right: 180.0,
            bottom: 100.0,
        };
        let shape = rounded_shape(&ctx, rect, 16.0)?;
        unsafe {
            ctx.BeginDraw();
            ctx.SetTransform(&Matrix3x2::translation(0.5, 0.5));
            for theme in [
                Theme::new(true, true, Accent::Blue, 1.0),
                Theme::new(false, false, Accent::Green, 0.5),
                Theme::current(true, Accent::Graphite, 1.0),
            ] {
                outer_shadow(&ctx, &shape, &theme, 1.25)?;
                glass_layer(&ctx, &shape, rect, &theme, 1.25)?;
                for piece in [
                    Piece::Plain,
                    Piece::Selected,
                    Piece::Accent,
                    Piece::Keycap,
                    Piece::Tab,
                ] {
                    glass_piece(&ctx, &shape, rect, &theme, piece, 1.25)?;
                }
            }
            ctx.EndDraw(None, None)
        }
    }

    /// The short shadow falls below the piece onto the base, never under the
    /// translucent piece itself.
    #[test]
    fn piece_shadow_stays_outside_the_piece() -> Result<()> {
        let (_factory, ctx, target) = canvas(120, 120)?;
        let rect = D2D_RECT_F {
            left: 20.0,
            top: 20.0,
            right: 100.0,
            bottom: 100.0,
        };
        let theme = Theme::new(false, true, Accent::Blue, 1.0);
        let base = 0x2A;
        unsafe {
            ctx.BeginDraw();
            ctx.Clear(Some(&rgb(0x2A2A2E, 1.0)));
            glass_piece(
                &ctx,
                &rounded_shape(&ctx, rect, 9.0)?,
                rect,
                &theme,
                Piece::Plain,
                1.0,
            )?;
            ctx.EndDraw(None, None)?;
        }
        // Below the sheen, inside the glow: just the base under white α .08.
        let lifted = base as f32 + (255.0 - base as f32) * 0.08;
        let inside = red_at(&ctx, &target, 60, 88)?;
        assert!(
            (f32::from(inside) - lifted).abs() <= 2.0,
            "inside {inside}, expected ≈{lifted}"
        );
        // The 3 DIP strip just below the piece is shadowed.
        assert!(red_at(&ctx, &target, 60, 102)? < base);
        Ok(())
    }
}
