//! One-shot abilities dump (`CM_PLUS_DUMP_ABILITIES=1`): logs every unit's
//! buttonset with each button's `action_var` (the order id for order-type
//! buttons) and `action_func` address, plus the unit's name from units.txt.
//!
//! In Brood War the buttonset id equals the unit id (`change_unit_buttonset`
//! defaults the buttonset to the unit id), so walking `g_buttonset_table` by
//! unit id gives the complete unit -> buttons -> orders map with no live
//! units needed. This feeds the smart-cast classification (targeted spells
//! smart-cast, cast-all abilities like stim/siege broadcast) and the
//! per-unit cast-mode table in plus.yml.
//!
//! Runs once, on the first in-game draw (the extender resolves the table
//! pointer during engine init; reading it earlier — at plugin install — sees
//! the unresolved value).

use std::sync::atomic::{AtomicBool, Ordering};

use once_cell::sync::Lazy;

/// `g_buttonset_table` (ButtonSet** at 0x005187E8; the extender resolves it).
const G_BUTTONSET_TABLE: usize = 0x0051_87E8;

/// `mpq/arr/units.txt` names (id = line index), loaded once for the dump.
static UNIT_NAMES: Lazy<Vec<String>> = Lazy::new(crate::data::load_unit_names);

static DONE: AtomicBool = AtomicBool::new(false);

/// Minimal `Button` / `ButtonSet` layouts (BW/structures.h + the 1.16.1 dat).
#[repr(C)]
struct Button {
    position: u16,
    icon_id: u16,
    requirement_func: u32,
    action_func: u32,
    requirement_var: u16,
    action_var: u16,
    requirement_string_id: u16,
    action_string_id: u16,
}

#[repr(C)]
struct ButtonSet {
    buttons_in_set: u32,
    first_button: *const Button,
    connected_unit: u32,
}

/// How many unit ids to scan (the roster is under 600; the table is larger).
const SCAN_UNITS: usize = 800;
/// Sanity cap on a buttonset's button count.
const MAX_BUTTONS: u32 = 32;

/// Call from the draw pass: dump once when `CM_PLUS_DUMP_ABILITIES=1`.
pub(crate) unsafe fn dump_once() {
    if !crate::config::env_flag("CM_PLUS_DUMP_ABILITIES") || DONE.swap(true, Ordering::SeqCst) {
        return;
    }
    let table = std::ptr::read(G_BUTTONSET_TABLE as *const *const ButtonSet);
    if table.is_null() || !crate::overlay::readable_ptr(table as *const u8, 4) {
        crate::log::log("[cosmo-plus] abilities dump: buttonset table not resolved");
        return;
    }

    crate::log::log("[cosmo-plus] abilities dump: start");
    let mut logged = 0u32;
    for unit_id in 0..SCAN_UNITS {
        let bs = table.add(unit_id);
        if !crate::overlay::readable_ptr(bs as *const u8, core::mem::size_of::<ButtonSet>()) {
            break;
        }
        let count = (*bs).buttons_in_set.min(MAX_BUTTONS);
        let first = (*bs).first_button;
        if count == 0 || first.is_null() {
            continue;
        }
        let name = UNIT_NAMES
            .get(unit_id)
            .map(|s| s.trim())
            .unwrap_or("")
            .to_string();
        for i in 0..count as usize {
            let b = first.add(i);
            if !crate::overlay::readable_ptr(b as *const u8, core::mem::size_of::<Button>()) {
                break;
            }
            let button = &*b;
            // Only order-type buttons (action_var = a valid order id); build,
            // train and cancel buttons carry unit ids well past the order
            // table and are noise for the cast-mode classification.
            if button.action_var == 0 || button.action_var >= 499 {
                continue;
            }
            crate::log::log(&format!(
                "[cosmo-plus] abilities dump: unit={unit_id} ({name}) pos={} order={} action=0x{:08X}",
                button.position, button.action_var, button.action_func,
            ));
            logged += 1;
        }
    }
    crate::log::log(&format!(
        "[cosmo-plus] abilities dump: done ({logged} button(s) logged)"
    ));
}
