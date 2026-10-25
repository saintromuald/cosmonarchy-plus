//! In-place configuration reload (the safe half of hot reload): re-read
//! `cosmo_plus.conf` and re-arm every config-driven feature without restarting
//! the game. Code changes still need a rebuild + fast restart (`tools/
//! cosmo-restart.sh`); this covers the much more common config-only iteration.
//!
//! Zoom's memory patch is one-shot (re-applying it would need a re-scan of a
//! possibly already-modified site), so it is reported as "needs restart".

use std::sync::atomic::Ordering;

pub(crate) unsafe fn rearm() {
    // Config is re-read by config::load() (the REPL does that before calling here).

    crate::idle_workers::rearm();
    crate::tab_cycle::rearm();
    crate::larva_pick::rearm();
    crate::overlay::rearm();

    // Zoom: the site was already patched; a changed floor needs a restart.
    crate::log::log(
        "[cosmo-plus] reload: zoom floor changes need a restart (one-shot patch)",
    );

    crate::state::write_report("reload");
    crate::log::log("[cosmo-plus] reload: features re-armed from cosmo_plus.conf");
    let _ = Ordering::SeqCst;
}
