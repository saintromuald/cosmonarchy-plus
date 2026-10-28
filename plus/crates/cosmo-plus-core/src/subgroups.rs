//! SC2-style selection subgroup model (pure logic, unit-testable).
//!
//! StarCraft II divides a mixed selection into **subgroups** — one per distinct unit
//! type, in the order the types appear in the selection — and the Tab key ("Next
//! Subgroup") cycles the **active subgroup** forward, Shift+Tab backward. Only the
//! active subgroup's buttons are shown (its abilities win the hotkeys), while
//! non-ability orders still apply to the whole selection. Production buildings
//! behave identically to units, and burrowed/unburrowed units count as separate
//! types in SC2 (Patch 4.9.1) — the caller expresses that by handing in a derived
//! id (the burrow state folded into the id) so the model stays purely arithmetic.
//!
//! [`SubgroupModel`] owns the state machine. The adapter feeds it:
//!   * an **identity** — an order-insensitive hash over the whole selection, so
//!     membership changes (deaths, shift-deselects) are visible even though the
//!     underlying array rotates and the engine re-copies canonical order;
//!   * the **canonical order of cycle-worthy run keys** — each key encodes
//!     (derived id, ordinal of the run among equal derived ids). Keys are
//!     computed from the engine's *canonical* selection (never rotated by the
//!     adapter), so ordinals are stable across rotations, canonical re-copies,
//!     and head-of-run deaths (a run keeps its key while it has any members).
//!     An id-keyed model would always find the first occurrence and bounce
//!     between duplicate-type runs forever; pointer-token keys died with the
//!     run's first unit. (Both were real bugs in earlier iterations.)
//!
//! Three entry points:
//!   * [`SubgroupModel::select`] — a **selection event** (box/click/recall):
//!     SC2 resets to the first subgroup, no advance.
//!   * [`SubgroupModel::press`] — a Tab press: advance (or retreat) the active
//!     subgroup cyclically. If the selection changed since the last entry
//!     point, the active subgroup is kept when it still exists (SC2 keeps the
//!     subgroup alive while it has members) and reset to the first worthy
//!     subgroup otherwise.
//!   * [`SubgroupModel::ensure`] — the per-frame view: same keep-or-reset
//!     semantics, no advance.
//!
//! Fewer than two worthy runs makes every entry point [`SubgroupAction::Inert`]
//! (Tab is inert on a single-typed selection, exactly like SC2).

/// The derived-id flag for a burrowed unit (`derived_id` below).
pub const BURROW_MASK: u16 = 0x4000;
/// Orders that mean "burrowed" for the derived-id fold: the burrow windup
/// (`Burrow`, 0x74) and the settled burrowed state (`Burrowed`, 0x75).
/// A unit with `Unburrow` (0x76) counts as unburrowed.
pub const BURROW_ORDERS: &[u8] = &[0x74, 0x75];

/// Fold the burrow state into a unit id: burrowed/unburrowed units form
/// separate subgroups, like SC2 (Patch 4.9.1). Both adapter call sites must
/// use this, or the two paths produce different run sets.
pub fn derived_id(unit_id: u16, order: u8) -> u16 {
    if BURROW_ORDERS.contains(&order) {
        unit_id | BURROW_MASK
    } else {
        unit_id
    }
}

/// Strip the burrow flag for classification / name lookups.
pub fn base_id(derived: u16) -> u16 {
    derived & !BURROW_MASK
}

/// The stable key of one run: (derived id, ordinal among equal derived ids).
pub fn run_key(derived: u16, ordinal: u16) -> u32 {
    (u32::from(derived) << 16) | u32::from(ordinal)
}

/// Decode a run key back to (derived id, ordinal).
pub fn key_parts(key: u32) -> (u16, u16) {
    ((key >> 16) as u16, (key & 0xFFFF) as u16)
}

