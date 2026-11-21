#!/usr/bin/env python3
"""Make the colour variants of the program icon (Configuration > General >
Application icon: Blue, Red, Orange, Green, Colors) from the default icon
assets/appicons/pwtool_default_<size>.png: the red frame of the window is
recoloured; the yellow buttons, the white field and the shield keep their
colours. Writes assets/appicons/pwtool_icon_<variant>_<size>.png.

Usage: tools/make_icon_variants.py   (needs Pillow)
"""
import colorsys
import os
import sys

from PIL import Image

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "assets", "appicons")
SIZES = [16, 32, 48, 256]
# target hue (0..1) of the frame; None: a hue running around the icon
VARIANTS = {"blue": 0.60, "red": 0.985, "orange": 0.075, "green": 0.36, "colored": None}


def is_frame_red(h, s, v):
    return (h < 0.06 or h > 0.94) and s > 0.35 and v > 0.12


def shield_spans(img):
    """Per row of the lower right part: the x range of the shield (between
    its leftmost and rightmost yellow pixels; the red check mark is inside)."""
    w, h = img.size
    px = img.load()
    spans = {}
    for y in range(h // 2, h):
        xs = []
        for x in range(w // 2, w):
            r, g, b, a = px[x, y]
            hh, ss, vv = colorsys.rgb_to_hsv(r / 255, g / 255, b / 255)
            if a > 0 and 0.07 < hh < 0.2 and ss > 0.4 and vv > 0.4:
                xs.append(x)
        if xs:
            spans[y] = (min(xs), max(xs))
    return spans


def recolor(img, hue):
    img = img.convert("RGBA")
    w, h = img.size
    px = img.load()
    spans = shield_spans(img)
    for y in range(h):
        for x in range(w):
            r, g, b, a = px[x, y]
            if a == 0:
                continue
            hh, ss, vv = colorsys.rgb_to_hsv(r / 255, g / 255, b / 255)
            span = spans.get(y)
            if not is_frame_red(hh, ss, vv) or (span and span[0] <= x <= span[1]):
                continue
            # keep the shading: shift the hue, keep saturation and value
            t = hue if hue is not None else ((x / w) * 0.75 + (y / h) * 0.25) % 1.0
            nr, ng, nb = colorsys.hsv_to_rgb(t, ss, vv)
            px[x, y] = (round(nr * 255), round(ng * 255), round(nb * 255), a)
    return img


def main():
    for size in SIZES:
        src = os.path.join(ROOT, f"pwtool_default_{size}.png")
        base = Image.open(src)
        for name, hue in VARIANTS.items():
            out = os.path.join(ROOT, f"pwtool_icon_{name}_{size}.png")
            recolor(base, hue).save(out, optimize=True)
            print(out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
