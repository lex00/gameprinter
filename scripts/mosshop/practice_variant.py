#!/usr/bin/env python3
"""A practice variant of Moss Hop with its RAM moved, and the files to grade
stage 0 on it.

    scripts/mosshop/practice_variant.py NAME [zp=+N] [LABEL=ADDR ...] [--out DIR]
        [--patch scripts/mosshop/variants/X.patch] [--part-of LABEL=ROUTINE ...]
        [--element LABEL=TAG ...]

zp=+N moves every zero-page name in ram.inc by N; LABEL=ADDR puts one name
at a hex address (OAM=0600 puts the sprite table on page 6). --patch applies
a change to mosshop.s (an idiom other games use, in
scripts/mosshop/variants/); --part-of and --element go to ground_truth.py
for the labels it adds. --cfg puts it on another board (an ld65 layout,
such as scripts/mosshop/variants/uxrom.cfg) and --prg-base says where its
fixed bank starts in the PRG. The variant is
built into DIR (default $GP_HOME/work/variants/NAME) with:

  mosshop.nes, mosshop.dbg   the build (ca65 and ld65, as scripts/mosshop/build.sh)
  mosshop.toml               assets/mosshop.toml, each PRG offset moved as far as
                             its label moved
  ground-truth.toml          from its own build (ground_truth.py --src)
  mapping.toml               templates/mosshop/mapping.toml, RAM moved by ram.inc's names
  key.toml                   games/mosshop/static-key.toml, its paths and the
                             harvest's address-named names (slots_20, state_05,
                             c_0A) moved by zp

Grade it with `gp model static DIR/mosshop.nes [--assets DIR/mosshop.toml]
--grade DIR/key.toml`. A practice variant is the stage 0 lane's own
test of an idiom (ROADMAP.md, phase 1); the held-out variants are graded by
someone else.
"""

import argparse
import os
import re
import subprocess
import sys

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SRC = os.path.join(REPO, "games", "mosshop")


def ram(path):
    """ram.inc's names and addresses."""
    out = {}
    for line in open(path):
        m = re.match(r"^(\w+)\s*=\s*\$([0-9A-Fa-f]+)", line)
        if m:
            out[m.group(1)] = int(m.group(2), 16)
    return out


def write_ram(out, moves, zp):
    lines = []
    for line in open(os.path.join(SRC, "ram.inc")):
        m = re.match(r"^(\w+)(\s*=\s*)\$([0-9A-Fa-f]+)(.*)$", line.rstrip("\n"))
        if m:
            name, width = m.group(1), len(m.group(3))
            a = int(m.group(3), 16)
            if name in moves:
                a = moves[name]
            elif a < 0x100:
                a += zp
                if a > 0xFF:
                    sys.exit("practice_variant.py: %s moves out of the zero page" % name)
            line = "%s%s$%0*X%s\n" % (name, m.group(2), width, a, m.group(4))
        lines.append(line)
    open(os.path.join(out, "ram.inc"), "w").write("".join(lines))


def moved_mapping(old, new, out):
    """The mapping with each RAM address moved to where its name went."""
    def move(a):
        best = None
        for n, b in sorted(old.items()):
            if b <= a and (best is None or b > best[1]):
                best = (n, b)
        if best is None:
            sys.exit("practice_variant.py: no name at or below $%X" % a)
        return new[best[0]] + (a - best[1])

    lines = []
    for line in open(os.path.join(REPO, "templates", "mosshop", "mapping.toml")).read().split("\n"):
        if re.match(r"^\s*(ram|frac|pages)\s*=", line):
            code, sep, comment = line.partition("#")
            code = re.sub(r"0x([0-9A-Fa-f]+)",
                          lambda m: "0x%0*X" % (len(m.group(1)), move(int(m.group(1), 16))), code)
            line = code + sep + comment
        lines.append(line)
    open(os.path.join(out, "mapping.toml"), "w").write("\n".join(lines))


def labels(path):
    """ld65's label file: name -> CPU address."""
    out = {}
    for line in open(path):
        parts = line.split()
        if len(parts) == 3 and parts[0] == "al":
            out[parts[2].lstrip(".")] = int(parts[1], 16)
    return out


def moved_assets(old, new, out, prg_base=0):
    """assets/mosshop.toml with each PRG offset moved as far as its label
    moved: a patch that adds code moves the data after it. `prg_base` is
    how far into the variant's PRG Moss Hop's 16KB bank now starts (a
    board with more banks before its fixed one)."""
    def move(name, off):
        if name not in old or name not in new:
            sys.exit("practice_variant.py: no label %s in both builds" % name)
        return off + new[name] - old[name] + prg_base

    lines, label, region = [], None, None
    for line in open(os.path.join(REPO, "assets", "mosshop.toml")).read().split("\n"):
        if line.startswith("[[asset]]"):
            label, region = None, None
        m = re.match(r'^label = "(\w+)"', line)
        if m:
            label = m.group(1)
        m = re.match(r'^region = "(\w+)"', line)
        if m:
            region = m.group(1)
        m = re.match(r"^offset = 0x([0-9A-Fa-f]+)(.*)$", line)
        if m and region == "prg" and label:
            line = "offset = 0x%04X%s" % (move(label, int(m.group(1), 16)), m.group(2))
        if 'region = "prg"' in line and line.lstrip().startswith("{"):
            for lk, ok in (("label", "offset"), ("hi_label", "hi_offset")):
                lm = re.search(r'\b%s = "(\w+)"' % lk, line)
                om = re.search(r"\b%s = 0x([0-9A-Fa-f]+)" % ok, line)
                if lm and om:
                    line = line.replace(om.group(0), "%s = 0x%04X" % (ok, move(lm.group(1), int(om.group(1), 16))))
        lines.append(line)
    open(os.path.join(out, "mosshop.toml"), "w").write("\n".join(lines))


