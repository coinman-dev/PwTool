[English](CHANGELOG.md) | [Русский](CHANGELOG.ru_RU.md)

# Changelog

## Unreleased

### Added

- **Windows version** (Windows 10/11, x64) as one `.exe` that needs nothing installed and
  imports only the DLLs of Windows itself, cross-compiled on Linux with llvm-mingw.
  Platform functions use the Win32 API like the original Password Tech: tray icon with
  menu and balloon notifications, global hot keys (`RegisterHotKey`), autotype
  (`SendInput`), clipboard excluded from clipboard history and cloud sync, autostart via
  the `Run` registry key, Always on Top, single instance, file locking of open databases,
  locked memory pages (`VirtualLock`), and dates in the regional format. Settings are
  stored in `%APPDATA%\PwTool`; portable mode works as on Linux.
- KDE Plasma on Wayland: Always on Top, Reset All Window Positions and the database search
  by the title of the active window work through KWin scripts (over D-Bus, without GTK).
- The open database and the settings are saved when the user session ends (Windows shut
  down or log off; on Linux through the session monitor of `xdg-desktop-portal`).
- Release workflow builds `pwtool_<version>_windows_x64.exe` and runs the tests and the
  console mode on a Windows runner before publishing.

### Changed

- **The interface no longer uses GTK.** The windows of the original are drawn with egui
  from the original form descriptions, with the same layout. The Linux package depends only
  on the C/C++ runtime, OpenGL (EGL) and xkbcommon instead of GTK 4.
- Translations, the list of common passwords, script examples and the PDF manuals are
  built into the program (compressed); own translations can still be placed in the
  settings folder.
- The colour variants of the program icon (Settings > General > Application icon) are
  made from the Password Tool icon (tools/make_icon_variants.py) instead of the cube of
  Password Tech. The About window shows the copyright of the original author without his
  e-mail address, followed by "Copyright © 2026 CoinMan.dev".
- All windows share one style: the windows built in code (font dialog, message boxes) use
  the sizes of the controls of the original forms; combo boxes are as tall as the edits and
  buttons, spin buttons have larger arrows. The font dialog is translated.
- Long translated captions take the free space next to them or use a slightly smaller
  font instead of being cut off; a caption that still does not fit shows its full text as
  a tooltip.
- Screen readers get the text of labels, the names of icon buttons and the captions of
  input fields in all windows; the PassCube entry list is a list with its entries and the
  selected one (eframe is patched to expose windows other than the main one).
- PassCube suggests the user names of the other entries while a user name is typed (a
  list below the field; arrow keys and Enter or a click choose one), as in Password Tech.
- Messages about things that are normal on many systems (no tray host, no session monitor
  of the portal) are printed only when `PWTOOL_DEBUG` is set.
- Windows look closer to Password Tech: the arrow of editable combo boxes lies inside the
  field (it was cut off by the buttons next to it), "Generate ▾", "Advanced ▾" and "Select
  font ▾" are split buttons whose arrow opens their menu, the show/hide password button
  shows "•••", the separate "Reset to default" font button is gone (its menu has "Restore
  Default"), and a window with tabs is only as wide as its tabs need. Labels that ran under
  the field next to them or out of the window (wider fonts than Tahoma, mainly on Linux)
  use a smaller font.
- Buttons and fields with a context menu (such as "Generate", "Advanced" and the password
  box) did not react to a left click in some cases; the right-click menu no longer covers
  them.
- Hungarian translation: the "At least" check boxes of the Character Set Builder showed a
  wrong text taken over from the original translation.

### Known limitations

- Text cannot be dragged with the mouse from the program into other programs (copy and
  paste works).

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
