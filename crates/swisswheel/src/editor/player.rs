//! Video playback with sound through Media Foundation's media engine in
//! frame-server mode: the engine decodes and plays the sound itself, and
//! hands each new frame over as a texture on the panel's own D3D11 device.

use std::path::Path;
use std::sync::OnceLock;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, S_OK, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_ALPHA_MODE_IGNORE, D2D1_PIXEL_FORMAT};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_PROPERTIES1, ID2D1Bitmap1, ID2D1DeviceContext,
};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGISurface;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};
use windows::core::{BSTR, Interface, Result, implement};

use crate::wheel::render::create_d3d_device_with;

/// Posted to the panel by the engine's callback; `wParam` is the
/// `MF_MEDIA_ENGINE_EVENT` code.
pub(super) const WM_APP_PLAYER: u32 = WM_APP + 51;

/// Starts Media Foundation once per process; later calls repeat the first result.
pub(super) fn startup() -> Result<()> {
    static STARTED: OnceLock<bool> = OnceLock::new();
    let ok = *STARTED.get_or_init(|| {
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }
            .inspect_err(|e| log_error!("Media Foundation을 시작할 수 없습니다: {e}"))
            .is_ok()
    });
    if ok {
        Ok(())
    } else {
        Err(windows::core::Error::from_hresult(
            windows::Win32::Foundation::E_FAIL,
        ))
    }
}

/// A device both Direct2D and the media engine can use: BGRA for drawing,
/// video support for decoding (dropped when the driver refuses it), and
/// multithread protection, since the engine works on its own threads.
pub(super) fn video_device() -> Result<ID3D11Device> {
    let device = create_d3d_device_with(
        D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
    )
    .or_else(|_| create_d3d_device_with(D3D11_CREATE_DEVICE_BGRA_SUPPORT))?;
    unsafe {
        let _ = device
            .cast::<ID3D11Multithread>()?
            .SetMultithreadProtected(true);
    }
    Ok(device)
}

/// The engine events the panel acts on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum PlayerEvent {
    Ready,
    Ended,
    Seeked,
    Error,
}

impl PlayerEvent {
    pub fn from_wparam(w: usize) -> Option<PlayerEvent> {
        match MF_MEDIA_ENGINE_EVENT(w as i32) {
            MF_MEDIA_ENGINE_EVENT_CANPLAY => Some(PlayerEvent::Ready),
            MF_MEDIA_ENGINE_EVENT_ENDED => Some(PlayerEvent::Ended),
            MF_MEDIA_ENGINE_EVENT_SEEKED => Some(PlayerEvent::Seeked),
            MF_MEDIA_ENGINE_EVENT_ERROR => Some(PlayerEvent::Error),
            _ => None,
        }
    }
}

/// The engine's callback, called on its own threads: it only posts the
/// event to the panel and never touches UI state. A panel gone by then
/// just misses the message.
#[implement(IMFMediaEngineNotify)]
struct Notify {
    panel: isize,
}

impl IMFMediaEngineNotify_Impl for Notify_Impl {
    fn EventNotify(&self, event: u32, _param1: usize, _param2: u32) -> Result<()> {
        if PlayerEvent::from_wparam(event as usize).is_some() {
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(self.panel as *mut _)),
                    WM_APP_PLAYER,
                    WPARAM(event as usize),
                    LPARAM(0),
                );
            }
        }
        Ok(())
    }
}

/// The frame target: a texture the engine copies frames into, and the
/// Direct2D bitmap over it.
struct Target {
    size: (u32, u32),
    texture: ID3D11Texture2D,
    bitmap: ID2D1Bitmap1,
}

pub(super) struct Player {
    engine: IMFMediaEngine,
    device: ID3D11Device,
    /// Kept alive as long as the engine uses it.
    _manager: IMFDXGIDeviceManager,
    target: Option<Target>,
    /// The time stamp of the frame last transferred.
    last_pts: Option<i64>,
}

