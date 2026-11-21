#!/usr/bin/env python3
"""diff.py <reference.png> <render.png> <diff.png> [limit]

Pixel-close comparison of a browser-rendered design board against Triton's
own render. Font rasterization differs (Skia vs cosmic-text, hinted webfonts
vs embedded statics), so the metric counts pixels whose worst channel differs
by more than a tolerance rather than demanding equality. Writes an amplified
diff image and exits nonzero when the mismatch share exceeds <limit> percent.
"""

import sys

from PIL import Image, ImageChops

TOLERANCE = 24  # per-channel abs difference that counts as a different pixel


def main() -> int:
    ref_path, got_path, out_path = sys.argv[1:4]
    limit = float(sys.argv[4]) if len(sys.argv) > 4 else 5.0

    ref = Image.open(ref_path).convert("RGBA")
    got = Image.open(got_path).convert("RGBA")
    if ref.size != got.size:
        print(f"size mismatch: reference {ref.size}, render {got.size}")
        return 1

    delta = ImageChops.difference(ref, got)
    # Worst-channel difference per pixel.
    r, g, b, a = delta.split()
    worst = ImageChops.lighter(ImageChops.lighter(ImageChops.lighter(r, g), b), a)

    total = worst.size[0] * worst.size[1]
    hist = worst.histogram()
    over = sum(hist[TOLERANCE + 1 :])
    sum_abs = sum(i * n for i, n in enumerate(hist))

    pct = over / total * 100
    print(f"{over} / {total} px over tolerance {TOLERANCE} ({pct:.2f}%)")
    print(f"mean abs channel diff {sum_abs / total:.2f}")

    worst.point(lambda v: 255 if v > TOLERANCE else v * 4).convert("RGB").save(out_path)
    print(f"diff image: {out_path}")

    if pct > limit:
        print(f"FAIL: {pct:.2f}% > limit {limit}%")
        return 1
    print(f"PASS: {pct:.2f}% <= limit {limit}%")
    return 0


if __name__ == "__main__":
    sys.exit(main())
