//! Video and audio work, done by FFmpeg in a child process (spec 4.4).

use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read};
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use swisswheel_core::catalog::Operation;
use swisswheel_core::config::Quality;
use swisswheel_core::format::Format;
use swisswheel_core::media::{self, MediaInfo, MediaPlan, PlanError, Progress};
use swisswheel_core::options::Preset;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject,
};
use windows::Win32::System::Threading::CREATE_NO_WINDOW;

use super::EngineError;

/// How often a running FFmpeg is checked for cancellation and exit.
const POLL: Duration = Duration::from_millis(100);
/// How much of FFmpeg's stderr is kept for the log.
const STDERR_TAIL: usize = 2048;

#[derive(Debug, Clone, PartialEq)]
pub struct Tools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

/// The bundled copy in `<exe folder>\ffmpeg\`, else one on `PATH`.
pub fn locate() -> Option<Tools> {
    let exe = std::env::current_exe().ok()?;
    locate_in(exe.parent()?, std::env::var_os("PATH").as_deref())
}

pub fn locate_in(app_dir: &Path, path_var: Option<&OsStr>) -> Option<Tools> {
    let in_dir = |dir: &Path| {
        let tools = Tools {
            ffmpeg: dir.join("ffmpeg.exe"),
            ffprobe: dir.join("ffprobe.exe"),
        };
        (tools.ffmpeg.is_file() && tools.ffprobe.is_file()).then_some(tools)
    };
    in_dir(&app_dir.join("ffmpeg"))
        .or_else(|| std::env::split_paths(path_var?).find_map(|dir| in_dir(&dir)))
}

/// Reads the input's streams and decides the FFmpeg arguments.
pub fn prepare(
    tools: &Tools,
    op: &Operation,
    preset: Option<Preset>,
    input: &Path,
    quality: &Quality,
) -> Result<MediaPlan, EngineError> {
    let format = Format::from_path(input)
        .ok_or_else(|| EngineError::Unsupported(input.display().to_string()))?;
    let info = probe(tools, input)?;
    media::plan(op, preset, format, &info, quality).map_err(|e| match e {
        PlanError::NoAudio => EngineError::NoAudio,
        PlanError::NoVideo => EngineError::NoVideo,
        PlanError::Unsupported => EngineError::Unsupported(format!("{op:?} on {format:?}")),
    })
}

