# Plus data pass — authoritative ids and requested values

Two sources, deliberately:
- **Ids** come from `reference/cosmonarchy-gptp/BW/enumerations/UnitId.h` — the enum the
  local 19 Sep 2026 build was compiled from. `units.txt` is *not* reliably id-ordered
  (names and ids diverge for several entries), so never index it.
- **Current costs** below are from the public tech-tree export
  (`https://cosmonarchy.com/export/all.json`, fetched 2026-10-21). They must be confirmed
  against the live table dump (`CM_PLUS_DUMP=1`) before shipping deltas.

## Named structure costs (absolute, user-requested)

| Unit | id | published now (ore/gas/time) | requested |
|---|---:|---|---|
| Zerg Mutath Spire | 141 | 150 / 150 / 45s | **250 / 300** |
| Zerg Tosvrol Haunt | 197 | 400 / 700 / 60s | **400 / 850** |
| Zerg Alkag Iteth | 170 | 250 / 150 / 50s | **350 / 300** |
| Zerg Chokral Chazsal | 330 | 750 / 500 / 60s | **750 / 650** |
| Zerg Matravil Nest | 138 | 150 / 200 / 40s | **250 / 350** |
| Zerg Almakis Antre | 139 | 500 / 600 / 60s | **600 / 750** |
| Zerg Othstol Oviform | 152 | 900 / 2100 / 40s | **600 / 1000** |
| Zerg Axtoth Axis | 130 | 600 / 300 / 50s | **900 / 1100** |
| Zerg Geszkath Grotto | 219 | 250 / 500 / 60s | **400 / 900** |
| Zerg Kethraj Keep | 303 | 500 / 900 / 75s | **650 / 1300** |
| Zerg Rakzogth Roost | 136 | 350 / 500 / 60s | **650 / 1300** |

Spelling note: the user's "Alkag Itech", "Matravl Nest", "Kethrak Keep", "Rakzogh Roost"
are the in-game `Alkag Iteth`, `Matravil Nest`, `Kethraj Keep`, `Rakzogth Roost`.

Id note: `plus.yml` resolves units **by name** against `mpq/arr/units.txt` (line index ==
id), not by the ids in this table. The live dump (`CM_PLUS_DUMP=1`) confirmed every id here
except `Kethraj Keep`, which is **303** (212 is `Wall Missile Trap (Right)`).

## Gas extractors — tier deltas

Tiers are the mod's public tiers (the C++ `get_gas_tier` numbers them 2/3/4 because the
neutral geyser is 1). Requested: tier 1 −100 ore and −10s, tier 2 −150 ore and −15s,
tier 3 −200 ore and −20s.

| Tier | Unit | id | now | after |
|---|---|---:|---|---|
| 1 | Terran Sieve | 419 | 200 / 0 / 30s | 100 / 0 / 20s |
| 1 | Protoss Cedent | 420 | 150 / 50 / 20s | 50 / 50 / 10s |
| 1 | Zerg Eskigant | 421 | 150 / 0 / 30s | 50 / 0 / 20s |
| 1 | Askosi Arch | 457 | 150 / 0 / 50s | 50 / 0 / 40s |
| 2 | Terran Reservoir | 110 | 300 / 0 / 35s | 150 / 0 / 20s |
| 2 | Protoss Aquifer | 157 | 400 / 0 / 40s | 250 / 0 / 25s |
| 2 | Zerg Excisant | 149 | 250 / 0 / 35s | 100 / 0 / 20s |
| 2 | Askosi Abutment | 539 | 250 / 0 / 50s | 100 / 0 / 35s |
| 3 | Terran Wellbore | 365 | 600 / 100 / 45s | 400 / 100 / 25s |
| 3 | Zerg Ezigrisant | 364 | 400 / 100 / 40s | 200 / 100 / 20s |

Bloodsummon (`Capillary`, `Artery`) exist in the export but not in the local `UnitId.h`;
resolve those by name at runtime if the local build has them, otherwise skip.

## In-place extractor upgrades

Requested: every gas building can be upgraded in place, for the cost of the next tier's
building and with the next tier's build time.

- Zerg already morphs (`Eskigant → Excisant → Ezigrisant`; the C++ comment on
  `ZergEzigrisant` says "Morphed from Excisant"). Its morph data can be reused as the
  reference behaviour.
- Terran/Protoss/Askosi have no morph. Three options:
  1. Add a **morph-style order** that swaps the unit id at the same position, spends the
     next tier's cost/time, and reuses the build/morph animation (cheapest, no new art).
  2. A button that **kills the old extractor and starts construction of the next tier** at
     the same tile (needs a worker? no — direct create like a morph).
  3. Leave the upgrade to the existing "build the next tier on the geyser" flow and only
     adjust its cost/time.
- Option 1 is the closest to the request and is deterministic; it needs a new order/button
  and a `UnitId` pair table, not new art. Do it after the saturation overlay.

## Verification

`CM_PLUS_DUMP=1` writes `cosmo_plus_dump.csv` (id, name, ore, gas, time, speed). Compare
the published values above against the dump; if they differ, the dump wins and the `plus.yml`
deltas are recomputed from it. The live table is the only authority for the local build.

## Advanced workers (tier 2) — deltas

Requested: +25 ore, +25 gas, -7s build time, 1.5x *speed* inside a base (Ultraload),
counted as 1.5 in the saturation overlay. Implemented as runtime deltas over the **live** dat values (no
published absolute costs needed), keyed by the tier-2 ids in `saturation::WORKER_TIERS`:

| Unit | id | applies |
|---|---|---|
| Terran Talos / Ogun (Worker Mode) | 20 / 494 | +25/+25, -7s, x1.5 |
| Zerg Gosvileth / Agrileth | 47 / 302 | +25/+25, -7s, x1.5 |
| Protoss Prudence / Autocort | 184 / 495 | +25/+25, -7s, x1.5 |
| Askosi Dyad / Constituent | 444 / 566 | +25/+25, -7s, x1.5 |

- Config: `advanced_workers:` section in `plus.yml` (deltas default to the requested
  values).
- Ultraload: `ultraload.rs` raises `flingy.top_speed` for a tier-2 worker while it has
  harvest duty (a harvest target or carried cargo) and is within `depot_radius_px`
  (default 480 px = 20 tiles) of a completed town hall of its own player, restoring the
  dat value when it leaves. Speed is the right lever: the node's single 42-frame mining
  slot caps payloads per patch, so a saturated advanced base mines exactly what a
  saturated basic base mines — advanced workers just saturate with fewer of them.
- The earlier per-trip carry multiplier (scaling `carried_resource_count` after
  `transfer_resource_to_worker`, 0x004696D0) was **removed**: it raised the per-patch
  ceiling by 1.5x instead of making the worker faster, so saturated advanced bases
  out-earned basic ones.
- Icon: `ultraload_icon.rs` appends a disabled button (`IconId::UseStimpack` placeholder,
  engine `BTNSACT_DoNothing` action) to each tier-2 worker's command card. Replace
  `PLACEHOLDER_ICON` with the real icon when the art exists.
- Agrileth build time: the first build is a **morph** (Droleth → Agrileth, 20s) and later
  builds train from the hatchery (30s); both charge the dat build-time field, so the
  delta shortens both (13s morph / 23s train).
- Idle-worker hotkey: the idle predicate now also accepts workers parked on
  `Guard`/`PlayerGuard` orders (Agrileth keeps a stale harvest target while parked).
- Overlay label: `t1+t2x1.5=eff/optimal` above gas nodes and the town hall — the
  advanced worker weighs 1.5 in the count, and the math is written out.
