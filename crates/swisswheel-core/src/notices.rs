//! Rules for the stack of notice cards that replaces balloon notifications.
//!
//! Pure logic: time comes in as `Instant` arguments, drawing lives elsewhere.

use std::path::PathBuf;
use std::time::{Duration, Instant};

const APPEAR: Duration = Duration::from_millis(200);
const CLOSE: Duration = Duration::from_millis(150);
const MORPH: Duration = Duration::from_millis(200);
const FRAME: Duration = Duration::from_millis(16);
/// How often an idle card's timer bar is redrawn.
const BAR_STEP: Duration = Duration::from_millis(50);
/// Most cards shown at once (cards that are fading out do not count).
const MAX_CARDS: usize = 3;

/// What a card is about; decides how long it stays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    Done,
    Failed,
    Info,
    Warning,
    KeyPrompt,
    KeyResult,
    /// A newer version is out; carries [업데이트] and [이 버전 건너뛰기].
    Update,
}

impl NoticeKind {
    /// How long the card stays before it closes by itself.
    /// `KeyPrompt` must equal the app's `CAPTURE_TIMEOUT` (crates/swisswheel/src/app.rs).
    pub fn lifetime(self) -> Duration {
        let secs = match self {
            NoticeKind::Done | NoticeKind::Info | NoticeKind::KeyPrompt => 5,
            NoticeKind::Failed | NoticeKind::Warning => 8,
            NoticeKind::KeyResult => 3,
            NoticeKind::Update => 15,
        };
        Duration::from_secs(secs)
    }
}

/// What an update notice is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Product {
    App,
    Ffmpeg,
}

/// What a click on a card, or on one of its buttons, does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoticeAction {
    OpenFolder(PathBuf),
    CancelCapture,
    /// Start the update, as the update tile or the download panel does.
    Update(Product),
    /// Don't announce this version (or older) again.
    Skip(Product, String),
}

/// A button on a card, in a row under its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoticeButton {
    pub label: &'static str,
    pub action: NoticeAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub kind: NoticeKind,
    pub title: String,
    pub body: String,
    /// A click on the card outside its buttons.
    pub action: Option<NoticeAction>,
    pub buttons: Vec<NoticeButton>,
}

pub type NoticeId = u64;

/// One card as the UI should draw it right now.
#[derive(Debug, Clone, PartialEq)]
pub struct CardView {
    pub id: NoticeId,
    pub kind: NoticeKind,
    pub title: String,
    pub body: String,
    pub action_label: Option<&'static str>,
    /// The buttons' labels, left to right.
    pub buttons: Vec<&'static str>,
    /// 1 -> 0 over the card's life (timer bar); paused time is not counted.
    pub remaining: f32,
    /// 0 -> 1 over 200 ms after posting.
    pub appear: f32,
    /// 0 -> 1 over 150 ms while fading out.
    pub closing: Option<f32>,
    /// 0 -> 1 over 200 ms when this card took the progress card's slot.
    pub from_progress: Option<f32>,
}

#[derive(Debug)]
struct Card {
    id: NoticeId,
    notice: Notice,
    posted: Instant,
    paused_at: Option<Instant>,
    paused_total: Duration,
    closing_since: Option<Instant>,
    morph_since: Option<Instant>,
}

impl Card {
    /// Time on the card's clock: wall time since posting minus paused time.
    fn elapsed(&self, now: Instant) -> Duration {
        let end = self.paused_at.unwrap_or(now);
        end.saturating_duration_since(self.posted)
            .saturating_sub(self.paused_total)
    }

    /// The instant the timer runs out; `None` while paused.
    fn expiry(&self) -> Option<Instant> {
        match self.paused_at {
            Some(_) => None,
            None => Some(self.posted + self.paused_total + self.notice.kind.lifetime()),
        }
    }

    /// When the fade started: by click, dismiss or overflow, or by the timer running out.
    fn closing_start(&self, now: Instant) -> Option<Instant> {
        self.closing_since
            .or_else(|| self.expiry().filter(|&e| e <= now))
    }

