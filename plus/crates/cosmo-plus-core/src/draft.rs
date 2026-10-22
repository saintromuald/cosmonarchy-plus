//! Draft mode (Mechabellum-style roguelite) — pure, host-testable logic.
//!
//! Every 2 minutes of game time both players are shown the same 4 options and each
//! picks one (or skips). Options depend on the race pair and the draft number, and are
//! drawn deterministically from a shared seed, so both peers compute the identical
//! tuple with no network round trip. This module owns:
//!
//! * the draft clock (frame -> draft number),
//! * the race-pair pool keying,
//! * the option/effect data model (the tables themselves live in `plus.yml`),
//! * the weighted without-replacement draw,
//! * the draft session state and its fixed-size serialization (the blob the plugin
//!   exports through `cosmo_plus_state_*`).

/// `RaceId::Enum` values (from `BW/enumerations.h` / the mod's `RaceId`), matching
/// [`crate::antimeta::race`].
pub mod race {
    pub const ZERG: u8 = 0;
    pub const TERRAN: u8 = 1;
    pub const PROTOSS: u8 = 2;
    pub const ASKOSI: u8 = 69;
    pub const BLOODSUMMON: u8 = 70;
    /// Sentry value for "no race" (player slot with no race assigned).
    pub const NONE: u8 = 0xFF;
    /// Wildcard pool key: options valid for any race pair.
    pub const ANY: u8 = 0xFE;
}

/// Frames per game second (the mod runs the normal speed cadence; 1 game second = 24
/// frames at the fastest replay-normalised rate).
pub const FRAMES_PER_SECOND: u32 = 24;
/// First draft happens at this many seconds of game time.
pub const FIRST_DRAFT_SECONDS: u32 = 2 * 60;
/// Drafts repeat every this many seconds.
pub const DRAFT_INTERVAL_SECONDS: u32 = 2 * 60;

/// Draft number (1-based) at which mid-tier content unlocks (unit drops, items).
pub const MID_TIER_DRAFT: u32 = 4;
/// Draft number at which late-tier content unlocks (temporary buffs, ult drops).
pub const LATE_TIER_DRAFT: u32 = 6;

/// Number of options shown per draft.
pub const OPTION_COUNT: usize = 4;
/// Maximum playable player slots (PlayerLimit).
pub const PLAYER_LIMIT: usize = 8;

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

    fn next_below(&mut self, n: u64) -> u64 {
        // Modulo bias is irrelevant for pool draws and keeps the function trivially
        // reproducible across peers/languages.
        self.next_u64() % n
    }
}

/// The draft number active at `frame`, or `None` before the first draft.
/// Draft `n` covers `[draft_frame(n), draft_frame(n+1))`.
pub fn draft_number_at_frame(frame: u32) -> Option<u32> {
    let frame = frame as i64;
    let first = FIRST_DRAFT_SECONDS as i64 * FRAMES_PER_SECOND as i64;
    let interval = DRAFT_INTERVAL_SECONDS as i64 * FRAMES_PER_SECOND as i64;
    if frame < first {
        return None;
    }
    Some(1 + ((frame - first) / interval) as u32)
}

/// The exact frame at which draft `n` (1-based) triggers.
pub fn draft_frame(n: u32) -> u32 {
    let first = FIRST_DRAFT_SECONDS * FRAMES_PER_SECOND;
    let interval = DRAFT_INTERVAL_SECONDS * FRAMES_PER_SECOND;
    first + (n - 1) * interval
}

/// The content tier for a draft number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Early,
    Mid,
    Late,
}

pub fn tier_for_draft(draft: u32) -> Tier {
    if draft < MID_TIER_DRAFT {
        Tier::Early
    } else if draft < LATE_TIER_DRAFT {
        Tier::Mid
    } else {
        Tier::Late
    }
}

/// Unordered race-pair key. `pair_key(a, b) == pair_key(b, a)`; the key packs the two
/// race bytes so the pool can be looked up in a table keyed by pair.
pub fn pair_key(a: u8, b: u8) -> u16 {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    (u16::from(lo) << 8) | u16::from(hi)
}

// ---------------------------------------------------------------------------
// Option/effect model. The tables live in `plus.yml` (`draft:` section); this is the
// parsed shape the plugin consumes.
// ---------------------------------------------------------------------------

/// Where a spawned drop anchors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    /// First completed town hall (the anti-meta anchor).
    TownHall,
    /// The player's expansion town hall if one exists, else the town hall.
    Expansion,
}

