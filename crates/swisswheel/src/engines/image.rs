//! Image conversions and tools, done in-process with the `image` crate.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use ::image::codecs::ico::{IcoEncoder, IcoFrame};
use ::image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
use ::image::imageops::{self, FilterType};
use ::image::{
    DynamicImage, ExtendedColorType, ImageDecoder, ImageEncoder, ImageFormat, ImageReader,
    RgbImage, RgbaImage,
};
use swisswheel_core::catalog::{Operation, Tool};
use swisswheel_core::config::Quality;
use swisswheel_core::editor::{Axis, EditOp, Turn};
use swisswheel_core::format::{Format, MediaKind};
use swisswheel_core::options::{Level, Preset};

use super::EngineError;

const ICO_SIZES: [u32; 6] = [256, 128, 64, 48, 32, 16];
const COMPRESS_JPG_QUALITY: u8 = 75;
const COMPRESS_WEBP_QUALITY: u8 = 70;
/// The "높게" and "작게" quality options (spec 2.5); "보통" is the config value.
const HIGH_JPG_QUALITY: u8 = 95;
const HIGH_WEBP_QUALITY: u8 = 92;
const SMALL_JPG_QUALITY: u8 = 75;
const SMALL_WEBP_QUALITY: u8 = 70;

/// Image in, image out. Anything touching video (animated GIF → MP4,
/// MKV → GIF) belongs to FFmpeg.
///
/// `Tool::Edit` is deliberately absent: an editor save carries `Job.edits`,
/// and `jobs` routes it to [`apply_edits`] or the info strip before it ever
/// asks `supports`.
pub fn supports(op: &Operation, input: Format) -> bool {
    input.kind() == MediaKind::Image
        && match op {
            Operation::Convert(target) => target.kind() == MediaKind::Image,
            Operation::Tool(Tool::Shrink) => true,
            Operation::Tool(Tool::CompressImage) => {
                matches!(input, Format::Jpg | Format::Png | Format::Webp)
            }
            _ => false,
        }
}

pub fn output_format(op: &Operation, input: Format) -> Format {
    match op {
        Operation::Convert(target) => *target,
        _ => input,
    }
}

/// Reads `input`, applies `op`, and writes the result to `output` (a temp path,
/// so the format is always explicit, never guessed from its extension).
/// `preset` is the option chosen in the side panel (spec 2.5); `None`, or a
/// preset that doesn't apply to `op`, means the defaults.
pub fn run(
    op: &Operation,
    preset: Option<Preset>,
    input: &Path,
    output: &Path,
    quality: &Quality,
) -> Result<(), EngineError> {
    let input_format = Format::from_path(input)
        .ok_or_else(|| EngineError::Unsupported(input.display().to_string()))?;
    if !supports(op, input_format) {
        return Err(EngineError::Unsupported(format!(
            "{op:?} on {input_format:?}"
        )));
    }
    let img = decode(input)?;
    match op {
        Operation::Convert(target) => {
            let (jpg, webp) = match preset {
                Some(Preset::Quality(Level::High)) => (HIGH_JPG_QUALITY, HIGH_WEBP_QUALITY),
                Some(Preset::Quality(Level::Small)) => (SMALL_JPG_QUALITY, SMALL_WEBP_QUALITY),
                _ => (quality.jpg, quality.webp),
            };
            encode(&img, *target, jpg, webp, output)
        }
        Operation::Tool(Tool::Shrink) => {
            let percent = match preset {
                Some(Preset::Scale(p)) => u32::from(p),
                _ => 50,
            };
            let side = |v: u32| (v * percent / 100).max(1);
            let small =
                img.resize_exact(side(img.width()), side(img.height()), FilterType::Lanczos3);
            encode(&small, input_format, quality.jpg, quality.webp, output)
        }
        Operation::Tool(Tool::CompressImage) => match input_format {
            Format::Png => compress_png(&img, output),
            _ => encode(
                &img,
                input_format,
                COMPRESS_JPG_QUALITY,
                COMPRESS_WEBP_QUALITY,
                output,
            ),
        },
        _ => unreachable!("checked by supports()"),
    }
}

