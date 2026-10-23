# 업데이트 알림 설계 문서

- 작성일: 2026-10-23
- 상태: 승인됨 (대화에서 합의), 사용자 검토 대기
- 기반: `main` (771c727). 업데이터(`2026-10-22-updater-design.md`), 다운로드 패널(`2026-10-23-download-panel-design.md`), 알림 카드(`notices.rs`, `notice.rs`)를 재사용한다.

## 1. 목적과 범위

**목적:** 앱이 켜질 때 SwissWheel과 FFmpeg의 새 버전을 확인해 알림 카드로 알린다. 카드의 **[이 버전 건너뛰기]**로 그 버전을 무시하면, 더 새 버전이 나올 때까지 다시 알리지 않는다. 큰 오류가 있는 버전이나 업데이트하기 싫은 사용자를 위한 기능이다.

**범위:**
- 확인은 앱이 켜지고 1분 뒤 한 번만 한다(하루 한 번 같은 주기 확인은 없다).
- 설정의 일반 탭에 "업데이트 자동 확인 켬/끔" 줄을 더한다. 기본값은 켬이다.
- 알림 카드가 버튼을 가질 수 있게 한다(지금은 카드 전체가 한 가지 동작).

**범위 밖:**
- 주기적 확인, 확인 간격 설정.
- 건너뛴 버전을 되돌리는 UI(`config.toml`에서 지우면 된다).
- 직접 설치해 PATH에 있는 FFmpeg의 알림(다운로드 패널 원칙대로 건드리지 않는다).
- 알림 카드 버튼의 마우스 올림 강조, 키보드 조작(카드는 포커스를 받지 않는다).
- 풍선 알림(알림 창을 못 만든 경우)의 버튼. 풍선에는 글만 나온다.

## 2. 사용자 경험

### 2.1 확인 시점
- 앱이 켜지고 **60초 뒤** 한 번, 백그라운드에서 두 가지를 확인한다. 로그인 직후에는 네트워크가 아직 안 붙은 경우가 많아서 기다린다.
  - **SwissWheel:** 업데이트 타일이 보는 GitHub 릴리스 주소(`update::endpoint()`).
  - **FFmpeg:** SwissWheel이 직접 설치한 FFmpeg(`Installed::Own`)이고 그 버전을 읽을 수 있을 때만. 다운로드 패널의 상태 확인을 그대로 쓰므로, 결과가 패널의 "최신 버전" 기억에도 들어간다.
- 60초가 될 때 자동 확인이 꺼져 있으면 아무것도 하지 않는다. 켜는 동안 앱이 이미 1분 넘게 돌고 있었다면 다음 시작 때부터 확인한다.
- 디버그 미리보기(`SWISSWHEEL_PREVIEW`)에서는 확인하지 않는다.
- 확인에 실패해도(인터넷 없음, 해석 실패) 알림은 없고 로그에만 남는다.

### 2.2 알림 카드
```
┌──────────────────────────────────┐
│ (⤓)  업데이트                     │
│      SwissWheel v0.3.0이 나왔습니다 │   FFmpeg는 "FFmpeg 9.0.3이 나왔습니다"
│      [  업데이트  ] [이 버전 건너뛰기] │
│ ──────────────────────────────── │   ← 타이머 막대
└──────────────────────────────────┘
```
- 새 종류 `NoticeKind::Update`: 15초 동안 떠 있다. 마우스를 올리면 시간이 멈춘다(다른 카드와 같음). 아이콘은 다운로드 모양, 강조색 원.
- 버튼 두 개가 본문 아래 한 줄에 나란히 놓인다. 글 칸 너비를 반씩 나눈다. 첫 버튼은 강조색으로 채우고, 둘째는 옅은 바탕이다.
- **[업데이트]:** 업데이트 타일(앱)이나 다운로드 패널의 [업데이트](FFmpeg)를 누른 것과 같다. 확인 창이 한 번 뜨고, 변환 중이거나 다른 작업이 돌면 지금처럼 거절 알림이 뜬다.
- **[이 버전 건너뛰기]:** 그 버전을 `config.toml`에 저장하고 카드를 닫는다. 앱을 다시 켜도 유지된다.
- 카드의 버튼이 아닌 곳을 누르거나 시간이 다 되면 카드만 닫힌다. 그 버전은 다음 시작 때 다시 알린다.
- 전체 화면(게임 등)이면 다른 알림처럼 보류했다가 끝나면 15초를 처음부터 다시 띄운다. 방해 금지는 작업 결과만 막으므로 업데이트 카드는 뜬다.
- 앱과 FFmpeg에 둘 다 새 버전이 있으면 카드 두 장이 뜬다.

### 2.3 알릴지 판단
- 다음이 모두 맞을 때만 알린다.
  1. 최신 버전이 지금 버전보다 새것이다.
  2. 건너뛴 버전이 없거나, 최신 버전이 건너뛴 버전보다도 새것이다.
- 건너뛴 값을 해석할 수 없으면(손으로 고친 값 등) 없는 것으로 본다.
- 업데이트 타일과 다운로드 패널처럼 **직접** 확인할 때는 건너뛰기와 상관없이 지금처럼 새 버전을 보여 주고 받을 수 있다. 건너뛰기는 자동 알림만 막는다.

### 2.4 설정
- 일반 탭: `Windows 시작 시 실행`, **`업데이트 자동 확인`**, `강조색` 순서(3줄, `MAX_ROWS`와 같다).
- 값은 켬/끔, ←/→로 바꾼다. 바로 저장된다.

### 2.5 `config.toml`
```toml
[updates]
auto_check = true
skip_app = "0.3.0"      # 건너뛴 SwissWheel 버전, 없으면 줄이 없다
skip_ffmpeg = "8.0.1"   # 건너뛴 FFmpeg 버전
```

