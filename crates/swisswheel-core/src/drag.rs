//! Decides, from raw mouse/keyboard hook events, when to open the wheel and
//! which key presses to swallow. Pure state machine; the hook module feeds it.

use crate::config::Config;

/// Win32 virtual-key codes for the three wheel keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyBindings {
    pub open: u16,
    pub cycle: u16,
    pub cancel: u16,
}

impl From<&Config> for KeyBindings {
    fn from(c: &Config) -> Self {
        KeyBindings {
            open: c.open_key.vk(),
            cycle: c.cycle_key.vk(),
            cancel: c.cancel_key.vk(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragEvent {
    ButtonDown {
        x: i32,
        y: i32,
    },
    Move {
        x: i32,
        y: i32,
    },
    ButtonUp,
    KeyDown(u16),
    KeyUp(u16),
    /// The app closed the wheel because no file drag arrived in time.
    EnterTimedOut,
    /// The app closed the wheel (drop, cancel, or release outside).
    WheelClosed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragCommand {
    OpenWheel,
    Cycle,
    Cancel,
    ButtonReleasedWhileOpen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    pub swallow: bool,
    pub command: Option<DragCommand>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Button {
    Up,
    Pressed { x: i32, y: i32 },
    Dragging,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wheel {
    Closed,
    Open,
    /// Closed after a timeout; don't reopen until the open key or button is released.
    Suppressed,
}

pub struct DragTracker {
    keys: KeyBindings,
    threshold: (i32, i32),
    button: Button,
    wheel: Wheel,
    /// Indexed by virtual-key code: the last key-down for this key was swallowed.
    swallowed: [bool; 256],
}

impl DragTracker {
    /// `threshold`: the system drag distance (`SM_CXDRAG`, `SM_CYDRAG`).
    pub fn new(keys: KeyBindings, threshold: (i32, i32)) -> Self {
        DragTracker {
            keys,
            threshold,
            button: Button::Up,
            wheel: Wheel::Closed,
            swallowed: [false; 256],
        }
    }

    pub fn wheel_open(&self) -> bool {
        self.wheel == Wheel::Open
    }

    pub fn on_event(&mut self, event: DragEvent) -> Decision {
        let pass = Decision {
            swallow: false,
            command: None,
        };
        match event {
            DragEvent::ButtonDown { x, y } => {
                self.button = Button::Pressed { x, y };
                pass
            }
            DragEvent::Move { x, y } => {
                if let Button::Pressed { x: x0, y: y0 } = self.button
                    && ((x - x0).abs() >= self.threshold.0 || (y - y0).abs() >= self.threshold.1)
                {
                    self.button = Button::Dragging;
                }
                pass
            }
            DragEvent::ButtonUp => {
                self.button = Button::Up;
                match self.wheel {
                    Wheel::Open => Decision {
                        swallow: false,
                        command: Some(DragCommand::ButtonReleasedWhileOpen),
                    },
                    Wheel::Suppressed => {
                        self.wheel = Wheel::Closed;
                        pass
                    }
                    Wheel::Closed => pass,
                }
            }
            DragEvent::EnterTimedOut => {
                if self.wheel == Wheel::Open {
                    self.wheel = Wheel::Suppressed;
                }
                pass
            }
            DragEvent::WheelClosed => {
                if self.wheel == Wheel::Open {
                    self.wheel = Wheel::Closed;
                }
                pass
            }
            DragEvent::KeyDown(vk) => {
                let command = self.key_down(vk);
                // Key repeat of the open key while the wheel is up is swallowed too.
                let swallow =
                    command.is_some() || (self.wheel == Wheel::Open && vk == self.keys.open);
                self.set_swallowed(vk, swallow);
                Decision { swallow, command }
            }
            DragEvent::KeyUp(vk) => {
                if vk == self.keys.open && self.wheel == Wheel::Suppressed {
                    self.wheel = Wheel::Closed;
                }
                let swallow = self.is_swallowed(vk);
                self.set_swallowed(vk, false);
                Decision {
                    swallow,
                    command: None,
                }
            }
        }
    }

    /// The wheel command this key press triggers, updating the wheel state.
    fn key_down(&mut self, vk: u16) -> Option<DragCommand> {
        let k = self.keys;
        match self.wheel {
            Wheel::Closed if vk == k.open && self.button == Button::Dragging => {
                self.wheel = Wheel::Open;
                Some(DragCommand::OpenWheel)
            }
            Wheel::Open if vk == k.cycle => Some(DragCommand::Cycle),
            Wheel::Open if vk == k.cancel => {
                self.wheel = Wheel::Closed;
                Some(DragCommand::Cancel)
            }
            _ => None,
        }
    }

    fn is_swallowed(&self, vk: u16) -> bool {
        self.swallowed.get(vk as usize).copied().unwrap_or(false)
    }

    fn set_swallowed(&mut self, vk: u16, value: bool) {
        if let Some(slot) = self.swallowed.get_mut(vk as usize) {
            *slot = value;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPACE: u16 = 0x20;
    const TAB: u16 = 0x09;
    const ESC: u16 = 0x1B;
    const A: u16 = 0x41;

    fn tracker() -> DragTracker {
        DragTracker::new(
            KeyBindings {
                open: SPACE,
                cycle: TAB,
                cancel: ESC,
            },
            (4, 4),
        )
    }

    fn pass() -> Decision {
        Decision {
            swallow: false,
            command: None,
        }
    }

    fn dragging() -> DragTracker {
        let mut t = tracker();
        t.on_event(DragEvent::ButtonDown { x: 100, y: 100 });
        t.on_event(DragEvent::Move { x: 104, y: 100 });
        t
    }

    #[test]
    fn space_without_drag_passes_through() {
        let mut t = tracker();
        assert_eq!(t.on_event(DragEvent::KeyDown(SPACE)), pass());
        t.on_event(DragEvent::ButtonDown { x: 0, y: 0 });
        t.on_event(DragEvent::Move { x: 3, y: 3 }); // below threshold
        assert_eq!(t.on_event(DragEvent::KeyDown(SPACE)), pass());
        assert_eq!(t.on_event(DragEvent::KeyUp(SPACE)), pass());
    }

    #[test]
    fn space_while_dragging_opens_and_swallows_pair() {
        let mut t = dragging();
        assert_eq!(
            t.on_event(DragEvent::KeyDown(SPACE)),
            Decision {
                swallow: true,
                command: Some(DragCommand::OpenWheel)
            }
        );
        assert!(t.wheel_open());
        assert_eq!(
            t.on_event(DragEvent::KeyDown(SPACE)),
            Decision {
                swallow: true,
                command: None
            }
        ); // repeat
        assert_eq!(
            t.on_event(DragEvent::KeyUp(SPACE)),
            Decision {
                swallow: true,
                command: None
            }
        );
    }

    #[test]
    fn tab_and_esc_only_while_open() {
        let mut t = dragging();
        assert_eq!(t.on_event(DragEvent::KeyDown(TAB)), pass());
        t.on_event(DragEvent::KeyDown(SPACE));
        assert_eq!(
            t.on_event(DragEvent::KeyDown(TAB)),
            Decision {
                swallow: true,
                command: Some(DragCommand::Cycle)
            }
        );
        assert_eq!(
            t.on_event(DragEvent::KeyUp(TAB)),
            Decision {
                swallow: true,
                command: None
            }
        );
        assert_eq!(t.on_event(DragEvent::KeyDown(A)), pass());
        assert_eq!(
            t.on_event(DragEvent::KeyDown(ESC)),
            Decision {
                swallow: true,
                command: Some(DragCommand::Cancel)
            }
        );
        assert!(!t.wheel_open());
        // still dragging: Space reopens
        t.on_event(DragEvent::KeyUp(SPACE));
        assert_eq!(
            t.on_event(DragEvent::KeyDown(SPACE)).command,
            Some(DragCommand::OpenWheel)
        );
    }

    #[test]
    fn timeout_suppresses_reopen_until_key_up() {
        let mut t = dragging();
        t.on_event(DragEvent::KeyDown(SPACE));
        assert_eq!(t.on_event(DragEvent::EnterTimedOut), pass());
        assert!(!t.wheel_open());
        assert_eq!(t.on_event(DragEvent::KeyDown(SPACE)), pass()); // repeat goes to the app
        assert_eq!(t.on_event(DragEvent::KeyUp(SPACE)), pass()); // its down was passed, so the up is too
        assert_eq!(
            t.on_event(DragEvent::KeyDown(SPACE)).command,
            Some(DragCommand::OpenWheel)
        );
    }

    #[test]
    fn button_up_while_open_waits_for_wheel_closed() {
        let mut t = dragging();
        t.on_event(DragEvent::KeyDown(SPACE));
        assert_eq!(
            t.on_event(DragEvent::ButtonUp),
            Decision {
                swallow: false,
                command: Some(DragCommand::ButtonReleasedWhileOpen)
            }
        );
        assert!(t.wheel_open());
        t.on_event(DragEvent::WheelClosed);
        assert!(!t.wheel_open());
        assert_eq!(t.on_event(DragEvent::KeyDown(SPACE)), pass()); // no longer dragging
    }

    #[test]
    fn button_and_move_never_swallowed() {
        let mut t = dragging();
        t.on_event(DragEvent::KeyDown(SPACE));
        assert!(!t.on_event(DragEvent::Move { x: 300, y: 300 }).swallow);
        assert!(!t.on_event(DragEvent::ButtonDown { x: 1, y: 1 }).swallow);
    }
}
