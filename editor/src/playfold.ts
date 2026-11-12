// The play panel's logic, apart from the browser (docs/show.md): the
// buttons held on the keyboard and a gamepad, the input log's frame lines
// the page uploads, the fixed-timestep pacing, and a play session's events
// folded into what the panel draws. Pure, so the tests run it on made-up
// input and events.

import type { CaptureEvent, PlayMark } from "./api";

/** The NES's frame rate, NTSC: the page paces frames at it. */
export const NES_FPS = 60.0988;

/** Frames per upload while playing; each piece stays under the server's body limit. */
export const UPLOAD_FRAMES = 2000;

/**
 * The keys for each button, A, B, Select, Start, Up, Down, Left, Right
 * from bit 0 (the input log's bits), as the interpreter preview maps them.
 */
export const KEYS: string[][] = [
  ["KeyX", "Space"],
  ["KeyZ", "ShiftLeft"],
  ["ShiftRight", "Tab"],
  ["Enter"],
  ["ArrowUp", "KeyW"],
  ["ArrowDown", "KeyS"],
  ["ArrowLeft", "KeyA"],
  ["ArrowRight", "KeyD"],
];

/** The key that drops a mark while playing. */
export const MARK_KEY = "KeyM";

/** The buttons held, from the keys down. */
export function padFromKeys(down: Set<string>): number {
  return KEYS.reduce((acc, ks, bit) => (ks.some((k) => down.has(k)) ? acc | (1 << bit) : acc), 0);
}

/** What a gamepad holds, in the standard mapping (buttons 0 and 2 are A and B, 8 and 9 Select and Start, 12 to 15 the d-pad; the left stick also steers). */
export function padFromGamepad(g: { readonly buttons: readonly { pressed: boolean }[]; readonly axes: readonly number[] } | null | undefined): number {
  if (!g) return 0;
  const b = (i: number) => g.buttons[i]?.pressed === true;
  const ax = g.axes[0] ?? 0;
  const ay = g.axes[1] ?? 0;
  let pad = 0;
  if (b(0) || b(1)) pad |= 1;
  if (b(2) || b(3)) pad |= 2;
  if (b(8)) pad |= 4;
  if (b(9)) pad |= 8;
  if (b(12) || ay < -0.5) pad |= 16;
  if (b(13) || ay > 0.5) pad |= 32;
  if (b(14) || ax < -0.5) pad |= 64;
  if (b(15) || ax > 0.5) pad |= 128;
  return pad;
}

/** The gamepad buttons, in the standard mapping, that save and load: the left trigger saves, the right trigger loads, so a player never takes a hand off the controller. */
export const SAVE_BUTTON = 6;
export const LOAD_BUTTON = 7;

/**
 * The save and load presses since the last read: a trigger counts once
 * when it goes down (pressed, or pulled past halfway on an analog
 * trigger), not on every frame it is held. `held` is what was down last
 * time; the result carries what is down now for the next call.
 */
export function triggerPresses(
  g: { readonly buttons: readonly { pressed: boolean; value?: number }[] } | null | undefined,
  held: { save: boolean; load: boolean },
): { save: boolean; load: boolean; held: { save: boolean; load: boolean } } {
  const down = (i: number) => {
    const b = g?.buttons[i];
    return b !== undefined && (b.pressed || (b.value ?? 0) > 0.5);
  };
  const now = { save: down(SAVE_BUTTON), load: down(LOAD_BUTTON) };
  return { save: now.save && !held.save, load: now.load && !held.load, held: now };
}

/** The frame lines of an input log's text, without its two header lines. */
export function frameLines(log: string): string[] {
  return log.split("\n").slice(2).filter((l) => l.length > 0);
}

/**
 * How many frames to run now, on a fixed timestep: `acc` is the time not
 * yet run, in milliseconds. The clock decides when a frame runs, never
 * what it does. A long pause runs at most `max` frames, so the page never
 * races to catch up.
 */
export function framesDue(acc: number, elapsed: number, max = 4): { frames: number; acc: number } {
  const tick = 1000 / NES_FPS;
  let a = Math.min(acc + elapsed, tick * max);
  let n = 0;
  while (a >= tick) {
    a -= tick;
    n += 1;
  }
  return { frames: n, acc: a };
}