    fn animating(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.posted) < APPEAR
            || self.closing_start(now).is_some()
            || self
                .morph_since
                .is_some_and(|m| now.saturating_duration_since(m) < MORPH)
    }
}

pub struct Notices {
    cards: Vec<Card>,
    held: Vec<Notice>,
    hovered: Option<NoticeId>,
    next_id: NoticeId,
    dnd: bool,
    quiet: bool,
}

impl Default for Notices {
    fn default() -> Self {
        Self::new()
    }
}

fn fraction(since: Instant, now: Instant, span: Duration) -> f32 {
    (now.saturating_duration_since(since).as_secs_f32() / span.as_secs_f32()).clamp(0.0, 1.0)
}

impl Notices {
    pub fn new() -> Notices {
        Notices {
            cards: Vec::new(),
            held: Vec::new(),
            hovered: None,
            next_id: 1,
            dnd: false,
            quiet: false,
        }
    }

    pub fn set_do_not_disturb(&mut self, on: bool) {
        self.dnd = on;
    }

    /// Show a card. While quiet it is held instead; a `KeyPrompt` posted while
    /// quiet is dropped (it is only meaningful live) but an id is still returned.
    pub fn post(&mut self, notice: Notice, now: Instant) -> NoticeId {
        self.add(notice, now, false)
    }

    /// Like `post` for a finished job. `None` when do-not-disturb drops it.
    /// The card takes the progress card's slot only for the last job while it is shown.
    pub fn post_job(
        &mut self,
        notice: Notice,
        last_job: bool,
        progress_shown: bool,
        now: Instant,
    ) -> Option<NoticeId> {
        if self.dnd && matches!(notice.kind, NoticeKind::Done | NoticeKind::Failed) {
            return None;
        }
        Some(self.add(notice, now, last_job && progress_shown))
    }

    fn add(&mut self, notice: Notice, now: Instant, morph: bool) -> NoticeId {
        let id = self.next_id;
        self.next_id += 1;
        if self.quiet {
            if notice.kind != NoticeKind::KeyPrompt {
                self.hold(notice);
            }
            return id;
        }
        self.cards.push(Card {
            id,
            notice,
            posted: now,
            paused_at: None,
            paused_total: Duration::ZERO,
            closing_since: None,
            morph_since: morph.then_some(now),
        });
        // The oldest cards beyond the cap start fading right away. A key
        // prompt goes last: capture keeps waiting, so its card must stay.
        let mut live: Vec<usize> = (0..self.cards.len())
            .filter(|&i| self.cards[i].closing_start(now).is_none())
            .collect();
        live.sort_by_key(|&i| self.cards[i].notice.kind == NoticeKind::KeyPrompt);
        let excess = live.len().saturating_sub(MAX_CARDS);
        for &i in &live[..excess] {
            self.cards[i].closing_since = Some(now);
        }
        id
    }

    fn hold(&mut self, notice: Notice) {
        self.held.push(notice);
        if self.held.len() > MAX_CARDS {
            self.held.remove(0);
        }
    }

    /// Start the fade of a card.
    pub fn dismiss(&mut self, id: NoticeId, now: Instant) {
        if let Some(card) = self.cards.iter_mut().find(|c| c.id == id)
            && card.closing_start(now).is_none()
        {
            card.closing_since = Some(now);
        }
    }

    /// The cursor is over `id` (pausing only that card) or over none. A key
    /// prompt doesn't pause: its bar shows the capture timeout, which runs on.
    pub fn hover(&mut self, id: Option<NoticeId>, now: Instant) {
        if self.hovered == id {
            return;
        }
        if let Some(prev) = self.hovered.take()
            && let Some(card) = self.cards.iter_mut().find(|c| c.id == prev)
            && let Some(at) = card.paused_at.take()
        {
            card.paused_total += now.saturating_duration_since(at);
        }
        if let Some(id) = id
            && let Some(card) = self.cards.iter_mut().find(|c| c.id == id)
            && card.closing_start(now).is_none()
        {
            if card.notice.kind != NoticeKind::KeyPrompt {
                card.paused_at = Some(now);
            }
            self.hovered = Some(id);
        }
    }

