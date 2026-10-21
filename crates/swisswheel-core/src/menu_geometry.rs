//! Desktop menu layout (spec 2.6): the bar of tiles and the settings panel
//! under it, in physical pixels relative to the menu window.
//!
//! Constants are in DIPs (scale 1.0); callers pass the monitor's DPI scale.

use crate::geometry::Rect;
use crate::menu::{MenuHit, TILES};
use crate::settings::Dir;

pub const TILE: f32 = 112.0;
pub const TILE_GAP: f32 = 8.0;
pub const BAR_PAD: f32 = 8.0;
/// The bar's gap from the top of the work area.
pub const TOP_MARGIN: f32 = 16.0;
pub const PANEL_GAP: f32 = 8.0;
pub const ROW: f32 = 36.0;
pub const PANEL_PAD: f32 = 8.0;
pub const STEP_BUTTON: f32 = 28.0;
pub const VALUE_WIDTH: f32 = 88.0;
/// The bar slides down this far while opening; the window has room above it.
pub const DROP: f32 = 8.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Box2 {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Box2 {
    /// Edges count as inside; NaN never does.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.left && x <= self.right && y >= self.top && y <= self.bottom
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RowBoxes {
    pub row: Box2,
    pub back: Box2,
    pub forward: Box2,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MenuLayout {
    /// The window's size; it always has room for the open panel.
    pub width: i32,
    pub height: i32,
    pub bar: Box2,
    pub tiles: [Box2; 3],
    pub panel: Box2,
    /// "설정 파일에 오류가 있어 저장하지 않습니다", above the rows.
    pub notice: Option<Box2>,
    pub rows: Vec<RowBoxes>,
}

pub fn layout(rows: usize, notice: bool, scale: f32) -> MenuLayout {
    let s = |dip: f32| dip * scale;
    let bar_width = TILES.len() as f32 * TILE + (TILES.len() - 1) as f32 * TILE_GAP + 2.0 * BAR_PAD;
    let bar = Box2 {
        left: 0.0,
        top: s(DROP),
        right: s(bar_width),
        bottom: s(DROP + TILE + 2.0 * BAR_PAD),
    };
    let tiles = std::array::from_fn(|i| {
        let left = BAR_PAD + i as f32 * (TILE + TILE_GAP);
        Box2 {
            left: s(left),
            top: s(DROP + BAR_PAD),
            right: s(left + TILE),
            bottom: s(DROP + BAR_PAD + TILE),
        }
    });
    let lines = rows + usize::from(notice);
    let panel = Box2 {
        left: 0.0,
        top: bar.bottom + s(PANEL_GAP),
        right: bar.right,
        bottom: bar.bottom + s(PANEL_GAP + 2.0 * PANEL_PAD + lines as f32 * ROW),
    };
    let line = |i: usize| Box2 {
        left: panel.left + s(PANEL_PAD),
        top: panel.top + s(PANEL_PAD + i as f32 * ROW),
        right: panel.right - s(PANEL_PAD),
        bottom: panel.top + s(PANEL_PAD + (i + 1) as f32 * ROW),
    };
    let row_boxes = (0..rows)
        .map(|i| {
            let row = line(i + usize::from(notice));
            let forward = Box2 {
                left: row.right - s(STEP_BUTTON),
                ..row
            };
            let back_right = forward.left - s(VALUE_WIDTH);
            let back = Box2 {
                left: back_right - s(STEP_BUTTON),
                right: back_right,
                ..row
            };
            RowBoxes { row, back, forward }
        })
        .collect();
    MenuLayout {
        width: bar.right.ceil() as i32,
        height: panel.bottom.ceil() as i32,
        bar,
        tiles,
        panel,
        notice: notice.then(|| line(0)),
        rows: row_boxes,
    }
}

/// Top-left of the window on screen: centered in the work area, with the bar
/// `TOP_MARGIN` below its top edge.
pub fn window_origin(work_area: Rect, layout: &MenuLayout, scale: f32) -> (i32, i32) {
    let x = work_area.left + (work_area.right - work_area.left - layout.width) / 2;
    let y = work_area.top + ((TOP_MARGIN - DROP) * scale).round() as i32;
    (x, y)
}

pub fn hit(layout: &MenuLayout, panel_open: bool, x: f32, y: f32) -> MenuHit {
    if let Some(i) = layout.tiles.iter().position(|t| t.contains(x, y)) {
        return MenuHit::Tile(i);
    }
    if layout.bar.contains(x, y) {
        return MenuHit::Panel;
    }
    if !panel_open || !layout.panel.contains(x, y) {
        return MenuHit::Outside;
    }
    for (i, r) in layout.rows.iter().enumerate() {
        if r.back.contains(x, y) {
            return MenuHit::Step(i, Dir::Back);
        }
        if r.forward.contains(x, y) {
            return MenuHit::Step(i, Dir::Forward);
        }
        if r.row.contains(x, y) {
            return MenuHit::Row(i);
        }
    }
    MenuHit::Panel
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Rect;
    use crate::menu::MenuHit;
    use crate::settings::Dir;

    fn mid(b: &Box2) -> (f32, f32) {
        ((b.left + b.right) / 2.0, (b.top + b.bottom) / 2.0)
    }

    #[test]
    fn bar_and_panel_share_width() {
        let l = layout(9, false, 1.0);
        assert_eq!(l.width, 368);
        assert_eq!(l.bar.right - l.bar.left, 368.0);
        assert_eq!(l.panel.right - l.panel.left, 368.0);
        assert_eq!(l.panel.bottom - l.panel.top, 8.0 * 2.0 + 9.0 * 36.0);
        assert_eq!(l.height as f32, l.panel.bottom);
        assert_eq!(l.panel.top, l.bar.bottom + 8.0);
    }

    #[test]
    fn origin_is_top_center_of_work_area() {
        let l = layout(9, false, 1.0);
        let work = Rect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        };
        assert_eq!(window_origin(work, &l, 1.0), ((1920 - 368) / 2, 16 - 8));
    }

    #[test]
    fn origin_on_negative_monitor() {
        let l = layout(9, false, 1.5);
        let work = Rect {
            left: -2560,
            top: -200,
            right: 0,
            bottom: 1240,
        };
        let (x, y) = window_origin(work, &l, 1.5);
        assert_eq!(x, -2560 + (2560 - l.width) / 2);
        assert_eq!(y, -200 + 12);
    }

    #[test]
    fn hit_tiles_rows_and_steps() {
        let l = layout(9, false, 1.0);
        let (x, y) = mid(&l.tiles[1]);
        assert_eq!(hit(&l, false, x, y), MenuHit::Tile(1));
        let (x, y) = mid(&l.rows[3].row);
        assert_eq!(hit(&l, false, x, y), MenuHit::Outside); // panel closed
        assert_eq!(hit(&l, true, l.rows[3].row.left + 4.0, y), MenuHit::Row(3));
        let (x, y) = mid(&l.rows[3].back);
        assert_eq!(hit(&l, true, x, y), MenuHit::Step(3, Dir::Back));
        let (x, y) = mid(&l.rows[3].forward);
        assert_eq!(hit(&l, true, x, y), MenuHit::Step(3, Dir::Forward));
        assert_eq!(
            hit(&l, true, l.panel.left + 2.0, l.panel.top + 2.0),
            MenuHit::Panel
        );
        assert_eq!(hit(&l, true, f32::NAN, 0.0), MenuHit::Outside);
    }

    #[test]
    fn bar_margins_are_not_outside() {
        let l = layout(9, false, 1.0);
        let gap_x = (l.tiles[0].right + l.tiles[1].left) / 2.0;
        assert_eq!(hit(&l, false, gap_x, mid(&l.tiles[0]).1), MenuHit::Panel);
        assert_eq!(
            hit(&l, false, l.bar.left + 2.0, l.bar.top + 2.0),
            MenuHit::Panel
        );
        assert_eq!(hit(&l, false, 2.0, 2.0), MenuHit::Outside); // above the bar
    }

    #[test]
    fn notice_row_pushes_settings_down() {
        let l = layout(9, true, 1.0);
        let notice = l.notice.unwrap();
        assert_eq!(l.rows[0].row.top, notice.bottom);
        let y = (notice.top + notice.bottom) / 2.0;
        assert_eq!(hit(&l, true, notice.left + 4.0, y), MenuHit::Panel);
    }

    #[test]
    fn hit_scales_with_dpi() {
        let l1 = layout(9, false, 1.0);
        let l2 = layout(9, false, 1.25);
        assert_eq!(l2.width, (368.0f32 * 1.25).ceil() as i32);
        let b = &l2.tiles[2];
        assert_eq!(hit(&l2, false, b.left + 1.0, b.top + 1.0), MenuHit::Tile(2));
        assert_eq!(l2.tiles[2].left, l1.tiles[2].left * 1.25);
    }
}
