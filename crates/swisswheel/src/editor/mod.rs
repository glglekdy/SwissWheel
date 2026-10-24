//! The image editor panel (spec 2.2–2.8, 3.2): one panel at a time, owned
//! by the UI thread. `editor_control` decides what every key and click
//! does; this module keeps the panel's lifetime, decodes the image off the
//! UI thread, and hands the app what the panel ended with.

mod render;
mod window;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use ::image::imageops::{self, FilterType};
use ::image::{DynamicImage, RgbaImage};
use swisswheel_core::appearance::Accent;
use swisswheel_core::editor::{EditPlan, text};
use swisswheel_core::editor_control::{EditorState, Outcome};
use swisswheel_core::editor_geometry::{EditorHit, EditorLayout, panel_layout};
use swisswheel_core::format::Format;
use windows::Win32::Foundation::{E_FAIL, HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, PostMessageW, SetTimer, WM_APP};

use crate::engines::EngineError;
use crate::engines::image::{decode, dimensions};
use crate::surface::Surface;
use crate::theme::Theme;
use render::Previews;
use window::Windows;

/// Posted to the panel by the decode thread once its slot is filled.
const WM_APP_EDITOR_LOADED: u32 = WM_APP + 50;
/// Panel timer: the discard prompt expires.
const DISCARD_TIMER: usize = 1;

/// What the panel ended with, for the app (`take_events`).
#[derive(Debug)]
pub enum EditorEvent {
    Saved(PathBuf, EditPlan),
    Closed,
    Failed(String),
}

/// The decode thread's result: the decoded size and the two previews.
struct Decoded {
    width: u32,
    height: u32,
    color: RgbaImage,
    gray: RgbaImage,
}

type Slot = Arc<Mutex<Option<Result<Decoded, String>>>>;

struct Editor {
    path: PathBuf,
    name: String,
    /// Who hears about events, and with which message.
    owner: HWND,
    notify: u32,
    state: EditorState,
    layout: EditorLayout,
    scale: f32,
    theme: Theme,
    strip_available: bool,
    hover: EditorHit,
    /// The button a press started on, for a click on release.
    press: Option<EditorHit>,
    slot: Slot,
    /// The decoded previews, kept to rebuild the bitmaps after a device loss.
    decoded: Option<(RgbaImage, RgbaImage)>,
    // Declared before `windows` so they are released before the windows go.
    previews: Option<Previews>,
    surface: Option<Surface>,
    windows: Windows,
}

thread_local! {
    static EDITOR: RefCell<Option<Editor>> = const { RefCell::new(None) };
    static EVENTS: RefCell<Vec<EditorEvent>> = const { RefCell::new(Vec::new()) };
}

/// Opens the panel for `path` on the monitor under the cursor, showing
/// "불러오는 중" while a thread decodes the image. Every event later is
/// queued for `take_events` and announced by posting `notify` to `owner`.
/// An error (unreadable header, no window or renderer) leaves no window.
pub fn open(path: PathBuf, owner: HWND, notify: u32, accent: Accent) -> windows::core::Result<()> {
    if is_open() {
        // Callers check `is_open` first; one panel at a time.
        log_error!(
            "편집기가 이미 열려 있어 {} 열기를 무시합니다",
            path.display()
        );
        focus();
        return Ok(());
    }
    let name = file_name(&path);
    let size = dimensions(&path).map_err(|e| {
        log_error!("편집기: 크기를 읽을 수 없습니다: {e}");
        windows::core::Error::new(E_FAIL, e.to_string())
    })?;
    let place = window::placement();
    let layout = panel_layout(place.work, place.scale, size);
    let windows = Windows::create(owner, place.monitor, layout.window)?;
    let theme = Theme::current(windows.glass_supported, accent, 1.0);
    let strip_available = matches!(
        Format::from_path(&path),
        Some(Format::Jpg | Format::Png | Format::Webp)
    );
    let slot: Slot = Arc::default();
    let mut editor = Editor {
        path: path.clone(),
        name,
        owner,
        notify,
        state: EditorState::loading(),
        layout,
        scale: place.scale,
        theme,
        strip_available,
        hover: EditorHit::Nothing,
        press: None,
        slot: Arc::clone(&slot),
        decoded: None,
        previews: None,
        surface: None,
        windows,
    };
    if !editor.redraw() {
        // Dropping `editor` destroys the windows.
        return Err(windows::core::Error::new(E_FAIL, "편집기 렌더러 없음"));
    }
    let (dim, panel) = (editor.windows.dim, editor.windows.panel);
    let preview = editor.layout.preview;
    let longest = (preview.right - preview.left)
        .max(preview.bottom - preview.top)
        .round()
        .max(1.0) as u32;
    EDITOR.with(|cell| *cell.borrow_mut() = Some(editor));
    log_info!(
        "편집기 열기: {} ({}×{}, 배율 {})",
        path.display(),
        size.0,
        size.1,
        place.scale
    );
    window::show(dim, panel);
    spawn_decode(path, panel, longest, slot);
    Ok(())
}

