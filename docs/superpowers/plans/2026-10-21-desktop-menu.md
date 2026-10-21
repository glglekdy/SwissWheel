# 바탕화면 메뉴 바와 설정 패널 구현 계획

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 바탕화면에서 `open_key`(기본 Space)를 누르면 화면 상단 가운데에 설정·플러그인·업데이트 타일 바가 뜨고, 설정 타일에서 내려오는 패널로 `config.toml`의 9개 값을 바로 바꾸고 저장·적용한다.

**Architecture:** 판정과 상태는 `swisswheel-core`의 순수 모듈(`settings`, `menu`, `menu_geometry`, 확장한 `drag`)에 두고 단위 테스트한다. 앱은 휠처럼 활성화되지 않는 최상위 팝업(`menubar`)에 Direct2D로 그리고, 키는 저수준 훅 → `DragTracker` → `PostMessage`로, 마우스는 팝업 창 메시지로 받는다. 설정 변경은 `app.rs`가 훅 키 바인딩, `Runner` 품질, 진행률 표시, Run 레지스트리에 즉시 반영하고 `toml_edit`으로 주석을 보존하며 저장한다.

**Tech Stack:** Rust 2024, `windows` 0.61 (Win32, Direct2D, DirectWrite, Windows.UI.Composition), `toml` 0.8, `toml_edit` 0.22 (`serde` 기능), `tempfile` (테스트)

**Spec:** `docs/superpowers/specs/2026-10-21-desktop-menu-design.md`

## Global Constraints

- 타일 순서와 라벨: `설정`, `플러그인`, `업데이트`. 아이콘 글리프: 설정 `U+E713`(Settings), 플러그인 `U+EA86`(Puzzle), 업데이트 `U+E895`(Sync).
- 플러그인·업데이트 타일 알림: 제목은 타일 라벨, 본문 `다음 버전에서 지원 예정입니다` (기존 동작 휠과 같은 `tray.notify`).
- 설정 항목 순서·라벨·값은 스펙 2.4 표 그대로. 품질 단계 50~100 (5 단위), MP3 단계 128/160/192/256/320, 표시 문자열 `"192 kbps"`, 켬/끔은 `"켬"`/`"끔"`, 키 표시는 `Space`/`Tab`/`Esc`/`CapsLock`/`` ` ``.
- 모든 단계 목록은 끝에서 멈춘다 (순환 없음). 불리언은 `[끔, 켬]` 순서.
- 애니메이션: 바 열림 150ms(위에서 8 DIP 내려오며 페이드), 패널 120ms 페이드. 기존 `OPEN_ANIMATION`, `PANEL_FADE` 상수를 재사용.
- 색·테마·유리 판정은 `wheel::render`의 `palette`, `light_theme`, `transparency_enabled`를 그대로 쓴다. 선택 강조는 `accent_top`/`accent_bottom`/`accent_text`.
- 사용자에게 보이는 문구와 로그는 한국어, 코드 주석은 영어 (기존 코드 관례).
- 훅 콜백은 막히지 않아야 한다: 새 판정은 클래스 이름 비교와 `GetGUIThreadInfo` 한 번까지.
- 저장 오류 알림 제목 `설정을 저장하지 못했습니다`, 깨진 설정 안내 행 `설정 파일에 오류가 있어 저장하지 않습니다`, 자동 실행 실패 알림 제목 `시작 프로그램 등록에 실패했습니다`.
- 새 의존성은 `toml_edit` 하나뿐 (`swisswheel` 크레이트, `Cargo.lock`의 0.22.27과 일치).
- `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`가 매 커밋에서 통과해야 한다 (CI 기준).

## Review Focus

1. **이름 바꾸기·검색 입력 중 Space** — 바탕화면 아이콘 이름 바꾸기(Edit 포커스) 중에는 Space가 그대로 입력되어야 한다. → Task 4의 `desktop_check` 순수 판정 테스트 `rename_edit_is_not_desktop`.
2. **열기 키를 누르고 있기** — Space를 길게 누르면 바가 열린 뒤 반복 입력으로 닫혔다 열리기를 반복하면 안 된다. 닫은 뒤 계속 누르고 있어도 다시 열리면 안 된다. → Task 4 테스트 `held_open_key_does_not_reopen_after_close`.
3. **패널에서 열기 키를 다른 키로 바꾼 직후** — 새 키가 곧바로 닫기 키가 되고, 옛 키(예: Space)는 앱으로 통과해야 한다. → Task 4 테스트 `set_keys_while_menu_open_uses_new_open_key`.
4. **보조 모니터(음수 좌표)와 125/150% 배율** — 바가 커서가 있는 모니터 작업 영역 상단 가운데에 정확히 놓이고 히트 테스트가 그림과 맞아야 한다. → Task 3 테스트 `origin_on_negative_monitor`, `hit_scales_with_dpi`.
5. **사용자가 손으로 고친 `config.toml`** — 주석, 키 순서, 모르는 키, 패널 단계 밖 값이 저장 후에도 살아 있어야 한다. → Task 5 테스트 `save_keeps_comments_order_and_unknown_keys`, Task 1 테스트 `off_step_values_show_as_is_and_step_toward_direction`.

---

## 파일 구조

| 파일 | 책임 |
|---|---|
| `crates/swisswheel-core/src/settings.rs` (신규) | 설정 항목 9개: 라벨, 값 표시, ← / → 한 단계 |
| `crates/swisswheel-core/src/menu.rs` (신규) | 메뉴 세션 상태 머신: 타일·패널 선택, 키·마우스 → 동작 |
| `crates/swisswheel-core/src/menu_geometry.rs` (신규) | 바·패널 배치(창 기준 물리 픽셀), 창 위치, 히트 테스트 |
| `crates/swisswheel-core/src/drag.rs` | `DesktopKeyDown`, 메뉴 상태, 메뉴 키 삼킴, 키 반복 판정, `set_keys` |
| `crates/swisswheel/src/hook.rs` | 바탕화면 판정, 명령 인코딩 확장, `set_keys`/`set_menu_enabled` |
| `crates/swisswheel/src/config_file.rs` | `save()` (toml_edit 병합) |
| `crates/swisswheel/src/autostart.rs` (신규) | HKCU Run 값 읽기/쓰기 |
| `crates/swisswheel/src/jobs.rs` | 공유 품질 `RwLock` + `set_quality` |
| `crates/swisswheel/src/surface.rs` (신규) | `progress.rs`의 `Surface`를 일반화한 유리 + D2D 그리기 면 |
| `crates/swisswheel/src/progress.rs` | `surface::Surface` 사용으로 교체 (동작 동일) |
| `crates/swisswheel/src/menubar/{mod,window,render}.rs` (신규) | 바 팝업 창, 마우스 메시지, 그리기 |
| `crates/swisswheel/src/app.rs` | 메뉴 열기/닫기, 입력 배선, 설정 적용·저장, 자동 닫힘 |
| `README.md`, `docs/manual-test.md`, `docs/superpowers/specs/2026-10-20-swisswheel-design.md` | 문서 |

---

### Task 1: 설정 항목 (`settings.rs`)

**Files:**
- Create: `crates/swisswheel-core/src/settings.rs`
- Modify: `crates/swisswheel-core/src/lib.rs` (`pub mod settings;`)

**Interfaces:**
- Consumes: `config::{Config, Key, Quality, ProgressDisplay}`
- Produces:
  - `pub enum Dir { Back, Forward }` (← / →) — `#[derive(Debug, Clone, Copy, PartialEq, Eq)]`
  - `pub enum SettingItem { OpenKey, CycleKey, CancelKey, StartWithWindows, JpgQuality, WebpQuality, Mp3Bitrate, ProgressWindow, TrayRing }` — 같은 derive
  - `pub const ITEMS: [SettingItem; 9]` (위 순서)
  - `impl SettingItem { pub fn label(self) -> &'static str; pub fn value(self, c: &Config) -> String }`
  - `pub fn step(c: &Config, item: SettingItem, dir: Dir) -> Option<Config>` — 바뀌지 않으면 `None`
  - `pub fn key_label(k: Key) -> &'static str`

