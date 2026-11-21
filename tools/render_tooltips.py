#!/usr/bin/env python3
"""Render PoE-style item tooltips from clipboard (Ctrl+C) item texts.

Stand-in test images until real screenshots exist: same layout rules, PoE text colours,
small caps like the game's Fontin SmallCaps (a free serif font imitates it), dividers and a
header banner. Writes two variants per item:

  screens/<name>.png  - clean, like a PC screenshot
  photos/<name>.jpg   - simulated phone photo of a TV: perspective, moire, blur, glare, noise, JPEG

Usage: render_tooltips.py <items dir> <out dir>
"""
import math
import random
import re
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont

FONT = "/usr/share/fonts/liberation/LiberationSerif-Regular.ttf"
FONT_ITALIC = "/usr/share/fonts/liberation/LiberationSerif-Italic.ttf"
SIZE = 20

RARITY_COLOR = {
    "Normal": (200, 200, 200), "Magic": (136, 136, 255), "Rare": (255, 255, 119),
    "Unique": (175, 96, 37), "Currency": (170, 158, 130), "Gem": (27, 162, 155),
    "Divination Card": (200, 200, 200),
}
GREY = (127, 127, 127)
WHITE = (255, 255, 255)
MOD = (136, 136, 255)
CRAFTED = (184, 218, 242)
FRACTURED = (162, 145, 98)
CORRUPTED = (210, 0, 0)
FLAVOUR = (175, 96, 37)

ROLL = re.compile(r"(\d)\(-?\d+(?:\.\d+)?-\d+(?:\.\d+)?\)")
KEYWORD = re.compile(r"\[(?:[^\]|]*\|)?([^\]]*)\]")
SUFFIX = re.compile(r" \((implicit|crafted|fractured|enchant|rune|desecrated)\)$")


def parse(text):
    """Clipboard text -> (rarity, header lines, [section of (segments)]) ready to draw."""
    secs = [[l.strip() for l in s.strip().splitlines() if l.strip()] for s in text.split("--------")]
    secs = [s for s in secs if s]
    head = [l for l in secs[0] if not l.startswith("Item Class:")]
    rarity = head[0].split(": ", 1)[1]
    names = head[1:]
    out = []
    last = len(secs) - 1
    for si, sec in enumerate(secs[1:], start=1):
        rows = []
        if sec[0] == "Requirements:":
            parts = []
            for l in sec[1:]:
                k, v = l.split(": ", 1)
                parts.append(f"Level {v}" if k == "Level" else f"{v} {k}")
            rows.append([("Requires ", GREY, False), (", ".join(parts), WHITE, False)])
            out.append(rows)
            continue
        flavour = rarity == "Unique" and si == last and not any(": " in l for l in sec)
        for l in sec:
            if l.startswith("{"):
                continue
            l = KEYWORD.sub(r"\1", ROLL.sub(r"\1", l))
            if flavour:
                rows.append([(l, FLAVOUR, True)])
            elif l == "Corrupted":
                rows.append([(l, CORRUPTED, False)])
            elif ": " in l:
                k, v = l.split(": ", 1)
                aug = "(augmented)" in v
                v = v.replace(" (augmented)", "").replace("(augmented)", "")
                rows.append([(k + ": ", GREY, False), (v, MOD if aug else WHITE, False)])
            elif rarity in ("Currency",) or l.startswith("Place into") or l.startswith("Right click"):
                rows.append([(l, GREY, False)])
            elif len(l.split()) <= 3 and not re.search(r"\d", l) and si == 1:
                rows.append([(l, GREY, False)])  # weapon class line, e.g. "Two Hand Axe"
            else:
                m = SUFFIX.search(l)
                kind = m.group(1) if m else "explicit"
                l = SUFFIX.sub("", l)
                col = {"crafted": CRAFTED, "enchant": CRAFTED, "fractured": FRACTURED}.get(kind, MOD)
                rows.append([(l, col, False)])
        out.append(rows)
    return rarity, names, out


class SmallCaps:
    """Draw text the way Fontin SmallCaps looks: lowercase as smaller capitals."""

    def __init__(self, size, italic=False):
        path = FONT_ITALIC if italic else FONT
        self.big = ImageFont.truetype(path, size)
        self.small = ImageFont.truetype(path, int(size * 0.78))

    def width(self, s):
        return sum(self._font(c).getlength(self._glyph(c)) for c in s)

    def _font(self, c):
        return self.small if c.islower() else self.big

    def _glyph(self, c):
        return c.upper() if c.islower() else c

    def draw(self, d, x, y_base, s, fill):
        for c in s:
            f = self._font(c)
            g = self._glyph(c)
            d.text((x, y_base), g, font=f, fill=fill, anchor="ls")
            x += f.getlength(g)
        return x


