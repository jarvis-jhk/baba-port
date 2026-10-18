//! The rules PRNG (`docs/spec/turn.md` §12): a small, platform-independent
//! generator stored in the level state and in every undo record.

/// SplitMix64. Deterministic on every platform; `Copy` so it can be saved in
/// undo records for free.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rng {
    state: u64,
}

impl Rng {
    /// A generator from a seed.
    pub fn new(seed: u64) -> Rng {
        Rng { state: seed }
    }

    /// Seed for a level start: a fixed function of world, level file and
    /// unit count (the port's analogue of the original's seeding mode 2).
    pub fn for_level(world: &str, stem: &str, units: usize) -> Rng {
        let mut h = crate::hash::Fnv::new();
        h.bytes(world.as_bytes());
        h.u8(0);
        h.bytes(stem.as_bytes());
        h.u64(units as u64);
        Rng::new(h.finish())
    }

    /// Raw state (hashing).
    pub fn state(&self) -> u64 {
        self.state
    }

    /// Next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform integer in `lo..=hi` (`lo` if the range is empty).
    pub fn range(&mut self, lo: i32, hi: i32) -> i32 {
        if hi <= lo {
            let _ = self.next_u64();
            return lo;
        }
        let span = (hi as i64 - lo as i64 + 1) as u64;
        // Rejection sampling keeps the draw unbiased and platform-exact.
        let zone = u64::MAX - (u64::MAX % span);
        loop {
            let v = self.next_u64();
            if v < zone {
                return lo + (v % span) as i32;
            }
        }
    }
}

/// The cosmetic generator (`turn.md` §12.1): sound variants, particle
/// chances and jitter, wobble start frames. Owned by the presentation,
/// never part of the simulation state, never touched by the sim.
#[derive(Clone, Debug)]
pub struct CosmeticRng(Rng);

impl CosmeticRng {
    /// A generator from any seed (e.g. the clock: outcomes never depend
    /// on it).
    pub fn new(seed: u64) -> CosmeticRng {
        CosmeticRng(Rng::new(seed ^ 0x5DEE_CE66_D1CE_4E5B))
    }

    /// Uniform integer in `lo..=hi`.
    pub fn range(&mut self, lo: i32, hi: i32) -> i32 {
        self.0.range(lo, hi)
    }

    /// True with probability 1/n (always for n ≤ 1).
    pub fn one_in(&mut self, n: u8) -> bool {
        n <= 1 || self.0.range(1, n as i32) == 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_bounds_and_determinism() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..1000 {
            let x = a.range(1, 6);
            assert!((1..=6).contains(&x));
            assert_eq!(x, b.range(1, 6));
        }
    }
}
