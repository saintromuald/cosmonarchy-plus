# Saturation overlay — implementation & verification

The saturation overlay draws two `assigned/optimal` rows above each visible mineral field and
gas node. This documents exactly what is drawn, the hook used, the transform, the toggle,
and the evidence that it works.

## What is drawn

For every resource node (mineral field `176/177/178`, gas extractor `419/420/421/457`,
`110/157/149/539`, `365/364` — and the neutral geysers `188/221`), two rows of text appear
above the patch:

```
<tier-1 assigned>/<optimal>
<tier-2 assigned>/<optimal>
```

* `assigned` = the number of **completed** workers whose harvest link
  (`Unit.unit_specific.worker.current_harvest_target`) points at the node, split by the
  mod's basic/advanced worker ids (`saturation::WORKER_TIERS`). Basic = Mason/Droleth/
  Scribe/Pilgrim (tier 1); advanced = Ogun/Autocort/Talos/Gosvileth/Agrileth/Prudence/Dyad/
  Constituent (tier 2). Workers still under construction (`remaining_build_time != 0`) are
  excluded.
* `optimal` = the measured model `n* = ceil(1 + (2·D/v + δ)/42)` with `v = 5.0 px/frame`
  (the functional worker top speed `1280/256`) and `D` the straight-line distance from the
  node to the nearest completed drop-off (town hall / resource depot —
  `saturation::DEPOT_IDS = 106/215/131/154/429`).

Because every *functional* worker shares `top_speed = 1280`, `n*` does **not** depend on
tier: the two rows show the same optimum, and the split is the truth of the measurement
(≈ 3 for a typical mineral line at 200 px), not the `0/8` vs `0/4` placeholders. The six
placeholder tier-2 slots (`top_speed = 1`) are immobile and are flagged in `cosmo_plus.log`
(not on screen).

## Colour

* **White** (`TextColor::White = 0x04`) while `assigned < optimal`.
* The **owner's race colour** via `get_race_textcolor` once `assigned >= optimal`:
  Zerg purple `0x10`, Terran river-blue `0x0E`, Protoss yellow `0x03`, Askosi dark-green
  `0x18`, Bloodsummon red `0x06`, Kogri grey2 `0x1E`.

The owner is the player whose nearest completed depot serves the node (read from
`g_player_table[owner].race` at `0x0057EEE0`, race at offset `0x09`), falling back to the
local player when no depot exists. Every node is drawn in the colour of its own owner, so
allies' patches colour correctly.

## The hook — why the draw proc, not the sim tick

The overlay must never run during rollback re-simulation. It therefore hooks the **game
draw proc** at `0x004BD68D` (`g_draw_game_proc`, the operand that GPTP's
`graphics/draw_hook.cpp` also patches), not the `StepObjects` tick. At install time we read
whatever `0x004BD68D` currently points at (GPTP's `draw_hook`, since `gptp.qdp` loads before
`zz-cosmo-plus.qdp`), save it, and write our own `void __stdcall (Bitmap*, Bounds*)` hook
that chains to the saved pointer and then draws. The hook installs via
`samase_shim`'s `write_exe_memory` and logs `old -> new`:

```
[cosmo-plus] overlay: hooked game draw proc (old 0x02685260 -> new 0x79B96EF0, patch=1)
```

`0x02685260` is GPTP's `draw_hook` inside `gptp.qdp`; `0x79B96EF0` is our Rust hook in
`zz-cosmo-plus.qdp`. The pointer is stable and re-verified each run.

## Text primitives and the transform

The two rows are drawn with the 1.16.1 low-level text functions, called from Rust by address
with the same `PUSHAD`/`POPAD` wrapper `bwhelper.h` uses (register ABI, not stdcall):

| Function | Address | ABI |
|---|---|---|
| `bw_print_xy(text, x, y)` | `0x004202B0` | `eax = text`, `esi = x`, `[esp] = y` (callee `ret 4`) |
| `set_font_handle(u32*)` | `0x0041FB30` | `ecx = font` |
| `get_color_handle(TextColor)` | `0x0041F610` | `al = colour` |

`bw_print_xy` draws into `*g_current_surface` (`0x006CF4A8`), which is the **world buffer**
while the mod is magnified (its 200% player default) and the screen buffer at 100%. So
drawing at the same transform the mod's `ON_MAP` shapes use — `screen = world - camera` —
is magnified correctly by the mod's own `scale_world`, with **no zoom math of our own**.
The camera is read directly from `g_screen_x` / `g_screen_y` (`0x00628448` / `0x00628470`;
`BW_DATA(s32*, …)` makes `g_screen_x` a constant pointer *equal to* the address, so the
value is a single deref, not two). The clip box `g_draw_text` is set to the whole surface so
our labels are not clipped to the message-log box, and `font_base[1]` (`0x006CE0F8`) is the
in-game font.

