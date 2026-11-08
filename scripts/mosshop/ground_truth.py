#!/usr/bin/env python3
"""Moss Hop's ground truth from its source: every routine, every branch,
and the inputs each routine reads.

    scripts/mosshop/ground_truth.py            # build, then write games/mosshop/ground-truth.toml
    scripts/mosshop/ground_truth.py --check    # exit 1 if the committed file is stale
    scripts/mosshop/ground_truth.py --src DIR --out FILE [--part-of LABEL=ROUTINE ...]

With --src it reads a variant of Moss Hop instead: DIR holds the variant's
mosshop.s and ram.inc and the build of them (the ROM and ld65's debug file,
as a held-out test's folder does), so each variant is graded against its
own addresses. --part-of says a top-level label is a place in another
routine's code (a jump table's entry the routine runs on into, or the table
itself), not a routine of its own; --element tags a routine the variant
adds. A variant gets no never-rows: they are keyed to Moss Hop's source
lines.

It builds the game with scripts/mosshop/build.sh into a temporary folder and
reads three things: games/mosshop/mosshop.s for the routines (top-level
labels in the CODE segment), their instructions and branches; ram.inc for
what each RAM address is; and ld65's debug file for the address of every
source line, so each routine and branch carries the CPU address a trace of
the ROM sees. Nothing here reads the ROM's bytes. The file it writes is what
a generator that reads only the ROM is measured against
(docs/research/mosshop-testbed.md).

A routine's inputs are the memory its own instructions read (a load, a
compare, arithmetic, a bit test, or the read half of a read-modify-write),
sorted into kinds: the controller, the frame counter, the level's cells, the
game's state, scratch, hardware registers and ROM tables. A read through a
pointer is named by what the pointer is set to in that routine's code.
Calls are listed but their reads aren't folded in: a trace attributes a read
to the routine whose instruction made it, and so does this file.
"""

import argparse
import hashlib
import os
import re
import subprocess
import sys
import tempfile

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SRC = os.path.join(REPO, "games", "mosshop")
OUT = os.path.join(SRC, "ground-truth.toml")

READS = {"lda", "ldx", "ldy", "cmp", "cpx", "cpy", "adc", "sbc", "and", "ora", "eor", "bit"}
RMW = {"inc", "dec", "asl", "lsr", "rol", "ror"}
WRITES = {"sta", "stx", "sty"}
BRANCHES = {"bcc", "bcs", "beq", "bne", "bmi", "bpl", "bvc", "bvs"}
HARDWARE = {"PPUCTRL", "PPUMASK", "PPUSTATUS", "OAMADDR", "PPUSCROLL", "PPUADDR",
            "PPUDATA", "DMC_FREQ", "OAMDMA", "JOY1", "APU_FRAME"}

# What each routine is part of: the element tags a generator's rows can be
# grouped by. Every routine in the CODE segment must have one.
ELEMENT = {
    "reset": "system", "main": "system", "nmi": "system", "irq": "system",
    "redraw_cells": "level", "new_game": "rules", "start_level": "rules",
    "restore_level": "level", "restore_half": "level", "read_pad": "input",
    "move_player": "player", "apply_gravity": "player", "move_vertically": "player",
    "bump_head": "block", "take_coin": "coin", "step_right": "player",
    "step_left": "player", "solid_beside_player": "player",
    "solid_under_player": "player", "solid_at": "level", "cell_at": "level",
    "set_cell": "level", "queue_redraw": "level", "kill_player": "player",
    "update_dying": "rules", "tick_squashed": "slug", "move_enemies": "slug",
    "touch_enemies": "slug", "follow_player": "camera", "hide_sprites": "draw",
    "draw_sprites": "draw", "draw_hud": "hud", "hud_number": "hud",
    "hud_sprite": "hud", "draw_metasprite": "draw", "load_palette": "draw",
    "draw_level": "level",
}


# Pointers a routine reads through that its caller sets: restore_level sets
# restore_half's source row (the level in ROM) and its cells (in a page).
POINTERS = {
    "restore_half": {"TMP_PTR": ["level"], "TMP_DST": ["LEVEL_LEFT", "LEVEL_RIGHT"]},
}


