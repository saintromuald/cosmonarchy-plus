//! Global production queue with config-driven icon size.
//!
//! The mod's `StatResource` draws a small production queue (28px slots) in the
//! top bar behind the "Show Global Queue" options checkbox. The user wants
//! **twice the size**, selectable strictly through the config file:
//!
//!   `CM_PLUS_QUEUE_ICON_SCALE=2` (default off)
//!
//! When armed:
//!   * the mod's small production slot widgets are hidden every frame from the
//!     draw pass (the mod's 20-tick timer re-shows them; the hide runs after
//!     the timer phase and before the dialog repaint, so it wins — verified
//!     with headless screenshots). The hide is draw-pass-only: dialog memory
//!     writes must never happen in the sim tick, which runs during rollback
//!     re-simulation;
//!   * the plugin draws its own 2x queue for the LOCAL player, camera-relative
//!     at the top-left of the view (below the idle badge, above the Tab
//!     indicator), two rows of up to 16 slots each. Camera-relative because the
//!     top bar region cannot be reached from the world draw pass (the mod's HUD
//!     dialogs repaint over the world blit — verified; the project abandoned
//!     screen-buffer dialog hooks as fragile).
//!
//! Data: the mod's `g_production` arrays (`ProductionArrays` in
//! `hooks/unit_stats/supply_hooks.h`: `types[8][16] u16`, `timers[8][16] u16`,
//! `counts[8][16] u8`), filled by the mod's `stat_res_get_production` and
//! zeroed by `stat_res_refresh_production_arrays`. The runtime address is
//! resolved from the naked-asm signature of the refresh function in gptp.qdp:
//!
//!   `B8 E4 00 E4 00 BF <imm32>`   (mov eax,0xE400E4; mov edi, g_production)
//!
//! verified against both shipped builds (Prerelease RVA 0x126B61, Original
//! RVA 0xA2D51). The live imm is loader-relocated, so it is read lazily at
//! first draw and accepted only inside the module's writable committed span
//! (base-delta fallback otherwise, like `cam_labels`).
//!
//! Rendering is read-only with respect to simulation (draw pass only), so it
//! is determinism- and rollback-neutral. Icons are the engine's loaded
//! command-card icons (`g_cmdicons_grp` → `unit\cmdbtns\cmdicons.grp`,
//! indexed by the unit dat icon field — the same recognisable icon set the
//! command card uses; the mod's queue uses a different GRP, so the look is
//! similar but not pixel-identical — documented). The count text uses the
//! overlay font; the progress bar replicates the mod's math and guards.
//!
//! The queue is suppressed while the changes panel or the larva entry window
//! is open (they share the top-left corner), and does nothing in replays or
//! when the local player id is out of range.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use core::mem::size_of;

/// `g_statres_dialog` (bw::Dialog*) — the mod's StatResource root dialog
/// (the mod sets this vanilla global to its dialog; the exe's countdown code
/// walks it for index 0xFFF6).
const G_STATRES_DIALOG: usize = 0x0068C234;
/// `*g_draw_color_index` (char*) — colour for `draw_box` fills.
const G_DRAW_COLOR_INDEX: usize = 0x006CF4AC;
/// `helper::draw_box(u16 x, u16 y, u16 w, u16 h)` — filled rect, colour from
/// `*g_draw_color_index` (the mod's statres uses exactly this for its bar).
const ADDR_DRAW_BOX: usize = 0x004E1D20;
static DRAW_BOX_ADDR: u32 = ADDR_DRAW_BOX as u32;

/// The mod's production slot widgets: stored bounds are authored container
/// coords (children are drawn/hit-tested root-relative), 28×32 at the exact
/// 30px pitch packing leftward from ProductionCursorX = 872, rows at
/// FirstRowY + 30r. Anything else matching width/height is excluded by the
/// pitch/row checks.
const SLOT_W: i16 = 28;
const SLOT_H: i16 = 32;
const SLOT_FIRST_X: i16 = 872 - 30 * 16; // slot 15's left edge (392)
const SLOT_LAST_X: i16 = 872 - 30; // slot 0's left edge (842)
const SLOT_PITCH: i16 = 30;
const ROW_PITCH: i16 = 30;
const ROW_FIRST_Y: i16 = 4;
const ROWS: i16 = 8;

/// `DialogFlags::Visible` (the codebase already treats 0x8 as the visible bit
/// for the game-menu patch).
const DLG_VISIBLE: u32 = 0x8;

/// The production array layout (see supply_hooks.h).
const PLAYER_LIMIT: usize = 8;
const ROW_SIZE: usize = 16;
const TYPES_OFF: usize = 0;
const TIMERS_OFF: usize = PLAYER_LIMIT * ROW_SIZE * 2; // 256
const COUNTS_OFF: usize = TIMERS_OFF + PLAYER_LIMIT * ROW_SIZE * 2; // 768

