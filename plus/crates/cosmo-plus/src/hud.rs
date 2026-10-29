//! Screen-fixed HUD overlays (idle-worker badge, Tab subgroup indicator,
//! C1–C8 camera labels, nydus minimap numbers), drawn from the draw-proc
//! hook with a zoom-aware screen mapping.
//!
//! The draw hook runs on the world surface, which the fork's zoom crops and
//! magnifies onto the screen. A screen-fixed element drawn at plain screen
//! coordinates on that surface lands outside the crop at magnified zoom and
//! never shows. Instead of hooking deeper layers (the fork's call sites are
//! relocation targets the game re-applies at init — patching them corrupts
//! the call), the overlays map screen positions into the crop:
//!
//!     surface = crop.left/top + screen * units / 10000
//!
//! where `units` = percent × 100 (10000 = 100%, the fork's BaseUnits). At
//! 100% the crop covers the whole surface and the mapping is the identity;
//! at 200% a screen point lands in the center crop that actually reaches
//! the screen. The zoom state lives in gptp's `.data` (the `g_zoom`
//! WorldZoom object): loader-relocated once at load, never re-patched.
//!
//! `g_zoom` is resolved by validating the candidate layout against the
//! draw-hook surface at first draw (units in [10000, 30000], the crop a
//! sane sub-rect of the surface) — the exact address cannot drift silently.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Preferred-base addresses of the WorldZoom member statics. `units` is
/// `m_current_units` (the draw hook compares it against 10000 = BaseUnits),
/// so `g_zoom` = units - 0x00; the crop follows at +0x14 (four u16s:
/// left/top/width/height, CAMERA-RELATIVE per systems/display/zoom.h). The
/// two neighbours are candidates for other member layouts; validation picks
/// the real one.
const UNITS_PREFERRED: u32 = 0x1058_4890;
const CROP_OFFSET: usize = 0x14;

/// Runtime addresses, resolved once gptp's base is known.
static ZOOM_UNITS_ADDR: AtomicU32 = AtomicU32::new(0);
static ZOOM_CROP_ADDR: AtomicU32 = AtomicU32::new(0);
static RESOLVED: AtomicBool = AtomicBool::new(false);
/// Cached units (percent × 100) and crop for the screen mapping.
static UNITS: AtomicU32 = AtomicU32::new(10000);
static CROP_LEFT: AtomicU32 = AtomicU32::new(0);
static CROP_TOP: AtomicU32 = AtomicU32::new(0);
static CROP_W: AtomicU32 = AtomicU32::new(0);
static CROP_H: AtomicU32 = AtomicU32::new(0);

/// Resolved status for the boot report (zoom statics + screen layer).
pub(crate) fn zoom_status() -> String {
    match (ZOOM_UNITS_ADDR.load(Ordering::SeqCst), RESOLVED.load(Ordering::SeqCst)) {
        (0, _) => "disabled (gptp.qdp not loaded)".to_string(),
        (addr, false) => format!("pending (units @ 0x{addr:08X}, resolves at first draw)"),
        (addr, true) => format!("armed (zoom crop @ 0x{addr:08X})"),
    }
}

/// Resolve the runtime addresses of the zoom statics from the gptp module
/// base (`.data` statics are loader-relocated once; nothing re-patches them).
pub(crate) fn install_zoom_statics() {
    let Some((base, _size)) = crate::zoom::gptp_module() else {
        crate::log::log("[cosmo-plus] hud overlay: gptp.qdp not loaded; disabled");
        return;
    };
    // gptp.qdp's preferred base is 0x1000_0000, but Wine frequently loads it
    // below that (e.g. 0x0256_0000). The delta is modular arithmetic on u32
    // addresses — it must wrap, so use wrapping_sub (a plain subtraction
    // panics in debug builds and kills the game at boot).
    let delta = (base as u32).wrapping_sub(0x1000_0000u32);
    let units = UNITS_PREFERRED + delta;
    ZOOM_UNITS_ADDR.store(units, Ordering::SeqCst);
    ZOOM_CROP_ADDR.store(units + CROP_OFFSET as u32, Ordering::SeqCst);
    crate::log::log(&format!(
        "[cosmo-plus] hud overlay: zoom statics units @ 0x{units:08X} crop @ 0x{:08X} \
         (validates at first draw)",
        units + CROP_OFFSET as u32
    ));
}

