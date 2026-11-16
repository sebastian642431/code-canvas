# Draws the Code Canvas icon: nested folder boxes with code bars and a relation curve.
from PIL import Image, ImageDraw
S = 1024  # supersampled, downscaled for anti-aliasing
img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
d = ImageDraw.Draw(img)
def R(x0, y0, x1, y1, r, fill=None, outline=None, w=1):
    d.rounded_rectangle([x0, y0, x1, y1], radius=r, fill=fill, outline=outline, width=w)
R(40, 40, S - 40, S - 40, 190, fill=(12, 15, 22, 255), outline=(42, 52, 74, 255), w=14)
# outer folder
R(150, 190, S - 150, S - 170, 60, outline=(61, 220, 132, 255), w=22)
# two file cards
R(215, 290, 470, 770, 28, fill=(20, 27, 42, 255), outline=(122, 162, 247, 255), w=16)
R(560, 380, 810, 770, 28, fill=(20, 27, 42, 255), outline=(122, 162, 247, 255), w=16)
bars = [((255, 79, 184), 150), ((192, 202, 245), 190), ((158, 206, 106), 110), ((42, 195, 222), 170), ((192, 202, 245), 130), ((187, 154, 247), 160)]
for i, (c, w) in enumerate(bars):
    y = 340 + i * 66
    d.rounded_rectangle([255, y, 255 + w, y + 26], radius=13, fill=c + (255,))
for i, (c, w) in enumerate(bars[1:5]):
    y = 430 + i * 66
    d.rounded_rectangle([600, y, 600 + int(w * 0.9), y + 26], radius=13, fill=c + (255,))
# relation curve between the cards
def bez(p0, p1, p2, p3, n=120):
    pts = []
    for i in range(n + 1):
        t = i / n; u = 1 - t
        pts.append((u**3*p0[0] + 3*u*u*t*p1[0] + 3*u*t*t*p2[0] + t**3*p3[0], u**3*p0[1] + 3*u*u*t*p1[1] + 3*u*t*t*p2[1] + t**3*p3[1]))
    return pts
pts = bez((420, 353), (540, 353), (480, 509), (600, 509))
d.line(pts, fill=(255, 180, 84, 255), width=20, joint="curve")
for p in (pts[0], pts[-1]):
    d.ellipse([p[0] - 20, p[1] - 20, p[0] + 20, p[1] + 20], fill=(255, 180, 84, 255))
out = "crates/code-canvas/assets/"
big = img.resize((256, 256), Image.LANCZOS)
big.save(out + "icon.png")
big.save(out + "icon.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
open(out + "icon_64.rgba", "wb").write(img.resize((64, 64), Image.LANCZOS).tobytes())
print("icon written")
