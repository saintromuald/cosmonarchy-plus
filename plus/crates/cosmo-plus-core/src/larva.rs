//! Larva count entry — pure state machine for the "pick N larva" feature.
//!
//! Flow: press the larva hotkey, type 1–2 digits (1–99), confirm (Enter or the
//! entry timeout) → exactly that many larva are selected (clamped to the
//! engine's 12-unit selection cap and to the larva actually available). An
//! empty buffer at timeout means "select all" (the mod's Select Larva button
//! behaviour).

/// How long (ms) the entry window stays open without input before it fires.
pub const ENTRY_TIMEOUT_MS: u64 = 2000;
/// The engine cannot select more than this many units at once (`SelectionCap`).
pub const SELECTION_CAP: u32 = 12;
/// The entered count is clamped to this (two digits; the engine caps lower).
pub const MAX_ENTERED: u32 = 99;

/// The digit-entry state, shared between the keyboard hook and the draw pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Entry {
    /// The buffered digits as their numeric value (0 = nothing entered yet).
    pub value: u32,
    /// How many digits have been entered.
    pub digits: u8,
}

impl Entry {
    /// Append one digit (0–9). Ignores a digit that would exceed 99, and
    /// leading zeros collapse (01 = 1).
    pub fn push_digit(&mut self, d: u32) {
        if d > 9 {
            return;
        }
        if self.digits == 0 {
            self.value = d;
            self.digits = 1;
            return;
        }
        if self.digits >= 2 {
            return; // 99 is the maximum; later digits are ignored
        }
        let v = self.value.saturating_mul(10).saturating_add(d);
        if v <= MAX_ENTERED {
            self.value = v;
            self.digits += 1;
        }
    }

    /// The confirmed count: the entered value (at least 1), clamped to the
    /// selection cap. An empty buffer means "select all" (`None`).
    pub fn confirmed(&self) -> Option<u32> {
        if self.digits == 0 {
            None
        } else {
            Some(self.value.clamp(1, SELECTION_CAP))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_one_and_two_digit_values() {
        let mut e = Entry::default();
        e.push_digit(6);
        assert_eq!(e.confirmed(), Some(6));
        e.push_digit(0);
        // The raw buffer holds 60; the confirmed count clamps to the engine's cap.
        assert_eq!(e.value, 60);
        assert_eq!(e.confirmed(), Some(SELECTION_CAP));
        let mut e = Entry::default();
        e.push_digit(1);
        e.push_digit(2);
        assert_eq!(e.confirmed(), Some(12));
    }

    #[test]
    fn leading_zero_collapses() {
        let mut e = Entry::default();
        e.push_digit(0);
        e.push_digit(6);
        assert_eq!(e.confirmed(), Some(6));
    }

    #[test]
    fn clamps_to_the_selection_cap() {
        let mut e = Entry::default();
        e.push_digit(9);
        e.push_digit(9);
        assert_eq!(e.confirmed(), Some(SELECTION_CAP));
    }

    #[test]
    fn ignores_third_digit_and_overflow() {
        let mut e = Entry::default();
        e.push_digit(9);
        e.push_digit(9);
        e.push_digit(1); // ignored
        assert_eq!(e.value, 99);
        assert_eq!(e.confirmed(), Some(SELECTION_CAP));
    }

    #[test]
    fn empty_buffer_means_select_all() {
        let e = Entry::default();
        assert_eq!(e.confirmed(), None);
    }
}
