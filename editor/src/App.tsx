// The workspace's shell. With a capture run open (?run=NAME), the page is
// the main screen (Unified.tsx, docs/show.md): the original played live,
// one row of controls, the line about the background work and the save
// state filmstrip. The run is chosen once: opening the page without one
// opens the run last played in this browser. Everything else is in one
// drawer, Details, closed by default: the recording measured against the
// rebuilt game (GameView.tsx, with its charts, ladder and moments), the
// run's stages, numbers and log, and the trace inspector (pick a
// recording, and a second one to compare, scrub to a frame, and every
// registered view draws that frame). With no run the drawer is open. The
// URL holds the state (?a=ID&b=ID&frame=N&run=NAME), so a link opens the
// same view.

import { WatchPanel } from "./Watch";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Annotations } from "./Annotations";
import { Explainer, GameView, dismissExplainer, explainerDismissed } from "./GameView";
import { LivePanel, personInput, sendInput, useLive } from "./Live";
import { Unified } from "./Unified";
import { PlayPanel } from "./Play";
import { Api, type CaptureRun, type DiffJson, type GameWords, type PairJson, type RecordingEntry } from "./api";
import { heldNames } from "./decode/input";
import { FrameCache, loadRecording, type Pane } from "./pane";
import { recordSeek, seekStats } from "./seek";
import { modelFrameAt } from "./views/pairing";
import { registeredViews } from "./views/registry";
import "./views/index";
import { LogPanel, MechanicView, type View, ViewTabs, readView, useMechanic, writeView } from "./Views";

interface Selection {
  a?: string;
  b?: string;
  /** A run of the recording's model, shown beside it. */
  m?: string;
  /** A capture run, shown in the live panel. */
  run?: string;
  frame: number;
}

export function readUrl(search: string): Selection {
  const p = new URLSearchParams(search);
  const frame = Number(p.get("frame") ?? "0");
  return { a: p.get("a") ?? undefined, b: p.get("b") ?? undefined, m: p.get("m") ?? undefined, run: p.get("run") ?? undefined, frame: Number.isFinite(frame) && frame >= 0 ? Math.floor(frame) : 0 };
}

export function writeUrl(s: Selection): string {
  const p = new URLSearchParams();
  if (s.a) p.set("a", s.a);
  if (s.b) p.set("b", s.b);
  if (s.m) p.set("m", s.m);
  if (s.run) p.set("run", s.run);
  p.set("frame", String(s.frame));
  return `?${p.toString()}`;
}

function useCache(api: Api, id: string | undefined) {
  const [cache, setCache] = useState<FrameCache>();
  const [error, setError] = useState<string>();
  useEffect(() => {
    setCache(undefined);
    setError(undefined);
    if (!id) return;
    let live = true;
    loadRecording(api, id)
      .then((rec) => live && setCache(new FrameCache(api, rec)))
      .catch((e: Error) => live && setError(e.message));
    return () => {
      live = false;
    };
  }, [api, id]);
  return { cache, error };
}

const defaultApi = new Api();

/** A frame number box that seeks on Enter or when it loses focus, not on every keystroke. */
function FrameInput({ frame, max, onCommit }: { frame: number; max: number; onCommit: (f: number) => void }) {
  const [text, setText] = useState(String(frame));
  useEffect(() => setText(String(frame)), [frame]);
  const commit = () => {
    const n = Number(text);
    if (text.trim() === "" || !Number.isFinite(n)) setText(String(frame));
    else onCommit(n);
  };
  return (
    <input
      type="number"
      min={0}
      max={max}
      value={text}
      onChange={(e) => setText(e.target.value)}
      onKeyDown={(e) => e.key === "Enter" && commit()}
      onBlur={commit}
      data-testid="frame-input"
    />
  );
}

const LAST_RUN = "gp.lastRun";

/** The run last opened in this browser; storage can be missing or refuse. */
function lastRun(): string | undefined {
  try {
    return localStorage.getItem(LAST_RUN) ?? undefined;
  } catch {
    return undefined;
  }
}

