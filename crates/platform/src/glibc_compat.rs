//! aarch64 glibc builds (PortMaster) must load on glibc 2.17. glibc 2.27
//! added new default versions of `expf` (and friends) that any `f32::exp`
//! links against when built with a newer sysroot; defining `expf` in the
//! executable keeps the binary off `expf@GLIBC_2.27`. It goes through
//! `exp` (still `exp@GLIBC_2.17` in our 2.28 sysroot); `black_box` stops
//! LLVM from folding `(float)exp((double)x)` back into an `expf` call.
//! `scripts/portmaster/check-glibc.py --max 2.17` guards this.

#[unsafe(no_mangle)]
pub extern "C" fn expf(x: f32) -> f32 {
    std::hint::black_box((x as f64).exp()) as f32
}

/// `powf` likewise (glibc 2.27 re-versioned it; `pow` is still
/// `pow@GLIBC_2.17` in the 2.28 sysroot).
#[unsafe(no_mangle)]
pub extern "C" fn powf(x: f32, y: f32) -> f32 {
    std::hint::black_box((x as f64).powf(y as f64)) as f32
}
