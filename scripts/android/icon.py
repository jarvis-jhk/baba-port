#!/usr/bin/env python3
"""Launcher icon for the APK from the player's own game data (stdlib only).

Reads Baba's sprite (Data/Sprites/baba_0_1.png) and the default palette
(Data/Palettes/default.png) from a game install and writes the two layers of
an adaptive icon (docs/platform-android.md, "Launcher icon"):

  OUT/fg.png  432x432 (108 dp at xxxhdpi): Baba, tinted with its palette
              colour, scaled up (nearest) into the 66 dp safe circle
  OUT/bg.png  the level background colour

The result contains game art: it is only for builds that stay private (the
owner's devices), never for a public release or for git.

Usage: icon.py GAME_DIR OUT_DIR
"""
import os
import struct
import sys
import zlib

SIZE = 432       # 108 dp * 4 (xxxhdpi)
SAFE = 264       # 66 dp * 4: the circle every launcher mask keeps
BABA_COLOUR = (0, 3)   # palette cell of the baba object (x, y)
BACKGROUND = (0, 4)    # palette cell of the level background


def read_png(path):
    """8-bit non-interlaced PNG -> (w, h, rows of (r, g, b, a))."""
    d = open(path, "rb").read()
    if d[:8] != b"\x89PNG\r\n\x1a\n":
        raise SystemExit(f"{path}: not a PNG")
    pos, idat, plte, trns = 8, b"", None, None
    while pos < len(d):
        ln, typ = struct.unpack_from(">I4s", d, pos)
        body = d[pos + 8:pos + 8 + ln]
        if typ == b"IHDR":
            w, h, depth, ctype, _, _, inter = struct.unpack(">IIBBBBB", body)
        elif typ == b"PLTE":
            plte = [tuple(body[i:i + 3]) for i in range(0, len(body), 3)]
        elif typ == b"tRNS":
            trns = body
        elif typ == b"IDAT":
            idat += body
        pos += 12 + ln
    if depth != 8 or inter:
        raise SystemExit(f"{path}: only 8-bit non-interlaced PNGs")
    bpp = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}[ctype]
    raw = zlib.decompress(idat)
    stride = w * bpp
    prev = bytearray(stride)
    rows = []
    for y in range(h):
        f = raw[y * (stride + 1)]
        line = bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for i in range(stride):
            a = line[i - bpp] if i >= bpp else 0
            b = prev[i]
            c = prev[i - bpp] if i >= bpp else 0
            if f == 1:
                line[i] = (line[i] + a) & 255
            elif f == 2:
                line[i] = (line[i] + b) & 255
            elif f == 3:
                line[i] = (line[i] + (a + b) // 2) & 255
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        prev = line
        px = []
        for x in range(w):
            v = line[x * bpp:(x + 1) * bpp]
            if ctype == 0:
                px.append((v[0], v[0], v[0], 255))
            elif ctype == 2:
                px.append((v[0], v[1], v[2], 255))
            elif ctype == 3:
                alpha = trns[v[0]] if trns and v[0] < len(trns) else 255
                px.append(plte[v[0]] + (alpha,))
            elif ctype == 4:
                px.append((v[0], v[0], v[0], v[1]))
            else:
                px.append(tuple(v))
        rows.append(px)
    return w, h, rows


def write_png(path, w, h, rows):
    def chunk(t, b):
        return struct.pack(">I", len(b)) + t + b + struct.pack(">I", zlib.crc32(t + b))
    raw = b"".join(b"\0" + bytes(c for p in row for c in p) for row in rows)
    open(path, "wb").write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
                           + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))


def find(game, *parts):
    # Linux/Windows installs have Data/ at the root, the Mac bundle under
    # Contents/Resources.
    for root in (game, os.path.join(game, "Contents", "Resources")):
        p = os.path.join(root, *parts)
        if os.path.isfile(p):
            return p
    raise SystemExit(f"{game}: no {'/'.join(parts)} (not a Baba Is You install?)")


def main():
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    game, out = sys.argv[1:]
    _, _, pal = read_png(find(game, "Data", "Palettes", "default.png"))
    tint = pal[BABA_COLOUR[1]][BABA_COLOUR[0]]
    bg = pal[BACKGROUND[1]][BACKGROUND[0]][:3] + (255,)
    w, h, spr = read_png(find(game, "Data", "Sprites", "baba_0_1.png"))
    pts = [(x, y) for y in range(h) for x in range(w) if spr[y][x][3]]
    if not pts:
        raise SystemExit("empty sprite")
    x0, x1 = min(p[0] for p in pts), max(p[0] for p in pts) + 1
    y0, y1 = min(p[1] for p in pts), max(p[1] for p in pts) + 1
    bw, bh = x1 - x0, y1 - y0
    # The largest integer scale whose box fits the safe circle.
    s = max(1, int(SAFE / (bw * bw + bh * bh) ** 0.5))
    ox, oy = (SIZE - bw * s) // 2, (SIZE - bh * s) // 2
    clear = (0, 0, 0, 0)
    fg = [[clear] * SIZE for _ in range(SIZE)]
    for y in range(bh):
        for x in range(bw):
            r, g, b, a = spr[y0 + y][x0 + x]
            if not a:
                continue
            p = (r * tint[0] // 255, g * tint[1] // 255, b * tint[2] // 255, a)
            for yy in range(oy + y * s, oy + (y + 1) * s):
                fg[yy][ox + x * s:ox + (x + 1) * s] = [p] * s
    os.makedirs(out, exist_ok=True)
    write_png(os.path.join(out, "fg.png"), SIZE, SIZE, fg)
    write_png(os.path.join(out, "bg.png"), 108, 108, [[bg] * 108 for _ in range(108)])
    print(f"icon: baba {bw}x{bh} px at {s}x, tint {tint[:3]}, background {bg[:3]} -> {out}")


if __name__ == "__main__":
    main()
