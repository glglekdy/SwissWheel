//! The state of one open wheel: which items it shows and which one is hovered.

use crate::catalog::{InputItem, Operation, WheelKind, initial_wheel, items_for, next_wheel};
use crate::geometry::{Hit, hit_test};

pub struct WheelSession {
    /// `None` until the dragged files are known (DragEnter).
    inputs: Option<Vec<InputItem>>,
    wheel: Option<WheelKind>,
    items: Vec<Operation>,
    hovered: Hit,
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
        }
    }

    pub fn set_inputs(&mut self, inputs: Vec<InputItem>) {
        let wheel = initial_wheel(&inputs);
        self.inputs = Some(inputs);
        self.show(wheel);
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

    /// Updates the hovered sector; `true` if it changed (redraw needed).
    pub fn hover(&mut self, dx: f32, dy: f32, scale: f32) -> bool {
        let hit = hit_test(self.items.len(), dx, dy, scale);
        let changed = hit != self.hovered;
        self.hovered = hit;
        changed
    }

    pub fn hovered(&self) -> Hit {
        self.hovered
    }

    pub fn drop_at(&self, dx: f32, dy: f32, scale: f32) -> Option<Operation> {
        match hit_test(self.items.len(), dx, dy, scale) {
            Hit::Item(i) => self.items.get(i).copied(),
            Hit::Cancel | Hit::Outside => None,
        }
    }

    fn show(&mut self, wheel: Option<WheelKind>) {
        self.wheel = wheel;
        self.items = match (wheel, &self.inputs) {
            (Some(w), Some(inputs)) => items_for(w, inputs),
            _ => Vec::new(),
        };
        self.hovered = Hit::Cancel;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{InputItem, Operation, Tool, WheelKind};
    use crate::format::Format;
    use crate::geometry::Hit;

    fn png() -> InputItem {
        InputItem::File {
            format: Some(Format::Png),
            animated_gif: false,
        }
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
        assert!(s.hover(0.0, -100.0, 1.0));
        assert_eq!(s.hovered(), Hit::Item(0));
        assert!(!s.hover(1.0, -101.0, 1.0)); // same sector
        assert_eq!(
            s.drop_at(0.0, -100.0, 1.0),
            Some(Operation::Convert(Format::Jpg))
        );
        assert_eq!(s.drop_at(0.0, 0.0, 1.0), None); // cancel center
    }

    #[test]
    fn cycle_switches_wheel_and_clears_hover() {
        let mut s = WheelSession::new();
        s.set_inputs(vec![png()]);
        s.hover(0.0, -100.0, 1.0);
        s.cycle();
        assert_eq!(s.wheel(), Some(WheelKind::Tools));
        assert_eq!(s.hovered(), Hit::Cancel);
        assert_eq!(
            s.drop_at(0.0, -100.0, 1.0),
            Some(Operation::Tool(Tool::HalveImage))
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
}
