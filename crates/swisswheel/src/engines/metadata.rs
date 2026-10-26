//! Lossless removal of metadata (EXIF, GPS, XMP, ...) from image files: only the
//! info blocks are cut, every other byte is copied as it is.

use std::path::Path;

use super::EngineError;

/// Writes a copy of `input` (JPEG, PNG, or WebP) without its metadata blocks.
pub fn strip(input: &Path, output: &Path) -> Result<(), EngineError> {
    let bytes =
        std::fs::read(input).map_err(|e| EngineError::Read(format!("{}: {e}", input.display())))?;
    let read_err = |e: String| EngineError::Read(format!("{}: {e}", input.display()));
    let stripped = if bytes.starts_with(&[0xFF, 0xD8]) {
        strip_jpeg(&bytes).map_err(read_err)?
    } else if bytes.starts_with(&PNG_SIGNATURE) || is_webp(&bytes) {
        if is_tilted(&bytes) {
            // Dropping the EXIF would leave it lying down: decode (which
            // applies the orientation) and save again.
            return super::image::apply_edits(&[], input, output);
        }
        if bytes.starts_with(&PNG_SIGNATURE) {
            strip_png(&bytes)
        } else {
            strip_webp(&bytes)
        }
        .map_err(read_err)?
    } else {
        return Err(EngineError::Unsupported(format!(
            "{}: not a JPEG, PNG, or WebP",
            input.display()
        )));
    };
    std::fs::write(output, stripped)
        .map_err(|e| EngineError::Write(format!("{}: {e}", output.display())))
}

/// Copies a JPEG without EXIF/XMP/comment/trailer data. The color profile and
/// every byte from the first SOS to the primary EOI stay as they are. A tilted
/// photo keeps a minimal EXIF holding only its orientation.
fn strip_jpeg(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if !bytes.starts_with(&[0xFF, 0xD8]) {
        return Err("not a JPEG".into());
    }
    let truncated = || "truncated JPEG".to_string();
    let mut kept: Vec<&[u8]> = Vec::new();
    let mut first_is_app0 = false;
    let mut orientation = None;
    let mut i = 2;
    // Segments up to the first SOS.
    let sos_at = loop {
        match bytes.get(i) {
            None => return Err(truncated()),
            Some(0xFF) => {}
            Some(_) => return Err("bad marker".into()),
        }
        let marker = *bytes.get(i + 1).ok_or_else(truncated)?;
        match marker {
            0xFF => {
                i += 1; // fill byte
                continue;
            }
            0xDA => break i,
            0xD9 => return Err("no image data".into()),
            0x01 | 0xD0..=0xD8 => {
                // Standalone markers have no length.
                kept.push(&bytes[i..i + 2]);
                i += 2;
                continue;
            }
            _ => {}
        }
        let len = u16::from_be_bytes([
            *bytes.get(i + 2).ok_or_else(truncated)?,
            *bytes.get(i + 3).ok_or_else(truncated)?,
        ]) as usize;
        if len < 2 {
            return Err("bad segment length".into());
        }
        let end = i + 2 + len;
        if end > bytes.len() {
            return Err(truncated());
        }
        let payload = &bytes[i + 4..end];
        let keep = match marker {
            0xE1 => {
                if orientation.is_none()
                    && let Some(tiff) = payload.strip_prefix(b"Exif\0\0")
                {
                    orientation = exif_orientation(tiff);
                }
                false
            }
            0xE2 => payload.starts_with(b"ICC_PROFILE\0"),
            0xE3..=0xED | 0xEF | 0xFE => false,
            _ => true, // APP0, APP14 and the segments the image needs
        };
        if keep {
            if kept.is_empty() {
                first_is_app0 = marker == 0xE0;
            }
            kept.push(&bytes[i..end]);
        }
        i = end;
    };

    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(&[0xFF, 0xD8]);
    let tilted = orientation.filter(|o| (2..=8).contains(o));
    let mut pending = tilted.map(orientation_app1);
    for (n, seg) in kept.iter().enumerate() {
        if n == 1 || (n == 0 && !first_is_app0) {
            // Right after APP0, or right after SOI when there is none.
            if let Some(app1) = pending.take() {
                out.extend_from_slice(&app1);
            }
        }
        out.extend_from_slice(seg);
    }
    if let Some(app1) = pending.take() {
        out.extend_from_slice(&app1);
    }

    // From the first SOS: entropy-coded data, with length-prefixed segments
    // (further SOS, DHT, ...) between scans, up to the primary EOI.
    let mut j = sos_at;
    let mut at_segment = true;
    loop {
        if at_segment {
            let len = u16::from_be_bytes([
                *bytes.get(j + 2).ok_or_else(truncated)?,
                *bytes.get(j + 3).ok_or_else(truncated)?,
            ]) as usize;
            let end = j + 2 + len;
            if len < 2 || end > bytes.len() {
                return Err(truncated());
            }
            out.extend_from_slice(&bytes[j..end]);
            j = end;
            at_segment = false;
            continue;
        }
        let &b = bytes.get(j).ok_or_else(truncated)?;
        if b != 0xFF {
            out.push(b);
            j += 1;
            continue;
        }
        match bytes.get(j + 1).ok_or_else(truncated)? {
            0x00 | 0xD0..=0xD7 => {
                out.extend_from_slice(&bytes[j..j + 2]);
                j += 2;
            }
            0xFF => {
                out.push(0xFF); // fill byte before a marker
                j += 1;
            }
            0xD9 => {
                out.extend_from_slice(&[0xFF, 0xD9]);
                return Ok(out);
            }
            _ => at_segment = true,
        }
    }
}

