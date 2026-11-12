// The live player: gp_play.wasm (crates/gp-play, gp-nes for the browser)
// over its plain exports, as preview.js drives gp_web.wasm. One module
// holds two consoles: slot 0 for playing, slot 1 for a replay. Nothing
// here reads a clock; the panel decides when to step. A slot's whole state
// saves to bytes and restores from them (free play's save points,
// docs/show.md): the console, the input log from power-on and every
// frame's hashes.
//
// The module carries wasm-bindgen imports from tetanes-core's web code
// (local storage, time) that gp-nes never calls, so each is given a stub
// that throws if it ever is.

const text = new TextDecoder();
const enc = new TextEncoder();

interface Exports {
  memory: WebAssembly.Memory;
  gp_alloc(len: number): number;
  gp_out_ptr(): number;
  gp_out_len(): number;
  gp_play_load(slot: number, ptr: number, len: number): number;
  gp_play_replay(slot: number, ptr: number, len: number, romLen: number): number;
  gp_play_step(slot: number, pad: number, reset: number): number;
  gp_play_advance(slot: number): number;
  gp_play_run_to(slot: number, frame: number): number;
  gp_play_frame(slot: number): number;
  gp_play_rgba(slot: number): number;
  gp_play_input_log(slot: number): number;
  gp_play_digest(slot: number): number;
  gp_play_hash(slot: number): number;
  gp_play_save(slot: number): number;
  gp_play_restore(slot: number, ptr: number, len: number): number;
}

/** Stubs for every import the module names. */
export function stubImports(module: WebAssembly.Module): WebAssembly.Imports {
  const out: Record<string, Record<string, () => never>> = {};
  for (const i of WebAssembly.Module.imports(module)) {
    (out[i.module] ??= {})[i.name] = () => {
      throw new Error(`gp_play.wasm called the browser import ${i.module}.${i.name}, which gp-nes never needs`);
    };
  }
  return out;
}

export class NesPlayer {
  private constructor(private readonly x: Exports) {}

  static async load(url: string): Promise<NesPlayer> {
    const bytes = await (await fetch(url)).arrayBuffer();
    const module = await WebAssembly.compile(bytes);
    const instance = await WebAssembly.instantiate(module, stubImports(module));
    return new NesPlayer(instance.exports as unknown as Exports);
  }

  private mem(): Uint8Array {
    return new Uint8Array(this.x.memory.buffer);
  }
  private put(bytes: Uint8Array): number {
    const p = this.x.gp_alloc(bytes.length);
    this.mem().set(bytes, p);
    return p;
  }
  private out(): string {
    const p = this.x.gp_out_ptr();
    return text.decode(this.mem().slice(p, p + this.x.gp_out_len()));
  }
  private check(code: number): string {
    if (code !== 0) throw new Error(this.out());
    return this.out();
  }

  /** Powers a ROM on in a slot. */
  power(slot: number, rom: Uint8Array): void {
    this.check(this.x.gp_play_load(slot, this.put(rom), rom.length));
  }
  /** Powers a ROM on to replay an input log. */
  replay(slot: number, rom: Uint8Array, log: string): void {
    const l = enc.encode(log);
    const both = new Uint8Array(rom.length + l.length);
    both.set(rom, 0);
    both.set(l, rom.length);
    this.check(this.x.gp_play_replay(slot, this.put(both), both.length, rom.length));
  }
  /** One frame; whether the game read the pad. */
  step(slot: number, pad: number, reset = false): boolean {
    const r = this.x.gp_play_step(slot, pad, reset ? 1 : 0);
    if (r < 0) throw new Error(this.out());
    return r === 1;
  }
  /** The replayed log's next frame; false at its end. */
  advance(slot: number): boolean {
    const r = this.x.gp_play_advance(slot);
    if (r < 0) throw new Error(this.out());
    return r === 1;
  }
  runTo(slot: number, frame: number): number {
    const r = this.x.gp_play_run_to(slot, frame);
    if (r < 0) throw new Error(this.out());
    return r;
  }
  frame(slot: number): number {
    return this.x.gp_play_frame(slot);
  }
  /** The last frame as 256x240 RGBA, copied out. */
  rgba(slot: number): Uint8ClampedArray<ArrayBuffer> {
    const p = this.x.gp_play_rgba(slot);
    return new Uint8ClampedArray(this.x.memory.buffer.slice(p, p + 256 * 240 * 4) as ArrayBuffer);
  }
  inputLog(slot: number): string {
    return this.check(this.x.gp_play_input_log(slot));
  }
  digest(slot: number): string {
    return this.check(this.x.gp_play_digest(slot));
  }
  hash(slot: number): string {
    return this.check(this.x.gp_play_hash(slot));
  }
  /** The slot's whole state as bytes, copied out. */
  save(slot: number): Uint8Array {
    const p = this.x.gp_play_save(slot);
    if (p === 0) throw new Error(this.out());
    return this.mem().slice(p, p + this.x.gp_out_len());
  }
  /** Back to bytes `save` gave; new presses branch from that frame. */
  restore(slot: number, bytes: Uint8Array): void {
    this.check(this.x.gp_play_restore(slot, this.put(bytes), bytes.length));
  }
}
