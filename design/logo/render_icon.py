#!/usr/bin/env python3
"""Render the LogLens logo to PNG/ICNS/ICO without third-party deps.

Source of truth: logo-final.svg. Geometry is expressed at a 512x512 canvas
and scaled to every requested size with 4x supersampling for clean edges.
Pillow is the only requirement.
"""
import os
from PIL import Image, ImageDraw

ROOT = os.path.dirname(os.path.abspath(__file__))
BASE = 512.0
SS = 4  # supersampling factor
TOP = (76, 155, 255)     # #4C9BFF
BOTTOM = (43, 103, 224)  # #2B67E0
ROBOT_EYE = (43, 103, 224)
WHITE = (255, 255, 255)
RED = (255, 123, 114)
YELLOW = (255, 200, 87)
GREEN = (91, 224, 160)


def draw_logo(size):
    n = size * SS
    k = n / BASE

    def box(x1, y1, x2, y2):
        return [round(x1 * k), round(y1 * k), round(x2 * k), round(y2 * k)]

    def lw(v):
        return max(1, round(v * k))

    canvas = Image.new("RGBA", (n, n), (0, 0, 0, 0))

    # full-bleed vertical gradient, then clip to rounded square via mask
    grad = Image.new("RGBA", (n, n), (0, 0, 0, 0))
    gd = ImageDraw.Draw(grad)
    for y in range(n):
        t = y / max(1, n - 1)
        color = tuple(round(TOP[i] + (BOTTOM[i] - TOP[i]) * t) for i in range(3)) + (255,)
        gd.line([(0, y), (n, y)], fill=color)
    mask = Image.new("L", (n, n), 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, n - 1, n - 1], radius=lw(112), fill=255)
    canvas.paste(grad, (0, 0), mask)

    d = ImageDraw.Draw(canvas, "RGBA")

    # log panel
    d.rounded_rectangle(box(88, 156, 424, 378), radius=lw(26),
                        outline=WHITE + (255,), width=lw(18))

    # level dots + log lines
    for cy, dot in ((216, RED), (268, YELLOW), (320, GREEN)):
        d.ellipse(box(121, cy - 11, 143, cy + 11), fill=dot + (255,))
    d.rounded_rectangle(box(160, 208, 336, 224), radius=lw(8), fill=WHITE + (242,))
    d.rounded_rectangle(box(160, 260, 382, 276), radius=lw(8), fill=WHITE + (191,))
    d.rounded_rectangle(box(160, 312, 300, 328), radius=lw(8), fill=WHITE + (140,))

    # android robot peeking above the panel
    d.pieslice(box(300, 108, 396, 204), start=180, end=360, fill=WHITE + (255,))
    for cx in (322, 374):
        d.line([(cx * k, 112 * k), (cx * k, 132 * k)], fill=WHITE + (255,), width=lw(10))
        d.ellipse(box(cx - 7, 99, cx + 7, 113), fill=WHITE + (255,))
    for cx in (326, 370):
        d.ellipse(box(cx - 7, 133, cx + 7, 147), fill=ROBOT_EYE + (255,))

    return canvas.resize((size, size), Image.LANCZOS)


def main():
    out = os.path.join(ROOT, "out")
    os.makedirs(out, exist_ok=True)

    png_sizes = [16, 32, 48, 64, 128, 256, 512, 1024]
    for n in png_sizes:
        draw_logo(n).save(os.path.join(out, f"icon-{n}.png"))

    iconset = os.path.join(out, "icon.iconset")
    os.makedirs(iconset, exist_ok=True)
    pairs = {
        "icon_16x16.png": 16, "icon_16x16@2x.png": 32,
        "icon_32x32.png": 32, "icon_32x32@2x.png": 64,
        "icon_128x128.png": 128, "icon_128x128@2x.png": 256,
        "icon_256x256.png": 256, "icon_256x256@2x.png": 512,
        "icon_512x512.png": 512, "icon_512x512@2x.png": 1024,
    }
    for name, n in pairs.items():
        draw_logo(n).save(os.path.join(iconset, name))

    draw_logo(256).save(
        os.path.join(out, "icon.ico"),
        sizes=[(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
    )
    print("PNG sizes:", ", ".join(str(n) for n in png_sizes))
    print("Wrote", out)


if __name__ == "__main__":
    main()
