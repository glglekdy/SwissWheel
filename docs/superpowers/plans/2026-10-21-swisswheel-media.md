# SwissWheel 영상·오디오 변환 + 옵션 패널 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 영상·오디오 변환과 영상 도구(압축·오디오 추출·음소거)를 FFmpeg로 실제 실행하고, 칸에 커서를 올리면 뜨는 옵션 패널(GIF 프레임·크기, 영상 해상도, 압축 강도, 이미지 축소 비율·품질)과 트레이 툴팁 진행률을 더한다 (spec 2.5, 3.1, 3.1.1, 3.2, 4.4 / 8장 4단계).

**Architecture:** 옵션 모델(`options`), FFmpeg 인자 결정과 ffprobe/진행률 해석(`media`), 패널 배치·히트 테스트(`geometry`), 패널 유지 규칙(`session`)은 OS 무관한 순수 로직으로 `swisswheel-core`에 두고 단위 테스트한다. 앱 크레이트의 `engines::ffmpeg`는 FFmpeg를 찾아 창 없이 Job Object에 묶어 실행하며 진행률·취소·복사 실패 시 재시도를 처리한다. 작업 실행기는 이미지 풀과 미디어 풀(2개)을 따로 두고, 작업은 고른 옵션(`Preset`)을 함께 나른다. 옵션 패널은 휠 창을 패널 자리까지 넓혀 같은 표면에 그리고, 창 영역으로 히트 테스트 범위를 조절한다.

**Tech Stack:** 기존 그대로 (Rust 1.99, `windows` 0.61, `image` 0.25, `jpeg-encoder` 0.6, `webp` 0.3, `oxipng` 9). 새 크레이트 없음. 외부 실행 파일: FFmpeg GPL 빌드 (개발 PC에는 WinGet으로 9.0.1이 PATH에 있음).

**Spec:** `docs/superpowers/specs/2026-10-20-swisswheel-design.md` — 특히 2.3, 2.5, 3.1.1, 3.2, 4.3, 4.4. 인코딩 인자·옵션 값은 spec 표가 기준이며 이 계획에서 다시 적지 않는다.

**Branch:** `feat/media` (이미 생성됨, `feat/shell-and-image` 위). 앞 브랜치가 리뷰 후 `main`에 합쳐지면 이 브랜치를 rebase한다.

## Global Constraints

- 이 계획 밖: **동작 휠(공유·복사·잘라내기·경로 복사·터미널), 자동 실행, 트레이 메뉴 확장, FFmpeg 동봉·패키징.** 동작 항목 드롭은 지금처럼 "다음 버전에서 지원 예정입니다" 알림.
- `swisswheel-core`는 OS 비의존 (의존성 `serde`, `toml`, `thiserror`만).
- FFmpeg 찾기: `<실행 파일 폴더>\ffmpeg\` 에 `ffmpeg.exe`와 `ffprobe.exe`가 둘 다 있으면 그것, 아니면 `PATH`.
- FFmpeg 실행 고정 인자: `-hide_banner -nostdin -loglevel error -y -progress pipe:1 -nostats -i <입력> <계획 인자> <임시 출력>`. 계획 인자는 항상 `-f <먹서>`로 끝난다 (임시 이름이 `.swisswheel-tmp`라서). `CREATE_NO_WINDOW`, stdin null, `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` Job Object.
- FFmpeg 인자는 셸을 거치지 않으므로(`Command::args`) 필터 안의 쉼표는 작은따옴표로 감싼다: `scale='min(480,iw)':-2`, `scale=-2:'trunc(min(720,ih)/2)*2'`.
- 동시 실행: 이미지 `min(코어 수, 4)`, FFmpeg 2. 앱 종료 시 실행 중 FFmpeg는 죽이고 대기 파일은 처리하지 않으며 임시 파일을 지운다.
- 원본은 절대 수정·삭제·덮어쓰기 금지. 결과 확정은 기존 `fsutil::finalize`(대체 없는 이동)만.
- 드롭 응답은 지금처럼 항목·옵션 줄 위 `DROPEFFECT_COPY`, 그 밖 `DROPEFFECT_NONE`.
- UI 문구 한국어. 커밋 메시지 끝:
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` 와 `Claude-Session: https://claude.ai/code/session_016PYsmXwW5jKgU1htHsgH9G`
- 매 Task 끝: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`. FFmpeg가 필요한 테스트는 FFmpeg가 없을 때 건너뛰지 말고 `"FFmpeg(ffmpeg.exe, ffprobe.exe)가 PATH에 필요합니다"`로 panic.

## Review Focus

1. **영상 작업 도중 트레이에서 종료** — 수 초 안에 종료, ffmpeg.exe·`.swisswheel-tmp`·반쯤 쓴 결과가 남지 않아야 한다. → Task 5 `cancel_kills_ffmpeg`, Task 6 `shutdown_cancels_running_media`
2. **GIF 칸(왼쪽 아래)에서 오른쪽 패널로 곧장 감** — 가운데(취소 영역)와 옵션이 있는 WEBM·MOV 칸을 지나가도 패널이 닫히거나 바뀌면 안 된다 (300ms 머물러야만 전환). → Task 7 `panel_survives_the_trip_across_the_wheel`
3. **화면 오른쪽 끝에서 휠을 엶** — 패널이 왼쪽에 떠야 하고, 왼쪽일 때 히트 테스트도 거울상이어야 한다. → Task 7 `panel_side_flips_at_the_right_edge`, `panel_hit_mirrors_on_the_left`
4. **홀수 크기 입력** (애니메이션 GIF 81×61, 해상도 옵션으로 줄인 결과) — libx264는 홀수 크기를 거부한다. → Task 5 `animated_gif_to_mp4`, `resolution_scales_down_only`
5. **오디오가 없는 영상을 MP3로·오디오 추출** — FFmpeg 오류가 아니라 "오디오 트랙이 없습니다". → Task 3 `audio_targets`, Task 5 `tools_on_video`

---

### Task 1: 옵션 모델 (core)

**Files:**
- Create: `crates/swisswheel-core/src/options.rs`
- Modify: `crates/swisswheel-core/src/lib.rs`, `crates/swisswheel-core/src/catalog.rs` (`Tool::HalveImage` → `Tool::Shrink`, 라벨 "축소"; 이름 바꾸기는 앱 크레이트의 사용처와 `wheel/glyph.rs`까지), `crates/swisswheel-core/src/naming.rs`

**Interfaces:**
- Produces:
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)] pub enum Preset { Gif { fps: u8, max_width: Option<u16> }, Resolution(Option<u16>) /* 최대 높이, None = 원본 */, Compress(Strength), Scale(u8) /* 퍼센트 */, Quality(Level) }`
  - `pub enum Strength { Light, Normal, Strong }`, `pub enum Level { High, Normal, Small }` (둘 다 `Debug, Clone, Copy, PartialEq, Eq, Hash`)
  - `pub struct Choice { pub preset: Preset, pub label: &'static str, pub detail: &'static str }`
  - `pub struct OptionSet { pub title: &'static str, pub choices: &'static [Choice], pub default: usize }` + `impl OptionSet { pub fn default_preset(&self) -> Preset }`
  - `pub fn options_for(op: &Operation, inputs: &[InputItem]) -> Option<&'static OptionSet>` — 모든 입력이 (애니메이션 아닌) 영상이면: `Convert(Gif)` → GIF, `Convert(Mp4|Mkv|Webm|Mov|Avi)` → 해상도, `Tool(CompressVideo)` → 압축 강도. 모든 입력이 이미지면: `Tool(Shrink)` → 축소 비율, `Convert(Jpg|Webp)` → 품질. 그 밖(섞인 선택, 오디오, 애니메이션 GIF, 폴더 포함)은 `None`.
  - 세트 내용 (spec 2.5): 제목 "GIF 옵션" `[작게 "10fps · 320px", 보통 "15fps · 480px", 부드럽게 "24fps · 원본 크기"]` 기본 1 / "해상도" `[원본 "크기 그대로", 1080p "Full HD", 720p "HD", 480p "SD"]` 기본 0 / "압축 강도" `[약하게 "화질 우선", 보통 "균형", 강하게 "용량 우선 · 720p"]` 기본 1 / "축소 비율" `[25% "가로·세로 1/4", 50% "가로·세로 1/2", 75% "가로·세로 3/4"]` 기본 1 / "품질" `[높게 "용량 큼", 보통 "균형", 작게 "용량 작음"]` 기본 1.
  - `naming::suffix_for(op: &Operation, preset: Option<Preset>) -> Option<String>` — `Tool(Shrink)`는 `"<p>%"`(없으면 `"50%"`), `Convert(_)`에 `Resolution(Some(h))`면 `"<h>p"`, 그 밖은 기존 `Tool::suffix()`.

