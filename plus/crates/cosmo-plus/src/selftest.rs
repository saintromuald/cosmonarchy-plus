//! Boot self-test, gated by `CM_PLUS_SELFTEST=1`: the checks that need the
//! live game context (resolved globals, api function pointers, module
//! presence) plus the pure core checks that matter at boot. Writes a
//! machine-readable verdict to `cosmo_plus.selftest` so the headless harness
//! (`tools/verify-plus.sh`) can gate on it without parsing prose.

use cosmo_plus_core::hotkeys::parse_key;

pub(crate) unsafe fn run() {
    if !crate::config::env_flag("CM_PLUS_SELFTEST") {
        return;
    }
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut fails: Vec<&'static str> = Vec::new();

    macro_rules! check {
        ($name:expr, $cond:expr) => {{
            if $cond {
                passed += 1;
            } else {
                failed += 1;
                fails.push($name);
                crate::log::log(&format!("[cosmo-plus] selftest: FAIL {0}", $name));
            }
        }};
    }

    // Hotkey parsing (pure).
    check!("hotkey-comma", parse_key(",") == Some(0xBC));
    check!("hotkey-f9", parse_key("F9") == Some(0x78));
    check!("hotkey-name", parse_key("COMMA") == Some(0xBC));
    check!("hotkey-hex", parse_key("0x7A") == Some(0x7A));
    check!("hotkey-garbage", parse_key("nonsense").is_none());

    // Plugin api surface.
    let api = crate::plus::api();
    check!("api.game", (api.game)().is_some());
    check!("api.first_active_unit", (api.first_active_unit)().is_some());
    check!("api.first_hidden_unit", (api.first_hidden_unit)().is_some());
    check!("api.local_player_id", (api.local_player_id)().is_some());
    let write_mem = api.write_exe_memory as usize;
    check!("api.write_exe_memory", write_mem != 0);

    // Loaded modules.
    check!("gptp-present", crate::zoom::gptp_module().is_some());

    // Validated globals.
    check!("minimap-rect-sane", crate::state::minimap_rect().is_some());

    // Hook ownership: idle-workers, tab-cycle and larva-pick install their
    // keyboard hooks at boot (cam-pan's waits for the game window).
    check!(
        "keyboard-hooks",
        crate::state::keyboard_hook_count() >= 3
    );

    // State report was written and parses as JSON-ish.
    let report_ok = std::fs::read_to_string("cosmo_plus.state.json")
        .map(|t| t.contains("\"globals_bad\": 0"))
        .unwrap_or(false);
    check!("state-report", report_ok);

    let verdict = if failed == 0 { "PASS" } else { "FAIL" };
    if let Ok(mut f) = std::fs::File::create("cosmo_plus.selftest") {
        use std::io::Write;
        let _ = write!(
            f,
            "{verdict} {passed} passed, {failed} failed{}",
            if fails.is_empty() {
                String::new()
            } else {
                format!(": {}", fails.join(", "))
            }
        );
    }
    crate::log::log(&format!(
        "[cosmo-plus] selftest: {verdict} {passed} passed, {failed} failed"
    ));
}