- [ ] **Step 1: 실패하는 테스트 작성** (`settings.rs` 하단 `mod tests`)

```rust
#[test]
fn labels_and_values_in_order() {
    let c = Config::default();
    let rows: Vec<_> = ITEMS.iter().map(|i| (i.label(), i.value(&c))).collect();
    assert_eq!(rows, [
        ("휠 열기 키", "Space".into()), ("휠 전환 키", "Tab".into()), ("취소 키", "Esc".into()),
        ("Windows 시작 시 실행", "끔".into()), ("JPG 품질", "90".into()), ("WebP 품질", "85".into()),
        ("MP3 비트레이트", "192 kbps".into()), ("진행률 카드", "켬".into()), ("트레이 링", "켬".into()),
    ]);
}

#[test]
fn quality_steps_by_five_and_stops_at_ends() {
    let c = Config::default();
    assert_eq!(step(&c, SettingItem::JpgQuality, Dir::Forward).unwrap().quality.jpg, 95);
    assert_eq!(step(&c, SettingItem::JpgQuality, Dir::Back).unwrap().quality.jpg, 85);
    let top = Config { quality: Quality { jpg: 100, ..Quality::default() }, ..c.clone() };
    assert_eq!(step(&top, SettingItem::JpgQuality, Dir::Forward), None);
    let bottom = Config { quality: Quality { webp: 50, ..Quality::default() }, ..c };
    assert_eq!(step(&bottom, SettingItem::WebpQuality, Dir::Back), None);
}

#[test]
fn off_step_values_show_as_is_and_step_toward_direction() {
    let c = Config { quality: Quality { jpg: 42, webp: 87, mp3_kbps: 200 }, ..Config::default() };
    assert_eq!(SettingItem::JpgQuality.value(&c), "42");
    assert_eq!(step(&c, SettingItem::JpgQuality, Dir::Forward).unwrap().quality.jpg, 50);
    assert_eq!(step(&c, SettingItem::JpgQuality, Dir::Back), None);
    assert_eq!(step(&c, SettingItem::WebpQuality, Dir::Back).unwrap().quality.webp, 85);
    assert_eq!(step(&c, SettingItem::Mp3Bitrate, Dir::Forward).unwrap().quality.mp3_kbps, 256);
    assert_eq!(step(&c, SettingItem::Mp3Bitrate, Dir::Back).unwrap().quality.mp3_kbps, 192);
}

#[test]
fn keys_skip_values_used_by_the_other_two() {
    let c = Config::default(); // Space, Tab, Esc
    // open_key: Space → (Tab, Esc taken) → CapsLock
    assert_eq!(step(&c, SettingItem::OpenKey, Dir::Forward).unwrap().open_key, Key::CapsLock);
    assert_eq!(step(&c, SettingItem::OpenKey, Dir::Back), None); // Space is first
    // cancel_key: Esc → back skips Tab → Space taken → None
    assert_eq!(step(&c, SettingItem::CancelKey, Dir::Back), None);
    let next = step(&c, SettingItem::CancelKey, Dir::Forward).unwrap();
    assert_eq!(next.cancel_key, Key::CapsLock);
    assert!(crate::config::parse(&toml::to_string(&next).unwrap()).is_ok()); // stays valid
}

#[test]
fn booleans_are_off_then_on() {
    let c = Config::default();
    assert!(step(&c, SettingItem::StartWithWindows, Dir::Forward).unwrap().start_with_windows);
    assert_eq!(step(&c, SettingItem::StartWithWindows, Dir::Back), None);
    assert!(!step(&c, SettingItem::TrayRing, Dir::Back).unwrap().progress.tray_ring);
    assert_eq!(step(&c, SettingItem::ProgressWindow, Dir::Forward), None);
}
```

- [ ] **Step 2: 실패 확인** — `cargo test -p swisswheel-core settings` → 컴파일 실패 (`settings` 모듈 없음)
- [ ] **Step 3: 구현** — 단계 목록: 키 `[Space, Tab, Escape, CapsLock, Backquote]`, 품질 `(50..=100).step_by(5)`, MP3 `[128, 160, 192, 256, 320]`, 불리언 `[false, true]`. 공통 헬퍼 `fn next_in<T: PartialOrd + Copy>(steps: &[T], current: T, dir: Dir, skip: impl Fn(T) -> bool) -> Option<T>`가 "현재 값보다 엄격히 큰(Forward) / 작은(Back) 첫 단계 중 skip이 아닌 것"을 돌려준다. 키는 `Key`에 순서가 없으므로 단계 목록의 인덱스로 비교한다. 이 규칙 하나로 단계 위 값과 단계 밖 값이 모두 처리된다.
- [ ] **Step 4: 통과 확인** — `cargo test -p swisswheel-core settings` → 5 passed
- [ ] **Step 5: 커밋** — `git commit -m "feat(core): settings items with stepping rules for the settings panel"`

---

### Task 2: 메뉴 세션 (`menu.rs`)

**Files:**
- Create: `crates/swisswheel-core/src/menu.rs`
- Modify: `crates/swisswheel-core/src/lib.rs`

**Interfaces:**
- Consumes: `settings::{Dir, SettingItem, ITEMS}`
- Produces:
  - `pub enum Tile { Settings, Plugins, Update }`, `pub const TILES: [Tile; 3]`, `impl Tile { pub fn label(self) -> &'static str }` (`설정`/`플러그인`/`업데이트`)
  - `pub enum MenuKey { Left, Right, Up, Down, Enter, Escape, Open }` — `Open`은 현재 `open_key`
  - `pub enum MenuHit { Tile(usize), Row(usize), Step(usize, Dir), Panel, Outside }` — `Panel`은 패널 안 행이 아닌 곳(여백·안내 행)
  - `pub enum MenuAction { None, Redraw, Change(SettingItem, Dir), Notice(Tile), Close }`
  - `pub struct MenuSession` with `new() -> Self`, `key(&mut self, MenuKey) -> MenuAction`, `hover(&mut self, MenuHit) -> bool` (다시 그릴지), `click(&mut self, MenuHit) -> MenuAction`, `scroll(&mut self, MenuHit, Dir) -> MenuAction`, `tile(&self) -> usize`, `row(&self) -> Option<usize>` (패널이 닫혀 있으면 `None`), `hovered(&self) -> MenuHit`
  - 모든 enum `#[derive(Debug, Clone, Copy, PartialEq, Eq)]`

- [ ] **Step 1: 실패하는 테스트 작성**

