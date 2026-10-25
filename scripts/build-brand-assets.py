#!/usr/bin/env python3
"""Regenerate platform icon containers and outlined wordmarks from checked-in art.

Maintainer tool for macOS: requires sips, iconutil, and Python fontTools.
Normal builds use the checked-in outputs and do not need these tools.
The generated raster artwork is preserved; only platform sizing/encoding changes.
"""
from pathlib import Path
import base64
import struct
import subprocess
import tempfile

from fontTools.pens.boundsPen import BoundsPen
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parents[1]
BRAND = ROOT / "crates/daw-ui/assets/brand"


def resize(source, size, destination):
    subprocess.run(
        ["sips", "-z", str(size), str(size), str(source), "--out", str(destination)],
        check=True, stdout=subprocess.DEVNULL,
    )


def icons():
    source = BRAND / "rill-icon-source.png"
    with tempfile.TemporaryDirectory(prefix="rill-brand-") as temporary:
        directory = Path(temporary)
        sizes = {}
        for size in (16, 24, 32, 48, 64, 128, 256, 512, 1024):
            path = directory / f"{size}.png"
            resize(source, size, path)
            sizes[size] = path.read_bytes()
        for size, filename in [(64, "rill-icon-64.png"), (256, "rill-icon-256.png"), (1024, "rill-icon.png")]:
            (BRAND / filename).write_bytes(sizes[size])
        # ICO is a directory of unmodified PNG payloads; no pixel editing here.
        ico_sizes = (16, 24, 32, 48, 64, 128, 256)
        headers = [struct.pack("<HHH", 0, 1, len(ico_sizes))]
        offset = 6 + 16 * len(ico_sizes)
        for size in ico_sizes:
            payload = sizes[size]
            headers.append(struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(payload), offset))
            offset += len(payload)
        (BRAND / "rill.ico").write_bytes(b"".join(headers) + b"".join(sizes[size] for size in ico_sizes))
        iconset = directory / "Rill.iconset"
        iconset.mkdir()
        for size in (16, 32, 128, 256, 512):
            (iconset / f"icon_{size}x{size}.png").write_bytes(sizes[size])
            (iconset / f"icon_{size}x{size}@2x.png").write_bytes(sizes[size * 2])
        subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o", str(BRAND / "rill.icns")], check=True)


def wordmarks():
    # Real font outlines keep the exported logotype as crisp as the UI text.
    font = TTFont(ROOT / "crates/daw-ui/assets/fonts/Inter-SemiBold.ttf")
    glyphs = font.getGlyphSet()
    cmap = font.getBestCmap()
    tracking = font["head"].unitsPerEm * 0.09
    outlines = SVGPathPen(glyphs)
    bounds = BoundsPen(glyphs)
    cursor = 0
    for letter in "RILL":
        glyph = glyphs[cmap[ord(letter)]]
        transform = (1, 0, 0, 1, cursor, 0)
        glyph.draw(TransformPen(outlines, transform))
        glyph.draw(TransformPen(bounds, transform))
        cursor += glyph.width + tracking
    left, bottom, right, top = bounds.bounds
    scale = 128 / (top - bottom)
    word_width = (right - left) * scale
    icon = base64.b64encode((BRAND / "rill-icon-256.png").read_bytes()).decode("ascii")
    width = 32 + 176 + 40 + word_width + 32
    for theme, color in [("light", "#1B1F21"), ("dark", "#E5EAE8")]:
        svg = f'''<svg xmlns="http://www.w3.org/2000/svg" width="{width:.3f}" height="208" viewBox="0 0 {width:.3f} 208" role="img" aria-labelledby="title">
  <title id="title">RILL</title>
  <image x="32" y="16" width="176" height="176" href="data:image/png;base64,{icon}"/>
  <path fill="{color}" transform="translate({248-left*scale:.3f} {40+top*scale:.3f}) scale({scale:.8f} {-scale:.8f})" d="{outlines.getCommands()}"/>
</svg>
'''
        (BRAND / f"rill-logo-{theme}.svg").write_text(svg)


if __name__ == "__main__":
    icons()
    wordmarks()
    print("Updated RILL platform icons and outlined wordmarks.")
