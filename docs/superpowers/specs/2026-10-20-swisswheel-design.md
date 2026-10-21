# SwissWheel v0.1 설계 문서

- 작성일: 2026-10-20
- 상태: 승인됨 (대화에서 합의), 사용자 검토 대기
- 영감: tangerineformac.com (macOS용 드래그-휠 파일 변환기). 기능 컨셉만 참고하며 이름·로고·문구·디자인은 독자적으로 만든다.

## 1. 목적과 성공 기준

**목적:** 윈도우용 무료 오픈소스 오프라인 파일 변환기. 탐색기에서 파일을 드래그하는 도중 키 하나로 원형 휠을 띄워, 원하는 포맷/도구/동작 위에 놓기만 하면 처리된다. 파일은 절대 외부로 업로드되지 않는다.

**성공 기준 (v0.1):**
1. 탐색기 드래그 중 Space → 휠 표시 → 항목 위에 드롭 → 같은 폴더에 결과 파일 생성, 의 흐름이 Windows 10 22H2 / Windows 11에서 동작한다.
2. 탐색기의 기존 드래그 동작(Shift 이동, Ctrl 복사, Alt 바로가기)은 전혀 영향받지 않는다.
3. 원본 파일은 어떤 경우에도 수정·삭제·덮어쓰기 되지 않는다.
4. 대기 중 메모리(Private Working Set) 15MB 이하.
5. 3장 기능 목록이 모두 동작한다.

**비목표 (v0.1):** 키보드 전용 모드, 구간 선택이 필요한 도구(트림·분할), HEIC/AVIF, PDF·문서·슬라이드·스프레드시트·압축파일, Float, ARM64 빌드. (설정 GUI는 `2026-10-21-desktop-menu-design.md`, 업데이트는 `2026-10-22-updater-design.md`에서 다룬다.)

## 2. 사용자 경험

### 2.1 휠 열기와 조작
| 입력 | 조건 | 동작 |
|---|---|---|
| Space | 파일 드래그 중, 휠 닫힘 | 커서 위치에 **변환** 휠 표시 (키를 떼도 유지) |
| Tab | 휠 열림 | 변환 → 도구 → 동작 → 변환 … 순환 |
| Esc | 휠 열림 | 휠만 닫음. 드래그는 계속됨 (탐색기로 전달하지 않음) |
| 드롭 | 휠의 항목 위 | 해당 작업 실행, 휠 닫힘 |
| 드롭 | 휠 중앙(취소 영역) 또는 휠 밖 | 아무 작업 없음 / 휠 밖이면 원래 대상에 정상 드롭 |
| 커서 올림 | 옵션이 있는 칸 (2.5) | 휠 옆에 그 칸의 옵션 패널 표시. 패널의 줄에 드롭하면 그 설정, 칸에 바로 드롭하면 기본값 |

- 휠이 열려 있는 동안에만 Space·Tab·Esc를 가로챈다. 그 외에는 키 입력을 일절 건드리지 않는다.
- 키 바인딩은 `config.toml`에서 변경 가능 (`open_key`, `cycle_key`, `cancel_key`).
- 현재 휠에 표시할 항목이 없으면(예: 섞인 파일 → 변환 휠) Tab 순환에서 그 휠을 건너뛴다. 처음 열릴 때도 항목이 있는 첫 휠을 보여준다.

### 2.2 휠 모양
- 지름 약 320px(DPI 배율 적용), 중앙 원(반지름 약 48px)은 취소 영역 + 현재 휠 이름 표시.
- 바깥 고리를 항목 수만큼 섹터로 등분. 항목이 12개를 넘으면 두 번째 고리를 추가 (최대 24개).
- 드래그 커서가 올라간 섹터는 강조 + 살짝 확대 애니메이션 (120ms).
- 열림/닫힘/휠 전환은 페이드·스케일 애니메이션 (150ms 이내). 시스템 다크/라이트 테마를 따른다.
- 배경은 **유리(아크릴) 효과**: 휠 뒤 화면을 시스템 블러로 흐리게 비추고(`Windows.UI.Composition` `HostBackdropBrush`, 원형 클립) 그 위에 반투명 틴트와 칸을 그린다. 윈도우의 "투명 효과"가 꺼져 있으면 불투명 배경으로 대체한다.
- 화면 가장자리에서는 휠이 화면 안에 들어오도록 위치를 보정한다 (멀티 모니터: 커서가 있는 모니터 기준).

