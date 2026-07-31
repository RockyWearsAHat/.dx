#!/usr/bin/env python3
"""Render packaging/icon.svg to the PNG sizes browsers and stores ask for.

    ./packaging/build-icons.py            -> editor/github/icons/dx-{16,32,48,128}.png
    ./packaging/build-icons.py --icns F   -> F, the .icns DX.app and every .dx file wear

The results are committed, because `dx` carries the extension inside the binary and a binary
cannot embed what the tree does not have -- the same reason the wasm is committed. This is a
release tool and is not in the shipped binary.

Why an icon is not optional: the Chrome Web Store rejects a submission without a 128px icon,
addons.mozilla.org wants one, and Safari's converter warns and produces an extension with no
icon at all -- which is then what a reader hunts for in Safari's settings.

Why this rasterizes instead of driving a browser: headless Chrome's --screenshot writes a
fully transparent image on some machines (measured: every pixel (0,0,0,0), for a plain red
page, in old and new headless alike). An icon that silently renders blank is exactly the
defect a file-size check waves through, so the pixels are computed here where they can be
asserted. The icon is only filled rectangles, which makes that a few lines rather than a
rendering engine.
"""

import re
import struct
import subprocess
import sys
import tempfile
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SVG = ROOT / "packaging" / "icon.svg"
OUT = ROOT / "editor" / "github" / "icons"
SIZES = (16, 32, 48, 128)

# What an .icns holds, as (pixels, iconset name). Finder picks from these for the app, for
# every .dx file on the disk, and for the icon that flies out of the Dock -- so the large
# sizes are not decoration, they are the ones a person actually looks at.
ICNS_SIZES = (
    (16, "icon_16x16"),
    (32, "icon_16x16@2x"),
    (32, "icon_32x32"),
    (64, "icon_32x32@2x"),
    (128, "icon_128x128"),
    (256, "icon_128x128@2x"),
    (256, "icon_256x256"),
    (512, "icon_256x256@2x"),
    (512, "icon_512x512"),
    (1024, "icon_512x512@2x"),
)

# Each pixel is sampled this many times per axis, then averaged. Rectangle edges land on
# fractional pixels at 16px, and without this they alternate between hard-on and hard-off --
# which is what makes a small icon look broken rather than small.
SUPERSAMPLE = 4

RECT = re.compile(
    r'<rect\s+x="([\d.]+)"\s+y="([\d.]+)"\s+width="([\d.]+)"\s+height="([\d.]+)"\s+fill="#([0-9a-fA-F]{6})"\s*/>'
)


def parse(svg_text):
    """The viewBox size and every filled rectangle, in document order.

    Order matters: rectangles are painted in the order they appear, so a later one covers an
    earlier one exactly as the SVG says.
    """
    # Comments first: the file explains itself, and prose naming a <rect> or a <path> is not a
    # shape. Counting them as shapes is how this refused to build its own icon once.
    svg_text = re.sub(r"<!--.*?-->", "", svg_text, flags=re.DOTALL)

    box = re.search(r'viewBox="0 0 ([\d.]+) ([\d.]+)"', svg_text)
    if not box:
        sys.exit("icon.svg has no viewBox")
    side = float(box.group(1))
    if side != float(box.group(2)):
        sys.exit("icon.svg must be square")

    rects = [
        (float(x), float(y), float(w), float(h), bytes.fromhex(fill))
        for x, y, w, h, fill in RECT.findall(svg_text)
    ]

    # Anything this cannot draw has to be an error. A silently ignored <path> or <circle>
    # would leave the icon missing a piece that is plainly there in the source.
    drawn = len(rects)
    declared = svg_text.count("<rect")
    shapes = len(re.findall(r"<(path|circle|ellipse|polygon|line|text|g)\b", svg_text))
    if drawn != declared or shapes:
        sys.exit(
            f"icon.svg has shapes this renderer does not draw "
            f"({declared} rects declared, {drawn} understood, {shapes} other shapes). "
            f"Keep the icon to filled <rect> elements, or teach build-icons.py the rest."
        )
    return side, rects


