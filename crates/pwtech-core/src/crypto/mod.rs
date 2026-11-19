//! Cryptographic primitives with the exact semantics used by the original.
//!
//! Counterparts: `upstream/src/crypto/` (PolarSSL AES/SHA, `chacha.c`,
//! `CryptUtil.cpp`, `SymmetricCipher.h`, `hmac.h`). Standard algorithms are
//! taken from RustCrypto crates; everything with original-specific behaviour
//! (ChaCha variant, CBC chaining state, PBKDF2 with cancel flag) is ported.

pub mod chacha;

use std::sync::atomic::{AtomicBool, Ordering};

use aes::cipher::{Array, BlockCipherDecrypt, BlockCipherEncrypt, KeyInit};
use hmac::{Hmac, Mac};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};
use zeroize::{Zeroize, Zeroizing};

use chacha::ChaCha;

/// SHA-256 of `data` (`sha256(…, 0)` in PolarSSL).
pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// SHA-512 of `data`.
pub fn sha512(data: &[u8]) -> [u8; 64] {
    Sha512::digest(data).into()
}

/// SHA-1 of `data`.
pub fn sha1(data: &[u8]) -> [u8; 20] {
    Sha1::digest(data).into()
}

/// Incremental SHA-256 (`sha256_starts/update/finish`).
#[derive(Clone, Default)]
pub struct Sha256Ctx(Sha256);

impl Sha256Ctx {
    pub fn new() -> Self {
        Self(Sha256::new())
    }
    pub fn update(&mut self, data: &[u8]) {
        Digest::update(&mut self.0, data);
    }
    pub fn finish(self) -> [u8; 32] {
        self.0.finalize().into()
    }
}

/// HMAC-SHA-256 (`sha256_hmac(key, data)`).
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut m = HmacSha256::new(key);
    m.update(data);
    m.finish()
}

/// HMAC-SHA-512.
pub fn hmac_sha512(key: &[u8], data: &[u8]) -> [u8; 64] {
    let mut m = HmacSha512::new(key);
    m.update(data);
    m.finish()
}

/// HMAC-SHA-1 (`sha1_hmac`).
pub fn hmac_sha1(key: &[u8], data: &[u8]) -> [u8; 20] {
    let mut m = <Hmac<Sha1> as KeyInit>::new_from_slice(key).expect("HMAC accepts any key size");
    Mac::update(&mut m, data);
    m.finalize().into_bytes().into()
}

/// Incremental HMAC-SHA-256 that can be reset to its keyed initial state
/// (`sha256_hmac_starts/update/finish/reset`).
#[derive(Clone)]
pub struct HmacSha256 {
    initial: Hmac<Sha256>,
    cur: Hmac<Sha256>,
}

impl HmacSha256 {
    pub fn new(key: &[u8]) -> Self {
        let m = <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("HMAC accepts any key size");
        Self { initial: m.clone(), cur: m }
    }
    pub fn update(&mut self, data: &[u8]) {
        Mac::update(&mut self.cur, data);
    }
    /// Return the MAC and reset to the keyed initial state.
    pub fn finish(&mut self) -> [u8; 32] {
        let m = std::mem::replace(&mut self.cur, self.initial.clone());
        m.finalize().into_bytes().into()
    }
    pub fn reset(&mut self) {
        self.cur = self.initial.clone();
    }
}

/// Incremental HMAC-SHA-512.
#[derive(Clone)]
pub struct HmacSha512 {
    cur: Hmac<Sha512>,
}

impl HmacSha512 {
    pub fn new(key: &[u8]) -> Self {
        Self {
            cur: <Hmac<Sha512> as KeyInit>::new_from_slice(key).expect("HMAC accepts any key size"),
        }
    }
    pub fn update(&mut self, data: &[u8]) {
        Mac::update(&mut self.cur, data);
    }
    pub fn finish(self) -> [u8; 64] {
        self.cur.finalize().into_bytes().into()
    }
}

/// HMAC of either kind, selected at run time (database hash type).
pub enum HmacAny {
    Sha256(HmacSha256),
    Sha512(HmacSha512),
}

impl HmacAny {
    pub fn len(&self) -> usize {
        match self {
            HmacAny::Sha256(_) => 32,
            HmacAny::Sha512(_) => 64,
        }
    }
    pub fn update(&mut self, data: &[u8]) {
        match self {
            HmacAny::Sha256(m) => m.update(data),
            HmacAny::Sha512(m) => m.update(data),
        }
    }
    pub fn finish(self) -> Zeroizing<Vec<u8>> {
        Zeroizing::new(match self {
            HmacAny::Sha256(mut m) => m.finish().to_vec(),
            HmacAny::Sha512(m) => m.finish().to_vec(),
        })
    }
}

/// Number of PBKDF2 iterations used when the original calls `pbkdf2_256bit`
/// without an explicit count.
pub const PBKDF2_DEFAULT_ITERATIONS: u32 = 8192;

