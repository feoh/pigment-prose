#!/usr/bin/env python3
"""Compare two directories of same-named 8-bit RGB PNGs (task 23).

    python3 scripts/png-compare.py REFERENCE_DIR CANDIDATE_DIR

For each NN.png in REFERENCE_DIR, prints PSNR, the largest channel
difference and the share of differing channels against CANDIDATE_DIR/NN.png,
then a summary. Standard library only (zlib + a PNG row decoder), so it runs
on any CI runner. Exit status 1 if a file is missing or sizes differ.
"""
import pathlib
import struct
import sys
import zlib
from math import log10


def decode(path):
    data = path.read_bytes()
    assert data[:8] == b"\x89PNG\r\n\x1a\n", path
    i, idat, w = 8, b"", None
    while i < len(data):
        n, kind = struct.unpack(">I4s", data[i : i + 8])
        body = data[i + 8 : i + 8 + n]
        if kind == b"IHDR":
            w, h, depth, color, _, _, interlace = struct.unpack(">IIBBBBB", body)
            assert depth == 8 and color == 2 and interlace == 0, f"{path}: not 8-bit RGB"
        elif kind == b"IDAT":
            idat += body
        i += 12 + n
    raw = zlib.decompress(idat)
    bpp, stride = 3, w * 3
    out = bytearray(stride * h)
    prev = bytearray(stride)
    pos = 0
    for y in range(h):
        f = raw[pos]
        row = bytearray(raw[pos + 1 : pos + 1 + stride])
        pos += 1 + stride
        for x in range(stride):
            a = row[x - bpp] if x >= bpp else 0
            b = prev[x]
            c = prev[x - bpp] if x >= bpp else 0
            if f == 1:
                row[x] = (row[x] + a) & 255
            elif f == 2:
                row[x] = (row[x] + b) & 255
            elif f == 3:
                row[x] = (row[x] + (a + b) // 2) & 255
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pred = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                row[x] = (row[x] + pred) & 255
        out[y * stride : (y + 1) * stride] = row
        prev = row
    return w, h, bytes(out)


def main():
    ref, cand = map(pathlib.Path, sys.argv[1:3])
    status, worst, rows = 0, None, 0
    for r in sorted(ref.glob("*.png")):
        c = cand / r.name
        if not c.exists():
            print(f"{r.name}: MISSING in {cand}")
            status = 1
            continue
        (w1, h1, a), (w2, h2, b) = decode(r), decode(c)
        if (w1, h1) != (w2, h2):
            print(f"{r.name}: size {w1}x{h1} vs {w2}x{h2}")
            status = 1
            continue
        se = sum((x - y) * (x - y) for x, y in zip(a, b))
        maxd = max(abs(x - y) for x, y in zip(a, b))
        differ = sum(1 for x, y in zip(a, b) if x != y) / len(a)
        psnr = float("inf") if se == 0 else 10 * log10(255 * 255 / (se / len(a)))
        print(f"{r.name}: PSNR {psnr:.1f} dB, max channel difference {maxd}/255, {100 * differ:.1f}% of channels differ")
        worst = psnr if worst is None else min(worst, psnr)
        rows += 1
    if rows:
        print(f"{rows} images; worst PSNR {worst:.1f} dB")
    return status


if __name__ == "__main__":
    sys.exit(main())
