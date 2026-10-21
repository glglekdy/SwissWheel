"""Builds assets/swisswheel.ico (the exe's icon) from assets/logo.png.

The white logo sits on a dark rounded square so it reads on light and dark
Explorer backgrounds alike. Run from the repo root after changing the logo:

    python assets/make_ico.py

Needs Pillow.
"""

from pathlib import Path

from PIL import Image, ImageDraw

HERE = Path(__file__).parent
SIZES = [16, 20, 24, 32, 40, 48, 64, 256]
BACKGROUND = (0x2B, 0x2B, 0x2B, 255)
SUPERSAMPLE = 4


def logo() -> Image.Image:
    img = Image.open(HERE / "logo.png").convert("RGBA")
    return img.crop(img.getchannel("A").getbbox())


def render(size: int, mark: Image.Image) -> Image.Image:
    big = size * SUPERSAMPLE
    canvas = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    ImageDraw.Draw(canvas).rounded_rectangle(
        (0, 0, big - 1, big - 1), radius=round(big * 0.22), fill=BACKGROUND
    )
    # Small icons give the logo more room so its four pieces stay apart.
    share = 0.8 if size <= 24 else 0.7
    side = round(big * share)
    scaled = mark.resize((side, side), Image.LANCZOS)
    offset = (big - side) // 2
    canvas.alpha_composite(scaled, (offset, offset))
    return canvas.resize((size, size), Image.LANCZOS)


def main() -> None:
    mark = logo()
    images = [render(s, mark) for s in SIZES]
    images[-1].save(
        HERE / "swisswheel.ico",
        sizes=[(s, s) for s in SIZES],
        append_images=images[:-1],
    )


if __name__ == "__main__":
    main()
