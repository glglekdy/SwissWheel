//! The tray icon, drawn in code: a small red-and-white wheel. No image assets.

use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
    DeleteObject,
};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO};

const RED: [u8; 3] = [0xE0, 0x3A, 0x3E];
const WHITE: [u8; 3] = [0xFF, 0xFF, 0xFF];
const DARK: [u8; 3] = [0x2B, 0x2B, 0x2B];
const SAMPLES: u32 = 4; // per axis, for anti-aliasing

/// Premultiplied BGRA pixels, top-down, `size * size` entries.
pub fn pixels(busy: bool, size: u32) -> Vec<[u8; 4]> {
    let s = size as f32;
    let (c, outer, hub) = (s / 2.0, s / 2.0 - 0.5, s * 0.17);
    let mut out = Vec::with_capacity((size * size) as usize);
    for py in 0..size {
        for px in 0..size {
            let mut acc = [0f32; 4];
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let x = px as f32 + (sx as f32 + 0.5) / SAMPLES as f32 - c;
                    let y = py as f32 + (sy as f32 + 0.5) / SAMPLES as f32 - c;
                    let r = x.hypot(y);
                    let color = if r > outer {
                        continue;
                    } else if r < hub {
                        if busy { DARK } else { WHITE }
                    } else if r < hub + 0.9 || r > outer - 1.0 {
                        RED // rim and hub outline keep the shape readable at 16 px
                    } else {
                        let deg = x.atan2(-y).to_degrees().rem_euclid(360.0);
                        if ((deg / 60.0) as u32).is_multiple_of(2) {
                            RED
                        } else {
                            WHITE
                        }
                    };
                    for (a, v) in acc.iter_mut().zip([color[2], color[1], color[0], 255]) {
                        *a += f32::from(v);
                    }
                }
            }
            let n = (SAMPLES * SAMPLES) as f32;
            let alpha = acc[3] / n / 255.0;
            // acc holds straight colors summed over covered samples only, so dividing
            // by the full sample count yields premultiplied values directly.
            out.push([
                (acc[0] / n) as u8,
                (acc[1] / n) as u8,
                (acc[2] / n) as u8,
                (alpha * 255.0) as u8,
            ]);
        }
    }
    out
}

/// Creates an HICON the caller must free with `DestroyIcon`.
pub fn make(busy: bool, size: i32) -> windows::core::Result<HICON> {
    let size = size.max(16);
    let px = pixels(busy, size as u32);
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

    #[test]
    fn icon_is_round_with_opaque_wheel() {
        let size = 32;
        let px = pixels(false, size);
        let at = |x: u32, y: u32| px[(y * size + x) as usize];
        assert_eq!(at(0, 0)[3], 0); // corner outside the circle
        assert_eq!(at(16, 2)[3], 255); // rim
        assert_eq!(at(16, 16), [255, 255, 255, 255]); // white hub
        assert_eq!(
            pixels(true, size)[(16 * size + 16) as usize][..3],
            [0x2B, 0x2B, 0x2B]
        ); // busy hub
    }
}
