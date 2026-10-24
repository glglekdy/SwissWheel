//! Which wheel items to offer for a dragged selection.

use crate::format::{Format, MediaKind};

/// One dragged item, as far as the catalog needs to know it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputItem {
    Folder,
    File {
        format: Option<Format>,
        animated_gif: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WheelKind {
    Convert,
    Tools,
    Actions,
}

impl WheelKind {
    const CYCLE: [WheelKind; 3] = [WheelKind::Convert, WheelKind::Tools, WheelKind::Actions];

    pub fn label(self) -> &'static str {
        match self {
            WheelKind::Convert => "변환",
            WheelKind::Tools => "도구",
            WheelKind::Actions => "동작",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tool {
    Shrink,
    CompressImage,
    CompressVideo,
    ExtractAudio,
    Mute,
    /// Puts the dragged items into a new folder beside them.
    Bundle,
}

impl Tool {
    pub fn label(self) -> &'static str {
        match self {
            Tool::Shrink => "축소",
            Tool::CompressImage => "압축",
            Tool::CompressVideo => "영상 압축",
            Tool::ExtractAudio => "오디오 추출",
            Tool::Mute => "음소거",
            Tool::Bundle => "폴더로 묶기",
        }
    }

    /// Appended to the output file name as `name (suffix).ext`.
    pub fn suffix(self) -> Option<&'static str> {
        match self {
            Tool::Shrink => Some("50%"),
            Tool::CompressImage | Tool::CompressVideo => Some("compressed"),
            Tool::ExtractAudio | Tool::Bundle => None,
            Tool::Mute => Some("muted"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Share,
    Copy,
    Cut,
    CopyPath,
    Terminal,
}

impl Action {
    const ALL: [Action; 5] = [
        Action::Share,
        Action::Copy,
        Action::Cut,
        Action::CopyPath,
        Action::Terminal,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Action::Share => "공유",
            Action::Copy => "복사",
            Action::Cut => "잘라내기",
            Action::CopyPath => "경로 복사",
            Action::Terminal => "터미널",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operation {
    Convert(Format),
    Tool(Tool),
    Action(Action),
}

impl Operation {
    pub fn label(&self) -> &'static str {
        match self {
            Operation::Convert(f) => f.label(),
            Operation::Tool(t) => t.label(),
            Operation::Action(a) => a.label(),
        }
    }
}

const IMAGE_TARGETS: [Format; 7] = [
    Format::Jpg,
    Format::Png,
    Format::Webp,
    Format::Bmp,
    Format::Tiff,
    Format::Gif,
    Format::Ico,
];
pub(crate) const VIDEO_TARGETS: [Format; 7] = [
    Format::Mp4,
    Format::Mkv,
    Format::Webm,
    Format::Mov,
    Format::Avi,
    Format::Gif,
    Format::Mp3,
];
pub(crate) const ANIMATED_GIF_TARGETS: [Format; 2] = [Format::Mp4, Format::Webm];
pub(crate) const AUDIO_TARGETS: [Format; 6] = [
    Format::Mp3,
    Format::M4a,
    Format::Wav,
    Format::Flac,
    Format::Ogg,
    Format::Opus,
];

/// Items for `wheel`, in fixed display order. For several inputs this is the
/// intersection of each input's items.
pub fn items_for(wheel: WheelKind, inputs: &[InputItem]) -> Vec<Operation> {
    let Some((first, rest)) = inputs.split_first() else {
        return Vec::new();
    };
    let mut items = items_for_one(wheel, first);
    for input in rest {
        let other = items_for_one(wheel, input);
        items.retain(|op| other.contains(op));
    }
    // Any two or more items can go into a new folder, last in the Tools wheel.
    if wheel == WheelKind::Tools && inputs.len() >= 2 {
        items.push(Operation::Tool(Tool::Bundle));
    }
    items
}

/// The first wheel (Convert, Tools, Actions) that has items.
pub fn initial_wheel(inputs: &[InputItem]) -> Option<WheelKind> {
    WheelKind::CYCLE
        .into_iter()
        .find(|&w| !items_for(w, inputs).is_empty())
}

/// The next wheel after `current` that has items, wrapping around; `current`
/// itself if no other wheel has any.
pub fn next_wheel(current: WheelKind, inputs: &[InputItem]) -> WheelKind {
    let start = WheelKind::CYCLE.iter().position(|&w| w == current).unwrap();
    (1..=3)
        .map(|i| WheelKind::CYCLE[(start + i) % 3])
        .find(|&w| !items_for(w, inputs).is_empty())
        .unwrap_or(current)
}

fn items_for_one(wheel: WheelKind, input: &InputItem) -> Vec<Operation> {
    if wheel == WheelKind::Actions {
        return Action::ALL.map(Operation::Action).to_vec();
    }
    let InputItem::File {
        format: Some(format),
        animated_gif,
    } = *input
    else {
        return Vec::new();
    };
    let animated = animated_gif && format == Format::Gif;
    match wheel {
        WheelKind::Convert => {
            let targets: &[Format] = if animated {
                &ANIMATED_GIF_TARGETS
            } else {
                match format.kind() {
                    MediaKind::Image => &IMAGE_TARGETS,
                    MediaKind::Video => &VIDEO_TARGETS,
                    MediaKind::Audio => &AUDIO_TARGETS,
                }
            };
            targets
                .iter()
                .filter(|&&t| t != format)
                .map(|&t| Operation::Convert(t))
                .collect()
        }
        WheelKind::Tools => {
            let tools: &[Tool] = if animated {
                &[]
            } else {
                match (format.kind(), format) {
                    (MediaKind::Image, Format::Jpg | Format::Png | Format::Webp) => {
                        &[Tool::Shrink, Tool::CompressImage]
                    }
                    (MediaKind::Image, _) => &[Tool::Shrink],
                    (MediaKind::Video, _) => &[Tool::CompressVideo, Tool::ExtractAudio, Tool::Mute],
                    (MediaKind::Audio, _) => &[],
                }
            };
            tools.iter().map(|&t| Operation::Tool(t)).collect()
        }
        WheelKind::Actions => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::Format;

    fn file(f: Format) -> InputItem {
        InputItem::File {
            format: Some(f),
            animated_gif: false,
        }
    }

    #[test]
    fn single_image_convert_excludes_self() {
        use Format::*;
        assert_eq!(
            items_for(WheelKind::Convert, &[file(Png)]),
            [Jpg, Webp, Bmp, Tiff, Gif, Ico]
                .map(Operation::Convert)
                .to_vec()
        );
    }

    #[test]
    fn video_audio_and_animated_gif_convert() {
        use Format::*;
        assert_eq!(
            items_for(WheelKind::Convert, &[file(Mkv)]),
            [Mp4, Webm, Mov, Avi, Gif, Mp3]
                .map(Operation::Convert)
                .to_vec()
        );
        assert_eq!(
            items_for(WheelKind::Convert, &[file(Wma)]),
            [Mp3, M4a, Wav, Flac, Ogg, Opus]
                .map(Operation::Convert)
                .to_vec()
        );
        let anim = InputItem::File {
            format: Some(Gif),
            animated_gif: true,
        };
        assert_eq!(
            items_for(WheelKind::Convert, &[anim]),
            [Mp4, Webm].map(Operation::Convert).to_vec()
        );
        assert!(items_for(WheelKind::Tools, &[anim]).is_empty());
    }

    #[test]
    fn tools_per_kind() {
        use Format::*;
        assert_eq!(
            items_for(WheelKind::Tools, &[file(Bmp)]),
            vec![Operation::Tool(Tool::Shrink)]
        );
        assert_eq!(
            items_for(WheelKind::Tools, &[file(Jpg)]),
            vec![
                Operation::Tool(Tool::Shrink),
                Operation::Tool(Tool::CompressImage)
            ]
        );
        assert_eq!(
            items_for(WheelKind::Tools, &[file(Mp4)]),
            [Tool::CompressVideo, Tool::ExtractAudio, Tool::Mute]
                .map(Operation::Tool)
                .to_vec()
        );
        assert!(items_for(WheelKind::Tools, &[file(Flac)]).is_empty());
    }

    #[test]
    fn mixed_and_unknown_selections() {
        use Format::*;
        // same format several times
        assert_eq!(
            items_for(WheelKind::Convert, &[file(Png), file(Png)]),
            items_for(WheelKind::Convert, &[file(Png)])
        );
        // different images -> intersection
        assert_eq!(
            items_for(WheelKind::Convert, &[file(Png), file(Jpg)]),
            [Webp, Bmp, Tiff, Gif, Ico].map(Operation::Convert).to_vec()
        );
        // image + audio -> only actions
        let mixed = [file(Png), file(Mp3)];
        assert!(items_for(WheelKind::Convert, &mixed).is_empty());
        assert_eq!(
            items_for(WheelKind::Tools, &mixed),
            vec![Operation::Tool(Tool::Bundle)]
        );
        assert_eq!(items_for(WheelKind::Actions, &mixed).len(), 5);
        // folder included, unknown extension
        assert!(items_for(WheelKind::Convert, &[file(Png), InputItem::Folder]).is_empty());
        let unknown = InputItem::File {
            format: None,
            animated_gif: false,
        };
        assert!(items_for(WheelKind::Tools, &[unknown]).is_empty());
        assert_eq!(initial_wheel(&[unknown]), Some(WheelKind::Actions));
        // empty input
        assert!(items_for(WheelKind::Actions, &[]).is_empty());
        assert_eq!(initial_wheel(&[]), None);
    }

    #[test]
    fn bundle_for_two_or_more_of_anything() {
        use Format::*;
        let bundle = Operation::Tool(Tool::Bundle);
        let tools = |inputs: &[InputItem]| items_for(WheelKind::Tools, inputs);
        assert_eq!(tools(&[file(Png), file(Jpg)]).last(), Some(&bundle));
        assert_eq!(tools(&[file(Png), file(Mp3)]), vec![bundle]);
        assert_eq!(tools(&[InputItem::Folder, InputItem::Folder]), vec![bundle]);
        assert!(!tools(&[file(Png)]).contains(&bundle));
        assert!(!tools(&[InputItem::Folder]).contains(&bundle));
        assert!(!items_for(WheelKind::Convert, &[file(Png), file(Jpg)]).contains(&bundle));
        // a mixed selection now has a Tools wheel
        assert_eq!(
            next_wheel(WheelKind::Actions, &[file(Png), file(Mp3)]),
            WheelKind::Tools
        );
        assert_eq!(Tool::Bundle.label(), "폴더로 묶기");
        assert_eq!(Tool::Bundle.suffix(), None);
    }

    #[test]
    fn wheel_cycling_skips_empty() {
        use Format::*;
        assert_eq!(initial_wheel(&[file(Png)]), Some(WheelKind::Convert));
        assert_eq!(
            next_wheel(WheelKind::Convert, &[file(Png)]),
            WheelKind::Tools
        );
        assert_eq!(
            next_wheel(WheelKind::Actions, &[file(Png)]),
            WheelKind::Convert
        );
        // FLAC has no tools -> Convert is followed by Actions
        assert_eq!(
            next_wheel(WheelKind::Convert, &[file(Flac)]),
            WheelKind::Actions
        );
        // folder: actions only -> cycles back to itself
        assert_eq!(
            next_wheel(WheelKind::Actions, &[InputItem::Folder]),
            WheelKind::Actions
        );
        assert_eq!(next_wheel(WheelKind::Tools, &[]), WheelKind::Tools);
    }

    #[test]
    fn labels() {
        assert_eq!(WheelKind::Tools.label(), "도구");
        assert_eq!(Operation::Convert(Format::Webp).label(), "WEBP");
        assert_eq!(Operation::Action(Action::CopyPath).label(), "경로 복사");
        assert_eq!(Tool::Mute.suffix(), Some("muted"));
        assert_eq!(Tool::ExtractAudio.suffix(), None);
    }
}
