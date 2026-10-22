# Draft mode (Mechabellum-style roguelite) — design notes

Goal: every 2 minutes of game time, both players are shown the **same 4 options** and each
picks one. Options depend on the **race pair** (yours + your opponent's) and on **how late
the game is**. After minute 8 the pool grows unit drops, items and temporary buffs. Same
options for both players = the draft is *shared* (both read the same board, like
Mechabellum's shared shop); the picks are private until both apply.

This is a plan, not an implementation. Everything is grounded in what Plus already does
(the anti-meta spawn/resource code, the overlay draw pass, the template system from
`docs/antimeta-mode-notes.md`) and in the samase_shim `PluginApi` (verified against the
vendored checkout).

---

## 1. Mode identity

A **new game type, "Draft" (type 47)**, added exactly like Anti Meta (46): an
`Draft(1).got` template (copy of Melee, mode byte 1) + `templates.lst` entry, detected by
Plus from `*g_game_type == 47`. Type 47 because 45 = Mindshare, 46 = Anti Meta.

Open choice for you:
- **A. Draft includes the anti-meta start roll** (4–8 workers, 100–300/0–125) — very
  roguelite: random start + periodic draft. Recommended.
- B. Draft starts standard, anti-meta stays separate.

Either way the draft loop below is identical.

## 2. The draft loop

- **Cadence:** first draft at **minute 2** of game time, then every 2 minutes. Game time
  = `game->frame_count / 24`; frames are synced, so every peer triggers the same draft at
  the same frame. No wall clock anywhere.
- **Window:** the 4-option panel shows for **30 seconds** (non-blocking v1 — the sim keeps
  running; pausing is a later option, see §7). Players pick with **hotkeys 1–4** (or
  click, later). No pick within the window = no effect that round (v1; auto-pick is a
  toggle later).
- **Reveal:** both players see the same 4 options the moment the window opens. Picks are
  applied **simultaneously at window close** (deterministic fixed order, players 0..7), so
  nobody gains from picking late.

### Phases (pool tiers)

| Draft # | Game time | Tier | Pool contents |
|---|---|---|---|
| 1–3 | 2, 4, 6 min | **Early** | economy, scouting, cheap tempo units, one permanent upgrade |
| 4–5 | 8, 10 min | **Mid** | unit drops (squad of 2–4 units at your town hall), items, economy swings |
| 6+ | 12+ min | **Late** | temporary buffs (60–120 s), big drops, tech snipes, "ult" options |

The user's rule "past minute 8 you can start having unit drops, items, temporary buffs"
maps to: drops/items enter at draft 4 (minute 8); temporary buffs at draft 6 (minute 12);
each tier keeps a couple of lower-tier options so the pool stays mixed (e.g. Mid pool is
~40% drops, ~30% items, ~20% economy, ~10% early leftovers).

## 3. Option pool design

**Symmetric pool rule:** the pool is a function of the **unordered race pair**
`{your race, opponent race}` + tier + draft index + match seed. Both players compute the
same pool. A (T vs Z) game and a (Z vs T) game draw from the same table.

**Selection:** pool tables are YAML data (`plus.yml` section `draft:`), like the existing
`overrides:` — every option is a reviewable line of config, and balance tuning never
touches code. Each option: `id`, `tier`, `race_pair` (or `*`), `effect` (typed), `weight`.

**Deterministic draw:** `seed64 = splitmix(match_seed ^ mix(draft_index))`; draw 4 options
from the tier pool by weight, without replacement. Both peers derive the identical tuple.
The draw is pure Rust in `cosmo-plus-core` (host-testable, like `antimeta::roll`).

### Example early-tier options (Terran vs Zerg, minute 2)

| Option | Effect |
|---|---|
| Prospectors | +150 minerals now |
| Deep Recon | Reveal enemy town-hall area for 8 s |
| Vanguard | Spawn 2 Masons immediately (T) / 2 Droleths (Z) |
| Blueprints | Refund 50% of your next structure (one-shot token) |

### Example mid-tier (minute 8, drops/items)

| Option | Effect |
|---|---|
| Supply Drop | Squad drop: 4 tier-1.5 combat units at your town hall |
| Assault Pod | One siege unit + 2 escorts, dropped at your *expansion* (or town hall if none) |
| Targeting Core | Item: next fight's first volley +20% damage (token) |
| Reinforced Plating | Permanent: your structures +150 HP |

### Example late-tier (minute 12, temporary buffs)

| Option | Effect |
|---|---|
| Bloodlust | Temporary: all your units +1 weapon upgrade level for 90 s |
| Overclock | Temporary: +25% attack cooldown reduction for 60 s |
| Warp Recall | Instant: full heal + refill energy on all your units |
| Leviathan | Ult drop: one capital unit at your town hall |

(These are sketches to illustrate the shape — the real tables are yours to curate; see §7.)

## 4. Effect catalog, by implementation cost

Cost is judged against what Plus already does. "Proven" = same code path already shipped
in anti-meta/overlay/data.

| Effect | Mechanism | Cost |
|---|---|---|
| Spawn units (any id, N, at town hall/expansion) | `create_unit` + finish/show/strength chain — **proven** (`antimeta::spawn_workers`) | Easy |
| Resource injections | write `g_game_minerals/g_total_*` — **proven** (`set_resources`) | Easy |
| Add/remove workers | **proven** | Easy |
| Instant heal / energy refill | iterate units, write `hp`/energy fields (in snapped pools) | Easy |
| Temporary ±upgrade level (weapons/armour) | write `unit->upgrade_levels` (+ timer + restore via state blob) — engine maths already applies upgrade levels everywhere | Easy–Medium |
| Temporary speed/cooldown buffs | same pattern if a persistent field exists; else a small hook at the mod's existing stat hook points | Medium |
| Reveal map area / full vision | engine fog reveal calls exist (gptp uses them); expose one via address | Medium |
| Free tech / unlock | write the player's upgrade flags (snapped player arrays) | Medium |
| Refund token (one-shot) | flag in Plus state; hook the order/building path to discount — needs the command hook | Medium |
| Items with per-unit persistence | Plus-owned buff table keyed by unit pointer | Medium |
| Pause during the draft window | engine pause (synced) triggered from the same frame on both peers | Medium |

Anything that mutates **persistent sim state** (tokens, buff timers, active drops) must
live in Plus-owned state that rolls back — see §5.

## 5. Determinism + netcode rules (the five rules, applied)

1. **Sim state is snapped or derived.** Draft state (draft #, the 4 drawn options, each
   player's pick, active buffs with restore frames, tokens) is serialized through a new
   `cosmo_plus_state_size/save/load/rollback_enable` export set — the exact Layer B/C
   pattern the rollback plugin already resolves by name
   (`crates/cosmo-rollback/src/plugin_state.rs`). The **one-line change** to the netcode
   crate is adding `("cosmo_plus", "cosmo_plus_")` to its plugin list; it contains no
   gameplay knowledge and stays optional (resolution fails cleanly when Plus isn't
   loaded). Replays get the same state via `api.extend_save`.
2. **No gameplay logic in Lua/aise.** All draft logic lives in the Rust plugin.
3. **UI reads, never writes.** The draft panel is drawn in the existing draw-proc hook
   (like the saturation overlay): text via `bw_print_xy`, per-player highlight from
   `api.local_player_id`. Drawing is not re-simmed, so per-player differences are safe.
4. **Same inputs, same result.** Pool derivation = pure function of (match seed, draft
   index, race pair, tier, previous picks). Picks enter the sim as **synced game
   commands**, not as raw keyboard reads: register a custom command id via
   `api.hook_ingame_command` (gptp's `SelectFactionCommand`/`GameCommand_Custom` is the
   same mechanism and already works under the rollback netcode), and inject the local
   player's hotkey choice through `api.send_command`. Local input → synced command →
   identical re-sim on both peers.
5. **Version-locked.** The draft option tables hash into the build id so mismatched
   `plus.yml` builds refuse to start (same as the existing overrides hashing).

## 6. Implementation map (what plugs into what)

| Piece | Where | Existing hook / pattern |
|---|---|---|
| Mode detection (type 47) | `antimeta.rs` `enabled()`-style read of `*g_game_type` | done for 46 |
| Template + lst entry | `tools/make-template.py` (extend for `Draft(1).got`) | done for Anti Meta |
| Option tables | `plus.yml` `draft:` section, parsed by `cosmo-plus-core` | `overrides:` pattern |
| Pool draw (pure) | `cosmo-plus-core/src/draft.rs` + host tests | `antimeta::roll` pattern |
| Draft clock | `frame.rs` per-frame hook, `frame_count / 24` | existing `FIRST_TICK` |
| Choice input | `api.hook_ingame_command` (new cmd id) + `api.send_command` | gptp faction command |
| Panel UI | `overlay.rs` draw pass, hotkey hints, countdown | saturation overlay |
| Effects | `antimeta.rs`-style spawn/resource code, generalized | `spawn_workers`/`set_resources` |
| Draft state blob | new `cosmo_plus_state_*` exports + serializer | gptp/aise Layer B/C |
| Announcements | `api.print_text` | anti-meta roll announce |

## 7. Phased plan with exit criteria

**Spike 0 — command round-trip (the only real unknown).** Register a no-op custom
ingame command id, send it from a hotkey, verify it arrives as a *synced* command on both
clients in a CrownLink rollback match (`cosmo_rollback.log` shows the command on both
sides, `desyncs=0`). One afternoon; everything else is known territory.
*Exit:* a hotkey press is visible in both clients' logs via the command stream.

**Phase 1 — draft skeleton, early tier only.** Mode type 47, draft clock, fixed pool of 4
economy/worker/scout options, 1–4 hotkeys, simultaneous apply, state blob + netcode
one-liner, replays. *Exit:* `tools/crownlink_match.sh` with the mode active shows
`desyncs=0` for a 10-minute scripted match with picks on both sides; replay reproduces
the same picks.

**Phase 2 — mid tier (minute 8+).** Drops (any unit ids + counts, town hall/expansion
anchor), items as one-shot tokens, reveal effects. Pool data in `plus.yml`.
*Exit:* scripted drop picks land deterministically; both players see identical units.

**Phase 3 — late tier + temporary buffs.** Upgrade-level buffs with restore, speed/
cooldown buffs via a hook if needed, "ult" drops. *Exit:* a buff expires mid-fight under
rollback (`desyncs=0`), buffs stack deterministically.

**Phase 4 — curation + AI.** Fill out the per-race-pair tables (this is the human part:
I can't judge "feels Mechabellum" — the tables live in `plus.yml` so you iterate
without builds). Computer players pick by a simple deterministic rule (highest-weight
effect for their race). Optional: pause during the window, click-to-pick, draft history
panel.

## 8. Decisions I need from you

1. **Mode placement:** new "Draft" game type (recommended), or fold the draft into
   Anti Meta?
2. **Start:** draft mode with the anti-meta random start (recommended), or standard start?
3. **Window behaviour:** non-blocking 30 s window with no auto-pick (v1, recommended),
   auto-pick option 1 on timeout, or pause the sim during the window?
4. **First draft:** minute 2 (recommended) or minute 0?
5. **Option pool curation:** do you want to write the per-race-pair tables yourself (I
   scaffold the YAML + validation), or should I draft an initial set for you to tune?
