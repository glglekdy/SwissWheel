//! The download tile's installer: fetch GyanD's essentials build, check it
//! against GitHub's digest, and put `ffmpeg.exe` and `ffprobe.exe` where
//! `engines::ffmpeg::locate` looks (spec 3.3).

use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use sha2::{Digest as _, Sha256};
use swisswheel_core::ffmpeg_release::{
    DIALOG_TITLE, ENDPOINT, FfmpegRelease, ReleaseError, parse_release, prompt_body,
};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::engines::ffmpeg::install_dirs;
use crate::update::{MessageBoxUi, Ui, http};

const JSON_LIMIT: usize = 1 << 20;
const ZIP_LIMIT: usize = 512 << 20;

#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// Notice `text::installed(tag)`.
    Installed {
        tag: String,
    },
    CheckFailed,
    MissingAsset,
    DownloadFailed,
    Corrupt,
    /// No folder to install into, a zip without the tools, or a failed move.
    InstallFailed,
    Declined,
}

#[derive(Debug, PartialEq)]
pub enum RemoveOutcome {
    Removed,
    /// A file couldn't be deleted (in use); the rest are gone.
    Failed,
    Declined,
}

/// Shared with the UI thread: download fraction (0..=10000) and, once
/// confirmed, the release tag (the progress row's name).
#[derive(Default)]
pub struct Progress {
    pub fraction: AtomicU32,
    pub tag: Mutex<Option<String>>,
}

/// The whole check → confirm → download → verify → install flow; blocks.
/// `dirs` are the install folders in order of preference (see
/// [`install_dirs`]); nothing reaches one before the zip matches its digest.
pub fn run(endpoint: &str, dirs: &[PathBuf], ui: &dyn Ui, progress: &Progress) -> Outcome {
    log_info!("FFmpeg 확인: {endpoint}");
    let release = match http::get(endpoint, JSON_LIMIT, &mut |_, _| {}) {
        Ok(body) => match parse_release(&String::from_utf8_lossy(&body)) {
            Ok(release) => release,
            Err(ReleaseError::NoAsset) => {
                log_error!("FFmpeg: 릴리스에 essentials zip이나 digest가 없습니다");
                return Outcome::MissingAsset;
            }
            Err(ReleaseError::Unreadable) => {
                log_error!("FFmpeg: 릴리스 정보를 해석할 수 없습니다");
                return Outcome::CheckFailed;
            }
        },
        Err(e) => {
            log_error!("FFmpeg 확인 실패: {e}");
            return Outcome::CheckFailed;
        }
    };
    log_info!(
        "FFmpeg 최신 릴리스 {} ({}, {}바이트)",
        release.tag,
        release.zip_url,
        release.size
    );
    let Some(target) = pick_target(dirs) else {
        log_error!("FFmpeg: 설치할 수 있는 폴더가 없습니다");
        return Outcome::InstallFailed;
    };
    if !ui.confirm(&prompt_body(&release.tag, release.size, &target)) {
        log_info!("FFmpeg: 사용자가 취소했습니다");
        return Outcome::Declined;
    }
    *progress.tag.lock().unwrap() = Some(release.tag.clone());
    log_info!("FFmpeg: {}에 설치합니다", target.display());
    let partial = partial_path(&target);
    let outcome = download_and_install(&release, &target, &partial, progress);
    remove_partial(&partial);
    outcome
}

