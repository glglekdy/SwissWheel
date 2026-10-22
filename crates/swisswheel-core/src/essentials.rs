//! The apps SwissWheel's basic features need (download panel spec 2): what
//! each one's row says, which buttons it offers, and when a button is refused.

use std::path::Path;

use crate::ffmpeg_release;
use crate::update::{self, TileDecision};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Essential {
    Ffmpeg,
}

/// The download panel's rows, top to bottom.
pub const ESSENTIALS: [Essential; 1] = [Essential::Ffmpeg];

/// The most buttons a row has (update and remove).
pub const MAX_BUTTONS: usize = 2;

impl Essential {
    pub fn label(self) -> &'static str {
        match self {
            Essential::Ffmpeg => "FFmpeg",
        }
    }

    /// What it is needed for, after the name.
    pub fn purpose(self) -> &'static str {
        match self {
            Essential::Ffmpeg => "영상·오디오",
        }
    }
}

/// Where the app found it, and the version it reports (`None`: unreadable).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Installed {
    None,
    /// On `PATH`, installed by the user; never updated or removed.
    System(Option<String>),
    /// The app's own copy, in one of `install_dirs()`.
    Own(Option<String>),
}

/// The newest release, as far as the app knows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Latest {
    #[default]
    Unknown,
    Checking,
    Found(String),
    Failed,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct EssentialState {
    /// `None` until the first check comes back.
    pub installed: Option<Installed>,
    pub latest: Latest,
    /// A download into the app's folder is running: its fraction, 0.0 → 1.0.
    pub busy: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EssentialAction {
    Install,
    /// The app's own copy next to a system one.
    InstallOwn,
    Update,
    Remove,
}

impl EssentialAction {
    pub fn label(self) -> &'static str {
        match self {
            EssentialAction::Install => "설치",
            EssentialAction::InstallOwn => "전용 설치",
            EssentialAction::Update => "업데이트",
            EssentialAction::Remove => "제거",
        }
    }
}

/// The row's status text and buttons, left to right (spec 2.2).
pub fn view(state: &EssentialState) -> (String, Vec<EssentialAction>) {
    use EssentialAction::*;
    if let Some(fraction) = state.busy {
        return (format!("받는 중 {}%", (fraction * 100.0).round()), vec![]);
    }
    match &state.installed {
        None => ("확인 중…".into(), vec![]),
        Some(Installed::None) => ("설치 안 됨".into(), vec![Install]),
        Some(Installed::System(Some(v))) => (format!("시스템 {v} 사용 중"), vec![InstallOwn]),
        Some(Installed::System(None)) => ("시스템 FFmpeg 사용 중".into(), vec![InstallOwn]),
        Some(Installed::Own(None)) => ("버전 알 수 없음".into(), vec![Update, Remove]),
        Some(Installed::Own(Some(v))) => match &state.latest {
            Latest::Unknown | Latest::Checking => (v.clone(), vec![Remove]),
            Latest::Failed => (format!("{v} · 확인 실패"), vec![Remove]),
            Latest::Found(latest) => match is_newer(latest, v) {
                Some(false) => (format!("{v} · 최신"), vec![Remove]),
                Some(true) => (format!("{v} · {latest} 있음"), vec![Update, Remove]),
                None => (format!("{v} · 최신 {latest}"), vec![Update, Remove]),
            },
        },
    }
}

/// The panel's top line.
pub fn header(checking: bool) -> String {
    if checking {
        "필수 앱 · 확인 중…".into()
    } else {
        "필수 앱".into()
    }
}

/// `9.0.2` from `ffmpeg version 9.0.2-essentials_build-www.gyan.dev …`;
/// `None` for builds without a dotted number (git builds).
pub fn parse_ffmpeg_version(first_line: &str) -> Option<String> {
    let word = first_line
        .strip_prefix("ffmpeg version ")?
        .split_whitespace()
        .next()?;
    let version = word.split('-').next()?;
    let version = version.strip_prefix('n').unwrap_or(version);
    // A dotted number: `2024-05-01-git-…` is a date, not a version.
    (version.contains('.') && numbers(version).is_some()).then(|| version.to_string())
}

