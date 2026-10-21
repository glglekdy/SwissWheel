//! Desktop menu layout (spec 2.6, settings spec 2.5): the bar of tiles and the settings panel
//! under it, in physical pixels relative to the menu window.
//!
//! Constants are in DIPs (scale 1.0); callers pass the monitor's DPI scale.

use crate::geometry::Rect;
use crate::menu::{MenuHit, TILES};
use crate::settings::{CATEGORIES, Category, Dir, MAX_ROWS};

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
    /// A shortcut row's key, drawn where ◀ value ▶ would be.
    pub keycap: Box2,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MenuLayout {
    /// The window's size; it always has room for the open panel.
    pub width: i32,
    pub height: i32,
    pub bar: Box2,
    pub tiles: [Box2; 3],
    pub panel: Box2,
    /// "설정 파일에 오류가 있어 저장하지 않습니다", above the tabs.
    pub notice: Option<Box2>,
    /// The category tabs, left to right.
    pub tabs: [Box2; 4],
    /// `MAX_ROWS` rows; a tab with fewer leaves the rest empty.
    pub rows: Vec<RowBoxes>,
    /// The hint line under the rows.
    pub hint: Box2,
}

/// Which rows of the open panel react: the selected tab's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PanelShape {
    pub rows: usize,
    /// Shortcut rows: the value is a keycap, not ◀ / ▶.
    pub keycaps: bool,
}

impl PanelShape {
    pub fn of(category: Category) -> PanelShape {
        PanelShape {
            rows: category.items().len(),
            keycaps: category == Category::Keys,
        }
    }
}

pub fn layout(notice: bool, scale: f32) -> MenuLayout {
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
    // Notice (if any), tabs, the rows, then the hint.
    let first = usize::from(notice);
    let lines = first + 1 + MAX_ROWS + 1;
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
    let tab_line = line(first);
    let tab_width = (bar_width - 2.0 * PANEL_PAD) / CATEGORIES.len() as f32;
    let tabs = std::array::from_fn(|i| Box2 {
        left: panel.left + s(PANEL_PAD + i as f32 * tab_width),
        right: panel.left + s(PANEL_PAD + (i + 1) as f32 * tab_width),
        ..tab_line
    });
    let rows = (0..MAX_ROWS)
        .map(|i| {
            let row = line(first + 1 + i);
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
            let keycap = Box2 {
                left: back.left,
                right: forward.right,
                ..row
            };
            RowBoxes {
                row,
                back,
                forward,
                keycap,
            }
        })
        .collect();
    MenuLayout {
        width: bar.right.ceil() as i32,
        height: panel.bottom.ceil() as i32,
        bar,
        tiles,
        panel,
        notice: notice.then(|| line(0)),
        tabs,
        rows,
        hint: line(first + 1 + MAX_ROWS),
    }
}

/// Top-left of the window on screen: centered in the work area, with the bar
/// `TOP_MARGIN` below its top edge.
pub fn window_origin(work_area: Rect, layout: &MenuLayout, scale: f32) -> (i32, i32) {
    let x = work_area.left + (work_area.right - work_area.left - layout.width) / 2;
    let y = work_area.top + ((TOP_MARGIN - DROP) * scale).round() as i32;
    (x, y)
}

