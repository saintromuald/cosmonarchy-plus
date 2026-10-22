//! Cosmonarchy Plus core: pure, dependency-free logic shared by the plugin and its tests.
//!
//! This crate has no Windows-only dependencies, so `cargo test` runs on the host. It
//! holds the pieces that must be provably deterministic and unit-testable:
//!   * [`saturation`] — the harvest-cycle model and worker/tier tables,
//!   * [`yaml`] — the tiny `plus.yml` parser,
//!   * [`config`] — the `plus.yml` schema (`PlusConfig`) and its parser,
//!   * [`zoom`] — the `WorldZoom` player-floor patch signature and encoding.

#![allow(dead_code)]

pub mod antimeta;
pub mod config;
pub mod saturation;
pub mod yaml;
pub mod zoom;
