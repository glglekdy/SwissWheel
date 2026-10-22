//! The resident app: single instance, hidden host window, tray, hooks, the
//! wheel, and the job runner, all driven from one UI-thread message loop.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use swisswheel_core::catalog::Operation;
use swisswheel_core::config::{Config, Key};
use swisswheel_core::drag::{DragCommand, DragEvent, KeyBindings};
use swisswheel_core::ffmpeg_release;
use swisswheel_core::geometry::Hit;
use swisswheel_core::menu::{MenuAction, MenuHit, MenuSession, Tile};
use swisswheel_core::menu_geometry;
use swisswheel_core::notices::{Notice, NoticeAction, NoticeId, NoticeKind, Notices};
use swisswheel_core::restart;
use swisswheel_core::session::{Selection, WheelSession};
use swisswheel_core::settings::{self, Dir, SettingItem};
use swisswheel_core::summary::{
    balloon_body, busy_tooltip, idle_tooltip, job_summary, selection_label,
};
use swisswheel_core::update::{self as release, ReadyDecision, TileDecision, Version, text};
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
use windows::Win32::UI::Shell::{NIN_BALLOONUSERCLICK, ShellExecuteW};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, Result, w};

use crate::dnd::DropTarget;
use crate::engines::ffmpeg as ffmpeg_tools;
use crate::ffmpeg_install;
use crate::hook::{self, Hooks, WM_APP_DRAG, WM_APP_FOREGROUND};
use crate::jobs::{Job, JobReport, ProgressRow, Runner};
use crate::menubar::{self, MenuBar, MenuMouse, MenuView};
use crate::notice::{NoticeWindow, WM_APP_NOTICE, quiet_now};
use crate::probe;
use crate::tray::{Tray, WM_APP_TRAY};
use crate::update::{self, Outcome, swap};
use crate::wheel::{PanelView, WheelView, WheelWindow};
use crate::{autostart, config_file};

