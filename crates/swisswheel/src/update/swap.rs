//! Swapping the running exe for the downloaded one, and undoing it.
//!
//! A running exe can't be deleted or overwritten, but it can be renamed:
//! the old one steps aside as `swisswheel.exe.old`, the new one takes its
//! name, and the next start removes the leftover.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};

/// `<exe folder>\swisswheel.exe.new`: the verified download.
pub fn new_path(exe: &Path) -> PathBuf {
    sibling(exe, "swisswheel.exe.new")
}

/// `<exe folder>\swisswheel.exe.old`: the previous exe, removed on next start.
pub fn old_path(exe: &Path) -> PathBuf {
    sibling(exe, "swisswheel.exe.old")
}

fn sibling(exe: &Path, name: &str) -> PathBuf {
    exe.parent().unwrap_or(Path::new(".")).join(name)
}

/// Whether the exe's folder takes new files (not, say, Program Files).
pub fn writable(exe: &Path) -> bool {
    let probe = new_path(exe);
    let made = std::fs::File::create(&probe).is_ok();
    if made {
        let _ = std::fs::remove_file(&probe);
    }
    made
}

pub fn write_new(exe: &Path, bytes: &[u8]) -> io::Result<()> {
    std::fs::write(new_path(exe), bytes)
}

/// exe → .old, .new → exe, then `launch(exe)`; on any failure restores exe.
pub fn swap(exe: &Path, launch: impl FnOnce(&Path) -> io::Result<()>) -> io::Result<()> {
    let (new, old) = (new_path(exe), old_path(exe));
    if !new.is_file() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "no downloaded exe"));
    }
    // A leftover .old would block nothing (rename replaces it) but is stale.
    let _ = std::fs::remove_file(&old);
    // std's rename is MoveFileExW with MOVEFILE_REPLACE_EXISTING.
    std::fs::rename(exe, &old)?;
    if let Err(e) = std::fs::rename(&new, exe) {
        std::fs::rename(&old, exe)?;
        return Err(e);
    }
    if let Err(e) = launch(exe) {
        let _ = std::fs::remove_file(exe);
        std::fs::rename(&old, exe)?;
        return Err(e);
    }
    Ok(())
}

/// Removes a leftover .old and .new; what can't go now goes next start.
pub fn cleanup(exe: &Path) {
    for path in [old_path(exe), new_path(exe)] {
        if path.exists()
            && let Err(e) = std::fs::remove_file(&path)
        {
            log_error!(
                "업데이트 잔여 파일을 지우지 못했습니다: {} ({e})",
                path.display()
            );
        }
    }
}

/// Starts `exe --after-update <this pid>`, so it waits for us to exit.
pub fn launch_after_update(exe: &Path) -> io::Result<()> {
    launch_with(exe, "--after-update")
}

/// Starts `exe --restarted <this pid>` (the restart tile), so it waits for
/// us to exit.
pub fn launch_restarted(exe: &Path) -> io::Result<()> {
    launch_with(exe, swisswheel_core::restart::ARG)
}

fn launch_with(exe: &Path, arg: &str) -> io::Result<()> {
    std::process::Command::new(exe)
        .arg(arg)
        .arg(std::process::id().to_string())
        .spawn()
        .map(drop)
}

/// Waits for process `pid` to end; `true` if it did (or is already gone).
pub fn wait_for_exit(pid: u32, timeout: Duration) -> bool {
    let Ok(process) = (unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) }) else {
        return true; // no such process
    };
    let ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
    let exited = unsafe { WaitForSingleObject(process, ms) } == WAIT_OBJECT_0;
    unsafe {
        let _ = CloseHandle(process);
    }
    exited
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Instant;

    fn exe_in(dir: &Path) -> PathBuf {
        let exe = dir.join("swisswheel.exe");
        fs::write(&exe, b"old").unwrap();
        exe
    }

    #[test]
    fn launch_restarted_reports_a_missing_exe() {
        let dir = tempfile::tempdir().unwrap();
        assert!(launch_restarted(&dir.path().join("없음.exe")).is_err());
    }

    #[test]
    fn swap_replaces_and_keeps_old() {
        let dir = tempfile::tempdir().unwrap();
        let exe = exe_in(dir.path());
        write_new(&exe, b"new").unwrap();
        swap(&exe, |_| Ok(())).unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"new");
        assert_eq!(fs::read(old_path(&exe)).unwrap(), b"old");
        assert!(!new_path(&exe).exists());
    }

    #[test]
    fn swap_restores_when_launch_fails() {
        let dir = tempfile::tempdir().unwrap();
        let exe = exe_in(dir.path());
        write_new(&exe, b"new").unwrap();
        let result = swap(&exe, |_| Err(std::io::Error::other("no")));
        assert!(result.is_err());
        assert_eq!(fs::read(&exe).unwrap(), b"old");
        assert!(!new_path(&exe).exists());
        assert!(!old_path(&exe).exists());
    }

    #[test]
    fn swap_restores_when_new_missing() {
        let dir = tempfile::tempdir().unwrap();
        let exe = exe_in(dir.path());
        assert!(swap(&exe, |_| Ok(())).is_err());
        assert_eq!(fs::read(&exe).unwrap(), b"old");
    }

    #[test]
    fn swap_in_unicode_folder() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("업데이트 테스트 폴더");
        fs::create_dir(&folder).unwrap();
        let exe = exe_in(&folder);
        assert!(writable(&exe));
        assert!(!new_path(&exe).exists());
        write_new(&exe, b"new").unwrap();
        swap(&exe, |_| Ok(())).unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"new");
        assert_eq!(fs::read(old_path(&exe)).unwrap(), b"old");
    }

    #[test]
    fn cleanup_removes_leftovers() {
        let dir = tempfile::tempdir().unwrap();
        let exe = exe_in(dir.path());
        fs::write(old_path(&exe), b"x").unwrap();
        fs::write(new_path(&exe), b"y").unwrap();
        cleanup(&exe);
        assert!(!old_path(&exe).exists());
        assert!(!new_path(&exe).exists());
        assert_eq!(fs::read(&exe).unwrap(), b"old");
    }

    #[test]
    fn not_writable_for_missing_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!writable(&dir.path().join("없음").join("swisswheel.exe")));
    }

    fn ping(count: u32) -> std::process::Child {
        std::process::Command::new("cmd")
            .args(["/c", &format!("ping -n {count} 127.0.0.1 >nul")])
            .spawn()
            .unwrap()
    }

    #[test]
    fn wait_for_exit_waits_for_child() {
        let mut child = ping(2);
        let start = Instant::now();
        assert!(wait_for_exit(child.id(), Duration::from_secs(15)));
        assert!(start.elapsed() >= Duration::from_millis(500));
        assert!(child.try_wait().unwrap().is_some());
    }

    #[test]
    fn wait_for_exit_times_out() {
        let mut child = ping(6);
        assert!(!wait_for_exit(child.id(), Duration::from_millis(300)));
        let _ = child.kill();
        let _ = child.wait();
    }
}
