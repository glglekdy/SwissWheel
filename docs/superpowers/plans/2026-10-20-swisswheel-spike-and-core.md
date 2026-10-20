# SwissWheel 스파이크 + Core 크레이트 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 드래그-휠 방식이 윈도우에서 실제로 가능한지 스파이크로 검증하고, OS에 의존하지 않는 `swisswheel-core` 크레이트(포맷·카탈로그·휠 기하·출력 이름·설정)를 테스트와 함께 완성한다.

**Architecture:** Cargo 워크스페이스에 `swisswheel-core`(순수 로직 lib)와 `swisswheel`(Win32 bin, 이 계획에서는 빈 껍데기)를 둔다. 스파이크는 워크스페이스 밖 `spike/dragwheel/`의 버림 코드다. 2~6단계(훅·휠·dnd·엔진·동작·패키징)는 스파이크 결과를 반영해 **별도 계획**으로 작성한다.

**Tech Stack:** Rust stable (edition 2024, MSVC 툴체인), `windows` 크레이트(스파이크), `serde` 1 + `toml` 0.8, `thiserror` 2, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-10-20-swisswheel-design.md`

## Global Constraints

- 대상 OS: Windows 10 22H2 / Windows 11, x64.
- 라이선스: GPL-3.0-or-later (`Cargo.toml`의 `license = "GPL-3.0-or-later"`).
- `swisswheel-core`는 `windows` 크레이트나 어떤 OS API에도 의존하지 않는다 (의존성: `serde`, `toml`, `thiserror`만).
- UI 문구는 한국어 (휠 이름: 변환 / 도구 / 동작).
- 기본 키: `open_key = "Space"`, `cycle_key = "Tab"`, `cancel_key = "Escape"`.
- 휠 치수(DPI 배율 1.0 기준): 바깥 반지름 160px, 취소 원 반지름 48px, 고리당 최대 12개, 최대 24개.
- 품질 기본값: JPG 90, WebP 85, MP3 192kbps.
- 원본 파일은 절대 덮어쓰지 않는다 — 출력 경로는 입력 경로와 같아질 수 없다.
- CI: `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`.
- 모든 커밋 메시지 끝에 `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **대문자·변형 확장자** (`IMG_0001.JPG`, `a.jpeg`, `scan.TIF`, `song.aif`) — 정상 인식되어야 한다. → Task 2 `from_extension_is_case_insensitive_and_aliased`
2. **같은 포맷 여러 개 / 종류가 섞인 선택** (PNG+PNG, PNG+MP3, 파일+폴더, 확장자 없는 파일) — 같은 포맷이면 자기 자신만 빠지고, 섞이면 변환·도구 휠이 비고 동작 휠만 남아야 한다. → Task 3 `mixed_and_unknown_selections`
3. **까다로운 파일 이름** (`report.final.png`, 확장자 없는 `README`, 이미 `a (1).png`가 있음, 한글 `사진.png`) — 마지막 확장자만 바뀌고, 충돌 시 다음 번호를 쓰고, 유니코드가 보존되어야 한다. → Task 5 `tricky_names`
4. **섹터 경계와 고DPI** (0°/360° 이음매, 반지름 정확히 48·104·160, 배율 1.5) — 빈틈이나 겹침 없이 정확히 한 결과가 나와야 한다. → Task 4 `boundaries_and_seam`, `dpi_scale`
5. **잘못되거나 일부만 있는 설정 파일** (`jpg = 0`, `jpg = "high"`, `open_key`와 `cycle_key`가 같음, 모르는 키, 일부 키만 있음) — 잘못된 값은 오류, 모르는 키는 무시, 빠진 키는 기본값. → Task 6 `invalid_configs`, `partial_and_unknown_keys`

---

### Task 0: 개발 환경 준비

**Files:** 없음 (시스템 설치)

**Interfaces:**
- Produces: `cargo`, `rustc`(stable, `x86_64-pc-windows-msvc`), MSVC 링커, Windows SDK.

- [ ] **Step 1: rustup 설치**

Run: `winget install --id Rustlang.Rustup -e --accept-source-agreements --accept-package-agreements`
그 다음 새 셸에서: `rustup default stable-x86_64-pc-windows-msvc; rustup component add clippy rustfmt`

- [ ] **Step 2: MSVC 빌드 도구 확인/설치**

임시 폴더(스크래치패드)에서 `cargo new hello; cd hello; cargo run` 실행.
Expected: `Hello, world!`. `link.exe not found` 오류가 나면 `winget search BuildTools`로 최신 Visual Studio Build Tools ID를 찾아 `--override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"`로 설치한 뒤 다시 실행. (수 GB 다운로드 — 사용자 승인 후 진행)

- [ ] **Step 3: 확인**

Run: `cargo --version; rustc --version; cargo clippy --version`
Expected: 세 명령 모두 버전 출력.

---

### Task 1: 스파이크 — 드래그 중 휠 창 띄우기 검증 (버림 코드)

**Files:**
- Create: `spike/dragwheel/Cargo.toml`, `spike/dragwheel/src/main.rs` (워크스페이스에 포함하지 않음)
- Create: `docs/superpowers/spikes/2026-10-20-drag-spike.md` (결과 기록)

