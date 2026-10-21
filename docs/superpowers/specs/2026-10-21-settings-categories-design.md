# 설정 패널 카테고리 탭과 키 바인딩 설계 문서

- 작성일: 2026-10-21
- 상태: 승인됨 (대화에서 합의), 사용자 검토 대기
- 기반: `feat/desktop-menu` (d7db256). `2026-10-21-desktop-menu-design.md`의 2.3(설정 패널 조작), 2.4(설정 항목), 2.6(모양) 중 패널 부분을 이 문서가 대신한다. 바, 열기, 자동 닫힘, 저장 방식은 그대로다.

## 1. 목적과 범위

**목적:** 한 줄로 이어진 설정 9개를 성격별 탭으로 나눠 찾기 쉽게 하고, 단축키는 목록을 넘기는 대신 원하는 키를 직접 눌러 지정하게 한다.

**이번 범위:**
1. 패널 위쪽에 카테고리 탭: 단축키 · 도구 · 변환 · 일반
2. 단축키 행: 선택 → Enter(또는 값 클릭) → 키를 눌러 바인딩. 겹치면 맞바꿈
3. 도구 탭 새 설정 2개: 축소 비율 기본값, 영상 압축 강도 기본값. 휠 옵션 패널의 "기본" 줄이 이 값을 따른다
4. 패널 맨 아래 안내 줄

**범위 밖:** GIF·영상 해상도·이미지 품질 단계 등 변환 옵션 세트의 기본값 설정, 휠에 보일 도구 켜기/끄기, 글자·숫자·수정키 바인딩, 탭 위 마우스 휠로 카테고리 넘기기.

**성공 기준:**
1. 키보드만으로 모든 탭의 모든 항목을 바꿀 수 있고, 마우스로도 같다.
2. 단축키 행에서 허용된 키를 누르면 즉시 바인딩·저장되고, 다른 단축키와 겹치면 서로 맞바뀐다. 허용되지 않은 키는 거절되고 계속 기다린다.
3. 키 입력 대기 중 누른 키는 바탕화면이나 다른 앱으로 새지 않는다(누름과 뗌 모두).
4. 도구 기본값을 바꾸면 다음 드롭부터 축소·영상 압축 타일에 바로 놓았을 때 그 값으로 실행되고, 옵션 패널의 "기본" 표시도 그 줄로 옮겨 간다.
5. 탭을 바꿔도 패널 크기가 변하지 않는다.

## 2. 사용자 경험

### 2.1 카테고리와 항목 (탭은 왼쪽부터)

| 탭 | 라벨 | `config.toml` 키 | 값 | 바꾸는 법 |
|---|---|---|---|---|
| 단축키 | 휠 열기 키 | `open_key` | 2.3의 키 | 키 입력 |
| | 휠 전환 키 | `cycle_key` | 〃 | 키 입력 |
| | 취소 키 | `cancel_key` | 〃 | 키 입력 |
| 도구 | 축소 비율 | `tools.shrink_percent` | 25% / 50% / 75% (기본 50) | ← / → |
| | 영상 압축 | `tools.compress_video` | 약하게 / 보통 / 강하게 (`"Light"` / `"Normal"` / `"Strong"`, 기본 보통) | ← / → |
| 변환 | JPG 품질 | `quality.jpg` | 50~100, 5 단위 | ← / → |
| | WebP 품질 | `quality.webp` | 50~100, 5 단위 | ← / → |
| | MP3 비트레이트 | `quality.mp3_kbps` | 128 / 160 / 192 / 256 / 320 kbps | ← / → |
| 일반 | Windows 시작 시 실행 | `start_with_windows` | 켬 / 끔 | ← / → |
| | 진행률 카드 | `progress.window` | 켬 / 끔 | ← / → |
| | 트레이 링 | `progress.tray_ring` | 켬 / 끔 | ← / → |

- ← / → 항목의 단계 밖 값, 끝값에서 멈춤 규칙은 기존 문서 2.4와 같다.
- `tools.shrink_percent`는 25·50·75만 유효하다. 다른 값이 파일에 있으면 다른 잘못된 값과 같이 경고를 띄우고 기본값으로 동작한다(파일은 건드리지 않음).

### 2.2 키보드

