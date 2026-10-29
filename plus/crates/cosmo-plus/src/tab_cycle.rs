//! SC2-style Tab subgroup cycling ("Next Subgroup" / "Previous Subgroup").
//!
//! How StarCraft II does it (the reference for this feature):
//!   * a mixed selection is split into **subgroups**, one per distinct unit type, in
//!     the order the types appear in the selection (production buildings and units
//!     behave the same — see Blizzard's control-group guide);
//!   * **Tab** cycles the *active subgroup* forward, **Shift+Tab** backward;
//!   * only the active subgroup's buttons are shown, so its spells/abilities win the
//!     hotkeys, while plain orders (move/attack/stop) still go to every selected unit;
//!   * a fresh selection resets to the first subgroup; a subgroup stays active while
//!     it has any members;
//!   * burrowed/unburrowed units are separate subgroups (SC2 4.9.1).
//!
//! Cosmonarchy Plus adds the user's caveat: Tab visits ONLY the subgroups whose
//! card carries at least one ability (spell/morph/research/…) — spellcasters and
//! special-ability units/buildings. Pure production buildings and attack-only
//! units are not Tab stops. This matches SC2's own principle that units with
//! abilities rank higher in subgroup priority (SC2 3.9.0 "War Bots now have
//! lower subgroup priority than units that have abilities").
//!
//! Engine mechanics:
//!   * the pure SC2 state machine lives in `cosmo_plus_core::subgroups::SubgroupModel`,
//!     fed with run keys computed from the **canonical selection**
//!     (`ccmu_extender.new_active_selection`, reached through the exe's pointer
//!     global at 0x006284B8 — synced through select commands, NEVER rotated by us).
//!     Keying on (derived id, ordinal) keeps the model stable across array
//!     rotations, canonical re-copies and head-of-run deaths — the two earlier
//!     schemes failed here (id keys bounced between duplicate-type runs; pointer
//!     tokens died with the run's first unit);
//!   * `get_selected_unit()` (buttonsets.cpp) = the **first completed unit in
//!     `g_client_selection_group`** — the client selection table. The active
//!     subgroup is made first by **rotating that table** (viewer-local: the table
//!     is not part of the synced game state) and the fork's per-frame
//!     `update_current_buttonset` (0x004599A0, jump-patched) then binds the
//!     subgroup's full card from `g_current_buttonset_id` (our override);
//!   * the portrait/wireframe (`g_active_portrait_unit`) follows the same unit.
//!
//! The engine re-copies its canonical selection order and re-derives GroupMixed on
//! its own events, so `apply_display` re-asserts the rotation + override every
//! frame while the cycle is active. A **detour at the engine's selection entry
//! point** (`select_units_if_alt_not_held`, 0x0046FA00 — validated unpatched at
//! install) latches a selection-event flag, which resets the cycle to the first
//! subgroup exactly like SC2 (box/click/recall and the plugin's own idle/larva
//! picks all count as fresh selections).
//!
//! The Tab key is **swallowed** by a thread keyboard hook (installed after gptp's,
//! so it runs first) while in a game — stock Brood War binds Tab to hiding the
//! terrain, which this replaces. Outside chat and modal dialogs the key is never
//! swallowed; Ctrl/Alt+Tab passes through to the game. Tab stays inert when the
//! selection has fewer than two worthy subgroups, and when the fork's card
//! override is not available (unpatched build) the whole cycle disarms loudly.
//!
//! Cosmonarchy extends the selection to `SelectionLimit` = 240 units and moves the
//! selection tables out of the exe into `ccmu_extender` (inside `gptp.qdp`).
//! The client table's runtime address is recovered from the immediates the
//! extender patches into `update_selected_unit_data` (0x004C38BE / 0x004C38F4);
//! the canonical table's address is the exe pointer at 0x006284B8 rewritten by
//! the extender.
//!
//! Keys: `CM_PLUS_TAB_KEY` (default `TAB`), `CM_PLUS_TAB_PREV_KEY` (default empty =
//! Shift+Tab), kill switch `CM_PLUS_NO_TAB_CYCLE=1`.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU16, AtomicU32, AtomicUsize, Ordering};
use std::sync::Mutex;

use once_cell::sync::Lazy;

use bw_dat::structs::Unit;

use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_CONTROL, VK_MENU, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowsHookExW, WH_KEYBOARD};

