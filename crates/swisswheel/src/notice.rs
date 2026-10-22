//! The notice window above the tray (spec 2.4, 2.6): the progress card at
//! the bottom and notice cards stacked above it, the newest nearest. It
//! never takes focus; only the cards (with their shadow) take the mouse,
//! everything else beside them reaches the window below. The progress card
//! alone lets clicks through, as it always has.

use std::cell::{Cell, RefCell};

use swisswheel_core::appearance::Accent;
use swisswheel_core::notices::{CardView, NoticeId, NoticeKind};
use swisswheel_core::summary::{more_files, progress_value};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dwm::{DWMWA_USE_HOSTBACKDROPBRUSH, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateRoundRectRgn, DeleteObject, GetMonitorInfoW, HRGN, MONITOR_DEFAULTTOPRIMARY,
    MONITORINFO, MonitorFromPoint, RGN_ERROR, RGN_OR, ScreenToClient, SetWindowRgn,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::Shell::{
    QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, Interface, Result, w};
use windows_numerics::Vector2;

use crate::fonts;
use crate::jobs::ProgressRow;
use crate::surface::{Surface, draw_text, rounded};
use crate::theme::{Piece, Theme, glass_layer, glass_piece, outer_shadow, rounded_shape};

/// Posted to the owner window: `wparam` 0 = the hovered card changed
/// (`lparam` = its id, or -1 for none), 1 = a card was clicked (`lparam` = id).
pub const WM_APP_NOTICE: u32 = WM_APP + 33;

/// Card shape (DIPs), shared by the progress card and the notice cards.
const WIDTH: f32 = 300.0;
const CORNER: f32 = 18.0;
const PAD_X: f32 = 14.0;
const PAD_Y: f32 = 12.0;
/// Between stacked cards.
const GAP: f32 = 8.0;
/// Between the cards and the edge of the work area.
const MARGIN: f32 = 12.0;
/// Room around the cards for their outer shadow.
const SHADOW_MARGIN: f32 = 10.0;
/// How far a new card slides in from the right.
const SLIDE: f32 = 24.0;

/// Progress card rows (DIPs).
const HEADER: f32 = 32.0;
const ROW: f32 = 42.0;
const MORE: f32 = 22.0;
const BOTTOM: f32 = 8.0;
const BAR: f32 = 5.0;
/// Rows listed before the rest collapse into "외 N개".
const MAX_ROWS: usize = 5;

/// Notice card parts (DIPs).
const ICON: f32 = 30.0;
const ICON_GAP: f32 = 12.0;
const TITLE: f32 = 20.0;
const BODY_LINE: f32 = 17.0;
const BODY_MAX_LINES: usize = 2;
const ACTION: f32 = 19.0;
/// The timer bar, the space above it, and below it to the card's edge.
const TIMER: f32 = 2.0;
const TIMER_ABOVE: f32 = 10.0;
const TIMER_BELOW: f32 = 6.0;

/// Cards drawn at once: three live ones plus those fading out.
const MAX_DRAWN: usize = 6;
/// The tallest stack the window has room for.
const MAX_STACK: f32 = card_height(MAX_ROWS, true) + 4.0 * (GAP + notice_height(2, true));

/// Not exported by the `windows` crate under WindowsAndMessaging.
const WM_MOUSELEAVE: u32 = 0x02A3;

thread_local! {
    /// The cards under the mouse, in window pixels, in `hit_list` order.
    static HITS: RefCell<Vec<(NoticeId, D2D_RECT_F)>> = const { RefCell::new(Vec::new()) };
    /// The card last reported as hovered.
    static HOVER: Cell<Option<NoticeId>> = const { Cell::new(None) };
    /// A WM_MOUSELEAVE is requested for the current hover.
    static TRACKING: Cell<bool> = const { Cell::new(false) };
}

/// Full screen, presentation mode, or similar: notices should wait.
#[expect(
    dead_code,
    reason = "the app polls it once notices go through the cards"
)]
pub fn quiet_now() -> bool {
    matches!(
        unsafe { SHQueryUserNotificationState() },
        Ok(QUNS_BUSY | QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE)
    )
}

pub struct NoticeWindow {
    hwnd: HWND,
    glass_supported: bool,
    surface: Option<Surface>,
    scale: f32,
    /// `SHADOW_MARGIN` in whole pixels.
    margin: f32,
    size: (i32, i32),
    visible: bool,
    /// The window region last set, to skip setting it again every frame.
    region: Vec<[i32; 4]>,
    /// `WS_EX_TRANSPARENT` is set: only the progress card shows, and clicks
    /// pass through it as they always have.
    click_through: bool,
}

