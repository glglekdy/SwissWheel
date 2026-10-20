# SwissWheel 셸 + 이미지 변환 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 트레이에 상주하다가 파일 드래그 중 Space로 진짜 휠을 띄우고, 이미지 변환·도구를 실제로 실행해 결과를 알림으로 알려주는 앱을 완성한다 (spec 8장 2·3단계).

**Architecture:** OS 무관 상태 기계(드래그 추적 `DragTracker`, 열린 휠 모델 `WheelSession`, 알림 문구)를 `swisswheel-core`에 추가해 단위 테스트한다. `swisswheel` bin은 Win32 셸(트레이·훅·DirectComposition/Direct2D 휠 창·OLE 드롭 타깃)과 작업 실행기·이미지 엔진을 가진다. 실행기와 엔진은 임시 폴더 기반 테스트로 검증하고, 화면·입력 부분은 수동 체크리스트로 검증한다.

**Tech Stack:** Rust 1.99 (edition 2024), `windows`/`windows-core` 0.61, `image` 0.25, `webp` 0.3, `oxipng` 9 (`default-features = false`), `thiserror` 2, dev: `tempfile` 3.

**Spec:** `docs/superpowers/specs/2026-10-20-swisswheel-design.md` (스파이크 결과: `docs/superpowers/spikes/2026-10-20-drag-spike.md`)

## Global Constraints

- 이 계획은 spec 8장 2·3단계까지만: **영상·오디오 변환/도구, 동작(공유·복사·잘라내기·경로 복사·터미널), 자동 실행, 설치 파일은 다음 계획.** 휠에는 catalog대로 모두 표시하되, 이 계획에서 구현하지 않은 항목에 드롭하면 작업 대신 알림 "다음 버전에서 지원 예정입니다"를 띄운다.
- `swisswheel-core`는 계속 OS 비의존 (의존성: `serde`, `toml`, `thiserror`만).
- 원본은 절대 수정·삭제·덮어쓰기 금지. 결과 파일 확정은 **기존 파일을 대체하지 않는 이동**(`MoveFileExW`에 `MOVEFILE_REPLACE_EXISTING` 없이)으로만 한다. `std::fs::rename`은 대상을 덮어쓰므로 사용 금지.
- 드롭 응답 `pdwEffect`는 항목 위면 `DROPEFFECT_COPY`, 취소 영역이면 `DROPEFFECT_NONE`. 절대 `DROPEFFECT_MOVE` 금지.
- 훅 콜백은 `DragTracker` 호출 + `PostMessageW`만 하고 즉시 반환.
- 휠: 바깥 반지름 160, 취소 원 48 (DIP), Per-Monitor DPI Aware v2, 열림/닫힘/전환 애니메이션 150ms 이내, 강조 확대 120ms, 시스템 다크/라이트 테마 추종.
- 품질 기본값: JPG 90, WebP 85 (config의 `quality`). 도구: 이미지 압축은 JPG 75 / PNG `oxipng` 무손실 / WebP 70, 50% 축소는 Lanczos3.
- 이미지 작업 동시 실행 수: `min(코어 수, 4)`.
- 로그: `%LOCALAPPDATA%\SwissWheel\logs\swisswheel.log`. 설정: `%APPDATA%\SwissWheel\config.toml`.
- UI 문구 한국어. 커밋 메시지 끝에 `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- 매 Task 끝: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` 통과.

## Review Focus

1. **같은 파일을 같은 포맷으로 연달아 두 번 드롭** (첫 작업이 끝나기 전) — 두 결과가 서로 다른 이름(`a.jpg`, `a (1).jpg`)이어야 하고 어느 것도 덮어쓰지 않아야 한다. → Task 7 `concurrent_jobs_get_distinct_outputs`
2. **깨진 파일·0바이트 파일이 섞인 일괄 작업** — 그 파일만 실패로 보고되고 나머지는 처리되며, `.swisswheel-tmp` 임시 파일이 남지 않아야 한다. → Task 7 `bad_input_fails_alone_and_leaves_no_temp`
3. **정사각형이 아닌 큰 이미지 → ICO** (예: 1000×300) — 비율을 유지한 채 투명 여백으로 정사각형을 만들어 모든 크기가 들어가야 한다. → Task 6 `ico_from_wide_image_keeps_aspect`
4. **드래그하지 않을 때의 Space, 파일이 아닌 드래그 중 Space 길게 누르기** — 평소 타이핑의 Space는 절대 삼키지 않고, 시간 초과 뒤 키 반복은 원래 앱으로 통과해야 한다. → Task 2 `space_without_drag_passes_through`, `timeout_suppresses_reopen_until_key_up`
5. **한글·공백이 들어간 폴더와 파일 이름** — 탐지·변환·결과 이름이 모두 정상이어야 한다. → Task 5 `probe_handles_unicode_paths`, Task 7 `batch_in_unicode_folder`

---

### Task 1: 앱 기반 — 의존성, 로그, 설정 파일 입출력

**Files:**
- Modify: `crates/swisswheel/Cargo.toml` (의존성: `windows` 0.61, `windows-core` 0.61, `image` 0.25, `webp` 0.3, `oxipng = { version = "9", default-features = false }`, `thiserror` 2; dev: `tempfile` 3)
- Create: `crates/swisswheel/src/logging.rs`, `crates/swisswheel/src/config_file.rs`
- Modify: `crates/swisswheel/src/main.rs` (`mod logging; mod config_file;`, 맨 위 `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`)
- Modify: `.gitignore` (`spike/**/Cargo.lock` 추가 — 이전 리뷰의 미뤄둔 항목)