    /// Click a card, on its button `button` or elsewhere: starts its fade
    /// and returns that button's action, or the card's (also for a button it
    /// doesn't have). A closing or unknown card is ignored, so a click
    /// during the fade cannot act twice.
    pub fn click(
        &mut self,
        id: NoticeId,
        button: Option<usize>,
        now: Instant,
    ) -> Option<NoticeAction> {
        let card = self.cards.iter_mut().find(|c| c.id == id)?;
        if card.closing_start(now).is_some() {
            return None;
        }
        card.closing_since = Some(now);
        match button.and_then(|i| card.notice.buttons.get(i)) {
            Some(b) => Some(b.action.clone()),
            None => card.notice.action.clone(),
        }
    }

    /// Going quiet holds visible cards (key prompts are dropped) and hides
    /// everything; leaving it re-posts the newest three at `now` with full timers.
    pub fn set_quiet(&mut self, quiet: bool, now: Instant) {
        if quiet == self.quiet {
            return;
        }
        self.quiet = quiet;
        if quiet {
            self.hovered = None;
            for card in std::mem::take(&mut self.cards) {
                if card.closing_start(now).is_none() && card.notice.kind != NoticeKind::KeyPrompt {
                    self.hold(card.notice);
                }
            }
        } else {
            for notice in std::mem::take(&mut self.held) {
                self.add(notice, now, false);
            }
        }
    }

    pub fn quiet(&self) -> bool {
        self.quiet
    }

    /// Start fades of expired cards and drop fully faded ones.
    pub fn tick(&mut self, now: Instant) {
        for card in &mut self.cards {
            card.closing_since = card.closing_start(now);
        }
        self.cards
            .retain(|c| c.closing_since.is_none_or(|s| now < s + CLOSE));
        if let Some(h) = self.hovered
            && !self.cards.iter().any(|c| c.id == h)
        {
            self.hovered = None;
        }
    }

    /// Cards to draw, nearest the progress card (the newest) first. Empty while quiet.
    pub fn cards(&self, now: Instant) -> Vec<CardView> {
        if self.quiet {
            return Vec::new();
        }
        self.cards
            .iter()
            .rev()
            .filter_map(|c| {
                let closing = c.closing_start(now).map(|s| fraction(s, now, CLOSE));
                if closing == Some(1.0) {
                    return None;
                }
                let life = c.notice.kind.lifetime().as_secs_f32();
                Some(CardView {
                    id: c.id,
                    kind: c.notice.kind,
                    title: c.notice.title.clone(),
                    body: c.notice.body.clone(),
                    action_label: match c.notice.action {
                        Some(NoticeAction::OpenFolder(_)) => Some("클릭하면 폴더 열기"),
                        _ => None,
                    },
                    buttons: c.notice.buttons.iter().map(|b| b.label).collect(),
                    remaining: (1.0 - c.elapsed(now).as_secs_f32() / life).clamp(0.0, 1.0),
                    appear: fraction(c.posted, now, APPEAR),
                    closing,
                    from_progress: c.morph_since.map(|m| fraction(m, now, MORPH)),
                })
            })
            .collect()
    }

    /// When the app should next call `tick` and repaint: one frame ahead while
    /// anything animates, else the next timer-bar step (50 ms) or expiry, whichever
    /// is sooner. `None` when nothing is shown or only paused cards remain, so
    /// callers must re-arm after `hover`.
    pub fn next_wake(&self, now: Instant) -> Option<Instant> {
        if self.quiet || self.cards.is_empty() {
            return None;
        }
        if self.cards.iter().any(|c| c.animating(now)) {
            return Some(now + FRAME);
        }
        let expiry = self.cards.iter().filter_map(Card::expiry).min()?;
        Some(expiry.min(now + BAR_STEP))
    }

