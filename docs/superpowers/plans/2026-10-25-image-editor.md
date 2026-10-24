# Image Editor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Dropping one still image on a new ✎편집 wheel tile opens a floating glass panel where the user crops (with aspect chips), rotates, flips, grayscales, and strips info, with undo/redo, then saves `photo (edited).jpg` (or a lossless `photo (no info).jpg`) next to the original.

**Architecture:** All editor rules — edit history, the original→result transform, crop math, panel layout and hit testing, key/click/drag handling, the discard prompt — live in `swisswheel-core` and are unit tested. The app crate adds a Win32/Direct2D `editor/` module that only owns windows, a decode thread, and drawing, plus engine code (`apply_edits`, lossless `metadata::strip`) and a `Job.edits` field. `app.rs` only wires open/close/save through two posted messages.

**Tech Stack:** Rust 2024, `windows` 0.61 (Win32, Direct2D, DirectWrite, Composition via the existing `Surface`), `image` 0.25, `jpeg-encoder` 0.6, `webp` 0.3.

**Spec:** `docs/superpowers/specs/2026-10-25-image-editor-design.md`

## Global Constraints

- Originals are never modified; outputs go through the existing reserve → temp → `finalize` flow.
- Output names: pixel edits `이름 (edited).ext`, info-only `이름 (no info).ext`; same format as the input.
- Re-encode quality: JPG 95 / WebP 92 (`HIGH_JPG_QUALITY`, `HIGH_WEBP_QUALITY`); other formats lossless as today.
- The ✎편집 tile appears only for exactly one still image (JPG·PNG·WebP·BMP·TIFF·GIF·ICO, not an animated GIF), last in the Tools wheel; label `"편집"`, glyph `'\u{E70F}'`.
- Info strip is available only for JPG·PNG·WebP.
- Keys: C crop, R / Shift+R rotate cw / ccw, H flip horizontal, V flip vertical, G grayscale, I info, Ctrl+Z undo, Ctrl+Y and Ctrl+Shift+Z redo, Enter, Esc; Alt+F4 (`WM_CLOSE`) acts as Esc.
- Chips in order: `자유`, `원본`, `1:1`, `4:3`, `3:4`, `16:9`, `9:16`; `원본` = the current result's ratio.
- Minimum crop: 16×16 image pixels, or the whole side when the image side is shorter.
- Panel: centered in the work area of the monitor under the cursor; at most 80% of the work area; at least 720×480 DIPs (but never more than the work area).
- Dim window: layered, black, alpha 96/255, `WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED`; the panel is owned by it.
- Discard prompt lasts 3 s.
- Copy (Korean, exact): `"편집"` (tile, notice title), `"불러오는 중"`, `"편집을 버릴까요? Esc를 한 번 더 누르세요"`, `"편집하면 정보는 자동으로 빠집니다"`, `"편집기가 이미 열려 있습니다"`, `"편집기를 열 수 없습니다"`, `"편집기를 닫은 뒤 다시 시도하세요"`, footer buttons `"Esc 취소"` / `"↵ 저장"`, in crop mode `"Esc 자르기 취소"` / `"↵ 자르기 적용"`.
- New message ids: `WM_APP_EDITOR_OPEN = WM_APP + 38`, `WM_APP_EDITOR = WM_APP + 39` (app); panel-private `WM_APP_EDITOR_LOADED = WM_APP + 50`. Panel timer id `1` for the discard prompt.
- Running the app: only with `SWISSWHEEL_PREVIEW` set (a preview never rewrites the autostart Run value). Before any screen check, look at the foreground window and skip the check if a full-screen game owns it; list skipped checks in the task report.
- Never stage files you didn't change; stage by name.

## Review Focus

1. Opening the panel synchronously inside OLE `Drop` would freeze Explorer's drag loop and let Explorer take focus back → Task 12 posts `WM_APP_EDITOR_OPEN` and opens after `Drop` returns; reviewer checks `submit` never calls `editor::open` directly.
2. `SetForegroundWindow` can be refused (foreground lock), leaving keys going to Explorer → Task 10 falls back to `AttachThreadInput`; manual check in Task 13.
3. Tiny or extreme images (a 10×10 icon, a 12000×40 banner) must give a positive preview area and a valid crop → Task 3 test `tiny_image_min_is_whole_side`, Task 4 test `extreme_aspect_has_positive_preview`.
4. A ratio chip on bounds that can't hold the ratio at 16 px (9:16 on 100×20) must stay inside the bounds → Task 3 test `fit_ratio_in_narrow_bounds_stays_inside`.
5. JPEG scan data contains `FF 00` and restart markers, and EXIF may be big-endian; the stripper must copy the scan verbatim and still find orientation → Task 8 tests `scan_with_ff_bytes_is_copied_verbatim`, `big_endian_orientation_is_kept`.

---

### Task 1: The ✎편집 tile

**Files:**
- Modify: `crates/swisswheel-core/src/catalog.rs`
- Modify: `crates/swisswheel/src/wheel/glyph.rs`

**Interfaces:**
- Produces: `Tool::Edit` (label `"편집"`, `suffix()` → `Some("edited")`); `items_for(WheelKind::Tools, inputs)` appends `Operation::Tool(Tool::Edit)` last when `inputs` is exactly one `InputItem::File { format: Some(f), animated_gif: false }` with `f.kind() == MediaKind::Image` (an animated GIF never). `items_for_one` does not return `Edit`.

- [ ] **Step 1: Write the failing tests** (in `catalog.rs` tests)

```rust
#[test]
fn edit_only_for_one_still_image() {
    use Format::*;
    let tools = |inputs: &[InputItem]| items_for(WheelKind::Tools, inputs);
    assert_eq!(tools(&[file(Png)]).last(), Some(&Operation::Tool(Tool::Edit)));
    assert_eq!(tools(&[file(Bmp)]), [Tool::Shrink, Tool::Edit].map(Operation::Tool).to_vec());
    assert!(!tools(&[file(Png), file(Jpg)]).contains(&Operation::Tool(Tool::Edit)));
    let animated = InputItem::File { format: Some(Gif), animated_gif: true };
    assert!(tools(&[animated]).is_empty());
    assert!(!tools(&[file(Mp4)]).contains(&Operation::Tool(Tool::Edit)));
    assert!(tools(&[InputItem::Folder]).is_empty());
    assert_eq!(Tool::Edit.label(), "편집");
    assert_eq!(Tool::Edit.suffix(), Some("edited"));
}
```
In `glyph.rs` tests add `Tool::Edit` to `TOOLS` (length 6) and assert `glyph(&Operation::Tool(Tool::Edit)) == '\u{E70F}'`.

