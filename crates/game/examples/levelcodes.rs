//! Level-code client from the command line (development):
//!
//! ```sh
//! BABA_LEVELCODES_CACHE=scratch/m9/server-cache \
//!   cargo run -p baba-port --example levelcodes -- featured [PAGE]
//! BABA_LEVELCODES_CACHE=scratch/m9/server-cache \
//!   cargo run -p baba-port --example levelcodes -- get CODE OUTDIR
//! ```
//!
//! Every response is recorded in the cache folder and served from there
//! afterwards (`BABA_LEVELCODES_OFFLINE=1`: never go online), so the real
//! server is contacted at most once per URL. Download only; there is no
//! upload here.

use std::path::PathBuf;

use baba_game::levelcodes::net::Server;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let s = Server::from_env();
    eprintln!(
        "server {} cache {:?} offline {}",
        s.base, s.cache, s.offline
    );
    match args.first().map(String::as_str) {
        Some("featured") => {
            let page = args.get(1).and_then(|p| p.parse().ok()).unwrap_or(0);
            // THUMBS=DIR also fetches the thumbnails (one request each).
            let thumbs = std::env::var_os("THUMBS").map(PathBuf::from);
            match s.featured(page, thumbs.as_deref()) {
                Ok(p) => {
                    println!("pages {} thumbs {}", p.pages, p.thumb_base);
                    for r in &p.entries {
                        println!(
                            "{}\t{}\tby {}\t{}\t{}",
                            r.id(),
                            r.name(),
                            r.author(),
                            r.subtitle(),
                            r.desc()
                        );
                    }
                }
                Err(e) => println!("Error: {e}"),
            }
        }
        Some("get") if args.len() >= 3 => match s.download(&args[1], &PathBuf::from(&args[2])) {
            Ok(d) => println!(
                "{} {:?} png={} -> {}",
                d.code,
                d.record.fields,
                d.has_png,
                d.dir.display()
            ),
            Err(e) => println!("Error: {e}"),
        },
        _ => eprintln!("usage: levelcodes featured [PAGE] | get CODE OUTDIR"),
    }
}
