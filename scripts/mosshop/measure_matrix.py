#!/usr/bin/env python3
"""Measure a `gp clips matrix` run on Moss Hop against its ground truth.

    scripts/mosshop/measure_matrix.py MATRIX_DIR [--again MATRIX_DIR] [--json]

MATRIX_DIR is a folder `gp clips matrix --write` wrote (matrix.json,
recipes.toml). It reads games/mosshop/ground-truth.toml and labels.toml
and prints recall, precision, inputs found, clips needed and, with
--again, whether a second run wrote the same bytes and how much of the
target grid the two share (docs/research/mosshop-testbed.md).

A ground-truth row is a routine's run or one direction of one of its
branches. The matrix finds a run row when one of its routines starts at
the routine's address, and a branch row by the branch's address and
direction. An input is a RAM label a routine's own instructions read,
other than scratch; the matrix finds it when one of its columns, in a row
of a matrix routine that starts inside the ground-truth routine, is a byte
of that label.
"""

import argparse
import json
import os
import sys
import tomllib

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
INPUT_KINDS = ("controller", "frame counter", "tiles", "state")


def load_gt():
    gt = tomllib.load(open(os.path.join(REPO, "games/mosshop/ground-truth.toml"), "rb"))
    labels = tomllib.load(open(os.path.join(REPO, "games/mosshop/labels.toml"), "rb"))["label"]
    return gt, labels


PORTS = {0x4016: "JOY1", 0x4017: "JOY2"}


def label_of(labels, addr):
    if addr in PORTS:
        return PORTS[addr]
    for l in labels:
        if l["addr"] <= addr < l["addr"] + l["size"]:
            return l["name"]
    return None


def gt_rows_keys(routines):
    out = set()
    for r in routines:
        if r.get("reached") != "play":
            continue
        out.add(("run", r["addr"]))
        for b in r.get("branch", []):
            for d in ("taken", "not_taken"):
                out.add((d, b["addr"]))
    return out


def measure(mdir, gt, labels):
    m = json.load(open(os.path.join(mdir, "matrix.json")))
    rows = m["row_list"]
    routines = gt["routine"]
    starts = sorted((r["addr"], r) for r in routines)

    def gt_routine_at(a):
        best = None
        for s, r in starts:
            if s <= a:
                best = r
        return best

    run_rows = {int(r["routine_addr"], 16): r for r in rows if r["kind"] == "run"}
    br_rows = {(int(r["addr"], 16), r["kind"]): r for r in rows if r["kind"] != "run"}
    gt_rows = []
    reachable = set()
    for r in routines:
        play = r.get("reached") == "play"
        gt_rows.append(("run", r["addr"], r))
        if play:
            reachable.add(("run", r["addr"]))
        for b in r["branch"] if "branch" in r else []:
            for d in ("taken", "not_taken"):
                gt_rows.append((d, b["addr"], r))
                if play and ("never_" + d) not in b:
                    reachable.add((d, b["addr"]))
    gt_keys = set()
    found, filled, full = 0, 0, 0
    r_found, r_filled = 0, 0
    fillable, fillable_filled = 0, 0
    per_el = {}
    missed = []
    for kind, a, r in gt_rows:
        mr = run_rows.get(a) if kind == "run" else br_rows.get((a, kind))
        gt_keys.add((kind, a))
        e = per_el.setdefault(r["element"], [0, 0])
        e[1] += 1
        if mr is None:
            missed.append("%s %s %04X" % (r["name"], kind, a))
            continue
        found += 1
        e[0] += 1
        ok = mr["filled_after"] > 0 or not mr["columns"]
        if (kind, a) in reachable:
            r_found += 1
            r_filled += ok
        if ok:
            filled += 1
        if mr["filled_after"] == len(mr["columns"]):
            full += 1
        if (kind, a) in reachable:
            fillable += len(mr["columns"])
            fillable_filled += mr["filled_after"]
    gen_keys = [("run", int(r["routine_addr"], 16)) if r["kind"] == "run" else (r["kind"], int(r["addr"], 16)) for r in rows]
    real = sum(1 for k in gen_keys if k in gt_keys)

    # Inputs, per ground-truth routine.
    cols_by_gt = {}
    for r in rows:
        g = gt_routine_at(int(r["routine_addr"], 16))
        if g is None:
            continue
        s = cols_by_gt.setdefault(g["name"], set())
        s.update(int(c, 16) for c in r["columns"])
    inputs_total, inputs_found, col_total, col_real = 0, 0, 0, 0
    per_routine = {}
    for r in routines:
        want = {x["label"].split("+")[0] for x in r["reads"] if x["kind"] in INPUT_KINDS}
        reads_any = {x["label"].split("+")[0] for x in r["reads"]}
        cols = cols_by_gt.get(r["name"], set())
        got_labels = {label_of(labels, c) for c in cols}
        hit = want & got_labels
        inputs_total += len(want)
        inputs_found += len(hit)
        col_total += len(cols)
        col_real += sum(1 for c in cols if label_of(labels, c) in reads_any)
        per_routine[r["name"]] = {"want": sorted(want), "found": sorted(hit), "missed": sorted(want - got_labels)}

    # The never-taken detector: rows the matrix says play never takes,
    # against the ground truth's never_taken and never_not_taken.
    gt_never = {k for k in gt_rows_keys(routines) if k not in reachable and k[0] != "run"}
    said = {(r["kind"], int(r["addr"], 16)) for r in rows
            if (r.get("why") or "").startswith("never taken in play")}
    never = {"said": len(said), "right": len(said & gt_never), "ground_truth": len(gt_never & {
        (r["kind"], int(r["addr"], 16)) for r in rows}),
        "wrong": sorted("%s %04X" % k for k in said - gt_never),
        "missed": sorted("%s %04X" % k for k in (gt_never - said) if k in {(r["kind"], int(r["addr"], 16)) for r in rows})}
    recipes = m["recipes"]
    existing = m["existing"]
    kept = existing["clips"] - len(existing["drop_candidates"])
    cells = {(r["kind"], r["addr"], c) for r in rows for c in r["columns"]}
    return {
        "gt_rows": len(gt_rows),
        "found": found,
        "recall": found / len(gt_rows),
        "reachable": len(reachable),
        "reachable_found": r_found,
        "reachable_filled": r_filled,
        "filled": filled,
        "full": full,
        "gen_rows": len(rows),
        "real_rows": real,
        "precision": real / len(rows) if rows else 0,
        "routines_found": sum(1 for r in routines if r["addr"] in run_rows),
        "routines": len(routines),
        "gen_routines": len(run_rows),
        "inputs_total": inputs_total,
        "inputs_found": inputs_found,
        "column_precision": col_real / col_total if col_total else 0,
        "cells": m["cells"],
        "filled_cells": m["filled_after"],
        "fillable_cells": fillable,
        "fillable_filled": fillable_filled,
        "recipes": len(recipes),
        "recipe_frames": sum(r["frames"] for r in recipes),
        "existing_kept": kept,
        "clips_needed": len(recipes) + kept,
        "gaps": m["gaps"],
        "per_element": {k: "%d/%d" % tuple(v) for k, v in sorted(per_el.items())},
        "missed_rows": missed,
        "per_routine": per_routine,
        "never": never,
        "_cells": cells,
    }


