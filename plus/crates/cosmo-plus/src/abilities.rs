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
use std::sync::Mutex;

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

/// Orders that are NOT abilities (Cosmonarchy's orders.txt ids): movement,
/// attack/stop/patrol/hold/watch, worker gather/repair/build context,
/// transport load/unload, and the engine's script/context orders. Any other
/// order button on a unit's card makes it cycle-worthy for Tab. Morphs
/// (Drone Mutate, Unit/Building Morph, Archon merge), lift/land, nuke and
/// the extended custom orders all stay OUT of this list — they are
/// abilities worth cycling to.
const GENERIC_ORDERS: &[u16] = &[
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, // die/stop/guard/ignore/attack
    14, 15, 17, 18, 19, 20, 21, 22, 23, 24, // attack move, scripts, nothing
    34, 35, 36, 37, 38, 39, 40, // repair, addons, train, rally
    49, // follow
    50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 68, 69,
    // carrier/reaver/interceptor context
    77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, // harvest
    92, 93, 94, 95, 96, 97, // transport/load, idle powerup
    100, 101, 102, 103, // watch, creep scripts
    107, 108, // hold position
    123, 133, 150, 151, 152, // unknown, right-click script, collision, patrol
    154, 155, 156, 157, 158, 159, 160, 161, 162, 163, // computer/neutral
    168, 169, 170, 171, 172, 173, 174, // doors/traps, warp in (unused)
    177, 178, 179, 187, 188, // cleric movement, junk yard dog, fatal
];

/// Units that must cycle even though their card holds only generic orders.
/// The Lakizilisk (the lurker) has no manual burrow button — its card is
/// attack-only — but it is an essential unit to Tab to.
const ALWAYS_CYCLE: &[u16] = &[103, 425]; // ZergLakizilisk, ZergLakiziliskZorthos

/// Lazily-filled per-unit classification (None = not classified yet).
static WORTHY: Lazy<Mutex<Vec<Option<bool>>>> = Lazy::new(|| Mutex::new(Vec::new()));

/// Whether a unit's card carries at least one ability button (or the unit is
/// on the always-cycle list). Tab cycles only through worthy subgroups.
/// Results are cached per unit id; an unresolved buttonset table returns
/// false WITHOUT caching (it resolves during game init).
pub(crate) unsafe fn cycle_worthy(unit_id: u16) -> bool {
    if ALWAYS_CYCLE.contains(&unit_id) {
        return true;
    }
    let id = unit_id as usize;
    {
        let cache = WORTHY.lock().unwrap();
        if let Some(&Some(v)) = cache.get(id) {
            return v;
        }
    }
    let Some(worthy) = unsafe { classify(unit_id) } else {
        return false; // table not resolved yet; retry next time
    };
    let mut cache = WORTHY.lock().unwrap();
    if cache.len() <= id {
        cache.resize(id + 1, None);
    }
    cache[id] = Some(worthy);
    worthy
}

/// Read the unit's buttonset and look for a non-generic order button.
/// `None` = the buttonset table is not resolved yet.
unsafe fn classify(unit_id: u16) -> Option<bool> {
    let table = std::ptr::read(G_BUTTONSET_TABLE as *const *const ButtonSet);
    if table.is_null() || !crate::overlay::readable_ptr(table as *const u8, 4) {
        return None;
    }
    let bs = table.add(unit_id as usize);
    if !crate::overlay::readable_ptr(bs as *const u8, core::mem::size_of::<ButtonSet>()) {
        return None;
    }
    let count = (*bs).buttons_in_set.min(MAX_BUTTONS);
    let first = (*bs).first_button;
    if count == 0 || first.is_null() {
        return Some(false);
    }
    for i in 0..count as usize {
        let b = first.add(i);
        if !crate::overlay::readable_ptr(b as *const u8, core::mem::size_of::<Button>()) {
            break;
        }
        let order = (*b).action_var;
        if order == 0 || order >= 499 {
            continue; // build/train/cancel buttons carry unit ids
        }
        if !GENERIC_ORDERS.contains(&order) {
            return Some(true);
        }
    }
    Some(false)
}

/// Call from the draw pass: dump once when `CM_PLUS_DUMP_ABILITIES=1`. The
/// buttonset table resolves during game init, so a failed read retries on
/// the next frame until it succeeds.
pub(crate) unsafe fn dump_once() {
    if !crate::config::env_flag("CM_PLUS_DUMP_ABILITIES") || DONE.load(Ordering::SeqCst) {
        return;
    }
    let table = std::ptr::read(G_BUTTONSET_TABLE as *const *const ButtonSet);
    if table.is_null() || !crate::overlay::readable_ptr(table as *const u8, 4) {
        return; // not resolved yet; retry next frame
    }
    DONE.store(true, Ordering::SeqCst);
    dump_table(table);
}

/// Dump from the keyboard hook at the MENU (frame 0): the buttonset table
/// may not be resolved yet, so this retries on every key press until it is.
pub(crate) unsafe fn dump_on_menu_key() {
    if !crate::config::env_flag("CM_PLUS_DUMP_ABILITIES") || DONE.load(Ordering::SeqCst) {
        return;
    }
    let table = std::ptr::read(G_BUTTONSET_TABLE as *const *const ButtonSet);
    if table.is_null() || !crate::overlay::readable_ptr(table as *const u8, 4) {
        return; // not resolved yet; retry on the next key press
    }
    DONE.store(true, Ordering::SeqCst);
    dump_table(table);
}

unsafe fn dump_table(table: *const ButtonSet) {

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
