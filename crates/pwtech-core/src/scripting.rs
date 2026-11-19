//! Password scripting with Lua 5.5 (`upstream/src/util/Scripting.*`).
//!
//! A script defines `generate(num, [passw, sec])` returning a password and its
//! entropy, optionally `init(numPassw, destType, genFlags, advancedFlags,
//! numChars, numWords, format)` and `script_flags` (1 = the script generates
//! passwords on its own). The functions `pwtech_random`, `pwtech_password`,
//! `pwtech_phonetic`, `pwtech_passphrase`, `pwtech_word`, `pwtech_numwords`
//! and `pwtech_format` give access to the password generator.
//!
//! The original runs scripts in a separate thread and kills it if the user
//! cancels; here an instruction-count hook checks a cancel flag instead.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use mlua::{HookTriggers, Lua, MultiValue, Value, VmState};
use zeroize::Zeroizing;

use crate::language::trl_format;
use crate::passw::generator::{PASSW_FLAG_PHONETICMIXEDCASE, PasswordGenerator};
use crate::random::RandomGenerator;
use crate::unicode::{str_to_w32, w32_to_string};

const MAX_PASSW_CHARS: usize = 16000;
const E_LUA_ERROR: &str = "Lua error: ";
const E_UNKNOWN_FUNC: &str = "Required function \"%1\" not defined in script";
const MAX_SCRIPT_SIZE: u64 = 1_485_760;

/// Script flag: the script generates passwords without input
/// (`FLAG_STANDALONE_PASSW_GEN`).
pub const FLAG_STANDALONE_PASSW_GEN: i32 = 1;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct LuaError(pub String);

/// `randbl_52new()`: double in [0, 1) from two 32-bit values (Doornik).
fn randbl_52new(rng: &mut dyn RandomGenerator) -> f64 {
    const M_RAN_INVM32: f64 = 2.328_306_436_538_696_289_06e-10;
    const M_RAN_INVM52: f64 = 2.220_446_049_250_313_080_85e-16;
    let r1 = rng.get_word32() as i32;
    let r2 = rng.get_word32() as i32;
    r1 as f64 * M_RAN_INVM32 + (0.5 + M_RAN_INVM52 / 2.0) + (r2 & 0x000F_FFFF) as f64 * M_RAN_INVM52
}

/// `lua_tointeger()` truncated to a C `int`.
fn to_int(lua: &Lua, v: Option<&Value>) -> i32 {
    match v {
        Some(v) => lua.coerce_integer(v.clone()).ok().flatten().unwrap_or(0) as i32,
        None => 0,
    }
}

fn to_str(lua: &Lua, v: Option<&Value>) -> Option<String> {
    let v = v?;
    lua.coerce_string(v.clone()).ok().flatten().map(|s| s.to_string_lossy())
}

/// A loaded script (`LuaScript`).
pub struct LuaScript {
    lua: Lua,
    file_name: PathBuf,
    flags: i32,
    initialized: bool,
    cancel: Arc<AtomicBool>,
}

fn lua_error_message(e: &mlua::Error) -> String {
    let (kind, msg) = match e {
        mlua::Error::SyntaxError { message, .. } => ("Syntax error during precompilation", message.clone()),
        mlua::Error::RuntimeError(m) => ("Runtime error", m.clone()),
        mlua::Error::MemoryError(m) => ("Out of memory", m.clone()),
        mlua::Error::CallbackError { traceback, cause } => {
            return format!("{E_LUA_ERROR}Runtime error\n{}\n{traceback}", lua_error_message(cause).trim_start_matches(E_LUA_ERROR));
        }
        other => ("Runtime error", other.to_string()),
    };
    if msg.is_empty() { format!("{E_LUA_ERROR}{kind}") } else { format!("{E_LUA_ERROR}{kind}\n{msg}") }
}

impl LuaScript {
    /// New interpreter with the standard libraries.
    pub fn new() -> Self {
        let lua = Lua::new();
        let cancel = Arc::new(AtomicBool::new(false));
        let c = cancel.clone();
        let _ = lua.set_hook(HookTriggers::new().every_nth_instruction(10_000), move |_, _| {
            if c.load(Ordering::Relaxed) {
                Err(mlua::Error::runtime("Script cancelled"))
            } else {
                Ok(VmState::Continue)
            }
        });
        LuaScript { lua, file_name: PathBuf::new(), flags: 0, initialized: false, cancel }
    }