- [ ] **Step 1: 실패하는 테스트 작성** (`options.rs`, `naming.rs`의 `mod tests`; `fn video() / image() / audio()`는 `InputItem::File { format: Some(Mp4 | Png | Mp3), animated_gif: false }`)

```rust
#[test]
fn option_sets_follow_the_input_kind() {
    use Format::*;
    let title = |op: Operation, inputs: &[InputItem]| options_for(&op, inputs).map(|s| s.title);
    assert_eq!(title(Operation::Convert(Gif), &[video()]), Some("GIF 옵션"));
    assert_eq!(title(Operation::Convert(Mkv), &[video()]), Some("해상도"));
    assert_eq!(title(Operation::Tool(Tool::CompressVideo), &[video()]), Some("압축 강도"));
    assert_eq!(title(Operation::Tool(Tool::Shrink), &[image()]), Some("축소 비율"));
    assert_eq!(title(Operation::Convert(Jpg), &[image()]), Some("품질"));
    assert_eq!(title(Operation::Convert(Webp), &[image()]), Some("품질"));
    // No options: image → GIF/PNG, audio, mixed kinds, animated GIF → video.
    assert_eq!(title(Operation::Convert(Gif), &[image()]), None);
    assert_eq!(title(Operation::Convert(Png), &[image()]), None);
    assert_eq!(title(Operation::Convert(Mp3), &[video()]), None);
    assert_eq!(title(Operation::Convert(Ogg), &[audio()]), None);
    assert_eq!(title(Operation::Convert(Gif), &[image(), video()]), None);
    let anim = InputItem::File { format: Some(Gif), animated_gif: true };
    assert_eq!(title(Operation::Convert(Mp4), &[anim]), None);
}

#[test]
fn defaults_match_the_spec() {
    let gif = options_for(&Operation::Convert(Format::Gif), &[video()]).unwrap();
    assert_eq!(gif.default_preset(), Preset::Gif { fps: 15, max_width: Some(480) });
    assert_eq!(gif.choices[0].preset, Preset::Gif { fps: 10, max_width: Some(320) });
    assert_eq!(gif.choices[2].preset, Preset::Gif { fps: 24, max_width: None });
    let res = options_for(&Operation::Convert(Format::Mp4), &[InputItem::File { format: Some(Format::Mkv), animated_gif: false }]).unwrap();
    assert_eq!(res.choices.iter().map(|c| c.preset).collect::<Vec<_>>(),
               [None, Some(1080), Some(720), Some(480)].map(Preset::Resolution).to_vec());
    assert_eq!(res.default_preset(), Preset::Resolution(None));
    assert_eq!(options_for(&Operation::Tool(Tool::Shrink), &[image()]).unwrap().default_preset(), Preset::Scale(50));
    assert_eq!(options_for(&Operation::Tool(Tool::CompressVideo), &[video()]).unwrap().default_preset(), Preset::Compress(Strength::Normal));
    assert_eq!(options_for(&Operation::Convert(Format::Jpg), &[image()]).unwrap().default_preset(), Preset::Quality(Level::Normal));
}

// naming.rs
#[test]
fn suffixes_carry_the_chosen_option() {
    let shrink = Operation::Tool(Tool::Shrink);
    assert_eq!(suffix_for(&shrink, Some(Preset::Scale(25))).as_deref(), Some("25%"));
    assert_eq!(suffix_for(&shrink, None).as_deref(), Some("50%"));
    assert_eq!(suffix_for(&Operation::Convert(Format::Mp4), Some(Preset::Resolution(Some(720)))).as_deref(), Some("720p"));
    assert_eq!(suffix_for(&Operation::Convert(Format::Mp4), Some(Preset::Resolution(None))), None);
    assert_eq!(suffix_for(&Operation::Tool(Tool::CompressVideo), Some(Preset::Compress(Strength::Strong))).as_deref(), Some("compressed"));
    assert_eq!(suffix_for(&Operation::Convert(Format::Gif), Some(Preset::Gif { fps: 10, max_width: Some(320) })), None);
}
```

기존 catalog 테스트의 `HalveImage`·`"50% 축소"`는 `Shrink`·`"축소"`로 바꾼다.

- [ ] **Step 2: 실패 확인** — Run: `cargo test -p swisswheel-core` / Expected: 컴파일 실패 (`options` 없음)
- [ ] **Step 3: 구현** — 세트는 `static` 상수. 입력 종류 판정은 모든 항목이 `InputItem::File { format: Some(f), animated_gif }`이고 `f.kind()`가 같으며 애니메이션 GIF가 없을 때만.
- [ ] **Step 4: 통과 확인** — Run: `cargo test --workspace` / Expected: 모두 통과
- [ ] **Step 5: 커밋** — `git commit -am "feat(core): option sets for wheel items"`

---

### Task 2: ffprobe·진행률 해석 (core)

**Files:**
- Create: `crates/swisswheel-core/src/media.rs` (이 Task는 해석 부분)
- Modify: `crates/swisswheel-core/src/lib.rs`

**Interfaces:**
- Produces:
  - `#[derive(Debug, Clone, Default, PartialEq)] pub struct MediaInfo { pub duration_secs: Option<f64>, pub video: Option<String>, pub audio: Option<String> }` — 종류별 **첫** 스트림 코덱.
  - `pub fn parse_probe(text: &str) -> MediaInfo` — `ffprobe -v error -show_entries format=duration:stream=codec_type,codec_name -of default=noprint_wrappers=1` 출력 (`codec_name=` 다음 줄 `codec_type=`, 끝에 `duration=`; `N/A`·CRLF 가능).
  - `#[derive(Debug, PartialEq)] pub enum Progress { OutTime(f64), End }`, `pub fn parse_progress(line: &str) -> Option<Progress>` — `out_time_us=`, `out_time_ms=`(FFmpeg에서 이것도 마이크로초), `progress=end`.
  - `pub fn fraction(out_time_secs: f64, duration_secs: Option<f64>) -> Option<f32>` — 0..=1로 자름, 길이 없음·0 이하는 `None`.
  - `pub fn muxer(format: Format) -> Option<&'static str>` — spec 3.1.1 먹서 목록, 입력 전용(WMV·WMA·AIFF)과 이미지는 `None`.

- [ ] **Step 1: 실패하는 테스트 작성**