    /// Empties the stack and the held queue, returning the notices of the
    /// cards not fading out and the held ones, oldest first (to show them
    /// some other way).
    pub fn take_all(&mut self, now: Instant) -> Vec<Notice> {
        self.hovered = None;
        std::mem::take(&mut self.cards)
            .into_iter()
            .filter(|c| c.closing_start(now).is_none())
            .map(|c| c.notice)
            .chain(std::mem::take(&mut self.held))
            .collect()
    }

    /// Restart a live card's timer as if posted at `now`, without a new appear
    /// animation. `false` when the card is gone or fading out.
    pub fn restart(&mut self, id: NoticeId, now: Instant) -> bool {
        let Some(card) = self.cards.iter_mut().find(|c| c.id == id) else {
            return false;
        };
        if card.closing_start(now).is_some() {
            return false;
        }
        card.paused_at = None;
        card.paused_total = now.saturating_duration_since(card.posted);
        if self.hovered == Some(id) {
            self.hovered = None;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn note(kind: NoticeKind, title: &str) -> Notice {
        Notice {
            kind,
            title: title.to_string(),
            body: String::new(),
            action: None,
            buttons: Vec::new(),
        }
    }

    fn done(title: &str) -> Notice {
        note(NoticeKind::Done, title)
    }

    #[test]
    fn newest_nearest_progress_and_max_three() {
        let t0 = Instant::now();
        let mut n = Notices::new();
        let a = n.post(done("A"), t0);
        let b = n.post(done("B"), t0);
        let c = n.post(done("C"), t0);
        let d = n.post(done("D"), t0);
        let cards = n.cards(t0);
        assert_eq!(
            cards.iter().map(|c| c.id).collect::<Vec<_>>(),
            vec![d, c, b, a]
        );
        assert!(cards[3].closing.is_some());
        assert!(cards[..3].iter().all(|c| c.closing.is_none()));
        n.tick(t0 + ms(151));
        let ids: Vec<_> = n.cards(t0 + ms(151)).iter().map(|c| c.id).collect();
        assert_eq!(ids, vec![d, c, b]);
    }

    #[test]
    fn the_cap_evicts_the_oldest_card_that_is_not_the_key_prompt() {
        let t0 = Instant::now();
        let mut n = Notices::new();
        let prompt = n.post(note(NoticeKind::KeyPrompt, "바꿀 키를 누르세요"), t0);
        let a = n.post(done("A"), t0 + ms(1));
        let b = n.post(done("B"), t0 + ms(2));
        let c = n.post(done("C"), t0 + ms(3));
        let now = t0 + ms(3);
        let live: Vec<_> = n
            .cards(now)
            .iter()
            .filter(|c| c.closing.is_none())
            .map(|c| c.id)
            .collect();
        assert_eq!(live, vec![c, b, prompt]);
        let closing: Vec<_> = n
            .cards(now)
            .iter()
            .filter(|c| c.closing.is_some())
            .map(|c| c.id)
            .collect();
        assert_eq!(closing, vec![a]);
    }

    #[test]
    fn lifetimes_per_kind() {
        let t0 = Instant::now();
        for (kind, secs) in [
            (NoticeKind::Done, 5),
            (NoticeKind::Failed, 8),
            (NoticeKind::KeyResult, 3),
        ] {
            let mut n = Notices::new();
            n.post(note(kind, "x"), t0);
            let before = t0 + Duration::from_secs(secs) - ms(100);
            n.tick(before);
            assert_eq!(n.cards(before).len(), 1);
            assert!(n.cards(before)[0].closing.is_none());
            let after = t0 + Duration::from_secs(secs) + ms(150);
            n.tick(after);
            assert!(n.cards(after).is_empty(), "{kind:?}");
        }
        let mut n = Notices::new();
        n.post(done("x"), t0);
        let r = n.cards(t0 + ms(2500))[0].remaining;
        assert!((r - 0.5).abs() < 0.01);
    }

    #[test]
    fn hover_pauses_only_that_card() {
        let t0 = Instant::now();
        let mut n = Notices::new();
        let a = n.post(done("A"), t0);
        n.post(done("B"), t0 + ms(1000));
        n.hover(Some(a), t0 + ms(1000));
        // B expires at t0+6s and is gone after its fade; A stays while hovered.
        n.tick(t0 + ms(6200));
        let cards = n.cards(t0 + ms(6200));
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].id, a);
        n.hover(None, t0 + ms(11000));
        let r = n.cards(t0 + ms(11000))[0].remaining;
        assert!((r - 0.8).abs() < 0.01, "A has 4 s left, got {r}");
        n.tick(t0 + ms(14900));
        assert_eq!(n.cards(t0 + ms(14900))[0].closing, None);
        n.tick(t0 + ms(15200));
        assert!(n.cards(t0 + ms(15200)).is_empty());
    }

