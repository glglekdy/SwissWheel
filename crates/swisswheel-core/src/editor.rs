//! Edit history for the image editor: ops, undo/redo and the original-to-result transform.

/// A quarter turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Turn {
    /// Clockwise.
    Cw,
    /// Counter-clockwise.
    Ccw,
}

/// A mirror axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Left-right mirror.
    Horizontal,
    /// Top-bottom mirror.
    Vertical,
}

/// A rectangle in whole image pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl PixelRect {
    /// The rectangle covering a whole `(width, height)` image.
    pub fn full((w, h): (u32, u32)) -> Self {
        Self { x: 0, y: 0, w, h }
    }
}

/// One recorded edit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EditOp {
    /// Crop, in the coordinates of the result of the ops before it.
    Crop(PixelRect),
    Rotate(Turn),
    Flip(Axis),
    /// Set grayscale on or off.
    Grayscale(bool),
    /// Set whether metadata is stripped.
    StripInfo(bool),
}

/// What kind of output a save produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveKind {
    /// Pixels changed: `name (edited).ext`.
    Edited,
    /// Only metadata is removed: `name (no info).ext`.
    StripOnly,
}

impl SaveKind {
    /// The parenthesised word in the output file name.
    pub fn suffix(self) -> &'static str {
        match self {
            SaveKind::Edited => "edited",
            SaveKind::StripOnly => "no info",
        }
    }
}

/// The ops to apply on save, with the kind of output they produce.
#[derive(Debug, Clone, PartialEq)]
pub struct EditPlan {
    pub ops: Vec<EditOp>,
    pub kind: SaveKind,
}

/// An affine map `(x, y)` → `(xx·x + xy·y + x0, yx·x + yy·y + y0)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine {
    pub xx: f64,
    pub xy: f64,
    pub yx: f64,
    pub yy: f64,
    pub x0: f64,
    pub y0: f64,
}

impl Affine {
    pub const IDENTITY: Affine = Affine {
        xx: 1.0,
        xy: 0.0,
        yx: 0.0,
        yy: 1.0,
        x0: 0.0,
        y0: 0.0,
    };

    /// Maps a point.
    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.xx * x + self.xy * y + self.x0,
            self.yx * x + self.yy * y + self.y0,
        )
    }

    /// The map that applies `self` first, then `next`.
    pub fn then(&self, next: &Affine) -> Affine {
        Affine {
            xx: next.xx * self.xx + next.xy * self.yx,
            xy: next.xx * self.xy + next.xy * self.yy,
            yx: next.yx * self.xx + next.yy * self.yx,
            yy: next.yx * self.xy + next.yy * self.yy,
            x0: next.xx * self.x0 + next.xy * self.y0 + next.x0,
            y0: next.yx * self.x0 + next.yy * self.y0 + next.y0,
        }
    }
}

/// The edits made so far on one image, with undo and redo.
#[derive(Debug, Clone)]
pub struct EditSession {
    original: (u32, u32),
    strip_available: bool,
    ops: Vec<EditOp>,
    redo: Vec<EditOp>,
}

impl EditSession {
    /// A session over a `width`×`height` image; `strip_available` says whether info can be stripped.
    pub fn new(width: u32, height: u32, strip_available: bool) -> Self {
        Self {
            original: (width, height),
            strip_available,
            ops: Vec::new(),
            redo: Vec::new(),
        }
    }

    /// Records `op`. Returns false (recording nothing) for a no-op: a toggle equal to its
    /// current value, a strip when unavailable, or a crop of the whole image.
    pub fn push(&mut self, op: EditOp) -> bool {
        let ignored = match op {
            EditOp::Grayscale(on) => on == self.grayscale(),
            EditOp::StripInfo(on) => !self.strip_available || on == self.strip_info(),
            EditOp::Crop(r) => r == PixelRect::full(self.size()),
            EditOp::Rotate(_) | EditOp::Flip(_) => false,
        };
        if ignored {
            return false;
        }
        self.ops.push(op);
        self.redo.clear();
        true
    }

    /// Undoes the last op. Returns false when there is nothing to undo.
    pub fn undo(&mut self) -> bool {
        match self.ops.pop() {
            Some(op) => {
                self.redo.push(op);
                true
            }
            None => false,
        }
    }