/** The model frame an original frame reaches: the polled frames up to it, as the model plays them. */
export function modelFrame(lines: string[], frame: number): number {
  let n = 0;
  for (let i = 0; i < Math.min(frame, lines.length); i++) if (lines[i].startsWith("P")) n += 1;
  return n;
}

/** What `gp show explain` said about a mark. */
export interface Explanation {
  mark: number;
  summary: string;
  missing: string[];
  model: { frames?: [number, number] | null; events?: [string, number][]; counters?: { name: string; before: number; after: number }[]; tiles_changed?: number; spawned?: unknown[] };
  /** The original's side: interactions, cells, counters (`gp show explain`). */
  original?: Record<string, unknown>;
  from?: number;
  to?: number;
  note?: string;
}

export interface PlayState {
  seq: number;
  run?: string;
  name?: string;
  template?: string | null;
  realStore?: boolean;
  store?: string;
  marks: PlayMark[];
  stopped: boolean;
  recording?: string;
  frames?: number;
  digestMatches?: boolean;
  explanations: Record<number, Explanation>;
  explaining: boolean;
  /** The preview build: building, ok, skipped or failed. */
  preview?: { status: string; why?: string };
  errors: string[];
  /** The newest events last. */
  log: CaptureEvent[];
}

export function emptyPlay(): PlayState {
  return { seq: 0, marks: [], stopped: false, explanations: {}, explaining: false, errors: [], log: [] };
}

const str = (v: unknown): string => (typeof v === "string" ? v : v === undefined || v === null ? "" : JSON.stringify(v));

/** Folds a session's events (in sequence order); events at or below `s.seq` are ignored. */
export function foldPlay(s: PlayState, events: CaptureEvent[]): PlayState {
  const next: PlayState = { ...s, marks: [...s.marks], explanations: { ...s.explanations }, errors: [...s.errors], log: [...s.log] };
  for (const e of events) {
    if (e.seq <= next.seq) continue;
    next.seq = e.seq;
    const d = e.data ?? {};
    switch (e.kind) {
      case "play.start":
        next.run = str(d.run);
        next.name = str(d.name);
        next.template = typeof d.template === "string" ? d.template : null;
        next.realStore = d.real_store === true;
        next.store = str(d.store);
        break;
      case "play.mark": {
        const m = d as unknown as PlayMark;
        if (!next.marks.some((x) => x.id === m.id)) next.marks.push(m);
        break;
      }
      case "play.stop":
        next.stopped = true;
        break;
      case "play.recorded":
        next.recording = str(d.id);
        next.frames = typeof d.frames === "number" ? d.frames : undefined;
        next.digestMatches = d.digest_matches === true;
        break;
      case "play.annotated":
        next.marks = next.marks.map((m) => (m.id === d.mark ? { ...m, annotation: d.annotation as number } : m));
        break;
      case "show.start":
        next.explaining = true;
        break;
      case "show.mark": {
        const id = d.mark as number;
        next.explanations[id] = {
          mark: id,
          summary: str(d.summary),
          missing: Array.isArray(d.missing) ? (d.missing as string[]) : [],
          model: (d.model ?? {}) as Explanation["model"],
          original: d.original && typeof d.original === "object" ? (d.original as Record<string, unknown>) : undefined,
          from: typeof d.from === "number" ? d.from : undefined,
          to: typeof d.to === "number" ? d.to : undefined,
          note: typeof d.note === "string" ? d.note : undefined,
        };
        break;
      }
      case "show.end":
        next.explaining = false;
        if (d.model_error) next.errors.push(`the model didn't run: ${str(d.model_error)}`);
        break;
      case "show.preview":
        next.preview = { status: str(d.status), why: typeof d.why === "string" ? d.why : undefined };
        break;
      case "play.error":
      case "show.error":
        next.errors.push(str(d.message));
        next.explaining = false;
        break;
    }
    next.log.push(e);
  }
  if (next.log.length > 300) next.log = next.log.slice(next.log.length - 300);
  return next;
}

/** A mark's place on a timeline of `frames` frames, as percentages. */
export function markBox(m: { from: number; to: number }, frames: number): { left: number; width: number } {
  const n = Math.max(1, frames + 1);
  return { left: (m.from / n) * 100, width: Math.max(0.5, ((m.to - m.from + 1) / n) * 100) };
}

