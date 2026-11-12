// The calm main screen's rules (docs/show.md, "The main screen"): which
// clips are in progress and in what order, the at most two badges on each
// thumbnail, which questions are for a person ("needs you"), a clip's
// status chip and where it parts. The user asked for it on 2026-10-30:
// "mostly the UI should be only the render screen along with thumbnails
// of what is in process with indicators of what might need attention or
// has a question on it". Pure, so the tests run it on made-up states.

import type { GameWords } from "./api";
import { type WorkAttempt, type WorkEntry, type WorkState, fixReady, justResolved, openTells } from "./agentfold";
import type { BgQuestion, BgState } from "./backfold";
import { measureName } from "./lockfold";
import type { ClipScore, StripClip } from "./stripfold";

/** How long a clip counts as "changed recently" after its score moved. */
export const RECENT_MS = 10 * 60 * 1000;

/** A badge on a thumbnail. */
export type BadgeKind = "fix-ready" | "resolved" | "needs-you" | "tell" | "agent" | "fixing" | "scoring" | "better" | "stuck";

export interface Badge {
  kind: BadgeKind;
  label: string;
}

/** What the rules read: the queue, the work record, and when this page saw each clip's score change. */
export interface Ctx {
  bg?: BgState;
  work?: WorkState;
  /** Clip -> when its score last changed on this page (ms). */
  changed?: Record<string, number>;
  now: number;
}

/** A conflict question: the game's code and the fit disagree, so a person checks the mapping. */
export function isConflict(q: Pick<BgQuestion, "choices"> & { kind?: string }): boolean {
  if (q.kind) return q.kind === "conflict";
  return q.choices.includes("check the mapping");
}

/** A clip the person recorded, not one the tool made (`gen.`). */
export function isRecorded(clip: string): boolean {
  return !clip.startsWith("gen.");
}

/**
 * Whether an open question is a real request for a person: a conflict,
 * or one on a clip the person recorded. The generic "Watch the clip"
 * questions on clips the tool made are not; the clip agents take those.
 */
export function questionNeedsYou(q: BgQuestion & { kind?: string }): boolean {
  if (q.answer) return false;
  return isConflict(q) || isRecorded(q.clip);
}

/**
 * The asks an agent posted that nobody has answered: a work entry of kind
 * "ask" with no later entry by the person, or "done", on the same clip.
 */
export function agentAsks(work: WorkState | undefined, clip?: string): WorkEntry[] {
  const entries = [...(work?.entries ?? [])].sort((a, b) => a.when_ms - b.when_ms);
  const out: WorkEntry[] = [];
  entries.forEach((e, i) => {
    if (e.kind !== "ask" || !e.clip || (clip !== undefined && e.clip !== clip)) return;
    const closed = entries.slice(i + 1).some((x) => x.clip === e.clip && (x.by === "you" || x.kind === "done" || x.kind === "answer"));
    if (!closed) out.push(e);
  });
  return out;
}

/** The questions on a clip that need the person, and the rest (shown on the clip's detail only). */
export function clipQuestions(bg: BgState | undefined, clip: string): { needsYou: BgQuestion[]; other: BgQuestion[] } {
  const open = (bg?.questions ?? []).filter((q) => q.clip === clip && !q.answer);
  return { needsYou: open.filter(questionNeedsYou), other: open.filter((q) => !questionNeedsYou(q)) };
}

/** Every clip with something for the person: a question that needs them or an agent's ask. */
export function needsYouClips(bg: BgState | undefined, work: WorkState | undefined): Set<string> {
  const out = new Set<string>();
  for (const q of bg?.questions ?? []) if (questionNeedsYou(q)) out.add(q.clip);
  for (const e of agentAsks(work)) out.add(e.clip!);
  // A fix waiting to be merged is not for the person, whatever else is open.
  for (const f of work?.fix_ready?.fixes ?? []) out.delete(f.clip);
  return out;
}

/** Whether a clip agent (or the driver) is on the clip, alone or as one of its group. */
export function agentOn(bg: BgState | undefined, work: WorkState | undefined, clip: string): boolean {
  const busy = (s?: string) => s === "starting" || s === "working";
  if ((bg?.clip_agents ?? []).some((a) => busy(a.state) && (a.clip === clip || (a.clips ?? []).includes(clip)))) return true;
  if (work?.clip_agent && busy(work.clip_agent.state) && work.clip_agent.clip === clip) return true;
  return (work?.working ?? []).some((w) => w.clip === clip);
}

