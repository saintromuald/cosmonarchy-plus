//! Anti-meta mode engine side: apply one symmetric start roll per game.
//!
//! Runs once, at the first game tick (from `frame::FIRST_TICK`). It draws the shared,
//! synced RNG seed through the plugin api, derives the roll with
//! [`cosmo_plus_core::antimeta::roll`], then for every active player (Human or Computer,
//! with a completed town hall) adjusts the starting worker count to the rolled value and
//! overwrites the mineral/gas counters. Everything is deterministic in fixed player order
//! (0..8) and stable unit order, so two peers compute the same result without a round trip.
//!
//! Units are created/removed through the mod's own 1.16.1 helpers, wrapped with the exact
//! `PUSHAD`/`POPAD` + custom-register conventions the C++ wrapper uses (`BW/bwhelper.cpp`):
//!   * `create_unit`       0x004A09D0 — unit id in CX, x in EAX, then `PUSH y`, `PUSH player`
//!   * `update_unit_stats_and_finish_building` 0x004A01F0 — EAX = unit
//!   * `show_finished_unit` 0x0049EC30 — ESI = unit, returns b32 in EAX
//!   * `update_unit_strength` 0x0049FA40 — EAX = unit (applies default orders: harvesting)
//!   * `bwfunc_remove`      0x00475710 — EAX = unit (after setting `UserActionFlags_SilentRemove`)
//!
//! No netcode here: this is a data/rules feature only.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use bw_dat::structs::Unit;

use cosmo_plus_core::antimeta::{self, Roll};

// ---------------------------------------------------------------------------
// Fixed 1.16.1 addresses (BW/bwdata.h + bwhelper.h, cross-checked against the mod).
// ---------------------------------------------------------------------------
const ADDR_CREATE_UNIT: usize = 0x004A09D0;
const ADDR_FINISH_UNIT_PRE: usize = 0x004A01F0; // update_unit_stats_and_finish_building
const ADDR_SHOW_FINISHED_UNIT: usize = 0x0049EC30;
const ADDR_FINISH_UNIT_POST: usize = 0x0049FA40; // update_unit_strength
const ADDR_BWFUNC_REMOVE: usize = 0x00475710;

/// `g_game_minerals` / `g_total_minerals` / `g_game_vespene` / `g_total_vespene` —
/// each an array of 12 `s32` (BW/bwdata.h:521-524).
const ADDR_GAME_MINERALS: usize = 0x0057F0F0;
const ADDR_TOTAL_MINERALS: usize = 0x0057F180;
const ADDR_GAME_VESPENE: usize = 0x0057F120;
const ADDR_TOTAL_VESPENE: usize = 0x0057F150;

/// Player table (12 × 0x24): `type` at 0x08, `race` at 0x09 (`BW/structures/Player.h`).
const ADDR_PLAYER_TABLE: usize = 0x0057EEE0;
const PLAYER_STRUCT_SIZE: usize = 0x24;
const PLAYER_TYPE_OFFSET: usize = 0x08;
const PLAYER_RACE_OFFSET: usize = 0x09;

/// `UserActionFlags_SilentRemove` (BW/enumerations.h:1432). Lives in `unit->user_action_flags`
/// (offset 0x93), which `bw_dat` exposes as `Unit::order_flags`.
const SILENT_REMOVE_BIT: u8 = 0x04;

/// `PlayerType::Computer` / `PlayerType::Human` (BW/enumerations.h).
const PLAYER_TYPE_COMPUTER: u8 = 1;
const PLAYER_TYPE_HUMAN: u8 = 2;

/// Number of playable player slots (`PlayerLimit`, BW/structures.h:25).
const PLAYER_LIMIT: usize = 8;

/// Head of the engine's game type list (`g_game_type_list`, BW/bwdata.h:1479). Nodes are
/// `GameTypeList`: `prev` (0), `next` (4), `name[64]` (8), `game_type` u8 (0x48),
/// `unused` u8 (0x49), `sub_type` u16 (0x4A). Built by the template loader at startup.
const ADDR_GAME_TYPE_LIST: usize = 0x0051A270;

