//! The random number generator, held in the world's state so a snapshot
//! carries it and a replay draws the same numbers. SplitMix64 (Steele, Lea
//! and Flood, "Fast splittable pseudorandom number generators", OOPSLA
//! 2014): one 64-bit word of state, integer arithmetic only.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rng {
    pub state: u64,
}

impl Rng {
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `0..n`, or 0 when `n` is 0.
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next_u64() % n }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_numbers() {
        let (mut a, mut b) = (Rng::new(7), Rng::new(7));
        let xs: Vec<u64> = (0..4).map(|_| a.next_u64()).collect();
        let ys: Vec<u64> = (0..4).map(|_| b.next_u64()).collect();
        assert_eq!(xs, ys);
        // The reference sequence for seed 0 (the paper's C code).
        assert_eq!(Rng::new(0).next_u64(), 0xE220_A839_7B1D_CDAF);
        assert!(Rng::new(1).below(10) < 10);
    }
}
