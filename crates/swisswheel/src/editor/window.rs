//! The editor's two windows and their input: a dim window over the whole
//! monitor that swallows clicks, and the panel above it, which takes the
//! keyboard focus. Input goes to `EditorState`; what it returns is settled
//! by the parent module.

use std::cell::Cell;
use std::time::Instant;

use swisswheel_core::crop::CropHandle;
use swisswheel_core::editor_control::{EditorCommand, Outcome, key_command};
use swisswheel_core::editor_geometry::{EditorHit, EditorTool, Fit, handle_at, hit_test};
use swisswheel_core::geometry::Rect;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_USE_HOSTBACKDROPBRUSH, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    BLACK_BRUSH, GetMonitorInfoW, GetStockObject, HBRUSH, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MonitorFromPoint, ScreenToClient,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::Ime::{HIMC, ImmAssociateContext, ImmGetVirtualKey};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
    VK_CONTROL, VK_F4, VK_PROCESSKEY, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, PCWSTR, Result, w};

use super::{
    DISCARD_TIMER, Editor, WM_APP_EDITOR_LOADED, focus, on_loaded, render, settle, with_editor,
};

/// The dim window's opacity (of 255).
const DIM_ALPHA: u8 = 96;
/// Not exported by the `windows` crate under WindowsAndMessaging.
const WM_MOUSELEAVE: u32 = 0x02A3;
/// `WM_KEYDOWN`'s lParam bit for a key that was already down (auto-repeat).
const KEY_REPEAT: isize = 1 << 30;

thread_local! {
    /// A WM_MOUSELEAVE is requested for the current hover.
    static TRACKING: Cell<bool> = const { Cell::new(false) };
}

/// Where the panel goes: the monitor under the cursor, in screen pixels.
pub(super) struct Placement {
    pub monitor: Rect,
    pub work: Rect,
    pub scale: f32,
}

pub(super) fn placement() -> Placement {
    let rect = |r: windows::Win32::Foundation::RECT| Rect {
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
    };
    unsafe {
        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);
        let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
        let (mut dpi_x, mut dpi_y) = (96, 96);
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(monitor, &mut info);
        Placement {
            monitor: rect(info.rcMonitor),
            work: rect(info.rcWork),
            scale: dpi_x as f32 / 96.0,
        }
    }
}

/// The dim window and the panel it owns; dropping destroys both.
pub(super) struct Windows {
    pub dim: HWND,
    pub panel: HWND,
    /// The OS accepted `DWMWA_USE_HOSTBACKDROPBRUSH` (Windows 11 and later).
    pub glass_supported: bool,
}

impl Windows {
    /// Both windows, still hidden: the dim one over `monitor`, owned by
    /// `owner` (the app's host window), and the panel at `panel`.
    pub fn create(owner: HWND, monitor: Rect, panel: Rect) -> Result<Windows> {
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let dim_class = w!("SwissWheelEditorDim");
            let panel_class = w!("SwissWheelEditor");
            // Fails harmlessly when already registered by an earlier panel.
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(dim_proc),
                hInstance: instance.into(),
                lpszClassName: dim_class,
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
                ..Default::default()
            });
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(panel_proc),
                hInstance: instance.into(),
                lpszClassName: panel_class,
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                ..Default::default()
            });
            let dim = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED,
                dim_class,
                w!("SwissWheel editor dim"),
                WS_POPUP,
                monitor.left,
                monitor.top,
                monitor.right - monitor.left,
                monitor.bottom - monitor.top,
                Some(owner),
                None,
                Some(instance.into()),
                None,
            )?;
            // From here on, dropping `windows` cleans up after a failure.
            let mut windows = Windows {
                dim,
                panel: HWND::default(),
                glass_supported: false,
            };
            SetLayeredWindowAttributes(dim, COLORREF(0), DIM_ALPHA, LWA_ALPHA)?;
            windows.panel = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP,
                panel_class,
                w!("SwissWheel 편집"),
                WS_POPUP,
                panel.left,
                panel.top,
                panel.right - panel.left,
                panel.bottom - panel.top,
                Some(dim),
                None,
                Some(instance.into()),
                None,
            )?;
            // No IME for the panel: in 한글 mode the letter shortcuts would
            // reach WM_KEYDOWN as VK_PROCESSKEY and do nothing.
            ImmAssociateContext(windows.panel, HIMC::default());
            let enable = BOOL::from(true);
            windows.glass_supported = DwmSetWindowAttribute(
                windows.panel,
                DWMWA_USE_HOSTBACKDROPBRUSH,
                std::ptr::from_ref(&enable).cast(),
                size_of::<BOOL>() as u32,
            )
            .is_ok();
            Ok(windows)
        }
    }
}