`windows` 기능 목록 시작점 (컴파일러가 요구하면 추가): `Win32_Foundation`, `Win32_Graphics_Direct2D`, `Win32_Graphics_Direct2D_Common`, `Win32_Graphics_Direct3D`, `Win32_Graphics_Direct3D11`, `Win32_Graphics_DirectComposition`, `Win32_Graphics_DirectWrite`, `Win32_Graphics_Dxgi`, `Win32_Graphics_Dxgi_Common`, `Win32_Graphics_Gdi`, `Win32_Storage_FileSystem`, `Win32_System_Com`, `Win32_System_Com_StructuredStorage`, `Win32_System_LibraryLoader`, `Win32_System_Ole`, `Win32_System_Registry`, `Win32_System_SystemServices`, `Win32_System_Threading`, `Win32_UI_HiDpi`, `Win32_UI_Input_KeyboardAndMouse`, `Win32_UI_Shell`, `Win32_UI_WindowsAndMessaging`.

**Interfaces:**
- Produces:
  - `logging::init(dir: &Path) -> std::io::Result<()>` — `dir` 생성 후 `swisswheel.log`에 추가 기록. `logging::write(level: &str, msg: &str)`; 매크로 `log_info!`, `log_error!` (`format!` 인자). 각 줄: `YYYY-MM-DD HH:MM:SS.mmm LEVEL msg` (로컬 시각, `GetLocalTime`). 디버그 빌드는 `eprintln!`도.
  - `config_file::load(dir: &Path) -> (Config, Option<String>)` — `dir/config.toml`이 없으면 `default_toml()`로 생성하고 기본값 반환; 파싱/검증 오류면 **파일은 건드리지 않고** 기본값과 경고 문구(`ConfigError`의 Display) 반환.
  - `config_file::app_dirs() -> (PathBuf, PathBuf)` — (`%APPDATA%\SwissWheel`, `%LOCALAPPDATA%\SwissWheel\logs`), 환경 변수 `APPDATA`/`LOCALAPPDATA` 사용.

- [ ] **Step 1: 실패하는 테스트 작성** (`config_file.rs`, `logging.rs` 안 `mod tests`, `tempfile::tempdir()` 사용)

```rust
#[test]
fn missing_file_is_created_with_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let (config, warning) = load(dir.path());
    assert_eq!(config, Config::default());
    assert!(warning.is_none());
    let text = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
    assert_eq!(swisswheel_core::config::parse(&text), Ok(Config::default()));
}

#[test]
fn broken_file_is_kept_and_defaults_used() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[quality]\njpg = 0\n").unwrap();
    let (config, warning) = load(dir.path());
    assert_eq!(config, Config::default());
    assert!(warning.unwrap().contains("quality.jpg"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "[quality]\njpg = 0\n");
}

#[test]
fn valid_file_is_loaded() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.toml"), "open_key = \"CapsLock\"\n").unwrap();
    assert_eq!(load(dir.path()).0.open_key, swisswheel_core::config::Key::CapsLock);
}

// logging.rs
#[test]
fn log_lines_are_appended() {
    let dir = tempfile::tempdir().unwrap();
    init(dir.path()).unwrap();
    write("INFO", "첫 줄");
    write("ERROR", "둘째 줄");
    let text = std::fs::read_to_string(dir.path().join("swisswheel.log")).unwrap();
    let lines: Vec<_> = text.lines().collect();
    assert!(lines[lines.len() - 2].ends_with("INFO 첫 줄"));
    assert!(lines[lines.len() - 1].ends_with("ERROR 둘째 줄"));
}
```

- [ ] **Step 2: 실패 확인** — Run: `cargo test -p swisswheel` / Expected: 컴파일 오류 (`load`, `init` 미정의)
- [ ] **Step 3: 구현** — 로그 파일 핸들은 `OnceLock<Mutex<File>>`. `init`을 여러 번 부르면 마지막 경로로 교체(테스트 격리용).
- [ ] **Step 4: 통과 확인** — Run: `cargo test -p swisswheel` / Expected: 4 passed
- [ ] **Step 5: 커밋** — `feat(app): logging and config file loading`

---

### Task 2: `DragTracker` — 드래그·키 상태 기계 (core)

**Files:**
- Create: `crates/swisswheel-core/src/drag.rs`; Modify: `crates/swisswheel-core/src/lib.rs`

**Interfaces:**
- Produces:
  - `pub struct KeyBindings { pub open: u16, pub cycle: u16, pub cancel: u16 }` + `impl From<&Config> for KeyBindings` (`Key::vk`)
  - `pub enum DragEvent { ButtonDown { x: i32, y: i32 }, Move { x: i32, y: i32 }, ButtonUp, KeyDown(u16), KeyUp(u16), EnterTimedOut, WheelClosed }`
  - `pub enum DragCommand { OpenWheel, Cycle, Cancel, ButtonReleasedWhileOpen }`
  - `pub struct Decision { pub swallow: bool, pub command: Option<DragCommand> }` (`Debug, PartialEq`)
  - `pub struct DragTracker` + `pub fn new(keys: KeyBindings, threshold: (i32, i32)) -> Self` + `pub fn on_event(&mut self, event: DragEvent) -> Decision` + `pub fn wheel_open(&self) -> bool`
  - 모든 enum `Debug, Clone, Copy, PartialEq, Eq`.

**규칙 (spec 2.1, 4.2 + 스파이크):**
- 버튼 상태: `Up` → `ButtonDown` → `Pressed(pos)` → x 또는 y로 임계값 **이상** 이동 → `Dragging`. `ButtonUp` → `Up`.
- 휠 상태: `Closed` / `Open` / `Suppressed`.
- `KeyDown(open)`: `Closed`+`Dragging` → `Open`, 삼킴, `OpenWheel`. `Open` → 삼킴(키 반복). `Suppressed` → 통과. 그 외 → 통과.
- `KeyDown(cycle)`/`KeyDown(cancel)`: `Open`일 때만 삼킴 + `Cycle`/`Cancel`. `Cancel`은 휠을 `Closed`로 (드래그는 계속, 다시 열 수 있음).
- `KeyUp(vk)`: 마지막 `KeyDown(vk)`를 삼켰으면 삼킴, 아니면 통과. `Suppressed`에서 `KeyUp(open)` → `Closed`.
- `ButtonUp`: `Open`이면 `ButtonReleasedWhileOpen` (휠은 앱이 `WheelClosed`를 보낼 때까지 `Open` 유지), `Suppressed`면 `Closed`.
- `EnterTimedOut`: `Open` → `Suppressed`. `WheelClosed`: `Open` → `Closed`.
- 버튼·이동 이벤트는 절대 삼키지 않는다.

