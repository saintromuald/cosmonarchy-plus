# Differences between Cosmonarchy OG and Cosmonarchy Plus

Every change Cosmonarchy Plus makes on top of stock Cosmonarchy BW, as of this
writing. Also shown in-game at any time: press **Pause/Break** during a match
(`CM_PLUS_CHANGES_KEY` to rebind).

> **Many more changes are coming soon** — economy passes, more QoL hotkeys,
> UI polish, and netcode hardening are all in active development. This list
> grows with every release.

## Gameplay & economy

- **Flat worker saturation** — every mineral patch takes 3 workers and every
  gas node takes 3, with no diminishing returns beyond that.
- **Cheaper, faster gas** — tier 1–3 gas extractors cost less and build faster
  (values in `plus.yml`).
- **Advanced (tier-2) workers** — cost +25 gas and build much faster, so they
  come online early instead of being a late-game luxury.
- **Ultraload passive** — tier-2 workers move +50% faster while working inside
  20 tiles of one of their own completed town halls. Shown as a greyed passive
  button on their command card.
- **plus.yml** — the entire economy pass is configurable by editing `plus.yml`
  (mineral/gas/build-time overrides per unit).

## UI & quality of life

- **Smart Tab (SC2-style subgroups)** — Tab highlights the next unit type in a
  mixed selection and its abilities win the hotkeys; the selection itself never
  changes, so orders still apply to everyone. Shift+Tab goes back. The stock
  "hide terrain" Tab binding is replaced.
- **Idle workers** — F9 selects one idle worker per press (press again for the
  next). An SC2-style badge shows the idle worker's icon and count mid-left.
  Idle = standing still with nothing to do, including parked tier-2 workers
  (Agrileth, Talos, and friends).
- **Larva pick** — F12, type a count, Enter: exactly that many larva are
  selected through the engine's own selection routine.
- **Saturation overlay** — worker counts above your town halls and gas nodes,
  race-coloured when saturated. Only your own bases are ever labelled.
- **Nydus numbering** — each Cesiant canal/exit pair gets a number, drawn
  in-world and on the minimap at both ends. Only your network is numbered.
- **Camera labels** — C1–C8 on the minimap at the exact positions of the F1–F8
  camera hotkeys.
- **Camera pan** — CapsLock + WASD (or arrow keys) pans the camera.
- **Player zoom floor 100%** — the full observer zoom range, instead of the
  stock 200% minimum.
- **Changes panel** — Pause/Break shows this list in-game.
- **Privacy** — every piece of map text is local-only: your opponent can never
  see your labels, and you can never see theirs.

## Game modes

- **Anti Meta (46)** — per-game start randomisation: 4–8 workers and
  100–300/0–125 resources in steps of 5, symmetric across teams, from a shared
  deterministic seed.
- **Draft (47)** — a Mechabellum-style pick phase before the match.

## Netcode

- **Rollback netcode** (GGPO-style) — commands act instantly; prediction and
  rollback keep the match in sync, with hash-agreement checks every few frames.
  One-sided installs fall back to lockstep automatically.
- **CrownLink rollback channel** — a fork of CrownLink carries the rollback
  traffic next to the normal turn stream.
- **Host / Guest / Solo** — armed by the launcher with one click; Solo (no
  netcode) is the mode for skirmish vs AI and replays.
- **Bug capture** — Ctrl+F11 (or the freeze watchdog) saves a diagnostic bundle
  with an on-screen confirmation, for reporting desyncs and crashes.

## Stability & distribution

- **Crash handler** — access violations are caught and written to
  `cosmo_crash.log` with registers and the game tick.
- **temp.mpq healing** — a truncated game-data archive (from an interrupted
  rebuild) is detected and rebuilt automatically instead of breaking map loads.
- **Launcher** — the open-source Cosmonarchy Plus Launcher: install, update,
  version pick, patch pipeline, variant staging, replays, account linking, and
  one-click Play (fullscreen).
- **Packaging** — Windows installer/updater, macOS installer, and an Arch AUR
  package, all pinned so a release can never ship a plugin that does not match
  its game build.

---

*Last updated: 2026-10-24*
