// Free play (docs/show.md, "Free play"): gp serve's side of a session and
// the panel's pure helpers. The page plays the original live and keeps
// each save point's state bytes for fast loading; the server keeps each
// branch's input from power-on and each save point's input log, so every
// save point is rebuilt natively from its log.

import type { Api, StartState } from "./api";
import type { Scenario } from "./scenariofold";
import type { ClipMeta, Mark } from "./stripfold";
import { NES_FPS } from "./playfold";

export interface FreeBranch {
  id: number;
  from_save: number | null;
  from_frame: number;
  frames: number;
}

export interface FreeSave {
  id: number;
  branch: number;
  frame: number;
  hash: string;
  digest: string;
  made_ms: number;
  picture: boolean;
  /** The server's rebuild from the save point's log, once it's done. */
  native?: { hash: string; digest: string; matches: boolean } | null;
  error?: string | null;
}

export interface FreeSession {
  schema: number;
  id: string;
  run: string;
  runs: string[];
  recording: string | null;
  frame: number;
  started_ms: number;
  branches: FreeBranch[];
  saves: FreeSave[];
  clips: Scenario[];
  /** Each clip's place and mark, by clip ID. */
  clip_meta?: Record<string, ClipMeta>;
}

/** Where a clip was cut from, sent with it. */
export interface ClipPlace {
  branch: number;
  from: number;
  cut: "auto" | "manual";
  ended: "save" | "load" | "cap" | "stop";
}

/** The start state of a branch's frame, or why there's none yet. */
export type FreeStart = ({ ready: true } & StartState) | { ready: false; asked: number; why: string };

/** Frame lines sent to the server at a time: about 22 KB, under its 64 KB body limit. */
export const UPLOAD_LINES = 2000;

/** Frames played past a save point before the rebuilt game's start is asked for: the start can be a frame or two after it. */
export const FOLLOW_AFTER = 20;

/** How long a clip runs at most, in seconds, unless the setting says otherwise. */
export const CLIP_SECONDS = 8;

const enc = encodeURIComponent;

export class FreePlayClient {
  constructor(private readonly api: Api) {}
  sessions(run: string): Promise<{ sessions: FreeSession[] }> {
    return this.api.json(`/api/play/free?run=${enc(run)}`);
  }
  session(id: string): Promise<{ session: FreeSession }> {
    return this.api.json(`/api/play/free/${enc(id)}`);
  }
  start(run: string, recording: string | undefined, frame: number): Promise<{ session: FreeSession }> {
    return this.api.post("/api/play/free", recording ? { run, recording, frame } : { run });
  }
  branch(id: string, fromSave?: number): Promise<{ branch: FreeBranch }> {
    return this.api.post(`/api/play/free/${enc(id)}/branches`, fromSave === undefined ? {} : { from_save: fromSave });
  }
  log(id: string, branch: number, offset: number, lines: string): Promise<{ frames: number }> {
    return this.api.post(`/api/play/free/${enc(id)}/branches/${branch}/log`, { offset, lines });
  }
  save(id: string, s: { branch: number; frame: number; hash: string; digest: string; picture?: string }): Promise<{ save: FreeSave }> {
    return this.api.post(`/api/play/free/${enc(id)}/saves`, s);
  }
  saveLog(id: string, k: number): Promise<string> {
    return this.api.text(`/api/play/free/${enc(id)}/saves/${k}/input.log`);
  }
  /** A save point's player state, rebuilt by the server from its input log and checked against its hash. */
  state(id: string, k: number): Promise<Uint8Array> {
    return this.api.bytes(`/api/play/free/${enc(id)}/saves/${k}/state`);
  }
  /** Keeps, skips or stars a clip. */
  mark(id: string, clip: string, mark: Mark): Promise<{ session: FreeSession }> {
    return this.api.post(`/api/play/free/${enc(id)}/clips/${enc(clip)}`, { mark });
  }
  picture(id: string, k: number): string {
    return this.api.url(`/api/play/free/${enc(id)}/saves/${k}/picture.png`);
  }
  startState(id: string, branch: number, at: number): Promise<FreeStart> {
    return this.api.post(`/api/play/free/${enc(id)}/start-state`, { branch, at });
  }
  include(id: string, run: string): Promise<{ session: FreeSession }> {
    return this.api.post(`/api/play/free/${enc(id)}/include`, { run });
  }
  /** Saves a take from a save point as a clip of the run's scenarios; `replace` records it again over a clip. */
  clip(run: string, save: string, log: string, replace?: string, place?: ClipPlace): Promise<{ saved: boolean; scenario: Scenario }> {
    return this.api.post(`/api/capture/${enc(run)}/scenarios/played`, { save, log, ...(replace ? { replace } : {}), ...(place ?? {}) });
  }
}

