// The game view's Scenarios in words and data (docs/inspector.md,
// "Scenarios"): the sequences of presses `gp interp scenarios` runs on both
// games from the original's state at one frame, the state of the job that
// scores or refines them, and the sentence each step says. The server says
// what is happening ("Testing: Jump, held 15 frames (4 of 22)"); this file
// adds the marks, the headline for a thumbnail, the trend and the client.
// Nothing here names a game.

export type SequenceState = "locked" | "open" | "failing" | "passed";

export interface MeasureScore {
  path: string;
  together: number;
  total: number;
  tolerance: string;
  max: string;
  mean: string;
  first?: { frame: number; original: string; model: string };
  bins: { from: string; to: string; frames: number }[];
  owners: string[];
  distance: string[];
}

export interface ScenarioResult {
  id: string;
  exact: boolean;
  score: number;
  measured: number;
  original_from: number;
  original_to: number;
  first?: { frame: number; original_frame: number; path: string; original: string; model: string; distance: string; mechanic: string; paths: string[] };
  says: string;
  measures: MeasureScore[];
  headline: { exact: number; measures: number; together_permille: number; says: string };
  moved: string[];
}

export interface Scenario {
  id: string;
  name: string;
  tier: number;
  mechanic: string;
  params: { path: string; value: string }[];
  frames: number;
  presses: string[];
  requires: string[];
  log?: string;
  start?: { input: string; frame: number; snapshot?: string };
  source: { kind: "generated" } | { kind: "played"; when: string; by: string };
}

export interface LadderRow {
  scenario: Scenario;
  state: SequenceState;
  result: ScenarioResult;
  says: string;
  needs: string;
  unlocks: string;
  history: { model: string; label: string; together_permille: number; exact: boolean }[];
  trend: string | null;
  video: { input: string; from: number; to: number };
}

export interface Ladder {
  state: "done";
  says: string;
  bar_says: string;
  start: { input: string; asked: number | null; original_frame: number; frame: number; seconds: number; says: string; loaded: number; approximated: number; missing: number };
  next: string | null;
  start_here: string | null;
  first_failing: string | null;
  tiers: { tier: number; exact: number; passed: number; total: number }[];
  passed: number;
  locked: number;
  scenarios: LadderRow[];
  model: string | null;
}

export type JobState = "idle" | "running" | "done" | "stuck" | "stopped" | "failed";

/** The run's scenarios job, as `GET /api/capture/{run}/scenarios` gives it. */
export interface ScenariosState {
  recording: string;
  state: JobState;
  job?: "score" | "refine" | "auto";
  says: string;
  progress?: { k: number; n: number; scenario: string; name: string } | null;
  ladder?: Ladder;
  refine?: { scenario: string; rounds: { round: number; param: string; old: string; new: string }[]; trying: { param: string; value: string; mechanic: string } | null; result?: RefineResult };
  auto?: { says?: string; steps?: { scenario: string; says: string; passes: boolean }[] };
  error?: string;
}

export interface RefineResult {
  scenario: string;
  name: string;
  says: string;
  passes: boolean;
  changed: boolean;
  also_fixed: string[];
  worse: string[];
}

/** The original's side of one sequence: the input to its start, then its presses. */
export interface ScenarioInput {
  scenario: Scenario;
  start_frame: number;
  model_frame: number;
  frames: number;
  log: string;
  state: string;
}

/** A sequence's mark in words. */
export function mark(s: SequenceState): string {
  switch (s) {
    case "passed":
      return "matches";
    case "failing":
      return "doesn't match";
    case "locked":
      return "locked";
    case "open":
      return "not run yet";
  }
}

/** The mark for a row, with "being worked on" while a refine or auto job is on it. */
export function rowMark(row: LadderRow, st?: ScenariosState): string {
  if (st?.state === "running" && (st.refine?.scenario === row.scenario.id || st.progress?.scenario === row.scenario.id)) return "being worked on";
  return mark(row.state);
}

/** The thumbnail's line: "9/10 measures, 96%". */
export function headline(r: ScenarioResult): string {
  return r.headline.says;
}

/** Tiers in words. */
export function tierName(t: number): string {
  switch (t) {
    case 0:
      return "Played by you";
    case 1:
      return "Each button alone";
    case 2:
      return "Buttons together";
    case 3:
      return "With directions";
    default:
      return `Tier ${t}`;
  }
}

