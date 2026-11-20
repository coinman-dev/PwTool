//! Application state shared by the console mode and the GUI: paths, command
//! line, configuration file, language, random pool, password generator and
//! its settings. Mirrors the non-visual parts of `TMainForm` (`Main.cpp`) and
//! the start-up code in `PwTech.cpp`.

use std::path::{Path, PathBuf};

use pwtool_core::config::{
    CONFIG_ID, CHARSETLIST_DEFAULTENTRIES, GeneratorSettings, IniFile, PwGenProfile, WLFNLIST_DEFAULT, find_profile,
    load_profiles, remove_comment_and_trim, save_profiles,
};
use pwtool_core::generate::{
    CommonPasswords, GenDest, GenRequest, PASSWOPTION_EACHCHARONLYONCE, PASSWOPTION_EXCLUDEAMBIG,
    PASSWOPTION_INCLUDE_CHAR_FROM_EACH_SUBSET, PASSWOPTION_INCLUDESUBSET, PASSWOPTION_LOWERCASEWORDS, PassphraseLength,
    parse_passphrase_length,
};
use pwtool_core::language::{set_language, trl, trl_format};
use pwtool_core::language_file::LanguageFile;
use pwtool_core::passw::PasswordGenerator;
use pwtool_core::passw::generator::WordListError;
use pwtool_core::random::{EntropyManager, PoolCipherType, RandomPool};
use pwtool_core::text_file::CharEncoding;
use pwtool_core::util::format_float_1;

use crate::program::*;

/// Command line options (`CmdLineOptions`).
#[derive(Debug, Default, Clone)]
pub struct CmdLineOptions {
    pub ini_file_name: Option<PathBuf>,
    pub profile_name: String,
    pub unknown_switches: Vec<String>,
    pub passw_db_file_name: Option<PathBuf>,
    pub gen_num_passw: u64,
    pub config_readonly: bool,
    pub show_help: bool,
    pub silent: bool,
}

/// Parse the command line like `_tWinMain` in `PwTech.cpp`: switches start
/// with `-` or `--` (`/` is not accepted on Linux), the first non-switch
/// argument is the database to open.
pub fn parse_cmdline(args: &[String]) -> CmdLineOptions {
    let mut o = CmdLineOptions::default();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a.len() < 2 || !a.starts_with('-') {
            if o.passw_db_file_name.is_none() {
                o.passw_db_file_name = Some(PathBuf::from(a));
            }
            i += 1;
            continue;
        }
        let name = a.trim_start_matches('-');
        let next = args.get(i + 1);
        match name.to_ascii_lowercase().as_str() {
            "help" | "h" => o.show_help = true,
            "ini" => {
                if let Some(n) = next {
                    o.ini_file_name = Some(PathBuf::from(n));
                    i += 1;
                }
            }
            "readonly" => o.config_readonly = true,
            "profile" => {
                if let Some(n) = next {
                    o.profile_name = n.clone();
                    i += 1;
                }
            }
            "gen" => {
                let mut n = 1i64;
                if let Some(v) = next.and_then(|v| v.parse::<i64>().ok()).filter(|&v| v >= 0) {
                    n = v;
                    i += 1;
                }
                o.gen_num_passw = n.max(1) as u64;
            }
            "silent" => o.silent = true,
            "opendb" => {
                if let Some(n) = next {
                    o.passw_db_file_name = Some(PathBuf::from(n));
                    i += 1;
                }
            }
            _ => o.unknown_switches.push(format!("\"{name}\"")),
        }
        i += 1;
    }
    o
}

/// Directories used by the program.
#[derive(Debug, Clone)]
pub struct AppPaths {
    /// Directory of the executable.
    pub exe_dir: PathBuf,
    /// Read-only data shipped with the program (word lists, translations,
    /// common passwords, script examples).
    pub resources: PathBuf,
    /// Writable data: configuration, seed file, user translations
    /// (`g_sAppDataPath`).
    pub data_dir: PathBuf,
    pub ini_file: PathBuf,
}

