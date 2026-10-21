# SwissWheel

Offline file converter for Windows. Drag a file in Explorer, press **Space**, and drop it on a format in the wheel that pops up under your cursor. **Tab** switches between the convert, tools, and actions wheels; **Esc** closes the wheel and keeps your drag going. Nothing is ever uploaded.

## Usage

1. Run `swisswheel.exe`. A red-and-white wheel icon appears in the notification area.
2. Start dragging one or more files in Explorer or on the desktop.
3. While dragging, press **Space**. Drop on an item to run it; drop on the center to cancel.
4. The result is saved next to the original (`photo.png` → `photo.jpg`). Originals are never modified or overwritten; name clashes get a number (`photo (1).jpg`).
5. A notification reports the result. Click it to open the folder with the result selected.

Quit from the tray icon's right-click menu. Settings live in `%APPDATA%\SwissWheel\config.toml` (keys and quality); logs in `%LOCALAPPDATA%\SwissWheel\logs`.

## What works now (v0.1 in progress)

- Image conversion between JPG, PNG, WebP, BMP, TIFF, GIF, and ICO (multi-size, aspect kept)
- Image tools: shrink (25/50/75%), compress (JPG/WebP re-encode, lossless PNG optimization)
- Video conversion between MP4, MKV, WebM, MOV, and AVI, plus video → GIF and video → MP3; animated GIF → MP4/WebM. Compatible streams are copied instead of re-encoded, so MP4 ↔ MKV is near-instant.
- Audio conversion between MP3, M4A, WAV, FLAC, OGG, and Opus (WMA and AIFF as input)
- Video tools: compress (H.264), extract audio (copied when possible), mute
- Options: some items take options (GIF frame rate and size, video resolution, compression strength, shrink ratio, JPG/WebP quality). Rest on such an item for a moment and a panel opens beside the wheel; drop on one of its rows, or on the item itself for the default.
- Progress: while jobs run, a small card above the tray lists each file with a progress bar, and a ring around the tray icon fills up. Either can be turned off in `config.toml` (`[progress] window`, `tray_ring`). Quitting cancels running video work and cleans up.

Video and audio need FFmpeg: put `ffmpeg.exe` and `ffprobe.exe` in an `ffmpeg` folder next to `swisswheel.exe`, or have them on `PATH` (the installer will include them from the next version).

Shown in the wheel but not yet implemented (dropping on them says "다음 버전에서 지원 예정입니다"): the actions wheel (share, copy, cut, copy path, terminal).

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

Manual checks that automated tests can't cover: [docs/manual-test.md](docs/manual-test.md). Design: [docs/superpowers/specs/2026-10-20-swisswheel-design.md](docs/superpowers/specs/2026-10-20-swisswheel-design.md).

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
