//! Update checks against GitHub Releases: versions, the release JSON, the
//! checksum file, and the text and decisions around them.

use std::fmt;

pub const ENDPOINT: &str = "https://api.github.com/repos/glglekdy/SwissWheel/releases/latest";
pub const EXE_ASSET: &str = "swisswheel.exe";
pub const SHA_ASSET: &str = "swisswheel.exe.sha256";
/// Release notes beyond this many characters are cut in the prompt.
pub const NOTES_LIMIT: usize = 600;
/// The title of every update notification.
pub const TITLE: &str = "업데이트";

/// A release version, `major.minor.patch`; tags may carry a leading `v`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.strip_prefix('v').unwrap_or(s);
        let mut parts = s.split('.');
        let mut next = || -> Option<u32> {
            let part = parts.next()?;
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            part.parse().ok()
        };
        let version = Version {
            major: next()?,
            minor: next()?,
            patch: next()?,
        };
        parts.next().is_none().then_some(version)
    }

    /// `v0.2.0이` / `v0.2.2가`: the subject particle for the version as read
    /// aloud, which ends on its last digit.
    fn with_subject(self) -> String {
        // 영 일 삼 육 칠 팔 end in a consonant; 이 사 오 구 do not.
        let particle = if matches!(self.patch % 10, 0 | 1 | 3 | 6 | 7 | 8) {
            "이"
        } else {
            "가"
        };
        format!("v{self}{particle}")
    }

    /// `v0.2.0으로` / `v0.2.1로`: 로 after a vowel or ㄹ (일 칠 팔).
    fn with_direction(self) -> String {
        let particle = if matches!(self.patch % 10, 0 | 3 | 6) {
            "으로"
        } else {
            "로"
        };
        format!("v{self}{particle}")
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// What the app needs from GitHub's "latest release" response.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: Version,
    pub notes: String,
    pub page_url: String,
    pub exe_url: Option<String>,
    pub sha_url: Option<String>,
}

/// `None` when the text isn't JSON or the tag isn't a version.
pub fn parse_release(json: &str) -> Option<Release> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let version = Version::parse(value.get("tag_name")?.as_str()?)?;
    let page_url = value.get("html_url")?.as_str()?.to_string();
    let notes = value
        .get("body")
        .and_then(|b| b.as_str())
        .unwrap_or_default()
        .to_string();
    let asset = |name: &str| {
        value
            .get("assets")?
            .as_array()?
            .iter()
            .find(|a| a.get("name").and_then(|n| n.as_str()) == Some(name))?
            .get("browser_download_url")?
            .as_str()
            .map(str::to_string)
    };
    Some(Release {
        version,
        notes,
        page_url,
        exe_url: asset(EXE_ASSET),
        sha_url: asset(SHA_ASSET),
    })
}

/// The SHA-256 in a `.sha256` file: its first token, 64 hex digits in
/// either case, optionally followed by the file name.
pub fn parse_checksum(text: &str) -> Option<[u8; 32]> {
    let hex = text.split_whitespace().next()?.as_bytes();
    if hex.len() != 64 {
        return None;
    }
    let digit = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let mut out = [0u8; 32];
    for (i, pair) in hex.chunks(2).enumerate() {
        out[i] = digit(pair[0])? << 4 | digit(pair[1])?;
    }
    Some(out)
}

/// The confirmation prompt; notes are trimmed and cut at [`NOTES_LIMIT`].
pub fn prompt_body(new: Version, current: Version, notes: &str) -> String {
    let notes = notes.trim();
    let head = format!("{} 있습니다 (지금 v{current}).", new.with_subject());
    if notes.is_empty() {
        return format!("{head}\n\n지금 업데이트할까요?");
    }
    let mut cut: String = notes.chars().take(NOTES_LIMIT).collect();
    if notes.chars().count() > NOTES_LIMIT {
        cut.push('…');
    }
    format!("{head}\n\n{cut}\n\n지금 업데이트할까요?")
}

/// Shown when the exe's folder can't be written (e.g. Program Files).
pub fn not_writable_body(new: Version) -> String {
    format!(
        "{} 있습니다. 이 위치에서는 자동으로 바꿀 수 없어 다운로드 페이지를 엽니다.",
        new.with_subject()
    )
}

/// The old process's id from `--after-update <pid>`.
pub fn after_update_pid<I: IntoIterator<Item = String>>(args: I) -> Option<u32> {
    let mut args = args.into_iter();
    args.find(|a| a == "--after-update")?;
    args.next()?.parse().ok()
}

