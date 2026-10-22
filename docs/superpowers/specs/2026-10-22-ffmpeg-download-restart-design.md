# FFmpeg 다운로드·재시작 타일 설계 문서

- 작성일: 2026-10-22
- 상태: 승인됨 (대화에서 합의), 사용자 검토 대기
- 기반: `main` (b0eee2b). 업데이터(`2026-10-22-updater-design.md`)의 HTTP·진행 표시·재시작 핸드오프를 재사용한다.

## 1. 목적과 범위

**목적:** 메뉴 바에 타일 두 개를 더한다.
- **다운로드:** 영상·오디오 변환에 필요한 FFmpeg(`ffmpeg.exe`, `ffprobe.exe`)를 받아 SwissWheel이 바로 쓰도록 설정한다. 이미 쓸 수 있으면 "다운로드 완료"만 알린다.
- **재시작:** SwissWheel을 끄고 다시 켠다.

**범위 밖:** FFmpeg 업데이트·삭제, 받을 빌드 고르기(essentials만), 시작할 때 자동 설치, 다른 도구 다운로드.

**성공 기준:**
1. FFmpeg가 없는 PC에서 다운로드 타일 → [예] 한 번이면, 앱을 다시 켜지 않아도 MP4 → MP3 변환이 된다.
2. 이미 FFmpeg를 쓸 수 있으면(앱 폴더든 `PATH`든) 아무것도 받지 않고 "다운로드 완료" 알림이 뜬다.
3. 받은 파일의 SHA-256이 GitHub가 알려 준 값과 다르면 아무것도 설치되지 않는다.
4. 어느 단계에서 실패하거나 앱을 꺼도, 반쯤 설치된 FFmpeg가 쓰이는 일이 없다.
5. 받는 동안에도 대기 메모리가 크게 늘지 않는다(받은 zip을 메모리에 다 올리지 않는다).
6. 재시작 타일을 누르면 트레이 아이콘이 잠깐 사라졌다 다시 생기고, 설정과 "Windows 시작 시 실행"은 그대로다. 트레이 아이콘은 하나뿐이다.

## 2. 사용자 경험

### 2.1 메뉴 바
타일 순서: **설정 · 다운로드 · 플러그인 · 업데이트 · 재시작**. 바 너비는 타일 수에서 계산되므로 다섯 칸으로 넓어진다. 키보드·마우스 동작은 기존 타일과 같다(←/→ 이동, Enter·↓·클릭으로 실행, 실행하면 바가 닫힘).

### 2.2 다운로드 타일

| 상황 | 동작 |
|---|---|
| FFmpeg를 이미 찾을 수 있음 (3.2의 위치 중 하나) | 알림 "다운로드 완료 · FFmpeg가 이미 준비되어 있습니다" |
| 업데이트나 FFmpeg 다운로드가 진행 중 | 알림 "이미 업데이트나 다운로드를 진행하는 중입니다" |
| 그 밖 | 백그라운드에서 최신 FFmpeg 릴리스 확인 (2.3) |

변환 중이어도 받을 수 있다. 실행 중인 작업은 시작할 때 이미 FFmpeg를 찾았으므로 영향이 없다.

### 2.3 확인과 받기
- 최신 릴리스는 GitHub `GyanD/codexffmpeg`의 "latest"를 쓴다. 자산 중 이름이 `-essentials_build.zip`으로 끝나는 것을 고른다.
- 확인 창: `MessageBoxW`(`MB_YESNO | MB_ICONINFORMATION | MB_TOPMOST | MB_SETFOREGROUND`), 제목 "SwissWheel 다운로드", 본문 "FFmpeg 9.0.2(약 115MB)를 받아 설정할까요?\n\n설치 위치: <폴더>". 버전은 태그를 그대로 쓰고, 크기는 자산의 `size`를 MB로 반올림한다. [아니요]는 아무 일도 하지 않는다.
- 진행은 기존 진행 표시(진행 창, 트레이 링, 툴팁)에 한 줄: 파일 이름 "FFmpeg 9.0.2", 작업 "다운로드". 툴팁은 "SwissWheel · FFmpeg 받는 중 · 42%".
- 결과 알림 (제목 "다운로드"):

