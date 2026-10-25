//! The video editor's state machine: keys, clicks, the timeline, playback, crop mode, the
//! discard prompt and footer notices.
//!
//! The Win32 panel forwards every event here, runs the queued [`PlayerAction`]s and draws.

use std::time::Instant;

use crate::crop::{CHIPS, CropHandle, drag, fit_ratio};
use crate::editor::{Axis, EditOp, PixelRect, Turn, text as editor_text};
use crate::editor_control::{CropMode, DiscardPrompt, NOTICE_WINDOW};
use crate::editor_geometry::{VideoHit, VideoTool};
use crate::video_edit::{AudioMode, Speed, VideoInfo, VideoOp, VideoPlan, VideoSession, text};

/// Frame time used for stepping when the frame rate is unknown.
const DEFAULT_FPS: f64 = 30.0;

/// Pressing play this close to the last kept end starts over.
const END_SLACK: f64 = 0.05;

/// Something the user asked the video editor to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCommand {
    PlayPause,
    StepBack,
    StepForward,
    BackSecond,
    ForwardSecond,
    Start,
    End,
    Split,
    TogglePiece,
    Crop,
    RotateCw,
    RotateCcw,
    FlipH,
    FlipV,
    Slower,
    Faster,
    CycleSpeed,
    CycleAudio,
    Reverse,
    SaveFrame,
    Undo,
    Redo,
    Enter,
    Escape,
}

/// The command a key press stands for, given the Windows virtual-key code and modifiers.
pub fn video_key(vk: u16, shift: bool, ctrl: bool) -> Option<VideoCommand> {
    use VideoCommand::*;
    if ctrl {
        return match vk {
            0x5A if shift => Some(Redo),
            0x5A => Some(Undo),
            0x59 => Some(Redo),
            _ => None,
        };
    }
    match vk {
        0x20 => Some(PlayPause),
        0x25 if shift => Some(BackSecond),
        0x25 => Some(StepBack),
        0x27 if shift => Some(ForwardSecond),
        0x27 => Some(StepForward),
        0x24 => Some(Start),
        0x23 => Some(End),
        0x53 => Some(Split),
        0x44 => Some(TogglePiece),
        0x43 => Some(Crop),
        0x52 if shift => Some(RotateCcw),
        0x52 => Some(RotateCw),
        0x48 => Some(FlipH),
        0x56 => Some(FlipV),
        0xDB => Some(Slower),
        0xDD => Some(Faster),
        0x4D => Some(CycleAudio),
        0x42 => Some(Reverse),
        0x46 => Some(SaveFrame),
        0x0D => Some(Enter),
        0x1B => Some(Escape),
        _ => None,
    }
}

/// Whether a held key's auto-repeat should act again: only ← and →.
pub fn repeats(vk: u16) -> bool {
    matches!(vk, 0x25 | 0x27)
}

/// Something the panel must tell the media player to do, in order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlayerAction {
    Play,
    Pause,
    /// Go to this many seconds of the original.
    Seek(f64),
    /// One frame forward (`true`) or back.
    StepFrame(bool),
    Rate(f64),
    Mute(bool),
}

/// What the panel should do after an event.
#[derive(Debug, PartialEq)]
pub enum VideoOutcome {
    Nothing,
    Redraw,
    Save(VideoPlan),
    /// Save the picture at `t` seconds with the frame ops applied.
    SaveFrame {
        t: f64,
        ops: Vec<EditOp>,
    },
    Close,
}

/// The video editor's whole interactive state.
#[derive(Debug)]
pub struct VideoState {
    session: Option<VideoSession>,
    crop: Option<CropMode>,
    discard: DiscardPrompt,
    /// A transient footer message and when it goes away.
    notice: Option<(&'static str, Instant)>,
    /// Seconds of the original.
    position: f64,
    playing: bool,
    fallback: bool,
    /// While the timeline is held: whether playback resumes on release.
    scrub: Option<bool>,
    actions: Vec<PlayerAction>,
}

impl VideoState {
    /// The state while the video is still being opened.
    pub fn loading() -> Self {
        Self {
            session: None,
            crop: None,
            discard: DiscardPrompt::default(),
            notice: None,
            position: 0.0,
            playing: false,
            fallback: false,
            scrub: None,
            actions: Vec::new(),
        }
    }

