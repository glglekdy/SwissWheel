//! Edit history for the video editor: pieces of the timeline, frame ops, speed, sound and
//! reverse, with undo and redo.

use crate::editor::{Affine, EditOp, History, PixelRect, frame_geometry};
use crate::media::MediaInfo;

/// No split closer than this to the start, the end or another cut, in seconds.
const MIN_PIECE: f64 = 0.1;

/// Reverse buffers every decoded frame; past this many bytes it is refused (1 GiB).
const REVERSE_LIMIT: f64 = 1_073_741_824.0;

/// A playback speed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speed {
    Half,
    Normal,
    OneHalf,
    Double,
}

impl Speed {
    pub fn factor(self) -> f64 {
        match self {
            Speed::Half => 0.5,
            Speed::Normal => 1.0,
            Speed::OneHalf => 1.5,
            Speed::Double => 2.0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Speed::Half => "0.5×",
            Speed::Normal => "1×",
            Speed::OneHalf => "1.5×",
            Speed::Double => "2×",
        }
    }

    /// One step slower, stopping at 0.5×.
    pub fn slower(self) -> Speed {
        match self {
            Speed::Half | Speed::Normal => Speed::Half,
            Speed::OneHalf => Speed::Normal,
            Speed::Double => Speed::OneHalf,
        }
    }

    /// One step faster, stopping at 2×.
    pub fn faster(self) -> Speed {
        match self {
            Speed::Half => Speed::Normal,
            Speed::Normal => Speed::OneHalf,
            Speed::OneHalf | Speed::Double => Speed::Double,
        }
    }

    /// The speed button's order: 1× → 1.5× → 2× → 0.5× → 1×.
    pub fn cycle(self) -> Speed {
        match self {
            Speed::Normal => Speed::OneHalf,
            Speed::OneHalf => Speed::Double,
            Speed::Double => Speed::Half,
            Speed::Half => Speed::Normal,
        }
    }
}

/// What happens to the sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioMode {
    Keep,
    Mute,
    Normalize,
}

impl AudioMode {
    /// The sound button's order: keep → mute → normalize → keep.
    pub fn next(self) -> AudioMode {
        match self {
            AudioMode::Keep => AudioMode::Mute,
            AudioMode::Mute => AudioMode::Normalize,
            AudioMode::Normalize => AudioMode::Keep,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AudioMode::Keep => text::SOUND,
            AudioMode::Mute => text::MUTE,
            AudioMode::Normalize => text::NORMALIZE,
        }
    }
}

/// One recorded video edit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VideoOp {
    /// A crop, turn or flip of the picture.
    Frame(EditOp),
    /// Cut the piece holding this time (seconds) in two.
    Split(f64),
    /// Remove or restore the piece holding this time.
    Toggle(f64),
    Speed(Speed),
    Audio(AudioMode),
    Reverse(bool),
}

/// One piece of the timeline, in seconds of the original.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub kept: bool,
}

/// What the editor needs to know about the video. Sizes are display sizes.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub fps: Option<f64>,
    pub has_audio: bool,
}

impl VideoInfo {
    /// The editable facts of probed media: `None` without a video stream, a size or a
    /// positive duration.
    pub fn from_media(info: &MediaInfo) -> Option<VideoInfo> {
        info.video.as_ref()?;
        let duration = info.duration_secs.filter(|d| *d > 0.0)?;
        Some(VideoInfo {
            width: info.width?,
            height: info.height?,
            duration,
            fps: info.fps,
            has_audio: info.audio.is_some(),
        })
    }
}

/// Why an op was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Removing the only kept piece.
    LastSegment,
    /// Reverse past the memory limit.
    ReverseTooLong,
}

impl Refusal {
    pub fn text(self) -> &'static str {
        match self {
            Refusal::LastSegment => text::LAST_PIECE,
            Refusal::ReverseTooLong => text::REVERSE_TOO_LONG,
        }
    }
}

