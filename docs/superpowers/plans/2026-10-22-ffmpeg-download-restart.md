# FFmpeg 다운로드·재시작 타일 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 메뉴 바에 "다운로드"(FFmpeg 받아 설치)와 "재시작"(SwissWheel 다시 켜기) 타일을 더한다.

**Architecture:** 판단·문구·JSON 해석은 `swisswheel-core`의 새 모듈 `ffmpeg_release`, `restart`에 둔다. 앱 쪽 `ffmpeg_install.rs`는 업데이터와 같은 모양의 백그라운드 스레드로, 기존 `update::http`(파일로 바로 쓰는 `get_to` 추가)와 `update::Ui`를 재사용한다. 재시작은 업데이트의 재시작 핸드오프(`--after-update <pid>` → 옛 프로세스 종료 대기 → 뮤텍스)를 `--restarted <pid>`로 재사용한다.

**Tech Stack:** Rust 2024, `windows` 0.61 (WinHTTP, MessageBoxW), `sha2`, `serde_json`, 새로 `zip` 9.

**Spec:** `docs/superpowers/specs/2026-10-22-ffmpeg-download-restart-design.md`

## Global Constraints

- 타일 순서: 설정 · 다운로드 · 플러그인 · 업데이트 · 재시작.
- FFmpeg 엔드포인트: `https://api.github.com/repos/GyanD/codexffmpeg/releases/latest`. 자산은 이름이 `-essentials_build.zip`으로 끝나는 것. 디버그 빌드만 `SWISSWHEEL_FFMPEG_URL`로 바꿀 수 있다.
- 해시는 자산의 `digest`(`sha256:<64자 16진수>`)와 비교한다. 맞기 전에는 아무것도 풀지 않는다.
- 크기 상한: 릴리스 JSON 1MB, zip 512MB. zip은 메모리가 아니라 파일로 받는다.
- 설치 위치: `<exe 폴더>\ffmpeg\`에 쓸 수 있으면 거기, 아니면 `%LOCALAPPDATA%\SwissWheel\ffmpeg\`. 임시 폴더는 같은 부모의 `ffmpeg.partial\`.
- `locate()` 순서: `<exe 폴더>\ffmpeg\` → `%LOCALAPPDATA%\SwissWheel\ffmpeg\` → `PATH`. 한 폴더에 `ffmpeg.exe`와 `ffprobe.exe`가 둘 다 있어야 인정한다.
- 확인 창 제목 "SwissWheel 다운로드", 알림 제목 "다운로드"·"재시작".
- 새 의존성: `zip = { version = "9", default-features = false, features = ["deflate-flate2-zlib-rs"] }`(읽기·테스트용 쓰기 둘 다 이 기능으로 된다).
- 로그·알림·UI 문구는 한국어, 코드 주석은 영어(기존 파일과 같게).
- 모든 작업 뒤 `cargo test --workspace`와 `cargo clippy --workspace --all-targets -- -D warnings`가 통과해야 한다.

**문구 (스펙 2.2–2.4, 그대로 쓴다):**

| 상수 | 문구 |
|---|---|
| `ffmpeg_release::text::READY` | 다운로드 완료 · FFmpeg가 이미 준비되어 있습니다 |
| `ffmpeg_release::text::RUNNING` | 이미 업데이트나 다운로드를 진행하는 중입니다 |
| `ffmpeg_release::text::CHECK_FAILED` | FFmpeg 정보를 확인할 수 없습니다 |
| `ffmpeg_release::text::NO_ASSET` | 받을 FFmpeg 파일을 찾을 수 없습니다 |
| `ffmpeg_release::text::DOWNLOAD_FAILED` | FFmpeg를 받지 못했습니다 |
| `ffmpeg_release::text::CORRUPT` | 받은 FFmpeg 파일이 손상되었습니다 |
| `ffmpeg_release::text::INSTALL_FAILED` | FFmpeg를 설치하지 못했습니다 |
| `ffmpeg_release::text::installed("9.0.2")` | FFmpeg 9.0.2 설정을 마쳤습니다 |
| `restart::text::BACKGROUND` | 업데이트나 다운로드가 끝난 뒤 다시 눌러 주세요 |
| `restart::text::RESTARTED` | 다시 시작했습니다 |
| `restart::text::FAILED` | 다시 시작하지 못했습니다 |

변환 중 재시작 거절은 기존 `update::text::BUSY`("진행 중인 변환이 끝난 뒤 다시 눌러 주세요")를 쓴다. 설치 완료 문구는 태그 끝 숫자에 따라 조사(을/를)가 바뀌지 않도록 스펙의 "…를 설정했습니다" 대신 "… 설정을 마쳤습니다"로 한다. Task 7에서 스펙 문구도 맞춘다.

## Review Focus

1. **같은 릴리스에 `.7z`·`full_build.zip` 같은 자산이 섞여 있음.** 정확히 `-essentials_build.zip`만 골라야 한다(Task 2 `picks_only_the_essentials_zip`).
2. **사용자 폴더 이름이 한글이거나 공백이 있음**(`C:\Users\김지훈\AppData\Local`). 설치와 찾기가 그대로 돼야 한다(Task 5 `installs_into_a_unicode_folder`).
3. **받는 도중 디스크가 꽉 차서 쓰기가 실패함.** `ffmpeg.partial`이 남지 않고 "받지 못했습니다"가 떠야 한다(Task 3 `sink_error_stops_the_download`, Task 5 `failures_leave_no_partial`).
4. **지난번에 꺼지면서 남은 `ffmpeg.partial`이 있는 채로 다시 받음.** 이전 찌꺼기가 섞이지 않아야 한다(Task 5 `stale_partial_is_replaced`).
5. **업데이트 확인 창이 떠 있는 동안 다운로드 타일을 누름, 또는 그 반대.** 둘이 동시에 돌지 않아야 한다(Task 2 판단 테스트, Task 6에서 `on_tile`에 두 상태를 합쳐 넘김).

---

### Task 1: 메뉴에 두 타일 추가

**Files:**
- Modify: `crates/swisswheel-core/src/menu.rs` (`Tile`, `TILES`, `label`, tests)
- Modify: `crates/swisswheel/src/menubar/render.rs:36-38` (`glyph`)

**Interfaces:**
- Produces: `Tile::Download`, `Tile::Restart`; `TILES: [Tile; 5] = [Settings, Download, Plugins, Update, Restart]`; 라벨 "다운로드", "재시작". 새 타일은 기존 `MenuAction::Notice(tile)`로 나간다(앱은 Task 6·7 전까지 "다음 버전에서 지원 예정입니다"를 띄운다).

- [ ] **Step 1: 테스트를 새 순서에 맞춰 고친다**

`starts_on_settings_and_stops_at_ends`: `Right`가 네 번 `Redraw`, 다섯 번째는 `None`, `m.tile() == 4`.
`enter_or_down_runs_the_tile`을 다음으로 바꾼다:
```rust
let mut m = MenuSession::new();
m.key(MenuKey::Right);
assert_eq!(m.key(MenuKey::Enter), MenuAction::Notice(Tile::Download));
m.key(MenuKey::Right);
assert_eq!(m.key(MenuKey::Enter), MenuAction::Notice(Tile::Plugins));
m.key(MenuKey::Right);
assert_eq!(m.key(MenuKey::Down), MenuAction::Notice(Tile::Update));
m.key(MenuKey::Right);
assert_eq!(m.key(MenuKey::Enter), MenuAction::Notice(Tile::Restart));
```
새 테스트 `tile_order_and_labels`:
```rust
assert_eq!(TILES, [Tile::Settings, Tile::Download, Tile::Plugins, Tile::Update, Tile::Restart]);
assert_eq!(Tile::Download.label(), "다운로드");
assert_eq!(Tile::Restart.label(), "재시작");
```

- [ ] **Step 2: 실패 확인**

Run: `cargo test -p swisswheel-core menu::`
Expected: 컴파일 실패 (`Tile::Download` 없음)

- [ ] **Step 3: `Tile`·`TILES`·`label`을 고치고, `render.rs`의 `glyph`에 `Tile::Download => "\u{E896}"` (Download), `Tile::Restart => "\u{E72C}"` (Refresh)를 더한다**

- [ ] **Step 4: 통과 확인**

Run: `cargo test --workspace` 그리고 `cargo clippy --workspace --all-targets -- -D warnings`
Expected: 모두 PASS (`menu_geometry` 테스트가 타일 수를 고정해 두었으면 다섯 칸에 맞춰 값을 고친다)

- [ ] **Step 5: 커밋**

```bash
git add crates/swisswheel-core/src/menu.rs crates/swisswheel/src/menubar/render.rs crates/swisswheel-core/src/menu_geometry.rs
git commit -m "feat(menu): add download and restart tiles"
```

---

### Task 2: core — FFmpeg 릴리스 해석과 두 타일의 판단

**Files:**
- Create: `crates/swisswheel-core/src/ffmpeg_release.rs`
- Create: `crates/swisswheel-core/src/restart.rs`
- Modify: `crates/swisswheel-core/src/lib.rs` (`pub mod ffmpeg_release; pub mod restart;`)

**Interfaces:**
- Consumes: `update::parse_checksum(&str) -> Option<[u8; 32]>`, `update::TileDecision { Start, Refuse(&'static str) }`, `update::text::BUSY`.
- Produces (`ffmpeg_release`):
  - `pub const ENDPOINT: &str`, `pub const ZIP_SUFFIX: &str = "-essentials_build.zip"`, `pub const TITLE: &str = "다운로드"`, `pub const DIALOG_TITLE: &str = "SwissWheel 다운로드"`
  - `pub struct FfmpegRelease { pub tag: String, pub zip_url: String, pub size: u64, pub sha256: [u8; 32] }` (`Debug, Clone, PartialEq`)
  - `pub enum ReleaseError { Unreadable, NoAsset }` (`Debug, PartialEq`)
  - `pub fn parse_release(json: &str) -> Result<FfmpegRelease, ReleaseError>`
  - `pub fn parse_digest(digest: &str) -> Option<[u8; 32]>`
  - `pub fn prompt_body(tag: &str, size: u64, dir: &std::path::Path) -> String`
  - `pub fn on_download_tile(ready: bool, busy: bool) -> TileDecision`
  - `pub mod text { READY, RUNNING, CHECK_FAILED, NO_ASSET, DOWNLOAD_FAILED, CORRUPT, INSTALL_FAILED, fn installed(tag: &str) -> String }`
- Produces (`restart`):
  - `pub const TITLE: &str = "재시작"`, `pub const ARG: &str = "--restarted"`
  - `pub fn restarted_pid<I: IntoIterator<Item = String>>(args: I) -> Option<u32>`
  - `pub fn on_tile(active_jobs: usize, background: bool) -> TileDecision` (`background` = 업데이트·다운로드 스레드나 교체 대기 중)
  - `pub mod text { BACKGROUND, RESTARTED, FAILED }`

- [ ] **Step 1: 실패하는 테스트를 쓴다 (`ffmpeg_release.rs`)**

```rust
const HEX: &str = "60f467265b1e312373dbcd92200c2618a74850f98d3d078e94296bb3fa2047ba";

fn asset(name: &str, digest: Option<&str>) -> String {
    let digest = digest.map_or("null".into(), |d| format!("\"{d}\""));
    format!(r#"{{"name":"{name}","size":114768076,"digest":{digest},"browser_download_url":"https://x/{name}"}}"#)
}

fn json(assets: &[String]) -> String {
    format!(r#"{{"tag_name":"9.0.2","assets":[{}]}}"#, assets.join(","))
}

#[test]
fn picks_only_the_essentials_zip() {
    let j = json(&[
        asset("ffmpeg-9.0.2-essentials_build.7z", Some(&format!("sha256:{HEX}"))),
        asset("ffmpeg-9.0.2-full_build.zip", Some(&format!("sha256:{HEX}"))),
        asset("ffmpeg-9.0.2-essentials_build.zip", Some(&format!("sha256:{HEX}"))),
    ]);
    let r = parse_release(&j).unwrap();
    assert_eq!(r.tag, "9.0.2");
    assert_eq!(r.zip_url, "https://x/ffmpeg-9.0.2-essentials_build.zip");
    assert_eq!(r.size, 114768076);
    assert_eq!(r.sha256[0], 0x60);
}

#[test]
fn missing_zip_or_digest_is_no_asset() {
    let no_zip = json(&[asset("ffmpeg-9.0.2-essentials_build.7z", Some(&format!("sha256:{HEX}")))]);
    assert_eq!(parse_release(&no_zip), Err(ReleaseError::NoAsset));
    let no_digest = json(&[asset("ffmpeg-9.0.2-essentials_build.zip", None)]);
    assert_eq!(parse_release(&no_digest), Err(ReleaseError::NoAsset));
    assert_eq!(parse_release("not json"), Err(ReleaseError::Unreadable));
    assert_eq!(parse_release(r#"{"assets":[]}"#), Err(ReleaseError::Unreadable)); // no tag
}

#[test]
fn digest_forms() {
    assert!(parse_digest(&format!("sha256:{HEX}")).is_some());
    assert_eq!(parse_digest(&format!("sha512:{HEX}")), None);
    assert_eq!(parse_digest(HEX), None);
    assert_eq!(parse_digest("sha256:abc"), None);
}

#[test]
fn prompt_rounds_megabytes() {
    let body = prompt_body("9.0.2", 114_768_076, Path::new(r"D:\앱\ffmpeg"));
    assert_eq!(body, "FFmpeg 9.0.2(약 115MB)를 받아 설정할까요?\n\n설치 위치: D:\\앱\\ffmpeg");
}

#[test]
fn download_tile() {
    assert_eq!(on_download_tile(true, true), TileDecision::Refuse(text::READY));
    assert_eq!(on_download_tile(false, true), TileDecision::Refuse(text::RUNNING));
    assert_eq!(on_download_tile(false, false), TileDecision::Start);
    assert_eq!(text::installed("9.0.2"), "FFmpeg 9.0.2 설정을 마쳤습니다");
}
```

`restart.rs` 테스트:
```rust
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
```

- [ ] **Step 2: 실패 확인**

Run: `cargo test -p swisswheel-core ffmpeg_release:: restart::`
Expected: 컴파일 실패 (모듈 없음)

- [ ] **Step 3: 두 모듈을 구현한다**

- `parse_release`: `serde_json::Value`로 읽는다. `tag_name` 문자열이 없으면 `Unreadable`. `assets`에서 `name`이 `ZIP_SUFFIX`로 끝나는 첫 항목을 찾고, `browser_download_url`·`size`·`digest`(→ `parse_digest`) 중 하나라도 없으면 `NoAsset`.
- `parse_digest`: `strip_prefix("sha256:")` 뒤를 `update::parse_checksum`에 넘긴다.
- `prompt_body`: MB는 `(size + 500_000) / 1_000_000`, 경로는 `dir.display()`.
- `restarted_pid`: `update::after_update_pid`와 같은 방식이고 `ARG`를 찾는다.

- [ ] **Step 4: 통과 확인**

Run: `cargo test -p swisswheel-core`
Expected: PASS

- [ ] **Step 5: 커밋**

```bash
git add crates/swisswheel-core/src/ffmpeg_release.rs crates/swisswheel-core/src/restart.rs crates/swisswheel-core/src/lib.rs
git commit -m "feat(core): FFmpeg release parsing and download/restart tile rules"
```

---

### Task 3: http — 본문을 파일로 바로 쓰는 `get_to`

**Files:**
- Modify: `crates/swisswheel/src/update/http.rs`
- Modify: `crates/swisswheel/src/update/mod.rs:5-6` (`mod test_server` → `pub(crate) mod test_server`, Task 5 테스트가 쓴다)

**Interfaces:**
- Produces:
  - `pub fn get_to(url: &str, limit: usize, sink: &mut dyn std::io::Write, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<u64, HttpError>`: 받은 바이트 수를 돌려준다.
  - `HttpError::Io(#[from] std::io::Error)`, 표시 문구 `"쓰기 오류: {0}"`.
  - `get`은 시그니처를 그대로 두고 `Vec`을 sink로 하는 `get_to` 호출이 된다.

- [ ] **Step 1: 실패하는 테스트를 쓴다**

```rust
#[test]
fn get_to_streams_into_the_sink() {
    let body: Vec<u8> = (0..102400u32).map(|i| (i * 7) as u8).collect();
    let server = TestServer::start(vec![("/big", Reply::Ok(body.clone()))]);
    let mut sink = Vec::new();
    let mut last = None;
    let n = get_to(&format!("{}/big", server.base), 1 << 20, &mut sink, &mut |g, t| last = Some((g, t))).unwrap();
    assert_eq!((n, sink), (102400, body));
    assert_eq!(last, Some((102400, Some(102400))));
}

#[test]
fn sink_error_stops_the_download() {
    struct Full;
    impl std::io::Write for Full {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("disk full"))
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    let server = TestServer::start(vec![("/x", Reply::Ok(vec![1; 4096]))]);
    let result = get_to(&format!("{}/x", server.base), 1 << 20, &mut Full, &mut |_, _| {});
    assert!(matches!(result, Err(HttpError::Io(_))), "{result:?}");
}
```

- [ ] **Step 2: 실패 확인**

Run: `cargo test -p swisswheel update::http::`
Expected: 컴파일 실패 (`get_to` 없음)

- [ ] **Step 3: 읽기 루프를 고정 64KB 버퍼 → `sink.write_all`로 바꾸고, 상한·잘림 검사는 받은 바이트 수로 한다. `get`은 `get_to`에 위임한다**

- [ ] **Step 4: 통과 확인**

Run: `cargo test -p swisswheel update::`
Expected: 기존 http·업데이트 테스트를 포함해 모두 PASS

- [ ] **Step 5: 커밋**

```bash
git add crates/swisswheel/src/update/http.rs crates/swisswheel/src/update/mod.rs
git commit -m "feat(http): stream a download into any writer"
```

---

### Task 4: FFmpeg 찾기 — 설치 폴더 두 곳

**Files:**
- Modify: `crates/swisswheel/src/engines/ffmpeg.rs:40-56` 와 `locate_in` 테스트(약 405-435행)

**Interfaces:**
- Produces:
  - `pub fn install_dirs() -> Vec<PathBuf>`: `[<exe 폴더>\ffmpeg, %LOCALAPPDATA%\SwissWheel\ffmpeg]` 순서. exe 위치나 `LOCALAPPDATA`를 모르면 그 항목을 뺀다.
  - `pub fn locate_in(dirs: &[PathBuf], path_var: Option<&OsStr>) -> Option<Tools>`: 이제 `dirs`는 ffmpeg 폴더 자체다(`app_dir.join("ffmpeg")`를 하지 않는다).
  - `pub fn locate() -> Option<Tools>` = `locate_in(&install_dirs(), PATH)`.

- [ ] **Step 1: `locate_in` 테스트를 새 시그니처로 고치고 순서 검사를 더한다**

```rust
#[test]
fn locate_prefers_install_dirs_in_order() {
    let (app, local, path) = (tempdir(), tempdir(), tempdir());
    let fill = |d: &Path, names: &[&str]| {
        std::fs::create_dir_all(d).unwrap();
        for n in names { std::fs::write(d.join(n), b"").unwrap(); }
    };
    let both = ["ffmpeg.exe", "ffprobe.exe"];
    fill(path.path(), &both);
    let path_var = std::env::join_paths([path.path()]).unwrap();
    let dirs = [app.path().join("ffmpeg"), local.path().join("ffmpeg")];
    assert_eq!(locate_in(&dirs, Some(&path_var)).unwrap().ffmpeg, path.path().join("ffmpeg.exe"));
    fill(&dirs[1], &both);
    assert_eq!(locate_in(&dirs, Some(&path_var)).unwrap().ffmpeg, dirs[1].join("ffmpeg.exe"));
    fill(&dirs[0], &["ffmpeg.exe"]); // no ffprobe: skipped
    assert_eq!(locate_in(&dirs, Some(&path_var)).unwrap().ffmpeg, dirs[1].join("ffmpeg.exe"));
    fill(&dirs[0], &both);
    assert_eq!(locate_in(&dirs, Some(&path_var)).unwrap().ffprobe, dirs[0].join("ffprobe.exe"));
}
```
(`tempdir`는 `tempfile::tempdir().unwrap()`이다.) 기존 `locate_in` 테스트는 이 테스트로 대신한다.

- [ ] **Step 2: 실패 확인**

Run: `cargo test -p swisswheel engines::ffmpeg::tests::locate`
Expected: 컴파일 실패 (시그니처 다름)

- [ ] **Step 3: `install_dirs`, `locate_in`, `locate`를 구현한다**

`jobs.rs:111`의 `ffmpeg::locate()` 호출은 그대로 둔다.

- [ ] **Step 4: 통과 확인**

Run: `cargo test -p swisswheel engines::`
Expected: PASS

- [ ] **Step 5: 커밋**

```bash
git add crates/swisswheel/src/engines/ffmpeg.rs
git commit -m "feat(ffmpeg): look in the app's own install folders before PATH"
```

---

### Task 5: FFmpeg 받기·설치 스레드

**Files:**
- Create: `crates/swisswheel/src/ffmpeg_install.rs`
- Modify: `crates/swisswheel/src/main.rs` (`mod ffmpeg_install;`)
- Modify: `crates/swisswheel/Cargo.toml` (`zip` 의존성, Global Constraints의 줄)
- Modify: `crates/swisswheel/src/update/mod.rs` (`MessageBoxUi`가 제목을 받게)

**Interfaces:**
- Consumes: `http::get`, `http::get_to` (Task 3), `update::Ui`, `ffmpeg_release::*` (Task 2), `engines::ffmpeg::install_dirs` (Task 4).
- Produces:
  - `pub struct MessageBoxUi { pub title: &'static str }` (`update/mod.rs`). 업데이터 `spawn`은 `MessageBoxUi { title: "SwissWheel 업데이트" }`를 쓴다.
  - `pub enum Outcome { Installed { tag: String }, CheckFailed, MissingAsset, DownloadFailed, Corrupt, InstallFailed, Declined }` (`Debug, PartialEq`)
  - `#[derive(Default)] pub struct Progress { pub fraction: AtomicU32, pub tag: Mutex<Option<String>> }`: 사용자가 [예]를 누른 뒤 `tag`가 채워진다.
  - `pub fn run(endpoint: &str, dirs: &[PathBuf], ui: &dyn Ui, progress: &Progress) -> Outcome`
  - `pub fn partial_path(target: &Path) -> PathBuf`: 같은 부모의 `ffmpeg.partial`.
  - `pub fn cleanup(dirs: &[PathBuf])`: 각 `partial_path`를 지운다. 실패하면 `log_error!`만 남긴다.
  - `pub fn endpoint() -> String`: 디버그에서 `SWISSWHEEL_FFMPEG_URL`, 아니면 `ffmpeg_release::ENDPOINT`.
  - `pub struct Handle { pub progress: Arc<Progress> }`, `pub fn spawn(hwnd: isize, msg: u32, slot: Arc<Mutex<Option<Outcome>>>) -> Handle`: `update::spawn`과 같은 모양이고 `run(&endpoint(), &install_dirs(), &MessageBoxUi { title: DIALOG_TITLE }, …)`을 부른다.

**`run` 순서 (스펙 3.3):**
1. JSON을 받는다(`get`, 1MB) → `parse_release`. 네트워크·해석 실패는 `CheckFailed`, `NoAsset`은 `MissingAsset`.
2. `pick_target(dirs)`: 부모 폴더를 `create_dir_all`할 수 있고 거기에 탐침 파일을 만들었다 지울 수 있는 첫 `dir`. 없으면 `InstallFailed`.
3. `ui.confirm(&prompt_body(tag, size, &target))`가 아니면 `Declined`. 맞으면 `progress.tag`를 채운다.
4. `partial`을 지우고(`remove_dir_all`, 없어도 됨) 새로 만든다. `partial\download.zip`에 `get_to`(512MB)로 받는다. sink는 파일에 쓰면서 `Sha256`을 갱신하는 작은 `struct HashingWriter<W>`. 실패는 `DownloadFailed`, 해시가 다르면 `Corrupt`.
5. `extract(&zip, &partial)` 실패는 `InstallFailed`.
6. `target`을 `create_dir_all`하고, `ffprobe.exe` → `LICENSE`(있으면) → `ffmpeg.exe` 순서로 `std::fs::rename`. 실패는 `InstallFailed`.
7. 결과와 상관없이 `Declined`·`CheckFailed`·`MissingAsset`이 아니면 마지막에 `partial`을 지운다. `Installed { tag }`.

- `fn extract(zip: &Path, into: &Path) -> io::Result<()>`: 항목 이름을 `/`로 나눠 정확히 `[_, "bin", "ffmpeg.exe"]`, `[_, "bin", "ffprobe.exe"]`, `[_, "LICENSE"]`인 것만 `into.join(<마지막 조각>)`에 `io::copy`한다. 두 exe 중 하나라도 없으면 `io::ErrorKind::InvalidData`.

- [ ] **Step 1: 실패하는 테스트를 쓴다**

테스트 도우미: `zip::ZipWriter`로 메모리에 zip을 만드는 `fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8>`, `TestServer`에 `/latest`(자산 `ffmpeg-1.0-essentials_build.zip`, `size`, `digest: "sha256:<그 zip의 해시>"`, url `{base}/zip`)와 `/zip`을 올리는 `fn serve(zip: Vec<u8>, digest_of: &[u8]) -> TestServer`, 업데이터 테스트의 `FakeUi`와 같은 가짜 UI.

```rust
const GOOD: &[(&str, &[u8])] = &[
    ("ffmpeg-1.0-essentials_build/bin/ffmpeg.exe", b"MZ ffmpeg"),
    ("ffmpeg-1.0-essentials_build/bin/ffprobe.exe", b"MZ ffprobe"),
    ("ffmpeg-1.0-essentials_build/bin/ffplay.exe", b"MZ ffplay"),
    ("ffmpeg-1.0-essentials_build/LICENSE", b"GPL"),
    ("ffmpeg-1.0-essentials_build/doc/faq.html", b"<html>"),
    ("../evil.exe", b"MZ evil"),
];

#[test]
fn installs_only_the_tools() {
    let root = tempfile::tempdir().unwrap();
    let dirs = [root.path().join("app").join("ffmpeg"), root.path().join("local").join("ffmpeg")];
    std::fs::create_dir(root.path().join("app")).unwrap();
    let zip = make_zip(GOOD);
    let server = serve(zip.clone(), &zip);
    let ui = FakeUi::new(true);
    let progress = Progress::default();
    assert_eq!(run(&format!("{}/latest", server.base), &dirs, &ui, &progress),
               Outcome::Installed { tag: "1.0".into() });
    let mut names: Vec<_> = std::fs::read_dir(&dirs[0]).unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    names.sort();
    assert_eq!(names, ["LICENSE", "ffmpeg.exe", "ffprobe.exe"]);
    assert_eq!(std::fs::read(dirs[0].join("ffmpeg.exe")).unwrap(), b"MZ ffmpeg");
    assert!(!partial_path(&dirs[0]).exists());
    assert!(!root.path().join("evil.exe").exists() && !root.path().join("app").join("evil.exe").exists());
    assert_eq!(progress.fraction.load(Ordering::Relaxed), 10000);
    assert!(ui.confirms.borrow()[0].contains(&dirs[0].display().to_string()));
    assert!(crate::engines::ffmpeg::locate_in(&dirs, None).is_some());
}

#[test]
fn installs_into_a_unicode_folder() { /* installs_only_the_tools 와 같되 root 아래 "김 지훈" 폴더를 쓰고, Installed 와 locate_in 성공만 확인 */ }

#[test]
fn falls_back_when_the_app_folder_is_missing() {
    // dirs[0] 의 부모가 없는 경로(파일 아래 경로)면 dirs[1] 에 설치된다
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("file"), b"").unwrap();
    let dirs = [root.path().join("file").join("ffmpeg"), root.path().join("local").join("ffmpeg")];
    let zip = make_zip(GOOD);
    let server = serve(zip.clone(), &zip);
    assert!(matches!(run(&format!("{}/latest", server.base), &dirs, &FakeUi::new(true), &Progress::default()),
                     Outcome::Installed { .. }));
    assert!(dirs[1].join("ffprobe.exe").is_file());
}

#[test]
fn failures_leave_no_partial() {
    // (a) digest 가 다른 zip → Corrupt, (b) ffprobe 없는 zip → InstallFailed,
    // (c) /zip 이 Reply::Drop → DownloadFailed. 각 경우 dirs[0] 와 partial_path(&dirs[0]) 가 없다.
}

#[test]
fn declined_or_missing_touches_nothing() {
    // FakeUi::new(false) → Declined; essentials zip 없는 JSON → MissingAsset; /latest 가 500 → CheckFailed.
    // 각 경우 dirs[0] 와 partial 이 없다.
}

#[test]
fn stale_partial_is_replaced() {
    // partial_path(&dirs[0]) 에 "old.txt" 를 만들어 두고 설치 → Installed, partial 없음, dirs[0] 에 old.txt 없음.
}

#[test]
fn completes_a_half_installed_folder() {
    // dirs[0] 에 ffmpeg.exe(b"old")만 둔 채 설치 → 두 exe 모두 새 내용.
}

#[test]
fn cleanup_removes_partials() {
    // 두 dir 의 partial 폴더를 만들고 cleanup(&dirs) → 둘 다 없음. 없는 경우에도 패닉 없음.
}
```

- [ ] **Step 2: 실패 확인**

Run: `cargo test -p swisswheel ffmpeg_install::`
Expected: 컴파일 실패 (모듈 없음)

- [ ] **Step 3: `Cargo.toml`에 `zip`을 더하고, `MessageBoxUi`에 `title`을 넣고, `ffmpeg_install.rs`를 위 순서대로 구현한다**

모든 단계(요청 URL, 고른 자산, 설치 위치, 받은 크기, 해시 결과, 각 이동)를 `log_info!`/`log_error!`로 남긴다(업데이터와 같게).

- [ ] **Step 4: 통과 확인**

Run: `cargo test --workspace` 그리고 `cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS

- [ ] **Step 5: 커밋**

```bash
git add Cargo.lock crates/swisswheel/Cargo.toml crates/swisswheel/src/ffmpeg_install.rs crates/swisswheel/src/main.rs crates/swisswheel/src/update/mod.rs
git commit -m "feat(ffmpeg): download, verify, and install FFmpeg in the background"
```

---

### Task 6: 앱 — 다운로드 타일 연결

**Files:**
- Modify: `crates/swisswheel/src/app.rs`
- Modify: `docs/manual-test.md`, `README.md:30`

**Interfaces:**
- Consumes: `ffmpeg_install::{spawn, Handle, Outcome, cleanup}`, `engines::ffmpeg::{locate, install_dirs}`, `ffmpeg_release::{on_download_tile, text, TITLE}`.
- Produces (Task 7이 쓴다): `AppState.ffmpeg: Option<ffmpeg_install::Handle>`.

- [ ] **Step 1: 상태와 메시지를 더한다**

`const WM_APP_FFMPEG: u32 = WM_APP + 32;`, `AppState`에 `ffmpeg: Option<ffmpeg_install::Handle>`와 `ffmpeg_slot: Arc<Mutex<Option<ffmpeg_install::Outcome>>>`를 두고, `wnd_proc`에서 `WM_APP_FFMPEG => with_state(AppState::on_ffmpeg_done)`.

- [ ] **Step 2: 타일·결과를 처리한다**

- `on_menu_action`: `MenuAction::Notice(Tile::Download) => { self.close_menu(); self.start_ffmpeg(); }`.
- `fn start_ffmpeg(&mut self)`: `on_download_tile(ffmpeg::locate().is_some(), self.update.is_some() || self.ffmpeg.is_some())`. `Refuse(t)`이면 `tray.notify(ffmpeg_release::TITLE, t)`, `Start`이면 `spawn`한 뒤 `update_busy()`.
- `fn on_ffmpeg_done(&mut self)`: 슬롯을 꺼낸다. `Installed { tag }`는 `notify(installed(&tag))`, `Declined`는 아무것도 안 하고, 나머지는 각 `text::*`로 `warn`. 끝에 `update_busy()`.
- `start_update`: `release::on_tile(self.update.is_some() || self.ffmpeg.is_some(), self.active_jobs)`로 넘긴다.
- `update_busy`: FFmpeg 행을 업데이트 행 옆에 만든다. `progress.tag`가 `Some(tag)`일 때만 `ProgressRow { title: format!("FFmpeg {tag} · 다운로드"), fraction }`. 툴팁은 변환이 없을 때 `"SwissWheel · FFmpeg 받는 중 · {n}%"`. 타이머 유지 조건에 `self.ffmpeg.is_some()`을 더한다.
- `run()` 시작부, `swap::cleanup` 옆: `ffmpeg_install::cleanup(&crate::engines::ffmpeg::install_dirs())`.
- `preview`에 `"ffmpeg" => self.start_ffmpeg()`(디버그 전용, `SWISSWHEEL_FFMPEG_URL`로 시험).

- [ ] **Step 3: 빌드·테스트**

Run: `cargo test --workspace` 그리고 `cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS

- [ ] **Step 4: 문서**

- `README.md:30`을 "Video and audio need FFmpeg: open the menu and choose **다운로드** to fetch and set it up, or put `ffmpeg.exe` and `ffprobe.exe` in an `ffmpeg` folder next to `swisswheel.exe`, or have them on `PATH`."로 바꾼다.
- `docs/manual-test.md` 표에 M49–M53을 더한다:
  - M49: PATH에서 FFmpeg를 빼고 exe 폴더의 `ffmpeg\`를 지운 뒤 메뉴 → 다운로드 → [예]. 기대: 진행 창에 "FFmpeg <버전> · 다운로드" 줄, "FFmpeg <버전> 설정을 마쳤습니다" 알림, `ffmpeg\`에 `ffmpeg.exe`·`ffprobe.exe`·`LICENSE`, 재시작 없이 MP4 → MP3가 됨.
  - M50: M49 직후 다시 다운로드. 기대: "다운로드 완료 · FFmpeg가 이미 준비되어 있습니다", 로그에 요청 없음.
  - M51: 받는 도중 트레이 → 종료 → 다시 실행. 기대: `ffmpeg.partial` 없음.
  - M52: 디버그 + `SWISSWHEEL_FFMPEG_URL`(digest를 틀리게 한 로컬 JSON). 기대: "받은 FFmpeg 파일이 손상되었습니다", `ffmpeg\` 없음.
  - M53: exe를 `C:\Program Files\SwissWheel`에 두고 M49. 기대: `%LOCALAPPDATA%\SwissWheel\ffmpeg\`에 설치되고 변환이 됨.

- [ ] **Step 5: 커밋**

```bash
git add crates/swisswheel/src/app.rs README.md docs/manual-test.md
git commit -m "feat(app): wire the download tile to the FFmpeg installer"
```

---

### Task 7: 앱 — 재시작 타일

**Files:**
- Modify: `crates/swisswheel/src/update/swap.rs` (`launch_restarted` + test)
- Modify: `crates/swisswheel/src/app.rs`
- Modify: `docs/manual-test.md`, `README.md`, `docs/superpowers/specs/2026-10-21-desktop-menu-design.md`, `docs/superpowers/specs/2026-10-22-ffmpeg-download-restart-design.md` (설치 완료 문구)

**Interfaces:**
- Consumes: `restart::{on_tile, restarted_pid, text, TITLE, ARG}`, `AppState.ffmpeg` (Task 6).
- Produces: `pub fn launch_restarted(exe: &Path) -> io::Result<()>`: `exe --restarted <지금 pid>`를 띄운다.

- [ ] **Step 1: `swap.rs`에 실패하는 테스트를 쓴다**

`launch_after_update`를 일반화한 `fn launch_with(exe: &Path, arg: &str) -> io::Result<()>`를 두고 두 함수가 그것을 부른다. 테스트는 없는 exe 경로에 `launch_restarted`가 `Err`를 돌려주는지 본다:
```rust
#[test]
fn launch_restarted_reports_a_missing_exe() {
    let dir = tempfile::tempdir().unwrap();
    assert!(launch_restarted(&dir.path().join("없음.exe")).is_err());
}
```

- [ ] **Step 2: 실패 확인 후 구현**

Run: `cargo test -p swisswheel update::swap::`
Expected: 컴파일 실패 → `launch_with`/`launch_restarted`를 구현한 뒤 PASS

- [ ] **Step 3: 앱에 연결한다**

- `on_menu_action`: `MenuAction::Notice(Tile::Restart) => { self.close_menu(); self.restart(); }`. 그러면 남는 `Notice(tile)` 분기에는 `Tile::Plugins`만 들어온다.
- `fn restart(&mut self)`: `restart::on_tile(self.active_jobs, self.update.is_some() || self.ffmpeg.is_some() || self.pending_swap.is_some())`. `Refuse(t)`이면 `notify(restart::TITLE, t)`. `Start`이면 `current_exe().and_then(|exe| swap::launch_restarted(&exe))`가 `Ok`일 때 `log_info!("재시작합니다")` 후 `PostMessageW(WM_CLOSE)`(`apply_swap`과 같게), `Err`일 때 `log_error!` 후 `warn(restart::TITLE, restart::text::FAILED)`.
- `run()` 시작부: `let args: Vec<String> = std::env::args().collect();` `let after_update = release::after_update_pid(args.clone());` `let restarted = restart::restarted_pid(args);` 기다릴 pid는 `after_update.or(restarted)`이고, 같은 `swap::wait_for_exit(pid, AFTER_UPDATE_WAIT)`를 뮤텍스 전에 부른다. 로그는 "재시작 뒤 시작 (이전 프로세스 종료: {exited})". 트레이를 만든 뒤 `restarted.is_some()`이면 `notify(restart::TITLE, restart::text::RESTARTED)`.

- [ ] **Step 4: 빌드·테스트**

Run: `cargo test --workspace` 그리고 `cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS

- [ ] **Step 5: 문서**

- `docs/manual-test.md`:
  - M54: 메뉴 → 재시작. 기대: 트레이 아이콘이 잠깐 사라졌다 생기고 "다시 시작했습니다" 알림. 트레이 아이콘 하나, 설정 그대로, Run 레지스트리 값 그대로.
  - M55: 긴 영상 변환 중 재시작. 기대: "진행 중인 변환이 끝난 뒤 다시 눌러 주세요". FFmpeg 받는 중 재시작. 기대: "업데이트나 다운로드가 끝난 뒤 다시 눌러 주세요".
- README: 메뉴 설명에 다운로드·재시작 타일을 한 줄씩 더한다.
- 메뉴 설계 문서의 타일 목록을 다섯 개로 바꾸고 이 스펙을 링크한다.
- 이 스펙의 설치 완료 문구를 "FFmpeg 9.0.2 설정을 마쳤습니다"로 바꾼다.

- [ ] **Step 6: 커밋**

```bash
git add crates/swisswheel/src/update/swap.rs crates/swisswheel/src/app.rs README.md docs/
git commit -m "feat(app): restart tile that hands off to a fresh process"
```
