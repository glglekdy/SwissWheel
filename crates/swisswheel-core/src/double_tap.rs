//! Spots two quick taps of one key with nothing in between: left Ctrl twice
//! opens the desktop menu from any window. Times are the hook's millisecond
//! tick count, which wraps.

/// Left Ctrl. Right Ctrl is the 한자 key on Korean keyboards.
pub const LEFT_CTRL: u16 = 0xA2;
/// From the first press to the second release.
pub const WINDOW_MS: u32 = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    /// First press, held.
    Down {
        start: u32,
    },
    /// First press released.
    Tapped {
        start: u32,
    },
    /// Second press, held.
    DownAgain {
        start: u32,
    },
}

pub struct DoubleTap {
    key: u16,
    window_ms: u32,
    state: State,
}

impl DoubleTap {
    pub fn new(key: u16, window_ms: u32) -> Self {
        DoubleTap {
            key,
            window_ms,
            state: State::Idle,
        }
    }

    pub fn key_down(&mut self, vk: u16, time: u32) {
        if vk != self.key {
            self.state = State::Idle;
            return;
        }
        self.state = match self.state {
            State::Tapped { start } if self.within(start, time) => State::DownAgain { start },
            // Held: an auto-repeat, not a new press.
            held @ (State::Down { .. } | State::DownAgain { .. }) => held,
            State::Idle | State::Tapped { .. } => State::Down { start: time },
        };
    }

    /// `true` when this release completes the double tap.
    pub fn key_up(&mut self, vk: u16, time: u32) -> bool {
        let (next, fired) = match self.state {
            _ if vk != self.key => (State::Idle, false),
            State::Down { start } if self.within(start, time) => (State::Tapped { start }, false),
            State::DownAgain { start } if self.within(start, time) => (State::Idle, true),
            _ => (State::Idle, false),
        };
        self.state = next;
        fired
    }

    fn within(&self, start: u32, time: u32) -> bool {
        time.wrapping_sub(start) <= self.window_ms
    }

    /// Any other input (a mouse button, the wheel) breaks the taps up:
    /// Ctrl+click twice selects, it doesn't open the menu.
    pub fn interrupt(&mut self) {
        self.state = State::Idle;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const C: u16 = 0x43;

    fn tap() -> DoubleTap {
        DoubleTap::new(LEFT_CTRL, WINDOW_MS)
    }

    fn double(t: &mut DoubleTap, times: [u32; 4]) -> bool {
        t.key_down(LEFT_CTRL, times[0]);
        assert!(!t.key_up(LEFT_CTRL, times[1]));
        t.key_down(LEFT_CTRL, times[2]);
        t.key_up(LEFT_CTRL, times[3])
    }

    #[test]
    fn two_quick_taps_fire_on_the_second_release() {
        assert!(double(&mut tap(), [1000, 1080, 1200, 1280]));
    }

    #[test]
    fn too_slow_does_not_fire() {
        assert!(!double(&mut tap(), [1000, 1080, 1400, 1501]));
        assert!(double(&mut tap(), [1000, 1080, 1400, 1500])); // just in time
    }

    #[test]
    fn a_slow_first_tap_starts_over_at_the_next() {
        let mut t = tap();
        assert!(!double(&mut t, [0, 100, 900, 950])); // the 2nd is a new 1st
        t.key_down(LEFT_CTRL, 1000);
        assert!(t.key_up(LEFT_CTRL, 1050));
    }

    #[test]
    fn a_key_in_between_breaks_it() {
        // Ctrl+C then Ctrl+V: Ctrl taps, but with keys in between.
        let mut t = tap();
        t.key_down(LEFT_CTRL, 0);
        t.key_down(C, 20);
        t.key_up(C, 40);
        t.key_up(LEFT_CTRL, 60);
        t.key_down(LEFT_CTRL, 100);
        assert!(!t.key_up(LEFT_CTRL, 140));
        let mut t = tap();
        t.key_down(LEFT_CTRL, 0);
        t.key_up(LEFT_CTRL, 50);
        t.key_down(LEFT_CTRL, 100);
        t.key_down(C, 120); // Ctrl+C right after a tap
        t.key_up(C, 130);
        assert!(!t.key_up(LEFT_CTRL, 150));
    }

    #[test]
    fn a_click_in_between_breaks_it() {
        let mut t = tap();
        t.key_down(LEFT_CTRL, 0);
        t.key_up(LEFT_CTRL, 50);
        t.interrupt();
        t.key_down(LEFT_CTRL, 100);
        assert!(!t.key_up(LEFT_CTRL, 150));
    }

    #[test]
    fn held_key_repeats_are_one_press() {
        let mut t = tap();
        t.key_down(LEFT_CTRL, 0);
        t.key_down(LEFT_CTRL, 30); // auto-repeat
        t.key_up(LEFT_CTRL, 60);
        t.key_down(LEFT_CTRL, 100);
        t.key_down(LEFT_CTRL, 130);
        assert!(t.key_up(LEFT_CTRL, 160));
    }

    #[test]
    fn three_taps_fire_once() {
        let mut t = tap();
        assert!(double(&mut t, [0, 40, 80, 120]));
        t.key_down(LEFT_CTRL, 160);
        assert!(!t.key_up(LEFT_CTRL, 200));
    }

    #[test]
    fn other_keys_never_fire() {
        let mut t = tap();
        t.key_down(0xA3, 0); // right Ctrl
        t.key_up(0xA3, 40);
        t.key_down(0xA3, 80);
        assert!(!t.key_up(0xA3, 120));
    }

    #[test]
    fn tick_count_wrap() {
        assert!(double(&mut tap(), [u32::MAX - 100, u32::MAX - 20, 50, 120]));
    }
}
