//! Draft mode (Mechabellum-style roguelite) — engine side.
//!
//! Every 2 minutes of game time the game pauses on both peers, both players are shown
//! the same 4 options (drawn deterministically from the shared RNG seed + race pair),
//! and each picks with hotkeys 1–4 (Esc = skip). When every active player has answered,
//! the picks apply simultaneously and the game unpauses.
//!
//! Determinism map (see `docs/draft-mode-notes.md`):
//!   * the clock and the pause write are derived from synced game state (`frame_count`,
//!     `g_is_game_paused` at 0x006509C4 — the flag is in the rollback snapshot ranges);
//!   * the option pool is a pure function of (match seed, draft number, race pair);
//!   * picks travel as a **synced game command** (id 93, fixed 4 bytes) through the
//!     normal command stream — never raw keyboard reads in the sim;
//!   * the whole session state serializes through the `cosmo_plus_state_*` blob, which
//!     the rollback plugin captures by name (plugin_state.rs) and which replays
//!     re-derive by re-running the recorded pick commands.
//!
//! The command hook runs in the engine's command processing path (it keeps running
//! while the sim is paused — that's how unpause commands work), so picks are received
//! and the unpause decision is made there, exactly like gptp's faction commands.

use std::sync::atomic::{AtomicU32, Ordering};

use bw_dat::structs::Unit;

use cosmo_plus_core::draft::{self, DraftOption, DraftState, Effect, PICK_NONE, PICK_SKIP};
use cosmo_plus_core::config::validate_draft_pools;

// ---------------------------------------------------------------------------
// Fixed 1.16.1 addresses.
// ---------------------------------------------------------------------------

/// `*g_game_type` (BW/bwdata.h:221).
const ADDR_GAME_TYPE: usize = 0x00596820;
/// `g_is_game_paused` (BW/bwdata.h:1045) — in the rollback snapshot ranges.
const ADDR_IS_PAUSED: usize = 0x006509C4;
/// `helper::queue_game_command` (BW/bwhelper.cpp:6338): ECX = params, EDX = length.
/// Broadcasts the command to every peer (including the local client) through the
/// normal game command stream the netcode already carries.
const ADDR_QUEUE_GAME_COMMAND: usize = 0x00485BD0;
/// Player table base (12 × 0x24): `type` at 0x08, `race` at 0x09.
const ADDR_PLAYER_TABLE: usize = 0x0057EEE0;
const PLAYER_STRUCT_SIZE: usize = 0x24;
const PLAYER_TYPE_OFFSET: usize = 0x08;
const PLAYER_RACE_OFFSET: usize = 0x09;

/// `PlayerType::Computer` / `PlayerType::Human`.
const PLAYER_TYPE_COMPUTER: u8 = 1;
const PLAYER_TYPE_HUMAN: u8 = 2;

/// Game type byte of the `Draft(1).got` lobby template (45 = Mindshare, 46 = Anti Meta).
pub(crate) const GAME_TYPE_DRAFT: u8 = 47;

/// Our custom game command id. **89** (0x59) deliberately: the samase replay-data path
/// records commands with `id < 0x5d`, so picks at id 89 are captured into replays
/// automatically and re-delivered on playback (ids 93–98 would not be, and a replay
/// would then hang paused forever). 89 is free in the engine's GameCommand table
/// (between MinimapPing 88 and MergeDarkArchon 90); gptp's Custom id is 99.
const DRAFT_COMMAND_ID: u8 = 89;
/// Fixed command size: {id, player, choice, pad}.
const DRAFT_COMMAND_SIZE: u32 = 4;
/// Choice byte for "skip".
const CHOICE_SKIP: u8 = 0xFF;

// ---------------------------------------------------------------------------
// Session state (the blob payload).
// ---------------------------------------------------------------------------