/// One slot of the stack, in window pixels.
struct Placed<'a> {
    rect: D2D_RECT_F,
    /// The notice, or `None` for the progress card.
    card: Option<&'a CardView>,
    body: &'a [String],
    opacity: f32,
    /// How much of the content shows (a card growing out of the progress
    /// card fades its content in).
    reveal: f32,
}

impl Placed<'_> {
    fn closing(&self) -> bool {
        self.card.is_some_and(|c| c.closing.is_some())
    }
}

impl NoticeWindow {
    /// `owner` is the app's host window; it receives `WM_APP_NOTICE`.
    pub fn new(owner: HWND) -> Result<NoticeWindow> {
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let class = w!("SwissWheelNotice");
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(wnd_proc),
                hInstance: instance.into(),
                lpszClassName: class,
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                ..Default::default()
            });
            // LAYERED so that adding TRANSPARENT lets clicks through (see
            // `set_click_through`); fully opaque, the swap chain draws everything.
            let hwnd = CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP
                    | WS_EX_TOPMOST
                    | WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE
                    | WS_EX_LAYERED,
                class,
                w!("SwissWheel notices"),
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
            Ok(NoticeWindow {
                hwnd,
                glass_supported,
                surface: None,
                scale: 1.0,
                margin: 0.0,
                size: (1, 1),
                visible: false,
                region: Vec::new(),
                click_through: false,
            })
        }
    }

    /// Draws the progress card (when `progress` has rows) under `cards`
    /// (newest first), or hides the window when there is neither.
    pub fn render(
        &mut self,
        progress: &[ProgressRow],
        cards: &[CardView],
        accent: Accent,
    ) -> Result<()> {
        if progress.is_empty() && cards.is_empty() {
            self.hide();
            return Ok(());
        }
        if !self.visible {
            self.place();
        }
        self.set_click_through(cards.is_empty());
        if self.surface.is_none() {
            let (w, h) = (self.size.0 as u32, self.size.1 as u32);
            self.surface = Some(Surface::new(self.hwnd, w, h, 1 + MAX_DRAWN)?);
        }
        if let Err(e) = self.draw(progress, cards, accent) {
            self.surface = None; // device lost or similar: rebuilt next time
            return Err(e);
        }
        if !self.visible {
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            }
            self.visible = true;
        }
        self.rehover();
        Ok(())
    }

    /// Cards slide under a cursor that isn't moving, which sends no
    /// WM_MOUSEMOVE: test the cursor against the cards just drawn.
    fn rehover(&self) {
        let mut pt = POINT::default();
        let id = unsafe {
            if GetCursorPos(&mut pt).is_err() || WindowFromPoint(pt) != self.hwnd {
                None
            } else {
                let _ = ScreenToClient(self.hwnd, &mut pt);
                hit(pt.x as f32, pt.y as f32)
            }
        };
        set_hover(self.hwnd, id);
    }

    /// Lets clicks through the whole window (`on`) or only beside the cards.
    fn set_click_through(&mut self, on: bool) {
        if self.click_through == on {
            return;
        }
        unsafe {
            let style = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE);
            let flag = WS_EX_TRANSPARENT.0 as isize;
            let style = if on { style | flag } else { style & !flag };
            SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, style);
        }
        self.click_through = on;
    }

    /// Hides the window and frees its GPU resources (it is idle most of the time).
    pub fn hide(&mut self) {
        if self.visible {
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
            self.visible = false;
        }
        self.surface = None;
        self.region.clear();
        HITS.with_borrow_mut(Vec::clear);
        if HOVER.take().is_some() {
            post_to_owner(self.hwnd, 0, -1);
        }
        TRACKING.set(false);
    }

    /// Bottom-right of the primary monitor's work area (where the tray is by
    /// default), at that monitor's DPI, tall enough for the fullest stack.
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
            self.margin = (SHADOW_MARGIN * self.scale).round();
            let m = self.margin as i32;
            let width = (WIDTH * self.scale).round() as i32 + 2 * m;
            let height = (MAX_STACK * self.scale).ceil() as i32 + 2 * m;
            let edge = (MARGIN * self.scale).round() as i32;
            if self.size != (width, height) {
                self.surface = None;
            }
            self.size = (width, height);
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                wa.right - edge - width + m,
                wa.bottom - edge - height + m,
                width,
                height,
                SWP_NOACTIVATE,
            );
        }
    }

    fn draw(&mut self, progress: &[ProgressRow], cards: &[CardView], accent: Accent) -> Result<()> {
        let scale = self.scale;
        let fonts = Formats::new(scale)?;
        let column = (WIDTH - 2.0 * PAD_X - ICON - ICON_GAP) * scale;
        let bodies: Vec<(NoticeId, Vec<String>)> = cards
            .iter()
            .map(|c| Ok((c.id, body_lines(&c.body, &fonts.wrap, column)?)))
            .collect::<Result<_>>()?;
        let body_of = |id: NoticeId| {
            bodies
                .iter()
                .find(|b| b.0 == id)
                .map_or(&[][..], |b| b.1.as_slice())
        };

        let shown = progress.len().min(MAX_ROWS);
        let more = progress.len() > shown;
        let bottom = self.size.1 as f32 - self.margin;
        let placed = arrange(shown, more, cards, body_of, self.margin, bottom, scale);
        self.set_region(&placed);
        HITS.set(hit_list(&placed));

        let Some(surface) = &self.surface else {
            return Ok(());
        };
        let base = Theme::current(self.glass_supported, accent, 1.0);
        let corner = CORNER * scale;
        for i in 0..1 + MAX_DRAWN {
            let p = placed.get(i).filter(|_| base.glass);
            surface.set_glass(i, p.map(|p| p.rect), corner)?;
            if let Some(p) = p {
                surface.set_glass_opacity(i, p.opacity)?;
            }
        }
        let ctx = surface.ctx();
        unsafe {
            ctx.BeginDraw();
            ctx.Clear(Some(&D2D1_COLOR_F::default()));
            for p in &placed {
                let theme = Theme::new(base.light, base.glass, accent, p.opacity);
                let shape = rounded_shape(ctx, p.rect, corner)?;
                outer_shadow(ctx, &shape, &theme, scale)?;
                glass_layer(ctx, &shape, p.rect, &theme, scale)?;
                let content = Theme::new(base.light, base.glass, accent, p.opacity * p.reveal);
                // A card growing out of the progress card is shorter than its content.
                ctx.PushAxisAlignedClip(&p.rect, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                let drawn = match p.card {
                    None => draw_progress(ctx, &fonts, progress, p.rect, &content, scale),
                    Some(card) => {
                        draw_notice(surface, &fonts, card, p.body, p.rect, &content, scale)
                    }
                };
                ctx.PopAxisAlignedClip();
                drawn?;
            }
            ctx.EndDraw(None, None)?;
        }
        surface.present()
    }

    /// Limits the window to the cards and their shadow; clicks beside them
    /// reach the window below.
    fn set_region(&mut self, placed: &[Placed]) {
        let m = self.margin;
        let rects: Vec<[i32; 4]> = placed
            .iter()
            .map(|p| {
                [
                    (p.rect.left - m).floor() as i32,
                    (p.rect.top - m).floor() as i32,
                    (p.rect.right + m).ceil() as i32 + 1,
                    (p.rect.bottom + m).ceil() as i32 + 1,
                ]
            })
            .collect();
        if rects == self.region {
            return;
        }
        let corner = (2.0 * (CORNER * self.scale + m)).round() as i32;
        let Some(region) = union_region(&rects, corner) else {
            return; // out of GDI resources: tried again next frame
        };
        unsafe {
            // On success the system owns the region from here on.
            if SetWindowRgn(self.hwnd, Some(region), false) == 0 {
                let _ = DeleteObject(region.into());
                return;
            }
        }
        self.region = rects;
    }
}