    /// The video's facts are known.
    pub fn loaded(&mut self, info: VideoInfo) {
        self.session = Some(VideoSession::new(info));
        self.position = 0.0;
    }

    pub fn session(&self) -> Option<&VideoSession> {
        self.session.as_ref()
    }

    pub fn crop(&self) -> Option<&CropMode> {
        self.crop.as_ref()
    }

    /// The playback position in seconds of the original.
    pub fn position(&self) -> f64 {
        self.position
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    /// Whether the player failed and frames come from FFmpeg instead.
    pub fn fallback(&self) -> bool {
        self.fallback
    }

    /// Switches to frames from FFmpeg (no playback); the first time, says so in the footer.
    pub fn set_fallback(&mut self, now: Instant) {
        if self.fallback {
            return;
        }
        self.fallback = true;
        self.playing = false;
        self.notify(text::FALLBACK, now);
    }

    /// The player actions queued since the last call, oldest first.
    pub fn take_actions(&mut self) -> Vec<PlayerAction> {
        std::mem::take(&mut self.actions)
    }

    /// Applies `cmd`.
    pub fn command(&mut self, cmd: VideoCommand, now: Instant) -> VideoOutcome {
        let Some(session) = self.session.as_ref() else {
            return if cmd == VideoCommand::Escape {
                VideoOutcome::Close
            } else {
                VideoOutcome::Nothing
            };
        };

        if cmd == VideoCommand::Escape && self.crop.is_none() {
            if !session.has_edits() || self.discard.is_up(now) {
                return VideoOutcome::Close;
            }
            self.discard.arm(now);
            return VideoOutcome::Redraw;
        }

        // Anything else disarms the prompt; that alone is worth a redraw.
        let prompt_cleared = self.discard.clear();
        let outcome = if let Some(outcome) = self.navigate(cmd) {
            outcome
        } else if let Some(crop) = self.crop.take() {
            self.crop_command(crop, cmd)
        } else {
            self.edit_command(cmd, now)
        };
        if outcome == VideoOutcome::Nothing && prompt_cleared {
            VideoOutcome::Redraw
        } else {
            outcome
        }
    }

    /// Playback and seeking, allowed in crop mode too. `None` for other commands.
    fn navigate(&mut self, cmd: VideoCommand) -> Option<VideoOutcome> {
        let session = self.session.as_ref()?;
        let frame = 1.0 / session.info().fps.unwrap_or(DEFAULT_FPS);
        let duration = session.info().duration;
        let (first, last) = (session.first_kept_start(), session.last_kept_end());
        let target = match cmd {
            VideoCommand::PlayPause => return Some(self.play_pause()),
            VideoCommand::StepBack | VideoCommand::StepForward => {
                let forward = cmd == VideoCommand::StepForward;
                let t = self.position + if forward { frame } else { -frame };
                if self.fallback {
                    t
                } else {
                    self.pause();
                    self.position = t.clamp(0.0, duration);
                    self.actions.push(PlayerAction::StepFrame(forward));
                    return Some(VideoOutcome::Redraw);
                }
            }
            VideoCommand::BackSecond => self.position - 1.0,
            VideoCommand::ForwardSecond => self.position + 1.0,
            VideoCommand::Start => first,
            VideoCommand::End => last,
            _ => return None,
        };
        self.seek(target);
        Some(VideoOutcome::Redraw)
    }

    fn play_pause(&mut self) -> VideoOutcome {
        let Some(session) = self.session.as_ref() else {
            return VideoOutcome::Nothing;
        };
        if self.fallback {
            return VideoOutcome::Nothing;
        }
        if self.playing {
            self.pause();
            return VideoOutcome::Redraw;
        }
        let from = match session.skip_from(self.position) {
            Some(t) if self.position < session.last_kept_end() - END_SLACK => t,
            _ => session.first_kept_start(),
        };
        if from != self.position {
            self.seek(from);
        }
        self.playing = true;
        self.actions.push(PlayerAction::Play);
        VideoOutcome::Redraw
    }

    fn crop_command(&mut self, crop: CropMode, cmd: VideoCommand) -> VideoOutcome {
        match cmd {
            VideoCommand::Crop | VideoCommand::Enter => {
                if let Some(session) = self.session.as_mut() {
                    // A crop is never refused; the whole frame is simply not recorded.
                    let _ = session.push(VideoOp::Frame(EditOp::Crop(crop.rect)));
                }
                VideoOutcome::Redraw
            }
            VideoCommand::Escape => VideoOutcome::Redraw,
            _ => {
                self.crop = Some(crop);
                VideoOutcome::Nothing
            }
        }
    }

    fn edit_command(&mut self, cmd: VideoCommand, now: Instant) -> VideoOutcome {
        let Some(session) = self.session.as_mut() else {
            return VideoOutcome::Nothing;
        };
        let op = match cmd {
            VideoCommand::Crop => {
                self.crop = Some(CropMode {
                    rect: PixelRect::full(session.size()),
                    chip: 0,
                    drag: None,
                });
                self.pause();
                return VideoOutcome::Redraw;
            }
            VideoCommand::Enter => {
                return match session.plan() {
                    None => VideoOutcome::Nothing,
                    Some(plan) if plan.reverse && !session.reverse_fits() => {
                        self.notify(text::REVERSE_TOO_LONG, now);
                        VideoOutcome::Redraw
                    }
                    Some(plan) => VideoOutcome::Save(plan),
                };
            }
            VideoCommand::SaveFrame => {
                let ops = session.frame_ops();
                self.pause();
                return VideoOutcome::SaveFrame {
                    t: self.position,
                    ops,
                };
            }
            VideoCommand::Undo | VideoCommand::Redo => {
                let before = (session.speed(), session.audio());
                let changed = if cmd == VideoCommand::Undo {
                    session.undo()
                } else {
                    session.redo()
                };
                self.sync_player(before, now);
                return redraw_if(changed);
            }
            VideoCommand::Split => VideoOp::Split(self.position),
            VideoCommand::TogglePiece => VideoOp::Toggle(self.position),
            VideoCommand::RotateCw => VideoOp::Frame(EditOp::Rotate(Turn::Cw)),
            VideoCommand::RotateCcw => VideoOp::Frame(EditOp::Rotate(Turn::Ccw)),
            VideoCommand::FlipH => VideoOp::Frame(EditOp::Flip(Axis::Horizontal)),
            VideoCommand::FlipV => VideoOp::Frame(EditOp::Flip(Axis::Vertical)),
            VideoCommand::Slower => VideoOp::Speed(session.speed().slower()),
            VideoCommand::Faster => VideoOp::Speed(session.speed().faster()),
            VideoCommand::CycleSpeed => VideoOp::Speed(session.speed().cycle()),
            VideoCommand::CycleAudio => VideoOp::Audio(session.audio().next()),
            VideoCommand::Reverse => VideoOp::Reverse(!session.reverse()),
            // Handled by `command` and `navigate` before dispatch.
            VideoCommand::Escape
            | VideoCommand::PlayPause
            | VideoCommand::StepBack
            | VideoCommand::StepForward
            | VideoCommand::BackSecond
            | VideoCommand::ForwardSecond
            | VideoCommand::Start
            | VideoCommand::End => return VideoOutcome::Nothing,
        };
        let before = (session.speed(), session.audio());
        match session.push(op) {
            Ok(changed) => {
                self.sync_player(before, now);
                redraw_if(changed)
            }
            Err(refusal) => {
                self.notify(refusal.text(), now);
                VideoOutcome::Redraw
            }
        }
    }

    /// Queues `Rate`/`Mute` for whatever changed since `before`; a switch to normalize says
    /// in the footer that it only applies on save.
    fn sync_player(&mut self, before: (Speed, AudioMode), now: Instant) {
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let (speed, audio) = (session.speed(), session.audio());
        if speed != before.0 {
            self.actions.push(PlayerAction::Rate(speed.factor()));
        }
        if audio != before.1 {
            self.actions
                .push(PlayerAction::Mute(audio == AudioMode::Mute));
            if audio == AudioMode::Normalize {
                self.notify(text::NORMALIZE_ON_SAVE, now);
            }
        }
    }

    /// Called by the panel for a key press that [`video_key`] does not map; it disarms the
    /// discard prompt like any other key.
    pub fn key_other(&mut self, now: Instant) -> VideoOutcome {
        let was_up = self.discard.is_up(now);
        self.discard.clear();
        redraw_if(was_up)
    }

    /// Applies a click on `hit`. A press on the bar goes to [`VideoState::timeline_press`].
    pub fn click(&mut self, hit: VideoHit, now: Instant) -> VideoOutcome {
        let cmd = match hit {
            VideoHit::Tool(VideoTool::Crop) => VideoCommand::Crop,
            VideoHit::Tool(VideoTool::Rotate) => VideoCommand::RotateCw,
            VideoHit::Tool(VideoTool::FlipH) => VideoCommand::FlipH,
            VideoHit::Tool(VideoTool::FlipV) => VideoCommand::FlipV,
            VideoHit::Tool(VideoTool::Speed) => VideoCommand::CycleSpeed,
            VideoHit::Tool(VideoTool::Audio) => VideoCommand::CycleAudio,
            VideoHit::Tool(VideoTool::Reverse) => VideoCommand::Reverse,
            VideoHit::Tool(VideoTool::Frame) => VideoCommand::SaveFrame,
            VideoHit::Play => VideoCommand::PlayPause,
            VideoHit::Split => VideoCommand::Split,
            VideoHit::Toggle => VideoCommand::TogglePiece,
            VideoHit::Undo => VideoCommand::Undo,
            VideoHit::Redo => VideoCommand::Redo,
            VideoHit::Save => VideoCommand::Enter,
            VideoHit::Cancel => VideoCommand::Escape,
            VideoHit::Chip(i) => return self.chip(i),
            VideoHit::Bar | VideoHit::Preview | VideoHit::Nothing => {
                return redraw_if(self.discard.clear());
            }
        };
        self.command(cmd, now)
    }

    fn chip(&mut self, i: usize) -> VideoOutcome {
        let cleared = redraw_if(self.discard.clear());
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
            VideoOutcome::Redraw
        } else {
            cleared
        }
    }

