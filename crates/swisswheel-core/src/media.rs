//! FFmpeg work, decided without running anything: how to read ffprobe and
//! `-progress` output, and which arguments each operation needs (spec 3.1.1).

use crate::catalog::{ANIMATED_GIF_TARGETS, AUDIO_TARGETS, Operation, Tool, VIDEO_TARGETS};
use crate::config::Quality;
use crate::editor::{Axis, EditOp, Turn};
use crate::format::{Format, MediaKind};
use crate::options::{Preset, Strength};
use crate::video_edit::{AudioMode, Speed, VideoPlan};

/// What to run for one file. `input_args` go before `-i <input>`; each attempt goes
/// between `-i <input>` and the output path, and a later attempt runs only if the one
/// before it failed.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaPlan {
    pub output: Format,
    /// Arguments placed before `-i <input>`, such as an input seek.
    pub input_args: Vec<String>,
    pub attempts: Vec<Vec<String>>,
    pub duration_secs: Option<f64>,
}

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum PlanError {
    #[error("no audio stream")]
    NoAudio,
    #[error("no video stream")]
    NoVideo,
    #[error("unsupported")]
    Unsupported,
}

/// Work that FFmpeg does (everything else is the image engine's).
pub fn supports(op: &Operation, input: Format) -> bool {
    match (input.kind(), op) {
        (MediaKind::Video, Operation::Convert(t)) => *t != input && VIDEO_TARGETS.contains(t),
        (MediaKind::Video, Operation::Tool(t)) => {
            matches!(t, Tool::CompressVideo | Tool::ExtractAudio | Tool::Mute)
        }
        (MediaKind::Audio, Operation::Convert(t)) => *t != input && AUDIO_TARGETS.contains(t),
        (MediaKind::Image, Operation::Convert(t)) => {
            input == Format::Gif && ANIMATED_GIF_TARGETS.contains(t)
        }
        _ => false,
    }
}

/// The FFmpeg arguments for `op` on an input of `input` format, following
/// spec 3.1.1 and the chosen option (`None` = the option set's default).
pub fn plan(
    op: &Operation,
    preset: Option<Preset>,
    input: Format,
    info: &MediaInfo,
    quality: &Quality,
) -> Result<MediaPlan, PlanError> {
    if !supports(op, input) {
        return Err(PlanError::Unsupported);
    }
    let has_video = || info.video.is_some().then_some(()).ok_or(PlanError::NoVideo);
    let has_audio = || info.audio.is_some().then_some(()).ok_or(PlanError::NoAudio);
    let (output, attempts) = match *op {
        Operation::Convert(Format::Gif) => {
            has_video()?;
            (Format::Gif, vec![gif(preset)])
        }
        Operation::Convert(target) if target.kind() == MediaKind::Audio => {
            has_audio()?;
            (target, vec![encode_audio(target, quality)])
        }
        Operation::Convert(target) => {
            has_video()?;
            let max_short_side = match preset {
                Some(Preset::Resolution(h)) => h,
                _ => None,
            };
            let encode = Video {
                crf: 23,
                aac_kbps: 192,
                max_short_side,
                audio: true,
            };
            let mut attempts = Vec::new();
            if max_short_side.is_none()
                && input != Format::Gif
                && let Some(copy) = remux(target, info)
            {
                attempts.push(copy);
            }
            attempts.push(encode.args(target, quality));
            (target, attempts)
        }
        Operation::Tool(Tool::CompressVideo) => {
            has_video()?;
            let output = match input {
                Format::Mp4 | Format::Mkv | Format::Mov => input,
                _ => Format::Mp4,
            };
            let (crf, aac_kbps, max_short_side) = match preset {
                Some(Preset::Compress(Strength::Light)) => (24, 160, None),
                Some(Preset::Compress(Strength::Strong)) => (32, 96, Some(720)),
                _ => (28, 128, None),
            };
            let encode = Video {
                crf,
                aac_kbps,
                max_short_side,
                audio: true,
            };
            (output, vec![encode.args(output, quality)])
        }
        Operation::Tool(Tool::ExtractAudio) => {
            has_audio()?;
            let copyable = match info.audio.as_deref() {
                Some("aac") => Some(Format::M4a),
                Some("mp3") => Some(Format::Mp3),
                Some("opus") => Some(Format::Opus),
                Some("vorbis") => Some(Format::Ogg),
                Some("flac") => Some(Format::Flac),
                Some("pcm_s16le") => Some(Format::Wav),
                _ => None,
            };
            match copyable {
                Some(target) => {
                    let copy = args(&["-map", "0:a:0", "-c:a", "copy"], target);
                    (target, vec![copy, encode_audio(target, quality)])
                }
                None => (Format::Mp3, vec![encode_audio(Format::Mp3, quality)]),
            }
        }
        Operation::Tool(Tool::Mute) => {
            has_video()?;
            let output = if muxer(input).is_some() {
                input
            } else {
                Format::Mp4
            };
            let encode = Video {
                crf: 23,
                aac_kbps: 192,
                max_short_side: None,
                audio: false,
            };
            let mut attempts = Vec::new();
            if output == input {
                let mut copy = strings(&["-map", "0:v", "-an", "-c:v", "copy"]);
                if matches!(output, Format::Mp4 | Format::Mov) {
                    copy.extend(strings(&["-movflags", "+faststart"]));
                }
                attempts.push(finish(copy, output));
            }
            attempts.push(encode.args(output, quality));
            (output, attempts)
        }
        _ => return Err(PlanError::Unsupported),
    };
    Ok(MediaPlan {
        output,
        input_args: Vec::new(),
        attempts,
        duration_secs: info.duration_secs,
    })
}

/// libopus refuses some layouts, notably 5.1(side), which AC3 and DTS 5.1
/// decode to; FFmpeg converts to the nearest of these instead.
const OPUS_LAYOUTS: &str = "aformat=channel_layouts=7.1|5.1|stereo|mono";

/// libx264 rejects odd sizes; this rounds both sides down to even.
const EVEN_SIZE: &str = "scale=trunc(iw/2)*2:trunc(ih/2)*2";

