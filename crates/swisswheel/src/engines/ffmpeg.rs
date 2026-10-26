//! Video and audio work, done by FFmpeg in a child process (spec 4.4).

use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read};
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use swisswheel_core::catalog::Operation;
use swisswheel_core::config::Quality;
use swisswheel_core::format::Format;
use swisswheel_core::media::{self, MediaInfo, MediaPlan, PlanError, Progress};
use swisswheel_core::options::Preset;
use swisswheel_core::video_edit::VideoPlan;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
    QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
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

/// Where the app keeps its own FFmpeg, in the order it looks:
/// `<exe folder>\ffmpeg\`, then `%LOCALAPPDATA%\SwissWheel\ffmpeg\` (where the
/// download tile installs when the exe's folder can't be written).
pub fn install_dirs() -> Vec<PathBuf> {
    let app = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join("ffmpeg")));
    let local = std::env::var_os("LOCALAPPDATA")
        .map(|dir| PathBuf::from(dir).join("SwissWheel").join("ffmpeg"));
    app.into_iter().chain(local).collect()
}

/// The app's own copy (see [`install_dirs`]), else one on `PATH`.
pub fn locate() -> Option<Tools> {
    locate_in(&install_dirs(), std::env::var_os("PATH").as_deref())
}

/// The first of `dirs`, then of `PATH`, that has both tools.
pub fn locate_in(dirs: &[PathBuf], path_var: Option<&OsStr>) -> Option<Tools> {
    let in_dir = |dir: &Path| {
        let tools = Tools {
            ffmpeg: dir.join("ffmpeg.exe"),
            ffprobe: dir.join("ffprobe.exe"),
        };
        (tools.ffmpeg.is_file() && tools.ffprobe.is_file()).then_some(tools)
    };
    dirs.iter()
        .find_map(|dir| in_dir(dir))
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
    media::plan(op, preset, format, &info, quality)
        .map_err(|e| plan_error(e, || format!("{op:?} on {format:?}")))
}

/// Reads the input's streams and decides the arguments of a video editor save.
pub fn prepare_video_edit(
    tools: &Tools,
    plan: &VideoPlan,
    input: &Path,
    quality: &Quality,
) -> Result<MediaPlan, EngineError> {
    let format = Format::from_path(input)
        .ok_or_else(|| EngineError::Unsupported(input.display().to_string()))?;
    let info = probe(tools, input)?;
    media::video_edit_plan(format, &info, plan, quality)
        .map_err(|e| plan_error(e, || format!("video edit on {format:?}")))
}

fn plan_error(e: PlanError, what: impl FnOnce() -> String) -> EngineError {
    match e {
        PlanError::NoAudio => EngineError::NoAudio,
        PlanError::NoVideo => EngineError::NoVideo,
        PlanError::Unsupported => EngineError::Unsupported(what()),
    }
}

pub fn probe(tools: &Tools, input: &Path) -> Result<MediaInfo, EngineError> {
    let out = Command::new(&tools.ffprobe)
        .args(["-v", "error", "-show_entries"])
        .arg(
            "format=duration:stream=codec_type,codec_name,width,height,             avg_frame_rate,r_frame_rate,sample_rate:stream_side_data=rotation",
        )
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
            &plan.input_args,
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
                remove_output(output);
                if matches!(e, EngineError::Cancelled) {
                    return Err(e);
                }
                last = e;
            }
        }
    }
    Err(last)
}

#[allow(clippy::too_many_arguments)]
fn run_once(
    tools: &Tools,
    input_args: &[String],
    args: &[String],
    duration: Option<f64>,
    input: &Path,
    output: &Path,
    progress: &dyn Fn(f32),
    cancel: &AtomicBool,
) -> Result<(), EngineError> {
    let mut child = Command::new(&tools.ffmpeg)
        .args(["-hide_banner", "-nostdin", "-loglevel", "error", "-y"])
        .args(["-progress", "pipe:1", "-nostats"])
        .args(input_args)
        .arg("-i")
        .arg(input)
        .args(args)
        .arg(output)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW.0)
        .spawn()
        .map_err(|e| EngineError::Failed(format!("ffmpeg: {e}")))?;
    // Dropped (closing the job, which kills what is left) on every way out.
    let job = RunJob::new();
    if let Some(job) = &job {
        job.assign(&child);
    }

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
            // The whole job, so FFmpeg dies even behind a launcher (Chocolatey
            // shims, .cmd wrappers) and the pipes close for the readers below.
            match &job {
                Some(job) => job.terminate(),
                None => {
                    let _ = child.kill();
                }
            }
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

