#!/usr/bin/env python3
"""Crawl sc1db map categories: extract {name, file_url, rating, downloads} per map.

Categories: bounds, defense, rpg, assault, board, micro, movie, puzzle,
strategy, terrain, others. Pages: /sc1db/browse/maps/<cat>/<page>/ (page 0 = base).

Output: research/sc1db.json  {category: [{name, url, rating, downloads}]}
"""
import json
import re
import sys
import time
import urllib.request

BASE = "https://staredit.net"
CATS = ["bounds", "defense", "rpg", "assault", "board", "micro",
        "movie", "puzzle", "strategy", "terrain", "others"]
OUT = "/tmp/opencode/ums-port/research/sc1db.json"

UA = {"User-Agent": "Mozilla/5.0 (X11; Linux x86_64; rv:130.0) Gecko/20100101 Firefox/130.0"}

def get(url, tries=3):
    for i in range(tries):
        try:
            req = urllib.request.Request(url, headers=UA)
            with urllib.request.urlopen(req, timeout=30) as r:
                return r.read().decode("utf-8", "replace")
        except Exception as e:
            print(f"  retry {i} for {url}: {e}", file=sys.stderr)
            time.sleep(2 + 2 * i)
    return None

FILE_RE = re.compile(
    r'<a href="(/sc1db/file/\d+/)">\s*(.*?)\s*</a>.*?'
    r'Rating:</span>.*?</span>\s*&nbsp;\s*'
    r'<span class="secondary">Downloads:</span>\s*([\d,]+)',
    re.S)

def parse_page(html):
    out = []
    # each file infobox: header link + rating/downloads
    for m in re.finditer(
            r'<div class="sc1db_fileinfo_header">\s*<a href="(/sc1db/file/(\d+)/)">(.*?)</a>',
            html, re.S):
        url, fid, name = m.groups()
        name = re.sub(r"<[^>]+>", "", name)
        name = name.replace("&#39;", "'").replace("&quot;", '"').replace("&amp;", "&")
        name = re.sub(r"\s+", " ", name).strip()
        # search the tail for downloads within this box
        tail = html[m.end():m.end() + 4000]
        dm = re.search(r"Downloads:</span>\s*([\d,]+)", tail)
        rm = re.search(r"Rating:</span>\s*(.*?)&nbsp;", tail, re.S)
        rating = None
        if rm:
            rtxt = re.sub(r"<[^>]+>", "", rm.group(1)).strip()
            mm = re.search(r"([\d.]+)\s*/\s*5", rtxt)
            rating = float(mm.group(1)) if mm else None
        downloads = int(dm.group(1).replace(",", "")) if dm else None
        out.append({"name": name, "url": BASE + url, "id": int(fid),
                    "rating": rating, "downloads": downloads})
    return out

def next_page(html, cat):
    m = re.search(
        r'class="pages">.*?</div>', html, re.S)
    if not m:
        return None
    links = re.findall(r'href="(/sc1db/browse/maps/%s/(\d+)/)"' % cat, m.group(0))
    if not links:
        return None
    return BASE + links[-1][0]

def main():
    result = {}
    for cat in CATS:
        page = f"{BASE}/sc1db/browse/maps/{cat}/"
        items, n, seen_pages = [], 0, set()
        while page and n < 40 and page not in seen_pages:
            seen_pages.add(page)
            print(f"[{cat}] page {n}: {page}", flush=True)
            html = get(page)
            if html is None:
                print(f"[{cat}] FAILED at page {n}", flush=True)
                break
            found = parse_page(html)
            items.extend(found)
            page = next_page(html, cat)
            n += 1
            time.sleep(0.6)
        result[cat] = items
        print(f"[{cat}] total {len(items)} items", flush=True)
    with open(OUT, "w") as f:
        json.dump(result, f, indent=1, ensure_ascii=False)
    print("wrote", OUT)

if __name__ == "__main__":
    main()
