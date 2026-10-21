//! The state of one open wheel: which items it shows, which one is hovered,
//! and which item's option panel is open beside it (spec 2.5).

use crate::catalog::{InputItem, Operation, WheelKind, initial_wheel, items_for, next_wheel};
use crate::config::Tools;
use crate::geometry::{Hit, PanelHit, Side, hit_test, panel_hit};
use crate::options::{OptionSet, Preset, options_for};

/// How long the cursor must rest on a tile before its panel opens or takes
/// over: the way from a tile to a panel on the far side crosses the cancel
/// area and other tiles that have options of their own.
pub const PANEL_DWELL_MS: u64 = 300;

/// What a drop picks: the operation and the option for it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Selection {
    pub op: Operation,
    /// `Some` whenever the item has options (the panel row, or the default
    /// when dropped on the tile itself).
    pub preset: Option<Preset>,
}

#[derive(Debug, Clone, Copy)]
pub struct PanelState {
    /// The wheel item whose options are shown.
    pub item: usize,
    pub set: &'static OptionSet,
    pub hovered: Option<usize>,
    /// The row a drop on the tile uses (marked "기본").
    pub default: usize,
}

pub struct WheelSession {
    /// `None` until the dragged files are known (DragEnter).
    inputs: Option<Vec<InputItem>>,
    wheel: Option<WheelKind>,
    items: Vec<Operation>,
    hovered: Hit,
    side: Side,
    /// The item whose panel is open.
    panel: Option<usize>,
    /// An item waiting to open its panel: (index, since when in ms).
    pending: Option<(usize, u64)>,
    tools: Tools,
}

impl Default for WheelSession {
    fn default() -> Self {
        Self::new()
    }
}

impl WheelSession {
    pub fn new() -> Self {
        WheelSession {
            inputs: None,
            wheel: None,
            items: Vec::new(),
            hovered: Hit::Cancel,
            side: Side::Right,
            panel: None,
            pending: None,
            tools: Tools::default(),
        }
    }

    /// The tool defaults from the settings (shrink ratio, compress strength).
    pub fn set_tools(&mut self, tools: Tools) {
        self.tools = tools;
    }

    pub fn set_inputs(&mut self, inputs: Vec<InputItem>) {
        let wheel = initial_wheel(&inputs);
        self.inputs = Some(inputs);
        self.show(wheel);
    }

    pub fn set_panel_side(&mut self, side: Side) {
        self.side = side;
    }

    pub fn is_loading(&self) -> bool {
        self.inputs.is_none()
    }

    pub fn wheel(&self) -> Option<WheelKind> {
        self.wheel
    }

    pub fn items(&self) -> &[Operation] {
        &self.items
    }

    pub fn cycle(&mut self) {
        if let (Some(inputs), Some(current)) = (&self.inputs, self.wheel) {
            let next = next_wheel(current, inputs);
            self.show(Some(next));
        }
    }

    /// Updates the hovered sector or panel row at `now_ms`; `true` if the
    /// highlight or the panel changed (redraw needed). Passing over tiles
    /// never closes or replaces the panel; resting on one for
    /// `PANEL_DWELL_MS` does.
    pub fn hover(&mut self, dx: f32, dy: f32, scale: f32, now_ms: u64) -> bool {
        let before = (self.hovered, self.panel);
        let hit = self.hit(dx, dy, scale);
        match hit {
            Hit::Item(i) if self.panel != Some(i) && self.options(i).is_some() => {
                if self.pending.is_none_or(|(p, _)| p != i) {
                    self.pending = Some((i, now_ms));
                }
            }
            _ => self.pending = None,
        }
        self.hovered = hit;
        self.tick(now_ms);
        (self.hovered, self.panel) != before
    }

    /// Finishes a dwell even when the cursor stopped moving (no DragOver
    /// comes then); `true` if the panel changed.
    pub fn tick(&mut self, now_ms: u64) -> bool {
        match self.pending {
            Some((i, since)) if now_ms.saturating_sub(since) >= PANEL_DWELL_MS => {
                self.pending = None;
                self.panel = Some(i);
                true
            }
            _ => false,
        }
    }

    pub fn switch_pending(&self) -> bool {
        self.pending.is_some()
    }

    pub fn hovered(&self) -> Hit {
        self.hovered
    }

    pub fn panel(&self) -> Option<PanelState> {
        let item = self.panel?;
        let set = self.options(item)?;
        Some(PanelState {
            item,
            set,
            default: set.default_index(&self.tools),
            hovered: match self.hovered {
                Hit::Choice(r) => Some(r),
                _ => None,
            },
        })
    }

    pub fn drop_at(&self, dx: f32, dy: f32, scale: f32) -> Option<Selection> {
        match self.hit(dx, dy, scale) {
            Hit::Item(i) => Some(Selection {
                op: *self.items.get(i)?,
                preset: self
                    .options(i)
                    .map(|set| set.choices[set.default_index(&self.tools)].preset),
            }),
            Hit::Choice(row) => {
                let item = self.panel?;
                Some(Selection {
                    op: self.items[item],
                    preset: Some(self.options(item)?.choices.get(row)?.preset),
                })
            }
            Hit::Cancel | Hit::Outside | Hit::Panel => None,
        }
    }

