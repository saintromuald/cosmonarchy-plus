//! Screen-fixed HUD overlays (idle-worker badge, Tab subgroup indicator,
//! C1–C8 camera labels, nydus minimap numbers), drawn at the engine's own
//! screen-text layer.
//!
//! Why this hook: the draw-proc hook (0x004BD68D) runs on the **world
//! surface**, which the fork's zoom resamples (crop + magnify) onto the
//! screen buffer. Anything drawn near the world surface's edges — a HUD
//! badge anchored to the minimap, say — falls outside the zoom crop and
//! never reaches the screen once the player zooms in. The fork draws its
//! own screen-space text (message log, objectives) through
//! `draw_deferred_screen_text`, called **after the resample** when
//! magnified — on the buffer that actually gets presented. This module
//! patches that call's rel32 to a wrapper that chains the original, then
//! draws the overlays: exactly the layer SC2 puts its HUD text on.
//!
//! At 100% zoom the fork does not resample (the world surface is presented
//! directly) and that call site is skipped, so the overlays also draw from
//! the world pass (overlay.rs) — visible there because nothing is cropped.
//! The two paths never double-draw: one is skipped whenever the other shows.
//!
//! The call site is resolved by a masked signature scan of gptp.qdp:
//! the guarded call (`if (g_zoom.magnified())`) sits right before the
//! layer-walk init (`mov edx, 7`), which pins the pattern uniquely. The
//! rel32 is then read live, so relocations never matter.

use std::sync::atomic::{AtomicU32, Ordering};

/// Signature (masked): `mov ecx, [g_zoom-state]; test ecx,ecx; je +5;
/// e8 rel32; mov edx, 7` — the magnified-branch deferred-screen-text call
/// right before the screen-layer walk init (grid.cpp, i==2 block).
const SITE_SIG: &[u8] = &[
    0x8B, 0x0D, 0x00, 0x00, 0x00, 0x00, // mov ecx, ds:0x10584888 (relocated)
    0x85, 0xC9, // test ecx, ecx
    0x74, 0x05, // je +5
    0xE8, 0x00, 0x00, 0x00, 0x00, // call rel32
    0xBA, 0x07, 0x00, 0x00, 0x00, // mov edx, 7 (layer-walk init)
];
const SITE_MASK: &[u8] = &[
    b'x', b'x', b'?', b'?', b'?', b'?', b'x', b'x', b'x', b'x', b'x', b'?', b'?', b'?', b'?',
    b'x', b'x', b'x', b'x', b'x',
];

/// The original call target (draw_deferred_screen_text), resolved live.
static CHAIN: AtomicU32 = AtomicU32::new(0);

/// Resolved status for the boot report.
pub(crate) fn status() -> String {
    let chain = CHAIN.load(Ordering::SeqCst);
    if chain == 0 {
        "disabled (deferred-screen-text call site not found)".to_string()
    } else {
        format!("armed (chain 0x{chain:08X})")
    }
}

/// Install the wrapper: scan gptp for the guarded-call site, chain the
/// original target, and retarget the call to the wrapper.
pub(crate) fn install() {
    unsafe {
        let Some((base, size)) = crate::zoom::gptp_module() else {
            crate::log::log("[cosmo-plus] hud overlay: gptp.qdp not loaded; disabled");
            return;
        };
        let Some(off) = crate::state::scan(base, size, SITE_SIG, SITE_MASK) else {
            crate::log::log(
                "[cosmo-plus] hud overlay: deferred-screen-text call site not found; \
                 disabled (different gptp build?)",
            );
            return;
        };
        let site = off + 10; // the E8
        let rel = std::ptr::read((site as *const i32));
        let chain = site as u32 + 5 + rel as u32;
        CHAIN.store(chain, Ordering::SeqCst);

        let my = wrapper as *const () as usize as u32;
        let new_rel = my.wrapping_sub(site as u32 + 5) as i32;
        let patch: [u8; 5] = [
            0xE8,
            new_rel as u8,
            (new_rel >> 8) as u8,
            (new_rel >> 16) as u8,
            (new_rel >> 24) as u8,
        ];
        let api = crate::plus::api();
        let r = (api.write_exe_memory)(site, patch.as_ptr(), patch.len());
        crate::log::log(&format!(
            "[cosmo-plus] hud overlay: hooked deferred screen text @ 0x{site:08X} \
             (chain 0x{chain:08X}, patch={r})"
        ));
        crate::state::hook("deferred-screen-text", site, chain as usize, my as usize);
    }
}

/// Replaces the original callee: chain it first (same registers the caller
/// would have passed — none), then draw the screen-fixed overlays.
unsafe extern "C" fn wrapper() {
    let chain = CHAIN.load(Ordering::SeqCst);
    if chain != 0 {
        core::arch::asm!(
            "pushad",
            "call dword ptr [{func}]",
            "popad",
            func = sym CHAIN,
            options(nostack),
        );
    }
    draw_overlays();
}

/// Draw every screen-fixed overlay: the small font is selected by each
/// element (they share the same setup dance), and the surface is the game's
/// screen buffer at this point, so coordinates are plain screen pixels.
pub(crate) unsafe fn draw_overlays() {
    if crate::frame::frame_count() == 0 {
        return; // not in a game
    }
    let mut buf = [0u8; 16];
    // Minimap-position labels first (they sit on the map panel).
    crate::nydus::draw_minimap(&mut buf);
    crate::cam_labels::draw_minimap(&mut buf);
    // HUD badges: the idle-worker counter above the minimap and the Tab
    // subgroup indicator.
    crate::overlay::draw_idle_counter();
    crate::overlay::draw_tab_indicator();
    // Diagnostic probe (CM_PLUS_HUD_PROBE=1): markers at the candidate
    // coordinate transforms, for the headless screenshot-diff bisect.
    if crate::config::env_flag("CM_PLUS_HUD_PROBE") {
        probe_markers();
    }
}

/// One-shot state log + per-frame markers at the candidate transforms:
/// M1 = plain screen coords, M2 = camera-relative (world-surface style),
/// M3 = anchored on the minimap rect. The screenshot diff between a probe
/// run and a plain run shows exactly where each transform lands.
unsafe fn probe_markers() {
    let rect = crate::state::minimap_rect().unwrap_or([6, 348, 133, 475]);
    let (cx, cy) = crate::overlay::camera();
    static LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !LOGGED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        let surface = std::ptr::read(0x006C_F4A8 as *const usize);
        let font = crate::overlay::in_game_font() as usize;
        crate::log::log(&format!(
            "[cosmo-plus] hud probe: surface=0x{surface:08X} font=0x{font:08X} \
             rect={rect:?} camera=({cx},{cy})"
        ));
    }
    crate::overlay::get_color_handle(cosmo_plus_core::saturation::text_color::YELLOW);
    let m1 = b"M1\0";
    let m2 = b"M2\0";
    let m3 = b"M3\0";
    crate::overlay::bw_print_xy(m1.as_ptr(), 8, 8);
    crate::overlay::bw_print_xy(m2.as_ptr(), 8 + cx, 8 + cy);
    crate::overlay::bw_print_xy(m3.as_ptr(), rect[0] + 2, rect[1] + 2);
}
