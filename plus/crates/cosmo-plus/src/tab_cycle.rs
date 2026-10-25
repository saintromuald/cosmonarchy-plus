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
//! Brood War has no subgroup concept, so this emulates it by **rotating the engine's
//! own client selection array**: the active subgroup's units are moved to the front,
//! so `selection[0]` — the unit the command card, spell hotkeys and portrait all
//! derive from — belongs to the active subgroup. Spells therefore dispatch from the
//! subgroup you Tab to (press Z with Mavericks active = Stim, Tab, press Z = the
//! Phalanx siege), while the selection multiset never changes, so move/attack orders
//! still reach every selected unit. The rotation is purely viewer-local (the client
//! selection table is not part of the synced game state and no select command or
//! sound is generated), matching how SC2 keeps the subgroup state per player.
//!
//! The Tab key is **swallowed** by a thread keyboard hook (installed after gptp's,
//! so it runs first) while in a game — stock Brood War binds Tab to hiding the
//! terrain, which this replaces. Outside chat and modal dialogs the key is never
//! swallowed. Tab stays inert when the selection has a single unit type.
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

use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowsHookExW, WH_KEYBOARD};

use cosmo_plus_core::hotkeys::{parse_key, DEFAULT_TAB_KEY};
use cosmo_plus_core::subgroups::subgroup_runs;

/// `g_client_selection_count` (u8) — maintained by `update_selected_unit_data`.
const CLIENT_SELECTION_COUNT: usize = 0x0059723D;
/// `g_chat_message_type` (`u8*`) — the pointed byte is nonzero while chat is open.
const CHAT_MESSAGE_TYPE: usize = 0x0068C144;
/// `g_popup_dialog` (`BinDlg**`) — nonzero while a modal game dialog is up.
const POPUP_DIALOG: usize = 0x006D5BF4;
/// `g_active_portrait_unit` (Unit**) — the unit the engine's wireframe, stats
/// panel and command buttons display. Written from the draw pass to highlight the
/// active subgroup.
const G_ACTIVE_PORTRAIT_UNIT: usize = 0x00597248;

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

/// The keyboard hook handle, kept for the process lifetime.
static HOOK: Mutex<Option<isize>> = Mutex::new(None);
/// Key events swallowed by the hook, queued for the draw pass: 1 = forward,
/// 2 = backward, 0 = none.
static QUEUED: AtomicU32 = AtomicU32::new(0);

/// Game frame until which the indicator stays visible.
static SHOW_UNTIL_FRAME: AtomicU32 = AtomicU32::new(0);

/// Selection identity and the active-subgroup state. `base_sorted` is the
/// selection (as sorted raw Unit* pointers) that the current cycle belongs to —
/// order-insensitive, because the cycle itself rotates the array. A selection
/// whose sorted pointers differ is a fresh one and resets the cycle.
/// `groups_order` remembers the unit-id order of the subgroups as they first
/// appeared, so the indicator numbers stay stable across rotations.
struct Cycle {
    /// Active subgroup index into `groups_order` (0-based).
    active: u32,
    groups: u32,
    count: usize,
    active_id: u16,
    active_ptr: u32,
    base_sorted: Vec<u32>,
    groups_order: Vec<u16>,
}

