//! Draws the desktop menu: glass tiles (icon above, label below) on a glass
//! bar, and the settings panel under it (spec 2.4).

use swisswheel_core::menu::{DownloadFocus, Focus, MenuHit, TILES, Tile};
use swisswheel_core::menu_geometry::{Box2, DROP, MenuLayout, essential_button};
use swisswheel_core::settings::CATEGORIES;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::core::Result;

use super::{DownloadView, MenuView, RowView};
use crate::fonts;
use crate::surface::{Surface, draw_text, inset, rounded};
use crate::theme::{Piece, Theme, glass_layer, glass_piece, outer_shadow, rounded_shape};

/// Rounding of the bar, the panel, and what is on them (DIPs).
pub(super) const BAR_CORNER: f32 = 22.0;
pub(super) const PANEL_CORNER: f32 = 20.0;
const TILE_CORNER: f32 = 16.0;
const ROW_CORNER: f32 = 11.0;
const KEYCAP_CORNER: f32 = 7.0;
/// The groove behind the tabs, and the selected tab's pill inside it.
const GROOVE_CORNER: f32 = 12.0;
const PILL_CORNER: f32 = 9.0;
const PILL_INSET: f32 = 3.0;
/// The accent rim on the selected tab while the keyboard is on the tabs.
const FOCUS_RIM: f32 = 1.5;
/// The accent row's color dot and its gap to the value.
const SWATCH: f32 = 10.0;
const SWATCH_GAP: f32 = 6.0;

const GLYPH_BACK: &str = "\u{E76B}"; // ChevronLeft
const GLYPH_FORWARD: &str = "\u{E76C}"; // ChevronRight
const GLYPH_WARNING: &str = "\u{E7BA}"; // Warning

fn glyph(tile: Tile) -> &'static str {
    match tile {
        Tile::Settings => "\u{E713}", // Settings
        Tile::Download => "\u{E896}", // Download
        Tile::Plugins => "\u{EA86}",  // Puzzle
        Tile::Update => "\u{E895}",   // Sync
        Tile::Restart => "\u{E72C}",  // Refresh
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
    let theme = Theme::current(glass_supported, view.accent, view.progress);
    // The bar slides down into place while it opens.
    let dy = -DROP * scale * (1.0 - view.progress);
    let bar = rect(layout.bar, dy);
    surface.set_glass(0, theme.glass.then_some(bar), BAR_CORNER * scale)?;
    let panel_open = view.rows.is_some() || view.download.is_some();
    let panel = panel_open.then(|| rect(layout.panel, 0.0));
    surface.set_glass(1, panel.filter(|_| theme.glass), PANEL_CORNER * scale)?;

    let icon = fonts::icon_format(surface.icon_font(), 34.0 * scale)?;
    let label = fonts::trimmed_format(
        13.0 * scale,
        DWRITE_FONT_WEIGHT_SEMI_BOLD,
        DWRITE_TEXT_ALIGNMENT_CENTER,
    )?;
    let ctx = surface.ctx();
    unsafe {
        ctx.BeginDraw();
        ctx.Clear(Some(&D2D1_COLOR_F::default()));
        let brush = |color: &D2D1_COLOR_F| ctx.CreateSolidColorBrush(color, None);

        let shape = rounded_shape(ctx, bar, BAR_CORNER * scale)?;
        outer_shadow(ctx, &shape, &theme, scale)?;
        glass_layer(ctx, &shape, bar, &theme, scale)?;
        let (text, on_accent) = (brush(&theme.text)?, brush(&theme.on_accent)?);
        for (i, &tile) in TILES.iter().enumerate() {
            let r = rect(layout.tiles[i], dy);
            let piece = if i == view.selected {
                Piece::Accent
            } else if view.hovered == MenuHit::Tile(i) {
                Piece::Selected
            } else {
                Piece::Plain
            };
            let shape = rounded_shape(ctx, r, TILE_CORNER * scale)?;
            glass_piece(ctx, &shape, r, &theme, piece, scale)?;
            let text = if piece == Piece::Accent {
                &on_accent
            } else {
                &text
            };
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
                text,
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
                text,
            );
        }

        if let Some(panel) = panel {
            let theme = Theme::new(theme.light, theme.glass, view.accent, view.panel_progress);
            if let Some(rows) = view.rows {
                draw_panel(surface, view, rows, layout, panel, &theme, scale)?;
            } else if let Some(download) = &view.download {
                draw_download_panel(surface, view, download, layout, panel, &theme, scale)?;
            }
        }
        ctx.EndDraw(None, None)?;
    }
    surface.present()
}

