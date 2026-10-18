#!/usr/bin/env python3
"""Checks a probe screencap: corners show the clear colour, the centre shows
one of the two checkerboard texture colours (probes/android/src/lib.rs).
Usage: check-shot.py shot.png   (stdlib only PNG decoder)"""
import struct
import sys
import zlib

CLEAR = (26, 51, 115)
TEX = [(255, 200, 0), (230, 40, 120)]


def decode(path):
    d = open(path, "rb").read()
    assert d[:8] == b"\x89PNG\r\n\x1a\n"
    pos, idat = 8, b""
    while pos < len(d):
        n, typ = struct.unpack(">I4s", d[pos:pos + 8])
        body = d[pos + 8:pos + 8 + n]
        if typ == b"IHDR":
            w, h, depth, ctype = struct.unpack(">IIBB", body[:10])
        elif typ == b"IDAT":
            idat += body
        pos += 12 + n
    assert depth == 8 and ctype in (2, 6)
    bpp = 3 if ctype == 2 else 4
    raw, stride = zlib.decompress(idat), w * bpp
    rows, prev, i = [], bytearray(stride), 0
    for _ in range(h):
        f, line = raw[i], bytearray(raw[i + 1:i + 1 + stride])
        i += 1 + stride
        for x in range(stride):
            a = line[x - bpp] if x >= bpp else 0
            b = prev[x]
            c = prev[x - bpp] if x >= bpp else 0
            if f == 1: line[x] = (line[x] + a) & 255
            elif f == 2: line[x] = (line[x] + b) & 255
            elif f == 3: line[x] = (line[x] + (a + b) // 2) & 255
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[x] = (line[x] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        rows.append(line)
        prev = line
    return w, h, lambda x, y: tuple(rows[y][x * bpp:x * bpp + 3])


w, h, px = decode(sys.argv[1])
near = lambda p, q: all(abs(a - b) <= 3 for a, b in zip(p, q))  # noqa: E731
ok = True
for x, y in [(5, 5), (w - 6, 5), (5, h - 6), (w - 6, h - 6)]:
    if not near(px(x, y), CLEAR):
        print("corner", (x, y), px(x, y), "is not the clear colour", CLEAR); ok = False
c = px(w // 2 + 7, h // 2 + 7)
if not any(near(c, t) for t in TEX):
    print("centre", c, "is not a texture colour"); ok = False
print("%dx%d: %s" % (w, h, "OK (clear colour + textured quad)" if ok else "FAILED"))
sys.exit(0 if ok else 1)
