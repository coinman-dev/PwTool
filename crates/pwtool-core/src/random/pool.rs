//! Cryptographically secure random pool (`upstream/src/random/RandomPool.*`).
//!
//! The design is carried over unchanged (see the long comment at the top of
//! `RandomPool.h`): entropy is collected in an add buffer and mixed into a
//! 32-byte pool with keyed BLAKE2b; output is produced by AES-256-CTR, ChaCha20
//! or ChaCha8 under a key derived from the pool, with a "generator gate" (key
//! change) after every request. All buffers live in a locked memory page at a
//! random offset and are overwritten with noise after use.
//!
//! Differences to the original:
//! - the BLAKE2b state and the cipher context live outside the locked page
//!   (they are wiped after use);
//! - `Randomize()` gathers Linux system values instead of Windows API values.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::{LazyLock, Mutex, MutexGuard};

use blake2::Blake2bMac;
use blake2::digest::consts::U32;
use blake2::digest::{KeyInit, Mac};
use zeroize::{Zeroize, Zeroizing};

use super::RandomGenerator;
use super::fast::Jsf32;
use super::hrtimer::{clock_ns, filetime_now, high_res_timer};
use crate::crypto::chacha::ChaCha;
use crate::crypto::{Aes, increment_counter};
use crate::secure_mem::{LockedPage, PAGE_SIZE};

/// Pool size = hash length = cipher key length.
pub const POOL_SIZE: usize = 32;
/// Maximum entropy the pool can provide, in bits.
pub const MAX_ENTROPY: u32 = (POOL_SIZE * 8) as u32;

const KEY_SIZE: usize = 32;
const CTR_SIZE: usize = 16;
const ADDBUF_SIZE: usize = 512;
const GETBUF_SIZE: usize = 64;
const TEMPBUF_SIZE: usize = CTR_SIZE;

const POOL_OFFSET: usize = 0;
const ADDBUF_OFFSET: usize = POOL_OFFSET + POOL_SIZE;
const CIPHERKEY_OFFSET: usize = ADDBUF_OFFSET + ADDBUF_SIZE;
const SECCTR_OFFSET: usize = CIPHERKEY_OFFSET + KEY_SIZE;
const GETBUF_OFFSET: usize = SECCTR_OFFSET + CTR_SIZE;
const TEMPBUF_OFFSET: usize = GETBUF_OFFSET + GETBUF_SIZE;
const POOL_DATA_SIZE: usize = TEMPBUF_OFFSET + TEMPBUF_SIZE;
const UNUSED_SIZE_MAX: usize = PAGE_SIZE - POOL_DATA_SIZE;
const UNUSED_SIZE_MIN: usize = 64;

/// Output cipher of the pool (Configuration | Security, `RandomPoolCipher`
/// in `PwTech.ini`: 0 = AES-CTR, 1 = ChaCha20, 2 = ChaCha8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoolCipherType {
    AesCtr = 0,
    ChaCha20 = 1,
    ChaCha8 = 2,
}

impl PoolCipherType {
    pub fn from_index(i: i32) -> Option<Self> {
        match i {
            0 => Some(Self::AesCtr),
            1 => Some(Self::ChaCha20),
            2 => Some(Self::ChaCha8),
            _ => None,
        }
    }
}

/// Counter-based cipher behind the pool (`RandPoolCipher::CtrBasedCipher`).
enum PoolCipher {
    Aes(Option<Aes>),
    ChaCha(ChaCha, u32),
}

impl PoolCipher {
    fn new(t: PoolCipherType) -> Self {
        match t {
            PoolCipherType::AesCtr => PoolCipher::Aes(None),
            PoolCipherType::ChaCha20 => PoolCipher::ChaCha(ChaCha::new(&[0u8; 32]), 20),
            PoolCipherType::ChaCha8 => PoolCipher::ChaCha(ChaCha::new(&[0u8; 32]), 8),
        }
    }

    fn set_key(&mut self, key: &[u8; 32]) {
        match self {
            PoolCipher::Aes(a) => *a = Some(Aes::new(key)),
            PoolCipher::ChaCha(c, rounds) => {
                c.key_setup(key);
                c.set_rounds(*rounds);
            }
        }
    }

    fn clear(&mut self) {
        match self {
            PoolCipher::Aes(a) => *a = None,
            PoolCipher::ChaCha(c, rounds) => {
                c.key_setup(&[0u8; 32]);
                c.set_rounds(*rounds);
            }
        }
    }

