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
/// The adapter feeds the **cycle-worthy run keys** — each key is a run's
/// first unit pointer token (unique per run, stable across array rotations),
/// in the order the adapter wants them cycled (first appearance among the
/// ability-bearing units, which is what Tab should visit). The model compares
/// the selection identity (an order-insensitive hash the adapter computes
/// over the whole selection, since the underlying array rotates as the model
/// cycles) and returns what to do.
///
/// Keying by pointer token — never by unit id — matters: a mixed selection
/// can hold the same unit type in two separate runs, and an id-keyed model
/// would always find the first occurrence and bounce between the duplicates
/// forever. The token is unique per run, so the canonical order cycles
/// through every run exactly once, SC2-exact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubgroupModel {
    /// Order-insensitive identity of the selection this state belongs to.
    identity: u64,
    /// The canonical subgroup order: the cycle-worthy runs' keys, in
    /// first-appearance order. Never changes while the selection is the same.
    order: Vec<u32>,
    /// The active run's key (stable across array rotations).
    active_key: u32,
}

/// What one key press should do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubgroupAction {
    /// Nothing worth cycling (fewer than two worthy runs): Tab is inert.
    Inert,
    /// The selection changed since the last press: the active subgroup is the
    /// first one (SC2 resets to the first subgroup on a fresh selection).
    Reset { active_key: u32, groups: u32 },
    /// Same selection, cycled: the newly active subgroup. `index` is the
    /// subgroup's position in the canonical order.
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

    /// Feed a key press: `dir` = +1 (Tab) / -1 (Shift+Tab). Returns the action
    /// to take and updates the internal state. A selection whose identity
    /// differs from the model's is a fresh selection: SC2 resets to the first
    /// subgroup without advancing.
    pub fn press(&mut self, identity: u64, keys: Vec<u32>, dir: i32) -> SubgroupAction {
        let fresh = identity != self.identity;
        if fresh {
            self.identity = identity;
            self.order = keys;
            self.active_key = self.order.first().copied().unwrap_or(0);
        }
        let groups = self.order.len() as u32;
        if groups < 2 {
            return SubgroupAction::Inert;
        }
        if fresh {
            return SubgroupAction::Reset {
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
        SubgroupAction::Cycle {
            active_key: self.active_key,
            index: next,
            groups,
        }
    }

    /// Feed the per-frame view of the selection (no key press). A changed
    /// identity resets to the first subgroup (SC2's fresh-selection rule, so a
    /// mixed selection shows the first worthy subgroup's card even before any
    /// Tab); otherwise the stored active subgroup stands.
    pub fn ensure(&mut self, identity: u64, keys: Vec<u32>) -> SubgroupAction {
        let fresh = identity != self.identity;
        if fresh {
            self.identity = identity;
            self.order = keys;
            self.active_key = self.order.first().copied().unwrap_or(0);
        }
        let groups = self.order.len() as u32;
        if groups < 2 {
            return SubgroupAction::Inert;
        }
        if fresh {
            return SubgroupAction::Reset {
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
            active_key: self.active_key,
            index: idx as u32,
            groups,
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

    #[test]
    fn fewer_than_two_worthy_runs_is_inert() {
        let keys = vec![1000u32];
        let mut m = SubgroupModel::new(identity_of(&keys), keys.clone());
        assert!(matches!(m.press(identity_of(&keys), keys.clone(), 1), SubgroupAction::Inert));
    }

    #[test]
    fn tab_cycles_forward_and_wraps() {
        let keys = vec![1000u32, 1008, 1012];
        let id = identity_of(&keys);
        let mut m = SubgroupModel::new(id, keys.clone());
        assert_eq!(m.press(id, keys.clone(), 1), SubgroupAction::Cycle {
            active_key: 1008, index: 1, groups: 3
        });
        assert_eq!(m.press(id, keys.clone(), 1), SubgroupAction::Cycle {
            active_key: 1012, index: 2, groups: 3
        });
        assert_eq!(m.press(id, keys.clone(), 1), SubgroupAction::Cycle {
            active_key: 1000, index: 0, groups: 3
        });
    }

    #[test]
    fn shift_tab_cycles_backward() {
        let keys = vec![1000u32, 1008, 1012];
        let id = identity_of(&keys);
        let mut m = SubgroupModel::new(id, keys.clone());
        assert_eq!(m.press(id, keys.clone(), -1), SubgroupAction::Cycle {
            active_key: 1012, index: 2, groups: 3
        });
    }

    #[test]
    fn fresh_selection_resets_to_the_first_subgroup() {
        let first = vec![1000u32, 1008];
        let second = vec![2000u32, 2004];
        let mut m = SubgroupModel::new(identity_of(&first), first.clone());
        let _ = m.press(identity_of(&first), first.clone(), 1);
        assert_eq!(m.press(identity_of(&second), second.clone(), 1), SubgroupAction::Reset {
            active_key: 2000, groups: 2
        });
        assert_eq!(m.press(identity_of(&second), second.clone(), 1), SubgroupAction::Cycle {
            active_key: 2004, index: 1, groups: 2
        });
    }

    #[test]
    fn ensure_resets_on_a_fresh_selection_without_advancing() {
        let first = vec![1000u32, 1008, 1012];
        let second = vec![2000u32, 2004];
        let mut m = SubgroupModel::new(identity_of(&first), first.clone());
        let _ = m.press(identity_of(&first), first.clone(), 1);
        assert_eq!(m.ensure(identity_of(&second), second.clone()), SubgroupAction::Reset {
            active_key: 2000, groups: 2
        });
        assert_eq!(m.ensure(identity_of(&second), second.clone()), SubgroupAction::Cycle {
            active_key: 2000, index: 0, groups: 2
        });
        assert_eq!(m.press(identity_of(&second), second.clone(), 1), SubgroupAction::Cycle {
            active_key: 2004, index: 1, groups: 2
        });
    }

    /// The rotation-invariance regression: the adapter rotates the array
    /// under the model, but the keys are stable, so cycling never bounces.
    #[test]
    fn cycling_is_invariant_under_array_rotation() {
        let keys = vec![1000u32, 1008, 1012, 1016];
        let id = identity_of(&keys);
        let mut m = SubgroupModel::new(id, keys.clone());
        assert_eq!(m.press(id, keys.clone(), 1), SubgroupAction::Cycle {
            active_key: 1008, index: 1, groups: 4
        });
        // The array rotated (fact first); the adapter feeds the same keys.
        assert_eq!(m.ensure(id, keys.clone()), SubgroupAction::Cycle {
            active_key: 1008, index: 1, groups: 4
        });
        assert_eq!(m.press(id, keys.clone(), 1), SubgroupAction::Cycle {
            active_key: 1012, index: 2, groups: 4
        });
        assert_eq!(m.press(id, keys.clone(), 1), SubgroupAction::Cycle {
            active_key: 1016, index: 3, groups: 4
        });
        assert_eq!(m.press(id, keys.clone(), 1), SubgroupAction::Cycle {
            active_key: 1000, index: 0, groups: 4
        });
    }
}
