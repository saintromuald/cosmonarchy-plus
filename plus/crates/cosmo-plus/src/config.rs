//! Config loading (process environment + file lookup).
//!
//! Two files:
//!   * `cosmo_plus.conf` — a simple `key=value` file (`#` starts a comment). Every key
//!     must start with `CM_PLUS_`; each becomes an environment variable, which is what
//!     the rest of the plugin reads. Looked up in the working directory (the game's
//!     directory) and then in `plugins/`.
//!   * `plus.yml` — the gameplay data overrides (absolute unit costs, gas-tier deltas,
//!     saturation tuning). Parsed by `cosmo_plus_core::config`; looked up in the working
//!     directory and then `plugins/`.
//!
//! Gameplay rules that are not per-install knobs live in `plus.yml`; see
//! `docs/PLUS-DATA.md`.

use cosmo_plus_core::config::PlusConfig;

use once_cell::sync::Lazy;
use std::sync::Mutex;

/// Config keys applied from `cosmo_plus.conf` (for the boot state report).
static APPLIED_KEYS: Lazy<Mutex<Vec<String>>> = Lazy::new(|| Mutex::new(Vec::new()));

/// The keys the `plus:` section of `cosmonarchy.yml` (the mod's own config
/// file) may set, mapped to their `CM_PLUS_*` counterparts. The plugin's own
/// `cosmo_plus.conf` wins over the yml section; real environment variables
/// win over both.
const YML_KEYS: &[(&str, &str)] = &[
    ("idle-key", "CM_PLUS_IDLE_WORKER_KEY"),
    ("tab-key", "CM_PLUS_TAB_KEY"),
    ("tab-prev-key", "CM_PLUS_TAB_PREV_KEY"),
    ("larva-key", "CM_PLUS_LARVA_KEY"),
    ("changes-key", "CM_PLUS_CHANGES_KEY"),
    ("overlay-font", "CM_PLUS_OVERLAY_FONT"),
    ("idle-badge-x", "CM_PLUS_IDLE_BADGE_X"),
    ("idle-badge-y", "CM_PLUS_IDLE_BADGE_Y"),
    ("zoom-floor", "CM_PLUS_ZOOM_PLAYER_MIN"),
    ("mode", "CM_PLUS_MODE"),
    ("saturation", "CM_PLUS_SATURATION"),
    ("no-tab-cycle", "CM_PLUS_NO_TAB_CYCLE"),
    ("repl", "CM_PLUS_REPL"),
];

/// Apply the `plus:` section of `cosmonarchy.yml` (looked up in the game
/// directory, `plugins/` and next to Starcraft.exe, like everything else).
///
/// The mod's config file is a full YAML document the shared parser is too
/// strict for, so the `plus:` block is extracted with a dedicated line scan:
/// the section is a top-level `plus:` key whose values are scalars. Only keys
/// not already set by a real environment variable are applied;
/// `cosmo_plus.conf` runs after this and wins.
fn load_yml_plus_section() -> u32 {
    let mut applied = 0u32;
    for path in [
        "cosmonarchy.yml",
        "plugins/cosmonarchy.yml",
        "../Starcraft/cosmonarchy.yml",
        "../../Starcraft/cosmonarchy.yml",
    ] {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let mut section_indent: Option<usize> = None;
        for line in text.lines() {
            let trimmed = line.trim_start();
            let indent = line.len() - trimmed.len();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            match section_indent {
                None => {
                    if trimmed == "plus:" {
                        section_indent = Some(indent);
                    }
                }
                Some(sec) => {
                    if indent <= sec {
                        break; // out of the plus: block
                    }
                    let Some((key, value)) = trimmed.split_once(':') else {
                        continue;
                    };
                    let key = key.trim();
                    let value = value.split('#').next().unwrap_or("").trim();
                    let value = value.trim_matches(['"', '\'']);
                    let Some((_, env_key)) = YML_KEYS.iter().find(|(k, _)| *k == key) else {
                        continue;
                    };
                    if std::env::var(env_key).is_ok() {
                        continue; // a real environment variable already set it
                    }
                    std::env::set_var(env_key, value);
                    APPLIED_KEYS
                        .lock()
                        .unwrap()
                        .push(format!("{env_key} (from {path} plus.{key})"));
                    applied += 1;
                }
            }
        }
        if applied > 0 {
            crate::log::log(&format!(
                "[cosmo-plus] cosmonarchy.yml plus: section: {applied} setting(s)"
            ));
        }
        return applied;
    }
    applied
}

/// The keys applied from the conf file and the yml `plus:` section, in order.
pub fn applied_keys() -> Vec<String> {
    APPLIED_KEYS.lock().unwrap().clone()
}

pub fn load() -> u32 {
    // The mod's own config file first (its `plus:` section), so one file can
    // hold everything; the plugin's conf then overrides it.
    let mut applied = load_yml_plus_section();
    for path in [
        "cosmo_plus.conf",
        "plugins/cosmo_plus.conf",
        // A user-owned conf next to Starcraft.exe: the launcher re-syncs the
        // version directory from git (clobbering edits there), and the mod
        // rewrites cosmonarchy.yml on exit (dropping unknown keys) — this
        // location survives both, so personal settings belong here.
        "../Starcraft/cosmo_plus.conf",
        "../../Starcraft/cosmo_plus.conf",
    ] {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim().trim_matches('"');
            if !key.starts_with("CM_PLUS_") || key.len() == "CM_PLUS_".len() {
                continue;
            }
            std::env::set_var(key, value);
            APPLIED_KEYS.lock().unwrap().push(key.to_string());
            applied += 1;
        }
        if applied > 0 {
            crate::log::log(&format!("[cosmo-plus] armed from {path}: {applied} setting(s)"));
        }
    }
    applied
}

pub fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|s| !s.is_empty())
}

pub fn env_flag(key: &str) -> bool {
    matches!(env(key).as_deref(), Some("1") | Some("true") | Some("yes") | Some("on"))
}

/// Load `plus.yml` from the working directory, then `plugins/`. Returns `None` if the
/// file is absent from both (vanilla behaviour).
pub fn load_plus_yml() -> Option<PlusConfig> {
    for path in ["plus.yml", "plugins/plus.yml"] {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        match cosmo_plus_core::config::parse_plus_yml(&text) {
            Ok(cfg) => {
                crate::log::log(&format!(
                    "[cosmo-plus] loaded {path}: {} override(s), {} gas tier(s)",
                    cfg.overrides.len(),
                    cfg.gas_tier_deltas.len()
                ));
                return Some(cfg);
            }
            Err(e) => {
                crate::log::log(&format!("[cosmo-plus] ERROR parsing {path}: {e}"));
                return None;
            }
        }
    }
    None
}
