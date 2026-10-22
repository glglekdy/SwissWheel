# 다운로드 패널 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 다운로드 타일이 FFmpeg를 바로 받는 대신, 필수 앱을 설치·업데이트·제거하는 패널을 연다.

**Architecture:** 상태 → 문구·버튼, 버전 해석·비교, 거절 규칙은 `swisswheel-core`의 새 모듈 `essentials`에 둔다. 패널 안 위치와 키 처리는 `menu::MenuSession`, 상자와 클릭 판정은 `menu_geometry`가 맡는다. 앱 쪽은 상태 확인 스레드(`essentials_status.rs`), 기존 설치기에 제거와 "이 폴더 먼저"를 더한 `ffmpeg_install.rs`, 그리기(`menubar`), 연결(`app.rs`)이다.

**Tech Stack:** Rust 2024, `windows` 0.61 (Direct2D, MessageBoxW), 기존 `update::http`·`update::Ui`·`update::test_server`.

**Spec:** `docs/superpowers/specs/2026-10-23-download-panel-design.md`

## Global Constraints

- 스펙의 `tools.rs`/`Tool`은 `catalog::Tool`과 이름이 겹쳐 **`essentials.rs`/`Essential`**로 쓴다(뜻은 같다). 다른 이름도 표의 것을 쓴다.
- 필수 앱 목록: `ESSENTIALS: [Essential; 1] = [Essential::Ffmpeg]`, 줄당 버튼 최대 2개(`MAX_BUTTONS = 2`).
- 최신 버전은 마지막 **성공** 확인 뒤 1시간(`LATEST_TTL = 3600 s`) 동안 다시 받지 않는다. 실패는 기억하지 않는다. 확인 스레드는 한 번에 하나.
- `ffmpeg -version`은 `CREATE_NO_WINDOW`로 실행하고 5초가 넘으면 버린다(버전 `None`).
- 제거는 `install_dirs()` 안의 `ffmpeg.exe` → `ffprobe.exe` → `LICENSE`만, 빈 폴더는 폴더도. PATH의 FFmpeg는 절대 건드리지 않는다.
- 업데이트·제거는 변환 중이면 거절, 설치·전용 설치는 변환 중에도 된다. 다른 업데이트·다운로드·제거·교체 대기 중이면 모두 거절.
- 확인 창은 UI 스레드에서 띄우지 않는다(지금 규칙).
- 새 메시지: `WM_APP_ESSENTIALS = WM_APP + 33`(상태 확인 끝), `WM_APP_FFMPEG_REMOVE = WM_APP + 34`(제거 끝).
- 버튼 상자: 폭 88 DIP, 간격 8 DIP, 줄 오른쪽 끝부터.
- 로그·알림·UI 문구는 한국어, 코드 주석은 영어(기존 파일과 같게).
- Task 4·5에서 만든 앱 쪽 함수(`remove`, `spawn_remove`, `forget_tools`, `essentials_status::spawn`)는 Task 6에서 처음 쓰인다. 그때까지 각 항목에 `#[allow(dead_code)] // wired in Task 6`을 달고, Task 6에서 모두 지운다.
- 모든 작업 뒤 `cargo test --workspace`와 `cargo clippy --workspace --all-targets -- -D warnings`가 통과해야 한다.

**문구 (스펙 2.2–2.4 그대로):**

| 이름 | 문구 |
|---|---|
| `Essential::Ffmpeg.label()` / `.purpose()` | FFmpeg / 영상·오디오 |
| `essentials::header(false)` / `header(true)` | 필수 앱 / 필수 앱 · 확인 중… |
| 상태: 모름 · 없음 | 확인 중… · 설치 안 됨 |
| 상태: 시스템 | 시스템 {v} 사용 중 (버전 모름: 시스템 FFmpeg 사용 중) |
| 상태: 전용 | {v} · {v} · 최신 · {v} · {최신} 있음 · {v} · 확인 실패 · 버전 알 수 없음 · {v} · 최신 {최신} |
| 상태: 받는 중 | 받는 중 {n}% (`n = (fraction*100).round()`) |
| `EssentialAction` 라벨 | 설치 · 전용 설치 · 업데이트 · 제거 |
| `essentials::remove_prompt(Some("9.0.2"), dir, true)` | SwissWheel이 받은 FFmpeg 9.0.2를 지울까요?\n\n위치: {dir}\n\n지운 뒤에는 시스템 FFmpeg를 씁니다 |
| `remove_prompt(None, dir, false)` | SwissWheel이 받은 FFmpeg를 지울까요?\n\n위치: {dir} |
| `essentials::text::REMOVED` / `REMOVE_FAILED` | FFmpeg를 지웠습니다 / FFmpeg를 지우지 못했습니다 |
| 거절 | `ffmpeg_release::text::RUNNING`, `update::text::BUSY` (기존 상수) |

## Review Focus

