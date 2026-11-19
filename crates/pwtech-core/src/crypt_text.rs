//! Text encryption (`upstream/src/crypto/CryptText.*`, used by Tools |
//! Encrypt/Decrypt Clipboard and "Encrypt & Copy"). Format: see
//! `.ai/FORMATS.md` §2.

use zeroize::Zeroizing;

use crate::crypto::{Aes, HmacSha256, Sha256Ctx, align_to_block_size, base64, pbkdf2_256bit};
use crate::random::RandomGenerator;
use crate::text_file::{ansi_encode_lossy, ansi_encoding};

pub const CRYPTTEXT_VERSION: u8 = 3;
pub const CRYPTTEXT_MAXTEXTBYTES: usize = 134_217_728;
const MAGIC: [u8; 3] = *b"PWG";
const HMAC_LENGTH: usize = 32;
const HEADER_SIZE: usize = 8;
const BASE64_LINE_LENGTH: usize = 76;

/// Error codes (`CRYPTTEXT_ERROR_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CryptTextError {
    #[error("Clipboard error")]
    Clipboard,
    #[error("No text")]
    NoText,
    #[error("Text too long")]
    TextTooLong,
    #[error("Out of memory")]
    OutOfMemory,
    #[error("Text corrupted")]
    TextCorrupted,
    #[error("Bad key")]
    BadKey,
    #[error("Decompression failed")]
    DecomprFailed,
}

/// `initCrypto()`: AES-256 key and HMAC-SHA-256 keyed with the same derived
/// key. Version 0 uses the pre-PBKDF2 derivation.
fn init_crypto(passw: &[u8], salt: &[u8; 16], version: u8) -> (Aes, HmacSha256) {
    let derived: Zeroizing<[u8; 32]> = if version == 0 {
        let mut k = Zeroizing::new([0u8; 32]);
        k[..16].copy_from_slice(salt);
        for _ in 0..8192 {
            let mut h = Sha256Ctx::new();
            h.update(&k[..]);
            h.update(passw);
            *k = h.finish();
        }
        k
    } else {
        pbkdf2_256bit(passw, salt, 8192, None)
    };
    (Aes::new(&derived[..]), HmacSha256::new(&derived[..]))
}

/// `EncryptText()`: compress, authenticate and encrypt `text`; returns the
/// base64 text (CRLF line breaks every 76 characters). `passw` are the
/// password bytes (UTF-16LE in the original).
pub fn encrypt_text(text: &str, passw: &[u8], rng: &mut dyn RandomGenerator) -> Result<String, CryptTextError> {
    if text.is_empty() {
        return Err(CryptTextError::NoText);
    }
    let utf8 = text.as_bytes();
    // the original counts the terminating zero
    if utf8.len() + 1 > CRYPTTEXT_MAXTEXTBYTES {
        return Err(CryptTextError::TextTooLong);
    }
    let text_len = utf8.len();
    let buf_size = HEADER_SIZE + text_len + text_len / 16 + 64 + 3 + HMAC_LENGTH;
    let buf_size = 16 + align_to_block_size(buf_size, 16);
    let mut buf = Zeroizing::new(vec![0u8; buf_size]);

    rng.get_data(&mut buf[..16]);
    let mut pos = 16;
    buf[pos..pos + 3].copy_from_slice(&MAGIC);
    buf[pos + 3] = CRYPTTEXT_VERSION;
    buf[pos + 4..pos + 8].copy_from_slice(&(text_len as u32).to_le_bytes());
    pos += HEADER_SIZE;

    let compr = crate::lzo::compress(utf8);
    buf[pos..pos + compr.len()].copy_from_slice(&compr);
    pos += compr.len();

    let mut crypt_len = HEADER_SIZE + compr.len() + HMAC_LENGTH;
    let last_block = (crypt_len & 0x0f) as u8;
    buf[15] = (buf[15] & 0xf0) | last_block;
    if last_block != 0 {
        let start = 16 + crypt_len;
        let n = 16 - last_block as usize;
        rng.get_data(&mut buf[start..start + n]);
        crypt_len += n;
    }

    let mut iv: [u8; 16] = buf[..16].try_into().unwrap();
    let (aes, mut hmac) = init_crypto(passw, &iv, CRYPTTEXT_VERSION);
    hmac.update(&buf[16..16 + HEADER_SIZE + compr.len()]);
    let mac = hmac.finish();
    buf[pos..pos + HMAC_LENGTH].copy_from_slice(&mac);

    aes.cbc_encrypt(&mut iv, &mut buf[16..16 + crypt_len]);
    Ok(base64::encode(&buf[..16 + crypt_len], BASE64_LINE_LENGTH))
}