/** The job running on the clip now, if one is. */
export function jobOn(bg: BgState | undefined, clip: string) {
  return (bg?.jobs ?? (bg?.job ? [bg.job] : [])).find((j) => j.clip === clip);
}

/** Whether the clip got better lately: a job just done raised its score, or this page saw it rise. */
export function gotBetter(ctx: Ctx, clip: string, trend?: number[]): boolean {
  const d = [...(ctx.bg?.done ?? [])].reverse().find((x) => x.clip === clip);
  if (d && !d.error && d.before != null && d.after > d.before) return true;
  const at = ctx.changed?.[clip];
  if (trend && trend.length > 1 && at !== undefined && ctx.now - at < RECENT_MS) return trend[trend.length - 1] > trend[trend.length - 2];
  return false;
}

/** Whether the loop is stuck on the clip: no numbers made it match, and nobody is on it. */
export function isStuck(ctx: Ctx, clip: string, score?: ClipScore): boolean {
  if (score?.exact) return false;
  return !!ctx.bg?.stuck?.[clip];
}

/** Whether the clip's score changed lately: a job just done on it, or this page saw it move. */
export function changedRecently(ctx: Ctx, clip: string): boolean {
  if ((ctx.bg?.done ?? []).some((d) => d.clip === clip)) return true;
  const at = ctx.changed?.[clip];
  return at !== undefined && ctx.now - at < RECENT_MS;
}

/**
 * A thumbnail's badges, at most two, the most urgent first: a fix ready
 * to merge (which is never needs you), a note just resolved, needs you,
 * then working (agent, fixing or scoring), then got better, then stuck
 * (only when nothing else is on it).
 */
export function clipBadges(ctx: Ctx, c: Pick<StripClip, "id" | "score">, trend?: number[], needs?: Set<string>): Badge[] {
  const out: Badge[] = [];
  const ready = !!fixReady(ctx.work, c.id);
  if (ready) out.push({ kind: "fix-ready", label: "fix ready" });
  if (justResolved(ctx.work, c.id, ctx.now)) out.push({ kind: "resolved", label: "your note" });
  const needsYou = !ready && (needs ? needs.has(c.id) : clipQuestions(ctx.bg, c.id).needsYou.length > 0 || agentAsks(ctx.work, c.id).length > 0);
  if (needsYou) out.push({ kind: "needs-you", label: "needs you" });
  const job = jobOn(ctx.bg, c.id);
  if (ready) {
    // Nothing else says who is working while a fix waits.
  } else if (agentOn(ctx.bg, ctx.work, c.id)) out.push({ kind: "agent", label: "agent" });
  else if (job?.kind === "refine") out.push({ kind: "fixing", label: "fixing" });
  else if (job?.kind === "score") out.push({ kind: "scoring", label: "scoring" });
  // The person's own note, not amber: it waits for a reply.
  if (openTells(ctx.work, c.id).length) out.push({ kind: "tell", label: "told" });
  if (out.length < 2 && gotBetter(ctx, c.id, trend)) out.push({ kind: "better", label: "better" });
  if (out.length === 0 && isStuck(ctx, c.id, c.score)) out.push({ kind: "stuck", label: "stuck" });
  return out.slice(0, 2);
}

/** In progress: a job or an agent is on it, it needs the person, or it changed recently. */
export function inProgress(ctx: Ctx, c: Pick<StripClip, "id">, needs?: Set<string>): boolean {
  const n = needs ?? needsYouClips(ctx.bg, ctx.work);
  return n.has(c.id) || !!fixReady(ctx.work, c.id) || openTells(ctx.work, c.id).length > 0 || !!jobOn(ctx.bg, c.id) || agentOn(ctx.bg, ctx.work, c.id) || changedRecently(ctx, c.id);
}

/**
 * The thumbnails in order: needs you first (it must be impossible to
 * miss), then the ones being worked on, then those that got better, then
 * the rest changed recently, newest first; the clip ID breaks ties.
 */
