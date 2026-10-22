//! The in-app updater: check GitHub Releases, download, verify, swap.

pub mod http;
pub mod swap;
#[cfg(test)]
pub(crate) mod test_server;

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use sha2::{Digest as _, Sha256};
use swisswheel_core::update::{
    ENDPOINT, Version, not_writable_body, parse_checksum, parse_release, prompt_body,
};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    IDYES, MB_ICONINFORMATION, MB_OK, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO, MessageBoxW,
    PostMessageW,
};
use windows::core::{HSTRING, w};

const JSON_LIMIT: usize = 1 << 20;
const SHA_LIMIT: usize = 1024;
const EXE_LIMIT: usize = 64 << 20;

#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// Notice `text::up_to_date(current)`.
    UpToDate,
    CheckFailed,
    MissingAssets,
    DownloadFailed,
    Corrupt,
    Declined,
    /// The exe's folder can't be written; the app opens this page.
    NotWritable {
        page_url: String,
    },
    /// `swisswheel.exe.new` is written and verified.
    Ready {
        version: Version,
    },
}

/// The update thread's questions to the user.
pub trait Ui {
    fn confirm(&self, body: &str) -> bool;
    fn inform(&self, body: &str);
}

/// Shared with the UI thread: download fraction (0..=10000) and, once
/// confirmed, the new version (the progress row's name).
#[derive(Default)]
pub struct Progress {
    pub fraction: AtomicU32,
    pub version: Mutex<Option<Version>>,
}

/// The whole check → confirm → download → verify flow; blocks. Nothing is
/// written next to `exe` until the download matches its checksum.
pub fn run(
    endpoint: &str,
    current: Version,
    exe: &Path,
    ui: &dyn Ui,
    progress: &Progress,
) -> Outcome {
    log_info!("업데이트 확인: {endpoint}");
    let release = match http::get(endpoint, JSON_LIMIT, &mut |_, _| {}) {
        Ok(body) => match parse_release(&String::from_utf8_lossy(&body)) {
            Some(release) => release,
            None => {
                log_error!("업데이트 확인: 릴리스 정보를 해석할 수 없습니다");
                return Outcome::CheckFailed;
            }
        },
        Err(e) => {
            log_error!("업데이트 확인 실패: {e}");
            return Outcome::CheckFailed;
        }
    };
    log_info!("최신 릴리스 v{} (지금 v{current})", release.version);
    if release.version <= current {
        return Outcome::UpToDate;
    }
    let (Some(exe_url), Some(sha_url)) = (&release.exe_url, &release.sha_url) else {
        log_error!("업데이트: 릴리스에 실행 파일이나 체크섬이 없습니다");
        return Outcome::MissingAssets;
    };
    if !swap::writable(exe) {
        log_info!(
            "업데이트: {}에 쓸 수 없어 다운로드 페이지를 엽니다",
            exe.display()
        );
        ui.inform(&not_writable_body(release.version));
        return Outcome::NotWritable {
            page_url: release.page_url,
        };
    }
    if !ui.confirm(&prompt_body(release.version, current, &release.notes)) {
        log_info!("업데이트: 사용자가 취소했습니다");
        return Outcome::Declined;
    }
    *progress.version.lock().unwrap() = Some(release.version);

    let expected = match http::get(sha_url, SHA_LIMIT, &mut |_, _| {}) {
        Ok(body) => parse_checksum(&String::from_utf8_lossy(&body)),
        Err(e) => {
            log_error!("업데이트: 체크섬을 받지 못했습니다: {e}");
            return Outcome::DownloadFailed;
        }
    };
    let bytes = match http::get(exe_url, EXE_LIMIT, &mut |got, total| {
        if let Some(total) = total.filter(|&t| t > 0) {
            let fraction = (got.min(total) * 10000 / total) as u32;
            progress.fraction.store(fraction, Ordering::Relaxed);
        }
    }) {
        Ok(bytes) => bytes,
        Err(e) => {
            log_error!("업데이트: 실행 파일을 받지 못했습니다: {e}");
            return Outcome::DownloadFailed;
        }
    };
    log_info!("업데이트: {}바이트 받음", bytes.len());
    let actual: [u8; 32] = Sha256::digest(&bytes).into();
    if expected != Some(actual) {
        log_error!("업데이트: 체크섬이 맞지 않습니다");
        return Outcome::Corrupt;
    }
    if let Err(e) = swap::write_new(exe, &bytes) {
        log_error!("업데이트: 새 실행 파일을 쓰지 못했습니다: {e}");
        let _ = std::fs::remove_file(swap::new_path(exe));
        return Outcome::DownloadFailed;
    }
    log_info!("업데이트: v{} 준비됨", release.version);
    Outcome::Ready {
        version: release.version,
    }
}

