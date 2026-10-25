//! The idle-worker predicate, pure and host-testable.
//!
//! The plugin feeds real engine units through [`UnitView`]; the tests feed a
//! fake. A worker counts as idle when it is completed, carries nothing, is not
//! mid-harvest, is standing still, and either has no harvest link or sits on a
//! parked order (Stop / Guard / PlayerGuard — the game parks jobless workers on
//! those, and some workers, e.g. Zerg Agrileth, keep a stale harvest target
//! while parked; the parked orders catch those too).

/// Orders a jobless worker can hold while parked.
pub const ORDER_STOP: u8 = 0x01;
pub const ORDER_GUARD: u8 = 0x02;
pub const ORDER_PLAYER_GUARD: u8 = 0x03;

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
}

/// A worker is idle when it is completed, carries nothing, is not mid-harvest,
/// is standing still, and either has no harvest link or sits on a parked order.
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
    crate::saturation::worker_tier(u.unit_id()).is_some() && worker_is_idle(u)
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
}