1. 패널을 연 채 설치·업데이트가 끝남 → 앱 줄이 다시 열지 않아도 새 버전·버튼으로 바뀌어야 한다(Task 6: `on_ffmpeg_done` 뒤 상태 재확인 + `refresh_download_panel`; 수동 M80).
2. 상태 확인 결과가 메뉴가 닫혔거나 설정 패널이 열린 뒤에 도착 → 상태만 저장, 설정 패널 선택은 그대로(Task 2 `set_tool_actions_leaves_settings_alone`).
3. 버튼 수가 확인 결과로 줄거나 사라짐(업데이트 있음 → 최신) → 선택이 범위 밖을 가리키지 않아야 한다(Task 2 `actions_shrink_pulls_the_button_back`).
4. 네트워크가 끊긴 채 패널을 여러 번 엶 → 알림이 뜨지 않고, 열 때마다 다시 확인한다(Task 6 `latest_fetch_due` 시험).
5. 한 설치 폴더에 사용자가 둔 다른 파일 → 제거 뒤에도 남고 폴더도 남는다(Task 4 `remove_keeps_other_files`).

---

### Task 1: `essentials` 핵심 규칙 (core)

**Files:**
- Create: `crates/swisswheel-core/src/essentials.rs`
- Modify: `crates/swisswheel-core/src/lib.rs` (`pub mod essentials;`)

**Interfaces:**
- Produces:
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum Essential { Ffmpeg }`, `pub const ESSENTIALS: [Essential; 1]`, `pub const MAX_BUTTONS: usize = 2`, `Essential::label(self) -> &'static str`, `Essential::purpose(self) -> &'static str`
  - `#[derive(Debug, Clone, PartialEq, Eq)] pub enum Installed { None, System(Option<String>), Own(Option<String>) }`
  - `#[derive(Debug, Clone, PartialEq, Eq)] pub enum Latest { Unknown, Checking, Found(String), Failed }`
  - `#[derive(Debug, Clone, PartialEq, Default)] pub struct EssentialState { pub installed: Option<Installed>, pub latest: Latest, pub busy: Option<f32> }` (`Latest: Default = Unknown`)
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum EssentialAction { Install, InstallOwn, Update, Remove }`, `EssentialAction::label(self) -> &'static str`
  - `pub fn view(state: &EssentialState) -> (String, Vec<EssentialAction>)`
  - `pub fn header(checking: bool) -> String`
  - `pub fn parse_ffmpeg_version(first_line: &str) -> Option<String>`
  - `pub fn is_newer(latest: &str, installed: &str) -> Option<bool>`
  - `pub fn on_action(action: EssentialAction, jobs_active: bool, background_busy: bool) -> update::TileDecision`
  - `pub fn remove_prompt(version: Option<&str>, dir: &Path, system: bool) -> String`
  - `pub mod text { pub const REMOVED: &str; pub const REMOVE_FAILED: &str; }`

- [ ] **Step 1: 실패하는 시험 쓰기** (`essentials.rs`의 `mod tests`)

```rust
fn own(v: &str, latest: Latest) -> EssentialState {
    EssentialState { installed: Some(Installed::Own(Some(v.into()))), latest, busy: None }
}
use EssentialAction::*;

#[test]
fn view_follows_the_spec_table() {
    let s = |installed, latest| EssentialState { installed, latest, busy: None };
    assert_eq!(view(&s(None, Latest::Checking)), ("확인 중…".into(), vec![]));
    assert_eq!(view(&s(Some(Installed::None), Latest::Failed)), ("설치 안 됨".into(), vec![Install]));
    assert_eq!(view(&s(Some(Installed::System(Some("8.0".into()))), Latest::Failed)), ("시스템 8.0 사용 중".into(), vec![InstallOwn]));
    assert_eq!(view(&s(Some(Installed::System(None)), Latest::Unknown)), ("시스템 FFmpeg 사용 중".into(), vec![InstallOwn]));
    assert_eq!(view(&own("9.0.2", Latest::Checking)), ("9.0.2".into(), vec![Remove]));
    assert_eq!(view(&own("9.0.2", Latest::Found("9.0.2".into()))), ("9.0.2 · 최신".into(), vec![Remove]));
    assert_eq!(view(&own("9.0.2", Latest::Found("9.0.3".into()))), ("9.0.2 · 9.0.3 있음".into(), vec![Update, Remove]));
    assert_eq!(view(&own("9.0.2", Latest::Failed)), ("9.0.2 · 확인 실패".into(), vec![Remove]));
    assert_eq!(view(&s(Some(Installed::Own(None)), Latest::Found("9.0.3".into()))), ("버전 알 수 없음".into(), vec![Update, Remove]));
    assert_eq!(view(&own("9.0.2", Latest::Found("2025-01".into()))), ("9.0.2 · 최신 2025-01".into(), vec![Update, Remove]));
    let busy = EssentialState { busy: Some(0.42), ..own("9.0.2", Latest::Found("9.0.3".into())) };
    assert_eq!(view(&busy), ("받는 중 42%".into(), vec![]));
}

#[test]
fn ffmpeg_version_lines() {
    let p = parse_ffmpeg_version;
    assert_eq!(p("ffmpeg version 9.0.2-essentials_build-www.gyan.dev Copyright (c) 2000-2026"), Some("9.0.2".into()));
    assert_eq!(p("ffmpeg version n7.1-3-gabc Copyright"), Some("7.1".into()));
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
    use crate::update::TileDecision::*;
    assert_eq!(on_action(Install, false, true), Refuse(crate::ffmpeg_release::text::RUNNING));
    assert_eq!(on_action(Remove, true, true), Refuse(crate::ffmpeg_release::text::RUNNING));
    assert_eq!(on_action(Remove, true, false), Refuse(crate::update::text::BUSY));
    assert_eq!(on_action(Update, true, false), Refuse(crate::update::text::BUSY));
    assert_eq!(on_action(Install, true, false), Start);
    assert_eq!(on_action(InstallOwn, true, false), Start);
    assert_eq!(on_action(Remove, false, false), Start);
}

#[test]
fn texts() {
    assert_eq!((header(false), header(true)), ("필수 앱".into(), "필수 앱 · 확인 중…".into()));
    let dir = Path::new(r"C:\SW\ffmpeg");
    assert_eq!(remove_prompt(Some("9.0.2"), dir, true),
        "SwissWheel이 받은 FFmpeg 9.0.2를 지울까요?\n\n위치: C:\\SW\\ffmpeg\n\n지운 뒤에는 시스템 FFmpeg를 씁니다");
    assert_eq!(remove_prompt(None, dir, false), "SwissWheel이 받은 FFmpeg를 지울까요?\n\n위치: C:\\SW\\ffmpeg");
    assert_eq!([Install, InstallOwn, Update, Remove].map(EssentialAction::label), ["설치", "전용 설치", "업데이트", "제거"]);
    assert!(ESSENTIALS.len() <= crate::settings::MAX_ROWS);
}
```

