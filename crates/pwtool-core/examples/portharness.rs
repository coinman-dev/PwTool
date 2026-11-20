//! portharness: drives the Rust port (`pwtool-core`) with the same commands,
//! options and JSON field names as the reference harness of the original
//! core (`tools/refharness`, see its README), so that `tools/refcompare`
//! can compare both outputs field by field.
//!
//! ```text
//! cargo run -q -p pwtool-core --example portharness -- [--det-rand-seed HEX]
//!     [--file-encoding N] [--acp N] <command> [options]
//! ```
//!
//! `--acp N` (port only) selects the "ANSI" code page (1250…1258); the
//! reference harness takes it from the locale of wine. Output: one JSON
//! object per run (UTF-8, binary data as lower-case hex, doubles with an
//! extra `X_bits` field holding the IEEE-754 bits). Exit code 0 = success,
//! 1 = error (`"ok":false`), 2 = usage error. Inputs that the port cannot
//! represent (e.g. lone UTF-16 surrogates) give the exception
//! `"Unsupported"`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};

use pwtool_core::crypt_text::{self, CryptTextError};
use pwtool_core::crypto::{self, chacha::ChaCha};
use pwtool_core::generate;
use pwtool_core::passw::database::{self, PasswDatabase, Tag};
use pwtool_core::passw::generator::*;
use pwtool_core::random::{AesCtrPrng, RandomGenerator, RandomPool};
use pwtool_core::scripting::{FLAG_STANDALONE_PASSW_GEN, LuaScript};
use pwtool_core::text_file::{self, CharEncoding};
use pwtool_core::unicode::{str_to_w32, w32_to_string};
use pwtool_core::util::floor_entropy_bits;
use zeroize::Zeroizing;

// ---------------------------------------------------------------------------
// errors

struct HErr {
    class: String,
    msg: String,
}

type R<T> = Result<T, HErr>;

fn herr(msg: impl Into<String>) -> HErr {
    HErr { class: "HarnessError".into(), msg: msg.into() }
}

fn unsupported(msg: impl Into<String>) -> HErr {
    HErr { class: "Unsupported".into(), msg: msg.into() }
}

fn db_err(e: database::DbError) -> HErr {
    let class = match &e {
        database::DbError::Error(_) => "EPasswDbError",
        database::DbError::InvalidFormat(_) => "EPasswDbInvalidFormat",
        database::DbError::InvalidKey(_) => "EPasswDbInvalidKey",
        database::DbError::Io(_) => "EStreamError",
    };
    HErr { class: class.into(), msg: e.to_string() }
}

// ---------------------------------------------------------------------------
// hex

fn to_hex(b: &[u8]) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(b.len() * 2);
    for &x in b {
        s.push(H[(x >> 4) as usize] as char);
        s.push(H[(x & 15) as usize] as char);
    }
    s
}

fn from_hex(s: &str) -> R<Vec<u8>> {
    let t: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    if t.len() % 2 != 0 {
        return Err(herr("odd number of hex digits"));
    }
    let val = |c: u8| -> R<u8> {
        match c {
            b'0'..=b'9' => Ok(c - b'0'),
            b'a'..=b'f' => Ok(c - b'a' + 10),
            b'A'..=b'F' => Ok(c - b'A' + 10),
            _ => Err(herr(format!("invalid hex digit '{}'", c as char))),
        }
    };
    t.chunks(2).map(|p| Ok(val(p[0])? * 16 + val(p[1])?)).collect()
}

// ---------------------------------------------------------------------------
// JSON writer (same layout as harness_util.h)

#[derive(Default)]
struct J {
    out: String,
    stack: Vec<bool>,
    after_key: bool,
}

impl J {
    fn prefix(&mut self) {
        if self.after_key {
            self.after_key = false;
            return;
        }
        if let Some(first) = self.stack.last_mut() {
            if !*first {
                self.out.push(',');
            }
            *first = false;
        }
    }
    fn begin_object(&mut self) {
        self.prefix();
        self.out.push('{');
        self.stack.push(true);
    }
    fn end_object(&mut self) {
        self.stack.pop();
        self.out.push('}');
    }
    fn begin_array(&mut self) {
        self.prefix();
        self.out.push('[');
        self.stack.push(true);
    }
    fn end_array(&mut self) {
        self.stack.pop();
        self.out.push(']');
    }
    fn key(&mut self, k: &str) -> &mut Self {
        self.prefix();
        self.write_string(k);
        self.out.push(':');
        self.after_key = true;
        self
    }
    fn write_string(&mut self, s: &str) {
        self.out.push('"');
        for c in s.chars() {
            match c {
                '"' => self.out.push_str("\\\""),
                '\\' => self.out.push_str("\\\\"),
                '\n' => self.out.push_str("\\n"),
                '\r' => self.out.push_str("\\r"),
                '\t' => self.out.push_str("\\t"),
                '\u{8}' => self.out.push_str("\\b"),
                '\u{c}' => self.out.push_str("\\f"),
                c if (c as u32) < 0x20 => self.out.push_str(&format!("\\u{:04x}", c as u32)),
                c => self.out.push(c),
            }
        }
        self.out.push('"');
    }
    fn str(&mut self, s: &str) {
        self.prefix();
        self.write_string(s);
    }
    fn hex(&mut self, b: &[u8]) {
        self.str(&to_hex(b));
    }
    fn boolean(&mut self, v: bool) {
        self.prefix();
        self.out.push_str(if v { "true" } else { "false" });
    }
    fn num(&mut self, v: i64) {
        self.prefix();
        self.out.push_str(&v.to_string());
    }
    fn unum(&mut self, v: u64) {
        self.prefix();
        self.out.push_str(&v.to_string());
    }
    fn dbl(&mut self, v: f64) {
        self.prefix();
        if !v.is_finite() {
            self.out.push_str(&format!("\"{v}\""));
        } else {
            let s = format!("{v:?}");
            self.out.push_str(&s);
        }
    }
    fn dbl_with_bits(&mut self, name: &str, v: f64) {
        self.key(name).dbl(v);
        let bits = format!("{:016x}", v.to_bits());
        self.key(&format!("{name}_bits")).str(&bits);
    }
}

// ---------------------------------------------------------------------------
// JSON parser (db-create spec)

#[derive(Debug, Clone)]
enum JV {
    Null,
    Bool(bool),
    Num(String),
    Str(String),
    Arr(Vec<JV>),
    Obj(Vec<(String, JV)>),
}

impl JV {
    fn get(&self, k: &str) -> Option<&JV> {
        match self {
            JV::Obj(v) => v.iter().find(|(n, _)| n == k).map(|(_, v)| v),
            _ => None,
        }
    }
}

struct JP<'a> {
    s: &'a [u8],
    p: usize,
}