**Interfaces:**
- Produces: 결과 문서. 2단계 계획이 이 문서의 결론을 근거로 작성된다. 스파이크 코드는 재사용하지 않는다.

- [ ] **Step 1: 스파이크 프로그램 작성**

콘솔 앱(`println!`으로 로그, 각 로그에 `Instant` 기준 ms 타임스탬프). `windows` 크레이트 기능: `Win32_Foundation`, `Win32_UI_WindowsAndMessaging`, `Win32_UI_Input_KeyboardAndMouse`, `Win32_System_Ole`, `Win32_System_Com`, `Win32_UI_Shell`, `Win32_Graphics_Gdi`, `Win32_System_SystemServices`. 동작:
1. `OleInitialize` 후 `WH_MOUSE_LL`, `WH_KEYBOARD_LL` 설치. 콜백은 상태 갱신 + `PostMessageW`만.
2. 왼쪽 버튼 down 후 `SM_CXDRAG/SM_CYDRAG` 이상 이동 → `drag_candidate`. 이때 Space down → 키 삼킴, 커서 중심에 320×320 창 표시 (`WS_EX_LAYERED|WS_EX_TOPMOST|WS_EX_TOOLWINDOW|WS_EX_NOACTIVATE`, `SetLayeredWindowAttributes` 알파 200, GDI로 원 그리기면 충분).
3. 명령줄 인자 `--inject`가 있으면 창 표시 직후 `SendInput`으로 `MOUSEEVENTF_MOVE` (dx=0, dy=0) 주입.
4. 창에 `RegisterDragDrop` — `IDropTarget` 구현에서 `DragEnter`(창 표시 후 경과 ms, CF_HDROP 파일 목록), `DragOver`(좌표, 10회에 1번만), `DragLeave`, `Drop`(좌표, 파일 목록) 로그. `DragEnter/DragOver/Drop` 모두 `*pdwEffect = DROPEFFECT_COPY`.
5. 창 표시 후 300ms 안에 `DragEnter`가 없으면 창 닫고 `TIMEOUT` 로그.
6. 창이 열린 동안 Tab → 삼키고 `TAB` 로그, Esc → 삼키고 창 닫고 `ESC` 로그.
7. 왼쪽 버튼 up → 창 닫기.

- [ ] **Step 2: 빌드**

Run: `cargo build --manifest-path spike/dragwheel/Cargo.toml`
Expected: 경고는 허용, 오류 없이 빌드.

- [ ] **Step 3: 수동 검증 (S1~S9) 후 결과 문서 작성**

`cargo run --manifest-path spike/dragwheel/Cargo.toml` (그리고 `-- --inject`로 한 번 더) 실행 상태에서 테스트용 폴더 `D:\spiketest\a\`(테스트 파일 몇 개)와 `D:\spiketest\b\`를 만들어 확인. 각 항목에 PASS/FAIL, 관찰값을 기록:

| ID | 절차 | PASS 조건 |
|---|---|---|
| S1 | 탐색기에서 파일 드래그 중 Space | 창 표시. 탐색기 쪽에 Space 입력 흔적 없음 |
| S2 | S1에서 마우스를 움직이지 않음 (`--inject` 유/무 각각) | `--inject` 시 DragEnter가 300ms 안에 도착. 유/무 차이 ms 기록 |
| S3 | **같은 드라이브** 안에서 드래그(기본 동작이 이동) → 창에 드롭 | 파일 목록 로그, **원본이 `a\`에 그대로 있음**. Shift를 누른 채 드래그한 경우도 반복 |
| S4 | 창 열린 상태에서 Esc → 계속 드래그해서 `b\`에 드롭 | 창 닫힘, 드래그 유지, `b\`로 정상 이동/복사 |
| S5 | 창 열린 상태에서 Tab | `TAB` 로그, 탐색기 포커스 변화 없음 |
| S6 | 창 제목 표시줄 드래그 중 Space / 메모장 텍스트 선택 드래그 중 Space | 300ms 안에 `TIMEOUT`, 이후 정상 동작 |
| S7 | 바탕화면 아이콘을 드래그 중 Space → 드롭 | S1·S3과 같은 결과 |
| S8 | 1분간 드래그를 20회 이상 반복 | 커서 끊김 없음, 훅이 해제되지 않음 (이후에도 Space 감지됨) |
| S9 | 창이 열린 채 창 밖에서 버튼 놓기 | 창 닫힘, 원래 위치에 정상 드롭 |

결과 문서에는 표, 측정값, 그리고 **설계 변경 필요 사항**(없으면 "없음")을 적는다. FAIL 항목이 있으면 대안을 시도하고 그 결과도 기록한다 (예: S2 실패 → 휠을 커서에서 살짝 비켜서 표시, S3 실패 → `DROPEFFECT_LINK` 반환).

- [ ] **Step 4: 커밋**

```bash
git add spike/dragwheel/Cargo.toml spike/dragwheel/src/main.rs docs/superpowers/spikes/2026-10-20-drag-spike.md
git commit -m "spike: verify drag-time wheel window on Windows"
```

**게이트:** S1·S3·S4·S6 중 하나라도 대안 없이 FAIL이면 Task 2로 넘어가지 말고 사용자와 설계를 다시 논의한다. (Task 2~6은 OS에 의존하지 않지만, 근본 방식이 막히면 프로젝트 방향 자체가 바뀌기 때문이다.)

---

### Task 2: 워크스페이스 + CI + `Format`

**Files:**
- Create: `Cargo.toml` (workspace, `members = ["crates/*"]`, `exclude = ["spike"]`, `resolver = "3"`)
- Create: `rust-toolchain.toml` (`channel = "stable"`, components `clippy`, `rustfmt`)
- Create: `.gitignore` (`/target`, `spike/**/target`)
- Create: `LICENSE` (GPL-3.0 전문: `curl -o LICENSE https://www.gnu.org/licenses/gpl-3.0.txt`)
- Create: `README.md` (한 문단 소개 + 빌드 방법 `cargo build`)
- Create: `.github/workflows/ci.yml`
- Create: `crates/swisswheel-core/Cargo.toml`, `crates/swisswheel-core/src/lib.rs`, `crates/swisswheel-core/src/format.rs`
- Create: `crates/swisswheel/Cargo.toml`, `crates/swisswheel/src/main.rs` (`fn main() {}`, `swisswheel-core`에 path 의존)
- Test: `crates/swisswheel-core/src/format.rs` 안의 `#[cfg(test)] mod tests`

