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
/// left/top/width/height). The two neighbours are candidates for other
/// member layouts; validation picks the real one.
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

/// Resolved status for the boot report.
pub(crate) fn status() -> String {
    match (ZOOM_UNITS_ADDR.load(Ordering::SeqCst), RESOLVED.load(Ordering::SeqCst)) {
        (0, _) => "disabled (gptp.qdp not loaded)".to_string(),
        (addr, false) => format!("pending (units @ 0x{addr:08X}, resolves at first draw)"),
        (addr, true) => format!("armed (zoom crop @ 0x{addr:08X})"),
    }
}

/// Resolve the runtime addresses of the zoom statics from the gptp module
/// base (`.data` statics are loader-relocated once; nothing re-patches them).
pub(crate) fn install() {
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

/// Map a screen position to world-surface coordinates for the current zoom
/// (identity at 100%). The zoom statics are re-read every frame (see
/// `refresh_zoom_view`); the addresses are loader-stable, the VALUES change
/// whenever the player zooms — the old once-cached crop went stale on the
/// first wheel-scroll and pushed screen-fixed overlays out of the visible
/// area.
pub(crate) unsafe fn screen_to_surface(sx: i32, sy: i32) -> (i32, i32) {
    let units = UNITS.load(Ordering::Relaxed).max(10000) as i32;
    let wx = CROP_LEFT.load(Ordering::Relaxed) as i32 + sx * units / 10000;
    let wy = CROP_TOP.load(Ordering::Relaxed) as i32 + sy * units / 10000;
    (wx, wy)
}

/// The current zoom percentage (100 = 100%), for size compensation: an overlay
/// that must keep a constant SCREEN size draws at `size * 10000 / zoom_units`
/// surface pixels, because the crop magnifies surface pixels onto the screen.
pub(crate) fn zoom_units() -> u32 {
    UNITS.load(Ordering::Relaxed).max(10000)
}

/// Re-read the live zoom statics (units + crop) once per frame, at the top of
/// the draw pass. The values change on every wheel-scroll; a torn read during
/// a transition is clamped by the sanity range and self-heals next frame.
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
        && w <= surface_w
        && h <= surface_h
        && l + w <= surface_w
        && t + h <= surface_h;
    if !RESOLVED.load(Ordering::SeqCst) {
        RESOLVED.store(true, Ordering::SeqCst);
        crate::log::log(&format!(
            "[cosmo-plus] hud overlay: zoom layout units={units} crop=({l},{t},{w},{h}) \
             surface=({surface_w},{surface_h}) -> {}",
            if ok { "VALID" } else { "INVALID (identity mapping)" }
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

/// Minimal head of the vanilla `graphics::Bitmap` (width/height, u16 each).
#[repr(C)]
struct BitmapHead {
    width: u16,
    height: u16,
}

/// Draw every screen-fixed overlay on the world surface, mapped into the
/// zoom crop. Called from the draw-proc hook (overlay.rs).
pub(crate) unsafe fn draw_overlays(surface: *mut core::ffi::c_void, _bounds: *mut core::ffi::c_void) {
    if crate::frame::frame_count() == 0 {
        return; // not in a game
    }
    let surf = surface as *const BitmapHead;
    let (mut w, mut h) = (640, 480);
    if !surf.is_null() {
        let (sw, sh) = ((*surf).width as i32, (*surf).height as i32);
        // The dims only validate the zoom layout; nonsense values just skip
        // the strict check (the mapping itself never needs them).
        if (0..=4096).contains(&sw) && (0..=4096).contains(&sh) {
            w = sw;
            h = sh;
        }
    }
    // Re-read the live zoom statics once per frame (the player may have zoomed
    // since last frame; the crop mapping must follow).
    refresh_zoom_view(w, h);
    let mut buf = [0u8; 16];
    // Minimap-position labels first (they sit on the map panel).
    crate::nydus::draw_minimap(&mut buf);
    crate::cam_labels::draw_minimap(&mut buf);
    // HUD badges: the SC2-style idle-worker counter and the Tab subgroup
    // indicator, both screen-fixed through the crop mapping.
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
/// M3 = anchored on the minimap rect.
unsafe fn probe_markers() {
    let rect = crate::state::minimap_rect().unwrap_or([6, 348, 133, 475]);
    let (cx, cy) = crate::overlay::camera();
    static LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !LOGGED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        crate::log::log(&format!(
            "[cosmo-plus] hud probe: rect={rect:?} camera=({cx},{cy}) units={}",
            UNITS.load(Ordering::Relaxed)
        ));
    }
    crate::overlay::get_color_handle(cosmo_plus_core::saturation::text_color::YELLOW);
    let m1 = b"M1\0";
    let m2 = b"M2\0";
    let m3 = b"M3\0";
    let (x1, y1) = screen_to_surface(8, 8);
    let (x2, y2) = screen_to_surface(8 + cx, 8 + cy);
    let (x3, y3) = screen_to_surface(rect[0] + 2, rect[1] + 2);
    crate::overlay::bw_print_xy(m1.as_ptr(), x1, y1);
    crate::overlay::bw_print_xy(m2.as_ptr(), x2, y2);
    crate::overlay::bw_print_xy(m3.as_ptr(), x3, y3);
}
