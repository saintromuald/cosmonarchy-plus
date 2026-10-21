//! Anti-meta mode: one symmetric start roll per game.
//!
//! Pure, host-testable. The roll is derived from a seed that both peers share (the
//! game's synced RNG seed), so every client computes the same tuple without any
//! network round trip. Ranges are inclusive and quantised as the mode specifies:
//!
//! * workers  : 4..=8
//! * minerals : 100..=300 in steps of 5
//! * gas      :   0..=125 in steps of 5

/// One game's start roll. Applied identically to every active player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Roll {
    pub workers: u8,
    pub minerals: u32,
    pub gas: u32,
}

/// SplitMix64: tiny, fast, deterministic, no dependencies.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn next_below(&mut self, n: u32) -> u32 {
        // Modulo bias is irrelevant for these small ranges and keeps the function
        // trivially reproducible across peers/languages.
        (self.next_u64() % u64::from(n)) as u32
    }
}

/// Derive the roll from a shared seed.
pub fn roll(seed: u32) -> Roll {
    let mut rng = SplitMix64(u64::from(seed) ^ 0x504C_5553_414E_5449); // "PLUSANTI"
    Roll {
        workers: 4 + rng.next_below(5) as u8,
        minerals: 100 + 5 * rng.next_below(41),
        gas: 5 * rng.next_below(26),
    }
}

// ---------------------------------------------------------------------------
// Pure start-state helpers (host-testable, no engine access). The engine side
// (`cosmo-plus/src/antimeta.rs`) uses these to plan each player's adjustment.
// ---------------------------------------------------------------------------

/// `RaceId::Enum` values (from `BW/enumerations.h` / the mod's `RaceId`), matching the
/// `race_textcolor` table in [`crate::saturation`].
pub mod race {
    pub const ZERG: u8 = 0;
    pub const TERRAN: u8 = 1;
    pub const PROTOSS: u8 = 2;
    pub const ASKOSI: u8 = 69;
}

/// Tier-1 (starting) worker unit id for a race, mirroring the mod's
/// `get_racial_worker_id` (`CM/cm_ids.cpp`): Mason / Droleth / Scribe / Pilgrim.
/// Returns `None` for a race with no worker (Bloodsummon, Kogri, Neutral, …).
pub fn racial_worker_id(race: u8) -> Option<u16> {
    Some(match race {
        race::TERRAN => 7,    // Terran Mason
        race::ZERG => 41,     // Zerg Droleth
        race::PROTOSS => 64,  // Protoss Scribe
        race::ASKOSI => 439,  // Askosi Pilgrim
        _ => return None,
    })
}

/// Town-hall unit id for a race, mirroring `get_racial_town_center_id`. Returns `None`
/// for a race with no town hall.
pub fn racial_town_hall_id(race: u8) -> Option<u16> {
    Some(match race {
        race::TERRAN => 106,  // Terran Ministry
        race::ZERG => 131,    // Zerg Hachirosk
        race::PROTOSS => 154, // Protoss Nexus
        race::ASKOSI => 429,  // Askosi Landlock
        _ => return None,
    })
}

/// The tier-1 worker ids counted and spawned by the mode (one per playable race).
pub const TIER1_WORKER_IDS: &[u16] = &[7, 41, 64, 439];

/// How many workers to remove / add to move `current` to `target`.
/// Returns `(to_remove, to_add)`; at most one of the two is non-zero.
pub fn worker_adjustment(current: u32, target: u32) -> (u32, u32) {
    if current >= target {
        (current - target, 0)
    } else {
        (0, target - current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_hold_for_many_seeds() {
        for seed in 0..100_000u32 {
            let r = roll(seed);
            assert!((4..=8).contains(&r.workers), "{r:?}");
            assert!((100..=300).contains(&r.minerals), "{r:?}");
            assert_eq!(r.minerals % 5, 0, "{r:?}");
            assert!(r.gas <= 125, "{r:?}");
            assert_eq!(r.gas % 5, 0, "{r:?}");
        }
    }

    #[test]
    fn deterministic() {
        assert_eq!(roll(12345), roll(12345));
        assert_ne!(roll(1), roll(2));
    }

    #[test]
    fn endpoints_reachable() {
        let mut saw_low = false;
        let mut saw_high = false;
        let mut saw_zero_gas = false;
        let mut saw_max_gas = false;
        for seed in 0..10_000u32 {
            let r = roll(seed);
            saw_low |= r.workers == 4 && r.minerals == 100 && r.gas == 0;
            saw_high |= r.workers == 8 && r.minerals == 300 && r.gas == 125;
            saw_zero_gas |= r.gas == 0;
            saw_max_gas |= r.gas == 125;
        }
        assert!(saw_zero_gas && saw_max_gas, "gas endpoints not hit");
        // Low/high corners are rare; only require that both extremes of each axis occur.
        let mut w = [false; 9];
        let mut m = [false; 61];
        for seed in 0..10_000u32 {
            let r = roll(seed);
            w[r.workers as usize] = true;
            m[((r.minerals - 100) / 5) as usize] = true;
        }
        assert!(w[4] && w[8], "worker endpoints not hit");
        assert!(m[0] && m[40], "mineral endpoints not hit");
        let _ = (saw_low, saw_high);
    }

    #[test]
    fn worker_adjustment_plan() {
        assert_eq!(worker_adjustment(6, 8), (0, 2));
        assert_eq!(worker_adjustment(6, 4), (2, 0));
        assert_eq!(worker_adjustment(6, 6), (0, 0));
        assert_eq!(worker_adjustment(0, 8), (0, 8));
        assert_eq!(worker_adjustment(12, 4), (8, 0));
    }

    #[test]
    fn racial_ids_match_authoritative_mapping() {
        assert_eq!(racial_worker_id(race::TERRAN), Some(7));
        assert_eq!(racial_worker_id(race::ZERG), Some(41));
        assert_eq!(racial_worker_id(race::PROTOSS), Some(64));
        assert_eq!(racial_worker_id(race::ASKOSI), Some(439));
        assert_eq!(racial_worker_id(70), None); // Bloodsummon has no worker
        assert_eq!(racial_town_hall_id(race::TERRAN), Some(106));
        assert_eq!(racial_town_hall_id(race::ZERG), Some(131));
        assert_eq!(racial_town_hall_id(race::PROTOSS), Some(154));
        assert_eq!(racial_town_hall_id(race::ASKOSI), Some(429));
        assert_eq!(racial_town_hall_id(70), None);
    }
}
