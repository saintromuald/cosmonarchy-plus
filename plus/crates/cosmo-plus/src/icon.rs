//! Engine icon blit for HUD badges (the idle-worker counter, SC2-style).
//!
//! Uses the engine's own loaded command-icon GRP and pixel blit, so the icons
//! match the command card exactly and the palette is handled by the engine.
//! Addresses and calling convention from the gptp fork's `bwhelper.cpp`
//! (`bw_print_icon`) and `bwdata.h` (`g_cmdicons_grp`).

/// `g_cmdicons_grp` (GrpHead*): the loaded command-card icons GRP
/// (`unit\cmdbtns\cmdicons.grp`). The unit dat's icon field (0x43) indexes it.
const G_CMDICONS_GRP: usize = 0x0068_C1E0;
/// `helper::bw_print_icon(Bounds* eax, FxnDrawIconData* edi, int pos_x, int pos_y)`.
const BW_PRINT_ICON: usize = 0x0041_D090;
static PRINT_ICON_ADDR: u32 = BW_PRINT_ICON as u32;

#[repr(C)]
struct GrpFrame {
    x: u8,
    y: u8,
    w: u8,
    h: u8,
    data_offset: u32,
}

#[repr(C)]
struct GrpHead {
    frame_count: u16,
    width: i16,
    height: i16,
    // GrpFrame frames[] follows (8 bytes each).
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Bounds {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    width: i32,
    height: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct FxnDrawIconData {
    w: u16,
    h: u16,
    pixel_data: *const u8,
}

/// Blit the command-card icon `icon_id` at screen position (x, y). Returns the
/// frame size, or None when the icons GRP is not loaded yet.
pub(crate) unsafe fn draw_unit_icon(icon_id: u32, x: i32, y: i32) -> Option<(i32, i32)> {
    let grp = *(G_CMDICONS_GRP as *const *const GrpHead);
    if grp.is_null() {
        return None;
    }
    static FIRST: std::sync::Once = std::sync::Once::new();
    FIRST.call_once(|| {
        crate::log::log(&format!(
            "[cosmo-plus] icon: grp={grp:p} frames={} at x={x} y={y}",
            (*grp).frame_count
        ));
    });
    let count = (*grp).frame_count as usize;
    let idx = if (icon_id as usize) < count {
        icon_id as usize
    } else {
        0
    };
    let frames = (grp as *const u8).add(6) as *const GrpFrame;
    let f = &*frames.add(idx);
    let mut w = f.w as i32;
    let h = f.h as i32;
    let mut offset = f.data_offset;
    // Loaded-GRP quirk (same as gptp's statres hook): a set high bit means the
    // offset was folded into a wider row; clear it and widen the frame.
    if offset & 0x8000_0000 != 0 {
        offset &= 0x7FFF_FFFF;
        w += 256;
    }
    static FRAME: std::sync::Once = std::sync::Once::new();
    FRAME.call_once(|| {
        crate::log::log(&format!(
            "[cosmo-plus] icon: id={icon_id} idx={idx} w={w} h={h} off={offset:#x}"
        ));
    });
    let bounds = Bounds {
        left: 0,
        top: 0,
        right: w,
        bottom: h,
        width: w,
        height: h,
    };
    let data = FxnDrawIconData {
        w: w as u16,
        h: h as u16,
        pixel_data: (grp as *const u8).add(offset as usize),
    };
    core::arch::asm!(
        "pushad",
        "push {y}",
        "push {x}",
        "call dword ptr [{func}]",
        "popad",
        x = in(reg) x,
        y = in(reg) y,
        in("eax") &bounds as *const Bounds as u32,
        in("edi") &data as *const FxnDrawIconData as u32,
        func = sym PRINT_ICON_ADDR,
        options(nostack),
    );
    Some((w, h))
}
