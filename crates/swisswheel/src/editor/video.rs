//! The video editor panel: a second slot beside the image panel, sharing
//! its windows. `video_control` decides what every key and click does;
//! this module keeps the panel's lifetime, runs the player (or, for files
//! it can't play, the FFmpeg scrubber) and hands the app what it ended with.

use std::cell::RefCell;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use swisswheel_core::appearance::Accent;
use swisswheel_core::crop::CropHandle;
use swisswheel_core::editor::text as editor_text;
use swisswheel_core::editor_geometry::{
    Fit, Timeline, VideoHit, VideoLayout, handle_at, video_hit_test, video_layout,
};
use swisswheel_core::media::scrub_size;
use swisswheel_core::video_control::{
    PlayerAction, VideoCommand, VideoOutcome, VideoState, repeats, video_key,
};
use swisswheel_core::video_edit::{AudioMode, VideoInfo, text};
use windows::Win32::Foundation::{E_FAIL, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_SIZE_U, D2D1_ALPHA_MODE_IGNORE, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_PROPERTIES1, ID2D1Bitmap1, ID2D1DeviceContext,
};
use windows::Win32::Graphics::Direct3D11::ID3D11Device;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::UI::Input::Ime::ImmGetVirtualKey;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, VK_CONTROL, VK_F4, VK_PROCESSKEY, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

use super::player::{Player, PlayerEvent, WM_APP_PLAYER, startup, video_device};
use super::scrub::{ScrubFrame, Scrubber, WM_APP_SCRUB};
use super::window::{self, KEY_REPEAT, Windows, client_point};
use super::{EditorEvent, announce, file_name, video_render};
use crate::engines::ffmpeg::{self, Tools};
use crate::surface::Surface;
use crate::theme::Theme;

/// Panel timers: notices and the discard prompt run out; playback frames.
const NOTICE_TIMER: usize = 1;
const PLAYBACK_TIMER: usize = 2;
const PLAYBACK_MS: u32 = 16;
/// No first frame this long after the player is ready: Windows loaded the
/// file but can't show its picture (it may still play the sound).
const FIRST_FRAME_WAIT: Duration = Duration::from_secs(3);
/// How long frames keep being taken while one is due after a seek: the
/// engine finishes a paused seek only once its frame is taken.
const FRAME_WAIT: Duration = Duration::from_secs(15);

/// When a frame is due from the player.
#[derive(Debug, Default)]
struct FrameWatch {
    /// The first frame must come by then, or the panel falls back.
    first_by: Option<Instant>,
    got_first: bool,
    /// A frame is due since then (a seek, a step, a reopen).
    due_since: Option<Instant>,
}

impl FrameWatch {
    /// The player is ready (again).
    fn ready(&mut self, now: Instant) {
        if !self.got_first && self.first_by.is_none() {
            self.first_by = Some(now + FIRST_FRAME_WAIT);
        }
        self.due_since = Some(now);
    }

    /// A seek or step asked for a new frame.
    fn expect(&mut self, now: Instant) {
        self.due_since = Some(now);
    }

    /// A frame arrived.
    fn frame(&mut self) {
        self.got_first = true;
        self.first_by = None;
        self.due_since = None;
    }

    fn give_up(&self, now: Instant) -> bool {
        self.first_by.is_some_and(|by| now >= by)
    }

    /// Whether frames should still be taken.
    fn pumping(&self, now: Instant) -> bool {
        self.first_by.is_some() || self.due_since.is_some_and(|since| now < since + FRAME_WAIT)
    }
}

struct VideoEditor {
    path: PathBuf,
    name: String,
    owner: HWND,
    notify: u32,
    info: VideoInfo,
    /// The display rotation ffprobe found (counterclockwise degrees).
    rotation: u32,
    tools: Tools,
    state: VideoState,
    layout: VideoLayout,
    scale: f32,
    theme: Theme,
    hover: VideoHit,
    /// The button a press started on, for a click on release.
    press: Option<VideoHit>,
    /// Whether a frame is due from the player.
    watch: FrameWatch,
    /// The player was rebuilt (device lost): go back here once it is ready.
    resume_at: Option<f64>,
    // Declared before what they draw with, so they are released first.
    frame: Option<ID2D1Bitmap1>,
    /// `frame` came from FFmpeg, which turns it upright itself.
    frame_upright: bool,
    scrub_failed: bool,
    player: Option<Player>,
    scrubber: Option<Scrubber>,
    surface: Option<Surface>,
    device: Option<ID3D11Device>,
    windows: Windows,
}

