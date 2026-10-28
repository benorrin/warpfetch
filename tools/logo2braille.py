#!/usr/bin/env python3
"""Convert a single-colour logo (SVG or PNG) to braille art for src/logos.rs.

Usage: tools/logo2braille.py [--bold N] LOGO [MAX_COLS] [MAX_ROWS]

Every braille character is a 2x4 grid of dots, and terminal cells are about
twice as tall as they are wide, so the dots come out square. The logo is scaled
to fit in MAX_COLS x MAX_ROWS cells (default 28 x 13) keeping its aspect ratio.

Needs ksvgtopng (KDE) for SVG input and ImageMagick's `magick`. Anything
non-transparent in the image counts as ink. --bold N thickens thin strokes
by N pixels (at 1024px) before scaling down.
"""
import os
import subprocess
import sys
import tempfile


def load_pgm(path):
    data = open(path, "rb").read()
    magic, w, h, maxval, pixels = data.split(maxsplit=4)
    assert magic == b"P5", "expected binary PGM"
    w, h, maxval = int(w), int(h), int(maxval)
    return w, h, [[pixels[y * w + x] * 2 < maxval for x in range(w)] for y in range(h)]


def braille(w, h, ink):
    dots = [(0, 0, 0x01), (0, 1, 0x02), (0, 2, 0x04), (1, 0, 0x08),
            (1, 1, 0x10), (1, 2, 0x20), (0, 3, 0x40), (1, 3, 0x80)]
    rows = []
    for cy in range(0, h, 4):
        row = ""
        for cx in range(0, w, 2):
            v = sum(bit for dx, dy, bit in dots
                    if cy + dy < h and cx + dx < w and ink[cy + dy][cx + dx])
            row += chr(0x2800 + v) if v else " "
        rows.append(row.rstrip())
    while rows and not rows[-1]:
        rows.pop()
    while rows and not rows[0]:
        rows.pop(0)
    return rows


def main():
    args = sys.argv[1:]
    bold = 0
    if args[:1] == ["--bold"]:
        bold, args = int(args[1]), args[2:]
    src = args[0]
    cols = int(args[1]) if len(args) > 1 else 28
    rows = int(args[2]) if len(args) > 2 else 13
    with tempfile.TemporaryDirectory() as tmp:
        png = os.path.join(tmp, "logo.png")
        if src.endswith(".svg"):
            subprocess.run(["ksvgtopng", "1024", "1024", os.path.abspath(src), png], check=True)
        else:
            png = src
        pgm = os.path.join(tmp, "logo.pgm")
        thicken = ["-morphology", "Erode", f"Disk:{bold}"] if bold else []
        subprocess.run(["magick", png, "-alpha", "extract", "-threshold", "50%", "-negate",
                        *thicken, "-trim", "+repage", "-filter", "Box",
                        "-resize", f"{cols * 2}x{rows * 4}", "-threshold", "50%", pgm], check=True)
        for line in braille(*load_pgm(pgm)):
            print(line)


if __name__ == "__main__":
    main()
