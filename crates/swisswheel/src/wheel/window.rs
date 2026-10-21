//! The topmost, non-activating popup that hosts the wheel, its option panel,
//! and the drop target.

use std::time::{Duration, Instant};

use swisswheel_core::geometry::{
    OUTER_RADIUS, PANEL_OVERLAP, PANEL_WIDTH, Rect, Side, clamp_center, panel_rect, panel_side,
};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_USE_HOSTBACKDROPBRUSH, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateEllipticRgn, CreateRoundRectRgn, DeleteObject, GetMonitorInfoW,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, RGN_OR, SetWindowRgn,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, w};

use super::WheelView;
use super::render::{PANEL_CORNER, Renderer};

/// Hovered sectors grow outward by this much (DIPs); the window leaves room for it.
pub(super) const HOVER_GROW: f32 = 6.0;

pub struct WheelWindow {
    hwnd: HWND,
    /// The OS accepted `DWMWA_USE_HOSTBACKDROPBRUSH` (Windows 11 and later).
    glass_supported: bool,
    renderer: Option<Renderer>,
    scale: f32,
    /// Wheel center on screen.
    center: POINT,
    /// Wheel center inside the window.
    origin: POINT,
    width: i32,
    height: i32,
    side: Side,
    /// Rows of the open panel, as the window region currently has it.
    panel_rows: Option<usize>,
    visible: bool,
    last_shown: Instant,
}

impl WheelWindow {
    /// `owner` is the app's host window: the wheel is destroyed with it and
    /// forwards WM_CLOSE to it (see `wnd_proc`).
    pub fn new(owner: HWND) -> windows::core::Result<WheelWindow> {
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
                Some(owner), // for a WS_POPUP this sets the owner, not a parent
                None,
                Some(instance.into()),
                None,
            )?;
            // A classic Win32 window must opt in before Windows.UI.Composition
            // host-backdrop brushes (the glass blur) draw anything (Windows 11+).
            let enable = BOOL::from(true);
            let glass_supported = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_HOSTBACKDROPBRUSH,
                std::ptr::from_ref(&enable).cast(),
                size_of::<BOOL>() as u32,
            )
            .is_ok();
            if !glass_supported {
                log_info!("이 Windows 버전은 유리 효과를 지원하지 않아 불투명 휠을 사용합니다");
            }
            Ok(WheelWindow {
                hwnd,
                glass_supported,
                renderer: None,
                scale: 1.0,
                center: POINT::default(),
                origin: POINT::default(),
                width: 1,
                height: 1,
                side: Side::Right,
                panel_rows: None,
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

    pub fn panel_side(&self) -> Side {
        self.side
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Positions the (still hidden) window around `cursor`, kept inside the
    /// work area of the cursor's monitor, at that monitor's DPI. The window
    /// is wide enough for the option panel on whichever side it will open.
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
            let work = Rect {
                left: wa.left,
                top: wa.top,
                right: wa.right,
                bottom: wa.bottom,
            };
            let (cx, cy) = clamp_center((cursor.x, cursor.y), work, self.scale);
            self.center = POINT { x: cx, y: cy };
            self.side = panel_side(cx, work, self.scale);

            let radius = ((OUTER_RADIUS + HOVER_GROW) * self.scale).ceil() as i32;
            let panel_reach =
                ((OUTER_RADIUS - PANEL_OVERLAP + PANEL_WIDTH) * self.scale).ceil() as i32;
            let extra = (panel_reach - radius).max(0);
            self.width = radius * 2 + extra;
            self.height = radius * 2;
            self.origin = match self.side {
                Side::Right => POINT {
                    x: radius,
                    y: radius,
                },
                Side::Left => POINT {
                    x: radius + extra,
                    y: radius,
                },
            };
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                cx - self.origin.x,
                cy - self.origin.y,
                self.width,
                self.height,
                SWP_NOACTIVATE,
            );
            self.panel_rows = Some(usize::MAX); // force the region update below
            self.set_panel(None);
        }
    }

    /// Limits hit testing (and so the drop target) to the disc, plus the
    /// panel while one is open; elsewhere drops reach the window beneath.
    pub fn set_panel(&mut self, rows: Option<usize>) {
        if self.panel_rows == rows {
            return;
        }
        self.panel_rows = rows;
        let radius = ((OUTER_RADIUS + HOVER_GROW) * self.scale).ceil() as i32;
        let (ox, oy) = (self.origin.x, self.origin.y);
        unsafe {
            let region =
                CreateEllipticRgn(ox - radius, oy - radius, ox + radius + 1, oy + radius + 1);
            if let Some(rows) = rows {
                let r = panel_rect(self.side, rows, self.scale);
                let corner = (2.0 * PANEL_CORNER * self.scale).round() as i32;
                let panel = CreateRoundRectRgn(
                    ox + r.left.floor() as i32,
                    oy + r.top.floor() as i32,
                    ox + r.right.ceil() as i32 + 1,
                    oy + r.bottom.ceil() as i32 + 1,
                    corner,
                    corner,
                );
                CombineRgn(Some(region), Some(region), Some(panel), RGN_OR);
                let _ = DeleteObject(panel.into());
            }
            // The system owns the region from here on.
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

    /// Shows nothing while the window waits for DragEnter, so a drag that turns
    /// out not to carry files never flashes the wheel. Also wipes the frame
    /// left over from the previous open.
    pub fn clear(&mut self) {
        self.with_renderer(Renderer::clear);
    }

    pub fn render(&mut self, view: &WheelView) {
        let scale = self.scale;
        let glass = self.glass_supported;
        let origin = (self.origin.x as f32, self.origin.y as f32);
        self.with_renderer(|r| r.draw(view, scale, glass, origin));
    }

    fn with_renderer(&mut self, f: impl FnOnce(&mut Renderer) -> windows::core::Result<()>) {
        let size = (self.width as u32, self.height as u32);
        if self.renderer.as_ref().is_some_and(|r| r.size() != size) {
            self.renderer = None;
        }
        if self.renderer.is_none() {
            match Renderer::new(self.hwnd, size.0, size.1) {
                Ok(r) => self.renderer = Some(r),
                Err(e) => {
                    log_error!("휠 렌더러를 만들 수 없습니다: {e}");
                    return;
                }
            }
        }
        if let Some(r) = &mut self.renderer
            && let Err(e) = f(r)
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
        WM_CLOSE => {
            // taskkill sends WM_CLOSE to the first top-level window it finds,
            // which may be this one. Destroying only the wheel would leave the
            // app running without it, so hand the request to the host window.
            if let Ok(owner) = unsafe { GetWindow(hwnd, GW_OWNER) } {
                unsafe {
                    let _ = PostMessageW(Some(owner), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
