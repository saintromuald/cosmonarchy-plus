# Cosmonarchy Plus

A gameplay fork of [Cosmonarchy BW](https://fraudsclub.com/cosmonarchy-bw/) built on top
of the rollback netcode in this repository's `netcode/` submodule.

Two products, deliberately separate:

- **`netcode/`** — the rollback client and QDP plugin, unmodified by Plus. This is the
  piece to share with upstream; it has no gameplay knowledge.
- **`plus/`** — the Cosmonarchy Plus QDP plugin and its data/config. Gameplay and QoL
  only; it never depends on netcode internals.

The only contract between them is optional and by export name:
`cosmo_rollback_match_seed()` (when present) gives Plus the shared match seed it needs
for deterministic start randomisation. Absent, Plus falls back to a seed derived from
the map checksum.

**Plus clients only play Plus clients of the same build.** A Plus replay will not
replay on vanilla Cosmonarchy.

## Features

1. **Saturation overlay** — per-patch tier-1/tier-2 worker counts with a race-coloured
   "saturated" state; empirically calibrated from worker speed.
2. **Economy data pass** — gas-extractor tier costs/build times, named structure costs,
   in-place extractor upgrades (see `docs/PLUS-DATA.md`).
3. **Anti-meta mode** — per-game start randomisation (4–8 workers, 100–300/0–125
   resources in steps of 5), symmetric across teams, from the shared seed. The
   roll re-applies idempotently across the first four sim frames so rollback
   re-simulation converges identically on every peer. Selectable in the lobby's
   **Anti Meta** game type (46); the mod-side enum/label patch lives in
   `docs/patches/gptp-antimeta-mode.notes` for upstream.
4. **Player zoom-out floor at full observer depth** — the widescreen build's
   `WorldZoom` lets witnesses/replays zoom from the 200% default down to 100%
   (8 wheel-notches). Plus lowers the player floor from 200% to **100%** — the
   same ladder as observer mode — with a one-time 5-byte patch of
   `apply_display_role` in `gptp.qdp`, verified against the clamp signature
   before it is written. Override with `CM_PLUS_ZOOM_PLAYER_MIN` (percent,
   100–300) in `cosmo_plus.conf`. See `docs/zoom-floor.md`.
5. **Idle-worker hotkey** — one key press selects the next idle worker of the local
   player through the engine's own selection routine (network-synced); an SC2-style
   counter — worker icon + "Idle Workers: N" — is fixed on screen above the minimap
   (the SC2 lower-left corner region), size-stable at every zoom. Workers busy
   constructing/repairing/morphing or waiting on queued construction do NOT count
   as idle (SC2 5.0.14 parity — this fixes Masons being counted idle during the
   build window), and the immobile placeholder tier-2 workers are excluded.
   Default **F9** (unbound in the game and the mod); configure with
   `CM_PLUS_IDLE_WORKER_KEY` in `cosmo_plus.conf` — named keys (`F12`, `SPACE`,
   `COMMA`, …), the punctuation character itself (`,`), letters, digits, or a hex
   virtual-key code like `0x7A`. The key is swallowed in-game (so a control group
   on the same key does not fire); Ctrl/Alt/Shift + key still goes to the game.
6. **Saturation labels in the small font** — the per-node and town-hall counts
   draw with `font_base[0]` (`FONT_SMALL`) instead of the standard in-game font;
   `CM_PLUS_OVERLAY_FONT=std` restores the previous look.
7. **Larva picker** — select hatcheries, press the chord (default **Ctrl+Space**;
   hold Ctrl, press Space, release Ctrl), type a count (1–99, clamped to 12),
   Enter or the 2 s timeout → exactly that many larva are selected through the
   engine's selection routine (network-synced). No digits = select all. Digit
   keys are swallowed during entry so control groups never fire — Ctrl+digits
   still reach the game. Configure with `CM_PLUS_LARVA_KEY` in
   `cosmo_plus.conf`: any bare key (`CAPSLOCK`, `F12`, …) or a chord
   (`CTRL+SPACE`, `CTRL+SHIFT+<key>`, `ALT+<key>`).
8. **SC2-style Tab subgroup cycling** — with a mixed selection (several unit types,
   or several building types on one hotkey), **Tab** cycles the *active subgroup*
   forward and **Shift+Tab** backward, the way StarCraft II's "Next Subgroup" works.
   With the Plus caveat, Tab visits ONLY spellcasters and units/buildings with
   special abilities (pure production buildings and attack-only units are not Tab
   stops — SC2's own "abilities raise subgroup priority" principle). The active
   subgroup's units move to the front of the selection, so their buttons lead the
   command card and win the hotkeys — press Tab until the spellcaster or
   production building you want is active, then cast/produce immediately. Ability
   orders on a mixed selection act on one subgroup (SC2 subgroup scoping); the
   whole selection stays selected, so move/attack orders still apply to everything
   (exactly like SC2), a fresh selection resets the cycle, and burrowed/unburrowed
   units are separate subgroups (SC2 4.9.1). Tab is dedicated to this: it is
   unbound in stock Brood War and in the mod's hotkey layer, and the
   plugin ignores it while chat or a dialog is open, so the key is never stolen.
   Works with the mod's extended 240-unit selections. Configure with
   `CM_PLUS_TAB_KEY` / `CM_PLUS_TAB_PREV_KEY` (separate backward key) in
   `cosmo_plus.conf`; `CM_PLUS_NO_TAB_CYCLE=1` disables it.
9. **Global production queue with config-driven 2x icons** —
   `CM_PLUS_QUEUE_ICON_SCALE=2` (config file ONLY — there is no in-game checkbox)
   hides the mod's small top-bar production icons and draws the local player's
   queue with 2x icons at the top-left of the view (two rows, icons + counts +
   progress bars, from the mod's own `g_production` arrays). Unset/0 keeps the
   mod's small queue untouched; replays keep the mod's per-player queue.

8. **Changes-vs-OG panel** — the in-game menu (**F10**) has a dedicated
   **Plus Changes (P)** row (a real control added to `gamemenu.bin` in the
   multiplayer-free slot above Options): choose it (or press **P**) for a
   compact, line-by-line list of what Plus changes compared with stock
   Cosmonarchy (saturation, gas tiers, advanced workers, ultraload, zoom, Tab
   cycling, camera pan, F9/F12, lobby game types, rollback, crash capture).
   The plugin wires the row and hides it in single player, where Save Game
   uses that slot; older dialog templates fall back to relabelling Help.
   **Pause/Break** toggles the same panel anywhere, and a short hint shows for
   the first ~20 seconds of each game. Configure the panel key with
   `CM_PLUS_CHANGES_KEY`; the panel is drawn in the draw pass, so it is
   local-only and safe with rollback.

## Development loop & verification

```
tools/verify-plus.sh             # host tests + cross build + headless boot + self-test
tools/cosmo-check.sh [build]     # latest boot report (globals, features, hooks)
tools/cosmo-repl "badge 40 280"  # poke a running game (CM_PLUS_REPL=1)
tools/cosmo-restart.sh [build]   # fast relaunch after a code change
```

See `docs/dev-loop.md` (which loop to use when) and `docs/hook-order.md`
(ownership contract between gptp, Plus and the rollback plugin). The plugin
writes `cosmo_plus.state.json` every boot: every fixed global with a
plausibility verdict, every feature's armed/disarmed status, config keys and
hook owners — a boot report diff is how regressions are found.

## Build & install

```
tools/build.sh                 # cargo xwin, i686-pc-windows-msvc
tools/install-plus.sh Release  # copies to plugins/zz-cosmo-plus.qdp
```

## Distribution

Players never touch git, plugins, or folders. CI cuts one release per version
(see `docs/RELEASING.md`); each platform has a one-click installer that also
updates, and none of them touch an existing Cosmonarchy install:

- **Windows** — `CosmonarchyPlus-Setup-<v>.zip` (source: `dist/windows/`)
- **macOS** — `CosmonarchyPlus-Mac-Setup-<v>.zip` (source: `dist/macos/`)
- **Arch** — `cosmonarchy-plus-bin` on the AUR (source: `packaging/aur/`)

The Wine work needed to run the game (Windows 10 mode, `ddraw=native,builtin`
so cnc-ddraw loads, cnc-ddraw next to the exe) is applied automatically by
`packaging/common/provision-prefix.sh` on both Linux and macOS.

## Status

See `docs/PLAN.md` for the phased plan, the feasibility matrix, and what is still
blocked. Measurements and their method live in `docs/measurements/`.
