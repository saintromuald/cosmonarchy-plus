//! SC2-style Tab subgroup cycling ("Next Subgroup" / "Previous Subgroup").
//!
//! How StarCraft II does it (the reference for this feature):
//!   * a mixed selection is split into **subgroups**, one per distinct unit type, in
//!     the order the types appear in the selection (production buildings and units
//!     behave the same — see Blizzard's control-group guide);
//!   * **Tab** cycles the *active subgroup* forward, **Shift+Tab** backward;
//!   * only the active subgroup's buttons are shown, so its spells/abilities win the
//!     hotkeys, while plain orders (move/attack/stop) still go to every selected unit;
//!   * selecting fresh resets to the first subgroup.
//!
//! Brood War has no subgroup concept, so this emulates it by **rotating the client
//! selection** so the active subgroup's units come first: BW lists the first-selected
//! unit type's buttons first on the command card and gives them hotkey priority, so
//! after each Tab the active group's abilities (spells, production, morphs) are the
//! ones under the player's fingers. The selection content never changes — every unit
//! stays selected, so movement/attack orders still apply to the whole group, exactly
//! like SC2. The wireframe reorders to show the active subgroup first, the visible
//! "highlight".
//!
//! The rotation is **client-side only** (`send_command = 0` into the engine's own
//! selection routine), viewer-local and netcode-safe, like the idle-worker/larva
//! selections. Tab stays inert when the selection has a single unit type, when chat
//! or a modal dialog is open, or outside a game — the key is never swallowed.
//!
//! Cosmonarchy extends the selection to `SelectionLimit` = 240 units and moves the
//! client selection table out of the exe into `ccmu_extender` (inside `gptp.qdp`).
//! Its runtime address is recovered from the immediates the extender patches into
//! `update_selected_unit_data` (0x004C38BE / 0x004C38F4), validated against the
//! end-of-table immediate, with the 1.16.1 12-slot table as a fallback.
//!
//! Keys: `CM_PLUS_TAB_KEY` (default `TAB`), `CM_PLUS_TAB_PREV_KEY` (default empty =
//! Shift+Tab), kill switch `CM_PLUS_NO_TAB_CYCLE=1`.

use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicUsize, Ordering};
use std::sync::Mutex;

use once_cell::sync::Lazy;

use bw_dat::structs::Unit;

use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

use cosmo_plus_core::hotkeys::{parse_key, DEFAULT_TAB_KEY};
use cosmo_plus_core::subgroups::subgroup_rotation;

/// `g_client_selection_count` (u8) — maintained by `update_selected_unit_data`.
const CLIENT_SELECTION_COUNT: usize = 0x0059723D;
/// `g_chat_message_type` (`u8*`) — the pointed byte is nonzero while chat is open.
const CHAT_MESSAGE_TYPE: usize = 0x0068C144;
/// `g_popup_dialog` (`BinDlg**`) — nonzero while a modal game dialog is up.
const POPUP_DIALOG: usize = 0x006D5BF4;

/// Immediates the selection extender rewrites with the runtime address of
/// `ccmu_extender.new_client_selection_group` and of its one-past-end slot.
/// (`update_selected_unit_data`: `mov edi, imm32` / `cmp ebx, imm32`.)
const ANCHOR_SEL_BASE: usize = 0x004C38BE;
const ANCHOR_SEL_END: usize = 0x004C38F4;
/// Same immediates in `unit_CanCloak` (`mov edx, imm32` / `cmp edx, imm32`) —
/// secondary anchors in case the primary pair is disturbed.
const ANCHOR_SEL_BASE2: usize = 0x00423556;
const ANCHOR_SEL_END2: usize = 0x004235CD;

/// The 1.16.1 selection table, used only when the extender anchors do not validate.
const VANILLA_SELECTION: usize = 0x00597208;
/// Cosmonarchy's extended selection limit (`SelectionLimit`).
const EXTENDED_LIMIT: usize = 240;
/// Stock 1.16.1 selection limit.
const VANILLA_LIMIT: usize = 12;

const VK_SHIFT: i32 = 0x10;

/// How many frames the subgroup indicator stays visible after a Tab press (~1.5 s).
const INDICATOR_FRAMES: u32 = 36;

