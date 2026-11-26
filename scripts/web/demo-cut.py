"""Cuts a demo video from recordings of the screen page (scripts/web/demo-record.mjs) by a plan.

    python3 -I scripts/web/demo-cut.py PLAN.toml OUT.mp4

A recording holds the original on its left half and the model in a theme on
its right, frame for frame. The plan (docs/demo-video.md) lists the video's
parts in order: a `card` (lines of text on a plain picture) or a `shot` (a
span of a recording, shown as the theme alone, the original alone, both side
by side, a fade from the one to the other and back, or a wipe). A shot can
carry bubbles: a few words that pop up over the picture for some seconds,
with a tail to the thing they are about and a short blip as they come.

Text is drawn from a folder of letter pictures (`letters`: letter-A.png,
letter-0.png, letter-x021.png for "!" by its code point), so the video's
words are in the theme's own lettering. ffmpeg does the rest: no text
filter is needed.
"""

import math
import os
import struct
import subprocess
import sys
import tomllib
import wave

from PIL import Image, ImageDraw, ImageFont

RATE = 48000


def die(msg):
    print(f"demo-cut: {msg}", file=sys.stderr)
    sys.exit(2)


class Letters:
    """The letter pictures of a folder, cropped to their ink, at a size."""

    def __init__(self, folder, height):
        self.h = height
        self.glyphs = {}
        for name in sorted(os.listdir(folder)):
            if not (name.startswith("letter-") and name.endswith(".png")):
                continue
            key = name[len("letter-") : -len(".png")]
            ch = chr(int(key[1:], 16)) if key.startswith("x") and len(key) > 1 else key
            im = Image.open(os.path.join(folder, name)).convert("RGBA")
            box = im.getchannel("A").getbbox()
            if not box:
                continue
            # The whole height is kept, so letters share a base line.
            im = im.crop((box[0], 0, box[2], im.height))
            k = height / im.height
            im = im.resize((max(1, round(im.width * k)), height), Image.NEAREST)
            self.glyphs[ch] = im
        if not self.glyphs:
            die(f"no letter-*.png in {folder}")
        self.gap = max(1, height // 16)
        self.space = height // 2

    def width(self, text):
        w = 0
        for ch in text:
            g = self.glyphs.get(ch.upper())
            w += (g.width if g else self.space) + self.gap
        return max(0, w - self.gap)

    def draw(self, im, x, y, text):
        for ch in text:
            g = self.glyphs.get(ch.upper())
            if g:
                im.alpha_composite(g, (x, y))
                x += g.width
            else:
                x += self.space
            x += self.gap

    def wrap(self, text, most):
        lines = []
        for para in text.split("\n"):
            line = ""
            for word in para.split():
                t = f"{line} {word}".strip()
                if line and self.width(t) > most:
                    lines.append(line)
                    line = word
                else:
                    line = t
            lines.append(line)
        return lines


class Plain:
    """A font file's letters at a size, for a line the letter pictures can't
    spell: an address, whose small letters and slash matter."""

    def __init__(self, path, height, ink):
        try:
            self.font = ImageFont.truetype(path, height)
        except OSError as e:
            die(f"plain_font {path}: {e}")
        self.h = height
        self.ink = ink

    def width(self, text):
        return round(self.font.getlength(text))

    def draw(self, im, x, y, text):
        ImageDraw.Draw(im).text((x, y), text, font=self.font, fill=self.ink)

    def wrap(self, text, most):
        return [text]


def colour(v, default):
    v = v or default
    v = v.lstrip("#")
    return tuple(int(v[i : i + 2], 16) for i in (0, 2, 4)) + (255,)


def bubble_picture(letters, text, most, fill, edge, tail):
    """A bubble's picture and where its tail's tip is in it."""
    lines = letters.wrap(text, most)
    pad = letters.h // 2
    lead = letters.h + letters.h // 4
    w = max(letters.width(l) for l in lines) + 2 * pad
    h = lead * len(lines) - letters.h // 4 + 2 * pad
    t = letters.h  # the tail's length
    b = max(2, letters.h // 8)  # the edge's width
    im = Image.new("RGBA", (w + 2 * b, h + 2 * b + t), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    d.fontmode = "1"
    up = tail.startswith("up")
    top = t if up else 0
    d.rounded_rectangle((0, top, w + 2 * b - 1, top + h + 2 * b - 1), radius=pad, fill=edge)
    d.rounded_rectangle((b, top + b, w + b - 1, top + h + b - 1), radius=max(1, pad - b), fill=fill)
    # The tail: a triangle from the bubble's edge to its tip, left or right
    # of the middle.
    cx = (w + 2 * b) // 4 if tail.endswith("left") else 3 * (w + 2 * b) // 4
    if up:
        tip = (cx, 0)
        d.polygon([(cx - t // 2 - b, top + b), (cx + t // 2 + b, top + b), tip], fill=edge)
        d.polygon([(cx - t // 2 + b, top + 2 * b), (cx + t // 2 - b, top + 2 * b), (cx, 3 * b)], fill=fill)
    else:
        base = top + h + b
        tip = (cx, im.height - 1)
        d.polygon([(cx - t // 2 - b, base), (cx + t // 2 + b, base), tip], fill=edge)
        d.polygon([(cx - t // 2 + b, base - b), (cx + t // 2 - b, base - b), (cx, im.height - 1 - 3 * b)], fill=fill)
    y = top + b + pad
    for l in lines:
        letters.draw(im, b + pad + (w - 2 * pad - letters.width(l)) // 2, y, l)
        y += lead
    return im, tip


def card_picture(size, letters_big, letters_small, plain, lines, ground):
    """A card: its first line large, the rest small. A line written as
    { plain = "..." } is drawn as typed, in the plain font."""
    im = Image.new("RGBA", size, ground)
    rows = []
    for n, line in enumerate(lines):
        if isinstance(line, dict):
            rows.append((plain, line["plain"]))
            continue
        L = letters_big if n == 0 else letters_small
        for part in L.wrap(line, size[0] - 240):
            rows.append((L, part))
    total = sum(L.h + L.h // 2 for L, _ in rows)
    y = (size[1] - total) // 2
    for L, part in rows:
        L.draw(im, (size[0] - L.width(part)) // 2, y, part)
        y += L.h + L.h // 2
    return im


def blips(path, times, length):
    """A sound track of `length` seconds with a short rising blip at each time."""
    n = int(length * RATE)
    data = [0.0] * n
    for t in times:
        k0 = int(t * RATE)
        dur = int(0.11 * RATE)
        ph = 0.0
        for k in range(dur):
            if k0 + k >= n:
                break
            u = k / dur
            ph += 2 * math.pi * (520 + 620 * u) / RATE
            env = min(1.0, k / 200) * (1 - u) ** 1.5
            data[k0 + k] += 0.32 * env * (math.sin(ph) + 0.3 * math.sin(2 * ph))
    with wave.open(path, "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(b"".join(struct.pack("<hh", *(2 * [int(max(-1, min(1, v)) * 32767)])) for v in data))


def run(cmd):
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        die(f"ffmpeg failed:\n{' '.join(cmd)}\n{r.stderr[-1500:]}")


SHOWS = {
    "theme": "[0:v]fps={fps},crop=iw/2:ih:iw/2:0[g]",
    "original": "[0:v]fps={fps},crop=iw/2:ih:0:0[g]",
    "both": "[0:v]fps={fps},scale={bw}:{bh}:flags=lanczos[g]",
    # A is the theme, B the original: the theme's share swings from all to
    # none and back every `swing` seconds, starting on the theme.
    "fade": "[0:v]fps={fps},split[l][r];[r]crop=iw/2:ih:iw/2:0,format=gbrp[a];[l]crop=iw/2:ih:0:0,format=gbrp[b];"
    "[a][b]blend=all_expr='A*(0.5+0.5*cos(2*PI*T/{swing}))+B*(0.5-0.5*cos(2*PI*T/{swing}))'[g]",
    "wipe": "[0:v]fps={fps},split[l][r];[r]crop=iw/2:ih:iw/2:0,format=gbrp[a];[l]crop=iw/2:ih:0:0,format=gbrp[b];"
    "[a][b]blend=all_expr='if(lt(X,W*(0.5+0.5*cos(2*PI*T/{swing}))),B,A)'[g]",
}


def main():
    if len(sys.argv) != 3:
        die("usage: python3 -I scripts/web/demo-cut.py PLAN.toml OUT.mp4")
    plan_path, out = sys.argv[1], sys.argv[2]
    plan = tomllib.load(open(plan_path, "rb"))
    base = os.path.dirname(os.path.abspath(plan_path))
    at = lambda p: p if os.path.isabs(p) else os.path.join(base, p)
    W, H = plan.get("size", [1920, 1080])
    fps = plan.get("fps", 30)
    ground = colour(plan.get("ground"), "0b1020")
    fill = colour(plan.get("bubble"), "16213a")
    edge = colour(plan.get("bubble_edge"), "f3e3b0")
    folder = at(plan["letters"])
    small = Letters(folder, plan.get("letter_height", 32))
    big = Letters(folder, plan.get("title_height", 64))
    plain = Plain(plan.get("plain_font", "/System/Library/Fonts/Menlo.ttc"), plan.get("plain_height", 44), colour(plan.get("plain_ink"), "f3e3b0"))
    # Letters drawn on a tile of their own colour: with no ground named, the
    # frame takes that colour, so no tile shows round a letter.
    corner = next(iter(small.glyphs.values())).getpixel((0, 0))
    if "ground" not in plan and corner[3] == 255:
        ground = corner
    work = at(plan.get("work", "demo-work"))
    os.makedirs(work, exist_ok=True)
    ground_hex = "0x%02x%02x%02x" % ground[:3]
    parts = []
    for n, part in enumerate(plan.get("part", [])):
        dst = os.path.join(work, f"part-{n:02}.mp4")
        length = float(part["length"])
        enc = ["-r", str(fps), "-c:v", "libx264", "-preset", "medium", "-crf", "16", "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "192k", "-ar", str(RATE), "-ac", "2", "-t", f"{length}"]
        if part["kind"] == "card":
            png = os.path.join(work, f"card-{n:02}.png")
            card_picture((W, H), big, small, plain, part["lines"], ground).save(png)
            run(["ffmpeg", "-v", "error", "-y", "-loop", "1", "-t", f"{length}", "-i", png, "-f", "lavfi", "-t", f"{length}", "-i", f"anullsrc=r={RATE}:cl=stereo", "-vf", f"fps={fps},format=yuv420p"] + enc + [dst])
        elif part["kind"] == "shot":
            show = part.get("show", "theme")
            if show not in SHOWS:
                die(f"part {n}: show is one of {', '.join(SHOWS)}")
            inputs = ["-ss", f"{float(part.get('start', 0))}", "-t", f"{length}", "-i", at(part["file"])]
            # Where the game's picture is in the frame, for a bubble's place:
            # x and y are fractions of it. Both screens side by side are
            # fitted inside the frame; one screen alone is shown as it is.
            probe = subprocess.run(["ffprobe", "-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height", "-of", "csv=p=0", at(part["file"])], capture_output=True, text=True).stdout.strip().split(",")
            rw, rh = int(probe[0]), int(probe[1])
            if show == "both":
                k = min((W - 64) / rw, (H - 64) / rh)
                gw, gh = int(rw * k) // 2 * 2, int(rh * k) // 2 * 2
            else:
                gw, gh = rw // 2, rh
            gx, gy = (W - gw) // 2, (H - gh) // 2
            graph = SHOWS[show].format(fps=fps, bw=gw, bh=gh, swing=part.get("swing", 4))
            if gw > W or gh > H:
                # A screen larger than the frame (a theme drawn at six
                # times a 256 by 224 game) is brought down to fit it.
                k = min(W / gw, H / gh)
                gw, gh = int(gw * k) // 2 * 2, int(gh * k) // 2 * 2
                gx, gy = (W - gw) // 2, (H - gh) // 2
                graph += f";[g]scale={gw}:{gh}:flags=lanczos[gs];[gs]pad="
            else:
                graph += ";[g]pad="
            graph += f"{W}:{H}:(ow-iw)/2:(oh-ih)/2:color={ground_hex}[v0]"
            last = "v0"
            times = []
            # The bubbles of a shot are one stream of whole-frame pictures,
            # clear but for the bubbles showing, laid over the video once:
            # ffmpeg fails when two pictures are laid over a recording one
            # after the other ("Error while filtering: Invalid argument").
            shown = []
            for k, bub in enumerate(part.get("bubble", [])):
                tail = bub.get("tail", "down-left")
                im, tip = bubble_picture(small, bub["text"], bub.get("width", 620), fill, edge, tail)
                px = gx + round(float(bub["x"]) * gw) - tip[0]
                py = gy + round(float(bub["y"]) * gh) - tip[1]
                px = max(8, min(W - im.width - 8, px))
                py = max(8, min(H - im.height - 8, py))
                t0, t1 = float(bub["at"]), float(bub["at"]) + float(bub.get("for", 4))
                times.append(t0)
                # It pops: two frames at four fifths of its size, then whole.
                pop = im.resize((im.width * 4 // 5, im.height * 4 // 5), Image.NEAREST)
                grow = 2 / fps
                shown.append((t0, t0 + grow, pop, (px + im.width // 10, py + im.height // 10)))
                shown.append((t0 + grow, t1, im, (px, py)))
            if shown:
                cuts = sorted({0.0, length} | {min(length, max(0.0, t)) for a, b, _, _ in shown for t in (a, b)})
                listing = os.path.join(work, f"over-{n:02}.txt")
                lines = ["ffconcat version 1.0"]
                for j, (a, b) in enumerate(zip(cuts, cuts[1:])):
                    over = Image.new("RGBA", (W, H), (0, 0, 0, 0))
                    for s0, s1, pic, at_xy in shown:
                        if s0 <= a < s1:
                            over.alpha_composite(pic, at_xy)
                    f_over = os.path.join(work, f"over-{n:02}-{j:02}.png")
                    over.save(f_over)
                    lines += [f"file '{f_over}'", f"duration {b - a:.6f}"]
                # The concat reader wants the last picture named again.
                lines.append(lines[-2])
                open(listing, "w").write("\n".join(lines) + "\n")
                inputs += ["-f", "concat", "-safe", "0", "-i", listing]
                a = len([x for x in inputs if x == "-i"]) - 1
                graph += f";[{a}:v]fps={fps},format=rgba[ov];[{last}][ov]overlay=0:0:eof_action=pass[q]"
                last = "q"
            wav = os.path.join(work, f"blips-{n:02}.wav")
            blips(wav, times, length)
            inputs += ["-i", wav]
            b = len([x for x in inputs if x == "-i"]) - 1
            graph += f";[0:a]aresample={RATE}:async=1,volume={part.get('volume', 1.0)}[ga];[ga][{b}:a]amix=inputs=2:duration=first:normalize=0[a]"
            graph += f";[{last}]scale={W}:{H}:flags=neighbor,setsar=1,format=yuv420p[vout]"
            run(["ffmpeg", "-v", "error", "-y"] + inputs + ["-filter_complex", graph, "-map", "[vout]", "-map", "[a]"] + enc + [dst])
        else:
            die(f"part {n}: kind is card or shot")
        parts.append(dst)
        print(f"part {n}: {part['kind']} {part.get('show', '')} {length} s")
    if not parts:
        die("the plan has no [[part]]")
    listing = os.path.join(work, "parts.txt")
    open(listing, "w").write("".join(f"file '{p}'\n" for p in parts))
    run(["ffmpeg", "-v", "error", "-y", "-f", "concat", "-safe", "0", "-i", listing, "-c", "copy", "-movflags", "+faststart", out])
    print(f"wrote {out}")


main()