fn draw_panel(
    surface: &Surface,
    view: &MenuView,
    rows: &[RowView],
    layout: &MenuLayout,
    panel: D2D_RECT_F,
    theme: &Theme,
    scale: f32,
) -> Result<()> {
    let ctx = surface.ctx();
    let name = fonts::trimmed_format(
        13.0 * scale,
        DWRITE_FONT_WEIGHT_SEMI_BOLD,
        DWRITE_TEXT_ALIGNMENT_LEADING,
    )?;
    let small = fonts::trimmed_format(
        12.0 * scale,
        DWRITE_FONT_WEIGHT_NORMAL,
        DWRITE_TEXT_ALIGNMENT_LEADING,
    )?;
    let tab = fonts::text_format(
        13.0 * scale,
        DWRITE_FONT_WEIGHT_SEMI_BOLD,
        DWRITE_TEXT_ALIGNMENT_CENTER,
    )?;
    let value = fonts::trimmed_format(
        13.0 * scale,
        DWRITE_FONT_WEIGHT_BOLD,
        DWRITE_TEXT_ALIGNMENT_CENTER,
    )?;
    let chevrons = fonts::icon_format(surface.icon_font(), 10.0 * scale)?;
    let icons = fonts::icon_format(surface.icon_font(), 12.0 * scale)?;
    let pad = 12.0 * scale;
    unsafe {
        let brush = |color: &D2D1_COLOR_F| ctx.CreateSolidColorBrush(color, None);
        let (text, subtext) = (brush(&theme.text)?, brush(&theme.subtext)?);
        panel_backdrop(surface, view, layout, panel, theme, &small, scale)?;

        // Tabs: a segmented control, the selected tab a pill in the groove;
        // with the keyboard on the tabs the pill takes an accent rim and label.
        let tabs_focused = view.focus == Some(Focus::Tabs);
        let accent = brush(&theme.accent)?;
        let groove = D2D_RECT_F {
            right: layout.tabs[CATEGORIES.len() - 1].right,
            ..rect(layout.tabs[0], 0.0)
        };
        ctx.FillRoundedRectangle(
            &rounded(groove, GROOVE_CORNER * scale),
            &brush(&theme.line)?,
        );
        for (i, &category) in CATEGORIES.iter().enumerate() {
            let r = rect(layout.tabs[i], 0.0);
            let selected = i == view.category;
            if selected {
                let pill = inset(r, PILL_INSET * scale);
                let shape = rounded_shape(ctx, pill, PILL_CORNER * scale)?;
                glass_piece(ctx, &shape, pill, theme, Piece::Tab, scale)?;
                if tabs_focused {
                    let rim = inset(pill, FOCUS_RIM * scale / 2.0);
                    let rim = rounded(rim, (PILL_CORNER - FOCUS_RIM / 2.0) * scale);
                    ctx.DrawRoundedRectangle(&rim, &accent, FOCUS_RIM * scale, None);
                }
            }
            let color = if selected && tabs_focused {
                &accent
            } else if selected || view.hovered == MenuHit::Tab(i) {
                &text
            } else {
                &subtext
            };
            draw_text(ctx, category.label(), &tab, r, color);
        }

        for (i, (row, boxes)) in rows.iter().zip(&layout.rows).enumerate() {
            let r = rect(boxes.row, 0.0);
            let selected = view.focus == Some(Focus::Row(i));
            let hovered = matches!(
                view.hovered,
                MenuHit::Row(j) | MenuHit::Step(j, _) | MenuHit::KeyCap(j) if j == i
            );
            if selected {
                let shape = rounded_shape(ctx, r, ROW_CORNER * scale)?;
                glass_piece(ctx, &shape, r, theme, Piece::Selected, scale)?;
            } else if hovered {
                // Flat, so it never reads as the selected row.
                ctx.FillRoundedRectangle(&rounded(r, ROW_CORNER * scale), &brush(&theme.line)?);
            }
            let back = rect(boxes.back, 0.0);
            let forward = rect(boxes.forward, 0.0);
            let warn_width = 22.0 * scale;
            draw_text(
                ctx,
                row.label,
                &name,
                D2D_RECT_F {
                    left: r.left + pad,
                    right: back.left - warn_width,
                    ..r
                },
                &text,
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
                    &brush(&theme.failure)?,
                );
            }
            if row.keycap {
                let cap = inset(rect(boxes.keycap, 0.0), 4.0 * scale);
                // A key being waited for turns into an accent piece.
                let waiting = selected && view.capturing;
                let piece = if waiting {
                    Piece::Accent
                } else {
                    Piece::Keycap
                };
                let shape = rounded_shape(ctx, cap, KEYCAP_CORNER * scale)?;
                glass_piece(ctx, &shape, cap, theme, piece, scale)?;
                let (label, color) = if waiting {
                    ("입력…", brush(&theme.on_accent)?)
                } else {
                    (row.value.as_str(), text.clone())
                };
                draw_text(ctx, label, &value, cap, &color);
                continue;
            }
            let step = |enabled: bool| {
                brush(&if enabled {
                    theme.subtext
                } else {
                    faded(theme.subtext, 0.4)
                })
            };
            draw_text(ctx, GLYPH_BACK, &chevrons, back, &step(row.back)?);
            draw_text(ctx, GLYPH_FORWARD, &chevrons, forward, &step(row.forward)?);
            let value_box = D2D_RECT_F {
                left: back.right,
                right: forward.left,
                ..r
            };
            match row.swatch {
                Some(swatch) => draw_swatched(
                    ctx,
                    &row.value,
                    &value,
                    value_box,
                    faded(swatch, theme.alpha),
                    &text,
                    scale,
                )?,
                None => draw_text(ctx, &row.value, &value, value_box, &text),
            }
        }
    }
    Ok(())
}