fn is_webp(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP"
}

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// Copies a PNG without text, EXIF, and timestamp chunks; every other chunk
/// (including `iCCP` and all `IDAT`) is copied byte for byte. Anything after
/// `IEND` is dropped.
fn strip_png(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if !bytes.starts_with(&PNG_SIGNATURE) {
        return Err("not a PNG".into());
    }
    let truncated = || "truncated PNG".to_string();
    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(&PNG_SIGNATURE);
    let mut i = PNG_SIGNATURE.len();
    loop {
        let head = bytes.get(i..i + 8).ok_or_else(truncated)?;
        let len = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
        let ty = &head[4..8];
        let end = i
            .checked_add(12)
            .and_then(|n| n.checked_add(len))
            .filter(|&end| end <= bytes.len())
            .ok_or_else(truncated)?;
        if !matches!(ty, b"tEXt" | b"zTXt" | b"iTXt" | b"eXIf" | b"tIME") {
            out.extend_from_slice(&bytes[i..end]);
        }
        i = end;
        if ty == b"IEND" {
            return Ok(out);
        }
    }
}

/// Copies a WebP without its EXIF and XMP chunks, clearing their VP8X flags
/// and fixing the RIFF size. A simple (non-VP8X) file has no such chunks and
/// is copied unchanged.
fn strip_webp(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return Err("not a WebP".into());
    }
    let truncated = || "truncated WebP".to_string();
    let riff = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    let limit = riff.checked_add(8).ok_or_else(truncated)?;
    if limit > bytes.len() || limit < 12 {
        return Err(truncated());
    }
    if &bytes[12..16.min(limit)] != b"VP8X" {
        return Ok(bytes.to_vec());
    }
    let mut out = bytes[..12].to_vec();
    let mut i = 12;
    while i < limit {
        let head = bytes.get(i..i + 8).filter(|_| i + 8 <= limit);
        let head = head.ok_or_else(truncated)?;
        let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
        let data_end = (i + 8).checked_add(len).filter(|&e| e <= limit);
        let data_end = data_end.ok_or_else(truncated)?;
        // The pad byte may be missing only at the very end of the file.
        let end = (data_end + len % 2).min(limit);
        match &head[..4] {
            b"EXIF" | b"XMP " => {}
            b"VP8X" => {
                if len < 10 {
                    return Err("bad VP8X chunk".into());
                }
                let start = out.len();
                out.extend_from_slice(&bytes[i..end]);
                out[start + 8] &= !0x0C;
            }
            _ => out.extend_from_slice(&bytes[i..end]),
        }
        i = end;
    }
    let size = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&size.to_le_bytes());
    Ok(out)
}

