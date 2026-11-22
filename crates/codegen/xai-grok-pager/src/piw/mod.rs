//! Live pi-workflows runs inside Grok Build.
//!
//! The `pi-workflows` workflow server runs one instance per machine and is
//! watched through [`client::PiwClient`]; [`gate`] answers the human gates a
//! run parks on; [`mirror`] writes each server run as an acpx-format run
//! folder so the existing flow screens read it unchanged; [`runtime`] owns
//! the pager's shared client and mirror.

pub mod client;
pub mod gate;
pub mod mirror;
pub mod runtime;
