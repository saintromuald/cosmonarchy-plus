//! Saturation model: per-node optimal worker count, derived from measured harvest timing.
//!
//! A resource node has exactly **one** mining slot: only one worker is `is_harvesting`
//! at a time (`begin_harvest` sets `node->building.resource.is_mining = 1`; the rest
//! queue in `next_gatherer` — `reference/cosmonarchy-gptp/hooks/harvest.cpp`). The slot
//! services one payload every `MINING_FRAMES` when a worker is always waiting, so the
//! node's throughput ceiling is `1 / MINING_FRAMES` payloads/frame.
//!
//! A single worker's cycle is: travel to node (`d/v`) + mine (`MINING_FRAMES`) + travel
//! back (`d/v`) + drop-off (instant in `return_resources_from_unit`, plus a small
//! turn/approach overhead `DROPOFF_FRAMES`). So its solo cycle is
//! `MINING_FRAMES + round_trip`, where `round_trip = 2*d/v + DROPOFF_FRAMES`.
//!
//! With `n` workers the node delivers `n / (MINING_FRAMES + round_trip)` payloads/frame
//! (no contention) capped at `1 / MINING_FRAMES` (slot ceiling). The smallest `n` that
//! reaches the ceiling is the saturation count:
//!
//! ```text
//! n / (M + R) >= 1 / M   =>   n >= (M + R) / M = 1 + R / M
//! n* = ceil(1 + R / M)
//! ```
//!
//! Only the worker *speed* (trip cadence) enters `R`; payload and `M` are per-node, so
//! the tier-1 vs tier-2 optimum differs purely because their flingy top speeds differ.

/// Game-logic frame rate (the mod's `operator""secs` returns `seconds * 24`).
pub const FPS: f64 = 24.0;
/// Mining service time per payload. `HarvestTimer = 1.75secs = 42` frames.
pub const MINING_FRAMES: f64 = 42.0;
/// Mining service time on a depleted field. `HarvestTimerDepleted = 3.75secs = 90`.
pub const MINING_FRAMES_DEPLETED: f64 = 90.0;
/// Fixed drop-off / turn-around overhead per trip, in frames. The transfer itself is
/// instant; this is the measured approach/turn slack (refined by `docs/measurements`).
pub const DROPOFF_FRAMES: f64 = 0.0;

/// Worker unit ids -> `(tier, race)`. Tier 1 = starting worker, tier 2 = advanced worker.
/// Ids are from `reference/cosmonarchy-gptp/BW/enumerations/UnitId.h` (confirmed against
/// the live `mpq/arr/units.txt`, which is id-indexed).
pub const WORKER_TIERS: &[(u16, u8, &str)] = &[
    (7, 1, "Terran"),   // Terran Mason
    (20, 2, "Terran"),  // Terran Talos
    (494, 2, "Terran"), // Terran Ogun (Worker Mode)
    (41, 1, "Zerg"),    // Zerg Droleth
    (47, 2, "Zerg"),    // Zerg Gosvileth
    (302, 2, "Zerg"),   // Zerg Agrileth
    (64, 1, "Protoss"), // Protoss Scribe
    (184, 2, "Protoss"),// Protoss Prudence
    (495, 2, "Protoss"),// Protoss Autocort
    (439, 1, "Askosi"), // Askosi Pilgrim
    (444, 2, "Askosi"), // Askosi Dyad
    (566, 2, "Askosi"), // Askosi Constituent
];

/// How much one advanced (tier-2) worker counts towards the effective worker
/// count. Ultraload gives it 1.5x the tier-1 move speed while it works a node in
/// range of its base, so below saturation it does 1.5 workers' worth of trips.
/// At saturation the node's single 42-frame mining slot is the ceiling and every
/// extra worker idles, so a saturated advanced base mines exactly what a
/// saturated basic base mines — the advantage is needing fewer workers, not a
/// higher ceiling.
pub const ADVANCED_WORKER_WEIGHT: f64 = 1.5;

/// The effective worker count for a node/depot with `tier1` basic and `tier2`
/// advanced workers: `tier1 + tier2 * ADVANCED_WORKER_WEIGHT`.
pub fn weighted_count(tier1: u32, tier2: u32) -> f64 {
    tier1 as f64 + tier2 as f64 * ADVANCED_WORKER_WEIGHT
}

/// The tier-1 (basic) worker id of the same race as a tier-2 worker id.
/// Ultraload is defined as "1.5x the tier-1 worker speed", so this is the
/// reference value even if the advanced worker's own dat speed differs.
pub fn tier1_worker_of(tier2_id: u16) -> Option<u16> {
    let race = WORKER_TIERS
        .iter()
        .find(|&&(id, _, _)| id == tier2_id)
        .map(|&(_, _, race)| race)?;
    WORKER_TIERS
        .iter()
        .find(|&&(id, tier, r)| id != tier2_id && tier == 1 && r == race)
        .map(|&(id, _, _)| id)
}

