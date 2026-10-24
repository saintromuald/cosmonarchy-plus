//! Idle-worker hotkey: one key press selects a single idle worker of the local
//! player, and the HUD shows the live idle-worker count above the minimap
//! (drawn by the overlay's `draw_idle_counter`, SC2-style).
//!
//! An idle worker is a completed worker unit that is not gathering, not carrying
//! cargo back, **not moving**, and either has no harvest link or sits on a
//! parked order (Stop / Guard / PlayerGuard — the game parks jobless workers on
//! those, and some workers, e.g. Zerg Agrileth, keep a stale harvest target
//! while parked; the parked orders catch those too).
//!
//! The selection goes through the engine's own click-selection routine
//! (`select_units_if_alt_not_held`, `BW/bwhelper.cpp`, 0x0046FA00), which replaces the
//! current selection, plays the selection sound and **sends the synced select command**
//! to peers — so the selection is network-consistent and rollback-safe (it travels in
//! the normal turn stream, never as a raw memory write).
//!
//! One press selects **one** idle worker (the first one found); each further press
//! selects the next, SC2-F1 style.
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

/// Orders a jobless worker can hold while parked: the game parks workers on
/// `Stop` (0x01), `Guard` (0x02) and `PlayerGuard` (0x03).
const ORDER_STOP: u8 = 0x01;
const ORDER_GUARD: u8 = 0x02;
const ORDER_PLAYER_GUARD: u8 = 0x03;

/// A worker is idle when it is completed, carries nothing, is not mid-harvest,
/// is standing still, and either has no harvest link or sits on a parked order.
fn worker_is_idle(u: &bw_dat::structs::Unit) -> bool {
    if u.remaining_build_time != 0 {
        return false;
    }
    let w = unsafe { &u.unit_specific.worker };
    if w.carried_resource_count != 0 || w.is_harvesting != 0 {
        return false;
    }
    // Moving workers are on their way somewhere; only parked workers count.
    let f = &u.flingy;
    if f.current_speed != 0 || f.current_speed_x != 0 || f.current_speed_y != 0 {
        return false;
    }
    if w.current_harvest_target.is_null() {
        return true;
    }
    matches!(u.order, ORDER_STOP | ORDER_GUARD | ORDER_PLAYER_GUARD)
}

/// The local player's idle-worker count (the HUD counter value).
pub(crate) unsafe fn count_idle(local: u32) -> u32 {
    let mut n = 0u32;
    crate::frame::for_each_unit(|u| {
        if (*u).player as u32 != local {
            return;
        }
        if worker_tier((*u).unit_id).is_none() {
            return;
        }
        if worker_is_idle(&*u) {
            n += 1;
        }
    });
    n
}

/// The command-card icon id of the first idle worker, or None when none are
/// idle. Used for the SC2-style HUD badge.
pub(crate) unsafe fn first_idle_icon(local: u32) -> Option<u32> {
    let mut icon: Option<u32> = None;
    crate::frame::for_each_unit(|u| {
        if icon.is_some() {
            return;
        }
        if (*u).player as u32 != local {
            return;
        }
        if worker_tier((*u).unit_id).is_none() {
            return;
        }
        if !worker_is_idle(&*u) {
            return;
        }
        icon = Some(bw_dat::UnitId((*u).unit_id).icon());
    });
    icon
}

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
                "[cosmo-plus] idle workers: hotkey {name} (VK 0x{vk:02X}) selects one idle worker"
            ));
        }
        None => crate::log::log(&format!(
            "[cosmo-plus] idle workers: unknown CM_PLUS_IDLE_WORKER_KEY={name:?}; hotkey disabled \
             (use e.g. F9, F12, SPACE, or a hex VK like 0x7A)"
        )),
    }
}

/// The local player's id, or `None` while it is not yet resolvable.
pub(crate) unsafe fn local_player() -> Option<u32> {
    let api = crate::plus::api();
    let f = (api.local_player_id)()?;
    Some(f())
}

/// Called from the draw pass every frame. On the hotkey's rising edge, select
/// **one** idle worker through the engine's own selection routine (synced
/// command). Each further press selects the next idle worker.
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
    let mut first: *mut bw_dat::structs::Unit = core::ptr::null_mut();
    crate::frame::for_each_unit(|u| {
        if !first.is_null() {
            return;
        }
        if (*u).player as u32 != local {
            return;
        }
        if worker_tier((*u).unit_id).is_none() {
            return;
        }
        if !worker_is_idle(&*u) {
            return;
        }
        first = u;
    });
    if first.is_null() {
        crate::log::log("[cosmo-plus] idle workers: none found");
        return;
    }
    crate::log::log("[cosmo-plus] idle workers: selecting 1");
    let list: [*mut bw_dat::structs::Unit; SELECTION_CAP] = [first; SELECTION_CAP];
    crate::select::select_units_if_alt_not_held(list.as_ptr(), 1, 1, 1);
}