/// This build's version.
pub fn current() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("crate version is x.y.z")
}

/// The releases endpoint; debug builds may point it elsewhere with
/// `SWISSWHEEL_UPDATE_URL` (a local server, while the repository is private).
pub fn endpoint() -> String {
    if cfg!(debug_assertions)
        && let Ok(url) = std::env::var("SWISSWHEEL_UPDATE_URL")
    {
        return url;
    }
    ENDPOINT.to_string()
}

/// Windows' own message box, shown from the update thread (never the UI
/// thread: its modal loop would starve the hooks' messages).
pub struct MessageBoxUi;

impl Ui for MessageBoxUi {
    fn confirm(&self, body: &str) -> bool {
        let flags = MB_YESNO | MB_ICONINFORMATION | MB_TOPMOST | MB_SETFOREGROUND;
        unsafe {
            MessageBoxW(None, &HSTRING::from(body), w!("SwissWheel 업데이트"), flags) == IDYES
        }
    }

    fn inform(&self, body: &str) {
        let flags = MB_OK | MB_ICONINFORMATION | MB_TOPMOST | MB_SETFOREGROUND;
        unsafe { MessageBoxW(None, &HSTRING::from(body), w!("SwissWheel 업데이트"), flags) };
    }
}

/// A running update thread.
pub struct Handle {
    pub progress: Arc<Progress>,
}