    /// The open panel takes the point first; it is drawn over the wheel's edge.
    fn hit(&self, dx: f32, dy: f32, scale: f32) -> Hit {
        if let Some(set) = self.panel.and_then(|i| self.options(i)) {
            match panel_hit(self.side, set.choices.len(), dx, dy, scale) {
                Some(PanelHit::Row(r)) => return Hit::Choice(r),
                Some(PanelHit::Header) => return Hit::Panel,
                None => {}
            }
        }
        hit_test(self.items.len(), dx, dy, scale)
    }

    fn options(&self, item: usize) -> Option<&'static OptionSet> {
        options_for(self.items.get(item)?, self.inputs.as_deref()?)
    }

    fn show(&mut self, wheel: Option<WheelKind>) {
        self.wheel = wheel;
        self.items = match (wheel, &self.inputs) {
            (Some(w), Some(inputs)) => items_for(w, inputs),
            _ => Vec::new(),
        };
        self.hovered = Hit::Cancel;
        self.panel = None;
        self.pending = None;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{InputItem, Operation, Tool, WheelKind};
    use crate::format::Format;
    use crate::geometry::{Hit, Side, panel_rect};
    use crate::options::{Level, Preset};

    fn file(format: Format) -> InputItem {
        InputItem::File {
            format: Some(format),
            animated_gif: false,
        }
    }
    fn png() -> InputItem {
        file(Format::Png)
    }
    /// MP4 → items [MKV, WEBM, MOV, AVI, GIF, MP3]; GIF sits at 240° (lower left).
    fn video() -> InputItem {
        file(Format::Mp4)
    }

    /// The middle of item `i` of `n` (radius 100).
    fn at(i: usize, n: usize) -> (f32, f32) {
        let rad = (i as f32 * 360.0 / n as f32).to_radians();
        (100.0 * rad.sin(), -100.0 * rad.cos())
    }

    /// The middle of panel row `i`.
    fn row_point(side: Side, rows: usize, i: usize) -> (f32, f32) {
        let r = panel_rect(side, rows, 1.0);
        let y =
            r.top + crate::geometry::PANEL_HEADER + (i as f32 + 0.5) * crate::geometry::PANEL_ROW;
        ((r.left + r.right) / 2.0, y)
    }

    fn open_panel(s: &mut WheelSession, i: usize) {
        let (x, y) = at(i, s.items().len());
        s.hover(x, y, 1.0, 0);
        assert!(s.tick(PANEL_DWELL_MS));
    }

    fn sel(op: Operation, preset: Option<Preset>) -> Option<Selection> {
        Some(Selection { op, preset })
    }

    #[test]
    fn loading_until_inputs_arrive() {
        let mut s = WheelSession::new();
        assert!(s.is_loading());
        assert_eq!(s.wheel(), None);
        assert_eq!(s.drop_at(0.0, -100.0, 1.0), None);
        s.set_inputs(vec![png()]);
        assert!(!s.is_loading());
        assert_eq!(s.wheel(), Some(WheelKind::Convert));
        assert_eq!(s.items()[0], Operation::Convert(Format::Jpg));
    }

    #[test]
    fn hover_reports_changes_and_drop_picks_item() {
        let mut s = WheelSession::new();
        s.set_inputs(vec![png()]); // 6 items: JPG at top
        assert!(s.hover(0.0, -100.0, 1.0, 0));
        assert_eq!(s.hovered(), Hit::Item(0));
        assert!(!s.hover(1.0, -101.0, 1.0, 0)); // same sector
        // JPG has quality options: dropping on the tile means the default.
        assert_eq!(
            s.drop_at(0.0, -100.0, 1.0),
            sel(
                Operation::Convert(Format::Jpg),
                Some(Preset::Quality(Level::Normal))
            )
        );
        assert_eq!(s.drop_at(0.0, 0.0, 1.0), None); // cancel center
    }

    #[test]
    fn cycle_switches_wheel_and_clears_hover() {
        let mut s = WheelSession::new();
        s.set_inputs(vec![png()]);
        s.hover(0.0, -100.0, 1.0, 0);
        s.cycle();
        assert_eq!(s.wheel(), Some(WheelKind::Tools));
        assert_eq!(s.hovered(), Hit::Cancel);
        assert_eq!(
            s.drop_at(0.0, -100.0, 1.0),
            sel(Operation::Tool(Tool::Shrink), Some(Preset::Scale(50)))
        );
    }

    #[test]
    fn empty_inputs_have_no_wheel() {
        let mut s = WheelSession::new();
        s.set_inputs(vec![]);
        assert!(!s.is_loading());
        assert_eq!(s.wheel(), None);
        s.cycle();
        assert_eq!(s.drop_at(0.0, -100.0, 1.0), None);
    }

    #[test]
    fn resting_on_an_option_tile_opens_its_panel() {
        let mut s = WheelSession::new();
        s.set_inputs(vec![video()]);
        let (x, y) = at(4, 6); // GIF
        assert!(s.hover(x, y, 1.0, 0)); // hovered changed
        assert!(s.panel().is_none() && s.switch_pending());
        assert!(!s.hover(x + 1.0, y, 1.0, 299));
        assert!(s.panel().is_none());
        assert!(s.tick(PANEL_DWELL_MS));
        let p = s.panel().unwrap();
        assert_eq!((p.item, p.set.title, p.hovered), (4, "GIF 옵션", None));
        assert!(!s.switch_pending());
        // Leaving before the dwell ends cancels it.
        let mut t = WheelSession::new();
        t.set_inputs(vec![video()]);
        let (x, y) = at(2, 6); // MOV
        t.hover(x, y, 1.0, 0);
        let (x, y) = at(5, 6);
        t.hover(x, y, 1.0, 100);
        assert!(!t.tick(400));
        assert!(t.panel().is_none());
    }

    #[test]
    fn panel_survives_the_trip_across_the_wheel() {
        let mut s = WheelSession::new();
        s.set_inputs(vec![video()]);
        open_panel(&mut s, 4); // GIF, lower left
        s.hover(0.0, 0.0, 1.0, 1000); // through the cancel area
        let (x, y) = at(2, 6); // MOV has its own options — but only passing
        s.hover(x, y, 1.0, 1100);
        let (rx, ry) = row_point(Side::Right, 3, 1);
        s.hover(rx, ry, 1.0, 1200);
        assert!(!s.tick(2000));
        assert_eq!(s.panel().unwrap().set.title, "GIF 옵션");
        assert_eq!(s.hovered(), Hit::Choice(1));
        // Resting on another option tile switches; a tile without options never does.
        let (x, y) = at(0, 6); // MKV
        s.hover(x, y, 1.0, 3000);
        assert!(s.tick(3000 + PANEL_DWELL_MS));
        assert_eq!(s.panel().unwrap().set.title, "해상도");
        let (x, y) = at(5, 6); // MP3
        s.hover(x, y, 1.0, 4000);
        s.tick(5000);
        assert_eq!(s.panel().unwrap().set.title, "해상도");
    }

    #[test]
    fn tile_drop_uses_configured_tool_default() {
        use crate::catalog::Tool;
        use crate::config::Tools;
        use crate::options::Strength;
        let mut s = WheelSession::new();
        s.set_inputs(vec![video()]);
        s.cycle(); // tools wheel
        s.set_tools(Tools {
            shrink_percent: 50,
            compress_video: Strength::Strong,
        });
        let op = Operation::Tool(Tool::CompressVideo);
        let i = s.items().iter().position(|o| *o == op).unwrap();
        let (x, y) = at(i, s.items().len());
        assert_eq!(
            s.drop_at(x, y, 1.0),
            sel(op, Some(Preset::Compress(Strength::Strong)))
        );
        open_panel(&mut s, i);
        assert_eq!(s.panel().unwrap().default, 2);
    }

    #[test]
    fn drop_on_a_row_uses_its_preset_and_the_tile_uses_the_default() {
        let mut s = WheelSession::new();
        s.set_inputs(vec![video()]);
        let (gx, gy) = at(4, 6);
        open_panel(&mut s, 4);
        let (rx, ry) = row_point(Side::Right, 3, 0);
        s.hover(rx, ry, 1.0, 1000);
        assert_eq!(s.hovered(), Hit::Choice(0));
        assert_eq!(s.panel().unwrap().hovered, Some(0));
        assert_eq!(
            s.drop_at(rx, ry, 1.0),
            sel(
                Operation::Convert(Format::Gif),
                Some(Preset::Gif {
                    fps: 10,
                    max_width: Some(320)
                })
            )
        );
        assert_eq!(
            s.drop_at(gx, gy, 1.0),
            sel(
                Operation::Convert(Format::Gif),
                Some(Preset::Gif {
                    fps: 15,
                    max_width: Some(480)
                })
            )
        );
        let r = panel_rect(Side::Right, 3, 1.0);
        assert_eq!(s.drop_at((r.left + r.right) / 2.0, r.top + 5.0, 1.0), None); // header
        let (mx, my) = at(5, 6);
        assert_eq!(
            s.drop_at(mx, my, 1.0),
            sel(Operation::Convert(Format::Mp3), None)
        );
    }

    #[test]
    fn left_panel_and_cycle() {
        let mut s = WheelSession::new();
        s.set_inputs(vec![video()]);
        s.set_panel_side(Side::Left);
        open_panel(&mut s, 4);
        let (rx, ry) = row_point(Side::Left, 3, 2);
        s.hover(rx, ry, 1.0, 1000);
        assert_eq!(s.hovered(), Hit::Choice(2));
        s.cycle();
        assert!(s.panel().is_none() && !s.switch_pending());
    }
}
