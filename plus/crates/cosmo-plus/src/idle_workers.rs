//! Idle-worker hotkey: one key press selects a single idle worker of the local
//! player, and the HUD shows the live idle-worker count above the minimap
//! (drawn by the overlay's `draw_idle_counter`, SC2-style).
//!
//! An idle worker is a completed worker unit that is not gathering, not carrying
//! cargo back, **not moving**, and either has no harvest link or sits on a
//! parked order (Stop / Guard / PlayerGuard — the game parks jobless workers on
//! those, and some workers, e.g. Zerg Agrileth, keep a stale harvest target
//! while parked; the parked orders catch those too).
//!
//! The selection goes through the engine's own click-selection routine
//! (`select_units_if_alt_not_held`, `BW/bwhelper.cpp`, 0x0046FA00), which replaces the
//! current selection, plays the selection sound and **sends the synced select command**
//! to peers — so the selection is network-consistent and rollback-safe (it travels in
//! the normal turn stream, never as a raw memory write).
//!
//! One press selects **one** idle worker; each further press selects the next one,
//! SC2-F1 style, wrapping back to the first.
//!
//! The key is captured by a thread keyboard hook (the same mechanism as the Tab
//! subgroup cycle), not by polling `GetAsyncKeyState` from the draw pass: the
//! game's own handler consumes function keys before a polled read would see a
//! tap, which is why the first F9 build never fired. The hook queues the press
//! and the draw pass performs the selection, so it still never runs during a
//! rollback re-simulation. The press is swallowed, so a control group bound to
//! the same key does not also fire; modified presses (Ctrl/Alt/Shift + key) are
//! left to the game, and the key is inert in chat and in modal dialogs.
//!
//! Configure with `CM_PLUS_IDLE_WORKER_KEY` in `cosmo_plus.conf` (or the environment):
//! named keys (`F1`–`F12`, `A`–`Z`, `0`–`9`, `SPACE`, `TAB`, `COMMA`, `PERIOD`, …),
//! the punctuation character itself (`,`), or a raw virtual-key code as hex (`0x7A`).
//! The default is **F9** — unbound in stock Brood War and in the mod's hotkey layer
//! (F1–F8 are camera screens, F10 the menu, F11 the HUD toggle).

use std::sync::atomic::{AtomicBool, AtomicU16, AtomicUsize, Ordering};
use std::sync::Mutex;

use cosmo_plus_core::hotkeys::{parse_key, DEFAULT_IDLE_WORKER_KEY};
use cosmo_plus_core::idle::{is_idle_worker, UnitView};

use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_CONTROL, VK_MENU, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowsHookExW, WH_KEYBOARD};

/// `g_chat_message_type` — non-null/!= 0 while the chat line is open.
const CHAT_MESSAGE_TYPE: usize = 0x0068C144;
/// `g_popup_dialog` — non-null while a modal dialog is up.
const POPUP_DIALOG: usize = 0x006D5BF4;

/// How many units one selection can hold (`SelectionCap`).
const SELECTION_CAP: usize = 12;
/// Most idle workers tracked for cycling (well above any real army of workers).
const MAX_IDLE: usize = 64;

/// The engine adapter for the core's [`UnitView`] (a local wrapper, because the
/// orphan rule forbids implementing the core trait for `bw_dat`'s type here).
struct View<'a>(&'a bw_dat::structs::Unit);

impl UnitView for View<'_> {
    fn unit_id(&self) -> u16 {
        self.0.unit_id
    }
    fn player(&self) -> u8 {
        self.0.player
    }
    fn build_time_remaining(&self) -> u16 {
        self.0.remaining_build_time
    }
    fn carried_resources(&self) -> u8 {
        unsafe { self.0.unit_specific.worker.carried_resource_count }
    }
    fn is_harvesting(&self) -> bool {
        unsafe { self.0.unit_specific.worker.is_harvesting != 0 }
    }
    fn is_moving(&self) -> bool {
        let f = &self.0.flingy;
        f.current_speed != 0 || f.current_speed_x != 0 || f.current_speed_y != 0
    }
    fn has_harvest_target(&self) -> bool {
        !unsafe { self.0.unit_specific.worker.current_harvest_target }.is_null()
    }
    fn order(&self) -> u8 {
        self.0.order
    }
}