/// Decodes by content (a misnamed file still works) and applies EXIF orientation.
pub(crate) fn decode(input: &Path) -> Result<DynamicImage, EngineError> {
    let read = |e: &dyn std::fmt::Display| EngineError::Read(format!("{}: {e}", input.display()));
    let bytes = std::fs::read(input).map_err(|e| read(&e))?;
    let reader = ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|e| read(&e))?;
    let jpeg_split = reader.format() == Some(ImageFormat::Jpeg) && has_split_scans(&bytes);
    let mut decoder = reader.into_decoder().map_err(|e| read(&e))?;
    let orientation = decoder.orientation().map_err(|e| read(&e))?;
    let mut img = if jpeg_split {
        decode_split_scans(&bytes).map_err(|e| read(&e))?
    } else {
        DynamicImage::from_decoder(decoder).map_err(|e| read(&e))?
    };
    img.apply_orientation(orientation);
    Ok(img)
}

/// Replays the editor's `ops` on the full-resolution image and writes it in
/// the input's own format at high quality.
pub fn apply_edits(ops: &[EditOp], input: &Path, output: &Path) -> Result<(), EngineError> {
    let format = Format::from_path(input)
        .ok_or_else(|| EngineError::Unsupported(input.display().to_string()))?;
    let mut img = decode(input)?;
    let mut gray = false;
    for op in ops {
        match *op {
            EditOp::Crop(r) => {
                let inside = r.w > 0
                    && r.h > 0
                    && r.x.checked_add(r.w).is_some_and(|x| x <= img.width())
                    && r.y.checked_add(r.h).is_some_and(|y| y <= img.height());
                if !inside {
                    return Err(EngineError::Failed(format!(
                        "crop {r:?} outside {}x{}",
                        img.width(),
                        img.height()
                    )));
                }
                img = img.crop_imm(r.x, r.y, r.w, r.h);
            }
            EditOp::Rotate(Turn::Cw) => img = img.rotate90(),
            EditOp::Rotate(Turn::Ccw) => img = img.rotate270(),
            EditOp::Flip(Axis::Horizontal) => img = img.fliph(),
            EditOp::Flip(Axis::Vertical) => img = img.flipv(),
            EditOp::Grayscale(on) => gray = on,
            EditOp::StripInfo(_) => {}
        }
    }
    if gray {
        // Keeps alpha: images with it come back as LumaA.
        img = img.grayscale();
    }
    encode(&img, format, HIGH_JPG_QUALITY, HIGH_WEBP_QUALITY, output)
}

/// Pixel size after EXIF orientation, from the header alone.
pub(crate) fn dimensions(input: &Path) -> Result<(u32, u32), EngineError> {
    let read = |e: &dyn std::fmt::Display| EngineError::Read(format!("{}: {e}", input.display()));
    let reader = ImageReader::open(input)
        .map_err(|e| read(&e))?
        .with_guessed_format()
        .map_err(|e| read(&e))?;
    let mut decoder = reader.into_decoder().map_err(|e| read(&e))?;
    let (w, h) = decoder.dimensions();
    let orientation = decoder.orientation().map_err(|e| read(&e))?;
    use ::image::metadata::Orientation::*;
    Ok(match orientation {
        Rotate90 | Rotate270 | Rotate90FlipH | Rotate270FlipH => (h, w),
        _ => (w, h),
    })
}

/// Whether a sequential JPEG stores its channels in separate scans, as this
/// app's JPGs did before they became progressive. The image crate's decoder
/// reads such files as luma only, so the colors come out green.
fn has_split_scans(jpeg: &[u8]) -> bool {
    let mut frame_channels = None;
    let mut i = 2; // past SOI
    loop {
        let Some(&[0xFF, marker]) = jpeg.get(i..i + 2) else {
            return false;
        };
        if marker == 0xFF {
            i += 1; // fill byte
            continue;
        }
        let Some(&[hi, lo]) = jpeg.get(i + 2..i + 4) else {
            return false;
        };
        let end = i + 2 + usize::from(u16::from_be_bytes([hi, lo]));
        let Some(segment) = jpeg.get(i + 4..end) else {
            return false;
        };
        match marker {
            // Baseline and extended sequential; progressive files decode fine.
            0xC0 | 0xC1 => frame_channels = segment.get(5).copied(),
            0xDA => {
                return frame_channels
                    .zip(segment.first())
                    .is_some_and(|(f, s)| *s < f);
            }
            _ => {}
        }
        i = end;
    }
}

