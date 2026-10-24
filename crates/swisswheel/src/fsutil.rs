//! Output path reservations and the no-replace move that finalizes a result.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use swisswheel_core::naming;
use windows::Win32::Storage::FileSystem::{
    MOVEFILE_COPY_ALLOWED, MOVEFILE_WRITE_THROUGH, MoveFileExW,
};
use windows::core::HSTRING;

/// Moves `temp` to `final_path`, failing with `AlreadyExists` rather than
/// replacing an existing file (unlike `std::fs::rename`).
pub fn finalize(temp: &Path, final_path: &Path) -> std::io::Result<()> {
    unsafe {
        MoveFileExW(
            &HSTRING::from(temp),
            &HSTRING::from(final_path),
            MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|e| io_error(&e))
}

/// Moves a file or folder to `to`, failing with `AlreadyExists` rather than
/// replacing anything. A file may cross drives (copied, then deleted); a
/// folder can't, and fails with `CrossesDevices`.
pub fn move_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    unsafe {
        MoveFileExW(
            &HSTRING::from(from),
            &HSTRING::from(to),
            MOVEFILE_COPY_ALLOWED | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|e| io_error(&e))
}

/// `windows::core::Error` carries an HRESULT; unwrap FACILITY_WIN32 errors to
/// the Win32 code so std maps them to the right `ErrorKind`.
fn io_error(e: &windows::core::Error) -> std::io::Error {
    let hr = e.code().0 as u32;
    if hr & 0xFFFF_0000 == 0x8007_0000 {
        std::io::Error::from_raw_os_error((hr & 0xFFFF) as i32)
    } else {
        std::io::Error::other(e.message())
    }
}

/// Output paths handed out to running jobs but not yet on disk, so two jobs
/// never pick the same name.
#[derive(Default)]
pub struct Reservations {
    taken: Mutex<HashSet<String>>,
}

fn key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

impl Reservations {
    pub fn reserve(&self, input: &Path, extension: &str, suffix: Option<&str>) -> PathBuf {
        let mut taken = self.taken.lock().unwrap();
        let out = naming::output_path(input, extension, suffix, &|p| {
            p.exists() || taken.contains(&key(p))
        });
        taken.insert(key(&out));
        out
    }

    pub fn release(&self, path: &Path) {
        self.taken.lock().unwrap().remove(&key(path));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finalize_never_replaces() {
        let dir = tempfile::tempdir().unwrap();
        let (tmp, fin) = (
            dir.path().join("a.jpg.swisswheel-tmp"),
            dir.path().join("a.jpg"),
        );
        std::fs::write(&tmp, "new").unwrap();
        std::fs::write(&fin, "old").unwrap();
        assert_eq!(
            finalize(&tmp, &fin).unwrap_err().kind(),
            std::io::ErrorKind::AlreadyExists
        );
        assert_eq!(std::fs::read_to_string(&fin).unwrap(), "old");
        std::fs::remove_file(&fin).unwrap();
        finalize(&tmp, &fin).unwrap();
        assert_eq!(std::fs::read_to_string(&fin).unwrap(), "new");
        assert!(!tmp.exists());
    }

    #[test]
    fn reservations_hand_out_distinct_paths() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("a.png");
        std::fs::write(&input, "x").unwrap();
        let r = Reservations::default();
        let first = r.reserve(&input, "jpg", None);
        let second = r.reserve(&input, "jpg", None);
        assert_eq!(first, dir.path().join("a.jpg"));
        assert_eq!(second, dir.path().join("a (1).jpg"));
        r.release(&first);
        assert_eq!(r.reserve(&input, "jpg", None), dir.path().join("a.jpg"));
    }
}
