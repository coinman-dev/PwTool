//! Password database ("PassCube") — port of `upstream/src/passw/PasswDatabase.*`.
//!
//! File format: see `.ai/FORMATS.md` §1. Reads versions 1.0–1.8 and writes
//! version 1.8, compatible with Password Tech 3.6.x for Windows.
//!
//! Differences to the original:
//! - files are written to a temporary file in the same directory and then
//!   renamed (the original overwrites the file in place);
//! - instead of Windows share modes, an advisory `flock` is held on the open
//!   file; saving fails if another process holds the file open;
//! - passwords are kept in memory encrypted with ChaCha20 under a per-session
//!   key, as in the original, but their plaintext is UTF-8 instead of UTF-16.

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use flate2::read::ZlibDecoder;
use zeroize::{Zeroize, Zeroizing};

use crate::crypto::chacha::ChaCha;
use crate::crypto::{
    HmacAny, HmacSha256, HmacSha512, SymmetricCipher, align_to_block_size, hmac_sha1, hmac_sha256, hmac_sha512,
    pbkdf2_256bit,
};
use crate::language::trl;
use crate::random::fast::fast_fill;
use crate::random::{RandomGenerator, RandomPool};
use crate::secure_mem::LockedPage;

pub const VERSION_HIGH: u16 = 1;
pub const VERSION_LOW: u16 = 8;
pub const VERSION: u16 = (VERSION_HIGH << 8) | VERSION_LOW;
pub const KEY_HASH_ITERATIONS: u32 = 300_000;
pub const CIPHER_AES256: u8 = 0;
pub const CIPHER_CHACHA20: u8 = 1;
pub const COMPRESSION_DEFLATE: u8 = 1;
pub const MAX_PASSW_HISTORY_SIZE: u32 = 0xff;

const HEADER_MAGIC: u32 = 0x6264_5750; // "PWdb"
const FH_FLAG_RECOVERY_KEY: u32 = 1;
const FH_FLAG_PASSW_UTF8: u32 = 2;
const FLAG_DEFAULT_USER_NAME: u32 = 1;
const FLAG_PASSW_FORMAT_SEQ: u32 = 2;
const FLAG_PASSW_EXPIRY_DAYS: u32 = 4;
const FLAG_DEFAULT_PASSW_HISTORY_SIZE: u32 = 8;
const FLAG_MASTER_PASSW_EXPIRY: u32 = 16;
const FLAG_DESCRIPTION: u32 = 32;
const MAX_FILE_SIZE: u64 = 104_857_600;
const PARAMSTR_DEFAULT_USER_NAME: &str = "DefUserName";
const PARAMSTR_PASSW_FORMAT_SEQ: &str = "PWFormatSeq";

const FILE_HEADER_SIZE: usize = 18;
const DB_HEADER_SIZE: usize = 26;
const PASSW_HISTORY_HEADER_SIZE: usize = 7;

const SECMEM_KEY_LENGTH: usize = 32;
const SECMEM_SALT_LENGTH: usize = 16;
const DB_KEY_LENGTH: usize = 32;
const DB_SALT_LENGTH: usize = 32;
const DB_RECOVERY_KEY_BLOCK_LENGTH: usize = 2 * (DB_SALT_LENGTH + DB_KEY_LENGTH);
const HASH_SHA256: u8 = 0;
const HASH_SHA512: u8 = 1;
const KDF_PBKDF2_SHA256: u8 = 0;

unsafe extern "C" {
    fn mz_compress2(dest: *mut u8, dest_len: *mut std::ffi::c_ulong, src: *const u8, src_len: std::ffi::c_ulong, level: i32) -> i32;
    fn mz_compressBound(src_len: std::ffi::c_ulong) -> std::ffi::c_ulong;
}

/// zlib stream of `data` produced by miniz (the library of the original,
/// `Deflate` of `DataCompressor.h`), so that files are byte-identical.
fn miniz_compress(data: &[u8], level: i32) -> io::Result<Zeroizing<Vec<u8>>> {
    // SAFETY: the buffers are valid for the given lengths; miniz writes at
    // most `dest_len` bytes and updates it.
    unsafe {
        let mut dest_len = mz_compressBound(data.len() as std::ffi::c_ulong);
        let mut out = Zeroizing::new(vec![0u8; dest_len as usize]);
        let rc = mz_compress2(out.as_mut_ptr(), &mut dest_len, data.as_ptr(), data.len() as std::ffi::c_ulong, level);
        if rc != 0 {
            return Err(io::Error::other("deflate() failed"));
        }
        out.truncate(dest_len as usize);
        Ok(out)
    }
}

/// Default compression level of miniz (`MZ_DEFAULT_LEVEL`).
const MZ_DEFAULT_LEVEL: i32 = 6;
const MZ_BEST_COMPRESSION: i32 = 9;

pub const E_INVALID_FORMAT: &str = "Invalid/unknown file format";

/// Database errors (`EPasswDbError` and subclasses).
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("{0}")]
    Error(String),
    #[error("{0}")]
    InvalidFormat(String),
    /// Wrong key or modified file (HMAC mismatch).
    #[error("{0}")]
    InvalidKey(String),
    #[error("{0}")]
    Io(#[from] io::Error),
}

impl DbError {
    fn msg(s: &str) -> Self {
        DbError::Error(s.to_string())
    }
    fn format(s: &str) -> Self {
        DbError::InvalidFormat(s.to_string())
    }
}

fn invalid_format() -> DbError {
    DbError::format(E_INVALID_FORMAT)
}

fn invalid_key() -> DbError {
    DbError::InvalidKey(trl("Master key is invalid or file contents have been modified"))
}

/// Field types (`PasswDbEntry::FieldType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldType {
    Title = 0,
    UserName = 1,
    Password = 2,
    Url = 3,
    Keyword = 4,
    Notes = 5,
    KeyValueList = 6,
    Tags = 7,
    CreationTime = 8,
    ModificationTime = 9,
    PasswChangeTime = 10,
    PasswExpiryDate = 11,
    PasswHistory = 12,
}

pub const NUM_FIELDS: usize = 13;
pub const NUM_STRING_FIELDS: usize = 8;
const FIELD_END: u8 = 0xff;

/// Field names as stored in the file (`GetFieldName()`).
pub const FIELD_NAMES: [&str; NUM_FIELDS] = [
    "Title",
    "UserName",
    "Password",
    "URL",
    "Keyword",
    "Notes",
    "KeyValueList",
    "Tags",
    "CreationTime",
    "ModificationTime",
    "PasswChangeTime",
    "PasswExpiryDate",
    "PasswHistory",
];

/// A tag, ordered like `std::set<SecureWString>` in the original: by buffer
/// size (UTF-16 length + 1) first, then byte-wise on the UTF-16LE data.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Tag(pub String);

impl Tag {
    fn key(&self) -> (usize, Vec<u8>) {
        let units: Vec<u16> = self.0.encode_utf16().collect();
        (units.len(), units.iter().flat_map(|u| u.to_le_bytes()).collect())
    }
}

impl Ord for Tag {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key().cmp(&other.key())
    }
}

impl PartialOrd for Tag {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Drop for Tag {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// Password history of an entry (`PasswDbEntry::PasswHistory`): time of the
/// password change (`FILETIME`, local time) and the old password.
#[derive(Clone, Debug)]
pub struct PasswHistory {
    history: Vec<(u64, Zeroizing<String>)>,
    max_size: u32,
    active: bool,
}

impl Default for PasswHistory {
    fn default() -> Self {
        PasswHistory { history: Vec::new(), max_size: 1, active: false }
    }
}

impl PasswHistory {
    pub fn new(max_size: u32, active: bool) -> Self {
        PasswHistory { history: Vec::new(), max_size: max_size.max(1), active }
    }
    pub fn is_empty(&self) -> bool {
        !self.active && self.history.is_empty()
    }
    pub fn len(&self) -> usize {
        self.history.len()
    }
    pub fn max_size(&self) -> u32 {
        self.max_size
    }
    pub fn active(&self) -> bool {
        self.active
    }
    pub fn set_max_size(&mut self, n: u32) {
        self.max_size = n.max(1);
        self.history.truncate(self.max_size as usize);
    }
    pub fn set_active(&mut self, active: bool) {
        self.active = active;
    }
    pub fn iter(&self) -> impl Iterator<Item = &(u64, Zeroizing<String>)> {
        self.history.iter()
    }
    pub fn clear(&mut self) {
        self.history.clear();
    }
    /// `AdoptFrom()`: take over the settings of another history.
    pub fn adopt_from(&mut self, src: &PasswHistory) {
        self.set_active(src.active);
        if src.history.is_empty() {
            self.clear();
        }
        self.set_max_size(src.max_size);
    }
    /// `AddEntry()`: only if active and the password is not empty.
    pub fn add_entry(&mut self, time: u64, passw: Zeroizing<String>, to_front: bool) {
        if self.active && !passw.is_empty() {
            if to_front {
                self.history.insert(0, (time, passw));
            } else {
                self.history.push((time, passw));
            }
            self.history.truncate(self.max_size as usize);
        }
    }
}

/// Database entry (`PasswDbEntry`).
pub struct PasswDbEntry {
    id: u32,
    index: u32,
    /// Title, user name, password (plaintext only if plaintext mode is on),
    /// URL, keyword, notes, key-value list (`k=v,…`), tags (`a,b,…`).
    pub strings: [Zeroizing<String>; NUM_STRING_FIELDS],
    pub creation_time: u64,
    pub modification_time: u64,
    pub passw_change_time: u64,
    creation_time_set: bool,
    modification_time_set: bool,
    passw_change_time_set: bool,
    /// `(year << 16) | (month << 8) | day`, 0 = none.
    pub passw_expiry_date: u32,
    pub user_flags: u32,
    pub user_tag: i32,
    enc_passw: Zeroizing<Vec<u8>>,
    passw_hash: [u8; 20],
    key_value_list: Vec<(Zeroizing<String>, Zeroizing<String>)>,
    tags: BTreeSet<Tag>,
    passw_history: PasswHistory,
}

impl PasswDbEntry {
    fn new(id: u32, index: u32, set_time_stamps: bool, max_history: u32, history_active: bool) -> Self {
        let now = if set_time_stamps { crate::random::hrtimer::local_filetime_now() } else { 0 };
        PasswDbEntry {
            id,
            index,
            strings: Default::default(),
            creation_time: now,
            modification_time: now,
            passw_change_time: 0,
            creation_time_set: set_time_stamps,
            modification_time_set: set_time_stamps,
            passw_change_time_set: false,
            passw_expiry_date: 0,
            user_flags: 0,
            user_tag: 0,
            enc_passw: Zeroizing::new(Vec::new()),
            passw_hash: [0; 20],
            key_value_list: Vec::new(),
            tags: BTreeSet::new(),
            passw_history: PasswHistory::new(max_history, history_active),
        }
    }