impl JP<'_> {
    fn fail<T>(&self, m: &str) -> R<T> {
        Err(herr(format!("JSON parse error at offset {}: {m}", self.p)))
    }
    fn ws(&mut self) {
        while self.p < self.s.len() && matches!(self.s[self.p], b' ' | b'\t' | b'\n' | b'\r') {
            self.p += 1;
        }
    }
    fn peek(&self) -> u8 {
        *self.s.get(self.p).unwrap_or(&0)
    }
    fn hex4(&mut self) -> R<u32> {
        if self.p + 4 > self.s.len() {
            return self.fail("bad \\u escape");
        }
        let t = std::str::from_utf8(&self.s[self.p..self.p + 4]).map_err(|_| herr("bad \\u escape"))?;
        self.p += 4;
        u32::from_str_radix(t, 16).map_err(|_| herr("bad \\u escape"))
    }
    fn string(&mut self) -> R<String> {
        if self.peek() != b'"' {
            return self.fail("expected string");
        }
        self.p += 1;
        let mut units: Vec<u16> = Vec::new();
        let mut run: Vec<u8> = Vec::new();
        let flush = |run: &mut Vec<u8>, units: &mut Vec<u16>| {
            if !run.is_empty() {
                units.extend(String::from_utf8_lossy(run).encode_utf16());
                run.clear();
            }
        };
        loop {
            if self.p >= self.s.len() {
                return self.fail("unterminated string");
            }
            let c = self.s[self.p];
            self.p += 1;
            if c == b'"' {
                break;
            }
            if c != b'\\' {
                run.push(c);
                continue;
            }
            flush(&mut run, &mut units);
            let e = self.peek();
            self.p += 1;
            match e {
                b'"' => units.push('"' as u16),
                b'\\' => units.push('\\' as u16),
                b'/' => units.push('/' as u16),
                b'b' => units.push(8),
                b'f' => units.push(12),
                b'n' => units.push(10),
                b'r' => units.push(13),
                b't' => units.push(9),
                b'u' => units.push(self.hex4()? as u16),
                _ => return self.fail("bad escape"),
            }
        }
        flush(&mut run, &mut units);
        String::from_utf16(&units).map_err(|_| unsupported("lone surrogate in JSON string"))
    }
    fn value(&mut self) -> R<JV> {
        self.ws();
        match self.peek() {
            b'{' => {
                self.p += 1;
                let mut v = Vec::new();
                self.ws();
                if self.peek() == b'}' {
                    self.p += 1;
                    return Ok(JV::Obj(v));
                }
                loop {
                    self.ws();
                    let k = self.string()?;
                    self.ws();
                    if self.peek() != b':' {
                        return self.fail("expected ':'");
                    }
                    self.p += 1;
                    let x = self.value()?;
                    v.push((k, x));
                    self.ws();
                    match self.peek() {
                        b',' => self.p += 1,
                        b'}' => {
                            self.p += 1;
                            return Ok(JV::Obj(v));
                        }
                        _ => return self.fail("expected ',' or '}'"),
                    }
                }
            }
            b'[' => {
                self.p += 1;
                let mut v = Vec::new();
                self.ws();
                if self.peek() == b']' {
                    self.p += 1;
                    return Ok(JV::Arr(v));
                }
                loop {
                    v.push(self.value()?);
                    self.ws();
                    match self.peek() {
                        b',' => self.p += 1,
                        b']' => {
                            self.p += 1;
                            return Ok(JV::Arr(v));
                        }
                        _ => return self.fail("expected ',' or ']'"),
                    }
                }
            }
            b'"' => Ok(JV::Str(self.string()?)),
            _ => {
                for (lit, v) in [("true", JV::Bool(true)), ("false", JV::Bool(false)), ("null", JV::Null)] {
                    if self.s[self.p..].starts_with(lit.as_bytes()) {
                        self.p += lit.len();
                        return Ok(v);
                    }
                }
                let start = self.p;
                if self.peek() == b'-' {
                    self.p += 1;
                }
                while self.p < self.s.len() && matches!(self.s[self.p], b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
                {
                    self.p += 1;
                }
                let t = String::from_utf8_lossy(&self.s[start..self.p]).to_string();
                if t.is_empty() || t == "-" {
                    return self.fail("unexpected character");
                }
                Ok(JV::Num(t))
            }
        }
    }
}

fn parse_json(text: &[u8]) -> R<JV> {
    let mut p = JP { s: text, p: 0 };
    let v = p.value()?;
    p.ws();
    if p.p != text.len() {
        return p.fail("trailing characters");
    }
    Ok(v)
}

/// `stoull(s, nullptr, 0)` / `wcstoll(s, &end, 0)`-like parsing (base prefix
/// `0x` = hex, leading `0` = octal); the whole string must be consumed.
fn parse_c_int(s: &str) -> Option<i64> {
    let t = s.trim_start();
    let (neg, t) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let (radix, digits) = if t.len() > 2 && (t.starts_with("0x") || t.starts_with("0X")) {
        (16, &t[2..])
    } else if t.len() > 1 && t.starts_with('0') {
        (8, &t[1..])
    } else {
        (10, t)
    };
    if digits.is_empty() {
        return None;
    }
    let v = u64::from_str_radix(digits, radix).ok()? as i64;
    Some(if neg { v.wrapping_neg() } else { v })
}

fn j_str(v: &JV, k: &str) -> R<String> {
    match v.get(k) {
        None | Some(JV::Null) => Ok(String::new()),
        Some(JV::Str(s)) => Ok(s.clone()),
        _ => Err(herr(format!("spec: '{k}' must be a string"))),
    }
}

fn j_has(v: &JV, k: &str) -> bool {
    !matches!(v.get(k), None | Some(JV::Null))
}

fn j_u64(v: &JV, k: &str, def: u64) -> R<u64> {
    match v.get(k) {
        None | Some(JV::Null) => Ok(def),
        Some(JV::Bool(b)) => Ok(*b as u64),
        Some(JV::Str(s)) => parse_c_int(s).map(|x| x as u64).ok_or_else(|| herr(format!("spec: '{k}': bad number"))),
        Some(JV::Num(t)) if !t.contains(['.', 'e', 'E']) => {
            if let Some(n) = t.strip_prefix('-') {
                Ok(n.parse::<i64>().map_err(|_| herr("bad number"))?.wrapping_neg() as u64)
            } else {
                // strtoull saturates
                Ok(t.parse::<u64>().unwrap_or(u64::MAX))
            }
        }
        _ => Err(herr(format!("spec: '{k}' must be an integer"))),
    }
}

fn j_bool(v: &JV, k: &str, def: bool) -> R<bool> {
    match v.get(k) {
        None | Some(JV::Null) => Ok(def),
        Some(JV::Bool(b)) => Ok(*b),
        Some(JV::Num(t)) => Ok(t.parse::<f64>().unwrap_or(0.0) != 0.0),
        _ => Err(herr(format!("spec: '{k}' must be a boolean"))),
    }
}

/// Expiry date: integer (encoded), `{"y":..,"m":..,"d":..}` or "YYYY-MM-DD".
fn j_expiry(v: &JV, k: &str) -> R<u32> {
    match v.get(k) {
        None | Some(JV::Null) => Ok(0),
        Some(x @ JV::Obj(_)) => Ok(database::encode_expiry_date(
            j_u64(x, "y", 0)? as i32,
            j_u64(x, "m", 0)? as u32,
            j_u64(x, "d", 0)? as u32,
        )),
        Some(JV::Str(s)) => {
            // swscanf("%d-%d-%d")
            if let Some((y, m, d)) = scan_ymd(s) {
                return Ok(database::encode_expiry_date(y, m as u32, d as u32));
            }
            Ok(j_u64(v, k, 0)? as u32)
        }
        _ => Ok(j_u64(v, k, 0)? as u32),
    }
}

fn scan_ymd(s: &str) -> Option<(i32, i32, i32)> {
    fn int(s: &str) -> Option<(i32, &str)> {
        let t = s.trim_start();
        let b = t.as_bytes();
        let mut i = 0;
        if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
            i += 1;
        }
        let st = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == st {
            return None;
        }
        Some((t[..i].parse::<i64>().ok()? as i32, &t[i..]))
    }
    let (y, r) = int(s)?;
    let r = r.strip_prefix('-')?;
    let (m, r) = int(r)?;
    let r = r.strip_prefix('-')?;
    let (d, _) = int(r)?;
    Some((y, m, d))
}

// ---------------------------------------------------------------------------
// command line

const FLAG_OPTS: [&str; 15] = [
    "include-each-subset",
    "exclude-ambig",
    "determine-subsets",
    "lowercase-words",
    "add-length",
    "hashapass",
    "recovery",
    "combine",
    "first-char-not-lc",
    "old",
    "digest",
    "header-only",
    "utf16-passw",
    "exclude-duplicates",
    "check-each",
];

struct Args {
    opts: BTreeMap<String, Vec<String>>,
    flags: BTreeSet<String>,
}

impl Args {
    fn parse(argv: &[String]) -> R<Self> {
        let mut a = Args { opts: BTreeMap::new(), flags: BTreeSet::new() };
        let mut i = 0;
        while i < argv.len() {
            let s = &argv[i];
            if s.len() > 2 && s.starts_with("--") {
                let name = &s[2..];
                if let Some(eq) = name.find('=') {
                    a.opts.entry(name[..eq].to_string()).or_default().push(name[eq + 1..].to_string());
                    i += 1;
                    continue;
                }
                if FLAG_OPTS.contains(&name) {
                    a.flags.insert(name.to_string());
                    i += 1;
                    continue;
                }
                if i + 1 >= argv.len() {
                    return Err(herr(format!("option --{name} needs a value")));
                }
                a.opts.entry(name.to_string()).or_default().push(argv[i + 1].clone());
                i += 2;
            } else {
                return Err(herr(format!("unexpected argument {s}")));
            }
        }
        Ok(a)
    }
    fn has(&self, n: &str) -> bool {
        self.opts.contains_key(n)
    }
    fn flag(&self, n: &str) -> bool {
        self.flags.contains(n)
    }
    fn str(&self, n: &str) -> Option<&str> {
        self.opts.get(n).and_then(|v| v.last()).map(|s| s.as_str())
    }
    fn req(&self, n: &str) -> R<&str> {
        self.str(n).ok_or_else(|| herr(format!("missing required option --{n}")))
    }
    fn num(&self, n: &str, def: i64) -> R<i64> {
        match self.str(n) {
            None => Ok(def),
            Some(s) => parse_c_int(s).ok_or_else(|| herr(format!("option --{n}: not a number"))),
        }
    }
    fn req_num(&self, n: &str) -> R<i64> {
        if !self.has(n) {
            return Err(herr(format!("missing required option --{n}")));
        }
        self.num(n, 0)
    }