thread_local! {
    static VIDEO: RefCell<Option<VideoEditor>> = const { RefCell::new(None) };
}

/// Opens the panel for `path`: reads it with ffprobe first (an unreadable
/// file opens nothing and queues `Failed`), shows "불러오는 중" while the
/// player loads, and falls back to FFmpeg frames if it can't.
pub(super) fn open(
    path: PathBuf,
    owner: HWND,
    notify: u32,
    accent: Accent,
    tools: Tools,
) -> windows::core::Result<()> {
    let name = file_name(&path);
    let media = match ffmpeg::probe(&tools, &path) {
        Ok(media) => media,
        Err(e) => {
            log_error!("영상 편집기: 읽을 수 없습니다: {e}");
            announce(owner, notify, EditorEvent::Failed(e.user_message(&name)));
            return Ok(());
        }
    };
    let Some(info) = VideoInfo::from_media(&media) else {
        log_error!("영상 편집기: 크기나 길이를 모릅니다: {media:?}");
        let message = format!("{name}: {}", text::UNREADABLE);
        announce(owner, notify, EditorEvent::Failed(message));
        return Ok(());
    };
    let place = window::placement();
    let layout = video_layout(place.work, place.scale, (info.width, info.height));
    let windows = Windows::create(
        owner,
        place.monitor,
        layout.window,
        VIDEO_CLASS,
        Some(video_proc),
    )?;
    let theme = Theme::current(windows.glass_supported, accent, 1.0);
    let mut editor = VideoEditor {
        path: path.clone(),
        name,
        owner,
        notify,
        info: info.clone(),
        rotation: media.rotation,
        tools,
        state: VideoState::loading(),
        layout,
        scale: place.scale,
        theme,
        hover: VideoHit::Nothing,
        press: None,
        watch: FrameWatch::default(),
        resume_at: None,
        frame: None,
        frame_upright: false,
        scrub_failed: false,
        player: None,
        scrubber: None,
        surface: None,
        device: None,
        windows,
    };
    if !editor.redraw() {
        return Err(windows::core::Error::new(E_FAIL, "영상 편집기 렌더러 없음"));
    }
    let (dim, panel) = (editor.windows.dim, editor.windows.panel);
    VIDEO.with(|cell| *cell.borrow_mut() = Some(editor));
    log_info!(
        "영상 편집기 열기: {} ({}×{}, {:.1} s, 회전 {}, 배율 {})",
        path.display(),
        info.width,
        info.height,
        info.duration,
        media.rotation,
        place.scale
    );
    window::show(dim, panel);
    let now = Instant::now();
    with_video(|ed| {
        ed.start_player(now);
        ed.run_actions(now);
    });
    Ok(())
}

const VIDEO_CLASS: PCWSTR = w!("SwissWheelVideoEditor");

pub(super) fn is_open() -> bool {
    VIDEO.with(|cell| cell.try_borrow().map_or(true, |e| e.is_some()))
}

pub(super) fn focus() {
    if let Some(panel) = with_video(|ed| ed.windows.panel) {
        window::raise(panel);
    }
}

/// Destroys the panel without an event.
pub(super) fn close() {
    if let Some(editor) = take_video() {
        log_info!("영상 편집기 닫기 (이벤트 없음)");
        drop(editor);
    }
}

/// A click on the dim window beside the panel: like any click, it clears
/// the discard prompt.
pub(super) fn dim_click(now: Instant) {
    if let Some(outcome) = with_video(|ed| ed.state.click(VideoHit::Nothing, now)) {
        settle(outcome, now);
    }
}

fn with_video<R>(f: impl FnOnce(&mut VideoEditor) -> R) -> Option<R> {
    VIDEO.with(|cell| {
        let mut editor = cell.try_borrow_mut().ok()?;
        editor.as_mut().map(f)
    })
}