    #[test]
    fn hover_does_not_pause_key_prompt() {
        // Its bar shows the capture timeout, which hovering doesn't stop.
        let t0 = Instant::now();
        let mut n = Notices::new();
        let id = n.post(note(NoticeKind::KeyPrompt, "key"), t0);
        n.hover(Some(id), t0 + ms(1000));
        let r = n.cards(t0 + ms(2500))[0].remaining;
        assert!((r - 0.5).abs() < 0.01, "got {r}");
        assert!(n.next_wake(t0 + ms(2500)).is_some());
        let t = t0 + ms(5200);
        n.tick(t);
        assert!(n.cards(t).is_empty());
    }

    #[test]
    fn click_runs_action_and_closes() {
        let t0 = Instant::now();
        let mut n = Notices::new();
        let p = PathBuf::from("C:/out");
        let mut notice = done("A");
        notice.action = Some(NoticeAction::OpenFolder(p.clone()));
        let id = n.post(notice, t0);
        assert_eq!(
            n.click(id, None, t0 + ms(300)),
            Some(NoticeAction::OpenFolder(p))
        );
        assert!(n.cards(t0 + ms(310))[0].closing.is_some());
    }

    #[test]
    fn click_on_closing_card_is_ignored() {
        let t0 = Instant::now();
        let mut n = Notices::new();
        let mut notice = done("A");
        notice.action = Some(NoticeAction::OpenFolder(PathBuf::from("x")));
        let id = n.post(notice, t0);
        assert!(n.click(id, None, t0 + ms(300)).is_some());
        assert_eq!(n.click(id, None, t0 + ms(350)), None);
        assert_eq!(n.click(999, None, t0 + ms(350)), None);
    }

    #[test]
    fn key_prompt_click_cancels_capture() {
        let t0 = Instant::now();
        let mut n = Notices::new();
        let mut notice = note(NoticeKind::KeyPrompt, "key");
        notice.action = Some(NoticeAction::CancelCapture);
        let id = n.post(notice, t0);
        assert_eq!(n.click(id, None, t0), Some(NoticeAction::CancelCapture));
    }

    #[test]
    fn morph_only_for_last_job() {
        let t0 = Instant::now();
        let mut n = Notices::new();
        n.post_job(done("A"), false, true, t0).unwrap();
        assert_eq!(n.cards(t0)[0].from_progress, None);
        n.post_job(done("B"), true, true, t0).unwrap();
        assert_eq!(n.cards(t0)[0].from_progress, Some(0.0));
        assert_eq!(n.cards(t0 + ms(200))[0].from_progress, Some(1.0));
        n.post_job(done("C"), true, false, t0).unwrap();
        assert_eq!(n.cards(t0)[0].from_progress, None);
    }