/// Resolved selection table base and its capacity (set once, at first use).
static SEL_BASE: AtomicUsize = AtomicUsize::new(0);
static SEL_LIMIT: AtomicUsize = AtomicUsize::new(0);
static RESOLVED: AtomicBool = AtomicBool::new(false);

/// Forward cycle key (default Tab) and optional separate backward key (0 = none).
static FWD_VK: AtomicU16 = AtomicU16::new(0);
static PREV_VK: AtomicU16 = AtomicU16::new(0);
/// Rising-edge state: bit 14 = forward key held last poll, bit 15 = prev key.
static KEY_EDGE: AtomicU32 = AtomicU32::new(0);

/// Game frame until which the indicator stays visible.
static SHOW_UNTIL_FRAME: AtomicU32 = AtomicU32::new(0);

/// Selection identity and rotation state. `base` is the selection (as raw Unit*
/// pointers) at the last reset; `written` is what the last rotation wrote to the
/// engine. A selection that matches neither is a fresh one and resets the cycle.
struct Cycle {
    active: u32,
    groups: u32,
    count: usize,
    active_id: u16,
    base: [u32; EXTENDED_LIMIT],
    written: [u32; EXTENDED_LIMIT],
}

static CYCLE: Lazy<Mutex<Cycle>> = Lazy::new(|| {
    Mutex::new(Cycle {
        active: 0,
        groups: 0,
        count: 0,
        active_id: 0,
        base: [0; EXTENDED_LIMIT],
        written: [0; EXTENDED_LIMIT],
    })
});

pub(crate) fn install() {
    if crate::config::env_flag("CM_PLUS_NO_TAB_CYCLE") {
        crate::log::log("[cosmo-plus] tab cycle: disabled (CM_PLUS_NO_TAB_CYCLE)");
        return;
    }
    let fwd_name = crate::config::env("CM_PLUS_TAB_KEY")
        .unwrap_or_else(|| DEFAULT_TAB_KEY.to_string());
    match parse_key(&fwd_name) {
        Some(vk) => {
            FWD_VK.store(vk, Ordering::SeqCst);
            crate::log::log(&format!(
                "[cosmo-plus] tab cycle: {fwd_name} (VK 0x{vk:02X}) cycles to the next \
                 subgroup of a mixed selection; Shift+{fwd_name} cycles backward (SC2 Tab)"
            ));
        }
        None => {
            crate::log::log(&format!(
                "[cosmo-plus] tab cycle: unknown CM_PLUS_TAB_KEY={fwd_name:?}; subgroup \
                 cycling disabled (use e.g. TAB, F12, SPACE, or a hex VK like 0x09)"
            ));
            return;
        }
    }
    if let Some(prev_name) = crate::config::env("CM_PLUS_TAB_PREV_KEY") {
        match parse_key(&prev_name) {
            Some(vk) if vk != FWD_VK.load(Ordering::SeqCst) => {
                PREV_VK.store(vk, Ordering::SeqCst);
                crate::log::log(&format!(
                    "[cosmo-plus] tab cycle: {prev_name} (VK 0x{vk:02X}) cycles backward"
                ));
            }
            Some(_) => crate::log::log(
                "[cosmo-plus] tab cycle: CM_PLUS_TAB_PREV_KEY equals the forward key; \
                 backward stays Shift+forward",
            ),
            None => crate::log::log(&format!(
                "[cosmo-plus] tab cycle: unknown CM_PLUS_TAB_PREV_KEY={prev_name:?}; \
                 backward stays Shift+forward"
            )),
        }
    }
    // Resolve the selection table now: `gptp.qdp` (the mod) loads before this plugin
    // and has already rewritten the anchor immediates, so the extended table resolves
    // during the menu boot and is logged for the headless probe.
    unsafe {
        resolve_selection();
    }
}

