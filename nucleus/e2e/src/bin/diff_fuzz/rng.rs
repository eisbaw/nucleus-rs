//! Seeded splitmix64 RNG — tiny, dependency-free, deterministic.
//!
//! Same seed => same stream => same generated programs => same result.
//! No wall-clock or unseeded randomness enters program generation, which
//! is what makes a failing seed an exact reproducer (the determinism-in-
//! seed property the differential harness relies on; see the binary
//! crate docstring in `main.rs`).

/// Seeded splitmix64 stream.
pub(crate) struct Rng {
    pub(crate) state: u64,
}

impl Rng {
    pub(crate) fn new(seed: u64) -> Self {
        Rng { state: seed }
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        // splitmix64 (public-domain reference constants).
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Approximately uniform in `[lo, hi]` inclusive. Requires `lo <= hi`.
    /// NOTE: modulo reduction is biased for spans that do not divide 2^64;
    /// for the tiny spans used here (single digits .. low hundreds) the
    /// bias is negligible and does not affect the differential property.
    /// Not a CSPRNG.
    pub(crate) fn range(&mut self, lo: u64, hi: u64) -> u64 {
        debug_assert!(lo <= hi);
        let span = hi - lo + 1;
        lo + (self.next_u64() % span)
    }

    pub(crate) fn i32_value(&mut self) -> i32 {
        self.next_u64() as i32
    }

    pub(crate) fn choice<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.range(0, (items.len() - 1) as u64) as usize]
    }

    /// A `bool` with the given numerator/denominator probability. Used to
    /// bias structural choices (e.g. "~1 in 3 affine ops").
    pub(crate) fn chance(&mut self, num: u64, den: u64) -> bool {
        debug_assert!(num <= den && den >= 1);
        self.range(0, den - 1) < num
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic_for_a_seed() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn range_stays_in_bounds() {
        let mut r = Rng::new(99);
        for _ in 0..1000 {
            let v = r.range(3, 9);
            assert!((3..=9).contains(&v));
        }
    }

    #[test]
    fn chance_extremes_are_total() {
        let mut r = Rng::new(7);
        for _ in 0..50 {
            assert!(r.chance(1, 1)); // always
            assert!(!r.chance(0, 4)); // never
        }
    }
}
