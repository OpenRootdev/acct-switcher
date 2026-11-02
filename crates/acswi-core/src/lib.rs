//! acswi: keep several signed-in accounts per app on one Windows PC and switch between them.
//!
//! The engine is generic: every app is described by an [`def::AppDef`] (TOML). See
//! `docs/DESIGN.md` for the model and `defs/` for examples.

#![cfg(windows)]

pub mod credman;
pub mod def;
pub mod dpapi;
pub mod engine;
pub mod envvar;
pub mod error;
pub mod items;
pub mod learn;
pub mod native;
pub mod paths;
pub mod process;
pub mod state;
pub mod vault;

pub use def::{AppDef, Strategy};
pub use engine::{Acswi, Actor, AddStep, AppView, ProfileView, StatusView, SwitchOptions, SwitchReport};
pub use error::{Code, Error, Result};
