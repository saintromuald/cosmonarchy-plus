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

/// The live address of the GLOBAL THAT POINTS AT the table (0 = unresolved).
/// On the widescreen release build the vanilla global at 0x005187E8 holds
/// garbage (observed: 0x6), so the table is located by scanning the fork's
/// live `update_current_buttonset` reimplementation for table-pointer loads
/// and validating each candidate as a ButtonSet array.
static TABLE_GLOBAL_ADDR: AtomicUsize = AtomicUsize::new(0);
/// The resolved table representation: 0 = pointer array; 12/16/20/24 = a
/// value array of ButtonSet structs with that stride.
static TABLE_STRIDE: AtomicUsize = AtomicUsize::new(0);

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

use std::sync::atomic::AtomicUsize;

/// Whether `ptr` plausibly points at the ButtonSet array: several known unit
/// ids must resolve to readable ButtonSets with sane button counts.
unsafe fn table_plausible(table: *const ButtonSet) -> bool {
    if table.is_null() || !crate::overlay::readable_ptr(table as *const u8, 4) {
        return false;
    }
    let mut ok = 0;
    for &id in &[7u16, 41, 64, 106, 131] {
        let bs = table.add(id as usize);
        if !crate::overlay::readable_ptr(bs as *const u8, core::mem::size_of::<ButtonSet>()) {
            continue;
        }
        let count = (*bs).buttons_in_set;
        if count == 0 || count > MAX_BUTTONS {
            continue;
        }
        let first = (*bs).first_button;
        if first.is_null()
            || !crate::overlay::readable_ptr(first as *const u8, core::mem::size_of::<Button>())
        {
            continue;
        }
        ok += 1;
    }
    ok >= 3
}

/// Resolve the live buttonset table pointer: validate the vanilla exe global
/// first, then scan the fork's live `update_current_buttonset` (the E9 target
/// at 0x004599A0) for absolute table-pointer loads and validate each
/// candidate. Returns the pointer to the TABLE (not the global).
/// Strong validation of one ButtonSet: buttons_in_set sane, first_button a
/// readable pointer, and connected_unit == the unit id (the convention the
/// rest of the code relies on: buttonset id == unit id). The previous
/// count-only check latched onto a false-positive (a unit stat table) and
/// made every unit classify as ability-less.
unsafe fn buttonset_valid(bs: *const ButtonSet, id: u16) -> bool {
    if bs.is_null() || !crate::overlay::readable_ptr(bs as *const u8, core::mem::size_of::<ButtonSet>()) {
        return false;
    }
    let count = (*bs).buttons_in_set;
    if count == 0 || count > MAX_BUTTONS {
        return false;
    }
    let first = (*bs).first_button;
    if first.is_null() || !crate::overlay::readable_ptr(first as *const u8, core::mem::size_of::<Button>()) {
        return false;
    }
    (*bs).connected_unit as u16 == id
}

/// Sweep a memory range for the buttonset table. Returns (array base, stride)
/// where stride 0 = pointer array, stride > 0 = value array of ButtonSets.
unsafe fn sweep_for_table(base: usize, size: usize) -> Option<(usize, usize)> {
    const IDS: &[u16] = &[7, 35, 41, 63, 64, 106, 131, 314];
    const STRIDES: &[usize] = &[12, 16, 20, 24];
    let step = 4usize;
    let mut cell = base;
    let mut cells = 0u32;
    while cell + 4 <= base + size && cells < 3_000_000 {
        cells += 1;
        // (a) the cell's value is the array base.
        let arr = std::ptr::read(cell as *const usize);
        if arr != 0 && (arr as usize) % 4 == 0 {
            // pointer array: validate the range once, then raw-read.
            if crate::overlay::readable_ptr(arr as *const u8, 800 * 4) {
                let mut ok = 0;
                for &id in IDS {
                    let p = std::ptr::read((arr + id as usize * 4) as *const *const ButtonSet);
                    if buttonset_valid(p, id) {
                        ok += 1;
                    }
                }
                if ok >= 5 {
                    return Some((arr, 0));
                }
            }
            // value array: each id at arr + id*stride.
            for &st in STRIDES {
                let need = IDS[IDS.len() - 1] as usize * st + core::mem::size_of::<ButtonSet>();
                if !crate::overlay::readable_ptr(arr as *const u8, need) {
                    continue;
                }
                let mut ok = 0;
                for &id in IDS {
                    let bs = (arr + id as usize * st) as *const ButtonSet;
                    if buttonset_valid(bs, id) {
                        ok += 1;
                    }
                }
                if ok >= 5 {
                    return Some((arr, st));
                }
            }
        }
        // (b) the cell itself is a pointer-array base.
        if crate::overlay::readable_ptr(cell as *const u8, 800 * 4) {
            let mut ok = 0;
            for &id in IDS {
                let p = std::ptr::read((cell + id as usize * 4) as *const *const ButtonSet);
                if buttonset_valid(p, id) {
                    ok += 1;
                }
            }
            if ok >= 5 {
                return Some((cell, 0));
            }
        }
        cell += step;
    }
    None
}

