# SwissWheel 영상·오디오 변환 (FFmpeg) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 영상·오디오 변환과 영상 도구(압축·오디오 추출·음소거)를 FFmpeg로 실제 실행하고, 트레이 툴팁에 진행률을 보여준다 (spec 8장 4단계).

**Architecture:** FFmpeg 명령 인자를 정하는 일(어떤 코덱·품질·먹서, 스트림 복사 가능 여부, ffprobe/`-progress` 출력 해석)은 OS 무관한 순수 함수로 `swisswheel-core::media`에 두고 단위 테스트한다. 앱 크레이트의 `engines::ffmpeg`는 ffmpeg/ffprobe를 찾고, 자식 프로세스를 창 없이 Job Object에 묶어 실행하며 진행률과 취소를 처리한다. 작업 실행기는 이미지 풀과 미디어 풀(2개)을 따로 두고 진행률을 조회할 수 있게 하며, 종료 시 실행 중인 FFmpeg를 죽인다.

**Tech Stack:** 기존 그대로 (Rust 1.99, `windows` 0.61, `image` 0.25, `jpeg-encoder` 0.6 …). 새 크레이트 의존성 없음. 외부 실행 파일: FFmpeg GPL 빌드 (개발 PC에는 WinGet으로 PATH에 9.0.1 설치됨).

**Spec:** `docs/superpowers/specs/2026-10-20-swisswheel-design.md` (3.1, 3.2, 2.4, 4.4)

**Branch:** `feat/shell-and-image`(최종 리뷰 전)에서 `feat/media`를 따서 작업한다. 앞 브랜치가 리뷰 후 `main`에 합쳐지면 이 브랜치를 그 위로 rebase한다.

## Global Constraints

