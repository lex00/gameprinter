// The public page's ROM lift under Node (node --test scripts/web/public/):
// gp-web-lift.wasm run by wasi.js over its in-memory file system, as the
// page runs it (docs/web.md, "The public page").
//
//   GP_WEB_LIFT_OUT  the folder scripts/web/build-lift.sh wrote (required;
//                    the tests skip without it)
//   GP_WEB_PREP      the native gp-web-prep, to compare with (optional)
//   GP_ROM_DIR       where the person's ROMs are; the ROMs of the lift
//                    folder's templates (its sources-nes.toml) are found
//                    there by their hashes and read, never copied
//   GP_WORKER_OUT    where the comparison writes its files (else the
//                    system's temp folder)
//   GP_WEB_LIFT_ARCADE_OUT  a folder like GP_WEB_LIFT_OUT's for an arcade
//                    game (lift/ with gp-web-lift.wasm and the
//                    play/lift/ files build-page.sh writes for it); the
//                    arcade tests skip without it. They lift made-up zips
//                    only: a set is known by its chips' real hashes, so
//                    what a made-up zip can show is the refusals.

import { test } from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { fileURLToPath } from "node:url";

import { MemFs, runWasi } from "./wasi.js";
import { themesInOrder } from "./lift.js";

const here = path.dirname(fileURLToPath(import.meta.url));
const out = process.env.GP_WEB_LIFT_OUT;
const skip = out ? false : "GP_WEB_LIFT_OUT isn't set (run scripts/web/build-lift.sh OUT first)";

// The game the lift folder is for, from the folder itself: game.json's
// title and templates, sources-nes.toml's hashes and titles, and each
// template's load.json when it only loads to play another's model. The
// tests name no game: what they hold a lift to is the folder's own.
const liftDir = out ? path.join(out, "lift") : null;
const gameJson = out ? JSON.parse(fs.readFileSync(path.join(liftDir, "game.json"), "utf8")) : { templates: [] };
// The [[source]] tables of sources-nes.toml, as {id, title, sha256}: the
// fields are one key = "string" a line.
function sources() {
  if (!out) return [];
  const list = [];
  for (const line of fs.readFileSync(path.join(liftDir, "sources-nes.toml"), "utf8").split("\n")) {
    if (line.trim() === "[[source]]") list.push({});
    const m = /^(id|title|sha256) = "([^"]*)"$/.exec(line.trim());
    if (m && list.length) list.at(-1)[m[1]] = m[2];
  }
  return list;
}
const SOURCES = sources();
const loadOf = (t) => {
  const f = out && path.join(liftDir, t, "load.json");
  return f && fs.existsSync(f) ? JSON.parse(fs.readFileSync(f, "utf8")) : null;
};
// A template's model name (model.toml's [model] name).
const modelName = (t) => /^\[model\][^[]*?^name = "([^"]+)"/ms.exec(fs.readFileSync(path.join(liftDir, t, "model.toml"), "utf8"))?.[1] ?? null;
const TEMPLATES = gameJson.templates.map((id) => ({ id, source: SOURCES.find((s) => s.id === id) ?? null, load: loadOf(id) }));
const owners = TEMPLATES.filter((t) => !t.load);
const loaders = TEMPLATES.filter((t) => t.load);

async function lift(rom, from = out, romPath = "/rom.nes") {
  const dir = path.join(from, "lift");
  const index = JSON.parse(fs.readFileSync(path.join(dir, "index.json"), "utf8"));
  const mem = new MemFs();
  mem.writeFile(romPath, rom);
  for (const f of index.files) mem.writeFile(`/template/${f}`, fs.readFileSync(path.join(dir, f)));
  mem.mkdirp("/out");
  const module = new WebAssembly.Module(fs.readFileSync(path.join(dir, "gp-web-lift.wasm")));
  const t0 = performance.now();
  const run = await runWasi(module, ["gp-web-lift", romPath, "/template", "/out"], mem);
  run.ms = performance.now() - t0;
  run.result = JSON.parse(new TextDecoder().decode(mem.readFile("/out/result.json")));
  run.mem = mem;
  return run;
}

// The headerless SHA-256, as sources/nes.toml keys ROMs.
function headerless(bytes) {
  if (bytes.length < 16 || bytes.toString("latin1", 0, 4) !== "NES\x1a") return null;
  const start = 16 + (bytes[6] & 4 ? 512 : 0);
  return createHash("sha256").update(bytes.subarray(start)).digest("hex");
}

