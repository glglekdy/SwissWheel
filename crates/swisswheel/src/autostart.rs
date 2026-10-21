//! "Windows 시작 시 실행": the `SwissWheel` value under the user's Run key.
#![expect(dead_code, reason = "used once the menu is wired into the app")]

use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ,
    RegCloseKey, RegCreateKeyExW, RegDeleteKeyValueW, RegGetValueW, RegSetValueExW,
};
use windows::core::{PCWSTR, Result, w};

const RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const VALUE: PCWSTR = w!("SwissWheel");

/// True only if the Run value points at this executable, so an entry left
/// by a copy elsewhere is rewritten at startup.
pub fn is_enabled() -> bool {
    is_enabled_in(RUN)
}

pub fn set(enabled: bool) -> Result<()> {
    set_in(RUN, enabled)
}

/// The Run command for this executable: its full path in quotes.
fn command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\"", exe.display()))
}

fn is_enabled_in(subkey: PCWSTR) -> bool {
    let mut buf = [0u16; 1024];
    let mut size = size_of_val(&buf) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey,
            VALUE,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status.is_err() {
        return false;
    }
    let chars = (size as usize / 2).saturating_sub(1); // size includes the NUL
    let stored = String::from_utf16_lossy(&buf[..chars.min(buf.len())]);
    command().is_some_and(|c| stored.eq_ignore_ascii_case(&c))
}

fn set_in(subkey: PCWSTR, enabled: bool) -> Result<()> {
    if !enabled {
        let status = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, subkey, VALUE) };
        return if status == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            status.ok()
        };
    }
    let command = command().ok_or_else(windows::core::Error::from_win32)?;
    let data: Vec<u8> = command
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    let mut key = HKEY::default();
    unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            subkey,
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
        .ok()?;
        let result = RegSetValueExW(key, VALUE, None, REG_SZ, Some(&data)).ok();
        let _ = RegCloseKey(key);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RegDeleteTreeW};
    use windows::core::w;

    #[test]
    fn set_and_clear_round_trip() {
        let key = w!("Software\\SwissWheelTest\\Run");
        let _ = set_in(key, false);
        assert!(!is_enabled_in(key));
        set_in(key, true).unwrap();
        assert!(is_enabled_in(key));
        set_in(key, false).unwrap();
        assert!(!is_enabled_in(key));
        set_in(key, false).unwrap(); // clearing twice is fine
        unsafe {
            let _ = RegDeleteTreeW(HKEY_CURRENT_USER, w!("Software\\SwissWheelTest"));
        }
    }
}
