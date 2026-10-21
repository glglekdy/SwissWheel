//! Runs jobs (one operation over the dropped files) on two worker pools:
//! one for in-process image work and a smaller one for FFmpeg (spec 4.4).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use swisswheel_core::catalog::Operation;
use swisswheel_core::config::Quality;
use swisswheel_core::format::Format;
use swisswheel_core::media;
use swisswheel_core::naming::{suffix_for, temp_path};
use swisswheel_core::options::Preset;

use crate::engines::ffmpeg::{self, Tools};
use crate::engines::{self, EngineError};
use crate::fsutil::{Reservations, finalize};

#[derive(Debug, Clone)]
pub struct Job {
    pub op: Operation,
    /// The option chosen in the side panel; `None` means the defaults.
    pub preset: Option<Preset>,
    pub inputs: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct JobReport {
    pub op: Operation,
    /// Successful outputs, in input order.
    pub outputs: Vec<PathBuf>,
    /// One user-facing message per failed input, in input order.
    pub failures: Vec<String>,
}

type OnDone = Box<dyn Fn(JobReport) + Send + Sync>;

struct Shared {
    quality: Quality,
    reservations: Reservations,
    on_done: OnDone,
    /// Found once at startup; media files fail with `MissingFfmpeg` without it.
    tools: Option<Tools>,
    /// Set by `shutdown`: running FFmpeg is killed, queued files are skipped.
    cancel: AtomicBool,
    /// Jobs submitted but not yet reported, for `Runner::progress`.
    active: Mutex<Vec<Arc<JobState>>>,
}

/// One job's progress; the worker that finishes the last file reports it.
struct JobState {
    op: Operation,
    preset: Option<Preset>,
    results: Mutex<Vec<Option<Result<PathBuf, String>>>>,
    /// Per file, for `Runner::progress` and `Runner::rows`.
    files: Mutex<Vec<FileState>>,
    names: Vec<String>,
    remaining: AtomicUsize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum FileState {
    Waiting,
    Running(f32),
    Done,
}

impl FileState {
    fn fraction(self) -> f32 {
        match self {
            FileState::Waiting => 0.0,
            FileState::Running(p) => p,
            FileState::Done => 1.0,
        }
    }
}

/// A file of a job that hasn't finished, for the progress window.
#[derive(Debug, Clone, PartialEq)]
pub struct ProgressRow {
    pub file: String,
    pub op: Operation,
    /// `None` while the file waits for a worker.
    pub fraction: Option<f32>,
}

struct Work {
    job: Arc<JobState>,
    index: usize,
    input: PathBuf,
}

pub struct Runner {
    image_tx: Sender<Work>,
    media_tx: Sender<Work>,
    workers: Vec<JoinHandle<()>>,
    shared: Arc<Shared>,
}

impl Runner {
    pub fn new(
        image_workers: usize,
        media_workers: usize,
        quality: Quality,
        on_done: OnDone,
    ) -> Runner {
        let tools = ffmpeg::locate();
        if tools.is_none() {
            log_info!("FFmpeg를 찾지 못했습니다: 영상·오디오 작업은 실패로 처리됩니다");
        }
        let shared = Arc::new(Shared {
            quality,
            reservations: Reservations::default(),
            on_done,
            tools,
            cancel: AtomicBool::new(false),
            active: Mutex::new(Vec::new()),
        });
        let mut workers = Vec::new();
        let mut pool = |count: usize| {
            let (tx, rx) = mpsc::channel::<Work>();
            let rx = Arc::new(Mutex::new(rx));
            for _ in 0..count.max(1) {
                let (rx, shared) = (Arc::clone(&rx), Arc::clone(&shared));
                workers.push(std::thread::spawn(move || worker(&rx, &shared)));
            }
            tx
        };
        let image_tx = pool(image_workers);
        let media_tx = pool(media_workers);
        Runner {
            image_tx,
            media_tx,
            workers,
            shared,
        }
    }

    pub fn submit(&self, job: Job) {
        if job.inputs.is_empty() {
            (self.shared.on_done)(JobReport {
                op: job.op,
                outputs: Vec::new(),
                failures: Vec::new(),
            });
            return;
        }
        let n = job.inputs.len();
        let state = Arc::new(JobState {
            op: job.op,
            preset: job.preset,
            results: Mutex::new(vec![None; n]),
            files: Mutex::new(vec![FileState::Waiting; n]),
            names: job.inputs.iter().map(|p| file_name(p)).collect(),
            remaining: AtomicUsize::new(n),
        });
        self.shared.active.lock().unwrap().push(Arc::clone(&state));
        for (index, input) in job.inputs.into_iter().enumerate() {
            let is_media = Format::from_path(&input).is_some_and(|f| media::supports(&job.op, f));
            let tx = if is_media {
                &self.media_tx
            } else {
                &self.image_tx
            };
            // Send only fails once every worker has exited, which shutdown() prevents.
            let _ = tx.send(Work {
                job: Arc::clone(&state),
                index,
                input,
            });
        }
    }