fn take_video() -> Option<VideoEditor> {
    VIDEO.with(|cell| cell.try_borrow_mut().ok()?.take())
}

/// Runs what the state queued for the player, then acts on `outcome`.
/// Called outside `with_video`, since closing destroys windows.
fn settle(outcome: VideoOutcome, now: Instant) {
    with_video(|ed| ed.run_actions(now));
    match outcome {
        VideoOutcome::Nothing => {}
        VideoOutcome::Redraw => {
            if with_video(|ed| ed.redraw()) == Some(false) {
                finish(|_| EditorEvent::Failed(editor_text::OPEN_FAILED.to_string()));
            }
        }
        VideoOutcome::Save(plan) => finish(|path| EditorEvent::SavedVideo(path, plan)),
        VideoOutcome::SaveFrame { t, ops } => {
            // The panel stays open; the app starts the job.
            let drawn = with_video(|ed| {
                let event = EditorEvent::SaveFrame(ed.path.clone(), t, ops);
                log_info!("영상 편집기: 장면 저장 {event:?}");
                announce(ed.owner, ed.notify, event);
                ed.redraw()
            });
            if drawn == Some(false) {
                finish(|_| EditorEvent::Failed(editor_text::OPEN_FAILED.to_string()));
            }
        }
        VideoOutcome::Close => finish(|_| EditorEvent::Closed),
    }
}

/// Destroys the panel and queues the event `make` builds from its path.
fn finish(make: impl FnOnce(PathBuf) -> EditorEvent) {
    let Some(editor) = take_video() else {
        return;
    };
    let (owner, notify) = (editor.owner, editor.notify);
    let event = make(editor.path.clone());
    drop(editor);
    log_info!("영상 편집기 닫힘: {event:?}");
    announce(owner, notify, event);
}

/// The counterclockwise turn a frame of `frame` pixels needs to stand like
/// the display size `display`, for a file with display `rotation`: the
/// rotation, when the frame comes sideways (the engine left it as stored);
/// otherwise none. A 180° file is trusted to the engine.
fn extra_rotation(display: (u32, u32), rotation: u32, frame: (u32, u32)) -> u32 {
    if rotation % 180 == 90 && frame == (display.1, display.0) {
        rotation
    } else {
        0
    }
}

/// An FFmpeg frame as a bitmap of `ctx`.
fn scrub_bitmap(
    ctx: &ID2D1DeviceContext,
    frame: &ScrubFrame,
) -> windows::core::Result<ID2D1Bitmap1> {
    let props = D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_IGNORE,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
        ..Default::default()
    };
    unsafe {
        ctx.CreateBitmap(
            D2D_SIZE_U {
                width: frame.size.0,
                height: frame.size.1,
            },
            Some(frame.bgra.as_ptr().cast()),
            frame.size.0 * 4,
            &props,
        )
    }
}

impl VideoEditor {
    /// Starts the player on the panel's device; without one, falls back.
    fn start_player(&mut self, now: Instant) {
        let opened = startup().and_then(|()| match &self.device {
            Some(device) => Player::open(self.windows.panel, device, &self.path),
            None => Err(windows::core::Error::new(E_FAIL, "장치 없음")),
        });
        match opened {
            Ok(player) => self.player = Some(player),
            Err(e) => {
                log_error!("영상 편집기: 재생할 수 없어 장면만 넘겨봅니다: {e}");
                self.fall_back(now);
            }
        }
    }

    /// Frames from FFmpeg from now on; the edits so far stay.
    fn fall_back(&mut self, now: Instant) {
        self.player = None;
        if self.state.session().is_none() {
            self.state.loaded(self.info.clone());
        }
        self.state.set_fallback(now);
        let preview = self.layout.preview;
        let longest = (preview.right - preview.left)
            .max(preview.bottom - preview.top)
            .round()
            .max(2.0) as u32;
        let size = scrub_size((self.info.width, self.info.height), longest);
        let scrubber = Scrubber::new(
            self.tools.clone(),
            self.path.clone(),
            size,
            self.windows.panel,
        );
        scrubber.request(self.state.position());
        self.scrubber = Some(scrubber);
    }