/// Rounded rectangles `[left, top, right, bottom]` joined into one region,
/// or `None` (with nothing leaked) if GDI fails.
fn union_region(rects: &[[i32; 4]], corner: i32) -> Option<HRGN> {
    unsafe {
        let region = CreateRoundRectRgn(0, 0, 0, 0, 0, 0);
        if region.is_invalid() {
            return None;
        }
        for &[l, t, r, b] in rects {
            let part = CreateRoundRectRgn(l, t, r, b, corner, corner);
            let joined = !part.is_invalid()
                && CombineRgn(Some(region), Some(region), Some(part), RGN_OR) != RGN_ERROR;
            if !part.is_invalid() {
                let _ = DeleteObject(part.into());
            }
            if !joined {
                let _ = DeleteObject(region.into());
                return None;
            }
        }
        Some(region)
    }
}

/// The stack in window pixels, ending at `bottom` with cards starting at
/// `left`, in paint order: fading cards first, so the others paint over them.
fn arrange<'a>(
    progress_rows: usize,
    more: bool,
    cards: &'a [CardView],
    body_of: impl Fn(NoticeId) -> &'a [String],
    left: f32,
    bottom: f32,
    scale: f32,
) -> Vec<Placed<'a>> {
    let px = |dip: f32| (dip * scale).round();
    let mut placed: Vec<Placed> = stack(progress_rows, more, cards, |c| body_of(c.id).len())
        .into_iter()
        .map(|(slot, top, height)| {
            let card = match slot {
                Slot::Progress => None,
                Slot::Card(id) => cards.iter().find(|c| c.id == id),
            };
            let m = card.map_or(
                Motion {
                    dx: 0.0,
                    opacity: 1.0,
                    reveal: 1.0,
                },
                motion,
            );
            let left = left + px(m.dx);
            Placed {
                rect: D2D_RECT_F {
                    left,
                    top: bottom - px(top),
                    right: left + px(WIDTH),
                    bottom: bottom - px(top - height),
                },
                card,
                body: card.map_or(&[][..], |c| body_of(c.id)),
                opacity: m.opacity,
                reveal: m.reveal,
            }
        })
        .collect();
    placed.truncate(1 + MAX_DRAWN);
    placed.sort_by_key(|p| !p.closing());
    placed
}

