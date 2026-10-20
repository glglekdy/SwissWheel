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

/// Callback message for tray events (`NOTIFYICON_VERSION_4` layout).
pub const WM_APP_TRAY: u32 = WM_APP + 10;
const ICON_ID: u32 = 1;

pub struct Tray {
    hwnd: HWND,
    idle: HICON,
    busy_icon: HICON,
    busy: bool,
    tooltip: String,
}

impl Tray {
    pub fn new(hwnd: HWND) -> windows::core::Result<Tray> {
        let size = unsafe { GetSystemMetrics(SM_CXSMICON) };
        let tray = Tray {
            hwnd,
            idle: icon::make(false, size)?,
            busy_icon: icon::make(true, size)?,
            busy: false,
            tooltip: "SwissWheel".into(),
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

    pub fn set_busy(&mut self, busy: bool, tooltip: &str) {
        self.busy = busy;
        self.tooltip = tooltip.to_string();
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &self.data());
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
            hIcon: if self.busy { self.busy_icon } else { self.idle },
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
            let _ = DestroyIcon(self.busy_icon);
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
