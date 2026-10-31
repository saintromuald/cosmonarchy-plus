//! Screen-fixed overlay drawing through the fork's exported graphics helpers
//! (`cosmo_plus_gfx_text` / `cosmo_plus_gfx_box` / `cosmo_plus_gfx_filled_box`).
//!
//! The fork's graphics system draws ON_SCREEN shapes post-resample in
//! `draw_deferred_shapes` and handles the present through its own refresh
//! path (`want_refresh` -> `refresh_screen`), so anything registered here is
//! screen-fixed at every zoom and always presented. This replaces the
//! previous approach of drawing with the vanilla text primitives inside the
//! game layer update, whose pixels the fork's dirty-cell present never
//! composited.
//!
//! The helpers live in `gptp.qdp`, which the hotloader reloads between games,
//! so the function pointers are re-resolved whenever a new game starts (see
//! `rearm`).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use once_cell::sync::Lazy;

use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};

type GfxTextFn = unsafe extern "C" fn(i32, i32, *const u8);
type GfxBoxFn = unsafe extern "C" fn(i32, i32, i32, i32, u8);

static GFX_TEXT: AtomicU32 = AtomicU32::new(0);
static GFX_BOX: AtomicU32 = AtomicU32::new(0);
static GFX_FILLED_BOX: AtomicU32 = AtomicU32::new(0);

/// The HMODULE the pointers were resolved from (0 = never resolved).
static RESOLVED_FROM: AtomicU32 = AtomicU32::new(0);
static RESOLVE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

unsafe fn resolve_once() {
    let _guard = RESOLVE_LOCK.lock().unwrap();
    let current = GetModuleHandleA(b"gptp.qdp\0".as_ptr() as *const u8) as usize;
    if current == 0 {
        return;
    }
    if RESOLVED_FROM.load(Ordering::SeqCst) == current as u32 {
        return; // already resolved from this module instance
    }
    let mut get = |name: &[u8]| -> u32 {
        let p = GetProcAddress(current as *mut core::ffi::c_void, name.as_ptr() as *const u8);
        p.map(|f| f as usize as u32).unwrap_or(0)
    };
    GFX_TEXT.store(get(b"cosmo_plus_gfx_text\0"), Ordering::SeqCst);
    GFX_BOX.store(get(b"cosmo_plus_gfx_box\0"), Ordering::SeqCst);
    GFX_FILLED_BOX.store(get(b"cosmo_plus_gfx_filled_box\0"), Ordering::SeqCst);
    RESOLVED_FROM.store(current as u32, Ordering::SeqCst);
}

/// Whether the fork exports the gfx helpers (false on older gptp builds; the
/// overlay text then keeps its legacy draw path).
pub(crate) fn available() -> bool {
    unsafe { resolve_once() };
    GFX_TEXT.load(Ordering::SeqCst) != 0
}

/// Re-resolve after a hotloader reload (new game, possibly new gptp module).
pub(crate) fn rearm() {
    RESOLVED_FROM.store(0, Ordering::SeqCst);
    unsafe { resolve_once() };
}

/// Draw one NUL-terminated text line at plain screen coordinates through the
/// fork's graphics system. Falls back to `fallback` (the legacy engine text
/// primitive) when the export is absent.
pub(crate) unsafe fn text(x: i32, y: i32, s: *const u8, fallback: impl FnOnce()) {
    let f = GFX_TEXT.load(Ordering::SeqCst);
    if f == 0 {
        fallback();
        return;
    }
    let f: GfxTextFn = core::mem::transmute(f);
    f(x, y, s);
}

pub(crate) unsafe fn box_outline(left: i32, top: i32, right: i32, bottom: i32, color: u8) {
    let f = GFX_BOX.load(Ordering::SeqCst);
    if f == 0 {
        return;
    }
    let f: GfxBoxFn = core::mem::transmute(f);
    f(left, top, right, bottom, color);
}

pub(crate) unsafe fn box_filled(left: i32, top: i32, right: i32, bottom: i32, color: u8) {
    let f = GFX_FILLED_BOX.load(Ordering::SeqCst);
    if f == 0 {
        return;
    }
    let f: GfxBoxFn = core::mem::transmute(f);
    f(left, top, right, bottom, color);
}
