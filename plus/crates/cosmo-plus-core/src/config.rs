//! The `plus.yml` schema and its parser (pure — no file I/O or logging here).
//!
//! The file is a small YAML subset parsed by [`crate::yaml`]. This module turns the
//! parsed tree into a typed [`PlusConfig`]. The plugin's `config` module does the file
//! lookup and logging on top of this.

use std::collections::BTreeMap;

use crate::draft::{Anchor, DraftOption, Effect};
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

/// Advanced-worker (tier-2) tuning, applied at runtime to the live dat.
///
/// Cost/build-time deltas only. The old per-trip mining multiplier was removed:
/// it raised the per-patch income ceiling instead of making the worker faster,
/// which broke the invariant "a saturated tier-2 base mines the same as a
/// saturated tier-1 base". See [`UltraloadConfig`].
#[derive(Debug, Clone, Copy)]
pub struct AdvancedWorkersConfig {
    /// Added to the unit's current mineral cost (frames; clamped at 0).
    pub minerals_delta: i32,
    /// Added to the unit's current gas cost (clamped at 0).
    pub gas_delta: i32,
    /// Added to the build time in seconds (negative shortens; clamped at 0).
    pub build_time_delta_seconds: i32,
}

impl Default for AdvancedWorkersConfig {
    fn default() -> Self {
        AdvancedWorkersConfig {
            minerals_delta: 25,
            gas_delta: 25,
            build_time_delta_seconds: -7,
        }
    }
}

/// Ultraload: the tier-2 worker passive.
///
/// A tier-2 (advanced) worker that is working a resource node inside a base —
/// i.e. within `depot_radius_px` of a completed town hall of its own player —
/// moves at `speed_multiplier` times the tier-1 worker speed. The worker's
/// `flingy.top_speed` is raised while the condition holds and restored when it
/// does not, so the node's 42-frame mining slot stays the only throughput
/// ceiling: a saturated tier-2 base mines exactly what a saturated tier-1 base
/// mines, it just needs fewer workers to keep every patch busy.
#[derive(Debug, Clone, Copy)]
pub struct UltraloadConfig {
    /// Master switch. Defaults to on.
    pub enabled: bool,
    /// Speed multiplier applied while the passive is active (1.0 = off).
    pub speed_multiplier: f32,
    /// Radius around a completed town hall in which the passive is active,
    /// in pixels (24 px per tile).
    pub depot_radius_px: i32,
}

impl Default for UltraloadConfig {
    fn default() -> Self {
        UltraloadConfig {
            enabled: true,
            speed_multiplier: 1.5,
            depot_radius_px: 480,
        }
    }
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
    /// Advanced-worker tuning (`advanced_workers:` section). `None` = not
    /// configured (no deltas).
    pub advanced_workers: Option<AdvancedWorkersConfig>,
    /// Ultraload passive for tier-2 workers (`ultraload:` section).
    pub ultraload: UltraloadConfig,
    /// Optional gameplay mode (e.g. `antimeta`). `None` = no mode (default off).
    /// The `CM_PLUS_MODE` environment variable, when set, takes precedence over this.
    pub mode: Option<String>,
    /// Draft-mode option tables (`draft:` section).
    pub draft: Vec<DraftOption>,
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

    // Advanced-worker tuning: all keys optional, defaulting to the requested
    // values (+25/+25 ore/gas, -7s build time).
    if let Some(aw) = root.get("advanced_workers") {
        let Yaml::Map(entries) = aw else {
            return Err("`advanced_workers:` must be a map".to_string());
        };
        let get = |k: &str| entries.iter().find(|(kk, _)| kk == k).map(|(_, v)| v);
        let mut c = AdvancedWorkersConfig::default();
        if let Some(v) = get("minerals_delta").and_then(Yaml::as_i64) {
            c.minerals_delta = v as i32;
        }
        if let Some(v) = get("gas_delta").and_then(Yaml::as_i64) {
            c.gas_delta = v as i32;
        }
        if let Some(v) = get("build_time_delta_seconds").and_then(Yaml::as_i64) {
            c.build_time_delta_seconds = v as i32;
        }
        cfg.advanced_workers = Some(c);
    }

