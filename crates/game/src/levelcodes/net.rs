//! Talking to the level server: a [`Server`] (base URL + transport), the
//! download and list requests (§3, §4) and a background [`Job`] the UI
//! polls while it shows its waiting screen (§2.3).
//!
//! Transports: plain HTTPS ([`baba_http`]); or, for development and tests,
//! a record/replay cache folder (`BABA_LEVELCODES_CACHE`): responses with
//! status 200 are stored by URL and served from there afterwards, so the
//! real server is contacted at most once per URL. With
//! `BABA_LEVELCODES_OFFLINE=1` a cache miss fails with `networkerror`
//! instead of going online.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Duration;

use baba_http::Response;

use super::store::{Downloaded, TEMP};
use super::{FEATURED, LcError, ListPage, Record, SERVER, VERSION, parse_level, parse_list};

pub const ENV_CACHE: &str = "BABA_LEVELCODES_CACHE";
pub const ENV_OFFLINE: &str = "BABA_LEVELCODES_OFFLINE";

/// Client timeout (the original's value is unknown; 15 s is generous for
/// a few KB).
pub const TIMEOUT: Duration = Duration::from_secs(15);

/// Where requests go and how.
#[derive(Clone, Debug)]
pub struct Server {
    /// API base ending in `/` (`…/prod/`).
    pub base: String,
    /// Record/replay folder.
    pub cache: Option<PathBuf>,
    /// Never touch the network (cache only).
    pub offline: bool,
    pub timeout: Duration,
}

impl Server {
    /// The real server, or the overrides from the environment.
    pub fn from_env() -> Server {
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let mut base = env(super::ENV_SERVER).unwrap_or_else(|| SERVER.to_string());
        if !base.ends_with('/') {
            base.push('/');
        }
        Server {
            base,
            cache: env(ENV_CACHE).map(PathBuf::from),
            offline: env(ENV_OFFLINE).is_some_and(|v| v != "0"),
            timeout: TIMEOUT,
        }
    }

    pub fn at(base: &str) -> Server {
        Server {
            base: base.to_string(),
            cache: None,
            offline: false,
            timeout: TIMEOUT,
        }
    }

    fn cache_file(&self, url: &str) -> Option<PathBuf> {
        let dir = self.cache.as_ref()?;
        let name: String = url
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        Some(dir.join(name))
    }

    /// `GET` through the cache / network.
    pub fn get(&self, url: &str) -> Result<Response, LcError> {
        let cf = self.cache_file(url);
        if let Some(f) = &cf
            && let Ok(body) = std::fs::read(f)
        {
            log::info!("level server (cached): {url}");
            return Ok(Response { status: 200, body });
        }
        if self.offline {
            log::warn!("level server offline, not cached: {url}");
            return Err(LcError::Network);
        }
        log::info!("level server: GET {url}");
        let r = baba_http::get(url, self.timeout).map_err(|e| match e {
            baba_http::Error::Timeout => LcError::Timeout,
            baba_http::Error::Network(m) => {
                log::warn!("{url}: {m}");
                LcError::Network
            }
        })?;
        if r.status == 200
            && let Some(f) = &cf
        {
            if let Some(d) = f.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(f, &r.body);
        }
        Ok(r)
    }

    fn get_ok(&self, url: &str) -> Result<Vec<u8>, LcError> {
        let r = self.get(url)?;
        if r.status != 200 {
            log::warn!("{url}: HTTP {}", r.status);
            return Err(super::status_error(r.status, &r.body));
        }
        Ok(r.body)
    }

    pub fn api(&self, path: &str, query: &str) -> String {
        format!("{}{path}?ver={VERSION}{query}", self.base)
    }

