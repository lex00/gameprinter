"""Build Game Printer's derived graph for one game as two Parquet tables.

Prototype for docs/research/graphs.md, brought from game-printer, where it
read templates/GAME/. Reads only; writes nodes.parquet, edges.parquet and
manifest.json into OUT. Nothing it writes may be committed: the code edges
come from the pinned disassembly under $GP_HOME/work/.

  python3 -I -B build_graph.py --game GAMES/donkey-kong [--source ID] \
      --export HF_DIR --asm DISASM.ASM --dbg DISASM.dbg \
      --coverage coverage.json --out OUT

GAME is a package folder in the games checkout ($GP_GAMES_DIR): its
model.toml, mapping.toml, sprites.toml (when it has one) and
assets/SOURCE.toml, where SOURCE is the ROM entry the asset map is named
for (default: the model's `source`). Attempts come from the git log of the
repo GAME is in. HF_DIR is game-printer's `gp dataset export --hf` output
(facts, mapping, mechanics, clips, scores), which gameprinter has no command
for yet. OUT should be outside every repo. Every id is "<kind>:<name>", so
the same inputs give the same rows in the same order, and the manifest
records each input's BLAKE2b hash.
"""

import argparse
import bisect
import hashlib
import json
import re
import subprocess
import tomllib
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq

WRITES = {"sta", "stx", "sty", "inc", "dec", "asl", "lsr", "rol", "ror"}
READS = {"lda", "ldx", "ldy", "cmp", "cpx", "cpy", "adc", "sbc", "and", "ora", "eor", "bit"}
RMW = {"inc", "dec", "asl", "lsr", "rol", "ror"}

LABEL = re.compile(r"^([A-Za-z_][A-Za-z0-9_]*):")
EQU = re.compile(r"^([A-Za-z_][A-Za-z0-9_]*)\s*=\s*\$([0-9A-Fa-f]+)")
INSN = re.compile(r"^\s+([a-z]{3})\b\s*([^;]*)")
DW = re.compile(r"^\s+\.dw\s+([^;]*)")
IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")


def h(path):
    return hashlib.blake2b(Path(path).read_bytes(), digest_size=16).hexdigest()


class G:
    def __init__(self):
        self.nodes = {}
        self.edges = set()

    def node(self, kind, name, **attrs):
        nid = f"{kind}:{name}"
        if nid not in self.nodes:
            self.nodes[nid] = (kind, name, json.dumps(attrs, sort_keys=True) if attrs else None)
        return nid

    def edge(self, src, rel, dst, via):
        self.edges.add((src, rel, dst, via))


def dbg_symbols(path):
    vals = {}
    for line in Path(path).read_text().splitlines():
        if not line.startswith("sym\t"):
            continue
        f = dict(kv.split("=", 1) for kv in line[4:].split(",") if "=" in kv)
        name = f.get("name", "").strip('"')
        if "val" in f:
            vals[name] = (int(f["val"], 16), f.get("type"))
    return vals