## Toggle

* **Default on** when `plus.yml` exists (`saturation.enabled` defaults to `true`).
* `saturation:\n  enabled: false` in `plus.yml` turns it off.
* `CM_PLUS_SATURATION=1` / `CM_PLUS_SATURATION=0` overrides at runtime.
* **Hidden when depleted** (`resource.amount == 0`) or under construction.
* **Off-screen nodes are clipped** by `bw_print_xy` (it clamps to the current surface).
* **Fog of war is not checked**: labels are drawn for every node on the map, including
  enemy/enemy-fog nodes. This is a deliberate simplification for the first cut — the labels
  only read synced unit state, so they never desync, but a fog-aware pass (allies + witness
  only, like the mod's aura circles do with `bw::are_mutual_allies`) is a possible follow-up.
* Shown in replay/witness too (read-only; the spec's "hide in replay?" is decided *show*).

Two test-only knobs (documented, not gameplay): `CM_PLUS_SATURATION_OPT_CAP=N` caps the
per-node optimum so the white→colour flip can be forced without waiting for a saturated
base; `CM_PLUS_SATURATION_LOG=1` writes the per-node dump below.

## Verification

Build, install, boot headlessly, start the replay, screenshot, and read the log.

```bash
tools/build.sh
tools/install-plus.sh Release            # -> plugins/zz-cosmo-plus.qdp
# cosmo_plus.conf: CM_PLUS_SATURATION_LOG=1  (optional CM_PLUS_SATURATION_OPT_CAP=2)
bash ~/Projects/cosmonarchy-rollback/tools/headless_probe.sh Release \
  "490,182;940,590;320,645;350,238;940,590" "" /tmp/opencode/overlay.png 55 50
```

`cosmo_plus.log` (OCR-free evidence — the values, not pixels):

```
[cosmo-plus] overlay: enabled=true
[cosmo-plus] overlay: hooked game draw proc (old 0x02685260 -> new 0x79B96EF0, patch=1)
[cosmo-plus] overlay: functional movers — tier1 [Terran, Zerg, Protoss, Askosi], tier2 [Terran, Protoss]
[cosmo-plus] overlay: Zerg id 47 (tier 2) is immobile (speed 0.004)   # + 5 more placeholders
[cosmo-plus] overlay frame 1: 150 node(s) camera=(64,288) [...] | (192,560) 0+0=0/3 dist=202 owner=0 race=2 col=04 sat=false
[cosmo-plus] overlay frame 721: ... | (192,560) 2+0=2/3 dist=202 owner=0 race=2 col=04 sat=false
```

Observed in a real replay (camera fixed at `(64,288)`):

* 150 nodes counted; owners/races resolve to the two base players (owner 0 = Protoss
  `race=2`, owner 1 = Zerg `race=0`).
* Node `(192,560)` ramps `0 → 1 → 2` assigned; `optimal=3` from `dist=202` (`ceil(1 + 202/105)`),
  stays `col=04` (white) because `2 < 3` — the honest, non-forced truth.
* With `CM_PLUS_SATURATION_OPT_CAP=2`, the same node shows `2/2` and flips to `col=03`
  (Protoss yellow) once `assigned >= optimal` — the colour transition works.

Screenshot pixel check (A/B, overlay on vs off, same replay, `-crop 640x480+320+120`):

| metric | overlay OFF | overlay ON |
|---|---|---|
| yellow-ish text px (`r>170,g>130,b<120`) | 7147 | **7529** (+382) |
| light-grey text px (`160<r<200, g,b` high) | 658 | **871** (+213) |

The overlay adds white (`0/2`, `1/2`) and yellow (`2/2` saturated) text pixels above the
patches. The delta is small because only a handful of the 150 map nodes are inside the
camera view at any instant. Screenshots: `overlay-release.png` (real values, all white),
`overlay-cap2.png` (forced saturation, yellow labels), `overlay-off.png` (toggle off).

## No simulation writes

The overlay only reads `Unit` fields and the player table, and draws via the text
primitives. It never mutates `Unit`/`UnitExt`, the RNG, or anything in the state hash; the
per-frame cache lives in plugin memory. A run with `CM_PLUS_SATURATION=0` produces the same
gameplay (the toggle only gates drawing), and the draw hook runs in the render pass, which
rollback re-simulation never executes.