impl Player {
    /// Loads `path` paused; `WM_APP_PLAYER` with the ready (or error)
    /// event follows once the engine knows the file.
    pub fn open(panel: HWND, device: &ID3D11Device, path: &Path) -> Result<Player> {
        unsafe {
            let mut token = 0;
            let mut manager = None;
            MFCreateDXGIDeviceManager(&mut token, &mut manager)?;
            let manager = manager.ok_or_else(|| windows::core::Error::from(E_FAIL_HR))?;
            manager.ResetDevice(device, token)?;
            let notify: IMFMediaEngineNotify = Notify {
                panel: panel.0 as isize,
            }
            .into();
            let mut attrs = None;
            MFCreateAttributes(&mut attrs, 3)?;
            let attrs = attrs.ok_or_else(|| windows::core::Error::from(E_FAIL_HR))?;
            attrs.SetUnknown(&MF_MEDIA_ENGINE_DXGI_MANAGER, &manager)?;
            attrs.SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &notify)?;
            attrs.SetUINT32(
                &MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT,
                DXGI_FORMAT_B8G8R8A8_UNORM.0 as u32,
            )?;
            let factory: IMFMediaEngineClassFactory =
                CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER)?;
            let engine = factory.CreateInstance(0, &attrs)?;
            let player = Player {
                engine,
                device: device.clone(),
                _manager: manager,
                target: None,
                last_pts: None,
            };
            player.engine.SetAutoPlay(false)?;
            player
                .engine
                .SetSource(&BSTR::from(path.as_os_str().to_string_lossy().as_ref()))?;
            Ok(player)
        }
    }

    pub fn play(&self) {
        unsafe {
            let _ = self.engine.Play();
        }
    }

    pub fn pause(&self) {
        unsafe {
            let _ = self.engine.Pause();
        }
    }

    pub fn seek(&self, t: f64) {
        unsafe {
            let _ = self.engine.SetCurrentTime(t.max(0.0));
        }
    }

    pub fn set_rate(&self, factor: f64) {
        unsafe {
            let _ = self.engine.SetDefaultPlaybackRate(factor);
            let _ = self.engine.SetPlaybackRate(factor);
        }
    }

    pub fn set_muted(&self, muted: bool) {
        unsafe {
            let _ = self.engine.SetMuted(muted);
        }
    }

    pub fn current_time(&self) -> f64 {
        unsafe { self.engine.GetCurrentTime() }
    }

    pub fn ended(&self) -> bool {
        unsafe { self.engine.IsEnded().as_bool() }
    }

    /// One frame forward or back, paused.
    pub fn step_frame(&self, forward: bool) {
        unsafe {
            if let Ok(ex) = self.engine.cast::<IMFMediaEngineEx>() {
                let _ = ex.FrameStep(forward);
            }
        }
    }

    /// The decoded frame size, once the engine knows the video.
    pub fn native_size(&self) -> Option<(u32, u32)> {
        let (mut w, mut h) = (0, 0);
        unsafe {
            self.engine
                .GetNativeVideoSize(Some(&mut w), Some(&mut h))
                .ok()?;
        }
        (w > 0 && h > 0).then_some((w, h))
    }

    /// The newest frame as a bitmap of `ctx` (which must draw on this
    /// player's device), or `None` when no new frame came since the last call.
    pub fn frame(&mut self, ctx: &ID2D1DeviceContext) -> Result<Option<ID2D1Bitmap1>> {
        let Some(size) = self.native_size() else {
            return Ok(None);
        };
        let mut pts = 0i64;
        // S_FALSE (no new frame) is a success to the `windows` wrapper, so ask the vtable.
        let hr = unsafe {
            (Interface::vtable(&self.engine).OnVideoStreamTick)(
                Interface::as_raw(&self.engine),
                &mut pts,
            )
        };
        if hr != S_OK || (self.last_pts == Some(pts) && self.target.is_some()) {
            return Ok(None);
        }
        if self.target.as_ref().is_none_or(|t| t.size != size) {
            self.target = Some(self.make_target(ctx, size)?);
        }
        let Some(target) = &self.target else {
            return Ok(None);
        };
        let dst = RECT {
            left: 0,
            top: 0,
            right: size.0 as i32,
            bottom: size.1 as i32,
        };
        let black = MFARGB {
            rgbBlue: 0,
            rgbGreen: 0,
            rgbRed: 0,
            rgbAlpha: 255,
        };
        unsafe {
            self.engine
                .TransferVideoFrame(&target.texture, None, &dst, Some(&black))?;
        }
        self.last_pts = Some(pts);
        Ok(Some(target.bitmap.clone()))
    }

    fn make_target(&self, ctx: &ID2D1DeviceContext, size: (u32, u32)) -> Result<Target> {
        unsafe {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: size.0,
                Height: size.1,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
                ..Default::default()
            };
            let mut texture = None;
            self.device
                .CreateTexture2D(&desc, None, Some(&mut texture))?;
            let texture = texture.ok_or_else(|| windows::core::Error::from(E_FAIL_HR))?;
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
            let bitmap =
                ctx.CreateBitmapFromDxgiSurface(&texture.cast::<IDXGISurface>()?, Some(&props))?;
            Ok(Target {
                size,
                texture,
                bitmap,
            })
        }
    }

    /// Stops the engine; it plays and calls back no more.
    pub fn shutdown(&self) {
        unsafe {
            let _ = self.engine.Shutdown();
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.shutdown();
    }
}

