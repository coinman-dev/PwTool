//! Password strength estimation with zxcvbn-c (`ZxcvbnMatch()`), the same C
//! implementation and dictionaries as the original.

use std::ffi::CString;
use std::os::raw::{c_char, c_double, c_void};

unsafe extern "C" {
    fn ZxcvbnMatch(passwd: *const c_char, user_dict: *const *const c_char, info: *mut *mut c_void) -> c_double;
}

/// Entropy estimate in bits for a password (UTF-8), as shown when
/// "Use advanced password strength estimation (zxcvbn)" is enabled.
pub fn zxcvbn_match(passw: &str) -> f64 {
    let mut bytes = passw.as_bytes().to_vec();
    // stop at an embedded NUL, like the C string API
    if let Some(p) = bytes.iter().position(|&b| b == 0) {
        bytes.truncate(p);
    }
    let c = CString::new(bytes).unwrap();
    // SAFETY: valid NUL-terminated string; no user dictionary, no info list.
    let r = unsafe { ZxcvbnMatch(c.as_ptr(), std::ptr::null(), std::ptr::null_mut()) };
    let mut v = c.into_bytes();
    zeroize::Zeroize::zeroize(&mut v);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weak_vs_strong() {
        let weak = zxcvbn_match("password");
        let strong = zxcvbn_match("Xk#9vT!q2Lm@7wRz");
        assert!(weak < 10.0, "{weak}");
        assert!(strong > 60.0, "{strong}");
        assert_eq!(zxcvbn_match(""), 0.0);
    }
}