```rust
#[test]
fn parses_probe_output() {
    let info = parse_probe("codec_name=h264\ncodec_type=video\ncodec_name=aac\ncodec_type=audio\nduration=2.000000\n");
    assert_eq!(info, MediaInfo { duration_secs: Some(2.0), video: Some("h264".into()), audio: Some("aac".into()) });
}

#[test]
fn probe_keeps_first_of_each_kind_and_tolerates_na_and_crlf() {
    let info = parse_probe("codec_name=mjpeg\ncodec_type=video\ncodec_name=flac\ncodec_type=audio\ncodec_name=png\ncodec_type=video\nduration=N/A\n");
    assert_eq!(info, MediaInfo { duration_secs: None, video: Some("mjpeg".into()), audio: Some("flac".into()) });
    let gif = parse_probe("codec_name=gif\r\ncodec_type=video\r\nduration=1.000000\r\n");
    assert_eq!(gif, MediaInfo { duration_secs: Some(1.0), video: Some("gif".into()), audio: None });
}

#[test]
fn parses_progress_lines() {
    assert_eq!(parse_progress("out_time_us=1500000"), Some(Progress::OutTime(1.5)));
    assert_eq!(parse_progress("out_time_ms=2000000"), Some(Progress::OutTime(2.0)));
    assert_eq!(parse_progress("out_time_us=N/A"), None);
    assert_eq!(parse_progress("progress=end\r"), Some(Progress::End));
    assert_eq!(parse_progress("frame=20"), None);
}

#[test]
fn fraction_is_clamped() {
    assert_eq!(fraction(1.0, Some(4.0)), Some(0.25));
    assert_eq!(fraction(5.0, Some(4.0)), Some(1.0));
    assert_eq!(fraction(1.0, None), None);
    assert_eq!(fraction(1.0, Some(0.0)), None);
}

#[test]
fn muxers_are_explicit() {
    use Format::*;
    for (f, m) in [(Mp4, "mp4"), (Mkv, "matroska"), (Webm, "webm"), (Mov, "mov"), (Avi, "avi"), (Gif, "gif"),
                   (Mp3, "mp3"), (M4a, "ipod"), (Wav, "wav"), (Flac, "flac"), (Ogg, "ogg"), (Opus, "opus")] {
        assert_eq!(muxer(f), Some(m), "{f:?}");
    }
    for f in [Wmv, Wma, Aiff, Png] {
        assert_eq!(muxer(f), None, "{f:?}");
    }
}
```

- [ ] **Step 2: 실패 확인** — Run: `cargo test -p swisswheel-core media` / Expected: 컴파일 실패
- [ ] **Step 3: 구현** — `parse_probe`는 줄마다 `trim_end()`, 마지막 `codec_name`을 기억했다가 `codec_type`이 `video`/`audio`이고 그 칸이 비었을 때만 채운다.
- [ ] **Step 4: 통과 확인** — Run: `cargo test -p swisswheel-core media` / Expected: 5 passed
- [ ] **Step 5: 커밋** — `git commit -am "feat(core): parse ffprobe and ffmpeg progress output"`

---

### Task 3: FFmpeg 작업 계획 (core)

**Files:**
- Modify: `crates/swisswheel-core/src/media.rs`, `crates/swisswheel-core/src/catalog.rs` (`VIDEO_TARGETS`·`AUDIO_TARGETS`·`ANIMATED_GIF_TARGETS`를 `pub(crate)`로)

**Interfaces:**
- Consumes: Task 1 `Preset`·`Strength`, Task 2 `MediaInfo`·`muxer`.
- Produces:
  - `#[derive(Debug, Clone, PartialEq)] pub struct MediaPlan { pub output: Format, pub attempts: Vec<Vec<String>>, pub duration_secs: Option<f64> }` — `attempts`는 순서대로 시도하는 인자 목록(각각 `-i <입력>` 뒤·출력 경로 앞, `"-f", <먹서>`로 끝남).
  - `#[derive(Debug, PartialEq, thiserror::Error)] pub enum PlanError { #[error("no audio stream")] NoAudio, #[error("no video stream")] NoVideo, #[error("unsupported")] Unsupported }`
  - `pub fn supports(op: &Operation, input: Format) -> bool` — 영상 입력 → `VIDEO_TARGETS` 변환·영상 도구 3개, 오디오 입력 → `AUDIO_TARGETS` 변환, GIF 입력 → `ANIMATED_GIF_TARGETS` 변환.
  - `pub fn plan(op: &Operation, preset: Option<Preset>, input: Format, info: &MediaInfo, quality: &Quality) -> Result<MediaPlan, PlanError>` — spec 3.1.1·2.5 그대로. `preset == None`은 그 세트의 기본값과 같다. 해상도 `Some(h)`면 복사 시도 없이 `-vf scale=-2:'trunc(min(<h>,ih)/2)*2'`(짝수 보정 겸함)로 인코딩. 압축 강도: 약하게 CRF 24·`160k`, 보통 28·`128k`, 강하게 32·`96k` + 위 720 축소 필터. GIF: `fps=<fps>,scale='min(<w>,iw)':-2:flags=lanczos,split[a][b];[a]palettegen[p];[b][p]paletteuse` (원본 크기면 `fps=<fps>,split…`).

- [ ] **Step 1: 실패하는 테스트 작성** (헬퍼 `info(v, a)`(길이 `Some(2.0)`), `has(args, &[..])`(연속 부분열), `ends_with_f(args, m)`)