fn download_and_install(
    release: &FfmpegRelease,
    target: &Path,
    partial: &Path,
    progress: &Progress,
) -> Outcome {
    let zip = partial.join("download.zip");
    let file = match fresh_dir(partial).and_then(|()| File::create(&zip)) {
        Ok(file) => file,
        Err(e) => {
            log_error!("FFmpeg: 임시 폴더를 만들지 못했습니다: {e}");
            return Outcome::InstallFailed;
        }
    };
    let mut sink = HashingWriter {
        inner: BufWriter::new(file),
        hasher: Sha256::new(),
    };
    let got = http::get_to(&release.zip_url, ZIP_LIMIT, &mut sink, &mut |got, total| {
        if let Some(total) = total.filter(|&t| t > 0) {
            let fraction = (got.min(total) * 10000 / total) as u32;
            progress.fraction.store(fraction, Ordering::Relaxed);
        }
    });
    let HashingWriter { inner, hasher } = sink;
    // Closes the file before it is read back or removed.
    let flushed = inner.into_inner().map(drop).map_err(|e| e.into_error());
    match (got, flushed) {
        (Err(e), _) => {
            log_error!("FFmpeg를 받지 못했습니다: {e}");
            return Outcome::DownloadFailed;
        }
        (Ok(_), Err(e)) => {
            log_error!("FFmpeg를 받지 못했습니다: {e}");
            return Outcome::DownloadFailed;
        }
        (Ok(n), Ok(())) => log_info!("FFmpeg: {n}바이트 받음"),
    }
    let actual: [u8; 32] = hasher.finalize().into();
    if actual != release.sha256 {
        log_error!("FFmpeg: 체크섬이 맞지 않습니다");
        return Outcome::Corrupt;
    }
    if let Err(e) = extract(&zip, partial) {
        log_error!("FFmpeg: 압축을 풀지 못했습니다: {e}");
        return Outcome::InstallFailed;
    }
    if let Err(e) = move_into(partial, target) {
        log_error!("FFmpeg: {}로 옮기지 못했습니다: {e}", target.display());
        return Outcome::InstallFailed;
    }
    log_info!("FFmpeg {} 설치 완료", release.tag);
    Outcome::Installed {
        tag: release.tag.clone(),
    }
}

/// Writes through to `inner`, hashing what passes.
struct HashingWriter<W> {
    inner: W,
    hasher: Sha256,
}

impl<W: Write> Write for HashingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// The scratch folder beside an install folder; same volume, so the final
/// moves are renames.
pub fn partial_path(target: &Path) -> PathBuf {
    target.with_file_name("ffmpeg.partial")
}

/// Removes what an interrupted download left beside each install folder.
pub fn cleanup(dirs: &[PathBuf]) {
    for dir in dirs {
        remove_partial(&partial_path(dir));
    }
}

fn remove_partial(partial: &Path) {
    match fs::remove_dir_all(partial) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => log_error!(
            "FFmpeg 임시 폴더를 지우지 못했습니다: {} ({e})",
            partial.display()
        ),
        _ => {}
    }
}

/// The first folder whose parent can be made and written to.
fn pick_target(dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .find(|dir| {
            let Some(parent) = dir.parent() else {
                return false;
            };
            let probe = parent.join(".swisswheel-write-test");
            let made = fs::create_dir_all(parent).is_ok() && File::create(&probe).is_ok();
            if made {
                let _ = fs::remove_file(&probe);
            }
            made
        })
        .cloned()
}

fn fresh_dir(dir: &Path) -> io::Result<()> {
    match fs::remove_dir_all(dir) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    fs::create_dir(dir)
}

/// Writes the zip's `<top>/bin/ffmpeg.exe`, `<top>/bin/ffprobe.exe`, and
/// `<top>/LICENSE` into `into` under those fixed names; the entries' own
/// paths are never used as paths. Both tools must be there.
fn extract(zip: &Path, into: &Path) -> io::Result<()> {
    let mut archive = zip::ZipArchive::new(File::open(zip)?).map_err(io::Error::other)?;
    let (mut ffmpeg, mut ffprobe) = (false, false);
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(io::Error::other)?;
        let name = entry.name().map_err(io::Error::other)?.into_owned();
        let parts: Vec<&str> = name.split('/').collect();
        let file = match parts.as_slice() {
            [_, "bin", file @ ("ffmpeg.exe" | "ffprobe.exe")] => *file,
            [_, "LICENSE"] => "LICENSE",
            _ => continue,
        };
        io::copy(&mut entry, &mut File::create(into.join(file))?)?;
        ffmpeg |= file == "ffmpeg.exe";
        ffprobe |= file == "ffprobe.exe";
    }
    if ffmpeg && ffprobe {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "zip에 ffmpeg.exe나 ffprobe.exe가 없습니다",
        ))
    }
}