    pub fn id(&self) -> u32 {
        self.id
    }
    pub fn index(&self) -> u32 {
        self.index
    }
    pub fn string(&self, f: FieldType) -> &str {
        &self.strings[f as usize]
    }
    pub fn set_string(&mut self, f: FieldType, s: &str) {
        self.strings[f as usize] = Zeroizing::new(s.to_string());
    }
    pub fn is_passw_empty(&self) -> bool {
        self.enc_passw.is_empty()
    }
    pub fn has_plaintext_passw(&self) -> bool {
        !self.enc_passw.is_empty() && !self.strings[FieldType::Password as usize].is_empty()
    }
    /// `UpdateModificationTime()`.
    pub fn update_modification_time(&mut self, passw_changed: bool) {
        self.modification_time = crate::random::hrtimer::local_filetime_now();
        self.modification_time_set = true;
        if passw_changed {
            self.passw_change_time = self.modification_time;
            self.passw_change_time_set = true;
        }
    }
    pub fn key_value_list(&self) -> &[(Zeroizing<String>, Zeroizing<String>)] {
        &self.key_value_list
    }
    pub fn set_key_value_list(&mut self, list: Vec<(Zeroizing<String>, Zeroizing<String>)>) {
        self.key_value_list = list;
        self.update_key_value_string();
    }
    /// `GetKeyValue()`: case-insensitive key lookup (`_wcsicmp`).
    pub fn key_value(&self, key: &str) -> Option<&str> {
        self.key_value_list.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str())
    }
    /// `SetKeyValue()`.
    pub fn set_key_value(&mut self, key: &str, value: &str) {
        if key.is_empty() || value.is_empty() {
            return;
        }
        if let Some(kv) = self.key_value_list.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
            kv.1 = Zeroizing::new(value.to_string());
            return;
        }
        self.key_value_list.push((Zeroizing::new(key.to_string()), Zeroizing::new(value.to_string())));
    }
    /// `GetKeyValueListAsString(sep)`.
    pub fn key_value_list_as_string(&self, sep: char) -> Zeroizing<String> {
        let mut s = Zeroizing::new(String::new());
        for (k, v) in &self.key_value_list {
            if !s.is_empty() {
                s.push(sep);
            }
            s.push_str(k);
            s.push('=');
            s.push_str(v);
        }
        s
    }
    pub fn clear_key_value_list(&mut self) {
        self.key_value_list.clear();
        self.strings[FieldType::KeyValueList as usize] = Zeroizing::new(String::new());
    }
    /// `UpdateKeyValueString()`: display string with ',' separators.
    pub fn update_key_value_string(&mut self) {
        self.strings[FieldType::KeyValueList as usize] = self.key_value_list_as_string(',');
    }
    fn parse_key_value_list(&mut self, list: &str) {
        if utf16_size(list) < 4 {
            return;
        }
        for item in list.split('\n').filter(|s| !s.is_empty()) {
            let units: Vec<char> = item.chars().collect();
            if let Some(pos) = units.iter().position(|&c| c == '=') {
                if pos > 0 && pos < units.len() - 1 {
                    let k: String = units[..pos].iter().collect();
                    let v: String = units[pos + 1..].iter().collect();
                    self.key_value_list.push((Zeroizing::new(k), Zeroizing::new(v)));
                }
            }
        }
    }
    pub fn tags(&self) -> &BTreeSet<Tag> {
        &self.tags
    }
    pub fn set_tags(&mut self, tags: BTreeSet<Tag>) {
        self.tags = tags;
        self.update_tags_string();
    }
    pub fn check_tag(&self, tag: &str) -> bool {
        self.tags.contains(&Tag(tag.to_string()))
    }
    /// `AddTag()`: `false` if the tag already exists.
    pub fn add_tag(&mut self, tag: &str) -> bool {
        self.tags.insert(Tag(tag.to_string()))
    }
    /// `GetTagsAsString(sep)`.
    pub fn tags_as_string(&self, sep: char) -> Zeroizing<String> {
        let mut s = Zeroizing::new(String::new());
        for t in &self.tags {
            if !s.is_empty() {
                s.push(sep);
            }
            s.push_str(&t.0);
        }
        s
    }
    pub fn update_tags_string(&mut self) {
        self.strings[FieldType::Tags as usize] = self.tags_as_string(',');
    }
    pub fn clear_tags(&mut self) {
        self.tags.clear();
        self.strings[FieldType::Tags as usize] = Zeroizing::new(String::new());
    }
    fn parse_tag_list(&mut self, list: &str) {
        if utf16_size(list) < 2 {
            return;
        }
        for t in list.split('\n').filter(|s| !s.is_empty()) {
            self.tags.insert(Tag(t.to_string()));
        }
    }
    pub fn passw_history(&self) -> &PasswHistory {
        &self.passw_history
    }
    pub fn passw_history_mut(&mut self) -> &mut PasswHistory {
        &mut self.passw_history
    }
    /// `AddCurrentPasswToHistory()`.
    pub fn add_current_passw_to_history(&mut self, passw: Zeroizing<String>) {
        let t = self.passw_change_time;
        self.passw_history.add_entry(t, passw, true);
    }
    /// Creation time as text (`CreationTimeString`).
    pub fn creation_time_string(&self) -> String {
        if self.creation_time_set { crate::util::time_stamp_to_string(self.creation_time) } else { String::new() }
    }
    pub fn modification_time_string(&self) -> String {
        if self.modification_time_set { crate::util::time_stamp_to_string(self.modification_time) } else { String::new() }
    }
    pub fn passw_change_time_string(&self) -> String {
        if self.passw_change_time_set { crate::util::time_stamp_to_string(self.passw_change_time) } else { String::new() }
    }
    pub fn passw_expiry_date_string(&self) -> String {
        expiry_date_to_string(self.passw_expiry_date)
    }
}

/// Size of a `SecureWString` holding `s` (UTF-16 units + terminator).
fn utf16_size(s: &str) -> usize {
    s.encode_utf16().count() + 1
}

/// `DecodeExpiryDate()`.
pub fn decode_expiry_date(date: u32) -> Option<(i32, u32, u32)> {
    let year = (date >> 16) as i32;
    let month = (date >> 8) & 0xff;
    let day = date & 0xff;
    (year > 0 && (1..=12).contains(&month) && (1..=31).contains(&day)).then_some((year, month, day))
}

/// `EncodeExpiryDate()`.
pub fn encode_expiry_date(year: i32, month: u32, day: u32) -> u32 {
    ((year as u32) << 16) | (month << 8) | day
}

fn is_valid_calendar_date(y: i32, m: u32, d: u32) -> bool {
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let dim = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1601..=9999).contains(&y) && d >= 1 && d <= dim
}

/// `ExpiryDateToString()`: short date in the user's locale, empty if the
/// date is invalid (`GetDateFormatEx` fails).
pub fn expiry_date_to_string(date: u32) -> String {
    match decode_expiry_date(date) {
        Some((y, m, d)) if date != 0 && is_valid_calendar_date(y, m, d) => crate::util::format_short_date(y, m, d),
        _ => String::new(),
    }
}

/// Plaintext file header.
struct FileHeader {
    magic: u32,
    header_size: u8,
    version: u16,
    flags: u32,
    cipher_type: u8,
    hash_type: u8,
    kdf_type: u8,
    kdf_iterations: u32,
}

impl FileHeader {
    fn parse(b: &[u8]) -> Self {
        FileHeader {
            magic: u32::from_le_bytes(b[0..4].try_into().unwrap()),
            header_size: b[4],
            version: u16::from_le_bytes(b[5..7].try_into().unwrap()),
            flags: u32::from_le_bytes(b[7..11].try_into().unwrap()),
            cipher_type: b[11],
            hash_type: b[12],
            kdf_type: b[13],
            kdf_iterations: u32::from_le_bytes(b[14..18].try_into().unwrap()),
        }
    }
    fn to_bytes(&self) -> [u8; FILE_HEADER_SIZE] {
        let mut b = [0u8; FILE_HEADER_SIZE];
        b[0..4].copy_from_slice(&self.magic.to_le_bytes());
        b[4] = self.header_size;
        b[5..7].copy_from_slice(&self.version.to_le_bytes());
        b[7..11].copy_from_slice(&self.flags.to_le_bytes());
        b[11] = self.cipher_type;
        b[12] = self.hash_type;
        b[13] = self.kdf_type;
        b[14..18].copy_from_slice(&self.kdf_iterations.to_le_bytes());
        b
    }
}

/// Encrypted inner header (`PasswDbHeader`).
#[derive(Default)]
struct DbHeader {
    magic: u32,
    header_size: u16,
    flags: u32,
    num_var_params: u8,
    num_fields: u8,
    num_entries: u32,
    compression_algo: u8,
    compression_level: u8,
    uncompressed_size: u32,
    compressed_size: u32,
}

impl DbHeader {
    fn parse(b: &[u8]) -> Self {
        DbHeader {
            magic: u32::from_le_bytes(b[0..4].try_into().unwrap()),
            header_size: u16::from_le_bytes(b[4..6].try_into().unwrap()),
            flags: u32::from_le_bytes(b[6..10].try_into().unwrap()),
            num_var_params: b[10],
            num_fields: b[11],
            num_entries: u32::from_le_bytes(b[12..16].try_into().unwrap()),
            compression_algo: b[16],
            compression_level: b[17],
            uncompressed_size: u32::from_le_bytes(b[18..22].try_into().unwrap()),
            compressed_size: u32::from_le_bytes(b[22..26].try_into().unwrap()),
        }
    }
    fn to_bytes(&self) -> [u8; DB_HEADER_SIZE] {
        let mut b = [0u8; DB_HEADER_SIZE];
        b[0..4].copy_from_slice(&self.magic.to_le_bytes());
        b[4..6].copy_from_slice(&self.header_size.to_le_bytes());
        b[6..10].copy_from_slice(&self.flags.to_le_bytes());
        b[10] = self.num_var_params;
        b[11] = self.num_fields;
        b[12..16].copy_from_slice(&self.num_entries.to_le_bytes());
        b[16] = self.compression_algo;
        b[17] = self.compression_level;
        b[18..22].copy_from_slice(&self.uncompressed_size.to_le_bytes());
        b[22..26].copy_from_slice(&self.compressed_size.to_le_bytes());
        b
    }
}

