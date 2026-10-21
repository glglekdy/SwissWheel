//! The topmost, non-activating popup that hosts the desktop menu. It never
//! takes focus, so the desktop keeps its selection; keys arrive through the
//! keyboard hook and mouse input through this window's messages.

use std::cell::Cell;
use std::time::{Duration, Instant};

use swisswheel_core::geometry::Rect;
use swisswheel_core::menu_geometry::{Box2, MenuLayout, layout, window_origin};
use swisswheel_core::settings::ITEMS;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_USE_HOSTBACKDROPBRUSH, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateRoundRectRgn, DeleteObject, GetMonitorInfoW, HRGN, MONITOR_DEFAULTTONEAREST,
    MONITORINFO, MonitorFromPoint, RGN_OR, ScreenToClient, SetWindowRgn,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, w};

use super::render::{BAR_CORNER, PANEL_CORNER, draw};
use super::{MenuMouse, MenuView};
use crate::surface::Surface;

/// Not exported by the `windows` crate under WindowsAndMessaging.
const WM_MOUSELEAVE: u32 = 0x02A3;

thread_local! {
    /// A WM_MOUSELEAVE is requested for the current hover.
    static TRACKING: Cell<bool> = const { Cell::new(false) };
}

pub struct MenuBar {
    hwnd: HWND,
    /// The OS accepted `DWMWA_USE_HOSTBACKDROPBRUSH` (Windows 11 and later).
    glass_supported: bool,
    surface: Option<Surface>,
    scale: f32,
    layout: MenuLayout,
    /// The window's top-left on screen.
    origin: POINT,
    panel_open: bool,
    visible: bool,
    last_shown: Instant,
}

impl MenuBar {
    /// `owner` is the app's host window, as for the wheel.
    pub fn new(owner: HWND) -> windows::core::Result<MenuBar> {
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let class = w!("SwissWheelMenu");
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(wnd_proc),
                hInstance: instance.into(),
                lpszClassName: class,
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                ..Default::default()
            });
            let hwnd = CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class,
                w!("SwissWheel menu"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                Some(owner),
                None,
                Some(instance.into()),
                None,
            )?;
            let enable = BOOL::from(true);
            let glass_supported = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_HOSTBACKDROPBRUSH,
                std::ptr::from_ref(&enable).cast(),
                size_of::<BOOL>() as u32,
            )
            .is_ok();
            Ok(MenuBar {
                hwnd,
                glass_supported,
                surface: None,
                scale: 1.0,
                layout: layout(ITEMS.len(), false, 1.0),
                origin: POINT::default(),
                panel_open: false,
                visible: false,
                last_shown: Instant::now(),
            })
        }
    }

    pub fn layout(&self) -> &MenuLayout {
        &self.layout
    }

    pub fn panel_open(&self) -> bool {
        self.panel_open
    }

    /// Sizes and positions the (still hidden) window at the top center of
    /// the work area of the monitor under `cursor`, at that monitor's DPI.
    /// `notice` adds the "can't save" line to the panel.
    pub fn place(&mut self, cursor: POINT, notice: bool) {
        unsafe {
            let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
            let (mut dpi_x, mut dpi_y) = (96, 96);
            let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
            self.scale = dpi_x as f32 / 96.0;
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let _ = GetMonitorInfoW(monitor, &mut info);
            let wa = info.rcWork;
            let work = Rect {
                left: wa.left,
                top: wa.top,
                right: wa.right,
                bottom: wa.bottom,
            };
            self.layout = layout(ITEMS.len(), notice, self.scale);
            let (x, y) = window_origin(work, &self.layout, self.scale);
            self.origin = POINT { x, y };
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                x,
                y,
                self.layout.width,
                self.layout.height,
                SWP_NOACTIVATE,
            );
        }
        self.panel_open = true; // force the region update below
        self.set_panel(false);
    }

    /// Limits hit testing to the bar (with the strip it slides in from) and,
    /// while open, the panel; clicks elsewhere reach the desktop.
    pub fn set_panel(&mut self, open: bool) {
        if self.panel_open == open {
            return;
        }
        self.panel_open = open;
        let l = &self.layout;
        let bar = Box2 { top: 0.0, ..l.bar };
        unsafe {
            let region = round_rect_region(bar, BAR_CORNER * self.scale);
            if open {
                let panel = round_rect_region(l.panel, PANEL_CORNER * self.scale);
                CombineRgn(Some(region), Some(region), Some(panel), RGN_OR);
                let _ = DeleteObject(panel.into());
            }
            // The system owns the region from here on.
            SetWindowRgn(self.hwnd, Some(region), false);
        }
    }

    /// Whether a screen point is on the visible bar or panel.
    pub fn contains_screen(&self, x: i32, y: i32) -> bool {
        let (x, y) = ((x - self.origin.x) as f32, (y - self.origin.y) as f32);
        let l = &self.layout;
        Box2 { top: 0.0, ..l.bar }.contains(x, y) || (self.panel_open && l.panel.contains(x, y))
    }

    pub fn show(&mut self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
        }
        self.visible = true;
        self.last_shown = Instant::now();
    }

    pub fn hide(&mut self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        self.visible = false;
        TRACKING.set(false);
    }

    /// Draws a frame. `false` when there is nothing to draw with (the GPU
    /// surface can't be created): the window would show nothing, so the
    /// caller turns the menu off.
    pub fn render(&mut self, view: &MenuView) -> bool {
        let size = (self.layout.width as u32, self.layout.height as u32);
        if self.surface.as_ref().is_some_and(|s| s.size() != size) {
            self.surface = None;
        }
        if self.surface.is_none() {
            match Surface::new(self.hwnd, size.0, size.1, 2) {
                Ok(s) => self.surface = Some(s),
                Err(e) => {
                    log_error!("메뉴 렌더러를 만들 수 없습니다: {e}");
                    return false;
                }
            }
        }
        if let Some(surface) = &self.surface
            && let Err(e) = draw(
                surface,
                view,
                &self.layout,
                self.scale,
                self.glass_supported,
            )
        {
            // Device lost or similar: rebuild on the next frame.
            log_error!("메뉴 그리기 실패: {e}");
            self.surface = None;
        }
        true
    }

    /// Frees GPU resources when the menu has been unused for `idle`.
    pub fn release_if_idle(&mut self, idle: Duration) {
        if !self.visible && self.surface.is_some() && self.last_shown.elapsed() >= idle {
            self.surface = None;
            log_info!("메뉴 렌더러 해제 (미사용 {}분)", idle.as_secs() / 60);
        }
    }
}