/// Resolve the client selection table at runtime. The extender rewrites two
/// immediates inside `update_selected_unit_data` with the table address and its
/// one-past-end address; a valid pair is exactly `EXTENDED_LIMIT * 4` bytes apart.
/// Falls back to the vanilla 12-slot table when the pair does not validate.
unsafe fn resolve_selection() -> bool {
    if RESOLVED.load(Ordering::SeqCst) {
        return SEL_BASE.load(Ordering::SeqCst) != 0;
    }
    let anchors = [
        (ANCHOR_SEL_BASE, ANCHOR_SEL_END),
        (ANCHOR_SEL_BASE2, ANCHOR_SEL_END2),
    ];
    for &(base_site, end_site) in &anchors {
        let base = std::ptr::read(base_site as *const u32) as usize;
        let end = std::ptr::read(end_site as *const u32) as usize;
        if base != 0 && end == base + EXTENDED_LIMIT * 4 {
            SEL_BASE.store(base, Ordering::SeqCst);
            SEL_LIMIT.store(EXTENDED_LIMIT, Ordering::SeqCst);
            RESOLVED.store(true, Ordering::SeqCst);
            crate::log::log(&format!(
                "[cosmo-plus] tab cycle: extended selection table at 0x{base:08X} \
                 ({EXTENDED_LIMIT} slots, anchor 0x{base_site:08X})"
            ));
            return true;
        }
    }
    SEL_BASE.store(VANILLA_SELECTION, Ordering::SeqCst);
    SEL_LIMIT.store(VANILLA_LIMIT, Ordering::SeqCst);
    RESOLVED.store(true, Ordering::SeqCst);
    crate::log::log(
        "[cosmo-plus] tab cycle: selection extender anchors did not validate; using the \
         vanilla 12-slot table",
    );
    true
}

/// Read the client selection as (unit ids, raw pointers), in selection order.
/// Returns `None` when the selection is empty or the table is not ready.
unsafe fn read_selection() -> Option<(Vec<u16>, Vec<u32>)> {
    if !resolve_selection() {
        return None;
    }
    let base = SEL_BASE.load(Ordering::SeqCst);
    if base == 0 {
        return None;
    }
    let limit = SEL_LIMIT.load(Ordering::SeqCst).max(1);
    let count =
        (std::ptr::read(CLIENT_SELECTION_COUNT as *const u8) as usize).min(limit);
    if count == 0 {
        return None;
    }
    let list = base as *const *mut Unit;
    let mut ids = Vec::with_capacity(count);
    let mut ptrs = Vec::with_capacity(count);
    for i in 0..count {
        let unit = *list.add(i);
        if unit.is_null() {
            break;
        }
        ptrs.push(unit as u32);
        ids.push((*unit).unit_id);
    }
    if ids.is_empty() {
        return None;
    }
    Some((ids, ptrs))
}

/// `CM_PLUS_TAB_SELFTEST=1`: build a synthetic mixed selection from the local
/// player's units (up to 24, in unit-table order — workers and buildings both, so
/// the list has 2+ subgroups whenever the player owns both) and feed it through the
/// normal rotation path. Diagnostic only; exists so the rotation can be exercised
/// headlessly without a naturally mixed selection on the map.
unsafe fn selftest_selection() -> Option<(Vec<u16>, Vec<u32>)> {
    let Some(local) = crate::idle_workers::local_player() else {
        return None;
    };
    let mut ids: Vec<u16> = Vec::new();
    let mut ptrs: Vec<u32> = Vec::new();
    crate::frame::for_each_unit(|u| {
        if ptrs.len() >= 24 {
            return;
        }
        let unit = &*u;
        if unit.player as u32 != local {
            return;
        }
        ptrs.push(u as u32);
        ids.push(unit.unit_id);
    });
    if ids.len() < 2 {
        return None;
    }
    crate::log::log(&format!(
        "[cosmo-plus] tab cycle selftest: synthetic selection of {} units, ids {:?}",
        ids.len(),
        &ids[..ids.len().min(12)]
    ));
    Some((ids, ptrs))
}

