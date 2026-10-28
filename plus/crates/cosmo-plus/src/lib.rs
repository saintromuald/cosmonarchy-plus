//! Cosmonarchy Plus: gameplay/QoL layer for Cosmonarchy BW.
//!
//! Deliberately independent of the rollback netcode crate: Plus must be shippable
//! to players who only want the gameplay changes, and the netcode must be
//! shareable with upstream without dragging gameplay with it.

#![allow(dead_code, non_snake_case)]

mod abilities;
mod antimeta;
mod cam_pan;
mod cam_labels;
mod casting;
mod config;
mod crash;
mod data;
mod draft;
mod frame;
mod hud;
mod icon;
mod idle_workers;
mod larva_pick;
mod log;
mod mpqdraft;
mod nydus;
mod overlay;
mod plugin;
mod queue;
mod reload;
mod repl;
mod plus;
mod select;
mod selftest;
mod state;
mod tab_cycle;
mod ultraload;
mod ultraload_icon;
mod zoom;

/// Bumped when the Plus <-> netcode/handshake surface changes.
#[no_mangle]
pub extern "system" fn cosmo_plus_abi() -> u32 {
    1
}

/// Build identity for match gating and replay tagging. Set by the build script
/// (`COSMO_PLUS_BUILD`); falls back to the crate version.
#[no_mangle]
pub extern "system" fn cosmo_plus_build_id() -> u32 {
    option_env!("COSMO_PLUS_BUILD_ID")
        .and_then(|s| u32::from_str_radix(s, 16).ok())
        .unwrap_or(1)
}
