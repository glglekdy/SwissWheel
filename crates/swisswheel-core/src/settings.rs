//! The settings panel's rows: what each shows and how ← / → change it.

use crate::config::{Config, Key, Strength};

/// ← (`Back`) or → (`Forward`) on a settings row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Back,
    Forward,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingItem {
    OpenKey,
    CycleKey,
    CancelKey,
    StartWithWindows,
    JpgQuality,
    WebpQuality,
    Mp3Bitrate,
    ProgressWindow,
    TrayRing,
    ShrinkPercent,
    CompressVideo,
}

/// The panel's tabs (settings spec 2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Keys,
    Tools,
    Convert,
    General,
}

/// The tabs, left to right.
pub const CATEGORIES: [Category; 4] = [
    Category::Keys,
    Category::Tools,
    Category::Convert,
    Category::General,
];

/// The most rows any tab has; the panel is always this tall.
pub const MAX_ROWS: usize = 3;

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::Keys => "단축키",
            Category::Tools => "도구",
            Category::Convert => "변환",
            Category::General => "일반",
        }
    }

    /// The tab's rows, top to bottom.
    pub fn items(self) -> &'static [SettingItem] {
        use SettingItem::*;
        match self {
            Category::Keys => &[OpenKey, CycleKey, CancelKey],
            Category::Tools => &[ShrinkPercent, CompressVideo],
            Category::Convert => &[JpgQuality, WebpQuality, Mp3Bitrate],
            Category::General => &[StartWithWindows, ProgressWindow, TrayRing],
        }
    }
}

const MP3_KBPS: [u16; 5] = [128, 160, 192, 256, 320];
const SHRINK_PERCENTS: [u8; 3] = [25, 50, 75];
const STRENGTHS: [Strength; 3] = [Strength::Light, Strength::Normal, Strength::Strong];

/// Quality steps: 50 to 100 by 5. `config.rs` still accepts 1 to 100.
fn quality_steps() -> impl DoubleEndedIterator<Item = u8> {
    (50..=100).step_by(5)
}

pub fn key_label(k: Key) -> &'static str {
    match k {
        Key::Space => "Space",
        Key::Tab => "Tab",
        Key::Escape => "Esc",
        Key::CapsLock => "CapsLock",
        Key::Backquote => "`",
        Key::F1 => "F1",
        Key::F2 => "F2",
        Key::F3 => "F3",
        Key::F4 => "F4",
        Key::F5 => "F5",
        Key::F6 => "F6",
        Key::F7 => "F7",
        Key::F8 => "F8",
        Key::F9 => "F9",
        Key::F10 => "F10",
        Key::F11 => "F11",
        Key::F12 => "F12",
        Key::Insert => "Insert",
        Key::Delete => "Delete",
        Key::Home => "Home",
        Key::End => "End",
        Key::PageUp => "PgUp",
        Key::PageDown => "PgDn",
        Key::Pause => "Pause",
        Key::ScrollLock => "ScrollLock",
    }
}

fn on_off(value: bool) -> String {
    if value { "켬" } else { "끔" }.to_string()
}

fn strength_label(s: Strength) -> &'static str {
    match s {
        Strength::Light => "약하게",
        Strength::Normal => "보통",
        Strength::Strong => "강하게",
    }
}