```rust
#[test]
fn starts_on_settings_and_stops_at_ends() {
    let mut m = MenuSession::new();
    assert_eq!((m.tile(), m.row()), (0, None));
    assert_eq!(m.key(MenuKey::Left), MenuAction::None);
    assert_eq!(m.key(MenuKey::Right), MenuAction::Redraw);
    assert_eq!(m.key(MenuKey::Right), MenuAction::Redraw);
    assert_eq!(m.key(MenuKey::Right), MenuAction::None);
    assert_eq!(m.tile(), 2);
}

#[test]
fn enter_or_down_runs_the_tile() {
    let mut m = MenuSession::new();
    m.key(MenuKey::Right);
    assert_eq!(m.key(MenuKey::Enter), MenuAction::Notice(Tile::Plugins));
    m.key(MenuKey::Right);
    assert_eq!(m.key(MenuKey::Down), MenuAction::Notice(Tile::Update));
    let mut m = MenuSession::new();
    assert_eq!(m.key(MenuKey::Down), MenuAction::Redraw);
    assert_eq!(m.row(), Some(0));
}

#[test]
fn panel_rows_change_values_and_close_step_by_step() {
    let mut m = MenuSession::new();
    m.key(MenuKey::Enter);
    assert_eq!(m.key(MenuKey::Down), MenuAction::Redraw);
    assert_eq!(m.key(MenuKey::Right), MenuAction::Change(SettingItem::CycleKey, Dir::Forward));
    assert_eq!(m.key(MenuKey::Left), MenuAction::Change(SettingItem::CycleKey, Dir::Back));
    for _ in 0..20 { m.key(MenuKey::Down); }
    assert_eq!(m.row(), Some(8)); // stops at the last row
    assert_eq!(m.key(MenuKey::Escape), MenuAction::Redraw);
    assert_eq!(m.row(), None);
    assert_eq!(m.key(MenuKey::Escape), MenuAction::Close);
}

#[test]
fn up_on_first_row_returns_to_bar_and_open_closes_everything() {
    let mut m = MenuSession::new();
    m.key(MenuKey::Enter);
    assert_eq!(m.key(MenuKey::Up), MenuAction::Redraw);
    assert_eq!(m.row(), None);
    m.key(MenuKey::Enter);
    assert_eq!(m.key(MenuKey::Open), MenuAction::Close);
}

#[test]
fn left_right_on_bar_with_panel_closed_only_moves_tiles() {
    let mut m = MenuSession::new();
    m.key(MenuKey::Enter);
    m.key(MenuKey::Escape);
    assert_eq!(m.key(MenuKey::Right), MenuAction::Redraw);
    assert_eq!(m.tile(), 1);
}

#[test]
fn mouse_hits() {
    let mut m = MenuSession::new();
    assert!(m.hover(MenuHit::Tile(2)));
    assert_eq!(m.tile(), 2);
    assert!(!m.hover(MenuHit::Tile(2)));
    assert_eq!(m.click(MenuHit::Tile(0)), MenuAction::Redraw); // opens the panel
    assert_eq!(m.row(), Some(0));
    assert_eq!(m.click(MenuHit::Row(4)), MenuAction::Redraw);
    assert_eq!(m.row(), Some(4));
    assert_eq!(m.click(MenuHit::Step(5, Dir::Back)), MenuAction::Change(SettingItem::WebpQuality, Dir::Back));
    assert_eq!(m.row(), Some(5));
    assert_eq!(m.scroll(MenuHit::Row(6), Dir::Forward), MenuAction::Change(SettingItem::Mp3Bitrate, Dir::Forward));
    assert_eq!(m.scroll(MenuHit::Tile(1), Dir::Forward), MenuAction::None);
    assert_eq!(m.click(MenuHit::Panel), MenuAction::None);
    assert_eq!(m.click(MenuHit::Outside), MenuAction::Close);
    assert_eq!(m.click(MenuHit::Tile(0)), MenuAction::None); // settings already open
}
```

- [ ] **Step 2: 실패 확인** — `cargo test -p swisswheel-core menu::` → 컴파일 실패
- [ ] **Step 3: 구현** — 상태는 `tile: usize`, `row: Option<usize>`, `hovered: MenuHit`. 패널은 설정 타일에서만 열린다. 패널이 열려 있으면 ← / →는 `Change`, 바에 있으면 타일 이동. 설정 타일을 다시 실행(Enter·클릭)하면 패널이 이미 열려 있을 때 `None`. 다른 타일로 hover/이동하면 패널을 닫는다(`row = None`).
- [ ] **Step 4: 통과 확인** — `cargo test -p swisswheel-core menu::` → 6 passed
- [ ] **Step 5: 커밋** — `git commit -m "feat(core): menu session state for the desktop bar"`

---

### Task 3: 바·패널 배치 (`menu_geometry.rs`)

**Files:**
- Create: `crates/swisswheel-core/src/menu_geometry.rs`
- Modify: `crates/swisswheel-core/src/lib.rs`

**Interfaces:**
- Consumes: `geometry::Rect`, `menu::MenuHit`, `settings::Dir`
- Produces (DIP 상수, `pub const`): `TILE = 112.0`, `TILE_GAP = 8.0`, `BAR_PAD = 8.0`, `TOP_MARGIN = 16.0`, `PANEL_GAP = 8.0`, `ROW = 36.0`, `PANEL_PAD = 8.0`, `STEP_BUTTON = 28.0`, `VALUE_WIDTH = 88.0`, `DROP = 8.0`(열림 애니메이션 이동 거리, 창 위쪽 여유)
  - `pub struct Box2 { pub left: f32, pub top: f32, pub right: f32, pub bottom: f32 }` + `fn contains(&self, x: f32, y: f32) -> bool` (NaN 거부)
  - `pub struct RowBoxes { pub row: Box2, pub back: Box2, pub forward: Box2 }`
  - `pub struct MenuLayout { pub width: i32, pub height: i32, pub bar: Box2, pub tiles: [Box2; 3], pub panel: Box2, pub notice: Option<Box2>, pub rows: Vec<RowBoxes> }` — 창 기준 물리 픽셀. 바 위쪽에 `DROP` 여유.
  - `pub fn layout(rows: usize, notice: bool, scale: f32) -> MenuLayout` — 바 폭 = `3*TILE + 2*TILE_GAP + 2*BAR_PAD`; 패널 폭 = 바 폭; 패널 높이 = `2*PANEL_PAD + (rows + notice as usize) * ROW`; 안내 행은 패널 맨 위; 각 행의 `back`/`forward`는 행 오른쪽 끝에서 `forward`(STEP_BUTTON) ← 값(VALUE_WIDTH) ← `back`(STEP_BUTTON)
  - `pub fn window_origin(work_area: Rect, layout: &MenuLayout, scale: f32) -> (i32, i32)` — 가로 가운데, 세로 `work_area.top + TOP_MARGIN*scale - DROP*scale`(바가 `TOP_MARGIN`에 오도록)
  - `pub fn hit(layout: &MenuLayout, panel_open: bool, x: f32, y: f32) -> MenuHit`

- [ ] **Step 1: 실패하는 테스트 작성**

