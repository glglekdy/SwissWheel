//! The wheel popup: window management and Direct2D rendering.

mod glyph;
pub(crate) mod render;
mod window;

use swisswheel_core::appearance::Accent;
use swisswheel_core::catalog::Operation;
use swisswheel_core::geometry::{Hit, Side};
use swisswheel_core::options::Choice;

pub use window::WheelWindow;

/// Everything the renderer needs for one frame.
pub struct WheelView<'a> {
    /// The current wheel's name (변환/도구/동작), small, in the center.
    pub title: &'a str,
    /// What is being dragged ("2.4 MB"), large, in the center.
    pub detail: &'a str,
    pub items: &'a [Operation],
    pub hovered: Hit,
    /// Open animation, 0.0 → 1.0.
    pub progress: f32,
    /// Fills the hovered tile and row.
    pub accent: Accent,
    /// The option panel beside the wheel, when open (spec 2.5).
    pub panel: Option<PanelView<'a>>,
}

pub struct PanelView<'a> {
    pub title: &'a str,
    pub choices: &'a [Choice],
    pub default: usize,
    pub hovered: Option<usize>,
    pub side: Side,
    /// Fade-in, 0.0 → 1.0.
    pub progress: f32,
}
