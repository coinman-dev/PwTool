//! Safe wrapper around miniLZO (LZO1X-1), the same C code and version (2.10)
//! as in `upstream/src/minilzo`.

use std::os::raw::{c_int, c_long, c_short, c_uint, c_ulong, c_void};
use std::sync::Once;

use zeroize::Zeroizing;

type LzoUint = c_ulong;

/// Size of the work memory needed by `lzo1x_1_compress`
/// (`LZO1X_1_MEM_COMPRESS` = 16384 * sizeof(lzo_bytep)).
const MEM_COMPRESS: usize = 16384 * std::mem::size_of::<*const u8>();

const LZO_E_OK: c_int = 0;

unsafe extern "C" {
    fn __lzo_init_v2(
        v: c_uint,
        s1: c_int,
        s2: c_int,
        s3: c_int,
        s4: c_int,
        s5: c_int,
        s6: c_int,
        s7: c_int,
        s8: c_int,
        s9: c_int,
    ) -> c_int;
    fn lzo_version() -> c_uint;
    fn lzo1x_1_compress(
        src: *const u8,
        src_len: LzoUint,
        dst: *mut u8,
        dst_len: *mut LzoUint,
        wrkmem: *mut c_void,
    ) -> c_int;
    fn lzo1x_decompress_safe(
        src: *const u8,
        src_len: LzoUint,
        dst: *mut u8,
        dst_len: *mut LzoUint,
        wrkmem: *mut c_void,
    ) -> c_int;
}

/// Layout of `lzo_callback_t` (only its size is passed to `__lzo_init_v2`).
#[repr(C)]
struct LzoCallback {
    nalloc: *mut c_void,
    nfree: *mut c_void,
    nprogress: *mut c_void,
    user1: *mut c_void,
    user2: LzoUint,
    user3: LzoUint,
}

fn init() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        // SAFETY: lzo_init() with the type sizes of this platform, exactly as
        // the lzo_init() macro in minilzo.h does.
        let rc = unsafe {
            __lzo_init_v2(
                lzo_version(),
                std::mem::size_of::<c_short>() as c_int,
                std::mem::size_of::<c_int>() as c_int,
                std::mem::size_of::<c_long>() as c_int,
                std::mem::size_of::<u32>() as c_int,
                std::mem::size_of::<LzoUint>() as c_int,
                std::mem::size_of::<*mut u8>() as c_int,
                std::mem::size_of::<*mut u8>() as c_int,
                std::mem::size_of::<*mut c_void>() as c_int,
                std::mem::size_of::<LzoCallback>() as c_int,
            )
        };
        assert_eq!(rc, LZO_E_OK, "lzo_init() failed");
    });
}

/// Worst-case size of LZO1X output for `len` input bytes, as used by the
/// original (`len + len / 16 + 64 + 3`).
pub fn max_compressed_len(len: usize) -> usize {
    len + len / 16 + 64 + 3
}

/// Compress with LZO1X-1. The output buffer and work memory are wiped.
pub fn compress(src: &[u8]) -> Zeroizing<Vec<u8>> {
    init();
    let mut out = Zeroizing::new(vec![0u8; max_compressed_len(src.len())]);
    let mut wrk = Zeroizing::new(vec![0u8; MEM_COMPRESS]);
    let mut out_len: LzoUint = out.len() as LzoUint;
    // SAFETY: buffers are large enough per the LZO documentation.
    let rc = unsafe {
        lzo1x_1_compress(
            src.as_ptr(),
            src.len() as LzoUint,
            out.as_mut_ptr(),
            &mut out_len,
            wrk.as_mut_ptr().cast(),
        )
    };
    assert_eq!(rc, LZO_E_OK, "lzo1x_1_compress failed");
    out.truncate(out_len as usize);
    out
}

/// Decompress LZO1X data with overrun checks. `max_len` is the size of the
/// output buffer; the result may be shorter. Returns `None` on corrupt input
/// (the original then reports `CRYPTTEXT_ERROR_DECOMPRFAILED`).
pub fn decompress_safe(src: &[u8], max_len: usize) -> Option<Zeroizing<Vec<u8>>> {
    init();
    let mut out = Zeroizing::new(vec![0u8; max_len]);
    let mut out_len: LzoUint = max_len as LzoUint;
    // SAFETY: out has max_len bytes, the safe decompressor checks bounds.
    let rc = unsafe {
        lzo1x_decompress_safe(
            src.as_ptr(),
            src.len() as LzoUint,
            out.as_mut_ptr(),
            &mut out_len,
            std::ptr::null_mut(),
        )
    };
    if rc != LZO_E_OK {
        return None;
    }
    out.truncate(out_len as usize);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let data = b"Password Tech Password Tech Password Tech 0123456789".repeat(20);
        let c = compress(&data);
        assert!(c.len() < data.len());
        let d = decompress_safe(&c, data.len()).unwrap();
        assert_eq!(&*d, &data[..]);
        assert!(decompress_safe(&c[..c.len() / 2], data.len()).is_none());
    }
}
