//! SC2-style selection subgroup model (pure logic, unit-testable).
//!
//! StarCraft II divides a mixed selection into **subgroups** — one per distinct unit
//! type, in the order the types appear in the selection — and the Tab key ("Next
//! Subgroup") cycles the **active subgroup** forward, Shift+Tab backward. Only the
//! active subgroup's buttons are shown (its abilities win the hotkeys), while
//! non-ability orders still apply to the whole selection.
//!
//! Brood War has no subgroup concept, so Cosmonarchy Plus emulates it by *rotating*
//! the selection: the active subgroup's units move to the front of the selection
//! list. BW's command card lists the first-selected unit type's buttons first and
//! gives them hotkey priority, so the rotation puts the active subgroup's abilities
//! (spells, production, morphs) under the player's fingers — everything else in the
//! selection stays selected, exactly like SC2.
//!
//! This module owns only the arithmetic: given the ordered unit ids of a selection,
//! the active group index and a direction, it returns the rotated index permutation.
//! The plugin feeds it live ids and applies the permutation to the engine selection.

/// One subgroup rotation step.
///
/// `ids` is the selection in its current (base) order. Consecutive equal ids form a
/// subgroup; subgroups are ordered by first appearance. The result rotates the base
/// order so subgroup `next_active` comes first, preserving the cyclic order of every
/// other subgroup (and of units inside each subgroup).
///
/// Returns `None` when there is nothing to cycle (fewer than two subgroups).
pub fn subgroup_rotation(ids: &[u16], active: u32, dir: i32) -> Option<SubgroupRotation> {
    if ids.len() < 2 {
        return None;
    }
    // Run boundaries: index where each subgroup starts.
    let mut starts: Vec<usize> = vec![0];
    for i in 1..ids.len() {
        if ids[i] != ids[i - 1] {
            starts.push(i);
        }
    }
    let groups = starts.len() as u32;
    if groups < 2 {
        return None;
    }
    let next = (active as i32 + dir).rem_euclid(groups as i32) as u32;
    let start = starts[next as usize];
    // Rotate: [start..len) ++ [0..start).
    let mut order = Vec::with_capacity(ids.len());
    for i in 0..ids.len() {
        order.push(((start + i) % ids.len()) as u32);
    }
    Some(SubgroupRotation {
        order,
        active: next,
        groups,
        active_unit_id: ids[start],
    })
}

/// The result of [`subgroup_rotation`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubgroupRotation {
    /// `order[i]` is the index into the base selection of the i-th unit in the
    /// rotated selection.
    pub order: Vec<u32>,
    /// The new active subgroup (0-based).
    pub active: u32,
    /// The number of subgroups.
    pub groups: u32,
    /// The unit id of the active subgroup (for the on-screen indicator).
    pub active_unit_id: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[u16]) -> Vec<u16> {
        v.to_vec()
    }

    #[test]
    fn cycles_forward_through_groups() {
        let base = ids(&[1, 1, 1, 2, 2, 3, 3]);
        let r1 = subgroup_rotation(&base, 0, 1).unwrap();
        assert_eq!(r1.active, 1);
        assert_eq!(r1.groups, 3);
        assert_eq!(r1.active_unit_id, 2);
        // Rotated order: group 2 first, then 3, then 1.
        assert_eq!(r1.order, vec![3, 4, 5, 6, 0, 1, 2]);

        let r2 = subgroup_rotation(&base, 1, 1).unwrap();
        assert_eq!(r2.active, 2);
        assert_eq!(r2.active_unit_id, 3);
        assert_eq!(r2.order, vec![5, 6, 0, 1, 2, 3, 4]);

        // Wraps back to the first group.
        let r3 = subgroup_rotation(&base, 2, 1).unwrap();
        assert_eq!(r3.active, 0);
        assert_eq!(r3.order, vec![0, 1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn cycles_backward_with_negative_direction() {
        let base = ids(&[1, 1, 1, 2, 2, 3, 3]);
        let r = subgroup_rotation(&base, 0, -1).unwrap();
        assert_eq!(r.active, 2);
        assert_eq!(r.active_unit_id, 3);
        assert_eq!(r.order, vec![5, 6, 0, 1, 2, 3, 4]);
    }

    #[test]
    fn interleaved_types_are_separate_groups() {
        // [A B A] — three runs, three subgroups (matches BW's run semantics).
        let base = ids(&[7, 8, 7]);
        let r = subgroup_rotation(&base, 0, 1).unwrap();
        assert_eq!(r.groups, 3);
        assert_eq!(r.order, vec![1, 2, 0]);
    }

    #[test]
    fn single_group_returns_none() {
        assert!(subgroup_rotation(&ids(&[1, 1, 1, 1]), 0, 1).is_none());
        assert!(subgroup_rotation(&ids(&[9]), 0, 1).is_none());
        assert!(subgroup_rotation(&ids(&[]), 0, 1).is_none());
    }

    #[test]
    fn buildings_and_units_share_the_same_model() {
        // Unit ids are opaque; building ids cycle exactly like unit ids.
        let base = ids(&[100, 100, 200]);
        let r = subgroup_rotation(&base, 0, 1).unwrap();
        assert_eq!(r.active_unit_id, 200);
        assert_eq!(r.order, vec![2, 0, 1]);
    }
}
