// The controls' timeline (docs/mechanic-page.md, "The controls' timing"):
// every change of the keys and the gamepad with the time it happened, and
// the buttons each game frame reads, taken at that frame's own time.
//
// Before this the page read the controls once for each game frame it
// stepped, at whatever moment the browser ran the step. A late browser
// frame steps up to four game frames in one go, and all four read the same
// buttons: a short press counted as four frames, or a release was seen four
// frames late. A press and release between two steps was never seen. And a
// gamepad, which a page can only poll, was read no oftener than the screen
// was drawn.
//
// Three settings, since controllers differ (the owner, 2026-11-22):
//   leadMs    how far past a frame's own time its reading may look, up to
//             now: 0 reads exactly at the frame's time; a frame's length
//             reads the newest state, the quickest to answer.
//   assistMs  how long a press is held on after it is let go: a pull that
//             fell a few ms short of what the game asks counts as made.
//   pollMs    how often the gamepad is polled.

//   stickTravel  how far a stick must move, as a percent of its reach, to
//             count as a pull beginning or ending. A pull ends as soon as
//             the stick has come back that far from where it got to, not
//             when it is back near the middle: the release is read at the
//             first of its travel home.

//   speed     how fast the game runs when you play it, as a percent of
//             its own speed: 50 is half speed, to see a later level
//             without the hands for it yet. Clips always play at 100.

export const DEFAULTS = Object.freeze({ leadMs: 8, assistMs: 0, pollMs: 4, stickTravel: 6, speed: 100 });

/** A stick read as a pull by which way it is travelling (the owner,
 * 2026-11-22: 'registering as early as possible on the stick's travel away
 * from down', 'its about releases being late'). It is pulled once it has
 * gone out `travel` of its reach from the lowest it has been since the last
 * pull, past a small rest zone; it is let go once it has come back `travel`
 * from the furthest it got. A stick read at a fixed halfway point is let go
 * only when it is most of the way home. */
export class StickPull {
  constructor(travelPercent = DEFAULTS.stickTravel) {
    this.travel = Math.min(0.5, Math.max(0.01, travelPercent / 100));
    this.pulled = false;
    /** While pulled, the furthest out; while not, the lowest since. */
    this.mark = 0;
  }

  /** The stick is `m` of its reach from the middle (0 to 1): pulled or not. */
  read(m) {
    const REST = 0.12;
    if (this.pulled) {
      if (m > this.mark) this.mark = m;
      if (m <= this.mark - this.travel || m < REST) { this.pulled = false; this.mark = m; }
    } else {
      if (m < this.mark) this.mark = m;
      if (m >= this.mark + this.travel && m >= REST) { this.pulled = true; this.mark = m; }
    }
    return this.pulled;
  }
}

/** Settings from `saved` (an object, or JSON text) over the defaults, each
 * kept inside its range. */
export function settings(saved) {
  let s = saved;
  if (typeof s === "string") {
    try { s = JSON.parse(s); } catch (e) { s = null; }
  }
  const num = (v, d, lo, hi) => (Number.isFinite(Number(v)) && v !== null && v !== "" ? Math.min(hi, Math.max(lo, Number(v))) : d);
  return {
    leadMs: num(s?.leadMs, DEFAULTS.leadMs, 0, 50),
    assistMs: num(s?.assistMs, DEFAULTS.assistMs, 0, 100),
    pollMs: num(s?.pollMs, DEFAULTS.pollMs, 1, 16),
    stickTravel: num(s?.stickTravel, DEFAULTS.stickTravel, 1, 50),
    speed: num(s?.speed, DEFAULTS.speed, 25, 100),
  };
}

export class InputTimeline {
  constructor(opts = {}) {
    this.opts = settings(opts);
    /** Changes, oldest first: the buttons from `t` on. */
    this.changes = [{ t: -Infinity, bits: 0 }];
    /** The time of the last frame taken. */
    this.taken = -Infinity;
    /** The buttons the last frame taken read. */
    this.prev = 0;
    /** What the frames read, for the page's own measure: each press as the
     * ms it was held and the frames that saw it. */
    this.seen = [];
    this.open = new Map();
  }

  /** The buttons are `bits` from time `t` on. */
  set(bits, t) {
    const last = this.changes[this.changes.length - 1];
    if (last.bits === bits) return;
    // A change stamped before the newest one goes just after it: times
    // from two clocks (an event's, a poll's) never reorder what happened.
    this.changes.push({ t: Math.max(t, last.t), bits });
    for (let b = 1; b <= bits || b <= last.bits; b <<= 1) {
      if ((bits & b) && !(last.bits & b)) this.open.set(b, { down: t, frames: 0 });
      if (!(bits & b) && (last.bits & b)) {
        const o = this.open.get(b);
        if (o) { this.seen.push({ bit: b, ms: t - o.down, frames: o.frames, o }); o.up = t; }
        if (this.seen.length > 200) this.seen.shift();
      }
    }
  }

  /** The buttons at time `t`, with each release put off by assistMs. */
  at(t) {
    let bits = 0;
    const hold = this.opts.assistMs;
    let state = 0;
    for (const c of this.changes) {
      if (c.t > t) break;
      state = c.bits;
    }
    bits = state;
    if (hold > 0) {
      // A button let go less than assistMs before t is still down.
      for (let i = this.changes.length - 1; i > 0; i--) {
        const c = this.changes[i];
        if (c.t > t) continue;
        if (c.t <= t - hold) break;
        bits |= this.changes[i - 1].bits & ~c.bits;
      }
    }
    return bits;
  }

  /** The buttons the game frame at time `t` reads, at `now`. Frames are
   * taken in order. A press that began and ended since the last frame taken
   * is read once, so a quick tap isn't lost. */
  take(t, now = t) {
    const read = Math.max(this.taken, Math.min(now, t + this.opts.leadMs));
    let bits = this.at(read);
    // Buttons that went down after the last frame's reading and are up
    // again by this one: read once, unless the last frame had them down.
    let tapped = 0;
    for (const c of this.changes) {
      if (c.t > read) break;
      if (c.t > this.taken) tapped |= c.bits;
    }
    bits |= tapped & ~this.prev;
    for (const [b, o] of this.open) {
      if (bits & b) o.frames++;
      if (o.up !== undefined && o.up <= read) this.open.delete(b);
    }
    this.taken = read;
    this.prev = bits;
    // Changes older than the assist's reach behind this frame are done.
    const keep = read - this.opts.assistMs - 1;
    while (this.changes.length > 2 && this.changes[1].t <= keep) this.changes.shift();
    return bits;
  }

  /** The newest press let go: the ms held and the frames that read it. */
  lastPress() {
    const p = this.seen[this.seen.length - 1];
    return p ? { ms: Math.round(p.ms), frames: p.o.frames } : null;
  }

  /** The last presses of `bit` as the ms held and the frames that read
   * them, newest last. */
  presses(bit, n = 10) {
    return this.seen.filter((p) => p.bit === bit).slice(-n).map((p) => ({ ms: Math.round(p.ms), frames: p.o.frames }));
  }
}
