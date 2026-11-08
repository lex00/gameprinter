#!/usr/bin/env python3
"""Measure `gp capture grid`'s targets on Moss Hop against its ground truth.

    scripts/mosshop/measure_grid.py GRID_JSON [--again GRID_JSON ...] [--json]

GRID_JSON is what `gp --json capture grid RUN` prints. With a code part
(`--code`), its rows `code:sub_XXXX[:taken@ADDR]` are the code matrix's,
and this says how many of games/mosshop/ground-truth.toml's rows reachable
in play the grid holds, how many have a cell with a clip, which inputs its
columns find, and whether the parts of the game a player sees (the ? block,
the coins, the slug, death, the pit, game over and the level's end) each
have a row. With --again, the share of cells two grids have in common
(Jaccard), for the code part and for the whole grid
(docs/research/mosshop-testbed.md).
"""

import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from measure_matrix import INPUT_KINDS, label_of, load_gt  # noqa: E402

# What a player sees, by the ground truth's routine and the source line of
# a branch (games/mosshop/mosshop.s): (words, routine, line or None, way).
STORY = [
    ("? block bumped", "bump_head", None, "run"),
    ("coin taken", "take_coin", None, "run"),
    ("slug walks", "move_enemies", None, "run"),
    ("slug touched or stomped", "touch_enemies", None, "run"),
    ("squashed slug's time", "tick_squashed", None, "run"),
    ("death", "kill_player", None, "run"),
    ("pit", "move_player", 466, "not_taken"),
    ("game over", "update_dying", 772, "taken"),
    ("level end", "move_player", 473, "not_taken"),
]


def load(path):
    d = json.load(open(path))
    return d.get("data", d)


def parse_row(rid):
    """`code:sub_C2D8` -> ("run", 0xC2D8); `code:sub_C2D8:taken@C2E0` -> ("taken", 0xC2E0)."""
    parts = rid.split(":")
    if len(parts) == 2:
        return ("run", int(parts[1].split("_")[-1], 16))
    kind, at = parts[2].split("@")
    return (kind, int(at, 16))


