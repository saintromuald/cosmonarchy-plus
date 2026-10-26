//! Engine selection call shared by the hotkey features (idle workers, larva pick).

use bw_dat::structs::Unit;

/// The game's own "center on this unit" routine (0x004E6020) — the path the
/// portrait double-click and Ctrl+C use. It reads the unit's sprite position,
/// computes the zoom-aware centered camera (tile-aligned, minus half the
/// viewport) and calls move_screen itself. ABI: `eax = Unit*`, no stack args.
/// This is the unit-specific centering — move_screen alone takes a camera
/// position, which is why raw unit positions landed the view elsewhere.
const ADDR_MOVE_SCREEN_TO_UNIT: usize = 0x004E6020;
static MOVE_SCREEN_TO_UNIT_ADDR: u32 = ADDR_MOVE_SCREEN_TO_UNIT as u32;

/// Center the camera on a unit through the engine's own routine (zoom-aware
/// in the widescreen build).
pub(crate) unsafe fn move_screen_to_unit(unit: *mut Unit) {
    core::arch::asm!(
        "pushad",
        "mov eax, {unit}",
        "call dword ptr [{func}]",
        "popad",
        unit = in(reg) unit,
        func = sym MOVE_SCREEN_TO_UNIT_ADDR,
        options(nostack),
    );
}

/// `helper::move_screen` (0x0049C440) — centers the view on world-px (x, y),
/// clamped by the engine's scroll limits. Register ABI from `bwhelper.cpp`:
/// `movzx eax, x; movzx ecx, y; call`.
const ADDR_MOVE_SCREEN: usize = 0x0049C440;

/// Center the camera on a world-pixel position (the engine clamps to the map).
pub(crate) unsafe fn move_screen(x: u16, y: u16) {
    core::arch::asm!(
        "pushad",
        "movzx eax, {x:x}",
        "movzx ecx, {y:x}",
        "call dword ptr [{func}]",
        "popad",
        x = in(reg) x,
        y = in(reg) y,
        func = sym ADDR_MOVE_SCREEN_ADDR,
        options(nostack),
    );
}
static ADDR_MOVE_SCREEN_ADDR: u32 = ADDR_MOVE_SCREEN as u32;

/// The engine's click-selection routine (BW/bwhelper.cpp `select_units_if_alt_not_held`).
const ADDR_SELECT_UNITS: usize = 0x0046FA00;

/// `helper::select_units_if_alt_not_held` (0x0046FA00): EBX = unit list, EDI = length,
/// [esp] = play_sound, [esp+4] = send_command (callee cleans the two stack args).
///
/// The call target is referenced through a static (`call dword ptr [sym]`), and the
/// register args use named registers — a plain `in(reg)` operand could be allocated to
/// EDI/EBX and then be clobbered by the other operand's load (this exact bug shipped
/// once and turned the call into `call <len>`).
static SELECT_UNITS_ADDR: u32 = ADDR_SELECT_UNITS as u32;

/// Replace the selection with `list` through the engine's own routine, playing the
/// selection sound and sending the synced select command (multiplayer/rollback-safe).
pub(crate) unsafe fn select_units_if_alt_not_held(
    list: *const *mut Unit,
    len: u32,
    play_sound: u32,
    send_command: u32,
) {
    core::arch::asm!(
        "pushad",
        "push {send}",
        "push {sound}",
        "call dword ptr [{func}]",
        "popad",
        in("ebx") list as u32,
        in("edi") len,
        sound = in(reg) play_sound,
        send = in(reg) send_command,
        func = sym SELECT_UNITS_ADDR,
        options(nostack),
    );
}
