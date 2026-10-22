//! The restart tile: when it may run, and the hand-off argument the new
//! process gets.

use crate::update::{TileDecision, text::BUSY};

/// The title of every restart notification.
pub const TITLE: &str = "재시작";
/// `--restarted <old pid>`: the new process waits for the old one to exit.
pub const ARG: &str = "--restarted";

/// The old process's id from `--restarted <pid>`.
pub fn restarted_pid<I: IntoIterator<Item = String>>(args: I) -> Option<u32> {
    let mut args = args.into_iter();
    args.find(|a| a == ARG)?;
    args.next()?.parse().ok()
}

/// The restart tile was pressed. `background` is an update or FFmpeg
/// download thread, or an update waiting to be swapped in.
pub fn on_tile(active_jobs: usize, background: bool) -> TileDecision {
    if active_jobs > 0 {
        TileDecision::Refuse(BUSY)
    } else if background {
        TileDecision::Refuse(text::BACKGROUND)
    } else {
        TileDecision::Start
    }
}

/// Notification bodies (all titled [`TITLE`]).
pub mod text {
    pub const BACKGROUND: &str = "업데이트나 다운로드가 끝난 뒤 다시 눌러 주세요";
    pub const RESTARTED: &str = "다시 시작했습니다";
    pub const FAILED: &str = "다시 시작하지 못했습니다";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::{self, TileDecision};

    #[test]
    fn restarted_arg() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(restarted_pid(args(&["exe", "--restarted", "42"])), Some(42));
        assert_eq!(restarted_pid(args(&["exe", "--restarted", "x"])), None);
        assert_eq!(restarted_pid(args(&["exe", "--after-update", "42"])), None);
        assert_eq!(restarted_pid(args(&["exe"])), None);
    }

    #[test]
    fn tile_refuses_while_anything_runs() {
        assert_eq!(on_tile(1, false), TileDecision::Refuse(update::text::BUSY));
        assert_eq!(on_tile(1, true), TileDecision::Refuse(update::text::BUSY));
        assert_eq!(on_tile(0, true), TileDecision::Refuse(text::BACKGROUND));
        assert_eq!(on_tile(0, false), TileDecision::Start);
    }
}