def measure(g, gt, labels):
    routines = gt["routine"]
    starts = sorted((r["addr"], r) for r in routines)

    def gt_at(a):
        best = None
        for s, r in starts:
            if s <= a:
                best = r
        return best

    reachable = set()
    gt_rows = {}
    for r in routines:
        play = r.get("reached") == "play"
        gt_rows[("run", r["addr"])] = r
        if play:
            reachable.add(("run", r["addr"]))
        for b in r.get("branch", []):
            for d in ("taken", "not_taken"):
                gt_rows[(d, b["addr"])] = r
                if play and ("never_" + d) not in b:
                    reachable.add((d, b["addr"]))
    cells = [c for c in g["cells"] if c["row"].startswith("code:")]
    model_cells = [c for c in g["cells"] if not c["row"].startswith("code:")]
    code_rows = [r for r in g["rows"] if r["kind"] == "code"]
    keys = {r["id"]: parse_row(r["id"]) for r in code_rows}
    filled_rows = {c["row"] for c in cells if c["clips"]}
    has_cells = {c["row"] for c in cells}
    found = {k for k in keys.values() if k in gt_rows}
    r_found = found & reachable
    r_filled = {keys[r] for r in keys if keys[r] in reachable and (r in filled_rows or r not in has_cells)}
    # Inputs: a routine's columns are its rows' cells' columns.
    cols_by = {}
    for c in cells:
        kind, a = keys[c["row"]]
        routine_addr = int(c["row"].split(":")[1].split("_")[-1], 16)
        r = gt_at(routine_addr)
        if r is None:
            continue
        cols_by.setdefault(r["name"], set()).add(int(c["col"].split(":")[1], 16))
    in_total, in_found, missed = 0, 0, []
    for r in routines:
        want = {x["label"].split("+")[0] for x in r["reads"] if x["kind"] in INPUT_KINDS}
        got = {label_of(labels, a) for a in cols_by.get(r["name"], set())}
        in_total += len(want)
        in_found += len(want & got)
        missed += ["%s %s" % (r["name"], m) for m in sorted(want - got)]
    # The story: each part's row and whether a clip covers a cell of it.
    by_name = {r["name"]: r for r in routines}
    story = {}
    for words, name, line, way in STORY:
        r = by_name[name]
        if line is None:
            key = ("run", r["addr"])
        else:
            b = next(b for b in r["branch"] if b["line"] == line)
            key = (way, b["addr"])
        rid = next((i for i, k in keys.items() if k == key), None)
        story[words] = "missing" if rid is None else ("filled" if rid in filled_rows or rid not in has_cells else "empty")
    per_el = {}
    for k in found:
        e = gt_rows[k]["element"]
        per_el[e] = per_el.get(e, 0) + 1
    return {
        "code_rows": len(code_rows),
        "real_rows": len(found),
        "precision": len(found) / len(code_rows) if code_rows else 0,
        "reachable": len(reachable),
        "reachable_found": len(r_found),
        "reachable_filled": len(r_filled),
        "code_cells": len(cells),
        "code_filled": sum(1 for c in cells if c["clips"]),
        "code_gaps": sum(1 for c in cells if not c["clips"] and c.get("gap")),
        "model_cells": len(model_cells),
        "model_filled": sum(1 for c in model_cells if c["clips"]),
        "inputs_total": in_total,
        "inputs_found": in_found,
        "inputs_missed": missed,
        "elements": sorted({r["group"] for r in code_rows}),
        "per_gt_element": dict(sorted(per_el.items())),
        "story": story,
        "made": g.get("made", 0),
        "_code": {(c["row"], c["col"]) for c in cells},
        "_all": {(c["row"], c["col"]) for c in g["cells"]},
    }


def jaccard(a, b):
    return len(a & b) / len(a | b) if a | b else 1.0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("grid")
    ap.add_argument("--again", action="append", default=[], help="another grid's JSON: stability")
    ap.add_argument("--json", action="store_true")
    a = ap.parse_args()
    gt, labels = load_gt()
    r = measure(load(a.grid), gt, labels)
    r["again"] = []
    for p in a.again:
        r2 = measure(load(p), gt, labels)
        r["again"].append({"grid": p, "code_cells_jaccard": jaccard(r["_code"], r2["_code"]),
                           "all_cells_jaccard": jaccard(r["_all"], r2["_all"]),
                           "code_filled": r2["code_filled"], "code_cells": r2["code_cells"]})
    del r["_code"], r["_all"]
    if a.json:
        json.dump(r, sys.stdout, indent=1)
        return
    print("code rows: %d, %d of them ground-truth rows (precision %.1f%%)" % (r["code_rows"], r["real_rows"], 100 * r["precision"]))
    print("reachable in play: %d; %d in the grid, %d with a cell filled" % (r["reachable"], r["reachable_found"], r["reachable_filled"]))
    print("code cells: %d, %d filled, %d listed as gaps; model cells: %d, %d filled" % (
        r["code_cells"], r["code_filled"], r["code_gaps"], r["model_cells"], r["model_filled"]))
    print("inputs: %d of %d" % (r["inputs_found"], r["inputs_total"]), "missed:", ", ".join(r["inputs_missed"]))
    print("story:", ", ".join("%s %s" % kv for kv in r["story"].items()))
    print("elements (%d): %s" % (len(r["elements"]), "; ".join(r["elements"])))
    for x in r["again"]:
        print("again %s: code cells Jaccard %.3f, all cells %.3f, %d of %d filled" % (
            x["grid"], x["code_cells_jaccard"], x["all_cells_jaccard"], x["code_filled"], x["code_cells"]))


if __name__ == "__main__":
    main()