| 결과 | 알림 |
|---|---|
| 설치 끝 | "FFmpeg 9.0.2 설정을 마쳤습니다" (태그 끝 숫자에 따라 조사가 바뀌지 않게) |
| 확인 실패 (네트워크, HTTP 오류, JSON 해석 실패) | "FFmpeg 정보를 확인할 수 없습니다" |
| 릴리스에 essentials zip이 없거나 `digest`가 없음 | "받을 FFmpeg 파일을 찾을 수 없습니다" |
| 받기 실패 (끊김, 타임아웃, 크기 상한) | "FFmpeg를 받지 못했습니다" |
| SHA-256 불일치 | "받은 FFmpeg 파일이 손상되었습니다" |
| zip 안에 `ffmpeg.exe`나 `ffprobe.exe`가 없음, 풀기·옮기기 실패 | "FFmpeg를 설치하지 못했습니다" |

원인은 모두 로그에 남긴다.

### 2.4 재시작 타일

| 상황 | 동작 |
|---|---|
| 변환 진행 중 | 알림 "진행 중인 변환이 끝난 뒤 다시 눌러 주세요" |
| 업데이트나 FFmpeg 다운로드 진행 중 | 알림 "업데이트나 다운로드가 끝난 뒤 다시 눌러 주세요" |
| 업데이트 교체를 기다리는 중 (`pending_swap`) | 위와 같은 알림 |
| 그 밖 | 새 프로세스를 띄우고 지금 앱은 평소대로 종료 (3.5). 새 앱이 알림 "다시 시작했습니다" |
| 새 프로세스를 띄우지 못함 | 알림 "다시 시작하지 못했습니다". 지금 앱은 계속 실행 |

## 3. 구조

### 3.1 모듈

| 위치 | 책임 |
|---|---|
| `swisswheel-core/src/menu.rs` | `Tile::Download`, `Tile::Restart`, 라벨, `TILES` 순서 |
| `swisswheel-core/src/ffmpeg_release.rs` (새로) | 순수 로직: 릴리스 JSON → `FfmpegRelease{tag, zip_url, size, sha256}`, `digest`("sha256:<64자>") 해석, 확인 창 본문, 알림 문구, 다운로드·재시작 타일 판단 |
| `swisswheel-core/src/update.rs` | `--after-update`와 같은 꼴의 `--restarted <pid>` 인자 해석 |
| `swisswheel/src/update/http.rs` | `get`의 본문 처리를 `io::Write` 대상으로 일반화한 `get_to`. `get`은 `Vec`에 쓰는 `get_to`가 된다 |
| `swisswheel/src/ffmpeg_install.rs` (새로) | 다운로드 스레드: 확인 → 확인 창 → 파일로 받기(받으며 해시) → 검사 → 풀기 → 옮기기. 결과를 UI 스레드로 `PostMessage` |
| `swisswheel/src/engines/ffmpeg.rs` | `locate_in`이 LocalAppData 위치도 본다 (3.2) |
| `app.rs` | 두 타일 연결, 진행 행 합치기, 재시작, 시작 시 `--restarted` 처리·`ffmpeg.partial` 정리 |

- 새 의존성: `zip`(`default-features = false`, `deflate`만).
- 엔드포인트 `https://api.github.com/repos/GyanD/codexffmpeg/releases/latest`는 상수다. 디버그 빌드에서만 `SWISSWHEEL_FFMPEG_URL`로 바꿀 수 있다(로컬 서버 시험용, `SWISSWHEEL_UPDATE_URL`과 같은 방식).

