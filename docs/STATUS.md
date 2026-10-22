# Status — 2026-10-21

Evidence-backed state of each request. Update this file, not the plan's prose, when a
phase lands.

## 1. Saturation overlay — DONE and verified in-game

- Model: `docs/measurements/saturation.md`, 25 host tests in `cosmo-plus-core`.
- Measured: worker flingy speeds, harvest timer 42f, delivery cadence (93–124 f solo,
  50 f with 2 workers), per-node assignment counts.
- **Finding:** functional miners all have the same speed (1280). `Talos`, `Gosvileth`,
  `Agrileth`, `Prudence`, `Dyad`, `Constituent` have flingy `top_speed = 1` in this build
  (placeholder/immobile). So a tier-1 vs tier-2 optimum split has **no speed basis** here;
  the overlay shows the truth (equal denominators), not 8/4.
- Drawing: `overlay.rs`, hooked at the game draw proc (draw pass, not simulation), two
  `assigned/optimal` rows per node, white → owner's race colour when saturated.
- Verified: node counts/optima in `cosmo_plus.log`; A/B pixel diff; screenshots
  `/tmp/opencode/overlay-release.png`, `overlay-cap2.png`, `overlay-filtered.png`.
- Hardened after review: only nodes within 15 tiles of a depot are shown (no "/20" noise)
  and fogged nodes are skipped.
- Known remaining: rows are unlabelled (top = tier 1, bottom = tier 2); gas nodes use the
  same mineral-derived optimum; no OCR test (log + pixel diff instead).

## 2. Data pass — DONE and verified in a live game

- `plus.yml` overrides applied on first tick; `cosmo_plus.log` shows 40 field writes.
- All 11 named Zerg structures match `docs/PLUS-DATA.md` exactly (verified from the
  post-override dump).
- Gas tiers: ore −100/−150/−200 and time −10/−15/−20 s applied to Sieve/Cedent/Eskigant/
  Arch, Reservoir/Aquifer/Excisant/Abutment, Wellbore/Ezigrisant.
- Bloodsummon `Capillary`/`Artery` are not in the live tier table; skipped (documented).
- **Not done:** in-place extractor upgrades. Feasibility in
  `docs/extractor-upgrade-notes.md`: the engine already replaces the geyser unit with the
  extractor in place, so "upgrade" is "allow the next tier to be built on a capped node";
  needs a build-placement hook and its own verification pass. Zerg already morphs.

## 3. Anti-meta mode — roll + engine application DONE, lobby game type DONE

- `cosmo-plus-core/src/antimeta.rs`: deterministic SplitMix64 roll from a u32 seed;
  workers 4–8, minerals 100–300/5, gas 0–125/5; pure helpers (`racial_worker_id`,
  `racial_town_hall_id`, `worker_adjustment`) with host tests.
- Engine side `cosmo-plus/src/antimeta.rs`, applied once **per game** (game-pointer +
  frame-reset guard; replays are skipped so playback is never rewritten):
  - activation: **lobby game type first** — `*g_game_type == 46` (the `Anti Meta(1).got`
    template) enables the mode with no config; `CM_PLUS_MODE` overrides (explicit
    on/off); `plus.yml` `mode: antimeta` is the last fallback; default off. Resolved
    state + game type logged.
  - seed = the game's synced RNG seed (`api.rng_seed` → `g_rng_seed` 0x0051CA14), read once;
    identical on both peers at game start.
  - per active player (Human/Computer, completed town hall), in fixed order 0..7:
    count completed tier-1 workers, remove excess (furthest first, `silent_remove` =
    `UserActionFlags_SilentRemove` + `bwfunc_remove` 0x00475710), spawn missing near the
    town hall (`create_unit` 0x004A09D0 + `update_unit_stats_and_finish_building` 0x004A01F0
    + `show_finished_unit` 0x0049EC30 + `update_unit_strength` 0x0049FA40, retrying a ring of
    offsets on collision), and set `g_game_*` / `g_total_*` (0x0057F0F0/0x0057F120 +
    0x0057F180/0x0057F150) to the rolled minerals/gas.
  - announces the roll via `api.print_text` (falls back to log only).
