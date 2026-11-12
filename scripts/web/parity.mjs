// The web build against the native interpreter, frame by frame
// (docs/web.md, "Parity"): gp_web.wasm from a preview build plays each
// input the native run played (scripts/web/parity.sh, through
// gp-web-prep), and every frame's state hash, the spec digest and the
// hooks' label must be the native run's. The module runs in Node, the
// same bytes the page runs.
//   node scripts/web/parity.mjs --dist OUT --runs DIR
// DIR holds, per input NAME: NAME.frames (the input), NAME.hashes (the
// native hashes, power-on first), NAME.json (the native run's
// spec_digest and ext_hooks) and, NAME.level, the level a run started at,
// empty for the model's own start, which the bundle gets as the page's
// `level=` gives it (parity.sh writes it empty: the native run can't
// start at another level now). index.txt, when there, lists the names.
// OUT is a build of scripts/web/build-preview.sh: gp_web.wasm, bundle.json.
// Prints one line per input and a JSON summary last.
// Exit codes: 0 every frame agreed, 1 a hash, digest or label differs,
// 2 couldn't run.
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join, resolve } from "node:path";
import { withLevel } from "./page/level.js";

const arg = (n) => {
  const i = process.argv.indexOf(`--${n}`);
  if (i < 0 || !process.argv[i + 1]) {
    console.error(`parity: give --${n}`);
    process.exit(2);
  }
  return resolve(process.argv[i + 1]);
};
const dist = arg("dist");
const runs = arg("runs");

let x;
try {
  x = new WebAssembly.Instance(new WebAssembly.Module(readFileSync(join(dist, "gp_web.wasm"))), {}).exports;
} catch (e) {
  console.error(`parity: ${join(dist, "gp_web.wasm")}: ${e.message}`);
  process.exit(2);
}
const mem = () => new Uint8Array(x.memory.buffer);
const put = (bytes) => {
  const p = x.gp_alloc(bytes.length);
  mem().set(bytes, p);
  return p;
};
const out = () => new TextDecoder().decode(mem().slice(x.gp_out_ptr(), x.gp_out_ptr() + x.gp_out_len()));
const check = (code) => {
  if (code !== 0) throw new Error(out());
  return out();
};

const bundle = readFileSync(join(dist, "bundle.json"));
// index.txt (scripts/web/parity.sh writes it) names the inputs; without
// it, every .frames file is one.
const index = join(runs, "index.txt");
const names = existsSync(index)
  ? readFileSync(index, "utf8").split("\n").filter(Boolean)
  : readdirSync(runs)
      .filter((f) => f.endsWith(".frames"))
      .map((f) => f.slice(0, -".frames".length))
      .sort();
if (names.length === 0) {
  console.error(`parity: no .frames files in ${runs}`);
  process.exit(2);
}
const results = [];
for (const name of names) {
  const native = JSON.parse(readFileSync(join(runs, `${name}.json`), "utf8"));
  const want = readFileSync(join(runs, `${name}.hashes`), "utf8").trim().split("\n");
  const levelFile = join(runs, `${name}.level`);
  const level = existsSync(levelFile) ? readFileSync(levelFile, "utf8").trim() : "";
  const b = level ? withLevel(bundle, level) : bundle;
  let hooks;
  try {
    hooks = check(x.gp_load(put(b), b.length));
  } catch (e) {
    console.log(`${name}: gp_web.wasm refused the model: ${e.message}`);
    results.push({ name, ok: false, error: e.message });
    continue;
  }
  const digest = check(x.gp_spec_digest());
  const frames = new TextEncoder().encode(readFileSync(join(runs, `${name}.frames`), "utf8"));
  const got = check(x.gp_run_frames(put(frames), frames.length)).trim().split("\n");
  const first = want.findIndex((h, i) => got[i] !== h);
  const r = {
    name,
    frames: want.length - 1,
    hooks,
    hooks_ok: hooks === native.ext_hooks,
    digest_ok: digest === native.spec_digest,
    lengths_ok: got.length === want.length,
    // The first state that differs: 0 is power-on, N the state after frame N.
    first_difference: first < 0 ? null : first,
  };
  r.ok = r.hooks_ok && r.digest_ok && r.lengths_ok && r.first_difference === null;
  results.push(r);
  const why = [
    r.hooks_ok ? "" : `hooks "${hooks}" against native "${native.ext_hooks}"`,
    r.digest_ok ? "" : `spec digest ${digest} against native ${native.spec_digest}`,
    r.lengths_ok ? "" : `${got.length} hashes against ${want.length}`,
    r.first_difference === null ? "" : `state ${first} differs`,
  ].filter(Boolean);
  console.log(`${name}: ${r.frames} frames, ${r.ok ? "every state hash agrees" : why.join("; ")}`);
}
const ok = results.every((r) => r.ok);
console.log(JSON.stringify({ ok, runs: results }));
process.exit(ok ? 0 : 1);
