// A watch session's events (docs/watch.md, gp-events' watch format)
// folded into what the watch view draws: the PRG map lighting up as code
// first runs, bank by bank; the places the static reader stopped and which
// a run went past; edges and pointer values; the search's ticker and the
// clips it kept; what stage 0 read after the runs; the behaviour gp
// behave's detectors found; and a feed in words.
// Pure, so the tests fold a session file.

/** One line of a session's events.ndjson. */
export interface WatchEvent {
  v: number;
  seq: number;
  /** The recipe a run's event came from, by hash. */
  run?: string;
  frame?: number;
  level?: number;
  kind: string;
  data: Record<string, unknown>;
}

/** Where something was first seen: the event, the recipe and its frame. */
export interface Seen {
  seq: number;
  run?: string;
  frame?: number;
}

export interface StopView {
  id: string;
  pc: string;
  target?: string;
  behind: number;
  /** Set once a run took the stop's transfer. */
  reached?: Seen & { routine: string };
}

export interface KeptClip extends Seen {
  id: string;
  start: string;
  presses: string[];
  frames: number;
  recipe: string;
  why: string;
  endHash: string;
}

export interface WatchState {
  seq: number;
  rom?: { sha256: string; mapper: number; prgBytes: number };
  starts: { id: string; recipe: string; frames: number }[];
  explore?: { aim: string; targets: string[]; budget: number; seed: number };
  /** PRG ROM 1 KB pages with code run in them, each with where it first ran. */
  pages: Record<number, Seen>;
  /** PRG bytes run so far. */
  codeBytes: number;
  /** Switched banks in the order their first code ran. */
  banks: (Seen & { bank: number; at: string })[];
  edges: number;
  edgeKinds: Record<string, number>;
  pointers: (Seen & { ptr: number; value: number; to: string })[];
  stops: StopView[];
  read?: { read: number; routines: number };
  /** Routines read before the runs, from the first snapshot. */
  readAlone?: number;
  facts: (Seen & { what: string; routine: string })[];
  /** What gp behave's detectors found in the runs, in words. */
  behaviour: { seq: number; what: string; routine: string; why: string }[];
  progress?: { ran: number; budget: number; kept: number; best: [number, number] | null };
  kept: KeptClip[];
  ended: boolean;
  /** The newest last, at most LOG_SIZE. */
  feed: (Seen & { text: string; kind: string })[];
}

/** The most feed lines kept. */
export const LOG_SIZE = 400;

export function emptyWatch(): WatchState {
  return { seq: 0, starts: [], pages: {}, codeBytes: 0, banks: [], edges: 0, edgeKinds: {}, pointers: [], stops: [], facts: [], behaviour: [], kept: [], ended: false, feed: [] };
}

const hex4 = (n: number) => n.toString(16).toUpperCase().padStart(4, "0");

/** An event in words, for the feed; undefined for one too small to list (each edge). */
export function phrase(e: WatchEvent): string | undefined {
  const d = e.data as Record<string, any>;
  const at = e.frame !== undefined ? ` (frame ${e.frame})` : "";
  switch (e.kind) {
    case "watch.start":
      return `Watching a ROM of ${Math.round(d.prg_bytes / 1024)} KB PRG on mapper ${d.mapper}, from ${d.starts.length} start clips`;
    case "snapshot":
      return `Stage 0 reads ${d.read} of ${d.routines} routines; ${d.stops.length} places where it stopped`;
    case "new-code": {
      const bytes = (d.ranges as [number, number][]).reduce((n, [a, b]) => n + b - a, 0);
      return `${bytes} bytes of code run for the first time, from ${(d.at as string[])[0]}${at}`;
    }
    case "bank":
      return `Bank ${d.bank} runs for the first time, at ${d.at}${at}`;
    case "pointer":
      return `A jump through $${hex4(d.ptr)} goes to ${d.to} ($${hex4(d.value)})${at}`;
    case "stop.reached":
      return `A run goes past where the reader stopped (${d.stop}) into ${d.routine}${at}`;
    case "fact":
      return `With the runs, stage 0 reads ${d.routine}`;
    case "explore.progress":
      return undefined;
    case "clip.kept":
      return `The search keeps a clip from ${d.start}: ${d.why}`;
    case "behaviour":
      return `Behaviour: ${d.what}${d.routine ? ` (${d.routine})` : ""}`;
    case "watch.end":
      return `Done: ${d.read} of ${d.routines} routines read; ${d.kept} clips kept${(d.reached as string[]).length ? `, reaching ${(d.reached as string[]).join(", ")}` : ""}`;
    default:
      return undefined;
  }
}

