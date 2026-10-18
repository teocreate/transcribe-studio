"""Generates the app icon with nothing but the Python standard library.

Draws a rounded-square tile with a vertical blue gradient and a centred audio
waveform, supersampled 4x for clean edges, then writes both a PNG set and a
multi-resolution ICO.
"""

import math
import os
import struct
import zlib

OUT_DIR = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "icons")
SIZES = [16, 32, 48, 64, 128, 256]
SS = 4  # supersampling factor

# Apple-ish system blue, slightly deeper at the bottom of the tile.
TOP = (10, 132, 255)
BOTTOM = (0, 82, 204)
BAR = (255, 255, 255)

# Relative bar heights of the waveform, left to right.
WAVE = [0.30, 0.52, 0.80, 1.00, 0.68, 0.92, 0.44, 0.24]


def rounded_rect_alpha(x, y, w, h, radius):
    """Coverage of a rounded rectangle at a point, 0.0 to 1.0 (hard edged;
    anti-aliasing comes from supersampling)."""
    cx = min(max(x, radius), w - radius)
    cy = min(max(y, radius), h - radius)
    dx = x - cx
    dy = y - cy
    if dx == 0 and dy == 0:
        return 1.0
    return 1.0 if dx * dx + dy * dy <= radius * radius else 0.0


def render(size):
    big = size * SS
    radius = big * 0.225
    # Accumulators for the supersampled grid.
    pixels = bytearray(size * size * 4)

    bar_count = len(WAVE)
    # Waveform occupies the middle 62% of the tile.
    field_w = big * 0.62
    field_x = (big - field_w) / 2.0
    gap_ratio = 0.52
    bar_w = field_w / (bar_count + (bar_count - 1) * gap_ratio)
    gap = bar_w * gap_ratio
    bar_radius = bar_w / 2.0
    max_h = big * 0.50
    center_y = big / 2.0

    def sample(px, py):
        """Returns (r, g, b, a) for one supersample point."""
        if rounded_rect_alpha(px, py, big, big, radius) == 0.0:
            return (0, 0, 0, 0)

        t = py / big
        r = round(TOP[0] + (BOTTOM[0] - TOP[0]) * t)
        g = round(TOP[1] + (BOTTOM[1] - TOP[1]) * t)
        b = round(TOP[2] + (BOTTOM[2] - TOP[2]) * t)

        for i, amp in enumerate(WAVE):
            bx = field_x + i * (bar_w + gap)
            if not (bx - 0.5 <= px <= bx + bar_w + 0.5):
                continue
            half = max(amp * max_h / 2.0, bar_radius)
            # Stadium shape: a rectangle with semicircular caps.
            top = center_y - half + bar_radius
            bot = center_y + half - bar_radius
            cx = min(max(px, bx + bar_radius), bx + bar_w - bar_radius)
            cy = min(max(py, top), bot)
            dx = px - cx
            dy = py - cy
            if dx * dx + dy * dy <= bar_radius * bar_radius:
                return BAR + (255,)
        return (r, g, b, 255)

    for y in range(size):
        for x in range(size):
            acc_r = acc_g = acc_b = acc_a = 0
            for sy in range(SS):
                for sx in range(SS):
                    px = x * SS + sx + 0.5
                    py = y * SS + sy + 0.5
                    r, g, b, a = sample(px, py)
                    acc_r += r * a
                    acc_g += g * a
                    acc_b += b * a
                    acc_a += a
            n = SS * SS
            alpha = acc_a / n
            idx = (y * size + x) * 4
            if alpha <= 0:
                continue
            pixels[idx] = min(255, round(acc_r / acc_a))
            pixels[idx + 1] = min(255, round(acc_g / acc_a))
            pixels[idx + 2] = min(255, round(acc_b / acc_a))
            pixels[idx + 3] = min(255, round(alpha))
    return bytes(pixels)


def png_bytes(size, rgba):
    def chunk(tag, data):
        body = tag + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    raw = bytearray()
    stride = size * 4
    for y in range(size):
        raw.append(0)  # no filter
        raw += rgba[y * stride:(y + 1) * stride]

    header = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
        + chunk(b"IEND", b"")
    )


def main():
    os.makedirs(OUT_DIR, exist_ok=True)
    entries = []
    for size in SIZES:
        data = png_bytes(size, render(size))
        entries.append((size, data))
        with open(os.path.join(OUT_DIR, f"{size}x{size}.png"), "wb") as fh:
            fh.write(data)
        print(f"  {size}x{size}.png  {len(data):>7} B")

    # Tauri also wants these exact names for the bundler.
    for name, size in (("icon.png", 256), ("128x128@2x.png", 256), ("32x32.png", 32)):
        src = dict(entries)[size]
        with open(os.path.join(OUT_DIR, name), "wb") as fh:
            fh.write(src)

    # ICO directory: PNG-compressed entries, supported since Windows Vista.
    count = len(entries)
    offset = 6 + 16 * count
    directory = bytearray(struct.pack("<HHH", 0, 1, count))
    blobs = bytearray()
    for size, data in entries:
        dim = 0 if size >= 256 else size
        directory += struct.pack(
            "<BBBBHHII", dim, dim, 0, 0, 1, 32, len(data), offset + len(blobs)
        )
        blobs += data
    with open(os.path.join(OUT_DIR, "icon.ico"), "wb") as fh:
        fh.write(bytes(directory) + bytes(blobs))
    print(f"  icon.ico     {6 + 16 * count + len(blobs):>7} B  ({count} sizes)")


if __name__ == "__main__":
    main()
