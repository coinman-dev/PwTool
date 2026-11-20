//! Protection of sensitive data in memory.
//!
//! Counterpart of `upstream/src/main/SecureMem.h` and `upstream/src/util/MemUtil.*`.
//! The original keeps secrets in buffers that are wiped on release and, for the
//! most sensitive data (random pool, database keys), in a memory page locked
//! with `VirtualLock`. Here the same is done with `zeroize`, `mlock` and
//! `MADV_DONTDUMP` on Linux, and `VirtualAlloc` + `VirtualLock` on Windows.

use std::ptr::NonNull;

use zeroize::Zeroize;

pub use zeroize::{Zeroizing, Zeroize as _};

use crate::crypto::chacha::ChaCha;

/// Size of the locked memory page used by the random pool and the database
/// (`POOLPAGE_SIZE`, `SECMEM_SIZE` in the original).
pub const PAGE_SIZE: usize = 4096;

/// Overwrite a buffer with zeros in a way the optimiser cannot remove
/// (`memzero()` in the original).
#[inline]
pub fn memzero(buf: &mut [u8]) {
    buf.zeroize();
}

/// Encrypt or decrypt (the operation is symmetric) a buffer with ChaCha20 under
/// a 128- or 256-bit key, IV and counter zero (`memcrypt()` in `MemUtil.cpp`).
/// Used to keep keys encrypted while they are stored in memory.
pub fn memcrypt(src: &[u8], dest: &mut [u8], key: &[u8]) {
    assert!(key.len() >= 16, "memcrypt: Key too small");
    assert_eq!(src.len(), dest.len());
    let mut ctx = ChaCha::new(&key[..if key.len() >= 32 { 32 } else { 16 }]);
    ctx.encrypt_bytes(src, dest);
}

/// In-place variant of [`memcrypt`].
pub fn memcrypt_in_place(buf: &mut [u8], key: &[u8]) {
    assert!(key.len() >= 16, "memcrypt: Key too small");
    let mut ctx = ChaCha::new(&key[..if key.len() >= 32 { 32 } else { 16 }]);
    ctx.apply_keystream(buf);
}

/// A page of memory that is locked into RAM (never swapped), excluded from core
/// dumps and wiped before it is released (`VirtualAlloc` + `VirtualLock` in the
/// original). If locking fails (e.g. `RLIMIT_MEMLOCK` too small) the page is
/// still usable, only the swap protection is missing; [`LockedPage::is_locked`]
/// reports it.
pub struct LockedPage {
    ptr: NonNull<u8>,
    locked: bool,
}

// The page is exclusively owned.
unsafe impl Send for LockedPage {}
unsafe impl Sync for LockedPage {}

impl LockedPage {
    /// Allocate a new zero-filled page, locking it into RAM if `lock` is true.
    #[cfg(windows)]
    pub fn alloc(lock: bool) -> Self {
        use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_READWRITE, VirtualAlloc, VirtualLock};
        // SAFETY: fresh committed allocation, checked for failure below.
        let ptr = unsafe { VirtualAlloc(std::ptr::null(), PAGE_SIZE, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE) };
        if ptr.is_null() {
            panic!("LockedPage: VirtualAlloc failed: {}", std::io::Error::last_os_error());
        }
        // SAFETY: ptr is a valid allocation of PAGE_SIZE bytes.
        let locked = lock && unsafe { VirtualLock(ptr, PAGE_SIZE) } != 0;
        LockedPage { ptr: NonNull::new(ptr.cast()).unwrap(), locked }
    }

    /// Allocate a new zero-filled page, locking it into RAM if `lock` is true.
    #[cfg(unix)]
    pub fn alloc(lock: bool) -> Self {
        // SAFETY: anonymous private mapping, checked for failure below.
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                PAGE_SIZE,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            panic!("LockedPage: mmap failed: {}", std::io::Error::last_os_error());
        }
        // SAFETY: ptr is a valid mapping of PAGE_SIZE bytes.
        let locked = unsafe {
            libc::madvise(ptr, PAGE_SIZE, libc::MADV_DONTDUMP);
            lock && libc::mlock(ptr, PAGE_SIZE) == 0
        };
        LockedPage { ptr: NonNull::new(ptr.cast()).unwrap(), locked }
    }

    /// Allocate a new zero-filled locked page.
    pub fn new() -> Self {
        Self::alloc(true)
    }

    pub fn is_locked(&self) -> bool {
        self.locked
    }

    pub fn as_slice(&self) -> &[u8] {
        // SAFETY: the mapping is PAGE_SIZE bytes long and owned by self.
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), PAGE_SIZE) }
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: as above, and &mut self guarantees exclusive access.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), PAGE_SIZE) }
    }
}

