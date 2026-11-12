// The client for `gp serve` (docs/inspector.md). Shapes are the matching
// commands' `--json` output; only the fields the inspector reads are typed.

import type { FieldDef } from "./decode/trace";

export interface RecordingEntry {
  id: string;
  name: string;
  frames: number;
  source: string | null;
  created_at: string;
  retired: boolean;
}

export interface RecordingShow {
  id: string;
  names: { name: string; created_at: string; golden: boolean }[];
  stale: boolean;
  recording: {
    frames: number;
    checkpoint_interval: number;
    has_trace: boolean;
    has_events: boolean;
    source_id: string | null;
    runtime: { name: string; version: string };
  };
}

export interface Manifest {
  frames: number;
  input_log: string;
  checkpoint_interval: number;
  trace?: { schema: number; chunk_frames: number; fields: string; chunks: string[] };
  events?: { schema: number; chunk_frames: number; chunks: string[] };
}

export interface FieldsJson {
  id: string;
  state_magic: string;
  state_version: number;
  fields: FieldDef[];
}

export interface FrameJson {
  id: string;
  frame: number;
  from: "trace" | "replay";
  /** Each field's bytes as hex. */
  fields: Record<string, string>;
}

export interface EventJson {
  frame: number;
  scanline: number;
  dot: number;
  kind: string;
  addr: number;
  value: number;
}

export interface EventsJson {
  id: string;
  frames: [number, number];
  from: string;
  events: EventJson[];
}

export interface DiffJson {
  a: string;
  b: string;
  frame: number | null;
  /** Frame counts, when no frame differs. */
  frames?: [number, number];
  ram_differs?: boolean;
  fields?: { field: string; bytes: [number, number | null, number | null][] }[];
}

export interface Label {
  name: string;
  addr: number;
  size: number;
  description: string;
}

export interface LabelsJson {
  id: string;
  game: string | null;
  file: string | null;
  missing: string | null;
  labels: Label[];
}

export interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** What's under a box of the picture (`GET .../select`). */
export interface SelectJson {
  id: string;
  frame: number;
  from: string;
  game: string | null;
  /** The state mapping the entities come from, if any. */
  mapping: string | null;
  /** Why there are no entities, when there's no mapping. */
  missing: string | null;
  rect: Box;
  camera: { path: string; x: number | null; y: number | null } | null;
  world: Box | null;
  sprites: { index: number; oam: number; box: Box; tile: number; attr: number }[];
  entities: {
    id: number;
    path: string;
    kind: string;
    /** In screen pixels; y and h are null when the mapping has no y. */
    box: { x: number; y: number | null; w: number; h: number | null };
    fields: { path: string; value: string; ram: number[]; note: string | null }[];
  }[];
  ram: { addr: number; value: number; labels: string[]; paths: string[] }[];
}

/** A game's lifted tile sets and palettes (`GET /api/assets/{game}`). */
export interface AssetsJson {
  game: string;
  dir: string;
  missing: string | null;
  tiles: { name: string; tiles: number; tiles_per_row: number; width: number; height: number }[];
  palettes: { name: string; palettes: number[][]; error?: string }[];
}

export interface TileSetJson {
  game: string;
  name: string;
  tiles: number;
  tiles_per_row: number;
  width: number;
  height: number;
  /** One digit, 0 to 3, a pixel, row by row. */
  pixels: string;
}

/** One annotation (docs/formats/annotations.md). */
export interface Note {
  id: number;
  from: number;
  to: number;
  text: string;
  tags?: string[];
  created_at: string;
  updated_at?: string;
}

export interface NoteDraft {
  from: number;
  to: number;
  text: string;
  tags?: string[];
}

/** An original recording and a model run lined up (`GET /api/pair`). */
export interface PairJson {
  original: { id: string; name: string; frames: number };
  model: { id: string; name: string; frames: number };
  game: string;
  mapping: string;
  /** `polled` (the mapping's alignment) or `frame` (the whole log played as it is). */
  alignment: "polled" | "frame";
  input_matches: boolean;
  first_input_mismatch: number | null;
  pairs: number;
  /** The model frame at each original frame, the last paired one at or before it. */
  model_frame: number[];
  paths: { path: string; compared: number; agreed: number; first: { original_frame: number; model_frame: number; original: string; model: string | null } | null }[];
  /** Inclusive ranges of original frames where some mapped path differs. */
  differing: [number, number][];
}

