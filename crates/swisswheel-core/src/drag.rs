//! Decides, from raw mouse/keyboard hook events, when to open the wheel and
//! which key presses to swallow. Pure state machine; the hook module feeds it.

use crate::config::Config;
use crate::menu::MenuKey;

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
    /// A key press while the desktop is the foreground window (and not
    /// renaming an icon). Otherwise the same as `KeyDown`.
    DesktopKeyDown(u16),
    KeyUp(u16),
    /// The app closed the wheel because no file drag arrived in time.
    EnterTimedOut,
    /// The app closed the wheel (drop, cancel, or release outside).
    WheelClosed,
    /// The app closed the desktop menu.
    MenuClosed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragCommand {
    OpenWheel,
    Cycle,
    Cancel,
    ButtonReleasedWhileOpen,
    OpenMenu,
    MenuKey(MenuKey),
    /// The left button went down while the menu is open; the app closes the
    /// menu if this is outside it.
    MenuPointerDown {
        x: i32,
        y: i32,
    },
    /// A drag started while the menu was open; the tracker already closed it.
    CloseMenu,
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
    /// Indexed by virtual-key code: down and not yet up. A key-down for a
    /// held key is an auto-repeat (low-level hooks have no repeat flag).
    held: [bool; 256],
    menu: bool,
    /// Off when the menu window couldn't be created: the open key passes.
    menu_enabled: bool,
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
            held: [false; 256],
            menu: false,
            menu_enabled: true,
        }
    }

    pub fn menu_open(&self) -> bool {
        self.menu
    }

    /// New bindings from the settings panel; take effect on the next key.
    pub fn set_keys(&mut self, keys: KeyBindings) {
        self.keys = keys;
    }

    pub fn set_menu_enabled(&mut self, enabled: bool) {
        self.menu_enabled = enabled;
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
                Decision {
                    swallow: false,
                    command: self.menu.then_some(DragCommand::MenuPointerDown { x, y }),
                }
            }
            DragEvent::Move { x, y } => {
                if let Button::Pressed { x: x0, y: y0 } = self.button
                    && ((x - x0).abs() >= self.threshold.0 || (y - y0).abs() >= self.threshold.1)
                {
                    self.button = Button::Dragging;
                    if self.menu {
                        self.menu = false;
                        return Decision {
                            swallow: false,
                            command: Some(DragCommand::CloseMenu),
                        };
                    }
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
            DragEvent::MenuClosed => {
                self.menu = false;
                pass
            }
            DragEvent::KeyDown(vk) => self.on_key_down(vk, false),
            DragEvent::DesktopKeyDown(vk) => self.on_key_down(vk, true),
            DragEvent::KeyUp(vk) => {
                self.set_held(vk, false);
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

    fn on_key_down(&mut self, vk: u16, desktop: bool) -> Decision {
        let repeat = self.is_held(vk);
        self.set_held(vk, true);
        let (swallow, command) = if self.menu {
            match self.menu_key(vk) {
                Some(key @ (MenuKey::Left | MenuKey::Right | MenuKey::Up | MenuKey::Down)) => {
                    (true, Some(DragCommand::MenuKey(key)))
                }
                Some(_) if repeat => (true, None),
                Some(key) => (true, Some(DragCommand::MenuKey(key))),
                None => (false, None),
            }
        } else {
            let command = self.key_down(vk);
            if command.is_some() || (self.wheel == Wheel::Open && vk == self.keys.open) {
                // Key repeat of the open key while the wheel is up is swallowed too.
                (true, command)
            } else if desktop
                && vk == self.keys.open
                && !repeat
                && self.menu_enabled
                && self.wheel == Wheel::Closed
                && self.button != Button::Dragging
            {
                self.menu = true;
                (true, Some(DragCommand::OpenMenu))
            } else {
                (false, None)
            }
        };
        self.set_swallowed(vk, swallow);
        Decision { swallow, command }
    }

    /// What a key means to the open menu, if anything.
    fn menu_key(&self, vk: u16) -> Option<MenuKey> {
        Some(match vk {
            _ if vk == self.keys.open => MenuKey::Open,
            0x25 => MenuKey::Left,
            0x26 => MenuKey::Up,
            0x27 => MenuKey::Right,
            0x28 => MenuKey::Down,
            0x0D => MenuKey::Enter,
            0x1B => MenuKey::Escape,
            _ => return None,
        })
    }

    fn is_held(&self, vk: u16) -> bool {
        self.held.get(vk as usize).copied().unwrap_or(false)
    }

    fn set_held(&mut self, vk: u16, value: bool) {
        if let Some(slot) = self.held.get_mut(vk as usize) {
            *slot = value;
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

    const LEFT: u16 = 0x25;
    const UP: u16 = 0x26;
    const RIGHT: u16 = 0x27;
    const ENTER: u16 = 0x0D;
    const CAPS: u16 = 0x14;

    fn swallow(command: DragCommand) -> Decision {
        Decision {
            swallow: true,
            command: Some(command),
        }
    }

    fn swallow_only() -> Decision {
        Decision {
            swallow: true,
            command: None,
        }
    }

    fn menu_open() -> DragTracker {
        let mut t = tracker();
        t.on_event(DragEvent::DesktopKeyDown(SPACE));
        t.on_event(DragEvent::KeyUp(SPACE));
        t
    }

    #[test]
    fn space_on_desktop_opens_menu_and_swallows_pair() {
        let mut t = tracker();
        assert_eq!(
            t.on_event(DragEvent::DesktopKeyDown(SPACE)),
            swallow(DragCommand::OpenMenu)
        );
        assert!(t.menu_open());
        assert_eq!(t.on_event(DragEvent::KeyUp(SPACE)), swallow_only());
        t.on_event(DragEvent::MenuClosed);
        assert_eq!(t.on_event(DragEvent::KeyDown(SPACE)), pass()); // not the desktop
    }

    #[test]
    fn menu_keys_are_swallowed_others_pass() {
        let mut t = menu_open();
        let right = swallow(DragCommand::MenuKey(MenuKey::Right));
        assert_eq!(t.on_event(DragEvent::KeyDown(RIGHT)), right);
        assert_eq!(t.on_event(DragEvent::KeyDown(RIGHT)), right); // arrows repeat
        assert_eq!(t.on_event(DragEvent::KeyUp(RIGHT)), swallow_only());
        assert_eq!(
            t.on_event(DragEvent::KeyDown(UP)).command,
            Some(DragCommand::MenuKey(MenuKey::Up))
        );
        assert_eq!(
            t.on_event(DragEvent::KeyDown(ENTER)),
            swallow(DragCommand::MenuKey(MenuKey::Enter))
        );
        assert_eq!(t.on_event(DragEvent::KeyDown(ENTER)), swallow_only()); // no repeat
        assert_eq!(t.on_event(DragEvent::KeyDown(A)), pass());
        assert_eq!(
            t.on_event(DragEvent::KeyDown(ESC)),
            swallow(DragCommand::MenuKey(MenuKey::Escape))
        );
        assert_eq!(
            t.on_event(DragEvent::DesktopKeyDown(SPACE)),
            swallow(DragCommand::MenuKey(MenuKey::Open))
        );
        assert!(t.menu_open()); // only the app closes it
        t.on_event(DragEvent::MenuClosed);
        assert!(!t.menu_open());
    }

    #[test]
    fn held_open_key_does_not_reopen_after_close() {
        let mut t = tracker();
        t.on_event(DragEvent::DesktopKeyDown(SPACE));
        assert_eq!(t.on_event(DragEvent::DesktopKeyDown(SPACE)), swallow_only());
        t.on_event(DragEvent::MenuClosed);
        // Still held: the repeats don't reopen it.
        assert_eq!(t.on_event(DragEvent::DesktopKeyDown(SPACE)).command, None);
        t.on_event(DragEvent::KeyUp(SPACE));
        assert_eq!(
            t.on_event(DragEvent::DesktopKeyDown(SPACE)).command,
            Some(DragCommand::OpenMenu)
        );
    }

    #[test]
    fn drag_wins_over_menu() {
        let mut t = dragging();
        assert_eq!(
            t.on_event(DragEvent::DesktopKeyDown(SPACE)).command,
            Some(DragCommand::OpenWheel)
        );
        let mut t = tracker();
        t.on_event(DragEvent::DesktopKeyDown(SPACE));
        assert_eq!(
            t.on_event(DragEvent::ButtonDown { x: 5, y: 6 }),
            Decision {
                swallow: false,
                command: Some(DragCommand::MenuPointerDown { x: 5, y: 6 })
            }
        );
        assert_eq!(
            t.on_event(DragEvent::Move { x: 50, y: 6 }),
            Decision {
                swallow: false,
                command: Some(DragCommand::CloseMenu)
            }
        );
        assert!(!t.menu_open());
    }

    #[test]
    fn set_keys_while_menu_open_uses_new_open_key() {
        let mut t = menu_open();
        t.set_keys(KeyBindings {
            open: CAPS,
            cycle: TAB,
            cancel: ESC,
        });
        assert_eq!(t.on_event(DragEvent::DesktopKeyDown(SPACE)), pass());
        assert_eq!(t.on_event(DragEvent::KeyUp(SPACE)), pass());
        assert_eq!(
            t.on_event(DragEvent::DesktopKeyDown(CAPS)).command,
            Some(DragCommand::MenuKey(MenuKey::Open))
        );
    }

    #[test]
    fn disabled_menu_lets_space_through() {
        let mut t = tracker();
        t.set_menu_enabled(false);
        assert_eq!(t.on_event(DragEvent::DesktopKeyDown(SPACE)), pass());
    }

    #[test]
    fn left_is_menu_key() {
        let mut t = menu_open();
        assert_eq!(
            t.on_event(DragEvent::KeyDown(LEFT)).command,
            Some(DragCommand::MenuKey(MenuKey::Left))
        );
    }
}