/// The cards in the order a point is tried against them: fading cards
/// first, so a click where one fades never reaches the card sliding into
/// its place (and is ignored); then the others, topmost painted first.
fn hit_list(placed: &[Placed]) -> Vec<(NoticeId, D2D_RECT_F)> {
    let (fading, live): (Vec<&Placed>, Vec<&Placed>) = placed
        .iter()
        .filter(|p| p.card.is_some())
        .partition(|p| p.closing());
    fading
        .iter()
        .rev()
        .chain(live.iter().rev())
        .filter_map(|p| Some((p.card?.id, p.rect)))
        .collect()
}

/// The first card in `hits` containing `(x, y)`.
fn hit_at(hits: &[(NoticeId, D2D_RECT_F)], x: f32, y: f32) -> Option<NoticeId> {
    hits.iter()
        .find(|(_, r)| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
        .map(|&(id, _)| id)
}

/// What occupies a place in the stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Progress,
    Card(NoticeId),
}

/// The progress card's height for `rows` rows, plus the "외 N개" line.
const fn card_height(rows: usize, more: bool) -> f32 {
    HEADER + rows as f32 * ROW + if more { MORE } else { 0.0 } + BOTTOM
}

/// A notice card's height with `body_lines` lines of body text.
const fn notice_height(body_lines: usize, action: bool) -> f32 {
    let text = TITLE + body_lines as f32 * BODY_LINE + if action { ACTION } else { 0.0 };
    let content = if text > ICON { text } else { ICON };
    PAD_Y + content + TIMER_ABOVE + TIMER + TIMER_BELOW
}

/// Each slot with its top edge and height in DIPs, measured up from the
/// stack's bottom: the progress card (`progress_rows` rows) at the bottom,
/// then `cards` (newest first) above it. Appearing and closing cards take
/// only part of their room, so the cards above slide.
fn stack(
    progress_rows: usize,
    more: bool,
    cards: &[CardView],
    body_lines: impl Fn(&CardView) -> usize,
) -> Vec<(Slot, f32, f32)> {
    let mut slots = Vec::with_capacity(cards.len() + 1);
    let mut bottom = 0.0;
    if progress_rows > 0 {
        let height = card_height(progress_rows, more);
        slots.push((Slot::Progress, height, height));
        bottom = height + GAP;
    }
    // A card taking the progress card's slot starts at its height: the one
    // row of the job that just finished, or what is still listed.
    let morph_from = card_height(progress_rows.max(1), more);
    for card in cards {
        let own = notice_height(body_lines(card), card.action_label.is_some());
        let (height, shown) = match card.from_progress {
            Some(f) => (morph_from + (own - morph_from) * f, 1.0),
            None => (own, card.appear),
        };
        slots.push((Slot::Card(card.id), bottom + height, height));
        bottom += (height + GAP) * shown * (1.0 - card.closing.unwrap_or(0.0));
    }
    slots
}

/// How a card moves and fades.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Motion {
    /// Offset to the right, DIPs.
    dx: f32,
    opacity: f32,
    /// The share of `opacity` the content gets.
    reveal: f32,
}

fn motion(card: &CardView) -> Motion {
    let fade = 1.0 - card.closing.unwrap_or(0.0);
    match card.from_progress {
        // Growing out of the progress card: in place, its content fading in.
        Some(f) => Motion {
            dx: 0.0,
            opacity: fade,
            reveal: f,
        },
        None => Motion {
            dx: (1.0 - card.appear) * SLIDE,
            opacity: card.appear * fade,
            reveal: 1.0,
        },
    }
}

