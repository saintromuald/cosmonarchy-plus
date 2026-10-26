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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubgroupModel {
    /// Order-insensitive identity of the selection this state belongs to.
    identity: u64,
    /// The ordered (possibly derived) ids of the selection, as fed in.
    ids: Vec<u16>,
    /// The active subgroup index into `subgroup_runs(&ids)`.
    active: u32,
}

/// What one key press should do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubgroupAction {
    /// Single-typed selection: SC2's Tab is inert — do nothing.
    Inert,
    /// The selection changed since the last press: the active subgroup is the
    /// first one (SC2 resets to the first subgroup on a fresh selection).
    Reset { active_id: u16, groups: u32 },
    /// Same selection, cycled: the newly active subgroup.
    Cycle { active_id: u16, index: u32, groups: u32 },
}

impl SubgroupModel {
    pub fn new(identity: u64, ids: Vec<u16>) -> Self {
        Self {
            identity,
            ids,
            active: 0,
        }
    }

    pub fn group_count(&self) -> usize {
        subgroup_runs(&self.ids).len()
    }

    pub fn active_index(&self) -> u32 {
        self.active.min(self.group_count().saturating_sub(1) as u32)
    }

    /// The id of the currently active subgroup (the first one when the model
    /// was just created).
    pub fn active_id(&self) -> u16 {
        subgroup_runs(&self.ids)
            .get(self.active_index() as usize)
            .map(|r| r.unit_id)
            .unwrap_or(0)
    }

    /// Feed a key press: `dir` = +1 (Tab) / -1 (Shift+Tab). Returns the action
    /// to take and updates the internal state. A selection whose identity
    /// differs from the model's is a fresh selection: SC2 resets to the first
    /// subgroup without advancing.
    pub fn press(&mut self, identity: u64, ids: Vec<u16>, dir: i32) -> SubgroupAction {
        let fresh = identity != self.identity;
        if fresh {
            self.identity = identity;
            self.ids = ids;
            self.active = 0;
        } else {
            // Same selection: refresh the ids (the array rotates underneath).
            self.ids = ids;
        }
        let runs = subgroup_runs(&self.ids);
        if runs.len() < 2 {
            return SubgroupAction::Inert;
        }
        let groups = runs.len() as u32;
        if fresh {
            return SubgroupAction::Reset {
                active_id: runs[0].unit_id,
                groups,
            };
        }
        let active = next_active(self.active, groups, dir);
        self.active = active;
        SubgroupAction::Cycle {
            active_id: runs[active as usize].unit_id,
            index: active,
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
        assert_eq!(m.group_count(), 1);
        assert!(matches!(m.press(identity_of(&ids), ids.clone(), 1), SubgroupAction::Inert));
    }

    #[test]
    fn barracks_factory_starport_are_three_subgroups() {
        // rax, rax, fact, fact, port — three subgroups in selection order.
        let ids = vec![106, 106, 120, 120, 132];
        let mut m = SubgroupModel::new(identity_of(&ids), ids.clone());
        assert_eq!(m.group_count(), 3);
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
        assert_eq!(m.group_count(), 2);
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
        assert_eq!(m.group_count(), 3);
    }
}