/// One-shot debug dump of `g_game_type_list` for `CM_PLUS_DUMP_GAME_TYPES=1`. Prints every
/// template the engine actually loaded (name + type + subtype) to `cosmo_plus.log`; used to
/// verify that a newly added `.got` template made it into the lobby list.
pub(crate) fn debug_dump_game_types_once() {
    static TRIES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !crate::config::env_flag("CM_PLUS_DUMP_GAME_TYPES") || DONE.load(std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    // The template list is built during startup; retry across the first ~600 draws.
    if TRIES.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 600 {
        crate::log::log("[cosmo-plus] game types: dump gave up (list never became non-empty)");
        DONE.store(true, std::sync::atomic::Ordering::SeqCst);
        return;
    }
    let Some(node) = read_game_type_list_head() else {
        return;
    };
    DONE.store(true, std::sync::atomic::Ordering::SeqCst);
    dump_game_type_list();
}

fn read_game_type_list_head() -> Option<usize> {
    let head = unsafe { *(ADDR_GAME_TYPE_LIST as *const usize) };
    // Null in this list means 0 or a sign-extended negative (the engine checks `jle`).
    (head >= 0x400000 && head < 0x8000_0000).then_some(head)
}

/// Walk `g_game_type_list` and log every entry. Shared by the once-guards and the loader
/// hook (which fires immediately after the list is built).
pub(crate) fn dump_game_type_list() {
    let Some(mut n) = read_game_type_list_head() else {
        crate::log::log("[cosmo-plus] game types: list head null/empty");
        return;
    };
    let mut guard = 0;
    while n != 0 && guard < 64 {
        let name = unsafe { read_c_str((n + 8) as *const u8, 63) };
        let gt = unsafe { *((n + 0x48) as *const u8) };
        let st = unsafe { *((n + 0x4A) as *const u16) };
        crate::log::log(&format!(
            "[cosmo-plus] game type list: name={name:?} type={gt} subtype={st}"
        ));
        let next = unsafe { *((n + 4) as *const usize) };
        if next == 0 || next > 0x8000_0000 {
            break;
        }
        n = next;
        guard += 1;
    }
    crate::log::log("[cosmo-plus] game types: dump complete");
}

// ---------------------------------------------------------------------------
// Template-loader hook (CM_PLUS_DUMP_GAME_TYPES only, debug instrumentation).
//
// The engine builds `g_game_type_list` once at startup with
//   push 0x4AB150          ; list-insert callback          (0x004BAC89)
//   call 0x4AB860          ; enumerate templates.lst + load each .got
// We replace the 5-byte `call` with a call to our naked wrapper, which runs the original
// loader and then dumps the freshly built list to the log. This fires before any dialog
// opens, so the dump shows exactly what the lobby dropdown will contain.
// ---------------------------------------------------------------------------

/// Addresses of the `call 0x4AB860` instructions to patch. The loader is reached from
/// two sites: the dialog-init path (0x004DB046) and the one-shot startup path
/// (0x004BAC8E, whose preceding instruction at 0x004BAC89 is `push 0x4AB150`). Hooking
/// both guarantees the dump fires at whichever path initialises the list first.
const ADDR_TEMPLATE_LOAD_CALLS: [usize; 2] = [0x004DB046, 0x004BAC8E];
/// The original callee (0x4AB860, `ret` convention: the caller pops the pushed callback).
static ORIG_TEMPLATE_LOADER: u32 = 0x004AB860;

/// Called by the naked wrapper below; dumps the list after the loader returns.
/// `#[no_mangle]` so the global-asm wrapper can call it directly by symbol
/// (`_cosmo_plus_after_template_load` under the MSVC naming convention).
#[no_mangle]
unsafe extern "C" fn cosmo_plus_after_template_load() {
    dump_game_type_list();
}

core::arch::global_asm!(
    ".global _cosmo_plus_template_load_dump_wrapper",
    "_cosmo_plus_template_load_dump_wrapper:",
    "pushad",
    "call dword ptr [_cosmo_plus_orig_template_loader]",
    "popad",
    "call _cosmo_plus_after_template_load",
    "ret",
    ".globl _cosmo_plus_orig_template_loader",
    ".set _cosmo_plus_orig_template_loader, {orig}",
    orig = const ORIG_TEMPLATE_LOADER,
);

unsafe extern "C" {
    fn cosmo_plus_template_load_dump_wrapper();
}

/// Patch the two `call 0x4AB860` sites to run the dump wrapper after the loader
/// (only when the dump flag is armed).
pub(crate) fn install_game_type_dump_hook() {
    if !crate::config::env_flag("CM_PLUS_DUMP_GAME_TYPES") {
        return;
    }
    let api = crate::plus::api();
    let wrapper = cosmo_plus_template_load_dump_wrapper as usize;
    for &site in &ADDR_TEMPLATE_LOAD_CALLS {
        let first = unsafe { *(site as *const u8) };
        if first != 0xE8 {
            crate::log::log(&format!(
                "[cosmo-plus] game types: call site 0x{site:08X} not as expected (0x{first:02X}); skipped"
            ));
            continue;
        }
        let rel = (wrapper as i64 - (site as i64 + 5)) as i32;
        let mut insn = [0u8; 5];
        insn[0] = 0xE8;
        insn[1..5].copy_from_slice(&rel.to_le_bytes());
        let r = unsafe { (api.write_exe_memory)(site, insn.as_ptr(), 5) };
        crate::log::log(&format!(
            "[cosmo-plus] game types: hooked template loader at 0x{site:08X} (wrapper 0x{wrapper:08X}, patch={r})"
        ));
    }
}

unsafe fn read_c_str(ptr: *const u8, max: usize) -> String {
    let mut s = Vec::with_capacity(max);
    for i in 0..max {
        let c = *ptr.add(i);
        if c == 0 {
            break;
        }
        s.push(c);
    }
    String::from_utf8_lossy(&s).into_owned()
}

/// `*g_game_type` (BW/bwdata.h:221). Carries the lobby template's game-type byte into the
/// match (engine copies it from the selected `templates/*.got` entry at 0x00449882 and it is
/// part of the synced game data, so every peer reads the same value).
const ADDR_GAME_TYPE: usize = 0x00596820;

/// Game type byte of the shipped `Anti Meta(1).got` lobby template. Distinct from
/// `GameType::Unranked` (30) on purpose: the mode must be **selectable in the lobby**, not
/// silently folded into Unranked. 45 is taken (Mindshare), so Plus uses 46.
pub(crate) const GAME_TYPE_ANTI_META: u8 = 46;

/// Guarantees the mode is applied once per **game** (not once per process: a player can
/// return to the menu and start another match without relaunching).
static LAST_GAME: AtomicUsize = AtomicUsize::new(0);
static APPLIED_GAME: AtomicUsize = AtomicUsize::new(0);
static LAST_FRAME: AtomicU32 = AtomicU32::new(0);

/// Per-frame entry point. Applies the roll on the first frames of each new game and does
/// nothing afterwards. Replays are never modified (that would corrupt playback), and a
/// plugin that somehow finds itself mid-game does not touch the running economy.
pub(crate) unsafe fn on_game_tick() {
    let api = crate::plus::api();
    let game = match (api.game)() {
        Some(f) => f() as usize,
        None => return,
    };
    if game == 0 {
        return;
    }
    let frame = (*(game as *const bw_dat::structs::Game)).frame_count;
    let last_frame = LAST_FRAME.swap(frame, Ordering::SeqCst);
    if LAST_GAME.swap(game, Ordering::SeqCst) != game || frame < last_frame {
        APPLIED_GAME.store(0, Ordering::SeqCst);
    }
    if APPLIED_GAME.load(Ordering::SeqCst) == game {
        return;
    }
    if frame > 4 {
        // Loaded mid-game (or a state we did not start): never rewrite a live economy.
        APPLIED_GAME.store(game, Ordering::SeqCst);
        return;
    }
    if is_replay() {
        crate::log::log("[cosmo-plus] anti-meta: replay detected -> skipped");
        APPLIED_GAME.store(game, Ordering::SeqCst);
        return;
    }
    if !enabled() {
        APPLIED_GAME.store(game, Ordering::SeqCst);
        return;
    }
    apply_roll();
    APPLIED_GAME.store(game, Ordering::SeqCst);
}

/// `g_game_data.is_replay` (0x005967F8 + 0x32).
fn is_replay() -> bool {
    const G_GAME_DATA: usize = 0x005967F8;
    const IS_REPLAY_OFFSET: usize = 0x32;
    unsafe { *( (G_GAME_DATA + IS_REPLAY_OFFSET) as *const u8 ) != 0 }
}

/// Apply the anti-meta roll once, if the mode is enabled.
unsafe fn apply_roll() {
    let api = crate::plus::api();
    let seed = match (api.rng_seed)() {
        Some(f) => f(),
        None => {
            crate::log::log("[cosmo-plus] anti-meta: ERROR rng_seed accessor missing; using seed 0");
            0
        }
    };
    let mut roll = antimeta::roll(seed);
    // Test-only override (like `CM_PLUS_SATURATION_OPT_CAP`): force the worker target so the
    // spawn path can be exercised without a lucky seed. Clamped to the legal 4..=8 range.
    if let Some(v) = crate::config::env("CM_PLUS_ANTIMETA_FORCE_WORKERS") {
        if let Ok(n) = v.trim().parse::<u8>() {
            let forced = n.clamp(4, 8);
            crate::log::log(&format!(
                "[cosmo-plus] anti-meta: TEST forcing workers {} -> {forced}",
                roll.workers
            ));
            roll.workers = forced;
        }
    }
    crate::log::log(&format!(
        "[cosmo-plus] anti-meta: seed={seed:#010x} roll workers={} minerals={} gas={}",
        roll.workers, roll.minerals, roll.gas
    ));

    let mut any = false;
    for player in 0..PLAYER_LIMIT {
        let player = player as u8;
        if !player_active(player) {
            continue;
        }
        // Race determines the worker id; a race with no worker (or no town hall) is skipped.
        let race = player_race(player);
        let Some(worker_id) = antimeta::racial_worker_id(race) else {
            continue;
        };
        // Anchor: the player's first completed town hall.
        let Some((tx, ty)) = find_town_hall(player) else {
            continue;
        };

        let mut workers = collect_workers(player);
        let before = workers.len() as u32;
        let (to_remove, to_add) = antimeta::worker_adjustment(before, roll.workers as u32);

        let removed = remove_excess(&mut workers, to_remove, tx, ty);
        let added = spawn_workers(player, worker_id, to_add, tx, ty);

        set_resources(player as usize, roll.minerals, roll.gas);

        crate::log::log(&format!(
            "[cosmo-plus] anti-meta: player {player} race {race} town_hall=({tx},{ty}) workers {before} -> {} (removed {removed}, added {added}) minerals={} gas={}",
            roll.workers, roll.minerals, roll.gas
        ));
        any = true;
    }

    if any {
        announce(&roll);
    } else {
        crate::log::log("[cosmo-plus] anti-meta: no active player with a town hall; nothing applied");
    }
}

/// Resolve the mode switch and log the resolved state. Precedence:
///
/// 1. `CM_PLUS_MODE` (env or `cosmo_plus.conf`) — explicit and wins over everything,
///    including the lobby game type. `antimeta`/`on`/`1`/`true` force-enables, any other
///    value (e.g. `off`/`0`) force-disables — useful for testing an Anti Meta lobby
///    without the roll.
/// 2. The lobby game type: a match created with the **Anti Meta** template (`*g_game_type`
///    == 46) enables the mode automatically. This is the primary, selectable path.
/// 3. `plus.yml` `mode: antimeta` — the documented fallback for maps/launchers that bypass
///    the lobby template list. Both clients must agree.
///
/// Default off.
fn enabled() -> bool {
    if let Some(m) = crate::config::env("CM_PLUS_MODE") {
        let on = m.eq_ignore_ascii_case("antimeta")
            || m.eq_ignore_ascii_case("on")
            || m == "1"
            || m.eq_ignore_ascii_case("true");
        crate::log::log(&format!(
            "[cosmo-plus] anti-meta: CM_PLUS_MODE={m} -> enabled={on} (explicit; overrides the lobby game type)"
        ));
        return on;
    }
    let game_type = unsafe { *(ADDR_GAME_TYPE as *const u8) };
    if game_type == GAME_TYPE_ANTI_META {
        crate::log::log(&format!(
            "[cosmo-plus] anti-meta: lobby game type Anti Meta ({GAME_TYPE_ANTI_META}) -> enabled"
        ));
        return true;
    }
    if let Some(m) = crate::plus::config().and_then(|c| c.mode.as_deref()) {
        let on = m.eq_ignore_ascii_case("antimeta");
        crate::log::log(&format!("[cosmo-plus] anti-meta: plus.yml mode={m} -> enabled={on}"));
        return on;
    }
    crate::log::log(&format!(
        "[cosmo-plus] anti-meta: game type {game_type}, no mode set -> disabled (default off)"
    ));
    false
}

// ---------------------------------------------------------------------------
// State readers.
// ---------------------------------------------------------------------------

unsafe fn player_type(player: u8) -> u8 {
    let base = ADDR_PLAYER_TABLE as *const u8;
    *base.add(player as usize * PLAYER_STRUCT_SIZE + PLAYER_TYPE_OFFSET)
}

unsafe fn player_race(player: u8) -> u8 {
    let base = ADDR_PLAYER_TABLE as *const u8;
    *base.add(player as usize * PLAYER_STRUCT_SIZE + PLAYER_RACE_OFFSET)
}

fn player_active(player: u8) -> bool {
    let t = unsafe { player_type(player) };
    t == PLAYER_TYPE_HUMAN || t == PLAYER_TYPE_COMPUTER
}

/// The first completed town-hall (DEPOT) owned by `player`, as its centre `(x, y)`.
unsafe fn find_town_hall(player: u8) -> Option<(i32, i32)> {
    let mut found = None;
    crate::frame::for_each_unit(|u| {
        if found.is_some() {
            return;
        }
        let id = (*u).unit_id;
        if (*u).player == player && (*u).remaining_build_time == 0 && cosmo_plus_core::saturation::DEPOT_IDS.contains(&id) {
            let p = (*u).flingy.position;
            found = Some((p.x as i32, p.y as i32));
        }
    });
    found
}

/// Collect the player's completed tier-1 workers (excluding eggs / morphing units).
unsafe fn collect_workers(player: u8) -> Vec<*mut Unit> {
    let mut v = Vec::new();
    crate::frame::for_each_unit(|u| {
        if (*u).player == player
            && (*u).remaining_build_time == 0
            && antimeta::TIER1_WORKER_IDS.contains(&(*u).unit_id)
        {
            v.push(u);
        }
    });
    v
}

// ---------------------------------------------------------------------------
// Unit mutation (inline asm, matching the C++ wrappers exactly).
// ---------------------------------------------------------------------------

// Function addresses as `static`s so they can be referenced with a `sym` operand and
// called indirectly through memory (exactly how the C++ wrappers do it: `CALL func_address`
// compiles to `call dword ptr [func_address]`). This keeps every register free for the
// runtime arguments, since LLVM reserves ESI/EDI for itself on this target.
static CREATE_UNIT_ADDR: u32 = ADDR_CREATE_UNIT as u32;
static FINISH_UNIT_PRE_ADDR: u32 = ADDR_FINISH_UNIT_PRE as u32;
static SHOW_FINISHED_UNIT_ADDR: u32 = ADDR_SHOW_FINISHED_UNIT as u32;
static FINISH_UNIT_POST_ADDR: u32 = ADDR_FINISH_UNIT_POST as u32;
static BWFUNC_REMOVE_ADDR: u32 = ADDR_BWFUNC_REMOVE as u32;
/// Single scratch cell for call return values, written inside the asm (the C++ wrappers use
/// a per-function `static result` for the same reason: a register write would be undone by
/// `POPAD`). The game loop is single-threaded and each result is read immediately.
static mut ASM_RESULT: u32 = 0;

/// `helper::create_unit` (0x004A09D0): unit id in CX, x in EAX, `PUSH y`, `PUSH player`.
/// Returns the created unit, or null when the position is blocked.
#[inline(never)]
unsafe fn create_unit(unit_id: u16, x: i32, y: i32, player: i32) -> *mut Unit {
    core::arch::asm!(
        "pushad",
        "push {player}", // player (pushed first)
        "push {y}",      // y (pushed second, on top)
        "mov eax, {x}",
        "mov ecx, {unit_id}",
        "call dword ptr [{func}]",
        "mov dword ptr [{result}], eax",
        "popad",
        player = in(reg) player,
        y = in(reg) y,
        x = in(reg) x,
        unit_id = in(reg) unit_id as u32,
        func = sym CREATE_UNIT_ADDR,
        result = sym ASM_RESULT,
        options(nostack),
    );
    ASM_RESULT as *mut Unit
}

/// `update_unit_stats_and_finish_building` (0x004A01F0): EAX = unit.
#[inline(never)]
unsafe fn finish_unit_pre(unit: *mut Unit) {
    core::arch::asm!(
        "pushad",
        "mov eax, {unit}",
        "call dword ptr [{func}]",
        "popad",
        unit = in(reg) unit as u32,
        func = sym FINISH_UNIT_PRE_ADDR,
        options(nostack),
    );
}

/// `show_finished_unit` (0x0049EC30): ESI = unit, returns b32 in EAX.
#[inline(never)]
unsafe fn show_finished_unit(unit: *mut Unit) -> u32 {
    core::arch::asm!(
        "pushad",
        "mov esi, {unit}",
        "call dword ptr [{func}]",
        "mov dword ptr [{result}], eax",
        "popad",
        unit = in(reg) unit as u32,
        func = sym SHOW_FINISHED_UNIT_ADDR,
        result = sym ASM_RESULT,
        options(nostack),
    );
    ASM_RESULT
}

/// `update_unit_strength` (0x0049FA40): EAX = unit (finishes and applies default orders).
#[inline(never)]
unsafe fn finish_unit_post(unit: *mut Unit) {
    core::arch::asm!(
        "pushad",
        "mov eax, {unit}",
        "call dword ptr [{func}]",
        "popad",
        unit = in(reg) unit as u32,
        func = sym FINISH_UNIT_POST_ADDR,
        options(nostack),
    );
}

/// Silently remove a unit: set `UserActionFlags_SilentRemove`, then `bwfunc_remove`
/// (0x00475710, EAX = unit).
#[inline(never)]
unsafe fn silent_remove(unit: *mut Unit) {
    (*unit).order_flags |= SILENT_REMOVE_BIT; // order_flags == user_action_flags (0x93)
    core::arch::asm!(
        "pushad",
        "mov eax, {unit}",
        "call dword ptr [{func}]",
        "popad",
        unit = in(reg) unit as u32,
        func = sym BWFUNC_REMOVE_ADDR,
        options(nostack),
    );
}

// ---------------------------------------------------------------------------
// Worker adjustment.
// ---------------------------------------------------------------------------

fn dist2(u: *mut Unit, tx: i32, ty: i32) -> i64 {
    let p = unsafe { (*u).flingy.position };
    let dx = p.x as i64 - tx as i64;
    let dy = p.y as i64 - ty as i64;
    dx * dx + dy * dy
}

/// Remove `to_remove` workers, furthest from the town hall first. Returns the count removed.
unsafe fn remove_excess(workers: &mut [*mut Unit], to_remove: u32, tx: i32, ty: i32) -> u32 {
    if to_remove == 0 {
        return 0;
    }
    // Stable sort by distance descending; the vector order is already deterministic.
    workers.sort_by(|a, b| dist2(*b, tx, ty).cmp(&dist2(*a, tx, ty)));
    let mut removed = 0;
    for &w in workers.iter() {
        if removed >= to_remove {
            break;
        }
        silent_remove(w);
        removed += 1;
    }
    removed
}

/// Deterministic spawn ring around the town hall: closer radii first, 8 directions each.
fn spawn_offsets() -> impl Iterator<Item = (i32, i32)> {
    const DIRECTIONS: [(i32, i32); 8] = [
        (1, 0),
        (1, 1),
        (0, 1),
        (-1, 1),
        (-1, 0),
        (-1, -1),
        (0, -1),
        (1, -1),
    ];
    const RADII: [i32; 5] = [80, 112, 144, 176, 208];
    RADII
        .into_iter()
        .flat_map(move |r| DIRECTIONS.into_iter().map(move |(dx, dy)| (dx * r, dy * r)))
}

/// Create `to_add` workers near the town hall, skipping blocked positions. Returns the count
/// actually added (fewer than requested if every candidate position is blocked).
unsafe fn spawn_workers(player: u8, worker_id: u16, to_add: u32, tx: i32, ty: i32) -> u32 {
    let mut added = 0;
    for (ox, oy) in spawn_offsets() {
        if added >= to_add {
            break;
        }
        let sx = tx + ox;
        let sy = ty + oy;
        let u = create_unit(worker_id, sx, sy, player as i32);
        if u.is_null() {
            continue; // blocked; try the next offset
        }
        finish_unit_pre(u);
        if show_finished_unit(u) != 0 {
            finish_unit_post(u);
        }
        added += 1;
    }
    added
}

// ---------------------------------------------------------------------------
// Resources + announcement.
// ---------------------------------------------------------------------------

/// Overwrite the per-player mineral and gas counters (game and total/score).
unsafe fn set_resources(player: usize, minerals: u32, gas: u32) {
    *((ADDR_GAME_MINERALS as *mut i32).add(player)) = minerals as i32;
    *((ADDR_TOTAL_MINERALS as *mut i32).add(player)) = minerals as i32;
    *((ADDR_GAME_VESPENE as *mut i32).add(player)) = gas as i32;
    *((ADDR_TOTAL_VESPENE as *mut i32).add(player)) = gas as i32;
}

/// Announce the roll in-game via `api.print_text` if it resolves; otherwise log only.
unsafe fn announce(roll: &Roll) {
    let msg = format!(
        "Cosmonarchy Plus anti-meta roll: {} workers, {} minerals, {} gas",
        roll.workers, roll.minerals, roll.gas
    );
    let c = match std::ffi::CString::new(msg) {
        Ok(c) => c,
        Err(_) => {
            crate::log::log("[cosmo-plus] anti-meta: announce string rejected; logged only");
            return;
        }
    };
    let api = crate::plus::api();
    match (api.print_text)() {
        Some(f) => {
            f(c.as_ptr() as *const u8);
            crate::log::log("[cosmo-plus] anti-meta: announced roll in-game");
        }
        None => {
            crate::log::log("[cosmo-plus] anti-meta: print_text unavailable; roll logged only");
        }
    }
}
