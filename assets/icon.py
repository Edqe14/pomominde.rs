import sys
from PIL import Image, ImageDraw, ImageFilter
S = 4  # supersample
N = 1024 * S
out = sys.argv[1]

def lerp(a, b, t): return tuple(round(x + (y - x) * t) for x, y in zip(a, b))

# macOS icon grid: 824px body inside 1024 canvas, ~185px corner radius
body = (100 * S, 100 * S, 924 * S, 924 * S)
r = 185 * S

mask = Image.new("L", (N, N), 0)
ImageDraw.Draw(mask).rounded_rectangle(body, r, fill=255)

# soft drop shadow
shadow = Image.new("RGBA", (N, N), (0, 0, 0, 0))
sm = mask.point(lambda v: v * 0.35).filter(ImageFilter.GaussianBlur(14 * S))
shadow.putalpha(sm.transform(sm.size, Image.AFFINE, (1, 0, 0, 0, 1, -10 * S)))

# body gradient (work red)
grad = Image.new("RGBA", (N, N))
gd = ImageDraw.Draw(grad)
top, bot = (0xA8, 0x44, 0x42), (0x7C, 0x2C, 0x2C)
for y in range(N):
    gd.line([(0, y), (N, y)], fill=lerp(top, bot, y / N) + (255,))
icon = Image.alpha_composite(Image.new("RGBA", (N, N), (0, 0, 0, 0)), shadow)
icon.paste(grad, (0, 0), mask)

# progress ring: faint track + 3/4 arc with round caps
c, rad, w = N // 2, 250 * S, 60 * S
box = (c - rad, c - rad, c + rad, c + rad)
ring = Image.new("RGBA", (N, N), (0, 0, 0, 0))
rd = ImageDraw.Draw(ring)
rd.ellipse(box, outline=(255, 255, 255, 38), width=w)
icon = Image.alpha_composite(icon, ring)
ring = Image.new("RGBA", (N, N), (0, 0, 0, 0))
rd = ImageDraw.Draw(ring)
cream = (0xEE, 0xEA, 0xE4, 255)
rd.arc(box, -90, 180, fill=cream, width=w)
import math
for a in (-90, 180):
    t = math.radians(a)
    x, y = c + (rad - w / 2) * math.cos(t), c + (rad - w / 2) * math.sin(t)
    rd.ellipse((x - w / 2, y - w / 2, x + w / 2, y + w / 2), fill=cream)
icon = Image.alpha_composite(icon, ring)

icon.resize((1024, 1024), Image.LANCZOS).save(out)
