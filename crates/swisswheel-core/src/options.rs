//! Option sets: the choices the side panel offers for some wheel items
//! (spec 2.5). Dropping on the item itself means the set's default.

use crate::catalog::{InputItem, Operation, Tool};
use crate::config::Tools;
use crate::format::{Format, MediaKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Preset {
    Gif {
        fps: u8,
        max_width: Option<u16>,
    },
    /// Maximum length of the short side (720 = "720p", portrait or not);
    /// `None` keeps the original size.
    Resolution(Option<u16>),
    Compress(Strength),
    /// Percent of the original width and height.
    Scale(u8),
    Quality(Level),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Strength {
    Light,
    Normal,
    Strong,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Level {
    High,
    Normal,
    Small,
}

#[derive(Debug)]
pub struct Choice {
    pub preset: Preset,
    pub label: &'static str,
    pub detail: &'static str,
}

#[derive(Debug)]
pub struct OptionSet {
    pub title: &'static str,
    pub choices: &'static [Choice],
    pub default: usize,
}

impl OptionSet {
    pub fn default_preset(&self) -> Preset {
        self.choices[self.default].preset
    }

    /// The row a drop on the tile itself uses: the one `tools` names for the
    /// shrink and video-compress sets, the fixed default for the rest.
    pub fn default_index(&self, tools: &Tools) -> usize {
        self.choices
            .iter()
            .position(|c| match c.preset {
                Preset::Scale(p) => p == tools.shrink_percent,
                Preset::Compress(s) => s == tools.compress_video,
                _ => false,
            })
            .unwrap_or(self.default)
    }
}

const fn choice(preset: Preset, label: &'static str, detail: &'static str) -> Choice {
    Choice {
        preset,
        label,
        detail,
    }
}

static GIF: OptionSet = OptionSet {
    title: "GIF 옵션",
    choices: &[
        choice(
            Preset::Gif {
                fps: 10,
                max_width: Some(320),
            },
            "작게",
            "10fps · 320px",
        ),
        choice(
            Preset::Gif {
                fps: 15,
                max_width: Some(480),
            },
            "보통",
            "15fps · 480px",
        ),
        choice(
            Preset::Gif {
                fps: 24,
                max_width: None,
            },
            "부드럽게",
            "24fps · 원본 크기",
        ),
    ],
    default: 1,
};

static RESOLUTION: OptionSet = OptionSet {
    title: "해상도",
    choices: &[
        choice(Preset::Resolution(None), "원본", "크기 그대로"),
        choice(Preset::Resolution(Some(1080)), "1080p", "Full HD"),
        choice(Preset::Resolution(Some(720)), "720p", "HD"),
        choice(Preset::Resolution(Some(480)), "480p", "SD"),
    ],
    default: 0,
};

static COMPRESS: OptionSet = OptionSet {
    title: "압축 강도",
    choices: &[
        choice(Preset::Compress(Strength::Light), "약하게", "화질 우선"),
        choice(Preset::Compress(Strength::Normal), "보통", "균형"),
        choice(
            Preset::Compress(Strength::Strong),
            "강하게",
            "용량 우선 · 720p",
        ),
    ],
    default: 1,
};

static SCALE: OptionSet = OptionSet {
    title: "축소 비율",
    choices: &[
        choice(Preset::Scale(25), "25%", "가로·세로 1/4"),
        choice(Preset::Scale(50), "50%", "가로·세로 1/2"),
        choice(Preset::Scale(75), "75%", "가로·세로 3/4"),
    ],
    default: 1,
};

static QUALITY: OptionSet = OptionSet {
    title: "품질",
    choices: &[
        choice(Preset::Quality(Level::High), "높게", "용량 큼"),
        choice(Preset::Quality(Level::Normal), "보통", "균형"),
        choice(Preset::Quality(Level::Small), "작게", "용량 작음"),
    ],
    default: 1,
};

/// The options `op` offers for these inputs. Only a selection of one media
/// kind gets options; mixed selections and animated GIFs get none.
pub fn options_for(op: &Operation, inputs: &[InputItem]) -> Option<&'static OptionSet> {
    match (common_kind(inputs)?, op) {
        (MediaKind::Video, Operation::Convert(Format::Gif)) => Some(&GIF),
        (MediaKind::Video, Operation::Convert(target)) if target.kind() == MediaKind::Video => {
            Some(&RESOLUTION)
        }
        (MediaKind::Video, Operation::Tool(Tool::CompressVideo)) => Some(&COMPRESS),
        (MediaKind::Image, Operation::Tool(Tool::Shrink)) => Some(&SCALE),
        (MediaKind::Image, Operation::Convert(Format::Jpg | Format::Webp)) => Some(&QUALITY),
        _ => None,
    }
}

/// The media kind every input shares, if any (no folders, no animated GIFs).
fn common_kind(inputs: &[InputItem]) -> Option<MediaKind> {
    let mut kind = None;
    for input in inputs {
        let InputItem::File {
            format: Some(format),
            animated_gif: false,
        } = *input
        else {
            return None;
        };
        if kind.is_some_and(|k| k != format.kind()) {
            return None;
        }
        kind = Some(format.kind());
    }
    kind
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(format: Format) -> InputItem {
        InputItem::File {
            format: Some(format),
            animated_gif: false,
        }
    }
    fn video() -> InputItem {
        file(Format::Mp4)
    }
    fn image() -> InputItem {
        file(Format::Png)
    }
    fn audio() -> InputItem {
        file(Format::Mp3)
    }

    #[test]
    fn option_sets_follow_the_input_kind() {
        use Format::*;
        let title = |op: Operation, inputs: &[InputItem]| options_for(&op, inputs).map(|s| s.title);
        assert_eq!(title(Operation::Convert(Gif), &[video()]), Some("GIF 옵션"));
        assert_eq!(title(Operation::Convert(Mkv), &[video()]), Some("해상도"));
        assert_eq!(
            title(Operation::Tool(Tool::CompressVideo), &[video()]),
            Some("압축 강도")
        );
        assert_eq!(
            title(Operation::Tool(Tool::Shrink), &[image()]),
            Some("축소 비율")
        );
        assert_eq!(title(Operation::Convert(Jpg), &[image()]), Some("품질"));
        assert_eq!(title(Operation::Convert(Webp), &[image()]), Some("품질"));
        // No options: image → GIF/PNG, audio, mixed kinds, animated GIF → video.
        assert_eq!(title(Operation::Convert(Gif), &[image()]), None);
        assert_eq!(title(Operation::Convert(Png), &[image()]), None);
        assert_eq!(title(Operation::Convert(Mp3), &[video()]), None);
        assert_eq!(title(Operation::Convert(Ogg), &[audio()]), None);
        assert_eq!(title(Operation::Convert(Gif), &[image(), video()]), None);
        let anim = InputItem::File {
            format: Some(Gif),
            animated_gif: true,
        };
        assert_eq!(title(Operation::Convert(Mp4), &[anim]), None);
    }

    #[test]
    fn defaults_match_the_spec() {
        let gif = options_for(&Operation::Convert(Format::Gif), &[video()]).unwrap();
        assert_eq!(
            gif.default_preset(),
            Preset::Gif {
                fps: 15,
                max_width: Some(480)
            }
        );
        assert_eq!(
            gif.choices[0].preset,
            Preset::Gif {
                fps: 10,
                max_width: Some(320)
            }
        );
        assert_eq!(
            gif.choices[2].preset,
            Preset::Gif {
                fps: 24,
                max_width: None
            }
        );
        let res = options_for(&Operation::Convert(Format::Mp4), &[file(Format::Mkv)]).unwrap();
        assert_eq!(
            res.choices.iter().map(|c| c.preset).collect::<Vec<_>>(),
            [None, Some(1080), Some(720), Some(480)]
                .map(Preset::Resolution)
                .to_vec()
        );
        assert_eq!(res.default_preset(), Preset::Resolution(None));
        assert_eq!(
            options_for(&Operation::Tool(Tool::Shrink), &[image()])
                .unwrap()
                .default_preset(),
            Preset::Scale(50)
        );
        assert_eq!(
            options_for(&Operation::Tool(Tool::CompressVideo), &[video()])
                .unwrap()
                .default_preset(),
            Preset::Compress(Strength::Normal)
        );
        assert_eq!(
            options_for(&Operation::Convert(Format::Jpg), &[image()])
                .unwrap()
                .default_preset(),
            Preset::Quality(Level::Normal)
        );
    }

    #[test]
    fn tool_defaults_pick_the_default_row() {
        let tools = Tools {
            shrink_percent: 75,
            compress_video: Strength::Light,
        };
        let shrink = options_for(&Operation::Tool(Tool::Shrink), &[image()]).unwrap();
        assert_eq!(shrink.default_index(&tools), 2);
        let compress = options_for(&Operation::Tool(Tool::CompressVideo), &[video()]).unwrap();
        assert_eq!(compress.default_index(&tools), 0);
        let gif = options_for(&Operation::Convert(Format::Gif), &[video()]).unwrap();
        assert_eq!(gif.default_index(&tools), gif.default); // not a tool set
    }
}
