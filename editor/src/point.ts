// Pause, point and tell (docs/point-and-tell.md): the person pauses the
// two games, clicks a spot or drags a box on either, sees what's there in
// plain words ("a brick", "the goomba", "the score") and says what should
// happen there. Pure, so the tests drive it with made-up events and boxes.

import type { Box, CaptureEvent, CaptureInput } from "./api";

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

/** A reply under a request. */
export interface RequestReply {
  reply: number;
  status: string;
  text: string;
  lines: string[];
  proposal?: { item: string; kind: string };
  wouldBuild?: Record<string, unknown> | null;
  /** Set on a progress note from gp serve's local agent: `working`, `failed` or `done`. */
  agent?: string;
}

/** A request as the workspace shows it, folded from the run's `request` and `request.reply` events. */
export interface RequestView {
  id: number;
  frame: number;
  recording: string;
  label: string;
  text: string;
  choice?: string;
  rect?: Box;
  status: string;
  replies: RequestReply[];
}

const str = (v: unknown): string => (typeof v === "string" ? v : "");

/** Folds `request` and `request.reply` events into requests by number. Other events change nothing. */
export function foldRequests(s: Record<number, RequestView>, events: CaptureEvent[]): Record<number, RequestView> {
  let next = s;
  for (const e of events) {
    const d = e.data ?? {};
    if (e.kind === "request") {
      const id = Number(d.id);
      if (next[id]) continue;
      const ctx = (d.context ?? {}) as Record<string, unknown>;
      next = {
        ...next,
        [id]: {
          id,
          frame: Number(d.frame),
          recording: str(d.recording),
          label: str(d.label) || "this spot",
          text: str(d.text),
          choice: typeof d.choice === "string" ? d.choice : undefined,
          rect: ctx.rect as Box | undefined,
          status: "open",
          replies: [],
        },
      };
    } else if (e.kind === "request.reply") {
      const id = Number(d.request);
      const r = next[id];
      if (!r || r.replies.some((x) => x.reply === Number(d.reply))) continue;
      const data = (d.data ?? {}) as Record<string, unknown>;
      const reply: RequestReply = {
        reply: Number(d.reply),
        status: str(d.status),
        text: str(d.text),
        lines: Array.isArray(d.lines) ? (d.lines as string[]) : [],
        proposal: (data.proposal as RequestReply["proposal"]) ?? undefined,
        wouldBuild: (data.would_build as Record<string, unknown>) ?? null,
        agent: typeof data.agent === "string" ? data.agent : undefined,
      };
      next = { ...next, [id]: { ...r, status: reply.status, replies: [...r.replies, reply] } };
    } else if (e.kind === "input") {
      // A decision on a reply's proposal decides the request.
      const input = (d.input ?? {}) as { kind?: string; item?: string };
      if ((input.kind === "accept" || input.kind === "reject") && input.item) {
        for (const r of Object.values(next)) {
          if (r.replies.some((x) => x.proposal?.item === input.item) || input.item === `request:${r.id}`) {
            next = { ...next, [r.id]: { ...r, status: input.kind === "accept" ? "accepted" : "rejected" } };
          }
        }
      }
    }
  }
  return next;
}

/** A request's status in a few plain words. */
export function statusWords(status: string, explaining = false, agentWorking = false): string {
  if (agentWorking) return "an agent is working on this";
  if (explaining) return "looking at what the original does there...";
  switch (status) {
    case "open":
      return "waiting for an answer";
    case "waiting":
      return "explained once the session is recorded";
    case "explained":
      return "explained";
    case "proposed":
      return "a change to look at";
    case "needs_mechanic":
      return "needs a new kind of part in the model (passed on to be built)";
    case "accepted":
      return "you took the change";
    case "rejected":
      return "you turned the change down";
    default:
      return status;
  }
}

/** The words a choice key stands for. */
export function choiceWords(key?: string): string | undefined {
  return CHOICES.find((c) => c.key === key)?.words;
}