/// `CM_PLUS_QUEUE_ICON_SCALE` armed state: 2 = plugin queue (2x icons, mod's
/// slots hidden); anything else = off (the mod's queue is untouched).
static ARMED: AtomicBool = AtomicBool::new(false);

/// Resolved `g_production` base (0 = not resolved yet).
static PROD_BASE: AtomicUsize = AtomicUsize::new(0);
/// Whether the resolution already failed loudly once (never rescan per frame).
static RESOLVE_FAILED: AtomicBool = AtomicBool::new(false);

/// Resolved status for the boot report.
pub(crate) fn status() -> String {
    if !ARMED.load(Ordering::SeqCst) {
        "off (CM_PLUS_QUEUE_ICON_SCALE unset; the mod's queue is untouched)".to_string()
    } else {
        "armed (2x icons; the mod's production slots are hidden)".to_string()
    }
}

/// Config resolution (one-time at install, re-armed on reload).
pub(crate) fn install() {
    rearm();
}

pub(crate) fn rearm() {
    let scale = crate::config::env("CM_PLUS_QUEUE_ICON_SCALE")
        .map(|v| v.trim().to_ascii_lowercase())
        .unwrap_or_default();
    let armed = scale == "2" || scale == "2x" || scale == "double" || scale == "big";
    ARMED.store(armed, Ordering::SeqCst);
    crate::log::log(&format!(
        "[cosmo-plus] queue: CM_PLUS_QUEUE_ICON_SCALE={scale:?} -> {}",
        if armed { "2x queue armed (mod slots hidden)" } else { "off" }
    ));
}

/// Scan gptp.qdp for the refresh-function signature and resolve `g_production`.
unsafe fn resolve_production() -> bool {
    if PROD_BASE.load(Ordering::SeqCst) != 0 {
        return true;
    }
    if RESOLVE_FAILED.load(Ordering::SeqCst) {
        return false; // already failed loudly; never rescan per frame
    }
    let Some((base, size)) = crate::zoom::gptp_module() else {
        fail_once("gptp.qdp not loaded");
        return false;
    };
    const SIG: &[u8] = b"\xB8\xE4\x00\xE4\x00\xBF";
    let mask = vec![b'x'; SIG.len()];
    // state::scan returns an ABSOLUTE address (base + offset) — see its use in
    // cam_labels.rs (`off - base`). Do not add the module base again.
    let Some(off) = crate::state::scan(base, size, SIG, &mask) else {
        fail_once("g_production signature not found in gptp.qdp");
        return false;
    };
    let imm_addr = off + 6;
    let imm = std::ptr::read(imm_addr as *const u32) as usize;
    let candidate = if imm >= base && imm < base + size {
        imm // live (relocated) value
    } else {
        // Not relocated yet (or a different loader): apply the base delta.
        (base as u32)
            .wrapping_add(imm.wrapping_sub(0x1000_0000) as u32)
            as usize
    };
    // The array must sit in the module's writable committed span.
    let needed = COUNTS_OFF + PLAYER_LIMIT * ROW_SIZE;
    if candidate < base
        || candidate + needed > base + size
        || !crate::overlay::readable_ptr(candidate as *const u8, needed)
    {
        fail_once(&format!("g_production resolved to 0x{candidate:08X} (out of range)"));
        return false;
    }
    PROD_BASE.store(candidate, Ordering::SeqCst);
    crate::log::log(&format!(
        "[cosmo-plus] queue: g_production at 0x{candidate:08X} (signature @ gptp+0x{:X}, imm 0x{imm:08X})",
        off - base,
    ));
    true
}

fn fail_once(why: &str) {
    if !RESOLVE_FAILED.swap(true, Ordering::SeqCst) {
        crate::log::log(&format!("[cosmo-plus] queue: disabled — {why}"));
    }
}

