//! Ultraload command-card icon (placeholder art).
//!
//! The ability is a passive, so there is nothing to click. To make it visible we
//! append one *disabled* button to each tier-2 worker's button set:
//!
//!   * requirement function — our own `fastcall` stub returning `Disabled`
//!     (BTNSCOND ABI: `ecx = requirement_var`, `edx = player`, `Unit*` on the
//!     stack), so the icon draws greyed and never fires;
//!   * action function — the engine's `BTNSACT_DoNothing` (0x00423180), so even a
//!     forced click cannot call a null pointer;
//!   * icon — `IconId::UseStimpack` (237) as placeholder art, per the request to
//!     pick any existing icon until the real one is drawn.
//!
//! The original button array is copied into static storage, one button appended,
//! and the set's count/pointer are updated in place. Only the tier-2 worker ids
//! are touched, so no other unit's card changes. If a card is already full (all
//! nine grid positions used) the icon is skipped and logged rather than
//! displacing an existing button.

use std::sync::atomic::{AtomicBool, Ordering};

use bw_dat::structs::Unit;

use cosmo_plus_core::saturation;

/// `g_buttonset_table` (ButtonSet*), fixed address in the 1.16.1 exe.
const BUTTONSET_TABLE_ADDR: usize = 0x0051_87E8;
/// `BTNSACT_DoNothing` — a real engine function, safe to invoke.
const ACT_DO_NOTHING: usize = 0x0042_3180;
/// Placeholder icon: `IconId::UseStimpack`.
const PLACEHOLDER_ICON: u16 = 237;
/// Command-card grid is 3x3.
const GRID_SLOTS: usize = 9;
/// Buttons per set we are willing to copy (shipped worker sets are far smaller).
const BUTTON_CAP: usize = 16;
const BUTTON_SIZE: usize = 20;
const MAX_SETS: usize = 8;

#[repr(C)]
#[derive(Clone, Copy)]
struct Button {
    position: u16,
    icon_id: u16,
    requirement_func: usize,
    action_func: usize,
    requirement_var: u16,
    action_var: u16,
    requirement_string_id: u16,
    action_string_id: u16,
}

#[repr(C)]
struct ButtonSet {
    buttons_in_set: u32,
    first_button: *mut Button,
    connected_unit: u32,
}

/// Static copy of each patched set's buttons (appended button included).
static mut BUFFERS: [[u8; BUTTON_CAP * BUTTON_SIZE]; MAX_SETS] = [[0u8; BUTTON_CAP * BUTTON_SIZE]; MAX_SETS];
static PATCHED: [AtomicBool; MAX_SETS] = [const { AtomicBool::new(false) }; MAX_SETS];
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// BTNSCOND stub: always `Disabled` (-1). Fastcall keeps the engine's ABI.
#[allow(improper_ctypes_definitions)]
unsafe extern "fastcall" fn ultraload_requirement(
    _requirement_var: u32,
    _player: u32,
    _unit: *mut Unit,
) -> i32 {
    -1
}

/// Patch every tier-2 worker's button set once. Called at the first game tick,
/// when the button table is guaranteed to be loaded.
pub(crate) fn install_once() {
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    if crate::config::env_flag("CM_PLUS_NO_ULTRALOAD_ICON") {
        crate::log::log("[cosmo-plus] ultraload icon: disabled by CM_PLUS_NO_ULTRALOAD_ICON");
        return;
    }

    let mut index = 0usize;
    let mut patched = 0u32;
    for &(id, tier, race) in saturation::WORKER_TIERS {
        if tier != 2 {
            continue;
        }
        if index >= MAX_SETS {
            break;
        }
        unsafe {
            if patch_set(index, id) {
                patched += 1;
                crate::log::log(&format!(
                    "[cosmo-plus] ultraload icon: appended passive button to {race} worker \
                     id {id} (icon {PLACEHOLDER_ICON})"
                ));
            }
        }
        index += 1;
    }
    crate::log::log(&format!(
        "[cosmo-plus] ultraload icon: {patched} worker set(s) patched"
    ));
}

unsafe fn patch_set(index: usize, unit_id: u16) -> bool {
    let table = unsafe { *(BUTTONSET_TABLE_ADDR as *const *mut ButtonSet) };
    if table.is_null() {
        crate::log::log("[cosmo-plus] ultraload icon: button table is null; skipped");
        return false;
    }
    if PATCHED[index].swap(true, Ordering::SeqCst) {
        return false;
    }

    // The button table is indexed by unit id; refuse anything outside the table.
    let entries = bw_dat::UnitId::entry_amount();
    if entries != u32::MAX && unit_id as u32 >= entries {
        return false;
    }

    let set = unsafe { &mut *table.add(unit_id as usize) };
    let count = set.buttons_in_set as usize;
    if count == 0 || set.first_button.is_null() || count >= BUTTON_CAP - 1 {
        crate::log::log(&format!(
            "[cosmo-plus] ultraload icon: unit {unit_id} set unusable (count {count}); skipped"
        ));
        return false;
    }

    // Find the first free grid slot; never displace an existing button.
    if count >= GRID_SLOTS {
        crate::log::log(&format!(
            "[cosmo-plus] ultraload icon: unit {unit_id} card is full ({count} buttons); skipped"
        ));
        return false;
    }
    let mut used = [false; GRID_SLOTS];
    for i in 0..count {
        let pos = unsafe { (*set.first_button.add(i)).position } as usize;
        if pos < GRID_SLOTS {
            used[pos] = true;
        }
    }
    let Some(position) = used.iter().position(|taken| !taken) else {
        crate::log::log(&format!(
            "[cosmo-plus] ultraload icon: unit {unit_id} card is full ({count} buttons); skipped"
        ));
        return false;
    };

    let buffer = core::ptr::addr_of_mut!(BUFFERS[index]) as *mut Button;
    unsafe {
        core::ptr::copy_nonoverlapping(set.first_button, buffer, count);
        *buffer.add(count) = Button {
            position: position as u16,
            icon_id: PLACEHOLDER_ICON,
            requirement_func: ultraload_requirement as *const () as usize,
            action_func: ACT_DO_NOTHING,
            requirement_var: 0,
            action_var: 0,
            requirement_string_id: 0,
            action_string_id: 0,
        };
        set.first_button = buffer;
        set.buttons_in_set = (count + 1) as u32;
    }
    true
}
