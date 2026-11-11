// Plays a ROM in gp_play.wasm (crates/gp-play) the way the mechanic page
// does, frame by frame with scripted buttons, and writes the input log
// and the frame hash digest it got. `just play-web` then replays the log
// natively (crates/gp-play/examples/digest.rs) and checks the digests match.
// On the way it checks free play's save points in the module: the state
// saved at frame 600, restored after the last frame and played on with the
// same buttons, reaches the same digest.
//   node scripts/web/check-play.mjs WASM ROM OUT_LOG
// Prints the digest. Exit codes: 0 played, 2 couldn't.
import fs from "node:fs";

const [wasm, romPath, outLog] = process.argv.slice(2);
if (!outLog) {
  console.error("usage: node scripts/web/check-play.mjs WASM ROM OUT_LOG");
  process.exit(2);
}
try {
  const module = new WebAssembly.Module(fs.readFileSync(wasm));
  // tetanes-core's web imports, which gp-nes never calls.
  const imports = {};
  for (const i of WebAssembly.Module.imports(module)) {
    (imports[i.module] ??= {})[i.name] = () => {
      throw new Error(`called ${i.module}.${i.name}`);
    };
  }
  const x = new WebAssembly.Instance(module, imports).exports;
  const out = () => new TextDecoder().decode(new Uint8Array(x.memory.buffer, x.gp_out_ptr(), x.gp_out_len()));
  const rom = fs.readFileSync(romPath);
  const p = x.gp_alloc(rom.length);
  new Uint8Array(x.memory.buffer, p, rom.length).set(rom);
  if (x.gp_play_load(0, p, rom.length)) throw new Error(out());
  // Start, then run right with a jump every second and a half, drawing
  // every frame as the panel does.
  const t0 = performance.now();
  const pad = (i) => (i >= 30 && i < 36 ? 0x08 : 0) | (i > 120 ? 0x80 | (i % 90 < 20 ? 0x01 : 0) : 0);
  let saved;
  for (let i = 0; i < 1200; i++) {
    if (i === 600) {
      const at = x.gp_play_save(0);
      if (!at) throw new Error(out());
      saved = new Uint8Array(x.memory.buffer, at, x.gp_out_len()).slice();
    }
    if (x.gp_play_step(0, pad(i), 0) < 0) throw new Error(out());
    x.gp_play_rgba(0);
  }
  const ms = performance.now() - t0;
  if (x.gp_play_digest(0)) throw new Error(out());
  const first = out();
  // Back to the save point, and the same buttons again.
  const q = x.gp_alloc(saved.length);
  new Uint8Array(x.memory.buffer, q, saved.length).set(saved);
  if (x.gp_play_restore(0, q, saved.length)) throw new Error(out());
  if (x.gp_play_frame(0) !== 600) throw new Error(`restored to frame ${x.gp_play_frame(0)}, not 600`);
  for (let i = 600; i < 1200; i++) if (x.gp_play_step(0, pad(i), 0) < 0) throw new Error(out());
  if (x.gp_play_digest(0)) throw new Error(out());
  if (out() !== first) throw new Error(`restored at frame 600 and played on, the digest is ${out()}, not ${first}`);
  if (x.gp_play_input_log(0)) throw new Error(out());
  fs.writeFileSync(outLog, out());
  if (x.gp_play_digest(0)) throw new Error(out());
  console.log(out());
  console.error(`played 1200 frames in ${ms.toFixed(0)} ms`);
} catch (e) {
  console.error(String(e?.stack ?? e));
  process.exit(2);
}
