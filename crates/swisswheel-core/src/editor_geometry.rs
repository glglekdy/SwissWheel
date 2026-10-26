//! Editor panel layouts (image and video): sizes, button boxes, hit testing, the
//! image-to-view fit and the video timeline.
//!
//! Constants are in DIPs (scale 1.0); callers pass the monitor's DPI scale. Boxes are in
//! physical pixels relative to the window's top-left; the window itself is in screen pixels.

use crate::crop::CropHandle;
use crate::editor::PixelRect;
use crate::geometry::Rect;
use crate::menu_geometry::Box2;

pub const TOOLBAR: f32 = 48.0;
pub const CHIP_ROW: f32 = 40.0;
pub const FOOTER: f32 = 48.0;
pub const PAD: f32 = 12.0;
pub const CORNER: f32 = 16.0;
/// Room around the panel for its shadow; the window is this much larger on every side.
pub const SHADOW_MARGIN: f32 = 10.0;
pub const MIN_W: f32 = 720.0;
pub const MIN_H: f32 = 480.0;
/// The panel is at most this share of the work area.
pub const MAX_FRACTION: f32 = 0.8;
pub const TOOL_BUTTON: f32 = 76.0;
pub const CHIP: f32 = 56.0;
pub const FOOTER_BUTTON: f32 = 120.0;
/// How far from a crop edge or corner a grab still counts.
pub const HANDLE_REACH: f32 = 12.0;
/// The video panel's row between the preview and the footer.
pub const TIMELINE_ROW: f32 = 44.0;
pub const PLAY_BUTTON: f32 = 44.0;
pub const TIME_TEXT: f32 = 140.0;
pub const PIECE_BUTTON: f32 = 96.0;

/// Gap between neighbouring buttons.
const GAP: f32 = 8.0;
/// Width of the undo and redo buttons.
const ICON_BUTTON: f32 = 56.0;
/// Height of the timeline bar, centered in its row.
const BAR: f32 = 16.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorTool {
    Crop,
    Rotate,
    FlipH,
    FlipV,
    Gray,
    Info,
}