/// Called from the draw pass each frame (like `idle_workers::poll`).
pub(crate) unsafe fn poll() {
    let frame = crate::frame::frame_count();
    if frame == 0 {
        return;
    }
    let fwd_vk = FWD_VK.load(Ordering::SeqCst);
    if fwd_vk == 0 {
        return;
    }
    // Never react while chat or a modal dialog is open (Tab must stay a plain key
    // there), mirroring the mod's own `handle_key_event` guard.
    let chat = std::ptr::read(CHAT_MESSAGE_TYPE as *const *const u8);
    if !chat.is_null() && std::ptr::read(chat) != 0 {
        return;
    }
    if std::ptr::read(POPUP_DIALOG as *const usize) != 0 {
        return;
    }

    let prev_vk = PREV_VK.load(Ordering::SeqCst);
    let fwd_down = (GetAsyncKeyState(fwd_vk as i32) as u16 & 0x8000) != 0;
    let prev_down =
        prev_vk != 0 && (GetAsyncKeyState(prev_vk as i32) as u16 & 0x8000) != 0;

    let mut edge = KEY_EDGE.load(Ordering::SeqCst);
    let fwd_was = edge & (1 << 14) != 0;
    let prev_was = edge & (1 << 15) != 0;
    edge &= !(1 << 14 | 1 << 15);
    if fwd_down {
        edge |= 1 << 14;
    }
    if prev_down {
        edge |= 1 << 15;
    }
    KEY_EDGE.store(edge, Ordering::SeqCst);

    let fwd_press = fwd_down && !fwd_was;
    let prev_press = prev_down && !prev_was;
    if !(fwd_press || prev_press) {
        return;
    }
    // SC2: Tab forward, Shift+Tab backward.
    let dir: i32 = if prev_press {
        -1
    } else if (GetAsyncKeyState(VK_SHIFT) as u16 & 0x8000) != 0 {
        -1
    } else {
        1
    };

    let selftest = crate::config::env_flag("CM_PLUS_TAB_SELFTEST");
    let picked = if selftest {
        selftest_selection()
    } else {
        read_selection()
    };
    let Some((ids, ptrs)) = picked else {
        return;
    };
    let count = ids.len();
    if count < 2 {
        return;
    }

    let mut cycle = CYCLE.lock().unwrap();
    // Fresh selection (anything that is not the result of our own last rotation)
    // resets the cycle to its first subgroup, like re-selecting in SC2.
    let is_our_rotation = cycle.count == count && cycle.written[..count] == ptrs[..];
    if !is_our_rotation {
        cycle.active = 0;
        cycle.count = count;
        cycle.base[..count].copy_from_slice(&ptrs);
    }
    if crate::config::env_flag("CM_PLUS_TAB_DEBUG") {
        crate::log::log(&format!(
            "[cosmo-plus] tab cycle debug: press dir={dir:+} count={count} ids={:?} \
             base={} our_rotation={is_our_rotation}",
            &ids[..count.min(12)],
            cycle.count,
        ));
    }
    let Some(rotation) = subgroup_rotation(&ids, cycle.active, dir) else {
        cycle.groups = 1;
        if crate::config::env_flag("CM_PLUS_TAB_DEBUG") {
            crate::log::log("[cosmo-plus] tab cycle debug: single subgroup, Tab inert");
        }
        return; // single-type selection: Tab stays inert
    };

    // Apply the rotated order through the engine's own selection routine
    // (client-side only: silent, no synced command).
    let mut rotated: [*mut Unit; EXTENDED_LIMIT] =
        [core::ptr::null_mut(); EXTENDED_LIMIT];
    for (i, &idx) in rotation.order.iter().enumerate() {
        rotated[i] = ptrs[idx as usize] as *mut Unit;
    }
    crate::select::select_units_if_alt_not_held(rotated.as_ptr(), count as u32, 0, 0);

    for (i, &idx) in rotation.order.iter().enumerate() {
        cycle.written[i] = ptrs[idx as usize];
    }
    cycle.active = rotation.active;
    cycle.groups = rotation.groups;
    cycle.active_id = rotation.active_unit_id;
    drop(cycle);

    SHOW_UNTIL_FRAME.store(frame.saturating_add(INDICATOR_FRAMES), Ordering::SeqCst);
    crate::log::log(&format!(
        "[cosmo-plus] tab cycle: active subgroup {}/{} ({} units, id {})",
        rotation.active + 1,
        rotation.groups,
        count,
        rotation.active_unit_id
    ));
}

/// Indicator state for the overlay: `Some((active, total))` while the Tab indicator
/// should be visible.
pub(crate) fn indicator(frame: u32) -> Option<(u32, u32)> {
    if frame == 0 || frame > SHOW_UNTIL_FRAME.load(Ordering::SeqCst) {
        return None;
    }
    let cycle = CYCLE.lock().unwrap();
    if cycle.groups < 2 {
        return None;
    }
    Some((cycle.active + 1, cycle.groups))
}

/// The active subgroup's unit id (0 = none), for the indicator's unit name.
pub(crate) fn active_unit_id() -> u16 {
    CYCLE.lock().unwrap().active_id
}
