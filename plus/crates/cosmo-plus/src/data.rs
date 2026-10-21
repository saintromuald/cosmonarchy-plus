//! Live dat access: read/patch the in-memory `units.dat` / `flingy.dat` tables.
//!
//! The dat is read through `bw_dat` (the same crate ShieldBattery/aise use), which we
//! initialise against the live DatTable array resolved by `samase_shim`. Overrides are
//! written directly to the dat field arrays (the source of truth the engine charges
//! from), never to a `.datex` file — that keeps Plus reviewable and revertable.
//!
//! Field indices are the fixed 1.16.1 + DatExtend layout (`BW/bwdata.h`):
//!   units.dat:  0 = graphic/flingy id (u16), 40 = mineral cost (u16), 41 = gas (u16),
//!               42 = build time (u16); 54 fields total.
//!   flingy.dat: 1 = top speed (u32, 1/256 px/frame), 2 = acceleration (u16),
//!               4 = turn speed (u8); 7 fields total.

use std::collections::BTreeMap;

use bw_dat::structs::DatTable;

use cosmo_plus_core::config::PlusConfig;
use cosmo_plus_core::saturation;

/// units.dat field indices.
const FIELD_MINERAL_COST: usize = 40;
const FIELD_GAS_COST: usize = 41;
const FIELD_BUILD_TIME: usize = 42;
/// Number of fields in the extended units.dat / flingy.dat.
const UNITS_FIELD_COUNT: usize = 54;
const FLINGY_FIELD_COUNT: usize = 7;
/// flingy.dat field index for top speed.
const FIELD_FLINGY_TOP_SPEED: usize = 1;

/// `bw_dat` dat init done (once per process; the dat loads once at startup).
static DAT_INIT: std::sync::Once = std::sync::Once::new();

/// Resolve the dat table pointers and initialise `bw_dat`. Safe to call repeatedly;
/// the actual initialisation runs once.
pub unsafe fn init() {
    let api = crate::plus::api();
    DAT_INIT.call_once(|| unsafe {
        let units = resolve(api, 0);
        let flingy = resolve(api, 2);
        if let Some(units) = units {
            bw_dat::init_units(units as *const _, UNITS_FIELD_COUNT);
        }
        if let Some(flingy) = flingy {
            bw_dat::init_flingy(flingy as *const _, FLINGY_FIELD_COUNT);
        }
        let units_entries = if let Some(units) = units {
            (*(units as *const DatTable)).entries
        } else {
            0
        };
        crate::log::log(&format!(
            "[cosmo-plus] dat init: units_dat=0x{:x} ({} entries), flingy_dat=0x{:x}",
            units.map(|p| p as usize).unwrap_or(0),
            units_entries,
            flingy.map(|p| p as usize).unwrap_or(0),
        ));
    });
}

/// Resolve a dat table pointer via the plugin api (`dat_type`: 0 = units, 2 = flingy).
unsafe fn resolve(api: &samase_shim::PluginApi, dat_type: u32) -> Option<*mut core::ffi::c_void> {
    let f = (api.dat)(dat_type)?;
    Some(f())
}

/// The number of unit entries in the live units.dat.
pub fn unit_entry_count() -> u32 {
    bw_dat::UnitId::entry_amount()
}

/// Read a unit's (minerals, gas, build_time) from the live table.
pub fn read_cost(id: u16) -> (u32, u32, u32) {
    let u = bw_dat::UnitId(id);
    (u.mineral_cost(), u.gas_cost(), u.build_time())
}

/// A worker's flingy top speed in px/frame, if it has a valid flingy entry.
pub fn worker_speed_px_per_frame(id: u16) -> Option<f64> {
    let u = bw_dat::UnitId(id);
    let flingy = u.flingy();
    if flingy.0 == 0 {
        return None;
    }
    Some(saturation::top_speed_to_px_per_frame(flingy.top_speed()))
}

/// Whether a unit id is flagged as a worker in the live table.
pub fn is_worker(id: u16) -> bool {
    bw_dat::UnitId(id).is_worker()
}

/// Direct write access to a `u16`-element dat field, for applying overrides.
///
/// `dat_type` selects the table (0 = units), `field` the field index. Returns the old
/// value so the caller can log old -> new.
unsafe fn write_u16_field(dat_type: u32, field: usize, id: u16, new: u16) -> Option<u16> {
    let base = resolve(crate::plus::api(), dat_type)?;
    let table = base as *const DatTable;
    let field = &*table.add(field);
    if field.entry_size != 2 || id as u32 >= field.entries {
        return None;
    }
    let data = field.data as *mut u16;
    let old = unsafe { *data.add(id as usize) };
    unsafe { *data.add(id as usize) = new };
    Some(old)
}