impl Drop for Windows {
    fn drop(&mut self) {
        unsafe {
            if !self.panel.is_invalid() {
                let _ = DestroyWindow(self.panel);
            }
            let _ = DestroyWindow(self.dim);
        }
        TRACKING.set(false);
    }
}

/// Shows both windows and gives the panel the keyboard.
pub(super) fn show(dim: HWND, panel: HWND) {
    unsafe {
        let _ = ShowWindow(dim, SW_SHOWNOACTIVATE);
        let _ = ShowWindow(panel, SW_SHOW);
    }
    raise(panel);
}

/// Puts the panel (and its dim window) on top and makes it the foreground
/// window. A process that isn't in the foreground may not take it, so if
/// the plain call is refused, this joins the foreground thread's input
/// for a moment, which lifts that lock.
pub(super) fn raise(panel: HWND) {
    unsafe {
        if let Ok(dim) = GetWindow(panel, GW_OWNER) {
            let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
            let _ = SetWindowPos(dim, Some(HWND_TOPMOST), 0, 0, 0, 0, flags);
        }
        let _ = SetForegroundWindow(panel);
        if GetForegroundWindow() == panel {
            log_info!("편집기 포커스: SetForegroundWindow");
            return;
        }
        let theirs = GetWindowThreadProcessId(GetForegroundWindow(), None);
        let ours = GetCurrentThreadId();
        let attached =
            theirs != 0 && theirs != ours && AttachThreadInput(ours, theirs, true).as_bool();
        let _ = SetForegroundWindow(panel);
        let _ = SetFocus(Some(panel));
        if attached {
            let _ = AttachThreadInput(ours, theirs, false);
        }
        let won = GetForegroundWindow() == panel;
        log_info!(
            "편집기 포커스: AttachThreadInput ({})",
            if won { "성공" } else { "실패" }
        );
    }
}

impl Editor {
    /// What is under a panel point; the info button counts only when shown.
    fn hit(&self, x: f32, y: f32) -> EditorHit {
        match hit_test(&self.layout, self.state.crop().is_some(), x, y) {
            EditorHit::Tool(EditorTool::Info) if !render::info_shown(&self.state) => {
                EditorHit::Nothing
            }
            hit => hit,
        }
    }

    /// The image fit inside the preview, while an image is loaded.
    fn fit(&self) -> Option<Fit> {
        let size = self.state.session()?.size();
        Some(Fit::new(self.layout.preview, size))
    }

    /// The crop handle under a panel point, in crop mode.
    fn handle(&self, x: f32, y: f32) -> Option<CropHandle> {
        let crop = self.state.crop()?;
        handle_at(&self.fit()?, crop.rect, x, y, self.scale)
    }

    fn dragging(&self) -> Option<CropHandle> {
        self.state.crop()?.dragging()
    }

    fn key(&mut self, vk: u16, now: Instant) -> Outcome {
        let down = |key: u16| unsafe { GetKeyState(i32::from(key)) } < 0;
        match key_command(vk, down(VK_SHIFT.0), down(VK_CONTROL.0)) {
            Some(cmd) => self.state.command(cmd, now),
            None => self.state.key_other(now),
        }
    }

    fn press(&mut self, x: f32, y: f32) {
        let hit = self.hit(x, y);
        if matches!(hit, EditorHit::Preview | EditorHit::Nothing)
            && let Some(handle) = self.handle(x, y)
            && let Some(fit) = self.fit()
        {
            self.state.crop_press(handle, fit.to_image(x, y));
            self.press = None;
        } else {
            self.press = Some(hit);
        }
    }

    fn release(&mut self, x: f32, y: f32, now: Instant) -> Outcome {
        if self.dragging().is_some() {
            self.state.crop_release();
            return Outcome::Nothing;
        }
        let hit = self.hit(x, y);
        match self.press.take() {
            Some(pressed) if pressed == hit => self.state.click(hit, now),
            _ => Outcome::Nothing,
        }
    }

    fn mouse_move(&mut self, x: f32, y: f32) -> Outcome {
        if self.dragging().is_some() {
            let moved = self
                .fit()
                .is_some_and(|fit| self.state.crop_move(fit.to_image(x, y)));
            return if moved {
                Outcome::Redraw
            } else {
                Outcome::Nothing
            };
        }
        self.hover_to(self.hit(x, y))
    }

    fn hover_to(&mut self, hit: EditorHit) -> Outcome {
        if self.hover == hit {
            return Outcome::Nothing;
        }
        self.hover = hit;
        Outcome::Redraw
    }

    /// The cursor for a panel point: resize arrows on crop handles.
    fn cursor(&self, x: f32, y: f32) -> PCWSTR {
        match self.dragging().or_else(|| self.handle(x, y)) {
            Some(CropHandle::TopLeft | CropHandle::BottomRight) => IDC_SIZENWSE,
            Some(CropHandle::TopRight | CropHandle::BottomLeft) => IDC_SIZENESW,
            Some(CropHandle::Left | CropHandle::Right) => IDC_SIZEWE,
            Some(CropHandle::Top | CropHandle::Bottom) => IDC_SIZENS,
            Some(CropHandle::Move) => IDC_SIZEALL,
            None => IDC_ARROW,
        }
    }

