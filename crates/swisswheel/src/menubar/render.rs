//! Draws the desktop menu: translucent square tiles (icon above, label below)
//! on a glass bar, and the settings panel under it.

use swisswheel_core::menu::{Focus, MenuHit, TILES, Tile};
use swisswheel_core::menu_geometry::{Box2, DROP, MenuLayout, PANEL_PAD};
use swisswheel_core::settings::CATEGORIES;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::core::{Interface, PCWSTR, Result, w};
use windows_numerics::Vector2;

use super::MenuView;
use crate::surface::{Surface, draw_text, inset, rounded};
use crate::wheel::render::{Palette, light_theme, palette, transparency_enabled};

/// Rounding of the bar, the panel, tiles, and the selected row (DIPs).
pub(super) const BAR_CORNER: f32 = 12.0;
pub(super) const PANEL_CORNER: f32 = 10.0;
const TILE_CORNER: f32 = 8.0;
const ROW_CORNER: f32 = 8.0;
const KEYCAP_CORNER: f32 = 6.0;
/// The selected tab's underline stops this far from the tab's sides.
const TAB_UNDERLINE_INSET: f32 = 16.0;
/// Unselected tiles are see-through, like the reference's grey squares.
const TILE_ALPHA: f32 = 0.55;
const TILE_HOVER_ALPHA: f32 = 0.8;

const FONT: PCWSTR = w!("Segoe UI Variable Text");
const GLYPH_BACK: &str = "\u{E76B}"; // ChevronLeft
const GLYPH_FORWARD: &str = "\u{E76C}"; // ChevronRight
const GLYPH_WARNING: &str = "\u{E7BA}"; // Warning

fn glyph(tile: Tile) -> &'static str {
    match tile {
        Tile::Settings => "\u{E713}", // Settings
        Tile::Plugins => "\u{EA86}",  // Puzzle
        Tile::Update => "\u{E895}",   // Sync
    }
}

fn rect(b: Box2, dy: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left: b.left,
        top: b.top + dy,
        right: b.right,
        bottom: b.bottom + dy,
    }
}

fn faded(c: D2D1_COLOR_F, by: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { a: c.a * by, ..c }
}

pub(super) fn draw(
    surface: &Surface,
    view: &MenuView,
    layout: &MenuLayout,
    scale: f32,
    glass_supported: bool,
) -> Result<()> {
    let glass = glass_supported && transparency_enabled();
    let light = light_theme();
    let colors = palette(light, glass, view.progress);
    // The bar slides down into place while it opens.
    let dy = -DROP * scale * (1.0 - view.progress);
    let bar = rect(layout.bar, dy);
    surface.set_glass(0, glass.then_some(bar), BAR_CORNER * scale)?;
    let panel = view.rows.map(|_| rect(layout.panel, 0.0));
    surface.set_glass(1, panel.filter(|_| glass), PANEL_CORNER * scale)?;

    let icon = surface.text_format(
        surface.icon_font(),
        34.0 * scale,
        DWRITE_FONT_WEIGHT_NORMAL,
        DWRITE_TEXT_ALIGNMENT_CENTER,
    )?;
    let label = surface.text_format(
        FONT,
        13.0 * scale,
        DWRITE_FONT_WEIGHT_SEMI_BOLD,
        DWRITE_TEXT_ALIGNMENT_CENTER,
    )?;
    let ctx = surface.ctx();
    unsafe {
        ctx.BeginDraw();
        ctx.Clear(Some(&D2D1_COLOR_F::default()));
        let brush = |color: &D2D1_COLOR_F| ctx.CreateSolidColorBrush(color, None);

        ctx.FillRoundedRectangle(&rounded(bar, BAR_CORNER * scale), &brush(&colors.disc)?);
        ctx.DrawRoundedRectangle(
            &rounded(inset(bar, 0.5 * scale), BAR_CORNER * scale),
            &brush(&colors.rim)?,
            scale,
            None,
        );
        for (i, &tile) in TILES.iter().enumerate() {
            let r = rect(layout.tiles[i], dy);
            let selected = i == view.selected;
            let alpha = if view.hovered == MenuHit::Tile(i) {
                TILE_HOVER_ALPHA
            } else {
                TILE_ALPHA
            };
            let (top, bottom, text) = if selected {
                (colors.accent_top, colors.accent_bottom, colors.accent_text)
            } else {
                (
                    faded(colors.tile_top, alpha),
                    faded(colors.tile_bottom, alpha),
                    colors.text,
                )
            };
            let fill = vertical_gradient(ctx, r.top, r.bottom, &top, &bottom)?;
            ctx.FillRoundedRectangle(&rounded(r, TILE_CORNER * scale), &fill);
            let text = brush(&text)?;
            let split = r.top + (r.bottom - r.top) * 0.62;
            let pad = 10.0 * scale;
            draw_text(
                ctx,
                glyph(tile),
                &icon,
                D2D_RECT_F {
                    top: r.top + pad,
                    bottom: split,
                    ..r
                },
                &text,
            );
            draw_text(
                ctx,
                tile.label(),
                &label,
                D2D_RECT_F {
                    top: split,
                    bottom: r.bottom - pad,
                    ..r
                },
                &text,
            );
        }

        if let (Some(rows), Some(panel)) = (view.rows, panel) {
            let colors = palette(light, glass, view.panel_progress);
            draw_panel(surface, view, rows, layout, panel, &colors, scale)?;
        }
        ctx.EndDraw(None, None)?;
    }
    surface.present()
}

