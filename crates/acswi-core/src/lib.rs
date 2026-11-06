//! acswi: keep several signed-in accounts per app on one computer and switch between them.
//!
//! The engine is generic: every app is described by an [`def::AppDef`] (TOML). See
//! `docs/DESIGN.md` for the model and `defs/` for examples.

pub mod credman;
pub mod def;
#[cfg(windows)]
pub mod dpapi;
pub mod engine;
pub mod envvar;
pub mod error;
pub mod expiry;
pub mod items;
pub mod mcp;
pub mod learn;
pub mod native;
pub mod paths;
pub mod process;
pub mod registry;
pub mod seal;
pub mod state;
pub mod vault;

pub use def::{AppDef, Strategy};
pub use engine::{Acswi, Actor, AddStep, AppView, ExpiringView, ProfileView, StatusView, SwitchOptions, SwitchReport};
pub use error::{Code, Error, Result};
pub use expiry::{Expiry, Level};
