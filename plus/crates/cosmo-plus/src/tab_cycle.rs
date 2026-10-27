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
//! Brood War has no subgroup concept, so this drives the engine's own card
//! pipeline the way the fork's buttonset code reads it:
//!
//!   * `get_selected_unit()` (buttonsets.cpp) = the **first completed unit in
//!     `g_client_selection_group`** — the client selection table. So the active
//!     subgroup is made first by **rotating that table** (viewer-local: the table
//!     is not part of the synced game state).
//!   * Mixed selections otherwise get the generic `Buttons_GroupMixed` card
//!     (update_buttonset_ex). The override writes `g_current_buttonset_id` =
//!     the active subgroup unit's `current_buttonset`, so every frame the fork's
//!     `update_current_buttonset` (0x004599A0) binds the subgroup's full card.
//!   * The portrait/wireframe (`g_active_portrait_unit`) follows the same unit,
//!     and spell dispatch reads the same pointer, so hotkeys route to the
//!     active subgroup exactly like SC2.
//!
//! The engine re-copies its canonical selection order (`active_player_selection`)
//! and re-derives GroupMixed on its own events, so `apply_display` re-asserts the
//! rotation + override every frame while the cycle is active (guarded by the
//! selection identity, so a fresh selection resets to the first subgroup).
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
use cosmo_plus_core::subgroups::{subgroup_runs, SubgroupAction, SubgroupModel, SubgroupRun};

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
/// `g_current_buttonset_id` (s16, buttonsets.h) — the card's buttonset id; the
/// fork's per-frame `update_current_buttonset` reads it. The override writes the
/// active subgroup's buttonset here so the card shows that subgroup's spells
/// instead of the mixed-selection GroupMixed card.
const G_CURRENT_BUTTONSET_ID: usize = 0x0068C1C4;
/// `update_current_buttonset` (0x004599A0, buttonsets.h) — the fork's card
/// updater, replaced by a jump (E9) to its C++ reimplementation. Called after
/// writing the override so the card rebuilds from the active subgroup.
const ADDR_UPDATE_CURRENT_BUTTONSET: usize = 0x004599A0;
/// `update_buttonset` (0x00458DE0) — re-derives the stock card; called once
/// when the cycle deactivates so the engine's own card (single or GroupMixed)
/// comes back.
const ADDR_UPDATE_BUTTONSET: usize = 0x00458DE0;
static UPDATE_CURRENT_BUTTONSET_ADDR: u32 = ADDR_UPDATE_CURRENT_BUTTONSET as u32;
static UPDATE_BUTTONSET_ADDR: u32 = ADDR_UPDATE_BUTTONSET as u32;
/// Whether the fork's jump patch is present at `update_current_buttonset`
/// (checked at install): only then is calling the address register-safe. On an
/// unpatched build the card override is skipped (the cycle still rotates the
/// table and aims the portrait).
static CARD_READY: AtomicBool = AtomicBool::new(false);

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

/// Resolved status for the boot report.
pub(crate) fn status() -> String {
    let vk = FWD_VK.load(Ordering::SeqCst);
    if vk == 0 {
        "disabled (no valid key)".to_string()
    } else {
        format!("armed (VK 0x{vk:02X})")
    }
}
static PREV_VK: AtomicU16 = AtomicU16::new(0);

/// The keyboard hook handle, kept for the process lifetime.
static HOOK: Mutex<Option<isize>> = Mutex::new(None);
/// Key events swallowed by the hook, queued for the draw pass: 1 = forward,
/// 2 = backward, 0 = none.
static QUEUED: AtomicU32 = AtomicU32::new(0);

/// Game frame until which the indicator stays visible.
static SHOW_UNTIL_FRAME: AtomicU32 = AtomicU32::new(0);

/// Display state for the overlay/portrait aim: the active subgroup and the
/// unit it points at. The cycle arithmetic itself lives in the pure
/// `cosmo_plus_core::subgroups::SubgroupModel` (host-tested).
struct Display {
    /// Active subgroup index (0-based).
    active: u32,
    groups: u32,
    active_id: u16,
    active_ptr: u32,
}

static DISPLAY: Lazy<Mutex<Display>> = Lazy::new(|| {
    Mutex::new(Display {
        active: 0,
        groups: 0,
        active_id: 0,
        active_ptr: 0,
    })
});

/// The SC2 subgroup state machine (pure; see cosmo-plus-core).
static MODEL: Lazy<Mutex<Option<SubgroupModel>>> = Lazy::new(|| Mutex::new(None));

