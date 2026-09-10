// lib.rs

//! # StackQL Deploy - Library Crate
//!
//! The `stackql-deploy` binary (`src/main.rs`) is a thin CLI wrapper over the
//! modules exposed here. Exposing them as a library lets the integration
//! tests under `tests/` construct a `CommandRunner` against an in-process
//! mock StackQL server and drive the `build` / `teardown` flows directly,
//! without a real stackql binary, provider registry, or cloud credentials.
//!
//! This crate is not intended as a public API: module layout and function
//! signatures may change between releases without notice.

pub mod app;
pub mod commands;
pub mod core;
pub mod error;
pub mod globals;
pub mod resource;
pub mod template;
pub mod utils;
