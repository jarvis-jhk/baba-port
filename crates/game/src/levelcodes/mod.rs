//! Online level codes (`docs/spec/levelcodes.md`): download a published
//! level by its code, browse the featured list, keep the code history and
//! the completion of online levels. Upload and report are behind the
//! `upload` build feature, off by default (never exercised against the real
//! server; tests use a local mock).
//!
//! | module | what |
//! |---|---|
//! | (this) | codes, response records, error strings |
//! | [`net`] | transports (HTTPS, record/replay cache), requests, background jobs |
//! | [`store`] | temp/download files, the user `levels` world, history, `[levels_won]` |
//! | `upload` | multipart + zip upload and report (feature `upload`) |

#[cfg(test)]
pub mod mock;
pub mod net;
pub mod store;
#[cfg(feature = "upload")]
pub mod upload;

/// The original's level server (an AWS API Gateway stage; §2.1).
pub const SERVER: &str = "https://8778eno8ha.execute-api.eu-central-1.amazonaws.com/prod/";
/// Protocol version sent as `ver=` (§2.1).
pub const VERSION: u32 = 2;
/// Overrides [`SERVER`] (a local mock in tests).
pub const ENV_SERVER: &str = "BABA_LEVELSERVER";
/// Category of the "Featured levels" button (§4.1).
pub const FEATURED: &str = "featured";

/// Normalises typed input to a code (§1.2): upper case, at most 9
/// characters, a hyphen inserted after 4 when exactly 8 were typed
/// without one. Returns `None` unless the result is `XXXX-XXXX`
/// (letters and digits).
pub fn normalize_code(input: &str) -> Option<String> {
    let mut s: String = input
        .trim()
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if s.chars().count() == 8 && !s.contains('-') {
        s.insert(4, '-');
    }
    is_code(&s).then_some(s)
}

/// `XXXX-XXXX` with ASCII letters/digits (§1.1, §1.4).
pub fn is_code(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 9
        && b[4] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || c.is_ascii_alphanumeric())
}

/// Error strings shown after "Error:" (§5; not translated).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LcError {
    Invalid,
    Timeout,
    UsageLimit,
    OutdatedVersion,
    SpriteError,
    Network,
    /// HTTP 500: the server's text.
    Server(String),
}

impl LcError {
    pub fn as_str(&self) -> &str {
        match self {
            LcError::Invalid => "invalid",
            LcError::Timeout => "timeout",
            LcError::UsageLimit => "usagelimit",
            LcError::OutdatedVersion => "outdatedversion",
            LcError::SpriteError => "spriteerror",
            LcError::Network => "networkerror",
            LcError::Server(t) => t,
        }
    }
}

impl std::fmt::Display for LcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Maps a non-success HTTP status (§5).
pub fn status_error(status: u16, body: &[u8]) -> LcError {
    match status {
        408 => LcError::Timeout,
        429 => LcError::UsageLimit,
        500 => {
            let t = String::from_utf8_lossy(body).trim().to_string();
            if t.is_empty() {
                LcError::Network
            } else {
                LcError::Server(t.chars().take(80).collect())
            }
        }
        512 => LcError::SpriteError,
        513 => LcError::OutdatedVersion,
        _ => LcError::Network,
    }
}

/// One level record (§3.3): `key`, `value` pairs separated by byte 0x01.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Record {
    pub fields: Vec<(String, String)>,
}

impl Record {
    pub fn parse(line: &str) -> Record {
        let line = line.trim_end_matches(['\r', '\n']);
        let mut toks: Vec<&str> = line.split('\u{1}').collect();
        // Tolerate a leading or trailing separator.
        if toks.first() == Some(&"") {
            toks.remove(0);
        }
        if toks.len() % 2 == 1 && toks.last() == Some(&"") {
            toks.pop();
        }
        let fields = toks
            .chunks(2)
            .filter(|p| p.len() == 2)
            .map(|p| (p[0].trim().to_string(), p[1].trim().to_string()))
            .collect();
        Record { fields }
    }

    pub fn get(&self, key: &str) -> &str {
        self.fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .unwrap_or("")
    }

    pub fn id(&self) -> &str {
        self.get("id")
    }
    pub fn name(&self) -> &str {
        self.get("name")
    }
    pub fn author(&self) -> &str {
        self.get("author")
    }
    /// Shown as the subtitle (§3.3).
    pub fn subtitle(&self) -> &str {
        self.get("difficulty")
    }
    /// Shown as the tagline (§3.3).
    pub fn desc(&self) -> &str {
        self.get("desc")
    }
}

