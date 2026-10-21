//! The `plus.yml` schema and its parser (pure — no file I/O or logging here).
//!
//! The file is a small YAML subset parsed by [`crate::yaml`]. This module turns the
//! parsed tree into a typed [`PlusConfig`]. The plugin's `config` module does the file
//! lookup and logging on top of this.

use std::collections::BTreeMap;

use crate::yaml::{self, Yaml};

/// One absolute unit override (`overrides:`). An omitted field means "unchanged".
#[derive(Debug, Clone, Default)]
pub struct Override {
    pub unit: String,
    pub minerals: Option<u16>,
    pub gas: Option<u16>,
    pub build_time: Option<u16>,
}

/// Per-tier gas extractor deltas. Deltas, applied as `current + delta`, clamped at 0.
#[derive(Debug, Clone, Copy, Default)]
pub struct GasTierDelta {
    pub minerals: i32,
    pub build_time_seconds: i32,
}

/// Saturation display tuning (consumed by the overlay task; parsed and stored
/// now so `plus.yml` is the single source of truth).
#[derive(Debug, Clone)]
pub struct SaturationConfig {
    /// Master switch for the saturation overlay. Defaults to **on** when `plus.yml` is
    /// present (the user wants it); set `enabled: false` to turn it off, or use the
    /// `CM_PLUS_SATURATION` environment variable to override.
    pub enabled: bool,
    /// Fallback optimal denominator per worker tier if the measured curve is unavailable.
    pub tier_defaults: BTreeMap<u8, u32>,
    /// Raw extra keys, preserved for the overlay task.
    pub extra: BTreeMap<String, Yaml>,
}

