//! Nydus numbering: each canal/exit pair gets a number, drawn above the
//! buildings in-world and on the minimap, so the player can tell which canal
//! feeds which exit ("where your units are going").
//!
//! A canal and its exit link each other through `Unit.building.nydus_exit`
//! (UnitLayout.h offset 0x0D0; `make_nydus_exit.cpp` sets `canal->exit` and
//! `exit->canal`). Pairs are numbered in unit-slot order (creation order), so
//! the numbering is deterministic on every peer; both members of a pair show
//! the same number.
//!
//! Drawing happens in the **draw pass** (called from `overlay`'s draw hook
//! after the world is drawn), never during rollback re-simulation. The
//! in-world labels use the same world-minus-camera transform the saturation
//! overlay uses; the minimap labels are screen-fixed, positioned by the
//! engine's minimap rect and the tile -> minimap-px projection.

use std::io::Write;

use bw_dat::structs::Unit;

use cosmo_plus_core::saturation::text_color;

use crate::idle_workers::local_player;
use crate::overlay::{bw_print_xy, camera, get_color_handle};

/// Cesiant (Nydus network) unit ids: the canal plus its morph forms.
/// `UnitId.h`: Nydus=134, Cagrant=143, Spraith=144, Surkith=146, Larvosk=216,
/// Spirtith=292, Skortrith=293, Ocsal=354, Kazitrith=355.
pub const CESIAN_IDS: &[u16] = &[134, 143, 144, 146, 216, 292, 293, 354, 355];

/// `Unit.building.nydus_exit` offset (UnitLayout.h `/*0x0D0*/ Unit* nydus_exit`).
const UNIT_NYDUS_EXIT_OFFSET: usize = 0x0D0;
/// The minimap rect (left/top/right/bottom, screen px).
pub(crate) const G_MINIMAP_RECT: usize = 0x0051_2D00;
/// Map width / height in tiles (`Game.map_size` pair, read by the rollback
/// client's takeover log from the same addresses).
pub(crate) const MAP_SIZE_X: usize = 0x0057_F1D4;
pub(crate) const MAP_SIZE_Y: usize = 0x0057_F1D6;
/// World pixels per tile.
pub(crate) const PX_PER_TILE: i32 = 32;

/// One numbered nydus pair: both members at their world positions.
struct NydusPair {
    number: u32,
    a: (i32, i32),
    b: (i32, i32),
}

unsafe fn nydus_exit_of(unit: *const Unit) -> *const Unit {
    *(unit as *const u8).add(UNIT_NYDUS_EXIT_OFFSET) as *const Unit
}

unsafe fn is_cesiant(unit: *const Unit) -> bool {
    CESIAN_IDS.contains(&(*unit).unit_id)
}

/// Collect the canal/exit pairs for the local player, numbered in unit-slot
/// order. A pair is counted once (when visiting the member with the lower
/// unit address, i.e. the one created first).
unsafe fn collect_pairs(local: u32) -> Vec<NydusPair> {
    let mut pairs: Vec<(*const Unit, *const Unit)> = Vec::new();
    crate::frame::for_each_unit(|u| {
        if (*u).player as u32 != local || !is_cesiant(u) {
            return;
        }
        let exit = nydus_exit_of(u);
        if exit.is_null() || exit == u {
            return;
        }
        if (u as usize) < (exit as usize) && pairs.iter().all(|&(a, _b)| a != u && a != exit) {
            pairs.push((u, exit));
        }
    });
    pairs
        .into_iter()
        .enumerate()
        .map(|(i, (x, y))| NydusPair {
            number: i as u32 + 1,
            a: world_pos(x),
            b: world_pos(y),
        })
        .collect()
}

unsafe fn world_pos(unit: *const Unit) -> (i32, i32) {
    let p = (*unit).flingy.position;
    (p.x as i32, p.y as i32)
}

/// Draw the in-world numbers (called from the overlay draw hook, after the
/// world is drawn; `buf` is the shared scratch label buffer).
pub(crate) unsafe fn draw_world(buf: &mut [u8; 16]) {
    let Some(local) = local_player() else {
        return;
    };
    let pairs = collect_pairs(local);
    if pairs.is_empty() {
        return;
    }
    let (cx, cy) = camera();
    get_color_handle(text_color::WHITE);
    for p in &pairs {
        for &(x, y) in &[p.a, p.b] {
            buf.fill(0);
            let label = format!("nydus {}", p.number);
            let _ = write!(&mut buf[..], "{label}");
            bw_print_xy(buf.as_ptr(), x - cx - 6, y - cy - 30);
        }
    }
}

/// Draw the same numbers onto the minimap. Called from hud.rs through the
/// minimap dispatch — on the **screen buffer** after the zoom resample — so
/// the minimap rect is in plain screen coordinates and the labels are
/// screen-fixed (the old world-surface version added the camera, which
/// pushed the labels out of view).
pub(crate) unsafe fn draw_minimap(buf: &mut [u8; 16]) {
    let Some(local) = local_player() else {
        return;
    };
    let pairs = collect_pairs(local);
    if pairs.is_empty() {
        return;
    }
    let rect = *(G_MINIMAP_RECT as *const [i32; 4]);
    let (left, top, right, bottom) = (rect[0], rect[1], rect[2], rect[3]);
    let map_w = *(MAP_SIZE_X as *const u16) as i32;
    let map_h = *(MAP_SIZE_Y as *const u16) as i32;
    if map_w <= 0 || map_h <= 0 || right <= left || bottom <= top {
        return;
    }
    get_color_handle(text_color::WHITE);
    for p in &pairs {
        for &(x, y) in &[p.a, p.b] {
            let mx = left + x / PX_PER_TILE * (right - left) / map_w;
            let my = top + y / PX_PER_TILE * (bottom - top) / map_h;
            buf.fill(0);
            let label = format!("{}", p.number);
            let _ = write!(&mut buf[..], "{label}");
            // Centred on the point, screen-fixed via the zoom mapping.
            let (sx, sy) = crate::hud::screen_to_surface(mx - 2, my - 4);
            bw_print_xy(buf.as_ptr(), sx, sy);
        }
    }
}