function rememberRun(run: string): void {
  try {
    localStorage.setItem(LAST_RUN, run);
  } catch {
    // Not kept; the picker asks again next time.
  }
}

/** The page: a watch session's view with ?watch=NAME (docs/watch.md), else the workspace. */
export function App({ api = defaultApi }: { api?: Api }) {
  const q = new URLSearchParams(typeof location === "undefined" ? "" : location.search);
  // A watch panel with no game to play (gp mechanic watch GAME TOPIC).
  if (q.get("view") === "watch" && q.get("game") && q.get("topic")) return <LogPanel api={api} game={q.get("game")!} topic={q.get("topic")!} />;
  const watch = q.get("watch");
  return watch ? <WatchPanel api={api} name={watch} /> : <Workspace api={api} />;
}

function Workspace({ api = defaultApi }: { api?: Api }) {
  // The one header's tabs (Views.tsx): the view and the agent's log beside it.
  const [view, setViewState] = useState<View>(() => readView(typeof location === "undefined" ? "" : location.search));
  const viewRef = useRef(view);
  viewRef.current = view;
  const setView = (v: View) => {
    setViewState(v);
    if (typeof history !== "undefined") history.replaceState(null, "", writeView(location.search, v));
  };
  const [log, setLog] = useState(false);
  const [list, setList] = useState<RecordingEntry[]>([]);
  const [listError, setListError] = useState<string>();
  const [sel, setSel] = useState<Selection>(() => readUrl(typeof location === "undefined" ? "" : location.search));
  const A = useCache(api, sel.a);
  const B = useCache(api, sel.b);
  const M = useCache(api, sel.m);
  const [pair, setPair] = useState<PairJson>();
  const [pairError, setPairError] = useState<string>();
  const [panes, setPanes] = useState<{ a?: Pane; b?: Pane; m?: Pane }>({});
  const [diff, setDiff] = useState<DiffJson>();
  const [frameError, setFrameError] = useState<string>();
  const seekStart = useRef<{ frame: number; t: number } | undefined>(undefined);
  const [lastSeek, setLastSeek] = useState<number>();
  // The play panel (docs/show.md), open with ?play in the URL or the button.
  const [play, setPlay] = useState(() => typeof location !== "undefined" && new URLSearchParams(location.search).has("play"));
  // The run's events, folded for the game view and the details alike.
  const events = useLive(api, sel.run);
  const mechanic = useMechanic(api, log ? sel.run : undefined);
  const [game, setGame] = useState<GameWords>();
  const [runs, setRuns] = useState<CaptureRun[]>([]);
  const [said, setSaid] = useState<string>();
  const [explain, setExplain] = useState(() => !explainerDismissed());
  const [details, setDetails] = useState(() => typeof location !== "undefined" && new URLSearchParams(location.search).has("details"));
  useEffect(() => {
    setGame(undefined);
    if (!sel.run) return;
    let live = true;
    api
      .captureGame(sel.run)
      .then((g) => live && setGame(g))
      .catch(() => live && setGame(undefined));
    return () => {
      live = false;
    };
  }, [api, sel.run]);
  useEffect(() => {
    api
      .captureRuns()
      .then((r) => {
        setRuns(r.runs);
        // The run is chosen once: the last one played opens by itself.
        const last = lastRun();
        if (!sel.run && !sel.a && last && r.runs.some((x) => x.run === last)) setSel((s) => (s.run ? s : { ...s, run: last }));
      })
      .catch(() => setRuns([]));
  }, [api, sel.run]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    if (sel.run) rememberRun(sel.run);
  }, [sel.run]);
  const gameShown = !!sel.run;

  useEffect(() => {
    api
      .recordings()
      .then((r) => setList(r.recordings))
      .catch((e: Error) => setListError(e.message));
  }, [api]);

  const frames = Math.min(A.cache?.recording.manifest.frames ?? 0, B.cache?.recording.manifest.frames ?? Infinity);

  const setFrame = useCallback(
    (f: number) => {
      const clamped = Math.max(0, Math.min(frames || 0, Math.round(f)));
      seekStart.current = { frame: clamped, t: performance.now() };
      setSel((s) => ({ ...s, frame: clamped }));
    },
    [frames],
  );

  // Keep the URL in step.
  useEffect(() => {
    if (typeof history !== "undefined") history.replaceState(null, "", writeView(writeUrl(sel), viewRef.current));
  }, [sel]);

  // In diff mode, find the first difference and go there.
  useEffect(() => {
    setDiff(undefined);
    setFrameError(undefined);
    if (!sel.a || !sel.b) return;
    let live = true;
    api
      .diff(sel.a, sel.b)
      .then((d) => {
        if (!live) return;
        setDiff(d);
        if (d.frame !== null) {
          seekStart.current = { frame: d.frame, t: performance.now() };
          setSel((s) => ({ ...s, frame: d.frame! }));
        }
      })
      .catch((e: Error) => live && setFrameError(e.message));
    return () => {
      live = false;
    };
  }, [api, sel.a, sel.b]);

  // Line a model run up with the recording.
  useEffect(() => {
    setPair(undefined);
    setPairError(undefined);
    if (!sel.a || !sel.m) return;
    let live = true;
    api
      .pair(sel.a, sel.m)
      .then((p) => live && setPair(p))
      .catch((e: Error) => live && setPairError(e.message));
    return () => {
      live = false;
    };
  }, [api, sel.a, sel.m]);

  // Load the frame for each open recording.
  useEffect(() => {
    if (!A.cache) return;
    let live = true;
    // Moving on aborts this frame's requests, so a drag across the slider
    // doesn't queue every frame it passes on the server.
    const abort = new AbortController();
    const frame = Math.min(sel.frame, frames);
    if (!seekStart.current || seekStart.current.frame !== frame) seekStart.current = { frame, t: performance.now() };
    const modelFrame = modelFrameAt(pair, frame);
    Promise.all([
      A.cache.pane(frame, abort.signal),
      B.cache ? B.cache.pane(frame, abort.signal) : Promise.resolve(undefined),
      M.cache && pair ? M.cache.pane(Math.min(modelFrame, M.cache.recording.manifest.frames), abort.signal) : Promise.resolve(undefined),
    ])
      .then(([a, b, m]) => {
        if (!live) return;
        setFrameError(undefined);
        setPanes({ a, b, m });
      })
      .catch((e: Error) => live && setFrameError(e.message));
    return () => {
      live = false;
      abort.abort();
    };
  }, [A.cache, B.cache, M.cache, pair, sel.frame, frames]);

  // A seek ends when its frame has been drawn: after React commits and the
  // browser paints.
  useEffect(() => {
    const start = seekStart.current;
    if (!panes.a || !start || start.frame !== panes.a.frame) return;
    seekStart.current = undefined;
    const done = () => {
      const ms = performance.now() - start.t;
      recordSeek(start.frame, ms);
      setLastSeek(ms);
    };
    if (typeof requestAnimationFrame === "undefined") done();
    else requestAnimationFrame(() => setTimeout(done, 0));
  }, [panes]);

  // Arrow keys step a frame, with shift ten; Home and End go to the ends.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target;
      if ((t instanceof HTMLInputElement && t.type !== "range") || t instanceof HTMLSelectElement) return;
      // The game's keys while the play panel's screen has focus.
      if (t instanceof HTMLCanvasElement && t.dataset.testid === "play-screen") return;
      const step = e.shiftKey ? 10 : 1;
      if (e.key === "ArrowRight") setFrame(sel.frame + step);
      else if (e.key === "ArrowLeft") setFrame(sel.frame - step);
      else if (e.key === "Home") setFrame(0);
      else if (e.key === "End") setFrame(frames);
      else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [sel.frame, frames, setFrame]);

  const views = useMemo(() => registeredViews().filter((v) => !v.diffOnly || sel.b), [sel.b]);
  const a = panes.a;
  const shown = a && a.id === A.cache?.recording.id && a.frame === Math.min(sel.frame, frames);
  // Views draw only panes of the recordings now open: after switching, the
  // old recording's frame isn't shown under the new one's name.
  const current =
    a &&
    a.id === A.cache?.recording.id &&
    (panes.b?.id ?? undefined) === (B.cache?.recording.id ?? undefined) &&
    (!pair || (panes.m?.id ?? undefined) === (M.cache?.recording.id ?? undefined));
  const input = a && a.frame > 0 ? a.input[a.frame - 1] : undefined;
  const stats = seekStats();

  const send = sendInput(api, sel.run, runs, events.state, setSaid);
  const title = game?.vocab.title;

  // The page's bar, the explainer and the play panel: at the top when no
  // run is open, under the two games' controls when one is, so the two
  // games are the first thing on the page.
  const bar = (
    <>
      <header className="top">
        <h1>{title ? `${title} in Game Printer` : "Game Printer"}</h1>
        <label>
          run{" "}
          <select data-testid="pick-game" value={sel.run ?? ""} onChange={(e) => setSel((s) => ({ ...s, run: e.target.value || undefined }))}>
            <option value="">none</option>
            {runs.map((r) => (
              <option key={r.run} value={r.run}>
                {r.run}
              </option>
            ))}
          </select>
        </label>
        <button data-testid="toggle-play" onClick={() => setPlay(!play)}>
          {play ? "hide the play panel" : "play the original"}
        </button>
        {(() => {
          // The rebuilt game, playable: the chosen run's, else the first
          // run that has one. Always here, so it doesn't hide behind a run.
          const r = runs.find((x) => x.run === sel.run && x.preview) ?? runs.find((x) => x.preview);
          return (
            <a
              className={`button${r ? "" : " disabled"}`}
              data-testid="top-play-rebuild"
              href={r ? api.capturePreview(r.run, "index.html") : undefined}
              target="_blank"
              rel="noopener"
              aria-disabled={!r}
              title={r ? `Play ${r.run}'s rebuilt game in a new tab: arrows move, Z jumps, X runs, Enter starts` : "No run has a rebuilt game yet"}
            >
              play the rebuild
            </a>
          );
        })()}
        <button data-testid="explainer-open" onClick={() => setExplain(true)}>
          What am I looking at?
        </button>
      </header>
      <Explainer
        open={explain}
        playing={play && !sel.run}
        onClose={() => {
          setExplain(false);
          dismissExplainer();
        }}
      />
      {play && <PlayPanel api={api} onRecorded={(run) => setSel((s) => ({ ...s, run }))} />}
    </>
  );

  return (
    <div className={gameShown ? "app with-game" : "app"}>
      {sel.run ? (
        <>
          <ViewTabs view={view} setView={setView} log={log} setLog={setLog} />
          <div className={log ? "views-row with-log" : "views-row"}>
            <div className="view-main">{view === "clips" ? <Unified key={sel.run} api={api} run={sel.run} words={game} /> : <MechanicView key={sel.run} api={api} run={sel.run} view={view} />}</div>
            {log && <LogPanel api={api} game={sel.run} topic={mechanic} />}
          </div>
          {bar}
        </>
      ) : (
        bar
      )}
      {said && <p className="hint">{said}</p>}
      {!sel.run && !play && <p className="hint">Choose a run above to play it, or press "play the original".</p>}

      <details className="details" open={!gameShown || details} onToggle={(e) => setDetails((e.target as HTMLDetailsElement).open)} data-testid="details">
        <summary>Details: the recording measured against the rebuilt game (charts, the scenario ladder, moments), the run's stages and log, and the trace inspector</summary>
        {sel.run && details && (
          <div className="details-recording">
            <h2>The recording, measured</h2>
            <GameView api={api} run={sel.run} words={game} live={events.state} play={events.play} send={(i) => send(personInput(i))} />
          </div>
        )}
        <div className={sel.run ? "details-body has-run" : "details-body"}>
          <LivePanel api={api} run={sel.run} events={events} setRun={(run: string | undefined) => setSel((s) => ({ ...s, run }))} open={(a) => setSel((s) => ({ ...s, a, frame: 0 }))} />
          <div className="inspector">
            <header>
              <h2>Trace inspector</h2>
              <label>
                Recording{" "}
                <select
                  data-testid="pick-a"
                  value={list.find((r) => sel.a && r.id.startsWith(sel.a))?.id ?? ""}
                  onChange={(e) => setSel({ a: e.target.value || undefined, b: sel.b, m: sel.m, run: sel.run, frame: 0 })}
                >
                  <option value="">choose a recording</option>
                  {list.map((r) => (
                    <option key={r.id} value={r.id}>
                      {r.name} ({r.id.slice(0, 8)}, {r.frames} frames)
                    </option>
                  ))}
                </select>
              </label>
              <label>
                model run{" "}
                <select
                  data-testid="pick-m"
                  value={list.find((r) => sel.m && r.id.startsWith(sel.m))?.id ?? ""}
                  onChange={(e) => setSel({ ...sel, m: e.target.value || undefined })}
                >
                  <option value="">nothing</option>
                  {list.map((r) => (
                    <option key={r.id} value={r.id}>
                      {r.name} ({r.id.slice(0, 8)})
                    </option>
                  ))}
                </select>
              </label>
              <label>
                compare with{" "}
                <select
                  data-testid="pick-b"
                  value={list.find((r) => sel.b && r.id.startsWith(sel.b))?.id ?? ""}
                  onChange={(e) => setSel({ ...sel, b: e.target.value || undefined })}
                >
                  <option value="">nothing</option>
                  {list.map((r) => (
                    <option key={r.id} value={r.id}>
                      {r.name} ({r.id.slice(0, 8)})
                    </option>
                  ))}
                </select>
              </label>
              {listError && <span className="error">{listError}</span>}
            </header>

            {A.error && <p className="error">{A.error}</p>}
            {B.error && <p className="error">{B.error}</p>}
            {M.error && <p className="error">{M.error}</p>}
            {pairError && <p className="error">{pairError}</p>}
            {!sel.a && <p className="hint">Choose a recording. `gp recordings list` shows the same list.</p>}

            {A.cache && (
              <section className="scrubber">
                <button onClick={() => setFrame(sel.frame - 1)} aria-label="previous frame">
                  &lt;
                </button>
                <input
                  type="range"
                  min={0}
                  max={frames}
                  value={Math.min(sel.frame, frames)}
                  onChange={(e) => setFrame(Number(e.target.value))}
                  data-testid="scrubber"
                />
                <button onClick={() => setFrame(sel.frame + 1)} aria-label="next frame">
                  &gt;
                </button>
                <label>
                  frame <FrameInput frame={Math.min(sel.frame, frames)} max={frames} onCommit={setFrame} /> of {frames}
                </label>
                <span className="status" data-testid="status">
                  {shown ? `frame ${a.frame} from the ${a.from}` : "loading"}
                  {input ? `; input ${heldNames(input.pads[0] ?? "")}${input.polled ? "" : " (lag frame)"}` : ""}
                  {lastSeek !== undefined ? `; seek ${lastSeek.toFixed(0)} ms (slowest ${stats.max.toFixed(0)} ms)` : ""}
                </span>
              </section>
            )}
            {A.cache && <Annotations api={api} id={A.cache.recording.id} frames={frames} frame={Math.min(sel.frame, frames)} setFrame={setFrame} />}
            {frameError && <p className="error">{frameError}</p>}

            {a && current && (
              <main className="views">
                {views.map((v) => (
                  <section key={v.id} className={v.wide ? "view wide" : "view"} data-view={v.id}>
                    <h2>{v.title}</h2>
                    <v.component api={api} frame={a.frame} a={a} b={panes.b} diff={diff} m={pair ? panes.m : undefined} pair={pair} setFrame={setFrame} />
                  </section>
                ))}
              </main>
            )}
          </div>
        </div>
      </details>
    </div>
  );
}