**Interfaces:**
- Produces:
  - `pub enum Format { Jpg, Png, Webp, Bmp, Tiff, Gif, Ico, Mp4, Mkv, Webm, Mov, Avi, Wmv, Mp3, M4a, Wav, Flac, Ogg, Opus, Wma, Aiff }` (`Debug, Clone, Copy, PartialEq, Eq, Hash`)
  - `pub enum MediaKind { Image, Video, Audio }` (같은 derive)
  - `impl Format { pub fn from_extension(ext: &str) -> Option<Format>; pub fn from_path(path: &std::path::Path) -> Option<Format>; pub fn extension(self) -> &'static str; pub fn label(self) -> &'static str; pub fn kind(self) -> MediaKind }`
  - `lib.rs`: `pub mod format;` (이후 Task에서 `catalog`, `geometry`, `naming`, `config` 추가)

- [ ] **Step 1: 실패하는 테스트 작성**

```rust
#[test]
fn from_extension_is_case_insensitive_and_aliased() {
    assert_eq!(Format::from_extension("JPG"), Some(Format::Jpg));
    assert_eq!(Format::from_extension("jpeg"), Some(Format::Jpg));
    assert_eq!(Format::from_extension("Tif"), Some(Format::Tiff));
    assert_eq!(Format::from_extension("aif"), Some(Format::Aiff));
    assert_eq!(Format::from_extension("opus"), Some(Format::Opus));
    assert_eq!(Format::from_extension("txt"), None);
    assert_eq!(Format::from_extension(""), None);
}

#[test]
fn from_path_uses_last_extension() {
    use std::path::Path;
    assert_eq!(Format::from_path(Path::new(r"C:\x\report.final.PNG")), Some(Format::Png));
    assert_eq!(Format::from_path(Path::new(r"C:\x\README")), None);
    assert_eq!(Format::from_path(Path::new(r"C:\x\사진.webp")), Some(Format::Webp));
}

#[test]
fn extension_label_and_kind() {
    assert_eq!(Format::Jpg.extension(), "jpg");
    assert_eq!(Format::Tiff.extension(), "tiff");
    assert_eq!(Format::M4a.label(), "M4A");
    assert_eq!(Format::Gif.kind(), MediaKind::Image); // 애니메이션 여부는 catalog의 InputItem이 판단
    assert_eq!(Format::Wmv.kind(), MediaKind::Video);
    assert_eq!(Format::Wma.kind(), MediaKind::Audio);
}
```

- [ ] **Step 2: 실패 확인**

Run: `cargo test -p swisswheel-core`
Expected: FAIL (컴파일 오류 — `Format` 미정의)

- [ ] **Step 3: `format.rs` 구현**

별칭은 `jpeg`→Jpg, `tif`→Tiff, `aif`→Aiff만. `label()`은 확장자를 대문자로 (`"JPG"`, `"TIFF"`, `"M4A"` …).

- [ ] **Step 4: 테스트 통과 확인**

Run: `cargo test -p swisswheel-core`
Expected: PASS (3 tests)

- [ ] **Step 5: CI 워크플로 작성 후 로컬에서 같은 검사 실행**

`.github/workflows/ci.yml`:
```yaml
name: ci
on: [push, pull_request]
jobs:
  check:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with: { components: "clippy, rustfmt" }
      - run: cargo fmt --all --check
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo test --workspace
```
Run: `cargo fmt --all --check; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace`
Expected: 모두 성공.

- [ ] **Step 6: 커밋**

```bash
git add Cargo.toml rust-toolchain.toml .gitignore LICENSE README.md .github crates
git commit -m "feat(core): workspace scaffold, CI, and Format type"
```

---

### Task 3: 카탈로그 — 입력 → 휠 항목

**Files:**
- Create: `crates/swisswheel-core/src/catalog.rs`
- Modify: `crates/swisswheel-core/src/lib.rs` (`pub mod catalog;`)
- Test: `catalog.rs` 안의 `mod tests`

