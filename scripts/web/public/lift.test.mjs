// The public page's ROM lift under Node (node --test scripts/web/public/):
// gp-web-lift.wasm run by wasi.js over its in-memory file system, as the
// page runs it (docs/web.md, "The public page").
//
//   GP_WEB_LIFT_OUT  the folder scripts/web/build-lift.sh wrote (required;
//                    the tests skip without it)
//   GP_WEB_PREP      the native gp-web-prep, to compare with (optional)
//   GP_ROM_DIR       where the person's ROMs are; the Donkey Kong (JU) and
//                    Original Edition ROMs are found there by their hashes
//                    and read, never copied
//   GP_WORKER_OUT    where the comparison writes its files (else the
//                    system's temp folder)
//   GP_WEB_LIFT_ARCADE_OUT  a folder like GP_WEB_LIFT_OUT's for an arcade
//                    game (Tapper's: lift/ with gp-web-lift.wasm and the
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
const repo = path.resolve(here, "../../..");
const out = process.env.GP_WEB_LIFT_OUT;
const skip = out ? false : "GP_WEB_LIFT_OUT isn't set (run scripts/web/build-lift.sh OUT first)";
const DK = "aa408f5a6b97c0d738e7e8b489a5617ad4a9ecdee2b05c4ee504210ce31b2825";
const OE = "45440d8ef512b4660dce0c753e110004aef96fa30f2967f9260e946d808db07d";

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
  if (!dir || !fs.existsSync(dir)) return null;
  for (const name of fs.readdirSync(dir)) {
    const p = path.join(dir, name);
    if (!name.endsWith(".nes") || !fs.statSync(p).isFile()) continue;
    const b = fs.readFileSync(p);
    if (headerless(b) === sha) return b;
  }
  return null;
}

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

test("a ROM of no Donkey Kong is refused, naming the ones the page knows", { skip }, async () => {
  // A made-up NROM image: 16KB of PRG and 8KB of CHR counting up.
  const rom = new Uint8Array(16 + 16384 + 8192);
  rom.set([0x4e, 0x45, 0x53, 0x1a, 1, 1]);
  for (let i = 16; i < rom.length; i++) rom[i] = (i * 7) & 0xff;
  const run = await lift(rom);
  assert.equal(run.code, 3, run.stderr);
  assert.equal(run.result.ok, false);
  assert.match(run.result.error, /doesn't look like a Donkey Kong: Original Edition ROM/);
  assert.deepEqual(run.result.expected.map((e) => e.id), ["donkey-kong-oe", "donkey-kong-ju"]);
  assert.equal([...run.mem.files.keys()].filter((f) => f.startsWith("/out/")).length, 1, "only result.json");
});

test("a file that isn't a ROM is refused", { skip }, async () => {
  const run = await lift(new TextEncoder().encode("PK\x03\x04 not a rom"));
  assert.equal(run.code, 3, run.stderr);
  assert.match(run.result.error, /isn't an NES ROM/);
});

const rom = out ? findRom(DK) : null;
test(
  "the Donkey Kong (JU) ROM plays JU's game on Original Edition's model through its own asset map, without 50m and the stage-clear scene",
  { skip: skip || (rom ? false : "the Donkey Kong (JU) ROM isn't in $GP_ROM_DIR/nes") },
  async () => {
    const run = await lift(rom);
    assert.equal(run.code, 0, run.stderr);
    assert.equal(run.result.ok, true);
    assert.equal(run.result.sha256, DK);
    for (const f of ["bundle.json", "preview.json", "profiles.toml", "assets/present.txt", "assets/sound.json"]) {
      assert.ok(run.result.files.includes(f), `${f} is written`);
    }
    const f = run.result.found;
    assert.equal(f.template, "donkey-kong-ju", "lifted through JU's own asset map (templates/donkey-kong-oe [[loads]])");
    assert.equal(f.plays, "donkey-kong-oe");
    assert.equal(f.match, "exact");
    const by = (k) => f.assets.filter((a) => a.from === k).map((a) => a.name);
    assert.ok(by("same").includes("stage_screens") && by("same").includes("tiles") && by("same").includes("sound_jingles"));
    // What JU's game hasn't (template.toml's [[loads.without]]): 50m's
    // assets and the stage-clear scene's are left blank, and nothing is
    // filled from anywhere.
    for (const n of ["stage_screens_50m", "ground_runs_50m", "ladders_50m", "scene_sprite_tiles", "palette_scene"]) {
      assert.ok(by("blank").includes(n), `${n} is left blank: ${JSON.stringify(by("blank"))}`);
    }
    for (const a of f.assets) assert.ok(["same", "blank"].includes(a.from), `${a.name}: ${a.from}`);
    for (const a of f.assets.filter((a) => a.from === "blank")) assert.ok(["50m", "stage-clear scene"].includes(a.without), `${a.name}: without ${a.without}`);
    const preview = JSON.parse(new TextDecoder().decode(run.mem.readFile("/out/preview.json")));
    assert.equal(preview.model, "donkey_kong_oe");
    assert.ok(!preview.levels.includes("m50"), "the model has no level m50 for JU");
    // The model's text in the bundle: its stages list keeps m50 in the
    // game's order and skips it (stages.skip), so 25m, 75m, 100m; the
    // stage-clear scene's rule and kinds are gone.
    const bundle = new TextDecoder().decode(run.mem.readFile("/out/bundle.json"));
    const order = bundle.match(/\[stages\]\\norder = \[([^\]]*)\]/)?.[1] ?? "";
    const skip = bundle.match(/\\nskip = \[([^\]]*)\]/)?.[1] ?? "";
    assert.ok(order.includes("m75") && skip.includes("m50") && !skip.includes("m75"), `the stages list skips 50m and plays 75m: order ${order}, skip ${skip}`);
    for (const t of ["[rules.clear_scene]", "[entities.kong_carry]", "[entities.pauline_carry]"]) assert.ok(!bundle.includes(t), `${t} is left out`);
    console.log(`lift JU: ${Math.round(run.ms)} ms, ${run.result.files.length} files, ${by("blank").length} left blank (${by("blank").join(", ")})`);
  },
);

