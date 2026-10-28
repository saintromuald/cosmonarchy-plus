//! The idle-worker predicate, pure and host-testable.
//!
//! The plugin feeds real engine units through [`UnitView`]; the tests feed a
//! fake. A worker counts as idle when it is completed, carries nothing, is not
//! mid-harvest, is standing still, is not busy constructing/repairing/morphing
//! and not waiting on a queued construction order, and either has no harvest
//! link or sits on a parked order (Stop / Guard / PlayerGuard — the game parks
//! jobless workers on those, and some workers, e.g. Zerg Agrileth, keep a
//! stale harvest target while parked; the parked orders catch those too).

/// Orders a jobless worker can hold while parked.
pub const ORDER_STOP: u8 = 0x01;
pub const ORDER_GUARD: u8 = 0x02;
pub const ORDER_PLAYER_GUARD: u8 = 0x03;

/// Orders a worker holds while it is BUSY building, repairing or morphing —
/// none of these are idle, even when the worker stands still with no harvest
/// link. This is the SC2 5.0.14 rule ("Workers waiting for Refinery, Extractor
/// or Assimilator to finish do not count as idle") applied to the BW order
/// model, and it fixes the reported bug: a Mason mid-construction
/// (`ConstructingBuilding`, 0x21) stood still with no harvest target and was
/// counted idle for exactly the construction window.
///
/// Ids from the mod's `BW/enumerations/OrderId.h` (verified present there):
///   DroneStartBuild 0x19, DroneBuild 0x1A, BuildTerran 0x1E,
///   BuildProtoss1 0x1F, BuildProtoss2 0x20, ConstructingBuilding 0x21,
///   Repair1 0x22, Repair2 0x23, BuildAddon 0x25,
///   MorphUnit 0x2A, MorphStructure 0x2B (a morphing drone is busy).
pub const BUSY_ORDERS: &[u8] = &[
    0x19, 0x1A, 0x1E, 0x1F, 0x20, 0x21, 0x22, 0x23, 0x25, 0x2A, 0x2B,
];

/// The smallest slice of a unit the idle logic needs. Implemented by the engine
/// adapter (`bw_dat::structs::Unit`) and by the test fake.
pub trait UnitView {
    fn unit_id(&self) -> u16;
    fn player(&self) -> u8;
    fn build_time_remaining(&self) -> u16;
    fn carried_resources(&self) -> u8;
    fn is_harvesting(&self) -> bool;
    fn is_moving(&self) -> bool;
    fn has_harvest_target(&self) -> bool;
    fn order(&self) -> u8;
    /// Whether the worker is waiting on a queued construction/repair order
    /// (shift-queued build commands). SC2 does not count such workers idle.
    /// The engine adapter walks the raw order queue; the default is false so
    /// the fake and any adapter without queue access stay safe.
    fn has_queued_build(&self) -> bool {
        false
    }
    /// Whether the worker cannot move at all (placeholder tiers with
    /// `top_speed <= 1`). A permanently-parked placeholder would otherwise
    /// read as "1 idle" for the whole game. Default false.
    fn is_immobile(&self) -> bool {
        false
    }
}

/// A worker is idle when it is completed, carries nothing, is not mid-harvest,
/// is standing still, is not busy (constructing/repairing/morphing, or queued
/// to build), and either has no harvest link or sits on a parked order.
pub fn worker_is_idle(u: &impl UnitView) -> bool {
    if u.build_time_remaining() != 0 {
        return false;
    }
    if u.carried_resources() != 0 || u.is_harvesting() {
        return false;
    }
    // Moving workers are on their way somewhere; only parked workers count.
    if u.is_moving() {
        return false;
    }
    // Constructing/repairing/morphing workers (and workers waiting on queued
    // construction) are busy — never idle, even while standing still.
    if BUSY_ORDERS.contains(&u.order()) || u.has_queued_build() {
        return false;
    }
    if !u.has_harvest_target() {
        return true;
    }
    matches!(
        u.order(),
        ORDER_STOP | ORDER_GUARD | ORDER_PLAYER_GUARD
    )
}

