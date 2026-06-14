#!/usr/bin/env python3
"""Generate a simple app icon (PNG) for Tauri — no external deps."""
import os
import struct
import zlib

SIZE = 512


def png(path, pixels):
    def chunk(tag, data):
        c = tag + data
        return struct.pack(">I", len(data)) + c + struct.pack(">I", zlib.crc32(c) & 0xFFFFFFFF)

    raw = bytearray()
    for y in range(SIZE):
        raw.append(0)  # filter type 0
        for x in range(SIZE):
            raw += bytes(pixels(x, y))
    sig = b"\x89PNG\r\n\x1a\n"
    ihdr = struct.pack(">IIBBBBB", SIZE, SIZE, 8, 6, 0, 0, 0)  # 8-bit RGBA
    idat = zlib.compress(bytes(raw), 9)
    with open(path, "wb") as f:
        f.write(sig + chunk(b"IHDR", ihdr) + chunk(b"IDAT", idat) + chunk(b"IEND", b""))


def pixel(x, y):
    # Rounded dark square with a centered bullet — a nod to Logseq's outliner.
    cx, cy = SIZE / 2, SIZE / 2
    # background
    r, g, b, a = 0x1A, 0x1B, 0x1E, 255
    # corner rounding
    radius = 90
    inx = min(x, SIZE - 1 - x)
    iny = min(y, SIZE - 1 - y)
    if inx < radius and iny < radius:
        dx, dy = radius - inx, radius - iny
        if dx * dx + dy * dy > radius * radius:
            a = 0
    # centered bullet
    d2 = (x - cx) ** 2 + (y - cy) ** 2
    if d2 < 70 ** 2:
        r, g, b = 0x6C, 0xA6, 0xE6
    return (r, g, b, a)


def main():
    here = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    icons = os.path.join(here, "src-tauri", "icons")
    os.makedirs(icons, exist_ok=True)
    png(os.path.join(icons, "icon.png"), pixel)
    print("wrote", os.path.join(icons, "icon.png"))


if __name__ == "__main__":
    main()