/// The draft session. Single-threaded game loop: plain statics are fine.
static STATE: std::sync::Mutex<DraftState> = std::sync::Mutex::new(DraftState {
    draft: 0,
    next_draft: 1,
    options: [0; draft::OPTION_COUNT],
    picks: [PICK_NONE; draft::PLAYER_LIMIT],
    applied: false,
});
/// Guard so the state resets once per game (game pointer change), like anti-meta.
static LAST_GAME: AtomicU32 = AtomicU32::new(0);
/// Local player id, cached from the api (set lazily; used for the panel highlight).
static LOCAL_PLAYER: AtomicU32 = AtomicU32::new(u32::MAX);
/// Rollback enable switch for the blob serializer.
static ROLLBACK_ENABLED: AtomicU32 = AtomicU32::new(0);

const BLOB_VERSION: u32 = 1;

fn with_state<R>(f: impl FnOnce(&mut DraftState) -> R) -> R {
    f(&mut STATE.lock().unwrap())
}

/// Reset the session when a new game starts (same guard pattern as anti-meta).
fn reset_for_new_game(game: usize) {
    let last = LAST_GAME.swap(game as u32, Ordering::SeqCst);
    if last != game as u32 {
        *STATE.lock().unwrap() = DraftState::default();
        crate::log::log("[cosmo-plus] draft: new game -> state reset");
    }
}

// ---------------------------------------------------------------------------
// Mode resolution.
// ---------------------------------------------------------------------------

/// Draft mode is active when the match was created with the Draft game type (47).
pub(crate) fn enabled() -> bool {
    unsafe { *(ADDR_GAME_TYPE as *const u8) == GAME_TYPE_DRAFT }
}

fn is_replay() -> bool {
    const G_GAME_DATA: usize = 0x005967F8;
    const IS_REPLAY_OFFSET: usize = 0x32;
    unsafe { *((G_GAME_DATA + IS_REPLAY_OFFSET) as *const u8) != 0 }
}

// ---------------------------------------------------------------------------
// Per-frame clock (runs in the step hook, after anti-meta).
// ---------------------------------------------------------------------------

/// The per-frame entry point: trigger drafts at their frame, finalize interrupted
/// drafts when the game resumes unpaused without answers.
pub(crate) unsafe fn on_game_tick() {
    if !enabled() {
        return;
    }
    let api = crate::plus::api();
    let game = match (api.game)() {
        Some(f) => f() as usize,
        None => return,
    };
    if game == 0 {
        return;
    }
    reset_for_new_game(game);
    let frame = (*(game as *const bw_dat::structs::Game)).frame_count;
    let paused = *(ADDR_IS_PAUSED as *const u8) != 0;

    let (trigger_now, in_progress) = with_state(|s| {
        (
            s.draft == 0 && s.next_draft != 0 && frame >= draft::draft_frame(s.next_draft),
            s.draft != 0,
        )
    });

    if trigger_now && !paused {
        begin_draft(frame);
    } else if in_progress && !paused && !is_replay() {
        // The game resumed without a command-driven finish (manual unpause): finalize
        // with skips for everyone who hasn't answered.
        with_state(|s| {
            let active = active_mask();
            for (p, pick) in s.picks.iter_mut().enumerate() {
                if *pick == PICK_NONE && active & (1 << p) != 0 {
                    *pick = PICK_SKIP;
                }
            }
        });
        finish_draft();
    }
}

/// Open a draft: draw the pool, open the panel, pause the game.
unsafe fn begin_draft(frame: u32) {
    let api = crate::plus::api();
    let seed = match (api.rng_seed)() {
        Some(f) => f(),
        None => 0,
    };

    let Some(drawn) = draw_pool(seed) else {
        // No tables in plus.yml: log and carry on without pausing (never soft-lock).
        crate::log::log("[cosmo-plus] draft: no draft.options in plus.yml -> skipped (no pause)");
        with_state(|s| s.next_draft += 1);
        return;
    };

    let (draft_no, next) = with_state(|s| (s.next_draft, s.next_draft + 1));
    with_state(|s| {
        s.draft = draft_no;
        s.next_draft = next;
        s.options = drawn;
        s.picks = [PICK_NONE; draft::PLAYER_LIMIT];
        s.applied = false;
    });

    crate::log::log(&format!(
        "[cosmo-plus] draft: draft #{draft_no} opens at frame {frame} (seed={seed:#010x})"
    ));

    // Pause deterministically: same frame on both peers -> same write. The flag is
    // covered by the rollback snapshot ranges.
    *(ADDR_IS_PAUSED as *mut u8) = 1;
}

