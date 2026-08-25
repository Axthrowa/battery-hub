"""Rebuild the Windows icon set from app-icon.png.

The artwork is a tall battery, so a plain square downscale leaves a two-pixel
sliver at 16x16 — the size Task Manager and the tray use. Small entries are
therefore cut from a square crop around the battery body, which fills the
canvas, while large entries keep the whole device.

Usage (Windows, needs Pillow):
    python scripts/make-icons.py
"""

from __future__ import annotations

import struct
from io import BytesIO
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "app-icon.png"
ICONS = ROOT / "src-tauri" / "icons"

# Sizes that are too small for the full silhouette to survive.
CROPPED_SIZES = (16, 20, 24, 32)
FULL_SIZES = (48, 64, 128, 256)


def square_crop(img: Image.Image) -> Image.Image:
    """Square region centred on the artwork, as wide as the artwork itself."""
    box = img.getbbox()
    if not box:
        return img
    left, top, right, bottom = box
    width = right - left
    centre_y = (top + bottom) // 2
    half = width // 2
    top = max(0, centre_y - half)
    bottom = min(img.height, centre_y + half)
    return img.crop((left, top, right, bottom))


def variant(img: Image.Image, size: int, cropped: bool) -> Image.Image:
    source = square_crop(img) if cropped else img
    side = max(source.size)
    canvas = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    canvas.paste(source, ((side - source.width) // 2, (side - source.height) // 2), source)
    return canvas.resize((size, size), Image.LANCZOS)


def png_bytes(img: Image.Image) -> bytes:
    buffer = BytesIO()
    img.save(buffer, format="PNG")
    return buffer.getvalue()


def write_ico(path: Path, entries: list[tuple[int, bytes]]) -> None:
    """Assemble a PNG-compressed ICO so each size can carry its own artwork."""
    header = struct.pack("<HHH", 0, 1, len(entries))
    offset = len(header) + 16 * len(entries)
    directory = b""
    for size, payload in entries:
        directory += struct.pack(
            "<BBBBHHII",
            0 if size >= 256 else size,
            0 if size >= 256 else size,
            0,
            0,
            1,
            32,
            len(payload),
            offset,
        )
        offset += len(payload)
    path.write_bytes(header + directory + b"".join(payload for _, payload in entries))


def main() -> int:
    img = Image.open(SOURCE).convert("RGBA")

    entries = [(size, png_bytes(variant(img, size, True))) for size in CROPPED_SIZES]
    entries += [(size, png_bytes(variant(img, size, False))) for size in FULL_SIZES]
    entries.sort(key=lambda item: item[0])
    write_ico(ICONS / "icon.ico", entries)

    # Small standalone PNGs follow the same rule; the tray renders at 16-24 px.
    variant(img, 32, True).save(ICONS / "32x32.png")
    variant(img, 32, True).save(ICONS / "tray.png")
    variant(img, 128, False).save(ICONS / "128x128.png")
    variant(img, 256, False).save(ICONS / "128x128@2x.png")
    variant(img, 512, False).save(ICONS / "icon.png")
    print(f"wrote {ICONS/'icon.ico'} with sizes {[size for size, _ in entries]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