# Branch directions the level's data never lets the game take in play,
# from reading the code against the level and the patrols: a row a
# generator can't fill, whatever it does. Keyed by (routine, source line).
NEVER = {
    ("step_right", "taken"): (575, "the goal at x 468 clears the level before the player can reach WORLD_MAX_X, 496"),
    ("cell_at", "taken@667"): (667, "no point the game tests is below row 240 in play"),
    ("cell_at", "taken@670"): (670, "no point the game tests is past the level's two pages in play"),
    ("queue_redraw", "taken"): (712, "at most six cells change before a restart, under REDRAW_MAX, 8"),
    ("move_enemies", "not_taken@813"): (813, "no patrol crosses a page boundary walking right: the first is 128-176, the second 256-352"),
    ("move_enemies", "taken@821"): (821, "a patrol's end and its x share a high byte"),
    ("move_enemies", "not_taken@827"): (827, "the second slug turns at 256, so it never steps left from a low byte of 0"),
    ("move_enemies", "taken@836"): (836, "a patrol's start and its x share a high byte"),
}


def never_for(name, line, direction):
    for (n, d), (ln, why) in NEVER.items():
        if n != name:
            continue
        if "@" in d:
            dd, at = d.split("@")
            if dd == direction and int(at) == line:
                return why
        elif d == direction and (ln is None or ln == line):
            return why
    return None


def play_routines(routines):
    """The routines reached from the main loop and the NMI by calls and
    jumps: everything else runs only at power-on."""
    by = {r["name"]: r for r in routines}
    seen, todo = set(), ["main", "nmi"]
    while todo:
        n = todo.pop()
        if n in seen or n not in by:
            continue
        seen.add(n)
        todo += by[n]["calls"]
    # irq is the NMI handler's last instruction, an RTI it falls into.
    seen.add("irq")
    return seen


def build(out):
    subprocess.run([os.path.join(REPO, "scripts", "mosshop", "build.sh"), out],
                   check=True, stdout=subprocess.DEVNULL)


def ram_map():
    """ram.inc's names: address and the comment."""
    out = {}
    for line in open(os.path.join(SRC, "ram.inc")):
        m = re.match(r"^(\w+)\s*=\s*\$([0-9A-Fa-f]+)\s*(?:;\s*(.*))?$", line)
        if m:
            out[m.group(1)] = (int(m.group(2), 16), (m.group(3) or "").strip())
    return out


# Source line -> the bank its code is in, for code in a switched region
# (line_addrs fills it on a banked board).
LINE_BANK = {}


def line_addrs(dbg, bank_size=None):
    """Source line number of mosshop.s -> CPU address, from ld65's debug file.
    With `bank_size` (a banked board's switched bank), lines in a segment at
    $8000 to $BFFF also get their bank in LINE_BANK, from where the segment
    is in the file."""
    segs, spans, files, lines = {}, {}, {}, []
    seg_bank = {}
    for raw in open(dbg):
        kind, _, rest = raw.rstrip("\n").partition("\t")
        f = dict(kv.split("=", 1) for kv in rest.split(",") if "=" in kv)
        if kind == "seg":
            segs[f["id"]] = int(f["start"], 16)
            if bank_size and 0x8000 <= segs[f["id"]] < 0xC000 and "ooffs" in f:
                seg_bank[f["id"]] = (int(f["ooffs"]) - 16) // bank_size
        elif kind == "span":
            spans[f["id"]] = (f["seg"], int(f["start"]))
        elif kind == "file":
            files[f["id"]] = f["name"].strip('"')
        elif kind == "line":
            lines.append(f)
    main = [k for k, v in files.items() if v.endswith("mosshop.s")][0]
    out = {}
    for f in lines:
        if f.get("file") != main or "span" not in f:
            continue
        # A line in a macro has several spans; the first is its own.
        seg, start = spans[f["span"].split("+")[0]]
        addr = segs[seg] + start
        n = int(f["line"])
        out[n] = min(out.get(n, addr), addr)
        if seg in seg_bank:
            LINE_BANK[n] = seg_bank[seg]
    return out


def symbol(operand):
    """The symbol an operand names and its offset: `ENEMY_X,x` -> ENEMY_X, 0."""
    op = operand.strip()
    if not op or op.startswith("#") or op.lower() == "a":
        return None
    indirect = op.startswith("(")
    m = re.match(r"\(?\s*([A-Za-z_]\w*)\s*(?:\+\s*(\d+))?", op)
    if not m:
        return None
    return m.group(1), int(m.group(2) or 0), indirect


def kind_of(name, ram):
    if name in HARDWARE:
        return "controller" if name == "JOY1" else "hardware"
    if name not in ram:
        return "rom table"
    if name in ("PAD", "PAD_PREV", "PAD_NEW"):
        return "controller"
    if name == "FRAMES":
        return "frame counter"
    if name in ("LEVEL_LEFT", "LEVEL_RIGHT"):
        return "tiles"
    if name.startswith("TMP_"):
        return "scratch"
    return "state"


