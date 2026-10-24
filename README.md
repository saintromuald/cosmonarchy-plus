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
   resources in steps of 5), symmetric across teams, from the shared seed.
4. **Player zoom-out floor at full observer depth** — the widescreen build's
   `WorldZoom` lets witnesses/replays zoom from the 200% default down to 100%
   (8 wheel-notches). Plus lowers the player floor from 200% to **100%** — the
   same ladder as observer mode — with a one-time 5-byte patch of
   `apply_display_role` in `gptp.qdp`, verified against the clamp signature
   before it is written. Override with `CM_PLUS_ZOOM_PLAYER_MIN` (percent,
   100–300) in `cosmo_plus.conf`. See `docs/zoom-floor.md`.
5. **Idle-worker hotkey** — one key press selects every idle worker of the local
   player through the engine's own selection routine (network-synced). Default
   **F9** (unbound in the game and the mod); configure with
   `CM_PLUS_IDLE_WORKER_KEY` in `cosmo_plus.conf` (named keys like `F12`/`SPACE`,
   letters, digits, or a hex virtual-key code like `0x7A`).
6. **Saturation labels in the small font** — the per-node and town-hall counts
   draw with `font_base[0]` (`FONT_SMALL`) instead of the standard in-game font;
   `CM_PLUS_OVERLAY_FONT=std` restores the previous look.
7. **Larva picker** — select hatcheries, press the key (default **F12**), type a
   count (1–99, clamped to 12), Enter or the 2 s timeout → exactly that many larva
   are selected through the engine's selection routine (network-synced). No digits
   = select all. Digit keys are swallowed during entry so control groups never
   fire. Configure with `CM_PLUS_LARVA_KEY` in `cosmo_plus.conf`.
8. **SC2-style Tab subgroup cycling** — with a mixed selection (several unit types,
   or several building types on one hotkey), **Tab** cycles the *active subgroup*
   forward and **Shift+Tab** backward, the way StarCraft II's "Next Subgroup" works.
   The active subgroup's units move to the front of the selection, so their buttons
   lead the command card and win the hotkeys — press Tab until the spellcaster or
   production building you want is active, then cast/produce immediately. The
   whole selection stays selected, so move/attack orders still apply to everything
   (exactly like SC2), and a fresh selection resets the cycle. Tab is dedicated to
   this: it is unbound in stock Brood War and in the mod's hotkey layer, and the
   plugin ignores it while chat or a dialog is open, so the key is never stolen.
   Works with the mod's extended 240-unit selections. Configure with
   `CM_PLUS_TAB_KEY` / `CM_PLUS_TAB_PREV_KEY` (separate backward key) in
   `cosmo_plus.conf`; `CM_PLUS_NO_TAB_CYCLE=1` disables it.

8. **Changes-vs-OG panel** — press **Pause/Break** in a game for a compact,
   line-by-line list of what Plus changes compared with stock Cosmonarchy
   (saturation, gas tiers, advanced workers, ultraload, zoom, Tab cycling,
   camera pan, F9/F12, lobby game types, rollback, crash capture). A short hint
   shows for the first ~20 seconds of each game. Configure the key with
   `CM_PLUS_CHANGES_KEY`; the panel is drawn in the draw pass, so it is
   local-only and safe with rollback.

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
