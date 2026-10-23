//! Ultraload — the tier-2 worker passive.
//!
//! A tier-2 (advanced) worker that is working a node inside one of its own bases
//! — i.e. within `ultraload.depot_radius_px` of a completed town hall — moves at
//! `ultraload.speed_multiplier` times the tier-1 worker speed. The worker's
//! `flingy.top_speed` is raised while the condition holds and restored to the
//! dat value when it does not.
//!
//! Why speed and not payload: a resource node has exactly one mining slot that
//! services one payload every 42 frames (`HarvestTimer`), so payload size raises
//! the per-patch ceiling while speed does not. Boosting speed keeps the
//! invariant "a saturated advanced base mines exactly what a saturated basic
//! base mines"; the advanced worker's advantage is that fewer of them are needed
//! to keep every patch busy.
//!
//! Determinism: the condition is derived from snapped unit state (position,
//! player, harvest duty) and the write is idempotent, so every peer computes the
//! same `top_speed` on the same frame, including during rollback re-simulation.

use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};

use cosmo_plus_core::saturation;

/// Speed multiplier as a percentage (150 = 1.5x).
static SPEED_PCT: AtomicU32 = AtomicU32::new(150);
/// Squared depot radius in pixels.
static RADIUS_SQ: AtomicI64 = AtomicI64::new(480 * 480);
/// `ultraload.enabled`; off means "never touch top_speed".
static ENABLED: AtomicU32 = AtomicU32::new(1);
/// Diagnostics: how many boosts have been logged so far.
static LOGGED: AtomicU32 = AtomicU32::new(0);

/// Install-time configuration (called once from `plus::install`).
pub(crate) fn install() {
    let cfg = crate::plus::config()
        .map(|c| c.ultraload)
        .unwrap_or_default();

    let pct = if cfg.enabled {
        (cfg.speed_multiplier * 100.0).round().clamp(100.0, 400.0) as u32
    } else {
        100
    };
    let radius = cfg.depot_radius_px.max(0) as i64;

    SPEED_PCT.store(pct, Ordering::SeqCst);
    RADIUS_SQ.store(radius * radius, Ordering::SeqCst);
    ENABLED.store(u32::from(cfg.enabled && pct > 100), Ordering::SeqCst);

    crate::log::log(&format!(
        "[cosmo-plus] ultraload: {} (speed x{:.2}, depot radius {} px = {} tiles)",
        if ENABLED.load(Ordering::SeqCst) == 1 {
            "enabled"
        } else {
            "disabled"
        },
        pct as f64 / 100.0,
        radius,
        radius / 24,
    ));
}

/// One completed town hall, flattened for the distance test.
#[derive(Clone, Copy)]
struct Depot {
    x: i32,
    y: i32,
    player: u8,
}

/// Per-frame pass: raise or restore each tier-2 worker's top speed.
pub(crate) fn on_game_tick() {
    if ENABLED.load(Ordering::Relaxed) == 0 {
        return;
    }
    let pct = SPEED_PCT.load(Ordering::Relaxed);
    let radius_sq = RADIUS_SQ.load(Ordering::Relaxed);
    if pct <= 100 {
        return;
    }

    unsafe {
        // 1. Collect completed town halls (few dozen at most).
        let mut depots: [Depot; 64] = [Depot {
            x: 0,
            y: 0,
            player: 0,
        }; 64];
        let mut depot_count = 0usize;
        crate::frame::for_each_unit(|u| {
            let id = (*u).unit_id;
            if saturation::DEPOT_IDS.contains(&id) && (*u).flags & 1 != 0 {
                if depot_count < depots.len() {
                    let p = (*u).flingy.position;
                    depots[depot_count] = Depot {
                        x: p.x as i32,
                        y: p.y as i32,
                        player: (*u).player,
                    };
                    depot_count += 1;
                }
            }
        });
        if depot_count == 0 {
            return;
        }

        // 2. Boost tier-2 workers that are on harvest duty inside their base.
        crate::frame::for_each_unit(|u| {
            let id = (*u).unit_id;
            if saturation::worker_tier(id) != Some(2) {
                return;
            }

            let own = bw_dat::FlingyId((*u).flingy.flingy_id).top_speed();
            // Ultraload is defined against the tier-1 worker of the same race.
            let reference = saturation::tier1_worker_of(id)
                .map(|t1| bw_dat::UnitId(t1).flingy().top_speed())
                .filter(|&v| v > 0)
                .unwrap_or(own);
            if reference == 0 || own == 0 {
                return;
            }
            let boosted = (reference as u64 * pct as u64 / 100) as u32;
            if boosted <= own {
                return;
            }

            let worker = (*u).unit_specific.worker;
            let on_duty =
                !worker.current_harvest_target.is_null() || worker.carried_resource_count > 0;
            if !on_duty {
                return;
            }

            let p = (*u).flingy.position;
            let px = p.x as i32;
            let py = p.y as i32;
            let player = (*u).player;
            let in_base = depots[..depot_count].iter().any(|d| {
                d.player == player
                    && (d.x - px) as i64 * (d.x - px) as i64
                        + (d.y - py) as i64 * (d.y - py) as i64
                        <= radius_sq
            });

            let current = (*u).flingy.top_speed;
            // Respect third-party speed modifiers: only touch the field when it
            // holds the worker's own dat value or the value we wrote ourselves.
            if current != own && current != boosted {
                return;
            }
            let want = if in_base { boosted } else { own };
            if current == want {
                return;
            }
            (*u).flingy.top_speed = want;

            let n = LOGGED.fetch_add(1, Ordering::Relaxed);
            if n < 8 {
                crate::log::log(&format!(
                    "[cosmo-plus] ultraload: worker id {id} (player {player}) \
                     speed {current} -> {want} ({})",
                    if in_base { "in base" } else { "left base" }
                ));
            }
        });
    }
}
