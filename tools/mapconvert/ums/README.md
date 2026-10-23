# UMS port pipeline (CMPlus)

Tools used to build the CMPlus "Top 100 Brood War UMS maps" pack
(`Maps/CMBW-Root/CMBW-maps/CMPlus/`). See the CMPlus README for the pack
itself; this directory is the reusable pipeline.

## Pipeline stages

1. **Research** — crawl objective popularity metrics:
   - `crawl_repmastered.py` — UMS rows from RepMastered `/maps` (games played)
   - `crawl_nydus.py` — nydus.gg UMS database (games + downloads + direct links)
   - `crawl_sc1db.py` — Staredit Network sc1db per-genre ratings/downloads
   - `crawl_scc.py` — Blizzard SCC Map of the Week/Month archive index
   - `aggregate.py` — merge by normalised name (+ fuzzy), composite score,
     genre quotas, official-classics quota, download resolution (with
     title-verified sc1db search fallback) -> `selection.json`

2. **Download** — `download.py` (curl, MPQ magic + zip validation).

3. **Analyze** — `extract.mjs` (MPQ -> scenario.chk) + `analyze.mjs`
   (bw-chk: version/sections/forces/triggers, EUD detection, strings dump).

4. **Port** — `port.py` per-map: conversion decision, safe naming (nydus
   slugs for non-ASCII titles), static re-verification, `manifest.csv` with
   per-map compat notes (vanilla -> Cosmo unit slot mapping from
   `slot_mapping.json`, built from the BWAPI UnitType enum vs the mod's
   `CMBW-reference.txt`).

5. **Convert** — `convert.py` for SCR-era maps: STRx->STR, VER 206->205,
   VER 64->63, preserving ALL inner MPQ files (imported sounds) via the
   Storm MPQ v0 reader/writer (`mpq.py` / `mpq_write.py`).

6. **In-game test** — `test_ums.sh` / `run_gametests.sh`: headless Xvfb boot,
   OCR row finder, conditional briefing handling. NOTE: pressing Return
   in-game exits the game under the current mod build, so the harness never
   sends keys after the start counter moves.

## Notes / gotchas

- 1.16.1 rejects VER 64 maps ("intended for use with StarCraft Expansion
  Set") — convert to 63.
- nydus download links carry HTML-escaped `&amp;` — unescape before use.
- sc1db search relevance is poor; always verify the result's file page title.
- RepMastered filtered views are login-gated; crawl the unfiltered list and
  filter map class client-side.
- The game's `anti-meta: game type` log line is the in-game "game started"
  signal (one per game).
