# 영상 편집기 설계 문서

- 작성일: 2026-10-26
- 상태: 승인됨 (대화에서 합의), 사용자 검토 대기
- 기반: `main` (6f02503). 사진 편집기(`2026-10-25-image-editor-design.md`)의 편집 패널 틀(`editor/`, `editor_control.rs`, `editor_geometry.rs`, `crop.rs`), FFmpeg 엔진(`engines/ffmpeg.rs`, `media.rs`), 작업 실행기(`jobs.rs`)를 재사용한다. 그 문서 7장의 엔진 결정을 이어받는다.

## 1. 목적과 범위

**목적:** 영상 한 개를 드래그해 도구 휠의 **✎편집**에 놓으면 사진 편집기와 같은 떠 있는 패널이 열린다. 소리까지 재생하면서 타임라인에서 필요 없는 구간을 여러 군데 빼고, 영역 자르기·회전·반전·배속·소리·역재생을 쌓은 뒤 한 번에 저장한다. 지금 장면은 PNG로 바로 저장할 수 있다. 원본은 바꾸지 않는다.

**범위:**
- 영상 미리보기: Windows 재생 엔진(Media Foundation)으로 소리까지 재생, 못 여는 파일은 FFmpeg로 장면 넘겨보기.
- 타임라인: 나누기·조각 빼기/되살리기, 재생 위치 이동.
- 도구: 영역 자르기(비율 칩), 회전, 좌우·상하 반전, 배속(0.5×·1.5×·2×), 소리(끄기·크기 맞추기), 역재생(짧은 영상만), 장면 PNG 저장.
- 되돌리기·다시 하기, 저장·취소(사진 편집기와 같은 규칙).
- 사진 편집기와 공유하는 부분의 작은 정리(3.1).

**범위 밖:**
- 무손실 빠른 자르기(키프레임 맞춤). 저장은 항상 정확한 지점에서 다시 인코딩한다.
- 조각 순서 바꾸기, 여러 영상 이어 붙이기, 조각 경계 끌어서 옮기기.
- 역재생·소리 크기 맞추기의 미리 듣기/보기(저장 결과에만 적용).
- 긴 영상의 조각 역재생, 오디오 파일 편집, 움직이는 GIF 편집.
- 자막·글자·색 보정·크기 조절(휠의 축소 도구가 있다)·화질 선택.
- 타임라인 썸네일·파형.
- 원본 덮어쓰기.

## 2. 사용자 경험

### 2.1 여는 방법
- 영상 **한 개**(MP4·MKV·WebM·MOV·AVI·WMV)를 드래그해 Space → 도구 휠 맨 끝에 **✎편집** 타일. 여러 개·폴더·움직이는 GIF·오디오면 없다.
- FFmpeg가 필요하다. 다른 영상 도구 타일과 같은 확인 경로(없으면 기존 설치 안내, 설치 중이면 기존 거절 문구)를 탄다.
- 놓으면 패널이 열린다(사진 편집기 2.1·2.2와 같은 창, 같은 열기 흐름). 영상 정보를 읽고 재생 엔진을 여는 동안 미리보기 자리에 "불러오는 중", 도구는 눌리지 않는다.

### 2.2 패널
```
╭──────────────────────────────────────────────────────────────────╮
│ ✂영역 ↻회전 ⇆좌우 ⇅상하 ⏩1× 🔊소리 ⇄역재생 📷장면          clip.mp4 │
│ [자유][원본][1:1][4:3][3:4][16:9][9:16]       ← 영역 자르기 중에만 │
│                                                                  │
│                       [ 영상 미리보기 ]                          │
│                                                                  │
│ ▶ 0:12.4 / 1:30.0  ┃█████▌░░░░▌██████████▌░░▌████┃   ✂나누기 ⊖빼기 │
│ ↶ ↷   1920×1080 · 1:30 → 1080×1080 · 0:41 · 2×     Esc 취소 ↵ 저장 │
╰──────────────────────────────────────────────────────────────────╯
```
- 크기·위치·어둡게 하는 창·포커스·테마는 사진 편집기와 같다. 패널 비율은 영상의 화면 기준 크기(회전 메타데이터 적용, 2.9)로 정하고, 하단에 타임라인 줄 한 줄이 더 있다.
- 하단 가운데: `원본 크기 · 원본 길이 → 결과 크기 · 결과 길이 · 배속`. 편집이 없으면 원본만. 결과 길이는 남은 길이 ÷ 배속.
- 도구 모음 버튼 8개. 배속 버튼은 지금 배속(`1×`)을 글자로 보이고, 소리 버튼은 지금 상태 아이콘(소리·음소거·크기 맞춤)을 보인다.

