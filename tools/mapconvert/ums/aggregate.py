#!/usr/bin/env python3
"""Aggregate popularity metrics across sources and select the top 100 UMS maps.

Metrics (objective, multi-source):
  1. actual games played  (RepMastered total plays + nydus games)   w=0.40
  2. downloads            (nydus + sc1db)                           w=0.30
  3. ratings              (sc1db)                                   w=0.10
  4. official recognition (Blizzard SCC archive picks)              w=0.20

Name matching: normalized keys, exact first, then difflib fuzzy (>=0.8) between
RepMastered names and nydus/sc1db names (links play counts to downloadable files).

Download resolution order: nydus direct link -> sc1db name match -> sc1db search
-> SCC zip -> none (dropped).

Genre quotas keep the list diverse (82 quota slots + 18 filled by global score).

Output: research/selection.json
"""
import difflib
import json
import math
import re
import subprocess
import sys
import time
import urllib.parse
import urllib.request
from collections import defaultdict

R = "/tmp/opencode/ums-port/research/"
UA = {"User-Agent": "Mozilla/5.0 (X11; Linux x86_64; rv:130.0) Gecko/20100101 Firefox/130.0"}

def norm_name(s):
    s = s.lower()
    s = re.sub(r"^\(?\d*\s*[x×v]?\s*\d*\s*\)?\s*", "", s)
    s = re.sub(r"\[.*?\]", "", s)
    s = re.sub(r"v?\d+(\.\d+)*([-_ ](final|beta|ver))?$", "", s)
    s = re.sub(r"[^a-z0-9가-힣ぁ-んァ-ン一-龥]+", "", s)
    return s

def key(s):
    k = norm_name(s)
    k = re.sub(r"(the|a|of|defense|defence|deff?|bound|rpg|map|maps)$", "", k)
    return k

# ---------- load ----------
nydus = [json.loads(l) for l in open(R + "nydus.jsonl")]
rep = [json.loads(l) for l in open(R + "repmastered.jsonl")]
sc1db = json.load(open(R + "sc1db.json"))
scc = [json.loads(l) for l in open(R + "scc_analysis.jsonl")]
scc = [r for r in scc if not r.get("error") and r["triggers"]["count"] > 10]

sc1 = []
for cat, items in sc1db.items():
    for it in items:
        it["cat"] = cat
        sc1.append(it)

def idx(rows, namefn):
    d = defaultdict(list)
    for r in rows:
        d[namefn(r)].append(r)
    return d

nydus_idx = idx(nydus, lambda r: key(r["name"]))
sc1_idx = idx(sc1, lambda r: key(r["name"]))
rep_idx = idx(rep, lambda r: key(r["name"]))
scc_idx = idx(scc, lambda r: key(r["title"]))

all_nydus_keys = list(nydus_idx.keys())
all_sc1_keys = list(sc1_idx.keys())

def fuzzy(match_keys, k):
    best, bestr = None, 0.0
    for mk in match_keys:
        r = difflib.SequenceMatcher(None, mk, k).ratio()
        if r > bestr:
            best, bestr = mk, r
    return (best, bestr) if bestr >= 0.80 else (None, bestr)

def sc1db_search(phrase):
    """Search sc1db; verify the top result's title is actually similar."""
    try:
        data = urllib.parse.urlencode({"phrase": phrase}).encode()
        req = urllib.request.Request("https://staredit.net/sc1db/search/", data=data, headers=UA)
        with urllib.request.urlopen(req, timeout=30) as r:
            html = r.read().decode("utf-8", "replace")
        ids = [int(x) for x in re.findall(r'href="/sc1db/file/(\d+)/"', html)]
        if not ids:
            return None
        fid = ids[0]
        req = urllib.request.Request(f"https://staredit.net/sc1db/file/{fid}/", headers=UA)
        with urllib.request.urlopen(req, timeout=30) as r:
            page = r.read().decode("utf-8", "replace")
        tm = re.search(r"<title>([^<]*)", page)
        title = re.sub(r"\s+", " ", tm.group(1)).strip() if tm else ""
        # similarity between the searched phrase and the found title
        from difflib import SequenceMatcher
        ratio = SequenceMatcher(None, phrase.lower(), title.lower()).ratio()
        if ratio >= 0.4 or any(w in title.lower() for w in phrase.lower().split() if len(w) > 3):
            return fid
        print(f"  sc1db search {phrase!r}: top hit {title!r} rejected (ratio {ratio:.2f})",
              file=sys.stderr)
        return None
    except Exception as e:
        print(f"  sc1db search fail {phrase!r}: {e}", file=sys.stderr)
        return None

