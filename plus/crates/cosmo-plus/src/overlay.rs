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
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
/// `font_base` (`graphics::Font**`); `font_base[1]` is the in-game font.
const FONT_BASE: usize = 0x006CE0F4;
/// `g_draw_text` (Box16: left/top/right/bottom, i16 each) — the text clip box.
const G_DRAW_TEXT: usize = 0x006CE0C8;
/// `g_print_xy_alignment` (u8).
const G_PRINT_XY_ALIGNMENT: usize = 0x006CE110;
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
/// Test-only cap on the per-node optimum (from `CM_PLUS_SATURATION_OPT_CAP`), so the
/// white->race-colour flip can be forced without waiting for a fully-saturated base.
static OPT_CAP: AtomicUsize = AtomicUsize::new(usize::MAX);
/// Whether to write a periodic per-node dump to the log (from `CM_PLUS_SATURATION_LOG=1`).
static LOG_DUMP: AtomicBool = AtomicBool::new(false);

/// One resource node's display state.
struct NodeState {
    x: i32,
    y: i32,
    tier1: u32,
    tier2: u32,
    optimal: u32,
    color: u8,
    /// Straight-line distance to the nearest depot (px), for the log.
    dist: u32,
    /// The owner player id (depot owner) used for the race colour.
    owner: u8,
    /// The owner's `RaceId` (for the log).
    race: u8,
}

/// Per-frame cache; recomputed only when the logic frame advances. Scratch vectors are
/// reused so the hot path allocates nothing.
struct Cache {
    frame: u32,
    nodes: Vec<NodeState>,
    scratch_nodes: Vec<(*mut Unit, i32, i32)>,
    scratch_depots: Vec<(u8, i32, i32)>,
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
    }
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
    draw_pass();
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
    for n in nodes {
        draw_node(n, &mut buf);
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
            cache.scratch_nodes.push((u, p.x as i32, p.y as i32));
        } else if saturation::DEPOT_IDS.contains(&id) {
            let p = (*u).flingy.position;
            cache.scratch_depots.push(((*u).player, p.x as i32, p.y as i32));
        } else if let Some(tier) = saturation::worker_tier(id) {
            let target = (*u).unit_specific.worker.current_harvest_target;
            cache.scratch_workers.push((tier, target));
        }
    });

    let local = local_player_id() as u8;

    for &(node, nx, ny) in &cache.scratch_nodes {
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
        for &(player, dx, dy) in &cache.scratch_depots {
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
        let mut optimal = saturation::optimal_count_at_distance(dist);
        let cap = OPT_CAP.load(Ordering::SeqCst);
        if cap != usize::MAX {
            optimal = optimal.min(cap as u32);
        }
        let assigned = tier1 + tier2;
        let saturated = assigned >= optimal;
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
            optimal,
            color,
            dist: dist as u32,
            owner,
            race,
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
        "[cosmo-plus] overlay frame {frame}: {} node(s) camera=({cam_x},{cam_y}) [x,y t1+t2=assigned/optimal dist owner race colour saturated]",
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
            n.optimal,
            n.dist,
            n.owner,
            n.race,
            n.color,
            n.tier1 + n.tier2 >= n.optimal,
        ));
    }
    crate::log::log(&msg);
}

/// Draw the two rows above one node (world -> screen transform: `world - camera`).
unsafe fn draw_node(n: &NodeState, buf: &mut [u8; 16]) {
    let (cx, cy) = camera();
    let sx = n.x - cx;
    let sy = n.y - cy;

    get_color_handle(n.color);

    buf.fill(0);
    let _ = write!(&mut buf[..], "{}/{}", n.tier1, n.optimal);
    bw_print_xy(buf.as_ptr(), sx - 8, sy - 28);

    buf.fill(0);
    let _ = write!(&mut buf[..], "{}/{}", n.tier2, n.optimal);
    bw_print_xy(buf.as_ptr(), sx - 8, sy - 14);
}

// ---------------------------------------------------------------------------
// Low-level 1.16.1 text primitives (custom register ABI; wrapped with PUSHAD/POPAD the
// same way `bwhelper.h` does, so no register is clobbered).
// ---------------------------------------------------------------------------

/// `helper::bw_print_xy` — eax = text, esi = x, [esp] = y (callee cleans the arg).
#[inline(never)]
unsafe fn bw_print_xy(text: *const u8, x: i32, y: i32) {
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
unsafe fn get_color_handle(color: u8) {
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
unsafe fn set_font_handle(font: *mut u32) {
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

/// The in-game font handle (`font_base[1]`), or null if not initialised.
unsafe fn in_game_font() -> *mut u32 {
    let font_base = FONT_BASE as *const *const u32;
    *font_base.add(1) as *mut u32
}

/// The current camera position in world px. `g_screen_x` is `(s32*)0x00628448` (a constant
/// pointer), so the value lives **directly at** the address — one deref, not two.
unsafe fn camera() -> (i32, i32) {
    let x = std::ptr::read(G_SCREEN_X_PTR as *const i32);
    let y = std::ptr::read(G_SCREEN_Y_PTR as *const i32);
    (x, y)
}

/// Write the text clip box (`g_draw_text`) so our labels are not clipped to the message log.
unsafe fn write_draw_text(left: i16, top: i16, right: i16, bottom: i16) {
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