    fn block_size(&self) -> usize {
        match self {
            PoolCipher::Aes(_) => 16,
            PoolCipher::ChaCha(..) => 64,
        }
    }

    fn max_blocks(&self) -> u64 {
        match self {
            PoolCipher::Aes(_) => 65536,
            PoolCipher::ChaCha(..) => 0x4_0000_0000, // 1 TB before a key change
        }
    }

    fn aes(&self) -> &Aes {
        match self {
            PoolCipher::Aes(Some(a)) => a,
            _ => panic!("RandomPool: cipher key not set"),
        }
    }

    /// `FillBlocks()`.
    fn fill_blocks(&mut self, buf: &mut [u8], counter: &mut [u8; CTR_SIZE], blocks: usize) {
        match self {
            PoolCipher::Aes(_) => {
                for i in 0..blocks {
                    let out = self.aes().encrypt_block(counter);
                    buf[i * 16..i * 16 + 16].copy_from_slice(&out);
                    increment_counter(counter);
                }
            }
            PoolCipher::ChaCha(c, _) => c.keystream_bytes(&mut buf[..blocks * 64]),
        }
    }

    /// `SelfKey()`: derive a new key from the output and set it.
    fn self_key(&mut self, key_out: &mut [u8; KEY_SIZE], counter: &mut [u8; CTR_SIZE]) {
        match self {
            PoolCipher::Aes(_) => {
                self.fill_blocks(&mut key_out[..], counter, 2);
            }
            PoolCipher::ChaCha(c, _) => c.keystream_bytes(&mut key_out[..]),
        }
        let k = *key_out;
        self.set_key(&k);
    }

    /// `ProcessCounterOrIV()`.
    fn process_counter_or_iv(&mut self, ctr: &mut [u8; CTR_SIZE]) {
        match self {
            PoolCipher::Aes(_) => *ctr = self.aes().encrypt_block(ctr),
            PoolCipher::ChaCha(c, _) => {
                c.apply_keystream(&mut ctr[..]);
                let (iv, ctr2) = ctr.split_at(8);
                c.iv_setup(iv, Some(ctr2));
            }
        }
    }
}

/// The random pool. Use [`RandomPool::global`] for the program-wide instance
/// (`RandomPool::GetInstance()`).
pub struct RandomPool {
    page: LockedPage,
    offset: usize,
    lock_phys_mem: bool,
    randomize_fixed_items_added: bool,
    cipher_type: PoolCipherType,
    cipher: PoolCipher,
    add_buf_pos: usize,
    get_buf_pos: usize,
    touch_pool_index: usize,
    num_blocks: u64,
    key_set: bool,
    fast_rng: Option<Jsf32>,
    /// Test hook, see [`RandomPool::set_deterministic_source`].
    det_source: Option<Box<dyn RandomGenerator>>,
}

static GLOBAL: LazyLock<Mutex<RandomPool>> = LazyLock::new(|| {
    Mutex::new(RandomPool::new(PoolCipherType::ChaCha20, Some(Jsf32::new()), true))
});

impl RandomPool {
    /// New pool. With `fast_rng`, wiped buffers are overwritten with noise;
    /// with `lock_phys_mem`, the data starts at a random offset of the page.
    pub fn new(cipher: PoolCipherType, fast_rng: Option<Jsf32>, lock_phys_mem: bool) -> Self {
        let mut pool = RandomPool {
            page: LockedPage::new(),
            offset: 0,
            lock_phys_mem,
            randomize_fixed_items_added: false,
            cipher_type: cipher,
            cipher: PoolCipher::new(cipher),
            add_buf_pos: 0,
            get_buf_pos: 0,
            touch_pool_index: 0,
            num_blocks: 0,
            key_set: false,
            fast_rng,
            det_source: None,
        };
        let mut noise = Zeroizing::new(vec![0u8; PAGE_SIZE]);
        pool.clear_buf(&mut noise);
        pool.page.as_mut_slice().copy_from_slice(&noise);
        if lock_phys_mem {
            if let Some(f) = pool.fast_rng.as_mut() {
                pool.offset = f.get_num_range_between(UNUSED_SIZE_MIN as u32, UNUSED_SIZE_MAX as u32 + 1)
                    as usize;
            }
        }
        pool
    }