fn round_rect_region(b: Box2, corner: f32) -> HRGN {
    let corner = (2.0 * corner).round() as i32;
    unsafe {
        CreateRoundRectRgn(
            b.left.floor() as i32,
            b.top.floor() as i32,
            b.right.ceil() as i32 + 1,
            b.bottom.ceil() as i32 + 1,
            corner,
            corner,
        )
    }
}

/// Client coordinates packed in a mouse message's `lParam`.
fn client_point(lparam: LPARAM) -> (f32, f32) {
    let x = (lparam.0 & 0xFFFF) as u16 as i16;
    let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16;
    (x as f32, y as f32)
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let event = match msg {
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_CLOSE => {
            // Hand taskkill's WM_CLOSE to the host window, like the wheel does.
            if let Ok(owner) = unsafe { GetWindow(hwnd, GW_OWNER) } {
                unsafe {
                    let _ = PostMessageW(Some(owner), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            }
            return LRESULT(0);
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
            MenuMouse::Move { x, y }
        }
        WM_MOUSELEAVE => {
            TRACKING.set(false);
            MenuMouse::Leave
        }
        WM_LBUTTONDOWN => {
            let (x, y) = client_point(lparam);
            MenuMouse::Click { x, y }
        }
        WM_MOUSEWHEEL => {
            // Screen coordinates here, unlike the other mouse messages.
            let (sx, sy) = client_point(lparam);
            let mut pt = POINT {
                x: sx as i32,
                y: sy as i32,
            };
            unsafe {
                let _ = ScreenToClient(hwnd, &mut pt);
            }
            MenuMouse::Wheel {
                x: pt.x as f32,
                y: pt.y as f32,
                delta: ((wparam.0 >> 16) & 0xFFFF) as u16 as i16,
            }
        }
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    };
    crate::app::on_menu_mouse(event);
    LRESULT(0)
}
