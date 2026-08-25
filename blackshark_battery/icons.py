"""Small tray icons drawn with Pillow — no asset files required."""

from __future__ import annotations

from functools import lru_cache

from PIL import Image, ImageDraw, ImageFont

SIZE = 64


def _color_for(percent: int | None, charging: bool, missing: bool) -> tuple[int, int, int]:
    if missing or percent is None:
        return (150, 150, 155)
    if charging:
        return (56, 176, 222)
    if percent <= 15:
        return (220, 70, 70)
    if percent <= 35:
        return (230, 170, 50)
    return (70, 190, 110)


@lru_cache(maxsize=8)
def _font(size: int) -> ImageFont.ImageFont:
    for name in ("segoeuib.ttf", "segoeui.ttf", "arialbd.ttf", "arial.ttf", "tahoma.ttf"):
        try:
            return ImageFont.truetype(name, size)
        except OSError:
            continue
    return ImageFont.load_default()


@lru_cache(maxsize=128)
def make_icon(percent: int | None, *, charging: bool = False, missing: bool = False) -> Image.Image:
    """Cached: the tray only ever shows ~200 distinct icons, and each redraw
    costs a Pillow render plus a Win32 HICON rebuild inside pystray."""
    color = _color_for(percent, charging, missing)
    img = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)
    draw.rounded_rectangle((1, 1, SIZE - 2, SIZE - 2), radius=12, fill=(20, 22, 26, 240), outline=color, width=3)

    if missing or percent is None:
        text = "?"
        font = _font(36)
    elif percent >= 100:
        text = "100"
        font = _font(22)
    else:
        text = str(int(percent))
        font = _font(32 if percent >= 10 else 36)

    bbox = draw.textbbox((0, 0), text, font=font)
    tw, th = bbox[2] - bbox[0], bbox[3] - bbox[1]
    x = (SIZE - tw) / 2 - bbox[0]
    y = (SIZE - th) / 2 - bbox[1] - 1
    draw.text((x, y), text, font=font, fill=color)

    if charging and not missing:
        draw.rectangle((SIZE - 10, 6, SIZE - 6, 14), fill=color)
    return img
