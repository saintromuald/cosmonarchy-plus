//! Engine selection call shared by the hotkey features (idle workers, larva pick).

use bw_dat::structs::Unit;

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
