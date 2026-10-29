//! Saturation overlay: read-only per-node worker counts drawn in the **draw pass**.
//!
//! The overlay hooks the game draw proc at `0x004BD68D` (the operand patched by GPTP's
//! `graphics/draw_hook.cpp`), chains to whatever it previously pointed at, and then draws
//! two `assigned/optimal` rows above each visible resource node. Because the hook runs in
//! the draw pass — not the `StepObjects` simulation tick — rollback re-simulation never
//! executes it, and nothing here writes simulation state (see `docs/measurements/overlay.md`).
//!
//! Text is drawn with the 1.16.1 low-level primitives (`bw_print_xy` / `set_font_handle` /
//! `get_color_handle`). `bw_print_xy` targets `*g_current_surface` (`0x006CF4A8`), which is
//! the world buffer while the mod is magnified (its default 200% zoom), so drawing at
//! `world - camera` coordinates — the same transform the mod's `ON_MAP` shapes use — is
//! magnified correctly by `scale_world` with no zoom math of our own.

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::Mutex;

use once_cell::sync::Lazy;

use bw_dat::structs::Unit;

use cosmo_plus_core::saturation;
use cosmo_plus_core::saturation::text_color;

// ---------------------------------------------------------------------------
// Fixed 1.16.1 addresses (BW/bwdata.h + bwhelper.h).
// ---------------------------------------------------------------------------
/// The game draw proc operand (`g_draw_game_proc`), patched by GPTP's draw_hook.cpp.
const DRAW_GAME_PROC_PTR: usize = 0x004BD68D;
/// `font_base` (`graphics::Font**`); `font_base[0]` is the small font
/// (`graphics::FONT_SMALL`), `font_base[1]` the standard in-game font.
const FONT_BASE: usize = 0x006CE0F4;
/// Which font the overlay labels use: 0 = small (`FONT_SMALL`, default), 1 = standard
/// in-game font (`CM_PLUS_OVERLAY_FONT=std` restores the previous look).
static FONT_INDEX: AtomicUsize = AtomicUsize::new(0);
/// `g_draw_text` (Box16: left/top/right/bottom, i16 each) — the text clip box.
const G_DRAW_TEXT: usize = 0x006CE0C8;
/// `g_print_xy_alignment` (u8).
pub(crate) const G_PRINT_XY_ALIGNMENT: usize = 0x006CE110;
/// `g_screen_x` / `g_screen_y` (`s32*`, i.e. the address of the camera value in world px).
const G_SCREEN_X_PTR: usize = 0x00628448;
const G_SCREEN_Y_PTR: usize = 0x00628470;
/// `g_player_table` — 12 × `Player` (0x24 bytes), `race` at offset 0x09.
const G_PLAYER_TABLE: usize = 0x0057EEE0;
const PLAYER_STRUCT_SIZE: usize = 0x24;
const PLAYER_RACE_OFFSET: usize = 0x09;

/// A node further than this from any depot is not part of a working base; showing an
/// "optimum" for it (20–30 workers) is noise, so it is skipped.
const SERVICE_RADIUS_PX: i64 = 480; // 15 tiles

const ADDR_BW_PRINT_XY: usize = 0x004202B0;
const ADDR_SET_FONT_HANDLE: usize = 0x0041FB30;
const ADDR_GET_COLOR_HANDLE: usize = 0x0041F610;

/// The draw proc we chain to (read at install time; GPTP's draw_hook since `gptp.qdp`
/// loads before `zz-cosmo-plus.qdp`).
static OLD_DRAW_PROC: AtomicUsize = AtomicUsize::new(0);
/// Whether the overlay is enabled (resolved once at install).
static ENABLED: AtomicBool = AtomicBool::new(false);

/// Whether the saturation overlay switch resolved to on.
pub(crate) fn enabled() -> bool {
    ENABLED.load(Ordering::SeqCst)
}
/// Test-only cap on the per-node optimum (from `CM_PLUS_SATURATION_OPT_CAP`), so the
/// white->race-colour flip can be forced without waiting for a fully-saturated base.
static OPT_CAP: AtomicUsize = AtomicUsize::new(usize::MAX);
/// Whether to write a periodic per-node dump to the log (from `CM_PLUS_SATURATION_LOG=1`).
static LOG_DUMP: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------------------
// "Changes vs OG" tip panel (Pause/Break by default; CM_PLUS_CHANGES_KEY).
// ---------------------------------------------------------------------------
/// Line-by-line, player-facing differences from stock Cosmonarchy. Keep these
/// short: they are drawn with the in-game font, one line each.
const CHANGES_LINES: &[&str] = &[
    "Workers: 3 per mineral patch and 3 per gas node (flat, no diminishing)",
    "Gas: tier 1-3 cost less and build faster",
    "Advanced workers: +25 gas, much shorter build times",
    "Ultraload: +50% move speed, 20-tile depot radius",
    "Zoom: player floor 100% instead of the default 200%",
    "Smart Tab: cycles caster/ability subgroups (replaces hide-terrain)",
    "Camera pan: Ctrl + WASD or arrow keys",
    "Idle workers: F9 selects the next idle worker (key configurable)",
    "Larva pick: Ctrl+Space, type a count, Enter (key configurable)",
    "Minimap: C1-C8 camera labels and nydus canal numbers",
    "Saturation counts above your own town halls and gas",
    "Lobby: Anti Meta (46) and Draft (47) game types",
    "Netcode: rollback host/join (armed by the launcher)",
    "Crash: Ctrl+F11 saves a bug bundle for reporting",
];
/// The in-game F10 menu dialog (`g_popup_dialog`), exposed by the game as a
/// BinDlg* global. Its Help control is relabelled to "Plus Changes" and its
/// interaction function replaced with ours - a real menu entry without
/// touching the dialog .bin or the engine UI.
const G_POPUP_DIALOG: usize = 0x006D5BF4;
const MENU_TEXT: &[u8] = b"pPlus Changes (\0";
const CHANGES_DEFAULT_KEY: u16 = 0x13; // VK_PAUSE
const CHANGES_HINT_FRAMES: u32 = 24 * 20; // ~20 s at 24 logic frames/s
static CHANGES_VISIBLE: AtomicBool = AtomicBool::new(false);
static CHANGES_KEY: AtomicU32 = AtomicU32::new(CHANGES_DEFAULT_KEY as u32);
static CHANGES_DOWN: AtomicBool = AtomicBool::new(false);
static CHANGES_HINT_FROM: AtomicU32 = AtomicU32::new(0);
static CHANGES_LAST_FRAME: AtomicU32 = AtomicU32::new(u32::MAX);

