# Player zoom-out floor — full observer-mode depth

Plus lowers the in-game player zoom-out floor from the stock 200% to **100%**,
giving players exactly the observer-mode (witness/replay) zoom depth: the same
8-notch wheel ladder observers get.

## Background

The widescreen build of `gptp.qdp` implements zoom in `systems/display/zoom.*`
(reference: `cosmonarchy-gptp`). Zoom percent is *window-relative
magnification*: 200% is the vanilla-ish default, 100% is 1:1 (deepest view).
The wheel ladder steps 12.5% per notch and has detents at 200% and 300%.

Each game, `apply_display_role()` sets the role's floor:

```cpp
g_display.set_min_zoom_percent(full_fov ? zoom::WitnessMinPercent   // 100
                                        : zoom::PlayerMinPercent);  // 200
g_zoom.begin_game();
```

- Witnesses/replays may wheel from 200% down to 100% — 8 notches.
- Stock players are held at 200% — no zoom-out at all.

Plus gives players the **same floor as witnesses: 100%**, so zooming out in a
match behaves exactly like observer mode. The witness branch is untouched.

## Mechanism

At plugin load, `zoom::install_player_zoom_floor()` (in `cosmo-plus`) locates
the loaded `gptp.qdp`, searches for the compiled `apply_display_role` branch
(signature in `cosmo_plus_core::zoom::SITE_PATTERN`), verifies the call target
is `set_min_zoom_percent` (clamp to [100, 300] + the 10000-unit world-extent
math), then rewrites the player branch's `mov eax, 200` immediate to
`mov eax, 100`. The write is read back and logged.

The patched bytes live in gptp's read-only `.text`; the rollback netcode
snapshots only writable sections, and zoom is viewer-local, so the patch is
invisible to the simulation and to rollback restore.

## Configuration

`CM_PLUS_ZOOM_PLAYER_MIN` in `cosmo_plus.conf` (or the environment) overrides
the floor, clamped to the engine's bounds of 100–300. The default is 100
(observer parity). Setting 200 restores the stock behaviour (no patch is
written).

## Verification

- Host tests: `plus/crates/cosmo-plus-core/src/zoom.rs` re-maps the widescreen
  `gptp.qdp` and asserts the site is found once, the stock immediate is 200,
  the call target contains the clamp/world-math signatures, and the encoded
  patch is `B8 64 00 00 00`. Run with:

  ```sh
  COSMO_GPTP_DLL=/path/to/widescreen/gptp.qdp cargo test -p cosmo-plus-core zoom
  ```

- In-game: `cosmo_plus.log` on a widescreen build shows

  ```
  [cosmo-plus] zoom: player floor 100% (witness floor 100%) @ 0x…; call target 0x…
  ```

  where the call target differs from the site by the verified offset to
  `set_min_zoom_percent`. On a pre-widescreen build (e.g. a backdated 19 Sep
  release) the signature is absent and the patch is skipped with a log line.

## Known limits

- The signature targets the widescreen `gptp.qdp` compiled layout. If upstream
  rebuilds `apply_display_role` differently, the patch is skipped (logged)
  rather than misapplied; update `SITE_PATTERN`/`TARGET_*_SIG` from the new
  disassembly and re-run the host tests.
- The floor is applied per game start by the game's own
  `apply_display_role`; nothing in Plus re-applies it mid-game.