# ---------- merge ----------
merged = {}

def entry(k):
    return merged.setdefault(k, {"names": set(), "games": 0, "downloads": 0,
                                 "rating": None, "official": False,
                                 "sources": {}, "categories": set(),
                                 "search_phrase": None})

# pass 1: exact-key merges
for r in nydus:
    e = entry(key(r["name"]))
    e["sources"].setdefault("nydus", r)
    e["names"].add(r["name"])
for r in sc1:
    e = entry(key(r["name"]))
    e["sources"].setdefault("sc1db", r)
    e["names"].add(r["name"])
for r in rep:
    k = key(r["name"])
    e = entry(k)
    e["sources"].setdefault("rep", r)
    e["names"].add(r["name"])
for r in scc:
    e = entry(key(r["title"]))
    e["sources"].setdefault("scc", r)
    e["official"] = True
    e["names"].add(r["title"])

# pass 2: fuzzy-link repmastered rows lacking an exact nydus match
for r in rep:
    k = key(r["name"])
    e = merged[k]
    if "nydus" in e["sources"]:
        continue
    mk, ratio = fuzzy(all_nydus_keys, k)
    if mk:
        e2 = merged[mk]
        e2["sources"].setdefault("rep_fuzzy", r)
        e2["names"].add(r["name"])
        e2["fuzzy_ratio"] = ratio
        merged[k] = e2  # alias
    # else: still try sc1db fuzzy
    elif not any(x in e["sources"] for x in ("sc1db", "scc")):
        mk2, ratio2 = fuzzy(all_sc1_keys, k)
        if mk2:
            e2 = merged[mk2]
            e2["sources"].setdefault("rep_fuzzy", r)
            e2["names"].add(r["name"])
            e2["fuzzy_ratio"] = ratio2
            merged[k] = e2

# ---------- metrics ----------
for k, e in list(merged.items()):
    rep_games = None
    for src_key in ("rep", "rep_fuzzy"):
        if src_key in e["sources"]:
            rep_games = max(rep_games or 0, e["sources"][src_key]["games"] or 0)
    ny = e["sources"].get("nydus")
    e["games"] = max(rep_games or 0, (ny or {}).get("games") or 0)
    ny_dl = (ny or {}).get("downloads")
    sc_dl = e["sources"]["sc1db"]["downloads"] if "sc1db" in e["sources"] else None
    e["downloads"] = max(ny_dl or 0, sc_dl or 0)
    if "sc1db" in e["sources"]:
        e["rating"] = e["sources"]["sc1db"]["rating"]
        e["categories"].add(e["sources"]["sc1db"]["cat"])
    if ny and ny.get("category"):
        e["categories"].add(ny["category"].lower())
    e["display"] = min((n for n in e["names"] if n and n.strip()), key=len, default="")

# ---------- download resolution ----------
def resolve(e):
    ny = e["sources"].get("nydus")
    if ny and ny.get("dl"):
        mm = re.match(r".*/maps/\d+-(.+)", ny.get("page") or "")
        return ny["dl"], mm.group(1) if mm else None
    if "sc1db" in e["sources"]:
        return "https://staredit.net/sc1db/download/%d/" % e["sources"]["sc1db"]["id"], None
    if "scc" in e["sources"]:
        return "scczip:" + e["sources"]["scc"]["source"], None
    return None, None

search_cache = {}
for k, e in merged.items():
    e["dl"], e["slug"] = resolve(e)
    if not e["dl"] and not any(ch >= "가" and ch <= "힣" for ch in e["display"]):
        phrase = re.sub(r"[^a-zA-Z0-9 ]", " ", e["display"])[:40]
        phrase = phrase.strip()
        if phrase and phrase.lower() not in search_cache:
            search_cache[phrase.lower()] = sc1db_search(phrase)
            time.sleep(0.4)
        fid = search_cache.get(phrase.lower())
        if fid:
            e["dl"] = "https://staredit.net/sc1db/download/%d/" % fid

