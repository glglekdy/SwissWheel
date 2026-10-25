//! The editor's state machine: keys, clicks, crop mode and the discard prompt.
//!
//! The Win32 panel forwards every event here and only draws the result.

use std::time::{Duration, Instant};

use crate::crop::{CHIPS, CropHandle, drag, fit_ratio};
use crate::editor::{Axis, EditOp, EditPlan, EditSession, PixelRect, Turn, text};
use crate::editor_geometry::{EditorHit, EditorTool};

/// How long the discard prompt and other footer notices stay up.
pub const NOTICE_WINDOW: Duration = Duration::from_secs(3);

/// The "discard edits?" prompt armed by a first Escape.
#[derive(Debug, Clone, Default)]
pub struct DiscardPrompt {
    until: Option<Instant>,
}

impl DiscardPrompt {
    /// Arms the prompt for [`NOTICE_WINDOW`] from `now`.
    pub fn arm(&mut self, now: Instant) {
        self.until = Some(now + NOTICE_WINDOW);
    }

    /// Whether the prompt is armed and not yet expired.
    pub fn is_up(&self, now: Instant) -> bool {
        self.until.is_some_and(|deadline| now < deadline)
    }

    /// Disarms the prompt. True if it was set, expired or not.
    pub fn clear(&mut self) -> bool {
        self.until.take().is_some()
    }

    /// When the armed prompt expires.
    pub fn deadline(&self) -> Option<Instant> {
        self.until
    }
}

/// Something the user asked the editor to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorCommand {
    Crop,
    RotateCw,
    RotateCcw,
    FlipH,
    FlipV,
    Gray,
    Info,
    Undo,
    Redo,
    Enter,
    Escape,
}

/// The command a key press stands for, given the Windows virtual-key code and modifiers.
pub fn key_command(vk: u16, shift: bool, ctrl: bool) -> Option<EditorCommand> {
    use EditorCommand::*;
    if ctrl {
        return match vk {
            0x5A if shift => Some(Redo),
            0x5A => Some(Undo),
            0x59 => Some(Redo),
            _ => None,
        };
    }
    match vk {
        0x43 => Some(Crop),
        0x52 if shift => Some(RotateCcw),
        0x52 => Some(RotateCw),
        0x48 => Some(FlipH),
        0x56 => Some(FlipV),
        0x47 => Some(Gray),
        0x49 => Some(Info),
        0x0D => Some(Enter),
        0x1B => Some(Escape),
        _ => None,
    }
}

/// Crop mode: the rectangle being chosen, the active ratio chip and any drag in progress.
#[derive(Debug, Clone, PartialEq)]
pub struct CropMode {
    pub rect: PixelRect,
    /// Index into [`CHIPS`].
    pub chip: usize,
    /// The grabbed handle, the rectangle at the press and the press point.
    pub(crate) drag: Option<(CropHandle, PixelRect, (f64, f64))>,
}

impl CropMode {
    /// The handle being dragged, if any.
    pub fn dragging(&self) -> Option<CropHandle> {
        self.drag.map(|(handle, _, _)| handle)
    }
}

/// What the panel should do after an event.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Nothing,
    Redraw,
    Save(EditPlan),
    Close,
}

/// The editor's whole interactive state.
#[derive(Debug)]
pub struct EditorState {
    session: Option<EditSession>,
    crop: Option<CropMode>,
    discard: DiscardPrompt,
}

impl EditorState {
    /// The state while the image is still being decoded.
    pub fn loading() -> Self {
        Self {
            session: None,
            crop: None,
            discard: DiscardPrompt::default(),
        }
    }

    /// The image finished loading.
    pub fn loaded(&mut self, width: u32, height: u32, strip_available: bool) {
        self.session = Some(EditSession::new(width, height, strip_available));
    }

    pub fn session(&self) -> Option<&EditSession> {
        self.session.as_ref()
    }

    pub fn crop(&self) -> Option<&CropMode> {
        self.crop.as_ref()
    }

    /// When the armed discard prompt expires.
    pub fn discard_deadline(&self) -> Option<Instant> {
        self.discard.deadline()
    }