/// PBKDF2 with HMAC-SHA-256 as PRF, producing exactly one 256-bit block
/// (`pbkdf2_256bit()` in `CryptUtil.cpp`). If `cancel` becomes `true` the loop
/// stops early and the (then useless) partial result is returned; callers must
/// check the flag, as the original does.
pub fn pbkdf2_256bit(
    passw: &[u8],
    salt: &[u8],
    iterations: u32,
    cancel: Option<&AtomicBool>,
) -> Zeroizing<[u8; 32]> {
    let mut prf = HmacSha256::new(passw);
    prf.update(salt);
    prf.update(&[0, 0, 0, 1]);
    let mut result = Zeroizing::new(prf.finish());
    let mut md = Zeroizing::new(*result);
    let mut i = 1;
    while i < iterations && !cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
        prf.update(&*md);
        *md = prf.finish();
        for (r, m) in result.iter_mut().zip(md.iter()) {
            *r ^= m;
        }
        i += 1;
    }
    result
}

/// Round `len` up to a multiple of `block_size` (`alignToBlockSize`).
pub fn align_to_block_size(len: usize, block_size: usize) -> usize {
    let rest = len % block_size;
    if rest != 0 { len + block_size - rest } else { len }
}

/// Increment a big-endian counter of any length by one
/// (`incrementCounter<Nbits>`).
pub fn increment_counter(counter: &mut [u8]) {
    for b in counter.iter_mut().rev() {
        *b = b.wrapping_add(1);
        if *b != 0 {
            break;
        }
    }
}

/// AES block cipher context with PolarSSL-style operations.
#[derive(Clone)]
pub enum Aes {
    Aes128(aes::Aes128),
    Aes192(aes::Aes192),
    Aes256(aes::Aes256),
}

impl Aes {
    /// Key of 16, 24 or 32 bytes.
    pub fn new(key: &[u8]) -> Self {
        match key.len() {
            16 => Aes::Aes128(aes::Aes128::new_from_slice(key).unwrap()),
            24 => Aes::Aes192(aes::Aes192::new_from_slice(key).unwrap()),
            32 => Aes::Aes256(aes::Aes256::new_from_slice(key).unwrap()),
            n => panic!("Invalid AES key size {n}"),
        }
    }

    /// `aes_crypt_ecb(AES_ENCRYPT)` for one 16-byte block.
    pub fn encrypt_block(&self, input: &[u8; 16]) -> [u8; 16] {
        let mut b = Array::from(*input);
        match self {
            Aes::Aes128(c) => c.encrypt_block(&mut b),
            Aes::Aes192(c) => c.encrypt_block(&mut b),
            Aes::Aes256(c) => c.encrypt_block(&mut b),
        }
        b.into()
    }

    /// `aes_crypt_ecb(AES_DECRYPT)` for one 16-byte block.
    pub fn decrypt_block(&self, input: &[u8; 16]) -> [u8; 16] {
        let mut b = Array::from(*input);
        match self {
            Aes::Aes128(c) => c.decrypt_block(&mut b),
            Aes::Aes192(c) => c.decrypt_block(&mut b),
            Aes::Aes256(c) => c.decrypt_block(&mut b),
        }
        b.into()
    }

    /// `aes_crypt_cbc(AES_ENCRYPT)` in place. `buf.len()` must be a multiple of
    /// 16; `iv` is updated so that a following call continues the chain.
    pub fn cbc_encrypt(&self, iv: &mut [u8; 16], buf: &mut [u8]) {
        assert!(buf.len() % 16 == 0, "CBC length must be a multiple of 16");
        for block in buf.chunks_exact_mut(16) {
            let mut x = [0u8; 16];
            for k in 0..16 {
                x[k] = block[k] ^ iv[k];
            }
            let out = self.encrypt_block(&x);
            block.copy_from_slice(&out);
            *iv = out;
            x.zeroize();
        }
    }

    /// `aes_crypt_cbc(AES_DECRYPT)` in place; `iv` is updated.
    pub fn cbc_decrypt(&self, iv: &mut [u8; 16], buf: &mut [u8]) {
        assert!(buf.len() % 16 == 0, "CBC length must be a multiple of 16");
        for block in buf.chunks_exact_mut(16) {
            let ct: [u8; 16] = block.try_into().unwrap();
            let mut pt = self.decrypt_block(&ct);
            for k in 0..16 {
                block[k] = pt[k] ^ iv[k];
            }
            *iv = ct;
            pt.zeroize();
        }
    }
}

/// Direction of a [`SymmetricCipher`] (`EncryptionAlgorithm::Mode`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Encrypt,
    Decrypt,
}

/// Database ciphers with the interface of `SymmetricCipher.h`.
pub enum SymmetricCipher {
    /// AES in CBC mode; IV state carried between calls.
    AesCbc { aes: Aes, iv: [u8; 16] },
    /// Bernstein ChaCha20 with 64-bit IV.
    ChaCha20(ChaCha),
}

impl SymmetricCipher {
    pub fn aes_cbc(key: &[u8]) -> Self {
        SymmetricCipher::AesCbc { aes: Aes::new(key), iv: [0; 16] }
    }