- [ ] **Step 2: 실패 확인** — `cargo test -p swisswheel-core essentials` → 컴파일 실패(정의 없음).
- [ ] **Step 3: 구현.** `parse_ffmpeg_version`: `"ffmpeg version "` 뒤 첫 단어를 `-`에서 자르고, 앞의 `n` 하나를 떼고, 점으로 나눈 조각이 모두 숫자일 때만 `Some`. `is_newer`: 점으로 나눈 `u32` 목록을 짧은 쪽에 0을 채워 사전식 비교. `view`: `busy`가 가장 먼저, 그다음 `installed`(표 순서).
- [ ] **Step 4: 통과 확인** — `cargo test -p swisswheel-core essentials` → 5 passed.
- [ ] **Step 5: 커밋** — `feat(core): essentials state, versions, and refusal rules`

---

### Task 2: 메뉴의 다운로드 패널 위치와 키 (core)

**Files:**
- Modify: `crates/swisswheel-core/src/menu.rs`
- Modify: `crates/swisswheel/src/app.rs` (`on_menu_action`의 새 변형만: 아래 Step 3)

**Interfaces:**
- Consumes: Task 1 `Essential`, `ESSENTIALS`, `EssentialAction`.
- Produces:
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum DownloadFocus { Header, Row { essential: usize, button: Option<usize> } }`
  - `MenuSession::download_focus(&self) -> Option<DownloadFocus>`, `MenuSession::panel_open(&self) -> bool` (설정 또는 다운로드)
  - `MenuSession::set_actions(&mut self, actions: Vec<Vec<EssentialAction>>)` (줄마다 버튼), `MenuSession::actions(&self) -> &[Vec<EssentialAction>]`
  - `MenuAction::OpenDownloads` (다운로드 패널이 막 열림), `MenuAction::Essential(Essential, EssentialAction)`
  - `MenuHit::EssentialRow(usize)`, `MenuHit::EssentialButton(usize, usize)`
  - 다운로드 타일은 더 이상 `MenuAction::Notice(Tile::Download)`를 내지 않는다.

- [ ] **Step 1: 실패하는 시험 쓰기** (`menu.rs` `mod tests`; 기존 `enter_or_down_runs_the_tile`의 다운로드 줄을 `OpenDownloads`로 고친다)

```rust
fn downloads(actions: Vec<Vec<EssentialAction>>) -> MenuSession {
    let mut m = MenuSession::new();
    m.key(MenuKey::Right);
    assert_eq!(m.key(MenuKey::Enter), MenuAction::OpenDownloads);
    m.set_actions(actions);
    m
}
use EssentialAction::*;

#[test]
fn download_tile_opens_its_panel_on_the_header() {
    let mut m = downloads(vec![vec![Install]]);
    assert_eq!(m.download_focus(), Some(DownloadFocus::Header));
    assert!(m.panel_open() && m.focus().is_none());
    assert_eq!(m.key(MenuKey::Enter), MenuAction::Redraw); // header → row
    let mut n = MenuSession::new();
    n.key(MenuKey::Right);
    assert_eq!(n.key(MenuKey::Down), MenuAction::OpenDownloads);
    assert_eq!(n.click(MenuHit::Tile(1)), MenuAction::None); // already open
}

#[test]
fn rows_and_buttons_move_and_run() {
    let mut m = downloads(vec![vec![Update, Remove]]);
    m.key(MenuKey::Down);
    assert_eq!(m.download_focus(), Some(DownloadFocus::Row { essential: 0, button: Some(0) }));
    assert_eq!(m.key(MenuKey::Down), MenuAction::None); // one row
    assert_eq!(m.key(MenuKey::Left), MenuAction::None);
    assert_eq!(m.key(MenuKey::Right), MenuAction::Redraw);
    assert_eq!(m.key(MenuKey::Right), MenuAction::None);
    assert_eq!(m.key(MenuKey::Enter), MenuAction::Essential(Essential::Ffmpeg, Remove));
    assert_eq!(m.key(MenuKey::Escape), MenuAction::Redraw);
    assert_eq!(m.download_focus(), Some(DownloadFocus::Header));
    assert_eq!(m.key(MenuKey::Up), MenuAction::Redraw);
    assert!(!m.panel_open());
}

