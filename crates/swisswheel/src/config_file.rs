//! Reading `%APPDATA%\SwissWheel\config.toml`, creating it with defaults if
//! missing, and saving edits from the settings panel.

use std::path::{Path, PathBuf};

use swisswheel_core::config::{self, Config};
use toml_edit::{DocumentMut, Item, Table, Value};

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

/// Writes the settings that changed from `before` to `after` into
/// `dir/config.toml`. Everything else in the file stays as it is: comments,
/// key order, keys this version doesn't know, edits the user made by hand
/// while the app was running, and defaults left unwritten. A file that
/// isn't TOML is left alone (`InvalidData`).
pub fn save(dir: &Path, before: &Config, after: &Config) -> std::io::Result<()> {
    let path = dir.join("config.toml");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let invalid = |e: &dyn std::fmt::Display| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
    };
    let mut doc: DocumentMut = text.parse().map_err(|e| invalid(&e))?;
    let old = toml_edit::ser::to_document(before).map_err(|e| invalid(&e))?;
    let new = toml_edit::ser::to_document(after).map_err(|e| invalid(&e))?;
    merge(doc.as_table_mut(), old.as_table(), new.as_table());
    std::fs::create_dir_all(dir)?;
    // Write beside it and swap, so a failed write never leaves half a file.
    let temp = dir.join("config.toml.tmp");
    std::fs::write(&temp, doc.to_string())?;
    std::fs::rename(&temp, &path)
}

/// Copies into `dst` the keys whose value differs between `old` and `new`,
/// keeping each existing value's surrounding comments and whitespace.
fn merge(dst: &mut Table, old: &Table, new: &Table) {
    for (key, item) in new.iter() {
        let before = old.get(key);
        if let Some(new_table) = as_table(item) {
            let old_table = before.and_then(as_table).unwrap_or_default();
            match dst.get_mut(key) {
                Some(Item::Table(dst_table)) => merge(dst_table, &old_table, &new_table),
                _ => {
                    // A section the file doesn't have yet: only if it gets keys.
                    let mut section = Table::new();
                    merge(&mut section, &old_table, &new_table);
                    if !section.is_empty() {
                        dst.insert(key, Item::Table(section));
                    }
                }
            }
            continue;
        }
        let changed = before.and_then(Item::as_value).map(Value::to_string)
            != item.as_value().map(Value::to_string);
        if !changed {
            continue;
        }
        match (dst.get_mut(key), item) {
            (Some(Item::Value(old)), Item::Value(new)) => {
                let decor = old.decor().clone();
                *old = new.clone();
                *old.decor_mut() = decor;
            }
            _ => {
                dst.insert(key, item.clone());
            }
        }
    }
}

/// The serializer may write nested structs as inline tables; the file keeps
/// them as `[section]` tables.
fn as_table(item: &Item) -> Option<Table> {
    match item {
        Item::Table(t) => Some(t.clone()),
        Item::Value(Value::InlineTable(t)) => Some(t.clone().into_table()),
        _ => None,
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

    #[test]
    fn save_keeps_comments_order_and_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "# my keys\nopen_key = \"Space\" # hold me\nfuture = 1\n\n[quality]\n# sharp\njpg = 42\n",
        )
        .unwrap();
        let before = load(dir.path()).0;
        let mut c = before.clone();
        c.open_key = Key::CapsLock;
        c.quality.webp = 70;
        save(dir.path(), &before, &c).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.starts_with("# my keys\nopen_key = \"CapsLock\" # hold me\nfuture = 1\n"),
            "{text}"
        );
        assert!(text.contains("# sharp\njpg = 42"), "{text}");
        assert!(text.contains("webp = 70"), "{text}");
        assert_eq!(swisswheel_core::config::parse(&text).unwrap(), c);
    }

    #[test]
    fn save_creates_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("SwissWheel");
        let c = Config {
            start_with_windows: true,
            ..Config::default()
        };
        save(&sub, &Config::default(), &c).unwrap();
        assert_eq!(load(&sub).0, c);
    }

    #[test]
    fn save_writes_only_what_changed() {
        // The user set webp = 70 by hand while the app still has 85.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "open_key = \"Space\"\n\n[quality]\nwebp = 70\n").unwrap();
        let before = Config::default();
        let mut after = before.clone();
        after.progress.tray_ring = false;
        save(dir.path(), &before, &after).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("webp = 70"), "{text}");
        assert!(text.contains("tray_ring = false"), "{text}");
        assert!(!text.contains("jpg"), "defaults stay unwritten: {text}");
        assert!(!text.contains("cycle_key"), "{text}");
    }

    #[test]
    fn save_adds_tools_section_when_changed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.toml"), "open_key = \"Space\"\n").unwrap();
        let before = Config::default();
        let mut after = before.clone();
        after.tools.shrink_percent = 75;
        save(dir.path(), &before, &after).unwrap();
        let text = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(
            text.contains("[tools]") && text.contains("shrink_percent = 75"),
            "{text}"
        );
        assert!(!text.contains("compress_video"), "{text}");
    }

    #[test]
    fn save_refuses_unreadable_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "this is not toml").unwrap();
        let mut after = Config::default();
        after.quality.jpg = 80;
        let err = save(dir.path(), &Config::default(), &after).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "this is not toml");
    }
}