| 포커스 | ← / → | ↑ | ↓ | Enter | Esc |
|---|---|---|---|---|---|
| 바 (설정 타일) | 타일 이동 | – | 패널 열기 → 탭 줄 | 패널 열기 → 탭 줄 | 메뉴 닫기 |
| 탭 줄 | 카테고리 전환 (끝에서 멈춤) | 패널 닫고 바로 | 첫 항목 | 첫 항목 | 패널 닫고 바로 |
| 항목 | 값 변경 (단축키 행은 아무 일 없음) | 위 항목, 첫 항목에서는 탭 줄 | 아래 항목 (끝에서 멈춤) | 단축키 행: 키 입력 대기 / 그 밖: 없음 | 탭 줄 |
| 키 입력 대기 | 다음에 누른 키 하나가 바인딩 대상 (2.3) | | | | |

- 패널을 열 때마다 탭은 **단축키**에서 시작한다. 같은 메뉴가 열려 있는 동안에는 패널을 닫았다 다시 열어도 마지막 탭을 기억한다.
- 카테고리를 바꾸면 항목 선택은 없어지고 포커스는 탭 줄에 남는다.
- `open_key`는 지금처럼 메뉴 전체를 닫는다. 단, 키 입력 대기 중에는 바인딩 대상이다.

### 2.3 키 입력 대기

- 시작: 단축키 행에서 Enter, 또는 그 행의 값(키캡) 클릭.
- 허용 키와 파일 이름:

  | 키 | `config.toml` 값 | 패널 표시 |
  |---|---|---|
  | Space, Tab, Esc, CapsLock, \` | `"Space"`, `"Tab"`, `"Escape"`, `"CapsLock"`, `"Backquote"` (기존) | Space, Tab, Esc, CapsLock, \` |
  | F1~F12 | `"F1"`~`"F12"` | F1~F12 |
  | Insert, Delete, Home, End | `"Insert"`, `"Delete"`, `"Home"`, `"End"` | Insert, Delete, Home, End |
  | PageUp, PageDown | `"PageUp"`, `"PageDown"` | PgUp, PgDn |
  | Pause, ScrollLock | `"Pause"`, `"ScrollLock"` | Pause, ScrollLock |

- 허용 키를 누르면: 바인딩 → 즉시 저장·적용 → 대기 끝. 지금 값과 같은 키면 아무것도 바뀌지 않고 대기만 끝난다.
- 다른 단축키가 그 키를 쓰고 있으면 두 키를 맞바꾼다. 안내 줄: "휠 열기 키가 Tab으로 바뀌었습니다" (상대 항목과 그 항목이 새로 받은 키).
- 허용되지 않은 키(글자, 숫자, 방향키, Enter, Shift/Ctrl/Alt/Win 등): 안내 줄 "이 키는 쓸 수 없습니다", 계속 대기. 그 키의 누름과 뗌은 어디로도 전달되지 않는다.
- 취소: 5초 동안 입력 없음, 또는 그 키캡 말고 다른 곳 클릭(다른 행, 탭, 바 타일 포함. 바·패널 바깥 클릭은 지금처럼 메뉴 전체를 닫는다). 바인딩은 바뀌지 않는다.
- 대기 중 메뉴가 닫히면(바탕화면이 아닌 창이 전경으로, 드래그 시작 등) 대기도 끝난다.
- 열기 키를 바탕화면에서 원래 쓰는 키(Delete, F2, F5, Home 등)로 정하면 바탕화면에서는 그 키가 원래 기능 대신 메뉴를 연다. README에 적는다.

### 2.4 마우스

| 입력 | 동작 |
|---|---|
| 탭 클릭 | 그 카테고리로 전환, 포커스는 탭 줄 |
| 탭 위에 올림 | 강조만 (카테고리는 그대로) |
| 행 클릭 | 그 행 선택 |
| ◀ / ▶ 클릭, 행 위 마우스 휠 | ← / → 와 같음 (단축키 행엔 ◀ ▶가 없고 휠도 무시) |
| 단축키 행의 키캡 클릭 | 키 입력 대기 시작 |

### 2.5 모양

```
┌──────────────────────────────────┐
│ (설정 파일 오류 알림 — 있을 때만)  │
│  단축키    도구    변환    일반     │  탭 줄: 고른 탭은 흰 글자 + 밑줄
│ ─────────────────────────────────│
│ 휠 열기 키              [ Space ] │  단축키 행: ◀ ▶ 없이 키캡
│ 휠 전환 키              [  Tab  ] │
│ 취소 키                 [ 입력… ] │  대기 중: 강조 테두리
│ ─────────────────────────────────│
│ 바꿀 키를 누르세요 · 다른 곳 클릭 시 취소 │  안내 줄
└──────────────────────────────────┘
```

- 탭 4개는 패널 안쪽 폭을 똑같이 나눈다. 고른 탭은 본문색 글자와 밑줄, 나머지는 흐린 글자. 포커스가 탭 줄에 있으면 고른 탭에 행 선택과 같은 강조 배경. 마우스를 올린 탭은 옅은 강조.
- 패널 높이는 탭 줄 + 항목 3줄(가장 많은 카테고리) + 안내 줄로 고정. 도구 탭(2줄)은 아래 한 줄이 빈다.
- 단축키 행의 값은 ◀ ▶ 자리까지 포함한 폭의 키캡(둥근 테두리 상자). 대기 중엔 강조색 테두리와 "입력…".
- 안내 줄 문구 (흐린 글자):
  - 탭 줄: "←/→ 카테고리 · ↓ 항목"
  - 단축키 행: "Enter로 키 변경"
  - 그 밖의 행: "←/→로 값 변경"
  - 대기 중: "바꿀 키를 누르세요 · 다른 곳 클릭 시 취소"
  - 결과 문구(맞바꿈, 거절)는 다음 조작(키, 클릭, 휠)이 있을 때까지 위 문구 대신 보인다.

## 3. 구조

### 3.1 `swisswheel-core`

- `config.rs`
  - `Key`에 2.3의 키 추가. serde 이름은 변형 이름 그대로. `Key::ALL`, `Key::from_vk(u16) -> Option<Key>` 추가.
  - `Tools { shrink_percent: u8, compress_video: Strength }`, `Config.tools`. 기본 50 / `Normal`. 검증: `shrink_percent ∈ {25, 50, 75}`.
- `options.rs`: `Strength`에 `Serialize`/`Deserialize`. `OptionSet::default_index(&self, tools: &Tools) -> usize` — 축소 비율·압축 강도 세트면 설정값과 같은 프리셋의 줄, 아니면 고정 `default`.
- `session.rs`: 휠 세션이 `Tools`를 받아 드롭 시 기본 프리셋과 패널의 "기본" 줄을 `default_index`로 정한다.
- `settings.rs`
  - `Category { Keys, Tools, Convert, General }`, `CATEGORIES`, `label()`, `items() -> &'static [SettingItem]`.
  - `SettingItem::ShrinkPercent`, `CompressVideo` 추가, `is_key()`.
  - `step()`은 단축키 항목에 `None`.
  - `bind(c, item, key) -> Bound { config, swapped: Option<(SettingItem, Key)> }` — 같은 키면 `config` 그대로, 다른 단축키가 쓰면 맞바꿈.
  - 기존 평면 `ITEMS`는 없앤다.