/// Deletes a failed attempt's output and waits (up to two seconds) until it
/// is really gone: right after FFmpeg is killed, a handle to the file can
/// still be open for a moment (the process winding down, or an antivirus
/// scan), and the file stays visible until it closes.
fn remove_output(output: &Path) {
    for _ in 0..40 {
        if !output.exists() {
            return;
        }
        let _ = std::fs::remove_file(output);
        std::thread::sleep(Duration::from_millis(50));
    }
    log_error!("임시 파일을 지우지 못했습니다: {}", output.display());
}

fn wait(child: &mut Child) -> Result<std::process::ExitStatus, EngineError> {
    child.wait().map_err(|e| EngineError::Failed(e.to_string()))
}

/// One FFmpeg run and everything it was started through, in a job object:
/// `terminate` stops all of it at once, and closing the handle (on drop, or
/// when the app dies) kills whatever is still running. The moment between
/// spawning and assigning is accepted; a launcher takes far longer than that
/// to start FFmpeg.
pub(crate) struct RunJob(HANDLE);

impl RunJob {
    pub(crate) fn new() -> Option<RunJob> {
        unsafe {
            let job = RunJob(CreateJobObjectW(None, None).ok()?);
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&limits).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
            .ok()?;
            Some(job)
        }
    }

    pub(crate) fn assign(&self, child: &Child) {
        unsafe {
            let _ = AssignProcessToJobObject(self.0, HANDLE(child.as_raw_handle()));
        }
    }

    /// Kills every process in the job and waits (up to two seconds) until they
    /// are gone: termination is asynchronous, and FFmpeg must have let go of
    /// its output before the caller deletes it.
    fn terminate(&self) {
        unsafe {
            let _ = TerminateJobObject(self.0, 1);
            for _ in 0..200 {
                let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
                let done = QueryInformationJobObject(
                    Some(self.0),
                    JobObjectBasicAccountingInformation,
                    std::ptr::from_mut(&mut info).cast(),
                    size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                    None,
                )
                .map_or(true, |()| info.ActiveProcesses == 0);
                if done {
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

impl Drop for RunJob {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
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
    use swisswheel_core::editor::{EditOp, PixelRect, Turn};
    use swisswheel_core::format::Format;
    use swisswheel_core::media::{MediaInfo, MediaPlan};
    use swisswheel_core::options::{Preset, Strength};
    use swisswheel_core::video_edit::{AudioMode, Speed};

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

    /// 320×240 at 10 fps for one second, stored with a 90° display rotation
    /// (as phones write portrait clips): shown 240×320.
    pub(crate) fn rotated_clip(dir: &Path) -> PathBuf {
        let plain = pattern(dir, "plain-for-rot.mp4", "320x240", 1);
        let plain = plain.to_str().unwrap();
        make(
            dir,
            "rot.mp4",
            &["-display_rotation:v:0", "90", "-i", plain, "-c", "copy"],
        )
    }

    /// `secs` of 320×240 test pattern at 10 fps with a 440 Hz tone at 44.1 kHz,
    /// in 4:2:0 like camera clips (Windows' H.264 decoder refuses 4:4:4).
    pub(crate) fn tone_clip(dir: &Path, name: &str, secs: u32) -> PathBuf {
        let video = format!("testsrc=duration={secs}:size=320x240:rate=10");
        let audio = format!("sine=frequency=440:sample_rate=44100:duration={secs}");
        make(
            dir,
            name,
            &[
                "-f",
                "lavfi",
                "-i",
                &video,
                "-f",
                "lavfi",
                "-i",
                &audio,
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-shortest",
            ],
        )
    }

    /// Two seconds of 64×64 at 10 fps: red for the first second, blue for the last.
    pub(crate) fn color_steps(dir: &Path, name: &str) -> PathBuf {
        make(
            dir,
            name,
            &[
                "-f",
                "lavfi",
                "-i",
                "color=red:s=64x64:r=10:d=1",
                "-f",
                "lavfi",
                "-i",
                "color=blue:s=64x64:r=10:d=1",
                "-filter_complex",
                "[0:v][1:v]concat=n=2:v=1:a=0",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
            ],
        )
    }

    /// Probes `input`, plans `plan` on it and runs it into `out`.
    pub(crate) fn run_video_edit(input: &Path, plan: &VideoPlan, out: &Path) -> MediaPlan {
        let tools = tools();
        let media = prepare_video_edit(&tools, plan, input, &Quality::default()).unwrap();
        run(&tools, &media, input, out, &|_| {}, &AtomicBool::new(false)).unwrap();
        media
    }

    fn edit(kept: Vec<(f64, f64)>) -> VideoPlan {
        VideoPlan {
            kept,
            frame: vec![],
            speed: Speed::Normal,
            audio: AudioMode::Keep,
            reverse: false,
        }
    }

    fn secs(path: &Path) -> f64 {
        probe_out(path).duration_secs.unwrap()
    }

    fn near(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() <= 0.15,
            "{actual} s, expected {expected} s"
        );
    }

    /// The first decoded pixel of `path` as RGB.
    fn first_pixel(path: &Path) -> [u8; 3] {
        let out = Command::new(tools().ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-i"])
            .arg(path)
            .args([
                "-frames:v",
                "1",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgb24",
                "pipe:1",
            ])
            .output()
            .unwrap();
        [out.stdout[0], out.stdout[1], out.stdout[2]]
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
    fn locate_prefers_install_dirs_in_order() {
        let tempdir = || tempfile::tempdir().unwrap();
        let (app, local, path) = (tempdir(), tempdir(), tempdir());
        let fill = |d: &Path, names: &[&str]| {
            std::fs::create_dir_all(d).unwrap();
            for n in names {
                std::fs::write(d.join(n), b"").unwrap();
            }
        };
        let both = ["ffmpeg.exe", "ffprobe.exe"];
        fill(path.path(), &both);
        let path_var = std::env::join_paths([path.path()]).unwrap();
        let dirs = [app.path().join("ffmpeg"), local.path().join("ffmpeg")];
        assert_eq!(locate_in(&dirs, None), None);
        assert_eq!(
            locate_in(&dirs, Some(&path_var)).unwrap().ffmpeg,
            path.path().join("ffmpeg.exe")
        );
        fill(&dirs[1], &both);
        assert_eq!(
            locate_in(&dirs, Some(&path_var)).unwrap().ffmpeg,
            dirs[1].join("ffmpeg.exe")
        );
        fill(&dirs[0], &["ffmpeg.exe"]); // no ffprobe: skipped
        assert_eq!(
            locate_in(&dirs, Some(&path_var)).unwrap().ffmpeg,
            dirs[1].join("ffmpeg.exe")
        );
        fill(&dirs[0], &both);
        assert_eq!(
            locate_in(&dirs, Some(&path_var)).unwrap(),
            Tools {
                ffmpeg: dirs[0].join("ffmpeg.exe"),
                ffprobe: dirs[0].join("ffprobe.exe")
            }
        );
    }

    #[test]
    fn install_dirs_are_app_then_local() {
        let dirs = install_dirs();
        let exe = std::env::current_exe().unwrap();
        assert_eq!(dirs[0], exe.parent().unwrap().join("ffmpeg"));
        let local = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap());
        assert_eq!(dirs[1], local.join("SwissWheel").join("ffmpeg"));
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
    fn resolution_limits_the_short_side() {
        // A portrait phone clip at "480p" keeps 480 across, not 480 tall.
        let dir = tempfile::tempdir().unwrap();
        let tall = pattern(dir.path(), "tall.mp4", "720x1280", 1);
        let out = convert(
            Operation::Convert(Format::Mkv),
            Some(Preset::Resolution(Some(480))),
            &tall,
            "tall.mkv",
        );
        let (w, h) = dims(&out);
        assert_eq!(w, 480);
        assert!(h % 2 == 0 && (850..=856).contains(&h), "{w}x{h}");
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
            input_args: Vec::new(),
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
    fn cancel_reaches_ffmpeg_behind_a_launcher() {
        // Package managers (Chocolatey) put a launcher on PATH that starts the
        // real FFmpeg as its child. Killing only the launcher leaves FFmpeg
        // encoding and holding the pipes, so quitting would wait for it.
        let dir = tempfile::tempdir().unwrap();
        let long = make(
            dir.path(),
            "long.mp4",
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=60:size=320x240:rate=10",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
            ],
        );
        let real = tools();
        let launcher = dir.path().join("ffmpeg.cmd");
        // -re reads the input in real time, so FFmpeg is still busy when cancelled.
        std::fs::write(
            &launcher,
            format!("@\"{}\" -re %*\r\n", real.ffmpeg.display()),
        )
        .unwrap();
        let tools = Tools {
            ffmpeg: launcher,
            ffprobe: real.ffprobe,
        };
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
            std::thread::sleep(Duration::from_millis(500));
            flag.store(true, Ordering::SeqCst);
        });
        let out = dir.path().join("long.webm.swisswheel-tmp");
        let started = Instant::now();
        let err = run(&tools, &plan, &long, &out, &|_| {}, &cancel).unwrap_err();
        assert!(matches!(err, EngineError::Cancelled), "{err:?}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "took {:?}",
            started.elapsed()
        );
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
    fn probe_reports_display_size_of_rotated_clip() {
        let dir = tempfile::tempdir().unwrap();
        let clip = rotated_clip(dir.path());
        let info = probe_out(&clip);
        assert_eq!((info.width, info.height), (Some(240), Some(320)));
        assert_eq!(info.rotation % 180, 90);
        assert_eq!(info.fps, Some(10.0));
    }

    #[test]
    fn input_args_go_before_the_input() {
        // An input seek of 1.5 s into a 2 s clip leaves about half a second.
        let dir = tempfile::tempdir().unwrap();
        let clip = make(dir.path(), "clip.mp4", CLIP);
        let plan = MediaPlan {
            output: Format::Mp4,
            input_args: ["-ss", "1.500"].map(String::from).to_vec(),
            attempts: vec![
                ["-c:v", "libx264", "-c:a", "aac", "-f", "mp4"]
                    .map(String::from)
                    .to_vec(),
            ],
            duration_secs: Some(0.5),
        };
        let out = dir.path().join("o.mp4");
        run(
            &tools(),
            &plan,
            &clip,
            &out,
            &|_| {},
            &AtomicBool::new(false),
        )
        .unwrap();
        let secs = probe_out(&out).duration_secs.unwrap();
        assert!((secs - 0.5).abs() < 0.15, "{secs}");
    }

    #[test]
    fn removed_pieces_shorten_the_clip() {
        let dir = tempfile::tempdir().unwrap();
        let clip = tone_clip(dir.path(), "tone.mp4", 6);
        let out = dir.path().join("o.mp4");
        run_video_edit(&clip, &edit(vec![(0.0, 2.0), (4.0, 6.0)]), &out);
        near(secs(&out), 4.0);
        assert_eq!(probe_out(&out).audio.as_deref(), Some("aac"));
    }

    #[test]
    fn crop_then_rotate_sizes() {
        let dir = tempfile::tempdir().unwrap();
        let clip = tone_clip(dir.path(), "tone.mp4", 1);
        let mut plan = edit(vec![(0.0, 1.0)]);
        plan.frame = vec![
            EditOp::Crop(PixelRect {
                x: 0,
                y: 0,
                w: 200,
                h: 100,
            }),
            EditOp::Rotate(Turn::Cw),
        ];
        let out = dir.path().join("o.mp4");
        run_video_edit(&clip, &plan, &out);
        assert_eq!(dims(&out), (100, 200));
    }

    #[test]
    fn odd_crop_encodes() {
        let dir = tempfile::tempdir().unwrap();
        let clip = tone_clip(dir.path(), "tone.mp4", 1);
        let mut plan = edit(vec![(0.0, 1.0)]);
        plan.frame = vec![EditOp::Crop(PixelRect {
            x: 1,
            y: 1,
            w: 101,
            h: 57,
        })];
        let out = dir.path().join("o.mp4");
        run_video_edit(&clip, &plan, &out);
        assert_eq!(dims(&out), (100, 56));
    }

    #[test]
    fn double_speed_halves_duration() {
        let dir = tempfile::tempdir().unwrap();
        let four = tone_clip(dir.path(), "four.mp4", 4);
        let mut plan = edit(vec![(0.0, 4.0)]);
        plan.speed = Speed::Double;
        let fast = dir.path().join("fast.mp4");
        run_video_edit(&four, &plan, &fast);
        near(secs(&fast), 2.0);
        let two = tone_clip(dir.path(), "two.mp4", 2);
        let mut plan = edit(vec![(0.0, 2.0)]);
        plan.speed = Speed::Half;
        let slow = dir.path().join("slow.mp4");
        run_video_edit(&two, &plan, &slow);
        near(secs(&slow), 4.0);
    }

    #[test]
    fn mute_drops_audio_and_normalize_keeps_rate() {
        let dir = tempfile::tempdir().unwrap();
        let clip = tone_clip(dir.path(), "tone.mp4", 2);
        let mut plan = edit(vec![(0.0, 2.0)]);
        plan.audio = AudioMode::Mute;
        let muted = dir.path().join("muted.mp4");
        run_video_edit(&clip, &plan, &muted);
        assert_eq!(probe_out(&muted).audio, None);
        plan.audio = AudioMode::Normalize;
        let loud = dir.path().join("loud.mp4");
        run_video_edit(&clip, &plan, &loud);
        assert_eq!(probe_out(&loud).sample_rate, Some(44100));
    }

    #[test]
    fn webm_normalize_saves() {
        let dir = tempfile::tempdir().unwrap();
        let tone = tone_clip(dir.path(), "tone.mp4", 2);
        let tone = tone.to_str().unwrap();
        let webm = make(
            dir.path(),
            "tone.webm",
            &[
                "-i",
                tone,
                "-c:v",
                "libvpx-vp9",
                "-deadline",
                "realtime",
                "-c:a",
                "libopus",
            ],
        );
        let mut plan = edit(vec![(0.0, 2.0)]);
        plan.audio = AudioMode::Normalize;
        let out = dir.path().join("o.webm");
        run_video_edit(&webm, &plan, &out);
        assert_eq!(probe_out(&out).sample_rate, Some(48000));
    }

    #[test]
    fn reverse_starts_with_the_last_frame() {
        let dir = tempfile::tempdir().unwrap();
        let clip = color_steps(dir.path(), "steps.mp4");
        assert!(first_pixel(&clip)[0] > 200, "the fixture starts red");
        let mut plan = edit(vec![(0.0, 2.0)]);
        plan.reverse = true;
        let out = dir.path().join("o.mp4");
        run_video_edit(&clip, &plan, &out);
        let [r, _, b] = first_pixel(&out);
        assert!(b > r, "expected blue first, got r={r} b={b}");
    }

    #[test]
    fn rotated_input_crops_in_display_space() {
        let dir = tempfile::tempdir().unwrap();
        let clip = rotated_clip(dir.path()); // shown 240×320
        let mut plan = edit(vec![(0.0, 1.0)]);
        plan.frame = vec![EditOp::Crop(PixelRect {
            x: 0,
            y: 0,
            w: 240,
            h: 100,
        })];
        let out = dir.path().join("o.mp4");
        run_video_edit(&clip, &plan, &out);
        let info = probe_out(&out);
        assert_eq!((info.width, info.height), (Some(240), Some(100)));
    }

    #[test]
    fn frame_png_has_the_edited_size() {
        let dir = tempfile::tempdir().unwrap();
        let clip = tone_clip(dir.path(), "tone.mp4", 1);
        let plan = media::video_frame_plan(0.5, &[EditOp::Rotate(Turn::Cw)]);
        let out = dir.path().join("frame.png");
        run(
            &tools(),
            &plan,
            &clip,
            &out,
            &|_| {},
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(::image::image_dimensions(&out).unwrap(), (240, 320));
    }

    #[test]
    fn every_container_saves() {
        let dir = tempfile::tempdir().unwrap();
        let tone = tone_clip(dir.path(), "tone.mp4", 1);
        let tone = tone.to_str().unwrap();
        for (name, codecs) in [
            ("t.webm", ["-c:v", "libvpx-vp9", "-c:a", "libopus"]),
            ("t.avi", ["-c:v", "mpeg4", "-c:a", "libmp3lame"]),
            ("t.wmv", ["-c:v", "wmv2", "-c:a", "wmav2"]),
            ("t.mov", ["-c:v", "libx264", "-c:a", "aac"]),
            ("t.mkv", ["-c:v", "libx264", "-c:a", "aac"]),
        ] {
            let mut args = vec!["-i", tone];
            args.extend(codecs);
            let input = make(dir.path(), name, &args);
            let mut plan = edit(vec![(0.0, 1.0)]);
            plan.speed = Speed::OneHalf;
            let out = dir.path().join(format!("out-{name}"));
            let media = run_video_edit(&input, &plan, &out);
            let expected = if name.ends_with(".wmv") {
                Format::Mp4
            } else {
                Format::from_path(&input).unwrap()
            };
            assert_eq!(media.output, expected, "{name}");
            near(secs(&out), 1.0 / 1.5);
        }
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