pub const TOOLS: [EditorTool; 6] = [
    EditorTool::Crop,
    EditorTool::Rotate,
    EditorTool::FlipH,
    EditorTool::FlipV,
    EditorTool::Gray,
    EditorTool::Info,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorHit {
    Tool(EditorTool),
    Chip(usize),
    Undo,
    Redo,
    Cancel,
    Save,
    Preview,
    Nothing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoTool {
    Crop,
    Rotate,
    FlipH,
    FlipV,
    Speed,
    Audio,
    Reverse,
    Frame,
}

pub const VIDEO_TOOLS: [VideoTool; 8] = [
    VideoTool::Crop,
    VideoTool::Rotate,
    VideoTool::FlipH,
    VideoTool::FlipV,
    VideoTool::Speed,
    VideoTool::Audio,
    VideoTool::Reverse,
    VideoTool::Frame,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoHit {
    Tool(VideoTool),
    Chip(usize),
    Play,
    Bar,
    Split,
    Toggle,
    Undo,
    Redo,
    Cancel,
    Save,
    Preview,
    Nothing,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EditorLayout {
    /// Screen pixels, includes `SHADOW_MARGIN` on every side.
    pub window: Rect,
    pub panel: Box2,
    pub tools: [Box2; 6],
    /// The rest of the toolbar row: the file name.
    pub name: Box2,
    pub chips: [Box2; 7],
    pub preview: Box2,
    pub undo: Box2,
    pub redo: Box2,
    /// Between redo and cancel: the status text.
    pub label: Box2,
    pub cancel: Box2,
    pub save: Box2,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoLayout {
    /// Screen pixels, includes `SHADOW_MARGIN` on every side.
    pub window: Rect,
    pub panel: Box2,
    pub tools: [Box2; 8],
    /// The rest of the toolbar row: the file name.
    pub name: Box2,
    pub chips: [Box2; 7],
    pub preview: Box2,
    pub play: Box2,
    /// The playback time text.
    pub time: Box2,
    /// The timeline bar.
    pub bar: Box2,
    pub split: Box2,
    /// Remove or restore the current piece.
    pub toggle: Box2,
    pub undo: Box2,
    pub redo: Box2,
    /// Between redo and cancel: the status text.
    pub label: Box2,
    pub cancel: Box2,
    pub save: Box2,
}

fn bx(left: f32, top: f32, right: f32, bottom: f32) -> Box2 {
    Box2 {
        left,
        top,
        right,
        bottom,
    }
}

/// The parts every editor panel shares: window, toolbar, chip row and footer. The space
/// between the chip row and the footer (`body_top`..`foot_top`) is the caller's.
struct Chrome<const N: usize> {
    window: Rect,
    panel: Box2,
    tools: [Box2; N],
    name: Box2,
    chips: [Box2; 7],
    body_top: f32,
    foot_top: f32,
    undo: Box2,
    redo: Box2,
    label: Box2,
    cancel: Box2,
    save: Box2,
}

/// The panel for `media` centered in `work_area`, with `extra` DIPs of rows below the
/// preview. The chip row is always reserved, so the preview does not jump when crop mode
/// starts.
fn chrome<const N: usize>(work_area: Rect, scale: f32, media: (u32, u32), extra: f32) -> Chrome<N> {
    let s = scale;
    let work_w = (work_area.right - work_area.left) as f32;
    let work_h = (work_area.bottom - work_area.top) as f32;
    let margin = SHADOW_MARGIN * s;
    let chrome_w = 2.0 * PAD * s;
    let chrome_h = (TOOLBAR + CHIP_ROW + FOOTER + 2.0 * PAD + extra) * s;

    let avail_w = (work_w * MAX_FRACTION - chrome_w).max(1.0);
    let avail_h = (work_h * MAX_FRACTION - chrome_h).max(1.0);
    let (iw, ih) = (media.0.max(1) as f32, media.1.max(1) as f32);
    let fit = (avail_w / iw).min(avail_h / ih);
    let (pre_w, pre_h) = ((iw * fit).max(1.0), (ih * fit).max(1.0));

    // The window must fit the work area, shadow included.
    let cap_w = (work_w - 2.0 * margin).max(1.0);
    let cap_h = (work_h - 2.0 * margin).max(1.0);
    let pw = (pre_w + chrome_w).max(MIN_W * s).min(cap_w).round();
    let ph = (pre_h + chrome_h).max(MIN_H * s).min(cap_h).round();

    let m = margin.round();
    let (win_w, win_h) = (pw + 2.0 * m, ph + 2.0 * m);
    let left = work_area.left + ((work_w - win_w) / 2.0).round() as i32;
    let top = work_area.top + ((work_h - win_h) / 2.0).round() as i32;
    let window = Rect {
        left,
        top,
        right: left + win_w as i32,
        bottom: top + win_h as i32,
    };

    let pad = PAD * s;
    let gap = GAP * s;
    let (pl, pt, pr, pb) = (m, m, m + pw, m + ph);
    let panel = bx(pl, pt, pr, pb);

    // Toolbar.
    let (tb_top, tb_bottom) = (pt + 4.0 * s, pt + (TOOLBAR - 4.0) * s);
    let mut x = pl + pad;
    let tools = std::array::from_fn(|_| {
        let b = bx(x, tb_top, x + TOOL_BUTTON * s, tb_bottom);
        x += (TOOL_BUTTON + GAP) * s;
        b
    });
    let name = bx(x + pad - gap, tb_top, (pr - pad).max(x), tb_bottom);

    // Chip row.
    let chip_top = pt + TOOLBAR * s;
    let chip_bottom = pt + (TOOLBAR + CHIP_ROW - 4.0) * s;
    let mut x = pl + pad;
    let chips = std::array::from_fn(|_| {
        let b = bx(x, chip_top, x + CHIP * s, chip_bottom);
        x += (CHIP + GAP) * s;
        b
    });

    // Footer.
    let foot_top = pb - FOOTER * s;
    let (btn_top, btn_bottom) = (foot_top + 8.0 * s, pb - 8.0 * s);
    let save = bx(pr - pad - FOOTER_BUTTON * s, btn_top, pr - pad, btn_bottom);
    let cancel = bx(
        save.left - gap - FOOTER_BUTTON * s,
        btn_top,
        save.left - gap,
        btn_bottom,
    );
    let undo = bx(pl + pad, btn_top, pl + pad + ICON_BUTTON * s, btn_bottom);
    let redo = bx(
        undo.right + gap,
        btn_top,
        undo.right + gap + ICON_BUTTON * s,
        btn_bottom,
    );
    let label = bx(
        redo.right + gap,
        btn_top,
        (cancel.left - gap).max(redo.right + gap),
        btn_bottom,
    );

    Chrome {
        window,
        panel,
        tools,
        name,
        chips,
        body_top: pt + (TOOLBAR + CHIP_ROW + PAD) * s,
        foot_top,
        undo,
        redo,
        label,
        cancel,
        save,
    }
}

/// The editor window for an image, centered in `work_area`.
pub fn panel_layout(work_area: Rect, scale: f32, image: (u32, u32)) -> EditorLayout {
    let c = chrome(work_area, scale, image, 0.0);
    let pad = PAD * scale;
    EditorLayout {
        window: c.window,
        panel: c.panel,
        tools: c.tools,
        name: c.name,
        chips: c.chips,
        preview: bx(
            c.panel.left + pad,
            c.body_top,
            c.panel.right - pad,
            c.foot_top - pad,
        ),
        undo: c.undo,
        redo: c.redo,
        label: c.label,
        cancel: c.cancel,
        save: c.save,
    }
}

/// The editor window for a video of display size `video`: the image panel's rows plus the
/// timeline row (play, time, bar, split and remove) between the preview and the footer.
pub fn video_layout(work_area: Rect, scale: f32, video: (u32, u32)) -> VideoLayout {
    let s = scale;
    let c = chrome(work_area, s, video, TIMELINE_ROW);
    let (pad, gap) = (PAD * s, GAP * s);
    let (pl, pr) = (c.panel.left, c.panel.right);

    let row_top = c.foot_top - TIMELINE_ROW * s;
    let (btn_top, btn_bottom) = (row_top + 4.0 * s, c.foot_top - 4.0 * s);
    let play = bx(pl + pad, btn_top, pl + pad + PLAY_BUTTON * s, btn_bottom);
    let time = bx(
        play.right + gap,
        btn_top,
        play.right + gap + TIME_TEXT * s,
        btn_bottom,
    );
    let toggle = bx(pr - pad - PIECE_BUTTON * s, btn_top, pr - pad, btn_bottom);
    let split = bx(
        toggle.left - gap - PIECE_BUTTON * s,
        btn_top,
        toggle.left - gap,
        btn_bottom,
    );
    let bar_top = row_top + (TIMELINE_ROW - BAR) / 2.0 * s;
    let bar = bx(
        time.right + gap,
        bar_top,
        (split.left - gap).max(time.right + gap),
        bar_top + BAR * s,
    );

    VideoLayout {
        window: c.window,
        panel: c.panel,
        tools: c.tools,
        name: c.name,
        chips: c.chips,
        preview: bx(pl + pad, c.body_top, pr - pad, row_top - pad),
        play,
        time,
        bar,
        split,
        toggle,
        undo: c.undo,
        redo: c.redo,
        label: c.label,
        cancel: c.cancel,
        save: c.save,
    }
}

/// What is under `(x, y)` in window pixels. Chips react only while `cropping`.
pub fn hit_test(layout: &EditorLayout, cropping: bool, x: f32, y: f32) -> EditorHit {
    if let Some(i) = layout.tools.iter().position(|b| b.contains(x, y)) {
        return EditorHit::Tool(TOOLS[i]);
    }
    if cropping && let Some(i) = layout.chips.iter().position(|b| b.contains(x, y)) {
        return EditorHit::Chip(i);
    }
    let buttons = [
        (layout.undo, EditorHit::Undo),
        (layout.redo, EditorHit::Redo),
        (layout.cancel, EditorHit::Cancel),
        (layout.save, EditorHit::Save),
        (layout.preview, EditorHit::Preview),
    ];
    for (b, hit) in buttons {
        if b.contains(x, y) {
            return hit;
        }
    }
    EditorHit::Nothing
}

/// What is under `(x, y)` in the video panel. Chips react only while `cropping`; the bar
/// reaches [`HANDLE_REACH`] above and below.
pub fn video_hit_test(layout: &VideoLayout, cropping: bool, x: f32, y: f32) -> VideoHit {
    if let Some(i) = layout.tools.iter().position(|b| b.contains(x, y)) {
        return VideoHit::Tool(VIDEO_TOOLS[i]);
    }
    if cropping && let Some(i) = layout.chips.iter().position(|b| b.contains(x, y)) {
        return VideoHit::Chip(i);
    }
    // The bar is `BAR` DIPs tall, so its height over `BAR` is the DPI scale.
    let reach = HANDLE_REACH * (layout.bar.bottom - layout.bar.top) / BAR;
    let bar = Box2 {
        top: layout.bar.top - reach,
        bottom: layout.bar.bottom + reach,
        ..layout.bar
    };
    let buttons = [
        (layout.play, VideoHit::Play),
        (layout.split, VideoHit::Split),
        (layout.toggle, VideoHit::Toggle),
        (layout.undo, VideoHit::Undo),
        (layout.redo, VideoHit::Redo),
        (layout.cancel, VideoHit::Cancel),
        (layout.save, VideoHit::Save),
        (bar, VideoHit::Bar),
        (layout.preview, VideoHit::Preview),
    ];
    for (b, hit) in buttons {
        if b.contains(x, y) {
            return hit;
        }
    }
    VideoHit::Nothing
}

/// The timeline bar as a map between x and seconds of the original.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Timeline {
    pub left: f32,
    pub right: f32,
    pub duration: f64,
}

impl Timeline {
    pub fn from_bar(bar: Box2, duration: f64) -> Timeline {
        Timeline {
            left: bar.left,
            right: bar.right,
            duration,
        }
    }

    /// The time under `x`; outside the bar, its nearer end.
    pub fn time_at(&self, x: f32) -> f64 {
        let width = self.right - self.left;
        if width <= 0.0 {
            return 0.0;
        }
        (f64::from(x - self.left) / f64::from(width) * self.duration).clamp(0.0, self.duration)
    }

    /// Where `t` sits on the bar.
    pub fn x_of(&self, t: f64) -> f32 {
        if self.duration <= 0.0 {
            return self.left;
        }
        self.left + ((t / self.duration) as f32) * (self.right - self.left)
    }
}

/// Where an image sits in a view box: the largest scale that fits, centered.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    /// View pixels per image pixel.
    pub scale: f32,
    pub left: f32,
    pub top: f32,
}

impl Fit {
    pub fn new(area: Box2, size: (u32, u32)) -> Fit {
        let (iw, ih) = (size.0.max(1) as f32, size.1.max(1) as f32);
        let (aw, ah) = (area.right - area.left, area.bottom - area.top);
        let scale = (aw / iw).min(ah / ih).max(f32::MIN_POSITIVE);
        Fit {
            scale,
            left: area.left + (aw - iw * scale) / 2.0,
            top: area.top + (ah - ih * scale) / 2.0,
        }
    }

    /// The image pixel position under a view point (may lie outside the image).
    pub fn to_image(&self, x: f32, y: f32) -> (f64, f64) {
        (
            f64::from(x - self.left) / f64::from(self.scale),
            f64::from(y - self.top) / f64::from(self.scale),
        )
    }

    /// An image rectangle in view coordinates.
    pub fn to_view(&self, r: PixelRect) -> Box2 {
        let left = self.left + r.x as f32 * self.scale;
        let top = self.top + r.y as f32 * self.scale;
        Box2 {
            left,
            top,
            right: left + r.w as f32 * self.scale,
            bottom: top + r.h as f32 * self.scale,
        }
    }
}

/// The crop handle under a view point: corners first, then edges, then the inside (`Move`).
///
/// An edge reaches [`HANDLE_REACH`] outward but at most a third of the
/// crop's size inward, so the two sides of an axis never overlap: on a
/// small crop the nearer side wins and the middle third still moves.
pub fn handle_at(fit: &Fit, crop: PixelRect, x: f32, y: f32, scale: f32) -> Option<CropHandle> {
    let v = fit.to_view(crop);
    let reach = HANDLE_REACH * scale;
    let inward_x = reach.min((v.right - v.left) / 3.0);
    let inward_y = reach.min((v.bottom - v.top) / 3.0);
    let (nl, nr) = (
        x >= v.left - reach && x <= v.left + inward_x,
        x <= v.right + reach && x >= v.right - inward_x,
    );
    let (nt, nb) = (
        y >= v.top - reach && y <= v.top + inward_y,
        y <= v.bottom + reach && y >= v.bottom - inward_y,
    );
    match (nl, nr, nt, nb) {
        (true, _, true, _) => return Some(CropHandle::TopLeft),
        (_, true, true, _) => return Some(CropHandle::TopRight),
        (_, true, _, true) => return Some(CropHandle::BottomRight),
        (true, _, _, true) => return Some(CropHandle::BottomLeft),
        _ => {}
    }
    let in_x = x >= v.left && x <= v.right;
    let in_y = y >= v.top && y <= v.bottom;
    if nt && in_x {
        return Some(CropHandle::Top);
    }
    if nb && in_x {
        return Some(CropHandle::Bottom);
    }
    if nl && in_y {
        return Some(CropHandle::Left);
    }
    if nr && in_y {
        return Some(CropHandle::Right);
    }
    (x > v.left && x < v.right && y > v.top && y < v.bottom).then_some(CropHandle::Move)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> Rect {
        Rect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        }
    }

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
        assert_eq!(
            hit_test(&l, false, x, y),
            EditorHit::Tool(EditorTool::Rotate)
        );
        let (x, y) = c(l.chips[2]);
        assert_eq!(hit_test(&l, false, x, y), EditorHit::Nothing);
        assert_eq!(hit_test(&l, true, x, y), EditorHit::Chip(2));
        let (x, y) = c(l.save);
        assert_eq!(hit_test(&l, false, x, y), EditorHit::Save);
    }

    #[test]
    fn fit_round_trip_and_handles() {
        let area = Box2 {
            left: 10.0,
            top: 20.0,
            right: 410.0,
            bottom: 320.0,
        };
        let f = Fit::new(area, (800, 600));
        assert!((f.scale - 0.5).abs() < 1e-6);
        let (ix, iy) = f.to_image(210.0, 170.0);
        assert!((ix - 400.0).abs() < 1e-3 && (iy - 300.0).abs() < 1e-3);
        let crop = PixelRect {
            x: 100,
            y: 100,
            w: 200,
            h: 200,
        };
        let v = f.to_view(crop);
        let mid_x = (v.left + v.right) / 2.0;
        let mid_y = (v.top + v.bottom) / 2.0;
        assert_eq!(
            handle_at(&f, crop, v.left + 1.0, v.top + 1.0, 1.0),
            Some(CropHandle::TopLeft)
        );
        assert_eq!(
            handle_at(&f, crop, mid_x, v.top, 1.0),
            Some(CropHandle::Top)
        );
        assert_eq!(
            handle_at(&f, crop, mid_x, mid_y, 1.0),
            Some(CropHandle::Move)
        );
        assert_eq!(handle_at(&f, crop, area.right, area.bottom, 1.0), None);
    }

    #[test]
    fn every_handle_reachable_on_a_tiny_crop() {
        let area = Box2 {
            left: 10.0,
            top: 20.0,
            right: 410.0,
            bottom: 320.0,
        };
        let f = Fit::new(area, (800, 600)); // 0.5 view px per image px
        let crop = PixelRect {
            x: 100,
            y: 100,
            w: 20,
            h: 20,
        };
        let v = f.to_view(crop);
        assert!((v.right - v.left - 10.0).abs() < 1e-3, "a 10×10 view rect");
        let mid_x = (v.left + v.right) / 2.0;
        let mid_y = (v.top + v.bottom) / 2.0;
        let at = |x: f32, y: f32| handle_at(&f, crop, x, y, 1.0);
        // Outside the rect: the nearer side wins.
        assert_eq!(at(v.left - 2.0, v.top - 2.0), Some(CropHandle::TopLeft));
        assert_eq!(at(v.right + 2.0, v.top - 2.0), Some(CropHandle::TopRight));
        assert_eq!(
            at(v.right + 2.0, v.bottom + 2.0),
            Some(CropHandle::BottomRight)
        );
        assert_eq!(
            at(v.left - 2.0, v.bottom + 2.0),
            Some(CropHandle::BottomLeft)
        );
        assert_eq!(at(v.right + 2.0, mid_y), Some(CropHandle::Right));
        assert_eq!(at(mid_x, v.bottom + 2.0), Some(CropHandle::Bottom));
        assert_eq!(at(v.left - 2.0, mid_y), Some(CropHandle::Left));
        assert_eq!(at(mid_x, v.top - 2.0), Some(CropHandle::Top));
        // Inside: the outer thirds belong to the edges, the center moves.
        assert_eq!(at(v.left + 1.0, v.top + 1.0), Some(CropHandle::TopLeft));
        assert_eq!(
            at(v.right - 1.0, v.bottom - 1.0),
            Some(CropHandle::BottomRight)
        );
        assert_eq!(at(v.right - 1.0, mid_y), Some(CropHandle::Right));
        assert_eq!(at(mid_x, v.bottom - 1.0), Some(CropHandle::Bottom));
        assert_eq!(at(mid_x, mid_y), Some(CropHandle::Move));
        assert_eq!(at(v.right + HANDLE_REACH + 1.0, mid_y), None);
    }

    #[test]
    fn video_layout_rows() {
        let work = Rect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        };
        let l = video_layout(work, 1.0, (1920, 1080));
        assert!(l.preview.bottom <= l.bar.top);
        assert!(l.bar.bottom <= l.undo.top);
        assert!(l.play.right <= l.time.left && l.time.right <= l.bar.left);
        assert!(l.bar.right <= l.split.left && l.split.right <= l.toggle.left);
        assert!(l.tools[7].right <= l.panel.right);
        // Same window as the image panel would get plus one timeline row at most.
        let img = panel_layout(work, 1.0, (1920, 1080));
        assert!((l.window.right - l.window.left) <= (img.window.right - img.window.left));
    }

    #[test]
    fn video_layout_portrait_and_scale() {
        let work = Rect {
            left: 0,
            top: 0,
            right: 2560,
            bottom: 1400,
        };
        let l = video_layout(work, 2.0, (1080, 1920));
        assert!(l.preview.right - l.preview.left > 0.0 && l.preview.bottom - l.preview.top > 0.0);
        assert!(l.tools[7].right <= l.panel.right);
    }

    #[test]
    fn video_hits() {
        let l = video_layout(
            Rect {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1040,
            },
            1.0,
            (1280, 720),
        );
        let c = |b: Box2| ((b.left + b.right) / 2.0, (b.top + b.bottom) / 2.0);
        let (x, y) = c(l.tools[4]);
        assert_eq!(
            video_hit_test(&l, false, x, y),
            VideoHit::Tool(VideoTool::Speed)
        );
        let (x, y) = c(l.chips[2]);
        assert_eq!(video_hit_test(&l, false, x, y), VideoHit::Nothing);
        assert_eq!(video_hit_test(&l, true, x, y), VideoHit::Chip(2));
        let (x, y) = c(l.bar);
        assert_eq!(video_hit_test(&l, false, x, y), VideoHit::Bar);
        let (x, y) = c(l.toggle);
        assert_eq!(video_hit_test(&l, false, x, y), VideoHit::Toggle);
    }

    #[test]
    fn timeline_round_trip_and_clamp() {
        let t = Timeline {
            left: 100.0,
            right: 500.0,
            duration: 20.0,
        };
        assert_eq!(t.time_at(300.0), 10.0);
        assert_eq!(t.x_of(5.0), 200.0);
        assert_eq!(t.time_at(0.0), 0.0);
        assert_eq!(t.time_at(900.0), 20.0);
    }

    #[test]
    fn boxes_stay_inside_panel_and_ordered() {
        for s in [1.0, 1.5, 2.0] {
            let l = panel_layout(area(), s, (800, 600));
            let others = [l.undo, l.redo, l.cancel, l.save, l.preview];
            for b in l.tools.iter().chain(&l.chips).chain(&others) {
                assert!(b.left >= l.panel.left && b.right <= l.panel.right);
                assert!(b.top >= l.panel.top && b.bottom <= l.panel.bottom);
            }
            assert!(l.redo.right <= l.label.left && l.label.right <= l.cancel.left);
            assert!(l.cancel.right <= l.save.left);
        }
    }
}