/// An idle worker: a worker-tier unit that is currently idle.
pub fn is_idle_worker(u: &impl UnitView) -> bool {
    crate::saturation::worker_tier(u.unit_id()).is_some() && !u.is_immobile() && worker_is_idle(u)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy)]
    struct Fake {
        unit_id: u16,
        player: u8,
        remaining: u16,
        carried: u8,
        harvesting: bool,
        moving: bool,
        has_target: bool,
        order: u8,
        queued_build: bool,
        immobile: bool,
    }

    impl Fake {
        fn idle(unit_id: u16) -> Self {
            Self {
                unit_id,
                player: 0,
                remaining: 0,
                carried: 0,
                harvesting: false,
                moving: false,
                has_target: false,
                order: 0,
                queued_build: false,
                immobile: false,
            }
        }
    }

    impl UnitView for Fake {
        fn unit_id(&self) -> u16 {
            self.unit_id
        }
        fn player(&self) -> u8 {
            self.player
        }
        fn build_time_remaining(&self) -> u16 {
            self.remaining
        }
        fn carried_resources(&self) -> u8 {
            self.carried
        }
        fn is_harvesting(&self) -> bool {
            self.harvesting
        }
        fn is_moving(&self) -> bool {
            self.moving
        }
        fn has_harvest_target(&self) -> bool {
            self.has_target
        }
        fn order(&self) -> u8 {
            self.order
        }
        fn has_queued_build(&self) -> bool {
            self.queued_build
        }
        fn is_immobile(&self) -> bool {
            self.immobile
        }
    }

    #[test]
    fn parked_worker_without_target_is_idle() {
        assert!(worker_is_idle(&Fake::idle(7)));
    }

    #[test]
    fn incomplete_worker_is_not_idle() {
        let mut u = Fake::idle(7);
        u.remaining = 12;
        assert!(!worker_is_idle(&u));
    }

    #[test]
    fn carrying_or_harvesting_worker_is_not_idle() {
        let mut u = Fake::idle(7);
        u.carried = 1;
        assert!(!worker_is_idle(&u));
        let mut u = Fake::idle(7);
        u.harvesting = true;
        assert!(!worker_is_idle(&u));
    }

    #[test]
    fn moving_worker_is_not_idle() {
        let mut u = Fake::idle(7);
        u.moving = true;
        assert!(!worker_is_idle(&u));
    }

    #[test]
    fn stale_target_is_idle_only_on_parked_orders() {
        // Parked orders count even with a stale harvest target.
        for &order in &[ORDER_STOP, ORDER_GUARD, ORDER_PLAYER_GUARD] {
            let mut u = Fake::idle(7);
            u.has_target = true;
            u.order = order;
            assert!(worker_is_idle(&u), "order {order:#x} should count as parked");
        }
        // Any other order with a target means the worker is busy.
        let mut u = Fake::idle(7);
        u.has_target = true;
        u.order = 0x04;
        assert!(!worker_is_idle(&u));
    }

    #[test]
    fn non_worker_units_are_never_idle_workers() {
        // is_idle_worker also requires the unit id to be a worker tier.
        let u = Fake::idle(0); // id 0 is not in WORKER_TIERS
        assert!(worker_is_idle(&u));
        assert!(!is_idle_worker(&u));
    }

    #[test]
    fn idle_worker_combines_tier_and_state() {
        let mut u = Fake::idle(7); // Terran Mason, tier-1
        u.moving = true;
        assert!(!is_idle_worker(&u));
        assert!(is_idle_worker(&Fake::idle(7)));
    }

    #[test]
    fn constructing_worker_is_never_idle() {
        // The reported bug: a Mason standing still during ConstructingBuilding
        // (0x21) has no harvest link and was counted idle for the build window.
        for &order in BUSY_ORDERS {
            let mut u = Fake::idle(7);
            u.order = order;
            assert!(!worker_is_idle(&u), "busy order {order:#x} must not be idle");
        }
        // Even with a stale harvest target the busy orders win.
        let mut u = Fake::idle(7);
        u.order = 0x21;
        u.has_target = true;
        assert!(!worker_is_idle(&u));
        // Non-busy orders are unaffected.
        let mut u = Fake::idle(7);
        u.order = 0x11;
        assert!(worker_is_idle(&u));
    }

    #[test]
    fn queued_build_worker_is_not_idle() {
        let mut u = Fake::idle(7);
        u.queued_build = true;
        assert!(!worker_is_idle(&u));
        assert!(!is_idle_worker(&u));
    }

    #[test]
    fn immobile_placeholder_is_not_an_idle_worker() {
        let mut u = Fake::idle(20); // Talos, immobile tier-2 placeholder
        u.immobile = true;
        assert!(worker_is_idle(&u));
        assert!(!is_idle_worker(&u));
    }
}