def jaccard(a, b):
    return len(a & b) / len(a | b) if a | b else 1.0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("mdir")
    ap.add_argument("--again", help="a second run's folder: determinism or stability")
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--verbose", action="store_true")
    a = ap.parse_args()
    gt, labels = load_gt()
    r = measure(a.mdir, gt, labels)
    if a.again:
        r2 = measure(a.again, gt, labels)
        same = all(open(os.path.join(a.mdir, f), "rb").read() == open(os.path.join(a.again, f), "rb").read()
                   for f in ("matrix.json", "recipes.toml"))
        r["again"] = {"byte_identical": same, "cells_jaccard": jaccard(r["_cells"], r2["_cells"]),
                      "rows_jaccard": jaccard({c[:2] for c in r["_cells"]}, {c[:2] for c in r2["_cells"]}),
                      "recipes": r2["recipes"], "filled_cells": r2["filled_cells"]}
    del r["_cells"]
    if a.json:
        json.dump(r, sys.stdout, indent=1)
        return
    print("rows: %d of %d ground-truth rows found (recall %.1f%%), %d with a cell filled, %d full" % (
        r["found"], r["gt_rows"], 100 * r["recall"], r["filled"], r["full"]))
    print("reachable in play: %d rows; %d found (%.1f%%), %d with a cell filled (%.1f%%)" % (
        r["reachable"], r["reachable_found"], 100 * r["reachable_found"] / r["reachable"],
        r["reachable_filled"], 100 * r["reachable_filled"] / r["reachable"]))
    print("precision: %d of %d generated rows are ground-truth rows (%.1f%%)" % (r["real_rows"], r["gen_rows"], 100 * r["precision"]))
    print("routines: %d of %d found by their start; %d generated" % (r["routines_found"], r["routines"], r["gen_routines"]))
    print("inputs: %d of %d found (%.1f%%); column precision %.1f%%" % (
        r["inputs_found"], r["inputs_total"], 100 * r["inputs_found"] / max(1, r["inputs_total"]), 100 * r["column_precision"]))
    print("cells: %d, %d filled; clips needed %d (%d recipes, %d frames, and %d of the run's clips)" % (
        r["cells"], r["filled_cells"], r["clips_needed"], r["recipes"], r["recipe_frames"], r["existing_kept"]))
    print("fillable cells (on rows reachable in play): %d, %d filled (%.1f%%)" % (
        r["fillable_cells"], r["fillable_filled"], 100 * r["fillable_filled"] / max(1, r["fillable_cells"])))
    print("gaps:", r["gaps"])
    n = r["never"]
    print("never-taken detector: says %d rows, %d of them right, of %d in the ground truth; wrong: %s; missed: %s" % (
        n["said"], n["right"], n["ground_truth"], ", ".join(n["wrong"]) or "none", ", ".join(n["missed"]) or "none"))
    print("per element:", r["per_element"])
    if "again" in r:
        print("again:", r["again"])
    if a.verbose:
        print("missed rows:", "; ".join(r["missed_rows"]))
        for k, v in r["per_routine"].items():
            if v["missed"]:
                print("  %s missed inputs: %s" % (k, ", ".join(v["missed"])))


if __name__ == "__main__":
    main()
