//! Classifying dragged paths for the catalog (folder, format, animated GIF).

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use image::AnimationDecoder;
use image::codecs::gif::GifDecoder;
use swisswheel_core::catalog::InputItem;
use swisswheel_core::format::Format;

pub fn input_item(path: &Path) -> InputItem {
    if path.is_dir() {
        return InputItem::Folder;
    }
    let format = Format::from_path(path);
    let animated_gif = format == Some(Format::Gif) && is_animated_gif(path);
    InputItem::File {
        format,
        animated_gif,
    }
}

/// Decodes at most two frames; an unreadable GIF counts as a still image.
fn is_animated_gif(path: &Path) -> bool {
    let Ok(file) = File::open(path) else {
        return false;
    };
    let Ok(decoder) = GifDecoder::new(BufReader::new(file)) else {
        return false;
    };
    decoder.into_frames().take(2).filter(Result::is_ok).count() >= 2
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::Path;
    use swisswheel_core::catalog::InputItem;
    use swisswheel_core::format::Format;

    /// Writes a 4x4 GIF with `frames` frames.
    pub(crate) fn write_gif(path: &Path, frames: usize) {
        use image::codecs::gif::{GifEncoder, Repeat};
        use image::{Delay, Frame, RgbaImage};
        let file = std::fs::File::create(path).unwrap();
        let mut enc = GifEncoder::new(file);
        enc.set_repeat(Repeat::Infinite).unwrap();
        for i in 0..frames {
            let img = RgbaImage::from_pixel(4, 4, image::Rgba([(i * 80) as u8, 0, 0, 255]));
            enc.encode_frame(Frame::from_parts(
                img,
                0,
                0,
                Delay::from_numer_denom_ms(100, 1),
            ))
            .unwrap();
        }
    }

    #[test]
    fn probe_classifies_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let still = dir.path().join("still.gif");
        let anim = dir.path().join("anim.GIF");
        write_gif(&still, 1);
        write_gif(&anim, 2);
        std::fs::write(dir.path().join("notes.txt"), "x").unwrap();
        assert_eq!(input_item(dir.path()), InputItem::Folder);
        assert_eq!(
            input_item(&still),
            InputItem::File {
                format: Some(Format::Gif),
                animated_gif: false
            }
        );
        assert_eq!(
            input_item(&anim),
            InputItem::File {
                format: Some(Format::Gif),
                animated_gif: true
            }
        );
        assert_eq!(
            input_item(&dir.path().join("notes.txt")),
            InputItem::File {
                format: None,
                animated_gif: false
            }
        );
    }

    #[test]
    fn probe_handles_unicode_paths() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("여름 사진");
        std::fs::create_dir(&sub).unwrap();
        let p = sub.join("바다 1.gif");
        write_gif(&p, 2);
        assert_eq!(
            input_item(&p),
            InputItem::File {
                format: Some(Format::Gif),
                animated_gif: true
            }
        );
    }
}
