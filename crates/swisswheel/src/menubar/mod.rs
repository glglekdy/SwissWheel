//! The desktop menu popup: a bar of tiles at the top of the screen and the
//! settings panel under it (menu design spec 2).

mod render;
mod window;

use swisswheel_core::appearance::Accent;
use swisswheel_core::config::Config;
use swisswheel_core::essentials::{ESSENTIALS, EssentialState, view};
use swisswheel_core::menu::{DownloadFocus, Focus, MenuHit};
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

/// One download panel row as drawn.
pub struct EssentialRowView {
    pub label: &'static str,
    pub purpose: &'static str,
    pub status: String,
    /// Left to right.
    pub buttons: Vec<&'static str>,
    /// A download into the app's folder, 0.0 → 1.0.
    pub progress: Option<f32>,
}

/// The download panel's rows, one per entry of `ESSENTIALS`.
pub fn essential_rows(states: &[EssentialState]) -> Vec<EssentialRowView> {
    ESSENTIALS
        .iter()
        .zip(states)
        .map(|(essential, state)| {
            let (status, actions) = view(state);
            EssentialRowView {
                label: essential.label(),
                purpose: essential.purpose(),
                status,
                buttons: actions.iter().map(|a| a.label()).collect(),
                progress: state.busy,
            }
        })
        .collect()
}

/// The open download panel.
pub struct DownloadView<'a> {
    pub header: String,
    pub rows: &'a [EssentialRowView],
    pub focus: Option<DownloadFocus>,
}

/// Everything the renderer needs for one frame.
pub struct MenuView<'a> {
    pub selected: usize,
    /// The selected tab's rows while the settings panel is open.
    pub rows: Option<&'a [RowView]>,
    /// While the download panel is open.
    pub download: Option<DownloadView<'a>>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use swisswheel_core::essentials::{EssentialState, Installed, Latest};

    #[test]
    fn essential_rows_carry_the_view() {
        let state = EssentialState {
            installed: Some(Installed::Own(Some("9.0.2".into()))),
            latest: Latest::Found("9.0.3".into()),
            busy: None,
        };
        let rows = essential_rows(&[state]);
        assert_eq!(
            (rows[0].label, rows[0].purpose, rows[0].status.as_str()),
            ("FFmpeg", "영상·오디오", "9.0.2 · 9.0.3 있음")
        );
        assert_eq!(rows[0].buttons, ["업데이트", "제거"]);
        assert_eq!(rows[0].progress, None);
        let busy = EssentialState {
            busy: Some(0.5),
            ..EssentialState::default()
        };
        assert_eq!(essential_rows(&[busy])[0].progress, Some(0.5));
    }
}