/// Draw the 4-option pool for this draft from the `plus.yml` tables.
///
/// `None` when the tables are absent/empty. Both peers derive the identical indices
/// because the seed is the game's synced RNG seed and the race pair is synced state.
unsafe fn draw_pool(seed: u32) -> Option<[u32; draft::OPTION_COUNT]> {
    let cfg = crate::plus::config()?;
    if cfg.draft.is_empty() {
        return None;
    }
    let draft_no = with_state(|s| s.next_draft);
    let pair = match_pair();
    let available: Vec<u32> = cfg
        .draft
        .iter()
        .enumerate()
        .filter(|(_, o)| o.min_draft <= draft_no && (o.pair.is_none() || o.pair == Some(pair)))
        .map(|(i, _)| i as u32)
        .collect();
    if available.len() < draft::OPTION_COUNT {
        crate::log::log(&format!(
            "[cosmo-plus] draft: only {} option(s) available for pair {pair:?} at draft #{draft_no} (< {})",
            available.len(),
            draft::OPTION_COUNT
        ));
        return None;
    }
    let weights: Vec<u32> = available
        .iter()
        .map(|&i| cfg.draft[i as usize].weight)
        .collect();
    let drawn = draft::draw_options(u64::from(seed) ^ u64::from(draft_no), &weights, draft::OPTION_COUNT);
    let mut out = [0u32; draft::OPTION_COUNT];
    for (k, &w) in drawn.iter().enumerate() {
        out[k] = available[w];
    }
    for (k, &idx) in out.iter().enumerate() {
        crate::log::log(&format!(
            "[cosmo-plus] draft: option {k}: {} — {}",
            cfg.draft[idx as usize].name,
            cfg.draft[idx as usize].id
        ));
    }
    Some(out)
}

/// The unordered race pair of the first two active players (v1 is a 1v1 mode).
unsafe fn match_pair() -> (u8, u8) {
    let mut races = [255u8; 2];
    let mut n = 0;
    for p in 0..draft::PLAYER_LIMIT as u8 {
        if n == 2 {
            break;
        }
        let t = player_type(p);
        if t == PLAYER_TYPE_HUMAN || t == PLAYER_TYPE_COMPUTER {
            races[n] = player_race(p);
            n += 1;
        }
    }
    if n < 2 {
        (races[0], races[0])
    } else {
        (races[0], races[1])
    }
}

unsafe fn player_type(player: u8) -> u8 {
    let base = ADDR_PLAYER_TABLE as *const u8;
    *base.add(player as usize * PLAYER_STRUCT_SIZE + PLAYER_TYPE_OFFSET)
}

unsafe fn player_race(player: u8) -> u8 {
    let base = ADDR_PLAYER_TABLE as *const u8;
    *base.add(player as usize * PLAYER_STRUCT_SIZE + PLAYER_RACE_OFFSET)
}

/// Bitmask of active (Human/Computer) player slots.
fn active_mask() -> u8 {
    let mut mask = 0u8;
    for p in 0..draft::PLAYER_LIMIT as u8 {
        let t = unsafe { player_type(p) };
        if t == PLAYER_TYPE_HUMAN || t == PLAYER_TYPE_COMPUTER {
            mask |= 1 << p;
        }
    }
    mask
}

// ---------------------------------------------------------------------------
// Finish + apply.
// ---------------------------------------------------------------------------

