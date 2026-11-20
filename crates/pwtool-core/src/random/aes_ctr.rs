//! Deterministic generator based on AES-256 in counter mode
//! (`upstream/src/random/AESCtrPRNG.*`). Its output must match the original
//! bit for bit; see `.ai/FORMATS.md` §5.

use zeroize::{Zeroize, Zeroizing};

use super::RandomGenerator;
use crate::crypto::{Aes, PBKDF2_DEFAULT_ITERATIONS, increment_counter, pbkdf2_256bit, sha256};

pub const BLOCK_SIZE: usize = 16;
pub const KEY_SIZE: usize = 32;
const GETBUF_SIZE: usize = 64;
const MAX_BLOCKS: u32 = 65536;

pub struct AesCtrPrng {
    initial_key: Zeroizing<[u8; KEY_SIZE]>,
    cipher: Aes,
    counter: [u8; BLOCK_SIZE],
    get_buf: [u8; GETBUF_SIZE],
    get_buf_pos: usize,
    num_blocks: u32,
}

impl AesCtrPrng {
    /// Generator seeded with `Seed(seed)`: key = SHA-256(seed).
    pub fn from_seed(seed: &[u8]) -> Self {
        Self::from_key(Zeroizing::new(sha256(seed)))
    }

    /// Generator seeded with `SeedWithKey(key, param)`:
    /// key = PBKDF2-HMAC-SHA256(key, salt = param, 8192 iterations).
    pub fn from_key_and_param(key: &[u8], param: &[u8]) -> Self {
        Self::from_key(pbkdf2_256bit(key, param, PBKDF2_DEFAULT_ITERATIONS, None))
    }

    fn from_key(key: Zeroizing<[u8; KEY_SIZE]>) -> Self {
        let cipher = Aes::new(&*key);
        let mut g = AesCtrPrng {
            initial_key: key,
            cipher,
            counter: [0; BLOCK_SIZE],
            get_buf: [0; GETBUF_SIZE],
            get_buf_pos: GETBUF_SIZE,
            num_blocks: 0,
        };
        g.reset();
        g
    }

    /// `SeedWithKey()` on an existing instance.
    pub fn seed_with_key(&mut self, key: &[u8], param: &[u8]) {
        self.initial_key = pbkdf2_256bit(key, param, PBKDF2_DEFAULT_ITERATIONS, None);
        self.reset();
    }

    /// Return to the state right after the last seeding (`Reset()`).
    pub fn reset(&mut self) {
        self.cipher = Aes::new(&*self.initial_key);
        self.counter.zeroize();
        self.get_buf_pos = GETBUF_SIZE;
        self.num_blocks = 0;
    }

    fn fill_get_buf(&mut self) {
        for i in (0..GETBUF_SIZE).step_by(BLOCK_SIZE) {
            let out = self.cipher.encrypt_block(&self.counter);
            self.get_buf[i..i + BLOCK_SIZE].copy_from_slice(&out);
            increment_counter(&mut self.counter);
            self.num_blocks += 1;
        }
        if self.num_blocks >= MAX_BLOCKS {
            let mut new_key = Zeroizing::new([0u8; KEY_SIZE]);
            for i in (0..KEY_SIZE).step_by(BLOCK_SIZE) {
                let out = self.cipher.encrypt_block(&self.counter);
                new_key[i..i + BLOCK_SIZE].copy_from_slice(&out);
                increment_counter(&mut self.counter);
            }
            self.cipher = Aes::new(&*new_key);
            self.num_blocks = 0;
        }
        self.get_buf_pos = 0;
    }
}

impl RandomGenerator for AesCtrPrng {
    fn seed(&mut self, seed: &[u8]) {
        *self.initial_key = sha256(seed);
        self.reset();
    }

    fn get_data(&mut self, buf: &mut [u8]) {
        let mut pos = 0;
        while pos < buf.len() {
            if self.get_buf_pos == GETBUF_SIZE {
                self.fill_get_buf();
            }
            let n = (buf.len() - pos).min(GETBUF_SIZE - self.get_buf_pos);
            buf[pos..pos + n].copy_from_slice(&self.get_buf[self.get_buf_pos..self.get_buf_pos + n]);
            pos += n;
            self.get_buf_pos += n;
        }
    }

    fn get_byte(&mut self) -> u8 {
        if self.get_buf_pos == GETBUF_SIZE {
            self.fill_get_buf();
        }
        let b = self.get_buf[self.get_buf_pos];
        self.get_buf_pos += 1;
        b
    }
}

impl Drop for AesCtrPrng {
    fn drop(&mut self) {
        self.counter.zeroize();
        self.get_buf.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_block_is_aes_of_zero_counter() {
        let mut g = AesCtrPrng::from_seed(b"abc");
        let key = sha256(b"abc");
        let expected = Aes::new(&key).encrypt_block(&[0; 16]);
        let mut out = [0u8; 16];
        g.get_data(&mut out);
        assert_eq!(out, expected);
        // Second block uses counter 1 (big-endian increment of the last byte).
        let mut ctr = [0u8; 16];
        ctr[15] = 1;
        let expected2 = Aes::new(&key).encrypt_block(&ctr);
        g.get_data(&mut out);
        assert_eq!(out, expected2);
    }

    #[test]
    fn reset_restarts_sequence() {
        let mut g = AesCtrPrng::from_key_and_param(b"key", b"");
        let mut a = [0u8; 100];
        g.get_data(&mut a);
        g.reset();
        let mut b = [0u8; 100];
        g.get_data(&mut b);
        assert_eq!(a, b);
    }

    #[test]
    fn rekey_after_max_blocks() {
        // After 65536 blocks (1 MiB) the key changes; the stream must stay
        // consistent between byte-wise and bulk reads.
        let mut g1 = AesCtrPrng::from_seed(b"x");
        let mut g2 = AesCtrPrng::from_seed(b"x");
        let mut bulk = vec![0u8; (MAX_BLOCKS as usize + 8) * BLOCK_SIZE];
        g1.get_data(&mut bulk);
        let tail_start = bulk.len() - 256;
        let mut skip = vec![0u8; tail_start];
        g2.get_data(&mut skip);
        let tail: Vec<u8> = (0..256).map(|_| g2.get_byte()).collect();
        assert_eq!(&bulk[tail_start..], &tail[..]);
    }
}