use cosmo_plus_core::hotkeys::{parse_key, DEFAULT_TAB_KEY};
use cosmo_plus_core::subgroups::{
    base_id, derived_id, run_keys, subgroup_runs, SubgroupAction, SubgroupModel,
};

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
/// `select_units_if_alt_not_held` (0x0046FA00, `BW/bwhelper.cpp`) — the engine's
/// selection entry point (click/box/recall and the plugin's own selections).
/// Detoured to latch the selection-event flag; verified unpatched (55 8B EC) at
/// install, otherwise the identity-hash fallback stands and the boot report says so.
const ADDR_SELECT_UNITS: usize = 0x0046FA00;

static UPDATE_CURRENT_BUTTONSET_ADDR: u32 = ADDR_UPDATE_CURRENT_BUTTONSET as u32;
static UPDATE_BUTTONSET_ADDR: u32 = ADDR_UPDATE_BUTTONSET as u32;
static SELECT_ORIGINAL_ADDR: AtomicU32 = AtomicU32::new(0);
/// Whether the fork's jump patch is present at `update_current_buttonset`
/// (checked at install): only then is calling the address register-safe. Without
/// it the whole cycle is inert (no rotation, no override) — see `install`.
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

/// Immediates the extender rewrites with the runtime address of the CANONICAL
/// selection array `ccmu_extender.new_active_selection` and its one-past-end
/// slot, inside `is_selected_unit_group_enabled` (0x0049A2C0):
/// `mov edx, imm32` (imm @ 0x0049A2C8) / `cmp edx, imm32` (imm @ 0x0049A305).
/// Verified against the shipped exe (vanilla immediates 0x006284B8 /
/// 0x006284E8 = the stock 12-slot canonical array — 0x006284B8 is the ARRAY
/// itself, not a pointer to it, which is why the cycle must never read it as
/// a pointer). A valid pair is exactly `EXTENDED_LIMIT * 4` bytes apart.
const ANCHOR_CANON_BASE: usize = 0x0049A2C8;
const ANCHOR_CANON_END: usize = 0x0049A305;
/// The vanilla 12-slot canonical array (fallback when the extender anchors do
/// not validate — the extender may leave it untouched on some builds).
const VANILLA_CANON: usize = 0x006284B8;

/// The 1.16.1 client selection table, used only when the extender anchors do not validate.
const VANILLA_SELECTION: usize = 0x00597208;
/// Cosmonarchy's extended selection limit (`SelectionLimit`).
const EXTENDED_LIMIT: usize = 240;
/// Stock 1.16.1 selection limit.
const VANILLA_LIMIT: usize = 12;

/// How many frames the subgroup indicator stays visible after a Tab press (~1.5 s).
const INDICATOR_FRAMES: u32 = 36;

/// Resolved selection table bases and capacities (set once, at first use).
static SEL_BASE: AtomicUsize = AtomicUsize::new(0);
static SEL_LIMIT: AtomicUsize = AtomicUsize::new(0);
static RESOLVED: AtomicBool = AtomicBool::new(false);

/// Resolved canonical table base (set lazily, re-validated on failure).
static CANON_BASE: AtomicUsize = AtomicUsize::new(0);
static CANON_LIMIT: AtomicUsize = AtomicUsize::new(0);

/// Forward cycle key (default Tab) and optional separate backward key (0 = none).
static FWD_VK: AtomicU16 = AtomicU16::new(0);
static PREV_VK: AtomicU16 = AtomicU16::new(0);

/// Whether the selection-event detour is armed (boot report).
static SELECT_DETOUR_ARMED: AtomicBool = AtomicBool::new(false);
/// Latched by the selection-entry detour; consumed by the draw pass.
static SELECTION_EVENT: AtomicBool = AtomicBool::new(false);

/// The keyboard hook handle, kept for the process lifetime.
static HOOK: Mutex<Option<isize>> = Mutex::new(None);
/// Key presses swallowed by the hook, queued for the draw pass: positive =
/// forward presses, negative = backward presses (saturating, so rapid presses
/// never collapse to one).
static QUEUED: AtomicI32 = AtomicI32::new(0);

/// Game frame until which the indicator stays visible.
static SHOW_UNTIL_FRAME: AtomicU32 = AtomicU32::new(0);

/// One canonical run's cached shape: key, derived id, and the pointer set of its
/// members (in canonical order). Pointer sets survive array rotations and
/// canonical re-copies; membership is checked by pointer identity.
struct CanonRun {
    key: u32,
    derived_id: u16,
    ptrs: Vec<u32>,
}

