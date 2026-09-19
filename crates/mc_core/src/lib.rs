//! `mc_core` — pure domain logic for CTMLauncher.
//!
//! This crate intentionally contains **no** terminal/UI code. It exposes an
//! async, modular engine covering authentication, instance management,
//! modloader installation, Modrinth/modpack integration, JVM launching and
//! log analysis.

pub mod auth;
pub mod error;
pub mod img;
pub mod install;
pub mod instance;
pub mod launch;
pub mod logs;
pub mod modpack;
pub mod modrinth;
pub mod skins;
pub mod util;
pub mod version;

pub use error::{CoreError, Result};