/// Order-insensitive selection identity: FNV-1a over the sorted raw unit
/// pointers, so the rotated array still matches the selection it belongs to.
fn selection_identity(ptrs: &[u32]) -> u64 {
    let mut sorted = ptrs.to_vec();
    sorted.sort_unstable();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &p in &sorted {
        h ^= p as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

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
    rearm_announce();
    // Resolve the selection table now: `gptp.qdp` (the mod) loads before this plugin
    // and has already rewritten the anchor immediates, so the extended table resolves
    // during the menu boot and is logged for the headless probe.
    unsafe {
        resolve_selection();
        // The card override needs the fork's jump-patched update_current_buttonset:
        // calling the vanilla function with our register state would be unsafe.
        let first = std::ptr::read(ADDR_UPDATE_CURRENT_BUTTONSET as *const u8);
        CARD_READY.store(first == 0xE9, Ordering::SeqCst);
        crate::log::log(&format!(
            "[cosmo-plus] tab cycle: card override {} (update_current_buttonset first byte 0x{first:02X})",
            if CARD_READY.load(Ordering::SeqCst) { "armed" } else { "disabled" }
        ));
    }
}

/// Re-parse the forward/backward keys from config (in-place reload).
pub(crate) fn rearm() {
    if let Some(name) = crate::config::env("CM_PLUS_TAB_KEY") {
        if let Some(vk) = parse_key(&name) {
            FWD_VK.store(vk, Ordering::SeqCst);
        }
    }
    if let Some(name) = crate::config::env("CM_PLUS_TAB_PREV_KEY") {
        if let Some(vk) = parse_key(&name) {
            PREV_VK.store(vk, Ordering::SeqCst);
        }
    }
    rearm_announce();
}

fn rearm_announce() {
    crate::log::log(&format!(
        "[cosmo-plus] tab cycle: reloaded keys (fwd VK 0x{:02X}, prev VK 0x{:02X})",
        FWD_VK.load(Ordering::SeqCst),
        PREV_VK.load(Ordering::SeqCst)
    ));
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
        crate::state::hook("keyboard:tab-cycle", h as usize, 0, 0);
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
    if crate::config::env_flag("CM_PLUS_TAB_DEBUG") {
        let fwd = FWD_VK.load(Ordering::SeqCst);
        if key == fwd as u32 || key == PREV_VK.load(Ordering::SeqCst) as u32 {
            crate::log::log(&format!(
                "[cosmo-plus] tab debug: hook saw key 0x{key:02X} (fwd 0x{fwd:02X}) frame={}",
                crate::frame::frame_count_safe()
            ));
        }
    }
    // Never interfere outside a running game.
    if crate::frame::frame_count() == 0 {
        // Menu: any key press triggers the one-shot abilities dump when the
        // flag is armed (the buttonset table is live by now).
        crate::abilities::dump_on_menu_key();
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
        if crate::config::env_flag("CM_PLUS_TAB_DEBUG") {
            crate::log::log("[cosmo-plus] tab debug: hook queued the press (swallowed)");
        }
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

/// The client selection as raw pointers, for the casting hook.
pub(crate) unsafe fn selection_ptrs() -> Option<(Vec<u16>, Vec<u32>)> {
    read_selection()
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
/// player's units (up to `EXTENDED_LIMIT`, in unit-table order — workers and
/// buildings both, so the list has 2+ subgroups whenever the player owns both)
/// and feed it through the normal advance path. Diagnostic only; exists so the
/// cycle can be exercised headlessly without a naturally mixed selection on the
/// map.
unsafe fn selftest_selection() -> Option<(Vec<u16>, Vec<u32>)> {
    let Some(local) = crate::idle_workers::local_player() else {
        return None;
    };
    let mut ids: Vec<u16> = Vec::new();
    let mut ptrs: Vec<u32> = Vec::new();
    crate::frame::for_each_unit(|u| {
        if ptrs.len() >= EXTENDED_LIMIT {
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
        &ids[..ids.len().min(200)]
    ));
    Some((ids, ptrs))
}

/// Called from the draw pass each frame (like `idle_workers::poll`). Consumes the
/// key presses queued by the keyboard hook and makes the next subgroup active:
/// rotate the client selection table so the subgroup is first, then drive the
/// engine's own card pipeline at it (portrait + buttonset override + rebuild).
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
        if crate::config::env_flag("CM_PLUS_TAB_DEBUG") {
            crate::log::log("[cosmo-plus] tab cycle debug: press with no readable selection");
        }
        return;
    };

    // Feed the press through the pure SC2 subgroup model. Only the
    // cycle-worthy runs (units with abilities, plus the always-cycle list)
    // take part: Tab visits exactly those, SC2-style.
    let identity = selection_identity(&ptrs);
    let runs = subgroup_runs(&ids);
    let keys: Vec<u32> = runs
        .iter()
        .filter(|r| crate::abilities::cycle_worthy(r.unit_id))
        .filter_map(|r| ptrs.get(r.start).copied())
        .filter(|&k| k != 0)
        .collect();
    let action = {
        let mut guard = MODEL.lock().unwrap();
        let model = guard.get_or_insert_with(|| SubgroupModel::new(identity, keys.clone()));
        model.press(identity, keys.clone(), dir)
    };

    let (active, groups, active_key): (u32, u32, u32) = match action {
        SubgroupAction::Inert => {
            if crate::config::env_flag("CM_PLUS_TAB_DEBUG") {
                crate::log::log(
                    "[cosmo-plus] tab cycle debug: no worthy subgroups (abilities only), Tab inert",
                );
            }
            return;
        }
        SubgroupAction::Reset { active_key, groups } => {
            if crate::config::env_flag("CM_PLUS_TAB_DEBUG") {
                crate::log::log(&format!(
                    "[cosmo-plus] tab cycle debug: fresh selection (first subgroup key 0x{active_key:08X})"
                ));
            }
            (0, groups, active_key)
        }
        SubgroupAction::Cycle {
            active_key,
            index,
            groups,
        } => (index, groups, active_key),
    };

    // The active subgroup's run in the CURRENT (possibly engine-reordered)
    // array, resolved by its unique first-unit token (duplicate unit types
    // may form several runs); make it first and aim the engine's card at it.
    let Some(run) = runs
        .iter()
        .find(|r| ptrs.get(r.start).copied() == Some(active_key))
        .cloned()
    else {
        return;
    };
    let active_id = run.unit_id;
    let rep = if selftest { 0 } else { apply_subgroup(&run, &ptrs) };

    {
        let mut d = DISPLAY.lock().unwrap();
        d.active = active;
        d.groups = groups;
        d.active_id = active_id;
        d.active_ptr = rep;
    }
    SHOW_UNTIL_FRAME.store(frame.saturating_add(INDICATOR_FRAMES), Ordering::SeqCst);
    crate::log::log(&format!(
        "[cosmo-plus] tab cycle: active subgroup {}/{} (id {}, selection[0] now {})",
        active + 1,
        groups,
        active_id,
        ptrs.get(run.start).copied().unwrap_or(0),
    ));
}

/// Rotate the subgroup run to the front of the client table, then drive the
/// engine's card at the run's representative unit. Returns that unit pointer.
unsafe fn apply_subgroup(run: &SubgroupRun, ptrs: &[u32]) -> u32 {
    if run.start != 0 {
        rotate_selection(run.start, run.len, ptrs.len());
    }
    let rep = representative(run, ptrs);
    write_card(rep);
    rep
}

/// The engine's card builder (`get_selected_unit`) skips units still under
/// construction, so the representative mirrors that: the run's first completed
/// unit, falling back to the run's first unit.
unsafe fn representative(run: &SubgroupRun, ptrs: &[u32]) -> u32 {
    let end = (run.start + run.len).min(ptrs.len());
    for i in run.start..end {
        let unit = ptrs[i] as *mut Unit;
        if (*unit).remaining_build_time == 0 {
            return ptrs[i];
        }
    }
    ptrs[run.start.min(ptrs.len().saturating_sub(1))]
}

/// Aim the portrait and the card at the active subgroup's unit: the engine's
/// per-frame `update_current_buttonset` binds `g_current_buttonset_id` (our
/// override) while the first completed unit of the table (our representative)
/// supplies the card's per-unit state, so the subgroup's spells show and
/// dispatch exactly like SC2.
unsafe fn write_card(rep: u32) {
    let unit = rep as *mut Unit;
    std::ptr::write(G_ACTIVE_PORTRAIT_UNIT as *mut u32, rep);
    std::ptr::write(G_CURRENT_BUTTONSET_ID as *mut u16, (*unit).buttons);
    if CARD_READY.load(Ordering::SeqCst) {
        core::arch::asm!(
            "pushad",
            "call dword ptr [{func}]",
            "popad",
            func = sym UPDATE_CURRENT_BUTTONSET_ADDR,
            options(nostack),
        );
    }
}

/// Reset the per-game state (called when a new game starts).
pub(crate) fn reset() {
    *MODEL.lock().unwrap() = None;
    *DISPLAY.lock().unwrap() = Display {
        active: 0,
        groups: 0,
        active_id: 0,
        active_ptr: 0,
    };
    QUEUED.store(0, Ordering::SeqCst);
    SHOW_UNTIL_FRAME.store(0, Ordering::SeqCst);
}

/// Called from the draw pass every frame (after `poll`): the per-frame view
/// that keeps the engine aimed at the active subgroup. On a fresh selection
/// (identity change) it resets to the first subgroup — SC2 shows the first
/// subgroup's card the moment a mixed selection is made, before any Tab. While
/// the cycle is active it re-asserts the rotation and card override whenever
/// the engine re-copied its canonical selection order or re-derived the
/// GroupMixed card (its own selection-change events).
pub(crate) unsafe fn apply_display() {
    if crate::frame::frame_count() == 0 {
        return;
    }
    let Some((ids, ptrs)) = read_selection() else {
        deactivate();
        return;
    };
    let identity = selection_identity(&ptrs);
    let runs = subgroup_runs(&ids);
    let keys: Vec<u32> = runs
        .iter()
        .filter(|r| crate::abilities::cycle_worthy(r.unit_id))
        .filter_map(|r| ptrs.get(r.start).copied())
        .filter(|&k| k != 0)
        .collect();
    let action = {
        let mut guard = MODEL.lock().unwrap();
        let model = guard.get_or_insert_with(|| SubgroupModel::new(identity, keys.clone()));
        model.ensure(identity, keys.clone())
    };
    let active_key = match action {
        SubgroupAction::Inert => {
            deactivate();
            return;
        }
        SubgroupAction::Reset { active_key, .. } => active_key,
        SubgroupAction::Cycle { active_key, .. } => active_key,
    };

    let Some(run) = runs
        .iter()
        .find(|r| ptrs.get(r.start).copied() == Some(active_key))
        .cloned()
    else {
        deactivate();
        return;
    };
    let active_id = run.unit_id;
    let rep = representative(&run, &ptrs);

    // Re-assert only when the engine clobbered our state: the run moved
    // (canonical re-copy), the portrait was re-derived, or the buttonset id
    // was reset to GroupMixed. Otherwise the card already shows the subgroup
    // and a per-frame rebuild would only flicker.
    let portrait = std::ptr::read(G_ACTIVE_PORTRAIT_UNIT as *const u32);
    let card = std::ptr::read(G_CURRENT_BUTTONSET_ID as *const u16);
    let ok = run.start == 0
        && portrait == rep
        && card == (*(rep as *mut Unit)).buttons;
    if !ok {
        apply_subgroup(&run, &ptrs);
    }

    {
        let mut d = DISPLAY.lock().unwrap();
        d.active = if runs.is_empty() {
            0
        } else {
            runs.iter()
                .position(|r| r.unit_id == active_id)
                .unwrap_or(0) as u32
        };
        d.groups = runs.len() as u32;
        d.active_id = active_id;
        d.active_ptr = rep;
    }
}

/// Leave the active-cycle state: restore the engine's own card (single-unit or
/// GroupMixed) once, then clear the display. No-op while already inactive.
unsafe fn deactivate() {
    let mut d = DISPLAY.lock().unwrap();
    if d.groups >= 2 {
        d.active = 0;
        d.groups = 0;
        d.active_id = 0;
        d.active_ptr = 0;
        if CARD_READY.load(Ordering::SeqCst) {
            core::arch::asm!(
                "pushad",
                "call dword ptr [{func}]",
                "popad",
                func = sym UPDATE_BUTTONSET_ADDR,
                options(nostack),
            );
        }
    }
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

/// Indicator state for the overlay: `Some((active, total))` while the Tab indicator
/// should be visible.
pub(crate) fn indicator(frame: u32) -> Option<(u32, u32)> {
    if frame == 0 || frame > SHOW_UNTIL_FRAME.load(Ordering::SeqCst) {
        return None;
    }
    let d = DISPLAY.lock().unwrap();
    if d.groups < 2 {
        return None;
    }
    Some((d.active + 1, d.groups))
}

/// The active subgroup's unit id (0 = none), for the indicator's unit name.
pub(crate) fn active_unit_id() -> u16 {
    DISPLAY.lock().unwrap().active_id
}
