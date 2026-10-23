#!/usr/bin/env python3
"""Port pipeline: downloaded maps -> Cosmonarchy-loadable .scx in out/CMPlus/<Genre>/.

Per map:
  1. extract scenario.chk (node extract.mjs); failure => status extract-fail
  2. analyze (node analyze.mjs)
  3. conversion:
     - SCR-format (ver 205/206 or STRx section): patch VER + rename STRx -> STR,
       rewrap as classic Storm MPQ (wrap.py)
     - otherwise copy as-is
  4. safe filename (ASCII, <= 31 chars), dedupe
  5. static verify: re-extract + re-analyze the FINAL file
  6. record everything in out/manifest.csv

Usage: port.py [--only idx1,idx2] [--skip-download]
"""
import csv
import json
import os
import re
import shutil
import subprocess
import sys

R = "/tmp/opencode/ums-port"
TOOLS = R + "/tools"
WRAP = "/home/saintbellarmine/Projects/cosmonarchy-plus/tools/mapconvert/wrap.py"
RAW = R + "/dl/raw"
WORK = R + "/work"
OUT = R + "/out/CMPlus"

def sh(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)

def extract(mapfile, out_chk):
    r = sh(["node", TOOLS + "/extract.mjs", mapfile, out_chk])
    return r.returncode == 0 and os.path.exists(out_chk)

def analyze(f, node="node"):
    r = sh([node, TOOLS + "/analyze.mjs", f])
    try:
        return json.loads(r.stdout.strip().splitlines()[-1])
    except Exception:
        return {"error": r.stdout[-200:]}

def safe_name(name, genre, slug=None):
    s = name
    if slug:
        s = slug.replace("-", " ")[:27]
    s = re.sub(r"[^A-Za-z0-9 _\-\.]", "", s)
    s = re.sub(r"\s+", " ", s).strip()
    if len(s) > 27:
        s = s[:27].rstrip(" ._-")
    return s or "Map"

def main():
    only = None
    if "--only" in sys.argv:
        only = {int(x) for x in sys.argv[sys.argv.index("--only") + 1].split(",")}
    os.makedirs(WORK, exist_ok=True)
    os.makedirs(OUT, exist_ok=True)
    sel = json.load(open(R + "/research/selection.json"))
    dlman = json.load(open(R + "/dl/manifest.json"))
    rows = []
    if only is not None and os.path.exists(R + "/out/manifest.csv"):
        with open(R + "/out/manifest.csv") as f:
            rows = list(csv.DictReader(f))
    used_names = {}
    for i, s in enumerate(sel):
        if only is not None and i not in only:
            continue
        if only is not None:
            rows = [r for r in rows if int(r["idx"]) != i]
        dm = dlman[i]
        row = {"idx": i, "name": s["name"], "genre": s["genre"],
               "games": s["games"], "downloads": s["downloads"],
               "rating": s["rating"], "official": s["official"],
               "source_url": s.get("dl", ""), "status": "?", "converted": "",
               "ver_in": "", "ver_out": "", "triggers": "", "eud": "",
               "units_in_triggers": "", "final_file": "", "notes": ""}
        if dm["status"] != "ok":
            row["status"] = dm["status"]
            rows.append(row)
            print(f"{i:03d} {dm['status']}")
            continue
        src = RAW + "/" + dm["dest"]
        stem = f"{i:03d}"
        chk = f"{WORK}/{stem}.chk"
        if not extract(src, chk):
            row["status"] = "extract-fail"
            rows.append(row)
            print(f"{i:03d} extract-fail {dm['dest']}")
            continue
        a = analyze(chk)
        if "error" in a:
            row["status"] = "chk-parse-fail"
            row["notes"] = a["error"][:120]
            rows.append(row)
            print(f"{i:03d} chk-parse-fail {a['error'][:60]}")
            continue
        row["ver_in"] = a.get("ver")
        row["triggers"] = (a.get("triggers") or {}).get("count", 0)
        row["eud"] = ((a.get("triggers") or {}).get("eudActions", 0) or 0) + ((a.get("triggers") or {}).get("eudConditions", 0) or 0)
        row["units_in_triggers"] = len((a.get("triggers") or {}).get("unitIds", []))
        # compatibility notes: which referenced vanilla slots Cosmo repurposed
        notes = []
        slotmap = {m["slot"]: m for m in json.load(open(R + "/research/slot_mapping.json"))}
        for uid in (a.get("triggers") or {}).get("unitIds", []):
            m = slotmap.get(uid)
            if m and m["cosmo"]:
                vname = m["vanilla"].replace("_", " ")
                # repurposed: cosmo name's first word race/type differs wildly
                if not vname.lower().startswith(m["cosmo"].lower()[:8]) and vname.lower().split()[0] not in m["cosmo"].lower():
                    notes.append(f"{uid}:{m['vanilla']}->{m['cosmo']}")
        row["compat_notes"] = "; ".join(notes[:8]) + (" ..." if len(notes) > 8 else "")
        needs_conv = a.get("hasSTRx") or a.get("ver") in (206, 64)
        name = safe_name(s["name"], s["genre"], s.get("slug"))
        genre_dir = OUT + "/" + s["genre"]
        os.makedirs(genre_dir, exist_ok=True)
        base = name
        k = 1
        while (base, s["genre"]) in used_names:
            k += 1
            base = f"{name}_{k}"
        used_names[(base, s["genre"])] = True
        final = f"{genre_dir}/{base}.scx"
        if needs_conv:
            r = sh([sys.executable, TOOLS + "/convert.py", src, final])
            if r.returncode not in (0, 2):
                row["status"] = "convert-fail"
                row["notes"] = (r.stdout or "")[-150:]
                rows.append(row)
                print(f"{i:03d} convert-fail {base}")
                continue
            row["converted"] = "scr->1161" if r.returncode == 0 else "copy(no-conv)"
            if r.returncode == 2:
                shutil.copy(src, final)
        else:
            shutil.copy(src, final)
            row["converted"] = "copy"
        # verify final file
        vchk = f"{WORK}/{stem}.v.chk"
        if not extract(final, vchk):
            row["status"] = "verify-extract-fail"
            rows.append(row)
            print(f"{i:03d} verify-extract-fail {base}")
            continue
        va = analyze(vchk)
        if "error" in va:
            row["status"] = "verify-parse-fail"
            row["notes"] = va["error"][:120]
            rows.append(row)
            print(f"{i:03d} verify-parse-fail {base}")
            continue
        row["ver_out"] = va.get("ver")
        row["status"] = "ok"
        row["final_file"] = f"CMPlus/{s['genre']}/{base}.scx"
        rows.append(row)
        print(f"{i:03d} ok  ver {row['ver_in']}->{row['ver_out']} {base}.scx "
              f"(trig {row['triggers']}, eud {row['eud']})")
        os.unlink(vchk)
    # manifest.csv
    cols = ["idx", "status", "name", "genre", "final_file", "converted",
            "ver_in", "ver_out", "triggers", "eud", "units_in_triggers",
            "games", "downloads", "rating", "official", "compat_notes", "source_url", "notes"]
    with open(R + "/out/manifest.csv", "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=cols)
        w.writeheader()
        for r in rows:
            w.writerow({c: r.get(c, "") for c in cols})
    ok = sum(1 for r in rows if r["status"] == "ok")
    print(f"\n{ok}/{len(rows)} ported OK")

if __name__ == "__main__":
    main()