/// One resource node's display state.
struct NodeState {
    x: i32,
    y: i32,
    tier1: u32,
    tier2: u32,
    /// Flat worker capacity: 3 for a mineral patch, 3 for a gas node.
    capacity: u32,
    color: u8,
    /// Straight-line distance to the nearest depot (px), for the log.
    dist: u32,
    /// The owner player id (depot owner) used for the race colour.
    owner: u8,
    /// The owner's `RaceId` (for the log).
    race: u8,
    /// True for gas nodes (drawn per-node); mineral fields only feed the depot display.
    is_gas: bool,
}

/// One completed town hall's aggregate mineral-line display (SC2-style: the count is
/// drawn above the base building instead of on every mineral field).
struct DepotState {
    x: i32,
    y: i32,
    owner: u8,
    race: u8,
    tier1: u32,
    tier2: u32,
    /// Flat capacity: 3 workers × the mineral patches this depot serves.
    capacity: u32,
    color: u8,
}

/// Per-frame cache; recomputed only when the logic frame advances. Scratch vectors are
/// reused so the hot path allocates nothing.
struct Cache {
    frame: u32,
    nodes: Vec<NodeState>,
    depots: Vec<DepotState>,
    scratch_nodes: Vec<(*mut Unit, i32, i32, u16)>,
    scratch_depots: Vec<(u8, i32, i32, i32)>, // player, x, y, sprite ptr
    scratch_workers: Vec<(u8, *mut Unit)>,
    /// Last logic frame we wrote a node dump to `cosmo_plus.log` (OCR-free verification).
    last_log: u32,
}

// The raw `*mut Unit` pointers make `Cache` !Send by default. Access is serialised by the
// `Mutex` and the game loop is single-threaded, so it is sound to mark it `Send`.
unsafe impl Send for Cache {}

static CACHE: Lazy<Mutex<Cache>> = Lazy::new(|| {
    Mutex::new(Cache {
        frame: 0,
        nodes: Vec::new(),
        depots: Vec::new(),
        scratch_nodes: Vec::new(),
        scratch_depots: Vec::new(),
        scratch_workers: Vec::new(),
        last_log: 0,
    })
});

/// Install the draw-proc hook and log the functional-mover matrix (research note).
pub(crate) fn install() {
    let enabled = compute_enabled();
    ENABLED.store(enabled, Ordering::SeqCst);
    crate::log::log(&format!("[cosmo-plus] overlay: enabled={enabled}"));

    // Overlay label font: the small font (font_base[0], graphics::FONT_SMALL) by
    // default, the standard in-game font (font_base[1]) with CM_PLUS_OVERLAY_FONT=std.
    let idx = match crate::config::env("CM_PLUS_OVERLAY_FONT").as_deref() {
        Some(v) if v.eq_ignore_ascii_case("std") => 1,
        Some(v) if !v.eq_ignore_ascii_case("small") => {
            crate::log::log(&format!(
                "[cosmo-plus] overlay: unknown CM_PLUS_OVERLAY_FONT={v:?}; using small"
            ));
            0
        }
        _ => 0,
    };
    FONT_INDEX.store(idx, Ordering::SeqCst);
    crate::log::log(&format!(
        "[cosmo-plus] overlay: label font = font_base[{idx}] ({})",
        if idx == 0 { "small" } else { "std" }
    ));

    if let Some(v) = crate::config::env("CM_PLUS_SATURATION_OPT_CAP") {
        if let Ok(n) = v.parse::<u32>() {
            OPT_CAP.store(n as usize, Ordering::SeqCst);
            crate::log::log(&format!(
                "[cosmo-plus] overlay: TEST optimum cap = {n} (forces saturation)"
            ));
        }
    }

    LOG_DUMP.store(crate::config::env_flag("CM_PLUS_SATURATION_LOG"), Ordering::SeqCst);

    unsafe {
        let old = std::ptr::read(DRAW_GAME_PROC_PTR as *const u32);
        OLD_DRAW_PROC.store(old as usize, Ordering::SeqCst);
        let my = draw_hook as *const () as usize as u32;
        let api = crate::plus::api();
        let r = (api.write_exe_memory)(
            DRAW_GAME_PROC_PTR,
            (&my as *const u32) as *const u8,
            4,
        );
        crate::log::log(&format!(
            "[cosmo-plus] overlay: hooked game draw proc (old 0x{old:08X} -> new 0x{my:08X}, patch={r})"
        ));
        crate::state::hook("draw-game-proc", DRAW_GAME_PROC_PTR, old as usize, my as usize);
        // Ownership audit: the pointer we chained should live inside gptp.qdp.
        // Anything else means another plugin stacked a hook first (or the
        // build moved) — loud, not silent.
        match crate::zoom::gptp_module() {
            Some((base, size)) if (old as usize) >= base && (old as usize) < base + size => {
                crate::log::log("[cosmo-plus] overlay: draw proc owned by gptp.qdp (expected)");
            }
            Some((base, size)) => {
                crate::log::log(&format!(
                    "[cosmo-plus] overlay: WARNING draw proc 0x{old:08X} outside gptp.qdp \
                     (0x{base:08X}+{size:#x}) — another plugin hooked it first?"
                ));
            }
            None => {
                crate::log::log(
                    "[cosmo-plus] overlay: WARNING gptp.qdp not found; draw proc owner unknown",
                );
            }
        }
    }

    install_changes();
}

/// Resolve the master switch: `CM_PLUS_SATURATION` env override wins, otherwise
/// `plus.yml`'s `saturation.enabled` (defaulting to **on** when `plus.yml` exists).
fn compute_enabled() -> bool {
    if let Some(v) = crate::config::env("CM_PLUS_SATURATION") {
        let v = v.to_ascii_lowercase();
        return matches!(v.as_str(), "1" | "true" | "yes" | "on");
    }
    crate::plus::config()
        .map(|c| c.saturation.enabled)
        .unwrap_or(false)
}

/// Re-resolve the overlay font and saturation switch (in-place reload).
pub(crate) fn rearm() {
    let idx = match crate::config::env("CM_PLUS_OVERLAY_FONT").as_deref() {
        Some(v) if v.eq_ignore_ascii_case("std") => 1,
        _ => 0,
    };
    FONT_INDEX.store(idx, Ordering::SeqCst);
    ENABLED.store(compute_enabled(), Ordering::SeqCst);
    crate::log::log(&format!(
        "[cosmo-plus] overlay: reloaded (font index {idx}, enabled {})",
        ENABLED.load(Ordering::SeqCst)
    ));
}