/// Map a screen position to world-surface coordinates for the current zoom:
/// camera + crop offset + screen * units / 10000. The camera term is what the
/// previous version was missing — the crop is camera-RELATIVE, so screen (0,0)
/// is the visible view's top-left only after adding both.
pub(crate) unsafe fn screen_to_surface(sx: i32, sy: i32) -> (i32, i32) {
    let (cx, cy) = crate::overlay::camera();
    let units = UNITS.load(Ordering::Relaxed).max(10000) as i32;
    let wx = cx + CROP_LEFT.load(Ordering::Relaxed) as i32 + sx * units / 10000;
    let wy = cy + CROP_TOP.load(Ordering::Relaxed) as i32 + sy * units / 10000;
    (wx, wy)
}

/// The current zoom percentage (100 = 100%), for size compensation: an overlay
/// that must keep a constant SCREEN size draws at `size * 10000 / zoom_units`
/// surface pixels, because the crop magnifies surface pixels onto the screen.
pub(crate) fn zoom_units() -> u32 {
    UNITS.load(Ordering::Relaxed).max(10000)
}

/// Re-read the live zoom statics (units + crop) once per frame, at the top of
/// the draw pass. The crop is CAMERA-RELATIVE — the mod's `systems/display/
/// zoom.h` documents it as "Crop rectangle in world pixels relative to
/// camera" — so a valid crop may start far beyond the surface bounds (the
/// 200% view is camera + crop.left/top). The previous surface-fit validation
/// wrongly rejected exactly that (crop 960,270,960,540 vs a 640x480
/// surface), fell back to identity, and pushed every screen-fixed overlay
/// into world space. Accept the crop whenever the zoom ladder and the
/// dimensions are sane.
unsafe fn refresh_zoom_view(surface_w: i32, surface_h: i32) {
    let units_addr = ZOOM_UNITS_ADDR.load(Ordering::SeqCst);
    if units_addr == 0 {
        return; // gptp.qdp not loaded; identity mapping
    }
    let units = std::ptr::read(units_addr as *const u32);
    let crop = ZOOM_CROP_ADDR.load(Ordering::SeqCst) as *const u16;
    let (l, t, w, h) = (
        *crop as i32,
        *crop.add(1) as i32,
        *crop.add(2) as i32,
        *crop.add(3) as i32,
    );
    let ok = (10000..=30000).contains(&units)
        && w > 0
        && h > 0
        && w <= 4096
        && h <= 4096;
    if !RESOLVED.load(Ordering::SeqCst) {
        RESOLVED.store(true, Ordering::SeqCst);
        crate::log::log(&format!(
            "[cosmo-plus] hud overlay: zoom layout units={units} crop=({l},{t},{w},{h}) \
             surface=({surface_w},{surface_h}) -> {}",
            if ok { "VALID (camera-relative crop)" } else { "INVALID (identity mapping)" }
        ));
    }
    if ok {
        UNITS.store(units, Ordering::Relaxed);
        CROP_LEFT.store(l as u32, Ordering::Relaxed);
        CROP_TOP.store(t as u32, Ordering::Relaxed);
        CROP_W.store(w as u32, Ordering::Relaxed);
        CROP_H.store(h as u32, Ordering::Relaxed);
    } else {
        // Identity fallback: whole surface, 100%.
        UNITS.store(10000, Ordering::Relaxed);
        CROP_LEFT.store(0, Ordering::Relaxed);
        CROP_TOP.store(0, Ordering::Relaxed);
    }
}

/// Refresh the per-frame zoom view. Called from the draw hook BEFORE every
/// overlay so all of them share one consistent crop read for the frame.
pub(crate) unsafe fn refresh() {
    refresh_zoom_view(0, 0);
}

/// The current crop offset relative to the camera (world px).
pub(crate) fn crop_offset() -> (i32, i32) {
    (
        CROP_LEFT.load(Ordering::Relaxed) as i32,
        CROP_TOP.load(Ordering::Relaxed) as i32,
    )
}

