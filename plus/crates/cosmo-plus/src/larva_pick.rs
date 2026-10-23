//! "Pick N larva" hotkey.
//!
//! Press the larva key (default F12, unbound in the game and the mod), type 1–2
//! digits, confirm (Enter or the 2 s timeout) → exactly that many larva of the
//! currently selected hatcheries are selected, clamped to the engine's 12-unit
//! selection cap. An empty entry at timeout selects all larva (the mod's Select
//! Larva button behaviour).
//!
//! While the entry window is open, the digit keys are **consumed** by a thread
//! keyboard hook (installed after gptp's, so it runs first) — without this, typing
//! a count would also trigger the game's control groups (jumping the camera). The
//! hook is inert outside the entry window.
//!
//! The selection itself happens in the draw pass (the hook only records the
//! confirmed count) through the engine's own selection routine, so the select
//! command is network-synced and rollback-safe. Larva are identified the same way
//! the mod's Select Larva button does: `connected_unit` (Unit+0x80) points at the
//! hatchery, and the unit id is `ZergLarva` (35) or `ZergOcsalLarva` (353).

use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, Ordering};
use std::sync::Mutex;

use bw_dat::structs::Unit;

use cosmo_plus_core::hotkeys::{parse_key, DEFAULT_LARVA_KEY};
use cosmo_plus_core::larva::{Entry, ENTRY_TIMEOUT_MS};

/// `g_client_selection_group` — the client's current selection, `UnitsSel` of 12
/// `Unit*` in place (ccmu.h: `new_client_selection_group // 00597208`).
const CLIENT_SELECTION: usize = 0x00597208;
/// `g_client_selection_count` (u8).
const CLIENT_SELECTION_COUNT: usize = 0x0059723D;
/// `Unit.connected_unit` — larva store a reference to the hatchery that made them
/// (`BW/structures/UnitLayout.h`: `/*0x080*/ Unit* connected_unit`).
const UNIT_CONNECTED_UNIT: usize = 0x080;
/// Larva unit ids (`BW/enumerations/UnitId.h`).
const ID_ZERG_LARVA: u16 = 35;
const ID_ZERG_OCSAL_LARVA: u16 = 353;

/// The resolved larva key VK (0 = disabled), set once at install.
static KEY_VK: AtomicU16 = AtomicU16::new(0);
/// The entry window length in ms (`CM_PLUS_LARVA_ENTRY_MS`, default 2000).
static ENTRY_MS: AtomicU32 = AtomicU32::new(ENTRY_TIMEOUT_MS as u32);
/// Whether the count-entry window is open.
static ACTIVE: AtomicBool = AtomicBool::new(false);
/// Tick-count deadline of the entry window (GetTickCount).
static DEADLINE: AtomicU32 = AtomicU32::new(0);
/// Set once the entry is confirmed (Enter) or timed out; the draw pass executes.
static CONFIRMED: AtomicBool = AtomicBool::new(false);
/// The digit buffer.
static ENTRY: Mutex<Entry> = Mutex::new(Entry {
    value: 0,
    digits: 0,
});

/// The keyboard hook handle, kept for the process lifetime.
static HOOK: Mutex<Option<isize>> = Mutex::new(None);

/// Resolve and log the hotkey, and install the keyboard hook (one-time, at load).
pub(crate) fn install() {
    if let Some(v) = crate::config::env("CM_PLUS_LARVA_ENTRY_MS") {
        if let Ok(ms) = v.trim().parse::<u32>() {
            let ms = ms.clamp(500, 10000);
            ENTRY_MS.store(ms, Ordering::SeqCst);
            crate::log::log(&format!("[cosmo-plus] larva pick: entry window {ms} ms"));
        }
    }
    let name = crate::config::env("CM_PLUS_LARVA_KEY")
        .unwrap_or_else(|| DEFAULT_LARVA_KEY.to_string());
    match parse_key(&name) {
        Some(vk) => {
            KEY_VK.store(vk, Ordering::SeqCst);
            crate::log::log(&format!(
                "[cosmo-plus] larva pick: key {name} (VK 0x{vk:02X}); press it, type a \
                 count (1-99, clamped to 12), Enter or wait to select"
            ));
        }
        None => crate::log::log(&format!(
            "[cosmo-plus] larva pick: unknown CM_PLUS_LARVA_KEY={name:?}; disabled"
        )),
    }
    if KEY_VK.load(Ordering::SeqCst) != 0 {
        install_hook();
    }
}

use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowsHookExW, WH_KEYBOARD};

fn install_hook() {
    unsafe {
        let h = SetWindowsHookExW(WH_KEYBOARD, Some(keyboard_proc), std::ptr::null_mut(), GetCurrentThreadId());
        if h.is_null() {
            crate::log::log("[cosmo-plus] larva pick: SetWindowsHookEx failed; \
                             count entry disabled (the key will do nothing)");
            return;
        }
        crate::log::log("[cosmo-plus] larva pick: keyboard hook installed (digits are \
                         consumed while entering a count)");
        *HOOK.lock().unwrap() = Some(h as isize);
    }
}

