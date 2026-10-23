#!/usr/bin/env python3
"""Crawl RepMastered /maps pages (unfiltered, default sort = Included Games desc)
and keep rows with map class UMS (data-mapclassid="C").

Filters are login-gated, so we crawl the unfiltered list and filter client-side.
Stops when we have >= TARGET_UMS rows or hit MAX_PAGES.

Output: research/repmastered.jsonl  one JSON per UMS row.
"""
import json
import re
import sys
import time
import urllib.request

BASE = "https://repmastered.icza.net/maps"
OUT = "/tmp/opencode/ums-port/research/repmastered.jsonl"
TARGET_UMS = 500
MAX_PAGES = 700
SLEEP = 0.5

UA = {"User-Agent": "Mozilla/5.0 (X11; Linux x86_64; rv:130.0) Gecko/20100101 Firefox/130.0"}

def get(url, tries=4):
    for i in range(tries):
        try:
            req = urllib.request.Request(url, headers=UA)
            with urllib.request.urlopen(req, timeout=30) as r:
                return r.read().decode("utf-8", "replace")
        except Exception as e:
            print(f"  retry {i} for {url}: {e}", file=sys.stderr)
            time.sleep(3 + 3 * i)
    return None

ROW_RE = re.compile(r"<tr align=\"center\">(.*?)</tr>", re.S)

def num(s):
    """Parse '26.4m', '935k', '549' -> float."""
    s = s.strip().replace(",", "")
    m = re.match(r"([\d.]+)\s*([km]?)", s)
    if not m:
        return None
    v = float(m.group(1))
    return v * 1_000_000 if m.group(2) == "m" else v * 1_000 if m.group(2) == "k" else v

def parse_row(row):
    """Return dict for a UMS row, or None.

    Numeric title attr order in a row: variants, totalGames, games(included),
    excludedGames, totalPlays.
    """
    if 'data-mapclassid="C"' not in row:
        return None
    name = re.search(r'data-title="([^"]+)"', row)
    titles = [t.group(1) for t in re.finditer(r'title="([^"]+)"', row)]
    nums = []
    for t in titles:
        v = num(t)
        if v is not None:
            nums.append(v)
    games = nums[2] if len(nums) >= 3 else None
    plays = nums[4] if len(nums) >= 5 else None
    variants = nums[0] if nums else None
    mlast = re.search(r'title="(\d{4}-\d{2}-\d{2})"\s*>([^<]*)</td>', row)
    last = mlast.group(2).strip() if mlast else None
    return {"name": name.group(1) if name else None,
            "games": games, "plays": plays,
            "variants": variants, "last_game": last}

def main():
    seen = set()
    count = 0
    page = 1
    with open(OUT, "w") as f:
        while page <= MAX_PAGES and count < TARGET_UMS:
            url = BASE if page == 1 else f"{BASE}?page={page}"
            html = get(url)
            if html is None:
                print(f"page {page} FAILED", flush=True)
                break
            if "<table id=\"mapsTable\">" not in html:
                print(f"page {page}: no maps table (gated?), stopping", flush=True)
                break
            rows = ROW_RE.findall(html)
            ums_here = 0
            for row in rows:
                item = parse_row(row)
                if item is None:
                    continue
                ums_here += 1
                key = (item["name"], item["games"])
                if key in seen:
                    continue
                seen.add(key)
                item["page"] = page
                f.write(json.dumps(item, ensure_ascii=False) + "\n")
                count += 1
            print(f"page {page}: {len(rows)} rows, {ums_here} UMS, kept {count}", flush=True)
            page += 1
            time.sleep(SLEEP)
    print(f"done: {count} UMS rows, stopped at page {page-1}")

if __name__ == "__main__":
    main()