- [ ] **Step 1: 실패하는 테스트 작성**

```rust
const SPACE: u16 = 0x20; const TAB: u16 = 0x09; const ESC: u16 = 0x1B; const A: u16 = 0x41;
fn tracker() -> DragTracker { DragTracker::new(KeyBindings { open: SPACE, cycle: TAB, cancel: ESC }, (4, 4)) }
fn pass() -> Decision { Decision { swallow: false, command: None } }
fn dragging() -> DragTracker {
    let mut t = tracker();
    t.on_event(DragEvent::ButtonDown { x: 100, y: 100 });
    t.on_event(DragEvent::Move { x: 104, y: 100 });
    t
}

#[test]
fn space_without_drag_passes_through() {
    let mut t = tracker();
    assert_eq!(t.on_event(DragEvent::KeyDown(SPACE)), pass());
    t.on_event(DragEvent::ButtonDown { x: 0, y: 0 });
    t.on_event(DragEvent::Move { x: 3, y: 3 }); // below threshold
    assert_eq!(t.on_event(DragEvent::KeyDown(SPACE)), pass());
    assert_eq!(t.on_event(DragEvent::KeyUp(SPACE)), pass());
}

#[test]
fn space_while_dragging_opens_and_swallows_pair() {
    let mut t = dragging();
    assert_eq!(t.on_event(DragEvent::KeyDown(SPACE)), Decision { swallow: true, command: Some(DragCommand::OpenWheel) });
    assert!(t.wheel_open());
    assert_eq!(t.on_event(DragEvent::KeyDown(SPACE)), Decision { swallow: true, command: None }); // repeat
    assert_eq!(t.on_event(DragEvent::KeyUp(SPACE)), Decision { swallow: true, command: None });
}

#[test]
fn tab_and_esc_only_while_open() {
    let mut t = dragging();
    assert_eq!(t.on_event(DragEvent::KeyDown(TAB)), pass());
    t.on_event(DragEvent::KeyDown(SPACE));
    assert_eq!(t.on_event(DragEvent::KeyDown(TAB)), Decision { swallow: true, command: Some(DragCommand::Cycle) });
    assert_eq!(t.on_event(DragEvent::KeyUp(TAB)), Decision { swallow: true, command: None });
    assert_eq!(t.on_event(DragEvent::KeyDown(A)), pass());
    assert_eq!(t.on_event(DragEvent::KeyDown(ESC)), Decision { swallow: true, command: Some(DragCommand::Cancel) });
    assert!(!t.wheel_open());
    // still dragging: Space reopens
    t.on_event(DragEvent::KeyUp(SPACE));
    assert_eq!(t.on_event(DragEvent::KeyDown(SPACE)).command, Some(DragCommand::OpenWheel));
}

#[test]
fn timeout_suppresses_reopen_until_key_up() {
    let mut t = dragging();
    t.on_event(DragEvent::KeyDown(SPACE));
    assert_eq!(t.on_event(DragEvent::EnterTimedOut), pass());
    assert!(!t.wheel_open());
    assert_eq!(t.on_event(DragEvent::KeyDown(SPACE)), pass()); // repeat goes to the app
    assert_eq!(t.on_event(DragEvent::KeyUp(SPACE)), pass()); // its down was passed, so the up is too
    assert_eq!(t.on_event(DragEvent::KeyDown(SPACE)).command, Some(DragCommand::OpenWheel));
}

#[test]
fn button_up_while_open_waits_for_wheel_closed() {
    let mut t = dragging();
    t.on_event(DragEvent::KeyDown(SPACE));
    assert_eq!(t.on_event(DragEvent::ButtonUp), Decision { swallow: false, command: Some(DragCommand::ButtonReleasedWhileOpen) });
    assert!(t.wheel_open());
    t.on_event(DragEvent::WheelClosed);
    assert!(!t.wheel_open());
    assert_eq!(t.on_event(DragEvent::KeyDown(SPACE)), pass()); // no longer dragging
}

#[test]
fn button_and_move_never_swallowed() {
    let mut t = dragging();
    t.on_event(DragEvent::KeyDown(SPACE));
    assert!(!t.on_event(DragEvent::Move { x: 300, y: 300 }).swallow);
    assert!(!t.on_event(DragEvent::ButtonDown { x: 1, y: 1 }).swallow);
}
```

- [ ] **Step 2: 실패 확인** — `cargo test -p swisswheel-core drag` / 컴파일 오류
- [ ] **Step 3: 구현** — 삼킨 키는 `[bool; 256]`(vk 인덱스)로 기록; 통과시킨 `KeyDown`은 해당 플래그를 지운다.
- [ ] **Step 4: 통과 확인** — 6 passed
- [ ] **Step 5: 커밋** — `feat(core): drag/key state machine for opening the wheel`

---

### Task 3: `WheelSession` — 열린 휠 모델 (core)

**Files:** Create `crates/swisswheel-core/src/session.rs`; Modify `lib.rs`

**Interfaces:**
- Consumes: `catalog::{InputItem, WheelKind, Operation, items_for, initial_wheel, next_wheel}`, `geometry::{hit_test, Hit}`
- Produces:
  - `pub struct WheelSession` + `pub fn new() -> Self` (로딩 상태: 입력 없음)
  - `pub fn set_inputs(&mut self, inputs: Vec<InputItem>)` — 휠 = `initial_wheel`, 강조 해제
  - `pub fn is_loading(&self) -> bool` (아직 `set_inputs` 전)
  - `pub fn wheel(&self) -> Option<WheelKind>`, `pub fn items(&self) -> &[Operation]`
  - `pub fn cycle(&mut self)` — `next_wheel`, 강조 해제
  - `pub fn hover(&mut self, dx: f32, dy: f32, scale: f32) -> bool` — 강조가 바뀌었으면 `true`
  - `pub fn hovered(&self) -> Hit` (초기값 `Hit::Cancel`)
  - `pub fn drop_at(&self, dx: f32, dy: f32, scale: f32) -> Option<Operation>`