    /// A press on the timeline at `t` seconds: pauses (resuming on release) and seeks.
    pub fn timeline_press(&mut self, t: f64, _now: Instant) -> VideoOutcome {
        if self.session.is_none() {
            return VideoOutcome::Nothing;
        }
        self.discard.clear();
        self.scrub = Some(self.playing);
        self.pause();
        self.seek(t);
        VideoOutcome::Redraw
    }

    /// The held timeline moved to `t` seconds.
    pub fn timeline_move(&mut self, t: f64) -> VideoOutcome {
        if self.scrub.is_none() {
            return VideoOutcome::Nothing;
        }
        let before = self.position;
        self.seek(t);
        redraw_if(self.position != before)
    }

    /// The timeline was let go; playback resumes if it was playing at the press.
    pub fn timeline_release(&mut self) -> VideoOutcome {
        match self.scrub.take() {
            Some(true) => {
                self.playing = true;
                self.actions.push(PlayerAction::Play);
                VideoOutcome::Redraw
            }
            Some(false) => VideoOutcome::Redraw,
            None => VideoOutcome::Nothing,
        }
    }

    /// Whether the timeline is being held.
    pub fn scrubbing(&self) -> bool {
        self.scrub.is_some()
    }

    /// The player reports its time `t` (and whether it reached the end) while playing:
    /// skips removed pieces and stops after the last kept one.
    pub fn tick(&mut self, t: f64, ended: bool) -> VideoOutcome {
        let Some(session) = self.session.as_ref().filter(|_| self.playing) else {
            return VideoOutcome::Nothing;
        };
        let before = self.position;
        match session.skip_from(t) {
            _ if ended => self.stop_at_end(),
            Some(next) if next > t + 0.001 => self.seek(next),
            Some(_) => self.position = t,
            None => self.stop_at_end(),
        }
        redraw_if(!self.playing || (self.position - before).abs() >= 0.01)
    }

