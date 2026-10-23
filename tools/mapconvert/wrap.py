#!/usr/bin/env python3
"""Storm MPQ v0 wrapper writer for StarCraft maps (MapDori format).

Converts an extracted SCR-format chk into a 1.16.1-compatible .scx:
  - patch the VER section version (205/206 -> 205; 59/63 left as-is)
  - rename the SCR string-table section STRx -> STR
  - re-wrap into a standard Storm MPQ: encrypted hash/block tables, implode-compressed
    sectors with a sector offset table, per-sector Storm encryption.

Format spec (reverse-engineered from ShieldBattery/scm-extractor and the 1.16.1
storm.dll + StarCraft.exe):
  inner file name:  staredit\scenario.chk
  tables:           dword stream cipher, keys hashString('(block table)') /
                    hashString('(hash table)'); the seed update always uses the
                    PLAINTEXT dword, so encrypt/decrypt are mirror functions
  file data:        sector offset table (numSectors+1 u32) encrypted with
                    key-1, then per-sector blobs encrypted with key+i (fresh
                    stream each); sector blob = 0x08 type byte + implode data
"""
import struct
import sys

CRYPT = None

def init():
    global CRYPT
    if CRYPT:
        return
    t = [0] * 0x500
    seed = 0x00100001
    for i1 in range(0x100):
        i2 = i1
        for _ in range(5):
            seed = (seed * 125 + 3) % 0x2AAAAB
            t1 = (seed & 0xFFFF) << 16
            seed = (seed * 125 + 3) % 0x2AAAAB
            t[i2] = t1 | (seed & 0xFFFF)
            i2 += 0x100
    CRYPT = t

def hash_string(s, typ):
    """Storm hashString: typ 0=table offset, 1=nameA, 2=nameB, 3=file key."""
    init()
    seed1 = 0x7FED7FED
    seed2 = 0xEEEEEEEE
    for c in s.upper().encode("latin1"):
        seed1 = (CRYPT[typ * 0x100 + c] ^ ((seed1 + seed2) & 0xFFFFFFFF)) & 0xFFFFFFFF
        seed2 = (c + seed2 + ((seed2 << 5) & 0xFFFFFFFF) + seed1 + 3) & 0xFFFFFFFF
    return seed1 & 0xFFFFFFFF

class Cipher:
    """Storm MPQ dword stream cipher. The seed update always uses the PLAINTEXT dword,
    so encrypt and decrypt are mirror functions (decrypt matches ShieldBattery's Decrypter)."""
    def __init__(self, key):
        self.key = key & 0xFFFFFFFF
        self.seed = 0xEEEEEEEE
    def _advance(self, plain):
        old = self.key
        left = ((~old & 0xFFFFFFFF) << 0x15) & 0xFFFFFFFF
        left = (left + 0x11111111) & 0xFFFFFFFF
        self.key = (left | (old >> 0x0B)) & 0xFFFFFFFF
        self.seed = (plain + self.seed + ((self.seed << 5) & 0xFFFFFFFF) + 3) & 0xFFFFFFFF
    def encrypt(self, plain):
        p = plain & 0xFFFFFFFF
        self.seed = (self.seed + CRYPT[0x400 + (self.key & 0xFF)]) & 0xFFFFFFFF
        c = p ^ ((self.key + self.seed) & 0xFFFFFFFF)
        self._advance(p)
        return c
    def decrypt(self, ciph):
        c = ciph & 0xFFFFFFFF
        self.seed = (self.seed + CRYPT[0x400 + (self.key & 0xFF)]) & 0xFFFFFFFF
        p = c ^ ((self.key + self.seed) & 0xFFFFFFFF)
        self._advance(p)
        return p

def crypt_words(key, words):
    """Encrypt table dwords (encrypt direction, pairs with the game's/Storm's decrypt)."""
    c = Cipher(key)
    return [c.encrypt(w) for w in words]

def decrypt_words(key, words):
    c = Cipher(key)
    return [c.decrypt(w) for w in words]

def encrypt_bytes(key, data):
    """Stream-encrypt byte data: full dwords encrypted, trailing 1-3 bytes left plaintext
    (matches Storm's SFileEncrypt and the extractor's DecrypterStream flush behavior)."""
    c = Cipher(key)
    out = bytearray()
    n = len(data) & ~3
    for i in range(0, n, 4):
        w = struct.unpack_from("<I", data, i)[0]
        out += struct.pack("<I", c.encrypt(w))
    out += data[n:]
    return bytes(out)

