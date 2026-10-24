//! Draws the editor panel (spec 2.2–2.4) on the menu's glass: the toolbar,
//! the ratio chips in crop mode, the preview with its crop overlay, and the
//! footer. A frame is a pure function of the state it is given.

use std::time::Instant;

use ::image::RgbaImage;
use swisswheel_core::crop::CHIPS;
use swisswheel_core::editor::{PixelRect, text};
use swisswheel_core::editor_control::EditorState;
use swisswheel_core::editor_geometry::{CORNER, EditorHit, EditorLayout, EditorTool, Fit, TOOLS};
use swisswheel_core::menu_geometry::Box2;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::core::Result;
use windows_numerics::{Matrix3x2, Vector2};

use crate::fonts;
use crate::surface::{Surface, draw_text, rounded};
use crate::theme::{Piece, Theme, glass_layer, glass_piece, outer_shadow, rounded_shape};

/// Rounding of the buttons and chips (DIPs).
const BUTTON_CORNER: f32 = 9.0;
/// Tool icon size and its gap to the label (DIPs).
const ICON: f32 = 16.0;
const ICON_GAP: f32 = 6.0;
/// Side of a crop handle square (DIPs).
const HANDLE: f32 = 8.0;
/// Opacity of a control that can't be used now.
const DIMMED: f32 = 0.4;

const GLYPH_UNDO: &str = "\u{E7A7}"; // Undo
const GLYPH_REDO: &str = "\u{E7A6}"; // Redo

/// Code points present in both Segoe Fluent Icons and Segoe MDL2 Assets.
fn glyph(tool: EditorTool) -> &'static str {
    match tool {
        EditorTool::Crop => "\u{E7A8}",   // Crop
        EditorTool::Rotate => "\u{E7AD}", // Rotate
        EditorTool::FlipH => "\u{E8AB}",  // Switch: arrows left and right
        EditorTool::FlipV => "\u{E8CB}",  // Sort: arrows up and down
        EditorTool::Gray => "\u{E793}",   // Contrast: a half-filled circle
        EditorTool::Info => "\u{EA39}",   // Blocked2: a slashed circle
    }
}

fn label(tool: EditorTool) -> &'static str {
    match tool {
        EditorTool::Crop => "자르기",
        EditorTool::Rotate => "회전",
        EditorTool::FlipH => "좌우",
        EditorTool::FlipV => "상하",
        EditorTool::Gray => "흑백",
        EditorTool::Info => "정보",
    }
}

/// Whether the info button is shown: only once loaded, and only for the
/// formats whose info can be stripped (spec 2.6).
pub(super) fn info_shown(state: &EditorState) -> bool {
    state.session().is_some_and(|s| s.strip_available())
}

/// The decoded image as GPU bitmaps, in color and in gray.
pub struct Previews {
    color: ID2D1Bitmap1,
    gray: ID2D1Bitmap1,
}

pub fn previews(surface: &Surface, color: &RgbaImage, gray: &RgbaImage) -> Result<Previews> {
    Ok(Previews {
        color: bitmap(surface.ctx(), color)?,
        gray: bitmap(surface.ctx(), gray)?,
    })
}

/// `image` as a premultiplied BGRA bitmap.
fn bitmap(ctx: &ID2D1DeviceContext, image: &RgbaImage) -> Result<ID2D1Bitmap1> {
    let bgra: Vec<u8> = image
        .as_raw()
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|&[r, g, b, a]| {
            let times_alpha = |c: u8| ((u16::from(c) * u16::from(a) + 127) / 255) as u8;
            [times_alpha(b), times_alpha(g), times_alpha(r), a]
        })
        .collect();
    let props = D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
        ..Default::default()
    };
    unsafe {
        ctx.CreateBitmap(
            D2D_SIZE_U {
                width: image.width(),
                height: image.height(),
            },
            Some(bgra.as_ptr().cast()),
            image.width() * 4,
            &props,
        )
    }
}

fn rect(b: Box2) -> D2D_RECT_F {
    D2D_RECT_F {
        left: b.left,
        top: b.top,
        right: b.right,
        bottom: b.bottom,
    }
}

fn faded(c: D2D1_COLOR_F, by: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { a: c.a * by, ..c }
}

