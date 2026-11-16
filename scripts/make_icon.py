# Builds every icon asset of the app from one square image:
#
#   python scripts/make_icon.py                      # from assets/icon_source.png
#   python scripts/make_icon.py path/to/icon.png     # from another image
#   python scripts/make_icon.py --placeholder        # draw a simple placeholder
#
# Outputs, in crates/code-canvas/assets/:
#   icon.png       256 px   (README, packaging)
#   icon.ico       16-256   (the .exe, shortcuts, taskbar)
#   icon_128.rgba  128 px   (the logo in the app's header)
#   icon_64.rgba   64 px    (the window icon)
#
# The artwork should be a square PNG, ideally 1024 px, with a transparent
# background around the icon shape. Requires Pillow.
import sys

from PIL import Image, ImageDraw

OUT = "crates/code-canvas/assets/"


def placeholder():
    """Nested folder boxes with code bars and a relation curve."""
    s = 1024  # supersampled, downscaled for anti-aliasing
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)

    def box(x0, y0, x1, y1, r, fill=None, outline=None, w=1):
        d.rounded_rectangle([x0, y0, x1, y1], radius=r, fill=fill, outline=outline, width=w)

    box(40, 40, s - 40, s - 40, 190, fill=(12, 15, 22, 255), outline=(42, 52, 74, 255), w=14)
    box(150, 190, s - 150, s - 170, 60, outline=(61, 220, 132, 255), w=22)
    box(215, 290, 470, 770, 28, fill=(20, 27, 42, 255), outline=(122, 162, 247, 255), w=16)
    box(560, 380, 810, 770, 28, fill=(20, 27, 42, 255), outline=(122, 162, 247, 255), w=16)
    bars = [
        ((255, 79, 184), 150),
        ((192, 202, 245), 190),
        ((158, 206, 106), 110),
        ((42, 195, 222), 170),
        ((192, 202, 245), 130),
        ((187, 154, 247), 160),
    ]
    for i, (c, w) in enumerate(bars):
        y = 340 + i * 66
        d.rounded_rectangle([255, y, 255 + w, y + 26], radius=13, fill=c + (255,))
    for i, (c, w) in enumerate(bars[1:5]):
        y = 430 + i * 66
        d.rounded_rectangle([600, y, 600 + int(w * 0.9), y + 26], radius=13, fill=c + (255,))

    def bezier(p0, p1, p2, p3, n=120):
        pts = []
        for i in range(n + 1):
            t = i / n
            u = 1 - t
            pts.append(
                (
                    u**3 * p0[0] + 3 * u * u * t * p1[0] + 3 * u * t * t * p2[0] + t**3 * p3[0],
                    u**3 * p0[1] + 3 * u * u * t * p1[1] + 3 * u * t * t * p2[1] + t**3 * p3[1],
                )
            )
        return pts

    pts = bezier((420, 353), (540, 353), (480, 509), (600, 509))
    d.line(pts, fill=(255, 180, 84, 255), width=20, joint="curve")
    for p in (pts[0], pts[-1]):
        d.ellipse([p[0] - 20, p[1] - 20, p[0] + 20, p[1] + 20], fill=(255, 180, 84, 255))
    return img


def load(path):
    """Any image -> square RGBA. Transparent margins around the artwork are
    trimmed (keeping 2%), so the icon is not smaller than its neighbours in
    the taskbar; a non-square result is padded with transparency."""
    src = Image.open(path).convert("RGBA")
    box = src.getchannel("A").point(lambda a: 255 if a > 8 else 0).getbbox()
    if box:
        src = src.crop(box)
    side = int(max(src.size) * 1.04)
    img = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    img.paste(src, ((side - src.width) // 2, (side - src.height) // 2))
    return img


arg = sys.argv[1] if len(sys.argv) > 1 else OUT + "icon_source.png"
img = placeholder() if arg == "--placeholder" else load(arg)
big = img.resize((256, 256), Image.LANCZOS)
big.save(OUT + "icon.png")
big.save(OUT + "icon.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
for size in (128, 64):
    with open(OUT + "icon_%d.rgba" % size, "wb") as f:
        f.write(img.resize((size, size), Image.LANCZOS).tobytes())
print("icon assets written to", OUT, "from", arg)
