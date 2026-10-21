//! Plugin entry point for the 1.16.1 plugin surface.
//!
//! Mirrors the rollback plugin's bootstrap but with no netcode: load the config,
//! bring up `samase_shim`, then let [`crate::plus`] install its hooks/features.

use samase_shim::PluginApi;

pub(crate) fn initialize() {
    unsafe { samase_shim::on_win_main(startup) }
}

#[no_mangle]
pub extern "C" fn Initialize() {
    initialize();
}

unsafe fn startup() {
    crate::config::load();
    let ctx = samase_shim::init_1161();
    // Copy the api out of the context before it drops (the drop installs the
    // step_objects hook we register below, and frees the context's memory).
    let api: PluginApi = std::ptr::read(ctx.api());
    crate::plus::set_api(api);

    crate::log::log(&format!(
        "[cosmo-plus] loaded v{} abi={} build={:#010x}",
        env!("CARGO_PKG_VERSION"),
        crate::cosmo_plus_abi(),
        crate::cosmo_plus_build_id()
    ));
    crate::plus::install();
}