    /// The discard prompt's timer fired: redraw once it has run out.
    fn tick(&mut self, now: Instant) -> Outcome {
        match self.state.discard_deadline() {
            Some(deadline) if deadline > now => {
                self.arm_discard_timer(now);
                Outcome::Nothing
            }
            _ => Outcome::Redraw,
        }
    }
}

/// Client coordinates packed in a mouse message's `lParam`.
fn client_point(lparam: LPARAM) -> (f32, f32) {
    let x = (lparam.0 & 0xFFFF) as u16 as i16;
    let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16;
    (x as f32, y as f32)
}

unsafe extern "system" fn panel_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let now = Instant::now();
    let outcome = match msg {
        WM_KEYDOWN => {
            // Holding a key must not repeat its command.
            if lparam.0 & KEY_REPEAT != 0 {
                return LRESULT(0);
            }
            let mut vk = wparam.0 as u16;
            // Should an IME still be attached, ask it for the real key.
            if vk == VK_PROCESSKEY.0 {
                vk = unsafe { ImmGetVirtualKey(hwnd) } as u16;
            }
            with_editor(|ed| ed.key(vk, now))
        }
        WM_SYSKEYDOWN => {
            // Handled here, not by DefWindowProc: it would turn every
            // auto-repeat of Alt+F4 into another WM_CLOSE, and the panel
            // has no system menu for Alt or F10 to open.
            if lparam.0 & KEY_REPEAT != 0 {
                return LRESULT(0);
            }
            with_editor(|ed| {
                if wparam.0 as u16 == VK_F4.0 {
                    ed.state.command(EditorCommand::Escape, now)
                } else {
                    ed.state.key_other(now)
                }
            })
        }
        // No menu to pick from: no mnemonic beep, no menu mode.
        WM_SYSCHAR => return LRESULT(0),
        WM_SYSCOMMAND if wparam.0 & 0xFFF0 == SC_KEYMENU as usize => return LRESULT(0),
        WM_CLOSE => with_editor(|ed| ed.state.command(EditorCommand::Escape, now)),
        WM_LBUTTONDOWN => {
            let (x, y) = client_point(lparam);
            with_editor(|ed| ed.press(x, y));
            unsafe { SetCapture(hwnd) };
            None
        }
        WM_LBUTTONUP => {
            let (x, y) = client_point(lparam);
            let outcome = with_editor(|ed| ed.release(x, y, now));
            unsafe {
                let _ = ReleaseCapture();
            }
            outcome
        }
        WM_CAPTURECHANGED => {
            // Capture taken away mid-drag (e.g. by a system dialog).
            with_editor(|ed| ed.state.crop_release());
            None
        }
        WM_MOUSEMOVE => {
            if !TRACKING.get() {
                let mut track = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                TRACKING.set(unsafe { TrackMouseEvent(&mut track) }.is_ok());
            }
            let (x, y) = client_point(lparam);
            with_editor(|ed| ed.mouse_move(x, y))
        }
        WM_MOUSELEAVE => {
            TRACKING.set(false);
            with_editor(|ed| ed.hover_to(EditorHit::Nothing))
        }
        WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT => {
            let mut pt = POINT::default();
            let cursor = unsafe {
                let _ = GetCursorPos(&mut pt);
                let _ = ScreenToClient(hwnd, &mut pt);
                with_editor(|ed| ed.cursor(pt.x as f32, pt.y as f32))
            };
            if let Ok(cursor) = unsafe { LoadCursorW(None, cursor.unwrap_or(IDC_ARROW)) } {
                unsafe { SetCursor(Some(cursor)) };
            }
            return LRESULT(1);
        }
        WM_TIMER if wparam.0 == DISCARD_TIMER => with_editor(|ed| ed.tick(now)),
        WM_APP_EDITOR_LOADED => {
            on_loaded();
            None
        }
        // The panel stays where it opened (spec 3.2).
        WM_DPICHANGED => return LRESULT(0),
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    };
    if let Some(outcome) = outcome {
        settle(outcome);
    }
    LRESULT(0)
}

unsafe extern "system" fn dim_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        // A click beside the panel doesn't close it; it hands focus back
        // and, like any click, clears the discard prompt.
        WM_LBUTTONDOWN => {
            focus();
            let now = Instant::now();
            if let Some(outcome) = with_editor(|ed| ed.state.click(EditorHit::Nothing, now)) {
                settle(outcome);
            }
            LRESULT(0)
        }
        // Closing is the panel's business (Esc, or the app quitting).
        WM_CLOSE => LRESULT(0),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
