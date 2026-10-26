//! Screen-fixed HUD overlays (idle-worker badge, Tab subgroup indicator,
//! C1–C8 camera labels, nydus minimap numbers), drawn through the minimap
//! overlay dispatch (`0x004A3570`).
//!
//! Why this hook: the draw-proc hook (0x004BD68D) runs on the **world
//! surface**, which the fork's zoom resamples (crop + magnify) onto the
//! screen buffer. Anything drawn near the world surface's edges — a HUD
//! badge anchored to the minimap, say — falls outside the zoom crop and
//! never reaches the screen once the player zooms in. The minimap dialog,
//! by contrast, draws on the **screen buffer after the resample** (the UI
//! layer pass), so screen-fixed text drawn there is visible at every zoom.
//!
//! The fork already replaced 0x004A3570 with a jump (`E9 rel32`) to its
//! minimap overlay draw (gptp.qdp). This module reads that jump at install,
//! writes its own, and calls the original target first with the original
//! registers — the same chain pattern as the draw-proc hook. On a build
//! without the fork hook, the vanilla function (same fastcall signature) is
//! chained instead, so the overlays still work.
//!
//! Signature of the chained function (fastcall + two stack args, RET 8):
//! `void __fastcall (bw::Dialog* this /*ecx*/, DialogEvent* event /*edx*/,
//!                   u32 user, void* draw_ctx)`.

use std::sync::atomic::{AtomicU32, Ordering};

/// The minimap overlay root's draw dispatch, replaced by the fork with a jump.
const ADDR_MINIMAP_OVERLAY_DRAW: usize = 0x004A3570;

/// The function the trampoline calls before drawing (fork wrapper or vanilla).
static CHAIN: AtomicU32 = AtomicU32::new(0);

/// Resolved status for the boot report.
pub(crate) fn status() -> String {
    let chain = CHAIN.load(Ordering::SeqCst);
    if chain == 0 {
        "not installed".to_string()
    } else {
        format!("armed (chain 0x{chain:08X})")
    }
}

/// Install the trampoline: chain whatever is at the dispatch site now, then
/// draw the screen-fixed overlays.
pub(crate) fn install() {
    unsafe {
        let site = ADDR_MINIMAP_OVERLAY_DRAW as *const u8;
        let first = std::ptr::read(site);
        let chain = if first == 0xE9 {
            let rel = std::ptr::read(site.add(1) as *const i32);
            (ADDR_MINIMAP_OVERLAY_DRAW as u32).wrapping_add(5).wrapping_add(rel as u32)
        } else {
            ADDR_MINIMAP_OVERLAY_DRAW as u32
        };
        CHAIN.store(chain, Ordering::SeqCst);

        let my = trampoline as *const () as usize as u32;
        let rel = my.wrapping_sub(ADDR_MINIMAP_OVERLAY_DRAW as u32 + 5) as i32;
        let jump: [u8; 5] = [
            0xE9,
            rel as u8,
            (rel >> 8) as u8,
            (rel >> 16) as u8,
            (rel >> 24) as u8,
        ];
        let api = crate::plus::api();
        let r = (api.write_exe_memory)(ADDR_MINIMAP_OVERLAY_DRAW, jump.as_ptr(), jump.len());
        crate::log::log(&format!(
            "[cosmo-plus] hud overlay: hooked minimap dispatch (chain 0x{chain:08X}, patch={r})"
        ));
        crate::state::hook("minimap-overlay-draw", ADDR_MINIMAP_OVERLAY_DRAW, 0, my as usize);
    }
}

/// Naked fastcall trampoline. At entry the game has passed the fastcall
/// registers and two stack args: `[ESP+4] = user`, `[ESP+8] = draw_ctx`
/// (with the game's return address at `[ESP]`). The chained function has
/// exactly that same signature and ends in `RET 8`, so it must be entered
/// with an identical frame — a plain `call` would push an extra return
/// address and shift its arguments. The two args are re-pushed first; the
/// chain's own `RET 8` pops them again, restoring the pre-push state.
#[unsafe(naked)]
unsafe extern "fastcall" fn trampoline() {
    core::arch::naked_asm!(
        // Save everything; the chain needs ECX (this) and EDX (event) intact.
        "pushad",
        // Rebuild the chain's exact frame: push draw_ctx, then user.
        "mov eax, [esp + 40]",
        "push eax",
        "mov eax, [esp + 40]",
        "push eax",
        // Chain first (fork/vanilla minimap draw); its RET 8 pops the two
        // args back off, so ESP lands exactly at the post-pushad state.
        "call dword ptr [{chain}]",
        // Align the stack for the Rust draw call, draw, restore.
        "mov ebx, esp",
        "and esp, 0xFFFFFFF0",
        "sub esp, 16",
        "call {draw}",
        "mov esp, ebx",
        "popad",
        "ret 8",
        chain = sym CHAIN,
        draw = sym draw_overlays,
    );
}

/// Draw every screen-fixed overlay: the small font is selected by each
/// element (they share the same setup dance), and the surface is the game's
/// screen buffer at this point, so coordinates are plain screen pixels.
unsafe extern "C" fn draw_overlays() {
    if crate::frame::frame_count() == 0 {
        return; // not in a game
    }
    let mut buf = [0u8; 16];
    // Minimap-position labels first (they sit on the map panel).
    crate::nydus::draw_minimap(&mut buf);
    crate::cam_labels::draw_minimap(&mut buf);
    // HUD badges: the idle-worker counter above the minimap and the Tab
    // subgroup indicator.
    crate::overlay::draw_idle_counter();
    crate::overlay::draw_tab_indicator();
}
