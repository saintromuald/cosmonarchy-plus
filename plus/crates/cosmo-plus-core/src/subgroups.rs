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
/// Subgroups are keyed by each run's **first unit pointer**, never by unit
/// id: a mixed selection can hold the same unit type in two separate runs
/// (click order interleaves types), and an id-keyed model would always find
/// the first occurrence and bounce between the duplicates forever. The
/// pointer token is unique per run and stable across array rotations, so the
/// canonical order (first appearance, fixed when the selection is fresh)
/// cycles through every run exactly once, SC2-exact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubgroupModel {
    /// Order-insensitive identity of the selection this state belongs to.
    identity: u64,
    /// The canonical subgroup order: each run's first-unit pointer token, in
    /// first-appearance order. Never changes while the selection is the same.
    order: Vec<u32>,
    /// The active run's token (stable across array rotations).
    active_key: u32,
}

/// What one key press should do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubgroupAction {
    /// Single-typed selection: SC2's Tab is inert — do nothing.
    Inert,
    /// The selection changed since the last press: the active subgroup is the
    /// first one (SC2 resets to the first subgroup on a fresh selection).
    Reset {
        active_id: u16,
        active_key: u32,
        groups: u32,
    },
    /// Same selection, cycled: the newly active subgroup. `index` is the
    /// subgroup's position in the canonical order.
    Cycle {
        active_id: u16,
        active_key: u32,
        index: u32,
        groups: u32,
    },
}

/// The canonical subgroup order: one (key, id) pair per run, in first
/// appearance order. The key is the run's first unit token (pointer) — unique
/// even when the same unit id appears in several runs.
fn canonical_keys(ids: &[u16], ptrs: &[u32]) -> Vec<(u32, u16)> {
    subgroup_runs(ids)
        .into_iter()
        .map(|r| (ptrs[r.start.min(ptrs.len().saturating_sub(1))], r.unit_id))
        .collect()
}

impl SubgroupModel {
    pub fn new(identity: u64, ids: Vec<u16>, ptrs: Vec<u32>) -> Self {
        let order = canonical_keys(&ids, &ptrs);
        Self {
            identity,
            active_key: order.first().map(|&(k, _)| k).unwrap_or(0),
            order: order.into_iter().map(|(k, _)| k).collect(),
        }
    }

    /// The id of the currently active subgroup (the first one when the model
    /// was just created).
    pub fn active_id(&self) -> u16 {
        self.active_key as u16
    }