**Interfaces:**
- Consumes: `Format`, `MediaKind` (Task 2)
- Produces:
  - `pub enum InputItem { Folder, File { format: Option<Format>, animated_gif: bool } }`
  - `pub enum WheelKind { Convert, Tools, Actions }` + `pub fn label(self) -> &'static str` (`"변환"`, `"도구"`, `"동작"`)
  - `pub enum Tool { HalveImage, CompressImage, CompressVideo, ExtractAudio, Mute }` + `pub fn label(self) -> &'static str` (`"50% 축소"`, `"압축"`, `"영상 압축"`, `"오디오 추출"`, `"음소거"`) + `pub fn suffix(self) -> Option<&'static str>` (`Some("50%")`, `Some("compressed")`, `Some("compressed")`, `None`, `Some("muted")`)
  - `pub enum Action { Share, Copy, Cut, CopyPath, Terminal }` + `pub fn label(self) -> &'static str` (`"공유"`, `"복사"`, `"잘라내기"`, `"경로 복사"`, `"터미널"`)
  - `pub enum Operation { Convert(Format), Tool(Tool), Action(Action) }` + `pub fn label(&self) -> &'static str` (Convert는 `Format::label`)
  - `pub fn items_for(wheel: WheelKind, inputs: &[InputItem]) -> Vec<Operation>`
  - `pub fn initial_wheel(inputs: &[InputItem]) -> Option<WheelKind>` — 항목이 있는 첫 휠 (Convert→Tools→Actions 순), 입력이 비면 `None`
  - `pub fn next_wheel(current: WheelKind, inputs: &[InputItem]) -> WheelKind` — 순환하며 빈 휠 건너뜀, 모두 비면 `current`
- 모든 enum: `Debug, Clone, Copy, PartialEq, Eq, Hash`.

**규칙 (spec 3.1~3.4):** 항목 순서는 아래 나열 순서 고정.
- 이미지(정지 GIF 포함) 변환 대상: Jpg, Png, Webp, Bmp, Tiff, Gif, Ico (자기 자신 제외)
- 영상(Mp4, Mkv, Webm, Mov, Avi, Wmv) 변환 대상: Mp4, Mkv, Webm, Mov, Avi, Gif, Mp3 (자기 자신 제외)
- 애니메이션 GIF 변환 대상: Mp4, Webm
- 오디오 변환 대상: Mp3, M4a, Wav, Flac, Ogg, Opus (자기 자신 제외)
- 도구: HalveImage = 이미지(정지 GIF 포함), CompressImage = Jpg/Png/Webp, CompressVideo·ExtractAudio·Mute = 영상 6종 (애니메이션 GIF는 도구 없음)
- 여러 입력: 변환·도구는 각 입력 목록의 교집합(순서는 위 고정 순서). 폴더나 `format: None` 파일이 하나라도 있으면 변환·도구는 빈 목록.
- 동작: 입력이 하나 이상이면 항상 5개 전부 (Share, Copy, Cut, CopyPath, Terminal).

- [ ] **Step 1: 실패하는 테스트 작성**