export interface PathRow {
  path: string;
  original: string;
  model: string | null;
  differs: boolean;
  note: string | null;
  ram: number[];
  labels: string[];
}

export interface EntityBox {
  path: string;
  kind: string;
  /** On the picture; y and h are null when the mapping has no y. */
  box: { x: number; y: number | null; w: number; h: number | null };
}

/** Both sides of the pair at one original frame (`GET /api/pair/frame`). */
export interface PairFrameJson {
  original: { id: string; frame: number };
  model: { id: string; frame: number };
  asked: number;
  aligned: boolean;
  alignment: string;
  paths: PathRow[];
  differing: number;
  original_entities: EntityBox[];
  model_entities: EntityBox[];
  camera: { path: string; x: number; y: number } | null;
}

export interface CardFact {
  path: string;
  value: unknown;
  evidence: { entry: string | null; source: string; confidence: number; note: string | null; refs: { at: string; in: string | null }[] } | null;
  level: string | null;
  rule: number | null;
  tolerance: Record<string, unknown> | null;
}

export interface CardCheck {
  part: string;
  kind: string;
  quantity: string;
  case: string | null;
  against: string;
  level: string;
  status: string;
  samples: number;
  expected: string | null;
  actual: string | null;
  tolerance: string | null;
  frame: number | null;
  detail: string | null;
  subject: string | null;
  validated_at: string;
}

/** What the model says about a path (`GET .../card`). */
export interface CardJson {
  id: string;
  frame: number;
  path: string;
  part: string;
  model: { name: string; dir: string };
  level: string | null;
  facts: CardFact[];
  facts_total: number;
  values: { path: string; value: string; ram?: number[]; labels?: string[]; note?: string | null }[];
  checks: CardCheck[];
  results_for: string;
  links: { path: string; kind: "part" | "fact" }[];
}

export interface FactsJson {
  model: string;
  dir: string;
  facts: { path: string; value: unknown; level: string | null }[];
}

/** A change: model paths and their new values, as `model_edit` takes them. */
export interface EditDraft {
  path: string;
  value?: unknown;
  remove?: boolean;
}

export interface VariantDraft {
  name: string;
  edits: EditDraft[];
}

export interface VariantRun {
  name: string;
  ok: boolean;
  error?: string;
  edits?: EditDraft[];
  frames?: number;
  /** The first model frame whose state differs from the unchanged model's. */
  first_difference?: number | null;
  camera?: [number, number][];
  entities?: { path: string; kind: string; hitbox: [number, number, number, number]; x: number[]; y: number[]; live: number[] }[];
}

export interface VariantsJson {
  id: string;
  frames: number;
  variants: VariantRun[];
}

import type { PointJson } from "./point";

export class ApiError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message);
  }
}

/** One layer of a theme stack (docs/formats/theme.md). */
export interface Layer {
  theme: string;
  scope: "baked" | "personal";
  enabled?: boolean;
}

/** A palette rule: color changes for a role, a group or `*`. */
export interface PaletteRule {
  target: string;
  replace?: [string, string][];
  ramp?: string[];
  hue?: number;
  saturation?: number;
  lightness?: number;
  tint?: string;
  tint_amount?: number;
}

/** A live effect, by `kind`. */
export type Effect =
  | { kind: "gradient"; target: string; colors: string[] }
  | { kind: "glow"; target: string; color: string; radius?: number }
  | { kind: "particles"; target?: string; style: "fireflies" | "snow" | "rain" | "sparks"; color: string; count: number; seed?: number }
  | { kind: "scanlines"; strength: number }
  | { kind: "vignette"; strength: number }
  | { kind: "tint"; color: string; amount: number };

