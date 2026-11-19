//! Program configuration edited in the Configuration dialog (`struct
//! Configuration` of `upstream/src/main/Configuration.h`, loaded in
//! `TMainForm::LoadConfig()` and `TPasswMngForm::LoadConfig()`), with the
//! same keys of `PwTech.ini`.

use std::collections::BTreeMap;

use pwtech_core::config::IniFile;
use pwtech_core::text_file::CharEncoding;

pub const AUTOCLEARCLIPTIME_MIN: i32 = 3;
pub const AUTOCLEARCLIPTIME_MAX: i32 = 32767;
pub const AUTOCLEARCLIPTIME_DEFAULT: i32 = 10;
pub const AUTOCLEARPASSWTIME_MIN: i32 = 3;
pub const AUTOCLEARPASSWTIME_MAX: i32 = 32767;
pub const AUTOCLEARPASSWTIME_DEFAULT: i32 = 30;
pub const HOTKEYS_MAX_NUM: usize = 10;
pub const DATABASE_MAX_NUM_BACKUPS: i32 = 999;

const MAIN_ID: &str = "Main";
const DB_ID: &str = "PasswManager";

/// `AutoCheckUpdates`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoCheckUpdates {
    Daily = 0,
    Weekly = 1,
    Monthly = 2,
    Disabled = 3,
}

impl AutoCheckUpdates {
    pub fn from_index(i: i32) -> Option<Self> {
        Some(match i {
            0 => Self::Daily,
            1 => Self::Weekly,
            2 => Self::Monthly,
            3 => Self::Disabled,
            _ => return None,
        })
    }
}

/// `HotKeyAction`.
pub mod hot_key_action {
    pub const NONE: i32 = 0;
    pub const SINGLE_PASSW: i32 = 1;
    pub const PASSW_LIST: i32 = 2;
    pub const PASSW_CLIPBOARD: i32 = 3;
    pub const PASSW_MSGBOX: i32 = 4;
    pub const PASSW_AUTOTYPE: i32 = 5;
    pub const SHOW_MPPG: i32 = 6;
    pub const SHOW_PASSW_MANAGER: i32 = 7;
    pub const SEARCH_DATABASE: i32 = 8;
    pub const SEARCH_DATABASE_AUTOTYPE: i32 = 9;
}

/// `HotKeyEntry`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HotKeyEntry {
    pub action: i32,
    pub show_main_win: bool,
}

/// `HotKeyList`: VCL `TShortCut` (key code | scShift 0x2000 | scCtrl 0x4000
/// | scAlt 0x8000) → entry, ordered like `std::map`.
pub type HotKeyList = BTreeMap<u16, HotKeyEntry>;

/// `AutoSaveDatabase`.
pub const ASD_ENTRY_MODIFICATION: i32 = 0;
pub const ASD_EVERY_CHANGE: i32 = 1;

/// `Configuration::Database`.
#[derive(Debug, Clone, PartialEq)]
pub struct DatabaseConfig {
    pub clear_clip_close_lock: bool,
    pub lock_minimize: bool,
    pub lock_idle: bool,
    pub lock_idle_time: i32,
    pub lock_auto_save: bool,
    pub create_backup: bool,
    /// Not stored in the configuration file (as in the original).
    pub number_backups: bool,
    pub open_window_on_startup: bool,
    pub open_last_db_on_startup: bool,
    pub warn_expired_entries: bool,
    pub warn_entries_expire_soon: bool,
    pub warn_expire_num_days: i32,
    pub max_num_backups: i32,
    pub auto_save: bool,
    pub auto_save_option: i32,
    pub default_autotype_sequence: String,
    pub keep_recent_files: bool,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        DatabaseConfig {
            clear_clip_close_lock: true,
            lock_minimize: true,
            lock_idle: true,
            lock_idle_time: 300,
            lock_auto_save: false,
            create_backup: true,
            number_backups: true,
            open_window_on_startup: false,
            open_last_db_on_startup: true,
            warn_expired_entries: false,
            warn_entries_expire_soon: false,
            warn_expire_num_days: 7,
            max_num_backups: 5,
            auto_save: false,
            auto_save_option: ASD_ENTRY_MODIFICATION,
            default_autotype_sequence: String::new(),
            keep_recent_files: true,
        }
    }
}