```rust
fn file(f: Format) -> InputItem { InputItem::File { format: Some(f), animated_gif: false } }

#[test]
fn single_image_convert_excludes_self() {
    use Format::*;
    assert_eq!(items_for(WheelKind::Convert, &[file(Png)]),
        [Jpg, Webp, Bmp, Tiff, Gif, Ico].map(Operation::Convert).to_vec());
}

#[test]
fn video_audio_and_animated_gif_convert() {
    use Format::*;
    assert_eq!(items_for(WheelKind::Convert, &[file(Mkv)]),
        [Mp4, Webm, Mov, Avi, Gif, Mp3].map(Operation::Convert).to_vec());
    assert_eq!(items_for(WheelKind::Convert, &[file(Wma)]),
        [Mp3, M4a, Wav, Flac, Ogg, Opus].map(Operation::Convert).to_vec());
    let anim = InputItem::File { format: Some(Gif), animated_gif: true };
    assert_eq!(items_for(WheelKind::Convert, &[anim]), [Mp4, Webm].map(Operation::Convert).to_vec());
    assert!(items_for(WheelKind::Tools, &[anim]).is_empty());
}

#[test]
fn tools_per_kind() {
    use Format::*;
    assert_eq!(items_for(WheelKind::Tools, &[file(Bmp)]), vec![Operation::Tool(Tool::HalveImage)]);
    assert_eq!(items_for(WheelKind::Tools, &[file(Jpg)]),
        vec![Operation::Tool(Tool::HalveImage), Operation::Tool(Tool::CompressImage)]);
    assert_eq!(items_for(WheelKind::Tools, &[file(Mp4)]),
        [Tool::CompressVideo, Tool::ExtractAudio, Tool::Mute].map(Operation::Tool).to_vec());
    assert!(items_for(WheelKind::Tools, &[file(Flac)]).is_empty());
}

#[test]
fn mixed_and_unknown_selections() {
    use Format::*;
    // 같은 포맷 여러 개
    assert_eq!(items_for(WheelKind::Convert, &[file(Png), file(Png)]),
        items_for(WheelKind::Convert, &[file(Png)]));
    // 서로 다른 이미지 → 교집합
    assert_eq!(items_for(WheelKind::Convert, &[file(Png), file(Jpg)]),
        [Webp, Bmp, Tiff, Gif, Ico].map(Operation::Convert).to_vec());
    // 이미지 + 오디오 → 변환·도구 비고 동작만
    let mixed = [file(Png), file(Mp3)];
    assert!(items_for(WheelKind::Convert, &mixed).is_empty());
    assert!(items_for(WheelKind::Tools, &mixed).is_empty());
    assert_eq!(items_for(WheelKind::Actions, &mixed).len(), 5);
    // 폴더 포함, 모르는 확장자
    assert!(items_for(WheelKind::Convert, &[file(Png), InputItem::Folder]).is_empty());
    let unknown = InputItem::File { format: None, animated_gif: false };
    assert!(items_for(WheelKind::Tools, &[unknown]).is_empty());
    assert_eq!(initial_wheel(&[unknown]), Some(WheelKind::Actions));
    // 빈 입력
    assert!(items_for(WheelKind::Actions, &[]).is_empty());
    assert_eq!(initial_wheel(&[]), None);
}

#[test]
fn wheel_cycling_skips_empty() {
    use Format::*;
    assert_eq!(initial_wheel(&[file(Png)]), Some(WheelKind::Convert));
    assert_eq!(next_wheel(WheelKind::Convert, &[file(Png)]), WheelKind::Tools);
    assert_eq!(next_wheel(WheelKind::Actions, &[file(Png)]), WheelKind::Convert);
    // FLAC: 도구 없음 → Convert 다음은 Actions
    assert_eq!(next_wheel(WheelKind::Convert, &[file(Flac)]), WheelKind::Actions);
    // 폴더: 동작만 → 자기 자신으로 돌아옴
    assert_eq!(next_wheel(WheelKind::Actions, &[InputItem::Folder]), WheelKind::Actions);
    assert_eq!(next_wheel(WheelKind::Tools, &[]), WheelKind::Tools);
}

#[test]
fn labels() {
    assert_eq!(WheelKind::Tools.label(), "도구");
    assert_eq!(Operation::Convert(Format::Webp).label(), "WEBP");
    assert_eq!(Operation::Action(Action::CopyPath).label(), "경로 복사");
    assert_eq!(Tool::Mute.suffix(), Some("muted"));
    assert_eq!(Tool::ExtractAudio.suffix(), None);
}
```

- [ ] **Step 2: 실패 확인**

Run: `cargo test -p swisswheel-core catalog`
Expected: FAIL (컴파일 오류)

- [ ] **Step 3: `catalog.rs` 구현**

입력 하나의 목록을 만드는 내부 함수 `fn items_for_one(wheel, &InputItem) -> Vec<Operation>`를 두고, 첫 입력의 목록에서 나머지 모든 입력 목록에 들어 있는 것만 남기는 방식으로 교집합을 구한다(순서 유지).

- [ ] **Step 4: 통과 확인**

Run: `cargo test -p swisswheel-core catalog`
Expected: PASS (6 tests)

- [ ] **Step 5: 커밋**

```bash
git add crates/swisswheel-core/src/catalog.rs crates/swisswheel-core/src/lib.rs
git commit -m "feat(core): catalog of wheel items per input selection"
```

---

### Task 4: 휠 기하 — 섹터 판정과 위치 보정

**Files:**
- Create: `crates/swisswheel-core/src/geometry.rs`
- Modify: `crates/swisswheel-core/src/lib.rs` (`pub mod geometry;`)
- Test: `geometry.rs` 안의 `mod tests`

**Interfaces:**
- Produces:
  - `pub const OUTER_RADIUS: f32 = 160.0; pub const CANCEL_RADIUS: f32 = 48.0; pub const RING_SPLIT_RADIUS: f32 = 104.0; pub const RING_CAPACITY: usize = 12; pub const MAX_ITEMS: usize = 24;`
  - `pub struct Sector { pub inner_radius: f32, pub outer_radius: f32, pub start_deg: f32, pub end_deg: f32 }` (`Debug, Clone, Copy, PartialEq`) — 각도는 12시 방향 0°, 시계 방향 증가, `start_deg`는 음수일 수 있음(0번 섹터는 12시를 중심으로 걸침)
  - `pub fn sector(item_count: usize, index: usize, scale: f32) -> Sector`
  - `pub enum Hit { Cancel, Item(usize), Outside }` (`Debug, Clone, Copy, PartialEq, Eq`)
  - `pub fn hit_test(item_count: usize, dx: f32, dy: f32, scale: f32) -> Hit` — `dx, dy`는 휠 중심 기준 물리 픽셀, y는 아래로 증가
  - `pub struct Rect { pub left: i32, pub top: i32, pub right: i32, pub bottom: i32 }` (`Debug, Clone, Copy, PartialEq, Eq`)
  - `pub fn clamp_center(cursor: (i32, i32), work_area: Rect, scale: f32) -> (i32, i32)` — 휠(반지름 `OUTER_RADIUS * scale`, 올림)이 작업 영역 안에 완전히 들어오도록 중심을 옮김

