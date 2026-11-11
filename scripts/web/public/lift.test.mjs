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

import { test } from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { fileURLToPath } from "node:url";

import { MemFs, runWasi } from "./wasi.js";

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, "../../..");
const out = process.env.GP_WEB_LIFT_OUT;
const skip = out ? false : "GP_WEB_LIFT_OUT isn't set (run scripts/web/build-lift.sh OUT first)";
const DK = "aa408f5a6b97c0d738e7e8b489a5617ad4a9ecdee2b05c4ee504210ce31b2825";
const OE = "45440d8ef512b4660dce0c753e110004aef96fa30f2967f9260e946d808db07d";

async function lift(rom) {
  const dir = path.join(out, "lift");
  const index = JSON.parse(fs.readFileSync(path.join(dir, "index.json"), "utf8"));
  const mem = new MemFs();
  mem.writeFile("/rom.nes", rom);
  for (const f of index.files) mem.writeFile(`/template/${f}`, fs.readFileSync(path.join(dir, f)));
  mem.mkdirp("/out");
  const module = new WebAssembly.Module(fs.readFileSync(path.join(dir, "gp-web-lift.wasm")));
  const t0 = performance.now();
  const run = await runWasi(module, ["gp-web-lift", "/rom.nes", "/template", "/out"], mem);
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