/// Scales down so the short side is at most `h` pixels (never up), keeping
/// sizes even: "720p" of a portrait phone clip means 720 across, not tall.
fn max_short_side_filter(h: u16) -> String {
    format!(
        "scale='if(gte(iw,ih),-2,trunc(min({h},iw)/2)*2)':'if(gte(iw,ih),trunc(min({h},ih)/2)*2,-2)'"
    )
}

struct Video {
    /// H.264 CRF; VP9 and MPEG-4 use their own fixed quality.
    crf: u8,
    aac_kbps: u32,
    max_short_side: Option<u16>,
    audio: bool,
}

impl Video {
    fn args(&self, target: Format, quality: &Quality) -> Vec<String> {
        let mut a = strings(&["-map", "0:v:0"]);
        if self.audio {
            a.extend(strings(&["-map", "0:a?"]));
        }
        a.extend(video_codec(target, self.crf));
        let scale = self.max_short_side.map(max_short_side_filter);
        let vf = match target {
            Format::Webm | Format::Avi => scale,
            // MP4, MOV, MKV: libx264 needs even sizes even without scaling.
            _ => Some(scale.unwrap_or_else(|| EVEN_SIZE.to_string())),
        };
        if let Some(vf) = vf {
            a.extend(["-vf".to_string(), vf]);
        }
        if self.audio {
            a.extend(audio_codec(target, self.aac_kbps, quality));
            if target == Format::Webm {
                a.extend(strings(&["-af", OPUS_LAYOUTS]));
            }
        } else {
            a.push("-an".to_string());
        }
        a.extend(faststart(target));
        finish(a, target)
    }
}

/// The video encoder for `target` (`crf` is H.264's; VP9 and MPEG-4 use their own).
fn video_codec(target: Format, crf: u8) -> Vec<String> {
    match target {
        Format::Webm => strings(&[
            "-c:v",
            "libvpx-vp9",
            "-crf",
            "32",
            "-b:v",
            "0",
            "-deadline",
            "good",
            "-cpu-used",
            "4",
            "-row-mt",
            "1",
            // GIFs decode to RGB with alpha, which libvpx-vp9 refuses;
            // yuv420p is also what every player handles.
            "-pix_fmt",
            "yuv420p",
        ]),
        Format::Avi => strings(&["-c:v", "mpeg4", "-q:v", "4"]),
        // MP4, MOV, MKV
        _ => {
            let mut a = strings(&["-c:v", "libx264", "-preset", "medium", "-crf"]);
            a.push(crf.to_string());
            a.extend(strings(&["-pix_fmt", "yuv420p"]));
            a
        }
    }
}

/// The audio encoder for `target`; WebM's Opus also needs [`OPUS_LAYOUTS`] in the filters.
fn audio_codec(target: Format, aac_kbps: u32, quality: &Quality) -> Vec<String> {
    match target {
        Format::Webm => strings(&["-c:a", "libopus", "-b:a", "128k"]),
        Format::Avi => {
            let mut a = strings(&["-c:a", "libmp3lame", "-b:a"]);
            a.push(format!("{}k", quality.mp3_kbps));
            a
        }
        _ => {
            let mut a = strings(&["-c:a", "aac", "-b:a"]);
            a.push(format!("{aac_kbps}k"));
            a
        }
    }
}

fn faststart(target: Format) -> Vec<String> {
    if matches!(target, Format::Mp4 | Format::Mov) {
        strings(&["-movflags", "+faststart"])
    } else {
        Vec::new()
    }
}

/// The container a video edit saves to: the input's, except WMV (no encoder settings),
/// which saves as MP4.
pub fn video_edit_output(input: Format) -> Format {
    match input {
        Format::Wmv => Format::Mp4,
        other => other,
    }
}

/// The loudness target for "크기 맞춤".
const LOUDNORM: &str = "loudnorm=I=-16:TP=-1.5:LRA=11";

/// The FFmpeg arguments that save `plan` on an `input` video: one exact re-encode through
/// a filter graph that keeps `plan.kept`, then applies the frame ops, reverse and speed to
/// the picture and reverse, speed and loudness to the sound.
pub fn video_edit_plan(
    input: Format,
    info: &MediaInfo,
    plan: &VideoPlan,
    quality: &Quality,
) -> Result<MediaPlan, PlanError> {
    if info.video.is_none() {
        return Err(PlanError::NoVideo);
    }
    if plan.kept.is_empty() {
        return Err(PlanError::Unsupported);
    }
    let output = video_edit_output(input);
    let audio = info.audio.is_some() && plan.audio != AudioMode::Mute;
    let factor = plan.speed.factor();

    let mut input_args = Vec::new();
    let mut graph = Vec::new();
    let (v_in, a_in) = if let [(a, b)] = plan.kept[..] {
        let whole = a <= 0.0 && info.duration_secs.is_none_or(|d| b >= d);
        if !whole {
            input_args = vec!["-ss".into(), secs(a), "-t".into(), secs(b - a)];
        }
        ("[0:v:0]".to_string(), "[0:a:0]".to_string())
    } else {
        let n = plan.kept.len();
        let labels = |prefix: &str| (0..n).map(|i| format!("[{prefix}{i}]")).collect::<String>();
        graph.push(format!("[0:v:0]split={n}{}", labels("v")));
        for (i, (a, b)) in plan.kept.iter().enumerate() {
            graph.push(format!(
                "[v{i}]trim=start={}:end={},setpts=PTS-STARTPTS[pv{i}]",
                secs(*a),
                secs(*b)
            ));
        }
        if audio {
            graph.push(format!("[0:a:0]asplit={n}{}", labels("a")));
            for (i, (a, b)) in plan.kept.iter().enumerate() {
                graph.push(format!(
                    "[a{i}]atrim=start={}:end={},asetpts=PTS-STARTPTS[pa{i}]",
                    secs(*a),
                    secs(*b)
                ));
            }
        }
        let pieces: String = (0..n)
            .map(|i| {
                if audio {
                    format!("[pv{i}][pa{i}]")
                } else {
                    format!("[pv{i}]")
                }
            })
            .collect();
        let (a_flag, outs) = if audio { (1, "[cv][ca]") } else { (0, "[cv]") };
        graph.push(format!("{pieces}concat=n={n}:v=1:a={a_flag}{outs}"));
        ("[cv]".to_string(), "[ca]".to_string())
    };

    let mut video = frame_filters(&plan.frame);
    if plan.reverse {
        video.push("reverse".into());
    }
    if plan.speed != Speed::Normal {
        video.push(format!("setpts=PTS/{factor}"));
        // Back onto the source rate, or the encoder rounds the new timestamps
        // onto the old frame grid and the clip comes out longer.
        if let Some(fps) = info.fps.filter(|f| *f > 0.0) {
            video.push(format!("fps={fps}"));
        }
    }
    video.push(EVEN_SIZE.into());
    graph.push(format!("{v_in}{}[vout]", video.join(",")));

    if audio {
        let mut sound = Vec::new();
        if plan.reverse {
            sound.push("areverse".to_string());
        }
        if plan.speed != Speed::Normal {
            sound.push(format!("atempo={factor}"));
        }
        if plan.audio == AudioMode::Normalize {
            sound.push(LOUDNORM.into());
        }
        if output == Format::Webm {
            sound.push(OPUS_LAYOUTS.into());
        }
        if sound.is_empty() {
            sound.push("anull".into());
        }
        graph.push(format!("{a_in}{}[aout]", sound.join(",")));
    }

    let mut a = strings(&["-filter_complex"]);
    a.push(graph.join(";"));
    a.extend(strings(&["-map", "[vout]"]));
    if audio {
        a.extend(strings(&["-map", "[aout]"]));
    } else {
        a.push("-an".into());
    }
    a.extend(video_codec(output, 23));
    if audio {
        a.extend(audio_codec(output, 192, quality));
        if plan.audio == AudioMode::Normalize {
            // loudnorm resamples to 192 kHz; Opus only takes 48 kHz.
            let rate = match output {
                Format::Webm => 48_000,
                _ => info.sample_rate.unwrap_or(48_000),
            };
            a.extend(["-ar".to_string(), rate.to_string()]);
        }
    }
    a.extend(faststart(output));

    let kept: f64 = plan.kept.iter().map(|(a, b)| b - a).sum();
    Ok(MediaPlan {
        output,
        input_args,
        attempts: vec![finish(a, output)],
        duration_secs: Some(kept / factor),
    })
}

