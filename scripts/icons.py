"""Regenerate the app icons and favicon from docs/brand/imadive-icon.svg.

Needs cairosvg and Pillow (pip install cairosvg pillow) and, for icon.icns, macOS iconutil.
With Homebrew's cairo: DYLD_FALLBACK_LIBRARY_PATH=/opt/homebrew/lib python3 scripts/icons.py
"""

import io
import re
import shutil
import subprocess
import tempfile
from pathlib import Path

import cairosvg
from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "docs/brand/imadive-icon.svg"
DESKTOP = ROOT / "desktop/icons"
PNGS = ROOT / "docs/brand/png"

# The artwork spans x 358..893, y 262..795: square boxes around its centre.
CX, CY = 625.5, 528.5
APP_SIDE = 608  # a little margin, as app icons have
FAVICON_SIDE = 560  # nearly edge to edge: every pixel counts at 16 px

ICO_SIZES = [16, 24, 32, 48, 64, 128, 256]
PNG_SIZES = [16, 32, 48, 64, 128, 180, 192, 256, 512, 1024]


def square(svg: str, side: float) -> str:
    box = f'viewBox="{CX - side / 2:g} {CY - side / 2:g} {side:g} {side:g}"'
    return re.sub(r'viewBox="[^"]*"', box, svg, count=1)


def render(svg: str, size: int) -> Image.Image:
    png = cairosvg.svg2png(bytestring=svg.encode(), output_width=size, output_height=size)
    return Image.open(io.BytesIO(png)).convert("RGBA")


def main() -> None:
    art = SOURCE.read_text()
    app, fav = square(art, APP_SIDE), square(art, FAVICON_SIDE)

    (ROOT / "web/favicon.svg").write_text(fav)

    PNGS.mkdir(exist_ok=True)
    for size in PNG_SIZES:
        render(app, size).save(PNGS / f"imadive-{size}.png")

    render(app, 32).save(DESKTOP / "32x32.png")
    render(app, 128).save(DESKTOP / "128x128.png")
    render(app, 256).save(DESKTOP / "128x128@2x.png")
    render(app, 1024).save(DESKTOP / "icon.png")

    # Small sizes come from the favicon box, so the mark fills more of the few pixels there.
    ico = [render(fav if s <= 32 else app, s) for s in ICO_SIZES]
    ico[-1].save(DESKTOP / "icon.ico", sizes=[(s, s) for s in ICO_SIZES], append_images=ico[:-1])
    ico[3].save(PNGS / "favicon.ico", sizes=[(16, 16), (32, 32), (48, 48)], append_images=[ico[0], ico[2]])

    if shutil.which("iconutil"):
        with tempfile.TemporaryDirectory() as tmp:
            iconset = Path(tmp) / "icon.iconset"
            iconset.mkdir()
            for base in [16, 32, 128, 256, 512]:
                render(app, base).save(iconset / f"icon_{base}x{base}.png")
                render(app, base * 2).save(iconset / f"icon_{base}x{base}@2x.png")
            subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o", str(DESKTOP / "icon.icns")], check=True)
    else:
        print("iconutil not found (macOS only): icon.icns left as it was")


if __name__ == "__main__":
    main()