- [ ] **Step 1: 실패하는 테스트 작성**

```rust
use crate::catalog::*; use crate::format::Format; use crate::geometry::Hit;
fn png() -> InputItem { InputItem::File { format: Some(Format::Png), animated_gif: false } }

#[test]
fn loading_until_inputs_arrive() {
    let mut s = WheelSession::new();
    assert!(s.is_loading());
    assert_eq!(s.wheel(), None);
    assert_eq!(s.drop_at(0.0, -100.0, 1.0), None);
    s.set_inputs(vec![png()]);
    assert!(!s.is_loading());
    assert_eq!(s.wheel(), Some(WheelKind::Convert));
    assert_eq!(s.items()[0], Operation::Convert(Format::Jpg));
}

#[test]
fn hover_reports_changes_and_drop_picks_item() {
    let mut s = WheelSession::new();
    s.set_inputs(vec![png()]); // 6 items: JPG at top
    assert!(s.hover(0.0, -100.0, 1.0));
    assert_eq!(s.hovered(), Hit::Item(0));
    assert!(!s.hover(1.0, -101.0, 1.0)); // same sector
    assert_eq!(s.drop_at(0.0, -100.0, 1.0), Some(Operation::Convert(Format::Jpg)));
    assert_eq!(s.drop_at(0.0, 0.0, 1.0), None); // cancel center
}

#[test]
fn cycle_switches_wheel_and_clears_hover() {
    let mut s = WheelSession::new();
    s.set_inputs(vec![png()]);
    s.hover(0.0, -100.0, 1.0);
    s.cycle();
    assert_eq!(s.wheel(), Some(WheelKind::Tools));
    assert_eq!(s.hovered(), Hit::Cancel);
    assert_eq!(s.drop_at(0.0, -100.0, 1.0), Some(Operation::Tool(Tool::HalveImage)));
}

#[test]
fn empty_inputs_have_no_wheel() {
    let mut s = WheelSession::new();
    s.set_inputs(vec![]);
    assert!(!s.is_loading());
    assert_eq!(s.wheel(), None);
    s.cycle();
    assert_eq!(s.drop_at(0.0, -100.0, 1.0), None);
}
```

- [ ] **Step 2: 실패 확인** → **Step 3: 구현** (`items`는 휠이 바뀔 때만 다시 계산해 보관) → **Step 4: 4 passed** → **Step 5: 커밋** `feat(core): open-wheel session model`

---

### Task 4: 알림 문구 (core)

**Files:** Create `crates/swisswheel-core/src/summary.rs`; Modify `lib.rs`

**Interfaces:**
- Produces: `pub fn job_summary(op: &Operation, succeeded: usize, failures: &[String]) -> (String, String)` — (제목, 본문)
- 문구 (spec 2.4의 예시 문장을 조사 처리 없이 쓰도록 바꿈 — 영문 포맷명 뒤 '로/으로' 판별을 피하기 위함):
  - 모두 성공: 제목 `"{label} 완료"`, 본문 `"파일 {n}개 · 클릭하면 폴더를 엽니다"`
  - 일부/전부 실패: 제목 `"{label}: {k}개 실패"`, 본문 `"{첫 오류}"` + 성공이 있으면 `"\n성공 {n}개 · 클릭하면 폴더를 엽니다"`
  - `label`은 `Operation::label()`; `Convert`는 `"PNG 변환"`처럼 `" 변환"`을 붙인다.

- [ ] **Step 1: 실패하는 테스트 작성**

```rust
#[test]
fn all_succeeded() {
    let (t, b) = job_summary(&Operation::Convert(Format::Png), 3, &[]);
    assert_eq!(t, "PNG 변환 완료");
    assert_eq!(b, "파일 3개 · 클릭하면 폴더를 엽니다");
    let (t, _) = job_summary(&Operation::Tool(Tool::HalveImage), 1, &[]);
    assert_eq!(t, "50% 축소 완료");
}

#[test]
fn partial_and_total_failure() {
    let errs = vec!["사진.png: 이미지를 읽을 수 없습니다".to_string(), "b.png: x".to_string()];
    let (t, b) = job_summary(&Operation::Convert(Format::Jpg), 1, &errs);
    assert_eq!(t, "JPG 변환: 2개 실패");
    assert_eq!(b, "사진.png: 이미지를 읽을 수 없습니다\n성공 1개 · 클릭하면 폴더를 엽니다");
    let (_, b) = job_summary(&Operation::Convert(Format::Jpg), 0, &errs[..1]);
    assert_eq!(b, "사진.png: 이미지를 읽을 수 없습니다");
}
```

- [ ] **Step 2~5:** 실패 확인 → 구현 → 2 passed → 커밋 `feat(core): job summary notification text`

---

### Task 5: 입력 탐지 + 안전한 파일 확정

**Files:** Create `crates/swisswheel/src/probe.rs`, `crates/swisswheel/src/fsutil.rs`; Modify `main.rs` (`mod`)

**Interfaces:**
- Produces:
  - `probe::input_item(path: &Path) -> InputItem` — 폴더면 `Folder`; 아니면 `Format::from_path`; GIF면 `image::codecs::gif::GifDecoder`로 프레임을 **최대 2개까지만** 읽어 2개 이상이면 `animated_gif: true`. 읽기 실패 GIF는 정지로 취급.
  - `fsutil::finalize(temp: &Path, final_path: &Path) -> std::io::Result<()>` — `MoveFileExW(temp, final, MOVEFILE_WRITE_THROUGH)` (대체 플래그 없음). 대상이 있으면 `ErrorKind::AlreadyExists`.
  - `fsutil::Reservations` (`Default`, `Sync`) + `pub fn reserve(&self, input: &Path, extension: &str, suffix: Option<&str>) -> PathBuf` — `naming::output_path`를 `|p| p.exists() || 예약됨(p)`로 호출하고 결과를 예약(소문자 경로 기준) + `pub fn release(&self, path: &Path)`

