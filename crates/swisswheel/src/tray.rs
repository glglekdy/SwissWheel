//! The notification-area icon: idle/busy state, tooltip, and balloon notifications.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_INFO, NIIF_WARNING, NIM_ADD,
    NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NOTIFYICON_VERSION_4, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, GetSystemMetrics, HICON, SM_CXSMICON, WM_APP,
};

use crate::icon;
use crate::theme::taskbar_light_theme;

/// Callback message for tray events (`NOTIFYICON_VERSION_4` layout).
pub const WM_APP_TRAY: u32 = WM_APP + 10;
const ICON_ID: u32 = 1;

pub struct Tray {
    hwnd: HWND,
    idle: HICON,
    /// The taskbar is light: the logo is drawn black instead of white.
    light: bool,
    busy: bool,
    tooltip: String,
    icon_size: i32,
    /// The logo with a progress ring, at a 5% step (0..=20).
    ring: Option<(u8, HICON)>,
}

impl Tray {
    pub fn new(hwnd: HWND) -> windows::core::Result<Tray> {
        let size = unsafe { GetSystemMetrics(SM_CXSMICON) };
        let light = taskbar_light_theme();
        let tray = Tray {
            hwnd,
            idle: icon::make(light, size)?,
            light,
            busy: false,
            tooltip: "SwissWheel".into(),
            icon_size: size,
            ring: None,
        };
        tray.add();
        Ok(tray)
    }

    /// Adds the icon; also used to restore it after Explorer restarts.
    pub fn add(&self) {
        let mut data = self.data();
        unsafe {
            let _ = Shell_NotifyIconW(NIM_ADD, &data);
            data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &data);
        }
    }

    /// `ring` is the overall progress to draw around the logo, or `None` for
    /// the plain one. The icon is rebuilt only when it moves a 5% step.
    pub fn set_busy(&mut self, busy: bool, tooltip: &str, ring: Option<f32>) {
        self.busy = busy;
        self.tooltip = tooltip.to_string();
        let step = ring.map(|p| (p.clamp(0.0, 1.0) * 20.0).round() as u8);
        if step != self.ring.map(|(s, _)| s) {
            self.set_ring(step);
        }
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &self.data());
        }
    }

    /// Redraws the icon in the taskbar's current color if the Windows theme
    /// changed (white logo on a dark taskbar, black on a light one).
    pub fn refresh_theme(&mut self) {
        let light = taskbar_light_theme();
        if light == self.light {
            return;
        }
        self.light = light;
        match icon::make(light, self.icon_size) {
            Ok(idle) => unsafe {
                let _ = DestroyIcon(std::mem::replace(&mut self.idle, idle));
            },
            Err(e) => log_error!("트레이 아이콘을 만들 수 없습니다: {e}"),
        }
        self.set_ring(self.ring.map(|(s, _)| s));
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &self.data());
        }
    }

    fn set_ring(&mut self, step: Option<u8>) {
        if let Some((_, old)) = self.ring.take() {
            unsafe {
                let _ = DestroyIcon(old);
            }
        }
        if let Some(step) = step {
            match icon::make_progress(f32::from(step) / 20.0, self.light, self.icon_size) {
                Ok(hicon) => self.ring = Some((step, hicon)),
                Err(e) => log_error!("진행 아이콘을 만들 수 없습니다: {e}"),
            }
        }
    }

    pub fn notify(&self, title: &str, body: &str) {
        self.balloon(title, body, false);
    }

    pub fn warn(&self, title: &str, body: &str) {
        self.balloon(title, body, true);
    }

    fn balloon(&self, title: &str, body: &str, warning: bool) {
        let mut data = self.data();
        data.uFlags |= NIF_INFO;
        data.dwInfoFlags = if warning { NIIF_WARNING } else { NIIF_INFO };
        copy_wide(&mut data.szInfoTitle, title);
        copy_wide(&mut data.szInfo, body);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }

    fn data(&self) -> NOTIFYICONDATAW {
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: ICON_ID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: WM_APP_TRAY,
            hIcon: match (self.busy, self.ring) {
                (true, Some((_, ring))) => ring,
                _ => self.idle,
            },
            ..Default::default()
        };
        copy_wide(&mut data.szTip, &self.tooltip);
        data
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.data());
            let _ = DestroyIcon(self.idle);
            if let Some((_, ring)) = self.ring {
                let _ = DestroyIcon(ring);
            }
        }
    }
}

/// Copies `s` into a fixed UTF-16 buffer, truncating and null-terminating.
fn copy_wide(dst: &mut [u16], s: &str) {
    let max = dst.len() - 1;
    let mut n = 0;
    for unit in s.encode_utf16().take(max) {
        dst[n] = unit;
        n += 1;
    }
    dst[n] = 0;
}