```rust
#[test]
fn bar_and_panel_share_width() {
    let l = layout(9, false, 1.0);
    assert_eq!(l.width, 368);
    assert_eq!(l.bar.right - l.bar.left, 368.0);
    assert_eq!(l.panel.right - l.panel.left, 368.0);
    assert_eq!(l.panel.bottom - l.panel.top, 8.0 * 2.0 + 9.0 * 36.0);
    assert_eq!(l.height as f32, l.panel.bottom);
    assert_eq!(l.panel.top, l.bar.bottom + 8.0);
}

#[test]
fn origin_is_top_center_of_work_area() {
    let l = layout(9, false, 1.0);
    let work = Rect { left: 0, top: 0, right: 1920, bottom: 1040 };
    assert_eq!(window_origin(work, &l, 1.0), ((1920 - 368) / 2, 16 - 8));
}

#[test]
fn origin_on_negative_monitor() {
    let l = layout(9, false, 1.5);
    let work = Rect { left: -2560, top: -200, right: 0, bottom: 1240 };
    let (x, y) = window_origin(work, &l, 1.5);
    assert_eq!(x, -2560 + (2560 - l.width) / 2);
    assert_eq!(y, -200 + 12);
}

#[test]
fn hit_tiles_rows_and_steps() {
    let l = layout(9, false, 1.0);
    let mid = |b: &Box2| ((b.left + b.right) / 2.0, (b.top + b.bottom) / 2.0);
    let (x, y) = mid(&l.tiles[1]);
    assert_eq!(hit(&l, false, x, y), MenuHit::Tile(1));
    let (x, y) = mid(&l.rows[3].row);
    assert_eq!(hit(&l, false, x, y), MenuHit::Outside); // panel closed
    assert_eq!(hit(&l, true, l.rows[3].row.left + 4.0, y), MenuHit::Row(3));
    let (x, y) = mid(&l.rows[3].back);
    assert_eq!(hit(&l, true, x, y), MenuHit::Step(3, Dir::Back));
    let (x, y) = mid(&l.rows[3].forward);
    assert_eq!(hit(&l, true, x, y), MenuHit::Step(3, Dir::Forward));
    assert_eq!(hit(&l, true, l.panel.left + 2.0, l.panel.top + 2.0), MenuHit::Panel);
    assert_eq!(hit(&l, true, f32::NAN, 0.0), MenuHit::Outside);
}

#[test]
fn notice_row_pushes_settings_down() {
    let l = layout(9, true, 1.0);
    let notice = l.notice.unwrap();
    assert_eq!(l.rows[0].row.top, notice.bottom);
    let y = (notice.top + notice.bottom) / 2.0;
    assert_eq!(hit(&l, true, notice.left + 4.0, y), MenuHit::Panel);
}

#[test]
fn hit_scales_with_dpi() {
    let l1 = layout(9, false, 1.0);
    let l2 = layout(9, false, 1.25);
    assert_eq!(l2.width, (368.0f32 * 1.25).ceil() as i32);
    let b = &l2.tiles[2];
    assert_eq!(hit(&l2, false, b.left + 1.0, b.top + 1.0), MenuHit::Tile(2));
    assert_eq!(l2.tiles[2].left, l1.tiles[2].left * 1.25);
}
```

- [ ] **Step 2: 실패 확인** — `cargo test -p swisswheel-core menu_geometry` → 컴파일 실패
- [ ] **Step 3: 구현** — 위 Interfaces의 공식대로. 바 여백과 타일 사이 틈(`TILE_GAP`)은 `MenuHit::Panel`(클릭해도 닫히지 않음). `Outside`는 바와 (열렸을 때) 패널의 바깥에만.
- [ ] **Step 4: 통과 확인** — `cargo test -p swisswheel-core menu_geometry` → 6 passed
- [ ] **Step 5: 커밋** — `git commit -m "feat(core): desktop bar and settings panel layout"`

---

### Task 4: 트래커와 훅 — 바탕화면 Space, 메뉴 키 (`drag.rs`, `hook.rs`)

**Files:**
- Modify: `crates/swisswheel-core/src/drag.rs`
- Modify: `crates/swisswheel/src/hook.rs`

**Interfaces:**
- Consumes: `menu::MenuKey`
- Produces:
  - `DragEvent::DesktopKeyDown(u16)` — 전경이 바탕화면일 때 훅이 `KeyDown` 대신 보냄. 그 밖에는 `KeyDown`과 똑같이 처리
  - `DragEvent::MenuClosed` — 앱이 바를 닫았을 때
  - `DragCommand::OpenMenu`, `DragCommand::MenuKey(MenuKey)`, `DragCommand::MenuPointerDown { x: i32, y: i32 }`, `DragCommand::CloseMenu`
  - `DragTracker::set_keys(&mut self, keys: KeyBindings)`, `menu_open(&self) -> bool`, `set_menu_enabled(&mut self, enabled: bool)` (기본 `true`)
  - `hook.rs`: `pub fn set_keys(keys: KeyBindings)`, `pub fn set_menu_enabled(enabled: bool)`, `pub fn is_desktop(hwnd: HWND) -> bool` (전경 변경 처리에서도 씀), `command_code`/`command_from_code`가 `(usize, isize)` ↔ `DragCommand`로 바뀜: `pub fn encode(command: DragCommand) -> (WPARAM, LPARAM)`, `pub fn decode(wparam: WPARAM, lparam: LPARAM) -> Option<DragCommand>`
  - `hook.rs` 내부 순수 함수 `fn desktop_check(foreground_class: &str, has_defview: bool, focus_class: &str) -> bool`

- [ ] **Step 1: 실패하는 테스트 작성** (`drag.rs` 테스트 모듈; 기존 테스트는 그대로 두고 추가)

