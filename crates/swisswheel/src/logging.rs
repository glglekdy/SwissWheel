//! Append-only log file at `%LOCALAPPDATA%\SwissWheel\logs\swisswheel.log`.

use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use windows::Win32::System::SystemInformation::GetLocalTime;

static FILE: OnceLock<Mutex<Option<File>>> = OnceLock::new();

/// Opens `dir/swisswheel.log` for appending, creating `dir`. Calling again
/// switches to the new location.
pub fn init(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("swisswheel.log"))?;
    *FILE.get_or_init(|| Mutex::new(None)).lock().unwrap() = Some(file);
    Ok(())
}

pub fn write(level: &str, msg: &str) {
    let t = unsafe { GetLocalTime() };
    let line = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03} {level} {msg}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds
    );
    if cfg!(debug_assertions) {
        eprintln!("{line}");
    }
    if let Some(lock) = FILE.get()
        && let Some(file) = lock.lock().unwrap().as_mut()
    {
        let _ = writeln!(file, "{line}");
    }
}

macro_rules! log_info {
    ($($arg:tt)*) => { $crate::logging::write("INFO", &format!($($arg)*)) };
}

macro_rules! log_error {
    ($($arg:tt)*) => { $crate::logging::write("ERROR", &format!($($arg)*)) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_lines_are_appended() {
        let dir = tempfile::tempdir().unwrap();
        init(dir.path()).unwrap();
        write("INFO", "첫 줄");
        write("ERROR", "둘째 줄");
        let text = std::fs::read_to_string(dir.path().join("swisswheel.log")).unwrap();
        // Other tests run in parallel and log through the same global file,
        // so their lines may land in between; only the order of ours counts.
        let lines: Vec<_> = text.lines().collect();
        let first = lines.iter().position(|l| l.ends_with("INFO 첫 줄"));
        let second = lines.iter().position(|l| l.ends_with("ERROR 둘째 줄"));
        assert!(
            matches!((first, second), (Some(a), Some(b)) if a < b),
            "{lines:?}"
        );
    }
}
