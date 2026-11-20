//! Program constants (`upstream/src/main/ProgramDef.h`), adapted for
//! Password Tool, the port of Password Tech for Linux and Windows.

/// Name of the program.
pub const PROGRAM_NAME: &str = "Password Tool";
/// Short name (binary, configuration directory).
pub const PROGRAM_SHORT_NAME: &str = "PwTool";
/// Name of the original program this one is based on.
pub const ORIGINAL_NAME: &str = "Password Tech";
/// Version of the original this port corresponds to. Used for the
/// compatibility check of translation files and in `LastVersion`.
pub const PROGRAM_VERSION: &str = "3.6.1";
/// Minimum version of translation files.
pub const PROGRAM_LANGVER_MIN: &str = "3.6.0";
/// Version of Password Tool.
pub const PORT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const PROGRAM_AUTHOR: &str = "Christian Thöing";
pub const PROGRAM_AUTHOR_EMAIL: &str = "c.thoeing@web.de";
pub const PROGRAM_COPYRIGHT: &str = "Copyright © 2002-2026";
/// Repository of Password Tool (used for "Visit Website", "Get
/// Translations" and the update check).
pub const PORT_REPOSITORY: &str = "https://github.com/coinman-dev/PwTool";
/// Donation page of the original author.
pub const PROGRAM_URL_DONATE: &str = "https://www.paypal.com/donate/?hosted_button_id=XLLCYFJZLS5V8";
pub const PROGRAM_URL_ORIGINAL: &str = "https://pwgen-win.sourceforge.io";
pub const PROGRAM_HELPFILE: &str = "manual.pdf";
pub const PROGRAM_LICENSEFILE: &str = "license.txt";
pub const PROGRAM_INIFILE: &str = "PwTool.ini";
pub const PROGRAM_RANDSEEDFILE: &str = "randseed.dat";
pub const LANGUAGE_DEFAULT_NAME: &str = "English";
pub const LANGUAGE_DEFAULT_CODE: &str = "en";

/// Version string shown to the user.
pub fn version_string() -> String {
    format!("{PORT_VERSION} ({PROGRAM_VERSION})")
}