- [ ] **Step 1: 실패하는 테스트 작성** (이미지는 테스트 안에서 `image`로 생성)

```rust
#[test]
fn probe_classifies_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let still = dir.path().join("still.gif");
    let anim = dir.path().join("anim.GIF");
    write_gif(&still, 1); write_gif(&anim, 2); // helper: n frames of 4x4 via GifEncoder
    std::fs::write(dir.path().join("notes.txt"), "x").unwrap();
    assert_eq!(input_item(dir.path()), InputItem::Folder);
    assert_eq!(input_item(&still), InputItem::File { format: Some(Format::Gif), animated_gif: false });
    assert_eq!(input_item(&anim), InputItem::File { format: Some(Format::Gif), animated_gif: true });
    assert_eq!(input_item(&dir.path().join("notes.txt")), InputItem::File { format: None, animated_gif: false });
}

#[test]
fn probe_handles_unicode_paths() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("여름 사진");
    std::fs::create_dir(&sub).unwrap();
    let p = sub.join("바다 1.gif");
    write_gif(&p, 2);
    assert_eq!(input_item(&p), InputItem::File { format: Some(Format::Gif), animated_gif: true });
}

#[test]
fn finalize_never_replaces() {
    let dir = tempfile::tempdir().unwrap();
    let (tmp, fin) = (dir.path().join("a.jpg.swisswheel-tmp"), dir.path().join("a.jpg"));
    std::fs::write(&tmp, "new").unwrap();
    std::fs::write(&fin, "old").unwrap();
    assert_eq!(finalize(&tmp, &fin).unwrap_err().kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read_to_string(&fin).unwrap(), "old");
    std::fs::remove_file(&fin).unwrap();
    finalize(&tmp, &fin).unwrap();
    assert_eq!(std::fs::read_to_string(&fin).unwrap(), "new");
    assert!(!tmp.exists());
}

#[test]
fn reservations_hand_out_distinct_paths() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("a.png");
    std::fs::write(&input, "x").unwrap();
    let r = Reservations::default();
    let first = r.reserve(&input, "jpg", None);
    let second = r.reserve(&input, "jpg", None);
    assert_eq!(first, dir.path().join("a.jpg"));
    assert_eq!(second, dir.path().join("a (1).jpg"));
    r.release(&first);
    assert_eq!(r.reserve(&input, "jpg", None), dir.path().join("a.jpg"));
}
```

- [ ] **Step 2~5:** 실패 확인 → 구현 (`MoveFileExW`의 `ERROR_ALREADY_EXISTS`/`ERROR_FILE_EXISTS` → `AlreadyExists`) → 4 passed → 커밋 `feat(app): input probing, output reservations, no-replace finalize`

---

### Task 6: 이미지 엔진

**Files:** Create `crates/swisswheel/src/engines/mod.rs`, `crates/swisswheel/src/engines/image.rs`; Modify `main.rs`

**Interfaces:**
- Consumes: `Operation`, `Format`, `Tool`, `Quality`
- Produces:
  - `engines::EngineError` (`thiserror`): `Read(String)`, `Unsupported(String)`, `Write(String)`, `Failed(String)` + `pub fn user_message(&self, file_name: &str) -> String` → `"{file}: 이미지를 읽을 수 없습니다"`, `"{file}: 지원하지 않는 작업입니다"`, `"{file}: 결과 파일을 쓸 수 없습니다"`, `"{file}: 변환에 실패했습니다"`
  - `engines::image::supports(op: &Operation) -> bool` — 이미지 대상 `Convert`와 `HalveImage`/`CompressImage`만 `true`
  - `engines::image::output_format(op: &Operation, input: Format) -> Format` — `Convert(f)` → `f`, 도구 → `input`
  - `engines::image::run(op: &Operation, input: &Path, output: &Path, quality: &Quality) -> Result<(), EngineError>` — `output`(임시 경로)에 `output_format`으로 기록. 확장자로 포맷을 추론하지 말고 명시적으로 인코딩.

**규칙 (spec 3.1, 3.2):** JPG/BMP 출력 시 알파를 흰 배경에 합성. WebP 손실 인코딩은 `webp` 크레이트(품질 `quality.webp`, 압축 도구는 70). ICO는 256/128/64/48/32/16 멀티 사이즈, 정사각형이 아니면 비율 유지 + 투명 여백. 50% 축소는 `(w/2).max(1)` × `(h/2).max(1)`, Lanczos3. PNG 압축은 `oxipng::optimize_from_memory` 기본 옵션.

- [ ] **Step 1: 실패하는 테스트 작성** (헬퍼 `rgba_png(path, w, h, alpha)`로 생성)