    fn stop_at_end(&mut self) {
        self.pause();
        if let Some(session) = self.session.as_ref() {
            self.position = session.last_kept_end();
        }
    }

    /// Starts dragging `handle` from the picture point `at`. Ignored outside crop mode.
    pub fn crop_press(&mut self, handle: CropHandle, at: (f64, f64)) {
        if let Some(crop) = self.crop.as_mut() {
            crop.drag = Some((handle, crop.rect, at));
        }
    }

    /// Continues a drag to the picture point `at`. True when the rectangle changed.
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

    /// The footer message to show instead of the sizes, if any.
    pub fn footer_text(&self, hover: VideoHit, now: Instant) -> Option<&'static str> {
        if self.discard.is_up(now) {
            return Some(editor_text::DISCARD);
        }
        if let Some((message, until)) = self.notice
            && now < until
        {
            return Some(message);
        }
        let normalize = self
            .session
            .as_ref()
            .is_some_and(|s| s.audio() == AudioMode::Normalize);
        match hover {
            VideoHit::Tool(VideoTool::Audio) if normalize => Some(text::NORMALIZE_ON_SAVE),
            VideoHit::Play if self.fallback => Some(text::FALLBACK),
            _ => None,
        }
    }

    /// When the last footer message (discard prompt or notice) goes away; redraw then.
    pub fn notice_deadline(&self) -> Option<Instant> {
        self.discard
            .deadline()
            .into_iter()
            .chain(self.notice.map(|(_, until)| until))
            .max()
    }