    /// Redoes the last undone op. Returns false when there is nothing to redo.
    pub fn redo(&mut self) -> bool {
        match self.redo.pop() {
            Some(op) => {
                self.ops.push(op);
                true
            }
            None => false,
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.ops.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// The recorded ops, oldest first.
    pub fn ops(&self) -> &[EditOp] {
        &self.ops
    }

    /// The size of the image before any edit.
    pub fn original_size(&self) -> (u32, u32) {
        self.original
    }

    /// The size of the result after all recorded ops.
    pub fn size(&self) -> (u32, u32) {
        self.ops.iter().fold(self.original, |(w, h), op| match op {
            EditOp::Crop(r) => (r.w, r.h),
            EditOp::Rotate(_) => (h, w),
            _ => (w, h),
        })
    }

    /// Whether the result is grayscale.
    pub fn grayscale(&self) -> bool {
        self.last_toggle(|op| match op {
            EditOp::Grayscale(on) => Some(*on),
            _ => None,
        })
    }

    /// Whether info is stripped on save.
    pub fn strip_info(&self) -> bool {
        self.last_toggle(|op| match op {
            EditOp::StripInfo(on) => Some(*on),
            _ => None,
        })
    }

    pub fn strip_available(&self) -> bool {
        self.strip_available
    }

    /// Whether any op changes pixels (crop, rotate, flip, or grayscale on).
    pub fn has_pixel_edits(&self) -> bool {
        self.grayscale()
            || self
                .ops
                .iter()
                .any(|op| matches!(op, EditOp::Crop(_) | EditOp::Rotate(_) | EditOp::Flip(_)))
    }

    /// The kind of save the current edits call for, or `None` when there is nothing to save.
    pub fn save_kind(&self) -> Option<SaveKind> {
        if self.has_pixel_edits() {
            Some(SaveKind::Edited)
        } else if self.strip_info() {
            Some(SaveKind::StripOnly)
        } else {
            None
        }
    }

    /// The ops and kind to save, or `None` when there is nothing to save.
    pub fn plan(&self) -> Option<EditPlan> {
        self.save_kind().map(|kind| EditPlan {
            ops: self.ops.clone(),
            kind,
        })
    }

    /// The map from original image coordinates to result coordinates.
    pub fn to_result(&self) -> Affine {
        let mut size = self.original;
        let mut total = Affine::IDENTITY;
        for op in &self.ops {
            let (w, h) = (f64::from(size.0), f64::from(size.1));
            let step = match op {
                EditOp::Rotate(Turn::Cw) => Affine {
                    xx: 0.0,
                    xy: -1.0,
                    yx: 1.0,
                    yy: 0.0,
                    x0: h,
                    y0: 0.0,
                },
                EditOp::Rotate(Turn::Ccw) => Affine {
                    xx: 0.0,
                    xy: 1.0,
                    yx: -1.0,
                    yy: 0.0,
                    x0: 0.0,
                    y0: w,
                },
                EditOp::Flip(Axis::Horizontal) => Affine {
                    xx: -1.0,
                    x0: w,
                    ..Affine::IDENTITY
                },
                EditOp::Flip(Axis::Vertical) => Affine {
                    yy: -1.0,
                    y0: h,
                    ..Affine::IDENTITY
                },
                EditOp::Crop(r) => Affine {
                    x0: -f64::from(r.x),
                    y0: -f64::from(r.y),
                    ..Affine::IDENTITY
                },
                EditOp::Grayscale(_) | EditOp::StripInfo(_) => continue,
            };
            total = total.then(&step);
            size = match op {
                EditOp::Crop(r) => (r.w, r.h),
                EditOp::Rotate(_) => (size.1, size.0),
                _ => size,
            };
        }
        total
    }

    fn last_toggle(&self, pick: impl Fn(&EditOp) -> Option<bool>) -> bool {
        self.ops.iter().rev().find_map(pick).unwrap_or(false)
    }
}

/// Copy shown by the editor (Korean).
pub mod text {
    pub const TITLE: &str = "편집";
    pub const LOADING: &str = "불러오는 중";
    pub const DISCARD: &str = "편집을 버릴까요? Esc를 한 번 더 누르세요";
    pub const INFO_LOCKED: &str = "편집하면 정보는 자동으로 빠집니다";
    pub const ALREADY_OPEN: &str = "편집기가 이미 열려 있습니다";
    pub const OPEN_FAILED: &str = "편집기를 열 수 없습니다";
    pub const CLOSE_FIRST: &str = "편집기를 닫은 뒤 다시 시도하세요";
    pub const CANCEL: &str = "Esc 취소";
    pub const SAVE: &str = "↵ 저장";
    pub const CROP_CANCEL: &str = "Esc 자르기 취소";
    pub const CROP_APPLY: &str = "↵ 자르기 적용";
}

#[cfg(test)]
mod tests {
    use super::*;

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
        s.push(EditOp::Rotate(Turn::Cw)); // 20×40
        s.push(EditOp::Crop(PixelRect {
            x: 5,
            y: 10,
            w: 10,
            h: 20,
        })); // 10×20
        s.push(EditOp::Rotate(Turn::Ccw)); // 20×10
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
}