/// Minimal head of the vanilla `graphics::Bitmap` (width/height, u16 each).
#[repr(C)]
struct BitmapHead {
    width: u16,
    height: u16,
}

/// Signature (masked) of the fork's `draw_deferred_screen_text` BODY:
/// `cmp byte [imm],0; je ret; mov byte [imm],0; call draw_engine_screen_text;
/// ret` — the guarded `g_screen_text_pending` flag with the SAME imm32 in the
/// cmp and the mov (the static lives in gptp's .data). This identifies the
/// function itself, not a call site, so caller-shape differences between
/// builds never matter: the call site is then found by scanning for the one
/// E8 rel32 that targets this body. The call runs AFTER `g_zoom.scale_world`
/// resamples the world onto the presented screen buffer (grid.cpp), so
/// chaining it draws at plain screen coordinates on the layer SC2 puts its
/// HUD text — fixed position and fixed size at every zoom.
const BODY_SIG: &[u8] = &[
    0x80, 0x3D, 0x00, 0x00, 0x00, 0x00, 0x00, // cmp byte [imm32], 0
    0x74, 0x00, // je +imm8
    0xC6, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00, // mov byte [imm32], 0
];
const BODY_MASK: &[u8] = &[
    b'x', b'x', b'?', b'?', b'?', b'?', b'x', b'x', b'x', b'x', b'x', b'?', b'?', b'?', b'?',
    b'x',
];

/// The original call target (`draw_deferred_screen_text`), resolved live from
/// the call's rel32 (relocations never matter this way).
static CHAIN: AtomicU32 = AtomicU32::new(0);

/// Resolved status for the boot report.
pub(crate) fn status() -> String {
    let chain = CHAIN.load(Ordering::SeqCst);
    if chain == 0 {
        "disabled (deferred-screen-text call site not found in gptp.qdp)".to_string()
    } else {
        format!("armed (chain 0x{chain:08X})")
    }
}

/// Whether the screen-corner overlays draw on the deferred screen layer this
/// frame: the hook is armed AND the view is magnified (the fork skips the
/// call site at 100%, where the world pass draws instead).
pub(crate) fn screen_layer_active() -> bool {
    CHAIN.load(Ordering::SeqCst) != 0 && UNITS.load(Ordering::Relaxed) != 10000
}

/// Install both hud mechanisms: the zoom statics resolution (per-frame crop
/// reads) and the deferred-screen-text hook (zoom-independent screen
/// overlays).
pub(crate) fn install() {
    install_zoom_statics();
    unsafe {
        install_deferred_site();
    }
}

