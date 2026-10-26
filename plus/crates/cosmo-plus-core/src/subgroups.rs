//! SC2-style selection subgroup model (pure logic, unit-testable).
//!
//! StarCraft II divides a mixed selection into **subgroups** — one per distinct unit
//! type, in the order the types appear in the selection — and the Tab key ("Next
//! Subgroup") cycles the **active subgroup** forward, Shift+Tab backward. Only the
//! active subgroup's buttons are shown (its abilities win the hotkeys), while
//! non-ability orders still apply to the whole selection. Production buildings
//! behave identically to units (barracks + factory + starport = three subgroups),
//! and burrowed/unburrowed units count as separate types in SC2 — the caller
//! expresses that by handing in a derived id (e.g. the burrow state folded into
//! the id) so the model stays purely arithmetic.
//!
//! [`SubgroupModel`] owns the whole state machine: given the ordered ids of the
//! current selection plus an opaque identity for it, it decides on every key press
//! whether the selection is fresh (reset to the first subgroup), unchanged (cycle),
//! or single-typed (Tab is inert, exactly like SC2).

/// One subgroup run: the contiguous block of equal unit ids starting at `start`
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

/// The full SC2 subgroup state machine.
///
/// The caller feeds the current selection each time a key is pressed (or the
/// display needs the active group); the model compares the selection identity
/// and returns what to do. The identity must be order-insensitive (the sorted
/// unit-pointer hash the engine crate computes) because the underlying array
/// gets rotated as the model cycles.
///
/// The active subgroup is tracked by **unit id**, never by index: the array
/// order is rotated by the adapter on every cycle, so an index into it would
/// point at a different subgroup after each rotation. The cyclic subgroup
/// order itself is invariant under rotation, so "the next run after the
/// active id in the current order" is the SC2-exact advance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubgroupModel {
    /// Order-insensitive identity of the selection this state belongs to.
    identity: u64,
    /// The canonical subgroup order (unit ids), fixed when the selection is
    /// first seen. The adapter rotates the underlying array on every cycle,
    /// but the canonical order — first appearance in the original selection,
    /// SC2's subgroup order — never changes.
    order: Vec<u16>,
    /// The unit id of the active subgroup (stable across array rotations).
    active_id: u16,
}

/// What one key press should do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubgroupAction {
    /// Single-typed selection: SC2's Tab is inert — do nothing.
    Inert,
    /// The selection changed since the last press: the active subgroup is the
    /// first one (SC2 resets to the first subgroup on a fresh selection).
    Reset { active_id: u16, groups: u32 },
    /// Same selection, cycled: the newly active subgroup. `index` is the
    /// subgroup's position in the canonical order.
    Cycle { active_id: u16, index: u32, groups: u32 },
}

/// The canonical subgroup order: the distinct unit ids by first appearance.
fn canonical_order(ids: &[u16]) -> Vec<u16> {
    subgroup_runs(ids)
        .into_iter()
        .map(|r| r.unit_id)
        .collect()
}

impl SubgroupModel {
    pub fn new(identity: u64, ids: Vec<u16>) -> Self {
        let order = canonical_order(&ids);
        Self {
            identity,
            active_id: order.first().copied().unwrap_or(0),
            order,
        }
    }

    /// The id of the currently active subgroup (the first one when the model
    /// was just created).
    pub fn active_id(&self) -> u16 {
        self.active_id
    }

    /// Feed a key press: `dir` = +1 (Tab) / -1 (Shift+Tab). Returns the action
    /// to take and updates the internal state. A selection whose identity
    /// differs from the model's is a fresh selection: SC2 resets to the first
    /// subgroup without advancing.
    pub fn press(&mut self, identity: u64, ids: Vec<u16>, dir: i32) -> SubgroupAction {
        let fresh = identity != self.identity;
        if fresh {
            self.identity = identity;
            self.order = canonical_order(&ids);
            self.active_id = self.order.first().copied().unwrap_or(0);
        }
        let groups = self.order.len() as u32;
        if groups < 2 {
            return SubgroupAction::Inert;
        }
        if fresh {
            return SubgroupAction::Reset {
                active_id: self.active_id,
                groups,
            };
        }
        let idx = self
            .order
            .iter()
            .position(|&id| id == self.active_id)
            .unwrap_or(0);
        let next = next_active(idx as u32, groups, dir);
        self.active_id = self.order[next as usize];
        SubgroupAction::Cycle {
            active_id: self.active_id,
            index: next,
            groups,
        }
    }