pub fn is_open() -> bool {
    // Borrowed means a handler is running, so it is open.
    EDITOR.with(|cell| cell.try_borrow().map_or(true, |e| e.is_some()))
}

/// Brings the open panel to the front with keyboard focus.
pub fn focus() {
    if let Some(panel) = with_editor(|ed| ed.windows.panel) {
        window::raise(panel);
    }
}

/// Destroys the panel without an event (a cancelled or quitting app).
pub fn close() {
    if let Some(editor) = take_editor() {
        log_info!("편집기 닫기 (이벤트 없음)");
        drop(editor);
    }
}

pub fn take_events() -> Vec<EditorEvent> {
    EVENTS.with(|events| std::mem::take(&mut *events.borrow_mut()))
}

/// Runs `f` on the open editor; `None` when there is none, or when it is
/// already in use (a message sent to the panel while another is handled).
fn with_editor<R>(f: impl FnOnce(&mut Editor) -> R) -> Option<R> {
    EDITOR.with(|cell| {
        let mut editor = cell.try_borrow_mut().ok()?;
        editor.as_mut().map(f)
    })
}

fn take_editor() -> Option<Editor> {
    EDITOR.with(|cell| cell.try_borrow_mut().ok()?.take())
}

/// Acts on what an event led to. Called outside `with_editor`, since
/// closing destroys windows, which sends them messages.
fn settle(outcome: Outcome) {
    match outcome {
        Outcome::Nothing => {}
        Outcome::Redraw => {
            if with_editor(|ed| {
                ed.arm_discard_timer(Instant::now());
                ed.redraw()
            }) == Some(false)
            {
                finish(|_| EditorEvent::Failed(text::OPEN_FAILED.to_string()));
            }
        }
        Outcome::Save(plan) => finish(|path| EditorEvent::Saved(path, plan)),
        Outcome::Close => finish(|_| EditorEvent::Closed),
    }
}

/// Destroys the panel and queues the event `make` builds from its path.
fn finish(make: impl FnOnce(PathBuf) -> EditorEvent) {
    let Some(editor) = take_editor() else {
        return;
    };
    let (owner, notify) = (editor.owner, editor.notify);
    let event = make(editor.path.clone());
    drop(editor);
    log_info!("편집기 닫힘: {event:?}");
    EVENTS.with(|events| events.borrow_mut().push(event));
    unsafe {
        let _ = PostMessageW(Some(owner), notify, WPARAM(0), LPARAM(0));
    }
}

/// `WM_APP_EDITOR_LOADED`: takes the decode thread's result.
fn on_loaded() {
    let failed = with_editor(|ed| {
        let result = ed.slot.lock().unwrap().take()?;
        match result {
            Ok(d) => {
                ed.state.loaded(d.width, d.height, ed.strip_available);
                ed.decoded = Some((d.color, d.gray));
                None
            }
            Err(e) => {
                log_error!("편집기: 이미지를 읽을 수 없습니다: {e}");
                Some(EngineError::Read(e).user_message(&ed.name))
            }
        }
    });
    match failed {
        Some(Some(message)) => finish(|_| EditorEvent::Failed(message)),
        Some(None) => settle(Outcome::Redraw),
        None => {}
    }
}