    /// Lock and return the program-wide pool.
    pub fn global() -> MutexGuard<'static, RandomPool> {
        GLOBAL.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Independent pool seeded with 32 bytes from `src`, which is flushed
    /// afterwards (`RandomPool(RandomPool& src, …)`); used by worker threads.
    pub fn derive_from(src: &mut RandomPool) -> Self {
        let mut p = RandomPool::new(src.cipher_type, Some(Jsf32::new()), false);
        let mut entropy = Zeroizing::new([0u8; POOL_SIZE]);
        src.get_data(&mut entropy[..]);
        p.add_data(&entropy[..]);
        src.flush();
        p
    }

    /// Test hook: while set, [`get_data`](RandomGenerator::get_data) and
    /// [`get_byte`](RandomGenerator::get_byte) return the output of `src`
    /// instead of the pool's own (the pool state is otherwise unaffected).
    /// Makes salts, IVs and padding reproducible for comparisons with the
    /// original; `None` restores normal operation. Not used by the program.
    pub fn set_deterministic_source(&mut self, src: Option<Box<dyn RandomGenerator>>) {
        self.det_source = src;
    }

    /// Whether the page could be locked into RAM.
    pub fn is_memory_locked(&self) -> bool {
        self.page.is_locked()
    }

    pub fn cipher(&self) -> PoolCipherType {
        self.cipher_type
    }

    /// `SetCipher()`.
    pub fn set_cipher(&mut self, cipher: PoolCipherType) {
        if cipher != self.cipher_type {
            self.cipher = PoolCipher::new(cipher);
            self.cipher_type = cipher;
            if self.key_set {
                self.set_key();
            }
        }
    }

    // --- page regions -------------------------------------------------------

    fn region(&mut self, off: usize, len: usize) -> &mut [u8] {
        let o = self.offset + off;
        &mut self.page.as_mut_slice()[o..o + len]
    }

    fn read_array<const N: usize>(&mut self, off: usize) -> [u8; N] {
        let mut a = [0u8; N];
        a.copy_from_slice(self.region(off, N));
        a
    }

    fn clear_buf(&mut self, buf: &mut [u8]) {
        match self.fast_rng.as_mut() {
            Some(f) => f.get_data(buf),
            None => buf.zeroize(),
        }
    }

    fn clear_region(&mut self, off: usize, len: usize) {
        let mut tmp = Zeroizing::new(vec![0u8; len]);
        self.clear_buf(&mut tmp);
        self.region(off, len).copy_from_slice(&tmp);
    }

    /// Keep the locked page in use (`TouchPool()`).
    pub fn touch_pool(&mut self) {
        if self.lock_phys_mem && self.offset > 0 {
            if self.touch_pool_index >= self.offset {
                self.touch_pool_index = 0;
            }
            let i = self.touch_pool_index;
            let b = &mut self.page.as_mut_slice()[i];
            *b = b.wrapping_sub(1);
            self.touch_pool_index += 1;
        }
    }

    /// Move the pool to a new memory page (`MovePool()`).
    pub fn move_pool(&mut self) -> bool {
        let mut new_page = LockedPage::new();
        self.clear_buf(new_page.as_mut_slice());
        let o = self.offset;
        new_page.as_mut_slice()[o..].copy_from_slice(&self.page.as_slice()[o..]);
        self.page = new_page;
        true
    }

    // --- core algorithm -----------------------------------------------------

    /// `UpdatePool()`: P <- BLAKE2b_P(add buffer || counter || timer).
    fn update_pool(&mut self) {
        let pool: [u8; POOL_SIZE] = self.read_array(POOL_OFFSET);
        let mut h = <Blake2bMac<U32> as KeyInit>::new_from_slice(&pool).expect("key size");
        let mut pool = Zeroizing::new(pool);
        if self.add_buf_pos != 0 {
            let n = self.add_buf_pos;
            Mac::update(&mut h, self.region(ADDBUF_OFFSET, n));
            self.clear_region(ADDBUF_OFFSET, n);
            self.add_buf_pos = 0;
        }
        if self.key_set {
            Mac::update(&mut h, self.region(SECCTR_OFFSET, CTR_SIZE));
        }
        let timer = high_res_timer().to_le_bytes();
        self.region(TEMPBUF_OFFSET, 8).copy_from_slice(&timer);
        Mac::update(&mut h, self.region(TEMPBUF_OFFSET, 8));
        pool.copy_from_slice(&h.finalize().into_bytes());
        self.region(POOL_OFFSET, POOL_SIZE).copy_from_slice(&*pool);
        self.clear_region(TEMPBUF_OFFSET, 8);
        if self.key_set {
            self.clear_region(SECCTR_OFFSET, CTR_SIZE);
            self.cipher.clear();
            self.clear_region(GETBUF_OFFSET, GETBUF_SIZE);
            self.get_buf_pos = 0;
            self.num_blocks = 0;
            self.key_set = false;
        }
    }

