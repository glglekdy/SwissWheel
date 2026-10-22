//! FFmpeg downloads from GitHub Releases (GyanD's Windows builds): the
//! release JSON, the asset's digest, and the text and decisions around them.

use std::path::Path;

use crate::update::{TileDecision, parse_checksum};

pub const ENDPOINT: &str = "https://api.github.com/repos/GyanD/codexffmpeg/releases/latest";
/// The build that has `ffmpeg.exe` and `ffprobe.exe` and little else.
pub const ZIP_SUFFIX: &str = "-essentials_build.zip";
/// The title of every download notification.
pub const TITLE: &str = "다운로드";
pub const DIALOG_TITLE: &str = "SwissWheel 다운로드";

#[derive(Debug, Clone, PartialEq)]
pub struct FfmpegRelease {
    /// The release tag as GyanD writes it ("9.0.2"); shown, never compared.
    pub tag: String,
    pub zip_url: String,
    pub size: u64,
    pub sha256: [u8; 32],
}

#[derive(Debug, PartialEq)]
pub enum ReleaseError {
    /// Not JSON, or no tag.
    Unreadable,
    /// No essentials zip, or it lacks a URL, size, or SHA-256 digest.
    NoAsset,
}

pub fn parse_release(json: &str) -> Result<FfmpegRelease, ReleaseError> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|_| ReleaseError::Unreadable)?;
    let tag = value
        .get("tag_name")
        .and_then(|t| t.as_str())
        .ok_or(ReleaseError::Unreadable)?
        .to_string();
    let zip = value
        .get("assets")
        .and_then(|a| a.as_array())
        .and_then(|assets| {
            assets.iter().find(|a| {
                a.get("name")
                    .and_then(|n| n.as_str())
                    .is_some_and(|n| n.ends_with(ZIP_SUFFIX))
            })
        })
        .ok_or(ReleaseError::NoAsset)?;
    let field = |name: &str| zip.get(name);
    let zip_url = field("browser_download_url").and_then(|u| u.as_str());
    let size = field("size").and_then(|s| s.as_u64());
    let sha256 = field("digest")
        .and_then(|d| d.as_str())
        .and_then(parse_digest);
    match (zip_url, size, sha256) {
        (Some(zip_url), Some(size), Some(sha256)) => Ok(FfmpegRelease {
            tag,
            zip_url: zip_url.to_string(),
            size,
            sha256,
        }),
        _ => Err(ReleaseError::NoAsset),
    }
}

/// GitHub's asset digest, `sha256:<64 hex digits>`.
pub fn parse_digest(digest: &str) -> Option<[u8; 32]> {
    parse_checksum(digest.strip_prefix("sha256:")?)
}

/// The confirmation prompt; the size in (decimal) megabytes, rounded.
pub fn prompt_body(tag: &str, size: u64, dir: &Path) -> String {
    let mb = (size + 500_000) / 1_000_000;
    format!(
        "FFmpeg {tag}(약 {mb}MB)를 받아 설정할까요?\n\n설치 위치: {}",
        dir.display()
    )
}

/// The download tile was pressed. An FFmpeg the app can already use wins:
/// there is nothing to download either way.
pub fn on_download_tile(ready: bool, busy: bool) -> TileDecision {
    if ready {
        TileDecision::Refuse(text::READY)
    } else if busy {
        TileDecision::Refuse(text::RUNNING)
    } else {
        TileDecision::Start
    }
}

/// Notification bodies (all titled [`TITLE`]).
pub mod text {
    pub const READY: &str = "다운로드 완료 · FFmpeg가 이미 준비되어 있습니다";
    pub const RUNNING: &str = "이미 업데이트나 다운로드를 진행하는 중입니다";
    pub const CHECK_FAILED: &str = "FFmpeg 정보를 확인할 수 없습니다";
    pub const NO_ASSET: &str = "받을 FFmpeg 파일을 찾을 수 없습니다";
    pub const DOWNLOAD_FAILED: &str = "FFmpeg를 받지 못했습니다";
    pub const CORRUPT: &str = "받은 FFmpeg 파일이 손상되었습니다";
    pub const INSTALL_FAILED: &str = "FFmpeg를 설치하지 못했습니다";

    pub fn installed(tag: &str) -> String {
        format!("FFmpeg {tag} 설정을 마쳤습니다")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::TileDecision;
    use std::path::Path;

    const HEX: &str = "60f467265b1e312373dbcd92200c2618a74850f98d3d078e94296bb3fa2047ba";

    fn asset(name: &str, digest: Option<&str>) -> String {
        let digest = digest.map_or("null".into(), |d| format!("\"{d}\""));
        format!(
            r#"{{"name":"{name}","size":114768076,"digest":{digest},"browser_download_url":"https://x/{name}"}}"#
        )
    }

    fn json(assets: &[String]) -> String {
        format!(r#"{{"tag_name":"9.0.2","assets":[{}]}}"#, assets.join(","))
    }

    fn sha() -> String {
        format!("sha256:{HEX}")
    }

    #[test]
    fn picks_only_the_essentials_zip() {
        let j = json(&[
            asset("ffmpeg-9.0.2-essentials_build.7z", Some(&sha())),
            asset("ffmpeg-9.0.2-full_build.zip", Some(&sha())),
            asset("ffmpeg-9.0.2-essentials_build.zip", Some(&sha())),
        ]);
        let r = parse_release(&j).unwrap();
        assert_eq!(r.tag, "9.0.2");
        assert_eq!(r.zip_url, "https://x/ffmpeg-9.0.2-essentials_build.zip");
        assert_eq!(r.size, 114768076);
        assert_eq!(r.sha256[0], 0x60);
    }

    #[test]
    fn missing_zip_or_digest_is_no_asset() {
        let no_zip = json(&[asset("ffmpeg-9.0.2-essentials_build.7z", Some(&sha()))]);
        assert_eq!(parse_release(&no_zip), Err(ReleaseError::NoAsset));
        let no_digest = json(&[asset("ffmpeg-9.0.2-essentials_build.zip", None)]);
        assert_eq!(parse_release(&no_digest), Err(ReleaseError::NoAsset));
        assert_eq!(parse_release("not json"), Err(ReleaseError::Unreadable));
        assert_eq!(
            parse_release(r#"{"assets":[]}"#),
            Err(ReleaseError::Unreadable)
        );
    }

    #[test]
    fn digest_forms() {
        assert!(parse_digest(&sha()).is_some());
        assert_eq!(parse_digest(&format!("sha512:{HEX}")), None);
        assert_eq!(parse_digest(HEX), None);
        assert_eq!(parse_digest("sha256:abc"), None);
    }

    #[test]
    fn prompt_rounds_megabytes() {
        let body = prompt_body("9.0.2", 114_768_076, Path::new(r"D:\앱\ffmpeg"));
        assert_eq!(
            body,
            "FFmpeg 9.0.2(약 115MB)를 받아 설정할까요?\n\n설치 위치: D:\\앱\\ffmpeg"
        );
    }

    #[test]
    fn download_tile() {
        assert_eq!(
            on_download_tile(true, true),
            TileDecision::Refuse(text::READY)
        );
        assert_eq!(
            on_download_tile(false, true),
            TileDecision::Refuse(text::RUNNING)
        );
        assert_eq!(on_download_tile(false, false), TileDecision::Start);
        assert_eq!(text::installed("9.0.2"), "FFmpeg 9.0.2 설정을 마쳤습니다");
    }
}