/// Text formats for one frame.
struct Formats {
    header: IDWriteTextFormat,
    row: IDWriteTextFormat,
    value: IDWriteTextFormat,
    title: IDWriteTextFormat,
    body: IDWriteTextFormat,
    /// The body's size, wrapping, to find its line breaks.
    wrap: IDWriteTextFormat,
    action: IDWriteTextFormat,
}

impl Formats {
    fn new(scale: f32) -> Result<Formats> {
        use fonts::trimmed_format as trimmed;
        let leading = DWRITE_TEXT_ALIGNMENT_LEADING;
        Ok(Formats {
            header: trimmed(11.0 * scale, DWRITE_FONT_WEIGHT_SEMI_BOLD, leading)?,
            row: trimmed(12.5 * scale, DWRITE_FONT_WEIGHT_SEMI_BOLD, leading)?,
            value: trimmed(
                11.0 * scale,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_TEXT_ALIGNMENT_TRAILING,
            )?,
            title: trimmed(13.5 * scale, DWRITE_FONT_WEIGHT_BOLD, leading)?,
            body: trimmed(12.0 * scale, DWRITE_FONT_WEIGHT_NORMAL, leading)?,
            wrap: wrapping_format(12.0 * scale)?,
            action: trimmed(11.5 * scale, DWRITE_FONT_WEIGHT_SEMI_BOLD, leading)?,
        })
    }
}

/// Body text at `size`, wrapping at the box's width.
fn wrapping_format(size: f32) -> Result<IDWriteTextFormat> {
    let format = fonts::text_format(
        size,
        DWRITE_FONT_WEIGHT_NORMAL,
        DWRITE_TEXT_ALIGNMENT_LEADING,
    )?;
    unsafe { format.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)? };
    Ok(format)
}

/// `body` broken into the lines it takes in `width` pixels, at most two.
/// The last one carries the rest of the text (paragraphs joined by spaces)
/// for its single-line format to cut with "…".
fn body_lines(body: &str, wrap: &IDWriteTextFormat, width: f32) -> Result<Vec<String>> {
    let wide: Vec<u16> = body.trim().encode_utf16().collect();
    if wide.is_empty() {
        return Ok(Vec::new());
    }
    let mut lines = Vec::new();
    unsafe {
        let layout = fonts::factory().CreateTextLayout(&wide, wrap, width, f32::MAX)?;
        let mut metrics = DWRITE_TEXT_METRICS::default();
        layout.GetMetrics(&mut metrics)?;
        // The buffer must hold every line, or nothing is filled in.
        lines.resize(metrics.lineCount as usize, DWRITE_LINE_METRICS::default());
        let mut count = 0;
        layout.GetLineMetrics(Some(&mut lines), &mut count)?;
        lines.truncate(count as usize);
    }
    let count = lines.len().clamp(1, BODY_MAX_LINES);
    let mut start = 0;
    let mut out = Vec::with_capacity(count);
    for (i, line) in lines.iter().take(count).enumerate() {
        let text = if i + 1 == count {
            String::from_utf16_lossy(&wide[start..]).replace(['\r', '\n'], " ")
        } else {
            let end = start + (line.length - line.newlineLength) as usize;
            String::from_utf16_lossy(&wide[start..end])
        };
        out.push(text.trim().to_string());
        start += line.length as usize;
    }
    Ok(out)
}

/// Spec 2.4: "변환 중 · N개", then a row per file with an accent bar.
fn draw_progress(
    ctx: &ID2D1DeviceContext,
    f: &Formats,
    rows: &[ProgressRow],
    card: D2D_RECT_F,
    theme: &Theme,
    scale: f32,
) -> Result<()> {
    let shown = rows.len().min(MAX_ROWS);
    let hidden = rows.len() - shown;
    let pad = PAD_X * scale;
    let text_box = |top: f32, height: f32| D2D_RECT_F {
        left: card.left + pad,
        top,
        right: card.right - pad,
        bottom: top + height,
    };
    unsafe {
        let brush = |color: &D2D1_COLOR_F| ctx.CreateSolidColorBrush(color, None);
        let (text, subtext) = (brush(&theme.text)?, brush(&theme.subtext)?);
        let header = format!("변환 중 · {}개", rows.len());
        draw_text(
            ctx,
            &header,
            &f.header,
            text_box(card.top + 4.0 * scale, (HEADER - 4.0) * scale),
            &subtext,
        );
        let (track, accent) = (brush(&theme.track)?, brush(&theme.accent)?);
        for (i, row) in rows.iter().take(shown).enumerate() {
            let top = card.top + (HEADER + i as f32 * ROW) * scale;
            let line = text_box(top, 20.0 * scale);
            draw_text(
                ctx,
                &row.title,
                &f.row,
                D2D_RECT_F {
                    right: line.right - 44.0 * scale, // room for "100%"
                    ..line
                },
                &text,
            );
            draw_text(ctx, &progress_value(row.fraction), &f.value, line, &subtext);
            let bar = D2D_RECT_F {
                top: top + 25.0 * scale,
                bottom: top + (25.0 + BAR) * scale,
                ..line
            };
            let radius = BAR * scale / 2.0;
            ctx.FillRoundedRectangle(&rounded(bar, radius), &track);
            if let Some(p) = row.fraction {
                let filled = D2D_RECT_F {
                    right: bar.left + (bar.right - bar.left) * p.clamp(0.0, 1.0),
                    ..bar
                };
                // Under a bar's own diameter the rounded ends would bulge.
                if filled.right - filled.left >= 2.0 * radius {
                    ctx.FillRoundedRectangle(&rounded(filled, radius), &accent);
                }
            }
        }
        if hidden > 0 {
            let top = card.top + (HEADER + shown as f32 * ROW) * scale;
            draw_text(
                ctx,
                &more_files(hidden),
                &f.header,
                text_box(top, MORE * scale),
                &subtext,
            );
        }
    }
    Ok(())
}

