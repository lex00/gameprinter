// The live panel: a capture run as it happens, beside the recording views
// in the same workspace (docs/capture-live.md). It takes the run's events
// as server-sent events (polling when the stream fails), folds them
// (livefold.ts) and draws the stages, the scorecard numbers with each
// round's deltas, the questions to answer, the gaps, what's pinned as won't
// model, the loops' tables, the log of what the pipeline, the agent and the
// person did, and the interpreter preview the run built. Answers, pins and
// notes go to the run as inputs; its watcher runs the next round.

import { useEffect, useState } from "react";
import { CAPTURE_FORMAT, type Api, type CaptureEvent, type CaptureInput, type CaptureRun, type EventSourceLike } from "./api";
import { AssetEdits } from "./AssetEdits";
import { HEADLINE, delta, emptyLive, fmt, fold, logLine, openGaps, openQuestions, wontModel, type LiveState } from "./livefold";
import { MEANINGS } from "./phrase";
import { emptyPlay, foldPlay, type PlayState } from "./playfold";

/** How often the panel asks for new events when it polls. */
export const POLL_MS = 500;
/** The most events `GET /api/capture/{run}/events` returns at once. */
const PAGE = 2000;

/** How the panel gets events: pushed (server-sent events) or by asking. */
export type Transport = "stream" | "poll";

/** What a subscription tells its owner. */
export interface Sink<E> {
  events: (events: E[]) => void;
  transport: (t: Transport) => void;
  error: (message: string | undefined) => void;
}
export type LiveSink = Sink<CaptureEvent>;

/** Where a subscription gets events: a page after a seq, and a stream when there is one. */
export interface Feed<E> {
  page(after: number, signal: AbortSignal): Promise<E[]>;
  stream(after: number): EventSourceLike | undefined;
}

/**
 * Follows a feed of events numbered by `seq`: the stream first (the
 * browser resumes it with Last-Event-ID), and polling from the last event it
 * gave when the stream fails or the platform has none. Each event is passed
 * on once, in order. Returns the function that stops it. Capture runs and
 * watch sessions (Watch.tsx) both follow theirs with it.
 */
export function follow<E extends { seq: number }>(feed: Feed<E>, sink: Sink<E>, pollMs = POLL_MS): () => void {
  let live = true;
  let seq = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const abort = new AbortController();
  const take = (events: E[]) => {
    const fresh = events.filter((e) => e.seq > seq);
    if (!fresh.length) return;
    seq = fresh[fresh.length - 1].seq;
    sink.events(fresh);
  };
  const poll = async () => {
    let full = false;
    try {
      const events = await feed.page(seq, abort.signal);
      if (!live) return;
      full = events.length >= PAGE;
      take(events);
      sink.error(undefined);
    } catch (e) {
      if (live) sink.error((e as Error).message);
    }
    // A full page means more are waiting: ask again at once.
    if (live) timer = setTimeout(poll, full ? 0 : pollMs);
  };
  const source = feed.stream(0);
  if (source) {
    sink.transport("stream");
    source.onmessage = (m) => {
      if (!live) return;
      try {
        take([JSON.parse(m.data) as E]);
        sink.error(undefined);
      } catch {
        // Not an event; skip it.
      }
    };
    source.onerror = () => {
      source.close();
      if (!live) return;
      sink.transport("poll");
      void poll();
    };
  } else {
    sink.transport("poll");
    void poll();
  }
  return () => {
    live = false;
    abort.abort();
    source?.close();
    if (timer) clearTimeout(timer);
  };
}

/** Follows a capture run's events ([`follow`] on its routes). */
export function subscribe(api: Api, run: string, sink: LiveSink, pollMs = POLL_MS): () => void {
  return follow<CaptureEvent>(
    {
      page: async (after, signal) => (await api.captureEvents(run, after, signal)).events,
      stream: (after) => api.captureStream(run, after),
    },
    sink,
    pollMs,
  );
}

/** A run's events folded twice: as a capture run (livefold) and as a play session (playfold). */
export interface RunEvents {
  state: LiveState;
  play: PlayState;
  error?: string;
  transport?: Transport;
}

export function useLive(api: Api, run: string | undefined, pollMs = POLL_MS): RunEvents {
  const [state, setState] = useState<{ live: LiveState; play: PlayState }>(() => ({ live: emptyLive(), play: emptyPlay() }));
  const [error, setError] = useState<string>();
  const [transport, setTransport] = useState<Transport>();
  useEffect(() => {
    setState({ live: emptyLive(), play: emptyPlay() });
    setError(undefined);
    setTransport(undefined);
    if (!run) return;
    return subscribe(
      api,
      run,
      {
        events: (evs) => setState((s) => ({ live: fold(s.live, evs), play: foldPlay(s.play, evs) })),
        transport: setTransport,
        error: setError,
      },
      pollMs,
    );
  }, [api, run, pollMs]);
  return { state: state.live, play: state.play, error, transport };
}