/// `ffmpeg.exe` goes last; `locate` wants both, so a folder is never used
/// half-moved.
fn move_into(partial: &Path, target: &Path) -> io::Result<()> {
    fs::create_dir_all(target)?;
    for name in ["ffprobe.exe", "LICENSE", "ffmpeg.exe"] {
        let from = partial.join(name);
        if from.is_file() {
            fs::rename(&from, target.join(name))?;
        }
    }
    Ok(())
}

/// The names `move_into` puts in an install folder, the only files removed.
const INSTALLED: [&str; 3] = ["ffmpeg.exe", "ffprobe.exe", "LICENSE"];

/// After the user confirms `prompt`, deletes the app's own copy from each
/// of `dirs`, and each folder left empty; anything else stays. Blocks.
pub fn remove(dirs: &[PathBuf], prompt: &str, ui: &dyn Ui) -> RemoveOutcome {
    if !ui.confirm(prompt) {
        log_info!("FFmpeg 제거: 사용자가 취소했습니다");
        return RemoveOutcome::Declined;
    }
    let mut failed = false;
    for dir in dirs {
        for name in INSTALLED {
            let path = dir.join(name);
            match fs::remove_file(&path) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => {
                    log_error!("FFmpeg: {}를 지우지 못했습니다: {e}", path.display());
                    failed = true;
                }
                _ => {}
            }
        }
        // Fails while other files remain, which then stay with their folder.
        let _ = fs::remove_dir(dir);
    }
    if failed {
        RemoveOutcome::Failed
    } else {
        log_info!("FFmpeg를 지웠습니다");
        RemoveOutcome::Removed
    }
}

/// `dirs` with `prefer` first: an update goes where the old copy is.
fn preferring(dirs: Vec<PathBuf>, prefer: Option<&Path>) -> Vec<PathBuf> {
    let Some(prefer) = prefer else {
        return dirs;
    };
    std::iter::once(prefer.to_path_buf())
        .chain(dirs.into_iter().filter(|d| d != prefer))
        .collect()
}

/// The releases endpoint; debug builds may point it elsewhere with
/// `SWISSWHEEL_FFMPEG_URL` (a local server).
pub fn endpoint() -> String {
    if cfg!(debug_assertions)
        && let Ok(url) = std::env::var("SWISSWHEEL_FFMPEG_URL")
    {
        return url;
    }
    ENDPOINT.to_string()
}

/// A running download thread.
pub struct Handle {
    pub progress: Arc<Progress>,
}

/// Spawns the download thread; when done it stores the outcome in `slot`
/// and posts `msg` to `hwnd`. The app never joins it: on exit the thread
/// ends with the process, and the next start removes its scratch folder.
/// `prefer`: the folder to install into if it can be written (an update).
pub fn spawn(
    hwnd: isize,
    msg: u32,
    slot: Arc<Mutex<Option<Outcome>>>,
    prefer: Option<PathBuf>,
) -> Handle {
    let progress = Arc::new(Progress::default());
    let shared = Arc::clone(&progress);
    std::thread::spawn(move || {
        let ui = MessageBoxUi {
            title: DIALOG_TITLE,
        };
        let dirs = preferring(install_dirs(), prefer.as_deref());
        let outcome = run(&endpoint(), &dirs, &ui, &shared);
        *slot.lock().unwrap() = Some(outcome);
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as *mut _)), msg, WPARAM(0), LPARAM(0));
        }
    });
    Handle { progress }
}

