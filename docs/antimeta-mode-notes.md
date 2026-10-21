# Anti-meta mode — integration notes

Goal: a selectable game mode that, once per game, sets **both teams** to
- 4–8 starting workers,
- 100–300 starting minerals in steps of 5,
- 0–125 starting gas in steps of 5,

and announces the roll so players know what they got. Everything must be identical on
every peer and reproducible from the shared state.

## Where a mode lives

- In-game type: `*g_game_type` (u8, `0x00596820`) — compared in `bw::get_game_kind()`.
- Lobby template: `Release/mpq/templates/*.got`, enumerated via `templates.lst`.
- `.got` layout (confirmed by diffing): byte 0 = name length, 1..0x1C = name, then the
  settings block; **byte `0x41` = game type**, bytes `0x42..0x43` = subtype (LE u16).
  `Melee(1).got` has `0x41 = 0x02`; `Massacre(1).got` has `0x41 = 0x22` (=34).
- `g_game_data.got_game_type` / `got_game_subtype` carry the chosen template into the
  game (struct offsets 0x6D/0x6F, `BW/structures.h:181`).

Two ways to mark the mode:

1. **New template** `Anti Meta(1).got` (copy of Melee with a subtype marker) added to
   `templates.lst`. No engine change; Plus detects `game_type == Melee/Unranked` plus the
   marker. Risk: the launcher may overwrite `templates/` on update; ship it as an overlay
   file and document it.
2. **Config flag** `CM_PLUS_MODE=antimeta` (both clients) for testing and for maps where
   the lobby list is not used.

Recommendation: implement (1) with (2) as the test path; verify the marker survives a
real CrownLink lobby handshake before relying on it.

## Determinism

- Seed the roll from the **game's own synced RNG state** (`g_rng_seed` = 0x0051CA14, part of
  the snapped state), never the wall clock. The seed value is identical on both peers at game
  start. The roll itself is a deterministic SplitMix64 expansion of that seed (the pure
  `roll(seed)` in `cosmo-plus-core`), so both peers compute the same tuple.
- Plus runs before the first frame, therefore before the rollback plugin's first snapshot
  and before any player command. That makes the modified start part of the initial state on
  both peers.
- Do not randomise per player: draw one tuple and apply it to every player slot that is in
  the game (same roll for both teams), as requested.

## Application point

- Native equivalent of the mod's `on_game_start` (`0x004C9A51`): after `create_init_units`
  and before the `GameKind` switch. The Rust plugin currently only has the frame hook; the
  first `run_game_tick` is late but still before input is processed — acceptable for v1,
  and testable. A dedicated init hook is cleaner if the address resolves.
- Adjust units: count each player's completed workers; spawn/remove to reach the target.
  Keep the town hall and any pre-placed buildings. Spawn location: near the town hall,
  using the game's `create_unit` path (same as `aise`).
- Resources: `g_game_minerals` (`0x0057F0F0`), `g_game_vespene` (`0x0057F120`), plus the
  matching `g_total_*` counters used for score.
- Announce with a centre-screen message (`bw::println`) so players see the roll.

## Implemented (v1)

The engine side is in `plus/crates/cosmo-plus/src/antimeta.rs`, wired into the first-tick
block in `frame.rs`. It is a data/rules feature only — no netcode.

- **Activation.** `CM_PLUS_MODE=antimeta` (environment, or a `CM_PLUS_MODE=antimeta` line in
  `cosmo_plus.conf`) wins over `plus.yml` `mode: antimeta`. Absent both, the mode is **off**.
  The resolved state is logged to `cosmo_plus.log`. It applies **once per game**, not once
  per process: a game-pointer/frame-reset guard re-arms it when a new match starts, so
  returning to the menu and starting another game rolls again.
- **Replays are skipped.** If `g_game_data.is_replay` is set, the mode logs
  `replay detected -> skipped` and does not touch the economy (rewriting a replay's start
  would corrupt playback).
- **Seed.** The game's synced RNG seed (`g_rng_seed`, `0x0051CA14`), read through
  `api.rng_seed`, at the first game tick. Both peers read the same value at game start, so
  no round trip is needed.
- **Roll.** `cosmo_plus_core::antimeta::roll(seed)` (SplitMix64) → workers 4..=8,
  minerals 100..=300 step 5, gas 0..=125 step 5. Logged as `seed=… roll workers=… …`.
- **Per active player** (Human or Computer, not neutral, with a completed town hall), in
  fixed player order 0..7:
  1. find the first completed town hall (`DEPOT_IDS`) and its centre position;
  2. count completed tier-1 workers (Mason 7 / Droleth 41 / Scribe 64 / Pilgrim 439; units
     with `remaining_build_time == 0`, so eggs/morphs are excluded);
  3. remove excess workers silently, furthest from the town hall first
     (`unit->user_action_flags |= UserActionFlags_SilentRemove` then `bwfunc_remove`
     0x00475710);
  4. spawn missing workers near the town hall with the game's `create_unit` (0x004A09D0)
     + `update_unit_stats_and_finish_building` (0x004A01F0) + `show_finished_unit`
     (0x0049EC30) + `update_unit_strength` (0x0049FA40). Spawn position is the town-hall
     centre plus a deterministic ring of offsets (8 directions × radii 80..208 px); if a
     position is blocked, the next offset is tried. Never spawns inside the town hall
     (the create path rejects colliding positions);
  5. overwrite `g_game_minerals`/`g_total_minerals`/`g_game_vespene`/`g_total_vespene` for
     that player with the rolled values.
  Each player logs `workers {before} -> {target} (removed, added) minerals gas`.
- **Announce.** The roll is shown in-game via `api.print_text` (the `BWFXN_DisplayText`
  path, player 8, non-replay). If the accessor is unavailable, it logs only.

### Lobby limitation

The mode is **not** a lobby/game-type entry. It is a per-client config flag: both clients in
a match must set `CM_PLUS_MODE=antimeta` (or `mode: antimeta` in their `plus.yml`) and agree
on the same value, or the starts will desync. A lobby template (`Anti Meta(1).got`) is not
shipped because the engine's `is_melee_game`/`GameTypeList` rejects unknown (type, subtype)
pairs — a new entry needs an engine change, not just a `.got` file.

## Edge cases

- Maps that pre-place more workers than the cap (some UMS maps): remove down to the target,
  but never remove non-worker units.
- Fewer than 4 start locations/players: apply only to active players.
- Replays: **skipped** by design (see above). A replay records whatever start the live game
  had, so playback reproduces it without re-rolling.
- Rollback: no new persistent state is needed (the roll is applied once, before the first
  snapshot), so no serializer changes. If the roll is stored for display, put it in plugin
  state.
