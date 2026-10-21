//! Anti-meta mode: one symmetric start roll per game.
//!
//! Pure, host-testable. The roll is derived from a seed that both peers share (the
//! game's synced RNG seed), so every client computes the same tuple without any
//! network round trip. Ranges are inclusive and quantised as the mode specifies:
//!
//! * workers  : 4..=8
//! * minerals : 100..=300 in steps of 5
//! * gas      :   0..=125 in steps of 5

/// One game's start roll. Applied identically to every active player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Roll {
    pub workers: u8,
    pub minerals: u32,
    pub gas: u32,
}

/// SplitMix64: tiny, fast, deterministic, no dependencies.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn next_below(&mut self, n: u32) -> u32 {
        // Modulo bias is irrelevant for these small ranges and keeps the function
        // trivially reproducible across peers/languages.
        (self.next_u64() % u64::from(n)) as u32
    }
}

/// Derive the roll from a shared seed.
pub fn roll(seed: u32) -> Roll {
    let mut rng = SplitMix64(u64::from(seed) ^ 0x504C_5553_414E_5449); // "PLUSANTI"
    Roll {
        workers: 4 + rng.next_below(5) as u8,
        minerals: 100 + 5 * rng.next_below(41),
        gas: 5 * rng.next_below(26),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_hold_for_many_seeds() {
        for seed in 0..100_000u32 {
            let r = roll(seed);
            assert!((4..=8).contains(&r.workers), "{r:?}");
            assert!((100..=300).contains(&r.minerals), "{r:?}");
            assert_eq!(r.minerals % 5, 0, "{r:?}");
            assert!(r.gas <= 125, "{r:?}");
            assert_eq!(r.gas % 5, 0, "{r:?}");
        }
    }

    #[test]
    fn deterministic() {
        assert_eq!(roll(12345), roll(12345));
        assert_ne!(roll(1), roll(2));
    }

    #[test]
    fn endpoints_reachable() {
        let mut saw_low = false;
        let mut saw_high = false;
        let mut saw_zero_gas = false;
        let mut saw_max_gas = false;
        for seed in 0..10_000u32 {
            let r = roll(seed);
            saw_low |= r.workers == 4 && r.minerals == 100 && r.gas == 0;
            saw_high |= r.workers == 8 && r.minerals == 300 && r.gas == 125;
            saw_zero_gas |= r.gas == 0;
            saw_max_gas |= r.gas == 125;
        }
        assert!(saw_zero_gas && saw_max_gas, "gas endpoints not hit");
        // Low/high corners are rare; only require that both extremes of each axis occur.
        let mut w = [false; 9];
        let mut m = [false; 61];
        for seed in 0..10_000u32 {
            let r = roll(seed);
            w[r.workers as usize] = true;
            m[((r.minerals - 100) / 5) as usize] = true;
        }
        assert!(w[4] && w[8], "worker endpoints not hit");
        assert!(m[0] && m[40], "mineral endpoints not hit");
        let _ = (saw_low, saw_high);
    }
}