function findRom(sha) {
  const dir = process.env.GP_ROM_DIR && path.join(process.env.GP_ROM_DIR, "nes");
  if (!sha || !dir || !fs.existsSync(dir)) return null;
  for (const name of fs.readdirSync(dir)) {
    const p = path.join(dir, name);
    if (!name.endsWith(".nes") || !fs.statSync(p).isFile()) continue;
    const b = fs.readFileSync(p);
    if (headerless(b) === sha) return b;
  }
  return null;
}
for (const t of TEMPLATES) t.rom = findRom(t.source?.sha256);
const romSkip = (t) => skip || (t?.rom ? false : `the ${t?.source?.title ?? "game's"} ROM ${t?.id ?? ""} isn't in $GP_ROM_DIR/nes`);

// The themes the page gives the lift are listed in Settings, Theme in the
// order given (page.js puts the game's own namespace first), not in the
// lift's folder-name order; a theme the ROM's game ships keeps its place.
test("the themes given are listed in the order given", () => {
  const index = JSON.stringify({
    themes: [
      { id: "clay", title: "Clay" },
      { id: "harbor-lights", title: "Harbor Lights" },
      { id: "turnips", title: "Turnips" },
    ],
  });
  const got = JSON.parse(themesInOrder(index, ["turnips", "harbor-lights"]));
  assert.deepEqual(got.themes.map((t) => t.id), ["clay", "turnips", "harbor-lights"]);
});

test("a ROM of no game of the folder is refused, naming the ones the page knows", { skip }, async () => {
  // A made-up NROM image: 16KB of PRG and 8KB of CHR counting up.
  const rom = new Uint8Array(16 + 16384 + 8192);
  rom.set([0x4e, 0x45, 0x53, 0x1a, 1, 1]);
  for (let i = 16; i < rom.length; i++) rom[i] = (i * 7) & 0xff;
  const run = await lift(rom);
  assert.equal(run.code, 3, run.stderr);
  assert.equal(run.result.ok, false);
  assert.ok(run.result.error.includes(`doesn't look like a ${gameJson.title} ROM`), run.result.error);
  assert.deepEqual(run.result.expected.map((e) => e.id), gameJson.templates);
  assert.equal([...run.mem.files.keys()].filter((f) => f.startsWith("/out/")).length, 1, "only result.json");
});

test("a file that isn't a ROM is refused", { skip }, async () => {
  const run = await lift(new TextEncoder().encode("PK\x03\x04 not a rom"));
  assert.equal(run.code, 3, run.stderr);
  assert.match(run.result.error, /isn't an NES ROM/);
});

// Each template that only loads to play another's model (its load.json):
// its ROM plays its game on that model through its own asset map, without
// what its game hasn't ([[loads.without]]: those levels and parts).
for (const t of loaders.length ? loaders : [null]) {
  test(
    `a ROM a template only loads to play plays its game on the model it names, through its own asset map, without what its game hasn't${t ? ` (${t.id})` : ""}`,
    { skip: t ? romSkip(t) : skip || "the lift folder has no template that only loads to play" },
    async () => {
      const load = t.load;
      const run = await lift(t.rom);
      assert.equal(run.code, 0, run.stderr);
      assert.equal(run.result.ok, true);
      assert.equal(run.result.sha256, t.source.sha256);
      for (const f of ["bundle.json", "preview.json", "profiles.toml", "assets/present.txt", "assets/sound.json"]) {
        assert.ok(run.result.files.includes(f), `${f} is written`);
      }
      const f = run.result.found;
      assert.equal(f.template, t.id, "lifted through its own asset map (the model's template's [[loads]])");
      assert.equal(f.plays, load.plays);
      assert.equal(f.match, "exact");
      const by = (k) => f.assets.filter((a) => a.from === k).map((a) => a.name);
      assert.ok(by("same").length > 0, "assets are found");
      // What its game hasn't: those assets are left blank, and nothing is
      // filled from anywhere.
      const whats = load.without.map((w) => w.what);
      for (const w of load.without) for (const n of w.blank ?? []) assert.ok(by("blank").includes(n), `${n} (${w.what}) is left blank: ${JSON.stringify(by("blank"))}`);
      for (const a of f.assets) assert.ok(["same", "blank"].includes(a.from), `${a.name}: ${a.from}`);
      for (const a of f.assets.filter((a) => a.from === "blank")) assert.ok(whats.includes(a.without), `${a.name}: without ${a.without}`);
      const preview = JSON.parse(new TextDecoder().decode(run.mem.readFile("/out/preview.json")));
      assert.equal(preview.model, modelName(load.plays));
      const gone = load.without.flatMap((w) => w.levels ?? []);
      for (const l of gone) assert.ok(!preview.levels.includes(l), `the model has no level ${l} for it`);
      // The model's text in the bundle: its stages list keeps the levels it
      // hasn't in the game's order and skips them (stages.skip), and plays
      // the rest; the parts it hasn't are gone.
      const bundle = new TextDecoder().decode(run.mem.readFile("/out/bundle.json"));
      // (A JSON string: its quotes come escaped.)
      const order = (bundle.match(/\[stages\]\\norder = \[([^\]]*)\]/)?.[1] ?? "").replaceAll('\\"', '"');
      const skipped = (bundle.match(/\\nskip = \[([^\]]*)\]/)?.[1] ?? "").replaceAll('\\"', '"');
      for (const l of load.levels) assert.ok(order.includes(`"${l}"`) && !skipped.includes(`"${l}"`), `the stages list plays ${l}: order ${order}, skip ${skipped}`);
      for (const l of gone) assert.ok(skipped.includes(`"${l}"`), `the stages list skips ${l}: order ${order}, skip ${skipped}`);
      for (const part of load.without.flatMap((w) => w.parts ?? [])) {
        for (const tbl of [`[rules.${part}]`, `[entities.${part}]`]) assert.ok(!bundle.includes(tbl), `${tbl} is left out`);
      }
      console.log(`lift ${t.id}: ${Math.round(run.ms)} ms, ${run.result.files.length} files, ${by("blank").length} left blank (${by("blank").join(", ")})`);
    },
  );
}