- **Lobby entry (v2).** `tools/make-template.py` generates `Anti Meta(1).got` (name
  "Anti Meta", game type 46, subtype 1; all other settings inherited from `Melee(1).got`).
  The engine's template loader builds `g_game_type_list` from `templates.lst`, so the file
  shows up in the Create Game dropdown with **no engine change** — verified in the 1.16.1
  disassembly (loader 0x4AB750, list insert 0x4AB150, dropdown `compile_list` 0x4AAD00
  filters subtype==1, `is_melee_game` 0x4AAFA0 checks the `.got` mode byte at 0x49, which
  is also `victory_condition == Melee`). Peers without the template are booted by the
  map-authorisation lookup (0x4AAEA0) → vanilla clients cannot join an Anti Meta lobby.
  `cosmo-variant` re-stages the template + `templates.lst` line on every Plus launch (the
  release tree is `git reset --hard` at launch, so the files cannot live there).
  Known cosmetic gaps (gptp-side, documented): blank "Game Mode" label in the chat room
  (magic_enum has no 46) and Discord presence shows "Custom".
- Mode selection v1 (config only) still works: `CM_PLUS_MODE=antimeta` or `plus.yml` mode.
- Verification: `tools/build.sh` passes. Real skirmish (Pagan Imperium, 1v1):
  `seed=0x4c2ddafb roll workers=8 minerals=120 gas=10`; both players `6 -> 8 (added 2)`;
  on-screen 120 minerals / 10 gas and 8 workers; overlay still drawn. Replay boot with the
  mode on logs `replay detected -> skipped` and changes nothing.
  See `docs/antimeta-mode-notes.md` §Implemented.
- **Still to verify in a live lobby:** the dropdown entry, one roll on both peers via
  CrownLink (`desyncs=0`), and the vanilla-client boot. The mechanism is in place end to
  end; two items are pending real-world confirmation:
  - **Template visibility in the engine's list.** Registry-selection tests on this machine
    showed the game's template list follows `templates.lst` edits but did **not** pick up
    the added `.got` (a `Custom Type` registry value of `Anti Meta` fell back to the
    default while `Massacre`/`Mindshare` matched). The `.got` was verified packed into
    `temp.mpq` (byte-level), so the open question is whether the protected hotloader
    resolves *new* files by hash. `CM_PLUS_DUMP_GAME_TYPES=1` + the template-loader hooks
    (0x004DB046/0x004BAC8E) answer this from inside the engine: the next game boot logs
    the full `g_game_type_list`. If `Anti Meta` is absent there, the template must be
    shipped through Fraud Launcher 2 (the release repo commit is prepared: branch `plus`,
    `311e7cc` in the local cosmonarchy-bw-release checkout), not just staged as a loose
    file.
  - **Full CrownLink lobby test** with the mode selected from the dropdown.

## 4. Player zoom-out floor — full observer depth — DONE (observer parity: 100%)

- `cosmo-plus-core/src/zoom.rs`: pure signature search + encoding; 3 host tests, one of
  which re-maps the widescreen `gptp.qdp` (run with `COSMO_GPTP_DLL=… cargo test -p
  cosmo-plus-core zoom`).
- `cosmo-plus/src/zoom.rs`: locates the loaded `gptp.qdp`, verifies the compiled
  `apply_display_role` branch and its `set_min_zoom_percent` call target, rewrites the
  player immediate 200 → 100 (`B8 64 00 00 00`), reads back, logs.
- Players now wheel through the **same 8-notch ladder as witnesses/replays** (100% floor =
  observer-mode depth). `CM_PLUS_ZOOM_PLAYER_MIN` env/conf override, clamped 100–300.
- Mechanism verified on the 20 Oct release (widescreen `gptp.qdp`, 6,075,904 bytes) via
  headless probe: the patch lands at the compiled layout (site = module base + 0xb49dd,
  target = base + 0xb4730) and the write reads back. Overlay hook, data pass (11
  overrides) and rollback pool relocation all still work on the widescreen build. See
  `docs/zoom-floor.md`.

## Separation

- `netcode/` untouched by all of the above.
- `plus/` has no link-time dependency on the netcode; the only planned coupling is the
  shared RNG seed, which is already a game global (`bw::rng_seed`), so no new ABI is needed.
