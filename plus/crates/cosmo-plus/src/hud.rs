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

/// The current crop fields (world px, camera-relative).
pub(crate) fn crop_left() -> i32 { CROP_LEFT.load(Ordering::Relaxed) as i32 }
pub(crate) fn crop_top() -> i32 { CROP_TOP.load(Ordering::Relaxed) as i32 }
pub(crate) fn crop_w() -> i32 { CROP_W.load(Ordering::Relaxed) as i32 }
pub(crate) fn crop_h() -> i32 { CROP_H.load(Ordering::Relaxed) as i32 }

/// The current crop offset relative to the camera (world px). At 100% zoom
/// the crop is meaningless — the whole surface is visible — and the release
/// build leaves stale values there, so it is forced to (0,0).
pub(crate) fn crop_offset() -> (i32, i32) {
    if zoom_units() == 10000 {
        return (0, 0);
    }
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

/// The overlay position scale for the CURRENT layer: on the screen layer the
/// surface is the presentation buffer (~2x the 640x480-authored space on the
/// widescreen build — the live minimap rect is ~2x the authored 127px), so
/// all authored positions multiply by this. The world pass is 1:1.
static SCREEN_SCALE: AtomicU32 = AtomicU32::new(1000);

/// The overlay position scale (1.0 on the world pass; ~2.0 on the screen
/// layer of the widescreen build).
pub(crate) fn screen_scale() -> f32 {
    (SCREEN_SCALE.load(Ordering::Relaxed) as f32) / 1000.0
}

unsafe fn refresh_screen_scale(screen: bool) {
    if !screen {
        SCREEN_SCALE.store(1000, Ordering::Relaxed);
        return;
    }
    if let Some(rect) = crate::state::minimap_rect() {
        let w = rect[2] - rect[0];
        if (40..=640).contains(&w) {
            // Authored minimap width is 127; anything wider means the
            // presentation is scaled up.
            let scale = (w as f32 / 127.0).clamp(1.0, 4.0);
            SCREEN_SCALE.store((scale * 1000.0) as u32, Ordering::Relaxed);
            return;
        }
    }
    SCREEN_SCALE.store(1000, Ordering::Relaxed);
}

/// Which layer the screen-corner overlays draw on this frame:
///   * magnified (200%/300%): the re-chained DEFERRED screen-text pass —
///     post resample, on the presented buffer, fixed position and size;
///   * 100%: the world surface IS the presented surface; the world pass at
///     camera + (0,0) crop is exact.
pub(crate) fn screen_layer_active() -> bool {
    CHAIN.load(Ordering::SeqCst) != 0 && zoom_units() != 10000
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
                 gptp.qdp; trying the engine's screen-text call site",
            );
            install_via_screen_text_call();
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
            if target >= 0 && target < size as i64 {
                if target == body_rva as i64 {
                    count += 1;
                    if site.is_none() {
                        site = Some(i);
                    }
                } else if std::ptr::read((base + target as usize) as *const u8) == 0xE9 {
                    // Incremental-link jmp thunk: the call targets a 5-byte
                    // jump into the real body.
                    let jrel = std::ptr::read((base + target as usize + 1) as *const i32) as i64;
                    let ultimate = target + 5 + jrel;
                    if ultimate >= 0 && ultimate < size as i64 && ultimate == body_rva as i64 {
                        count += 1;
                        if site.is_none() {
                            site = Some(i);
                        }
                    }
                }
            }
        }
        let Some(site_rva) = site else {
            crate::log::log(
                "[cosmo-plus] hud overlay: no caller of draw_deferred_screen_text found; \
                 trying the engine's screen-text call site",
            );
            install_via_screen_text_call();
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

/// Fallback strategy: chain the engine's own screen-text call site
/// (`draw_screen_text`, 0x004BD614). The fork call_patches this site at load
/// (the on-disk exe still targets vanilla 0x48CF60), so the live rel32 is
/// read and chained — whether it points at the fork's function or the
/// vanilla pass, our overlays draw right after the message log, on the same
/// surface. If the build defers the text pass to after the zoom resample,
/// this surface is pre-resample and the overlays may be clobbered — the
/// deferred-body strategy above is the preferred path; this one is the
/// empirical fallback.
unsafe fn install_via_screen_text_call() {
    const SITE: usize = 0x004BD614;
    let b = std::ptr::read(SITE as *const u8);
    if b != 0xE8 {
        crate::log::log(&format!(
            "[cosmo-plus] hud overlay: screen-text call site holds 0x{b:02X} (expected E8); \
             falling back to the world pass"
        ));
        return;
    }
    let rel = std::ptr::read((SITE + 1) as *const i32);
    let chain = (SITE as u32).wrapping_add(5).wrapping_add(rel as u32);
    CHAIN.store(chain, Ordering::SeqCst);

    let my = wrapper as *const () as usize as u32;
    let new_rel = my.wrapping_sub(SITE as u32 + 5) as i32;
    let patch: [u8; 5] = [
        0xE8,
        new_rel as u8,
        (new_rel >> 8) as u8,
        (new_rel >> 16) as u8,
        (new_rel >> 24) as u8,
    ];
    let api = crate::plus::api();
    let r = (api.write_exe_memory)(SITE, patch.as_ptr(), patch.len());
    crate::log::log(&format!(
        "[cosmo-plus] hud overlay: chained the engine's screen-text call @ 0x{SITE:08X} \
         (chain 0x{chain:08X}, patch={r}); overlays draw with the message log"
    ));
    crate::state::hook("screen-text-call", SITE, chain as usize, my as usize);

    // The chained function (the fork's draw_screen_text) DEFERS its own
    // drawing while magnified — the live dump of the user's build shows
    // `cmp dword [units],0x2710; jbe direct; mov byte [pending],1; ret`.
    // The overlays therefore belong on the fork's DEFERRED pass (post
    // resample, on the presented buffer). Derive the pending flag's live
    // address from the chained function's own bytes, locate the deferred
    // body (cmp/mov on the same flag), find its caller (thunk-aware) and
    // re-chain there. Any failure keeps the exe-call chain (the text still
    // survives via the partial-frame resampler's dirty-cell path).
    if discover_deferred_pass(chain as usize) {
        crate::log::log(
            "[cosmo-plus] hud overlay: re-chained to the deferred screen-text pass \
             (post-resample, presented buffer)",
        );
    }
}

/// From the chained draw_screen_text's live bytes, locate the deferred pass:
/// the pending flag's address comes from the `mov byte [flag],1` right after
/// the `cmp [units],0x2710; jbe` guard. The deferred function guards the
/// flag: `cmp byte [flag],0; je; mov byte [flag],0; call draw_engine; ret`.
/// Then find its single caller (following jmp thunks) and re-chain.
unsafe fn discover_deferred_pass(chain_fn: usize) -> bool {
    let fail = |step: &str| {
        crate::log::log(&format!(
            "[cosmo-plus] hud overlay: deferred rediscovery failed at {step}"
        ));
        false
    };
    let b = |off: usize| std::ptr::read((chain_fn + off) as *const u8);
    if b(0) != 0x81 || b(1) != 0x3D {
        return fail("signature (chain fn is not `cmp dword [units],...`)");
    }
    let units_imm = std::ptr::read((chain_fn + 2) as *const u32);
    let cmp_val = std::ptr::read((chain_fn + 6) as *const u16);
    if units_imm == 0 || cmp_val != 0x2710 {
        return fail(&format!(
            "units/cmp check (units_imm 0x{units_imm:08X}, cmp 0x{cmp_val:04X})"
        ));
    }
    // The `mov byte [flag],1` (C6 05 imm32 01) sits after the jbe rel32 (+8).
    let mut flag = 0usize;
    for off in 10..=16 {
        if b(off) == 0xC6 && b(off + 1) == 0x05 {
            flag = std::ptr::read((chain_fn + off + 2) as *const u32) as usize;
            break;
        }
    }
    if flag == 0 {
        return fail("pending-flag locator (no C6 05 after the jbe)");
    }
    crate::log::log(&format!(
        "[cosmo-plus] hud overlay: deferred rediscovery: pending flag 0x{flag:08X}"
    ));
    // Deferred body: cmp byte [flag],0; je ...; mov byte [flag],0 ...
    let Some((base, size)) = crate::zoom::gptp_module() else {
        return fail("gptp module lookup");
    };
    let mut body: Option<usize> = None;
    for i in 0..size.saturating_sub(24) {
        let site = base + i;
        if std::ptr::read(site as *const u8) != 0x80
            || std::ptr::read((site + 1) as *const u8) != 0x3D
        {
            continue;
        }
        let imm = std::ptr::read((site + 2) as *const u32) as usize;
        if imm != flag || std::ptr::read((site + 6) as *const u8) != 0x00 {
            continue;
        }
        // je follows; then a C6 05 with the same imm within 24 bytes.
        for off in 7..24 {
            if std::ptr::read((site + off) as *const u8) == 0xC6
                && std::ptr::read((site + off + 1) as *const u8) == 0x05
                && std::ptr::read((site + off + 2) as *const u32) as usize == flag
            {
                body = Some(site);
                break;
            }
        }
        if body.is_some() {
            break;
        }
    }
    let Some(body) = body else {
        return fail("deferred body scan (guard on the pending flag not found)");
    };
    crate::log::log(&format!(
        "[cosmo-plus] hud overlay: deferred rediscovery: body 0x{body:08X}"
    ));
    // Caller: the E8 targeting the body (through a jmp thunk if needed).
    let mut site: Option<usize> = None;
    for i in 0..size.saturating_sub(5) {
        let s = base + i;
        if std::ptr::read(s as *const u8) != 0xE8 {
            continue;
        }
        let rel = std::ptr::read((s + 1) as *const i32) as i64;
        let t = s as i64 + 5 + rel;
        if t == body as i64 {
            site = Some(s);
            break;
        }
        if t > 0 && t < (base + size) as i64 && std::ptr::read(t as *const u8) == 0xE9 {
            let jrel = std::ptr::read((t as usize + 1) as *const i32) as i64;
            if t + 5 + jrel == body as i64 {
                site = Some(s);
                break;
            }
        }
    }
    let Some(site) = site else {
        return fail("caller scan (no call targets the body)");
    };
    CHAIN.store(body as u32, Ordering::SeqCst);
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
        "[cosmo-plus] hud overlay: deferred pass @ 0x{site:08X} (body 0x{body:08X}, \
         flag 0x{flag:08X}, patch={r})"
    ));
    crate::state::hook("deferred-screen-text", site, body as usize, my as usize);
    true
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
    static CALLS: AtomicU32 = AtomicU32::new(0);
    let n = CALLS.fetch_add(1, Ordering::Relaxed);
    if n < 3 {
        let surf = std::ptr::read(0x006C_F4A8 as *const usize);
        crate::log::log(&format!(
            "[cosmo-plus] hud wrapper: frame={} surface=0x{surf:08X} units={} scale={:.2} \
             crop=({},{},{},{})",
            crate::frame::frame_count_safe(),
            zoom_units(),
            screen_scale(),
            crop_left(),
            crop_top(),
            crop_w(),
            crop_h(),
        ));
    }
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
    refresh_screen_scale(screen);
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
    let sc = screen_scale();
    crate::overlay::bw_print_xy(
        buf.as_ptr(),
        origin.0 + (8.0 * sc) as i32,
        origin.1 + (320.0 * sc) as i32,
    );
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