# ---------- scores ----------
G = math.log1p(max((e["games"] for e in merged.values()), default=1))
D = math.log1p(max((e["downloads"] for e in merged.values()), default=1))
for k, e in merged.items():
    e["score"] = (0.40 * math.log1p(e["games"]) / G
                  + 0.30 * math.log1p(e["downloads"]) / D
                  + 0.10 * ((e["rating"] or 0) / 5.0)
                  + 0.20 * (1.0 if e["official"] else 0.0))

# ---------- genre ----------
def genre(e):
    cats = e["categories"]
    n = e["display"].lower()
    if "bound" in cats or "bound" in n or "바운드" in n: return "Bound"
    if "tower-defense" in cats or re.search(r"tower[\s_-]?def|타워|poker", n): return "TowerDefense"
    if "defense" in cats or re.search(r"\bdef(ense|ence)?\b|디펜스|디팬스|언덕|블러드|컴까기|디펜", n): return "Defense"
    if "rpg" in cats or "rpg" in n or "알피지" in n or "special forces" in n: return "RPG"
    if "arena" in cats or re.search(r"\barenas?\b|hero wars?|히어로", n): return "Arena"
    if "micro" in cats or "micro" in n or "컨트롤" in n: return "Micro"
    if "minihra" in cats or re.search(r"미니|puzzle|quiz|퀴즈|movie|board|card|soccer|football|racing|basketball|dodge", n): return "Puzzle"
    if "bunker" in n: return "TowerDefense"
    if re.search(r"mercenar|deception|execution", n): return "Strategy"
    if "strategy" in cats or "assault" in cats or "macro" in cats or "diplo" in n: return "Strategy"
    return "Other"

for k, e in merged.items():
    e["genre"] = genre(e)

# ---------- selection ----------
QUOTA = {"TowerDefense": 13, "Defense": 11, "Bound": 10, "RPG": 10,
         "Arena": 8, "Micro": 7, "Puzzle": 7, "Strategy": 8, "Other": 8}
OFFICIAL_TARGET = 10

ranked = sorted(merged.values(), key=lambda e: (-e["score"], -e["games"]))
ranked = [e for e in ranked if e["dl"]]
# dedupe aliased entries (same object under two keys)
seen_ids, ranked2 = set(), []
for e in ranked:
    if id(e) in seen_ids:
        continue
    seen_ids.add(id(e))
    ranked2.append(e)
ranked = ranked2

picked, per_genre = [], defaultdict(int)
official_picked = 0
have = set()

# pass 0: guarantee a slice of officially recognized classics (editorial metric)
for e in ranked:
    if official_picked >= 15:
        break
    if e["official"]:
        picked.append(e)
        have.add(id(e))
        per_genre[e["genre"]] += 1
        official_picked += 1

# pass 1: genre quotas
for e in ranked:
    if id(e) in have:
        continue
    g = e["genre"]
    if per_genre[g] >= QUOTA.get(g, 8):
        continue
    picked.append(e)
    have.add(id(e))
    per_genre[g] += 1

# pass 2: fill remaining by global score
for e in ranked:
    if id(e) in have:
        continue
    picked.append(e)
    have.add(id(e))
    if len(picked) >= 100:
        break

selection = []
for e in picked:
    selection.append({
        "name": e["display"],
        "genre": e["genre"],
        "games": e["games"],
        "downloads": e["downloads"],
        "rating": e["rating"],
        "official": e["official"],
        "score": round(e["score"], 4),
        "dl": e["dl"],
        "slug": e["slug"],
        "fuzzy": e.get("fuzzy_ratio"),
        "categories": sorted(e["categories"]),
        "all_names": sorted(e["names"])[:5],
    })

json.dump(selection, open(R + "selection.json", "w"), indent=1, ensure_ascii=False)
from collections import Counter
print("selected", len(selection))
print(Counter(s["genre"] for s in selection))
print("official picks:", sum(1 for s in selection if s["official"]))
print("fuzzy-linked:", sum(1 for s in selection if s["fuzzy"]))
print("dl sources: nydus=%d sc1db=%d scc=%d search=%d" % (
    sum(1 for s in selection if "nydus" in s["dl"]),
    sum(1 for s in selection if "sc1db" in s["dl"]),
    sum(1 for s in selection if s["dl"].startswith("scczip")),
    sum(1 for s in selection if "sc1db/search" in s["dl"])))
for s in selection[:20]:
    print(f"  {s['score']:.3f} g={s['games']:>10,.0f} dl={s['downloads']:>6} {s['genre']:<12} {s['name'][:44]}")
