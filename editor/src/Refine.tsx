// The game view's "Refining" section, under the lockstep (docs/inspector.md,
// "The game view"): starts the refinement loop on the run's working copy,
// from the paused frame or from the start, and shows it as it goes. Each
// round is a sentence with a "go there" button; while it searches, the
// number it's trying. After each round the rebuilt game loads the new
// numbers (the preview's `bundle` message) and both games play the window
// once, and the lockstep above is measured again. A stuck window shows its
// question, with the two games' values and the general mechanic proposed.
// The words come from refinefold.ts.

import { useCallback, useEffect, useRef, useState } from "react";
import type { Api, CaptureInput, GameWords } from "./api";
import { type RefineState, measureWords, refineStatus, roundLine, tryingLine } from "./refinefold";

export interface RefinePanelProps {
  api: Api;
  run: string;
  words?: GameWords;
  recording?: string;
  /** The view's frame, the original's. */
  frame: number;
  paused: boolean;
  /** Both games to an original frame. */
  seek: (frame: number) => void;
  /** Both games play the original's frames `from` to `to` once. */
  watch: (from: number, to: number) => void;
  /** The model changed: measure the lockstep again. */
  changed: () => void;
  send: (i: Omit<CaptureInput, "v" | "actor">) => void;
  /** The loop's state as it changes, so the view can focus on its window. */
  onState?: (st: RefineState | undefined) => void;
  /** Changed when a loop is started elsewhere (refine this moment): read it. */
  kick?: number;
}

const POLL_MS = 400;

/** The rebuilt game's frame, told to load the run's bundle again. */
function reloadModel() {
  const f = document.querySelector<HTMLIFrameElement>('[data-testid="game-model"]');
  f?.contentWindow?.postMessage({ gp: "bundle" }, location.origin);
}

export function RefinePanel({ api, run, words: g, recording, frame, paused, seek, watch, changed, send, onState, kick }: RefinePanelProps) {
  const [st, setSt] = useState<RefineState>();
  useEffect(() => onState?.(st), [st]); // eslint-disable-line react-hooks/exhaustive-deps
  const [error, setError] = useState<string>();
  const seen = useRef(0);
  const read = useCallback(
    () =>
      recording
        ? api.captureRefine(run, recording).then((s) => {
            setSt(s);
            return s;
          })
        : Promise.resolve(undefined),
    [api, run, recording],
  );

  // The last loop's result when the view opens, without replaying its rounds.
  useEffect(() => {
    read()
      .then((s) => (seen.current = s?.rounds.length ?? 0))
      .catch(() => undefined);
  }, [read]);

  // A loop started elsewhere: read it, and its rounds play as they come.
  useEffect(() => {
    if (!kick) return;
    seen.current = 0;
    read().catch((e: Error) => setError(e.message));
  }, [kick]); // eslint-disable-line react-hooks/exhaustive-deps

  // While it runs: poll, and play each new round's window with the new numbers.
  useEffect(() => {
    if (!st?.running) return;
    const t = setTimeout(() => {
      read().catch((e: Error) => setError(e.message));
    }, POLL_MS);
    return () => clearTimeout(t);
  }, [st, read]);
  useEffect(() => {
    const n = st?.rounds.length ?? 0;
    if (n <= seen.current) return;
    seen.current = n;
    const r = st!.rounds[n - 1];
    reloadModel();
    changed();
    watch(r.window.original_from, r.window.original_to);
  }, [st?.rounds.length]); // eslint-disable-line react-hooks/exhaustive-deps
  // Once it stops, the lockstep is measured again on the final numbers.
  const was = useRef(false);
  useEffect(() => {
    if (was.current && st && !st.running) changed();
    was.current = !!st?.running;
  }, [st?.running]); // eslint-disable-line react-hooks/exhaustive-deps

  const start = (from?: number) => {
    if (!recording) return;
    setError(undefined);
    seen.current = 0;
    api
      .captureRefineStart(run, recording, from)
      .then((r) => {
        if (!r.started) setError(r.why ?? "the loop didn't start");
        return read();
      })
      .catch((e: Error) => setError(e.message));
  };
  const stop = () => {
    api
      .captureRefineStop(run)
      .then(read)
      .catch((e: Error) => setError(e.message));
  };

  if (!recording) return null;
  const running = !!st?.running;
  const questions = st?.state === "stuck" ? (st.result?.questions ?? []) : [];
  return (
    <section className="refine" data-testid="refine">
      <h2>Refining</h2>
      <div className="lockstep-controls">
        {!running && paused && (
          <button onClick={() => start(frame)} data-testid="refine-here" title="Fit the model's numbers window by window, starting at the first split from this frame">
            refine from here
          </button>
        )}
        {!running && (
          <button onClick={() => start(undefined)} data-testid="refine-start" title="Fit the model's numbers window by window from the first split">
            refine from the start
          </button>
        )}
        {running && (
          <button onClick={stop} data-testid="refine-stop">
            stop
          </button>
        )}
      </div>
      {st && st.state !== "none" && (
        <p className="lockstep-summary" data-testid="refine-status">
          {refineStatus(st)}
        </p>
      )}
      {running && st?.trying && (
        <p className="hint" data-testid="refine-trying">
          {tryingLine(g, st.trying)}
        </p>
      )}
      {error && <p className="error">{error}</p>}
      {st && st.rounds.length > 0 && (
        <ol className="refine-rounds" data-testid="refine-rounds">
          {st.rounds.map((r) => (
            <li key={r.round}>
              Round {r.round}. {roundLine(g, r)} <button onClick={() => seek(r.window.original_from)}>go there</button>
            </li>
          ))}
        </ol>
      )}
      {questions.map((q) => (
        <div key={q.id} className="refine-question" data-testid="refine-question">
          <p>
            No numbers make {q.measures.length === 1 ? "this measure" : "these measures"} exact here. <button onClick={() => seek(q.window.original_from)}>go there</button>
          </p>
          <ul>
            {q.measures.map((m) => (
              <li key={m.path}>
                {measureWords(g, m)} <span className="hint">{m.why}.</span>
              </li>
            ))}
          </ul>
          <p>Proposed, as a general mechanic other games can use: {q.proposal}.</p>
          <button className="option" onClick={() => send({ kind: "answer", question: q.id, choice: 1 })} data-testid="refine-approve">
            approve it
          </button>{" "}
          <button className="option" onClick={() => send({ kind: "answer", question: q.id, choice: 2 })}>
            keep the mechanic as it is
          </button>
        </div>
      ))}
    </section>
  );
}