/// Log which worker tiers have a functional mover (speed > 1 px/frame) per race. The six
/// placeholder tier-2 slots are immobile in this build; their rows still show the same
/// measured optimum (the truth of the model), but this note flags them for the reader.
///
/// Runs once, on the first frame (deferred until `data::init()` has set up `bw_dat`, so the
/// flingy speeds are real rather than zero).
static LOG_MOVERS: std::sync::Once = std::sync::Once::new();
fn log_functional_movers() {
    let mut notes: Vec<String> = Vec::new();
    let mut tier1_races: Vec<&str> = Vec::new();
    let mut tier2_races: Vec<&str> = Vec::new();
    for &(id, tier, race) in saturation::WORKER_TIERS {
        let speed = crate::data::worker_speed_px_per_frame(id).unwrap_or(0.0);
        if speed <= 1.0 {
            notes.push(format!("{race} id {id} (tier {tier}) is immobile (speed {speed:.3})"));
        } else if tier == 1 {
            tier1_races.push(race);
        } else {
            tier2_races.push(race);
        }
    }
    crate::log::log(&format!(
        "[cosmo-plus] overlay: functional movers — tier1 [{}], tier2 [{}]",
        tier1_races.join(", "),
        tier2_races.join(", ")
    ));
    for n in notes {
        crate::log::log(&format!("[cosmo-plus] overlay: {n}"));
    }
}

/// The draw-proc hook. Signature must match `DrawGameProc`:
/// `void __stdcall (graphics::Bitmap* surface, Bounds* bounds)`.
unsafe extern "stdcall" fn draw_hook(surface: *mut core::ffi::c_void, bounds: *mut core::ffi::c_void) {
    let old = OLD_DRAW_PROC.load(Ordering::SeqCst);
    if old != 0 {
        let old: unsafe extern "stdcall" fn(*mut core::ffi::c_void, *mut core::ffi::c_void) =
            core::mem::transmute(old);
        old(surface, bounds);
    }
    // Refresh the per-frame zoom view FIRST: every screen-corner overlay this
    // frame (larva entry, queue, changes panel, idle counter, Tab indicator)
    // anchors at view_origin() = camera + crop offset, and the crop can change
    // between frames (wheel zoom). One consistent read per frame.
    crate::hud::refresh();
    // Draft mode UI (independent of the saturation overlay switch).
    crate::draft::draw_panel();
    crate::draft::poll_pick_input();
    // Idle-worker hotkey (also independent of the saturation switch; needs the draw
    // hook, which is installed regardless).
    crate::idle_workers::poll();
    // Larva picker: fires the selection on a confirmed count, then draws its entry UI.
    crate::larva_pick::poll();
    crate::larva_pick::draw();
    // Smart Tab: rotate the selection and drive the engine's card at the active
    // subgroup here in the draw pass (UI-safe). Never in the sim step — see frame.rs.
    // apply_display runs BEFORE poll so a Tab in the same frame as a fresh
    // selection is normalised against it first (otherwise the first Tab after
    // a fast box-select would be eaten).
    crate::tab_cycle::apply_display();
    crate::tab_cycle::poll();
    // Global production queue (config-driven 2x icons; see queue.rs).
    crate::queue::poll_hide();
    crate::queue::draw();
    // Rollback bug-capture confirmation (drawn regardless of the overlay switch).
    draw_capture_confirmation();
    // Dev REPL: poll the command file and draw transient probes.
    crate::repl::poll();
    crate::repl::draw();
    // Nydus in-world numbers: world annotations that must be visible whenever a
    // game runs, regardless of the saturation overlay switch. (The minimap
    // labels, idle badge and Tab indicator are screen-fixed and draw through
    // the minimap dispatch — see hud.rs — so the zoom crop never hides them.)
    draw_world_annotations();
    draw_pass();
    // Screen-fixed HUD overlays on the world surface, mapped into the zoom
    // crop (identity at 100%): visible at every zoom level.
    crate::hud::draw_overlays(surface, bounds);
    // One-shot abilities dump (CM_PLUS_DUMP_ABILITIES=1): unit -> buttons ->
    // orders map for the smart-cast classification.
    crate::abilities::dump_once();
    patch_game_menu();
    poll_changes();
    draw_changes();
}

/// In-world annotations that are independent of the saturation overlay: the
/// nydus canal/exit numbers above the buildings. Drawn every frame from the
/// draw hook (never gated by the overlay switch or by an empty node list —
/// a base whose patches are mined out still shows its Cesiant numbers).
unsafe fn draw_world_annotations() {
    if crate::frame::frame_count() == 0 {
        return; // not in a game
    }
    let font = in_game_font();
    if font.is_null() {
        return;
    }
    set_font_handle(font);
    write_draw_text(0, 0, 0x7FFF, 0x7FFF);
    *(G_PRINT_XY_ALIGNMENT as *mut u8) = 0x11;
    let mut buf = [0u8; 16];
    crate::nydus::draw_world(&mut buf);
    set_font_handle(core::ptr::null_mut());
}