/// The local player's idle-worker count (the HUD counter value).
pub(crate) unsafe fn count_idle(local: u32) -> u32 {
    let mut n = 0u32;
    crate::frame::for_each_unit(|u| {
        if (*u).player as u32 != local {
            return;
        }
        if !is_idle_worker(&View(&*u)) {
            return;
        }
        n += 1;
    });
    n
}

/// The command-card icon id of the first idle worker, or None when none are
/// idle. Used for the SC2-style HUD badge.
pub(crate) unsafe fn first_idle_icon(local: u32) -> Option<u32> {
    let mut icon: Option<u32> = None;
    crate::frame::for_each_unit(|u| {
        if icon.is_some() {
            return;
        }
        if (*u).player as u32 != local {
            return;
        }
        if !is_idle_worker(&View(&*u)) {
            return;
        }
        icon = Some(bw_dat::UnitId((*u).unit_id).icon());
    });
    icon
}

/// The resolved hotkey VK (0 = disabled), set once at install.
static KEY_VK: AtomicU16 = AtomicU16::new(0);
/// Set by the keyboard hook on a fresh (non-repeat) press; consumed by the draw pass.
static QUEUED: AtomicBool = AtomicBool::new(false);
/// The last unit this hotkey selected, so the next press picks the next idle worker.
static LAST_SELECTED: AtomicUsize = AtomicUsize::new(0);
/// The keyboard hook handle, kept for the process lifetime.
static HOOK: Mutex<Option<isize>> = Mutex::new(None);

/// Resolved status for the boot report.
pub(crate) fn status() -> String {
    let vk = KEY_VK.load(Ordering::SeqCst);
    if vk == 0 {
        "disabled (no valid key)".to_string()
    } else {
        format!("armed (VK 0x{vk:02X})")
    }
}

/// Resolve and log the hotkey, and install the keyboard hook (one-time, at load).
pub(crate) fn install() {
    let name = crate::config::env("CM_PLUS_IDLE_WORKER_KEY")
        .unwrap_or_else(|| DEFAULT_IDLE_WORKER_KEY.to_string());
    match parse_key(&name) {
        Some(vk) => {
            KEY_VK.store(vk, Ordering::SeqCst);
            crate::log::log(&format!(
                "[cosmo-plus] idle workers: hotkey {name} (VK 0x{vk:02X}) selects one idle worker"
            ));
            install_hook();
        }
        None => crate::log::log(&format!(
            "[cosmo-plus] idle workers: unknown CM_PLUS_IDLE_WORKER_KEY={name:?}; hotkey disabled \
             (use e.g. F9, F12, COMMA, ',', SPACE, or a hex VK like 0x7A)"
        )),
    }
}

/// Re-resolve the hotkey from config (in-place reload). The keyboard hook
/// stays installed; it just looks at the new VK.
pub(crate) fn rearm() {
    let name = crate::config::env("CM_PLUS_IDLE_WORKER_KEY")
        .unwrap_or_else(|| DEFAULT_IDLE_WORKER_KEY.to_string());
    match parse_key(&name) {
        Some(vk) => {
            KEY_VK.store(vk, Ordering::SeqCst);
            crate::log::log(&format!(
                "[cosmo-plus] idle workers: reloaded hotkey {name} (VK 0x{vk:02X})"
            ));
        }
        None => {
            KEY_VK.store(0, Ordering::SeqCst);
            crate::log::log(&format!(
                "[cosmo-plus] idle workers: reloaded with unknown key {name:?}; disabled"
            ));
        }
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
                "[cosmo-plus] idle workers: SetWindowsHookEx failed; the hotkey is disabled",
            );
            return;
        }
        crate::log::log(
            "[cosmo-plus] idle workers: keyboard hook installed (the key is swallowed in-game)",
        );
        crate::state::hook("keyboard:idle-workers", h as usize, 0, 0);
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