### 2.3 재생과 타임라인
| 키 | 동작 |
|---|---|
| Space, [▶] | 재생/멈춤. 끝에서 누르면 처음(첫 남은 조각)부터 |
| ← / → | 한 프레임 뒤/앞(멈춤 상태로) |
| Shift+← / Shift+→ | 1초 뒤/앞 |
| Home / End | 처음 / 끝 |

- 타임라인 막대는 **원본 전체 길이**를 그린다. 남은 조각은 채움, 뺀 조각은 빗금·흐림, 나눔 지점은 세로 눈금, 재생 위치는 강조색 세로선.
- 막대를 누르면 그 위치로 가고, 누른 채 끌면 따라간다(재생 중이면 끄는 동안 멈췄다가 놓으면 이어 재생).
- 재생 중 뺀 조각에 들어가면 다음 남은 조각의 시작으로 건너뛴다. 마지막 남은 조각 끝에 닿으면 멈춘다.
- 재생 속도는 고른 배속을 따른다. 소리 "끄기"면 미리보기도 음소거.
- 멈춘 상태에서 뺀 조각 위에 있으면 장면을 흐리게 그리고 가운데에 "빠진 구간"을 보인다.
- 시간 표시는 `분:초.십분의일초`(1시간 이상이면 `시:분:초.십분의일초`).

### 2.4 구간 빼기
- **S** / [✂나누기]: 재생 위치에서 조각을 나눈다. 영상 처음·끝이나 기존 나눔 지점에서 0.1초 안쪽이면 아무것도 하지 않는다.
- **D** / [⊖빼기]: 재생 위치가 있는 조각을 뺀다. 이미 뺀 조각이면 되살린다. 버튼 글자도 [⊕되살리기]로 바뀐다.
- 남은 조각이 하나뿐일 때 그 조각을 빼려 하면 하단에 "조각을 모두 뺄 수는 없습니다"(3초).
- 나누기·빼기·되살리기는 모두 되돌리기 기록에 쌓인다. 나누기만 하고 빼지 않으면 결과에는 영향이 없다(편집 없음으로 친다).

### 2.5 도구
| 도구 | 키 | 동작 |
|---|---|---|
| ✂ 영역 | C | 사진 편집기 2.4와 같은 자르기 모드(손잡이·비율 칩·Enter 적용·Esc 취소). 자를 크기는 짝수 픽셀로 맞춘다(H.264·VP9 요구). 최소 16×16. 자르기 모드 동안 재생은 멈추고 타임라인은 쓸 수 있다 |
| ↻ 회전 | R / Shift+R | 시계 / 반시계 방향 90° |
| ⇆ 좌우 · ⇅ 상하 | H · V | 반전 |
| ⏩ 배속 | `]` 빠르게 · `[` 느리게, 버튼은 순환 | 0.5× · 1× · 1.5× · 2× 네 단계. `[`·`]`는 끝에서 멈춘다. 버튼은 1× → 1.5× → 2× → 0.5× → 1× |
| 🔊 소리 | M | 그대로 → 끄기 → 크기 맞추기 → 그대로 순환. 크기 맞추기에 마우스를 올리거나 바꾼 직후 하단에 "소리 크기는 저장할 때 맞춥니다". 소리 트랙이 없으면 흐리고 눌리지 않는다 |
| ⇄ 역재생 | B | 켬/끔 토글. 미리보기는 정방향 그대로, 미리보기 왼쪽 위에 "역재생" 표시 |
| 📷 장면 | F | 지금 장면을 바로 저장(2.7) |