/// What a save applies: kept ranges in seconds of the original, then the frame ops, speed,
/// sound and reverse.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoPlan {
    pub kept: Vec<(f64, f64)>,
    pub frame: Vec<EditOp>,
    pub speed: Speed,
    pub audio: AudioMode,
    pub reverse: bool,
}

/// The edits made so far on one video, with undo and redo.
#[derive(Debug, Clone)]
pub struct VideoSession {
    info: VideoInfo,
    history: History<VideoOp>,
}

impl VideoSession {
    pub fn new(info: VideoInfo) -> Self {
        Self {
            info,
            history: History::new(),
        }
    }

    pub fn info(&self) -> &VideoInfo {
        &self.info
    }

    /// Records `op`. `Ok(false)` records nothing for a no-op: a split within 0.1 s of the
    /// start, the end or a cut, a value equal to the current one, sound ops without sound,
    /// or a crop of the whole frame.
    pub fn push(&mut self, op: VideoOp) -> Result<bool, Refusal> {
        let accepted = match op {
            VideoOp::Frame(frame) => match frame {
                EditOp::Crop(r) => r != PixelRect::full(self.size()),
                EditOp::Rotate(_) | EditOp::Flip(_) => true,
                EditOp::Grayscale(_) | EditOp::StripInfo(_) => false,
            },
            VideoOp::Split(t) => {
                let duration = self.info.duration;
                t > 0.0
                    && t < duration
                    && std::iter::once(0.0)
                        .chain(self.segments().iter().map(|s| s.end))
                        .all(|cut| (t - cut).abs() >= MIN_PIECE)
            }
            VideoOp::Toggle(t) => {
                let piece = self.segment_at(t);
                if piece.kept && self.segments().iter().filter(|s| s.kept).count() == 1 {
                    return Err(Refusal::LastSegment);
                }
                true
            }
            VideoOp::Speed(speed) => speed != self.speed(),
            VideoOp::Audio(mode) => self.info.has_audio && mode != self.audio(),
            VideoOp::Reverse(on) => {
                if on == self.reverse() {
                    false
                } else if on && !self.reverse_fits() {
                    return Err(Refusal::ReverseTooLong);
                } else {
                    true
                }
            }
        };
        if accepted {
            self.history.push(op);
        }
        Ok(accepted)
    }

    pub fn undo(&mut self) -> bool {
        self.history.undo()
    }