- 이 계획은 spec 8장 4단계만: **동작 휠(공유·복사·잘라내기·경로 복사·터미널), 자동 실행, 트레이 메뉴 확장, FFmpeg 동봉·패키징은 다음 계획.** 동작 항목 드롭은 지금처럼 "다음 버전에서 지원 예정입니다" 알림.
- `swisswheel-core`는 계속 OS 비의존 (의존성: `serde`, `toml`, `thiserror`만).
- 품질 (spec 3.1/3.2): MP3 `quality.mp3_kbps`(기본 192), 영상 H.264 CRF 23 / VP9 CRF 32, 영상 압축 H.264 CRF 28 + 오디오 128kbps, 컨테이너 유지(MP4·MKV·MOV), 불가 시 MP4.
- FFmpeg 찾기: `<실행 파일 폴더>\ffmpeg\ffmpeg.exe` + `ffprobe.exe`, 둘 다 없으면 `PATH`.
- FFmpeg 실행: `-hide_banner -nostdin -loglevel error -y -progress pipe:1 -nostats -i <입력> <인자> <임시 출력>`. 임시 출력은 `.swisswheel-tmp`로 끝나므로 **항상 `-f <먹서>`를 명시**한다. `CREATE_NO_WINDOW`, 표준 입력 null, 프로세스는 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` Job Object에 할당.
- 동시 실행: 이미지 `min(코어 수, 4)`, FFmpeg 2.
- 원본은 절대 수정·삭제·덮어쓰기 금지. 결과 확정은 기존 `fsutil::finalize`(대체 없는 이동)만 사용.
- UI 문구 한국어. 커밋 메시지 끝:
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` 와 `Claude-Session: https://claude.ai/code/session_016PYsmXwW5jKgU1htHsgH9G`
- 매 Task 끝: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` 통과. FFmpeg가 필요한 테스트는 FFmpeg가 없으면 건너뛰지 말고 "FFmpeg(ffmpeg.exe, ffprobe.exe)가 PATH에 필요합니다"로 실패한다.

## 인코딩 규칙 (이 계획이 정하는 값)

| 출력 | 인자 (`-map` 뒤, `-f` 앞) |
|---|---|
| MP4, MOV, MKV (인코딩) | `-c:v libx264 -preset medium -crf 23 -pix_fmt yuv420p -vf scale=trunc(iw/2)*2:trunc(ih/2)*2 -c:a aac -b:a 192k` (+ MP4·MOV는 `-movflags +faststart`) |
| WEBM | `-c:v libvpx-vp9 -crf 32 -b:v 0 -deadline good -cpu-used 4 -row-mt 1 -c:a libopus -b:a 128k` |
| AVI | `-c:v mpeg4 -q:v 4 -c:a libmp3lame -b:a <mp3_kbps>k` |
| GIF | `-map 0:v:0 -vf fps=15,scale='min(480,iw)':-2:flags=lanczos,split[a][b];[a]palettegen[p];[b][p]paletteuse -loop 0` |
| MP3 / M4A / WAV / FLAC / OGG / OPUS | `-map 0:a:0` + `libmp3lame -b:a <mp3_kbps>k` / `aac -b:a 192k` / `pcm_s16le` / `flac` / `libvorbis -q:a 6` / `libopus -b:a 128k` |

- 영상 출력의 매핑: `-map 0:v:0 -map 0:a?`.
- **스트림 복사 먼저 시도** (인코딩보다 수백 배 빠름): MKV는 항상, MP4·MOV는 영상 ∈ {h264, hevc} 이고 오디오 ∈ {없음, aac, mp3}, WEBM은 영상 ∈ {vp8, vp9, av1} 이고 오디오 ∈ {없음, opus, vorbis}. AVI와 애니메이션 GIF 입력은 복사하지 않는다. 복사 인자: `-map 0:v:0 -map 0:a? -c copy` (+ hevc→MP4·MOV는 `-tag:v hvc1`, MP4·MOV는 `-movflags +faststart`). 복사가 실패하면 인코딩으로 다시 한다.
- 먹서: MP4 `mp4`, MKV `matroska`, WEBM `webm`, MOV `mov`, AVI `avi`, GIF `gif`, MP3 `mp3`, M4A `ipod`, WAV `wav`, FLAC `flac`, OGG `ogg`, OPUS `opus`. (WMV·WMA·AIFF는 입력 전용 → 먹서 없음)
- 오디오 추출: 오디오 코덱이 aac→M4A, mp3→MP3, opus→OPUS, vorbis→OGG, flac→FLAC, pcm_s16le→WAV 이면 `-map 0:a:0 -c:a copy` 먼저(실패 시 그 포맷으로 인코딩), 그 밖의 코덱은 MP3 인코딩.
- 음소거: 출력 = 입력 포맷(먹서가 없으면 MP4). 입력 포맷 그대로면 `-map 0:v -an -c:v copy` 먼저, 다음은 그 포맷의 영상 인코딩 + `-an`.

## Review Focus

1. **영상 작업 도중 트레이에서 종료** — 앱이 수 초 안에 끝나야 하고, ffmpeg.exe가 남거나 `.swisswheel-tmp`·반쯤 쓴 결과가 남으면 안 된다. → Task 3 `cancel_kills_ffmpeg`, Task 4 `shutdown_cancels_running_media`
2. **홀수 크기 입력** (애니메이션 GIF 81×61, 세로 폰 영상) — libx264는 홀수 크기를 거부하므로 짝수 보정 없이는 실패한다. → Task 3 `animated_gif_to_mp4`
3. **오디오가 없는 영상을 MP3로·오디오 추출** — FFmpeg 오류 대신 "오디오 트랙이 없습니다"가 떠야 한다. → Task 2 `audio_targets`, Task 3 `tools_on_video`
4. **스트림 복사가 실패하는 조합** — 복사 시도가 실패해도 인코딩으로 끝까지 만들어야 하고, 실패한 시도의 임시 파일을 지워야 한다. → Task 2 `compatible_streams_are_remuxed_first`, Task 3 `later_attempt_runs_when_first_fails`
5. **한글·공백 경로와 `.swisswheel-tmp` 확장자** — 먹서를 확장자에서 추측하면 실패한다. → Task 3 `unicode_paths`

---

### Task 1: ffprobe·진행률 해석 (core)

**Files:**
- Create: `crates/swisswheel-core/src/media.rs` (이 Task는 해석 부분만)
- Modify: `crates/swisswheel-core/src/lib.rs` (`pub mod media;`)

**Interfaces:**
- Produces:
  - `#[derive(Debug, Clone, Default, PartialEq)] pub struct MediaInfo { pub duration_secs: Option<f64>, pub video: Option<String>, pub audio: Option<String> }` — 종류별 **첫** 스트림의 코덱 이름.
  - `pub fn parse_probe(text: &str) -> MediaInfo` — `ffprobe -v error -show_entries format=duration:stream=codec_type,codec_name -of default=noprint_wrappers=1` 출력(`codec_name=` 다음 줄에 `codec_type=`, 마지막에 `duration=`; `N/A` 가능, CRLF 가능).
  - `#[derive(Debug, PartialEq)] pub enum Progress { OutTime(f64), End }` (초 단위), `pub fn parse_progress(line: &str) -> Option<Progress>` — `out_time_us=`, 없으면 `out_time_ms=`(FFmpeg에서 이것도 마이크로초), `progress=end`.
  - `pub fn fraction(out_time_secs: f64, duration_secs: Option<f64>) -> Option<f32>` — 0..=1로 자름, 길이가 없거나 0 이하면 `None`.
  - `pub fn muxer(format: Format) -> Option<&'static str>` — 위 먹서 표.

