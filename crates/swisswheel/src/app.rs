//! The resident app: single instance, hidden host window, tray, hooks, the
//! wheel, and the job runner, all driven from one UI-thread message loop.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use swisswheel_core::catalog::Operation;
use swisswheel_core::config::Config;
use swisswheel_core::drag::{DragCommand, DragEvent, KeyBindings};
use swisswheel_core::geometry::Hit;
use swisswheel_core::menu::{MenuAction, MenuHit, MenuSession};
use swisswheel_core::menu_geometry;
use swisswheel_core::session::{Selection, WheelSession};
use swisswheel_core::settings::{self, Dir, SettingItem};
use swisswheel_core::summary::{busy_tooltip, job_summary, selection_label};
use windows::System::DispatcherQueueController;
use windows::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, POINT, WPARAM,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::{
    DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE, IDropTarget, OleInitialize, RegisterDragDrop,
    RevokeDragDrop,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::System::WinRT::{
    CreateDispatcherQueueController, DQTAT_COM_NONE, DQTYPE_THREAD_CURRENT, DispatcherQueueOptions,
};
use windows::Win32::UI::Accessibility::HWINEVENTHOOK;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_MOVE, MOUSEINPUT, SendInput,
};
use windows::Win32::UI::Shell::NIN_BALLOONUSERCLICK;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Result, w};

use crate::dnd::DropTarget;
use crate::hook::{self, Hooks, WM_APP_DRAG, WM_APP_FOREGROUND};
use crate::jobs::{Job, JobReport, Runner};
use crate::menubar::{self, MenuBar, MenuMouse, MenuView};
use crate::probe;
use crate::progress::ProgressWindow;
use crate::tray::{Tray, WM_APP_TRAY};
use crate::wheel::{PanelView, WheelView, WheelWindow};
use crate::{autostart, config_file};

const ID_MENU_EXIT: usize = 1;
const WM_APP_JOB_DONE: u32 = WM_APP + 30;
/// FFmpeg jobs that run at once (spec 4.4).
const MEDIA_WORKERS: usize = 2;

const TIMER_ENTER_TIMEOUT: usize = 1;
const TIMER_MOUSE_UP_CLOSE: usize = 2;
const TIMER_ANIMATION: usize = 3;
const TIMER_MAINTENANCE: usize = 4;
const TIMER_PROGRESS: usize = 5;
/// Finishes a panel dwell while the cursor rests (no DragOver arrives then).
const TIMER_PANEL: usize = 6;
/// Runs the desktop menu's open slide and panel fade.
const TIMER_MENU_ANIMATION: usize = 7;
const PANEL_TICK_MS: u32 = 50;
/// How often the tray tooltip refreshes the progress of running jobs.
const PROGRESS_INTERVAL_MS: u32 = 500;

/// Shown at the top of the settings panel when config.toml couldn't be read.
const CONFIG_NOTICE: &str = "설정 파일에 오류가 있어 저장하지 않습니다";

const ENTER_TIMEOUT: Duration = Duration::from_millis(300);
const MOUSE_UP_CLOSE_DELAY: Duration = Duration::from_millis(200);
const OPEN_ANIMATION: Duration = Duration::from_millis(150);
const PANEL_FADE: Duration = Duration::from_millis(120);
const MAINTENANCE_INTERVAL: Duration = Duration::from_secs(60);
const RENDERER_IDLE: Duration = Duration::from_secs(10 * 60);
const HOOK_REINSTALL: Duration = Duration::from_secs(10 * 60);

pub(crate) struct AppState {
    hwnd: HWND,
    tray: Tray,
    hooks: Hooks,
    wheel: WheelWindow,
    /// The desktop menu's popup; `None` if it couldn't be created.
    menubar: Option<MenuBar>,
    /// The open desktop menu, if any, and when it and its panel opened.
    menu: Option<MenuSession>,
    menu_opened_at: Instant,
    menu_panel_at: Instant,
    /// Foreground watch while the menu is open (it closes when another
    /// window comes forward).
    foreground_hook: Option<HWINEVENTHOOK>,
    progress_window: ProgressWindow,
    /// The settings in effect; the settings panel edits them live.
    config: Config,
    config_dir: PathBuf,
    /// False when config.toml was unusable at startup: edits apply but are
    /// not saved over the file the user may be fixing.
    config_writable: bool,
    /// The row whose last change couldn't be saved (shows ⚠).
    save_failed: Option<SettingItem>,
    _drop_target: IDropTarget,
    _dispatcher: DispatcherQueueController,
    runner: Option<Runner>,
    reports: Arc<Mutex<VecDeque<JobReport>>>,
    /// The open wheel, if any, and the files dragged onto it.
    session: Option<WheelSession>,
    dragged: Vec<PathBuf>,
    /// Center label for the open wheel ("2.4 MB"); see `selection_summary`.
    selection: String,
    opened_at: Instant,
    /// When the wheel opened: the clock for the panel dwell.
    session_started: Instant,
    /// The item whose option panel is shown, and since when (for its fade).
    panel_item: Option<usize>,
    panel_shown_at: Instant,
    active_jobs: usize,
    /// First output of the last finished job, opened when its balloon is clicked.
    last_output: Option<PathBuf>,
    hooks_installed_at: Instant,
    taskbar_created: u32,
}

