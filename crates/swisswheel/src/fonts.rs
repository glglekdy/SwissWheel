//! Pretendard, embedded in the exe and registered with DirectWrite.

use std::cell::OnceCell;

use windows::Win32::Graphics::DirectWrite::*;
use windows::core::{Interface, PCWSTR, Result, w};

pub const FAMILY: PCWSTR = w!("Pretendard");
/// Used with the system collection when the embedded fonts can't be loaded.
pub const FALLBACK: PCWSTR = w!("Segoe UI Variable Text");

const FILES: [&[u8]; 3] = [
    include_bytes!("../../../assets/fonts/Pretendard-Regular.otf"),
    include_bytes!("../../../assets/fonts/Pretendard-SemiBold.otf"),
    include_bytes!("../../../assets/fonts/Pretendard-Bold.otf"),
];

thread_local! {
    static FACTORY: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) }
        .expect("DirectWrite is available on every supported Windows");
    static COLLECTION: OnceCell<Option<IDWriteFontCollection1>> = const { OnceCell::new() };
}

/// The shared DirectWrite factory.
pub fn factory() -> IDWriteFactory {
    FACTORY.with(Clone::clone)
}

/// The embedded Pretendard collection, or `None` (logged once) when it
/// can't be built, e.g. on Windows without font sets (before 10 1703).
/// Built once per thread; the UI is single-threaded.
pub fn collection() -> Option<IDWriteFontCollection1> {
    COLLECTION.with(|cell| {
        cell.get_or_init(|| {
            load()
                .inspect_err(|e| log_error!("글꼴을 불러올 수 없어 Segoe UI로 그립니다: {e}"))
                .ok()
        })
        .clone()
    })
}

fn load() -> Result<IDWriteFontCollection1> {
    let factory = factory();
    let factory5: IDWriteFactory5 = factory.cast()?;
    unsafe {
        let loader = factory5.CreateInMemoryFontFileLoader()?;
        factory.RegisterFontFileLoader(&loader)?;
        let builder = factory5.CreateFontSetBuilder()?;
        for data in FILES {
            // With an owner object DirectWrite reads `data` in place instead of
            // copying it; the shared factory lives as long as the process.
            let file = loader.CreateInMemoryFontFileReference(
                &factory,
                data.as_ptr().cast(),
                data.len() as u32,
                &factory,
            )?;
            builder.AddFontFile(&file)?;
        }
        factory5.CreateFontCollectionFromFontSet(&builder.CreateFontSet()?)
    }
}

/// Single-line text in Pretendard, centered vertically.
pub fn text_format(
    size: f32,
    weight: DWRITE_FONT_WEIGHT,
    alignment: DWRITE_TEXT_ALIGNMENT,
) -> Result<IDWriteTextFormat> {
    format_in(collection(), size, weight, alignment)
}

/// `text_format` over `fonts`, or the fallback family when it is `None`.
fn format_in(
    fonts: Option<IDWriteFontCollection1>,
    size: f32,
    weight: DWRITE_FONT_WEIGHT,
    alignment: DWRITE_TEXT_ALIGNMENT,
) -> Result<IDWriteTextFormat> {
    let family = if fonts.is_some() { FAMILY } else { FALLBACK };
    let fonts: Option<IDWriteFontCollection> = fonts.map(Into::into);
    unsafe {
        let format = factory().CreateTextFormat(
            family,
            fonts.as_ref(),
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("ko-kr"),
        )?;
        format.SetTextAlignment(alignment)?;
        format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        Ok(format)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::{BOOL, HSTRING};

    fn find_family(fonts: &IDWriteFontCollection1, name: &str) -> Option<IDWriteFontFamily1> {
        let name = HSTRING::from(name);
        let (mut index, mut exists) = (0, BOOL::default());
        unsafe {
            fonts.FindFamilyName(&name, &mut index, &mut exists).ok()?;
            exists
                .as_bool()
                .then(|| fonts.GetFontFamily(index).unwrap())
        }
    }

    fn first_matching(family: &IDWriteFontFamily1, weight: DWRITE_FONT_WEIGHT) -> IDWriteFont {
        unsafe {
            family
                .GetFirstMatchingFont(weight, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL)
                .unwrap()
        }
    }

    trait FontExt {
        fn weight(&self) -> DWRITE_FONT_WEIGHT;
        fn simulations(&self) -> DWRITE_FONT_SIMULATIONS;
        fn has_char(&self, ch: char) -> bool;
    }

    impl FontExt for IDWriteFont {
        fn weight(&self) -> DWRITE_FONT_WEIGHT {
            unsafe { self.GetWeight() }
        }
        fn simulations(&self) -> DWRITE_FONT_SIMULATIONS {
            unsafe { self.GetSimulations() }
        }
        fn has_char(&self, ch: char) -> bool {
            unsafe { self.HasCharacter(ch as u32).unwrap().as_bool() }
        }
    }

    fn family_name(format: &IDWriteTextFormat) -> String {
        unsafe {
            let mut buf = vec![0u16; format.GetFontFamilyNameLength() as usize + 1];
            format.GetFontFamilyName(&mut buf).unwrap();
            String::from_utf16_lossy(&buf[..buf.len() - 1])
        }
    }

    #[test]
    fn pretendard_is_registered_with_real_weights_and_hangul() {
        let fonts = collection().expect("embedded fonts load");
        let family = find_family(&fonts, "Pretendard").expect("family present");
        for (weight, sim) in [
            (DWRITE_FONT_WEIGHT_REGULAR, false),
            (DWRITE_FONT_WEIGHT_SEMI_BOLD, false),
            (DWRITE_FONT_WEIGHT_BOLD, false),
        ] {
            let font = first_matching(&family, weight);
            assert_eq!(font.weight(), weight);
            assert_eq!(font.simulations() != DWRITE_FONT_SIMULATIONS_NONE, sim);
            for ch in ['가', '똠', '뷁', 'A', '→'] {
                assert!(font.has_char(ch), "{ch}");
            }
        }
    }

    #[test]
    fn text_format_uses_pretendard() {
        let f = text_format(13.0, DWRITE_FONT_WEIGHT_BOLD, DWRITE_TEXT_ALIGNMENT_CENTER).unwrap();
        assert_eq!(family_name(&f), "Pretendard");
    }

    #[test]
    fn text_format_falls_back_to_segoe_without_pretendard() {
        let f = format_in(
            None,
            13.0,
            DWRITE_FONT_WEIGHT_BOLD,
            DWRITE_TEXT_ALIGNMENT_CENTER,
        )
        .unwrap();
        assert_eq!(family_name(&f), "Segoe UI Variable Text");
    }
}