    /// `SetKey()`: derive the output key K <- BLAKE2b_T(P) and a new counter.
    fn set_key(&mut self) {
        self.update_pool();
        let timer = high_res_timer().to_le_bytes();
        self.region(TEMPBUF_OFFSET, 8).copy_from_slice(&timer);
        let temp: Zeroizing<[u8; TEMPBUF_SIZE]> = Zeroizing::new(self.read_array(TEMPBUF_OFFSET));
        let mut h = <Blake2bMac<U32> as KeyInit>::new_from_slice(&temp[..]).expect("key size");
        Mac::update(&mut h, self.region(POOL_OFFSET, POOL_SIZE));
        let mut key = Zeroizing::new([0u8; KEY_SIZE]);
        key.copy_from_slice(&h.finalize().into_bytes());
        self.clear_region(TEMPBUF_OFFSET, TEMPBUF_SIZE);
        self.cipher.set_key(&key);
        self.clear_region(CIPHERKEY_OFFSET, KEY_SIZE);
        let mut ctr = Zeroizing::new(new_counter_or_iv());
        self.cipher.process_counter_or_iv(&mut ctr);
        self.region(SECCTR_OFFSET, CTR_SIZE).copy_from_slice(&*ctr);
        self.get_buf_pos = GETBUF_SIZE;
        self.num_blocks = 0;
        self.key_set = true;
    }

    /// `GeneratorGate()`: change the output key and counter.
    fn generator_gate(&mut self) {
        let mut ctr: Zeroizing<[u8; CTR_SIZE]> = Zeroizing::new(self.read_array(SECCTR_OFFSET));
        let mut key = Zeroizing::new([0u8; KEY_SIZE]);
        self.cipher.self_key(&mut key, &mut ctr);
        self.clear_region(CIPHERKEY_OFFSET, KEY_SIZE);
        let temp = Zeroizing::new(new_counter_or_iv());
        for i in 0..CTR_SIZE {
            ctr[i] ^= temp[i];
        }
        self.clear_region(TEMPBUF_OFFSET, CTR_SIZE);
        self.cipher.process_counter_or_iv(&mut ctr);
        self.region(SECCTR_OFFSET, CTR_SIZE).copy_from_slice(&*ctr);
        self.num_blocks = 0;
    }

    /// `FillBuf()`: fill as many whole cipher blocks of `buf` as possible.
    fn fill_buf(&mut self, buf: &mut [u8]) -> usize {
        let bs = self.cipher.block_size();
        let full_blocks = buf.len() / bs;
        assert!(full_blocks != 0, "RandomPool::FillBuf(): Number of blocks is zero");
        let mut pos = 0;
        let mut remaining = full_blocks;
        while remaining != 0 {
            let n = (remaining as u64).min(self.cipher.max_blocks() - self.num_blocks) as usize;
            let mut ctr: Zeroizing<[u8; CTR_SIZE]> = Zeroizing::new(self.read_array(SECCTR_OFFSET));
            self.cipher.fill_blocks(&mut buf[pos..], &mut ctr, n);
            self.region(SECCTR_OFFSET, CTR_SIZE).copy_from_slice(&*ctr);
            pos += n * bs;
            remaining -= n;
            self.num_blocks += n as u64;
            if self.num_blocks >= self.cipher.max_blocks() {
                self.generator_gate();
            }
        }
        full_blocks * bs
    }

    /// `FillGetBuf()`.
    fn fill_get_buf(&mut self) {
        let mut buf = Zeroizing::new([0u8; GETBUF_SIZE]);
        self.fill_buf(&mut buf[..]);
        self.region(GETBUF_OFFSET, GETBUF_SIZE).copy_from_slice(&*buf);
        self.generator_gate();
        self.get_buf_pos = 0;
    }

