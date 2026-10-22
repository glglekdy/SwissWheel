# SwissWheel

Offline file converter for Windows. Drag a file in Explorer, press **Space**, and drop it on a format in the wheel that pops up under your cursor. **Tab** switches between the convert, tools, and actions wheels; **Esc** closes the wheel and keeps your drag going. Nothing is ever uploaded.

## Usage

1. Run `swisswheel.exe`. The SwissWheel logo appears in the notification area (white on a dark taskbar, black on a light one; it may sit under the ^ overflow).
2. Start dragging one or more files in Explorer or on the desktop.
3. While dragging, press **Space**. Drop on an item to run it; drop on the center to cancel.
4. The result is saved next to the original (`photo.png` → `photo.jpg`). Originals are never modified or overwritten; name clashes get a number (`photo (1).jpg`).
5. A card above the tray (bottom-right) reports the result. Click it to open the folder with the result selected; the click closes the card and doesn't take focus from the app you're typing in. Cards pause while the cursor is over them. In a full-screen app, a presentation, or while Windows is busy, cards wait and the newest three show when it ends (a key prompt posted meanwhile is dropped).

Without a drag, click the desktop and press **Space** to open the menu bar at the top of the screen: **설정** (settings), **다운로드** (set up FFmpeg, see below), **플러그인** (plugins), **업데이트** (update), **재시작** (restart SwissWheel; it waits while files are converting or an update or download runs). Move with **←/→**, open with **Enter**, close with **Esc** or **Space**. The settings panel has tabs — **단축키** (keys), **도구** (tools), **변환** (convert), **알림** (notices), **일반** (general): **←/→** on the tab line switches tabs (or click one), **↓** goes into its settings. There **↑/↓** picks a setting and **←/→** changes it (or click ◀ ▶, or scroll); changes apply at once and are saved, no restart needed. **일반** holds **강조색** (accent color: 블루, 퍼플, 핑크, 레드, 오렌지, 그린, 그래파이트); **알림** holds the progress card, the tray ring, and **방해 금지** (do not disturb), which hides only the job done/failed cards — menu results and key prompts still show, and the tray tooltip keeps showing the last job ("마지막 작업: …").

To change a shortcut key, select its row and press **Enter** (or click the key shown), then press the new key. Allowed keys: Space, Tab, Esc, CapsLock, \`, F1–F12, Insert, Delete, Home, End, PageUp, PageDown, Pause, ScrollLock. A key another shortcut already uses swaps the two. A card shows while it waits; it ends after 5 seconds, a click elsewhere, or a click on the card. Note that the open key also opens this menu on the desktop: bound to a key the desktop uses itself (Delete, F2, F5, Home…), that key opens the menu there instead of doing its usual job.

Quit from the tray icon's right-click menu. Settings can be changed in the menu's settings panel or in `%APPDATA%\SwissWheel\config.toml` (comments you add there are kept when the panel saves); logs in `%LOCALAPPDATA%\SwissWheel\logs`.

## What works now (v0.1 in progress)

- Image conversion between JPG, PNG, WebP, BMP, TIFF, GIF, and ICO (multi-size, aspect kept)
- Image tools: shrink (25/50/75%), compress (JPG/WebP re-encode, lossless PNG optimization)
- Video conversion between MP4, MKV, WebM, MOV, and AVI, plus video → GIF and video → MP3; animated GIF → MP4/WebM. Compatible streams are copied instead of re-encoded, so MP4 ↔ MKV is near-instant.
- Audio conversion between MP3, M4A, WAV, FLAC, OGG, and Opus (WMA and AIFF as input)
- Video tools: compress (H.264), extract audio (copied when possible), mute
- Options: some items take options (GIF frame rate and size, video resolution, compression strength, shrink ratio, JPG/WebP quality). Rest on such an item for a moment and a panel opens beside the wheel; drop on one of its rows, or on the item itself for the default.
- Progress: while jobs run, a small card above the tray lists each file with a progress bar, and a ring around the tray icon fills up. When a job finishes, its card turns into the result card. Either can be turned off in the settings panel or `config.toml` (`[progress] window`, `tray_ring`). Quitting cancels running video work and cleans up.
- Desktop menu and settings panel: wheel keys (press to bind), the shrink ratio and video compression strength used when you drop on the tool itself, start with Windows, JPG/WebP quality, MP3 bitrate, and the progress displays, accent color, and do not disturb, all applied live. The UI uses a liquid-glass look (transparency can be turned off in Windows) with the embedded Pretendard font.

Video and audio need FFmpeg: choose **다운로드** in the desktop menu to fetch the latest essentials build from [GyanD/codexffmpeg](https://github.com/GyanD/codexffmpeg/releases) and set it up (checked against GitHub's SHA-256; it goes in an `ffmpeg` folder next to `swisswheel.exe`, or `%LOCALAPPDATA%\SwissWheel\ffmpeg` when that folder can't be written). If FFmpeg is already usable it just says so. You can also put `ffmpeg.exe` and `ffprobe.exe` in either folder yourself, or have them on `PATH`.

Shown but not yet implemented (they say "다음 버전에서 지원 예정입니다"): the actions wheel (share, copy, cut, copy path, terminal), and the menu's plugin tile.

## Updates

Choose **업데이트** in the desktop menu. SwissWheel checks the latest GitHub release and, if it is newer, asks before downloading it. The download is checked against the release's SHA-256 before it replaces `swisswheel.exe`; the app then restarts on the new version with your settings as they were. If anything fails, the current version stays. While files are converting, it asks you to try again when they finish; if a conversion starts during the download, the update waits for it. When `swisswheel.exe` sits in a folder it can't write to (such as Program Files), it opens the download page instead.

## Known limitations

- Drags that start in an app running as administrator can't be received (Windows UIPI). Don't run SwissWheel itself as administrator either.
- Apps that implement their own drag-and-drop instead of OLE won't open the wheel.
- The wheel may not appear over exclusive full-screen games.

## Build

Requires Rust (stable, MSVC toolchain) and Visual Studio Build Tools with the C++ workload. The tests also need FFmpeg (`ffmpeg.exe`, `ffprobe.exe`) on `PATH`; they make their own sample clips with it.

```
cargo build --release -p swisswheel
cargo test --workspace
```

To publish a version: raise `version` in the root `Cargo.toml`, commit, then `git tag vX.Y.Z` and `git push origin vX.Y.Z`. The release workflow checks that the tag matches `Cargo.toml`, runs the tests, builds, and publishes `swisswheel.exe` with `swisswheel.exe.sha256` as a GitHub release. The in-app updater reads that release, so the repository has to be public for other people's copies to see it.

Manual checks that automated tests can't cover: [docs/manual-test.md](docs/manual-test.md). Design: [docs/superpowers/specs/2026-10-20-swisswheel-design.md](docs/superpowers/specs/2026-10-20-swisswheel-design.md).

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).

Pretendard (assets/fonts, embedded in `swisswheel.exe`) — Copyright (c) 2021, Kil Hyung-jin (https://github.com/orioncactus/pretendard), with Reserved Font Name Pretendard. SIL Open Font License 1.1, included unmodified; full text in [assets/fonts/OFL.txt](assets/fonts/OFL.txt), also attached to every release as `OFL.txt`.
