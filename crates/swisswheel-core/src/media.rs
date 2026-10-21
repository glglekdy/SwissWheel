//! FFmpeg work, decided without running anything: how to read ffprobe and
//! `-progress` output, and which arguments each operation needs (spec 3.1.1).

use crate::format::Format;

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
