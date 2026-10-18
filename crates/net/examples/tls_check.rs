//! One HTTPS GET of the Warpzone server's public info (no account, no
//! identity file written): checks the TLS stack of a build, e.g. the
//! pure-Rust provider on PortMaster under qemu (`docs/platform-portmaster.md`).
//! Usage: tls_check [server-url]   (default: WARPZONE_SERVER or the public one)

fn main() {
    baba_net::tls::install_crypto_provider();
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(baba_net::warpzone_core::server_url);
    let api = baba_net::warpzone_core::api::Api::new(
        &url,
        std::sync::Arc::new(baba_net::warpzone_core::identity::Identity::generate()),
    );
    let t = std::time::Instant::now();
    match api.info() {
        Ok(i) => println!(
            "HTTPS {url}: OK in {} ms via {}: {} {}",
            t.elapsed().as_millis(),
            baba_net::tls::crypto_backend(),
            i.name,
            i.version
        ),
        Err(e) => {
            eprintln!("HTTPS {url}: {e}");
            std::process::exit(1);
        }
    }
}