    /// Flag that aborts a running script when set.
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }

    pub fn file_name(&self) -> &Path {
        &self.file_name
    }

    /// `script_flags` of the loaded script.
    pub fn flags(&self) -> i32 {
        self.flags
    }

    /// Run `f` with the `pwtech_*` functions bound to `pg` and `rng`.
    fn with_api<R>(
        &self,
        pg: Option<&PasswordGenerator>,
        rng: &mut dyn RandomGenerator,
        f: impl FnOnce(&Lua) -> mlua::Result<R>,
    ) -> mlua::Result<R> {
        let rng = RefCell::new(rng);
        let lua = &self.lua;
        lua.scope(|scope| {
            let g = lua.globals();
            let rngr = &rng;
            g.set(
                "pwtech_random",
                scope.create_function(move |lua, args: MultiValue| {
                    let mut r = rngr.borrow_mut();
                    if args.is_empty() {
                        return Ok(MultiValue::from_vec(vec![Value::Number(randbl_52new(&mut **r))]));
                    }
                    let mut start = 1;
                    let mut end = to_int(lua, args.front());
                    if args.len() >= 2 {
                        start = end;
                        end = to_int(lua, args.get(1));
                    }
                    if end < start {
                        return Ok(MultiValue::new());
                    }
                    let range = (end as i64 - start as i64 + 1) as u32;
                    let v = r.get_num_range(range) as i64 + start as i64;
                    Ok(MultiValue::from_vec(vec![Value::Integer(v as i32 as i64)]))
                })?,
            )?;
            g.set(
                "pwtech_password",
                scope.create_function(move |lua, args: MultiValue| {
                    let Some(pg) = pg else { return Ok(MultiValue::new()) };
                    if args.is_empty() {
                        return Ok(MultiValue::new());
                    }
                    let len = to_int(lua, args.front());
                    if len <= 0 {
                        return Ok(MultiValue::new());
                    }
                    let flags = to_int(lua, args.get(1)) as u32;
                    let p = pg.get_password(&mut **rngr.borrow_mut(), len.min(MAX_PASSW_CHARS as i32), flags);
                    let s = Zeroizing::new(w32_to_string(&p));
                    Ok(MultiValue::from_vec(vec![
                        Value::String(lua.create_string(s.as_bytes())?),
                        Value::Number(pg.custom_char_set_entropy() * len as f64),
                    ]))
                })?,
            )?;
            g.set(
                "pwtech_passphrase",
                scope.create_function(move |lua, args: MultiValue| {
                    let Some(pg) = pg else { return Ok(MultiValue::new()) };
                    if args.is_empty() {
                        return Ok(MultiValue::new());
                    }
                    let words = to_int(lua, args.front()).min(100);
                    if words <= 0 {
                        return Ok(MultiValue::new());
                    }
                    let chars = to_str(lua, args.get(1)).unwrap_or_default();
                    let chars = Zeroizing::new(str_to_w32(&chars));
                    let flags = to_int(lua, args.get(2)) as u32;
                    let (p, _) = pg.get_passphrase(&mut **rngr.borrow_mut(), words, &chars, flags);
                    let s = Zeroizing::new(w32_to_string(&p));
                    Ok(MultiValue::from_vec(vec![
                        Value::String(lua.create_string(s.as_bytes())?),
                        Value::Number(pg.word_list_entropy() * words as f64),
                    ]))
                })?,
            )?;
            g.set(
                "pwtech_phonetic",
                scope.create_function(move |lua, args: MultiValue| {
                    let Some(pg) = pg else { return Ok(MultiValue::new()) };
                    if args.is_empty() {
                        return Ok(MultiValue::new());
                    }
                    let len = to_int(lua, args.front()).min(MAX_PASSW_CHARS as i32);
                    if len <= 0 {
                        return Ok(MultiValue::new());
                    }
                    let flags = to_int(lua, args.get(1)) as u32;
                    let p = pg.get_phonetic_passw(&mut **rngr.borrow_mut(), len, flags);
                    let s = Zeroizing::new(w32_to_string(&p));
                    let e = pg.phonetic_entropy();
                    let sec = len as f64 * if flags & PASSW_FLAG_PHONETICMIXEDCASE != 0 { e + 1.0 } else { e };
                    Ok(MultiValue::from_vec(vec![
                        Value::String(lua.create_string(s.as_bytes())?),
                        Value::Number(sec),
                    ]))
                })?,
            )?;
            g.set(
                "pwtech_word",
                scope.create_function(move |lua, args: MultiValue| {
                    let Some(pg) = pg else { return Ok(MultiValue::new()) };
                    let index = if args.is_empty() {
                        rngr.borrow_mut().get_num_range(pg.word_list_size() as u32) as i32
                    } else {
                        let i = to_int(lua, args.front()) - 1;
                        if i < 0 || i >= pg.word_list_size() {
                            return Ok(MultiValue::new());
                        }
                        i
                    };
                    let w = pg.get_word(index);
                    Ok(MultiValue::from_vec(vec![Value::String(lua.create_string(w.as_bytes())?)]))
                })?,
            )?;
            g.set(
                "pwtech_numwords",
                scope.create_function(move |_, _: MultiValue| {
                    let Some(pg) = pg else { return Ok(MultiValue::new()) };
                    Ok(MultiValue::from_vec(vec![Value::Integer(pg.word_list_size() as i64)]))
                })?,
            )?;
            g.set(
                "pwtech_format",
                scope.create_function(move |lua, args: MultiValue| {
                    let Some(pg) = pg else { return Ok(MultiValue::new()) };
                    if args.is_empty() {
                        return Ok(MultiValue::new());
                    }
                    let Some(fmt) = to_str(lua, args.front()).filter(|s| !s.is_empty()) else {
                        return Ok(MultiValue::new());
                    };
                    let flags = to_int(lua, args.get(1)) as u32;
                    let mut sec = 0.0;
                    let res = pg.get_format_passw(
                        &mut **rngr.borrow_mut(),
                        &str_to_w32(&fmt),
                        flags,
                        None,
                        MAX_PASSW_CHARS,
                        Some(&mut sec),
                    );
                    if res.passw.is_empty() {
                        return Ok(MultiValue::new());
                    }
                    let s = Zeroizing::new(w32_to_string(&res.passw));
                    Ok(MultiValue::from_vec(vec![
                        Value::String(lua.create_string(s.as_bytes())?),
                        Value::Number(sec),
                    ]))
                })?,
            )?;
            f(lua)
        })
    }

    /// `LoadFile()`: load and run the script's top-level code.
    pub fn load_file(
        &mut self,
        path: &Path,
        pg: Option<&PasswordGenerator>,
        rng: &mut dyn RandomGenerator,
    ) -> Result<(), LuaError> {
        let meta = std::fs::metadata(path).map_err(|e| LuaError(e.to_string()))?;
        if meta.len() > MAX_SCRIPT_SIZE {
            return Err(LuaError("Script file too large".into()));
        }
        let mut data = std::fs::read(path).map_err(|e| LuaError(e.to_string()))?;
        // the BOM is skipped (TStringFileStreamW with BOM detection)
        let bom = if data.starts_with(&crate::text_file::UTF8_BOM) {
            3
        } else if data.starts_with(&crate::text_file::UNICODE_BOM) || data.starts_with(&crate::text_file::UNICODE_BOM_SWAPPED) {
            2
        } else {
            0
        };
        data.drain(..bom);
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        self.cancel.store(false, Ordering::Relaxed);
        self.with_api(pg, rng, |lua| lua.load(&data[..]).set_name(format!("={name}")).exec())
            .map_err(|e| LuaError(lua_error_message(&e)))?;
        self.flags = match self.lua.globals().get::<Value>("script_flags") {
            Ok(Value::Integer(i)) => i as i32,
            Ok(Value::Number(n)) => n as i32,
            _ => 0,
        };
        self.file_name = path.to_path_buf();
        self.initialized = false;
        Ok(())
    }

    /// Mark that `init()` must be called before the next `generate()` (start
    /// of a new generation run).
    pub fn reset_init(&mut self) {
        self.initialized = false;
    }

    /// `InitGenerate()`: call `init()` if defined.
    #[allow(clippy::too_many_arguments)]
    pub fn init_generate(
        &mut self,
        pg: &PasswordGenerator,
        rng: &mut dyn RandomGenerator,
        num_passw: u64,
        dest_type: i32,
        gen_flags: i32,
        advanced_flags: u32,
        num_chars: i32,
        num_words: i32,
        format: &str,
    ) -> Result<bool, LuaError> {
        self.initialized = true;
        let f: Option<mlua::Function> = self.lua.globals().get("init").ok();
        let Some(f) = f else { return Ok(false) };
        self.with_api(Some(pg), rng, |_| {
            f.call::<()>((num_passw as i64, dest_type, gen_flags, advanced_flags as i64, num_chars, num_words, format))
        })
        .map_err(|e| LuaError(lua_error_message(&e)))?;
        Ok(true)
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// `CallGenerate()`: returns the script's password (at most 16000
    /// characters) and entropy.
    pub fn call_generate(
        &mut self,
        pg: &PasswordGenerator,
        rng: &mut dyn RandomGenerator,
        num: u64,
        src_passw: Option<&str>,
        src_sec: f64,
    ) -> Result<(Zeroizing<String>, f64), LuaError> {
        let f: Option<mlua::Function> = self.lua.globals().get("generate").ok();
        let Some(f) = f else {
            return Err(LuaError(format!("{E_LUA_ERROR}{}", trl_format(E_UNKNOWN_FUNC, &["generate"]))));
        };
        let (s, sec) = self
            .with_api(Some(pg), rng, |lua| {
                let mut args = vec![Value::Integer(num as i64)];
                if let Some(p) = src_passw.filter(|p| !p.is_empty()) {
                    args.push(Value::String(lua.create_string(p.as_bytes())?));
                    args.push(Value::Number(src_sec));
                }
                let ret: MultiValue = f.call(MultiValue::from_vec(args))?;
                let s = to_str(lua, ret.front()).unwrap_or_default();
                let sec = match ret.get(1) {
                    Some(v) => lua.coerce_number(v.clone()).ok().flatten().unwrap_or(0.0),
                    None => 0.0,
                };
                Ok((Zeroizing::new(s), sec))
            })
            .map_err(|e| LuaError(lua_error_message(&e)))?;
        let mut s = s;
        if s.chars().count() > MAX_PASSW_CHARS {
            let mut t = Zeroizing::new(String::new());
            let mut units = 0;
            for c in s.chars() {
                units += c.len_utf16();
                if units > MAX_PASSW_CHARS {
                    break;
                }
                t.push(c);
            }
            s = t;
        }
        Ok((s, sec))
    }
}

