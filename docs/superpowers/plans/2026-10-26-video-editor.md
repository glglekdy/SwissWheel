# Video Editor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Dropping one video on the ✎편집 wheel tile opens the floating editor panel with sound playback (Media Foundation), a timeline where the user splits and removes pieces, and tools for area crop, rotate, flip, speed, sound (mute / normalize), short reverse and saving the current frame as PNG; saving writes `clip (edited).ext` next to the original with FFmpeg.

**Architecture:** All rules — the video edit history, pieces and kept ranges, the reverse limit, layout and hit testing, key/click/timeline handling, and every FFmpeg argument — live in `swisswheel-core` and are unit tested. The app crate adds `editor/video.rs` (a second panel slot beside the image panel, sharing its windows code), `editor/player.rs` (an `IMFMediaEngine` in frame-server mode drawing into the panel's own D3D11 device), `editor/scrub.rs` (FFmpeg single-frame fallback) and `editor/video_render.rs`. Saves become ordinary jobs through a new `Edits` enum on `Job`.

**Tech Stack:** Rust 2024, `windows` 0.61 (Win32, Direct2D, DirectWrite, Direct3D11, Media Foundation, Composition via `Surface`), `windows-core` `#[implement]`, FFmpeg/ffprobe (external, located by `engines::ffmpeg::locate`).

**Spec:** `docs/superpowers/specs/2026-10-26-video-editor-design.md` (read with the image editor spec `docs/superpowers/specs/2026-10-25-image-editor-design.md`, whose panel rules it reuses).

## Global Constraints

- Originals are never modified; outputs go through the existing reserve → temp → `finalize` flow in `jobs.rs`.
- Output names: video save `이름 (edited).ext` in the input's container (WMV input → `.mp4`); frame save `이름 (frame 0m12.40s).png`, from one hour `이름 (frame 1h02m03.40s).png`.
- Encoders (unchanged from conversion): MP4·MOV·MKV(·WMV→MP4) `libx264 -preset medium -crf 23 -pix_fmt yuv420p` + `aac -b:a 192k`, MP4·MOV add `-movflags +faststart`; WebM `libvpx-vp9 -crf 32 -b:v 0 -deadline good -cpu-used 4 -row-mt 1 -pix_fmt yuv420p` + `libopus -b:a 128k` with `aformat=channel_layouts=7.1|5.1|stereo|mono`; AVI `mpeg4 -q:v 4` + `libmp3lame -b:a {quality.mp3_kbps}k`.
- Loudness: `loudnorm=I=-16:TP=-1.5:LRA=11`, then `-ar <source sample rate>` (unknown → `48000`; WebM always `48000`).
- Speeds: exactly 0.5, 1, 1.5, 2. Reverse limit: `out_w × out_h × 1.5 × fps × kept_seconds > 1_073_741_824` (or fps unknown) refuses.
- Split rule: no split within 0.1 s of the start, the end, or an existing cut. The last kept piece cannot be removed.
- The ✎편집 tile appears for exactly one still image (unchanged) or exactly one video (MP4·MKV·WebM·MOV·AVI·WMV), last in the Tools wheel.
- Keys (video panel): Space play/pause; ←/→ one frame (auto-repeat allowed for these two only); Shift+←/→ 1 s; Home/End; S split; D remove/restore; C area; R / Shift+R; H; V; `[` slower; `]` faster; M sound; B reverse; F frame; Ctrl+Z; Ctrl+Y and Ctrl+Shift+Z; Enter; Esc; Alt+F4 acts as Esc. VK codes: Space `0x20`, ← `0x25`, → `0x27`, Home `0x24`, End `0x23`, S `0x53`, D `0x44`, `[` `0xDB`, `]` `0xDD`, M `0x4D`, B `0x42`, F `0x46`.
- Copy (Korean, exact, in `video_edit::text`): `"빠진 구간"`, `"역재생"`, `"조각을 모두 뺄 수는 없습니다"`, `"소리 크기는 저장할 때 맞춥니다"`, `"역재생은 짧은 영상만 할 수 있습니다 (1080p 기준 약 10초). 구간을 줄이면 켤 수 있어요"`, `"재생할 수 없는 형식이라 장면만 넘겨봅니다"`, `"장면을 불러오지 못했습니다"`, `"영상을 읽을 수 없습니다"`, buttons `"나누기"`, `"빼기"`, `"되살리기"`, tool labels `"영역"`, `"회전"`, `"좌우"`, `"상하"`, speed `"0.5×"`/`"1×"`/`"1.5×"`/`"2×"`, `"소리"`/`"음소거"`/`"크기 맞춤"`, `"역재생"`, `"장면"`. Reused from `editor::text`: `"편집"`, `"불러오는 중"`, the discard prompt, `"편집기가 이미 열려 있습니다"`, `"편집기를 열 수 없습니다"`, footer buttons.
- Transient notices last 3 s (same as the discard prompt).
- Glyphs (Segoe MDL2/Fluent): area `E7A8`, rotate `E7AD`, flip H `E8AB`, flip V `E8CB`, speed `E916`, sound `E767` / mute `E74F` / normalize `E8D6`, reverse `E8EE`, frame `E722`, play `E768`, pause `E769`, split `E8C6`, remove `E738`, restore `E710`.
- Panel-private messages: `WM_APP_EDITOR_LOADED = WM_APP + 50` (image, existing), `WM_APP_PLAYER = WM_APP + 51` (engine event code in `wParam`), `WM_APP_SCRUB = WM_APP + 52`. Timers on the video panel: `1` notices/discard, `2` playback at 16 ms.
- Time display `m:ss.d` (`1:30.0`), from one hour `h:mm:ss.d`.
- Running the app: only with `SWISSWHEEL_PREVIEW` set (a preview never rewrites the autostart Run value). Before any screen check, look at the foreground window and skip the check if a full-screen game owns it; list skipped checks in the task report. Never stop the user's installed `swisswheel.exe`.
- FFmpeg tests need `ffmpeg.exe`/`ffprobe.exe` on `PATH` like the existing engine tests.
- Never stage files you didn't change; stage by name.

## Rulings (decided while planning, recorded in the spec)

- **R1** No pre-tile FFmpeg install prompt exists in the code; opening a video without FFmpeg shows `EngineError::MissingFfmpeg.user_message(name)` as a warning card and opens nothing.
- **R2** ffprobe runs synchronously in `open` (before any window), like the image editor's header read; "불러오는 중" covers the media engine loading.
- **R3** The video panel is a separate slot (`editor/video.rs`, its own window procedure) instead of a `Content` enum; `editor::{open, is_open, focus, close, take_events}` cover both slots.
- **R4** `MediaPlan` gains `input_args: Vec<String>` placed before `-i`, for accurate fast input seeking (`-ss`/`-t`).
- **R5** WebM normalize always resamples to 48000 (Opus).

## Review Focus

1. A video without an audio stream must save every edit (cuts, speed, reverse) without an audio graph → Task 4 test `plan_without_audio_has_no_audio_graph`, Task 3 test `audio_ops_ignored_without_audio`.
2. A phone portrait clip (display-matrix rotation) must crop in display coordinates and come out upright → Task 2 test `rotation_swaps_display_size`, Task 5 test `rotated_input_crops_in_display_space`.
3. Odd crop sizes must encode with H.264/VP9 → Task 4 test `odd_crop_is_floored_to_even`, Task 5 test `odd_crop_encodes`.
4. Media whose ffprobe output lacks a duration or size cannot be edited and must refuse cleanly → Task 3 test `video_info_needs_size_and_duration`.
5. WebM with 44.1 kHz audio plus normalize must not hand Opus an unsupported rate → Task 4 test `webm_normalize_uses_48k`, Task 5 test `webm_normalize_saves`.

---

### Task 1: Share history, geometry and the discard prompt

Pure refactor of the image editor; behavior unchanged.

**Files:**
- Modify: `crates/swisswheel-core/src/editor.rs`
- Modify: `crates/swisswheel-core/src/editor_control.rs`

**Interfaces:**
- Produces (in `editor.rs`):
  - `pub struct History<Op> { .. }` with `new() -> Self`, `push(&mut self, op: Op)` (clears redo), `undo(&mut self) -> bool`, `redo(&mut self) -> bool`, `can_undo(&self) -> bool`, `can_redo(&self) -> bool`, `ops(&self) -> &[Op]`. `#[derive(Debug, Clone)]`, `impl<Op> Default`.
  - `pub fn frame_geometry<'a>(original: (u32, u32), ops: impl IntoIterator<Item = &'a EditOp>) -> ((u32, u32), Affine)` — folds `Crop`/`Rotate`/`Flip`, skips other ops. `EditSession::size()` and `to_result()` call it.
- Produces (in `editor_control.rs`):
  - `pub struct DiscardPrompt { until: Option<Instant> }` (`Default`) with `arm(&mut self, now: Instant)`, `is_up(&self, now: Instant) -> bool`, `clear(&mut self) -> bool` (true if it was set), `deadline(&self) -> Option<Instant>`. `pub const NOTICE_WINDOW: Duration = 3 s` (rename of `DISCARD_WINDOW`, now public). `EditorState` uses it.

- [ ] **Step 1: Write the failing tests** (in `editor.rs` tests)

```rust
#[test]
fn history_undo_redo_and_new_push_clears_redo() {
    let mut h = History::new();
    h.push(1);
    h.push(2);
    assert!(h.undo());
    assert_eq!(h.ops(), &[1]);
    assert!(h.can_redo());
    h.push(3);
    assert!(!h.can_redo());
    assert_eq!(h.ops(), &[1, 3]);
    assert!(h.undo() && h.undo() && !h.undo());
}

#[test]
fn frame_geometry_skips_non_frame_ops() {
    let ops = [
        EditOp::Rotate(Turn::Cw),
        EditOp::Grayscale(true),
        EditOp::Crop(PixelRect { x: 10, y: 20, w: 30, h: 40 }),
    ];
    let (size, a) = frame_geometry((200, 100), &ops);
    assert_eq!(size, (30, 40));
    // original (0, 0) → rotated (100, 0) → cropped (90, -20)
    assert_eq!(a.apply(0.0, 0.0), (90.0, -20.0));
}
```

- [ ] **Step 2: Run** `cargo test -p swisswheel-core editor` — Expected: FAIL (no `History`, no `frame_geometry`).
- [ ] **Step 3: Implement** `History`, `frame_geometry`, `DiscardPrompt`; make `EditSession` hold `History<EditOp>` and `EditorState` hold `DiscardPrompt`. Keep every public method of `EditSession`/`EditorState` with the same signature.
- [ ] **Step 4: Run** `cargo test --workspace` — Expected: PASS, including every existing editor test unchanged.
- [ ] **Step 5: Commit** `refactor(core): share edit history, frame geometry and the discard prompt`

---

### Task 2: ffprobe reads size, rate, sample rate and rotation

**Files:**
- Modify: `crates/swisswheel-core/src/media.rs` (`MediaInfo`, `parse_probe`, `MediaPlan`)
- Modify: `crates/swisswheel/src/engines/ffmpeg.rs` (`probe` args, `run_once` uses `input_args`, tests)

**Interfaces:**
- Produces:
  - `MediaInfo { duration_secs, video, audio, width: Option<u32>, height: Option<u32>, fps: Option<f64>, sample_rate: Option<u32>, rotation: u32 }` — `width`/`height` are **display** size (swapped when `rotation` is 90 or 270), from the first video stream; `fps` = `avg_frame_rate`, falling back to `r_frame_rate` when that is `0/0` or missing; `sample_rate` from the first audio stream; `rotation` = ffprobe's `rotation` of the first video stream normalized into `{0, 90, 180, 270}` via `rem_euclid(360)` (so `-90` → `270`). `Default` keeps working.
  - `MediaPlan { output, input_args: Vec<String>, attempts, duration_secs }`; every existing constructor sets `input_args: Vec::new()`. `run_once` passes `input_args` right before `-i`.
  - ffprobe call: `-show_entries format=duration:stream=codec_type,codec_name,width,height,avg_frame_rate,r_frame_rate,sample_rate:stream_side_data=rotation`.
- `parse_probe` approach: remember the `codec_type` of the stream being read (it precedes `width` etc.); assign size/rates/rotation only for the first video stream and `sample_rate` only for the first audio stream. Apply the display swap after parsing.

- [ ] **Step 1: Write the failing tests**

In `media.rs` tests:
```rust
#[test]
fn probe_reads_video_fields() {
    let text = "codec_name=h264\ncodec_type=video\nwidth=1920\nheight=1080\nr_frame_rate=30/1\navg_frame_rate=30000/1001\n\
                codec_name=aac\ncodec_type=audio\nsample_rate=44100\nr_frame_rate=0/0\navg_frame_rate=0/0\nduration=12.5\n";
    let i = parse_probe(text);
    assert_eq!((i.width, i.height), (Some(1920), Some(1080)));
    assert!((i.fps.unwrap() - 29.97).abs() < 0.01);
    assert_eq!(i.sample_rate, Some(44100));
    assert_eq!(i.rotation, 0);
    assert_eq!(i.duration_secs, Some(12.5));
}

#[test]
fn rotation_swaps_display_size() {
    let text = "codec_name=h264\ncodec_type=video\nwidth=1920\nheight=1080\navg_frame_rate=0/0\nr_frame_rate=25/1\nrotation=-90\nduration=3\n";
    let i = parse_probe(text);
    assert_eq!((i.width, i.height, i.rotation), (Some(1080), Some(1920), 270));
    assert_eq!(i.fps, Some(25.0));
    let upside = parse_probe("codec_name=h264\ncodec_type=video\nwidth=640\nheight=480\nrotation=180\n");
    assert_eq!((upside.width, upside.height, upside.rotation), (Some(640), Some(480), 180));
}
```
Keep the existing `parse_probe` tests unchanged.

In `ffmpeg.rs` tests (fixture: `make(dir, "rot.mp4", &["-f","lavfi","-i","testsrc=duration=1:size=320x240:rate=10","-c:v","libx264","-metadata:s:v:0","rotate=90"])`; if that build ignores `rotate`, write the clip first then `make(dir, "rot.mp4", &["-display_rotation:v:0","90","-i", plain, "-c","copy"])`):
```rust
#[test]
fn probe_reports_display_size_of_rotated_clip() {
    let dir = tempfile::tempdir().unwrap();
    let clip = rotated_clip(dir.path()); // helper above, pub(crate) for later tasks
    let info = probe_out(&clip);
    assert_eq!((info.width, info.height), (Some(240), Some(320)));
    assert_eq!(info.rotation % 180, 90);
    assert_eq!(info.fps, Some(10.0));
}
```

- [ ] **Step 2: Run** `cargo test -p swisswheel-core media` and `cargo test -p swisswheel probe_reports` — Expected: FAIL (missing fields).
- [ ] **Step 3: Implement** the fields, parsing, the ffprobe args and `input_args`. Fix every `MediaInfo { .. }` / `MediaPlan { .. }` literal the compiler reports (`..Default::default()` for test infos).
- [ ] **Step 4: Run** `cargo test --workspace` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(media): probe size, frame rate, sample rate and rotation`

---

### Task 3: Video edit session

**Files:**
- Create: `crates/swisswheel-core/src/video_edit.rs` (add `pub mod video_edit;` to `lib.rs`)

**Interfaces:**
- Consumes: `History`, `frame_geometry`, `EditOp`, `PixelRect`, `Affine` (Task 1); `MediaInfo` (Task 2).
- Produces:
  - `pub enum Speed { Half, Normal, OneHalf, Double }` with `factor(self) -> f64`, `label(self) -> &'static str` (`"0.5×"` …), `slower(self) -> Speed`, `faster(self) -> Speed` (stop at the ends), `cycle(self) -> Speed` (1×→1.5×→2×→0.5×→1×).
  - `pub enum AudioMode { Keep, Mute, Normalize }` with `next(self) -> AudioMode`, `label(self) -> &'static str` (`"소리"`, `"음소거"`, `"크기 맞춤"`).
  - `pub enum VideoOp { Frame(EditOp), Split(f64), Toggle(f64), Speed(Speed), Audio(AudioMode), Reverse(bool) }` (`Frame` holds only `Crop`/`Rotate`/`Flip`).
  - `pub struct Segment { pub start: f64, pub end: f64, pub kept: bool }`.
  - `pub struct VideoInfo { pub width: u32, pub height: u32, pub duration: f64, pub fps: Option<f64>, pub has_audio: bool }` with `pub fn from_media(info: &MediaInfo) -> Option<VideoInfo>` (needs a video stream, width, height and duration > 0).
  - `pub enum Refusal { LastSegment, ReverseTooLong }` with `text(self) -> &'static str`.
  - `pub struct VideoPlan { pub kept: Vec<(f64, f64)>, pub frame: Vec<EditOp>, pub speed: Speed, pub audio: AudioMode, pub reverse: bool }` (`Clone, Debug, PartialEq`).
  - `pub struct VideoSession` with:
    `new(info: VideoInfo) -> Self`, `info(&self) -> &VideoInfo`,
    `push(&mut self, op: VideoOp) -> Result<bool, Refusal>`, `undo`, `redo`, `can_undo`, `can_redo` (as `EditSession`),
    `segments(&self) -> Vec<Segment>`, `segment_at(&self, t: f64) -> Segment`, `is_kept(&self, t: f64) -> bool`,
    `kept_ranges(&self) -> Vec<(f64, f64)>` (adjacent kept pieces merged), `kept_duration(&self) -> f64`, `output_duration(&self) -> f64`,
    `skip_from(&self, t: f64) -> Option<f64>` (t itself if kept, else the next kept piece's start, `None` if none after t),
    `first_kept_start(&self) -> f64`, `last_kept_end(&self) -> f64`,
    `frame_ops(&self) -> Vec<EditOp>`, `size(&self) -> (u32, u32)`, `to_result(&self) -> Affine`,
    `speed(&self) -> Speed`, `audio(&self) -> AudioMode`, `reverse(&self) -> bool`,
    `reverse_fits(&self) -> bool`, `has_edits(&self) -> bool`, `plan(&self) -> Option<VideoPlan>`.
  - `pub fn format_time(t: f64) -> String` (`75.04` → `"1:15.0"`, `3723.4` → `"1:02:03.4"`).
  - `pub mod text` with the Global Constraints copy.
- Rules: segments are derived by replaying `Split`/`Toggle` in order over `[0, duration]`; a piece is `[start, end)` except the last, which includes `duration`. `push` returns `Ok(false)` and records nothing for: a split within 0.1 s of 0, `duration` or a cut; `Speed`/`Audio`/`Reverse` equal to the current value; `Audio` when `!has_audio`; a `Frame(Crop)` equal to the full current size. `Toggle` that would leave no kept piece → `Err(LastSegment)`. `Reverse(true)` when `!reverse_fits()` (evaluated as if on) → `Err(ReverseTooLong)`. `reverse_fits`: `size().0 * size().1 * 1.5 * fps * kept_duration() <= 1_073_741_824.0`, false when `fps` is `None`. `has_edits`: any removed piece, any frame op, speed ≠ 1×, audio ≠ Keep, reverse on.

- [ ] **Step 1: Write the failing tests**

```rust
fn info(duration: f64) -> VideoInfo {
    VideoInfo { width: 1920, height: 1080, duration, fps: Some(30.0), has_audio: true }
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
    assert_eq!(s.segment_at(10.0), Segment { start: 5.0, end: 10.0, kept: false });
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
    assert_eq!(long.push(VideoOp::Reverse(true)), Err(Refusal::ReverseTooLong));
    let mut short = VideoSession::new(info(11.5));
    assert_eq!(short.push(VideoOp::Reverse(true)), Ok(true));
    let mut unknown = VideoSession::new(VideoInfo { fps: None, ..info(1.0) });
    assert_eq!(unknown.push(VideoOp::Reverse(true)), Err(Refusal::ReverseTooLong));
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
    let mut s = VideoSession::new(VideoInfo { has_audio: false, ..info(5.0) });
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
    assert_eq!((plan.speed, plan.audio, plan.reverse), (Speed::Normal, AudioMode::Normalize, false));
}

#[test]
fn video_info_needs_size_and_duration() {
    let full = MediaInfo {
        video: Some("h264".into()), width: Some(640), height: Some(360),
        duration_secs: Some(2.0), ..Default::default()
    };
    assert!(VideoInfo::from_media(&full).is_some());
    assert!(VideoInfo::from_media(&MediaInfo { duration_secs: None, ..full.clone() }).is_none());
    assert!(VideoInfo::from_media(&MediaInfo { width: None, ..full.clone() }).is_none());
    assert!(VideoInfo::from_media(&MediaInfo { video: None, ..full }).is_none());
}

#[test]
fn time_text() {
    assert_eq!(format_time(0.0), "0:00.0");
    assert_eq!(format_time(75.04), "1:15.0");
    assert_eq!(format_time(3723.4), "1:02:03.4");
}
```

- [ ] **Step 2: Run** `cargo test -p swisswheel-core video_edit` — Expected: FAIL (module missing).
- [ ] **Step 3: Implement** `video_edit.rs` per the Interfaces block.
- [ ] **Step 4: Run** `cargo test -p swisswheel-core video_edit` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): video edit session with pieces, speed, sound and reverse`

---

### Task 4: FFmpeg arguments for video edits, frames and scrubbing

**Files:**
- Modify: `crates/swisswheel-core/src/media.rs`
- Modify: `crates/swisswheel-core/src/naming.rs`

**Interfaces:**
- Consumes: `VideoPlan`, `Speed`, `AudioMode` (Task 3); `MediaInfo`, `MediaPlan.input_args` (Task 2).
- Produces:
  - `pub fn video_edit_output(input: Format) -> Format` (WMV → MP4, else the input).
  - `pub fn video_edit_plan(input: Format, info: &MediaInfo, plan: &VideoPlan, quality: &Quality) -> Result<MediaPlan, PlanError>` (`NoVideo` without a video stream).
  - `pub fn video_frame_plan(t: f64, frame: &[EditOp]) -> MediaPlan` (output `Format::Png`).
  - `pub fn scrub_size(display: (u32, u32), longest: u32) -> (u32, u32)` (fit inside `longest`, never upscale, both even and ≥ 2).
  - `pub fn scrub_args(t: f64, size: (u32, u32)) -> (Vec<String>, Vec<String>)` — (before `-i`, after the input path).
  - In `naming.rs`: `pub fn frame_suffix(t: f64) -> String` (`"frame 0m12.40s"`, `"frame 1h02m03.40s"`; round to hundredths first).
- Graph construction (one attempt, always `-filter_complex`):
  - One kept range `(a, b)` that is not the whole clip: `input_args = ["-ss", a, "-t", b - a]` and the graph reads `[0:v:0]` / `[0:a:0]` directly; the whole clip: no `input_args`.
  - Several ranges: `[0:v:0]split=N[v0]…`, `[0:a:0]asplit=N[a0]…` (audio only when kept), per piece `trim=start=a:end=b,setpts=PTS-STARTPTS` / `atrim=start=a:end=b,asetpts=PTS-STARTPTS`, then `concat=n=N:v=1:a=1` (`a=0` without audio).
  - The graph string lists the video part before the audio part.
  - Video chain in order: each frame op → `crop=W:H:X:Y` with `W`,`H` floored to even, `transpose=1` (Cw) / `transpose=2` (Ccw), `hflip`, `vflip`; then `reverse` if on; then `setpts=PTS/{f}` unless 1×; then `scale=trunc(iw/2)*2:trunc(ih/2)*2`; label `[vout]`.
  - Audio chain (when the input has audio and mode ≠ Mute): `areverse` if on; `atempo={f}` unless 1×; `loudnorm=I=-16:TP=-1.5:LRA=11` if Normalize; WebM appends the Opus `aformat`; empty chain uses `anull`; label `[aout]`.
  - Output args: `-filter_complex <graph> -map [vout]` (+ `-map [aout]`, or `-an`), then the target's codec args (no `-vf`/`-af`/`-map` from the conversion path), `-ar <rate>` when normalizing (R5), then `-f <muxer>`. Split `Video::args` so the codec part is shared; conversion output must not change.
  - `duration_secs = Some(kept_duration / speed)`.
  - Frame plan: `input_args = ["-ss", t]`, args `-map 0:v:0 -frames:v 1` + (`-vf <frame chain>` when there are frame ops) + `-c:v png -f image2 -update 1`.
  - Scrub: before `["-ss", t]`, after `["-frames:v","1","-vf","scale=W:H","-f","rawvideo","-pix_fmt","bgra","pipe:1"]`.
  - Seconds are written with `format!("{:.3}", t)`.

- [ ] **Step 1: Write the failing tests** (in `media.rs`; `has(args, &[..])` is the existing contiguous-slice helper)

```rust
fn vinfo(audio: bool) -> MediaInfo {
    MediaInfo {
        video: Some("h264".into()), audio: audio.then(|| "aac".into()),
        width: Some(1920), height: Some(1080), fps: Some(30.0),
        sample_rate: audio.then_some(44100), duration_secs: Some(10.0), ..Default::default()
    }
}
fn vplan(kept: Vec<(f64, f64)>) -> VideoPlan {
    VideoPlan { kept, frame: vec![], speed: Speed::Normal, audio: AudioMode::Keep, reverse: false }
}
fn graph(p: &MediaPlan) -> String {
    let a = &p.attempts[0];
    a[a.iter().position(|s| s == "-filter_complex").unwrap() + 1].clone()
}

#[test]
fn conversion_args_unchanged() {
    // Recorded from main before the refactor: paste the exact vectors that
    // `plan(&Operation::Convert(target), None, Format::Mkv, &info, &Quality::default())`
    // returns for Mp4, Webm and Avi with h264+aac input, and assert equality.
}

#[test]
fn one_range_seeks_on_the_input() {
    let p = video_edit_plan(Format::Mp4, &vinfo(true), &vplan(vec![(2.0, 5.5)]), &Quality::default()).unwrap();
    assert_eq!(p.input_args, ["-ss", "2.000", "-t", "3.500"]);
    assert!(!graph(&p).contains("trim"));
    assert_eq!(p.duration_secs, Some(3.5));
    assert_eq!(p.output, Format::Mp4);
}

#[test]
fn several_ranges_trim_and_concat() {
    let p = video_edit_plan(Format::Mkv, &vinfo(true), &vplan(vec![(0.0, 3.0), (6.0, 10.0)]), &Quality::default()).unwrap();
    let g = graph(&p);
    assert!(g.contains("split=2") && g.contains("asplit=2"));
    assert!(g.contains("trim=start=6.000:end=10.000,setpts=PTS-STARTPTS"));
    assert!(g.contains("concat=n=2:v=1:a=1"));
    assert!(p.input_args.is_empty());
    assert_eq!(p.duration_secs, Some(7.0));
}

#[test]
fn filters_follow_the_edit_order() {
    let mut plan = vplan(vec![(0.0, 10.0)]);
    plan.frame = vec![EditOp::Rotate(Turn::Cw), EditOp::Crop(PixelRect { x: 1, y: 2, w: 100, h: 50 }), EditOp::Flip(Axis::Horizontal)];
    plan.reverse = true;
    plan.speed = Speed::Double;
    let g = graph(&video_edit_plan(Format::Mp4, &vinfo(true), &plan, &Quality::default()).unwrap());
    let at = |s: &str| g.find(s).unwrap_or_else(|| panic!("{s} missing in {g}"));
    assert!(at("transpose=1") < at("crop=100:50:1:2"));
    assert!(at("crop=100:50:1:2") < at("hflip"));
    assert!(at("hflip") < at("reverse"));
    assert!(at("reverse") < at("setpts=PTS/2"));
    assert!(at("areverse") < at("atempo=2"));
}

#[test]
fn odd_crop_is_floored_to_even() {
    let mut plan = vplan(vec![(0.0, 10.0)]);
    plan.frame = vec![EditOp::Crop(PixelRect { x: 3, y: 3, w: 101, h: 57 })];
    let g = graph(&video_edit_plan(Format::Mp4, &vinfo(true), &plan, &Quality::default()).unwrap());
    assert!(g.contains("crop=100:56:3:3"));
}

#[test]
fn plan_without_audio_has_no_audio_graph() {
    let mut plan = vplan(vec![(0.0, 3.0), (6.0, 10.0)]);
    plan.speed = Speed::OneHalf;
    plan.reverse = true;
    let p = video_edit_plan(Format::Mp4, &vinfo(false), &plan, &Quality::default()).unwrap();
    let g = graph(&p);
    assert!(!g.contains("asplit") && !g.contains("atempo") && !g.contains("[aout]"));
    assert!(g.contains("concat=n=2:v=1:a=0"));
    assert!(has(&p.attempts[0], &["-an"]));
}

#[test]
fn mute_and_normalize() {
    let mut plan = vplan(vec![(0.0, 10.0)]);
    plan.audio = AudioMode::Mute;
    let muted = video_edit_plan(Format::Mp4, &vinfo(true), &plan, &Quality::default()).unwrap();
    assert!(has(&muted.attempts[0], &["-an"]) && !graph(&muted).contains("[aout]"));
    plan.audio = AudioMode::Normalize;
    let loud = video_edit_plan(Format::Mov, &vinfo(true), &plan, &Quality::default()).unwrap();
    assert!(graph(&loud).contains("loudnorm=I=-16:TP=-1.5:LRA=11"));
    assert!(has(&loud.attempts[0], &["-ar", "44100"]));
}

#[test]
fn webm_normalize_uses_48k() {
    let mut plan = vplan(vec![(0.0, 10.0)]);
    plan.audio = AudioMode::Normalize;
    let p = video_edit_plan(Format::Webm, &vinfo(true), &plan, &Quality::default()).unwrap();
    assert!(has(&p.attempts[0], &["-ar", "48000"]));
    assert!(graph(&p).contains("aformat=channel_layouts=7.1|5.1|stereo|mono"));
    assert!(has(&p.attempts[0], &["-c:v", "libvpx-vp9"]));
}

#[test]
fn wmv_saves_as_mp4() {
    assert_eq!(video_edit_output(Format::Wmv), Format::Mp4);
    assert_eq!(video_edit_output(Format::Avi), Format::Avi);
    let p = video_edit_plan(Format::Wmv, &vinfo(true), &vplan(vec![(0.0, 10.0)]), &Quality::default()).unwrap();
    assert_eq!(p.output, Format::Mp4);
    assert!(has(&p.attempts[0], &["-c:v", "libx264"]));
}

#[test]
fn frame_and_scrub_args() {
    let f = video_frame_plan(12.4, &[EditOp::Rotate(Turn::Ccw)]);
    assert_eq!(f.output, Format::Png);
    assert_eq!(f.input_args, ["-ss", "12.400"]);
    assert!(has(&f.attempts[0], &["-frames:v", "1"]) && has(&f.attempts[0], &["-vf", "transpose=2"]));
    assert!(has(&f.attempts[0], &["-c:v", "png", "-f", "image2", "-update", "1"]));
    assert_eq!(scrub_size((1920, 1080), 960), (960, 540));
    assert_eq!(scrub_size((101, 51), 960), (100, 50));
    let (before, after) = scrub_args(1.5, (960, 540));
    assert_eq!(before, ["-ss", "1.500"]);
    assert!(has(&after, &["-vf", "scale=960:540", "-f", "rawvideo", "-pix_fmt", "bgra", "pipe:1"]));
}
```
In `naming.rs`:
```rust
#[test]
fn frame_suffixes() {
    assert_eq!(frame_suffix(12.4), "frame 0m12.40s");
    assert_eq!(frame_suffix(59.999), "frame 1m00.00s");
    assert_eq!(frame_suffix(3723.4), "frame 1h02m03.40s");
}
```

- [ ] **Step 2: Fill `conversion_args_unchanged`** by running the current `plan` on `main`'s code (before touching `Video::args`) and pasting the three vectors; run it — Expected: PASS.
- [ ] **Step 3: Run** `cargo test -p swisswheel-core media naming` — Expected: the new tests FAIL (functions missing).
- [ ] **Step 4: Implement** the functions and the `Video::args` split.
- [ ] **Step 5: Run** `cargo test -p swisswheel-core` — Expected: PASS (including `conversion_args_unchanged`).
- [ ] **Step 6: Commit** `feat(media): ffmpeg arguments for video edits, frames and scrubbing`

---

### Task 5: Jobs run video edits and frame saves

**Files:**
- Modify: `crates/swisswheel/src/jobs.rs`
- Modify: `crates/swisswheel/src/engines/ffmpeg.rs` (tests, fixture helpers)
- Modify: `crates/swisswheel/src/app.rs` (the one `edits: Some(plan)` site → `Edits::Image(plan)`)

**Interfaces:**
- Consumes: Tasks 2–4.
- Produces:
  - `pub enum Edits { Image(EditPlan), Video(VideoPlan), Frame { t: f64, ops: Vec<EditOp> } }` in `jobs.rs`; `Job.edits: Option<Edits>`.
  - `process`: `Edits::Image` → today's path. `Edits::Video` → `ffmpeg::probe` → `media::video_edit_plan(format, &info, plan, &quality)` → `Engine::Media`, extension from `plan.output`, suffix `"edited"`. `Edits::Frame` → `media::video_frame_plan(t, ops)` → `Engine::Media`, extension `png`, suffix `frame_suffix(t)`. Missing FFmpeg → `EngineError::MissingFfmpeg` as for other media.
  - `pub(crate) fn tone_clip(dir, name, secs) -> PathBuf` in `ffmpeg.rs` tests: `testsrc=duration={secs}:size=320x240:rate=10` + `sine=frequency=440:sample_rate=44100:duration={secs}`, `libx264` + `aac`, `.mp4`; and `pub(crate) fn color_steps(dir, name) -> PathBuf`: 2 s at 10 fps whose first frame is red and last frame blue (e.g. `color=red:d=1` and `color=blue:d=1` concatenated, 64x64).
  - `pub(crate) fn run_video_edit(input: &Path, plan: &VideoPlan, out: &Path)` test helper calling `prepare`-like code: probe → `video_edit_plan` → `run`.

- [ ] **Step 1: Write the failing tests** (engine tests in `ffmpeg.rs`; durations read with `probe_out(..).duration_secs`, tolerance ±0.15 s)

```rust
#[test]
fn removed_pieces_shorten_the_clip()        // tone_clip 6 s, kept [(0,2),(4,6)] → ~4 s, audio stream present
#[test]
fn crop_then_rotate_sizes()                 // frame [Crop{0,0,200,100}, Rotate(Cw)] on 320x240 → dims (100, 200)
#[test]
fn odd_crop_encodes()                       // Crop{1,1,101,57} → dims (100, 56)
#[test]
fn double_speed_halves_duration()           // 4 s at 2× → ~2 s; 0.5× on 2 s → ~4 s
#[test]
fn mute_drops_audio_and_normalize_keeps_rate() // Mute → info.audio None; Normalize → sample_rate Some(44100)
#[test]
fn webm_normalize_saves()                   // tone_clip converted to .webm input first; Normalize → succeeds, sample_rate Some(48000)
#[test]
fn reverse_starts_with_the_last_frame()     // color_steps → first frame of output is blue (decode with `ffmpeg -frames:v 1 -f rawvideo -pix_fmt rgb24 pipe:1`, check b > r)
#[test]
fn rotated_input_crops_in_display_space()   // rotated_clip (display 240x320) with Crop{0,0,240,100} → dims (240, 100)
#[test]
fn frame_png_has_the_edited_size()          // video_frame_plan(0.5, [Rotate(Cw)]) on 320x240 → PNG 240x320 (read with `image::image_dimensions`)
#[test]
fn every_container_saves()                  // make .webm, .avi, .wmv from tone_clip (`-c:v wmv2 -c:a wmav2` for wmv); one 1.5× edit each succeeds; wmv output Format::Mp4
```
In `jobs.rs` tests:
```rust
#[test]
fn video_edit_and_frame_jobs_name_their_outputs() {
    let dir = tempfile::tempdir().unwrap();
    let clip = pattern(dir.path(), "클립.mp4", "320x240", 2);
    let plan = VideoPlan { kept: vec![(0.0, 1.0)], frame: vec![], speed: Speed::Normal, audio: AudioMode::Keep, reverse: false };
    let reports = run_jobs(vec![
        Job { op: Operation::Tool(Tool::Edit), preset: None, inputs: vec![clip.clone()], edits: Some(Edits::Video(plan)) },
        Job { op: Operation::Tool(Tool::Edit), preset: None, inputs: vec![clip.clone()], edits: Some(Edits::Frame { t: 1.0, ops: vec![] }) },
    ]);
    let mut outs: Vec<_> = reports.iter().flat_map(|r| r.outputs.clone()).collect();
    outs.sort();
    assert_eq!(outs, vec![dir.path().join("클립 (edited).mp4"), dir.path().join("클립 (frame 0m01.00s).png")]);
    assert!(clip.exists());
}
```

- [ ] **Step 2: Run** `cargo test -p swisswheel ffmpeg jobs` — Expected: FAIL (no `Edits`).
- [ ] **Step 3: Implement** `Edits` and the `process` branches; update the existing image edit tests and `app.rs` to `Edits::Image`.
- [ ] **Step 4: Run** `cargo test --workspace` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(jobs): run video edits and frame saves`

---

### Task 6: Video panel layout and timeline geometry

**Files:**
- Modify: `crates/swisswheel-core/src/editor_geometry.rs`

**Interfaces:**
- Consumes: `panel_layout` constants (`TOOLBAR`, `CHIP_ROW`, `FOOTER`, `PAD`, `TOOL_BUTTON`, `CHIP`, `FOOTER_BUTTON`, `MIN_W`, `MIN_H`, `MAX_FRACTION`, `SHADOW_MARGIN`).
- Produces:
  - `pub const TIMELINE_ROW: f32 = 44.0; pub const PLAY_BUTTON: f32 = 44.0; pub const TIME_TEXT: f32 = 140.0; pub const PIECE_BUTTON: f32 = 96.0;`
  - `pub enum VideoTool { Crop, Rotate, FlipH, FlipV, Speed, Audio, Reverse, Frame }`, `pub const VIDEO_TOOLS: [VideoTool; 8]` in that order.
  - `pub enum VideoHit { Tool(VideoTool), Chip(usize), Play, Bar, Split, Toggle, Undo, Redo, Cancel, Save, Preview, Nothing }`.
  - `pub struct VideoLayout { window, panel, tools: [Box2; 8], name, chips: [Box2; 7], preview, play, time, bar, split, toggle, undo, redo, label, cancel, save }`.
  - `pub fn video_layout(work_area: Rect, scale: f32, video: (u32, u32)) -> VideoLayout` — as `panel_layout`, with the timeline row (`TIMELINE_ROW`) between the preview and the footer: play button at the left, then the time text, then the bar filling the rest up to the split and toggle buttons at the right. Shares a private helper with `panel_layout` for window placement, toolbar, chips and footer (no change to `panel_layout`'s output).
  - `pub fn video_hit_test(layout: &VideoLayout, cropping: bool, x: f32, y: f32) -> VideoHit` (chips only while cropping; the bar counts with `HANDLE_REACH` vertical slack).
  - `pub struct Timeline { pub left: f32, pub right: f32, pub duration: f64 }` with `from_bar(bar: Box2, duration: f64)`, `time_at(&self, x: f32) -> f64` (clamped to `[0, duration]`), `x_of(&self, t: f64) -> f32`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn video_layout_rows() {
    let work = Rect { left: 0, top: 0, right: 1920, bottom: 1040 };
    let l = video_layout(work, 1.0, (1920, 1080));
    assert!(l.preview.bottom <= l.bar.top);
    assert!(l.bar.bottom <= l.undo.top);
    assert!(l.play.right <= l.time.left && l.time.right <= l.bar.left);
    assert!(l.bar.right <= l.split.left && l.split.right <= l.toggle.left);
    assert!(l.tools[7].right <= l.panel.right);
    // Same window as the image panel would get plus one timeline row at most.
    let img = panel_layout(work, 1.0, (1920, 1080));
    assert!((l.window.right - l.window.left) <= (img.window.right - img.window.left));
}

#[test]
fn video_layout_portrait_and_scale() {
    let work = Rect { left: 0, top: 0, right: 2560, bottom: 1400 };
    let l = video_layout(work, 2.0, (1080, 1920));
    assert!(l.preview.right - l.preview.left > 0.0 && l.preview.bottom - l.preview.top > 0.0);
    assert!(l.tools[7].right <= l.panel.right);
}

#[test]
fn video_hits() {
    let l = video_layout(Rect { left: 0, top: 0, right: 1920, bottom: 1040 }, 1.0, (1280, 720));
    let c = |b: Box2| ((b.left + b.right) / 2.0, (b.top + b.bottom) / 2.0);
    let (x, y) = c(l.tools[4]);
    assert_eq!(video_hit_test(&l, false, x, y), VideoHit::Tool(VideoTool::Speed));
    let (x, y) = c(l.chips[2]);
    assert_eq!(video_hit_test(&l, false, x, y), VideoHit::Nothing);
    assert_eq!(video_hit_test(&l, true, x, y), VideoHit::Chip(2));
    let (x, y) = c(l.bar);
    assert_eq!(video_hit_test(&l, false, x, y), VideoHit::Bar);
    let (x, y) = c(l.toggle);
    assert_eq!(video_hit_test(&l, false, x, y), VideoHit::Toggle);
}

#[test]
fn timeline_round_trip_and_clamp() {
    let t = Timeline { left: 100.0, right: 500.0, duration: 20.0 };
    assert_eq!(t.time_at(300.0), 10.0);
    assert_eq!(t.x_of(5.0), 200.0);
    assert_eq!(t.time_at(0.0), 0.0);
    assert_eq!(t.time_at(900.0), 20.0);
}
```

- [ ] **Step 2: Run** `cargo test -p swisswheel-core editor_geometry` — Expected: FAIL.
- [ ] **Step 3: Implement**.
- [ ] **Step 4: Run** `cargo test -p swisswheel-core` — Expected: PASS (existing layout tests unchanged).
- [ ] **Step 5: Commit** `feat(core): video panel layout and timeline geometry`

---

### Task 7: Video panel controller

**Files:**
- Create: `crates/swisswheel-core/src/video_control.rs` (add to `lib.rs`)

**Interfaces:**
- Consumes: `VideoSession`, `VideoOp`, `Refusal`, `text` (Task 3); `VideoHit`, `VideoTool` (Task 6); `CropMode`, `DiscardPrompt`, `NOTICE_WINDOW` (Task 1, `CropMode` fields made `pub(crate)` where needed); `crop::{CHIPS, drag, fit_ratio}`.
- Produces:
  - `pub enum VideoCommand { PlayPause, StepBack, StepForward, BackSecond, ForwardSecond, Start, End, Split, TogglePiece, Crop, RotateCw, RotateCcw, FlipH, FlipV, Slower, Faster, CycleSpeed, CycleAudio, Reverse, SaveFrame, Undo, Redo, Enter, Escape }`.
  - `pub fn video_key(vk: u16, shift: bool, ctrl: bool) -> Option<VideoCommand>` (Global Constraints table; Ctrl only for Z/Y).
  - `pub fn repeats(vk: u16) -> bool` — true only for ← and →.
  - `pub enum PlayerAction { Play, Pause, Seek(f64), StepFrame(bool), Rate(f64), Mute(bool) }`.
  - `pub enum VideoOutcome { Nothing, Redraw, Save(VideoPlan), SaveFrame { t: f64, ops: Vec<EditOp> }, Close }`.
  - `pub struct VideoState` with: `loading() -> Self`, `loaded(&mut self, info: VideoInfo)`, `session(&self) -> Option<&VideoSession>`, `crop(&self) -> Option<&CropMode>`, `position(&self) -> f64`, `playing(&self) -> bool`, `fallback(&self) -> bool`, `set_fallback(&mut self, now: Instant)`, `take_actions(&mut self) -> Vec<PlayerAction>`, `command(&mut self, cmd: VideoCommand, now: Instant) -> VideoOutcome`, `click(&mut self, hit: VideoHit, now: Instant) -> VideoOutcome`, `key_other(&mut self, now: Instant) -> VideoOutcome`, `timeline_press(&mut self, t: f64, now: Instant) -> VideoOutcome`, `timeline_move(&mut self, t: f64) -> VideoOutcome`, `timeline_release(&mut self) -> VideoOutcome`, `scrubbing(&self) -> bool`, `tick(&mut self, t: f64, ended: bool) -> VideoOutcome`, `crop_press/crop_move/crop_release` (as `EditorState`), `footer_text(&self, hover: VideoHit, now: Instant) -> Option<&'static str>`, `notice_deadline(&self) -> Option<Instant>`.
- Rules:
  - Actions queue: every seek sets `position` and queues `Seek(t)`. In fallback mode `StepBack`/`StepForward` seek by `1/fps` (fps unknown → 1/30) and `PlayPause` does nothing; otherwise they queue `StepFrame(false/true)` and pause first if playing.
  - `PlayPause` while playing → `Pause`. While paused: if `position >= last_kept_end() - 0.05` or the position's piece is removed with nothing after it, seek to `first_kept_start()` first; otherwise seek to `skip_from(position)` if it differs; then `Play`.
  - After any op that changes `speed()` or `audio()` (including undo/redo), queue `Rate(factor)` / `Mute(audio == Mute)`.
  - `tick(t, ended)` while playing: updates `position`; `skip_from(t)` → `Some(x)` with `x > t + 0.001` queues `Seek(x)`; `None` or `ended` → `Pause`, position = `last_kept_end()`. Returns `Redraw` when the position moved by ≥ 1/100 s or state changed.
  - Timeline press pauses (remembering it was playing) and seeks; move seeks; release resumes play if it was playing.
  - `Crop` enters crop mode (pausing playback); crop mode blocks everything except the crop commands, timeline, play/pause and stepping (spec 2.5).
  - Refusals and the normalize change set a 3 s notice (`text::LAST_PIECE`, `text::REVERSE_TOO_LONG`, `text::NORMALIZE_ON_SAVE`); `set_fallback` sets `text::FALLBACK` once. `footer_text` priority: discard prompt, notice, hover on Audio while Normalize → `NORMALIZE_ON_SAVE`, hover on Play in fallback → `FALLBACK`.
  - `Enter`: no edits → `Nothing`; reverse on and `!reverse_fits()` → notice `REVERSE_TOO_LONG`, `Redraw`; else `Save(plan)`. `Escape`: discard prompt with `has_edits()` (as `EditorState`).
  - `SaveFrame`: pause if playing, return `SaveFrame { t: position, ops: session.frame_ops() }` (allowed with no edits).

- [ ] **Step 1: Write the failing tests**

```rust
fn ready(duration: f64) -> VideoState {
    let mut s = VideoState::loading();
    s.loaded(VideoInfo { width: 1280, height: 720, duration, fps: Some(25.0), has_audio: true });
    s
}

#[test]
fn keys() {
    assert_eq!(video_key(0x20, false, false), Some(VideoCommand::PlayPause));
    assert_eq!(video_key(0x25, true, false), Some(VideoCommand::BackSecond));
    assert_eq!(video_key(0x27, false, false), Some(VideoCommand::StepForward));
    assert_eq!(video_key(0xDD, false, false), Some(VideoCommand::Faster));
    assert_eq!(video_key(0x44, false, false), Some(VideoCommand::TogglePiece));
    assert_eq!(video_key(0x5A, true, true), Some(VideoCommand::Redo));
    assert_eq!(video_key(0x53, false, true), None);
    assert!(repeats(0x25) && !repeats(0x20));
}

#[test]
fn play_skips_removed_pieces_and_stops_at_the_end() {
    let now = Instant::now();
    let mut s = ready(10.0);
    s.timeline_press(3.0, now); s.timeline_release();
    s.command(VideoCommand::Split, now);
    s.timeline_press(6.0, now); s.timeline_release();
    s.command(VideoCommand::Split, now);
    s.timeline_press(4.0, now); s.timeline_release();
    s.command(VideoCommand::TogglePiece, now); // remove [3, 6)
    s.take_actions();
    s.command(VideoCommand::PlayPause, now);
    assert_eq!(s.take_actions(), vec![PlayerAction::Seek(6.0), PlayerAction::Play]);
    s.tick(9.99, true);
    assert_eq!(s.take_actions(), vec![PlayerAction::Pause]);
    s.command(VideoCommand::PlayPause, now); // at the end: from the first kept start
    assert_eq!(s.take_actions(), vec![PlayerAction::Seek(0.0), PlayerAction::Play]);
    s.tick(3.2, false);
    assert_eq!(s.take_actions(), vec![PlayerAction::Seek(6.0)]);
}

#[test]
fn speed_and_mute_reach_the_player_and_undo() {
    let now = Instant::now();
    let mut s = ready(10.0);
    s.command(VideoCommand::Faster, now);
    s.command(VideoCommand::CycleAudio, now);
    assert_eq!(s.take_actions(), vec![PlayerAction::Rate(1.5), PlayerAction::Mute(true)]);
    s.command(VideoCommand::Undo, now);
    assert_eq!(s.take_actions(), vec![PlayerAction::Mute(false)]);
}

#[test]
fn refusals_show_a_notice() {
    let now = Instant::now();
    let mut s = ready(10.0);
    assert_eq!(s.command(VideoCommand::TogglePiece, now), VideoOutcome::Redraw);
    assert_eq!(s.footer_text(VideoHit::Nothing, now), Some(text::LAST_PIECE));
    let mut long = ready(600.0);
    long.command(VideoCommand::Reverse, now);
    assert_eq!(long.footer_text(VideoHit::Nothing, now), Some(text::REVERSE_TOO_LONG));
    assert_eq!(long.footer_text(VideoHit::Nothing, now + NOTICE_WINDOW), None);
}

#[test]
fn crop_mode_pauses_and_blocks_tools() {
    let now = Instant::now();
    let mut s = ready(10.0);
    s.command(VideoCommand::PlayPause, now);
    s.take_actions();
    s.command(VideoCommand::Crop, now);
    assert_eq!(s.take_actions(), vec![PlayerAction::Pause]);
    assert_eq!(s.command(VideoCommand::RotateCw, now), VideoOutcome::Nothing);
    assert_eq!(s.command(VideoCommand::Split, now), VideoOutcome::Nothing);
    s.command(VideoCommand::Escape, now);
    assert!(s.crop().is_none());
}

#[test]
fn save_discard_and_frame() {
    let now = Instant::now();
    let mut s = ready(10.0);
    assert_eq!(s.command(VideoCommand::Enter, now), VideoOutcome::Nothing);
    assert!(matches!(s.command(VideoCommand::SaveFrame, now), VideoOutcome::SaveFrame { t, .. } if t == 0.0));
    s.command(VideoCommand::RotateCw, now);
    assert!(matches!(s.command(VideoCommand::Enter, now), VideoOutcome::Save(_)));
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
    assert_eq!(s.take_actions(), vec![PlayerAction::Pause, PlayerAction::Seek(2.0), PlayerAction::Seek(2.5), PlayerAction::Play]);
}
```

- [ ] **Step 2: Run** `cargo test -p swisswheel-core video_control` — Expected: FAIL.
- [ ] **Step 3: Implement** `video_control.rs`.
- [ ] **Step 4: Run** `cargo test -p swisswheel-core` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): video panel controller`

---

### Task 8: Media Foundation player

**Files:**
- Create: `crates/swisswheel/src/editor/player.rs`
- Modify: `crates/swisswheel/Cargo.toml` (`Win32_Media_MediaFoundation`, plus only the sub-features the compiler asks for)
- Modify: `crates/swisswheel/src/wheel/render.rs` (`create_d3d_device_with(flags: D3D11_CREATE_DEVICE_FLAG)`; `create_d3d_device()` calls it with `BGRA_SUPPORT`)
- Modify: `crates/swisswheel/src/surface.rs` (`Surface::with_device(hwnd, device: ID3D11Device, width, height, glass_regions)`; `new` calls it)

**Interfaces:**
- Produces (all `pub(super)` in `editor::player`):
  - `pub fn startup() -> Result<()>` — `MFStartup(MF_VERSION, MFSTARTUP_FULL)` once per process (a `OnceLock<bool>`); later calls return the first result.
  - `pub fn video_device() -> Result<ID3D11Device>` — `BGRA_SUPPORT | VIDEO_SUPPORT`, hardware then WARP, `ID3D10Multithread::SetMultithreadProtected(true)`.
  - `pub enum PlayerEvent { Ready, Ended, Seeked, Error }` with `from_wparam(w: usize) -> Option<PlayerEvent>` mapping `MF_MEDIA_ENGINE_EVENT_CANPLAY`, `_ENDED`, `_SEEKED`, `_ERROR`.
  - `pub struct Player` with `open(panel: HWND, device: &ID3D11Device, path: &Path) -> Result<Player>` (creates the DXGI manager, `IMFMediaEngine` with `MF_MEDIA_ENGINE_DXGI_MANAGER`, `MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT = DXGI_FORMAT_B8G8R8A8_UNORM`, `MF_MEDIA_ENGINE_CALLBACK` = an `#[implement(IMFMediaEngineNotify)]` object that posts `WM_APP_PLAYER` with the event code to `panel`; sets the source with `SetSource` on a `BSTR` of the path; does not autoplay), `play`, `pause`, `seek(t: f64)`, `set_rate(f: f64)` (`SetPlaybackRate`), `set_muted(bool)`, `current_time(&self) -> f64`, `step_frame(&self, forward: bool)` (`IMFMediaEngineEx::FrameStep`), `native_size(&self) -> Option<(u32, u32)>`, `frame(&mut self, ctx: &ID2D1DeviceContext) -> Result<Option<ID2D1Bitmap1>>` (on `OnVideoStreamTick` success, `TransferVideoFrame` into a BGRA texture of `native_size` created once with `D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE`, wrapped once by `CreateBitmapFromDxgiSurface`; returns the bitmap whenever a new frame was transferred), `shutdown(&self)` (`IMFMediaEngine::Shutdown`, also on `Drop`).
- The notify object only holds the panel `HWND` as `isize` and calls `PostMessageW`; it must not touch UI state.

- [ ] **Step 1: Write the failing test** (in `player.rs`; this needs a GPU or WARP and runs a message-less wait)

```rust
#[test]
fn opens_a_clip_and_reports_ready() {
    let dir = tempfile::tempdir().unwrap();
    let clip = crate::engines::ffmpeg::tests::tone_clip(dir.path(), "a.mp4", 2);
    startup().unwrap();
    let device = video_device().unwrap();
    // A message-only window receives WM_APP_PLAYER; pump until Ready or 5 s.
    let (hwnd, events) = test_window();
    let player = Player::open(hwnd, &device, &clip).unwrap();
    assert_eq!(pump_until(&events, PlayerEvent::Ready, Duration::from_secs(5)), true);
    assert_eq!(player.native_size(), Some((320, 240)));
    player.seek(1.0);
    assert!(pump_until(&events, PlayerEvent::Seeked, Duration::from_secs(5)));
    assert!((player.current_time() - 1.0).abs() < 0.2);
}
```
`test_window`/`pump_until` are test-only helpers in the same file (`HWND_MESSAGE` window whose proc pushes `PlayerEvent::from_wparam` into a thread-local `Vec`; `PeekMessageW` loop with 10 ms sleeps).

- [ ] **Step 2: Run** `cargo test -p swisswheel player` — Expected: FAIL (module missing).
- [ ] **Step 3: Implement** the device helpers, `Surface::with_device`, and `player.rs`. Register `mod player;` in `editor/mod.rs`.
- [ ] **Step 4: Run** `cargo test -p swisswheel player` — Expected: PASS. If the machine has no working media pipeline the test fails at `Player::open`; report that rather than ignoring the test.
- [ ] **Step 5: Run** `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` — Expected: PASS / clean.
- [ ] **Step 6: Commit** `feat(editor): Media Foundation player`

---

### Task 9: FFmpeg frame scrubber

**Files:**
- Create: `crates/swisswheel/src/editor/scrub.rs`

**Interfaces:**
- Consumes: `media::{scrub_args, scrub_size}` (Task 4); `engines::ffmpeg::Tools`.
- Produces:
  - `pub(super) struct ScrubFrame { pub t: f64, pub size: (u32, u32), pub bgra: Vec<u8> }`.
  - `pub(super) struct Scrubber` with `new(tools: Tools, input: PathBuf, size: (u32, u32), panel: HWND) -> Scrubber` (spawns one worker thread), `request(&self, t: f64)` (replaces any pending request; the worker always takes the newest), `take(&self) -> Option<Result<ScrubFrame, String>>` (the latest finished result). The worker runs `ffmpeg -hide_banner -nostdin -loglevel error <before> -i <input> <after>` with `CREATE_NO_WINDOW`, checks `bgra.len() == w*h*4`, stores the result and posts `WM_APP_SCRUB` to the panel. `Drop` stops the worker and kills a running FFmpeg (spawn it inside a `RunJob`-style job object; reuse the one in `ffmpeg.rs` by making it `pub(crate)`).
  - `pub(super) fn grab(tools: &Tools, input: &Path, t: f64, size: (u32, u32)) -> Result<Vec<u8>, String>` — the single synchronous grab the worker calls.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn grabs_a_bgra_frame_of_the_asked_size() {
    let dir = tempfile::tempdir().unwrap();
    let clip = crate::engines::ffmpeg::tests::color_steps(dir.path(), "c.mp4");
    let tools = crate::engines::ffmpeg::tests::tools();
    let bgra = grab(&tools, &clip, 1.5, (32, 32)).unwrap();
    assert_eq!(bgra.len(), 32 * 32 * 4);
    let (b, r) = (bgra[0], bgra[2]);
    assert!(b > 200 && r < 60, "expected blue at 1.5 s, got b={b} r={r}");
    assert!(grab(&tools, &dir.path().join("missing.mp4"), 0.0, (32, 32)).is_err());
}
```

- [ ] **Step 2: Run** `cargo test -p swisswheel scrub` — Expected: FAIL.
- [ ] **Step 3: Implement** `scrub.rs` (register `mod scrub;`).
- [ ] **Step 4: Run** `cargo test -p swisswheel scrub` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(editor): ffmpeg frame scrubber for unplayable files`

---

### Task 10: Video panel window and drawing

**Files:**
- Create: `crates/swisswheel/src/editor/video.rs`
- Create: `crates/swisswheel/src/editor/video_render.rs`
- Modify: `crates/swisswheel/src/editor/window.rs` (`Windows::create` takes the panel class name and window procedure; `dim_proc` forwards a click to whichever slot is open; export `client_point`, `KEY_REPEAT`, `TRACKING` handling as `pub(super)` helpers so `video.rs` reuses them)
- Modify: `crates/swisswheel/src/editor/render.rs` (make `crop_overlay`, `backdrop`, footer-button and toolbar-button drawing reusable: `pub(super)` free functions taking `ctx`, theme, scale)
- Modify: `crates/swisswheel/src/editor/mod.rs` (`open` stays the image entry; new `pub fn open_video(path, owner, notify, accent, tools: Tools)` calls `video::open`; `is_open`/`focus`/`close` cover both slots; `EditorEvent` gains `SavedVideo(PathBuf, VideoPlan)` and `SaveFrame(PathBuf, f64, Vec<EditOp>)`)

**Interfaces:**
- Consumes: Tasks 1–9.
- Produces:
  - `editor::video::open(path, owner, notify, accent, tools: Tools) -> windows::core::Result<()>`: probe (R2) → `VideoInfo::from_media` (none → queue `Failed(format!("{name}: {}", video_edit::text::READ_FAILED))`) → `video_layout` → `Windows::create(.., w!("SwissWheelVideoEditor"), video_proc)` → `video_device()` → `Surface::with_device` → draw "불러오는 중" → `player::startup()` then `Player::open`; any player error → fallback (`set_fallback`, `Scrubber::new`, `request(0.0)`).
  - On `PlayerEvent::Ready`: `state.loaded(info)` (if not already), apply queued `Rate`/`Mute`, draw the first frame. `Error` → fallback (keep the session). `Seeked`/`Ended` → `state.tick(current_time, ended)`.
  - Playback timer `2` at 16 ms runs while `state.playing()` or a seek is pending: pull `player.frame(ctx)`, `state.tick`, redraw. Timer `1` follows `state.notice_deadline()`.
  - After every handler, drain `state.take_actions()` into the player (or, in fallback, `Seek(t)` → `scrubber.request(t)`; others ignored).
  - `WM_KEYDOWN` auto-repeat passes only when `video_control::repeats(vk)`.
  - Drawing (`video_render::draw`): toolbar of 8 tools (speed shows `Speed::label`, audio shows the `AudioMode` glyph and label, Reverse/Speed/Audio filled with the accent when not default, Audio dimmed without audio); chips while cropping; the frame drawn like the image preview with `to_result()` and, when `native_size` is the display size transposed, an extra rotation by `MediaInfo.rotation` before it; "빠진 구간" over a dimmed frame when paused on a removed piece; "역재생" badge at the preview's top-left while reverse is on; timeline: kept pieces filled, removed pieces hatched (diagonal lines clipped to the piece) at 40% opacity, cut ticks, accent playhead, play/pause glyph, `format_time(position) / format_time(duration)`; split button `"나누기"`, toggle button `"빼기"` or `"되살리기"` by the piece under the playhead; footer like the image panel with `원본 W×H · 길이 → 결과 W×H · 길이 · 배속` (only the original part when there are no edits) or `footer_text`; "장면을 불러오지 못했습니다" in the preview after a scrub error; "불러오는 중" before the first frame.
  - Device loss: rebuild device, surface, player (reopen at the current position, paused) like `Editor::redraw`; a second failure closes with `Failed(OPEN_FAILED)`.
  - `VideoOutcome::Save(plan)` closes with `SavedVideo`; `SaveFrame` queues the event without closing and posts `notify`; `Close` closes with `Closed`.

- [ ] **Step 1: Write the failing test** (in `video.rs`; pure helper)

```rust
#[test]
fn engine_frame_needs_rotation_only_when_transposed() {
    // display 1080×1920 (rotation 90): an engine frame of 1920×1080 needs turning, 1080×1920 does not.
    assert_eq!(extra_rotation((1080, 1920), 90, (1920, 1080)), 90);
    assert_eq!(extra_rotation((1080, 1920), 90, (1080, 1920)), 0);
    assert_eq!(extra_rotation((1280, 720), 0, (1280, 720)), 0);
    assert_eq!(extra_rotation((1280, 720), 180, (1280, 720)), 0);
}
```
(`fn extra_rotation(display: (u32, u32), rotation: u32, frame: (u32, u32)) -> u32` — `rotation` when `rotation % 180 == 90` and `frame == (display.1, display.0)`, else 0. A 180° clip is trusted to the engine.)

- [ ] **Step 2: Run** `cargo test -p swisswheel extra_rotation` — Expected: FAIL.
- [ ] **Step 3: Implement** `video.rs`, `video_render.rs` and the `window.rs`/`render.rs`/`mod.rs` changes. The image panel's behavior must not change.
- [ ] **Step 4: Run** `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check` — Expected: PASS / clean.
- [ ] **Step 5: Commit** `feat(editor): video panel with playback, timeline and tools`

---

### Task 11: Wire the video editor into the app

**Files:**
- Modify: `crates/swisswheel-core/src/catalog.rs`
- Modify: `crates/swisswheel/src/app.rs`

**Interfaces:**
- Consumes: Task 10's `EditorEvent` variants and `editor::open`.
- Produces:
  - `items_for(WheelKind::Tools, inputs)` appends `Edit` for exactly one `InputItem::File { format: Some(f), animated_gif: false }` with `f.kind()` `Image` or `Video`.
  - `open_editor`: picks by `Format::from_path(..).kind()`. For a video, `engines::ffmpeg::locate()`; `None` → warning card `EngineError::MissingFfmpeg.user_message(name)` (R1) and nothing opens; else `editor::open_video(path, hwnd, WM_APP_EDITOR, accent, tools)`.
  - `on_editor_events`: `SavedVideo(path, plan)` → `Job { op: Tool(Edit), preset: None, inputs: [path], edits: Some(Edits::Video(plan)) }`; `SaveFrame(path, t, ops)` → `Edits::Frame { t, ops }` job, panel stays open; both bump `active_jobs` like the image save.
  - Log lines: `"작업 시작: 영상 편집 · {path}"`, `"작업 시작: 장면 저장 · {path}"`.

- [ ] **Step 1: Write the failing test** (in `catalog.rs`, extend the existing edit test)

```rust
#[test]
fn edit_for_one_video_too() {
    use Format::*;
    let tools = |inputs: &[InputItem]| items_for(WheelKind::Tools, inputs);
    for f in [Mp4, Mkv, Webm, Mov, Avi, Wmv] {
        assert_eq!(tools(&[file(f)]).last(), Some(&Operation::Tool(Tool::Edit)), "{f:?}");
    }
    assert!(!tools(&[file(Mp4), file(Mov)]).contains(&Operation::Tool(Tool::Edit)));
    assert!(!tools(&[file(Mp3)]).contains(&Operation::Tool(Tool::Edit)));
    assert!(!tools(&[file(Mp4), file(Png)]).contains(&Operation::Tool(Tool::Edit)));
}
```
Remove the old assertion that MP4 has no `Edit`.

- [ ] **Step 2: Run** `cargo test -p swisswheel-core catalog` — Expected: FAIL.
- [ ] **Step 3: Implement** the catalog rule and the app wiring.
- [ ] **Step 4: Run** `cargo test --workspace`, clippy, fmt — Expected: PASS / clean.
- [ ] **Step 5: Screen check** (skip if a full-screen game owns the foreground; record the Run value first even though `SWISSWHEEL_PREVIEW` should leave it alone): drop a generated `tone_clip` and a portrait `rotated_clip` on ✎편집; check playback with sound, Space, ←/→, timeline drag, S/D, speed, mute, crop, save, frame save. Report what was and wasn't checked.
- [ ] **Step 6: Commit** `feat(app): open videos in the editor and run their saves`

---

### Task 12: Docs

**Files:**
- Modify: `README.md`
- Modify: `docs/manual-test.md`

- [ ] **Step 1:** README — under the image editor section add "Video editor": opening (one video, FFmpeg needed), playback, pieces (S/D), tools and keys from Global Constraints, outputs (`(edited)`, WMV→MP4, `(frame …).png`), reverse limit, fallback mode.
- [ ] **Step 2:** `docs/manual-test.md` — a "영상 편집기" section numbered from **M123**, one row per spec 5.4 item, in the file's existing table format.
- [ ] **Step 3:** Commit `docs: video editor`
