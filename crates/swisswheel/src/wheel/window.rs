//! The topmost, non-activating popup that hosts the wheel and the drop target.

use std::time::{Duration, Instant};

use swisswheel_core::geometry::{OUTER_RADIUS, Rect, clamp_center};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateEllipticRgn, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
    SetWindowRgn,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

use super::WheelView;
use super::render::Renderer;

/// Hovered sectors grow outward by this much (DIPs); the window leaves room for it.
pub(super) const HOVER_GROW: f32 = 6.0;

pub struct WheelWindow {
    hwnd: HWND,
    renderer: Option<Renderer>,
    scale: f32,
    center: POINT,
    size: i32,
    visible: bool,
    last_shown: Instant,
}

impl WheelWindow {
    pub fn new() -> windows::core::Result<WheelWindow> {
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let class = w!("SwissWheelWheel");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wnd_proc),
                hInstance: instance.into(),
                lpszClassName: class,
                ..Default::default()
            };
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class,
                w!("SwissWheel wheel"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(instance.into()),
                None,
            )?;
            Ok(WheelWindow {
                hwnd,
                renderer: None,
                scale: 1.0,
                center: POINT::default(),
                size: 1,
                visible: false,
                last_shown: Instant::now(),
            })
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    pub fn center(&self) -> POINT {
        self.center
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Positions the (still hidden) window around `cursor`, kept inside the
    /// work area of the cursor's monitor, at that monitor's DPI.
    pub fn place_at(&mut self, cursor: POINT) {
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
            let (cx, cy) = clamp_center(
                (cursor.x, cursor.y),
                Rect {
                    left: wa.left,
                    top: wa.top,
                    right: wa.right,
                    bottom: wa.bottom,
                },
                self.scale,
            );
            self.center = POINT { x: cx, y: cy };
            let radius = ((OUTER_RADIUS + HOVER_GROW) * self.scale).ceil() as i32;
            self.size = radius * 2;
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                cx - radius,
                cy - radius,
                self.size,
                self.size,
                SWP_NOACTIVATE,
            );
            // The region limits hit testing (and so the drop target) to the disc.
            let region = CreateEllipticRgn(0, 0, self.size + 1, self.size + 1);
            SetWindowRgn(self.hwnd, Some(region), false);
        }
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
    }

    pub fn render(&mut self, view: &WheelView) {
        let size = self.size as u32;
        if self.renderer.as_ref().is_some_and(|r| r.size() != size) {
            self.renderer = None;
        }
        if self.renderer.is_none() {
            match Renderer::new(self.hwnd, size) {
                Ok(r) => self.renderer = Some(r),
                Err(e) => {
                    log_error!("휠 렌더러를 만들 수 없습니다: {e}");
                    return;
                }
            }
        }
        if let Some(r) = &mut self.renderer
            && let Err(e) = r.draw(view, self.scale)
        {
            // Device lost or similar: rebuild on the next frame.
            log_error!("휠 그리기 실패: {e}");
            self.renderer = None;
        }
    }

    /// Frees GPU resources when the wheel has been unused for `idle` (spec 4.3).
    pub fn release_if_idle(&mut self, idle: Duration) {
        if !self.visible && self.renderer.is_some() && self.last_shown.elapsed() >= idle {
            self.renderer = None;
            log_info!("휠 렌더러 해제 (미사용 {}분)", idle.as_secs() / 60);
        }
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
