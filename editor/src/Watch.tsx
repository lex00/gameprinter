// The watch view (docs/watch.md): a watch session as it happens, opened by
// ?watch=NAME. It follows the session's events (the stream, polling when
// that fails), folds them (watchfold.ts) and draws the game playing a
// recipe the session ran in gp_play.wasm, the PRG map filling in bank by
// bank as code first runs, the places the static reader stopped and the
// runs that went past them, the search's ticker and kept clips, and a feed
// in words. Clicking a line of the feed from a run asks for that frame's
// instructions (L3, made again by gp serve); clicking the game asks which
// routine wrote the sprites there. The page posts what it shows as a `view`
// input, for the agent's view_now, and lists the agent's calls.

import { useEffect, useRef, useState } from "react";
import type { Api } from "./api";
import { follow, type Transport } from "./Live";
import { NesPlayer } from "./nesplayer";
import { inputLine, watchDetail, watchEvents, watchInput, watchInputs, watchRecipe, watchRom, watchStream, type Detail, type WatchInput } from "./watchapi";
import { bankRows, emptyWatch, fold, type Seen, type WatchState } from "./watchfold";

/** The recipe the game shows: a kept clip, else a start clip. */
interface Shown {
  recipe: string;
  label: string;
  /** The gp-nes hash at its last frame, when the session gave one. */
  endHash?: string;
}

/** What the page follows of a session, folded. */
export function useWatch(api: Api, name: string): { state: WatchState; transport?: Transport; error?: string } {
  const [state, setState] = useState<WatchState>(emptyWatch);
  const [transport, setTransport] = useState<Transport>();
  const [error, setError] = useState<string>();
  useEffect(() => {
    setState(emptyWatch());
    return follow(
      { page: (after, signal) => watchEvents(api, name, after, signal), stream: (after) => watchStream(api, name, after) },
      { events: (evs) => setState((s) => fold(s, evs)), transport: setTransport, error: setError },
    );
  }, [api, name]);
  return { state, transport, error };
}