/** An input from the person in the panel, in the current format. */
export function personInput(i: Omit<CaptureInput, "v" | "actor">): CaptureInput {
  return { v: CAPTURE_FORMAT, actor: "person", ...i };
}

export function LiveBody({ state, run, api, send, open }: { state: LiveState; run: string; api: Api; send: (i: CaptureInput) => void; open?: (recording: string) => void }) {
  const [note, setNote] = useState("");
  const keys = [...HEADLINE.filter((k) => k in state.metrics), ...Object.keys(state.metrics).filter((k) => !HEADLINE.includes(k)).sort()];
  const [all, setAll] = useState(false);
  const shown = all ? keys : keys.filter((k) => HEADLINE.includes(k));
  const questions = openQuestions(state);
  const pinned = wontModel(state);
  const gaps = openGaps(state);
  const input = (i: Omit<CaptureInput, "v" | "actor">) => send(personInput(i));
  return (
    <div className="live" data-testid="live">
      <p className="status" data-testid="live-status">
        {run}: round {state.round}, {state.running ? "running" : state.watch ? state.watch.state : "idle"}, {state.seq} events
        {state.watch?.reason ? ` (${state.watch.reason})` : ""}
      </p>

      <ol className="stages" data-testid="live-stages">
        {state.order.map((name) => {
          const st = state.stages[name];
          return (
            <li key={name} className={`stage ${st.status}${st.result && st.result !== "ok" ? ` was-${st.result}` : ""}`} title={st.fromRun ? `${st.summary} (from run ${st.fromRun})` : st.summary}>
              <b>{name}</b> {st.status}
              {st.result && st.result !== "ok" ? ` (${st.result})` : ""}
              {st.ms ? ` ${st.ms} ms` : ""}
            </li>
          );
        })}
      </ol>

      <table className="metrics" data-testid="live-metrics">
        <tbody>
          {shown.map((k) => {
            const m = state.metrics[k];
            const d = delta(m);
            return (
              <tr key={k} className={[m.round === state.round && d ? "changed" : "", m.budget !== undefined && m.value > m.budget ? "over" : ""].join(" ").trim() || undefined}>
                <td title={MEANINGS[k]?.means}>{k}</td>
                <td>{fmt(m.value)}</td>
                <td>{m.budget !== undefined ? `budget ${fmt(m.budget)}` : d}</td>
                <td className="hint">{MEANINGS[k]?.means ?? ""}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
      {keys.length > shown.length || all ? (
        <button onClick={() => setAll(!all)}>{all ? "headline numbers" : `all ${keys.length} numbers`}</button>
      ) : null}

      {state.rounds.length > 0 && (
        <table className="rounds" data-testid="live-rounds">
          <thead>
            <tr>
              <th>round</th>
              <th>ms</th>
              <th>failed</th>
              <th>open</th>
              <th>changed</th>
            </tr>
          </thead>
          <tbody>
            {state.rounds.map((r) => (
              <tr key={r.round}>
                <td>{r.round}</td>
                <td>{r.ms}</td>
                <td>{r.failed}</td>
                <td>{r.open}</td>
                <td>
                  {Object.entries(r.deltas)
                    .filter(([k]) => r.round === 1 ? HEADLINE.includes(k) : true)
                    .map(([k, v]) => `${k} ${fmt(v.prev)} -> ${fmt(v.value)}`)
                    .join("; ")}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      <section className="questions" data-testid="live-questions">
        <h3>Questions ({questions.length})</h3>
        {questions.map((q) => (
          <div key={q.id} className="question">
            <p>{q.text}</p>
            {q.candidates.map((c, i) => (
              <button key={i} onClick={() => input({ kind: "answer", question: q.id, choice: i + 1 })}>
                {c.value} ({c.from})
              </button>
            ))}
            {q.path && (
              <button onClick={() => input({ kind: "pin", path: q.path })} title="won't model: stop asking and leave it out of the counts">
                pin {q.path}
              </button>
            )}
          </div>
        ))}
      </section>

      {gaps.length > 0 && (
        <section className="gaps" data-testid="live-gaps">
          <h3>Gaps ({gaps.length})</h3>
          <ul>
            {gaps.map((g) => (
              <li key={g.id}>
                {g.id} <span className="hint">{g.text}</span>{" "}
                <button onClick={() => input({ kind: "pin", path: g.id })} title="won't model: leave it out of the counts">
                  pin
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}

      {pinned.length > 0 && (
        <section className="pinned" data-testid="live-pinned">
          <h3>Won't model ({pinned.length})</h3>
          <ul>
            {pinned.map((p) => (
              <li key={p.pin}>
                {p.id} <span className="hint">{p.text}</span> <span className="wont">won't model</span>{" "}
                <button onClick={() => input({ kind: "unpin", path: p.pin })}>unpin</button>
              </li>
            ))}
          </ul>
        </section>
      )}

      {Object.entries(state.tables).map(([table, rows]) => {
        const cols = [...new Set(rows.flatMap((r) => Object.keys(r.cells).filter((k) => typeof r.cells[k] !== "object")))];
        return (
          <section key={table} className="table" data-testid={`live-table-${table}`}>
            <h3>{table} rounds</h3>
            <table>
              <thead>
                <tr>
                  <th>#</th>
                  <th>round</th>
                  {cols.map((c) => (
                    <th key={c}>{c}</th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {rows.map((r) => (
                  <tr key={r.index}>
                    <td>{r.index}</td>
                    <td>{r.round}</td>
                    {cols.map((c) => (
                      <td key={c}>{typeof r.cells[c] === "number" ? fmt(r.cells[c] as number) : String(r.cells[c] ?? "")}</td>
                    ))}
                  </tr>
                ))}
              </tbody>
            </table>
          </section>
        );
      })}

      <AssetEdits api={api} run={run} />

      {state.previewRound !== undefined && (
        <section className="preview">
          <h3>The model, regenerated (round {state.previewRound})</h3>
          {/* The preview page draws at 768x720; shown at half size. */}
          <div className="preview-frame">
            <iframe key={state.previewRound} title="interpreter preview" src={api.capturePreview(run, "index.html")} width={768} height={720} />
          </div>
        </section>
      )}

      {state.recordings.length > 0 && open && (
        <section className="recordings" data-testid="live-recordings">
          <h3>The original</h3>
          {state.recordings.map((r) => (
            <button key={r.id} onClick={() => open(r.id)} title="open it in the inspector">
              {r.id.slice(0, 8)} ({r.text})
            </button>
          ))}
        </section>
      )}

      <section className="log" data-testid="live-log">
        <h3>Log</h3>
        <form
          onSubmit={(e) => {
            e.preventDefault();
            if (note.trim()) input({ kind: "note", text: note.trim() });
            setNote("");
          }}
        >
          <input value={note} onChange={(e) => setNote(e.target.value)} placeholder="a note to the agent and the log" />
        </form>
        <ul>
          {[...state.log].reverse().map((e) => (
            <li key={e.seq} className={`actor-${e.actor}`}>
              <span className="when">r{e.round} {e.at_ms} ms</span> <span className="actor">{e.actor}</span> {logLine(e)}
            </li>
          ))}
        </ul>
      </section>
    </div>
  );
}

/** The run's raw panel: a run picker and the run, in the details. `run` comes from the URL (`?run=NAME`); the caller follows its events. */
export function LivePanel({ api, run, setRun, open, events }: { api: Api; run?: string; setRun: (r: string | undefined) => void; open?: (recording: string) => void; events: RunEvents }) {
  const [runs, setRuns] = useState<CaptureRun[]>([]);
  const [sent, setSent] = useState<string>();
  const { state, error, transport } = events;
  useEffect(() => {
    api
      .captureRuns()
      .then((r) => setRuns(r.runs))
      .catch(() => setRuns([]));
  }, [api, run]);
  const send = sendInput(api, run, runs, state, setSent);
  return (
    <section className="live-panel">
      <h2>Capture run</h2>
      <RunPicker runs={runs} run={run} setRun={setRun} />
      {error && <p className="error">{error}</p>}
      {run && transport && <p className="hint" data-testid="live-transport">{transport === "stream" ? "live (server-sent events)" : "polling"}</p>}
      {sent && <p className="hint">{sent}</p>}
      {run && <LiveBody state={state} run={run} api={api} send={send} open={open} />}
    </section>
  );
}

/** The runs to choose from, newest first. */
export function RunPicker({ runs, run, setRun }: { runs: CaptureRun[]; run?: string; setRun: (r: string | undefined) => void }) {
  return (
    <select data-testid="pick-run" value={run ?? ""} onChange={(e) => setRun(e.target.value || undefined)}>
      <option value="">no run</option>
      {runs.map((r) => (
        <option key={r.run} value={r.run}>
          {r.run} (round {r.round}, {r.state}
          {r.pending_inputs ? `, ${r.pending_inputs} inputs waiting` : ""})
        </option>
      ))}
    </select>
  );
}

/** Sends a person's input to a run and says what happens next. */
export function sendInput(api: Api, run: string | undefined, runs: CaptureRun[], state: LiveState, said: (s: string) => void) {
  return (input: CaptureInput) => {
    if (!run) return;
    api
      .captureInput(run, input)
      .then((r) =>
        said(
          runs.find((x) => x.run === run)?.watched || state.watch
            ? `input ${r.inputs} sent; the run's watcher applies it`
            : `input ${r.inputs} sent; the next round applies it (gp capture resume ${run}, or watch it: gp capture watch ${run})`,
        ),
      )
      .catch((e: Error) => said(e.message));
  };
}