/// Result of a decryption: the text, and whether it came from an old version
/// that stored ANSI text.
pub struct DecryptedText {
    pub text: Zeroizing<String>,
}

/// `DecryptText()` for one format version (`0` = PWGen < 2.3.0, otherwise the
/// version determines only whether the text is UTF-8 (≥ 2) or ANSI).
/// `text` is the base64 text; each character is reduced to its low byte, as
/// the original does.
pub fn decrypt_text(text: &str, passw: &[u8], version: u8) -> Result<DecryptedText, CryptTextError> {
    if text.is_empty() {
        return Err(CryptTextError::NoText);
    }
    let ascii: Zeroizing<Vec<u8>> = Zeroizing::new(text.encode_utf16().map(|u| u as u8).collect());
    let min_len = if version == 0 { 48 } else { 64 };
    match base64::decoded_len_estimate(&ascii) {
        Ok(n) if n >= min_len => {}
        _ => return Err(CryptTextError::TextCorrupted),
    }
    let mut buf = Zeroizing::new(base64::decode(&ascii).map_err(|_| CryptTextError::TextCorrupted)?);
    let buf_size = buf.len();
    if buf_size & 0x0f != 0 {
        return Err(CryptTextError::TextCorrupted);
    }
    let hmac_len = if version == 0 { 16 } else { HMAC_LENGTH };
    let mut crypt_len = buf_size - 16;
    if crypt_len < 16 + hmac_len {
        return Err(CryptTextError::TextCorrupted);
    }
    let header_size = if version == 0 { 4 } else { HEADER_SIZE };
    let mut iv: [u8; 16] = buf[..16].try_into().unwrap();
    let (aes, mut hmac) = init_crypto(passw, &iv, version);
    let mut last_block = (buf[15] & 0x0f) as usize;
    if last_block == 0 {
        last_block = 16;
    }

    let text_bytes: u32;
    let enc_hmac: bool;
    if version == 0 {
        crypt_len -= hmac_len;
        enc_hmac = false;
        hmac.update(&buf[..buf_size - 16]);
        let mac = hmac.finish();
        if buf[16 + crypt_len..16 + crypt_len + hmac_len] != mac[..hmac_len] {
            return Err(CryptTextError::BadKey);
        }
        aes.cbc_decrypt(&mut iv, &mut buf[16..16 + crypt_len]);
        text_bytes = u32::from_le_bytes(buf[16..20].try_into().unwrap());
    } else {
        aes.cbc_decrypt(&mut iv, &mut buf[16..32]);
        if buf[16..19] != MAGIC {
            return Err(CryptTextError::BadKey);
        }
        let hdr_version = buf[19];
        text_bytes = u32::from_le_bytes(buf[20..24].try_into().unwrap());
        if text_bytes == 0 || text_bytes as usize > CRYPTTEXT_MAXTEXTBYTES {
            return Err(CryptTextError::BadKey);
        }
        enc_hmac = hdr_version >= 3;
        if !enc_hmac {
            crypt_len -= hmac_len;
        }
        if crypt_len > 16 {
            aes.cbc_decrypt(&mut iv, &mut buf[32..16 + crypt_len]);
        }
        if enc_hmac {
            crypt_len -= 16 - last_block;
        }
        let mac_data_len = if enc_hmac { crypt_len - hmac_len } else { crypt_len };
        hmac.update(&buf[16..16 + mac_data_len]);
        let mac = hmac.finish();
        let stored = if enc_hmac { 16 + crypt_len - hmac_len } else { 16 + crypt_len };
        if buf[stored..stored + hmac_len] != mac[..hmac_len] {
            return Err(CryptTextError::BadKey);
        }
    }

    let mut to_decompr = crypt_len as isize - header_size as isize;
    if enc_hmac {
        to_decompr -= hmac_len as isize;
    } else {
        to_decompr -= 16 - last_block as isize;
    }
    if to_decompr < 0 {
        return Err(CryptTextError::DecomprFailed);
    }
    let src = &buf[16 + header_size..16 + header_size + to_decompr as usize];
    let out = crate::lzo::decompress_safe(src, text_bytes as usize).ok_or(CryptTextError::DecomprFailed)?;
    // The output is treated as a zero-terminated string.
    let end = out.iter().position(|&b| b == 0).unwrap_or(out.len());
    let bytes = &out[..end];
    let text = if version >= 2 {
        Zeroizing::new(String::from_utf8_lossy(bytes).into_owned())
    } else {
        Zeroizing::new(ansi_encoding().decode_without_bom_handling(bytes).0.into_owned())
    };
    Ok(DecryptedText { text })
}

