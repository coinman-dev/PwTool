//! Logic of the MP (master password) password generator
//! (`upstream/src/main/MPPasswGen.cpp`, lines 286–470) and of the key used by
//! the deterministic random generator (`Main.cpp:4215`).

use zeroize::Zeroizing;

use crate::crypto::{base64, hmac_sha1, hmac_sha256, sha256};
use crate::random::{AesCtrPrng, RandomGenerator};

/// Salt string for deriving the key from the master password
/// (`MPPG_KEYGEN_SALTSTR`).
pub const MPPG_KEYGEN_SALTSTR: &str = "PWGen MP Password Generator";

pub const PASSW_MAX_CHARS: i32 = 10000;
pub const PASSW_DEFAULT_LEN: i32 = 16;

/// Character sets of the MP generator (`MPPG_CHARSETS`).
pub const MPPG_CHARSETS: [&str; 6] = [
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789",
    "ACEFHJKLMNPRTUVWXYabcdefghijkmnopqrstuvwxyz3479",
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~",
    "ACEFHJKLMNPRTUVWXYabcdefghijkmnopqrstuvwxyz3479!\"#$%&'()*+,-./:;<=>?@[\\]^_`{}~",
    "0123456789abcdef",
    "0123456789ABCDEF",
];

/// Format of the password checksum shown after entering the master password.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasswHashType {
    /// Two decimal digits.
    Dec99 = 0,
    /// Four decimal digits.
    Dec9999 = 1,
    /// Four hex digits.
    Hex16Bit = 2,
    /// Eight hex digits.
    Hex32Bit = 3,
}

impl PasswHashType {
    pub fn from_index(i: i32) -> Self {
        match i {
            0 => Self::Dec99,
            1 => Self::Dec9999,
            2 => Self::Hex16Bit,
            _ => Self::Hex32Bit,
        }
    }
}

fn utf16le(s: &str) -> Zeroizing<Vec<u8>> {
    Zeroizing::new(s.encode_utf16().flat_map(u16::to_le_bytes).collect())
}

/// `unicodeToLSByte()`: low byte of each UTF-16 code unit.
pub fn unicode_to_ls_byte(s: &str) -> Zeroizing<Vec<u8>> {
    Zeroizing::new(s.encode_utf16().map(|u| u as u8).collect())
}

/// Key derived from a master password: HMAC-SHA-256(key = password as
/// UTF-16LE, data = `MPPG_KEYGEN_SALTSTR`). Also the key of the
/// deterministic random generator of the main window.
pub fn derive_key(master_passw: &str) -> Zeroizing<[u8; 32]> {
    Zeroizing::new(hmac_sha256(&utf16le(master_passw), MPPG_KEYGEN_SALTSTR.as_bytes()))
}

/// Deterministic random generator as set up by Tools | Deterministic Random
/// Generator | Set up: `SeedWithKey(derive_key(password), empty)`.
pub fn deterministic_generator(master_passw: &str) -> AesCtrPrng {
    AesCtrPrng::from_key_and_param(&derive_key(master_passw)[..], &[])
}

/// Checksum of the master password ("Show checksum of password").
pub fn passw_checksum(master_passw: &str, hash_type: PasswHashType) -> String {
    let key = derive_key(master_passw);
    let hash = sha256(&key[..]);
    let n = if hash_type == PasswHashType::Hex16Bit { 2 } else { 4 };
    let mut v: u32 = 0;
    for &b in &hash[..n] {
        v = (v << 8) | b as u32;
    }
    match hash_type {
        PasswHashType::Dec99 => format!("{:02}", v % 100),
        PasswHashType::Dec9999 => format!("{:04}", v % 10000),
        PasswHashType::Hex16Bit => format!("{:04x}", v & 0xffff),
        PasswHashType::Hex32Bit => format!("{v:08x}"),
    }
}

/// The key kept by the MP generator window: the derived key, or for
/// "Hashapass" compatibility the low bytes of the password characters.
pub fn generator_key(master_passw: &str, hashapass: bool) -> Zeroizing<Vec<u8>> {
    if hashapass {
        unicode_to_ls_byte(master_passw)
    } else {
        Zeroizing::new(derive_key(master_passw).to_vec())
    }
}

/// Generate a password (`GenerateBtnClick`). Returns the password and its
/// entropy in bits (not capped).
pub fn generate(
    key: &[u8],
    param: &str,
    charset_index: usize,
    length: i32,
    add_length_to_param: bool,
    hashapass: bool,
) -> (Zeroizing<String>, f64) {
    if hashapass {
        let param_bytes = unicode_to_ls_byte(param);
        let mac = Zeroizing::new(hmac_sha1(key, &param_bytes));
        let p = base64::encode(&mac[..6], 0);
        let bits = 8.0 * 6.0;
        return (Zeroizing::new(p), bits);
    }
    let mut param_data = utf16le(param);
    if add_length_to_param {
        param_data.push(0);
        param_data.push((length & 0xff) as u8);
        param_data.push(((length >> 8) & 0xff) as u8);
    }
    let mut rng = AesCtrPrng::from_key_and_param(key, &param_data);
    let charset: Vec<char> = MPPG_CHARSETS[charset_index].chars().collect();
    let mut passw = Zeroizing::new(String::with_capacity(length.max(0) as usize));
    for _ in 0..length.max(0) {
        passw.push(charset[rng.get_num_range(charset.len() as u32) as usize]);
    }
    let bits = crate::math::delphi_log2(charset.len() as f64) * length as f64;
    (passw, bits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashapass_known_vector() {
        // Hashapass reference: password "master", parameter "example" ->
        // first 6 bytes of HMAC-SHA1 in base64 (8 characters).
        let key = generator_key("master", true);
        let (p, bits) = generate(&key, "example", 0, 16, false, true);
        assert_eq!(p.len(), 8);
        assert_eq!(bits, 48.0);
        let mac = hmac_sha1(b"master", b"example");
        assert_eq!(p.as_str(), base64::encode(&mac[..6], 0));
    }

    #[test]
    fn deterministic_and_parameter_dependent() {
        let key = generator_key("master", false);
        let (a, _) = generate(&key, "site", 0, 16, false, false);
        let (b, _) = generate(&key, "site", 0, 16, false, false);
        let (c, _) = generate(&key, "site", 0, 16, true, false);
        assert_eq!(a.as_str(), b.as_str());
        assert_ne!(a.as_str(), c.as_str());
        assert_eq!(a.len(), 16);
        assert_eq!(passw_checksum("master", PasswHashType::Hex32Bit).len(), 8);
        assert_eq!(passw_checksum("master", PasswHashType::Dec99).len(), 2);
    }
}