impl Default for LuaScript {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::random::AesCtrPrng;

    fn script(src: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("pwtech-lua-{}-{}.lua", std::process::id(), src.len()));
        std::fs::write(&p, src).unwrap();
        p
    }

    #[test]
    fn api_functions() {
        let pg = PasswordGenerator::new();
        let mut rng = AesCtrPrng::from_seed(b"lua");
        let p = script(
            r#"
            script_flags = 1
            function generate(num)
              local p, s = pwtech_password(8)
              local w = pwtech_word(1)
              local r = pwtech_random(3, 5)
              local f = pwtech_format("4d")
              return p .. "-" .. w .. "-" .. r .. "-" .. f .. "-" .. pwtech_numwords(), s
            end
            "#,
        );
        let mut s = LuaScript::new();
        s.load_file(&p, Some(&pg), &mut rng).unwrap();
        assert_eq!(s.flags(), FLAG_STANDALONE_PASSW_GEN);
        let (pw, sec) = s.call_generate(&pg, &mut rng, 1, None, 0.0).unwrap();
        let parts: Vec<&str> = pw.split('-').collect();
        assert_eq!(parts[0].len(), 8);
        assert_eq!(parts[1], "a");
        assert!(["3", "4", "5"].contains(&parts[2]));
        assert_eq!(parts[3].len(), 4);
        assert_eq!(parts[4], "8192");
        assert!((sec - 8.0 * 62f64.log2()).abs() < 1e-9);
        std::fs::remove_file(p).unwrap();
    }

    #[test]
    fn upstream_examples_run() {
        let pg = PasswordGenerator::new();
        let mut rng = AesCtrPrng::from_seed(b"lua");
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../upstream/script_examples");
        let Ok(rd) = std::fs::read_dir(&dir) else { return };
        for e in rd {
            let path = e.unwrap().path();
            let mut s = LuaScript::new();
            s.load_file(&path, Some(&pg), &mut rng).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            s.init_generate(&pg, &mut rng, 1, 0, 1 | 2, 0, 12, 4, "").unwrap();
            let (pw, _) = s
                .call_generate(&pg, &mut rng, 1, Some("abcdefghijkl"), 70.0)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert!(!pw.is_empty(), "{}", path.display());
        }
    }

    #[test]
    fn errors_and_cancel() {
        let pg = PasswordGenerator::new();
        let mut rng = AesCtrPrng::from_seed(b"lua");
        let p = script("function generate() error('boom') end -- x");
        let mut s = LuaScript::new();
        s.load_file(&p, Some(&pg), &mut rng).unwrap();
        let e = s.call_generate(&pg, &mut rng, 1, None, 0.0).unwrap_err();
        assert!(e.0.starts_with("Lua error: Runtime error"), "{}", e.0);
        let p2 = script("function generate() while true do end end -- infinite loop");
        let mut s = LuaScript::new();
        s.load_file(&p2, Some(&pg), &mut rng).unwrap();
        let c = s.cancel_flag();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            c.store(true, Ordering::Relaxed);
        });
        assert!(s.call_generate(&pg, &mut rng, 1, None, 0.0).is_err());
        let p3 = script("syntax error here");
        assert!(LuaScript::new().load_file(&p3, Some(&pg), &mut rng).unwrap_err().0.contains("Syntax error"));
        for f in [p, p2, p3] {
            let _ = std::fs::remove_file(f);
        }
    }
}