/// The ButtonSet for a unit id, from the resolved table (either layout).
unsafe fn buttonset_for(unit_id: u16) -> *const ButtonSet {
    let base = TABLE_GLOBAL_ADDR.load(Ordering::SeqCst);
    if base == 0 {
        return std::ptr::null();
    }
    let stride = TABLE_STRIDE.load(Ordering::SeqCst);
    if stride == 0 {
        let p = std::ptr::read((base + unit_id as usize * 4) as *const *const ButtonSet);
        p
    } else {
        (base + unit_id as usize * stride) as *const ButtonSet
    }
}

unsafe fn resolve_table() -> *const ButtonSet {
    let cached = TABLE_GLOBAL_ADDR.load(Ordering::SeqCst);
    if cached != 0 {
        return std::ptr::read(cached as *const *const ButtonSet);
    }
    // 1. The vanilla global.
    let vanilla = std::ptr::read(G_BUTTONSET_TABLE as *const *const ButtonSet);
    if table_plausible(vanilla) {
        TABLE_GLOBAL_ADDR.store(G_BUTTONSET_TABLE, Ordering::SeqCst);
        crate::log::log(&format!(
            "[cosmo-plus] abilities: buttonset table @ 0x{:08X} (vanilla global 0x{G_BUTTONSET_TABLE:08X})",
            vanilla as usize,
        ));
        return vanilla;
    }
    // 2. Brute-force sweep: the table (pointer array or value array) lives in
    //    writable memory — gptp's own .data/.bss or a heap block referenced
    //    from it. For every 4-byte cell, (a) treat the cell's value as the
    //    array base, (b) treat the cell itself as the array base; validate a
    //    few known unit ids for each layout.
    if let Some(base) = crate::zoom::gptp_module() {
        if let Some(t) = sweep_for_table(base.0, base.1) {
            TABLE_GLOBAL_ADDR.store(t.0, Ordering::SeqCst);
            TABLE_STRIDE.store(t.1, Ordering::SeqCst);
            crate::log::log(&format!(
                "[cosmo-plus] abilities: buttonset table @ 0x{:08X} ({} stride {}) via memory sweep",
                t.0,
                if t.1 == 0 { "pointer array" } else { "value array" },
                t.1,
            ));
            return std::ptr::read(t.0 as *const *const ButtonSet);
        }
    }
    // 3. Scan the fork's live card updater for `A1 imm32` / `8B ?? imm32`
    //    loads whose pointed-to value is a plausible table.
    const UPDATER: usize = 0x004599A0;
    let first = std::ptr::read(UPDATER as *const u8);
    if first == 0xE9 {
        let rel = std::ptr::read((UPDATER + 1) as *const i32);
        let fns = UPDATER.wrapping_add(5).wrapping_add(rel as usize);
        for off in 0..2048usize {
            let site = fns + off;
            let b = std::ptr::read(site as *const u8);
            let (imm, len) = match b {
                0xA1 => (std::ptr::read((site + 1) as *const u32) as usize, 5),
                0x8B => {
                    let b2 = std::ptr::read((site + 1) as *const u8);
                    if (0x05..=0x3D).contains(&b2) && b2 % 8 == 5 {
                        (std::ptr::read((site + 2) as *const u32) as usize, 6)
                    } else {
                        continue;
                    }
                }
                _ => continue,
            };
            let _ = len;
            if !crate::overlay::readable_ptr(imm as *const u8, 4) {
                continue;
            }
            let table = std::ptr::read(imm as *const *const ButtonSet);
            if table_plausible(table) {
                TABLE_GLOBAL_ADDR.store(imm, Ordering::SeqCst);
                crate::log::log(&format!(
                    "[cosmo-plus] abilities: buttonset table @ 0x{:08X} (global 0x{imm:08X} \
                     found in the fork's card updater @ 0x{fns:08X}+0x{off:X})",
                    table as usize,
                ));
                return table;
            }
        }
    }
    static LOGGED: AtomicBool = AtomicBool::new(false);
    if !LOGGED.swap(true, Ordering::SeqCst) {
        crate::log::log(&format!(
            "[cosmo-plus] abilities: buttonset table UNRESOLVED (vanilla global holds 0x{:08X}); \
             the Tab cycle cannot classify abilities and stays inert",
            vanilla as usize,
        ));
    }
    std::ptr::null()
}

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
    25, 26, 30, 31, 32, 33, // DroneStartBuild 0x19, DroneBuild 0x1A,
    //   BuildTerran 0x1E, BuildProtoss1/2 0x1F/0x20, ConstructingBuilding 0x21
    //   (worker construction orders — they appear in the COMMAND byte for
    //   build/morph commands and must never be narrowed; the structure id
    //   rides in the command's payload, which narrowing would drop)
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