/// `9.0.2` → `[9, 0, 2]`; `None` unless every part is a number.
fn numbers(version: &str) -> Option<Vec<u32>> {
    version.split('.').map(|part| part.parse().ok()).collect()
}

/// Whether `latest` is past `installed`, comparing dotted numbers; `None`
/// when either isn't one.
pub fn is_newer(latest: &str, installed: &str) -> Option<bool> {
    let (mut a, mut b) = (numbers(latest)?, numbers(installed)?);
    let len = a.len().max(b.len());
    a.resize(len, 0);
    b.resize(len, 0);
    Some(a > b)
}

/// A button was pressed. Other background work comes first; a running
/// conversion holds the exe, so it can't be replaced or removed meanwhile.
pub fn on_action(
    action: EssentialAction,
    jobs_active: bool,
    background_busy: bool,
) -> TileDecision {
    let holds_the_exe = matches!(action, EssentialAction::Update | EssentialAction::Remove);
    if background_busy {
        TileDecision::Refuse(ffmpeg_release::text::RUNNING)
    } else if jobs_active && holds_the_exe {
        TileDecision::Refuse(update::text::BUSY)
    } else {
        TileDecision::Start
    }
}

/// The remove confirmation's body.
pub fn remove_prompt(version: Option<&str>, dir: &Path, system: bool) -> String {
    let name = match version {
        Some(v) => format!("FFmpeg {v}를"),
        None => "FFmpeg를".into(),
    };
    let mut body = format!(
        "SwissWheel이 받은 {name} 지울까요?

위치: {}",
        dir.display()
    );
    if system {
        body.push_str(
            "

지운 뒤에는 시스템 FFmpeg를 씁니다",
        );
    }
    body
}

/// Notification bodies (titled [`ffmpeg_release::TITLE`]).
pub mod text {
    pub const REMOVED: &str = "FFmpeg를 지웠습니다";
    pub const REMOVE_FAILED: &str = "FFmpeg를 지우지 못했습니다";
}

#[cfg(test)]
mod tests {
    use super::*;
    use EssentialAction::*;

    fn own(v: &str, latest: Latest) -> EssentialState {
        EssentialState {
            installed: Some(Installed::Own(Some(v.into()))),
            latest,
            busy: None,
        }
    }

    fn s(installed: Option<Installed>, latest: Latest) -> EssentialState {
        EssentialState {
            installed,
            latest,
            busy: None,
        }
    }

    fn found(v: &str) -> Latest {
        Latest::Found(v.into())
    }

    #[test]
    fn view_follows_the_spec_table() {
        let v = |state: &EssentialState| view(state);
        assert_eq!(v(&s(None, Latest::Checking)), ("확인 중…".into(), vec![]));
        assert_eq!(
            v(&s(Some(Installed::None), Latest::Failed)),
            ("설치 안 됨".into(), vec![Install])
        );
        assert_eq!(
            v(&s(
                Some(Installed::System(Some("8.0".into()))),
                Latest::Failed
            )),
            ("시스템 8.0 사용 중".into(), vec![InstallOwn])
        );
        assert_eq!(
            v(&s(Some(Installed::System(None)), Latest::Unknown)),
            ("시스템 FFmpeg 사용 중".into(), vec![InstallOwn])
        );
        assert_eq!(
            v(&own("9.0.2", Latest::Checking)),
            ("9.0.2".into(), vec![Remove])
        );
        assert_eq!(
            v(&own("9.0.2", found("9.0.2"))),
            ("9.0.2 · 최신".into(), vec![Remove])
        );
        assert_eq!(
            v(&own("9.0.2", found("9.0.3"))),
            ("9.0.2 · 9.0.3 있음".into(), vec![Update, Remove])
        );
        assert_eq!(
            v(&own("9.0.2", Latest::Failed)),
            ("9.0.2 · 확인 실패".into(), vec![Remove])
        );
        assert_eq!(
            v(&s(Some(Installed::Own(None)), found("9.0.3"))),
            ("버전 알 수 없음".into(), vec![Update, Remove])
        );
        assert_eq!(
            v(&own("9.0.2", found("2025-01"))),
            ("9.0.2 · 최신 2025-01".into(), vec![Update, Remove])
        );
        let busy = EssentialState {
            busy: Some(0.42),
            ..own("9.0.2", found("9.0.3"))
        };
        assert_eq!(v(&busy), ("받는 중 42%".into(), vec![]));
    }

