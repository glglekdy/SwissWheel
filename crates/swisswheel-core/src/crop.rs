//! Crop rectangle math: handle dragging, aspect ratios and the ratio chips.

use crate::editor::PixelRect;

/// Smallest crop side in image pixels.
pub const MIN_CROP: u32 = 16;

/// The grab points of the crop rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CropHandle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
    Move,
}

/// A crop aspect ratio choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AspectRatio {
    Free,
    /// The ratio of the current result.
    Original,
    Fixed(u32, u32),
}

impl AspectRatio {
    /// Width / height, or `None` for a free crop.
    pub fn value(self, bounds: (u32, u32)) -> Option<f64> {
        match self {
            AspectRatio::Free => None,
            AspectRatio::Original => Some(bounds.0 as f64 / bounds.1 as f64),
            AspectRatio::Fixed(w, h) => Some(w as f64 / h as f64),
        }
    }
}

/// The ratio chips, in display order.
pub const CHIPS: [(AspectRatio, &str); 7] = [
    (AspectRatio::Free, "자유"),
    (AspectRatio::Original, "원본"),
    (AspectRatio::Fixed(1, 1), "1:1"),
    (AspectRatio::Fixed(4, 3), "4:3"),
    (AspectRatio::Fixed(3, 4), "3:4"),
    (AspectRatio::Fixed(16, 9), "16:9"),
    (AspectRatio::Fixed(9, 16), "9:16"),
];

/// Smallest crop side for an image side of `bound` pixels.
pub fn min_side(bound: u32) -> u32 {
    MIN_CROP.min(bound)
}

/// Rounds a float rectangle to pixels and clamps it inside `bounds`, each
/// side at least its minimum.
fn finish(x: f64, y: f64, w: f64, h: f64, bounds: (u32, u32)) -> PixelRect {
    let w = (w.round() as u32).clamp(min_side(bounds.0), bounds.0);
    let h = (h.round() as u32).clamp(min_side(bounds.1), bounds.1);
    let x = (x.round().max(0.0) as u32).min(bounds.0 - w);
    let y = (y.round().max(0.0) as u32).min(bounds.1 - h);
    PixelRect { x, y, w, h }
}

/// Clamps `v` to `lo..=hi`; `hi` wins when the range is empty.
fn clamp_hi(v: f64, lo: f64, hi: f64) -> f64 {
    v.max(lo).min(hi)
}

/// The rectangle after dragging `handle` by the total movement `dx`, `dy`
/// (image pixels since the press). `start` lies inside `bounds`.
pub fn drag(
    start: PixelRect,
    handle: CropHandle,
    dx: f64,
    dy: f64,
    bounds: (u32, u32),
    ratio: Option<f64>,
) -> PixelRect {
    use CropHandle::*;
    let (bw, bh) = (bounds.0 as f64, bounds.1 as f64);
    let (mw, mh) = (min_side(bounds.0) as f64, min_side(bounds.1) as f64);
    let (x, y) = (start.x as f64, start.y as f64);
    let (w, h) = (start.w as f64, start.h as f64);
    let (right, bottom) = (x + w, y + h);

    if handle == Move {
        let nx = clamp_hi(x + dx, 0.0, bw - w);
        let ny = clamp_hi(y + dy, 0.0, bh - h);
        return finish(nx, ny, w, h, bounds);
    }

    let moves_left = matches!(handle, TopLeft | Left | BottomLeft);
    let moves_right = matches!(handle, TopRight | Right | BottomRight);
    let moves_top = matches!(handle, TopLeft | Top | TopRight);
    let moves_bottom = matches!(handle, BottomLeft | Bottom | BottomRight);
    let horizontal = moves_left || moves_right;
    let vertical = moves_top || moves_bottom;

    // Room toward the bounds on the moving side.
    let room_w = if moves_left { right } else { bw - x };
    let room_h = if moves_top { bottom } else { bh - y };

    let Some(ratio) = ratio else {
        let (mut nx, mut ny, mut nw, mut nh) = (x, y, w, h);
        if moves_left {
            nx = clamp_hi(x + dx, 0.0, right - mw);
            nw = right - nx;
        } else if moves_right {
            nw = clamp_hi(w + dx, mw, room_w);
        }
        if moves_top {
            ny = clamp_hi(y + dy, 0.0, bottom - mh);
            nh = bottom - ny;
        } else if moves_bottom {
            nh = clamp_hi(h + dy, mh, room_h);
        }
        return finish(nx, ny, nw, nh, bounds);
    };

    // The dragged side is the width for left/right/corner handles and the
    // height for top/bottom. Staying inside bounds wins over the minimum.
    if horizontal {
        let want = if moves_left { w - dx } else { w + dx };
        let hi = if vertical {
            room_w.min(room_h * ratio)
        } else {
            room_w.min(bh * ratio)
        };
        let nw = clamp_hi(want, mw.max(mh * ratio), hi);
        let nh = nw / ratio;
        let nx = if moves_left { right - nw } else { x };
        let ny = if !vertical {
            clamp_hi(y + h / 2.0 - nh / 2.0, 0.0, bh - nh)
        } else if moves_top {
            bottom - nh
        } else {
            y
        };
        finish(nx, ny, nw, nh, bounds)
    } else {
        let want = if moves_top { h - dy } else { h + dy };
        let nh = clamp_hi(want, mh.max(mw / ratio), room_h.min(bw / ratio));
        let nw = nh * ratio;
        let ny = if moves_top { bottom - nh } else { y };
        let nx = clamp_hi(x + w / 2.0 - nw / 2.0, 0.0, bw - nw);
        finish(nx, ny, nw, nh, bounds)
    }
}