- 회전·반전·영역 자르기는 미리보기에 바로 보인다(사진 편집기와 같은 변환 그리기).
- **역재생 한도:** `결과 가로 × 결과 세로 × 1.5 × fps × 남은 길이`가 1 GiB를 넘거나 셋 중 하나라도 모르면 켤 수 없다. 하단에 "역재생은 짧은 영상만 할 수 있습니다 (1080p 기준 약 10초). 구간을 줄이면 켤 수 있어요"(3초). 켠 뒤 조각을 되살리거나 영역을 넓혀 한도를 넘으면 저장할 때 같은 문구로 막는다(저장은 하지 않고 패널은 열린 채).
- 영역 자르기 모드 동안 다른 도구·나누기·빼기·되돌리기는 흐리고 눌리지 않는다(사진 편집기와 같다). 재생·탐색은 된다.

### 2.6 대체 모드 (Windows가 못 여는 파일)
- 재생 엔진이 파일을 열지 못하거나 재생 중 오류를 내면 대체 모드로 바뀐다. 편집 기록은 그대로다.
- [▶]·Space는 흐리고, 하단에 처음 한 번 "재생할 수 없는 형식이라 장면만 넘겨봅니다"(3초, 마우스를 ▶에 올리면 다시).
- 타임라인 이동·프레임 이동·Shift+화살표는 FFmpeg로 그 시점 장면 한 장을 받아 보인다. 받는 동안은 앞 장면을 그대로 둔다.
- 대체 모드의 프레임 이동은 `1/fps`초씩 시간을 옮긴다(fps를 모르면 1/30초).

### 2.7 장면 저장
- **F** / [📷장면]: 지금 재생 위치의 장면을, 쌓인 영역 자르기·회전·반전을 적용해 원본 해상도 PNG로 저장한다. 배속·소리·역재생·구간은 상관없다.
- 재생 중이면 멈추고 저장한다. 패널은 열린 채, 진행·결과 카드는 다른 작업과 같다.
- 이름: `clip (frame 0m12.40s).png`(원본 옆, 겹치면 기존 번호 규칙). 1시간 이상이면 `1h02m03.40s`.
- 편집 기록에 쌓이지 않고 되돌릴 수 없다. 편집이 없어도 할 수 있다.

### 2.8 저장·취소·되돌리기
- 사진 편집기 2.5·2.7·2.8과 같다: Ctrl+Z / Ctrl+Y(Ctrl+Shift+Z), Enter·[↵ 저장](편집이 없으면 흐림), Esc·[Esc 취소](편집이 있으면 두 번 확인 3초).
- 편집 있음 = 뺀 조각이 하나 이상, 또는 영역·회전·반전, 배속 ≠ 1×, 소리 ≠ 그대로, 역재생 켬. 나누기만은 편집이 아니다.
- 저장하면 패널을 닫고 작업 실행기로 넘긴다. 결과는 원본 옆 `clip (edited).mp4`, 원본과 같은 컨테이너·기존 변환 설정의 코덱:
  - MP4·MOV·MKV: H.264 CRF 23 (`-preset medium`, yuv420p) + AAC 192k, MP4·MOV는 `+faststart`.
  - WebM: VP9 CRF 32 + Opus 128k.
  - AVI: mpeg4 `-q:v 4` + MP3(설정의 비트레이트).
  - **WMV는 인코딩 설정이 없으므로 MP4로 저장한다**(`clip (edited).mp4`).
- 진행률은 기대 출력 길이(남은 길이 ÷ 배속) 기준.

