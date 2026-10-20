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
- Image tools: 50% downscale, compress (JPG/WebP re-encode, lossless PNG optimization)

Shown in the wheel but not yet implemented (dropping on them says "다음 버전에서 지원 예정입니다"): video and audio conversion and tools (FFmpeg), and the actions wheel (share, copy, cut, copy path, terminal).

## Known limitations

- Drags that start in an app running as administrator can't be received (Windows UIPI). Don't run SwissWheel itself as administrator either.
- Apps that implement their own drag-and-drop instead of OLE won't open the wheel.
- The wheel may not appear over exclusive full-screen games.

## Build

Requires Rust (stable, MSVC toolchain) and Visual Studio Build Tools with the C++ workload.

```
cargo build --release -p swisswheel
cargo test --workspace
```

Manual checks that automated tests can't cover: [docs/manual-test.md](docs/manual-test.md). Design: [docs/superpowers/specs/2026-10-20-swisswheel-design.md](docs/superpowers/specs/2026-10-20-swisswheel-design.md).

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
