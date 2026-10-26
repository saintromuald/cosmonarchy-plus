//! Ctrl + WASD camera pan.
//!
//! While Ctrl is **held**, W/A/S/D are swallowed and re-posted to the game window
//! as Up/Left/Down/Right key events, so the camera pans like WASD panning in a modern
//! RTS without moving the left hand. Release Ctrl and WASD behave normally again.
//!
//! The game's own wndproc processes the posted arrow events exactly like real key
//! presses (down/up pairs, including auto-repeat while a key is held), so no engine
//! input-table layout is assumed here. Disable with `CM_PLUS_NO_CAM_PAN=1`.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    FindWindowA, PostMessageA, SetWindowsHookExW, WM_KEYDOWN, WM_KEYUP, WH_KEYBOARD,
};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;

const VK_CONTROL: i32 = 0x11;
const VK_W: u32 = 0x57;
const VK_A: u32 = 0x41;
const VK_S: u32 = 0x53;
const VK_D: u32 = 0x44;
const VK_UP: u32 = 0x26;
const VK_LEFT: u32 = 0x25;
const VK_DOWN: u32 = 0x28;
const VK_RIGHT: u32 = 0x27;

/// The game window handle (`SWarClass`), found once at install.
static GAME_HWND: AtomicUsize = AtomicUsize::new(0);
/// Whether the hook is installed (master switch resolved at install).
static ENABLED: AtomicBool = AtomicBool::new(false);

/// Resolved status for the boot report.
pub(crate) fn status() -> String {
    if ENABLED.load(Ordering::SeqCst) {
        "armed (Ctrl + WASD)".to_string()
    } else {
        "disabled (hook failed or no window)".to_string()
    }
}
/// The keyboard hook handle, kept for the process lifetime.
static HOOK: Mutex<Option<isize>> = Mutex::new(None);

pub(crate) fn install() {
    if crate::config::env_flag("CM_PLUS_NO_CAM_PAN") {
        crate::log::log("[cosmo-plus] cam pan: disabled (CM_PLUS_NO_CAM_PAN)");
        return;
    }
    unsafe {
        let hwnd = FindWindowA(c"SWarClass".as_ptr() as *const u8, std::ptr::null());
        if hwnd as usize != 0 {
            GAME_HWND.store(hwnd as usize, Ordering::SeqCst);
            crate::log::log(&format!(
                "[cosmo-plus] cam pan: hold Ctrl and W/A/S/D pans the camera (window 0x{:x})",
                hwnd as usize
            ));
        } else {
            crate::log::log(
                "[cosmo-plus] cam pan: game window not found yet; hook forwards nothing \
                 until a window exists",
            );
        }
        let h = SetWindowsHookExW(
            WH_KEYBOARD,
            Some(keyboard_proc),
            std::ptr::null_mut(),
            GetCurrentThreadId(),
        );
        if h.is_null() {
            crate::log::log(
                "[cosmo-plus] cam pan: SetWindowsHookEx failed; Ctrl+WASD pan disabled",
            );
            return;
        }
        ENABLED.store(true, Ordering::SeqCst);
        crate::state::hook("keyboard:cam-pan", h as usize, 0, 0);
        *HOOK.lock().unwrap() = Some(h as isize);
    }
}

fn call_next(code: i32, wparam: usize, lparam: isize) -> isize {
    use windows_sys::Win32::UI::WindowsAndMessaging::CallNextHookEx;
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

/// WH_KEYBOARD hook: while Ctrl is held, WASD is swallowed and mirrored into the
/// matching arrow key. Runs ahead of gptp's own hook (installed later = runs first).
unsafe extern "system" fn keyboard_proc(code: i32, wparam: usize, lparam: isize) -> isize {
    if code < 0 || !ENABLED.load(Ordering::SeqCst) {
        return call_next(code, wparam, lparam);
    }
    let vk = wparam as u32;
    let arrow = match vk {
        VK_W => Some(VK_UP),
        VK_A => Some(VK_LEFT),
        VK_S => Some(VK_DOWN),
        VK_D => Some(VK_RIGHT),
        _ => None,
    };
    let Some(arrow) = arrow else {
        return call_next(code, wparam, lparam);
    };
    let ctrl_held = (GetAsyncKeyState(VK_CONTROL) as u16 & 0x8000) != 0;
    if !ctrl_held {
        return call_next(code, wparam, lparam);
    }
    // Only during a game (never swallow WASD in menus/dialogs).
    if unsafe { crate::frame::frame_count() } == 0 {
        return call_next(code, wparam, lparam);
    }

    let down = lparam as u32 & (1 << 31) == 0;
    let mut hwnd = GAME_HWND.load(Ordering::SeqCst) as windows_sys::Win32::Foundation::HWND;
    if hwnd as usize == 0 {
        // Window not resolved at install: find it lazily on first use.
        hwnd = unsafe { FindWindowA(c"SWarClass".as_ptr() as *const u8, std::ptr::null()) };
        GAME_HWND.store(hwnd as usize, Ordering::SeqCst);
    }
    if hwnd as usize != 0 {
        let msg = if down { WM_KEYDOWN } else { WM_KEYUP };
        // lParam: down = 1; up = released + previous-down bits, like a real key-up.
        let lp = if down { 0x0000_0001u32 } else { 0xC000_0001u32 };
        unsafe { PostMessageA(hwnd, msg, arrow as usize, lp as isize) };
    }
    1 // swallow the original WASD event while Ctrl is held
}