/// An effect a draft option applies to the player who picked it.
///
/// The vocabulary is deliberately small and maps onto already-proven or cheap engine
/// writes (see `docs/draft-mode-notes.md` §4). New effect kinds extend this enum +
/// the plugin's apply code; the data stays in `plus.yml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Spawn `count` completed units of each id near the anchor.
    SpawnUnits {
        units: Vec<u16>,
        count: u8,
        anchor: Anchor,
    },
    /// Immediate resource injection (clamped at the engine's i32 counters).
    Resources { minerals: i32, gas: i32 },
    /// Full heal of every unit the player owns.
    HealAll,
    /// Refill energy/shields of every unit the player owns.
    RechargeAll,
    /// Reveal the enemy start location (temporary full vision of the area).
    Reveal { seconds: u32 },
    /// Temporary weapon-upgrade levels (+n levels for `seconds`, then restore).
    TempWeaponLevels { levels: i8, seconds: u32 },
    /// Temporary armour-upgrade levels (+n levels for `seconds`, then restore).
    TempArmorLevels { levels: i8, seconds: u32 },
    /// One-shot refund token: next structure cost refunded (state-flag effect).
    RefundToken,
}

/// One draft option as parsed from `plus.yml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftOption {
    /// Stable id (used in logs and picks).
    pub id: String,
    /// Shown name (also rendered on the panel).
    pub name: String,
    /// Minimum draft number for the option to be in the pool.
    pub min_draft: u32,
    /// Pool weight.
    pub weight: u32,
    /// Race-pair filter: `None` = any pair, `Some((a, b))` = exactly this unordered
    /// pair (the plugin resolves the player's race before drawing).
    pub pair: Option<(u8, u8)>,
    pub effect: Effect,
}

/// Deterministic weighted draw of `count` distinct pool indices (weights from the pool).
/// Pools shorter than `count` yield what they have (validation elsewhere requires
/// pools of at least [`OPTION_COUNT`] entries per pair/tier).
pub fn draw_options(seed: u64, weights: &[u32], count: usize) -> Vec<usize> {
    let mut rng = SplitMix64(seed ^ 0x504C_5553_4452_4146); // "PLUSDRAF"
    let mut available: Vec<usize> = (0..weights.len()).collect();
    let mut out = Vec::with_capacity(count);
    while out.len() < count && !available.is_empty() {
        let total: u64 = available.iter().map(|&i| u64::from(weights[i])).sum();
        if total == 0 {
            // All remaining weights are zero: take the first remaining deterministically.
            out.push(available.remove(0));
            continue;
        }
        let mut roll = rng.next_below(total);
        let mut pick = 0;
        for (pos, &i) in available.iter().enumerate() {
            let w = u64::from(weights[i]);
            if roll < w {
                pick = pos;
                break;
            }
            roll -= w;
        }
        out.push(available.remove(pick));
    }
    out
}

// ---------------------------------------------------------------------------
// Draft session state + fixed-size serialization.
//
// The blob is the plugin's `cosmo_plus_state_size/save/load` payload: it must be
// identical on both peers at every save point (rollback) and stable across the
// draft lifetime (replays re-run the commands, but the state lets a load mid-draft
// resume correctly). Versioned: bump `STATE_VERSION` on layout changes.
// ---------------------------------------------------------------------------

pub const STATE_VERSION: u32 = 1;

/// `-1` = player has not responded yet.
pub const PICK_NONE: i8 = -1;
/// The player skipped (pressed Esc / timed out via manual unpause).
pub const PICK_SKIP: i8 = -2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftState {
    /// Draft number in progress (0 = no active draft).
    pub draft: u32,
    /// Next draft number to trigger (1-based; monotonically increasing).
    pub next_draft: u32,
    /// Indices into the (build-version-locked) pool table for the 4 shown options.
    pub options: [u32; OPTION_COUNT],
    /// Per player (0..7): option index 0..3, or `PICK_NONE` / `PICK_SKIP`.
    pub picks: [i8; PLAYER_LIMIT],
    /// Set once the picks have been applied (guards re-application across rollbacks).
    pub applied: bool,
}

impl Default for DraftState {
    fn default() -> Self {
        DraftState {
            draft: 0,
            next_draft: 1,
            options: [0; OPTION_COUNT],
            picks: [PICK_NONE; PLAYER_LIMIT],
            applied: false,
        }
    }
}