/// Decryption as done by the main window (`TMainForm::CryptText()`): the
/// password is passed as UTF-16LE (current versions); with `old_version`
/// ("PWGen <2.3.0" checked) versions 0, 1 (ANSI password) and 2 are tried.
pub fn decrypt_text_with_password(text: &str, passw: &str, old_version: bool) -> Result<DecryptedText, CryptTextError> {
    let utf16: Zeroizing<Vec<u8>> = Zeroizing::new(passw.encode_utf16().flat_map(u16::to_le_bytes).collect());
    let ansi = if old_version { Some(ansi_encode_lossy(passw)) } else { None };
    let mut version = if old_version { 0 } else { CRYPTTEXT_VERSION };
    loop {
        let pw: &[u8] = if version < 2 { ansi.as_deref().map(|v| &v[..]).unwrap_or(&[]) } else { &utf16 };
        let r = decrypt_text(text, pw, version);
        match r {
            Err(CryptTextError::TextCorrupted | CryptTextError::BadKey) => {
                version += 1;
                if version >= CRYPTTEXT_VERSION {
                    return r;
                }
            }
            _ => return r,
        }
    }
}

/// Encryption as done by the main window: password as UTF-16LE.
pub fn encrypt_text_with_password(text: &str, passw: &str, rng: &mut dyn RandomGenerator) -> Result<String, CryptTextError> {
    let utf16: Zeroizing<Vec<u8>> = Zeroizing::new(passw.encode_utf16().flat_map(u16::to_le_bytes).collect());
    encrypt_text(text, &utf16, rng)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::random::AesCtrPrng;

    #[test]
    fn roundtrip() {
        let mut rng = AesCtrPrng::from_seed(b"seed");
        for text in ["x", "Hello, world!", &"Пароль 😀 ".repeat(100)] {
            let enc = encrypt_text_with_password(text, "secret", &mut rng).unwrap();
            assert!(enc.split("\r\n").all(|l| l.len() <= 76));
            let dec = decrypt_text_with_password(&enc, "secret", false).unwrap();
            assert_eq!(dec.text.as_str(), text);
            assert_eq!(decrypt_text_with_password(&enc, "wrong", false).err(), Some(CryptTextError::BadKey));
        }
    }

    #[test]
    fn deterministic_output_layout() {
        let mut rng = AesCtrPrng::from_seed(b"seed");
        let enc = encrypt_text_with_password("abc", "pw", &mut rng).unwrap();
        let raw = base64::decode(enc.as_bytes()).unwrap();
        // IV + 8 header + 3+? compressed + 32 HMAC, padded to 16
        assert_eq!(raw.len() % 16, 0);
        let crypt_len = raw.len() - 16;
        let unpadded = raw[15] as usize & 0xf;
        assert!(unpadded == 0 || crypt_len % 16 == 0);
    }

    #[test]
    fn corrupted() {
        assert_eq!(decrypt_text_with_password("abc", "x", false).err(), Some(CryptTextError::TextCorrupted));
        assert_eq!(decrypt_text_with_password("", "x", false).err(), Some(CryptTextError::NoText));
    }
}