fn draw_panel(
    surface: &Surface,
    view: &MenuView,
    rows: &[super::RowView],
    layout: &MenuLayout,
    panel: D2D_RECT_F,
    colors: &Palette,
    scale: f32,
) -> Result<()> {
    let ctx = surface.ctx();
    let leading = surface.text_format(
        FONT,
        13.0 * scale,
        DWRITE_FONT_WEIGHT_NORMAL,
        DWRITE_TEXT_ALIGNMENT_LEADING,
    )?;
    let small = surface.text_format(
        FONT,
        12.0 * scale,
        DWRITE_FONT_WEIGHT_NORMAL,
        DWRITE_TEXT_ALIGNMENT_LEADING,
    )?;
    let value = surface.text_format(
        FONT,
        13.0 * scale,
        DWRITE_FONT_WEIGHT_SEMI_BOLD,
        DWRITE_TEXT_ALIGNMENT_CENTER,
    )?;
    let icons = surface.text_format(
        surface.icon_font(),
        12.0 * scale,
        DWRITE_FONT_WEIGHT_NORMAL,
        DWRITE_TEXT_ALIGNMENT_CENTER,
    )?;
    let pad = 12.0 * scale;
    unsafe {
        let brush = |color: &D2D1_COLOR_F| ctx.CreateSolidColorBrush(color, None);
        // A hairline across the panel's inside at `y`.
        let divider = |y: f32| -> Result<()> {
            let line = D2D_RECT_F {
                left: panel.left + PANEL_PAD * scale,
                top: y - 0.5 * scale,
                right: panel.right - PANEL_PAD * scale,
                bottom: y + 0.5 * scale,
            };
            ctx.FillRectangle(&line, &brush(&colors.rim)?);
            Ok(())
        };
        ctx.FillRoundedRectangle(&rounded(panel, PANEL_CORNER * scale), &brush(&colors.disc)?);
        ctx.DrawRoundedRectangle(
            &rounded(inset(panel, 0.5 * scale), PANEL_CORNER * scale),
            &brush(&colors.rim)?,
            scale,
            None,
        );
        if let (Some(text), Some(b)) = (view.notice, layout.notice) {
            let r = rect(b, 0.0);
            draw_text(
                ctx,
                text,
                &small,
                D2D_RECT_F {
                    left: r.left + pad,
                    ..r
                },
                &brush(&colors.subtext)?,
            );
        }

        for (i, &category) in CATEGORIES.iter().enumerate() {
            let r = rect(layout.tabs[i], 0.0);
            let selected = i == view.category;
            let focused = selected && view.focus == Some(Focus::Tabs);
            if focused {
                let fill = vertical_gradient(
                    ctx,
                    r.top,
                    r.bottom,
                    &colors.accent_top,
                    &colors.accent_bottom,
                )?;
                ctx.FillRoundedRectangle(&rounded(r, ROW_CORNER * scale), &fill);
            } else if view.hovered == MenuHit::Tab(i) {
                let fill = brush(&faded(colors.tile_top, 0.35))?;
                ctx.FillRoundedRectangle(&rounded(r, ROW_CORNER * scale), &fill);
            }
            let text = if focused {
                colors.accent_text
            } else if selected {
                colors.text
            } else {
                colors.subtext
            };
            draw_text(ctx, category.label(), &value, r, &brush(&text)?);
            if selected && !focused {
                let underline = D2D_RECT_F {
                    left: r.left + TAB_UNDERLINE_INSET * scale,
                    top: r.bottom - 4.0 * scale,
                    right: r.right - TAB_UNDERLINE_INSET * scale,
                    bottom: r.bottom - 2.0 * scale,
                };
                ctx.FillRectangle(&underline, &brush(&colors.accent_top)?);
            }
        }
        divider(layout.tabs[0].bottom)?;

        for (i, (row, boxes)) in rows.iter().zip(&layout.rows).enumerate() {
            let r = rect(boxes.row, 0.0);
            let selected = view.focus == Some(Focus::Row(i));
            let hovered = matches!(
                view.hovered,
                MenuHit::Row(j) | MenuHit::Step(j, _) | MenuHit::KeyCap(j) if j == i
            );
            if selected {
                let fill = vertical_gradient(
                    ctx,
                    r.top,
                    r.bottom,
                    &colors.accent_top,
                    &colors.accent_bottom,
                )?;
                ctx.FillRoundedRectangle(&rounded(r, ROW_CORNER * scale), &fill);
            } else if hovered {
                let fill = brush(&faded(colors.tile_top, 0.35))?;
                ctx.FillRoundedRectangle(&rounded(r, ROW_CORNER * scale), &fill);
            }
            let (text, dim) = if selected {
                (colors.accent_text, faded(colors.accent_text, 0.4))
            } else {
                (colors.text, faded(colors.subtext, 0.6))
            };
            let text_brush = brush(&text)?;
            let back = rect(boxes.back, 0.0);
            let forward = rect(boxes.forward, 0.0);
            let warn_width = 22.0 * scale;
            draw_text(
                ctx,
                row.label,
                &leading,
                D2D_RECT_F {
                    left: r.left + pad,
                    right: back.left - warn_width,
                    ..r
                },
                &text_brush,
            );
            if row.failed {
                draw_text(
                    ctx,
                    GLYPH_WARNING,
                    &icons,
                    D2D_RECT_F {
                        left: back.left - warn_width,
                        right: back.left,
                        ..r
                    },
                    &brush(if selected {
                        &colors.accent_text
                    } else {
                        &colors.accent_top
                    })?,
                );
            }
            if row.keycap {
                let cap = inset(rect(boxes.keycap, 0.0), 4.0 * scale);
                let waiting = selected && view.capturing;
                // On the selected row's accent fill a rim-colored outline
                // would vanish, so it takes the row's text color there; a
                // key being waited for gets a darker inside and a thicker one.
                let (stroke, width) = match (waiting, selected) {
                    (true, _) => (text, 2.0 * scale),
                    (false, true) => (text, scale),
                    (false, false) => (colors.rim, scale),
                };
                if waiting {
                    ctx.FillRoundedRectangle(
                        &rounded(cap, KEYCAP_CORNER * scale),
                        &brush(&faded(colors.disc, 0.6))?,
                    );
                }
                ctx.DrawRoundedRectangle(
                    &rounded(inset(cap, width / 2.0), KEYCAP_CORNER * scale),
                    &brush(&stroke)?,
                    width,
                    None,
                );
                let label = if waiting {
                    "입력…"
                } else {
                    row.value.as_str()
                };
                draw_text(ctx, label, &value, cap, &text_brush);
                continue;
            }
            let step_brush = |enabled: bool| brush(if enabled { &text } else { &dim });
            draw_text(ctx, GLYPH_BACK, &icons, back, &step_brush(row.back)?);
            draw_text(
                ctx,
                GLYPH_FORWARD,
                &icons,
                forward,
                &step_brush(row.forward)?,
            );
            draw_text(
                ctx,
                &row.value,
                &value,
                D2D_RECT_F {
                    left: back.right,
                    right: forward.left,
                    ..r
                },
                &text_brush,
            );
        }

        let hint = rect(layout.hint, 0.0);
        divider(hint.top)?;
        draw_text(
            ctx,
            view.hint,
            &small,
            D2D_RECT_F {
                left: hint.left + pad,
                ..hint
            },
            &brush(&colors.subtext)?,
        );
    }
    Ok(())
}

fn vertical_gradient(
    ctx: &ID2D1DeviceContext,
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
        let rt: &ID2D1RenderTarget = ctx;
        let collection =
            rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?;
        let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
            startPoint: Vector2 { X: 0.0, Y: y0 },
            endPoint: Vector2 { X: 0.0, Y: y1 },
        };
        ctx.CreateLinearGradientBrush(&props, None, &collection)?
            .cast()
    }
}
