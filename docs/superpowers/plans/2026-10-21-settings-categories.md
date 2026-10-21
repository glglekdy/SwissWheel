# Settings Categories and Key Capture Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split the desktop menu's settings panel into tabs (단축키 · 도구 · 변환 · 일반), bind shortcut keys by pressing them, and add two tool-default settings that the wheel's option panel follows.

**Architecture:** Pure logic stays in `swisswheel-core`: config (`Key`, `Tools`), `settings` (categories, `step`, `bind`), `menu` (tab/row/capture state machine), `menu_geometry` (tabs, keycaps, hint line), `drag` (one-shot key capture in the hook's tracker). The app crate wires the capture through `hook`, owns the 5-second timer and result hint, and draws the new panel.

**Tech Stack:** Rust 2024, `windows` 0.61, Direct2D/DirectWrite via `menubar::render`, serde/toml/toml_edit.

**Spec:** `docs/superpowers/specs/2026-10-21-settings-categories-design.md`

## Global Constraints

- Tabs, in order: 단축키 · 도구 · 변환 · 일반. Items per tab exactly as spec 2.1.
- Bindable keys and their file names/labels exactly as spec 2.3 (25 keys). Existing names `"Space"`, `"Tab"`, `"Escape"`, `"CapsLock"`, `"Backquote"` stay valid.
- `[tools]`: `shrink_percent` ∈ {25, 50, 75}, default 50; `compress_video` ∈ `"Light"`/`"Normal"`/`"Strong"`, default `"Normal"`. Labels 약하게 / 보통 / 강하게.
- Capture timeout 5 s. Panel height fixed: tabs line + 3 rows + hint line (+ notice line when the config can't be saved).
- Hint texts (exact): `"←/→ 카테고리 · ↓ 항목"`, `"Enter로 키 변경"`, `"←/→로 값 변경"`, `"바꿀 키를 누르세요 · 다른 곳 클릭 시 취소"`, `"이 키는 쓸 수 없습니다"`, swap: `"{other label}는 이제 {key label}입니다"` (e.g. `"휠 열기 키는 이제 Tab입니다"`; this wording avoids 로/으로 after Latin key names — update spec 2.3 to match in Task 8).
- Between Task 4 and Task 7 the `swisswheel` app crate does not compile (core APIs change first); verify those tasks with `cargo test -p swisswheel-core`. Every task from Task 7 on runs the whole workspace.
- Comments and naming follow the surrounding code (short doc comments, Korean UI strings, English code comments).

## Review Focus

1. A key held since before capture started (its auto-repeat) must not be taken as the captured key — the next fresh press is. (Task 6 test `capture_ignores_repeats_of_held_keys`.)
2. A rejected key (letter, arrow) during capture must not reach the desktop on press or release, and capture must stay armed afterwards. (Task 6 `capture_swallows_any_key_once_and_its_release`; Task 7 rearm path in app.)
3. Binding the key a row already has must change nothing and write nothing. (Task 2 `bind_same_key_is_a_no_op`; Task 7 skips `apply_config` when unchanged.)
4. A hand-written `config.toml` with `shrink_percent = 40` or an unknown key name must fall back to defaults with a warning, not crash or rewrite the file. (Task 1 `tools_validation` and existing `broken_file_is_kept_and_defaults_used`.)
5. Clicking the empty third row on the 도구 tab, or the hint line, must do nothing (not select a nonexistent row). (Task 5 `tools_tab_has_no_third_row`.)

---

### Task 1: Config — more keys and `[tools]`

**Files:**
- Modify: `crates/swisswheel-core/src/config.rs`
- Modify: `crates/swisswheel-core/src/options.rs` (derive serde on `Strength`)
- Modify: `crates/swisswheel-core/src/settings.rs` (`key_label` for the new keys)
- Test: `crates/swisswheel-core/src/config.rs`, `crates/swisswheel/src/config_file.rs`

**Interfaces:**
- Produces: `Key` variants `Space, Tab, Escape, CapsLock, Backquote, F1..F12, Insert, Delete, Home, End, PageUp, PageDown, Pause, ScrollLock`; `Key::ALL: [Key; 25]` (that order); `Key::vk(self) -> u16`; `Key::from_vk(u16) -> Option<Key>`; `pub struct Tools { pub shrink_percent: u8, pub compress_video: Strength }` (`#[serde(default)]`, `Default` = 50 / `Normal`); `Config.tools: Tools`; `settings::key_label(Key) -> &'static str` (labels per spec 2.3: `PgUp`, `PgDn`, `Esc`, `` ` ``, others = name).
- VK codes: F1–F12 = 0x70–0x7B, Insert 0x2D, Delete 0x2E, Home 0x24, End 0x23, PageUp 0x21, PageDown 0x22, Pause 0x13, ScrollLock 0x91.

- [ ] **Step 1: Write failing tests** in `config.rs` tests:

```rust
#[test]
fn new_keys_parse_and_round_trip() {
    let c = parse("open_key = \"F5\"\ncycle_key = \"PageUp\"\ncancel_key = \"Pause\"\n").unwrap();
    assert_eq!((c.open_key, c.cycle_key, c.cancel_key), (Key::F5, Key::PageUp, Key::Pause));
    assert_eq!(parse(&toml::to_string_pretty(&c).unwrap()), Ok(c));
}

#[test]
fn vk_codes_round_trip() {
    assert_eq!(Key::ALL.len(), 25);
    for k in Key::ALL {
        assert_eq!(Key::from_vk(k.vk()), Some(k));
    }
    assert_eq!(Key::F1.vk(), 0x70);
    assert_eq!(Key::F12.vk(), 0x7B);
    assert_eq!(Key::ScrollLock.vk(), 0x91);
    for vk in [0x41, 0x30, 0x25, 0x0D, 0xA0, 0x5B] {
        assert_eq!(Key::from_vk(vk), None); // A, 0, ←, Enter, Shift, Win
    }
}

#[test]
fn tools_defaults_and_validation() {
    let c = Config::default();
    assert_eq!((c.tools.shrink_percent, c.tools.compress_video), (50, Strength::Normal));
    let c = parse("[tools]\nshrink_percent = 25\ncompress_video = \"Strong\"\n").unwrap();
    assert_eq!((c.tools.shrink_percent, c.tools.compress_video), (25, Strength::Strong));
    assert!(matches!(parse("[tools]\nshrink_percent = 40"), Err(ConfigError::Invalid(_))));
    assert!(matches!(parse("[tools]\ncompress_video = \"Max\""), Err(ConfigError::Parse(_))));
}
```

Also change `invalid_configs`: `open_key = "F5"` now parses, so replace that case with `open_key = "F13"` → `Err(ConfigError::Parse(_))`. In `config_file.rs` tests add:

```rust
#[test]
fn save_adds_tools_section_when_changed() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.toml"), "open_key = \"Space\"\n").unwrap();
    let before = Config::default();
    let mut after = before.clone();
    after.tools.shrink_percent = 75;
    save(dir.path(), &before, &after).unwrap();
    let text = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
    assert!(text.contains("[tools]") && text.contains("shrink_percent = 75"), "{text}");
    assert!(!text.contains("compress_video"), "{text}");
}
```

- [ ] **Step 2: Run** `cargo test -p swisswheel-core config` — expected: compile errors (`Key::F5`, `Key::ALL`, `tools` missing).

- [ ] **Step 3: Implement.** Add the variants, `ALL`, `from_vk` (linear search over `ALL`), `Tools`, `Config.tools`; derive `Serialize, Deserialize` on `options::Strength`; validate `shrink_percent` with message `"tools.shrink_percent는 25, 50, 75 중 하나여야 합니다 (현재 {n})"`. Extend `settings::key_label`. Update the `ProgressDisplay` doc comment ("A settings window will edit these later") to say the settings panel edits them.

- [ ] **Step 4: Run** `cargo test --workspace` — expected: all pass.

- [ ] **Step 5: Commit** `feat(core): bindable F-keys and navigation keys, [tools] defaults`

---

### Task 2: Settings — categories, tool items, `bind`

**Files:**
- Modify: `crates/swisswheel-core/src/settings.rs`

**Interfaces:**
- Consumes: Task 1 `Key`, `Tools`, `Strength`.
- Produces:
  - `pub enum Category { Keys, Tools, Convert, General }`, `pub const CATEGORIES: [Category; 4]` (that order), `Category::label(self) -> &'static str` (단축키 / 도구 / 변환 / 일반), `Category::items(self) -> &'static [SettingItem]`.
  - `pub const MAX_ROWS: usize = 3;`
  - `SettingItem::ShrinkPercent` (label `"축소 비율"`, value `"50%"`), `SettingItem::CompressVideo` (label `"영상 압축"`, value `"보통"`); `SettingItem::is_key(self) -> bool`.
  - `step(c, item, dir)` returns `None` for the three key items; ShrinkPercent steps over `[25, 50, 75]`, CompressVideo over `[Light, Normal, Strong]` (compare by position).
  - `pub struct Bound { pub config: Config, pub swapped: Option<(SettingItem, Key)> }`, `pub fn bind(c: &Config, item: SettingItem, key: Key) -> Bound`. Non-key `item`: config unchanged, `swapped: None`.
  - Keep `ITEMS` for now (still used by `menu.rs` and the app until Task 4/7); Task 7 deletes it.

- [ ] **Step 1: Write failing tests** (replace `keys_skip_values_used_by_the_other_two`, which no longer applies):

```rust
#[test]
fn categories_in_order_with_their_items() {
    use SettingItem::*;
    let labels: Vec<_> = CATEGORIES.iter().map(|c| c.label()).collect();
    assert_eq!(labels, ["단축키", "도구", "변환", "일반"]);
    assert_eq!(Category::Keys.items(), [OpenKey, CycleKey, CancelKey]);
    assert_eq!(Category::Tools.items(), [ShrinkPercent, CompressVideo]);
    assert_eq!(Category::Convert.items(), [JpgQuality, WebpQuality, Mp3Bitrate]);
    assert_eq!(Category::General.items(), [StartWithWindows, ProgressWindow, TrayRing]);
    assert!(CATEGORIES.iter().all(|c| c.items().len() <= MAX_ROWS));
}

#[test]
fn tool_defaults_step_and_stop() {
    let c = Config::default();
    assert_eq!(SettingItem::ShrinkPercent.value(&c), "50%");
    assert_eq!(SettingItem::CompressVideo.value(&c), "보통");
    let c = step(&c, SettingItem::ShrinkPercent, Dir::Forward).unwrap();
    assert_eq!(c.tools.shrink_percent, 75);
    assert_eq!(step(&c, SettingItem::ShrinkPercent, Dir::Forward), None);
    let c = step(&c, SettingItem::CompressVideo, Dir::Back).unwrap();
    assert_eq!(SettingItem::CompressVideo.value(&c), "약하게");
    assert_eq!(step(&c, SettingItem::CompressVideo, Dir::Back), None);
}

#[test]
fn key_items_do_not_step() {
    for item in Category::Keys.items() {
        assert!(item.is_key());
        assert_eq!(step(&Config::default(), *item, Dir::Forward), None);
    }
    assert!(!SettingItem::JpgQuality.is_key());
}

#[test]
fn bind_new_key() {
    let b = bind(&Config::default(), SettingItem::OpenKey, Key::F5);
    assert_eq!(b.config.open_key, Key::F5);
    assert_eq!(b.swapped, None);
}

#[test]
fn bind_taken_key_swaps() {
    let b = bind(&Config::default(), SettingItem::CycleKey, Key::Space);
    assert_eq!((b.config.open_key, b.config.cycle_key), (Key::Tab, Key::Space));
    assert_eq!(b.swapped, Some((SettingItem::OpenKey, Key::Tab)));
    assert!(crate::config::parse(&toml::to_string(&b.config).unwrap()).is_ok());
}

#[test]
fn bind_same_key_is_a_no_op() {
    let b = bind(&Config::default(), SettingItem::CancelKey, Key::Escape);
    assert_eq!(b.config, Config::default());
    assert_eq!(b.swapped, None);
}
```

Update `labels_and_values_in_order` to iterate `CATEGORIES.iter().flat_map(|c| c.items())` with the 11 rows in spec 2.1 order (도구: `("축소 비율", "50%")`, `("영상 압축", "보통")`).

- [ ] **Step 2: Run** `cargo test -p swisswheel-core settings` — expected: compile errors.
- [ ] **Step 3: Implement** as in Interfaces.
- [ ] **Step 4: Run** `cargo test --workspace` — expected: pass.
- [ ] **Step 5: Commit** `feat(core): settings categories, tool defaults, and key binding with swap`

---

### Task 3: Wheel option panel follows tool defaults

**Files:**
- Modify: `crates/swisswheel-core/src/options.rs`, `crates/swisswheel-core/src/session.rs`, `crates/swisswheel/src/app.rs` (`open_wheel_at`, `render`)

**Interfaces:**
- Consumes: Task 1 `Tools`.
- Produces: `OptionSet::default_index(&self, tools: &Tools) -> usize` — index of the choice whose preset is `Scale(tools.shrink_percent)` or `Compress(tools.compress_video)`, else `self.default`. `WheelSession::set_tools(&mut self, tools: Tools)` (field defaults to `Tools::default()`); `PanelState.default: usize`. A drop on the item itself uses `choices[default_index].preset`.

- [ ] **Step 1: Write failing tests**

In `options.rs`:
```rust
#[test]
fn tool_defaults_pick_the_default_row() {
    let tools = Tools { shrink_percent: 75, compress_video: Strength::Light };
    let shrink = options_for(&Operation::Tool(Tool::Shrink), &[image()]).unwrap();
    assert_eq!(shrink.default_index(&tools), 2);
    let compress = options_for(&Operation::Tool(Tool::CompressVideo), &[video()]).unwrap();
    assert_eq!(compress.default_index(&tools), 0);
    let gif = options_for(&Operation::Convert(Format::Gif), &[video()]).unwrap();
    assert_eq!(gif.default_index(&tools), gif.default); // not a tool set
}
```
In `session.rs`, next to `drop_on_a_row_uses_its_preset_and_the_tile_uses_the_default`, add `tile_drop_uses_configured_tool_default`: build a session for one PNG on the tools wheel (copy that test's setup), `set_tools(Tools { shrink_percent: 25, .. })`, drop on the Shrink tile → `preset == Some(Preset::Scale(25))`; open its panel → `panel().unwrap().default == 0`.

- [ ] **Step 2: Run** `cargo test -p swisswheel-core options session` — expected: compile errors.
- [ ] **Step 3: Implement**; in `app.rs` call `session.set_tools(self.config.tools.clone())` where `WheelSession::new()` is created in `open_wheel_at`, and pass `default: p.default` instead of `p.set.default` in `render`.
- [ ] **Step 4: Run** `cargo test --workspace` — expected: pass.
- [ ] **Step 5: Commit** `feat: tool defaults from settings drive the wheel's option panel`

---

### Task 4: Menu session — tabs, rows, capture

**Files:**
- Modify: `crates/swisswheel-core/src/menu.rs` (rewrite `MenuSession` and its tests)

**Interfaces:**
- Consumes: Task 2 `Category`, `CATEGORIES`, `SettingItem::is_key`.
- Produces:
  - `pub enum Focus { Tabs, Row(usize) }` (Copy, Eq).
  - `MenuHit` adds `Tab(usize)` and `KeyCap(usize)`.
  - `MenuAction` adds `StartCapture`, `Bind(SettingItem, u16)`, `CancelCapture`.
  - `MenuSession` methods: `tile() -> usize`, `category() -> Category`, `focus() -> Option<Focus>` (`None` = panel closed), `capturing() -> bool`, `hovered() -> MenuHit`, `key(MenuKey) -> MenuAction`, `hover(MenuHit) -> bool`, `click(MenuHit) -> MenuAction`, `scroll(MenuHit, Dir) -> MenuAction`, `capture(vk: u16) -> MenuAction`, `rearm()`, `cancel_capture() -> bool`, `hint() -> &'static str`.
- Behavior: spec 2.2 table and 2.4. Details the spec leaves open:
  - `category` survives panel close/reopen within one session; a new session starts on `Keys`.
  - While capturing, `key()` and `scroll()` return `None`. `capture(vk)` → `capturing = false`, `Bind(current row's item, vk)`; when not capturing → `None`. `rearm()` sets `capturing = true` again (the app calls it after rejecting a key). `cancel_capture()` returns whether it was capturing.
  - `click()` while capturing: same `KeyCap(i)` as the current row → `None`; `Outside` → `Close`; anything else → stop capturing, apply the normal click, and return its action, except that `None`/`Redraw` become `CancelCapture`.
  - `hint()`: capturing → `"바꿀 키를 누르세요 · 다른 곳 클릭 시 취소"`; `Tabs` → `"←/→ 카테고리 · ↓ 항목"`; key row → `"Enter로 키 변경"`; other row → `"←/→로 값 변경"`; panel closed → `""`.

- [ ] **Step 1: Write failing tests** (replace the module's tests; keep `starts_on_settings_and_stops_at_ends` and the Notice cases of `enter_or_down_runs_the_tile` as they are):

```rust
fn panel() -> MenuSession {
    let mut m = MenuSession::new();
    assert_eq!(m.key(MenuKey::Enter), MenuAction::Redraw);
    m
}

#[test]
fn enter_opens_panel_on_tabs_of_keys() {
    let m = panel();
    assert_eq!((m.focus(), m.category()), (Some(Focus::Tabs), Category::Keys));
    assert_eq!(m.hint(), "←/→ 카테고리 · ↓ 항목");
}

#[test]
fn tabs_switch_and_stop_at_ends() {
    let mut m = panel();
    assert_eq!(m.key(MenuKey::Left), MenuAction::None);
    for _ in 0..3 { assert_eq!(m.key(MenuKey::Right), MenuAction::Redraw); }
    assert_eq!(m.key(MenuKey::Right), MenuAction::None);
    assert_eq!(m.category(), Category::General);
    assert_eq!(m.focus(), Some(Focus::Tabs));
}

#[test]
fn rows_move_within_category_and_back_to_tabs() {
    let mut m = panel();
    m.key(MenuKey::Right); // 도구: 2 rows
    assert_eq!(m.key(MenuKey::Down), MenuAction::Redraw);
    assert_eq!(m.key(MenuKey::Down), MenuAction::Redraw);
    assert_eq!(m.key(MenuKey::Down), MenuAction::None);
    assert_eq!(m.focus(), Some(Focus::Row(1)));
    assert_eq!(m.key(MenuKey::Right), MenuAction::Change(SettingItem::CompressVideo, Dir::Forward));
    m.key(MenuKey::Up);
    assert_eq!(m.key(MenuKey::Up), MenuAction::Redraw);
    assert_eq!(m.focus(), Some(Focus::Tabs));
    m.key(MenuKey::Down);
    assert_eq!(m.key(MenuKey::Escape), MenuAction::Redraw);
    assert_eq!(m.focus(), Some(Focus::Tabs));
    assert_eq!(m.key(MenuKey::Escape), MenuAction::Redraw);
    assert_eq!(m.focus(), None);
    assert_eq!(m.key(MenuKey::Escape), MenuAction::Close);
}

#[test]
fn category_is_kept_when_panel_reopens() {
    let mut m = panel();
    m.key(MenuKey::Right);
    m.key(MenuKey::Up); // closes panel
    m.key(MenuKey::Enter);
    assert_eq!(m.category(), Category::Tools);
}

#[test]
fn key_rows_capture_on_enter_and_ignore_left_right() {
    let mut m = panel();
    m.key(MenuKey::Down);
    assert_eq!(m.hint(), "Enter로 키 변경");
    assert_eq!(m.key(MenuKey::Right), MenuAction::None);
    assert_eq!(m.key(MenuKey::Enter), MenuAction::StartCapture);
    assert!(m.capturing());
    assert_eq!(m.key(MenuKey::Open), MenuAction::None); // the hook sends MenuCapture instead
    assert_eq!(m.capture(0x74), MenuAction::Bind(SettingItem::OpenKey, 0x74));
    assert!(!m.capturing());
    assert_eq!(m.capture(0x74), MenuAction::None);
}

#[test]
fn rearm_and_cancel() {
    let mut m = panel();
    m.key(MenuKey::Down);
    m.key(MenuKey::Enter);
    m.capture(0x41);
    m.rearm();
    assert!(m.capturing());
    assert!(m.cancel_capture());
    assert!(!m.cancel_capture());
}

#[test]
fn mouse_on_tabs_rows_and_keycaps() {
    let mut m = panel();
    assert!(m.hover(MenuHit::Tab(2)));
    assert_eq!(m.category(), Category::Keys); // hover only highlights
    assert_eq!(m.click(MenuHit::Tab(2)), MenuAction::Redraw);
    assert_eq!((m.category(), m.focus()), (Category::Convert, Some(Focus::Tabs)));
    assert_eq!(m.click(MenuHit::Step(1, Dir::Back)), MenuAction::Change(SettingItem::WebpQuality, Dir::Back));
    assert_eq!(m.scroll(MenuHit::Row(2), Dir::Forward), MenuAction::Change(SettingItem::Mp3Bitrate, Dir::Forward));
    m.click(MenuHit::Tab(0));
    assert_eq!(m.scroll(MenuHit::Row(0), Dir::Forward), MenuAction::None);
    assert_eq!(m.click(MenuHit::KeyCap(2)), MenuAction::StartCapture);
    assert_eq!(m.focus(), Some(Focus::Row(2)));
}

#[test]
fn clicks_while_capturing() {
    let mut m = panel();
    m.click(MenuHit::KeyCap(0));
    assert_eq!(m.click(MenuHit::KeyCap(0)), MenuAction::None);
    assert!(m.capturing());
    assert_eq!(m.click(MenuHit::Row(1)), MenuAction::CancelCapture);
    assert_eq!((m.capturing(), m.focus()), (false, Some(Focus::Row(1))));
    m.click(MenuHit::KeyCap(1));
    assert_eq!(m.click(MenuHit::KeyCap(2)), MenuAction::StartCapture);
    assert_eq!(m.click(MenuHit::Outside), MenuAction::Close);
}
```

- [ ] **Step 2: Run** `cargo test -p swisswheel-core menu::` — expected: compile errors.
- [ ] **Step 3: Implement.** State: `tile`, `category: usize`, `panel: Option<Focus>`, `capturing: bool`, `hovered`. Hovering a non-settings tile still closes the panel (as `select_tile` does now). Stop using `ITEMS` here.
- [ ] **Step 4: Run** `cargo test -p swisswheel-core` — expected: pass (the app crate is expected to fail to build until Task 7).
- [ ] **Step 5: Commit** `feat(core): menu session with category tabs and key capture`

---

### Task 5: Menu geometry — tabs, keycaps, hint line

**Files:**
- Modify: `crates/swisswheel-core/src/menu_geometry.rs`

**Interfaces:**
- Consumes: Task 2 `Category`, `MAX_ROWS`; Task 4 `MenuHit::Tab`, `MenuHit::KeyCap`.
- Produces:
  - `RowBoxes { row, back, forward, keycap }`; `keycap` spans `back.left..forward.right` of the row.
  - `MenuLayout` adds `tabs: [Box2; 4]` and `hint: Box2`; `rows` always has `MAX_ROWS` entries.
  - `pub fn layout(notice: bool, scale: f32) -> MenuLayout`. Lines from the panel top after `PANEL_PAD`: notice (if any), tabs, `MAX_ROWS` rows, hint, each `ROW` high. The tabs split the inner width (`width - 2·PANEL_PAD`) into 4 equal boxes.
  - `#[derive(Clone, Copy)] pub struct PanelShape { pub rows: usize, pub keycaps: bool }`, `PanelShape::of(Category) -> PanelShape`.
  - `pub fn hit(layout: &MenuLayout, panel: Option<PanelShape>, x: f32, y: f32) -> MenuHit`: `None` = panel closed. Only the first `rows` rows hit. A keycap row gives `KeyCap(i)` inside `keycap` and `Row(i)` elsewhere in the row. Other rows give `Step`/`Row` as now. Tabs give `Tab(i)`. Notice, hint, and margins give `Panel`.

- [ ] **Step 1: Write failing tests** (update the existing ones to `layout(notice, scale)` and `hit(&l, Some(shape)/None, ..)`):

```rust
fn keys() -> Option<PanelShape> { Some(PanelShape::of(Category::Keys)) }
fn convert() -> Option<PanelShape> { Some(PanelShape::of(Category::Convert)) }

#[test]
fn panel_has_tabs_three_rows_and_hint() {
    let l = layout(false, 1.0);
    assert_eq!(l.panel.bottom - l.panel.top, 8.0 * 2.0 + 5.0 * 36.0);
    assert_eq!(l.rows.len(), 3);
    assert_eq!(l.tabs[0].top, l.panel.top + 8.0);
    assert_eq!((l.tabs[0].left, l.tabs[3].right), (8.0, 360.0));
    assert_eq!(l.tabs[1].left - l.tabs[0].left, 88.0);
    assert_eq!(l.rows[0].row.top, l.tabs[0].bottom);
    assert_eq!(l.hint.top, l.rows[2].row.bottom);
    assert_eq!((l.rows[0].keycap.left, l.rows[0].keycap.right), (l.rows[0].back.left, l.rows[0].forward.right));
}

#[test]
fn hit_tabs_keycaps_and_steps() {
    let l = layout(false, 1.0);
    let (x, y) = mid(&l.tabs[2]);
    assert_eq!(hit(&l, keys(), x, y), MenuHit::Tab(2));
    assert_eq!(hit(&l, None, x, y), MenuHit::Outside);
    let (x, y) = mid(&l.rows[1].keycap);
    assert_eq!(hit(&l, keys(), x, y), MenuHit::KeyCap(1));
    assert_eq!(hit(&l, keys(), l.rows[1].row.left + 4.0, y), MenuHit::Row(1));
    let (x, y) = mid(&l.rows[1].back);
    assert_eq!(hit(&l, convert(), x, y), MenuHit::Step(1, Dir::Back));
    let (x, y) = mid(&l.hint);
    assert_eq!(hit(&l, convert(), x, y), MenuHit::Panel);
}

#[test]
fn tools_tab_has_no_third_row() {
    let l = layout(false, 1.0);
    let tools = Some(PanelShape::of(Category::Tools));
    let (x, y) = mid(&l.rows[2].row);
    assert_eq!(hit(&l, tools, x, y), MenuHit::Panel);
    let (x, y) = mid(&l.rows[2].forward);
    assert_eq!(hit(&l, tools, x, y), MenuHit::Panel);
}
```

`notice_row_pushes_settings_down` becomes: with notice, `l.tabs[0].top == notice.bottom`.

- [ ] **Step 2: Run** `cargo test -p swisswheel-core menu_geometry` — expected: compile errors.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p swisswheel-core` — expected: pass.
- [ ] **Step 5: Commit** `feat(core): menu layout with category tabs, keycaps, and a hint line`

---

### Task 6: Drag tracker — one-shot key capture

**Files:**
- Modify: `crates/swisswheel-core/src/drag.rs`

**Interfaces:**
- Produces: `DragCommand::MenuCapture(u16)`; `DragTracker::set_capture(&mut self, on: bool)`.
- Behavior: while the menu is open and capture is on, the next **non-repeat** key-down of any key is swallowed: send `MenuCapture(vk)`, mark it in `menu_press` and as swallowed (so its repeats and release are swallowed too), then turn capture off. Repeats of keys already held go through the existing rules first. `MenuClosed`, and a drag closing the menu, turn capture off.

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn capture_swallows_any_key_once_and_its_release() {
    let mut t = menu_open();
    t.set_capture(true);
    assert_eq!(t.on_event(DragEvent::KeyDown(0x41)), swallow(DragCommand::MenuCapture(0x41)));
    assert_eq!(t.on_event(DragEvent::KeyDown(0x41)), swallow_only()); // its repeat
    assert_eq!(t.on_event(DragEvent::KeyUp(0x41)), swallow_only());
    assert_eq!(t.on_event(DragEvent::KeyDown(0x41)), pass()); // capture was one-shot
}

#[test]
fn capture_takes_the_open_key_and_modifiers() {
    let mut t = menu_open();
    t.set_capture(true);
    assert_eq!(t.on_event(DragEvent::DesktopKeyDown(SPACE)), swallow(DragCommand::MenuCapture(SPACE)));
    t.on_event(DragEvent::KeyUp(SPACE));
    t.set_capture(true);
    assert_eq!(t.on_event(DragEvent::KeyDown(0xA0)), swallow(DragCommand::MenuCapture(0xA0)));
}

#[test]
fn capture_ignores_repeats_of_held_keys() {
    let mut t = menu_open();
    t.on_event(DragEvent::KeyDown(0x27)); // → taken by the menu
    t.set_capture(true);
    assert_eq!(
        t.on_event(DragEvent::KeyDown(0x27)).command,
        Some(DragCommand::MenuKey(MenuKey::Right))
    );
    assert_eq!(t.on_event(DragEvent::KeyDown(0x74)), swallow(DragCommand::MenuCapture(0x74)));
}

#[test]
fn closing_the_menu_ends_capture() {
    let mut t = menu_open();
    t.set_capture(true);
    t.on_event(DragEvent::MenuClosed);
    let mut t2 = t; // reopen
    t2.on_event(DragEvent::DesktopKeyDown(SPACE));
    t2.on_event(DragEvent::KeyUp(SPACE));
    assert_eq!(t2.on_event(DragEvent::KeyDown(0x41)), pass());
}
```

- [ ] **Step 2: Run** `cargo test -p swisswheel-core drag` — expected: compile errors.
- [ ] **Step 3: Implement**: put the capture branch in `on_key_down` right after the `repeat && is_menu_press` block.
- [ ] **Step 4: Run** `cargo test -p swisswheel-core` — expected: pass.
- [ ] **Step 5: Commit** `feat(core): capture the next key press for the settings panel`

---

### Task 7: App — hook, menu flow, and drawing the tabbed panel

**Files:**
- Modify: `crates/swisswheel/src/hook.rs`, `crates/swisswheel/src/app.rs`, `crates/swisswheel/src/menubar/mod.rs`, `crates/swisswheel/src/menubar/window.rs`, `crates/swisswheel/src/menubar/render.rs`
- Modify: `crates/swisswheel-core/src/settings.rs` (delete `ITEMS`)

**Interfaces:**
- Consumes: Tasks 1–6.
- Produces:
  - `hook::encode`/`decode`: `MenuCapture(vk)` ↔ `(WPARAM(21), LPARAM(vk))`. `hook::set_capture(on: bool)` via `with_tracker`.
  - `menubar::rows(config: &Config, category: Category, failed: Option<SettingItem>) -> Vec<RowView>`; `RowView` adds `keycap: bool` (= `item.is_key()`).
  - `MenuView` replaces `row` with `category: usize`, `focus: Option<Focus>`, `capturing: bool`, `hint: &'a str`. `rows` stays `Option<&[RowView]>` (`Some` while the panel is open).
  - `MenuBar::place` and `MenuBar::new` call `layout(notice, scale)`.
  - App constants: `TIMER_MENU_CAPTURE: usize = 8`, `CAPTURE_TIMEOUT: Duration = 5 s`. Field `menu_hint: Option<String>`.

- [ ] **Step 1: Write the failing test** — add `DragCommand::MenuCapture(0x74)` and `DragCommand::MenuCapture(0x91)` to `hook::tests::commands_round_trip`.
- [ ] **Step 2: Run** `cargo test -p swisswheel hook` — expected: compile errors.
- [ ] **Step 3: Implement `hook`** (encode/decode, `set_capture`).
- [ ] **Step 4: Implement app menu flow in `app.rs`:**
  - `DragCommand::MenuCapture(vk)` → `menu.capture(vk)` → `on_menu_action`.
  - Clear `menu_hint` when a `MenuKey`, `MenuCapture`, click, or wheel arrives (before handling it). Hover doesn't clear it.
  - `on_menu_mouse` hits with `menu.focus().map(|_| PanelShape::of(menu.category()))`.
  - Arms of `on_menu_action`:
    - `StartCapture`: `hook::set_capture(true)`, (re)start `TIMER_MENU_CAPTURE`, render.
    - `CancelCapture`: `hook::set_capture(false)`, kill the timer, render.
    - `Bind(item, vk)`: kill the timer, then match `Key::from_vk(vk)`:
      - `None`: `menu.rearm()`, `hook::set_capture(true)`, restart the timer, set the hint to `"이 키는 쓸 수 없습니다"`.
      - `Some(key)`: call `settings::bind`. Call `apply_config(bound.config, item)` only if it differs from `self.config`. On a swap, set the hint to `format!("{}는 이제 {}입니다", other.label(), key_label(k))`.
      - Render in both cases.
  - `TIMER_MENU_CAPTURE` fires: kill it; if `menu.cancel_capture()`, call `hook::set_capture(false)` and render.
  - `close_menu`: kill `TIMER_MENU_CAPTURE` and clear `menu_hint`.
  - `render_menu`:
    - `panel_open = menu.focus().is_some()`.
    - Rows come from `menubar::rows(&self.config, menu.category(), self.save_failed)`.
    - `hint` is `self.menu_hint.as_deref().unwrap_or(menu.hint())`.
- [ ] **Step 5: Implement drawing in `render.rs`:**
  - Tabs line: `CATEGORIES` labels centered in `layout.tabs`.
    - The selected tab gets `colors.text` and a 2-DIP underline in `colors.accent_top`, inset 16 DIP from the tab's sides.
    - Other tabs get `colors.subtext`.
    - When `focus == Some(Focus::Tabs)`, the selected tab gets the accent gradient fill, like a selected row.
    - A hovered tab (`MenuHit::Tab`) gets the faded `tile_top` fill that rows use.
  - Divider: a 1-px line in `colors.rim` at `tabs[0].bottom`, inset by `PANEL_PAD`.
  - Key rows (`row.keycap`): no ◀ ▶. Draw the value inside a rounded rectangle on `boxes.keycap`, inset 4 DIP, with a 1-px `colors.rim` stroke. When `capturing` and this is the focused row, use a 2-px `colors.accent_top` stroke and the text `"입력…"`.
  - Hint: `view.hint` in the `small` format and `colors.subtext` in `layout.hint`, with a 1-px `colors.rim` divider at `hint.top`.
  - Only rows `0..rows.len()` are drawn.
- [ ] **Step 6: Previews.**
  - Update `preview("menu-settings")` to the new `MenuView`: Keys tab, `focus: Some(Focus::Row(0))`, notice on.
  - Add `"menu-tools"`: Tools tab, focus on tabs, no notice, hovered `Tab(2)`.
  - Add `"menu-capture"`: Keys tab, row 1 capturing, hint `"바꿀 키를 누르세요 · 다른 곳 클릭 시 취소"`.
  - Add the new names to the doc comment.
- [ ] **Step 7: Delete `settings::ITEMS`**, then run `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings`. Expected: all pass, no warnings.
- [ ] **Step 8: Look at the panel.** For each of `menu-settings`, `menu-tools`, and `menu-capture`, run `$env:SWISSWHEEL_PREVIEW="<name>"; cargo run -p swisswheel` and take a screenshot. Check that:
  - the tabs, underline, keycaps, and hint are inside the panel;
  - nothing overlaps;
  - the 도구 tab leaves the third row empty.
- [ ] **Step 9: Commit** `feat(app): tabbed settings panel with press-to-bind shortcut keys`

---

### Task 8: Docs

**Files:**
- Modify: `README.md`, `docs/manual-test.md`, `docs/superpowers/specs/2026-10-21-desktop-menu-design.md`, `docs/superpowers/specs/2026-10-21-settings-categories-design.md`

- [ ] **Step 1: README.**
  - Usage paragraph: the settings panel has tabs (단축키 · 도구 · 변환 · 일반), switched with ←/→ on the tab line, with ↓ to enter the rows. Shortcut rows change by pressing Enter (or clicking the key) and then the new key. A key already in use swaps.
  - List the allowed keys.
  - Note that an open key the desktop already uses (Delete, F2, F5, Home…) opens the menu there instead of its usual action.
  - "What works now" line: add tool defaults (shrink ratio, video compression strength).
- [ ] **Step 2: `docs/manual-test.md`.** Add rows M34–M38:
  - M34: tab switching by keys and by click; hovering a tab doesn't switch.
  - M35: bind the open key to F5, then press F5 on the desktop.
  - M36: set the cycle key to Space → swap hint shown, and the open key becomes Tab.
  - M37: press A during capture → rejected and nothing typed on the desktop. Wait 5 s → capture cancelled.
  - M38: set shrink ratio to 25%, drop a PNG on 축소 → the output is 25%, and the option panel marks 25% as 기본.
- [ ] **Step 3: Specs.**
  - In the old spec, put "→ `2026-10-21-settings-categories-design.md`로 대체" under the 2.3 and 2.4 headings.
  - In the new spec, change the swap hint example in 2.3 and 2.5 to `"휠 열기 키는 이제 Tab입니다"`.
- [ ] **Step 4: Commit** `docs: settings tabs, key binding, and tool defaults`
