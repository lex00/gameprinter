// The background work's side of the page (docs/show.md, "The background
// work"): gp serve's queue on a run, which scores every kept clip and
// fixes numbers on the one that drifts apart earliest. Its state in one
// sentence for the status line, its rounds in words, and its client. Pure
// but for the client, so the tests run it on made-up states.

import type { Api, GameWords } from "./api";
import { pathPhrase } from "./phrase";
import { questionNeedsYou } from "./calmfold";
import type { ClipScore } from "./stripfold";

export interface BgScore extends ClipScore {
  model: string;
  score: number;
  drift_path?: string | null;
}

export interface BgQuestion {
  id: string;
  clip: string;
  clip_name: string;
  at?: number | null;
  ask: string;
  choices: string[];
  detail: string;
  model: string;
  answer?: string | null;
  /** `shape` (no numbers fix it) or `conflict` (the code and the fit disagree); none on older questions. */
  kind?: string;
  /** The measure wrong there. */
  path?: string;
}

/** A clip agent the queue handed stuck clips to (`serve/clip_agent.rs`): one clip and the clips stuck on the same thing. */
export interface BgClipAgent {
  clip: string;
  clips?: string[];
  state: string;
  /** "Mario's speed across". */
  about?: string;
  /** "Agent 2: Mario's speed across (3 clips). Looking at ...". */
  row?: string;
  says?: string;
  slot?: number;
  attempts?: number;
  started_ms?: number;
}

export interface BgChange {
  param: string;
  old: string;
  new: string;
  mechanic?: string;
}

/** One line of the history: a refine round, or an answer. */
export interface BgRound {
  round?: number;
  when_ms: number;
  clip?: string;
  clip_name?: string;
  changes?: BgChange[];
  fixed?: number;
  worse?: string[];
  passes?: boolean;
  stuck?: boolean;
  stopped?: boolean;
  /** The measure the round worked on: the original's values and the rebuilt game's before and after. */
  curve?: { path: string; words: string; original?: string[] | null; before?: string[] | null; after?: string[] | null } | null;
  answer?: string;
  choice?: string;
  says: string;
}

/**
 * A job running now, with how far it is (`serve/workers.rs`): a score's
 * frames compared, measures exact and first wrong frame so far; a refine's
 * pass and trial and its best split count against the start.
 */
export interface BgJob {
  kind: "score" | "refine" | "make";
  clip: string;
  name: string;
  model?: string;
  slot?: number;
  k?: number;
  n?: number;
  round?: number;
  frames?: number;
  total?: number;
  together?: number;
  measures?: number;
  exact?: number;
  first_wrong?: number | null;
  pass?: "impact" | "fine";
  impact_k?: number;
  impact_n?: number;
  fine_k?: number;
  fine_knobs?: number;
  base?: number | null;
  best?: number | null;
  kept?: number;
  target?: { frame?: number; says?: string; measures?: { path: string }[] } | null;
  impact?: { says?: string } | null;
  trying?: { param: string; value: string; words?: string; pass?: string; split?: number; curve?: { path?: string | null; frames?: number[]; values?: (number | null)[] } };
}

/** A job just done, with its clip's score before and after. */
export interface BgDone {
  seq: number;
  kind: "score" | "refine";
  clip: string;
  name: string;
  before?: number | null;
  after: number;
  exact: boolean;
  error?: string | null;
  says?: string;
}

export interface BgState {
  run?: string;
  enabled: boolean;
  working?: boolean;
  recording?: string | null;
  scores: Record<string, BgScore>;
  questions: BgQuestion[];
  round?: number;
  /** The first job running now (all of them are in `jobs`). */
  job?: BgJob | null;
  says?: string;
  error?: string | null;
  /** The clip the person asked to fix first. */
  focus?: string | null;
  /** Every job running now; `job` is the first of them. */
  jobs?: BgJob[];
  /** The jobs just done, newest last. */
  done?: BgDone[];
  /** How many jobs work at once, and the most this machine allows. */
  workers?: number;
  max_workers?: number;
  /** Why no new job starts, when memory holds them. */
  holding?: string;
  /** The measurement checks on the original: while they fail, no number is fitted. */
  check?: { passed: boolean; says: string; failures?: string[]; clips?: number } | null;
  /** The last replay of the run's ledger (`gp capture verify`): "verified: 120 of 120 results reproduce", and each that didn't. */
  verified?: { checked: number; reproduced: number; failures: string[]; says: string } | null;
  /** The clips the tool makes by itself: making them now, and what it does ("Generating: World 1-3, 12 of 40."). */
  generating?: { active: boolean; finished?: boolean; says?: string; error?: string | null } | null;
  /** The clip agents working now. */
  clip_agents?: BgClipAgent[];
  /** Why no clip agent works on the stuck active clips, in one line, when none does. */
  agents_idle?: string | null;
  /** Everything holding work back: what, since when, and the fix. */
  holds?: { kind: string; says: string; since_secs?: number | null; fix: string }[];
  /** The IDs of the active clips, those the queue works on; `scores` and `questions` hold only theirs. */
  active?: string[];
  /** The active clips counted: how many, scored and exact, and every score the file holds. */
  totals?: { active: number; scored: number; exact: number; stored: number };
  /** Clips where no numbers made it match, by clip, with the model it stuck on. */
  stuck?: Record<string, string>;
  /** How many clip agents work at once, and the most. */
  agents?: number;
  max_agents?: number;
}

const enc = encodeURIComponent;

