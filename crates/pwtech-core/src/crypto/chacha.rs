//! ChaCha stream cipher — exact port of `upstream/src/crypto/chacha.c`
//! (D. J. Bernstein's reference implementation, modified by C. Thöing).
//!
//! Properties that matter for compatibility with the original:
//! - 64-bit nonce ("IV") in state words 14–15, 64-bit block counter in words
//!   12–13 (original Bernstein variant, *not* the IETF variant of RFC 8439);
//! - configurable number of rounds (default 20, minimum 6);
//! - every call consumes whole 64-byte blocks: when a call ends in the middle of
//!   a block, the rest of that block's keystream is discarded and the next call
//!   starts with a fresh block. Callers that need a continuous stream must call
//!   with multiples of 64 bytes (the database code does exactly that).

use zeroize::{Zeroize, ZeroizeOnDrop};

const SIGMA: &[u8; 16] = b"expand 32-byte k";
const TAU: &[u8; 16] = b"expand 16-byte k";
const DEFAULT_NROUNDS: u32 = 20;

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct ChaCha {
    input: [u32; 16],
    nrounds: u32,
}

#[inline(always)]
fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

#[inline(always)]
fn quarter_round(x: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    x[a] = x[a].wrapping_add(x[b]);
    x[d] = (x[d] ^ x[a]).rotate_left(16);
    x[c] = x[c].wrapping_add(x[d]);
    x[b] = (x[b] ^ x[c]).rotate_left(12);
    x[a] = x[a].wrapping_add(x[b]);
    x[d] = (x[d] ^ x[a]).rotate_left(8);
    x[c] = x[c].wrapping_add(x[d]);
    x[b] = (x[b] ^ x[c]).rotate_left(7);
}

impl ChaCha {
    /// `chacha_keysetup()`: key of 32 bytes (256 bit) or 16 bytes (128 bit).
    /// Counter and IV are set to zero, number of rounds to 20.
    pub fn new(key: &[u8]) -> Self {
        let mut ctx = ChaCha { input: [0; 16], nrounds: DEFAULT_NROUNDS };
        ctx.key_setup(key);
        ctx
    }

    /// `chacha_keysetup()`.
    pub fn key_setup(&mut self, key: &[u8]) {
        let (k2, constants): (&[u8], &[u8; 16]) = match key.len() {
            32 => (&key[16..32], SIGMA),
            16 => (&key[0..16], TAU),
            n => panic!("ChaCha: invalid key length {n}"),
        };
        for i in 0..4 {
            self.input[4 + i] = le32(&key[4 * i..]);
            self.input[8 + i] = le32(&k2[4 * i..]);
            self.input[i] = le32(&constants[4 * i..]);
        }
        self.input[12] = 0;
        self.input[13] = 0;
        self.input[14] = 0;
        self.input[15] = 0;
        self.nrounds = DEFAULT_NROUNDS;
    }

    /// `chacha_nrounds()`: values below 6 are raised to 6.
    pub fn set_rounds(&mut self, nrounds: u32) {
        self.nrounds = nrounds.max(6);
    }

    /// `chacha_ivsetup()`: 8-byte IV; optional 8-byte initial block counter
    /// (zero if `None`).
    pub fn iv_setup(&mut self, iv: &[u8], ctr: Option<&[u8]>) {
        self.input[12] = ctr.map_or(0, le32);
        self.input[13] = ctr.map_or(0, |c| le32(&c[4..]));
        self.input[14] = le32(iv);
        self.input[15] = le32(&iv[4..]);
    }