    fn notify(&mut self, message: &'static str, now: Instant) {
        self.notice = Some((message, now + NOTICE_WINDOW));
    }

    /// Goes to `t` (clamped to the video) and tells the player.
    fn seek(&mut self, t: f64) {
        let duration = self.session.as_ref().map_or(0.0, |s| s.info().duration);
        self.position = t.clamp(0.0, duration);
        self.actions.push(PlayerAction::Seek(self.position));
    }

    fn pause(&mut self) {
        if self.playing {
            self.playing = false;
            self.actions.push(PlayerAction::Pause);
        }
    }
}

fn redraw_if(changed: bool) -> VideoOutcome {
    if changed {
        VideoOutcome::Redraw
    } else {
        VideoOutcome::Nothing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready(duration: f64) -> VideoState {
        let mut s = VideoState::loading();
        s.loaded(VideoInfo {
            width: 1280,
            height: 720,
            duration,
            fps: Some(25.0),
            has_audio: true,
        });
        s
    }

    #[test]
    fn keys() {
        assert_eq!(video_key(0x20, false, false), Some(VideoCommand::PlayPause));
        assert_eq!(video_key(0x25, true, false), Some(VideoCommand::BackSecond));
        assert_eq!(
            video_key(0x27, false, false),
            Some(VideoCommand::StepForward)
        );
        assert_eq!(video_key(0xDD, false, false), Some(VideoCommand::Faster));
        assert_eq!(
            video_key(0x44, false, false),
            Some(VideoCommand::TogglePiece)
        );
        assert_eq!(video_key(0x5A, true, true), Some(VideoCommand::Redo));
        assert_eq!(video_key(0x53, false, true), None);
        assert!(repeats(0x25) && !repeats(0x20));
    }

    #[test]
    fn play_skips_removed_pieces_and_stops_at_the_end() {
        let now = Instant::now();
        let mut s = ready(10.0);
        s.timeline_press(3.0, now);
        s.timeline_release();
        s.command(VideoCommand::Split, now);
        s.timeline_press(6.0, now);
        s.timeline_release();
        s.command(VideoCommand::Split, now);
        s.timeline_press(4.0, now);
        s.timeline_release();
        s.command(VideoCommand::TogglePiece, now); // remove [3, 6)
        s.take_actions();
        s.command(VideoCommand::PlayPause, now);
        assert_eq!(
            s.take_actions(),
            vec![PlayerAction::Seek(6.0), PlayerAction::Play]
        );
        s.tick(9.99, true);
        assert_eq!(s.take_actions(), vec![PlayerAction::Pause]);
        s.command(VideoCommand::PlayPause, now); // at the end: from the first kept start
        assert_eq!(
            s.take_actions(),
            vec![PlayerAction::Seek(0.0), PlayerAction::Play]
        );
        s.tick(3.2, false);
        assert_eq!(s.take_actions(), vec![PlayerAction::Seek(6.0)]);
    }

    #[test]
    fn speed_and_mute_reach_the_player_and_undo() {
        let now = Instant::now();
        let mut s = ready(10.0);
        s.command(VideoCommand::Faster, now);
        s.command(VideoCommand::CycleAudio, now);
        assert_eq!(
            s.take_actions(),
            vec![PlayerAction::Rate(1.5), PlayerAction::Mute(true)]
        );
        s.command(VideoCommand::Undo, now);
        assert_eq!(s.take_actions(), vec![PlayerAction::Mute(false)]);
    }

    #[test]
    fn refusals_show_a_notice() {
        let now = Instant::now();
        let mut s = ready(10.0);
        assert_eq!(
            s.command(VideoCommand::TogglePiece, now),
            VideoOutcome::Redraw
        );
        assert_eq!(
            s.footer_text(VideoHit::Nothing, now),
            Some(text::LAST_PIECE)
        );
        let mut long = ready(600.0);
        long.command(VideoCommand::Reverse, now);
        assert_eq!(
            long.footer_text(VideoHit::Nothing, now),
            Some(text::REVERSE_TOO_LONG)
        );
        assert_eq!(
            long.footer_text(VideoHit::Nothing, now + NOTICE_WINDOW),
            None
        );
    }

    #[test]
    fn crop_mode_pauses_and_blocks_tools() {
        let now = Instant::now();
        let mut s = ready(10.0);
        s.command(VideoCommand::PlayPause, now);
        s.take_actions();
        s.command(VideoCommand::Crop, now);
        assert_eq!(s.take_actions(), vec![PlayerAction::Pause]);
        assert_eq!(
            s.command(VideoCommand::RotateCw, now),
            VideoOutcome::Nothing
        );
        assert_eq!(s.command(VideoCommand::Split, now), VideoOutcome::Nothing);
        s.command(VideoCommand::Escape, now);
        assert!(s.crop().is_none());
    }

    #[test]
    fn save_discard_and_frame() {
        let now = Instant::now();
        let mut s = ready(10.0);
        assert_eq!(s.command(VideoCommand::Enter, now), VideoOutcome::Nothing);
        assert!(
            matches!(s.command(VideoCommand::SaveFrame, now), VideoOutcome::SaveFrame { t, .. } if t == 0.0)
        );
        s.command(VideoCommand::RotateCw, now);
        assert!(matches!(
            s.command(VideoCommand::Enter, now),
            VideoOutcome::Save(_)
        ));
        assert_eq!(s.command(VideoCommand::Escape, now), VideoOutcome::Redraw);
        assert_eq!(s.command(VideoCommand::Escape, now), VideoOutcome::Close);
    }

    #[test]
    fn fallback_steps_by_frame_time_and_cannot_play() {
        let now = Instant::now();
        let mut s = ready(10.0);
        s.set_fallback(now);
        assert_eq!(s.footer_text(VideoHit::Nothing, now), Some(text::FALLBACK));
        s.command(VideoCommand::StepForward, now);
        assert_eq!(s.take_actions(), vec![PlayerAction::Seek(0.04)]);
        s.command(VideoCommand::PlayPause, now);
        assert!(s.take_actions().is_empty());
    }

    #[test]
    fn timeline_drag_pauses_and_resumes() {
        let now = Instant::now();
        let mut s = ready(10.0);
        s.command(VideoCommand::PlayPause, now);
        s.take_actions();
        s.timeline_press(2.0, now);
        s.timeline_move(2.5);
        s.timeline_release();
        assert_eq!(
            s.take_actions(),
            vec![
                PlayerAction::Pause,
                PlayerAction::Seek(2.0),
                PlayerAction::Seek(2.5),
                PlayerAction::Play
            ]
        );
    }
}