thread_local! {
    static STATE: RefCell<Option<AppState>> = const { RefCell::new(None) };
}

/// Runs `f` with the app state. Returns `None` if the app isn't running or the
/// state is already borrowed (a re-entrant message during a cross-process
/// call), so a stray message never panics the UI thread.
fn with_state<R>(f: impl FnOnce(&mut AppState) -> R) -> Option<R> {
    STATE.with(|s| s.try_borrow_mut().ok().and_then(|mut s| s.as_mut().map(f)))
}

pub fn run() -> Result<()> {
    let _instance = unsafe { CreateMutexW(None, true, w!("Local\\SwissWheel.SingleInstance"))? };
    // A debug preview may run beside an installed copy: it only draws.
    let previewing = cfg!(debug_assertions) && std::env::var_os("SWISSWHEEL_PREVIEW").is_some();
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS && !previewing {
        return Ok(()); // already running: the existing tray icon stays
    }
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        OleInitialize(None)?;
    }
    // Windows.UI.Composition (the wheel's glass backdrop) needs a DispatcherQueue
    // on this thread; it runs off our ordinary message loop. OLE already set up
    // the apartment, hence DQTAT_COM_NONE.
    let dispatcher = unsafe {
        CreateDispatcherQueueController(DispatcherQueueOptions {
            dwSize: size_of::<DispatcherQueueOptions>() as u32,
            threadType: DQTYPE_THREAD_CURRENT,
            apartmentType: DQTAT_COM_NONE,
        })?
    };

    let (config_dir, log_dir) = config_file::app_dirs();
    if let Err(e) = crate::logging::init(&log_dir) {
        eprintln!("log init failed: {e}");
    }
    log_info!("SwissWheel {} 시작", env!("CARGO_PKG_VERSION"));
    let (config, warning) = config_file::load(&config_dir);

    let hwnd = create_host_window()?;
    let tray = Tray::new(hwnd)?;
    if let Some(w) = &warning {
        log_error!("설정 파일 오류: {w}");
        tray.warn("설정 파일 오류", &format!("{w}\n기본 설정으로 실행합니다"));
    }
    // The Run entry follows the setting (e.g. after the exe moved).
    if autostart::is_enabled() != config.start_with_windows
        && let Err(e) = autostart::set(config.start_with_windows)
    {
        log_error!("시작 프로그램 등록 실패: {e}");
    }
    let hooks = hook::install(hwnd, KeyBindings::from(&config))?;
    let wheel = WheelWindow::new(hwnd)?;
    // Without its window the desktop menu stays off; Space then passes through.
    let menubar = MenuBar::new(hwnd)
        .inspect_err(|e| {
            log_error!("메뉴 창을 만들 수 없습니다: {e}");
            hook::set_menu_enabled(false);
        })
        .ok();
    let progress_window = ProgressWindow::new(hwnd)?;
    let drop_target: IDropTarget = DropTarget.into();
    unsafe { RegisterDragDrop(wheel.hwnd(), &drop_target)? };

    let reports = Arc::new(Mutex::new(VecDeque::new()));
    let runner = {
        let reports = Arc::clone(&reports);
        let host = hwnd.0 as isize; // HWND isn't Send; workers only need its value
        let workers = std::thread::available_parallelism()
            .map_or(2, |n| n.get())
            .min(4);
        Runner::new(
            workers,
            MEDIA_WORKERS,
            config.quality.clone(),
            Box::new(move |report| {
                reports.lock().unwrap().push_back(report);
                unsafe {
                    let _ = PostMessageW(
                        Some(HWND(host as *mut _)),
                        WM_APP_JOB_DONE,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }),
        )
    };

    let taskbar_created = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
    unsafe {
        SetTimer(
            Some(hwnd),
            TIMER_MAINTENANCE,
            MAINTENANCE_INTERVAL.as_millis() as u32,
            None,
        )
    };
    STATE.set(Some(AppState {
        hwnd,
        tray,
        hooks,
        wheel,
        menubar,
        menu: None,
        menu_opened_at: Instant::now(),
        menu_panel_at: Instant::now(),
        foreground_hook: None,
        progress_window,
        config_writable: warning.is_none(),
        config,
        config_dir,
        save_failed: None,
        _drop_target: drop_target,
        _dispatcher: dispatcher,
        runner: Some(runner),
        reports,
        session: None,
        dragged: Vec::new(),
        selection: String::new(),
        opened_at: Instant::now(),
        session_started: Instant::now(),
        panel_item: None,
        panel_shown_at: Instant::now(),
        active_jobs: 0,
        last_output: None,
        hooks_installed_at: Instant::now(),
        taskbar_created,
    }));
    #[cfg(debug_assertions)]
    if let Ok(what) = std::env::var("SWISSWHEEL_PREVIEW") {
        with_state(|s| s.preview(&what));
    }

    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.into() {
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    log_info!("SwissWheel 종료");
    Ok(())
}

fn create_host_window() -> Result<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = w!("SwissWheelHost");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance.into(),
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassW(&wc);
        // A hidden top-level window (not message-only) so it receives broadcast
        // messages such as TaskbarCreated and a polite WM_CLOSE from taskkill.
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!("SwissWheel"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_APP_DRAG => {
            if let Some(command) = hook::decode(wparam, lparam) {
                with_state(|s| s.on_drag_command(command));
            }
        }
        WM_TIMER => {
            with_state(|s| s.on_timer(wparam.0));
        }
        WM_APP_JOB_DONE => {
            with_state(AppState::on_jobs_done);
        }
        WM_APP_FOREGROUND => {
            with_state(|s| s.on_foreground(HWND(wparam.0 as *mut _)));
        }
        // Monitors, DPI, or the work area changed: the bar's placement is stale.
        WM_DISPLAYCHANGE | WM_DPICHANGED => {
            with_state(AppState::close_menu);
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
        }
        WM_SETTINGCHANGE => {
            if wparam.0 == SPI_SETWORKAREA.0 as usize {
                with_state(AppState::close_menu);
            }
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
        }
        WM_APP_TRAY => on_tray(hwnd, wparam, lparam),
        WM_COMMAND if wparam.0 & 0xFFFF == ID_MENU_EXIT => unsafe {
            let _ = DestroyWindow(hwnd);
        },
        WM_CLOSE => unsafe {
            log_info!("종료 요청 (WM_CLOSE)");
            let _ = DestroyWindow(hwnd);
        },
        WM_DESTROY => {
            log_info!("종료 중: 상태 정리 시작");
            if let Some(mut state) = STATE.take() {
                unsafe {
                    let _ = RevokeDragDrop(state.wheel.hwnd());
                }
                if let Some(watch) = state.foreground_hook.take() {
                    hook::unwatch_foreground(watch);
                }
                log_info!("종료 중: 작업 대기");
                if let Some(runner) = state.runner.take() {
                    runner.shutdown(); // finish queued files before exiting
                }
                log_info!("종료 중: 리소스 해제");
            } // dropping the state removes the tray icon and the hooks
            log_info!("종료 중: 메시지 루프 종료 요청");
            unsafe { PostQuitMessage(0) };
        }
        _ => {
            let restored = with_state(|s| {
                if msg == s.taskbar_created {
                    s.tray.add();
                    true
                } else {
                    false
                }
            });
            if restored != Some(true) {
                return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
            }
        }
    }
    LRESULT(0)
}

impl AppState {
    fn on_drag_command(&mut self, command: DragCommand) {
        match command {
            DragCommand::OpenWheel => self.open_wheel(),
            DragCommand::Cycle => {
                if let Some(session) = &mut self.session {
                    session.cycle();
                    self.sync_panel();
                    self.render();
                }
            }
            DragCommand::Cancel => self.close_wheel(),
            DragCommand::OpenMenu => self.open_menu(),
            DragCommand::MenuKey(key) => match &mut self.menu {
                Some(menu) => {
                    let action = menu.key(key);
                    self.on_menu_action(action);
                }
                None => hook::notify(DragEvent::MenuClosed), // out of step: resync
            },
            DragCommand::MenuPointerDown { x, y } => {
                let inside = self
                    .menubar
                    .as_ref()
                    .is_some_and(|bar| bar.contains_screen(x, y));
                if !inside {
                    self.close_menu();
                }
            }
            DragCommand::CloseMenu => self.close_menu(),
            DragCommand::ButtonReleasedWhileOpen => unsafe {
                // WM_LBUTTONUP reaches the hook before OLE delivers Drop; wait for it.
                SetTimer(
                    Some(self.hwnd),
                    TIMER_MOUSE_UP_CLOSE,
                    MOUSE_UP_CLOSE_DELAY.as_millis() as u32,
                    None,
                );
            },
        }
    }

    fn open_menu(&mut self) {
        let Some(bar) = &mut self.menubar else {
            hook::notify(DragEvent::MenuClosed);
            return;
        };
        if self.session.is_some() {
            hook::notify(DragEvent::MenuClosed); // the wheel is up
            return;
        }
        let mut cursor = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut cursor);
        }
        bar.place(cursor, !self.config_writable);
        bar.show();
        self.menu = Some(MenuSession::new());
        self.menu_opened_at = Instant::now();
        if self.foreground_hook.is_none() {
            self.foreground_hook = hook::watch_foreground();
        }
        self.render_menu();
        unsafe { SetTimer(Some(self.hwnd), TIMER_MENU_ANIMATION, 16, None) };
    }

    fn close_menu(&mut self) {
        if self.menu.take().is_none() {
            return;
        }
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_MENU_ANIMATION);
        }
        if let Some(bar) = &mut self.menubar {
            bar.hide();
            bar.set_panel(false);
        }
        if let Some(watch) = self.foreground_hook.take() {
            hook::unwatch_foreground(watch);
        }
        hook::notify(DragEvent::MenuClosed);
    }

    /// Another window came forward (Alt+Tab, the Start menu, a click on a
    /// window): the menu belongs to the desktop, so it closes.
    fn on_foreground(&mut self, hwnd: HWND) {
        if self.menu.is_some() && !hook::is_desktop(hwnd) {
            self.close_menu();
        }
    }

    fn on_menu_mouse(&mut self, event: MenuMouse) {
        let (Some(menu), Some(bar)) = (&mut self.menu, &self.menubar) else {
            return;
        };
        let panel_open = menu.row().is_some();
        let hit = |x, y| menu_geometry::hit(bar.layout(), panel_open, x, y);
        let action = match event {
            MenuMouse::Move { x, y } => {
                if menu.hover(hit(x, y)) {
                    MenuAction::Redraw
                } else {
                    MenuAction::None
                }
            }
            MenuMouse::Leave => {
                if menu.hover(MenuHit::Outside) {
                    MenuAction::Redraw
                } else {
                    MenuAction::None
                }
            }
            MenuMouse::Click { x, y } => menu.click(hit(x, y)),
            MenuMouse::Wheel { x, y, delta } => {
                let dir = if delta > 0 { Dir::Forward } else { Dir::Back };
                menu.scroll(hit(x, y), dir)
            }
        };
        self.on_menu_action(action);
    }

    fn on_menu_action(&mut self, action: MenuAction) {
        match action {
            MenuAction::None => {}
            MenuAction::Redraw => self.render_menu(),
            MenuAction::Change(item, dir) => {
                if let Some(new) = settings::step(&self.config, item, dir) {
                    self.apply_config(new, item);
                }
                self.render_menu();
            }
            MenuAction::Notice(tile) => {
                self.close_menu();
                self.tray
                    .notify(tile.label(), "다음 버전에서 지원 예정입니다");
            }
            MenuAction::Close => self.close_menu(),
        }
    }

    /// Puts a settings change into effect right away, then saves it.
    fn apply_config(&mut self, mut new: Config, item: SettingItem) {
        let old = &self.config;
        if (new.open_key, new.cycle_key, new.cancel_key)
            != (old.open_key, old.cycle_key, old.cancel_key)
        {
            hook::set_keys(KeyBindings::from(&new));
        }
        if new.quality != old.quality
            && let Some(runner) = &self.runner
        {
            runner.set_quality(new.quality.clone());
        }
        if new.start_with_windows != old.start_with_windows
            && let Err(e) = autostart::set(new.start_with_windows)
        {
            log_error!("시작 프로그램 등록 실패: {e}");
            self.tray
                .warn("시작 프로그램 등록에 실패했습니다", &e.message());
            new.start_with_windows = old.start_with_windows;
        }
        let progress_changed = new.progress != old.progress;
        let before = std::mem::replace(&mut self.config, new);
        if progress_changed {
            self.update_busy();
        }
        if !self.config_writable {
            return; // see CONFIG_NOTICE
        }
        match config_file::save(&self.config_dir, &before, &self.config) {
            Ok(()) => self.save_failed = None,
            Err(e) => {
                log_error!("설정 저장 실패: {e}");
                if self.save_failed.is_none() {
                    self.tray.warn("설정을 저장하지 못했습니다", &e.to_string());
                }
                self.save_failed = Some(item);
            }
        }
    }

    fn render_menu(&mut self) {
        let (Some(menu), Some(bar)) = (&self.menu, &mut self.menubar) else {
            return;
        };
        let panel_open = menu.row().is_some();
        if panel_open != bar.panel_open() {
            bar.set_panel(panel_open);
            if panel_open {
                self.menu_panel_at = Instant::now();
                unsafe { SetTimer(Some(self.hwnd), TIMER_MENU_ANIMATION, 16, None) };
            }
        }
        let rows = menubar::rows(&self.config, self.save_failed);
        let view = MenuView {
            selected: menu.tile(),
            rows: panel_open.then_some(rows.as_slice()),
            row: menu.row(),
            hovered: menu.hovered(),
            notice: (!self.config_writable).then_some(CONFIG_NOTICE),
            progress: fraction(self.menu_opened_at, OPEN_ANIMATION),
            panel_progress: fraction(self.menu_panel_at, PANEL_FADE),
        };
        bar.render(&view);
    }

    fn open_wheel(&mut self) {
        let mut cursor = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut cursor);
        }
        self.open_wheel_at(cursor);
    }

    fn open_wheel_at(&mut self, cursor: POINT) {
        self.close_menu(); // the tracker already did; this hides the bar
        self.dragged.clear();
        self.wheel.place_at(cursor);
        let mut session = WheelSession::new();
        session.set_panel_side(self.wheel.panel_side());
        self.session = Some(session);
        self.session_started = Instant::now();
        self.panel_item = None;
        // Invisible until DragEnter proves this is a file drag (no flash on a
        // title-bar drag); the open animation starts in on_drag_enter.
        self.wheel.clear();
        self.wheel.show();
        unsafe {
            SetTimer(
                Some(self.hwnd),
                TIMER_ENTER_TIMEOUT,
                ENTER_TIMEOUT.as_millis() as u32,
                None,
            );
            // A zero-pixel move makes the OLE drag loop hit-test right away, so
            // DragEnter arrives without the user moving the mouse (spike S2).
            let input = INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dwFlags: MOUSEEVENTF_MOVE,
                        ..Default::default()
                    },
                },
            };
            SendInput(&[input], size_of::<INPUT>() as i32);
        }
    }

    fn close_wheel(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_ENTER_TIMEOUT);
            let _ = KillTimer(Some(self.hwnd), TIMER_MOUSE_UP_CLOSE);
            let _ = KillTimer(Some(self.hwnd), TIMER_ANIMATION);
            let _ = KillTimer(Some(self.hwnd), TIMER_PANEL);
        }
        self.wheel.hide();
        self.wheel.set_panel(None);
        self.panel_item = None;
        self.session = None;
        hook::notify(DragEvent::WheelClosed);
    }

    fn render(&mut self) {
        let Some(session) = &self.session else {
            return;
        };
        if session.is_loading() {
            return; // stays transparent until the files are known
        }
        let title = session.wheel().map_or("—", |kind| kind.label());
        let progress =
            (self.opened_at.elapsed().as_secs_f32() / OPEN_ANIMATION.as_secs_f32()).min(1.0);
        let panel = session.panel().map(|p| PanelView {
            title: p.set.title,
            choices: p.set.choices,
            default: p.set.default,
            hovered: p.hovered,
            side: self.wheel.panel_side(),
            progress: (self.panel_shown_at.elapsed().as_secs_f32() / PANEL_FADE.as_secs_f32())
                .min(1.0),
        });
        let view = WheelView {
            title,
            detail: &self.selection,
            items: session.items(),
            hovered: session.hovered(),
            progress,
            panel,
        };
        self.wheel.render(&view);
    }

    /// Debug builds only. `SWISSWHEEL_PREVIEW` = `panel`, `panel-left`,
    /// `progress`, `menu`, or `menu-settings` shows that UI at startup without
    /// a drag or key press, so the drawing can be checked by eye.
    #[cfg(debug_assertions)]
    fn preview(&mut self, what: &str) {
        use swisswheel_core::catalog::{InputItem, Operation};
        use swisswheel_core::format::Format;
        use swisswheel_core::geometry::{panel_rect, sector};
        use swisswheel_core::session::PANEL_DWELL_MS;

        let ago = |secs| Instant::now().checked_sub(Duration::from_secs(secs));
        match what {
            "panel" | "panel-left" => {
                let (sw, sh) =
                    unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
                let x = if what == "panel" { sw / 2 } else { sw - 200 };
                self.open_wheel_at(POINT { x, y: sh / 2 });
                let scale = self.wheel.scale();
                let Some(session) = &mut self.session else {
                    return;
                };
                session.set_inputs(vec![InputItem::File {
                    format: Some(Format::Mp4),
                    animated_gif: false,
                }]);
                // Rest on GIF, then hover the first panel row.
                let n = session.items().len();
                let gif = session
                    .items()
                    .iter()
                    .position(|op| *op == Operation::Convert(Format::Gif))
                    .unwrap_or(0);
                let s = sector(n, gif, scale);
                let (r, deg) = (
                    (s.inner_radius + s.outer_radius) / 2.0,
                    ((s.start_deg + s.end_deg) / 2.0).to_radians(),
                );
                session.hover(r * deg.sin(), -r * deg.cos(), scale, 0);
                session.tick(PANEL_DWELL_MS);
                let rows = session.panel().map_or(0, |p| p.set.choices.len());
                let rect = panel_rect(self.wheel.panel_side(), rows, scale);
                let row_y = rect.top + (30.0 + 20.0) * scale;
                session.hover((rect.left + rect.right) / 2.0, row_y, scale, 1000);
                self.selection = "12 MB".into();
                self.opened_at = ago(1).unwrap_or_else(Instant::now);
                self.sync_panel();
                self.panel_shown_at = ago(1).unwrap_or_else(Instant::now);
                self.render();
            }
            "progress" => {
                let row = |file: &str, op, fraction| crate::jobs::ProgressRow {
                    file: file.into(),
                    op,
                    fraction,
                };
                let rows = [
                    row("긴영상.mp4", Operation::Convert(Format::Webm), Some(0.37)),
                    row(
                        "여행 브이로그 2026 최종 편집본 진짜최종.mp4",
                        Operation::Convert(Format::Mp4),
                        Some(0.82),
                    ),
                    row("노래.flac", Operation::Convert(Format::Mp3), None),
                    row("움짤.gif", Operation::Convert(Format::Mp4), None),
                    row(
                        "a.mp4",
                        Operation::Tool(swisswheel_core::catalog::Tool::CompressVideo),
                        None,
                    ),
                    row("b.mp4", Operation::Convert(Format::Mkv), None),
                    row("c.mp4", Operation::Convert(Format::Mkv), None),
                ];
                self.progress_window.update(&rows);
                self.tray.set_busy(true, "SwissWheel · preview", Some(0.4));
            }
            "menu" | "menu-settings" => {
                let Some(bar) = &mut self.menubar else {
                    return;
                };
                let settings = what == "menu-settings";
                let mut cursor = POINT::default();
                unsafe {
                    let _ = GetCursorPos(&mut cursor);
                }
                bar.place(cursor, settings);
                bar.set_panel(settings);
                let config = swisswheel_core::config::Config::default();
                let rows = crate::menubar::rows(
                    &config,
                    settings.then_some(swisswheel_core::settings::SettingItem::WebpQuality),
                );
                let view = crate::menubar::MenuView {
                    selected: if settings { 0 } else { 1 },
                    rows: settings.then_some(rows.as_slice()),
                    row: settings.then_some(3),
                    hovered: if settings {
                        swisswheel_core::menu::MenuHit::Row(6)
                    } else {
                        swisswheel_core::menu::MenuHit::Tile(2)
                    },
                    notice: settings.then_some(CONFIG_NOTICE),
                    progress: 1.0,
                    panel_progress: 1.0,
                };
                bar.show();
                bar.render(&view);
            }
            other => log_error!("SWISSWHEEL_PREVIEW: 알 수 없는 값 {other}"),
        }
    }

    /// Milliseconds since the wheel opened: the session's clock for dwells.
    fn now_ms(&self) -> u64 {
        self.session_started.elapsed().as_millis() as u64
    }

    /// Brings the window region, the panel fade, and the dwell timer in line
    /// with the session's panel. Call after anything that may change it.
    fn sync_panel(&mut self) {
        let panel = self.session.as_ref().and_then(WheelSession::panel);
        let item = panel.map(|p| p.item);
        if item != self.panel_item {
            self.panel_item = item;
            self.panel_shown_at = Instant::now();
            if item.is_some() {
                unsafe { SetTimer(Some(self.hwnd), TIMER_ANIMATION, 16, None) }; // fade in
            }
        }
        self.wheel.set_panel(panel.map(|p| p.set.choices.len()));
        let pending = self
            .session
            .as_ref()
            .is_some_and(WheelSession::switch_pending);
        unsafe {
            if pending {
                SetTimer(Some(self.hwnd), TIMER_PANEL, PANEL_TICK_MS, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), TIMER_PANEL);
            }
        }
    }

    fn on_timer(&mut self, id: usize) {
        match id {
            TIMER_ENTER_TIMEOUT => {
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), TIMER_ENTER_TIMEOUT);
                }
                if self.session.as_ref().is_some_and(WheelSession::is_loading) {
                    // Not a file drag (window move, text selection, …).
                    hook::notify(DragEvent::EnterTimedOut);
                    self.close_wheel();
                }
            }
            TIMER_MOUSE_UP_CLOSE => self.close_wheel(),
            TIMER_ANIMATION => {
                self.render();
                if self.opened_at.elapsed() >= OPEN_ANIMATION
                    && self.panel_shown_at.elapsed() >= PANEL_FADE
                {
                    unsafe {
                        let _ = KillTimer(Some(self.hwnd), TIMER_ANIMATION);
                    }
                }
            }
            TIMER_PROGRESS => self.update_busy(),
            TIMER_MENU_ANIMATION => {
                self.render_menu();
                if self.menu.is_none()
                    || (self.menu_opened_at.elapsed() >= OPEN_ANIMATION
                        && self.menu_panel_at.elapsed() >= PANEL_FADE)
                {
                    unsafe {
                        let _ = KillTimer(Some(self.hwnd), TIMER_MENU_ANIMATION);
                    }
                }
            }
            TIMER_PANEL => {
                let now = self.now_ms();
                if self.session.as_mut().is_some_and(|s| s.tick(now)) {
                    self.sync_panel();
                    self.render();
                } else {
                    self.sync_panel(); // stops the timer once nothing is pending
                }
            }
            TIMER_MAINTENANCE => {
                self.wheel.release_if_idle(RENDERER_IDLE);
                if let Some(bar) = &mut self.menubar {
                    bar.release_if_idle(RENDERER_IDLE);
                }
                if !self.wheel.is_visible()
                    && self.menu.is_none()
                    && self.hooks_installed_at.elapsed() >= HOOK_REINSTALL
                {
                    match self.hooks.reinstall() {
                        Ok(()) => self.hooks_installed_at = Instant::now(),
                        Err(e) => log_error!("훅 재설치 실패: {e}"),
                    }
                }
            }
            _ => {}
        }
    }

    /// Offset of a screen point from the wheel center, in physical pixels.
    fn offset(&self, x: i32, y: i32) -> (f32, f32) {
        let c = self.wheel.center();
        ((x - c.x) as f32, (y - c.y) as f32)
    }

    fn effect(&self) -> DROPEFFECT {
        // Never DROPEFFECT_MOVE: the drag source would delete the original.
        match self.session.as_ref().map(WheelSession::hovered) {
            Some(Hit::Item(_) | Hit::Choice(_)) => DROPEFFECT_COPY,
            _ => DROPEFFECT_NONE,
        }
    }

    fn on_jobs_done(&mut self) {
        let reports: Vec<_> = self.reports.lock().unwrap().drain(..).collect();
        for report in reports {
            self.active_jobs = self.active_jobs.saturating_sub(1);
            let (title, body) = job_summary(&report.op, report.outputs.len(), &report.failures);
            log_info!(
                "{title}: 성공 {} / 실패 {}",
                report.outputs.len(),
                report.failures.len()
            );
            for failure in &report.failures {
                log_error!("{failure}");
            }
            if let Some(first) = report.outputs.first() {
                self.last_output = Some(first.clone());
            }
            if report.failures.is_empty() {
                self.tray.notify(&title, &body);
            } else {
                self.tray.warn(&title, &body);
            }
        }
        self.update_busy();
    }

    /// Tray tooltip and icon; the progress timer runs only while jobs do.
    fn update_busy(&mut self) {
        let active = self.active_jobs > 0;
        let progress = self.runner.as_ref().and_then(Runner::progress);
        let ring = progress.filter(|_| active && self.config.progress.tray_ring);
        self.tray
            .set_busy(active, &busy_tooltip(self.active_jobs, progress), ring);
        let rows = match &self.runner {
            Some(runner) if active && self.config.progress.window => runner.rows(),
            _ => Vec::new(),
        };
        self.progress_window.update(&rows);
        unsafe {
            if self.active_jobs > 0 {
                SetTimer(Some(self.hwnd), TIMER_PROGRESS, PROGRESS_INTERVAL_MS, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), TIMER_PROGRESS);
            }
        }
    }

    fn submit(&mut self, selection: Selection) {
        let Selection { op, preset } = selection;
        if let Operation::Action(action) = op {
            self.tray
                .notify(action.label(), "다음 버전에서 지원 예정입니다");
            return;
        }
        let inputs = std::mem::take(&mut self.dragged);
        log_info!("작업 시작: {} · 파일 {}개", op.label(), inputs.len());
        let Some(runner) = &self.runner else {
            return;
        };
        runner.submit(Job { op, preset, inputs });
        self.active_jobs += 1;
        self.update_busy();
    }
}