def code(g, asm, syms):
    """Routines, calls, jumps, dispatch tables, RAM and data reads."""
    lines = Path(asm).read_text(errors="replace").splitlines()
    src = Path(asm).name
    equ = {}
    labels = []  # (line_no, name)
    targets = set()
    for i, line in enumerate(lines, 1):
        if m := EQU.match(line):
            equ[m.group(1)] = int(m.group(2), 16)
        elif m := LABEL.match(line):
            labels.append((i, m.group(1)))
        if m := INSN.match(line):
            op, arg = m.group(1), m.group(2).strip()
            if op in ("jsr", "jmp") and (t := IDENT.match(arg)):
                targets.add(t.group(0))
        elif m := DW.match(line):
            targets.update(IDENT.findall(m.group(1)))
    labset = {n for _, n in labels}
    routines = {n for n in targets if n in labset}
    routines |= {"Start", "NonMaskableInterrupt"} & labset
    # A label is data when the lines under it hold no instruction.
    kinds = {}
    for k, (ln, name) in enumerate(labels):
        end = labels[k + 1][0] if k + 1 < len(labels) else len(lines)
        body = lines[ln:end - 1]
        kinds[name] = "code" if any(INSN.match(b) for b in body) else "data"
    cur = None
    for i, line in enumerate(lines, 1):
        if (m := LABEL.match(line)) and m.group(1) in routines:
            cur = m.group(1)
            g.node("routine", cur, addr=syms.get(cur, (None,))[0], line=i)
        if cur is None:
            continue
        r = f"routine:{cur}"
        if m := INSN.match(line):
            op, arg = m.group(1), m.group(2).strip()
            t = IDENT.search(arg)
            if not t:
                continue
            name = t.group(0)
            if op == "jsr" and name in routines:
                g.edge(r, "calls", g.node("routine", name), f"{src}:{i}")
            elif op == "jmp" and name in routines and name != cur:
                g.edge(r, "jumps_to", g.node("routine", name), f"{src}:{i}")
            elif arg.startswith("#"):
                if name in equ:
                    g.edge(r, "uses_const", g.node("const", name, value=equ[name]), f"{src}:{i}")
            elif name in equ and equ[name] < 0x800:
                ram = g.node("ram", name, addr=equ[name])
                if op in WRITES:
                    g.edge(r, "writes", ram, f"{src}:{i}")
                if op in READS or op in RMW:
                    g.edge(r, "reads", ram, f"{src}:{i}")
            elif kinds.get(name) == "data":
                g.edge(r, "reads_data", g.node("data", name, addr=syms.get(name, (None,))[0]), f"{src}:{i}")
        elif m := DW.match(line):
            for name in IDENT.findall(m.group(1)):
                if name in routines:
                    g.edge(r, "dispatches", g.node("routine", name), f"{src}:{i}")
    starts = sorted((syms[n][0], n) for n in routines if n in syms and syms[n][0] >= 0x8000)
    return equ, starts


def assets(g, game, source):
    amap = tomllib.loads((game / f"assets/{source}.toml").read_text())
    for a in amap["asset"]:
        an = g.node("asset", a["name"], format=a["format"])
        labs = [a.get("label")] + [c.get("label") for c in a.get("column", [])]
        labs += [e.get("label") for e in a.get("entry", [])]
        for lab in filter(None, labs):
            base = lab.split("+")[0].split("-")[0]
            g.edge(an, "stored_at", g.node("data", base), f"assets/{source}.toml")
    model = tomllib.loads((game / "model.toml").read_text())
    for slot, v in model["assets"].items():
        s = g.node("slot", slot, format=v.get("kind"))
        if (src := v.get("from")) and src.get("asset"):
            g.edge(s, "lifted_from", g.node("asset", src["asset"]), "model.toml")
    sp = game / "sprites.toml"
    sprites = tomllib.loads(sp.read_text()) if sp.exists() else {}
    for t in sprites.get("table", []):
        g.edge("slot:sprites", "built_from", g.node("asset", t["asset"]), "sprites.toml")
    for ent, e in model["entities"].items():
        en = g.node("entity", ent, role=e.get("role"))
        spr = e.get("sprite") or {}
        rows = set()
        for c in spr.get("clips", []):
            rows |= {f["row"] for f in c.get("frames", []) if "row" in f}
        if "frame" in spr and "row" in spr["frame"]:
            rows.add(spr["frame"]["row"])
        for row in sorted(rows):
            fr = g.node("frame", f"sprites[{row}]")
            g.edge(en, "drawn_with", fr, "model.toml")
            g.edge(fr, "in_slot", "slot:sprites", "model.toml")
        if spr.get("slot"):
            g.edge(en, "uses_tiles", g.node("slot", spr["slot"]), "model.toml")
    for lvl, L in model["levels"].items():
        ln = g.node("level", lvl)
        for sp in L.get("spawn", []):
            g.edge(ln, "spawns", f"entity:{sp['entity']}", "model.toml")
        for k, v in (L.get("draw") or {}).items():
            if isinstance(v, str) and f"slot:{v}" in g.nodes:
                g.edge(ln, "draws_with", f"slot:{v}", "model.toml")
    return model