```rust
#[test]
fn converts_between_all_image_formats() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src.png");
    rgba_png(&src, 64, 48, 255);
    for target in [Format::Jpg, Format::Webp, Format::Bmp, Format::Tiff, Format::Gif, Format::Ico] {
        let out = dir.path().join(format!("out.{}.tmp", target.extension()));
        run(&Operation::Convert(target), &src, &out, &Quality::default()).unwrap();
        let fmt = image::ImageReader::open(&out).unwrap().with_guessed_format().unwrap().format();
        assert!(fmt.is_some(), "{target:?} produced unreadable output");
    }
}

#[test]
fn jpg_from_transparent_png_is_white() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("t.png");
    rgba_png(&src, 8, 8, 0); // fully transparent black
    let out = dir.path().join("t.jpg.tmp");
    run(&Operation::Convert(Format::Jpg), &src, &out, &Quality::default()).unwrap();
    let px = image::open(&out).unwrap().to_rgb8().get_pixel(4, 4).0;
    assert!(px.iter().all(|&c| c > 245), "{px:?}");
}

#[test]
fn ico_from_wide_image_keeps_aspect() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("wide.png");
    rgba_png(&src, 1000, 300, 255);
    let out = dir.path().join("wide.ico.tmp");
    run(&Operation::Convert(Format::Ico), &src, &out, &Quality::default()).unwrap();
    let decoder = image::codecs::ico::IcoDecoder::new(std::io::BufReader::new(std::fs::File::open(&out).unwrap())).unwrap();
    let img = image::DynamicImage::from_decoder(decoder).unwrap().to_rgba8(); // largest entry
    assert_eq!(img.dimensions(), (256, 256));
    assert_eq!(img.get_pixel(128, 2).0[3], 0); // top padding transparent
    assert_eq!(img.get_pixel(128, 128).0[3], 255); // content opaque
}

#[test]
fn halve_and_compress() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("big.jpg");
    image::RgbImage::from_fn(301, 200, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, 90])).save(&src).unwrap();
    let half = dir.path().join("half.tmp");
    run(&Operation::Tool(Tool::HalveImage), &src, &half, &Quality::default()).unwrap();
    assert_eq!(image::open(&half).unwrap().dimensions(), (150, 100));
    let small = dir.path().join("small.tmp");
    run(&Operation::Tool(Tool::CompressImage), &src, &small, &Quality::default()).unwrap();
    assert!(std::fs::metadata(&small).unwrap().len() < std::fs::metadata(&src).unwrap().len());
}

#[test]
fn unreadable_input_is_read_error() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("broken.png");
    std::fs::write(&src, b"not an image").unwrap();
    let err = run(&Operation::Convert(Format::Jpg), &src, &dir.path().join("o.tmp"), &Quality::default()).unwrap_err();
    assert!(matches!(err, EngineError::Read(_)));
    assert_eq!(err.user_message("broken.png"), "broken.png: 이미지를 읽을 수 없습니다");
}
```

- [ ] **Step 2: 실패 확인** — 컴파일 오류
- [ ] **Step 3: 구현** — 디코드는 `ImageReader::open(..).with_guessed_format()` (확장자가 틀린 파일도 내용으로 판별).
- [ ] **Step 4: 통과 확인** — 5 passed (릴리스가 아닌 디버그에서 ICO/oxipng가 느리면 `[profile.dev.package."*"] opt-level = 2`를 워크스페이스에 추가 — Ruling으로 기록)
- [ ] **Step 5: 커밋** — `feat(app): image conversion engine`

---

### Task 7: 작업 실행기

**Files:** Create `crates/swisswheel/src/jobs.rs`; Modify `main.rs`

**Interfaces:**
- Consumes: `probe`, `fsutil::{Reservations, finalize}`, `naming::temp_path`, `engines::image`, `Tool::suffix`, `Config`
- Produces:
  - `pub struct Job { pub op: Operation, pub inputs: Vec<PathBuf> }`
  - `pub struct JobReport { pub op: Operation, pub outputs: Vec<PathBuf>, pub failures: Vec<String> }` (`failures`는 `EngineError::user_message` 결과)
  - `pub struct Runner` + `pub fn new(workers: usize, quality: Quality, on_done: Box<dyn Fn(JobReport) + Send + Sync>) -> Runner` + `pub fn submit(&self, job: Job)` + `pub fn shutdown(self)` (대기 중 작업 처리 후 스레드 종료)
  - 파일 단위로 워커에 분배하고 한 `Job`의 모든 파일이 끝나면 `on_done`을 정확히 한 번 호출 (출력은 입력 순서).
  - `engines::image::supports(op)`가 `false`면 파일을 처리하지 않고 각 파일에 `"{file}: 다음 버전에서 지원 예정입니다"` 실패를 담는다.

**파일 하나 처리:** `reserve(input, ext, suffix)` → `temp_path(final)` → `run` → `finalize` → `release`. 실패 시 임시 파일 삭제 후 `release`. `finalize`가 `AlreadyExists`면(예약 이후 다른 프로그램이 같은 이름을 만든 경우) 한 번 더 `reserve`해서 재시도.

- [ ] **Step 1: 실패하는 테스트 작성** (헬퍼 `run_jobs(jobs) -> Vec<JobReport>`: `Runner`에 채널로 보고를 모아 `shutdown` 후 반환)

```rust
#[test]
fn batch_convert() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a.png"), dir.path().join("b.png"));
    rgba_png(&a, 10, 10, 255); rgba_png(&b, 10, 10, 255);
    let reports = run_jobs(vec![Job { op: Operation::Convert(Format::Jpg), inputs: vec![a, b] }]);
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].outputs, vec![dir.path().join("a.jpg"), dir.path().join("b.jpg")]);
    assert!(reports[0].failures.is_empty());
}

#[test]
fn concurrent_jobs_get_distinct_outputs() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.png");
    rgba_png(&a, 600, 600, 255);
    let job = || Job { op: Operation::Convert(Format::Jpg), inputs: vec![a.clone()] };
    let mut outs: Vec<_> = run_jobs(vec![job(), job()]).into_iter().flat_map(|r| r.outputs).collect();
    outs.sort();
    assert_eq!(outs, vec![dir.path().join("a (1).jpg"), dir.path().join("a.jpg")]);
}

#[test]
fn bad_input_fails_alone_and_leaves_no_temp() {
    let dir = tempfile::tempdir().unwrap();
    let good = dir.path().join("good.png");
    rgba_png(&good, 10, 10, 255);
    let empty = dir.path().join("empty.png");
    std::fs::write(&empty, b"").unwrap();
    let r = &run_jobs(vec![Job { op: Operation::Convert(Format::Webp), inputs: vec![empty, good] }])[0];
    assert_eq!(r.outputs, vec![dir.path().join("good.webp")]);
    assert_eq!(r.failures, vec!["empty.png: 이미지를 읽을 수 없습니다".to_string()]);
    let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap()
        .filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().ends_with(".swisswheel-tmp")).collect();
    assert!(leftovers.is_empty());
}

#[test]
fn batch_in_unicode_folder() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("가족 앨범");
    std::fs::create_dir(&sub).unwrap();
    let p = sub.join("할머니 생신.png");
    rgba_png(&p, 10, 10, 255);
    let r = &run_jobs(vec![Job { op: Operation::Tool(Tool::HalveImage), inputs: vec![p] }])[0];
    assert_eq!(r.outputs, vec![sub.join("할머니 생신 (50%).png")]);
}

#[test]
fn unsupported_operations_report_per_file() {
    let dir = tempfile::tempdir().unwrap();
    let v = dir.path().join("clip.mp4");
    std::fs::write(&v, b"x").unwrap();
    let r = &run_jobs(vec![Job { op: Operation::Convert(Format::Mp3), inputs: vec![v] }])[0];
    assert!(r.outputs.is_empty());
    assert_eq!(r.failures, vec!["clip.mp4: 다음 버전에서 지원 예정입니다".to_string()]);
}
```