```rust
#[test]
fn video_convert_reencodes_with_spec_quality() {
    let q = Quality::default();
    let p = plan(&Operation::Convert(Format::Mp4), None, Format::Avi, &info(Some("mpeg4"), Some("mp3")), &q).unwrap();
    assert_eq!((p.output, p.attempts.len()), (Format::Mp4, 1));
    let a = &p.attempts[0];
    assert!(has(a, &["-c:v", "libx264"]) && has(a, &["-crf", "23"]) && has(a, &["-c:a", "aac", "-b:a", "192k"]));
    assert!(has(a, &["-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2"]) && has(a, &["-movflags", "+faststart"]) && ends_with_f(a, "mp4"));
    let w = &plan(&Operation::Convert(Format::Webm), None, Format::Avi, &info(Some("mpeg4"), Some("mp3")), &q).unwrap().attempts[0];
    assert!(has(w, &["-c:v", "libvpx-vp9", "-crf", "32"]) && has(w, &["-c:a", "libopus"]) && ends_with_f(w, "webm"));
    let avi = &plan(&Operation::Convert(Format::Avi), None, Format::Mp4, &info(Some("h264"), Some("aac")), &q).unwrap().attempts[0];
    assert!(has(avi, &["-c:v", "mpeg4"]) && has(avi, &["-b:a", "192k"]) && ends_with_f(avi, "avi"));
}

#[test]
fn compatible_streams_are_remuxed_first() {
    let q = Quality::default();
    let mkv = plan(&Operation::Convert(Format::Mkv), None, Format::Mp4, &info(Some("h264"), Some("aac")), &q).unwrap();
    assert_eq!(mkv.attempts.len(), 2);
    assert!(has(&mkv.attempts[0], &["-c", "copy"]) && ends_with_f(&mkv.attempts[0], "matroska"));
    assert!(has(&mkv.attempts[1], &["-c:v", "libx264"]));
    let hevc = plan(&Operation::Convert(Format::Mp4), None, Format::Mkv, &info(Some("hevc"), Some("aac")), &q).unwrap();
    assert!(has(&hevc.attempts[0], &["-tag:v", "hvc1"]));
    assert!(has(&plan(&Operation::Convert(Format::Webm), None, Format::Mkv, &info(Some("vp9"), Some("opus")), &q).unwrap().attempts[0], &["-c", "copy"]));
    let one = |op, input, v, a| plan(&Operation::Convert(op), None, input, &info(Some(v), Some(a)), &q).unwrap().attempts.len();
    assert_eq!(one(Format::Mp4, Format::Mkv, "h264", "opus"), 1); // opus isn't an MP4 copy by our rule
    assert_eq!(one(Format::Avi, Format::Mkv, "mpeg4", "mp3"), 1); // AVI never copies
}

#[test]
fn resolution_option_reencodes_and_scales_down() {
    let q = Quality::default();
    let p = plan(&Operation::Convert(Format::Mkv), Some(Preset::Resolution(Some(720))), Format::Mp4, &info(Some("h264"), Some("aac")), &q).unwrap();
    assert_eq!(p.attempts.len(), 1); // no copy when resizing
    assert!(has(&p.attempts[0], &["-vf", "scale=-2:'trunc(min(720,ih)/2)*2'"]));
    let w = plan(&Operation::Convert(Format::Webm), Some(Preset::Resolution(Some(480))), Format::Mp4, &info(Some("h264"), Some("aac")), &q).unwrap();
    assert!(has(&w.attempts[0], &["-vf", "scale=-2:'trunc(min(480,ih)/2)*2'"]));
    assert_eq!(plan(&Operation::Convert(Format::Mkv), Some(Preset::Resolution(None)), Format::Mp4, &info(Some("h264"), Some("aac")), &q).unwrap().attempts.len(), 2);
}

#[test]
fn animated_gif_to_video_encodes_with_even_size() {
    let p = plan(&Operation::Convert(Format::Mp4), None, Format::Gif, &info(Some("gif"), None), &Quality::default()).unwrap();
    assert_eq!(p.attempts.len(), 1);
    assert!(has(&p.attempts[0], &["-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2"]));
}

#[test]
fn audio_targets() {
    let q = Quality::default();
    let mp3 = &plan(&Operation::Convert(Format::Mp3), None, Format::Flac, &info(None, Some("flac")), &q).unwrap().attempts[0];
    assert!(has(mp3, &["-map", "0:a:0"]) && has(mp3, &["-c:a", "libmp3lame", "-b:a", "192k"]) && ends_with_f(mp3, "mp3"));
    let loud = Quality { mp3_kbps: 320, ..Quality::default() };
    assert!(has(&plan(&Operation::Convert(Format::Mp3), None, Format::Wav, &info(None, Some("pcm_s16le")), &loud).unwrap().attempts[0], &["-b:a", "320k"]));
    assert!(ends_with_f(&plan(&Operation::Convert(Format::M4a), None, Format::Mp3, &info(None, Some("mp3")), &q).unwrap().attempts[0], "ipod"));
    assert!(has(&plan(&Operation::Convert(Format::Wav), None, Format::Flac, &info(None, Some("flac")), &q).unwrap().attempts[0], &["-c:a", "pcm_s16le"]));
    assert!(plan(&Operation::Convert(Format::Mp3), None, Format::Mp4, &info(Some("h264"), Some("aac")), &q).is_ok());
    assert_eq!(plan(&Operation::Convert(Format::Mp3), None, Format::Mp4, &info(Some("h264"), None), &q), Err(PlanError::NoAudio));
}

#[test]
fn gif_options() {
    let q = Quality::default();
    let vf = |preset| {
        let p = plan(&Operation::Convert(Format::Gif), preset, Format::Mp4, &info(Some("h264"), Some("aac")), &q).unwrap();
        assert!(has(&p.attempts[0], &["-loop", "0"]) && ends_with_f(&p.attempts[0], "gif"));
        p.attempts[0].iter().find(|s| s.contains("paletteuse")).unwrap().clone()
    };
    assert!(vf(None).starts_with("fps=15,scale='min(480,iw)':-2:flags=lanczos,"));
    assert!(vf(Some(Preset::Gif { fps: 10, max_width: Some(320) })).starts_with("fps=10,scale='min(320,iw)':-2"));
    assert!(vf(Some(Preset::Gif { fps: 24, max_width: None })).starts_with("fps=24,split"));
}

#[test]
fn video_tools() {
    let q = Quality::default();
    let c = plan(&Operation::Tool(Tool::CompressVideo), None, Format::Webm, &info(Some("vp9"), Some("opus")), &q).unwrap();
    assert_eq!(c.output, Format::Mp4);
    assert!(has(&c.attempts[0], &["-crf", "28"]) && has(&c.attempts[0], &["-b:a", "128k"]));
    let strong = plan(&Operation::Tool(Tool::CompressVideo), Some(Preset::Compress(Strength::Strong)), Format::Mp4, &info(Some("h264"), Some("aac")), &q).unwrap();
    assert!(has(&strong.attempts[0], &["-crf", "32"]) && has(&strong.attempts[0], &["-b:a", "96k"]));
    assert!(has(&strong.attempts[0], &["-vf", "scale=-2:'trunc(min(720,ih)/2)*2'"]));
    let light = plan(&Operation::Tool(Tool::CompressVideo), Some(Preset::Compress(Strength::Light)), Format::Mkv, &info(Some("h264"), Some("aac")), &q).unwrap();
    assert_eq!(light.output, Format::Mkv);
    assert!(has(&light.attempts[0], &["-crf", "24"]) && has(&light.attempts[0], &["-b:a", "160k"]));
    let ex = plan(&Operation::Tool(Tool::ExtractAudio), None, Format::Mp4, &info(Some("h264"), Some("aac")), &q).unwrap();
    assert_eq!((ex.output, ex.attempts.len()), (Format::M4a, 2));
    assert!(has(&ex.attempts[0], &["-c:a", "copy"]));
    assert_eq!(plan(&Operation::Tool(Tool::ExtractAudio), None, Format::Mkv, &info(Some("h264"), Some("ac3")), &q).unwrap().output, Format::Mp3);
    assert_eq!(plan(&Operation::Tool(Tool::ExtractAudio), None, Format::Mp4, &info(Some("h264"), None), &q), Err(PlanError::NoAudio));
    let mute = plan(&Operation::Tool(Tool::Mute), None, Format::Mp4, &info(Some("h264"), Some("aac")), &q).unwrap();
    assert_eq!(mute.output, Format::Mp4);
    assert!(has(&mute.attempts[0], &["-an"]) && has(&mute.attempts[0], &["-c:v", "copy"]));
    let wmv = plan(&Operation::Tool(Tool::Mute), None, Format::Wmv, &info(Some("wmv2"), Some("wmav2")), &q).unwrap();
    assert_eq!(wmv.output, Format::Mp4);
    assert!(has(&wmv.attempts[0], &["-c:v", "libx264"]));
}

#[test]
fn supports_only_media_work() {
    use Format::*;
    assert!(supports(&Operation::Convert(Mp3), Mkv));
    assert!(supports(&Operation::Convert(Webm), Gif));
    assert!(supports(&Operation::Tool(Tool::ExtractAudio), Mp4));
    assert!(!supports(&Operation::Convert(Jpg), Png));
    assert!(!supports(&Operation::Convert(Png), Gif));
    assert!(!supports(&Operation::Convert(Mp4), Mp3));
    assert!(!supports(&Operation::Tool(Tool::Shrink), Mp4));
    assert!(!supports(&Operation::Tool(Tool::Mute), Mp3));
    assert_eq!(plan(&Operation::Convert(Mp4), None, Mp4, &info(Some("h264"), None), &Quality::default()), Err(PlanError::Unsupported));
}
```

- [ ] **Step 2: 실패 확인** — Run: `cargo test -p swisswheel-core media` / Expected: 컴파일 실패 (`plan` 없음)
- [ ] **Step 3: 구현** — 인자 조각은 `&'static str` 상수로, 수치가 들어가는 조각만 `format!`.
- [ ] **Step 4: 통과 확인** — Run: `cargo test -p swisswheel-core` / Expected: 모두 통과 (media 13개)
- [ ] **Step 5: 커밋** — `git commit -am "feat(core): plan ffmpeg arguments per operation and option"`

---

### Task 4: 이미지 엔진 옵션

**Files:**
- Modify: `crates/swisswheel/src/engines/image.rs`

**Interfaces:**
- Consumes: Task 1 `Preset::{Scale, Quality}`, `Level`.
- Produces: `pub fn run(op: &Operation, preset: Option<Preset>, input: &Path, output: &Path, quality: &Quality) -> Result<(), EngineError>` — `Tool(Shrink)`는 `Scale(p)`(기본 50)로 가로·세로 `p%`(최소 1px), `Convert(Jpg|Webp)`는 `Quality(High)` → JPG 95·WEBP 92, `Normal`/`None` → `quality.jpg`·`quality.webp`, `Small` → 75·70. 그 밖의 `preset`은 무시.

