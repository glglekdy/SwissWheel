//! The progress card above the tray (spec 2.4): one row per running or
//! queued file, glass like the wheel, click-through, never focused, and
//! hidden when nothing runs.

use swisswheel_core::appearance::Accent;
use swisswheel_core::summary::{more_files, progress_value};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dwm::{DWMWA_USE_HOSTBACKDROPBRUSH, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, Result, w};

use crate::fonts;
use crate::jobs::ProgressRow;
use crate::surface::{Surface, draw_text, rounded};
use crate::theme::{Theme, glass_layer, rounded_shape};

/// Card layout in DIPs.
const WIDTH: f32 = 300.0;
const HEADER: f32 = 32.0;
const ROW: f32 = 42.0;
const MORE: f32 = 22.0;
const PAD: f32 = 14.0;
const BOTTOM: f32 = 8.0;
const CORNER: f32 = 10.0;
const BAR: f32 = 4.0;
/// Gap between the card and the edge of the work area.
const MARGIN: f32 = 12.0;
/// Rows listed before the rest collapse into "외 N개".
const MAX_ROWS: usize = 5;

pub struct ProgressWindow {
    hwnd: HWND,
    glass_supported: bool,
    surface: Option<Surface>,
    scale: f32,
    visible: bool,
}

impl ProgressWindow {
    /// `owner` is the app's host window, as for the wheel.
    pub fn new(owner: HWND) -> Result<ProgressWindow> {
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let class = w!("SwissWheelProgress");
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(wnd_proc),
                hInstance: instance.into(),
                lpszClassName: class,
                ..Default::default()
            });
            // LAYERED + TRANSPARENT: mouse input goes to whatever is beneath.
            let hwnd = CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP
                    | WS_EX_TOPMOST
                    | WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE
                    | WS_EX_LAYERED
                    | WS_EX_TRANSPARENT,
                class,
                w!("SwissWheel progress"),
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
            SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA)?;
            let enable = BOOL::from(true);
            let glass_supported = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_HOSTBACKDROPBRUSH,
                std::ptr::from_ref(&enable).cast(),
                size_of::<BOOL>() as u32,
            )
            .is_ok();
            Ok(ProgressWindow {
                hwnd,
                glass_supported,
                surface: None,
                scale: 1.0,
                visible: false,
            })
        }
    }

    /// Shows `rows`, or hides the card when there are none.
    pub fn update(&mut self, rows: &[ProgressRow], accent: Accent) {
        if rows.is_empty() {
            self.hide();
            return;
        }
        if !self.visible {
            self.place();
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            }
            self.visible = true;
        }
        let (scale, glass) = (self.scale, self.glass_supported);
        if let Some(surface) = &mut self.surface
            && let Err(e) = draw_card(surface, rows, accent, scale, glass)
        {
            log_error!("진행 창 그리기 실패: {e}");
            self.surface = None; // rebuilt on the next show
        }
    }

    /// Hides the card and frees its GPU resources (it is idle most of the time).
    pub fn hide(&mut self) {
        if self.visible {
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
            self.visible = false;
        }
        self.surface = None;
    }

    /// Bottom-right of the primary monitor's work area (where the tray is by
    /// default), at that monitor's DPI.
    fn place(&mut self) {
        unsafe {
            let monitor = MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY);
            let (mut dpi_x, mut dpi_y) = (96, 96);
            let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
            self.scale = dpi_x as f32 / 96.0;
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let _ = GetMonitorInfoW(monitor, &mut info);
            let wa = info.rcWork;
            let width = (WIDTH * self.scale).ceil() as i32;
            let height = (card_height(MAX_ROWS, true) * self.scale).ceil() as i32;
            let margin = (MARGIN * self.scale).round() as i32;
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                wa.right - margin - width,
                wa.bottom - margin - height,
                width,
                height,
                SWP_NOACTIVATE,
            );
            if self.surface.is_none() {
                match Surface::new(self.hwnd, width as u32, height as u32, 1) {
                    Ok(s) => self.surface = Some(s),
                    Err(e) => log_error!("진행 창을 만들 수 없습니다: {e}"),
                }
            }
        }
    }
}