#[test]
fn actions_shrink_pulls_the_button_back() {
    let mut m = downloads(vec![vec![Update, Remove]]);
    m.key(MenuKey::Down);
    m.key(MenuKey::Right);
    m.set_actions(vec![vec![Remove]]);
    assert_eq!(m.download_focus(), Some(DownloadFocus::Row { essential: 0, button: Some(0) }));
    m.set_actions(vec![vec![]]);
    assert_eq!(m.download_focus(), Some(DownloadFocus::Row { essential: 0, button: None }));
    assert_eq!(m.key(MenuKey::Enter), MenuAction::None);
    m.set_actions(vec![vec![Install]]); // buttons came back: the first is picked
    assert_eq!(m.download_focus(), Some(DownloadFocus::Row { essential: 0, button: Some(0) }));
}

#[test]
fn set_tool_actions_leaves_settings_alone() {
    let mut m = panel(); // the settings panel
    m.set_actions(vec![vec![Install]]);
    assert_eq!(m.focus(), Some(Focus::Tabs));
    assert_eq!(m.download_focus(), None);
}

#[test]
fn moving_to_another_tile_closes_the_download_panel() {
    let mut m = downloads(vec![vec![Install]]);
    m.key(MenuKey::Down);
    m.hover(MenuHit::Tile(0));
    assert_eq!(m.download_focus(), None);
    assert!(!m.panel_open());
}

#[test]
fn clicks_on_rows_and_buttons() {
    let mut m = downloads(vec![vec![Update, Remove]]);
    assert_eq!(m.click(MenuHit::EssentialRow(0)), MenuAction::Redraw);
    assert_eq!(m.download_focus(), Some(DownloadFocus::Row { essential: 0, button: Some(0) }));
    assert_eq!(m.click(MenuHit::EssentialButton(0, 1)), MenuAction::Essential(Essential::Ffmpeg, Remove));
    assert_eq!(m.click(MenuHit::EssentialButton(0, 5)), MenuAction::None);
    assert_eq!(m.click(MenuHit::Outside), MenuAction::Close);
}
```

- [ ] **Step 2: 실패 확인** — `cargo test -p swisswheel-core menu::` → 컴파일 실패.
- [ ] **Step 3: 구현.** `MenuSession`에 `download: Option<DownloadFocus>`, `actions: Vec<Vec<EssentialAction>>`(처음엔 `ESSENTIALS` 수만큼 빈 목록). `activate()`의 `Tile::Download`: 이미 열렸으면 `None`, 아니면 `Header`로 열고 `OpenDownloads`. `select_tile`은 다운로드가 아닌 타일로 가면 `download = None`. `key`는 `self.download`가 `Some`일 때 스펙 2.3 표대로 먼저 처리한다(`Open`은 그대로 `Close`). 줄에 들어가거나 클릭으로 줄을 고르면 버튼은 `Some(0)`(버튼이 없으면 `None`). `set_actions`는 스펙 2.3 "선택 당김"대로 맞춘다. `scroll`은 다운로드 패널에서 `None`. `app.rs`의 `on_menu_action`에는 일단 `MenuAction::OpenDownloads => self.render_menu()`, `MenuAction::Essential(..) => {}`를 더한다(Task 6에서 채움).
- [ ] **Step 4: 통과 확인** — `cargo test --workspace` 통과, clippy 통과.
- [ ] **Step 5: 커밋** — `feat(menu): download panel focus, keys, and clicks`

---

### Task 3: 다운로드 패널 배치와 클릭 판정 (core)

**Files:**
- Modify: `crates/swisswheel-core/src/menu_geometry.rs`

**Interfaces:**
- Consumes: Task 1 `MAX_BUTTONS`; Task 2 `MenuHit::EssentialRow/EssentialButton`.
- Produces:
  - `pub const BUTTON: f32 = 88.0; pub const BUTTON_GAP: f32 = 8.0;`
  - `MenuLayout.download_header: Box2` (설정의 탭 줄과 같은 줄)
  - `MenuLayout.button_slots: Vec<[Box2; MAX_BUTTONS]>` (`MAX_ROWS`줄, `[0]`이 오른쪽 끝)
  - `pub fn essential_button(layout: &MenuLayout, row: usize, count: usize, button: usize) -> Option<Box2>` (`count`개 중 왼쪽부터 `button`번째 = 슬롯 `count-1-button`)
  - `pub fn hit_download(layout: &MenuLayout, buttons: Option<&[usize]>, x: f32, y: f32) -> MenuHit` (`buttons[i]` = i번째 줄 버튼 수, `None` = 패널 닫힘. 타일·바 판정은 `hit`과 같은 코드를 나눠 쓴다)

- [ ] **Step 1: 실패하는 시험 쓰기**

```rust
#[test]
fn buttons_fill_rows_from_the_right() {
    let l = layout(false, 1.0);
    let row = l.rows[0].row;
    let only = essential_button(&l, 0, 1, 0).unwrap();
    assert_eq!((only.right, only.right - only.left), (row.right, BUTTON));
    let (a, b) = (essential_button(&l, 0, 2, 0).unwrap(), essential_button(&l, 0, 2, 1).unwrap());
    assert_eq!(b, only);
    assert_eq!(a.right, b.left - BUTTON_GAP);
    assert_eq!((a.top, a.bottom), (row.top, row.bottom));
    assert!(essential_button(&l, 0, 2, 2).is_none());
    assert!(essential_button(&l, 0, 3, 0).is_none()); // more than MAX_BUTTONS
    let header = Box2 { left: l.panel.left + PANEL_PAD, right: l.panel.right - PANEL_PAD, ..l.tabs[0] };
    assert_eq!(l.download_header, header);
}

