//! Low-level mouse and keyboard hooks. The callbacks only feed `DragTracker`
//! and post its commands to the host window; they never block.

use std::cell::{Cell, RefCell};

use swisswheel_core::double_tap::{self, DoubleTap};
use swisswheel_core::drag::{DragCommand, DragEvent, DragTracker, KeyBindings};
use swisswheel_core::menu::MenuKey;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromWindow,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

/// Posted to the host window; the parameters are `encode(DragCommand)`.
pub const WM_APP_DRAG: u32 = WM_APP + 20;
/// Posted to the host window while the desktop menu watches the foreground;
/// `wParam` is the new foreground window.
pub const WM_APP_FOREGROUND: u32 = WM_APP + 21;

thread_local! {
    static TRACKER: RefCell<Option<DragTracker>> = const { RefCell::new(None) };
    static TARGET: Cell<HWND> = Cell::new(HWND::default());
    static MENU_TAP: RefCell<DoubleTap> =
        RefCell::new(DoubleTap::new(double_tap::LEFT_CTRL, double_tap::WINDOW_MS));
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

const MENU_KEYS: [MenuKey; 7] = [
    MenuKey::Left,
    MenuKey::Right,
    MenuKey::Up,
    MenuKey::Down,
    MenuKey::Enter,
    MenuKey::Escape,
    MenuKey::Open,
];

/// Packs a command into a posted message's parameters. Screen coordinates
/// fit in 16 bits each, as in `MAKELPARAM`.
pub fn encode(command: DragCommand) -> (WPARAM, LPARAM) {
    let code = match command {
        DragCommand::OpenWheel => 1,
        DragCommand::Cycle => 2,
        DragCommand::Cancel => 3,
        DragCommand::ButtonReleasedWhileOpen => 4,
        DragCommand::OpenMenu => 5,
        DragCommand::CloseMenu => 6,
        DragCommand::MenuKey(key) => 10 + MENU_KEYS.iter().position(|&k| k == key).unwrap_or(0),
        DragCommand::MenuPointerDown { x, y } => {
            let packed = (x as u16 as u32) | ((y as u16 as u32) << 16);
            return (WPARAM(20), LPARAM(packed as isize));
        }
        DragCommand::MenuCapture(vk) => return (WPARAM(21), LPARAM(vk as isize)),
    };
    (WPARAM(code), LPARAM(0))
}

pub fn decode(wparam: WPARAM, lparam: LPARAM) -> Option<DragCommand> {
    Some(match wparam.0 {
        1 => DragCommand::OpenWheel,
        2 => DragCommand::Cycle,
        3 => DragCommand::Cancel,
        4 => DragCommand::ButtonReleasedWhileOpen,
        5 => DragCommand::OpenMenu,
        6 => DragCommand::CloseMenu,
        code @ 10..=16 => DragCommand::MenuKey(MENU_KEYS[code - 10]),
        20 => DragCommand::MenuPointerDown {
            x: (lparam.0 & 0xFFFF) as u16 as i16 as i32,
            y: ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32,
        },
        21 => DragCommand::MenuCapture(lparam.0 as u16),
        _ => return None,
    })
}

/// Reports foreground window changes as `WM_APP_FOREGROUND` (the desktop
/// menu closes when another window comes forward). Out of context, so the
/// callback runs on this thread's message loop.
pub fn watch_foreground() -> Option<HWINEVENTHOOK> {
    let hook = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(foreground_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        )
    };
    (!hook.is_invalid()).then_some(hook)
}

pub fn unwatch_foreground(hook: HWINEVENTHOOK) {
    unsafe {
        let _ = UnhookWinEvent(hook);
    }
}

unsafe extern "system" fn foreground_proc(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    _object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    unsafe {
        let _ = PostMessageW(
            Some(TARGET.get()),
            WM_APP_FOREGROUND,
            WPARAM(hwnd.0 as usize),
            LPARAM(0),
        );
    }
}

/// Rebinds the wheel and menu keys (settings panel). UI thread only, like
/// the hooks themselves.
pub fn set_keys(keys: KeyBindings) {
    with_tracker(|t| t.set_keys(keys));
}

/// Turns the desktop menu off when its window can't be created.
pub fn set_menu_enabled(enabled: bool) {
    with_tracker(|t| t.set_menu_enabled(enabled));
}

/// The image editor opened or closed: while open, the menu stays shut.
pub fn set_editor_open(open: bool) {
    with_tracker(|t| t.set_editor_open(open));
}

/// The settings panel waits for a key: the next press comes back as
/// `MenuCapture` instead of reaching any window.
pub fn set_capture(on: bool) {
    with_tracker(|t| t.set_capture(on));
}

fn with_tracker(f: impl FnOnce(&mut DragTracker)) {
    TRACKER.with(|t| {
        if let Ok(mut t) = t.try_borrow_mut()
            && let Some(t) = t.as_mut()
        {
            f(t);
        }
    });
}

/// Whether `hwnd` is the desktop with keyboard focus on its icons (not on
/// the edit box of an icon being renamed). Any failure counts as "no", so
/// the key goes to the app as usual.
pub fn is_desktop(hwnd: HWND) -> bool {
    if hwnd.is_invalid() {
        return false;
    }
    unsafe {
        let has_defview =
            FindWindowExW(Some(hwnd), None, w!("SHELLDLL_DefView"), PCWSTR::null()).is_ok();
        let mut info = GUITHREADINFO {
            cbSize: size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        let thread = GetWindowThreadProcessId(hwnd, None);
        let focus = if thread != 0 && GetGUIThreadInfo(thread, &mut info).is_ok() {
            class_name(info.hwndFocus)
        } else {
            String::new()
        };
        desktop_check(&class_name(hwnd), has_defview, &focus)
    }
}

fn class_name(hwnd: HWND) -> String {
    if hwnd.is_invalid() {
        return String::new();
    }
    let mut buf = [0u16; 64];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..len.max(0) as usize])
}