/// Apply the picks of every active player in fixed order, then close the draft.
/// Called from the command hook (all answered) or the tick hook (manual unpause).
unsafe fn finish_draft() {
    let (draft_no, options, picks, applied) = with_state(|s| {
        (s.draft, s.options, s.picks, s.applied)
    });
    if applied || draft_no == 0 {
        return;
    }
    let cfg = match crate::plus::config() {
        Some(c) => c,
        None => return,
    };
    for p in 0..draft::PLAYER_LIMIT as u8 {
        let choice = picks[p as usize];
        if choice == PICK_NONE {
            continue;
        }
        if choice == PICK_SKIP {
            crate::log::log(&format!("[cosmo-plus] draft: player {p} skipped draft #{draft_no}"));
            continue;
        }
        let idx = options[choice as usize] as usize;
        let Some(opt) = cfg.draft.get(idx) else {
            crate::log::log(&format!("[cosmo-plus] draft: player {p} picked unknown option index {idx}"));
            continue;
        };
        crate::log::log(&format!(
            "[cosmo-plus] draft: player {p} picked \"{}\" ({})",
            opt.name, opt.id
        ));
        apply_effect(p, &opt.effect);
    }
    with_state(|s| {
        s.applied = true;
        s.draft = 0;
        s.picks = [PICK_NONE; draft::PLAYER_LIMIT];
    });
    *(ADDR_IS_PAUSED as *mut u8) = 0;
    crate::log::log(&format!("[cosmo-plus] draft: draft #{draft_no} closed; game unpaused"));
    announce_close(draft_no);
}

/// Apply one effect to a player. v1 implements the proven write paths; the rest log
/// as unimplemented until their phases land.
unsafe fn apply_effect(player: u8, effect: &Effect) {
    match effect {
        Effect::Resources { minerals, gas } => {
            add_resources(player, *minerals, *gas);
        }
        Effect::SpawnUnits { units, count, anchor } => {
            spawn_drop(player, units, *count, *anchor);
        }
        Effect::HealAll => {
            let n = heal_all(player);
            crate::log::log(&format!("[cosmo-plus] draft: healed {n} unit(s) for player {player}"));
        }
        Effect::RechargeAll
        | Effect::Reveal { .. }
        | Effect::TempWeaponLevels { .. }
        | Effect::TempArmorLevels { .. }
        | Effect::RefundToken => {
            crate::log::log(&format!(
                "[cosmo-plus] draft: effect {effect:?} not implemented yet (phase 2/3)"
            ));
        }
    }
}

/// `g_game_minerals` / `g_total_minerals` / `g_game_vespene` / `g_total_vespene`.
const ADDR_GAME_MINERALS: usize = 0x0057F0F0;
const ADDR_TOTAL_MINERALS: usize = 0x0057F180;
const ADDR_GAME_VESPENE: usize = 0x0057F120;
const ADDR_TOTAL_VESPENE: usize = 0x0057F150;

unsafe fn add_resources(player: u8, minerals: i32, gas: i32) {
    let p = player as usize;
    *((ADDR_GAME_MINERALS as *mut i32).add(p)) += minerals;
    *((ADDR_TOTAL_MINERALS as *mut i32).add(p)) += minerals;
    *((ADDR_GAME_VESPENE as *mut i32).add(p)) += gas;
    *((ADDR_TOTAL_VESPENE as *mut i32).add(p)) += gas;
}

// Unit creation chain addresses (see antimeta.rs for the full conventions).
const ADDR_CREATE_UNIT: usize = 0x004A09D0;
const ADDR_FINISH_UNIT_PRE: usize = 0x004A01F0;
const ADDR_SHOW_FINISHED_UNIT: usize = 0x0049EC30;
const ADDR_FINISH_UNIT_POST: usize = 0x0049FA40;

static CREATE_UNIT_ADDR: u32 = ADDR_CREATE_UNIT as u32;
static FINISH_UNIT_PRE_ADDR: u32 = ADDR_FINISH_UNIT_PRE as u32;
static SHOW_FINISHED_UNIT_ADDR: u32 = ADDR_SHOW_FINISHED_UNIT as u32;
static FINISH_UNIT_POST_ADDR: u32 = ADDR_FINISH_UNIT_POST as u32;
static mut ASM_RESULT: u32 = 0;

