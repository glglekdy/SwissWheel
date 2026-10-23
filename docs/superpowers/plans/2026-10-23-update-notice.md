# Update Notice Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One minute after start, check SwissWheel and the app's own FFmpeg for a newer release and show a notice card with [업데이트] and [이 버전 건너뛰기]; a skipped version is not announced again until a newer one appears; a General-tab row turns the check off.

**Architecture:** Pure rules (config section, settings row, card buttons, "should we announce" decisions, copy) live in `swisswheel-core`. The app crate adds a check-only updater thread, draws and hit-tests card buttons in the notice window, and wires a one-shot timer, a result message, and the button actions in `app.rs`. FFmpeg reuses the download panel's status check.

**Tech Stack:** Rust 2024, `windows` crate (Win32, Direct2D/DirectWrite), serde/toml, toml_edit.

**Spec:** `docs/superpowers/specs/2026-10-23-update-notice-design.md`

## Global Constraints

- Check runs once, `DELAY` = 60 s after start; never in a debug preview (`SWISSWHEEL_PREVIEW`); skipped when `config.updates.auto_check` is false at fire time.
- FFmpeg is announced only for `Installed::Own(Some(version))`; a `PATH` FFmpeg never.
- Failed checks: log only, no card.
- Card: `NoticeKind::Update`, lifetime 15 s, title `"업데이트"` (`update::TITLE`), bodies `"SwissWheel v0.3.0이 나왔습니다"` / `"FFmpeg 9.0.3이 나왔습니다"` (subject particle 이/가 by the last digit, as `Version::with_subject` does), buttons `"업데이트"` then `"이 버전 건너뛰기"`.
- Announce only when latest > current AND (no skip OR latest > skip); an unparsable skip counts as none.
- Manual checks (update tile, download panel) ignore skips.
- `config.toml`: `[updates]` with `auto_check = true` (default), `skip_app`, `skip_ffmpeg` (strings, omitted when unset).
- General tab order: `Windows 시작 시 실행`, `업데이트 자동 확인`, `강조색`.
- `WM_APP_NOTICE`: `wparam` 0 = hover, 1 = card click, `2 + i` = button `i` click; `lparam` = id.
- New ids: `WM_APP_AUTO_CHECK = WM_APP + 37`, `TIMER_AUTO_CHECK = 12`.
- Worker threads never show a dialog on the UI thread; results go through a slot + `PostMessageW`.
- Never stage files you didn't change; stage by name.

## Review Focus

1. A card held while full screen and re-posted later must keep its buttons and their actions → Task 2 test `quiet_release_keeps_buttons`.
2. A click on a button while its card fades must do nothing (no second update, no double save) → Task 2 test `button_click_on_closing_card_is_ignored`.
3. A button index the card doesn't have (stale window hits) acts like a card click, not a panic → Task 2 test `unknown_button_acts_like_the_card`.
4. A hand-edited, unparsable `skip_app`/`skip_ffmpeg` must not silence notices forever → Task 3 tests `unparsable_skip_counts_as_none`.
5. The 60 s timer fires while the download panel's check is already running (`checking`): the FFmpeg decision must still be made from the finished report, not dropped → Task 6 ruling + reviewer check (app wiring, no unit seam).

---

### Task 1: Config section and settings row

**Files:**
- Modify: `crates/swisswheel-core/src/config.rs`
- Modify: `crates/swisswheel-core/src/settings.rs`

**Interfaces:**
- Produces: `config::Updates { pub auto_check: bool, pub skip_app: Option<String>, pub skip_ffmpeg: Option<String> }` (`#[serde(default)]`, `Default` gives `auto_check: true`, `skip_*` use `#[serde(skip_serializing_if = "Option::is_none")]`); `Config.updates: Updates`; `SettingItem::AutoCheck` (label `"업데이트 자동 확인"`, value 켬/끔, ←/→ like `StartWithWindows`).

- [ ] **Step 1: Write the failing tests**

