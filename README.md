[English](README.md) | [Русский](README.ru_RU.md)

<p align="center">
  <img src="assets/about_logo.png" alt="Password Tool" width="160">
</p>

<h1 align="center">Password Tool</h1>
<p align="center">Cryptographically secure password generator and encrypted password manager for Linux.</p>

[![Release](https://img.shields.io/badge/release-0.1.0--beta-orange)](https://github.com/coinman-dev/PwTool/releases/tag/v0.1.0-beta)
[![Build](https://github.com/coinman-dev/PwTool/actions/workflows/release.yml/badge.svg)](https://github.com/coinman-dev/PwTool/actions/workflows/release.yml)
[![Linux](https://img.shields.io/badge/Linux-Ubuntu%2024.04%2B%20%2F%20Debian%2013%2B-blue)](#download)
[![Rust](https://img.shields.io/badge/Rust-1.90%2B-orange)](#building)
[![License](https://img.shields.io/badge/license-GPL--2.0--or--later-blue)](LICENSE)

**Password Tool** (PwTool) is a Linux port of [Password Tech](https://github.com/cthoeing/PasswordTech) 3.6.1
written in Rust and GTK4. It preserves the original windows, cryptographic algorithms,
and file formats: password databases (`.pwdb`), configuration files (`PwTool.ini` / `PwTech.ini`),
profiles, word lists, Lua scripts, and translations are compatible with the Windows version.

<p align="center">
  <img src="doc/images/en/main.png" alt="Password Tool: main window with password generator and 16-block strength meter">
</p>

> **0.1.0-beta is a prerelease.**
> Keep backups of important `.pwdb` databases before testing new releases, and report any
> issues on [GitHub Issues](https://github.com/coinman-dev/PwTool/issues). Password Tool
> is an independent Linux port and is not affiliated with the author of Password Tech.

## Download

| Edition | Download | Usage |
|---|---|---|
| Linux package | [pwtool_0.1.0.beta_amd64.deb](https://github.com/coinman-dev/PwTool/releases/download/v0.1.0-beta/pwtool_0.1.0.beta_amd64.deb) | `sudo apt install ./pwtool_0.1.0.beta_amd64.deb` installs Password Tool, icons, translations, script examples, and PDF manuals. |

Ubuntu 24.04 or newer, Debian 13 or newer, amd64 (GTK 4.10+). No advertising or telemetry.
Update checks contact only GitHub and can be disabled in **Options → Configuration**.
[All releases](https://github.com/coinman-dev/PwTool/releases) ·
[Changelog](CHANGELOG.md).

## Quick start

1. Install the `.deb` package with `sudo apt install ./pwtool_0.1.0.beta_amd64.deb`.
2. Open **Password Tool** from your application menu (**Utilities**) or run `pwtool`.
3. Choose a character set, passphrase word list, format string, or Lua script and click
   **Generate** — the 16-block strength meter shows estimated entropy in bits.
4. Click the database icon on the toolbar (or **File → Password Manager**) to open
   **PassCube** and store credentials in an encrypted `.pwdb` database.

The interface supports 13 languages, light, dark, and system themes, and custom GUI fonts.

## Features

- **Cryptographically secure RNG**: entropy pool backed by AES-256-CTR or ChaCha20, seeded
  from OS entropy, user input events, high-resolution timers, and optional entropy files;
  persistent random seed file on exit.
- **Flexible password generation**: character sets with custom frequency weights, ambiguous
  character exclusion, phonetic (trigram-based) passwords, word-list passphrases, pattern
  format strings, and embedded **Lua** scripting.
- **Password strength meter**: 16-block visual strength bar combining entropy calculation,
  common-password checking, and **zxcvbn** analysis.
- **PassCube password manager**: encrypted `.pwdb` databases compatible in both directions
  with Password Tech 3.6.1 (master password + optional key file, recovery key, tags,
  expiry warnings, password history, custom key-value fields, CSV import/export, backups).
- **Master-password generator (MP)**: deterministic password generation derived from a
  master password and site parameters (including Hashapass mode).
- **Text encryption**: encrypt and decrypt clipboard or arbitrary text directly from the
  toolbar.
- **Profiles & batch generation**: save reusable generator presets, generate lists of
  passwords to a window or file, or create random data and trigram files.
- **CLI mode**: generate passwords, passphrases, and formatted strings directly from the
  terminal without opening a window.
- **Bundled documentation**: `manual.pdf` and `scripting.pdf` are installed locally and
  open from **Help** or the toolbar.

## Desktop integration

- **System tray**: StatusNotifierItem (KDE Plasma and other SNI-capable desktops), with
  minimize-to-tray and quick password generation from the tray menu.
- **Autotype & global hot keys**: implemented via `xdg-desktop-portal` (`RemoteDesktop`
  and `GlobalShortcuts`); the desktop prompts for permission on first use.
- **Secure clipboard**: copied passwords are marked as secret (`x-kde-passwordManagerHint`)
  so clipboard managers such as Klipper do not keep them in history, with configurable
  auto-clear timeout.

## Settings and data

| What | Where |
|---|---|
| Program binary | `/usr/bin/pwtool` |
| Bundled data (translations, word lists, script examples, PDF manuals) | `/usr/share/pwtool/` |
| Settings, profiles, and random seed | `~/.config/pwtool/PwTool.ini`, `~/.config/pwtool/randseed.dat` |
| Portable mode | Place `PwTool.ini` next to the `pwtool` binary |
| Password databases (`.pwdb`) | Wherever you save them |

**Options → Restore Settings from Backup** also accepts backup files from Password Tech
(`PwTech.ini`).

## Command line

```bash
pwtool --help
pwtool --gen 5
pwtool --profile "My Profile" --gen 10
pwtool --ini /path/to/PwTool.ini --readonly
pwtool --opendb ~/vault.pwdb
pwtool --silent
```

## Building

Requires Rust 1.90+, a C/C++ compiler (`build-essential`), `pkg-config`, and `libgtk-4-dev`
(GTK 4.10 or newer). LibreOffice (`libreoffice-writer-nogui`) is optional and used to build
`manual.pdf` and `scripting.pdf` from `doc/manual/*.odt`.

```bash
cargo test --workspace --locked
cargo build --release --locked -p pwtool

packaging/install.sh /usr/local    # install to PREFIX
packaging/build-deb.sh             # build target/pwtool_<version>_<arch>.deb
```

Headless GUI automation:

```bash
tools/headless.sh env PWTOOL_AUTOMATE="click GenerateBtn;wait 400;print PasswBox;quit" ./target/debug/pwtool
```

## Automated releases

Pushing a version tag such as `v0.1.0-beta` runs the
[release workflow](.github/workflows/release.yml). The tag must match `version` in
`Cargo.toml`. GitHub runs the workspace tests on Ubuntu 24.04, converts the PDF manuals,
builds and checks `pwtool_<version>_amd64.deb`, computes `SHA256SUMS.txt`, and publishes
the release. Tags containing a hyphen suffix are automatically marked as prereleases.

## License

GNU General Public License v2 or later — see [LICENSE](LICENSE).

Based on **Password Tech** (Copyright © 2002–2026 Christian Thöing). Third-party components
retain their own licenses: miniLZO (GPL v2+), miniz (MIT), zxcvbn-c (MIT), and Lua (MIT,
via `mlua`).
