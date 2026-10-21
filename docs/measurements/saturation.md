# Saturation model — measurements and derivation

The headline feature is a per-patch `assigned/optimal` display. "Optimal" has to be
*measured*, not asserted, so this document records the exact worker speeds, the harvest
cycle timing, the derivation of the per-patch optimum, and the empirical validation.

**Headline result: the live flingy data shows no speed difference between the functional
tier-1 and tier-2 harvest workers.** Every functional worker (tier-1 *and* the tier-2
workers that actually mine) has `top_speed = 1280` in `flingy.dat` — identical. The
user's `0/8` vs `0/4` split has no basis in measured speed, and a number of the units
listed as "tier-2 workers" in `economic_units.cpp` have placeholder flingy stats
(`speed=1, accel=1, halt=1`) and cannot move in the current build at all. Details below.

---

## 1. Method

Three independent sources, all reproducible from this tree:

1. **The live `flingy.dat` / `units.dat` tables.** `CM_PLUS_DUMP=1` writes
   `cosmo_plus_dump.csv` (id, name, minerals, gas, build_time, flingy id, top_speed, …).
   The same values were re-checked by parsing the raw binary `Release/mpq/arr/flingy.datex`
   directly (see the `python3` one-liner in §6), so the numbers below are not an artifact
   of the plugin.
2. **The harvest-cycle code.** `reference/cosmonarchy-gptp/hooks/harvest.cpp` and
   `BW/api.h` define the mining timer.
3. **A live-game delivery logger** (`CM_PLUS_MEASURE=1`), which watches each mineral
   node's `resource_amount` decrease and records the frame, the node, and the tier-1/tier-2
   worker counts targeting it. This produced `cosmo_plus_measure.csv` and
   `cosmo_plus_speed.csv`.

`Cosmonarchy BW.exe` runs at **24 logic frames/second** (the mod's `operator""secs`
returns `seconds * 24`, `types.h:71`).

---

## 2. Worker speeds (exact, from the live flingy data)

`flingy.dat` field 1 is `top_speed`, stored as 8.8 fixed point; speed in px/frame is
`top_speed / 256` (standard Brood War scale). Values verified against `flingy.datex`.

| Worker | id | tier | flingy id | top_speed | px/frame | accel | halt | notes |
|---|---|---|---|---|---|---|---|---|
| Terran Mason | 7 | 1 | 81 | 1280 | 5.0 | 67 | 12227 | starting worker |
| Terran Talos | 20 | 2 | 290 | **1** | ~0 | 1 | 1 | placeholder — cannot move |
| Terran Ogun (Worker Mode) | 494 | 2 | 575 | 1280 | 5.0 | 67 | 12227 | same speed as tier-1 |
| Zerg Droleth | 41 | 1 | 5 | 1280 | 5.0 | 67 | 12227 | starting worker |
| Zerg Gosvileth | 47 | 2 | 410 | **1** | ~0 | 1 | 1 | placeholder |
| Zerg Agrileth | 302 | 2 | 578 | **1** | ~0 | 1 | 1 | placeholder |
| Protoss Scribe | 64 | 1 | 42 | 1280 | 5.0 | 67 | 12227 | starting worker |
| Protoss Prudence | 184 | 2 | 411 | **1** | ~0 | 1 | 1 | placeholder (flying) |
| Protoss Autocort | 495 | 2 | 574 | 1280 | 5.0 | 67 | 12227 | same speed as tier-1 |
| Askosi Pilgrim | 439 | 1 | 564 | 1280 | 5.0 | 67 | 12227 | starting worker |
| Askosi Dyad | 444 | 2 | 598 | **1** | ~0 | 1 | 1 | placeholder |
| Askosi Constituent | 566 | 2 | 659 | **1** | ~0 | 1 | 1 | placeholder |

The tier-1 workers (Mason/Droleth/Scribe/Pilgrim) and the two *functional* tier-2 workers
(Ogun, Autocort) are byte-identical in flingy terms: `speed=1280, accel=67, halt=12227,
turn=40, move=2`. **There is no tier speed difference.**

The six units with `top_speed=1` (`Talos, Gosvileth, Prudence, Agrileth, Dyad,
Constituent`) carry placeholder flingy stats. In the current 19 Sep 2026 build they are
not functional movers — they are campaign/hero/“builder” slots that either do not harvest
or are wired up elsewhere. The live-worker runtime peak speed observed in a real game was
`current_speed = 1126` (= 4.4 px/frame), i.e. the worker reaches ~88% of its 1280 top
speed on a short mineral-line trip (acceleration/turn overhead).

---

## 3. Harvest cycle timing

From `harvest.cpp` / `api.h`:

- **Mining service time** `HarvestTimer = 1.75 secs = 42` frames (depleted field
  `HarvestTimerDepleted = 3.75 secs = 90` frames). This is the `main_order_timer` a worker
  spends on the node before `transfer_resource_to_worker` fires.
- **Drop-off is instant** — `return_resources_from_unit` adds the carried amount to the
  player's bank the moment the worker reaches the depot.
- A node has exactly **one mining slot**: `begin_harvest` sets
  `node->building.resource.is_mining = 1`; other workers queue in `next_gatherer`.
- The mod's own wander heuristic treats **2 workers per node** as "always optimal"
  (`harvester_is_active`, `harvest.cpp:11-17`), which is a useful sanity anchor.