In `config.rs` tests:
```rust
#[test]
fn updates_defaults_and_skips() {
    let c = Config::default();
    assert!(c.updates.auto_check);
    assert_eq!((c.updates.skip_app.as_deref(), c.updates.skip_ffmpeg.as_deref()), (None, None));
    assert!(default_toml().contains("[updates]"));
    assert!(!default_toml().contains("skip_"));
    let c = parse("[updates]\nauto_check = false\nskip_app = \"0.3.0\"\nskip_ffmpeg = \"8.0.1\"\n").unwrap();
    assert!(!c.updates.auto_check);
    assert_eq!(c.updates.skip_app.as_deref(), Some("0.3.0"));
    assert_eq!(c.updates.skip_ffmpeg.as_deref(), Some("8.0.1"));
    assert_eq!(parse(&toml::to_string_pretty(&c).unwrap()), Ok(c));
}
```
In `settings.rs`: extend `labels_and_values_in_order` with `("업데이트 자동 확인", "켬")` between `Windows 시작 시 실행` and `강조색`; in `categories_in_order_with_their_items` assert `Category::General.items() == [StartWithWindows, AutoCheck, Accent]`; add:
```rust
#[test]
fn auto_check_toggles() {
    let c = Config::default();
    let off = step(&c, SettingItem::AutoCheck, Dir::Back).unwrap();
    assert!(!off.updates.auto_check);
    assert_eq!(SettingItem::AutoCheck.value(&off), "끔");
    assert_eq!(step(&c, SettingItem::AutoCheck, Dir::Forward), None);
    assert!(step(&off, SettingItem::AutoCheck, Dir::Forward).unwrap().updates.auto_check);
}
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p swisswheel-core`
Expected: compile errors (`updates`, `AutoCheck` unknown).

- [ ] **Step 3: Implement** `Updates`, `Config.updates`, `SettingItem::AutoCheck` (label, value, step, General items).

- [ ] **Step 4: Run** `cargo test -p swisswheel-core` → all pass; `cargo build -p swisswheel` → builds (exhaustive matches in the app crate, if any, get the new item).

- [ ] **Step 5: Commit** `feat(core): updates config section and auto-check setting`

### Task 2: Notice card buttons and the Update kind

**Files:**
- Modify: `crates/swisswheel-core/src/notices.rs`
- Modify: `crates/swisswheel/src/app.rs` (struct literals and `click` calls only, to keep building)
- Modify: `crates/swisswheel/src/notice.rs` (test helper `card()` gets `buttons: vec![]`; `glyph` gets `Update`)

**Interfaces:**
- Produces:
  - `NoticeKind::Update` (lifetime 15 s).
  - `pub enum Product { App, Ffmpeg }` (Debug, Clone, Copy, PartialEq, Eq).
  - `NoticeAction::Update(Product)`, `NoticeAction::Skip(Product, String)`.
  - `pub struct NoticeButton { pub label: &'static str, pub action: NoticeAction }` (Debug, Clone, PartialEq, Eq).
  - `Notice.buttons: Vec<NoticeButton>`.
  - `Notices::click(&mut self, id: NoticeId, button: Option<usize>, now: Instant) -> Option<NoticeAction>`: `Some(i)` within range → that button's action; out of range or `None` → the card's `action`. Closing/unknown card → `None`, as now.
  - `CardView.buttons: Vec<&'static str>`.

- [ ] **Step 1: Write the failing tests** (in `notices.rs`; existing `click(id, now)` calls become `click(id, None, now)`, `note()` gets `buttons: Vec::new()`)

```rust
fn update_card() -> Notice {
    Notice {
        kind: NoticeKind::Update,
        title: "업데이트".into(),
        body: "SwissWheel v0.3.0이 나왔습니다".into(),
        action: None,
        buttons: vec![
            NoticeButton { label: "업데이트", action: NoticeAction::Update(Product::App) },
            NoticeButton { label: "이 버전 건너뛰기", action: NoticeAction::Skip(Product::App, "0.3.0".into()) },
        ],
    }
}

#[test]
fn buttons_run_their_own_action() {
    let t0 = Instant::now();
    let mut n = Notices::new();
    let id = n.post(update_card(), t0);
    assert_eq!(n.cards(t0)[0].buttons, ["업데이트", "이 버전 건너뛰기"]);
    assert_eq!(n.click(id, Some(1), t0), Some(NoticeAction::Skip(Product::App, "0.3.0".into())));
    assert!(n.cards(t0 + ms(10))[0].closing.is_some());
    let id = n.post(update_card(), t0);
    assert_eq!(n.click(id, Some(0), t0), Some(NoticeAction::Update(Product::App)));
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
    assert_eq!(n.click(cards[0].id, Some(0), t0 + ms(100)), Some(NoticeAction::Update(Product::App)));
}
```

- [ ] **Step 2: Run** `cargo test -p swisswheel-core notices` → compile errors.