/// The card's height in DIPs for `rows` rows, plus the "외 N개" line.
fn card_height(rows: usize, more: bool) -> f32 {
    HEADER + rows as f32 * ROW + if more { MORE } else { 0.0 } + BOTTOM
}

fn draw_card(
    surface: &Surface,
    rows: &[ProgressRow],
    accent: Accent,
    scale: f32,
    glass_supported: bool,
) -> Result<()> {
    let theme = Theme::current(glass_supported, accent, 1.0);
    let shown = rows.len().min(MAX_ROWS);
    let hidden = rows.len() - shown;
    let (w, h) = (surface.size().0 as f32, surface.size().1 as f32);
    // Anchored to the bottom so the card grows upward from the tray.
    let card = D2D_RECT_F {
        left: 0.0,
        top: h - card_height(shown, hidden > 0) * scale,
        right: w,
        bottom: h,
    };
    let corner = CORNER * scale;
    surface.set_glass(0, theme.glass.then_some(card), corner)?;

    let leading = fonts::trimmed_format(
        12.5 * scale,
        DWRITE_FONT_WEIGHT_SEMI_BOLD,
        DWRITE_TEXT_ALIGNMENT_LEADING,
    )?;
    let small_leading = fonts::trimmed_format(
        11.0 * scale,
        DWRITE_FONT_WEIGHT_SEMI_BOLD,
        DWRITE_TEXT_ALIGNMENT_LEADING,
    )?;
    let trailing = fonts::trimmed_format(
        11.0 * scale,
        DWRITE_FONT_WEIGHT_NORMAL,
        DWRITE_TEXT_ALIGNMENT_TRAILING,
    )?;
    let ctx = surface.ctx();
    unsafe {
        ctx.BeginDraw();
        ctx.Clear(Some(&D2D1_COLOR_F::default()));
        let brush = |color: &D2D1_COLOR_F| ctx.CreateSolidColorBrush(color, None);
        glass_layer(ctx, &rounded_shape(ctx, card, corner)?, card, &theme, scale)?;

        let pad = PAD * scale;
        let text_box = |top: f32, height: f32| D2D_RECT_F {
            left: card.left + pad,
            top,
            right: card.right - pad,
            bottom: top + height,
        };
        let header = format!("변환 중 · {}개", rows.len());
        draw_text(
            ctx,
            &header,
            &small_leading,
            text_box(card.top + 4.0 * scale, HEADER * scale - 4.0 * scale),
            &brush(&theme.subtext)?,
        );

        let track = brush(&theme.track)?;
        for (i, row) in rows.iter().take(shown).enumerate() {
            let top = card.top + (HEADER + i as f32 * ROW) * scale;
            let line = text_box(top, 20.0 * scale);
            draw_text(
                ctx,
                &row.title,
                &leading,
                D2D_RECT_F {
                    right: line.right - 44.0 * scale, // room for "100%"
                    ..line
                },
                &brush(&theme.text)?,
            );
            draw_text(
                ctx,
                &progress_value(row.fraction),
                &trailing,
                line,
                &brush(&theme.subtext)?,
            );
            let bar = D2D_RECT_F {
                left: line.left,
                top: top + 26.0 * scale,
                right: line.right,
                bottom: top + (26.0 + BAR) * scale,
            };
            let radius = BAR * scale / 2.0;
            ctx.FillRoundedRectangle(&rounded(bar, radius), &track);
            if let Some(p) = row.fraction {
                let filled = D2D_RECT_F {
                    right: bar.left + (bar.right - bar.left) * p.clamp(0.0, 1.0),
                    ..bar
                };
                ctx.FillRoundedRectangle(&rounded(filled, radius), &brush(&theme.accent)?);
            }
        }
        if hidden > 0 {
            let top = card.top + (HEADER + shown as f32 * ROW) * scale;
            draw_text(
                ctx,
                &more_files(hidden),
                &small_leading,
                text_box(top, MORE * scale),
                &brush(&theme.subtext)?,
            );
        }
        ctx.EndDraw(None, None)?;
    }
    surface.present()
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
            // Hand taskkill's WM_CLOSE to the host window, like the wheel does.
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
