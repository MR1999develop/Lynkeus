# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 MR
"""Genera l'icona di Lynkeus Searcher: una lente sopra un foglio di testo.

Solo libreria standard. Ogni pixel e' campionato su una griglia SSxSS
(supersampling) e le forme sono definite in uno spazio 256x256.
Uso: python -I assets/src/genera_icona.py
"""
import math
import os
import struct
import zlib

PNG_SIZES = (16, 24, 32, 48, 64, 128, 256)
ICO_SIZES = (16, 24, 32, 48, 256)

SLATE = (43, 58, 74)      # contorno del foglio
PAPER = (244, 246, 248)   # foglio
LINE = (136, 150, 165)    # righe di testo
ACCENT = (31, 111, 235)   # unico colore d'accento (blu)
GLASS = (234, 242, 255)   # interno della lente

LENS_C = (156.0, 128.0)
LENS_R_OUT, LENS_R_IN = 56.0, 38.0
HANDLE_W = 24.0
_D = math.sqrt(0.5)
HANDLE_A = (LENS_C[0] + _D * 50, LENS_C[1] + _D * 50)
HANDLE_B = (LENS_C[0] + _D * 108, LENS_C[1] + _D * 108)


def in_round_rect(x, y, x0, y0, x1, y1, r):
    cx = min(max(x, x0 + r), x1 - r)
    cy = min(max(y, y0 + r), y1 - r)
    return (x - cx) ** 2 + (y - cy) ** 2 <= r * r and x0 <= x <= x1 and y0 <= y <= y1


def in_capsule(x, y, a, b, half_w):
    dx, dy = b[0] - a[0], b[1] - a[1]
    t = ((x - a[0]) * dx + (y - a[1]) * dy) / (dx * dx + dy * dy)
    t = min(max(t, 0.0), 1.0)
    px, py = a[0] + t * dx, a[1] + t * dy
    return (x - px) ** 2 + (y - py) ** 2 <= half_w * half_w


def sample(x, y):
    """Colore RGBA del punto (x, y) nello spazio 256x256, dal basso verso l'alto."""
    color = None
    if in_round_rect(x, y, 36, 24, 180, 228, 16):
        color = SLATE
        if in_round_rect(x, y, 48, 36, 168, 216, 8):
            color = PAPER
        for (ly, lx1) in ((60, 154), (92, 154), (196, 124)):
            if 64 <= x <= lx1 and ly <= y <= ly + 14:
                color = LINE
    d = math.hypot(x - LENS_C[0], y - LENS_C[1])
    if d <= LENS_R_IN:
        color = GLASS
    elif d <= LENS_R_OUT:
        color = ACCENT
    elif in_capsule(x, y, HANDLE_A, HANDLE_B, HANDLE_W / 2):
        color = ACCENT
    return color


def render(size):
    ss = 8 if size <= 32 else 4
    scale = 256.0 / size
    rows = []
    for py in range(size):
        row = bytearray([0])  # filtro PNG: None
        for px in range(size):
            r = g = b = a = 0
            for sy in range(ss):
                for sx in range(ss):
                    c = sample((px + (sx + 0.5) / ss) * scale, (py + (sy + 0.5) / ss) * scale)
                    if c:
                        r += c[0]; g += c[1]; b += c[2]; a += 1
            n = ss * ss
            if a:
                row += bytes((round(r / a), round(g / a), round(b / a), round(255 * a / n)))
            else:
                row += bytes(4)
        rows.append(bytes(row))
    return b"".join(rows)


def png_bytes(size):
    def chunk(tag, data):
        body = tag + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)

    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr)
            + chunk(b"IDAT", zlib.compress(render(size), 9)) + chunk(b"IEND", b""))


def ico_bytes(images):
    head = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    entries, blobs = b"", b""
    for size, data in images:
        dim = 0 if size >= 256 else size
        entries += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(data), offset)
        blobs += data
        offset += len(data)
    return head + entries + blobs


def main():
    out = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    png_dir = os.path.join(out, "png")
    os.makedirs(png_dir, exist_ok=True)
    data = {s: png_bytes(s) for s in PNG_SIZES}
    for s, d in data.items():
        with open(os.path.join(png_dir, f"icon-{s}.png"), "wb") as f:
            f.write(d)
    with open(os.path.join(out, "icon-256.png"), "wb") as f:
        f.write(data[256])
    with open(os.path.join(out, "icon.ico"), "wb") as f:
        f.write(ico_bytes([(s, data[s]) for s in ICO_SIZES]))


if __name__ == "__main__":
    main()