```rust
const LEFT: u16 = 0x25; const UP: u16 = 0x26; const RIGHT: u16 = 0x27; const ENTER: u16 = 0x0D;
const CAPS: u16 = 0x14;

fn swallow(command: DragCommand) -> Decision { Decision { swallow: true, command: Some(command) } }

#[test]
fn space_on_desktop_opens_menu_and_swallows_pair() {
    let mut t = tracker();
    assert_eq!(t.on_event(DragEvent::DesktopKeyDown(SPACE)), swallow(DragCommand::OpenMenu));
    assert!(t.menu_open());
    assert_eq!(t.on_event(DragEvent::KeyUp(SPACE)), Decision { swallow: true, command: None });
    t.on_event(DragEvent::MenuClosed);
    assert_eq!(t.on_event(DragEvent::KeyDown(SPACE)), pass()); // not the desktop
}

#[test]
fn menu_keys_are_swallowed_others_pass() {
    let mut t = tracker();
    t.on_event(DragEvent::DesktopKeyDown(SPACE));
    t.on_event(DragEvent::KeyUp(SPACE));
    assert_eq!(t.on_event(DragEvent::KeyDown(RIGHT)), swallow(DragCommand::MenuKey(MenuKey::Right)));
    assert_eq!(t.on_event(DragEvent::KeyDown(RIGHT)), swallow(DragCommand::MenuKey(MenuKey::Right))); // arrows repeat
    assert_eq!(t.on_event(DragEvent::KeyUp(RIGHT)), Decision { swallow: true, command: None });
    assert_eq!(t.on_event(DragEvent::KeyDown(UP)).command, Some(DragCommand::MenuKey(MenuKey::Up)));
    assert_eq!(t.on_event(DragEvent::KeyDown(ENTER)), swallow(DragCommand::MenuKey(MenuKey::Enter)));
    assert_eq!(t.on_event(DragEvent::KeyDown(ENTER)), Decision { swallow: true, command: None }); // no repeat
    assert_eq!(t.on_event(DragEvent::KeyDown(A)), pass());
    assert_eq!(t.on_event(DragEvent::KeyDown(ESC)), swallow(DragCommand::MenuKey(MenuKey::Escape)));
    assert_eq!(t.on_event(DragEvent::DesktopKeyDown(SPACE)), swallow(DragCommand::MenuKey(MenuKey::Open)));
    assert!(t.menu_open()); // only the app closes it
    t.on_event(DragEvent::MenuClosed);
    assert!(!t.menu_open());
}

#[test]
fn held_open_key_does_not_reopen_after_close() {
    let mut t = tracker();
    t.on_event(DragEvent::DesktopKeyDown(SPACE));
    assert_eq!(t.on_event(DragEvent::DesktopKeyDown(SPACE)), Decision { swallow: true, command: None });
    t.on_event(DragEvent::MenuClosed);
    // still held: repeats pass through and do not reopen
    assert_eq!(t.on_event(DragEvent::DesktopKeyDown(SPACE)).command, None);
    t.on_event(DragEvent::KeyUp(SPACE));
    assert_eq!(t.on_event(DragEvent::DesktopKeyDown(SPACE)).command, Some(DragCommand::OpenMenu));
}

#[test]
fn drag_wins_over_menu() {
    let mut t = dragging();
    assert_eq!(t.on_event(DragEvent::DesktopKeyDown(SPACE)).command, Some(DragCommand::OpenWheel));
    let mut t = tracker();
    t.on_event(DragEvent::DesktopKeyDown(SPACE));
    assert_eq!(t.on_event(DragEvent::ButtonDown { x: 5, y: 6 }),
        Decision { swallow: false, command: Some(DragCommand::MenuPointerDown { x: 5, y: 6 }) });
    assert_eq!(t.on_event(DragEvent::Move { x: 50, y: 6 }),
        Decision { swallow: false, command: Some(DragCommand::CloseMenu) });
    assert!(!t.menu_open());
}

#[test]
fn set_keys_while_menu_open_uses_new_open_key() {
    let mut t = tracker();
    t.on_event(DragEvent::DesktopKeyDown(SPACE));
    t.on_event(DragEvent::KeyUp(SPACE));
    t.set_keys(KeyBindings { open: CAPS, cycle: TAB, cancel: ESC });
    assert_eq!(t.on_event(DragEvent::DesktopKeyDown(SPACE)), pass());
    assert_eq!(t.on_event(DragEvent::KeyUp(SPACE)), pass());
    assert_eq!(t.on_event(DragEvent::DesktopKeyDown(CAPS)).command, Some(DragCommand::MenuKey(MenuKey::Open)));
}

#[test]
fn disabled_menu_lets_space_through() {
    let mut t = tracker();
    t.set_menu_enabled(false);
    assert_eq!(t.on_event(DragEvent::DesktopKeyDown(SPACE)), pass());
}

#[test]
fn left_is_menu_key() {
    let mut t = tracker();
    t.on_event(DragEvent::DesktopKeyDown(SPACE));
    assert_eq!(t.on_event(DragEvent::KeyDown(LEFT)).command, Some(DragCommand::MenuKey(MenuKey::Left)));
}
```

`hook.rs` 하단에 `#[cfg(test)] mod tests` 추가:

```rust
#[test]
fn desktop_classes() {
    assert!(desktop_check("Progman", true, "SysListView32"));
    assert!(desktop_check("WorkerW", true, "SysListView32"));
    assert!(!desktop_check("WorkerW", false, "SysListView32")); // a wallpaper WorkerW
    assert!(!desktop_check("CabinetWClass", true, "DirectUIHWND")); // Explorer window
}

#[test]
fn rename_edit_is_not_desktop() {
    assert!(!desktop_check("Progman", true, "Edit"));
}

#[test]
fn commands_round_trip() {
    for c in [DragCommand::OpenWheel, DragCommand::Cycle, DragCommand::Cancel,
              DragCommand::ButtonReleasedWhileOpen, DragCommand::OpenMenu, DragCommand::CloseMenu,
              DragCommand::MenuKey(MenuKey::Open), DragCommand::MenuKey(MenuKey::Escape),
              DragCommand::MenuPointerDown { x: -2500, y: 1400 }] {
        let (w, l) = encode(c);
        assert_eq!(decode(w, l), Some(c));
    }
}
```

- [ ] **Step 2: 실패 확인** — `cargo test --workspace drag:: hook::` → 컴파일 실패
- [ ] **Step 3: 트래커 구현** — 필드 `menu: bool`, `menu_enabled: bool`, `held: [bool; 256]`. `KeyDown`/`DesktopKeyDown` 공통 처리에서 `repeat = held[vk]`를 먼저 읽고 `held[vk] = true`; `KeyUp`에서 `false`. 우선순위: 휠 열림 → 기존 처리 / 메뉴 열림 → 메뉴 키 매핑(`open`이면 `Open`, VK 0x25~0x28 화살표, 0x0D `Enter`, 0x1B `Escape`; 화살표 외에는 `repeat`이면 명령 없이 삼킴, 그 외 키는 통과) / 드래그 중 → 기존 휠 열기 / `DesktopKeyDown(open)` && `!repeat` && `menu_enabled` → `menu = true`, `OpenMenu`. `ButtonDown`: 메뉴 열림이면 `MenuPointerDown`. `Move`로 `Dragging` 전이 시 메뉴 열림이면 `menu = false`, `CloseMenu`. `held`는 `swallowed`와 같은 범위 검사(`get`/`get_mut`)로.
- [ ] **Step 4: 훅 구현** — `keyboard_proc`의 키 누름에서 `if is_desktop(GetForegroundWindow())`이면 `DesktopKeyDown`. `is_desktop`: `GetClassNameW`로 전경 클래스, `FindWindowExW(Some(fg), None, w!("SHELLDLL_DefView"), None).is_ok()`, `GetGUIThreadInfo(GetWindowThreadProcessId(fg, None), &mut info)`의 `hwndFocus` 클래스(실패 시 빈 문자열)를 `desktop_check`에 넘긴다. `desktop_check` = 전경이 `Progman`/`WorkerW` && `has_defview` && 포커스 클래스 ≠ `Edit`. `encode`: 기존 코드 1~4 유지, `OpenMenu`=5, `CloseMenu`=6, `MenuKey`=10+변형 순번(0~6), `MenuPointerDown`=20이고 좌표는 `LPARAM`에 `(x as u16 as u32 | (y as u16 as u32) << 16)` (`decode`에서 `i16`로 복원). `feed`가 `encode` 결과를 그대로 `PostMessageW`. `app.rs`의 `WM_APP_DRAG` 처리도 `hook::decode(wparam, lparam)`로 바꾼다. `set_keys`/`set_menu_enabled`는 `TRACKER`를 `try_borrow_mut`로 빌려 위임.
- [ ] **Step 5: 통과 확인** — `cargo test --workspace` → 기존 + 새 테스트 모두 통과, `cargo clippy --workspace -- -D warnings` 깨끗. (`app.rs`의 새 명령 분기는 다음 태스크 전까지 `DragCommand::OpenMenu | ... => {}`로 둔다.)
- [ ] **Step 6: 커밋** — `git commit -m "feat(core,app): open a menu with the open key on the desktop"`

---

### Task 5: 설정 저장 (`config_file::save`)

**Files:**
- Modify: `crates/swisswheel/Cargo.toml` (`toml_edit = { version = "0.22", features = ["serde"] }`)
- Modify: `crates/swisswheel/src/config_file.rs`

**Interfaces:**
- Produces: `pub fn save(dir: &Path, config: &Config) -> std::io::Result<()>` — 파일이 없으면 만든다(`create_dir_all` 포함). 기존 파일이 TOML로 읽히지 않으면 `io::ErrorKind::InvalidData`로 실패하고 파일을 건드리지 않는다.