### 2.3 출력 규칙
- 결과는 원본과 같은 폴더, 이름은 `원본이름.새확장자`. 도구는 `원본이름 (compressed).확장자`처럼 접미사를 붙인다. 이미지 축소는 비율(`사진 (25%).png`), 해상도를 고른 영상 변환은 해상도(`영상 (720p).mp4`)를 접미사로 붙인다.
- 이름이 이미 있으면 `이름 (1).ext`, `이름 (2).ext` … 로 회피. **덮어쓰기 금지.**
- 원본 폴더에 쓰기 권한이 없으면 실패로 처리하고 알림으로 알린다 (다른 위치로 몰래 저장하지 않음).
- 작업 중 생성된 임시 파일은 같은 폴더에 `.swisswheel-tmp` 접미사로 쓰고 성공 시 rename, 실패 시 삭제.

### 2.4 피드백
- 작업 시작: 트레이 아이콘이 "작업 중" 상태 아이콘으로 바뀜 (툴팁에 진행 중 개수/퍼센트).
- 진행 표시 (각각 설정 `[progress]`로 켜고 끔, 기본 둘 다 켬 — 나중의 설정 창이 이 값을 편집한다):
  - `window`: 작업이 있는 동안 주 모니터 작업 영역 오른쪽 아래에 휠과 같은 유리 카드. 파일마다 "이름 → 포맷", 진행 막대, % 또는 "대기". 최대 5줄, 나머지는 "외 N개". 클릭이 아래 창으로 통과하고 포커스를 가져가지 않으며 끝나면 사라진다.
  - `tray_ring`: 트레이 아이콘 둘레가 전체 진행률만큼 12시부터 시계 방향으로 채워짐 (5% 단위로 갱신).