/// Reader over the decrypted (and decompressed) data stream.
struct StreamReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl StreamReader<'_> {
    fn read_type<const N: usize>(&mut self) -> Result<[u8; N], DbError> {
        if self.pos + N > self.buf.len() {
            return Err(invalid_format());
        }
        let a: [u8; N] = self.buf[self.pos..self.pos + N].try_into().unwrap();
        self.pos += N;
        Ok(a)
    }
    fn read_u32(&mut self) -> Result<u32, DbError> {
        Ok(u32::from_le_bytes(self.read_type::<4>()?))
    }
    fn read_field_size(&mut self) -> Result<usize, DbError> {
        Ok(self.read_u32()? as usize)
    }
    fn read_field_index(&mut self) -> Result<u8, DbError> {
        Ok(self.read_type::<1>()?[0])
    }
    /// `ReadField<T>()`: size prefix must equal `N`.
    fn read_field<const N: usize>(&mut self) -> Result<[u8; N], DbError> {
        if self.read_field_size()? != N {
            return Err(invalid_format());
        }
        if self.pos + N > self.buf.len() {
            return Err(invalid_format());
        }
        let a: [u8; N] = self.buf[self.pos..self.pos + N].try_into().unwrap();
        self.pos += N;
        Ok(a)
    }
    fn read_ansi_string_raw(&mut self) -> Result<(Zeroizing<Vec<u8>>, usize), DbError> {
        let size = self.read_field_size()?;
        if size == 0 {
            return Ok((Zeroizing::new(Vec::new()), 0));
        }
        if self.pos.checked_add(size).is_none_or(|e| e > self.buf.len()) {
            return Err(invalid_format());
        }
        let s = &self.buf[self.pos..self.pos + size];
        self.pos += size;
        // AssignStr() + later C-string use: stop at the first NUL
        let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
        Ok((Zeroizing::new(s[..end].to_vec()), size))
    }
    fn read_ansi_string(&mut self) -> Result<Zeroizing<Vec<u8>>, DbError> {
        Ok(self.read_ansi_string_raw()?.0)
    }
    fn read_string(&mut self) -> Result<Zeroizing<String>, DbError> {
        let b = self.read_ansi_string()?;
        Ok(Zeroizing::new(String::from_utf8_lossy(&b).into_owned()))
    }
    fn skip_field(&mut self) -> Result<(), DbError> {
        let n = self.read_field_size()?;
        self.pos = self.pos.checked_add(n).ok_or_else(invalid_format)?;
        if self.pos > self.buf.len() {
            return Err(invalid_format());
        }
        Ok(())
    }
}

/// Writer for the data stream (`Write`, `WriteField`, `WriteString`).
struct StreamWriter {
    buf: Zeroizing<Vec<u8>>,
}