    /// Applies `cmd`.
    pub fn command(&mut self, cmd: EditorCommand, now: Instant) -> Outcome {
        let Some(session) = self.session.as_ref() else {
            return if cmd == EditorCommand::Escape {
                Outcome::Close
            } else {
                Outcome::Nothing
            };
        };

        if cmd == EditorCommand::Escape && self.crop.is_none() {
            if session.save_kind().is_none() {
                return Outcome::Close;
            }
            if self.discard.is_up(now) {
                return Outcome::Close;
            }
            self.discard.arm(now);
            return Outcome::Redraw;
        }

        // Anything else disarms the prompt; that alone is worth a redraw.
        let prompt_cleared = self.discard.clear();
        let outcome = match self.crop.take() {
            Some(crop) => self.crop_command(crop, cmd),
            None => self.normal_command(cmd),
        };
        if outcome == Outcome::Nothing && prompt_cleared {
            Outcome::Redraw
        } else {
            outcome
        }
    }

    fn crop_command(&mut self, crop: CropMode, cmd: EditorCommand) -> Outcome {
        match cmd {
            EditorCommand::Crop | EditorCommand::Enter => {
                if let Some(session) = self.session.as_mut() {
                    session.push(EditOp::Crop(crop.rect));
                }
                Outcome::Redraw
            }
            EditorCommand::Escape => Outcome::Redraw,
            _ => {
                self.crop = Some(crop);
                Outcome::Nothing
            }
        }
    }

    fn normal_command(&mut self, cmd: EditorCommand) -> Outcome {
        let Some(session) = self.session.as_mut() else {
            return Outcome::Nothing;
        };
        let changed = match cmd {
            EditorCommand::Crop => {
                self.crop = Some(CropMode {
                    rect: PixelRect::full(session.size()),
                    chip: 0,
                    drag: None,
                });
                true
            }
            EditorCommand::RotateCw => session.push(EditOp::Rotate(Turn::Cw)),
            EditorCommand::RotateCcw => session.push(EditOp::Rotate(Turn::Ccw)),
            EditorCommand::FlipH => session.push(EditOp::Flip(Axis::Horizontal)),
            EditorCommand::FlipV => session.push(EditOp::Flip(Axis::Vertical)),
            EditorCommand::Gray => session.push(EditOp::Grayscale(!session.grayscale())),
            EditorCommand::Info => {
                !session.has_pixel_edits() && session.push(EditOp::StripInfo(!session.strip_info()))
            }
            EditorCommand::Undo => session.undo(),
            EditorCommand::Redo => session.redo(),
            EditorCommand::Enter => {
                return match session.plan() {
                    Some(plan) => Outcome::Save(plan),
                    None => Outcome::Nothing,
                };
            }
            // Handled in `command` before dispatch.
            EditorCommand::Escape => false,
        };
        if changed {
            Outcome::Redraw
        } else {
            Outcome::Nothing
        }
    }

    /// Called by the panel for a key press that [`key_command`] does not map; it disarms the
    /// discard prompt like any other key.
    pub fn key_other(&mut self, now: Instant) -> Outcome {
        let was_up = self.discard.is_up(now);
        self.discard.clear();
        if was_up {
            Outcome::Redraw
        } else {
            Outcome::Nothing
        }
    }

    /// Applies a click on `hit`.
    pub fn click(&mut self, hit: EditorHit, now: Instant) -> Outcome {
        let cmd = match hit {
            EditorHit::Tool(EditorTool::Crop) => EditorCommand::Crop,
            EditorHit::Tool(EditorTool::Rotate) => EditorCommand::RotateCw,
            EditorHit::Tool(EditorTool::FlipH) => EditorCommand::FlipH,
            EditorHit::Tool(EditorTool::FlipV) => EditorCommand::FlipV,
            EditorHit::Tool(EditorTool::Gray) => EditorCommand::Gray,
            EditorHit::Tool(EditorTool::Info) => EditorCommand::Info,
            EditorHit::Undo => EditorCommand::Undo,
            EditorHit::Redo => EditorCommand::Redo,
            EditorHit::Save => EditorCommand::Enter,
            EditorHit::Cancel => EditorCommand::Escape,
            EditorHit::Chip(i) => return self.chip(i),
            EditorHit::Preview | EditorHit::Nothing => return self.disarm(),
        };
        self.command(cmd, now)
    }

    /// Clears the discard prompt; a redraw is due only if it was up.
    fn disarm(&mut self) -> Outcome {
        if self.discard.clear() {
            Outcome::Redraw
        } else {
            Outcome::Nothing
        }
    }