/// Spawns the update thread; when done it stores the outcome in `slot` and
/// posts `msg` to `hwnd`. The app never joins it: on exit the thread ends
/// with the process, and nothing is on disk before verification.
pub fn spawn(hwnd: isize, msg: u32, slot: Arc<Mutex<Option<Outcome>>>) -> Handle {
    let progress = Arc::new(Progress::default());
    let shared = Arc::clone(&progress);
    std::thread::spawn(move || {
        let outcome = match std::env::current_exe() {
            Ok(exe) => run(&endpoint(), current(), &exe, &MessageBoxUi, &shared),
            Err(e) => {
                log_error!("업데이트: 실행 파일 위치를 알 수 없습니다: {e}");
                Outcome::CheckFailed
            }
        };
        *slot.lock().unwrap() = Some(outcome);
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as *mut _)), msg, WPARAM(0), LPARAM(0));
        }
    });
    Handle { progress }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::sync::atomic::Ordering;
    use test_server::{Reply, TestServer};

    struct FakeUi {
        answer: bool,
        confirms: RefCell<Vec<String>>,
        informs: RefCell<Vec<String>>,
    }

    impl FakeUi {
        fn new(answer: bool) -> FakeUi {
            FakeUi {
                answer,
                confirms: RefCell::new(Vec::new()),
                informs: RefCell::new(Vec::new()),
            }
        }
    }

    impl Ui for FakeUi {
        fn confirm(&self, body: &str) -> bool {
            self.confirms.borrow_mut().push(body.to_string());
            self.answer
        }
        fn inform(&self, body: &str) {
            self.informs.borrow_mut().push(body.to_string());
        }
    }

    const CURRENT: Version = Version {
        major: 0,
        minor: 1,
        patch: 0,
    };
    const EXE: &[u8] = b"MZ new swisswheel";

    fn release(tag: &str, with_assets: bool) -> Reply {
        let assets = if with_assets {
            r#"{"name":"swisswheel.exe","browser_download_url":"{base}/exe"},
               {"name":"swisswheel.exe.sha256","browser_download_url":"{base}/sha"}"#
        } else {
            ""
        };
        Reply::Ok(
            format!(
                r#"{{"tag_name":"{tag}","html_url":"{{base}}/page","body":"고침","assets":[{assets}]}}"#
            )
            .into_bytes(),
        )
    }

    fn sha_of(bytes: &[u8]) -> Reply {
        let hex: String = Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        Reply::Ok(format!("{hex}  swisswheel.exe").into_bytes())
    }

    /// A server with the release, the exe reply, and the checksum of `EXE`.
    fn serve(tag: &str, exe: Reply) -> TestServer {
        TestServer::start(vec![
            ("/latest", release(tag, true)),
            ("/exe", exe),
            ("/sha", sha_of(EXE)),
        ])
    }

    fn exe_in(dir: &Path) -> PathBuf {
        let exe = dir.join("swisswheel.exe");
        std::fs::write(&exe, b"old").unwrap();
        exe
    }

    fn check(server: &TestServer, exe: &Path, ui: &FakeUi, progress: &Progress) -> Outcome {
        run(
            &format!("{}/latest", server.base),
            CURRENT,
            exe,
            ui,
            progress,
        )
    }

    #[test]
    fn newer_release_downloads_and_verifies() {
        let dir = tempfile::tempdir().unwrap();
        let exe = exe_in(dir.path());
        let server = serve("v9.0.0", Reply::Ok(EXE.to_vec()));
        let ui = FakeUi::new(true);
        let progress = Progress::default();
        let nine = Version::parse("9.0.0").unwrap();
        assert_eq!(
            check(&server, &exe, &ui, &progress),
            Outcome::Ready { version: nine }
        );
        assert_eq!(std::fs::read(swap::new_path(&exe)).unwrap(), EXE);
        assert_eq!(progress.fraction.load(Ordering::Relaxed), 10000);
        assert_eq!(*progress.version.lock().unwrap(), Some(nine));
        assert!(ui.confirms.borrow()[0].contains("v9.0.0이 있습니다"));
    }

    #[test]
    fn same_version_is_up_to_date() {
        let dir = tempfile::tempdir().unwrap();
        let exe = exe_in(dir.path());
        let server = serve("v0.1.0", Reply::Ok(EXE.to_vec()));
        let ui = FakeUi::new(true);
        assert_eq!(
            check(&server, &exe, &ui, &Progress::default()),
            Outcome::UpToDate
        );
        assert!(ui.confirms.borrow().is_empty() && ui.informs.borrow().is_empty());
    }

    #[test]
    fn declined_downloads_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let exe = exe_in(dir.path());
        let server = serve("v9.0.0", Reply::Ok(EXE.to_vec()));
        let ui = FakeUi::new(false);
        assert_eq!(
            check(&server, &exe, &ui, &Progress::default()),
            Outcome::Declined
        );
        assert!(!swap::new_path(&exe).exists());
    }

    #[test]
    fn corrupt_download_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let exe = exe_in(dir.path());
        let server = serve("v9.0.0", Reply::Ok(b"MZ tampered".to_vec()));
        assert_eq!(
            check(&server, &exe, &FakeUi::new(true), &Progress::default()),
            Outcome::Corrupt
        );
        assert!(!swap::new_path(&exe).exists());
        assert_eq!(std::fs::read(&exe).unwrap(), b"old");
    }

    #[test]
    fn dropped_connection_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let exe = exe_in(dir.path());
        let cut = Reply::Drop {
            send: EXE[..5].to_vec(),
            claimed: EXE.len(),
        };
        let server = serve("v9.0.0", cut);
        assert_eq!(
            check(&server, &exe, &FakeUi::new(true), &Progress::default()),
            Outcome::DownloadFailed
        );
        assert!(!swap::new_path(&exe).exists());
        assert_eq!(std::fs::read(&exe).unwrap(), b"old");
    }

    #[test]
    fn missing_assets_reported() {
        let dir = tempfile::tempdir().unwrap();
        let exe = exe_in(dir.path());
        let server = TestServer::start(vec![("/latest", release("v9.0.0", false))]);
        assert_eq!(
            check(&server, &exe, &FakeUi::new(true), &Progress::default()),
            Outcome::MissingAssets
        );
    }

    #[test]
    fn server_error_is_check_failed() {
        let dir = tempfile::tempdir().unwrap();
        let exe = exe_in(dir.path());
        let server = TestServer::start(vec![("/latest", Reply::Status(500))]);
        assert_eq!(
            check(&server, &exe, &FakeUi::new(true), &Progress::default()),
            Outcome::CheckFailed
        );
    }

    #[test]
    fn unwritable_folder_opens_page() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("없음").join("swisswheel.exe");
        let server = serve("v9.0.0", Reply::Ok(EXE.to_vec()));
        let ui = FakeUi::new(true);
        assert_eq!(
            check(&server, &exe, &ui, &Progress::default()),
            Outcome::NotWritable {
                page_url: format!("{}/page", server.base)
            }
        );
        assert_eq!(ui.informs.borrow().len(), 1);
        assert!(ui.confirms.borrow().is_empty());
    }
}