export function thumbOrder<T extends { clip: Pick<StripClip, "id" | "score"> }>(ctx: Ctx, cards: T[], trends: Record<string, number[]> = {}): T[] {
  const needs = needsYouClips(ctx.bg, ctx.work);
  const rank = (c: T): [number, number] => {
    const id = c.clip.id;
    if (fixReady(ctx.work, id)) return [0, -1];
    if (needs.has(id)) return [0, 0];
    if (jobOn(ctx.bg, id) || agentOn(ctx.bg, ctx.work, id) || openTells(ctx.work, id).length) return [1, 0];
    if (gotBetter(ctx, id, trends[id])) return [2, -(ctx.changed?.[id] ?? 0)];
    return [3, -(ctx.changed?.[id] ?? 0)];
  };
  return cards
    .filter((c) => inProgress(ctx, c.clip, needs))
    .map((c) => ({ c, r: rank(c) }))
    .sort((a, b) => a.r[0] - b.r[0] || a.r[1] - b.r[1] || a.c.clip.id.localeCompare(b.c.clip.id))
    .map((x) => x.c);
}

export type Status = "fix ready" | "needs you" | "being fixed" | "matches" | "close" | "differs" | "waiting" | "can't start";

/** A clip's status chip on its detail. */
export function clipStatus(ctx: Ctx, c: Pick<StripClip, "id" | "score">): Status {
  if (fixReady(ctx.work, c.id)) return "fix ready";
  if (clipQuestions(ctx.bg, c.id).needsYou.length || agentAsks(ctx.work, c.id).length) return "needs you";
  if (jobOn(ctx.bg, c.id)?.kind === "refine" || agentOn(ctx.bg, ctx.work, c.id)) return "being fixed";
  const s = c.score;
  if (!s) return "waiting";
  if (s.error) return "can't start";
  if (s.exact) return "matches";
  return s.permille >= 900 ? "close" : "differs";
}

/** How much of a clip matches, 0 to 1000: exact is all of it. */
export function matchPermille(s: ClipScore | undefined): number | undefined {
  if (!s || s.error) return undefined;
  return s.exact ? 1000 : s.permille;
}

/** Where the two part: the first wrong frame and one line naming what parts ("Mario's speed across"). */
export function whereItParts(g: GameWords | undefined, s: (ClipScore & { drift_path?: string | null }) | undefined): { frame: number; what: string } | undefined {
  if (!s || s.exact || s.error) return undefined;
  const tracks = [...(s.tracks ?? [])].sort((a, b) => a.frame - b.frame);
  const t = tracks[0];
  if (t) return { frame: t.frame, what: measureName(g, t.path) };
  if (s.drift != null) return { frame: s.drift, what: s.drift_path ? measureName(g, s.drift_path) : "the two games" };
  return undefined;
}

/** An attempt as one row: its title, the clip before and after (0 to 1000), and how many other clips got better or worse. */
export function attemptRow(a: WorkAttempt): { title: string; before?: number; after?: number; better: number; worse: number; busy: boolean } {
  const pm = (x: WorkAttempt["before"]) => (x ? (x.exact ? 1000 : x.permille) : undefined);
  const failed = (a.model_says ?? "").startsWith("It failed") || (a.model_says ?? "").startsWith("Not scored");
  return { title: a.title.replace(/\.$/, ""), before: pm(a.before), after: pm(a.after), better: a.better.length, worse: a.worse.length, busy: !a.after && !failed };
}

/** The fidelity header's exact count: "170 / 255" clips exact. */
export function exactWords(f: { behaviour: { clips: number; exact: number } } | undefined): string {
  return f ? `${f.behaviour.exact} / ${f.behaviour.clips}` : "";
}

/** The rounds worth a row, newest first: a run of rounds that say the same (stuck again and again) shows once. */
export function roundRows<T extends { round?: number; changes?: unknown[]; stuck?: boolean; says: string }>(rounds: T[], n = 3): T[] {
  const out: T[] = [];
  const key = (r: T) => (r.changes?.length ? `c${r.round}` : r.stuck ? "stuck" : r.says);
  for (const r of rounds) {
    if (r.round === undefined) continue;
    if (out.length && key(out[out.length - 1]) === key(r)) continue;
    out.push(r);
    if (out.length >= n) break;
  }
  return out;
}
