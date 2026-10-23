#!/usr/bin/env python3
"""Download the selected maps from their sources.

Sources:
  - nydus.gg:  https://nydus.gg/mapa.php?id=N&download=1
  - sc1db:     https://staredit.net/sc1db/download/<id>/
  - scc zip:   local dl/scc_maps/<name>

Saves to dl/raw/<idx>_<sanitized>.scx(.scm). Continues past failures.
"""
import html
import json
import os
import re
import subprocess
import sys
import time
import urllib.request

R = "/tmp/opencode/ums-port"
RAW = R + "/dl/raw"
UA = "Mozilla/5.0 (X11; Linux x86_64; rv:130.0) Gecko/20100101 Firefox/130.0"
COOKIEJAR = R + "/dl/cookies.txt"

def sanitize(s):
    s = re.sub(r"[^A-Za-z0-9 _\-\.\[\]\(\)]", "", s)
    return s[:40]

def download(url, dest, tries=3):
    for i in range(tries):
        try:
            r = subprocess.run(
                ["curl", "-s", "-L", "-A", UA, "-b", COOKIEJAR, "-c", COOKIEJAR,
                 "-o", dest, "-w", "%{http_code} %{size_download}", "--max-time", "90", url],
                capture_output=True, text=True)
            parts = r.stdout.strip().split()
            code, size = parts[0], int(parts[1]) if len(parts) > 1 else 0
            if code == "200" and size > 0:
                # validate: SC maps are Storm MPQs; some sources ship a ZIP
                with open(dest, "rb") as f:
                    head = f.read(4)
                if head == b"MPQ\x1a":
                    return dest, size
                if head == b"PK\x03\x04":
                    import zipfile
                    try:
                        z = zipfile.ZipFile(dest)
                        members = [m for m in z.namelist()
                                   if m.lower().endswith((".scx", ".scm", ".chk"))]
                        if members:
                            data = z.read(members[0])
                            if data[:4] == b"MPQ\x1a":
                                open(dest, "wb").write(data)
                                return dest, len(data)
                    except Exception as e:
                        print(f"  zip fail {url}: {e}", file=sys.stderr)
                print(f"  bad magic {head!r} for {url}", file=sys.stderr)
                os.unlink(dest)
            else:
                print(f"  http {code} size {size} for {url}", file=sys.stderr)
        except Exception as e:
            print(f"  retry {i} {url}: {e}", file=sys.stderr)
        time.sleep(2 + 2 * i)
    return None, 0

def main():
    os.makedirs(RAW, exist_ok=True)
    sel = json.load(open(R + "/research/selection.json"))
    manifest = []
    for i, s in enumerate(sel):
        name = sanitize(s["name"])
        dl = s.get("dl")
        if dl:
            dl = html.unescape(dl)
        status, dest, size = "skip", "", 0
        if not dl:
            status = "no-url"
        elif dl.startswith("scczip:"):
            src = R + "/dl/scc_maps/" + dl.split(":", 1)[1]
            if os.path.exists(src):
                ext = os.path.splitext(src)[1] or ".scx"
                dest = f"{RAW}/{i:03d}_{name}{ext}"
                subprocess.run(["cp", src, dest])
                size = os.path.getsize(dest)
                status = "ok"
            else:
                status = "scc-missing"
        else:
            ext = ".scx"
            dest = f"{RAW}/{i:03d}_{name}{ext}"
            got, size = download(dl, dest)
            if got:
                dest = got
                status = "ok"
            else:
                status = "fail"
        manifest.append({"idx": i, "name": s["name"], "status": status,
                         "dest": os.path.basename(dest) if dest else "", "size": size,
                         "genre": s["genre"]})
        print(f"{i:03d} {status:<10} {size:>8} {os.path.basename(dest) if dest else s['name'][:40]}")
        time.sleep(0.3)
    json.dump(manifest, open(R + "/dl/manifest.json", "w"), indent=1)
    ok = sum(1 for m in manifest if m["status"] == "ok")
    print(f"downloaded {ok}/{len(manifest)}")

if __name__ == "__main__":
    main()