/// Center label for the dragged paths, e.g. "2.4 MB" or "3개 · 7.1 MB".
fn selection_summary(paths: &[PathBuf]) -> String {
    let (mut files, mut folders, mut bytes) = (0, 0, 0);
    for path in paths {
        match std::fs::metadata(path) {
            Ok(m) if m.is_dir() => folders += 1,
            Ok(m) => {
                files += 1;
                bytes += m.len();
            }
            Err(_) => files += 1, // still counted; its size just isn't known
        }
    }
    selection_label(files, folders, bytes)
}

/// How far an animation that started at `start` and lasts `length` has run,
/// 0.0 → 1.0.
fn fraction(start: Instant, length: Duration) -> f32 {
    (start.elapsed().as_secs_f32() / length.as_secs_f32()).min(1.0)
}

/// Mouse input on the desktop menu (from its window procedure).
pub fn on_menu_mouse(event: MenuMouse) {
    with_state(|s| s.on_menu_mouse(event));
}

// Drop target callbacks (called on the UI thread by OLE).

pub fn on_drag_enter(paths: Vec<PathBuf>, x: i32, y: i32) -> DROPEFFECT {
    with_state(|s| {
        let (dx, dy) = s.offset(x, y);
        let scale = s.wheel.scale();
        let now = s.now_ms();
        let session = s.session.as_mut()?;
        // DragEnter repeats each time the cursor re-enters; files are read once.
        let first_enter = session.is_loading();
        if first_enter {
            session.set_inputs(paths.iter().map(|p| probe::input_item(p)).collect());
        }
        session.hover(dx, dy, scale, now);
        if first_enter {
            s.selection = selection_summary(&paths);
            s.dragged = paths;
            s.opened_at = Instant::now(); // the wheel becomes visible now
            unsafe { SetTimer(Some(s.hwnd), TIMER_ANIMATION, 16, None) };
        }
        s.sync_panel();
        s.render();
        Some(s.effect())
    })
    .flatten()
    .unwrap_or(DROPEFFECT_NONE)
}