impl DatabaseConfig {
    /// Part of `TPasswMngForm::LoadConfig()`.
    pub fn load(ini: &IniFile) -> Self {
        let mut c = DatabaseConfig::default();
        c.clear_clip_close_lock = ini.read_bool(DB_ID, "ClearClipCloseLock", true);
        c.lock_minimize = ini.read_bool(DB_ID, "LockMinimize", true);
        c.lock_idle = ini.read_bool(DB_ID, "LockIdle", true);
        c.lock_idle_time = ini.read_int(DB_ID, "LockIdleTime", 300).clamp(10, 32767);
        c.lock_auto_save = ini.read_bool(DB_ID, "LockAutoSave", false);
        c.create_backup = ini.read_bool(DB_ID, "CreateBackup", true);
        c.max_num_backups = ini.read_int(DB_ID, "MaxNumBackups", 5).clamp(1, DATABASE_MAX_NUM_BACKUPS);
        c.open_window_on_startup = ini.read_bool(DB_ID, "OpenWindowOnStartup", false);
        c.open_last_db_on_startup = ini.read_bool(DB_ID, "OpenLastDbOnStartup", true);
        c.warn_expired_entries = ini.read_bool(DB_ID, "WarnExpiredEntries", false);
        c.warn_entries_expire_soon = ini.read_bool(DB_ID, "WarnEntriesExpireSoon", false);
        c.warn_expire_num_days = ini.read_int(DB_ID, "WarnExpireNumDays", 7);
        c.auto_save = ini.read_bool(DB_ID, "AutoSave", false);
        let o = ini.read_int(DB_ID, "AutoSaveOption", 0);
        if (ASD_ENTRY_MODIFICATION..=ASD_EVERY_CHANGE).contains(&o) {
            c.auto_save_option = o;
        }
        c.default_autotype_sequence =
            ini.read_string(DB_ID, "DefaultAutotypeSeq", "{username}{tab}{password}{enter}");
        c.keep_recent_files = ini.read_bool(DB_ID, "KeepRecentFiles", true);
        c
    }

    /// Part of `TPasswMngForm::SaveConfig()`.
    pub fn save(&self, ini: &mut IniFile) {
        ini.write_bool(DB_ID, "ClearClipCloseLock", self.clear_clip_close_lock);
        ini.write_bool(DB_ID, "LockMinimize", self.lock_minimize);
        ini.write_bool(DB_ID, "LockIdle", self.lock_idle);
        ini.write_integer(DB_ID, "LockIdleTime", self.lock_idle_time as i64);
        ini.write_bool(DB_ID, "LockAutoSave", self.lock_auto_save);
        ini.write_bool(DB_ID, "CreateBackup", self.create_backup);
        ini.write_integer(DB_ID, "MaxNumBackups", self.max_num_backups as i64);
        ini.write_bool(DB_ID, "OpenWindowOnStartup", self.open_window_on_startup);
        ini.write_bool(DB_ID, "OpenLastDbOnStartup", self.open_last_db_on_startup);
        ini.write_bool(DB_ID, "KeepRecentFiles", self.keep_recent_files);
        ini.write_bool(DB_ID, "WarnExpiredEntries", self.warn_expired_entries);
        ini.write_bool(DB_ID, "WarnEntriesExpireSoon", self.warn_entries_expire_soon);
        ini.write_integer(DB_ID, "WarnExpireNumDays", self.warn_expire_num_days as i64);
        ini.write_bool(DB_ID, "AutoSave", self.auto_save);
        ini.write_integer(DB_ID, "AutoSaveOption", self.auto_save_option as i64);
        ini.write_string(DB_ID, "DefaultAutotypeSeq", &self.default_autotype_sequence);
    }
}

/// `struct Configuration`.
#[derive(Debug, Clone, PartialEq)]
pub struct Configuration {
    pub ui_style_name: String,
    pub app_icon_name: String,
    pub gui_font_string: String,
    pub auto_clear_clip: bool,
    pub auto_clear_clip_time: i32,
    pub auto_clear_passw: bool,
    pub auto_clear_passw_time: i32,
    pub minimize_autotype: bool,
    pub autotype_delay: i32,
    pub test_common_passw: bool,
    pub use_advanced_passw_est: bool,
    pub show_sys_tray_icon_const: bool,
    pub minimize_to_sys_tray: bool,
    pub confirm_exit: bool,
    pub launch_system_startup: bool,
    pub load_profile_startup: bool,
    pub load_profile_name: String,
    pub random_pool_cipher: i32,
    pub auto_check_updates: AutoCheckUpdates,
    pub file_encoding: CharEncoding,
    /// `NewlineChar`: 0 = Windows (CRLF), 1 = Unix (LF).
    pub file_newline_char: i32,
    pub hot_keys: HotKeyList,
    /// Code of the language (`Configuration::Language.Code`).
    pub language_code: String,
    pub database: DatabaseConfig,
}

