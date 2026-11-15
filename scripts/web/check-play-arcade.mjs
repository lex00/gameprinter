// Plays an arcade input log in gp_arcade_play.wasm (crates/gp-arcade-play)
// the way the mechanic page does, a frame at a time with gp_play_step and
// drawing each frame, and writes every frame's picture hash, one in hex a
// line. `just play-web-arcade` then replays the same log natively
// (crates/gp-arcade-play/examples/pictures.rs) and checks the two lists
// are the same. On the way it checks a save point: the state saved at the
// log's middle, restored after its last frame and played on with the same
// controls, reaches the same picture digest. It also checks the sound
// export: stereo at 48 kHz, one or two 20 ms pieces a frame (none in frame 1), and not
// silent over the whole log. With SOUND_OUT it writes each frame's sound
// there, from frame 1: the sample count and the FNV-1a hash of the f32
// bytes in hex, a line each, which `just play-web-arcade` compares with
// the native replay's (crates/gp-arcade-play/examples/sounds.rs). With
// LATCHES too (crates/gp-arcade-play/examples/latches.rs: each frame's
// latch writes as they reached the sound board), it plays them on the
// module's sound board alone (gp_sound_load, gp_sound_frame), as a page
// whose game is a model would, and each frame's sound must be the whole
// machine's.
//   node scripts/web/check-play-arcade.mjs WASM ZIP LOG OUT [SOUND_OUT [LATCHES]]
// Prints the digest. Exit codes: 0 played, 2 couldn't.
import fs from "node:fs";

const [wasm, zipPath, logPath, out, soundOut, latchesPath] = process.argv.slice(2);

/** The 32-bit FNV-1a hash of `bytes`, as gp_arcade_play::fnv1a makes it. */
function fnv1a(bytes) {
  let h = 0x811c9dc5;
  for (const b of bytes) h = Math.imul(h ^ b, 0x01000193) >>> 0;
  return h >>> 0;
}
if (!out) {
  console.error("usage: node scripts/web/check-play-arcade.mjs WASM ZIP LOG OUT");
  process.exit(2);
}

/** The frames' controls from an arcade log (`gp-arcade-input 0`). */
function frames(text) {
  const lines = text.split("\n");
  if (lines[0].trim() !== "gp-arcade-input 0") throw new Error(`${logPath}: not an arcade input log`);
  let columns = null;
  const out = [];
  for (const raw of lines.slice(1)) {
    const line = raw.split("#")[0].trim();
    if (!line || line.startsWith("system ")) continue;
    if (line.startsWith("columns ")) { columns = line.slice(8).trim(); continue; }
    const [n, held] = line.split(/\s+/);
    let bits = 0;
    [...held].forEach((c, i) => { if (c === columns[i]) bits |= 1 << i; });
    for (let k = 0; k < Number(n); k++) out.push(bits);
  }
  return out;
}