const E_FAIL_HR: windows::core::HRESULT = windows::Win32::Foundation::E_FAIL;

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::time::{Duration, Instant};
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::core::w;

    thread_local! {
        static SEEN: RefCell<Vec<PlayerEvent>> = const { RefCell::new(Vec::new()) };
    }

    unsafe extern "system" fn test_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        if msg == WM_APP_PLAYER {
            if let Some(event) = PlayerEvent::from_wparam(w.0) {
                SEEN.with(|seen| seen.borrow_mut().push(event));
            }
            return LRESULT(0);
        }
        unsafe { DefWindowProcW(hwnd, msg, w, l) }
    }

    /// A message-only window that records the player's events.
    fn test_window() -> HWND {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let instance = GetModuleHandleW(None).unwrap();
            let class = w!("SwissWheelPlayerTest");
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(test_proc),
                hInstance: instance.into(),
                lpszClassName: class,
                ..Default::default()
            });
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class,
                w!(""),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(instance.into()),
                None,
            )
            .unwrap()
        }
    }

    /// Pumps messages, calling `tick` between rounds like the panel's
    /// playback timer, until `want` arrives (true) or `limit` passes (false).
    fn pump_until(want: PlayerEvent, limit: Duration, tick: &mut dyn FnMut()) -> bool {
        let started = Instant::now();
        while started.elapsed() < limit {
            unsafe {
                let mut msg = MSG::default();
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            let found = SEEN.with(|seen| {
                let mut seen = seen.borrow_mut();
                let found = seen.contains(&want);
                seen.clear();
                found
            });
            if found {
                return true;
            }
            tick();
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    fn d2d_context(device: &ID3D11Device) -> ID2D1DeviceContext {
        use windows::Win32::Graphics::Direct2D::*;
        use windows::Win32::Graphics::Dxgi::IDXGIDevice;
        unsafe {
            let factory: ID2D1Factory1 =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).unwrap();
            factory
                .CreateDevice(&device.cast::<IDXGIDevice>().unwrap())
                .unwrap()
                .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)
                .unwrap()
        }
    }

    #[test]
    fn plays_seeks_and_hands_over_frames() {
        let dir = tempfile::tempdir().unwrap();
        let clip = crate::engines::ffmpeg::tests::tone_clip(dir.path(), "a.mp4", 2);
        let hwnd = test_window();
        startup().unwrap();
        let device = video_device().unwrap();
        let ctx = d2d_context(&device);
        let mut player = Player::open(hwnd, &device, &clip).unwrap();
        assert!(pump_until(
            PlayerEvent::Ready,
            Duration::from_secs(5),
            &mut || {}
        ));
        assert_eq!(player.native_size(), Some((320, 240)));
        // A paused seek only completes once its frame is taken, as the panel's timer does.
        player.seek(1.0);
        let mut frames = Vec::new();
        let seeked = pump_until(PlayerEvent::Seeked, Duration::from_secs(5), &mut || {
            frames.extend(player.frame(&ctx).unwrap());
        });
        assert!(seeked);
        assert!((player.current_time() - 1.0).abs() < 0.2);
        frames.extend(player.frame(&ctx).unwrap());
        let last = frames.last().expect("a frame after the seek");
        let size = unsafe { last.GetPixelSize() };
        assert_eq!((size.width, size.height), (320, 240));
        assert!(
            player.frame(&ctx).unwrap().is_none(),
            "the same frame twice"
        );
        player.shutdown();
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }

    #[test]
    fn a_broken_file_reports_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let broken = dir.path().join("broken.mp4");
        std::fs::write(&broken, b"not a video").unwrap();
        let hwnd = test_window();
        startup().unwrap();
        let device = video_device().unwrap();
        let player = Player::open(hwnd, &device, &broken);
        if let Ok(player) = player {
            assert!(pump_until(
                PlayerEvent::Error,
                Duration::from_secs(5),
                &mut || {}
            ));
            player.shutdown();
        }
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
}