#[test]
fn hit_on_the_download_panel() {
    let l = layout(false, 1.25);
    let mid = |b: Box2| ((b.left + b.right) / 2.0, (b.top + b.bottom) / 2.0);
    let (x, y) = mid(essential_button(&l, 0, 2, 1).unwrap());
    assert_eq!(hit_download(&l, Some(&[2]), x, y), MenuHit::EssentialButton(0, 1));
    assert_eq!(hit_download(&l, Some(&[1]), x, y), MenuHit::EssentialButton(0, 0));
    let (x, y) = (l.rows[0].row.left + 4.0, mid(l.rows[0].row).1);
    assert_eq!(hit_download(&l, Some(&[1]), x, y), MenuHit::EssentialRow(0));
    let (x, y) = mid(l.rows[1].row); // no second essential
    assert_eq!(hit_download(&l, Some(&[1]), x, y), MenuHit::Panel);
    let (x, y) = mid(l.tiles[1]);
    assert_eq!(hit_download(&l, Some(&[1]), x, y), MenuHit::Tile(1));
    let (x, y) = mid(l.rows[0].row);
    assert_eq!(hit_download(&l, None, x, y), MenuHit::Outside);
}
```

- [ ] **Step 2: 실패 확인** — `cargo test -p swisswheel-core menu_geometry` → 컴파일 실패.
- [ ] **Step 3: 구현** — `layout()`에서 `line(first)`를 `download_header`로, 각 줄의 버튼 슬롯을 `row.right`부터 왼쪽으로 `BUTTON`·`BUTTON_GAP`(DIP × scale)씩.
- [ ] **Step 4: 통과 확인** — `cargo test --workspace`, clippy.
- [ ] **Step 5: 커밋** — `feat(menu): download panel layout and hit testing`

---

### Task 4: 제거, "이 폴더 먼저", 경로 잊기 (앱)

**Files:**
- Modify: `crates/swisswheel/src/ffmpeg_install.rs`
- Modify: `crates/swisswheel/src/jobs.rs`

**Interfaces:**
- Produces:
  - `#[derive(Debug, PartialEq)] pub enum RemoveOutcome { Removed, Failed, Declined }`
  - `pub fn remove(dirs: &[PathBuf], prompt: &str, ui: &dyn Ui) -> RemoveOutcome` (막는다; 확인 뒤 각 폴더에서 세 파일, 비면 폴더)
  - `pub fn spawn_remove(hwnd: isize, msg: u32, slot: Arc<Mutex<Option<RemoveOutcome>>>, prompt: String)` (`install_dirs()` 대상, 제목 `DIALOG_TITLE`)
  - `pub fn spawn(hwnd: isize, msg: u32, slot: Arc<Mutex<Option<Outcome>>>, prefer: Option<PathBuf>) -> Handle` (인자 추가: `prefer`를 `install_dirs()` 맨 앞으로 옮겨 `run`에 넘김 — 스펙 3.1의 `target`을 이 방식으로 한다)
  - `fn preferring(dirs: Vec<PathBuf>, prefer: Option<&Path>) -> Vec<PathBuf>` (같은 경로는 한 번만)
  - `Runner::forget_tools(&self)` (`jobs.rs`: 기억한 `Tools`를 `None`으로)

- [ ] **Step 1: 실패하는 시험 쓰기** (`ffmpeg_install.rs` 기존 `FakeUi`·`names` 도우미 재사용)