fn decode_split_scans(jpeg: &[u8]) -> Result<DynamicImage, String> {
    let mut decoder = jpeg_decoder::Decoder::new(jpeg);
    let pixels = decoder.decode().map_err(|e| e.to_string())?;
    let info = decoder.info().ok_or("JPG 정보 없음")?;
    if info.pixel_format != jpeg_decoder::PixelFormat::RGB24 {
        return Err(format!("{:?} JPG", info.pixel_format));
    }
    RgbImage::from_raw(info.width.into(), info.height.into(), pixels)
        .map(DynamicImage::ImageRgb8)
        .ok_or_else(|| "JPG 크기 불일치".to_string())
}

fn encode(
    img: &DynamicImage,
    format: Format,
    jpg_quality: u8,
    webp_quality: u8,
    output: &Path,
) -> Result<(), EngineError> {
    let write =
        |e: &dyn std::fmt::Display| EngineError::Write(format!("{}: {e}", output.display()));
    let mut out = BufWriter::new(File::create(output).map_err(|e| write(&e))?);
    match format {
        Format::Jpg => write_jpg(&on_white(img), jpg_quality, &mut out).map_err(|e| write(&e))?,
        Format::Bmp => DynamicImage::ImageRgb8(on_white(img))
            .write_to(&mut out, ImageFormat::Bmp)
            .map_err(|e| write(&e))?,
        Format::Png => PngEncoder::new_with_quality(
            &mut out,
            // The crate's default is its "fast, minimal" level.
            CompressionType::Default,
            PngFilter::Adaptive,
        )
        .write_image(
            img.as_bytes(),
            img.width(),
            img.height(),
            img.color().into(),
        )
        .map_err(|e| write(&e))?,
        Format::Tiff => {
            // The TIFF encoder has no gray-with-alpha mode.
            let img = match img {
                DynamicImage::ImageLumaA8(_) | DynamicImage::ImageLumaA16(_) => {
                    &DynamicImage::ImageRgba8(img.to_rgba8())
                }
                _ => img,
            };
            img.write_to(&mut out, ImageFormat::Tiff)
                .map_err(|e| write(&e))?
        }
        Format::Gif => DynamicImage::ImageRgba8(img.to_rgba8())
            .write_to(&mut out, ImageFormat::Gif)
            .map_err(|e| write(&e))?,
        Format::Webp => {
            let rgba = img.to_rgba8();
            let encoded = webp::Encoder::from_rgba(&rgba, rgba.width(), rgba.height())
                .encode(f32::from(webp_quality));
            out.write_all(&encoded).map_err(|e| write(&e))?;
        }
        Format::Ico => write_ico(img, &mut out).map_err(|e| write(&e))?,
        other => return Err(EngineError::Unsupported(format!("image output {other:?}"))),
    }
    out.flush().map_err(|e| write(&e))
}

/// Optimized Huffman tables and 4:2:0 chroma, as most JPEG writers do; the
/// image crate's own encoder has neither and writes 30-45% larger files.
/// Progressive, because with optimized tables a baseline file gets one scan
/// per channel, which the image crate's decoder reads as green; two scans
/// (DC, then all of AC) cost the least over baseline.
fn write_jpg(img: &RgbImage, quality: u8, out: &mut impl Write) -> Result<(), String> {
    // JPEG itself caps both sides at 65535 pixels.
    let (Ok(w), Ok(h)) = (u16::try_from(img.width()), u16::try_from(img.height())) else {
        return Err(format!(
            "JPG는 가로·세로 65535픽셀까지입니다 ({}x{})",
            img.width(),
            img.height()
        ));
    };
    let mut encoder = jpeg_encoder::Encoder::new(out, quality);
    encoder.set_optimized_huffman_tables(true);
    encoder.set_progressive_scans(2);
    encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::F_2_2);
    encoder
        .encode(img.as_raw(), w, h, jpeg_encoder::ColorType::Rgb)
        .map_err(|e| e.to_string())
}