try {
  const module = new WebAssembly.Module(fs.readFileSync(wasm));
  const imports = {};
  for (const i of WebAssembly.Module.imports(module)) {
    (imports[i.module] ??= {})[i.name] = () => { throw new Error(`called ${i.module}.${i.name}`); };
  }
  const x = new WebAssembly.Instance(module, imports).exports;
  const text = () => new TextDecoder().decode(new Uint8Array(x.memory.buffer, x.gp_out_ptr(), x.gp_out_len()));
  const put = (bytes) => {
    const p = x.gp_alloc(bytes.length);
    new Uint8Array(x.memory.buffer, p, bytes.length).set(bytes);
    return p;
  };
  const zip = fs.readFileSync(zipPath);
  if (x.gp_play_load(0, put(zip), zip.length)) throw new Error(text());
  if (x.gp_play_info(0)) throw new Error(text());
  const info = JSON.parse(text());
  if (info.width !== 512 || info.height !== 480) throw new Error(`gp_play_info: ${JSON.stringify(info)}`);
  if (!info.sound || x.gp_play_audio_channels() !== 2 || x.gp_play_audio_rate() !== 48000) {
    throw new Error(`the sound export: ${JSON.stringify(info)}, ${x.gp_play_audio_channels()} channels at ${x.gp_play_audio_rate()}`);
  }
  const pads = frames(fs.readFileSync(logPath, "utf8"));
  let loud = 0;
  const sounds = [];
  const half = Math.floor(pads.length / 2);
  const t0 = performance.now();
  let saved;
  for (let i = 0; i < pads.length; i++) {
    if (i === half) {
      const at = x.gp_play_save(0);
      if (!at) throw new Error(text());
      saved = new Uint8Array(x.memory.buffer, at, x.gp_out_len()).slice();
    }
    if (x.gp_play_step(0, pads[i], 0) < 0) throw new Error(text());
    x.gp_play_rgba(0);
    const a = x.gp_play_audio(0);
    if (!a) throw new Error(text());
    const n = x.gp_out_len() / 4;
    // Frame 1 ends before MAME's first 20 ms stream update.
    if (n !== 2 * 960 && n !== 4 * 960 && !(i === 0 && n === 0)) throw new Error(`frame ${i + 1}: ${n} samples of sound`);
    const raw = new Uint8Array(x.memory.buffer.slice(a, a + n * 4));
    sounds.push(`${n} ${fnv1a(raw).toString(16).padStart(8, "0")}\n`);
    const f = new Float32Array(raw.buffer);
    for (const v of f) if (Math.abs(v) > 0.01) loud++;
  }
  if (loud === 0) throw new Error("the sound export was silent for the whole log");
  const ms = performance.now() - t0;
  if (x.gp_play_pictures(0)) throw new Error(text());
  fs.writeFileSync(out, text());
  if (soundOut) fs.writeFileSync(soundOut, sounds.join(""));
  if (latchesPath) {
    // The sound board alone, fed the latch writes the whole machine made.
    if (x.gp_sound_load(put(zip), zip.length)) throw new Error(text());
    const lat = fs.readFileSync(latchesPath);
    let at = 0;
    let writes = 0;
    for (let i = 0; i < sounds.length; i++) {
      if (at + 4 > lat.length) throw new Error(`${latchesPath} ends before frame ${i + 1}`);
      const len = lat.readUInt32LE(at);
      at += 4;
      const ok = len ? x.gp_sound_frame(put(lat.subarray(at, at + len)), len) : x.gp_sound_frame(0, 0);
      if (ok) throw new Error(text());
      at += len;
      writes += len / 10;
      const a = x.gp_sound_audio();
      if (!a) throw new Error(text());
      const raw = new Uint8Array(x.memory.buffer.slice(a, a + x.gp_out_len()));
      const line = `${raw.length / 4} ${fnv1a(raw).toString(16).padStart(8, "0")}\n`;
      if (line !== sounds[i]) throw new Error(`frame ${i + 1}: the sound board alone made ${line.trim()}, the whole machine ${sounds[i].trim()}`);
    }
    console.error(`the sound board alone, fed ${writes} latch writes, sounded as the whole machine on all ${sounds.length} frames`);
  }
  if (x.gp_play_digest(0)) throw new Error(text());
  const first = text();
  if (x.gp_play_restore(0, put(saved), saved.length)) throw new Error(text());
  if (x.gp_play_frame(0) !== half) throw new Error(`restored to frame ${x.gp_play_frame(0)}, not ${half}`);
  for (let i = half; i < pads.length; i++) if (x.gp_play_step(0, pads[i], 0) < 0) throw new Error(text());
  if (x.gp_play_digest(0)) throw new Error(text());
  if (text() !== first) throw new Error(`restored at frame ${half} and played on, the digest is ${text()}, not ${first}`);
  console.log(first);
  console.error(`${loud} samples of sound above 1%`);
  console.error(`played ${pads.length} frames in ${ms.toFixed(0)} ms (${(pads.length / (ms / 1000)).toFixed(0)} a second; the game runs 30)`);
} catch (e) {
  console.error(String(e?.stack ?? e));
  process.exit(2);
}
