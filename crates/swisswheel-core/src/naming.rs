//! Output file names: next to the input, never overwriting anything.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use crate::catalog::{Operation, Tool};
use crate::options::Preset;

pub const TEMP_SUFFIX: &str = ".swisswheel-tmp";

/// The `(suffix)` for an operation's output name, including the chosen
/// option where it changes what the file is (`사진 (25%).png`, `영상 (720p).mp4`).
pub fn suffix_for(op: &Operation, preset: Option<Preset>) -> Option<String> {
    match (op, preset) {
        (Operation::Tool(Tool::Shrink), Some(Preset::Scale(p))) => Some(format!("{p}%")),
        (Operation::Convert(_), Some(Preset::Resolution(Some(h)))) => Some(format!("{h}p")),
        (Operation::Tool(tool), _) => tool.suffix().map(str::to_string),
        _ => None,
    }
}

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

/// The folder "폴더로 묶기" makes; numbered like any other result.
pub const NEW_FOLDER: &str = "새 폴더";

/// `dir` + `name`, or `stem (n).extension` counting `n` up from 1 while the
/// candidate `exists`. A folder's `name` (`is_file` false) has no extension,
/// so the number goes at its end: `v1.2 (1)`.
pub fn free_path(
    dir: &Path,
    name: &OsStr,
    is_file: bool,
    exists: &dyn Fn(&Path) -> bool,
) -> PathBuf {
    let as_path = Path::new(name);
    let (stem, extension) = match (is_file, as_path.file_stem(), as_path.extension()) {
        (true, Some(stem), Some(extension)) => (stem, Some(extension)),
        _ => (name, None),
    };
    (0u32..)
        .map(|n| {
            let mut candidate = stem.to_owned();
            if n > 0 {
                candidate.push(format!(" ({n})"));
            }
            if let Some(extension) = extension {
                candidate.push(".");
                candidate.push(extension);
            }
            dir.join(candidate)
        })
        .find(|candidate| !exists(candidate))
        .expect("u32 range exhausted")
}

/// The `(suffix)` of a saved video frame: `frame 0m12.40s`, from one hour `frame 1h02m03.40s`.
pub fn frame_suffix(t: f64) -> String {
    let hundredths = (t.max(0.0) * 100.0).round() as u64;
    let (h, m, s, c) = (
        hundredths / 360_000,
        hundredths / 6_000 % 60,
        hundredths / 100 % 60,
        hundredths % 100,
    );
    if h > 0 {
        format!("frame {h}h{m:02}m{s:02}.{c:02}s")
    } else {
        format!("frame {m}m{s:02}.{c:02}s")
    }
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

    #[test]
    fn frame_suffixes() {
        assert_eq!(frame_suffix(12.4), "frame 0m12.40s");
        assert_eq!(frame_suffix(59.999), "frame 1m00.00s");
        assert_eq!(frame_suffix(3723.4), "frame 1h02m03.40s");
    }

    fn none(_: &Path) -> bool {
        false
    }

    #[test]
    fn free_paths_count_up() {
        let taken = |p: &Path| {
            [
                r"C:\p\새 폴더",
                r"C:\p\a.png",
                r"C:\p\a (1).png",
                r"C:\p\v1.2",
            ]
            .iter()
            .any(|t| Path::new(t) == p)
        };
        let free =
            |name: &str, is_file| free_path(Path::new(r"C:\p"), OsStr::new(name), is_file, &taken);
        assert_eq!(free(NEW_FOLDER, false), PathBuf::from(r"C:\p\새 폴더 (1)"));
        assert_eq!(free("a.png", true), PathBuf::from(r"C:\p\a (2).png"));
        assert_eq!(free("b.png", true), PathBuf::from(r"C:\p\b.png"));
        // a folder's name keeps its dot: no extension to put the number before
        assert_eq!(free("v1.2", false), PathBuf::from(r"C:\p\v1.2 (1)"));
        assert_eq!(free("README", true), PathBuf::from(r"C:\p\README"));
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
    fn suffixes_carry_the_chosen_option() {
        use crate::catalog::{Operation, Tool};
        use crate::format::Format;
        use crate::options::{Preset, Strength};
        let shrink = Operation::Tool(Tool::Shrink);
        assert_eq!(
            suffix_for(&shrink, Some(Preset::Scale(25))).as_deref(),
            Some("25%")
        );
        assert_eq!(suffix_for(&shrink, None).as_deref(), Some("50%"));
        assert_eq!(
            suffix_for(
                &Operation::Convert(Format::Mp4),
                Some(Preset::Resolution(Some(720)))
            )
            .as_deref(),
            Some("720p")
        );
        assert_eq!(
            suffix_for(
                &Operation::Convert(Format::Mp4),
                Some(Preset::Resolution(None))
            ),
            None
        );
        assert_eq!(
            suffix_for(
                &Operation::Tool(Tool::CompressVideo),
                Some(Preset::Compress(Strength::Strong))
            )
            .as_deref(),
            Some("compressed")
        );
        assert_eq!(
            suffix_for(
                &Operation::Convert(Format::Gif),
                Some(Preset::Gif {
                    fps: 10,
                    max_width: Some(320)
                })
            ),
            None
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
