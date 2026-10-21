# Status — 2026-10-21

Evidence-backed state of each request. Update this file, not the plan's prose, when a
phase lands.

## 1. Saturation overlay — model done, drawing in progress

- Model: `docs/measurements/saturation.md`, 20 host tests in `cosmo-plus-core`.
- Measured: worker flingy speeds, harvest timer 42f, delivery cadence (93–124 f solo,
  50 f with 2 workers), per-node assignment counts.
- **Finding:** functional miners all have the same speed (1280). `Talos`, `Gosvileth`,
  `Agrileth`, `Prudence`, `Dyad`, `Constituent` have flingy `top_speed = 1` in this build
  (placeholder/immobile). So a tier-1 vs tier-2 optimum split has **no speed basis** here;
  the overlay must show the truth (likely equal denominators), not 8/4.
- Drawing: in progress (separate deep task). Acceptance checklist:
  `docs/ACCEPTANCE.md` §1.

## 2. Data pass — DONE and verified in a live game

- `plus.yml` overrides applied on first tick; `cosmo_plus.log` shows 40 field writes.
- All 11 named Zerg structures match `docs/PLUS-DATA.md` exactly (verified from the
  post-override dump).
- Gas tiers: ore −100/−150/−200 and time −10/−15/−20 s applied to Sieve/Cedent/Eskigant/
  Arch, Reservoir/Aquifer/Excisant/Abutment, Wellbore/Ezigrisant.
- Bloodsummon `Capillary`/`Artery` are not in the live tier table; skipped (documented).
- **Not done:** in-place extractor upgrades. Needs a custom order/button; Zerg already
  morphs, the other races do not. Design options in `docs/PLUS-DATA.md`.

## 3. Anti-meta mode — roll logic DONE, engine application pending

- `cosmo-plus-core/src/antimeta.rs`: deterministic SplitMix roll from a u32 seed;
  workers 4–8, minerals 100–300/5, gas 0–125/5; 3 tests including 100k seeds.
- Engine side: design and addresses in `docs/antimeta-mode-notes.md`
  (`create_unit` 0x004A09D0 custom convention, `bwfunc_remove` 0x00475710, resources at
  0x0057F0F0/0x0057F120 + totals, player table 0x0057EEE0).
- Mode selection v1: `CM_PLUS_MODE=antimeta` (config); lobby template is a stretch goal —
  the `.got` type field is a **u16 at 0x40**, and `is_melee_game` rejects unknown
  (type, subtype) pairs, so a new entry needs the engine's `GameTypeList`, not just a file.

## Separation

- `netcode/` untouched by all of the above.
- `plus/` has no link-time dependency on the netcode; the only planned coupling is the
  shared RNG seed, which is already a game global (`bw::rng_seed`), so no new ABI is needed.
