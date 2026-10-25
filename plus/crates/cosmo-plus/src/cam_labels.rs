//! C1–C8 camera hotkey labels on the minimap.
//!
//! gptp's extended camera hotkeys (F1–F8; save with Ctrl/Shift+F) keep eight
//! camera positions in the `Hotkeys` global (`v2u16 screens[8]`, world px).
//! gptp.qdp exports nothing — adding a `__declspec(dllexport)` to it makes the
//! game's plugin loader misbehave (the game froze at match start with an
//! export build, verified empirically) — so the screens array is resolved by
//! **signature scan**, the same technique `zoom.rs` uses for its patch site:
//! the F1–F8 handler's indexed access carries the absolute (preferred-base)
//! VA of `hotkeys.screens` as an imm32.
//!
//! Nothing is drawn when the signature is missing (different gptp build — the
//! feature disarms loudly, and the boot report says so) or outside a game.

use std::sync::OnceLock;

/// `CameraCap` in the fork (F1..F8).
const CAMERA_CAP: usize = 8;

/// The F1–F8 handler in the fork's `handle_key_event` (x86, MSVC):
///   lea eax,[ecx-0x70]; cmp eax,7; ja done;
///   cmp [esi+0xd],0 (control); cmp [esi+0xe],0 (shift);
///   push dword [ecx*4 + hotkeys.screens]
/// The imm32 at offset 22 is the absolute VA of `hotkeys.screens` at the
/// image's preferred base (0x10000000). Verified against the shipped build.
const SCREENS_SIG: &[u8] = &[
    0x8D, 0x41, 0x90, // lea eax,[ecx-0x70]
    0x83, 0xF8, 0x07, // cmp eax,0x7
    0x77, 0x47, // ja done
    0x80, 0x7E, 0x0D, 0x00, // cmp byte [esi+0xd],0  (control held)
    0x75, 0x1B, // jne save-branch
    0x80, 0x7E, 0x0E, 0x00, // cmp byte [esi+0xe],0  (shift held)
    0x75, 0x15, // jne save-branch
    0xFF, 0x34, 0x8D, // push dword [ecx*4 + imm32]
];
const SCREENS_IMM_OFF: usize = 23;
const GPTP_PREFERRED_BASE: u32 = 0x1000_0000;

/// Resolved status for the boot report.
pub(crate) fn status() -> String {
    match SCREENS_ADDR.get().copied().flatten() {
        Some(addr) => format!("armed (C1-C8 minimap labels @ 0x{addr:08X})"),
        None => match SCREENS_SIG_OFF.get().copied().flatten() {
            Some(_) => "armed (signature found; address resolves at first draw)".to_string(),
            None => "disabled (camera-screen signature not found in gptp.qdp)".to_string(),
        },
    }
}

static SCREENS_SIG_OFF: OnceLock<Option<usize>> = OnceLock::new();
/// The live screens address, resolved at first draw (relocations happen after install).
static SCREENS_ADDR: OnceLock<Option<usize>> = OnceLock::new();

/// Scan gptp.qdp for the camera-screen handler once; the screens *address*
/// itself is resolved lazily at first draw, because the game relocates gptp's
/// immediates after plugin install (the boot-time value is not the live one).
pub(crate) fn install() {
    let _ = SCREENS_SIG_OFF.get_or_init(|| unsafe {
        let Some((base, size)) = crate::zoom::gptp_module() else {
            crate::log::log("[cosmo-plus] camera labels: gptp.qdp not loaded");
            return None;
        };
        let Some(off) = crate::state::scan(base, size, SCREENS_SIG, &[b'x'; SCREENS_SIG.len()])
        else {
            crate::log::log(
                "[cosmo-plus] camera labels: camera-screen signature not found in gptp.qdp; \
                 C1-C8 minimap labels disabled (different gptp build?)",
            );
            return None;
        };
        crate::log::log(&format!(
            "[cosmo-plus] camera labels: camera-screen handler found @ +0x{:X} \
             (screens resolved lazily at first draw)",
            off - base,
        ));
        Some(off - base)
    });
}

/// Resolve `hotkeys.screens` from the live imm32 at the signature site. The
/// imm is rewritten to the loaded address once the game relocates gptp, so
/// this runs at first draw, not at install.
unsafe fn resolve_screens_live() -> Option<usize> {
    let Some(Some(sig_off)) = SCREENS_SIG_OFF.get().copied() else {
        return None;
    };
    let Some((base, size)) = crate::zoom::gptp_module() else {
        return None;
    };
    let imm = std::ptr::read_unaligned((base + sig_off + SCREENS_IMM_OFF) as *const u32) as usize;
    // Prefer the live (relocated) address; fall back to the preferred-base
    // value when the game has not relocated yet.
    let target = if imm >= base && imm < base + size {
        imm
    } else {
        (base as u32)
            .wrapping_add(imm.wrapping_sub(GPTP_PREFERRED_BASE as usize) as u32)
            as usize
    };
    if target < base || target >= base + size {
        crate::log::log(&format!(
            "[cosmo-plus] camera labels: screens imm {imm:#010x} resolves outside gptp.qdp; disabled"
        ));
        return None;
    }
    Some(target)
}

/// Draw the C1–C8 labels onto the minimap. Called from the overlay draw pass
/// (after the nydus minimap numbers; the small font is already selected).
pub(crate) unsafe fn draw_minimap(buf: &mut [u8; 16]) {
    if crate::frame::frame_count() == 0 {
        return;
    }
    let resolved = SCREENS_ADDR.get_or_init(|| unsafe { resolve_screens_live() });
    let Some(addr) = *resolved else {
        return;
    };
    let screens = addr as *const u16;

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
        // Reject implausible saves (they would project off the minimap).
        if x < 0
            || y < 0
            || x as i64 >= map_w as i64 * crate::nydus::PX_PER_TILE as i64 * 2
            || y as i64 >= map_h as i64 * crate::nydus::PX_PER_TILE as i64 * 2
        {
            continue;
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
