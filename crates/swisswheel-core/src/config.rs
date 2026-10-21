//! `config.toml`: parsing, defaults, and validation. File I/O lives in the app crate.

use serde::{Deserialize, Serialize};

pub use crate::options::Strength;

/// Keys the wheel and menu can be bound to (settings spec 2.3). Letters,
/// digits, arrows, Enter, and modifiers are left out on purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Key {
    Space,
    Tab,
    Escape,
    CapsLock,
    Backquote,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    Insert,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    Pause,
    ScrollLock,
}

impl Key {
    pub const ALL: [Key; 25] = [
        Key::Space,
        Key::Tab,
        Key::Escape,
        Key::CapsLock,
        Key::Backquote,
        Key::F1,
        Key::F2,
        Key::F3,
        Key::F4,
        Key::F5,
        Key::F6,
        Key::F7,
        Key::F8,
        Key::F9,
        Key::F10,
        Key::F11,
        Key::F12,
        Key::Insert,
        Key::Delete,
        Key::Home,
        Key::End,
        Key::PageUp,
        Key::PageDown,
        Key::Pause,
        Key::ScrollLock,
    ];

    /// Win32 virtual-key code.
    pub fn vk(self) -> u16 {
        match self {
            Key::Space => 0x20,
            Key::Tab => 0x09,
            Key::Escape => 0x1B,
            Key::CapsLock => 0x14,
            Key::Backquote => 0xC0,
            Key::F1 => 0x70,
            Key::F2 => 0x71,
            Key::F3 => 0x72,
            Key::F4 => 0x73,
            Key::F5 => 0x74,
            Key::F6 => 0x75,
            Key::F7 => 0x76,
            Key::F8 => 0x77,
            Key::F9 => 0x78,
            Key::F10 => 0x79,
            Key::F11 => 0x7A,
            Key::F12 => 0x7B,
            Key::Insert => 0x2D,
            Key::Delete => 0x2E,
            Key::Home => 0x24,
            Key::End => 0x23,
            Key::PageUp => 0x21,
            Key::PageDown => 0x22,
            Key::Pause => 0x13,
            Key::ScrollLock => 0x91,
        }
    }

    /// The bindable key with this virtual-key code, if any.
    pub fn from_vk(vk: u16) -> Option<Key> {
        Key::ALL.into_iter().find(|k| k.vk() == vk)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Quality {
    pub jpg: u8,
    pub webp: u8,
    pub mp3_kbps: u16,
}

impl Default for Quality {
    fn default() -> Self {
        Quality {
            jpg: 90,
            webp: 85,
            mp3_kbps: 192,
        }
    }
}

/// Where running jobs show their progress (spec 2.4). The settings panel
/// edits these.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProgressDisplay {
    /// The card above the tray, one row per file.
    pub window: bool,
    /// A ring around the tray icon that fills as jobs advance.
    pub tray_ring: bool,
}

impl Default for ProgressDisplay {
    fn default() -> Self {
        ProgressDisplay {
            window: true,
            tray_ring: true,
        }
    }
}

/// What a drop on a tool's tile (not one of its option rows) uses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tools {
    /// Shrink: 25, 50, or 75 percent.
    pub shrink_percent: u8,
    pub compress_video: Strength,
}

impl Default for Tools {
    fn default() -> Self {
        Tools {
            shrink_percent: 50,
            compress_video: Strength::Normal,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub open_key: Key,
    pub cycle_key: Key,
    pub cancel_key: Key,
    pub start_with_windows: bool,
    pub quality: Quality,
    pub progress: ProgressDisplay,
    pub tools: Tools,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            open_key: Key::Space,
            cycle_key: Key::Tab,
            cancel_key: Key::Escape,
            start_with_windows: false,
            quality: Quality::default(),
            progress: ProgressDisplay::default(),
            tools: Tools::default(),
        }
    }
}

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum ConfigError {
    #[error("설정 파일을 읽을 수 없습니다: {0}")]
    Parse(String),
    #[error("설정 값이 올바르지 않습니다: {0}")]
    Invalid(String),
}

/// Missing keys take defaults; unknown keys are ignored.
pub fn parse(text: &str) -> Result<Config, ConfigError> {
    let config: Config = toml::from_str(text).map_err(|e| ConfigError::Parse(e.to_string()))?;
    validate(&config)?;
    Ok(config)
}

pub fn default_toml() -> String {
    toml::to_string_pretty(&Config::default()).expect("default config serializes")
}