- [ ] **Step 2: Run** `cargo test -p swisswheel-core catalog` — Expected: FAIL (no `Tool::Edit`).
- [ ] **Step 3: Implement** `Tool::Edit` in `catalog.rs` and the `items_for` rule; add the glyph arm (`// Edit`). Fix every other exhaustive `match` on `Tool` the compiler reports (`Edit` is never an FFmpeg tool).
- [ ] **Step 4: Run** `cargo test -p swisswheel-core catalog` and `cargo test -p swisswheel glyph` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): edit tile for one still image`

---

### Task 2: Edit session

**Files:**
- Create: `crates/swisswheel-core/src/editor.rs` (add `pub mod editor;` to `lib.rs`)

**Interfaces:**
- Produces:
  - `pub enum Turn { Cw, Ccw }`, `pub enum Axis { Horizontal, Vertical }` (`Debug, Clone, Copy, PartialEq, Eq`).
  - `pub struct PixelRect { pub x: u32, pub y: u32, pub w: u32, pub h: u32 }` (`Copy, PartialEq, Debug`), `PixelRect::full((w, h))`.
  - `pub enum EditOp { Crop(PixelRect), Rotate(Turn), Flip(Axis), Grayscale(bool), StripInfo(bool) }` — a crop is in the coordinates of the result of the ops before it.
  - `pub enum SaveKind { Edited, StripOnly }`, `SaveKind::suffix(self) -> &'static str` (`"edited"` / `"no info"`).
  - `pub struct EditPlan { pub ops: Vec<EditOp>, pub kind: SaveKind }` (`Clone, Debug, PartialEq`).
  - `pub struct Affine { pub xx: f64, pub xy: f64, pub yx: f64, pub yy: f64, pub x0: f64, pub y0: f64 }` mapping `(x, y)` → `(xx·x + xy·y + x0, yx·x + yy·y + y0)`; `Affine::IDENTITY`, `apply(&self, x, y) -> (f64, f64)`, `then(&self, next: &Affine) -> Affine` (self first).
  - `pub struct EditSession` with `new(width: u32, height: u32, strip_available: bool)`, `push(&mut self, op: EditOp) -> bool`, `undo(&mut self) -> bool`, `redo(&mut self) -> bool`, `can_undo`, `can_redo`, `ops(&self) -> &[EditOp]`, `original_size`, `size(&self) -> (u32, u32)`, `grayscale`, `strip_info`, `strip_available`, `has_pixel_edits`, `save_kind(&self) -> Option<SaveKind>`, `plan(&self) -> Option<EditPlan>`, `to_result(&self) -> Affine`.
  - `pub mod text` with every copy constant from Global Constraints: `TITLE`, `LOADING`, `DISCARD`, `INFO_LOCKED`, `ALREADY_OPEN`, `OPEN_FAILED`, `CLOSE_FIRST`, `CANCEL`, `SAVE`, `CROP_CANCEL`, `CROP_APPLY`.

Rules: `push` returns false and records nothing for a toggle equal to the current value, a `StripInfo` when `!strip_available`, or a crop equal to `PixelRect::full(size())`; a recorded push clears redo. Rotations swap `size()`. `has_pixel_edits` = any crop/rotate/flip in `ops`, or `grayscale()`. `save_kind`: `Edited` if `has_pixel_edits`, else `StripOnly` if `strip_info()`, else `None`. Transform per op on a `(w, h)` result: Cw `(x, y) → (h − y, x)`; Ccw `(x, y) → (y, w − x)`; Flip H `(w − x, y)`; Flip V `(x, h − y)`; Crop `(x − r.x, y − r.y)`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn undo_redo_and_ignored_pushes() {
    let mut s = EditSession::new(40, 20, true);
    assert!(!s.push(EditOp::Grayscale(false)));
    assert!(!s.push(EditOp::Crop(PixelRect::full((40, 20)))));
    assert!(s.push(EditOp::Rotate(Turn::Cw)));
    assert_eq!(s.size(), (20, 40));
    assert!(s.undo() && s.can_redo());
    assert!(s.push(EditOp::Flip(Axis::Horizontal)));
    assert!(!s.can_redo());
    assert!(!EditSession::new(4, 4, false).push(EditOp::StripInfo(true)));
}

#[test]
fn rotate_crop_rotate_size_and_transform() {
    let mut s = EditSession::new(40, 20, true);
    s.push(EditOp::Rotate(Turn::Cw));                                   // 20×40
    s.push(EditOp::Crop(PixelRect { x: 5, y: 10, w: 10, h: 20 }));      // 10×20
    s.push(EditOp::Rotate(Turn::Ccw));                                  // 20×10
    assert_eq!(s.size(), (20, 10));
    // Original (10, 5): Cw → (15, 10); crop → (10, 0); Ccw on 10×20 → (0, 0).
    let (x, y) = s.to_result().apply(10.0, 5.0);
    assert!((x - 0.0).abs() < 1e-9 && (y - 0.0).abs() < 1e-9);
}

#[test]
fn save_kinds() {
    let mut s = EditSession::new(8, 8, true);
    assert_eq!(s.save_kind(), None);
    s.push(EditOp::Grayscale(true));
    s.push(EditOp::Grayscale(false));
    assert_eq!(s.save_kind(), None);
    s.push(EditOp::StripInfo(true));
    assert_eq!(s.save_kind(), Some(SaveKind::StripOnly));
    s.push(EditOp::Flip(Axis::Vertical));
    assert_eq!(s.plan().unwrap().kind, SaveKind::Edited);
    assert_eq!(SaveKind::StripOnly.suffix(), "no info");
}
```

- [ ] **Step 2: Run** `cargo test -p swisswheel-core editor::` — Expected: FAIL (module missing).
- [ ] **Step 3: Implement** `editor.rs` per Interfaces; `size()`, `grayscale()`, `strip_info()` and `to_result()` are folds over `ops()` (a redo stack holds undone ops).
- [ ] **Step 4: Run** `cargo test -p swisswheel-core editor::` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): edit session with undo and the result transform`

---

### Task 3: Crop math

**Files:**
- Create: `crates/swisswheel-core/src/crop.rs` (`pub mod crop;`)

**Interfaces:**
- Consumes: `editor::PixelRect`.
- Produces:
  - `pub enum CropHandle { TopLeft, Top, TopRight, Right, BottomRight, Bottom, BottomLeft, Left, Move }`.
  - `pub enum AspectRatio { Free, Original, Fixed(u32, u32) }`, `AspectRatio::value(self, bounds: (u32, u32)) -> Option<f64>` (w/h; `Original` = bounds ratio; `Free` = `None`).
  - `pub const CHIPS: [(AspectRatio, &str); 7]` in the Global Constraints order; `pub const MIN_CROP: u32 = 16`.
  - `pub fn min_side(bound: u32) -> u32` = `MIN_CROP.min(bound)`.
  - `pub fn drag(start: PixelRect, handle: CropHandle, dx: f64, dy: f64, bounds: (u32, u32), ratio: Option<f64>) -> PixelRect` — `dx, dy` are the total image-pixel movement since the press (not incremental).
  - `pub fn fit_ratio(crop: PixelRect, bounds: (u32, u32), ratio: f64) -> PixelRect` — the largest rect of `ratio` inside `bounds`, centered on `crop`'s center, shifted to stay inside.