    /// Average progress over every file of the jobs not yet reported, or
    /// `None` when nothing is running.
    pub fn progress(&self) -> Option<f32> {
        let active = self.shared.active.lock().unwrap();
        let (mut sum, mut count) = (0.0, 0usize);
        for job in active.iter() {
            let files = job.files.lock().unwrap();
            sum += files.iter().map(|f| f.fraction()).sum::<f32>();
            count += files.len();
        }
        (count > 0).then(|| sum / count as f32)
    }

    /// Every unfinished file of the jobs not yet reported, in submission order.
    pub fn rows(&self) -> Vec<ProgressRow> {
        let active = self.shared.active.lock().unwrap();
        let mut rows = Vec::new();
        for job in active.iter() {
            let files = job.files.lock().unwrap();
            for (name, state) in job.names.iter().zip(files.iter()) {
                let fraction = match *state {
                    FileState::Waiting => None,
                    FileState::Running(p) => Some(p),
                    FileState::Done => continue,
                };
                rows.push(ProgressRow {
                    file: name.clone(),
                    op: job.op,
                    fraction,
                });
            }
        }
        rows
    }

    /// Kills running FFmpeg work, skips queued files, then stops the workers.
    /// Image files already being converted finish (they take moments).
    pub fn shutdown(self) {
        self.shared.cancel.store(true, Ordering::SeqCst);
        drop(self.image_tx);
        drop(self.media_tx);
        for w in self.workers {
            let _ = w.join();
        }
    }
}

fn worker(rx: &Mutex<Receiver<Work>>, shared: &Shared) {
    loop {
        let Ok(work) = rx.lock().unwrap().recv() else {
            return;
        };
        let result = if shared.cancel.load(Ordering::SeqCst) {
            Err(EngineError::Cancelled.user_message(&file_name(&work.input)))
        } else {
            work.job.files.lock().unwrap()[work.index] = FileState::Running(0.0);
            process(&work, shared)
        };
        work.job.results.lock().unwrap()[work.index] = Some(result);
        work.job.files.lock().unwrap()[work.index] = FileState::Done;
        if work.job.remaining.fetch_sub(1, Ordering::AcqRel) == 1 {
            shared
                .active
                .lock()
                .unwrap()
                .retain(|j| !Arc::ptr_eq(j, &work.job));
            (shared.on_done)(report(&work.job));
        }
    }
}

fn report(job: &JobState) -> JobReport {
    let mut outputs = Vec::new();
    let mut failures = Vec::new();
    for result in job.results.lock().unwrap().drain(..).flatten() {
        match result {
            Ok(path) => outputs.push(path),
            Err(msg) => failures.push(msg),
        }
    }
    JobReport {
        op: job.op,
        outputs,
        failures,
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Which engine converts a file, decided before its output name is reserved
/// (for media the output format depends on what ffprobe finds).
enum Engine<'a> {
    Image,
    Media(&'a Tools, media::MediaPlan),
}

/// Converts one file: reserve a name, write to a temp file, then move it into
/// place without replacing anything. The temp file never survives a failure.
fn process(work: &Work, shared: &Shared) -> Result<PathBuf, String> {
    let (op, preset, input) = (&work.job.op, work.job.preset, work.input.as_path());
    let name = file_name(input);
    let fail = |e: EngineError| {
        log_error!("{name}: {e}");
        e.user_message(&name)
    };
    let format =
        Format::from_path(input).ok_or_else(|| fail(EngineError::Unsupported(name.clone())))?;
    let (engine, extension) = if engines::image::supports(op, format) {
        (
            Engine::Image,
            engines::image::output_format(op, format).extension(),
        )
    } else if media::supports(op, format) {
        let tools = shared
            .tools
            .as_ref()
            .ok_or_else(|| fail(EngineError::MissingFfmpeg))?;
        let plan = ffmpeg::prepare(tools, op, preset, input, &shared.quality).map_err(fail)?;
        let extension = plan.output.extension();
        (Engine::Media(tools, plan), extension)
    } else {
        return Err(fail(EngineError::Unsupported(format!("{op:?} on {name}"))));
    };
    let suffix = suffix_for(op, preset);

    let mut target = shared
        .reservations
        .reserve(input, extension, suffix.as_deref());
    let temp = temp_path(&target);
    let result = match &engine {
        Engine::Image => engines::image::run(op, preset, input, &temp, &shared.quality),
        Engine::Media(tools, plan) => {
            let progress =
                |p: f32| work.job.files.lock().unwrap()[work.index] = FileState::Running(p);
            ffmpeg::run(tools, plan, input, &temp, &progress, &shared.cancel)
        }
    };
    if let Err(e) = result {
        let _ = std::fs::remove_file(&temp);
        shared.reservations.release(&target);
        return Err(fail(e));
    }
    // Another program may have created our reserved name meanwhile; pick a new one.
    for _ in 0..3 {
        match finalize(&temp, &target) {
            Ok(()) => {
                shared.reservations.release(&target);
                return Ok(target);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                shared.reservations.release(&target);
                target = shared
                    .reservations
                    .reserve(input, extension, suffix.as_deref());
            }
            Err(_) => break,
        }
    }
    let _ = std::fs::remove_file(&temp);
    shared.reservations.release(&target);
    Err(fail(EngineError::Write(target.display().to_string())))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::ffmpeg::tests::{CLIP, make, pattern};
    use crate::engines::image::tests::rgba_png;
    use std::sync::Mutex;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    use swisswheel_core::catalog::{Operation, Tool};
    use swisswheel_core::config::Quality;
    use swisswheel_core::format::Format;
    use swisswheel_core::options::Preset;

    /// A 2+2-worker runner whose reports arrive on the returned channel.
    fn runner() -> (Runner, mpsc::Receiver<JobReport>) {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let runner = Runner::new(
            2,
            2,
            Quality::default(),
            Box::new(move |r| tx.lock().unwrap().send(r).unwrap()),
        );
        (runner, rx)
    }

    /// Submits `jobs` and collects one report per job.
    fn run_jobs(jobs: Vec<Job>) -> Vec<JobReport> {
        let (runner, rx) = runner();
        let n = jobs.len();
        for job in jobs {
            runner.submit(job);
        }
        let reports = rx.iter().take(n).collect();
        runner.shutdown();
        reports
    }

    fn job(op: Operation, inputs: Vec<PathBuf>) -> Job {
        Job {
            op,
            preset: None,
            inputs,
        }
    }

    #[test]
    fn batch_convert() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.png"), dir.path().join("b.png"));
        rgba_png(&a, 10, 10, 255);
        rgba_png(&b, 10, 10, 255);
        let reports = run_jobs(vec![job(Operation::Convert(Format::Jpg), vec![a, b])]);
        assert_eq!(reports.len(), 1);
        assert_eq!(
            reports[0].outputs,
            vec![dir.path().join("a.jpg"), dir.path().join("b.jpg")]
        );
        assert!(reports[0].failures.is_empty());
    }

    #[test]
    fn concurrent_jobs_get_distinct_outputs() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.png");
        rgba_png(&a, 600, 600, 255);
        let one = || job(Operation::Convert(Format::Jpg), vec![a.clone()]);
        let mut outs: Vec<_> = run_jobs(vec![one(), one()])
            .into_iter()
            .flat_map(|r| r.outputs)
            .collect();
        outs.sort();
        assert_eq!(
            outs,
            vec![dir.path().join("a (1).jpg"), dir.path().join("a.jpg")]
        );
    }

    #[test]
    fn bad_input_fails_alone_and_leaves_no_temp() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("good.png");
        rgba_png(&good, 10, 10, 255);
        let empty = dir.path().join("empty.png");
        std::fs::write(&empty, b"").unwrap();
        let r = &run_jobs(vec![job(
            Operation::Convert(Format::Webp),
            vec![empty, good],
        )])[0];
        assert_eq!(r.outputs, vec![dir.path().join("good.webp")]);
        assert_eq!(
            r.failures,
            vec!["empty.png: 이미지를 읽을 수 없습니다".to_string()]
        );
        assert!(temp_files(dir.path()).is_empty());
    }