// Each template with a model of its own: its ROM loads it with every asset
// of its asset map found, and each template that loads to play it shares
// its program.
for (const t of owners) {
  test(`a ROM loads its own template's model with every asset found (${t.id})`, { skip: romSkip(t) }, async () => {
    const run = await lift(t.rom);
    assert.equal(run.code, 0, run.stderr);
    const f = run.result.found;
    assert.equal(f.template, t.id);
    assert.equal(f.match, "exact");
    assert.ok(f.assets.length > 0 && f.assets.every((a) => a.from === "same"), JSON.stringify(f.assets));
    for (const l of loaders.filter((l) => l.load.plays === t.id)) {
      const c = f.compared.find((c) => c.template === l.id);
      assert.ok(c?.program_share > 0.9, `${l.id}'s program is there (${c?.program_share})`);
    }
    console.log(`lift ${t.id}: ${Math.round(run.ms)} ms`);
    const prep = process.env.GP_WEB_PREP;
    if (!prep) return;
    // The native gp-web-prep on the same lifted files writes the same bytes
    // (GP_WEB_PREP_TEMPLATE: the template's folder it reads).
    const tdir = process.env.GP_WEB_PREP_TEMPLATE;
    if (!tdir) return console.log("GP_WEB_PREP is set without GP_WEB_PREP_TEMPLATE (the template's folder), so gp-web-prep isn't compared");
    const work = fs.mkdtempSync(path.join(process.env.GP_WORKER_OUT || os.tmpdir(), "lift-test-"));
    for (const [p, b] of run.mem.files) {
      if (p.startsWith("/out/lifted/")) {
        const to = path.join(work, "lifted", p.slice("/out/lifted/".length));
        fs.mkdirSync(path.dirname(to), { recursive: true });
        fs.writeFileSync(to, b);
      }
    }
    execFileSync(prep, [tdir, path.join(work, "lifted"), path.join(work, "prep")], { stdio: "pipe" });
    for (const f of run.result.files) {
      const native = fs.readFileSync(path.join(work, "prep", f));
      assert.ok(Buffer.from(run.mem.readFile(`/out/${f}`)).equals(native), `${f} is the same as gp-web-prep's`);
    }
  });
}

// The lifted pictures (assets/*.png) of a run, as name -> SHA-256.
function pictures(run) {
  const out = new Map();
  for (const f of run.result.files) {
    const m = /^assets\/([^/]+\.png)$/.exec(f);
    if (m) out.set(m[1], createHash("sha256").update(run.mem.readFile(`/out/${f}`)).digest("hex"));
  }
  return out;
}