/// The idle-worker counter, SC2-style: a worker icon + "Idle Workers: N" at
/// the top-left of the VIEW (camera-relative), so it is always on screen at
/// every zoom and on every build. Camera-relative is deliberate: the
/// screen-fixed mapping (hud::screen_to_surface) depends on the widescreen
/// WorldZoom statics, whose layout differs between gptp builds — on builds
/// where it fails the mapping is identity and "screen" coordinates land on
/// the WORLD (the counter then only shows near the map origin). The view
/// corner is the one coordinate space that has always been visible on this
/// engine (the changes panel and Tab indicator use it). Grey race worker
/// icon at 0, yellow when workers are idle.
pub(crate) unsafe fn draw_idle_counter() {
    if crate::frame::frame_count() == 0 {
        return; // not in a game
    }
    let Some(local) = crate::idle_workers::local_player() else {
        return;
    };
    let font = in_game_font();
    if font.is_null() {
        return;
    }
    if !crate::repl::badge_shown() {
        return;
    }
    let (n, icon) = crate::idle_workers::idle_count_and_icon(local);
    static FIRST_DRAW: std::sync::Once = std::sync::Once::new();
    FIRST_DRAW.call_once(|| {
        crate::log::log(&format!("[cosmo-plus] idle counter: first draw n={n}"));
    });

    // Top-left of the view, below the short changes-panel hint line.
    let (cx, cy) = view_origin();
    let (ox, oy) = crate::repl::badge_pin().unwrap_or((4, 54));
    let tx = cx + ox;
    let ty = cy + oy;

    // The race's greyed worker at 0 (SC2 shows the worker button greyed), the
    // first idle worker's icon otherwise.
    let race = player_race(local as u8);
    let icon_id = icon.unwrap_or_else(|| tier1_worker_icon(race));
    crate::icon::draw_unit_icon_scaled(icon_id, tx, ty, 24, 22);

    set_font_handle(font);
    write_draw_text(0, 0, 0x7FFF, 0x7FFF);
    *(G_PRINT_XY_ALIGNMENT as *mut u8) = 0x11;

    // Shadow, then "Idle Workers: N" in yellow (grey at 0).
    let mut buf = [0u8; 24];
    buf.fill(0);
    let label = format!("Idle Workers: {n}");
    let len = label.len().min(buf.len() - 1);
    buf[..len].copy_from_slice(&label.as_bytes()[..len]);
    let text_x = tx + 28;
    get_color_handle(text_color::BLACK);
    bw_print_xy(buf.as_ptr(), text_x + 1, ty + 1);
    get_color_handle(if n > 0 { text_color::YELLOW } else { text_color::GREY2 });
    bw_print_xy(buf.as_ptr(), text_x, ty);
    set_font_handle(core::ptr::null_mut());
}

/// The tier-1 worker unit id for a `RaceId::Enum` value (0 Zerg, 1 Terran,
/// 2 Protoss, 69 Askosi), matching `saturation::WORKER_TIERS`. Used for the
/// idle badge when no worker is idle (SC2 shows the race's worker greyed).
fn tier1_worker_icon(race: u8) -> u32 {
    let unit_id: u16 = match race {
        0 => 41,   // Zerg Droleth
        2 => 64,   // Protoss Scribe
        69 => 439, // Askosi Pilgrim
        _ => 7,    // Terran Mason
    };
    bw_dat::UnitId(unit_id).icon()
}

/// The Tab subgroup indicator (`Tab 2/3 · Marines`), drawn for ~1.5 s after each Tab
/// press. Camera-relative like the Changes panel, so it stays inside the view at
/// every zoom; its text renders at world scale (2x on screen at 200% zoom) —
/// the same deliberate trade-off the Changes panel makes (constant-size
/// screen-fixed text would need a scaled font this pipeline does not have).
pub(crate) unsafe fn draw_tab_indicator() {
    let Some((active, total)) = crate::tab_cycle::indicator(crate::frame::frame_count()) else {
        return;
    };
    let font = in_game_font();
    if font.is_null() {
        return;
    }
    set_font_handle(font);
    write_draw_text(0, 0, 0x7FFF, 0x7FFF);
    *(G_PRINT_XY_ALIGNMENT as *mut u8) = 0x11;
    get_color_handle(text_color::WHITE);
    let mut buf = [0u8; 64];
    buf.fill(0);
    let id = crate::tab_cycle::active_unit_id() as usize;
    let name = UNIT_NAMES
        .get(id)
        .map(|s| s.as_str())
        .unwrap_or("")
        .trim();
    let label = if name.is_empty() {
        format!("Tab {active}/{total}")
    } else {
        // Keep the line short; long unit names are cut off.
        let mut short = String::with_capacity(28);
        for c in name.chars() {
            if short.len() + 6 >= 28 {
                break;
            }
            short.push(c);
        }
        format!("Tab {active}/{total} \u{b7} {short}")
    };
    let _ = write!(&mut buf[..], "{label}");
    // Camera-relative like the Changes panel: always inside the view, at
    // every zoom level.
    let (cx, cy) = view_origin();
    bw_print_xy(buf.as_ptr(), cx + 60, cy + 250);
    set_font_handle(core::ptr::null_mut());
}

/// `mpq/arr/units.txt` names (id = line index), loaded once for the indicator.
static UNIT_NAMES: Lazy<Vec<String>> = Lazy::new(crate::data::load_unit_names);

/// The last frame captured by the rollback plugin's bug-capture (export poll), so we can
/// confirm on screen that Ctrl+F11 (or the freeze watchdog) saved a bundle.
static LAST_SEEN_CAPTURE: AtomicU32 = AtomicU32::new(0);
static CAPTURE_SHOW_UNTIL: AtomicU32 = AtomicU32::new(0);

fn rollback_last_capture() -> u32 {
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
    use std::sync::atomic::AtomicUsize;
    static FN: AtomicUsize = AtomicUsize::new(0);
    let mut ptr = FN.load(Ordering::SeqCst);
    if ptr == 0 {
        unsafe {
            let module = GetModuleHandleA(c"zz-cosmo-rollback.qdp".as_ptr() as *const u8);
            if module as usize != 0 {
                match GetProcAddress(
                    module,
                    c"cosmo_rollback_last_capture".as_ptr() as *const u8,
                ) {
                    Some(f) => {
                        ptr = f as usize;
                        FN.store(ptr, Ordering::SeqCst);
                    }
                    None => ptr = 0,
                }
            }
        }
    }
    if ptr == 0 {
        return 0;
    }
    unsafe { core::mem::transmute::<usize, unsafe extern "system" fn() -> u32>(ptr)() }
}

unsafe fn draw_capture_confirmation() {
    let frame = crate::frame::frame_count();
    if frame == 0 {
        return;
    }
    let last = rollback_last_capture();
    if last != 0 && last != LAST_SEEN_CAPTURE.load(Ordering::SeqCst) {
        LAST_SEEN_CAPTURE.store(last, Ordering::SeqCst);
        CAPTURE_SHOW_UNTIL.store(frame.saturating_add(72), Ordering::SeqCst); // ~3 s
    }
    if frame > CAPTURE_SHOW_UNTIL.load(Ordering::SeqCst) {
        return;
    }
    let font = in_game_font();
    if font.is_null() {
        return;
    }
    set_font_handle(font);
    write_draw_text(0, 0, 0x7FFF, 0x7FFF);
    *(G_PRINT_XY_ALIGNMENT as *mut u8) = 0x11;
    get_color_handle(text_color::YELLOW);
    let mut buf = [0u8; 32];
    buf.fill(0);
    let _ = write!(&mut buf[..], "BUG SAVED f{}", last);
    let (cx, cy) = view_origin();
    bw_print_xy(buf.as_ptr(), cx + 60, cy + 264);
    set_font_handle(core::ptr::null_mut());
}