- [ ] **Step 1: 실패하는 테스트 작성** (`media.rs`의 `mod tests`)

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
    let expect = [(Mp4, "mp4"), (Mkv, "matroska"), (Webm, "webm"), (Mov, "mov"), (Avi, "avi"), (Gif, "gif"),
                  (Mp3, "mp3"), (M4a, "ipod"), (Wav, "wav"), (Flac, "flac"), (Ogg, "ogg"), (Opus, "opus")];
    for (f, m) in expect {
        assert_eq!(muxer(f), Some(m), "{f:?}");
    }
    for f in [Wmv, Wma, Aiff, Png] {
        assert_eq!(muxer(f), None, "{f:?}");
    }
}
```

- [ ] **Step 2: 실패 확인** — Run: `cargo test -p swisswheel-core media` / Expected: 컴파일 실패 (`parse_probe` 등 없음)
- [ ] **Step 3: 구현** — 위 Interfaces 그대로. `parse_probe`는 줄마다 `trim_end()` 후 `key=value`로 나누고, 마지막 `codec_name`을 기억했다가 `codec_type`이 `video`/`audio`일 때 그 종류가 비어 있으면 채운다.
- [ ] **Step 4: 통과 확인** — Run: `cargo test -p swisswheel-core media` / Expected: 5 passed
- [ ] **Step 5: 커밋** — `git add crates/swisswheel-core && git commit -m "feat(core): parse ffprobe and ffmpeg progress output"`

---

### Task 2: FFmpeg 작업 계획 (core)

**Files:**
- Modify: `crates/swisswheel-core/src/media.rs`

**Interfaces:**
- Consumes: Task 1의 `MediaInfo`, `muxer`; `catalog::{Operation, Tool}`; `config::Quality`.
- Produces:
  - `#[derive(Debug, Clone, PartialEq)] pub struct MediaPlan { pub output: Format, pub attempts: Vec<Vec<String>>, pub duration_secs: Option<f64> }` — `attempts`는 순서대로 시도할 인자 목록(각각 `-i <입력>` 뒤, 출력 경로 앞에 들어감, 항상 `"-f", <먹서>`로 끝남). `duration_secs`는 `info`에서 복사.
  - `#[derive(Debug, PartialEq, thiserror::Error)] pub enum PlanError { #[error("no audio stream")] NoAudio, #[error("no video stream")] NoVideo, #[error("unsupported")] Unsupported }`
  - `pub fn supports(op: &Operation, input: Format) -> bool` — FFmpeg가 맡는 조합: 영상 입력 → 영상 대상·GIF·MP3 변환과 영상 도구 3개, 오디오 입력 → 오디오 대상 변환, GIF 입력 → MP4·WEBM 변환. 나머지 `false`.
  - `pub fn plan(op: &Operation, input: Format, info: &MediaInfo, quality: &Quality) -> Result<MediaPlan, PlanError>` — 위 "인코딩 규칙" 그대로. 대상이 입력과 같거나 `supports`가 거짓이면 `Unsupported`, 오디오 출력인데 `info.audio == None`이면 `NoAudio`, 영상·GIF 출력이거나 음소거인데 `info.video == None`이면 `NoVideo`.

- [ ] **Step 1: 실패하는 테스트 작성** (헬퍼 `fn info(v: Option<&str>, a: Option<&str>) -> MediaInfo`(길이 `Some(2.0)`), `fn has(args: &[String], seq: &[&str]) -> bool`(연속 부분열), `fn ends_with_f(args: &[String], m: &str) -> bool`)

