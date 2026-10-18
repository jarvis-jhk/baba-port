//! A local mock of the level server for tests (never the real one).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

/// Path+query → (status, body).
pub type Routes = HashMap<String, (u16, Vec<u8>)>;
/// Request lines with their bodies.
pub type Seen = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

pub struct Mock {
    /// `http://127.0.0.1:<port>/prod/`.
    pub base: String,
    /// `http://127.0.0.1:<port>/files/`.
    pub files: String,
    /// Unknown paths get 404.
    pub routes: Arc<Mutex<Routes>>,
    /// Request lines seen (`GET /prod/...`) with their bodies.
    pub seen: Seen,
}

/// Serves `routes` on a local port until the process ends.
pub fn start() -> Mock {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    let seen: Seen = Arc::default();
    let routes: Arc<Mutex<Routes>> = Arc::default();
    let (s2, r2) = (seen.clone(), routes.clone());
    std::thread::spawn(move || {
        for c in l.incoming() {
            let Ok(mut c) = c else { break };
            let mut r = BufReader::new(c.try_clone().unwrap());
            let mut line = String::new();
            if r.read_line(&mut line).is_err() {
                continue;
            }
            let mut len = 0usize;
            loop {
                let mut h = String::new();
                if r.read_line(&mut h).is_err() || h.trim().is_empty() {
                    break;
                }
                if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0; len];
            let _ = r.read_exact(&mut body);
            let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
            s2.lock().unwrap().push((line.trim().to_string(), body));
            let (st, b) = r2
                .lock()
                .unwrap()
                .get(&path)
                .cloned()
                .unwrap_or((404, b"nope".to_vec()));
            let _ = write!(
                c,
                "HTTP/1.1 {st} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                b.len()
            );
            let _ = c.write_all(&b);
        }
    });
    Mock {
        base: format!("http://{addr}/prod/"),
        files: format!("http://{addr}/files/"),
        routes,
        seen,
    }
}

/// A server with one level `ABCD-1234`, a featured page and some error
/// codes.
pub fn standard() -> Mock {
    let m = start();
    let f = &m.files;
    let mut r = m.routes.lock().unwrap();
    r.insert(
        "/prod/get_level?ver=2&level=ABCD-1234".into(),
        (
            200,
            format!("{f}\nname\x01Test\x01author\x01me\x01difficulty\x01sub\n").into_bytes(),
        ),
    );
    r.insert(
        "/prod/get_level?ver=2&level=LIMT-0000".into(),
        (429, Vec::new()),
    );
    r.insert(
        "/prod/get_level?ver=2&level=OLDV-0000".into(),
        (513, Vec::new()),
    );
    r.insert(
        "/prod/get_level?ver=2&level=NONE-0000".into(),
        (200, format!("{f}\n\n").into_bytes()),
    );
    r.insert("/files/ABCD-1234.l".into(), (200, b"LDATA".to_vec()));
    // Like the real server: zlib-wrapped.
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(b"[general]\r\nname=Test\r\n").unwrap();
    r.insert("/files/ABCD-1234.ld".into(), (200, z.finish().unwrap()));
    r.insert("/files/ABCD-1234.png".into(), (200, b"PNG".to_vec()));
    r.insert(
        "/prod/list_levels2?ver=2&category=featured&page=0".into(),
        (
            200,
            format!("{f}\n2\nid\x01ABCD-1234\x01name\x01Test\x01author\x01me\n").into_bytes(),
        ),
    );
    drop(r);
    m
}