Rules: `Move` translates and clamps inside `bounds`. Edge handles move one edge; with a ratio the other dimension follows, centered on the start rect's center line. Corner handles keep the opposite corner fixed; with a ratio the width comes from `dx` and height = width / ratio, then both shrink together if either exceeds the room toward the bounds. Results are always inside `bounds` and at least `min_side` on each axis; when a ratio can't also satisfy the minimum inside bounds, staying inside bounds wins.

- [ ] **Step 1: Write the failing tests**

```rust
const B: (u32, u32) = (200, 100);
fn r(x: u32, y: u32, w: u32, h: u32) -> PixelRect { PixelRect { x, y, w, h } }

#[test]
fn free_handles() {
    let s = r(50, 20, 100, 60);
    assert_eq!(drag(s, CropHandle::Right, 30.0, 99.0, B, None), r(50, 20, 130, 60));
    assert_eq!(drag(s, CropHandle::TopLeft, -10.0, -5.0, B, None), r(40, 15, 110, 65));
    assert_eq!(drag(s, CropHandle::Right, 500.0, 0.0, B, None), r(50, 20, 150, 60));
    assert_eq!(drag(s, CropHandle::Left, 200.0, 0.0, B, None), r(134, 20, 16, 60));
    assert_eq!(drag(s, CropHandle::Move, 500.0, -500.0, B, None), r(100, 0, 100, 60));
}

#[test]
fn ratio_is_kept() {
    let s = r(50, 20, 80, 60); // 4:3
    let out = drag(s, CropHandle::BottomRight, 40.0, 0.0, B, Some(4.0 / 3.0));
    assert_eq!((out.x, out.y), (50, 20));
    assert!((out.w as f64 / out.h as f64 - 4.0 / 3.0).abs() < 0.03);
    assert!(out.x + out.w <= 200 && out.y + out.h <= 100);
}

#[test]
fn fit_ratio_centers_and_stays_inside() {
    let out = fit_ratio(r(0, 0, 200, 100), B, 1.0);
    assert_eq!(out, r(50, 0, 100, 100));
    let edge = fit_ratio(r(180, 0, 20, 20), B, 1.0);
    assert!(edge.x + edge.w <= 200 && edge.w == edge.h);
}

#[test]
fn fit_ratio_in_narrow_bounds_stays_inside() {
    let out = fit_ratio(r(0, 0, 100, 20), (100, 20), 9.0 / 16.0);
    assert!(out.x + out.w <= 100 && out.y + out.h <= 20 && out.w >= 1 && out.h >= 1);
}

#[test]
fn tiny_image_min_is_whole_side() {
    assert_eq!(min_side(10), 10);
    let out = drag(r(0, 0, 10, 10), CropHandle::Right, -50.0, 0.0, (10, 10), None);
    assert_eq!(out, r(0, 0, 10, 10));
}

#[test]
fn chips_and_ratio_values() {
    assert_eq!(CHIPS.map(|c| c.1), ["자유", "원본", "1:1", "4:3", "3:4", "16:9", "9:16"]);
    assert_eq!(AspectRatio::Original.value((300, 100)), Some(3.0));
    assert_eq!(AspectRatio::Free.value((300, 100)), None);
}
```

- [ ] **Step 2: Run** `cargo test -p swisswheel-core crop::` — Expected: FAIL.
- [ ] **Step 3: Implement** `crop.rs` per Interfaces and Rules (work in `f64`, round to the nearest pixel at the end, then clamp).
- [ ] **Step 4: Run** `cargo test -p swisswheel-core crop::` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): crop handle and ratio math`

---

### Task 4: Panel layout, hit testing, fit

**Files:**
- Create: `crates/swisswheel-core/src/editor_geometry.rs` (`pub mod editor_geometry;`)

**Interfaces:**
- Consumes: `geometry::Rect`, `menu_geometry::Box2`, `editor::PixelRect`, `crop::{CropHandle, CHIPS}`.
- Produces:
  - DIP constants: `TOOLBAR = 48.0`, `CHIP_ROW = 40.0`, `FOOTER = 48.0`, `PAD = 12.0`, `CORNER = 16.0`, `SHADOW_MARGIN = 10.0`, `MIN_W = 720.0`, `MIN_H = 480.0`, `MAX_FRACTION = 0.8`, `TOOL_BUTTON = 76.0`, `CHIP = 56.0`, `FOOTER_BUTTON = 120.0`, `HANDLE_REACH = 12.0`.
  - `pub enum EditorTool { Crop, Rotate, FlipH, FlipV, Gray, Info }`, `pub const TOOLS: [EditorTool; 6]` in that order.
  - `pub enum EditorHit { Tool(EditorTool), Chip(usize), Undo, Redo, Cancel, Save, Preview, Nothing }`.
  - `pub struct EditorLayout { pub window: Rect /* screen px, includes SHADOW_MARGIN */, pub panel: Box2, pub tools: [Box2; 6], pub name: Box2, pub chips: [Box2; 7], pub preview: Box2, pub undo: Box2, pub redo: Box2, pub label: Box2, pub cancel: Box2, pub save: Box2 }` — boxes in window pixels.
  - `pub fn panel_layout(work_area: Rect, scale: f32, image: (u32, u32)) -> EditorLayout` — the chip row is always reserved (the preview doesn't jump when crop mode starts).
  - `pub fn hit_test(layout: &EditorLayout, cropping: bool, x: f32, y: f32) -> EditorHit` — chips only when `cropping`.
  - `pub struct Fit { pub scale: f32, pub left: f32, pub top: f32 }`, `Fit::new(area: Box2, size: (u32, u32)) -> Fit`, `to_image(&self, x: f32, y: f32) -> (f64, f64)`, `to_view(&self, r: PixelRect) -> Box2`.
  - `pub fn handle_at(fit: &Fit, crop: PixelRect, x: f32, y: f32, scale: f32) -> Option<CropHandle>` — corners before edges, within `HANDLE_REACH × scale` px; inside → `Move`.

Panel sizing: chrome height = `TOOLBAR + CHIP_ROW + FOOTER + 2·PAD`, chrome width = `2·PAD`; the largest preview of the image's aspect that fits `MAX_FRACTION` of the work area minus chrome; panel = preview + chrome, raised to `MIN_W × MIN_H` (capped at the work area); window = panel + `SHADOW_MARGIN` on every side, centered in `work_area`.

- [ ] **Step 1: Write the failing tests**

```rust
fn area() -> Rect { Rect { left: 0, top: 0, right: 1920, bottom: 1040 } }