- 완료: 풍선 알림 "3개 파일을 PNG로 변환했습니다". 알림 클릭 시 결과 폴더를 열고 결과 파일 선택.
- 실패: 풍선 알림에 실패 개수와 첫 번째 오류 요약. 상세 로그는 `%LOCALAPPDATA%\SwissWheel\logs\`.

### 2.5 옵션 패널
드래그 중에는 클릭할 수 없으므로 옵션은 **드롭할 수 있는 줄**로 고른다.
- 옵션이 있는 칸에 커서가 올라가면 휠 옆(오른쪽, 작업 영역에 자리가 없으면 왼쪽)에 그 칸의 패널을 붙여 표시한다. 패널은 휠 가장자리에 겹쳐 붙어 있어 휠 → 패널 이동 중 창 밖으로 나가지 않는다.
- 패널은 옵션이 있는 칸에 커서가 **300ms 머물렀을 때** 열리거나 그 칸의 것으로 바뀐다. 휠 → 패널로 가는 길에 다른 칸(옵션이 있는 칸 포함)이나 취소 영역을 지나가도 패널은 그대로다 — 영상 변환 휠은 거의 모든 칸에 옵션이 있고, 반대편 패널로 가는 직선은 가운데를 지나기 때문. 패널은 휠 전환(Tab)·휠 닫힘에서만 닫힌다.
- 패널: 휠과 같은 유리 스타일, 맨 위에 제목("GIF 옵션"), 줄마다 이름 + 작은 설명, 기본값 줄에 "기본" 표시. 커서가 올라간 줄은 휠 칸과 같은 강조색.
- 패널이 닫혀 있으면 패널 자리는 히트 테스트되지 않는다(그 아래 창에 정상 드롭).

| 칸 | 선택지 (굵게 = 기본) |
|---|---|
| GIF (영상 → GIF) | 작게 10fps·320px / **보통 15fps·480px** / 부드럽게 24fps·원본 크기 |
| MP4·MKV·WEBM·MOV·AVI (영상 → 영상) | **원본** / 1080p / 720p / 480p — 짧은 변 기준(세로 영상의 720p는 가로 720), 원본보다 작을 때만 줄임(확대 안 함). 원본이 아니면 스트림 복사 없이 인코딩 |
| 영상 압축 | 약하게 CRF 24·오디오 160kbps / **보통 CRF 28·128kbps** / 강하게 CRF 32·96kbps·짧은 변 720 이하로 축소 |
| 이미지 축소 | 25% / **50%** / 75% |
| JPG·WEBP (이미지 → 이미지) | 높게 JPG 95·WEBP 92 / **보통 (설정의 `quality.jpg`·`quality.webp`)** / 작게 JPG 75·WEBP 70 |

## 3. 기능 목록 (v0.1)

### 3.1 변환
| 입력 | 출력 | 엔진 |
|---|---|---|
| JPG, PNG, WebP, BMP, TIFF, GIF(정지), ICO | JPG, PNG, WebP, BMP, TIFF, GIF, ICO (자기 자신 제외) | `image` + `webp` 크레이트 |
| MP4, MKV, WebM, MOV, AVI, WMV | MP4, MKV, WebM, MOV, AVI, GIF, MP3 (자기 자신 제외) | FFmpeg |
| GIF(애니메이션) | MP4, WebM | FFmpeg |
| MP3, M4A, WAV, FLAC, OGG, Opus, WMA, AIFF | MP3, M4A, WAV, FLAC, OGG, Opus (자기 자신 제외) | FFmpeg |

- GIF는 프레임 수로 정지/애니메이션을 판별한다. 정지 GIF는 이미지 그룹, 애니메이션 GIF는 영상 그룹으로 분류한다.
- 투명도가 있는 이미지를 JPG/BMP로 변환하면 흰 배경으로 합성한다.
- ICO 출력은 256/128/64/48/32/16 크기를 포함하는 멀티 사이즈 아이콘으로 만든다.
- 품질 기본값: JPG 90, WebP 손실 85, MP3 192kbps, 영상 H.264 CRF 23 / VP9 CRF 32.

### 3.1.1 FFmpeg 인코딩 규칙
| 출력 | 인자 |
|---|---|
| MP4, MOV, MKV | `-c:v libx264 -preset medium -crf 23 -pix_fmt yuv420p`, 짝수 크기 보정 `scale=trunc(iw/2)*2:trunc(ih/2)*2`, `-c:a aac -b:a 192k`, MP4·MOV는 `-movflags +faststart` |
| WEBM | `-c:v libvpx-vp9 -crf 32 -b:v 0 -deadline good -cpu-used 4 -row-mt 1 -c:a libopus -b:a 128k` |
| AVI | `-c:v mpeg4 -q:v 4 -c:a libmp3lame -b:a <mp3_kbps>k` |
| GIF | `fps=<fps>,scale='min(<폭>,iw)':-2:flags=lanczos` + palettegen/paletteuse, `-loop 0` |
| MP3 / M4A / WAV / FLAC / OGG / OPUS | `libmp3lame <mp3_kbps>k` / `aac 192k` / `pcm_s16le` / `flac` / `libvorbis -q:a 6` / `libopus 128k` |

- 스트림 복사를 먼저 시도한다(해상도 옵션이 원본일 때만): MKV는 항상, MP4·MOV는 영상 ∈ {h264, hevc}·오디오 ∈ {없음, aac, mp3}, WEBM은 영상 ∈ {vp8, vp9, av1}·오디오 ∈ {없음, opus, vorbis}. AVI와 애니메이션 GIF 입력은 복사하지 않는다. 복사가 실패하면 인코딩으로 다시 한다.
- 임시 출력 이름이 `.swisswheel-tmp`로 끝나므로 먹서는 항상 `-f`로 명시한다 (MP4 `mp4`, MKV `matroska`, WEBM `webm`, MOV `mov`, AVI `avi`, GIF `gif`, MP3 `mp3`, M4A `ipod`, WAV `wav`, FLAC `flac`, OGG `ogg`, OPUS `opus`).
- 오디오 추출: aac→M4A, mp3→MP3, opus→OPUS, vorbis→OGG, flac→FLAC, pcm_s16le→WAV는 스트림 복사, 그 밖은 MP3 인코딩. 오디오가 없으면 "오디오 트랙이 없습니다".
- 음소거: 입력 포맷 유지(먹서가 없는 WMV는 MP4), 영상 스트림 복사 먼저.

### 3.2 도구
| 도구 | 대상 | 동작 |
|---|---|---|
| 이미지 축소 | 이미지 | 가로·세로 25/50/75% (기본 50%, 2.5), Lanczos3 |
| 이미지 압축 | JPG, PNG, WebP | JPG 품질 75 / PNG 무손실 최적화(`oxipng`) / WebP 품질 70 |
| 영상 압축 | 영상 | H.264, 강도별 CRF·오디오 (기본 CRF 28·128kbps, 2.5), 컨테이너 유지(MP4·MKV·MOV, 그 밖은 MP4) |
| 오디오 추출 | 영상 | 오디오 스트림 복사 가능하면 복사(M4A/OGG 등), 아니면 MP3 |
| 음소거 | 영상 | 오디오 스트림 제거, 영상 스트림 복사 |

### 3.3 동작 (모든 파일·폴더·섞인 선택에 사용 가능)
| 동작 | 구현 |
|---|---|
| 공유 | Windows 공유 창 (`IDataTransferManagerInterop::ShowShareUIForWindow`) |
| 복사 | 클립보드에 파일 목록 (CF_HDROP + Preferred DropEffect = COPY) |
| 잘라내기 | 클립보드에 파일 목록 (Preferred DropEffect = MOVE) |
| 경로 복사 | 전체 경로를 줄바꿈으로 이어 텍스트로 클립보드에 복사 |
| 터미널 | 첫 항목의 폴더(폴더면 그 자체)에서 Windows Terminal(`wt.exe -d`), 없으면 PowerShell 실행 |

### 3.4 여러 파일 드롭
- 변환/도구 휠에는 **모든 파일에 적용 가능한 항목의 교집합**만 표시한다.
- 폴더가 하나라도 포함되면 동작 휠만 쓸 수 있다.

## 4. 아키텍처

단일 프로세스, 트레이 상주. Rust (stable, MSVC 툴체인), `windows` 크레이트로 Win32/COM/WinRT 호출.

```
UI 스레드 (메시지 루프 하나)
 ├─ hook    WH_KEYBOARD_LL / WH_MOUSE_LL — 상태 추적, 메시지 게시만
 ├─ wheel   WS_EX_LAYERED|TOPMOST|TOOLWINDOW|NOACTIVATE 창, Direct2D + DirectComposition
 ├─ dnd     IDropTarget (OLE, STA) — CF_HDROP 파싱, DragOver 섹터 판정 전달
 └─ tray    Shell_NotifyIcon, 컨텍스트 메뉴, 풍선 알림
        │ Job 요청 (채널)
        ▼