impl Default for LockedPage {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for LockedPage {
    fn drop(&mut self) {
        self.as_mut_slice().zeroize();
        #[cfg(windows)]
        // SAFETY: releasing the allocation created in alloc().
        unsafe {
            use windows_sys::Win32::System::Memory::{MEM_RELEASE, VirtualFree, VirtualUnlock};
            if self.locked {
                VirtualUnlock(self.ptr.as_ptr().cast(), PAGE_SIZE);
            }
            VirtualFree(self.ptr.as_ptr().cast(), 0, MEM_RELEASE);
        }
        #[cfg(unix)]
        // SAFETY: unmapping the mapping created in alloc().
        unsafe {
            if self.locked {
                libc::munlock(self.ptr.as_ptr().cast(), PAGE_SIZE);
            }
            libc::munmap(self.ptr.as_ptr().cast(), PAGE_SIZE);
        }
    }
}

/// Process-wide hardening done once at start-up: forbid core dumps and
/// attaching debuggers of the same user (`ptrace`). The original has no exact
/// equivalent; on Windows other processes of the same user cannot read the
/// locked pages either.
#[cfg(unix)]
pub fn harden_process() {
    // SAFETY: plain syscalls with constant arguments.
    unsafe {
        let rl = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        libc::setrlimit(libc::RLIMIT_CORE, &rl);
        libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0);
    }
}

/// Process-wide hardening on Windows: no Windows Error Reporting dialog and
/// crash dump with process memory (the original does nothing here).
#[cfg(windows)]
pub fn harden_process() {
    use windows_sys::Win32::System::Diagnostics::Debug::{
        SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX, SetErrorMode,
    };
    // SAFETY: constant flags.
    unsafe {
        SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX);
    }
}

/// Byte buffer that is wiped when dropped (`SecureMem<word8>`).
pub type SecureBytes = Zeroizing<Vec<u8>>;

/// String that is wiped when dropped (`SecureWString`).
pub type SecureString = Zeroizing<String>;

/// Sequence of Unicode scalar values wiped when dropped (`SecureW32String`,
/// UTF-32 strings used by the password generator).
pub type SecureChars = Zeroizing<Vec<char>>;

/// Create a zero-filled secure byte buffer of the given size.
pub fn secure_bytes(len: usize) -> SecureBytes {
    Zeroizing::new(vec![0u8; len])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locked_page_roundtrip() {
        let mut page = LockedPage::new();
        page.as_mut_slice()[..4].copy_from_slice(b"test");
        assert_eq!(&page.as_slice()[..4], b"test");
        assert_eq!(page.as_slice().len(), PAGE_SIZE);
    }

    #[test]
    fn memcrypt_is_symmetric() {
        let key = [7u8; 16];
        let data = b"some secret key material".to_vec();
        let mut enc = vec![0u8; data.len()];
        memcrypt(&data, &mut enc, &key);
        assert_ne!(enc, data);
        let mut dec = vec![0u8; data.len()];
        memcrypt(&enc, &mut dec, &key);
        assert_eq!(dec, data);
    }
}