- [ ] **Step 3: Implement** the core types and `click`. Then in the app crate: add `buttons: Vec::new()` to every `Notice { .. }` literal in `app.rs` (`info`, `key_prompt`, the job notice near line 1429, the two preview cards); `on_notice` and `on_preview_notice` call `click(id, wparam.0.checked_sub(2), now)`; `notice.rs` `glyph(NoticeKind::Update)` = `"\u{E896}"` (Download), the test helper `card()` gets `buttons: vec![]`. `balloon` handles `Update` like `Info` (no change needed if it falls to `_`).

- [ ] **Step 4: Run** `cargo test -p swisswheel-core` and `cargo test -p swisswheel` → all pass.

- [ ] **Step 5: Commit** `feat(notices): cards with buttons and an update kind`

### Task 3: When to announce, and the card

**Files:**
- Create: `crates/swisswheel-core/src/update_notice.rs` (+ `pub mod update_notice;` in `lib.rs`)
- Modify: `crates/swisswheel-core/src/update.rs` (make the 이/가 choice reusable for a version string)

**Interfaces:**
- Consumes: `update::Version`, `update::TITLE`, `essentials::{EssentialState, Installed, Latest, is_newer}`, Task 2's `Notice`, `NoticeButton`, `NoticeAction`, `Product`, `NoticeKind::Update`.
- Produces:
  - `pub const DELAY: Duration` = 60 s.
  - `pub fn app_due(latest: Version, current: Version, skipped: Option<&str>) -> bool`.
  - `pub fn ffmpeg_due(state: &EssentialState, skipped: Option<&str>) -> Option<String>` — the tag to announce.
  - `pub fn notice(product: Product, version: &str) -> Notice` — `version` without `v` (`"0.3.0"`, `"9.0.3"`); the Skip button carries that same string.
  - `pub mod text { pub fn app_available(v: Version) -> String; pub fn ffmpeg_available(tag: &str) -> String; pub const UPDATE: &str = "업데이트"; pub const SKIP: &str = "이 버전 건너뛰기"; }`

- [ ] **Step 1: Write the failing tests** (in `update_notice.rs`)

```rust
fn v(s: &str) -> Version { Version::parse(s).unwrap() }

fn own(installed: &str, latest: Latest) -> EssentialState {
    EssentialState { installed: Some(Installed::Own(Some(installed.into()))), latest, busy: None }
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
    assert_eq!(ffmpeg_due(&own("8.0", Latest::Found("8.1".into())), Some("8.0.1")), Some("8.1".into()));
    assert_eq!(ffmpeg_due(&own("8.0", Latest::Failed), None), None);
    assert_eq!(ffmpeg_due(&own("8.0", Latest::Unknown), None), None);
    let system = EssentialState { installed: Some(Installed::System(Some("7.0".into()))), latest: found(), busy: None };
    assert_eq!(ffmpeg_due(&system, None), None);
    let unknown = EssentialState { installed: Some(Installed::Own(None)), latest: found(), busy: None };
    assert_eq!(ffmpeg_due(&unknown, None), None);
}

#[test]
fn bodies_take_the_right_particle() {
    assert_eq!(text::app_available(v("0.3.0")), "SwissWheel v0.3.0이 나왔습니다");
    assert_eq!(text::app_available(v("0.3.2")), "SwissWheel v0.3.2가 나왔습니다");
    assert_eq!(text::ffmpeg_available("9.0.3"), "FFmpeg 9.0.3이 나왔습니다");
    assert_eq!(text::ffmpeg_available("8.0.5"), "FFmpeg 8.0.5가 나왔습니다");
}

#[test]
fn the_card_has_update_then_skip() {
    let n = notice(Product::Ffmpeg, "9.0.3");
    assert_eq!((n.kind, n.title.as_str(), n.body.as_str()), (NoticeKind::Update, "업데이트", "FFmpeg 9.0.3이 나왔습니다"));
    assert_eq!(n.action, None);
    assert_eq!(n.buttons, [
        NoticeButton { label: "업데이트", action: NoticeAction::Update(Product::Ffmpeg) },
        NoticeButton { label: "이 버전 건너뛰기", action: NoticeAction::Skip(Product::Ffmpeg, "9.0.3".into()) },
    ]);
    assert_eq!(notice(Product::App, "0.3.0").body, "SwissWheel v0.3.0이 나왔습니다");
    assert_eq!(DELAY, Duration::from_secs(60));
}
```

- [ ] **Step 2: Run** `cargo test -p swisswheel-core update_notice` → fails (module missing).

