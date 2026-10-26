//! SC2-style smart casting (pure logic, unit-testable).
//!
//! StarCraft II has two casting behaviours for a group of casters:
//!
//! * **Smart cast** — one press casts with **one** unit: the closest eligible
//!   caster (enough energy, in range) acts; the next press picks the next.
//!   Applies to targeted spells by default (storm, lockdown, force field, ...).
//! * **All at once** — every selected unit acts simultaneously. SC2's
//!   non-smartcast class: stim, burrow, blink, cloaking, transformations.
//!
//! The player's selection never changes; the command itself is narrowed to a
//! single caster by the engine adapter.
//!
//! Classification: `plus.yml` may pin a unit type to a mode
//! (`cast-modes: { "12": all, ... }`); otherwise the SC2 default applies —
//! **targeted** spell orders smart-cast, **untargeted** ones (stim-like)
//! cast all at once. Holding Shift while casting forces all-at-once (the
//! Blizzard FAQ's suggested override).

/// How a spell command issued with several casters selected behaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastMode {
    /// One eligible caster acts per press (closest to the target).
    Smart,
    /// Every selected unit acts at once.
    All,
}

/// Untargeted orders that SC2 treats as cast-all (stim, burrow, cloaking,
/// transformations, siege/viking-style morphs). Everything else targeted
/// defaults to smart cast. These are **Cosmonarchy's** order ids (the mod's
/// orders.dat remapped the vanilla slots — vanilla 0x4D is "Idle (Power-Up)"
/// in the mod, not Siege Mode); the mod's orders.txt is the reference, and
/// the per-unit cast-modes table in plus.yml can pin individual units.
const DEFAULT_ALL_AT_ONCE: &[u8] = &[
    98,  // Siege Mode (siege tanks, morph-class)
    99,  // Tank Mode
    109, // Cloak
    110, // Decloak
    116, // Burrow
    118, // Unburrow
];

/// The SC2 default mode for an order: untargeted (cast-all class) orders are
/// `All`; targeted spell orders are `Smart`.
pub fn default_mode(order_id: u8) -> CastMode {
    if DEFAULT_ALL_AT_ONCE.contains(&order_id) {
        CastMode::All
    } else {
        CastMode::Smart
    }
}

/// Resolve the effective mode for a cast: an explicit `plus.yml` override for
/// the unit type wins; otherwise the SC2 default for the order. Holding Shift
/// (the override modifier) forces `All`.
pub fn resolve_mode(order_id: u8, unit_id: u16, override_all: Option<&CastMode>, shift_held: bool) -> CastMode {
    if shift_held {
        return CastMode::All;
    }
    match override_all {
        Some(mode) => *mode,
        None => default_mode(order_id),
    }
}

/// A caster candidate for the one-per-press pick: its opaque id, position in
/// world pixels, and whether it can cast (energy/cooldown — the adapter checks).
#[derive(Debug, Clone, Copy)]
pub struct Caster {
    pub id: u32,
    pub x: i32,
    pub y: i32,
    pub eligible: bool,
}

/// Pick the single caster for a smart cast: the **eligible** one closest to
/// the target point (SC2 picks the closest eligible caster). Falls back to
/// the first eligible, then to the first caster at all.
pub fn pick_caster(casters: &[Caster], target_x: i32, target_y: i32) -> Option<u32> {
    let mut best: Option<&Caster> = None;
    let mut best_dist = i64::MAX;
    let mut any: Option<&Caster> = None;
    for c in casters {
        if c.eligible {
            let dx = (c.x - target_x) as i64;
            let dy = (c.y - target_y) as i64;
            let d = dx * dx + dy * dy;
            if d < best_dist {
                best_dist = d;
                best = Some(c);
            }
        } else if any.is_none() {
            any = Some(c);
        }
    }
    best.or(any).map(|c| c.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stim_is_all_at_once_by_default() {
        // Cosmonarchy order ids (orders.txt): siege/tank/cloak/burrow.
        assert_eq!(default_mode(98), CastMode::All); // Siege Mode
        assert_eq!(default_mode(99), CastMode::All); // Tank Mode
        assert_eq!(default_mode(109), CastMode::All); // Cloak
        assert_eq!(default_mode(116), CastMode::All); // Burrow
    }

    #[test]
    fn targeted_spells_are_smart_by_default() {
        assert_eq!(default_mode(142), CastMode::Smart); // Psionic Storm (targeted)
        assert_eq!(default_mode(144), CastMode::Smart); // Plague
    }

    #[test]
    fn shift_forces_all() {
        assert_eq!(resolve_mode(142, 12, None, true), CastMode::All);
        assert_eq!(resolve_mode(98, 12, None, true), CastMode::All);
    }

    #[test]
    fn explicit_override_wins_over_the_default() {
        let o = Some(CastMode::All);
        assert_eq!(resolve_mode(142, 12, o.as_ref(), false), CastMode::All);
        let s = Some(CastMode::Smart);
        assert_eq!(resolve_mode(98, 12, s.as_ref(), false), CastMode::Smart);
    }

    #[test]
    fn picks_the_closest_eligible_caster() {
        let casters = [
            Caster { id: 1, x: 1000, y: 1000, eligible: true },
            Caster { id: 2, x: 100, y: 100, eligible: true },
            Caster { id: 3, x: 50, y: 50, eligible: false }, // no energy
        ];
        assert_eq!(pick_caster(&casters, 120, 120), Some(2));
    }

    #[test]
    fn falls_back_to_the_first_caster_when_nobody_is_eligible() {
        let casters = [
            Caster { id: 7, x: 0, y: 0, eligible: false },
            Caster { id: 8, x: 900, y: 900, eligible: false },
        ];
        assert_eq!(pick_caster(&casters, 0, 0), Some(7));
    }
}