/// The largest rectangle of `ratio` inside `bounds`, centered on `crop`'s
/// center and shifted to stay inside.
pub fn fit_ratio(crop: PixelRect, bounds: (u32, u32), ratio: f64) -> PixelRect {
    let (bw, bh) = (bounds.0 as f64, bounds.1 as f64);
    let w = bw.min(bh * ratio);
    let h = w / ratio;
    let cx = crop.x as f64 + crop.w as f64 / 2.0;
    let cy = crop.y as f64 + crop.h as f64 / 2.0;
    let w = (w.round() as u32).clamp(1, bounds.0);
    let h = (h.round() as u32).clamp(1, bounds.1);
    let x = ((cx - w as f64 / 2.0).round().max(0.0) as u32).min(bounds.0 - w);
    let y = ((cy - h as f64 / 2.0).round().max(0.0) as u32).min(bounds.1 - h);
    PixelRect { x, y, w, h }
}

#[cfg(test)]
mod tests {
    use super::*;

    const B: (u32, u32) = (200, 100);
    fn r(x: u32, y: u32, w: u32, h: u32) -> PixelRect {
        PixelRect { x, y, w, h }
    }

    #[test]
    fn free_handles() {
        let s = r(50, 20, 100, 60);
        assert_eq!(
            drag(s, CropHandle::Right, 30.0, 99.0, B, None),
            r(50, 20, 130, 60)
        );
        assert_eq!(
            drag(s, CropHandle::TopLeft, -10.0, -5.0, B, None),
            r(40, 15, 110, 65)
        );
        assert_eq!(
            drag(s, CropHandle::Right, 500.0, 0.0, B, None),
            r(50, 20, 150, 60)
        );
        assert_eq!(
            drag(s, CropHandle::Left, 200.0, 0.0, B, None),
            r(134, 20, 16, 60)
        );
        assert_eq!(
            drag(s, CropHandle::Move, 500.0, -500.0, B, None),
            r(100, 0, 100, 60)
        );
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
    fn ratio_edge_handle_centers_the_other_side() {
        let s = r(50, 20, 80, 60);
        assert_eq!(
            drag(s, CropHandle::Right, -20.0, 0.0, B, Some(1.0)),
            r(50, 20, 60, 60)
        );
        assert_eq!(
            drag(s, CropHandle::Bottom, 0.0, -20.0, B, Some(1.0)),
            r(70, 20, 40, 40)
        );
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
        let out = drag(
            r(0, 0, 10, 10),
            CropHandle::Right,
            -50.0,
            0.0,
            (10, 10),
            None,
        );
        assert_eq!(out, r(0, 0, 10, 10));
    }

    #[test]
    fn chips_and_ratio_values() {
        assert_eq!(
            CHIPS.map(|c| c.1),
            ["자유", "원본", "1:1", "4:3", "3:4", "16:9", "9:16"]
        );
        assert_eq!(AspectRatio::Original.value((300, 100)), Some(3.0));
        assert_eq!(AspectRatio::Free.value((300, 100)), None);
    }
}