const oe = out ? findRom(OE) : null;
test(
  "Original Edition loads its own model with every asset found",
  { skip: skip || (oe ? false : "the Original Edition ROM isn't in $GP_ROM_DIR/nes") },
  async () => {
    const run = await lift(oe);
    assert.equal(run.code, 0, run.stderr);
    const f = run.result.found;
    assert.equal(f.template, "donkey-kong-oe");
    assert.equal(f.match, "exact");
    assert.ok(f.assets.length >= 21 && f.assets.every((a) => a.from === "same"), JSON.stringify(f.assets));
    for (const n of ["stage_screens", "ground_runs_25m", "ladders_25m", "palettes_25m", "sprite_tiles", "tiles"]) {
      assert.ok(f.assets.some((a) => a.name === n), `25m's ${n} is found`);
    }
    const ju = f.compared.find((c) => c.template === "donkey-kong-ju");
    assert.ok(ju.program_share > 0.9, `JU's program is there (${ju.program_share})`);
    console.log(`lift OE: ${Math.round(run.ms)} ms`);
    const prep = process.env.GP_WEB_PREP;
    if (!prep) return;
    // The native gp-web-prep on the same lifted files writes the same bytes.
    const work = fs.mkdtempSync(path.join(process.env.GP_WORKER_OUT || os.tmpdir(), "lift-test-"));
    for (const [p, b] of run.mem.files) {
      if (p.startsWith("/out/lifted/")) {
        const to = path.join(work, "lifted", p.slice("/out/lifted/".length));
        fs.mkdirSync(path.dirname(to), { recursive: true });
        fs.writeFileSync(to, b);
      }
    }
    execFileSync(prep, [path.join(repo, "templates/donkey-kong-oe"), path.join(work, "lifted"), path.join(work, "prep")], { stdio: "pipe" });
    for (const f of run.result.files) {
      const native = fs.readFileSync(path.join(work, "prep", f));
      assert.ok(Buffer.from(run.mem.readFile(`/out/${f}`)).equals(native), `${f} is the same as gp-web-prep's`);
    }
  },
);

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
// made on one ROM's pictures. JU plays Original Edition's model without
// 50m and the stage-clear scene, so its sprite frames and palette sets
// must be numbered as Original Edition's are, or a theme draws another
// thing in their place (issue 13: a ladder piece for the hero, a carrying
// pose for the lady). So both lifts name the same sprite frames and sets,
// and a picture JU draws as Original Edition does under one name isn't
// JU's under another (left-out kinds' frames are blank in JU; a blank
// picture both have under one name doesn't count). The pictures' bytes
// are only compared here, never kept.
test(
  "Donkey Kong (JU) names its pictures as Original Edition does, so one theme fits both",
  { skip: skip || (rom && oe ? false : "the Donkey Kong (JU) and Original Edition ROMs aren't both in $GP_ROM_DIR/nes") },
  async () => {
    const [a, b] = [pictures(await lift(oe)), pictures(await lift(rom))];
    const sprites = (m) => [...m.keys()].filter((n) => /^(frame|hud)-/.test(n)).sort();
    assert.deepEqual(sprites(b), sprites(a), "the same sprite frames and HUD sets, by name");
    const named = new Map();
    for (const [n, h] of a) named.set(h, [...(named.get(h) ?? []), n]);
    const moved = [];
    for (const [n, h] of b) {
      if (a.get(n) === h) continue;
      const elsewhere = (named.get(h) ?? []).filter((x) => b.get(x) !== h);
      if (elsewhere.length) moved.push(`${n} (Original Edition's ${elsewhere.join(", ")})`);
    }
    assert.deepEqual(moved, [], "a picture JU draws as Original Edition does has the same name in both");
    const same = [...b].filter(([n, h]) => a.get(n) === h).length;
    console.log(`JU's pictures by Original Edition's names: ${same} of ${b.size} the same picture`);
  },
);

test(
  "a Donkey Kong (JU) dump with its sprite tiles gone is refused, as nothing fills them",
  { skip: skip || (rom ? false : "the Donkey Kong (JU) ROM isn't in $GP_ROM_DIR/nes") },
  async () => {
    // In memory only: another header byte (so another file, the same
    // program) and the first pattern table's bytes turned over.
    const b = Uint8Array.from(rom);
    b[7] ^= 0x10;
    const chr = 16 + 16384;
    for (let i = 0; i < 4096; i++) b[chr + i] ^= 0xa5;
    const run = await lift(b);
    assert.equal(run.code, 2, run.stderr);
    assert.equal(run.result.ok, false);
    assert.match(run.result.error, /sprite_tiles \(slot sprite_tiles\) isn't in this ROM/);
  },
);

test("a 1 MB ROM of no Donkey Kong is searched and refused in well under a second", { skip }, async () => {
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