/** A theme's file as JSON. Restyles are kept as they are. */
export interface ThemeDoc {
  format: string;
  id: string;
  title: string;
  description?: string;
  author?: string;
  license: string;
  for?: string[];
  palette?: PaletteRule[];
  effect?: Effect[];
  restyle?: unknown[];
  style?: unknown;
}

/** A template's themes (`GET /api/themes/{model}`). */
export interface ThemesJson {
  model: string;
  title: string;
  themes: { id: string; scope: "baked" | "personal"; theme: ThemeDoc | null; problem: string | null }[];
  stack: Layer[];
  baselines: string[];
  entity_roles: Record<string, string>;
  roles: string[];
  groups: string[];
}

export interface ReadabilityGroup {
  group: string;
  pixels: number;
  distinct: number;
  contrast: number;
  verdict: "ok" | "warn" | "refuse";
  why: string | null;
}

export interface Readability {
  verdict: "ok" | "warn" | "refuse";
  background: string[];
  groups: ReadabilityGroup[];
  pairs: { a: string; b: string; delta_e: number; verdict: "ok" | "warn" | "refuse" }[];
  absent: string[];
}

export interface PreviewRequest {
  layers: Layer[];
  drafts?: Record<string, ThemeDoc>;
  baseline?: string;
  recording?: string;
  frames: number[];
}

/** A themed preview (`POST /api/themes/{model}/preview`). */
export interface PreviewJson {
  model: string;
  frames: { frame: number; png: string }[];
  length: number;
  digest: string;
  readability: Readability;
  style: { part: string; metric: string; value: number; bound: string; pass: boolean }[] | null;
}

/** How the inspector reaches the server; tests pass their own `fetch`. */
/** One event of a capture run (`gp capture`, docs/formats/capture-events.md, format 1). */
export interface CaptureEvent {
  v: number;
  seq: number;
  at_ms: number;
  round: number;
  actor: "pipeline" | "person" | "agent";
  kind: string;
  stage?: string;
  data?: Record<string, unknown>;
}

/** A capture run as `GET /api/capture` lists it. */
export interface CaptureRun {
  run: string;
  model: string;
  round: number;
  events: number;
  /** A watched run says what its watcher does: running, debounce, waiting, steady, stopped. */
  state: string;
  /** Whether a watcher (`gp capture watch`, a `gp host` capture job) holds the run. */
  watched?: boolean;
  pending_inputs: number;
  started_ms: number;
  preview: boolean;
  pins?: string[];
}

/** The event and input format the panel speaks (docs/formats/capture-events.md). */
export const CAPTURE_FORMAT = 1;

/** An input to a run: an answer to a question, a pin (won't model), an unpin or a note. */
export interface CaptureInput {
  v: 0 | 1;
  actor: "person" | "agent";
  kind: "answer" | "pin" | "unpin" | "note" | "accept" | "reject" | "request";
  question?: string;
  choice?: number;
  path?: string;
  text?: string;
  /** For accept and reject: the sprite edit, `<copy>:<seq>`. */
  item?: string;
  /** For a request: the frame, the region in both games and the things there (docs/point-and-tell.md). */
  data?: Record<string, unknown>;
}

/** A sprite edit an agent told a run of (`GET /api/capture/{run}/edits`, docs/sprite-editing.md). */
export interface AssetEdit {
  item: string;
  text?: string;
  data: {
    status: "proposed" | "placed" | "accepted" | "rejected" | "undone";
    decided_by?: string | null;
    entity: string;
    /** A level tile kind's look instead of an entity's sprite: its name; entity is then empty. */
    tile?: string | null;
    slot: string;
    frames?: string[];
    ops?: string[];
    prompt?: string | null;
    pixels_changed?: number;
    critic?: number | null;
    checks?: { name: string; verdict: string; message: string }[];
    previews?: { edit?: string; context?: string };
    shots?: { row: number; pose: string; baseline: string; frame: number }[];
  };
}

/** What the panel needs of the browser's EventSource; tests pass their own. */
export interface EventSourceLike {
  onmessage: ((e: { data: string; lastEventId?: string }) => void) | null;
  onerror: ((e: unknown) => void) | null;
  close(): void;
}