impl AppPaths {
    /// Determine the paths. Portable mode as in the original: a `PwTool.ini`
    /// next to the executable is used unless it contains `UseAppDataPath=1`.
    pub fn detect(ini_override: Option<&Path>) -> Self {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."));
        let resources = Self::find_resources(&exe_dir);
        let config_home = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_else(|| PathBuf::from("."));
        let app_data = config_home.join("pwtool");

        if let Some(ini) = ini_override {
            let ini = if ini.parent().map(|p| p.as_os_str().is_empty()).unwrap_or(true) {
                exe_dir.join(ini)
            } else {
                ini.to_path_buf()
            };
            let data_dir = ini.parent().map(Path::to_path_buf).unwrap_or(app_data);
            return AppPaths { exe_dir, resources, data_dir, ini_file: ini };
        }
        let portable_ini = exe_dir.join(PROGRAM_INIFILE);
        if portable_ini.is_file() {
            let use_app_data =
                IniFile::load(&portable_ini).map(|i| i.read_bool(CONFIG_ID, "UseAppDataPath", false)).unwrap_or(false);
            if !use_app_data {
                return AppPaths { exe_dir: exe_dir.clone(), resources, data_dir: exe_dir, ini_file: portable_ini };
            }
        }
        AppPaths { exe_dir, resources, ini_file: app_data.join(PROGRAM_INIFILE), data_dir: app_data }
    }

    fn find_resources(exe_dir: &Path) -> PathBuf {
        if let Some(d) = std::env::var_os("PWTOOL_DATA_DIR") {
            return PathBuf::from(d);
        }
        let candidates = [
            exe_dir.join("../share/pwtool"),
            exe_dir.to_path_buf(),
            PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets")),
        ];
        for c in &candidates {
            if c.join("common_passwords.txt").is_file() {
                return c.clone();
            }
        }
        exe_dir.to_path_buf()
    }

    /// Resolve a file name without directory (`ExtractFilePath(…).IsEmpty()`
    /// → `g_sExePath + name` in the original): the data directory first,
    /// then the resources.
    pub fn resolve(&self, name: &str) -> PathBuf {
        let p = Path::new(name);
        if p.is_absolute() || p.parent().is_some_and(|d| !d.as_os_str().is_empty()) {
            return expand_home(name);
        }
        let d = self.data_dir.join(name);
        if d.exists() { d } else { self.resources.join(name) }
    }
}

fn expand_home(name: &str) -> PathBuf {
    if let Some(rest) = name.strip_prefix("~/") {
        if let Some(h) = std::env::var_os("HOME") {
            return PathBuf::from(h).join(rest);
        }
    }
    PathBuf::from(name)
}

/// An available translation (`LanguageEntry`).
#[derive(Debug, Clone)]
pub struct LanguageEntry {
    pub code: String,
    pub name: String,
    pub version: String,
    pub file_name: Option<PathBuf>,
}

/// Non-visual state of the main window.
pub struct AppCore {
    pub paths: AppPaths,
    pub cmd: CmdLineOptions,
    pub ini: IniFile,
    pub fake_ini: bool,
    /// `g_config`.
    pub config: crate::app_config::Configuration,
    pub settings: GeneratorSettings,
    pub profiles: Vec<PwGenProfile>,
    pub passw_gen: PasswordGenerator,
    pub common_passw: Option<CommonPasswords>,
    pub file_encoding: CharEncoding,
    pub newline: String,
    pub test_common_passw: bool,
    pub use_advanced_passw_est: bool,
    pub languages: Vec<LanguageEntry>,
    pub language: usize,
    pub help_file_name: PathBuf,
    pub startup_errors: Vec<String>,
    /// Text below the character set box (`m_sCharSetInfo`).
    pub char_set_info: String,
    pub char_set_error: bool,
    pub word_list_info: String,
    pub word_list_error: Option<String>,
}

