//! Per-frame hook + first-tick initialisation.
//!
//! Plus uses `samase_shim`'s `hook_step_objects` (not a raw `whack` frame hook), so it
//! never competes with the rollback plugin's outer `GameFrame` trampoline. The callback
//! runs once per game-logic frame, after `StepObjects`, so unit order state is settled.
//!
//! Everything here is read-only with respect to simulation, except the one-time dat
//! override application at the first tick (deterministic: same config -> same writes).

use std::collections::BTreeMap;
use std::sync::Mutex;

use once_cell::sync::Lazy;
use bw_dat::structs::Unit;

use cosmo_plus_core::saturation;

/// Applied once, at the first game tick.
static FIRST_TICK: std::sync::Once = std::sync::Once::new();

/// Whether the `CM_PLUS_MEASURE=1` delivery-cadence logger is armed.
fn measure_enabled() -> bool {
    crate::config::env_flag("CM_PLUS_MEASURE")
}

/// Register the per-frame callback through the plugin api.
pub(crate) fn install() {
    let api = crate::plus::api();
    let result = unsafe { (api.hook_step_objects)(tick, 1) };
    if result == 0 {
        crate::log::log("[cosmo-plus] ERROR: hook_step_objects failed");
    } else {
        crate::log::log("[cosmo-plus] installed per-frame hook (after StepObjects)");
    }
}

/// The per-frame callback.
unsafe extern "C" fn tick() {
    unsafe {
        // Cosmonarchy's hotloader rebuilds the mpq (and the dat tables) on each
        // game launch, so the table pointers can move between sessions. Refresh
        // bw_dat and re-apply the (idempotent) dat overrides whenever they do —
        // the overrides are deterministic per config, so writing them per session
        // yields the same result as writing them once.
        if crate::data::init() {
            let names = crate::data::load_unit_names();
            if let Some(cfg) = crate::plus::config() {
                let applied = crate::data::apply_overrides(cfg, &names);
                crate::log::log(&format!("[cosmo-plus] applied {applied} dat override field(s)"));
                let worker_applied = crate::data::apply_advanced_worker_deltas(cfg);
                crate::log::log(&format!(
                    "[cosmo-plus] applied {worker_applied} advanced-worker dat field(s)"
                ));
            }
            if crate::config::env_flag("CM_PLUS_DUMP") {
                crate::data::dump_csv(&names);
            }
        }
        FIRST_TICK.call_once(|| {
            // In-game integration report: the strict globals (icons, fonts,
            // draw-text box) are now required to hold live values.
            crate::state::write_report("in-game");
            crate::antimeta::debug_dump_game_types_once();
            crate::ultraload_icon::install_once();
            if measure_enabled() {
                measure_begin();
            }
        });
        crate::antimeta::on_game_tick();
        crate::draft::on_game_tick();
        crate::ultraload::on_game_tick();
        // NOTE: the subgroup Tab press is deliberately NOT processed here.
        // The rotation + command-card refresh are UI work and must run in the
        // draw pass; running update_selected_unit_data inside the simulation
        // step corrupts the command card (the hotkeyed group lost every
        // ability). See tab_cycle.rs.
        crate::crash::note_tick();
        if measure_enabled() {
            measure_tick();
        }
    }
}

// ---------------------------------------------------------------------------
// Delivery-cadence measurement (CM_PLUS_MEASURE=1) — research only, never shipped
// gameplay. Produces three CSVs, post-processed into `docs/measurements/saturation.csv`:
//   cosmo_plus_measure.csv  per-node delivery events (resource_amount decrease)
//   cosmo_plus_speed.csv    peak worker speed (flingy current_speed, 8.8 fixed point)
//   cosmo_plus_pos.csv      per-frame worker positions/velocities for the first 360 frames
// ---------------------------------------------------------------------------

struct MeasureState {
    prev: BTreeMap<usize, u16>, // node ptr -> last resource_amount
    file: std::fs::File,
    speed_file: std::fs::File,
    pos_file: std::fs::File,
    // unit_id -> (max current_speed, current_speed_x at max, current_speed_y at max)
    max_speed: BTreeMap<u16, (i32, i32, i32)>,
    frames: u32,
}

static MEASURE: Lazy<Mutex<Option<MeasureState>>> = Lazy::new(|| Mutex::new(None));

/// Town-hall / resource-depot ids workers return cargo to (see `saturation::DEPOT_IDS`).
use cosmo_plus_core::saturation::DEPOT_IDS as TOWN_HALL_IDS;