/// Whether the image crate's decoder reports an EXIF orientation that has to
/// be applied. A decoder error counts as upright: the byte stripper checks the
/// structure itself.
fn is_tilted(bytes: &[u8]) -> bool {
    use ::image::ImageDecoder;
    ::image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()
        .and_then(|r| r.into_decoder().ok())
        .and_then(|mut d| d.orientation().ok())
        .is_some_and(|o| o != ::image::metadata::Orientation::NoTransforms)
}

/// The orientation (tag 0x0112) in IFD0 of a TIFF block, either byte order.
fn exif_orientation(tiff: &[u8]) -> Option<u16> {
    let big = match tiff.get(..4)? {
        b"II*\0" => false,
        b"MM\0*" => true,
        _ => return None,
    };
    let u16_at = |at: usize| -> Option<u16> {
        let b: [u8; 2] = tiff.get(at..at + 2)?.try_into().ok()?;
        Some(if big {
            u16::from_be_bytes(b)
        } else {
            u16::from_le_bytes(b)
        })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let b: [u8; 4] = tiff.get(at..at + 4)?.try_into().ok()?;
        Some(if big {
            u32::from_be_bytes(b)
        } else {
            u32::from_le_bytes(b)
        })
    };
    let ifd = u32_at(4)? as usize;
    let count = u16_at(ifd)? as usize;
    (0..count).find_map(|n| {
        let entry = ifd + 2 + n * 12;
        if u16_at(entry)? == 0x0112 && u16_at(entry + 2)? == 3 && u32_at(entry + 4)? == 1 {
            u16_at(entry + 8)
        } else {
            None
        }
    })
}

/// A complete APP1 segment holding only the orientation tag.
fn orientation_app1(orientation: u16) -> Vec<u8> {
    let mut out = vec![0xFF, 0xE1, 0x00, 34];
    out.extend_from_slice(b"Exif\0\0II*\0");
    out.extend_from_slice(&8u32.to_le_bytes()); // IFD0 offset
    out.extend_from_slice(&1u16.to_le_bytes()); // one entry
    out.extend_from_slice(&0x0112u16.to_le_bytes());
    out.extend_from_slice(&3u16.to_le_bytes()); // SHORT
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&orientation.to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&0u32.to_le_bytes()); // no next IFD
    out
}