- [ ] **Step 1: 실패하는 테스트 작성** (기존 테스트는 `run(op, None, …)`으로)

```rust
#[test]
fn shrink_uses_the_chosen_scale() {
    /* 400×200 PNG: Scale(25) → 100×50, Scale(75) → 300×150, None → 200×100 */
}

#[test]
fn quality_levels_order_file_sizes() {
    /* jpg_output_is_optimized의 노이즈 사진으로 Convert(Jpg): High > Normal > Small (바이트 수),
       Convert(Webp)도 같은 순서 */
}
```

- [ ] **Step 2: 실패 확인** — Run: `cargo test -p swisswheel engines::image` / Expected: 컴파일 실패 (`run` 인자 수)
- [ ] **Step 3: 구현**
- [ ] **Step 4: 통과 확인** — Run: `cargo test -p swisswheel engines::image` / Expected: 10 passed
- [ ] **Step 5: 커밋** — `git commit -am "feat(image): shrink scale and quality options"`

---

### Task 5: FFmpeg 엔진 (app)

**Files:**
- Create: `crates/swisswheel/src/engines/ffmpeg.rs`
- Modify: `crates/swisswheel/src/engines/mod.rs`, `crates/swisswheel/Cargo.toml` (`windows` 기능 `Win32_System_JobObjects` 등 컴파일러가 요구하는 것)

**Interfaces:**
- Consumes: Task 2·3 `media::*`.
- Produces:
  - `EngineError` 추가 변형과 알림 문구: `MediaRead(String)` "영상·오디오 파일을 읽을 수 없습니다", `NoAudio` "오디오 트랙이 없습니다", `NoVideo` "영상 트랙이 없습니다", `MissingFfmpeg` "FFmpeg를 찾을 수 없습니다", `Cancelled` "취소되었습니다". `Failed(String)`에는 FFmpeg stderr 끝부분(최대 2KB)을 담는다(문구는 기존 "변환에 실패했습니다").
  - `#[derive(Debug, Clone, PartialEq)] pub struct Tools { pub ffmpeg: PathBuf, pub ffprobe: PathBuf }`
  - `pub fn locate() -> Option<Tools>` = `locate_in(<exe 폴더>, std::env::var_os("PATH").as_deref())`; `pub fn locate_in(app_dir: &Path, path_var: Option<&OsStr>) -> Option<Tools>`.
  - `pub fn prepare(tools: &Tools, op: &Operation, preset: Option<Preset>, input: &Path, quality: &Quality) -> Result<MediaPlan, EngineError>` — ffprobe + `media::plan`. ffprobe 실패는 `MediaRead`, `PlanError`는 같은 이름의 변형.
  - `pub fn run(tools: &Tools, plan: &MediaPlan, input: &Path, output: &Path, progress: &dyn Fn(f32), cancel: &AtomicBool) -> Result<(), EngineError>` — `attempts`를 차례로, 실패한 시도의 출력은 지우고 다음. `progress=end`에서 1.0. `cancel`이 서면 100ms 안에 kill, 출력 삭제, `Cancelled`.
  - 테스트 헬퍼 `#[cfg(test)] pub(crate) mod tests`: `tools() -> Tools`(없으면 Global Constraints 문구로 panic), `make(dir, name, args: &[&str]) -> PathBuf`(`ffmpeg -y <args> <dir/name>`), `probe_out(path) -> MediaInfo`, `dims(path) -> (u32, u32)`(`-select_streams v:0 -show_entries stream=width,height`). 기본 클립: `["-f","lavfi","-i","testsrc=duration=2:size=160x120:rate=10","-f","lavfi","-i","sine=duration=2","-c:v","libx264","-c:a","aac","-shortest"]`.

구현 메모: stdout은 별도 스레드가 줄 단위로 읽어 `parse_progress`→`fraction` 값을 `mpsc`로 보낸다. 호출 스레드는 `recv_timeout(100ms)`로 받으며 `cancel`과 `try_wait()`를 본다 (`progress`가 `Send`일 필요 없음). stderr도 별도 스레드에서 끝 2KB만 보관. Job Object는 프로세스 전역에 한 번 만들어 핸들을 유지 (앱이 죽으면 닫히며 ffmpeg도 종료). 생성과 할당 사이의 짧은 틈은 감수한다.

- [ ] **Step 1: 실패하는 테스트 작성**

```rust
#[test]
fn locates_bundled_before_path() {
    let (app, path) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let touch = |dir: &Path| for n in ["ffmpeg.exe", "ffprobe.exe"] { std::fs::write(dir.join(n), b"").unwrap(); };
    touch(path.path());
    let path_var = std::env::join_paths([path.path()]).unwrap();
    assert_eq!(locate_in(app.path(), Some(&path_var)).unwrap().ffmpeg, path.path().join("ffmpeg.exe"));
    let bundled = app.path().join("ffmpeg");
    std::fs::create_dir(&bundled).unwrap();
    std::fs::write(bundled.join("ffmpeg.exe"), b"").unwrap(); // no ffprobe yet → still PATH
    assert_eq!(locate_in(app.path(), Some(&path_var)).unwrap().ffmpeg, path.path().join("ffmpeg.exe"));
    touch(&bundled);
    assert_eq!(locate_in(app.path(), Some(&path_var)).unwrap(),
               Tools { ffmpeg: bundled.join("ffmpeg.exe"), ffprobe: bundled.join("ffprobe.exe") });
    assert_eq!(locate_in(tempfile::tempdir().unwrap().path(), None), None);
}

#[test]
fn converts_video() { /* clip.mp4 → Webm (vp9, opus), Mp3 (None, mp3), Gif (gif, None), Mkv (h264, aac: 복사됨) */ }

#[test]
fn audio_conversions() { /* tone.flac(sine 2초) → Mp3, Opus, Wav: audio mp3 / opus / pcm_s16le, 길이 2.0 ± 0.2 */ }

#[test]
fn tools_on_video() {
    /* clip.mp4: ExtractAudio → plan.output == M4a, audio aac; Mute → video Some·audio None;
       CompressVideo(Strong) → video h264; silent.mp4(오디오 없음) prepare(ExtractAudio) → Err(NoAudio) */
}

#[test]
fn animated_gif_to_mp4() { /* testsrc size=81x61 → anim.gif → Convert(Mp4) → video h264, dims 둘 다 짝수 */ }

#[test]
fn resolution_scales_down_only() {
    /* 1초 1280×720 클립 → Convert(Mkv) + Resolution(Some(480)) → dims (854, 480);
       기본 160×120 클립 + Resolution(Some(480)) → (160, 120) */
}

#[test]
fn gif_option_sets_fps() { /* clip.mp4 → Gif + Gif{fps:10, max_width:Some(80)} → dims.0 == 80, 프레임 수 ≈ 20 (ffprobe -count_frames) */ }

#[test]
fn progress_reaches_one() { /* clip.mp4 → Webm: 값 1개 이상, 감소하지 않음, 마지막 == 1.0 */ }

#[test]
fn later_attempt_runs_when_first_fails() {
    let plan = MediaPlan {
        output: Format::Mp4,
        attempts: vec![
            ["-c", "copy", "-bsf:v", "no_such_filter", "-f", "mp4"].map(String::from).to_vec(),
            ["-c:v", "libx264", "-c:a", "aac", "-f", "mp4"].map(String::from).to_vec(),
        ],
        duration_secs: Some(2.0),
    };
    /* run → Ok, 출력 video h264 */
}

#[test]
fn cancel_kills_ffmpeg() { /* 20초 640×480 → Webm, 300ms 뒤 다른 스레드가 cancel: Err(Cancelled), 5초 미만, 출력 없음 */ }

#[test]
fn unicode_paths() { /* "가족 영상/생일 파티.mp4" → Mp3, 출력 "…/생일 파티.mp3.swisswheel-tmp" → audio mp3 */ }

#[test]
fn unreadable_file_is_media_read_error() { /* "broken.mp4" = b"not a video" → prepare → Err(MediaRead(_)) */ }
```

