//! Cosmonarchy Plus feature installation.
//!
//! Everything here must be:
//!   * deterministic (same inputs -> same result on every peer),
//!   * driven by snapped/derived game state (no wall clock, no unseeded RNG),
//!   * independent of the netcode crate (optional handshake only, by export name).
//!
//! Features land in this order:
//!   1. data overrides (`plus.yml`) — costs/build times,
//!   2. saturation model — measured worker trip cadence,
//!   3. saturation overlay — read-only drawing (separate task),
//!   4. anti-meta mode — deterministic start randomisation.

use once_cell::sync::OnceCell;

use samase_shim::PluginApi;

use cosmo_plus_core::config::PlusConfig;

/// The plugin api, bit-copied out of the `samase_shim` context so it survives that
/// context's drop (the context installs the step_objects hook on drop).
static API: OnceCell<PluginApi> = OnceCell::new();
/// The parsed `plus.yml`, if present.
static CONFIG: OnceCell<Option<PlusConfig>> = OnceCell::new();

pub(crate) fn set_api(api: PluginApi) {
    let _ = API.set(api);
}

pub(crate) fn api() -> &'static PluginApi {
    API.get().expect("cosmo-plus api not initialized")
}

pub(crate) fn config() -> Option<&'static PlusConfig> {
    CONFIG.get().and_then(|c| c.as_ref())
}

pub(crate) fn install() {
    let cfg = crate::config::load_plus_yml();
    crate::state::feature(
        "data-overrides",
        match cfg.as_ref() {
            Some(c) => format!("armed ({} override(s))", c.overrides.len()),
            None => "disabled (no plus.yml)".to_string(),
        },
    );
    let _ = CONFIG.set(cfg);

    crate::log::log(&format!(
        "[cosmo-plus] feature build: v{} abi={} build={:#010x} rev={}",
        env!("CARGO_PKG_VERSION"),
        crate::cosmo_plus_abi(),
        crate::cosmo_plus_build_id(),
        env!("COSMO_PLUS_GIT_REV"),
    ));

    // Crash self-capture first: vectored exception handler + panic hook.
    unsafe { crate::crash::install() };
    crate::state::feature("crash-capture", "armed");

    // May be too early for the template list (built during engine init); the first game
    // tick calls this again as a fallback.
    crate::antimeta::debug_dump_game_types_once();
    // Debug-only: patch the template loader so the list is dumped right after it is built.
    if !crate::config::env_flag("CM_PLUS_NO_TEMPLATE_HOOK") {
        crate::antimeta::install_game_type_dump_hook();
    }
    // Draft mode (Mechabellum-style): pick-command hook + pool validation.
    crate::draft::install();

    // Bisection kill-switches (diagnostics): each hook can be disabled independently to
    // isolate a boot-time crash/hang. Off by default (everything installs).
    if !crate::config::env_flag("CM_PLUS_NO_FRAME_HOOK") {
        crate::frame::install();
        crate::state::feature("frame-hook", "armed (after StepObjects)");
    } else {
        crate::state::feature("frame-hook", "disarmed: CM_PLUS_NO_FRAME_HOOK");
    }
    if !crate::config::env_flag("CM_PLUS_NO_DRAW_HOOK") {
        crate::overlay::install();
        crate::state::feature(
            "draw-hook-overlay",
            if crate::overlay::enabled() { "armed" } else { "off by config" },
        );
    } else {
        crate::state::feature("draw-hook-overlay", "disarmed: CM_PLUS_NO_DRAW_HOOK");
    }
    if !crate::config::env_flag("CM_PLUS_NO_ZOOM") {
        crate::zoom::install_player_zoom_floor();
        crate::state::feature("zoom-floor", crate::zoom::status());
    } else {
        crate::state::feature("zoom-floor", "disarmed: CM_PLUS_NO_ZOOM");
    }

    // Idle-worker hotkey (F9 by default; see idle_workers.rs). Independent of the
    // overlay switch — it is polled from the draw hook, so it needs that hook.
    crate::idle_workers::install();
    crate::state::feature("idle-workers", crate::idle_workers::status());
    crate::ultraload::install();
    crate::state::feature("ultraload", crate::ultraload::status());
    // Camera hotkey labels (C1-C8 on the minimap): needs the gptp export.
    crate::cam_labels::install();
    crate::state::feature("camera-labels", crate::cam_labels::status());
    // Larva picker (F12 by default; see larva_pick.rs). Needs the draw hook and
    // installs its own keyboard hook for the count entry.
    crate::larva_pick::install();
    crate::state::feature("larva-pick", crate::larva_pick::status());
    // CapsLock + WASD camera pan (see cam_pan.rs). Own keyboard hook.
    crate::cam_pan::install();
    crate::state::feature("cam-pan", crate::cam_pan::status());
    // SC2-style Tab subgroup cycling (see tab_cycle.rs). Polled from the draw hook.
    crate::tab_cycle::install();
    crate::state::feature("tab-cycle", crate::tab_cycle::status());
    // Dev REPL (CM_PLUS_REPL=1 only; inert otherwise).
    crate::repl::install();
    // SC2-style smart casting (CM_PLUS_SMART_CAST=1 only; inert otherwise).
    crate::casting::install();

    // Boot integration report: resolve + verify every fixed global, dump the
    // feature/hook/config map to cosmo_plus.state.json (also reachable from the
    // REPL `state` command).
    unsafe { crate::state::write_report("boot") };
    unsafe { crate::selftest::run() };
}