- `menu.rs`
  - `MenuSession`: `tile`, `panel: Option<Panel>`, `category`(메뉴가 열려 있는 동안 유지), `hovered`. `Panel { focus: Focus, capturing: bool }`, `Focus { Tabs, Row(usize) }`.
  - `MenuHit::Tab(usize)`, `MenuHit::KeyCap(usize)` 추가.
  - `MenuAction::StartCapture`, `Bind(SettingItem, u16)`, `CancelCapture` 추가.
  - `capture(vk) -> MenuAction`: 대기 중이면 `Bind(현재 행 항목, vk)`. 허용 여부 판단과 결과 문구는 앱이 `settings::bind`로 한다. 거절이면 앱이 `rearm`해서 대기를 유지한다.
  - `cancel_capture()`: 타이머 만료용.
- `menu_geometry.rs`: 탭 줄 상자 4개, 행 상자(단축키 행은 `keycap` 상자), 안내 줄 상자. `layout()`은 고정 3행으로 계산하고 `hit()`이 `Tab`·`KeyCap`을 돌려준다.
- `drag.rs`: `capture: bool`. 메뉴가 열려 있고 `capture`이면 다음 키 누름(반복 아님) 하나를 어떤 키든 삼키고 `DragCommand::MenuCapture(vk)`를 보낸 뒤 `capture`를 끈다. 그 키는 `menu_press`로 표시돼 뗌도 삼켜진다. 메뉴가 닫히면 `capture`도 꺼진다. `set_capture(bool)` 추가.