/** The rows grouped by tier, in ladder order. */
export function byTier(l: Ladder): { tier: number; rows: LadderRow[]; exact: number; total: number }[] {
  const out: { tier: number; rows: LadderRow[]; exact: number; total: number }[] = [];
  for (const r of l.scenarios) {
    let t = out.find((x) => x.tier === r.scenario.tier);
    if (!t) {
      const n = l.tiers.find((x) => x.tier === r.scenario.tier);
      t = { tier: r.scenario.tier, rows: [], exact: n?.exact ?? 0, total: n?.total ?? 0 };
      out.push(t);
    }
    t.rows.push(r);
  }
  return out;
}

/** The frame a thumbnail shows, the original's: the first split, else the end of the presses. */
export function tellingFrame(row: LadderRow): number {
  const f = row.result.first;
  if (f) return f.original_frame;
  return Math.max(row.result.original_from, row.result.original_to - 30);
}

/** The sentence for the job's state: the server's, or one for a state it hasn't written. */
export function jobSays(st: ScenariosState | undefined): string {
  if (!st) return "Loading.";
  if (st.state === "failed") return st.says || `It failed: ${st.error ?? ""}`;
  return st.says;
}

/** The next step's button label. */
export function nextAction(st: ScenariosState | undefined): "run" | "refine" | "record" | "wait" {
  if (!st || st.state === "idle" || st.state === "failed" || st.state === "stopped") return "run";
  if (st.state === "running") return "wait";
  if (st.ladder?.next) return "refine";
  return "record";
}

/** The row the ladder says to work on next. */
export function nextRow(l: Ladder | undefined): LadderRow | undefined {
  return l?.scenarios.find((r) => r.scenario.id === l.next);
}

/** The scenarios endpoints, on the page's own server. */
export class ScenarioClient {
  constructor(
    private readonly run: string,
    private readonly base = "",
    private readonly fetchFn: typeof fetch = (...a) => fetch(...a),
  ) {}
  private url(p: string): string {
    return `${this.base}/api/capture/${encodeURIComponent(this.run)}/scenarios${p}`;
  }
  private async json<T>(url: string, init?: RequestInit): Promise<T> {
    const r = await this.fetchFn(url, init);
    if (!r.ok) throw new Error(`${r.status}: ${await r.text()}`);
    return (await r.json()) as T;
  }
  private post<T>(p: string, body: unknown): Promise<T> {
    return this.json(this.url(p), { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
  }
  state(recording: string): Promise<ScenariosState> {
    return this.json(this.url(`?recording=${encodeURIComponent(recording)}`));
  }
  score(recording: string, at?: number): Promise<{ started: boolean; why?: string }> {
    return this.post("", { recording, at });
  }
  refine(recording: string, scenario: string, at?: number): Promise<{ started: boolean; why?: string }> {
    return this.post("/refine", { recording, scenario, at });
  }
  auto(recording: string, at?: number): Promise<{ started: boolean; why?: string }> {
    return this.post("/auto", { recording, at });
  }
  stop(): Promise<{ stopping: boolean }> {
    return this.post("/stop", {});
  }
  played(recording: string, at: number, log: string, by = "player"): Promise<{ saved: boolean; scenario: Scenario }> {
    return this.post("/played", { recording, at, log, by });
  }
  input(recording: string, scenario: string): Promise<ScenarioInput> {
    return this.json(this.url(`/input?recording=${encodeURIComponent(recording)}&scenario=${encodeURIComponent(scenario)}`));
  }
}

/** How long a recorded sequence runs, unless set: 5 seconds. */
export const RECORD_SECONDS = 5;

const ORDER = "RLDUTSBA";

/** Recorded presses as an input log (gp-input-log text), one line a frame: `P` when the game read the pad. */
export function playedLog(frames: { pad: number; polled: boolean }[]): string {
  const line = (pad: number) => [...ORDER].map((l, i) => ((pad >> (7 - i)) & 1 ? l : ".")).join("");
  return ["gp-input-log 0", "controllers 1", ...frames.map((f) => `${f.polled ? "P" : "-"} ${line(f.pad)}`)].join("\n") + "\n";
}

/** Which input is live while recording. */
export function inputHint(gamepad: boolean): string {
  return gamepad ? "Gamepad connected" : "Keyboard: arrows, X = A, Z = B";
}

/** The recording's countdown and progress in words. */
export function recordSays(phase: "ready" | "count" | "recording" | "saving" | "saved", n: number, of: number): string {
  switch (phase) {
    case "ready":
      return `Record up to ${Math.round(of / 60)} seconds from here.`;
    case "count":
      return `Starting in ${n}...`;
    case "recording":
      return `Recording: ${((of - n) / 60).toFixed(1)} s left.`;
    case "saving":
      return "Saving the sequence.";
    case "saved":
      return "Saved. Next: run the scenarios to compare it.";
  }
}