    pub fn redo(&mut self) -> bool {
        self.history.redo()
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// The pieces of the whole timeline, in order: the splits and toggles replayed over
    /// `[0, duration]`.
    pub fn segments(&self) -> Vec<Segment> {
        let mut pieces = vec![Segment {
            start: 0.0,
            end: self.info.duration,
            kept: true,
        }];
        for op in self.history.ops() {
            match *op {
                VideoOp::Split(t) => {
                    let i = piece_index(&pieces, t);
                    let piece = pieces[i];
                    pieces[i].end = t;
                    pieces.insert(i + 1, Segment { start: t, ..piece });
                }
                VideoOp::Toggle(t) => {
                    let i = piece_index(&pieces, t);
                    pieces[i].kept = !pieces[i].kept;
                }
                _ => {}
            }
        }
        pieces
    }

    /// The piece holding `t`: `[start, end)`, except the last piece, which also holds the end.
    pub fn segment_at(&self, t: f64) -> Segment {
        let pieces = self.segments();
        pieces[piece_index(&pieces, t)]
    }

    pub fn is_kept(&self, t: f64) -> bool {
        self.segment_at(t).kept
    }

    /// The kept time ranges, adjacent kept pieces merged.
    pub fn kept_ranges(&self) -> Vec<(f64, f64)> {
        let mut ranges: Vec<(f64, f64)> = Vec::new();
        for s in self.segments().into_iter().filter(|s| s.kept) {
            match ranges.last_mut() {
                Some(last) if last.1 == s.start => last.1 = s.end,
                _ => ranges.push((s.start, s.end)),
            }
        }
        ranges
    }

    /// Seconds of the original that are kept.
    pub fn kept_duration(&self) -> f64 {
        self.kept_ranges().iter().map(|(a, b)| b - a).sum()
    }

    /// Seconds of the saved result: kept seconds over the speed.
    pub fn output_duration(&self) -> f64 {
        self.kept_duration() / self.speed().factor()
    }

    /// Where playback at `t` should be: `t` itself when kept, else the start of the next kept
    /// piece; `None` when no kept piece follows.
    pub fn skip_from(&self, t: f64) -> Option<f64> {
        if self.is_kept(t) {
            return Some(t);
        }
        self.segments()
            .iter()
            .find(|s| s.kept && s.start > t)
            .map(|s| s.start)
    }

    pub fn first_kept_start(&self) -> f64 {
        self.kept_ranges().first().map_or(0.0, |r| r.0)
    }

    pub fn last_kept_end(&self) -> f64 {
        self.kept_ranges()
            .last()
            .map_or(self.info.duration, |r| r.1)
    }

    /// The crops, turns and flips, oldest first.
    pub fn frame_ops(&self) -> Vec<EditOp> {
        self.history
            .ops()
            .iter()
            .filter_map(|op| match op {
                VideoOp::Frame(frame) => Some(*frame),
                _ => None,
            })
            .collect()
    }

    /// The picture size after the frame ops.
    pub fn size(&self) -> (u32, u32) {
        frame_geometry((self.info.width, self.info.height), &self.frame_ops()).0
    }

    /// The map from original picture coordinates to result coordinates.
    pub fn to_result(&self) -> Affine {
        frame_geometry((self.info.width, self.info.height), &self.frame_ops()).1
    }

    pub fn speed(&self) -> Speed {
        self.last(|op| match op {
            VideoOp::Speed(s) => Some(*s),
            _ => None,
        })
        .unwrap_or(Speed::Normal)
    }

    pub fn audio(&self) -> AudioMode {
        self.last(|op| match op {
            VideoOp::Audio(a) => Some(*a),
            _ => None,
        })
        .unwrap_or(AudioMode::Keep)
    }

    pub fn reverse(&self) -> bool {
        self.last(|op| match op {
            VideoOp::Reverse(on) => Some(*on),
            _ => None,
        })
        .unwrap_or(false)
    }

    /// Whether reversing the kept pieces stays under the memory limit
    /// (`width × height × 1.5 × fps × kept seconds`); false when the frame rate is unknown.
    pub fn reverse_fits(&self) -> bool {
        let Some(fps) = self.info.fps else {
            return false;
        };
        let (w, h) = self.size();
        f64::from(w) * f64::from(h) * 1.5 * fps * self.kept_duration() <= REVERSE_LIMIT
    }

    /// Whether a save would change anything. Splits alone do not.
    pub fn has_edits(&self) -> bool {
        self.segments().iter().any(|s| !s.kept)
            || !self.frame_ops().is_empty()
            || self.speed() != Speed::Normal
            || self.audio() != AudioMode::Keep
            || self.reverse()
    }

    /// What to save, or `None` when there is nothing to save.
    pub fn plan(&self) -> Option<VideoPlan> {
        self.has_edits().then(|| VideoPlan {
            kept: self.kept_ranges(),
            frame: self.frame_ops(),
            speed: self.speed(),
            audio: self.audio(),
            reverse: self.reverse(),
        })
    }

    fn last<T>(&self, pick: impl Fn(&VideoOp) -> Option<T>) -> Option<T> {
        self.history.ops().iter().rev().find_map(pick)
    }
}

/// The index of the piece holding `t`; times past the end fall in the last piece.
fn piece_index(pieces: &[Segment], t: f64) -> usize {
    pieces
        .iter()
        .position(|s| t < s.end)
        .unwrap_or(pieces.len() - 1)
}

/// `m:ss.d`, from one hour `h:mm:ss.d`.
pub fn format_time(t: f64) -> String {
    let tenths = (t.max(0.0) * 10.0).round() as u64;
    let (h, m, s, d) = (
        tenths / 36_000,
        tenths / 600 % 60,
        tenths / 10 % 60,
        tenths % 10,
    );
    if h > 0 {
        format!("{h}:{m:02}:{s:02}.{d}")
    } else {
        format!("{m}:{s:02}.{d}")
    }
}

/// Copy shown by the video editor (Korean).
pub mod text {
    pub const REMOVED: &str = "빠진 구간";
    pub const LAST_PIECE: &str = "조각을 모두 뺄 수는 없습니다";
    pub const NORMALIZE_ON_SAVE: &str = "소리 크기는 저장할 때 맞춥니다";
    pub const REVERSE_TOO_LONG: &str =
        "역재생은 짧은 영상만 할 수 있습니다 (1080p 기준 약 10초). 구간을 줄이면 켤 수 있어요";
    pub const FALLBACK: &str = "재생할 수 없는 형식이라 장면만 넘겨봅니다";
    pub const SCRUB_FAILED: &str = "장면을 불러오지 못했습니다";
    pub const UNREADABLE: &str = "영상을 읽을 수 없습니다";
    pub const SPLIT: &str = "나누기";
    pub const REMOVE: &str = "빼기";
    pub const RESTORE: &str = "되살리기";
    pub const AREA: &str = "영역";
    pub const ROTATE: &str = "회전";
    pub const FLIP_H: &str = "좌우";
    pub const FLIP_V: &str = "상하";
    pub const SOUND: &str = "소리";
    pub const MUTE: &str = "음소거";
    pub const NORMALIZE: &str = "크기 맞춤";
    /// The tool label and the badge on the preview.
    pub const REVERSE: &str = "역재생";
    pub const FRAME: &str = "장면";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Turn;

