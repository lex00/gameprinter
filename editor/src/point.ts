// Pause, point and tell (docs/point-and-tell.md): the person pauses the
// two games, clicks a spot or drags a box on either, sees what's there in
// plain words ("a brick", "the goomba", "the score") and says what should
// happen there. Pure, so the tests drive it with made-up events and boxes.

import type { Box, CaptureInput } from "./api";

export const W = 256;
export const H = 240;

/** A click is a box this big around the spot: about one level cell. */
export const CLICK = 16;

/** A drag shorter than this is a click. */
export const CLICK_SLOP = 3;

/** The quick choices, as gp-capture's requests::CHOICES has them. */
export const CHOICES: { key: string; words: string }[] = [
  { key: "missing", words: "this is missing" },
  { key: "position", words: "wrong position" },
  { key: "look", words: "wrong look" },
  { key: "sound", words: "wrong sound" },
  { key: "react", words: "should react when I hit it" },
];

/** Where a pointer is on the 256x240 picture, from where the screen is drawn on the page. */
export function screenPoint(clientX: number, clientY: number, r: { left: number; top: number; width: number; height: number }): { x: number; y: number } {
  const x = Math.floor(((clientX - r.left) / Math.max(1, r.width)) * W);
  const y = Math.floor(((clientY - r.top) / Math.max(1, r.height)) * H);
  return { x: Math.max(0, Math.min(W - 1, x)), y: Math.max(0, Math.min(H - 1, y)) };
}

/** The box from a drag's two ends, in whole pixels on the picture; a click is a CLICK-sized box around the spot. */
export function boxFrom(a: { x: number; y: number }, b: { x: number; y: number }): Box {
  if (Math.abs(a.x - b.x) < CLICK_SLOP && Math.abs(a.y - b.y) < CLICK_SLOP) {
    const x = Math.max(0, Math.min(W - CLICK, a.x - CLICK / 2));
    const y = Math.max(0, Math.min(H - CLICK, a.y - CLICK / 2));
    return { x, y, w: CLICK, h: CLICK };
  }
  const x = Math.max(0, Math.min(a.x, b.x));
  const y = Math.max(0, Math.min(a.y, b.y));
  return { x, y, w: Math.min(W, Math.max(a.x, b.x) + 1) - x, h: Math.min(H, Math.max(a.y, b.y) + 1) - y };
}

/** Whether a key press pauses or resumes: Space or P, unless the person is typing. */
export function isPauseKey(e: { code?: string; key?: string; target?: unknown }): boolean {
  const t = e.target as { tagName?: string; isContentEditable?: boolean } | null | undefined;
  const tag = t?.tagName?.toLowerCase();
  if (tag === "input" || tag === "textarea" || tag === "select" || t?.isContentEditable) return false;
  return e.code === "Space" || e.code === "KeyP" || e.key === "p" || e.key === "P";
}

/** One thing under the box, as `GET /api/capture/{run}/point` names it. */
export interface PointThing {
  what: "entity" | "hud" | "cell" | "sprite";
  label: string;
  kind?: string;
  role?: string | null;
  /** For an entity: how many each game has under the box. */
  original?: number;
  model?: number;
  name?: string;
  counter?: string;
}

/** What `GET /api/capture/{run}/point` answers. */
export interface PointJson {
  run: string;
  kind: "capture" | "play";
  recording: string;
  recording_id: string | null;
  frame: number;
  model_frame: number;
  rect: Box;
  label: string;
  things: PointThing[];
  original: { world?: Box | null; camera?: unknown; entities?: { kind: string; box: { x: number; y: number | null; w: number; h: number | null } }[]; error?: string };
  model: { world?: Box; camera?: [number, number]; entities?: { kind: string; box: Box }[]; error?: string };
}

/** Which games have a thing: "in both games", "only in the original", "only in the rebuilt game". */
export function where(t: PointThing): string {
  if (t.what !== "entity") return "";
  const o = (t.original ?? 0) > 0;
  const m = (t.model ?? 0) > 0;
  if (o && m) return "in both games";
  if (o) return "only in the original";
  if (m) return "only in the rebuilt game";
  return "";
}

/** The request a Tell sends: the frame, the region in both games, the things there and the words. */
export function requestInput(p: PointJson, choice: string | undefined, text: string): CaptureInput {
  return {
    v: 1,
    actor: "person",
    kind: "request",
    text: text.trim(),
    data: {
      frame: p.frame,
      recording: p.recording,
      recording_id: p.recording_id,
      choice: choice ?? null,
      label: p.label,
      rect: p.rect,
      // What the explanation reads: the resolved things and where the box
      // is in each game's world. The sprites and fields stay on the server.
      point: {
        label: p.label,
        rect: p.rect,
        things: p.things,
        model_frame: p.model_frame,
        original: { world: p.original.world ?? null, camera: p.original.camera ?? null },
        model: { world: p.model.world ?? null, camera: p.model.camera ?? null },
      },
    },
  };
}

/** The words a choice key stands for. */
export function choiceWords(key?: string): string | undefined {
  return CHOICES.find((c) => c.key === key)?.words;
}
