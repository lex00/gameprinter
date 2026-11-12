// Claude's work on a run's clips (docs/agents.md, "Working on a clip in the
// loop"): the run's work record from gp serve, with notes and attempts by
// the driver (the coordinator in its own lane worktrees) or the clip agent
// gp serve starts on a clip. Its sentences for the clip card's Agent
// section and the progress panel, and its client. Pure but for the
// client, so the tests run it on made-up records.

import type { Api } from "./api";

/** A clip's score in an attempt. */
export interface WorkSide {
  permille: number;
  score: number;
  exact: boolean;
  first_wrong?: number | null;
  first_path?: string | null;
  says: string;
  error?: string | null;
}

export interface WorkAttempt {
  n: number;
  by: string;
  clip: string;
  clip_name?: string;
  title: string;
  says?: string;
  before?: WorkSide | null;
  after?: WorkSide | null;
  better: string[];
  worse: string[];
  others: number;
  /** The trial preview's folder under the run, when it was built. */
  preview?: string | null;
  preview_error?: string | null;
  branch?: string | null;
  commit?: string | null;
  dirty?: boolean;
  /** It changes how a mechanic works, so it needs the person's OK. */
  shape?: boolean;
  model_says?: string;
  approved?: boolean;
  when_ms: number;
  /** The server's line: "Attempt 2: ... Clip: 62% -> 81%; others: 3 better, none worse." */
  line?: string;
  keepable?: boolean;
}

export interface WorkEntry {
  kind: string;
  by: string;
  clip?: string | null;
  text?: string;
  on?: string | null;
  attempt?: number | null;
  id?: number | null;
  reply_to?: number | null;
  when_ms: number;
}

/** What the person told the agent about a clip (docs/agents.md, "Telling the agent"), with the replies under it. */
export interface WorkTell {
  id: number;
  clip: string;
  text: string;
  by: string;
  /** The clip's frame and the original's. */
  frame?: number | null;
  orig_frame?: number | null;
  /** A spot [x, y] or a region [x, y, w, h], in game pixels. */
  at?: [number, number] | null;
  region?: [number, number, number, number] | null;
  when_ms: number;
  /** No reply yet. */
  open: boolean;
  /** A clip agent is on the clip now, so it reads it between steps. */
  agent?: boolean;
  replies: { by: string; text: string; when_ms: number }[];
  /** Whether the clip got better since the note (docs/agents.md, "Telling the agent"). */
  credit?: TellCredit;
}

/** `fixed` (a linked attempt is on main), `improved` (better, no linked attempt on main), `pending` (a linked fix waits to be merged) or `open`. */
export interface TellCredit {
  status: "fixed" | "improved" | "pending" | "open";
  words: string;
  before?: { permille: number; score: number; exact: boolean };
  after?: { permille: number; score: number; exact: boolean };
  /** "97% -> exact". */
  change?: string;
  attempt?: number | null;
  frames_gained?: number;
  when_ms?: number;
  /** "USER TELL RESOLVED on <clip>: 97% -> exact, by attempt 117". */
  says?: string;
}

/** The person's notes resolved in the run. */
export interface TellTally {
  resolved: number;
  fixed: number;
  improved: number;
  exact: number;
  frames: number;
  open: number;
}

/** An approved fix that is not on main yet. */
export interface FixReady {
  clip: string;
  attempt: number;
  title: string;
  exact: boolean;
  /** "A fix is ready: attempt 117 makes this clip exact. It's waiting to be merged." */
  says: string;
}

/** What one writer is doing now. */
export interface Working {
  by: string;
  clip?: string | null;
  on?: string | null;
  last?: string | null;
  when_ms: number;
  says: string;
}

/** The clip agent gp serve runs on a clip of the run, when there is one. */
export interface ClipAgent {
  clip: string;
  state: "starting" | "working" | "matched" | "stopped" | "limit" | "failed" | "finished";
  says: string;
  attempts?: number;
  started_ms?: number;
  log?: string;
}

export interface WorkState {
  run: string;
  working: Working[];
  attempts: WorkAttempt[];
  entries: WorkEntry[];
  clip_agent?: ClipAgent | null;
  tells?: WorkTell[];
  tell_tally?: TellTally;
  fix_ready?: { count: number; fixes: FixReady[] };
}

/** How long a thumbnail keeps its star after a note resolved. */
export const STAR_MS = 5 * 60 * 1000;

/** The fix waiting to be merged for a clip, if there is one. */
export function fixReady(s: WorkState | undefined, clip: string): FixReady | undefined {
  return (s?.fix_ready?.fixes ?? []).find((f) => f.clip === clip);
}

/** Whether a note on the clip resolved lately. */
export function justResolved(s: WorkState | undefined, clip: string, now: number): boolean {
  return (s?.tells ?? []).some((t) => t.clip === clip && (t.credit?.status === "fixed" || t.credit?.status === "improved") && t.credit.when_ms !== undefined && now - t.credit.when_ms < STAR_MS);
}

/** The header's quiet counters: "your notes: 4 fixed" and "2 fixes ready to merge"; none when there is nothing to say. */
export function headerCounts(s: WorkState | undefined): { notes?: string; ready?: string } {
  const out: { notes?: string; ready?: string } = {};
  const t = s?.tell_tally;
  if (t && t.resolved > 0) out.notes = t.improved > 0 ? `your notes: ${t.fixed} fixed, ${t.improved} improved` : `your notes: ${t.fixed} fixed`;
  const n = s?.fix_ready?.count ?? 0;
  if (n > 0) out.ready = `${n} ${n === 1 ? "fix" : "fixes"} ready to merge`;
  return out;
}