```rust
#[test]
fn video_convert_reencodes_with_spec_quality() {
    let q = Quality::default();
    let p = plan(&Operation::Convert(Format::Mp4), Format::Avi, &info(Some("mpeg4"), Some("mp3")), &q).unwrap();
    assert_eq!(p.output, Format::Mp4);
    assert_eq!(p.attempts.len(), 1);
    let a = &p.attempts[0];
    assert!(has(a, &["-c:v", "libx264"]) && has(a, &["-crf", "23"]) && has(a, &["-c:a", "aac"]));
    assert!(has(a, &["-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2"]) && has(a, &["-movflags", "+faststart"]));
    assert!(ends_with_f(a, "mp4"));
    let w = &plan(&Operation::Convert(Format::Webm), Format::Avi, &info(Some("mpeg4"), Some("mp3")), &q).unwrap().attempts[0];
    assert!(has(w, &["-c:v", "libvpx-vp9", "-crf", "32"]) && has(w, &["-c:a", "libopus"]) && ends_with_f(w, "webm"));
    let avi = &plan(&Operation::Convert(Format::Avi), Format::Mp4, &info(Some("h264"), Some("aac")), &q).unwrap().attempts[0];
    assert!(has(avi, &["-c:v", "mpeg4"]) && has(avi, &["-b:a", "192k"]) && ends_with_f(avi, "avi"));
}

#[test]
fn compatible_streams_are_remuxed_first() {
    let q = Quality::default();
    let mkv = plan(&Operation::Convert(Format::Mkv), Format::Mp4, &info(Some("h264"), Some("aac")), &q).unwrap();
    assert_eq!(mkv.attempts.len(), 2);
    assert!(has(&mkv.attempts[0], &["-c", "copy"]) && ends_with_f(&mkv.attempts[0], "matroska"));
    assert!(has(&mkv.attempts[1], &["-c:v", "libx264"]));
    let hevc = plan(&Operation::Convert(Format::Mp4), Format::Mkv, &info(Some("hevc"), Some("aac")), &q).unwrap();
    assert!(has(&hevc.attempts[0], &["-tag:v", "hvc1"]));
    let vp9 = plan(&Operation::Convert(Format::Webm), Format::Mkv, &info(Some("vp9"), Some("opus")), &q).unwrap();
    assert!(has(&vp9.attempts[0], &["-c", "copy"]));
    // Opus doesn't belong in MP4 by our rule, and AVI never copies.
    assert_eq!(plan(&Operation::Convert(Format::Mp4), Format::Mkv, &info(Some("h264"), Some("opus")), &q).unwrap().attempts.len(), 1);
    assert_eq!(plan(&Operation::Convert(Format::Avi), Format::Mkv, &info(Some("mpeg4"), Some("mp3")), &q).unwrap().attempts.len(), 1);
}

#[test]
fn animated_gif_to_video_encodes_with_even_size() {
    let p = plan(&Operation::Convert(Format::Mp4), Format::Gif, &info(Some("gif"), None), &Quality::default()).unwrap();
    assert_eq!(p.attempts.len(), 1);
    assert!(has(&p.attempts[0], &["-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2"]));
}

#[test]
fn audio_targets() {
    let q = Quality::default();
    let mp3 = plan(&Operation::Convert(Format::Mp3), Format::Flac, &info(None, Some("flac")), &q).unwrap();
    assert!(has(&mp3.attempts[0], &["-map", "0:a:0"]) && has(&mp3.attempts[0], &["-c:a", "libmp3lame", "-b:a", "192k"]));
    assert!(ends_with_f(&mp3.attempts[0], "mp3"));
    let loud = Quality { mp3_kbps: 320, ..Quality::default() };
    assert!(has(&plan(&Operation::Convert(Format::Mp3), Format::Wav, &info(None, Some("pcm_s16le")), &loud).unwrap().attempts[0], &["-b:a", "320k"]));
    assert!(ends_with_f(&plan(&Operation::Convert(Format::M4a), Format::Mp3, &info(None, Some("mp3")), &q).unwrap().attempts[0], "ipod"));
    assert!(has(&plan(&Operation::Convert(Format::Wav), Format::Flac, &info(None, Some("flac")), &q).unwrap().attempts[0], &["-c:a", "pcm_s16le"]));
    // From a video: fine with audio, a clear error without.
    assert!(plan(&Operation::Convert(Format::Mp3), Format::Mp4, &info(Some("h264"), Some("aac")), &q).is_ok());
    assert_eq!(plan(&Operation::Convert(Format::Mp3), Format::Mp4, &info(Some("h264"), None), &q), Err(PlanError::NoAudio));
}

#[test]
fn video_to_gif_uses_a_palette() {
    let p = plan(&Operation::Convert(Format::Gif), Format::Mp4, &info(Some("h264"), Some("aac")), &Quality::default()).unwrap();
    assert_eq!(p.output, Format::Gif);
    let a = &p.attempts[0];
    assert!(a.iter().any(|s| s.starts_with("fps=15,scale='min(480,iw)'") && s.contains("paletteuse")));
    assert!(has(a, &["-loop", "0"]) && ends_with_f(a, "gif"));
}

#[test]
fn video_tools() {
    let q = Quality::default();
    let c = plan(&Operation::Tool(Tool::CompressVideo), Format::Webm, &info(Some("vp9"), Some("opus")), &q).unwrap();
    assert_eq!(c.output, Format::Mp4);
    assert!(has(&c.attempts[0], &["-crf", "28"]) && has(&c.attempts[0], &["-b:a", "128k"]));
    assert_eq!(plan(&Operation::Tool(Tool::CompressVideo), Format::Mkv, &info(Some("h264"), Some("aac")), &q).unwrap().output, Format::Mkv);
    let ex = plan(&Operation::Tool(Tool::ExtractAudio), Format::Mp4, &info(Some("h264"), Some("aac")), &q).unwrap();
    assert_eq!(ex.output, Format::M4a);
    assert!(has(&ex.attempts[0], &["-c:a", "copy"]) && ex.attempts.len() == 2);
    let ac3 = plan(&Operation::Tool(Tool::ExtractAudio), Format::Mkv, &info(Some("h264"), Some("ac3")), &q).unwrap();
    assert_eq!(ac3.output, Format::Mp3);
    assert_eq!(plan(&Operation::Tool(Tool::ExtractAudio), Format::Mp4, &info(Some("h264"), None), &q), Err(PlanError::NoAudio));
    let mute = plan(&Operation::Tool(Tool::Mute), Format::Mp4, &info(Some("h264"), Some("aac")), &q).unwrap();
    assert_eq!(mute.output, Format::Mp4);
    assert!(has(&mute.attempts[0], &["-an"]) && has(&mute.attempts[0], &["-c:v", "copy"]));
    let wmv = plan(&Operation::Tool(Tool::Mute), Format::Wmv, &info(Some("wmv2"), Some("wmav2")), &q).unwrap();
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
    assert!(!supports(&Operation::Convert(Mp4), Mp3)); // audio → video isn't offered
    assert!(!supports(&Operation::Tool(Tool::HalveImage), Mp4));
    assert!(!supports(&Operation::Tool(Tool::Mute), Mp3));
    assert_eq!(plan(&Operation::Convert(Mp4), Mp4, &info(Some("h264"), None), &Quality::default()), Err(PlanError::Unsupported));
}
```