- [ ] **Step 3: Implement.** In `update.rs` factor the particle choice into `pub(crate) fn subject_particle(last_digit: u32) -> &'static str` used by `with_subject`; `ffmpeg_available` takes the tag's last ASCII digit (none → "이"). `notice(Product::App, s)` parses `s` with `Version::parse` for the body and falls back to `"SwissWheel v{s}이 나왔습니다"` only if parsing fails (it won't for app callers). Skip parsing: app `Version::parse`, FFmpeg `is_newer(latest, skip)` where `None` = unparsable = no skip.

- [ ] **Step 4: Run** `cargo test -p swisswheel-core` → all pass.

- [ ] **Step 5: Commit** `feat(core): decide and word update notices`

### Task 4: Check-only updater thread

**Files:**
- Modify: `crates/swisswheel/src/update/mod.rs`

**Interfaces:**
- Produces: `pub fn latest(endpoint: &str) -> Option<Release>` (logs failures the way `run` does now; `run` calls it and maps `None` to `Outcome::CheckFailed`); `pub fn spawn_check(hwnd: isize, msg: u32, slot: Arc<Mutex<Option<Option<Version>>>>)` — a thread that stores `latest(&endpoint()).map(|r| r.version)` and posts `msg`; no dialog.

- [ ] **Step 1: Write the failing test** (in `update/mod.rs` tests, using the existing `release()` helper and `TestServer`)

```rust
#[test]
fn latest_reads_the_release_or_gives_up() {
    let server = TestServer::start(vec![
        ("/latest", release("v0.3.0", true)),
        ("/broken", Reply::Ok(b"not json".to_vec())),
        ("/down", Reply::Status(500)),
    ]);
    let url = |p: &str| format!("{}{p}", server.base);
    assert_eq!(latest(&url("/latest")).map(|r| r.version), Version::parse("0.3.0"));
    assert_eq!(latest(&url("/broken")), None);
    assert_eq!(latest(&url("/down")), None);
}
```

- [ ] **Step 2: Run** `cargo test -p swisswheel update::` → fails (`latest` missing).

- [ ] **Step 3: Implement** `latest`, rewrite `run`'s first block to use it, add `spawn_check`.

- [ ] **Step 4: Run** `cargo test -p swisswheel update::` → all pass (existing `run` tests unchanged).

- [ ] **Step 5: Commit** `feat(update): check for a release without asking`

### Task 5: Buttons in the notice window

**Files:**
- Modify: `crates/swisswheel/src/notice.rs`

**Interfaces:**
- Consumes: Task 2's `CardView.buttons`.
- Produces: `WM_APP_NOTICE` with `wparam = 2 + i` for a click on button `i` (doc comment updated).
- Internal: `const BUTTON: f32 = 26.0; const BUTTON_ROW: f32 = 34.0; const BUTTON_GAP: f32 = 8.0;` — `notice_height(body_lines, action, buttons: bool)` adds `BUTTON_ROW` when `buttons`; `MAX_STACK` uses `notice_height(2, true, true)`; `fn button_rects(card: D2D_RECT_F, count: usize, scale: f32) -> Vec<D2D_RECT_F>` — the row sits right above the timer area (`bottom = card.bottom - (TIMER_ABOVE + TIMER + TIMER_BELOW) * scale`, height `BUTTON * scale`), spans the text column (`left = card.left + (PAD_X + ICON + ICON_GAP) * scale`, `right = card.right - PAD_X * scale`), split into `count` equal parts with `BUTTON_GAP` between; thread-local `BUTTON_HITS: Vec<(NoticeId, usize, D2D_RECT_F)>` filled beside `HITS` for live (not closing) cards; `fn button_at(hits: &[(NoticeId, usize, D2D_RECT_F)], id: NoticeId, x: f32, y: f32) -> Option<usize>`.
- Drawing: first button filled `theme.accent` with `theme.on_accent` text, the others `theme.track` with `theme.text`; corner `BUTTON * scale / 2`; label centered (a centered trimmed format in `Formats`, 11.5 semibold).

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn buttons_add_a_row() {
    assert!(near(notice_height(1, false, true) - notice_height(1, false, false), BUTTON_ROW));
    let mut c = card(1);
    c.buttons = vec!["업데이트", "이 버전 건너뛰기"];
    let s = stack(0, false, &[c], |_| 1);
    assert!(near(s[0].2, notice_height(1, false, true)));
}

