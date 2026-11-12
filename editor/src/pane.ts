// What the shell loads for each open recording, and the windows it loads
// events in. Views get these through ViewProps (views/registry.ts).

import type { Api, DiffJson, EventsJson, LabelsJson, Manifest, RecordingShow } from "./api";
import type { FieldDef } from "./decode/trace";
import { parseInputLog, type InputFrame } from "./decode/input";
import { fromFrameJson, type FrameState } from "./state";

/** Frames per event window, the trace's default chunk size. */
export const WINDOW = 64;

/** The event window holding `frame`: [start, end), frame 0 excluded. */
export function eventWindow(frame: number, frames: number): [number, number] {
  const base = Math.floor(frame / WINDOW) * WINDOW;
  return [Math.max(1, base), Math.min(frames + 1, base + WINDOW)];
}

export interface Recording {
  id: string;
  name: string;
  show: RecordingShow;
  manifest: Manifest;
  fields: FieldDef[];
  labels: LabelsJson;
  input: InputFrame[];
}

/** A recording at the current frame. */
export interface Pane extends Recording {
  frame: number;
  state?: FrameState;
  /** The frame before, for what changed. */
  prev?: FrameState;
  /** Where the state came from: the trace, or replay. */
  from?: string;
  events?: EventsJson;
}

export async function loadRecording(api: Api, id: string): Promise<Recording> {
  const [show, manifest, fields, labels, input] = await Promise.all([
    api.show(id),
    api.manifest(id),
    api.fields(id),
    api.labels(id),
    api.input(id),
  ]);
  const name = show.names[show.names.length - 1]?.name ?? id.slice(0, 12);
  return { id: show.id, name, show, manifest, fields: fields.fields, labels, input: parseInputLog(input) };
}

/** States kept per recording; older ones are dropped, least recently used first. */
export const MAX_STATES = 512;

/**
 * Caches a recording's frames and event windows as they're fetched. A fetch
 * started for a seek is aborted when the user moves on (see App.tsx), and
 * an aborted or failed fetch isn't kept, so it's tried again next time.
 */
export class FrameCache {
  private states = new Map<number, Promise<{ state: FrameState; from: string }>>();
  private windows = new Map<number, Promise<EventsJson>>();
  constructor(
    private readonly api: Api,
    readonly recording: Recording,
  ) {}

  /** How many states are cached. */
  get size(): number {
    return this.states.size;
  }

  state(frame: number, signal?: AbortSignal): Promise<{ state: FrameState; from: string }> {
    let p = this.states.get(frame);
    if (p) {
      // Most recently used goes to the end.
      this.states.delete(frame);
    } else {
      p = this.api.frame(this.recording.id, frame, signal).then((f) => ({ state: fromFrameJson(f), from: f.from }));
      const mine = p;
      p.catch(() => {
        if (this.states.get(frame) === mine) this.states.delete(frame);
      });
    }
    this.states.set(frame, p);
    while (this.states.size > MAX_STATES) this.states.delete(this.states.keys().next().value!);
    return p;
  }

  events(frame: number, signal?: AbortSignal): Promise<EventsJson | undefined> {
    const [start, end] = eventWindow(frame, this.recording.manifest.frames);
    if (start >= end) return Promise.resolve(undefined);
    let p = this.windows.get(start);
    if (!p) {
      p = this.api.events(this.recording.id, start, end, signal);
      const mine = p;
      p.catch(() => {
        if (this.windows.get(start) === mine) this.windows.delete(start);
      });
      this.windows.set(start, p);
      while (this.windows.size > 64) this.windows.delete(this.windows.keys().next().value!);
    }
    return p;
  }

  /** The pane at `frame`: its state, the one before, and its events. */
  async pane(frame: number, signal?: AbortSignal): Promise<Pane> {
    const [cur, prev, events] = await Promise.all([
      this.state(frame, signal),
      frame > 0 ? this.state(frame - 1, signal) : Promise.resolve(undefined),
      this.events(frame, signal),
    ]);
    return { ...this.recording, frame, state: cur.state, from: cur.from, prev: prev?.state, events };
  }
}

/** The RAM addresses where two states differ, from a diff's `ram` field or by comparing. */
export function differingRam(a?: FrameState, b?: FrameState): Set<number> {
  const out = new Set<number>();
  const x = a?.get("ram");
  const y = b?.get("ram");
  if (!x || !y) return out;
  for (let i = 0; i < Math.max(x.length, y.length); i++) if (x[i] !== y[i]) out.add(i);
  return out;
}

export type { DiffJson };