impl Configuration {
    /// The configuration part of `TMainForm::LoadConfig()` plus the
    /// database part of `TPasswMngForm::LoadConfig()`.
    pub fn load(ini: &IniFile) -> Self {
        let mut t = ini.read_int(MAIN_ID, "AutoClearClipboardTime", AUTOCLEARCLIPTIME_DEFAULT);
        if !(AUTOCLEARCLIPTIME_MIN..=AUTOCLEARCLIPTIME_MAX).contains(&t) {
            t = AUTOCLEARCLIPTIME_DEFAULT;
        }
        let mut tp = ini.read_int(MAIN_ID, "AutoClearPasswBoxTime", AUTOCLEARPASSWTIME_DEFAULT);
        if !(AUTOCLEARPASSWTIME_MIN..=AUTOCLEARPASSWTIME_MAX).contains(&tp) {
            tp = AUTOCLEARPASSWTIME_DEFAULT;
        }
        let cipher = ini.read_int(MAIN_ID, "RandomPoolCipher", 1);
        let enc = ini.read_int(MAIN_ID, "FileEncoding", CharEncoding::Utf8 as i32);
        // deliberate deviation: Unix newlines by default on Linux
        let mut nl = ini.read_int(MAIN_ID, "FileNewlineChar", 1);
        if !(0..=1).contains(&nl) {
            nl = 1;
        }
        let gui_font = ini.read_string(MAIN_ID, "GUIFont", "");
        Configuration {
            ui_style_name: ini.read_string(MAIN_ID, "GUIStyle", ""),
            app_icon_name: ini.read_string(MAIN_ID, "AppIcon", ""),
            gui_font_string: if font_string_parts(&gui_font) >= 2 { gui_font } else { String::new() },
            auto_clear_clip: ini.read_bool(MAIN_ID, "AutoClearClipboard", true),
            auto_clear_clip_time: t,
            auto_clear_passw: ini.read_bool(MAIN_ID, "AutoClearPasswBox", true),
            auto_clear_passw_time: tp,
            minimize_autotype: ini.read_bool(MAIN_ID, "MinimizeAutotype", true),
            autotype_delay: ini.read_int(MAIN_ID, "AutotypeDelay", 75),
            test_common_passw: ini.read_bool(MAIN_ID, "TestCommonPassw", true),
            use_advanced_passw_est: ini.read_bool(MAIN_ID, "UseAdvancedPasswEst", true),
            show_sys_tray_icon_const: ini.read_bool(MAIN_ID, "SystemTrayIconShowConst", true),
            minimize_to_sys_tray: ini.read_bool(MAIN_ID, "SystemTrayIconMinimize", true),
            confirm_exit: ini.read_bool(MAIN_ID, "ConfirmExit", false),
            launch_system_startup: ini.read_bool(MAIN_ID, "LaunchSystemStartup", false),
            load_profile_startup: ini.read_bool(MAIN_ID, "LoadProfileStartup", false),
            load_profile_name: ini.read_string(MAIN_ID, "LoadProfileStartupName", ""),
            random_pool_cipher: if (0..=2).contains(&cipher) { cipher } else { 1 },
            auto_check_updates: AutoCheckUpdates::from_index(ini.read_int(MAIN_ID, "AutoCheckUpdates", 1))
                .unwrap_or(AutoCheckUpdates::Weekly),
            file_encoding: CharEncoding::from_index(enc).unwrap_or(CharEncoding::Utf8),
            file_newline_char: nl,
            hot_keys: parse_hot_keys(&ini.read_string(MAIN_ID, "HotKeyList", "")),
            language_code: ini.read_string(MAIN_ID, "Language", ""),
            database: DatabaseConfig::load(ini),
        }
    }