def mapping(g, game, equ, export):
    mp = tomllib.loads((game / "mapping.toml").read_text())
    by_addr = {}
    for n, a in equ.items():
        if a < 0x800:
            by_addr.setdefault(a, []).append(n)
    for kind in mp.get("slots", {}).get("kind", []):
        en = f"entity:{kind['entity']}"
        for i in kind.get("ids", []):
            g.edge(en, "has_object_id", g.node("objid", f"0x{i:02X}"), "mapping.toml")
    import duckdb
    rows = duckdb.sql(f"select scope, path, owner, ram from '{export}/data/mapping.parquet'").fetchall()
    for scope, path, owner, ram in rows:
        f = g.node("field", f"{scope}:{path}", where=scope)
        for a in json.loads(ram or "[]"):
            addr = int(a, 16)
            for lab in by_addr.get(addr, []):
                g.edge(f"ram:{lab}", "mapped_to", f, "mapping.toml")
        if owner:
            g.edge(f, "measures_for", g.node("mechanic", owner), "mapping.toml")
        m = re.match(r"entities\.([a-z0-9_]+)", path)
        if m and f"entity:{m.group(1)}" in g.nodes:
            g.edge(f, "of_entity", f"entity:{m.group(1)}", "mapping.toml")
    # Object IDs are the disassembly's constants: Goomba = $06.
    return by_addr


def facts(g, export):
    import duckdb
    q = f"""select distinct path, mechanic, kind, evidence_entry, routine
            from '{export}/data/facts.parquet'"""
    for path, mech, kind, entry, routine in duckdb.sql(q).fetchall():
        f = g.node("fact", path, evidence=kind)
        if mech:
            g.edge(f, "part_of", g.node("mechanic", mech), "facts.parquet")
        if routine:
            g.edge(f, "cites", g.node("routine", routine), "facts.parquet")
        m = re.match(r"(entities|levels|assets)\.([a-z0-9_]+)", path)
        if m:
            kind2 = {"entities": "entity", "levels": "level", "assets": "slot"}[m.group(1)]
            if f"{kind2}:{m.group(2)}" in g.nodes:
                g.edge(f, "about", f"{kind2}:{m.group(2)}", "facts.parquet")
    q = f"select mechanic, used_by from '{export}/data/mechanics.parquet'"
    for mech, used_by in duckdb.sql(q).fetchall():
        m = re.match(r"entities\.([a-z0-9_]+)", used_by)
        if m and f"entity:{m.group(1)}" in g.nodes:
            g.edge(f"entity:{m.group(1)}", "carries", g.node("mechanic", mech), "mechanics.parquet")


def clips(g, export, coverage, starts):
    import duckdb
    q = f"select clip_id, level, holds, guards from '{export}/data/clips.parquet'"
    for cid, level, holds, guards in duckdb.sql(q).fetchall():
        c = g.node("clip", cid, kept=True)
        if level:
            g.edge(c, "in_level", g.node("level", level), "clips.toml")
        for hd in json.loads(holds or "[]") if isinstance(holds, str) else (holds or []):
            if hd.startswith("mechanic:"):
                g.edge(c, "holds", g.node("mechanic", hd[9:]), "clips.toml")
            elif hd.startswith("part:"):
                _, mech, path = hd.split(":", 2)
                g.edge(c, "holds_path", g.node("fact", path), "clips.toml")
        for gd in json.loads(guards or "[]") if isinstance(guards, str) else (guards or []):
            g.edge(c, "guards", g.node("mutant", gd), "clips.toml")
    addrs = [a for a, _ in starts]
    cov = json.loads(Path(coverage).read_text())
    for p in cov["pool"]:
        c = g.node("clip", p["id"])
        hit = set()
        for a in p["ids"] or []:
            if 0x8000 <= a < 0x10000:
                k = bisect.bisect_right(addrs, a) - 1
                if k >= 0:
                    hit.add(starts[k][1])
        for r in sorted(hit):
            g.edge(c, "runs", f"routine:{r}", "coverage.json")


