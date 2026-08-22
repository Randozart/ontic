//! Deterministic PRNG (xorshift64*) for reproducible probe generation.

/// Seedable xorshift64* generator. Same seed + same call sequence ⇒ same
/// probes, so sieve verdicts are reproducible across runs.
pub struct Rng {
    state: u64,
}

impl Rng {
    /// Create a generator from an arbitrary nonzero seed.
    pub fn new(seed: u64) -> Self {
        let s = if seed == 0 { 0x9E3779B97F4A7C15 } else { seed };
        Rng { state: s }
    }

    /// Next raw 64-bit value (xorshift64* step).
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    /// Uniform integer in [lo, hi] inclusive. Panics if lo > hi.
    pub fn range_i64(&mut self, lo: i64, hi: i64) -> i64 {
        assert!(lo <= hi, "rng range inverted");
        let span = (hi as i128 - lo as i128 + 1) as u64;
        // Modulo bias is negligible for probe domains; determinism matters more.
        let r = self.next_u64() % span;
        lo + r as i64
    }

    /// Uniform integer in [0, hi) exclusive.
    pub fn below(&mut self, hi: usize) -> usize {
        assert!(hi > 0, "rng below(0)");
        (self.next_u64() % hi as u64) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deterministic_sequence() {
        let mut a = Rng::new(0x5EED);
        let mut b = Rng::new(0x5EED);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn test_range_bounds_respected() {
        let mut r = Rng::new(42);
        for _ in 0..1000 {
            let v = r.range_i64(-3, 3);
            assert!((-3..=3).contains(&v));
        }
    }
}
