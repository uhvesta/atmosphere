"""Draws the Humata 2 app icon candidates (A, B, C) with the standard library only.

Usage: draw_icon.py --candidate A --size 1024 --out icon.png

Shapes are signed-distance fields in a 1024 design space, rendered at 2x the
output size and box-filtered down, so edges are anti-aliased and the output
is byte-for-byte the same on every run.
"""
import argparse
import math
import struct
import zlib

DESIGN = 1024.0
SS = 2  # supersampling factor per axis
BODY_HALF = 412.0  # 824 px across, per the macOS icon grid

# House colours from apps/humata2/ui/src/theme.rs.
BLUE = (0x0A, 0x6C, 0xFF)
BLUE_LIGHT = (0x3B, 0x8C, 0xFF)
GREEN = (0x34, 0xC7, 0x59)
ORANGE = (0xFF, 0x9F, 0x0A)
TEXT = (0xE8, 0xE8, 0xEA)
INK_TOP = (0x2E, 0x2E, 0x31)
INK_BOTTOM = (0x14, 0x14, 0x15)
LINE = (0x3A, 0x3A, 0x3D)
STEM = (0x6A, 0x6A, 0x70)  # neutral, opaque, so overlaps never show


def lerp(a, b, t):
    return tuple(a[i] + (b[i] - a[i]) * t for i in range(3))


def smooth(edge0, edge1, x):
    t = min(1.0, max(0.0, (x - edge0) / (edge1 - edge0)))
    return t * t * (3 - 2 * t)


def d_squircle(x, y, half=BODY_HALF):
    nx, ny = abs(x - 512) / half, abs(y - 512) / half
    return (((nx**5 + ny**5) ** 0.2) - 1.0) * half


def d_rbox(p, hw, hh, r):
    qx, qy = abs(p[0]) - hw + r, abs(p[1]) - hh + r
    return math.hypot(max(qx, 0), max(qy, 0)) + min(max(qx, qy), 0) - r


def d_circle(x, y, cx, cy, r):
    return math.hypot(x - cx, y - cy) - r


def d_capsule(x, y, a, b, r):
    px, py = x - a[0], y - a[1]
    bx, by = b[0] - a[0], b[1] - a[1]
    h = min(1.0, max(0.0, (px * bx + py * by) / (bx * bx + by * by)))
    return math.hypot(px - bx * h, py - by * h) - r


class Canvas:
    def __init__(self, n):
        self.n = n
        self.k = n / DESIGN
        self.px = [0.0] * (n * n * 4)  # premultiplied RGBA, 0..255

    def paint(self, bbox, sdf, colour, alpha=1.0, soft=0.0):
        """colour(x, y) -> rgb; sdf(x, y) -> distance in design units.

        With soft > 0 the edge fades out smoothly over that many units
        instead of being a hard anti-aliased edge (for shadows).
        """
        k, n = self.k, self.n
        x0 = max(0, int(bbox[0] * k) - 2)
        x1 = min(n, int(bbox[2] * k) + 3)
        y0 = max(0, int(bbox[1] * k) - 2)
        y1 = min(n, int(bbox[3] * k) + 3)
        px = self.px
        for j in range(y0, y1):
            y = (j + 0.5) / k
            for i in range(x0, x1):
                x = (i + 0.5) / k
                d = sdf(x, y)
                if soft:
                    cov = (1.0 - smooth(0.0, soft, d)) * alpha
                else:
                    cov = min(1.0, max(0.0, 0.5 - d * k)) * alpha
                if cov <= 0.0:
                    continue
                r, g, b = colour(x, y)
                o = (j * n + i) * 4
                inv = 1.0 - cov
                px[o] = r * cov + px[o] * inv
                px[o + 1] = g * cov + px[o + 1] * inv
                px[o + 2] = b * cov + px[o + 2] * inv
                px[o + 3] = 255.0 * cov + px[o + 3] * inv

    def png(self, size):
        n, f = self.n, self.n // size
        raw = bytearray()
        for j in range(size):
            raw.append(0)
            for i in range(size):
                acc = [0.0, 0.0, 0.0, 0.0]
                for dj in range(f):
                    base = ((j * f + dj) * n + i * f) * 4
                    for di in range(f):
                        for c in range(4):
                            acc[c] += self.px[base + di * 4 + c]
                a = acc[3] / (f * f)
                if a <= 0:
                    raw += b"\0\0\0\0"
                else:
                    s = 255.0 / a / (f * f)
                    raw += bytes(
                        [min(255, int(acc[c] * s + 0.5)) for c in range(3)]
                        + [min(255, int(a + 0.5))]
                    )
        def chunk(tag, data):
            body = tag + data
            return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))
        return (
            b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
            + chunk(b"IEND", b"")
        )


def solid(c):
    return lambda x, y: c


def vgrad(top, bottom, y0=100.0, y1=924.0):
    return lambda x, y: lerp(top, bottom, min(1.0, max(0.0, (y - y0) / (y1 - y0))))


