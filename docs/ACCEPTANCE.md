# Acceptance criteria — what "done" means for each request

Use this to judge the implementation, not the intent. Anything not demonstrated by a
command/output here is not done.

## 1. Saturation overlay

- [ ] For a visible mineral field, the overlay draws **two** lines: tier-1 `assigned/optimal`
      and tier-2 `assigned/optimal` (e.g. `0/8`, `0/4`), above the patch.
- [ ] The optimal numbers are derived from **measured** worker behaviour: the
      `docs/measurements/saturation.csv` contains raw delivery timestamps (or counts per
      fixed window) for each worker tier at several worker counts, and
      `docs/measurements/saturation.md` shows the derivation and the fit.
- [ ] Tier-1 and tier-2 optima differ if their speeds differ; the difference is explained by
      the measured speed/trip time, not asserted.
- [ ] Mixed workforces: the overlay states what the current mix needs to saturate
      (the combined rule is documented and tested).
- [ ] Text is **white** while under-saturated and switches to the **owner's race colour**
      when saturated (`get_race_textcolor`: Zerg purple, Terran river blue, Protoss yellow,
      Askosi dark green).
- [ ] Hidden when the node is not visible / depleted; off by default? (decide and document)
- [ ] Does not alter the state hash: run a replay with the overlay on and off, compare
      `cosmo_rollback.log` hashes.
- [ ] `tools/build.sh` passes and `cosmo_plus.log` shows the feature build id.

## 2. Data pass

- [ ] `cosmo_plus_dump.csv` lists id, name, ore, gas, build time for the local build.
- [ ] The 11 named Zerg structures match `docs/PLUS-DATA.md` exactly (ore/gas).
- [ ] Gas extractors: every tier-1/2/3 structure for every race present has ore reduced by
      100/150/200 and build time by 10/15/20 s, clamped at 0, and the resulting table is in
      the dump.
- [ ] In-place upgrade exists for each race's extractor and costs/times the next tier's
      building; Zerg morphs keep working; the upgrade is deterministic.
- [ ] The overrides apply on both peers identically and before any build is possible
      (verified by starting a game and opening a build button in the first second).
- [ ] `plus.yml` is the only place values live; deleting it restores vanilla.

## 3. Anti-meta mode

- [ ] Selectable as its own mode: the **Anti Meta** entry in the Create Game game-type
      dropdown (staged `Anti Meta(1).got`, game type 46, via the `cosmonarchy-plus`
      launcher; `CM_PLUS_MODE`/`plus.yml mode:` remain as explicit override/fallback).
- [ ] One roll per game, applied to **all** active players identically:
      workers ∈ {4..8}, minerals ∈ {100..300} step 5, gas ∈ {0..125} step 5.
- [ ] The roll is from the shared/synced RNG (or otherwise identical on both peers); two
      clients in a real match report the same numbers.
- [ ] The roll is announced in-game (centre message) so players can adapt.
- [ ] Works with the rollback plugin armed: `tools/crownlink_match.sh` shows `desyncs=0`
      with the mode active.
- [ ] Replays reproduce the same start.
- [ ] A client without the staged template is booted from an Anti Meta lobby (no silent
      mixed-mode match).

## Cross-cutting

- [ ] `netcode/` has no Plus-specific code and still passes its own gate.
- [ ] Both plugins load together (`cosmo_plus.log` and `cosmo_rollback.log`).
- [ ] Every numeric claim in the docs is traceable to a dump, a measurement, or an equation.
