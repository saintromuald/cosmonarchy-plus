//! Boot-time integration report: every fixed address the plugin reads is
//! resolved, sanity-checked and written to `cosmo_plus.state.json`, together
//! with the armed/disarmed status of each feature, the config keys applied and
//! every hook the plugin installed.
//!
//! This turns "a feature silently stopped working" into a diff between two
//! boot reports: a relocated global, a failed hook or a missing config key
//! shows up here instead of hours of archaeology. Features that depend on a
//! global read the *validated* value, so a bad address disarms the feature
//! with an explicit log instead of drawing garbage or crashing later.

use std::collections::BTreeMap;
use std::sync::Mutex;

use once_cell::sync::Lazy;

/// A fixed global with a reader and a plausibility check.
struct Global {
    name: &'static str,
    addr: usize,
    /// Format the raw bytes at `addr` for the report.
    read: unsafe fn(usize) -> String,
    /// None = always ok; Some(err) = why the value is not plausible.
    check: unsafe fn(usize) -> Option<&'static str>,
    /// Strict globals are only *required* once a game is running: several UI
    /// globals (icons GRP, fonts, draw-text box) are legitimately zero while
    /// the game is still booting, and a boot-stage report must not flag them.
    strict: bool,
}

unsafe fn read_i32(addr: usize) -> String {
    format!("{}", std::ptr::read(addr as *const i32))
}

unsafe fn read_u32(addr: usize) -> String {
    format!("{:#x}", std::ptr::read(addr as *const u32))
}

unsafe fn read_u8(addr: usize) -> String {
    format!("{:#x}", std::ptr::read(addr as *const u8))
}

unsafe fn read_ptr(addr: usize) -> String {
    format!("{:p}", std::ptr::read(addr as *const *const u8))
}

unsafe fn read_rect(addr: usize) -> String {
    let r = std::ptr::read(addr as *const [i32; 4]);
    format!("{r:?}")
}

unsafe fn check_rect(addr: usize) -> Option<&'static str> {
    let r = std::ptr::read(addr as *const [i32; 4]);
    if r[0] >= r[2] || r[1] >= r[3] {
        return Some("left>=right or top>=bottom");
    }
    for v in r {
        if !(-16384..16384).contains(&v) {
            return Some("coordinate outside +-16384");
        }
    }
    None
}

unsafe fn check_nonnull(addr: usize) -> Option<&'static str> {
    if std::ptr::read(addr as *const *const u8).is_null() {
        Some("null")
    } else {
        None
    }
}

unsafe fn check_code(addr: usize) -> Option<&'static str> {
    let v = std::ptr::read(addr as *const u32);
    if v == 0 || v == 0xFFFF_FFFF {
        Some("not a live code pointer")
    } else {
        None
    }
}

unsafe fn check_any(_: usize) -> Option<&'static str> {
    None
}

/// The globals the plugin depends on. Read at report time so the dump always
/// reflects live state (and a REPL `state` command can re-verify).
const GLOBALS: &[Global] = &[
    Global { name: "G_MINIMAP_RECT", addr: 0x0051_2D00, read: read_rect, check: check_rect, strict: true },
    // Inline arrays, not pointers: always "ok" once readable.
    Global { name: "G_PLAYER_TABLE", addr: 0x0057_EEE0, read: read_u32, check: check_any, strict: false },
    // Loaded with the UI: legitimately zero while the game boots.
    Global { name: "G_CMDICONS_GRP", addr: 0x0068_C1E0, read: read_ptr, check: check_nonnull, strict: true },
    Global { name: "FONT_BASE", addr: 0x006C_E0F4, read: read_ptr, check: check_nonnull, strict: true },
    // Scratch memory rewritten by every UI pass: report it, never fail on it.
    Global { name: "G_DRAW_TEXT", addr: 0x006C_E0C8, read: read_rect, check: check_any, strict: false },
    Global { name: "G_PRINT_XY_ALIGNMENT", addr: 0x006C_E110, read: read_u8, check: check_any, strict: false },
    Global { name: "G_SCREEN_X", addr: 0x0062_8448, read: read_i32, check: check_any, strict: false },
    Global { name: "G_SCREEN_Y", addr: 0x0062_8470, read: read_i32, check: check_any, strict: false },
    Global { name: "CHAT_MESSAGE_TYPE", addr: 0x0068_C144, read: read_ptr, check: check_any, strict: false },
    Global { name: "POPUP_DIALOG", addr: 0x006D_5BF4, read: read_u32, check: check_any, strict: false },
    Global { name: "BW_PRINT_XY", addr: 0x0042_02B0, read: read_u32, check: check_code, strict: true },
    Global { name: "BW_PRINT_ICON", addr: 0x0041_D090, read: read_u32, check: check_code, strict: true },
    Global { name: "SET_FONT_HANDLE", addr: 0x0041_FB30, read: read_u32, check: check_code, strict: true },
    Global { name: "GET_COLOR_HANDLE", addr: 0x0041_F610, read: read_u32, check: check_code, strict: true },
    Global { name: "DRAW_GAME_PROC_PTR", addr: 0x004B_D68D, read: read_u32, check: check_code, strict: true },
    Global { name: "SELECT_UNITS", addr: 0x0046_FA00, read: read_u32, check: check_code, strict: true },
];

/// Feature status registry: `feature("name", "armed: detail")`.
static FEATURES: Lazy<Mutex<BTreeMap<&'static str, String>>> =
    Lazy::new(|| Mutex::new(BTreeMap::new()));

