//! C1–C8 camera hotkey labels on the minimap.
//!
//! gptp's extended camera hotkeys (F1–F8; save with Ctrl/Shift+F) keep eight
//! camera positions. The fork exports them through `cosmo_camera_screens`
//! (CameraCap × v2u16 of world px), and this module draws a small centred
//! label — C1..C8 — at each position on the minimap, so the player sees at a
//! glance which key jumps where. Nothing is drawn when the export is missing
//! (older gptp build) or outside a game.

use std::sync::OnceLock;

/// The gptp export name. `const` C-string.
const EXPORT_NAME: &[u8] = b"cosmo_camera_screens\0";
/// gptp's module name as it loads in the game.
const GPTp_MODULE: &[u8] = b"gptp.qdp\0";

/// `CameraCap` in the fork (F1..F8).
const CAMERA_CAP: usize = 8;

/// Resolved export (pointer into gptp.qdp), set once at install.
static SCREENS_FN: OnceLock<Option<unsafe extern "system" fn() -> *const u16>> =
    OnceLock::new();

/// Resolve the gptp export once and log the result.
pub(crate) fn install() {
    let _ = SCREENS_FN.get_or_init(|| unsafe {
        use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
        let module = GetModuleHandleA(GPTp_MODULE.as_ptr() as *const u8);
        if module.is_null() {
            crate::log::log("[cosmo-plus] camera labels: gptp.qdp not loaded");
            return None;
        }
        let proc = GetProcAddress(module, EXPORT_NAME.as_ptr() as *const u8);
        if proc.is_none() {
            crate::log::log(
                "[cosmo-plus] camera labels: gptp build lacks cosmo_camera_screens; \
                 C1-C8 minimap labels disabled (rebuild gptp to enable)",
            );
            return None;
        }
        let raw = proc.unwrap() as usize as *mut core::ffi::c_void;
        crate::log::log("[cosmo-plus] camera labels: C1-C8 minimap labels enabled");
        Some(core::mem::transmute::<
            *mut core::ffi::c_void,
            unsafe extern "system" fn() -> *const u16,
        >(raw))
    });
}

/// Draw the C1–C8 labels onto the minimap. Called from the overlay draw pass
/// (after the nydus minimap numbers; the small font is already selected).
pub(crate) unsafe fn draw_minimap(buf: &mut [u8; 16]) {
    if crate::frame::frame_count() == 0 {
        return;
    }
    let Some(Some(screens_fn)) = SCREENS_FN.get().copied() else {
        return;
    };
    let screens = screens_fn();
    if screens.is_null() {
        return;
    }

    let rect = *(crate::nydus::G_MINIMAP_RECT as *const [i32; 4]);
    let (left, top, right, bottom) = (rect[0], rect[1], rect[2], rect[3]);
    let map_w = *(crate::nydus::MAP_SIZE_X as *const u16) as i32;
    let map_h = *(crate::nydus::MAP_SIZE_Y as *const u16) as i32;
    if map_w <= 0 || map_h <= 0 || right <= left || bottom <= top {
        return;
    }
    let (cx, cy) = crate::overlay::camera();
    crate::overlay::get_color_handle(cosmo_plus_core::saturation::text_color::WHITE);

    for i in 0..CAMERA_CAP {
        let x = *screens.add(i * 2) as i32;
        let y = *screens.add(i * 2 + 1) as i32;
        if x == 0 && y == 0 {
            continue; // never saved
        }
        let mx = left + x / crate::nydus::PX_PER_TILE * (right - left) / map_w;
        let my = top + y / crate::nydus::PX_PER_TILE * (bottom - top) / map_h;
        buf.fill(0);
        let label = format!("C{}", i + 1);
        let len = label.len().min(buf.len());
        buf[..len].copy_from_slice(&label.as_bytes()[..len]);
        // Centred on the point (small font, 2 chars) — screen-fixed like the
        // nydus minimap labels.
        crate::overlay::bw_print_xy(buf.as_ptr(), mx + cx - 5, my + cy - 4);
    }
}