    /// Feed the per-frame view of the selection (no key press). A changed
    /// identity resets to the first subgroup (SC2's fresh-selection rule, so a
    /// mixed selection shows the first subgroup's card even before any Tab);
    /// otherwise the stored active subgroup stands — the array may have been
    /// reordered underneath (the engine re-copies its canonical selection
    /// order on its own events), so the fed ids are only used to detect
    /// freshness.
    pub fn ensure(&mut self, identity: u64, ids: Vec<u16>) -> SubgroupAction {
        let fresh = identity != self.identity;
        if fresh {
            self.identity = identity;
            self.order = canonical_order(&ids);
            self.active_id = self.order.first().copied().unwrap_or(0);
        }
        let groups = self.order.len() as u32;
        if groups < 2 {
            return SubgroupAction::Inert;
        }
        if fresh {
            return SubgroupAction::Reset {
                active_id: self.active_id,
                groups,
            };
        }
        let idx = self
            .order
            .iter()
            .position(|&id| id == self.active_id)
            .unwrap_or(0);
        SubgroupAction::Cycle {
            active_id: self.active_id,
            index: idx as u32,
            groups,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An identity that changes per test selection.
    fn identity_of(ids: &[u16]) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &id in ids {
            h ^= id as u64;
            h = h.wrapping_mul(0x100_0000_01b3);
        }
        h
    }

    #[test]
    fn twelve_mavericks_are_one_subgroup_and_tab_is_inert() {
        let ids = vec![12u16; 12];
        let mut m = SubgroupModel::new(identity_of(&ids), ids.clone());
        assert!(matches!(m.press(identity_of(&ids), ids.clone(), 1), SubgroupAction::Inert));
    }

    #[test]
    fn barracks_factory_starport_are_three_subgroups() {
        // rax, rax, fact, fact, port — three subgroups in selection order.
        let ids = vec![106, 106, 120, 120, 132];
        let mut m = SubgroupModel::new(identity_of(&ids), ids.clone());
        assert_eq!(m.active_id(), 106);
    }

    #[test]
    fn tab_cycles_forward_and_wraps() {
        let ids = vec![106, 106, 120, 132];
        let id = identity_of(&ids);
        let mut m = SubgroupModel::new(id, ids.clone());
        let a = m.press(id, ids.clone(), 1);
        assert_eq!(a, SubgroupAction::Cycle { active_id: 120, index: 1, groups: 3 });
        let b = m.press(id, ids.clone(), 1);
        assert_eq!(b, SubgroupAction::Cycle { active_id: 132, index: 2, groups: 3 });
        let c = m.press(id, ids.clone(), 1);
        assert_eq!(c, SubgroupAction::Cycle { active_id: 106, index: 0, groups: 3 });
    }

    #[test]
    fn shift_tab_cycles_backward() {
        let ids = vec![106, 120, 132];
        let id = identity_of(&ids);
        let mut m = SubgroupModel::new(id, ids.clone());
        let a = m.press(id, ids.clone(), -1);
        assert_eq!(a, SubgroupAction::Cycle { active_id: 132, index: 2, groups: 3 });
    }

    #[test]
    fn fresh_selection_resets_to_the_first_subgroup() {
        let first = vec![106, 120];
        let second = vec![120, 132]; // a different selection
        let id1 = identity_of(&first);
        let id2 = identity_of(&second);
        let mut m = SubgroupModel::new(id1, first.clone());
        let _ = m.press(id1, first.clone(), 1);
        // The player selects something else: the model resets to the first
        // subgroup WITHOUT advancing.
        let a = m.press(id2, second.clone(), 1);
        assert_eq!(a, SubgroupAction::Reset { active_id: 120, groups: 2 });
        // The next press on the same selection cycles normally.
        let b = m.press(id2, second.clone(), 1);
        assert_eq!(b, SubgroupAction::Cycle { active_id: 132, index: 1, groups: 2 });
        // And the active id before any press on the fresh selection is 120.
        let m2 = SubgroupModel::new(id2, second.clone());
        assert_eq!(m2.active_id(), 120);
    }

    #[test]
    fn burrowed_and_unburrowed_are_separate_subgroups_via_derived_ids() {
        // The engine folds the burrow state into the id: 7 = unburrowed
        // roach, 0x8007 = burrowed. The model just sees two different ids.
        let ids = vec![7, 7, 0x8007];
        let id = identity_of(&ids);
        let mut m = SubgroupModel::new(id, ids.clone());
        assert_eq!(m.active_id(), 7);
        let a = m.press(id, ids.clone(), 1);
        assert_eq!(a, SubgroupAction::Cycle { active_id: 0x8007, index: 1, groups: 2 });
    }

    #[test]
    fn mixed_units_and_buildings_share_the_same_rules() {
        // Marines, Medics and a Barracks: same state machine.
        let ids = vec![0, 0, 34, 106];
        let id = identity_of(&ids);
        let mut m = SubgroupModel::new(id, ids.clone());
        assert_eq!(m.active_id(), 0);
    }

    /// The adapter rotates the selection array under the model on every cycle;
    /// the model must keep cycling through ALL subgroups regardless of the
    /// rotated order fed back to it (the engine's array order changes, the
    /// identity — order-insensitive — does not).
    #[test]
    fn cycling_is_invariant_under_array_rotation() {
        // rax, fact, port, nexus — four subgroups.
        let ids = vec![106, 106, 120, 132, 154];
        let id = identity_of(&ids);
        let mut m = SubgroupModel::new(id, ids.clone());

        // Press Tab: active = fact (120). The adapter rotates fact to the
        // front, so the next feed sees the rotated order.
        let a = m.press(id, ids.clone(), 1);
        assert_eq!(a, SubgroupAction::Cycle { active_id: 120, index: 1, groups: 4 });

        let rotated = vec![120, 106, 106, 132, 154]; // fact first now
        // Per-frame ensure on the rotated array: still fact, canonical index 1.
        let e = m.ensure(id, rotated.clone());
        assert_eq!(e, SubgroupAction::Cycle { active_id: 120, index: 1, groups: 4 });

        // Next Tab on the rotated array: port (132), NOT back to rax — the
        // canonical order (rax, fact, port, nexus) is what cycles.
        let b = m.press(id, rotated.clone(), 1);
        assert_eq!(b, SubgroupAction::Cycle { active_id: 132, index: 2, groups: 4 });

        // And again after the adapter rotates port to the front.
        let rotated2 = vec![132, 120, 106, 106, 154];
        let c = m.press(id, rotated2.clone(), 1);
        assert_eq!(c, SubgroupAction::Cycle { active_id: 154, index: 3, groups: 4 });

        // Wraps around to rax (106) after the full cycle.
        let rotated3 = vec![154, 132, 120, 106, 106];
        let d = m.press(id, rotated3.clone(), 1);
        assert_eq!(d, SubgroupAction::Cycle { active_id: 106, index: 0, groups: 4 });
    }

    #[test]
    fn ensure_resets_on_a_fresh_selection_without_advancing() {
        let first = vec![106, 106, 120];
        let second = vec![120, 132];
        let id1 = identity_of(&first);
        let id2 = identity_of(&second);
        let mut m = SubgroupModel::new(id1, first.clone());
        let _ = m.press(id1, first.clone(), 1);
        // The per-frame view sees a fresh selection: back to the first
        // subgroup, no advance.
        assert_eq!(
            m.ensure(id2, second.clone()),
            SubgroupAction::Reset { active_id: 120, groups: 2 }
        );
        // Same selection again: the stored active subgroup stands.
        assert_eq!(
            m.ensure(id2, second.clone()),
            SubgroupAction::Cycle { active_id: 120, index: 0, groups: 2 }
        );
        // And Tab from here advances to the second subgroup.
        assert_eq!(
            m.press(id2, second.clone(), 1),
            SubgroupAction::Cycle { active_id: 132, index: 1, groups: 2 }
        );
    }

    #[test]
    fn ensure_is_inert_for_a_single_typed_selection() {
        let ids = vec![12u16; 5];
        let id = identity_of(&ids);
        let mut m = SubgroupModel::new(id, ids.clone());
        assert!(matches!(m.ensure(id, ids.clone()), SubgroupAction::Inert));
    }
}