/// The panel's glass, shadow, and the config notice above its first line.
fn panel_backdrop(
    surface: &Surface,
    view: &MenuView,
    layout: &MenuLayout,
    panel: D2D_RECT_F,
    theme: &Theme,
    small: &IDWriteTextFormat,
    scale: f32,
) -> Result<()> {
    let ctx = surface.ctx();
    unsafe {
        let shape = rounded_shape(ctx, panel, PANEL_CORNER * scale)?;
        outer_shadow(ctx, &shape, theme, scale)?;
        glass_layer(ctx, &shape, panel, theme, scale)?;
        if let (Some(notice), Some(b)) = (view.notice, layout.notice) {
            let r = rect(b, 0.0);
            draw_text(
                ctx,
                notice,
                small,
                D2D_RECT_F {
                    left: r.left + 12.0 * scale,
                    ..r
                },
                &ctx.CreateSolidColorBrush(&theme.subtext, None)?,
            );
        }
    }
    Ok(())
}

/// The download panel (download panel spec 2.2): a header line, then one
/// row per essential with its name, status, and buttons from the right.
fn draw_download_panel(
    surface: &Surface,
    view: &MenuView,
    download: &DownloadView,
    layout: &MenuLayout,
    panel: D2D_RECT_F,
    theme: &Theme,
    scale: f32,
) -> Result<()> {
    let ctx = surface.ctx();
    let name = fonts::trimmed_format(
        13.0 * scale,
        DWRITE_FONT_WEIGHT_SEMI_BOLD,
        DWRITE_TEXT_ALIGNMENT_LEADING,
    )?;
    let small = fonts::trimmed_format(
        12.0 * scale,
        DWRITE_FONT_WEIGHT_NORMAL,
        DWRITE_TEXT_ALIGNMENT_LEADING,
    )?;
    let button = fonts::trimmed_format(
        12.0 * scale,
        DWRITE_FONT_WEIGHT_SEMI_BOLD,
        DWRITE_TEXT_ALIGNMENT_CENTER,
    )?;
    let pad = 12.0 * scale;
    let gap = 8.0 * scale;
    unsafe {
        let brush = |color: &D2D1_COLOR_F| ctx.CreateSolidColorBrush(color, None);
        let (text, subtext) = (brush(&theme.text)?, brush(&theme.subtext)?);
        let (accent, line) = (brush(&theme.accent)?, brush(&theme.line)?);
        panel_backdrop(surface, view, layout, panel, theme, &small, scale)?;
        let header = rect(layout.download_header, 0.0);
        draw_text(
            ctx,
            &download.header,
            &small,
            D2D_RECT_F {
                left: header.left + pad,
                ..header
            },
            &subtext,
        );

        for (i, (row, boxes)) in download.rows.iter().zip(&layout.rows).enumerate() {
            let r = rect(boxes.row, 0.0);
            let focus = match download.focus {
                Some(DownloadFocus::Row { essential, button }) if essential == i => Some(button),
                _ => None,
            };
            let hovered = matches!(
                view.hovered,
                MenuHit::EssentialRow(j) | MenuHit::EssentialButton(j, _) if j == i
            );
            if focus.is_some() {
                let shape = rounded_shape(ctx, r, ROW_CORNER * scale)?;
                glass_piece(ctx, &shape, r, theme, Piece::Selected, scale)?;
            } else if hovered {
                ctx.FillRoundedRectangle(&rounded(r, ROW_CORNER * scale), &line);
            }
            let count = row.buttons.len();
            // Text stops where the leftmost button starts.
            let text_right =
                essential_button(layout, i, count, 0).map_or(r.right - pad, |b| b.left - gap);
            let label_right = r.left + pad + 70.0 * scale;
            draw_text(
                ctx,
                row.label,
                &name,
                D2D_RECT_F {
                    left: r.left + pad,
                    right: label_right,
                    ..r
                },
                &text,
            );
            let purpose_right = label_right + 80.0 * scale;
            draw_text(
                ctx,
                row.purpose,
                &small,
                D2D_RECT_F {
                    left: label_right,
                    right: purpose_right,
                    ..r
                },
                &subtext,
            );
            let status = D2D_RECT_F {
                left: purpose_right,
                right: text_right,
                ..r
            };
            draw_text(ctx, &row.status, &small, status, &text);
            if let Some(fraction) = row.progress {
                // A thin bar under the status text.
                let bottom = r.bottom - 6.0 * scale;
                let track = D2D_RECT_F {
                    top: bottom - 2.0 * scale,
                    bottom,
                    ..status
                };
                ctx.FillRectangle(&track, &line);
                let filled = D2D_RECT_F {
                    right: track.left + (track.right - track.left) * fraction.clamp(0.0, 1.0),
                    ..track
                };
                ctx.FillRectangle(&filled, &accent);
            }
            for (b, label) in row.buttons.iter().enumerate() {
                let Some(boxed) = essential_button(layout, i, count, b) else {
                    continue;
                };
                let cap = inset(rect(boxed, 0.0), 4.0 * scale);
                let shape = rounded_shape(ctx, cap, KEYCAP_CORNER * scale)?;
                glass_piece(ctx, &shape, cap, theme, Piece::Keycap, scale)?;
                let selected = focus == Some(Some(b));
                if selected {
                    let rim = inset(cap, FOCUS_RIM * scale / 2.0);
                    let rim = rounded(rim, (KEYCAP_CORNER - FOCUS_RIM / 2.0) * scale);
                    ctx.DrawRoundedRectangle(&rim, &accent, FOCUS_RIM * scale, None);
                }
                let color = if selected {
                    &accent
                } else if view.hovered == MenuHit::EssentialButton(i, b) {
                    &text
                } else {
                    &subtext
                };
                draw_text(ctx, label, &button, cap, color);
            }
        }
    }
    Ok(())
}