/// Display state for the overlay/portrait aim.
struct Display {
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

/// Resolved status for the boot report.
pub(crate) fn status() -> String {
    let vk = FWD_VK.load(Ordering::SeqCst);
    let mut parts: Vec<String> = Vec::new();
    if vk == 0 {
        parts.push("disabled (no valid key)".to_string());
    } else {
        parts.push(format!("armed (VK 0x{vk:02X})"));
    }
    if !CARD_READY.load(Ordering::SeqCst) {
        parts.push("card override missing (inert)".to_string());
    }
    if !SELECT_DETOUR_ARMED.load(Ordering::SeqCst) {
        parts.push("selection detour missing (hash fallback)".to_string());
    }
    if CANON_BASE.load(Ordering::SeqCst) == 0 {
        parts.push("canonical selection unresolved (cycle inert until it resolves)".to_string());
    }
    parts.join(", ")
}

/// The resolved forward/backward VKs (0 = none), for the boot-time
/// key-collision check.
pub(crate) fn fwd_vk() -> u16 {
    FWD_VK.load(Ordering::SeqCst)
}
pub(crate) fn prev_vk() -> u16 {
    PREV_VK.load(Ordering::SeqCst)
}

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
    install_selection_detour();
    rearm_announce();
    // Resolve the selection table now: `gptp.qdp` (the mod) loads before this plugin
    // and has already rewritten the anchor immediates, so the extended table resolves
    // during the menu boot and is logged for the headless probe.
    unsafe {
        resolve_selection();
        resolve_canonical();
        // The card override needs the fork's jump-patched update_current_buttonset:
        // calling the vanilla function with our register state would be unsafe.
        let first = std::ptr::read(ADDR_UPDATE_CURRENT_BUTTONSET as *const u8);
        CARD_READY.store(first == 0xE9, Ordering::SeqCst);
        crate::log::log(&format!(
            "[cosmo-plus] tab cycle: card override {} (update_current_buttonset first byte 0x{first:02X})",
            if CARD_READY.load(Ordering::SeqCst) { "armed" } else { "disabled" }
        ));
        if !CARD_READY.load(Ordering::SeqCst) {
            crate::log::log(
                "[cosmo-plus] tab cycle: unpatched build — Tab stays inert (no rotation, \
                 no card override) so the cycle never shows a broken half-state",
            );
        }
    }
    crate::state::hook("selection:tab-cycle-detour", ADDR_SELECT_UNITS, 0, SELECT_ORIGINAL_ADDR.load(Ordering::SeqCst) as usize);
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

/// Detour the engine's selection entry point so every fresh selection (click,
/// box, recall, our own idle/larva picks, the mod's remix selections) latches a
/// flag that resets the cycle — SC2's fresh-selection semantics. The detour
/// replays the replaced prologue (`push ebp; mov ebp, esp`), latches the flag
/// without clobbering registers, and tail-jumps to the original + 5. Installed
/// only when the site is still the unpatched prologue; otherwise the hash
/// fallback stands and the boot report says so.
fn install_selection_detour() {
    unsafe {
        let first5: [u8; 5] = [
            std::ptr::read(ADDR_SELECT_UNITS as *const u8),
            std::ptr::read((ADDR_SELECT_UNITS + 1) as *const u8),
            std::ptr::read((ADDR_SELECT_UNITS + 2) as *const u8),
            std::ptr::read((ADDR_SELECT_UNITS + 3) as *const u8),
            std::ptr::read((ADDR_SELECT_UNITS + 4) as *const u8),
        ];
        if first5 != [0x55, 0x8B, 0xEC, 0x83, 0xFF] {
            crate::log::log(&format!(
                "[cosmo-plus] tab cycle: selection entry point at 0x{ADDR_SELECT_UNITS:08X} \
                 is {first5:02X?}; detour skipped (another plugin patched it?) — \
                 fresh-selection reset falls back to the identity hash"
            ));
            return;
        }
        let orig = (ADDR_SELECT_UNITS + 5) as u32;
        SELECT_ORIGINAL_ADDR.store(orig, Ordering::SeqCst);
        let trampoline = selection_event_trampoline as *const () as usize as u32;
        // E9 rel32 (site+5)
        let rel = trampoline.wrapping_sub((ADDR_SELECT_UNITS + 5) as u32);
        let api = crate::plus::api();
        let patch: [u8; 5] = [0xE9, rel as u8, (rel >> 8) as u8, (rel >> 16) as u8, (rel >> 24) as u8];
        let r = (api.write_exe_memory)(ADDR_SELECT_UNITS, patch.as_ptr(), 5);
        SELECT_DETOUR_ARMED.store(r != 0, Ordering::SeqCst);
        crate::log::log(&format!(
            "[cosmo-plus] tab cycle: selection detour installed (0x{ADDR_SELECT_UNITS:08X} -> \
             0x{trampoline:08X}, patch={r}); every fresh selection resets the cycle"
        ));
    }
}

/// 5-byte-detour trampoline: replay the replaced prologue, latch the selection
/// event, jump back into the original function.
#[unsafe(naked)]
unsafe extern "C" fn selection_event_trampoline() {
    core::arch::naked_asm!(
        "push ebp",
        "mov ebp, esp",
        "pushad",
        "mov eax, offset {flag}",
        "mov byte ptr [eax], 1",
        "popad",
        "jmp dword ptr [{orig}]",
        flag = sym SELECTION_EVENT,
        orig = sym SELECT_ORIGINAL_ADDR,
    );
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

/// Any of Ctrl / Alt / Shift currently held.
unsafe fn modifier_down() -> bool {
    [VK_SHIFT, VK_CONTROL, VK_MENU].iter().any(|&vk| {
        (GetAsyncKeyState(vk as i32) as u16 & 0x8000) != 0
    })
}

/// WH_KEYBOARD hook: runs ahead of gptp's own hook. Swallows Tab (and the
/// configured backward key) while a game is running, so the stock "hide terrain"
/// binding never fires — the press is queued for the draw pass instead. Chat,
/// modal dialogs, Ctrl/Alt-modified presses and auto-repeats are never
/// interfered with (auto-repeats are swallowed but not queued).
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
    if !is_fwd && (prev_vk == 0 || key != prev_vk as u32) {
        return call_next(code, wparam, lparam);
    }
    // Modified presses stay with the game: Ctrl+Tab / Alt+Tab are not ours.
    let shift = (GetAsyncKeyState(VK_SHIFT as i32) as u16 & 0x8000) != 0;
    let ctrl = (GetAsyncKeyState(VK_CONTROL as i32) as u16 & 0x8000) != 0;
    let alt = (GetAsyncKeyState(VK_MENU as i32) as u16 & 0x8000) != 0;
    if ctrl || alt {
        return call_next(code, wparam, lparam);
    }
    let is_prev = !is_fwd || shift; // backward key wins; Shift+forward goes back
    // Bit 30 = the key was already down: auto-repeat. Swallow it, do not queue.
    let repeat = lparam as u32 & (1 << 30) != 0;
    if !repeat {
        let _ = QUEUED.try_update(Ordering::SeqCst, Ordering::SeqCst, |q: i32| {
            let nq = if is_prev { q.saturating_sub(1) } else { q.saturating_add(1) };
            Some(nq.clamp(-4, 4))
        });
    }
    if crate::config::env_flag("CM_PLUS_TAB_DEBUG") {
        crate::log::log("[cosmo-plus] tab debug: hook queued the press (swallowed)");
    }
    1 // swallow: no terrain toggle, ever
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

/// Resolve the canonical selection table (the extender's
/// `new_active_selection`), in build-robust order:
///
/// 1. The extender rewrites two immediates inside
///    `is_selected_unit_group_enabled` with the array's runtime address and
///    its one-past-end slot — the same anchor pattern as the client table,
///    verified on the harness build. On SOME release builds those sites are
///    not rewritten (garbage immediates), so this alone is not enough.
/// 2. Struct fallback: `ccmu_extender` lays the three selection arrays out
///    contiguously — `new_client_selection_group[240]`,
///    `new_local_selection_group[240]`, `new_active_selection[240]` (see
///    `hooks/limits/ccmu.h`) — so the canonical array sits exactly
///    `2 * SelectionLimit * 4` bytes after the (reliably resolved) client
///    table. Validated by the readable range + per-slot pointer checks on
///    read (see `read_array`).
/// 3. Vanilla 12-slot array as the last resort (logged loudly).
unsafe fn resolve_canonical() -> bool {
    let cached = CANON_BASE.load(Ordering::SeqCst);
    if cached != 0 && crate::overlay::readable_ptr(cached as *const u8, EXTENDED_LIMIT * 4) {
        return true;
    }
    // 1. Extender anchor pair (only when both validate exactly).
    let base = std::ptr::read(ANCHOR_CANON_BASE as *const u32) as usize;
    let end = std::ptr::read(ANCHOR_CANON_END as *const u32) as usize;
    if base != 0 && end == base + EXTENDED_LIMIT * 4 {
        CANON_BASE.store(base, Ordering::SeqCst);
        CANON_LIMIT.store(EXTENDED_LIMIT, Ordering::SeqCst);
        crate::log::log(&format!(
            "[cosmo-plus] tab cycle: canonical selection table at 0x{base:08X} \
             ({EXTENDED_LIMIT} slots, anchor 0x{ANCHOR_CANON_BASE:08X})"
        ));
        return true;
    }
    // 2. Struct-delta fallback from the (extended) client table.
    if resolve_selection() && SEL_LIMIT.load(Ordering::SeqCst) == EXTENDED_LIMIT {
        let client = SEL_BASE.load(Ordering::SeqCst);
        let delta = EXTENDED_LIMIT * 4 + EXTENDED_LIMIT * 4; // client + local
        let candidate = client + delta;
        if crate::overlay::readable_ptr(candidate as *const u8, EXTENDED_LIMIT * 4) {
            CANON_BASE.store(candidate, Ordering::SeqCst);
            CANON_LIMIT.store(EXTENDED_LIMIT, Ordering::SeqCst);
            crate::log::log(&format!(
                "[cosmo-plus] tab cycle: canonical selection anchors did not validate \
                 (base 0x{base:08X}, end 0x{end:08X}); resolved via the extender struct \
                 (client 0x{client:08X} + 0x{delta:X} = 0x{candidate:08X})"
            ));
            return true;
        }
    }
    // 3. Vanilla 12-slot array (last resort — the engine's own selection may
    //    not even be that table; the cycle then reads stale data, so this is
    //    logged loudly and the boot report flags it).
    CANON_BASE.store(VANILLA_CANON, Ordering::SeqCst);
    CANON_LIMIT.store(VANILLA_LIMIT, Ordering::SeqCst);
    static FAILED_ONCE: AtomicBool = AtomicBool::new(false);
    if !FAILED_ONCE.swap(true, Ordering::SeqCst) {
        crate::log::log(&format!(
            "[cosmo-plus] tab cycle: canonical selection UNRESOLVED (anchors 0x{base:08X}/0x{end:08X} \
             invalid, struct fallback unreadable); using the vanilla 12-slot table at \
             0x{VANILLA_CANON:08X} — Tab may not match this build's selection"
        ));
    }
    true
}

/// Read a raw selection array as (unit ids, pointers) in array order. Stops at
/// the first null slot (the engine nulls the tail on removal). Every entry is
/// pointer-validated before ANY dereference — a misresolved array (wrong
/// build layout) must degrade to an empty read, never to an access violation.
/// `limit` caps the walk.
unsafe fn read_array(base: usize, limit: usize) -> Option<(Vec<u16>, Vec<u32>)> {
    let list = base as *const *mut Unit;
    let mut ids = Vec::with_capacity(16);
    let mut ptrs = Vec::with_capacity(16);
    for i in 0..limit {
        let unit = *list.add(i);
        if unit.is_null() {
            break;
        }
        if !crate::overlay::readable_ptr(unit as *const u8, core::mem::size_of::<Unit>()) {
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

/// The canonical selection as (derived ids, raw pointers), in canonical order.
/// Returns `None` when the selection is empty or the table is not ready.
pub(crate) unsafe fn read_canonical_selection() -> Option<(Vec<u16>, Vec<u32>)> {
    if !resolve_canonical() {
        return None;
    }
    let (ids, ptrs) = read_array(
        CANON_BASE.load(Ordering::SeqCst),
        CANON_LIMIT.load(Ordering::SeqCst).max(1),
    )?;
    // Fold the burrow state into the ids (SC2: burrowed/unburrowed are
    // separate subgroups); the pointer array stays raw.
    let derived: Vec<u16> = ids
        .iter()
        .zip(&ptrs)
        .map(|(&id, &p)| derived_id(id, (*((p as *mut Unit))).order))
        .collect();
    Some((derived, ptrs))
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
        ids.push(derived_id(unit.unit_id, unit.order));
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

/// Build the canonical runs (cycle-worthy only): each run's key, derived id and
/// member pointer set. Non-worthy runs are filtered out (the caveat: Tab visits
/// spellcasters and ability carriers only).
fn canonical_runs(ids: &[u16], ptrs: &[u32]) -> Vec<CanonRun> {
    let runs = subgroup_runs(ids);
    let worthy_ids: Vec<u16> = runs
        .iter()
        .filter(|r| unsafe { crate::abilities::cycle_worthy(base_id(r.unit_id)) })
        .map(|r| r.unit_id)
        .collect();
    let keys = run_keys(&worthy_ids);
    runs.iter()
        .filter(|r| unsafe { crate::abilities::cycle_worthy(base_id(r.unit_id)) })
        .zip(keys.iter())
        .map(|(r, &key)| CanonRun {
            key,
            derived_id: r.unit_id,
            ptrs: ptrs[r.start..r.start + r.len].to_vec(),
        })
        .collect()
}

/// Consume the selection-event flag (latched by the detour) and normalise the
/// model against the current canonical selection. Returns the canonical state
/// the caller should act on.
unsafe fn normalize(selftest: bool) -> Option<(u64, Vec<u32>, Vec<CanonRun>)> {
    let picked = if selftest {
        selftest_selection()
    } else {
        read_canonical_selection()
    };
    let (ids, ptrs) = picked?;
    let identity = selection_identity(&ptrs);
    let runs = canonical_runs(&ids, &ptrs);
    let keys: Vec<u32> = runs.iter().map(|r| r.key).collect();
    let mut guard = MODEL.lock().unwrap();
    let model = guard.get_or_insert_with(|| SubgroupModel::new(identity, keys.clone()));
    if SELECTION_EVENT.swap(false, Ordering::SeqCst) {
        let _ = model.select(identity, keys.clone());
    }
    Some((identity, keys, runs))
}

/// The active run's pointer set, by key.
fn run_by_key(runs: &[CanonRun], key: u32) -> Option<&CanonRun> {
    runs.iter().find(|r| r.key == key)
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
    let dir: i32 = if queued < 0 { -1 } else { 1 };

    let selftest = crate::config::env_flag("CM_PLUS_TAB_SELFTEST");
    let Some((identity, keys, runs)) = normalize(selftest) else {
        if crate::config::env_flag("CM_PLUS_TAB_DEBUG") {
            crate::log::log("[cosmo-plus] tab cycle debug: press with no readable selection");
        }
        return;
    };

    let action = {
        let mut guard = MODEL.lock().unwrap();
        let model = guard.as_mut().expect("normalize created the model");
        model.press(identity, keys.clone(), dir)
    };

    let (active_key, groups) = match action {
        SubgroupAction::Inert => {
            if crate::config::env_flag("CM_PLUS_TAB_DEBUG") {
                // The raw selection is what the classification saw: log it so
                // a permanently-inert Tab can be told apart from a genuinely
                // ability-less selection.
                let raw = read_canonical_selection()
                    .map(|(ids, _)| {
                        ids.iter()
                            .map(|&d| format!("{}{}", base_id(d),
                                if crate::abilities::cycle_worthy(base_id(d)) { "+" } else { "-" }))
                            .collect::<Vec<_>>()
                            .join(",")
                    })
                    .unwrap_or_else(|| "<none>".to_string());
                crate::log::log(&format!(
                    "[cosmo-plus] tab cycle debug: no worthy subgroups, Tab inert; selection [{raw}] (+ = ability-bearing)"
                ));
            }
            return;
        }
        SubgroupAction::Reset { active_key, groups } => (active_key, groups),
        SubgroupAction::Cycle { active_key, groups, .. } => (active_key, groups),
    };

    let Some(run) = run_by_key(&runs, active_key) else {
        return;
    };
    let active_id = run.derived_id;
    let rep = if selftest { 0 } else { apply_subgroup(run, identity, &keys, &runs) };

    // Only when the card override is available does anything visibly change
    // (apply_subgroup is inert without it) — never show an indicator for a
    // press that did nothing.
    if CARD_READY.load(Ordering::SeqCst) {
        let mut d = DISPLAY.lock().unwrap();
        d.active = keys.iter().position(|&k| k == active_key).unwrap_or(0) as u32;
        d.groups = groups;
        d.active_id = active_id;
        d.active_ptr = rep;
        SHOW_UNTIL_FRAME.store(frame.saturating_add(INDICATOR_FRAMES), Ordering::SeqCst);
    }
    crate::log::log(&format!(
        "[cosmo-plus] tab cycle: active subgroup {}/{} (id {})",
        keys.iter().position(|&k| k == active_key).unwrap_or(0) + 1,
        groups,
        base_id(active_id),
    ));
}

/// Rotate the subgroup run to the front of the client table, then drive the
/// engine's card at the run's representative unit. Returns that unit pointer.
/// Inert (no rotation at all) when the fork's card override is unavailable.
unsafe fn apply_subgroup(
    run: &CanonRun,
    identity: u64,
    _keys: &[u32],
    _runs: &[CanonRun],
) -> u32 {
    if !CARD_READY.load(Ordering::SeqCst) {
        return run.ptrs[0];
    }
    rotate_client_to(&run.ptrs);
    let rep = representative(run);
    write_card(rep, base_id(run.derived_id));
    let _ = identity;
    rep
}

/// Rotate the engine's client selection array so the given pointer set leads.
/// The write is strictly bounded by the live client count and only includes
/// lead members actually present in the client table (the canonical table can
/// carry stale pointers beyond the engine's live count — writing them, or
/// writing more slots than the count, would corrupt the extender's struct).
/// The tail keeps every client-selected unit not in the set, in its current
/// order; the count is untouched.
unsafe fn rotate_client_to(lead: &[u32]) -> bool {
    let base = SEL_BASE.load(Ordering::SeqCst);
    if base == 0 {
        return false;
    }
    let limit = SEL_LIMIT.load(Ordering::SeqCst).max(1);
    let count = (std::ptr::read(CLIENT_SELECTION_COUNT as *const u8) as usize).min(limit);
    if count == 0 {
        return false;
    }
    let list = base as *const *mut Unit;
    let mut client: Vec<u32> = Vec::with_capacity(count);
    for i in 0..count {
        let p = *list.add(i) as u32;
        if p != 0 {
            client.push(p);
        }
    }
    let mut rotated: Vec<u32> = Vec::with_capacity(client.len());
    // Lead members present in the client table, in lead order (the canonical
    // run order the model wants first).
    for &p in lead {
        if client.contains(&p) {
            rotated.push(p);
        }
    }
    for &p in &client {
        if !rotated.contains(&p) {
            rotated.push(p);
        }
    }
    let list_mut = base as *mut *mut Unit;
    for (i, &p) in rotated.iter().enumerate() {
        *list_mut.add(i) = p as *mut Unit;
    }
    true
}

/// The engine's card builder (`get_selected_unit`) skips units still under
/// construction, so the representative mirrors that: the run's first completed
/// unit, falling back to the run's first unit.
unsafe fn representative(run: &CanonRun) -> u32 {
    for &p in &run.ptrs {
        if (*((p as *mut Unit))).remaining_build_time == 0 {
            return p;
        }
    }
    run.ptrs[0]
}

/// Aim the portrait and the card at the active subgroup's unit: the engine's
/// per-frame `update_current_buttonset` binds `g_current_buttonset_id` (our
/// override) while the first completed unit of the table (our representative)
/// supplies the card's per-unit state, so the subgroup's spells show and
/// dispatch exactly like SC2.
///
/// The buttonset id is the representative's LIVE buttonset when it is
/// completed (the fork supports per-unit buttonset overrides, e.g. infested
/// units — `ValueButtonsetOverride`); for a run with no completed unit (a run
/// of morphing hatcheries) it is the unit TYPE's id (buttonset id == unit
/// id), so the card shows the real ability card instead of the construction
/// placeholder.
unsafe fn write_card(rep: u32, fallback_buttonset_id: u16) {
    let unit = rep as *mut Unit;
    let buttonset_id = if (*unit).remaining_build_time == 0 {
        (*unit).buttons
    } else {
        fallback_buttonset_id
    };
    std::ptr::write(G_ACTIVE_PORTRAIT_UNIT as *mut u32, rep);
    std::ptr::write(G_CURRENT_BUTTONSET_ID as *mut u16, buttonset_id);
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
    SELECTION_EVENT.store(false, Ordering::SeqCst);
}

/// Called from the draw pass every frame (before `poll`): the per-frame view
/// that keeps the engine aimed at the active subgroup. Normalises the model
/// against the current canonical selection (keep the active subgroup while it
/// has members; reset to the first worthy one when it vanished or a selection
/// event fired), then re-asserts the rotation and card override whenever the
/// engine re-copied its canonical selection order or re-derived the GroupMixed
/// card.
pub(crate) unsafe fn apply_display() {
    if crate::frame::frame_count() == 0 {
        return;
    }
    let selftest = crate::config::env_flag("CM_PLUS_TAB_SELFTEST");
    let Some((identity, keys, runs)) = normalize(selftest) else {
        deactivate();
        return;
    };
    let active_key = {
        let mut guard = MODEL.lock().unwrap();
        let model = guard.as_mut().expect("normalize created the model");
        match model.ensure(identity, keys.clone()) {
            SubgroupAction::Inert => {
                deactivate();
                return;
            }
            SubgroupAction::Reset { active_key, .. } => active_key,
            SubgroupAction::Cycle { active_key, .. } => active_key,
        }
    };

    let Some(run) = run_by_key(&runs, active_key) else {
        deactivate();
        return;
    };
    let active_id = run.derived_id;
    let rep = representative(run);

    // Re-assert only when the engine clobbered our state: the run no longer
    // leads the client table, the portrait was re-derived, or the buttonset id
    // was reset. Otherwise the card already shows the subgroup and a per-frame
    // rebuild would only flicker.
    let portrait = std::ptr::read(G_ACTIVE_PORTRAIT_UNIT as *const u32);
    let card = std::ptr::read(G_CURRENT_BUTTONSET_ID as *const u16);
    let leads = client_leads_with(&run.ptrs);
    let ok = leads
        && portrait == rep
        && card == base_id(active_id);
    if !ok {
        apply_subgroup(run, identity, &keys, &runs);
    }

    {
        let mut d = DISPLAY.lock().unwrap();
        d.active = keys.iter().position(|&k| k == active_key).unwrap_or(0) as u32;
        d.groups = keys.len() as u32;
        d.active_id = active_id;
        d.active_ptr = rep;
    }
}

/// Whether the client table currently starts with the run's full pointer set
/// (set equality on the leading `len` slots, order preserved).
unsafe fn client_leads_with(lead: &[u32]) -> bool {
    let base = SEL_BASE.load(Ordering::SeqCst);
    if base == 0 {
        return false;
    }
    let list = base as *const *mut Unit;
    for (i, &p) in lead.iter().enumerate() {
        if *list.add(i) as u32 != p {
            return false;
        }
    }
    true
}

/// Leave the active-cycle state: restore the engine's own card (single-unit or
/// GroupMixed) and its own portrait derivation once, then clear the display.
/// No-op while already inactive.
unsafe fn deactivate() {
    let mut d = DISPLAY.lock().unwrap();
    if d.groups >= 2 {
        d.active = 0;
        d.groups = 0;
        d.active_id = 0;
        d.active_ptr = 0;
        if CARD_READY.load(Ordering::SeqCst) {
            // Let the engine re-derive both the card AND the portrait from the
            // canonical selection (the portrait must not linger on the wrong
            // unit). The buttonset-id write below is transient: the vanilla
            // `update_buttonset` re-derives the id inside the call, so the
            // placeholder value (0 = the generic card) only survives between
            // this write and the call.
            std::ptr::write(G_CURRENT_BUTTONSET_ID as *mut u16, 0);
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

/// Indicator state for the overlay: `Some((active, total))` while the Tab indicator
/// should be visible. Counts are the cycle-worthy groups only, so the indicator
/// matches the cycle the player actually tabs through.
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

/// The active subgroup's base unit id (0 = none), for the indicator's unit name.
pub(crate) fn active_unit_id() -> u16 {
    base_id(DISPLAY.lock().unwrap().active_id)
}

/// The units an ability-class order should be narrowed to (the tab cycle's
/// SC2 subgroup parity rule). Deterministic across peers: computed ONLY from
/// the canonical selection (synced, never rotated by us) — never from the
/// viewer-local client table or model state, because the command hook runs on
/// both peers with the same command bytes and both must narrow identically.
///
/// Returns `None` (broadcast — the normal engine path) when:
///   * the tab cycle is disabled or unarmed, or `CM_PLUS_NO_TAB_NARROW` is set
///     (bisection kill switch);
///   * the order is generic (move/attack/stop/harvest/build/train — those keep
///     broadcasting, exactly like SC2's plain orders);
///   * the selection is single-typed (the first worthy run IS the whole
///     selection — narrowing would be a no-op).
///
/// Otherwise the FIRST cycle-worthy canonical run whose card carries the
/// order — the subgroup the player's Tab UI shows for it. Shared orders
/// (burrow on zerglings AND lurkers) narrow to the first such run. When NO
/// worthy run's card carries the order, the run-filter falls back to the
/// first worthy run: for card-carrying abilities that case cannot arise
/// (any unit whose card carries an ability-class order is cycle-worthy), and
/// the fallback keeps the ALWAYS_CYCLE units (lurkers, whose card has no
/// manual burrow button) narrowing correctly — the conservative SC2-like
/// default either way. Known, documented deviation: after Tab-ing to a LATER
/// subgroup the narrowing still targets the first capability-bearing run
/// (SC2 would target the tabbed subgroup, but that state is viewer-local and
/// syncing it is a future netcode feature).
pub(crate) unsafe fn narrow_targets(order: u8) -> Option<Vec<u32>> {
    if crate::config::env_flag("CM_PLUS_NO_TAB_CYCLE")
        || crate::config::env_flag("CM_PLUS_NO_TAB_NARROW")
        || FWD_VK.load(Ordering::SeqCst) == 0
    {
        return None;
    }
    if !crate::abilities::is_ability_order(order) {
        return None;
    }
    let (ids, ptrs) = read_canonical_selection()?;
    let runs = canonical_runs(&ids, &ptrs);
    // Prefer the first worthy run whose card actually carries this order
    // (deterministic: the buttonset table is the same on every peer), so a
    // templar-only spell never narrows to an archon run that would eat it.
    let first = runs
        .iter()
        .find(|r| crate::abilities::has_order(base_id(r.derived_id), order))
        .or_else(|| runs.first())?;
    if first.ptrs.len() >= ptrs.len() {
        return None; // single-typed selection: broadcast is the same thing
    }
    Some(first.ptrs.clone())
}
