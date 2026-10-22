//! The desktop menu popup: a bar of tiles at the top of the screen and the
//! settings panel under it (menu design spec 2).

mod render;
mod window;

use swisswheel_core::appearance::Accent;
use swisswheel_core::config::Config;
use swisswheel_core::menu::{Focus, MenuHit};
use swisswheel_core::settings::{Category, Dir, SettingItem, step};
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;

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
    /// A shortcut row: the value is drawn as a key, without ◀ / ▶.
    pub keycap: bool,
    /// A color dot drawn before the value (the accent row).
    pub swatch: Option<D2D1_COLOR_F>,
}

/// The rows of `category`'s tab for `config`, top to bottom.
pub fn rows(config: &Config, category: Category, failed: Option<SettingItem>) -> Vec<RowView> {
    category
        .items()
        .iter()
        .map(|&item| RowView {
            label: item.label(),
            value: item.value(config),
            failed: failed == Some(item),
            back: step(config, item, Dir::Back).is_some(),
            forward: step(config, item, Dir::Forward).is_some(),
            keycap: item.is_key(),
            swatch: (item == SettingItem::Accent)
                .then(|| crate::theme::rgb(config.appearance.accent.rgb(), 1.0)),
        })
        .collect()
}

/// Everything the renderer needs for one frame.
pub struct MenuView<'a> {
    pub selected: usize,
    /// The selected tab's rows while the panel is open.
    pub rows: Option<&'a [RowView]>,
    /// Index into `CATEGORIES`.
    pub category: usize,
    pub focus: Option<Focus>,
    /// The focused shortcut row waits for a key.
    pub capturing: bool,
    pub hovered: MenuHit,
    pub accent: Accent,
    /// Shown above the tabs when the config file can't be saved to.
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
