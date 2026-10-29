//! "Pick N larva" hotkey.
//!
//! Press the larva key (default **Ctrl+Space**: hold Ctrl, press Space, release
//! Ctrl), type a count (2, 4, Enter = 24), confirm (Enter or the 2 s timeout) →
//! exactly that many larva of the currently selected hatcheries are selected,
//! up to the engine's extended 240-unit selection cap. An empty entry at
//! timeout selects all larva (the mod's Select Larva button behaviour). A fresh
//! chord press always starts a new entry, so a count already typed can be
//! replaced (`50` then Ctrl+Space, 3 = 3).
//!
//! While the entry window is open, the digit keys are **consumed** by a thread
//! keyboard hook (installed after gptp's, so it runs first) — without this, typing
//! a count would also trigger the game's control groups (jumping the camera). Digits
//! pressed WITH Ctrl held still reach the game (control groups win), so the
//! natural flow is: Ctrl+Space, release Ctrl, type the count. The hook is inert
//! outside the entry window, in chat and in modal dialogs.
//!
//! The selection itself happens in the draw pass (the hook only records the
//! confirmed count) through the engine's own selection routine, so the select
//! command is network-synced and rollback-safe. Larva are identified the same way
//! the mod's Select Larva button does: `connected_unit` (Unit+0x80) points at the
//! hatchery, and the unit id is `ZergLarva` (35) or `ZergOcsalLarva` (353).
//!
//! The binding is a full chord: `CM_PLUS_LARVA_KEY` accepts `CTRL+SPACE`,
//! `CTRL+SHIFT+<key>`, `ALT+<key>` etc. (extra held modifiers are ignored), or
//! any bare key (legacy behaviour). The default lives in the shipped
//! `cosmo_plus.conf.example`.

use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, Ordering};
use std::sync::Mutex;

use bw_dat::structs::Unit;

use cosmo_plus_core::hotkeys::{parse_binding, Binding, Modifiers, DEFAULT_LARVA_KEY};
use cosmo_plus_core::larva::{Entry, ENTRY_TIMEOUT_MS};

/// `Unit.connected_unit` — larva store a reference to the hatchery that made them
/// (`BW/structures/UnitLayout.h`: `/*0x080*/ Unit* connected_unit`).
const UNIT_CONNECTED_UNIT: usize = 0x080;
/// Larva unit ids (`BW/enumerations/UnitId.h`).
const ID_ZERG_LARVA: u16 = 35;
const ID_ZERG_OCSAL_LARVA: u16 = 353;
/// The extended selection limit (the engine's 240-unit cap; ccmu extender).
const SELECTION_LIMIT: usize = 240;
/// `g_chat_message_type` — non-null/!= 0 while the chat line is open.
const CHAT_MESSAGE_TYPE: usize = 0x0068C144;
/// `g_popup_dialog` — non-null while a modal dialog is up.
const POPUP_DIALOG: usize = 0x006D5BF4;

/// The resolved larva binding (VK + required modifiers), set once at install.
static KEY_VK: AtomicU16 = AtomicU16::new(0);
static KEY_MODS: AtomicU32 = AtomicU32::new(0); // packed Modifiers bits

/// Resolved status for the boot report.
pub(crate) fn status() -> String {
    let vk = KEY_VK.load(Ordering::SeqCst);
    let mods = Modifiers {
        ctrl: KEY_MODS.load(Ordering::SeqCst) & 4 != 0,
        shift: KEY_MODS.load(Ordering::SeqCst) & 2 != 0,
        alt: KEY_MODS.load(Ordering::SeqCst) & 1 != 0,
    };
    if vk == 0 {
        "disabled (no valid key)".to_string()
    } else if mods.any() {
        let names = [mods.ctrl.then_some("CTRL"), mods.shift.then_some("SHIFT"), mods.alt.then_some("ALT")]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("+");
        format!("armed (VK 0x{vk:02X} with {names})")
    } else {
        format!("armed (VK 0x{vk:02X})")
    }
}
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

/// Whether the count-entry window is open (the queue overlay suppresses itself
/// while it is, so the two do not overlap).
pub(crate) fn entry_active() -> bool {
    ACTIVE.load(Ordering::SeqCst)
}

/// The resolved binding (VK + modifier mask), for the boot-time key-collision
/// check. Mask bits: 4 = CTRL, 2 = SHIFT, 1 = ALT.
pub(crate) fn key_vk() -> u16 {
    KEY_VK.load(Ordering::SeqCst)
}
pub(crate) fn key_mods() -> u32 {
    KEY_MODS.load(Ordering::SeqCst)
}

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
    match parse_binding(&name) {
        Some(b) => {
            apply_binding(&b, &name);
        }
        None => crate::log::log(&format!(
            "[cosmo-plus] larva pick: unknown CM_PLUS_LARVA_KEY={name:?}; disabled"
        )),
    }
    if KEY_VK.load(Ordering::SeqCst) != 0 {
        install_hook();
    }
}

