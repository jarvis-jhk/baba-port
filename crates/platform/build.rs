//! Adds the project-local SDL2 link directory (see docs/platform.md):
//! Linux: `tools/lib` (a symlink to the system's `libSDL2-2.0.so.0`, so no
//! -dev package is needed; `scripts/bootstrap-tools.sh` creates it);
//! Windows cross builds: `tools/windows/sdl2/x86_64-w64-mingw32/lib`.
//! `BABA_SDL2_LIB_DIR` overrides the directory.

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=BABA_SDL2_LIB_DIR");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_FEATURE_SDL2").is_err() {
        return;
    }
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let target = std::env::var("TARGET").unwrap_or_default();
    let dir = match std::env::var("BABA_SDL2_LIB_DIR") {
        Ok(d) => PathBuf::from(d),
        Err(_) if target.contains("windows") => {
            root.join("tools/windows/sdl2/x86_64-w64-mingw32/lib")
        }
        Err(_) if target == "x86_64-unknown-linux-gnu" => root.join("tools/lib"),
        Err(_) => return,
    };
    if dir.exists() {
        println!(
            "cargo:rustc-link-search=native={}",
            dir.canonicalize().unwrap().display()
        );
    }
}
