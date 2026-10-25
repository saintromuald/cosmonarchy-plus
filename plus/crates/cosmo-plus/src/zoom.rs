//! Windows side of the player zoom-floor patch.
//!
//! Locates the loaded `gptp.qdp`, verifies the compiled `apply_display_role`
//! site with [`cosmo_plus_core::zoom::find_site`], and rewrites the player
//! branch's `mov eax, imm32` once at load. See the core module for the
//! signature and the observer-parity rationale.
//!
//! The patched bytes are in `gptp.qdp`'s read-only `.text`: the rollback
//! netcode snapshots only writable sections, and zoom is viewer-local, so the
//! write is invisible to the simulation and to rollback restore.

use std::mem::zeroed;

use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::Debug::FlushInstructionCache;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Module32First, Module32Next, MODULEENTRY32, TH32CS_SNAPMODULE,
    TH32CS_SNAPMODULE32,
};
use windows_sys::Win32::System::Memory::{VirtualProtect, PAGE_EXECUTE_READWRITE};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

use cosmo_plus_core::zoom as core;

/// Loaded module the patch targets.
const MODULE_NAME: &str = "gptp.qdp";
/// `cosmo_plus.conf` / environment override for the player floor, in percent.
const ENV_PLAYER_FLOOR: &str = "CM_PLUS_ZOOM_PLAYER_MIN";

/// Whether the player-floor patch was applied (for the boot report).
static PATCHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Resolved status for the boot report.
pub(crate) fn status() -> String {
    if PATCHED.load(std::sync::atomic::Ordering::SeqCst) {
        format!("armed (player floor {}%)", player_floor_percent())
    } else {
        "not patched (site missing or already modified)".to_string()
    }
}

/// Resolve the configured player floor: env override, clamped to the engine's
/// `set_min_zoom_percent` bounds, else the Plus default (125%).
fn player_floor_percent() -> u32 {
    match crate::config::env(ENV_PLAYER_FLOOR) {
        Some(v) => match v.trim().parse::<u32>() {
            Ok(p) if (core::MIN_PERCENT..=core::MAX_PERCENT).contains(&p) => p,
            _ => {
                crate::log::log(&format!(
                    "[cosmo-plus] zoom: {ENV_PLAYER_FLOOR}={v} out of range \
                     ({}-{}); using default {}",
                    core::MIN_PERCENT,
                    core::MAX_PERCENT,
                    core::PLAYER_FLOOR_PERCENT
                ));
                core::PLAYER_FLOOR_PERCENT
            }
        },
        None => core::PLAYER_FLOOR_PERCENT,
    }
}

/// Base and size of the loaded `gptp.qdp`, if present.
pub(crate) fn gptp_module() -> Option<(usize, usize)> {
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, 0);
        if snap == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut found = None;
        let mut me: MODULEENTRY32 = zeroed();
        me.dwSize = std::mem::size_of::<MODULEENTRY32>() as u32;
        if Module32First(snap, &mut me) != 0 {
            loop {
                let name_bytes = std::slice::from_raw_parts(
                    me.szModule.as_ptr() as *const u8,
                    me.szModule.len(),
                );
                let name = name_bytes
                    .iter()
                    .take_while(|b| **b != 0)
                    .map(|b| *b as char)
                    .collect::<String>();
                if name.eq_ignore_ascii_case(MODULE_NAME) {
                    found = Some((me.modBaseAddr as usize, me.modBaseSize as usize));
                    break;
                }
                if Module32Next(snap, &mut me) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
        found
    }
}

/// Rewrite the 5-byte `mov eax, imm32` at `addr` and read it back.
unsafe fn write_mov_imm(addr: usize, bytes: [u8; 5]) -> bool {
    let ptr = addr as *mut u8;
    let mut old: u32 = 0;
    if VirtualProtect(ptr as *const _, bytes.len(), PAGE_EXECUTE_READWRITE, &mut old) == 0 {
        return false;
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
    FlushInstructionCache(GetCurrentProcess(), ptr as *const _, bytes.len());
    let mut old2: u32 = 0;
    VirtualProtect(ptr as *const _, bytes.len(), old, &mut old2);
    std::slice::from_raw_parts(ptr, bytes.len()) == bytes
}

/// Install the Plus player zoom floor into `gptp.qdp` (one-time, idempotent).
pub(crate) fn install_player_zoom_floor() {
    let percent = player_floor_percent();
    if percent == core::STOCK_PLAYER_FLOOR_PERCENT {
        crate::log::log("[cosmo-plus] zoom: player floor is stock 200%; no patch needed");
        return;
    }
    let Some((base, size)) = gptp_module() else {
        crate::log::log("[cosmo-plus] zoom: gptp.qdp not loaded; player floor patch skipped");
        return;
    };
    let image = unsafe { std::slice::from_raw_parts(base as *const u8, size) };
    let Some(site) = core::find_site(image) else {
        crate::log::log(
            "[cosmo-plus] zoom: WorldZoom signature not found (pre-widescreen gptp.qdp?); \
             player floor patch skipped",
        );
        return;
    };
    let Some(current) = core::current_immediate(image, site) else {
        crate::log::log("[cosmo-plus] zoom: site out of bounds; player floor patch skipped");
        return;
    };
    let want = core::encode_mov_imm(percent);
    if current == want {
        crate::log::log(&format!(
            "[cosmo-plus] zoom: player floor already {percent}% @ 0x{:08x}",
            base + site.mov_imm
        ));
        return;
    }
    if current != core::encode_mov_imm(core::STOCK_PLAYER_FLOOR_PERCENT) {
        crate::log::log(&format!(
            "[cosmo-plus] zoom: unexpected bytes at site (0x{:08x}): {:02x?}; \
             patch not applied (gptp.qdp changed?)",
            base + site.mov_imm,
            current
        ));
        return;
    }
    let ok = unsafe { write_mov_imm(base + site.mov_imm, want) };
    if ok {
        crate::log::log(&format!(
            "[cosmo-plus] zoom: player floor {percent}% (witness floor {}%) @ 0x{:08x}; \
             call target 0x{:08x}",
            core::WITNESS_FLOOR_PERCENT,
            base + site.mov_imm,
            base + site.call_target
        ));
    } else {
        crate::log::log("[cosmo-plus] zoom: write did not verify; player floor patch skipped");
    }
}