/// Flattens transparency onto white for formats without alpha.
fn on_white(img: &DynamicImage) -> RgbImage {
    let rgba = img.to_rgba8();
    RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let [r, g, b, a] = rgba.get_pixel(x, y).0;
        let blend =
            |c: u8| ((u16::from(c) * u16::from(a) + 255 * (255 - u16::from(a))) / 255) as u8;
        ::image::Rgb([blend(r), blend(g), blend(b)])
    })
}

/// Multi-size icon; non-square images are fitted and padded with transparency.
fn write_ico(img: &DynamicImage, out: &mut impl Write) -> ::image::ImageResult<()> {
    let mut frames = Vec::with_capacity(ICO_SIZES.len());
    for size in ICO_SIZES {
        let fitted = img.resize(size, size, FilterType::Lanczos3).to_rgba8();
        let mut canvas = RgbaImage::new(size, size);
        let x = (size - fitted.width()) / 2;
        let y = (size - fitted.height()) / 2;
        imageops::overlay(&mut canvas, &fitted, x.into(), y.into());
        frames.push(IcoFrame::as_png(
            canvas.as_raw(),
            size,
            size,
            ExtendedColorType::Rgba8,
        )?);
    }
    IcoEncoder::new(out).encode_images(&frames)
}

fn compress_png(img: &DynamicImage, output: &Path) -> Result<(), EngineError> {
    let mut png = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png, ImageFormat::Png)
        .map_err(|e| EngineError::Failed(e.to_string()))?;
    let optimized = oxipng::optimize_from_memory(png.get_ref(), &oxipng::Options::default())
        .map_err(|e| EngineError::Failed(e.to_string()))?;
    std::fs::write(output, optimized)
        .map_err(|e| EngineError::Write(format!("{}: {e}", output.display())))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ::image::GenericImageView;
    use ::image::codecs::jpeg::JpegEncoder;
    use std::path::Path;
    use swisswheel_core::catalog::{Operation, Tool};
    use swisswheel_core::config::Quality;
    use swisswheel_core::editor::PixelRect;
    use swisswheel_core::format::Format;

    /// Writes a `w`x`h` PNG filled with one color at the given alpha.
    pub(crate) fn rgba_png(path: &Path, w: u32, h: u32, alpha: u8) {
        ::image::RgbaImage::from_pixel(w, h, ::image::Rgba([30, 120, 200, alpha]))
            .save(path)
            .unwrap();
    }

    #[test]
    fn converts_between_all_image_formats() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.png");
        rgba_png(&src, 64, 48, 255);
        for target in [
            Format::Jpg,
            Format::Webp,
            Format::Bmp,
            Format::Tiff,
            Format::Gif,
            Format::Ico,
        ] {
            let out = dir.path().join(format!("out.{}.tmp", target.extension()));
            run(
                &Operation::Convert(target),
                None,
                &src,
                &out,
                &Quality::default(),
            )
            .unwrap();
            let fmt = ::image::ImageReader::open(&out)
                .unwrap()
                .with_guessed_format()
                .unwrap()
                .format();
            assert!(fmt.is_some(), "{target:?} produced unreadable output");
        }
    }

    #[test]
    fn jpg_from_transparent_png_is_white() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("t.png");
        rgba_png(&src, 8, 8, 0); // fully transparent
        let out = dir.path().join("t.jpg.tmp");
        run(
            &Operation::Convert(Format::Jpg),
            None,
            &src,
            &out,
            &Quality::default(),
        )
        .unwrap();
        let px = open(&out).to_rgb8().get_pixel(4, 4).0;
        assert!(px.iter().all(|&c| c > 245), "{px:?}");
    }

    #[test]
    fn ico_from_wide_image_keeps_aspect() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("wide.png");
        rgba_png(&src, 1000, 300, 255);
        let out = dir.path().join("wide.ico.tmp");
        run(
            &Operation::Convert(Format::Ico),
            None,
            &src,
            &out,
            &Quality::default(),
        )
        .unwrap();
        let file = std::io::BufReader::new(std::fs::File::open(&out).unwrap());
        let decoder = ::image::codecs::ico::IcoDecoder::new(file).unwrap();
        let img = ::image::DynamicImage::from_decoder(decoder)
            .unwrap()
            .to_rgba8(); // largest entry
        assert_eq!(img.dimensions(), (256, 256));
        assert_eq!(img.get_pixel(128, 2).0[3], 0); // top padding transparent
        assert_eq!(img.get_pixel(128, 128).0[3], 255); // content opaque
    }

    #[test]
    fn halve_and_compress() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("big.jpg");
        let pixels = ::image::RgbImage::from_fn(301, 200, |x, y| {
            ::image::Rgb([(x % 256) as u8, (y % 256) as u8, 90])
        });
        // Quality 95 so that re-encoding at the compress tool's 75 must shrink it.
        let mut file = std::fs::File::create(&src).unwrap();
        ::image::codecs::jpeg::JpegEncoder::new_with_quality(&mut file, 95)
            .encode_image(&pixels)
            .unwrap();
        drop(file);
        let half = dir.path().join("half.tmp");
        run(
            &Operation::Tool(Tool::Shrink),
            None,
            &src,
            &half,
            &Quality::default(),
        )
        .unwrap();
        assert_eq!(open(&half).dimensions(), (150, 100));
        let small = dir.path().join("small.tmp");
        run(
            &Operation::Tool(Tool::CompressImage),
            None,
            &src,
            &small,
            &Quality::default(),
        )
        .unwrap();
        assert!(std::fs::metadata(&small).unwrap().len() < std::fs::metadata(&src).unwrap().len());
    }

    /// Flat blocks and thin lines, like a screenshot.
    fn screenshot_like(w: u32, h: u32) -> ::image::RgbaImage {
        ::image::RgbaImage::from_fn(w, h, |x, y| {
            if y % 7 == 0 && x % 50 < 35 {
                ::image::Rgba([30, 30, 30, 255])
            } else if (x / 40 + y / 30) % 2 == 0 {
                ::image::Rgba([240, 240, 240, 255])
            } else {
                ::image::Rgba([60, 120, 200, 255])
            }
        })
    }

    fn balanced_png_len(img: &::image::RgbaImage) -> u64 {
        use ::image::ImageEncoder;
        use ::image::codecs::png::{CompressionType, FilterType, PngEncoder};
        let mut v = Vec::new();
        PngEncoder::new_with_quality(&mut v, CompressionType::Default, FilterType::Adaptive)
            .write_image(
                img.as_raw(),
                img.width(),
                img.height(),
                ExtendedColorType::Rgba8,
            )
            .unwrap();
        v.len() as u64
    }

    #[test]
    fn png_output_is_properly_compressed() {
        // The image crate's PNG default is its "fast, minimal" deflate level,
        // which made a halved PNG twice the size of its source.
        let dir = tempfile::tempdir().unwrap();
        let pixels = screenshot_like(640, 400);
        let src = dir.path().join("shot.bmp");
        DynamicImage::ImageRgba8(pixels.clone())
            .to_rgb8()
            .save(&src)
            .unwrap();
        let out = dir.path().join("shot.png.tmp");
        run(
            &Operation::Convert(Format::Png),
            None,
            &src,
            &out,
            &Quality::default(),
        )
        .unwrap();
        let len = std::fs::metadata(&out).unwrap().len();
        let reference = balanced_png_len(&open(&out).to_rgba8());
        assert!(len * 100 <= reference * 105, "{len} B vs {reference} B");

        let png = dir.path().join("shot.png");
        pixels.save(&png).unwrap();
        let half = dir.path().join("half.png.tmp");
        run(
            &Operation::Tool(Tool::Shrink),
            None,
            &png,
            &half,
            &Quality::default(),
        )
        .unwrap();
        let len = std::fs::metadata(&half).unwrap().len();
        let reference = balanced_png_len(&open(&half).to_rgba8());
        assert!(len * 100 <= reference * 105, "{len} B vs {reference} B");
    }

    #[test]
    fn jpg_output_is_optimized() {
        // Optimized Huffman tables and 4:2:0 chroma make JPGs 30-45% smaller
        // than the image crate's encoder at the same quality.
        let dir = tempfile::tempdir().unwrap();
        let mut seed = 12345u32;
        let pixels = ::image::RgbImage::from_fn(480, 320, |x, y| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let noise = (seed >> 24) as u8 / 16;
            ::image::Rgb([
                (x / 2) as u8 + noise,
                (y / 2) as u8 + noise,
                ((x + y) / 4) as u8 + noise,
            ])
        });
        let src = dir.path().join("photo.png");
        pixels.save(&src).unwrap();
        let out = dir.path().join("photo.jpg.tmp");
        run(
            &Operation::Convert(Format::Jpg),
            None,
            &src,
            &out,
            &Quality::default(),
        )
        .unwrap();
        let mut baseline = Vec::new();
        JpegEncoder::new_with_quality(&mut baseline, Quality::default().jpg)
            .encode_image(&pixels)
            .unwrap();
        let len = std::fs::metadata(&out).unwrap().len();
        assert!(
            len * 100 < baseline.len() as u64 * 85,
            "{len} B vs image crate's {} B",
            baseline.len()
        );
        assert_eq!(open(&out).dimensions(), (480, 320));
    }

    /// Mean per-channel difference between two same-sized images.
    fn mean_error(a: &::image::RgbImage, b: &::image::RgbImage) -> f64 {
        let sum: u64 = a
            .as_raw()
            .iter()
            .zip(b.as_raw())
            .map(|(x, y)| u64::from(x.abs_diff(*y)))
            .sum();
        sum as f64 / a.as_raw().len() as f64
    }

    fn gradient(w: u32, h: u32) -> ::image::RgbImage {
        ::image::RgbImage::from_fn(w, h, |x, y| {
            ::image::Rgb([(x * 255 / w) as u8, 200, (y * 255 / h) as u8])
        })
    }

    #[test]
    fn jpg_output_reads_back_in_color() {
        // Optimized tables made the encoder write one scan per channel, which
        // the image crate's decoder reads as luma only: the colors came back
        // green the next time the file was converted or shrunk.
        let dir = tempfile::tempdir().unwrap();
        let pixels = gradient(301, 200);
        let src = dir.path().join("g.png");
        pixels.save(&src).unwrap();
        let out = dir.path().join("g.jpg.tmp");
        run(
            &Operation::Convert(Format::Jpg),
            None,
            &src,
            &out,
            &Quality::default(),
        )
        .unwrap();
        let err = mean_error(&pixels, &open(&out).to_rgb8());
        assert!(err < 5.0, "mean error {err:.1}");
    }

    #[test]
    fn shrinks_jpgs_from_earlier_versions_in_color() {
        // Earlier versions wrote baseline JPGs with one scan per channel.
        let dir = tempfile::tempdir().unwrap();
        let pixels = gradient(300, 200);
        let src = dir.path().join("old.jpg");
        let mut encoder = jpeg_encoder::Encoder::new_file(&src, 90).unwrap();
        encoder.set_optimized_huffman_tables(true);
        encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::F_2_2);
        encoder
            .encode(pixels.as_raw(), 300, 200, jpeg_encoder::ColorType::Rgb)
            .unwrap();
        let out = dir.path().join("old.png.tmp");
        run(
            &Operation::Convert(Format::Png),
            None,
            &src,
            &out,
            &Quality::default(),
        )
        .unwrap();
        let err = mean_error(&pixels, &open(&out).to_rgb8());
        assert!(err < 5.0, "mean error {err:.1}");
    }

    #[test]
    fn shrink_uses_the_chosen_scale() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("wide.png");
        rgba_png(&src, 400, 200, 255);
        for (preset, size) in [
            (Some(Preset::Scale(25)), (100, 50)),
            (Some(Preset::Scale(75)), (300, 150)),
            (None, (200, 100)),
        ] {
            let out = dir.path().join("out.png.tmp");
            run(
                &Operation::Tool(Tool::Shrink),
                preset,
                &src,
                &out,
                &Quality::default(),
            )
            .unwrap();
            assert_eq!(open(&out).dimensions(), size, "{preset:?}");
        }
    }

    #[test]
    fn quality_levels_order_file_sizes() {
        let dir = tempfile::tempdir().unwrap();
        let mut seed = 999u32;
        let pixels = ::image::RgbImage::from_fn(320, 240, |x, y| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let noise = (seed >> 24) as u8 / 8;
            ::image::Rgb([(x / 2) as u8 + noise, (y / 2) as u8 + noise, 80 + noise])
        });
        let src = dir.path().join("photo.png");
        pixels.save(&src).unwrap();
        for target in [Format::Jpg, Format::Webp] {
            let size = |level| {
                let out = dir.path().join(format!("{level:?}.tmp"));
                run(
                    &Operation::Convert(target),
                    Some(Preset::Quality(level)),
                    &src,
                    &out,
                    &Quality::default(),
                )
                .unwrap();
                std::fs::metadata(&out).unwrap().len()
            };
            let (high, normal, small) =
                (size(Level::High), size(Level::Normal), size(Level::Small));
            assert!(
                high > normal && normal > small,
                "{target:?}: {high} / {normal} / {small}"
            );
        }
    }

    #[test]
    fn unreadable_input_is_read_error() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("broken.png");
        std::fs::write(&src, b"not an image").unwrap();
        let err = run(
            &Operation::Convert(Format::Jpg),
            None,
            &src,
            &dir.path().join("o.tmp"),
            &Quality::default(),
        )
        .unwrap_err();
        assert!(matches!(err, EngineError::Read(_)));
        assert_eq!(
            err.user_message("broken.png"),
            "broken.png: 이미지를 읽을 수 없습니다"
        );
    }

    #[test]
    fn supports_only_image_to_image() {
        use Format::*;
        assert!(supports(&Operation::Convert(Jpg), Png));
        assert!(supports(&Operation::Convert(Gif), Png));
        assert!(supports(&Operation::Tool(Tool::Shrink), Bmp));
        assert!(!supports(&Operation::Convert(Gif), Mkv)); // video → GIF is FFmpeg's job
        assert!(!supports(&Operation::Convert(Mp4), Gif)); // animated GIF → video
        assert!(!supports(&Operation::Tool(Tool::Mute), Mp4));
        assert_eq!(output_format(&Operation::Convert(Webp), Png), Webp);
        assert_eq!(
            output_format(&Operation::Tool(Tool::CompressImage), Jpg),
            Jpg
        );
    }

    fn open(path: &Path) -> ::image::DynamicImage {
        ::image::ImageReader::open(path)
            .unwrap()
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap()
    }

    #[test]
    fn edits_rotate_then_crop_moves_marks() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("m.png");
        let mut img = ::image::RgbaImage::from_pixel(40, 20, ::image::Rgba([0, 0, 0, 255]));
        img.put_pixel(0, 0, ::image::Rgba([255, 0, 0, 255])); // top-left red
        img.put_pixel(39, 19, ::image::Rgba([0, 255, 0, 255])); // bottom-right green
        img.save(&src).unwrap();
        let out = dir.path().join("o.png");
        let ops = [
            EditOp::Rotate(Turn::Cw), // 20x40, red at (19, 0)
            EditOp::Crop(PixelRect {
                x: 10,
                y: 0,
                w: 10,
                h: 40,
            }), // red at (9, 0)
            EditOp::Flip(Axis::Horizontal), // red at (0, 0)
        ];
        apply_edits(&ops, &src, &out).unwrap();
        let got = open(&out).to_rgba8();
        assert_eq!(got.dimensions(), (10, 40));
        assert_eq!(got.get_pixel(0, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn edits_ccw_vertical_flip_and_last_grayscale_wins() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("m.png");
        let mut img = ::image::RgbaImage::from_pixel(40, 20, ::image::Rgba([0, 0, 0, 255]));
        img.put_pixel(0, 0, ::image::Rgba([255, 0, 0, 255]));
        img.save(&src).unwrap();
        let out = dir.path().join("o.png");
        let ops = [
            EditOp::Rotate(Turn::Ccw),    // 20x40, red at (0, 39)
            EditOp::Flip(Axis::Vertical), // red at (0, 0)
            EditOp::Grayscale(true),
            EditOp::StripInfo(true),
            EditOp::Grayscale(false),
        ];
        apply_edits(&ops, &src, &out).unwrap();
        let got = open(&out).to_rgba8();
        assert_eq!(got.dimensions(), (20, 40));
        assert_eq!(got.get_pixel(0, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn edits_crop_outside_the_image_fails() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("m.png");
        rgba_png(&src, 8, 8, 255);
        let out = dir.path().join("o.png");
        let crop = EditOp::Crop(PixelRect {
            x: 4,
            y: 0,
            w: 8,
            h: 8,
        });
        assert!(matches!(
            apply_edits(&[crop], &src, &out),
            Err(EngineError::Failed(_))
        ));
    }

    #[test]
    fn edits_grayscale_keeps_alpha_and_jpg_is_high_quality() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.png");
        rgba_png(&src, 8, 8, 128);
        let out = dir.path().join("a_out.png");
        apply_edits(&[EditOp::Grayscale(true)], &src, &out).unwrap();
        let got = open(&out).to_rgba8();
        for p in got.pixels() {
            assert_eq!(p.0[0], p.0[1]);
            assert_eq!(p.0[1], p.0[2]);
            assert_eq!(p.0[3], 128);
        }

        let pixels = gradient(256, 256);
        let jpg = dir.path().join("g.jpg");
        write_jpg(&pixels, 100, &mut File::create(&jpg).unwrap()).unwrap();
        let source = open(&jpg).to_rgb8();
        let out = dir.path().join("g_out.jpg");
        apply_edits(&[EditOp::Rotate(Turn::Cw)], &jpg, &out).unwrap();
        let rotated = ::image::imageops::rotate90(&source);
        let high = mean_error(&rotated, &open(&out).to_rgb8());
        let low_out = dir.path().join("g_low.jpg");
        let mut f = File::create(&low_out).unwrap();
        write_jpg(&rotated, 75, &mut f).unwrap();
        drop(f);
        let low = mean_error(&rotated, &open(&low_out).to_rgb8());
        assert!(high < low, "high {high:.2} vs q75 {low:.2}");
    }

    #[test]
    fn edits_grayscale_works_for_tiff_with_alpha_and_for_jpg() {
        let dir = tempfile::tempdir().unwrap();
        let tiff = dir.path().join("a.tif");
        ::image::RgbaImage::from_pixel(8, 8, ::image::Rgba([30, 120, 200, 128]))
            .save(&tiff)
            .unwrap();
        let out = dir.path().join("a_out.tif");
        apply_edits(&[EditOp::Grayscale(true)], &tiff, &out).unwrap();
        for p in open(&out).to_rgba8().pixels() {
            assert_eq!(p.0[0], p.0[1]);
            assert_eq!(p.0[1], p.0[2]);
        }

        let jpg = dir.path().join("c.jpg");
        write_jpg(&gradient(16, 16), 95, &mut File::create(&jpg).unwrap()).unwrap();
        let out = dir.path().join("c_out.jpg");
        apply_edits(&[EditOp::Grayscale(true)], &jpg, &out).unwrap();
        for p in open(&out).to_rgb8().pixels() {
            assert!(
                p.0[0].abs_diff(p.0[1]) <= 2 && p.0[1].abs_diff(p.0[2]) <= 2,
                "{p:?}"
            );
        }
    }

    #[test]
    fn dimensions_reads_a_plain_png_header() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("d.png");
        rgba_png(&src, 37, 11, 255);
        assert_eq!(dimensions(&src).unwrap(), (37, 11));
    }

    #[test]
    fn dimensions_of_a_broken_image_is_a_read_error() {
        // The editor shows this message when it cannot open a file.
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("broken.jpg");
        std::fs::write(&src, b"not an image").unwrap();
        let err = dimensions(&src).unwrap_err();
        assert!(matches!(err, EngineError::Read(_)));
        assert_eq!(
            err.user_message("broken.jpg"),
            "broken.jpg: 이미지를 읽을 수 없습니다"
        );
    }
}