    /// Hands the queued player actions to the player (or, in fallback, the
    /// seeks to the scrubber), then sets the timers to match.
    fn run_actions(&mut self, now: Instant) {
        for action in self.state.take_actions() {
            if let Some(player) = &self.player {
                match action {
                    PlayerAction::Play => player.play(),
                    PlayerAction::Pause => player.pause(),
                    PlayerAction::Seek(t) => {
                        player.seek(t);
                        self.watch.expect(now);
                    }
                    PlayerAction::Rate(factor) => player.set_rate(factor),
                    PlayerAction::Mute(muted) => player.set_muted(muted),
                }
            } else if let (Some(scrubber), PlayerAction::Seek(t)) = (&self.scrubber, action) {
                scrubber.request(t);
            }
        }
        self.arm_timers(now);
    }

    fn arm_timers(&self, now: Instant) {
        let panel = Some(self.windows.panel);
        let pumping = self.watch.pumping(now);
        unsafe {
            if self.player.is_some() && (self.state.playing() || pumping) {
                SetTimer(panel, PLAYBACK_TIMER, PLAYBACK_MS, None);
            } else {
                let _ = KillTimer(panel, PLAYBACK_TIMER);
            }
            match self.state.notice_deadline() {
                Some(deadline) if deadline > now => {
                    let ms = (deadline - now).as_millis() as u32 + 1;
                    SetTimer(panel, NOTICE_TIMER, ms, None);
                }
                _ => {
                    let _ = KillTimer(panel, NOTICE_TIMER);
                }
            }
        }
    }

    /// The playback timer: takes a new frame and follows the player's time;
    /// falls back when the first frame never comes.
    fn on_playback(&mut self, now: Instant) -> VideoOutcome {
        if self.player.is_some() && self.watch.give_up(now) {
            log_error!("영상 편집기: 장면이 오지 않아 장면만 넘겨봅니다");
            self.fall_back(now);
            return VideoOutcome::Redraw;
        }
        let mut redraw = false;
        if let (Some(player), Some(surface)) = (self.player.as_mut(), self.surface.as_ref()) {
            match player.frame(surface.ctx()) {
                Ok(Some(bitmap)) => {
                    self.frame = Some(bitmap);
                    self.frame_upright = false;
                    self.watch.frame();
                    redraw = true;
                }
                Ok(None) => {}
                Err(e) => log_error!("영상 편집기: 장면을 가져올 수 없습니다: {e}"),
            }
            if self.state.playing()
                && self.state.tick(player.current_time(), player.ended()) == VideoOutcome::Redraw
            {
                redraw = true;
            }
        }
        if redraw {
            VideoOutcome::Redraw
        } else {
            VideoOutcome::Nothing
        }
    }

    fn on_player(&mut self, event: PlayerEvent, now: Instant) -> VideoOutcome {
        let Some(player) = &self.player else {
            return VideoOutcome::Nothing;
        };
        match event {
            PlayerEvent::Ready => {
                if self.state.session().is_none() {
                    log_info!("영상 편집기: 재생 준비됨 {:?}", player.native_size());
                    self.state.loaded(self.info.clone());
                }
                if player.native_size().is_none() {
                    // Ready for the sound only: Windows can't decode the picture.
                    log_error!("영상 편집기: 장면 크기를 몰라 장면만 넘겨봅니다");
                    self.fall_back(now);
                    return VideoOutcome::Redraw;
                }
                if let Some(session) = self.state.session() {
                    player.set_rate(session.speed().factor());
                    player.set_muted(session.audio() == AudioMode::Mute);
                }
                if let Some(t) = self.resume_at.take() {
                    player.seek(t);
                }
                self.watch.ready(now);
                VideoOutcome::Redraw
            }
            PlayerEvent::Seeked => VideoOutcome::Redraw,
            PlayerEvent::Ended => {
                let t = player.current_time();
                self.state.tick(t, true)
            }
            PlayerEvent::Error => {
                log_error!("영상 편집기: 재생 오류, 장면만 넘겨봅니다");
                self.fall_back(now);
                VideoOutcome::Redraw
            }
        }
    }