    /// §3: the record, then `<base><CODE>.l/.ld/.png` into `temp_dir` as
    /// `temp.*`. A missing preview is not fatal.
    pub fn download(&self, code: &str, temp_dir: &Path) -> Result<Downloaded, LcError> {
        let Some(code) = super::normalize_code(code) else {
            return Err(LcError::Invalid);
        };
        let body = self.get_ok(&self.api("get_level", &format!("&level={code}")))?;
        let (base, rec) = parse_level(&body)?;
        let l = inflate(self.get_ok(&format!("{base}{code}.l"))?)?;
        let ld = inflate(self.get_ok(&format!("{base}{code}.ld"))?)?;
        let png = self.get_ok(&format!("{base}{code}.png")).ok();
        std::fs::create_dir_all(temp_dir).map_err(|_| LcError::Network)?;
        let ld = super::store::with_levelcode(&ld, &code);
        let w = |ext: &str, b: &[u8]| std::fs::write(temp_dir.join(format!("{TEMP}.{ext}")), b);
        w("l", &l).map_err(|_| LcError::Network)?;
        w("ld", &ld).map_err(|_| LcError::Network)?;
        let _ = std::fs::remove_file(temp_dir.join(format!("{TEMP}.png")));
        if let Some(p) = &png {
            let _ = w("png", p);
        }
        Ok(Downloaded {
            code,
            record: rec,
            dir: temp_dir.to_path_buf(),
            has_png: png.is_some(),
        })
    }

    /// §4: one page of a category; thumbnails go to `thumbs` as `<id>.png`
    /// (kept, so each is fetched once; `None` = no thumbnails).
    pub fn list(
        &self,
        category: &str,
        page: usize,
        thumbs: Option<&Path>,
    ) -> Result<ListPage, LcError> {
        let body =
            self.get_ok(&self.api("list_levels2", &format!("&category={category}&page={page}")))?;
        let p = parse_list(&body, page)?;
        if let Some(thumbs) = thumbs
            && !p.thumb_base.is_empty()
        {
            let _ = std::fs::create_dir_all(thumbs);
            for r in &p.entries {
                let id = r.id();
                if !super::is_code(id) {
                    continue;
                }
                let f = thumbs.join(format!("{id}.png"));
                if f.is_file() {
                    continue;
                }
                if let Ok(b) = self.get_ok(&format!("{}{id}.png", p.thumb_base)) {
                    let _ = std::fs::write(&f, b);
                }
            }
        }
        Ok(p)
    }

    pub fn featured(&self, page: usize, thumbs: Option<&Path>) -> Result<ListPage, LcError> {
        self.list(FEATURED, page, thumbs)
    }
}

/// The server keeps `.l` and `.ld` zlib-wrapped (seen on a real download,
/// 2026-10-18); plain files (`ACHTUNG!…`, `[…`) pass through.
pub fn inflate(b: Vec<u8>) -> Result<Vec<u8>, LcError> {
    let zlib =
        b.len() >= 2 && b[0] & 0x0f == 8 && u16::from_be_bytes([b[0], b[1]]).is_multiple_of(31);
    if !zlib {
        return Ok(b);
    }
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(&b[..]), &mut out).map_err(
        |e| {
            log::warn!("level file: bad zlib data: {e}");
            LcError::Invalid
        },
    )?;
    Ok(out)
}

/// What a background job produced.
#[derive(Debug)]
pub enum JobResult {
    Level(Result<Downloaded, LcError>),
    List(Result<ListPage, LcError>),
    /// Upload: the new code (feature `upload`).
    Uploaded(Result<String, LcError>),
}

/// A request running on a worker thread (§2.3).
pub struct Job {
    rx: Receiver<JobResult>,
    /// What the waiting screen says.
    pub what: JobKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobKind {
    Download,
    List,
    Upload,
}

impl Job {
    pub fn spawn(what: JobKind, f: impl FnOnce() -> JobResult + Send + 'static) -> Job {
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        Job { rx, what }
    }

    pub fn download(server: Server, code: String, temp_dir: PathBuf) -> Job {
        Job::spawn(JobKind::Download, move || {
            JobResult::Level(server.download(&code, &temp_dir))
        })
    }

