// The page's own loader for a game module: the module runs in a Worker, with
// no imports, a declared memory limit and a deadline on every call.
//
// The page calls the module synchronously, so the two sides share a buffer.
//   ctl[0]  2 idle, 1 a call is waiting
//   ctl[1]  which export, ctl[2] what to return (0 nothing, 1 the output
//   buffer, 3 the 32 bytes the call's result points at), ctl[3] argument count
//   ctl[4]  0 done, 1 the module trapped, 2 too big; ctl[5] bytes returned
//   f64 at 32: the call's result; f64 at 40: its time in the Worker, in ms
//   from byte 64: arguments going in, the answer coming out
// An argument is a tag byte: 0 and a little-endian f64, or 1, a u32 length
// and bytes, which the Worker puts in module memory and passes as (ptr, len).
//
// The page waits by reading the buffer in a loop, which holds it for as long
// as the call takes, as running the module on the page itself would. A shared
// buffer needs the page's Cross-Origin-Opener-Policy and
// Cross-Origin-Embedder-Policy headers (public/_headers), so with no isolation
// the loader refuses rather than run a module without its limits.

const DATA = 64;
// Room for a bundle or a picture of the settings screen at any size in use.
const BUFFER = 64 * 1024 * 1024;
// 256 MB, in Wasm's 64 KB pages.
export const MAX_PAGES = 4096;
// The deadlines of calls that aren't one frame, in ms.
const LOAD_LIMIT = 30000;
const RUN_LIMIT = 600000;

function leb(b, at) {
  let v = 0;
  let shift = 0;
  for (;;) {
    const x = b[at.i++];
    if (x === undefined) throw new Error("the module ends inside a number");
    v += (x & 0x7f) * 2 ** shift;
    if (!(x & 0x80)) return v;
    shift += 7;
  }
}

// The largest memory a module declares, in pages; null when it sets none.
export function declaredMemory(bytes) {
  const b = new Uint8Array(bytes);
  const at = { i: 8 };
  while (at.i < b.length) {
    const id = b[at.i++];
    const len = leb(b, at);
    const end = at.i + len;
    if (id === 5) {
      const count = leb(b, at);
      if (count !== 1) throw new Error(`the module defines ${count} memories`);
      const flags = leb(b, at);
      leb(b, at);
      return flags & 1 ? leb(b, at) : null;
    }
    at.i = end;
  }
  return null;
}

export class ModuleStopped extends Error {}

export class ModuleHost {
  // `onStop(message)` is told once, when a limit ends the module.
  static async open(bytes, onStop = () => {}) {
    if (typeof SharedArrayBuffer === "undefined" || !self.crossOriginIsolated) {
      throw new Error("this page wasn't served with its isolation headers, so it can't run a game module with its limits");
    }
    const pages = declaredMemory(bytes);
    if (pages === null) throw new Error("the game's module sets no memory limit, so the page won't run it");
    if (pages > MAX_PAGES) throw new Error(`the game's module asks for up to ${pages / 16} MB of memory; the page allows ${MAX_PAGES / 16} MB`);
    const module = await WebAssembly.compile(bytes);
    if (WebAssembly.Module.imports(module).length) throw new Error("the game's module imports something, so the page won't run it");
    const sab = new SharedArrayBuffer(BUFFER);
    const worker = new Worker(new URL("module-worker.js", import.meta.url));
    const names = await new Promise((resolve, reject) => {
      worker.onerror = (e) => reject(new Error(e.message || "the module's Worker didn't start"));
      worker.onmessage = ({ data }) => (data.error ? reject(new Error(`the game's module wouldn't start: ${data.error}`)) : resolve(data.names));
      worker.postMessage({ module, sab });
    }).catch((e) => {
      worker.terminate();
      throw e;
    });
    return new ModuleHost(worker, sab, names, onStop);
  }

  constructor(worker, sab, names, onStop) {
    this.worker = worker;
    this.sab = sab;
    this.names = names;
    this.index = new Map(names.map((n, i) => [n, i]));
    this.onStop = onStop;
    this.ctl = new Int32Array(sab, 0, 8);
    this.num = new Float64Array(sab, 32, 2);
    this.view = new DataView(sab);
    this.bytes = new Uint8Array(sab);
    // Ten frame-times; preview.js sets it from the game's frame rate.
    this.frameLimit = 167;
    this.stopped = null;
    Atomics.store(this.ctl, 0, 2);
    // Each frame's time in the Worker and round trip, in ms, for the checks.
    this.timing = { frames: 0, worker: 0, round: 0, workerMax: 0, roundMax: 0 };
  }

  has(name) {
    return this.index.has(name);
  }

  stop(message) {
    this.stopped = message;
    this.worker.terminate();
    this.onStop(message);
    throw new ModuleStopped(message);
  }

  // Calls an export: numbers go as they are, each Uint8Array as (ptr, len).
  // Returns its result and the bytes `want` asked for.
  call(name, args = [], want = 1) {
    if (this.stopped) throw new ModuleStopped(this.stopped);
    const i = this.index.get(name);
    if (i === undefined) throw new Error(`the game's module has no ${name}`);
    let at = DATA;
    for (const a of args) {
      if (typeof a === "number") {
        this.view.setUint8(at, 0);
        this.view.setFloat64(at + 1, a, true);
        at += 9;
      } else {
        if (at + 5 + a.length > BUFFER) throw new Error(`${a.length} bytes are too many to hand the game's module at once`);
        this.view.setUint8(at, 1);
        this.view.setUint32(at + 1, a.length, true);
        this.bytes.set(a, at + 5);
        at += 5 + a.length;
      }
    }
    this.ctl[1] = i;
    this.ctl[2] = want;
    this.ctl[3] = args.length;
    const limit = name === "gp_step" ? this.frameLimit : name === "gp_run_frames" ? RUN_LIMIT : LOAD_LIMIT;
    const t0 = performance.now();
    Atomics.store(this.ctl, 0, 1);
    Atomics.notify(this.ctl, 0);
    while (Atomics.load(this.ctl, 0) !== 2) {
      if (performance.now() - t0 > limit) {
        this.stop(name === "gp_step"
          ? `The game stopped: one frame took more than ${Math.round(limit)} ms.`
          : `The game stopped: its module didn't answer within ${Math.round(limit / 1000)} s.`);
      }
    }
    const status = this.ctl[4];
    const out = this.bytes.slice(DATA, DATA + this.ctl[5]);
    if (status === 1) this.stop(`The game stopped: its module failed (${new TextDecoder().decode(out)}).`);
    if (status === 2) throw new Error(new TextDecoder().decode(out));
    if (name === "gp_step") {
      const round = performance.now() - t0;
      const t = this.timing;
      t.frames += 1;
      t.worker += this.num[1];
      t.round += round;
      t.workerMax = Math.max(t.workerMax, this.num[1]);
      t.roundMax = Math.max(t.roundMax, round);
    }
    return { r: this.num[0], out };
  }
}