- [ ] **Step 2: 실패 확인** — Run: `cargo test -p swisswheel-core media` / Expected: 컴파일 실패 (`plan` 없음)
- [ ] **Step 3: 구현** — 인자는 `&'static str` 상수 조각(`H264`, `VP9`, …)을 `Vec<String>`으로 이어 붙인다. `supports`는 `catalog`의 `VIDEO_TARGETS`/`AUDIO_TARGETS`/`ANIMATED_GIF_TARGETS`와 같은 집합을 써야 하므로 그 상수들을 `pub(crate)`로 열어 재사용한다.
- [ ] **Step 4: 통과 확인** — Run: `cargo test -p swisswheel-core` / Expected: 기존 40 + 이 계획 12 passed
- [ ] **Step 5: 커밋** — `git commit -am "feat(core): plan ffmpeg arguments per operation"`

---

### Task 3: FFmpeg 엔진 (app)

**Files:**
- Create: `crates/swisswheel/src/engines/ffmpeg.rs`
- Modify: `crates/swisswheel/src/engines/mod.rs` (`pub mod ffmpeg;`, `EngineError` 변형 추가)
- Modify: `crates/swisswheel/Cargo.toml` (`windows` 기능 `Win32_System_JobObjects` 등 컴파일러가 요구하는 것)

