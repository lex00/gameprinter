// The mechanic page's live player, as plain JavaScript (no build step):
// gp_play.wasm (crates/gp-play, gp-nes for the browser) over its plain
// exports, as preview.js drives gp_web.wasm. One module holds two
// consoles: slot 0 for playing, slot 1 for a replay. Nothing here reads a
// clock; the panel decides when to step. A slot's whole state saves to
// bytes and restores from them (the save points): the console, the input
// log from power-on and every frame's hashes. Each frame's sound is kept
// until the next frame (`audio`); the page decides whether to play it.
//
// The module carries wasm-bindgen imports from tetanes-core's web code
// (local storage, time) that gp-nes never calls, so each is given a stub
// that throws if it ever is.

const text = new TextDecoder();
const enc = new TextEncoder();


/** Stubs for every import the module names. */
export function stubImports(module) {
  const out = {};
  for (const i of WebAssembly.Module.imports(module)) {
    (out[i.module] ??= {})[i.name] = () => {
      throw new Error(`gp_play.wasm called the browser import ${i.module}.${i.name}, which gp-nes never needs`);
    };
  }
  return out;
}

export class NesPlayer {
  constructor(x) {
    this.x = x;
  }

  /** Loads the module from `url`. Anything but a wasm answer (the
   * server's 404 when the player isn't built, as JSON) throws the server's
   * own words rather than a compile error on them. */
  static async load(url) {
    const r = await fetch(url);
    const type = r.headers.get("content-type") ?? "";
    if (!r.ok || !type.startsWith("application/wasm")) {
      let why = `${url}: ${r.status} ${type}`;
      try {
        const j = await r.json();
        if (j?.error) why = j.error;
      } catch (e) { /* not JSON: the status says it */ }
      throw new Error(why);
    }
    const bytes = await r.arrayBuffer();
    const module = await WebAssembly.compile(bytes);
    const instance = await WebAssembly.instantiate(module, stubImports(module));
    return new NesPlayer(instance.exports);
  }

  mem() {
    return new Uint8Array(this.x.memory.buffer);
  }
  put(bytes) {
    const p = this.x.gp_alloc(bytes.length);
    this.mem().set(bytes, p);
    return p;
  }
  out() {
    const p = this.x.gp_out_ptr();
    return text.decode(this.mem().slice(p, p + this.x.gp_out_len()));
  }
  check(code) {
    if (code !== 0) throw new Error(this.out());
    return this.out();
  }

  /** Powers a ROM on in a slot. */
  power(slot, rom) {
    this.check(this.x.gp_play_load(slot, this.put(rom), rom.length));
  }
  /** Powers a ROM on to replay an input log. */
  replay(slot, rom, log) {
    const l = enc.encode(log);
    const both = new Uint8Array(rom.length + l.length);
    both.set(rom, 0);
    both.set(l, rom.length);
    this.check(this.x.gp_play_replay(slot, this.put(both), both.length, rom.length));
  }
  /** One frame; whether the game read the pad. */
  step(slot, pad, reset = false) {
    const r = this.x.gp_play_step(slot, pad, reset ? 1 : 0);
    if (r < 0) throw new Error(this.out());
    return r === 1;
  }
  /** The replayed log's next frame; false at its end. */
  advance(slot) {
    const r = this.x.gp_play_advance(slot);
    if (r < 0) throw new Error(this.out());
    return r === 1;
  }
  runTo(slot, frame) {
    const r = this.x.gp_play_run_to(slot, frame);
    if (r < 0) throw new Error(this.out());
    return r;
  }
  frame(slot) {
    return this.x.gp_play_frame(slot);
  }
  /** The last frame as 256x240 RGBA, copied out. */
  rgba(slot) {
    const p = this.x.gp_play_rgba(slot);
    return new Uint8ClampedArray(this.x.memory.buffer.slice(p, p + 256 * 240 * 4));
  }
  /** The last frame's sound: mono samples at `audioRate()` a second,
   * copied out. Empty after a restore or a `runTo`, so a jump is silent. */
  audio(slot) {
    const p = this.x.gp_play_audio(slot);
    if (p === 0) throw new Error(this.out());
    return new Float32Array(this.x.memory.buffer.slice(p, p + this.x.gp_out_len()));
  }
  audioRate() {
    return this.x.gp_play_audio_rate();
  }
  inputLog(slot) {
    return this.check(this.x.gp_play_input_log(slot));
  }
  digest(slot) {
    return this.check(this.x.gp_play_digest(slot));
  }
  hash(slot) {
    return this.check(this.x.gp_play_hash(slot));
  }
  /** The slot's whole state as bytes, copied out. */
  save(slot) {
    const p = this.x.gp_play_save(slot);
    if (p === 0) throw new Error(this.out());
    return this.mem().slice(p, p + this.x.gp_out_len());
  }
  /** Back to bytes `save` gave; new presses branch from that frame. */
  restore(slot, bytes) {
    this.check(this.x.gp_play_restore(slot, this.put(bytes), bytes.length));
  }
}
