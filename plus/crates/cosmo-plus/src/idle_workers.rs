//! Idle-worker hotkey: one key press selects every idle worker of the local player.
//!
//! An idle worker is a completed worker unit whose `current_harvest_target` is null —
//! it is not gathering, not carrying cargo back, and not walking to a node. The
//! selection goes through the engine's own click-selection routine
//! (`select_units_if_alt_not_held`, `BW/bwhelper.cpp`, 0x0046FA00), which replaces the
//! current selection, plays the selection sound and **sends the synced select command**
//! to peers — so the selection is network-consistent and rollback-safe (it travels in
//! the normal turn stream, never as a raw memory write).
//!
//! The hotkey is polled from the draw pass (view-local input, like the draft picks),
//! so it never fires during a rollback re-simulation.
//!
//! Configure with `CM_PLUS_IDLE_WORKER_KEY` in `cosmo_plus.conf` (or the environment):
//! named keys (`F1`–`F12`, `A`–`Z`, `0`–`9`, `SPACE`, `TAB`, `HOME`, `END`) or a raw
//! virtual-key code as hex (`0x7A`). The default is **F9** — unbound in stock Brood War
//! and in the mod's hotkey layer (F1–F8 are camera screens, F10 the menu, F11 the HUD
//! toggle).

use std::sync::atomic::{AtomicU16, Ordering};

use cosmo_plus_core::hotkeys::{parse_key, DEFAULT_IDLE_WORKER_KEY};
use cosmo_plus_core::saturation::worker_tier;

/// How many units one selection can hold (`SelectionCap`).
const SELECTION_CAP: usize = 12;

/// The resolved hotkey VK (0 = disabled), set once at install.
static KEY_VK: AtomicU16 = AtomicU16::new(0);
/// Rising-edge state: bit 14 = previous frame's down state.
static KEY_EDGE: AtomicU16 = AtomicU16::new(0);

/// Resolve and log the hotkey (one-time, at install).
pub(crate) fn install() {
    let name = crate::config::env("CM_PLUS_IDLE_WORKER_KEY")
        .unwrap_or_else(|| DEFAULT_IDLE_WORKER_KEY.to_string());
    match parse_key(&name) {
        Some(vk) => {
            KEY_VK.store(vk, Ordering::SeqCst);
            crate::log::log(&format!(
                "[cosmo-plus] idle workers: hotkey {name} (VK 0x{vk:02X}) selects idle workers"
            ));
        }
        None => crate::log::log(&format!(
            "[cosmo-plus] idle workers: unknown CM_PLUS_IDLE_WORKER_KEY={name:?}; hotkey disabled \
             (use e.g. F9, F12, SPACE, or a hex VK like 0x7A)"
        )),
    }
}

/// The local player's id, or `None` while it is not yet resolvable.
unsafe fn local_player() -> Option<u32> {
    let api = crate::plus::api();
    let f = (api.local_player_id)()?;
    Some(f())
}

/// Called from the draw pass every frame. On the hotkey's rising edge, select every
/// idle worker through the engine's own selection routine (synced command).
pub(crate) unsafe fn poll() {
    let vk = KEY_VK.load(Ordering::SeqCst);
    if vk == 0 {
        return;
    }
    if crate::frame::frame_count() == 0 {
        return; // not in a game
    }
    let down = (windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(vk as i32)
        as u16
        & 0x8000)
        != 0;
    let prev = KEY_EDGE.load(Ordering::SeqCst);
    let was = prev & (1 << 14) != 0;
    let mut next = prev & !(1 << 14);
    if down {
        next |= 1 << 14;
    }
    KEY_EDGE.store(next, Ordering::SeqCst);
    if !(down && !was) {
        return;
    }

    let Some(local) = local_player() else {
        return;
    };
    let mut list: [*mut bw_dat::structs::Unit; SELECTION_CAP] =
        [core::ptr::null_mut(); SELECTION_CAP];
    let mut n = 0usize;
    crate::frame::for_each_unit(|u| {
        if n >= SELECTION_CAP {
            return;
        }
        if (*u).player as u32 != local {
            return;
        }
        if worker_tier((*u).unit_id).is_none() {
            return;
        }
        if (*u).remaining_build_time != 0 {
            return;
        }
        if !(*u).unit_specific.worker.current_harvest_target.is_null() {
            return;
        }
        list[n] = u;
        n += 1;
    });
    if n == 0 {
        crate::log::log("[cosmo-plus] idle workers: none found");
        return;
    }
    crate::log::log(&format!("[cosmo-plus] idle workers: selecting {n}"));
    crate::select::select_units_if_alt_not_held(list.as_ptr(), n as u32, 1, 1);
}