/// Whether a command order byte is an ability-class order (a spell/morph/
/// research/… — NOT a generic movement/harvest/construction order). Build
/// and train commands are excluded three ways: card build buttons carry unit
/// ids in the order slot (>= 499), the worker construction orders (0x19,
/// 0x1A, 0x1E-0x21) are in GENERIC_ORDERS, and the command hook additionally
/// broadcasts any command with a build/morph payload (see casting.rs). The
/// tab cycle's subgroup-scoped execution narrows only ability-class orders;
/// everything else broadcasts.
pub(crate) fn is_ability_order(order: u8) -> bool {
    let o = order as u16;
    o != 0 && o < 499 && !GENERIC_ORDERS.contains(&o)
}

/// Whether the unit type's card carries the given order (its buttonset has a
/// button with `action_var == order`). Used by the narrowing rule to target
/// the subgroup that can actually execute the order — deterministic, because
/// the buttonset table is identical on every peer. `false` while the table is
/// unresolved.
pub(crate) unsafe fn has_order(unit_id: u16, order: u8) -> bool {
    if resolve_table().is_null() {
        return false;
    }
    let bs = buttonset_for(unit_id);
    if bs.is_null() || !crate::overlay::readable_ptr(bs as *const u8, core::mem::size_of::<ButtonSet>()) {
        return false;
    }
    let count = (*bs).buttons_in_set.min(MAX_BUTTONS);
    let first = (*bs).first_button;
    if count == 0 || first.is_null() {
        return false;
    }
    for i in 0..count as usize {
        let b = first.add(i);
        if !crate::overlay::readable_ptr(b as *const u8, core::mem::size_of::<Button>()) {
            break;
        }
        if (*b).action_var == order as u16 {
            return true;
        }
    }
    false
}

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
    let classified = unsafe { classify(unit_id) };
    let Some(worthy) = classified else {
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
    if resolve_table().is_null() {
        return None;
    }
    let bs = buttonset_for(unit_id);
    if bs.is_null() || !crate::overlay::readable_ptr(bs as *const u8, core::mem::size_of::<ButtonSet>()) {
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
    let table = resolve_table();
    if table.is_null() {
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
    let table = resolve_table();
    if table.is_null() {
        return; // not resolved yet; retry on the next key press
    }
    DONE.store(true, Ordering::SeqCst);
    dump_table(table);
}

unsafe fn dump_table(table: *const ButtonSet) {

    crate::log::log("[cosmo-plus] abilities dump: start");
    let mut logged = 0u32;
    for unit_id in 0..SCAN_UNITS {
        let bs = buttonset_for(unit_id as u16);
        if bs.is_null() || !crate::overlay::readable_ptr(bs as *const u8, core::mem::size_of::<ButtonSet>()) {
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