/// The per-frame draw pass: recompute state on a new logic frame, then draw every node.
unsafe fn draw_pass() {
    if !ENABLED.load(Ordering::SeqCst) {
        return;
    }
    let frame = crate::frame::frame_count();
    if frame == 0 {
        return; // not in a game yet
    }

    // try_lock: never block (the game is single-threaded; this is just a re-entrancy guard).
    let Ok(mut cache) = CACHE.try_lock() else {
        return;
    };
    if cache.frame != frame {
        recompute(&mut cache, frame);
    }
    let nodes = &cache.nodes;
    if nodes.is_empty() {
        return;
    }

    let font = in_game_font();
    if font.is_null() {
        return;
    }

    set_font_handle(font);
    // Clip box = whole surface, so our labels are not clipped to the message-log box.
    write_draw_text(0, 0, 0x7FFF, 0x7FFF);
    // Default left alignment (0x11) set by set_font_handle; we centre manually.
    *(G_PRINT_XY_ALIGNMENT as *mut u8) = 0x11;

    let mut buf = [0u8; 16];
    // Only the local player's own bases get labels: the counts reveal which
    // mineral lines are worked, i.e. where expansions are — nothing is drawn
    // for other players (or the AI), and nothing of yours is shown to them.
    let local = local_player_id() as u8;
    for n in nodes {
        if n.owner != local {
            continue;
        }
        if n.is_gas {
            draw_node(n, &mut buf);
        }
    }
    // SC2-style: the mineral-line count sits above the town hall, not on the fields.
    for d in &cache.depots {
        if d.owner != local {
            continue;
        }
        draw_depot(d, &mut buf);
    }
    set_font_handle(core::ptr::null_mut());
}

/// Recompute the node list for the given frame (once per logic frame).
unsafe fn recompute(cache: &mut Cache, frame: u32) {
    LOG_MOVERS.call_once(log_functional_movers);
    cache.frame = frame;
    cache.nodes.clear();
    cache.scratch_nodes.clear();
    cache.scratch_depots.clear();
    cache.scratch_workers.clear();

    crate::frame::for_each_unit(|u| {
        let id = (*u).unit_id;
        // Exclude units still under construction / morphing.
        if (*u).remaining_build_time != 0 {
            return;
        }
        if saturation::is_resource_node(id) {
            // Hide depleted mineral fields.
            if saturation::is_mineral_field(id) && (*u).unit_specific2.resource.amount == 0 {
                return;
            }
            // Hide nodes under fog: `sprite->visibility_status` (0 = hidden) is exposed
            // as `visibility_mask` in bw_dat's 1.16.1 Sprite layout.
            let sprite = (*u).flingy.sprite;
            if sprite.is_null() || (*sprite).visibility_mask == 0 {
                return;
            }
            let p = (*u).flingy.position;
            cache.scratch_nodes.push((u, p.x as i32, p.y as i32, id));
        } else if saturation::DEPOT_IDS.contains(&id) {
            let p = (*u).flingy.position;
            let sprite = (*u).flingy.sprite;
            cache
                .scratch_depots
                .push(((*u).player, p.x as i32, p.y as i32, sprite as i32));
        } else if let Some(tier) = saturation::worker_tier(id) {
            let target = (*u).unit_specific.worker.current_harvest_target;
            cache.scratch_workers.push((tier, target));
        }
    });

    let local = local_player_id() as u8;

    for &(node, nx, ny, node_id) in &cache.scratch_nodes {
        let mut tier1 = 0u32;
        let mut tier2 = 0u32;
        for &(tier, target) in &cache.scratch_workers {
            if target == node {
                if tier == 1 {
                    tier1 += 1;
                } else {
                    tier2 += 1;
                }
            }
        }

        // Nearest completed depot -> owner + distance; skip nodes with no depot.
        let mut owner = local;
        let mut best_dist2 = i64::MAX;
        for &(player, dx, dy, _) in &cache.scratch_depots {
            let ddx = (dx - nx) as i64;
            let ddy = (dy - ny) as i64;
            let dist2 = ddx * ddx + ddy * ddy;
            if dist2 < best_dist2 {
                best_dist2 = dist2;
                owner = player;
            }
        }
        if best_dist2 == i64::MAX || best_dist2 > SERVICE_RADIUS_PX * SERVICE_RADIUS_PX {
            continue; // no working base serves this node
        }

        let dist = (best_dist2 as f64).sqrt();
        // Flat model: every node takes 3 workers (optimal placement assumed).
        let mut capacity = if saturation::is_mineral_field(node_id) {
            saturation::MINERAL_WORKERS_PER_PATCH
        } else {
            saturation::GAS_WORKERS_PER_NODE
        };
        let cap = OPT_CAP.load(Ordering::SeqCst);
        if cap != usize::MAX {
            capacity = capacity.min(cap as u32);
        }
        let assigned = tier1 + tier2;
        let saturated = assigned >= capacity;
        let race = player_race(owner);
        let color = if saturated {
            saturation::race_textcolor(race)
        } else {
            text_color::WHITE
        };

        cache.nodes.push(NodeState {
            x: nx,
            y: ny,
            tier1,
            tier2,
            capacity,
            color,
            dist: dist as u32,
            owner,
            race,
            is_gas: !saturation::is_mineral_field(node_id),
        });
    }

    // Aggregate each visible depot's mineral line: assigned workers + flat capacity
    // (3 per field it serves, within SERVICE_RADIUS). Drawn above the building.
    cache.depots.clear();
    for &(player, dx, dy, sprite_ptr) in &cache.scratch_depots {
        let sprite = sprite_ptr as *mut bw_dat::structs::Sprite;
        if sprite.is_null() || (*sprite).visibility_mask == 0 {
            continue; // depot itself is fogged; nothing to show
        }
        let mut tier1 = 0u32;
        let mut tier2 = 0u32;
        let mut capacity = 0u32;
        for n in &cache.nodes {
            if n.is_gas || n.owner != player {
                continue;
            }
            let ddx = (n.x - dx) as i64;
            let ddy = (n.y - dy) as i64;
            if ddx * ddx + ddy * ddy <= SERVICE_RADIUS_PX * SERVICE_RADIUS_PX {
                tier1 += n.tier1;
                tier2 += n.tier2;
                capacity += n.capacity;
            }
        }
        if capacity == 0 {
            continue; // no visible mineral line near this depot
        }
        let assigned = tier1 + tier2;
        let saturated = assigned >= capacity;
        let race = player_race(player);
        let color = if saturated {
            saturation::race_textcolor(race)
        } else {
            text_color::WHITE
        };
        cache.depots.push(DepotState {
            x: dx,
            y: dy,
            owner: player,
            race,
            tier1,
            tier2,
            capacity,
            color,
        });
    }

    // OCR-free verification: dump node states at game start and every 10 seconds, so the
    // assigned/optimal values and the white->race-colour flip are visible in the log.
    if LOG_DUMP.load(Ordering::SeqCst)
        && (frame == 1 || frame.saturating_sub(cache.last_log) >= 240)
    {
        log_nodes(frame, &cache.nodes, camera());
        cache.last_log = frame;
    }
}