/// Scan gptp.qdp for the fork's draw_deferred_screen_text BODY, then find the
/// single call targeting it, resolve the original target from the live rel32
/// (relocations never matter this way), and retarget that call to the
/// wrapper. Resolving the FUNCTION first — by its unique guarded-flag
/// machine code — makes the caller's exact shape irrelevant, so this works
/// across builds where the previous call-site signature matched the wrong
/// instruction sequence.
unsafe fn install_deferred_site() {
    {
        let Some((base, size)) = crate::zoom::gptp_module() else {
            crate::log::log("[cosmo-plus] hud overlay: gptp.qdp not loaded; disabled");
            return;
        };
        // 1. Find the function body. Two encoding variants: the bool flag
        //    stored as a byte (C6) or as a dword (C7) — the same imm32 in the
        //    guard compare and the clear store is the anchor either way.
        let mut body: Option<usize> = None;
        for store_op in [0xC6u8, 0xC7u8] {
            let sig: Vec<u8> = [
                0x80, 0x3D, 0, 0, 0, 0, 0x00, // cmp byte [imm32], 0
                0x74, 0, // je +imm8
                store_op, 0x05, 0, 0, 0, 0, // mov byte/dword [imm32], 0 (C7 adds 00 00 00 00)
            ]
            .into();
            let mut mask: Vec<u8> = vec![b'x'; sig.len()];
            mask[2..6].fill(b'?');
            mask[7] = b'?';
            mask[9..13].fill(b'?');
            if store_op == 0xC7 {
                // mov dword [imm32], 0 — 4 extra immediate bytes, all fixed.
                let mut sig = sig;
                sig.extend_from_slice(&[0, 0, 0, 0]);
                mask.extend_from_slice(&[b'x'; 4]);
                if let Some(off) = crate::state::scan(base, size, &sig, &mask) {
                    body = Some(off);
                    break;
                }
            } else if let Some(off) = crate::state::scan(base, size, &sig, &mask) {
                body = Some(off);
                break;
            }
        }
        let Some(body_off) = body else {
            crate::log::log(
                "[cosmo-plus] hud overlay: draw_deferred_screen_text body not found in \
                 gptp.qdp; screen-fixed overlays fall back to the world pass (zoom-scaled)",
            );
            return;
        };
        // `state::scan` returns a live absolute address; the offset within the
        // loaded image is the RVA. Call-site rel32s are computed in VA space,
        // whose offsets equal RVAs for intra-module calls (the image base
        // cancels), and reading them from LOADED memory picks up any
        // relocation the loader applied.
        let body_rva = body_off - base;
        let live_fn = body_off;

        // 2. Find the call site: the E8 rel32 targeting the function.
        let mut site: Option<usize> = None;
        let mut count = 0u32;
        for i in 0..size.saturating_sub(5) {
            if std::ptr::read((base + i) as *const u8) != 0xE8 {
                continue;
            }
            let rel = std::ptr::read((base + i + 1) as *const i32) as i64;
            let target = i as i64 + 5 + rel;
            if target >= 0 && target < size as i64 && target == body_rva as i64 {
                count += 1;
                if site.is_none() {
                    site = Some(i);
                }
            }
        }
        let Some(site_rva) = site else {
            crate::log::log(
                "[cosmo-plus] hud overlay: no caller of draw_deferred_screen_text found; \
                 falling back to the world pass",
            );
            return;
        };
        if count > 1 {
            crate::log::log(&format!(
                "[cosmo-plus] hud overlay: {count} callers of the deferred text pass; \
                 using the first (gptp+0x{site_rva:X})"
            ));
        }
        let site = base + site_rva;
        let chain = live_fn as u32;
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
            "[cosmo-plus] hud overlay: hooked deferred screen text (body @ gptp+0x{body_rva:X}, \
             call @ gptp+0x{site_rva:X}, chain 0x{chain:08X}, patch={r}); screen overlays \
             draw zoom-independent"
        ));
        crate::state::hook("deferred-screen-text", site, chain as usize, my as usize);
    }
}

/// Replaces the original callee: chain it first (it draws the fork's message
/// log), then draw the screen-fixed overlays. At this moment
/// `*g_current_surface` is the presented screen buffer, so coordinates are
/// plain screen pixels.
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
    draw_screen_overlays((0, 0));
}

/// Draw every screen-corner overlay at the given origin. On the deferred
/// screen layer the origin is (0, 0) = plain screen pixels; on the world-pass
/// fallback it is `view_origin()` = camera + crop offset.
pub(crate) unsafe fn draw_screen_overlays(origin: (i32, i32)) {
    if crate::frame::frame_count() == 0 {
        return; // not in a game
    }
    let mut buf = [0u8; 16];
    let screen = origin == (0, 0);
    // Minimap-position labels first (they sit on the map panel).
    crate::nydus::draw_minimap(&mut buf, screen);
    crate::cam_labels::draw_minimap(&mut buf, screen);
    // HUD overlays: the idle-worker counter and the Tab subgroup indicator.
    crate::overlay::draw_idle_counter(origin);
    crate::overlay::draw_tab_indicator(origin);
    // The changes panel + hint, the larva entry feedback, the 2x production
    // queue and the bug-capture confirmation share the screen layer.
    crate::overlay::draw_changes(origin);
    crate::overlay::draw_capture_confirmation(origin);
    crate::larva_pick::draw(origin);
    crate::queue::draw(origin);
    draw_build_rev(origin);
/// Diagnostic probe (CM_PLUS_HUD_PROBE=1): markers at the candidate
/// transforms, for the screenshot-diff bisect.
    if crate::config::env_flag("CM_PLUS_HUD_PROBE") {
        probe_markers();
    }
}