### 3.2 설치 위치와 찾는 순서
- 설치 위치: exe 폴더에 쓸 수 있으면(`swap::writable`과 같은 검사) `<exe 폴더>\ffmpeg\`, 아니면 `%LOCALAPPDATA%\SwissWheel\ffmpeg\`.
- `locate()` 찾는 순서: `<exe 폴더>\ffmpeg\` → `%LOCALAPPDATA%\SwissWheel\ffmpeg\` → `PATH`. 각 위치는 지금처럼 `ffmpeg.exe`와 `ffprobe.exe`가 **둘 다** 있어야 인정한다.
- 작업 러너(`jobs.rs`)는 FFmpeg를 아직 못 찾았으면 미디어 파일마다 `locate()`를 다시 부르고, 찾으면 그 뒤로 계속 쓴다. 그래서 설치가 끝난 뒤의 변환은 재시작 없이 새 FFmpeg를 쓴다.

### 3.3 받기와 설치 순서 (다운로드 스레드)
`target` = 3.2의 설치 폴더, `partial` = 같은 부모 폴더의 `ffmpeg.partial\`. 같은 볼륨이라 이름 바꾸기가 원자적이다.

1. 릴리스 JSON을 받는다(상한 1MB). zip 자산과 `digest`를 고른다.
2. 확인 창. [아니요]면 끝.
3. `partial`을 지우고 새로 만든다. zip을 `partial\download.zip`에 받는다(상한 512MB). 받는 조각마다 파일에 쓰면서 SHA-256을 갱신한다. 진행률은 업데이터와 같은 `Progress`(0..=10000)로 공유한다.
4. 해시가 `digest`와 다르면 `partial`을 지우고 "손상" 결과.
5. zip에서 경로가 `bin/ffmpeg.exe`, `bin/ffprobe.exe`로 끝나는 항목, 그리고 맨 위 폴더에 `LICENSE`가 있으면 그것을 `partial\`에 푼다. 다른 항목은 무시한다. 항목 이름은 경로로 쓰지 않고 정해진 파일 이름으로만 쓴다(zip 경로 조작 방지). 두 exe 중 하나라도 없으면 "설치 실패".
6. `target`을 만들고(`create_dir_all`) `ffprobe.exe`, `LICENSE`, `ffmpeg.exe` 순서로 `partial`에서 `target`으로 옮긴다(`MOVEFILE_REPLACE_EXISTING`). 둘 다 옮겨지기 전에는 `locate()`가 이 폴더를 인정하지 않는다.
7. `partial`을 지운다.

- 앱이 받는 도중 꺼지면 스레드도 함께 끝난다(업데이터와 같음). 남은 `partial`은 다음 시작 때 3.2의 두 위치에서 지운다.
- 실패한 단계 뒤에는 언제나 `partial`을 지운다. 지우지 못하면 로그만 남기고 다음 시작 때 다시 지운다.

### 3.4 스레드와 상태
- 확인 창은 다운로드 스레드에서 띄운다(업데이터 3.2와 같은 이유: UI 스레드의 모달 루프가 `with_state` 재진입 메시지를 버림).
- 앱 상태에 `ffmpeg: Option<ffmpeg_install::Handle>`을 둔다. 업데이트와 FFmpeg 다운로드는 동시에 하나만 돈다. 업데이트 타일 판단(`on_tile`)도 FFmpeg 다운로드가 돌면 "이미 진행하는 중" 알림을 낸다.
- `update_busy`는 업데이트 행처럼 FFmpeg 행을 진행 표시에 합친다. 행은 사용자가 [예]를 누른 뒤에만 생긴다.

### 3.5 재시작 순서 (UI 스레드)
1. 2.4의 거절 조건을 본다.
2. `exe --restarted <지금 pid>`를 실행한다(`swap::launch_after_update`와 같은 꼴의 `launch_restarted`). 실패하면 알림 후 끝.
3. `WM_CLOSE`를 post해서 지금 앱이 평소 종료 절차(작업 러너 정리, 트레이 아이콘 제거, 훅 해제)를 거친다.
4. 새 앱은 `--after-update`처럼 단일 인스턴스 뮤텍스를 잡기 **전에** 옛 프로세스가 끝나기를 기다린다(같은 `wait_for_exit`, 최대 15초). 시작이 끝나면 알림 "다시 시작했습니다".

## 4. 오류 처리와 안전
- HTTPS만 쓴다(기존 `http.rs` 규칙 그대로: 디버그·테스트 빌드만 `http://127.0.0.1` 허용).
- 해시는 GitHub API가 HTTPS로 알려 준 `digest`와 비교한다. 해시가 맞기 전에는 아무것도 풀지 않는다.
- 한계: "항상 최신"이므로 SwissWheel이 시험하지 않은 FFmpeg 버전이 설치될 수 있다. HTTPS와 해시는 전송 중 손상·변조를 막지만, GyanD 계정이나 GitHub 자체가 탈취되면 막지 못한다.
- GitHub API 비인증 한도(시간당 60회)는 손으로 누르는 버튼에 충분하다. 403/429는 "확인할 수 없습니다".
- FFmpeg essentials 빌드는 GPL이다. zip의 `LICENSE`를 설치 폴더에 함께 둔다.