impl Editor {
    /// Draws a frame; after a failure (device lost) rebuilds the surface
    /// and bitmaps and tries once more. `false` when that fails too.
    fn redraw(&mut self) -> bool {
        match self.draw() {
            Ok(()) => true,
            Err(e) => {
                log_error!("편집기 그리기 실패, 렌더러를 다시 만듭니다: {e}");
                self.previews = None;
                self.surface = None;
                self.draw()
                    .inspect_err(|e| log_error!("편집기 렌더러를 다시 만들 수 없습니다: {e}"))
                    .is_ok()
            }
        }
    }

    fn draw(&mut self) -> windows::core::Result<()> {
        if self.surface.is_none() {
            let w = self.layout.window;
            let (width, height) = ((w.right - w.left) as u32, (w.bottom - w.top) as u32);
            self.surface = Some(Surface::new(self.windows.panel, width, height, 1)?);
        }
        let Some(surface) = &self.surface else {
            return Ok(());
        };
        if self.previews.is_none()
            && let Some((color, gray)) = &self.decoded
        {
            self.previews = Some(render::previews(surface, color, gray)?);
        }
        render::draw(
            surface,
            &self.theme,
            &self.layout,
            self.scale,
            &self.state,
            self.previews.as_ref(),
            &self.name,
            self.hover,
            Instant::now(),
        )
    }

    /// Wakes the panel when the discard prompt runs out, so it disappears.
    fn arm_discard_timer(&self, now: Instant) {
        let panel = self.windows.panel;
        unsafe {
            match self.state.discard_deadline() {
                Some(deadline) if deadline > now => {
                    let ms = (deadline - now).as_millis() as u32 + 1;
                    SetTimer(Some(panel), DISCARD_TIMER, ms, None);
                }
                _ => {
                    let _ = KillTimer(Some(panel), DISCARD_TIMER);
                }
            }
        }
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// Decodes `path` on a worker thread into previews whose longer side is
/// `longest` pixels (never upscaled), then posts `WM_APP_EDITOR_LOADED` to
/// `panel`. It touches no UI object; a panel gone by then just misses it.
fn spawn_decode(path: PathBuf, panel: HWND, longest: u32, slot: Slot) {
    let panel = panel.0 as isize;
    std::thread::spawn(move || {
        let started = Instant::now();
        let result = decode(&path)
            .map(|image| previews_of(image, longest))
            .map_err(|e| e.to_string());
        if let Ok(d) = &result {
            log_info!(
                "편집기 미리보기 준비: {}×{} → {}×{} ({} ms)",
                d.width,
                d.height,
                d.color.width(),
                d.color.height(),
                started.elapsed().as_millis()
            );
        }
        *slot.lock().unwrap() = Some(result);
        unsafe {
            let _ = PostMessageW(
                Some(HWND(panel as *mut _)),
                WM_APP_EDITOR_LOADED,
                WPARAM(0),
                LPARAM(0),
            );
        }
    });
}

fn previews_of(image: DynamicImage, longest: u32) -> Decoded {
    let (width, height) = (image.width(), image.height());
    let mut color = image.into_rgba8();
    let side = width.max(height);
    if side > longest {
        let fit = |v: u32| ((u64::from(v) * u64::from(longest)) as f64 / f64::from(side)).round();
        let (w, h) = (fit(width).max(1.0) as u32, fit(height).max(1.0) as u32);
        color = imageops::resize(&color, w, h, FilterType::Triangle);
    }
    let gray = DynamicImage::ImageRgba8(color.clone())
        .grayscale()
        .into_rgba8();
    Decoded {
        width,
        height,
        color,
        gray,
    }
}