    // Ultraload: the tier-2 worker passive (conditional 1.5x speed in base).
    if let Some(ul) = root.get("ultraload") {
        let Yaml::Map(entries) = ul else {
            return Err("`ultraload:` must be a map".to_string());
        };
        let get = |k: &str| entries.iter().find(|(kk, _)| kk == k).map(|(_, v)| v);
        if let Some(s) = get("enabled").and_then(Yaml::as_str) {
            cfg.ultraload.enabled = parse_bool(s);
        }
        if let Some(v) = get("speed_multiplier").and_then(Yaml::as_f64) {
            let v = v as f32;
            if !(1.0..=4.0).contains(&v) {
                return Err("`ultraload.speed_multiplier:` must be within 1.0..=4.0".to_string());
            }
            cfg.ultraload.speed_multiplier = v;
        }
        if let Some(v) = get("depot_radius_px").and_then(Yaml::as_i64) {
            if !(0..=2000).contains(&v) {
                return Err("`ultraload.depot_radius_px:` must be within 0..=2000".to_string());
            }
            cfg.ultraload.depot_radius_px = v as i32;
        }
    }

    if let Some(mode) = root.get("mode") {
        match mode {
            Yaml::Scalar(s) => {
                let s = s.trim();
                if !s.is_empty() {
                    cfg.mode = Some(s.to_string());
                }
            }
            _ => return Err("`mode:` must be a scalar string".to_string()),
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

    if let Some(draft) = root.get("draft") {
        let Yaml::Map(entries) = draft else {
            return Err("`draft:` must be a map".to_string());
        };
        if let Some((_, options)) = entries.iter().find(|(k, _)| k == "options") {
            let Yaml::List(items) = options else {
                return Err("`draft.options:` must be a list".to_string());
            };
            cfg.draft = items.iter().map(parse_draft_option).collect::<Result<_, _>>()?;
        }
    }

    Ok(cfg)
}

/// Parse one `draft.options:` entry.
fn parse_draft_option(item: &Yaml) -> Result<DraftOption, String> {
    let Yaml::Map(entries) = item else {
        return Err("`draft.options:` items must be maps".to_string());
    };
    let get = |k: &str| entries.iter().find(|(kk, _)| kk == k).map(|(_, v)| v);

    let id = get("id").and_then(Yaml::as_str).ok_or("draft option missing `id:`")?;
    let name = get("name")
        .and_then(Yaml::as_str)
        .unwrap_or(id)
        .to_string();
    let min_draft = get("min_draft")
        .and_then(Yaml::as_i64)
        .unwrap_or(1)
        .max(1) as u32;
    let weight = get("weight")
        .and_then(Yaml::as_i64)
        .unwrap_or(10)
        .max(0) as u32;
    let pair = match get("pair") {
        Some(v) => parse_pair(v)?,
        None => None,
    };

    let effect = get("effect").ok_or(format!("draft option `{id}` missing `effect:`"))?;
    let effect = parse_effect(effect).map_err(|e| format!("draft option `{id}`: {e}"))?;

    Ok(DraftOption {
        id: id.to_string(),
        name,
        min_draft,
        weight,
        pair,
        effect,
    })
}

/// Parse a `pair:` value: `"Z/T"`, `"T/Z"` (unordered), `"*"`, or omitted.
fn parse_pair(v: &Yaml) -> Result<Option<(u8, u8)>, String> {
    let s = v.as_str().ok_or("`pair:` must be a string like \"Z/T\" or \"*\"")?;
    let s = s.trim();
    if s == "*" {
        return Ok(None);
    }
    let race_byte = |c: char| match c.to_ascii_uppercase() {
        'Z' => Ok(crate::draft::race::ZERG),
        'T' => Ok(crate::draft::race::TERRAN),
        'P' => Ok(crate::draft::race::PROTOSS),
        'A' => Ok(crate::draft::race::ASKOSI),
        'B' => Ok(crate::draft::race::BLOODSUMMON),
        other => Err(format!("unknown race letter `{other}` in pair `{s}`")),
    };
    let mut chars = s.chars().filter(|c| !c.is_whitespace() && *c != '/');
    let a = chars.next().map(race_byte).transpose()?.ok_or(format!("empty pair `{s}`"))?;
    let b = chars.next().map(race_byte).transpose()?.ok_or(format!("pair `{s}` needs two races"))?;
    if chars.next().is_some() {
        return Err(format!("pair `{s}` must be exactly two races"));
    }
    Ok(Some(if a <= b { (a, b) } else { (b, a) }))
}

/// Parse an `effect:` map into [`Effect`].
fn parse_effect(v: &Yaml) -> Result<Effect, String> {
    let Yaml::Map(entries) = v else {
        return Err("`effect:` must be a map with a `type:`".to_string());
    };
    let get = |k: &str| entries.iter().find(|(kk, _)| kk == k).map(|(_, v)| v);
    let ty = get("type")
        .and_then(Yaml::as_str)
        .ok_or("effect missing `type:`")?
        .trim();

    let i = |k: &str| get(k).and_then(Yaml::as_i64).unwrap_or(0) as i32;
    let u = |k: &str| get(k).and_then(Yaml::as_i64).unwrap_or(0) as u32;
    let anchor = || -> Result<Anchor, String> {
        match get("anchor").and_then(Yaml::as_str).unwrap_or("townhall").trim() {
            "townhall" => Ok(Anchor::TownHall),
            "expansion" => Ok(Anchor::Expansion),
            other => Err(format!("unknown anchor `{other}` (townhall|expansion)")),
        }
    };
    let units = || -> Result<Vec<u16>, String> {
        let Some(v) = get("units") else {
            return Err("spawn effect missing `units:` list".to_string());
        };
        let Yaml::List(items) = v else {
            return Err("`units:` must be a list of unit ids".to_string());
        };
        items
            .iter()
            .map(|it| {
                it.as_i64()
                    .and_then(|n| u16::try_from(n).ok())
                    .ok_or_else(|| "unit id out of range".to_string())
            })
            .collect()
    };

    Ok(match ty {
        "spawn" => Effect::SpawnUnits {
            units: units()?,
            count: u("count").max(1).min(12) as u8,
            anchor: anchor()?,
        },
        "resources" => Effect::Resources {
            minerals: i("minerals"),
            gas: i("gas"),
        },
        "heal_all" => Effect::HealAll,
        "recharge_all" => Effect::RechargeAll,
        "reveal" => Effect::Reveal {
            seconds: u("seconds").max(1),
        },
        "temp_weapon_levels" => Effect::TempWeaponLevels {
            levels: i("levels").clamp(-3, 3) as i8,
            seconds: u("seconds").max(1),
        },
        "temp_armor_levels" => Effect::TempArmorLevels {
            levels: i("levels").clamp(-3, 3) as i8,
            seconds: u("seconds").max(1),
        },
        "refund_token" => Effect::RefundToken,
        other => return Err(format!("unknown effect type `{other}`")),
    })
}

/// Validate that every race pair has enough options for every draft number the mode can
/// reach. Returns a list of human-readable gaps (empty = OK). Structural errors are
/// already rejected by the parser; this is a curation aid logged by the plugin.
pub fn validate_draft_pools(
    options: &[DraftOption],
    pairs: &[(u8, u8)],
    max_draft: u32,
) -> Vec<String> {
    use crate::draft::{OPTION_COUNT, tier_for_draft};

    let mut gaps = Vec::new();
    for &(a, b) in pairs {
        for draft in 1..=max_draft {
            let tier = tier_for_draft(draft);
            let available = options
                .iter()
                .filter(|o| {
                    o.min_draft <= draft
                        && match o.pair {
                            None => true,
                            Some(p) => p == (a, b),
                        }
                        && match tier {
                            // Lower-tier leftovers are allowed into higher tiers, so only
                            // require that the option's own tier isn't *above* this one.
                            _ => true,
                        }
                })
                .count();
            if available < OPTION_COUNT {
                gaps.push(format!(
                    "pair ({a},{b}) draft {draft} ({tier:?}): {available} options (< {OPTION_COUNT})"
                ));
            }
        }
    }
    gaps
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

    #[test]
    fn mode_field_parses_and_defaults_off() {
        assert!(parse_plus_yml("").unwrap().mode.is_none());
        let cfg = parse_plus_yml("mode: antimeta\n").unwrap();
        assert_eq!(cfg.mode.as_deref(), Some("antimeta"));
        let cfg = parse_plus_yml("mode: \"antimeta\"\n").unwrap();
        assert_eq!(cfg.mode.as_deref(), Some("antimeta"));
        // The YAML subset keeps all scalars as strings; a numeric mode is a non-matching
        // string, not an error.
        assert_eq!(parse_plus_yml("mode: 42\n").unwrap().mode.as_deref(), Some("42"));
        // A structured value is rejected.
        assert!(parse_plus_yml("mode:\n  - x\n").is_err());
    }

    #[test]
    fn draft_section_parses() {
        let yml = "\
draft:
  options:
    - id: prospectors
      name: Prospectors
      min_draft: 1
      weight: 10
      effect:
        type: resources
        minerals: 150
    - id: vanguard
      min_draft: 1
      pair: Z/T
      effect:
        type: spawn
        units: [7]
        count: 2
        anchor: townhall
    - id: bloodlust
      min_draft: 6
      pair: \"T/Z\"
      effect:
        type: temp_weapon_levels
        levels: 1
        seconds: 90
";
        let cfg = parse_plus_yml(yml).unwrap();
        assert_eq!(cfg.draft.len(), 3);
        assert_eq!(cfg.draft[0].id, "prospectors");
        assert_eq!(cfg.draft[0].name, "Prospectors"); // explicit name
        assert_eq!(cfg.draft[1].name, "vanguard"); // name falls back to id
        assert_eq!(
            cfg.draft[0].effect,
            Effect::Resources { minerals: 150, gas: 0 }
        );
        assert_eq!(
            cfg.draft[1].effect,
            Effect::SpawnUnits {
                units: vec![7],
                count: 2,
                anchor: Anchor::TownHall
            }
        );
        // "T/Z" normalises to the same unordered pair as "Z/T".
        assert_eq!(cfg.draft[1].pair, Some((0, 1)));
        assert_eq!(cfg.draft[2].pair, Some((0, 1)));
        assert_eq!(
            cfg.draft[2].effect,
            Effect::TempWeaponLevels { levels: 1, seconds: 90 }
        );
    }

    #[test]
    fn draft_section_rejects_bad_shapes() {
        // Missing effect.
        assert!(parse_plus_yml("draft:\n  options:\n    - id: x\n").is_err());
        // Unknown effect type.
        assert!(
            parse_plus_yml("draft:\n  options:\n    - id: x\n      effect:\n        type: magic\n")
                .is_err()
        );
        // Spawn without units.
        assert!(
            parse_plus_yml("draft:\n  options:\n    - id: x\n      effect:\n        type: spawn\n")
                .is_err()
        );
        // Bad pair letter.
        assert!(
            parse_plus_yml(
                "draft:\n  options:\n    - id: x\n      pair: Z/Q\n      effect:\n        type: heal_all\n"
            )
            .is_err()
        );
        // draft.options must be a list.
        assert!(
            parse_plus_yml("draft:\n  options: nope\n").is_err()
        );
    }

    #[test]
    fn validate_draft_pools_reports_gaps() {
        use crate::draft::{OPTION_COUNT, race};
        let mk = |id: &str, min: u32| DraftOption {
            id: id.to_string(),
            name: id.to_string(),
            min_draft: min,
            weight: 10,
            pair: None,
            effect: Effect::HealAll,
        };
        let options = vec![mk("a", 1), mk("b", 1), mk("c", 4), mk("d", 6)];
        let gaps = validate_draft_pools(&options, &[(race::ZERG, race::TERRAN)], 8);
        // Drafts 1-3 have 2 options (3 gaps), 4-5 have 3 (2 gaps), 6+ have exactly 4.
        assert_eq!(gaps.len(), 5);
        assert!(gaps.iter().all(|g| g.contains("< 4")));
        let gaps = validate_draft_pools(&options, &[], 8);
        assert!(gaps.is_empty());
        // 4 options from draft 1 onwards = no gaps.
        let full = vec![
            mk("a", 1), mk("b", 1), mk("c", 1), mk("d", 1),
        ];
        assert!(validate_draft_pools(&full, &[(0, 1)], 20).is_empty());
        assert_eq!(OPTION_COUNT, 4);
    }

    #[test]
    fn ultraload_defaults_to_on_at_one_and_a_half() {
        let cfg = parse_plus_yml("saturation:\n  tier1_default: 8\n").unwrap();
        assert!(cfg.ultraload.enabled);
        assert!((cfg.ultraload.speed_multiplier - 1.5).abs() < f32::EPSILON);
        assert_eq!(cfg.ultraload.depot_radius_px, 480);
    }

    #[test]
    fn ultraload_parses_and_validates() {
        let cfg = parse_plus_yml(
            "ultraload:\n  enabled: false\n  speed_multiplier: 2.0\n  depot_radius_px: 300\n",
        )
        .unwrap();
        assert!(!cfg.ultraload.enabled);
        assert!((cfg.ultraload.speed_multiplier - 2.0).abs() < f32::EPSILON);
        assert_eq!(cfg.ultraload.depot_radius_px, 300);

        assert!(parse_plus_yml("ultraload:\n  speed_multiplier: 9.0\n").is_err());
        assert!(parse_plus_yml("ultraload:\n  depot_radius_px: -5\n").is_err());

        // The old payload multiplier is gone from the schema; it is ignored, not parsed.
        let cfg = parse_plus_yml("advanced_workers:\n  mining_multiplier: 1.5\n").unwrap();
        assert!(cfg.advanced_workers.is_some());
    }
}