    fn block(&self) -> [u8; 64] {
        let mut x = self.input;
        let mut i = self.nrounds as i32;
        while i > 0 {
            quarter_round(&mut x, 0, 4, 8, 12);
            quarter_round(&mut x, 1, 5, 9, 13);
            quarter_round(&mut x, 2, 6, 10, 14);
            quarter_round(&mut x, 3, 7, 11, 15);
            quarter_round(&mut x, 0, 5, 10, 15);
            quarter_round(&mut x, 1, 6, 11, 12);
            quarter_round(&mut x, 2, 7, 8, 13);
            quarter_round(&mut x, 3, 4, 9, 14);
            i -= 2;
        }
        let mut out = [0u8; 64];
        for (k, word) in x.iter().enumerate() {
            let v = word.wrapping_add(self.input[k]);
            out[4 * k..4 * k + 4].copy_from_slice(&v.to_le_bytes());
        }
        x.zeroize();
        out
    }

    fn advance(&mut self) {
        self.input[12] = self.input[12].wrapping_add(1);
        if self.input[12] == 0 {
            self.input[13] = self.input[13].wrapping_add(1);
        }
    }

    /// `chacha_encrypt_bytes()` with separate input and output buffers of equal
    /// length. Decryption is the same operation.
    pub fn encrypt_bytes(&mut self, input: &[u8], output: &mut [u8]) {
        assert_eq!(input.len(), output.len());
        output.copy_from_slice(input);
        self.apply_keystream(output);
    }

    /// In-place `chacha_encrypt_bytes()`.
    pub fn apply_keystream(&mut self, buf: &mut [u8]) {
        for chunk in buf.chunks_mut(64) {
            let mut ks = self.block();
            self.advance();
            for (b, k) in chunk.iter_mut().zip(ks.iter()) {
                *b ^= k;
            }
            ks.zeroize();
        }
    }

    /// `chacha_keystream_bytes()`: write raw keystream.
    pub fn keystream_bytes(&mut self, out: &mut [u8]) {
        for chunk in out.chunks_mut(64) {
            let mut ks = self.block();
            self.advance();
            chunk.copy_from_slice(&ks[..chunk.len()]);
            ks.zeroize();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test vectors from chacha_self_test() in the original (RFC 7539 key
    // stream blocks, nonce = 8-byte counter || 8-byte IV).
    #[test]
    fn self_test_vectors() {
        let keys: [[u8; 32]; 3] = [
            [0; 32],
            {
                let mut k = [0; 32];
                k[31] = 1;
                k
            },
            {
                let mut k = [0; 32];
                k[1] = 0xff;
                k
            },
        ];
        let nonces: [[u8; 16]; 3] = [[0; 16], {
            let mut n = [0; 16];
            n[0] = 1;
            n
        }, {
            let mut n = [0; 16];
            n[0] = 2;
            n
        }];
        let expected = [
            "76b8e0ada0f13d90405d6ae55386bd28bdd219b8a08ded1aa836efcc8b770dc7da41597c5157488d7724e03fb8d84a376a43b8f41518a11cc387b669b2ee6586",
            "3aeb5224ecf849929b9d828db1ced4dd832025e8018b8160b82284f3c949aa5a8eca00bbb4a73bdad192b5c42f73f2fd4e273644c8b36125a64addeb006c13a0",
            "72d54dfbf12ec44b362692df94137f328fea8da73990265ec1bbbea1ae9af0ca13b25aa26cb4a648cb9b9d1be65b2c0924a66c54d545ec1b7374f4872e99f096",
        ];
        for i in 0..3 {
            let mut ctx = ChaCha::new(&keys[i]);
            ctx.iv_setup(&nonces[i][8..], Some(&nonces[i][..8]));
            let mut ks = [0u8; 64];
            ctx.keystream_bytes(&mut ks);
            assert_eq!(hex::encode(ks), expected[i], "vector {i}");
        }
    }

    #[test]
    fn partial_block_discards_rest() {
        let key = [3u8; 32];
        let mut a = ChaCha::new(&key);
        let mut first = [0u8; 10];
        let mut second = [0u8; 10];
        a.keystream_bytes(&mut first);
        a.keystream_bytes(&mut second);
        let mut b = ChaCha::new(&key);
        let mut full = [0u8; 128];
        b.keystream_bytes(&mut full);
        assert_eq!(first, full[..10]);
        assert_eq!(second, full[64..74]);
    }
}
