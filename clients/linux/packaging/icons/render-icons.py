#!/usr/bin/env python3
"""Render the hicolor PNG icons from the same geometry as the SVG.

The app icon is the tray icon without a status badge (agent::tray::icon_rgba):
a brand-blue disc with a white ring and a white centre dot, drawn on a 32-unit
grid and scaled. Edges are anti-aliased analytically (1 px soft edge), so every
size is rendered directly instead of being downscaled.

    python3 packaging/icons/render-icons.py

writes packaging/icons/hicolor/<N>x<N>/apps/io.github.dosibridge.DosiTracker.png
for every size the packages install. The output is deterministic; run it again
after changing the geometry here and in the SVG.
"""

import os
import struct
import zlib

APP_ID = "io.github.dosibridge.DosiTracker"
SIZES = (16, 22, 24, 32, 48, 64, 128, 256, 512)
BRAND = (0x00, 0x6B, 0xFF)
WHITE = (0xFF, 0xFF, 0xFF)

# The 32-unit design (matches the SVG): disc, ring between two radii, dot.
DISC_R = 15.5
RING_OUTER_R = 10.0
RING_INNER_R = 7.25
DOT_R = 3.5


def cover(d, r):
    """Coverage of a disc of radius r at distance d (1 px soft edge)."""
    return min(1.0, max(0.0, r - d + 0.5))


def render(size):
    k = size / 32.0
    centre = size / 2.0
    rows = []
    for y in range(size):
        row = bytearray([0])  # PNG filter type 0 (None)
        for x in range(size):
            d = ((x + 0.5 - centre) ** 2 + (y + 0.5 - centre) ** 2) ** 0.5
            # Premultiplied "over" compositing, back to front.
            r = g = b = a = 0.0
            for color, alpha in (
                (BRAND, cover(d, DISC_R * k)),
                (WHITE, cover(d, RING_OUTER_R * k) - cover(d, RING_INNER_R * k)),
                (WHITE, cover(d, DOT_R * k)),
            ):
                alpha = min(1.0, max(0.0, alpha))
                r = color[0] / 255.0 * alpha + r * (1.0 - alpha)
                g = color[1] / 255.0 * alpha + g * (1.0 - alpha)
                b = color[2] / 255.0 * alpha + b * (1.0 - alpha)
                a = alpha + a * (1.0 - alpha)
            if a > 0.0:
                px = (r / a, g / a, b / a, a)
            else:
                px = (0.0, 0.0, 0.0, 0.0)
            row.extend(int(round(min(1.0, v) * 255)) for v in px)
        rows.append(bytes(row))
    return b"".join(rows)


def chunk(kind, data):
    body = kind + data
    return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)


def png(size):
    header = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)  # 8-bit RGBA
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(render(size), 9))
        + chunk(b"IEND", b"")
    )


def main():
    root = os.path.join(os.path.dirname(os.path.abspath(__file__)), "hicolor")
    for size in SIZES:
        out_dir = os.path.join(root, f"{size}x{size}", "apps")
        os.makedirs(out_dir, exist_ok=True)
        path = os.path.join(out_dir, f"{APP_ID}.png")
        with open(path, "wb") as f:
            f.write(png(size))
        print(path)


if __name__ == "__main__":
    main()