/** The frame a click at `x` of a timeline `width` wide points at. */
export function frameAt(x: number, width: number, frames: number): number {
  if (width <= 0) return 0;
  return Math.max(0, Math.min(frames, Math.round((x / width) * frames)));
}

/** The window a replay of a mark plays: a second before it to a second after, inside the session. */
export function replayWindow(m: { from: number; to: number }, frames: number, lead = 60): { from: number; to: number } {
  return { from: Math.max(0, m.from - lead), to: Math.min(frames, m.to + lead) };
}

/**
 * The model's input from an input log's frame lines: a frames file
 * (`gp-frames 1`, as gp-gen-bevy writes it) of the polled frames, the
 * buttons as the log's bits (A at bit 0 up to Right at bit 7) and `r` for
 * a reset. Model frame n is the original's n-th polled frame, as the play
 * panel's replay and `gp interp compare` line them up.
 */
export function framesScript(lines: string[]): string {
  const out = ["gp-frames 1"];
  for (const l of lines) {
    const [flag, pad = ""] = l.split(" ");
    if (!flag.startsWith("P")) continue;
    let b = 0;
    for (let i = 0; i < 8; i++) if (pad[i] !== undefined && pad[i] !== ".") b |= 1 << (7 - i);
    out.push(`${b.toString(16).padStart(2, "0")}${flag.endsWith("!") ? " r" : ""}`);
  }
  return out.join("\n") + "\n";
}

/**
 * The game view's model from a start (docs/inspector.md, "The same
 * commands to both"): loaded with the original's state at its frame
 * `start`, it takes one step for each polled frame after it. At the
 * original's frame `frame` it has taken this many; before the start, none.
 */
export function modelSteps(lines: string[], start: number, frame: number): number {
  let n = 0;
  for (let i = Math.max(0, start); i < Math.min(frame, lines.length); i++) if (lines[i].startsWith("P")) n += 1;
  return n;
}

/** The input the view's model plays from the original's frame `start` on: its polled frames after it. */
export function scriptFrom(lines: string[], start: number): string {
  return framesScript(lines.slice(Math.max(0, start)));
}

/** The NES buttons by the input log's letters, Right first as the log writes them. */
const BUTTONS: [string, string][] = [
  ["right", "Right"],
  ["left", "Left"],
  ["down", "Down"],
  ["up", "Up"],
  ["start", "Start"],
  ["select", "Select"],
  ["b", "B"],
  ["a", "A"],
];

/**
 * The buttons pressed on the original's frame `frame`, named as the model
 * names its actions: "Right + B (run)". The two games get the same ones.
 */
export function pressedWords(lines: string[], frame: number, actions?: Record<string, string> | null): string {
  const l = lines[frame - 1];
  if (!l) return "nothing";
  const pad = l.split(" ")[1] ?? "";
  const names: string[] = [];
  // Right, Left, Down, Up first, then B and A, as a person reads a press.
  const order = [0, 1, 2, 3, 6, 7, 4, 5];
  for (const i of order) {
    if (pad[i] === undefined || pad[i] === ".") continue;
    const [button, label] = BUTTONS[i];
    const action = Object.entries(actions ?? {}).find(([, b]) => b.toLowerCase() === button)?.[0];
    names.push(action && action !== button ? `${label} (${action.replace(/_/g, " ")})` : label);
  }
  return names.length ? names.join(" + ") : "nothing";
}

/**
 * One step of the view's playback: `n` frames on from `f`. In a focus it
 * loops the stretch; a clip plays to its end once and stops; and crossing
 * `stopAt` (the frame the two drift apart) stops both games there, so the
 * person sees where they part (docs/inspector.md, "Where they drift apart").
 */
export function playStep(f: number, n: number, o: { total: number; focus?: { from: number; to: number }; clipEnd?: number; stopAt?: number }): { frame: number; stop: boolean } {
  if (o.stopAt !== undefined && f < o.stopAt && f + n >= o.stopAt) return { frame: o.stopAt, stop: true };
  if (o.focus && o.clipEnd === undefined) {
    const next = f + n;
    return { frame: next > Math.min(o.focus.to, o.total) ? o.focus.from : next, stop: false };
  }
  const end = Math.min(o.clipEnd ?? o.total, o.total);
  const next = Math.min(end, f + n);
  return { frame: next, stop: next >= end };
}
