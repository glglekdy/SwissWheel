//! Accent color presets for the glass UI.

use serde::{Deserialize, Serialize};

/// Failure red, used for failed notices regardless of the accent.
pub const FAILURE_RGB: u32 = 0xE5484D;

/// The accent presets the settings panel offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Accent {
    #[default]
    Blue,
    Purple,
    Pink,
    Red,
    Orange,
    Green,
    Graphite,
}

/// Every preset, in display order.
pub const ACCENTS: [Accent; 7] = [
    Accent::Blue,
    Accent::Purple,
    Accent::Pink,
    Accent::Red,
    Accent::Orange,
    Accent::Green,
    Accent::Graphite,
];

impl Accent {
    pub fn label(self) -> &'static str {
        match self {
            Accent::Blue => "블루",
            Accent::Purple => "퍼플",
            Accent::Pink => "핑크",
            Accent::Red => "레드",
            Accent::Orange => "오렌지",
            Accent::Green => "그린",
            Accent::Graphite => "그래파이트",
        }
    }

    /// 0xRRGGBB.
    pub fn rgb(self) -> u32 {
        match self {
            Accent::Blue => 0x0A84FF,
            Accent::Purple => 0x7D5BF0,
            Accent::Pink => 0xE0457F,
            Accent::Red => 0xE5484D,
            Accent::Orange => 0xE0680B,
            Accent::Green => 0x1E9E50,
            Accent::Graphite => 0x5C5C64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accents_in_order_with_labels_and_colors() {
        let rows: Vec<_> = ACCENTS.iter().map(|a| (a.label(), a.rgb())).collect();
        assert_eq!(
            rows,
            [
                ("블루", 0x0A84FF),
                ("퍼플", 0x7D5BF0),
                ("핑크", 0xE0457F),
                ("레드", 0xE5484D),
                ("오렌지", 0xE0680B),
                ("그린", 0x1E9E50),
                ("그래파이트", 0x5C5C64)
            ]
        );
        assert_eq!(Accent::default(), Accent::Blue);
    }
}