/// WH_KEYBOARD hook: runs ahead of gptp's own hook. Swallows the idle-worker key
/// while a game is running (so a control group on the same key never fires) and
/// queues the press for the draw pass. Chat, modal dialogs and modified presses
/// are never interfered with.
unsafe extern "system" fn keyboard_proc(code: i32, wparam: usize, lparam: isize) -> isize {
    if code < 0 {
        return call_next(code, wparam, lparam);
    }
    let key = wparam as u32;
    let down = lparam as u32 & (1 << 31) == 0; // bit 31 set = key-up
    if !down {
        return call_next(code, wparam, lparam);
    }
    let vk = KEY_VK.load(Ordering::SeqCst);
    if vk == 0 || key != vk as u32 {
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
    // Ctrl/Alt/Shift + key stay with the game (control-group assign/recall, etc.).
    if modifier_down() {
        return call_next(code, wparam, lparam);
    }
    // Bit 30 = the key was already down: auto-repeat. Swallow it, do not re-queue.
    if lparam as u32 & (1 << 30) == 0 {
        QUEUED.store(true, Ordering::SeqCst);
    }
    1 // swallow: the control group on this key never fires
}

/// Queue a press from outside the keyboard path (the dev REPL's `idle` command).
pub(crate) fn queue_press() {
    QUEUED.store(true, Ordering::SeqCst);
}

/// The local player's id, or `None` while it is not yet resolvable.
pub(crate) unsafe fn local_player() -> Option<u32> {
    let api = crate::plus::api();
    let f = (api.local_player_id)()?;
    Some(f())
}

/// Called from the draw pass every frame. Consumes a queued press and selects
/// **one** idle worker through the engine's own selection routine (synced
/// command). Each further press selects the next idle worker.
pub(crate) unsafe fn poll() {
    if !QUEUED.swap(false, Ordering::SeqCst) {
        return;
    }
    if crate::frame::frame_count() == 0 {
        return; // not in a game
    }
    let Some(local) = local_player() else {
        return;
    };

    let mut idle: [*mut bw_dat::structs::Unit; MAX_IDLE] =
        [core::ptr::null_mut(); MAX_IDLE];
    let mut count = 0usize;
    crate::frame::for_each_unit(|u| {
        if count >= MAX_IDLE {
            return;
        }
        if (*u).player as u32 != local {
            return;
        }
        if !is_idle_worker(&View(&*u)) {
            return;
        }
        idle[count] = u;
        count += 1;
    });
    if count == 0 {
        LAST_SELECTED.store(0, Ordering::SeqCst);
        crate::log::log("[cosmo-plus] idle workers: none found");
        return;
    }

    // Pick the idle worker after the last one we selected (wrapping).
    let last = LAST_SELECTED.load(Ordering::SeqCst);
    let mut idx = 0usize;
    if last != 0 {
        if let Some(pos) = (0..count).find(|&i| idle[i] as usize == last) {
            idx = (pos + 1) % count;
        }
    }
    let chosen = idle[idx];
    LAST_SELECTED.store(chosen as usize, Ordering::SeqCst);

    let pos = &(*chosen).flingy.position;
    crate::log::log(&format!(
        "[cosmo-plus] idle workers: selecting 1 of {count} (index {idx}) at ({},{})",
        pos.x, pos.y
    ));
    let list: [*mut bw_dat::structs::Unit; SELECTION_CAP] = [chosen; SELECTION_CAP];
    crate::select::select_units_if_alt_not_held(list.as_ptr(), 1, 1, 1);
    // SC2-F1 behaviour: jump the camera to the worker you just selected,
    // through the engine's own move_screen (the same call the fork's F1-F8
    // camera hotkeys use, so the clamps and the scroll system handle it).
    // Writing the camera globals directly fought the engine's scroll logic
    // and threw the view off — not done.
    crate::select::move_screen(pos.x as u16, pos.y as u16);
}
