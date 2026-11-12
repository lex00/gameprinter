// gp serve's watch routes (docs/watch.md, crates/gp-cli/src/serve/watch.rs),
// as a client in its own file over the Api's plain requests.

import type { Api, EventSourceLike } from "./api";
import type { WatchEvent } from "./watchfold";

/** A session as `GET /api/watch` lists it. */
export interface WatchSession {
  name: string;
  rom: string;
  events: number;
  writing: boolean;
}

/** L3 for a window of a recipe, as `gp reach --detail --json` gives it. */
export interface Detail {
  clip: string;
  start: number;
  end: number;
  instructions: number;
  interrupts: number;
  trace: string;
}

const at = (name: string) => `/api/watch/${encodeURIComponent(name)}`;

export function watchSessions(api: Api): Promise<{ sessions: WatchSession[] }> {
  return api.json("/api/watch");
}

export async function watchEvents(api: Api, name: string, after: number, signal?: AbortSignal): Promise<WatchEvent[]> {
  return (await api.json<{ events: WatchEvent[] }>(`${at(name)}/events?after=${after}`, signal)).events;
}

/** The session's events as server-sent events after `after`. */
export function watchStream(api: Api, name: string, after: number): EventSourceLike | undefined {
  return api.eventSource?.(api.url(`${at(name)}/stream?after=${after}`));
}

/** A recipe the session ran, as an input log from power-on. */
export function watchRecipe(api: Api, name: string, hash: string): Promise<string> {
  return api.text(`${at(name)}/recipes/${hash}`);
}

/** The session's ROM, when gp serve serves it to play; kept in memory only. */
export function watchRom(api: Api, name: string): Promise<Uint8Array> {
  return api.bytes(`${at(name)}/rom`);
}

/** L3 for frames `first` to `last` of a recipe the session ran. */
export function watchDetail(api: Api, name: string, recipe: string, first: number, last: number): Promise<Detail> {
  return api.json(`${at(name)}/detail?recipe=${recipe}&frames=${first}..=${last}`);
}

/** One line of a session's inputs.ndjson: a page's view or point, an agent's tool call. */
export interface WatchInput {
  v: number;
  actor: "person" | "agent";
  kind: "view" | "point" | "tool" | "note";
  data: Record<string, any>;
}

/** Sends an input; a point comes back with its answer in `data.answer`. */
export async function watchInput(api: Api, name: string, input: Omit<WatchInput, "v" | "actor">): Promise<WatchInput> {
  const r = await api.post<{ input: WatchInput }>(`${at(name)}/input`, { v: 1, actor: "person", ...input });
  return r.input;
}

export async function watchInputs(api: Api, name: string): Promise<WatchInput[]> {
  return (await api.json<{ inputs: WatchInput[] }>(`${at(name)}/input`)).inputs;
}

/** An input in words, for the feed. */
export function inputLine(i: WatchInput): string {
  const d = i.data ?? {};
  switch (i.kind) {
    case "tool":
      return `The agent called ${d.tool}: ${d.result}`;
    case "point": {
      const words: string[] = d.answer?.words ?? [];
      const who = i.actor === "agent" ? "The agent pointed" : "You pointed";
      return `${who} at frame ${d.frame}: ${words.length ? words.join("; ") : "no sprite there"}`;
    }
    case "view":
      return `${i.actor === "agent" ? "The agent" : "The page"} shows ${d.label ?? d.recipe}`;
    default:
      return `${i.actor}: ${d.text ?? ""}`;
  }
}