`engines/mod.rs`:
```rust
#[test]
fn media_user_messages() {
    assert_eq!(EngineError::NoAudio.user_message("a.mp4"), "a.mp4: 오디오 트랙이 없습니다");
    assert_eq!(EngineError::MissingFfmpeg.user_message("a.mp4"), "a.mp4: FFmpeg를 찾을 수 없습니다");
    assert_eq!(EngineError::MediaRead("x".into()).user_message("a.mp4"), "a.mp4: 영상·오디오 파일을 읽을 수 없습니다");
    assert_eq!(EngineError::Cancelled.user_message("a.mp4"), "a.mp4: 취소되었습니다");
}
```

- [ ] **Step 2: 실패 확인** — Run: `cargo test -p swisswheel engines` / Expected: 컴파일 실패
- [ ] **Step 3: 구현** — Interfaces·구현 메모대로.
- [ ] **Step 4: 통과 확인** — Run: `cargo test -p swisswheel engines` / Expected: 이미지 10 + FFmpeg 12 + 메시지 1 passed. 테스트 뒤 작업 관리자에 `ffmpeg.exe`가 남지 않았는지 확인.
- [ ] **Step 5: 커밋** — `git commit -am "feat(app): ffmpeg engine with progress, cancel, and copy fallback"`

---

### Task 6: 작업 실행기 — 옵션, 미디어 풀, 진행률, 종료 시 취소

**Files:**
- Modify: `crates/swisswheel/src/jobs.rs`

**Interfaces:**
- Consumes: Task 1 `naming::suffix_for`, Task 4 `image::run`, Task 5 `ffmpeg::{locate, prepare, run, Tools}`, `media::supports`.
- Produces:
  - `pub struct Job { pub op: Operation, pub preset: Option<Preset>, pub inputs: Vec<PathBuf> }` (`JobReport`는 그대로)
  - `Runner::new(image_workers: usize, media_workers: usize, quality: Quality, on_done: OnDone) -> Runner` — 생성 시 `ffmpeg::locate()` 한 번 (없으면 미디어 파일마다 `MissingFfmpeg`).
  - `Runner::progress(&self) -> Option<f32>` — 보고 전인 작업들의 파일 전체 평균(끝난 파일 1.0, 실행 중은 마지막 값, 대기 0.0); 작업이 없으면 `None`.
  - `Runner::shutdown(self)` — 취소 플래그: 실행 중 FFmpeg kill, 대기 파일은 처리 없이 "취소되었습니다"로 보고, 워커 join.
  - 파일별 라우팅: `image::supports` → 이미지 풀, `media::supports` → 미디어 풀, 둘 다 아니면 즉시 `Unsupported` 문구. 미디어는 `prepare`가 준 `plan.output` 확장자로 이름 예약. 이름 접미사는 `suffix_for(&op, preset)`.
  - 실패 시 `log_error!("{name}: {e}")`로 상세(FFmpeg stderr 포함)를 남긴다.

- [ ] **Step 1: 실패하는 테스트 작성** (`run_jobs`는 `Runner::new(2, 2, …)`, `ffmpeg::tests::make` 사용)

```rust
#[test]
fn media_job_converts_and_reports() { /* clip.mp4 → Convert(Mp3): outputs == [dir/"clip.mp3"], failures 없음 */ }

#[test]
fn media_output_extension_comes_from_probe() { /* clip.mp4(aac) → Tool(ExtractAudio): outputs == [dir/"clip.m4a"] */ }

#[test]
fn option_suffixes_reach_file_names() {
    /* a.png → Tool(Shrink) + Scale(25): [dir/"a (25%).png"];
       clip.mp4 → Convert(Mkv) + Resolution(Some(480)): [dir/"clip (480p).mkv"] */
}

#[test]
fn progress_while_running_and_none_when_idle() {
    /* 10초 320×240 → Webm. 보고 전에 progress()가 Some(p), 0.0 <= p <= 1.0 인 순간을 한 번 이상 관찰;
       보고 후 progress() == None; 그다음 shutdown */
}

#[test]
fn shutdown_cancels_running_media() {
    /* 30초 640×480 → Webm 제출, 500ms 뒤 shutdown(): 5초 미만, "clip.webm"도 ".swisswheel-tmp"로 끝나는 파일도 없음 */
}
```

기존 `unsupported_operations_report_per_file`은 대상을 `Convert(Format::Png)`, 기대값을 `"clip.mp4: 지원하지 않는 작업입니다"`로.

- [ ] **Step 2: 실패 확인** — Run: `cargo test -p swisswheel jobs` / Expected: 컴파일 실패
- [ ] **Step 3: 구현** — 기존 `worker` 루프를 채널 두 개로 두 벌 돌리고, 공유 상태에 `cancel: AtomicBool`과 보고 전 작업 목록을 둔다.
- [ ] **Step 4: 통과 확인** — Run: `cargo test --workspace` / Expected: 모두 통과
- [ ] **Step 5: 커밋** — `git commit -am "feat(app): media pool, options, progress, cancel on shutdown"`

---

### Task 7: 패널 배치·히트 테스트·유지 규칙 (core)

**Files:**
- Modify: `crates/swisswheel-core/src/geometry.rs`, `crates/swisswheel-core/src/session.rs`

**Interfaces:**
- Consumes: Task 1 `options_for`, `OptionSet`, `Preset`.
- Produces (geometry, DIP 상수 × `scale`, 좌표는 휠 중심 기준 물리 픽셀, y 아래쪽):
  - `pub const PANEL_WIDTH: f32 = 168.0; PANEL_HEADER: f32 = 30.0; PANEL_ROW: f32 = 40.0; PANEL_PAD: f32 = 6.0; PANEL_OVERLAP: f32 = 12.0;`
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum Side { Left, Right }`
  - `#[derive(Debug, Clone, Copy, PartialEq)] pub struct PanelRect { pub left: f32, pub top: f32, pub right: f32, pub bottom: f32 }`
  - `pub fn panel_rect(side: Side, rows: usize, scale: f32) -> PanelRect` — 오른쪽: `left = (OUTER_RADIUS - PANEL_OVERLAP)·s`, `right = left + PANEL_WIDTH·s`; 높이 `(HEADER + rows·ROW + PAD)·s`, 세로 가운데. 왼쪽은 좌우 거울상.
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum PanelHit { Header, Row(usize) }`, `pub fn panel_hit(side: Side, rows: usize, dx: f32, dy: f32, scale: f32) -> Option<PanelHit>` — 사각형 밖 `None`, 맨 위 `HEADER` 높이와 아래 `PAD`는 `Header`, 그 사이는 `Row(i)`.
  - `pub fn panel_side(center_x: i32, work_area: Rect, scale: f32) -> Side` — `center_x + ceil((OUTER_RADIUS - PANEL_OVERLAP + PANEL_WIDTH)·s) <= work_area.right`면 `Right`, 아니면 `Left`.
  - `Hit`에 `Choice(usize)`(패널 줄), `Panel`(패널 제목·여백) 추가.
- Produces (session):
  - `pub const PANEL_DWELL_MS: u64 = 300;`
  - `WheelSession::set_panel_side(&mut self, side: Side)` (기본 `Right`)
  - `hover(&mut self, dx: f32, dy: f32, scale: f32, now_ms: u64) -> bool`: 패널이 열려 있고 `panel_hit`이 `Some`이면 `Choice(i)`/`Panel`. 아니면 휠 `hit_test`. `Item(i)`이고 그 칸에 옵션이 있으며 지금 패널의 칸이 아니면 "전환 대기"를 `(i, now_ms)`로 시작(이미 같은 칸 대기 중이면 시각 유지); 그 밖의 위치로 가면 대기만 취소. **패널 자체는 지나가는 것으로 닫히거나 바뀌지 않는다.** 대기가 `PANEL_DWELL_MS`를 채웠으면 그 자리에서 전환. 반환값은 강조 또는 패널이 바뀌었으면 `true`.
  - `tick(&mut self, now_ms: u64) -> bool` — 커서가 멈춰 DragOver가 오지 않아도 대기를 마무리하려고 앱이 타이머로 부른다. 패널이 바뀌었으면 `true`. `switch_pending(&self) -> bool`.
  - `#[derive(Debug, Clone, Copy, PartialEq)] pub struct Selection { pub op: Operation, pub preset: Option<Preset> }`; `drop_at(&self, dx, dy, scale) -> Option<Selection>` — 칸이면 `(op, 그 세트의 default_preset 또는 None)`, 패널 줄이면 `(패널 칸의 op, Some(그 줄의 preset))`, `Panel`·`Cancel`·`Outside`는 `None`.
  - `pub fn panel(&self) -> Option<PanelState>` where `pub struct PanelState { pub item: usize, pub set: &'static OptionSet, pub hovered: Option<usize> }`.
  - `cycle()`과 `set_inputs()`는 패널을 닫는다.

