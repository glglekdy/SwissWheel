# SwissWheel

Offline file converter for Windows. Drag a file in Explorer, press **Space**, and drop it on a format in the wheel that pops up under your cursor. **Tab** switches between the convert, tools, and actions wheels; **Esc** closes the wheel and keeps your drag going. Nothing is ever uploaded.

Status: early development (v0.1 in progress). Design: [docs/superpowers/specs/2026-10-20-swisswheel-design.md](docs/superpowers/specs/2026-10-20-swisswheel-design.md).

## Build

Requires Rust (stable, MSVC toolchain) and Visual Studio Build Tools with the C++ workload.

```
cargo build
cargo test --workspace
```

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