```rust
fn installed_in(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    for f in ["ffmpeg.exe", "ffprobe.exe", "LICENSE"] { fs::write(dir.join(f), b"x").unwrap(); }
}

#[test]
fn remove_clears_both_install_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let dirs = vec![tmp.path().join("앱").join("ffmpeg"), tmp.path().join("로컬").join("ffmpeg")];
    dirs.iter().for_each(|d| installed_in(d));
    let ui = FakeUi::new(true);
    assert_eq!(remove(&dirs, "지울까요?", &ui), RemoveOutcome::Removed);
    assert_eq!(*ui.confirms.borrow(), ["지울까요?"]);
    assert!(dirs.iter().all(|d| !d.exists()));
}

#[test]
fn remove_keeps_other_files() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("ffmpeg");
    installed_in(&dir);
    fs::write(dir.join("내 메모.txt"), b"keep").unwrap();
    assert_eq!(remove(&[dir.clone()], "p", &FakeUi::new(true)), RemoveOutcome::Removed);
    assert_eq!(names(&dir), ["내 메모.txt"]);
}

#[test]
fn remove_declined_or_with_nothing_there() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("ffmpeg");
    installed_in(&dir);
    assert_eq!(remove(&[dir.clone()], "p", &FakeUi::new(false)), RemoveOutcome::Declined);
    assert_eq!(names(&dir), ["LICENSE", "ffmpeg.exe", "ffprobe.exe"]);
    assert_eq!(remove(&[tmp.path().join("없음")], "p", &FakeUi::new(true)), RemoveOutcome::Removed);
}

#[test]
fn remove_reports_a_file_it_cannot_delete() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("ffmpeg");
    installed_in(&dir);
    use std::os::windows::fs::OpenOptionsExt;
    // std shares delete by default; share nothing, as a running exe would.
    let _open = fs::OpenOptions::new().read(true).share_mode(0).open(dir.join("ffprobe.exe")).unwrap();
    assert_eq!(remove(&[dir.clone()], "p", &FakeUi::new(true)), RemoveOutcome::Failed);
}

#[test]
fn update_goes_to_the_preferred_dir() {
    let (a, b) = (PathBuf::from(r"C:\a\ffmpeg"), PathBuf::from(r"C:\b\ffmpeg"));
    assert_eq!(preferring(vec![a.clone(), b.clone()], Some(&b)), [b.clone(), a.clone()]);
    assert_eq!(preferring(vec![a.clone(), b.clone()], None), [a.clone(), b.clone()]);
}
```

`jobs.rs` 시험 `forgotten_ffmpeg_is_looked_up_again`: 기존 `ffmpeg_installed_after_start_is_used_without_restart`처럼 `with_locator`에 `AtomicBool`로 켜고 끄는 찾기 함수를 넘긴다. 켠 채 한 번 성공 → 끔 → `forget_tools()` → 다음 작업의 `failures == ["clip.mp4: FFmpeg를 찾을 수 없습니다"]`.

- [ ] **Step 2: 실패 확인** — `cargo test -p swisswheel ffmpeg_install jobs::tests::forgotten` → 컴파일 실패.
- [ ] **Step 3: 구현.** `remove`: 확인 거절 → `Declined`. 각 폴더에서 세 이름을 순서대로 지우고(`NotFound`는 성공으로 침), 다 지운 폴더는 `fs::remove_dir`(비어 있지 않으면 조용히 둔다). 한 파일이라도 다른 오류면 끝까지 시도한 뒤 `Failed`. 로그는 기존 "FFmpeg: …" 형식. `app.rs`의 기존 `spawn` 호출에는 `None`을 넘긴다.
- [ ] **Step 4: 통과 확인** — `cargo test --workspace`, clippy.
- [ ] **Step 5: 커밋** — `feat(ffmpeg): remove the app's own copy; update in place; forget the cached path`

---

### Task 5: 상태 확인 스레드 (앱)

**Files:**
- Create: `crates/swisswheel/src/essentials_status.rs`
- Modify: `crates/swisswheel/src/main.rs` (`mod essentials_status;`)

**Interfaces:**
- Consumes: Task 1 `Installed`, `parse_ffmpeg_version`; `engines::ffmpeg::locate_in`, `update::http::get`, `ffmpeg_release::parse_release`, `ffmpeg_install::endpoint`.
- Produces:
  - `#[derive(Debug, Clone, PartialEq)] pub struct Report { pub installed: Installed, pub own_dir: Option<PathBuf>, pub system: bool, pub latest: Option<Result<String, ()>> }` (`latest: None` = 받지 않음)
  - `pub fn check(dirs: &[PathBuf], path_var: Option<&OsStr>, endpoint: Option<&str>, read_version: &dyn Fn(&Path) -> Option<String>) -> Report`
  - `pub fn read_version(ffmpeg: &Path) -> Option<String>` (`-version`, 첫 줄, 5초)
  - `pub fn spawn(hwnd: isize, msg: u32, slot: Arc<Mutex<Option<Report>>>, fetch_latest: bool)` (`install_dirs()`, `PATH`, `endpoint()`)

- [ ] **Step 1: 실패하는 시험 쓰기**

