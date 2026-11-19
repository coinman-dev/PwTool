//! Fast non-cryptographic generator (`Jsf32RandGen` in
//! `upstream/src/random/FastPRNG.*`), used to fill wiped memory with noise and
//! for non-security decisions. Global instance = `g_fastRandGen`.

use std::sync::{LazyLock, Mutex};

use super::RandomGenerator;
use super::hrtimer::{filetime_now, monotonic_ns};

/// Bob Jenkins' small fast generator, 32-bit variant.
#[derive(Clone)]
pub struct Jsf32 {
    a: u32,
    b: u32,
    c: u32,
    d: u32,
}

impl Jsf32 {
    /// New generator seeded from the clock (`Jsf32RandGen()` calls
    /// `Randomize()`).
    pub fn new() -> Self {
        let mut g = Jsf32 { a: 0, b: 0, c: 0, d: 0 };
        g.randomize();
        g
    }

    /// Generator with a fixed seed (for tests).
    pub fn with_seed(seed: &[u8]) -> Self {
        let mut g = Jsf32 { a: 0, b: 0, c: 0, d: 0 };
        g.seed(seed);
        g
    }

    #[inline]
    fn next(&mut self) -> u32 {
        let e = self.a.wrapping_sub(self.b.rotate_left(27));
        self.a = self.b ^ self.c.rotate_left(17);
        self.b = self.c.wrapping_add(self.d);
        self.c = self.d.wrapping_add(e);
        self.d = e.wrapping_add(self.a);
        self.d
    }
}

impl Default for Jsf32 {
    fn default() -> Self {
        Self::new()
    }
}

impl RandomGenerator for Jsf32 {
    fn seed(&mut self, seed: &[u8]) {
        self.a = 0xf1ea5eed;
        let mut s = [0u8; 12];
        let n = seed.len().min(12);
        s[..n].copy_from_slice(&seed[..n]);
        self.b = u32::from_le_bytes(s[0..4].try_into().unwrap());
        self.c = u32::from_le_bytes(s[4..8].try_into().unwrap());
        self.d = u32::from_le_bytes(s[8..12].try_into().unwrap());
        for _ in 0..20 {
            self.next();
        }
    }

    /// Seed = FILETIME (8 bytes) + low+high part of the performance counter.
    fn randomize(&mut self) {
        let mut seed = [0u8; 12];
        seed[..8].copy_from_slice(&filetime_now().to_le_bytes());
        let t = monotonic_ns();
        let qpc = (t as u32).wrapping_add((t >> 32) as u32);
        seed[8..].copy_from_slice(&qpc.to_le_bytes());
        self.seed(&seed);
    }

    fn get_word32(&mut self) -> u32 {
        self.next()
    }

    fn get_data(&mut self, buf: &mut [u8]) {
        let mut chunks = buf.chunks_exact_mut(4);
        for c in &mut chunks {
            c.copy_from_slice(&self.next().to_le_bytes());
        }
        let rest = chunks.into_remainder();
        if !rest.is_empty() {
            let r = self.next().to_le_bytes();
            let n = rest.len();
            rest.copy_from_slice(&r[..n]);
        }
    }
}

impl Drop for Jsf32 {
    fn drop(&mut self) {
        self.a = 0;
        self.b = 0;
        self.c = 0;
        self.d = 0;
    }
}

/// Global fast generator (`g_fastRandGen`).
pub static FAST_RAND: LazyLock<Mutex<Jsf32>> = LazyLock::new(|| Mutex::new(Jsf32::new()));

/// Fill a buffer with noise from the global fast generator.
pub fn fast_fill(buf: &mut [u8]) {
    FAST_RAND.lock().unwrap().get_data(buf);
}

/// `fprng_rand()`.
pub fn fprng_rand() -> u32 {
    FAST_RAND.lock().unwrap().get_word32()
}

/// `fprng_rand(n)`: value in `0..n` by modulo (slightly biased, as in the
/// original; only used for non-security purposes).
pub fn fprng_rand_range(n: u32) -> u32 {
    fprng_rand() % n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_with_seed() {
        let mut a = Jsf32::with_seed(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
        let mut b = Jsf32::with_seed(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
        assert_eq!(a.get_word32(), b.get_word32());
        let mut x = [0u8; 7];
        let mut y = [0u8; 7];
        a.get_data(&mut x);
        b.get_data(&mut y);
        assert_eq!(x, y);
    }
}
