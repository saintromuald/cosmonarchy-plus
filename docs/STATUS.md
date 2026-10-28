# Status — 2026-10-21

Evidence-backed state of each request. Update this file, not the plan's prose, when a
phase lands.

## 0. SC2-parity session (2026-10-27) — Tab/idle/queue/larva/anti-meta rework

Five asks, designed under two external critique agents (two rounds each) and
implemented:

- **Tab cycle (caster/ability caveat, SC2 parity).** Core model rewritten:
  run keys = (derived id, ordinal) computed from the CANONICAL selection
  (`ccmu_extender.new_active_selection` via the exe pointer at 0x006284B8) —
  stable across rotations/canonical re-copies/head-of-run deaths; membership
  changes keep the active subgroup (SC2 semantics); a detour at the engine's
  selection entry (0x0046FA00, prologue-replaying, chain-jump) resets the
  cycle on every fresh selection (recall included). Burrowed/unburrowed are
  separate subgroups (derived-id fold, orders {0x74,0x75}). Adapter:
  modifier guards (Ctrl/Alt+Tab pass through), auto-repeat swallow, saturating
  press queue, `apply_display` before `poll`, worthy-only indicator, morphing
  runs show the type's card (buttonset id from the unit id), `CARD_READY`
  gating (fully inert without the fork's card patch), portrait restore on
  deactivate, bounded client-table rotation (never exceeds the live count,
  lead filtered to client membership). **Ability orders on a mixed selection
  are narrowed to one subgroup** (`tab_cycle::narrow_targets`, peer-symmetric:
  canonical state only — never the rotated client table or the viewer-local
  model; the command hook installs whenever smart-cast OR the tab cycle is
  armed). 79 core tests.
- **Idle workers.** SC2-style screen-fixed counter (worker icon +
  "Idle Workers: N", icon size-compensated per zoom) above the minimap.
  Predicate: BUSY_ORDERS {0x19,0x1A,0x1E,0x1F,0x20,0x21,0x22,0x23,0x25,0x2A,0x2B}
  exclude constructing/repairing/morphing workers (the Mason build-window
  bug), queued construction orders count as busy (guarded order-queue walk),
  immobile placeholder tier-2 workers excluded (live `top_speed <= 1`).
  One merged count+icon pass per frame.
- **Production queue 2x.** `CM_PLUS_QUEUE_ICON_SCALE=2` (config-file only):
  hides the mod's 28px production slot widgets every frame (g_statres_dialog
  0x0068C234 child walk, ltrb-bounds pitch/row predicate, visible bit 0x8)
  and draws the local player's 2x queue (two rows, cmdicons, counts,
  draw_box progress bars with the mod's guards) camera-relative top-left.
  `g_production` resolved by signature (`B8 E4 00 E4 00 BF <imm>`) — verified
  in BOTH shipped builds (Prerelease RVA 0x126B61 / Original RVA 0xA2D51) —
  live imm accepted only inside the module's committed span, base-delta
  fallback. Replays untouched; `CM_PLUS_QUEUE_TEST` forces synthetic rows for
  the screenshot harness (never writes the live arrays).
- **Larva Ctrl+Space.** `parse_binding` chords (`CTRL+SPACE`,
  `CTRL+SHIFT+<key>`, `ALT+<key>`; required-held semantics); default
  `CTRL+SPACE`; auto-repeat never toggles; chat/modal guard; Ctrl+digits pass
  through while entering; `cosmo_plus.conf.example` documents it. Verified
  LIVE in-game: the chord opened the entry window.
