"""Draws the app and tray icons and makes the alert chime, using only the standard library.

Run from the project folder: python3 tools/make-assets.py
Then: npx tauri icon src-tauri/icons/app-1024.png  (makes every icon size, .ico and .icns)
"""
import math, struct, wave, zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ICONS = ROOT / "src-tauri" / "icons"
PUBLIC = ROOT / "public"


def png(path, w, h, px):
    raw = b"".join(b"\x00" + bytes(px[y * w * 4:(y + 1) * w * 4]) for y in range(h))
    def chunk(t, d):
        c = struct.pack(">I", len(d)) + t + d
        return c + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    data = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")
    path.write_bytes(data)


def rounded(x, y, x0, y0, x1, y1, r):
    """True if (x, y) is inside the rounded rectangle."""
    cx = min(max(x, x0 + r), x1 - r)
    cy = min(max(y, y0 + r), y1 - r)
    return (x - cx) ** 2 + (y - cy) ** 2 <= r * r


def draw(size, shapes, ss=4):
    """shapes: list of (test(x, y) in 0..1 space, rgba). Later shapes paint over earlier."""
    n = size * ss
    px = bytearray(size * size * 4)
    for y in range(size):
        for x in range(size):
            acc = [0.0, 0.0, 0.0, 0.0]
            for sy in range(ss):
                for sx in range(ss):
                    u, v = (x * ss + sx + 0.5) / n, (y * ss + sy + 0.5) / n
                    col = (0, 0, 0, 0)
                    for test, rgba in shapes:
                        if test(u, v):
                            col = rgba
                    a = col[3] / 255
                    acc[0] += col[0] * a; acc[1] += col[1] * a; acc[2] += col[2] * a; acc[3] += a
            k = ss * ss
            a = acc[3] / k
            i = (y * size + x) * 4
            if a > 0:
                px[i:i + 4] = bytes([round(acc[0] / acc[3]), round(acc[1] / acc[3]), round(acc[2] / acc[3]), round(a * 255)])
    return px


def tiles(pad, gap, r, colours, circle_at=None):
    """A 2x2 grid of rounded tiles. circle_at turns one tile into a circle, the alert shape."""
    out = []
    cell = (1 - 2 * pad - gap) / 2
    for i, col in enumerate(colours):
        cx0 = pad + (i % 2) * (cell + gap)
        cy0 = pad + (i // 2) * (cell + gap)
        if i == circle_at:
            mx, my, rad = cx0 + cell / 2, cy0 + cell / 2, cell / 2
            out.append((lambda u, v, mx=mx, my=my, rad=rad: (u - mx) ** 2 + (v - my) ** 2 <= rad * rad, col))
        else:
            out.append((lambda u, v, a=cx0, b=cy0: rounded(u, v, a, b, a + cell, b + cell, r), col))
    return out


BLUE = (91, 141, 239, 255)
AMBER = (240, 163, 94, 255)
GREEN = (111, 207, 158, 255)
GREY = (150, 145, 160, 255)
BG = (31, 28, 36, 255)

ICONS.mkdir(parents=True, exist_ok=True)
PUBLIC.mkdir(parents=True, exist_ok=True)

# Tray: coloured so it reads on light and dark Windows taskbars. macOS uses the shape only.
for name, size in (("tray.png", 32), ("tray@2x.png", 64)):
    png(ICONS / name, size, size, draw(size, tiles(0.06, 0.12, 0.08, [BLUE] * 4)))
for name, size in (("tray-alert.png", 32), ("tray-alert@2x.png", 64)):
    png(ICONS / name, size, size, draw(size, tiles(0.06, 0.12, 0.08, [BLUE, AMBER, BLUE, BLUE], circle_at=1)))

# App icon at 1024, the size `tauri icon` wants.
size = 1024
shapes = [(lambda u, v: rounded(u, v, 0.04, 0.04, 0.96, 0.96, 0.2), BG)]
shapes += tiles(0.2, 0.08, 0.06, [BLUE, AMBER, GREEN, GREY])
png(ICONS / "app-1024.png", size, size, draw(size, shapes, ss=2))

# Chime: two soft notes with a quick fade, 0.35 s.
rate = 22050
frames = bytearray()
for i in range(int(rate * 0.35)):
    t = i / rate
    f = 880 if t < 0.12 else 1175
    env = min(1, t * 60) * math.exp(-6 * (t if t < 0.12 else t - 0.12))
    frames += struct.pack("<h", int(9000 * env * math.sin(2 * math.pi * f * t)))
with wave.open(str(PUBLIC / "chime.wav"), "wb") as w:
    w.setnchannels(1); w.setsampwidth(2); w.setframerate(rate); w.writeframes(bytes(frames))
print("icons and chime written")