    #[test]
    fn ffmpeg_version_lines() {
        let p = parse_ffmpeg_version;
        assert_eq!(
            p("ffmpeg version 9.0.2-essentials_build-www.gyan.dev Copyright (c) 2000-2026"),
            Some("9.0.2".into())
        );
        assert_eq!(
            p("ffmpeg version n7.1-3-gabc Copyright"),
            Some("7.1".into())
        );
        assert_eq!(p("ffmpeg version 8.0 Copyright"), Some("8.0".into()));
        assert_eq!(p("ffmpeg version N-12345-g1a2b3c Copyright"), None);
        assert_eq!(p("ffmpeg version 2024-05-01-git-abc Copyright"), None);
        assert_eq!(p(""), None);
        assert_eq!(p("Microsoft Windows [Version 10.0.26300]"), None);
    }

    #[test]
    fn version_comparison() {
        assert_eq!(is_newer("9.0.2", "9.0.2"), Some(false));
        assert_eq!(is_newer("9.0.3", "9.0.2"), Some(true));
        assert_eq!(is_newer("9.1", "9.0.2"), Some(true));
        assert_eq!(is_newer("9.0", "9.0.2"), Some(false));
        assert_eq!(is_newer("9.0.10", "9.0.9"), Some(true));
        assert_eq!(is_newer("8.1", "9.0.2"), Some(false));
        assert_eq!(is_newer("2025-01", "9.0.2"), None);
    }

    #[test]
    fn refusals() {
        use TileDecision::*;
        let running = Refuse(ffmpeg_release::text::RUNNING);
        let busy = Refuse(update::text::BUSY);
        assert_eq!(on_action(Install, false, true), running);
        assert_eq!(on_action(Remove, true, true), running);
        assert_eq!(on_action(Remove, true, false), busy);
        assert_eq!(on_action(Update, true, false), busy);
        assert_eq!(on_action(Install, true, false), Start);
        assert_eq!(on_action(InstallOwn, true, false), Start);
        assert_eq!(on_action(Remove, false, false), Start);
    }

    #[test]
    fn texts() {
        assert_eq!(
            (header(false), header(true)),
            ("필수 앱".into(), "필수 앱 · 확인 중…".into())
        );
        let dir = Path::new(r"C:\SW\ffmpeg");
        assert_eq!(
            remove_prompt(Some("9.0.2"), dir, true),
            "SwissWheel이 받은 FFmpeg 9.0.2를 지울까요?\n\n위치: C:\\SW\\ffmpeg\n\n지운 뒤에는 시스템 FFmpeg를 씁니다"
        );
        assert_eq!(
            remove_prompt(None, dir, false),
            "SwissWheel이 받은 FFmpeg를 지울까요?\n\n위치: C:\\SW\\ffmpeg"
        );
        assert_eq!(
            [Install, InstallOwn, Update, Remove].map(EssentialAction::label),
            ["설치", "전용 설치", "업데이트", "제거"]
        );
        assert!(ESSENTIALS.len() <= crate::settings::MAX_ROWS);
        assert_eq!(
            (Essential::Ffmpeg.label(), Essential::Ffmpeg.purpose()),
            ("FFmpeg", "영상·오디오")
        );
    }
}