### 3.2 `swisswheel` (앱)

- `hook.rs`: `MenuCapture` 인코딩(코드 21, vk는 LPARAM), `set_capture`.
- `app.rs`
  - `StartCapture` → `hook::set_capture(true)`, 5초 타이머(`TIMER_MENU_CAPTURE`), 다시 그림.
  - `MenuCapture(vk)` → `menu.capture(vk)` → `Bind`면 `Key::from_vk`. 없으면 "이 키는 쓸 수 없습니다" + 다시 `set_capture(true)` + 타이머 재시작. 있으면 `settings::bind` → `apply_config` → 맞바꿈 문구 → 타이머 해제.
  - `CancelCapture`, 메뉴 닫힘 → `set_capture(false)`, 타이머 해제.
  - 결과 문구(`menu_hint: Option<String>`)는 다음 메뉴 입력에서 지운다.
  - 휠 세션을 만들 때 `config.tools`를 넘긴다. 옵션 패널 렌더의 `default`도 세션이 준 값.
- `menubar/render.rs`: 탭 줄, 키캡 행, 안내 줄 그리기. `RowView`에 `keycap: bool`, `capturing: bool`. `MenuView`에 `category`, `tabs_focused`, `hovered_tab`, `hint`.

### 3.3 데이터 흐름 (키 바인딩)

```
Enter (단축키 행)
  → hook: MenuKey(Enter) → app → MenuSession::key → StartCapture
  → app: hook::set_capture(true), 타이머, 다시 그림
F5 누름
  → hook: tracker가 삼키고 MenuCapture(0x74) → app → MenuSession::capture → Bind(OpenKey, 0x74)
  → Key::from_vk → settings::bind → apply_config(키 재설정, 저장) → 안내 줄 → 다시 그림
F5 뗌 → menu_press로 삼켜짐
```

## 4. 오류 처리

- 저장 실패, 설정 파일 오류 시 동작은 기존 문서와 같다(값은 이번 실행에서만 적용, 오류 알림).
- `set_capture(true)`와 사용자의 키 누름 사이의 짧은 틈에 누른 키는 평소 메뉴 키 규칙대로 처리된다(기존 ms 단위 경합과 같은 성격, 받아들임).
- 키캡 대기 중 타이머 만료 전에 메뉴가 닫히면 타이머는 `close_menu`에서 해제한다.

## 5. 테스트

- 코어 단위 테스트
  - `config`: 새 `Key` 이름 파싱·직렬화 왕복, 기존 이름 호환, `[tools]` 기본값과 `shrink_percent` 검증.
  - `settings`: 카테고리별 항목, 도구 항목 `step`, 단축키 항목 `step`은 `None`, `bind`의 맞바꿈·같은 키·새 키.
  - `options`/`session`: `default_index`, 설정값이 드롭 기본 프리셋과 "기본" 줄에 반영.
  - `menu`: 2.2 표의 모든 칸, 탭 클릭·호버, 키캡 클릭, 대기 중 클릭 취소, `capture`, 대기 중 `open_key`가 바인딩되는 것.
  - `menu_geometry`: 고정 높이, 탭·키캡·행·◀▶ 적중.
  - `drag`: 대기 중 아무 키나 삼켜 `MenuCapture` 한 번, 뗌도 삼킴, 다음 키는 평소대로, 메뉴 닫히면 해제.
- 앱 단위 테스트: `MenuCapture` 인코딩 왕복, `config_file::save`가 `[tools]`를 새 섹션으로 쓰는 것.
- 눈 확인: `SWISSWHEEL_PREVIEW=menu-settings`에 탭·키 입력 대기 변형 추가(`menu-tools`, `menu-capture`).
- 수동 테스트(`docs/manual-test.md`): 탭 전환, F5 바인딩, 맞바꿈, 거절, 5초 취소, 대기 중 Space, 도구 기본값이 휠 드롭에 반영.

## 6. 문서

- README: 설정 패널 탭, 키 바인딩 방법, 허용 키, 열기 키를 바탕화면 키로 정할 때의 주의.
- `2026-10-21-desktop-menu-design.md` 2.3·2.4 머리에 "→ `2026-10-21-settings-categories-design.md`로 대체" 표시.
