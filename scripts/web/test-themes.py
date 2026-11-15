#!/usr/bin/env python3
# Two made-up large-art themes for Donkey Kong, for the page check's
# sections that need more than one theme besides the page's own
# (check-page.mjs: the overlapping theme picks, the theme line, a theme
# from a namespace that sorts before the game's). The release check
# publishes them into its scratch copy of the registry
# (scripts/web/test-registry.sh), never into the real one, so what the
# check covers doesn't hang on what people publish there.
#   python3 -I scripts/web/test-themes.py OUT
#
# OUT gets one theme folder a package, as lex00/turnips is laid out:
#   OUT/harbor-lights  acme/harbor-lights, "Harbor Lights": navy and amber
#                      rounded blobs (acme/ sorts before lex00/)
#   OUT/paper-moon     test/paper-moon, "Paper Moon": grey and white boxes
#                      with a dark rim (test/ sorts after lex00/)
# Each holds package.toml, theme/look.txt, theme/art.txt (scale 6) and a
# picture in theme/art/ for each of the game's moving pictures and its
# digits, drawn here from plain shapes and colours: nothing in them is
# read from a ROM or from the game's art. The stages, the belt ends, the
# score display and the title screen are left as the game draws them, so
# `gp theme look check` names those seven slots as still to draw and finds
# nothing else wrong but three bodies' feet rows (warnings). The picture
# names are the game's build's (`gp theme look check THEME --game
# donkey-kong` lists them by pose); a name the game doesn't have is passed
# over by the page.
# The game draws a picture in up to four colourings (frame-N-0.png to
# frame-N-3.png, glyph-0-3-D.png to glyph-3-3-D.png); each gets the same
# picture here. Poses the kit says to draw as nothing get a clear one.
import os
import struct
import sys
import zlib

SCALE = 6

# (picture names, width, height) in game pixels, from Donkey Kong's theme
# kit as `gp theme look check` names each pose's picture, colouring 0.
PICTURES = [
    ([f"frame-{n}-0.png" for n in range(0, 9)], 16, 16),  # rolling hazard
    (["frame-20-0.png"], 16, 8),  # lift platform
    ([f"frame-{n}-0.png" for n in range(22, 27)], 16, 16),  # wandering flame
    (["frame-38-0.png", "frame-39-0.png"], 8, 16),  # smashing tool
    ([f"frame-{n}-0.png" for n in range(41, 53)], 48, 32),  # big foe
    (["frame-69-0.png"], 16, 16),  # sliding ladder
    (["frame-71-0.png", "frame-72-0.png"], 16, 8),  # fire source flames
    (["frame-73-0.png", "frame-74-0.png"], 16, 24),  # the one to rescue
    ([f"frame-{n}-0.png" for n in range(77, 82)], 16, 16),  # sliding hazard
    ([f"frame-{n}-0.png" for n in range(82, 103)], 16, 16),  # hero
    (["frame-104-0.png"], 16, 16),  # bouncing hazard
    (["frame-106-0.png", "frame-107-0.png"], 16, 8),  # points
    (["screen-11-sprite-0.png", "screen-16-sprite-0.png"], 8, 8),  # cursor
    ([f"glyph-0-3-{n}.png" for n in range(30, 40)], 8, 8),  # digits
]

# Poses the kit says to draw as nothing.
EMPTY = {"frame-81-0.png", "frame-107-0.png"}


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
    if len(sys.argv) != 2:
        print("usage: python3 -I scripts/web/test-themes.py OUT", file=sys.stderr)
        sys.exit(2)
    out = sys.argv[1]
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
        for names, w, h in PICTURES:
            for name in names:
                pixel = (lambda x, y: (0, 0, 0, 0)) if name in EMPTY else draw(t, w * SCALE, h * SCALE, k)
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
