//! Draws the video panel: the eight tools, the frame with the edits so far,
//! the timeline row and the footer. A frame is a pure function of what it
//! is given, as for the image panel.

use std::time::Instant;

use swisswheel_core::crop::CHIPS;
use swisswheel_core::editor::{PixelRect, text as editor_text};
use swisswheel_core::editor_geometry::{
    CORNER, Fit, Timeline, VIDEO_TOOLS, VideoHit, VideoLayout, VideoTool,
};
use swisswheel_core::video_control::VideoState;
use swisswheel_core::video_edit::{AudioMode, Speed, VideoSession, format_time, text};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::core::Result;
use windows_numerics::Matrix3x2;

use super::render::{GLYPH_REDO, GLYPH_UNDO, Painter, faded, point, rect};
use crate::surface::{Surface, draw_text, rounded};
use crate::theme::Theme;

const GLYPH_PLAY: &str = "\u{E768}";
const GLYPH_PAUSE: &str = "\u{E769}";
const GLYPH_SPLIT: &str = "\u{E8C6}";
const GLYPH_REMOVE: &str = "\u{E738}";
const GLYPH_RESTORE: &str = "\u{E710}";
/// Opacity of the hatching over removed pieces.
const REMOVED_ALPHA: f32 = 0.4;
/// Distance between hatch lines (DIPs).
const HATCH: f32 = 5.0;
/// How far cut ticks and the playhead stick out of the bar (DIPs).
const TICK_OUT: f32 = 4.0;
const PLAYHEAD_OUT: f32 = 7.0;

/// Everything one frame of the panel shows.
pub(super) struct View<'a> {
    pub layout: &'a VideoLayout,
    pub scale: f32,
    pub state: &'a VideoState,
    /// The latest video frame, from the player or from FFmpeg.
    pub frame: Option<&'a ID2D1Bitmap1>,
    /// Counterclockwise degrees that turn `frame` upright.
    pub turn: u32,
    pub name: &'a str,
    pub hover: VideoHit,
    /// The last FFmpeg frame request failed.
    pub scrub_failed: bool,
}

/// Draws one frame and presents it.
pub(super) fn draw(surface: &Surface, theme: &Theme, view: &View, now: Instant) -> Result<()> {
    let panel = rect(view.layout.panel);
    surface.set_glass(0, theme.glass.then_some(panel), CORNER * view.scale)?;
    let p = Painter::new(surface, theme, view.scale)?;
    p.begin();
    let drawn = (|| {
        p.backdrop(panel)?;
        preview(&p, view)?;
        toolbar(&p, view)?;
        chips(&p, view)?;
        timeline(&p, view)?;
        footer(&p, view, now)
    })();
    p.end(surface, drawn)
}

fn tool_face(
    tool: VideoTool,
    session: &VideoSession,
    cropping: bool,
) -> (&'static str, &'static str, bool) {
    match tool {
        VideoTool::Crop => ("\u{E7A8}", text::AREA, cropping),
        VideoTool::Rotate => ("\u{E7AD}", text::ROTATE, false),
        VideoTool::FlipH => ("\u{E8AB}", text::FLIP_H, false),
        VideoTool::FlipV => ("\u{E8CB}", text::FLIP_V, false),
        VideoTool::Speed => (
            "\u{E916}",
            session.speed().label(),
            session.speed() != Speed::Normal,
        ),
        VideoTool::Audio => {
            let audio = session.audio();
            let glyph = match audio {
                AudioMode::Keep => "\u{E767}",
                AudioMode::Mute => "\u{E74F}",
                AudioMode::Normalize => "\u{E8D6}",
            };
            (glyph, audio.label(), audio != AudioMode::Keep)
        }
        VideoTool::Reverse => ("\u{E8EE}", text::REVERSE, session.reverse()),
        VideoTool::Frame => ("\u{E722}", text::FRAME, false),
    }
}

fn toolbar(p: &Painter, view: &View) -> Result<()> {
    let cropping = view.state.crop().is_some();
    if let Some(session) = view.state.session() {
        for (&tool, &b) in VIDEO_TOOLS.iter().zip(&view.layout.tools) {
            let (glyph, label, on) = tool_face(tool, session, cropping);
            let usable = (tool == VideoTool::Crop || !cropping)
                && (tool != VideoTool::Audio || session.info().has_audio);
            let hovered = view.hover == VideoHit::Tool(tool);
            p.tool(rect(b), glyph, label, on, usable, hovered)?;
        }
    }
    p.file_name(rect(view.layout.name), view.name)
}