def render(side, rects, size):
    """The icon at `size` x `size`, as RGBA rows.

    Supersampling is dropped above 128px: the source is a 128-unit square, so every larger
    size an .icns asks for is an exact integer scale and every rectangle edge already lands on
    a pixel boundary. Averaging 16 identical samples per pixel at 1024px would cost minutes to
    reproduce the same bytes.
    """
    samples = SUPERSAMPLE if size <= 128 else 1
    scale = size * samples / side
    span = size * samples

    # Supersampled canvas, transparent to start: the icon is a sheet on the browser's own
    # surface, not a square of white.
    canvas = bytearray(span * span * 4)
    for x, y, w, h, colour in rects:
        left, right = round(x * scale), round((x + w) * scale)
        top, bottom = round(y * scale), round((y + h) * scale)
        for row in range(max(top, 0), min(bottom, span)):
            base = (row * span + max(left, 0)) * 4
            pixel = colour + b"\xff"
            canvas[base : base + (min(right, span) - max(left, 0)) * 4] = pixel * (
                min(right, span) - max(left, 0)
            )

    # Average each block back down to one pixel, alpha included so edges fade rather than jag.
    out = bytearray()
    for row in range(size):
        out.append(0)  # PNG filter: none
        for column in range(size):
            totals = [0, 0, 0, 0]
            for dy in range(samples):
                start = ((row * samples + dy) * span + column * samples) * 4
                for dx in range(samples):
                    pixel = canvas[start + dx * 4 : start + dx * 4 + 4]
                    for channel in range(4):
                        totals[channel] += pixel[channel]
            count = samples * samples
            out.extend(total // count for total in totals)
    return bytes(out)


def png(size, rows):
    """A PNG file holding `rows`, which are already filtered scanlines."""

    def chunk(kind, payload):
        return (
            struct.pack(">I", len(payload))
            + kind
            + payload
            + struct.pack(">I", zlib.crc32(kind + payload))
        )

    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(rows, 9))
        + chunk(b"IEND", b"")
    )


def write(side, rects, size, path):
    """Draw the icon at `size` into `path`, and return how many pixels carry ink.

    The ink count is the check that matters, and the one a byte count cannot make. A blank
    render is the failure this whole file exists because of.
    """
    rows = render(side, rects, size)
    opaque = sum(1 for index in range(3, len(rows), 4) if rows[index] > 0)
    if opaque < size * size // 4:
        sys.exit(f"the {size}px icon came out (nearly) empty -- {opaque} opaque pixels")
    path.write_bytes(png(size, rows))
    return opaque


def icns(side, rects, target):
    """Write the macOS icon file DX.app and every .dx document wear.

    `iconutil` is the only thing that writes the format, and it takes a directory of
    conventionally named PNGs -- so the directory is a temporary one and only the .icns
    survives. Nothing here is committed: the app is built from icon.svg like everything else.
    """
    with tempfile.TemporaryDirectory() as work:
        iconset = Path(work) / "dx.iconset"
        iconset.mkdir()
        for size, name in ICNS_SIZES:
            write(side, rects, size, iconset / f"{name}.png")
        target.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(
            ["iconutil", "--convert", "icns", "--output", str(target), str(iconset)],
            check=True,
        )
    print(f"  icns   {target.stat().st_size:6} bytes  {target}")


def main():
    side, rects = parse(SVG.read_text(encoding="utf-8"))

    if "--icns" in sys.argv:
        index = sys.argv.index("--icns") + 1
        if index >= len(sys.argv):
            sys.exit("--icns needs a file to write")
        icns(side, rects, Path(sys.argv[index]))
        return

    OUT.mkdir(parents=True, exist_ok=True)
    for size in SIZES:
        path = OUT / f"dx-{size}.png"
        opaque = write(side, rects, size, path)
        print(f"  {size:3}px  {path.stat().st_size:6} bytes  {opaque} opaque pixels")

    print(f"wrote {OUT}")


if __name__ == "__main__":
    main()