#[inline(never)]
unsafe fn create_unit(unit_id: u16, x: i32, y: i32, player: i32) -> *mut Unit {
    core::arch::asm!(
        "pushad",
        "push {player}",
        "push {y}",
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

/// Town-hall anchor: first completed depot of the player (mirrors antimeta).
unsafe fn find_town_hall(player: u8) -> Option<(i32, i32)> {
    let mut found = None;
    crate::frame::for_each_unit(|u| {
        if found.is_some() {
            return;
        }
        if (*u).player == player
            && (*u).remaining_build_time == 0
            && cosmo_plus_core::saturation::DEPOT_IDS.contains(&(*u).unit_id)
        {
            let p = (*u).flingy.position;
            found = Some((p.x as i32, p.y as i32));
        }
    });
    found
}

/// Deterministic spawn ring around the anchor (8 directions × 5 radii).
fn spawn_offsets() -> impl Iterator<Item = (i32, i32)> {
    const DIRECTIONS: [(i32, i32); 8] = [
        (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1),
    ];
    const RADII: [i32; 5] = [80, 112, 144, 176, 208];
    RADII.into_iter().flat_map(move |r| DIRECTIONS.into_iter().map(move |(dx, dy)| (dx * r, dy * r)))
}

unsafe fn spawn_drop(player: u8, units: &[u16], count: u8, anchor: cosmo_plus_core::draft::Anchor) {
    let Some((tx, ty)) = find_town_hall(player) else {
        crate::log::log(&format!("[cosmo-plus] draft: player {player} has no town hall; drop cancelled"));
        return;
    };
    let mut added = 0u32;
    let target = u32::from(count) * units.len() as u32;
    for &id in units {
        for (ox, oy) in spawn_offsets() {
            if added >= target {
                break;
            }
            let u = create_unit(id, tx + ox, ty + oy, player as i32);
            if u.is_null() {
                continue;
            }
            finish_unit_pre(u);
            if show_finished_unit(u) != 0 {
                finish_unit_post(u);
            }
            added += 1;
        }
    }
    crate::log::log(&format!(
        "[cosmo-plus] draft: dropped {added}/{target} unit(s) for player {player} (anchor {anchor:?})"
    ));
}

/// Full heal: set every owned unit's hp to its base max (flingy.hitpoints is the unit's
/// live HP field; the dat max comes from bw_dat's unit table). Returns the count.
unsafe fn heal_all(player: u8) -> u32 {
    let mut n = 0;
    crate::frame::for_each_unit(|u| {
        if (*u).player != player || (*u).remaining_build_time != 0 {
            return;
        }
        let max = bw_dat::UnitId((*u).unit_id).hitpoints();
        let hp = (*u).flingy.hitpoints;
        if hp < max {
            (*u).flingy.hitpoints = max;
            n += 1;
        }
    });
    n
}

unsafe fn announce_close(draft_no: u32) {
    let msg = format!("Draft #{draft_no} resolved — choices applied!");
    let Ok(c) = std::ffi::CString::new(msg) else { return };
    let api = crate::plus::api();
    if let Some(f) = (api.print_text)() {
        f(c.as_ptr() as *const u8);
    }
}

// ---------------------------------------------------------------------------
// Pick command: {id, player, choice, pad} — id 89, fixed 4 bytes.
// ---------------------------------------------------------------------------

/// Fixed length for the command stream parser (samase's command-length override).
unsafe extern "C" fn draft_command_length(_data: *const u8, _len: u32) -> u32 {
    DRAFT_COMMAND_SIZE
}

/// Called by the engine's command stream for our command id. Runs on both peers with
/// the same command data, so `picks` stays identical everywhere.
///
/// Signature = samase's `IngameCommandHook`:
/// (data, len, command_user, unique_command_user, orig). We never call `orig` — the
/// engine has no handler for id 89 and the stream advances via the length override.
unsafe extern "C" fn pick_command_hook(
    data: *const u8,
    len: u32,
    _command_user: u32,
    _unique_user: u32,
    _orig: unsafe extern "C" fn(*const u8, u32),
) {
    if len < DRAFT_COMMAND_SIZE {
        return;
    }
    let bytes = core::slice::from_raw_parts(data, DRAFT_COMMAND_SIZE as usize);
    if bytes[0] != DRAFT_COMMAND_ID {
        return;
    }
    let player = bytes[1];
    let choice = bytes[2];
    if (player as usize) >= draft::PLAYER_LIMIT {
        return;
    }
    if choice > 3 && choice != CHOICE_SKIP {
        return;
    }
    let choice_i8 = if choice == CHOICE_SKIP { PICK_SKIP } else { choice as i8 };
    let in_progress = with_state(|s| {
        if s.draft != 0 && s.picks[player as usize] == PICK_NONE {
            s.picks[player as usize] = choice_i8;
            true
        } else {
            false
        }
    });
    if !in_progress {
        return;
    }
    crate::log::log(&format!(
        "[cosmo-plus] draft: received pick: player {player} -> {}",
        if choice == CHOICE_SKIP { "skip".to_string() } else { format!("option {choice}") }
    ));
    let done = with_state(|s| s.all_responded(active_mask()));
    if done {
        finish_draft();
    }
}

// ---------------------------------------------------------------------------
// Panel UI + hotkey input (draw pass — local-only reads, then a synced command).
// ---------------------------------------------------------------------------

/// Draw the draft panel in the draw pass. Runs on every peer; the highlight differs
/// per client (draw is never re-simmed, so that's fine).
pub(crate) unsafe fn draw_panel() {
    let (draft_no, options, picks) = with_state(|s| (s.draft, s.options, s.picks));
    if draft_no == 0 {
        return;
    }
    let cfg = match crate::plus::config() {
        Some(c) => c,
        None => return,
    };
    let local = LOCAL_PLAYER.load(Ordering::SeqCst) as usize;
    let names: Vec<String> = options
        .iter()
        .enumerate()
        .map(|(k, &i)| {
            let o: &DraftOption = &cfg.draft[i as usize];
            let pick_mark = if local < draft::PLAYER_LIMIT && picks[local] == i as i8 {
                "  <= (you)"
            } else {
                ""
            };
            format!("[{}] {}{}", k + 1, o.name, pick_mark)
        })
        .collect();
    let title = format!(
        "Draft #{} — pick with 1-4, Esc to skip ({}/{} answered)",
        draft_no,
        picks.iter().filter(|&&p| p != PICK_NONE).count(),
        draft::OPTION_COUNT
    );
    crate::overlay::draw_text_lines(&title, &names);
}

/// Poll the local keyboard for 1–4 / Esc while a draft is open, and queue the pick as a
/// synced command on the key's rising edge. Runs from the draw pass (the sim is paused;
/// the draw path still executes and is allowed to read local input).
pub(crate) unsafe fn poll_pick_input() {
    let draft_open = with_state(|s| s.draft != 0);
    if !draft_open {
        return;
    }
    let local = resolve_local_player();
    let Some(local) = local else { return };
    // 1..4, Escape.
    for key in [0x31u16, 0x32, 0x33, 0x34, 0x1B] {
        if key_rising_edge(key) {
            let choice = match key {
                0x31 => 0,
                0x32 => 1,
                0x33 => 2,
                0x34 => 3,
                _ => CHOICE_SKIP,
            };
            queue_pick_command(local, choice);
            break;
        }
    }
}

static KEY_EDGE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// 8 bits: low nibble = which key (1-4, Esc = 5), bit 3 = previous state (was-down).
fn key_rising_edge(vk: u16) -> bool {
    // Bit layout: bit k (0..4) = down state of key index k. Only keys 0..=4 tracked.
    let idx = match vk {
        0x31 => 0,
        0x32 => 1,
        0x33 => 2,
        0x34 => 3,
        0x1B => 4,
        _ => return false,
    };
    let down = (unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(vk as i32) } as u16 & 0x8000) != 0;
    let prev = KEY_EDGE.load(Ordering::SeqCst);
    let was = prev & (1 << idx) != 0;
    let mut next = prev & !(1 << idx);
    if down {
        next |= 1 << idx;
    }
    KEY_EDGE.store(next, Ordering::SeqCst);
    down && !was
}

fn resolve_local_player() -> Option<u8> {
    let cached = LOCAL_PLAYER.load(Ordering::SeqCst);
    if cached != u32::MAX {
        return Some(cached as u8);
    }
    let api = crate::plus::api();
    let f = unsafe { (api.local_player_id)()? };
    let id = unsafe { f() };
    LOCAL_PLAYER.store(id, Ordering::SeqCst);
    (id < draft::PLAYER_LIMIT as u32).then_some(id as u8)
}

unsafe fn queue_pick_command(player: u8, choice: u8) {
    let cmd = [DRAFT_COMMAND_ID, player, choice, 0u8];
    crate::log::log(&format!(
        "[cosmo-plus] draft: queueing local pick: player {player} -> {choice}"
    ));
    core::arch::asm!(
        "pushad",
        "mov edx, {len}",
        "mov ecx, {params}",
        "call dword ptr [{func}]",
        "popad",
        len = in(reg) DRAFT_COMMAND_SIZE,
        params = in(reg) cmd.as_ptr() as u32,
        func = sym QUEUE_GAME_COMMAND_ADDR,
        options(nostack),
    );
}
static QUEUE_GAME_COMMAND_ADDR: u32 = ADDR_QUEUE_GAME_COMMAND as u32;

// ---------------------------------------------------------------------------
// Install + state blob exports.
// ---------------------------------------------------------------------------

/// Register the pick-command hook. Runs once at plugin init (before any match).
pub(crate) fn install() {
    let api = crate::plus::api();
    let r = unsafe {
        (api.hook_ingame_command)(
            DRAFT_COMMAND_ID as u32,
            pick_command_hook,
            Some(draft_command_length),
        )
    };
    crate::log::log(&format!(
        "[cosmo-plus] draft: registered ingame command hook for id {DRAFT_COMMAND_ID} (result={r})"
    ));

    if let Some(cfg) = crate::plus::config() {
        let pairs = [(0u8, 0u8), (0, 1), (0, 2), (1, 1), (1, 2), (2, 2), (0, 69), (0, 70), (1, 69), (1, 70), (2, 69), (2, 70), (69, 69), (69, 70), (70, 70)];
        let gaps = validate_draft_pools(&cfg.draft, &pairs, 20);
        if !gaps.is_empty() {
            crate::log::log(&format!(
                "[cosmo-plus] draft: pool gaps ({} total), first: {}",
                gaps.len(),
                gaps[0]
            ));
        } else {
            crate::log::log("[cosmo-plus] draft: option pools validate for all race pairs");
        }
    }
}

// State blob exports, resolved by name by the rollback plugin (plugin_state.rs) and
// the replay system (extend_save). cdecl on i686 — `extern "C"`.

fn blob() -> Vec<u8> {
    let mut b = Vec::with_capacity(4 + DraftState::ENCODED_SIZE);
    b.extend_from_slice(&BLOB_VERSION.to_le_bytes());
    b.extend_from_slice(&with_state(|s| s.encode()));
    b
}

#[no_mangle]
pub unsafe extern "C" fn cosmo_plus_state_size() -> u32 {
    (4 + DraftState::ENCODED_SIZE) as u32
}

#[no_mangle]
pub unsafe extern "C" fn cosmo_plus_state_save(dst: *mut u8, cap: u32) -> u32 {
    let b = blob();
    if cap < b.len() as u32 {
        return 0;
    }
    core::ptr::copy_nonoverlapping(b.as_ptr(), dst, b.len());
    b.len() as u32
}

#[no_mangle]
pub unsafe extern "C" fn cosmo_plus_state_load(src: *const u8, len: u32) -> u32 {
    let bytes = core::slice::from_raw_parts(src, len as usize);
    if bytes.len() < 4 {
        return 0;
    }
    let version = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    if version != BLOB_VERSION {
        crate::log::log(&format!(
            "[cosmo-plus] draft: state load: unsupported blob version {version}"
        ));
        return 0;
    }
    match DraftState::decode(&bytes[4..]) {
        Some(s) => {
            *STATE.lock().unwrap() = s;
            1
        }
        None => 0,
    }
}

#[no_mangle]
pub unsafe extern "C" fn cosmo_plus_state_rollback_enable(v: i32) {
    ROLLBACK_ENABLED.store(v as u32, Ordering::SeqCst);
    crate::log::log(&format!(
        "[cosmo-plus] draft: rollback serializer enabled={}",
        v != 0
    ));
}