### 2.9 회전 메타데이터
- 휴대폰 세로 영상처럼 회전 메타데이터(display matrix)가 있으면 ffprobe가 읽은 회전으로 **화면 기준** 가로·세로를 정한다. 패널 크기, 영역 자르기 좌표, 결과 크기 표시는 모두 화면 기준이다.
- 재생 엔진이 회전이 적용되지 않은 장면을 주면(장면 크기의 가로·세로가 화면 기준과 뒤바뀌어 있으면) 그리기에서 메타데이터만큼 돌린다.
- 저장·장면 저장은 FFmpeg의 기본 자동 회전에 맡긴다. 그래서 필터의 좌표도 화면 기준이다.

### 2.10 다른 기능과의 관계
- 사진 편집기 2.9와 같다. 패널은 사진·영상 합쳐 한 번에 하나. 열린 동안 Ctrl 두 번 무시, 두 번째 ✎편집은 기존 패널을 앞으로 + "편집기가 이미 열려 있습니다", 재시작은 거절, 업데이트 교체는 닫을 때까지 대기.
- 장면 저장 작업이 도는 중에 패널을 닫아도 작업은 끝까지 간다.

## 3. 구조

### 3.1 core (`swisswheel-core`)
- **`editor.rs` 정리** (사진 동작은 바꾸지 않는다, 기존 테스트로 지킨다):
  - 되돌리기 기록을 `History<Op>`(`push`, `undo`, `redo`, `can_undo`, `can_redo`, `ops`)로 떼어 `EditSession`이 쓴다.
  - 자르기·회전·반전을 접어 결과 크기와 `Affine`을 구하는 계산을 `frame_geometry(original, ops) -> (size, Affine)`(`EditOp::Crop/Rotate/Flip`만 보고 나머지는 건너뜀)로 뗀다. `EditSession::size()`·`to_result()`가 이를 부른다.
- **`catalog.rs`:** `items_for`가 입력이 정확히 하나이고 영상(`MediaKind::Video`)일 때도 끝에 `Edit`. FFmpeg 필요 여부는 다른 영상 도구와 같은 판정(`essentials`의 타일 결정)이 `Tool::Edit` + 영상 입력을 FFmpeg 작업으로 본다.
- **새 `video_edit.rs`:**
  - `Speed { Half, Normal, OneHalf, Double }` (`factor()`), `AudioMode { Keep, Mute, Normalize }`.
  - `VideoOp { Frame(EditOp), Split(f64), Toggle(f64), Speed(Speed), Audio(AudioMode), Reverse(bool) }`. `Frame`은 `Crop`·`Rotate`·`Flip`만. `Toggle(t)`는 시각 `t`가 든 조각의 남김/뺌을 뒤집는다(조각 번호는 나누기로 바뀌므로 시각으로 가리킨다).
  - `Segment { start, end, kept }`.
  - `VideoSession::new(info: VideoInfo)` (`VideoInfo { width, height, duration, fps: Option<f64>, has_audio }`, 화면 기준 크기):
    - `push(op) -> Result<bool, Refusal>`: 같은 값 다시 쌓지 않음(`false`), 0.1초 규칙(`false`), 마지막 조각 빼기 → `Refusal::LastSegment`, 역재생 한도 → `Refusal::ReverseTooLong`, 소리 없는 영상의 `Audio` → `false`.
    - `undo`, `redo`, `can_undo`, `can_redo`.
    - `segments()`, `kept_ranges() -> Vec<(f64, f64)>`, `kept_duration()`, `output_duration()`(÷ 배속), `size()`·`to_result()`(`frame_geometry`), `speed()`, `audio()`, `reverse()`.
    - `segment_at(t)`, `is_kept(t)`, `skip_from(t) -> Option<f64>`(뺀 조각 안이면 다음 남은 조각 시작, 없으면 `None`=끝), `first_kept_start()`.
    - `reverse_fits() -> bool`(2.5 공식, 1 GiB = 1024³).
    - `has_edits()`(2.8), `plan() -> Option<VideoPlan>`.
  - `VideoPlan { kept: Vec<(f64, f64)>, frame: Vec<EditOp>, speed, audio, reverse }` — 시간 범위는 원본 기준 초.
  - `text`: 2장의 문구 상수.