/// Append a one-line-per-node dump to `cosmo_plus.log`.
unsafe fn log_nodes(frame: u32, nodes: &[NodeState], (cam_x, cam_y): (i32, i32)) {
    if nodes.is_empty() {
        return;
    }
    let mut msg = format!(
        "[cosmo-plus] overlay frame {frame}: {} node(s) camera=({cam_x},{cam_y}) [x,y t1+t2=assigned/capacity dist owner race colour saturated]",
        nodes.len()
    );
    for n in nodes {
        msg.push_str(&format!(
            " | ({},{}) {}+{}={}/{} dist={} owner={} race={} col={:02x} sat={}",
            n.x,
            n.y,
            n.tier1,
            n.tier2,
            n.tier1 + n.tier2,
            n.capacity,
            n.dist,
            n.owner,
            n.race,
            n.color,
            n.tier1 + n.tier2 >= n.capacity,
        ));
    }
    crate::log::log(&msg);
}

/// Draw the flat assigned/capacity row above one node (world -> screen transform:
/// `world - camera`). Format: `assigned/capacity` (e.g. `2/3` on a geyser).
unsafe fn draw_node(n: &NodeState, buf: &mut [u8; 16]) {
    let (cx, cy) = camera();
    let sx = n.x - cx;
    let sy = n.y - cy;

    get_color_handle(n.color);

    buf.fill(0);
    let label = saturation::format_flat(n.tier1 + n.tier2, n.capacity);
    let _ = write!(&mut buf[..], "{label}");
    bw_print_xy(buf.as_ptr(), sx - 10, sy - 28);
}

/// Draw the SC2-style mineral count above one town hall:
/// `assigned/capacity` (e.g. `12/24` on 8 patches), white while under-saturated,
/// the owner's race colour once saturated.
unsafe fn draw_depot(d: &DepotState, buf: &mut [u8; 16]) {
    let (cx, cy) = camera();
    let sx = d.x - cx;
    let sy = d.y - cy;

    get_color_handle(d.color);

    buf.fill(0);
    let label = saturation::format_flat(d.tier1 + d.tier2, d.capacity);
    let _ = write!(&mut buf[..], "{label}");
    bw_print_xy(buf.as_ptr(), sx - 8, sy - 40);
}

// ---------------------------------------------------------------------------
// Low-level 1.16.1 text primitives (custom register ABI; wrapped with PUSHAD/POPAD the
// same way `bwhelper.h` does, so no register is clobbered).
// ---------------------------------------------------------------------------

/// Resolve the changes-panel hotkey (one-time, at install).
fn install_changes() {
    let vk = crate::config::env("CM_PLUS_CHANGES_KEY")
        .as_deref()
        .and_then(cosmo_plus_core::hotkeys::parse_key)
        .unwrap_or(CHANGES_DEFAULT_KEY);
    CHANGES_KEY.store(vk as u32, Ordering::SeqCst);
    crate::log::log(&format!(
        "[cosmo-plus] changes panel: VK 0x{vk:02X} toggles changes vs OG"
    ));
    if crate::config::env_flag("CM_PLUS_MENU_SELFTEST") {
        unsafe { selftest_menu() };
    }
}

/// Poll the panel hotkey on its rising edge (draw pass = view-local input).
pub(crate) unsafe fn poll_changes() {
    if crate::frame::frame_count() == 0 {
        return;
    }
    let vk = CHANGES_KEY.load(Ordering::SeqCst);
    if vk == 0 {
        return;
    }
    let down = (windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(vk as i32)
        as u16
        & 0x8000)
        != 0;
    let was = CHANGES_DOWN.swap(down, Ordering::SeqCst);
    if down && !was {
        let now = !CHANGES_VISIBLE.load(Ordering::SeqCst);
        CHANGES_VISIBLE.store(now, Ordering::SeqCst);
        crate::log::log(&format!(
            "[cosmo-plus] changes panel: {}",
            if now { "shown" } else { "hidden" }
        ));
    }
}

/// Whether the changes panel is currently open (the queue overlay suppresses
/// itself while it is — they share the top-left corner).
pub(crate) fn changes_visible() -> bool {
    CHANGES_VISIBLE.load(Ordering::SeqCst)
}

/// Draw the changes panel, or a short "how to open it" hint for the first ~20 s
/// of each game. Independent of the saturation overlay switch.
pub(crate) unsafe fn draw_changes() {
    let frame = crate::frame::frame_count();
    if frame == 0 {
        return;
    }
    let last = CHANGES_LAST_FRAME.swap(frame, Ordering::SeqCst);
    if frame < last {
        CHANGES_HINT_FROM.store(frame, Ordering::SeqCst); // new game
    }
    if CHANGES_HINT_FROM.load(Ordering::SeqCst) == 0 {
        CHANGES_HINT_FROM.store(frame, Ordering::SeqCst);
    }

    let visible = CHANGES_VISIBLE.load(Ordering::SeqCst);
    let hint = !visible
        && frame.saturating_sub(CHANGES_HINT_FROM.load(Ordering::SeqCst)) < CHANGES_HINT_FRAMES;
    if !visible && !hint {
        return;
    }

    let font = in_game_font();
    if font.is_null() {
        return;
    }
    set_font_handle(font);
    write_draw_text(0, 0, 0x7FFF, 0x7FFF);
    *(G_PRINT_XY_ALIGNMENT as *mut u8) = 0x11;
    let (cx, cy) = view_origin();

    if visible {
        get_color_handle(text_color::YELLOW);
        let title = padded("COSMONARCHY PLUS - CHANGES VS OG (Pause/Break closes)");
        bw_print_xy(title.as_ptr(), cx + 60, cy + 24);
        get_color_handle(text_color::WHITE);
        for (i, line) in CHANGES_LINES.iter().enumerate() {
            let buf = padded(line);
            bw_print_xy(buf.as_ptr(), cx + 70, cy + 48 + i as i32 * 14);
        }
    } else {
        get_color_handle(text_color::YELLOW);
        let buf = padded("Cosmonarchy Plus: Pause/Break shows changes vs OG");
        bw_print_xy(buf.as_ptr(), cx + 60, cy + 24);
    }
    set_font_handle(core::ptr::null_mut());
}