/// The desktop is `Progman`, or a `WorkerW` once the wallpaper has been
/// animated, either way hosting `SHELLDLL_DefView`.
fn desktop_check(foreground_class: &str, has_defview: bool, focus_class: &str) -> bool {
    matches!(foreground_class, "Progman" | "WorkerW") && has_defview && focus_class != "Edit"
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
        let (wparam, lparam) = encode(command);
        unsafe {
            let _ = PostMessageW(Some(TARGET.get()), WM_APP_DRAG, wparam, lparam);
        }
    }
    decision.swallow
}

fn with_menu_tap<T>(f: impl FnOnce(&mut DoubleTap) -> T) -> Option<T> {
    MENU_TAP.with(|t| t.try_borrow_mut().ok().map(|mut t| f(&mut t)))
}

/// Whether `hwnd` covers its whole monitor, taskbar included, as a game or
/// a full-screen video does. A maximized window doesn't: its frame reaches
/// past the monitor's edges, and the taskbar's part stays uncovered.
fn is_fullscreen(hwnd: HWND) -> bool {
    if hwnd.is_invalid() {
        return false;
    }
    unsafe {
        let mut window = RECT::default();
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        !monitor.is_invalid()
            && GetWindowRect(hwnd, &mut window).is_ok()
            && GetMonitorInfoW(monitor, &mut info).as_bool()
            && window == info.rcMonitor
    }
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        let (x, y) = (info.pt.x, info.pt.y);
        let message = wparam.0 as u32;
        if matches!(
            message,
            WM_LBUTTONDOWN
                | WM_RBUTTONDOWN
                | WM_MBUTTONDOWN
                | WM_XBUTTONDOWN
                | WM_MOUSEWHEEL
                | WM_MOUSEHWHEEL
        ) {
            with_menu_tap(DoubleTap::interrupt); // Ctrl+click, Ctrl+scroll
        }
        let event = match message {
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
            WM_KEYDOWN | WM_SYSKEYDOWN if is_desktop(unsafe { GetForegroundWindow() }) => {
                Some(DragEvent::DesktopKeyDown(vk))
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => Some(DragEvent::KeyDown(vk)),
            WM_KEYUP | WM_SYSKEYUP => Some(DragEvent::KeyUp(vk)),
            _ => None,
        };
        let swallow = event.is_some_and(feed);
        let tapped = match event {
            Some(DragEvent::KeyUp(_)) => with_menu_tap(|t| t.key_up(vk, info.time)) == Some(true),
            Some(_) => {
                with_menu_tap(|t| t.key_down(vk, info.time));
                false
            }
            None => false,
        };
        if tapped {
            let front = unsafe { GetForegroundWindow() };
            feed(DragEvent::MenuShortcut {
                fullscreen: !is_desktop(front) && is_fullscreen(front),
            });
        }
        if swallow {
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_classes() {
        assert!(desktop_check("Progman", true, "SysListView32"));
        assert!(desktop_check("WorkerW", true, "SysListView32"));
        assert!(!desktop_check("WorkerW", false, "SysListView32")); // a wallpaper WorkerW
        assert!(!desktop_check("CabinetWClass", true, "DirectUIHWND")); // Explorer window
    }

    #[test]
    fn a_window_over_the_whole_monitor_is_fullscreen() {
        use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint};
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let m = unsafe {
            let primary = MonitorFromPoint(Default::default(), MONITOR_DEFAULTTOPRIMARY);
            assert!(GetMonitorInfoW(primary, &mut info).as_bool());
            info.rcMonitor
        };
        let popup = |l: i32, t: i32, r: i32, b: i32| unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                w!("STATIC"),
                PCWSTR::null(),
                WS_POPUP, // hidden: the rect is all that counts
                l,
                t,
                r - l,
                b - t,
                None,
                None,
                None,
                None,
            )
            .unwrap()
        };
        let full = popup(m.left, m.top, m.right, m.bottom);
        let smaller = popup(m.left, m.top, m.right, m.bottom - 48);
        assert!(is_fullscreen(full));
        assert!(!is_fullscreen(smaller));
        assert!(!is_fullscreen(HWND::default()));
        unsafe {
            let _ = DestroyWindow(full);
            let _ = DestroyWindow(smaller);
        }
    }

    #[test]
    fn rename_edit_is_not_desktop() {
        assert!(!desktop_check("Progman", true, "Edit"));
    }

    #[test]
    fn commands_round_trip() {
        for c in [
            DragCommand::OpenWheel,
            DragCommand::Cycle,
            DragCommand::Cancel,
            DragCommand::ButtonReleasedWhileOpen,
            DragCommand::OpenMenu,
            DragCommand::CloseMenu,
            DragCommand::MenuKey(MenuKey::Left),
            DragCommand::MenuKey(MenuKey::Open),
            DragCommand::MenuKey(MenuKey::Escape),
            DragCommand::MenuPointerDown { x: -2500, y: 1400 },
            DragCommand::MenuCapture(0x74),
            DragCommand::MenuCapture(0x91),
        ] {
            let (w, l) = encode(c);
            assert_eq!(decode(w, l), Some(c));
        }
    }
}