- **새 `video_control.rs`** (창과 무관, 단위 테스트):
  - `VideoCommand`: 재생 토글, 프레임 ±1, 1초 ±, 처음·끝, 나누기, 빼기 토글, 도구들, 되돌리기·다시 하기, 저장, 취소, 장면 저장, 자르기 모드 명령.
  - `video_key(vk, shift, ctrl) -> Option<VideoCommand>`.
  - `VideoState`: 로딩/준비, `VideoSession`, 자르기 모드(`editor_control::CropMode` 재사용), 취소 확인(사진 쪽 로직 재사용), 일시 안내 문구(3초), 재생 여부·재생 위치·대체 모드 여부. `command`·`click`·`key_other`·`timeline_press/move/release`·`footer_text`가 `Outcome`(다시 그리기, 플레이어에 할 일, 저장, 장면 저장, 닫기)을 돌려준다. 플레이어 동작은 `PlayerAction { Play, Pause, Seek(t), Rate(f64), Mute(bool) }` 목록으로 돌려줘 앱이 그대로 실행한다.
  - 사진 쪽 `editor_control.rs`에서 취소 확인·일시 안내 부분을 둘이 함께 쓰도록 작은 구조체로 뗀다(동작 그대로).
- **`editor_geometry.rs`:**
  - `video_layout(work_area, scale, video) -> VideoLayout`: 사진 배치에 타임라인 줄(`TIMELINE_ROW`)을 더한 것. 도구 8개, 칩 줄, 미리보기, 타임라인(재생 버튼, 시간 글자, 막대, 나누기·빼기 버튼), 하단 줄.
  - `video_hit_test(layout, cropping, x, y) -> VideoHit`.
  - `Timeline { bar: Box2, duration }`: `time_at(x)`(막대 밖은 양 끝으로), `x_of(t)`.
- **`media.rs`:**
  - `MediaInfo`에 `width`, `height`(화면 기준), `fps: Option<f64>`(`avg_frame_rate`, 0/0이면 `r_frame_rate`), `sample_rate: Option<u32>`, `rotation: i32`(90 단위로 정규화). 첫 영상·첫 소리 스트림 기준. ffprobe 인자에 `stream=width,height,avg_frame_rate,r_frame_rate,sample_rate:stream_side_data=rotation`을 더하고 `parse_probe`가 읽는다. 기존 필드·동작은 그대로.
  - `video_edit_plan(input: Format, info, plan: &VideoPlan, quality) -> Result<MediaPlan, PlanError>`:
    - 출력 형식: 입력과 같은 컨테이너, WMV는 MP4.
    - 구간: 남은 조각이 하나면 입력 앞 `-ss a -to b`(다시 인코딩이므로 정확), 여럿이면 `filter_complex`에서 `split`/`asplit` 후 조각마다 `trim=start:end,setpts=PTS-STARTPTS` / `atrim=…,asetpts=PTS-STARTPTS`, `concat=n=N:v=1:a=1`(소리 끄기·소리 없음이면 `a=0`).
    - 영상 사슬(위 다음): `frame` 순서대로 `crop=w:h:x:y`, `transpose=1`(시계)/`transpose=2`(반시계), `hflip`, `vflip` → 역재생이면 `reverse` → 배속이 1×가 아니면 `setpts=PTS/f` → 마지막에 짝수 크기 보정(`scale=trunc(iw/2)*2:trunc(ih/2)*2`).
    - 소리 사슬: 역재생이면 `areverse` → 배속이면 `atempo=f` → 크기 맞추기면 `loudnorm=I=-16:TP=-1.5:LRA=11` + 출력 `-ar 원본 샘플레이트`(모르면 48000). WebM은 기존 `OPUS_LAYOUTS` 변환을 이 사슬 끝에 붙인다(별도 `-af`를 쓰지 않는다). 소리 끄기면 `-an`.
    - 코덱 인자는 기존 `Video` 설정을 쓰되, `-vf`/`-af` 대신 이 그래프의 출력 라벨을 `-map`한다(기존 `Video::args`를 그래프 입력을 받도록 나눈다, 기존 변환 인자는 바뀌지 않음을 테스트로 지킨다).
    - `duration_secs`: 남은 길이 ÷ 배속. 시도는 하나.
  - `video_frame_plan(info, t, frame: &[EditOp]) -> MediaPlan`: `-ss t` 입력 앞, `-frames:v 1`, 영상 사슬(자르기·회전·반전만), PNG 출력.
  - 대체 모드 장면 인자: `scrub_args(t, max_w, max_h)` → `-ss t -i … -frames:v 1 -vf scale=…:force_original_aspect_ratio=decrease -f rawvideo -pix_fmt bgra pipe:1`(앱이 크기를 알 수 있게 scale 결과 크기를 미리 계산해 짝수로 고정).
