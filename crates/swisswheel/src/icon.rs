//! The tray icon: the SwissWheel logo (`assets/logo.png`) in the taskbar's
//! color, white on a dark taskbar and black on a light one, optionally
//! ringed by job progress in the accent color.

use std::sync::OnceLock;

use image::imageops::{self, FilterType};
use image::{GrayImage, Luma};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
    DeleteObject,
};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO};

const LOGO_PNG: &[u8] = include_bytes!("../../../assets/logo.png");
/// The unfilled part of the ring: the logo's color, this opaque.
const TRACK_ALPHA: f32 = 0.35;
const SAMPLES: u32 = 4; // per axis, for anti-aliasing

/// The logo's coverage, cropped to its visible part and centered in a square.
fn logo() -> &'static GrayImage {
    static LOGO: OnceLock<GrayImage> = OnceLock::new();
    LOGO.get_or_init(|| {
        let rgba = image::load_from_memory(LOGO_PNG)
            .expect("assets/logo.png decodes")
            .to_rgba8();
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
        for (x, y, p) in rgba.enumerate_pixels() {
            if p[3] > 0 {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
        }
        let (w, h) = (x1 + 1 - x0, y1 + 1 - y0);
        let side = w.max(h);
        let (dx, dy) = ((side - w) / 2, (side - h) / 2);
        GrayImage::from_fn(side, side, |x, y| {
            let inside = x >= dx && y >= dy && x - dx < w && y - dy < h;
            Luma([if inside {
                rgba.get_pixel(x0 + x - dx, y0 + y - dy)[3]
            } else {
                0
            }])
        })
    })
}

/// The logo's coverage at `size` × `size`.
fn coverage(size: u32) -> GrayImage {
    imageops::resize(logo(), size, size, FilterType::Lanczos3)
}

/// The logo's color on a light (`true`) or dark taskbar.
fn logo_color(light: bool) -> [u8; 3] {
    if light { [0, 0, 0] } else { [0xFF, 0xFF, 0xFF] }
}

/// Premultiplied BGRA pixels, top-down, `size * size` entries.
pub fn pixels(light: bool, size: u32) -> Vec<[u8; 4]> {
    let color = logo_color(light);
    coverage(size)
        .pixels()
        .map(|&Luma([a])| premultiply(color, a))
        .collect()
}

/// `0xRRGGBB` → `[r, g, b]`.
pub fn rgb(hex: u32) -> [u8; 3] {
    let [_, r, g, b] = hex.to_be_bytes();
    [r, g, b]
}

/// The logo, shrunk to fit inside a ring (filled part in `ring`, straight
/// RGB) that fills clockwise from 12 o'clock as `fraction` goes from 0 to 1.
pub fn progress_pixels(fraction: f32, light: bool, ring: [u8; 3], size: u32) -> Vec<[u8; 4]> {
    let s = size as f32;
    let color = logo_color(light);
    let outer = s / 2.0 - 0.5;
    let band = (s * 0.16).max(2.0);
    let filled = fraction.clamp(0.0, 1.0) * 360.0;
    let track = [color[0], color[1], color[2], (TRACK_ALPHA * 255.0) as u8];
    let mut out = rasterize(size, |x, y| {
        let r = x.hypot(y);
        if r > outer || r < outer - band {
            return None;
        }
        let deg = x.atan2(-y).to_degrees().rem_euclid(360.0);
        Some(if deg < filled {
            [ring[0], ring[1], ring[2], 255]
        } else {
            track
        })
    });
    // The logo inside the ring, with a gap between them.
    let gap = (s * 0.06).max(1.0);
    let inner = ((s - 2.0 * (band + gap)).floor() as u32).max(1);
    let offset = (size - inner) / 2;
    for (x, y, &Luma([a])) in coverage(inner).enumerate_pixels() {
        if a > 0 {
            let i = ((y + offset) * size + x + offset) as usize;
            out[i] = premultiply(color, a);
        }
    }
    out
}

/// Straight RGB at alpha `a` → premultiplied BGRA.
fn premultiply(rgb: [u8; 3], a: u8) -> [u8; 4] {
    let m = |c: u8| ((u16::from(c) * u16::from(a) + 127) / 255) as u8;
    [m(rgb[2]), m(rgb[1]), m(rgb[0]), a]
}

/// Samples `shape` (offsets from the center → straight RGBA, `None` =
/// transparent) into anti-aliased premultiplied BGRA.
fn rasterize(size: u32, shape: impl Fn(f32, f32) -> Option<[u8; 4]>) -> Vec<[u8; 4]> {
    let c = size as f32 / 2.0;
    let mut out = Vec::with_capacity((size * size) as usize);
    for py in 0..size {
        for px in 0..size {
            let mut acc = [0f32; 4];
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let x = px as f32 + (sx as f32 + 0.5) / SAMPLES as f32 - c;
                    let y = py as f32 + (sy as f32 + 0.5) / SAMPLES as f32 - c;
                    let Some([r, g, b, a]) = shape(x, y) else {
                        continue;
                    };
                    let a = f32::from(a) / 255.0;
                    for (acc, v) in acc.iter_mut().zip([b, g, r]) {
                        *acc += f32::from(v) * a;
                    }
                    acc[3] += a * 255.0;
                }
            }
            let n = (SAMPLES * SAMPLES) as f32;
            out.push(acc.map(|v| (v / n).round() as u8));
        }
    }
    out
}

