#!/usr/bin/env python3
"""PKWARE implode compressor (binary mode, 4K window) - matches the format
StarCraft's Storm writes/reads (as decoded by ShieldBattery's implode-decoder).

Format: header [type=0, dictbits=6, first data byte], then a LSB-first bitstream:
  literal: '0' + 8 bits (byte, LSB-first)
  match:   '1' + length SF code + length extra + distance SF code + distance extra
  end marker: '1' + length class 15 + extra 255

Decoded match length = lenBase[class] + extra + 2; decoded distance = code + 1.
Length-2 matches are limited to distance <= 255 (short form).
"""
import struct

# length classes: SF code (LSB-first value) + bit length + base + extra bits
LEN_CODE  = [0x05, 0x03, 0x01, 0x06, 0x0A, 0x02, 0x0C, 0x14, 0x04, 0x18, 0x08, 0x30, 0x10, 0x20, 0x40, 0x00]
LEN_BITS  = [3, 2, 3, 3, 4, 4, 4, 5, 5, 5, 5, 6, 6, 6, 7, 7]
LEN_BASE  = [0, 1, 2, 3, 4, 5, 6, 7, 8, 10, 14, 22, 38, 70, 134, 262]
LEN_EXTRA = [0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8]

# distance classes
DIST_CODE = [
    0x03, 0x0D, 0x05, 0x19, 0x09, 0x11, 0x01, 0x3E, 0x1E, 0x2E, 0x0E, 0x36, 0x16, 0x26, 0x06, 0x3A,
    0x1A, 0x2A, 0x0A, 0x32, 0x12, 0x22, 0x42, 0x02, 0x7C, 0x3C, 0x5C, 0x1C, 0x6C, 0x2C, 0x4C, 0x0C,
    0x74, 0x34, 0x54, 0x14, 0x64, 0x24, 0x44, 0x04, 0x78, 0x38, 0x58, 0x18, 0x68, 0x28, 0x48, 0x08,
    0xF0, 0x70, 0xB0, 0x30, 0xD0, 0x50, 0x90, 0x10, 0xE0, 0x60, 0xA0, 0x20, 0xC0, 0x40, 0x80, 0x00,
]
DIST_BITS = [
    0x02, 0x04, 0x04, 0x05, 0x05, 0x05, 0x05, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06,
    0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07,
    0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07,
    0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08,
]

DICT_BITS = 6
DICT_MASK = (1 << DICT_BITS) - 1
MIN_MATCH = 2
MAX_LEN = 517
MAX_DIST = 4095


class BitWriter:
    def __init__(self):
        self.buf = bytearray()
        self.cur = 0
        self.nbits = 0

    def write(self, value, nbits):
        if nbits == 0:
            return
        self.cur |= (value & ((1 << nbits) - 1)) << self.nbits
        self.nbits += nbits
        while self.nbits >= 8:
            self.buf.append(self.cur & 0xFF)
            self.cur >>= 8
            self.nbits -= 8

    def finish(self):
        if self.nbits:
            self.buf.append(self.cur & 0xFF)
        return bytes(self.buf)


def _len_class(enc_len):
    """Find the length class for an encoded length (actual length - 2, 0..515)."""
    for i in range(15, -1, -1):
        if enc_len >= LEN_BASE[i]:
            return i
    return 0


def _emit_match(bw, length, dist):
    enc = length - 2          # decoder: actual length = lenBase[class] + extra + 2
    cls = _len_class(enc)
    bw.write(1, 1)                                   # match flag
    bw.write(LEN_CODE[cls], LEN_BITS[cls])           # length SF code (LSB-first)
    bw.write(enc - LEN_BASE[cls], LEN_EXTRA[cls])
    d = dist - 1             # decoder: actual distance = code + 1
    if length == 2:
        dc = d >> 2
        bw.write(DIST_CODE[dc], DIST_BITS[dc])
        bw.write(d & 3, 2)
    else:
        dc = d >> DICT_BITS
        bw.write(DIST_CODE[dc], DIST_BITS[dc])
        bw.write(d & DICT_MASK, DICT_BITS)


def implode(data):
    """Compress `data` (bytes) with PKWARE implode, binary mode, 4K window."""
    n = len(data)
    bw = BitWriter()
    # hash chains for match finding (simple: map 2-byte prefixes to a recent offset)
    head = {}
    i = 0
    while i < n:
        # find the best match at i
        best_len = 0
        best_dist = 0
        if i + MIN_MATCH <= n:
            key = (data[i] << 8) | data[i + 1]
            pos = head.get(key, -1)
            if pos >= 0 and i - pos <= MAX_DIST:
                # extend the match
                maxl = min(MAX_LEN, n - i)
                l = 2
                while l < maxl and data[pos + l] == data[i + l]:
                    l += 1
                if l > best_len:
                    best_len = l
                    best_dist = i - pos
        # length-2 matches are limited to distance <= 255 by the format
        if best_len == 2 and best_dist > 255:
            best_len = 0
        if best_len >= MIN_MATCH:
            # try to extend over the next char too (greedy is fine for validity)
            _emit_match(bw, best_len, best_dist)
            # update hash for each position covered
            for j in range(i, min(i + best_len, n - 1)):
                key = (data[j] << 8) | data[j + 1]
                head[key] = j
            i += best_len
        else:
            bw.write(0, 1)              # literal flag
            bw.write(data[i], 8)        # literal byte (LSB-first)
            if i + 1 < n:
                key = (data[i] << 8) | data[i + 1]
                head[key] = i
            i += 1
    # end marker: match flag + length class 15 + extra 255
    bw.write(1, 1)
    bw.write(LEN_CODE[15], LEN_BITS[15])
    bw.write(255, LEN_EXTRA[15])
    body = bw.finish()
    return bytes([0, DICT_BITS]) + body


if __name__ == "__main__":
    import sys
    data = open(sys.argv[1], "rb").read()
    out = implode(data)
    open(sys.argv[2], "wb").write(out)
    print(f"imploded {len(data)} -> {len(out)} bytes")