/// `value` centered in `r` with a `swatch` dot before it.
fn draw_swatched(
    ctx: &ID2D1DeviceContext,
    value: &str,
    format: &IDWriteTextFormat,
    r: D2D_RECT_F,
    swatch: D2D1_COLOR_F,
    text: &ID2D1SolidColorBrush,
    scale: f32,
) -> Result<()> {
    let (dot, gap) = (SWATCH * scale, SWATCH_GAP * scale);
    let (width, _) = fonts::text_size(value, format)?;
    let width = width.min(r.right - r.left - dot - gap);
    let left = (r.left + r.right - dot - gap - width) / 2.0;
    let mid = (r.top + r.bottom) / 2.0;
    unsafe {
        ctx.FillEllipse(
            &D2D1_ELLIPSE {
                point: windows_numerics::Vector2 {
                    X: left + dot / 2.0,
                    Y: mid,
                },
                radiusX: dot / 2.0,
                radiusY: dot / 2.0,
            },
            &ctx.CreateSolidColorBrush(&swatch, None)?,
        );
    }
    let label = D2D_RECT_F {
        left: left + dot + gap,
        // A pixel of slack so the measured width never trims.
        right: left + dot + gap + width + 1.0,
        ..r
    };
    draw_text(ctx, value, format, label, text);
    Ok(())
}
