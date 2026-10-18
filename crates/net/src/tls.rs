//! TLS crypto for the Warpzone API client (ureq + rustls).
//!
//! Desktop builds use ring (enabled through `ureq/_ring` in this crate's
//! manifest). Android and aarch64 Linux (PortMaster) are cross-built without
//! a C compiler, so ring is left out there (see `crates/vendor/ureq`) and a
//! pure-Rust provider (`rustls-rustcrypto`) is installed as the process-wide
//! rustls default instead. [`NetSession::new`](crate::NetSession::new) calls
//! [`install_crypto_provider`]; anything else that talks HTTPS through
//! `warpzone_core` before that should call it first.

/// True where this build carries its own pure-Rust rustls provider.
pub const PURE_RUST_TLS: bool = cfg!(any(
    target_os = "android",
    all(target_os = "linux", target_arch = "aarch64")
));

/// Installs the process-wide rustls provider where ring is not built in.
/// Idempotent; a no-op on desktop targets (ureq picks ring itself).
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

/// Name of the TLS crypto backend, for logs.
pub fn crypto_backend() -> &'static str {
    if PURE_RUST_TLS {
        "rustls + rustls-rustcrypto (pure Rust)"
    } else {
        "rustls + ring"
    }
}
