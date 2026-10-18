//! Fault reports: panics and start-up failures are posted to the owner's
//! report endpoint, fire-and-forget (`docs/platform-android.md`, "Fault
//! reports").
//!
//! Only private builds report: the endpoint comes from `BABA_REPORT_URL` at
//! compile time (`scripts/android/private.sh` reads it from the git-ignored
//! `keys/`); without it every call here is a no-op. A panic is written to
//! `files/crash-report.json` by the hook (no network while panicking) and
//! sent in the background at the next start, then removed. Reports carry the
//! version, device model and the message, nothing personal.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

/// Compile-time endpoint (private builds only).
pub const URL: Option<&str> = option_env!("BABA_REPORT_URL");

const PENDING: &str = "crash-report.json";
const LANE: &str = "baba-port";

struct Reporter {
    url: &'static str,
    dir: PathBuf,
    source: String,
}

static REPORTER: OnceLock<Reporter> = OnceLock::new();

/// JSON string literal for `s`.
fn json_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// The report body (kind "bug").
pub fn body(source: &str, title: &str, detail: &str) -> String {
    // Titles are one line; details are capped so a huge message stays small.
    let title: String = title
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(200)
        .collect();
    let detail: String = detail.chars().take(4000).collect();
    format!(
        "{{\"lane\":{},\"kind\":\"bug\",\"title\":{},\"detail\":{},\"source\":{}}}",
        json_str(LANE),
        json_str(&title),
        json_str(&detail),
        json_str(source)
    )
}

fn post(url: &str, body: &str) -> bool {
    match baba_http::post(
        url,
        "application/json",
        body.as_bytes(),
        Duration::from_secs(20),
    ) {
        Ok(r) if (200..300).contains(&r.status) => true,
        Ok(r) => {
            log::warn!("fault report: HTTP {}", r.status);
            false
        }
        Err(e) => {
            log::warn!("fault report: {e}");
            false
        }
    }
}

fn send_pending(url: &'static str, dir: &Path) {
    let path = dir.join(PENDING);
    if let Ok(body) = std::fs::read_to_string(&path)
        && post(url, &body)
    {
        let _ = std::fs::remove_file(&path);
        log::info!("fault report from the last run sent");
    }
}

/// Arms the reporter: panic hook (keeps logging to logcat) and a background
/// send of the last run's panic, if any. `source` names app, version, device.
pub fn init(dir: PathBuf, source: String) {
    let Some(url) = URL else { return };
    if REPORTER.set(Reporter { url, dir, source }).is_err() {
        return;
    }
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        prev(info);
        if let Some(r) = REPORTER.get() {
            let msg = info
                .payload()
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "(non-string payload)".into());
            let at = info
                .location()
                .map(|l| format!("{}:{}", l.file(), l.line()))
                .unwrap_or_default();
            let thread = std::thread::current().name().unwrap_or("?").to_string();
            let b = body(
                &r.source,
                &format!("Android panic: {msg}"),
                &format!("panic at {at} (thread {thread}): {msg}"),
            );
            let _ = std::fs::write(r.dir.join(PENDING), b);
        }
    }));
    std::thread::spawn(move || {
        if let Some(r) = REPORTER.get() {
            send_pending(r.url, &r.dir);
        }
    });
}

/// Reports a fault that stops the app (fire-and-forget, background thread;
/// kept as pending first, so a report the exit cuts off goes next start).
pub fn report(title: &str, detail: &str) {
    let Some(r) = REPORTER.get() else { return };
    if std::fs::write(r.dir.join(PENDING), body(&r.source, title, detail)).is_ok() {
        std::thread::spawn(move || send_pending(r.url, &r.dir));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_is_escaped_json() {
        let b = body(
            "baba-port 0.1.0 android X",
            "a \"b\"\nsecond",
            "x\\y\t\u{1}",
        );
        assert_eq!(
            b,
            "{\"lane\":\"baba-port\",\"kind\":\"bug\",\"title\":\"a \\\"b\\\"\",\
             \"detail\":\"x\\\\y\\t\\u0001\",\"source\":\"baba-port 0.1.0 android X\"}"
        );
    }
}
