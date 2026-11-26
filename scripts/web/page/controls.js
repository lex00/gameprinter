// The controls of every page that plays a game: the browser's keys and
// gamepads handed, as they read, to gp_controls.wasm (crates/gp-controls),
// which decides everything a hand can feel: which button is which, when a
// stick counts as pushed, when a lever is pulled and let go, and which
// buttons each game frame reads (docs/controls.md).
//
// This file decides nothing. It is the one caller of the module, and both
// the public page (preview.js) and the screen page (mechanic/screen.js)
// import it, so a hand is read the same way on the page a person tests on
// and on the page the public plays (the owner, 2026-11-25: "the joystick
// pour is really slow in the browser.... it was ok in testing locally";
// "this should be handled by the game not the browser").

import { BINDINGS, NES, NES2 } from "./shell-keys.js";

const mine = (a) => BINDINGS.filter((b) => b.action === a);
const keysOf = (a) => mine(a).filter((b) => !b.shift).flatMap((b) => b.keys);

/** Each layout's keys, a list of `KeyboardEvent.code`s for each of its bits
 * from bit 0 (`keys` controller 1, `keys2` controller 2), from the shell's
 * table (shell-keys.js); `taken` the keys the game has that the shell's
 * table gives an action without Shift, and `names` what each bit is.
 * - nes: A, B, Select, Start, up, down, left, right.
 * - arcade: coin, start, right, left, down, up and the button, an arcade
 *   input log's columns. 5 drops a coin and 1 starts, as MAME keys them,
 *   and the NES layout's Select and Start keys do too. */
export const LAYOUTS = Object.freeze({
  nes: Object.freeze({ arcade: false, keys: NES.map(keysOf), keys2: NES2.map(keysOf), taken: new Set(), names: null }),
  arcade: Object.freeze({
    arcade: true,
    keys: [[...keysOf("select"), "Digit5"], [...keysOf("start"), "Digit1"], keysOf("right"), keysOf("left"), keysOf("down"), keysOf("up"), keysOf("a")],
    keys2: [],
    taken: new Set(["Digit5", "Digit1"]),
    names: ["coin", "start", "right", "left", "down", "up", "button"],
  }),
});

/** The timing's settings a person may change, with the module's defaults
 * (crates/gp-controls, Options) and how often the gamepad is polled. */
export const DEFAULTS = Object.freeze({ leadMs: 8, assistMs: 0, pollMs: 4, stickTravel: 6 });

const stamp = (t) => {
  const now = performance.now();
  return Number.isFinite(t) && t > 0 ? Math.min(t, now) : now;
};

let compiled = null;

/** Opens the controls on the module at `url` (gp_controls.wasm), in layout
 * `layout` ("nes" or "arcade").
 *
 * The page tells it the keys (`keyDown`, `keyUp`, `clearKeys`: the game's
 * keys are picked out by the layout, any other key is ignored) and calls
 * `start()` once; from then the gamepads are polled every `pollMs`. Each
 * game frame takes its buttons with `take(t, now)`: `t` the frame's own
 * time and `now` the present, both from `performance.now()`. */
export async function openControls(url, layout = "nes", options = {}) {
  compiled ??= WebAssembly.compileStreaming
    ? WebAssembly.compileStreaming(fetch(url)).catch(async () => WebAssembly.compile(await (await fetch(url)).arrayBuffer()))
    : fetch(url).then((r) => r.arrayBuffer()).then((b) => WebAssembly.compile(b));
  const x = (await WebAssembly.instantiate(await compiled, {})).exports;
  return new Controls(x, layout, options);
}

export class Controls {
  constructor(x, layout, options) {
    this.x = x;
    this.down = new Set();
    this.timer = 0;
    this.opts = { ...DEFAULTS };
    this.bits = 0;
    this.setLayout(layout);
    this.setOptions(options);
  }

