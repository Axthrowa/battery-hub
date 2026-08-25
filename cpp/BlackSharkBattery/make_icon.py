"""Generate a Razer-green style app.ico (original geometric mark, not an official asset dump)."""
from __future__ import annotations

from pathlib import Path

from PIL import Image, ImageDraw

OUT = Path(__file__).resolve().parent / "app.ico"
GREEN = (68, 214, 44, 255)  # Razer-like green
BLACK = (10, 12, 14, 255)
DARK = (20, 24, 22, 255)


def draw_mark(size: int) -> Image.Image:
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    m = size * 0.08
    d.rounded_rectangle((m, m, size - m, size - m), radius=size * 0.18, fill=BLACK)
    # Triple chevron / stylized heads pointing right (original geometry).
    cx, cy = size * 0.42, size * 0.50
    scale = size * 0.28
    for i, dy in enumerate((-0.55, 0.0, 0.55)):
        y = cy + dy * scale
        pts = [
            (cx - scale * 0.55, y - scale * 0.28),
            (cx + scale * 0.75, y),
            (cx - scale * 0.55, y + scale * 0.28),
            (cx - scale * 0.15, y),
        ]
        d.polygon(pts, fill=GREEN)
    # Eye dots
    r = max(1, size // 28)
    for dy in (-0.55, 0.0, 0.55):
        d.ellipse(
            (
                cx + scale * 0.15 - r,
                cy + dy * scale - r,
                cx + scale * 0.15 + r,
                cy + dy * scale + r,
            ),
            fill=DARK,
        )
    return img


def main() -> None:
    sizes = [16, 24, 32, 48, 64, 128, 256]
    images = [draw_mark(s) for s in sizes]
    images[0].save(
        OUT,
        format="ICO",
        sizes=[(s, s) for s in sizes],
        append_images=images[1:],
    )
    print("wrote", OUT)


if __name__ == "__main__":
    main()
