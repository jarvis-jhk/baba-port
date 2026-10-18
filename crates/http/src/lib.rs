//! # baba-http
//!
//! A deliberately small blocking HTTP(S) client for baba-port's online
//! level codes: `GET` and `POST` with one overall timeout, the status code
//! returned as a value (callers map codes to the original's error strings),
//! and response bodies capped in size. TLS is rustls through the workspace's
//! patched ureq (ring on desktop, a pure-Rust provider on Android and
//! aarch64 Linux, as in `baba_net::tls`).
//!
//! Plain `http://` URLs work too, which is how the tests talk to a local
//! mock server.

use std::time::Duration;

/// Largest body we accept (levels and thumbnails are a few KB).
pub const MAX_BODY: u64 = 8 << 20;

/// User agent sent with every request.
pub const USER_AGENT: &str = concat!("baba-port/", env!("CARGO_PKG_VERSION"));

/// A completed exchange (any status).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Response {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// Transport failures (no HTTP status).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The overall timeout elapsed (connect, send or receive).
    Timeout,
    /// DNS, connection, TLS or protocol failure.
    Network(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Timeout => write!(f, "timeout"),
            Error::Network(e) => write!(f, "network error: {e}"),
        }
    }
}

impl std::error::Error for Error {}

/// Installs the process-wide rustls provider where ring is not built in
/// (idempotent; a no-op on desktop targets).
pub fn install_crypto_provider() {
    #[cfg(any(
        target_os = "android",
        all(target_os = "linux", target_arch = "aarch64")
    ))]
    {
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            let _ = rustls::crypto::CryptoProvider::install_default(rustls_rustcrypto::provider());
        }
    }
}

fn agent(timeout: Duration) -> ureq::Agent {
    install_crypto_provider();
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .user_agent(USER_AGENT)
        .build()
        .into()
}

fn map_err(e: ureq::Error) -> Error {
    match e {
        ureq::Error::Timeout(_) => Error::Timeout,
        ureq::Error::Io(io) if io.kind() == std::io::ErrorKind::TimedOut => Error::Timeout,
        other => Error::Network(other.to_string()),
    }
}

fn finish(r: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Response, Error> {
    let mut r = r.map_err(map_err)?;
    let status = r.status().as_u16();
    let body = r
        .body_mut()
        .with_config()
        .limit(MAX_BODY)
        .read_to_vec()
        .map_err(map_err)?;
    Ok(Response { status, body })
}

/// `GET url`.
pub fn get(url: &str, timeout: Duration) -> Result<Response, Error> {
    log::debug!("GET {url}");
    finish(agent(timeout).get(url).call())
}

/// `POST url` with a body of `content_type`.
pub fn post(
    url: &str,
    content_type: &str,
    body: &[u8],
    timeout: Duration,
) -> Result<Response, Error> {
    log::debug!("POST {url} ({} bytes)", body.len());
    finish(
        agent(timeout)
            .post(url)
            .header("Content-Type", content_type)
            .send(body),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// One-shot local server answering with `reply` (raw HTTP).
    fn serve(reply: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        let h = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = s.read(&mut buf).unwrap();
            s.write_all(reply.as_bytes()).unwrap();
            String::from_utf8_lossy(&buf[..n]).into_owned()
        });
        (url, h)
    }

    #[test]
    fn get_returns_status_and_body() {
        let (url, h) =
            serve("HTTP/1.1 429 Too Many\r\nContent-Length: 2\r\nConnection: close\r\n\r\nno");
        let r = get(&format!("{url}/x?a=1"), Duration::from_secs(5)).unwrap();
        assert_eq!(r.status, 429);
        assert_eq!(r.body, b"no");
        let req = h.join().unwrap();
        assert!(req.starts_with("GET /x?a=1 HTTP/1.1"));
        assert!(req.contains(USER_AGENT));
    }

    #[test]
    fn timeout_is_reported() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", l.local_addr().unwrap());
        // Accept but never answer.
        let h = std::thread::spawn(move || {
            let c = l.accept();
            std::thread::sleep(Duration::from_millis(800));
            drop(c);
        });
        let r = get(&url, Duration::from_millis(300));
        assert_eq!(r, Err(Error::Timeout));
        h.join().unwrap();
    }

    #[test]
    fn refused_connection_is_a_network_error() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", l.local_addr().unwrap());
        drop(l);
        assert!(matches!(
            get(&url, Duration::from_secs(2)),
            Err(Error::Network(_))
        ));
    }
}
