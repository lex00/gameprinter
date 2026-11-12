// The game's own moments in words (docs/inspector.md, "Moments"): what
// `GET /api/capture/{run}/moments` gives (each jump, landing, death, life
// lost, counter change and level change of the original, with what the
// model does there on the same presses), named in the model's words, and
// where each sits on a strip. Nothing here names a game.

import type { GameWords } from "./api";
import type { Focus } from "./lockfold";
import { thing, words } from "./phrase";
import { NES_FPS } from "./playfold";

export type MomentKind = "jump" | "landing" | "death" | "life_lost" | "counter" | "level";

export interface MomentModel {
  from: number;
  to: number;
  /** The model's frame minus the original's: 2 is two frames later. */
  offset: number;
  peak: number | null;
  height: number | null;
  after: string | null;
}

export interface Moment {
  kind: MomentKind;
  n: number;
  part: string;
  mechanic: string | null;
  from: number;
  to: number;
  seconds_from: number;
  seconds_to: number;
  peak: number | null;
  height: number | null;
  before: string | null;
  after: string | null;
  /** The stretch a focus on it covers: half a second either side. */
  focus: Focus;
  checked: boolean;
  start: number | null;
  model: MomentModel | null;
}

export interface Moments {
  recording?: string;
  moments: Moment[];
  kinds: Record<string, { original: number; model: number; mechanic: string | null }>;
  first_frame: number;
  measured: number;
}

/** The kinds in the strip's order, each with its name and verbs. */
export const KIND_WORDS: Record<MomentKind, { name: string; does: string; doesnt: string }> = {
  jump: { name: "Jump", does: "jumps", doesnt: "doesn't jump" },
  landing: { name: "Landing", does: "lands", doesnt: "doesn't land" },
  death: { name: "Death", does: "dies", doesnt: "doesn't die" },
  life_lost: { name: "Life lost", does: "loses a life", doesnt: "doesn't lose a life" },
  counter: { name: "Change", does: "changes", doesnt: "doesn't change" },
  level: { name: "Level change", does: "changes level", doesnt: "stays in the level" },
};

/** Who a moment happened to: "Mario", "the second slug", "the coins". */
export function partName(g: GameWords | undefined, part: string): string {
  const e = /^entities\.([a-z0-9_]+)\[(\d+)\]$/.exec(part);
  if (e) {
    const n = Number(e[2]);
    return n === 0 ? thing(g, e[1]) : `the ${["", "second ", "third ", "fourth "][n] ?? `${n + 1}th `}${words(e[1])}`;
  }
  if (part.startsWith("counters.")) return `the ${words(part.slice(9))}`;
  if (part === "level") return "the level";
  return "something";
}

/** A moment's name: "Jump 1", "Coins 3" for a counter. */
export function momentName(m: Moment): string {
  if (m.kind === "counter") {
    const c = words(m.part.replace(/^counters\./, ""));
    return `${c.charAt(0).toUpperCase()}${c.slice(1)} ${m.n}`;
  }
  return `${KIND_WORDS[m.kind].name} ${m.n}`;
}

const px = (n: number) => `${Math.round(n * 10) / 10} px`;

function when(offset: number): string {
  if (offset === 0) return "on the same frame";
  const n = Math.abs(offset);
  return `${n === 1 ? "1 frame" : `${n} frames`} ${offset > 0 ? "later" : "earlier"}`;
}

/**
 * A moment in a line, as the strip's hover reads it: "Jump 1, 3.6 s: the
 * original goes up 66 px; the model 52 px, 2 frames later".
 */