impl AppCore {
    /// Start-up: configuration, language, seed file, generator settings,
    /// profiles, common passwords.
    pub fn new(cmd: CmdLineOptions) -> Self {
        pwtool_core::util::init_time_locale();
        pwtool_core::util::init_decimal_separator_from_locale();
        let paths = AppPaths::detect(cmd.ini_file_name.as_deref());
        if !cmd.config_readonly {
            let _ = std::fs::create_dir_all(&paths.data_dir);
        }
        let mut startup_errors = Vec::new();
        let (ini, fake_ini) = if paths.ini_file.exists() {
            match IniFile::load(&paths.ini_file) {
                Ok(i) => (i, false),
                Err(e) => {
                    startup_errors.push(format!(
                        "Could not load configuration file\n\"{}\":\n{e}",
                        paths.ini_file.display()
                    ));
                    (IniFile::new(), true)
                }
            }
        } else {
            (IniFile::new(), false)
        };

        // The original defaults to Windows line endings; on Linux LF is the
        // default unless PwTool.ini says otherwise (see `app_config`).
        let config = crate::app_config::Configuration::load(&ini);
        let file_encoding = config.file_encoding;
        let newline = config.newline().to_string();

        let mut app = AppCore {
            paths,
            cmd,
            settings: GeneratorSettings::default(),
            profiles: Vec::new(),
            passw_gen: PasswordGenerator::new(),
            common_passw: None,
            file_encoding,
            newline,
            test_common_passw: config.test_common_passw,
            use_advanced_passw_est: config.use_advanced_passw_est,
            config,
            languages: Vec::new(),
            language: 0,
            help_file_name: PathBuf::new(),
            startup_errors,
            char_set_info: String::new(),
            char_set_error: false,
            word_list_info: String::new(),
            word_list_error: None,
            ini,
            fake_ini,
        };

        app.load_lang_config();

        // random pool: cipher, seed file, system entropy
        {
            let mut pool = RandomPool::global();
            if let Some(c) = PoolCipherType::from_index(app.config.random_pool_cipher) {
                pool.set_cipher(c);
            }
            let seed = app.paths.data_dir.join(PROGRAM_RANDSEEDFILE);
            if seed.exists() && !pool.read_seed_file(&seed) {
                app.startup_errors.push(trl_format("Could not read random seed file\n\"%1\".", &[&seed.to_string_lossy()]));
            }
            pool.randomize_system();
        }
        EntropyManager::global().add_system_entropy(&mut RandomPool::global());
        app.write_rand_seed_file();

        if !app.cmd.unknown_switches.is_empty() {
            app.startup_errors.push(trl_format(
                "Unknown command line switch(es):\n%1",
                &[&(app.cmd.unknown_switches.join("; ") + ".")],
            ));
        }

        app.load_config();
        app
    }

    /// `WriteRandSeedFile()`.
    pub fn write_rand_seed_file(&self) {
        if self.cmd.config_readonly {
            return;
        }
        let seed = self.paths.data_dir.join(PROGRAM_RANDSEEDFILE);
        RandomPool::global().write_seed_file(&seed);
    }

    /// `LoadLangConfig()`: list translations and load the configured one.
    pub fn load_lang_config(&mut self) {
        self.languages.push(LanguageEntry {
            code: LANGUAGE_DEFAULT_CODE.into(),
            name: LANGUAGE_DEFAULT_NAME.into(),
            version: PROGRAM_VERSION.into(),
            file_name: None,
        });
        let dirs = [self.paths.resources.join("languages"), self.paths.resources.clone(), self.paths.data_dir.clone()];
        for dir in dirs {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            let mut files: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
            files.sort();
            for f in files {
                let ext = f.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                if ext != "po" && ext != "lng" {
                    continue;
                }
                match LanguageFile::load(&f, true) {
                    Ok(lf) => {
                        if self.languages.iter().any(|l| l.code == lf.code) {
                            continue;
                        }
                        self.languages.push(LanguageEntry {
                            code: lf.code,
                            name: lf.name,
                            version: lf.version,
                            file_name: Some(f),
                        });
                    }
                    Err(e) => {
                        let n = f.file_name().unwrap_or_default().to_string_lossy();
                        self.startup_errors.push(format!("{n}: {e}."));
                    }
                }
            }
        }
        let mut code = self.ini.read_string(CONFIG_ID, "Language", "");
        if code.is_empty() {
            code = system_language_code();
        }
        let idx = match self.languages.iter().position(|l| l.code == code) {
            Some(i) => i,
            None => {
                if !self.ini.read_string(CONFIG_ID, "Language", "").is_empty() {
                    self.startup_errors.push(format!("Could not find language \"{code}\"."));
                }
                0
            }
        };
        self.language = 0;
        if idx != 0 && self.change_language(idx) {
            self.language = idx;
        }
        self.config.language_code = self.languages[self.language].code.clone();
        self.help_file_name = self.paths.resources.join(PROGRAM_HELPFILE);
    }