```rust
fn tools_in(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    for f in ["ffmpeg.exe", "ffprobe.exe"] { fs::write(dir.join(f), b"").unwrap(); }
}
fn ver(v: &'static str) -> impl Fn(&Path) -> Option<String> { move |_| Some(v.into()) }

#[test]
fn own_copy_wins_over_path() {
    let tmp = tempfile::tempdir().unwrap();
    let (own, sys) = (tmp.path().join("own"), tmp.path().join("sys"));
    tools_in(&own); tools_in(&sys);
    let r = check(&[own.clone()], Some(sys.as_os_str()), None, &ver("9.0.2"));
    assert_eq!(r, Report { installed: Installed::Own(Some("9.0.2".into())), own_dir: Some(own), system: true, latest: None });
}

#[test]
fn system_only_and_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let sys = tmp.path().join("sys");
    tools_in(&sys);
    let r = check(&[tmp.path().join("own")], Some(sys.as_os_str()), None, &|_| None);
    assert_eq!((r.installed, r.own_dir, r.system), (Installed::System(None), None, true));
    let r = check(&[tmp.path().join("own")], None, None, &ver("x"));
    assert_eq!((r.installed, r.system), (Installed::None, false));
}

#[test]
fn latest_from_the_release_endpoint() {
    let json = br#"{"tag_name":"9.0.3","assets":[{"name":"ffmpeg-9.0.3-essentials_build.zip","size":1,"browser_download_url":"http://x/z.zip","digest":"sha256:0000000000000000000000000000000000000000000000000000000000000000"}]}"#;
    let server = TestServer::start(vec![("/latest", Reply::Ok(json.to_vec())), ("/bad", Reply::Status(500))]);
    let url = |p: &str| format!("{}{p}", server.base);
    let r = check(&[], None, Some(&url("/latest")), &|_| None);
    assert_eq!(r.latest, Some(Ok("9.0.3".into())));
    let r = check(&[], None, Some(&url("/bad")), &|_| None);
    assert_eq!(r.latest, Some(Err(())));
}

#[test]
fn reading_a_version_from_a_missing_exe_is_none() {
    assert_eq!(read_version(Path::new(r"C:\없음\ffmpeg.exe")), None);
}
```

(가짜 릴리스 JSON은 기존 `ffmpeg_release` 시험의 모양을 그대로 쓴다.)

- [ ] **Step 2: 실패 확인** — `cargo test -p swisswheel essentials_status` → 컴파일 실패.
- [ ] **Step 3: 구현.** `check`: `locate_in(dirs, None)` → `Own`, 아니면 `locate_in(&[], path_var)` → `System`, 아니면 `None`; `system`은 `locate_in(&[], path_var).is_some()`. `read_version`: `std::process::Command` + `CommandExt::creation_flags(0x0800_0000)`, `stdin/stderr` null, `stdout` piped, `try_wait`를 100ms 간격으로 5초까지, 넘으면 `kill`. 첫 줄을 `parse_ffmpeg_version`에. `spawn`은 `ffmpeg_install::spawn`과 같은 모양(슬롯에 넣고 `PostMessageW`).
- [ ] **Step 4: 통과 확인** — `cargo test --workspace`, clippy.
- [ ] **Step 5: 커밋** — `feat(app): check the installed and latest FFmpeg in the background`

---

### Task 6: 그리기와 연결, 문서 (앱)

**Files:**
- Modify: `crates/swisswheel/src/menubar/mod.rs`, `crates/swisswheel/src/menubar/render.rs`
- Modify: `crates/swisswheel/src/app.rs`
- Modify: `crates/swisswheel-core/src/ffmpeg_release.rs` (`on_download_tile`, `text::READY`와 그 시험 삭제)
- Modify: `README.md`, `docs/manual-test.md`, `docs/superpowers/specs/2026-10-21-desktop-menu-design.md`, `docs/superpowers/specs/2026-10-22-ffmpeg-download-restart-design.md`

**Interfaces:**
- Consumes: Task 1–5 전부.
- Produces (menubar):
  - `pub struct EssentialRowView { pub label: &'static str, pub purpose: &'static str, pub status: String, pub buttons: Vec<&'static str>, pub progress: Option<f32> }`
  - `pub fn essential_rows(states: &[EssentialState]) -> Vec<EssentialRowView>` (`view` + `label`)
  - `pub struct DownloadView<'a> { pub header: String, pub rows: &'a [EssentialRowView], pub focus: Option<DownloadFocus> }`, `MenuView.download: Option<DownloadView<'a>>`
- Produces (app): `fn background_busy(update: bool, ffmpeg: bool, removing: bool, pending_swap: bool) -> bool`, `fn latest_fetch_due(last_success: Option<Instant>, now: Instant) -> bool`

- [ ] **Step 1: 실패하는 시험 쓰기**

```rust
// menubar/mod.rs
#[test]
fn essential_rows_carry_the_view() {
    let state = EssentialState { installed: Some(Installed::Own(Some("9.0.2".into()))), latest: Latest::Found("9.0.3".into()), busy: None };
    let rows = essential_rows(&[state]);
    assert_eq!((rows[0].label, rows[0].purpose, rows[0].status.as_str()), ("FFmpeg", "영상·오디오", "9.0.2 · 9.0.3 있음"));
    assert_eq!(rows[0].buttons, ["업데이트", "제거"]);
    assert_eq!(rows[0].progress, None);
}

// app.rs (기존 a_waiting_update_keeps_downloads_and_restarts_off 고침 + 새 시험)
#[test]
fn a_removal_keeps_other_background_work_off() {
    assert!(background_busy(false, false, true, false));
    assert!(!background_busy(false, false, false, false));
}

#[test]
fn latest_is_fetched_again_after_an_hour_or_a_failure() {
    let now = Instant::now();
    assert!(latest_fetch_due(None, now));
    assert!(!latest_fetch_due(Some(now - Duration::from_secs(3599)), now));
    assert!(latest_fetch_due(Some(now - Duration::from_secs(3600)), now));
}
```