- [ ] **Step 2~5:** 실패 확인 → 구현 (`std::sync::mpsc` + `Arc<Mutex<Receiver>>` 워커 풀; Job별 남은 개수 카운터) → 5 passed → 커밋 `feat(app): job runner with reservations and temp-file safety`

---

### Task 8: 트레이 상주 앱 (수동 검증)

**Files:** Create `crates/swisswheel/src/app.rs`, `crates/swisswheel/src/tray.rs`, `crates/swisswheel/src/icon.rs`; Modify `main.rs`

**Interfaces:**
- Produces:
  - `icon::make(busy: bool, size: i32) -> HICON` — 32비트 DIB에 GDI로 그린 원형 휠 아이콘(오렌지 계열 6분할, `busy`면 중앙 점). 외부 이미지 파일 없음.
  - `tray::Tray` + `pub fn new(hwnd: HWND) -> Result<Tray>` + `pub fn set_busy(&mut self, busy: bool, tooltip: &str)` + `pub fn notify(&self, title: &str, body: &str)` (`NIF_INFO` 풍선) + `Drop`에서 `NIM_DELETE`. 콜백 메시지 `WM_APP_TRAY = WM_APP + 10`.
  - `app::run() -> windows::core::Result<()>` — 단일 인스턴스(이름 있는 뮤텍스 `Local\SwissWheel.SingleInstance`; 이미 있으면 조용히 종료), `SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2)`, `OleInitialize`, `logging::init`, `config_file::load`(경고가 있으면 알림), 메시지 전용이 아닌 숨김 창(트레이·타이머·`PostMessage` 수신용) 생성, 메시지 루프. 트레이 우클릭 메뉴: `종료`.
  - 마지막 알림을 클릭하면(`NIN_BALLOONUSERCLICK`) 마지막 보고의 첫 출력 파일을 `explorer.exe /select,"<path>"`로 연다.
  - `main()`은 `app::run()`만 호출.

- [ ] **Step 1: 구현 후 빌드** — Run: `cargo build -p swisswheel` / Expected: 성공
- [ ] **Step 2: 수동 확인** — `cargo run -p swisswheel` 백그라운드 실행 후: (a) 트레이에 휠 아이콘 표시 (b) 두 번째 실행은 즉시 종료(트레이 아이콘 1개 유지) (c) `%APPDATA%\SwissWheel\config.toml` 생성 (d) 우클릭 → 종료 시 아이콘 사라지고 프로세스 종료 (e) 로그에 시작/종료 기록. 결과를 ledger에 기록.
- [ ] **Step 3: 커밋** — `feat(app): tray-resident shell with single instance`

---

### Task 9: 훅 + 휠 창 + 드롭 타깃 연결 (수동 검증)

**Files:** Create `crates/swisswheel/src/hook.rs`, `crates/swisswheel/src/dnd.rs`, `crates/swisswheel/src/wheel/mod.rs`, `crates/swisswheel/src/wheel/window.rs`, `crates/swisswheel/src/wheel/render.rs`; Modify `app.rs`

**Interfaces:**
- Consumes: `DragTracker`, `WheelSession`, `Runner`, `probe::input_item`, `geometry::{clamp_center, sector}`, `job_summary`, `Tray`
- Produces:
  - `hook::install(target: HWND, keys: KeyBindings) -> Result<Hooks>` — `WH_MOUSE_LL`/`WH_KEYBOARD_LL`; 콜백은 스레드 로컬 `DragTracker::on_event` → `Decision.swallow`면 `LRESULT(1)`, `command`는 `PostMessageW(target, WM_APP_DRAG, command as usize)`. 임계값은 `SM_CXDRAG/SM_CYDRAG`. `hook::notify(event: DragEvent)` — 앱이 `EnterTimedOut`/`WheelClosed`를 전달할 때 사용. `Hooks` Drop 시 해제. 윈도우가 시간 초과로 훅을 조용히 해제해도 알려주는 API가 없으므로, 휠이 닫혀 있을 때 **10분마다 훅을 재설치**한다 (spec 4.2 "훅 해제 감지 시 재설치"의 단순화 — Ruling으로 기록).
  - `wheel::WheelWindow` — `WS_EX_NOREDIRECTIONBITMAP|WS_EX_TOPMOST|WS_EX_TOOLWINDOW|WS_EX_NOACTIVATE` 팝업, `SetWindowRgn(CreateEllipticRgn)`으로 원 밖은 히트 테스트에서 제외. `pub fn show_at(&mut self, cursor: POINT)` (커서 모니터의 `GetDpiForMonitor`와 `rcWork`로 `clamp_center`), `pub fn hide(&mut self)`, `pub fn scale(&self) -> f32`, `pub fn center(&self) -> POINT`, `pub fn render(&mut self, view: &WheelView)`.
  - `wheel::WheelView<'a> { pub loading: bool, pub title: &'a str, pub items: &'a [Operation], pub hovered: Hit, pub progress: f32 }` (`progress` 0..1 = 열림 애니메이션)
  - `render.rs` — D3D11(BGRA) → DXGI → `ID2D1DeviceContext`, `IDCompositionDevice` + `CreateSwapChainForComposition`(premultiplied alpha). 섹터는 `geometry::sector`로 경로 생성, 라벨은 DirectWrite(Segoe UI Variable → 없으면 Segoe UI). 색: 테마(`HKCU\...\Themes\Personalize\AppsUseLightTheme`)별 배경/섹터/강조 3색, 강조 섹터는 바깥 반지름 +6 DIP. 중앙 원에 휠 이름(`WheelKind::label`) 또는 로딩 중 `"…"`. 디바이스·스왑체인은 첫 표시 때 만들어 유지하고, 휠을 10분간 열지 않으면 해제한다 (spec 4.3). `D2DERR_RECREATE_TARGET`/디바이스 손실 시 다시 만든다.
  - `dnd::DropTarget` (`#[implement(IDropTarget)]`) — `DragEnter`: `CF_HDROP` 경로 → `probe::input_item` → 앱에 `set_inputs` 전달 후 다시 그리기; `DragOver`: `session.hover(pt - center, scale)`이 `true`면 다시 그리기, 강조가 `Item`이면 `DROPEFFECT_COPY`, 아니면 `DROPEFFECT_NONE`; `Drop`: `session.drop_at` → `Some(op)`이면 `Runner::submit(Job { op, inputs })` + `tray.set_busy(true, …)`; 마지막으로 휠 닫기. 앱 상태는 `Rc<RefCell<AppState>>`로 공유(모두 UI 스레드).

