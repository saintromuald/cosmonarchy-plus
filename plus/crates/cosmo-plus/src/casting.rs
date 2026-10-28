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
///   0x02 u16 unit id (build/morph payload)
///   0x04 u16 target x (world px)
///   0x06 u16 target y
///   0x08 u32 target Unit* (0 = ground target)
///   0x0C u16 fow target
const CMD_OFF_TYPE: usize = 0x00;
const CMD_OFF_ORDER: usize = 0x01;
const CMD_OFF_PAYLOAD: usize = 0x02;
const CMD_OFF_X: usize = 0x04;
const CMD_OFF_Y: usize = 0x06;
const CMD_OFF_UNIT: usize = 0x08;
/// Command type 0x0E = right-click (a ground-targeted order).
const CMD_TYPE_RIGHT_CLICK: u8 = 0x0E;

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
    // The command hook is needed by two features: smart casting (gated) and
    // the tab cycle's subgroup-scoped ability execution (active whenever the
    // tab cycle is armed — SC2 parity: abilities issued on a mixed selection
    // act on one subgroup, not the whole group).
    let tab_armed = !crate::config::env_flag("CM_PLUS_NO_TAB_CYCLE")
        && crate::tab_cycle::fwd_vk() != 0;
    if !ENABLED.load(Ordering::SeqCst) && !tab_armed {
        crate::log::log(
            "[cosmo-plus] command hook: not installed (smart cast off, tab cycle off)",
        );
        return;
    }
    let api = crate::plus::api();
    let r = unsafe { (api.hook_process_commands)(process_hook) };
    crate::log::log(&format!(
        "[cosmo-plus] command hook: installed (result={r}; smart cast {}, tab narrowing {})",
        if ENABLED.load(Ordering::SeqCst) { "on" } else { "off" },
        if tab_armed { "on" } else { "off" },
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
    // An empty turn carries no commands: nothing to inspect or rewrite. The pointer may
    // even be dangling (Rust's 0x1 for an empty Vec), so it must never be dereferenced —
    // this check runs *before* any read of the command head. (The rollback client once
    // passed an empty turn's dangling pointer here and faulted at address 1.)
    if command.is_null() || arg1 == 0 {
        orig(command, arg1, arg2);
        return;
    }
    let cmd_type = std::ptr::read((command as *const u8).add(CMD_OFF_TYPE));
    let order = std::ptr::read((command as *const u8).add(CMD_OFF_ORDER));
    // Build/morph commands carry the structure/unit id as a u16 payload at
    // offset 0x02 (CMDRECV_Build.cpp → cmdrecv_place_building_normal(x, y,
    // order_id, built_unit_id)). Narrowing would drop the payload and break
    // construction, so every payload command broadcasts untouched.
    let payload = std::ptr::read((command as *const u16).add(CMD_OFF_PAYLOAD / 2));
    let x = std::ptr::read((command as *const u16).add(CMD_OFF_X / 2)) as i32;
    let y = std::ptr::read((command as *const u16).add(CMD_OFF_Y / 2)) as i32;
    let target_unit = std::ptr::read((command as *const u32).add(CMD_OFF_UNIT / 4));
    // arg2 != 0: the command is being replayed (samase's ProcessCommands
    // hook passes a replay flag). Smart casting reads the VIEWER's keyboard
    // (Shift) and must never run then; subgroup narrowing is deterministic
    // and safe on replays.
    let replay = arg2 != 0;

    if crate::config::env_flag("CM_PLUS_CAST_DEBUG") {
        let head = core::slice::from_raw_parts(command as *const u8, 16);
        crate::log::log(&format!(
            "[cosmo-plus] cast debug: order=0x{order:02X} pos=({x},{y}) unit=0x{target_unit:08X} \
             args=({arg1},{arg2}) head={}",
            head.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join("")
        ));
    }

    // Unit-targeted commands (clicked on a unit) already apply to one thing:
    // never rewrite those. Payload commands (build/morph with a structure id)
    // must broadcast: narrowing would drop the payload.
    if target_unit != 0 || payload != 0 {
        orig(command, arg1, arg2);
        return;
    }

    // Subgroup-scoped ability execution (the tab cycle's SC2 parity rule):
    // an ability-class order on a mixed selection acts on ONE subgroup — the
    // first cycle-worthy run of the CANONICAL selection whose card carries
    // the order (peer-symmetric: the canonical table is synced through select
    // commands and is never rotated by us, and the buttonset table is the
    // same on every peer — see tab_cycle::narrow_targets). This is what stops
    // "Tab to lurkers, burrow" from burrowing the zerglings too. Generic
    // orders (move/attack/stop/harvest/build/train) keep broadcasting,
    // exactly like SC2.
    if let Some(targets) = crate::tab_cycle::narrow_targets(order) {
        // Command type 0x0E = right-click (ground-targeted); other types are
        // button/hotkey presses (non-targeted abilities like burrow/siege).
        let ground_targeted = cmd_type == CMD_TYPE_RIGHT_CLICK;
        for &t in &targets {
            let unit = t as *mut bw_dat::structs::Unit;
            if ground_targeted {
                issue_order_ground(unit, order as u32, x as u32, y as u32);
            } else {
                issue_order_nothing(unit, order);
            }
        }
        if crate::config::env_flag("CM_PLUS_CAST_DEBUG") {
            crate::log::log(&format!(
                "[cosmo-plus] cast debug: narrowed order 0x{order:02X} to {} unit(s) of the \
                 active subgroup{}",
                targets.len(),
                if ground_targeted { " (ground-targeted)" } else { " (non-targeted)" },
            ));
        }
        return; // broadcast blocked
    }

    if !ENABLED.load(Ordering::SeqCst) || replay {
        orig(command, arg1, arg2);
        return;
    }

    // Smart-cast candidate derivation reads the CANONICAL selection (synced,
    // never rotated by us) so both peers pick the same caster — the rotated
    // client table was viewer-local and could desync.
    let Some((_ids, ptrs)) = crate::tab_cycle::read_canonical_selection() else {
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

/// `helper::issue_order_targeting_nothing(Unit* esi, u8 order_id)` at
/// 0x00475310 — orders ONE unit to run a non-targeted order (burrow/siege/
/// cloak/morph). Mirrors the mod's bwhelper.cpp:1152 wrapper.
const ADDR_ISSUE_ORDER_NOTHING: usize = 0x00475310;
static ISSUE_ORDER_NOTHING_ADDR: u32 = ADDR_ISSUE_ORDER_NOTHING as u32;

unsafe fn issue_order_nothing(unit: *mut bw_dat::structs::Unit, order: u8) {
    core::arch::asm!(
        "pushad",
        "mov esi, {unit}",
        "movzx ecx, {order}",
        "call dword ptr [{func}]",
        "popad",
        unit = in(reg) unit,
        order = in(reg_byte) order,
        func = sym ISSUE_ORDER_NOTHING_ADDR,
        options(nostack),
    );
}