/// Hook registry: name -> (target address, what was there, what we put there).
static HOOKS: Lazy<Mutex<BTreeMap<&'static str, String>>> =
    Lazy::new(|| Mutex::new(BTreeMap::new()));

/// Record a feature's status (armed with details, or disarmed with the reason).
pub(crate) fn feature(name: &'static str, status: impl Into<String>) {
    FEATURES.lock().unwrap().insert(name, status.into());
}

/// Number of recorded hooks, and how many of them are keyboard hooks.
pub(crate) fn hook_count() -> usize {
    HOOKS.lock().unwrap().len()
}

pub(crate) fn keyboard_hook_count() -> usize {
    HOOKS
        .lock()
        .unwrap()
        .keys()
        .filter(|k| k.starts_with("keyboard:"))
        .count()
}

/// Record a hook installation for the report.
pub(crate) fn hook(name: &'static str, target: usize, old: usize, new: usize) {
    HOOKS.lock().unwrap().insert(
        name,
        format!("target=0x{target:08X} old=0x{old:08X} new=0x{new:08X}"),
    );
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Write `cosmo_plus.state.json` in the game directory: build identity, loaded
/// modules, every global with its live value and sanity verdict, feature
/// statuses, config keys and installed hooks.
pub(crate) unsafe fn write_report(stage: &'static str) {
    let in_game = stage == "in-game";
    let mut globals = String::new();
    let mut bad = 0usize;
    for g in GLOBALS {
        let value = (g.read)(g.addr);
        let err = (g.check)(g.addr);
        // Strict globals that are still zero during boot are "pending", not
        // broken; in-game they must hold.
        let ok = if err.is_some() && g.strict && !in_game {
            "pending"
        } else if err.is_some() {
            bad += 1;
            "false"
        } else {
            "true"
        };
        globals.push_str(&format!(
            "    \"{}\": {{\"addr\":\"0x{:08X}\",\"value\":\"{}\",\"ok\":\"{}\"}},\n",
            json_escape(g.name),
            g.addr,
            json_escape(&value),
            ok,
        ));
    }
    if globals.ends_with(",\n") {
        globals.pop();
        globals.pop();
    }

    let features = FEATURES
        .lock()
        .unwrap()
        .iter()
        .map(|(k, v)| {
            format!(
                "    \"{}\": \"{}\",\n",
                json_escape(k),
                json_escape(v)
            )
        })
        .collect::<String>();
    let features = features.trim_end_matches(",\n").to_string();

    let hooks = HOOKS
        .lock()
        .unwrap()
        .iter()
        .map(|(k, v)| format!("    \"{}\": \"{}\",\n", json_escape(k), json_escape(v)))
        .collect::<String>();
    let hooks = hooks.trim_end_matches(",\n").to_string();

    let keys = crate::config::applied_keys()
        .iter()
        .map(|k| format!("    \"{}\",\n", json_escape(k)))
        .collect::<String>();
    let keys = keys.trim_end_matches(",\n").to_string();

    let (gptp_base, gptp_size) = crate::zoom::gptp_module().unwrap_or((0, 0));

    let text = format!(
        "{{\n\
         \x20 \"build\": {{\"version\":\"{}\",\"abi\":{},\"build_id\":\"{:#x}\",\"rev\":\"{}\"}},\n\
         \x20 \"modules\": {{\"gptp.qdp\": {{\"base\":\"{:#x}\",\"size\":{}}}, \"cosmo_plus.dll\": {{\"base\":\"{:#x}\"}}}},\n\
         \x20 \"stage\": \"{}\",\n         \x20 \"globals_bad\": {},\n\
         \x20 \"globals\": {{\n{}\n }},\n\
         \x20 \"features\": {{\n{}\n }},\n\
         \x20 \"config_keys\": [\n{}\n ],\n\
         \x20 \"hooks\": {{\n{}\n }}\n\
         }}",
        env!("CARGO_PKG_VERSION"),
        crate::cosmo_plus_abi(),
        crate::cosmo_plus_build_id(),
        env!("COSMO_PLUS_GIT_REV"),
        gptp_base,
        gptp_size,
        crate::plugin::self_base(),
        stage,
        bad,
        globals,
        features,
        keys,
        hooks,
    );

    if let Ok(mut f) = std::fs::File::create("cosmo_plus.state.json") {
        use std::io::Write;
        let _ = f.write_all(text.as_bytes());
    }
    crate::log::log(&format!(
        "[cosmo-plus] state: {bad} global(s) fail plausibility; report written to cosmo_plus.state.json"
    ));
}

/// The validated minimap rect, or None when the global no longer holds sane
/// values (a relocated UI global — the feature that reads it should disarm).
pub(crate) unsafe fn minimap_rect() -> Option<[i32; 4]> {
    let addr = 0x0051_2D00;
    match check_rect(addr) {
        None => Some(std::ptr::read(addr as *const [i32; 4])),
        Some(_) => None,
    }
}

/// Generic byte-pattern scan (signature search) over a memory range. Used to
/// resolve addresses that move between builds instead of trusting constants.
pub(crate) unsafe fn scan(addr: usize, len: usize, pat: &[u8], mask: &[u8]) -> Option<usize> {
    if pat.len() != mask.len() || pat.is_empty() || len < pat.len() {
        return None;
    }
    let base = addr as *const u8;
    let end = len - pat.len();
    'outer: for off in 0..=end {
        for (i, &p) in pat.iter().enumerate() {
            if mask[i] == b'x' && *base.add(off + i) != p {
                continue 'outer;
            }
        }
        return Some(addr + off);
    }
    None
}
