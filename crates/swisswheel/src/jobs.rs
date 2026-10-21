//! Runs jobs (one operation over the dropped files) on a small worker pool.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use swisswheel_core::catalog::Operation;
use swisswheel_core::config::Quality;
use swisswheel_core::format::Format;
use swisswheel_core::naming::temp_path;

use crate::engines::{self, EngineError};
use crate::fsutil::{Reservations, finalize};

#[derive(Debug, Clone)]
pub struct Job {
    pub op: Operation,
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
}

/// One job's progress; the worker that finishes the last file reports it.
struct JobState {
    op: Operation,
    results: Mutex<Vec<Option<Result<PathBuf, String>>>>,
    remaining: AtomicUsize,
}

struct Work {
    job: Arc<JobState>,
    index: usize,
    input: PathBuf,
}

pub struct Runner {
    tx: Sender<Work>,
    workers: Vec<JoinHandle<()>>,
    shared: Arc<Shared>,
}

impl Runner {
    pub fn new(workers: usize, quality: Quality, on_done: OnDone) -> Runner {
        let (tx, rx) = mpsc::channel::<Work>();
        let rx = Arc::new(Mutex::new(rx));
        let shared = Arc::new(Shared {
            quality,
            reservations: Reservations::default(),
            on_done,
        });
        let workers = (0..workers.max(1))
            .map(|_| {
                let (rx, shared) = (Arc::clone(&rx), Arc::clone(&shared));
                std::thread::spawn(move || worker(&rx, &shared))
            })
            .collect();
        Runner {
            tx,
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
        let state = Arc::new(JobState {
            op: job.op,
            results: Mutex::new(vec![None; job.inputs.len()]),
            remaining: AtomicUsize::new(job.inputs.len()),
        });
        for (index, input) in job.inputs.into_iter().enumerate() {
            // Send only fails once every worker has exited, which shutdown() prevents.
            let _ = self.tx.send(Work {
                job: Arc::clone(&state),
                index,
                input,
            });
        }
    }

    /// Finishes queued work, then stops the workers.
    pub fn shutdown(self) {
        drop(self.tx);
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
        let result = process(&work.job.op, &work.input, shared);
        work.job.results.lock().unwrap()[work.index] = Some(result);
        if work.job.remaining.fetch_sub(1, Ordering::AcqRel) == 1 {
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

/// Converts one file: reserve a name, write to a temp file, then move it into
/// place without replacing anything. The temp file never survives a failure.
fn process(op: &Operation, input: &Path, shared: &Shared) -> Result<PathBuf, String> {
    let name = input
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let format = Format::from_path(input)
        .ok_or_else(|| EngineError::Unsupported(name.clone()).user_message(&name))?;
    if !engines::image::supports(op, format) {
        return Err(format!("{name}: 다음 버전에서 지원 예정입니다"));
    }
    let extension = engines::image::output_format(op, format).extension();
    let suffix = match op {
        Operation::Tool(tool) => tool.suffix(),
        _ => None,
    };

    let mut target = shared.reservations.reserve(input, extension, suffix);
    let temp = temp_path(&target);
    if let Err(e) = engines::image::run(op, input, &temp, &shared.quality) {
        let _ = std::fs::remove_file(&temp);
        shared.reservations.release(&target);
        return Err(e.user_message(&name));
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
                target = shared.reservations.reserve(input, extension, suffix);
            }
            Err(_) => break,
        }
    }
    let _ = std::fs::remove_file(&temp);
    shared.reservations.release(&target);
    Err(EngineError::Write(target.display().to_string()).user_message(&name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::image::tests::rgba_png;
    use std::sync::Mutex;
    use std::sync::mpsc;
    use swisswheel_core::catalog::{Operation, Tool};
    use swisswheel_core::config::Quality;
    use swisswheel_core::format::Format;

    /// Submits `jobs` to a 2-worker runner and collects one report per job.
    fn run_jobs(jobs: Vec<Job>) -> Vec<JobReport> {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let runner = Runner::new(
            2,
            Quality::default(),
            Box::new(move |r| tx.lock().unwrap().send(r).unwrap()),
        );
        let n = jobs.len();
        for job in jobs {
            runner.submit(job);
        }
        runner.shutdown();
        rx.iter().take(n).collect()
    }

    #[test]
    fn batch_convert() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.png"), dir.path().join("b.png"));
        rgba_png(&a, 10, 10, 255);
        rgba_png(&b, 10, 10, 255);
        let reports = run_jobs(vec![Job {
            op: Operation::Convert(Format::Jpg),
            inputs: vec![a, b],
        }]);
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
        let job = || Job {
            op: Operation::Convert(Format::Jpg),
            inputs: vec![a.clone()],
        };
        let mut outs: Vec<_> = run_jobs(vec![job(), job()])
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
        let r = &run_jobs(vec![Job {
            op: Operation::Convert(Format::Webp),
            inputs: vec![empty, good],
        }])[0];
        assert_eq!(r.outputs, vec![dir.path().join("good.webp")]);
        assert_eq!(
            r.failures,
            vec!["empty.png: 이미지를 읽을 수 없습니다".to_string()]
        );
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".swisswheel-tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn batch_in_unicode_folder() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("가족 앨범");
        std::fs::create_dir(&sub).unwrap();
        let p = sub.join("할머니 생신.png");
        rgba_png(&p, 10, 10, 255);
        let r = &run_jobs(vec![Job {
            op: Operation::Tool(Tool::Shrink),
            inputs: vec![p],
        }])[0];
        assert_eq!(r.outputs, vec![sub.join("할머니 생신 (50%).png")]);
    }

    #[test]
    fn unsupported_operations_report_per_file() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path().join("clip.mp4");
        std::fs::write(&v, b"x").unwrap();
        let r = &run_jobs(vec![Job {
            op: Operation::Convert(Format::Mp3),
            inputs: vec![v],
        }])[0];
        assert!(r.outputs.is_empty());
        assert_eq!(
            r.failures,
            vec!["clip.mp4: 다음 버전에서 지원 예정입니다".to_string()]
        );
    }
}