fn validate(c: &Config) -> Result<(), ConfigError> {
    let in_range = |name: &str, value: u16, lo: u16, hi: u16| {
        if (lo..=hi).contains(&value) {
            Ok(())
        } else {
            Err(ConfigError::Invalid(format!(
                "{name}는 {lo}~{hi} 사이여야 합니다 (현재 {value})"
            )))
        }
    };
    in_range("quality.jpg", c.quality.jpg.into(), 1, 100)?;
    in_range("quality.webp", c.quality.webp.into(), 1, 100)?;
    in_range("quality.mp3_kbps", c.quality.mp3_kbps, 32, 320)?;
    let shrink = c.tools.shrink_percent;
    if ![25, 50, 75].contains(&shrink) {
        return Err(ConfigError::Invalid(format!(
            "tools.shrink_percent는 25, 50, 75 중 하나여야 합니다 (현재 {shrink})"
        )));
    }
    let keys = [
        ("open_key", c.open_key),
        ("cycle_key", c.cycle_key),
        ("cancel_key", c.cancel_key),
    ];
    for (i, (a_name, a)) in keys.iter().enumerate() {
        for (b_name, b) in &keys[i + 1..] {
            if a == b {
                return Err(ConfigError::Invalid(format!(
                    "{a_name}와 {b_name}가 같은 키({a:?})입니다"
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip() {
        let c = Config::default();
        assert_eq!(
            (c.open_key, c.cycle_key, c.cancel_key),
            (Key::Space, Key::Tab, Key::Escape)
        );
        assert_eq!(
            (c.quality.jpg, c.quality.webp, c.quality.mp3_kbps),
            (90, 85, 192)
        );
        assert!(!c.start_with_windows);
        assert_eq!(parse(&default_toml()), Ok(c));
        assert_eq!(parse(""), Ok(Config::default()));
    }

    #[test]
    fn partial_and_unknown_keys() {
        let c = parse("open_key = \"CapsLock\"\nfuture_option = 3\n[quality]\njpg = 70\n").unwrap();
        assert_eq!(c.open_key, Key::CapsLock);
        assert_eq!(c.quality.jpg, 70);
        assert_eq!(c.quality.webp, 85);
        assert_eq!(c.cycle_key, Key::Tab);
    }

    #[test]
    fn invalid_configs() {
        assert!(matches!(
            parse("[quality]\njpg = 0"),
            Err(ConfigError::Invalid(_))
        ));
        assert!(matches!(
            parse("[quality]\nmp3_kbps = 999"),
            Err(ConfigError::Invalid(_))
        ));
        assert!(matches!(
            parse("open_key = \"Tab\""),
            Err(ConfigError::Invalid(_))
        )); // same as cycle_key
        assert!(matches!(
            parse("[quality]\njpg = \"high\""),
            Err(ConfigError::Parse(_))
        ));
        assert!(matches!(
            parse("open_key = \"F13\""),
            Err(ConfigError::Parse(_))
        ));
        assert!(matches!(
            parse("this is not toml"),
            Err(ConfigError::Parse(_))
        ));
    }

    #[test]
    fn progress_display_settings() {
        let c = Config::default();
        assert!(c.progress.window && c.progress.tray_ring);
        let c = parse(
            "[progress]
window = false
",
        )
        .unwrap();
        assert!(!c.progress.window && c.progress.tray_ring);
        let c = parse(
            "[progress]
tray_ring = false
",
        )
        .unwrap();
        assert!(c.progress.window && !c.progress.tray_ring);
        assert!(default_toml().contains("[progress]"));
    }

    #[test]
    fn virtual_key_codes() {
        assert_eq!(Key::Space.vk(), 0x20);
        assert_eq!(Key::Backquote.vk(), 0xC0);
    }

    #[test]
    fn new_keys_parse_and_round_trip() {
        let c = parse("open_key = \"F5\"\ncycle_key = \"PageUp\"\ncancel_key = \"Pause\"\n")
            .unwrap();
        assert_eq!(
            (c.open_key, c.cycle_key, c.cancel_key),
            (Key::F5, Key::PageUp, Key::Pause)
        );
        assert_eq!(parse(&toml::to_string_pretty(&c).unwrap()), Ok(c));
    }

    #[test]
    fn vk_codes_round_trip() {
        assert_eq!(Key::ALL.len(), 25);
        for k in Key::ALL {
            assert_eq!(Key::from_vk(k.vk()), Some(k));
        }
        assert_eq!(Key::F1.vk(), 0x70);
        assert_eq!(Key::F12.vk(), 0x7B);
        assert_eq!(Key::ScrollLock.vk(), 0x91);
        for vk in [0x41, 0x30, 0x25, 0x0D, 0xA0, 0x5B] {
            assert_eq!(Key::from_vk(vk), None); // A, 0, ←, Enter, Shift, Win
        }
    }

    #[test]
    fn tools_defaults_and_validation() {
        let c = Config::default();
        assert_eq!(
            (c.tools.shrink_percent, c.tools.compress_video),
            (50, Strength::Normal)
        );
        let c = parse("[tools]\nshrink_percent = 25\ncompress_video = \"Strong\"\n").unwrap();
        assert_eq!(
            (c.tools.shrink_percent, c.tools.compress_video),
            (25, Strength::Strong)
        );
        assert!(matches!(
            parse("[tools]\nshrink_percent = 40"),
            Err(ConfigError::Invalid(_))
        ));
        assert!(matches!(
            parse("[tools]\ncompress_video = \"Max\""),
            Err(ConfigError::Parse(_))
        ));
    }
}