fn measure_begin() {
    let mut file = match std::fs::File::create("cosmo_plus_measure.csv") {
        Ok(f) => f,
        Err(e) => {
            crate::log::log(&format!("[cosmo-plus] measure: cannot open csv: {e}"));
            return;
        }
    };
    let mut speed_file = match std::fs::File::create("cosmo_plus_speed.csv") {
        Ok(f) => f,
        Err(e) => {
            crate::log::log(&format!("[cosmo-plus] measure: cannot open speed csv: {e}"));
            return;
        }
    };
    let mut pos_file = match std::fs::File::create("cosmo_plus_pos.csv") {
        Ok(f) => f,
        Err(e) => {
            crate::log::log(&format!("[cosmo-plus] measure: cannot open pos csv: {e}"));
            return;
        }
    };
    use std::io::Write;
    let _ = writeln!(file, "frame,node_id,node_x,node_y,amount_before,amount_after,tier1,tier2");
    let _ = writeln!(speed_file, "unit_id,current_speed,current_speed_x,current_speed_y");
    let _ = writeln!(pos_file, "frame,unit_id,x,y,current_speed_x,current_speed_y");

    // Snapshot the starting layout: every mineral node and every town hall, once.
    unsafe {
        let mut lines = String::new();
        for_each_unit(|u| {
            let id = (*u).unit_id;
            if saturation::MINERAL_FIELD_IDS.contains(&id) {
                let p = (*u).flingy.position;
                let amt = (*u).unit_specific2.resource.amount;
                lines.push_str(&format!("node,{id},{},{},{}\n", p.x, p.y, amt));
            } else if TOWN_HALL_IDS.contains(&id) {
                let p = (*u).flingy.position;
                lines.push_str(&format!("townhall,{id},{},{}\n", p.x, p.y));
            }
        });
        let _ = write!(file, "{lines}");
    }

    *MEASURE.lock().unwrap() = Some(MeasureState {
        prev: BTreeMap::new(),
        file,
        speed_file,
        pos_file,
        max_speed: BTreeMap::new(),
        frames: 0,
    });

    let worker_ids: Vec<String> = saturation::WORKER_TIERS
        .iter()
        .map(|&(id, tier, race)| format!("{race} {id} (tier {tier})"))
        .collect();
    crate::log::log(&format!(
        "[cosmo-plus] measure armed: workers [{}]",
        worker_ids.join(", ")
    ));
}

fn measure_tick() {
    let frame = unsafe { frame_count() };
    let mut state = MEASURE.lock().unwrap();
    let Some(s) = state.as_mut() else { return };

    use std::io::Write;
    unsafe {
        // 1. Tally assigned tier-1/tier-2 workers per node pointer.
        let mut assigned: BTreeMap<usize, (u32, u32)> = BTreeMap::new();
        for_each_unit(|u| {
            if let Some(tier) = saturation::worker_tier((*u).unit_id) {
                let target = (*u).unit_specific.worker.current_harvest_target;
                if !target.is_null() {
                    let e = assigned.entry(target as usize).or_insert((0, 0));
                    if tier == 1 {
                        e.0 += 1;
                    } else {
                        e.1 += 1;
                    }
                }
            }
        });

        // 2. Per-worker: track peak speed and sample positions.
        for_each_unit(|u| {
            let id = (*u).unit_id;
            if saturation::worker_tier(id).is_none() {
                return;
            }
            let cs = (*u).flingy.current_speed;
            let csx = (*u).flingy.current_speed_x;
            let csy = (*u).flingy.current_speed_y;
            if cs > 0 {
                let entry = s.max_speed.entry(id).or_insert((0, 0, 0));
                if cs > entry.0 {
                    *entry = (cs, csx, csy);
                }
            }
            if s.frames < 360 {
                let p = (*u).flingy.position;
                let _ = writeln!(s.pos_file, "{frame},{id},{},{},{csx},{csy}", p.x, p.y);
            }
        });

        // 3. Detect mineral-node deliveries (resource_amount decrease).
        for_each_unit(|u| {
            let id = (*u).unit_id;
            if !saturation::MINERAL_FIELD_IDS.contains(&id) {
                return;
            }
            let amt = (*u).unit_specific2.resource.amount;
            let key = u as usize;
            let prev = s.prev.insert(key, amt);
            if let Some(prev) = prev {
                if amt < prev {
                    let p = (*u).flingy.position;
                    let (t1, t2) = assigned.get(&key).copied().unwrap_or((0, 0));
                    let _ = writeln!(
                        s.file,
                        "{frame},{id},{},{},{prev},{amt},{t1},{t2}",
                        p.x, p.y
                    );
                }
            }
        });

        // 4. Flush peak speeds periodically.
        s.frames += 1;
        if s.frames % 1000 == 0 {
            for (&id, &(cs, csx, csy)) in &s.max_speed {
                let _ = writeln!(s.speed_file, "{id},{cs},{csx},{csy}");
            }
            let _ = s.speed_file.flush();
        }
    }
}

pub(crate) unsafe fn frame_count() -> u32 {
    let api = crate::plus::api();
    match (api.game)() {
        Some(g) => {
            let g = g() as *const bw_dat::structs::Game;
            (*g).frame_count
        }
        None => 0,
    }
}

/// Safe wrapper for the boot report / REPL: 0 when no game is running.
pub(crate) fn frame_count_safe() -> u32 {
    unsafe { frame_count() }
}

/// Total units in the active + hidden lists (REPL diagnostics).
pub(crate) unsafe fn unit_count_safe() -> usize {
    let mut n = 0usize;
    for_each_unit(|_| n += 1);
    n
}

/// Call `f` for every unit in the active + hidden linked lists.
pub(crate) unsafe fn for_each_unit(mut f: impl FnMut(*mut Unit)) {
    let api = crate::plus::api();
    for list in [1u32, 0u32] {
        let getter = if list == 1 {
            (api.first_active_unit)()
        } else {
            (api.first_hidden_unit)()
        };
        let Some(getter) = getter else { continue };
        let mut u = getter() as *mut Unit;
        let mut guard = 0;
        while !u.is_null() && guard < 1_000_000 {
            f(u);
            u = (*u).flingy.next as *mut Unit;
            guard += 1;
        }
    }
}