/// Whether `tier1` basic + `tier2` advanced workers saturate an `optimal` of
/// `n` workers (effective count >= optimal).
pub fn is_saturated(tier1: u32, tier2: u32, optimal: u32) -> bool {
    weighted_count(tier1, tier2) >= optimal as f64
}

/// The overlay label showing the math: `2+3x1.5=6.5/4` (basic + advanced ×
/// weight = effective / optimal). The effective count is `x.0` or `x.5`, so the
/// fraction is written without trailing zeroes.
pub fn format_weighted(tier1: u32, tier2: u32, optimal: u32) -> String {
    let eff = weighted_count(tier1, tier2);
    let eff_str = if (eff - eff.floor()).abs() < 0.01 {
        format!("{:.0}", eff)
    } else {
        format!("{:.1}", eff)
    };
    format!(
        "{tier1}+{tier2}x{weight}={eff_str}/{optimal}",
        weight = if ADVANCED_WORKER_WEIGHT.fract() == 0.0 {
            format!("{:.0}", ADVANCED_WORKER_WEIGHT)
        } else {
            format!("{}", ADVANCED_WORKER_WEIGHT)
        }
    )
}

// ---------------------------------------------------------------------------
// Flat saturation model (the player-facing default).
//
// The overlay shows plain assigned/capacity numbers: a mineral patch holds
// **3 workers**, a gas node **3 workers**, assuming the base is built as close
// as possible (optimal placement). The distance/speed model above stays for
// the measurement docs, but the in-game labels use these constants.
// ---------------------------------------------------------------------------

/// Workers per mineral patch in the flat model.
pub const MINERAL_WORKERS_PER_PATCH: u32 = 3;
/// Workers per gas node (geyser) in the flat model.
pub const GAS_WORKERS_PER_NODE: u32 = 3;

/// The flat label: `assigned/capacity` (e.g. `12/24`, `2/3`).
pub fn format_flat(assigned: u32, capacity: u32) -> String {
    format!("{assigned}/{capacity}")
}

/// The flat capacity for `patches` mineral fields (3 workers each).
pub fn mineral_capacity(patches: u32) -> u32 {
    patches.saturating_mul(MINERAL_WORKERS_PER_PATCH)
}

/// Mineral-field unit ids (a.k.a. `ResourceMineralField{,_Type2,_Type3}`).
pub const MINERAL_FIELD_IDS: &[u16] = &[176, 177, 178];
/// Neutral gas node ids (`ResourceVespeneGeyser`, `ResourceVespeneRidge`).
pub const GAS_NODE_IDS: &[u16] = &[188, 221];

/// Town-hall / resource-depot ids workers return their cargo to. Confirmed against the
/// live `units.txt`: Terran Ministry (106), Terran Treasury (215), Zerg Hachirosk (131),
/// Protoss Nexus (154), Askosi Landlock (429).
pub const DEPOT_IDS: &[u16] = &[106, 215, 131, 154, 429];

/// The measured functional worker speed in px/frame (flingy `top_speed` 1280 / 256).
/// Every *functional* tier-1 and tier-2 harvest worker shares this speed; the six
/// placeholder tier-2 slots (`top_speed = 1`) are immobile and excluded from the model.
pub const FUNCTIONAL_SPEED_PX: f64 = 5.0;

/// The optimal worker count for a node `distance_px` away, using the measured functional
/// speed. Because that speed is tier-independent, this collapses to a function of distance
/// only: `n* = ceil(1 + (2·D/5.0)/42)` ≈ 2–3 for a normal mineral line.
pub fn optimal_count_at_distance(distance_px: f64) -> u32 {
    optimal_count(FUNCTIONAL_SPEED_PX, distance_px)
}

/// Whether a unit id is a mineral field.
pub fn is_mineral_field(id: u16) -> bool {
    MINERAL_FIELD_IDS.contains(&id)
}

/// Whether a unit id is a neutral gas node (geyser).
pub fn is_gas_node(id: u16) -> bool {
    GAS_NODE_IDS.contains(&id)
}

/// Whether a unit id is a gas extractor (the player building placed on a geyser).
pub fn is_gas_extractor(id: u16) -> bool {
    GAS_EXTRACTOR_TIERS.iter().any(|(_, ids)| ids.contains(&id))
}

/// Whether a unit id is any resource node the overlay tracks (mineral field, geyser, or
/// gas extractor).
pub fn is_resource_node(id: u16) -> bool {
    is_mineral_field(id) || is_gas_node(id) || is_gas_extractor(id)
}

