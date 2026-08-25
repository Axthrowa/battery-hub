"""Prepare the Razer logo PNG as a Tauri-ready app icon.

Removes the white background and recolours the mark to Razer green so it
reads correctly on both light and dark shells (taskbar, tray, installer).

Usage:
    python scripts/prepare_icon.py [source.png]
"""

from __future__ import annotations

import sys
from pathlib import Path

from PIL import Image

RAZER_GREEN = (68, 214, 44)
OUT_SIZE = 1024
PADDING_RATIO = 0.08


def build_icon(src: Path, dst: Path) -> None:
    img = Image.open(src).convert("RGBA")
    w, h = img.size
    px = img.load()

    for y in range(h):
        for x in range(w):
            r, g, b, a = px[x, y]
            if a == 0:
                continue
            # White background -> transparent; darker ink -> opaque green.
            alpha = 255 - min(r, g, b)
            if alpha < 12:
                px[x, y] = (0, 0, 0, 0)
            else:
                px[x, y] = (*RAZER_GREEN, alpha)

    bbox = img.getbbox()
    if bbox:
        img = img.crop(bbox)

    side = max(img.size)
    pad = int(side * PADDING_RATIO)
    canvas = Image.new("RGBA", (side + pad * 2, side + pad * 2), (0, 0, 0, 0))
    canvas.paste(
        img,
        ((canvas.width - img.width) // 2, (canvas.height - img.height) // 2),
        img,
    )
    canvas = canvas.resize((OUT_SIZE, OUT_SIZE), Image.LANCZOS)

    dst.parent.mkdir(parents=True, exist_ok=True)
    canvas.save(dst, format="PNG")
    print(f"wrote {dst} ({canvas.width}x{canvas.height})")


def main() -> int:
    src = Path(sys.argv[1]) if len(sys.argv) > 1 else Path.home() / "Downloads" / "razer.png"
    if not src.is_file():
        print(f"source not found: {src}", file=sys.stderr)
        return 1
    dst = Path(__file__).resolve().parent.parent / "app-icon.png"
    build_icon(src, dst)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