    /// `ChangeLanguage()`.
    pub fn change_language(&mut self, idx: usize) -> bool {
        let Some(entry) = self.languages.get(idx).cloned() else { return false };
        let Some(file) = entry.file_name else {
            set_language(None);
            self.language = 0;
            return true;
        };
        match LanguageFile::load(&file, false) {
            Ok(mut lf) => {
                // supplementary translations of Password Tool
                // (languages/port/<code>.po)
                let extra = self.paths.resources.join("languages").join("port").join(format!("{}.po", lf.code));
                if let Ok(x) = LanguageFile::load(&extra, false) {
                    lf.merge_missing(x);
                }
                if !lf.help_file.is_empty() {
                    self.help_file_name = self.paths.resolve(&lf.help_file);
                }
                set_language(Some(lf));
                self.language = idx;
                true
            }
            Err(e) => {
                self.startup_errors.push(format!("{}: {e}", file.display()));
                false
            }
        }
    }

    /// Generator part of `LoadConfig()`.
    pub fn load_config(&mut self) {
        self.settings = GeneratorSettings::load(&self.ini);
        self.profiles = load_profiles(&self.ini);
        let opts = self.settings.passw_options.clone();
        self.passw_gen.set_word_separator(&opts.word_separator);
        self.passw_gen.set_word_char_separator(&opts.word_char_separator);
        if self.load_trigram_file(&opts.trigram_file_name) <= 0 {
            self.settings.passw_options.trigram_file_name.clear();
        }
        let cs = self.settings.char_set_list[0].clone();
        if self.load_char_set(&cs) == 0 {
            self.startup_errors.push(trl_format("Character set \"%1\" is invalid.", &[&cs]));
            self.settings.char_set_list[0] = CHARSETLIST_DEFAULTENTRIES[0].to_string();
            self.load_char_set(CHARSETLIST_DEFAULTENTRIES[0]);
        }
        let wl = self.settings.wlfn_list[0].clone();
        if wl != WLFNLIST_DEFAULT && self.load_word_list_file(&wl) <= 0 {
            self.startup_errors.push(trl_format("Error while loading word list file\n\"%1\".", &[&wl]));
            self.settings.wlfn_list[0] = WLFNLIST_DEFAULT.to_string();
            self.load_word_list_file(WLFNLIST_DEFAULT);
        } else if wl == WLFNLIST_DEFAULT {
            self.load_word_list_file(WLFNLIST_DEFAULT);
        }

        if self.test_common_passw {
            self.load_common_passwords();
        }
    }

    /// Load the list of common passwords (errors go to `startup_errors`).
    pub fn load_common_passwords(&mut self) {
        let p = self.paths.resolve("common_passwords.txt");
        match CommonPasswords::load(&p) {
            Ok(c) => self.common_passw = Some(c),
            Err(e) => self.startup_errors.push(trl_format(
                "Error while loading list of common passwords:\n%1.",
                &[&e.to_string()],
            )),
        }
    }

    /// Save the configuration (generator part; the GUI adds its own keys).
    pub fn save_config(&mut self) -> std::io::Result<()> {
        self.save_config_to(None)
    }