def render(text, rng):
    rarity, names, sections = parse(text)
    sc = {False: SmallCaps(SIZE), True: SmallCaps(SIZE, italic=True)}
    head = SmallCaps(int(SIZE * 1.2))
    line_h = int(SIZE * 1.25)
    gap = int(SIZE * 0.9)
    widths = [head.width(n) for n in names]
    for sec in sections:
        for row in sec:
            widths.append(sum(sc[it].width(t) for t, _, it in row))
    w = int(max(widths) + 60)
    head_h = int(len(names) * SIZE * 1.4 + 16)
    h = head_h + sum(len(s) * line_h for s in sections) + len(sections) * gap + 16
    img = Image.new("RGB", (w, h), (9, 8, 6))
    px = img.load()
    for y in range(h):  # faint texture
        for x in range(0, w, 2):
            n = rng.randint(0, 6)
            px[x, y] = (9 + n, 8 + n, 6 + n)
    d = ImageDraw.Draw(img)
    # Header banner
    for y in range(head_h):
        t = 1 - abs(y - head_h / 2) / (head_h / 2)
        c = tuple(int(v * (0.4 + 0.6 * t)) for v in (58, 42, 22))
        d.line([(0, y), (w, y)], fill=c)
    d.rectangle([0, 0, w - 1, h - 1], outline=(70, 55, 35))
    col = RARITY_COLOR.get(rarity, WHITE)
    y = 8
    for n in names:
        y += int(SIZE * 1.4)
        head.draw(d, (w - head.width(n)) / 2, y - 6, n, col)
    y = head_h
    for si, sec in enumerate(sections):
        if si > 0:
            # divider
            yy = y + gap // 2 - line_h // 4
            for x in range(20, w - 20):
                t = 1 - abs(x - w / 2) / (w / 2 - 20)
                d.point((x, yy), fill=tuple(int(v * t) for v in (120, 95, 60)))
            y += gap
        else:
            y += gap // 2
        for row in sec:
            y += line_h
            total = sum(sc[it].width(t) for t, _, it in row)
            x = (w - total) / 2
            for t, c, it in row:
                x = sc[it].draw(d, x, y - 6, t, c)
    return img


def photo(img, rng):
    """Simulate a phone photo of the tooltip on a TV."""
    w, h = img.size
    # Place on a darker game-scene-ish background
    bg = Image.new("RGB", (int(w * 1.5), int(h * 1.3)), (30, 26, 22))
    noise = Image.effect_noise(bg.size, 30).convert("RGB")
    bg = Image.blend(bg, noise, 0.15)
    ox, oy = (bg.width - w) // 2, (bg.height - h) // 2
    bg.paste(img, (ox, oy))
    # Upscale (TV pixels are big), then perspective skew
    s = 2
    bg = bg.resize((bg.width * s, bg.height * s), Image.NEAREST)
    W, H = bg.size
    k = 0.06
    jit = lambda: rng.uniform(-k, k)
    src = [(W * jit(), H * jit()), (W * (1 + jit()), H * jit()), (W * (1 + jit()), H * (1 + jit())), (W * jit(), H * (1 + jit()))]
    coeffs = _perspective_coeffs([(0, 0), (W, 0), (W, H), (0, H)], src)
    bg = bg.transform((W, H), Image.PERSPECTIVE, coeffs, Image.BICUBIC)
    # Moire: interference of the TV pixel grid and the camera sensor
    ang = rng.uniform(0.1, 0.5)
    period = rng.uniform(5, 9)
    m = Image.new("L", (W, H))
    mp = m.load()
    ca, sa = math.cos(ang), math.sin(ang)
    for yy in range(H):
        for xx in range(W):
            mp[xx, yy] = int(225 + 30 * math.sin((xx * ca + yy * sa) * 2 * math.pi / period))
    bg = Image.composite(bg, Image.new("RGB", (W, H)), m) if False else Image.merge(
        "RGB", [Image.composite(ch, Image.new("L", (W, H), 0), m) for ch in bg.split()])
    bg = Image.blend(bg, Image.eval(bg, lambda v: v), 1.0)
    # Glare: soft bright blob
    g = Image.new("L", (W, H), 0)
    gd = ImageDraw.Draw(g)
    cx, cy, r = rng.uniform(0.2, 0.8) * W, rng.uniform(0.2, 0.8) * H, 0.35 * min(W, H)
    gd.ellipse([cx - r, cy - r, cx + r, cy + r], fill=rng.randint(30, 55))
    g = g.filter(ImageFilter.GaussianBlur(r / 2))
    bg = Image.composite(Image.new("RGB", (W, H), (255, 255, 255)), bg, g)
    bg = bg.filter(ImageFilter.GaussianBlur(rng.uniform(0.8, 1.4)))
    noise = Image.effect_noise((W, H), 18).convert("RGB")
    bg = Image.blend(bg, noise, 0.06)
    return bg


def _perspective_coeffs(dst, src):
    # Solve for the 8 coefficients mapping dst -> src (PIL convention).
    import itertools
    A, B = [], []
    for (x, y), (u, v) in zip(dst, src):
        A.append([x, y, 1, 0, 0, 0, -u * x, -u * y]); B.append(u)
        A.append([0, 0, 0, x, y, 1, -v * x, -v * y]); B.append(v)
    # Gaussian elimination (8x8)
    n = 8
    M = [A[i] + [B[i]] for i in range(n)]
    for c in range(n):
        p = max(range(c, n), key=lambda r: abs(M[r][c]))
        M[c], M[p] = M[p], M[c]
        for r in range(n):
            if r != c:
                f = M[r][c] / M[c][c]
                M[r] = [a - f * b for a, b in zip(M[r], M[c])]
    return [M[i][n] / M[i][i] for i in range(n)]


def main():
    items, out = Path(sys.argv[1]), Path(sys.argv[2])
    (out / "screens").mkdir(parents=True, exist_ok=True)
    (out / "photos").mkdir(parents=True, exist_ok=True)
    for f in sorted(items.glob("*.txt")):
        rng = random.Random(f.stem)
        img = render(f.read_text(), rng)
        img.save(out / "screens" / f"{f.stem}.png")
        photo(img, rng).save(out / "photos" / f"{f.stem}.jpg", quality=72)
        print(f.stem, img.size)


if __name__ == "__main__":
    main()
