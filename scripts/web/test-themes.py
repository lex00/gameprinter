#!/usr/bin/env python3
# Two made-up large-art themes for a game, for the page check's sections
# that need more than one theme besides the page's own (check-page.mjs: the
# overlapping theme picks, the theme line, a theme from a namespace that
# sorts before the game's). The release check publishes them into its
# scratch copy of the registry (scripts/web/test-registry.sh), never into
# the real one, so what the check covers doesn't hang on what people
# publish there.
#   python3 -I scripts/web/test-themes.py OUT GAME_DIR
#
# GAME_DIR is the game's folder in the games checkout; its page-check.toml
# [test_themes] names the pictures to draw: each entry's pictures of `size`
# game pixels, `name` with {n} put for each number from n[0] to n[1] or the
# names in `names` (the game's picture names, colouring 0, as `gp theme
# look check THEME --game GAME` names each pose's picture), and `empty`,
# the poses the kit says to draw as nothing; a name the game doesn't have
# is passed over by the page. OUT gets one theme folder a package, as a
# large-art theme package is laid out:
#   OUT/harbor-lights  acme/harbor-lights, "Harbor Lights": navy and amber
#                      rounded blobs (acme/ sorts before most namespaces)
#   OUT/paper-moon     test/paper-moon, "Paper Moon": grey and white boxes
#                      with a dark rim (test/ sorts after most)
# Each holds package.toml, theme/look.txt, theme/art.txt (scale 6) and a
# picture in theme/art/ for each picture named, drawn here from plain
# shapes and colours: nothing in them is read from a ROM or from the
# game's art.
# The game draws a picture in up to four colourings (frame-N-0.png to
# frame-N-3.png, glyph-0-3-D.png to glyph-3-3-D.png); each gets the same
# picture here.
import os
import struct
import sys
import tomllib
import zlib

SCALE = 6


def pictures_of(game_dir):
    """The game's [test_themes]: ([(names, width, height)], empty)."""
    path = os.path.join(game_dir, "page-check.toml")
    try:
        with open(path, "rb") as f:
            t = tomllib.load(f).get("test_themes")
    except OSError:
        t = None
    if not t or not t.get("pictures"):
        print(f"test-themes.py: {path} has no [test_themes] pictures, so there is nothing to draw the themes for", file=sys.stderr)
        sys.exit(2)
    def names(p):
        if "name" in p:
            first, last = p["n"]
            return [p["name"].replace("{n}", str(n)) for n in range(first, last + 1)]
        return list(p["names"])

    return [(names(p), p["size"][0], p["size"][1]) for p in t["pictures"]], set(t.get("empty", []))


def colourings(name):
    """The names the game draws one picture under, one a colouring."""
    if name.startswith("frame-"):
        return [name[: -len("0.png")] + f"{c}.png" for c in range(4)]
    if name.startswith("glyph-0-"):
        return [f"glyph-{c}-" + name[len("glyph-0-") :] for c in range(4)]
    return [name]


THEMES = [
    {
        "dir": "harbor-lights",
        "name": "acme/harbor-lights",
        "title": "Harbor Lights",
        "about": "navy and amber rounded blobs",
        "fill": (24, 48, 120, 255),
        "mark": (250, 190, 40, 255),
        "rim": None,
        "round": True,
    },
    {
        "dir": "paper-moon",
        "name": "test/paper-moon",
        "title": "Paper Moon",
        "about": "grey and white boxes with a dark rim",
        "fill": (205, 205, 210, 255),
        "mark": (255, 255, 255, 255),
        "rim": (40, 40, 48, 255),
        "round": False,
    },
]


def png(path, w, h, pixel):
    """A w by h RGBA PNG, pixel(x, y) giving each pixel's colour."""
    rows = []
    for y in range(h):
        row = bytearray(b"\x00")
        for x in range(w):
            row += bytes(pixel(x, y))
        rows.append(bytes(row))

    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)

    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)))
        f.write(chunk(b"IDAT", zlib.compress(b"".join(rows), 9)))
        f.write(chunk(b"IEND", b""))


def draw(theme, w, h, k):
    """The picture for pose k: the theme's shape filling the box, and a
    band whose place moves with k, so each pose looks different."""
    clear = (0, 0, 0, 0)
    band = (k * 5) % max(1, h - 2 * SCALE)

    def pixel(x, y):
        if theme["round"]:
            # An ellipse filling the box.
            dx = (x + 0.5 - w / 2) / (w / 2)
            dy = (y + 0.5 - h / 2) / (h / 2)
            if dx * dx + dy * dy > 1:
                return clear
        elif theme["rim"] and (x < SCALE or y < SCALE or x >= w - SCALE or y >= h - SCALE):
            return theme["rim"]
        if band <= y - SCALE < band + SCALE:
            return theme["mark"]
        return theme["fill"]

    return pixel


def main():
    if len(sys.argv) != 3:
        print("usage: python3 -I scripts/web/test-themes.py OUT GAME_DIR", file=sys.stderr)
        sys.exit(2)
    out = sys.argv[1]
    pictures, empty = pictures_of(sys.argv[2])
    for t in THEMES:
        d = os.path.join(out, t["dir"])
        art = os.path.join(d, "theme", "art")
        os.makedirs(art, exist_ok=True)
        lines = [
            "gp-art 1",
            f"# {t['title']}: made up for the page check, {t['about']} drawn by scripts/web/test-themes.py.",
            f"scale {SCALE}",
            "limit off",
        ]
        k = 0
        for names, w, h in pictures:
            for name in names:
                pixel = (lambda x, y: (0, 0, 0, 0)) if name in empty else draw(t, w * SCALE, h * SCALE, k)
                png(os.path.join(art, name), w * SCALE, h * SCALE, pixel)
                lines += [f"pic {c} art/{name} {w} {h} 0 0" for c in colourings(name)]
                k += 1
        with open(os.path.join(d, "theme", "art.txt"), "w") as f:
            f.write("\n".join(lines) + "\n")
        with open(os.path.join(d, "theme", "look.txt"), "w") as f:
            f.write(f"gp-look 1\nid {t['dir']}\ntitle {t['title']}\nart art.txt\n")
        with open(os.path.join(d, "package.toml"), "w") as f:
            f.write(
                f"# {t['name']}: made up for the page check (scripts/web/test-themes.py); never published to a real registry.\n"
                f'[package]\nname = "{t["name"]}"\nversion = "0.1.0"\nkind = "content"\ntitle = "{t["title"]}"\nlicense = "CC0-1.0"\n'
            )
        print(d)


if __name__ == "__main__":
    main()