// ---------------------------------------------------------------------------
// Game-menu entry: relabel the Help button to "Plus Changes" and handle it.
// ---------------------------------------------------------------------------

/// Minimal `BinDlg` (1.16.1) prefix; the rest is control-type data we do not touch.
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
    fxn_interact: Option<unsafe extern "fastcall" fn(*mut BinDlg, *mut DialogEvent) -> u32>,
}

/// Minimal `DialogEvent`.
#[repr(C)]
struct DialogEvent {
    user: u32,
    param: u16,
    field_6: u16,
    virtual_key: u16,
    virtual_key_modifier: u16,
    number_order: u16,
    current: [u16; 2],
    field_12: u16,
}

/// Runs when the relabelled menu entry is activated (mouse or its letter).
unsafe extern "fastcall" fn changes_menu_interact(
    _control: *mut BinDlg,
    event: *mut DialogEvent,
) -> u32 {
    if !event.is_null() {
        let ev = &*event;
        // 4/5 = left button down/up, 0 = key down (the menu's letter shortcut).
        if matches!(ev.number_order, 0 | 4 | 5) {
            let now = !CHANGES_VISIBLE.load(Ordering::SeqCst);
            CHANGES_VISIBLE.store(now, Ordering::SeqCst);
            crate::log::log(&format!(
                "[cosmo-plus] changes panel: {} from the game menu",
                if now { "shown" } else { "hidden" }
            ));
            return 1;
        }
    }
    0
}

/// True when [ptr, ptr+len) is committed, readable memory (VirtualQuery).
pub(crate) fn readable_ptr(ptr: *const u8, len: usize) -> bool {
    use windows_sys::Win32::System::Memory::{
        VirtualQuery, MEMORY_BASIC_INFORMATION, MEM_COMMIT, PAGE_GUARD, PAGE_NOACCESS,
    };
    if ptr.is_null() {
        return false;
    }
    let mut mbi: MEMORY_BASIC_INFORMATION = unsafe { core::mem::zeroed() };
    let queried = unsafe {
        VirtualQuery(
            ptr as *const core::ffi::c_void,
            &mut mbi,
            core::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
        )
    };
    if queried == 0 || mbi.State != MEM_COMMIT || mbi.Protect & (PAGE_NOACCESS | PAGE_GUARD) != 0 {
        return false;
    }
    let region_end = mbi.BaseAddress as usize + mbi.RegionSize;
    region_end >= ptr as usize + len
}

/// Bounded, guard-checked C string read.
fn read_cstr(ptr: *const u8, max: usize) -> Option<Vec<u8>> {
    if !readable_ptr(ptr, 1) {
        return None;
    }
    let mut out = Vec::with_capacity(max.min(64));
    for i in 0..max {
        if !readable_ptr(unsafe { ptr.add(i) }, 1) {
            return None;
        }
        let b = unsafe { *ptr.add(i) };
        if b == 0 {
            return Some(out);
        }
        out.push(b);
    }
    None
}

/// Runs every draw frame: when the in-game F10 menu is open, relabel its Help
/// control and take over its click. Outside a game (`frame_count == 0`) there is
/// no game menu; every pointer is validated before it is read, and the dialog
/// must look like the game menu (title contains "game" and it has a Help
/// control), so popups from other screens are never touched.
pub(crate) unsafe fn patch_game_menu() {
    if crate::frame::frame_count() == 0 {
        return;
    }
    let root = *(G_POPUP_DIALOG as *const *mut BinDlg);
    if !readable_ptr(root as *const u8, core::mem::size_of::<BinDlg>()) {
        return;
    }
    let Some(title) = read_cstr((*root).text, 64) else {
        return;
    };
    if !title.to_ascii_lowercase().windows(4).any(|w| w == b"game") {
        return;
    }
    patch_chain(root);
}

/// Walk a dialog chain. The new "Plus Changes" row added to gamemenu.bin is
/// picked up by its text; its handler is installed and it is shown only when
/// Save Game is hidden (multiplayer) - in single player the free slot is used
/// by Save, so our row would overlap it. Falls back to relabelling Help when
/// the dialog template has no Plus Changes row.
unsafe fn patch_chain(root: *mut BinDlg) -> bool {
    let mut dlg = root;
    let mut guard = 0;
    let mut our: *mut BinDlg = core::ptr::null_mut();
    let mut save_visible = false;
    while guard < 64 {
        if !readable_ptr(dlg as *const u8, core::mem::size_of::<BinDlg>()) {
            return false;
        }
        if let Some(bytes) = read_cstr((*dlg).text, 64) {
            let lower = bytes.to_ascii_lowercase();
            if lower.windows(12).any(|w| w == b"plus changes") {
                our = dlg;
            } else if lower.windows(9).any(|w| w == b"save game") && (*dlg).flags & 0x8 != 0 {
                save_visible = true;
            }
        }
        let next = (*dlg).next;
        if next.is_null() {
            break;
        }
        dlg = next;
        guard += 1;
    }

    if !our.is_null() {
        if save_visible {
            (*our).flags &= !0x8;
        } else {
            (*our).flags |= 0x8;
        }
        if (*our).fxn_interact != Some(changes_menu_interact) {
            (*our).fxn_interact = Some(changes_menu_interact);
            crate::log::log("[cosmo-plus] game menu: Plus Changes row wired up");
        }
        return true;
    }

    // Older dialog template without the added row: relabel Help instead.
    let mut dlg = root;
    let mut guard = 0;
    while guard < 64 {
        if !readable_ptr(dlg as *const u8, core::mem::size_of::<BinDlg>()) {
            return false;
        }
        if let Some(bytes) = read_cstr((*dlg).text, 64) {
            let lower = bytes.to_ascii_lowercase();
            if lower.windows(4).any(|w| w == b"help") {
                if (*dlg).text != MENU_TEXT.as_ptr() {
                    (*dlg).text = MENU_TEXT.as_ptr();
                    (*dlg).fxn_interact = Some(changes_menu_interact);
                    crate::log::log(
                        "[cosmo-plus] game menu: Help relabelled to Plus Changes (P)",
                    );
                }
                return true;
            }
        }
        let next = (*dlg).next;
        if next.is_null() {
            return false;
        }
        dlg = next;
        guard += 1;
    }
    false
}