impl SettingItem {
    pub fn label(self) -> &'static str {
        match self {
            SettingItem::OpenKey => "휠 열기 키",
            SettingItem::CycleKey => "휠 전환 키",
            SettingItem::CancelKey => "취소 키",
            SettingItem::StartWithWindows => "Windows 시작 시 실행",
            SettingItem::JpgQuality => "JPG 품질",
            SettingItem::WebpQuality => "WebP 품질",
            SettingItem::Mp3Bitrate => "MP3 비트레이트",
            SettingItem::ProgressWindow => "진행률 카드",
            SettingItem::TrayRing => "트레이 링",
            SettingItem::ShrinkPercent => "축소 비율",
            SettingItem::CompressVideo => "영상 압축",
        }
    }

    /// A shortcut key row: changed by pressing the key, not by ← / →.
    pub fn is_key(self) -> bool {
        matches!(
            self,
            SettingItem::OpenKey | SettingItem::CycleKey | SettingItem::CancelKey
        )
    }

    /// The current value as the panel shows it, even when it is off the
    /// panel's steps (a value written into `config.toml` by hand).
    pub fn value(self, c: &Config) -> String {
        match self {
            SettingItem::OpenKey => key_label(c.open_key).to_string(),
            SettingItem::CycleKey => key_label(c.cycle_key).to_string(),
            SettingItem::CancelKey => key_label(c.cancel_key).to_string(),
            SettingItem::StartWithWindows => on_off(c.start_with_windows),
            SettingItem::JpgQuality => c.quality.jpg.to_string(),
            SettingItem::WebpQuality => c.quality.webp.to_string(),
            SettingItem::Mp3Bitrate => format!("{} kbps", c.quality.mp3_kbps),
            SettingItem::ProgressWindow => on_off(c.progress.window),
            SettingItem::TrayRing => on_off(c.progress.tray_ring),
            SettingItem::ShrinkPercent => format!("{}%", c.tools.shrink_percent),
            SettingItem::CompressVideo => strength_label(c.tools.compress_video).to_string(),
        }
    }
}

/// The nearest step strictly past `current` in `dir` that `skip` allows.
/// Steps are ascending; this also moves an off-step value onto the steps.
fn next_in<T: PartialOrd + Copy>(
    steps: impl DoubleEndedIterator<Item = T>,
    current: T,
    dir: Dir,
    skip: impl Fn(T) -> bool,
) -> Option<T> {
    match dir {
        Dir::Forward => steps.into_iter().find(|&s| s > current && !skip(s)),
        Dir::Back => steps.into_iter().rev().find(|&s| s < current && !skip(s)),
    }
}

/// The config after one ← / → on `item`, or `None` when it can't move (end
/// of the steps, or a shortcut key row, which takes a key press instead).
pub fn step(c: &Config, item: SettingItem, dir: Dir) -> Option<Config> {
    let mut next = c.clone();
    match item {
        SettingItem::OpenKey | SettingItem::CycleKey | SettingItem::CancelKey => return None,
        SettingItem::StartWithWindows => {
            next.start_with_windows =
                next_in([false, true].into_iter(), c.start_with_windows, dir, |_| {
                    false
                })?;
        }
        SettingItem::JpgQuality => {
            next.quality.jpg = next_in(quality_steps(), c.quality.jpg, dir, |_| false)?;
        }
        SettingItem::WebpQuality => {
            next.quality.webp = next_in(quality_steps(), c.quality.webp, dir, |_| false)?;
        }
        SettingItem::Mp3Bitrate => {
            next.quality.mp3_kbps =
                next_in(MP3_KBPS.into_iter(), c.quality.mp3_kbps, dir, |_| false)?;
        }
        SettingItem::ProgressWindow => {
            next.progress.window =
                next_in([false, true].into_iter(), c.progress.window, dir, |_| false)?;
        }
        SettingItem::TrayRing => {
            next.progress.tray_ring =
                next_in([false, true].into_iter(), c.progress.tray_ring, dir, |_| {
                    false
                })?;
        }
        SettingItem::ShrinkPercent => {
            next.tools.shrink_percent = next_in(
                SHRINK_PERCENTS.into_iter(),
                c.tools.shrink_percent,
                dir,
                |_| false,
            )?;
        }
        SettingItem::CompressVideo => {
            // Strengths have no order of their own: compare positions.
            let index = |s: Strength| STRENGTHS.iter().position(|&x| x == s).unwrap_or(1);
            let i = next_in(
                0..STRENGTHS.len(),
                index(c.tools.compress_video),
                dir,
                |_| false,
            )?;
            next.tools.compress_video = STRENGTHS[i];
        }
    }
    Some(next)
}