#[test]
fn button_rects_split_the_text_column() {
    let r = D2D_RECT_F { left: 0.0, top: 0.0, right: WIDTH, bottom: notice_height(1, false, true) };
    let b = button_rects(r, 2, 1.0);
    assert_eq!(b.len(), 2);
    assert!(near(b[0].left, PAD_X + ICON + ICON_GAP));
    assert!(near(b[1].right, WIDTH - PAD_X));
    assert!(near(b[1].left - b[0].right, BUTTON_GAP));
    assert!(near(b[0].right - b[0].left, b[1].right - b[1].left));
    assert!(near(b[0].bottom, r.bottom - (TIMER_ABOVE + TIMER + TIMER_BELOW)));
    assert!(near(b[0].bottom - b[0].top, BUTTON));
    let hits: Vec<_> = b.iter().enumerate().map(|(i, &r)| (7, i, r)).collect();
    let mid = |r: D2D_RECT_F| ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
    let (x, y) = mid(b[1]);
    assert_eq!(button_at(&hits, 7, x, y), Some(1));
    assert_eq!(button_at(&hits, 8, x, y), None);
    assert_eq!(button_at(&hits, 7, x, b[1].top - 1.0), None);
}
```
(Existing `notice_height(n, action)` calls in tests gain `, false`.)

- [ ] **Step 2: Run** `cargo test -p swisswheel notice::` → compile errors.

- [ ] **Step 3: Implement** height, `button_rects`, `button_at`, `BUTTON_HITS` (cleared wherever `HITS` is), drawing in `draw_notice`, and `WM_LBUTTONDOWN`: a card hit then `button_at` → `post_to_owner(hwnd, 2 + i, id)`, else `post_to_owner(hwnd, 1, id)`.

- [ ] **Step 4: Run** `cargo test -p swisswheel` → all pass; `cargo clippy --workspace --all-targets -- -D warnings` clean.

- [ ] **Step 5: Commit** `feat(notice): draw and click card buttons`

### Task 6: Wire the startup check, actions, and skip

**Files:**
- Modify: `crates/swisswheel/src/app.rs`
- Modify: `crates/swisswheel/src/config_file.rs` (test only)

**Interfaces:**
- Consumes: Tasks 1–5 (`config.updates`, `update_notice::{DELAY, app_due, ffmpeg_due, notice}`, `update::spawn_check`, `NoticeAction::{Update, Skip}`, `Product`).
- Produces (inside `AppState`): `auto_check_slot: Arc<Mutex<Option<Option<Version>>>>`, `auto_ffmpeg: bool`; methods `start_auto_check()`, `on_auto_checked()`, `skip_version(product: Product, version: String)`, `save_config(before: &Config, item: Option<SettingItem>)`.

- [ ] **Step 1: Write the failing test** (in `config_file.rs` tests, next to `save_adds_tools_section_when_changed`, using its file setup style)

```rust
#[test]
fn save_records_a_skipped_version() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "# mine\nopen_key = \"Space\"\n").unwrap();
    let before = Config::default();
    let mut after = before.clone();
    after.updates.skip_app = Some("0.3.0".into());
    save(dir.path(), &before, &after).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.starts_with("# mine\nopen_key = \"Space\"\n"), "{text}");
    assert!(text.contains("[updates]\nskip_app = \"0.3.0\""), "{text}");
    assert!(!text.contains("auto_check"), "{text}");
    assert_eq!(config::parse(&text).unwrap().updates.skip_app.as_deref(), Some("0.3.0"));
}
```

- [ ] **Step 2: Run** `cargo test -p swisswheel config_file::` → expect PASS already (save is generic; Task 1 made the field). If it passes, record `Ruling: save test passes before Task 6 code — save_config is a pure move of existing logic; the test pins the file format the skip relies on` and continue. If it fails, fix `merge` first.

- [ ] **Step 3: Implement the wiring in `app.rs`:**
  - `const WM_APP_AUTO_CHECK: u32 = WM_APP + 37;`, `const TIMER_AUTO_CHECK: usize = 12;` with doc comments like the neighbors.
  - In `run`, after `STATE.set`, when `!previewing`: `SetTimer(Some(hwnd), TIMER_AUTO_CHECK, update_notice::DELAY.as_millis() as u32, None)`.
  - `on_timer` `TIMER_AUTO_CHECK`: `KillTimer`, then `start_auto_check()`.
  - `start_auto_check`: if `!self.config.updates.auto_check` → `log_info!` and return. Else `update::spawn_check(hwnd, WM_APP_AUTO_CHECK, slot)`; `self.auto_ffmpeg = true; self.start_essentials_check();` (it returns early when a check runs; that check's last report is then used).
  - `wnd_proc`: `WM_APP_AUTO_CHECK => with_state(AppState::on_auto_checked)`.
  - `on_auto_checked`: take the slot; `Some(Some(v))` and `app_due(v, update::current(), self.config.updates.skip_app.as_deref())` → `self.notify(update_notice::notice(Product::App, &v.to_string()))`; otherwise log (`"자동 확인: SwissWheel 알림 없음"` with the reason: up to date / skipped / failed).
  - `on_essentials_checked`: after applying the report, if `report.last && self.auto_ffmpeg` → `self.auto_ffmpeg = false;` then `ffmpeg_due(&self.essentials, self.config.updates.skip_ffmpeg.as_deref())` → `notify(notice(Product::Ffmpeg, &tag))`.
  - `on_notice` click arm: `Some(NoticeAction::Update(Product::App)) => self.start_update()`, `Some(NoticeAction::Update(Product::Ffmpeg)) => self.run_essential(EssentialAction::Update)`, `Some(NoticeAction::Skip(p, v)) => self.skip_version(p, v)`.
  - `skip_version`: clone config, set `skip_app`/`skip_ffmpeg`, `log_info!("{v} 버전을 건너뜁니다")`, `let before = std::mem::replace(&mut self.config, new); self.save_config(&before, None)`.
  - `save_config`: the tail of `apply_config` from `if !self.config_writable` on, moved verbatim; `self.save_failed = Some(item)` becomes `if let Some(item) = item { self.save_failed = Some(item) }`. `apply_config` calls `self.save_config(&before, Some(item))`.
  - Debug `notices` preview: add `update_notice::notice(Product::App, "0.3.0")` as a fourth card (`ago` 3).

- [ ] **Step 4: Run** `cargo test --workspace` → all pass; `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo fmt --check` clean.

- [ ] **Step 5: Commit** `feat(app): announce new versions at startup with skip`

### Task 7: Docs

**Files:**
- Modify: `README.md`, `docs/manual-test.md`, `docs/superpowers/specs/2026-10-22-updater-design.md`

- [ ] **Step 1:** README: after the updater sentence, add: a minute after start SwissWheel checks for a newer SwissWheel and (for its own copy) FFmpeg and shows a card with **업데이트** / **이 버전 건너뛰기**; a skipped version stays quiet until a newer one; **설정 → 일반 → 업데이트 자동 확인** turns the check off.
- [ ] **Step 2:** `docs/manual-test.md` after M83:
  - M84 가짜 릴리스(`SWISSWHEEL_UPDATE_URL`, 디버그) v9.9.9로 앱 실행, 1분 기다림 → 오른쪽 아래 "업데이트 · SwissWheel v9.9.9가 나왔습니다 [업데이트] [이 버전 건너뛰기]", 15초, 올리면 멈춤.
  - M85 M84 카드의 [업데이트] → 업데이트 확인 창(릴리스 노트).
  - M86 [이 버전 건너뛰기] → 카드 닫힘, `config.toml`에 `[updates] skip_app = "9.9.9"`. 다시 실행해 1분 → 카드 없음, 로그에 건너뜀. 업데이트 타일은 여전히 9.9.9를 보여 줌.
  - M87 가짜 릴리스를 v9.9.10으로 → 다시 카드가 뜸.
  - M88 카드 본문 클릭 → 카드만 닫힘; 다시 실행하면 또 뜸.
  - M89 설정 → 일반 → 업데이트 자동 확인 끔, 다시 실행해 1분 → 로그에 확인 안 함, 카드 없음.
  - M90 앱 전용 FFmpeg를 둔 채 `SWISSWHEEL_FFMPEG_URL` 태그를 9.9.9로 → "FFmpeg 9.9.9가 나왔습니다" 카드, [업데이트]는 다운로드 패널 업데이트와 같음, [건너뛰기]는 `skip_ffmpeg`. PATH의 FFmpeg만 있으면 카드 없음.
  - M91 1분 안에 전체 화면 게임 실행 → 카드는 게임을 끈 뒤 뜨고 버튼이 동작.
  - M92 인터넷을 끊고 실행, 1분 → 카드 없음, 로그에 실패.
- [ ] **Step 3:** updater design: one line under the header `- 자동 확인과 건너뛰기: `2026-10-23-update-notice-design.md``.
- [ ] **Step 4: Commit** `docs: update notice in README and manual tests`