#[test]
fn layout_fits_aspect_and_limits() {
    let l = panel_layout(area(), 1.0, (4000, 3000));
    let (pw, ph) = (l.panel.right - l.panel.left, l.panel.bottom - l.panel.top);
    assert!(pw <= 1920.0 * 0.8 + 0.5 && ph <= 1040.0 * 0.8 + 0.5);
    let cx = (l.window.left + l.window.right) / 2;
    assert!((cx - 960).abs() <= 1);
    let small = panel_layout(area(), 1.0, (50, 50));
    assert!(small.panel.right - small.panel.left >= 720.0 - 0.5);
    let hi = panel_layout(area(), 2.0, (4000, 3000));
    assert!(hi.window.right - hi.window.left <= 1920);
}

#[test]
fn extreme_aspect_has_positive_preview() {
    for size in [(12000, 40), (40, 12000), (1, 1)] {
        let l = panel_layout(area(), 1.5, size);
        assert!(l.preview.right > l.preview.left && l.preview.bottom > l.preview.top);
    }
}

#[test]
fn hits() {
    let l = panel_layout(area(), 1.0, (800, 600));
    let c = |b: Box2| ((b.left + b.right) / 2.0, (b.top + b.bottom) / 2.0);
    let (x, y) = c(l.tools[1]);
    assert_eq!(hit_test(&l, false, x, y), EditorHit::Tool(EditorTool::Rotate));
    let (x, y) = c(l.chips[2]);
    assert_eq!(hit_test(&l, false, x, y), EditorHit::Nothing);
    assert_eq!(hit_test(&l, true, x, y), EditorHit::Chip(2));
    let (x, y) = c(l.save);
    assert_eq!(hit_test(&l, false, x, y), EditorHit::Save);
}

#[test]
fn fit_round_trip_and_handles() {
    let area = Box2 { left: 10.0, top: 20.0, right: 410.0, bottom: 320.0 };
    let f = Fit::new(area, (800, 600));
    assert!((f.scale - 0.5).abs() < 1e-6);
    let (ix, iy) = f.to_image(210.0, 170.0);
    assert!((ix - 400.0).abs() < 1e-3 && (iy - 300.0).abs() < 1e-3);
    let crop = PixelRect { x: 100, y: 100, w: 200, h: 200 };
    let v = f.to_view(crop);
    assert_eq!(handle_at(&f, crop, v.left + 1.0, v.top + 1.0, 1.0), Some(CropHandle::TopLeft));
    assert_eq!(handle_at(&f, crop, (v.left + v.right) / 2.0, v.top, 1.0), Some(CropHandle::Top));
    assert_eq!(handle_at(&f, crop, (v.left + v.right) / 2.0, (v.top + v.bottom) / 2.0, 1.0), Some(CropHandle::Move));
    assert_eq!(handle_at(&f, crop, area.right, area.bottom, 1.0), None);
}
```

- [ ] **Step 2: Run** `cargo test -p swisswheel-core editor_geometry` — Expected: FAIL.
- [ ] **Step 3: Implement** `editor_geometry.rs`. Toolbar: tool buttons left to right from `panel.left + PAD`, `name` takes the rest of the row. Chips: left to right under the toolbar. Footer: `undo`, `redo` at the left, `save` at the right, `cancel` left of it, `label` between.
- [ ] **Step 4: Run** `cargo test -p swisswheel-core editor_geometry` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): editor panel layout, hits, and fit`

---

### Task 5: Editor controller

**Files:**
- Create: `crates/swisswheel-core/src/editor_control.rs` (`pub mod editor_control;`)

**Interfaces:**
- Consumes: Tasks 2–4.
- Produces:
  - `pub enum EditorCommand { Crop, RotateCw, RotateCcw, FlipH, FlipV, Gray, Info, Undo, Redo, Enter, Escape }`.
  - `pub fn key_command(vk: u16, shift: bool, ctrl: bool) -> Option<EditorCommand>` — `0x43` C, `0x52` R (Shift → Ccw), `0x48` H, `0x56` V, `0x47` G, `0x49` I, Ctrl+`0x5A` Z (Ctrl+Shift+Z → Redo), Ctrl+`0x59` Y, `0x0D` Enter, `0x1B` Escape; letters with Ctrl return `None` except Z/Y.
  - `pub struct CropMode { pub rect: PixelRect, pub chip: usize, drag: Option<(CropHandle, PixelRect, (f64, f64))> }` with `pub fn dragging(&self) -> Option<CropHandle>`.
  - `pub enum Outcome { Nothing, Redraw, Save(EditPlan), Close }`.
  - `pub struct EditorState` with `loading() -> Self`, `loaded(&mut self, width: u32, height: u32, strip_available: bool)`, `session(&self) -> Option<&EditSession>`, `crop(&self) -> Option<&CropMode>`, `command(&mut self, cmd: EditorCommand, now: Instant) -> Outcome`, `click(&mut self, hit: EditorHit, now: Instant) -> Outcome`, `crop_press(&mut self, handle: CropHandle, at: (f64, f64))`, `crop_move(&mut self, at: (f64, f64)) -> bool`, `crop_release(&mut self)`, `footer_text(&self, hover: EditorHit, now: Instant) -> Option<&'static str>`, `discard_deadline(&self) -> Option<Instant>`.

Rules:
- Loading: only `Escape` (→ `Close`) does anything.
- Normal mode: tool commands push the matching `EditOp` (`Gray`/`Info` toggle; `Info` ignored while `has_pixel_edits()`); `Crop` enters crop mode with `rect = PixelRect::full(size)` and chip 0; `Enter` → `Save(plan)` if any; `Escape` → `Close` when `save_kind()` is `None`, else starts the discard prompt; `Escape` again before the 3 s deadline → `Close`. Any other command or click clears the prompt.
- Crop mode: `Crop` / `Enter` push `Crop(rect)` (ignored if full) and leave; `Escape` leaves; other commands do nothing. A chip click sets `chip` and applies `fit_ratio` when the chip has a ratio. Drag uses `crop::drag` with the chip's ratio and the session `size()`.
- `click`: `Tool(t)` → that tool's command; `Undo`/`Redo`; `Save` → `Enter`; `Cancel` → `Escape`; `Chip(i)` as above; others `Nothing`.
- `footer_text`: `DISCARD` while the prompt is up; `INFO_LOCKED` when hovering `Tool(Info)` with pixel edits; else `None` (the renderer shows the size).