/// The result of pressing `key` on a shortcut row.
#[derive(Debug, Clone, PartialEq)]
pub struct Bound {
    pub config: Config,
    /// The other row that had `key`, and the key it got in exchange.
    pub swapped: Option<(SettingItem, Key)>,
}

fn key_of(c: &Config, item: SettingItem) -> Option<Key> {
    match item {
        SettingItem::OpenKey => Some(c.open_key),
        SettingItem::CycleKey => Some(c.cycle_key),
        SettingItem::CancelKey => Some(c.cancel_key),
        _ => None,
    }
}

fn set_key(c: &mut Config, item: SettingItem, key: Key) {
    match item {
        SettingItem::OpenKey => c.open_key = key,
        SettingItem::CycleKey => c.cycle_key = key,
        SettingItem::CancelKey => c.cancel_key = key,
        _ => {}
    }
}

/// Binds `key` to the shortcut row `item`. A key another row already uses
/// swaps: that row gets `item`'s old key. Other rows leave `c` as it is.
pub fn bind(c: &Config, item: SettingItem, key: Key) -> Bound {
    let mut config = c.clone();
    let Some(old) = key_of(c, item).filter(|&old| old != key) else {
        return Bound {
            config,
            swapped: None,
        };
    };
    let other = Category::Keys
        .items()
        .iter()
        .copied()
        .find(|&o| o != item && key_of(c, o) == Some(key));
    set_key(&mut config, item, key);
    if let Some(other) = other {
        set_key(&mut config, other, old);
    }
    Bound {
        config,
        swapped: other.map(|o| (o, old)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Key, Quality};

    #[test]
    fn labels_and_values_in_order() {
        let c = Config::default();
        let rows: Vec<_> = CATEGORIES
            .iter()
            .flat_map(|cat| cat.items())
            .map(|i| (i.label(), i.value(&c)))
            .collect();
        let expected = [
            ("휠 열기 키", "Space"),
            ("휠 전환 키", "Tab"),
            ("취소 키", "Esc"),
            ("축소 비율", "50%"),
            ("영상 압축", "보통"),
            ("JPG 품질", "90"),
            ("WebP 품질", "85"),
            ("MP3 비트레이트", "192 kbps"),
            ("Windows 시작 시 실행", "끔"),
            ("진행률 카드", "켬"),
            ("트레이 링", "켬"),
        ]
        .map(|(l, v)| (l, v.to_string()));
        assert_eq!(rows, expected);
    }

    #[test]
    fn categories_in_order_with_their_items() {
        use SettingItem::*;
        let labels: Vec<_> = CATEGORIES.iter().map(|c| c.label()).collect();
        assert_eq!(labels, ["단축키", "도구", "변환", "일반"]);
        assert_eq!(Category::Keys.items(), [OpenKey, CycleKey, CancelKey]);
        assert_eq!(Category::Tools.items(), [ShrinkPercent, CompressVideo]);
        assert_eq!(
            Category::Convert.items(),
            [JpgQuality, WebpQuality, Mp3Bitrate]
        );
        assert_eq!(
            Category::General.items(),
            [StartWithWindows, ProgressWindow, TrayRing]
        );
        assert!(CATEGORIES.iter().all(|c| c.items().len() <= MAX_ROWS));
    }

    #[test]
    fn tool_defaults_step_and_stop() {
        let c = Config::default();
        assert_eq!(SettingItem::ShrinkPercent.value(&c), "50%");
        assert_eq!(SettingItem::CompressVideo.value(&c), "보통");
        let c = step(&c, SettingItem::ShrinkPercent, Dir::Forward).unwrap();
        assert_eq!(c.tools.shrink_percent, 75);
        assert_eq!(step(&c, SettingItem::ShrinkPercent, Dir::Forward), None);
        let c = step(&c, SettingItem::CompressVideo, Dir::Back).unwrap();
        assert_eq!(SettingItem::CompressVideo.value(&c), "약하게");
        assert_eq!(step(&c, SettingItem::CompressVideo, Dir::Back), None);
    }

    #[test]
    fn key_items_do_not_step() {
        for item in Category::Keys.items() {
            assert!(item.is_key());
            assert_eq!(step(&Config::default(), *item, Dir::Forward), None);
        }
        assert!(!SettingItem::JpgQuality.is_key());
    }

    #[test]
    fn bind_new_key() {
        let b = bind(&Config::default(), SettingItem::OpenKey, Key::F5);
        assert_eq!(b.config.open_key, Key::F5);
        assert_eq!(b.swapped, None);
    }

    #[test]
    fn bind_taken_key_swaps() {
        let b = bind(&Config::default(), SettingItem::CycleKey, Key::Space);
        assert_eq!(
            (b.config.open_key, b.config.cycle_key),
            (Key::Tab, Key::Space)
        );
        assert_eq!(b.swapped, Some((SettingItem::OpenKey, Key::Tab)));
        assert!(crate::config::parse(&toml::to_string(&b.config).unwrap()).is_ok());
    }

    #[test]
    fn bind_same_key_is_a_no_op() {
        let b = bind(&Config::default(), SettingItem::CancelKey, Key::Escape);
        assert_eq!(b.config, Config::default());
        assert_eq!(b.swapped, None);
    }

    #[test]
    fn quality_steps_by_five_and_stops_at_ends() {
        let c = Config::default();
        let jpg = |c: &Config, dir| step(c, SettingItem::JpgQuality, dir).map(|c| c.quality.jpg);
        assert_eq!(jpg(&c, Dir::Forward), Some(95));
        assert_eq!(jpg(&c, Dir::Back), Some(85));
        let top = Config {
            quality: Quality {
                jpg: 100,
                ..Quality::default()
            },
            ..c.clone()
        };
        assert_eq!(jpg(&top, Dir::Forward), None);
        let bottom = Config {
            quality: Quality {
                webp: 50,
                ..Quality::default()
            },
            ..c
        };
        assert_eq!(step(&bottom, SettingItem::WebpQuality, Dir::Back), None);
    }

    #[test]
    fn off_step_values_show_as_is_and_step_toward_direction() {
        let c = Config {
            quality: Quality {
                jpg: 42,
                webp: 87,
                mp3_kbps: 200,
            },
            ..Config::default()
        };
        assert_eq!(SettingItem::JpgQuality.value(&c), "42");
        let q = |item, dir| step(&c, item, dir).map(|c| c.quality);
        assert_eq!(q(SettingItem::JpgQuality, Dir::Forward).unwrap().jpg, 50);
        assert_eq!(q(SettingItem::JpgQuality, Dir::Back), None);
        assert_eq!(q(SettingItem::WebpQuality, Dir::Back).unwrap().webp, 85);
        assert_eq!(
            q(SettingItem::Mp3Bitrate, Dir::Forward).unwrap().mp3_kbps,
            256
        );
        assert_eq!(q(SettingItem::Mp3Bitrate, Dir::Back).unwrap().mp3_kbps, 192);
    }

    #[test]
    fn booleans_are_off_then_on() {
        let c = Config::default();
        let start = step(&c, SettingItem::StartWithWindows, Dir::Forward).unwrap();
        assert!(start.start_with_windows);
        assert_eq!(step(&c, SettingItem::StartWithWindows, Dir::Back), None);
        let ring = step(&c, SettingItem::TrayRing, Dir::Back).unwrap();
        assert!(!ring.progress.tray_ring);
        assert_eq!(step(&c, SettingItem::ProgressWindow, Dir::Forward), None);
    }
}