/// What is at (`x`, `y`). `panel` is `None` while the panel is closed.
pub fn hit(layout: &MenuLayout, panel: Option<PanelShape>, x: f32, y: f32) -> MenuHit {
    if let Some(i) = layout.tiles.iter().position(|t| t.contains(x, y)) {
        return MenuHit::Tile(i);
    }
    if layout.bar.contains(x, y) {
        return MenuHit::Panel;
    }
    let Some(shape) = panel.filter(|_| layout.panel.contains(x, y)) else {
        return MenuHit::Outside;
    };
    if let Some(i) = layout.tabs.iter().position(|t| t.contains(x, y)) {
        return MenuHit::Tab(i);
    }
    for (i, r) in layout.rows.iter().take(shape.rows).enumerate() {
        if shape.keycaps && r.keycap.contains(x, y) {
            return MenuHit::KeyCap(i);
        }
        if !shape.keycaps && r.back.contains(x, y) {
            return MenuHit::Step(i, Dir::Back);
        }
        if !shape.keycaps && r.forward.contains(x, y) {
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
    use crate::settings::{Category, Dir};

    fn mid(b: &Box2) -> (f32, f32) {
        ((b.left + b.right) / 2.0, (b.top + b.bottom) / 2.0)
    }

    fn keys() -> Option<PanelShape> {
        Some(PanelShape::of(Category::Keys))
    }

    fn convert() -> Option<PanelShape> {
        Some(PanelShape::of(Category::Convert))
    }

    #[test]
    fn bar_and_panel_share_width() {
        let l = layout(false, 1.0);
        assert_eq!(l.width, 368);
        assert_eq!(l.bar.right - l.bar.left, 368.0);
        assert_eq!(l.panel.right - l.panel.left, 368.0);
        assert_eq!(l.height as f32, l.panel.bottom);
        assert_eq!(l.panel.top, l.bar.bottom + 8.0);
    }

    #[test]
    fn panel_has_tabs_three_rows_and_hint() {
        let l = layout(false, 1.0);
        assert_eq!(l.panel.bottom - l.panel.top, 8.0 * 2.0 + 5.0 * 36.0);
        assert_eq!(l.rows.len(), 3);
        assert_eq!(l.tabs[0].top, l.panel.top + 8.0);
        assert_eq!((l.tabs[0].left, l.tabs[3].right), (8.0, 360.0));
        assert_eq!(l.tabs[1].left - l.tabs[0].left, 88.0);
        assert_eq!(l.rows[0].row.top, l.tabs[0].bottom);
        assert_eq!(l.hint.top, l.rows[2].row.bottom);
        assert_eq!(
            (l.rows[0].keycap.left, l.rows[0].keycap.right),
            (l.rows[0].back.left, l.rows[0].forward.right)
        );
    }

    #[test]
    fn origin_is_top_center_of_work_area() {
        let l = layout(false, 1.0);
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
        let l = layout(false, 1.5);
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
        let l = layout(false, 1.0);
        let (x, y) = mid(&l.tiles[1]);
        assert_eq!(hit(&l, None, x, y), MenuHit::Tile(1));
        let (x, y) = mid(&l.rows[1].row);
        assert_eq!(hit(&l, None, x, y), MenuHit::Outside); // panel closed
        assert_eq!(
            hit(&l, convert(), l.rows[1].row.left + 4.0, y),
            MenuHit::Row(1)
        );
        let (x, y) = mid(&l.rows[1].back);
        assert_eq!(hit(&l, convert(), x, y), MenuHit::Step(1, Dir::Back));
        let (x, y) = mid(&l.rows[1].forward);
        assert_eq!(hit(&l, convert(), x, y), MenuHit::Step(1, Dir::Forward));
        assert_eq!(
            hit(&l, convert(), l.panel.left + 2.0, l.panel.top + 2.0),
            MenuHit::Panel
        );
        assert_eq!(hit(&l, convert(), f32::NAN, 0.0), MenuHit::Outside);
    }

    #[test]
    fn hit_tabs_keycaps_and_hint() {
        let l = layout(false, 1.0);
        let (x, y) = mid(&l.tabs[2]);
        assert_eq!(hit(&l, keys(), x, y), MenuHit::Tab(2));
        assert_eq!(hit(&l, None, x, y), MenuHit::Outside);
        let (x, y) = mid(&l.rows[1].keycap);
        assert_eq!(hit(&l, keys(), x, y), MenuHit::KeyCap(1));
        let (x, y) = mid(&l.rows[1].back);
        assert_eq!(hit(&l, keys(), x, y), MenuHit::KeyCap(1));
        assert_eq!(
            hit(&l, keys(), l.rows[1].row.left + 4.0, y),
            MenuHit::Row(1)
        );
        let (x, y) = mid(&l.hint);
        assert_eq!(hit(&l, convert(), x, y), MenuHit::Panel);
    }

    #[test]
    fn tools_tab_has_no_third_row() {
        let l = layout(false, 1.0);
        let tools = Some(PanelShape::of(Category::Tools));
        let (x, y) = mid(&l.rows[2].row);
        assert_eq!(hit(&l, tools, x, y), MenuHit::Panel);
        let (x, y) = mid(&l.rows[2].forward);
        assert_eq!(hit(&l, tools, x, y), MenuHit::Panel);
    }

    #[test]
    fn bar_margins_are_not_outside() {
        let l = layout(false, 1.0);
        let gap_x = (l.tiles[0].right + l.tiles[1].left) / 2.0;
        assert_eq!(hit(&l, None, gap_x, mid(&l.tiles[0]).1), MenuHit::Panel);
        assert_eq!(
            hit(&l, None, l.bar.left + 2.0, l.bar.top + 2.0),
            MenuHit::Panel
        );
        assert_eq!(hit(&l, None, 2.0, 2.0), MenuHit::Outside); // above the bar
    }

    #[test]
    fn notice_row_pushes_tabs_down() {
        let l = layout(true, 1.0);
        let notice = l.notice.unwrap();
        assert_eq!(l.tabs[0].top, notice.bottom);
        assert_eq!(l.panel.bottom - l.panel.top, 8.0 * 2.0 + 6.0 * 36.0);
        let y = (notice.top + notice.bottom) / 2.0;
        assert_eq!(hit(&l, keys(), notice.left + 4.0, y), MenuHit::Panel);
    }

    #[test]
    fn hit_scales_with_dpi() {
        let l1 = layout(false, 1.0);
        let l2 = layout(false, 1.25);
        assert_eq!(l2.width, (368.0f32 * 1.25).ceil() as i32);
        let b = &l2.tiles[2];
        assert_eq!(hit(&l2, None, b.left + 1.0, b.top + 1.0), MenuHit::Tile(2));
        assert_eq!(l2.tiles[2].left, l1.tiles[2].left * 1.25);
        assert_eq!(l2.tabs[3].right, l1.tabs[3].right * 1.25);
    }
}