def body(cv, top, bottom):
    full = (0, 0, 1024, 1024)
    # Soft shadow: a slightly smaller squircle pushed down, so nothing
    # shows above the top edge, fading out over about 56 units.
    cv.paint(
        (40, 40, 984, 984),
        lambda x, y: d_squircle(x, y - 22, BODY_HALF - 8),
        solid((0, 0, 0)),
        0.14,
        soft=56.0,
    )
    cv.paint(full, d_squircle, vgrad(top, bottom))
    # Thin top highlight: the body minus itself shifted down.
    cv.paint(
        full,
        lambda x, y: max(d_squircle(x, y), -d_squircle(x, y - 3)),
        solid((255, 255, 255)),
        0.22,
    )


def candidate_a(cv):
    """A terminal prompt: a blue chevron and a cursor bar on dark ink."""
    body(cv, INK_TOP, INK_BOTTOM)
    chev = vgrad(BLUE_LIGHT, BLUE, 330, 700)
    pts = ((330, 340), (520, 512), (330, 684))
    for a, b in zip(pts, pts[1:]):
        cv.paint((250, 280, 600, 750), lambda x, y, a=a, b=b: d_capsule(x, y, a, b, 48), chev)
    cv.paint((540, 600, 800, 760), lambda x, y: d_capsule(x, y, (590, 684), (710, 684), 48), solid(TEXT))


def candidate_b(cv):
    """Channels: a stack of three rows with status dots on a blue body."""
    body(cv, (0x4D, 0xA0, 0xFF), (0x07, 0x55, 0xE0))
    rows = ((330, 1.0, GREEN), (512, 0.34, ORANGE), (694, 0.34, TEXT))
    for cy, a, dot in rows:
        cv.paint(
            (230, cy - 60, 800, cy + 60),
            lambda x, y, cy=cy: d_rbox((x - 512, y - cy), 270, 58, 58),
            solid((255, 255, 255)),
            a,
        )
        dot_col = dot if a == 1.0 else dot
        cv.paint((250, cy - 40, 340, cy + 40), lambda x, y, cy=cy: d_circle(x, y, 300, cy, 26), solid(dot_col))
        bar = (0x2A, 0x3A, 0x5A) if a == 1.0 else (255, 255, 255)
        cv.paint(
            (350, cy - 20, 760, cy + 20),
            lambda x, y, cy=cy, a=a: d_capsule(x, y, (380, cy), (380 + (330 if a == 1.0 else 250), cy), 15),
            solid(bar),
            0.85 if a == 1.0 else 0.8,
        )


def candidate_c(cv):
    """A fork: one agent session branching into two, on dark ink."""
    body(cv, INK_TOP, INK_BOTTOM)
    top, left, right, split = (512, 300), (338, 694), (686, 694), (512, 452)
    nodes = ((top, BLUE_LIGHT), (left, GREEN), (right, ORANGE))
    segs = ((top, split), (split, left), (split, right))
    box = (200, 180, 830, 880)
    line_w = 29
    node_r = 80
    # Soft drop shadow under the whole mark.
    for a, b in segs:
        cv.paint(box, lambda x, y, a=a, b=b: d_capsule(x, y - 16, a, b, line_w), solid((0, 0, 0)), 0.30, soft=24.0)
    for (cx, cy), _ in nodes:
        cv.paint(box, lambda x, y, cx=cx, cy=cy: d_circle(x, y - 16, cx, cy, node_r), solid((0, 0, 0)), 0.30, soft=24.0)
    # Neutral lines, so only the nodes carry colour.
    for a, b in segs:
        cv.paint(box, lambda x, y, a=a, b=b: d_capsule(x, y, a, b, line_w), solid(STEM))
    cv.paint((470, 410, 560, 500), lambda x, y: d_circle(x, y, 512, 452, line_w), solid(STEM))
    ring = vgrad(INK_TOP, INK_BOTTOM)
    for (cx, cy), col in nodes:
        fill = vgrad(lerp(col, (255, 255, 255), 0.22), lerp(col, (0, 0, 0), 0.14), cy - node_r, cy + node_r)
        cv.paint((cx - 100, cy - 100, cx + 100, cy + 100), lambda x, y, cx=cx, cy=cy: d_circle(x, y, cx, cy, node_r), fill)
        cv.paint((cx - 100, cy - 100, cx + 100, cy + 100), lambda x, y, cx=cx, cy=cy: d_circle(x, y, cx, cy, 34), ring)
    # A cursor in the root node's hole: the nodes are sessions, not commits.
    cv.paint((490, 270, 540, 330), lambda x, y: d_rbox((x - top[0], y - top[1]), 7, 22, 7), solid(TEXT))


CANDIDATES = {"A": candidate_a, "B": candidate_b, "C": candidate_c}


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--candidate", required=True, choices=sorted(CANDIDATES))
    p.add_argument("--size", type=int, default=1024)
    p.add_argument("--out", required=True)
    a = p.parse_args()
    cv = Canvas(a.size * SS)
    CANDIDATES[a.candidate](cv)
    with open(a.out, "wb") as f:
        f.write(cv.png(a.size))


if __name__ == "__main__":
    main()
