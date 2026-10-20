//! Reading `%APPDATA%\SwissWheel\config.toml`, creating it with defaults if missing.

use std::path::{Path, PathBuf};

use swisswheel_core::config::{self, Config};

/// (`%APPDATA%\SwissWheel`, `%LOCALAPPDATA%\SwissWheel\logs`)
pub fn app_dirs() -> (PathBuf, PathBuf) {
    let env = |name: &str| PathBuf::from(std::env::var_os(name).unwrap_or_else(|| ".".into()));
    (
        env("APPDATA").join("SwissWheel"),
        env("LOCALAPPDATA").join("SwissWheel").join("logs"),
    )
}

/// Returns the config plus a user-facing warning when the file was unusable.
/// A broken file is left untouched so the user can fix it.
pub fn load(dir: &Path) -> (Config, Option<String>) {
    let path = dir.join("config.toml");
    match std::fs::read_to_string(&path) {
        Ok(text) => match config::parse(&text) {
            Ok(c) => (c, None),
            Err(e) => (Config::default(), Some(e.to_string())),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let written = std::fs::create_dir_all(dir)
                .and_then(|()| std::fs::write(&path, config::default_toml()));
            (
                Config::default(),
                written
                    .err()
                    .map(|e| format!("설정 파일을 만들 수 없습니다: {e}")),
            )
        }
        Err(e) => (
            Config::default(),
            Some(format!("설정 파일을 읽을 수 없습니다: {e}")),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use swisswheel_core::config::{Config, Key};

    #[test]
    fn missing_file_is_created_with_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let (config, warning) = load(dir.path());
        assert_eq!(config, Config::default());
        assert!(warning.is_none());
        let text = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert_eq!(swisswheel_core::config::parse(&text), Ok(Config::default()));
    }

    #[test]
    fn broken_file_is_kept_and_defaults_used() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[quality]\njpg = 0\n").unwrap();
        let (config, warning) = load(dir.path());
        assert_eq!(config, Config::default());
        assert!(warning.unwrap().contains("quality.jpg"));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[quality]\njpg = 0\n"
        );
    }

    #[test]
    fn valid_file_is_loaded() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.toml"), "open_key = \"CapsLock\"\n").unwrap();
        assert_eq!(load(dir.path()).0.open_key, Key::CapsLock);
    }
}