/** Minutes and seconds of play at a frame: "0:41". */
export function clock(frame: number): string {
  const s = Math.floor(frame / NES_FPS);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** Seconds, one decimal: "6.2 s". */
export function secs(frames: number): string {
  return `${(frames / NES_FPS).toFixed(1)} s`;
}

/** A frames file line for the rebuilt game: the pad's bits (the input log's bits are the frames file's) and a reset. */
export function modelLine(pad: number, reset = false): string {
  return `${(pad & 0xff).toString(16).padStart(2, "0")}${reset ? " r" : ""}`;
}

/** The latest save point, the one F8 goes back to. */
export function latest(saves: { id: number; made_ms?: number }[]): number | undefined {
  return saves.length ? saves.reduce((a, b) => (b.id > a.id ? b : a)).id : undefined;
}

/** What a save point's check says, in a few words. */
export function checkWords(s: Pick<FreeSave, "native" | "error">): string {
  if (s.error) return "the server couldn't rebuild it";
  if (!s.native) return "checking";
  return s.native.matches ? "rebuilt the same" : "REBUILT DIFFERENTLY";
}

/** The one sentence under the two games in free play: what's happening and the next thing to do. */
export function freeSays(s: { saves: number; follow?: "waiting" | "asking" | "following" | "none"; paused: boolean; model?: boolean }): string {
  if (s.paused) return "Paused. Press P to go on playing.";
  if (s.saves === 0) return "You're playing the original. Press F5 to make a save point when you reach a part you want to look at.";
  if (s.model === false) return "This run has no rebuilt game to follow you. F5 makes another save point; F8 goes back to the last one.";
  if (s.follow === "following") return "The rebuilt game plays your presses from the last save point. F5 makes another save point; F8 goes back to the last one.";
  if (s.follow === "none") return "The rebuilt game can't start from this save point yet (the game isn't under way). Play on and press F5 again.";
  return "The rebuilt game is getting ready to start from the save point. Keep playing.";
}

/** A clip length the setting accepts: 1 to 30 seconds, else the default. */
export function clipLength(v: number): number {
  return Number.isFinite(v) && v >= 1 ? Math.min(30, Math.round(v * 10) / 10) : CLIP_SECONDS;
}

export type TakePhase = "count" | "recording" | "saving" | "saved";

/** The sentence while a clip is made. */
export function takeSays(phase: TakePhase, count: number, n: number, max: number): string {
  if (phase === "count") return `Get ready: ${count}. Then play; the clip records your presses for up to ${secs(max)}.`;
  if (phase === "recording") return `Recording the clip: ${secs(n)} of ${secs(max)}. Press stop to end it early.`;
  if (phase === "saving") return "Saving the clip.";
  return "Clip saved.";
}

/** The card after a take: what was saved and from where. */
export function clipCard(frames: number, save: number, at: number, level?: string): string {
  return `Clip saved: ${secs(frames)} from ${level ? `${level} at ${clock(at)}` : `save point ${save} at ${clock(at)}`}. Watch it on both games, refine the rebuilt game on it, or record it again from the same start.`;
}