/// How far before the asked time the frame save starts decoding.
const FRAME_LOOKBACK: f64 = 1.0;

/// The arguments that save the picture shown at `t` seconds, with the frame ops applied,
/// as PNG. A seek alone would give the first frame at or after `t` (the next one, and none
/// at all past the last frame's start), so this decodes the second before `t` and keeps
/// the last frame that starts by then (`-update 1` overwrites the one image).
pub fn video_frame_plan(t: f64, frame: &[EditOp]) -> MediaPlan {
    let start = (t - FRAME_LOOKBACK).max(0.0);
    let mut a = strings(&["-map", "0:v:0", "-t"]);
    a.push(secs(t - start + 0.001));
    let filters = frame_filters(frame);
    if !filters.is_empty() {
        a.extend(["-vf".to_string(), filters.join(",")]);
    }
    a.extend(strings(&["-c:v", "png", "-f", "image2", "-update", "1"]));
    MediaPlan {
        output: Format::Png,
        input_args: vec!["-ss".into(), secs(start)],
        attempts: vec![a],
        duration_secs: None,
    }
}

/// The size a scrub frame is decoded at: `display` fitted inside `longest` pixels, never
/// scaled up, both sides even and at least 2.
pub fn scrub_size(display: (u32, u32), longest: u32) -> (u32, u32) {
    let (w, h) = display;
    let scale = (f64::from(longest) / f64::from(w.max(h).max(1))).min(1.0);
    let side = |v: u32| (((f64::from(v) * scale) as u32) & !1).max(2);
    (side(w), side(h))
}

/// The arguments that decode one BGRA frame of `size` at `t` seconds to stdout: the part
/// before `-i` and the part after the input path.
pub fn scrub_args(t: f64, size: (u32, u32)) -> (Vec<String>, Vec<String>) {
    // As for `video_frame_plan`: the frames of the second before `t`; the last one is shown.
    let start = (t - FRAME_LOOKBACK).max(0.0);
    let mut after = strings(&["-t"]);
    after.push(secs(t - start + 0.001));
    after.push("-vf".into());
    after.push(format!("scale={}:{}", size.0, size.1));
    after.extend(strings(&["-f", "rawvideo", "-pix_fmt", "bgra", "pipe:1"]));
    (vec!["-ss".into(), secs(start)], after)
}

/// FFmpeg filters for crops, turns and flips, in order. Crop sizes are floored to even
/// (H.264 and VP9 need even sizes).
fn frame_filters(frame: &[EditOp]) -> Vec<String> {
    frame
        .iter()
        .filter_map(|op| match op {
            EditOp::Crop(r) => Some(format!("crop={}:{}:{}:{}", r.w & !1, r.h & !1, r.x, r.y)),
            EditOp::Rotate(Turn::Cw) => Some("transpose=1".into()),
            EditOp::Rotate(Turn::Ccw) => Some("transpose=2".into()),
            EditOp::Flip(Axis::Horizontal) => Some("hflip".into()),
            EditOp::Flip(Axis::Vertical) => Some("vflip".into()),
            EditOp::Grayscale(_) | EditOp::StripInfo(_) => None,
        })
        .collect()
}

/// Seconds as FFmpeg reads them, to the millisecond.
fn secs(t: f64) -> String {
    format!("{t:.3}")
}

/// Stream copy into `target`, when its streams can go there as they are.
fn remux(target: Format, info: &MediaInfo) -> Option<Vec<String>> {
    let video = info.video.as_deref()?;
    let audio = info.audio.as_deref();
    let fits = match target {
        Format::Mkv => true,
        Format::Mp4 | Format::Mov => {
            matches!(video, "h264" | "hevc") && matches!(audio, None | Some("aac" | "mp3"))
        }
        Format::Webm => {
            matches!(video, "vp8" | "vp9" | "av1")
                && matches!(audio, None | Some("opus" | "vorbis"))
        }
        _ => false,
    };
    if !fits {
        return None;
    }
    let mut a = strings(&["-map", "0:v:0", "-map", "0:a?", "-c", "copy"]);
    if matches!(target, Format::Mp4 | Format::Mov) {
        if video == "hevc" {
            a.extend(strings(&["-tag:v", "hvc1"])); // plays in Apple players
        }
        a.extend(strings(&["-movflags", "+faststart"]));
    }
    Some(finish(a, target))
}