static CYCLE: Lazy<Mutex<Cycle>> = Lazy::new(|| {
    Mutex::new(Cycle {
        active: 0,
        groups: 0,
        count: 0,
        active_id: 0,
        active_ptr: 0,
        base_sorted: Vec::new(),
        groups_order: Vec::new(),
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
                "[cosmo-plus] tab cycle: {fwd_name} (VK 0x{vk:02X}) highlights the next \
                 subgroup; Shift+{fwd_name} goes back (SC2 Tab; the terrain toggle is \
                 replaced)"
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
                    "[cosmo-plus] tab cycle: {prev_name} (VK 0x{vk:02X}) highlights the \
                     previous subgroup"
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
    install_hook();
    // Resolve the selection table now: `gptp.qdp` (the mod) loads before this plugin
    // and has already rewritten the anchor immediates, so the extended table resolves
    // during the menu boot and is logged for the headless probe.
    unsafe {
        resolve_selection();
    }
}

fn install_hook() {
    unsafe {
        let h = SetWindowsHookExW(
            WH_KEYBOARD,
            Some(keyboard_proc),
            std::ptr::null_mut(),
            GetCurrentThreadId(),
        );
        if h.is_null() {
            crate::log::log(
                "[cosmo-plus] tab cycle: SetWindowsHookEx failed; the terrain toggle \
                 will stay bound to Tab",
            );
            return;
        }
        crate::log::log(
            "[cosmo-plus] tab cycle: keyboard hook installed (Tab is swallowed in-game)",
        );
        *HOOK.lock().unwrap() = Some(h as isize);
    }
}

fn call_next(code: i32, wparam: usize, lparam: isize) -> isize {
    use windows_sys::Win32::UI::WindowsAndMessaging::CallNextHookEx;
    unsafe {
        let hook = *HOOK.lock().unwrap();
        match hook {
            Some(h) => CallNextHookEx(h as _, code, wparam, lparam),
            None => 0,
        }
    }
}

/// WH_KEYBOARD hook: runs ahead of gptp's own hook. Swallows Tab (and the
/// configured backward key) while a game is running, so the stock "hide terrain"
/// binding never fires — the press is queued for the draw pass instead. Chat and
/// modal dialogs are never interfered with.
unsafe extern "system" fn keyboard_proc(code: i32, wparam: usize, lparam: isize) -> isize {
    if code < 0 {
        return call_next(code, wparam, lparam);
    }
    let key = wparam as u32;
    let down = lparam as u32 & (1 << 31) == 0; // bit 31 set = key-up
    if !down {
        return call_next(code, wparam, lparam);
    }
    // Never interfere outside a running game.
    if crate::frame::frame_count() == 0 {
        return call_next(code, wparam, lparam);
    }
    // Never while chat or a modal dialog is open.
    let chat = std::ptr::read(CHAT_MESSAGE_TYPE as *const *const u8);
    if !chat.is_null() && std::ptr::read(chat) != 0 {
        return call_next(code, wparam, lparam);
    }
    if std::ptr::read(POPUP_DIALOG as *const usize) != 0 {
        return call_next(code, wparam, lparam);
    }

    let fwd_vk = FWD_VK.load(Ordering::SeqCst);
    let prev_vk = PREV_VK.load(Ordering::SeqCst);
    let is_fwd = fwd_vk != 0 && key == fwd_vk as u32;
    let is_prev = (prev_vk != 0 && key == prev_vk as u32)
        || (is_fwd && (GetAsyncKeyState(VK_SHIFT) as u16 & 0x8000) != 0);
    if is_fwd || is_prev {
        QUEUED.store(if is_prev { 2 } else { 1 }, Ordering::SeqCst);
        return 1; // swallow: no terrain toggle, ever
    }
    call_next(code, wparam, lparam)
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
/// normal advance path. Diagnostic only; exists so the cycle can be exercised
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

/// Called from the draw pass each frame (like `idle_workers::poll`). Consumes the
/// key presses queued by the keyboard hook and rotates the engine's client
/// selection array so the next subgroup's units come first (selection[0] = the
/// unit whose spells/buttons the command card shows and dispatches).
pub(crate) unsafe fn poll() {
    let frame = crate::frame::frame_count();
    if frame == 0 {
        return;
    }
    let queued = QUEUED.swap(0, Ordering::SeqCst);
    if queued == 0 {
        return;
    }
    let dir: i32 = if queued == 2 { -1 } else { 1 };

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

    let mut sorted = ptrs.clone();
    sorted.sort_unstable();
    let mut cycle = CYCLE.lock().unwrap();
    // Fresh selection (different multiset, or different size, than the one the
    // current cycle belongs to) resets to the first subgroup, like re-selecting
    // in SC2. The array may be rotated from a previous cycle step, so identity
    // is the sorted pointer multiset, not the order.
    let same_selection = cycle.count == count && cycle.base_sorted == sorted;
    if !same_selection {
        cycle.active = 0;
        cycle.groups = 0;
        cycle.count = count;
        cycle.base_sorted = sorted;
        // Remember the subgroup type order as it appears in the fresh selection.
        let fresh_runs = subgroup_runs(&ids);
        cycle.groups_order = fresh_runs.iter().map(|r| r.unit_id).collect();
        if crate::config::env_flag("CM_PLUS_TAB_DEBUG") {
            crate::log::log(&format!(
                "[cosmo-plus] tab cycle debug: fresh selection count={count} ids={:?}",
                &ids[..count.min(12)],
            ));
        }
    }

    let runs = subgroup_runs(&ids);
    if runs.len() < 2 {
        cycle.groups = 1;
        if crate::config::env_flag("CM_PLUS_TAB_DEBUG") {
            crate::log::log("[cosmo-plus] tab cycle debug: single subgroup, Tab inert");
        }
        return; // single-type selection: Tab stays inert
    }
    if crate::config::env_flag("CM_PLUS_TAB_DEBUG") {
        crate::log::log(&format!(
            "[cosmo-plus] tab cycle debug: press dir={dir:+} count={count} ids={:?}",
            &ids[..count.min(12)],
        ));
    }

    let groups = runs.len() as u32;
    // The active subgroup always sits at runs[0]: a fresh selection has its
    // natural order, and a cycled one was rotated so the active run leads.
    // Tab moves to runs[1]; Shift+Tab wraps to runs[groups-1].
    let next_run_index = if dir > 0 { 1 } else { (groups - 1) as usize };
    let run = &runs[next_run_index];

    if !selftest {
        // Rotate the engine's own selection array: the chosen subgroup first,
        // everything else after it, order preserved. No select command and no
        // sound — this is viewer-local, exactly like SC2's subgroup pointer.
        rotate_selection(run.start, run.len, count);
    }

    let group_index = cycle
        .groups_order
        .iter()
        .position(|&id| id == run.unit_id)
        .unwrap_or(next_run_index) as u32;

    cycle.active = group_index;
    cycle.groups = groups;
    cycle.active_id = run.unit_id;
    cycle.active_ptr = ptrs[run.start];
    drop(cycle);

    SHOW_UNTIL_FRAME.store(frame.saturating_add(INDICATOR_FRAMES), Ordering::SeqCst);
    crate::log::log(&format!(
        "[cosmo-plus] tab cycle: active subgroup {}/{} ({} units, id {}, selection[0] now {})",
        group_index + 1,
        groups,
        count,
        run.unit_id,
        ptrs[run.start],
    ));
}

/// Rotate the engine's client selection array so the run
/// `[run_start, run_start + run_len)` is first. Reads the whole selection,
/// then writes it back rotated; the count is untouched.
unsafe fn rotate_selection(run_start: usize, run_len: usize, count: usize) -> bool {
    let base = SEL_BASE.load(Ordering::SeqCst);
    if base == 0 {
        return false;
    }
    let list = base as *mut *mut Unit;
    let mut rotated: Vec<u32> = Vec::with_capacity(count);
    for i in run_start..run_start + run_len {
        rotated.push(*list.add(i) as u32);
    }
    for i in 0..run_start {
        rotated.push(*list.add(i) as u32);
    }
    for i in run_start + run_len..count {
        rotated.push(*list.add(i) as u32);
    }
    for (i, &p) in rotated.iter().enumerate() {
        *list.add(i) = p as *mut Unit;
    }
    true
}

/// Called from the draw pass every frame (after `poll`): while a cycle is
/// active, confirm the engine's UI is aimed at the active subgroup's unit.
/// After the rotation, `selection[0]` already IS that unit, so the engine
/// re-derives the wireframe, stats panel and command buttons from it on its
/// own; the write is a belt-and-braces nudge for engine paths that only
/// refresh `g_active_portrait_unit` on selection-change events.
pub(crate) unsafe fn apply_display() {
    let cycle = CYCLE.lock().unwrap();
    if cycle.groups < 2 || cycle.active_ptr == 0 {
        return;
    }
    std::ptr::write(G_ACTIVE_PORTRAIT_UNIT as *mut u32, cycle.active_ptr);
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
