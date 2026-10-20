//! Low-level mouse and keyboard hooks. The callbacks only feed `DragTracker`
//! and post its commands to the host window; they never block.

use std::cell::{Cell, RefCell};

use swisswheel_core::drag::{DragCommand, DragEvent, DragTracker, KeyBindings};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Posted to the host window; `wParam` is `command_code(DragCommand)`.
pub const WM_APP_DRAG: u32 = WM_APP + 20;

thread_local! {
    static TRACKER: RefCell<Option<DragTracker>> = const { RefCell::new(None) };
    static TARGET: Cell<HWND> = Cell::new(HWND::default());
}

pub struct Hooks {
    mouse: HHOOK,
    keyboard: HHOOK,
}

/// Installs both hooks on this thread, which must run a message loop.
pub fn install(target: HWND, keys: KeyBindings) -> windows::core::Result<Hooks> {
    let threshold = unsafe { (GetSystemMetrics(SM_CXDRAG), GetSystemMetrics(SM_CYDRAG)) };
    TRACKER.set(Some(DragTracker::new(keys, threshold)));
    TARGET.set(target);
    let (mouse, keyboard) = set_hooks()?;
    Ok(Hooks { mouse, keyboard })
}

impl Hooks {
    /// Windows silently removes a hook whose callback ever exceeds
    /// `LowLevelHooksTimeout` and offers no way to detect it, so the app
    /// reinstalls periodically. Tracker state is kept.
    pub fn reinstall(&mut self) -> windows::core::Result<()> {
        let (mouse, keyboard) = set_hooks()?;
        self.unhook();
        (self.mouse, self.keyboard) = (mouse, keyboard);
        Ok(())
    }

    fn unhook(&self) {
        unsafe {
            let _ = UnhookWindowsHookEx(self.mouse);
            let _ = UnhookWindowsHookEx(self.keyboard);
        }
    }
}

impl Drop for Hooks {
    fn drop(&mut self) {
        self.unhook();
    }
}

fn set_hooks() -> windows::core::Result<(HHOOK, HHOOK)> {
    unsafe {
        let module = GetModuleHandleW(None)?;
        let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), Some(module.into()), 0)?;
        match SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), Some(module.into()), 0) {
            Ok(keyboard) => Ok((mouse, keyboard)),
            Err(e) => {
                let _ = UnhookWindowsHookEx(mouse);
                Err(e)
            }
        }
    }
}

/// Lets the app report wheel state changes (`EnterTimedOut`, `WheelClosed`).
pub fn notify(event: DragEvent) {
    feed(event);
}

pub fn command_code(command: DragCommand) -> usize {
    match command {
        DragCommand::OpenWheel => 1,
        DragCommand::Cycle => 2,
        DragCommand::Cancel => 3,
        DragCommand::ButtonReleasedWhileOpen => 4,
    }
}

pub fn command_from_code(code: usize) -> Option<DragCommand> {
    Some(match code {
        1 => DragCommand::OpenWheel,
        2 => DragCommand::Cycle,
        3 => DragCommand::Cancel,
        4 => DragCommand::ButtonReleasedWhileOpen,
        _ => return None,
    })
}

/// Feeds the tracker and posts any command; returns whether to swallow.
fn feed(event: DragEvent) -> bool {
    // try_borrow: never panic inside a hook, even if somehow re-entered.
    let decision = TRACKER.with(|t| {
        t.try_borrow_mut()
            .ok()
            .and_then(|mut t| t.as_mut().map(|t| t.on_event(event)))
    });
    let Some(decision) = decision else {
        return false;
    };
    if let Some(command) = decision.command {
        unsafe {
            let _ = PostMessageW(
                Some(TARGET.get()),
                WM_APP_DRAG,
                WPARAM(command_code(command)),
                LPARAM(0),
            );
        }
    }
    decision.swallow
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        let (x, y) = (info.pt.x, info.pt.y);
        let event = match wparam.0 as u32 {
            WM_LBUTTONDOWN => Some(DragEvent::ButtonDown { x, y }),
            WM_MOUSEMOVE => Some(DragEvent::Move { x, y }),
            WM_LBUTTONUP => Some(DragEvent::ButtonUp),
            _ => None,
        };
        if let Some(event) = event {
            feed(event); // mouse input is never swallowed
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let vk = info.vkCode as u16;
        let event = match wparam.0 as u32 {
            WM_KEYDOWN | WM_SYSKEYDOWN => Some(DragEvent::KeyDown(vk)),
            WM_KEYUP | WM_SYSKEYUP => Some(DragEvent::KeyUp(vk)),
            _ => None,
        };
        if let Some(event) = event
            && feed(event)
        {
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}