**규칙:**
- 반지름 `r = hypot(dx, dy) / scale`. `r < 48` → `Cancel`, `r > 160` → `Outside`.
- `item_count == 0`: `r <= 160`이면 `Cancel`.
- `item_count <= 12`: 고리 하나 `[48, 160]`, 섹터 `n`개.
- `item_count > 12`: 안쪽 고리 `[48, 104)`에 0~11번, 바깥 고리 `[104, 160]`에 12번 이후 `n-12`개. `item_count > 24`는 24로 취급.
- 고리 안 섹터 `k`개일 때 각도 `θ`(12시 0°, 시계 방향, `[0, 360)`)의 인덱스: `floor(((θ + 180/k) mod 360) / (360/k))`.

- [ ] **Step 1: 실패하는 테스트 작성**

```rust
const S: f32 = 1.0;

#[test]
fn center_and_outside() {
    assert_eq!(hit_test(6, 0.0, 0.0, S), Hit::Cancel);
    assert_eq!(hit_test(6, 0.0, -170.0, S), Hit::Outside);
    assert_eq!(hit_test(0, 0.0, -100.0, S), Hit::Cancel);
}

#[test]
fn single_ring_directions() {
    // 4개: 위(0) 오른쪽(1) 아래(2) 왼쪽(3)
    assert_eq!(hit_test(4, 0.0, -100.0, S), Hit::Item(0));
    assert_eq!(hit_test(4, 100.0, 0.0, S), Hit::Item(1));
    assert_eq!(hit_test(4, 0.0, 100.0, S), Hit::Item(2));
    assert_eq!(hit_test(4, -100.0, 0.0, S), Hit::Item(3));
}

#[test]
fn boundaries_and_seam() {
    assert_eq!(hit_test(4, 0.0, -48.0, S), Hit::Item(0));   // 취소 원 경계는 항목
    assert_eq!(hit_test(4, 0.0, -160.0, S), Hit::Item(0));  // 바깥 경계는 항목
    // 12시 바로 왼쪽(359.9°)도 0번
    let a = 359.9_f32.to_radians();
    assert_eq!(hit_test(4, 100.0 * a.sin(), -100.0 * a.cos(), S), Hit::Item(0));
    // 4개일 때 경계는 45° — 정확히 45°는 부동소수 오차로 불안정하므로 양쪽을 확인
    let b = 44.5_f32.to_radians();
    assert_eq!(hit_test(4, 100.0 * b.sin(), -100.0 * b.cos(), S), Hit::Item(0));
    let c = 45.5_f32.to_radians();
    assert_eq!(hit_test(4, 100.0 * c.sin(), -100.0 * c.cos(), S), Hit::Item(1));
}

#[test]
fn two_rings() {
    // 14개: 안쪽 12개, 바깥 2개 (12번 위, 13번 아래)
    assert_eq!(hit_test(14, 0.0, -80.0, S), Hit::Item(0));
    assert_eq!(hit_test(14, 0.0, -104.0, S), Hit::Item(12)); // 경계 104는 바깥 고리
    assert_eq!(hit_test(14, 0.0, 130.0, S), Hit::Item(13));
    assert_eq!(hit_test(30, 0.0, -130.0, S), Hit::Item(12)); // 24개로 제한
}

#[test]
fn dpi_scale() {
    assert_eq!(hit_test(4, 0.0, -230.0, 1.5), Hit::Item(0));  // 230/1.5 ≈ 153
    assert_eq!(hit_test(4, 0.0, -245.0, 1.5), Hit::Outside);  // ≈ 163
    assert_eq!(hit_test(4, 0.0, -70.0, 1.5), Hit::Cancel);    // ≈ 47
}

#[test]
fn sector_shape() {
    let s = sector(4, 0, 1.0);
    assert_eq!((s.inner_radius, s.outer_radius, s.start_deg, s.end_deg), (48.0, 160.0, -45.0, 45.0));
    let t = sector(14, 13, 2.0);
    assert_eq!((t.inner_radius, t.outer_radius, t.start_deg, t.end_deg), (208.0, 320.0, 90.0, 270.0));
}

#[test]
fn clamp_to_work_area() {
    let wa = Rect { left: 0, top: 0, right: 1920, bottom: 1040 };
    assert_eq!(clamp_center((960, 500), wa, 1.0), (960, 500));
    assert_eq!(clamp_center((10, 1035), wa, 1.0), (160, 880));
    assert_eq!(clamp_center((1919, 0), wa, 1.5), (1680, 240));
    // 보조 모니터(음수 좌표)
    let left = Rect { left: -1280, top: 0, right: 0, bottom: 1024 };
    assert_eq!(clamp_center((-1275, 512), left, 1.0), (-1120, 512));
}
```

- [ ] **Step 2: 실패 확인**

Run: `cargo test -p swisswheel-core geometry`
Expected: FAIL (컴파일 오류)

- [ ] **Step 3: `geometry.rs` 구현**

`θ = atan2(dx, -dy)`를 도 단위로 바꾼 뒤 `[0, 360)`으로 정규화한다. `hit_test`와 `sector`는 같은 고리·섹터 수 계산 헬퍼를 공유한다.

- [ ] **Step 4: 통과 확인**

Run: `cargo test -p swisswheel-core geometry`
Expected: PASS (7 tests)