## 5. 테스트

**자동 (`cargo test`):**
- **`menu.rs`**: 다섯 타일 순서와 끝에서 멈춤. Enter로 `Notice(Tile::Download)`, `Notice(Tile::Restart)`. 기존 테스트의 타일 번호를 새 순서에 맞춘다.
- **`ffmpeg_release.rs`**:
  - 실제 GyanD 응답 모양에서 essentials zip, 크기, digest 고르기.
  - essentials zip이 없음, `digest`가 없음, `digest`가 `sha256:`가 아님, 길이가 틀림.
  - 확인 창 본문의 크기 반올림.
  - 다운로드 타일 판단: 이미 있음 / 진행 중 / 시작.
  - 재시작 타일 판단: 변환 중 / 업데이트·다운로드 중 / 교체 대기 / 시작.
- **`update.rs`**: `--restarted 1234`, 숫자 아님, `--after-update`와 섞이지 않음.
- **`http.rs`**: `get_to`가 파일에 쓰고 진행 콜백을 부르는지(기존 `test_server`).
- **`ffmpeg_install.rs`**: 테스트 안에서 `zip`의 쓰기 기능(dev 의존성)으로 작은 zip을 만든다.
  - 끝에서 끝까지: 가짜 서버의 JSON과 zip → `target`에 세 파일, `partial` 없음.
  - 해시 불일치 → `target` 없음, `partial` 없음.
  - zip에 `ffprobe.exe`가 없음 → 설치 실패, `target` 없음.
  - `../evil.exe` 같은 항목은 무시.
  - `target`에 `ffmpeg.exe`만 있던 경우 → 둘 다 갖춰짐.
- **`engines/ffmpeg.rs`**: `locate_in` 순서(exe 폴더 → LocalAppData → `PATH`)와 한쪽만 있는 폴더 건너뛰기.

**수동 (`docs/manual-test.md`에 추가):**
- PATH에서 FFmpeg를 뺀 상태로 다운로드 → [예] → 진행 창 → 완료 알림 → 재시작 없이 MP4 → MP3.
- FFmpeg가 PATH에 있을 때 다운로드 → "다운로드 완료" 알림, 네트워크 요청 없음(로그).
- 디버그 빌드 + `SWISSWHEEL_FFMPEG_URL` 로컬 서버로 손상 파일 → 손상 알림, 설치 폴더 없음.
- 받는 도중 트레이 → 종료 → 다시 켜면 `ffmpeg.partial` 없음.
- exe를 Program Files에 두고 다운로드 → `%LOCALAPPDATA%\SwissWheel\ffmpeg\`에 설치, 변환 됨.
- 재시작 → 트레이 아이콘 하나, "다시 시작했습니다" 알림, 설정 그대로. 변환 중 재시작 → 거절 알림.

## 6. 문서
- README: FFmpeg 안내를 "메뉴 → 다운로드로 받거나, …"로 바꾸고 재시작 타일을 적는다.
- 메뉴 설계 문서(`2026-10-21-desktop-menu-design.md`)의 타일 목록에 두 타일을 더하고 이 문서를 링크한다.