const ID_MENU_EXIT: usize = 1;
const WM_APP_JOB_DONE: u32 = WM_APP + 30;
/// The update thread finished; its outcome is in `update_slot`.
const WM_APP_UPDATE: u32 = WM_APP + 31;
/// The FFmpeg download thread finished; its outcome is in `ffmpeg_slot`.
const WM_APP_FFMPEG: u32 = WM_APP + 32;
/// How long a freshly updated app waits for the old one to exit.
const AFTER_UPDATE_WAIT: Duration = Duration::from_secs(15);
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
/// Gives up waiting for a key on a shortcut row.
const TIMER_MENU_CAPTURE: usize = 8;
/// Fires once to swap in a waiting update after the last job's notice.
const TIMER_SWAP: usize = 9;
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(5);
const PANEL_TICK_MS: u32 = 50;
/// How often the tray tooltip refreshes the progress of running jobs.
const PROGRESS_INTERVAL_MS: u32 = 500;
/// Fires once at the notice cards' next frame, timer-bar step, or expiry.
const TIMER_NOTICE: usize = 10;
/// Re-checks full screen / presentation mode while cards are shown or held.
const TIMER_QUIET: usize = 11;
const QUIET_INTERVAL_MS: u32 = 2000;
/// Render failures in a row after which the notice window is given up.
const NOTICE_FAILURES: u8 = 2;

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
    /// The key prompt card while a shortcut row waits for a key.
    capture_notice: Option<NoticeId>,
    /// The last key result card, closed when the next result is posted so
    /// repeated rejections never stack.
    capture_result: Option<NoticeId>,
    /// Foreground watch while the menu is open (it closes when another
    /// window comes forward).
    foreground_hook: Option<HWINEVENTHOOK>,
    /// The progress card and notice cards above the tray; `None` when it
    /// couldn't be made or drawn (or `SWISSWHEEL_FORCE_BALLOON=1`): notices
    /// then go to balloons.
    notice_window: Option<NoticeWindow>,
    /// The notice cards' rules and timers.
    notices: Notices,
    /// The notice window's render failures in a row.
    notice_failures: u8,
    /// The progress card's rows, as `update_busy` last read them.
    progress_rows: Vec<ProgressRow>,
    /// `progress_rows` lists files of running jobs (not only a download).
    progress_jobs: bool,
    /// The last frame showed job rows on the progress card (the last job's
    /// card may take its slot).
    progress_shown: bool,
    /// `TIMER_QUIET` runs.
    quiet_watch: bool,
    /// Title of the last finished job, for the idle tooltip.
    last_result: Option<String>,
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
    /// The running update check or download, if any.
    update: Option<update::Handle>,
    update_slot: Arc<Mutex<Option<Outcome>>>,
    /// A verified download waiting for the running jobs to finish.
    pending_swap: Option<Version>,
    /// The running FFmpeg check, download, or install, if any.
    ffmpeg: Option<ffmpeg_install::Handle>,
    ffmpeg_slot: Arc<Mutex<Option<ffmpeg_install::Outcome>>>,
    /// Balloon fallback: the folder of the last balloon, opened when it is clicked.
    last_output: Option<PathBuf>,
    hooks_installed_at: Instant,
    taskbar_created: u32,
    /// The `notices` preview's cards and progress rows.
    #[cfg(debug_assertions)]
    preview_notices: Option<(Notices, Vec<ProgressRow>)>,
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
    // Started by an update or the restart tile: the old app holds the
    // single-instance mutex until it exits, so wait for it before taking it.
    let args: Vec<String> = std::env::args().collect();
    let after_update = release::after_update_pid(args.clone());
    let restarted = restart::restarted_pid(args);
    let old_exited = after_update
        .or(restarted)
        .map(|pid| swap::wait_for_exit(pid, AFTER_UPDATE_WAIT));
    let _instance = unsafe { CreateMutexW(None, true, w!("Local\\SwissWheel.SingleInstance"))? };
    // A debug preview may run beside an installed copy: it only draws.
    let previewing = previewing(std::env::var("SWISSWHEEL_PREVIEW").ok());
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
    if let Some(exited) = old_exited {
        let after = if restarted.is_some() {
            "재시작"
        } else {
            "업데이트"
        };
        log_info!("{after} 뒤 시작 (이전 프로세스 종료: {exited})");
    }
    if let Ok(exe) = std::env::current_exe() {
        swap::cleanup(&exe);
    }
    ffmpeg_install::cleanup(&ffmpeg_tools::install_dirs());
    // Shown once the app state (and the notice window) exists.
    let mut startup = Vec::new();
    if after_update.is_some() {
        startup.push(info(release::TITLE, &text::updated(update::current())));
    }
    if restarted.is_some() {
        startup.push(info(restart::TITLE, restart::text::RESTARTED));
    }
    if let Some(w) = &warning {
        log_error!("설정 파일 오류: {w}");
        startup.push(warning_notice(
            "설정 파일 오류",
            &format!("{w}\n기본 설정으로 실행합니다"),
        ));
    }
    if let Some(enable) = autostart_fix(
        autostart::is_enabled(),
        &config,
        warning.is_none(),
        previewing,
    ) && let Err(e) = autostart::set(enable)
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
    let notice_window = if std::env::var("SWISSWHEEL_FORCE_BALLOON").is_ok_and(|v| v == "1") {
        log_info!("SWISSWHEEL_FORCE_BALLOON: 알림 카드 대신 풍선 알림을 씁니다");
        None
    } else {
        NoticeWindow::new(hwnd)
            .inspect_err(|e| log_error!("알림 창을 만들 수 없어 풍선 알림을 씁니다: {e}"))
            .ok()
    };
    let mut notices = Notices::new();
    notices.set_do_not_disturb(config.notifications.do_not_disturb);
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
        capture_notice: None,
        capture_result: None,
        foreground_hook: None,
        notice_window,
        notices,
        notice_failures: 0,
        progress_rows: Vec::new(),
        progress_jobs: false,
        progress_shown: false,
        quiet_watch: false,
        last_result: None,
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
        update: None,
        update_slot: Arc::new(Mutex::new(None)),
        pending_swap: None,
        ffmpeg: None,
        ffmpeg_slot: Arc::new(Mutex::new(None)),
        last_output: None,
        hooks_installed_at: Instant::now(),
        taskbar_created,
        #[cfg(debug_assertions)]
        preview_notices: None,
    }));
    with_state(|s| {
        for notice in startup {
            s.notify(notice);
        }
    });
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
        WM_APP_UPDATE => {
            with_state(AppState::on_update_done);
        }
        WM_APP_FFMPEG => {
            with_state(AppState::on_ffmpeg_done);
        }
        WM_APP_FOREGROUND => {
            with_state(|s| s.on_foreground(HWND(wparam.0 as *mut _)));
        }
        // Hover and clicks on the notice cards.
        WM_APP_NOTICE => {
            with_state(|s| s.on_notice(wparam, lparam));
        }
        // Monitors, DPI, or the work area changed: the bar's and the notice
        // window's placement is stale.
        WM_DISPLAYCHANGE | WM_DPICHANGED => {
            with_state(|s| {
                s.close_menu();
                s.reposition_notices();
            });
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
        }
        WM_SETTINGCHANGE => {
            let work_area = wparam.0 == SPI_SETWORKAREA.0 as usize;
            with_state(|s| {
                if work_area {
                    s.close_menu();
                    s.reposition_notices();
                }
                // "ImmersiveColorSet" when the Windows mode changes; the tray
                // checks the registry itself, so any change is enough. The
                // cards pick up the theme and transparency when redrawn.
                s.tray.refresh_theme();
                s.render_notices();
            });
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
            DragCommand::MenuCapture(vk) => match &mut self.menu {
                Some(menu) => {
                    let action = menu.capture(vk);
                    self.on_menu_action(action);
                }
                None => hook::notify(DragEvent::MenuClosed),
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
        self.disarm_capture();
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
        let shape = menu
            .focus()
            .map(|_| menu_geometry::PanelShape::of(menu.category()));
        let hit = |x, y| menu_geometry::hit(bar.layout(), shape, x, y);
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
            MenuAction::Notice(Tile::Update) => {
                self.close_menu();
                self.start_update();
            }
            MenuAction::Notice(Tile::Download) => {
                self.close_menu();
                self.start_ffmpeg();
            }
            MenuAction::Notice(Tile::Restart) => {
                self.close_menu();
                self.restart();
            }
            MenuAction::Notice(tile) => {
                self.close_menu();
                self.notify(info(tile.label(), "다음 버전에서 지원 예정입니다"));
            }
            MenuAction::Close => self.close_menu(),
            MenuAction::StartCapture => {
                // Ends any earlier row's capture first (a click on another
                // row's key), then arms this row with its own card.
                self.disarm_capture();
                if let Some(item) = self.menu.as_ref().and_then(MenuSession::capture_item) {
                    self.arm_capture(item);
                }
                self.render_menu();
            }
            MenuAction::CancelCapture => {
                self.disarm_capture();
                self.render_menu();
            }
            MenuAction::Bind(item, vk) => {
                match Key::from_vk(vk) {
                    None => {
                        // Posted first: if it pushes the prompt past the
                        // card limit, arming posts the prompt again.
                        self.post_key_result("이 키는 쓸 수 없습니다", "");
                        // Keeps waiting on the same card, its timer restarted.
                        if let Some(menu) = &mut self.menu {
                            menu.rearm();
                        }
                        self.arm_capture(item);
                    }
                    Some(key) => {
                        self.disarm_capture();
                        let bound = settings::bind(&self.config, item, key);
                        if bound.config != self.config {
                            self.apply_config(bound.config, item);
                        }
                        if let Some((other, k)) = bound.swapped {
                            let body = format!(
                                "{}는 이제 {}입니다",
                                other.label(),
                                settings::key_label(k)
                            );
                            self.post_key_result("두 단축키를 맞바꿨습니다", &body);
                        }
                    }
                }
                self.render_menu();
            }
        }
    }

    /// Has the hook hand over the next key press for `item`, for up to
    /// `CAPTURE_TIMEOUT`, and shows the key prompt card. Armed again after a
    /// rejected key, the same card restarts its bar with the timeout.
    fn arm_capture(&mut self, item: SettingItem) {
        hook::set_capture(true);
        unsafe {
            SetTimer(
                Some(self.hwnd),
                TIMER_MENU_CAPTURE,
                CAPTURE_TIMEOUT.as_millis() as u32,
                None,
            )
        };
        let now = Instant::now();
        match self.capture_notice {
            Some(id) if self.notices.restart(id, now) => self.render_notices(),
            // Gone (clicked away, or dropped while quiet): a new card.
            _ => self.capture_notice = self.notify(key_prompt(item)),
        }
    }

    /// Shows a key result card in place of the previous one, if still up.
    fn post_key_result(&mut self, title: &str, body: &str) {
        if let Some(id) = self.capture_result.take() {
            self.notices.dismiss(id, Instant::now());
        }
        self.capture_result = self.notify(key_result(title, body));
    }

    /// Stops waiting for a key and closes the key prompt card.
    fn disarm_capture(&mut self) {
        hook::set_capture(false);
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_MENU_CAPTURE);
        }
        if let Some(id) = self.capture_notice.take() {
            self.notices.dismiss(id, Instant::now());
            self.render_notices();
        }
    }

    /// Stops waiting for a key: the timeout, or a click on the key prompt card.
    fn cancel_capture(&mut self) {
        self.disarm_capture();
        if self.menu.as_mut().is_some_and(MenuSession::cancel_capture) {
            self.render_menu();
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
        let progress_changed = new.progress != old.progress;
        let accent_changed = new.appearance.accent != old.appearance.accent;
        if new.start_with_windows != old.start_with_windows
            && let Err(e) = autostart::set(new.start_with_windows)
        {
            log_error!("시작 프로그램 등록 실패: {e}");
            new.start_with_windows = self.config.start_with_windows;
            self.notify(warning_notice(
                "시작 프로그램 등록에 실패했습니다",
                &e.message(),
            ));
        }
        self.notices
            .set_do_not_disturb(new.notifications.do_not_disturb);
        let before = std::mem::replace(&mut self.config, new);
        if progress_changed {
            self.update_busy(); // redraws the notice window too
        } else if accent_changed {
            self.render_notices();
        }
        if !self.config_writable {
            return; // see CONFIG_NOTICE
        }
        match config_file::save(&self.config_dir, &before, &self.config) {
            Ok(()) => self.save_failed = None,
            Err(e) => {
                log_error!("설정 저장 실패: {e}");
                if self.save_failed.is_none() {
                    self.notify(warning_notice("설정을 저장하지 못했습니다", &e.to_string()));
                }
                self.save_failed = Some(item);
            }
        }
    }

    fn render_menu(&mut self) {
        let (Some(menu), Some(bar)) = (&self.menu, &mut self.menubar) else {
            return;
        };
        let panel_open = menu.focus().is_some();
        if panel_open != bar.panel_open() {
            bar.set_panel(panel_open);
            if panel_open {
                self.menu_panel_at = Instant::now();
                unsafe { SetTimer(Some(self.hwnd), TIMER_MENU_ANIMATION, 16, None) };
            }
        }
        let category = menu.category();
        let rows = menubar::rows(&self.config, category, self.save_failed);
        let view = MenuView {
            selected: menu.tile(),
            rows: panel_open.then_some(rows.as_slice()),
            category: settings::CATEGORIES
                .iter()
                .position(|&c| c == category)
                .unwrap_or(0),
            focus: menu.focus(),
            capturing: menu.capturing(),
            hovered: menu.hovered(),
            accent: self.config.appearance.accent,
            notice: (!self.config_writable).then_some(CONFIG_NOTICE),
            progress: fraction(self.menu_opened_at, OPEN_ANIMATION),
            panel_progress: fraction(self.menu_panel_at, PANEL_FADE),
        };
        if !bar.render(&view) {
            // An invisible menu would still take the arrows and Enter.
            log_error!("메뉴를 그릴 수 없어 바탕화면 메뉴를 끕니다");
            self.close_menu();
            hook::set_menu_enabled(false);
        }
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
        session.set_tools(self.config.tools.clone());
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
            default: p.default,
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
            accent: self.config.appearance.accent,
            panel,
        };
        self.wheel.render(&view);
    }

    /// Debug builds only. `SWISSWHEEL_PREVIEW` = `panel`, `panel-left`,
    /// `progress`, `notices`, `menu`, `menu-settings`, `menu-tools`, `menu-capture`,
    /// `menu-notifications`, or `menu-general` shows that UI at startup
    /// without a drag or key press, so the drawing can be checked by eye.
    #[cfg(debug_assertions)]
    fn preview(&mut self, what: &str) {
        use swisswheel_core::catalog::{InputItem, Operation};
        use swisswheel_core::format::Format;
        use swisswheel_core::geometry::{panel_rect, sector};
        use swisswheel_core::session::PANEL_DWELL_MS;

        let ago = |secs| Instant::now().checked_sub(Duration::from_secs(secs));
        let row = |file: &str, op, fraction| ProgressRow {
            title: swisswheel_core::summary::progress_title(file, &op),
            fraction,
        };
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
            // Checks SWISSWHEEL_UPDATE_URL (a local server) as if the tile were pressed.
            "update" => self.start_update(),
            // Checks SWISSWHEEL_FFMPEG_URL (a local server) as if the tile were pressed.
            "ffmpeg" => self.start_ffmpeg(),
            "progress" => {
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
                if let Some(window) = &mut self.notice_window
                    && let Err(e) = window.render(&rows, &[], self.config.appearance.accent)
                {
                    log_error!("알림 창 그리기 실패: {e}");
                }
                self.tray.set_busy(true, "SwissWheel · preview", Some(0.4));
            }
            "notices" => {
                let rows = vec![
                    row("photo.png", Operation::Convert(Format::Jpg), Some(0.64)),
                    row("여행 영상.mp4", Operation::Convert(Format::Gif), None),
                ];
                self.preview_notices = Some((Notices::new(), rows));
                self.render_preview_notices();
            }
            "menu" | "menu-settings" | "menu-tools" | "menu-capture" | "menu-notifications"
            | "menu-general" => {
                use swisswheel_core::menu::{Focus, MenuHit};
                use swisswheel_core::settings::{CATEGORIES, Category, SettingItem};

                let Some(bar) = &mut self.menubar else {
                    return;
                };
                let panel = what != "menu";
                let notice = what == "menu-settings";
                let mut cursor = POINT::default();
                unsafe {
                    let _ = GetCursorPos(&mut cursor);
                }
                bar.place(cursor, notice);
                bar.set_panel(panel);
                // Defaults, but the accent from config.toml.
                let mut config = swisswheel_core::config::Config::default();
                config.appearance.accent = self.config.appearance.accent;
                let (category, focus, hovered, failed) = match what {
                    "menu-settings" => (
                        Category::Keys,
                        Some(Focus::Row(0)),
                        MenuHit::Row(2),
                        Some(SettingItem::CycleKey),
                    ),
                    "menu-tools" => (Category::Tools, Some(Focus::Tabs), MenuHit::Tab(2), None),
                    "menu-capture" => (Category::Keys, Some(Focus::Row(1)), MenuHit::Panel, None),
                    "menu-notifications" => (
                        Category::Notifications,
                        Some(Focus::Row(2)),
                        MenuHit::Row(0),
                        None,
                    ),
                    "menu-general" => {
                        (Category::General, Some(Focus::Row(1)), MenuHit::Panel, None)
                    }
                    _ => (Category::Keys, None, MenuHit::Tile(2), None),
                };
                let rows = crate::menubar::rows(&config, category, failed);
                let capturing = what == "menu-capture";
                let view = crate::menubar::MenuView {
                    selected: if panel { 0 } else { 1 },
                    rows: panel.then_some(rows.as_slice()),
                    category: CATEGORIES.iter().position(|&c| c == category).unwrap_or(0),
                    focus,
                    capturing,
                    hovered,
                    accent: config.appearance.accent,
                    notice: notice.then_some(CONFIG_NOTICE),
                    progress: 1.0,
                    panel_progress: 1.0,
                };
                bar.show();
                bar.render(&view);
                // The prompt names the row the preview shows waiting.
                if let (true, Some(Focus::Row(i))) = (capturing, focus)
                    && let Some(&item) = category.items().get(i)
                {
                    self.notify(key_prompt(item));
                }
            }
            other => log_error!("SWISSWHEEL_PREVIEW: 알 수 없는 값 {other}"),
        }
    }

    /// Steps the `notices` preview: posts its three cards again once all
    /// have closed, draws a frame, and sets the timer for the next one.
    #[cfg(debug_assertions)]
    fn render_preview_notices(&mut self) {
        use swisswheel_core::notices::{Notice, NoticeAction, NoticeKind};

        let Some((notices, rows)) = &mut self.preview_notices else {
            return;
        };
        let now = Instant::now();
        notices.tick(now);
        if notices.cards(now).is_empty() {
            let folder = || Some(NoticeAction::OpenFolder(std::env::temp_dir()));
            let cards = [
                (2, key_prompt(SettingItem::OpenKey)),
                (
                    1,
                    Notice {
                        kind: NoticeKind::Failed,
                        title: "1개 파일을 변환하지 못했습니다".into(),
                        // Three lines' worth: the second ends in "…".
                        body: "여행 브이로그 2026 최종 편집본 진짜최종.mp4: 지원하지 않는 오디오 코덱입니다 (pcm_s24le)\n성공 1개".into(),
                        action: folder(),
                    },
                ),
                (
                    0,
                    Notice {
                        kind: NoticeKind::Done,
                        title: "2개 파일을 변환했습니다".into(),
                        body: "photo.jpg 외 1개 · JPG, GIF".into(),
                        action: folder(),
                    },
                ),
            ];
            for (ago, notice) in cards {
                let at = now.checked_sub(Duration::from_secs(ago)).unwrap_or(now);
                notices.post(notice, at);
            }
        }
        let accent = self.config.appearance.accent;
        if let Some(window) = &mut self.notice_window
            && let Err(e) = window.render(rows, &notices.cards(now), accent)
        {
            log_error!("알림 창 그리기 실패: {e}");
        }
        unsafe {
            match notices.next_wake(now) {
                Some(at) => {
                    let ms = at.saturating_duration_since(now).as_millis().max(1) as u32;
                    SetTimer(Some(self.hwnd), TIMER_NOTICE, ms, None);
                }
                None => {
                    let _ = KillTimer(Some(self.hwnd), TIMER_NOTICE);
                }
            }
        }
    }

    /// Hover and clicks on the `notices` preview's cards.
    #[cfg(debug_assertions)]
    fn on_preview_notice(&mut self, wparam: WPARAM, lparam: LPARAM) {
        let Some((notices, _)) = &mut self.preview_notices else {
            return;
        };
        let now = Instant::now();
        match wparam.0 {
            0 => notices.hover(u64::try_from(lparam.0).ok(), now),
            _ => {
                let action = notices.click(lparam.0 as u64, now);
                log_info!("미리보기 카드 클릭: {action:?}");
            }
        }
        self.render_preview_notices();
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
            TIMER_NOTICE => {
                #[cfg(debug_assertions)]
                if self.preview_notices.is_some() {
                    self.render_preview_notices();
                    return;
                }
                self.render_notices();
            }
            TIMER_QUIET => {
                let was = self.notices.quiet();
                self.notices.set_quiet(quiet_now(), Instant::now());
                if self.notices.quiet() != was {
                    log_info!("전체 화면 등으로 알림 대기: {}", self.notices.quiet());
                }
                self.render_notices(); // also stops this timer once nothing is shown or held
            }
            TIMER_SWAP => {
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), TIMER_SWAP);
                }
                // A job started meanwhile: its end sets the timer again.
                if self.active_jobs == 0
                    && let Some(version) = self.pending_swap.take()
                {
                    self.apply_swap(version);
                }
            }
            TIMER_MENU_CAPTURE => self.cancel_capture(),
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
            let kind = if report.failures.is_empty() {
                NoticeKind::Done
            } else {
                NoticeKind::Failed
            };
            self.last_result = Some(title.clone());
            let notice = Notice {
                kind,
                title,
                body,
                action: report
                    .outputs
                    .first()
                    .cloned()
                    .map(NoticeAction::OpenFolder),
            };
            self.notify_job(notice, self.active_jobs == 0);
        }
        if let Some(delay) =
            release::on_jobs_finished(self.pending_swap.is_some(), self.active_jobs)
        {
            unsafe { SetTimer(Some(self.hwnd), TIMER_SWAP, delay.as_millis() as u32, None) };
        }
        self.update_busy(); // draws the new cards with the remaining rows
    }

    /// Shows a notice: a card, or a balloon without the notice window.
    /// The card's id, or `None` when it became a balloon.
    fn notify(&mut self, notice: Notice) -> Option<NoticeId> {
        if self.notice_window.is_none() {
            self.balloon(&notice);
            return None;
        }
        let id = self.notices.post(notice, Instant::now());
        self.render_notices();
        // A window given up on just now showed this notice as a balloon.
        self.notice_window.as_ref().map(|_| id)
    }

    /// Shows a finished job's notice; the last job's card may take the
    /// progress card's slot. `None` when do-not-disturb dropped it or it
    /// became a balloon. The caller redraws (see `on_jobs_done`), so that
    /// jobs finishing together are drawn once, against fresh progress rows.
    fn notify_job(&mut self, notice: Notice, last_job: bool) -> Option<NoticeId> {
        let title = notice.title.clone();
        let id = if self.notice_window.is_none() {
            if !self.config.notifications.do_not_disturb {
                self.balloon(&notice);
            }
            None
        } else {
            self.notices
                .post_job(notice, last_job, self.progress_shown, Instant::now())
        };
        if self.config.notifications.do_not_disturb {
            log_info!("방해 금지: 알림을 띄우지 않습니다 ({title})");
        }
        id
    }

    /// The balloon fallback; clicking it opens the notice's folder, if any,
    /// and its body says so (the card's action line in balloon form).
    fn balloon(&mut self, notice: &Notice) {
        self.last_output = match &notice.action {
            Some(NoticeAction::OpenFolder(path)) => Some(path.clone()),
            _ => None,
        };
        let body = balloon_body(&notice.body, self.last_output.is_some());
        match notice.kind {
            NoticeKind::Failed | NoticeKind::Warning => self.tray.warn(&notice.title, &body),
            _ => self.tray.notify(&notice.title, &body),
        }
    }

    /// Hover and clicks on the notice cards (`WM_APP_NOTICE`).
    fn on_notice(&mut self, wparam: WPARAM, lparam: LPARAM) {
        #[cfg(debug_assertions)]
        if self.preview_notices.is_some() {
            self.on_preview_notice(wparam, lparam);
            return;
        }
        let now = Instant::now();
        match wparam.0 {
            0 => self.notices.hover(u64::try_from(lparam.0).ok(), now),
            _ => match self.notices.click(lparam.0 as NoticeId, now) {
                Some(NoticeAction::OpenFolder(path)) => open_folder(&path),
                Some(NoticeAction::CancelCapture) => self.cancel_capture(),
                None => {}
            },
        }
        self.render_notices(); // a card left alone resumes its timer
    }

    /// Display, DPI, or work area changed: moves the notice window and redraws it.
    fn reposition_notices(&mut self) {
        if let Some(window) = &mut self.notice_window {
            window.reposition();
        }
        self.render_notices();
    }

    /// Draws the progress card and the notice cards, then sets the notice
    /// timer for the next frame and the quiet check. Two failed renders in a
    /// row give up on the window: notices go to balloons from then on.
    fn render_notices(&mut self) {
        #[cfg(debug_assertions)]
        if self.preview_notices.is_some() {
            return;
        }
        let Some(window) = &mut self.notice_window else {
            return;
        };
        let now = Instant::now();
        self.notices.tick(now);
        // Not watching yet: check before anything shows over a full-screen app.
        if !self.quiet_watch
            && (!self.progress_rows.is_empty() || !self.notices.cards(now).is_empty())
        {
            self.notices.set_quiet(quiet_now(), now);
        }
        let quiet = self.notices.quiet();
        let cards = self.notices.cards(now);
        // Quiet hides the whole window, the progress card too.
        let rows: &[ProgressRow] = if quiet { &[] } else { &self.progress_rows };
        match window.render(rows, &cards, self.config.appearance.accent) {
            Ok(()) => {
                self.notice_failures = 0;
                // Rows of an update or FFmpeg download don't count: that
                // card stays up after the last job.
                self.progress_shown = !quiet && self.progress_jobs;
            }
            Err(e) => {
                self.notice_failures += 1;
                self.progress_shown = false;
                if self.notice_failures >= NOTICE_FAILURES {
                    log_error!("알림 창을 그릴 수 없어 풍선 알림으로 바꿉니다: {e}");
                    window.hide();
                    self.notice_window = None;
                    unsafe {
                        let _ = KillTimer(Some(self.hwnd), TIMER_NOTICE);
                        let _ = KillTimer(Some(self.hwnd), TIMER_QUIET);
                    }
                    self.quiet_watch = false;
                    // The newest card (shown or held) goes on as a balloon,
                    // its folder and all; a balloon replaces the one before,
                    // so the rest are only logged.
                    let mut left = self.notices.take_all(now);
                    if let Some(newest) = left.pop() {
                        self.balloon(&newest);
                    }
                    if !left.is_empty() {
                        log_error!("알림 {}개를 표시하지 못하고 버렸습니다", left.len());
                    }
                    return;
                }
                log_error!("알림 창 그리기 실패: {e}");
            }
        }
        let watch = quiet || !rows.is_empty() || !cards.is_empty();
        unsafe {
            match self.notices.next_wake(now) {
                Some(at) => {
                    let ms = at.saturating_duration_since(now).as_millis().max(1) as u32;
                    SetTimer(Some(self.hwnd), TIMER_NOTICE, ms, None);
                }
                // A failed frame is tried again soon.
                None if self.notice_failures > 0 => {
                    SetTimer(Some(self.hwnd), TIMER_NOTICE, 50, None);
                }
                None => {
                    let _ = KillTimer(Some(self.hwnd), TIMER_NOTICE);
                }
            }
            if watch && !self.quiet_watch {
                SetTimer(Some(self.hwnd), TIMER_QUIET, QUIET_INTERVAL_MS, None);
            } else if !watch && self.quiet_watch {
                let _ = KillTimer(Some(self.hwnd), TIMER_QUIET);
            }
        }
        self.quiet_watch = watch;
    }

    fn start_update(&mut self) {
        let background = self.update.is_some() || self.ffmpeg.is_some();
        match release::on_tile(background, self.active_jobs) {
            TileDecision::Refuse(notice) => {
                self.notify(info(release::TITLE, notice));
            }
            TileDecision::Start => {
                self.update = Some(update::spawn(
                    self.hwnd.0 as isize,
                    WM_APP_UPDATE,
                    Arc::clone(&self.update_slot),
                ));
                self.update_busy();
            }
        }
    }

    fn on_update_done(&mut self) {
        self.update = None;
        let outcome = self.update_slot.lock().unwrap().take();
        let title = release::TITLE;
        let notice = match outcome {
            None | Some(Outcome::Declined) => None,
            Some(Outcome::UpToDate) => Some(info(title, &text::up_to_date(update::current()))),
            Some(Outcome::CheckFailed) => Some(warning_notice(title, text::CHECK_FAILED)),
            Some(Outcome::MissingAssets) => Some(warning_notice(title, text::NO_ASSETS)),
            Some(Outcome::DownloadFailed) => Some(warning_notice(title, text::DOWNLOAD_FAILED)),
            Some(Outcome::Corrupt) => Some(warning_notice(title, text::CORRUPT)),
            Some(Outcome::NotWritable { page_url }) => {
                open_page(&page_url);
                None
            }
            Some(Outcome::Ready { version }) => match release::on_ready(self.active_jobs) {
                ReadyDecision::SwapNow => {
                    self.apply_swap(version);
                    None
                }
                ReadyDecision::WaitForJobs => {
                    self.pending_swap = Some(version);
                    Some(info(title, text::WAITING))
                }
            },
        };
        if let Some(notice) = notice {
            self.notify(notice);
        }
        self.update_busy();
    }

    fn background_busy(&self) -> bool {
        background_busy(
            self.update.is_some(),
            self.ffmpeg.is_some(),
            self.pending_swap.is_some(),
        )
    }

    fn start_ffmpeg(&mut self) {
        let ready = ffmpeg_tools::locate().is_some();
        let busy = self.background_busy();
        match ffmpeg_release::on_download_tile(ready, busy) {
            TileDecision::Refuse(notice) => {
                self.notify(info(ffmpeg_release::TITLE, notice));
            }
            TileDecision::Start => {
                self.ffmpeg = Some(ffmpeg_install::spawn(
                    self.hwnd.0 as isize,
                    WM_APP_FFMPEG,
                    Arc::clone(&self.ffmpeg_slot),
                ));
                self.update_busy();
            }
        }
    }

    fn on_ffmpeg_done(&mut self) {
        use ffmpeg_install::Outcome as O;
        use ffmpeg_release::text as t;
        self.ffmpeg = None;
        let outcome = self.ffmpeg_slot.lock().unwrap().take();
        let title = ffmpeg_release::TITLE;
        let notice = match outcome {
            None | Some(O::Declined) => None,
            Some(O::Installed { tag }) => Some(info(title, &t::installed(&tag))),
            Some(O::CheckFailed) => Some(warning_notice(title, t::CHECK_FAILED)),
            Some(O::MissingAsset) => Some(warning_notice(title, t::NO_ASSET)),
            Some(O::DownloadFailed) => Some(warning_notice(title, t::DOWNLOAD_FAILED)),
            Some(O::Corrupt) => Some(warning_notice(title, t::CORRUPT)),
            Some(O::InstallFailed) => Some(warning_notice(title, t::INSTALL_FAILED)),
        };
        if let Some(notice) = notice {
            self.notify(notice);
        }
        self.update_busy();
    }

    /// Starts a fresh copy that waits for this one to exit, then closes
    /// normally (posted, like `apply_swap`).
    fn restart(&mut self) {
        match restart::on_tile(self.active_jobs, self.background_busy()) {
            TileDecision::Refuse(notice) => {
                self.notify(info(restart::TITLE, notice));
            }
            TileDecision::Start => {
                match std::env::current_exe().and_then(|exe| swap::launch_restarted(&exe)) {
                    Ok(()) => {
                        log_info!("재시작합니다");
                        unsafe {
                            let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                        }
                    }
                    Err(e) => {
                        log_error!("재시작 실패: {e}");
                        self.notify(warning_notice(restart::TITLE, restart::text::FAILED));
                    }
                }
            }
        }
    }

    /// Puts the verified download in place and starts it; this app then
    /// closes normally (posted: closing inside `with_state` would re-enter it).
    fn apply_swap(&mut self, version: Version) {
        let result =
            std::env::current_exe().and_then(|exe| swap::swap(&exe, swap::launch_after_update));
        match result {
            Ok(()) => {
                log_info!("업데이트: v{version}으로 교체했습니다. 다시 시작합니다");
                unsafe {
                    let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            }
            Err(e) => {
                log_error!("업데이트 교체 실패: {e}");
                self.notify(warning_notice(release::TITLE, text::SWAP_FAILED));
            }
        }
    }

    /// Tray tooltip and icon; the progress timer runs only while jobs do.
    fn update_busy(&mut self) {
        let active = self.active_jobs > 0;
        // The update's or FFmpeg's download row, once the user said yes (one
        // runs at a time), and what the tooltip says is downloading.
        let update_row = self.update.as_ref().and_then(|handle| {
            let version = (*handle.progress.version.lock().unwrap())?;
            let fraction = handle.progress.fraction.load(Ordering::Relaxed) as f32 / 10000.0;
            let row = ProgressRow {
                title: format!("SwissWheel v{version} · 업데이트"),
                fraction: Some(fraction),
            };
            Some((row, "업데이트"))
        });
        let ffmpeg_row = self.ffmpeg.as_ref().and_then(|handle| {
            let tag = handle.progress.tag.lock().unwrap().clone()?;
            let fraction = handle.progress.fraction.load(Ordering::Relaxed) as f32 / 10000.0;
            let row = ProgressRow {
                title: format!("FFmpeg {tag} · 다운로드"),
                fraction: Some(fraction),
            };
            Some((row, "FFmpeg"))
        });
        let (download, what) = update_row
            .or(ffmpeg_row)
            .map_or((None, ""), |(row, what)| (Some(row), what));
        let progress = if active {
            self.runner.as_ref().and_then(Runner::progress)
        } else {
            download.as_ref().and_then(|row| row.fraction)
        };
        let busy = active || download.is_some();
        let ring = progress.filter(|_| busy && self.config.progress.tray_ring);
        let tooltip = match (&download, progress) {
            (Some(_), Some(p)) if !active => {
                format!("SwissWheel · {what} 받는 중 · {}%", (p * 100.0).round())
            }
            _ if !busy => idle_tooltip(self.last_result.as_deref()),
            _ => busy_tooltip(self.active_jobs, progress),
        };
        self.tray.set_busy(busy, &tooltip, ring);
        let mut rows = match &self.runner {
            Some(runner) if active && self.config.progress.window => runner.rows(),
            _ => Vec::new(),
        };
        self.progress_jobs = !rows.is_empty();
        if self.config.progress.window {
            rows.extend(download);
        }
        self.progress_rows = rows;
        self.render_notices();
        unsafe {
            // While an update or FFmpeg thread runs too: the timer notices
            // when its download starts.
            if busy || self.update.is_some() || self.ffmpeg.is_some() {
                SetTimer(Some(self.hwnd), TIMER_PROGRESS, PROGRESS_INTERVAL_MS, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), TIMER_PROGRESS);
            }
        }
    }

    fn submit(&mut self, selection: Selection) {
        let Selection { op, preset } = selection;
        if let Operation::Action(action) = op {
            self.notify(info(action.label(), "다음 버전에서 지원 예정입니다"));
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

/// What the Run entry should be set to at startup, if anything: the setting,
/// when it differs from what is `registered` (e.g. the exe moved). Not when
/// the config file was unreadable (its defaults aren't the user's choice) or
/// for a debug preview (it would register the debug build).
fn autostart_fix(
    registered: bool,
    config: &Config,
    config_ok: bool,
    previewing: bool,
) -> Option<bool> {
    (config_ok && !previewing && registered != config.start_with_windows)
        .then_some(config.start_with_windows)
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
        // Only balloons of the fallback; the cards handle their own clicks.
        NIN_BALLOONUSERCLICK => {
            if let Some(Some(path)) = with_state(|s| s.last_output.clone()) {
                open_folder(&path);
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

/// Debug previews draw beside an installed copy, so they skip the
/// single-instance check and autostart; the update preview doesn't, since it
/// exercises the real restart hand-off.
fn previewing(preview: Option<String>) -> bool {
    cfg!(debug_assertions) && preview.is_some_and(|what| what != "update")
}

/// An update or FFmpeg thread runs, or a verified update waits to swap in
/// (which restarts the app): the download and restart tiles hold off.
fn background_busy(update: bool, ffmpeg: bool, pending_swap: bool) -> bool {
    update || ffmpeg || pending_swap
}

/// An app notice (ⓘ, 5 s).
fn info(title: &str, body: &str) -> Notice {
    Notice {
        kind: NoticeKind::Info,
        title: title.to_string(),
        body: body.to_string(),
        action: None,
    }
}

/// The card shown while a shortcut row waits for a key; clicking it cancels.
fn key_prompt(item: SettingItem) -> Notice {
    Notice {
        kind: NoticeKind::KeyPrompt,
        title: "바꿀 키를 누르세요".to_string(),
        body: format!("{} · 다른 곳을 클릭하면 취소", item.label()),
        action: Some(NoticeAction::CancelCapture),
    }
}

/// The outcome of binding a key (3 s).
fn key_result(title: &str, body: &str) -> Notice {
    Notice {
        kind: NoticeKind::KeyResult,
        ..info(title, body)
    }
}

/// An app warning (⚠, 8 s).
fn warning_notice(title: &str, body: &str) -> Notice {
    Notice {
        kind: NoticeKind::Warning,
        ..info(title, body)
    }
}

/// Opens Explorer on `path`'s folder with it selected. A folder deleted
/// meanwhile only gets a log line.
fn open_folder(path: &std::path::Path) {
    if let Err(e) = std::process::Command::new("explorer.exe")
        .arg(format!("/select,{}", path.display()))
        .spawn()
    {
        log_error!("탐색기를 열 수 없습니다 ({}): {e}", path.display());
    }
}

/// Opens a release page in the browser; only https links.
fn open_page(url: &str) {
    if !url.starts_with("https://") {
        log_error!("업데이트: 다운로드 페이지 주소가 https가 아닙니다: {url}");
        return;
    }
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(url),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_waiting_update_keeps_downloads_and_restarts_off() {
        // The swap restarts the app, which would cut a download short.
        assert!(background_busy(false, false, true));
        assert!(background_busy(true, false, false));
        assert!(background_busy(false, true, false));
        assert!(!background_busy(false, false, false));
    }

    #[test]
    fn update_preview_keeps_the_single_instance_rules() {
        // The restart check needs the mutex hand-off and autostart as in use.
        assert!(!previewing(Some("update".into())));
        assert_eq!(previewing(Some("panel".into())), cfg!(debug_assertions));
        assert!(!previewing(None));
    }

    #[test]
    fn autostart_follows_a_readable_config() {
        let on = Config {
            start_with_windows: true,
            ..Config::default()
        };
        assert_eq!(autostart_fix(false, &on, true, false), Some(true));
        assert_eq!(
            autostart_fix(true, &Config::default(), true, false),
            Some(false)
        );
        assert_eq!(autostart_fix(true, &on, true, false), None);
    }

    #[test]
    fn broken_config_or_preview_leaves_autostart_alone() {
        // Defaults stand in for a broken file: they must not unregister.
        assert_eq!(autostart_fix(true, &Config::default(), false, false), None);
        // A debug preview must not point the Run entry at itself.
        assert_eq!(
            autostart_fix(
                false,
                &Config {
                    start_with_windows: true,
                    ..Config::default()
                },
                true,
                true
            ),
            None
        );
    }
}