    fn chip(&mut self, i: usize) -> Outcome {
        let cleared = self.disarm();
        let (Some(session), Some(crop)) = (self.session.as_ref(), self.crop.as_mut()) else {
            return cleared;
        };
        let Some(&(ratio, _)) = CHIPS.get(i) else {
            return cleared;
        };
        let size = session.size();
        let before = (crop.chip, crop.rect);
        crop.chip = i;
        if let Some(value) = ratio.value(size) {
            crop.rect = fit_ratio(crop.rect, size, value);
        }
        if before != (crop.chip, crop.rect) {
            Outcome::Redraw
        } else {
            cleared
        }
    }

    /// Starts dragging `handle` from the image point `at`. Ignored outside crop mode.
    pub fn crop_press(&mut self, handle: CropHandle, at: (f64, f64)) {
        if let Some(crop) = self.crop.as_mut() {
            crop.drag = Some((handle, crop.rect, at));
        }
    }

    /// Continues a drag to the image point `at`. True when the rectangle changed.
    pub fn crop_move(&mut self, at: (f64, f64)) -> bool {
        let (Some(session), Some(crop)) = (self.session.as_ref(), self.crop.as_mut()) else {
            return false;
        };
        let Some((handle, start, from)) = crop.drag else {
            return false;
        };
        let size = session.size();
        let ratio = CHIPS[crop.chip].0.value(size);
        let rect = drag(start, handle, at.0 - from.0, at.1 - from.1, size, ratio);
        let changed = rect != crop.rect;
        crop.rect = rect;
        changed
    }

    /// Ends any drag.
    pub fn crop_release(&mut self) {
        if let Some(crop) = self.crop.as_mut() {
            crop.drag = None;
        }
    }

