# Anti-meta mode — integration notes

Goal: a selectable game mode that, once per game, sets **both teams** to
- 4–8 starting workers,
- 100–300 starting minerals in steps of 5,
- 0–125 starting gas in steps of 5,

and announces the roll so players know what they got. Everything must be identical on
every peer and reproducible from the shared state.

## Where a mode lives

- In-game type: `*g_game_type` (u8, `0x00596820`) — compared in `bw::get_game_kind()`.
- Lobby template: `Release/mpq/templates/*.got`, enumerated via `templates.lst`. The engine's
  template loader (StarCraft.exe 0x4AB750 + list insert 0x4AB150) builds `g_game_type_list`
  from `templates.lst` at startup; the Create Game dropdown (`compile_list` 0x4AAD00) lists
  every entry whose **subtype == 1**, and the chat-room label (0x4B9480) uses the template's
  name block. A new template therefore needs **no engine change** to appear in the dropdown.
- `.got` layout (loader-confirmed, cross-checked against all 26 shipped templates):
  byte 0 = version marker (**must be 3**), 0x01..0x20 = name block 1 (dropdown string),
  0x21..0x40 = name block 2 (secondary label), **0x41 = game type**, 0x42 = 0,
  0x43..0x44 = subtype (LE u16, must be < 8; only 1 is listed), 0x49 = "mode" byte.
- The mode byte is read by `is_melee_game` (0x4AAFA0): the engine scans `g_game_type_list`
  and returns true when the entry's byte at node+0x50 (`= .got` 0x49) is 1. It is also
  `g_game_data.victory_condition` (GameData +0x75): 1 = `VictoryCondition::Melee`. Melee's
  own `.got` has 1 there, the Cosmonarchy modes have 0 — so "is_melee" is literally
  "victory condition melee" in this engine.
- `g_game_data.got_game_type` / `got_game_subtype` (offsets 0x6D/0x6F) are a copy of the
  settings block from the matched list entry (0x4D33D1), and the selected template's type
  lands in `*g_game_type` (0x00449882). The (type, subtype) pair is part of the synced game
  data, and the map-authorisation check (`is_authorized_map` → 0x4AAEA0) boots peers that
  do not have the template in their own `templates.lst` — so a **vanilla client cannot join
  an Anti Meta lobby**, which is the version lock we want.

Two ways to mark the mode:

1. **New template** `Anti Meta(1).got` (copy of Melee with name "Anti Meta" and game type
   46; 45 is taken by Mindshare) added to `templates.lst`. No engine change. Plus detects
   `game_type == 46` and rolls. Shipped by `cosmo-variant`, which re-stages the two files
   after its `git reset --hard` of the build tree (the release repo itself is untouched).
2. **Config flag** `CM_PLUS_MODE=antimeta` (both clients) for testing and for maps where
   the lobby list is not used. `CM_PLUS_MODE=off` force-disables even in an Anti Meta lobby.

Implemented: (1) as the primary, selectable path with (2) as override/test path.

### Known cosmetic gaps (upstream gptp, not fixable from the Plus plugin)

- The pre-game chat room's "Game Mode" label (`update_game_info_labels`, gptp glu_chat.cpp)
  is `magic_enum::enum_name(GameType::Enum(x))`; gptp's enum has no 46, so the label is
  blank there (the Create Game dropdown itself shows "Anti Meta" from the template name).
  The same applies to the CrownLink join screen label and Discord rich presence, which
  falls back to "Custom".
- `bw::get_game_kind()` returns `GameKind::Invalid` for 46. That is deliberate: every
  gptp game-start branch and mode helper checks for a *specific* kind, so Invalid behaves
  like plain melee (no Omnipresence/Duopoly/Massacre init, cheats allowed). The one
  assert-hazard, `game_kind_to_filename_string` (asserts on unknown kinds), is only called
  from the campaign chain (`create_next_campaign_game_4DC1F7`), which an Anti Meta match
  never runs.

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

## Implemented

The engine side is in `plus/crates/cosmo-plus/src/antimeta.rs`, wired into the first-tick
block in `frame.rs`. It is a data/rules feature only — no netcode.

- **Activation.** The lobby game type wins: `*g_game_type == 46` (the `Anti Meta(1).got`
  template) enables the mode with no config on either client. `CM_PLUS_MODE`
  (`antimeta`/`on`/`1`/`true` enables, anything else disables) overrides the game type;
  `plus.yml` `mode: antimeta` is the last fallback. The resolved state and the read game
  type are logged to `cosmo_plus.log`. It applies **once per game**, not once per process:
  a game-pointer/frame-reset guard re-arms it when a new match starts, so returning to the
  menu and starting another game rolls again.
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

### Lobby game type (v2)

The mode is a **selectable game type** in the Create Game dropdown:

- **Template.** `tools/make-template.py` generates `Anti Meta(1).got` from the build's own
  `Melee(1).got`: name "Anti Meta", game type **46**, subtype 1, everything else (victory
  condition melee, starting units, fog, teams) inherited. Type 46 because 45 is Mindshare's.
- **Enumeration.** `templates.lst` must list the file; `cosmo-variant`'s `stage_assets`
  re-stages the `.got` and inserts the `.lst` line right after `Unranked(1).got` on every
  Plus launch (after its `git reset --hard`), so the release repo stays untouched and the
  dropdown shows **Anti Meta** directly under Unranked.
- **Activation.** `enabled()` reads `*g_game_type` (0x00596820) at the first game tick:
  46 → roll, no config needed. `CM_PLUS_MODE` still overrides (explicit on/off) and
  `plus.yml mode:` remains the fallback for non-lobby starts.
- **Peer gating.** The engine's map-authorisation check looks the (type, subtype) pair up
  in each client's own template list; a client without the staged template is booted
  (boot reason 9), so vanilla/older clients cannot join an Anti Meta lobby and silently
  desync.
- **No netcode involvement.** The type byte is part of the standard synced game data;
  neither CrownLink nor the rollback plugin inspects it.

### Lobby limitation (v1, superseded)

v1 shipped without the template: both clients had to set `CM_PLUS_MODE=antimeta` (or
`mode: antimeta` in `plus.yml`) by hand. That path still works and remains the documented
fallback for maps/launchers that bypass the lobby template list.

## Edge cases

- Maps that pre-place more workers than the cap (some UMS maps): remove down to the target,
  but never remove non-worker units.
- Fewer than 4 start locations/players: apply only to active players.
- Replays: **skipped** by design (see above). A replay records whatever start the live game
  had, so playback reproduces it without re-rolling.
- Rollback: no new persistent state is needed (the roll is applied once, before the first
  snapshot), so no serializer changes. If the roll is stored for display, put it in plugin
  state.