export type EventSourceFactory = (url: string) => EventSourceLike;

/** The browser's EventSource, where there is one. */
const browserEventSource: EventSourceFactory | undefined =
  typeof EventSource === "undefined" ? undefined : (url) => new EventSource(url) as unknown as EventSourceLike;

/** A ROM a play session can play (`GET /api/play/roms`, docs/show.md). */
export interface PlayRom {
  sha256: string;
  name: string;
  source: string | null;
  template: string | null;
}

/** A mark on a play session: frames `from` to `to`, both included. */
export interface PlayMark {
  id: number;
  from: number;
  to: number;
  note: string;
  by: "key" | "timeline";
  annotation?: number;
}

/** A play session (`play.json`). */
export interface PlaySession {
  run: string;
  rom: string;
  source: string | null;
  template: string | null;
  notes: string;
  real_store: boolean;
  store: string;
  preview: boolean;
  started_ms: number;
  state: "playing" | "recording" | "explaining" | "done" | "failed";
  frames: number;
  digest: string | null;
  recording: string | null;
  digest_matches: boolean | null;
  marks: PlayMark[];
  explanations: Record<string, unknown>;
  error?: string | null;
}

/** A mechanic as the game view phrases it (`GET /api/capture/{run}/game`, from `gp model mechanics`). */
export interface MechanicWords {
  name: string;
  about: string;
  class: string;
  params: { name: string; type?: string; unit?: string; optional?: boolean }[];
  emits?: string[];
  used_by?: string[];
  like?: string | null;
}

/** The original's state at a frame, translated into the model's (`GET /api/capture/{run}/start-state`). */
export interface StartState {
  /** The frame asked for. */
  asked: number;
  /** The first frame the model stands for at or after it: the start. */
  original_frame: number;
  /** The model's frame there. */
  frame: number;
  /** The canonical model state, in base64, for the preview's load-state. */
  state: string;
  notes: import("./lockfold").LockNote[];
}

/** A run's game in its own words (`GET /api/capture/{run}/game`, docs/inspector.md, "The game view"). */
export interface GameWords {
  run: string;
  kind: "capture" | "play";
  template?: string | null;
  vocab: {
    name: string | null;
    title: string | null;
    view: string | null;
    entities: { name: string; role: string | null; description: string | null }[];
    levels: { name: string; description: string | null; size: [number, number] | null; tile_size: number | null }[];
    counters: string[];
    rules: Record<string, string[]>;
    actions: Record<string, string> | null;
  };
  mechanics: { mechanics: MechanicWords[]; paths: { path: string; class: string; mechanic?: string; note?: string }[] };
  /** By baseline name, or `session`; `input` names the input log the run has for it. */
  recordings: { id: string; name: string; frames: number; input: string | null }[];
  rom: boolean;
  preview: boolean;
}

export class Api {
  constructor(
    private readonly base = "",
    private readonly fetchFn: typeof fetch = (...a) => fetch(...a),
    /** Server-sent events; undefined where the platform has none, and the panel polls. */
    readonly eventSource: EventSourceFactory | undefined = browserEventSource,
  ) {}

  private async get(path: string, signal?: AbortSignal): Promise<Response> {
    return this.request(path, { signal });
  }

  private async request(path: string, init: RequestInit): Promise<Response> {
    const res = await this.fetchFn(this.base + path, init);
    if (!res.ok) {
      let message = `${res.status} ${res.statusText}`;
      try {
        message = ((await res.json()) as { error?: string }).error ?? message;
      } catch {
        // Not JSON; keep the status line.
      }
      throw new ApiError(res.status, message);
    }
    return res;
  }

  async json<T>(path: string, signal?: AbortSignal): Promise<T> {
    return (await (await this.get(path, signal)).json()) as T;
  }

  async bytes(path: string): Promise<Uint8Array> {
    return new Uint8Array(await (await this.get(path)).arrayBuffer());
  }

  async text(path: string): Promise<string> {
    return (await this.get(path)).text();
  }

