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
use swisswheel_core::format::{Format, MediaKind};

use super::EngineError;

const ICO_SIZES: [u32; 6] = [256, 128, 64, 48, 32, 16];
const COMPRESS_JPG_QUALITY: u8 = 75;
const COMPRESS_WEBP_QUALITY: u8 = 70;

/// Image in, image out. Anything touching video (animated GIF → MP4,
/// MKV → GIF) belongs to FFmpeg.
pub fn supports(op: &Operation, input: Format) -> bool {
    input.kind() == MediaKind::Image
        && match op {
            Operation::Convert(target) => target.kind() == MediaKind::Image,
            Operation::Tool(Tool::HalveImage) => true,
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
pub fn run(
    op: &Operation,
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
        Operation::Convert(target) => encode(&img, *target, quality.jpg, quality.webp, output),
        Operation::Tool(Tool::HalveImage) => {
            let half = img.resize_exact(
                (img.width() / 2).max(1),
                (img.height() / 2).max(1),
                FilterType::Lanczos3,
            );
            encode(&half, input_format, quality.jpg, quality.webp, output)
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
fn decode(input: &Path) -> Result<DynamicImage, EngineError> {
    let read = |e: &dyn std::fmt::Display| EngineError::Read(format!("{}: {e}", input.display()));
    let reader = ImageReader::open(input)
        .map_err(|e| read(&e))?
        .with_guessed_format()
        .map_err(|e| read(&e))?;
    let mut decoder = reader.into_decoder().map_err(|e| read(&e))?;
    let orientation = decoder.orientation().map_err(|e| read(&e))?;
    let mut img = DynamicImage::from_decoder(decoder).map_err(|e| read(&e))?;
    img.apply_orientation(orientation);
    Ok(img)
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
        Format::Tiff => img
            .write_to(&mut out, ImageFormat::Tiff)
            .map_err(|e| write(&e))?,
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
            run(&Operation::Convert(target), &src, &out, &Quality::default()).unwrap();
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
            &Operation::Tool(Tool::HalveImage),
            &src,
            &half,
            &Quality::default(),
        )
        .unwrap();
        assert_eq!(open(&half).dimensions(), (150, 100));
        let small = dir.path().join("small.tmp");
        run(
            &Operation::Tool(Tool::CompressImage),
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
            &Operation::Tool(Tool::HalveImage),
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

    #[test]
    fn unreadable_input_is_read_error() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("broken.png");
        std::fs::write(&src, b"not an image").unwrap();
        let err = run(
            &Operation::Convert(Format::Jpg),
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
        assert!(supports(&Operation::Tool(Tool::HalveImage), Bmp));
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
}