작업 스레드 풀
 ├─ jobs     대기열·동시성 제한·출력 경로 결정·임시파일 처리
 ├─ engines  image (크레이트), ffmpeg (자식 프로세스, CREATE_NO_WINDOW)
 └─ actions  클립보드·공유·터미널 (공유/클립보드는 UI 스레드로 위임)

순수 로직 (OS 의존 없음, 단위 테스트 대상)
 ├─ catalog   입력 파일들 → 휠별 항목 목록
 ├─ geometry  휠 레이아웃, 점 → 섹터 판정
 ├─ naming    출력 경로 생성·충돌 회피
 └─ config    TOML 로드/기본값/검증
```

### 4.1 크레이트 구성
Cargo 워크스페이스 하나에 두 크레이트:
- `swisswheel-core` — catalog, geometry, naming, config, 작업 정의(`Job`, `Operation`), 엔진 인터페이스. OS 의존 없음, 순수 단위 테스트.
- `swisswheel` (bin) — hook, wheel, dnd, tray, jobs 실행기, engines, actions. Win32 의존.

### 4.2 드래그 감지 흐름
1. `WH_MOUSE_LL`이 왼쪽 버튼 down 위치를 기록. 버튼이 눌린 채로 시스템 드래그 임계값(`SM_CXDRAG/SM_CYDRAG`) 이상 움직이면 `drag_candidate = true`. 버튼 up 시 false.
2. `WH_KEYBOARD_LL`에서 `drag_candidate && open_key down` 이면 키를 삼키고(1 반환) UI 스레드에 `OpenWheel(cursor_pos)` 게시.
3. UI 스레드가 휠 창을 커서 중심에 **완전히 투명한 상태로** 표시하고(창 영역은 히트 테스트되므로 드롭 타깃으로 동작), `SendInput`으로 0픽셀 상대 이동을 주입해 OLE 드래그 루프가 즉시 히트 테스트하게 한다. 휠 그리기와 열림 애니메이션은 4단계의 `DragEnter`에서 시작한다 — 파일 드래그가 아니면 화면에 아무것도 보이지 않는다 (수동 확인에서 제목 표시줄 드래그 시 휠이 잠깐 깜빡이던 문제).
4. `IDropTarget::DragEnter`에서 `CF_HDROP` 파일 목록을 받아 catalog로 항목 계산 → 휠 채움.
5. 휠 표시 후 300ms 안에 `DragEnter`가 오지 않으면(파일 드래그가 아님 — 창 이동, 텍스트 선택 등) 휠을 닫는다. 이렇게 닫힌 뒤에는 `open_key`를 떼거나 왼쪽 버튼을 놓을 때까지 다시 열지 않고, 그동안의 `open_key` 입력은 삼키지 않고 원래 앱으로 통과시킨다 (스파이크 S6: 키 반복으로 약 330ms마다 다시 열리던 문제).
6. 휠이 열린 동안 `cycle_key`/`cancel_key`/`open_key`는 훅에서 삼키고 UI 스레드로 전달.
7. `DragLeave`(휠 밖으로 나감) 시 휠은 유지하되 강조 해제. 드래그가 휠 밖에서 끝나면 휠 닫힘. **주의:** 저수준 훅의 `WM_LBUTTONUP`은 OLE `Drop`보다 먼저 도착하므로, 버튼 up을 감지해도 즉시 닫지 말고 200ms 타이머 후 닫는다. 그 사이 `Drop`이 오면 `Drop` 처리에서 닫는다 (스파이크에서 검증; 즉시 닫으면 휠 위 드롭이 사라질 수 있음).
8. `Drop`: 지점의 섹터 → `Job` 생성 → 작업 풀로 전송. **`pdwEffect`는 항상 `DROPEFFECT_COPY`** (MOVE를 반환하면 소스가 원본을 삭제할 수 있음). 중앙 취소 영역이면 `DROPEFFECT_NONE`.

**훅 규칙:** 훅 콜백은 상태 갱신과 `PostMessage`만 하고 즉시 반환한다 (`LowLevelHooksTimeout` 초과 시 윈도우가 훅을 해제함). 훅이 해제된 것을 감지하면 재설치한다.

### 4.3 휠 렌더링
- DirectComposition 비주얼 + Direct2D 디바이스 컨텍스트로 픽셀 단위 알파를 그린다. 휠이 닫혀 있을 때는 창을 숨기고 D2D 리소스는 유지(재생성 비용 회피)하되, 10분간 사용이 없으면 해제해 대기 메모리를 줄인다.
- 텍스트는 DirectWrite (Segoe UI Variable, 없으면 Segoe UI). 칸은 시스템 아이콘 글꼴(Segoe Fluent Icons, Windows 10은 Segoe MDL2 Assets)의 아이콘 아래에 작은 이름을 쓴다 — 이미지 리소스를 넣지 않는다.
- Per-Monitor DPI Aware v2.
- 옵션 패널(2.5)은 별도 창이 아니라 휠 창을 패널 자리까지 넓혀 같은 표면에 그린다. 드롭 타깃이 하나라 휠 ↔ 패널 이동에 DragLeave/DragEnter가 생기지 않는다. 창 영역(`SetWindowRgn`)은 패널이 닫혀 있으면 원, 열려 있으면 원 ∪ 패널 사각형.

### 4.4 작업 실행
- 동시성: 이미지 작업은 `min(코어 수, 4)`개, FFmpeg 작업은 2개 동시 실행. 나머지는 대기열.
- FFmpeg: 앱 폴더의 `ffmpeg\ffmpeg.exe`(없으면 `PATH`의 ffmpeg/ffprobe)를 `-hide_banner -nostdin -progress pipe:1` 로 실행, `ffprobe`로 길이를 얻어 진행률 계산. 프로세스는 Job Object에 묶어 앱 종료 시 함께 종료. 앱을 종료하면 실행 중인 FFmpeg 작업은 즉시 취소하고(대기 중인 파일은 처리하지 않음) 임시 파일을 지운다 — 긴 영상 때문에 종료가 지연되지 않게.
- 엔진 인터페이스: `trait Engine { fn run(&self, job: &Job, progress: &dyn Fn(f32)) -> Result<PathBuf, EngineError>; }`
- 오류는 `thiserror` 기반 `EngineError`로 분류(입력 읽기 실패, 지원하지 않는 내용, 쓰기 실패, 엔진 비정상 종료) → 알림 문구로 매핑.

### 4.5 설정
`%APPDATA%\SwissWheel\config.toml` (없으면 기본값으로 생성):
```toml
open_key = "Space"
cycle_key = "Tab"
cancel_key = "Escape"
start_with_windows = false
[quality]
jpg = 90
webp = 85
mp3_kbps = 192
[progress]
window = true
tray_ring = true
```
- 파일이 깨졌으면 기본값으로 실행하고 알림으로 알린다 (파일은 덮어쓰지 않음).
- 트레이 메뉴: 시작 시 자동 실행(HKCU `Run` 키) 토글, 설정 파일 열기, 설정 다시 불러오기, 로그 폴더 열기, 정보, 종료.
- 중복 실행 방지: 이름 있는 뮤텍스.

## 5. 알려진 제한
- 관리자 권한으로 실행된 앱에서 시작한 드래그는 UIPI 때문에 받을 수 없다 (SwissWheel을 관리자로 실행하면 반대로 일반 앱 드래그를 못 받음 → 관리자 실행 비권장, 감지 시 경고).
- 일부 앱은 OLE 드래그 대신 자체 드래그를 쓰므로 휠이 뜨지 않을 수 있다 (300ms 타임아웃으로 조용히 닫힘).
- 전체 화면 게임 등 독점 전체 화면 위에는 휠이 표시되지 않을 수 있다.

## 6. 배포와 라이선스
- 앱 이름 SwissWheel, 라이선스 **GPL-3.0-or-later**.
- FFmpeg GPL 정적 빌드(win64)를 함께 배포하고, 해당 소스 위치와 라이선스를 `THIRD_PARTY_LICENSES`에 명시한다.
- 산출물: GitHub Releases에 포터블 zip + Inno Setup 설치 파일 (x64).
- CI: GitHub Actions (`windows-latest`)에서 `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, 릴리스 빌드.

