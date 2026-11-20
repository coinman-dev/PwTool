[English](CHANGELOG.md) | [Русский](CHANGELOG.ru_RU.md)

# Changelog

## Unreleased

### Added

- **Windows version** (Windows 10/11, x64) as a portable zip with GTK 4 included,
  cross-compiled on Linux with llvm-mingw. Platform functions use the Win32 API like the
  original Password Tech: tray icon with menu and balloon notifications, global hot keys
  (`RegisterHotKey`), autotype (`SendInput`), clipboard excluded from clipboard history and
  cloud sync, autostart via the `Run` registry key, Always on Top, single instance, file
  locking of open databases, locked memory pages (`VirtualLock`), and dates in the regional
  format. Settings are stored in `%APPDATA%\PwTool`; portable mode works as on Linux.
- Release workflow builds `pwtool_<version>_windows_x64.zip` and runs the tests and the
  console mode on a Windows runner before publishing.

## 0.1.0-beta — 2026-11-20

First public beta prerelease of **Password Tool** (Linux port of Password Tech 3.6.1 in Rust and GTK4).

### Added

- **Complete password generator** with character sets, frequency weights, ambiguous character exclusion, phonetic (trigram) passwords, word-list passphrases, pattern format strings, and Lua scripting.
- **PassCube password manager** with two-way `.pwdb` compatibility with Password Tech 3.6.1, master password + key files, recovery key, tags, expiry warnings, password history, custom key-value fields, CSV import/export, and backups.
- **Additional tools**: Master Password (MP) generator, clipboard/text encryption, random data file generator, trigram file builder, profile editor, password list generator, and CLI mode.
- **Desktop integration**: StatusNotifierItem system tray, Wayland/X11 autotype and global hot keys via `xdg-desktop-portal`, secret clipboard hints for Klipper, and 13 interface languages.
- **16-block password strength bar** with distinct color steps from weak to strong.
- **Custom application logo & icons** across the main toolbar, About dialog, window titlebar, tray, and desktop menu.
- **Reset to default** button next to the GUI font selector in Configuration.
- **Debian/Ubuntu `.deb` packaging** with bundled `manual.pdf` and `scripting.pdf` user manuals.

### Fixed

- Reliable opening of local documentation (`manual.pdf`, `scripting.pdf`) and external web links via GIO default handlers with `FileLauncher`/`UriLauncher` and `xdg-open` fallbacks.
- Fixed occasional ignored clicks on the **Generate** button caused by brief window insensitivity on fast generations and capture-phase click handling.
- Core parity fixes against Password Tech 3.6.1 across cryptography, database serialization, scripting, formatting, and translations (90 unit tests passing).