// A large-art look (a theme's look.txt and art.txt) replaces the game's
// pictures by name (frame-N-S.png, hud-S.png, glyph-S-...), and a theme is
// made on one ROM's pictures. A ROM that loads to play another's model
// without some of its parts must number its sprite frames and palette sets
// as the model's own ROM does, or a theme draws another thing in their
// place (issue 13: a ladder piece for the hero, a carrying pose for the
// lady). So both lifts name the same sprite frames and sets, and a picture
// the loader draws as the model's ROM does under one name isn't the
// loader's under another (left-out kinds' frames are blank in the loader; a
// blank picture both have under one name doesn't count). The pictures'
// bytes are only compared here, never kept.
for (const l of loaders.length ? loaders : [null]) {
  const o = l && TEMPLATES.find((t) => t.id === l.load.plays);
  test(
    `a ROM that loads to play names its pictures as the model's own ROM does, so one theme fits both${l ? ` (${l.id} and ${o?.id})` : ""}`,
    { skip: !l ? skip || "the lift folder has no template that only loads to play" : romSkip(l) || romSkip(o) },
    async () => {
      const [a, b] = [pictures(await lift(o.rom)), pictures(await lift(l.rom))];
      const sprites = (m) => [...m.keys()].filter((n) => /^(frame|hud)-/.test(n)).sort();
      assert.deepEqual(sprites(b), sprites(a), "the same sprite frames and HUD sets, by name");
      const named = new Map();
      for (const [n, h] of a) named.set(h, [...(named.get(h) ?? []), n]);
      const moved = [];
      for (const [n, h] of b) {
        if (a.get(n) === h) continue;
        const elsewhere = (named.get(h) ?? []).filter((x) => b.get(x) !== h);
        if (elsewhere.length) moved.push(`${n} (${o.id}'s ${elsewhere.join(", ")})`);
      }
      assert.deepEqual(moved, [], `a picture ${l.id} draws as ${o.id} does has the same name in both`);
      const same = [...b].filter(([n, h]) => a.get(n) === h).length;
      console.log(`${l.id}'s pictures by ${o.id}'s names: ${same} of ${b.size} the same picture`);
    },
  );
}