    #[test]
    fn batch_in_unicode_folder() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("가족 앨범");
        std::fs::create_dir(&sub).unwrap();
        let p = sub.join("할머니 생신.png");
        rgba_png(&p, 10, 10, 255);
        let r = &run_jobs(vec![job(Operation::Tool(Tool::Shrink), vec![p])])[0];
        assert_eq!(r.outputs, vec![sub.join("할머니 생신 (50%).png")]);
    }

    #[test]
    fn unsupported_operations_report_per_file() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path().join("clip.mp4");
        std::fs::write(&v, b"x").unwrap();
        let r = &run_jobs(vec![job(Operation::Convert(Format::Png), vec![v])])[0];
        assert!(r.outputs.is_empty());
        assert_eq!(
            r.failures,
            vec!["clip.mp4: 지원하지 않는 작업입니다".to_string()]
        );
    }

    #[test]
    fn media_job_converts_and_reports() {
        let dir = tempfile::tempdir().unwrap();
        let clip = make(dir.path(), "clip.mp4", CLIP);
        let r = &run_jobs(vec![job(Operation::Convert(Format::Mp3), vec![clip])])[0];
        assert_eq!(r.outputs, vec![dir.path().join("clip.mp3")]);
        assert!(r.failures.is_empty(), "{:?}", r.failures);
    }

    #[test]
    fn media_output_extension_comes_from_probe() {
        let dir = tempfile::tempdir().unwrap();
        let clip = make(dir.path(), "clip.mp4", CLIP); // AAC audio
        let r = &run_jobs(vec![job(Operation::Tool(Tool::ExtractAudio), vec![clip])])[0];
        assert_eq!(r.outputs, vec![dir.path().join("clip.m4a")]);
    }

    #[test]
    fn option_suffixes_reach_file_names() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("a.png");
        rgba_png(&png, 40, 40, 255);
        let clip = make(dir.path(), "clip.mp4", CLIP);
        let reports = run_jobs(vec![
            Job {
                op: Operation::Tool(Tool::Shrink),
                preset: Some(Preset::Scale(25)),
                inputs: vec![png],
            },
            Job {
                op: Operation::Convert(Format::Mkv),
                preset: Some(Preset::Resolution(Some(480))),
                inputs: vec![clip],
            },
        ]);
        let mut outs: Vec<_> = reports.into_iter().flat_map(|r| r.outputs).collect();
        outs.sort();
        assert_eq!(
            outs,
            vec![
                dir.path().join("a (25%).png"),
                dir.path().join("clip (480p).mkv")
            ]
        );
    }

    #[test]
    fn progress_while_running_and_none_when_idle() {
        let dir = tempfile::tempdir().unwrap();
        let clip = pattern(dir.path(), "clip.mp4", "320x240", 10);
        let (runner, rx) = runner();
        assert_eq!(runner.progress(), None);
        runner.submit(job(Operation::Convert(Format::Webm), vec![clip]));
        let mut seen = false;
        let report = loop {
            if let Some(p) = runner.progress() {
                assert!((0.0..=1.0).contains(&p), "{p}");
                seen = true;
            }
            if let Ok(r) = rx.recv_timeout(Duration::from_millis(20)) {
                break r;
            }
        };
        assert!(seen, "progress was never reported");
        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert_eq!(runner.progress(), None);
        runner.shutdown();
    }

    #[test]
    fn rows_list_running_and_waiting_files() {
        let dir = tempfile::tempdir().unwrap();
        let clips: Vec<_> = ["a.mp4", "b.mp4", "c.mp4"]
            .iter()
            .map(|n| pattern(dir.path(), n, "320x240", 10))
            .collect();
        let (runner, rx) = runner();
        assert!(runner.rows().is_empty());
        runner.submit(job(Operation::Convert(Format::Webm), clips));
        // Two media workers: two files run while the third waits.
        let (mut saw_running, mut saw_waiting) = (false, false);
        let report = loop {
            for row in runner.rows() {
                assert_eq!(row.op, Operation::Convert(Format::Webm));
                assert!(["a.mp4", "b.mp4", "c.mp4"].contains(&row.file.as_str()));
                match row.fraction {
                    Some(p) => {
                        assert!((0.0..=1.0).contains(&p));
                        saw_running = true;
                    }
                    None => saw_waiting = true,
                }
            }
            if let Ok(r) = rx.recv_timeout(Duration::from_millis(10)) {
                break r;
            }
        };
        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert!(saw_running && saw_waiting);
        assert!(runner.rows().is_empty());
        runner.shutdown();
    }

    #[test]
    fn shutdown_cancels_running_media() {
        let dir = tempfile::tempdir().unwrap();
        let clip = pattern(dir.path(), "clip.mp4", "640x480", 30);
        let (runner, _rx) = runner();
        runner.submit(job(Operation::Convert(Format::Webm), vec![clip]));
        std::thread::sleep(Duration::from_millis(500));
        let started = Instant::now();
        runner.shutdown();
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(!dir.path().join("clip.webm").exists());
        assert!(temp_files(dir.path()).is_empty());
    }

    fn temp_files(dir: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.to_string_lossy().ends_with(".swisswheel-tmp"))
            .collect()
    }
}