/** The game: the session's ROM replaying a recipe to its end in slot 1. A click asks what's under it. */
function Game({ api, name, shown, onPoint }: { api: Api; name: string; shown?: Shown; onPoint: (i: WatchInput) => void }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [status, setStatus] = useState<string>("");
  const [hash, setHash] = useState<string>();
  const [frame, setFrame] = useState<number>();
  const [pointing, setPointing] = useState<string>();
  const click = (ev: { clientX: number; clientY: number }) => {
    const c = canvas.current;
    if (!c || !shown || frame === undefined) return;
    const r = c.getBoundingClientRect();
    const x = Math.floor(((ev.clientX - r.left) / r.width) * 256);
    const y = Math.floor(((ev.clientY - r.top) / r.height) * 240);
    setPointing(`asking what's at ${x}, ${y}...`);
    watchInput(api, name, { kind: "point", data: { recipe: shown.recipe, frame, rect: [Math.max(0, x - 4), Math.max(0, y - 4), 8, 8] } })
      .then((i) => {
        setPointing(inputLine(i));
        onPoint(i);
      })
      .catch((e: Error) => setPointing(`can't point: ${e.message}`));
  };
  useEffect(() => {
    if (!shown) return;
    let live = true;
    setStatus("loading");
    setHash(undefined);
    (async () => {
      const [player, rom, log] = await Promise.all([NesPlayer.load(api.playWasm()), watchRom(api, name), watchRecipe(api, name, shown.recipe)]);
      if (!live) return;
      player.replay(1, rom, log);
      while (player.advance(1)) {
        // To the recipe's last frame.
      }
      const h = player.hash(1);
      const ctx = canvas.current?.getContext("2d");
      if (ctx) ctx.putImageData(new ImageData(player.rgba(1), 256, 240), 0, 0);
      setHash(h);
      setFrame(player.frame(1));
      setStatus(`frame ${player.frame(1)}`);
    })().catch((e: Error) => live && setStatus(`can't play: ${e.message}`));
    return () => {
      live = false;
    };
    // The recipe, not the object: the view makes a new one each render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [api, name, shown?.recipe]);
  const matches = hash && shown?.endHash ? hash === shown.endHash : undefined;
  return (
    <section className="watch-game">
      <h3>{shown ? shown.label : "No run yet"}</h3>
      <canvas ref={canvas} width={256} height={240} data-testid="watch-canvas" onClick={click} title="Click a sprite to find the code that wrote it" />
      <p data-testid="watch-game-status" data-hash={hash ?? ""} data-matches={matches === undefined ? "" : String(matches)}>
        {status}
        {matches === true ? "; the replay's last frame matches the session's" : matches === false ? "; the replay's last frame differs from the session's" : ""}
      </p>
      {pointing && <p data-testid="watch-point">{pointing}</p>}
    </section>
  );
}

/** The PRG map: a row a bank, a cell a 1 KB page, lit once code ran there. */
export function BankMap({ state }: { state: WatchState }) {
  const rows = bankRows(state);
  const lit = Object.keys(state.pages).length;
  return (
    <section className="watch-banks" data-testid="watch-banks" data-lit={lit}>
      <h3>
        PRG map: {lit} of {rows.length * (rows[0]?.pages.length ?? 0)} KB pages ran, {state.codeBytes} bytes
      </h3>
      {rows.map((r) => (
        <div key={r.bank} className="watch-bank" data-bank={r.bank} data-lit={r.pages.filter((p) => p.seen).length}>
          <span className="watch-bank-label">bank {r.bank}</span>
          {r.pages.map((p) => (
            <span key={p.page} className={p.seen ? "watch-cell lit" : "watch-cell"} title={p.seen ? `page ${p.page}: first ran at frame ${p.seen.frame} (event ${p.seen.seq})` : `page ${p.page}: not run`} />
          ))}
        </div>
      ))}
    </section>
  );
}

/** The places the static reader stopped, and whether a run went past each. */
export function Stops({ state }: { state: WatchState }) {
  return (
    <section className="watch-stops">
      <h3>Where the reader stopped</h3>
      {state.stops.length === 0 && <p>Nowhere: the reader read every routine it found.</p>}
      <ul>
        {state.stops.map((s) => (
          <li key={s.id} data-testid="watch-stop" data-reached={s.reached ? "yes" : "no"}>
            {s.pc}
            {s.target ? ` to ${s.target}` : ""}, {s.behind} routines behind it:{" "}
            {s.reached ? `a run went past, into ${s.reached.routine} at frame ${s.reached.frame}` : "no run has gone past yet"}
          </li>
        ))}
      </ul>
    </section>
  );
}

/** A session's inputs, asked for every two seconds: the agent's calls and the points. */
function useInputs(api: Api, name: string): [WatchInput[], (i: WatchInput) => void] {
  const [inputs, setInputs] = useState<WatchInput[]>([]);
  useEffect(() => {
    let live = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const ask = () =>
      watchInputs(api, name)
        .then((i) => live && setInputs(i))
        .catch(() => undefined)
        .finally(() => {
          if (live) timer = setTimeout(ask, 2000);
        });
    void ask();
    return () => {
      live = false;
      if (timer) clearTimeout(timer);
    };
  }, [api, name]);
  return [inputs, (i) => setInputs((xs) => [...xs, i])];
}

export function WatchBody({ api, name, state, transport, error, inputs = [], addInput = () => undefined }: { api: Api; name: string; state: WatchState; transport?: Transport; error?: string; inputs?: WatchInput[]; addInput?: (i: WatchInput) => void }) {
  const [chosen, setChosen] = useState<Shown>();
  const [detail, setDetail] = useState<{ at: Seen; d?: Detail; error?: string }>();
  const last = state.kept[state.kept.length - 1];
  const reachedRun = state.stops.find((s) => s.reached)?.reached?.run;
  const fallback: Shown | undefined = last
    ? { recipe: last.recipe, label: `Kept clip ${last.id}: ${last.why}`, endHash: last.endHash }
    : reachedRun
      ? { recipe: reachedRun, label: "The start run that went past a stop" }
      : undefined;
  const shown = chosen ?? fallback;
  // What the page shows, for the agent's view_now.
  const said = useRef<string>(undefined);
  useEffect(() => {
    if (!shown || said.current === shown.recipe) return;
    said.current = shown.recipe;
    watchInput(api, name, { kind: "view", data: { recipe: shown.recipe, label: shown.label } }).catch(() => undefined);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [api, name, shown?.recipe]);
  const ask = (at: Seen) => {
    if (!at.run || at.frame === undefined) return;
    setDetail({ at });
    watchDetail(api, name, at.run, at.frame, at.frame)
      .then((d) => setDetail({ at, d }))
      .catch((e: Error) => setDetail({ at, error: e.message }));
  };
  const p = state.progress;
  return (
    <div className="watch">
      <header>
        <h2>Watching {name}</h2>
        <p data-testid="watch-ticker">
          {state.read ? `Stage 0 reads ${state.read.read} of ${state.read.routines} routines (${state.readAlone ?? state.read.read} from the ROM alone). ` : ""}
          {p ? `Search: ${p.ran} of ${p.budget} candidates run, ${p.kept} kept. ` : ""}
          {state.ended ? "Done." : "Running."} {transport ? `(${transport})` : ""} {error ? `Error: ${error}` : ""}
        </p>
      </header>
      <Game api={api} name={name} shown={shown} onPoint={addInput} />
      <BankMap state={state} />
      <Stops state={state} />
      <section className="watch-edges" data-testid="watch-edges">
        <h3>Transfers of control and pointer values</h3>
        <p>
          {`${state.edges} edges first taken: ${Object.entries(state.edgeKinds)
            .sort((a, b) => b[1] - a[1])
            .map(([k, n]) => `${n} ${k}`)
            .join(", ")}`}
        </p>
        {state.pointers.length > 0 && (
          <ul>
            {state.pointers.map((p) => (
              <li key={p.seq}>
                jump through ${p.ptr.toString(16).toUpperCase().padStart(4, "0")} to {p.to}, first at frame {p.frame}
              </li>
            ))}
          </ul>
        )}
      </section>
      <section className="watch-behaviour" data-testid="watch-behaviour">
        <h3>Behaviour in the runs</h3>
        {state.behaviour.length === 0 && <p>{state.ended ? "No detector found anything." : "Read once the search ends."}</p>}
        <ul>
          {state.behaviour.map((b) => (
            <li key={b.seq}>
              {b.what}
              {b.routine ? `, ${b.routine}` : ""}: {b.why}
            </li>
          ))}
        </ul>
      </section>
      <section className="watch-kept">
        <h3>Clips the search kept</h3>
        <ul>
          {state.kept.map((k) => (
            <li key={k.seq}>
              <button onClick={() => setChosen({ recipe: k.recipe, label: `Kept clip ${k.id}: ${k.why}`, endHash: k.endHash })}>Play</button> {k.id}, {k.frames} frames from {k.start}: {k.why}
            </li>
          ))}
        </ul>
      </section>
      <section className="watch-inputs">
        <h3>The agent and you</h3>
        <ol data-testid="watch-inputs">
          {inputs
            .filter((i) => i.kind !== "view" || i.actor === "agent")
            .map((i, n) => (
              <li key={n} data-kind={i.kind} data-actor={i.actor}>
                {inputLine(i)}
              </li>
            ))}
        </ol>
      </section>
      <section className="watch-feed">
        <h3>What happened</h3>
        <ol data-testid="watch-feed">
          {state.feed.map((f) => (
            <li key={f.seq} data-kind={f.kind}>
              {f.run && f.frame !== undefined ? (
                <button className="link" onClick={() => ask(f)} title="Show this frame's instructions">
                  {f.text}
                </button>
              ) : (
                f.text
              )}
            </li>
          ))}
        </ol>
      </section>
      {detail && (
        <section className="watch-detail" data-testid="watch-detail">
          <h3>
            Instructions of frame {detail.at.frame} (event {detail.at.seq})
          </h3>
          {detail.error ? <p>{detail.error}</p> : detail.d ? <pre>{detail.d.trace.split("\n").slice(0, 400).join("\n")}</pre> : <p>Running the recipe again...</p>}
        </section>
      )}
    </div>
  );
}

export function WatchPanel({ api, name }: { api: Api; name: string }) {
  const w = useWatch(api, name);
  const [inputs, addInput] = useInputs(api, name);
  return <WatchBody api={api} name={name} {...w} inputs={inputs} addInput={addInput} />;
}
