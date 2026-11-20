//! Password generation and password databases (`upstream/src/passw/`).

pub mod database;
pub mod generator;
#[rustfmt::skip]
pub mod tables;

pub use database::PasswDatabase;
pub use generator::PasswordGenerator;