def assemble(folder):
    subprocess.run(["ca65", "-g", "-I", folder, "-o", os.path.join(folder, "mosshop.o"),
                    os.path.join(folder, "mosshop.s")], check=True)
    subprocess.run(["ld65", "-C", os.path.join(folder, "nrom.cfg"), "-o", os.path.join(folder, "mosshop.nes"),
                    "--dbgfile", os.path.join(folder, "mosshop.dbg"), "-Ln", os.path.join(folder, "labels.txt"),
                    os.path.join(folder, "mosshop.o")], check=True)


def key(out, zp):
    s = open(os.path.join(SRC, "static-key.toml")).read()
    for field, path in (("model", os.path.join(REPO, "templates", "mosshop", "model.toml")),
                        ("mapping", os.path.join(out, "mapping.toml")),
                        ("ground_truth", os.path.join(out, "ground-truth.toml")),
                        ("ram", os.path.join(out, "ram.inc"))):
        s, n = re.subn(r'^%s = ".*"$' % field, '%s = "%s"' % (field, path), s, count=1, flags=re.M)
        if n != 1:
            sys.exit("practice_variant.py: the key has no %s" % field)
    # The harvest names a slot group, a state byte or a counter by its
    # address, so those names move with the zero page.
    s = re.sub(r"\b(slots_|state_|c_)([0-9A-F]{2})\b",
               lambda m: "%s%02X" % (m.group(1), int(m.group(2), 16) + zp), s)
    open(os.path.join(out, "key.toml"), "w").write(s)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("name")
    ap.add_argument("moves", nargs="*", help="zp=+N or LABEL=ADDR (hex)")
    ap.add_argument("--out")
    ap.add_argument("--patch", help="a unified diff of games/mosshop/mosshop.s")
    ap.add_argument("--part-of", action="append", default=[], metavar="LABEL=ROUTINE")
    ap.add_argument("--element", action="append", default=[], metavar="LABEL=TAG")
    ap.add_argument("--cfg", help="an ld65 layout in place of nrom.cfg (another board)")
    ap.add_argument("--prg-base", type=lambda x: int(x, 0), default=0,
                    help="where Moss Hop's bank starts in the variant's PRG (0xC000 on a 64KB board)")
    a = ap.parse_args()
    home = os.environ.get("GP_HOME", os.path.expanduser("~/.game-printer"))
    out = os.path.abspath(a.out or os.path.join(home, "work", "variants", a.name))
    os.makedirs(out, exist_ok=True)
    zp, moves = 0, {}
    for m in a.moves:
        k, _, v = m.partition("=")
        if k == "zp":
            zp = int(v, 0)
        else:
            moves[k] = int(v, 16)
    for f in ("mosshop.s", "chr.s", "nrom.cfg"):
        open(os.path.join(out, f), "w").write(open(os.path.join(SRC, f)).read())
    if a.patch:
        subprocess.run(["patch", "-s", "-p3", "-d", out, "-i", os.path.abspath(a.patch)], check=True)
    if a.cfg:
        open(os.path.join(out, "nrom.cfg"), "w").write(open(a.cfg).read())
    write_ram(out, moves, zp)
    assemble(out)
    # Moss Hop itself, for where its labels are.
    base = os.path.join(out, "moss-hop")
    os.makedirs(base, exist_ok=True)
    for f in ("mosshop.s", "chr.s", "nrom.cfg", "ram.inc"):
        open(os.path.join(base, f), "w").write(open(os.path.join(SRC, f)).read())
    assemble(base)
    moved_assets(labels(os.path.join(base, "labels.txt")), labels(os.path.join(out, "labels.txt")), out,
                 a.prg_base)
    subprocess.run([sys.executable, "-I", "-B", os.path.join(REPO, "scripts", "mosshop", "ground_truth.py"),
                    "--src", out, "--out", os.path.join(out, "ground-truth.toml")]
                   + ["--part-of=" + x for x in a.part_of] + ["--element=" + x for x in a.element],
                   check=True, stdout=subprocess.DEVNULL)
    moved_mapping(ram(os.path.join(SRC, "ram.inc")), ram(os.path.join(out, "ram.inc")), out)
    key(out, zp)
    print("practice_variant.py: %s built in %s" % (a.name, out))


if __name__ == "__main__":
    main()