/// `TextColor` values from `BW/enumerations.h` (the values `get_color_handle` maps to a
/// font palette entry).
pub mod text_color {
    pub const WHITE: u8 = 0x04;
    pub const YELLOW: u8 = 0x03;
    pub const RED: u8 = 0x06;
    pub const RIVER_BLUE: u8 = 0x0E;
    pub const PURPLE: u8 = 0x10;
    pub const DARK_GREEN: u8 = 0x18;
    pub const GREY2: u8 = 0x1E;
}

/// The race text colour for a `RaceId::Enum` value (`get_race_textcolor`): Zerg purple,
/// Terran river-blue, Protoss yellow, Askosi dark-green, Bloodsummon red, Kogri grey2,
/// default white.
pub fn race_textcolor(race: u8) -> u8 {
    use text_color::*;
    match race {
        0 => PURPLE,       // RaceId::Zerg
        1 => RIVER_BLUE,   // RaceId::Terran
        2 => YELLOW,       // RaceId::Protoss
        69 => DARK_GREEN,  // RaceId::Askosi
        70 => RED,         // RaceId::Bloodsummon
        71 => GREY2,       // RaceId::Kogri
        _ => WHITE,
    }
}

/// Gas-extractor unit ids per *public* tier (1 = first extractor, matching the mod's
/// `get_gas_tier` values 2/3/4 with the neutral geyser counted as tier 1). From
/// `docs/PLUS-DATA.md` / `harvest.cpp`.
pub const GAS_EXTRACTOR_TIERS: &[(u32, &[u16])] = &[
    (1, &[419, 420, 421, 457]), // Terran Sieve, Protoss Cedent, Zerg Eskigant, Askosi Arch
    (2, &[110, 157, 149, 539]), // Terran Reservoir, Protoss Aquifer, Zerg Excisant, Askosi Abutment
    (3, &[365, 364]),           // Terran Wellbore, Zerg Ezigrisant
];

/// The worker tier (1 or 2) for a unit id, if it is a known worker.
pub fn worker_tier(unit_id: u16) -> Option<u8> {
    WORKER_TIERS
        .iter()
        .find(|&&(id, _, _)| id == unit_id)
        .map(|&(_, tier, _)| tier)
}

/// Convert a flingy `top_speed` dat value (u32, 1/256 px/frame fixed point) to px/frame.
pub fn top_speed_to_px_per_frame(top_speed: u32) -> f64 {
    top_speed as f64 / 256.0
}

/// Round-trip frames (out + back + drop-off) for a worker at `speed_px_per_frame`
/// travelling `distance_px` each way.
pub fn round_trip_frames(speed_px_per_frame: f64, distance_px: f64) -> f64 {
    2.0 * distance_px / speed_px_per_frame.max(f64::MIN_POSITIVE) + DROPOFF_FRAMES
}

/// Optimal worker count saturating a node `distance_px` away, for a worker of the given
/// speed. `n* = ceil(1 + round_trip / MINING_FRAMES)`.
pub fn optimal_count(speed_px_per_frame: f64, distance_px: f64) -> u32 {
    (1.0 + round_trip_frames(speed_px_per_frame, distance_px) / MINING_FRAMES).ceil() as u32
}

