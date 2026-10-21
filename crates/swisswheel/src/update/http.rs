//! HTTPS GET over WinHTTP, which ships with Windows and uses the system's
//! proxy and certificate store.

use std::ffi::c_void;

use windows::Win32::Networking::WinHttp::{
    ERROR_WINHTTP_HEADER_NOT_FOUND, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE,
    WINHTTP_OPEN_REQUEST_FLAGS, WINHTTP_QUERY_CONTENT_LENGTH, WINHTTP_QUERY_FLAG_NUMBER,
    WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest,
    WinHttpQueryDataAvailable, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse,
    WinHttpSendRequest, WinHttpSetTimeouts,
};
use windows::core::{HRESULT, HSTRING, PCWSTR, w};

/// Connect, send, and receive timeouts.
const TIMEOUT_MS: i32 = 30_000;

#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("주소 오류: {0}")]
    BadUrl(String),
    /// A non-200 status; 0 for a body cut short of its Content-Length.
    #[error("HTTP {0}")]
    Status(u32),
    #[error("응답이 너무 큽니다")]
    TooLarge,
    #[error("네트워크 오류: {0}")]
    Net(#[from] windows_core::Error),
}

/// GET `url`, following redirects; `progress(received, total)` after each chunk
/// (`total` from Content-Length, `None` if absent). Fails past `limit` bytes.
pub fn get(
    url: &str,
    limit: usize,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<Vec<u8>, HttpError> {
    let target = Target::parse(url)?;
    let agent = HSTRING::from(format!("SwissWheel/{}", env!("CARGO_PKG_VERSION")));
    unsafe {
        let session = Internet::new(WinHttpOpen(
            &agent,
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ))?;
        WinHttpSetTimeouts(session.0, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS)?;
        let connect = Internet::new(WinHttpConnect(
            session.0,
            &HSTRING::from(target.host.as_str()),
            target.port,
            0,
        ))?;
        let flags = if target.secure {
            WINHTTP_FLAG_SECURE
        } else {
            WINHTTP_OPEN_REQUEST_FLAGS(0)
        };
        let request = Internet::new(WinHttpOpenRequest(
            connect.0,
            w!("GET"),
            &HSTRING::from(target.path.as_str()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            flags,
        ))?;
        WinHttpSendRequest(request.0, None, None, 0, 0, 0)?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut())?;

        let status = query_number(request.0, WINHTTP_QUERY_STATUS_CODE)?.unwrap_or(0);
        if status != 200 {
            return Err(HttpError::Status(status));
        }
        let total = query_number(request.0, WINHTTP_QUERY_CONTENT_LENGTH)?.map(u64::from);
        if total.is_some_and(|t| t > limit as u64) {
            return Err(HttpError::TooLarge);
        }
        let mut body = Vec::with_capacity(total.unwrap_or(0) as usize);
        loop {
            let mut available = 0u32;
            WinHttpQueryDataAvailable(request.0, &mut available)?;
            if available == 0 {
                break;
            }
            if body.len() + available as usize > limit {
                return Err(HttpError::TooLarge);
            }
            let start = body.len();
            body.resize(start + available as usize, 0);
            let mut read = 0u32;
            WinHttpReadData(
                request.0,
                body[start..].as_mut_ptr().cast(),
                available,
                &mut read,
            )?;
            body.truncate(start + read as usize);
            progress(body.len() as u64, total);
        }
        if total.is_some_and(|t| (body.len() as u64) < t) {
            return Err(HttpError::Status(0));
        }
        Ok(body)
    }
}

/// A WinHTTP handle, closed on drop.
struct Internet(*mut c_void);

impl Internet {
    fn new(handle: *mut c_void) -> windows_core::Result<Internet> {
        if handle.is_null() {
            Err(windows_core::Error::from_win32())
        } else {
            Ok(Internet(handle))
        }
    }
}

impl Drop for Internet {
    fn drop(&mut self) {
        unsafe {
            let _ = WinHttpCloseHandle(self.0);
        }
    }
}

/// A numeric response header; `None` if the response doesn't have it.
unsafe fn query_number(request: *mut c_void, header: u32) -> windows_core::Result<Option<u32>> {
    let mut value = 0u32;
    let mut len = size_of::<u32>() as u32;
    let result = unsafe {
        WinHttpQueryHeaders(
            request,
            header | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&mut value as *mut u32).cast()),
            &mut len,
            std::ptr::null_mut(),
        )
    };
    match result {
        Ok(()) => Ok(Some(value)),
        Err(e) if e.code() == HRESULT::from_win32(ERROR_WINHTTP_HEADER_NOT_FOUND) => Ok(None),
        Err(e) => Err(e),
    }
}

