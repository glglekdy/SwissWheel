//! The fallback when Media Foundation can't play a file: single frames
//! decoded by FFmpeg on a worker thread, newest request first.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};

use swisswheel_core::media::scrub_args;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::CREATE_NO_WINDOW;
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use crate::engines::ffmpeg::{RunJob, Tools};

/// Posted to the panel when a frame (or an error) is ready to `take`.
pub(super) const WM_APP_SCRUB: u32 = WM_APP + 52;

pub(super) struct ScrubFrame {
    /// The time asked for; the panel draws whatever came last, tests check it.
    #[allow(dead_code)]
    pub t: f64,
    pub size: (u32, u32),
    /// Top-down rows of BGRA pixels.
    pub bgra: Vec<u8>,
}

#[derive(Default)]
struct State {
    pending: Option<f64>,
    done: Option<Result<ScrubFrame, String>>,
    stop: bool,
}

pub(super) struct Scrubber {
    shared: Arc<(Mutex<State>, Condvar)>,
}

impl Scrubber {
    /// Starts the worker; frames come out `size` pixels, BGRA.
    pub fn new(tools: Tools, input: PathBuf, size: (u32, u32), panel: HWND) -> Scrubber {
        let shared = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let worker = Arc::clone(&shared);
        let panel = panel.0 as isize;
        std::thread::spawn(move || {
            let (lock, wake) = &*worker;
            loop {
                let t = {
                    let mut state = lock.lock().unwrap();
                    loop {
                        if state.stop {
                            return;
                        }
                        if let Some(t) = state.pending.take() {
                            break t;
                        }
                        state = wake.wait(state).unwrap();
                    }
                };
                let result = grab(&tools, &input, t, size).map(|bgra| ScrubFrame { t, size, bgra });
                if let Err(e) = &result {
                    log_error!("장면을 불러오지 못했습니다 ({t:.2} s): {e}");
                }
                let mut state = lock.lock().unwrap();
                if state.stop {
                    return;
                }
                state.done = Some(result);
                drop(state);
                unsafe {
                    let _ = PostMessageW(
                        Some(HWND(panel as *mut _)),
                        WM_APP_SCRUB,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
        });
        Scrubber { shared }
    }

    /// Asks for the frame at `t`, replacing a request not yet started.
    pub fn request(&self, t: f64) {
        let (lock, wake) = &*self.shared;
        lock.lock().unwrap().pending = Some(t);
        wake.notify_one();
    }

    /// The latest finished frame or error, once.
    pub fn take(&self) -> Option<Result<ScrubFrame, String>> {
        self.shared.0.lock().unwrap().done.take()
    }
}

impl Drop for Scrubber {
    /// The worker stops after the frame it is on: one frame takes FFmpeg
    /// moments, and its job object ends FFmpeg with the app in any case.
    fn drop(&mut self) {
        let (lock, wake) = &*self.shared;
        lock.lock().unwrap().stop = true;
        wake.notify_one();
    }
}

/// The frame shown at `t` seconds of `input`, scaled to `size`, as BGRA.
pub(super) fn grab(
    tools: &Tools,
    input: &Path,
    t: f64,
    size: (u32, u32),
) -> Result<Vec<u8>, String> {
    let (before, after) = scrub_args(t, size);
    let child = Command::new(&tools.ffmpeg)
        .args(["-hide_banner", "-nostdin", "-loglevel", "error"])
        .args(&before)
        .arg("-i")
        .arg(input)
        .args(&after)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW.0)
        .spawn()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    let job = RunJob::new();
    if let Some(job) = &job {
        job.assign(&child);
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    // Every frame of the second before `t`; the last one is the frame shown at `t`.
    let want = size.0 as usize * size.1 as usize * 4;
    let mut bytes = out.stdout;
    if !out.status.success() || bytes.len() < want || bytes.len() % want != 0 {
        return Err(format!(
            "{} bytes, frames of {want}: {}",
            bytes.len(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(bytes.split_off(bytes.len() - want))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::ffmpeg::tests::{color_steps, tools};

    #[test]
    fn grabs_a_bgra_frame_of_the_asked_size() {
        let dir = tempfile::tempdir().unwrap();
        let clip = color_steps(dir.path(), "c.mp4");
        let tools = tools();
        let bgra = grab(&tools, &clip, 1.5, (32, 32)).unwrap();
        assert_eq!(bgra.len(), 32 * 32 * 4);
        let (b, r) = (bgra[0], bgra[2]);
        assert!(b > 200 && r < 60, "expected blue at 1.5 s, got b={b} r={r}");
        let red = grab(&tools, &clip, 0.2, (32, 32)).unwrap();
        assert!(red[2] > 200 && red[0] < 60, "expected red at 0.2 s");
        assert!(grab(&tools, &dir.path().join("missing.mp4"), 0.0, (32, 32)).is_err());
    }

    #[test]
    fn grabs_the_frame_shown_at_the_time_even_at_the_end() {
        let dir = tempfile::tempdir().unwrap();
        let clip = color_steps(dir.path(), "c.mp4");
        let tools = tools();
        let at = |t| grab(&tools, &clip, t, (16, 16)).unwrap();
        let shown = at(0.95);
        assert_eq!(shown.len(), 16 * 16 * 4);
        assert!(shown[2] > 200 && shown[0] < 60, "0.95 s still shows red");
        let end = at(2.0);
        assert_eq!(end.len(), 16 * 16 * 4);
        assert!(
            end[0] > 200 && end[2] < 60,
            "the end shows the last (blue) frame"
        );
    }

    #[test]
    fn the_worker_answers_the_newest_request() {
        let dir = tempfile::tempdir().unwrap();
        let clip = color_steps(dir.path(), "c.mp4");
        let scrubber = Scrubber::new(tools(), clip, (16, 16), HWND::default());
        scrubber.request(0.2);
        scrubber.request(1.5);
        let started = std::time::Instant::now();
        let frame = loop {
            if let Some(result) = scrubber.take()
                && let Ok(frame) = result
                && frame.t == 1.5
            {
                break frame;
            }
            assert!(started.elapsed().as_secs() < 10, "no frame for 1.5 s");
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        assert_eq!(frame.size, (16, 16));
        assert!(frame.bgra[0] > 200, "blue");
    }
}