impl Default for SaturationConfig {
    fn default() -> Self {
        SaturationConfig {
            enabled: true,
            tier_defaults: BTreeMap::new(),
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PlusConfig {
    pub overrides: Vec<Override>,
    pub gas_tier_deltas: BTreeMap<u32, GasTierDelta>,
    pub saturation: SaturationConfig,
}

/// Parse `plus.yml` text into a [`PlusConfig`].
pub fn parse_plus_yml(text: &str) -> Result<PlusConfig, String> {
    let root = yaml::parse(text)?;
    let mut cfg = PlusConfig::default();

    if let Some(ov) = root.get("overrides") {
        let Yaml::List(items) = ov else {
            return Err("`overrides:` must be a list".to_string());
        };
        for item in items {
            let Yaml::Map(entries) = item else {
                return Err("`overrides:` items must be maps".to_string());
            };
            let get = |k: &str| entries.iter().find(|(kk, _)| kk == k).map(|(_, v)| v);
            let Some(unit) = get("unit").and_then(Yaml::as_str) else {
                return Err("`overrides:` item missing `unit:`".to_string());
            };
            let mut o = Override {
                unit: unit.to_string(),
                ..Default::default()
            };
            if let Some(v) = get("minerals").and_then(Yaml::as_i64) {
                o.minerals = Some(clamp_u16(v)?);
            }
            if let Some(v) = get("gas").and_then(Yaml::as_i64) {
                o.gas = Some(clamp_u16(v)?);
            }
            if let Some(v) = get("build_time").and_then(Yaml::as_i64) {
                o.build_time = Some(clamp_u16(v)?);
            }
            cfg.overrides.push(o);
        }
    }

    if let Some(gtd) = root.get("gas_tier_deltas") {
        let Yaml::Map(entries) = gtd else {
            return Err("`gas_tier_deltas:` must be a map".to_string());
        };
        for (tier_str, v) in entries {
            let tier: u32 = tier_str.trim().parse().map_err(|_| {
                format!("gas_tier_deltas key `{tier_str}` is not an integer tier")
            })?;
            let mut d = GasTierDelta::default();
            if let Some(m) = v.get("minerals").and_then(Yaml::as_i64) {
                d.minerals = m as i32;
            }
            if let Some(t) = v.get("build_time_seconds").and_then(Yaml::as_i64) {
                d.build_time_seconds = t as i32;
            }
            cfg.gas_tier_deltas.insert(tier, d);
        }
    }

    if let Some(sat) = root.get("saturation") {
        let Yaml::Map(entries) = sat else {
            return Err("`saturation:` must be a map".to_string());
        };
        for (k, v) in entries {
            match (k.as_str(), v) {
                ("enabled", Yaml::Scalar(s)) => {
                    cfg.saturation.enabled = parse_bool(s);
                }
                ("tier1_default", Yaml::Scalar(s)) => {
                    if let Ok(n) = s.trim().parse::<u32>() {
                        cfg.saturation.tier_defaults.insert(1, n);
                    }
                }
                ("tier2_default", Yaml::Scalar(s)) => {
                    if let Ok(n) = s.trim().parse::<u32>() {
                        cfg.saturation.tier_defaults.insert(2, n);
                    }
                }
                (k, v) => {
                    cfg.saturation.extra.insert(k.to_string(), (*v).clone());
                }
            }
        }
    }

    Ok(cfg)
}

fn clamp_u16(v: i64) -> Result<u16, String> {
    if v < 0 || v > u16::MAX as i64 {
        Err(format!("value {v} is outside u16 range"))
    } else {
        Ok(v as u16)
    }
}

/// Parse a YAML scalar as a boolean, accepting the usual spellings.
fn parse_bool(s: &str) -> bool {
    matches!(
        s.trim().to_ascii_lowercase().as_str(),
        "true" | "yes" | "on" | "1"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_example_shape() {
        let yml = "overrides:\n  - unit: Terran Mason\n    minerals: 50\n    gas: 0\n\
gas_tier_deltas:\n  \"1\":\n    minerals: -100\n    build_time_seconds: -10\n\
saturation:\n  tier1_default: 8\n  tier2_default: 4\n";
        let cfg = parse_plus_yml(yml).unwrap();
        assert_eq!(cfg.overrides.len(), 1);
        assert_eq!(cfg.overrides[0].unit, "Terran Mason");
        assert_eq!(cfg.overrides[0].minerals, Some(50));
        assert_eq!(cfg.overrides[0].gas, Some(0));
        assert_eq!(cfg.overrides[0].build_time, None);
        assert_eq!(cfg.gas_tier_deltas[&1].minerals, -100);
        assert_eq!(cfg.gas_tier_deltas[&1].build_time_seconds, -10);
        assert_eq!(cfg.saturation.tier_defaults[&1], 8);
        assert_eq!(cfg.saturation.tier_defaults[&2], 4);
    }

    #[test]
    fn override_clamps_and_validates() {
        assert!(parse_plus_yml("overrides:\n  - unit: X\n    minerals: 70000\n").is_err());
        assert!(parse_plus_yml("overrides:\n  - unit: X\n    minerals: -5\n").is_err());
    }

    #[test]
    fn missing_sections_are_default() {
        let cfg = parse_plus_yml("saturation:\n  tier1_default: 8\n").unwrap();
        assert!(cfg.overrides.is_empty());
        assert!(cfg.gas_tier_deltas.is_empty());
    }

    #[test]
    fn saturation_enabled_defaults_on_and_parses() {
        // Absent `enabled:` defaults to on.
        let cfg = parse_plus_yml("saturation:\n  tier1_default: 8\n").unwrap();
        assert!(cfg.saturation.enabled);
        // Explicit false turns it off.
        let cfg = parse_plus_yml("saturation:\n  enabled: false\n").unwrap();
        assert!(!cfg.saturation.enabled);
        // Explicit true / spellings.
        for t in ["true", "yes", "on", "1"] {
            let cfg = parse_plus_yml(&format!("saturation:\n  enabled: {t}\n")).unwrap();
            assert!(cfg.saturation.enabled, "enabled: {t} should be on");
        }
    }
}