    /// String option NAME: `--NAME` | `--NAME-u16hex` | `--NAME-file`.
    fn has_wstr(&self, n: &str) -> bool {
        self.has(n) || self.has(&format!("{n}-u16hex")) || self.has(&format!("{n}-file"))
    }
    fn wstr(&self, n: &str, def: &str, required: bool) -> R<String> {
        if let Some(h) = self.str(&format!("{n}-u16hex")) {
            let b = from_hex(h)?;
            if b.len() % 2 != 0 {
                return Err(herr(format!("--{n}-u16hex: odd byte count")));
            }
            let units: Vec<u16> = b.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            return String::from_utf16(&units).map_err(|_| unsupported(format!("--{n}-u16hex: lone surrogate")));
        }
        if let Some(f) = self.str(&format!("{n}-file")) {
            let b = std::fs::read(f).map_err(|_| herr(format!("cannot open file {f}")))?;
            return Ok(String::from_utf8_lossy(&b).into_owned());
        }
        if let Some(s) = self.str(n) {
            return Ok(s.to_string());
        }
        if required {
            return Err(herr(format!("missing required option --{n}")));
        }
        Ok(def.to_string())
    }

    /// Byte option NAME: `--NAME-hex` | `--NAME-utf8` | `--NAME-utf16`.
    fn has_bytes(&self, n: &str) -> bool {
        self.has(&format!("{n}-hex")) || self.has(&format!("{n}-utf8")) || self.has(&format!("{n}-utf16"))
    }
    fn bytes(&self, n: &str, required: bool) -> R<Vec<u8>> {
        if let Some(h) = self.str(&format!("{n}-hex")) {
            return from_hex(h);
        }
        if let Some(s) = self.str(&format!("{n}-utf8")) {
            return Ok(s.as_bytes().to_vec());
        }
        if let Some(s) = self.str(&format!("{n}-utf16")) {
            return Ok(s.encode_utf16().flat_map(u16::to_le_bytes).collect());
        }
        if required {
            return Err(herr(format!("missing required option --{n}-hex")));
        }
        Ok(Vec::new())
    }
}

struct Globals {
    file_encoding: CharEncoding,
    acp: u32,
    det_log: Option<Arc<Mutex<Vec<Vec<u8>>>>>,
}

// ---------------------------------------------------------------------------
// RNG wrappers

/// Counts the bytes drawn (like `CountingRng` in the reference harness).
struct CountingRng<'a> {
    inner: &'a mut AesCtrPrng,
    count: u64,
}

impl RandomGenerator for CountingRng<'_> {
    fn seed(&mut self, s: &[u8]) {
        self.inner.seed(s);
    }
    fn get_data(&mut self, buf: &mut [u8]) {
        self.count += buf.len() as u64;
        self.inner.get_data(buf);
    }
    fn get_byte(&mut self) -> u8 {
        self.count += 1;
        self.inner.get_byte()
    }
}