/// One palette per frame: a single global palette would make FFmpeg hold the
/// whole video in memory until the input ends (gigabytes for a few minutes).
fn gif(preset: Option<Preset>) -> Vec<String> {
    let (fps, max_width) = match preset {
        Some(Preset::Gif { fps, max_width }) => (fps, max_width),
        _ => (15, Some(480)),
    };
    let scale = max_width
        .map(|w| format!("scale='min({w},iw)':-2:flags=lanczos,"))
        .unwrap_or_default();
    let filter = format!(
        "fps={fps},{scale}split[a][b];[a]palettegen=stats_mode=single[p];[b][p]paletteuse=new=1"
    );
    let mut a = strings(&["-map", "0:v:0", "-vf"]);
    a.push(filter);
    a.extend(strings(&["-loop", "0"]));
    finish(a, Format::Gif)
}

fn encode_audio(target: Format, quality: &Quality) -> Vec<String> {
    let mut a = strings(&["-map", "0:a:0", "-c:a"]);
    match target {
        Format::Mp3 => {
            a.extend(strings(&["libmp3lame", "-b:a"]));
            a.push(format!("{}k", quality.mp3_kbps));
        }
        Format::M4a => a.extend(strings(&["aac", "-b:a", "192k"])),
        Format::Wav => a.push("pcm_s16le".into()),
        Format::Flac => a.push("flac".into()),
        Format::Ogg => a.extend(strings(&["libvorbis", "-q:a", "6"])),
        _ => a.extend(strings(&["libopus", "-b:a", "128k", "-af", OPUS_LAYOUTS])), // Opus
    }
    finish(a, target)
}

fn strings(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

fn args(parts: &[&str], target: Format) -> Vec<String> {
    finish(strings(parts), target)
}

/// Every attempt names its muxer explicitly (see `muxer`).
fn finish(mut a: Vec<String>, target: Format) -> Vec<String> {
    a.extend(["-f".to_string(), muxer(target).unwrap_or("mp4").to_string()]);
    a
}

/// What ffprobe says about an input: its length, the codec of the first video and the
/// first audio stream, and the first video stream's size, rate and rotation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MediaInfo {
    pub duration_secs: Option<f64>,
    pub video: Option<String>,
    pub audio: Option<String>,
    /// Display width: the stored size turned by `rotation`.
    pub width: Option<u32>,
    /// Display height: the stored size turned by `rotation`.
    pub height: Option<u32>,
    pub fps: Option<f64>,
    /// Of the first audio stream.
    pub sample_rate: Option<u32>,
    /// Display rotation in degrees: 0, 90, 180 or 270.
    pub rotation: u32,
}

/// Parses `ffprobe -v error -show_entries format=duration:stream=codec_type,codec_name,
/// width,height,avg_frame_rate,r_frame_rate,sample_rate:stream_side_data=rotation
/// -of default=noprint_wrappers=1`: each stream prints `codec_name=` then `codec_type=`,
/// then its other fields; the container's `duration=` comes last.
pub fn parse_probe(text: &str) -> MediaInfo {
    #[derive(PartialEq)]
    enum Stream {
        FirstVideo,
        FirstAudio,
        Other,
    }
    let mut info = MediaInfo::default();
    let mut codec = None;
    let mut stream = Stream::Other;
    let (mut avg_rate, mut r_rate) = (None, None);
    for line in text.lines() {
        let Some((key, value)) = line.trim_end().split_once('=') else {
            continue;
        };
        let video = stream == Stream::FirstVideo;
        match key {
            "codec_name" => codec = Some(value.to_string()),
            "codec_type" => {
                stream = Stream::Other;
                let (slot, first) = match value {
                    "video" => (&mut info.video, Stream::FirstVideo),
                    "audio" => (&mut info.audio, Stream::FirstAudio),
                    _ => continue,
                };
                if slot.is_none() {
                    *slot = codec.take();
                    stream = first;
                }
            }
            "width" if video => info.width = value.parse().ok(),
            "height" if video => info.height = value.parse().ok(),
            "avg_frame_rate" if video => avg_rate = parse_rate(value),
            "r_frame_rate" if video => r_rate = parse_rate(value),
            "rotation" if video => {
                if let Ok(degrees) = value.parse::<f64>() {
                    info.rotation = ((degrees / 90.0).round() as i64 * 90).rem_euclid(360) as u32;
                }
            }
            "sample_rate" if stream == Stream::FirstAudio => info.sample_rate = value.parse().ok(),
            "duration" => info.duration_secs = value.parse().ok(),
            _ => {}
        }
    }
    info.fps = avg_rate.or(r_rate);
    if info.rotation % 180 == 90 {
        std::mem::swap(&mut info.width, &mut info.height);
    }
    info
}

/// A rate like `30000/1001`; `None` for `0/0` and other non-positive rates.
fn parse_rate(value: &str) -> Option<f64> {
    let (num, den) = value.split_once('/')?;
    let rate = num.parse::<f64>().ok()? / den.parse::<f64>().ok()?;
    (rate.is_finite() && rate > 0.0).then_some(rate)
}

#[derive(Debug, PartialEq)]
pub enum Progress {
    /// Seconds of output written so far.
    OutTime(f64),
    End,
}

/// One line of `-progress pipe:1` output. `out_time_ms` is microseconds too,
/// despite its name; older builds only print that one.
pub fn parse_progress(line: &str) -> Option<Progress> {
    let (key, value) = line.trim_end().split_once('=')?;
    match key {
        "out_time_us" | "out_time_ms" => value
            .parse::<u64>()
            .ok()
            .map(|us| Progress::OutTime(us as f64 / 1_000_000.0)),
        "progress" if value == "end" => Some(Progress::End),
        _ => None,
    }
}

/// Share of the work done, 0..=1; unknown without a positive duration.
pub fn fraction(out_time_secs: f64, duration_secs: Option<f64>) -> Option<f32> {
    let total = duration_secs.filter(|d| *d > 0.0)?;
    Some((out_time_secs / total).clamp(0.0, 1.0) as f32)
}