- [ ] **Step 1: 실패하는 테스트 작성** (기존 `mod tests`에 추가)

```rust
#[test]
fn save_keeps_comments_order_and_unknown_keys() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "# my keys\nopen_key = \"Space\" # hold me\nfuture = 1\n\n[quality]\n# sharp\njpg = 42\n").unwrap();
    let mut c = load(dir.path()).0;
    c.open_key = Key::CapsLock;
    c.quality.webp = 70;
    save(dir.path(), &c).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.starts_with("# my keys\nopen_key = \"CapsLock\" # hold me\nfuture = 1\n"));
    assert!(text.contains("# sharp\njpg = 42"));
    assert!(text.contains("webp = 70"));
    assert_eq!(swisswheel_core::config::parse(&text).unwrap(), c);
}

#[test]
fn save_creates_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("SwissWheel");
    let c = Config { start_with_windows: true, ..Config::default() };
    save(&sub, &c).unwrap();
    assert_eq!(load(&sub).0, c);
}

#[test]
fn save_refuses_unreadable_toml() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "this is not toml").unwrap();
    let err = save(dir.path(), &Config::default()).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "this is not toml");
}
```

- [ ] **Step 2: 실패 확인** — `cargo test -p swisswheel config_file` → 컴파일 실패
- [ ] **Step 3: 구현** — 기존 텍스트를 `toml_edit::DocumentMut`로 파싱(없으면 빈 문서), `toml_edit::ser::to_document(config)`를 재귀 병합: 양쪽 다 테이블이면 재귀, 기존 값이 있으면 값만 바꾸고 기존 `decor`(앞 주석·뒤 주석)를 새 값에 옮김, 없으면 삽입. 기존 문서에만 있는 키는 그대로 둔다. 같은 폴더에 `config.toml.tmp`로 쓴 뒤 `std::fs::rename`으로 교체(부분 쓰기 방지).
- [ ] **Step 4: 통과 확인** — `cargo test -p swisswheel config_file` → 6 passed
- [ ] **Step 5: 커밋** — `git commit -m "feat(app): save config edits without losing comments"`

---

### Task 6: Windows 시작 시 실행 (`autostart.rs`)

**Files:**
- Create: `crates/swisswheel/src/autostart.rs`
- Modify: `crates/swisswheel/src/main.rs` (`mod autostart;`)