pub fn on_drag_over(x: i32, y: i32) -> DROPEFFECT {
    with_state(|s| {
        let (dx, dy) = s.offset(x, y);
        let scale = s.wheel.scale();
        let now = s.now_ms();
        let changed = s.session.as_mut()?.hover(dx, dy, scale, now);
        s.sync_panel();
        if changed {
            s.render();
        }
        Some(s.effect())
    })
    .flatten()
    .unwrap_or(DROPEFFECT_NONE)
}

pub fn on_drag_leave() {
    with_state(|s| {
        let now = s.now_ms();
        if let Some(session) = &mut s.session
            && session.hover(f32::MAX, f32::MAX, 1.0, now)
        {
            s.sync_panel();
            s.render();
        }
    });
}

pub fn on_drop(x: i32, y: i32) -> DROPEFFECT {
    with_state(|s| {
        let (dx, dy) = s.offset(x, y);
        let op = s
            .session
            .as_ref()
            .and_then(|session| session.drop_at(dx, dy, s.wheel.scale()));
        let dragged = std::mem::take(&mut s.dragged);
        s.close_wheel();
        match op {
            Some(op) => {
                s.dragged = dragged;
                s.submit(op);
                DROPEFFECT_COPY
            }
            None => DROPEFFECT_NONE,
        }
    })
    .unwrap_or(DROPEFFECT_NONE)
}

fn on_tray(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) {
    // NOTIFYICON_VERSION_4: event in LOWORD(lParam), anchor point in wParam.
    match (lparam.0 & 0xFFFF) as u32 {
        WM_CONTEXTMENU | WM_RBUTTONUP => {
            let pt = POINT {
                x: (wparam.0 & 0xFFFF) as i16 as i32,
                y: ((wparam.0 >> 16) & 0xFFFF) as i16 as i32,
            };
            show_menu(hwnd, pt);
        }
        NIN_BALLOONUSERCLICK => {
            if let Some(Some(path)) = with_state(|s| s.last_output.clone()) {
                let _ = std::process::Command::new("explorer.exe")
                    .arg(format!("/select,{}", path.display()))
                    .spawn();
            }
        }
        _ => {}
    }
}

fn show_menu(hwnd: HWND, pt: POINT) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return;
        };
        let _ = AppendMenuW(menu, MF_STRING, ID_MENU_EXIT, w!("종료"));
        // Required so the menu closes when the user clicks elsewhere.
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(menu, TPM_RIGHTBUTTON, pt.x, pt.y, None, hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
    }
}