export function momentLine(g: GameWords | undefined, m: Moment): string {
  const at = `${momentName(m)}, ${(m.from / NES_FPS).toFixed(1)} s`;
  const k = KIND_WORDS[m.kind];
  const x = m.model;
  if (m.kind === "jump") {
    const orig = m.height === null ? "the original jumps" : `the original goes up ${px(m.height)}`;
    if (!m.checked) return `${at}: ${orig}`;
    if (!x) return `${at}: ${orig}; the model doesn't jump`;
    const h = x.height === null ? "jumps too" : px(x.height);
    return `${at}: ${orig}; the model ${h}${x.offset ? `, ${when(x.offset)}` : ""}`;
  }
  const who = partName(g, m.part);
  const orig =
    m.kind === "counter"
      ? `${who} go from ${m.before} to ${m.after}`
      : m.kind === "level"
        ? `the original goes from ${words(m.before ?? "")} to ${words(m.after ?? "")}`
        : m.kind === "life_lost"
          ? "the original loses a life"
          : `${who} ${k.does}`;
  if (!m.checked) return `${at}: ${orig}`;
  const later = x?.offset ? `, ${when(x.offset)}` : "";
  if (m.kind === "counter") return `${at}: ${orig}; ${x ? `the model's ${words(m.part.slice(9))} do too${later}` : `the model's don't change`}`;
  return `${at}: ${orig}; ${x ? `the model does too${later}` : `the model ${k.doesnt}`}`;
}

/** The moments in a stretch: any that overlaps it. */
export function inFocus(ms: Moment[], f: Focus): Moment[] {
  return ms.filter((m) => m.to >= f.from && m.from <= f.to);
}

/** A tick on a strip from `lo` to `hi`: its left edge and width, from 0 to 1. */
export function tick(m: Moment, lo: number, hi: number): { left: number; width: number } {
  const span = Math.max(1, hi - lo);
  const a = Math.max(lo, Math.min(hi, m.from));
  const b = Math.max(lo, Math.min(hi, m.to));
  return { left: (a - lo) / span, width: Math.max(0, (b - a) / span) };
}

/**
 * The moments a phrase names, as the tell box or the agent reads it:
 * "the very first jump" -> jump 1, "the third coin" -> counters.coins 3,
 * "the last death" -> the last death. Kinds by their words; numbers as
 * digits or ordinals.
 */
export function findMoment(ms: Moment[], said: string): Moment | undefined {
  const s = said.toLowerCase();
  const kinds: [MomentKind, RegExp][] = [
    ["jump", /\bjump/],
    ["landing", /\bland/],
    ["life_lost", /\blives?\b|\blife\b/],
    ["death", /\bdie|\bdeath|\bdied/],
    ["level", /\blevel/],
  ];
  let pick = ms;
  const k = kinds.find(([, r]) => r.test(s));
  if (k) pick = ms.filter((m) => m.kind === k[0]);
  else {
    const one = (n: string) => (n.endsWith("ies") ? `${n.slice(0, -3)}y` : n.replace(/s$/, ""));
    const counter = ms.find((m) => m.kind === "counter" && s.includes(one(words(m.part.slice(9)))));
    if (!counter) return undefined;
    pick = ms.filter((m) => m.part === counter.part);
  }
  if (!pick.length) return undefined;
  if (/\blast\b/.test(s)) return pick[pick.length - 1];
  const ord = ["first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth", "tenth"];
  const o = ord.findIndex((w) => new RegExp(`\\b${w}\\b`).test(s));
  const d = /\b(\d+)(st|nd|rd|th)?\b/.exec(s);
  const n = o >= 0 ? o + 1 : d ? Number(d[1]) : 1;
  return pick[n - 1];
}

/**
 * Where the moments are, for the view and a guide that drives it: loading
 * (finding them and checking the model at each), ready (how many, and how
 * many the model does too), failed, or none (no ROM to read them from).
 */
export interface MomentsPhase {
  state: "none" | "loading" | "ready" | "failed";
  says: string;
  next: string;
}

export function momentsPhase(ms: Moments | undefined, loading: boolean, error?: string): MomentsPhase {
  if (error) return { state: "failed", says: `The game's moments couldn't be found: ${error}.`, next: "The two games can still be compared a stretch at a time." };
  if (loading || !ms) return { state: "loading", says: "Finding the game's moments (jumps, landings, deaths, coins) and checking the model at each...", next: "Then each shows as a tick above the slider." };
  const j = ms.kinds.jump;
  const jumps = j ? ` ${j.original === 1 ? "1 jump" : `${j.original} jumps`}, and the model does ${j.model} of them.` : "";
  return {
    state: "ready",
    says: `${ms.moments.length === 1 ? "1 moment" : `${ms.moments.length} moments`} found.${jumps}`,
    next: "Click a tick to compare the two games there.",
  };
}
