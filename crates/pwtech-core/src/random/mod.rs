//! Random number generation (`upstream/src/random/`).
//!
//! The exact sequence of bytes consumed by [`RandomGenerator::get_num_range`]
//! and [`permute`] determines the output of the deterministic modes
//! (deterministic random generator, MP password generator), so these follow
//! `RandomGenerator.h` literally.

pub mod aes_ctr;
pub mod entropy;
pub mod fast;
pub mod hrtimer;
pub mod pool;

pub use aes_ctr::AesCtrPrng;
pub use entropy::{EntropyEventType, EntropyManager};
pub use fast::{Jsf32, fprng_rand, fprng_rand_range};
pub use pool::{PoolCipherType, RandomPool};

/// Common interface of all generators (`class RandomGenerator`).
pub trait RandomGenerator: Send {
    /// Seed the generator (`Seed()`).
    fn seed(&mut self, seed: &[u8]);

    /// Fill `buf` with random bytes (`GetData()`).
    fn get_data(&mut self, buf: &mut [u8]);

    /// Reseed from system parameters (`Randomize()`): by default the current
    /// time as `FILETIME`, as in `RandomGenerator.cpp`.
    fn randomize(&mut self) {
        self.seed(&hrtimer::filetime_now().to_le_bytes());
    }

    fn get_word64(&mut self) -> u64 {
        let mut b = [0u8; 8];
        self.get_data(&mut b);
        u64::from_le_bytes(b)
    }

    fn get_word32(&mut self) -> u32 {
        let mut b = [0u8; 4];
        self.get_data(&mut b);
        u32::from_le_bytes(b)
    }

    fn get_word16(&mut self) -> u16 {
        let mut b = [0u8; 2];
        self.get_data(&mut b);
        u16::from_le_bytes(b)
    }

    fn get_byte(&mut self) -> u8 {
        let mut b = [0u8; 1];
        self.get_data(&mut b);
        b[0]
    }

    /// Uniform value in `0..n` by rejection sampling (`GetNumRange(lNum)`).
    /// Reads 1, 2 or 4 bytes per attempt depending on `n`.
    ///
    /// # Panics
    /// If `n == 0` (the original throws `RandomGeneratorRangeError`).
    fn get_num_range(&mut self, n: u32) -> u32 {
        assert!(n != 0, "RandomGenerator::GetNumRange(): Invalid range");
        if n == 1 {
            return 0;
        }
        let r: u32;
        if n <= 256 {
            let max = n * (256 / n);
            loop {
                let v = self.get_byte() as u32;
                if v < max {
                    r = v;
                    break;
                }
            }
        } else if n <= 65536 {
            let max = n * (65536 / n);
            loop {
                let v = self.get_word16() as u32;
                if v < max {
                    r = v;
                    break;
                }
            }
        } else {
            let max = n as u64 * (0x1_0000_0000u64 / n as u64);
            loop {
                let v = self.get_word32();
                if (v as u64) < max {
                    r = v;
                    break;
                }
            }
        }
        if r < n { r } else { r % n }
    }

    /// Uniform value in `begin..end` (`GetNumRange(lBegin, lEnd)`).
    fn get_num_range_between(&mut self, begin: u32, end: u32) -> u32 {
        assert!(end > begin, "RandomGenerator::GetNumRange(): Invalid range");
        begin + self.get_num_range(end - begin)
    }
}

/// Fisher–Yates shuffle from the end of the slice (`Permute()`).
///
/// # Panics
/// If the slice is empty (the original throws `RandomGeneratorRangeError`).
pub fn permute<T, R: RandomGenerator + ?Sized>(rng: &mut R, items: &mut [T]) {
    assert!(!items.is_empty(), "RandomGenerator::Permute(): Invalid range");
    let mut i = items.len() - 1;
    while i > 0 {
        let j = rng.get_num_range(i as u32 + 1) as usize;
        if j != i {
            items.swap(i, j);
        }
        i -= 1;
    }
}

/// A generator that replays a fixed byte sequence; for tests.
#[cfg(test)]
pub(crate) struct ReplayRng(pub Vec<u8>, pub usize);

#[cfg(test)]
impl RandomGenerator for ReplayRng {
    fn seed(&mut self, _: &[u8]) {}
    fn get_data(&mut self, buf: &mut [u8]) {
        for b in buf {
            *b = self.0[self.1 % self.0.len()];
            self.1 += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn num_range_byte_rejection() {
        // n = 100: max = 200, so 250 and 200 are rejected, 199 -> 99.
        let mut r = ReplayRng(vec![250, 200, 199], 0);
        assert_eq!(r.get_num_range(100), 99);
        assert_eq!(r.1, 3);
    }

    #[test]
    fn num_range_word16_little_endian() {
        // n = 1000 reads 2 bytes LE: 0x01 0x02 -> 0x0201 = 513.
        let mut r = ReplayRng(vec![0x01, 0x02], 0);
        assert_eq!(r.get_num_range(1000), 513);
    }

    #[test]
    fn permute_order_of_draws() {
        // 3 elements: draws GetNumRange(3) then GetNumRange(2).
        let mut r = ReplayRng(vec![0, 0], 0);
        let mut v = [1, 2, 3];
        permute(&mut r, &mut v);
        // i=2, j=0 -> [3,2,1]; i=1, j=0 -> [2,3,1]
        assert_eq!(v, [2, 3, 1]);
    }
}