- **`naming.rs`:** 장면 저장 suffix `frame 0m12.40s`(`frame_suffix(t)`), 편집은 기존 `edited`.

### 3.2 앱 (`swisswheel`)
- **`editor/mod.rs`:** 패널 하나의 수명 틀은 그대로 두고 내용을 `enum Content { Image(..), Video(..) }`로 나눈다. `open(path, owner, notify, accent)`이 형식으로 고른다. 앱으로 보내는 신호에 `SavedVideo(path, VideoPlan, Format)`, `SaveFrame(path, t, Vec<EditOp>)`를 더한다. `window.rs`(어둡게 하는 창·패널·IME 끄기·포커스)는 함께 쓴다.
- **새 `editor/video.rs`:** 영상 패널의 상태(`VideoState`), 메시지 처리(키·마우스·타이머·플레이어 알림), 플레이어/대체 모드 전환. 열기: 백그라운드에서 `ffmpeg::probe`(실패면 닫고 실패 카드) → 패널 크기 → 창 → 플레이어 열기.
- **새 `editor/player.rs`** — Media Foundation:
  - 처음 쓸 때 `MFStartup` 한 번(앱 종료까지 유지).
  - 패널 전용 D3D11 장치(`BGRA_SUPPORT | VIDEO_SUPPORT`, `ID3D10Multithread::SetMultithreadProtected`)와 `MFCreateDXGIDeviceManager`. 패널의 D2D 장치 컨텍스트도 이 장치 위에 만든다.
  - `IMFMediaEngineClassFactory::CreateInstance`(`MF_MEDIA_ENGINE_DXGI_MANAGER`, `MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT = BGRA`, 프레임 서버 모드). 알림은 `IMFMediaEngineNotify`를 구현한 COM 객체가 패널 창에 `PostMessage`(로드됨, 오류, 끝남, 탐색 끝).
  - 기능: `open(path)`, `play`, `pause`, `seek(t)`, `set_rate(f)`, `set_muted(b)`, `current_time()`, `frame_step(forward)`(`IMFMediaEngineEx::FrameStep`), `native_size()`.
  - 그리기: 재생 중·탐색 직후에는 16ms 타이머마다 `OnVideoStreamTick`을 보고 새 장면이면 `TransferVideoFrame`으로 장면 크기의 BGRA 텍스처에 받는다. 그 텍스처의 DXGI 표면으로 D2D 비트맵을 만들어 둔다(한 번 만들고 재사용).
  - 재생 중 타이머마다 `video_control`에 재생 위치를 알리고, `skip_from`이 위치를 돌려주면 `seek`, 끝이면 `pause`.