/// Hide the mod's production slot widgets. Called from the DRAW pass every
/// frame — NEVER from the sim tick, which also runs during rollback
/// re-simulation (dialog-flag writes do not belong in the resim path). The
/// mod's 20-tick `StatResource::update` re-shows them, so a one-shot hide
/// would come back within a second; the draw-pass hide runs after the timer
/// phase and before the dialog repaint, so it wins.
pub(crate) unsafe fn poll_hide() {
    if !ARMED.load(Ordering::SeqCst) {
        return;
    }
    if crate::frame::frame_count() == 0 {
        return;
    }
    if crate::antimeta::is_replay() {
        return; // replays keep the mod's own per-player queue
    }
    let root = *(G_STATRES_DIALOG as *const *mut BinDlg);
    if root.is_null() {
        return;
    }
    let mut dlg = root;
    let mut hidden = 0u32;
    let mut guard = 0;
    while guard < 4096 {
        if !crate::overlay::readable_ptr(dlg as *const u8, size_of::<BinDlg>()) {
            break;
        }
        let b = (*dlg).bounds;
        // Bounds are inclusive ltrb (w = right - left + 1).
        if b[2] - b[0] + 1 == SLOT_W
            && b[3] - b[1] + 1 == SLOT_H
            && b[0] >= SLOT_FIRST_X
            && b[0] <= SLOT_LAST_X
            && (b[0] - SLOT_FIRST_X) % SLOT_PITCH == 0
            && b[1] >= ROW_FIRST_Y
            && b[1] <= ROW_FIRST_Y + (ROWS - 1) * ROW_PITCH
            && (b[1] - ROW_FIRST_Y) % ROW_PITCH == 0
        {
            (*dlg).flags &= !DLG_VISIBLE;
            hidden += 1;
        }
        let next = (*dlg).next;
        if next.is_null() {
            break;
        }
        dlg = next;
        guard += 1;
    }
    // Log the first walk that actually found widgets (the very first walk can
    // legitimately find none if the dialog tree is not built yet).
    static LOGGED: AtomicBool = AtomicBool::new(false);
    if hidden > 0 && !LOGGED.swap(true, Ordering::SeqCst) {
        crate::log::log(&format!(
            "[cosmo-plus] queue: mod production slots hidden ({hidden} widget(s))"
        ));
    }
}

/// Minimal `BinDlg` (1.16.1) prefix — mirror of overlay.rs's; only the fields
/// the widget walk needs.
#[repr(C)]
struct BinDlg {
    next: *mut BinDlg,
    bounds: [i16; 4],
    bitmap: [u8; 8],
    text: *const u8,
    flags: u32,
    unk_1c: u32,
    index: u16,
    control_type: u16,
    graphic: u16,
    user: *mut core::ffi::c_void,
}

/// The local player's production row, read from `g_production`. `None` when
/// unresolved, out of range, or in a replay.
struct Row {
    types: [u16; ROW_SIZE],
    timers: [u16; ROW_SIZE],
    counts: [u8; ROW_SIZE],
    slots: usize, // active slots (sentinel-terminated)
}

unsafe fn read_row() -> Option<Row> {
    if !resolve_production() {
        return None;
    }
    if crate::antimeta::is_replay() {
        return None; // the mod already draws every player's queue in replays
    }
    let Some(local) = crate::idle_workers::local_player() else {
        return None;
    };
    if local >= PLAYER_LIMIT as u32 {
        return None;
    }
    let base = PROD_BASE.load(Ordering::SeqCst);
    let mut row = Row {
        types: [0; ROW_SIZE],
        timers: [0; ROW_SIZE],
        counts: [0; ROW_SIZE],
        slots: 0,
    };
    let player_off = local as usize * ROW_SIZE;
    for slot in 0..ROW_SIZE {
        let timer = std::ptr::read((base + TIMERS_OFF + (player_off + slot) * 2) as *const u16);
        if timer == 65535 {
            break; // sentinel: the mod's list terminator
        }
        row.types[slot] = std::ptr::read((base + TYPES_OFF + (player_off + slot) * 2) as *const u16);
        row.timers[slot] = timer;
        row.counts[slot] = std::ptr::read((base + COUNTS_OFF + player_off + slot) as *const u8);
        row.slots = slot + 1;
    }
    if row.slots == 0 {
        return None;
    }
    Some(row)
}