- [ ] **Step 1: Write the failing tests**

```rust
fn ready(w: u32, h: u32) -> EditorState { let mut s = EditorState::loading(); s.loaded(w, h, true); s }

#[test]
fn keys() {
    assert_eq!(key_command(0x52, true, false), Some(EditorCommand::RotateCcw));
    assert_eq!(key_command(0x5A, true, true), Some(EditorCommand::Redo));
    assert_eq!(key_command(0x43, false, true), None);
}

#[test]
fn loading_ignores_all_but_escape() {
    let t = Instant::now();
    let mut s = EditorState::loading();
    assert_eq!(s.command(EditorCommand::RotateCw, t), Outcome::Nothing);
    assert_eq!(s.command(EditorCommand::Escape, t), Outcome::Close);
}

#[test]
fn discard_needs_a_second_escape_within_three_seconds() {
    let t = Instant::now();
    let mut s = ready(100, 50);
    s.command(EditorCommand::RotateCw, t);
    assert_eq!(s.command(EditorCommand::Escape, t), Outcome::Redraw);
    assert_eq!(s.footer_text(EditorHit::Nothing, t), Some(text::DISCARD));
    assert_eq!(s.command(EditorCommand::Escape, t + Duration::from_secs(4)), Outcome::Redraw);
    assert_eq!(s.command(EditorCommand::Escape, t + Duration::from_secs(5)), Outcome::Close);
    let mut clean = ready(100, 50);
    assert_eq!(clean.command(EditorCommand::Escape, t), Outcome::Close);
}

#[test]
fn crop_mode_applies_and_cancels() {
    let t = Instant::now();
    let mut s = ready(200, 100);
    s.command(EditorCommand::Crop, t);
    assert_eq!(s.command(EditorCommand::RotateCw, t), Outcome::Nothing);
    s.crop_press(CropHandle::Right, (200.0, 50.0));
    assert!(s.crop_move((150.0, 50.0)));
    s.crop_release();
    assert_eq!(s.command(EditorCommand::Enter, t), Outcome::Redraw);
    assert_eq!(s.session().unwrap().size(), (150, 100));
    s.command(EditorCommand::Crop, t);
    s.click(EditorHit::Chip(2), t); // 1:1
    assert_eq!(s.crop().unwrap().rect.w, s.crop().unwrap().rect.h);
    s.click(EditorHit::Cancel, t);
    assert!(s.crop().is_none());
    assert_eq!(s.session().unwrap().size(), (150, 100));
}

#[test]
fn enter_saves_and_info_locks() {
    let t = Instant::now();
    let mut s = ready(10, 10);
    assert_eq!(s.command(EditorCommand::Enter, t), Outcome::Nothing);
    s.command(EditorCommand::Info, t);
    assert!(matches!(s.command(EditorCommand::Enter, t), Outcome::Save(EditPlan { kind: SaveKind::StripOnly, .. })));
    s.command(EditorCommand::Gray, t);
    assert_eq!(s.footer_text(EditorHit::Tool(EditorTool::Info), t), Some(text::INFO_LOCKED));
    assert_eq!(s.command(EditorCommand::Info, t), Outcome::Nothing);
}
```

- [ ] **Step 2: Run** `cargo test -p swisswheel-core editor_control` — Expected: FAIL.
- [ ] **Step 3: Implement** `editor_control.rs` per Rules.
- [ ] **Step 4: Run** `cargo test -p swisswheel-core` — Expected: PASS (whole core crate).
- [ ] **Step 5: Commit** `feat(core): editor controller for keys, clicks, crop, and discard`

---

### Task 6: Menu and tiles hold off while the editor is open

**Files:**
- Modify: `crates/swisswheel-core/src/drag.rs`, `crates/swisswheel-core/src/restart.rs`, `crates/swisswheel-core/src/update.rs`
- Modify: `crates/swisswheel/src/hook.rs`

**Interfaces:**
- Produces: `DragTracker::set_editor_open(&mut self, open: bool)` (blocks `menu_may_open`, i.e. desktop Space and the double Ctrl); `hook::set_editor_open(open: bool)`; `restart::on_tile(active_jobs: usize, background: bool, editor_open: bool)`; `update::on_tile(update_running: bool, active_jobs: usize, editor_open: bool)`. Order of refusals: busy jobs first, then editor (`editor::text::CLOSE_FIRST`), then the existing background/running reason.

- [ ] **Step 1: Write the failing tests**

```rust
// drag.rs
#[test]
fn editor_open_blocks_the_menu() {
    let mut t = tracker();
    t.set_editor_open(true);
    assert_eq!(t.on_event(DragEvent::DesktopKeyDown(SPACE)), pass());
    t.set_editor_open(false);
    assert_eq!(t.on_event(DragEvent::DesktopKeyDown(SPACE)).command, Some(DragCommand::OpenMenu));
}
// restart.rs
#[test]
fn tile_refuses_while_editor_open() {
    assert_eq!(on_tile(0, false, true), TileDecision::Refuse(crate::editor::text::CLOSE_FIRST));
    assert_eq!(on_tile(1, false, true), TileDecision::Refuse(update::text::BUSY));
}
// update.rs: same shape for on_tile(false, 0, true) and on_tile(false, 1, true).
```
Also add a double-Ctrl case to the drag test using the existing double-tap test helper in `drag.rs`.

- [ ] **Step 2: Run** `cargo test -p swisswheel-core` — Expected: FAIL to compile.
- [ ] **Step 3: Implement** the flag and parameters; update existing callers in `app.rs` (`restart::on_tile`, `release::on_tile`) to pass `false` for now (Task 12 passes the real value) and existing tests to the new arity.
- [ ] **Step 4: Run** `cargo test -p swisswheel-core` and `cargo build -p swisswheel` — Expected: PASS / builds.
- [ ] **Step 5: Commit** `feat(core): keep the menu, restart, and update away from an open editor`

---

### Task 7: Apply edits in the image engine

**Files:**
- Modify: `crates/swisswheel/src/engines/image.rs`

**Interfaces:**
- Consumes: `editor::{EditOp, Turn, Axis}`.
- Produces: `pub(crate) fn decode(input: &Path) -> Result<DynamicImage, EngineError>` (now `pub(crate)`); `pub(crate) fn dimensions(input: &Path) -> Result<(u32, u32), EngineError>` (header only, EXIF orientation applied: orientations 5–8 swap); `pub fn apply_edits(ops: &[EditOp], input: &Path, output: &Path) -> Result<(), EngineError>`.

