//! FFmpeg work, decided without running anything: how to read ffprobe and
//! `-progress` output, and which arguments each operation needs (spec 3.1.1).

use crate::catalog::{ANIMATED_GIF_TARGETS, AUDIO_TARGETS, Operation, Tool, VIDEO_TARGETS};
use crate::config::Quality;
use crate::format::{Format, MediaKind};
use crate::options::{Preset, Strength};

/// What to run for one file. Each attempt goes between `-i <input>` and the
/// output path; a later attempt runs only if the one before it failed.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaPlan {
    pub output: Format,
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
            let max_height = match preset {
                Some(Preset::Resolution(h)) => h,
                _ => None,
            };
            let encode = Video {
                crf: 23,
                aac_kbps: 192,
                max_height,
                audio: true,
            };
            let mut attempts = Vec::new();
            if max_height.is_none()
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
            let (crf, aac_kbps, max_height) = match preset {
                Some(Preset::Compress(Strength::Light)) => (24, 160, None),
                Some(Preset::Compress(Strength::Strong)) => (32, 96, Some(720)),
                _ => (28, 128, None),
            };
            let encode = Video {
                crf,
                aac_kbps,
                max_height,
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
                max_height: None,
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
        attempts,
        duration_secs: info.duration_secs,
    })
}

/// libopus refuses some layouts, notably 5.1(side), which AC3 and DTS 5.1
/// decode to; FFmpeg converts to the nearest of these instead.
const OPUS_LAYOUTS: &str = "aformat=channel_layouts=7.1|5.1|stereo|mono";

/// libx264 rejects odd sizes; this rounds both sides down to even.
const EVEN_SIZE: &str = "scale=trunc(iw/2)*2:trunc(ih/2)*2";

/// Scales down to at most `h` pixels high (never up), keeping sizes even.
fn max_height_filter(h: u16) -> String {
    format!("scale=-2:'trunc(min({h},ih)/2)*2'")
}

struct Video {
    /// H.264 CRF; VP9 and MPEG-4 use their own fixed quality.
    crf: u8,
    aac_kbps: u32,
    max_height: Option<u16>,
    audio: bool,
}

impl Video {
    fn args(&self, target: Format, quality: &Quality) -> Vec<String> {
        let mut a = strings(&["-map", "0:v:0"]);
        if self.audio {
            a.extend(strings(&["-map", "0:a?"]));
        }
        let scale = self.max_height.map(max_height_filter);
        match target {
            Format::Webm => {
                a.extend(strings(&[
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
                ]));
                if let Some(vf) = scale {
                    a.extend(["-vf".to_string(), vf]);
                }
                if self.audio {
                    a.extend(strings(&[
                        "-c:a",
                        "libopus",
                        "-b:a",
                        "128k",
                        "-af",
                        OPUS_LAYOUTS,
                    ]));
                }
            }
            Format::Avi => {
                a.extend(strings(&["-c:v", "mpeg4", "-q:v", "4"]));
                if let Some(vf) = scale {
                    a.extend(["-vf".to_string(), vf]);
                }
                if self.audio {
                    a.extend(strings(&["-c:a", "libmp3lame", "-b:a"]));
                    a.push(format!("{}k", quality.mp3_kbps));
                }
            }
            // MP4, MOV, MKV
            _ => {
                a.extend(strings(&["-c:v", "libx264", "-preset", "medium", "-crf"]));
                a.push(self.crf.to_string());
                a.extend(strings(&["-pix_fmt", "yuv420p", "-vf"]));
                a.push(scale.unwrap_or_else(|| EVEN_SIZE.to_string()));
                if self.audio {
                    a.extend(strings(&["-c:a", "aac", "-b:a"]));
                    a.push(format!("{}k", self.aac_kbps));
                } else {
                    a.push("-an".to_string());
                }
                if matches!(target, Format::Mp4 | Format::Mov) {
                    a.extend(strings(&["-movflags", "+faststart"]));
                }
            }
        }
        if !self.audio && matches!(target, Format::Webm | Format::Avi) {
            a.push("-an".to_string());
        }
        finish(a, target)
    }
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

/// What ffprobe says about an input: its length and the codec of the first
/// video and the first audio stream.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MediaInfo {
    pub duration_secs: Option<f64>,
    pub video: Option<String>,
    pub audio: Option<String>,
}

/// Parses `ffprobe -v error -show_entries format=duration:stream=codec_type,codec_name
/// -of default=noprint_wrappers=1`: each stream prints `codec_name=` then
/// `codec_type=`, and the container's `duration=` comes last.
pub fn parse_probe(text: &str) -> MediaInfo {
    let mut info = MediaInfo::default();
    let mut codec = None;
    for line in text.lines() {
        let Some((key, value)) = line.trim_end().split_once('=') else {
            continue;
        };
        match key {
            "codec_name" => codec = Some(value.to_string()),
            "codec_type" => {
                let slot = match value {
                    "video" => &mut info.video,
                    "audio" => &mut info.audio,
                    _ => continue,
                };
                if slot.is_none() {
                    *slot = codec.take();
                }
            }
            "duration" => info.duration_secs = value.parse().ok(),
            _ => {}
        }
    }
    info
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
                audio: Some("aac".into())
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
                audio: Some("flac".into())
            }
        );
        let gif = parse_probe("codec_name=gif\r\ncodec_type=video\r\nduration=1.000000\r\n");
        assert_eq!(
            gif,
            MediaInfo {
                duration_secs: Some(1.0),
                video: Some("gif".into()),
                audio: None
            }
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
        }
    }

    /// `seq` appears in `args` as a contiguous run.
    fn has(args: &[String], seq: &[&str]) -> bool {
        args.windows(seq.len())
            .any(|w| w.iter().zip(seq).all(|(a, b)| a == b))
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
            &["-vf", "scale=-2:'trunc(min(720,ih)/2)*2'"]
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
            &["-vf", "scale=-2:'trunc(min(480,ih)/2)*2'"]
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
        assert!(has(a, &["-vf", "scale=-2:'trunc(min(720,ih)/2)*2'"]));
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
