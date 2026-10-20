//! Throwaway spike: can we pop a drop-target window under the cursor while
//! Explorer is in an OLE drag, triggered by Space? See
//! docs/superpowers/plans/2026-10-20-swisswheel-spike-and-core.md, Task 1.

use std::cell::{Cell, RefCell};
use std::sync::OnceLock;
use std::time::Instant;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::*;
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

const WHEEL_SIZE: i32 = 320;
const WM_OPEN_WHEEL: u32 = WM_APP + 1;
const WM_TAB: u32 = WM_APP + 2;
const WM_ESC: u32 = WM_APP + 3;
const WM_MOUSE_UP: u32 = WM_APP + 4;
const TIMER_ENTER_TIMEOUT: usize = 1;
const TIMER_MOUSE_UP_CLOSE: usize = 2;
const COLOR_KEY: COLORREF = COLORREF(0x00FF00FF); // magenta = transparent

static START: OnceLock<Instant> = OnceLock::new();

macro_rules! log {
    ($($arg:tt)*) => {
        println!("[{:>8} ms] {}", START.get().unwrap().elapsed().as_millis(), format!($($arg)*))
    };
}

thread_local! {
    static HWND_WHEEL: Cell<HWND> = Cell::new(HWND::default());
    static INJECT: Cell<bool> = const { Cell::new(false) };
    static BUTTON_DOWN: Cell<bool> = const { Cell::new(false) };
    static DOWN_POS: Cell<POINT> = Cell::new(POINT::default());
    static DRAG_CANDIDATE: Cell<bool> = const { Cell::new(false) };
    static WHEEL_OPEN: Cell<bool> = const { Cell::new(false) };
    static GOT_ENTER: Cell<bool> = const { Cell::new(false) };
    static SHOWN_AT: Cell<Option<Instant>> = const { Cell::new(None) };
    static OVER_COUNT: Cell<u32> = const { Cell::new(0) };
    // keys whose key-down we swallowed, so we swallow their key-up too
    static SWALLOWED: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
}

