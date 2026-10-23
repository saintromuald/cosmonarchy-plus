#!/usr/bin/env python3
"""Convert an SCR-format map (or any map needing STRx/VER fixes) to 1.16.1,
preserving ALL inner MPQ files (imported sounds etc.).

Strategy:
  - extract scenario.chk (known name)
  - analyze: if no STRx and ver in (59,63,205) -> no conversion needed
  - else: dump CHK strings -> candidate inner names for extra MPQ files
  - extract every hash entry (matching candidates); warn on unmatched
  - patch chk: STRx -> STR, VER 206 -> 205
  - rebuild MPQ with all files via mpq_write.py

Usage: convert.py <src.scx> <dst.scx>
Exit codes: 0 ok (converted or copy), 2 needs no conversion, 3 failed.
"""
import json
import os
import re
import struct
import subprocess
import sys
import tempfile

TOOLS = os.path.dirname(os.path.abspath(__file__))

def sh(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)

def node(args):
    return sh(["node"] + args)

def extract_chk(src):
    out = tempfile.NamedTemporaryFile(delete=False, suffix=".chk").name
    r = node([os.path.join(TOOLS, "extract.mjs"), src, out])
    if r.returncode != 0 or not os.path.exists(out):
        return None
    return out

def analyze_chk(chk):
    r = node([os.path.join(TOOLS, "analyze.mjs"), chk])
    try:
        return json.loads(r.stdout.strip().splitlines()[-1])
    except Exception:
        return {"error": r.stdout[-200:]}

def dump_strings(chk):
    r = node([os.path.join(TOOLS, "analyze.mjs"), chk, "--strings"])
    return r.stdout

def list_mpq(src):
    """Returns (entries, sector_shift) via mpq.py internals."""
    sys.path.insert(0, TOOLS)
    import mpq
    mpq.WRAP.init()
    d, files, ss = mpq.read_archive(src)
    return d, files, ss

def name_candidates(strings):
    cands = ["staredit\\scenario.chk"]
    for ln in strings.splitlines():
        if "\t" not in ln:
            continue
        s = ln.split("\t", 1)[1].strip()
        if "\\" in s and "." in s.split("\\")[-1] and len(s) < 200:
            s = s.strip('"').strip()
            if re.match(r"^[ -~]+$", s) and "\\" in s:
                cands.append(s)
    # dedupe preserving order
    out = []
    for c in cands:
        if c not in out:
            out.append(c)
    return out

def patch_chk(chk_path):
    d = bytearray(open(chk_path, "rb").read())
    patched = False
    i = 0
    while i + 8 <= len(d):
        nm = bytes(d[i:i + 4])
        sz = struct.unpack_from("<I", d, i + 4)[0]
        if sz > 0x4000000:
            break
        if nm == b"STRx":
            d[i:i + 4] = b"STR "
            patched = True
        if nm == b"VER " and sz >= 2:
            ver = struct.unpack_from("<H", d, i + 8)[0]
            if ver in (206, 64):
                struct.pack_into("<H", d, i + 8, 205 if ver == 206 else 63)
                patched = True
        i += 8 + sz
    if patched:
        open(chk_path, "wb").write(d)
    return patched

def main():
    src, dst = sys.argv[1], sys.argv[2]
    chk = extract_chk(src)
    if not chk:
        print("extract-fail")
        sys.exit(3)
    a = analyze_chk(chk)
    if "error" in a:
        print("parse-fail: " + str(a["error"])[:100])
        sys.exit(3)
    ver, has_strx = a.get("ver"), a.get("hasSTRx")
    if not has_strx and ver in (59, 63, 205):
        print("no-conversion (ver %s, plain STR)" % ver)
        os.unlink(chk)
        sys.exit(2)
    # conversion needed
    d, files, ss = list_mpq(src)
    strings = dump_strings(chk)
    cands = name_candidates(strings)
    chk_entry = next((k for k, m in files.items()), None)
    chk_key = None
    sys.path.insert(0, TOOLS)
    import mpq
    chk_name = "staredit\\scenario.chk"
    hA, hB = mpq.hash_string(chk_name, 1), mpq.hash_string(chk_name, 2)
    matched, unmatched = [], []
    for (a2, b2), meta in files.items():
        name = None
        for c in cands:
            if (mpq.hash_string(c, 1), mpq.hash_string(c, 2)) == (a2, b2):
                name = c
                break
        if name is None:
            unmatched.append((a2, b2, meta))
            continue
        matched.append((name, meta))
    # extract extra files
    extras = []
    for name, meta in matched:
        if name == chk_name:
            continue
        out = tempfile.NamedTemporaryFile(delete=False, suffix=".bin").name
        try:
            mpq.extract_file(d, meta, ss, name, out)
            if os.path.exists(out) and os.path.getsize(out) > 0:
                extras.append((name, out))
            else:
                os.unlink(out)
        except Exception as e:
            print(f"extra-file fail {name}: {e}", file=sys.stderr)
    patched = patch_chk(chk)
    args = [sys.executable, os.path.join(TOOLS, "mpq_write.py"), dst,
            chk, chk_name]
    for name, path in extras:
        args += [path, name]
    r = sh(args)
    if r.returncode != 0:
        print("write-fail: " + (r.stderr or "")[:200])
        sys.exit(3)
    # verify
    vchk = extract_chk(dst)
    if not vchk:
        print("verify-fail")
        sys.exit(3)
    va = analyze_chk(vchk)
    ok = "error" not in va and va.get("hasSTRx") is False
    print(f"converted ver {ver}->{va.get('ver')} strx {has_strx}->{va.get('hasSTRx')} "
          f"files={len(extras) + 1} unmatched={len(unmatched)} patched={patched} verify={'ok' if ok else 'BAD'}")
    for p in [chk, vchk] + [x[1] for x in extras]:
        try:
            os.unlink(p)
        except OSError:
            pass
    sys.exit(0 if ok else 3)

if __name__ == "__main__":
    main()