- **새 `editor/scrub.rs`:** 대체 모드 장면 받기. 작업 스레드 하나, 요청 슬롯 하나(마지막 요청만), `ffmpeg` 실행 결과(BGRA 바이트, 크기)를 슬롯 + 창 메시지로 돌려준다. 패널을 닫으면 실행 중인 FFmpeg를 끝낸다(기존 `RunJob` 방식).
- **새 `editor/video_render.rs`:** 미리보기 장면을 `to_result()` 변환으로 그리기(회전 메타데이터 보정 포함), 자르기 오버레이는 사진 쪽 그리기 함수를 공유, 타임라인(조각·빗금·눈금·재생 위치·시간 글자·버튼), "빠진 구간"·"역재생"·안내 문구, 도구 모음 8개(배속 글자, 소리 상태 아이콘).
- **`jobs.rs`:** `Job.edits: Option<Edits>`, `enum Edits { Image(EditPlan), Video { plan: VideoPlan }, Frame { t: f64, ops: Vec<EditOp> } }`. 실행:
  - `Video`: `ffprobe` → `media::video_edit_plan` → `ffmpeg::run`(기존 진행률·취소·임시 파일 흐름). suffix `edited`, 출력 확장자는 계획의 출력 형식.
  - `Frame`: `media::video_frame_plan` → `ffmpeg::run`, suffix `frame_suffix(t)`, `.png`.
- **`engines/ffmpeg.rs`:** `supports`에 `Tool::Edit` + 영상 입력(편집 계획이 있을 때만 실제 실행).
- **`app.rs`:** 연결만. `SavedVideo`·`SaveFrame`을 받아 `Job` 제출. 장면 저장 카드는 패널이 열린 동안에도 뜬다.
- **`Cargo.toml`:** `windows` 기능에 `Win32_Media_MediaFoundation`과, 그 API가 요구하는 하위 기능(컴파일러가 알려 주는 것만)을 더한다.

### 3.3 미리보기와 실제 결과의 일치
- 영역·회전·반전: 미리보기와 저장이 같은 `frame` ops를 같은 순서로 쓴다(사진 편집기 3.3과 같다). 짝수 보정으로 1px 차이는 허용.
- 구간: 미리보기는 `skip_from`으로 건너뛰고, 저장은 같은 `kept_ranges()`로 자른다. 재생 엔진의 탐색이 키프레임에 늦게 붙는 경우가 있어 건너뛸 때 수십 ms 어긋날 수 있다(저장 결과는 정확하다).
- 배속·소리 끄기는 미리보기에 반영, 역재생·크기 맞추기는 표시만.

## 4. 오류 처리
- ffprobe 실패(영상 스트림 없음 포함): 패널을 닫고 실패 카드 "`이름`: 영상을 읽을 수 없습니다".
- 재생 엔진·D3D 장치를 못 만듦, 파일 열기 실패, 재생 중 오류: 대체 모드(2.6). 로그에 원인.
- 대체 모드 장면 받기 실패: 미리보기 자리에 "장면을 불러오지 못했습니다", 다음 이동에서 다시 시도.
- 저장·장면 저장 실패: 기존 작업 실패 카드. 임시 경로에 쓰고 옮기는 기존 흐름이라 원본은 그대로.
- 저장 시 역재생 한도 초과: 2.5 문구, 저장하지 않음.
- 그래픽 장치를 잃으면 사진 편집기처럼 표면을 다시 만들고, 플레이어의 D3D 장치·DXGI 관리자(`ResetDevice`)·장면 텍스처도 다시 만든다. 다시 만들 수 없으면 편집을 버리고 닫고 실패 카드.

## 5. 테스트

### 5.1 core 단위 테스트
- `editor`: `History`·`frame_geometry`를 뗀 뒤 기존 테스트 전부 통과.
- `catalog`: 영상 한 개면 `Edit`이 끝에, 영상 둘·영상+사진·오디오·움직이는 GIF에는 없음, 사진 한 장 규칙은 그대로.
- `video_edit`:
  - 나누기(0.1초 규칙, 처음·끝 근처), 빼기·되살리기, 마지막 조각 거절, 되돌리기·다시 하기.
  - `kept_ranges`·`output_duration`(배속 포함), `skip_from`(조각 안·뺀 조각 안·마지막 뒤), `first_kept_start`.
  - `reverse_fits` 경계(1 GiB 바로 아래·위, fps 모름), 되살리기 뒤 저장 거절 판정.
  - `has_edits`: 나누기만은 아님, 각 도구는 편집.
  - 소리 없는 영상에서 `Audio` 무시.