fn point(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

/// Draws one frame and presents it.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    surface: &Surface,
    theme: &Theme,
    layout: &EditorLayout,
    scale: f32,
    state: &EditorState,
    bitmaps: Option<&Previews>,
    name: &str,
    hover: EditorHit,
    now: Instant,
) -> Result<()> {
    let panel = rect(layout.panel);
    surface.set_glass(0, theme.glass.then_some(panel), CORNER * scale)?;
    let frame = Frame {
        ctx: surface.ctx(),
        icons: fonts::icon_format(surface.icon_font(), ICON * scale)?,
        label: fonts::text_format(
            13.0 * scale,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_TEXT_ALIGNMENT_LEADING,
        )?,
        button: fonts::trimmed_format(
            13.0 * scale,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_TEXT_ALIGNMENT_CENTER,
        )?,
        small: fonts::trimmed_format(
            12.0 * scale,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_TEXT_ALIGNMENT_CENTER,
        )?,
        theme,
        layout,
        scale,
        state,
        hover,
    };
    let name_format = fonts::trimmed_format(
        12.0 * scale,
        DWRITE_FONT_WEIGHT_NORMAL,
        DWRITE_TEXT_ALIGNMENT_TRAILING,
    )?;
    let ctx = surface.ctx();
    unsafe {
        ctx.BeginDraw();
        ctx.SetTransform(&Matrix3x2::identity());
        ctx.Clear(Some(&D2D1_COLOR_F::default()));
    }
    let drawn = (|| {
        frame.backdrop(panel)?;
        frame.preview(bitmaps)?;
        frame.toolbar(name, &name_format)?;
        frame.chips()?;
        frame.footer(now)
    })();
    // Always end the frame, so a failed one doesn't leave it open.
    let ended = unsafe { ctx.EndDraw(None, None) };
    drawn?;
    ended?;
    surface.present()
}

/// What every part of a frame draws with.
struct Frame<'a> {
    ctx: &'a ID2D1DeviceContext,
    icons: IDWriteTextFormat,
    label: IDWriteTextFormat,
    button: IDWriteTextFormat,
    small: IDWriteTextFormat,
    theme: &'a Theme,
    layout: &'a EditorLayout,
    scale: f32,
    state: &'a EditorState,
    hover: EditorHit,
}

