//! The wheel popup: window management and Direct2D rendering.

mod glyph;
mod render;
mod window;

use swisswheel_core::catalog::Operation;
use swisswheel_core::geometry::Hit;

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
}