/// Node throughput (payloads/frame) with `n` workers.
pub fn throughput(n: u32, speed_px_per_frame: f64, distance_px: f64) -> f64 {
    let n = n.max(1) as f64;
    let rt = round_trip_frames(speed_px_per_frame, distance_px);
    (n / (MINING_FRAMES + rt)).min(1.0 / MINING_FRAMES)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A worker at 1.0 px/frame (top_speed 256), like the base workers.
    #[test]
    fn adjacent_patch_needs_one_worker() {
        // distance 0 -> round trip ~0 -> n* = 1
        assert_eq!(optimal_count(1.0, 0.0), 1);
    }

    #[test]
    fn round_trip_equal_to_mining_needs_two() {
        // round trip = 42 frames (2*d/v = 42 -> d/v = 21) -> n* = 2
        assert_eq!(round_trip_frames(1.0, 21.0), 42.0);
        assert_eq!(optimal_count(1.0, 21.0), 2);
    }

    #[test]
    fn monotone_in_distance() {
        let mut prev = 0;
        for d in (0..=600).step_by(10) {
            let n = optimal_count(1.0, d as f64);
            assert!(n >= prev, "n* must be non-decreasing, got {n} after {prev} at d={d}");
            prev = n;
        }
    }

    #[test]
    fn faster_worker_needs_fewer() {
        // Tier-2 worker at 2x speed needs fewer workers for the same distance.
        for d in [50u32, 100, 200, 400] {
            let d = d as f64;
            assert!(optimal_count(2.0, d) <= optimal_count(1.0, d));
        }
    }

    #[test]
    fn throughput_never_exceeds_slot_ceiling() {
        for d in [0u32, 50, 150, 300] {
            let t = throughput(16, 1.0, d as f64);
            assert!(t <= 1.0 / MINING_FRAMES + 1e-9);
        }
    }

    #[test]
    fn throughput_monotone_in_workers() {
        for d in [0u32, 100, 300] {
            let mut prev = 0.0;
            for n in 1..=12 {
                let t = throughput(n, 1.0, d as f64);
                assert!(t >= prev - 1e-9);
                prev = t;
            }
        }
    }

    #[test]
    fn tier1_reference_for_each_tier2() {
        for &(id, tier, race) in WORKER_TIERS {
            if tier != 2 {
                continue;
            }
            let t1 = tier1_worker_of(id).expect("every tier-2 worker has a tier-1 peer");
            let entry = WORKER_TIERS
                .iter()
                .find(|&&(i, _, _)| i == t1)
                .expect("tier-1 id is in the table");
            assert_eq!(entry.1, 1, "id {t1} must be tier 1");
            assert_eq!(entry.2, race, "id {t1} must be the same race as {id}");
            assert_eq!(tier1_worker_of(t1), None, "tier-1 has no tier-1 reference");
        }
    }

    #[test]
    fn worker_tier_table() {
        assert_eq!(worker_tier(7), Some(1));    // Mason
        assert_eq!(worker_tier(20), Some(2));   // Talos
        assert_eq!(worker_tier(494), Some(2));  // Ogun
        assert_eq!(worker_tier(41), Some(1));   // Droleth
        assert_eq!(worker_tier(64), Some(1));   // Scribe
        assert_eq!(worker_tier(495), Some(2));  // Autocort
        assert_eq!(worker_tier(439), Some(1));  // Pilgrim
        assert_eq!(worker_tier(566), Some(2));  // Constituent
        assert_eq!(worker_tier(9999), None);
    }

    #[test]
    fn speed_scale() {
        assert_eq!(top_speed_to_px_per_frame(256), 1.0);
        assert_eq!(top_speed_to_px_per_frame(512), 2.0);
    }

    #[test]
    fn optimal_at_distance_matches_doc_table() {
        // docs/measurements/saturation.md §4 table, at 5.0 px/frame.
        assert_eq!(optimal_count_at_distance(96.0), 2);   // tight line
        assert_eq!(optimal_count_at_distance(200.0), 3);  // typical line
        assert_eq!(optimal_count_at_distance(400.0), 5);  // distant expansion
        assert_eq!(optimal_count_at_distance(735.0), 8);  // ~23 tiles, unrealistic
    }

    #[test]
    fn optimal_at_distance_monotone() {
        let mut prev = 0;
        for d in (0..=700).step_by(10) {
            let n = optimal_count_at_distance(d as f64);
            assert!(n >= prev, "n* non-decreasing, {n} after {prev} at d={d}");
            prev = n;
        }
    }

    #[test]
    fn resource_node_classification() {
        assert!(is_mineral_field(176));
        assert!(is_mineral_field(178));
        assert!(!is_mineral_field(188));
        assert!(is_gas_node(188));
        assert!(is_gas_node(221));
        assert!(is_gas_extractor(419));
        assert!(is_gas_extractor(364));
        assert!(!is_gas_extractor(188));
        assert!(is_resource_node(176));
        assert!(is_resource_node(188));
        assert!(is_resource_node(419));
        assert!(!is_resource_node(7)); // Mason is a worker, not a node
    }

    #[test]
    fn weighted_count_and_label() {
        assert_eq!(weighted_count(2, 0), 2.0);
        assert_eq!(weighted_count(0, 2), 3.0);
        assert_eq!(weighted_count(1, 1), 2.5);
        assert!(!is_saturated(1, 1, 3));  // 2.5 < 3
        assert!(is_saturated(2, 2, 5));   // 2 + 3 = 5 >= 5
        assert_eq!(format_weighted(2, 0, 3), "2+0x1.5=2/3");
        assert_eq!(format_weighted(2, 3, 4), "2+3x1.5=6.5/4");
        assert_eq!(format_weighted(0, 2, 3), "0+2x1.5=3/3");
    }

    #[test]
    fn race_textcolor_table() {
        assert_eq!(race_textcolor(0), 0x10);  // Zerg -> Purple
        assert_eq!(race_textcolor(1), 0x0E);  // Terran -> RiverBlue
        assert_eq!(race_textcolor(2), 0x03);  // Protoss -> Yellow
        assert_eq!(race_textcolor(69), 0x18); // Askosi -> DarkGreen
        assert_eq!(race_textcolor(70), 0x06); // Bloodsummon -> Red
        assert_eq!(race_textcolor(71), 0x1E); // Kogri -> Grey2
        assert_eq!(race_textcolor(255), 0x04); // default -> White
    }
}