- [ ] **Step 5: 커밋**

```bash
git add crates/swisswheel-core/src/geometry.rs crates/swisswheel-core/src/lib.rs
git commit -m "feat(core): wheel sector geometry and hit testing"
```

---

### Task 5: 출력 경로 이름

**Files:**
- Create: `crates/swisswheel-core/src/naming.rs`
- Modify: `crates/swisswheel-core/src/lib.rs` (`pub mod naming;`)
- Test: `naming.rs` 안의 `mod tests`

**Interfaces:**
- Produces:
  - `pub fn output_path(input: &Path, extension: &str, suffix: Option<&str>, exists: &dyn Fn(&Path) -> bool) -> PathBuf`
  - `pub fn temp_path(final_path: &Path) -> PathBuf` — `final_path`에 `.swisswheel-tmp`를 덧붙임 (`photo.jpg` → `photo.jpg.swisswheel-tmp`)
  - `pub const TEMP_SUFFIX: &str = ".swisswheel-tmp";`

**규칙:** 같은 폴더, 이름 = 입력의 `file_stem` + (suffix가 있으면 `" (suffix)"`) + `"." + extension`. 후보가 `exists`이거나 입력 경로와 대소문자 무시 비교로 같으면 `" (1)"`, `" (2)"` …를 확장자 앞에 붙여 재시도. `exists`는 호출자가 주입(테스트에선 가짜, 실제 앱에선 `Path::exists`).

- [ ] **Step 1: 실패하는 테스트 작성**

```rust
use std::path::{Path, PathBuf};

fn none(_: &Path) -> bool { false }

#[test]
fn basic_conversion_and_suffix() {
    assert_eq!(output_path(Path::new(r"C:\p\photo.png"), "jpg", None, &none), PathBuf::from(r"C:\p\photo.jpg"));
    assert_eq!(output_path(Path::new(r"C:\p\photo.jpg"), "jpg", Some("compressed"), &none),
        PathBuf::from(r"C:\p\photo (compressed).jpg"));
}

#[test]
fn collisions_count_up() {
    let taken = [PathBuf::from(r"C:\p\photo.jpg"), PathBuf::from(r"C:\p\photo (1).jpg")];
    let exists = |p: &Path| taken.iter().any(|t| t == p);
    assert_eq!(output_path(Path::new(r"C:\p\photo.png"), "jpg", None, &exists), PathBuf::from(r"C:\p\photo (2).jpg"));
    let taken2 = [PathBuf::from(r"C:\p\v (muted).mp4")];
    let exists2 = |p: &Path| taken2.iter().any(|t| t == p);
    assert_eq!(output_path(Path::new(r"C:\p\v.mp4"), "mp4", Some("muted"), &exists2),
        PathBuf::from(r"C:\p\v (muted) (1).mp4"));
}

#[test]
fn never_returns_input_path() {
    // 대소문자만 다른 같은 이름도 원본으로 간주
    let out = output_path(Path::new(r"C:\p\clip.MP3"), "mp3", None, &none);
    assert_eq!(out, PathBuf::from(r"C:\p\clip (1).mp3"));
}

#[test]
fn tricky_names() {
    assert_eq!(output_path(Path::new(r"C:\p\report.final.png"), "webp", None, &none), PathBuf::from(r"C:\p\report.final.webp"));
    assert_eq!(output_path(Path::new(r"C:\p\README"), "txt", None, &none), PathBuf::from(r"C:\p\README.txt"));
    assert_eq!(output_path(Path::new(r"D:\사진\여름 휴가.png"), "jpg", Some("50%"), &none),
        PathBuf::from(r"D:\사진\여름 휴가 (50%).jpg"));
    let taken = [PathBuf::from(r"C:\p\a (1).jpg")];
    let exists = |p: &Path| taken.iter().any(|t| t == p);
    // "a (1).png" → "a (1).jpg"가 있으면 "a (1) (1).jpg" (원래 이름의 숫자를 해석하지 않음)
    assert_eq!(output_path(Path::new(r"C:\p\a (1).png"), "jpg", None, &exists), PathBuf::from(r"C:\p\a (1) (1).jpg"));
}

#[test]
fn temp_path_appends_suffix() {
    assert_eq!(temp_path(Path::new(r"C:\p\photo.jpg")), PathBuf::from(r"C:\p\photo.jpg.swisswheel-tmp"));
}
```

- [ ] **Step 2: 실패 확인**

Run: `cargo test -p swisswheel-core naming`
Expected: FAIL (컴파일 오류)

- [ ] **Step 3: `naming.rs` 구현**

대소문자 무시 비교는 `to_string_lossy().to_lowercase()`로 충분하다(윈도우 경로).

- [ ] **Step 4: 통과 확인**

Run: `cargo test -p swisswheel-core naming`
Expected: PASS (5 tests)

- [ ] **Step 5: 커밋**

```bash
git add crates/swisswheel-core/src/naming.rs crates/swisswheel-core/src/lib.rs
git commit -m "feat(core): collision-safe output path naming"
```

---

### Task 6: 설정 파싱과 검증