impl StreamWriter {
    fn write(&mut self, b: &[u8]) -> Result<(), DbError> {
        self.buf.extend_from_slice(b);
        if self.buf.len() as u64 > MAX_FILE_SIZE - 1024 {
            return Err(DbError::msg("Database size exceeds file size limit"));
        }
        Ok(())
    }
    fn write_field_buf(&mut self, data: &[u8], index: Option<u8>) -> Result<(), DbError> {
        if let Some(i) = index {
            self.write(&[i])?;
        }
        self.write(&(data.len() as u32).to_le_bytes())?;
        self.write(data)
    }
    /// `WriteString(const char*, len, index)`: with an index, empty strings
    /// are not written at all.
    fn write_ansi(&mut self, data: &[u8], index: Option<u8>) -> Result<(), DbError> {
        if index.is_none() || !data.is_empty() { self.write_field_buf(data, index) } else { Ok(()) }
    }
    /// `WriteString(const SecureWString&, index)` (UTF-8).
    fn write_string(&mut self, s: &str, index: Option<u8>) -> Result<(), DbError> {
        if !s.is_empty() {
            self.write_ansi(s.as_bytes(), index)
        } else if index.is_none() {
            self.write_field_buf(&[], None)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DbOpenState {
    Closed,
    Incomplete,
    Open,
}

/// Keys kept in a locked memory page while a database is open
/// (`m_pSecMem` in the original).
struct SecMem {
    page: LockedPage,
    off: usize,
}

const SM_MEMKEY: usize = 0;
const SM_MEMSALT: usize = SM_MEMKEY + SECMEM_KEY_LENGTH;
const SM_DBKEY: usize = SM_MEMSALT + SECMEM_SALT_LENGTH;
const SM_DBSALT: usize = SM_DBKEY + DB_KEY_LENGTH;
const SM_RECOVERY: usize = SM_DBSALT + DB_SALT_LENGTH;
const SM_SIZE: usize = SM_RECOVERY + DB_RECOVERY_KEY_BLOCK_LENGTH;

impl SecMem {
    fn new() -> Self {
        let mut page = LockedPage::new();
        fast_fill(page.as_mut_slice());
        let off = crate::random::fprng_rand_range(2048) as usize;
        SecMem { page, off }
    }
    fn get(&self, o: usize, len: usize) -> &[u8] {
        &self.page.as_slice()[self.off + o..self.off + o + len]
    }
    fn get_mut(&mut self, o: usize, len: usize) -> &mut [u8] {
        let off = self.off;
        &mut self.page.as_mut_slice()[off + o..off + o + len]
    }
    fn mem_key(&self) -> &[u8] {
        self.get(SM_MEMKEY, SECMEM_KEY_LENGTH)
    }
    fn mem_salt(&self) -> &[u8] {
        self.get(SM_MEMSALT, SECMEM_SALT_LENGTH)
    }
    fn db_key(&self) -> &[u8] {
        self.get(SM_DBKEY, DB_KEY_LENGTH)
    }
    fn db_key_mut(&mut self) -> &mut [u8] {
        self.get_mut(SM_DBKEY, DB_KEY_LENGTH)
    }
    fn db_salt(&self) -> &[u8] {
        self.get(SM_DBSALT, DB_SALT_LENGTH)
    }
    fn recovery(&self) -> &[u8] {
        self.get(SM_RECOVERY, DB_RECOVERY_KEY_BLOCK_LENGTH)
    }
    fn recovery_mut(&mut self) -> &mut [u8] {
        self.get_mut(SM_RECOVERY, DB_RECOVERY_KEY_BLOCK_LENGTH)
    }
}

const _: () = assert!(SM_SIZE + 2048 <= crate::secure_mem::PAGE_SIZE);

/// The password database.
pub struct PasswDatabase {
    entries: Vec<PasswDbEntry>,
    last_version: u16,
    cipher_type: u8,
    kdf_iterations: u32,
    next_entry_id: u32,
    sec: Option<SecMem>,
    file: Option<File>,
    pub default_user_name: Zeroizing<String>,
    pub passw_format_seq: Zeroizing<String>,
    pub description: Zeroizing<String>,
    pub default_passw_expiry_days: u32,
    pub default_max_passw_history_size: u32,
    pub master_passw_expiry_date: u32,
    plaintext_passw: bool,
    state: DbOpenState,
    recovery_key: bool,
    pub compressed: bool,
    pub passw_utf8: bool,
    pub compression_level: i32,
}

impl Default for PasswDatabase {
    fn default() -> Self {
        Self::new()
    }
}

fn hkdf_extract_256(key: &[u8], salt: &[u8]) -> Zeroizing<[u8; 32]> {
    Zeroizing::new(hmac_sha256(salt, key))
}

fn hkdf_extract_512(key: &[u8], salt: &[u8]) -> Zeroizing<[u8; 64]> {
    Zeroizing::new(hmac_sha512(salt, key))
}

fn create_cipher(cipher_type: u8, key: &[u8]) -> Result<SymmetricCipher, DbError> {
    match cipher_type {
        CIPHER_AES256 => Ok(SymmetricCipher::aes_cbc(key)),
        CIPHER_CHACHA20 => Ok(SymmetricCipher::chacha20(key)),
        _ => Err(DbError::msg("Cipher not supported")),
    }
}

/// Lock a file with `flock` (shared or exclusive), non-blocking.
fn flock(f: &File, exclusive: bool) -> bool {
    let op = if exclusive { libc::LOCK_EX } else { libc::LOCK_SH } | libc::LOCK_NB;
    // SAFETY: valid file descriptor.
    unsafe { libc::flock(f.as_raw_fd(), op) == 0 }
}

impl PasswDatabase {
    pub fn new() -> Self {
        PasswDatabase {
            entries: Vec::new(),
            last_version: 0,
            cipher_type: CIPHER_AES256,
            kdf_iterations: KEY_HASH_ITERATIONS,
            next_entry_id: 0,
            sec: None,
            file: None,
            default_user_name: Zeroizing::new(String::new()),
            passw_format_seq: Zeroizing::new(String::new()),
            description: Zeroizing::new(String::new()),
            default_passw_expiry_days: 0,
            default_max_passw_history_size: 0,
            master_passw_expiry_date: 0,
            plaintext_passw: false,
            state: DbOpenState::Closed,
            recovery_key: false,
            compressed: false,
            passw_utf8: true,
            compression_level: 0,
        }
    }

    /// `Close()`.
    pub fn close(&mut self) {
        self.sec = None;
        self.cipher_type = CIPHER_AES256;
        self.kdf_iterations = KEY_HASH_ITERATIONS;
        self.default_passw_expiry_days = 0;
        self.master_passw_expiry_date = 0;
        self.entries.clear();
        self.file = None;
        self.next_entry_id = 0;
        self.default_user_name = Zeroizing::new(String::new());
        self.state = DbOpenState::Closed;
        self.recovery_key = false;
        self.compressed = false;
        self.passw_utf8 = true;
        self.compression_level = 0;
    }

    /// `Initialize()`: session keys, new database salt, database key.
    fn initialize(&mut self, key: &[u8]) {
        let mut sec = SecMem::new();
        {
            let mut pool = RandomPool::global();
            let mut mk = Zeroizing::new([0u8; SECMEM_KEY_LENGTH]);
            pool.get_data(&mut mk[..]);
            sec.get_mut(SM_MEMKEY, SECMEM_KEY_LENGTH).copy_from_slice(&mk[..]);
            let mut salt = Zeroizing::new([0u8; SECMEM_SALT_LENGTH]);
            pool.get_data(&mut salt[..]);
            sec.get_mut(SM_MEMSALT, SECMEM_SALT_LENGTH).copy_from_slice(&salt[..]);
            let mut dbsalt = Zeroizing::new([0u8; DB_SALT_LENGTH]);
            pool.get_data(&mut dbsalt[..]);
            sec.get_mut(SM_DBSALT, DB_SALT_LENGTH).copy_from_slice(&dbsalt[..]);
            pool.flush();
        }
        if self.recovery_key {
            sec.db_key_mut().copy_from_slice(&key[..DB_KEY_LENGTH]);
        } else {
            let salt = sec.db_salt().to_vec();
            let k = pbkdf2_256bit(key, &salt, self.kdf_iterations, None);
            sec.db_key_mut().copy_from_slice(&k[..]);
        }
        self.sec = Some(sec);
    }

    fn check_db_not_open(&mut self) -> Result<(), DbError> {
        match self.state {
            DbOpenState::Incomplete => {
                self.close();
                Ok(())
            }
            DbOpenState::Open => Err(DbError::msg("Database already opened")),
            DbOpenState::Closed => Ok(()),
        }
    }

    fn check_db_open(&self) -> Result<(), DbError> {
        if self.is_open() { Ok(()) } else { Err(DbError::msg("Database not open")) }
    }

    pub fn is_open(&self) -> bool {
        self.state == DbOpenState::Open
    }

    /// Create a new database with the given master key (`New()`).
    pub fn create(&mut self, key: &[u8]) -> Result<(), DbError> {
        self.check_db_not_open()?;
        if key.is_empty() {
            return Err(DbError::msg("Specified \"key\" parameter is empty"));
        }
        self.initialize(key);
        self.last_version = VERSION;
        self.state = DbOpenState::Open;
        Ok(())
    }

    /// Read only the plaintext header (`Open(…, blReadHeaderOnly = true)`):
    /// version, cipher, KDF iterations, recovery key and UTF-8 flags.
    pub fn read_header(&mut self, path: &Path) -> Result<(), DbError> {
        self.open_impl(&[], path, true)
    }

    /// Open a database file with the given master key (`Open()`).
    pub fn open(&mut self, key: &[u8], path: &Path) -> Result<(), DbError> {
        self.open_impl(key, path, false)
    }

    fn open_impl(&mut self, key: &[u8], path: &Path, header_only: bool) -> Result<(), DbError> {
        self.check_db_not_open()?;
        if !header_only && key.is_empty() {
            return Err(DbError::msg("Specified \"key\" parameter is empty"));
        }
        self.state = DbOpenState::Incomplete;

        let mut file = File::open(path)?;
        let size = file.metadata()?.len();
        if size > MAX_FILE_SIZE {
            return Err(DbError::msg("File too large"));
        }
        if size < (FILE_HEADER_SIZE + DB_SALT_LENGTH + 8 + 16 + 32) as u64 {
            return Err(DbError::msg("Invalid file size"));
        }
        let mut data = Zeroizing::new(Vec::with_capacity(size as usize));
        file.read_to_end(&mut data)?;
        let fh = FileHeader::parse(&data[..FILE_HEADER_SIZE]);
        if fh.magic != HEADER_MAGIC {
            return Err(DbError::format("Unknown file format"));
        }
        if (fh.header_size as usize) < FILE_HEADER_SIZE || (fh.header_size as usize) > data.len() {
            return Err(invalid_format());
        }
        self.last_version = fh.version;
        if fh.version < 0x100 {
            return Err(DbError::format("Invalid version number"));
        }
        if fh.cipher_type > CIPHER_CHACHA20 {
            return Err(DbError::format("Encryption algorithm not supported"));
        }
        if fh.hash_type > HASH_SHA512 {
            return Err(DbError::format("Hash algorithm not supported"));
        }
        if fh.kdf_type > KDF_PBKDF2_SHA256 {
            return Err(DbError::format("Key derivation function not supported"));
        }
        if fh.kdf_iterations == 0 {
            return Err(DbError::format("Invalid number of KDF iterations"));
        }
        self.cipher_type = fh.cipher_type;
        self.kdf_iterations = fh.kdf_iterations;
        self.recovery_key = fh.version >= 0x103 && fh.flags & FH_FLAG_RECOVERY_KEY != 0;
        self.passw_utf8 = fh.version >= 0x107 && fh.flags & FH_FLAG_PASSW_UTF8 != 0;

        if header_only {
            return Ok(());
        }

        let content = &data[fh.header_size as usize..];
        let file_size = content.len();
        let new_auth = fh.version >= 0x108;

        let mut master_key: Zeroizing<Vec<u8>> = Zeroizing::new(Vec::new());
        let mut crypt_param_len = 0usize;
        let mut header = DbHeader::default();
        let mut buf: Zeroizing<Vec<u8>> = Zeroizing::new(Vec::new());
        let mut buf_pos = 0usize;

        for key_num in 0..2 {
            buf = Zeroizing::new(content.to_vec());
            let mut salt = Zeroizing::new([0u8; DB_SALT_LENGTH]);
            let derived: Zeroizing<[u8; 32]>;

            if self.recovery_key {
                buf_pos = (DB_SALT_LENGTH + DB_KEY_LENGTH) * key_num;
                if file_size < DB_RECOVERY_KEY_BLOCK_LENGTH
                    || buf_pos + DB_SALT_LENGTH + DB_KEY_LENGTH > file_size
                {
                    return Err(invalid_format());
                }
                derived = pbkdf2_256bit(key, &buf[buf_pos..buf_pos + DB_SALT_LENGTH], fh.kdf_iterations, None);
                let mut cipher = create_cipher(fh.cipher_type, &derived[..])?;
                cipher.set_iv(&buf[buf_pos..]);
                let mut mk = Zeroizing::new(buf[buf_pos + DB_SALT_LENGTH..buf_pos + DB_SALT_LENGTH + DB_KEY_LENGTH].to_vec());
                cipher.decrypt(&mut mk);
                master_key = mk;
                buf_pos = DB_RECOVERY_KEY_BLOCK_LENGTH;
            } else {
                if file_size < DB_SALT_LENGTH {
                    return Err(invalid_format());
                }
                derived = pbkdf2_256bit(key, &buf[..DB_SALT_LENGTH], fh.kdf_iterations, None);
            }

            if new_auth || !self.recovery_key {
                if buf_pos + DB_SALT_LENGTH > file_size {
                    return Err(invalid_format());
                }
                salt.copy_from_slice(&buf[buf_pos..buf_pos + DB_SALT_LENGTH]);
                buf_pos += DB_SALT_LENGTH;
            }

            let key_src: &[u8] = if self.recovery_key { &master_key } else { &derived[..] };
            let cipher_key: Zeroizing<Vec<u8>> = if new_auth {
                Zeroizing::new(hkdf_extract_256(key_src, &salt[..]).to_vec())
            } else {
                Zeroizing::new(key_src.to_vec())
            };
            let mut cipher = create_cipher(fh.cipher_type, &cipher_key)?;
            if buf_pos + cipher.iv_size() > file_size {
                return Err(invalid_format());
            }
            cipher.set_iv(&buf[buf_pos..]);
            buf_pos += cipher.iv_size();
            crypt_param_len = buf_pos;

            let mac_key: Zeroizing<Vec<u8>> = if new_auth {
                Zeroizing::new(hkdf_extract_512(key_src, &salt[..]).to_vec())
            } else {
                Zeroizing::new(key_src.to_vec())
            };
            let mut hmgen = Some(if fh.hash_type == HASH_SHA256 {
                HmacAny::Sha256(HmacSha256::new(&mac_key))
            } else {
                HmacAny::Sha512(HmacSha512::new(&mac_key))
            });
            let hmac_len = hmgen.as_ref().unwrap().len();
            if file_size < crypt_param_len + hmac_len {
                return Err(invalid_format());
            }
            let mut hmac = Zeroizing::new(vec![0u8; hmac_len]);
            if fh.version >= 0x101 {
                hmac.copy_from_slice(&buf[file_size - hmac_len..]);
            }

            let mut check_hmac: Option<Zeroizing<Vec<u8>>> = None;
            let mut can_decrypt = true;
            if new_auth {
                let mut h = hmgen.take().unwrap();
                h.update(&buf[..file_size - hmac_len]);
                let c = h.finish();
                if c[..] != hmac[..] {
                    can_decrypt = false;
                }
                check_hmac = Some(c);
            }

            if can_decrypt {
                let aligned = align_to_block_size(DB_HEADER_SIZE, cipher.block_size());
                if buf_pos + aligned > file_size {
                    return Err(invalid_format());
                }
                cipher.decrypt(&mut buf[buf_pos..buf_pos + aligned]);
                header = DbHeader::parse(&buf[buf_pos..buf_pos + DB_HEADER_SIZE]);
                if header.magic != HEADER_MAGIC {
                    if self.recovery_key && key_num == 0 {
                        continue;
                    }
                    return Err(invalid_key());
                }
                buf_pos += aligned;
                if fh.version >= 0x101 {
                    let rest = file_size as isize - buf_pos as isize - hmac_len as isize;
                    if rest > 0 {
                        let rest = rest as usize;
                        if cipher.requires_alignment() && rest % 16 != 0 {
                            return Err(invalid_key());
                        }
                        cipher.decrypt(&mut buf[buf_pos..buf_pos + rest]);
                    }
                } else {
                    if file_size > buf_pos {
                        let n = file_size - buf_pos;
                        if cipher.requires_alignment() && n % 16 != 0 {
                            return Err(invalid_key());
                        }
                        cipher.decrypt(&mut buf[buf_pos..file_size]);
                    }
                    hmac.copy_from_slice(&buf[file_size - hmac_len..]);
                }
                if let Some(mut h) = hmgen.take() {
                    let len = file_size as isize - crypt_param_len as isize - hmac_len as isize;
                    if len > 0 {
                        h.update(&buf[crypt_param_len..crypt_param_len + len as usize]);
                    }
                    check_hmac = Some(h.finish());
                }
            }

            if check_hmac.as_deref().map(|c| &c[..]) != Some(&hmac[..]) {
                if self.recovery_key && key_num == 0 {
                    continue;
                }
                return Err(invalid_key());
            }
            break;
        }

        let mut pos = crypt_param_len + header.header_size as usize;
        let key_for_init: Zeroizing<Vec<u8>> =
            if self.recovery_key { master_key.clone() } else { Zeroizing::new(key.to_vec()) };
        self.initialize(&key_for_init);
        if self.recovery_key {
            let rb = buf[..DB_RECOVERY_KEY_BLOCK_LENGTH].to_vec();
            self.sec.as_mut().unwrap().recovery_mut().copy_from_slice(&rb);
        }

        let stream: Zeroizing<Vec<u8>>;
        if fh.version >= 0x104 && header.compression_algo != 0 {
            if header.compression_algo > COMPRESSION_DEFLATE {
                return Err(DbError::msg("Compression algorithm not supported"));
            }
            let end = (pos + header.compressed_size as usize).min(buf.len());
            if pos > end {
                return Err(DbError::msg("Error while decompressing data"));
            }
            let mut out = Zeroizing::new(Vec::with_capacity(header.uncompressed_size as usize));
            let mut dec = ZlibDecoder::new(&buf[pos..end]);
            let ok = (&mut dec).take(header.uncompressed_size as u64 + 1).read_to_end(&mut out).is_ok();
            if !ok || out.len() != header.uncompressed_size as usize {
                return Err(DbError::msg("Error while decompressing data"));
            }
            stream = out;
            pos = 0;
            self.compressed = true;
            self.compression_level = header.compression_level as i32;
        } else {
            stream = Zeroizing::new(buf[..].to_vec());
            self.compressed = false;
            self.compression_level = 0;
        }

        let mut r = StreamReader { buf: &stream, pos };

        if fh.version >= 0x102 {
            for _ in 0..header.num_var_params {
                let flag = r.read_u32()?;
                if header.flags & FLAG_DEFAULT_USER_NAME != 0 && flag == FLAG_DEFAULT_USER_NAME {
                    self.default_user_name = r.read_string()?;
                } else if header.flags & FLAG_PASSW_FORMAT_SEQ != 0 && flag == FLAG_PASSW_FORMAT_SEQ {
                    self.passw_format_seq = r.read_string()?;
                } else if header.flags & FLAG_PASSW_EXPIRY_DAYS != 0 && flag == FLAG_PASSW_EXPIRY_DAYS {
                    let v = if fh.version >= 0x108 {
                        u32::from_le_bytes(r.read_field::<4>()?)
                    } else {
                        r.read_u32()?
                    };
                    self.default_passw_expiry_days = v.min(3650);
                } else if header.flags & FLAG_DEFAULT_PASSW_HISTORY_SIZE != 0 && flag == FLAG_DEFAULT_PASSW_HISTORY_SIZE {
                    self.default_max_passw_history_size =
                        if fh.version >= 0x108 { r.read_field::<1>()?[0] } else { r.read_type::<1>()?[0] } as u32;
                } else if header.flags & FLAG_MASTER_PASSW_EXPIRY != 0 && flag == FLAG_MASTER_PASSW_EXPIRY {
                    self.master_passw_expiry_date = if fh.version >= 0x108 {
                        u32::from_le_bytes(r.read_field::<4>()?)
                    } else {
                        r.read_u32()?
                    };
                } else if header.flags & FLAG_DESCRIPTION != 0 && flag == FLAG_DESCRIPTION {
                    self.description = r.read_string()?;
                } else {
                    r.skip_field()?;
                }
            }
        } else {
            for _ in 0..header.num_var_params {
                let name = r.read_ansi_string()?;
                let name = String::from_utf8_lossy(&name).to_string();
                if header.flags & FLAG_DEFAULT_USER_NAME != 0 && name.eq_ignore_ascii_case(PARAMSTR_DEFAULT_USER_NAME) {
                    self.default_user_name = r.read_string()?;
                } else if header.flags & FLAG_PASSW_FORMAT_SEQ != 0
                    && name.eq_ignore_ascii_case(PARAMSTR_PASSW_FORMAT_SEQ)
                {
                    self.passw_format_seq = r.read_string()?;
                } else {
                    r.skip_field()?;
                }
            }
        }

        let num_fields = header.num_fields as usize;
        let mut idx_conv: Vec<i32> = vec![-1; num_fields];
        let mut fields_used = [false; NUM_FIELDS];
        for conv in idx_conv.iter_mut() {
            let (name, raw_len) = r.read_ansi_string_raw()?;
            if raw_len == 0 {
                return Err(invalid_format());
            }
            let name = String::from_utf8_lossy(&name).to_string();
            if let Some(j) = FIELD_NAMES.iter().position(|f| f.eq_ignore_ascii_case(&name)) {
                *conv = j as i32;
                fields_used[j] = true;
            }
        }
        if !fields_used[FieldType::Title as usize]
            || !fields_used[FieldType::UserName as usize]
            || !fields_used[FieldType::Password as usize]
        {
            return Err(invalid_format());
        }

        for _ in 0..header.num_entries {
            let ei = self.add_db_entry();
            for _ in 0..=num_fields {
                let fi = r.read_field_index()?;
                if fi == FIELD_END {
                    break;
                }
                let fi = fi as usize;
                if fi < idx_conv.len() && idx_conv[fi] >= 0 {
                    let idx = idx_conv[fi] as usize;
                    match idx {
                        x if x == FieldType::KeyValueList as usize => {
                            let s = r.read_string()?;
                            let e = &mut self.entries[ei];
                            e.parse_key_value_list(&s);
                            e.update_key_value_string();
                        }
                        x if x == FieldType::Tags as usize => {
                            let s = r.read_string()?;
                            let e = &mut self.entries[ei];
                            e.parse_tag_list(&s);
                            e.update_tags_string();
                        }
                        x if x == FieldType::CreationTime as usize => {
                            self.entries[ei].creation_time = u64::from_le_bytes(r.read_field::<8>()?);
                            self.entries[ei].creation_time_set = true;
                        }
                        x if x == FieldType::ModificationTime as usize => {
                            self.entries[ei].modification_time = u64::from_le_bytes(r.read_field::<8>()?);
                            self.entries[ei].modification_time_set = true;
                        }
                        x if x == FieldType::PasswChangeTime as usize => {
                            self.entries[ei].passw_change_time = u64::from_le_bytes(r.read_field::<8>()?);
                            self.entries[ei].passw_change_time_set = true;
                        }
                        x if x == FieldType::PasswExpiryDate as usize => {
                            let d = u32::from_le_bytes(r.read_field::<4>()?);
                            let e = &mut self.entries[ei];
                            e.passw_expiry_date = d;
                            if expiry_date_to_string(d).is_empty() {
                                e.passw_expiry_date = 0;
                            }
                        }
                        x if x == FieldType::PasswHistory as usize => {
                            let h = r.read_type::<PASSW_HISTORY_HEADER_SIZE>()?;
                            let (flags, size, max) = (h[4], h[5], h[6]);
                            let mut items = Vec::new();
                            for _ in 0..size {
                                let ft = u64::from_le_bytes(r.read_type::<8>()?);
                                let s = r.read_string()?;
                                items.push((ft, s));
                            }
                            let hist = self.entries[ei].passw_history_mut();
                            hist.set_active(flags & 1 != 0);
                            hist.set_max_size(max as u32);
                            for (ft, s) in items {
                                hist.add_entry(ft, s, false);
                            }
                        }
                        _ => {
                            let s = r.read_string()?;
                            if idx == FieldType::Password as usize {
                                self.set_entry_passw_at(ei, &s);
                            } else {
                                self.entries[ei].strings[idx] = s;
                            }
                        }
                    }
                } else {
                    r.skip_field()?;
                }
            }
        }

        // keep the file open with a shared lock (original: fmShareDenyWrite)
        flock(&file, false);
        self.file = Some(file);
        self.state = DbOpenState::Open;
        Ok(())
    }

    /// `Save()`: write the database in format version 1.8.
    pub fn save(&mut self, path: &Path) -> Result<(), DbError> {
        self.check_db_open()?;
        if self.cipher_type > CIPHER_CHACHA20 {
            return Err(DbError::msg("Invalid cipher"));
        }
        if self.kdf_iterations == 0 {
            return Err(DbError::msg("Invalid number of KDF iterations"));
        }

        let mut fh = FileHeader {
            magic: HEADER_MAGIC,
            header_size: FILE_HEADER_SIZE as u8,
            version: VERSION,
            flags: 0,
            cipher_type: self.cipher_type,
            hash_type: HASH_SHA512,
            kdf_type: KDF_PBKDF2_SHA256,
            kdf_iterations: self.kdf_iterations,
        };
        if self.recovery_key {
            fh.flags |= FH_FLAG_RECOVERY_KEY;
        }
        if self.passw_utf8 {
            fh.flags |= FH_FLAG_PASSW_UTF8;
        }

        let (db_key, db_salt, recovery_block) = {
            let sec = self.sec.as_ref().unwrap();
            (
                Zeroizing::new(sec.db_key().to_vec()),
                Zeroizing::new(sec.db_salt().to_vec()),
                Zeroizing::new(sec.recovery().to_vec()),
            )
        };
        let mut cipher = create_cipher(self.cipher_type, &hkdf_extract_256(&db_key, &db_salt)[..])?;
        let mut iv = Zeroizing::new(vec![0u8; cipher.iv_size()]);
        RandomPool::global().get_data(&mut iv);
        cipher.set_iv(&iv);

        let mut header = DbHeader {
            magic: HEADER_MAGIC,
            header_size: DB_HEADER_SIZE as u16,
            num_fields: NUM_FIELDS as u8,
            num_entries: self.entries.len() as u32,
            ..Default::default()
        };
        if self.compressed {
            self.compression_level = if self.compression_level <= 0 {
                MZ_DEFAULT_LEVEL
            } else {
                self.compression_level.min(MZ_BEST_COMPRESSION)
            };
            header.compression_algo = COMPRESSION_DEFLATE;
            header.compression_level = self.compression_level as u8;
        }
        for (cond, flag) in [
            (!self.default_user_name.is_empty(), FLAG_DEFAULT_USER_NAME),
            (!self.passw_format_seq.is_empty(), FLAG_PASSW_FORMAT_SEQ),
            (self.default_passw_expiry_days != 0, FLAG_PASSW_EXPIRY_DAYS),
            (self.default_max_passw_history_size != 0, FLAG_DEFAULT_PASSW_HISTORY_SIZE),
            (self.master_passw_expiry_date != 0, FLAG_MASTER_PASSW_EXPIRY),
            (!self.description.is_empty(), FLAG_DESCRIPTION),
        ] {
            if cond {
                header.flags |= flag;
                header.num_var_params += 1;
            }
        }

        let mut w = StreamWriter { buf: Zeroizing::new(vec![0u8; DB_HEADER_SIZE]) };
        if header.flags & FLAG_DEFAULT_USER_NAME != 0 {
            w.write(&FLAG_DEFAULT_USER_NAME.to_le_bytes())?;
            w.write_string(&self.default_user_name, None)?;
        }
        if header.flags & FLAG_PASSW_FORMAT_SEQ != 0 {
            w.write(&FLAG_PASSW_FORMAT_SEQ.to_le_bytes())?;
            w.write_string(&self.passw_format_seq, None)?;
        }
        if header.flags & FLAG_PASSW_EXPIRY_DAYS != 0 {
            w.write(&FLAG_PASSW_EXPIRY_DAYS.to_le_bytes())?;
            w.write_field_buf(&self.default_passw_expiry_days.to_le_bytes(), None)?;
        }
        if header.flags & FLAG_DEFAULT_PASSW_HISTORY_SIZE != 0 {
            w.write(&FLAG_DEFAULT_PASSW_HISTORY_SIZE.to_le_bytes())?;
            w.write_field_buf(&[self.default_max_passw_history_size as u8], None)?;
        }
        if header.flags & FLAG_MASTER_PASSW_EXPIRY != 0 {
            w.write(&FLAG_MASTER_PASSW_EXPIRY.to_le_bytes())?;
            w.write_field_buf(&self.master_passw_expiry_date.to_le_bytes(), None)?;
        }
        if header.flags & FLAG_DESCRIPTION != 0 {
            w.write(&FLAG_DESCRIPTION.to_le_bytes())?;
            w.write_string(&self.description, None)?;
        }
        for name in FIELD_NAMES {
            w.write_ansi(name.as_bytes(), None)?;
        }

        for i in 0..self.entries.len() {
            for f in 0..NUM_STRING_FIELDS {
                let idx = Some(f as u8);
                if f == FieldType::Password as usize {
                    let p = self.get_entry_passw_at(i)?;
                    w.write_string(&p, idx)?;
                } else if f == FieldType::KeyValueList as usize {
                    w.write_string(&self.entries[i].key_value_list_as_string('\n'), idx)?;
                } else if f == FieldType::Tags as usize {
                    w.write_string(&self.entries[i].tags_as_string('\n'), idx)?;
                } else {
                    w.write_string(&self.entries[i].strings[f], idx)?;
                }
            }
            let e = &self.entries[i];
            w.write_field_buf(&e.creation_time.to_le_bytes(), Some(FieldType::CreationTime as u8))?;
            w.write_field_buf(&e.modification_time.to_le_bytes(), Some(FieldType::ModificationTime as u8))?;
            if e.passw_change_time != 0 {
                w.write_field_buf(&e.passw_change_time.to_le_bytes(), Some(FieldType::PasswChangeTime as u8))?;
            }
            let hist = &e.passw_history;
            if !hist.is_empty() {
                let n = (hist.len() as u32).min(MAX_PASSW_HISTORY_SIZE) as usize;
                // BlockSize counts the passwords in UTF-16 units, as the original
                let mut block_size = PASSW_HISTORY_HEADER_SIZE as u32;
                for (_, p) in hist.history.iter().take(n) {
                    block_size += 8 + 4 + p.encode_utf16().count() as u32;
                }
                let mut hh = [0u8; PASSW_HISTORY_HEADER_SIZE];
                hh[0..4].copy_from_slice(&block_size.to_le_bytes());
                hh[4] = if hist.active { 1 } else { 0 };
                hh[5] = n as u8;
                hh[6] = hist.max_size.min(MAX_PASSW_HISTORY_SIZE) as u8;
                w.write(&[FieldType::PasswHistory as u8])?;
                w.write(&hh)?;
                for (ft, p) in hist.history.iter().take(n) {
                    w.write(&ft.to_le_bytes())?;
                    w.write_string(p, None)?;
                }
            }
            if e.passw_expiry_date != 0 {
                w.write_field_buf(&e.passw_expiry_date.to_le_bytes(), Some(FieldType::PasswExpiryDate as u8))?;
            }
            w.write(&[FIELD_END])?;
        }

        let mut buf = w.buf;
        header.uncompressed_size = (buf.len() - DB_HEADER_SIZE) as u32;
        header.compressed_size = header.uncompressed_size;
        if self.compressed {
            let compr = miniz_compress(&buf[DB_HEADER_SIZE..], header.compression_level as i32)?;
            let mut nb = Zeroizing::new(Vec::with_capacity(DB_HEADER_SIZE + compr.len() + 16));
            nb.extend_from_slice(&[0u8; DB_HEADER_SIZE]);
            nb.extend_from_slice(&compr);
            // the original stores the size including the inner header
            header.compressed_size = nb.len() as u32;
            buf = nb;
        }
        buf[..DB_HEADER_SIZE].copy_from_slice(&header.to_bytes());

        if cipher.requires_alignment() {
            let aligned = align_to_block_size(buf.len(), cipher.block_size());
            if aligned > buf.len() {
                let mut pad = vec![0u8; aligned - buf.len()];
                RandomPool::global().get_data(&mut pad);
                buf.extend_from_slice(&pad);
            }
        }
        cipher.encrypt(&mut buf);

        let mac_key = hkdf_extract_512(&db_key, &db_salt);
        let mut hm = HmacSha512::new(&mac_key[..]);
        let mut out: Vec<u8> = Vec::with_capacity(FILE_HEADER_SIZE + 128 + 32 + 16 + buf.len() + 64);
        out.extend_from_slice(&fh.to_bytes());
        if self.recovery_key {
            out.extend_from_slice(&recovery_block);
            hm.update(&recovery_block);
        }
        out.extend_from_slice(&db_salt);
        hm.update(&db_salt);
        out.extend_from_slice(&iv);
        hm.update(&iv);
        out.extend_from_slice(&buf);
        hm.update(&buf);
        out.extend_from_slice(&hm.finish());

        // Saving fails if another process holds the file open (the original
        // cannot overwrite a file opened elsewhere with fmShareDenyWrite).
        self.file = None;
        if let Ok(existing) = File::open(path) {
            if !flock(&existing, true) {
                return Err(DbError::Io(io::Error::new(
                    io::ErrorKind::ResourceBusy,
                    "The file is being used by another process",
                )));
            }
        }
        let tmp = temp_path(path);
        {
            let mut f = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
            if let Err(e) = f.write_all(&out).and_then(|_| f.sync_all()) {
                let _ = fs::remove_file(&tmp);
                return Err(e.into());
            }
        }
        if let Ok(meta) = fs::metadata(path) {
            let _ = fs::set_permissions(&tmp, meta.permissions());
        }
        fs::rename(&tmp, path).inspect_err(|_| {
            let _ = fs::remove_file(&tmp);
        })?;
        let f = File::open(path)?;
        flock(&f, false);
        self.file = Some(f);
        self.last_version = VERSION;
        Ok(())
    }

    /// `ReleaseFile()`: give up the lock on the file.
    pub fn release_file(&mut self) {
        self.file = None;
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &[PasswDbEntry] {
        &self.entries
    }

    pub fn entry(&self, index: usize) -> &PasswDbEntry {
        &self.entries[index]
    }

    pub fn entry_mut(&mut self, index: usize) -> &mut PasswDbEntry {
        &mut self.entries[index]
    }

    /// Index of the entry with the given id.
    pub fn index_of_id(&self, id: u32) -> Option<usize> {
        self.entries.iter().position(|e| e.id == id)
    }

    fn add_db_entry(&mut self) -> usize {
        let id = self.next_entry_id;
        self.next_entry_id += 1;
        let idx = self.entries.len();
        self.entries.push(PasswDbEntry::new(id, idx as u32, false, 1, false));
        idx
    }

    /// `NewDbEntry()`: new entry with time stamps and the default user name.
    pub fn new_entry(&mut self) -> usize {
        let id = self.next_entry_id;
        self.next_entry_id += 1;
        let idx = self.entries.len();
        let mut e = PasswDbEntry::new(
            id,
            idx as u32,
            true,
            self.default_max_passw_history_size,
            self.default_max_passw_history_size > 0,
        );
        e.strings[FieldType::UserName as usize] = self.default_user_name.clone();
        self.entries.push(e);
        idx
    }

    /// `DuplicateDbEntry()`.
    pub fn duplicate_entry(&mut self, original: usize, title: &str) -> Result<usize, DbError> {
        let passw = self.get_entry_passw_at(original)?;
        let id = self.next_entry_id;
        self.next_entry_id += 1;
        let idx = self.entries.len();
        let mut d = PasswDbEntry::new(id, idx as u32, true, 1, false);
        let o = &self.entries[original];
        d.strings[FieldType::Title as usize] = Zeroizing::new(title.to_string());
        for f in [FieldType::UserName, FieldType::Url, FieldType::Keyword, FieldType::Notes] {
            d.strings[f as usize] = o.strings[f as usize].clone();
        }
        d.set_key_value_list(o.key_value_list.clone());
        d.set_tags(o.tags.clone());
        d.passw_history = o.passw_history.clone();
        d.passw_expiry_date = o.passw_expiry_date;
        d.passw_change_time = o.passw_change_time;
        d.passw_change_time_set = o.passw_change_time_set;
        self.entries.push(d);
        self.set_entry_passw_at(idx, &passw);
        Ok(idx)
    }

    /// `DeleteDbEntry()`.
    pub fn delete_entry(&mut self, index: usize) {
        if index < self.entries.len() {
            self.entries.remove(index);
            for (i, e) in self.entries.iter_mut().enumerate().skip(index) {
                e.index = i as u32;
            }
        }
    }

    /// `MoveDbEntry()`.
    pub fn move_entry(&mut self, cur: usize, new: usize) {
        let n = self.entries.len();
        if cur != new && cur < n && new < n {
            if cur < new {
                self.entries[cur..=new].rotate_left(1);
            } else {
                self.entries[new..=cur].rotate_right(1);
            }
            for (i, e) in self.entries.iter_mut().enumerate() {
                e.index = i as u32;
            }
        }
    }

    fn session_cipher(&self, id: u32) -> ChaCha {
        let sec = self.sec.as_ref().expect("database not initialised");
        let mut c = ChaCha::new(sec.mem_key());
        let mut iv = [0u8; 8];
        iv[..4].copy_from_slice(&id.to_le_bytes());
        c.iv_setup(&iv, None);
        c
    }

    fn set_entry_passw_at(&mut self, index: usize, passw: &str) {
        let id = self.entries[index].id;
        if passw.is_empty() {
            let e = &mut self.entries[index];
            e.enc_passw = Zeroizing::new(Vec::new());
            e.strings[FieldType::Password as usize] = Zeroizing::new(String::new());
            e.passw_hash = [0; 20];
            return;
        }
        let salt = self.sec.as_ref().unwrap().mem_salt().to_vec();
        let hash = hmac_sha1(&salt, passw.as_bytes());
        let mut enc = Zeroizing::new(passw.as_bytes().to_vec());
        self.session_cipher(id).apply_keystream(&mut enc);
        let plaintext = self.plaintext_passw;
        let e = &mut self.entries[index];
        e.passw_hash = hash;
        e.enc_passw = enc;
        if plaintext {
            e.strings[FieldType::Password as usize] = Zeroizing::new(passw.to_string());
        }
    }

    /// `SetDbEntryPassw()`.
    pub fn set_entry_passw(&mut self, index: usize, passw: &str) {
        self.set_entry_passw_at(index, passw);
    }

    fn get_entry_passw_at(&self, index: usize) -> Result<Zeroizing<String>, DbError> {
        let e = &self.entries[index];
        if e.enc_passw.is_empty() {
            return Ok(Zeroizing::new(String::new()));
        }
        if e.has_plaintext_passw() {
            return Ok(e.strings[FieldType::Password as usize].clone());
        }
        let mut dec = Zeroizing::new(e.enc_passw.to_vec());
        self.session_cipher(e.id).apply_keystream(&mut dec);
        let salt = self.sec.as_ref().unwrap().mem_salt();
        if hmac_sha1(salt, &dec) != e.passw_hash {
            return Err(DbError::msg("Internal error: Password decryption failed"));
        }
        let s = String::from_utf8(dec.to_vec()).map_err(|_| DbError::msg("Internal error: Password decryption failed"))?;
        Ok(Zeroizing::new(s))
    }

    /// `GetDbEntryPassw()`.
    pub fn get_entry_passw(&self, index: usize) -> Result<Zeroizing<String>, DbError> {
        self.get_entry_passw_at(index)
    }

    /// `SetPlaintextPassw()`: keep passwords in plaintext in `strings`
    /// (needed for showing them in the list and for searching).
    pub fn set_plaintext_passw(&mut self, plaintext: bool) -> Result<(), DbError> {
        if plaintext == self.plaintext_passw {
            return Ok(());
        }
        self.plaintext_passw = plaintext;
        for i in 0..self.entries.len() {
            if plaintext {
                let p = self.get_entry_passw_at(i)?;
                self.entries[i].strings[FieldType::Password as usize] = p;
            } else {
                self.entries[i].strings[FieldType::Password as usize] = Zeroizing::new(String::new());
            }
        }
        Ok(())
    }

    /// `CheckMasterKey()`.
    pub fn check_master_key(&self, key: &[u8]) -> Result<bool, DbError> {
        self.check_db_open()?;
        if key.is_empty() {
            return Err(DbError::msg("Specified \"key\" parameter is empty"));
        }
        let sec = self.sec.as_ref().unwrap();
        let salt = if self.recovery_key { &sec.recovery()[..DB_SALT_LENGTH] } else { sec.db_salt() };
        let mut check = Zeroizing::new(pbkdf2_256bit(key, salt, self.kdf_iterations, None).to_vec());
        if self.recovery_key {
            let mut c = create_cipher(self.cipher_type, &check)?;
            c.set_iv(sec.recovery());
            let mut k = Zeroizing::new(sec.recovery()[DB_SALT_LENGTH..DB_SALT_LENGTH + DB_KEY_LENGTH].to_vec());
            c.decrypt(&mut k);
            check = k;
        }
        Ok(check[..] == sec.db_key()[..])
    }

    /// `CheckRecoveryKey()`.
    pub fn check_recovery_key(&self, recovery_key: &[u8]) -> Result<bool, DbError> {
        self.check_db_open()?;
        if !self.recovery_key {
            return Err(DbError::msg("Recovery key not set"));
        }
        if recovery_key.is_empty() {
            return Err(DbError::msg("Specified \"key\" parameter is empty"));
        }
        let sec = self.sec.as_ref().unwrap();
        let slot = &sec.recovery()[DB_SALT_LENGTH + DB_KEY_LENGTH..];
        let derived = pbkdf2_256bit(recovery_key, &slot[..DB_SALT_LENGTH], self.kdf_iterations, None);
        let mut c = create_cipher(self.cipher_type, &derived[..])?;
        c.set_iv(slot);
        let mut k = Zeroizing::new(slot[DB_SALT_LENGTH..DB_SALT_LENGTH + DB_KEY_LENGTH].to_vec());
        c.decrypt(&mut k);
        Ok(k[..] == sec.db_key()[..])
    }

    /// `ChangeMasterKey()`. `kdf_iter_override = 0` keeps the current
    /// number of iterations. If `cancel` is set during the key derivation,
    /// nothing is changed.
    pub fn change_master_key(
        &mut self,
        new_key: &[u8],
        kdf_iter_override: u32,
        cancel: Option<&AtomicBool>,
        rng: Option<&mut dyn RandomGenerator>,
    ) -> Result<(), DbError> {
        self.check_db_open()?;
        if new_key.is_empty() {
            return Err(DbError::msg("Specified \"key\" parameter is empty"));
        }
        let iter = if kdf_iter_override == 0 { self.kdf_iterations } else { kdf_iter_override };
        let cancelled = || cancel.is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed));
        let cipher_type = self.cipher_type;
        if self.recovery_key {
            let mut salt = [0u8; DB_SALT_LENGTH];
            match rng {
                Some(r) => r.get_data(&mut salt),
                None => RandomPool::global().get_data(&mut salt),
            }
            let sec = self.sec.as_mut().unwrap();
            sec.recovery_mut()[..DB_SALT_LENGTH].copy_from_slice(&salt);
            let derived = pbkdf2_256bit(new_key, &salt, iter, cancel);
            if cancelled() {
                return Ok(());
            }
            let mut c = create_cipher(cipher_type, &derived[..])?;
            c.set_iv(&salt);
            let mut k = Zeroizing::new(sec.db_key().to_vec());
            c.encrypt(&mut k);
            sec.recovery_mut()[DB_SALT_LENGTH..DB_SALT_LENGTH + DB_KEY_LENGTH].copy_from_slice(&k);
        } else {
            let sec = self.sec.as_mut().unwrap();
            let salt = sec.db_salt().to_vec();
            let derived = pbkdf2_256bit(new_key, &salt, iter, cancel);
            if cancelled() {
                return Ok(());
            }
            sec.db_key_mut().copy_from_slice(&derived[..]);
        }
        if !cancelled() {
            self.kdf_iterations = iter;
        }
        Ok(())
    }

    /// `SetRecoveryKey()`: new random database key, encrypted with the
    /// master key (slot 1) and the recovery key (slot 2).
    pub fn set_recovery_key(&mut self, key: &[u8], recovery_key: &[u8]) -> Result<(), DbError> {
        self.check_db_open()?;
        if self.recovery_key {
            return Err(DbError::msg("Recovery key already set"));
        }
        if key.is_empty() || recovery_key.is_empty() {
            return Err(DbError::msg("Specified \"key\" parameter is empty"));
        }
        self.recovery_key = true;
        let cipher_type = self.cipher_type;
        let iter = self.kdf_iterations;
        let mut pool = RandomPool::global();
        let sec = self.sec.as_mut().unwrap();
        let mut dk = Zeroizing::new([0u8; DB_KEY_LENGTH]);
        pool.get_data(&mut dk[..]);
        sec.db_key_mut().copy_from_slice(&dk[..]);
        for slot in 0..2 {
            let off = slot * (DB_SALT_LENGTH + DB_KEY_LENGTH);
            let mut salt = [0u8; DB_SALT_LENGTH];
            pool.get_data(&mut salt);
            let src = if slot == 0 { key } else { recovery_key };
            let derived = pbkdf2_256bit(src, &salt, iter, None);
            let mut c = create_cipher(cipher_type, &derived[..])?;
            c.set_iv(&salt);
            let mut k = Zeroizing::new(dk.to_vec());
            c.encrypt(&mut k);
            let rb = sec.recovery_mut();
            rb[off..off + DB_SALT_LENGTH].copy_from_slice(&salt);
            rb[off + DB_SALT_LENGTH..off + DB_SALT_LENGTH + DB_KEY_LENGTH].copy_from_slice(&k);
        }
        pool.flush();
        Ok(())
    }

    /// `RemoveRecoveryKey()`.
    pub fn remove_recovery_key(&mut self, key: &[u8]) -> Result<(), DbError> {
        self.check_db_open()?;
        if !self.recovery_key {
            return Err(DbError::msg("Recovery key not set"));
        }
        if key.is_empty() {
            return Err(DbError::msg("Specified \"key\" parameter is empty"));
        }
        self.recovery_key = false;
        self.change_master_key(key, 0, None, None)?;
        fast_fill(self.sec.as_mut().unwrap().recovery_mut());
        Ok(())
    }

    /// `ExportToCsv()`: `col_mask` selects fields (bit = field index),
    /// `col_names` are the (translated) column titles.
    pub fn export_to_csv(
        &self,
        path: &Path,
        col_mask: u32,
        col_names: &[String; NUM_FIELDS],
        enc: crate::text_file::CharEncoding,
        newline: &str,
    ) -> Result<(), DbError> {
        let mut f = crate::text_file::TextFileWriter::create(path, enc, true)?;
        let mut header = String::new();
        for (i, name) in col_names.iter().enumerate() {
            if col_mask & (1 << i) != 0 {
                if !header.is_empty() {
                    header.push(',');
                }
                header.push('"');
                header.push_str(name);
                header.push('"');
            }
        }
        header.push_str(newline);
        f.write_str(&header)?;
        for (ei, e) in self.entries.iter().enumerate() {
            let mut j = 0;
            for i in 0..NUM_FIELDS {
                if col_mask & (1 << i) == 0 {
                    continue;
                }
                let field: Zeroizing<String> = match i {
                    x if x == FieldType::Password as usize => self.get_entry_passw_at(ei)?,
                    x if x == FieldType::CreationTime as usize => Zeroizing::new(e.creation_time_string()),
                    x if x == FieldType::ModificationTime as usize => Zeroizing::new(e.modification_time_string()),
                    x if x == FieldType::PasswChangeTime as usize => Zeroizing::new(e.passw_change_time_string()),
                    x if x == FieldType::PasswExpiryDate as usize => Zeroizing::new(e.passw_expiry_date_string()),
                    x if x == FieldType::PasswHistory as usize => {
                        let mut s = Zeroizing::new(String::new());
                        for (t, p) in e.passw_history.iter() {
                            if !s.is_empty() {
                                s.push(';');
                            }
                            if *t == 0 {
                                s.push('-');
                            } else {
                                s.push_str(&crate::util::time_stamp_to_string(*t));
                            }
                            s.push(';');
                            s.push_str(p);
                        }
                        s
                    }
                    x if x < NUM_STRING_FIELDS => {
                        let s = e.strings[x].as_str();
                        if x == FieldType::Notes as usize {
                            Zeroizing::new(s.replace("\r\n", " "))
                        } else {
                            Zeroizing::new(s.to_string())
                        }
                    }
                    _ => Zeroizing::new(String::new()),
                };
                let mut out = Zeroizing::new(String::new());
                if j > 0 {
                    out.push(',');
                }
                out.push('"');
                out.push_str(&field.replace('"', "\"\""));
                out.push('"');
                f.write_str(&out)?;
                j += 1;
            }
            f.write_str(newline)?;
        }
        Ok(())
    }

    // --- properties ----------------------------------------------------------

    pub fn version(&self) -> u16 {
        self.last_version
    }
    pub fn cipher_type(&self) -> u8 {
        self.cipher_type
    }
    /// `SetCipherType()`: not allowed while a recovery key is set.
    pub fn set_cipher_type(&mut self, t: u8) -> Result<(), DbError> {
        if self.recovery_key {
            return Err(DbError::msg("Action not allowed if recovery key is set"));
        }
        if t > CIPHER_CHACHA20 {
            return Err(DbError::msg("Cipher not supported"));
        }
        self.cipher_type = t;
        Ok(())
    }
    pub fn kdf_iterations(&self) -> u32 {
        self.kdf_iterations
    }
    /// `SetKdfIterations()`: not allowed while a recovery key is set.
    pub fn set_kdf_iterations(&mut self, n: u32) -> Result<(), DbError> {
        if self.recovery_key {
            return Err(DbError::msg("Action not allowed if recovery key is set"));
        }
        if n == 0 {
            return Err(DbError::msg("Invalid number of KDF iterations"));
        }
        self.kdf_iterations = n;
        Ok(())
    }
    pub fn has_recovery_key(&self) -> bool {
        self.recovery_key
    }
}

impl Drop for PasswDatabase {
    fn drop(&mut self) {
        self.close();
    }
}

fn temp_path(path: &Path) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mut rnd = [0u8; 6];
    fast_fill(&mut rnd);
    let suffix: String = rnd.iter().map(|b| format!("{b:02x}")).collect();
    path.with_file_name(format!(".{name}.{suffix}.tmp"))
}

/// `CreateKeyFile()`: 32 random bytes as 64 upper-case hex characters.
pub fn create_key_file(path: &Path) -> Result<(), DbError> {
    let mut key = Zeroizing::new([0u8; DB_KEY_LENGTH]);
    {
        let mut pool = RandomPool::global();
        pool.get_data(&mut key[..]);
        pool.flush();
    }
    crate::random::EntropyManager::global().consume_entropy_bits((DB_KEY_LENGTH * 8) as u32);
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut hex = Zeroizing::new(Vec::with_capacity(64));
    for b in key.iter() {
        hex.push(HEX[(b >> 4) as usize]);
        hex.push(HEX[(b & 15) as usize]);
    }
    let mut f = OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    f.write_all(&hex)?;
    Ok(())
}

/// `GetKeyFromKeyFile()`: 32-byte files as is, 64 hex characters decoded,
/// otherwise SHA-256 of the contents.
pub fn get_key_from_key_file(path: &Path) -> Result<Zeroizing<[u8; 32]>, DbError> {
    let data = Zeroizing::new(fs::read(path)?);
    if data.is_empty() {
        return Err(DbError::msg("Key file is empty"));
    }
    let mut key = Zeroizing::new([0u8; 32]);
    if data.len() == DB_KEY_LENGTH {
        key.copy_from_slice(&data);
        return Ok(key);
    }
    if data.len() == 2 * DB_KEY_LENGTH {
        let nibble = |c: u8| match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'A'..=b'F' => Some(c - b'A' + 10),
            b'a'..=b'f' => Some(c - b'a' + 10),
            _ => None,
        };
        let mut ok = true;
        for i in 0..DB_KEY_LENGTH {
            match (nibble(data[2 * i]), nibble(data[2 * i + 1])) {
                (Some(h), Some(l)) => key[i] = (h << 4) | l,
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            return Ok(key);
        }
    }
    *key = crate::crypto::sha256(&data);
    Ok(key)
}

/// `CombineKeySources()`: password bytes followed by the key from the key
/// file (if any).
pub fn combine_key_sources(passw: &[u8], key_file: Option<&Path>) -> Result<Zeroizing<Vec<u8>>, DbError> {
    if passw.is_empty() && key_file.is_none() {
        return Err(DbError::msg("Neither password nor key file is specified"));
    }
    let mut k = Zeroizing::new(passw.to_vec());
    if let Some(p) = key_file {
        let key = get_key_from_key_file(p).map_err(|e| match e {
            DbError::Io(_) => DbError::msg("Cannot open key file"),
            other => other,
        })?;
        k.extend_from_slice(&key[..]);
    }
    Ok(k)
}

/// Bytes of a master password as used for the key: UTF-8 for databases
/// with the UTF-8 flag (new databases), UTF-16LE for older ones
/// (`TPasswEnterDlg::GetPasswBinary()`).
pub fn passw_binary(passw: &str, utf8: bool) -> Zeroizing<Vec<u8>> {
    if utf8 {
        Zeroizing::new(passw.as_bytes().to_vec())
    } else {
        Zeroizing::new(passw.encode_utf16().flat_map(u16::to_le_bytes).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pwtool-db-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn fill(db: &mut PasswDatabase) {
        let i = db.new_entry();
        db.entry_mut(i).set_string(FieldType::Title, "Mail");
        db.entry_mut(i).set_string(FieldType::UserName, "me@example.org");
        db.entry_mut(i).set_string(FieldType::Notes, "line 1\r\nline 2");
        db.set_entry_passw(i, "pässwörd 😀");
        db.entry_mut(i).set_key_value("pin", "1234");
        db.entry_mut(i).add_tag("work");
        db.entry_mut(i).add_tag("ab");
        db.entry_mut(i).update_tags_string();
        db.entry_mut(i).update_key_value_string();
        db.entry_mut(i).passw_expiry_date = encode_expiry_date(2030, 2, 28);
        let h = db.entry_mut(i).passw_history_mut();
        h.set_active(true);
        h.set_max_size(5);
        db.entry_mut(i).add_current_passw_to_history(Zeroizing::new("old".into()));
        let j = db.new_entry();
        db.entry_mut(j).set_string(FieldType::Title, "Bank");
        db.set_entry_passw(j, "x");
        db.description = Zeroizing::new("My DB".into());
        db.default_user_name = Zeroizing::new("user".into());
        db.default_passw_expiry_days = 90;
        db.default_max_passw_history_size = 3;
    }

    fn check(db: &PasswDatabase) {
        assert_eq!(db.len(), 2);
        let e = db.entry(0);
        assert_eq!(e.string(FieldType::Title), "Mail");
        assert_eq!(e.string(FieldType::Notes), "line 1\r\nline 2");
        assert_eq!(db.get_entry_passw(0).unwrap().as_str(), "pässwörd 😀");
        assert_eq!(e.key_value("PIN"), Some("1234"));
        assert_eq!(e.tags_as_string(',').as_str(), "ab,work");
        assert_eq!(e.passw_expiry_date, encode_expiry_date(2030, 2, 28));
        assert_eq!(e.passw_history().len(), 1);
        assert_eq!(db.entry(1).string(FieldType::Title), "Bank");
        assert_eq!(db.description.as_str(), "My DB");
        assert_eq!(db.default_user_name.as_str(), "user");
        assert_eq!(db.default_passw_expiry_days, 90);
        assert_eq!(db.default_max_passw_history_size, 3);
    }

    #[test]
    fn roundtrip_all_variants() {
        let dir = tmpdir("rt");
        for cipher in [CIPHER_AES256, CIPHER_CHACHA20] {
            for compressed in [false, true] {
                let path = dir.join(format!("t{cipher}{compressed}.pwdb"));
                let key = b"master".to_vec();
                let mut db = PasswDatabase::new();
                db.create(&key).unwrap();
                db.set_cipher_type(cipher).unwrap();
                db.set_kdf_iterations(1000).unwrap();
                db.change_master_key(&key, 1000, None, None).unwrap();
                db.compressed = compressed;
                fill(&mut db);
                db.save(&path).unwrap();
                drop(db);

                let mut db = PasswDatabase::new();
                assert!(matches!(db.open(b"wrong", &path), Err(DbError::InvalidKey(_))));
                let mut db = PasswDatabase::new();
                db.open(&key, &path).unwrap();
                assert_eq!(db.version(), VERSION);
                assert_eq!(db.cipher_type(), cipher);
                assert_eq!(db.compressed, compressed);
                check(&db);
                assert!(db.check_master_key(&key).unwrap());
                assert!(!db.check_master_key(b"x").unwrap());
            }
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn recovery_key() {
        let dir = tmpdir("rec");
        let path = dir.join("r.pwdb");
        let mut db = PasswDatabase::new();
        db.create(b"master").unwrap();
        db.set_kdf_iterations(1000).unwrap();
        db.change_master_key(b"master", 1000, None, None).unwrap();
        fill(&mut db);
        db.set_recovery_key(b"master", b"recovery").unwrap();
        db.save(&path).unwrap();
        drop(db);
        for k in [&b"master"[..], &b"recovery"[..]] {
            let mut db = PasswDatabase::new();
            db.open(k, &path).unwrap();
            assert!(db.has_recovery_key());
            check(&db);
            assert!(db.check_recovery_key(b"recovery").unwrap());
            assert!(db.check_master_key(b"master").unwrap());
        }
        let mut db = PasswDatabase::new();
        db.open(b"recovery", &path).unwrap();
        db.change_master_key(b"new", 0, None, None).unwrap();
        db.save(&path).unwrap();
        drop(db);
        let mut db = PasswDatabase::new();
        db.open(b"new", &path).unwrap();
        db.remove_recovery_key(b"new2").unwrap();
        db.save(&path).unwrap();
        drop(db);
        let mut db = PasswDatabase::new();
        assert!(db.open(b"recovery", &path).is_err());
        let mut db = PasswDatabase::new();
        db.open(b"new2", &path).unwrap();
        assert!(!db.has_recovery_key());
        check(&db);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn key_files() {
        let dir = tmpdir("kf");
        let p = dir.join("k.key");
        create_key_file(&p).unwrap();
        let data = fs::read(&p).unwrap();
        assert_eq!(data.len(), 64);
        let k = get_key_from_key_file(&p).unwrap();
        assert_eq!(hex::encode_upper(*k), String::from_utf8(data).unwrap());
        fs::write(&p, b"some other content").unwrap();
        assert_eq!(*get_key_from_key_file(&p).unwrap(), crate::crypto::sha256(b"some other content"));
        let comb = combine_key_sources(b"pw", Some(&p)).unwrap();
        assert_eq!(comb.len(), 2 + 32);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn tag_order_matches_original() {
        let mut set = BTreeSet::new();
        for t in ["zz", "a", "work", "ab", "Z"] {
            set.insert(Tag(t.to_string()));
        }
        let v: Vec<&str> = set.iter().map(|t| t.0.as_str()).collect();
        assert_eq!(v, ["Z", "a", "ab", "zz", "work"]);
    }

    #[test]
    fn move_and_delete() {
        let mut db = PasswDatabase::new();
        db.create(b"k").unwrap();
        for t in ["a", "b", "c", "d"] {
            let i = db.new_entry();
            db.entry_mut(i).set_string(FieldType::Title, t);
        }
        db.move_entry(0, 2);
        let titles: Vec<&str> = db.entries().iter().map(|e| e.string(FieldType::Title)).collect();
        assert_eq!(titles, ["b", "c", "a", "d"]);
        db.move_entry(3, 0);
        let titles: Vec<&str> = db.entries().iter().map(|e| e.string(FieldType::Title)).collect();
        assert_eq!(titles, ["d", "b", "c", "a"]);
        db.delete_entry(1);
        assert_eq!(db.entry(1).index(), 1);
        assert_eq!(db.len(), 3);
    }

    #[test]
    fn malformed_header_does_not_panic() {
        let dir = tmpdir("malformed");
        let path = dir.join("bad.pwdb");
        // 106-byte file with valid magic, huge header_size and FH_FLAG_RECOVERY_KEY
        let mut buf = vec![0u8; 106];
        let fh = FileHeader {
            magic: HEADER_MAGIC,
            header_size: 255,
            version: VERSION,
            flags: FH_FLAG_RECOVERY_KEY,
            cipher_type: CIPHER_AES256,
            hash_type: HASH_SHA256,
            kdf_type: KDF_PBKDF2_SHA256,
            kdf_iterations: 1000,
        };
        buf[0..FILE_HEADER_SIZE].copy_from_slice(&fh.to_bytes());
        fs::write(&path, &buf).unwrap();
        let mut db = PasswDatabase::new();
        assert!(db.open(b"pw", &path).is_err());

        // Valid header_size=FILE_HEADER_SIZE, FH_FLAG_RECOVERY_KEY, but truncated body (106 bytes)
        let mut buf2 = vec![0u8; 106];
        let fh2 = FileHeader { header_size: FILE_HEADER_SIZE as u8, ..fh };
        buf2[0..FILE_HEADER_SIZE].copy_from_slice(&fh2.to_bytes());
        fs::write(&path, &buf2).unwrap();
        let mut db = PasswDatabase::new();
        assert!(db.open(b"pw", &path).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn entry_timestamps_kv_case_and_expiry_year() {
        let mut e = PasswDbEntry::new(1, 0, false, 0, false);
        // Omitted timestamps format as empty string
        assert_eq!(e.creation_time_string(), "");
        assert_eq!(e.modification_time_string(), "");
        assert_eq!(e.passw_change_time_string(), "");
        // Explicit timestamp (including 0) formats non-empty
        e.creation_time = 0;
        e.creation_time_set = true;
        assert!(!e.creation_time_string().is_empty());

        // Case-insensitive key_value / set_key_value
        e.set_key_value("ApiKey", "v1");
        assert_eq!(e.key_value("apikey"), Some("v1"));
        e.set_key_value("APIKEY", "v2");
        assert_eq!(e.key_value("ApiKey"), Some("v2"));
        assert_eq!(e.key_value_list().len(), 1);

        // Year bounds (1601..=9999)
        assert!(is_valid_calendar_date(1601, 1, 1));
        assert!(is_valid_calendar_date(9999, 12, 31));
        assert!(!is_valid_calendar_date(1600, 12, 31));
        assert!(!is_valid_calendar_date(10000, 1, 1));
    }
}