    fn on_scrub(&mut self) -> VideoOutcome {
        let Some(result) = self.scrubber.as_ref().and_then(Scrubber::take) else {
            return VideoOutcome::Nothing;
        };
        match result {
            Ok(frame) => {
                if let Some(surface) = &self.surface {
                    match scrub_bitmap(surface.ctx(), &frame) {
                        Ok(bitmap) => {
                            self.frame = Some(bitmap);
                            self.frame_upright = true;
                            self.scrub_failed = false;
                        }
                        Err(e) => log_error!("영상 편집기: 장면 그림을 만들 수 없습니다: {e}"),
                    }
                }
            }
            Err(_) => self.scrub_failed = true,
        }
        VideoOutcome::Redraw
    }

    /// Draws a frame; after a failure (device lost) rebuilds the device,
    /// the surface and the player at the same spot, paused, and tries once
    /// more. `false` when that fails too.
    fn redraw(&mut self) -> bool {
        let Err(e) = self.draw() else {
            return true;
        };
        log_error!("영상 편집기 그리기 실패, 렌더러를 다시 만듭니다: {e}");
        let now = Instant::now();
        if self.state.playing() {
            self.state.command(VideoCommand::PlayPause, now);
        }
        let had_player = self.player.take().is_some();
        self.frame = None;
        self.surface = None;
        self.device = None;
        if let Err(e) = self.draw() {
            log_error!("영상 편집기 렌더러를 다시 만들 수 없습니다: {e}");
            return false;
        }
        if had_player {
            self.resume_at = Some(self.state.position());
            self.start_player(now);
        } else if let Some(scrubber) = &self.scrubber {
            scrubber.request(self.state.position());
        }
        true
    }

    fn draw(&mut self) -> windows::core::Result<()> {
        if self.surface.is_none() {
            let device = video_device()?;
            let w = self.layout.window;
            let (width, height) = ((w.right - w.left) as u32, (w.bottom - w.top) as u32);
            self.surface = Some(Surface::with_device(
                self.windows.panel,
                device.clone(),
                width,
                height,
                1,
            )?);
            self.device = Some(device);
        }
        let Some(surface) = &self.surface else {
            return Ok(());
        };
        let turn = match &self.frame {
            Some(bitmap) if !self.frame_upright => {
                let px = unsafe { bitmap.GetPixelSize() };
                let display = (self.info.width, self.info.height);
                extra_rotation(display, self.rotation, (px.width, px.height))
            }
            _ => 0,
        };
        let view = video_render::View {
            layout: &self.layout,
            scale: self.scale,
            state: &self.state,
            frame: self.frame.as_ref(),
            turn,
            name: &self.name,
            hover: self.hover,
            scrub_failed: self.scrub_failed,
        };
        video_render::draw(surface, &self.theme, &view, Instant::now())
    }

    fn hit(&self, x: f32, y: f32) -> VideoHit {
        video_hit_test(&self.layout, self.state.crop().is_some(), x, y)
    }

    fn timeline(&self) -> Timeline {
        Timeline::from_bar(self.layout.bar, self.info.duration)
    }

    fn fit(&self) -> Option<Fit> {
        let size = self.state.session()?.size();
        Some(Fit::new(self.layout.preview, size))
    }

    fn handle(&self, x: f32, y: f32) -> Option<CropHandle> {
        let crop = self.state.crop()?;
        handle_at(&self.fit()?, crop.rect, x, y, self.scale)
    }

    fn dragging(&self) -> Option<CropHandle> {
        self.state.crop()?.dragging()
    }

    fn key(&mut self, vk: u16, now: Instant) -> VideoOutcome {
        let down = |key: u16| unsafe { GetKeyState(i32::from(key)) } < 0;
        match video_key(vk, down(VK_SHIFT.0), down(VK_CONTROL.0)) {
            Some(cmd) => self.state.command(cmd, now),
            None => self.state.key_other(now),
        }
    }