/// Assign the canonical order of keys to a list of derived ids, in canonical
/// (first-appearance) order: ordinal i for the (i+1)-th run of a given
/// derived id.
pub fn run_keys(runs: &[u16]) -> Vec<u32> {
    let mut next_ordinal: Vec<(u16, u16)> = Vec::new(); // (derived id, ordinal next)
    runs.iter()
        .map(|&derived| {
            let ordinal = match next_ordinal.iter_mut().find(|(d, _)| *d == derived) {
                Some(entry) => {
                    let ordinal = entry.1;
                    entry.1 += 1;
                    ordinal
                }
                None => {
                    next_ordinal.push((derived, 1));
                    0
                }
            };
            run_key(derived, ordinal)
        })
        .collect()
}

/// One subgroup run: the contiguous block of equal ids starting at `start`
/// (inclusive), `len` units long, all of `unit_id`. Runs are ordered by first
/// appearance, matching SC2's subgroup order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubgroupRun {
    /// Index into the selection where this subgroup starts.
    pub start: usize,
    /// Number of units in this subgroup.
    pub len: usize,
    /// The (possibly derived) unit id of this subgroup.
    pub unit_id: u16,
}

/// Split the ordered selection ids into subgroups: consecutive equal ids form one
/// run, ordered by first appearance.
pub fn subgroup_runs(ids: &[u16]) -> Vec<SubgroupRun> {
    let mut runs: Vec<SubgroupRun> = Vec::new();
    let mut i = 0usize;
    while i < ids.len() {
        let mut j = i + 1;
        while j < ids.len() && ids[j] == ids[i] {
            j += 1;
        }
        runs.push(SubgroupRun {
            start: i,
            len: j - i,
            unit_id: ids[i],
        });
        i = j;
    }
    runs
}

/// Advance (or retreat) the active subgroup index cyclically. `groups` is the
/// number of subgroups; `dir` is +1 (Tab) or -1 (Shift+Tab).
pub fn next_active(active: u32, groups: u32, dir: i32) -> u32 {
    if groups < 2 {
        return 0;
    }
    (active as i32 + dir).rem_euclid(groups as i32) as u32
}

/// The full SC2 subgroup state machine. See the module docs for the contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubgroupModel {
    /// Order-insensitive identity of the selection this state belongs to.
    identity: u64,
    /// The canonical subgroup order: the cycle-worthy runs' keys, in
    /// first-appearance order.
    order: Vec<u32>,
    /// The active run's key (stable across array rotations and deaths).
    active_key: u32,
}

/// What one interaction should do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubgroupAction {
    /// Nothing worth cycling (fewer than two worthy runs): Tab is inert.
    Inert,
    /// The active subgroup is (re)set to the first worthy one.
    Reset { active_key: u32, groups: u32 },
    /// The active subgroup stands or advanced. `index` is its position in the
    /// canonical order.
    Cycle { active_key: u32, index: u32, groups: u32 },
}

impl SubgroupModel {
    pub fn new(identity: u64, keys: Vec<u32>) -> Self {
        Self {
            identity,
            active_key: keys.first().copied().unwrap_or(0),
            order: keys,
        }
    }

    /// The active run's key (0 when nothing is active).
    pub fn active_key(&self) -> u32 {
        self.active_key
    }

    /// The canonical order (keys).
    pub fn order(&self) -> &[u32] {
        &self.order
    }

    fn adopt(&mut self, identity: u64, keys: Vec<u32>) -> bool {
        // true = the active key survived the identity/order change.
        let changed = identity != self.identity || keys != self.order;
        let survives = keys.contains(&self.active_key);
        self.identity = identity;
        self.order = keys;
        changed && survives
    }

    /// Feed a selection event (box/click/recall/our own idle-larva picks):
    /// SC2 resets to the first subgroup without advancing.
    pub fn select(&mut self, identity: u64, keys: Vec<u32>) -> SubgroupAction {
        let _ = self.adopt(identity, keys.clone());
        let groups = self.order.len() as u32;
        if groups < 2 {
            self.active_key = self.order.first().copied().unwrap_or(0);
            return SubgroupAction::Inert;
        }
        self.active_key = self.order[0];
        SubgroupAction::Reset {
            active_key: self.active_key,
            groups,
        }
    }

