# In-place gas extractor upgrades — feasibility

Requested: every race's gas building can be upgraded in place, paying the next tier's
building cost and taking the next tier's build time.

## What the engine already does

`begin_gas_building` (`hooks/orders/building_making/building_zerg.cpp:371`, 0x004678A0)
does **not** place a separate refinery next to a geyser. It takes the geyser unit, changes
its player, and `replace_unit_with_id(target, unit_id)` — the geyser *becomes* the
extractor, at the same position, set under construction with `hitpoints = max/10`.
`unit_id_is_uncapped_gas_node` (CM/cm_ids.cpp:24) is what gates this: only the raw
`ResourceVespeneGeyser` / `ResourceVespeneRidge` may be converted. A completed extractor is
a *capped* node, so a second build on it is rejected.

## Consequence

"Upgrade in place" is exactly "let the next tier be built on a capped node". The cost and
time already come from the next tier's `units.dat` entry — which the Plus data pass has
already adjusted. No new art or animation is needed: the existing build placement preview
and construction animation already apply to gas buildings.

## Two viable implementations

1. **Allow build-on-capped-node (preferred).**
   - Intercept the build-placement validity path so a completed extractor of the same race
     and a lower tier is accepted as a target for the next tier.
   - On acceptance, run the same sequence as `begin_gas_building`: give the old unit to the
     builder's player, `replace_unit_with_id(next_tier)`, set `remaining_build_time` from
     the new entry, deduct minerals/gas, mark under construction, transfer overlays.
   - The player uses the **existing build button**; the command is a normal synced
     `GameCommand::Build`, so multiplayer/rollback are unaffected.
   - Needs a Rust hook on the build/placement function plus a small reimplementation of the
     ~20-line conversion (it is all public Unit fields).
   - Watch out: the mod's `unit_is_geyser` (0x004688B0) is already jump-patched by GPTP, so
     hook the *current* target of that address and chain, not the stock body.

2. **Generic morph button.**
   - The game has `GameCommand_BuildingMorph = 53` and `Building_Morph` (0x00423860), plus
     Zerg's `Eskigant → Excisant → Ezigrisant` morph chain as a reference.
   - Add/replace a button in the extractor's button set that issues the morph command for
     the next tier. Requires writing the button-set array and matching the mod's dynamic
     button system (`hooks/interface/buttonsets.cpp`), which is C++-side today.
   - More moving parts than option 1, and non-Zerg morph visuals may look wrong.

## Why this is not in the first drop

Option 1 is a correct, small feature but it touches the build-placement path, which is
also where the AI and the mod's own gas logic live. It needs its own verification pass
(human build, AI build, cancel, refund, replay, rollback) before it ships. The data costs
and times are already live and verified; the upgrade is the only missing half.
