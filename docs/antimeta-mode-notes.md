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

- Use the **game's own RNG** at init (`g_rng_seed`, part of the snapped state), not a local
  RNG and never the wall clock. Both peers start from the same seed and draw in the same
  order (player 0..7, then workers, then minerals, then gas), so they compute the same roll.
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

## Edge cases

- Maps that pre-place more workers than the cap (some UMS maps): remove down to the target,
  but never remove non-worker units.
- Fewer than 4 start locations/players: apply only to active players.
- Replays: the mode is in the template, so a replay re-applies the same roll deterministically
  only if the RNG state at init is reproduced; verify, otherwise record the roll in the
  replay header/chat.
- Rollback: no new persistent state is needed (the roll is applied once), so no serializer
  changes. If the roll is stored for display, put it in plugin state.