fn apply_binding(b: &Binding, name: &str) {
    KEY_VK.store(b.vk, Ordering::SeqCst);
    KEY_MODS.store(b.mods.mask(), Ordering::SeqCst);
    let mods = format!(
        "{}{}{}",
        if b.mods.ctrl { "Ctrl+" } else { "" },
        if b.mods.shift { "Shift+" } else { "" },
        if b.mods.alt { "Alt+" } else { "" },
    );
    crate::log::log(&format!(
        "[cosmo-plus] larva pick: key {name} (VK 0x{:02X}); press {mods}it, type a \
         count (1-99), Enter or wait to select",
        b.vk
    ));
    if b.mods.alt {
        crate::log::log(
            "[cosmo-plus] larva pick: NOTE the chord uses Alt — Alt+Space is the \
             Windows system-menu combo and will be suppressed in-game",
        );
    }
}

/// Re-parse the larva key from config (in-place reload).
pub(crate) fn rearm() {
    let name = crate::config::env("CM_PLUS_LARVA_KEY").unwrap_or_else(|| DEFAULT_LARVA_KEY.to_string());
    match parse_binding(&name) {
        Some(b) => {
            apply_binding(&b, &name);
            crate::log::log(&format!("[cosmo-plus] larva pick: reloaded key {name}"));
        }
        None => {
            KEY_VK.store(0, Ordering::SeqCst);
            KEY_MODS.store(0, Ordering::SeqCst);
            crate::log::log(&format!("[cosmo-plus] larva pick: reloaded with unknown key {name:?}; disabled"));
        }
    }
}

use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
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
        crate::state::hook("keyboard:larva-pick", h as usize, 0, 0);
        *HOOK.lock().unwrap() = Some(h as isize);
    }
}

/// Whether the configured chord's modifiers are all currently held (extra
/// modifiers are ignored: "at least these held" semantics).
fn chord_held() -> bool {
    let mask = KEY_MODS.load(Ordering::SeqCst);
    if mask == 0 {
        return true;
    }
    let ctrl = (unsafe { GetAsyncKeyState(0x11) as u16 & 0x8000 }) != 0;
    let shift = (unsafe { GetAsyncKeyState(0x10) as u16 & 0x8000 }) != 0;
    let alt = (unsafe { GetAsyncKeyState(0x12) as u16 & 0x8000 }) != 0;
    let held = (ctrl as u32) << 2 | (shift as u32) << 1 | (alt as u32);
    held & mask == mask
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
    // Never while chat or a modal dialog is open (the idle/tab hooks do the
    // same; with a chord default this matters: Ctrl+Space mid-chat must not
    // open the entry window and eat the digits being typed into chat).
    let chat = std::ptr::read(CHAT_MESSAGE_TYPE as *const *const u8);
    if !chat.is_null() && std::ptr::read(chat) != 0 {
        return call_next(code, wparam, lparam);
    }
    if std::ptr::read(POPUP_DIALOG as *const usize) != 0 {
        return call_next(code, wparam, lparam);
    }

    let larva_vk = KEY_VK.load(Ordering::SeqCst);
    if larva_vk != 0 && key == larva_vk as u32 && chord_held() {
        // Bit 30 = the key was already down: auto-repeat. Swallow repeats
        // WITHOUT toggling (holding the chord would otherwise open and
        // instantly re-close the entry window at repeat rate).
        let repeat = lparam as u32 & (1 << 30) != 0;
        if !repeat {
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
        }
        return 1; // consume the key
    }

    if !ACTIVE.load(Ordering::SeqCst) {
        return call_next(code, wparam, lparam);
    }

    match key {
        // Main-row digits and numpad digits feed the count — but only bare
        // digits: Ctrl+0-9 are control groups and stay with the game (the
        // user releases Ctrl after the chord before typing the count).
        0x30..=0x39 | 0x60..=0x69 => {
            if (GetAsyncKeyState(0x11) as u16 & 0x8000) != 0 {
                return call_next(code, wparam, lparam);
            }
            let digit = if key >= 0x60 { key - 0x60 } else { key - 0x30 };
            ENTRY.lock().unwrap().push_digit(digit);
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

    // The hatcheries are the client's current selection (the extended table —
    // the 12-slot vanilla table would miss part of a big selection).
    let Some((_ids, sel_ptrs)) = crate::tab_cycle::selection_ptrs() else {
        crate::log::log("[cosmo-plus] larva pick: nothing selected (select hatcheries first)");
        return;
    };
    let hatcheries: Vec<*mut Unit> = sel_ptrs
        .iter()
        .map(|&p| p as *mut Unit)
        .filter(|&p| !p.is_null())
        .collect();
    if hatcheries.is_empty() {
        crate::log::log("[cosmo-plus] larva pick: nothing selected (select hatcheries first)");
        return;
    }

    // Gather larva: connected_unit points at one of the selected hatcheries and the
    // unit id is a larva type; hidden larva are skipped (same rule as the mod).
    let mut list: Vec<*mut Unit> = Vec::new();
    crate::frame::for_each_unit(|u| {
        if list.len() >= SELECTION_LIMIT {
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
        list.push(u);
    });

    let n = list.len();
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

/// Draw the count-entry feedback while the window is open, at the given
/// origin (screen layer or world-pass fallback).
pub(crate) unsafe fn draw(origin: (i32, i32)) {
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
    crate::overlay::draw_text_lines(origin, "Larva", &[line]);
}