/// The logo icon; free it with `DestroyIcon`.
pub fn make(light: bool, size: i32) -> windows::core::Result<HICON> {
    let size = size.max(16);
    from_pixels(&pixels(light, size as u32), size)
}

/// The logo with a progress ring in `ring` (straight RGB); free it with
/// `DestroyIcon`.
pub fn make_progress(
    fraction: f32,
    light: bool,
    ring: [u8; 3],
    size: i32,
) -> windows::core::Result<HICON> {
    let size = size.max(16);
    from_pixels(&progress_pixels(fraction, light, ring, size as u32), size)
}

fn from_pixels(px: &[[u8; 4]], size: i32) -> windows::core::Result<HICON> {
    unsafe {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let color = CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0)?;
        std::ptr::copy_nonoverlapping(px.as_ptr().cast::<u8>(), bits.cast::<u8>(), px.len() * 4);
        let mask = CreateBitmap(size, size, 1, 1, None);
        let icon = CreateIconIndirect(&ICONINFO {
            fIcon: true.into(),
            hbmMask: mask,
            hbmColor: color,
            ..Default::default()
        });
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opaque(px: &[[u8; 4]]) -> Vec<[u8; 4]> {
        px.iter().copied().filter(|p| p[3] == 255).collect()
    }

    #[test]
    fn logo_takes_the_taskbar_color() {
        let dark = pixels(false, 32);
        let light = pixels(true, 32);
        assert!(opaque(&dark).len() > 100);
        assert!(opaque(&dark).iter().all(|p| p[..3] == [255, 255, 255]));
        assert!(opaque(&light).iter().all(|p| p[..3] == [0, 0, 0]));
        assert_eq!(opaque(&dark).len(), opaque(&light).len());
    }

    #[test]
    fn corners_and_hub_are_transparent() {
        let size = 32;
        let px = pixels(false, size);
        assert_eq!(px[0][3], 0);
        assert_eq!(px[(16 * size + 16) as usize][3], 0); // the wheel's open center
    }

    #[test]
    fn every_tray_size_shows_the_logo() {
        for size in [16, 20, 24, 32, 40, 48, 64] {
            let px = pixels(false, size);
            assert_eq!(px.len(), (size * size) as usize);
            assert!(px.iter().any(|p| p[3] == 255), "{size}px");
        }
    }

    /// A pixel well inside the ring band of a 32 px icon, `deg` clockwise
    /// from 12 o'clock.
    fn ring_at(px: &[[u8; 4]], deg: f32) -> [u8; 4] {
        let rad = deg.to_radians();
        let x = (16.0 + 14.0 * rad.sin()) as u32;
        let y = (16.0 - 14.0 * rad.cos()) as u32;
        px[(y * 32 + x) as usize]
    }

    const GREEN: [u8; 3] = [0x1E, 0x9E, 0x50];

    #[test]
    fn progress_ring_fills_clockwise_from_the_top() {
        let size = 32;
        let px = progress_pixels(0.25, false, GREEN, size);
        let green = [GREEN[2], GREEN[1], GREEN[0], 255];
        assert_eq!(ring_at(&px, 45.0), green);
        let track = ring_at(&px, 135.0);
        assert_ne!(track, green);
        assert!((40..160).contains(&track[3]), "{track:?}"); // see-through
        assert_eq!(
            ring_at(&progress_pixels(0.0, false, GREEN, size), 45.0),
            track
        );
        assert_eq!(
            ring_at(&progress_pixels(1.0, false, GREEN, size), 300.0),
            green
        );
        assert_eq!(px[(16 * size + 16) as usize][3], 0); // the logo's open center
        assert!(!opaque(&px).iter().all(|p| *p == green)); // the logo is inside
    }

    #[test]
    fn progress_ring_takes_the_accent() {
        for hex in [0x0A84FF, 0xE5484D, 0x5C5C64] {
            let [r, g, b] = rgb(hex);
            let px = progress_pixels(1.0, false, rgb(hex), 32);
            assert_eq!(ring_at(&px, 90.0), [b, g, r, 255], "{hex:06X}");
        }
        assert_eq!(rgb(0x0A84FF), [0x0A, 0x84, 0xFF]);
    }

    #[test]
    fn exe_icon_has_every_size() {
        let ico = include_bytes!("../../../assets/swisswheel.ico");
        assert_eq!(ico[..4], [0, 0, 1, 0]); // reserved, type = icon
        let count = usize::from(u16::from_le_bytes([ico[4], ico[5]]));
        let sizes: Vec<u32> = (0..count)
            .map(|i| match ico[6 + i * 16] {
                0 => 256,
                w => u32::from(w),
            })
            .collect();
        assert_eq!(sizes, [16, 20, 24, 32, 40, 48, 64, 256]);
    }
}