impl DraftState {
    pub const ENCODED_SIZE: usize = 4 + 4 + 4 * OPTION_COUNT + PLAYER_LIMIT + 1;

    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(Self::ENCODED_SIZE);
        b.extend_from_slice(&self.draft.to_le_bytes());
        b.extend_from_slice(&self.next_draft.to_le_bytes());
        for o in &self.options {
            b.extend_from_slice(&o.to_le_bytes());
        }
        b.extend_from_slice(&self.picks.map(|p| p as u8));
        b.push(self.applied as u8);
        b
    }

    pub fn decode(data: &[u8]) -> Option<DraftState> {
        if data.len() != Self::ENCODED_SIZE {
            return None;
        }
        let mut s = DraftState::default();
        s.draft = u32::from_le_bytes(data[0..4].try_into().ok()?);
        s.next_draft = u32::from_le_bytes(data[4..8].try_into().ok()?);
        for (i, o) in s.options.iter_mut().enumerate() {
            let off = 8 + i * 4;
            *o = u32::from_le_bytes(data[off..off + 4].try_into().ok()?);
        }
        for (i, p) in s.picks.iter_mut().enumerate() {
            *p = data[8 + 4 * OPTION_COUNT + i] as i8;
        }
        s.applied = data[8 + 4 * OPTION_COUNT + PLAYER_LIMIT] != 0;
        Some(s)
    }

    /// True when every active human player (players with a pick slot in use) has
    /// responded with either a pick or a skip. `active_mask` marks the playable slots
    /// (Human/Computer); neutral/empty slots are ignored.
    pub fn all_responded(&self, active_mask: u8) -> bool {
        (0..PLAYER_LIMIT).all(|p| active_mask & (1 << p) == 0 || self.picks[p] != PICK_NONE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_clock() {
        assert_eq!(draft_number_at_frame(0), None);
        assert_eq!(draft_number_at_frame(draft_frame(1) - 1), None);
        assert_eq!(draft_number_at_frame(draft_frame(1)), Some(1));
        assert_eq!(draft_number_at_frame(draft_frame(1) + 1), Some(1));
        assert_eq!(draft_number_at_frame(draft_frame(4)), Some(4));
        assert_eq!(draft_number_at_frame(draft_frame(6) - 1), Some(5));
        // 2 minutes = 2880 frames.
        assert_eq!(draft_frame(1), 2880);
        assert_eq!(draft_frame(2), 5760);
    }

    #[test]
    fn tiers() {
        assert_eq!(tier_for_draft(1), Tier::Early);
        assert_eq!(tier_for_draft(3), Tier::Early);
        assert_eq!(tier_for_draft(4), Tier::Mid);
        assert_eq!(tier_for_draft(5), Tier::Mid);
        assert_eq!(tier_for_draft(6), Tier::Late);
        assert_eq!(tier_for_draft(20), Tier::Late);
    }

    #[test]
    fn pair_keys_are_unordered() {
        assert_eq!(pair_key(race::ZERG, race::TERRAN), pair_key(race::TERRAN, race::ZERG));
        assert_eq!(pair_key(1, 69), 0x0145);
        assert_eq!(pair_key(69, 1), 0x0145);
    }

    #[test]
    fn draw_is_deterministic_and_distinct() {
        let weights = [10u32, 10, 10, 10, 10, 10, 10, 10];
        let a = draw_options(0x1234_5678, &weights, 4);
        let b = draw_options(0x1234_5678, &weights, 4);
        assert_eq!(a, b, "same seed -> same draw");
        assert_ne!(draw_options(0x1234_5679, &weights, 4), a, "seed change -> draw change");
        let mut seen = std::collections::BTreeSet::new();
        for &i in &a {
            assert!(seen.insert(i), "no replacement: index {i} drawn twice");
        }
        assert_eq!(a.len(), 4);
    }

    #[test]
    fn draw_prefers_weight() {
        let weights = [1000u32, 1];
        let mut wins = 0;
        for seed in 0..200 {
            let d = draw_options(seed, &weights, 1);
            if d[0] == 0 {
                wins += 1;
            }
        }
        assert!(wins > 180, "heavy option should dominate ({wins}/200)");
    }

    #[test]
    fn draw_zero_weight_fallback() {
        // All-zero weights still yield distinct deterministic picks.
        let d = draw_options(7, &[0, 0, 0, 0], 4);
        assert_eq!(d.len(), 4);
        assert_eq!(d, vec![0, 1, 2, 3]);
    }

    #[test]
    fn state_roundtrip() {
        let mut s = DraftState {
            draft: 4,
            next_draft: 5,
            options: [3, 7, 12, 2],
            picks: [PICK_NONE; PLAYER_LIMIT],
            applied: false,
        };
        s.picks[0] = 1;
        s.picks[2] = PICK_SKIP;
        let bytes = s.encode();
        assert_eq!(bytes.len(), DraftState::ENCODED_SIZE);
        assert_eq!(DraftState::decode(&bytes), Some(s.clone()));

        s.applied = true;
        s.picks[5] = 3;
        let bytes = s.encode();
        assert_eq!(DraftState::decode(&bytes), Some(s));
    }

    #[test]
    fn state_rejects_bad_lengths() {
        assert!(DraftState::decode(&[]).is_none());
        assert!(DraftState::decode(&[0; DraftState::ENCODED_SIZE - 1]).is_none());
        assert!(DraftState::decode(&[0; DraftState::ENCODED_SIZE + 1]).is_none());
    }

    #[test]
    fn all_responded_ignores_inactive() {
        let mut s = DraftState::default();
        // Only players 0 and 1 active; neither has responded.
        assert!(!s.all_responded(0b11));
        s.picks[0] = 0;
        assert!(!s.all_responded(0b11));
        s.picks[1] = PICK_SKIP;
        assert!(s.all_responded(0b11));
        // Inactive slots' PICK_NONE must not block.
        s.picks[0] = PICK_NONE;
        assert!(!s.all_responded(0b11), "active player 0 regressed");
    }
}