So a worker's solo cycle is:

```
cycle = MINING_FRAMES + round_trip
round_trip = 2 · distance / speed + dropoff_overhead
```

---

## 4. Optimal-count derivation

With `n` workers on one node, the node delivers `n / (M + R)` payloads/frame (no
contention) capped at the slot ceiling `1 / M`. The smallest `n` that reaches the ceiling:

```
n / (M + R) ≥ 1 / M   ⟺   n ≥ (M + R) / M = 1 + R / M
n*(tier, D) = ⌈ 1 + (2·D/v_tier + δ) / M ⌉
```

with `M = 42` frames, `v_tier` the worker speed, `D` the node→depot distance (px), `δ` the
drop-off/turn overhead (≈ 0, refined by measurement).

**Because `v_tier` is identical (1280) for every functional worker, `n*` does not depend
on tier.** There is no tier-2 term in the optimum; a mixed worker base saturates the same
as a homogeneous one at the same distance. The function is implemented and unit-tested in
`plus/crates/cosmo-plus-core/src/saturation.rs` (`optimal_count`, `throughput`,
`round_trip_frames`).

### What the optimum is at real mineral-line distances

| distance D | round trip (2D / 5.0) | n* |
|---|---|---|
| 96 px (tight mineral line) | 38 fr | 2 |
| 200 px (typical mineral line) | 80 fr | 3 |
| 400 px (distant expansion) | 160 fr | 5 |
| 735 px (~23 tiles, unrealistic) | 294 fr | 8 |

So for a normal mineral line the measured optimum is **2–3 workers per patch** — which
agrees with the mod's own "2 workers = always optimal" wander threshold, and directly
contradicts the `0/8` default. The `0/8` figure would only be right for a patch ~23 tiles
from the depot.

---

## 5. Empirical validation

`CM_PLUS_MEASURE=1` logged every `resource_amount` decrease in a live (replay) game. The
starting workforce is all tier-1; each node had ~1–2 workers (the mod wanders workers
between nodes, so the per-node count varies). Median delivery gap per node vs. its
straight-line distance to the nearest depot (`docs/measurements/saturation.csv`):

| distance px | workers | measured cycle (frames) |
|---|---|---|
| 202 | 1 | 93 |
| 214 | 1 | 96 |
| 226 | 1 | 107 |
| 226 | 1 | 108 |
| 230 | 1 | 108 |
| 243 | 1 | 124 |
| 202 | 2 | 50 |

The 1-worker cycle grows with distance (93 → 124 frames) as the model predicts, and the
2-worker node roughly halves the gap (50 frames), confirming the single-slot queueing. The
cycle includes `42` frames of mining plus travel; the measured travel time is consistent
with a worker moving at ~4–5 px/frame (top speed 1280 with acceleration overhead) over a
real path that is shorter than the straight-line-to-centre distance (workers drop at the
depot's edge). The raw data is `cosmo_plus_measure.csv` / `cosmo_plus_speed.csv` from any
`CM_PLUS_MEASURE=1` run.

**Measured vs modelled, honestly:** the cadence points are *measured*; the `n*` curve is
*modelled* from those points plus the exact `top_speed` and `HarvestTimer`. No numbers are
fabricated — the only gaps are (a) I could not spawn arbitrary worker counts from the
plugin (no `create_unit` surface in the shim), so the curve's high-`n` end is modelled, not
measured; and (b) the tier-2 workers are absent at game start, so their speed is taken from
the live `flingy.dat`, which is authoritative for this build.

---

## 6. Commands run and results

```bash
# build + host unit tests
tools/build.sh                                   # Finished dev profile (OK)
cargo test -p cosmo-plus-core                    # 17 passed; 0 failed

# install + headless dump
tools/install-plus.sh Release                    # -> plugins/zz-cosmo-plus.qdp
# cosmo_plus.conf: CM_PLUS_DUMP=1 / CM_PLUS_MEASURE=1
bash ~/Projects/cosmonarchy-rollback/tools/headless_probe.sh Release \
  "490,182;940,590;320,645;350,238;940,590" "" /tmp/opencode/plus-dump.png 55 70
```

`cosmo_plus.log` showed `loaded`, `dat init: units_dat=0x513c30 (3000 entries)`,
`dumped 3000 unit rows`, `measure armed`. `cosmo_plus_dump.csv` has sane costs (e.g.
Mason `50/0/336`, Mutath Spire present) and the worker speeds above.

Verifying the speeds independently of the plugin (the `top_speed=1` values are real data,
not a read bug):

```bash
python3 -c 'import struct; d=open(".../mpq/arr/flingy.datex","rb").read();
N=len(d)//15; so=N*2
print([(i,struct.unpack("<I",d[so+i*4:so+i*4+4])[0]) for i in (5,42,81,290,410,411,564,574,575,578,598,659)])'
# -> flingy 5/42/81/564/574/575 = 1280 ; 290/410/411/578/598/659 = 1
```

## 7. Remaining / caveats

- The `0/8` / `0/4` tier defaults in `plus.yml` are retained as *fallback constants* only;
  the measured model above is authoritative and does not distinguish tier.
- Confirming the tier-2 workers' in-game cadence requires spawning them (needs a
  `create_unit`-style surface); until then their speed is taken from the authoritative
  `flingy.dat`.
- Overlay drawing is a separate task and was not attempted here.
