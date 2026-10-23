# Nydus numbering — implementation notes

Each Nydus canal/exit pair gets a number, drawn above both buildings in-world and on the
minimap at their positions, so the player can tell which canal feeds which exit.

## Structure

- Pairs come from the mutual `Unit.building.nydus_exit` link (UnitLayout.h offset `0x0D0`;
  `make_nydus_exit.cpp` sets `canal->exit` and `exit->canal`). A pair is counted once, when
  visiting the member with the lower unit address (the one created first).
- Numbering is by unit-slot order (creation order): deterministic on every peer, so the
  labels never diverge.
- Cesiant ids covered: Nydus `134`, Cagrant `143`, Spraith `144`, Surkith `146`,
  Larvosk `216`, Spirtith `292`, Skortrith `293`, Ocsal `354`, Kazitrith `355`.

## Drawing

- Runs in the **draw pass** (called from `overlay`'s draw hook after the world draws),
  never during rollback re-simulation; read-only.
- In-world: `world - camera` transform, same as the saturation overlay labels.
- Minimap: screen-fixed. The minimap rect is `g_minimap_rect` (`0x00512D00`), the map size
  `0x0057F1D4`/`0x0057F1D6` (tiles), world pixels per tile `32`; the number lands at
  `rect + unit/32 * rect/map`, expressed as `screen + camera` because the draw hook runs
  with the world surface current (the mod's `scale_world` maps `world - camera` onto the
  screen).

## Tuning

- No config knobs; always on when the overlay is enabled.
- Label colour: white (`TextColor::White = 0x04`), matching the unsaturated overlay rows.