**Interfaces:**
- Consumes: Task 1·2의 `media::{MediaInfo, MediaPlan, PlanError, parse_probe, parse_progress, fraction, plan}`.
- Produces:
  - `EngineError`에 추가: `MediaRead(String)` → "영상·오디오 파일을 읽을 수 없습니다", `NoAudio` → "오디오 트랙이 없습니다", `NoVideo` → "영상 트랙이 없습니다", `MissingFfmpeg` → "FFmpeg를 찾을 수 없습니다", `Cancelled` → "취소되었습니다". (`Failed(String)`의 내용은 FFmpeg stderr 끝부분 — 알림 문구는 기존 "변환에 실패했습니다".)
  - `#[derive(Debug, Clone, PartialEq)] pub struct Tools { pub ffmpeg: PathBuf, pub ffprobe: PathBuf }`
  - `pub fn locate() -> Option<Tools>` = `locate_in(<exe 폴더>, std::env::var_os("PATH").as_deref())`
  - `pub fn locate_in(app_dir: &Path, path_var: Option<&OsStr>) -> Option<Tools>` — `app_dir\ffmpeg\` 에 두 파일이 다 있으면 그것, 아니면 `PATH`의 각 폴더에서 두 파일이 다 있는 첫 폴더.
  - `pub fn prepare(tools: &Tools, op: &Operation, input: &Path, quality: &Quality) -> Result<MediaPlan, EngineError>` — ffprobe 실행 + `media::plan`. ffprobe가 실패하면 `MediaRead`, `PlanError`는 같은 이름의 변형으로.
  - `pub fn run(tools: &Tools, plan: &MediaPlan, input: &Path, output: &Path, progress: &dyn Fn(f32), cancel: &AtomicBool) -> Result<(), EngineError>` — `attempts`를 순서대로 실행, 실패한 시도의 출력 파일은 지우고 다음으로. 진행률은 `progress=end`에서 1.0. `cancel`이 서면 100ms 안에 프로세스를 죽이고 출력 파일을 지운 뒤 `Cancelled`.
  - 테스트 헬퍼 (`#[cfg(test)] pub(crate) mod tests`): `pub(crate) fn tools() -> Tools` (없으면 위 Global Constraints 문구로 panic), `pub(crate) fn make(dir: &Path, name: &str, args: &[&str]) -> PathBuf` — `ffmpeg -y <args> <dir/name>`로 픽스처를 만든다. 예: 2초 160×120 영상 + 사인파 `["-f","lavfi","-i","testsrc=duration=2:size=160x120:rate=10","-f","lavfi","-i","sine=duration=2","-c:v","libx264","-c:a","aac","-shortest"]`.

구현 메모: stdout은 별도 스레드에서 줄 단위로 읽어 `parse_progress` → `fraction` 값을 `mpsc`로 보내고, 호출 스레드는 `recv_timeout(100ms)`로 받으며 `cancel`과 `try_wait()`를 확인한다 (`progress`가 `Send`가 아니어도 되도록). stderr도 별도 스레드에서 읽어 끝 2KB만 남긴다. Job Object는 프로세스 전역에서 한 번 만들어 핸들을 유지한다 (앱이 죽으면 핸들이 닫히며 ffmpeg도 같이 종료). 생성과 할당 사이의 아주 짧은 틈은 감수한다.

- [ ] **Step 1: 실패하는 테스트 작성** (`ffmpeg.rs`의 `mod tests`; `probe_out(path) -> MediaInfo`는 `prepare` 안에서 쓰는 ffprobe 호출을 재사용)