- [ ] **Step 2: 실패 확인** — `cargo test -p swisswheel` → 컴파일 실패.
- [ ] **Step 3: 그리기.** `draw`는 `view.rows`나 `view.download`가 있으면 패널 유리를 켠다. `draw_download_panel`: 머리줄은 `notice` 아래 `layout.download_header`에 `small` 서식·`subtext` 색. 앱 줄은 설정 줄처럼(선택 줄 `Piece::Selected`, 마우스 줄 `theme.line`), 왼쪽 `label`(`name`)·뒤에 `purpose`(`small`·`subtext`), 버튼 왼쪽까지 `status`(`small`), 버튼은 `essential_button` 상자에 `Piece::Keycap`, 선택된 버튼은 탭의 `FOCUS_RIM` 강조 테두리, 마우스 버튼은 `text` 색 라벨. `progress`가 있으면 상태 글자 아래 2 DIP 높이 막대(`theme.line` 바탕, `theme.accent` 채움).
- [ ] **Step 4: 연결 (`app.rs`).**
  - 필드: `essentials: EssentialState`, `own_dir: Option<PathBuf>`, `system_ffmpeg: bool`, `latest_ok_at: Option<Instant>`, `checking: bool`, `status_slot: Arc<Mutex<Option<essentials_status::Report>>>`, `removing: bool`, `remove_slot: Arc<Mutex<Option<RemoveOutcome>>>`.
  - `start_essentials_check()`: `checking`이면 끝. `fetch = latest_fetch_due(latest_ok_at, now)`; `fetch`면 `essentials.latest = Latest::Checking`. `checking = true`, `essentials_status::spawn(..., fetch)`.
  - `WM_APP_ESSENTIALS` → `on_essentials_checked()`: `checking = false`, 보고서로 `installed`·`own_dir`·`system_ffmpeg`, `latest`가 `Some(Ok(tag))`면 `Found`+`latest_ok_at = now`, `Some(Err)`면 `Failed`+`latest_ok_at = None`, `None`이면 그대로. 끝에 `refresh_download_panel()`.
  - `refresh_download_panel()`: `essentials.busy` = FFmpeg 받기 중이면 `Some(fraction)`(태그 전이면 `Some(0.0)`), 아니면 `None`. 메뉴가 열려 있으면 `menu.set_actions(vec![view(&essentials).1])`, 다운로드 패널이면 `render_menu()`.
  - `on_menu_action`: `OpenDownloads` → `refresh_download_panel()`, `start_essentials_check()`, `render_menu()`. `Essential(Essential::Ffmpeg, a)` → `close_menu()`, `on_action(a, active_jobs > 0, self.background_busy())`; 거절은 `info(ffmpeg_release::TITLE, notice)`; 시작은 `Install | InstallOwn` → `start_install(None)`, `Update` → `start_install(own_dir.clone())`, `Remove` → `start_remove()`. 옛 `Notice(Tile::Download)` 팔과 `start_ffmpeg`는 지운다(디버그 미리보기 `"ffmpeg"`는 `start_install(None)`).
  - `start_remove()`: `own_dir` 없으면 `warning_notice(TITLE, text::REMOVE_FAILED)`. 있으면 `remove_prompt(own 버전, &own_dir, system_ffmpeg)`로 `spawn_remove`, `removing = true`. `WM_APP_FFMPEG_REMOVE` → `removing = false`, `Removed` → `info(.., REMOVED)`, `Failed` → `warning_notice(.., REMOVE_FAILED)`, `Declined` → 없음.
  - `on_ffmpeg_done`·제거 끝 모두: `runner.forget_tools()`, `start_essentials_check()`, `refresh_download_panel()`.
  - `update_busy()` 끝에서 다운로드 패널이 열려 있으면 `refresh_download_panel()`(받는 중 % 갱신; `TIMER_PROGRESS`가 이미 돈다).
  - `render_menu()`: 패널 열림은 `menu.panel_open()`. 설정 줄은 `focus()`가 있을 때만, `download`는 `download_focus()`가 있을 때 `DownloadView { header: header(self.checking), rows, focus }`.
  - `on_menu_mouse()`: 다운로드 패널이면 `hit_download(layout, Some(&counts))`(`counts` = `menu.actions()` 각 길이), 아니면 지금 `hit`.
  - `background_busy`에 `removing`을 더하고, 쓰는 곳(`start_update`, `restart`)도 함께.
- [ ] **Step 5: 문서.** README 다운로드 타일 문장 → "**다운로드** (a panel to install, update, or remove the apps SwissWheel needs — FFmpeg for now)". 메뉴 설계 문서 2.2 다운로드 줄 → "다운로드 패널을 연다(2026-10-23 설계)". FFmpeg 설계 문서 머리에 "다운로드 타일 동작은 `2026-10-23-download-panel-design.md`로 바뀜" 한 줄. `manual-test.md`에 M76–M83(스펙 5 수동 목록: 없음 → 설치, 시스템만 → 전용 설치, 최신, 업데이트 있음(가짜 릴리스 서버 9.9.9), 확인 실패(서버 끔), 제거(시스템 있음/없음), 변환 중 제거·업데이트 거절, 받는 중 패널 다시 열기).
- [ ] **Step 6: 통과 확인** — `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo build --release`.
- [ ] **Step 7: 커밋** — `feat(app): download panel for installing, updating, and removing FFmpeg`
