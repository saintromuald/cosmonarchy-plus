//! Smart Tab: cycle the active subgroup of the client selection.
//!
//! SC2-style tabbing for BW: with a mixed selection (say 3 marines + 2 medics), Tab
//! rotates the client selection so the next distinct unit type becomes the primary
//! entry. The command card follows the primary unit, so each Tab press brings up the
//! next group's abilities, and the wireframe reorders to show that group first — the
//! visible "highlight". Rotation wraps back to the first group.
//!
//! The rotation is **client-side only** (`send_command = 0` into the engine's own
//! selection routine), so it is viewer-local and netcode-safe, exactly like the idle
//! workers / larva-pick selections.

use std::sync::atomic::{AtomicU32, Ordering};

use bw_dat::structs::Unit;

use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

/// `g_client_selection_group` — the client's current selection, `UnitsSel` of 12
/// `Unit*` in place (same table larva_pick reads).
const CLIENT_SELECTION: usize = 0x00597208;
/// `g_client_selection_count` (u8).
const CLIENT_SELECTION_COUNT: usize = 0x0059723D;
const SELECTION_CAP: usize = 12;
/// Tab's virtual-key code.
const VK_TAB: i32 = 0x09;

/// Tab press edge (bit 14 = previously held), like idle_workers' KEY_EDGE.
static KEY_EDGE: AtomicU32 = AtomicU32::new(0);
/// Selection signature last seen (`unit_id << 8 | count`), to reset the rotation when
/// the player makes a fresh selection.
static LAST_SIG: AtomicU32 = AtomicU32::new(0);
/// How many Tab rotations are in effect since the last fresh selection.
static ROTATIONS: AtomicU32 = AtomicU32::new(0);
/// Distinct-group count of the current selection (0 = nothing to show).
static GROUPS: AtomicU32 = AtomicU32::new(0);
/// Game frame until which the indicator stays visible.
static SHOW_UNTIL_FRAME: AtomicU32 = AtomicU32::new(0);

pub(crate) fn install() {
    crate::log::log(
        "[cosmo-plus] smart tab: Tab cycles the active subgroup of a mixed selection \
         (units and buildings), client-side",
    );
}

/// Called from the draw pass each frame (like `idle_workers::poll`).
pub(crate) unsafe fn poll() {
    let frame = crate::frame::frame_count();
    if frame == 0 {
        return;
    }

    let down = (GetAsyncKeyState(VK_TAB) as u16 & 0x8000) != 0;
    let prev = KEY_EDGE.load(Ordering::SeqCst);
    let was = prev & (1 << 14) != 0;
    let mut next = prev & !(1 << 14);
    if down {
        next |= 1 << 14;
    }
    KEY_EDGE.store(next, Ordering::SeqCst);
    if !(down && !was) {
        return; // no fresh Tab press
    }

    let count = std::ptr::read(CLIENT_SELECTION_COUNT as *const u8) as usize;
    if count < 2 {
        return;
    }
    let list = CLIENT_SELECTION as *const *mut Unit;
    let first_id = (*(*list)).unit_id;

    // Reset the rotation when the selection changed since the last Tab.
    let sig = ((first_id as u32) << 8) | count as u32;
    if sig != LAST_SIG.load(Ordering::SeqCst) {
        LAST_SIG.store(sig, Ordering::SeqCst);
        ROTATIONS.store(0, Ordering::SeqCst);
    }

    // Find the first entry of a different type: the split point of the rotation.
    let mut split = None;
    for i in 1..count {
        if (*(*list.add(i))).unit_id != first_id {
            split = Some(i);
            break;
        }
    }
    let Some(split) = split else {
        GROUPS.store(1, Ordering::SeqCst);
        return; // single-type selection: nothing to cycle
    };

    // Count distinct groups (consecutive runs of the same unit id).
    let mut groups = 1u32;
    let mut last = first_id;
    for i in 1..count {
        let id = (*(*list.add(i))).unit_id;
        if id != last {
            groups += 1;
            last = id;
        }
    }
    GROUPS.store(groups, Ordering::SeqCst);

    // Rotate: [split..count] ++ [0..split].
    let mut rotated: [*mut Unit; SELECTION_CAP] = [core::ptr::null_mut(); SELECTION_CAP];
    for i in 0..count {
        rotated[i] = *list.add((split + i) % count);
    }
    // Client-side selection update: no sound, no synced command.
    crate::select::select_units_if_alt_not_held(rotated.as_ptr(), count as u32, 0, 0);

    ROTATIONS.fetch_add(1, Ordering::SeqCst);
    SHOW_UNTIL_FRAME.store(frame.saturating_add(36), Ordering::SeqCst); // ~1.5 s at 24 fps
    crate::log::log(&format!(
        "[cosmo-plus] smart tab: rotated to group {}/{groups} ({count} units)",
        active_group()
    ));
}

/// The 1-based active group index.
fn active_group() -> u32 {
    ROTATIONS.load(Ordering::SeqCst) % GROUPS.load(Ordering::SeqCst).max(1) + 1
}

/// Indicator state for the overlay: `Some((active, total))` while the Tab indicator
/// should be visible.
pub(crate) fn indicator(frame: u32) -> Option<(u32, u32)> {
    if frame > SHOW_UNTIL_FRAME.load(Ordering::SeqCst) {
        return None;
    }
    let total = GROUPS.load(Ordering::SeqCst);
    if total < 2 {
        return None;
    }
    Some((active_group(), total))
}
