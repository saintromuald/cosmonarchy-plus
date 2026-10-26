//! SC2-style smart casting: the engine adapter for the pure model in
//! `cosmo-plus-core::casting`.
//!
//! Brood War's native behaviour is cast-all (SC1-style): a spell ordered from
//! the command card is broadcast to every selected caster. The plugin narrows
//! it: when a smart-cast spell is ordered with several casters selected, the
//! command is intercepted (the game's command-processing hook) and re-issued
//! for the single closest caster through the engine's own per-unit order call
//! (`issue_order_targeting_ground`, 0x00475260 — the same call the game uses
//! for unit-targeted orders), so the rest of the group never wastes energy.
//!
//! Classification: `plus.yml` `cast-modes:` pins a unit type to `smart` or
//! `all`; otherwise the SC2 default applies (targeted spells smart, stim-like
//! orders all). Holding Shift while casting forces all-at-once.
//!
//! Gated by `CM_PLUS_SMART_CAST=1` until it is verified live (the command
//! layout is version-sensitive); `CM_PLUS_CAST_DEBUG=1` dumps every command
//! the hook sees, which is how the layout was derived.

use std::sync::atomic::{AtomicBool, Ordering};

use cosmo_plus_core::casting::{pick_caster, resolve_mode, CastMode, Caster};

use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

/// The BW 1.16.1 local `Command` layout (type/order/pos/unit), as processed by
/// the engine's command handler:
///   0x00 u8  type (0x0E right click, 0x1B normal click, ...)
///   0x01 u8  order id
///   0x02 u16 unit id (build commands)
///   0x04 u16 target x (world px)
///   0x06 u16 target y
///   0x08 u32 target Unit* (0 = ground target)
///   0x0C u16 fow target
const CMD_OFF_ORDER: usize = 0x01;
const CMD_OFF_X: usize = 0x04;
const CMD_OFF_Y: usize = 0x06;
const CMD_OFF_UNIT: usize = 0x08;

/// `helper::issue_order_targeting_ground(Unit* esi, u32 order_id, u32 x, u32 y)`
/// at 0x00475260 — orders ONE unit to cast/act at a position.
const ADDR_ISSUE_ORDER_GROUND: usize = 0x00475260;
static ISSUE_ORDER_GROUND_ADDR: u32 = ADDR_ISSUE_ORDER_GROUND as u32;

static ENABLED: AtomicBool = AtomicBool::new(false);

pub(crate) fn install() {
    ENABLED.store(
        crate::config::env_flag("CM_PLUS_SMART_CAST"),
        Ordering::SeqCst,
    );
    if !ENABLED.load(Ordering::SeqCst) {
        crate::log::log(
            "[cosmo-plus] smart cast: disabled (set CM_PLUS_SMART_CAST=1 to enable; \
             SC2 semantics, Shift forces all-at-once)",
        );
        return;
    }
    let api = crate::plus::api();
    let r = unsafe { (api.hook_process_commands)(process_hook) };
    crate::log::log(&format!(
        "[cosmo-plus] smart cast: command hook installed (result={r})"
    ));
}

/// The per-unit cast-mode override from `plus.yml` `cast-modes:` (keyed by
/// unit id as a decimal string, or by unit name).
fn configured_mode(unit_id: u16) -> Option<CastMode> {
    let cfg = crate::plus::config()?;
    cfg.cast_modes.get(&unit_id.to_string()).copied()
}

/// The command-processing hook: runs before the game applies the command to
/// the selection. Not calling `orig` blocks the broadcast.
unsafe extern "C" fn process_hook(
    command: *const core::ffi::c_void,
    arg1: u32,
    arg2: u32,
    orig: unsafe extern "C" fn(*const core::ffi::c_void, u32, u32),
) {
    if command.is_null() {
        orig(command, arg1, arg2);
        return;
    }
    let order = std::ptr::read((command as *const u8).add(CMD_OFF_ORDER));
    let x = std::ptr::read((command as *const u16).add(CMD_OFF_X / 2)) as i32;
    let y = std::ptr::read((command as *const u16).add(CMD_OFF_Y / 2)) as i32;
    let target_unit = std::ptr::read((command as *const u32).add(CMD_OFF_UNIT / 4));

    if crate::config::env_flag("CM_PLUS_CAST_DEBUG") {
        let head = core::slice::from_raw_parts(command as *const u8, 16);
        crate::log::log(&format!(
            "[cosmo-plus] cast debug: order=0x{order:02X} pos=({x},{y}) unit=0x{target_unit:08X} \
             args=({arg1},{arg2}) head={}",
            head.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join("")
        ));
    }

    // Unit-targeted commands (clicked on a unit) already apply to one thing:
    // never rewrite those.
    if target_unit != 0 {
        orig(command, arg1, arg2);
        return;
    }

    let Some((_ids, ptrs)) = crate::tab_cycle::selection_ptrs() else {
        orig(command, arg1, arg2);
        return;
    };
    if ptrs.len() < 2 {
        orig(command, arg1, arg2);
        return;
    }
    let Some(local) = crate::idle_workers::local_player() else {
        orig(command, arg1, arg2);
        return;
    };

    // The command card belongs to the selection's first unit; the candidates
    // are the selected units of that type owned by the local player.
    let first = ptrs[0] as *mut bw_dat::structs::Unit;
    let ty = (*first).unit_id;
    let mut casters: Vec<Caster> = Vec::new();
    for &p in &ptrs {
        let unit = &*(p as *mut bw_dat::structs::Unit);
        if unit.player as u32 != local || unit.unit_id != ty {
            continue;
        }
        let pos = &unit.flingy.position;
        casters.push(Caster {
            id: p,
            x: pos.x as i32,
            y: pos.y as i32,
            eligible: true, // energy filtering is a follow-up (needs the energy field)
        });
    }
    if casters.len() < 2 {
        orig(command, arg1, arg2);
        return;
    }

    let shift_held = (GetAsyncKeyState(0x10) as u16 & 0x8000) != 0;
    let mode = resolve_mode(order, ty, configured_mode(ty).as_ref(), shift_held);
    if mode == CastMode::All {
        orig(command, arg1, arg2);
        return;
    }

    // Smart cast: one caster acts. Block the broadcast and issue the order for
    // the single chosen caster through the engine's per-unit order call.
    let Some(chosen) = pick_caster(&casters, x, y) else {
        orig(command, arg1, arg2);
        return;
    };
    crate::log::log(&format!(
        "[cosmo-plus] smart cast: narrowed order 0x{order:02X} to caster 0x{chosen:08X}"
    ));
    issue_order_ground(chosen as *mut bw_dat::structs::Unit, order as u32, x as u32, y as u32);
}

unsafe fn issue_order_ground(
    unit: *mut bw_dat::structs::Unit,
    order: u32,
    x: u32,
    y: u32,
) {
    core::arch::asm!(
        "pushad",
        "push {y}",
        "push {x}",
        "push {order}",
        "mov esi, {unit}",
        "call dword ptr [{func}]",
        "popad",
        unit = in(reg) unit,
        order = in(reg) order,
        x = in(reg) x,
        y = in(reg) y,
        func = sym ISSUE_ORDER_GROUND_ADDR,
        options(nostack),
    );
}