impl Frame<'_> {
    fn brush(&self, color: D2D1_COLOR_F) -> Result<ID2D1SolidColorBrush> {
        unsafe { self.ctx.CreateSolidColorBrush(&color, None) }
    }

    fn fill(&self, r: D2D_RECT_F, color: D2D1_COLOR_F) -> Result<()> {
        let brush = self.brush(color)?;
        unsafe {
            self.ctx
                .FillRoundedRectangle(&rounded(r, BUTTON_CORNER * self.scale), &brush)
        };
        Ok(())
    }

    /// The panel's glass and shadow, as the menu panel has.
    fn backdrop(&self, panel: D2D_RECT_F) -> Result<()> {
        let shape = rounded_shape(self.ctx, panel, CORNER * self.scale)?;
        outer_shadow(self.ctx, &shape, self.theme, self.scale)?;
        glass_layer(self.ctx, &shape, panel, self.theme, self.scale)
    }

    fn toolbar(&self, name: &str, name_format: &IDWriteTextFormat) -> Result<()> {
        let (theme, scale) = (self.theme, self.scale);
        let session = self.state.session();
        let cropping = self.state.crop().is_some();
        let locked = session.is_some_and(|s| s.has_pixel_edits());
        for (&tool, &b) in TOOLS.iter().zip(&self.layout.tools) {
            if tool == EditorTool::Info && !info_shown(self.state) {
                continue;
            }
            let usable = match tool {
                EditorTool::Crop => true,
                EditorTool::Info => !cropping && !locked,
                _ => !cropping,
            };
            let usable = usable && session.is_some();
            let on = session.is_some_and(|s| match tool {
                EditorTool::Crop => cropping,
                EditorTool::Gray => s.grayscale(),
                // Edits strip the info anyway: shown on, and locked.
                EditorTool::Info => s.strip_info() || locked,
                _ => false,
            });
            let alpha = if usable { 1.0 } else { DIMMED };
            let r = rect(b);
            if on {
                self.fill(r, faded(theme.accent, alpha))?;
            } else if usable && self.hover == EditorHit::Tool(tool) {
                self.fill(r, theme.line)?;
            }
            let color = if on { theme.on_accent } else { theme.text };
            let brush = self.brush(faded(color, alpha))?;
            // The icon and the label side by side, centered together.
            let label = label(tool);
            let (width, _) = fonts::text_size(label, &self.label)?;
            let (icon, gap) = (ICON * scale, ICON_GAP * scale);
            let left = ((r.left + r.right - icon - gap - width) / 2.0).max(r.left);
            let icon_box = D2D_RECT_F {
                left,
                right: left + icon,
                ..r
            };
            draw_text(self.ctx, glyph(tool), &self.icons, icon_box, &brush);
            let label_box = D2D_RECT_F {
                left: left + icon + gap,
                right: r.right,
                ..r
            };
            draw_text(self.ctx, label, &self.label, label_box, &brush);
        }
        let name_box = rect(self.layout.name);
        draw_text(
            self.ctx,
            name,
            name_format,
            name_box,
            &self.brush(theme.subtext)?,
        );
        Ok(())
    }

    /// The ratio chips, in crop mode only.
    fn chips(&self) -> Result<()> {
        let Some(crop) = self.state.crop() else {
            return Ok(());
        };
        let theme = self.theme;
        for (i, ((_, label), &b)) in CHIPS.iter().zip(&self.layout.chips).enumerate() {
            let r = rect(b);
            let selected = crop.chip == i;
            let fill = if selected {
                theme.accent
            } else if self.hover == EditorHit::Chip(i) {
                theme.track
            } else {
                theme.line
            };
            self.fill(r, fill)?;
            let color = if selected {
                theme.on_accent
            } else {
                theme.text
            };
            draw_text(self.ctx, label, &self.button, r, &self.brush(color)?);
        }
        Ok(())
    }

    /// The image, transformed by the edits so far, or "불러오는 중".
    fn preview(&self, bitmaps: Option<&Previews>) -> Result<()> {
        let ctx = self.ctx;
        let Some(session) = self.state.session() else {
            let loading = fonts::text_format(
                14.0 * self.scale,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_TEXT_ALIGNMENT_CENTER,
            )?;
            let brush = self.brush(self.theme.subtext)?;
            draw_text(
                ctx,
                text::LOADING,
                &loading,
                rect(self.layout.preview),
                &brush,
            );
            return Ok(());
        };
        let Some(previews) = bitmaps else {
            return Ok(());
        };
        let size = session.size();
        let fit = Fit::new(self.layout.preview, size);
        let view = rect(fit.to_view(PixelRect::full(size)));
        let bitmap = if session.grayscale() {
            &previews.gray
        } else {
            &previews.color
        };
        // Bitmap pixels → original pixels → result pixels → the view.
        let pixels = unsafe { bitmap.GetPixelSize() };
        let (ow, oh) = session.original_size();
        let to_original = Matrix3x2::scale(
            ow as f32 / pixels.width.max(1) as f32,
            oh as f32 / pixels.height.max(1) as f32,
        );
        let a = session.to_result();
        let to_result = Matrix3x2 {
            M11: a.xx as f32,
            M12: a.yx as f32,
            M21: a.xy as f32,
            M22: a.yy as f32,
            M31: a.x0 as f32,
            M32: a.y0 as f32,
        };
        let to_view = Matrix3x2 {
            M31: fit.left,
            M32: fit.top,
            ..Matrix3x2::scale(fit.scale, fit.scale)
        };
        unsafe {
            // Only the result's rectangle: a crop leaves the rest outside.
            ctx.PushAxisAlignedClip(&view, D2D1_ANTIALIAS_MODE_ALIASED);
            ctx.SetTransform(&(to_original * to_result * to_view));
            ctx.DrawBitmap(
                bitmap,
                None,
                1.0,
                D2D1_INTERPOLATION_MODE_LINEAR,
                None,
                None,
            );
            ctx.SetTransform(&Matrix3x2::identity());
            ctx.PopAxisAlignedClip();
        }
        match self.state.crop() {
            Some(crop) => self.crop_overlay(view, rect(fit.to_view(crop.rect))),
            None => Ok(()),
        }
    }

    /// Shade outside the crop rectangle `c`, its border, thirds and handles.
    fn crop_overlay(&self, image: D2D_RECT_F, c: D2D_RECT_F) -> Result<()> {
        let ctx = self.ctx;
        let shade = self.brush(D2D1_COLOR_F {
            a: 0.5,
            ..Default::default()
        })?;
        let white = |a: f32| D2D1_COLOR_F {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a,
        };
        let border = self.brush(white(1.0))?;
        let thirds = self.brush(white(0.4))?;
        let outline = self.brush(D2D1_COLOR_F {
            a: 0.35,
            ..Default::default()
        })?;
        unsafe {
            for r in [
                D2D_RECT_F {
                    bottom: c.top,
                    ..image
                },
                D2D_RECT_F {
                    top: c.bottom,
                    ..image
                },
                D2D_RECT_F {
                    top: c.top,
                    bottom: c.bottom,
                    right: c.left,
                    ..image
                },
                D2D_RECT_F {
                    top: c.top,
                    bottom: c.bottom,
                    left: c.right,
                    ..image
                },
            ] {
                if r.right > r.left && r.bottom > r.top {
                    ctx.FillRectangle(&r, &shade);
                }
            }
            let (w, h) = (c.right - c.left, c.bottom - c.top);
            for k in [1.0, 2.0] {
                let x = c.left + w * k / 3.0;
                let y = c.top + h * k / 3.0;
                ctx.DrawLine(point(x, c.top), point(x, c.bottom), &thirds, 1.0, None);
                ctx.DrawLine(point(c.left, y), point(c.right, y), &thirds, 1.0, None);
            }
            let edge = D2D_RECT_F {
                left: c.left + 0.5,
                top: c.top + 0.5,
                right: c.right - 0.5,
                bottom: c.bottom - 0.5,
            };
            ctx.DrawRectangle(&edge, &border, 1.0, None);
            let half = HANDLE * self.scale / 2.0;
            let (mx, my) = ((c.left + c.right) / 2.0, (c.top + c.bottom) / 2.0);
            for (x, y) in [
                (c.left, c.top),
                (mx, c.top),
                (c.right, c.top),
                (c.right, my),
                (c.right, c.bottom),
                (mx, c.bottom),
                (c.left, c.bottom),
                (c.left, my),
            ] {
                let square = D2D_RECT_F {
                    left: x - half,
                    top: y - half,
                    right: x + half,
                    bottom: y + half,
                };
                ctx.FillRectangle(&square, &border);
                ctx.DrawRectangle(&square, &outline, 1.0, None);
            }
        }
        Ok(())
    }

    fn footer(&self, now: Instant) -> Result<()> {
        let (theme, layout) = (self.theme, self.layout);
        let session = self.state.session();
        let cropping = self.state.crop().is_some();

        let steps = [
            (layout.undo, GLYPH_UNDO, EditorHit::Undo),
            (layout.redo, GLYPH_REDO, EditorHit::Redo),
        ];
        for (b, glyph, hit) in steps {
            let usable = !cropping
                && session.is_some_and(|s| {
                    if hit == EditorHit::Undo {
                        s.can_undo()
                    } else {
                        s.can_redo()
                    }
                });
            let r = rect(b);
            if usable && self.hover == hit {
                self.fill(r, theme.line)?;
            }
            let alpha = if usable { 1.0 } else { DIMMED };
            draw_text(
                self.ctx,
                glyph,
                &self.icons,
                r,
                &self.brush(faded(theme.text, alpha))?,
            );
        }

        let (status, color) = match self.state.footer_text(self.hover, now) {
            Some(message) => (message.to_string(), theme.text),
            None => (size_text(self.state), theme.subtext),
        };
        draw_text(
            self.ctx,
            &status,
            &self.small,
            rect(layout.label),
            &self.brush(color)?,
        );

        let cancel = rect(layout.cancel);
        let fill = if self.hover == EditorHit::Cancel {
            theme.track
        } else {
            theme.line
        };
        self.fill(cancel, fill)?;
        let label = if cropping {
            text::CROP_CANCEL
        } else {
            text::CANCEL
        };
        draw_text(
            self.ctx,
            label,
            &self.button,
            cancel,
            &self.brush(theme.text)?,
        );

        let save = rect(layout.save);
        let usable = cropping || session.is_some_and(|s| s.save_kind().is_some());
        let label = if cropping {
            text::CROP_APPLY
        } else {
            text::SAVE
        };
        if usable {
            let corner = BUTTON_CORNER * self.scale;
            let shape = rounded_shape(self.ctx, save, corner)?;
            glass_piece(self.ctx, &shape, save, theme, Piece::Accent, self.scale)?;
            if self.hover == EditorHit::Save {
                self.fill(save, faded(theme.on_accent, 0.15))?;
            }
            let brush = self.brush(theme.on_accent)?;
            draw_text(self.ctx, label, &self.button, save, &brush);
        } else {
            self.fill(save, theme.line)?;
            let brush = self.brush(faded(theme.text, DIMMED))?;
            draw_text(self.ctx, label, &self.button, save, &brush);
        }
        Ok(())
    }
}

/// The footer's size line: the crop size in crop mode, otherwise the
/// original size, followed by the result's once there is something to save.
fn size_text(state: &EditorState) -> String {
    let Some(session) = state.session() else {
        return String::new();
    };
    if let Some(crop) = state.crop() {
        return format!("{}×{}", crop.rect.w, crop.rect.h);
    }
    let (ow, oh) = session.original_size();
    let (w, h) = session.size();
    if session.save_kind().is_some() {
        format!("{ow}×{oh} → {w}×{h}")
    } else {
        format!("{ow}×{oh}")
    }
}
