//! The desktop menu popup: a bar of tiles at the top of the screen and the
//! settings panel under it (menu design spec 2).

mod render;
mod window;

use swisswheel_core::config::Config;
use swisswheel_core::menu::MenuHit;
use swisswheel_core::settings::{Dir, ITEMS, SettingItem, step};

pub use window::MenuBar;

/// One settings row as drawn.
pub struct RowView {
    pub label: &'static str,
    pub value: String,
    /// Saving this row's last change failed (shows ⚠).
    pub failed: bool,
    /// Whether ◀ / ▶ can move the value; dimmed when not.
    pub back: bool,
    pub forward: bool,
}

/// The panel's rows for `config`, top to bottom.
pub fn rows(config: &Config, failed: Option<SettingItem>) -> Vec<RowView> {
    ITEMS
        .iter()
        .map(|&item| RowView {
            label: item.label(),
            value: item.value(config),
            failed: failed == Some(item),
            back: step(config, item, Dir::Back).is_some(),
            forward: step(config, item, Dir::Forward).is_some(),
        })
        .collect()
}

/// Everything the renderer needs for one frame.
pub struct MenuView<'a> {
    pub selected: usize,
    /// The settings panel's rows while it is open.
    pub rows: Option<&'a [RowView]>,
    pub row: Option<usize>,
    pub hovered: MenuHit,
    /// Shown above the rows when the config file can't be saved to.
    pub notice: Option<&'a str>,
    /// Bar open animation and panel fade, 0.0 → 1.0.
    pub progress: f32,
    pub panel_progress: f32,
}

/// Mouse input on the menu window, in pixels relative to the window.
#[derive(Debug, Clone, Copy)]
pub enum MenuMouse {
    Move { x: f32, y: f32 },
    Leave,
    Click { x: f32, y: f32 },
    Wheel { x: f32, y: f32, delta: i16 },
}
