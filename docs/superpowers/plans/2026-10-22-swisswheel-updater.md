# SwissWheel 업데이터 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 메뉴의 업데이트 타일이 GitHub Releases에서 새 버전을 확인하고, 승낙하면 받아서 검사·교체·재시작한다.

**Architecture:** 순수 로직(버전, 릴리스 JSON, 체크섬, 문구, 판단)은 `swisswheel-core::update`에 둔다. 앱 쪽 `swisswheel::update`는 세 부분이다: WinHTTP 클라이언트(`http`), 파일 교체(`swap`), 업데이트 스레드 흐름(`mod`). 업데이트 스레드는 확인 창을 직접 띄우고, 결과를 `PostMessageW`로 UI 스레드에 넘긴다. 교체는 UI 스레드가 작업이 모두 끝난 뒤에 한다.

**Tech Stack:** Rust 2024, `windows` 0.61 (`Win32_Networking_WinHttp` 추가), `serde_json` 1 (core), `sha2` 0.10 (app), GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-10-22-updater-design.md`

## Global Constraints

- 엔드포인트: `https://api.github.com/repos/glglekdy/SwissWheel/releases/latest`
- 릴리스 자산 이름: `swisswheel.exe`, `swisswheel.exe.sha256`
- 교체 파일 이름: 같은 폴더의 `swisswheel.exe.new`, `swisswheel.exe.old`. `current_exe()`의 파일 이름과 상관없이 이 두 이름을 쓴다(`<exe 폴더>\swisswheel.exe.new`).
- 시작 인자: `--after-update <pid>`. 옛 프로세스 대기 최대 15초.
- HTTP: `User-Agent: SwissWheel/<CARGO_PKG_VERSION>`, 연결·수신 타임아웃 30초, 본문 상한 JSON 1MB · exe 64MB. 릴리스 빌드는 `https://`만. `cfg!(debug_assertions)`에서만 `http://127.0.0.1` 허용.
- 디버그 빌드에서만 `SWISSWHEEL_UPDATE_URL`이 엔드포인트를 바꾼다.
- 확인 창: `MessageBoxW`, 제목 "SwissWheel 업데이트", 업데이트 스레드에서 띄운다(UI 스레드 금지).
- 릴리스 설명은 최대 600자, 넘으면 앞 600자 + "…".
- 받은 내용은 메모리에 두고, 해시 통과 뒤에만 `.new`로 쓴다. 종료 시 업데이트 스레드를 기다리지 않는다.
- 사용자 문구(그대로):
  - "진행 중인 변환이 끝난 뒤 다시 눌러 주세요"
  - "이미 업데이트를 진행하는 중입니다"
  - "최신 버전입니다 (v{현재})"
  - "업데이트를 확인할 수 없습니다"
  - "업데이트 파일을 찾을 수 없습니다"
  - "업데이트 파일을 받지 못했습니다"
  - "업데이트 파일이 손상되었습니다"
  - "변환이 끝나면 업데이트합니다"
  - "업데이트하지 못했습니다"
  - "v{새}으로 업데이트했습니다"
  - 확인 창 본문 "v{새}이 있습니다 (지금 v{현재}).\n\n{설명}\n\n지금 업데이트할까요?". 설명이 비면 가운데 단락을 뺀다.
  - 쓰기 불가 본문 "v{새}이 있습니다. 이 위치에서는 자동으로 바꿀 수 없어 다운로드 페이지를 엽니다."
