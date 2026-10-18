//! Minimal stderr logger; `BABA_LOG=debug|info|warn|error` (default info).

use log::{Level, LevelFilter, Log, Metadata, Record};

struct Stderr(Level);

impl Log for Stderr {
    fn enabled(&self, m: &Metadata) -> bool {
        m.level() <= self.0
    }

    fn log(&self, r: &Record) {
        if self.enabled(r.metadata()) {
            eprintln!("[{} {}] {}", r.level(), r.target(), r.args());
        }
    }

    fn flush(&self) {}
}

pub fn init() {
    let level = match std::env::var("BABA_LOG").as_deref() {
        Ok("debug") => Level::Debug,
        Ok("trace") => Level::Trace,
        Ok("warn") => Level::Warn,
        Ok("error") => Level::Error,
        _ => Level::Info,
    };
    let logger: &'static Stderr = Box::leak(Box::new(Stderr(level)));
    if log::set_logger(logger).is_ok() {
        log::set_max_level(LevelFilter::Trace);
    }
}
