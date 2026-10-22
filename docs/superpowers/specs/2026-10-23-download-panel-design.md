# 다운로드 패널 설계 문서

- 작성일: 2026-10-23
- 상태: 승인됨 (대화에서 합의), 사용자 검토 대기
- 기반: `main` (8627155, v0.2.0). FFmpeg 설치기(`2026-10-22-ffmpeg-download-restart-design.md`)와 설정 패널(`2026-10-21-settings-categories-design.md`)을 재사용한다.

## 1. 목적과 범위

**목적:** 다운로드 타일이 FFmpeg를 바로 받는 대신, SwissWheel 기본 기능에 필요한 앱(필수 앱)을 보여 주고 설치·업데이트·제거할 수 있는 **다운로드 패널**을 연다.

**범위:**
- 필수 앱 목록은 지금 FFmpeg 하나다. 앱을 더할 때는 목록 한 항목과 그 앱의 설치기만 더하면 되게 짠다(3.1).
- 패널을 열 때 설치된 버전과 최신 버전을 자동으로 확인한다.
- 설치·업데이트·제거는 버튼마다 확인 창 한 번 뒤에 백그라운드에서 한다.

**범위 밖:**
- 두 번째 필수 앱 자체(목록에 더할 준비만 한다).
- 패널 안 확인 창(확인은 지금처럼 `MessageBoxW`).
- 직접 설치해 PATH에 있는 FFmpeg의 업데이트·제거.
- 특정 버전 고르기(항상 최신, 이전 결정 그대로).

## 2. 사용자 경험

### 2.1 패널 열기
- 다운로드 타일에서 Enter / ↓ / 클릭 → 바 아래에 다운로드 패널이 열린다. 설정 타일과 같은 방식이고, 타일에 커서만 올려서는 열리지 않는다.
- 다른 타일로 옮기면 패널이 닫힌다(설정 패널과 같음).
- 크기·위치·유리·애니메이션은 설정 패널과 같다. 창 크기는 바뀌지 않는다.

### 2.2 패널 모양
```
┌────────────────────────────────────────────────────────┐
│ 필수 앱 · 확인 중…                                     │  ← 머리줄
│ FFmpeg · 영상·오디오   9.0.2 · 9.0.3 있음  [업데이트] [제거] │  ← 앱 줄
│                                                        │
│                                                        │
└────────────────────────────────────────────────────────┘
```
- 머리줄: "필수 앱". 확인 중이면 "필수 앱 · 확인 중…". 설정 오류 안내줄(notice)이 있으면 그 아래.
- 앱 줄: 왼쪽에 이름과 용도, 가운데에 상태, 오른쪽에 버튼(오른쪽 끝부터 채움).

| 상황 | 상태 | 버튼 |
|---|---|---|
| 받는 중 (설치·업데이트, 무엇보다 먼저) | 받는 중 {n}% | 없음 |
| 설치 상태를 아직 모름 (처음 연 직후) | 확인 중… | 없음 |
| 없음 | 설치 안 됨 | [설치] |
| PATH의 FFmpeg만 있음 | 시스템 {버전} 사용 중 | [전용 설치] |
| 전용 설치, 최신 버전 확인 중 | {버전} | [제거] |
| 전용 설치, 최신 | {버전} · 최신 | [제거] |
| 전용 설치, 새 버전 있음 | {버전} · {최신} 있음 | [업데이트] [제거] |
| 전용 설치, 최신 버전 확인 실패 | {버전} · 확인 실패 | [제거] |
| 전용 설치, 내 버전을 못 읽음 | 버전 알 수 없음 | [업데이트] [제거] |
| 전용 설치, 두 버전을 비교할 수 없음 | {버전} · 최신 {최신} | [업데이트] [제거] |