/// Spawns the remove thread: confirms `prompt`, removes from
/// `install_dirs()`, stores the outcome in `slot`, and posts `msg`.
pub fn spawn_remove(
    hwnd: isize,
    msg: u32,
    slot: Arc<Mutex<Option<RemoveOutcome>>>,
    prompt: String,
) {
    std::thread::spawn(move || {
        let ui = MessageBoxUi {
            title: DIALOG_TITLE,
        };
        let outcome = remove(&install_dirs(), &prompt, &ui);
        *slot.lock().unwrap() = Some(outcome);
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as *mut _)), msg, WPARAM(0), LPARAM(0));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::test_server::{Reply, TestServer};
    use std::cell::RefCell;
    use std::sync::atomic::Ordering;

    struct FakeUi {
        answer: bool,
        confirms: RefCell<Vec<String>>,
    }

    impl FakeUi {
        fn new(answer: bool) -> FakeUi {
            FakeUi {
                answer,
                confirms: RefCell::new(Vec::new()),
            }
        }
    }

    impl Ui for FakeUi {
        fn confirm(&self, body: &str) -> bool {
            self.confirms.borrow_mut().push(body.to_string());
            self.answer
        }
        fn inform(&self, _: &str) {}
    }

    const GOOD: &[(&str, &[u8])] = &[
        ("ffmpeg-1.0-essentials_build/bin/ffmpeg.exe", b"MZ ffmpeg"),
        ("ffmpeg-1.0-essentials_build/bin/ffprobe.exe", b"MZ ffprobe"),
        ("ffmpeg-1.0-essentials_build/bin/ffplay.exe", b"MZ ffplay"),
        ("ffmpeg-1.0-essentials_build/LICENSE", b"GPL"),
        ("ffmpeg-1.0-essentials_build/doc/faq.html", b"<html>"),
        ("../evil.exe", b"MZ evil"),
    ];

    fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        for (name, bytes) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    fn release_json(digest_of: &[u8], size: usize) -> Reply {
        Reply::Ok(
            format!(
                r#"{{"tag_name":"1.0","assets":[
                    {{"name":"ffmpeg-1.0-essentials_build.7z","size":1,"digest":"sha256:{h}","browser_download_url":"{{base}}/7z"}},
                    {{"name":"ffmpeg-1.0-essentials_build.zip","size":{size},"digest":"sha256:{h}","browser_download_url":"{{base}}/zip"}}
                ]}}"#,
                h = hex(digest_of)
            )
            .into_bytes(),
        )
    }

    /// Serves the release JSON (with the digest of `digest_of`) and `zip`.
    fn serve(zip: Reply, digest_of: &[u8]) -> TestServer {
        TestServer::start(vec![
            ("/latest", release_json(digest_of, digest_of.len())),
            ("/zip", zip),
        ])
    }

    fn good_server() -> TestServer {
        let zip = make_zip(GOOD);
        serve(Reply::Ok(zip.clone()), &zip)
    }

    /// `<root>/app/ffmpeg` (its parent exists) and `<root>/local/ffmpeg`.
    fn dirs_in(root: &Path) -> [PathBuf; 2] {
        std::fs::create_dir_all(root.join("app")).unwrap();
        [
            root.join("app").join("ffmpeg"),
            root.join("local").join("ffmpeg"),
        ]
    }

    fn install(server: &TestServer, dirs: &[PathBuf], ui: &FakeUi, progress: &Progress) -> Outcome {
        run(&format!("{}/latest", server.base), dirs, ui, progress)
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn installs_only_the_tools() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs_in(root.path());
        let server = good_server();
        let ui = FakeUi::new(true);
        let progress = Progress::default();
        assert_eq!(
            install(&server, &dirs, &ui, &progress),
            Outcome::Installed { tag: "1.0".into() }
        );
        assert_eq!(names(&dirs[0]), ["LICENSE", "ffmpeg.exe", "ffprobe.exe"]);
        assert_eq!(
            std::fs::read(dirs[0].join("ffmpeg.exe")).unwrap(),
            b"MZ ffmpeg"
        );
        assert_eq!(
            std::fs::read(dirs[0].join("ffprobe.exe")).unwrap(),
            b"MZ ffprobe"
        );
        assert_eq!(names(&root.path().join("app")), ["ffmpeg"]);
        assert!(!root.path().join("evil.exe").exists());
        assert!(!dirs[1].exists());
        assert_eq!(progress.fraction.load(Ordering::Relaxed), 10000);
        assert_eq!(progress.tag.lock().unwrap().as_deref(), Some("1.0"));
        assert!(ui.confirms.borrow()[0].contains(&dirs[0].display().to_string()));
        assert!(crate::engines::ffmpeg::locate_in(&dirs, None).is_some());
    }

    #[test]
    fn installs_into_a_unicode_folder() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs_in(&root.path().join("김 지훈"));
        let server = good_server();
        assert_eq!(
            install(&server, &dirs, &FakeUi::new(true), &Progress::default()),
            Outcome::Installed { tag: "1.0".into() }
        );
        assert!(crate::engines::ffmpeg::locate_in(&dirs, None).is_some());
    }

    #[test]
    fn falls_back_when_the_app_folder_cant_be_made() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("file"), b"").unwrap();
        let dirs = [
            root.path().join("file").join("ffmpeg"),
            root.path().join("local").join("ffmpeg"),
        ];
        let server = good_server();
        let ui = FakeUi::new(true);
        assert_eq!(
            install(&server, &dirs, &ui, &Progress::default()),
            Outcome::Installed { tag: "1.0".into() }
        );
        assert!(dirs[1].join("ffprobe.exe").is_file());
        assert!(ui.confirms.borrow()[0].contains(&dirs[1].display().to_string()));
    }

    #[test]
    fn failures_leave_no_partial() {
        let zip = make_zip(GOOD);
        let no_probe = make_zip(&GOOD[..1]);
        let cut = Reply::Drop {
            send: zip[..10].to_vec(),
            claimed: zip.len(),
        };
        let cases = [
            (
                serve(Reply::Ok(zip.clone()), b"something else"),
                Outcome::Corrupt,
            ),
            (
                serve(Reply::Ok(no_probe.clone()), &no_probe),
                Outcome::InstallFailed,
            ),
            (serve(cut, &zip), Outcome::DownloadFailed),
        ];
        for (server, expected) in cases {
            let root = tempfile::tempdir().unwrap();
            let dirs = dirs_in(root.path());
            assert_eq!(
                install(&server, &dirs, &FakeUi::new(true), &Progress::default()),
                expected
            );
            assert!(!dirs[0].exists(), "{expected:?}");
            assert!(!partial_path(&dirs[0]).exists(), "{expected:?}");
        }
    }

    #[test]
    fn declined_or_missing_touches_nothing() {
        let no_zip = Reply::Ok(br#"{"tag_name":"1.0","assets":[]}"#.to_vec());
        let cases = [
            (good_server(), false, Outcome::Declined),
            (
                TestServer::start(vec![("/latest", no_zip)]),
                true,
                Outcome::MissingAsset,
            ),
            (
                TestServer::start(vec![("/latest", Reply::Status(500))]),
                true,
                Outcome::CheckFailed,
            ),
        ];
        for (server, answer, expected) in cases {
            let root = tempfile::tempdir().unwrap();
            let dirs = dirs_in(root.path());
            assert_eq!(
                install(&server, &dirs, &FakeUi::new(answer), &Progress::default()),
                expected
            );
            assert_eq!(names(&root.path().join("app")), Vec::<String>::new());
        }
    }

    #[test]
    fn stale_partial_is_replaced() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs_in(root.path());
        let stale = partial_path(&dirs[0]);
        std::fs::create_dir(&stale).unwrap();
        std::fs::write(stale.join("old.txt"), b"x").unwrap();
        assert_eq!(
            install(
                &good_server(),
                &dirs,
                &FakeUi::new(true),
                &Progress::default()
            ),
            Outcome::Installed { tag: "1.0".into() }
        );
        assert!(!stale.exists());
        assert_eq!(names(&dirs[0]), ["LICENSE", "ffmpeg.exe", "ffprobe.exe"]);
    }

    #[test]
    fn completes_a_half_installed_folder() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs_in(root.path());
        std::fs::create_dir(&dirs[0]).unwrap();
        std::fs::write(dirs[0].join("ffmpeg.exe"), b"old").unwrap();
        assert_eq!(
            install(
                &good_server(),
                &dirs,
                &FakeUi::new(true),
                &Progress::default()
            ),
            Outcome::Installed { tag: "1.0".into() }
        );
        assert_eq!(
            std::fs::read(dirs[0].join("ffmpeg.exe")).unwrap(),
            b"MZ ffmpeg"
        );
        assert_eq!(
            std::fs::read(dirs[0].join("ffprobe.exe")).unwrap(),
            b"MZ ffprobe"
        );
    }

    #[test]
    fn cleanup_removes_partials() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs_in(root.path());
        std::fs::create_dir_all(partial_path(&dirs[0]).join("sub")).unwrap();
        std::fs::create_dir_all(partial_path(&dirs[1])).unwrap();
        cleanup(&dirs);
        assert!(!partial_path(&dirs[0]).exists());
        assert!(!partial_path(&dirs[1]).exists());
        cleanup(&dirs); // nothing left: no panic
    }

    fn installed_in(dir: &Path) {
        fs::create_dir_all(dir).unwrap();
        for f in ["ffmpeg.exe", "ffprobe.exe", "LICENSE"] {
            fs::write(dir.join(f), b"x").unwrap();
        }
    }

    #[test]
    fn remove_clears_both_install_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = vec![
            tmp.path().join("앱").join("ffmpeg"),
            tmp.path().join("로컬").join("ffmpeg"),
        ];
        dirs.iter().for_each(|d| installed_in(d));
        let ui = FakeUi::new(true);
        assert_eq!(remove(&dirs, "지울까요?", &ui), RemoveOutcome::Removed);
        assert_eq!(*ui.confirms.borrow(), ["지울까요?"]);
        assert!(dirs.iter().all(|d| !d.exists()));
    }

    #[test]
    fn remove_keeps_other_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("ffmpeg");
        installed_in(&dir);
        fs::write(dir.join("내 메모.txt"), b"keep").unwrap();
        assert_eq!(
            remove(std::slice::from_ref(&dir), "p", &FakeUi::new(true)),
            RemoveOutcome::Removed
        );
        assert_eq!(names(&dir), ["내 메모.txt"]);
    }

    #[test]
    fn remove_declined_or_with_nothing_there() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("ffmpeg");
        installed_in(&dir);
        assert_eq!(
            remove(std::slice::from_ref(&dir), "p", &FakeUi::new(false)),
            RemoveOutcome::Declined
        );
        assert_eq!(names(&dir), ["LICENSE", "ffmpeg.exe", "ffprobe.exe"]);
        assert_eq!(
            remove(&[tmp.path().join("없음")], "p", &FakeUi::new(true)),
            RemoveOutcome::Removed
        );
    }

    #[test]
    fn remove_reports_a_file_it_cannot_delete() {
        use std::os::windows::fs::OpenOptionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("ffmpeg");
        installed_in(&dir);
        // std shares delete by default; share nothing, as a running exe would.
        let open = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(dir.join("ffprobe.exe"))
            .unwrap();
        assert_eq!(
            remove(std::slice::from_ref(&dir), "p", &FakeUi::new(true)),
            RemoveOutcome::Failed
        );
        drop(open);
        assert_eq!(names(&dir), ["ffprobe.exe"]); // the rest went; a retry finishes
    }

    #[test]
    fn update_goes_to_the_preferred_dir() {
        let (a, b) = (PathBuf::from(r"C:\a\ffmpeg"), PathBuf::from(r"C:\b\ffmpeg"));
        assert_eq!(
            preferring(vec![a.clone(), b.clone()], Some(&b)),
            [b.clone(), a.clone()]
        );
        assert_eq!(
            preferring(vec![a.clone(), b.clone()], None),
            [a.clone(), b.clone()]
        );
        let c = PathBuf::from(r"D:\c\ffmpeg");
        assert_eq!(preferring(vec![a.clone()], Some(&c)), [c, a]);
    }
}
