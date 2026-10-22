//! What the download panel shows (spec 3.2): where FFmpeg is, the version
//! it reports, and the newest release. Runs on its own thread; never shows
//! a dialog.

use std::ffi::OsStr;
use std::io::{BufRead, BufReader};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use swisswheel_core::essentials::{Installed, parse_ffmpeg_version};
use swisswheel_core::ffmpeg_release::parse_release;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::engines::ffmpeg::{install_dirs, locate_in};
use crate::update::http;

const JSON_LIMIT: usize = 1 << 20;
/// How long `ffmpeg -version` may take; a hung exe is given up on.
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub installed: Installed,
    /// The folder of the app's own copy, if there is one.
    pub own_dir: Option<PathBuf>,
    /// An FFmpeg on `PATH`, whether or not the app's own copy wins.
    pub system: bool,
    /// The newest release's tag; `None` when it wasn't asked for.
    pub latest: Option<Result<String, ()>>,
}

/// Looks in `dirs` (the app's own folders), then `path_var`; asks
/// `endpoint` for the newest release when given. Blocks.
pub fn check(
    dirs: &[PathBuf],
    path_var: Option<&OsStr>,
    endpoint: Option<&str>,
    read_version: &dyn Fn(&Path) -> Option<String>,
) -> Report {
    let own = locate_in(dirs, None);
    let system = locate_in(&[], path_var);
    let installed = match (&own, &system) {
        (Some(own), _) => Installed::Own(read_version(&own.ffmpeg)),
        (None, Some(system)) => Installed::System(read_version(&system.ffmpeg)),
        (None, None) => Installed::None,
    };
    Report {
        installed,
        own_dir: own.and_then(|t| Some(t.ffmpeg.parent()?.to_path_buf())),
        system: system.is_some(),
        latest: endpoint.map(latest),
    }
}

fn latest(endpoint: &str) -> Result<String, ()> {
    let body = http::get(endpoint, JSON_LIMIT, &mut |_, _| {})
        .map_err(|e| log_error!("FFmpeg 최신 버전 확인 실패: {e}"))?;
    parse_release(&String::from_utf8_lossy(&body))
        .map(|release| release.tag)
        .map_err(|_| log_error!("FFmpeg: 릴리스 정보를 해석할 수 없습니다"))
}

/// The version `ffmpeg -version` reports on its first line. The line is
/// read on its own thread while this one waits: the rest of the output may
/// not fit the pipe, and a hung exe must not hold the check.
pub fn read_version(ffmpeg: &Path) -> Option<String> {
    let mut child = Command::new(ffmpeg)
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = BufReader::new(stdout).read_line(&mut line);
        let _ = tx.send(line);
    });
    let line = rx.recv_timeout(VERSION_TIMEOUT);
    // Either way it is done with: the line is in, or it hung.
    let _ = child.kill();
    let _ = child.wait();
    match line {
        Ok(line) => parse_ffmpeg_version(line.trim_end()),
        Err(_) => {
            log_error!("FFmpeg 버전을 읽지 못했습니다: {}", ffmpeg.display());
            None
        }
    }
}

/// Checks on a new thread (the app's folders, `PATH`, and, if
/// `fetch_latest`, the releases endpoint), stores the report in `slot`, and
/// posts `msg` to `hwnd`.
pub fn spawn(hwnd: isize, msg: u32, slot: Arc<Mutex<Option<Report>>>, fetch_latest: bool) {
    std::thread::spawn(move || {
        let endpoint = fetch_latest.then(crate::ffmpeg_install::endpoint);
        let path = std::env::var_os("PATH");
        let report = check(
            &install_dirs(),
            path.as_deref(),
            endpoint.as_deref(),
            &read_version,
        );
        *slot.lock().unwrap() = Some(report);
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as *mut _)), msg, WPARAM(0), LPARAM(0));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::test_server::{Reply, TestServer};
    use std::fs;

    fn tools_in(dir: &Path) {
        fs::create_dir_all(dir).unwrap();
        for f in ["ffmpeg.exe", "ffprobe.exe"] {
            fs::write(dir.join(f), b"").unwrap();
        }
    }

    fn ver(v: &'static str) -> impl Fn(&Path) -> Option<String> {
        move |_| Some(v.into())
    }

    #[test]
    fn own_copy_wins_over_path() {
        let tmp = tempfile::tempdir().unwrap();
        let (own, sys) = (tmp.path().join("own"), tmp.path().join("sys"));
        tools_in(&own);
        tools_in(&sys);
        let r = check(
            std::slice::from_ref(&own),
            Some(sys.as_os_str()),
            None,
            &ver("9.0.2"),
        );
        assert_eq!(
            r,
            Report {
                installed: Installed::Own(Some("9.0.2".into())),
                own_dir: Some(own),
                system: true,
                latest: None,
            }
        );
    }

    #[test]
    fn system_only_and_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let sys = tmp.path().join("sys");
        tools_in(&sys);
        let r = check(
            &[tmp.path().join("own")],
            Some(sys.as_os_str()),
            None,
            &|_| None,
        );
        assert_eq!(
            (r.installed, r.own_dir, r.system),
            (Installed::System(None), None, true)
        );
        let r = check(&[tmp.path().join("own")], None, None, &ver("x"));
        assert_eq!((r.installed, r.system), (Installed::None, false));
    }

    #[test]
    fn latest_from_the_release_endpoint() {
        let json = br#"{"tag_name":"9.0.3","assets":[{"name":"ffmpeg-9.0.3-essentials_build.zip","size":1,"browser_download_url":"http://x/z.zip","digest":"sha256:0000000000000000000000000000000000000000000000000000000000000000"}]}"#;
        let server = TestServer::start(vec![
            ("/latest", Reply::Ok(json.to_vec())),
            ("/bad", Reply::Status(500)),
        ]);
        let url = |p: &str| format!("{}{p}", server.base);
        let r = check(&[], None, Some(&url("/latest")), &|_| None);
        assert_eq!(r.latest, Some(Ok("9.0.3".into())));
        let r = check(&[], None, Some(&url("/bad")), &|_| None);
        assert_eq!(r.latest, Some(Err(())));
    }

    #[test]
    fn reading_a_version_from_a_missing_exe_is_none() {
        assert_eq!(read_version(Path::new(r"C:\없음\ffmpeg.exe")), None);
    }

    #[test]
    fn reads_the_version_a_real_ffmpeg_prints() {
        let Some(tools) = crate::engines::ffmpeg::locate() else {
            return; // the media tests need FFmpeg; this one only checks it
        };
        let out = Command::new(&tools.ffmpeg)
            .arg("-version")
            .output()
            .unwrap();
        let first = String::from_utf8_lossy(&out.stdout);
        let expected = parse_ffmpeg_version(first.lines().next().unwrap_or(""));
        assert_eq!(read_version(&tools.ffmpeg), expected);
    }
}