/// Temporary on-screen build stamp: the 7-char git rev of the running plugin,
/// drawn under the idle counter. Visible proof of which build is loaded —
/// remove before the final release.
unsafe fn draw_build_rev(origin: (i32, i32)) {
    let font = crate::overlay::in_game_font();
    if font.is_null() {
        return;
    }
    crate::overlay::set_font_handle(font);
    crate::overlay::write_draw_text(0, 0, 0x7FFF, 0x7FFF);
    *(crate::overlay::G_PRINT_XY_ALIGNMENT as *mut u8) = 0x11;
    crate::overlay::get_color_handle(cosmo_plus_core::saturation::text_color::YELLOW);
    let rev = option_env!("COSMO_PLUS_GIT_REV").unwrap_or("unknown");
    let mut buf = [0u8; 16];
    buf.fill(0);
    let label = format!("build {}", &rev[..rev.len().min(7)]);
    let len = label.len().min(buf.len() - 1);
    buf[..len].copy_from_slice(&label.as_bytes()[..len]);
    crate::overlay::bw_print_xy(buf.as_ptr(), origin.0 + 8, origin.1 + 320);
    crate::overlay::set_font_handle(core::ptr::null_mut());
}

/// One-shot state log + per-frame markers at the candidate transforms
/// (CM_PLUS_HUD_PROBE=1): the player reports which marker sits at the
/// screen's top-left corner, which pins the correct coordinate model for
/// the build:
///   A = crop-relative  (camera + crop.left/top + offset) — the mod's model
///   B = camera-relative (camera + offset)                  — current overlay model
///   C = raw surface coords (identity)                      — broken fallback
/// A yellow box outlines the hypothesized visible region for A.
unsafe fn probe_markers() {
    let (cx, cy) = crate::overlay::camera();
    let units = UNITS.load(Ordering::Relaxed);
    let (cl, ct, cw, ch) = (
        CROP_LEFT.load(Ordering::Relaxed) as i32,
        CROP_TOP.load(Ordering::Relaxed) as i32,
        CROP_W.load(Ordering::Relaxed) as i32,
        CROP_H.load(Ordering::Relaxed) as i32,
    );
    static LOGGED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let frame = crate::frame::frame_count_safe();
    if LOGGED.load(std::sync::atomic::Ordering::Relaxed) == 0 || frame % 600 == 0 {
        LOGGED.store(1, std::sync::atomic::Ordering::Relaxed);
        crate::log::log(&format!(
            "[cosmo-plus] hud probe: frame={frame} camera=({cx},{cy}) units={units} \
             crop=({cl},{ct},{cw},{ch})"
        ));
    }

    // A: crop-relative (the mod's documented model).
    let (ax, ay) = (cx + cl + 8, cy + ct + 8);
    // B: camera-relative.
    let (bx, by) = (cx + 8, cy + 8);
    // C: raw surface.
    let (cx3, cy3) = (8, 8);

    // Yellow box around the hypothesized visible region (crop-relative).
    crate::overlay::get_color_handle(cosmo_plus_core::saturation::text_color::YELLOW);
    let box_side = b"+\0";
    for i in 0..cw.max(0).min(1280) / 8 {
        crate::overlay::bw_print_xy(box_side.as_ptr(), cx + cl + i * 8, cy + ct);
        crate::overlay::bw_print_xy(box_side.as_ptr(), cx + cl + i * 8, cy + ct + ch);
    }
    for i in 0..ch.max(0).min(960) / 8 {
        crate::overlay::bw_print_xy(box_side.as_ptr(), cx + cl, cy + ct + i * 8);
        crate::overlay::bw_print_xy(box_side.as_ptr(), cx + cl + cw, cy + ct + i * 8);
    }

    crate::overlay::get_color_handle(cosmo_plus_core::saturation::text_color::WHITE);
    let a = b"A\0";
    let b = b"B\0";
    let c = b"C\0";
    crate::overlay::bw_print_xy(a.as_ptr(), ax, ay);
    crate::overlay::bw_print_xy(b.as_ptr(), bx, by);
    crate::overlay::bw_print_xy(c.as_ptr(), cx3, cy3);
}