export class BackgroundClient {
  constructor(private readonly api: Api) {}
  state(run: string): Promise<BgState> {
    return this.api.json(`/api/capture/${enc(run)}/background`);
  }
  history(run: string): Promise<{ rounds: BgRound[] }> {
    return this.api.json(`/api/capture/${enc(run)}/background/history`);
  }
  /** Turns the queue on; with a clip, that clip is fixed first. */
  start(run: string, clip?: string): Promise<BgState> {
    return this.api.post(`/api/capture/${enc(run)}/background/start`, clip ? { clip } : {});
  }
  stop(run: string): Promise<BgState> {
    return this.api.post(`/api/capture/${enc(run)}/background/stop`, {});
  }
  /** How many jobs work at once. */
  setWorkers(run: string, workers: number): Promise<{ workers: number; max: number }> {
    return this.api.post(`/api/capture/${enc(run)}/background/workers`, { workers });
  }
  /** Replays the run's ledger in the background: all of it, or a sample of `sample` results. */
  verify(run: string, sample?: number): Promise<BgState> {
    return this.api.post(`/api/capture/${enc(run)}/background/verify`, sample ? { sample } : {});
  }
  answer(run: string, question: string, choice: string): Promise<BgState> {
    return this.api.post(`/api/capture/${enc(run)}/background/answer`, { question, choice });
  }
}

/** The questions not answered yet. */
export function openQuestions(s: BgState | undefined): BgQuestion[] {
  return (s?.questions ?? []).filter((q) => !q.answer);
}

/** Rounds since local midnight of `now` that fixed a clip. */
export function fixedToday(history: BgRound[], now: number): number {
  const d = new Date(now);
  const midnight = new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  return history.filter((r) => r.when_ms >= midnight && r.round !== undefined).reduce((n, r) => n + (r.fixed ?? 0), 0);
}

/** A change in words: "Frog's gravity 0.1875 -> 0.25". */
export function changeWords(g: GameWords | undefined, c: BgChange): string {
  return `${pathPhrase(g, c.param)} ${c.old} -> ${c.new}`;
}

/** A round in words: "Round 12: Mario's pull down 0.1875 -> 0.15625, 6 clips fixed, none worse." */
export function roundWords(g: GameWords | undefined, r: BgRound): string {
  if (r.round === undefined) return r.says;
  const clips = (n: number) => (n === 1 ? "1 clip" : `${n} clips`);
  if (!r.changes?.length) return r.stuck ? `Round ${r.round}: no numbers make ${r.clip_name ?? "the clip"} match; a question for you.` : r.says;
  const worse = r.worse?.length ? `${clips(r.worse.length)} worse` : "none worse";
  return `Round ${r.round}: ${r.changes.map((c) => changeWords(g, c)).join(", ")}, ${clips(r.fixed ?? 0)} fixed, ${worse}.`;
}

/**
 * The status line: "Background: scoring clip 14 of 52; 3 fixed today; 1
 * question for you". Paused or off says so with the next thing to do.
 * While the tool makes clips, its progress comes first: "Generating:
 * World 1-3, 12 of 40. Background: ...".
 */
export function statusSentence(s: BgState | undefined, history: BgRound[], now: number, g?: GameWords): string {
  if (!s) return "Background: checking.";
  const parts: string[] = [];
  const j = s.working ? s.job : undefined;
  if (s.error && !j) parts.push(`stopped on an error: ${s.error}`);
  else if (!s.enabled && !j) parts.push(history.length ? "paused" : "off; turn it on to score your clips and fix numbers while you play");
  else if ((s.jobs?.length ?? 0) > 1) parts.push(`${s.jobs!.length} jobs at once: ${s.jobs!.map((x) => (x.kind === "score" ? `scoring ${x.name}` : x.kind === "refine" ? `fixing ${x.name}` : "making clips")).join(", ")}`);
  else if (j?.kind === "score") parts.push(`scoring clip ${j.k} of ${j.n}`);
  else if (j?.kind === "refine") parts.push(`round ${j.round}: fixing numbers on ${j.name}${j.trying ? ` (trying ${pathPhrase(g, j.trying.param)} = ${j.trying.value})` : ""}`);
  else {
    const scored = Object.values(s.scores);
    const differ = scored.filter((x) => !x.exact).length;
    parts.push(scored.length ? (differ ? `waiting; ${differ} of ${scored.length} clips differ` : `all ${scored.length} clips match`) : "waiting for clips");
  }
  const fixed = fixedToday(history, now);
  if (fixed) parts.push(`${fixed} fixed today`);
  // Only the questions that need a person (calmfold.ts); the loop's generic ones go to the clip agents.
  const q = openQuestions(s).filter(questionNeedsYou).length;
  if (q) parts.push(q === 1 ? "1 question for you" : `${q} questions for you`);
  // Why no clip agent works on the stuck clips, when none does.
  if (s.holds?.length) for (const h of s.holds) parts.push(h.says.replace(/\.$/, ""));
  else if (s.agents_idle) parts.push(s.agents_idle.replace(/\.$/, ""));
  // Making clips goes first, at the same pace: "Generating: World 1-3, 12 of 40."
  const gen = s.generating?.active && s.generating.says ? `${s.generating.says} ` : "";
  return `${gen}Background: ${parts.join("; ")}`;
}

/** Scores as the strip reads them, by clip ID. */
export function stripScores(s: BgState | undefined): Record<string, ClipScore> {
  return s?.scores ?? {};
}
