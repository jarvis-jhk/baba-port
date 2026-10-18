//! `log` → logcat (tag `babaport`).

use std::ffi::{CStr, CString, c_char};

const TAG: &CStr = c"babaport";

#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(prio: i32, tag: *const c_char, text: *const c_char) -> i32;
}

struct Logger;

impl log::Log for Logger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Info || m.target().starts_with("baba")
    }

    fn log(&self, r: &log::Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let prio = match r.level() {
            log::Level::Error => 6,
            log::Level::Warn => 5,
            log::Level::Info => 4,
            log::Level::Debug => 3,
            log::Level::Trace => 2,
        };
        let msg = CString::new(format!("{}", r.args()).replace('\0', "?")).unwrap_or_default();
        unsafe { __android_log_write(prio, TAG.as_ptr(), msg.as_ptr()) };
    }

    fn flush(&self) {}
}

static LOGGER: Logger = Logger;

pub fn init() {
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Info);
    std::panic::set_hook(Box::new(|p| log::error!("panic: {p}")));
}