/// Draw the 2x queue (camera-relative, top-left of the view). Suppressed while
/// the changes panel or the larva entry window is open — they share the corner.
pub(crate) unsafe fn draw() {
    if !ARMED.load(Ordering::SeqCst) {
        return;
    }
    if crate::frame::frame_count() == 0 {
        return;
    }
    if crate::overlay::changes_visible() || crate::larva_pick::entry_active() {
        return;
    }
    // Test-only data override (CM_PLUS_QUEUE_TEST): draw the synthetic row
    // instead of the live arrays for the headless screenshot harness.
    let test_row: Option<Row> = test_override().map(|(types, timers, counts)| {
        let mut r = Row {
            types: [0; ROW_SIZE],
            timers: [0; ROW_SIZE],
            counts: [0; ROW_SIZE],
            slots: types.len().min(ROW_SIZE),
        };
        for i in 0..r.slots {
            r.types[i] = types[i];
            r.timers[i] = timers[i];
            r.counts[i] = counts[i];
        }
        r
    });
    let row = match test_row {
        Some(r) => r,
        None => {
            let Some(r) = read_row() else { return };
            r
        }
    };
    let font = crate::overlay::in_game_font();
    if font.is_null() {
        return;
    }
    let (cx, cy) = crate::overlay::view_origin();
    crate::overlay::set_font_handle(font);
    crate::overlay::write_draw_text(0, 0, 0x7FFF, 0x7FFF);
    *(crate::overlay::G_PRINT_XY_ALIGNMENT as *mut u8) = 0x11;

    const ICON: i32 = 48; // 2x the mod's ~24px slot icons
    const PITCH: i32 = 56;
    const ROW_Y0: i32 = 44; // below the idle badge, above the Tab indicator
    const ROW_Y1: i32 = 108;
    let mut buf = [0u8; 24];
    for (i, slot) in (0..row.slots).enumerate() {
        // Row 2 restarts at the left edge (8 slots per row at 56px pitch fits
        // a 640px view); continuing the x cursor would push slots 12+ off-view.
        let x = cx + 4 + (i % 8) as i32 * PITCH;
        let y = cy + if i < 8 { ROW_Y0 } else { ROW_Y1 };
        let unit = row.types[slot];
        let icon_id = bw_dat::UnitId(unit).icon();
        crate::icon::draw_unit_icon_scaled(icon_id, x, y, ICON, ICON);

        // Count (white, small font), drawn over the icon's corner.
        buf.fill(0);
        let label = format!("{}", row.counts[slot]);
        let len = label.len().min(buf.len() - 1);
        buf[..len].copy_from_slice(&label.as_bytes()[..len]);
        crate::overlay::get_color_handle(cosmo_plus_core::saturation::text_color::WHITE);
        crate::overlay::bw_print_xy(buf.as_ptr(), x + 2, y + 2);

        // Progress bar: the mod's guards replicated (time_cost == 0 →
        // instant/morph unit, no bar; remaining > time_cost → compacting
        // queue, no bar), 2px tall at 2x.
        let time_cost = bw_dat::UnitId(unit).build_time() as u32;
        let remaining = row.timers[slot] as u32;
        if time_cost != 0 && remaining <= time_cost {
            let progress = (time_cost - remaining) * (ICON as u32) / time_cost;
            if progress > 0 {
                std::ptr::write(G_DRAW_COLOR_INDEX as *mut u8, 117); // the mod's bar colour
                draw_box(x as u16, (y + ICON) as u16, progress as u16, 2);
            }
        }
    }
    crate::overlay::set_font_handle(core::ptr::null_mut());
}

/// `helper::draw_box` with the PUSHAD guard (custom register ABI).
#[inline(never)]
unsafe fn draw_box(x: u16, y: u16, w: u16, h: u16) {
    core::arch::asm!(
        "pushad",
        "push {h}",
        "push {w}",
        "push {y}",
        "push {x}",
        "call dword ptr [{func}]",
        "popad",
        x = in(reg) x as u32,
        y = in(reg) y as u32,
        w = in(reg) w as u32,
        h = in(reg) h as u32,
        func = sym DRAW_BOX_ADDR,
        options(nostack),
    );
}

/// Test-only override for the production data (`CM_PLUS_QUEUE_TEST`): a
/// synthetic `types/timers/counts` tuple drawn instead of the live arrays, so
/// the headless screenshot harness can verify the 2x rendering without real
/// production. The live arrays are NEVER written (the mod's refresh wipes
/// them anyway).
pub(crate) fn test_override() -> Option<(Vec<u16>, Vec<u16>, Vec<u8>)> {
    let v = crate::config::env("CM_PLUS_QUEUE_TEST")?;
    let parts: Vec<&str> = v.split(';').collect();
    if parts.len() != 3 {
        return None;
    }
    let parse = |s: &str, out: &mut Vec<u16>| -> bool {
        for p in s.split(',') {
            let p = p.trim();
            if p.is_empty() {
                continue;
            }
            match p.parse::<u16>() {
                Ok(n) => out.push(n),
                Err(_) => return false,
            }
        }
        true
    };
    let mut types = Vec::new();
    let mut timers = Vec::new();
    if !parse(parts[0], &mut types) || !parse(parts[1], &mut timers) {
        return None;
    }
    if types.len() != timers.len() {
        return None;
    }
    let mut counts = Vec::new();
    for p in parts[2].split(',') {
        let p = p.trim();
        if p.is_empty() {
            continue;
        }
        match p.parse::<u8>() {
            Ok(n) => counts.push(n),
            Err(_) => return None,
        }
    }
    if counts.len() != types.len() {
        return None;
    }
    Some((types, timers, counts))
}