- `video_control`: 키 매핑, 자르기 모드 중 막힘, 취소 두 번 확인, 타임라인 누름·끌기의 `PlayerAction`, 끝에서 재생하면 처음으로, 일시 안내 문구 시간.
- `editor_geometry`: 영상 배치(가로·세로 영상, 배율), 클릭 판정, `time_at`/`x_of` 왕복과 양 끝 고정.
- `media`:
  - `parse_probe`가 새 필드를 읽음(회전 90/-90/180, `avg_frame_rate` 0/0 대체, 소리 없음).
  - `video_edit_plan`: 조각 하나(`-ss/-to`)와 여럿(`trim`·`concat`), `frame` 순서대로의 필터, 역재생 위치, 배속별 `setpts`·`atempo`, 끄기 `-an`·`a=0`, 크기 맞추기 `loudnorm`+`-ar`, WebM의 Opus 사슬, WMV → MP4, 기존 변환 인자 불변.
  - `video_frame_plan`, `scrub_args`.
- `naming`: `frame_suffix`(초 단위 반올림, 분·시 단위).

### 5.2 엔진 (FFmpeg가 있는 환경, `lavfi`로 만든 테스트 영상)
- 조각 두 개를 뺀 결과 길이(±0.1초), 소리 트랙 유지.
- 영역 자르기 + 시계 회전 뒤 크기, 홀수 영역의 짝수 보정.
- 2× 배속 길이, 0.5× 배속 길이.
- 끄기면 소리 트랙 없음, 크기 맞추기면 샘플레이트가 원본(44100)과 같음.
- 역재생: 프레임마다 색이 바뀌는 영상에서 결과 첫 장면 색 = 원본 마지막 장면 색.
- 장면 PNG: 크기(영역·회전 적용), 원본 옆 이름.
- 회전 메타데이터(`-display_rotation 90`)가 있는 영상: `MediaInfo` 화면 기준 크기, 영역 자르기 결과가 화면 기준 좌표.
- WebM·AVI·WMV 입력 각각 한 번씩 저장 성공(출력 형식 확인).

### 5.3 작업 흐름
- 영상 편집 `Job`이 원본 옆에 `clip (edited).mp4`를, 장면 저장 `Job`이 `clip (frame 0m01.00s).png`를 만들고 원본은 그대로.

### 5.4 수동 확인 (`docs/manual-test.md`에 더함)
- 영상 한 개일 때만 ✎편집 타일, FFmpeg 없을 때 설치 안내.
- 재생·소리·멈춤, 프레임 이동, Shift+화살표, Home/End, 타임라인 누르기·끌기.
- 나누기·빼기·되살리기, 재생 중 뺀 구간 건너뛰기, 마지막 조각 거절 문구.
- 배속 미리보기, 소리 끄기 미리보기, 크기 맞추기 안내, 소리 없는 영상에서 흐린 버튼.
- 역재생 한도 문구(긴 영상), 구간을 줄인 뒤 켜짐.
- 장면 저장 카드(패널 열린 채), 결과 PNG.
- 휴대폰 세로 영상의 미리보기 방향·영역 자르기·결과 방향.
- 대체 모드(Windows가 못 여는 코덱, 예: `-c:v ffv1`의 MKV)에서 장면 넘겨보기.
- 결과 영상이 Windows 기본 플레이어에서 재생되는지(MP4·WebM·MKV).
- 사진 편집기와 한 번에 하나 규칙, 여러 모니터·배율, 라이트·다크·투명 효과 끔.

## 6. 문서
- README: 편집기 항목에 영상 편집기, 키 목록.
- `docs/manual-test.md`: 5.4(기존 M122 다음 번호부터).