```rust
#[test]
fn locates_bundled_before_path() {
    let (app, path) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let touch = |dir: &Path| { for n in ["ffmpeg.exe", "ffprobe.exe"] { std::fs::write(dir.join(n), b"").unwrap(); } };
    touch(path.path());
    let path_var = std::env::join_paths([path.path()]).unwrap();
    assert_eq!(locate_in(app.path(), Some(&path_var)).unwrap().ffmpeg, path.path().join("ffmpeg.exe"));
    let bundled = app.path().join("ffmpeg");
    std::fs::create_dir(&bundled).unwrap();
    std::fs::write(bundled.join("ffmpeg.exe"), b"").unwrap(); // ffprobe missing → still PATH
    assert_eq!(locate_in(app.path(), Some(&path_var)).unwrap().ffmpeg, path.path().join("ffmpeg.exe"));
    touch(&bundled);
    assert_eq!(locate_in(app.path(), Some(&path_var)).unwrap(), Tools { ffmpeg: bundled.join("ffmpeg.exe"), ffprobe: bundled.join("ffprobe.exe") });
    assert_eq!(locate_in(tempfile::tempdir().unwrap().path(), None), None);
}

#[test]
fn converts_video() { /* clip.mp4 → Webm, Mp3, Gif, Mkv; 각 출력의 probe: (vp9, opus), (None, mp3), (gif, None), (h264, aac) */ }

#[test]
fn audio_conversions() { /* tone.flac(사인파 2초) → Mp3, Opus, Wav: audio == mp3 / opus / pcm_s16le, 길이 2.0 ± 0.2 */ }

#[test]
fn tools_on_video() {
    /* clip.mp4: ExtractAudio → plan.output == M4a, 출력 audio == aac;
       Mute → video Some, audio None; CompressVideo → video == h264;
       silent.mp4(오디오 없음): prepare(ExtractAudio) → Err(EngineError::NoAudio) */
}

#[test]
fn animated_gif_to_mp4() { /* testsrc size=81x61 → anim.gif → Convert(Mp4) → video == h264 */ }

#[test]
fn progress_reaches_one() { /* clip.mp4 → Webm: 받은 값이 1개 이상, 감소하지 않음, 마지막 == 1.0 */ }

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
    /* run(...) → Ok, 출력 probe video == h264 */
}

#[test]
fn cancel_kills_ffmpeg() {
    /* 20초 640×480 영상 → Webm, 300ms 뒤 다른 스레드가 cancel = true;
       run → Err(EngineError::Cancelled), 걸린 시간 < 5초, 출력 파일 없음 */
}

#[test]
fn unicode_paths() { /* "가족 영상/생일 파티.mp4" → Mp3, 출력은 "…/생일 파티.mp3.swisswheel-tmp" (임시 이름 그대로) → probe audio == mp3 */ }

#[test]
fn unreadable_file_is_media_read_error() { /* "broken.mp4"에 b"not a video" → prepare → Err(EngineError::MediaRead(_)) */ }
```

`engines/mod.rs`에도 추가:
```rust
#[test]
fn media_user_messages() {
    assert_eq!(EngineError::NoAudio.user_message("a.mp4"), "a.mp4: 오디오 트랙이 없습니다");
    assert_eq!(EngineError::MissingFfmpeg.user_message("a.mp4"), "a.mp4: FFmpeg를 찾을 수 없습니다");
    assert_eq!(EngineError::MediaRead("x".into()).user_message("a.mp4"), "a.mp4: 영상·오디오 파일을 읽을 수 없습니다");
}
```

- [ ] **Step 2: 실패 확인** — Run: `cargo test -p swisswheel engines` / Expected: 컴파일 실패
- [ ] **Step 3: 구현** — Interfaces와 구현 메모대로.
- [ ] **Step 4: 통과 확인** — Run: `cargo test -p swisswheel engines` / Expected: 이미지 8 + FFmpeg 10 + 메시지 1 passed. 작업 관리자에서 테스트 후 남은 `ffmpeg.exe`가 없는지 확인.
- [ ] **Step 5: 커밋** — `git commit -am "feat(app): ffmpeg engine with progress, cancel, and remux fallback"`

---

### Task 4: 작업 실행기 — 미디어 풀, 진행률, 종료 시 취소

**Files:**
- Modify: `crates/swisswheel/src/jobs.rs`

**Interfaces:**
- Consumes: Task 3의 `ffmpeg::{locate, prepare, run, Tools}`, `EngineError::{MissingFfmpeg, Cancelled}`; `media::supports`.
- Produces:
  - `Runner::new(image_workers: usize, media_workers: usize, quality: Quality, on_done: OnDone) -> Runner` — 생성 시 `ffmpeg::locate()`를 한 번 호출해 보관 (없으면 미디어 파일마다 `MissingFfmpeg` 실패).
  - `Runner::progress(&self) -> Option<f32>` — 아직 보고되지 않은 작업들의 파일 전체 평균 (끝난 파일 1.0, 실행 중 파일은 마지막 진행률, 대기 0.0). 작업이 없으면 `None`.
  - `Runner::shutdown(self)` — 취소 플래그를 세운다: 실행 중 FFmpeg는 죽고, 대기 중인 파일은 처리하지 않고 "취소되었습니다"로 보고한다. 그 뒤 워커를 join.
  - 파일별 라우팅: `image::supports` → 이미지 풀, `media::supports` → 미디어 풀, 둘 다 아니면 즉시 `EngineError::Unsupported` 문구 ("지원하지 않는 작업입니다"). 미디어는 `prepare`로 얻은 `plan.output`의 확장자로 이름을 예약한다.
  - 실패 시 `log_error!("{name}: {e}")`로 상세(`Display`, FFmpeg stderr 포함)를 남긴다.