/// The icon's glyph (Segoe Fluent Icons / MDL2 Assets).
fn glyph(kind: NoticeKind) -> &'static str {
    match kind {
        NoticeKind::Done => "\u{E73E}",                         // CheckMark
        NoticeKind::Info => "\u{E946}",                         // Info
        NoticeKind::Failed | NoticeKind::Warning => "\u{E7BA}", // Warning
        NoticeKind::KeyPrompt => "\u{E765}",                    // Keyboard
        NoticeKind::KeyResult => "\u{E8AB}",                    // Switch
    }
}

/// Spec 2.6: icon circle, title, body, action line, timer bar.
fn draw_notice(
    surface: &Surface,
    f: &Formats,
    card: &CardView,
    body: &[String],
    r: D2D_RECT_F,
    theme: &Theme,
    scale: f32,
) -> Result<()> {
    let ctx = surface.ctx();
    let failure = matches!(card.kind, NoticeKind::Failed | NoticeKind::Warning);
    let d = ICON * scale;
    let icon = D2D_RECT_F {
        left: r.left + PAD_X * scale,
        top: r.top + PAD_Y * scale,
        right: r.left + PAD_X * scale + d,
        bottom: r.top + PAD_Y * scale + d,
    };
    let circle = D2D1_ELLIPSE {
        point: Vector2 {
            X: icon.left + d / 2.0,
            Y: icon.top + d / 2.0,
        },
        radiusX: d / 2.0,
        radiusY: d / 2.0,
    };
    let icon_size = if card.kind == NoticeKind::KeyPrompt {
        15.0
    } else {
        14.0
    };
    let icon_format = fonts::icon_format(surface.icon_font(), icon_size * scale)?;
    unsafe {
        let brush = |color: &D2D1_COLOR_F| ctx.CreateSolidColorBrush(color, None);
        let glyph_color = if card.kind == NoticeKind::KeyPrompt {
            let shape: ID2D1Geometry = ctx.GetFactory()?.CreateEllipseGeometry(&circle)?.cast()?;
            glass_piece(ctx, &shape, icon, theme, Piece::Keycap, scale)?;
            theme.text
        } else {
            let fill = if failure { theme.failure } else { theme.accent };
            ctx.FillEllipse(&circle, &brush(&fill)?);
            theme.on_accent
        };
        draw_text(
            ctx,
            glyph(card.kind),
            &icon_format,
            icon,
            &brush(&glyph_color)?,
        );

        let column = |top: f32, height: f32| D2D_RECT_F {
            left: icon.right + ICON_GAP * scale,
            top,
            right: r.right - PAD_X * scale,
            bottom: top + height,
        };
        let mut top = r.top + PAD_Y * scale;
        // A title alone centers on the icon.
        let alone = body.is_empty() && card.action_label.is_none();
        let title_height = if alone { ICON } else { TITLE } * scale;
        draw_text(
            ctx,
            &card.title,
            &f.title,
            column(top, title_height),
            &brush(&theme.text)?,
        );
        top += title_height;
        let subtext = brush(&theme.subtext)?;
        for line in body {
            draw_text(ctx, line, &f.body, column(top, BODY_LINE * scale), &subtext);
            top += BODY_LINE * scale;
        }
        if let Some(action) = card.action_label {
            draw_text(
                ctx,
                action,
                &f.action,
                column(top, ACTION * scale),
                &brush(&theme.accent)?,
            );
        }

        // Time left before the card closes by itself.
        let bar = D2D_RECT_F {
            left: r.left + PAD_X * scale,
            top: r.bottom - (TIMER_BELOW + TIMER) * scale,
            right: r.right - PAD_X * scale,
            bottom: r.bottom - TIMER_BELOW * scale,
        };
        let radius = TIMER * scale / 2.0;
        ctx.FillRoundedRectangle(&rounded(bar, radius), &brush(&theme.line)?);
        let color = if failure { theme.failure } else { theme.accent };
        let filled = D2D_RECT_F {
            right: bar.left + (bar.right - bar.left) * card.remaining.clamp(0.0, 1.0),
            ..bar
        };
        if filled.right > filled.left {
            let color = D2D1_COLOR_F {
                a: 0.7 * theme.alpha,
                ..color
            };
            ctx.FillRoundedRectangle(&rounded(filled, radius), &brush(&color)?);
        }
    }
    Ok(())
}

