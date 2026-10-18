//! Platform-stable hashing (FNV-1a 64 over explicit little-endian bytes).

/// FNV-1a 64-bit hasher with explicit, endianness-independent writes.
#[derive(Clone, Copy, Debug)]
pub struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Fnv::new()
    }
}

impl Fnv {
    /// A fresh hasher.
    pub fn new() -> Fnv {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    /// Feed bytes.
    pub fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.0 ^= x as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    /// Feed a length-prefixed string.
    pub fn str(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.bytes(s.as_bytes());
    }
    /// Feed a byte.
    pub fn u8(&mut self, v: u8) {
        self.bytes(&[v]);
    }
    /// Feed a u32.
    pub fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }
    /// Feed an i32.
    pub fn i32(&mut self, v: i32) {
        self.bytes(&v.to_le_bytes());
    }
    /// Feed a u64.
    pub fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
    /// Feed a bool.
    pub fn bool(&mut self, v: bool) {
        self.u8(v as u8);
    }
    /// The hash.
    pub fn finish(&self) -> u64 {
        self.0
    }
}