**Files:**
- Create: `crates/swisswheel-core/src/config.rs`
- Modify: `crates/swisswheel-core/src/lib.rs` (`pub mod config;`), `crates/swisswheel-core/Cargo.toml` (`serde = { version = "1", features = ["derive"] }`, `toml = "0.8"`, `thiserror = "2"`)
- Test: `config.rs` 안의 `mod tests`

**Interfaces:**
- Produces:
  - `pub enum Key { Space, Tab, Escape, CapsLock, Backquote }` (`Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize`; TOML에서는 변형 이름 그대로 `"Space"` 등) + `pub fn vk(self) -> u16` (`0x20`, `0x09`, `0x1B`, `0x14`, `0xC0`)
  - `pub struct Quality { pub jpg: u8, pub webp: u8, pub mp3_kbps: u16 }` — 기본값 90 / 85 / 192
  - `pub struct Config { pub open_key: Key, pub cycle_key: Key, pub cancel_key: Key, pub start_with_windows: bool, pub quality: Quality }` — 기본값 Space / Tab / Escape / false / Quality::default(). 두 구조체 모두 `#[serde(default)]`, `Debug, Clone, PartialEq, Serialize, Deserialize`, `Default`는 위 값으로 수동 구현.
  - `pub enum ConfigError { Parse(String), Invalid(String) }` (`thiserror`, `Debug, PartialEq`)
  - `pub fn parse(text: &str) -> Result<Config, ConfigError>`
  - `pub fn default_toml() -> String` — `Config::default()`를 `toml::to_string_pretty`로 직렬화

**검증 규칙:** `quality.jpg`, `quality.webp`는 1~100, `quality.mp3_kbps`는 32~320, 세 키는 서로 달라야 함. 모르는 키는 무시(`deny_unknown_fields` 쓰지 않음). 파일 읽기/쓰기는 하지 않는다(bin 크레이트 몫).

- [ ] **Step 1: 실패하는 테스트 작성**

```rust
#[test]
fn defaults_round_trip() {
    let c = Config::default();
    assert_eq!((c.open_key, c.cycle_key, c.cancel_key), (Key::Space, Key::Tab, Key::Escape));
    assert_eq!((c.quality.jpg, c.quality.webp, c.quality.mp3_kbps), (90, 85, 192));
    assert!(!c.start_with_windows);
    assert_eq!(parse(&default_toml()), Ok(c));
    assert_eq!(parse(""), Ok(Config::default()));
}

#[test]
fn partial_and_unknown_keys() {
    let c = parse("open_key = \"CapsLock\"\nfuture_option = 3\n[quality]\njpg = 70\n").unwrap();
    assert_eq!(c.open_key, Key::CapsLock);
    assert_eq!(c.quality.jpg, 70);
    assert_eq!(c.quality.webp, 85);
    assert_eq!(c.cycle_key, Key::Tab);
}

#[test]
fn invalid_configs() {
    assert!(matches!(parse("[quality]\njpg = 0"), Err(ConfigError::Invalid(_))));
    assert!(matches!(parse("[quality]\nmp3_kbps = 999"), Err(ConfigError::Invalid(_))));
    assert!(matches!(parse("open_key = \"Tab\""), Err(ConfigError::Invalid(_)))); // cycle_key와 같음
    assert!(matches!(parse("[quality]\njpg = \"high\""), Err(ConfigError::Parse(_))));
    assert!(matches!(parse("open_key = \"F5\""), Err(ConfigError::Parse(_))));
    assert!(matches!(parse("this is not toml"), Err(ConfigError::Parse(_))));
}

#[test]
fn virtual_key_codes() {
    assert_eq!(Key::Space.vk(), 0x20);
    assert_eq!(Key::Backquote.vk(), 0xC0);
}
```

- [ ] **Step 2: 실패 확인**

Run: `cargo test -p swisswheel-core config`
Expected: FAIL (컴파일 오류)

- [ ] **Step 3: `config.rs` 구현**

`parse`는 `toml::from_str` 오류를 `ConfigError::Parse(e.to_string())`로, 검증 실패를 어떤 값이 왜 틀렸는지 담은 한국어 문구의 `ConfigError::Invalid`로 반환한다 (예: `"quality.jpg는 1~100 사이여야 합니다 (현재 0)"`).

- [ ] **Step 4: 전체 검사 통과 확인**

Run: `cargo fmt --all --check; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace`
Expected: 모두 성공. core 테스트 25개 PASS.

- [ ] **Step 5: 커밋**

```bash
git add crates/swisswheel-core/src/config.rs crates/swisswheel-core/src/lib.rs crates/swisswheel-core/Cargo.toml Cargo.lock
git commit -m "feat(core): config parsing with validation and defaults"
```

---

## 다음 계획 (이 계획 범위 밖)

스파이크 결과 문서를 바탕으로 `docs/superpowers/plans/YYYY-MM-DD-swisswheel-shell-and-engines.md`를 작성한다: 트레이·훅·휠 렌더링·dnd 연결(spec 8장 2단계) → 이미지 엔진·작업 실행기·알림(3) → FFmpeg(4) → 동작(5) → 설정 파일 입출력·자동 실행·패키징(6). 그 계획의 Review Focus에 넣을 것: 같은 파일을 같은 포맷으로 동시에 두 번 변환할 때의 출력 이름 경합(실행기에서 경로 예약 필요).
