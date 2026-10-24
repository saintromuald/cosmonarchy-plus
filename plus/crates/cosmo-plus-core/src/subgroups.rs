//! SC2-style selection subgroup model (pure logic, unit-testable).
//!
//! StarCraft II divides a mixed selection into **subgroups** — one per distinct unit
//! type, in the order the types appear in the selection — and the Tab key ("Next
//! Subgroup") cycles the **active subgroup** forward, Shift+Tab backward. Only the
//! active subgroup's buttons are shown (its abilities win the hotkeys), while
//! non-ability orders still apply to the whole selection.
//!
//! Brood War has no subgroup concept, so Cosmonarchy Plus emulates it **without
//! touching the selection**: the plugin tracks the subgroup run boundaries and the
//! active index, and points the engine's UI at the active unit. The selection stays
//! exactly as the player made it.
//!
//! This module owns only the arithmetic: given the ordered unit ids of a selection,
//! it returns the subgroup runs (start offset, length and unit id each).

/// One subgroup run: the contiguous block of equal unit ids starting at `start`
/// (inclusive), `len` units long, all of `unit_id`. Runs are ordered by first
/// appearance, matching SC2's subgroup order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubgroupRun {
    /// Index into the selection where this subgroup starts.
    pub start: usize,
    /// Number of units in this subgroup.
    pub len: usize,
    /// The unit id of this subgroup.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[u16]) -> Vec<u16> {
        v.to_vec()
    }

    #[test]
    fn splits_into_ordered_runs() {
        let runs = subgroup_runs(&ids(&[1, 1, 1, 2, 2, 3, 3]));
        assert_eq!(
            runs,
            vec![
                SubgroupRun { start: 0, len: 3, unit_id: 1 },
                SubgroupRun { start: 3, len: 2, unit_id: 2 },
                SubgroupRun { start: 5, len: 2, unit_id: 3 },
            ]
        );
    }

    #[test]
    fn interleaved_types_are_separate_groups() {
        // [A B A] — three runs, three subgroups (matches BW's run semantics).
        let runs = subgroup_runs(&ids(&[7, 8, 7]));
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].unit_id, 7);
        assert_eq!(runs[1].unit_id, 8);
        assert_eq!(runs[2].unit_id, 7);
    }

    #[test]
    fn active_cycles_forward_and_wraps() {
        assert_eq!(next_active(0, 3, 1), 1);
        assert_eq!(next_active(1, 3, 1), 2);
        assert_eq!(next_active(2, 3, 1), 0);
    }

    #[test]
    fn active_cycles_backward() {
        assert_eq!(next_active(0, 3, -1), 2);
        assert_eq!(next_active(1, 3, -1), 0);
    }

    #[test]
    fn single_group_stays_zero() {
        assert_eq!(next_active(0, 1, 1), 0);
        assert_eq!(next_active(0, 0, -1), 0);
    }

    #[test]
    fn empty_selection_has_no_runs() {
        assert!(subgroup_runs(&[]).is_empty());
        assert!(subgroup_runs(&[9]).len() == 1);
    }

    #[test]
    fn buildings_and_units_share_the_same_model() {
        // Unit ids are opaque; building ids split exactly like unit ids.
        let runs = subgroup_runs(&ids(&[100, 100, 200]));
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[1].unit_id, 200);
        assert_eq!(runs[1].start, 2);
    }
}