fn chips(p: &Painter, view: &View) -> Result<()> {
    let Some(crop) = view.state.crop() else {
        return Ok(());
    };
    for (i, ((_, label), &b)) in CHIPS.iter().zip(&view.layout.chips).enumerate() {
        p.chip(
            rect(b),
            label,
            crop.chip == i,
            view.hover == VideoHit::Chip(i),
        )?;
    }
    Ok(())
}

/// Turns frame pixels counterclockwise by `turn` degrees (0, 90, 180 or
/// 270) into the upright picture, for a frame of `size` pixels.
pub(super) fn turn_matrix(turn: u32, size: (u32, u32)) -> Matrix3x2 {
    let (w, h) = (size.0 as f32, size.1 as f32);
    let (m11, m12, m21, m22, m31, m32) = match turn {
        90 => (0.0, -1.0, 1.0, 0.0, 0.0, w),
        180 => (-1.0, 0.0, 0.0, -1.0, w, h),
        270 => (0.0, 1.0, -1.0, 0.0, h, 0.0),
        _ => return Matrix3x2::identity(),
    };
    Matrix3x2 {
        M11: m11,
        M12: m12,
        M21: m21,
        M22: m22,
        M31: m31,
        M32: m32,
    }
}

/// The frame, transformed by the edits so far, with what lies over it:
/// the removed-piece shade, the reverse badge and the crop overlay.
fn preview(p: &Painter, view: &View) -> Result<()> {
    let ctx = p.ctx;
    let state = view.state;
    let Some(session) = state.session() else {
        return p.centered_note(rect(view.layout.preview), editor_text::LOADING);
    };
    let size = session.size();
    let fit = Fit::new(view.layout.preview, size);
    let area = rect(fit.to_view(PixelRect::full(size)));
    match view.frame {
        Some(bitmap) => {
            // Frame pixels → upright frame → display pixels → result → the view.
            let px = unsafe { bitmap.GetPixelSize() };
            let frame = (px.width.max(1), px.height.max(1));
            let upright = if view.turn % 180 == 90 {
                (frame.1, frame.0)
            } else {
                frame
            };
            let info = session.info();
            let to_display = Matrix3x2::scale(
                info.width as f32 / upright.0 as f32,
                info.height as f32 / upright.1 as f32,
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
                ctx.PushAxisAlignedClip(&area, D2D1_ANTIALIAS_MODE_ALIASED);
                ctx.SetTransform(
                    &(turn_matrix(view.turn, frame) * to_display * to_result * to_view),
                );
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
        }
        None if !view.scrub_failed => p.centered_note(area, editor_text::LOADING)?,
        None => {}
    }
    let paused_on_removed = !state.playing() && !session.is_kept(state.position());
    if view.scrub_failed || paused_on_removed {
        let shade = p.brush(D2D1_COLOR_F {
            a: 0.55,
            ..Default::default()
        })?;
        unsafe { ctx.FillRectangle(&area, &shade) };
        let note = if view.scrub_failed {
            text::SCRUB_FAILED
        } else {
            text::REMOVED
        };
        let white = p.brush(D2D1_COLOR_F {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        })?;
        draw_text(ctx, note, &p.button, area, &white);
    }
    if session.reverse() {
        badge(p, area, text::REVERSE)?;
    }
    match state.crop() {
        Some(crop) => p.crop_overlay(area, rect(fit.to_view(crop.rect))),
        None => Ok(()),
    }
}

/// A small accent label at the top-left corner of `area`.
fn badge(p: &Painter, area: D2D_RECT_F, label: &str) -> Result<()> {
    let s = p.scale;
    let r = D2D_RECT_F {
        left: area.left + 8.0 * s,
        top: area.top + 8.0 * s,
        right: area.left + 8.0 * s + 64.0 * s,
        bottom: area.top + 8.0 * s + 24.0 * s,
    };
    p.fill(r, p.theme.accent)?;
    draw_text(p.ctx, label, &p.small, r, &p.brush(p.theme.on_accent)?);
    Ok(())
}

/// Play, the time, the bar with its pieces, and the split and remove buttons.
fn timeline(p: &Painter, view: &View) -> Result<()> {
    let (layout, state, theme) = (view.layout, view.state, p.theme);
    let Some(session) = state.session() else {
        return Ok(());
    };
    let ctx = p.ctx;
    let s = p.scale;
    let cropping = state.crop().is_some();

    let glyph = if state.playing() {
        GLYPH_PAUSE
    } else {
        GLYPH_PLAY
    };
    let hovered = view.hover == VideoHit::Play;
    p.icon_button(rect(layout.play), glyph, !state.fallback(), hovered)?;

    let duration = session.info().duration;
    let time = format!(
        "{} / {}",
        format_time(state.position()),
        format_time(duration)
    );
    draw_text(
        ctx,
        &time,
        &p.small,
        rect(layout.time),
        &p.brush(theme.text)?,
    );

    let bar = rect(layout.bar);
    let line = Timeline::from_bar(layout.bar, duration);
    let height = bar.bottom - bar.top;
    unsafe {
        ctx.FillRoundedRectangle(&rounded(bar, height / 2.0), &p.brush(theme.line)?);
    }
    let kept = p.brush(faded(theme.text, 0.35))?;
    let hatch = p.brush(faded(theme.text, REMOVED_ALPHA))?;
    let segments = session.segments();
    for seg in &segments {
        let piece = D2D_RECT_F {
            left: line.x_of(seg.start),
            right: line.x_of(seg.end),
            ..bar
        };
        unsafe {
            if seg.kept {
                ctx.FillRectangle(&piece, &kept);
            } else {
                ctx.PushAxisAlignedClip(&piece, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                let step = HATCH * s;
                let mut x = piece.left - height;
                while x < piece.right {
                    ctx.DrawLine(
                        point(x, bar.bottom),
                        point(x + height, bar.top),
                        &hatch,
                        1.0,
                        None,
                    );
                    x += step;
                }
                ctx.PopAxisAlignedClip();
            }
        }
    }
    let tick = p.brush(theme.text)?;
    for seg in segments.iter().skip(1) {
        let x = line.x_of(seg.start).round() + 0.5;
        unsafe {
            ctx.DrawLine(
                point(x, bar.top - TICK_OUT * s),
                point(x, bar.bottom + TICK_OUT * s),
                &tick,
                1.0,
                None,
            );
        }
    }
    let x = line.x_of(state.position());
    let head = D2D_RECT_F {
        left: x - 1.5 * s,
        right: x + 1.5 * s,
        top: bar.top - PLAYHEAD_OUT * s,
        bottom: bar.bottom + PLAYHEAD_OUT * s,
    };
    unsafe {
        ctx.FillRoundedRectangle(&rounded(head, 1.5 * s), &p.brush(theme.accent)?);
    }

    let hovered = view.hover == VideoHit::Split;
    p.tool(
        rect(layout.split),
        GLYPH_SPLIT,
        text::SPLIT,
        false,
        !cropping,
        hovered,
    )?;
    let (glyph, label) = if session.is_kept(state.position()) {
        (GLYPH_REMOVE, text::REMOVE)
    } else {
        (GLYPH_RESTORE, text::RESTORE)
    };
    let hovered = view.hover == VideoHit::Toggle;
    p.tool(rect(layout.toggle), glyph, label, false, !cropping, hovered)
}

fn footer(p: &Painter, view: &View, now: Instant) -> Result<()> {
    let (layout, state) = (view.layout, view.state);
    let session = state.session();
    let cropping = state.crop().is_some();
    for (b, glyph, hit) in [
        (layout.undo, GLYPH_UNDO, VideoHit::Undo),
        (layout.redo, GLYPH_REDO, VideoHit::Redo),
    ] {
        let usable = !cropping
            && session.is_some_and(|s| {
                if hit == VideoHit::Undo {
                    s.can_undo()
                } else {
                    s.can_redo()
                }
            });
        p.icon_button(rect(b), glyph, usable, view.hover == hit)?;
    }
    let message = state.footer_text(view.hover, now);
    p.status(rect(layout.label), message, &summary(state))?;
    let label = if cropping {
        editor_text::CROP_CANCEL
    } else {
        editor_text::CANCEL
    };
    p.text_button(
        rect(layout.cancel),
        label,
        true,
        view.hover == VideoHit::Cancel,
    )?;
    let usable = cropping || session.is_some_and(|s| s.has_edits());
    let label = if cropping {
        editor_text::CROP_APPLY
    } else {
        editor_text::SAVE
    };
    p.save_button(
        rect(layout.save),
        label,
        usable,
        view.hover == VideoHit::Save,
    )
}

/// The footer's line: the area size in crop mode; otherwise the original
/// size and length, then the result's once there are edits, and the speed.
pub(super) fn summary(state: &VideoState) -> String {
    let Some(session) = state.session() else {
        return String::new();
    };
    if let Some(crop) = state.crop() {
        return format!("{}×{}", crop.rect.w, crop.rect.h);
    }
    let info = session.info();
    let original = format!(
        "{}×{} · {}",
        info.width,
        info.height,
        format_time(info.duration)
    );
    if !session.has_edits() {
        return original;
    }
    let (w, h) = session.size();
    let mut line = format!(
        "{original} → {w}×{h} · {}",
        format_time(session.output_duration())
    );
    if session.speed() != Speed::Normal {
        line.push_str(" · ");
        line.push_str(session.speed().label());
    }
    line
}