#[cfg(test)]
pub(crate) fn exif_app1(orientation: u16, big_endian: bool, gps: bool) -> Vec<u8> {
    let u16b = |v: u16| {
        if big_endian {
            v.to_be_bytes()
        } else {
            v.to_le_bytes()
        }
    };
    let u32b = |v: u32| {
        if big_endian {
            v.to_be_bytes()
        } else {
            v.to_le_bytes()
        }
    };
    let mut out = b"Exif\0\0".to_vec();
    out.extend_from_slice(if big_endian { b"MM\0*" } else { b"II*\0" });
    out.extend_from_slice(&u32b(8));
    let entries: u16 = if gps { 2 } else { 1 };
    out.extend_from_slice(&u16b(entries));
    out.extend_from_slice(&u16b(0x0112));
    out.extend_from_slice(&u16b(3));
    out.extend_from_slice(&u32b(1));
    out.extend_from_slice(&u16b(orientation));
    out.extend_from_slice(&[0, 0]);
    if gps {
        // IFD0 is 2 + 2 * 12 + 4 bytes long, so the GPS IFD follows at 8 + 30.
        out.extend_from_slice(&u16b(0x8825));
        out.extend_from_slice(&u16b(4));
        out.extend_from_slice(&u32b(1));
        out.extend_from_slice(&u32b(38));
    }
    out.extend_from_slice(&u32b(0));
    if gps {
        out.extend_from_slice(&u16b(1));
        out.extend_from_slice(&u16b(1)); // GPSLatitudeRef
        out.extend_from_slice(&u16b(2)); // ASCII
        out.extend_from_slice(&u32b(2));
        out.extend_from_slice(b"N\0\0\0");
        out.extend_from_slice(&u32b(0));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::image::ImageDecoder;
    use ::image::metadata::Orientation;

    struct Opts {
        w: u16,
        h: u16,
        noisy: bool,
        restart: bool,
        progressive: bool,
    }

    fn jpeg_opts(app: &[(u8, Vec<u8>)], o: Opts) -> Vec<u8> {
        let (w, h) = (o.w as usize, o.h as usize);
        let mut seed = 12345u32;
        let mut rgb = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            for x in 0..w {
                if o.noisy {
                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    rgb.extend_from_slice(&[
                        (seed >> 24) as u8,
                        (seed >> 16) as u8,
                        (seed >> 8) as u8,
                    ]);
                } else {
                    rgb.extend_from_slice(&[(x * 7) as u8, (y * 11) as u8, (x + y) as u8]);
                }
            }
        }
        let mut buf = Vec::new();
        let mut enc = jpeg_encoder::Encoder::new(&mut buf, 90);
        if o.restart {
            enc.set_restart_interval(1);
        }
        if o.progressive {
            enc.set_progressive_scans(2);
        }
        for (nr, data) in app {
            enc.add_app_segment(*nr, data).unwrap();
        }
        enc.encode(&rgb, o.w, o.h, jpeg_encoder::ColorType::Rgb)
            .unwrap();
        buf
    }

    fn jpeg_with(app: &[(u8, Vec<u8>)]) -> Vec<u8> {
        jpeg_opts(
            app,
            Opts {
                w: 32,
                h: 16,
                noisy: false,
                restart: false,
                progressive: false,
            },
        )
    }

    /// From the first SOS marker to the end.
    fn scan(bytes: &[u8]) -> &[u8] {
        let at = bytes.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
        &bytes[at..]
    }

    /// Everything before the first SOS marker.
    fn head(bytes: &[u8]) -> &[u8] {
        let at = bytes.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
        &bytes[..at]
    }

    fn orientation_of(jpeg: &[u8]) -> Orientation {
        let reader = ::image::ImageReader::new(std::io::Cursor::new(jpeg))
            .with_guessed_format()
            .unwrap();
        reader.into_decoder().unwrap().orientation().unwrap()
    }

    #[test]
    fn jpeg_exif_and_xmp_go_icc_and_scan_stay() {
        let icc = [b"ICC_PROFILE\0\x01\x01".as_slice(), &[7u8; 40]].concat();
        let xmp = [b"http://ns.adobe.com/xap/1.0/\0".as_slice(), b"<x/>"].concat();
        let src = jpeg_with(&[(1, exif_app1(1, false, true)), (1, xmp), (2, icc.clone())]);
        let mut with_tail = src.clone();
        with_tail.extend_from_slice(b"TRAILER");
        let out = strip_jpeg(&with_tail).unwrap();
        assert!(!out.windows(6).any(|w| w == b"Exif\0\0"));
        assert!(!out.windows(4).any(|w| w == b"xap/"));
        assert!(out.windows(icc.len()).any(|w| w == icc.as_slice()));
        assert_eq!(scan(&out), scan(&src));
        assert!(out.ends_with(&[0xFF, 0xD9]));
    }

    #[test]
    fn rotated_photo_keeps_only_orientation() {
        let src = jpeg_with(&[(1, exif_app1(6, false, true))]);
        let out = strip_jpeg(&src).unwrap();
        let h = head(&out);
        let at = h.windows(6).position(|w| w == b"Exif\0\0").unwrap();
        assert_eq!(&h[at - 4..at - 2], [0xFF, 0xE1]);
        assert_eq!(&h[at - 2..at], [0, 34]);
        assert_eq!(exif_orientation(&h[at + 6..]), Some(6));
        assert_eq!(h.windows(6).filter(|w| *w == b"Exif\0\0").count(), 1);
        assert!(!h.windows(2).any(|w| w == [0x88, 0x25] || w == [0x25, 0x88]));
        assert_eq!(scan(&out), scan(&src));
        assert_eq!(orientation_of(&out), Orientation::Rotate90);
    }

    #[test]
    fn big_endian_orientation_is_kept() {
        let out = strip_jpeg(&jpeg_with(&[(1, exif_app1(8, true, false))])).unwrap();
        assert_eq!(orientation_of(&out), Orientation::Rotate270);
    }

    /// A TIFF block with one orientation entry whose count field, IFD offset,
    /// and length can be broken.
    fn tiff_entry(big: bool, ifd: u32, count: u32, keep: usize) -> Vec<u8> {
        let u16b = |v: u16| {
            if big {
                v.to_be_bytes()
            } else {
                v.to_le_bytes()
            }
        };
        let u32b = |v: u32| {
            if big {
                v.to_be_bytes()
            } else {
                v.to_le_bytes()
            }
        };
        let mut t = Vec::new();
        t.extend_from_slice(if big { b"MM\0*" } else { b"II*\0" });
        t.extend_from_slice(&u32b(ifd));
        t.extend_from_slice(&u16b(1));
        t.extend_from_slice(&u16b(0x0112));
        t.extend_from_slice(&u16b(3));
        t.extend_from_slice(&u32b(count));
        t.extend_from_slice(&u16b(6));
        t.extend_from_slice(&[0, 0]);
        t.truncate(keep);
        t
    }

    #[test]
    fn malformed_exif_orientation_is_none() {
        for big in [false, true] {
            let full = tiff_entry(big, 8, 1, usize::MAX);
            assert_eq!(exif_orientation(&full), Some(6));
            assert_eq!(exif_orientation(&tiff_entry(big, 8, 2, usize::MAX)), None);
            assert_eq!(exif_orientation(&tiff_entry(big, 8, 0, usize::MAX)), None);
            assert_eq!(
                exif_orientation(&tiff_entry(big, 1000, 1, usize::MAX)),
                None
            );
            // Cut inside the entry, before its value.
            for keep in [9, 12, 16, 18, 19] {
                assert_eq!(exif_orientation(&tiff_entry(big, 8, 1, keep)), None);
            }
        }
    }

    #[test]
    fn orientation_app1_sits_right_after_app0() {
        let src = jpeg_with(&[(1, exif_app1(6, false, false))]);
        let out = strip_jpeg(&src).unwrap();
        // SOI, APP0 (JFIF), then the inserted APP1.
        assert_eq!(&out[2..4], [0xFF, 0xE0]);
        let app0_len = u16::from_be_bytes([out[4], out[5]]) as usize;
        let at = 4 + app0_len;
        assert_eq!(&out[at..at + 2], [0xFF, 0xE1]);
        assert_eq!(&out[at + 4..at + 10], b"Exif\0\0");
    }

    #[test]
    fn orientation_app1_follows_soi_without_app0() {
        let src = jpeg_with(&[(1, exif_app1(6, false, false))]);
        let app0_len = u16::from_be_bytes([src[4], src[5]]) as usize;
        assert_eq!(&src[2..4], [0xFF, 0xE0]);
        let mut no_app0 = src[..2].to_vec();
        no_app0.extend_from_slice(&src[4 + app0_len..]);
        let out = strip_jpeg(&no_app0).unwrap();
        assert_eq!(&out[2..4], [0xFF, 0xE1]);
        assert_eq!(&out[6..12], b"Exif\0\0");
        assert_eq!(orientation_of(&out), Orientation::Rotate90);
    }

    #[test]
    fn upright_photo_gets_no_exif() {
        let out = strip_jpeg(&jpeg_with(&[(1, exif_app1(1, false, true))])).unwrap();
        assert!(!out.windows(6).any(|w| w == b"Exif\0\0"));
    }

    #[test]
    fn scan_with_ff_bytes_is_copied_verbatim() {
        let src = jpeg_opts(
            &[(1, exif_app1(1, false, false))],
            Opts {
                w: 64,
                h: 64,
                noisy: true,
                restart: true,
                progressive: false,
            },
        );
        let out = strip_jpeg(&src).unwrap();
        let s = scan(&out);
        assert!(s.windows(2).any(|w| w == [0xFF, 0x00]));
        assert!(
            s.windows(2)
                .any(|w| w[0] == 0xFF && (0xD0..=0xD7).contains(&w[1]))
        );
        assert_eq!(s, scan(&src));
    }

    #[test]
    fn progressive_scans_are_all_kept() {
        let src = jpeg_opts(
            &[(1, exif_app1(1, false, true))],
            Opts {
                w: 64,
                h: 64,
                noisy: true,
                restart: false,
                progressive: true,
            },
        );
        let sos = src.windows(2).filter(|w| *w == [0xFF, 0xDA]).count();
        assert!(sos >= 2, "expected a multi-scan file");
        let out = strip_jpeg(&src).unwrap();
        assert_eq!(scan(&out), scan(&src));
        assert!(out.ends_with(&[0xFF, 0xD9]));
    }

    #[test]
    fn data_after_eoi_is_dropped_even_if_it_is_a_jpeg() {
        let src = jpeg_with(&[(1, exif_app1(1, false, false))]);
        let second = jpeg_with(&[(1, exif_app1(3, false, true))]);
        let mut joined = src.clone();
        joined.extend_from_slice(&second);
        let out = strip_jpeg(&joined).unwrap();
        assert!(out.ends_with(&[0xFF, 0xD9]));
        assert_eq!(scan(&out), scan(&src));
        assert_eq!(out.windows(2).filter(|w| *w == [0xFF, 0xD8]).count(), 1);
        assert!(!out.windows(6).any(|w| w == b"Exif\0\0"));
    }

    #[test]
    fn dimensions_follow_orientation() {
        let src = jpeg_opts(
            &[(1, exif_app1(6, false, false))],
            Opts {
                w: 40,
                h: 20,
                noisy: false,
                restart: false,
                progressive: false,
            },
        );
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("r.jpg");
        std::fs::write(&p, src).unwrap();
        assert_eq!(crate::engines::image::dimensions(&p).unwrap(), (20, 40));
    }

    #[test]
    fn garbage_is_read_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.jpg");
        std::fs::write(&p, [0xFF, 0xD8, 0xFF, 0xE1, 0x00]).unwrap();
        assert!(matches!(
            strip(&p, &dir.path().join("o.jpg")),
            Err(EngineError::Read(_))
        ));
    }

    fn png_chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(ty);
        out.extend_from_slice(data);
        let mut crc = crc32fast::Hasher::new();
        crc.update(ty);
        crc.update(data);
        out.extend_from_slice(&crc.finalize().to_be_bytes());
        out
    }

    /// (type, data) of every chunk after the signature.
    fn png_chunks(bytes: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
        let mut out = Vec::new();
        let mut i = 8;
        while i < bytes.len() {
            let len = u32::from_be_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
            let ty: [u8; 4] = bytes[i + 4..i + 8].try_into().unwrap();
            out.push((ty, bytes[i + 8..i + 8 + len].to_vec()));
            i += 12 + len;
        }
        out
    }

    fn png_with(w: u32, h: u32, exif_orientation: u16) -> Vec<u8> {
        use ::image::ImageEncoder;
        let rgba: Vec<u8> = (0..w * h * 4).map(|n| (n * 13) as u8).collect();
        let mut plain = Vec::new();
        ::image::codecs::png::PngEncoder::new(&mut plain)
            .write_image(&rgba, w, h, ::image::ExtendedColorType::Rgba8)
            .unwrap();
        let exif = exif_app1(exif_orientation, false, true);
        // After IHDR: tIME, iCCP-like, tEXt, eXIf; then IDAT and the rest.
        let ihdr_end = 8 + 12 + 13;
        let mut out = plain[..ihdr_end].to_vec();
        out.extend_from_slice(&png_chunk(b"tIME", &[7, 234, 10, 25, 1, 2, 3]));
        out.extend_from_slice(&png_chunk(b"iCCP", b"icc\0\0profile-bytes"));
        out.extend_from_slice(&png_chunk(b"tEXt", b"Comment\0hi"));
        out.extend_from_slice(&png_chunk(b"eXIf", &exif[6..]));
        out.extend_from_slice(&plain[ihdr_end..]);
        out
    }

    #[test]
    fn png_text_and_exif_go_idat_stays() {
        let src = png_with(8, 8, 1);
        let out = strip_png(&src).unwrap();
        let (before, after) = (png_chunks(&src), png_chunks(&out));
        let types: Vec<_> = after.iter().map(|c| c.0).collect();
        for gone in [b"tEXt", b"zTXt", b"iTXt", b"eXIf", b"tIME"] {
            assert!(!types.contains(gone));
        }
        assert!(types.contains(b"iCCP"));
        let idat = |c: &[([u8; 4], Vec<u8>)]| -> Vec<Vec<u8>> {
            c.iter()
                .filter(|c| &c.0 == b"IDAT")
                .map(|c| c.1.clone())
                .collect()
        };
        assert_eq!(idat(&before), idat(&after));
        assert_eq!(types.last(), Some(b"IEND"));
        let a = ::image::load_from_memory(&src).unwrap();
        let b = ::image::load_from_memory(&out).unwrap();
        assert_eq!(a.as_bytes(), b.as_bytes());
    }

    fn webp_chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = ty.to_vec();
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
        if data.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    fn simple_webp() -> Vec<u8> {
        let rgba: Vec<u8> = (0..16 * 16 * 4).map(|n| (n * 5) as u8).collect();
        webp::Encoder::from_rgba(&rgba, 16, 16)
            .encode(80.0)
            .to_vec()
    }

    #[test]
    fn webp_exif_goes_and_header_is_fixed() {
        let simple = simple_webp();
        let mut vp8x = vec![0x08 | 0x04, 0, 0, 0];
        vp8x.extend_from_slice(&[15, 0, 0, 15, 0, 0]); // canvas 16x16, minus one
        let mut body = b"WEBP".to_vec();
        body.extend_from_slice(&webp_chunk(b"VP8X", &vp8x));
        body.extend_from_slice(&simple[12..]); // the VP8 chunk
        body.extend_from_slice(&webp_chunk(b"EXIF", &[1, 2, 3, 4, 5]));
        body.extend_from_slice(&webp_chunk(b"XMP ", b"<x/>!!!"));
        let mut src = b"RIFF".to_vec();
        src.extend_from_slice(&(body.len() as u32).to_le_bytes());
        src.extend_from_slice(&body);

        let out = strip_webp(&src).unwrap();
        assert!(!out.windows(4).any(|w| w == b"EXIF" || w == b"XMP "));
        assert_eq!(&out[12..16], b"VP8X");
        assert_eq!(out[20] & 0x0C, 0);
        assert_eq!(
            u32::from_le_bytes(out[4..8].try_into().unwrap()) as usize,
            out.len() - 8
        );
        assert_eq!(out.len() % 2, 0);
        assert_eq!(out.len(), 12 + 18 + simple[12..].len());
        let a = ::image::load_from_memory(&src).unwrap();
        let b = ::image::load_from_memory(&out).unwrap();
        assert_eq!((b.width(), b.height()), (16, 16));
        assert_eq!(a.as_bytes(), b.as_bytes());
    }

    #[test]
    fn simple_webp_is_copied_unchanged() {
        let simple = simple_webp();
        assert_eq!(strip_webp(&simple).unwrap(), simple);
    }

    #[test]
    fn broken_png_and_webp_are_read_errors() {
        let png = png_with(8, 8, 1);
        assert!(strip_png(&png[..png.len() - 5]).is_err());
        assert!(strip_png(b"not a png").is_err());
        let webp = simple_webp();
        assert!(strip_webp(&webp[..webp.len() - 5]).is_err());
        assert!(strip_webp(b"RIFF").is_err());
    }

    #[test]
    fn rotated_png_falls_back_to_reencode() {
        let dir = tempfile::tempdir().unwrap();
        let (src, dst) = (dir.path().join("r.png"), dir.path().join("o.png"));
        std::fs::write(&src, png_with(4, 2, 6)).unwrap();
        strip(&src, &dst).unwrap();
        let out = std::fs::read(&dst).unwrap();
        let img = ::image::load_from_memory(&out).unwrap();
        assert_eq!((img.width(), img.height()), (2, 4));
        assert!(!png_chunks(&out).iter().any(|c| &c.0 == b"eXIf"));
    }

    #[test]
    fn upright_png_through_strip_is_byte_stripped() {
        let dir = tempfile::tempdir().unwrap();
        let (src, dst) = (dir.path().join("u.png"), dir.path().join("o.png"));
        let bytes = png_with(4, 2, 1);
        std::fs::write(&src, &bytes).unwrap();
        strip(&src, &dst).unwrap();
        assert_eq!(std::fs::read(&dst).unwrap(), strip_png(&bytes).unwrap());
    }
}
