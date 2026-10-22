# UI Restyle (Liquid Glass) and Notice Cards Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restyle every surface (wheel, option panel, desktop menu, settings panel, progress card) as frosted liquid glass with Pretendard and a selectable accent, and replace Windows balloon notifications with the app's own glass notice cards.

**Architecture:** Pure rules stay in `swisswheel-core`: `appearance::Accent`, the new config keys, the 알림 tab, and `notices` (the card stack: order, timers, hover, morph, quiet hold, do-not-disturb). The app crate gets `fonts` (embedded Pretendard via DirectWrite's in-memory loader), `theme` (palette tokens + the two glass drawing routines every renderer shares), and `notice` (one no-activate window drawing the progress card and the notice cards, replacing `progress.rs`). `app.rs` routes every former `tray.notify`/`tray.warn` through one `notify` method that falls back to the balloon.

**Tech Stack:** Rust 2024, `windows` 0.61 (Direct2D, DirectWrite `IDWriteFactory5`, Windows.UI.Composition, `SHQueryUserNotificationState`), serde/toml/toml_edit.

**Spec:** `docs/superpowers/specs/2026-10-22-ui-restyle-design.md` (mockups in `docs/superpowers/specs/2026-10-22-ui-restyle/`)

## Global Constraints

- Base: the branch starts from `feat/ffmpeg-download-restart` (f66a017). Rebase onto `main` once that branch is merged, before Task 1.
- Layout numbers (wheel radii, `TILE`, `ROW`, panel widths, option panel geometry) do not change. Only corners, colors, fonts, and the hint line change.
- Corners (DIPs): wheel tile 9, option panel 16, option row 11, menu bar 22, menu tile 16, settings panel 20, selected row 11, notice card 18.
- Accent presets, order and colors exactly as spec 2.3; TOML names lowercase (`"blue"` … `"graphite"`); default `blue`. Failure red `#E5484D` regardless of accent.
- Color tokens exactly as the spec 2.1 table. Rim stops α .95 / .10 / .70 (light), ×0.4 in dark.
- Fonts: Pretendard v1.3.9 `public/static` Regular / SemiBold / Bold, unmodified. SHA-256: Regular `3ffbacde6ab8411f1d2db54bb9b1f0b3ee2a738932033722cf0388c06aed1c93`, SemiBold `c89bc43027dc7cde5726e96223376f8eec09302b2fc1f8147fd5b57cfc376118`, Bold `2e91915fab54df71cc9598ebf608b2bdb54c6fe3c066ac61dff0bc44fca71cc7`. Fallback family `Segoe UI Variable Text`.
- Notice timings: done 5 s, failed 8 s, info 5 s, warning 8 s, key result 3 s, key prompt = `CAPTURE_TIMEOUT` (5 s). Appear 200 ms, close 150 ms, progress→result morph 200 ms. At most 3 notice cards. Quiet re-check every 2 s.
- Notice card: width 300, padding 12/14, icon circle 30, title Bold 13.5, body Regular 12 (max 2 lines, ellipsis), action SemiBold 11.5, timer bar 2, gap 8, margin from work area 12.
- No new crates. Add `windows` features only if the compiler asks for one (expected: none; `Win32_UI_Shell` already covers `SHQueryUserNotificationState`).
- UI strings Korean, code comments English, short doc comments like the surrounding code. Every task ends with `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` green.

## Review Focus

1. A notice posted while another is being hovered must not reset or pause the new one's timer; only the hovered card pauses. (Task 6 `hover_pauses_only_that_card`.)
2. Two jobs finishing in the same `on_jobs_done` drain: only the one that leaves `active_jobs == 0` may take the progress card's slot; the other stacks above. (Task 6 `morph_only_for_last_job`; Task 8 passes `last_job` per report.)
3. Going quiet while a key prompt is up must not strand capture: the prompt is dropped from the held queue (it is only meaningful live) and the capture timer still ends capture. (Task 6 `quiet_drops_key_prompt`.)
4. A hand-edited `accent = "Blue"` or `accent = "teal"` must fall back to defaults with the existing config-error path, never panic or rewrite the file. (Task 1 `accent_invalid_is_parse_error`.)
5. Clicking where a card *was* during its 150 ms fade must not trigger its action twice or act on the card that slid into its place. (Task 6 `click_on_closing_card_is_ignored`.)

---

## Stage 1 — Foundation

### Task 1: `Accent` and the new config keys

**Files:**
- Create: `crates/swisswheel-core/src/appearance.rs`
- Modify: `crates/swisswheel-core/src/lib.rs` (pub mod), `crates/swisswheel-core/src/config.rs`
- Test: same files

**Interfaces:**
- Produces:
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)] #[serde(rename_all = "lowercase")] pub enum Accent { #[default] Blue, Purple, Pink, Red, Orange, Green, Graphite }`
  - `pub const ACCENTS: [Accent; 7]` (that order); `Accent::label(self) -> &'static str` (블루 … 그래파이트); `Accent::rgb(self) -> u32` (`0x0A84FF`, `0x7D5BF0`, `0xE0457F`, `0xE5484D`, `0xE0680B`, `0x1E9E50`, `0x5C5C64`)
  - `pub const FAILURE_RGB: u32 = 0xE5484D;`
  - `config::Appearance { pub accent: Accent }`, `config::Notifications { pub do_not_disturb: bool }` (both `#[serde(default)]`, defaults blue / false); `Config.appearance`, `Config.notifications`

