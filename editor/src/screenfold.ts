// The screen measure on the page (docs/interpreter.md, "The screen
// measure"; docs/show.md): where the two games' pictures differ, as boxes
// over the screen, and a line for each layer. The server measures; this
// reads its score (a clip's, gp_interp::screen::Score) or a lockstep's
// screen (its frames' boxes). Pure, so the tests run it on made-up scores.

/** One group of differing pixels, in the original's whole frame. */
export interface ScreenBox {
  x: number;
  y: number;
  w: number;
  h: number;
  pixels?: number;
  /** What the model draws there, by name. */
  what?: string[];
}

export type ScreenLayer = "sprites" | "background";

/** Frames in a row with the same boxes, counted from the clip's start. */
export interface ScreenSpan {
  from: number;
  to: number;
  layer: ScreenLayer;
  boxes: [number, number, number, number][];
}

export interface ScreenLayerScore {
  name: string;
  owner: string;
  same: number;
  total: number;
  first?: number | null;
  boxes?: ScreenBox[];
  more?: number;
  drift?: number | null;
  shift?: [number, number] | null;
}

/** A clip's screen score, as the background work keeps it. */
export interface ScreenScore {
  background: ScreenLayerScore;
  sprites: ScreenLayerScore;
  /** "Characters: ...", "Background: ...". */
  says: string[];
  spans?: ScreenSpan[];
}

/** A box to draw, with its layer. */
export interface Drawn {
  layer: ScreenLayer;
  x: number;
  y: number;
  w: number;
  h: number;
}

/** The boxes of the clip's frame `at`, both layers. */
export function boxesAt(score: ScreenScore | undefined, at: number): Drawn[] {
  const out: Drawn[] = [];
  for (const s of score?.spans ?? []) {
    if (at < s.from || at > s.to) continue;
    for (const [x, y, w, h] of s.boxes) out.push({ layer: s.layer, x, y, w, h });
  }
  return out;
}

/** A lockstep's screen (`gp interp compare --json`, under `screen`). */
export interface LockScreen {
  visible: [number, number, number, number];
  background: { frames: { boxes: ScreenBox[] }[] };
  sprites: { frames: { boxes: ScreenBox[] }[] };
}

/** The lockstep's compared frame showing the original's frame `frame`: the last one at or before it. */
export function lockIndex(frames: [number, number][], frame: number): number {
  let k = -1;
  for (let i = 0; i < frames.length && frames[i][1] <= frame; i++) k = i;
  return k;
}

/** The boxes of the lockstep's compared frame `k`. */
export function lockBoxes(s: LockScreen | undefined, k: number): Drawn[] {
  if (!s || k < 0) return [];
  const out: Drawn[] = [];
  for (const layer of ["sprites", "background"] as ScreenLayer[]) {
    for (const b of s[layer].frames[k]?.boxes ?? []) out.push({ layer, x: b.x, y: b.y, w: b.w, h: b.h });
  }
  return out;
}

/** The clip card's lines: one for the characters, one for the background. */
export function screenLines(score: ScreenScore | undefined): string[] {
  return score?.says ?? [];
}
