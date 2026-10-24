//! Which icon each wheel item shows: a code point in the system icon font
//! (Segoe Fluent Icons on Windows 11, Segoe MDL2 Assets on Windows 10; both
//! use the same code points for these icons).

use swisswheel_core::catalog::{Action, Operation, Tool};
use swisswheel_core::format::{Format, MediaKind};

pub fn glyph(op: &Operation) -> char {
    match op {
        Operation::Convert(Format::Gif) => '\u{E786}', // Slideshow
        Operation::Convert(Format::Ico) => '\u{ECAA}', // AppIconDefault
        Operation::Convert(f) => match f.kind() {
            MediaKind::Image => '\u{E91B}', // Photo
            MediaKind::Video => '\u{E714}', // Video
            MediaKind::Audio => '\u{E8D6}', // Audio
        },
        Operation::Tool(t) => match t {
            Tool::Shrink => '\u{E73F}', // BackToWindow (arrows pointing in)
            Tool::CompressImage | Tool::CompressVideo => '\u{F012}', // ZipFolder
            Tool::ExtractAudio => '\u{E767}', // Volume
            Tool::Mute => '\u{E74F}',   // Mute
            Tool::Edit => '\u{E70F}',   // Edit
        },
        Operation::Action(a) => match a {
            Action::Share => '\u{E72D}',
            Action::Copy => '\u{E8C8}',
            Action::Cut => '\u{E8C6}',
            Action::CopyPath => '\u{E71B}', // Link
            Action::Terminal => '\u{E756}', // CommandPrompt
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FORMATS: [Format; 21] = [
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
    const TOOLS: [Tool; 6] = [
        Tool::Shrink,
        Tool::CompressImage,
        Tool::CompressVideo,
        Tool::ExtractAudio,
        Tool::Mute,
        Tool::Edit,
    ];
    const ACTIONS: [Action; 5] = [
        Action::Share,
        Action::Copy,
        Action::Cut,
        Action::CopyPath,
        Action::Terminal,
    ];

    fn all() -> Vec<Operation> {
        FORMATS
            .map(Operation::Convert)
            .into_iter()
            .chain(TOOLS.map(Operation::Tool))
            .chain(ACTIONS.map(Operation::Action))
            .collect()
    }

    #[test]
    fn every_item_uses_the_icon_font() {
        for op in all() {
            let g = glyph(&op) as u32;
            assert!(
                (0xE000..=0xF8FF).contains(&g),
                "{op:?} → U+{g:04X} is outside the private use area"
            );
        }
    }

    #[test]
    fn formats_of_one_kind_share_an_icon() {
        let icon = |f| glyph(&Operation::Convert(f));
        for f in [Format::Png, Format::Webp, Format::Bmp, Format::Tiff] {
            assert_eq!(icon(f), icon(Format::Jpg), "{f:?}");
        }
        for f in [Format::Mkv, Format::Webm, Format::Mov, Format::Avi] {
            assert_eq!(icon(f), icon(Format::Mp4), "{f:?}");
        }
        for f in [Format::M4a, Format::Wav, Format::Flac, Format::Ogg] {
            assert_eq!(icon(f), icon(Format::Mp3), "{f:?}");
        }
        // GIF and ICO stand out from still images; the three kinds differ.
        assert_ne!(icon(Format::Gif), icon(Format::Jpg));
        assert_ne!(icon(Format::Ico), icon(Format::Jpg));
        assert_ne!(icon(Format::Jpg), icon(Format::Mp4));
        assert_ne!(icon(Format::Mp4), icon(Format::Mp3));
    }

    #[test]
    fn tools_and_actions_are_told_apart() {
        let tool = |t| glyph(&Operation::Tool(t));
        assert_eq!(tool(Tool::CompressImage), tool(Tool::CompressVideo));
        assert_ne!(tool(Tool::ExtractAudio), tool(Tool::Mute));
        assert_ne!(tool(Tool::Shrink), tool(Tool::CompressImage));
        assert_eq!(tool(Tool::Edit), '\u{E70F}');
        let mut actions: Vec<char> = ACTIONS
            .iter()
            .map(|&a| glyph(&Operation::Action(a)))
            .collect();
        actions.sort_unstable();
        actions.dedup();
        assert_eq!(actions.len(), ACTIONS.len(), "each action has its own icon");
    }
}