/// WH_KEYBOARD hook: runs ahead of gptp's own hook (installed later = runs first).
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
    if unsafe { crate::frame::frame_count() } == 0 {
        return call_next(code, wparam, lparam);
    }

    let larva_vk = KEY_VK.load(Ordering::SeqCst);
    if larva_vk != 0 && key == larva_vk as u32 {
        // Toggle: open the entry window, or cancel it when it is already open.
        if ACTIVE.swap(!ACTIVE.load(Ordering::SeqCst), Ordering::SeqCst) {
            // was open -> closed
            *ENTRY.lock().unwrap() = Entry::default();
            CONFIRMED.store(false, Ordering::SeqCst);
        } else {
            // was closed -> open
            *ENTRY.lock().unwrap() = Entry::default();
            CONFIRMED.store(false, Ordering::SeqCst);
            bump_deadline();
            crate::log::log("[cosmo-plus] larva pick: entry open — type a count");
        }
        return 1; // consume the key
    }

    if !ACTIVE.load(Ordering::SeqCst) {
        return call_next(code, wparam, lparam);
    }

    match key {
        // Main-row digits and numpad digits feed the count.
        0x30..=0x39 => {
            ENTRY.lock().unwrap().push_digit(key - 0x30);
            bump_deadline();
            return 1;
        }
        0x60..=0x69 => {
            ENTRY.lock().unwrap().push_digit(key - 0x60);
            bump_deadline();
            return 1;
        }
        // Enter confirms now.
        0x0D => {
            CONFIRMED.store(true, Ordering::SeqCst);
            ACTIVE.store(false, Ordering::SeqCst);
            return 1;
        }
        // Escape cancels.
        0x1B => {
            ACTIVE.store(false, Ordering::SeqCst);
            CONFIRMED.store(false, Ordering::SeqCst);
            *ENTRY.lock().unwrap() = Entry::default();
            crate::log::log("[cosmo-plus] larva pick: cancelled");
            return 1;
        }
        _ => call_next(code, wparam, lparam),
    }
}

fn bump_deadline() {
    DEADLINE.store(
        now_ticks().wrapping_add(ENTRY_MS.load(Ordering::SeqCst)),
        Ordering::SeqCst,
    );
}

fn now_ticks() -> u32 {
    unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount() }
}

fn call_next(code: i32, wparam: usize, lparam: isize) -> isize {
    use windows_sys::Win32::UI::WindowsAndMessaging::CallNextHookEx;
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

/// Called from the draw pass each frame: fires the selection when the entry was
/// confirmed or timed out, and closes the window on timeout.
pub(crate) unsafe fn poll() {
    if !ACTIVE.load(Ordering::SeqCst) && !CONFIRMED.load(Ordering::SeqCst) {
        return;
    }
    if ACTIVE.load(Ordering::SeqCst) && now_ticks() >= DEADLINE.load(Ordering::SeqCst) {
        // Timeout: fire with whatever is buffered (empty = select all).
        ACTIVE.store(false, Ordering::SeqCst);
        CONFIRMED.store(true, Ordering::SeqCst);
    }
    if !CONFIRMED.swap(false, Ordering::SeqCst) {
        return;
    }
    ACTIVE.store(false, Ordering::SeqCst);
    let entry = { *ENTRY.lock().unwrap() };
    *ENTRY.lock().unwrap() = Entry::default();
    let want = entry.confirmed(); // None = select all

    // The hatcheries are the client's current selection.
    let mut hatcheries: Vec<*mut Unit> = Vec::with_capacity(12);
    let sel = CLIENT_SELECTION as *const u32;
    let count = core::ptr::read(CLIENT_SELECTION_COUNT as *const u8) as usize;
    for i in 0..count.min(12) {
        let p = core::ptr::read(sel.add(i)) as *mut Unit;
        if !p.is_null() {
            hatcheries.push(p);
        }
    }
    if hatcheries.is_empty() {
        crate::log::log("[cosmo-plus] larva pick: nothing selected (select hatcheries first)");
        return;
    }

    // Gather larva: connected_unit points at one of the selected hatcheries and the
    // unit id is a larva type; hidden larva are skipped (same rule as the mod).
    let mut list: [*mut Unit; 12] = [core::ptr::null_mut(); 12];
    let mut n = 0usize;
    crate::frame::for_each_unit(|u| {
        if n >= 12 {
            return;
        }
        let id = (*u).unit_id;
        if id != ID_ZERG_LARVA && id != ID_ZERG_OCSAL_LARVA {
            return;
        }
        let connected = core::ptr::read((u as usize + UNIT_CONNECTED_UNIT) as *const *mut Unit);
        if connected.is_null() || !hatcheries.contains(&connected) {
            return;
        }
        let sprite = (*u).flingy.sprite;
        if sprite.is_null() || (*sprite).visibility_mask == 0 {
            return;
        }
        list[n] = u;
        n += 1;
    });

    if n == 0 {
        crate::log::log("[cosmo-plus] larva pick: selected hatcheries have no larva");
        return;
    }
    let take = match want {
        Some(w) => (w as usize).min(n),
        None => n,
    };
    crate::log::log(&format!(
        "[cosmo-plus] larva pick: selecting {take} of {n} larva (requested {})",
        want.map_or("all".to_string(), |w| w.to_string())
    ));
    crate::select::select_units_if_alt_not_held(list.as_ptr(), take as u32, 1, 1);
}

/// Called from the draw pass: show the count-entry feedback while the window is open.
pub(crate) unsafe fn draw() {
    if !ACTIVE.load(Ordering::SeqCst) {
        return;
    }
    let entry = *ENTRY.lock().unwrap();
    let shown = if entry.digits == 0 {
        "_".to_string()
    } else {
        entry.value.to_string()
    };
    let line = format!("count: {shown}  (Enter = select, Esc = cancel)");
    crate::overlay::draw_text_lines("Larva", &[line]);
}