- [ ] **Step 1: Write failing tests**

```rust
// appearance.rs
#[test]
fn accents_in_order_with_labels_and_colors() {
    let rows: Vec<_> = ACCENTS.iter().map(|a| (a.label(), a.rgb())).collect();
    assert_eq!(rows, [("블루", 0x0A84FF), ("퍼플", 0x7D5BF0), ("핑크", 0xE0457F), ("레드", 0xE5484D),
                      ("오렌지", 0xE0680B), ("그린", 0x1E9E50), ("그래파이트", 0x5C5C64)]);
    assert_eq!(Accent::default(), Accent::Blue);
}

// config.rs
#[test]
fn appearance_and_notifications() {
    let c = Config::default();
    assert_eq!((c.appearance.accent, c.notifications.do_not_disturb), (Accent::Blue, false));
    let c = parse("[appearance]\naccent = \"graphite\"\n[notifications]\ndo_not_disturb = true\n").unwrap();
    assert_eq!((c.appearance.accent, c.notifications.do_not_disturb), (Accent::Graphite, true));
    assert_eq!(parse(&toml::to_string_pretty(&c).unwrap()), Ok(c));
    assert!(default_toml().contains("accent = \"blue\""));
}

#[test]
fn accent_invalid_is_parse_error() {
    assert!(matches!(parse("[appearance]\naccent = \"Blue\""), Err(ConfigError::Parse(_))));
    assert!(matches!(parse("[appearance]\naccent = \"teal\""), Err(ConfigError::Parse(_))));
}
```