def parse(addrs, ram, part_of=None):
    part_of = part_of or {}
    text = open(os.path.join(SRC, "mosshop.s")).read().split("\n")
    routines, cur, seg = [], None, None
    pointers = {}  # pointer -> what the routine set it to
    for n, raw in enumerate(text, 1):
        s = raw.split(";", 1)[0].rstrip()
        m = re.match(r'^\.segment\s+"(\w+)"', s)
        if m:
            seg = m.group(1)
            cur = None
            continue
        if not (seg == "CODE" or (seg or "").startswith("BANK")) or not s.strip():
            continue
        m = re.match(r"^([A-Za-z_]\w*):", s)
        if m and m.group(1) in part_of:
            if cur is None or cur["name"] != part_of[m.group(1)]:
                sys.exit("ground_truth.py: %s follows %s, not %s" % (
                    m.group(1), cur and cur["name"], part_of[m.group(1)]))
            continue
        if m:
            cur = {"name": m.group(1), "line": n, "reads": {}, "writes": set(),
                   "calls": [], "branches": [], "last": [], "insns": 0}
            pointers = dict(POINTERS.get(cur["name"], {}))
            routines.append(cur)
            continue
        m = re.match(r"^(@\w+|:)", s)
        if m:
            continue
        m = re.match(r"^\s+([a-z]{3})\b\s*(.*)$", s)
        if not m or cur is None:
            continue
        op, operand = m.group(1), m.group(2).strip()
        cur["insns"] += 1
        if "addr" not in cur and n in addrs:
            cur["addr"] = addrs[n]
            if n in LINE_BANK:
                cur["bank"] = LINE_BANK[n]
        cur["end"] = n
        sym = symbol(operand)
        if op in BRANCHES:
            cond = "; ".join(cur["last"][-2:])
            cur["branches"].append({"line": n, "addr": addrs.get(n), "op": op,
                                    "target": operand, "after": cond})
        elif op in ("jsr", "jmp") and sym:
            cur["calls"].append(sym[0])
        if op in READS or (op in RMW and sym):
            if sym:
                name, off, ind = sym
                if ind:
                    # The pointer's two bytes, then what it points at.
                    cur["reads"].setdefault((name, 0), kind_of(name, ram))
                    cur["reads"].setdefault((name, 1), kind_of(name, ram))
                    for target in pointers.get(name, ["unknown"]):
                        cur["reads"].setdefault((target, 0), kind_of(target, ram))
                else:
                    cur["reads"].setdefault((name, off), kind_of(name, ram))
        if op in WRITES or (op in RMW and sym):
            if sym and sym[2]:
                for target in pointers.get(sym[0], ["unknown"]):
                    cur["writes"].add((target, 0))
            elif sym:
                cur["writes"].add((sym[0], sym[1]))
        # What a pointer is set to: `adc #<LEVEL_LEFT` / `lda level_rows_lo,x`
        # then `sta TMP_PTR`.
        if op == "sta" and sym and sym[0] in ("TMP_PTR", "TMP_DST") and sym[1] == 0:
            for prev in reversed(cur["last"]):
                pm = re.search(r"#<(\w+)|lda (\w+)_lo", prev)
                if pm:
                    tgt = pm.group(1) or pm.group(2)
                    pointers[sym[0]] = ["level" if tgt == "level_rows" else tgt]
                    break
        cur["last"].append(("%s %s" % (op, operand)).strip())
    return routines