def scores(g, export):
    import duckdb
    q = f"select model_hash, corpus_version, clip_id, exact from '{export}/data/scores.parquet'"
    for mh, cv, cid, exact in duckdb.sql(q).fetchall():
        s = g.node("score", f"{mh}/{cid}", exact=bool(exact))
        g.edge(s, "of_clip", g.node("clip", cid), "evidence/")
        g.edge(s, "under_model", g.node("model", mh), "evidence/")
        g.edge(s, "under_corpus", g.node("corpus", cv), "evidence/")


def attempts(g, game):
    out = subprocess.run(["git", "-C", str(game), "log", "--format=%h %s"],
                         capture_output=True, text=True, check=True).stdout
    for line in out.splitlines():
        sha, subj = line.split(" ", 1)
        m = re.match(r"Attempt (\d+) on (grid:[^:]+:[^:]+:v\d+@([a-z0-9_]+)\.[a-z0-9_.]+)", subj)
        if not m:
            continue
        a = g.node("attempt", m.group(1), commit=sha)
        cell = g.node("cell", m.group(2))
        g.edge(a, "on_cell", cell, "git log")
        g.edge(cell, "in_level", g.node("level", m.group(3)), "git log")
        kind, what = m.group(2).split(":")[1:3]
        if kind == "entity" and f"entity:{what}" in g.nodes:
            g.edge(cell, "about", f"entity:{what}", "git log")


def main():
    ap = argparse.ArgumentParser()
    for k in ("game", "export", "asm", "dbg", "coverage", "out"):
        ap.add_argument(f"--{k}", required=True)
    ap.add_argument("--source")
    o = ap.parse_args()
    game, out = Path(o.game), Path(o.out)
    source = o.source or tomllib.loads((game / "model.toml").read_text())["model"]["source"]
    out.mkdir(parents=True, exist_ok=True)
    g = G()
    syms = dbg_symbols(o.dbg)
    equ, starts = code(g, o.asm, syms)
    assets(g, game, source)
    mapping(g, game, equ, o.export)
    facts(g, o.export)
    clips(g, o.export, o.coverage, starts)
    scores(g, o.export)
    attempts(g, game)
    # Object ID nodes join the constants the code compares against.
    for nid in [n for n in g.nodes if n.startswith("objid:")]:
        val = int(nid.split(":")[1], 16)
        for cname, v in equ.items():
            if v == val and f"const:{cname}" in g.nodes:
                g.edge(nid, "named", f"const:{cname}", Path(o.asm).name)
    for s, _, d, _ in list(g.edges):
        for n in (s, d):
            if n not in g.nodes:
                k, name = n.split(":", 1)
                g.node(k, name)
    ids = sorted(g.nodes)
    nodes = pa.table({
        "id": ids,
        "kind": [g.nodes[i][0] for i in ids],
        "name": [g.nodes[i][1] for i in ids],
        "attrs": [g.nodes[i][2] for i in ids],
    })
    es = sorted(g.edges)
    edges = pa.table({
        "src": [e[0] for e in es], "rel": [e[1] for e in es],
        "dst": [e[2] for e in es], "via": [e[3] for e in es],
    })
    pq.write_table(nodes, out / "nodes.parquet", compression="zstd")
    pq.write_table(edges, out / "edges.parquet", compression="zstd")
    head = subprocess.run(["git", "-C", str(game), "rev-parse", "HEAD"],
                          capture_output=True, text=True).stdout.strip()
    manifest = {
        "format": "graph-0.0.1", "game": game.name, "source": source, "repo_commit": head,
        "inputs": {k: h(getattr(o, k)) for k in ("asm", "dbg", "coverage")}
        | {f"export/{t}": h(Path(o.export) / "data" / f"{t}.parquet")
           for t in ("facts", "mapping", "mechanics", "clips", "scores")}
        | {f: h(game / f) for f in (f"assets/{source}.toml", "model.toml",
                                    "mapping.toml", "sprites.toml")
           if (game / f).exists()},
        "nodes": len(ids), "edges": len(es),
        "outputs": {"nodes.parquet": h(out / "nodes.parquet"),
                    "edges.parquet": h(out / "edges.parquet")},
    }
    (out / "manifest.json").write_text(json.dumps(manifest, indent=1, sort_keys=True))
    print(json.dumps({"nodes": len(ids), "edges": len(es)}))


if __name__ == "__main__":
    main()