    /// The footer message to show instead of the image size, if any.
    pub fn footer_text(&self, hover: EditorHit, now: Instant) -> Option<&'static str> {
        if self.discard.is_up(now) {
            return Some(text::DISCARD);
        }
        let locked = self.session.as_ref().is_some_and(|s| s.has_pixel_edits());
        (hover == EditorHit::Tool(EditorTool::Info) && locked).then_some(text::INFO_LOCKED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{SaveKind, text};
    use crate::editor_geometry::EditorTool;

    fn ready(w: u32, h: u32) -> EditorState {
        let mut s = EditorState::loading();
        s.loaded(w, h, true);
        s
    }

    #[test]
    fn keys() {
        assert_eq!(
            key_command(0x52, true, false),
            Some(EditorCommand::RotateCcw)
        );
        assert_eq!(key_command(0x5A, true, true), Some(EditorCommand::Redo));
        assert_eq!(key_command(0x43, false, true), None);
        assert_eq!(key_command(0x5A, false, true), Some(EditorCommand::Undo));
        assert_eq!(key_command(0x59, false, true), Some(EditorCommand::Redo));
        assert_eq!(key_command(0x1B, false, false), Some(EditorCommand::Escape));
        assert_eq!(key_command(0x41, false, false), None);
    }

    #[test]
    fn loading_ignores_all_but_escape() {
        let t = Instant::now();
        let mut s = EditorState::loading();
        assert_eq!(s.command(EditorCommand::RotateCw, t), Outcome::Nothing);
        assert_eq!(s.command(EditorCommand::Escape, t), Outcome::Close);
    }

    #[test]
    fn discard_needs_a_second_escape_within_three_seconds() {
        let t = Instant::now();
        let mut s = ready(100, 50);
        s.command(EditorCommand::RotateCw, t);
        assert_eq!(s.command(EditorCommand::Escape, t), Outcome::Redraw);
        assert_eq!(s.footer_text(EditorHit::Nothing, t), Some(text::DISCARD));
        assert_eq!(
            s.command(EditorCommand::Escape, t + Duration::from_secs(4)),
            Outcome::Redraw
        );
        assert_eq!(
            s.command(EditorCommand::Escape, t + Duration::from_secs(5)),
            Outcome::Close
        );
        let mut clean = ready(100, 50);
        assert_eq!(clean.command(EditorCommand::Escape, t), Outcome::Close);
    }

    #[test]
    fn another_command_clears_the_prompt() {
        let t = Instant::now();
        let mut s = ready(100, 50);
        s.command(EditorCommand::RotateCw, t);
        s.command(EditorCommand::Escape, t);
        assert_eq!(s.command(EditorCommand::FlipH, t), Outcome::Redraw);
        assert_eq!(s.discard_deadline(), None);
        assert_eq!(s.command(EditorCommand::Escape, t), Outcome::Redraw);
        assert_eq!(s.click(EditorHit::Nothing, t), Outcome::Redraw);
        assert_eq!(s.footer_text(EditorHit::Nothing, t), None);
    }

    #[test]
    fn crop_mode_applies_and_cancels() {
        let t = Instant::now();
        let mut s = ready(200, 100);
        s.command(EditorCommand::Crop, t);
        assert_eq!(s.command(EditorCommand::RotateCw, t), Outcome::Nothing);
        s.crop_press(CropHandle::Right, (200.0, 50.0));
        assert_eq!(s.crop().unwrap().dragging(), Some(CropHandle::Right));
        assert!(s.crop_move((150.0, 50.0)));
        assert!(!s.crop_move((150.0, 50.0)));
        s.crop_release();
        assert_eq!(s.crop().unwrap().dragging(), None);
        assert_eq!(s.command(EditorCommand::Enter, t), Outcome::Redraw);
        assert_eq!(s.session().unwrap().size(), (150, 100));
        s.command(EditorCommand::Crop, t);
        s.click(EditorHit::Chip(2), t); // 1:1
        assert_eq!(s.crop().unwrap().rect.w, s.crop().unwrap().rect.h);
        s.click(EditorHit::Cancel, t);
        assert!(s.crop().is_none());
        assert_eq!(s.session().unwrap().size(), (150, 100));
    }

    #[test]
    fn crop_mode_edge_cases() {
        let t = Instant::now();
        let mut s = ready(200, 100);
        s.crop_press(CropHandle::Right, (200.0, 50.0));
        assert!(!s.crop_move((100.0, 50.0)));
        assert_eq!(s.click(EditorHit::Chip(2), t), Outcome::Nothing);
        s.command(EditorCommand::Crop, t);
        assert_eq!(s.command(EditorCommand::Undo, t), Outcome::Nothing);
        // Applying the untouched full rectangle records nothing.
        assert_eq!(s.command(EditorCommand::Enter, t), Outcome::Redraw);
        assert!(s.crop().is_none());
        assert!(!s.session().unwrap().can_undo());
    }

    #[test]
    fn enter_saves_and_info_locks() {
        let t = Instant::now();
        let mut s = ready(10, 10);
        assert_eq!(s.command(EditorCommand::Enter, t), Outcome::Nothing);
        s.command(EditorCommand::Info, t);
        assert!(matches!(
            s.command(EditorCommand::Enter, t),
            Outcome::Save(EditPlan {
                kind: SaveKind::StripOnly,
                ..
            })
        ));
        s.command(EditorCommand::Gray, t);
        assert_eq!(
            s.footer_text(EditorHit::Tool(EditorTool::Info), t),
            Some(text::INFO_LOCKED)
        );
        assert_eq!(s.command(EditorCommand::Info, t), Outcome::Nothing);
    }

    #[test]
    fn undo_and_redo_through_clicks() {
        let t = Instant::now();
        let mut s = ready(100, 50);
        assert_eq!(
            s.click(EditorHit::Tool(EditorTool::Rotate), t),
            Outcome::Redraw
        );
        assert_eq!(s.session().unwrap().size(), (50, 100));
        assert_eq!(s.click(EditorHit::Undo, t), Outcome::Redraw);
        assert_eq!(s.click(EditorHit::Undo, t), Outcome::Nothing);
        assert_eq!(s.click(EditorHit::Redo, t), Outcome::Redraw);
        let plan = s.session().unwrap().plan().unwrap();
        assert_eq!(s.click(EditorHit::Save, t), Outcome::Save(plan));
    }

    #[test]
    fn unmapped_key_clears_the_prompt() {
        let t = Instant::now();
        let mut s = ready(100, 50);
        assert_eq!(s.key_other(t), Outcome::Nothing);
        s.command(EditorCommand::RotateCw, t);
        s.command(EditorCommand::Escape, t);
        assert_eq!(s.key_other(t), Outcome::Redraw);
        assert_eq!(s.footer_text(EditorHit::Nothing, t), None);
        assert_eq!(s.key_other(t), Outcome::Nothing);
        // An already-expired prompt needs no redraw.
        s.command(EditorCommand::Escape, t);
        assert_eq!(s.key_other(t + Duration::from_secs(4)), Outcome::Nothing);
        assert_eq!(s.discard_deadline(), None);
    }
}