def toml_str(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def render(routines, ram, sha):
    def addr_of(name, off):
        if name in ram:
            return ram[name][0] + off
        return None

    play = play_routines(routines)
    lines = [
        "# Moss Hop's ground truth: every routine in games/mosshop/mosshop.s, every",
        "# branch, and the inputs each routine's own instructions read. Written by",
        "# scripts/mosshop/ground_truth.py from the source, ram.inc and ld65's debug",
        "# file; `scripts/mosshop/ground_truth.py --check` fails when it is stale.",
        "# A generator that reads only the ROM is measured against it",
        "# (docs/research/mosshop-testbed.md). Addresses are CPU addresses.",
        "",
        "[ground_truth]",
        "format = \"0.1.0\"",
        "rom_sha256 = %s" % toml_str(sha),
        "routines = %d" % len(routines),
        "branches = %d" % sum(len(r["branches"]) for r in routines),
        "# A row is a routine's run or one direction of one of its branches.",
        "rows = %d" % (len(routines) + 2 * sum(len(r["branches"]) for r in routines)),
        "inputs = %d" % sum(len([k for k in r["reads"] if r["reads"][k] != "scratch"]) for r in routines),
        "# Routines that run only at power-on, before the main loop: no clip",
        "# starts there.",
        "power_on = [%s]" % ", ".join(toml_str(r["name"]) for r in routines if r["name"] not in play),
        "",
    ]
    for r in routines:
        lines += [
            "[[routine]]",
            "name = %s" % toml_str(r["name"]),
            "element = %s" % toml_str(ELEMENT[r["name"]]),
            "reached = %s" % toml_str("play" if r["name"] in play else "power-on"),
            "addr = 0x%04X" % r["addr"],
        ] + (["bank = %d" % r["bank"]] if "bank" in r else []) + [
            "line = %d" % r["line"],
            "instructions = %d" % r["insns"],
        ]
        calls = sorted(set(r["calls"]))
        lines.append("calls = [%s]" % ", ".join(toml_str(c) for c in calls))
        reads = []
        for (name, off), kind in sorted(r["reads"].items(), key=lambda kv: (kv[0][0], kv[0][1])):
            label = name if off == 0 else "%s+%d" % (name, off)
            a = addr_of(name, off)
            at = ", addr = 0x%04X" % a if a is not None else ""
            reads.append("  { label = %s%s, kind = %s }," % (toml_str(label), at, toml_str(kind)))
        lines.append("reads = [")
        lines += reads
        lines.append("]")
        lines.append("writes = [%s]" % ", ".join(
            toml_str(n if o == 0 else "%s+%d" % (n, o)) for n, o in sorted(r["writes"])))
        for b in r["branches"]:
            lines += [
                "",
                "[[routine.branch]]",
                "addr = 0x%04X" % b["addr"],
                "line = %d" % b["line"],
                "op = %s" % toml_str(b["op"]),
                "taken_to = %s" % toml_str(b["target"]),
                "after = %s" % toml_str(b["after"]),
            ]
            for d in ("taken", "not_taken"):
                why = never_for(r["name"], b["line"], d)
                if why:
                    lines.append("never_%s = %s" % (d, toml_str(why)))
        lines.append("")
    return "\n".join(lines)


def only(folder, ext):
    """The one file in a folder with this extension."""
    found = sorted(f for f in os.listdir(folder) if f.endswith(ext))
    if len(found) != 1:
        sys.exit("ground_truth.py: %s has %d %s files, not one" % (folder, len(found), ext))
    return os.path.join(folder, found[0])


def main():
    global SRC, NEVER
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--check", action="store_true", help="exit 1 if the committed file differs")
    ap.add_argument("--out", default=OUT)
    ap.add_argument("--src", help="a variant's folder: its source, ram.inc, ROM and debug file")
    ap.add_argument("--part-of", action="append", default=[], metavar="LABEL=ROUTINE",
                    help="a top-level label that is a place in this routine's code")
    ap.add_argument("--element", action="append", default=[], metavar="LABEL=TAG",
                    help="the element tag of a routine a variant adds")
    a = ap.parse_args()
    part_of = dict(x.split("=", 1) for x in a.part_of)
    ELEMENT.update(dict(x.split("=", 1) for x in a.element))
    if a.src:
        if a.out == OUT:
            sys.exit("ground_truth.py: --src needs --out")
        SRC = os.path.abspath(a.src)
        NEVER = {}
        ram = ram_map()
        rom = open(only(SRC, ".nes"), "rb").read()
        # A board with more PRG than the CPU sees: its switched bank's size.
        mapper = (rom[6] >> 4) | (rom[7] & 0xF0)
        bank_size = {1: 0x4000, 2: 0x4000, 4: 0x2000}.get(mapper) if rom[4] > 2 else None
        addrs = line_addrs(only(SRC, ".dbg"), bank_size)
    else:
        ram = ram_map()
        with tempfile.TemporaryDirectory() as tmp:
            build(tmp)
            addrs = line_addrs(os.path.join(tmp, "mosshop.dbg"))
            rom = open(os.path.join(tmp, "mosshop.nes"), "rb").read()
    sha = hashlib.sha256(rom[16:]).hexdigest()
    routines = parse(addrs, ram, part_of)
    missing = [r["name"] for r in routines if r["name"] not in ELEMENT]
    if missing:
        sys.exit("ground_truth.py: routines with no element tag: %s" % ", ".join(missing))
    text = render(routines, ram, sha)
    if a.check:
        old = open(a.out).read() if os.path.exists(a.out) else ""
        if old != text:
            sys.exit("ground_truth.py: %s is stale; run scripts/mosshop/ground_truth.py" % a.out)
        print("ground_truth.py: %s is current" % a.out)
        return
    open(a.out, "w").write(text)
    print("ground_truth.py: %d routines, %d branches, written to %s" % (
        len(routines), sum(len(r["branches"]) for r in routines), a.out))


if __name__ == "__main__":
    main()