- [ ] **Step 1: 실패하는 테스트 작성** (session: 입력 `video()` = MP4 → 항목 `[Mkv, Webm, Mov, Avi, Gif, Mp3]`, GIF는 240°(왼쪽 아래); 헬퍼 `at(i, n) -> (f32, f32)` = 반지름 100, 각도 `i·360/n`의 `(100·sin, -100·cos)`; `row_point(side, rows, i)` = `panel_rect`로 그 줄 가운데; `open_panel(&mut s, i)` = `at(i, 6)`에 `hover(…, 0)` 후 `tick(PANEL_DWELL_MS)`)

```rust
// geometry.rs
#[test]
fn panel_rect_and_hit_on_the_right() {
    let r = panel_rect(Side::Right, 3, 1.0);
    assert_eq!((r.left, r.right), (148.0, 316.0));
    assert_eq!((r.top, r.bottom), (-78.0, 78.0)); // 30 + 3*40 + 6 = 156
    assert_eq!(panel_hit(Side::Right, 3, 200.0, -70.0, 1.0), Some(PanelHit::Header));
    assert_eq!(panel_hit(Side::Right, 3, 200.0, -40.0, 1.0), Some(PanelHit::Row(0)));
    assert_eq!(panel_hit(Side::Right, 3, 200.0, 70.0, 1.0), Some(PanelHit::Row(2)));
    assert_eq!(panel_hit(Side::Right, 3, 100.0, 0.0, 1.0), None);
    assert_eq!(panel_hit(Side::Right, 3, 400.0, 0.0, 2.0), Some(PanelHit::Row(1))); // scaled
}

#[test]
fn panel_hit_mirrors_on_the_left() {
    let r = panel_rect(Side::Left, 4, 1.0);
    assert_eq!((r.left, r.right), (-316.0, -148.0));
    assert_eq!(panel_hit(Side::Left, 4, -200.0, -40.0, 1.0), panel_hit(Side::Right, 4, 200.0, -40.0, 1.0));
    assert_eq!(panel_hit(Side::Left, 4, 200.0, -40.0, 1.0), None);
}

#[test]
fn panel_side_flips_at_the_right_edge() {
    let work = Rect { left: 0, top: 0, right: 1920, bottom: 1080 };
    assert_eq!(panel_side(960, work, 1.0), Side::Right);
    assert_eq!(panel_side(1920 - 160, work, 1.0), Side::Left); // needs 316 to the right
    assert_eq!(panel_side(1920 - 316, work, 1.0), Side::Right);
    assert_eq!(panel_side(1920 - 400, work, 1.5), Side::Left); // 316 * 1.5 = 474
}

// session.rs
#[test]
fn resting_on_an_option_tile_opens_its_panel() {
    let mut s = WheelSession::new();
    s.set_inputs(vec![video()]);
    let (x, y) = at(4, 6); // GIF
    assert!(s.hover(x, y, 1.0, 0)); // hovered changed
    assert!(s.panel().is_none() && s.switch_pending());
    assert!(!s.hover(x + 1.0, y, 1.0, 299));
    assert!(s.panel().is_none());
    assert!(s.tick(PANEL_DWELL_MS));
    let p = s.panel().unwrap();
    assert_eq!((p.item, p.set.title, p.hovered), (4, "GIF 옵션", None));
    assert!(!s.switch_pending());
    // Leaving before the dwell ends cancels it.
    let mut t = WheelSession::new();
    t.set_inputs(vec![video()]);
    let (x, y) = at(2, 6); // MOV
    t.hover(x, y, 1.0, 0);
    let (x, y) = at(5, 6);
    t.hover(x, y, 1.0, 100);
    assert!(!t.tick(400));
    assert!(t.panel().is_none());
}

#[test]
fn panel_survives_the_trip_across_the_wheel() {
    let mut s = WheelSession::new();
    s.set_inputs(vec![video()]);
    open_panel(&mut s, 4); // GIF, lower left
    s.hover(0.0, 0.0, 1.0, 1000); // through the cancel area
    let (x, y) = at(2, 6); // MOV has its own options — but only passing
    s.hover(x, y, 1.0, 1100);
    let (rx, ry) = row_point(Side::Right, 3, 1);
    s.hover(rx, ry, 1.0, 1200);
    assert!(!s.tick(2000));
    assert_eq!(s.panel().unwrap().set.title, "GIF 옵션");
    assert_eq!(s.hovered(), Hit::Choice(1));
    // Resting on another option tile switches; a tile without options never does.
    let (x, y) = at(0, 6); // MKV
    s.hover(x, y, 1.0, 3000);
    assert!(s.tick(3000 + PANEL_DWELL_MS));
    assert_eq!(s.panel().unwrap().set.title, "해상도");
    let (x, y) = at(5, 6); // MP3
    s.hover(x, y, 1.0, 4000);
    s.tick(5000);
    assert_eq!(s.panel().unwrap().set.title, "해상도");
}

#[test]
fn drop_on_a_row_uses_its_preset_and_the_tile_uses_the_default() {
    let mut s = WheelSession::new();
    s.set_inputs(vec![video()]);
    let (gx, gy) = at(4, 6);
    open_panel(&mut s, 4);
    let (rx, ry) = row_point(Side::Right, 3, 0);
    s.hover(rx, ry, 1.0, 1000);
    assert_eq!(s.hovered(), Hit::Choice(0));
    assert_eq!(s.panel().unwrap().hovered, Some(0));
    assert_eq!(s.drop_at(rx, ry, 1.0), Some(Selection { op: Operation::Convert(Format::Gif), preset: Some(Preset::Gif { fps: 10, max_width: Some(320) }) }));
    assert_eq!(s.drop_at(gx, gy, 1.0), Some(Selection { op: Operation::Convert(Format::Gif), preset: Some(Preset::Gif { fps: 15, max_width: Some(480) }) }));
    let r = panel_rect(Side::Right, 3, 1.0);
    assert_eq!(s.drop_at((r.left + r.right) / 2.0, r.top + 5.0, 1.0), None); // header
    let (mx, my) = at(5, 6);
    assert_eq!(s.drop_at(mx, my, 1.0), Some(Selection { op: Operation::Convert(Format::Mp3), preset: None }));
}

#[test]
fn left_panel_and_cycle() {
    let mut s = WheelSession::new();
    s.set_inputs(vec![video()]);
    s.set_panel_side(Side::Left);
    open_panel(&mut s, 4);
    let (rx, ry) = row_point(Side::Left, 3, 2);
    s.hover(rx, ry, 1.0, 1000);
    assert_eq!(s.hovered(), Hit::Choice(2));
    s.cycle();
    assert!(s.panel().is_none() && !s.switch_pending());
}
```

기존 session 테스트는 `hover(…, 0)`와 `Selection` 비교로 바꾼다.