## 3. 구조

### 3.1 core (`swisswheel-core`)
- `config.rs`: `Updates { auto_check: bool, skip_app: Option<String>, skip_ffmpeg: Option<String> }`, 기본값 `auto_check = true`. `None`은 쓰지 않는다(`skip_serializing_if`). `Config.updates`.
- `settings.rs`: `SettingItem::AutoCheck` ("업데이트 자동 확인", 켬/끔), 일반 탭에 넣는다.
- `notices.rs`:
  - `NoticeKind::Update` (15초).
  - `NoticeButton { label: &'static str, action: NoticeAction }`, `Notice.buttons: Vec<NoticeButton>`.
  - `NoticeAction`에 `Update(Product)`, `Skip(Product, String)`을 더한다. `Product { App, Ffmpeg }`.
  - `Notices::click(id, button: Option<usize>, now)`: 버튼을 누르면 그 버튼의 동작, 아니면 카드의 `action`. 닫히는 중인 카드는 지금처럼 무시한다.
  - `CardView.buttons: Vec<&'static str>`(라벨).
- 새 모듈 `update_notice.rs`:
  - `DELAY` (60초).
  - `app_due(latest: Version, current: Version, skipped: Option<&str>) -> bool`.
  - `ffmpeg_due(state: &EssentialState, skipped: Option<&str>) -> Option<String>`: 알릴 FFmpeg 버전.
  - `notice(product, version) -> Notice`: 2.2의 카드(제목 "업데이트", 본문, 버튼 둘).
  - `text`: 본문 문구. 받침에 따라 "이/가"를 붙인다(`v0.3.0이`, `9.0.2가`).

### 3.2 앱 (`swisswheel`)
- `update/mod.rs`: `run`의 릴리스 받기·해석 부분을 `latest(endpoint) -> Option<Release>`로 꺼낸다. `run`은 그것을 쓴다. `spawn_check(hwnd, msg, slot)`: 확인만 하는 스레드. 대화 상자를 띄우지 않는다.
- `notice.rs`:
  - 카드 높이에 버튼 줄(`BUTTONS`)을 더한다. 버튼 두 개를 그린다.
  - 버튼 자리를 따로 기억해(`BUTTON_HITS`), 클릭이 버튼이면 `WM_APP_NOTICE`를 `wparam = 2 + 버튼 번호`로 보낸다(0 = 올림, 1 = 카드 클릭은 그대로).
- `app.rs`:
  - `TIMER_AUTO_CHECK` (한 번만): 시작할 때 미리보기가 아니면 건다.
  - 타이머가 오면 `config.updates.auto_check`를 보고 앱 확인 스레드를 띄우고, FFmpeg 쪽은 `auto_ffmpeg = true`로 둔 뒤 `start_essentials_check()`를 부른다(이미 돌고 있으면 그 결과를 쓴다).
  - `WM_APP_AUTO_CHECK` (`WM_APP + 37`): 앱 결과로 `app_due`를 보고 카드를 띄운다.
  - `on_essentials_checked`: 마지막 보고이고 `auto_ffmpeg`이면 그것을 끄고 `ffmpeg_due`를 본다.
  - `on_notice`: `Update(App)` → `start_update()`, `Update(Ffmpeg)` → `run_essential(EssentialAction::Update)`, `Skip(p, v)` → 설정에 저장.
  - `apply_config`의 저장 부분을 `save_config(before, item: Option<SettingItem>)`로 꺼내 건너뛰기 저장도 같은 길로 간다(실패하면 지금처럼 경고 카드, 설정 줄 ⚠는 줄이 있을 때만).
  - 디버그 `notices` 미리보기에 업데이트 카드를 더한다.

## 4. 오류 처리와 안전
- 확인 스레드는 대화 상자를 띄우지 않고, 결과는 슬롯과 `PostMessageW`로만 넘긴다(기존 규칙).
- 자동 확인은 업데이트 다운로드(`self.update`)와 따로 돈다. 버튼을 눌렀을 때만 기존 규칙(`on_tile`, `essentials::on_action`)으로 시작 여부를 정한다.
- 설정 파일이 처음에 망가져 있었으면(`config_writable = false`) 건너뛰기는 이번 실행에만 적용되고 저장되지 않는다(다른 설정과 같다).

## 5. 테스트
- core:
  - `app_due`, `ffmpeg_due`: 새 버전 없음, 있음, 건너뜀과 같음, 건너뜀보다 새것, 건너뛴 값이 이상함, 시스템 FFmpeg, 버전 모름, 최신 모름.
  - 본문 문구와 받침.
  - `config`: 기본값, 읽기, 저장 왕복, 없는 `skip_*`는 쓰지 않음.
  - `settings`: 일반 탭 순서, 켬/끔 바꾸기.
  - `notices`: 버튼 클릭은 그 버튼의 동작, 카드 클릭은 카드 동작, 닫히는 카드는 무시, `Update` 수명 15초, `CardView.buttons`.
- 앱:
  - `update::latest`: 가짜 서버로 정상, 500, 해석 실패.
  - `notice.rs`: 버튼 줄이 카드 높이를 늘림, 버튼 자리 판정.
  - `config_file::save`: `[updates]`에 건너뛴 버전이 저장되고 다른 줄은 그대로.
- 수동 테스트: `docs/manual-test.md`에 M84부터.

## 6. 문서
- README: 자동 확인과 건너뛰기 한두 문장.
- `docs/manual-test.md`: M84~.
- 업데이터 설계 문서에 이 문서를 가리키는 한 줄.
