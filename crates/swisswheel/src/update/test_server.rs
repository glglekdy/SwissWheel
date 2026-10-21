//! A tiny HTTP/1.1 server on 127.0.0.1 for the update tests.

use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

pub enum Reply {
    Ok(Vec<u8>),
    Status(u16),
    /// 302 to this location.
    Redirect(String),
    /// Claims `claimed` bytes, sends `send`, then closes the connection.
    Drop {
        send: Vec<u8>,
        claimed: usize,
    },
}

pub struct TestServer {
    /// "http://127.0.0.1:PORT"
    pub base: String,
}

impl TestServer {
    /// Serves `routes` by exact path until the test process ends. The text
    /// `{base}` in a body or a redirect is replaced with the server's base URL,
    /// so a release JSON can point at the server's own files.
    pub fn start(routes: Vec<(&'static str, Reply)>) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let routes: Arc<Vec<(&'static str, Reply)>> = Arc::new(
            routes
                .into_iter()
                .map(|(path, reply)| (path, fill_base(reply, &base)))
                .collect(),
        );
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let routes = Arc::clone(&routes);
                std::thread::spawn(move || serve(stream, &routes));
            }
        });
        TestServer { base }
    }
}

fn fill_base(reply: Reply, base: &str) -> Reply {
    match reply {
        Reply::Ok(body) => match String::from_utf8(body) {
            Ok(text) => Reply::Ok(text.replace("{base}", base).into_bytes()),
            Err(e) => Reply::Ok(e.into_bytes()),
        },
        Reply::Redirect(to) => Reply::Redirect(to.replace("{base}", base)),
        other => other,
    }
}

fn serve(mut stream: TcpStream, routes: &[(&'static str, Reply)]) {
    let mut request = Vec::new();
    let mut buf = [0u8; 1024];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => request.extend_from_slice(&buf[..n]),
        }
    }
    let text = String::from_utf8_lossy(&request);
    let path = text.split_whitespace().nth(1).unwrap_or("/").to_string();
    let reply = routes.iter().find(|(p, _)| *p == path).map(|(_, r)| r);
    let _ = match reply {
        Some(Reply::Ok(body)) => respond(&mut stream, 200, "", body, body.len()),
        Some(Reply::Status(code)) => respond(&mut stream, *code, "", b"", 0),
        Some(Reply::Redirect(to)) => {
            respond(&mut stream, 302, &format!("Location: {to}\r\n"), b"", 0)
        }
        Some(Reply::Drop { send, claimed }) => respond(&mut stream, 200, "", send, *claimed),
        None => respond(&mut stream, 404, "", b"", 0),
    };
}

fn respond(
    stream: &mut TcpStream,
    code: u16,
    headers: &str,
    body: &[u8],
    length: usize,
) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {code} X\r\nContent-Length: {length}\r\nConnection: close\r\n{headers}\r\n"
    )?;
    stream.write_all(body)?;
    stream.flush()
}