- **Anti-meta as a real mode.** Roll re-applies idempotently on sim frames
  1..=4 (seed cached per game; resources + announce on frame 1 only — the
  hook runs post-StepObjects, later overwrites would eat mining income);
  resources apply to EVERY active player (team symmetry fixed); replays
  skipped. Mod-source patch (GameType::AntiMeta=46, GameKind::AntiMeta,
  get_game_kind, no-op mode-init case, cm_utils label/filename cases) applied
  to the local reference checkout and saved as
  `docs/patches/gptp-antimeta-mode.notes` (with the GameKind audit and the
  peer-gate warning: the template must stay OUT of the mod's mpq).
  `docs/antimeta-mode-notes.md` rewritten to match the new timing model.

**Verified:** 79 core tests; cross build; `verify-plus.sh` green (boot report
+ selftest 14/14, key-collisions none, larva armed as CTRL+SPACE, selection
detour installed, command hook narrowing armed); live on the widescreen
Prerelease build: idle counter drew (n=6), Ctrl+Space opened the entry
window.

**Blocked / pending live verification (harness degraded today, pre-existing —
reproduces with the pre-change build):** tab cycling with a real mixed
selection, subgroup-scoped ability execution, queue 2x render, anti-meta in a
live lobby, and the two-peer `desyncs=0` test. The canonical-selection
resolution is unverified on the widescreen build (it logs the pointer value
on failure — see `cosmo_plus.log`); if the extender does not rewrite
0x006284B8 there, the cycle stays inert loudly and the fallback plan is the
extender's own anchor immediates.

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

## 5. Idle-worker hotkey + badge — DONE (hook capture, configurable key, HUD badge)

- **Why F9 never fired:** the first build polled `GetAsyncKeyState` from the draw pass. The
  game's own hotkey layer consumes function keys before that poll sees a tap, so the press
  never reached the plugin (`cosmo_plus.log` had the install line but no `selecting` line).
  The key is now captured by a thread `WH_KEYBOARD` hook — the same mechanism that already
  works for Tab subgroup cycling — which queues the press and lets the draw pass do the
  selection (still rollback-safe: no selection during re-simulation).
- **Configurable key:** `CM_PLUS_IDLE_WORKER_KEY` in `cosmo_plus.conf`. `parse_key` now
  accepts the punctuation keys by character (`,`) or name (`COMMA`, `PERIOD`, `SLASH`, …)
  in addition to F1–F12, letters, digits and hex VK codes. The key is swallowed in-game, so
  a control group bound to it no longer fires; Ctrl/Alt/Shift + key is left to the game, and
  the hook is inert in chat and modal dialogs. One press selects one idle worker, the next
  press selects the next (SC2-F1 style).
- **Badge:** anchored to the live minimap rect (above the terrain/"Hide Terrain (Tab)"
  button, left edge aligned with it) instead of a hard-coded screen spot, and drawn in
  screen coordinates — the badge used to be at raw world coordinates, so it scrolled with
  the view (adding the camera to compensate puts it off the viewport; `bw_print_xy` is
  screen-space on the draw-hook surface). The count is drawn first, the command-card icon
  (nearest-neighbour scaled 64×60 → 24×22) last, so the number survives any icon-blit
  quirk. `CM_PLUS_IDLE_BADGE_X/Y` still override the position.
- **Verified:** `cosmo_plus.conf` armed (`armed from cosmo_plus.conf: 1 setting(s)`),
  `idle workers: hotkey , (VK 0xBC)`, `keyboard hook installed`, and an xdotool `,` press
  in a live game logged `idle workers: selecting 1 of 6 (index 0)`. The minimap rect read
  live from the running game (`6,348,133,475`) puts the badge at screen `18,280`.

## 6. Development infrastructure — DONE and verified end to end

- **Fake-engine core.** `cosmo-plus-core/src/idle.rs` holds the idle predicate behind a
  `UnitView` trait; the engine crate adapts `bw_dat::structs::Unit` through a local
  wrapper. 67 host tests cover every branch of the predicate — no game boot needed.
- **Self-verifying integration layer.** `cosmo-plus/src/state.rs` resolves and
  plausibility-checks every fixed global at boot, tracks feature/hook/config status, and
  writes `cosmo_plus.state.json` (boot stage, then again at the first game tick with
  strict checks). The idle badge reads the *validated* minimap rect and disarms loudly on
  a bad address.
- **Verification harness.** `tools/verify-plus.sh` = core tests → cross build → install
  into the dev prefix (cosmonarchy-agent2, never the player's) → headless boot on a
  private Xvfb → boot-report validation → plugin self-test verdict
  (`CM_PLUS_SELFTEST=1`, 14 checks, PASS/FAIL file). Green on the player build and the
  dev build.
- **Dev REPL.** `CM_PLUS_REPL=1` polls `cosmo_plus.cmd` (~2 Hz); commands: badge
  on/off/pin/default, transient icon/text probes on the live frame, `idle` (queues a
  selection), `state`, `frame`, `units` (per-player), `reload-config` (in-place
  re-arming of every config-driven feature). `tools/cosmo-repl` drives it.
- **Hot reload.** Config-only changes reload in place; code changes use
  `tools/cosmo-restart.sh` (kill + relaunch, no menu clicking). In-place DLL swap is
  blocked on the game's closed-source hotloader (loads plugins once per boot) — see
  `docs/dev-loop.md`.
- **Hook ownership.** Every hook is registered in the state report; the draw-proc
  install audits that the pointer it chains lives inside `gptp.qdp`'s image (verified:
  "draw proc owned by gptp.qdp (expected)"); the self-test requires the three boot-time
  keyboard hooks. Contract in `docs/hook-order.md`.

### Per-plugin verification (headless solo match, dev prefix)

| Plugin | Verified |
| --- | --- |
| `_qdp-hotloader.qdp` | all plugins loaded; match ran |
| `gptp.qdp` | samase api complete (selftest), draw proc owned by gptp, zoom site found+patched on the player build |
| `zz-cosmo-plus.qdp` | idle selection logged in-game, badge + REPL probes rendered (screenshot), overlay labels drawn, hooks 5/5, selftest 14/14 |
| `zz-cosmo-rollback.qdp` | loads, relocates `unit_finder_x/y` inside gptp.qdp (its core mechanism) |
| `aise.qdp` | AI ran: computer-player units present, sim advanced (147 → 152 units over 45 s) |
| `isdbg.qdp` | "Executable patched successfully… Listening for errors" |
| `fastmenus/mtl/vx4ex/aize_debug.qdp` | loaded with the game; menus navigated, match ran (their effects are in-game UX, exercised by the menu walk + match) |

**Camera labels — enabled on the shipped gptp, no rebuild.** The first attempt added a
`cosmo_camera_screens` export to a fresh gptp.qdp CI build, but that build was associated
with the game freezing at match start, and gptp exports nothing by design. The labels now
resolve `hotkeys.screens` by **signature scan** (the same technique as the zoom patch):
the F1-F8 handler's indexed access carries the array's imm32, which the game relocates
after plugin install, so it is read lazily at first draw. Verified against live game
memory (signature @ gptp+0x76f93, imm = loaded screens address); the boot report arms
`camera-labels` on the shipped build. Fork branches `plus/camera-screens-export` /
`plus/no-export-control` (both built green in CI) are left for reference.

## Separation

- `netcode/` untouched by all of the above.
- `plus/` has no link-time dependency on the netcode; the only planned coupling is the
  shared RNG seed, which is already a game global (`bw::rng_seed`), so no new ABI is needed.