    /// Feed a key press: `dir` = +1 (Tab) / -1 (Shift+Tab). Advances from the
    /// current subgroup; when the previous active subgroup vanished (deaths/
    /// deselection since the last entry point) the cycle restarts from the
    /// first worthy subgroup and then advances — so a Tab right after a fresh
    /// selection moves to the SECOND worthy subgroup, exactly like SC2.
    pub fn press(&mut self, identity: u64, keys: Vec<u32>, dir: i32) -> SubgroupAction {
        let _ = self.adopt(identity, keys.clone());
        let groups = self.order.len() as u32;
        if groups < 2 {
            self.active_key = self.order.first().copied().unwrap_or(0);
            return SubgroupAction::Inert;
        }
        let idx = self
            .order
            .iter()
            .position(|&k| k == self.active_key)
            .unwrap_or(0);
        let next = next_active(idx as u32, groups, dir);
        self.active_key = self.order[next as usize];
        SubgroupAction::Cycle {
            active_key: self.active_key,
            index: next,
            groups,
        }
    }

    /// Feed the per-frame view of the selection (no key press). The stored
    /// active subgroup stands while it has members; if it vanished the first
    /// worthy subgroup becomes active.
    pub fn ensure(&mut self, identity: u64, keys: Vec<u32>) -> SubgroupAction {
        let _ = self.adopt(identity, keys.clone());
        let groups = self.order.len() as u32;
        if groups < 2 {
            self.active_key = self.order.first().copied().unwrap_or(0);
            return SubgroupAction::Inert;
        }
        match self.order.iter().position(|&k| k == self.active_key) {
            Some(idx) => SubgroupAction::Cycle {
                active_key: self.active_key,
                index: idx as u32,
                groups,
            },
            None => {
                // The active subgroup lost its last member: restart at the
                // first worthy subgroup (SC2 resets when the subgroup is gone).
                self.active_key = self.order[0];
                SubgroupAction::Reset {
                    active_key: self.active_key,
                    groups,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An identity that changes per test selection.
    fn identity_of(tokens: &[u32]) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &t in tokens {
            h ^= t as u64;
            h = h.wrapping_mul(0x100_0000_01b3);
        }
        h
    }

    fn keys_of(derived: &[u16]) -> Vec<u32> {
        run_keys(derived)
    }

    #[test]
    fn fewer_than_two_worthy_runs_is_inert() {
        let keys = keys_of(&[7]);
        let id = identity_of(&[100]);
        let mut m = SubgroupModel::new(id, keys.clone());
        assert!(matches!(m.press(id, keys.clone(), 1), SubgroupAction::Inert));
        assert!(matches!(m.select(id, keys.clone()), SubgroupAction::Inert));
        assert!(matches!(m.ensure(id, keys.clone()), SubgroupAction::Inert));
    }

    #[test]
    fn tab_cycles_forward_and_wraps() {
        let keys = keys_of(&[7, 41, 64]);
        let id = identity_of(&[1, 2, 3]);
        let mut m = SubgroupModel::new(id, keys.clone());
        assert_eq!(m.press(id, keys.clone(), 1), SubgroupAction::Cycle {
            active_key: keys[1], index: 1, groups: 3
        });
        assert_eq!(m.press(id, keys.clone(), 1), SubgroupAction::Cycle {
            active_key: keys[2], index: 2, groups: 3
        });
        assert_eq!(m.press(id, keys.clone(), 1), SubgroupAction::Cycle {
            active_key: keys[0], index: 0, groups: 3
        });
    }

    #[test]
    fn shift_tab_cycles_backward() {
        let keys = keys_of(&[7, 41, 64]);
        let id = identity_of(&[1, 2, 3]);
        let mut m = SubgroupModel::new(id, keys.clone());
        assert_eq!(m.press(id, keys.clone(), -1), SubgroupAction::Cycle {
            active_key: keys[2], index: 2, groups: 3
        });
    }

    #[test]
    fn selection_event_resets_to_first_without_advancing() {
        let keys = keys_of(&[7, 41, 64]);
        let id = identity_of(&[1, 2, 3]);
        let mut m = SubgroupModel::new(id, keys.clone());
        let _ = m.press(id, keys.clone(), 1); // active = 41
        let _ = m.press(id, keys.clone(), 1); // active = 64
        let other_id = identity_of(&[9, 9, 9]);
        assert_eq!(m.select(other_id, keys.clone()), SubgroupAction::Reset {
            active_key: keys[0], groups: 3
        });
        // Tab right after the fresh selection advances to the SECOND subgroup.
        assert_eq!(m.press(other_id, keys.clone(), 1), SubgroupAction::Cycle {
            active_key: keys[1], index: 1, groups: 3
        });
    }

    #[test]
    fn membership_change_keeps_active_subgroup() {
        // A unit died / was shift-deselected, but the active subgroup survives.
        let keys = keys_of(&[7, 41, 64]);
        let id = identity_of(&[1, 2, 3, 4, 5]);
        let mut m = SubgroupModel::new(id, keys.clone());
        let _ = m.press(id, keys.clone(), 1); // active = 41
        let smaller = identity_of(&[1, 3, 4, 5]);
        assert_eq!(m.ensure(smaller, keys.clone()), SubgroupAction::Cycle {
            active_key: keys[1], index: 1, groups: 3
        });
        // And a Tab from there advances.
        assert_eq!(m.press(smaller, keys.clone(), 1), SubgroupAction::Cycle {
            active_key: keys[2], index: 2, groups: 3
        });
    }

    #[test]
    fn vanished_active_subgroup_resets_to_first() {
        let keys = keys_of(&[7, 41, 64]);
        let id = identity_of(&[1, 2, 3]);
        let mut m = SubgroupModel::new(id, keys.clone());
        let _ = m.press(id, keys.clone(), 1); // active = 41
        // The 41-run died: only [7, 64] remain (new identity, 41 missing).
        let new_keys = run_keys(&[7, 64]);
        let smaller = identity_of(&[1, 3]);
        assert_eq!(m.ensure(smaller, new_keys.clone()), SubgroupAction::Reset {
            active_key: new_keys[0], groups: 2
        });
        assert_eq!(m.press(smaller, new_keys.clone(), 1), SubgroupAction::Cycle {
            active_key: new_keys[1], index: 1, groups: 2
        });
    }

    #[test]
    fn duplicate_ids_get_distinct_ordinals_and_cycle_every_run() {
        // The duplicate-type bounce regression: A-run, L-run, B-run where A and
        // B share a derived id. Keys must stay distinct and stable no matter
        // how the adapter rotates the underlying array.
        let keys = run_keys(&[7, 41, 7]); // A(7,0), L(41,0), B(7,1)
        assert_ne!(keys[0], keys[2]);
        let id = identity_of(&[1, 2, 3]);
        let mut m = SubgroupModel::new(id, keys.clone());
        let mut order_seen = Vec::new();
        for _ in 0..3 {
            match m.press(id, keys.clone(), 1) {
                SubgroupAction::Cycle { active_key, .. } => order_seen.push(active_key),
                other => panic!("expected Cycle, got {other:?}"),
            }
        }
        assert_eq!(order_seen, vec![keys[1], keys[2], keys[0]]);
        // ensure() with the same keys keeps the same active key (rotation-proof).
        assert_eq!(m.ensure(id, keys.clone()), SubgroupAction::Cycle {
            active_key: keys[0], index: 0, groups: 3
        });
    }

    #[test]
    fn run_keys_are_stable_when_the_array_is_reordered() {
        // The adapter computes keys from the CANONICAL array; this test pins
        // that canonical order = first appearance. Reordering the input would
        // change ordinals, so the adapter must never feed rotated arrays.
        let canonical = run_keys(&[7, 41, 7, 64, 7]);
        let (d0, o0) = key_parts(canonical[0]);
        let (d2, o2) = key_parts(canonical[2]);
        let (d4, o4) = key_parts(canonical[4]);
        assert_eq!((d0, o0), (7, 0));
        assert_eq!((d2, o2), (7, 1));
        assert_eq!((d4, o4), (7, 2));
    }

    #[test]
    fn derived_id_folds_only_burrowed_states() {
        assert_eq!(derived_id(7, 0x74), 7 | BURROW_MASK);
        assert_eq!(derived_id(7, 0x75), 7 | BURROW_MASK);
        assert_eq!(derived_id(7, 0x76), 7); // unburrowing = unburrowed
        assert_eq!(derived_id(7, 0x00), 7);
        assert_eq!(base_id(derived_id(7, 0x74)), 7);
    }
}
