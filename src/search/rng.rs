//! xorshift64* PRNG. Zero-dependency build (ADR-005), so we roll our own.
//! NOT cryptographic; only used for fuzz/differential testing and search.

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed | 1) // must be nonzero
    }

    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    /// Uniform-ish in [0, n) for n > 0.
    pub fn below(&mut self, n: u64) -> u64 {
        debug_assert!(n > 0);
        (self.next() % n) as u64
    }

    /// Uniform in [0, 1) (53-bit precision — same construction as rand's f64).
    pub fn next_f64(&mut self) -> f64 {
        ((self.next() >> 11) as f64) / ((1u64 << 53) as f64)
    }

    /// Random u64 with a heavy tail of "interesting" values:
    /// small ints, powers of two, all-ones patterns, and pure noise.
    pub fn interesting_u64(&mut self) -> u64 {
        match self.below(8) {
            0 => self.below(17),                     // [0, 16]
            1 => self.below(17).wrapping_neg(),       // [-16, 0]
            2 => 1u64 << self.below(64),             // powers of two
            3 => (1u64 << self.below(64)).wrapping_neg(),
            4 => 0xFFFF_FFFF_FFFF_FFFFu64 << self.below(64), // runs of ones
            5 => match self.below(4) {
                0 => 0,
                1 => 1,
                2 => 0x8000_0000_0000_0000,
                _ => 0x7FFF_FFFF_FFFF_FFFF,
            },
            6 => self.below(256) as u64,             // small values (shift counts, etc.)
            _ => self.next(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..1000 {
            assert_eq!(a.next(), b.next());
        }
    }

    #[test]
    fn nonzero_seed_enforced() {
        let mut r = Rng::new(0);
        assert_ne!(r.next(), 0); // sequence must not collapse
    }
}