    fn info(duration: f64) -> VideoInfo {
        VideoInfo {
            width: 1920,
            height: 1080,
            duration,
            fps: Some(30.0),
            has_audio: true,
        }
    }

    #[test]
    fn split_and_remove_pieces() {
        let mut s = VideoSession::new(info(10.0));
        assert_eq!(s.push(VideoOp::Split(3.0)), Ok(true));
        assert_eq!(s.push(VideoOp::Split(6.0)), Ok(true));
        assert_eq!(s.push(VideoOp::Split(6.05)), Ok(false)); // near a cut
        assert_eq!(s.push(VideoOp::Split(0.05)), Ok(false)); // near the start
        assert!(!s.has_edits()); // splits alone change nothing
        assert_eq!(s.push(VideoOp::Toggle(4.0)), Ok(true));
        assert_eq!(s.kept_ranges(), vec![(0.0, 3.0), (6.0, 10.0)]);
        assert_eq!(s.kept_duration(), 7.0);
        assert!(s.has_edits());
        assert_eq!(s.skip_from(4.5), Some(6.0));
        assert_eq!(s.skip_from(1.0), Some(1.0));
        assert_eq!(s.push(VideoOp::Toggle(4.0)), Ok(true)); // restore
        assert_eq!(s.kept_ranges(), vec![(0.0, 10.0)]); // neighbours merge
        assert_eq!(s.segments().len(), 3);
        assert!(s.undo());
        assert_eq!(s.kept_ranges(), vec![(0.0, 3.0), (6.0, 10.0)]);
    }

    #[test]
    fn the_last_kept_piece_stays() {
        let mut s = VideoSession::new(info(10.0));
        assert_eq!(s.push(VideoOp::Toggle(1.0)), Err(Refusal::LastSegment));
        s.push(VideoOp::Split(5.0)).unwrap();
        s.push(VideoOp::Toggle(9.0)).unwrap();
        assert_eq!(s.skip_from(7.0), None);
        assert_eq!(s.last_kept_end(), 5.0);
        assert_eq!(s.push(VideoOp::Toggle(1.0)), Err(Refusal::LastSegment));
        assert_eq!(
            s.segment_at(10.0),
            Segment {
                start: 5.0,
                end: 10.0,
                kept: false
            }
        );
    }