/// Apply the config's absolute overrides and gas-tier deltas to the live dat.
/// Logs every applied change as `old -> new`. Returns the number of fields written.
pub unsafe fn apply_overrides(cfg: &PlusConfig, names: &[String]) -> u32 {
    let mut applied = 0u32;

    // Absolute overrides, resolved by name (from units.txt, id = index).
    let name_to_id: BTreeMap<&str, u16> = names
        .iter()
        .enumerate()
        .map(|(i, n)| (n.trim(), i as u16))
        .collect();

    for o in &cfg.overrides {
        let Some(&id) = name_to_id.get(o.unit.trim()) else {
            crate::log::log(&format!(
                "[cosmo-plus] WARNING override for unknown unit `{}` (skipped)",
                o.unit
            ));
            continue;
        };
        let (om, og, ot) = read_cost(id);
        if let Some(m) = o.minerals {
            if m != om as u16 {
                if let Some(old) = write_u16_field(0, FIELD_MINERAL_COST, id, m) {
                    crate::log::log(&format!(
                        "[cosmo-plus] override {} (id {id}): minerals {old} -> {m}",
                        o.unit
                    ));
                    applied += 1;
                }
            }
        }
        if let Some(g) = o.gas {
            if g != og as u16 {
                if let Some(old) = write_u16_field(0, FIELD_GAS_COST, id, g) {
                    crate::log::log(&format!(
                        "[cosmo-plus] override {} (id {id}): gas {old} -> {g}",
                        o.unit
                    ));
                    applied += 1;
                }
            }
        }
        if let Some(t) = o.build_time {
            if t != ot as u16 {
                if let Some(old) = write_u16_field(0, FIELD_BUILD_TIME, id, t) {
                    crate::log::log(&format!(
                        "[cosmo-plus] override {} (id {id}): build_time {old} -> {t}",
                        o.unit
                    ));
                    applied += 1;
                }
            }
        }
    }

    // Gas-tier deltas: applied to every extractor of that public tier.
    for (&tier, delta) in &cfg.gas_tier_deltas {
        let Some(units) = saturation::GAS_EXTRACTOR_TIERS
            .iter()
            .find(|(t, _)| *t == tier)
            .map(|(_, u)| *u)
        else {
            crate::log::log(&format!("[cosmo-plus] WARNING gas tier {tier} has no known units"));
            continue;
        };
        for &id in units {
            let (m, _, t) = read_cost(id);
            if delta.minerals != 0 {
                let new = (m as i64 + delta.minerals as i64).clamp(0, u16::MAX as i64) as u16;
                if let Some(old) = write_u16_field(0, FIELD_MINERAL_COST, id, new) {
                    crate::log::log(&format!(
                        "[cosmo-plus] gas tier {tier} (id {id}): minerals {old} -> {new}"
                    ));
                    applied += 1;
                }
            }
            if delta.build_time_seconds != 0 {
                let delta_frames = delta.build_time_seconds * saturation::FPS as i32;
                let new = (t as i64 + delta_frames as i64).clamp(0, u16::MAX as i64) as u16;
                if let Some(old) = write_u16_field(0, FIELD_BUILD_TIME, id, new) {
                    crate::log::log(&format!(
                        "[cosmo-plus] gas tier {tier} (id {id}): build_time {old} -> {new}"
                    ));
                    applied += 1;
                }
            }
        }
    }

    applied
}

/// Load `mpq/arr/units.txt` (id = line index) into a name list. Empty on failure.
pub fn load_unit_names() -> Vec<String> {
    for path in ["mpq/arr/units.txt", "plugins/../mpq/arr/units.txt"] {
        if let Ok(text) = std::fs::read_to_string(path) {
            return text.lines().map(|s| s.trim_end().to_string()).collect();
        }
    }
    Vec::new()
}

/// Dump every unit id's cost + worker speed to `cosmo_plus_dump.csv`.
pub fn dump_csv(names: &[String]) {
    let count = unit_entry_count();
    if count == 0 || count == u32::MAX {
        crate::log::log("[cosmo-plus] dump skipped: units.dat entry count unavailable");
        return;
    }
    let mut csv = String::from(
        "id,name,minerals,gas,build_time,flingy_id,top_speed_raw,speed_px_per_frame,is_worker,tier\n",
    );
    for id in 0..count.min(3000) as u16 {
        let (m, g, t) = read_cost(id);
        let u = bw_dat::UnitId(id);
        let flingy = u.flingy();
        let flingy_id = flingy.0;
        let top_speed_raw = flingy.top_speed();
        let speed = if flingy_id == 0 {
            None
        } else {
            Some(saturation::top_speed_to_px_per_frame(top_speed_raw))
        };
        let speed_s = match speed {
            Some(s) => format!("{s:.4}"),
            None => "0".to_string(),
        };
        let worker = if u.is_worker() { 1 } else { 0 };
        let tier = saturation::worker_tier(id).map(|t| t.to_string()).unwrap_or_default();
        let name = names.get(id as usize).map(|s| s.as_str()).unwrap_or("");
        // Minimal CSV escaping (names have no commas/quotes in practice).
        csv.push_str(&format!(
            "{id},{name},{m},{g},{t},{flingy_id},{top_speed_raw},{speed_s},{worker},{tier}\n"
        ));
    }
    if let Ok(mut f) = std::fs::File::create("cosmo_plus_dump.csv") {
        use std::io::Write;
        let _ = f.write_all(csv.as_bytes());
        crate::log::log(&format!(
            "[cosmo-plus] dumped {} unit rows to cosmo_plus_dump.csv",
            count.min(3000)
        ));
    } else {
        crate::log::log("[cosmo-plus] ERROR: could not write cosmo_plus_dump.csv");
    }
}
