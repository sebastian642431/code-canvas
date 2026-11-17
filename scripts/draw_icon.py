import math
import sys

from PIL import Image, ImageChops, ImageDraw, ImageFilter

SIDE = 1024
OVER = 4
S = SIDE * OVER

PLATE = ((28, 33, 48), (11, 13, 19))
LETTER = ((158, 205, 255), (58, 104, 255))
CARD = (255, 180, 84)
DARK = (13, 15, 22)


def gradient(top, bottom, angle=90.0):
    ramp = Image.linear_gradient("L").resize((S, S))
    if angle != 90.0:
        ramp = ramp.rotate(90.0 - angle, resample=Image.BICUBIC)
    channels = [ramp.point(lambda v, a=a, z=z: int(a + (z - a) * v / 255)) for a, z in zip(top, bottom)]
    return Image.merge("RGB", channels).convert("RGBA")


def flat(colour, alpha=255):
    return Image.new("RGBA", (S, S), colour + (alpha,))


def rounded(box, radius):
    shape = Image.new("L", (S, S), 0)
    ImageDraw.Draw(shape).rounded_rectangle([v * OVER for v in box], radius=radius * OVER, fill=255)
    return shape


def arc(centre, radius, width, start, end):
    shape = Image.new("L", (S, S), 0)
    d = ImageDraw.Draw(shape)
    cx, cy = centre[0] * OVER, centre[1] * OVER
    r, w = radius * OVER, width * OVER
    d.arc([cx - r - w / 2, cy - r - w / 2, cx + r + w / 2, cy + r + w / 2], start, end, fill=255, width=int(w))
    for angle in (start, end):
        x, y = cx + r * math.cos(math.radians(angle)), cy + r * math.sin(math.radians(angle))
        d.ellipse([x - w / 2, y - w / 2, x + w / 2, y + w / 2], fill=255)
    return shape


def icon():
    image = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    plate = rounded([44, 44, SIDE - 44, SIDE - 44], 228)
    image.paste(gradient(*PLATE), (0, 0), plate)
    rim = ImageChops.subtract(plate, rounded([49, 49, SIDE - 49, SIDE - 49], 223))
    image.paste(flat((255, 255, 255), 30), (0, 0), rim)

    centre, radius, width = (500, 512), 238, 128
    letter = arc(centre, radius, width, 50, 310)
    small = letter.resize((SIDE // 4, SIDE // 4), Image.BILINEAR).filter(ImageFilter.GaussianBlur(14))
    glow = ImageChops.multiply(small.resize((S, S), Image.BILINEAR), plate).point(lambda v: v * 46 // 255)
    image = Image.alpha_composite(image, Image.merge("RGBA", (*flat((64, 120, 255)).split()[:3], glow)))
    image.paste(gradient(*LETTER, angle=118.0), (0, 0), letter)

    x, y, side = centre[0] + 176, centre[1], 132
    image.paste(flat(DARK), (0, 0), rounded([x - side / 2 - 16, y - side / 2 - 16, x + side / 2 + 16, y + side / 2 + 16], 48))
    image.paste(flat(CARD), (0, 0), rounded([x - side / 2, y - side / 2, x + side / 2, y + side / 2], 34))
    return image.resize((SIDE, SIDE), Image.LANCZOS)


if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else "crates/code-canvas/assets/icon_source.png"
    icon().save(out)
    print("wrote", out)
