#!/usr/bin/env python3
"""Crawl Blizzard's SCC Map Archives (classic.battle.net/scc/os/...) 1998-2001.

Extracts every monthly Map of the Week/Month: title, download link, and whether
it is flagged "Use Map Settings" (i.e. a UMS/scenario map, not plain melee).

Output: research/scc_maps.jsonl
"""
import json
import re
import sys
import time
import urllib.request

INDEX = "http://classic.battle.net/scc/os/"
UA = {"User-Agent": "Mozilla/5.0 (X11; Linux x86_64; rv:130.0) Gecko/20100101 Firefox/130.0"}

# monthly pages from the index nav: MMYY.shtml, plus motm pages
PAGES = (
    [f"{m:02d}{y}.shtml" for y in (98, 99) for m in (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12)]
    + [f"{m:02d}{y}.shtml" for y in (0, 1) for m in (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12)]
    + ["motm2000.shtml", "motm2001.shtml"]
)

def get(url, tries=3):
    for i in range(tries):
        try:
            req = urllib.request.Request(url, headers=UA)
            with urllib.request.urlopen(req, timeout=30) as r:
                return r.read().decode("latin1", "replace")
        except Exception as e:
            print(f"  retry {i} for {url}: {e}", file=sys.stderr)
            time.sleep(2 + 2 * i)
    return None

TITLE_RE = re.compile(r'Download\s+(.*?)\s*\n', re.I)
LINK_RE = re.compile(r'href="((?:http://ftp\.blizzard\.com|ftp://)[^"]+)"')

def parse_page(html, page):
    out = []
    # Map entries: a "Download X" line followed by description; find links near titles
    links = [(m.group(1), m.start()) for m in LINK_RE.finditer(html)]
    # find titles: lines like "Download Zerg Soccer Now!" etc.
    for m in re.finditer(r'Download\s+([A-Za-z0-9][^<>\n]{2,60}?)(?:\s*(?:Now!|here)|\s*\|)', html):
        title = m.group(1).strip()
        # nearest link after this position
        near = [l for l in links if l[1] > m.start() and l[1] < m.start() + 4000]
        url = near[0][0] if near else None
        ums = False
        seg = html[m.start():m.start() + 4000]
        if re.search(r'Use Map Settings', seg):
            ums = True
        out.append({"title": title, "url": url, "ums": ums, "page": page})
    return out

def main():
    seen = {}
    for p in PAGES:
        url = INDEX + p
        html = get(url)
        if html is None:
            print(f"[{p}] FAILED")
            continue
        for item in parse_page(html, p):
            key = (item["title"], item["url"])
            if key not in seen:
                seen[key] = item
        print(f"[{p}] -> {len(seen)} total")
        time.sleep(0.4)
    with open("/tmp/opencode/ums-port/research/scc_maps.jsonl", "w") as f:
        for v in seen.values():
            f.write(json.dumps(v) + "\n")
    ums = [v for v in seen.values() if v["ums"]]
    print(f"done: {len(seen)} entries, {len(ums)} flagged UMS")

if __name__ == "__main__":
    main()