  recordings(): Promise<{ recordings: RecordingEntry[] }> {
    return this.json("/api/recordings");
  }
  show(id: string): Promise<RecordingShow> {
    return this.json(`/api/recordings/${id}`);
  }
  manifest(id: string): Promise<Manifest> {
    return this.json(`/api/recordings/${id}/manifest`);
  }
  input(id: string): Promise<string> {
    return this.text(`/api/recordings/${id}/input`);
  }
  fields(id: string): Promise<FieldsJson> {
    return this.json(`/api/recordings/${id}/fields`);
  }
  frame(id: string, frame: number, signal?: AbortSignal): Promise<FrameJson> {
    return this.json(`/api/recordings/${id}/frames/${frame}`, signal);
  }
  /** Events of frames `from` up to but not including `to`. */
  events(id: string, from: number, to: number, signal?: AbortSignal): Promise<EventsJson> {
    return this.json(`/api/recordings/${id}/events?frames=${from}..${to}`, signal);
  }
  chunk(id: string, i: number): Promise<Uint8Array> {
    return this.bytes(`/api/recordings/${id}/trace/${i}`);
  }
  labels(id: string): Promise<LabelsJson> {
    return this.json(`/api/recordings/${id}/labels`);
  }
  diff(a: string, b: string): Promise<DiffJson> {
    return this.json(`/api/diff?a=${a}&b=${b}`);
  }
  select(id: string, frame: number, box: Box, signal?: AbortSignal): Promise<SelectJson> {
    return this.json(`/api/recordings/${id}/select?frame=${frame}&rect=${box.x},${box.y},${box.w},${box.h}`, signal);
  }
  pair(original: string, model: string, signal?: AbortSignal): Promise<PairJson> {
    return this.json(`/api/pair?original=${original}&model=${model}`, signal);
  }
  pairFrame(original: string, model: string, frame: number, signal?: AbortSignal): Promise<PairFrameJson> {
    return this.json(`/api/pair/frame?original=${original}&model=${model}&frame=${frame}`, signal);
  }
  card(id: string, path: string, frame: number, results?: string, signal?: AbortSignal): Promise<CardJson> {
    const r = results ? `&results=${results}` : "";
    return this.json(`/api/recordings/${id}/card?path=${encodeURIComponent(path)}&frame=${frame}${r}`, signal);
  }
  facts(id: string): Promise<FactsJson> {
    return this.json(`/api/recordings/${id}/facts`);
  }
  variants(id: string, variants: VariantDraft[], signal?: AbortSignal): Promise<VariantsJson> {
    return this.json(`/api/recordings/${id}/variants?variants=${encodeURIComponent(JSON.stringify(variants))}`, signal);
  }
  colors(): Promise<{ colors: string[] }> {
    return this.json("/api/nes/colors");
  }
  assets(game: string): Promise<AssetsJson> {
    return this.json(`/api/assets/${encodeURIComponent(game)}`);
  }
  tileSet(game: string, name: string): Promise<TileSetJson> {
    return this.json(`/api/assets/${encodeURIComponent(game)}/tiles/${encodeURIComponent(name)}`);
  }
  annotations(id: string): Promise<{ id: string; file: string; annotations: Note[] }> {
    return this.json(`/api/recordings/${id}/annotations`);
  }
  private async write<T>(method: string, path: string, body?: unknown): Promise<T> {
    const init: RequestInit = { method };
    if (body !== undefined) {
      init.headers = { "Content-Type": "application/json" };
      init.body = JSON.stringify(body);
    }
    return (await (await this.request(path, init)).json()) as T;
  }
  addNote(id: string, note: NoteDraft): Promise<{ annotation: Note }> {
    return this.write("POST", `/api/recordings/${id}/annotations`, note);
  }
  updateNote(id: string, n: number, note: NoteDraft): Promise<{ annotation: Note }> {
    return this.write("PUT", `/api/recordings/${id}/annotations/${n}`, note);
  }
  deleteNote(id: string, n: number): Promise<{ deleted: number }> {
    return this.write("DELETE", `/api/recordings/${id}/annotations/${n}`);
  }
  themes(model: string): Promise<ThemesJson> {
    return this.json(`/api/themes/${encodeURIComponent(model)}`);
  }
  themePreview(model: string, req: PreviewRequest): Promise<PreviewJson> {
    return this.write("POST", `/api/themes/${encodeURIComponent(model)}/preview`, req);
  }
  captureRuns(): Promise<{ runs: CaptureRun[] }> {
    return this.json("/api/capture");
  }
  captureEvents(run: string, after: number, signal?: AbortSignal): Promise<{ run: string; events: CaptureEvent[] }> {
    return this.json(`/api/capture/${encodeURIComponent(run)}/events?after=${after}`, signal);
  }
  captureInput(run: string, input: CaptureInput): Promise<{ inputs: number; input?: CaptureInput; decision?: Record<string, unknown>; decision_error?: string }> {
    return this.write("POST", `/api/capture/${encodeURIComponent(run)}/input`, input);
  }
  /** The run's events as server-sent events after `after` (`id` is each event's seq). */
  captureStream(run: string, after: number): EventSourceLike | undefined {
    return this.eventSource?.(`${this.base}/api/capture/${encodeURIComponent(run)}/stream?after=${after}`);
  }
  /** The URL of a file of the interpreter preview a run built. */
  /** What's under a box of the screen at a frame, in both games (docs/point-and-tell.md). */
  capturePoint(run: string, recording: string, frame: number, box: Box, signal?: AbortSignal): Promise<PointJson> {
    const r = `${box.x},${box.y},${box.w},${box.h}`;
    return this.json(`/api/capture/${encodeURIComponent(run)}/point?recording=${encodeURIComponent(recording)}&frame=${frame}&rect=${r}`, signal);
  }
  /** Starts `gp request explain` on request `n`; its reply arrives in the run's events. */
  requestExplain(run: string, n: number): Promise<{ started: boolean }> {
    return this.write("POST", `/api/capture/${encodeURIComponent(run)}/requests/${n}/explain`, {});
  }
  captureRequests(run: string): Promise<{ run: string; explaining: number[] }> {
    return this.json(`/api/capture/${encodeURIComponent(run)}/requests`);
  }
  captureEdits(run: string): Promise<{ run: string; edits: AssetEdit[] }> {
    return this.json(`/api/capture/${encodeURIComponent(run)}/edits`);
  }
  /** The URL of an edit's preview: `edit` (before, after, changed) or `context` (the recorded frames). */
  captureEditPreview(run: string, copy: string, seq: string, which: "edit" | "context"): string {
    return `${this.base}/api/capture/${encodeURIComponent(run)}/edits/${encodeURIComponent(copy)}/${encodeURIComponent(seq)}/${which}`;
  }
  capturePreview(run: string, file: string): string {
    // The preview reads its build from beside the page (`files=here`), as
    // the mechanic page plays it, not from a game installed in the browser.
    const here = file.startsWith("index.html") ? `${file.includes("?") ? "&" : "?"}files=here` : "";
    return `${this.base}/api/capture/${encodeURIComponent(run)}/preview/${file}${here}`;
  }
  /** The original and the model in lockstep on one of the run's recordings (docs/inspector.md, "The game view"). */
  captureLockstep(run: string, recording: string, mode: "resync" | "free", from?: number): Promise<import("./lockfold").Lockstep> {
    const f = from === undefined ? "" : `&from=${from}`;
    return this.json(`/api/capture/${encodeURIComponent(run)}/lockstep?recording=${encodeURIComponent(recording)}&mode=${mode}${f}`);
  }
  /** The original's state at its frame `frame` as a model state, for the rebuilt game to start from (docs/inspector.md, "The same commands to both"). */
  captureStartState(run: string, recording: string, frame: number): Promise<StartState> {
    return this.json(`/api/capture/${encodeURIComponent(run)}/start-state?recording=${encodeURIComponent(recording)}&frame=${frame}`);
  }
  /** The game's own moments in a recording and what the model does at each (docs/interpreter.md, "Moments"). */
  captureMoments(run: string, recording: string): Promise<import("./momentfold").Moments> {
    return this.json(`/api/capture/${encodeURIComponent(run)}/moments?recording=${encodeURIComponent(recording)}`);
  }
  /** The refinement loop on a run's working copy: its state and rounds so far (docs/interpreter.md, "Refining from the lockstep"). */
  captureRefine(run: string, recording: string): Promise<import("./refinefold").RefineState> {
    return this.json(`/api/capture/${encodeURIComponent(run)}/refine?recording=${encodeURIComponent(recording)}`);
  }
  /** Starts the loop, its first window at the first split at or after the original's frame `from`. */
  captureRefineStart(run: string, recording: string, from?: number, scope?: { moment?: { kind: string; n: number }; mechanic?: string; window?: number; ask?: boolean }): Promise<{ started: boolean; why?: string }> {
    return this.write("POST", `/api/capture/${encodeURIComponent(run)}/refine`, { recording, from, ...scope });
  }
  captureRefineStop(run: string): Promise<{ stopping: boolean }> {
    return this.write("POST", `/api/capture/${encodeURIComponent(run)}/refine/stop`, {});
  }
  /** A run's game in its own words, for the game view. */
  captureGame(run: string): Promise<GameWords> {
    return this.json(`/api/capture/${encodeURIComponent(run)}/game`);
  }
  /** The ROM a run was started with or a session played; kept in memory only. */
  captureRom(run: string): Promise<Uint8Array> {
    return this.bytes(`/api/capture/${encodeURIComponent(run)}/rom`);
  }
  /** A baseline's input log as the run generated it, or `session`. */
  captureInputLog(run: string, name: string): Promise<string> {
    return this.text(`/api/capture/${encodeURIComponent(run)}/inputs/${encodeURIComponent(name)}`);
  }
  playRoms(): Promise<{ roms: PlayRom[]; wasm: boolean; scratch_store: string; real_store: string }> {
    return this.json("/api/play/roms");
  }
  /** A ROM's bytes, for the player; kept in memory only. */
  playRom(sha256: string): Promise<Uint8Array> {
    return this.bytes(`/api/play/roms/${sha256}`);
  }
  /** The live player module's URL. */
  playWasm(): string {
    return `${this.base}/api/play/gp_play.wasm`;
  }
  playStart(body: { rom: string; template?: string; notes: string; real_store: boolean; preview?: boolean }): Promise<{ session: PlaySession }> {
    return this.write("POST", "/api/play/sessions", body);
  }
  playSession(run: string): Promise<{ session: PlaySession }> {
    return this.json(`/api/play/sessions/${encodeURIComponent(run)}`);
  }
  playLog(run: string, offset: number, lines: string): Promise<{ frames: number }> {
    return this.write("POST", `/api/play/sessions/${encodeURIComponent(run)}/log`, { offset, lines });
  }
  playMark(run: string, mark: { from: number; to?: number; note: string; by: "key" | "timeline" }): Promise<{ mark: PlayMark }> {
    return this.write("POST", `/api/play/sessions/${encodeURIComponent(run)}/marks`, mark);
  }
  playStop(run: string, frames: number, digest: string): Promise<{ session: PlaySession }> {
    return this.write("POST", `/api/play/sessions/${encodeURIComponent(run)}/stop`, { frames, digest });
  }
  playInputLog(run: string): Promise<string> {
    return this.text(`/api/play/sessions/${encodeURIComponent(run)}/input.log`);
  }
  saveTheme(model: string, body: { theme?: ThemeDoc; layers?: Layer[] }): Promise<{ wrote: string[] }> {
    return this.write("POST", `/api/themes/${encodeURIComponent(model)}/save`, body);
  }  /** A POST to gp serve, for the clients in their own files (freefold.ts). */
  post<T>(path: string, body: unknown): Promise<T> {
    return this.write("POST", path, body);
  }
  /** A URL of gp serve's, for an image or a module. */
  url(path: string): string {
    return this.base + path;
  }
}