- 알림 제목은 모두 "업데이트".
- 커밋 트레일러: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` + `Claude-Session: https://claude.ai/code/session_016PYsmXwW5jKgU1htHsgH9G`
- 매 작업 끝: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` 모두 통과.

## Review Focus

1. **실제 GitHub 응답 모양.** 모르는 필드가 많고, `body`가 `null`이며, 자산이 여러 개다. 해석이 성공하고 맞는 자산을 고른다. → Task 1 `parses_real_github_shape_with_null_body`
2. **받는 중 연결이 끊기거나 해시가 틀림.** 디스크에 `.new`가 생기지 않고, 기존 exe가 그대로다. → Task 4 `corrupt_download_writes_nothing`, `dropped_connection_writes_nothing`
3. **한글·공백이 들어간 설치 폴더.** 쓰기 판정, 교체, 정리가 정상이다. → Task 3 `swap_in_unicode_folder`
4. **받는 동안 새 변환 시작.** 교체가 미뤄지고, 마지막 작업이 끝난 직후 교체한다. → Task 1 `ready_waits_for_jobs`, Task 5에서 `on_jobs_done` 연결
5. **옛 앱이 늦게 꺼짐.** 새 앱이 기다렸다가 뮤텍스를 잡는다. 너무 늦으면 15초 뒤 포기한다. → Task 3 `wait_for_exit_waits_for_child`, `wait_for_exit_times_out`

---

### Task 1: core — 버전, 릴리스 해석, 체크섬, 문구, 판단

**Files:**
- Create: `crates/swisswheel-core/src/update.rs`
- Modify: `crates/swisswheel-core/src/lib.rs` (`pub mod update;`), `crates/swisswheel-core/Cargo.toml` (`serde_json = "1"`)

**Interfaces — Produces:**
```rust
pub const ENDPOINT: &str = "https://api.github.com/repos/glglekdy/SwissWheel/releases/latest";
pub const EXE_ASSET: &str = "swisswheel.exe";
pub const SHA_ASSET: &str = "swisswheel.exe.sha256";
pub const NOTES_LIMIT: usize = 600;
pub const TITLE: &str = "업데이트";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version { pub major: u32, pub minor: u32, pub patch: u32 }
impl Version { pub fn parse(s: &str) -> Option<Version> }      // "v0.2.0" | "0.2.0"
impl std::fmt::Display for Version                              // "0.2.0" (no v)

#[derive(Debug, Clone, PartialEq)]
pub struct Release { pub version: Version, pub notes: String, pub page_url: String,
                     pub exe_url: Option<String>, pub sha_url: Option<String> }
pub fn parse_release(json: &str) -> Option<Release>             // None: bad JSON or bad tag
pub fn parse_checksum(text: &str) -> Option<[u8; 32]>           // first token, 64 hex, any case
pub fn prompt_body(new: Version, current: Version, notes: &str) -> String
pub fn not_writable_body(new: Version) -> String
pub fn after_update_pid<I: IntoIterator<Item = String>>(args: I) -> Option<u32>

