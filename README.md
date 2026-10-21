# Cosmonarchy Plus

A gameplay fork of [Cosmonarchy BW](https://fraudsclub.com/cosmonarchy-bw/) built on top
of the rollback netcode in this repository's `netcode/` submodule.

Two products, deliberately separate:

- **`netcode/`** — the rollback client and QDP plugin, unmodified by Plus. This is the
  piece to share with upstream; it has no gameplay knowledge.
- **`plus/`** — the Cosmonarchy Plus QDP plugin and its data/config. Gameplay and QoL
  only; it never depends on netcode internals.

The only contract between them is optional and by export name:
`cosmo_rollback_match_seed()` (when present) gives Plus the shared match seed it needs
for deterministic start randomisation. Absent, Plus falls back to a seed derived from
the map checksum.

**Plus clients only play Plus clients of the same build.** A Plus replay will not
replay on vanilla Cosmonarchy.

## Features

1. **Saturation overlay** — per-patch tier-1/tier-2 worker counts with a race-coloured
   "saturated" state; empirically calibrated from worker speed.
2. **Economy data pass** — gas-extractor tier costs/build times, named structure costs,
   in-place extractor upgrades (see `docs/PLUS-DATA.md`).
3. **Anti-meta mode** — per-game start randomisation (4–8 workers, 100–300/0–125
   resources in steps of 5), symmetric across teams, from the shared seed.

## Build & install

```
tools/build.sh                 # cargo xwin, i686-pc-windows-msvc
tools/install-plus.sh Release  # copies to plugins/zz-cosmo-plus.qdp
```

## Status

See `docs/PLAN.md` for the phased plan, the feasibility matrix, and what is still
blocked. Measurements and their method live in `docs/measurements/`.
