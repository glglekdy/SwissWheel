//! File formats SwissWheel understands.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaKind {
    Image,
    Video,
    Audio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    Jpg,
    Png,
    Webp,
    Bmp,
    Tiff,
    Gif,
    Ico,
    Mp4,
    Mkv,
    Webm,
    Mov,
    Avi,
    Wmv,
    Mp3,
    M4a,
    Wav,
    Flac,
    Ogg,
    Opus,
    Wma,
    Aiff,
}

impl Format {
    const ALL: [Format; 21] = [
        Format::Jpg,
        Format::Png,
        Format::Webp,
        Format::Bmp,
        Format::Tiff,
        Format::Gif,
        Format::Ico,
        Format::Mp4,
        Format::Mkv,
        Format::Webm,
        Format::Mov,
        Format::Avi,
        Format::Wmv,
        Format::Mp3,
        Format::M4a,
        Format::Wav,
        Format::Flac,
        Format::Ogg,
        Format::Opus,
        Format::Wma,
        Format::Aiff,
    ];

    /// Case-insensitive; accepts the aliases `jpeg`, `tif`, and `aif`.
    pub fn from_extension(ext: &str) -> Option<Format> {
        let ext = ext.to_ascii_lowercase();
        let ext = match ext.as_str() {
            "jpeg" => "jpg",
            "tif" => "tiff",
            "aif" => "aiff",
            other => other,
        };
        Self::ALL.into_iter().find(|f| f.extension() == ext)
    }

    pub fn from_path(path: &Path) -> Option<Format> {
        Self::from_extension(path.extension()?.to_str()?)
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Jpg => "jpg",
            Format::Png => "png",
            Format::Webp => "webp",
            Format::Bmp => "bmp",
            Format::Tiff => "tiff",
            Format::Gif => "gif",
            Format::Ico => "ico",
            Format::Mp4 => "mp4",
            Format::Mkv => "mkv",
            Format::Webm => "webm",
            Format::Mov => "mov",
            Format::Avi => "avi",
            Format::Wmv => "wmv",
            Format::Mp3 => "mp3",
            Format::M4a => "m4a",
            Format::Wav => "wav",
            Format::Flac => "flac",
            Format::Ogg => "ogg",
            Format::Opus => "opus",
            Format::Wma => "wma",
            Format::Aiff => "aiff",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Format::Jpg => "JPG",
            Format::Png => "PNG",
            Format::Webp => "WEBP",
            Format::Bmp => "BMP",
            Format::Tiff => "TIFF",
            Format::Gif => "GIF",
            Format::Ico => "ICO",
            Format::Mp4 => "MP4",
            Format::Mkv => "MKV",
            Format::Webm => "WEBM",
            Format::Mov => "MOV",
            Format::Avi => "AVI",
            Format::Wmv => "WMV",
            Format::Mp3 => "MP3",
            Format::M4a => "M4A",
            Format::Wav => "WAV",
            Format::Flac => "FLAC",
            Format::Ogg => "OGG",
            Format::Opus => "OPUS",
            Format::Wma => "WMA",
            Format::Aiff => "AIFF",
        }
    }

    pub fn kind(self) -> MediaKind {
        use Format::*;
        match self {
            Jpg | Png | Webp | Bmp | Tiff | Gif | Ico => MediaKind::Image,
            Mp4 | Mkv | Webm | Mov | Avi | Wmv => MediaKind::Video,
            Mp3 | M4a | Wav | Flac | Ogg | Opus | Wma | Aiff => MediaKind::Audio,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_extension_is_case_insensitive_and_aliased() {
        assert_eq!(Format::from_extension("JPG"), Some(Format::Jpg));
        assert_eq!(Format::from_extension("jpeg"), Some(Format::Jpg));
        assert_eq!(Format::from_extension("Tif"), Some(Format::Tiff));
        assert_eq!(Format::from_extension("aif"), Some(Format::Aiff));
        assert_eq!(Format::from_extension("opus"), Some(Format::Opus));
        assert_eq!(Format::from_extension("txt"), None);
        assert_eq!(Format::from_extension(""), None);
    }

    #[test]
    fn from_path_uses_last_extension() {
        use std::path::Path;
        assert_eq!(
            Format::from_path(Path::new(r"C:\x\report.final.PNG")),
            Some(Format::Png)
        );
        assert_eq!(Format::from_path(Path::new(r"C:\x\README")), None);
        assert_eq!(
            Format::from_path(Path::new(r"C:\x\사진.webp")),
            Some(Format::Webp)
        );
    }

    #[test]
    fn extension_label_and_kind() {
        assert_eq!(Format::Jpg.extension(), "jpg");
        assert_eq!(Format::Tiff.extension(), "tiff");
        assert_eq!(Format::M4a.label(), "M4A");
        assert_eq!(Format::Gif.kind(), MediaKind::Image); // animated GIFs are classified by catalog::InputItem
        assert_eq!(Format::Wmv.kind(), MediaKind::Video);
        assert_eq!(Format::Wma.kind(), MediaKind::Audio);
    }
}
