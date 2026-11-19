//! Core of the Linux port of **Password Tech 3.6.1** by Christian Thöing.
//!
//! Everything that does not depend on the user interface: cryptographic
//! primitives, random number generation, password generation, password
//! databases, clipboard text encryption, configuration and translations.
//! Module names follow the original source files in `upstream/src/`; file
//! formats and deterministic outputs are compatible with the Windows version
//! (see `.ai/FORMATS.md`).
//!
//! Licence: GPL-2.0-or-later, like the original.

pub mod config;
pub mod crypt_text;
pub mod crypto;
pub mod generate;
pub mod language;
pub mod language_file;
pub mod lzo;
pub mod mp_passw_gen;
pub mod passw;
pub mod random;
pub mod scripting;
pub mod secure_mem;
pub mod text_file;
pub mod unicode;
pub mod util;