    /// `SaveConfig(sFileName)`: save to the configuration file, or to
    /// `path` (backup; also when the configuration is read-only).
    pub fn save_config_to(&mut self, path: Option<&Path>) -> std::io::Result<()> {
        if path.is_none() && (self.cmd.config_readonly || self.fake_ini) {
            return Ok(());
        }
        self.config.save(&mut self.ini);
        self.ini.write_string(CONFIG_ID, "LastVersion", PROGRAM_VERSION);
        let code = if self.config.language_code.is_empty() {
            self.languages[self.language].code.clone()
        } else {
            self.config.language_code.clone()
        };
        self.ini.write_string(CONFIG_ID, "Language", &code);
        self.settings.save(&mut self.ini);
        let sections: Vec<String> = self
            .ini
            .sections()
            .filter(|s| s.to_ascii_lowercase().starts_with("pwgenprofile"))
            .map(|s| s.to_string())
            .collect();
        for s in sections {
            self.ini.erase_section(&s);
        }
        save_profiles(&mut self.ini, &self.profiles);
        match path {
            Some(p) => self.ini.save(p),
            None => {
                std::fs::create_dir_all(&self.paths.data_dir)?;
                self.ini.save(&self.paths.ini_file)
            }
        }
    }

    /// `LoadCharSet()`: returns the size of the set (0 = invalid).
    pub fn load_char_set(&mut self, input: &str) -> usize {
        let o = &self.settings.passw_options;
        let res = self.passw_gen.setup_char_sets(
            input,
            &o.ambig_chars,
            &o.special_symbols,
            o.flags & PASSWOPTION_INCLUDE_CHAR_FROM_EACH_SUBSET != 0,
            o.flags & PASSWOPTION_EXCLUDEAMBIG != 0,
            o.flags & PASSWOPTION_INCLUDESUBSET != 0,
        );
        if res.is_empty() {
            self.char_set_error = true;
            self.char_set_info = trl("Invalid character set.");
            return 0;
        }
        self.char_set_error = false;
        let n = self.passw_gen.custom_char_set_w32().len();
        self.char_set_info = trl_format(
            "%1 characters / %2 bits per character",
            &[&n.to_string(), &format_float_1(self.passw_gen.custom_char_set_entropy())],
        );
        n
    }

    /// `LoadWordListFile()`: returns the number of words (≤ 0 = error).
    pub fn load_word_list_file(&mut self, input: &str) -> i32 {
        let trimmed = remove_comment_and_trim(input);
        let file = (!trimmed.is_empty() && trimmed != WLFNLIST_DEFAULT).then(|| self.paths.resolve(&trimmed));
        let o = &self.settings.passw_options;
        let r = self.passw_gen.load_word_list_file(
            file.as_deref(),
            o.min_word_len,
            o.max_word_len,
            o.flags & PASSWOPTION_LOWERCASEWORDS != 0,
            self.file_encoding,
        );
        match r {
            Ok(n) => {
                self.word_list_error = None;
                self.word_list_info = trl_format(
                    "%1 words / %2 bits per word",
                    &[&self.passw_gen.word_list_size().to_string(), &format_float_1(self.passw_gen.word_list_entropy())],
                );
                n
            }
            Err(WordListError::Io(_)) => {
                self.word_list_error = Some(trl("Could not open file."));
                -1
            }
            Err(WordListError::TooFewWords) => {
                self.word_list_error = Some(trl("Not enough valid words."));
                0
            }
        }
    }

    /// `LoadTrigramFile()`: 1 = ok, 0 = invalid, -1 = cannot open.
    pub fn load_trigram_file(&mut self, name: &str) -> i32 {
        let path = (!name.is_empty()).then(|| self.paths.resolve(name));
        match self.passw_gen.load_trigram_file(path.as_deref()) {
            Ok(true) => 1,
            Ok(false) => {
                self.startup_errors.push(trl("Invalid trigram file."));
                0
            }
            Err(_) => {
                self.startup_errors.push(trl("Could not open trigram file."));
                -1
            }
        }
    }

    /// `LoadProfile(name)`.
    pub fn load_profile_by_name(&mut self, name: &str) -> bool {
        match find_profile(&self.profiles, name) {
            Some(i) => {
                self.load_profile(i);
                true
            }
            None => false,
        }
    }

    /// `LoadProfile(index)`.
    pub fn load_profile(&mut self, idx: usize) {
        let p = self.profiles[idx].clone();
        self.settings.apply_profile(&p);
        if let Some(o) = &p.advanced_passw_options {
            self.passw_gen.set_word_separator(&o.word_separator);
            self.passw_gen.set_word_char_separator(&o.word_char_separator);
            if self.load_trigram_file(&o.trigram_file_name) <= 0 {
                self.settings.passw_options.trigram_file_name.clear();
            }
        }
        let cs = self.settings.char_set_list[0].clone();
        self.load_char_set(&cs);
        let wl = self.settings.wlfn_list[0].clone();
        self.load_word_list_file(&wl);
    }