/// Runtime self-test of the dialog patching (CM_PLUS_MENU_SELFTEST=1): builds a
/// fake GameMenu chain in this process, patches it, verifies the relabel, then
/// restores the game's global pointer.
pub(crate) unsafe fn selftest_menu() {
    let mut root: BinDlg = core::mem::zeroed();
    let mut save: BinDlg = core::mem::zeroed();
    let mut plus: BinDlg = core::mem::zeroed();
    root.text = b"GameMenu\0".as_ptr();
    save.text = b"sSave Game (\0".as_ptr();
    save.flags = 0x0A18; // visible: single player
    plus.text = b"pPlus Changes (\0".as_ptr();
    plus.flags = 0x0A18;
    root.next = &mut save;
    save.next = &mut plus;

    let global = G_POPUP_DIALOG as *mut *mut BinDlg;
    let saved = *global;
    *global = &mut root;
    let found = patch_chain(&mut root);
    let hidden_in_sp = plus.flags & 0x8 == 0;
    save.flags &= !0x8; // multiplayer: Save is hidden, the row should show
    let found2 = patch_chain(&mut root);
    let visible_in_mp = plus.flags & 0x8 != 0;
    *global = saved;

    let ok = found
        && found2
        && hidden_in_sp
        && visible_in_mp
        && plus.fxn_interact.is_some();
    crate::log::log(&format!(
        "[cosmo-plus] game menu: selftest {}",
        if ok {
            "PASS (Plus Changes row wired, hidden in SP, shown in MP)"
        } else {
            "FAIL"
        }
    ));
}

/// Draw a small panel of text lines fixed to the screen (draft UI). Screen space is
/// expressed as camera-relative world coordinates, so the panel follows the camera.
/// Read-only: called from the draw pass, so per-client differences are safe.
pub(crate) unsafe fn draw_text_lines(title: &str, lines: &[String]) {
    let (cx, cy) = view_origin();
    get_color_handle(4); // white

    let title_buf = padded(title);
    bw_print_xy(title_buf.as_ptr(), cx + 60, cy + 24);
    for (i, line) in lines.iter().enumerate() {
        let buf = padded(line);
        bw_print_xy(buf.as_ptr(), cx + 70, cy + 48 + i as i32 * 14);
    }
}

/// NUL-terminated copy of `s` for the C text primitives.
fn padded(s: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity(s.len() + 1);
    v.extend_from_slice(s.as_bytes());
    v.push(0);
    v
}

/// `helper::bw_print_xy` — eax = text, esi = x, [esp] = y (callee cleans the arg).
#[inline(never)]
pub(crate) unsafe fn bw_print_xy(text: *const u8, x: i32, y: i32) {
    core::arch::asm!(
        "pushad",
        "mov esi, ebx",
        "push ecx",
        "call edx",
        "popad",
        in("eax") text,
        in("ebx") x,
        in("ecx") y,
        in("edx") ADDR_BW_PRINT_XY as i32,
        options(nostack),
    );
}

/// `helper::get_color_handle(TextColor)` — al = colour id.
#[inline(never)]
pub(crate) unsafe fn get_color_handle(color: u8) {
    core::arch::asm!(
        "pushad",
        "call edx",
        "popad",
        in("eax") color as i32,
        in("edx") ADDR_GET_COLOR_HANDLE as i32,
        options(nostack),
    );
}

/// `helper::set_font_handle(u32*)` — ecx = font handle.
#[inline(never)]
pub(crate) unsafe fn set_font_handle(font: *mut u32) {
    core::arch::asm!(
        "pushad",
        "call edx",
        "popad",
        in("ecx") font,
        in("edx") ADDR_SET_FONT_HANDLE as i32,
        options(nostack),
    );
}

// ---------------------------------------------------------------------------
// State readers.
// ---------------------------------------------------------------------------

/// The overlay label font handle (`font_base[FONT_INDEX]`), or null if not initialised.
pub(crate) unsafe fn in_game_font() -> *mut u32 {
    let font_base = FONT_BASE as *const *const u32;
    let idx = FONT_INDEX.load(Ordering::SeqCst);
    *font_base.add(idx) as *mut u32
}

/// The current camera position in world px. `g_screen_x` is `(s32*)0x00628448` (a constant
/// pointer), so the value lives **directly at** the address — one deref, not two.
pub(crate) unsafe fn camera() -> (i32, i32) {
    let x = std::ptr::read(G_SCREEN_X_PTR as *const i32);
    let y = std::ptr::read(G_SCREEN_Y_PTR as *const i32);
    (x, y)
}

/// The visible view's top-left corner in world px: camera + the zoom crop's
/// camera-relative offset. Every screen-corner overlay (idle counter, Tab
/// indicator, changes panel, larva entry, queue) anchors here — drawing at the
/// bare camera position places them outside the visible crop at 200% zoom,
/// which is why none of the Plus UI showed up in the release build.
pub(crate) unsafe fn view_origin() -> (i32, i32) {
    let (cx, cy) = camera();
    let (ox, oy) = crate::hud::crop_offset();
    (cx + ox, cy + oy)
}

/// Write the text clip box (`g_draw_text`) so our labels are not clipped to the message log.
pub(crate) unsafe fn write_draw_text(left: i16, top: i16, right: i16, bottom: i16) {
    *(G_DRAW_TEXT as *mut i16) = left;
    *((G_DRAW_TEXT + 2) as *mut i16) = top;
    *((G_DRAW_TEXT + 4) as *mut i16) = right;
    *((G_DRAW_TEXT + 6) as *mut i16) = bottom;
}

unsafe fn player_race(player: u8) -> u8 {
    let base = G_PLAYER_TABLE as *const u8;
    *base.add(player as usize * PLAYER_STRUCT_SIZE + PLAYER_RACE_OFFSET)
}

unsafe fn local_player_id() -> u32 {
    let api = crate::plus::api();
    match (api.local_player_id)() {
        Some(f) => f(),
        None => 0,
    }
}