/// Deterministic source for the random pool that records every request.
struct LoggingRng {
    inner: AesCtrPrng,
    log: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl RandomGenerator for LoggingRng {
    fn seed(&mut self, s: &[u8]) {
        self.inner.seed(s);
    }
    fn get_data(&mut self, buf: &mut [u8]) {
        self.inner.get_data(buf);
        self.log.lock().unwrap().push(buf.to_vec());
    }
    fn get_byte(&mut self) -> u8 {
        let mut b = [0u8];
        self.get_data(&mut b);
        b[0]
    }
}

fn enable_det_pool(seed: &[u8]) -> Arc<Mutex<Vec<Vec<u8>>>> {
    let log = Arc::new(Mutex::new(Vec::new()));
    let src = LoggingRng { inner: AesCtrPrng::from_seed(seed), log: log.clone() };
    RandomPool::global().set_deterministic_source(Some(Box::new(src)));
    log
}

// ---------------------------------------------------------------------------
// prng / hashes / chacha

fn cmd_prng(a: &Args, j: &mut J) -> R<()> {
    let mut prng = if a.has_bytes("seed") {
        j.key("seeding").str("Seed");
        AesCtrPrng::from_seed(&a.bytes("seed", true)?)
    } else {
        let key = a.bytes("key", true)?;
        let param = a.bytes("param", false)?;
        j.key("seeding").str("SeedWithKey");
        AesCtrPrng::from_key_and_param(&key, &param)
    };
    let mut skip = a.num("skip", 0)?;
    let mut buf = vec![0u8; 65536];
    while skip > 0 {
        let n = skip.min(buf.len() as i64) as usize;
        prng.get_data(&mut buf[..n]);
        skip -= n as i64;
    }
    if a.has("bytes") {
        let n = a.req_num("bytes")? as usize;
        let mut d = vec![0u8; n];
        if n > 0 {
            prng.get_data(&mut d);
        }
        if a.flag("digest") {
            j.key("data_len").num(n as i64);
            j.key("data_sha256").hex(&crypto::sha256(&d));
        } else {
            j.key("data").hex(&d);
        }
    }
    if a.has("numrange") {
        let m = a.req_num("numrange")?;
        let count = a.num("count", 1)?;
        j.key("numrange").num(m);
        j.key("values").begin_array();
        for _ in 0..count {
            j.unum(prng.get_num_range(m as u32) as u64);
        }
        j.end_array();
    }
    if a.has("after-bytes") {
        let n = a.req_num("after-bytes")? as usize;
        let mut d = vec![0u8; n];
        if n > 0 {
            prng.get_data(&mut d);
        }
        j.key("after").hex(&d);
    }
    Ok(())
}

fn cmd_pbkdf2(a: &Args, j: &mut J) -> R<()> {
    let passw = a.bytes("passw", true)?;
    let salt = a.bytes("salt", false)?;
    let iter = a.num("iter", 8192)?;
    let key = crypto::pbkdf2_256bit(&passw, &salt, iter as u32, None);
    j.key("iter").num(iter);
    j.key("key").hex(&key[..]);
    Ok(())
}

fn cmd_hmac(a: &Args, j: &mut J, bits: u32) -> R<()> {
    let key = a.bytes("key", true)?;
    let data = a.bytes("data", false)?;
    let mac: Vec<u8> = match bits {
        256 => crypto::hmac_sha256(&key, &data).to_vec(),
        512 => crypto::hmac_sha512(&key, &data).to_vec(),
        _ => crypto::hmac_sha1(&key, &data).to_vec(),
    };
    j.key("mac").hex(&mac);
    Ok(())
}

fn cmd_hash(a: &Args, j: &mut J, bits: u32) -> R<()> {
    let data = if let Some(f) = a.str("file") {
        std::fs::read(f).map_err(|_| herr(format!("cannot open file {f}")))?
    } else {
        a.bytes("data", false)?
    };
    let h: Vec<u8> = match bits {
        256 => crypto::sha256(&data).to_vec(),
        512 => crypto::sha512(&data).to_vec(),
        _ => crypto::sha1(&data).to_vec(),
    };
    j.key("hash").hex(&h);
    Ok(())
}

fn cmd_chacha(a: &Args, j: &mut J) -> R<()> {
    let key = a.bytes("key", true)?;
    if key.len() != 16 && key.len() != 32 {
        return Err(herr("key must be 16 or 32 bytes"));
    }
    let iv = a.bytes("iv", true)?;
    if iv.len() != 8 {
        return Err(herr("iv must be 8 bytes"));
    }
    let ctr = a.bytes("ctr", false)?;
    if !ctr.is_empty() && ctr.len() != 8 {
        return Err(herr("ctr must be 8 bytes"));
    }
    let rounds = a.num("rounds", 20)?;
    let mut c = ChaCha::new(&key);
    // chacha_nrounds(int): negative values are raised to 6 like small ones
    c.set_rounds(if (rounds as i32) < 0 { 0 } else { rounds as u32 });
    c.iv_setup(&iv, if ctr.is_empty() { None } else { Some(&ctr) });
    let encrypt = a.has_bytes("data");
    let data = if encrypt { a.bytes("data", true)? } else { vec![0u8; a.req_num("bytes")? as usize] };
    let mut chunks = Vec::new();
    if let Some(s) = a.str("chunks") {
        for p in s.split(',').filter(|p| !p.is_empty()) {
            chunks.push(p.parse::<usize>().map_err(|_| herr("bad --chunks"))?);
        }
    }
    let mut out = vec![0u8; data.len()];
    let mut off = 0;
    for ch in chunks {
        let ch = ch.min(data.len() - off);
        if encrypt {
            c.encrypt_bytes(&data[off..off + ch], &mut out[off..off + ch]);
        } else {
            c.keystream_bytes(&mut out[off..off + ch]);
        }
        off += ch;
    }
    if off < data.len() {
        if encrypt {
            c.encrypt_bytes(&data[off..], &mut out[off..]);
        } else {
            c.keystream_bytes(&mut out[off..]);
        }
    }
    j.key("rounds").num(rounds);
    j.key(if encrypt { "ciphertext" } else { "keystream" }).hex(&out);
    Ok(())
}

// ---------------------------------------------------------------------------
// password generator

fn seed_generator(a: &Args) -> R<(AesCtrPrng, &'static str)> {
    if a.has_wstr("seed-passw") {
        let pw = a.wstr("seed-passw", "", false)?;
        let key = pwtool_core::mp_passw_gen::derive_key(&pw);
        return Ok((AesCtrPrng::from_key_and_param(&key[..], &[]), "seed-passw"));
    }
    if a.has_bytes("seed") {
        return Ok((AesCtrPrng::from_seed(&a.bytes("seed", true)?), "seed"));
    }
    if a.has_bytes("key") {
        let key = a.bytes("key", true)?;
        let param = a.bytes("param", false)?;
        return Ok((AesCtrPrng::from_key_and_param(&key, &param), "key"));
    }
    Err(herr("no seed given (--seed-passw, --seed-hex or --key-hex)"))
}

fn char_set_type_index(t: CharSetType) -> i64 {
    match t {
        CharSetType::Standard => 0,
        CharSetType::StandardWithFreq => 1,
        CharSetType::Phonetic => 2,
        CharSetType::PhoneticUpperCase => 3,
        CharSetType::PhoneticMixedCase => 4,
    }
}

fn write_gen_info(j: &mut J, g: &PasswordGenerator) {
    j.key("custom_charset_type").num(char_set_type_index(g.custom_char_set_type()));
    j.dbl_with_bits("custom_charset_entropy", g.custom_char_set_entropy());
    j.key("custom_charset_unique_size").num(g.custom_char_set_unique_size() as i64);
    j.key("custom_charset_size").num(g.custom_char_set_w32().len() as i64);
    j.key("custom_charset").str(&w32_to_string(g.custom_char_set_w32()));
    j.key("word_list_size").num(g.word_list_size() as i64);
    j.dbl_with_bits("word_list_entropy", g.word_list_entropy());
    j.dbl_with_bits("phonetic_entropy", g.phonetic_entropy());
}

fn setup_generator(a: &Args, j: &mut J, g: &mut PasswordGenerator, gl: &Globals) -> R<()> {
    g.set_word_separator(&a.wstr("word-sep", "", false)?);
    g.set_word_char_separator(&a.wstr("word-char-sep", "", false)?);
    if let Some(t) = a.str("trigram") {
        let r = match g.load_trigram_file(Some(Path::new(t))) {
            Ok(true) => 1,
            Ok(false) => 0,
            Err(_) => -1,
        };
        j.key("trigram_load_result").num(r);
    }
    if let Some(w) = a.str("wordlist") {
        let r = match g.load_word_list_file(
            Some(Path::new(w)),
            a.num("min-word-len", 1)? as i32,
            a.num("max-word-len", WORDLIST_MAX_WORDLEN as i64)? as i32,
            a.flag("lowercase-words"),
            gl.file_encoding,
        ) {
            Ok(n) => n as i64,
            Err(WordListError::TooFewWords) => 0,
            Err(WordListError::Io(_)) => -1,
        };
        j.key("wordlist_load_result").num(r);
    }
    if a.has_wstr("charset")
        || a.has_wstr("ambig")
        || a.has_wstr("symbols")
        || a.flag("include-each-subset")
        || a.flag("exclude-ambig")
        || a.flag("determine-subsets")
    {
        let r = g.setup_char_sets(
            &a.wstr("charset", "<AZ><az><09>", false)?,
            &a.wstr("ambig", "", false)?,
            &a.wstr("symbols", "", false)?,
            a.flag("include-each-subset"),
            a.flag("exclude-ambig"),
            a.flag("determine-subsets"),
        );
        j.key("setup_charsets_result").str(&r);
    }
    Ok(())
}

fn cmd_gen(a: &Args, j: &mut J, gl: &Globals, force_mode: Option<&str>) -> R<()> {
    let (mut inner, seeding) = seed_generator(a)?;
    j.key("seeding").str(seeding);
    let mut g = PasswordGenerator::new();
    setup_generator(a, j, &mut g, gl)?;
    write_gen_info(j, &g);

    let mode = force_mode.map(|s| s.to_string()).unwrap_or_else(|| a.str("mode").unwrap_or("password").to_string());
    let count = a.num("count", 1)?;
    let flags = a.num("flags", 0)? as i32 as u32;
    j.key("mode").str(&mode);

    let consumed;
    {
        let mut rng = CountingRng { inner: &mut inner, count: 0 };
        match mode.as_str() {
            "main" => gen_main(a, j, &g, &mut rng, count as u64)?,
            "password" | "phonetic" => {
                let len = a.num("length", 16)? as i32;
                j.key("passwords").begin_array();
                for _ in 0..count {
                    let p = if mode == "password" {
                        g.get_password(&mut rng, len, flags)
                    } else {
                        g.get_phonetic_passw(&mut rng, len, flags)
                    };
                    j.begin_object();
                    j.key("len").num(p.len() as i64);
                    j.key("password").str(&w32_to_string(&p));
                    j.end_object();
                }
                j.end_array();
            }
            "passphrase" => {
                let words = a.num("words", 4)? as i32;
                let chars = str_to_w32(&a.wstr("chars", "", false)?);
                j.key("passwords").begin_array();
                for _ in 0..count {
                    let (p, net) = g.get_passphrase(&mut rng, words, &chars, flags);
                    j.begin_object();
                    j.key("len").num(p.len() as i64);
                    j.key("net_words_len").num(net as i64);
                    j.key("password").str(&w32_to_string(&p));
                    j.end_object();
                }
                j.end_array();
            }
            "format" => {
                let format = str_to_w32(&a.wstr("format", "", true)?);
                let passw_in = str_to_w32(&a.wstr("format-passw", "", false)?);
                let with_passw = a.has_wstr("format-passw");
                j.key("passwords").begin_array();
                for _ in 0..count {
                    let mut sec = 0.0f64;
                    let res = g.get_format_passw(
                        &mut rng,
                        &format,
                        flags,
                        if with_passw { Some(&passw_in[..]) } else { None },
                        generate::PASSWFORMAT_MAX_CHARS,
                        Some(&mut sec),
                    );
                    j.begin_object();
                    j.key("len").num(res.passw.len() as i64);
                    j.key("password").str(&w32_to_string(&res.passw));
                    j.dbl_with_bits("security", sec);
                    j.key("passw_used").num(res.passw_used as i64);
                    j.key("invalid_spec").str(&w32_to_string(&res.invalid_spec));
                    j.end_object();
                }
                j.end_array();
            }
            _ => return Err(herr("unknown --mode")),
        }
        consumed = rng.count;
    }
    j.key("rng_bytes_consumed").unum(consumed);
    let mut next = [0u8; 8];
    inner.get_data(&mut next);
    j.key("prng_next_bytes").hex(&next);
    Ok(())
}

/// Main window loop through `pwtool_core::generate::generate()`, with the
/// request built like `AppCore::gen_request()` + `apply_limits()` and the
/// script hook like `cli.rs`. The raw generator flags of the reference
/// harness are mapped back to `PASSWOPTION_*` flags; combinations the app
/// cannot produce are rejected as "Unsupported".
fn gen_main(a: &Args, j: &mut J, g: &PasswordGenerator, rng: &mut CountingRng<'_>, num: u64) -> R<()> {
    use generate::*;
    let chars_len = a.num("length", 0)? as i32;
    let num_words = a.num("words", 0)? as i32;
    let format = a.wstr("format", "", false)?;
    let pf = a.num("passw-flags", 0)? as u32;
    let phf = a.num("passphr-flags", 0)? as u32;
    let ff = a.num("format-flags", 0)? as u32;

    if pf & (PASSW_FLAG_FIRSTCHARNOTLC | PASSW_FLAG_PHONETICUPPERCASE | PASSW_FLAG_PHONETICMIXEDCASE | PASSW_FLAG_CHECKDUPLICATESBYSET)
        != 0
    {
        return Err(unsupported("raw passw flag that the app sets automatically"));
    }
    let mut opt = 0u32;
    let mut set_opt = |cond: bool, f: u32| {
        if cond {
            opt |= f;
        }
    };
    set_opt(pf & PASSW_FLAG_EXCLUDEREPCHARS != 0, PASSWOPTION_EXCLUDEREPCHARS);
    set_opt(pf & PASSW_FLAG_INCLUDESUBSET != 0, PASSWOPTION_INCLUDESUBSET);
    for i in 0..4 {
        set_opt(pf & (PASSW_FLAG_INCLUDEUPPERCASE << i) != 0, PASSWOPTION_INCLUDEUPPERCASE << i);
    }
    set_opt(pf & PASSW_FLAG_EACHCHARONLYONCE != 0, PASSWOPTION_EACHCHARONLYONCE);
    set_opt(pf & PASSW_FLAG_REMOVEWHITESPACE != 0, PASSWOPTION_REMOVEWHITESPACE);
    set_opt(phf & PASSPHR_FLAG_CAPITALIZEWORDS != 0, PASSWOPTION_CAPITALIZEWORDS);
    set_opt(phf & PASSPHR_FLAG_DONTSEPWORDS != 0, PASSWOPTION_DONTSEPWORDS);
    set_opt(phf & PASSPHR_FLAG_DONTSEPWCH != 0, PASSWOPTION_DONTSEPWORDSCHARS);
    set_opt(phf & PASSPHR_FLAG_REVERSEWCHORDER != 0, PASSWOPTION_REVERSEWCHORDER);
    set_opt(phf & PASSPHR_FLAG_EACHWORDONLYONCE != 0, PASSWOPTION_EACHWORDONLYONCE);
    set_opt(ff & PASSFORMAT_FLAG_EXCLUDEREPCHARS != 0, PASSWOPTION_EXCLUDEREPCHARS);
    set_opt(ff & PASSFORMAT_FLAG_REMOVEWHITESPACE != 0, PASSWOPTION_REMOVEWHITESPACE);
    set_opt(a.flag("first-char-not-lc"), PASSWOPTION_FIRSTCHARNOTLC);
    set_opt(a.flag("exclude-duplicates"), PASSWOPTION_EXCLUDEDUPLICATES);
    set_opt(a.flag("check-each"), PASSWOPTION_CHECKEACHPASSW);
    // the app sets passw and format flags from one option each
    if chars_len != 0 {
        if (opt & PASSWOPTION_EXCLUDEREPCHARS != 0) != (pf & PASSW_FLAG_EXCLUDEREPCHARS != 0)
            || (opt & PASSWOPTION_REMOVEWHITESPACE != 0) != (pf & PASSW_FLAG_REMOVEWHITESPACE != 0)
        {
            return Err(unsupported("inconsistent passw/format flags"));
        }
    }
    if !format.is_empty()
        && ((opt & PASSWOPTION_EXCLUDEREPCHARS != 0) != (ff & PASSFORMAT_FLAG_EXCLUDEREPCHARS != 0)
            || (opt & PASSWOPTION_REMOVEWHITESPACE != 0) != (ff & PASSFORMAT_FLAG_REMOVEWHITESPACE != 0))
    {
        return Err(unsupported("inconsistent passw/format flags"));
    }
    let combine = a.flag("combine") || phf & PASSPHR_FLAG_COMBINEWCH != 0;

    let passphrase_length = if num_words != 0 {
        match a.str("passphr-len") {
            Some(t) => parse_passphrase_length(t).map_err(herr)?,
            None => None,
        }
    } else {
        None
    };

    let mut req = GenRequest {
        dest: GenDest::GuiSingle,
        num_passw: num,
        chars_len,
        num_words,
        combine_words_chars: combine,
        passphrase_length,
        format: format.clone(),
        option_flags: opt,
        include_header: false,
        file_newline: "\n".into(),
        test_common_passw: false,
    };
    apply_limits(g, &mut req);

    let mut script: Option<LuaScript> = None;
    if let Some(path) = a.str("script") {
        let mut s = LuaScript::new();
        s.load_file(Path::new(path), Some(g), rng).map_err(|e| HErr { class: "ELuaError".into(), msg: e.0 })?;
        j.key("script_flags").num(s.flags() as i64);
        script = Some(s);
    }
    if req.chars_len == 0 && req.num_words == 0 && req.format.is_empty() && script.is_none() {
        return Err(herr("nothing to generate (--length/--words/--format/--script)"));
    }

    let mut gen_flags = 0;
    if req.chars_len != 0 {
        gen_flags |= 1;
    }
    if req.num_words != 0 {
        gen_flags |= 2;
    }
    if !req.format.is_empty() {
        gen_flags |= 4;
    }
    let dest = a.num("dest", 0)? as i32;
    let adv_flags = a.num("advanced-flags", opt as i64)? as u32;
    let standalone = script.as_ref().map(|s| s.flags() & FLAG_STANDALONE_PASSW_GEN != 0).unwrap_or(false);
    let has_script = script.is_some();
    let (rc, rw, rf) = (req.chars_len, req.num_words, req.format.clone());
    // security reported by the script for the latest password
    let last_script_sec = std::cell::Cell::new(f64::NAN);
    let mut hook = |r: &mut dyn RandomGenerator,
                    n: u64,
                    p: Option<&str>,
                    sec: f64|
     -> Result<(Zeroizing<String>, f64), String> {
        let s = script.as_mut().unwrap();
        if !s.is_initialized() {
            s.init_generate(g, r, num, dest, gen_flags, adv_flags, rc, rw, &rf).map_err(|e| e.to_string())?;
        }
        let res = s.call_generate(g, r, n, p, sec).map_err(|e| e.to_string());
        if let Ok((_, ssec)) = &res {
            last_script_sec.set(*ssec);
        }
        res
    };
    let script_arg: Option<(&mut generate::ScriptHook<'_>, bool)> =
        if has_script { Some((&mut hook, standalone)) } else { None };

    let check_each = num > 1 && opt & PASSWOPTION_CHECKEACHPASSW != 0;
    let mut out: Vec<(String, usize, f64, f64)> = Vec::new();
    let cancel = AtomicBool::new(false);
    let progress = AtomicU64::new(0);
    let res = generate::generate(g, rng, &req, None, script_arg, &cancel, &progress, &mut |p, _| {
        out.push((p.passw.to_string(), p.len, p.security, last_script_sec.get()));
        Ok(SinkAction::Continue)
    });
    if let Err(e) = res {
        return Err(HErr { class: "ELuaError".into(), msg: e });
    }
    // a stand-alone script generator replaces the other sources
    j.key("eff_length").num(if standalone { 0 } else { req.chars_len as i64 });
    j.key("eff_words").num(if standalone { 0 } else { req.num_words as i64 });
    j.key("option_flags").num(opt as i64);
    j.key("passwords").begin_array();
    for (i, (p, len, sec, ssec)) in out.iter().enumerate() {
        j.begin_object();
        j.key("password").str(p);
        j.key("len").num(*len as i64);
        j.dbl_with_bits("security", *sec);
        if i == 0 || check_each {
            j.key("floor_entropy_bits").num(floor_entropy_bits(*sec) as i64);
        }
        if !ssec.is_nan() {
            j.dbl_with_bits("script_security", *ssec);
        }
        j.end_object();
    }
    j.end_array();
    Ok(())
}

fn cmd_charsets(a: &Args, j: &mut J, gl: &Globals) -> R<()> {
    let mut g = PasswordGenerator::new();
    setup_generator(a, j, &mut g, gl)?;
    write_gen_info(j, &g);
    if a.has("perm-set-size") {
        let v = PasswordGenerator::calc_perm_set_entropy(a.req_num("perm-set-size")? as i32, a.num("perm-samples", 1)? as i32);
        j.dbl_with_bits("perm_set_entropy", v);
    }
    if a.has("word-index") {
        let i = a.req_num("word-index")? as i32;
        if i < 0 || i >= g.word_list_size() {
            return Err(unsupported("word index out of range"));
        }
        j.key("word").str(&g.get_word(i));
    }
    Ok(())
}

fn cmd_estimate(a: &Args, j: &mut J) -> R<()> {
    let s = a.wstr("passw", "", true)?;
    let d = PasswordGenerator::estimate_passw_security(&s);
    j.dbl_with_bits("security", d);
    j.key("floor_entropy_bits").num(floor_entropy_bits(d) as i64);
    Ok(())
}

// zxcvbn with the match list (the C library is linked into pwtool-core)
#[repr(C)]
struct ZxcMatch {
    begin: i32,
    length: i32,
    entrpy: f64,
    mlt_enpy: f64,
    typ: i32,
    next: *mut ZxcMatch,
}

unsafe extern "C" {
    fn ZxcvbnMatch(passwd: *const std::os::raw::c_char, user_dict: *const *const std::os::raw::c_char, info: *mut *mut ZxcMatch) -> f64;
    fn ZxcvbnFreeInfo(info: *mut ZxcMatch);
}

fn cmd_zxcvbn(a: &Args, j: &mut J) -> R<()> {
    let s = a.wstr("passw", "", true)?;
    let d = pwtool_core::zxcvbn::zxcvbn_match(&s);
    j.dbl_with_bits("entropy", d);
    j.key("floor_entropy_bits").num(floor_entropy_bits(d) as i64);
    let mut bytes = s.as_bytes().to_vec();
    if let Some(p) = bytes.iter().position(|&b| b == 0) {
        bytes.truncate(p);
    }
    let c = std::ffi::CString::new(bytes).unwrap();
    let mut info: *mut ZxcMatch = std::ptr::null_mut();
    // SAFETY: valid C string; the list is freed with ZxcvbnFreeInfo.
    let d2 = unsafe { ZxcvbnMatch(c.as_ptr(), std::ptr::null(), &mut info) };
    j.key("matches").begin_array();
    let mut p = info;
    while !p.is_null() {
        // SAFETY: element of the list returned by ZxcvbnMatch.
        let m = unsafe { &*p };
        j.begin_object();
        j.key("begin").num(m.begin as i64);
        j.key("length").num(m.length as i64);
        j.key("type").num(m.typ as i64);
        j.dbl_with_bits("entropy", m.entrpy);
        j.dbl_with_bits("mlt_entropy", m.mlt_enpy);
        j.end_object();
        p = m.next;
    }
    j.end_array();
    // SAFETY: list from ZxcvbnMatch.
    unsafe { ZxcvbnFreeInfo(info) };
    j.dbl_with_bits("entropy_with_info", d2);
    Ok(())
}

// ---------------------------------------------------------------------------
// crypt text

fn crypt_err_code(e: CryptTextError) -> i64 {
    match e {
        CryptTextError::Clipboard => 1,
        CryptTextError::NoText => 2,
        CryptTextError::TextTooLong => 3,
        CryptTextError::OutOfMemory => 4,
        CryptTextError::TextCorrupted => 5,
        CryptTextError::BadKey => 6,
        CryptTextError::DecomprFailed => 7,
    }
}

const CRYPT_NAMES: [&str; 8] =
    ["OK", "CLIPBOARD", "NOTEXT", "TEXTTOOLONG", "OUTOFMEMORY", "TEXTCORRUPTED", "BADKEY", "DECOMPRFAILED"];

fn encoding_for_codepage(cp: u32) -> R<&'static encoding_rs::Encoding> {
    use encoding_rs::*;
    Ok(match cp {
        874 => WINDOWS_874,
        1250 => WINDOWS_1250,
        1251 => WINDOWS_1251,
        1252 => WINDOWS_1252,
        1253 => WINDOWS_1253,
        1254 => WINDOWS_1254,
        1255 => WINDOWS_1255,
        1256 => WINDOWS_1256,
        1257 => WINDOWS_1257,
        1258 => WINDOWS_1258,
        _ => return Err(unsupported(format!("code page {cp}"))),
    })
}

fn cmd_crypt_enc(a: &Args, j: &mut J) -> R<()> {
    let pw = a.wstr("passw", "", true)?;
    let text = a.wstr("text", "", true)?;
    let seed = a.bytes("rand-seed", true)?;
    let mut rng = AesCtrPrng::from_seed(&seed);
    let r = crypt_text::encrypt_text_with_password(&text, &pw, &mut rng);
    match r {
        Ok(t) => {
            j.key("result").num(0);
            j.key("result_name").str("OK");
            j.key("clipboard_set").boolean(true);
            j.key("text").str(&t);
            j.key("via_ansi").boolean(true);
            j.key("ansi_bytes").hex(t.as_bytes());
        }
        Err(e) => {
            let c = crypt_err_code(e);
            j.key("result").num(c);
            j.key("result_name").str(CRYPT_NAMES[c as usize]);
            j.key("clipboard_set").boolean(false);
        }
    }
    Ok(())
}

fn cmd_crypt_dec(a: &Args, j: &mut J, gl: &Globals) -> R<()> {
    let pw = a.wstr("passw", "", true)?;
    let text = a.wstr("text", "", true)?;
    let cp = a.num("codepage", gl.acp as i64)? as u32;
    j.key("codepage").num(cp as i64);
    // versions 0 and 1: password in the given code page (as the reference
    // harness does; the app uses CP_ACP = ansi_encode_lossy())
    text_file::set_ansi_encoding(encoding_for_codepage(cp)?);
    let ansi = text_file::ansi_encode_lossy(&pw);
    text_file::set_ansi_encoding(encoding_for_codepage(gl.acp)?);
    let utf16: Vec<u8> = pw.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let try_version = |v: u8| {
        let p: &[u8] = if v < 2 { &ansi } else { &utf16 };
        crypt_text::decrypt_text(&text, p, v)
    };
    let mut attempts = Vec::new();
    let result;
    let mut version;
    if a.flag("old") {
        version = 0u8;
        loop {
            let r = try_version(version);
            let code = r.as_ref().err().map(|e| crypt_err_code(*e)).unwrap_or(0);
            attempts.push((version, code));
            if matches!(r, Err(CryptTextError::TextCorrupted | CryptTextError::BadKey))
                && version + 1 < crypt_text::CRYPTTEXT_VERSION
            {
                version += 1;
                continue;
            }
            result = r;
            break;
        }
    } else {
        version = a.num("version", crypt_text::CRYPTTEXT_VERSION as i64)? as u8;
        let r = try_version(version);
        attempts.push((version, r.as_ref().err().map(|e| crypt_err_code(*e)).unwrap_or(0)));
        result = r;
    }
    j.key("attempts").begin_array();
    for (v, c) in &attempts {
        j.begin_object();
        j.key("version").num(*v as i64);
        j.key("result").num(*c);
        j.end_object();
    }
    j.end_array();
    match result {
        Ok(t) => {
            j.key("result").num(0);
            j.key("result_name").str("OK");
            j.key("clipboard_set").boolean(true);
            j.key("text").str(&t.text);
            j.key("via_ansi").boolean(version < 2);
        }
        Err(e) => {
            let c = crypt_err_code(e);
            j.key("result").num(c);
            j.key("result_name").str(CRYPT_NAMES[c as usize]);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// databases

fn spec_path(p: &str, dir: &Path) -> Option<PathBuf> {
    if p.is_empty() {
        return None;
    }
    let q = Path::new(p);
    Some(if q.is_absolute() { q.to_path_buf() } else { dir.join(q) })
}

fn sha256_hex(b: &[u8]) -> String {
    to_hex(&crypto::sha256(b))
}

fn cmd_db_create(a: &Args, j: &mut J, gl: &mut Globals) -> R<()> {
    let spec_file = PathBuf::from(a.req("spec")?);
    let spec_dir = spec_file.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let text = std::fs::read(&spec_file).map_err(|_| herr("cannot open spec file"))?;
    let spec = parse_json(&text)?;
    if !matches!(spec, JV::Obj(_)) {
        return Err(herr("spec must be a JSON object"));
    }
    let out = PathBuf::from(a.req("out")?);

    if j_has(&spec, "rand_seed_hex") && gl.det_log.is_none() {
        let seed = from_hex(&j_str(&spec, "rand_seed_hex")?)?;
        gl.det_log = Some(enable_det_pool(&seed));
    }
    j.key("deterministic").boolean(gl.det_log.is_some());

    let mut db = PasswDatabase::new();
    let utf8 = j_bool(&spec, "passw_utf8", true)?;
    db.passw_utf8 = utf8;
    db.set_cipher_type(j_u64(&spec, "cipher", 0)? as u8).map_err(db_err)?;
    db.set_kdf_iterations(j_u64(&spec, "kdf_iterations", database::KEY_HASH_ITERATIONS as u64)? as u32).map_err(db_err)?;

    let key_file = spec_path(&j_str(&spec, "keyfile")?, &spec_dir);
    let key = database::combine_key_sources(&database::passw_binary(&j_str(&spec, "password")?, utf8), key_file.as_deref())
        .map_err(db_err)?;
    db.create(&key).map_err(db_err)?;

    db.default_user_name = Zeroizing::new(j_str(&spec, "default_user_name")?);
    db.passw_format_seq = Zeroizing::new(j_str(&spec, "passw_format_seq")?);
    db.description = Zeroizing::new(j_str(&spec, "description")?);
    db.default_passw_expiry_days = j_u64(&spec, "default_passw_expiry_days", 0)? as u32;
    db.default_max_passw_history_size = j_u64(&spec, "default_max_passw_history_size", 0)? as u32;
    db.master_passw_expiry_date = j_expiry(&spec, "master_passw_expiry_date")?;
    db.compressed = j_bool(&spec, "compressed", false)?;
    db.compression_level = j_u64(&spec, "compression_level", 0)? as i32;

    if let Some(JV::Arr(entries)) = spec.get("entries") {
        for e in entries {
            let idx = db.new_entry();
            {
                let en = db.entry_mut(idx);
                en.creation_time = j_u64(e, "creation_time", 0)?;
                en.modification_time = j_u64(e, "modification_time", 0)?;
                en.passw_change_time = j_u64(e, "passw_change_time", 0)?;
            }
            const KEYS: [&str; 6] = ["title", "user_name", "password", "url", "keyword", "notes"];
            for (i, k) in KEYS.iter().enumerate() {
                if i == 2 {
                    db.set_entry_passw(idx, &j_str(e, "password")?);
                } else if i != 1 || j_has(e, k) {
                    db.entry_mut(idx).strings[i] = Zeroizing::new(j_str(e, k)?);
                }
            }
            if let Some(JV::Arr(kv)) = e.get("key_values") {
                let mut list = Vec::new();
                for p in kv {
                    match p {
                        JV::Arr(x) if x.len() == 2 => {
                            let s = |v: &JV| match v {
                                JV::Str(s) => s.clone(),
                                _ => String::new(),
                            };
                            list.push((Zeroizing::new(s(&x[0])), Zeroizing::new(s(&x[1]))));
                        }
                        _ => return Err(herr("spec: key_values items must be [key, value]")),
                    }
                }
                db.entry_mut(idx).set_key_value_list(list);
            }
            if let Some(JV::Arr(tags)) = e.get("tags") {
                let mut set = BTreeSet::new();
                for t in tags {
                    if let JV::Str(s) = t {
                        set.insert(Tag(s.clone()));
                    } else {
                        set.insert(Tag(String::new()));
                    }
                }
                db.entry_mut(idx).set_tags(set);
            }
            db.entry_mut(idx).passw_expiry_date = j_expiry(e, "passw_expiry_date")?;
            if let Some(h @ JV::Obj(_)) = e.get("history") {
                let en = db.entry_mut(idx);
                let hist = en.passw_history_mut();
                hist.set_active(true);
                hist.set_max_size(j_u64(h, "max_size", 1)? as u32);
                hist.clear();
                if let Some(JV::Arr(he)) = h.get("entries") {
                    for x in he {
                        hist.add_entry(j_u64(x, "time", 0)?, Zeroizing::new(j_str(x, "password")?), false);
                    }
                }
                hist.set_active(j_bool(h, "active", true)?);
            }
        }
    }

    if j_has(&spec, "recovery_password") || j_has(&spec, "recovery_keyfile") {
        let rk = spec_path(&j_str(&spec, "recovery_keyfile")?, &spec_dir);
        let rkey = database::combine_key_sources(
            &database::passw_binary(&j_str(&spec, "recovery_password")?, utf8),
            rk.as_deref(),
        )
        .map_err(db_err)?;
        db.set_recovery_key(&key, &rkey).map_err(db_err)?;
    }

    db.save(&out).map_err(db_err)?;
    db.close();

    let f = std::fs::read(&out).map_err(|_| herr("cannot read output"))?;
    j.key("out").str(&out.to_string_lossy());
    j.key("file_size").num(f.len() as i64);
    j.key("file_sha256").str(&sha256_hex(&f));
    if !a.flag("digest") && f.len() <= 1 << 20 {
        j.key("file_hex").hex(&f);
    }
    Ok(())
}

fn write_file_layout(j: &mut J, f: &[u8]) {
    j.key("file_size").num(f.len() as i64);
    j.key("file_sha256").str(&sha256_hex(f));
    if f.len() < 18 {
        return;
    }
    let u32_at = |o: usize| u32::from_le_bytes([f[o], f[o + 1], f[o + 2], f[o + 3]]) as u64;
    j.key("header").begin_object();
    j.key("magic").unum(u32_at(0));
    j.key("header_size").num(f[4] as i64);
    j.key("version").num((f[5] as i64) | ((f[6] as i64) << 8));
    j.key("flags").unum(u32_at(7));
    j.key("cipher_type").num(f[11] as i64);
    j.key("hash_type").num(f[12] as i64);
    j.key("kdf_type").num(f[13] as i64);
    j.key("kdf_iterations").unum(u32_at(14));
    j.end_object();
    let mut pos = f[4] as usize;
    let version = (f[5] as u32) | ((f[6] as u32) << 8);
    let flags = u32_at(7);
    if version != 0x108 || pos > f.len() {
        return;
    }
    let iv_len = if f[11] == 1 { 8 } else { 16 };
    let hmac_len = if f[12] == 1 { 64 } else { 32 };
    j.key("layout").begin_object();
    if flags & 1 != 0 {
        if pos + 128 > f.len() {
            j.end_object();
            return;
        }
        j.key("recovery_block").hex(&f[pos..pos + 128]);
        pos += 128;
    }
    if pos + 32 + iv_len + hmac_len <= f.len() {
        j.key("salt").hex(&f[pos..pos + 32]);
        pos += 32;
        j.key("iv").hex(&f[pos..pos + iv_len]);
        pos += iv_len;
        j.key("ciphertext_len").num((f.len() - pos - hmac_len) as i64);
        j.key("hmac").hex(&f[f.len() - hmac_len..]);
    }
    j.end_object();
}

fn cmd_db_dump(a: &Args, j: &mut J) -> R<()> {
    let input = PathBuf::from(a.req("in")?);
    let f = std::fs::read(&input).map_err(|_| herr("cannot open file"))?;
    write_file_layout(j, &f);
    if a.flag("header-only") {
        return Ok(());
    }
    let key_file = a.str("keyfile").map(PathBuf::from);
    let pw = a.wstr("passw", "", false)?;
    let mut db = PasswDatabase::new();
    db.read_header(&input).map_err(db_err)?;
    let mut utf8 = db.passw_utf8;
    if a.flag("utf16-passw") {
        utf8 = false;
    }
    let key = database::combine_key_sources(&database::passw_binary(&pw, utf8), key_file.as_deref()).map_err(db_err)?;
    db.open(&key, &input).map_err(db_err)?;

    j.key("db").begin_object();
    j.key("version").num(db.version() as i64);
    j.key("cipher_type").num(db.cipher_type() as i64);
    j.key("kdf_iterations").unum(db.kdf_iterations() as u64);
    j.key("has_recovery_key").boolean(db.has_recovery_key());
    j.key("compressed").boolean(db.compressed);
    j.key("compression_level").num(db.compression_level as i64);
    j.key("passw_utf8").boolean(db.passw_utf8);
    j.key("default_user_name").str(&db.default_user_name);
    j.key("passw_format_seq").str(&db.passw_format_seq);
    j.key("description").str(&db.description);
    j.key("default_passw_expiry_days").unum(db.default_passw_expiry_days as u64);
    j.key("default_max_passw_history_size").unum(db.default_max_passw_history_size as u64);
    j.key("master_passw_expiry_date").unum(db.master_passw_expiry_date as u64);
    j.key("size").unum(db.len() as u64);
    let check = db.check_master_key(&key).map_err(db_err)?;
    j.key("check_master_key").boolean(check);
    j.end_object();

    j.key("entries").begin_array();
    for i in 0..db.len() {
        let pw = db.get_entry_passw(i).map_err(db_err)?;
        let e = db.entry(i);
        j.begin_object();
        j.key("index").unum(e.index() as u64);
        j.key("id").unum(e.id() as u64);
        j.key("title").str(&e.strings[0]);
        j.key("user_name").str(&e.strings[1]);
        j.key("password").str(&pw);
        j.key("url").str(&e.strings[3]);
        j.key("keyword").str(&e.strings[4]);
        j.key("notes").str(&e.strings[5]);
        j.key("key_value_string").str(&e.strings[6]);
        j.key("key_values").begin_array();
        for (k, v) in e.key_value_list() {
            j.begin_array();
            j.str(k);
            j.str(v);
            j.end_array();
        }
        j.end_array();
        j.key("tags_string").str(&e.strings[7]);
        j.key("tags").begin_array();
        for t in e.tags() {
            j.str(&t.0);
        }
        j.end_array();
        j.key("creation_time").unum(e.creation_time);
        j.key("modification_time").unum(e.modification_time);
        j.key("passw_change_time").unum(e.passw_change_time);
        j.key("passw_expiry_date").unum(e.passw_expiry_date as u64);
        let h = e.passw_history();
        j.key("history").begin_object();
        j.key("active").boolean(h.active());
        j.key("max_size").unum(h.max_size() as u64);
        j.key("entries").begin_array();
        for (t, p) in h.iter() {
            j.begin_object();
            j.key("time").unum(*t);
            j.key("password").str(p);
            j.end_object();
        }
        j.end_array();
        j.end_object();
        j.key("strings_locale").begin_object();
        j.key("creation_time").str(&e.creation_time_string());
        j.key("modification_time").str(&e.modification_time_string());
        j.key("passw_change_time").str(&e.passw_change_time_string());
        j.key("passw_expiry_date").str(&e.passw_expiry_date_string());
        j.end_object();
        j.end_object();
    }
    j.end_array();
    Ok(())
}

fn cmd_keyfile(a: &Args, j: &mut J) -> R<()> {
    let k = database::get_key_from_key_file(Path::new(a.req("in")?)).map_err(db_err)?;
    j.key("key").hex(&k[..]);
    Ok(())
}

fn cmd_keyfile_create(a: &Args, j: &mut J) -> R<()> {
    let out = PathBuf::from(a.req("out")?);
    database::create_key_file(&out).map_err(db_err)?;
    let f = std::fs::read(&out).map_err(|_| herr("cannot read key file"))?;
    j.key("out").str(&out.to_string_lossy());
    j.key("content").str(&String::from_utf8_lossy(&f));
    let k = database::get_key_from_key_file(&out).map_err(db_err)?;
    j.key("key").hex(&k[..]);
    Ok(())
}

fn cmd_trigram(a: &Args, j: &mut J, gl: &Globals) -> R<()> {
    let src = PathBuf::from(a.req("src")?);
    let dst = PathBuf::from(a.req("dst")?);
    let (sigma, entropy) = PasswordGenerator::create_trigram_file(&src, &dst, gl.file_encoding).map_err(|e| HErr {
        class: match e {
            TrigramError::Io(_) => "EStreamError".into(),
            TrigramError::NotEnoughTrigrams => "Exception".into(),
        },
        msg: e.to_string(),
    })?;
    j.key("sigma").unum(sigma as u64);
    j.dbl_with_bits("entropy", entropy);
    let f = std::fs::read(&dst).map_err(|_| herr("cannot read trigram file"))?;
    j.key("file_size").num(f.len() as i64);
    j.key("file_sha256").str(&sha256_hex(&f));
    let mut g = PasswordGenerator::new();
    let r = match g.load_trigram_file(Some(&dst)) {
        Ok(true) => 1,
        Ok(false) => 0,
        Err(_) => -1,
    };
    j.key("load_result").num(r);
    j.dbl_with_bits("phonetic_entropy_after_load", g.phonetic_entropy());
    Ok(())
}

fn cmd_mppg(a: &Args, j: &mut J) -> R<()> {
    use pwtool_core::mp_passw_gen as mp;
    let pw = a.wstr("passw", "", true)?;
    let param = a.wstr("param", "", false)?;
    let idx = a.num("charset-index", 0)?;
    let len = a.num("length", 16)? as i32;
    let hashapass = a.flag("hashapass");
    if !(0..6).contains(&idx) {
        return Err(herr("--charset-index must be 0..5"));
    }
    let key = mp::generator_key(&pw, hashapass);
    j.key("key").hex(&key);
    j.key("passw_hash").begin_object();
    for (i, n) in ["dec99", "dec9999", "hex16", "hex32"].iter().enumerate() {
        j.key(n).str(&mp::passw_checksum(&pw, mp::PasswHashType::from_index(i as i32)));
    }
    j.end_object();
    let (p, bits) = mp::generate(&key, &param, idx as usize, len, a.flag("add-length"), hashapass);
    j.key("password").str(&p);
    j.dbl_with_bits("bits", bits);
    j.key("length").num(if hashapass { 8 } else { len as i64 });
    Ok(())
}

// ---------------------------------------------------------------------------

fn usage() {
    eprintln!(
        "usage: portharness [--det-rand-seed HEX] [--file-encoding N] [--acp N] <command> [options]\n\
         commands: prng-aesctr pbkdf2 hmac-sha256 hmac-sha512 hmac-sha1 sha256 sha512 sha1 chacha gen\n\
         \x20         charsets estimate crypt-text-enc crypt-text-dec db-create db-dump keyfile\n\
         \x20         keyfile-create trigram-create mppg zxcvbn lua"
    );
}

fn run(argv: &[String], cmd_out: &mut String) -> Result<String, HErr> {
    let mut i = 0;
    let mut det_seed: Option<String> = None;
    let mut gl = Globals { file_encoding: CharEncoding::Utf8, acp: 0, det_log: None };
    let mut acp: Option<u32> = None;
    while i < argv.len() {
        match argv[i].as_str() {
            "--det-rand-seed" if i + 1 < argv.len() => {
                det_seed = Some(argv[i + 1].clone());
                i += 2;
            }
            "--file-encoding" if i + 1 < argv.len() => {
                let n = argv[i + 1].parse::<i32>().unwrap_or(0);
                gl.file_encoding = CharEncoding::from_index(n).ok_or_else(|| unsupported("file encoding"))?;
                i += 2;
            }
            "--acp" if i + 1 < argv.len() => {
                acp = Some(argv[i + 1].parse::<u32>().map_err(|_| herr("bad --acp"))?);
                i += 2;
            }
            _ => break,
        }
    }
    if i >= argv.len() {
        usage();
        std::process::exit(2);
    }
    let cmd = argv[i].clone();
    *cmd_out = cmd.clone();
    let a = Args::parse(&argv[i + 1..])?;

    gl.acp = match acp {
        Some(c) => c,
        None => {
            if text_file::ansi_encoding() == encoding_rs::WINDOWS_1251 {
                1251
            } else {
                1252
            }
        }
    };
    text_file::set_ansi_encoding(encoding_for_codepage(gl.acp)?);

    if let Some(s) = det_seed {
        gl.det_log = Some(enable_det_pool(&from_hex(&s)?));
    }

    let mut j = J::default();
    j.begin_object();
    j.key("ok").boolean(true);
    j.key("cmd").str(&cmd);
    j.key("acp").num(gl.acp as i64);
    match cmd.as_str() {
        "prng-aesctr" => cmd_prng(&a, &mut j)?,
        "pbkdf2" => cmd_pbkdf2(&a, &mut j)?,
        "hmac-sha256" => cmd_hmac(&a, &mut j, 256)?,
        "hmac-sha512" => cmd_hmac(&a, &mut j, 512)?,
        "hmac-sha1" => cmd_hmac(&a, &mut j, 160)?,
        "sha256" => cmd_hash(&a, &mut j, 256)?,
        "sha512" => cmd_hash(&a, &mut j, 512)?,
        "sha1" => cmd_hash(&a, &mut j, 160)?,
        "chacha" => cmd_chacha(&a, &mut j)?,
        "gen" => cmd_gen(&a, &mut j, &gl, None)?,
        "charsets" => cmd_charsets(&a, &mut j, &gl)?,
        "estimate" => cmd_estimate(&a, &mut j)?,
        "crypt-text-enc" => cmd_crypt_enc(&a, &mut j)?,
        "crypt-text-dec" => cmd_crypt_dec(&a, &mut j, &gl)?,
        "db-create" => cmd_db_create(&a, &mut j, &mut gl)?,
        "db-dump" => cmd_db_dump(&a, &mut j)?,
        "keyfile" => cmd_keyfile(&a, &mut j)?,
        "keyfile-create" => cmd_keyfile_create(&a, &mut j)?,
        "trigram-create" => cmd_trigram(&a, &mut j, &gl)?,
        "mppg" => cmd_mppg(&a, &mut j)?,
        "zxcvbn" => cmd_zxcvbn(&a, &mut j)?,
        "lua" => {
            if !a.has("script") {
                return Err(herr("missing required option --script"));
            }
            cmd_gen(&a, &mut j, &gl, Some("main"))?
        }
        _ => {
            usage();
            return Err(herr(format!("unknown command '{cmd}'")));
        }
    }
    if let Some(log) = &gl.det_log {
        j.key("randpool_log").begin_array();
        for e in log.lock().unwrap().iter() {
            j.hex(e);
        }
        j.end_array();
    }
    j.end_object();
    Ok(j.out)
}

fn main() {
    // dates and times in the user's locale, as the program does
    pwtool_core::util::init_time_locale();
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut cmd = String::new();
    // catch panics of the port, too (reported as exception "panic")
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&argv, &mut cmd)));
    let (out, code) = match r {
        Ok(Ok(s)) => (s, 0),
        Ok(Err(e)) => {
            let mut j = J::default();
            j.begin_object();
            j.key("ok").boolean(false);
            j.key("cmd").str(&cmd);
            j.key("exception").str(&e.class);
            j.key("error").str(&e.msg);
            j.end_object();
            (j.out, 1)
        }
        Err(p) => {
            let msg = p
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            let mut j = J::default();
            j.begin_object();
            j.key("ok").boolean(false);
            j.key("cmd").str(&cmd);
            j.key("exception").str("panic");
            j.key("error").str(&msg);
            j.end_object();
            (j.out, 1)
        }
    };
    println!("{out}");
    std::process::exit(code);
}
