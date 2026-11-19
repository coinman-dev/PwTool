//! Password generation and password databases (`upstream/src/passw/`).

pub mod generator;
#[rustfmt::skip]
pub mod tables;

pub use generator::PasswordGenerator;