pub fn probe(tools: &Tools, input: &Path) -> Result<MediaInfo, EngineError> {
    let out = Command::new(&tools.ffprobe)
        .args(["-v", "error", "-show_entries"])
        .arg("format=duration:stream=codec_type,codec_name")
        .args(["-of", "default=noprint_wrappers=1"])
        .arg(input)
        .stdin(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW.0)
        .output()
        .map_err(|e| EngineError::MediaRead(format!("{}: ffprobe: {e}", input.display())))?;
    let info = media::parse_probe(&String::from_utf8_lossy(&out.stdout));
    if !out.status.success() || (info.video.is_none() && info.audio.is_none()) {
        return Err(EngineError::MediaRead(format!(
            "{}: {}",
            input.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(info)
}

/// Runs the plan's attempts in order until one succeeds. `progress` gets
/// 0..=1 as FFmpeg reports it; setting `cancel` kills FFmpeg within `POLL`.
/// `output` never survives a failure.
pub fn run(
    tools: &Tools,
    plan: &MediaPlan,
    input: &Path,
    output: &Path,
    progress: &dyn Fn(f32),
    cancel: &AtomicBool,
) -> Result<(), EngineError> {
    let mut last = EngineError::Failed("no attempts".into());
    for attempt in &plan.attempts {
        let result = run_once(
            tools,
            attempt,
            plan.duration_secs,
            input,
            output,
            progress,
            cancel,
        );
        match result {
            Ok(()) => return Ok(()),
            Err(e) => {
                let _ = std::fs::remove_file(output);
                if matches!(e, EngineError::Cancelled) {
                    return Err(e);
                }
                last = e;
            }
        }
    }
    Err(last)
}

fn run_once(
    tools: &Tools,
    args: &[String],
    duration: Option<f64>,
    input: &Path,
    output: &Path,
    progress: &dyn Fn(f32),
    cancel: &AtomicBool,
) -> Result<(), EngineError> {
    let mut child = Command::new(&tools.ffmpeg)
        .args(["-hide_banner", "-nostdin", "-loglevel", "error", "-y"])
        .args(["-progress", "pipe:1", "-nostats", "-i"])
        .arg(input)
        .args(args)
        .arg(output)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW.0)
        .spawn()
        .map_err(|e| EngineError::Failed(format!("ffmpeg: {e}")))?;
    kill_with_app(&child);

    let (tx, rx) = mpsc::channel();
    let stdout = child.stdout.take().expect("piped");
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let value = match media::parse_progress(&line) {
                Some(Progress::OutTime(t)) => media::fraction(t, duration),
                Some(Progress::End) => Some(1.0),
                None => None,
            };
            if let Some(v) = value {
                let _ = tx.send(v);
            }
        }
    });
    let mut stderr = child.stderr.take().expect("piped");
    let errors = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        let start = buf.len().saturating_sub(STDERR_TAIL);
        String::from_utf8_lossy(&buf[start..]).trim().to_string()
    });

    let mut last = 0.0f32;
    let status = loop {
        if cancel.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            let _ = errors.join();
            return Err(EngineError::Cancelled);
        }
        match rx.recv_timeout(POLL) {
            // FFmpeg's out_time can step back slightly; never report going backwards.
            Ok(v) if v >= last => {
                last = v;
                progress(v);
            }
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break wait(&mut child)?,
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|e| EngineError::Failed(e.to_string()))?
        {
            for v in rx.try_iter() {
                if v >= last {
                    last = v;
                    progress(v);
                }
            }
            break status;
        }
    };
    let _ = reader.join();
    let stderr = errors.join().unwrap_or_default();
    if status.success() {
        Ok(())
    } else {
        Err(EngineError::Failed(stderr))
    }
}

fn wait(child: &mut Child) -> Result<std::process::ExitStatus, EngineError> {
    child.wait().map_err(|e| EngineError::Failed(e.to_string()))
}