    fn press(&mut self, x: f32, y: f32, now: Instant) -> VideoOutcome {
        let hit = self.hit(x, y);
        self.press = None;
        if hit == VideoHit::Bar && self.state.session().is_some() {
            return self.state.timeline_press(self.timeline().time_at(x), now);
        }
        if matches!(hit, VideoHit::Preview | VideoHit::Nothing)
            && let Some(handle) = self.handle(x, y)
            && let Some(fit) = self.fit()
        {
            self.state.crop_press(handle, fit.to_image(x, y));
            return VideoOutcome::Nothing;
        }
        self.press = Some(hit);
        VideoOutcome::Nothing
    }

    fn release(&mut self, x: f32, y: f32, now: Instant) -> VideoOutcome {
        if self.state.scrubbing() {
            return self.state.timeline_release();
        }
        if self.dragging().is_some() {
            self.state.crop_release();
            return VideoOutcome::Nothing;
        }
        let hit = self.hit(x, y);
        match self.press.take() {
            Some(pressed) if pressed == hit => self.state.click(hit, now),
            _ => VideoOutcome::Nothing,
        }
    }

    /// Capture taken away mid-drag (e.g. by a system dialog).
    fn capture_lost(&mut self) -> VideoOutcome {
        self.state.crop_release();
        self.state.timeline_release()
    }

    fn mouse_move(&mut self, x: f32, y: f32) -> VideoOutcome {
        if self.state.scrubbing() {
            return self.state.timeline_move(self.timeline().time_at(x));
        }
        if self.dragging().is_some() {
            let moved = self
                .fit()
                .is_some_and(|fit| self.state.crop_move(fit.to_image(x, y)));
            return if moved {
                VideoOutcome::Redraw
            } else {
                VideoOutcome::Nothing
            };
        }
        self.hover_to(self.hit(x, y))
    }

    fn hover_to(&mut self, hit: VideoHit) -> VideoOutcome {
        if self.hover == hit {
            return VideoOutcome::Nothing;
        }
        self.hover = hit;
        VideoOutcome::Redraw
    }

    fn cursor(&self, x: f32, y: f32) -> PCWSTR {
        window::handle_cursor(self.dragging().or_else(|| self.handle(x, y)))
    }

    /// The notice timer fired: redraw once the message has run out.
    fn notice_tick(&mut self, now: Instant) -> VideoOutcome {
        match self.state.notice_deadline() {
            Some(deadline) if deadline > now => VideoOutcome::Nothing,
            _ => VideoOutcome::Redraw,
        }
    }
}

