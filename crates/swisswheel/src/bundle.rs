//! "폴더로 묶기": puts the dragged items into a new folder beside the first.
//! Items may be placed from several workers at once; the folder is made by
//! whichever gets there first.

use std::ffi::OsStr;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use swisswheel_core::naming::{NEW_FOLDER, TEMP_SUFFIX, free_path};
use swisswheel_core::options::Transfer;

use crate::fsutil::{finalize, move_no_replace};

/// Tries per item before giving up on a free name another program keeps taking.
const ATTEMPTS: usize = 10;

pub struct Bundle {
    /// Where the new folder goes: the first item's folder.
    parent: PathBuf,
    /// Made on first use, so a job cancelled before it starts leaves nothing.
    folder: OnceLock<Result<PathBuf, String>>,
}

impl Bundle {
    pub fn new(first_input: &Path) -> Bundle {
        Bundle {
            parent: first_input.parent().unwrap_or(Path::new("")).to_path_buf(),
            folder: OnceLock::new(),
        }
    }

    /// Moves or copies `input` into the folder under a name nothing there has,
    /// returning where it went. `index` is the item's place in the job; it
    /// keeps temp names apart. A failed item stays where it was, and a failed
    /// copy leaves nothing behind.
    pub fn place(&self, index: usize, input: &Path, transfer: Transfer) -> Result<PathBuf, String> {
        let folder = self
            .folder
            .get_or_init(|| make_folder(&self.parent))
            .clone()?;
        let name = input.file_name().unwrap_or(input.as_os_str());
        let fail = |e: std::io::Error| {
            let shown = name.to_string_lossy();
            log_error!("폴더로 묶기: {}: {e}", input.display());
            match transfer {
                Transfer::Move => format!("{shown}: 옮길 수 없습니다"),
                Transfer::Copy => format!("{shown}: 복사할 수 없습니다"),
            }
        };
        let is_file = std::fs::symlink_metadata(input).map_err(fail)?.is_file();
        match transfer {
            Transfer::Move => move_in(input, &folder, name, is_file, index),
            Transfer::Copy => copy_in(input, &folder, name, is_file, index),
        }
        .map_err(fail)
    }

    /// Removes the folder if nothing went into it (every item failed).
    pub fn finish(&self) {
        if let Some(Ok(folder)) = self.folder.get() {
            // Fails, leaving it, when anything is inside.
            let _ = std::fs::remove_dir(folder);
        }
    }
}

fn make_folder(parent: &Path) -> Result<PathBuf, String> {
    let mut last = None;
    for _ in 0..ATTEMPTS {
        let folder = free_path(parent, OsStr::new(NEW_FOLDER), false, &|p| p.exists());
        match std::fs::create_dir(&folder) {
            Ok(()) => return Ok(folder),
            // Made by someone else meanwhile; take the next number.
            Err(e) if e.kind() == ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => {
                last = Some(e);
                break;
            }
        }
    }
    if let Some(e) = last {
        log_error!(
            "폴더로 묶기: {}에 새 폴더를 만들 수 없습니다: {e}",
            parent.display()
        );
    }
    Err("새 폴더를 만들 수 없습니다".to_string())
}

/// Moves `input` into `folder`. A folder on another drive is copied, then the
/// original deleted once the copy is complete.
fn move_in(
    input: &Path,
    folder: &Path,
    name: &OsStr,
    is_file: bool,
    index: usize,
) -> std::io::Result<PathBuf> {
    let mut last = None;
    for _ in 0..ATTEMPTS {
        let target = free_path(folder, name, is_file, &|p| p.exists());
        match move_no_replace(input, &target) {
            Ok(()) => return Ok(target),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => last = Some(e),
            Err(e) if e.kind() == ErrorKind::CrossesDevices && !is_file => {
                let target = copy_in(input, folder, name, is_file, index)?;
                if let Err(e) = std::fs::remove_dir_all(input) {
                    log_error!(
                        "폴더로 묶기: 복사한 원본을 지우지 못했습니다 ({}): {e}",
                        input.display()
                    );
                }
                return Ok(target);
            }
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| ErrorKind::AlreadyExists.into()))
}

/// Copies `input` into `folder` through a temp name, so a copy that fails
/// halfway is removed rather than left looking complete.
fn copy_in(
    input: &Path,
    folder: &Path,
    name: &OsStr,
    is_file: bool,
    index: usize,
) -> std::io::Result<PathBuf> {
    let temp = folder.join(format!("{index}{TEMP_SUFFIX}"));
    let copied = if is_file {
        std::fs::copy(input, &temp).map(drop)
    } else {
        copy_tree(input, &temp)
    };
    let remove_temp = || {
        let _ = if is_file {
            std::fs::remove_file(&temp)
        } else {
            std::fs::remove_dir_all(&temp)
        };
    };
    if let Err(e) = copied {
        remove_temp();
        return Err(e);
    }
    let mut last = None;
    for _ in 0..ATTEMPTS {
        let target = free_path(folder, name, is_file, &|p| p.exists());
        match finalize(&temp, &target) {
            Ok(()) => return Ok(target),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => {
                last = Some(e);
                break;
            }
        }
    }
    remove_temp();
    Err(last.unwrap_or_else(|| ErrorKind::AlreadyExists.into()))
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}