#[derive(Debug, PartialEq, Eq)]
pub enum TileDecision { Start, Refuse(&'static str) }
pub fn on_tile(update_running: bool, active_jobs: usize) -> TileDecision
#[derive(Debug, PartialEq, Eq)]
pub enum ReadyDecision { SwapNow, WaitForJobs }
pub fn on_ready(active_jobs: usize) -> ReadyDecision
pub mod text { pub const BUSY: &str; pub const RUNNING: &str; pub const CHECK_FAILED: &str;
               pub const NO_ASSETS: &str; pub const DOWNLOAD_FAILED: &str; pub const CORRUPT: &str;
               pub const WAITING: &str; pub const SWAP_FAILED: &str;
               pub fn up_to_date(v: Version) -> String; pub fn updated(v: Version) -> String; }
```
`on_tile`: 작업이 진행 중이면 `Refuse(text::BUSY)`. 그다음 업데이트가 진행 중이면 `Refuse(text::RUNNING)`. 둘 다 아니면 `Start`. 작업 쪽을 먼저 본다.

- [ ] **Step 1: Write the failing tests** (`#[cfg(test)] mod tests` in `update.rs`)
  - `version_parse_and_order`: `parse("v0.1.0") < parse("0.1.1") < parse("v0.2.0") < parse("0.10.0")`, `parse("v1.2.3") == parse("1.2.3")`, `Version::parse("v0.2.0").unwrap().to_string() == "0.2.0"`
  - `version_rejects_other_shapes`: `""`, `"1.2"`, `"1.2.3-beta"`, `"v1.2.3.4"`, `"vx.1.2"` → `None`
  - `parses_real_github_shape_with_null_body`: 실제 응답 모양 JSON이다. `tag_name: "v0.2.0"`, `html_url`, `"body": null`, 자산 3개(`swisswheel.exe`, `swisswheel.exe.sha256`, `other.zip`, 각 `browser_download_url`·`size`·`id`), 그 밖에 모르는 필드 다수. 결과는 `notes == ""`, `page_url == html_url`, `exe_url`과 `sha_url`이 각 자산의 URL이다.
  - `missing_assets_are_none`: 자산이 `other.zip`뿐이면 `exe_url == None && sha_url == None`이다.
  - `bad_release_json_is_none`: `"not json"`, 올바른 JSON에 `tag_name: "latest"` → `None`
  - `checksum_forms`: 해시만, `"<hex>  swisswheel.exe\n"`, 대문자 → `Some`. 63자, 16진수 아닌 글자, 빈 값 → `None`
  - `prompt_body_truncates_and_skips_empty_notes`:
    - 601자 설명이면 본문에 앞 600자 + "…"가 들어간다. 자를 때 문자 단위(`chars`)로 센다.
    - 설명이 빈 값이나 공백뿐이면 `"v0.2.0이 있습니다 (지금 v0.1.0).\n\n지금 업데이트할까요?"`
  - `after_update_arg`: `["exe","--after-update","1234"]` → `Some(1234)`, `["exe","--after-update","x"]`, `["exe"]`, `["exe","--after-update"]` → `None`
  - `tile_refuses_while_busy`: `on_tile(false,1) == Refuse(text::BUSY)`, `on_tile(true,0) == Refuse(text::RUNNING)`, `on_tile(true,2) == Refuse(text::BUSY)`, `on_tile(false,0) == Start`
  - `ready_waits_for_jobs`: `on_ready(0) == SwapNow`, `on_ready(1) == WaitForJobs`
  - `texts`: `text::up_to_date(v0.1.0) == "최신 버전입니다 (v0.1.0)"`, `text::updated(v0.2.0) == "v0.2.0으로 업데이트했습니다"`

- [ ] **Step 2: Run** `cargo test -p swisswheel-core update` — Expected: FAIL (모듈 없음)
- [ ] **Step 3: Implement** `update.rs`. `serde_json::Value`로 해석한다. 문구는 Global Constraints 그대로다.
- [ ] **Step 4: Run** `cargo test -p swisswheel-core update` — Expected: PASS 11/11
- [ ] **Step 5: Commit** `feat(core): update versions, release parsing, and decisions`

---

### Task 2: app — WinHTTP 클라이언트와 테스트 서버

**Files:**
- Create: `crates/swisswheel/src/update/mod.rs` (이 작업에서는 `pub mod http;`과 `#[cfg(test)] mod test_server;`만), `crates/swisswheel/src/update/http.rs`, `crates/swisswheel/src/update/test_server.rs`
- Modify: `crates/swisswheel/src/main.rs` (`mod update;`), `crates/swisswheel/Cargo.toml` (`windows` 기능 `Win32_Networking_WinHttp`)

**Interfaces — Produces:**
```rust
// http.rs
#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("주소 오류: {0}")] BadUrl(String),
    #[error("HTTP {0}")] Status(u32),
    #[error("응답이 너무 큽니다")] TooLarge,
    #[error("네트워크 오류: {0}")] Net(#[from] windows_core::Error),
}
/// GET `url`, following redirects; `progress(received, total)` after each chunk
/// (`total` from Content-Length, `None` if absent). Fails past `limit` bytes.
pub fn get(url: &str, limit: usize, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<Vec<u8>, HttpError>

// test_server.rs (cfg(test))
pub enum Reply { Ok(Vec<u8>), Status(u16), Redirect(String), Drop { send: Vec<u8>, claimed: usize } }
pub struct TestServer { pub base: String /* "http://127.0.0.1:PORT" */ }
impl TestServer { pub fn start(routes: Vec<(&'static str, Reply)>) -> TestServer }
```
- `get`은 `WinHttpCrackUrl`로 URL을 나눈다.
  - `https`면 `WINHTTP_FLAG_SECURE`를 붙인다.
  - `http`는 `cfg!(debug_assertions)`이고 호스트가 `127.0.0.1`일 때만 허용한다. 아니면 `BadUrl`이다.
  - 핸들은 `Drop`에서 `WinHttpCloseHandle`로 닫는 가드로 감싼다.
  - 타임아웃은 `WinHttpSetTimeouts(30_000 ×4)`, 상태 코드는 `WINHTTP_QUERY_STATUS_CODE | FLAG_NUMBER`로 얻는다.
  - 200이 아니면 `Status`다. 리다이렉트는 WinHTTP가 알아서 따라간다.
  - `Content-Length`가 상한보다 크거나 받은 양이 상한을 넘으면 `TooLarge`다.
  - 받은 양이 `Content-Length`보다 적은데 연결이 끝나면 `Net`이 아니라 `HttpError::Status(0)`로 본다. 잘린 응답이다.
- `TestServer`는 `TcpListener::bind("127.0.0.1:0")` 위에서 스레드 하나로 요청을 받는다.
  - 요청 줄에서 경로를 읽고, `Connection: close`로 응답한다.
  - `Drop`은 `Content-Length: claimed`를 보내고 `send`만큼만 쓴 뒤 소켓을 닫는다.

- [ ] **Step 1: Write the failing tests** in `http.rs`:
  - `gets_body_and_reports_progress`: 100 KB 본문 → `Ok` 본문 일치, 마지막 progress가 `(102400, Some(102400))`
  - `follows_redirect`: `/a` → `Redirect("/b")`, `/b` → `Ok(b"hi")`. `get(base+"/a")` 결과가 `b"hi"`다.
  - `not_found_is_status_error`: `Status(404)` → `Err(HttpError::Status(404))`
  - `over_limit_fails`: 2 KB 본문, `limit` 1024 → `Err(TooLarge)`
  - `truncated_body_fails`: `Drop{send: 10 bytes, claimed: 1000}` → `Err(_)`
  - `rejects_plain_http_elsewhere`: `get("http://example.com/x", ..)` → `Err(BadUrl(_))`. 연결하지 않는다.
- [ ] **Step 2: Run** `cargo test -p swisswheel http` — Expected: FAIL (모듈 없음)
- [ ] **Step 3: Implement** `http.rs`, `test_server.rs`
- [ ] **Step 4: Run** `cargo test -p swisswheel http` — Expected: PASS 6/6
- [ ] **Step 5: Commit** `feat(update): WinHTTP GET with redirects, limits, and progress`

---

### Task 3: app — 교체, 되돌리기, 정리, 옛 프로세스 대기

**Files:**
- Create: `crates/swisswheel/src/update/swap.rs`
- Modify: `crates/swisswheel/src/update/mod.rs` (`pub mod swap;`)

**Interfaces — Produces:**
```rust
pub fn new_path(exe: &Path) -> PathBuf          // <dir>\swisswheel.exe.new
pub fn old_path(exe: &Path) -> PathBuf          // <dir>\swisswheel.exe.old
pub fn writable(exe: &Path) -> bool             // create + delete new_path
pub fn write_new(exe: &Path, bytes: &[u8]) -> std::io::Result<()>
/// exe → .old, .new → exe, then `launch(exe)`; on any failure restores exe.
pub fn swap(exe: &Path, launch: impl FnOnce(&Path) -> std::io::Result<()>) -> std::io::Result<()>
pub fn cleanup(exe: &Path)                      // remove .old and .new, ignore errors
pub fn launch_after_update(exe: &Path) -> std::io::Result<()>  // CreateProcessW "--after-update <pid>"
pub fn wait_for_exit(pid: u32, timeout: Duration) -> bool      // true: exited (or no such pid)
```
`swap`은 스펙 3.3의 1~4를 따른다(`MoveFileExW`, `MOVEFILE_REPLACE_EXISTING`). `launch_after_update`의 명령줄은 `"<exe>" --after-update <GetCurrentProcessId()>`다.

- [ ] **Step 1: Write the failing tests** (tempdir에 `swisswheel.exe` 내용 `b"old"`)
  - `swap_replaces_and_keeps_old`: `write_new(b"new")` → `swap(exe, |_| Ok(()))` 결과 exe == `b"new"`, `.old` == `b"old"`, `.new` 없음
  - `swap_restores_when_launch_fails`: `launch`가 `Err` → exe == `b"old"`, `.new`·`.old` 없음, 결과 `Err`
  - `swap_restores_when_new_missing`: `.new` 없이 `swap` → `Err`, exe == `b"old"`
  - `swap_in_unicode_folder`: `업데이트 테스트 폴더` 안에서 첫 테스트와 같음 + `writable == true`
  - `cleanup_removes_leftovers`: `.old`·`.new` 생성 → `cleanup` → 둘 다 없음, exe 그대로
  - `not_writable_for_missing_dir`: 존재하지 않는 폴더의 exe → `writable == false`
  - `wait_for_exit_waits_for_child`: `cmd /c ping -n 2 127.0.0.1 >nul` 자식 → `wait_for_exit(pid, 15s) == true`. 걸린 시간 ≥ 0.5초, 자식은 이미 종료 상태.
  - `wait_for_exit_times_out`: `ping -n 6` 자식 → `wait_for_exit(pid, 300ms) == false`. 자식은 테스트 끝에서 kill.
- [ ] **Step 2: Run** `cargo test -p swisswheel swap` — Expected: FAIL
- [ ] **Step 3: Implement** `swap.rs`
- [ ] **Step 4: Run** `cargo test -p swisswheel swap` — Expected: PASS 8/8
- [ ] **Step 5: Commit** `feat(update): swap the exe with rollback, cleanup, and exit wait`

---

### Task 4: app — 업데이트 흐름

**Files:**
- Modify: `crates/swisswheel/src/update/mod.rs`, `crates/swisswheel/Cargo.toml` (`sha2 = "0.10"`)

**Interfaces — Consumes:** Task 1 전부, `http::get`, `swap::{writable, write_new}`.
**Produces:**
```rust
#[derive(Debug, PartialEq)]
pub enum Outcome {
    UpToDate,                       // notice text::up_to_date(current)
    CheckFailed, MissingAssets, DownloadFailed, Corrupt,
    Declined,
    NotWritable { page_url: String },   // the app opens it
    Ready { version: Version },         // .new written and verified
}
pub trait Ui { fn confirm(&self, body: &str) -> bool; fn inform(&self, body: &str); }
/// The whole check→download→verify flow; blocks.
/// Shared with the UI thread: download fraction (0..=10000) and, once confirmed, the new version.
pub struct Progress { pub fraction: AtomicU32, pub version: Mutex<Option<Version>> }
pub fn run(endpoint: &str, current: Version, exe: &Path, ui: &dyn Ui, progress: &Progress) -> Outcome
pub fn endpoint() -> String     // ENDPOINT, or SWISSWHEEL_UPDATE_URL in debug builds
pub struct MessageBoxUi;        // impl Ui via MessageBoxW (YESNO|ICONINFORMATION|TOPMOST|SETFOREGROUND; inform: OK)
pub struct Handle { pub progress: Arc<Progress> }
/// Spawns the update thread; when done, stores the outcome in `slot` and posts `msg` to `hwnd`.
pub fn spawn(hwnd: isize, msg: u32, slot: Arc<Mutex<Option<Outcome>>>) -> Handle
```
흐름:
1. JSON은 상한 1MB로 받는다. HTTP 실패나 해석 실패는 `CheckFailed`다.
2. `version <= current`면 `UpToDate`다.
3. 자산 URL이 없으면 `MissingAssets`다.
4. `!writable(exe)`면 `ui.inform(not_writable_body)`를 보이고 `NotWritable`이다.
5. `!ui.confirm(prompt_body)`면 `Declined`다.
6. 확인 뒤 `progress.version`에 새 버전을 넣는다. 진행 표시 이름에 쓴다.
7. `.sha256`은 상한 1KB, exe는 상한 64MB로 받는다. 실패는 `DownloadFailed`다.
8. `sha2::Sha256`이 다르거나 체크섬을 해석할 수 없으면 `Corrupt`다.
9. 해시가 맞으면 `write_new`를 하고 `Ready`다. 쓰기 실패는 `DownloadFailed`다.

`progress.fraction`은 exe를 받는 동안 `received*10000/total`이다. 모든 단계를 `log_info!`/`log_error!`로 남긴다.

- [ ] **Step 1: Write the failing tests** (`TestServer`와 tempdir의 `swisswheel.exe`, 테스트용 `struct FakeUi { answer: bool, shown: RefCell<Vec<String>> }`)
  - `newer_release_downloads_and_verifies`: 릴리스 v9.0.0, 맞는 체크섬 → `Ready{9.0.0}`, `.new` 내용 일치, `progress.fraction == 10000`, `progress.version == Some(9.0.0)`, confirm 본문에 "v9.0.0이 있습니다"
  - `same_version_is_up_to_date`: 릴리스 = `current` → `UpToDate`, UI 호출 없음
  - `declined_downloads_nothing`: `answer: false` → `Declined`, `.new` 없음
  - `corrupt_download_writes_nothing`: 체크섬 불일치 → `Corrupt`, `.new` 없음, exe 그대로
  - `dropped_connection_writes_nothing`: exe 경로가 `Drop` → `DownloadFailed`, `.new` 없음
  - `missing_assets_reported`: 자산 없음 → `MissingAssets`
  - `server_error_is_check_failed`: 엔드포인트 `Status(500)` → `CheckFailed`
  - `unwritable_folder_opens_page`: exe를 없는 폴더로 → `NotWritable{page_url}`, `inform` 호출 1회, `confirm` 0회
- [ ] **Step 2: Run** `cargo test -p swisswheel update::tests` — Expected: FAIL
- [ ] **Step 3: Implement** `run`, `endpoint`, `MessageBoxUi`, `spawn`
- [ ] **Step 4: Run** `cargo test -p swisswheel update::tests` — Expected: PASS 8/8
- [ ] **Step 5: Commit** `feat(update): check, confirm, download, and verify on a worker thread`

---

### Task 5: app — 타일, 진행 표시, 교체, 시작 처리 연결

**Files:**
- Modify: `crates/swisswheel/src/app.rs`, `crates/swisswheel/src/jobs.rs`, `crates/swisswheel/src/progress.rs`

**Interfaces — Consumes:** Task 1 `on_tile`/`on_ready`/`text`/`after_update_pid`/`TITLE`, Task 3 `swap`/`cleanup`/`launch_after_update`/`wait_for_exit`, Task 4 `spawn`/`Outcome`/`Handle`.
**Changes:**
- `ProgressRow`를 `{ title: String, fraction: Option<f32> }`로 바꾼다.
  - `Runner::rows`가 `progress_title(&file, &op)`로 `title`을 채운다.
  - `progress.rs`는 `row.title`을 그린다. `app.rs` 미리보기도 맞춘다.
- `AppState`에 세 필드를 더한다.
  - `update: Option<update::Handle>`: 업데이트 스레드가 도는 중이다.
  - `update_slot: Arc<Mutex<Option<Outcome>>>`
  - `pending_swap: Option<Version>`
- 새 메시지 `WM_APP_UPDATE = WM_APP + 31`을 둔다.
- `MenuAction::Notice(Tile::Update)`: 바를 닫고 `on_tile(self.update.is_some(), self.active_jobs)`로 판단한다.
  - `Refuse(t)`면 알림을 띄운다.
  - `Start`면 `update::spawn`으로 시작하고, `TIMER_PROGRESS`를 켠다. 그 밖의 타일은 지금처럼 알림이다.
- `update_busy`: 업데이트가 진행 중이고 `progress.version`이 `Some`이면 행 하나를 더한다.
  - 제목 `"SwissWheel v{ver} · 업데이트"`, 진행률은 `progress.fraction/10000`이다.
  - 트레이 링, 툴팁, 진행 창, 타이머는 "작업 또는 업데이트 받는 중"이면 켠다.
- `WM_APP_UPDATE`: `update = None`으로 두고 결과마다 처리한다.
  - `UpToDate` → `notify(TITLE, up_to_date(current))`
  - `CheckFailed`, `MissingAssets`, `DownloadFailed`, `Corrupt` → `warn(TITLE, …)`
  - `Declined` → 아무것도 안 한다.
  - `NotWritable{url}` → `ShellExecuteW("open", url)`
  - `Ready{v}` → `on_ready(active_jobs)`에 따른다. `SwapNow`면 `apply_swap(v)`, `WaitForJobs`면 `pending_swap = Some(v)`와 알림 `text::WAITING`.
- `on_jobs_done` 끝: `active_jobs == 0`이고 `pending_swap`이 있으면 `apply_swap`.
- `apply_swap`: `swap(&exe, launch_after_update)`.
  - 성공하면 `PostMessageW(hwnd, WM_CLOSE)`다. `with_state` 안에서 `DestroyWindow`를 부르면 `STATE`가 재진입하므로 쓰지 않는다.
  - 실패하면 `warn(TITLE, SWAP_FAILED)`와 로그를 남긴다.
- `run()` 시작:
  1. `after_update_pid(std::env::args())`가 있으면 **`CreateMutexW` 전에** `wait_for_exit(pid, 15s)`를 부른다.
  2. 트레이를 만든 뒤 `swap::cleanup(&current_exe)`를 부른다.
  3. 인자가 있었으면 `notify(TITLE, updated(current))`를 띄운다.

- [ ] **Step 1: Write the failing test** in `jobs.rs`: `rows_carry_titles`
  - PNG→JPG 작업 하나가 대기 중일 때 `rows()[0].title == "a.png → JPG"`, `fraction == None`이다.
  - 기존 rows 테스트는 새 필드에 맞춘다.
- [ ] **Step 2: Run** `cargo test -p swisswheel jobs` — Expected: FAIL (컴파일 오류: `title` 필드 없음)
- [ ] **Step 3: Implement** 위 변경 전부
- [ ] **Step 4: Run** `cargo test --workspace` — Expected: 전부 PASS. `cargo build --release` 성공
- [ ] **Step 5: 디버그 수동 확인**
  - `python -m http.server`로 가짜 릴리스를 연다. JSON의 URL이 `http://127.0.0.1:8000/...`인 것을 스크래치에 둔다.
  - `SWISSWHEEL_UPDATE_URL`을 넣고 `cargo build`한 디버그 exe를 스크래치 폴더에 복사해 실행한다.
  - 업데이트 타일 → 승낙 → 새 프로세스가 뜨고 완료 알림이 나오며, `.old`가 없는지 확인한다.
  - 결과를 ledger에 적는다.
- [ ] **Step 6: Commit** `feat(app): wire the update tile, progress, swap, and restart`

---

### Task 6: 릴리스 워크플로와 문서

**Files:**
- Create: `.github/workflows/release.yml`
- Modify: `README.md`, `docs/manual-test.md`, `docs/superpowers/specs/2026-10-20-swisswheel-design.md`, `docs/superpowers/specs/2026-10-21-desktop-menu-design.md`

- [ ] **Step 1: `release.yml`**
  - 트리거와 권한: `on: push: tags: ['v*']`, `permissions: contents: write`, `runs-on: windows-latest`, `shell: pwsh`
  - 단계: checkout → rust-toolchain stable
  - 버전 확인: `$tag = "${{ github.ref_name }}".TrimStart('v')`가 `cargo metadata --no-deps --format-version 1`의 `swisswheel` 패키지 버전과 다르면 `exit 1`
  - 테스트: `choco install ffmpeg -y --no-progress` → `cargo test --workspace`
  - 빌드: `cargo build --release`, `Copy-Item target/release/swisswheel.exe .`
  - 체크섬: `"$((Get-FileHash swisswheel.exe -Algorithm SHA256).Hash.ToLower())  swisswheel.exe" | Out-File -Encoding ascii -NoNewline swisswheel.exe.sha256`
  - 릴리스: `gh release create $env:GITHUB_REF_NAME swisswheel.exe swisswheel.exe.sha256 --generate-notes` (env `GH_TOKEN: ${{ github.token }}`)
- [ ] **Step 2: 검사.** `parse_checksum`이 위 출력 꼴(`<hex>  swisswheel.exe`, 끝 줄바꿈 없음)을 받는지 Task 1 `checksum_forms`가 이미 덮는지 확인한다. YAML은 `python -c "import yaml,sys;yaml.safe_load(open(sys.argv[1]))" .github/workflows/release.yml`로 확인한다. Expected: 오류 없음
- [ ] **Step 3: 문서**
  - README: 업데이트 타일 단락, "새 버전 내기" 절(스펙 3.5), 미구현 목록에서 업데이트 제거
  - manual-test: M-U1~M-U8 (스펙 5 수동 항목)
  - 본 설계 비목표에서 "자동 업데이트" 제거 + 링크, 메뉴 설계 업데이트 타일 행 → `2026-10-22-updater-design.md`
- [ ] **Step 4: Run** `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` — Expected: PASS
- [ ] **Step 5: Commit** `ci: publish releases from version tags; docs: updater`
