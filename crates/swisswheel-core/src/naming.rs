//! Output file names: next to the input, never overwriting anything.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub const TEMP_SUFFIX: &str = ".swisswheel-tmp";

/// `input`'s folder + `stem[ (suffix)][ (n)].extension`, counting `n` up from 1
/// while the candidate `exists` or equals `input` (ignoring case).
pub fn output_path(
    input: &Path,
    extension: &str,
    suffix: Option<&str>,
    exists: &dyn Fn(&Path) -> bool,
) -> PathBuf {
    let dir = input.parent().unwrap_or(Path::new(""));
    let mut base = input.file_stem().map(OsString::from).unwrap_or_default();
    if let Some(suffix) = suffix {
        base.push(format!(" ({suffix})"));
    }
    let input_lower = input.to_string_lossy().to_lowercase();
    (0u32..)
        .map(|n| {
            let mut name = base.clone();
            if n > 0 {
                name.push(format!(" ({n})"));
            }
            name.push(".");
            name.push(extension);
            dir.join(name)
        })
        .find(|candidate| {
            !exists(candidate) && candidate.to_string_lossy().to_lowercase() != input_lower
        })
        .expect("u32 range exhausted")
}

/// Where a job writes before renaming to `final_path` on success.
pub fn temp_path(final_path: &Path) -> PathBuf {
    let mut name = final_path.as_os_str().to_owned();
    name.push(TEMP_SUFFIX);
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn none(_: &Path) -> bool {
        false
    }

    #[test]
    fn basic_conversion_and_suffix() {
        assert_eq!(
            output_path(Path::new(r"C:\p\photo.png"), "jpg", None, &none),
            PathBuf::from(r"C:\p\photo.jpg")
        );
        assert_eq!(
            output_path(
                Path::new(r"C:\p\photo.jpg"),
                "jpg",
                Some("compressed"),
                &none
            ),
            PathBuf::from(r"C:\p\photo (compressed).jpg")
        );
    }

    #[test]
    fn collisions_count_up() {
        let taken = [
            PathBuf::from(r"C:\p\photo.jpg"),
            PathBuf::from(r"C:\p\photo (1).jpg"),
        ];
        let exists = |p: &Path| taken.iter().any(|t| t == p);
        assert_eq!(
            output_path(Path::new(r"C:\p\photo.png"), "jpg", None, &exists),
            PathBuf::from(r"C:\p\photo (2).jpg")
        );
        let taken2 = [PathBuf::from(r"C:\p\v (muted).mp4")];
        let exists2 = |p: &Path| taken2.iter().any(|t| t == p);
        assert_eq!(
            output_path(Path::new(r"C:\p\v.mp4"), "mp4", Some("muted"), &exists2),
            PathBuf::from(r"C:\p\v (muted) (1).mp4")
        );
    }

    #[test]
    fn never_returns_input_path() {
        // same name differing only in case counts as the input
        let out = output_path(Path::new(r"C:\p\clip.MP3"), "mp3", None, &none);
        assert_eq!(out, PathBuf::from(r"C:\p\clip (1).mp3"));
    }

    #[test]
    fn tricky_names() {
        assert_eq!(
            output_path(Path::new(r"C:\p\report.final.png"), "webp", None, &none),
            PathBuf::from(r"C:\p\report.final.webp")
        );
        assert_eq!(
            output_path(Path::new(r"C:\p\README"), "txt", None, &none),
            PathBuf::from(r"C:\p\README.txt")
        );
        assert_eq!(
            output_path(
                Path::new(r"D:\사진\여름 휴가.png"),
                "jpg",
                Some("50%"),
                &none
            ),
            PathBuf::from(r"D:\사진\여름 휴가 (50%).jpg")
        );
        let taken = [PathBuf::from(r"C:\p\a (1).jpg")];
        let exists = |p: &Path| taken.iter().any(|t| t == p);
        // "a (1).png" -> "a (1).jpg" taken -> "a (1) (1).jpg" (existing numbers are not parsed)
        assert_eq!(
            output_path(Path::new(r"C:\p\a (1).png"), "jpg", None, &exists),
            PathBuf::from(r"C:\p\a (1) (1).jpg")
        );
    }

    #[test]
    fn temp_path_appends_suffix() {
        assert_eq!(
            temp_path(Path::new(r"C:\p\photo.jpg")),
            PathBuf::from(r"C:\p\photo.jpg.swisswheel-tmp")
        );
    }
}
