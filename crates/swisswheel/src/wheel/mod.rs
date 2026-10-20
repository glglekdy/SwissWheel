//! The wheel popup: window management and Direct2D rendering.

mod render;
mod window;

use swisswheel_core::catalog::Operation;
use swisswheel_core::geometry::Hit;

pub use window::WheelWindow;

/// Everything the renderer needs for one frame.
pub struct WheelView<'a> {
    /// Shown in the center circle.
    pub title: &'a str,
    pub items: &'a [Operation],
    pub hovered: Hit,
    /// Open animation, 0.0 → 1.0.
    pub progress: f32,
}
