#!/usr/bin/env python3
"""Storm MPQ v0 multi-file writer (StarCraft 1.16.1 format).

Builds an archive with any number of inner files, each implode-compressed and
encrypted exactly like the game's Storm writes them (verified format used by
wrap.py, whose single-file output loads in-game).

Usage: mpq_write.py <outfile> <file1> <name1> [<file2> <name2> ...]
"""
import struct
import sys
import os

sys.path.insert(0, "/home/saintbellarmine/Projects/cosmonarchy-plus/tools/mapconvert")
import importlib.util
spec = importlib.util.spec_from_file_location("wrap",
    "/home/saintbellarmine/Projects/cosmonarchy-plus/tools/mapconvert/wrap.py")
WRAP = importlib.util.module_from_spec(spec)
spec.loader.exec_module(WRAP)

from implode import implode  # noqa: E402  (same dir as wrap.py)

SECTOR_SIZE = 4096

def hash_string(s, typ):
    seed1, seed2 = 0x7FED7FED, 0xEEEEEEEE
    for c in s.upper().encode("latin1"):
        seed1 = (WRAP.CRYPT[typ * 0x100 + c] ^ ((seed1 + seed2) & 0xFFFFFFFF)) & 0xFFFFFFFF
        seed2 = (c + seed2 + ((seed2 << 5) & 0xFFFFFFFF) + seed1 + 3) & 0xFFFFFFFF
    return seed1 & 0xFFFFFFFF

def build_mpq(entries):
    """entries: list of (inner_name, data_bytes). Returns MPQ bytes."""
    WRAP.init()
    nfiles = len(entries)
    hc = 4
    while hc < nfiles:
        hc <<= 1
    hc = max(hc, 16)
    bc = nfiles

    hdr_size = 0x20
    ht_size = 16 * hc
    bt_size = 16 * bc

    file_datas = []
    block_entries = []
    file_key_by_name = {}
    for name, data in entries:
        file_key = hash_string(name.rsplit("\\", 1)[-1], 3)
        num = (len(data) + SECTOR_SIZE - 1) // SECTOR_SIZE
        blobs = []
        for i in range(num):
            s = data[i * SECTOR_SIZE:(i + 1) * SECTOR_SIZE]
            c = implode(s)
            if len(c) + 1 < SECTOR_SIZE and len(c) + 1 < len(s):
                blobs.append(b"\x08" + c)
            else:
                blobs.append(s)
        offs = [(num + 1) * 4]
        for b in blobs:
            offs.append(offs[-1] + len(b))
        table = struct.pack("<" + "I" * (num + 1), *offs)
        enc_table = WRAP.encrypt_bytes((file_key - 1) & 0xFFFFFFFF, table)
        enc_blobs = [WRAP.encrypt_bytes((file_key + i) & 0xFFFFFFFF, b)
                     for i, b in enumerate(blobs)]
        filedata = enc_table + b"".join(enc_blobs)
        flags = 0x80000000 | 0x00010000 | 0x00000200
        file_datas.append(filedata)
        block_entries.append([len(filedata), len(data), flags])
        file_key_by_name[name] = file_key

    # layout
    data_off = hdr_size
    curs = data_off
    for fd in file_datas:
        curs += len(fd)
    ht_off = curs
    bt_off = ht_off + ht_size
    total = bt_off + bt_size

    # block table
    bt_vals = []
    curs = data_off
    for (flen, fsize, flags), fd in zip(block_entries, file_datas):
        bt_vals += [curs, flen, fsize, flags]
        curs += flen
    btk = hash_string("(block table)", 3)
    bt = struct.pack("<" + "I" * (4 * bc), *WRAP.crypt_words(btk, bt_vals))

    # hash table
    htd = [0xFFFFFFFF] * (4 * hc)
    for name, _data in entries:
        hA = hash_string(name, 1)
        hB = hash_string(name, 2)
        hO = hash_string(name, 0)
        i = hO & (hc - 1)
        while htd[i * 4] != 0xFFFFFFFF:
            i = (i + 1) % hc
        htd[i * 4:i * 4 + 3] = [hA, hB, 0]
        blk = next(k for k, e in enumerate(entries) if e[0] == name)
        htd[i * 4 + 3] = blk
    htk = hash_string("(hash table)", 3)
    ht = struct.pack("<" + "I" * (4 * hc), *WRAP.crypt_words(htk, htd))

    header = struct.pack("<4sIIHHIIII", b"MPQ\x1a", hdr_size, total, 0, 3,
                         ht_off, bt_off, hc, bc)
    return header + b"".join(file_datas) + ht + bt

def main():
    outfile = sys.argv[1]
    rest = sys.argv[2:]
    assert len(rest) % 2 == 0 and rest
    entries = []
    for i in range(0, len(rest), 2):
        path, name = rest[i], rest[i + 1]
        entries.append((name, open(path, "rb").read()))
    mpq = build_mpq(entries)
    open(outfile, "wb").write(mpq)
    print(f"wrote {outfile}: {len(entries)} files, {len(mpq)} bytes")

if __name__ == "__main__":
    main()