- [ ] **Step 1: 실패하는 테스트 작성** (`run_jobs`는 `Runner::new(2, 2, …)`로 바꾸고, `ffmpeg::tests::make` 사용)

```rust
#[test]
fn media_job_converts_and_reports() { /* clip.mp4 → Convert(Mp3): outputs == [dir/"clip.mp3"], failures 빈 목록 */ }

#[test]
fn media_output_extension_comes_from_probe() { /* clip.mp4(aac) → Tool(ExtractAudio): outputs == [dir/"clip.m4a"] */ }

#[test]
fn progress_while_running_and_none_when_idle() {
    /* 10초 320×240 → Webm. 보고가 오기 전에 progress()가 Some(p), 0.0 <= p <= 1.0 인 순간을 적어도 한 번 관찰;
       보고를 받은 뒤 progress() == None; 그다음 shutdown */
}

#[test]
fn shutdown_cancels_running_media() {
    /* 30초 640×480 → Webm 제출, 500ms 뒤 shutdown(): 걸린 시간 < 5초,
       폴더에 "clip.webm"도 ".swisswheel-tmp"로 끝나는 파일도 없음 */
}
```

기존 `unsupported_operations_report_per_file`은 대상을 `Convert(Format::Png)`로 바꾸고 기대값을 `"clip.mp4: 지원하지 않는 작업입니다"`로 고친다 (MP4→MP3는 이제 지원).

- [ ] **Step 2: 실패 확인** — Run: `cargo test -p swisswheel jobs` / Expected: 컴파일 실패 (`Runner::new` 인자 수, `progress` 없음)
- [ ] **Step 3: 구현** — 풀은 기존 `worker` 루프를 그대로 두 벌(채널 두 개) 돌리고, 공유 상태에 `cancel: AtomicBool`과 진행 중 작업 목록을 둔다.
- [ ] **Step 4: 통과 확인** — Run: `cargo test --workspace` / Expected: 모두 통과
- [ ] **Step 5: 커밋** — `git commit -am "feat(app): media worker pool with progress and cancel on shutdown"`

---

### Task 5: 앱 연결 · 툴팁 진행률 · 문서 · CI

**Files:**
- Modify: `crates/swisswheel-core/src/summary.rs` (`busy_tooltip`)
- Modify: `crates/swisswheel/src/app.rs`
- Modify: `README.md`, `docs/manual-test.md`, `docs/superpowers/specs/2026-10-20-swisswheel-design.md`, `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: Task 4의 `Runner::new(image, media, …)`, `Runner::progress`.
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
  - `busy_tooltip`: 퍼센트는 `(p.clamp(0, 1) * 100).round()`.
  - `app.rs`: `Runner::new(min(코어, 4), 2, …)`. `TIMER_PROGRESS` 500ms를 작업이 1개 이상일 때 켜고 0개가 되면 끈다. 틱마다와 `update_busy`에서 `tray.set_busy(active > 0, &busy_tooltip(active, runner.progress()))`.
  - `README.md`: 지원 표에 영상·오디오 줄 추가, "FFmpeg 필요: 앱 폴더 `ffmpeg\` 또는 PATH (설치 파일에는 다음 버전부터 포함)".
  - `docs/manual-test.md`: M13 영상 → WEBM 중 트레이 툴팁에 %가 오른다 / M14 긴 영상 작업 중 트레이 → 종료: 수 초 안에 종료, 작업 관리자에 ffmpeg.exe 없음, 임시 파일 없음 / M15 FLAC → MP3 완료 알림, 클릭 시 결과 선택 / M16 애니메이션 GIF → MP4 재생 확인.
  - spec 4.4에 "FFmpeg는 앱 폴더 `ffmpeg\`를 먼저, 없으면 PATH에서 찾는다"와 이 계획의 "인코딩 규칙" 표를 3.1 아래 "3.1.1 FFmpeg 인코딩 규칙"으로 옮긴다.
  - CI: `cargo test` 앞에 `- run: choco install ffmpeg -y --no-progress`.
- [ ] **Step 4: 통과 확인** — Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` / Expected: 모두 통과. 그다음 `cargo build --release`로 실행해 M13–M16을 직접 확인하고 결과를 `docs/manual-test.md` 표에 적는다.
- [ ] **Step 5: 커밋** — `git add -A && git commit -m "feat(app): media jobs end to end with progress tooltip"`