    /// The configuration part of `TMainForm::SaveConfig()` (language and
    /// profiles are written by `AppCore`).
    pub fn save(&self, ini: &mut IniFile) {
        ini.write_string(MAIN_ID, "GUIStyle", &self.ui_style_name);
        ini.write_string(MAIN_ID, "GUIFont", &self.gui_font_string);
        ini.write_string(MAIN_ID, "AppIcon", &self.app_icon_name);
        ini.write_bool(MAIN_ID, "AutoClearClipboard", self.auto_clear_clip);
        ini.write_integer(MAIN_ID, "AutoClearClipboardTime", self.auto_clear_clip_time as i64);
        ini.write_bool(MAIN_ID, "AutoClearPasswBox", self.auto_clear_passw);
        ini.write_integer(MAIN_ID, "AutoClearPasswBoxTime", self.auto_clear_passw_time as i64);
        ini.write_bool(MAIN_ID, "SystemTrayIconShowConst", self.show_sys_tray_icon_const);
        ini.write_bool(MAIN_ID, "SystemTrayIconMinimize", self.minimize_to_sys_tray);
        ini.write_bool(MAIN_ID, "MinimizeAutotype", self.minimize_autotype);
        ini.write_integer(MAIN_ID, "AutotypeDelay", self.autotype_delay as i64);
        ini.write_bool(MAIN_ID, "ConfirmExit", self.confirm_exit);
        ini.write_bool(MAIN_ID, "LaunchSystemStartup", self.launch_system_startup);
        ini.write_bool(MAIN_ID, "LoadProfileStartup", self.load_profile_startup);
        ini.write_string(MAIN_ID, "LoadProfileStartupName", &self.load_profile_name);
        ini.write_integer(MAIN_ID, "RandomPoolCipher", self.random_pool_cipher as i64);
        ini.write_bool(MAIN_ID, "TestCommonPassw", self.test_common_passw);
        ini.write_bool(MAIN_ID, "UseAdvancedPasswEst", self.use_advanced_passw_est);
        ini.write_string(MAIN_ID, "HotKeyList", &hot_keys_to_string(&self.hot_keys));
        ini.write_integer(MAIN_ID, "FileEncoding", self.file_encoding as i64);
        ini.write_integer(MAIN_ID, "FileNewlineChar", self.file_newline_char as i64);
        ini.write_integer(MAIN_ID, "AutoCheckUpdates", self.auto_check_updates as i64);
        self.database.save(ini);
    }

    /// `g_sNewline`.
    pub fn newline(&self) -> &'static str {
        if self.file_newline_char == 0 { "\r\n" } else { "\n" }
    }
}

/// Number of valid leading parts of a font string (`StringToFont()` with a
/// null font): name, size, style, colour.
pub fn font_string_parts(s: &str) -> usize {
    if s.is_empty() {
        return 0;
    }
    let mut n = 0;
    for (i, part) in s.split(';').enumerate() {
        match i {
            0 => {}
            1 | 2 => match part.trim().parse::<i32>() {
                Ok(v) if !(i == 1 && v == 0) => {}
                _ => return n,
            },
            3 => {}
            _ => break,
        }
        n = i + 1;
    }
    n
}

/// Parse `HotKeyList` ("shortcut,action,showmainwin;...").
pub fn parse_hot_keys(s: &str) -> HotKeyList {
    let mut list = HotKeyList::new();
    if s.chars().count() < 5 {
        return list;
    }
    for entry in s.split(';') {
        let p: Vec<&str> = entry.split(',').collect();
        if p.len() < 3 {
            continue;
        }
        let key = p[0].trim().parse::<i64>().unwrap_or(0) as u16;
        if key == 0 {
            continue;
        }
        let action = p[1].trim().parse::<i32>().unwrap_or(-1);
        if !(hot_key_action::NONE..=hot_key_action::SEARCH_DATABASE_AUTOTYPE).contains(&action) {
            continue;
        }
        let show = p[2].trim().parse::<i32>().unwrap_or(0) != 0;
        if action != hot_key_action::NONE || show {
            list.entry(key).or_insert(HotKeyEntry { action, show_main_win: show });
        }
    }
    list
}

/// Format `HotKeyList` as in `TMainForm::SaveConfig()`.
pub fn hot_keys_to_string(list: &HotKeyList) -> String {
    let mut s = String::new();
    for (k, e) in list {
        s.push_str(&format!("{},{},{};", k, e.action, e.show_main_win as i32));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hot_keys_roundtrip() {
        let l = parse_hot_keys("16455,1,0;24647,0,1;bad;0,1,1;123,99,0");
        assert_eq!(l.len(), 2);
        assert_eq!(l[&16455], HotKeyEntry { action: 1, show_main_win: false });
        assert_eq!(hot_keys_to_string(&l), "16455,1,0;24647,0,1;");
        assert_eq!(parse_hot_keys(&hot_keys_to_string(&l)), l);
    }

    #[test]
    fn font_strings() {
        assert_eq!(font_string_parts(""), 0);
        assert_eq!(font_string_parts("Tahoma;8;0;clWindowText"), 4);
        assert_eq!(font_string_parts("Tahoma;0"), 1);
        assert_eq!(font_string_parts("Tahoma;9"), 2);
    }
}
