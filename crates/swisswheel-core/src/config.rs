//! `config.toml`: parsing, defaults, and validation. File I/O lives in the app crate.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Key {
    Space,
    Tab,
    Escape,
    CapsLock,
    Backquote,
}

impl Key {
    /// Win32 virtual-key code.
    pub fn vk(self) -> u16 {
        match self {
            Key::Space => 0x20,
            Key::Tab => 0x09,
            Key::Escape => 0x1B,
            Key::CapsLock => 0x14,
            Key::Backquote => 0xC0,
        }
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub open_key: Key,
    pub cycle_key: Key,
    pub cancel_key: Key,
    pub start_with_windows: bool,
    pub quality: Quality,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            open_key: Key::Space,
            cycle_key: Key::Tab,
            cancel_key: Key::Escape,
            start_with_windows: false,
            quality: Quality::default(),
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
            parse("open_key = \"F5\""),
            Err(ConfigError::Parse(_))
        ));
        assert!(matches!(
            parse("this is not toml"),
            Err(ConfigError::Parse(_))
        ));
    }

    #[test]
    fn virtual_key_codes() {
        assert_eq!(Key::Space.vk(), 0x20);
        assert_eq!(Key::Backquote.vk(), 0xC0);
    }
}