**앱 흐름 (spec 4.2, 스파이크 반영):** `OpenWheel` → 새 `WheelSession`, `show_at(GetCursorPos)`, 0픽셀 `SendInput`, 300ms 타이머. 타이머 만료 시 `is_loading()`이면 `hook::notify(EnterTimedOut)` + 닫기. `Cycle` → `session.cycle()` + 다시 그리기. `Cancel` → 닫기. `ButtonReleasedWhileOpen` → 200ms 타이머 후 닫기. 모든 닫기는 `hook::notify(WheelClosed)`. 작업 완료(`on_done`은 워커 스레드) → 보고를 `Mutex<VecDeque>`에 넣고 `PostMessageW(WM_APP_JOB_DONE)` → UI 스레드에서 `job_summary`로 알림, 진행 중 작업이 0이면 `set_busy(false)`. 애니메이션은 16ms 타이머로 `progress`를 150ms 동안 0→1.

- [ ] **Step 1: 구현 후 빌드** — `cargo build -p swisswheel` 성공, clippy 경고 0
- [ ] **Step 2: 수동 확인** (`docs/manual-test.md`의 M1~M8, Task 10에서 문서화하지만 여기서 먼저 실행) — 결과를 ledger에 기록. 실패 항목은 superpowers:systematic-debugging으로 원인을 찾아 고친 뒤 재확인.
- [ ] **Step 3: 커밋** — `feat(app): drag hook, wheel window, and drop target`

---

### Task 10: 수동 테스트 문서 + 종단 확인

**Files:** Create `docs/manual-test.md`; Modify `README.md` (사용법·현재 지원 범위·알려진 제한 spec 5장)

`docs/manual-test.md` 항목 (각 항목에 절차와 PASS 조건):

| ID | 확인 |
|---|---|
| M1 | 탐색기에서 PNG 드래그 중 Space → 커서 위치에 휠, 마우스를 움직이지 않아도 300ms 안에 항목 표시 |
| M2 | Tab으로 변환 → 도구 → 동작 → 변환 순환, 도구가 없는 파일(FLAC)은 도구 휠 건너뜀 |
| M3 | 섹터 위에서 강조·확대, 중앙에서는 드롭 금지 커서 |
| M4 | PNG → JPG 드롭 → 같은 폴더에 `이름.jpg`, 알림 "JPG 변환 완료", 알림 클릭 시 탐색기가 결과 파일을 선택한 채 열림 |
| M5 | **원본이 드롭 5초 뒤에도 그대로 있음** (스파이크 미확정 건 재확인; 같은 드라이브 드래그와 Shift 드래그 모두) |
| M6 | 같은 파일을 연달아 두 번 JPG로 → `a.jpg`, `a (1).jpg` |
| M7 | 창 제목 표시줄 드래그 중 Space 길게 누름 → 휠이 한 번만 잠깐 뜨고 깜빡임 없음, 메모장 글자 선택 중 Space는 메모장에 입력됨 |
| M8 | Esc → 휠만 닫히고 다른 폴더로 정상 이동 / 휠 밖에서 놓기 → 원래 대상에 정상 드롭 |
| M9 | 화면 오른쪽 아래 구석에서 열기 → 휠이 화면 안으로 밀려 들어옴; DPI 125%/150%에서 크기·히트 영역 일치 |
| M10 | 다크/라이트 테마 전환 후 휠 색상 반영 |
| M11 | 영상(MP4)을 MP3로 드롭 → 알림 "…다음 버전에서 지원 예정입니다" |
| M12 | 트레이 대기 상태 메모리: 작업 관리자 "메모리(활성 개인 작업 집합)" 15MB 이하 (휠을 몇 번 연 뒤 1분 대기) |

- [ ] **Step 1: 문서 작성 후 릴리스 빌드로 M1~M12 수행** — Run: `cargo build --release -p swisswheel` 후 `target/release/swisswheel.exe` 실행. 사용자와 함께 수행하고 결과를 ledger에 기록. M12가 15MB를 넘으면 측정값을 기록하고 원인(예: D2D 리소스 유지)을 Ruling으로 남긴다 (spec 4.3의 10분 해제 정책이 이미 있음).
- [ ] **Step 2: 커밋** — `docs: manual test checklist and README usage`

---

## 다음 계획 (범위 밖)
FFmpeg 엔진(영상·오디오 변환, 영상 압축·오디오 추출·음소거, 진행률, Job Object), 동작 휠 5종, 트레이 메뉴 확장(시작 시 실행·설정 열기/다시 불러오기·로그 폴더·정보), 관리자 실행 감지 경고, Inno Setup·포터블 zip·`THIRD_PARTY_LICENSES`, CI 릴리스 빌드. 이전 리뷰에서 미룬 항목: naming 자기 충돌 비교를 파일 이름 기준으로, 설정 키 이름 대소문자 무시.