- [ ] **Step 2: Run** `cargo test -p swisswheel-core appearance config::` — expected: compile errors (no `appearance`, no `Accent`).
- [ ] **Step 3: Implement** `appearance.rs` and the two config structs per Interfaces. Put `appearance` and `notifications` last in `Config` so existing files keep their order when saved.
- [ ] **Step 4: Run** `cargo test --workspace` — expected: PASS (the app's `config_file` save tests keep comments since `toml_edit` only touches changed keys).
- [ ] **Step 5: Commit** `feat(core): accent presets and notification settings in config`

### Task 2: 알림 tab, 강조색 and 방해 금지 rows

**Files:**
- Modify: `crates/swisswheel-core/src/settings.rs`, `crates/swisswheel-core/src/menu_geometry.rs` (tabs follow `CATEGORIES.len()`), `crates/swisswheel/src/menubar/render.rs` and `mod.rs` only as far as the compiler needs for five tabs
- Test: `settings.rs`, `menu_geometry.rs`

**Interfaces:**
- Consumes: `Accent`, `ACCENTS`, `Config.appearance`, `Config.notifications` (Task 1)
- Produces: `Category::Notifications` (label `"알림"`), `CATEGORIES: [Category; 5]` = Keys, Tools, Convert, Notifications, General; `SettingItem::Accent` (label `"강조색"`, value = `Accent::label`), `SettingItem::DoNotDisturb` (label `"방해 금지"`, value 켬/끔); `MenuLayout.tabs: [Box2; CATEGORIES.len()]`; `MAX_ROWS` stays 3.

- [ ] **Step 1: Write failing tests** — update `labels_and_values_in_order` (new order: …, `("진행률 카드","켬")`, `("트레이 링","켬")`, `("방해 금지","끔")`, `("Windows 시작 시 실행","끔")`, `("강조색","블루")`) and `categories_in_order_with_their_items` (`["단축키","도구","변환","알림","일반"]`, `Notifications.items() == [ProgressWindow, TrayRing, DoNotDisturb]`, `General.items() == [StartWithWindows, Accent]`); add:

```rust
#[test]
fn accent_steps_through_presets_and_stops_at_ends() {
    let c = Config::default();
    assert_eq!(step(&c, SettingItem::Accent, Dir::Back), None);
    let c = step(&c, SettingItem::Accent, Dir::Forward).unwrap();
    assert_eq!(c.appearance.accent, Accent::Purple);
    let mut c = c; c.appearance.accent = Accent::Graphite;
    assert_eq!(step(&c, SettingItem::Accent, Dir::Forward), None);
}

#[test]
fn do_not_disturb_toggles() {
    let c = step(&Config::default(), SettingItem::DoNotDisturb, Dir::Forward).unwrap();
    assert!(c.notifications.do_not_disturb);
    assert_eq!(step(&c, SettingItem::DoNotDisturb, Dir::Forward), None);
}
```

  In `menu_geometry.rs`: `five_tabs_split_the_panel` — five tab boxes, equal width, touching, spanning `panel.left + PANEL_PAD .. panel.right - PANEL_PAD`.
- [ ] **Step 2: Run** `cargo test -p swisswheel-core settings menu_geometry` — expected: FAIL.
- [ ] **Step 3: Implement.** `Accent` steps by position in `ACCENTS`, like `CompressVideo` does with `STRENGTHS`. The tabs array length comes from `CATEGORIES.len()`.
- [ ] **Step 4: Run** `cargo test --workspace` and `SWISSWHEEL_PREVIEW=menu-settings cargo run -p swisswheel` — expected: tests PASS; preview shows five tabs and the 알림 tab's three rows (old look is fine here).
- [ ] **Step 5: Commit** `feat(core): notifications tab with accent and do-not-disturb rows`

### Task 3: Embedded Pretendard

**Files:**
- Create: `assets/fonts/Pretendard-Regular.otf`, `assets/fonts/Pretendard-SemiBold.otf`, `assets/fonts/Pretendard-Bold.otf`, `assets/fonts/OFL.txt`, `crates/swisswheel/src/fonts.rs`
- Modify: `crates/swisswheel/src/main.rs` (mod), `README.md` (License section)
- Test: `crates/swisswheel/src/fonts.rs`

**Interfaces:**
- Produces:
  - `pub const FAMILY: PCWSTR = w!("Pretendard");`, `pub const FALLBACK: PCWSTR = w!("Segoe UI Variable Text");`
  - `pub fn factory() -> IDWriteFactory5` — the shared factory (`DWRITE_FACTORY_TYPE_SHARED`), cached per thread
  - `pub fn collection() -> Option<IDWriteFontCollection1>` — built once per thread, `None` after logging if any step fails
  - `pub fn text_format(size: f32, weight: DWRITE_FONT_WEIGHT, alignment: DWRITE_TEXT_ALIGNMENT) -> Result<IDWriteTextFormat>` — Pretendard from `collection()` (fallback family + system collection when `None`), locale `ko-kr`, paragraph centered, no wrap (same as today's `font_format`)

- [ ] **Step 1: Fetch the fonts** from `https://github.com/orioncactus/pretendard/releases/download/v1.3.9/Pretendard-1.3.9.zip` (`public/static/` three files, `LICENSE.txt` → `OFL.txt`) into `assets/fonts/`; check `sha256sum` against Global Constraints.
- [ ] **Step 2: Write failing tests**

```rust
#[test]
fn pretendard_is_registered_with_real_weights_and_hangul() {
    let fonts = collection().expect("embedded fonts load");
    let family = find_family(&fonts, "Pretendard").expect("family present");
    for (weight, sim) in [(DWRITE_FONT_WEIGHT_REGULAR, false), (DWRITE_FONT_WEIGHT_SEMI_BOLD, false), (DWRITE_FONT_WEIGHT_BOLD, false)] {
        let font = first_matching(&family, weight);
        assert_eq!(font.weight(), weight);
        assert_eq!(font.simulations() != DWRITE_FONT_SIMULATIONS_NONE, sim);
        for ch in ['가', '똠', '뷁', 'A', '→'] { assert!(font.has_char(ch), "{ch}"); }
    }
}

#[test]
fn text_format_uses_pretendard() {
    let f = text_format(13.0, DWRITE_FONT_WEIGHT_BOLD, DWRITE_TEXT_ALIGNMENT_CENTER).unwrap();
    assert_eq!(family_name(&f), "Pretendard");
}
```
  (`find_family`, `first_matching`, `family_name` and the `weight/simulations/has_char` calls are small test-only helpers over `FindFamilyName`, `GetFirstMatchingFont`, `GetWeight`, `GetSimulations`, `HasCharacter`, `GetFontFamilyName`.)
- [ ] **Step 3: Run** `cargo test -p swisswheel fonts` — expected: FAIL (module missing).
- [ ] **Step 4: Implement** `fonts.rs`: `include_bytes!("../../../assets/fonts/…")` ×3 → `CreateInMemoryFontFileLoader` → `RegisterFontFileLoader` → `CreateInMemoryFontFileReference(factory, ptr, len, Some(&factory))` (the factory as owner object, so DirectWrite does not copy the 4.7 MB) → `CreateFontSetBuilder` → `AddFontFile` → `CreateFontSet` → `CreateFontCollectionFromFontSet`. Log `log_error!("글꼴을 불러올 수 없어 Segoe UI로 그립니다: {e}")` once on failure.
- [ ] **Step 5: Run** `cargo test --workspace` — expected: PASS. Add to README License: `Pretendard (assets/fonts) — SIL Open Font License 1.1, included unmodified.`
- [ ] **Step 6: Commit** `feat: embed Pretendard for all UI text`

### Task 4: Shared theme and glass drawing

**Files:**
- Create: `crates/swisswheel/src/theme.rs`
- Modify: `crates/swisswheel/src/main.rs` (mod); move `light_theme`, `taskbar_light_theme`, `transparency_enabled`, `rgb` out of `wheel/render.rs` into `theme.rs` and update their imports (`menubar/render.rs`, `progress.rs`, `tray.rs`). `Palette`/`palette()` stay in `wheel/render.rs` until Task 7 removes them.
- Test: `crates/swisswheel/src/theme.rs`

**Interfaces:**
- Consumes: `Accent::rgb`, `FAILURE_RGB` (Task 1)
- Produces:
  - `pub struct Theme { pub light: bool, pub glass: bool, pub alpha: f32, pub accent: D2D1_COLOR_F, pub failure: D2D1_COLOR_F, pub tint_top, tint_bottom, opaque, piece, piece_selected, glow, sheen, text, subtext, line, track: D2D1_COLOR_F, pub rim: [f32; 3] }` — every color already multiplied by `alpha`; `rim` = the three stop alphas (light `[0.95, 0.10, 0.70]`, dark ×0.4)
  - `Theme::new(light: bool, glass: bool, accent: Accent, alpha: f32) -> Theme`; `Theme::current(glass_supported: bool, accent: Accent, alpha: f32) -> Theme` (reads `light_theme()` and `transparency_enabled()`)
  - `pub enum Piece { Plain, Selected, Accent, Keycap }`
  - `pub fn glass_layer(ctx: &ID2D1DeviceContext, shape: &ID2D1Geometry, bounds: D2D_RECT_F, theme: &Theme, scale: f32) -> Result<()>`
  - `pub fn glass_piece(ctx: &ID2D1DeviceContext, shape: &ID2D1Geometry, bounds: D2D_RECT_F, theme: &Theme, piece: Piece, scale: f32) -> Result<()>`
  - `pub fn outer_shadow(ctx: &ID2D1DeviceContext, shape: &ID2D1Geometry, theme: &Theme, scale: f32) -> Result<()>` (for menu and cards; fits in a 10 DIP margin)

- [ ] **Step 1: Write failing tests** (pure color values, no device):

```rust
#[test]
fn light_and_dark_tokens() {
    let l = Theme::new(true, true, Accent::Blue, 1.0);
    assert_eq!(hex(l.tint_top), (0xFFFFFF, 0.32)); assert_eq!(hex(l.tint_bottom), (0xFFFFFF, 0.18));
    assert_eq!(hex(l.text), (0x1D1D1F, 1.0)); assert_eq!(hex(l.accent), (0x0A84FF, 0.85));
    assert_eq!(l.rim, [0.95, 0.10, 0.70]);
    let d = Theme::new(false, true, Accent::Green, 1.0);
    assert_eq!(hex(d.tint_top), (0x2A2A2E, 0.55)); assert_eq!(hex(d.text), (0xF5F5F7, 1.0));
    assert_eq!(hex(d.accent), (0x1E9E50, 0.85));
    assert_eq!(d.rim.map(|r| (r * 100.0).round()), [38.0, 4.0, 28.0]);
}

#[test]
fn opaque_without_glass_and_alpha_fades_everything() {
    let t = Theme::new(true, false, Accent::Red, 1.0);
    assert_eq!(hex(t.opaque), (0xF2F2F5, 1.0)); assert_eq!(hex(t.accent).1, 1.0);
    let half = Theme::new(true, true, Accent::Blue, 0.5);
    assert_eq!(hex(half.text).1, 0.5); assert_eq!(hex(half.failure), (0xE5484D, 0.5));
}
```
  (`hex` is a test helper: color → `(u32 rgb, alpha rounded to 2 places)`.)
- [ ] **Step 2: Run** `cargo test -p swisswheel theme` — expected: FAIL.
- [ ] **Step 3: Implement** the tokens from the spec 2.1 table. Drawing, in this order, with no D2D effects (they cost too much per hover frame):
  - `glass_layer`: fill `tint_top→tint_bottom` vertical gradient (or `opaque` when `!glass`); inner glow = `PushLayer` with `shape` as geometric mask, then `DrawGeometry(shape)` with `glow` at widths 7, 4, 2 DIP and α ⅓ each; sheen = `sheen→transparent` over the top half of `bounds`; rim = 1.3 DIP stroke with a linear gradient from `bounds` top-left to bottom-right, stops 0 / .45 / 1 at `rim[0..3]` white.
  - `glass_piece`: short shadow first (`shape` filled at +3 DIP y with black α .08 light / .30 dark, then stroked 6 DIP at α .04 / .15); then fill `piece` / `piece_selected` / `accent` / keycap fill (light `#FFFFFF` .80, dark `#FFFFFF` .12); then glow, sheen, rim as above (accent pieces glow with the accent mixed 60 % toward white, plus a 4 DIP stroke of the accent at α .45 under the fill as its colored shadow).
  - `outer_shadow`: `shape` stroked at widths 16, 10, 4 DIP with black α .04 each, offset +4 DIP y.
- [ ] **Step 4: Run** `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` — expected: PASS (no renderer uses the new drawing yet).
- [ ] **Step 5: Commit** `feat(app): shared glass theme with accent tokens`

## Stage 2 — New look

### Task 5: Wheel and option panel

**Files:**
- Modify: `crates/swisswheel/src/wheel/render.rs`, `crates/swisswheel/src/wheel/window.rs` (accent passed in), `crates/swisswheel/src/wheel/mod.rs` (`WheelView` gets `accent: Accent`), `crates/swisswheel/src/app.rs` (fill `accent` from `self.config.appearance.accent`)

**Interfaces:**
- Consumes: `Theme`, `glass_layer`, `glass_piece`, `Piece` (Task 4); `fonts::text_format` (Task 3)
- Produces: `WheelView.accent: Accent`. `Renderer` no longer creates its own `IDWriteFactory`; text formats come from `fonts::text_format`, and the icon font format is made on `fonts::factory()` with the system collection (`icon_font` unchanged).

- [ ] **Step 1: Restyle the disc and tiles.** Disc = `glass_layer` on the circle; each tile = `glass_piece(Plain)`, hovered tile `Piece::Accent` with white text; `TILE_CORNER` 9; hub = `glass_piece(Plain)` at `(CANCEL_RADIUS - HUB_INSET)`; hub texts Bold 14 / SemiBold 10.5; labels SemiBold as today. The window region stays the circle: no `outer_shadow` here.
- [ ] **Step 2: Restyle the option panel.** `PANEL_CORNER` 16, `ROW_CORNER` 11; panel = `glass_layer`; hovered row `Piece::Accent`; the default row gets a "기본" badge: a `Piece::Plain` pill sized to the text with `subtext` SemiBold 10.
- [ ] **Step 3: Check by eye** with `SWISSWHEEL_PREVIEW=panel cargo run -p swisswheel` and `=panel-left`, in light and dark, transparency on and off (Settings › Personalization › Colors). Expected: matches mockup `1-glass-technique.jpg` panel A and `2-accent.jpg`; text readable over a busy wallpaper; hover grows 6 DIP in 120 ms as before.
- [ ] **Step 4: Run** `cargo test --workspace` + clippy — expected: PASS.
- [ ] **Step 5: Commit** `feat(wheel): liquid glass wheel and option panel`

### Task 6: Notice rules (`notices`, pure)

> Core-only and independent of Tasks 5 and 7; numbered here so it is done before Stage 3 needs it.

**Files:**
- Create: `crates/swisswheel-core/src/notices.rs`; Modify: `crates/swisswheel-core/src/lib.rs`
- Test: `crates/swisswheel-core/src/notices.rs`

**Interfaces:**
- Produces:
  - `pub enum NoticeKind { Done, Failed, Info, Warning, KeyPrompt, KeyResult }` with `fn lifetime(self) -> Duration` (5/8/5/8/5/3 s)
  - `pub enum NoticeAction { OpenFolder(PathBuf), CancelCapture }`
  - `pub struct Notice { pub kind: NoticeKind, pub title: String, pub body: String, pub action: Option<NoticeAction> }`
  - `pub type NoticeId = u64;`
  - `pub struct CardView { pub id: NoticeId, pub kind: NoticeKind, pub title: String, pub body: String, pub action_label: Option<&'static str>, pub remaining: f32, pub appear: f32, pub closing: Option<f32>, pub from_progress: Option<f32> }` — `remaining` 1→0 for the timer bar; `appear` 0→1 over 200 ms; `closing` 0→1 over 150 ms while fading; `from_progress` 0→1 over 200 ms when this card took the progress card's slot. `action_label` is `"클릭하면 폴더 열기"` for `OpenFolder`, else `None`.
  - `pub struct Notices` with:
    - `new() -> Notices`, `set_do_not_disturb(&mut self, on: bool)`
    - `post(&mut self, notice: Notice, now: Instant) -> NoticeId`
    - `post_job(&mut self, notice: Notice, last_job: bool, progress_shown: bool, now: Instant) -> Option<NoticeId>` — `None` when do-not-disturb drops it
    - `dismiss(&mut self, id: NoticeId, now: Instant)` — starts the fade
    - `hover(&mut self, id: Option<NoticeId>, now: Instant)`, `click(&mut self, id: NoticeId, now: Instant) -> Option<NoticeAction>`
    - `set_quiet(&mut self, quiet: bool, now: Instant)`, `quiet(&self) -> bool`
    - `tick(&mut self, now: Instant)` — expires and drops fully faded cards
    - `cards(&self, now: Instant) -> Vec<CardView>` — nearest the progress card first; empty while quiet
    - `next_wake(&self, now: Instant) -> Option<Instant>` — next expiry, or `now + 16 ms` while anything animates

- [ ] **Step 1: Write failing tests** (all with `let t0 = Instant::now();` and `t0 + ms(..)`):
  - `newest_nearest_progress_and_max_three`: post A,B,C,D → `cards` ids `[D,C,B,A]` with A `closing.is_some()`; after `tick(t0+ms(151))` → `[D,C,B]`.
  - `lifetimes_per_kind`: Done gone at 5 s + 150 ms, still there at 4.9 s; Failed at 8 s; KeyResult at 3 s; `remaining` of Done at 2.5 s ≈ 0.5.
  - `hover_pauses_only_that_card`: post A at t0, B at t0+1 s, hover A at t0+1 s for 10 s, unhover → A has 4 s left, B expired at t0+6 s.
  - `click_runs_action_and_closes`: Done with `OpenFolder(p)` → `click` returns it once, card `closing`; `click_on_closing_card_is_ignored` returns `None` the second time and for an unknown id.
  - `key_prompt_click_cancels_capture`: `click` on KeyPrompt → `Some(CancelCapture)`.
  - `morph_only_for_last_job`: `post_job(_, last_job=false, progress_shown=true)` → `from_progress == None`; `post_job(_, true, true)` → `from_progress == Some(0.0)` at post time, `Some(1.0)` at +200 ms; `post_job(_, true, false)` → `None`.
  - `dnd_drops_job_cards_only`: with DND on, `post_job` returns `None` for Done and Failed; `post` of Info, Warning, KeyPrompt, KeyResult still shows.
  - `quiet_holds_and_releases_latest_three`: quiet on, post 5 Done at 1 s intervals, `cards` empty; at t0+20 s quiet off → the last 3, each with full `remaining` (held time not counted).
  - `quiet_drops_key_prompt`: KeyPrompt visible, quiet on then off → not shown again.
  - `next_wake_sleeps_until_expiry`: one Done idle after its appear animation → `next_wake == post time + 5 s`.
- [ ] **Step 2: Run** `cargo test -p swisswheel-core notices` — expected: FAIL.
- [ ] **Step 3: Implement.** Each card keeps `posted`, `paused_at: Option<Instant>`, `paused_total: Duration`, `closing_since: Option<Instant>`, `morph_since: Option<Instant>`. Quiet moves visible non-prompt cards into a held list (keeping their `Notice`, not their timing); releasing re-posts the newest three at `now`.
- [ ] **Step 4: Run** `cargo test --workspace` — expected: PASS.
- [ ] **Step 5: Commit** `feat(core): notice card rules`

### Task 7: Desktop menu and settings panel

**Files:**
- Modify: `crates/swisswheel/src/menubar/render.rs`, `crates/swisswheel/src/menubar/mod.rs` (`MenuView.accent`, swatch for the 강조색 row), `crates/swisswheel/src/menubar/window.rs` (10 DIP shadow margin around bar and panel), `crates/swisswheel/src/surface.rs` (`text_format` → `fonts::text_format`; keep `icon_font`), `crates/swisswheel/src/app.rs` (fill `accent`)
- Remove: `Palette` and `palette()` from `wheel/render.rs` once nothing uses them (the progress card in `progress.rs` switches to `Theme` minimally: card = `glass_layer`, bar = accent, until Task 9 replaces it)

**Interfaces:**
- Consumes: Task 3, Task 4
- Produces: `MenuView.accent: Accent`; `Row` gains `swatch: Option<D2D1_COLOR_F>` (set for `SettingItem::Accent`)

- [ ] **Step 1: Bar and tiles.** `BAR_CORNER` 22, `TILE_CORNER` 16; bar = `outer_shadow` + `glass_layer`; tiles `Piece::Plain`, hovered tile `Piece::Selected`, selected tile `Piece::Accent`.
- [ ] **Step 2: Panel.** `PANEL_CORNER` 20; tabs = one `line`-colored rounded groove (corner 12) across the tab line, selected tab a pill (light `#FFFFFF` .85, dark `#FFFFFF` .18, corner 9, inset 3) with `text`, others `subtext`; drop the underline (`TAB_UNDERLINE_INSET`). Selected row `Piece::Selected`, corner 11; ◀ ▶ glyphs in `subtext` at 10 px; value Bold; keycaps `Piece::Keycap` corner 7; the 강조색 value draws a 10 DIP dot of the accent before its label. The hint line still draws for now (Task 10 removes it).
- [ ] **Step 3: Window margin.** The menu window grows by 10 DIP on each side for `outer_shadow`; `menu_geometry` boxes are offset by that margin in `window.rs` only (core layout unchanged), and hit testing subtracts it.
- [ ] **Step 4: Check by eye** with `SWISSWHEEL_PREVIEW=menu`, `menu-settings`, `menu-tools`, `menu-capture`, light/dark, transparency on/off, each accent. Expected: matches `3-menu-settings.jpg` (five tiles instead of three).
- [ ] **Step 5: Run** `cargo test --workspace` + clippy — expected: PASS; `rg "palette\(" crates` finds nothing.
- [ ] **Step 6: Commit** `feat(menu): liquid glass desktop menu and settings panel`

## Stage 3 — Notice cards

### Task 8: Notice window

**Files:**
- Create: `crates/swisswheel/src/notice.rs`; Delete: `crates/swisswheel/src/progress.rs` (move `card_height`, row layout, `MAX_ROWS = 5`, "외 N개" into `notice.rs`)
- Modify: `crates/swisswheel/src/main.rs`, `crates/swisswheel/src/app.rs` (construct `NoticeWindow` instead of `ProgressWindow`; `update_busy` calls `render`)
- Test: `crates/swisswheel/src/notice.rs` (layout math only)

**Interfaces:**
- Consumes: `CardView`, `NoticeKind`, `NoticeId` (Task 6); `Theme`, `glass_layer`, `glass_piece`, `outer_shadow` (Task 4); `fonts::text_format`; `ProgressRow`
- Produces:
  - `pub const WM_APP_NOTICE: u32 = WM_APP + 33;` — posted to the owner: `wparam` 0 = hover changed (`lparam` = id or −1), 1 = click (`lparam` = id)
  - `pub struct NoticeWindow`; `NoticeWindow::new(owner: HWND) -> Result<NoticeWindow>`
  - `pub fn render(&mut self, progress: &[ProgressRow], cards: &[CardView], accent: Accent) -> Result<()>` — hides the window when both are empty; the progress card's title stays today's `format!("변환 중 · {}개", progress.len())`
  - `pub fn quiet_now() -> bool` — `SHQueryUserNotificationState` is `QUNS_BUSY`, `QUNS_RUNNING_D3D_FULL_SCREEN`, or `QUNS_PRESENTATION_MODE`; `false` on error
  - layout helper `fn stack(progress_rows: usize, more: bool, cards: &[CardView], body_lines: impl Fn(&CardView) -> usize) -> Vec<(Slot, f32 /*top, DIPs from bottom*/ , f32 /*height*/)>` where `enum Slot { Progress, Card(NoticeId) }`

- [ ] **Step 1: Write failing layout tests**: `progress_at_bottom_cards_above` (progress height `card_height(2,false)`, card 1 starts `8` above it, newest nearest); `appear_slides_in_and_close_fades` (x offset `(1 - appear) * 24 DIP`, opacity follows `closing`); `morph_interpolates_height` (at `from_progress = 0.5` the card's height is halfway between the progress card's and its own).
- [ ] **Step 2: Run** `cargo test -p swisswheel notice` — expected: FAIL.
- [ ] **Step 3: Implement the window**: `WS_POPUP`, ex-style `WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_NOREDIRECTIONBITMAP`, owned by the app window; `WM_MOUSEACTIVATE` → `MA_NOACTIVATE`; `SetWindowRgn` = union of card and progress rounded rects grown by the 10 DIP shadow margin; `TrackMouseEvent(TME_LEAVE)` for hover-out; mouse moves/clicks hit-test the slots and post `WM_APP_NOTICE`. Placement and DPI as `progress.rs` did (primary monitor work area, bottom-right, margin 12). Card drawing per spec 2.6: icon circle (Done/Info accent `\u{E73E}`/`\u{E946}`, Failed/Warning failure color `\u{E7BA}`, KeyPrompt `Piece::Keycap` with `\u{E765}`, KeyResult accent `\u{E8AB}`), title, body (2 lines, `DWRITE_TRIMMING_GRANULARITY_CHARACTER` with ellipsis sign), action line in accent, timer bar (2 DIP, accent α .7; failure color for Failed/Warning). Progress card per spec 2.4 with a 5 DIP rounded accent bar on a `track` groove; title `"변환 중 · N개"`.
- [ ] **Step 4: Preview.** Add `"notices"` to `AppState::preview`: progress card with two rows plus Done, Failed, KeyPrompt cards posted 0/1/2 s ago. Check by eye with `SWISSWHEEL_PREVIEW=notices`: matches `4-notice-cards.jpg`; clicking into Notepad then hovering/clicking a card leaves Notepad focused; clicks beside the cards reach the desktop.
- [ ] **Step 5: Run** `cargo test --workspace` + clippy — expected: PASS.
- [ ] **Step 6: Commit** `feat(app): notice window with progress and notice cards`

### Task 9: Route notifications through the cards

**Files:**
- Modify: `crates/swisswheel/src/app.rs`, `crates/swisswheel/src/tray.rs`, `crates/swisswheel-core/src/summary.rs`
- Test: `crates/swisswheel-core/src/summary.rs`

**Interfaces:**
- Consumes: `Notices`, `Notice`, `NoticeKind`, `NoticeAction` (Task 6); `NoticeWindow`, `WM_APP_NOTICE`, `quiet_now` (Task 8)
- Produces:
  - `AppState::notify(&mut self, notice: Notice)` and `AppState::notify_job(&mut self, notice: Notice, last_job: bool)` — the only ways to show a notification; with no `NoticeWindow` they call `tray.notify` (Done/Info/KeyResult) or `tray.warn` (Failed/Warning) and remember an `OpenFolder` path for `NIN_BALLOONUSERCLICK`
  - `summary::idle_tooltip(last_result: Option<&str>) -> String` — `"SwissWheel"` or `"SwissWheel · 마지막 작업: {title}"`, cut to 127 UTF-16 units ending in `…`
  - timers `TIMER_NOTICE = 10` (one-shot to `next_wake`), `TIMER_QUIET = 11` (2 s, only while cards are visible or held)

- [ ] **Step 1: Write failing test** `idle_tooltip_shows_last_result` (None → `"SwissWheel"`; `Some("3개 파일을 PNG로 변환했습니다")` → `"SwissWheel · 마지막 작업: 3개 파일을 PNG로 변환했습니다"`; a 200-char title → `encode_utf16().count() == 127` and ends with `'…'`).
- [ ] **Step 2: Run** `cargo test -p swisswheel-core summary` — expected: FAIL. **Step 3:** implement; run again — PASS.
- [ ] **Step 4: Rewire `app.rs`.**
  - Replace all 22 `tray.notify`/`tray.warn` calls with `notify`: `notify` → `NoticeKind::Info`, `warn` → `NoticeKind::Warning`; titles and bodies unchanged. Three of them (updated, restarted, config file error) run in `run` before `AppState` exists: collect them in a `Vec<Notice>` there and `notify` each right after the state is built.
  - `on_jobs_done`: per report, `last_job = (self.active_jobs == 0 after decrement)`; kind Done or Failed; action `OpenFolder(first output)` when there is one; `notify_job`. Keep the title for `idle_tooltip`; drop `last_output` except for the balloon fallback.
  - `notices.set_do_not_disturb(config.notifications.do_not_disturb)` at start and in `apply_config`.
  - `WM_APP_NOTICE` → `hover`/`click`; `OpenFolder(p)` runs the existing `explorer.exe /select,` spawn (log on failure); `CancelCapture` runs the same path as `MenuAction::CancelCapture`.
  - After any change, `notice_window.render(&rows, &notices.cards(now), accent)` and re-arm `TIMER_NOTICE`; `TIMER_QUIET` calls `notices.set_quiet(quiet_now(), now)`. If `render` fails twice in a row, log it, drop the `NoticeWindow`, and fall back to balloons for the rest of the run.
  - `WM_DISPLAYCHANGE`, `WM_DPICHANGED`, and a work-area `WM_SETTINGCHANGE` re-place and re-render the notice window.
  - Idle tooltip: `update_busy` uses `idle_tooltip(self.last_result.as_deref())` when not busy.
  - `SWISSWHEEL_FORCE_BALLOON=1` (any build) skips creating `NoticeWindow`; so does a creation error (logged).
  - `WM_SETTINGCHANGE` and an accent change re-render the notice window.
- [ ] **Step 5: Check by hand**: convert a file → progress card turns into the result card; click opens the folder with the file selected; two drops at once → two cards; full-screen video (F11 in a browser) during a conversion → no card until you leave full screen; DND on → no result card, tooltip shows it; `SWISSWHEEL_FORCE_BALLOON=1` → balloons as before and clicking one opens the folder.
- [ ] **Step 6: Run** `cargo test --workspace` + clippy — expected: PASS; `rg "tray\.(notify|warn)" crates/swisswheel/src/app.rs` finds only the two fallback calls inside `notify`/`notify_job`.
- [ ] **Step 7: Commit** `feat(app): show notifications as glass cards`

### Task 10: Drop the hint line; key prompts as cards

**Files:**
- Modify: `crates/swisswheel-core/src/menu.rs` (remove `hint()` and its tests), `crates/swisswheel-core/src/menu_geometry.rs` (remove `hint`; panel = notice line + tabs + `MAX_ROWS` rows), `crates/swisswheel/src/menubar/{mod,render}.rs` (remove `MenuView.hint` and its drawing), `crates/swisswheel/src/app.rs` (remove `menu_hint`)
- Test: `menu.rs`, `menu_geometry.rs`

**Interfaces:**
- Consumes: `AppState::notify`, `Notices::dismiss` (Tasks 6, 9)
- Produces: `AppState.capture_notice: Option<NoticeId>`

- [ ] **Step 1: Update tests**: `panel_has_tabs_three_rows` — `panel.bottom - panel.top == s(2*PANEL_PAD + 4*ROW)` without notice, `5*ROW` with; `hit_tabs_keycaps_and_panel` — a point 2 DIP above `panel.bottom` is `MenuHit::Panel`. Delete the `hint()` assertions in `menu.rs` tests.
- [ ] **Step 2: Run** `cargo test -p swisswheel-core menu` — expected: FAIL (height still includes the hint line). **Step 3:** remove the hint line from layout and `MenuSession`; run — PASS.
- [ ] **Step 4: Rewire capture in `app.rs`**: `arm_capture` posts `Notice { kind: KeyPrompt, title: "바꿀 키를 누르세요", body: format!("{} · 다른 곳을 클릭하면 취소", item.label()), action: Some(CancelCapture) }` and keeps the id (re-arm after a rejected key keeps the same card); `disarm_capture` dismisses it; a rejected key posts KeyResult `"이 키는 쓸 수 없습니다"`; a swap posts KeyResult with title `"두 단축키를 맞바꿨습니다"` and body `"{other label}는 이제 {key label}입니다"` (today's wording).
- [ ] **Step 5: Check by hand** with `SWISSWHEEL_PREVIEW=menu-capture` and the real panel: Enter on 휠 열기 키 → card with a 5 s bar; press F2 → card gone, keycap shows F2; bind the key another row has → swap card; press A → "쓸 수 없습니다" card and the prompt card stays; click the prompt card → capture ends.
- [ ] **Step 6: Run** `cargo test --workspace` + clippy — PASS. **Step 7: Commit** `feat(menu): drop the hint line, show key prompts as cards`

### Task 11: Docs

**Files:**
- Modify: `README.md`, `docs/manual-test.md`

- [ ] **Step 1: README**: Usage step 5 → "A card above the tray reports the result. Click it to open the folder with the result selected." Settings paragraph: tabs **단축키 · 도구 · 변환 · 알림 · 일반**, 강조색 under 일반, 진행률 카드 · 트레이 링 · 방해 금지 under 알림 (방해 금지 hides result cards; menu results and key prompts still show). Remove the hint line sentence about waiting for a key ("Waiting for a key ends…") → "a card shows while it waits; it ends after 5 seconds, a click elsewhere, or a click on the card." Progress bullet: the card turns into the result card. Mention `SWISSWHEEL_FORCE_BALLOON=1` only in `manual-test.md`.
- [ ] **Step 2: manual-test.md**: add spec 5's manual list verbatim as a "UI 리디자인" section with the previews to use (`panel`, `panel-left`, `menu*`, `notices`).
- [ ] **Step 3: Run** `cargo test --workspace`; release build `cargo build --release -p swisswheel` — expected: PASS; exe ≈ 14 MB. Start the release build, idle 1 minute, Task Manager memory ≤ 15 MB.
- [ ] **Step 4: Commit** `docs: liquid glass UI, notice cards, accent and do-not-disturb`