const enc = encodeURIComponent;

/** The body of a tell. */
export interface TellBody {
  text: string;
  frame?: number;
  orig_frame?: number;
  at?: [number, number];
  region?: [number, number, number, number];
}

/** A tell's body from what the person typed and where they pointed: a click is a spot (the middle of its box), a drag a region. */
export function tellBody(text: string, pin?: { box: { x: number; y: number; w: number; h: number }; click: boolean; frame: number; orig?: number }): TellBody {
  const body: TellBody = { text: text.trim() };
  if (!pin) return body;
  body.frame = pin.frame;
  if (pin.orig !== undefined) body.orig_frame = pin.orig;
  const b = pin.box;
  if (pin.click) body.at = [Math.round(b.x + b.w / 2), Math.round(b.y + b.h / 2)];
  else body.region = [b.x, b.y, b.w, b.h];
  return body;
}

/** Where the person pointed, in words. */
export function pinWords(pin: { box: { x: number; y: number; w: number; h: number }; click: boolean; frame: number }): string {
  const b = pin.box;
  const where = pin.click ? `the spot (${Math.round(b.x + b.w / 2)}, ${Math.round(b.y + b.h / 2)})` : `a ${b.w}x${b.h} region at (${b.x}, ${b.y})`;
  return `${where}, frame ${pin.frame}`;
}

/** The tells on a clip, oldest first. */
export function clipTells(s: WorkState | undefined, clip: string): WorkTell[] {
  return (s?.tells ?? []).filter((t) => t.clip === clip).sort((a, b) => a.id - b.id);
}

/** The tells with no reply yet. */
export function openTells(s: WorkState | undefined, clip?: string): WorkTell[] {
  return (s?.tells ?? []).filter((t) => t.open && (clip === undefined || t.clip === clip));
}

export class WorkClient {
  constructor(private readonly api: Api) {}
  state(run: string): Promise<WorkState> {
    return this.api.json(`/api/capture/${enc(run)}/agent`);
  }
  approve(run: string, attempt: number): Promise<WorkState> {
    return this.api.post(`/api/capture/${enc(run)}/agent/approve`, { attempt });
  }
  /** Starts the clip agent on a clip. */
  start(run: string, clip: string): Promise<WorkState> {
    return this.api.post(`/api/capture/${enc(run)}/clips/${enc(clip)}/agent`, {});
  }
  /** Tells the agent something about a clip, with the frame and the spot or region pointed at. */
  tell(run: string, clip: string, body: TellBody): Promise<{ tell: WorkTell; delivered: boolean; tells: WorkTell[] }> {
    return this.api.post(`/api/capture/${enc(run)}/clips/${enc(clip)}/tell`, body);
  }
  stop(run: string): Promise<WorkState> {
    return this.api.post(`/api/capture/${enc(run)}/agent/stop`, {});
  }
  /** The attempt's trial preview, in control mode. */
  previewUrl(run: string, n: number): string {
    return this.api.url(`/api/capture/${enc(run)}/agent/attempts/${n}/preview/index.html?control=1`);
  }
  curvePath(run: string, n: number): string {
    return `/api/capture/${enc(run)}/agent/attempts/${n}/curve`;
  }
}

/** "the driver" or "the clip agent". */
export function byWords(by: string): string {
  return by === "you" || by === "user" ? "you" : `the ${by}`;
}

/** The writers working on this clip now (or on the run with no clip named). */
export function workingOn(s: WorkState | undefined, clip: string): Working[] {
  return (s?.working ?? []).filter((w) => !w.clip || w.clip === clip);
}

export function clipAttempts(s: WorkState | undefined, clip: string): WorkAttempt[] {
  return (s?.attempts ?? []).filter((a) => a.clip === clip);
}

/** Whether an attempt is still being built and scored. */
export function attemptBusy(a: WorkAttempt): boolean {
  return !a.after && !(a.model_says ?? "").startsWith("It failed") && !(a.model_says ?? "").startsWith("Not scored");
}

/** An attempt in a line, with what happens to it now. */
export function attemptLine(a: WorkAttempt): string {
  const head = `Attempt ${a.n}: ${a.title.replace(/\.$/, "")}.`;
  if (a.after) return a.line ?? head;
  const m = a.model_says ?? "";
  if (m.startsWith("It failed")) return `${head} ${m}`;
  if (m.startsWith("Not scored")) return `${head} Not scored.`;
  return `${head} Building it and scoring the clips with the change now.`;
}

/** What keep says once pressed. */
export const APPROVED = "Approved; waiting to be merged into the game (the driver merges and restarts).";

/** The note under an attempt: approved, needs your OK, or why it can't be kept. */
export function attemptNote(a: WorkAttempt): string | undefined {
  if (a.approved) return APPROVED;
  if (!a.after) return undefined;
  if (a.keepable) return a.shape ? "This changes how a mechanic works, so it needs your OK: keep gives it." : undefined;
  if (a.worse.length) return "It makes other clips worse, so it can't be kept as it is.";
  return "It doesn't make the clip better.";
}

/** The clip agent's line on the card: "Agent: trying 'facing only turns on the ground' (attempt 2)". */
export function agentLine(a: ClipAgent | null | undefined, clip: string): { says: string; busy: boolean } | undefined {
  if (!a || a.clip !== clip) return undefined;
  return { says: a.says, busy: a.state === "starting" || a.state === "working" };
}

/** The progress panel's line: who is working on what, with the last thing done. */
export function progressLines(s: WorkState | undefined): { says: string; by: string }[] {
  return (s?.working ?? []).map((w) => ({ says: w.says, by: byWords(w.by) }));
}