`apply_edits`: decode, apply ops in order (`crop_imm`, `rotate90` for Cw, `rotate270` for Ccw, `fliph`/`flipv`, `StripInfo` ignored), then if the last `Grayscale` state is on, `grayscale()` (keeps alpha), then `encode(.., input format, HIGH_JPG_QUALITY, HIGH_WEBP_QUALITY, output)`. A crop outside the image is `EngineError::Failed`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn edits_rotate_then_crop_moves_marks() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("m.png");
    let mut img = ::image::RgbaImage::from_pixel(40, 20, ::image::Rgba([0, 0, 0, 255]));
    img.put_pixel(0, 0, ::image::Rgba([255, 0, 0, 255]));   // top-left red
    img.put_pixel(39, 19, ::image::Rgba([0, 255, 0, 255])); // bottom-right green
    img.save(&src).unwrap();
    let out = dir.path().join("o.png");
    let ops = [
        EditOp::Rotate(Turn::Cw),                                   // 20×40, red at (19, 0)
        EditOp::Crop(PixelRect { x: 10, y: 0, w: 10, h: 40 }),      // red at (9, 0)
        EditOp::Flip(Axis::Horizontal),                             // red at (0, 0)
    ];
    apply_edits(&ops, &src, &out).unwrap();
    let got = open(&out).to_rgba8();
    assert_eq!(got.dimensions(), (10, 40));
    assert_eq!(got.get_pixel(0, 0).0, [255, 0, 0, 255]);
}

#[test]
fn edits_grayscale_keeps_alpha_and_jpg_is_high_quality() {
    // rgba_png 8×8 alpha 128 → Grayscale(true) → every pixel has r == g == b and alpha 128.
    // gradient(256, 256) saved as JPG → Rotate(Cw) → mean_error vs the rotated source < the error at quality 75.
}
```
(`dimensions` is tested in Task 8, which adds the EXIF test helper.)
Write the two sketched tests in full with the helpers already in this test module (`rgba_png`, `gradient`, `mean_error`, `open`).

- [ ] **Step 2: Run** `cargo test -p swisswheel engines::image` — Expected: FAIL.
- [ ] **Step 3: Implement** per Interfaces.
- [ ] **Step 4: Run** `cargo test -p swisswheel engines::image` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(image): apply editor edits at full resolution`

---

### Task 8: Lossless info strip — JPG