    #[test]
    fn dnd_drops_job_cards_only() {
        let t0 = Instant::now();
        let mut n = Notices::new();
        n.set_do_not_disturb(true);
        assert_eq!(n.post_job(done("A"), true, true, t0), None);
        assert_eq!(
            n.post_job(note(NoticeKind::Failed, "F"), true, true, t0),
            None
        );
        assert!(n.cards(t0).is_empty());
        for kind in [
            NoticeKind::Info,
            NoticeKind::Warning,
            NoticeKind::KeyPrompt,
            NoticeKind::KeyResult,
        ] {
            n.post(note(kind, "x"), t0);
        }
        // Four posted; the cap of three pushes the oldest into its fade.
        let cards = n.cards(t0);
        assert_eq!(cards.len(), 4);
        assert_eq!(cards.iter().filter(|c| c.closing.is_none()).count(), 3);
    }

    #[test]
    fn quiet_holds_and_releases_latest_three() {
        let t0 = Instant::now();
        let mut n = Notices::new();
        n.set_quiet(true, t0);
        for i in 0..5 {
            n.post(done(&format!("N{i}")), t0 + Duration::from_secs(i + 1));
        }
        assert!(n.cards(t0 + Duration::from_secs(6)).is_empty());
        let t = t0 + Duration::from_secs(20);
        n.set_quiet(false, t);
        let cards = n.cards(t);
        let titles: Vec<_> = cards.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["N4", "N3", "N2"]);
        assert!(cards.iter().all(|c| c.remaining == 1.0));
    }

    #[test]
    fn quiet_drops_key_prompt() {
        let t0 = Instant::now();
        let mut n = Notices::new();
        n.post(note(NoticeKind::KeyPrompt, "key"), t0);
        n.set_quiet(true, t0 + ms(100));
        n.set_quiet(false, t0 + ms(200));
        assert!(n.cards(t0 + ms(200)).is_empty());
        // A prompt posted while quiet is dropped too, though post still returns an id.
        n.set_quiet(true, t0 + ms(300));
        n.post(note(NoticeKind::KeyPrompt, "key"), t0 + ms(300));
        n.set_quiet(false, t0 + ms(400));
        assert!(n.cards(t0 + ms(400)).is_empty());
    }

    #[test]
    fn next_wake_ticks_while_a_timer_runs() {
        let t0 = Instant::now();
        let mut n = Notices::new();
        assert_eq!(n.next_wake(t0), None);
        let a = n.post(done("A"), t0);
        assert_eq!(n.next_wake(t0 + ms(10)), Some(t0 + ms(10) + ms(16)));
        assert_eq!(n.next_wake(t0 + ms(300)), Some(t0 + ms(350)));
        let near = t0 + ms(4970);
        assert_eq!(n.next_wake(near), Some(t0 + Duration::from_secs(5)));
        n.hover(Some(a), t0 + ms(300));
        assert_eq!(n.next_wake(t0 + ms(400)), None);
    }

    #[test]
    fn restart_resets_timer() {
        let t0 = Instant::now();
        let mut n = Notices::new();
        let id = n.post(note(NoticeKind::KeyPrompt, "key"), t0);
        assert!(n.restart(id, t0 + Duration::from_secs(4)));
        let t = t0 + ms(8900);
        n.tick(t);
        let c = &n.cards(t)[0];
        assert!(c.closing.is_none());
        assert_eq!(c.appear, 1.0);
        let t = t0 + ms(9100);
        n.tick(t);
        assert!(n.cards(t)[0].closing.is_some());
        // Gone or fading: the caller posts a new card instead.
        assert!(!n.restart(id, t));
        assert!(!n.restart(999, t));
    }

    #[test]
    fn take_all_returns_live_and_held_newest_last() {
        let t0 = Instant::now();
        let mut n = Notices::new();
        let mut a = done("A");
        a.action = Some(NoticeAction::OpenFolder(PathBuf::from("C:/a")));
        let a_id = n.post(a.clone(), t0);
        let gone = n.post(done("gone"), t0);
        n.post(done("B"), t0);
        n.click(gone, None, t0); // fading: not taken
        let taken = n.take_all(t0 + ms(10));
        assert_eq!(
            taken.iter().map(|x| x.title.as_str()).collect::<Vec<_>>(),
            ["A", "B"]
        );
        assert_eq!(taken[0].action, a.action);
        assert!(n.cards(t0 + ms(10)).is_empty());
        assert_eq!(n.click(a_id, None, t0 + ms(20)), None);
        // Held notices come back too.
        n.set_quiet(true, t0 + ms(30));
        n.post(done("H"), t0 + ms(40));
        let taken = n.take_all(t0 + ms(50));
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].title, "H");
        n.set_quiet(false, t0 + ms(60));
        assert!(n.cards(t0 + ms(60)).is_empty());
    }

    #[test]
    fn posting_while_hovered_runs_the_new_timer() {
        let t0 = Instant::now();
        let mut n = Notices::new();
        let a = n.post(done("A"), t0);
        n.hover(Some(a), t0);
        let b = n.post(done("B"), t0 + ms(1000));
        let cards = n.cards(t0 + ms(3500));
        let bv = cards.iter().find(|c| c.id == b).unwrap();
        assert!((bv.remaining - 0.5).abs() < 0.01);
        let t = t0 + ms(6100);
        n.tick(t);
        let cards = n.cards(t);
        assert!(cards.iter().find(|c| c.id == b).unwrap().closing.is_some());
        let av = cards.iter().find(|c| c.id == a).unwrap();
        assert!(av.closing.is_none());
    }

    fn update_card() -> Notice {
        Notice {
            kind: NoticeKind::Update,
            title: "업데이트".into(),
            body: "SwissWheel v0.3.0이 나왔습니다".into(),
            action: None,
            buttons: vec![
                NoticeButton {
                    label: "업데이트",
                    action: NoticeAction::Update(Product::App),
                },
                NoticeButton {
                    label: "이 버전 건너뛰기",
                    action: NoticeAction::Skip(Product::App, "0.3.0".into()),
                },
            ],
        }
    }

    #[test]
    fn buttons_run_their_own_action() {
        let t0 = Instant::now();
        let mut n = Notices::new();
        let id = n.post(update_card(), t0);
        assert_eq!(n.cards(t0)[0].buttons, ["업데이트", "이 버전 건너뛰기"]);
        assert_eq!(
            n.click(id, Some(1), t0),
            Some(NoticeAction::Skip(Product::App, "0.3.0".into()))
        );
        assert!(n.cards(t0 + ms(10))[0].closing.is_some());
        let id = n.post(update_card(), t0);
        assert_eq!(
            n.click(id, Some(0), t0),
            Some(NoticeAction::Update(Product::App))
        );
        let id = n.post(update_card(), t0);
        assert_eq!(n.click(id, None, t0), None); // the card itself only closes
    }

    #[test]
    fn unknown_button_acts_like_the_card() {
        let t0 = Instant::now();
        let mut n = Notices::new();
        let mut card = update_card();
        card.action = Some(NoticeAction::CancelCapture);
        let id = n.post(card, t0);
        assert_eq!(n.click(id, Some(7), t0), Some(NoticeAction::CancelCapture));
    }

    #[test]
    fn button_click_on_closing_card_is_ignored() {
        let t0 = Instant::now();
        let mut n = Notices::new();
        let id = n.post(update_card(), t0);
        assert!(n.click(id, Some(0), t0).is_some());
        assert_eq!(n.click(id, Some(1), t0 + ms(50)), None);
    }

    #[test]
    fn update_cards_stay_fifteen_seconds() {
        assert_eq!(NoticeKind::Update.lifetime(), Duration::from_secs(15));
    }

    #[test]
    fn quiet_release_keeps_buttons() {
        let t0 = Instant::now();
        let mut n = Notices::new();
        n.set_quiet(true, t0);
        n.post(update_card(), t0);
        n.set_quiet(false, t0 + ms(100));
        let cards = n.cards(t0 + ms(100));
        assert_eq!(cards[0].buttons.len(), 2);
        assert_eq!(
            n.click(cards[0].id, Some(0), t0 + ms(100)),
            Some(NoticeAction::Update(Product::App))
        );
    }
}