/// The card at a client point.
fn hit(x: f32, y: f32) -> Option<NoticeId> {
    HITS.with_borrow(|hits| hit_at(hits, x, y))
}

/// Client coordinates packed in a mouse message's `lParam`.
fn client_point(lparam: LPARAM) -> (f32, f32) {
    let x = (lparam.0 & 0xFFFF) as u16 as i16;
    let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16;
    (x as f32, y as f32)
}

/// Records the hovered card and tells the owner when it changed. While a
/// card is hovered, a WM_MOUSELEAVE is requested so leaving is noticed too.
fn set_hover(hwnd: HWND, id: Option<NoticeId>) {
    if id.is_some() && !TRACKING.get() {
        let mut track = TRACKMOUSEEVENT {
            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: hwnd,
            dwHoverTime: 0,
        };
        TRACKING.set(unsafe { TrackMouseEvent(&mut track) }.is_ok());
    }
    if HOVER.replace(id) != id {
        post_to_owner(hwnd, 0, id.map_or(-1, |id| id as isize));
    }
}

fn post_to_owner(hwnd: HWND, wparam: usize, lparam: isize) {
    unsafe {
        if let Ok(owner) = GetWindow(hwnd, GW_OWNER) {
            let _ = PostMessageW(Some(owner), WM_APP_NOTICE, WPARAM(wparam), LPARAM(lparam));
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
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_CLOSE => {
            // Hand taskkill's WM_CLOSE to the host window, like the wheel does.
            if let Ok(owner) = unsafe { GetWindow(hwnd, GW_OWNER) } {
                unsafe {
                    let _ = PostMessageW(Some(owner), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            }
        }
        WM_MOUSEMOVE => {
            let (x, y) = client_point(lparam);
            set_hover(hwnd, hit(x, y));
        }
        WM_MOUSELEAVE => {
            TRACKING.set(false);
            set_hover(hwnd, None);
        }
        WM_LBUTTONDOWN => {
            let (x, y) = client_point(lparam);
            if let Some(id) = hit(x, y) {
                post_to_owner(hwnd, 1, id as isize);
            }
        }
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    LRESULT(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: NoticeId) -> CardView {
        CardView {
            id,
            kind: NoticeKind::Done,
            title: "2개 파일을 변환했습니다".into(),
            body: "photo.jpg 외 1개".into(),
            action_label: None,
            remaining: 1.0,
            appear: 1.0,
            closing: None,
            from_progress: None,
        }
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn progress_at_bottom_cards_above() {
        // Newest first, as `Notices::cards` returns them.
        let cards = [card(3), card(2)];
        let s = stack(2, false, &cards, |_| 1);
        let progress = card_height(2, false);
        let one = notice_height(1, false);
        assert_eq!(s.len(), 3);
        assert_eq!(s[0], (Slot::Progress, progress, progress));
        // The newest card sits 8 DIPs above the progress card.
        assert_eq!(s[1].0, Slot::Card(3));
        assert!(near(s[1].1 - s[1].2, progress + 8.0));
        assert!(near(s[1].2, one));
        assert_eq!(s[2].0, Slot::Card(2));
        assert!(near(s[2].1 - s[2].2, s[1].1 + 8.0));
        // Without progress the newest card is at the bottom.
        let s = stack(0, false, &cards, |_| 1);
        assert_eq!(s[0].0, Slot::Card(3));
        assert!(near(s[0].1, one));
    }

    #[test]
    fn body_lines_and_action_set_the_height() {
        assert!(notice_height(2, false) > notice_height(1, false));
        assert!(notice_height(1, true) > notice_height(1, false));
        let mut long = card(1);
        long.action_label = Some("클릭하면 폴더 열기");
        let s = stack(0, false, &[long], |_| 2);
        assert!(near(s[0].2, notice_height(2, true)));
    }

    #[test]
    fn body_keeps_two_lines_and_carries_the_rest_into_the_last() -> Result<()> {
        let wrap = wrapping_format(12.0)?;
        let width = WIDTH - 2.0 * PAD_X - ICON - ICON_GAP;
        assert!(body_lines("", &wrap, width)?.is_empty());
        assert_eq!(
            body_lines("photo.jpg 외 1개", &wrap, width)?,
            ["photo.jpg 외 1개"]
        );
        let long =
            "여행 브이로그 2026 최종 편집본 진짜최종.mp4: 지원하지 않는 코덱입니다\n성공 1개";
        let lines = body_lines(long, &wrap, width)?;
        assert_eq!(lines.len(), 2);
        assert!(long.starts_with(&lines[0]) && !lines[0].is_empty());
        // The second line runs on (to be cut with "…") with the paragraphs joined.
        assert!(lines[1].ends_with("코덱입니다 성공 1개"), "{lines:?}");
        assert_eq!(
            body_lines("짧은 오류\n성공 1개", &wrap, width)?,
            ["짧은 오류", "성공 1개"]
        );
        Ok(())
    }

    #[test]
    fn appear_slides_in_and_close_fades() {
        let mut c = card(1);
        c.appear = 0.5;
        let m = motion(&c);
        assert!(near(m.dx, 12.0));
        assert!(near(m.opacity, 0.5));
        c.appear = 0.0;
        assert!(near(motion(&c).dx, 24.0));
        c.appear = 1.0;
        c.closing = Some(0.25);
        let m = motion(&c);
        assert!(near(m.dx, 0.0));
        assert!(near(m.opacity, 0.75));
        // A card taking over the progress card's slot doesn't slide in.
        let mut morph = card(2);
        morph.appear = 0.0;
        morph.from_progress = Some(0.0);
        assert!(near(motion(&morph).dx, 0.0));
    }

    #[test]
    fn closing_card_lets_the_ones_above_slide_down() {
        let mut closing = card(2);
        closing.closing = Some(0.5);
        let s = stack(0, false, &[closing, card(1)], |_| 1);
        let one = notice_height(1, false);
        // Halfway through the fade, half of its room (with the gap) is left.
        assert!(near(s[1].1 - s[1].2, (one + 8.0) * 0.5));
    }

    /// The card painted on top where two overlap is the one a point hits;
    /// a fading card still comes first.
    #[test]
    fn hits_follow_paint_order() {
        let no_body = |_| &[][..];
        let mut new = card(2);
        new.appear = 0.2;
        let cards = [new, card(1)];
        let placed = arrange(0, false, &cards, no_body, 0.0, 1000.0, 1.0);
        let index = |placed: &[Placed], id| {
            placed
                .iter()
                .position(|p| p.card.unwrap().id == id)
                .unwrap()
        };
        let at = |id| index(&placed, id);
        // The older card has moved up by only a fifth of the new card's room,
        // so the two overlap and the older one, painted later, is on top.
        assert!(at(1) > at(2));
        let (old, new) = (placed[at(1)].rect, placed[at(2)].rect);
        assert!(old.bottom > new.top);
        let x = (old.left.max(new.left) + old.right.min(new.right)) / 2.0;
        let y = (new.top + old.bottom) / 2.0;
        assert_eq!(hit_at(&hit_list(&placed), x, y), Some(1));

        // A card fading under the one sliding down into its place.
        let mut fading = card(2);
        fading.closing = Some(0.5);
        let cards = [fading, card(1)];
        let placed = arrange(0, false, &cards, no_body, 0.0, 1000.0, 1.0);
        let (old, gone) = (
            placed[index(&placed, 1)].rect,
            placed[index(&placed, 2)].rect,
        );
        assert!(old.bottom > gone.top);
        let y = (gone.top + old.bottom) / 2.0;
        assert_eq!(hit_at(&hit_list(&placed), 150.0, y), Some(2));
    }

    #[test]
    fn morph_interpolates_height() {
        let own = notice_height(1, true);
        let mut c = card(1);
        c.action_label = Some("클릭하면 폴더 열기");
        c.from_progress = Some(0.5);
        // The progress card had the finished job's one row.
        let s = stack(0, false, std::slice::from_ref(&c), |_| 1);
        assert_eq!(s[0].0, Slot::Card(1));
        assert!(near(s[0].2, (card_height(1, false) + own) / 2.0));
        assert!(near(s[0].1, s[0].2));
        c.from_progress = Some(1.0);
        let s = stack(0, false, &[c], |_| 1);
        assert!(near(s[0].2, own));
    }
}