    /// Add entropy (`AddData()`): XORed into the add buffer, which is mixed
    /// into the pool whenever it is full.
    pub fn add_data(&mut self, data: &[u8]) {
        for &b in data {
            if self.add_buf_pos == ADDBUF_SIZE {
                self.update_pool();
            }
            let o = self.offset + ADDBUF_OFFSET + self.add_buf_pos;
            self.page.as_mut_slice()[o] ^= b;
            self.add_buf_pos += 1;
        }
    }

    /// Mix pending entropy into the pool and discard the current output key
    /// (`Flush()`).
    pub fn flush(&mut self) {
        self.update_pool();
    }

    /// Prepare for output (`RandReady()`).
    pub fn rand_ready(&mut self) {
        self.set_key();
    }

    /// Add system values to the pool (`Randomize()`). The original reads ~60
    /// Windows API values plus `BCryptGenRandom`; here the Linux equivalents
    /// plus 32 bytes from `getrandom()`.
    pub fn randomize_system(&mut self) {
        self.add_data(&high_res_timer().to_le_bytes());
        self.add_data(&filetime_now().to_le_bytes());

        // SAFETY: plain libc calls filling local structs.
        unsafe {
            let mut vfs: libc::statvfs = std::mem::zeroed();
            if libc::statvfs(c".".as_ptr(), &mut vfs) == 0 {
                add_pod(self, &vfs.f_bavail);
                add_pod(self, &vfs.f_ffree);
            }
            let mut si: libc::sysinfo = std::mem::zeroed();
            if libc::sysinfo(&mut si) == 0 {
                add_pod(self, &si);
            }
            add_pod(self, &libc::getpid());
            add_pod(self, &libc::getppid());
            add_pod(self, &libc::gettid());
            let mut ru: libc::rusage = std::mem::zeroed();
            if libc::getrusage(libc::RUSAGE_SELF, &mut ru) == 0 {
                add_pod(self, &ru);
            }
            if libc::getrusage(libc::RUSAGE_THREAD, &mut ru) == 0 {
                add_pod(self, &ru);
            }
        }
        for clk in [
            libc::CLOCK_MONOTONIC,
            libc::CLOCK_BOOTTIME,
            libc::CLOCK_PROCESS_CPUTIME_ID,
            libc::CLOCK_THREAD_CPUTIME_ID,
        ] {
            self.add_data(&clock_ns(clk).to_le_bytes());
        }

        if !self.randomize_fixed_items_added {
            // SAFETY: as above.
            unsafe {
                let mut uts: libc::utsname = std::mem::zeroed();
                if libc::uname(&mut uts) == 0 {
                    add_pod(self, &uts);
                }
                add_pod(self, &libc::getuid());
                add_pod(self, &libc::getgid());
                add_pod(self, &libc::sysconf(libc::_SC_NPROCESSORS_ONLN));
                add_pod(self, &libc::sysconf(libc::_SC_PHYS_PAGES));
            }
            let secs = ((filetime_now() - super::hrtimer::FILETIME_UNIX_EPOCH) / 10_000_000) as libc::time_t;
            self.add_data(&super::hrtimer::utc_offset_seconds(secs).to_le_bytes());
            self.randomize_fixed_items_added = true;
        }

        let mut os_rand = Zeroizing::new([0u8; POOL_SIZE]);
        if getrandom::fill(&mut os_rand[..]).is_ok() {
            self.add_data(&os_rand[..]);
        }
        self.add_data(&high_res_timer().to_le_bytes());
    }

    /// Write 64 random bytes to the seed file (`WriteSeedFile()`).
    pub fn write_seed_file(&mut self, path: &Path) -> bool {
        self.set_key();
        let mut buf = Zeroizing::new([0u8; 2 * POOL_SIZE]);
        self.get_data(&mut buf[..]);
        self.flush();
        fs::File::create(path).and_then(|mut f| f.write_all(&buf[..])).is_ok()
    }

    /// Add the contents of the seed file to the pool (`ReadSeedFile()`);
    /// returns `true` if at least `POOL_SIZE` bytes were read.
    pub fn read_seed_file(&mut self, path: &Path) -> bool {
        match fs::read(path) {
            Ok(data) => {
                let data = Zeroizing::new(data);
                let n = data.len().min(2 * POOL_SIZE);
                self.add_data(&data[..n]);
                n >= POOL_SIZE
            }
            Err(_) => false,
        }
    }
}

/// Add the raw bytes of a plain-old-data value (`AddData(const T&)`).
fn add_pod<T>(pool: &mut RandomPool, v: &T) {
    // SAFETY: reading the bytes of a fully initialised C struct/integer.
    let bytes = unsafe { std::slice::from_raw_parts((v as *const T).cast::<u8>(), std::mem::size_of::<T>()) };
    pool.add_data(bytes);
}