fn since_shown_ms() -> u128 {
    SHOWN_AT.get().map(|t| t.elapsed().as_millis()).unwrap_or(0)
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        match wparam.0 as u32 {
            WM_LBUTTONDOWN => {
                BUTTON_DOWN.set(true);
                DOWN_POS.set(info.pt);
                DRAG_CANDIDATE.set(false);
            }
            WM_MOUSEMOVE if BUTTON_DOWN.get() && !DRAG_CANDIDATE.get() => {
                let d = DOWN_POS.get();
                let (cx, cy) = unsafe { (GetSystemMetrics(SM_CXDRAG), GetSystemMetrics(SM_CYDRAG)) };
                if (info.pt.x - d.x).abs() >= cx || (info.pt.y - d.y).abs() >= cy {
                    DRAG_CANDIDATE.set(true);
                }
            }
            WM_LBUTTONUP => {
                BUTTON_DOWN.set(false);
                DRAG_CANDIDATE.set(false);
                if WHEEL_OPEN.get() {
                    unsafe { PostMessageW(Some(HWND_WHEEL.get()), WM_MOUSE_UP, WPARAM(0), LPARAM(0)).ok() };
                }
            }
            _ => {}
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let vk = info.vkCode;
        let msg = wparam.0 as u32;
        let is_down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
        let is_up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
        let hwnd = Some(HWND_WHEEL.get());

        if is_up && SWALLOWED.with_borrow_mut(|s| {
            if let Some(i) = s.iter().position(|&k| k == vk) {
                s.remove(i);
                true
            } else {
                false
            }
        }) {
            return LRESULT(1);
        }

        if is_down {
            let swallow = if WHEEL_OPEN.get() {
                match VIRTUAL_KEY(vk as u16) {
                    VK_SPACE => true,
                    VK_TAB => {
                        unsafe { PostMessageW(hwnd, WM_TAB, WPARAM(0), LPARAM(0)).ok() };
                        true
                    }
                    VK_ESCAPE => {
                        unsafe { PostMessageW(hwnd, WM_ESC, WPARAM(0), LPARAM(0)).ok() };
                        true
                    }
                    _ => false,
                }
            } else if vk == VK_SPACE.0 as u32 && DRAG_CANDIDATE.get() {
                WHEEL_OPEN.set(true); // set now so key repeat is swallowed too
                unsafe { PostMessageW(hwnd, WM_OPEN_WHEEL, WPARAM(0), LPARAM(0)).ok() };
                true
            } else {
                false
            };
            if swallow {
                SWALLOWED.with_borrow_mut(|s| {
                    if !s.contains(&vk) {
                        s.push(vk)
                    }
                });
                return LRESULT(1);
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn open_wheel(hwnd: HWND) {
    let mut pt = POINT::default();
    unsafe { GetCursorPos(&mut pt).ok() };
    GOT_ENTER.set(false);
    OVER_COUNT.set(0);
    SHOWN_AT.set(Some(Instant::now()));
    unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            pt.x - WHEEL_SIZE / 2,
            pt.y - WHEEL_SIZE / 2,
            WHEEL_SIZE,
            WHEEL_SIZE,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
        .ok();
        SetTimer(Some(hwnd), TIMER_ENTER_TIMEOUT, 300, None);
    }
    log!("OPEN at ({}, {})", pt.x, pt.y);
    if INJECT.get() {
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 { mi: MOUSEINPUT { dx: 0, dy: 0, dwFlags: MOUSEEVENTF_MOVE, ..Default::default() } },
        };
        let sent = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
        log!("INJECT zero move (sent={sent})");
    }
}

fn close_wheel(hwnd: HWND, reason: &str) {
    if !WHEEL_OPEN.get() {
        return;
    }
    WHEEL_OPEN.set(false);
    unsafe {
        KillTimer(Some(hwnd), TIMER_ENTER_TIMEOUT).ok();
        KillTimer(Some(hwnd), TIMER_MOUSE_UP_CLOSE).ok();
        ShowWindow(hwnd, SW_HIDE).ok().ok();
    }
    log!("CLOSE ({reason}) after {} ms", since_shown_ms());
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_OPEN_WHEEL => open_wheel(hwnd),
        WM_TAB => log!("TAB"),
        WM_ESC => {
            log!("ESC");
            close_wheel(hwnd, "esc");
        }
        WM_MOUSE_UP => unsafe {
            // Give a pending Drop a chance to arrive first.
            SetTimer(Some(hwnd), TIMER_MOUSE_UP_CLOSE, 200, None);
        },
        WM_TIMER => match wparam.0 {
            TIMER_ENTER_TIMEOUT => {
                unsafe { KillTimer(Some(hwnd), TIMER_ENTER_TIMEOUT).ok() };
                if !GOT_ENTER.get() {
                    log!("TIMEOUT: no DragEnter within 300 ms");
                    close_wheel(hwnd, "timeout");
                }
            }
            TIMER_MOUSE_UP_CLOSE => close_wheel(hwnd, "mouse up outside"),
            _ => {}
        },
        WM_PAINT => unsafe {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            let bg = CreateSolidBrush(COLOR_KEY);
            FillRect(hdc, &ps.rcPaint, bg);
            DeleteObject(bg.into()).ok().ok();
            let fg = CreateSolidBrush(COLORREF(0x00208CFF)); // orange (BGR)
            let old = SelectObject(hdc, fg.into());
            Ellipse(hdc, 0, 0, WHEEL_SIZE, WHEEL_SIZE).ok().ok();
            SelectObject(hdc, old);
            DeleteObject(fg.into()).ok().ok();
            EndPaint(hwnd, &ps).ok().ok();
        },
        WM_DESTROY => unsafe { PostQuitMessage(0) },
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    LRESULT(0)
}

fn dropped_files(data: &IDataObject) -> Vec<String> {
    let fmt = FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    let mut files = Vec::new();
    unsafe {
        let Ok(mut medium) = data.GetData(&fmt) else {
            return files;
        };
        let hdrop = HDROP(medium.u.hGlobal.0);
        let count = DragQueryFileW(hdrop, u32::MAX, None);
        for i in 0..count {
            let len = DragQueryFileW(hdrop, i, None) as usize;
            let mut buf = vec![0u16; len + 1];
            DragQueryFileW(hdrop, i, Some(&mut buf));
            files.push(String::from_utf16_lossy(&buf[..len]));
        }
        ReleaseStgMedium(&mut medium);
    }
    files
}

#[implement(IDropTarget)]
struct DropTarget;

impl IDropTarget_Impl for DropTarget_Impl {
    fn DragEnter(
        &self,
        pdataobj: Ref<'_, IDataObject>,
        _grfkeystate: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        pdweffect: *mut DROPEFFECT,
    ) -> Result<()> {
        GOT_ENTER.set(true);
        let files = pdataobj.as_ref().map(dropped_files).unwrap_or_default();
        log!("DRAGENTER {} ms after show at ({}, {}) files={:?}", since_shown_ms(), pt.x, pt.y, files);
        unsafe { *pdweffect = DROPEFFECT_COPY };
        Ok(())
    }

    fn DragOver(&self, _grfkeystate: MODIFIERKEYS_FLAGS, pt: &POINTL, pdweffect: *mut DROPEFFECT) -> Result<()> {
        let n = OVER_COUNT.get() + 1;
        OVER_COUNT.set(n);
        if n % 10 == 1 {
            log!("DRAGOVER #{n} at ({}, {})", pt.x, pt.y);
        }
        unsafe { *pdweffect = DROPEFFECT_COPY };
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        log!("DRAGLEAVE");
        Ok(())
    }

    fn Drop(
        &self,
        pdataobj: Ref<'_, IDataObject>,
        _grfkeystate: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        pdweffect: *mut DROPEFFECT,
    ) -> Result<()> {
        let files = pdataobj.as_ref().map(dropped_files).unwrap_or_default();
        log!("DROP at ({}, {}) files={:?} -> returning DROPEFFECT_COPY", pt.x, pt.y, files);
        unsafe { *pdweffect = DROPEFFECT_COPY };
        close_wheel(HWND_WHEEL.get(), "drop");
        Ok(())
    }
}

fn main() -> Result<()> {
    START.set(Instant::now()).unwrap();
    INJECT.set(std::env::args().any(|a| a == "--inject"));
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2).ok();
        OleInitialize(None)?;

        let instance = GetModuleHandleW(None)?;
        let class = w!("DragWheelSpike");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance.into(),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class,
            w!("dragwheel spike"),
            WS_POPUP,
            0,
            0,
            WHEEL_SIZE,
            WHEEL_SIZE,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        SetLayeredWindowAttributes(hwnd, COLOR_KEY, 200, LWA_COLORKEY | LWA_ALPHA)?;
        HWND_WHEEL.set(hwnd);

        let target: IDropTarget = DropTarget.into();
        RegisterDragDrop(hwnd, &target)?;

        let _mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), Some(instance.into()), 0)?;
        let _keys = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), Some(instance.into()), 0)?;
        log!("ready (inject={}). Drag a file in Explorer and press Space. Ctrl+C to quit.", INJECT.get());

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}
