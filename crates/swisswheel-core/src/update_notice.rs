//! The check a minute after start (update notice spec): whether a newer
//! SwissWheel or FFmpeg is worth a card, and the card itself.

use std::time::Duration;

use crate::essentials::EssentialState;
use crate::essentials::{Installed, Latest, is_newer};
use crate::notices::Notice;
use crate::notices::{NoticeAction, NoticeButton, NoticeKind, Product};
use crate::update::{TITLE, Version};

/// How long after start the check waits: the network is often not up yet
/// right after logging in.
pub const DELAY: Duration = Duration::from_secs(60);

/// A newer SwissWheel than `current`, and newer than the version the user
/// skipped. A skip that isn't a version counts as none.
pub fn app_due(latest: Version, current: Version, skipped: Option<&str>) -> bool {
    latest > current && skipped.and_then(Version::parse).is_none_or(|s| latest > s)
}

/// The FFmpeg release to announce: only for the app's own copy whose
/// version is known, newer than it and than a skipped version (one that
/// can't be compared counts as none).
pub fn ffmpeg_due(state: &EssentialState, skipped: Option<&str>) -> Option<String> {
    let (Some(Installed::Own(Some(installed))), Latest::Found(latest)) =
        (&state.installed, &state.latest)
    else {
        return None;
    };
    let past_skip = skipped.and_then(|s| is_newer(latest, s)).unwrap_or(true);
    (is_newer(latest, installed) == Some(true) && past_skip).then(|| latest.clone())
}

/// The card for `version` (without a leading `v`): [업데이트] then
/// [이 버전 건너뛰기]; a click elsewhere only closes it.
pub fn notice(product: Product, version: &str) -> Notice {
    let body = match product {
        Product::App => match Version::parse(version) {
            Some(v) => text::app_available(v),
            None => format!("SwissWheel v{version}이 나왔습니다"),
        },
        Product::Ffmpeg => text::ffmpeg_available(version),
    };
    Notice {
        kind: NoticeKind::Update,
        title: TITLE.to_string(),
        body,
        action: None,
        buttons: vec![
            NoticeButton {
                label: text::UPDATE,
                action: NoticeAction::Update(product),
            },
            NoticeButton {
                label: text::SKIP,
                action: NoticeAction::Skip(product, version.to_string()),
            },
        ],
    }
}

pub mod text {
    use crate::update::{Version, subject_particle};

    pub const UPDATE: &str = "업데이트";
    pub const SKIP: &str = "이 버전 건너뛰기";

    pub fn app_available(v: Version) -> String {
        format!("SwissWheel {} 나왔습니다", v.with_subject())
    }

    pub fn ffmpeg_available(tag: &str) -> String {
        let last = tag.chars().rev().find_map(|c| c.to_digit(10)).unwrap_or(0);
        format!("FFmpeg {tag}{} 나왔습니다", subject_particle(last))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    fn own(installed: &str, latest: Latest) -> EssentialState {
        EssentialState {
            installed: Some(Installed::Own(Some(installed.into()))),
            latest,
            busy: None,
        }
    }

    #[test]
    fn app_due_follows_current_and_skip() {
        assert!(!app_due(v("0.2.0"), v("0.2.0"), None));
        assert!(app_due(v("0.3.0"), v("0.2.0"), None));
        assert!(!app_due(v("0.3.0"), v("0.2.0"), Some("0.3.0")));
        assert!(app_due(v("0.3.1"), v("0.2.0"), Some("0.3.0")));
        assert!(!app_due(v("0.3.0"), v("0.2.0"), Some("v0.4.0")));
    }

    #[test]
    fn unparsable_skip_counts_as_none() {
        assert!(app_due(v("0.3.0"), v("0.2.0"), Some("latest")));
        let s = own("8.0", Latest::Found("8.0.1".into()));
        assert_eq!(ffmpeg_due(&s, Some("nightly")), Some("8.0.1".into()));
    }

    #[test]
    fn ffmpeg_due_only_for_the_apps_own_copy() {
        let found = || Latest::Found("8.0.1".into());
        assert_eq!(ffmpeg_due(&own("8.0", found()), None), Some("8.0.1".into()));
        assert_eq!(ffmpeg_due(&own("8.0.1", found()), None), None);
        assert_eq!(ffmpeg_due(&own("8.0", found()), Some("8.0.1")), None);
        assert_eq!(
            ffmpeg_due(&own("8.0", Latest::Found("8.1".into())), Some("8.0.1")),
            Some("8.1".into())
        );
        assert_eq!(ffmpeg_due(&own("8.0", Latest::Failed), None), None);
        assert_eq!(ffmpeg_due(&own("8.0", Latest::Unknown), None), None);
        let system = EssentialState {
            installed: Some(Installed::System(Some("7.0".into()))),
            latest: found(),
            busy: None,
        };
        assert_eq!(ffmpeg_due(&system, None), None);
        let unknown = EssentialState {
            installed: Some(Installed::Own(None)),
            latest: found(),
            busy: None,
        };
        assert_eq!(ffmpeg_due(&unknown, None), None);
    }

    #[test]
    fn bodies_take_the_right_particle() {
        assert_eq!(
            text::app_available(v("0.3.0")),
            "SwissWheel v0.3.0이 나왔습니다"
        );
        assert_eq!(
            text::app_available(v("0.3.2")),
            "SwissWheel v0.3.2가 나왔습니다"
        );
        assert_eq!(text::ffmpeg_available("9.0.3"), "FFmpeg 9.0.3이 나왔습니다");
        assert_eq!(text::ffmpeg_available("8.0.5"), "FFmpeg 8.0.5가 나왔습니다");
    }

    #[test]
    fn the_card_has_update_then_skip() {
        let n = notice(Product::Ffmpeg, "9.0.3");
        assert_eq!(
            (n.kind, n.title.as_str(), n.body.as_str()),
            (NoticeKind::Update, "업데이트", "FFmpeg 9.0.3이 나왔습니다")
        );
        assert_eq!(n.action, None);
        assert_eq!(
            n.buttons,
            [
                NoticeButton {
                    label: "업데이트",
                    action: NoticeAction::Update(Product::Ffmpeg)
                },
                NoticeButton {
                    label: "이 버전 건너뛰기",
                    action: NoticeAction::Skip(Product::Ffmpeg, "9.0.3".into())
                },
            ]
        );
        assert_eq!(
            notice(Product::App, "0.3.0").body,
            "SwissWheel v0.3.0이 나왔습니다"
        );
        assert_eq!(DELAY, Duration::from_secs(60));
    }
}