/// Puts FFmpeg in a job object that dies with the app, so a crash or a kill
/// of SwissWheel never leaves it running. The moment between spawning and
/// assigning is accepted.
fn kill_with_app(child: &Child) {
    static JOB: OnceLock<Option<isize>> = OnceLock::new();
    let job = JOB.get_or_init(|| unsafe {
        let job = CreateJobObjectW(None, None).ok()?;
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            std::ptr::from_ref(&limits).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
        .ok()?;
        // Kept open for the life of the process: closing it is what kills FFmpeg.
        Some(job.0 as isize)
    });
    if let Some(job) = *job {
        unsafe {
            let _ = AssignProcessToJobObject(HANDLE(job as *mut _), HANDLE(child.as_raw_handle()));
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};
    use swisswheel_core::catalog::{Operation, Tool};
    use swisswheel_core::config::Quality;
    use swisswheel_core::format::Format;
    use swisswheel_core::media::{MediaInfo, MediaPlan};
    use swisswheel_core::options::{Preset, Strength};

    /// Two seconds of 160×120 test pattern with a sine tone.
    pub(crate) const CLIP: &[&str] = &[
        "-f",
        "lavfi",
        "-i",
        "testsrc=duration=2:size=160x120:rate=10",
        "-f",
        "lavfi",
        "-i",
        "sine=duration=2",
        "-c:v",
        "libx264",
        "-c:a",
        "aac",
        "-shortest",
    ];

    pub(crate) fn tools() -> Tools {
        locate().expect("FFmpeg(ffmpeg.exe, ffprobe.exe)가 PATH에 필요합니다")
    }

    /// Writes `dir/name` with `ffmpeg -y <args>`.
    pub(crate) fn make(dir: &Path, name: &str, args: &[&str]) -> PathBuf {
        let out = dir.join(name);
        let status = Command::new(tools().ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-y"])
            .args(args)
            .arg(&out)
            .status()
            .unwrap();
        assert!(status.success(), "could not make fixture {name}");
        out
    }

    /// A test pattern of `size` lasting `secs`, without audio.
    pub(crate) fn pattern(dir: &Path, name: &str, size: &str, secs: u32) -> PathBuf {
        let src = format!("testsrc=duration={secs}:size={size}:rate=10");
        make(dir, name, &["-f", "lavfi", "-i", &src, "-c:v", "libx264"])
    }

    pub(crate) fn probe_out(path: &Path) -> MediaInfo {
        probe(&tools(), path).unwrap()
    }

    fn ffprobe_value(path: &Path, args: &[&str]) -> String {
        let out = Command::new(tools().ffprobe)
            .args(["-v", "error", "-select_streams", "v:0"])
            .args(args)
            .args(["-of", "default=noprint_wrappers=1:nokey=1"])
            .arg(path)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    pub(crate) fn dims(path: &Path) -> (u32, u32) {
        let text = ffprobe_value(path, &["-show_entries", "stream=width,height"]);
        let mut lines = text.lines().map(|l| l.trim().parse::<u32>().unwrap());
        (lines.next().unwrap(), lines.next().unwrap())
    }

    /// Runs `op` on `input` into `dir/name`, which is returned.
    fn convert(op: Operation, preset: Option<Preset>, input: &Path, name: &str) -> PathBuf {
        let tools = tools();
        let plan = prepare(&tools, &op, preset, input, &Quality::default()).unwrap();
        let out = input.with_file_name(name);
        run(&tools, &plan, input, &out, &|_| {}, &AtomicBool::new(false)).unwrap();
        out
    }

    fn codecs(path: &Path) -> (Option<String>, Option<String>) {
        let info = probe_out(path);
        (info.video, info.audio)
    }

    fn some(s: &str) -> Option<String> {
        Some(s.to_string())
    }

    #[test]
    fn locates_bundled_before_path() {
        let (app, path) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let touch = |dir: &Path| {
            for n in ["ffmpeg.exe", "ffprobe.exe"] {
                std::fs::write(dir.join(n), b"").unwrap();
            }
        };
        touch(path.path());
        let path_var = std::env::join_paths([path.path()]).unwrap();
        assert_eq!(
            locate_in(app.path(), Some(&path_var)).unwrap().ffmpeg,
            path.path().join("ffmpeg.exe")
        );
        let bundled = app.path().join("ffmpeg");
        std::fs::create_dir(&bundled).unwrap();
        std::fs::write(bundled.join("ffmpeg.exe"), b"").unwrap(); // no ffprobe yet → still PATH
        assert_eq!(
            locate_in(app.path(), Some(&path_var)).unwrap().ffmpeg,
            path.path().join("ffmpeg.exe")
        );
        touch(&bundled);
        assert_eq!(
            locate_in(app.path(), Some(&path_var)).unwrap(),
            Tools {
                ffmpeg: bundled.join("ffmpeg.exe"),
                ffprobe: bundled.join("ffprobe.exe")
            }
        );
        assert_eq!(locate_in(tempfile::tempdir().unwrap().path(), None), None);
    }

    #[test]
    fn converts_video() {
        let dir = tempfile::tempdir().unwrap();
        let clip = make(dir.path(), "clip.mp4", CLIP);
        let convert = |target, name| convert(Operation::Convert(target), None, &clip, name);
        assert_eq!(
            codecs(&convert(Format::Webm, "o.webm")),
            (some("vp9"), some("opus"))
        );
        assert_eq!(codecs(&convert(Format::Mp3, "o.mp3")), (None, some("mp3")));
        assert_eq!(codecs(&convert(Format::Gif, "o.gif")), (some("gif"), None));
        assert_eq!(
            codecs(&convert(Format::Mkv, "o.mkv")),
            (some("h264"), some("aac"))
        );
    }

    #[test]
    fn audio_conversions() {
        let dir = tempfile::tempdir().unwrap();
        let tone = make(
            dir.path(),
            "tone.flac",
            &["-f", "lavfi", "-i", "sine=duration=2", "-c:a", "flac"],
        );
        for (target, name, codec) in [
            (Format::Mp3, "o.mp3", "mp3"),
            (Format::Opus, "o.opus", "opus"),
            (Format::Wav, "o.wav", "pcm_s16le"),
        ] {
            let out = convert(Operation::Convert(target), None, &tone, name);
            let info = probe_out(&out);
            assert_eq!(info.audio.as_deref(), Some(codec));
            let secs = info.duration_secs.unwrap();
            assert!((secs - 2.0).abs() < 0.2, "{name}: {secs}");
        }
    }

    #[test]
    fn tools_on_video() {
        let dir = tempfile::tempdir().unwrap();
        let clip = make(dir.path(), "clip.mp4", CLIP);
        let tools = tools();
        let plan = prepare(
            &tools,
            &Operation::Tool(Tool::ExtractAudio),
            None,
            &clip,
            &Quality::default(),
        )
        .unwrap();
        assert_eq!(plan.output, Format::M4a);
        let audio = convert(Operation::Tool(Tool::ExtractAudio), None, &clip, "a.m4a");
        assert_eq!(codecs(&audio), (None, some("aac")));
        let muted = convert(Operation::Tool(Tool::Mute), None, &clip, "m.mp4");
        assert_eq!(codecs(&muted), (some("h264"), None));
        let small = convert(
            Operation::Tool(Tool::CompressVideo),
            Some(Preset::Compress(Strength::Strong)),
            &clip,
            "c.mp4",
        );
        assert_eq!(codecs(&small).0, some("h264"));
        let silent = pattern(dir.path(), "silent.mp4", "160x120", 1);
        let err = prepare(
            &tools,
            &Operation::Tool(Tool::ExtractAudio),
            None,
            &silent,
            &Quality::default(),
        )
        .unwrap_err();
        assert!(matches!(err, EngineError::NoAudio), "{err:?}");
    }

    #[test]
    fn animated_gif_to_mp4() {
        let dir = tempfile::tempdir().unwrap();
        let gif = make(
            dir.path(),
            "anim.gif",
            &["-f", "lavfi", "-i", "testsrc=duration=1:size=81x61:rate=5"],
        );
        let out = convert(Operation::Convert(Format::Mp4), None, &gif, "anim.mp4");
        assert_eq!(codecs(&out).0, some("h264"));
        let (w, h) = dims(&out);
        assert!(w % 2 == 0 && h % 2 == 0, "{w}x{h}");
    }

    #[test]
    fn animated_gif_to_webm() {
        let dir = tempfile::tempdir().unwrap();
        let gif = make(
            dir.path(),
            "anim.gif",
            &["-f", "lavfi", "-i", "testsrc=duration=1:size=81x61:rate=5"],
        );
        let out = convert(Operation::Convert(Format::Webm), None, &gif, "anim.webm");
        assert_eq!(codecs(&out).0, some("vp9"));
    }

    #[test]
    fn surround_audio_to_webm() {
        let dir = tempfile::tempdir().unwrap();
        let clip = make(
            dir.path(),
            "surround.mkv",
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=2:size=160x120:rate=10",
                "-f",
                "lavfi",
                "-i",
                "anullsrc=channel_layout=5.1(side):sample_rate=48000",
                "-t",
                "2",
                "-c:v",
                "libx264",
                "-c:a",
                "ac3",
            ],
        );
        let out = convert(
            Operation::Convert(Format::Webm),
            None,
            &clip,
            "surround.webm",
        );
        assert_eq!(codecs(&out), (some("vp9"), some("opus")));
    }

    #[test]
    fn resolution_scales_down_only() {
        let dir = tempfile::tempdir().unwrap();
        let hd = pattern(dir.path(), "hd.mp4", "1280x720", 1);
        let out = convert(
            Operation::Convert(Format::Mkv),
            Some(Preset::Resolution(Some(480))),
            &hd,
            "hd.mkv",
        );
        let (w, h) = dims(&out);
        assert_eq!(h, 480);
        assert!(w % 2 == 0 && (850..=856).contains(&w), "{w}x{h}");
        let clip = make(dir.path(), "clip.mp4", CLIP);
        let same = convert(
            Operation::Convert(Format::Mkv),
            Some(Preset::Resolution(Some(480))),
            &clip,
            "clip.mkv",
        );
        assert_eq!(dims(&same), (160, 120));
    }

    #[test]
    fn gif_option_sets_fps() {
        let dir = tempfile::tempdir().unwrap();
        let clip = make(dir.path(), "clip.mp4", CLIP);
        let out = convert(
            Operation::Convert(Format::Gif),
            Some(Preset::Gif {
                fps: 10,
                max_width: Some(80),
            }),
            &clip,
            "small.gif",
        );
        assert_eq!(dims(&out).0, 80);
        let frames: u32 = ffprobe_value(
            &out,
            &["-count_frames", "-show_entries", "stream=nb_read_frames"],
        )
        .parse()
        .unwrap();
        assert!((18..=22).contains(&frames), "{frames} frames");
    }

    #[test]
    fn progress_reaches_one() {
        let dir = tempfile::tempdir().unwrap();
        let clip = make(dir.path(), "clip.mp4", CLIP);
        let tools = tools();
        let plan = prepare(
            &tools,
            &Operation::Convert(Format::Webm),
            None,
            &clip,
            &Quality::default(),
        )
        .unwrap();
        let seen = RefCell::new(Vec::new());
        run(
            &tools,
            &plan,
            &clip,
            &dir.path().join("o.webm"),
            &|p| seen.borrow_mut().push(p),
            &AtomicBool::new(false),
        )
        .unwrap();
        let seen = seen.into_inner();
        assert!(!seen.is_empty());
        assert!(seen.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
        assert_eq!(seen.last(), Some(&1.0));
    }

    #[test]
    fn later_attempt_runs_when_first_fails() {
        let dir = tempfile::tempdir().unwrap();
        let clip = make(dir.path(), "clip.mp4", CLIP);
        let plan = MediaPlan {
            output: Format::Mp4,
            attempts: vec![
                ["-c", "copy", "-bsf:v", "no_such_filter", "-f", "mp4"]
                    .map(String::from)
                    .to_vec(),
                ["-c:v", "libx264", "-c:a", "aac", "-f", "mp4"]
                    .map(String::from)
                    .to_vec(),
            ],
            duration_secs: Some(2.0),
        };
        let out = dir.path().join("o.mp4.swisswheel-tmp");
        run(
            &tools(),
            &plan,
            &clip,
            &out,
            &|_| {},
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(codecs(&out).0, some("h264"));
    }

    #[test]
    fn cancel_kills_ffmpeg() {
        let dir = tempfile::tempdir().unwrap();
        let long = pattern(dir.path(), "long.mp4", "640x480", 20);
        let tools = tools();
        let plan = prepare(
            &tools,
            &Operation::Convert(Format::Webm),
            None,
            &long,
            &Quality::default(),
        )
        .unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancel);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            flag.store(true, Ordering::SeqCst);
        });
        let out = dir.path().join("long.webm.swisswheel-tmp");
        let started = Instant::now();
        let err = run(&tools, &plan, &long, &out, &|_| {}, &cancel).unwrap_err();
        assert!(matches!(err, EngineError::Cancelled), "{err:?}");
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(!out.exists());
    }

    #[test]
    fn unicode_paths() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("가족 영상");
        std::fs::create_dir(&sub).unwrap();
        let clip = make(&sub, "생일 파티.mp4", CLIP);
        let out = convert(
            Operation::Convert(Format::Mp3),
            None,
            &clip,
            "생일 파티.mp3.swisswheel-tmp",
        );
        assert_eq!(codecs(&out).1, some("mp3"));
    }

    #[test]
    fn unreadable_file_is_media_read_error() {
        let dir = tempfile::tempdir().unwrap();
        let broken = dir.path().join("broken.mp4");
        std::fs::write(&broken, b"not a video").unwrap();
        let err = prepare(
            &tools(),
            &Operation::Convert(Format::Mp3),
            None,
            &broken,
            &Quality::default(),
        )
        .unwrap_err();
        assert!(matches!(err, EngineError::MediaRead(_)), "{err:?}");
    }
}