/// `GetNewCounterOrIV()`: timer (8 bytes) || FILETIME (8 bytes).
fn new_counter_or_iv() -> [u8; CTR_SIZE] {
    let mut c = [0u8; CTR_SIZE];
    c[..8].copy_from_slice(&high_res_timer().to_le_bytes());
    c[8..].copy_from_slice(&filetime_now().to_le_bytes());
    c
}

impl RandomGenerator for RandomPool {
    /// `Seed()` = `AddData()`.
    fn seed(&mut self, seed: &[u8]) {
        self.add_data(seed);
    }

    /// `GetData()`.
    fn get_data(&mut self, buf: &mut [u8]) {
        if let Some(src) = self.det_source.as_mut() {
            src.get_data(buf);
            return;
        }
        if !self.key_set {
            self.set_key();
        }
        let mut pos = 0;
        let mut get_buf_filled = false;
        while pos < buf.len() {
            if self.get_buf_pos == GETBUF_SIZE {
                let remaining = buf.len() - pos;
                if remaining >= GETBUF_SIZE {
                    let generated = self.fill_buf(&mut buf[pos..]);
                    if generated == remaining {
                        self.generator_gate();
                        if get_buf_filled {
                            self.clear_region(GETBUF_OFFSET, GETBUF_SIZE);
                        }
                        return;
                    }
                    pos += generated;
                }
                self.fill_get_buf();
            }
            let n = (buf.len() - pos).min(GETBUF_SIZE - self.get_buf_pos);
            let src = self.offset + GETBUF_OFFSET + self.get_buf_pos;
            buf[pos..pos + n].copy_from_slice(&self.page.as_slice()[src..src + n]);
            pos += n;
            self.get_buf_pos += n;
            get_buf_filled = true;
        }
    }

    fn get_byte(&mut self) -> u8 {
        if let Some(src) = self.det_source.as_mut() {
            return src.get_byte();
        }
        if !self.key_set {
            self.set_key();
        }
        if self.get_buf_pos == GETBUF_SIZE {
            self.fill_get_buf();
        }
        let b = self.page.as_slice()[self.offset + GETBUF_OFFSET + self.get_buf_pos];
        self.get_buf_pos += 1;
        b
    }

    fn randomize(&mut self) {
        self.randomize_system();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exercise(cipher: PoolCipherType) {
        let mut p = RandomPool::new(cipher, Some(Jsf32::with_seed(b"test")), true);
        p.randomize_system();
        let mut a = [0u8; 1000];
        p.get_data(&mut a);
        let mut b = [0u8; 1000];
        p.get_data(&mut b);
        assert_ne!(a, b);
        assert!(a.iter().any(|&x| x != 0));
        // Small requests come from the get buffer.
        let x: Vec<u8> = (0..200).map(|_| p.get_byte()).collect();
        assert!(x.iter().any(|&v| v != x[0]));
        p.flush();
        let mut c = [0u8; 63];
        p.get_data(&mut c);
        assert!(c.iter().any(|&v| v != 0));
        assert!(p.get_num_range(10) < 10);
    }

    #[test]
    fn all_ciphers_work() {
        exercise(PoolCipherType::AesCtr);
        exercise(PoolCipherType::ChaCha20);
        exercise(PoolCipherType::ChaCha8);
    }

    #[test]
    fn cipher_switch_and_move() {
        let mut p = RandomPool::new(PoolCipherType::ChaCha20, Some(Jsf32::new()), true);
        let mut a = [0u8; 40];
        p.get_data(&mut a);
        p.set_cipher(PoolCipherType::AesCtr);
        p.get_data(&mut a);
        assert!(p.move_pool());
        p.touch_pool();
        p.get_data(&mut a);
        assert!(a.iter().any(|&v| v != 0));
    }

    #[test]
    fn seed_file_roundtrip() {
        let dir = std::env::temp_dir().join(format!("pwtool-seed-{}", std::process::id()));
        let mut p = RandomPool::new(PoolCipherType::ChaCha20, None, false);
        assert!(p.write_seed_file(&dir));
        assert_eq!(fs::metadata(&dir).unwrap().len(), 64);
        assert!(p.read_seed_file(&dir));
        fs::remove_file(&dir).unwrap();
    }
}