/// The parts of an `https://host[:port]/path` URL. Plain `http` is allowed
/// only to 127.0.0.1 in debug builds, for the tests and local trials.
#[derive(Debug, PartialEq)]
struct Target {
    secure: bool,
    host: String,
    port: u16,
    path: String,
}

impl Target {
    fn parse(url: &str) -> Result<Target, HttpError> {
        let bad = || HttpError::BadUrl(url.to_string());
        let (secure, rest) = if let Some(rest) = url.strip_prefix("https://") {
            (true, rest)
        } else if let Some(rest) = url.strip_prefix("http://") {
            (false, rest)
        } else {
            return Err(bad());
        };
        let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => (host, port.parse().map_err(|_| bad())?),
            None => (authority, if secure { 443 } else { 80 }),
        };
        if host.is_empty() || (!secure && !(cfg!(debug_assertions) && host == "127.0.0.1")) {
            return Err(bad());
        }
        Ok(Target {
            secure,
            host: host.to_string(),
            port,
            path: if path.is_empty() { "/" } else { path }.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::test_server::{Reply, TestServer};

    fn fetch(url: &str, limit: usize) -> (Result<Vec<u8>, HttpError>, Vec<(u64, Option<u64>)>) {
        let mut seen = Vec::new();
        let result = get(url, limit, &mut |got, total| seen.push((got, total)));
        (result, seen)
    }

    #[test]
    fn gets_body_and_reports_progress() {
        let body: Vec<u8> = (0..102400u32).map(|i| i as u8).collect();
        let server = TestServer::start(vec![("/big", Reply::Ok(body.clone()))]);
        let (result, seen) = fetch(&format!("{}/big", server.base), 1 << 20);
        assert_eq!(result.unwrap(), body);
        assert_eq!(seen.last(), Some(&(102400, Some(102400))));
    }

    #[test]
    fn follows_redirect() {
        let server = TestServer::start(vec![
            ("/a", Reply::Redirect("{base}/b".into())),
            ("/b", Reply::Ok(b"hi".to_vec())),
        ]);
        let (result, _) = fetch(&format!("{}/a", server.base), 1024);
        assert_eq!(result.unwrap(), b"hi");
    }

    #[test]
    fn not_found_is_status_error() {
        let server = TestServer::start(vec![("/gone", Reply::Status(404))]);
        let (result, _) = fetch(&format!("{}/gone", server.base), 1024);
        assert!(matches!(result, Err(HttpError::Status(404))), "{result:?}");
    }

    #[test]
    fn over_limit_fails() {
        let server = TestServer::start(vec![("/x", Reply::Ok(vec![7; 2048]))]);
        let (result, _) = fetch(&format!("{}/x", server.base), 1024);
        assert!(matches!(result, Err(HttpError::TooLarge)), "{result:?}");
    }

    #[test]
    fn truncated_body_fails() {
        let server = TestServer::start(vec![(
            "/cut",
            Reply::Drop {
                send: vec![1; 10],
                claimed: 1000,
            },
        )]);
        let (result, _) = fetch(&format!("{}/cut", server.base), 4096);
        assert!(result.is_err(), "{result:?}");
    }

    /// Needs the internet; `cargo test -- --ignored real_https`.
    #[test]
    #[ignore]
    fn real_https_reaches_github() {
        let (result, _) = fetch(swisswheel_core::update::ENDPOINT, 1 << 20);
        // 404 while the repository is private.
        assert!(
            matches!(result, Ok(_) | Err(HttpError::Status(404))),
            "{result:?}"
        );
    }

    #[test]
    fn rejects_plain_http_elsewhere() {
        let (result, seen) = fetch("http://example.com/x", 1024);
        assert!(matches!(result, Err(HttpError::BadUrl(_))), "{result:?}");
        assert!(seen.is_empty());
    }
}
