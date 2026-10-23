#!/usr/bin/env python3
"""Crawl nydus.gg UMS map database.

Fetches cat=type:ums sorted by 'world' (most played of all time) and 'pop'
(most downloaded), all pages. Extracts: name, games, downloads, category,
map page URL, download URL.

Output: research/nydus.jsonl
"""
import json
import re
import sys
import time
import urllib.request

BASE = "https://nydus.gg/maps"
OUT = "/tmp/opencode/ums-port/research/nydus.jsonl"
MAX_PAGES = 400
SLEEP = 0.4

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

ROW_RE = re.compile(r'<tr>\s*<td>.*?</tr>', re.S)

def parse_rows(html):
    out = []
    for row in ROW_RE.findall(html):
        name_m = re.search(r'class="mdb-mname"[^>]*>(.*?)</a>', row, re.S)
        if not name_m:
            continue
        name = re.sub(r"<[^>]+>", "", name_m.group(1))
        name = name.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
        page_m = re.search(r'href="(/maps/\d+-[^"]+)"', row)
        dl_m = re.search(r'href="(mapa\.php\?id=(\d+)&amp;download=1)"', row)
        games_m = re.search(r'class="mdb-msub">([\d\s,]+)\s*games</div>', row)
        dl_m2 = re.search(r'class="mdb-num">(\d+)</td>', row)
        cat_m = re.search(r'<td class="hide-sm">([^<]+)</td>', row)
        games = int(games_m.group(1).replace(" ", "").replace(",", "")) if games_m else None
        out.append({
            "name": name.strip(),
            "games": games,
            "downloads": int(dl_m2.group(1)) if dl_m2 else None,
            "category": cat_m.group(1).strip() if cat_m else None,
            "page": (BASE + page_m.group(1)) if page_m else None,
            "dl": ("https://nydus.gg/" + dl_m.group(1)) if dl_m else None,
        })
    return out

def main():
    seen = {}
    for sort in ("world",):
        page = 1
        prev_sig = None
        repeat = 0
        while page <= MAX_PAGES:
            url = f"{BASE}?cat=type:ums&sort={sort}"
            if page > 1:
                url += f"&p={page}"
            html = get(url)
            if html is None:
                print(f"[{sort}] page {page} FAILED", flush=True)
                break
            rows = parse_rows(html)
            if not rows:
                print(f"[{sort}] page {page}: no rows, done", flush=True)
                break
            sig = (rows[0]["name"], rows[0]["games"], rows[-1]["name"], rows[-1]["games"])
            if sig == prev_sig:
                repeat += 1
                if repeat >= 3:
                    print(f"[{sort}] page {page}: repeated tail page, done", flush=True)
                    break
            else:
                repeat = 0
            prev_sig = sig
            for r in rows:
                key = (r["name"], r["games"])
                if key not in seen:
                    seen[key] = r
                    r["sort"] = sort
            print(f"[{sort}] page {page}: {len(rows)} rows, total {len(seen)}", flush=True)
            page += 1
            time.sleep(SLEEP)
    with open(OUT, "w") as f:
        for r in seen.values():
            f.write(json.dumps(r, ensure_ascii=False) + "\n")
    print("wrote", OUT, len(seen))

if __name__ == "__main__":
    main()
