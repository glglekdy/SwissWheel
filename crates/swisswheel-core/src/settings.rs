//! The settings panel's rows: what each shows and how ← / → change it.

use crate::config::{Config, Key};

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
}

/// The panel's rows, top to bottom (spec 2.4).
pub const ITEMS: [SettingItem; 9] = [
    SettingItem::OpenKey,
    SettingItem::CycleKey,
    SettingItem::CancelKey,
    SettingItem::StartWithWindows,
    SettingItem::JpgQuality,
    SettingItem::WebpQuality,
    SettingItem::Mp3Bitrate,
    SettingItem::ProgressWindow,
    SettingItem::TrayRing,
];

const KEYS: [Key; 5] = [
    Key::Space,
    Key::Tab,
    Key::Escape,
    Key::CapsLock,
    Key::Backquote,
];
const MP3_KBPS: [u16; 5] = [128, 160, 192, 256, 320];

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
    }
}

fn on_off(value: bool) -> String {
    if value { "켬" } else { "끔" }.to_string()
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
        }
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

/// The config after one ← / → on `item`, or `None` when it can't move
/// (end of the steps, or every further key is taken by another binding).
pub fn step(c: &Config, item: SettingItem, dir: Dir) -> Option<Config> {
    let mut next = c.clone();
    match item {
        SettingItem::OpenKey | SettingItem::CycleKey | SettingItem::CancelKey => {
            let (slot, others) = match item {
                SettingItem::OpenKey => (&mut next.open_key, [c.cycle_key, c.cancel_key]),
                SettingItem::CycleKey => (&mut next.cycle_key, [c.open_key, c.cancel_key]),
                _ => (&mut next.cancel_key, [c.open_key, c.cycle_key]),
            };
            // Keys have no order of their own: compare positions in KEYS.
            let index = |k: Key| KEYS.iter().position(|&x| x == k).unwrap_or(0);
            let i = next_in(0..KEYS.len(), index(*slot), dir, |i| {
                others.contains(&KEYS[i])
            })?;
            *slot = KEYS[i];
        }
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
    }
    Some(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Key, Quality};

    #[test]
    fn labels_and_values_in_order() {
        let c = Config::default();
        let rows: Vec<_> = ITEMS.iter().map(|i| (i.label(), i.value(&c))).collect();
        let expected = [
            ("휠 열기 키", "Space"),
            ("휠 전환 키", "Tab"),
            ("취소 키", "Esc"),
            ("Windows 시작 시 실행", "끔"),
            ("JPG 품질", "90"),
            ("WebP 품질", "85"),
            ("MP3 비트레이트", "192 kbps"),
            ("진행률 카드", "켬"),
            ("트레이 링", "켬"),
        ]
        .map(|(l, v)| (l, v.to_string()));
        assert_eq!(rows, expected);
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
    fn keys_skip_values_used_by_the_other_two() {
        let c = Config::default(); // Space, Tab, Esc
        let open = step(&c, SettingItem::OpenKey, Dir::Forward).unwrap();
        assert_eq!(open.open_key, Key::CapsLock);
        assert_eq!(step(&c, SettingItem::OpenKey, Dir::Back), None); // Space is first
        assert_eq!(step(&c, SettingItem::CancelKey, Dir::Back), None);
        let next = step(&c, SettingItem::CancelKey, Dir::Forward).unwrap();
        assert_eq!(next.cancel_key, Key::CapsLock);
        assert!(crate::config::parse(&toml::to_string(&next).unwrap()).is_ok());
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