    #[test]
    fn speed_changes_output_duration() {
        let mut s = VideoSession::new(info(10.0));
        s.push(VideoOp::Speed(Speed::Double)).unwrap();
        assert_eq!(s.output_duration(), 5.0);
        assert_eq!(Speed::Normal.cycle(), Speed::OneHalf);
        assert_eq!(Speed::Half.cycle(), Speed::Normal);
        assert_eq!(Speed::Double.faster(), Speed::Double);
        assert_eq!(Speed::Normal.slower(), Speed::Half);
    }

    #[test]
    fn reverse_limit() {
        // 1920×1080×1.5×30 = 93_312_000 bytes per second → 11.5 s fits, 11.6 s does not.
        let mut long = VideoSession::new(info(11.6));
        assert_eq!(
            long.push(VideoOp::Reverse(true)),
            Err(Refusal::ReverseTooLong)
        );
        let mut short = VideoSession::new(info(11.5));
        assert_eq!(short.push(VideoOp::Reverse(true)), Ok(true));
        let mut unknown = VideoSession::new(VideoInfo {
            fps: None,
            ..info(1.0)
        });
        assert_eq!(
            unknown.push(VideoOp::Reverse(true)),
            Err(Refusal::ReverseTooLong)
        );
        // Cutting a long clip down makes it fit.
        long.push(VideoOp::Split(5.0)).unwrap();
        long.push(VideoOp::Toggle(8.0)).unwrap();
        assert_eq!(long.push(VideoOp::Reverse(true)), Ok(true));
        // Restoring the piece afterwards is allowed, but no longer fits (save refuses).
        long.push(VideoOp::Toggle(8.0)).unwrap();
        assert!(!long.reverse_fits());
    }

    #[test]
    fn audio_ops_ignored_without_audio() {
        let mut s = VideoSession::new(VideoInfo {
            has_audio: false,
            ..info(5.0)
        });
        assert_eq!(s.push(VideoOp::Audio(AudioMode::Mute)), Ok(false));
        assert_eq!(s.audio(), AudioMode::Keep);
    }

    #[test]
    fn frame_ops_and_plan() {
        let mut s = VideoSession::new(info(4.0));
        assert!(s.plan().is_none());
        s.push(VideoOp::Frame(EditOp::Rotate(Turn::Cw))).unwrap();
        assert_eq!(s.size(), (1080, 1920));
        s.push(VideoOp::Audio(AudioMode::Normalize)).unwrap();
        let plan = s.plan().unwrap();
        assert_eq!(plan.kept, vec![(0.0, 4.0)]);
        assert_eq!(plan.frame, vec![EditOp::Rotate(Turn::Cw)]);
        assert_eq!(
            (plan.speed, plan.audio, plan.reverse),
            (Speed::Normal, AudioMode::Normalize, false)
        );
    }

    #[test]
    fn video_info_needs_size_and_duration() {
        let full = MediaInfo {
            video: Some("h264".into()),
            width: Some(640),
            height: Some(360),
            duration_secs: Some(2.0),
            ..Default::default()
        };
        assert!(VideoInfo::from_media(&full).is_some());
        assert!(
            VideoInfo::from_media(&MediaInfo {
                duration_secs: None,
                ..full.clone()
            })
            .is_none()
        );
        assert!(
            VideoInfo::from_media(&MediaInfo {
                width: None,
                ..full.clone()
            })
            .is_none()
        );
        assert!(
            VideoInfo::from_media(&MediaInfo {
                video: None,
                ..full
            })
            .is_none()
        );
    }

    #[test]
    fn time_text() {
        assert_eq!(format_time(0.0), "0:00.0");
        assert_eq!(format_time(75.04), "1:15.0");
        assert_eq!(format_time(3723.4), "1:02:03.4");
    }
}
