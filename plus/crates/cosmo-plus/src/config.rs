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

/// The keys applied from the conf file, in order.
pub fn applied_keys() -> Vec<String> {
    APPLIED_KEYS.lock().unwrap().clone()
}

pub fn load() -> u32 {
    let mut applied = 0u32;
    for path in ["cosmo_plus.conf", "plugins/cosmo_plus.conf"] {
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