**Files:**
- Create: `crates/swisswheel/src/engines/metadata.rs` (`pub mod metadata;` in `engines/mod.rs`)
- Modify: `crates/swisswheel/Cargo.toml` (`[dev-dependencies] crc32fast = "1"`, used by Task 9's PNG builder)

**Interfaces:**
- Produces: `pub fn strip(input: &Path, output: &Path) -> Result<(), EngineError>` (dispatches on content; this task: JPG), `fn strip_jpeg(bytes: &[u8]) -> Result<Vec<u8>, String>`, `fn exif_orientation(tiff: &[u8]) -> Option<u16>` (both byte orders), `fn orientation_app1(orientation: u16) -> Vec<u8>` (the full segment: `FF E1`, length, `Exif\0\0`, `II*\0`, IFD at 8, one entry `0x0112 SHORT 1 value`, next-IFD 0). Test helper `pub(crate) fn exif_app1(orientation: u16, big_endian: bool, gps: bool) -> Vec<u8>` returns the APP1 payload (for `add_app_segment(1, ..)`) with tag 0x0112 and, when `gps`, a GPS IFD pointer (0x8825) plus one GPS entry.

Kept / dropped segments and the orientation rule: spec 3.4. Segment walking stops at SOS; from SOS to the first `FF D9` that is not `FF 00`-stuffed or `FF D0–D7` is copied verbatim, then EOI; everything after EOI is dropped.

- [ ] **Step 1: Write the failing tests**

```rust
fn jpeg_with(app: &[(u8, Vec<u8>)]) -> Vec<u8> { /* jpeg-encoder 32×16 gradient, add_app_segment for each, quality 90 */ }
fn scan(bytes: &[u8]) -> &[u8] { /* from the SOS marker to the end */ }

#[test]
fn jpeg_exif_and_xmp_go_icc_and_scan_stay() {
    let icc = [b"ICC_PROFILE\0\x01\x01".as_slice(), &[7u8; 40]].concat();
    let xmp = [b"http://ns.adobe.com/xap/1.0/\0".as_slice(), b"<x/>"].concat();
    let src = jpeg_with(&[(1, exif_app1(1, false, true)), (1, xmp), (2, icc.clone())]);
    let mut with_tail = src.clone();
    with_tail.extend_from_slice(b"TRAILER");
    let out = strip_jpeg(&with_tail).unwrap();
    assert!(!out.windows(6).any(|w| w == b"Exif\0\0"));
    assert!(!out.windows(4).any(|w| w == b"xap/"));
    assert!(out.windows(icc.len()).any(|w| w == icc.as_slice()));
    assert_eq!(scan(&out), scan(&src));
    assert!(out.ends_with(&[0xFF, 0xD9]));
}

#[test]
fn rotated_photo_keeps_only_orientation() {
    let src = jpeg_with(&[(1, exif_app1(6, false, true))]);
    let out = strip_jpeg(&src).unwrap();
    let payload = /* the APP1 payload in out */;
    assert_eq!(exif_orientation(&payload[6..]), Some(6));
    assert!(!out.windows(2).any(|w| w == [0x88, 0x25]) && !out.windows(2).any(|w| w == [0x25, 0x88]));
}

#[test]
fn big_endian_orientation_is_kept() {
    let out = strip_jpeg(&jpeg_with(&[(1, exif_app1(8, true, false))])).unwrap();
    // decoding `out` with the image crate reports Orientation::Rotate270 (EXIF 8)
}

#[test]
fn scan_with_ff_bytes_is_copied_verbatim() {
    // jpeg_with on a noisy 64×64 image with restart interval (encoder.set_restart_interval(1)):
    // the scan contains FF 00 and FF D0–D7; scan(out) == scan(src).
}

#[test]
fn dimensions_follow_orientation() {
    // 40×20 JPG from jpeg_with(&[(1, exif_app1(6, false, false))]) written to a file
    // → crate::engines::image::dimensions(&path) == Ok((20, 40)).
}

#[test]
fn garbage_is_read_error() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("x.jpg");
    std::fs::write(&p, [0xFF, 0xD8, 0xFF, 0xE1, 0x00]).unwrap();
    assert!(matches!(strip(&p, &dir.path().join("o.jpg")), Err(EngineError::Read(_))));
}
```
Write the sketched bodies in full.

- [ ] **Step 2: Run** `cargo test -p swisswheel engines::metadata` — Expected: FAIL.
- [ ] **Step 3: Implement** the JPG path.
- [ ] **Step 4: Run** `cargo test -p swisswheel engines::metadata` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(metadata): strip JPG info without re-encoding`

---

### Task 9: Lossless info strip — PNG and WebP

**Files:**
- Modify: `crates/swisswheel/src/engines/metadata.rs`

**Interfaces:**
- Produces: `fn strip_png(bytes: &[u8]) -> Result<Vec<u8>, String>`, `fn strip_webp(bytes: &[u8]) -> Result<Vec<u8>, String>`; `strip` handles all three, and for PNG/WebP whose EXIF orientation (via the image crate decoder's `orientation()`) is not `NoTransforms` falls back to `decode` + `encode` (PNG lossless, WebP 92).

PNG drops `tEXt`, `zTXt`, `iTXt`, `eXIf`, `tIME`; WebP drops `EXIF` and `XMP ` chunks (with their pad byte), clears VP8X flag bits 0x08 (EXIF) and 0x04 (XMP), and rewrites the RIFF size. A simple (non-VP8X) WebP is copied unchanged.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn png_text_and_exif_go_idat_stays() {
    // Build: a PNG from PngEncoder, then insert tEXt("Comment","hi"), eXIf(exif tiff, orientation 1),
    // and iCCP-like chunk before IDAT (CRCs via crc32fast).
    // strip_png: no tEXt/eXIf chunk types, iCCP kept, IDAT bytes equal, decodes to the same pixels.
}

#[test]
fn webp_exif_goes_and_header_is_fixed() {
    // Build: webp::Encoder lossy 16×16 → wrap as RIFF/WEBP with VP8X (flags 0x08, canvas 15/15 as 24-bit minus one),
    // the VP8 chunk, and an EXIF chunk of odd length (pad byte).
    // strip_webp: no "EXIF" chunk, VP8X flags & 0x0C == 0, RIFF size == len - 8, image crate decodes it.
}

#[test]
fn rotated_png_falls_back_to_reencode() {
    // PNG with eXIf orientation 6 at 4×2 → strip() output decodes to 2×4 and has no eXIf chunk.
}
```
Write them in full.

- [ ] **Step 2: Run** `cargo test -p swisswheel engines::metadata` — Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p swisswheel engines::metadata` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(metadata): strip PNG and WebP info`

---

### Task 10: Jobs carry edits

**Files:**
- Modify: `crates/swisswheel/src/jobs.rs` (and every `Job { .. }` literal in the crate: add `edits: None`)

**Interfaces:**
- Consumes: `editor::{EditPlan, SaveKind}`, `engines::image::apply_edits`, `engines::metadata::strip`.
- Produces: `Job.edits: Option<EditPlan>`. In `process`, a job with `edits` uses the input's own extension, suffix `plan.kind.suffix()`, and runs `apply_edits(&plan.ops, ..)` for `Edited` or `metadata::strip` for `StripOnly` (a new `Engine::Edit(&EditPlan)` arm); it never touches FFmpeg.

- [ ] **Step 1: Write the failing test** (in `jobs.rs` tests, using the existing `runner()` helper)

```rust
#[test]
fn edit_jobs_name_their_outputs() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("사진.png");
    rgba_png(&src, 8, 4, 255);
    let (runner, rx) = runner();
    let edited = EditPlan { ops: vec![EditOp::Rotate(Turn::Cw)], kind: SaveKind::Edited };
    runner.submit(Job { op: Operation::Tool(Tool::Edit), preset: None, inputs: vec![src.clone()], edits: Some(edited) });
    let report = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(report.outputs, vec![dir.path().join("사진 (edited).png")]);
    assert_eq!(::image::image_dimensions(&report.outputs[0]).unwrap(), (4, 8));
    let strip = EditPlan { ops: vec![EditOp::StripInfo(true)], kind: SaveKind::StripOnly };
    runner.submit(Job { op: Operation::Tool(Tool::Edit), preset: None, inputs: vec![src.clone()], edits: Some(strip) });
    let report = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(report.outputs, vec![dir.path().join("사진 (no info).png")]);
    assert!(src.exists());
}
```

- [ ] **Step 2: Run** `cargo test -p swisswheel jobs::` — Expected: FAIL to compile.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p swisswheel jobs::` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(jobs): run editor saves`

---

### Task 11: Editor windows and drawing

**Files:**
- Create: `crates/swisswheel/src/editor/mod.rs`, `editor/window.rs`, `editor/render.rs` (`mod editor;` in `main.rs`)

**Interfaces:**
- Consumes: Tasks 2–5, `Surface`, `theme::Theme`, `fonts`, `engines::image::{decode, dimensions}`.
- Produces (used by Task 12):
  - `pub enum EditorEvent { Saved(PathBuf, EditPlan), Closed, Failed(String) }`.
  - `pub fn open(path: PathBuf, owner: HWND, notify: u32, theme: Theme) -> windows::core::Result<()>` — reads `dimensions` (an error here is returned as `Err` after logging and makes no window), creates the windows on the monitor under the cursor, shows "불러오는 중", starts the decode thread; `notify` is the message posted to `owner` whenever an event is queued.
  - `pub fn is_open() -> bool`, `pub fn focus()`, `pub fn close()` (drops windows without an event), `pub fn take_events() -> Vec<EditorEvent>`.

State lives in a `thread_local! RefCell<Option<Editor>>` inside `editor/mod.rs` (the UI thread owns it; `app.rs` keeps none of it).

`window.rs`:
- Dim window per Global Constraints over the monitor rect; returns `MA_NOACTIVATE` to `WM_MOUSEACTIVATE`. Panel: `WS_POPUP`, `WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP`, owner = dim window, rect = `layout.window`, `Surface::new(hwnd, w, h, 1)` with glass at `layout.panel`, `CORNER × scale`.
- Focus: `SetForegroundWindow(panel)`; if `GetForegroundWindow() != panel`, `AttachThreadInput` to the foreground thread, `SetForegroundWindow`, `SetFocus`, detach.
- Messages: `WM_KEYDOWN` → `key_command(vk, GetKeyState(VK_SHIFT) < 0, GetKeyState(VK_CONTROL) < 0)`; `WM_CLOSE` → `Escape`; `WM_LBUTTONDOWN` → in crop mode on the preview `handle_at` → `crop_press` + `SetCapture`, else remember the hit; `WM_MOUSEMOVE` → `crop_move` or hover; `WM_LBUTTONUP` → `crop_release` / `click(hit)` when the up hit equals the down hit; `WM_SETCURSOR` → `IDC_SIZENWSE`/`IDC_SIZENESW`/`IDC_SIZEWE`/`IDC_SIZENS`/`IDC_SIZEALL` for handles, `IDC_ARROW` else; `WM_TIMER 1` → redraw (the discard prompt expires; set it for 3 s whenever `discard_deadline()` is `Some`); `WM_APP_EDITOR_LOADED` → take the decoded result from the slot.
- Mouse points convert to image coordinates through `Fit::new(layout.preview, session.size())`.

`mod.rs`:
- Decode thread: `decode(path)` → `(width, height, color: RgbaImage, gray: RgbaImage)` where the previews are resized (`imageops::resize`, `Triangle`) so the longer side equals `max(preview width, preview height)` in px (never upscaled); stored in an `Arc<Mutex<Option<Result<.., String>>>>` slot, then `PostMessageW(panel, WM_APP_EDITOR_LOADED)`. On `Err`: close windows, queue `Failed(EngineError::Read(..).user_message(name))`.
- `Outcome::Save(plan)` → close windows, queue `Saved(path, plan)`; `Outcome::Close` → close, queue `Closed`. Queue = `Vec<EditorEvent>` + `PostMessageW(owner, notify)`.
- On `D2DERR_RECREATE_TARGET` from present: recreate `Surface` and both bitmaps once; if that fails, close and queue `Failed(text::OPEN_FAILED)`.

`render.rs` — `pub fn draw(surface: &Surface, theme: &Theme, layout: &EditorLayout, scale: f32, state: &EditorState, bitmaps: Option<&Previews>, name: &str, hover: EditorHit, now: Instant) -> Result<()>`:
- Glass panel, outer shadow, rim as the menu panel does (`theme::glass_layer`, `outer_shadow`, `rounded_shape`).
- Toolbar: six buttons (icon from the system icon font + Korean label `자르기`, `회전`, `좌우`, `상하`, `흑백`, `정보`); hover highlight; `Gray`/`Info` filled with the accent when on (`Info` shown on and dimmed when locked); `Info` hidden when `!strip_available()`; all but `Crop` dimmed in crop mode and while loading. Pick icon code points present in both Segoe Fluent Icons and Segoe MDL2 Assets. File name right-aligned, trimmed.
- Chips row only in crop mode; the selected chip filled with the accent.
- Preview: premultiplied BGRA bitmaps made once from the RGBA previews; world transform = scale-to-original (`original / preview` size) → `to_result()` → `Fit` (`D2D_MATRIX_3X2_F { M11: xx, M12: yx, M21: xy, M22: yy, M31: x0, M32: y0 }`), clipped to `fit.to_view(PixelRect::full(size))`, linear interpolation; the gray bitmap when `grayscale()`. While loading: `text::LOADING` centered.
- Crop overlay: black 50% outside the crop rect, white 1 px border, thirds lines at 40%, 8 handles (squares of `8 × scale` px).
- Footer: undo/redo icons (dimmed when unavailable or cropping), center text = `footer_text(..)` or `"{w}×{h}"` / `"{w}×{h} → {w2}×{h2}"` (crop mode: the crop rect's size), buttons `CANCEL`/`SAVE` (crop mode `CROP_CANCEL`/`CROP_APPLY`), save dimmed when `save_kind()` is `None` outside crop mode.

- [ ] **Step 1: Build** `cargo build -p swisswheel` and `cargo clippy -p swisswheel --all-targets -- -D warnings` — Expected: clean.
- [ ] **Step 2: Run the whole suite** `cargo test --workspace` — Expected: PASS.
- [ ] **Step 3: Commit** `feat(editor): floating panel windows and drawing`

(Seen on screen in Task 12, once a preview can open it.)

---

### Task 12: Wire the editor into the app

**Files:**
- Modify: `crates/swisswheel/src/app.rs`

**Interfaces:**
- Consumes: Task 11's `editor::{open, is_open, focus, close, take_events, EditorEvent}`, Task 6's `hook::set_editor_open` and tile arities, Task 10's `Job.edits`.

Changes:
- `const WM_APP_EDITOR_OPEN: u32 = WM_APP + 38; const WM_APP_EDITOR: u32 = WM_APP + 39;` with doc comments like their neighbours.
- `submit`: `Operation::Tool(Tool::Edit)` → if `editor::is_open()`: `editor::focus()` and `notify(info(text::TITLE, text::ALREADY_OPEN))`; else store the single input in `self.edit_pending: Option<PathBuf>` and `PostMessageW(self.hwnd, WM_APP_EDITOR_OPEN, ..)`. Never open inside `submit` (it runs inside OLE `Drop`).
- `WM_APP_EDITOR_OPEN` → `editor::open(path, hwnd, WM_APP_EDITOR, Theme::current(..))`; `Err` → `notify(warning_notice(text::TITLE, text::OPEN_FAILED))`; `Ok` → `hook::set_editor_open(true)`.
- `WM_APP_EDITOR` → for each `take_events()`: `Saved(path, plan)` → `runner.submit(Job { op: Operation::Tool(Tool::Edit), preset: None, inputs: vec![path], edits: Some(plan) })`, `active_jobs += 1`, `update_busy()`; `Failed(msg)` → warning card; `Closed` → nothing. Then `hook::set_editor_open(editor::is_open())`.
- `restart()` and `start_update()` pass `editor::is_open()`.
- Quit path: `editor::close()` before the runner shuts down.
- `preview("editor")` (debug): write a 1600×1000 gradient PNG with colored corner marks to `%TEMP%\swisswheel-editor-preview.png`, then post `WM_APP_EDITOR_OPEN` for it; add `editor` to the doc comment's list.

- [ ] **Step 1: Build and test** `cargo build -p swisswheel`, `cargo clippy -p swisswheel --all-targets -- -D warnings`, `cargo test --workspace` — Expected: clean / PASS.
- [ ] **Step 2: Screen check** (skip if a full-screen game is foreground; report it): `SWISSWHEEL_PREVIEW=editor cargo run -p swisswheel`, screenshot the panel; then press C and drag a corner, press R, G, Ctrl+Z, Esc twice. Expected: panel centered with dimmed background, keys work without clicking first, crop overlay follows the mouse, Esc twice closes.
- [ ] **Step 3: Commit** `feat(app): open the editor from the wheel and save through jobs`

---

### Task 13: Docs

**Files:**
- Modify: `README.md`, `docs/manual-test.md`

- [ ] **Step 1:** README — "What works now": an image editor bullet (crop with ratios, rotate, flip, grayscale, lossless info strip, undo/redo, saves `name (edited).ext`); Usage: the ✎편집 tile for one still image and the key list from Global Constraints.
- [ ] **Step 2:** `docs/manual-test.md` — add the spec 5.4 checks, plus: focus lands in the panel after dropping from Explorer (type R without clicking), a 50 MP photo shows 불러오는 중, a phone portrait photo with EXIF orientation 6 opens upright and its `(no info)` copy stays upright in Explorer.
- [ ] **Step 3: Commit** `docs: image editor in README and manual tests`