    pub fn chacha20(key: &[u8]) -> Self {
        assert!(key.len() == 16 || key.len() == 32, "Invalid ChaCha20 key size");
        SymmetricCipher::ChaCha20(ChaCha::new(key))
    }

    /// Block size: 16 for AES, 64 for ChaCha20 (a call ending mid-block loses
    /// the rest of the keystream block).
    pub fn block_size(&self) -> usize {
        match self {
            SymmetricCipher::AesCbc { .. } => 16,
            SymmetricCipher::ChaCha20(_) => 64,
        }
    }

    pub fn iv_size(&self) -> usize {
        match self {
            SymmetricCipher::AesCbc { .. } => 16,
            SymmetricCipher::ChaCha20(_) => 8,
        }
    }

    pub fn requires_alignment(&self) -> bool {
        matches!(self, SymmetricCipher::AesCbc { .. })
    }

    /// `SetIV()`: reads `iv_size()` bytes from `iv`.
    pub fn set_iv(&mut self, iv: &[u8]) {
        match self {
            SymmetricCipher::AesCbc { iv: state, .. } => state.copy_from_slice(&iv[..16]),
            SymmetricCipher::ChaCha20(c) => c.iv_setup(&iv[..8], None),
        }
    }

    pub fn encrypt(&mut self, buf: &mut [u8]) {
        match self {
            SymmetricCipher::AesCbc { aes, iv } => aes.cbc_encrypt(iv, buf),
            SymmetricCipher::ChaCha20(c) => c.apply_keystream(buf),
        }
    }

    pub fn decrypt(&mut self, buf: &mut [u8]) {
        match self {
            SymmetricCipher::AesCbc { aes, iv } => aes.cbc_decrypt(iv, buf),
            SymmetricCipher::ChaCha20(c) => c.apply_keystream(buf),
        }
    }
}

impl Drop for SymmetricCipher {
    fn drop(&mut self) {
        if let SymmetricCipher::AesCbc { iv, .. } = self {
            iv.zeroize();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(s: &str) -> Vec<u8> {
        hex::decode(s).unwrap()
    }

    #[test]
    fn pbkdf2_matches_rfc7914_vector() {
        // RFC 7914 §11: PBKDF2-HMAC-SHA256, P="passwd", S="salt", c=1, first 32 bytes.
        let k = pbkdf2_256bit(b"passwd", b"salt", 1, None);
        assert_eq!(
            hex::encode(*k),
            "55ac046e56e3089fec1691c22544b605f94185216dde0465e68b9d57c20dacbc"
        );
        // RFC 7914: P="Password", S="NaCl", c=80000, first 32 bytes.
        let k = pbkdf2_256bit(b"Password", b"NaCl", 80000, None);
        assert_eq!(
            hex::encode(*k),
            "4ddcd8f60b98be21830cee5ef22701f9641a4418d04c0414aeff08876b34ab56"
        );
    }

    #[test]
    fn pbkdf2_cancel() {
        let flag = AtomicBool::new(true);
        let k = pbkdf2_256bit(b"a", b"b", 1_000_000, Some(&flag));
        assert_eq!(*k, *pbkdf2_256bit(b"a", b"b", 1, None));
    }

    #[test]
    fn aes_cbc_nist_vector() {
        // NIST SP 800-38A F.2.5 CBC-AES256.Encrypt, first two blocks.
        let key = h("603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4");
        let mut iv: [u8; 16] = h("000102030405060708090a0b0c0d0e0f").try_into().unwrap();
        let mut buf = h("6bc1bee22e409f96e93d7e117393172aae2d8a571e03ac9c9eb76fac45af8e51");
        let aes = Aes::new(&key);
        aes.cbc_encrypt(&mut iv, &mut buf[..16]);
        aes.cbc_encrypt(&mut iv, &mut buf[16..]); // chaining across calls
        assert_eq!(
            hex::encode(&buf),
            "f58c4c04d6e5f1ba779eabfb5f7bfbd69cfc4e967edb808d679f777bc6702c7d"
        );
        let mut iv: [u8; 16] = h("000102030405060708090a0b0c0d0e0f").try_into().unwrap();
        aes.cbc_decrypt(&mut iv, &mut buf);
        assert_eq!(
            hex::encode(&buf),
            "6bc1bee22e409f96e93d7e117393172aae2d8a571e03ac9c9eb76fac45af8e51"
        );
    }

    #[test]
    fn hmac_rfc4231_case2() {
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            hex::encode(mac),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        let mut inc = HmacSha256::new(b"Jefe");
        inc.update(b"what do ya want ");
        inc.update(b"for nothing?");
        assert_eq!(inc.finish(), mac);
        inc.update(b"what do ya want for nothing?");
        assert_eq!(inc.finish(), mac, "finish() must reset to the keyed state");
    }

    #[test]
    fn counter_increment() {
        let mut c = [0u8, 0xff, 0xff];
        increment_counter(&mut c);
        assert_eq!(c, [1, 0, 0]);
    }
}