    /// Build the generation request from the current settings (first part
    /// of `GeneratePassw()`); `num` for list destinations.
    pub fn gen_request(&self, dest: GenDest, num: u64) -> Result<GenRequest, String> {
        let s = &self.settings;
        let passphrase_length: Option<PassphraseLength> =
            if s.include_words && s.passphr_spec_len { parse_passphrase_length(&s.passphr_spec_len_string)? } else { None };
        let mut chars_len = if s.include_chars { s.chars_length } else { 0 };
        if chars_len != 0
            && s.passw_options.flags & PASSWOPTION_EACHCHARONLYONCE != 0
            && chars_len > self.passw_gen.custom_char_set_unique_size()
        {
            chars_len = self.passw_gen.custom_char_set_unique_size();
        }
        Ok(GenRequest {
            dest,
            num_passw: num,
            chars_len,
            num_words: if s.include_words { s.words_num } else { 0 },
            combine_words_chars: s.combine_words_chars,
            passphrase_length,
            format: if s.format_passw { s.format_list.first().cloned().unwrap_or_default() } else { String::new() },
            option_flags: s.passw_options.flags,
            include_header: s.include_header,
            file_newline: self.newline.clone(),
            test_common_passw: self.test_common_passw,
        })
    }

    /// Script file to run, if "Run script" is active.
    pub fn script_file(&self) -> Option<PathBuf> {
        if !self.settings.run_script {
            return None;
        }
        let name = remove_comment_and_trim(self.settings.script_list.first()?);
        if name.is_empty() { None } else { Some(self.paths.resolve(&name)) }
    }

    /// Text of "Command Line Arguments" (`m_sCmdLineInfo`).
    pub fn cmd_line_info() -> String {
        let mut s = trl("Usage:");
        s.push_str("\npwtool [--help] [--ini {file}] [--readonly] [--profile {profilename}] [--gen [number]] [--silent] [--opendb {filename}]\n");
        s.push_str(&format!("\n--help - {}", trl("Display this help message.")));
        s.push_str(&format!("\n--readonly - {}", trl("Do not write to disk unless explicitly requested.")));
        s.push_str(&format!("\n--profile - {}", trl("Load profile named 'profilename' on start-up.")));
        s.push_str(&format!(
            "\n--gen - {}",
            trl("Generate 'number' of passwords and display on the console, then close application.")
        ));
        s.push_str(&format!("\n--silent - {}", trl("Launch application in the background.")));
        s.push_str(&format!(
            "\n--opendb - {}",
            trl("Open database named 'filename' on start-up and show password manager window.")
        ));
        s
    }
}

/// Language code of the environment (`LANG`), e.g. `ru` for `ru_RU.UTF-8`.
pub fn system_language_code() -> String {
    let lang = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .find(|v| !v.is_empty() && v != "C" && v != "POSIX")
        .unwrap_or_default();
    lang.split(['_', '.', '@']).next().unwrap_or("").to_string()
}

/// `CompareVersionNumbers()`.
pub fn compare_versions(a: &str, b: &str) -> i32 {
    let p = |s: &str| -> Vec<i64> { s.split('.').map(|x| x.trim().parse().unwrap_or(0)).collect() };
    let (va, vb) = (p(a), p(b));
    for i in 0..va.len().max(vb.len()) {
        let x = va.get(i).copied().unwrap_or(0);
        let y = vb.get(i).copied().unwrap_or(0);
        if x != y {
            return if x < y { -1 } else { 1 };
        }
    }
    0
}

static RESTART: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Request a restart of the program after the main window has been closed
/// (`g_terminateAction = TerminateAction::RestartProgram`).
pub fn request_restart() {
    RESTART.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub fn restart_requested() -> bool {
    RESTART.load(std::sync::atomic::Ordering::Relaxed)
}