  /** Starts over in a layout: "nes" or "arcade". */
  setLayout(name) {
    this.layout = LAYOUTS[name] ?? LAYOUTS.nes;
    this.x.gp_controls_layout(this.layout.arcade ? 1 : 0);
    this.poll();
  }

  /** New settings: leadMs, assistMs, stickTravel (the module keeps each
   * inside its range, and `opts` has them as it keeps them) and pollMs.
   * One left out, or not a number, stays as it was. */
  setOptions(o = {}) {
    const num = (v, d) => (Number.isFinite(Number(v)) && v !== null && v !== "" ? Number(v) : d);
    const want = {
      leadMs: num(o.leadMs, this.opts.leadMs),
      assistMs: num(o.assistMs, this.opts.assistMs),
      stickTravel: num(o.stickTravel, this.opts.stickTravel),
    };
    this.x.gp_controls_options(want.leadMs, want.assistMs, want.stickTravel);
    const kept = (i, v) => (this.x.gp_controls_option ? this.x.gp_controls_option(i) : v);
    this.opts = {
      leadMs: kept(0, want.leadMs),
      assistMs: kept(1, want.assistMs),
      stickTravel: kept(2, want.stickTravel),
      pollMs: Math.min(16, Math.max(1, num(o.pollMs, this.opts.pollMs))),
    };
    if (this.timer) this.start();
  }

  /** The newest press let go: `{ms, frames}`, the ms it was held (whole)
   * and the frames that read it; null before the first. */
  lastPress() {
    const ms = this.x.gp_controls_last_ms?.() ?? -1;
    return ms < 0 ? null : { ms: Math.round(ms), frames: this.x.gp_controls_last_frames() >>> 0 };
  }

  /** The keyboard's buttons in the layout's bits: controller 1 in the low
   * byte, 2 in the next. */
  keyBits() {
    let bits = 0;
    this.layout.keys.forEach((ks, bit) => { if (ks.some((k) => this.down.has(k))) bits |= 1 << bit; });
    this.layout.keys2.forEach((ks, bit) => { if (ks.some((k) => this.down.has(k))) bits |= 1 << (bit + 8); });
    return bits;
  }

  /** Hands the module the gamepads as they read now and the keys, at time
   * `t`. The buttons they give. */
  poll(t = performance.now()) {
    for (const g of globalThis.navigator?.getGamepads?.() ?? []) {
      if (!g || !g.connected) continue;
      let buttons = 0;
      for (let i = 0; i < 32 && i < g.buttons.length; i++) if (g.buttons[i]?.pressed) buttons |= 1 << i;
      const a = (i) => (typeof g.axes[i] === "number" ? g.axes[i] : NaN);
      this.x.gp_controls_pad(g.index, buttons >>> 0, a(0), a(1), a(2), a(3));
    }
    this.bits = this.x.gp_controls_set(this.keyBits() >>> 0, t) >>> 0;
    return this.bits;
  }

  /** A key went down or up, at the event's own time: never later than now
   * (an event made in the page around a frame carries that page's clock,
   * which started earlier), and now when it gives no time. */
  keyDown(code, t) { this.down.add(code); this.poll(stamp(t)); }
  keyUp(code, t) { this.down.delete(code); this.poll(stamp(t)); }
  /** Every key is up: the page lost the keyboard. */
  clearKeys(t = performance.now()) { this.down.clear(); this.poll(t); }

  /** Polls the gamepads from now on. */
  start() {
    clearInterval(this.timer);
    this.timer = setInterval(() => this.poll(), this.opts.pollMs);
  }
  stop() { clearInterval(this.timer); this.timer = 0; }

  /** The buttons the game frame at time `t` reads, at `now`. */
  take(t, now = performance.now()) {
    this.poll(now);
    return this.x.gp_controls_take(t, now) >>> 0;
  }

  /** The buttons as they are now, read by no frame. */
  now() { return this.poll(); }

  /** The buttons a gamepad alone holds now: those no key holds. */
  padBits() { return (this.poll() & ~this.keyBits()) >>> 0; }
}
