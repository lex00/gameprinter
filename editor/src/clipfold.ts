// Clips that cut themselves from the play loop (docs/show.md, "Clips"):
// with clips on, every save (F5) or load (F8) starts a clip at that save
// point, and the clip ends at the next save or load, or at the cap,
// whichever comes first. A clip with no presses, or where the player only
// pressed Start or Select, is dropped unless the setting keeps it. Start
// and stop by hand stay too. Pure, so the tests run the rules on made-up
// frames.

import { NES_FPS } from "./playfold";

/** How long a clip runs at most unless the setting says otherwise, in seconds. */
export const CAP_SECONDS = 8;

/** The cap of a clip started by hand: the longest the server takes. */
export const MANUAL_CAP_SECONDS = 30;

export interface ClipSettings {
  /** Cut clips from the play loop at each save and load. */
  on: boolean;
  /** The cap, in seconds. */
  seconds: number;
  /** Drop clips with no presses, or with only Start and Select. */
  dropIdle: boolean;
}

export const DEFAULT_SETTINGS: ClipSettings = { on: true, seconds: CAP_SECONDS, dropIdle: true };

const KEY = "gp.clips.settings";

/** A cap the setting accepts: 1 to 30 seconds, else the default. */
export function capSeconds(v: number): number {
  return Number.isFinite(v) && v >= 1 ? Math.min(MANUAL_CAP_SECONDS, Math.round(v * 10) / 10) : CAP_SECONDS;
}

/** The settings as last chosen in this browser; storage can be missing or refuse. */
export function readSettings(): ClipSettings {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "null");
    if (v && typeof v === "object") {
      return { on: v.on !== false, seconds: capSeconds(Number(v.seconds)), dropIdle: v.dropIdle !== false };
    }
  } catch {
    // Not kept; the defaults stand.
  }
  return { ...DEFAULT_SETTINGS };
}

export function saveSettings(s: ClipSettings): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(s));
  } catch {
    // Kept for this page only.
  }
}

/** What ended a clip. */
export type ClipEnd = "save" | "load" | "cap" | "stop";

/** A clip being recorded: from frame `from` of branch `branch`, at the save point made or loaded there (`mark`). */
export interface OpenClip {
  from: number;
  branch: number;
  /** The page's number for the save or load that started it, to find its save point once the server has it. */
  mark: number;
  manual: boolean;
}

/** A clip that ended: its frames of the branch, `from` up to `to` (not included), and why. */
export interface EndedClip extends OpenClip {
  to: number;
  why: ClipEnd;
}

export interface CutState {
  open?: OpenClip;
  /** Counts saves and loads, for `mark`. */
  marks: number;
}

export function emptyCut(): CutState {
  return { marks: 0 };
}

export function capFrames(s: ClipSettings, manual: boolean): number {
  return Math.round((manual ? MANUAL_CAP_SECONDS : s.seconds) * NES_FPS);
}

/**
 * A save or a load: the open clip ends at `end`, the frame play had
 * reached, and with clips on a new one starts at `start` of `branch` (for
 * a save the same frame; after a load the save point's frame on the new
 * branch). `mark` is the new save or load's number.
 */
export function onMark(c: CutState, kind: "save" | "load", end: number, start: number, branch: number, s: ClipSettings): { state: CutState; ended?: EndedClip; mark: number } {
  const mark = c.marks + 1;
  const ended = c.open ? fix({ ...c.open, to: end, why: kind }) : undefined;
  return { state: { marks: mark, open: s.on ? { from: start, branch, mark, manual: false } : undefined }, ended, mark };
}

function fix(e: EndedClip): EndedClip {
  return { ...e, to: Math.max(e.from, e.to) };
}

/** A frame played: the open clip ends by itself at the cap. `frame` is the frame now reached. */
export function onFrame(c: CutState, frame: number, s: ClipSettings): { state: CutState; ended?: EndedClip } {
  const o = c.open;
  if (!o || frame - o.from < capFrames(s, o.manual)) return { state: c };
  return { state: { marks: c.marks }, ended: { ...o, to: o.from + capFrames(s, o.manual), why: "cap" } };
}

/** "Start clip" by hand: a save point is made at `frame` (`mark`), and the clip runs from it until stop or the longer cap. */
export function onManualStart(c: CutState, frame: number, branch: number): { state: CutState; ended?: EndedClip; mark: number } {
  const mark = c.marks + 1;
  const ended = c.open ? fix({ ...c.open, to: frame, why: "save" }) : undefined;
  return { state: { marks: mark, open: { from: frame, branch, mark, manual: true } }, ended, mark };
}

/** "Stop": the open clip ends at `frame`. */
export function onStop(c: CutState, frame: number): { state: CutState; ended?: EndedClip } {
  if (!c.open) return { state: c };
  return { state: { marks: c.marks }, ended: fix({ ...c.open, to: frame, why: "stop" }) };
}

/** Whether frame lines (`P RLDUTSBA`) hold a press of a button that plays: anything but Start and Select. */
export function hasPlay(lines: string[]): boolean {
  return lines.some((l) => /[RLDUBA]/.test(l.slice(l.indexOf(" ") + 1)));
}

/** Whether frame lines hold any press at all. */
export function hasPress(lines: string[]): boolean {
  return lines.some((l) => /[A-Z]/.test(l.slice(l.indexOf(" ") + 1)));
}

/** Whether an ended clip is kept, and why not when it isn't. */
export function keepClip(lines: string[], s: ClipSettings, manual = false): { keep: true } | { keep: false; why: string } {
  if (lines.length === 0) return { keep: false, why: "it has no frames" };
  if (manual || !s.dropIdle) return { keep: true };
  if (!hasPress(lines)) return { keep: false, why: "nothing was pressed" };
  if (!hasPlay(lines)) return { keep: false, why: "only Start or Select was pressed" };
  return { keep: true };
}

/** The sentence about the clip being cut now, for the line under the screen. */
export function cutSays(c: CutState, frame: number, s: ClipSettings): string {
  const o = c.open;
  if (!o) return s.on ? "Clips: on. The next save (F5) or load (F8) starts one." : "Clips: off. Press F5 and F8 as you like; start a clip by hand when you want one.";
  const n = Math.max(0, frame - o.from);
  return `Recording a clip: ${(n / NES_FPS).toFixed(1)} s of ${o.manual ? MANUAL_CAP_SECONDS : s.seconds} s${o.manual ? " (started by hand; stop ends it)" : "; the next save or load ends it"}.`;
}

/** One frame line (`P! RLDUTSBA`) as the player steps it: the pad's bits, the reset, and whether the game read the pad. */
export function frameOfLine(l: string): { pad: number; reset: boolean; polled: boolean } {
  const [flag, pad = ""] = l.split(" ");
  let b = 0;
  for (let i = 0; i < 8; i++) if (pad[i] !== undefined && pad[i] !== ".") b |= 1 << (7 - i);
  return { pad: b, reset: flag.endsWith("!"), polled: flag.startsWith("P") };
}