fn lines(body: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(body)
        .split('\n')
        .map(|l| l.trim_end_matches('\r').to_string())
        .collect()
}

/// `get_level` response (§3.2): file base URL and the level record.
pub fn parse_level(body: &[u8]) -> Result<(String, Record), LcError> {
    let l = lines(body);
    let base = l.first().map(|s| s.trim().to_string()).unwrap_or_default();
    let rec = Record::parse(l.get(1).map(String::as_str).unwrap_or(""));
    if base.is_empty() || rec.name().is_empty() || rec.author().is_empty() {
        return Err(LcError::Invalid);
    }
    Ok((base, rec))
}

/// One page of a level list (§4.2).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListPage {
    /// Thumbnail base URL (`<base><id>.png`).
    pub thumb_base: String,
    pub pages: usize,
    pub page: usize,
    pub entries: Vec<Record>,
}

/// `list_levels2` response.
pub fn parse_list(body: &[u8], page: usize) -> Result<ListPage, LcError> {
    let l = lines(body);
    let thumb_base = l.first().map(|s| s.trim().to_string()).unwrap_or_default();
    let pages = l
        .get(1)
        .and_then(|s| s.trim().parse::<usize>().ok())
        .ok_or(LcError::Invalid)?;
    let entries = l
        .iter()
        .skip(2)
        .filter(|s| !s.trim().is_empty())
        .map(|s| Record::parse(s))
        .filter(|r| !r.id().is_empty())
        .collect();
    Ok(ListPage {
        thumb_base,
        pages,
        page,
        entries,
    })
}

/// `name` cut for the list (§4.3): 21 chars + "..." when longer than 24.
pub fn cut_name(s: &str) -> String {
    cut(s, 24, 21)
}

/// Tagline cut (§4.3): 69 chars + "..." from 73 on.
pub fn cut_desc(s: &str) -> String {
    cut(s, 72, 69)
}

fn cut(s: &str, max: usize, keep: usize) -> String {
    if s.chars().count() > max {
        format!("{}...", s.chars().take(keep).collect::<String>())
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes() {
        assert_eq!(normalize_code("abcd1234").as_deref(), Some("ABCD-1234"));
        assert_eq!(normalize_code(" ab cd-12 34").as_deref(), Some("ABCD-1234"));
        assert_eq!(normalize_code("ABCD-123"), None);
        assert_eq!(normalize_code("ABCD_1234"), None);
        assert_eq!(normalize_code("ABCDE1234"), None);
        assert!(is_code("Q2W3-E4R5"));
    }

    #[test]
    fn records_and_responses() {
        let body = b"https://files.example/levels/\n\
            name\x01Rock Garden\x01author\x01someone\x01difficulty\x01easy\x01desc\x01push it\x01x\x01y\n";
        let (base, r) = parse_level(body).unwrap();
        assert_eq!(base, "https://files.example/levels/");
        assert_eq!(r.name(), "Rock Garden");
        assert_eq!(r.subtitle(), "easy");
        assert_eq!(r.desc(), "push it");
        assert_eq!(
            parse_level(b"https://x/\nname\x01\x01author\x01a"),
            Err(LcError::Invalid)
        );
        let list = b"https://t/\r\n3\r\nid\x01ABCD-1234\x01name\x01A\x01author\x01b\r\n\
            \x01id\x01EFGH-5678\x01name\x01B\x01author\x01c\x01\r\n";
        let p = parse_list(list, 1).unwrap();
        assert_eq!(p.pages, 3);
        assert_eq!(p.entries.len(), 2);
        assert_eq!(p.entries[1].id(), "EFGH-5678");
        assert_eq!(p.entries[1].author(), "c");
    }

    #[test]
    fn errors_and_cuts() {
        assert_eq!(status_error(429, b"").as_str(), "usagelimit");
        assert_eq!(status_error(513, b"").as_str(), "outdatedversion");
        assert_eq!(status_error(408, b"").as_str(), "timeout");
        assert_eq!(status_error(500, b"bad level\n").as_str(), "bad level");
        assert_eq!(status_error(404, b"").as_str(), "networkerror");
        assert_eq!(cut_name("short"), "short");
        assert_eq!(cut_name(&"x".repeat(25)), format!("{}...", "x".repeat(21)));
    }
}