#[derive(Debug, PartialEq, Eq)]
pub enum TileDecision {
    Start,
    /// Don't start; show this notice instead.
    Refuse(&'static str),
}

/// The update tile was pressed. Running conversions come first: the user
/// has to wait for them either way.
pub fn on_tile(update_running: bool, active_jobs: usize) -> TileDecision {
    if active_jobs > 0 {
        TileDecision::Refuse(text::BUSY)
    } else if update_running {
        TileDecision::Refuse(text::RUNNING)
    } else {
        TileDecision::Start
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ReadyDecision {
    SwapNow,
    WaitForJobs,
}

/// The new exe is downloaded and verified; swapping it in restarts the app,
/// so it waits for running conversions.
pub fn on_ready(active_jobs: usize) -> ReadyDecision {
    if active_jobs == 0 {
        ReadyDecision::SwapNow
    } else {
        ReadyDecision::WaitForJobs
    }
}

/// Notification bodies (all titled [`TITLE`]).
pub mod text {
    use super::Version;

    pub const BUSY: &str = "진행 중인 변환이 끝난 뒤 다시 눌러 주세요";
    pub const RUNNING: &str = "이미 업데이트를 진행하는 중입니다";
    pub const CHECK_FAILED: &str = "업데이트를 확인할 수 없습니다";
    pub const NO_ASSETS: &str = "업데이트 파일을 찾을 수 없습니다";
    pub const DOWNLOAD_FAILED: &str = "업데이트 파일을 받지 못했습니다";
    pub const CORRUPT: &str = "업데이트 파일이 손상되었습니다";
    pub const WAITING: &str = "변환이 끝나면 업데이트합니다";
    pub const SWAP_FAILED: &str = "업데이트하지 못했습니다";

    pub fn up_to_date(v: Version) -> String {
        format!("최신 버전입니다 (v{v})")
    }

    pub fn updated(v: Version) -> String {
        format!("{} 업데이트했습니다", v.with_direction())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn version_parse_and_order() {
        assert!(v("v0.1.0") < v("0.1.1"));
        assert!(v("0.1.1") < v("v0.2.0"));
        assert!(v("v0.2.0") < v("0.10.0"));
        assert_eq!(v("v1.2.3"), v("1.2.3"));
        assert_eq!(v("v0.2.0").to_string(), "0.2.0");
    }

    #[test]
    fn version_rejects_other_shapes() {
        for s in ["", "1.2", "1.2.3-beta", "v1.2.3.4", "vx.1.2"] {
            assert_eq!(Version::parse(s), None, "{s}");
        }
    }

    const GITHUB: &str = r#"{
      "url": "https://api.github.com/repos/glglekdy/SwissWheel/releases/1",
      "html_url": "https://github.com/glglekdy/SwissWheel/releases/tag/v0.2.0",
      "id": 1, "node_id": "RE_x", "tag_name": "v0.2.0", "target_commitish": "main",
      "name": "v0.2.0", "draft": false, "prerelease": false,
      "created_at": "2026-10-22T00:00:00Z", "published_at": "2026-10-22T00:00:00Z",
      "author": { "login": "glglekdy", "id": 7 },
      "assets": [
        { "id": 11, "name": "swisswheel.exe", "size": 5000000, "content_type": "application/octet-stream",
          "browser_download_url": "https://github.com/glglekdy/SwissWheel/releases/download/v0.2.0/swisswheel.exe" },
        { "id": 12, "name": "swisswheel.exe.sha256", "size": 81,
          "browser_download_url": "https://github.com/glglekdy/SwissWheel/releases/download/v0.2.0/swisswheel.exe.sha256" },
        { "id": 13, "name": "other.zip", "size": 9,
          "browser_download_url": "https://github.com/glglekdy/SwissWheel/releases/download/v0.2.0/other.zip" }
      ],
      "tarball_url": "https://api.github.com/x", "zipball_url": "https://api.github.com/y",
      "body": null
    }"#;

    #[test]
    fn parses_real_github_shape_with_null_body() {
        let r = parse_release(GITHUB).unwrap();
        assert_eq!(r.version, v("0.2.0"));
        assert_eq!(r.notes, "");
        assert_eq!(
            r.page_url,
            "https://github.com/glglekdy/SwissWheel/releases/tag/v0.2.0"
        );
        assert_eq!(
            r.exe_url.as_deref(),
            Some("https://github.com/glglekdy/SwissWheel/releases/download/v0.2.0/swisswheel.exe")
        );
        assert_eq!(
            r.sha_url.as_deref(),
            Some(
                "https://github.com/glglekdy/SwissWheel/releases/download/v0.2.0/swisswheel.exe.sha256"
            )
        );
    }

    #[test]
    fn missing_assets_are_none() {
        let json = r#"{"tag_name":"v0.2.0","html_url":"https://x","body":"notes",
            "assets":[{"name":"other.zip","browser_download_url":"https://x/other.zip"}]}"#;
        let r = parse_release(json).unwrap();
        assert_eq!(r.notes, "notes");
        assert_eq!((r.exe_url, r.sha_url), (None, None));
    }

    #[test]
    fn bad_release_json_is_none() {
        assert_eq!(parse_release("not json"), None);
        assert_eq!(
            parse_release(r#"{"tag_name":"latest","html_url":"https://x","assets":[]}"#),
            None
        );
    }

    const HEX: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";

    #[test]
    fn checksum_forms() {
        let bytes = parse_checksum(HEX).unwrap();
        assert_eq!(bytes[0], 0x9f);
        assert_eq!(bytes[31], 0x08);
        assert_eq!(
            parse_checksum(&format!("{HEX}  swisswheel.exe\n")),
            Some(bytes)
        );
        assert_eq!(parse_checksum(&HEX.to_uppercase()), Some(bytes));
        assert_eq!(parse_checksum(&HEX[1..]), None);
        assert_eq!(parse_checksum(&format!("{}g", &HEX[1..])), None);
        assert_eq!(parse_checksum(""), None);
    }

    #[test]
    fn prompt_body_truncates_and_skips_empty_notes() {
        let notes: String = "가".repeat(601);
        let body = prompt_body(v("0.2.0"), v("0.1.0"), &notes);
        assert!(body.contains(&format!("{}…", "가".repeat(600))), "{body}");
        assert!(!body.contains(&"가".repeat(601)));
        for empty in ["", "  \n "] {
            assert_eq!(
                prompt_body(v("0.2.0"), v("0.1.0"), empty),
                "v0.2.0이 있습니다 (지금 v0.1.0).\n\n지금 업데이트할까요?"
            );
        }
        assert_eq!(
            prompt_body(v("0.2.0"), v("0.1.0"), "고침"),
            "v0.2.0이 있습니다 (지금 v0.1.0).\n\n고침\n\n지금 업데이트할까요?"
        );
    }

    #[test]
    fn particles_follow_the_last_digit() {
        assert!(prompt_body(v("0.2.2"), v("0.1.0"), "").starts_with("v0.2.2가 있습니다"));
        assert!(prompt_body(v("0.2.1"), v("0.1.0"), "").starts_with("v0.2.1이 있습니다"));
        assert_eq!(text::updated(v("0.2.1")), "v0.2.1로 업데이트했습니다");
        assert_eq!(text::updated(v("0.2.3")), "v0.2.3으로 업데이트했습니다");
        assert_eq!(
            not_writable_body(v("0.2.5")),
            "v0.2.5가 있습니다. 이 위치에서는 자동으로 바꿀 수 없어 다운로드 페이지를 엽니다."
        );
    }

    #[test]
    fn after_update_arg() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            after_update_pid(args(&["exe", "--after-update", "1234"])),
            Some(1234)
        );
        assert_eq!(
            after_update_pid(args(&["exe", "--after-update", "x"])),
            None
        );
        assert_eq!(after_update_pid(args(&["exe"])), None);
        assert_eq!(after_update_pid(args(&["exe", "--after-update"])), None);
    }

    #[test]
    fn tile_refuses_while_busy() {
        assert_eq!(on_tile(false, 1), TileDecision::Refuse(text::BUSY));
        assert_eq!(on_tile(true, 0), TileDecision::Refuse(text::RUNNING));
        assert_eq!(on_tile(true, 2), TileDecision::Refuse(text::BUSY));
        assert_eq!(on_tile(false, 0), TileDecision::Start);
    }

    #[test]
    fn ready_waits_for_jobs() {
        assert_eq!(on_ready(0), ReadyDecision::SwapNow);
        assert_eq!(on_ready(1), ReadyDecision::WaitForJobs);
    }

    #[test]
    fn texts() {
        assert_eq!(text::up_to_date(v("0.1.0")), "최신 버전입니다 (v0.1.0)");
        assert_eq!(text::updated(v("0.2.0")), "v0.2.0으로 업데이트했습니다");
    }
}