- [ ] **Step 2: 실패 확인** — Run: `cargo test -p swisswheel-core` / Expected: 컴파일 실패
- [ ] **Step 3: 구현**
- [ ] **Step 4: 통과 확인** — Run: `cargo test --workspace` / Expected: 모두 통과 (`app.rs`는 컴파일만 맞춘다: `hover(…, 0)`, `submit(sel.op)`; 옵션 전달은 Task 8, 시간·타이머는 Task 9)
- [ ] **Step 5: 커밋** — `git commit -am "feat(core): option panel layout, hit testing, and sticky hover"`

---

### Task 8: 앱 연결 — 옵션 전달 · 툴팁 진행률 · 문서 · CI

**Files:**
- Modify: `crates/swisswheel-core/src/summary.rs`, `crates/swisswheel/src/app.rs`, `README.md`, `docs/manual-test.md`, `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: Task 6 `Job { op, preset, inputs }`·`Runner::new(image, media, …)`·`Runner::progress`, Task 7 `Selection`.
- Produces: `pub fn busy_tooltip(active_jobs: usize, progress: Option<f32>) -> String`

- [ ] **Step 1: 실패하는 테스트 작성** (`summary.rs`)

```rust
#[test]
fn busy_tooltips() {
    assert_eq!(busy_tooltip(0, None), "SwissWheel");
    assert_eq!(busy_tooltip(0, Some(0.5)), "SwissWheel");
    assert_eq!(busy_tooltip(1, None), "SwissWheel · 작업 중 1개");
    assert_eq!(busy_tooltip(2, Some(0.374)), "SwissWheel · 작업 중 2개 · 37%");
    assert_eq!(busy_tooltip(1, Some(1.7)), "SwissWheel · 작업 중 1개 · 100%");
}
```

- [ ] **Step 2: 실패 확인** — Run: `cargo test -p swisswheel-core summary` / Expected: 컴파일 실패
- [ ] **Step 3: 구현**
  - `busy_tooltip`: 퍼센트 `(p.clamp(0.0, 1.0) * 100.0).round()`.
  - `app.rs`: `Runner::new(min(코어, 4), 2, …)`; `on_drop`의 `Selection`을 `submit(sel.op, sel.preset)`로 넘겨 `Job.preset`에 담는다. `TIMER_PROGRESS` 500ms를 작업이 1개 이상일 때 켜고 0개면 끈다; 틱과 `update_busy`에서 `tray.set_busy(active > 0, &busy_tooltip(active, runner.progress()))`.
  - `README.md`: 지원 표에 영상·오디오 줄과 옵션 패널 설명, "FFmpeg 필요: 앱 폴더 `ffmpeg\` 또는 PATH (설치 파일에는 다음 버전부터 포함)".
  - `docs/manual-test.md`에 M13–M16: M13 영상 → WEBM 중 툴팁 %가 오른다 / M14 긴 영상 작업 중 트레이 → 종료: 수 초 안에 종료, 작업 관리자에 ffmpeg.exe 없음, 임시 파일 없음 / M15 FLAC → MP3 완료 알림, 클릭 시 결과 선택 / M16 애니메이션 GIF → MP4 재생.
  - CI: `cargo test` 앞에 `- run: choco install ffmpeg -y --no-progress`.
- [ ] **Step 4: 통과 확인** — Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` / Expected: 모두 통과. `cargo build --release`로 실행해 M13–M16을 확인하고 결과를 `docs/manual-test.md` 표에 적는다 (패널은 아직 안 그려지므로 칸에 드롭하면 기본값으로 변환된다).
- [ ] **Step 5: 커밋** — `git add -A && git commit -m "feat(app): media jobs end to end with options and progress tooltip"`

---

### Task 9: 옵션 패널 그리기 (수동 검증)

**Files:**
- Modify: `crates/swisswheel/src/wheel/mod.rs`, `crates/swisswheel/src/wheel/window.rs`, `crates/swisswheel/src/wheel/render.rs`, `crates/swisswheel/src/app.rs`, `docs/manual-test.md`

**Interfaces:**
- Consumes: Task 7 `panel_side`, `panel_rect`, `Side`, `WheelSession::{set_panel_side, panel}`, `PanelState`.
- Produces:
  - `WheelView`에 `pub panel: Option<PanelView<'a>>` — `pub struct PanelView<'a> { pub title: &'a str, pub choices: &'a [Choice], pub default: usize, pub hovered: Option<usize>, pub side: Side }`.
  - `WheelWindow::place_at`: `panel_side`로 쪽을 정하고, 창을 휠 정사각형 + 패널 쪽으로 `(PANEL_WIDTH - PANEL_OVERLAP + HOVER_GROW)·s`만큼 넓힌다(높이는 그대로 — 4줄 패널 196 DIP < 휠 332 DIP). 창 안에서의 휠 중심 위치를 보관하고 `pub fn panel_side(&self) -> Side`로 알려 준다.
  - `WheelWindow::set_panel(&mut self, rows: Option<usize>)` — 바뀔 때만 창 영역을 원(닫힘) 또는 원 ∪ 패널 사각형(`CombineRgn(RGN_OR)`)으로 갱신.
  - `Renderer::new(hwnd, width, height)`와 `draw`는 직사각형 표면, 휠 중심을 인자로 받는다. 유리 배경은 휠 원 클립 스프라이트 + 패널용 `CompositionRoundedRectangleGeometry` 클립 스프라이트(반경 10 DIP, 패널이 열렸을 때만 보임).
  - 패널 모양 (spec 2.5): 휠 원판과 같은 틴트·테두리, 모서리 10 DIP, 제목(10.5 semibold, subtext) 왼쪽 위, 줄마다 이름(13 semibold, 왼쪽)과 설명(10.5, subtext, 오른쪽 정렬), 기본 줄 설명 뒤에 `· 기본`. 강조 줄은 휠 칸과 같은 빨강 그라데이션 둥근 사각형 + 흰 글자. 열림은 120ms 페이드.
  - `app.rs`: 휠을 열 때 `session.set_panel_side(wheel.panel_side())`. `hover`/`tick`의 `now_ms`는 휠을 연 시각부터의 밀리초. `switch_pending()`이 참인 동안 `TIMER_PANEL` 50ms로 `tick`을 부른다. `hover`·`tick`이 `true`면 `wheel.set_panel(session.panel().map(|p| p.set.choices.len()))` 후 다시 그림. 오프셋 계산은 창 안의 휠 중심 기준.

- [ ] **Step 1: 구현** — 위 Interfaces대로. 순수 로직 변경은 없음 (Task 7이 담당).
- [ ] **Step 2: 검사** — Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` / Expected: 모두 통과
- [ ] **Step 3: 수동 확인** — `docs/manual-test.md`에 다음을 추가하고 release 빌드로 확인:
  - M17 영상 드래그 → Space → GIF 칸에 올림: 오른쪽에 "GIF 옵션" 패널(3줄, 보통 줄에 `· 기본`)
  - M18 GIF 칸(왼쪽 아래)에서 가운데를 지나 오른쪽 패널로 곧장 가도 패널 유지 / MKV 칸에 잠깐(0.3초) 머물면 "해상도"로 바뀜 / Tab이면 닫힘
  - M19 "작게" 줄에 드롭 → 결과 GIF가 가로 320 이하·10fps, "보통"보다 작음 / 제목 줄에 드롭 → 아무 일 없음
  - M20 화면 오른쪽 끝에서 열면 패널이 왼쪽 / 패널이 닫힌 상태에서 패널 자리에 드롭하면 그 아래 창에 정상 드롭
  - M21 사진 드래그 → 도구 휠 "축소" 칸 패널에서 25% → `이름 (25%).png`, 변환 휠 JPG 패널 "작게" → 보통보다 작은 파일
- [ ] **Step 4: 커밋** — `git add -A && git commit -m "feat(app): draw the option panel beside the wheel"`