- "전용 설치"는 SwissWheel이 받아 `install_dirs()`(exe 폴더 `ffmpeg\`, `%LOCALAPPDATA%\SwissWheel\ffmpeg\`)에 둔 것. "시스템"은 PATH에서 찾은 것.
- 시스템 FFmpeg만 있을 때 최신 확인이 실패해도 상태·버튼은 같다(전용 설치는 확인 창에서 다시 받는다).
- 시스템 버전을 읽지 못하면 "시스템 FFmpeg 사용 중".

### 2.3 조작
| 입력 | 머리줄 | 앱 줄 |
|---|---|---|
| ↓ / Enter | 첫 앱 줄로 | ↓: 다음 앱 줄 (없으면 그대로) |
| ↑ | 바로 (패널 닫힘) | 위 앱 줄, 첫 줄이면 머리줄 |
| ← / → | 없음 | 버튼 사이 이동 (끝에서 멈춤) |
| Enter | — | 선택된 버튼 실행 (버튼이 없으면 없음) |
| Esc | 바로 | 머리줄로 |
| 클릭 | 없음 | 줄: 그 줄 선택. 버튼: 그 버튼 실행 |
| Space(열기 키) | 메뉴 닫힘 | 메뉴 닫힘 |

- 앱 줄에 들어가면 그 줄의 첫 버튼(왼쪽)이 선택된다. 상태가 바뀌어 버튼 수가 줄면 선택은 마지막 버튼으로 당긴다. 버튼이 없어지면 선택 없음.
- 선택된 버튼과 마우스가 올라간 버튼은 설정 패널의 ◀ ▶처럼 강조한다.

### 2.4 버튼 실행
- 버튼을 실행하면 메뉴를 닫고, 지금 다운로드 타일처럼 확인 창을 한 번 띄운다.
  - 설치·전용 설치·업데이트: 지금 설치기의 확인 창 그대로("FFmpeg {tag}(약 {n}MB)를 받아 설정할까요?\n\n설치 위치: {dir}").
  - 제거: "SwissWheel이 받은 FFmpeg {버전}를 지울까요?\n\n위치: {dir}". 시스템 FFmpeg가 있으면 "\n\n지운 뒤에는 시스템 FFmpeg를 씁니다"를 덧붙인다.
- 거절 (확인 창 없이 알림, 제목 "다운로드"):
  - 다른 업데이트·다운로드가 진행 중이거나 업데이트가 교체를 기다리는 중 → "이미 업데이트나 다운로드를 진행하는 중입니다"
  - 제거·업데이트인데 변환이 진행 중 → "진행 중인 변환이 끝난 뒤 다시 눌러 주세요". 변환 중인 FFmpeg는 지우거나 덮어쓸 수 없기 때문이다(3.3). 설치·전용 설치는 다른 폴더에 새로 두므로 변환 중에도 된다.
- 받는 동안 진행률은 지금처럼 진행 카드("FFmpeg {tag} · 다운로드")와 트레이에 보인다. 패널을 다시 열면 앱 줄이 "받는 중 {n}%"로 계속 갱신된다.
- 끝나면 알림:
  - 설치·업데이트 성공: "FFmpeg {tag} 설정을 마쳤습니다"(지금 그대로)
  - 제거 성공: "FFmpeg를 지웠습니다"
  - 제거 실패: "FFmpeg를 지우지 못했습니다"
  - 받기 실패들: 지금 문구 그대로

## 3. 구조

### 3.1 모듈
**`swisswheel-core` (Win32 비의존, 단위 테스트)**
- `tools.rs` (새로)
  - `Tool { Ffmpeg }`, `TOOLS: [Tool; 1]`, `label()`("FFmpeg"), `purpose()`("영상·오디오").
  - `Installed { None, System(Option<String>), Own(Option<String>) }`: 찾은 위치와 읽은 버전.
  - `Latest { Unknown, Checking, Found(String), Failed }`.
  - `ToolState { installed: Option<Installed>, latest: Latest, busy: Option<f32> }` (`installed: None` = 아직 모름).
  - `ToolAction { Install, InstallOwn, Update, Remove }`.
  - `fn view(&ToolState) -> (String /*상태*/, Vec<ToolAction>)`: 2.2 표 그대로.
  - `fn parse_ffmpeg_version(first_line: &str) -> Option<String>`: `ffmpeg version 9.0.2-essentials_build-www.gyan.dev …` → `9.0.2`. `n7.1-…`처럼 앞에 `n`이 붙으면 뗀다. 숫자·점으로 된 버전이 아니면 `None`.
  - `fn is_newer(latest: &str, installed: &str) -> Option<bool>`: 점으로 나눈 숫자를 차례로 비교(`9.0.10 > 9.0.9`, `9.1 > 9.0.2`). 숫자가 아닌 조각이 있으면 `None`(→ 2.2 표의 "비교할 수 없음" 줄).
  - `fn on_action(action, jobs_active: bool, background_busy: bool) -> TileDecision`: 2.4의 거절 규칙.
  - 문구는 `text` 모듈에.
- `menu.rs`
  - 패널이 설정 전용에서 "지금 타일의 패널"이 된다. `panel: Option<Focus>`는 설정용 그대로 두고, 다운로드용 `DownloadFocus { Header, Row { tool: usize, button: Option<usize> } }`을 따로 둔다(`tile`이 무엇인지에 따라 하나만 쓰인다).
  - `MenuSession::set_tool_actions(&[Vec<ToolAction>])`: 앱이 상태가 바뀔 때마다 넘긴다. 버튼 선택을 2.3대로 당긴다.
  - 새 `MenuAction::Tool(Tool, ToolAction)`. `Notice(Tile::Download)`는 없어진다(다운로드 타일은 패널을 연다).
  - 새 `MenuHit::ToolRow(usize)`, `MenuHit::ToolButton(usize, usize)`.
- `menu_geometry.rs`: 다운로드 패널의 머리줄·앱 줄·버튼 상자(오른쪽 끝부터, 버튼 폭 88 DIP·간격 8), `hit`이 다운로드 패널일 때 쓸 `DownloadShape { rows, buttons: Vec<usize> }`.
- `ffmpeg_release.rs`: `on_download_tile`과 `text::READY`는 지운다(패널이 대신함). 나머지는 그대로.

**`swisswheel` (앱)**
- `tool_status.rs` (새로): 상태 확인 스레드. `ffmpeg -version`을 `CREATE_NO_WINDOW`로 실행해 첫 줄을 읽고(5초 제한), 최신 릴리스를 `ffmpeg_release::ENDPOINT`(디버그는 `SWISSWHEEL_FFMPEG_URL`)에서 받는다. 결과를 슬롯에 두고 `WM_APP_TOOLS`를 보낸다.
- `ffmpeg_install.rs`
  - `run`에 `target: Option<PathBuf>` 인자: 업데이트는 지금 전용 설치가 있는 폴더에 덮어쓴다(그 폴더가 쓰기 불가면 `pick_target`). 설치·전용 설치는 지금처럼 `pick_target`.
  - `remove(dirs, ui) -> RemoveOutcome { Removed, Failed, Declined }`: 확인 창 뒤 `install_dirs()` 각각에서 `ffmpeg.exe` → `ffprobe.exe` → `LICENSE` 순으로 지우고, 폴더가 비면 폴더도 지운다. 다른 파일은 남긴다.
  - `spawn_remove(hwnd, msg, slot)`: 제거 스레드.
- `jobs.rs`: `Runner::forget_tools()` — 기억한 FFmpeg 경로를 버려 다음 미디어 작업이 다시 찾게 한다. 설치·업데이트·제거가 끝날 때마다 부른다.
- `menubar/`: `MenuView`에 다운로드 패널 줄(`ToolRowView { label, purpose, status, buttons: Vec<&str>, progress }`)과 머리줄 문구, 그리기.
- `app.rs`: 패널을 열 때 상태 확인 시작, `WM_APP_TOOLS`/`WM_APP_FFMPEG`/`WM_APP_FFMPEG_REMOVE` 처리, `MenuAction::Tool` 실행, 받는 동안 열린 패널 갱신(기존 진행 타이머에서 `render_menu`).

### 3.2 상태 확인 (패널을 열 때)
1. 다운로드 패널이 열릴 때마다 설치 상태를 다시 읽는다: `install_dirs()`에서 찾으면 `Own`, PATH에서 찾으면 `System`, 없으면 `None`. 버전은 `ffmpeg -version`으로 읽는다(스레드에서).
2. 최신 버전은 마지막으로 **성공한** 확인이 1시간 안이면 그 값을 쓰고, 아니면 다시 받는다. 실패는 기억하지 않는다.
3. 확인 스레드는 한 번에 하나. 이미 돌고 있으면 새로 띄우지 않는다.
4. 결과가 오면 `ToolState`를 바꾸고, 메뉴가 열려 있으면 `set_tool_actions` 뒤 다시 그린다.
5. 설치·업데이트·제거가 끝나면 설치 상태를 다시 읽는다(최신 값은 그대로).

### 3.3 업데이트 순서
- 지금 설치기의 순서를 그대로 쓴다: 릴리스 JSON → 확인 창 → 부분 폴더에 받기 → SHA-256 확인 → 압축 풀기 → 옮기기.
- 옮기기는 `ffprobe.exe` → `LICENSE` → `ffmpeg.exe` 순으로 덮어쓴다(지금 그대로). 변환이 돌고 있으면 실행 중인 exe를 덮을 수 없으므로, 업데이트도 변환 중에는 거절한다(2.4 규칙을 업데이트에도 적용).

## 4. 오류 처리와 안전
- 제거는 SwissWheel이 넣은 이름(`ffmpeg.exe`, `ffprobe.exe`, `LICENSE`)만, `install_dirs()` 안에서만 지운다. PATH의 FFmpeg는 어떤 경우에도 건드리지 않는다.
- 제거 중 한 파일이라도 못 지우면 `Failed`. 이미 지운 파일은 되돌리지 않는다. 남은 반쪽은 `locate`가 두 파일이 다 있어야 쓰므로 쓰이지 않고, 다시 제거하면 마저 지운다.
- `ffmpeg -version`이 5초 안에 안 끝나거나 실패하면 버전은 `None`(알 수 없음). 설치 상태 판단에는 영향 없다.
- 최신 확인 실패는 패널 상태로만 보이고 알림은 띄우지 않는다(패널을 열 때마다 알림이 뜨지 않게).
- 메뉴가 닫혀 있을 때 확인 결과가 오면 상태만 저장한다.
- 확인 스레드와 설치·제거 스레드는 UI 스레드에서 메시지 박스를 띄우지 않는다(지금 규칙 그대로).

## 5. 테스트
**단위 (core)**
- `tools::view`: 2.2 표의 모든 줄.
- `parse_ffmpeg_version`: GyanD essentials, `n7.1-…`, git 빌드(`N-12345-g…`, `2024-…`), 빈 줄.
- `is_newer`: 같음, 끝자리, 자리 수 다름(`9.1` vs `9.0.2`), `9.0.10` vs `9.0.9`, 숫자 아님.
- `on_action`: 바쁠 때, 변환 중 제거·업데이트, 변환 중 설치는 허용.
- `menu`: 다운로드 타일 Enter → 패널 머리줄, ↓/↑/←/→/Esc/Enter, 버튼 수가 줄 때 선택 당김, 버튼 없는 줄의 Enter, 다른 타일로 옮기면 패널 닫힘, 클릭.
- `menu_geometry`: 버튼 상자가 오른쪽 끝부터 놓이고 패널 안에 들어감, `hit`.

**통합 (앱, 임시 폴더)**
- `remove`: 두 설치 폴더 모두에서 세 파일만 지우고 다른 파일은 남김, 빈 폴더 지움, 거절 시 그대로, 한 폴더만 있을 때.
- `run`에 `target`: 업데이트가 기존 폴더에 덮어씀.
- `forget_tools` 뒤 다음 작업이 다시 찾음(기존 `ffmpeg_installed_after_start_is_used_without_restart` 시험 방식).
- `tool_status`: 가짜 `ffmpeg.exe` 대신 `-version` 첫 줄 읽기는 순수 함수로 시험하고, 프로세스 실행은 수동 시험으로.

**수동 (`docs/manual-test.md`)**: 없음 → 설치, 시스템만 → 전용 설치, 최신, 업데이트 있음(가짜 릴리스 서버), 확인 실패(서버 끔), 제거(시스템 있음/없음), 변환 중 제거·업데이트 거절, 받는 중 패널 다시 열기.

## 6. 문서
- README: 다운로드 타일 설명을 "필수 앱을 설치·업데이트·제거하는 패널"로.
- 메뉴 설계 문서(2026-10-21) 2.2의 다운로드 타일 줄을 패널로 고친다.
- FFmpeg 다운로드 설계 문서(2026-10-22)에 "다운로드 타일 동작은 2026-10-23 다운로드 패널로 바뀜" 한 줄.