// A dump with its first pattern table's bytes turned over: the lift fills
// nothing, so it refuses the dump naming the slot it can't find.
const turned = loaders.find((t) => t.rom) ?? TEMPLATES.find((t) => t.rom) ?? loaders[0] ?? TEMPLATES[0] ?? null;
test(
  `a dump with its sprite tiles gone is refused, as nothing fills them${turned ? ` (${turned.id})` : ""}`,
  { skip: romSkip(turned) },
  async () => {
    // In memory only: another header byte (so another file, the same
    // program) and the first pattern table's bytes turned over.
    const b = Uint8Array.from(turned.rom);
    b[7] ^= 0x10;
    const chr = 16 + (b[6] & 4 ? 512 : 0) + b[4] * 16384;
    for (let i = 0; i < 4096; i++) b[chr + i] ^= 0xa5;
    const run = await lift(b);
    assert.equal(run.code, 2, run.stderr);
    assert.equal(run.result.ok, false);
    assert.match(run.result.error, /\(slot [a-z0-9_]+\) isn't in this ROM/);
  },
);

test("a 1 MB ROM of no game of the folder is searched and refused in well under a second", { skip }, async () => {
  const n = 16 + 512 * 1024 + 512 * 1024;
  const big = new Uint8Array(n);
  big.set([0x4e, 0x45, 0x53, 0x1a, 32, 64, 0x10]);
  let x = 0x2468ace1;
  for (let i = 16; i < n; i++) {
    x ^= x << 13;
    x ^= x >>> 17;
    x ^= x << 5;
    big[i] = x >>> 24;
  }
  const run = await lift(big);
  assert.equal(run.code, 3, run.stderr);
  console.log(`1 MB search: ${Math.round(run.ms)} ms`);
  assert.ok(run.ms < 1000, `${run.ms} ms`);
});

test(
  "the game's templates ship no theme of their own: the page's themes come from packages",
  { skip },
  async () => {
    const index = JSON.parse(fs.readFileSync(path.join(out, "lift", "index.json"), "utf8"));
    assert.deepEqual(index.files.filter((f) => f.includes("/themes/")), []);
  },
);

// ---- An arcade game's lift: the refusals a made-up zip can show ----

const arcadeOut = process.env.GP_WEB_LIFT_ARCADE_OUT;
const arcadeSkip = arcadeOut ? false : "GP_WEB_LIFT_ARCADE_OUT isn't set (an arcade game's lift folder, as build-page.sh writes it)";
const liftZip = (bytes) => lift(bytes, arcadeOut, "/rom.zip");
const onlyResult = (run) => [...run.mem.files.keys()].filter((f) => f.startsWith("/out/")).length === 1;
// A zip of stored entries, made here from made-up bytes.
function madeUpZip(files) {
  const table = Array.from({ length: 256 }, (_, n) => {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    return c >>> 0;
  });
  const crc = (d) => {
    let c = 0xffffffff;
    for (const x of d) c = table[(c ^ x) & 0xff] ^ (c >>> 8);
    return (c ^ 0xffffffff) >>> 0;
  };
  const locals = [];
  const centrals = [];
  let o = 0;
  for (const [name, data] of files) {
    const n = Buffer.from(name, "latin1");
    const h = Buffer.alloc(30);
    h.writeUInt32LE(0x04034b50, 0);
    h.writeUInt16LE(20, 4);
    h.writeUInt32LE(crc(data), 14);
    h.writeUInt32LE(data.length, 18);
    h.writeUInt32LE(data.length, 22);
    h.writeUInt16LE(n.length, 26);
    const c = Buffer.alloc(46);
    c.writeUInt32LE(0x02014b50, 0);
    c.writeUInt16LE(20, 4);
    c.writeUInt16LE(20, 6);
    c.writeUInt32LE(crc(data), 16);
    c.writeUInt32LE(data.length, 20);
    c.writeUInt32LE(data.length, 24);
    c.writeUInt16LE(n.length, 28);
    c.writeUInt32LE(o, 42);
    locals.push(h, n, data);
    centrals.push(c, n);
    o += 30 + n.length + data.length;
  }
  const cd = Buffer.concat(centrals);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(files.length, 8);
  end.writeUInt16LE(files.length, 10);
  end.writeUInt32LE(cd.length, 12);
  end.writeUInt32LE(o, 16);
  return Buffer.concat([...locals, cd, end]);
}
const arcadeTitle = () => JSON.parse(fs.readFileSync(path.join(arcadeOut, "lift", "game.json"), "utf8")).title;

test("an arcade game: a file that isn't a zip is refused, naming the game and the sets it takes", { skip: arcadeSkip }, async () => {
  const run = await liftZip(new TextEncoder().encode("NES\x1a not a romset"));
  assert.equal(run.code, 3, run.stderr);
  assert.equal(run.result.ok, false);
  const title = arcadeTitle();
  assert.match(run.result.error, new RegExp(`^This file isn't a zip\\. This page takes a MAME romset zip of ${title}: [a-z0-9]+(, [a-z0-9]+)*( or [a-z0-9]+)?( \\([a-z0-9, ]+ (isn't|aren't) drawn right yet\\))?\\.$`));
  assert.ok(run.result.expected.length >= 1, "the sets it takes are listed");
  assert.ok(onlyResult(run), "only result.json");
});

test("an arcade game: a zip of made-up files is refused as holding no chip of the game", { skip: arcadeSkip }, async () => {
  const bytes = Buffer.from(Array.from({ length: 4096 }, (_, i) => (i * 131) & 0xff));
  const run = await liftZip(madeUpZip([["readme.txt", Buffer.from("made up")], ["made-up.bin", bytes]]));
  assert.equal(run.code, 3, run.stderr);
  assert.match(run.result.error, new RegExp(`^This zip holds no chip of a ${arcadeTitle()} romset\\.`));
  assert.ok(onlyResult(run), "only result.json");
});

test("an arcade game: a zip that can't be read is refused", { skip: arcadeSkip }, async () => {
  const z = madeUpZip([["a.bin", Buffer.alloc(16, 1)]]);
  // The central directory's entry no longer starts as one.
  z[z.indexOf(Buffer.from("PK\x01\x02", "latin1")) + 2] = 9;
  const run = await liftZip(z);
  assert.equal(run.code, 3, run.stderr);
  assert.match(run.result.error, /^This zip can't be read/);
  assert.ok(onlyResult(run), "only result.json");
});
