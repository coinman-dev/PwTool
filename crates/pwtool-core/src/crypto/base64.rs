//! Base64 exactly as in `upstream/src/crypto/polarssl/base64.c` (PolarSSL,
//! extended by C. Thöing with optional line breaks).
//!
//! Encoder: CRLF is inserted *before* a 4-character group once the current
//! line holds `line_len` or more characters. Decoder: spaces are only allowed
//! right before a line end or at the end; at most two `=`, and no data after
//! them; an incomplete last group is ignored.

const ENC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn dec_map(c: u8) -> u8 {
    match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        b'=' => 64,
        _ => 127,
    }
}

/// Encode; `line_len = 0` disables line breaks.
pub fn encode(src: &[u8], line_len: usize) -> String {
    let mut out = String::with_capacity(src.len() * 4 / 3 + 4 + src.len() / 30);
    let mut line_chars = 0usize;
    let full = src.len() / 3 * 3;
    let mut i = 0;
    while i < full {
        let (c1, c2, c3) = (src[i] as usize, src[i + 1] as usize, src[i + 2] as usize);
        if line_len != 0 && line_chars >= line_len {
            out.push_str("\r\n");
            line_chars = 0;
        }
        out.push(ENC[(c1 >> 2) & 0x3f] as char);
        out.push(ENC[(((c1 & 3) << 4) + (c2 >> 4)) & 0x3f] as char);
        out.push(ENC[(((c2 & 15) << 2) + (c3 >> 6)) & 0x3f] as char);
        out.push(ENC[c3 & 0x3f] as char);
        line_chars += 4;
        i += 3;
    }
    if i < src.len() {
        let c1 = src[i] as usize;
        let c2 = if i + 1 < src.len() { src[i + 1] as usize } else { 0 };
        if line_len != 0 && line_chars >= line_len {
            out.push_str("\r\n");
        }
        out.push(ENC[(c1 >> 2) & 0x3f] as char);
        out.push(ENC[(((c1 & 3) << 4) + (c2 >> 4)) & 0x3f] as char);
        if i + 1 < src.len() {
            out.push(ENC[((c2 & 15) << 2) & 0x3f] as char);
        } else {
            out.push('=');
        }
        out.push('=');
    }
    out
}

/// Error of [`decode`] (`POLARSSL_ERR_BASE64_INVALID_CHARACTER`).
#[derive(Debug, PartialEq, Eq)]
pub struct InvalidCharacter;

/// Validate and return the size estimate of the first decoder pass
/// (`base64_decode(nullptr, &len, …)`), which the original uses for its
/// minimum length checks.
pub fn decoded_len_estimate(src: &[u8]) -> Result<usize, InvalidCharacter> {
    let slen = src.len();
    let mut n = 0usize;
    let mut j = 0u32;
    let mut i = 0usize;
    while i < slen {
        let mut x = 0;
        while i < slen && src[i] == b' ' {
            i += 1;
            x += 1;
        }
        if i == slen {
            break;
        }
        if slen - i >= 2 && src[i] == b'\r' && src[i + 1] == b'\n' {
            i += 1;
            continue;
        }
        if src[i] == b'\n' {
            i += 1;
            continue;
        }
        if x != 0 {
            return Err(InvalidCharacter);
        }
        if src[i] == b'=' {
            j += 1;
            if j > 2 {
                return Err(InvalidCharacter);
            }
        }
        let d = if src[i] > 127 { 127 } else { dec_map(src[i]) };
        if d == 127 {
            return Err(InvalidCharacter);
        }
        if d < 64 && j != 0 {
            return Err(InvalidCharacter);
        }
        n += 1;
        i += 1;
    }
    if n == 0 {
        return Ok(0);
    }
    let n = 6 * (n >> 3) + ((6 * (n & 7) + 7) >> 3);
    Ok(n - j as usize)
}

/// Decode (second pass of `base64_decode`).
pub fn decode(src: &[u8]) -> Result<Vec<u8>, InvalidCharacter> {
    decoded_len_estimate(src)?;
    let mut out = Vec::with_capacity(src.len() * 3 / 4);
    let mut j = 3u32;
    let mut n = 0;
    let mut x: u32 = 0;
    for &c in src {
        if c == b'\r' || c == b'\n' || c == b' ' {
            continue;
        }
        let d = dec_map(c);
        if d == 64 {
            j -= 1;
        }
        x = (x << 6) | (d & 0x3f) as u32;
        n += 1;
        if n == 4 {
            n = 0;
            if j > 0 {
                out.push((x >> 16) as u8);
            }
            if j > 1 {
                out.push((x >> 8) as u8);
            }
            if j > 2 {
                out.push(x as u8);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc4648_vectors() {
        for (p, e) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(encode(p.as_bytes(), 0), e);
            assert_eq!(decode(e.as_bytes()).unwrap(), p.as_bytes());
        }
    }

    #[test]
    fn line_breaks() {
        let data = [0xabu8; 120];
        let e = encode(&data, 76);
        let lines: Vec<&str> = e.split("\r\n").collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].len(), 76);
        assert_eq!(lines[1].len(), 76);
        assert_eq!(decode(e.as_bytes()).unwrap(), data);
    }

    #[test]
    fn invalid() {
        assert!(decode(b"Zm 9v").is_err());
        assert!(decode(b"Zm9v  \r\nYg==").is_ok());
        assert!(decode(b"Zg==Zg").is_err());
        assert!(decode("Zé==".as_bytes()).is_err());
    }
}