unsafe extern "system" fn video_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let now = Instant::now();
    let outcome = match msg {
        WM_KEYDOWN => {
            let mut vk = wparam.0 as u16;
            // Should an IME still be attached, ask it for the real key.
            if vk == VK_PROCESSKEY.0 {
                vk = unsafe { ImmGetVirtualKey(hwnd) } as u16;
            }
            // Holding a key repeats only frame stepping.
            if lparam.0 & KEY_REPEAT != 0 && !repeats(vk) {
                return LRESULT(0);
            }
            with_video(|ed| ed.key(vk, now))
        }
        WM_SYSKEYDOWN => {
            if lparam.0 & KEY_REPEAT != 0 {
                return LRESULT(0);
            }
            with_video(|ed| {
                if wparam.0 as u16 == VK_F4.0 {
                    ed.state.command(VideoCommand::Escape, now)
                } else {
                    ed.state.key_other(now)
                }
            })
        }
        WM_SYSCHAR => return LRESULT(0),
        WM_SYSCOMMAND if wparam.0 & 0xFFF0 == SC_KEYMENU as usize => return LRESULT(0),
        WM_CLOSE => with_video(|ed| ed.state.command(VideoCommand::Escape, now)),
        WM_LBUTTONDOWN => {
            let (x, y) = client_point(lparam);
            let outcome = with_video(|ed| ed.press(x, y, now));
            unsafe { SetCapture(hwnd) };
            outcome
        }
        WM_LBUTTONUP => {
            let (x, y) = client_point(lparam);
            let outcome = with_video(|ed| ed.release(x, y, now));
            unsafe {
                let _ = ReleaseCapture();
            }
            outcome
        }
        WM_CAPTURECHANGED => with_video(|ed| ed.capture_lost()),
        WM_MOUSEMOVE => {
            window::track_leave(hwnd);
            let (x, y) = client_point(lparam);
            with_video(|ed| ed.mouse_move(x, y))
        }
        0x02A3 => {
            // WM_MOUSELEAVE
            window::left();
            with_video(|ed| ed.hover_to(VideoHit::Nothing))
        }
        WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT => {
            window::set_cursor(hwnd, |x, y| with_video(|ed| ed.cursor(x, y)));
            return LRESULT(1);
        }
        WM_TIMER if wparam.0 == NOTICE_TIMER => with_video(|ed| ed.notice_tick(now)),
        WM_TIMER if wparam.0 == PLAYBACK_TIMER => with_video(|ed| ed.on_playback(now)),
        WM_APP_PLAYER => PlayerEvent::from_wparam(wparam.0)
            .and_then(|event| with_video(|ed| ed.on_player(event, now))),
        WM_APP_SCRUB => with_video(|ed| ed.on_scrub()),
        // The panel stays where it opened (spec 3.2).
        WM_DPICHANGED => return LRESULT(0),
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    };
    if let Some(outcome) = outcome {
        settle(outcome, now);
    }
    LRESULT(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::video_render::turn_matrix;

    #[test]
    fn no_first_frame_in_time_means_fall_back() {
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        let mut w = FrameWatch::default();
        assert!(!w.pumping(t0) && !w.give_up(t0));
        w.ready(t0);
        assert!(w.pumping(at(100)) && !w.give_up(at(2900)));
        assert!(w.give_up(at(3000)), "a file Windows loads but can't show");
        // Once a frame came, a later slow seek keeps pulling but never gives up.
        let mut w = FrameWatch::default();
        w.ready(t0);
        w.frame();
        assert!(!w.pumping(at(10)));
        w.expect(at(20));
        assert!(w.pumping(at(9000)) && !w.give_up(at(9000)));
        assert!(!w.pumping(at(20 + 15_000)));
        w.frame();
        w.ready(at(30_000)); // a second Ready after a seek
        assert!(!w.give_up(at(40_000)));
    }

    #[test]
    fn engine_frame_needs_rotation_only_when_transposed() {
        // display 1080×1920 (rotation 90): an engine frame of 1920×1080 needs turning, 1080×1920 does not.
        assert_eq!(extra_rotation((1080, 1920), 90, (1920, 1080)), 90);
        assert_eq!(extra_rotation((1080, 1920), 90, (1080, 1920)), 0);
        assert_eq!(extra_rotation((1280, 720), 0, (1280, 720)), 0);
        assert_eq!(extra_rotation((1280, 720), 180, (1280, 720)), 0);
    }

    #[test]
    fn turning_matches_ffmpeg_display_rotation() {
        // A 64×32 clip stored red left, blue right, with rotation 90, is
        // shown by FFmpeg 32×64 with blue on top and red at the bottom.
        let apply = |turn: u32, (x, y): (f32, f32)| {
            let m = turn_matrix(turn, (64, 32));
            (x * m.M11 + y * m.M21 + m.M31, x * m.M12 + y * m.M22 + m.M32)
        };
        let (_, red_y) = apply(90, (8.0, 16.0));
        let (_, blue_y) = apply(90, (56.0, 16.0));
        assert!(red_y > 32.0 && blue_y < 32.0, "red {red_y}, blue {blue_y}");
        // Every turn keeps the picture inside the upright frame.
        for turn in [90, 180, 270] {
            let upright = if turn == 180 {
                (64.0, 32.0)
            } else {
                (32.0, 64.0)
            };
            for corner in [(0.0, 0.0), (64.0, 0.0), (0.0, 32.0), (64.0, 32.0)] {
                let (x, y) = apply(turn, corner);
                assert!((0.0..=upright.0).contains(&x) && (0.0..=upright.1).contains(&y));
            }
        }
        assert_eq!(apply(0, (3.0, 4.0)), (3.0, 4.0));
    }
}