/// The `-f` name for an output format. Temp outputs end in `.swisswheel-tmp`,
/// so FFmpeg can never guess it from the extension.
pub fn muxer(format: Format) -> Option<&'static str> {
    Some(match format {
        Format::Mp4 => "mp4",
        Format::Mkv => "matroska",
        Format::Webm => "webm",
        Format::Mov => "mov",
        Format::Avi => "avi",
        Format::Gif => "gif",
        Format::Mp3 => "mp3",
        Format::M4a => "ipod",
        Format::Wav => "wav",
        Format::Flac => "flac",
        Format::Ogg => "ogg",
        Format::Opus => "opus",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{Axis, EditOp, PixelRect, Turn};
    use crate::video_edit::{AudioMode, Speed, VideoPlan};

    #[test]
    fn parses_probe_output() {
        let info = parse_probe(
            "codec_name=h264\ncodec_type=video\ncodec_name=aac\ncodec_type=audio\nduration=2.000000\n",
        );
        assert_eq!(
            info,
            MediaInfo {
                duration_secs: Some(2.0),
                video: Some("h264".into()),
                audio: Some("aac".into()),
                ..Default::default()
            }
        );
    }

    #[test]
    fn probe_keeps_first_of_each_kind_and_tolerates_na_and_crlf() {
        let info = parse_probe(
            "codec_name=mjpeg\ncodec_type=video\ncodec_name=flac\ncodec_type=audio\ncodec_name=png\ncodec_type=video\nduration=N/A\n",
        );
        assert_eq!(
            info,
            MediaInfo {
                duration_secs: None,
                video: Some("mjpeg".into()),
                audio: Some("flac".into()),
                ..Default::default()
            }
        );
        let gif = parse_probe("codec_name=gif\r\ncodec_type=video\r\nduration=1.000000\r\n");
        assert_eq!(
            gif,
            MediaInfo {
                duration_secs: Some(1.0),
                video: Some("gif".into()),
                audio: None,
                ..Default::default()
            }
        );
    }

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
        assert_eq!(
            (i.width, i.height, i.rotation),
            (Some(1080), Some(1920), 270)
        );
        assert_eq!(i.fps, Some(25.0));
        let upside =
            parse_probe("codec_name=h264\ncodec_type=video\nwidth=640\nheight=480\nrotation=180\n");
        assert_eq!(
            (upside.width, upside.height, upside.rotation),
            (Some(640), Some(480), 180)
        );
    }

    #[test]
    fn parses_progress_lines() {
        assert_eq!(
            parse_progress("out_time_us=1500000"),
            Some(Progress::OutTime(1.5))
        );
        assert_eq!(
            parse_progress("out_time_ms=2000000"),
            Some(Progress::OutTime(2.0))
        );
        assert_eq!(parse_progress("out_time_us=N/A"), None);
        assert_eq!(parse_progress("progress=end\r"), Some(Progress::End));
        assert_eq!(parse_progress("frame=20"), None);
    }

    #[test]
    fn fraction_is_clamped() {
        assert_eq!(fraction(1.0, Some(4.0)), Some(0.25));
        assert_eq!(fraction(5.0, Some(4.0)), Some(1.0));
        assert_eq!(fraction(1.0, None), None);
        assert_eq!(fraction(1.0, Some(0.0)), None);
    }

    fn info(video: Option<&str>, audio: Option<&str>) -> MediaInfo {
        MediaInfo {
            duration_secs: Some(2.0),
            video: video.map(String::from),
            audio: audio.map(String::from),
            ..Default::default()
        }
    }

    /// `seq` appears in `args` as a contiguous run.
    fn has(args: &[String], seq: &[&str]) -> bool {
        args.windows(seq.len())
            .any(|w| w.iter().zip(seq).all(|(a, b)| a == b))
    }

    fn sv(parts: &[&str]) -> Vec<String> {
        strings(parts)
    }

    fn vinfo(audio: bool) -> MediaInfo {
        MediaInfo {
            video: Some("h264".into()),
            audio: audio.then(|| "aac".into()),
            width: Some(1920),
            height: Some(1080),
            fps: Some(30.0),
            sample_rate: audio.then_some(44100),
            duration_secs: Some(10.0),
            ..Default::default()
        }
    }

    fn vplan(kept: Vec<(f64, f64)>) -> VideoPlan {
        VideoPlan {
            kept,
            frame: vec![],
            speed: Speed::Normal,
            audio: AudioMode::Keep,
            reverse: false,
        }
    }

    fn graph(p: &MediaPlan) -> String {
        let a = &p.attempts[0];
        a[a.iter().position(|s| s == "-filter_complex").unwrap() + 1].clone()
    }

    #[test]
    fn conversion_args_unchanged() {
        // Recorded from the code before `Video::args` was split for video edits.
        let input = MediaInfo {
            video: Some("h264".into()),
            audio: Some("aac".into()),
            duration_secs: Some(2.0),
            ..Default::default()
        };
        let attempts = |t| {
            convert(t, None, Format::Mkv, &input, &Quality::default())
                .unwrap()
                .attempts
        };
        assert_eq!(
            attempts(Format::Mp4),
            vec![
                sv(&[
                    "-map",
                    "0:v:0",
                    "-map",
                    "0:a?",
                    "-c",
                    "copy",
                    "-movflags",
                    "+faststart",
                    "-f",
                    "mp4"
                ]),
                sv(&[
                    "-map",
                    "0:v:0",
                    "-map",
                    "0:a?",
                    "-c:v",
                    "libx264",
                    "-preset",
                    "medium",
                    "-crf",
                    "23",
                    "-pix_fmt",
                    "yuv420p",
                    "-vf",
                    "scale=trunc(iw/2)*2:trunc(ih/2)*2",
                    "-c:a",
                    "aac",
                    "-b:a",
                    "192k",
                    "-movflags",
                    "+faststart",
                    "-f",
                    "mp4"
                ]),
            ]
        );
        assert_eq!(
            attempts(Format::Webm),
            vec![sv(&[
                "-map",
                "0:v:0",
                "-map",
                "0:a?",
                "-c:v",
                "libvpx-vp9",
                "-crf",
                "32",
                "-b:v",
                "0",
                "-deadline",
                "good",
                "-cpu-used",
                "4",
                "-row-mt",
                "1",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "libopus",
                "-b:a",
                "128k",
                "-af",
                "aformat=channel_layouts=7.1|5.1|stereo|mono",
                "-f",
                "webm"
            ])]
        );
        assert_eq!(
            attempts(Format::Avi),
            vec![sv(&[
                "-map",
                "0:v:0",
                "-map",
                "0:a?",
                "-c:v",
                "mpeg4",
                "-q:v",
                "4",
                "-c:a",
                "libmp3lame",
                "-b:a",
                "192k",
                "-f",
                "avi"
            ])]
        );
    }

    #[test]
    fn one_range_seeks_on_the_input() {
        let p = video_edit_plan(
            Format::Mp4,
            &vinfo(true),
            &vplan(vec![(2.0, 5.5)]),
            &Quality::default(),
        )
        .unwrap();
        assert_eq!(p.input_args, ["-ss", "2.000", "-t", "3.500"]);
        assert!(!graph(&p).contains("trim"));
        assert_eq!(p.duration_secs, Some(3.5));
        assert_eq!(p.output, Format::Mp4);
    }

    #[test]
    fn several_ranges_trim_and_concat() {
        let p = video_edit_plan(
            Format::Mkv,
            &vinfo(true),
            &vplan(vec![(0.0, 3.0), (6.0, 10.0)]),
            &Quality::default(),
        )
        .unwrap();
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
        plan.frame = vec![
            EditOp::Rotate(Turn::Cw),
            EditOp::Crop(PixelRect {
                x: 1,
                y: 2,
                w: 100,
                h: 50,
            }),
            EditOp::Flip(Axis::Horizontal),
        ];
        plan.reverse = true;
        plan.speed = Speed::Double;
        let g =
            graph(&video_edit_plan(Format::Mp4, &vinfo(true), &plan, &Quality::default()).unwrap());
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
        plan.frame = vec![EditOp::Crop(PixelRect {
            x: 3,
            y: 3,
            w: 101,
            h: 57,
        })];
        let g =
            graph(&video_edit_plan(Format::Mp4, &vinfo(true), &plan, &Quality::default()).unwrap());
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
    fn speed_keeps_the_source_frame_rate() {
        // Without `fps` the encoder rounds the new timestamps onto the old
        // frame grid and the clip comes out longer than asked.
        let mut plan = vplan(vec![(0.0, 10.0)]);
        plan.speed = Speed::Double;
        let g =
            graph(&video_edit_plan(Format::Mp4, &vinfo(true), &plan, &Quality::default()).unwrap());
        assert!(g.contains("setpts=PTS/2,fps=30,"), "{g}");
        let unknown = MediaInfo {
            fps: None,
            ..vinfo(true)
        };
        let g = graph(&video_edit_plan(Format::Mp4, &unknown, &plan, &Quality::default()).unwrap());
        assert!(g.contains("setpts=PTS/2,scale"), "{g}");
        plan.speed = Speed::Normal;
        let g =
            graph(&video_edit_plan(Format::Mp4, &vinfo(true), &plan, &Quality::default()).unwrap());
        assert!(!g.contains("fps="), "{g}");
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
        let p = video_edit_plan(
            Format::Wmv,
            &vinfo(true),
            &vplan(vec![(0.0, 10.0)]),
            &Quality::default(),
        )
        .unwrap();
        assert_eq!(p.output, Format::Mp4);
        assert!(has(&p.attempts[0], &["-c:v", "libx264"]));
    }

    #[test]
    fn frame_and_scrub_args() {
        let f = video_frame_plan(12.4, &[EditOp::Rotate(Turn::Ccw)]);
        assert_eq!(f.output, Format::Png);
        assert_eq!(f.input_args, ["-ss", "11.400"]);
        assert!(
            has(&f.attempts[0], &["-t", "1.001"]) && has(&f.attempts[0], &["-vf", "transpose=2"])
        );
        let first = video_frame_plan(0.3, &[]);
        assert_eq!(first.input_args, ["-ss", "0.000"]);
        assert!(has(&first.attempts[0], &["-t", "0.301"]));
        assert!(has(
            &f.attempts[0],
            &["-c:v", "png", "-f", "image2", "-update", "1"]
        ));
        assert_eq!(scrub_size((1920, 1080), 960), (960, 540));
        assert_eq!(scrub_size((101, 51), 960), (100, 50));
        let (before, after) = scrub_args(1.5, (960, 540));
        assert_eq!(before, ["-ss", "0.500"]);
        assert!(has(&after, &["-t", "1.001"]));
        assert!(has(
            &after,
            &[
                "-vf",
                "scale=960:540",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "bgra",
                "pipe:1"
            ]
        ));
    }

    fn ends_with_f(args: &[String], muxer: &str) -> bool {
        args.len() >= 2 && args[args.len() - 2] == "-f" && args[args.len() - 1] == muxer
    }

    fn convert(
        target: Format,
        preset: Option<Preset>,
        input: Format,
        info: &MediaInfo,
        quality: &Quality,
    ) -> Result<MediaPlan, PlanError> {
        plan(&Operation::Convert(target), preset, input, info, quality)
    }

    #[test]
    fn video_convert_reencodes_with_spec_quality() {
        let q = Quality::default();
        let p = convert(
            Format::Mp4,
            None,
            Format::Avi,
            &info(Some("mpeg4"), Some("mp3")),
            &q,
        )
        .unwrap();
        assert_eq!((p.output, p.attempts.len()), (Format::Mp4, 1));
        let a = &p.attempts[0];
        assert!(has(a, &["-c:v", "libx264"]) && has(a, &["-crf", "23"]));
        assert!(has(a, &["-c:a", "aac", "-b:a", "192k"]));
        assert!(has(a, &["-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2"]));
        assert!(has(a, &["-movflags", "+faststart"]) && ends_with_f(a, "mp4"));
        let w = &convert(
            Format::Webm,
            None,
            Format::Avi,
            &info(Some("mpeg4"), Some("mp3")),
            &q,
        )
        .unwrap()
        .attempts[0];
        assert!(has(w, &["-c:v", "libvpx-vp9", "-crf", "32"]));
        assert!(has(w, &["-c:a", "libopus"]) && ends_with_f(w, "webm"));
        let avi = &convert(
            Format::Avi,
            None,
            Format::Mp4,
            &info(Some("h264"), Some("aac")),
            &q,
        )
        .unwrap()
        .attempts[0];
        assert!(has(avi, &["-c:v", "mpeg4"]) && has(avi, &["-b:a", "192k"]));
        assert!(ends_with_f(avi, "avi"));
    }

    #[test]
    fn compatible_streams_are_remuxed_first() {
        let q = Quality::default();
        let mkv = convert(
            Format::Mkv,
            None,
            Format::Mp4,
            &info(Some("h264"), Some("aac")),
            &q,
        )
        .unwrap();
        assert_eq!(mkv.attempts.len(), 2);
        assert!(has(&mkv.attempts[0], &["-c", "copy"]));
        assert!(ends_with_f(&mkv.attempts[0], "matroska"));
        assert!(has(&mkv.attempts[1], &["-c:v", "libx264"]));
        let hevc = convert(
            Format::Mp4,
            None,
            Format::Mkv,
            &info(Some("hevc"), Some("aac")),
            &q,
        )
        .unwrap();
        assert!(has(&hevc.attempts[0], &["-tag:v", "hvc1"]));
        let vp9 = convert(
            Format::Webm,
            None,
            Format::Mkv,
            &info(Some("vp9"), Some("opus")),
            &q,
        )
        .unwrap();
        assert!(has(&vp9.attempts[0], &["-c", "copy"]));
        let tries = |target, input, v: &str, a: &str| {
            convert(target, None, input, &info(Some(v), Some(a)), &q)
                .unwrap()
                .attempts
                .len()
        };
        assert_eq!(tries(Format::Mp4, Format::Mkv, "h264", "opus"), 1); // opus isn't an MP4 copy by our rule
        assert_eq!(tries(Format::Avi, Format::Mkv, "mpeg4", "mp3"), 1); // AVI never copies
    }

    #[test]
    fn resolution_option_reencodes_and_scales_down() {
        let q = Quality::default();
        let src = info(Some("h264"), Some("aac"));
        let p = convert(
            Format::Mkv,
            Some(Preset::Resolution(Some(720))),
            Format::Mp4,
            &src,
            &q,
        )
        .unwrap();
        assert_eq!(p.attempts.len(), 1); // no copy when resizing
        assert!(has(
            &p.attempts[0],
            &[
                "-vf",
                "scale='if(gte(iw,ih),-2,trunc(min(720,iw)/2)*2)':'if(gte(iw,ih),trunc(min(720,ih)/2)*2,-2)'"
            ]
        ));
        let w = convert(
            Format::Webm,
            Some(Preset::Resolution(Some(480))),
            Format::Mp4,
            &src,
            &q,
        )
        .unwrap();
        assert!(has(
            &w.attempts[0],
            &[
                "-vf",
                "scale='if(gte(iw,ih),-2,trunc(min(480,iw)/2)*2)':'if(gte(iw,ih),trunc(min(480,ih)/2)*2,-2)'"
            ]
        ));
        let original = convert(
            Format::Mkv,
            Some(Preset::Resolution(None)),
            Format::Mp4,
            &src,
            &q,
        )
        .unwrap();
        assert_eq!(original.attempts.len(), 2);
    }

    #[test]
    fn animated_gif_to_video_encodes_with_even_size() {
        let p = convert(
            Format::Mp4,
            None,
            Format::Gif,
            &info(Some("gif"), None),
            &Quality::default(),
        )
        .unwrap();
        assert_eq!(p.attempts.len(), 1);
        assert!(has(
            &p.attempts[0],
            &["-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2"]
        ));
    }

    #[test]
    fn opus_output_accepts_surround_layouts() {
        // libopus rejects 5.1(side), which AC3/DTS 5.1 decodes to.
        let q = Quality::default();
        let downmix = ["-af", "aformat=channel_layouts=7.1|5.1|stereo|mono"];
        let webm = convert(
            Format::Webm,
            None,
            Format::Mkv,
            &info(Some("h264"), Some("ac3")),
            &q,
        )
        .unwrap();
        assert!(has(webm.attempts.last().unwrap(), &downmix));
        let opus = convert(
            Format::Opus,
            None,
            Format::Flac,
            &info(None, Some("flac")),
            &q,
        )
        .unwrap();
        assert!(has(&opus.attempts[0], &downmix));
    }

    #[test]
    fn vp9_output_pins_a_playable_pixel_format() {
        // A GIF decodes to RGB with alpha; left alone, FFmpeg picks gbrap, which
        // libvpx-vp9 refuses without -strict experimental.
        let q = Quality::default();
        for (input, info) in [
            (Format::Gif, info(Some("gif"), None)),
            (Format::Avi, info(Some("mpeg4"), Some("mp3"))),
        ] {
            let p = convert(Format::Webm, None, input, &info, &q).unwrap();
            let encode = p.attempts.last().unwrap();
            assert!(
                has(encode, &["-pix_fmt", "yuv420p"]),
                "{input:?}: {encode:?}"
            );
        }
    }

    #[test]
    fn audio_targets() {
        let q = Quality::default();
        let mp3 = &convert(
            Format::Mp3,
            None,
            Format::Flac,
            &info(None, Some("flac")),
            &q,
        )
        .unwrap()
        .attempts[0];
        assert!(has(mp3, &["-map", "0:a:0"]));
        assert!(has(mp3, &["-c:a", "libmp3lame", "-b:a", "192k"]) && ends_with_f(mp3, "mp3"));
        let loud = Quality {
            mp3_kbps: 320,
            ..Quality::default()
        };
        let p = convert(
            Format::Mp3,
            None,
            Format::Wav,
            &info(None, Some("pcm_s16le")),
            &loud,
        )
        .unwrap();
        assert!(has(&p.attempts[0], &["-b:a", "320k"]));
        let m4a = convert(Format::M4a, None, Format::Mp3, &info(None, Some("mp3")), &q).unwrap();
        assert!(ends_with_f(&m4a.attempts[0], "ipod"));
        let wav = convert(
            Format::Wav,
            None,
            Format::Flac,
            &info(None, Some("flac")),
            &q,
        )
        .unwrap();
        assert!(has(&wav.attempts[0], &["-c:a", "pcm_s16le"]));
        assert!(
            convert(
                Format::Mp3,
                None,
                Format::Mp4,
                &info(Some("h264"), Some("aac")),
                &q
            )
            .is_ok()
        );
        assert_eq!(
            convert(
                Format::Mp3,
                None,
                Format::Mp4,
                &info(Some("h264"), None),
                &q
            ),
            Err(PlanError::NoAudio)
        );
    }

    #[test]
    fn gif_options() {
        let q = Quality::default();
        let vf = |preset| {
            let p = convert(
                Format::Gif,
                preset,
                Format::Mp4,
                &info(Some("h264"), Some("aac")),
                &q,
            )
            .unwrap();
            assert_eq!(p.output, Format::Gif);
            assert!(has(&p.attempts[0], &["-loop", "0"]) && ends_with_f(&p.attempts[0], "gif"));
            p.attempts[0]
                .iter()
                .find(|s| s.contains("paletteuse"))
                .unwrap()
                .clone()
        };
        assert!(vf(None).starts_with("fps=15,scale='min(480,iw)':-2:flags=lanczos,"));
        assert!(
            vf(Some(Preset::Gif {
                fps: 10,
                max_width: Some(320)
            }))
            .starts_with("fps=10,scale='min(320,iw)':-2")
        );
        assert!(
            vf(Some(Preset::Gif {
                fps: 24,
                max_width: None
            }))
            .starts_with("fps=24,split")
        );
    }

    #[test]
    fn gif_palette_streams_instead_of_buffering_the_video() {
        // A global palette makes FFmpeg hold every frame until the input ends
        // (1.9 GB for 60 s of 720p); a per-frame palette streams.
        let p = convert(
            Format::Gif,
            None,
            Format::Mp4,
            &info(Some("h264"), Some("aac")),
            &Quality::default(),
        )
        .unwrap();
        let vf = p.attempts[0]
            .iter()
            .find(|s| s.contains("palette"))
            .unwrap();
        assert!(vf.contains("palettegen=stats_mode=single"), "{vf}");
        assert!(vf.ends_with("paletteuse=new=1"), "{vf}");
    }

    #[test]
    fn video_tools() {
        let q = Quality::default();
        let tool =
            |t, preset, input, info: &MediaInfo| plan(&Operation::Tool(t), preset, input, info, &q);
        let c = tool(
            Tool::CompressVideo,
            None,
            Format::Webm,
            &info(Some("vp9"), Some("opus")),
        )
        .unwrap();
        assert_eq!(c.output, Format::Mp4);
        assert!(has(&c.attempts[0], &["-crf", "28"]) && has(&c.attempts[0], &["-b:a", "128k"]));
        let strong = tool(
            Tool::CompressVideo,
            Some(Preset::Compress(Strength::Strong)),
            Format::Mp4,
            &info(Some("h264"), Some("aac")),
        )
        .unwrap();
        let a = &strong.attempts[0];
        assert!(has(a, &["-crf", "32"]) && has(a, &["-b:a", "96k"]));
        assert!(has(
            a,
            &[
                "-vf",
                "scale='if(gte(iw,ih),-2,trunc(min(720,iw)/2)*2)':'if(gte(iw,ih),trunc(min(720,ih)/2)*2,-2)'"
            ]
        ));
        let light = tool(
            Tool::CompressVideo,
            Some(Preset::Compress(Strength::Light)),
            Format::Mkv,
            &info(Some("h264"), Some("aac")),
        )
        .unwrap();
        assert_eq!(light.output, Format::Mkv);
        assert!(
            has(&light.attempts[0], &["-crf", "24"]) && has(&light.attempts[0], &["-b:a", "160k"])
        );
        let ex = tool(
            Tool::ExtractAudio,
            None,
            Format::Mp4,
            &info(Some("h264"), Some("aac")),
        )
        .unwrap();
        assert_eq!((ex.output, ex.attempts.len()), (Format::M4a, 2));
        assert!(has(&ex.attempts[0], &["-c:a", "copy"]));
        let ac3 = tool(
            Tool::ExtractAudio,
            None,
            Format::Mkv,
            &info(Some("h264"), Some("ac3")),
        )
        .unwrap();
        assert_eq!(ac3.output, Format::Mp3);
        assert_eq!(
            tool(
                Tool::ExtractAudio,
                None,
                Format::Mp4,
                &info(Some("h264"), None)
            ),
            Err(PlanError::NoAudio)
        );
        let mute = tool(
            Tool::Mute,
            None,
            Format::Mp4,
            &info(Some("h264"), Some("aac")),
        )
        .unwrap();
        assert_eq!(mute.output, Format::Mp4);
        assert!(has(&mute.attempts[0], &["-an"]) && has(&mute.attempts[0], &["-c:v", "copy"]));
        let wmv = tool(
            Tool::Mute,
            None,
            Format::Wmv,
            &info(Some("wmv2"), Some("wmav2")),
        )
        .unwrap();
        assert_eq!(wmv.output, Format::Mp4);
        assert!(has(&wmv.attempts[0], &["-c:v", "libx264"]));
    }

    #[test]
    fn supports_only_media_work() {
        use Format::*;
        assert!(supports(&Operation::Convert(Mp3), Mkv));
        assert!(supports(&Operation::Convert(Webm), Gif));
        assert!(supports(&Operation::Tool(Tool::ExtractAudio), Mp4));
        assert!(!supports(&Operation::Convert(Jpg), Png));
        assert!(!supports(&Operation::Convert(Png), Gif));
        assert!(!supports(&Operation::Convert(Mp4), Mp3));
        assert!(!supports(&Operation::Tool(Tool::Shrink), Mp4));
        assert!(!supports(&Operation::Tool(Tool::Mute), Mp3));
        assert_eq!(
            convert(
                Mp4,
                None,
                Mp4,
                &info(Some("h264"), None),
                &Quality::default()
            ),
            Err(PlanError::Unsupported)
        );
    }

    #[test]
    fn muxers_are_explicit() {
        use Format::*;
        for (f, m) in [
            (Mp4, "mp4"),
            (Mkv, "matroska"),
            (Webm, "webm"),
            (Mov, "mov"),
            (Avi, "avi"),
            (Gif, "gif"),
            (Mp3, "mp3"),
            (M4a, "ipod"),
            (Wav, "wav"),
            (Flac, "flac"),
            (Ogg, "ogg"),
            (Opus, "opus"),
        ] {
            assert_eq!(muxer(f), Some(m), "{f:?}");
        }
        for f in [Wmv, Wma, Aiff, Png] {
            assert_eq!(muxer(f), None, "{f:?}");
        }
    }
}