    /// Feed a key press: `dir` = +1 (Tab) / -1 (Shift+Tab). Returns the action
    /// to take and updates the internal state. A selection whose identity
    /// differs from the model's is a fresh selection: SC2 resets to the first
    /// subgroup without advancing.
    pub fn press(
        &mut self,
        identity: u64,
        ids: Vec<u16>,
        ptrs: Vec<u32>,
        dir: i32,
    ) -> SubgroupAction {
        let fresh = identity != self.identity;
        if fresh {
            self.identity = identity;
            let keys = canonical_keys(&ids, &ptrs);
            self.active_key = keys.first().map(|&(k, _)| k).unwrap_or(0);
            self.order = keys.into_iter().map(|(k, _)| k).collect();
        }
        let groups = self.order.len() as u32;
        if groups < 2 {
            return SubgroupAction::Inert;
        }
        let active_id = id_of(&ids, &ptrs, self.active_key).unwrap_or(0);
        if fresh {
            return SubgroupAction::Reset {
                active_id,
                active_key: self.active_key,
                groups,
            };
        }
        let idx = self
            .order
            .iter()
            .position(|&k| k == self.active_key)
            .unwrap_or(0);
        let next = next_active(idx as u32, groups, dir);
        self.active_key = self.order[next as usize];
        let active_id = id_of(&ids, &ptrs, self.active_key).unwrap_or(0);
        SubgroupAction::Cycle {
            active_id,
            active_key: self.active_key,
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
    pub fn ensure(
        &mut self,
        identity: u64,
        ids: Vec<u16>,
        ptrs: Vec<u32>,
    ) -> SubgroupAction {
        let fresh = identity != self.identity;
        if fresh {
            self.identity = identity;
            let keys = canonical_keys(&ids, &ptrs);
            self.active_key = keys.first().map(|&(k, _)| k).unwrap_or(0);
            self.order = keys.into_iter().map(|(k, _)| k).collect();
        }
        let groups = self.order.len() as u32;
        if groups < 2 {
            return SubgroupAction::Inert;
        }
        let active_id = id_of(&ids, &ptrs, self.active_key).unwrap_or(0);
        if fresh {
            return SubgroupAction::Reset {
                active_id,
                active_key: self.active_key,
                groups,
            };
        }
        let idx = self
            .order
            .iter()
            .position(|&k| k == self.active_key)
            .unwrap_or(0);
        SubgroupAction::Cycle {
            active_id,
            active_key: self.active_key,
            index: idx as u32,
            groups,
        }
    }
}

/// The unit id of the run whose first-unit token is `key`, in the current
/// array order.
fn id_of(ids: &[u16], ptrs: &[u32], key: u32) -> Option<u16> {
    let runs = subgroup_runs(ids);
    for r in runs {
        let k = ptrs.get(r.start.min(ptrs.len().saturating_sub(1))).copied();
        if k == Some(key) {
            return Some(r.unit_id);
        }
    }
    None
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

    /// Distinct fake pointer tokens, one per selection slot.
    fn ptrs_of(ids: &[u16]) -> Vec<u32> {
        (0..ids.len() as u32).map(|i| 1000 + i * 4).collect()
    }

    fn press(m: &mut SubgroupModel, ids: &[u16], dir: i32) -> SubgroupAction {
        m.press(identity_of(ids), ids.to_vec(), ptrs_of(ids), dir)
    }

    fn ensure(m: &mut SubgroupModel, ids: &[u16]) -> SubgroupAction {
        m.ensure(identity_of(ids), ids.to_vec(), ptrs_of(ids))
    }

    #[test]
    fn twelve_mavericks_are_one_subgroup_and_tab_is_inert() {
        let ids = vec![12u16; 12];
        let mut m = SubgroupModel::new(identity_of(&ids), ids.clone(), ptrs_of(&ids));
        assert!(matches!(press(&mut m, &ids, 1), SubgroupAction::Inert));
    }

    #[test]
    fn tab_cycles_forward_and_wraps() {
        let ids = vec![106, 106, 120, 132];
        let mut m = SubgroupModel::new(identity_of(&ids), ids.clone(), ptrs_of(&ids));
        assert_eq!(press(&mut m, &ids, 1), SubgroupAction::Cycle {
            active_id: 120, active_key: 1008, index: 1, groups: 3
        });
        assert_eq!(press(&mut m, &ids, 1), SubgroupAction::Cycle {
            active_id: 132, active_key: 1012, index: 2, groups: 3
        });
        assert_eq!(press(&mut m, &ids, 1), SubgroupAction::Cycle {
            active_id: 106, active_key: 1000, index: 0, groups: 3
        });
    }

    #[test]
    fn shift_tab_cycles_backward() {
        let ids = vec![106, 120, 132];
        let mut m = SubgroupModel::new(identity_of(&ids), ids.clone(), ptrs_of(&ids));
        assert_eq!(press(&mut m, &ids, -1), SubgroupAction::Cycle {
            active_id: 132, active_key: 1008, index: 2, groups: 3
        });
    }

    #[test]
    fn fresh_selection_resets_to_the_first_subgroup() {
        let first = vec![106, 120];
        let second = vec![120, 132]; // a different selection
        let mut m = SubgroupModel::new(identity_of(&first), first.clone(), ptrs_of(&first));
        let _ = press(&mut m, &first, 1);
        // The player selects something else: the model resets to the first
        // subgroup WITHOUT advancing.
        assert_eq!(press(&mut m, &second, 1), SubgroupAction::Reset {
            active_id: 120, active_key: 1000, groups: 2
        });
        // The next press on the same selection cycles normally.
        assert_eq!(press(&mut m, &second, 1), SubgroupAction::Cycle {
            active_id: 132, active_key: 1004, index: 1, groups: 2
        });
    }

    /// THE regression: the same unit type selected in two separate runs (click
    /// order interleaves types). An id-keyed model bounces between the two
    /// copies forever; the pointer-token model cycles through every run once.
    #[test]
    fn duplicate_runs_cycle_through_every_run_once() {
        // rax, fact, rax — id 106 twice, in two runs.
        let ids = vec![106, 106, 120, 106];
        let mut m = SubgroupModel::new(identity_of(&ids), ids.clone(), ptrs_of(&ids));
        // run 1 = 106@0 (key 1000), run 2 = 120@2 (key 1008), run 3 = 106@3 (key 1012).
        assert_eq!(press(&mut m, &ids, 1), SubgroupAction::Cycle {
            active_id: 120, active_key: 1008, index: 1, groups: 3
        });
        assert_eq!(press(&mut m, &ids, 1), SubgroupAction::Cycle {
            active_id: 106, active_key: 1012, index: 2, groups: 3
        });
        assert_eq!(press(&mut m, &ids, 1), SubgroupAction::Cycle {
            active_id: 106, active_key: 1000, index: 0, groups: 3
        });
        // And the per-frame ensure never bounces between the duplicates.
        assert_eq!(ensure(&mut m, &ids), SubgroupAction::Cycle {
            active_id: 106, active_key: 1000, index: 0, groups: 3
        });
        assert_eq!(ensure(&mut m, &ids), SubgroupAction::Cycle {
            active_id: 106, active_key: 1000, index: 0, groups: 3
        });
    }

    /// The adapter rotates the selection array under the model on every cycle;
    /// the model must keep cycling through ALL subgroups regardless of the
    /// rotated order fed back to it.
    #[test]
    fn cycling_is_invariant_under_array_rotation() {
        // rax, fact, port, nexus — four subgroups, tokens 1000/1008/1012/1016.
        let ids = vec![106, 106, 120, 132, 154];
        let mut m = SubgroupModel::new(identity_of(&ids), ids.clone(), ptrs_of(&ids));
        assert_eq!(press(&mut m, &ids, 1), SubgroupAction::Cycle {
            active_id: 120, active_key: 1008, index: 1, groups: 4
        });

        // The adapter rotated fact to the front: the same units, new order,
        // and the tokens travel with their units.
        let rotated = vec![120, 106, 106, 132, 154];
        let rptrs = vec![1008, 1000, 1004, 1012, 1016];
        let id = identity_of(&ids);
        let e = m.ensure(id, rotated.clone(), rptrs.clone());
        assert_eq!(e, SubgroupAction::Cycle {
            active_id: 120, active_key: 1008, index: 1, groups: 4
        });

        // Next Tab on the rotated array: port (132), NOT back to rax.
        let b = m.press(id, rotated.clone(), rptrs.clone(), 1);
        assert_eq!(b, SubgroupAction::Cycle {
            active_id: 132, active_key: 1012, index: 2, groups: 4
        });

        // And again after the adapter rotates port to the front.
        let rotated2 = vec![132, 120, 106, 106, 154];
        let rptrs2 = vec![1012, 1008, 1000, 1004, 1016];
        let c = m.press(id, rotated2.clone(), rptrs2.clone(), 1);
        assert_eq!(c, SubgroupAction::Cycle {
            active_id: 154, active_key: 1016, index: 3, groups: 4
        });

        // Wraps around to rax (106) after the full cycle.
        let rotated3 = vec![154, 132, 120, 106, 106];
        let rptrs3 = vec![1016, 1012, 1008, 1000, 1004];
        let d = m.press(id, rotated3.clone(), rptrs3.clone(), 1);
        assert_eq!(d, SubgroupAction::Cycle {
            active_id: 106, active_key: 1000, index: 0, groups: 4
        });
    }

    #[test]
    fn ensure_resets_on_a_fresh_selection_without_advancing() {
        let first = vec![106, 106, 120];
        let second = vec![120, 132];
        let mut m = SubgroupModel::new(identity_of(&first), first.clone(), ptrs_of(&first));
        let _ = press(&mut m, &first, 1);
        assert_eq!(ensure(&mut m, &second), SubgroupAction::Reset {
            active_id: 120, active_key: 1000, groups: 2
        });
        assert_eq!(ensure(&mut m, &second), SubgroupAction::Cycle {
            active_id: 120, active_key: 1000, index: 0, groups: 2
        });
        assert_eq!(press(&mut m, &second, 1), SubgroupAction::Cycle {
            active_id: 132, active_key: 1004, index: 1, groups: 2
        });
    }

    #[test]
    fn ensure_is_inert_for_a_single_typed_selection() {
        let ids = vec![12u16; 5];
        let mut m = SubgroupModel::new(identity_of(&ids), ids.clone(), ptrs_of(&ids));
        assert!(matches!(ensure(&mut m, &ids), SubgroupAction::Inert));
    }

    #[test]
    fn burrowed_and_unburrowed_are_separate_subgroups_via_derived_ids() {
        let ids = vec![7, 7, 0x8007];
        let mut m = SubgroupModel::new(identity_of(&ids), ids.clone(), ptrs_of(&ids));
        assert_eq!(press(&mut m, &ids, 1), SubgroupAction::Cycle {
            active_id: 0x8007, active_key: 1008, index: 1, groups: 2
        });
    }
}
