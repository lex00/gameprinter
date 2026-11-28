// The editor's build, as gp serve serves it at /index.html (the older
// mechanic page framed it in its Details drawer). It draws one view with
// no header of its own: ?embed=inspector&run=GAME the trace inspector (pick a recording,
// and a second one to compare, scrub to a frame, and every registered view
// draws that frame), any other ?embed=VIEW Embed.tsx's view. With no
// ?embed= it sends the visitor to the page at /. The URL holds the
// inspector's state (?a=ID&b=ID&frame=N), so a link opens the same view.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Annotations } from "./Annotations";
import { Embed } from "./Embed";
import { Api, type DiffJson, type PairJson, type RecordingEntry } from "./api";
import { heldNames } from "./decode/input";
import { FrameCache, loadRecording, type Pane } from "./pane";
import { recordSeek, seekStats } from "./seek";
import { modelFrameAt } from "./views/pairing";
import { registeredViews } from "./views/registry";
import "./views/index";

interface Selection {
  a?: string;
  b?: string;
  /** A run of the recording's model, shown beside it. */
  m?: string;
  /** The game (?run=GAME), kept in the URL. */
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

/** The editor's one page: ?embed=inspector the trace inspector,
 * ?embed=VIEW Embed.tsx's view, else a step over to gp serve's page. */
export function App({ api = defaultApi }: { api?: Api }) {
  const q = new URLSearchParams(typeof location === "undefined" ? "" : location.search);
  const embed = q.get("embed");
  if (embed === "inspector") return <Inspector api={api} />;
  if (embed) return <Embed api={api} view={embed} run={q.get("run") ?? undefined} />;
  return <ToPage />;
}

/** The old editor's own screens are gone: /index.html with no ?embed=
 * opens gp serve's page at /, which opens the last game. */
function ToPage() {
  useEffect(() => {
    if (typeof location !== "undefined") location.replace("/");
  }, []);
  return (
    <p className="hint">
      The editor's screens moved to the page: <a href="/">open it</a>.
    </p>
  );
}

/** The trace inspector alone, for the Details drawer of gp serve's one
 * page (?embed=inspector). */
function Inspector({ api = defaultApi }: { api?: Api }) {
  useEffect(() => {
    document.documentElement.classList.add("embed");
  }, []);
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
    if (typeof history === "undefined") return;
    history.replaceState(null, "", `${writeUrl(sel)}&embed=inspector`);
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

  // The trace inspector: pick a recording, and a second one to compare,
  // scrub to a frame, and every registered view draws that frame.
  const inspector = (
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
  );

  return <div className="app embed">{inspector}</div>;
}