/** Folds `events` (in seq order; those at or before `s.seq` are skipped) into a new state. */
export function fold(s: WatchState, events: WatchEvent[]): WatchState {
  const n: WatchState = {
    ...s,
    pages: { ...s.pages },
    banks: [...s.banks],
    edgeKinds: { ...s.edgeKinds },
    pointers: [...s.pointers],
    stops: s.stops.map((x) => ({ ...x })),
    facts: [...s.facts],
    behaviour: [...s.behaviour],
    kept: [...s.kept],
    feed: [...s.feed],
  };
  for (const e of events) {
    if (e.seq <= n.seq) continue;
    n.seq = e.seq;
    const d = e.data as Record<string, any>;
    const seen: Seen = { seq: e.seq, run: e.run, frame: e.frame };
    switch (e.kind) {
      case "watch.start":
        n.rom = { sha256: d.rom_sha256, mapper: d.mapper, prgBytes: d.prg_bytes };
        n.starts = d.starts;
        n.explore = d.explore;
        break;
      case "snapshot": {
        if (n.readAlone === undefined) n.readAlone = d.read;
        n.read = { read: d.read, routines: d.routines };
        // A stop already passed stays passed; new ones join.
        const passed = new Map(n.stops.filter((x) => x.reached).map((x) => [x.id, x.reached]));
        const known = new Set(n.stops.map((x) => x.id));
        for (const st of d.stops as StopView[]) {
          if (!known.has(st.id)) n.stops.push({ id: st.id, pc: st.pc, target: st.target ?? undefined, behind: st.behind, reached: passed.get(st.id) });
        }
        break;
      }
      case "new-code":
        for (const [a, b] of d.ranges as [number, number][]) {
          n.codeBytes += b - a;
          for (let p = Math.floor(a / 1024); p <= Math.floor((b - 1) / 1024); p++) n.pages[p] ??= seen;
        }
        break;
      case "bank":
        n.banks.push({ ...seen, bank: d.bank, at: d.at });
        break;
      case "edge":
        n.edges += 1;
        n.edgeKinds[d.edge] = (n.edgeKinds[d.edge] ?? 0) + 1;
        break;
      case "pointer":
        n.pointers.push({ ...seen, ptr: d.ptr, value: d.value, to: d.to });
        break;
      case "stop.reached": {
        const st = n.stops.find((x) => x.id === d.stop);
        if (st && !st.reached) st.reached = { ...seen, routine: d.routine };
        break;
      }
      case "fact":
        n.facts.push({ ...seen, what: d.what, routine: d.routine });
        break;
      case "behaviour":
        n.behaviour.push({ seq: e.seq, what: d.what, routine: d.routine, why: d.why });
        break;
      case "explore.progress":
        n.progress = { ran: d.ran, budget: d.budget, kept: d.kept, best: d.best ?? null };
        break;
      case "clip.kept":
        n.kept.push({ ...seen, id: d.id, start: d.start, presses: d.presses, frames: d.frames, recipe: d.recipe, why: d.why, endHash: d.end_hash });
        break;
      case "watch.end":
        n.ended = true;
        n.read = { read: d.read, routines: d.routines };
        break;
    }
    const text = phrase(e);
    if (text) n.feed.push({ ...seen, text, kind: e.kind });
  }
  if (n.feed.length > LOG_SIZE) n.feed = n.feed.slice(n.feed.length - LOG_SIZE);
  return n;
}

/** Parses an events.ndjson file's text: each whole line that is an event. */
export function parseLines(text: string): WatchEvent[] {
  const out: WatchEvent[] = [];
  for (const line of text.split("\n")) {
    if (!line.trim()) continue;
    try {
      out.push(JSON.parse(line) as WatchEvent);
    } catch {
      // A torn or foreign line; skip it.
    }
  }
  return out;
}

/** The PRG map's rows: one a bank of `bankBytes` (16 KB when the board's banks aren't known), each with its 1 KB pages. */
export function bankRows(s: WatchState, bankBytes = 16 * 1024): { bank: number; pages: { page: number; seen?: Seen }[] }[] {
  const total = s.rom?.prgBytes ?? 0;
  const per = Math.max(1, Math.floor(bankBytes / 1024));
  const rows = [];
  for (let b = 0; b * bankBytes < total; b++) {
    const pages = [];
    for (let i = 0; i < per; i++) {
      const page = b * per + i;
      pages.push({ page, seen: s.pages[page] });
    }
    rows.push({ bank: b, pages });
  }
  return rows;
}