## 7. 테스트 전략
- **단위 테스트 (`swisswheel-core`):** catalog(확장자·조합별 항목, 교집합, 폴더 포함 시 동작만), geometry(섹터 판정, 경계, 두 번째 고리, 화면 가장자리 보정), naming(충돌 회피, 도구 접미사), config(기본값, 잘못된 값).
- **통합 테스트:** `tests/fixtures/`의 작은 샘플 파일을 실제 엔진으로 변환하고, 결과 파일을 다시 디코드/ffprobe 해서 포맷을 확인. 원본 파일의 해시가 변하지 않았는지 확인.
- **수동 체크리스트 (`docs/manual-test.md`):** 탐색기 드래그 + Space/Tab/Esc, Shift·Ctrl·Alt 기존 드래그 무영향, 휠 밖 드롭, 멀티 모니터·DPI 125/150%, 바탕화면에서 드래그, 다크/라이트 테마, 대기 메모리 측정.

## 8. 개발 단계
0. **스파이크 (버림 코드):** 4.2의 1~8을 최소 구현으로 검증 — Space 감지, 휠 창 DragEnter/Drop 수신, 0픽셀 이동 주입 효과, Esc 삼킴 후 드래그 지속, `DROPEFFECT_COPY` 반환 시 원본 보존. 실패하는 항목이 있으면 설계를 수정한 뒤 진행.
1. 워크스페이스·CI·core 크레이트 (catalog, geometry, naming, config) + 단위 테스트.
2. 트레이·훅·휠 창·dnd 연결 (실제 작업 없이 드롭 결과 로그).
3. 이미지 엔진 + 작업 실행기 + 알림.
4. FFmpeg 엔진 (변환·도구).
5. 동작(공유·클립보드·터미널).
6. 설정·자동 실행·패키징(zip, Inno Setup)·문서.

## 9. v0.2 이후 후보
키보드 전용 모드(전역 단축키 + 탐색기 선택 항목 읽기), 설정 GUI, 트림·분할, HEIC/AVIF, PDF·문서, Float, ARM64, winget 등록.