    pub fn featured(server: Server, page: usize, thumbs: PathBuf) -> Job {
        Job::spawn(JobKind::List, move || {
            JobResult::List(server.featured(page, Some(&thumbs)))
        })
    }

    /// The result, once there.
    pub fn poll(&self) -> Option<JobResult> {
        match self.rx.try_recv() {
            Ok(r) => Some(r),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(JobResult::Level(Err(LcError::Network))),
        }
    }

    /// Blocks (tests, scripted runs).
    pub fn wait(self) -> JobResult {
        self.rx
            .recv()
            .unwrap_or(JobResult::Level(Err(LcError::Network)))
    }
}

/// A record for display when only the code is known.
pub fn record_for(code: &str, name: &str) -> Record {
    Record {
        fields: vec![("id".into(), code.into()), ("name".into(), name.into())],
    }
}

#[cfg(test)]
mod tests {
    use super::super::mock;
    use super::*;

    fn tmp(n: &str) -> PathBuf {
        let d = std::env::var_os("TMPDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("target"))
            .join(format!("baba-lc-{n}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn download_list_and_errors_against_a_mock() {
        let m = mock::standard();
        let s = Server::at(&m.base);
        let tmp = tmp("dl");
        let d = s.download("abcd1234", &tmp).unwrap();
        assert_eq!(d.code, "ABCD-1234");
        assert_eq!(d.record.subtitle(), "sub");
        assert_eq!(std::fs::read(tmp.join("temp.l")).unwrap(), b"LDATA");
        let ld = String::from_utf8(std::fs::read(tmp.join("temp.ld")).unwrap()).unwrap();
        assert!(ld.contains("levelcode=ABCD-1234"), "{ld}");
        assert!(d.has_png);
        assert_eq!(
            s.download("LIMT-0000", &tmp).unwrap_err(),
            LcError::UsageLimit
        );
        assert_eq!(
            s.download("OLDV-0000", &tmp).unwrap_err(),
            LcError::OutdatedVersion
        );
        assert_eq!(s.download("NONE-0000", &tmp).unwrap_err(), LcError::Invalid);
        assert_eq!(s.download("XXXX-YYYY", &tmp).unwrap_err(), LcError::Network);
        assert_eq!(s.download("short", &tmp).unwrap_err(), LcError::Invalid);
        let thumbs = tmp.join("thumbs");
        let p = s.featured(0, Some(&thumbs)).unwrap();
        assert_eq!(p.pages, 2);
        assert_eq!(std::fs::read(thumbs.join("ABCD-1234.png")).unwrap(), b"PNG");
        let j = Job::download(s.clone(), "ABCD-1234".into(), tmp.clone());
        assert!(matches!(j.wait(), JobResult::Level(Ok(_))));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn cache_records_then_replays_offline() {
        let m = mock::standard();
        let cache = tmp("cache");
        let mut s = Server::at(&m.base);
        s.cache = Some(cache.clone());
        let t = cache.join("t");
        s.download("ABCD-1234", &t).unwrap();
        let n = m.seen.lock().unwrap().len();
        assert_eq!(n, 4);
        s.offline = true;
        let d = s.download("ABCD-1234", &t).unwrap();
        assert_eq!(d.record.name(), "Test");
        assert_eq!(m.seen.lock().unwrap().len(), n);
        assert_eq!(s.download("EFGH-1234", &t).unwrap_err(), LcError::Network);
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn timeout_maps_to_timeout() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/prod/", l.local_addr().unwrap());
        let h = std::thread::spawn(move || {
            let c = l.accept();
            std::thread::sleep(Duration::from_millis(700));
            drop(c);
        });
        let mut s = Server::at(&base);
        s.timeout = Duration::from_millis(200);
        assert_eq!(
            s.download("ABCD-1234", &tmp("to")).unwrap_err(),
            LcError::Timeout
        );
        h.join().unwrap();
    }
}