**Interfaces:**
- Produces:
  - `pub fn is_enabled() -> bool`, `pub fn set(enabled: bool) -> windows::core::Result<()>` — `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, 값 이름 `SwissWheel`, 데이터 `"<현재 exe 전체 경로>"`(큰따옴표 포함, `REG_SZ`)
  - 테스트용 내부 함수 `fn is_enabled_in(subkey: PCWSTR) -> bool`, `fn set_in(subkey: PCWSTR, enabled: bool) -> Result<()>` — 공개 함수는 이것들을 Run 경로로 호출
  - `is_enabled`는 값이 있고 데이터가 현재 exe 경로를 가리킬 때만 `true` (다른 위치의 옛 설치를 가리키면 `false` → 시작 시 다시 씀)

- [ ] **Step 1: 실패하는 테스트 작성**

```rust
#[test]
fn set_and_clear_round_trip() {
    let key = w!("Software\\SwissWheelTest\\Run");
    let _ = set_in(key, false);
    assert!(!is_enabled_in(key));
    set_in(key, true).unwrap();
    assert!(is_enabled_in(key));
    set_in(key, false).unwrap();
    assert!(!is_enabled_in(key));
    set_in(key, false).unwrap(); // clearing twice is fine
    unsafe { let _ = RegDeleteTreeW(HKEY_CURRENT_USER, w!("Software\\SwissWheelTest")); }
}
```

- [ ] **Step 2: 실패 확인** — `cargo test -p swisswheel autostart` → 컴파일 실패
- [ ] **Step 3: 구현** — `RegCreateKeyExW` + `RegSetValueExW`(`REG_SZ`, UTF-16 + NUL), 해제는 `RegDeleteKeyValueW`에서 `ERROR_FILE_NOT_FOUND`를 성공으로 본다. 읽기는 `RegGetValueW(RRF_RT_REG_SZ)`. exe 경로는 `std::env::current_exe()`.
- [ ] **Step 4: 통과 확인** — `cargo test -p swisswheel autostart` → 1 passed
- [ ] **Step 5: 커밋** — `git commit -m "feat(app): register SwissWheel to start with Windows"`

---

### Task 7: 실행 중 품질 변경 (`jobs.rs`)

**Files:**
- Modify: `crates/swisswheel/src/jobs.rs`

**Interfaces:**
- Produces: `Runner::set_quality(&self, quality: Quality)`. `Shared.quality`는 `RwLock<Quality>`; 워커는 파일 하나를 시작할 때 `read().clone()`으로 한 번 읽어 그 파일 끝까지 쓴다.

- [ ] **Step 1: 실패하는 테스트 작성** (기존 테스트 헬퍼로 PNG 샘플을 만든다 — `batch_convert`가 쓰는 것과 같은 방식)

```rust
#[test]
fn set_quality_applies_to_the_next_job() {
    // Two JPG conversions of the same noisy PNG: quality 100, then 50 after set_quality.
    let (big, small) = convert_twice_with_quality(100, 50);
    assert!(small < big, "quality 50 ({small} B) should be smaller than 100 ({big} B)");
}
```

`convert_twice_with_quality(first: u8, second: u8) -> (u64, u64)`는 테스트 모듈 안 헬퍼: 노이즈 PNG(256×256)를 만들고 `Runner::new(1, 1, Quality { jpg: first, ..Quality::default() }, …)`로 JPG 변환 → 보고를 기다림 → `set_quality(Quality { jpg: second, .. })` → 같은 입력으로 다시 변환 → 두 출력 크기를 돌려준다. 보고 대기는 기존 테스트의 채널 패턴을 따른다.

- [ ] **Step 2: 실패 확인** — `cargo test -p swisswheel set_quality` → 컴파일 실패 (`set_quality` 없음)
- [ ] **Step 3: 구현** — `Shared.quality: RwLock<Quality>`, 사용처 두 곳(`ffmpeg::prepare`, `engines::image::run`)은 파일 시작 시 읽은 복사본을 넘긴다. 잠금이 독이 들었으면(`PoisonError`) `into_inner()`로 값을 쓴다.
- [ ] **Step 4: 통과 확인** — `cargo test -p swisswheel jobs` → 기존 + 1 passed
- [ ] **Step 5: 커밋** — `git commit -m "feat(app): let quality settings change while running"`

---

### Task 8: 공용 유리 면 (`surface.rs`) — 진행 카드에서 꺼내기

**Files:**
- Create: `crates/swisswheel/src/surface.rs`
- Modify: `crates/swisswheel/src/progress.rs` (자체 `Surface` 제거, `surface::Surface` 사용)
- Modify: `crates/swisswheel/src/main.rs` (`mod surface;`)
- Modify: `crates/swisswheel/src/wheel/render.rs` (`icon_font`를 `pub(crate)`로)

**Interfaces:**
- Produces:
  - `pub struct Surface` with
    - `pub fn new(hwnd: HWND, width: u32, height: u32, glass_regions: usize) -> Result<Surface>` — 유리(호스트 백드롭) 스프라이트를 `glass_regions`개 만든다(각각 둥근 사각형 클립)
    - `pub fn size(&self) -> (u32, u32)`
    - `pub fn set_glass(&self, index: usize, rect: Option<D2D_RECT_F>, corner: f32) -> Result<()>` — `None`이면 그 유리를 숨김
    - `pub fn ctx(&self) -> &ID2D1DeviceContext`, `pub fn dwrite(&self) -> &IDWriteFactory`
    - `pub fn present(&self) -> Result<()>` — `swap.Present(1, 0)`
    - `pub fn text_format(&self, family: PCWSTR, size: f32, weight: DWRITE_FONT_WEIGHT, align: DWRITE_TEXT_ALIGNMENT) -> Result<IDWriteTextFormat>` — 세로 가운데, 줄바꿈 없음, 말줄임표
  - `pub fn rounded(rect: D2D_RECT_F, r: f32) -> D2D1_ROUNDED_RECT`, `pub fn inset(rect: D2D_RECT_F, by: f32) -> D2D_RECT_F`, `pub fn draw_text(ctx, text: &str, format, rect, brush)` — 지금 `progress.rs`의 헬퍼를 옮긴다

- [ ] **Step 1: 옮기기** — `progress.rs`의 `Surface::new`를 `surface.rs`로 옮기며 `backdrop`/`clip`을 `Vec<(SpriteVisual, CompositionRoundedRectangleGeometry)>`로 일반화. `progress.rs`의 `draw`는 `set_glass(0, Some(card), corner)`, `ctx()`, `present()`를 쓰도록 바꾼다. 그리는 내용은 한 줄도 바꾸지 않는다.
- [ ] **Step 2: 확인** — `cargo build -p swisswheel && cargo test --workspace && cargo clippy --workspace -- -D warnings` 통과. 디버그 빌드로 `SWISSWHEEL_PREVIEW=progress` 실행 → 오른쪽 아래 카드가 리팩터 전과 같게 보인다(스크린샷으로 확인).
- [ ] **Step 3: 커밋** — `git commit -m "refactor(app): share the glass drawing surface"`

---

### Task 9: 메뉴 바 창과 그리기 (`menubar/`)

**Files:**
- Create: `crates/swisswheel/src/menubar/mod.rs`, `window.rs`, `render.rs`
- Modify: `crates/swisswheel/src/main.rs` (`mod menubar;`)
- Modify: `crates/swisswheel/src/app.rs` (미리보기 `menu`, `menu-settings` 추가)

**Interfaces:**
- Consumes: `surface::Surface`, `menu_geometry::{layout, window_origin, hit, MenuLayout}`, `menu::{MenuHit, TILES, Tile}`, `wheel::render::{palette, light_theme, transparency_enabled, icon_font}`
- Produces:
  - `mod.rs`: 
    ```rust
    pub struct MenuView<'a> {
        pub selected: usize,
        /// Rows of the open settings panel: (label, value, save failed).
        pub rows: Option<&'a [(&'static str, String, bool)]>,
        pub row: Option<usize>,
        pub hovered: MenuHit,
        pub notice: Option<&'a str>,
        /// Bar open animation and panel fade, 0.0 → 1.0.
        pub progress: f32,
        pub panel_progress: f32,
    }
    pub enum MenuMouse { Move { x: f32, y: f32 }, Leave, Click { x: f32, y: f32 }, Wheel { x: f32, y: f32, delta: i16 } }
    pub use window::MenuBar;
    ```
    좌표는 창 기준 물리 픽셀.
  - `window.rs`: `pub struct MenuBar` with `new(owner: HWND) -> Result<MenuBar>`, `place(&mut self, cursor: POINT, notice: bool)` (커서 모니터 DPI·작업 영역으로 `layout(ITEMS.len(), notice, scale)`과 `window_origin`), `layout(&self) -> &MenuLayout`, `set_panel(&mut self, open: bool)` (창 영역 = 바 둥근 사각형 ∪ 열렸으면 패널), `show`, `hide`, `is_visible`, `contains_screen(&self, x: i32, y: i32) -> bool` (현재 영역 기준), `render(&mut self, view: &MenuView)`, `release_if_idle(&mut self, idle: Duration)`. 창 스타일은 휠과 같음(`WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE`, `WM_MOUSEACTIVATE → MA_NOACTIVATE`, `WM_CLOSE`는 주인에게). `wnd_proc`는 `WM_MOUSEMOVE`(처음이면 `TrackMouseEvent(TME_LEAVE)`), `WM_MOUSELEAVE`, `WM_LBUTTONDOWN`, `WM_MOUSEWHEEL`(화면 좌표 → `ScreenToClient`)을 `crate::app::on_menu_mouse(MenuMouse)`로 넘긴다.
  - `render.rs`: `pub(super) fn draw(surface: &Surface, view: &MenuView, layout: &MenuLayout, scale: f32, glass: bool) -> Result<()>`
    - 바: 유리 0번 = 바 상자(모서리 12 DIP), 배경 `disc`, 테두리 `rim`. 열림 애니메이션은 `progress`로 바 전체를 `DROP*(1-progress)` 위로 올려 그리고 `alpha = progress`.
    - 타일: `TILE` 정사각형, 모서리 8 DIP, 기본은 `tile_top→tile_bottom` 그라데이션을 `alpha 0.55`로 낮춘 반투명, 선택 타일은 `accent_top→accent_bottom` + `accent_text`. 아이콘 34 DIP(아이콘 폰트), 라벨 13 DIP Semi-Bold(`Segoe UI Variable Text`), 아이콘은 타일 위쪽 60%, 라벨은 아래 40%.
    - 패널: 유리 1번 = 패널 상자(모서리 10 DIP), `panel_progress`로 페이드. 행: 왼쪽 라벨(13 DIP), 오른쪽 `◀ 값 ▶`(◀▶는 아이콘 폰트 `U+E76B`/`U+E76C`, 끝값이라 못 움직이면 `subtext`), 선택 행은 `accent` 둥근 사각형(모서리 8 DIP), 마우스 올린 행은 `tile_top` alpha 0.35. 저장 실패 행은 값 앞에 `⚠`(`U+E7BA`). 안내 행은 `subtext` 12 DIP.
  - 미리보기(디버그 빌드): `SWISSWHEEL_PREVIEW=menu` → 바만, `menu-settings` → 패널 열고 4번째 행 선택, 안내 행 포함.

- [ ] **Step 1: 창 구현** — 휠 창(`wheel/window.rs`)의 생성·DPI·`SetWindowRgn` 패턴을 따른다. 영역은 `CreateRoundRectRgn`으로 바와 (열렸으면) 패널을 합친다. 렌더러는 `with_renderer`처럼 크기가 바뀌면 다시 만들고, 실패하면 로그 후 `None`.
- [ ] **Step 2: 그리기 구현** — 위 명세대로.
- [ ] **Step 3: 미리보기 추가** — `app.rs`의 `preview`에 두 경우 추가. 이 태스크에서는 `AppState`에 `menubar: MenuBar` 필드만 추가하고(`run()`에서 생성, 실패 시 `log_error!` 후 `hook::set_menu_enabled(false)`와 `Option<MenuBar>` = `None`), 입력 배선은 Task 10.
- [ ] **Step 4: 확인** — `cargo build -p swisswheel`, clippy 깨끗. `SWISSWHEEL_PREVIEW=menu`, `menu-settings`를 다크/라이트, 배율 100%/150%에서 띄워 스크린샷으로 확인: 상단 가운데, 레퍼런스처럼 반투명 정사각 타일 3개(아이콘 + 아래 라벨), 선택 타일 강조, 패널 9행 + 안내 행, 잘림 없음.
- [ ] **Step 5: 커밋** — `git commit -m "feat(app): draw the desktop menu bar and settings panel"`

---

### Task 10: 앱 배선 — 열기·닫기·입력·설정 적용 (`app.rs`)

**Files:**
- Modify: `crates/swisswheel/src/app.rs`
- Modify: `crates/swisswheel/src/hook.rs` (전경 변경 WinEvent 훅)

**Interfaces:**
- Consumes: Task 1~9의 모든 공개 항목
- Produces:
  - `AppState`에 `config: Config`, `config_dir: PathBuf`, `config_writable: bool`(시작 시 `load` 경고가 없었으면 `true`), `menu: Option<MenuSession>`, `menubar: Option<MenuBar>`, `menu_opened_at: Instant`, `menu_panel_at: Instant`, `save_failed: Option<SettingItem>`, `foreground_hook: Option<HWINEVENTHOOK>` (기존 `progress_display` 필드는 `config.progress`로 대체)
  - `pub fn on_menu_mouse(event: MenuMouse)` (창 → 앱)
  - `hook.rs`: `pub const WM_APP_FOREGROUND: u32 = WM_APP + 21;`, `pub fn watch_foreground(target: HWND) -> Option<HWINEVENTHOOK>` (`SetWinEventHook(EVENT_SYSTEM_FOREGROUND, …, WINEVENT_OUTOFCONTEXT)`; 콜백은 `PostMessageW(target, WM_APP_FOREGROUND, WPARAM(hwnd as usize), 0)`), `pub fn unwatch(h: HWINEVENTHOOK)`
  - 새 타이머 `TIMER_MENU_ANIMATION = 7` (16ms, 바 열림·패널 페이드가 끝나면 멈춤)

동작 (스펙 2장 그대로):
- `OpenMenu` → 휠이 열려 있으면 무시하고 `hook::notify(MenuClosed)`. 아니면 `MenuSession::new()`, `menubar.place(커서, !config_writable)`, `set_panel(false)`, `show`, 전경 감시 시작, 애니메이션 타이머.
- `MenuKey(k)` → `session.key(k)` 결과 처리. `MenuPointerDown{x,y}` → `!menubar.contains_screen(x, y)`이면 닫기. `CloseMenu` → 닫기(트래커는 이미 닫힘이라 `notify` 불필요하지만 해도 무해).
- `on_menu_mouse`: `Move`/`Leave` → `hit`(Leave는 `Outside`)로 `hover`, 바뀌면 다시 그림. `Click` → `click(hit)`. `Wheel` → `scroll(hit, dir)`; 위로 굴림(`delta > 0`) = `Dir::Forward`(값 증가), 아래로 = `Dir::Back`.
- 동작 처리: `Redraw` → 패널 열림 상태가 바뀌었으면 `set_panel`·`menu_panel_at` 갱신 후 그림. `Notice(tile)` → 닫고 `tray.notify(tile.label(), "다음 버전에서 지원 예정입니다")`. `Close` → 닫기. `Change(item, dir)` → `settings::step(&config, item, dir)`가 `Some(new)`이면 `apply_config(new, item)`.
- `apply_config(new, item)`: 키가 바뀌었으면 `hook::set_keys(KeyBindings::from(&new))`; 품질이면 `runner.set_quality(new.quality.clone())`; 진행률이면 `config` 교체 후 `update_busy()`; 자동 실행이면 `autostart::set(new.start_with_windows)` — 실패 시 값을 되돌리고(`new.start_with_windows = !…`) `tray.warn("시작 프로그램 등록에 실패했습니다", &e.to_string())`. 그다음 `self.config = new`; `config_writable`이면 `config_file::save` — 실패 시 `save_failed = Some(item)`, 처음 실패일 때만 `tray.warn("설정을 저장하지 못했습니다", &e.to_string())`, `log_error!`; 성공 시 `save_failed = None`. 다시 그림.
- 닫기(`close_menu`): 타이머 중지, `hide`, `set_panel(false)`, 전경 감시 해제, `menu = None`, `hook::notify(DragEvent::MenuClosed)`.
- `WM_APP_FOREGROUND` → 메뉴가 열려 있고 `!hook::is_desktop(새 전경)`이면 닫기. `WM_DISPLAYCHANGE`, `WM_SETTINGCHANGE`(작업 영역 변경)·`WM_DPICHANGED` 계열 → 열려 있으면 닫기.
- 휠 열기(`open_wheel`) 시 메뉴가 열려 있으면 먼저 닫기.
- 시작 시: `autostart::is_enabled() != config.start_with_windows`이면 `autostart::set(config.start_with_windows)` (실패는 로그만).
- `TIMER_MAINTENANCE`에서 `menubar.release_if_idle(RENDERER_IDLE)`, 메뉴가 열려 있으면 훅 재설치를 미룬다(`!self.wheel.is_visible() && self.menu.is_none()`).
- `MenuView.rows`는 `ITEMS.iter().map(|i| (i.label(), i.value(&config), save_failed == Some(*i)))`, `notice`는 `config_writable`이 아니면 `Some("설정 파일에 오류가 있어 저장하지 않습니다")`.

- [ ] **Step 1: 구현** — 위 동작대로. 순수 판정은 이미 core에 있으므로 여기서는 배선만.
- [ ] **Step 2: 자동 확인** — `cargo fmt --check && cargo clippy --workspace -- -D warnings && cargo test --workspace` 통과.
- [ ] **Step 3: 실제 실행 확인** — 디버그 빌드 실행 후 직접 확인: 바탕화면 클릭 → Space → 바, ←/→/Enter로 설정 패널, ↓ 두 번 → ←로 Esc 키 행 값이 바뀌는지, `config.toml` 열어 값과 주석 확인, Esc 두 번으로 닫힘, 메모장에서 Space 정상 입력, Alt+Tab으로 닫힘, 플러그인 타일 알림.
- [ ] **Step 4: 커밋** — `git commit -m "feat(app): wire the desktop menu and live settings"`

---

### Task 11: 문서

**Files:**
- Modify: `README.md`, `docs/manual-test.md`, `docs/superpowers/specs/2026-10-20-swisswheel-design.md`

- [ ] **Step 1: README** — "Usage" 아래에 단락 추가: 바탕화면을 클릭한 뒤 **Space**를 누르면 화면 위쪽에 메뉴 바(설정 · 플러그인 · 업데이트)가 열린다. ←/→로 고르고 Enter, Esc로 닫는다. 설정 패널에서 ↑/↓로 항목, ←/→로 값을 바꾸면 바로 저장·적용된다. "Settings live in …" 문장은 "메뉴의 설정 패널이나 `%APPDATA%\SwissWheel\config.toml`에서 바꾼다"로. "What works now"에 설정 패널 한 줄, "Shown … but not yet implemented"에 플러그인·업데이트 타일 추가.
- [ ] **Step 2: 수동 체크리스트** — M23~M32 추가: 스펙 5장 "수동" 목록 항목을 각 한 행(절차 | PASS 조건)으로.
- [ ] **Step 3: 본 설계 문서** — 1장 비목표에서 "설정 GUI" 제거, 그 줄 끝에 "(설정 GUI는 `2026-10-21-desktop-menu-design.md`)" 표기.
- [ ] **Step 4: 커밋** — `git commit -m "docs: describe the desktop menu and settings panel"`