CHK_NAME = "staredit\\scenario.chk"

def build_mpq(chk_data, sector_shift=3):
    init()
    from implode import implode
    sector_size = 512 << sector_shift
    file_key = hash_string("scenario.chk", 3)
    num = (len(chk_data) + sector_size - 1) // sector_size
    blobs = []
    for i in range(num):
        s = chk_data[i * sector_size:(i + 1) * sector_size]
        c = implode(s)
        if len(c) + 1 < sector_size and len(c) + 1 < len(s):
            blobs.append(b"\x08" + c)  # implode compression type byte + stream
        else:
            blobs.append(s)
    # sector offset table: num+1 u32 cumulative offsets, starting AFTER the table itself
    offs = [(num + 1) * 4]
    for b in blobs:
        offs.append(offs[-1] + len(b))
    table = struct.pack("<" + "I" * (num + 1), *offs)

    # table encrypted with key-1; each sector blob with key+i (fresh stream each)
    enc_table = encrypt_bytes((file_key - 1) & 0xFFFFFFFF, table)
    enc_blobs = [encrypt_bytes((file_key + i) & 0xFFFFFFFF, b) for i, b in enumerate(blobs)]
    data = enc_table + b"".join(enc_blobs)

    flags = 0x80000000 | 0x00010000 | 0x00000200  # exists | encrypted | compressed

    hdr_size = 0x20
    hc, bc = 1024, 1
    ht_size = 16 * hc
    bt_size = 16 * bc
    data_off = hdr_size
    ht_off = data_off + len(data)
    bt_off = ht_off + ht_size
    total = bt_off + bt_size

    btd = [data_off, len(data), len(chk_data), flags]
    btk = hash_string("(block table)", 3)
    bt = struct.pack("<" + "I" * (4 * bc), *crypt_words(btk, btd))

    hA = hash_string(CHK_NAME, 1)
    hB = hash_string(CHK_NAME, 2)
    htd = [0xFFFFFFFF] * (4 * hc)
    idx = hash_string(CHK_NAME, 0) & (hc - 1)
    htd[idx * 4], htd[idx * 4 + 1], htd[idx * 4 + 2], htd[idx * 4 + 3] = hA, hB, 0, 0
    htk = hash_string("(hash table)", 3)
    ht = struct.pack("<" + "I" * (4 * hc), *crypt_words(htk, htd))

    header = struct.pack("<4sIIHHIIII", b"MPQ\x1a", hdr_size, total, 0, sector_shift,
                         ht_off, bt_off, hc, bc)
    return header + data + ht + bt

def find_ver(chk):
    """Locate the VER section (chk may start with TYPE instead). Returns (name_off, ver)."""
    i = 0
    while i + 8 <= len(chk):
        nm = chk[i:i+4]
        sz = struct.unpack_from("<I", chk, i+4)[0]
        if sz > 0x4000000:
            break
        if nm == b"VER ":
            return i, struct.unpack_from("<H", chk, i+8)[0]
        i += 8 + sz
    return -1, None

def patch_chk(chk, target_ver=205):
    voff, ver = find_ver(chk)
    assert ver is not None, f"no VER section found (starts {chk[:4]!r})"
    assert ver in (59, 63, 205, 206), f"unexpected version {ver}"
    p = bytearray(chk)
    if ver in (205, 206):
        struct.pack_into("<H", p, voff + 8, target_ver)
    # rename the SCR string-table section to the name the 1.16.1 engine expects
    i = 0
    while i + 8 <= len(p):
        nm = p[i:i+4]
        sz = struct.unpack_from("<I", p, i+4)[0]
        if sz > 0x4000000:
            break
        if nm == b"STRx":
            p[i:i+4] = b"STR "
        i += 8 + sz
    return bytes(p)

if __name__ == "__main__":
    src, dst = sys.argv[1], sys.argv[2]
    chk = open(src, "rb").read()
    chk = patch_chk(chk)
    mpq = build_mpq(chk)
    open(dst, "wb").write(mpq)
    print(f"wrote {dst}: chk={len(chk)} bytes -> mpq={len(mpq)} bytes")
