// The game view's "Scenarios" section (docs/inspector.md, "Scenarios"):
// sequences of a few presses, each run on the original and on the rebuilt
// game from the original's state at one quiet frame, and measured apart.
// The pieces take props and assume no layout, so a guided page can reuse
// them: the ladder list (ScenarioLadder), one sequence's side by side with
// its scores (ScenarioSideBySide), and the refine button with what it is
// doing (RefineOnScenario). ScenariosSection puts them together with the
// one next step. The words come from the server and scenariofold.ts.

import { useCallback, useEffect, useRef, useState } from "react";
import type { Api } from "./api";
import { NesPlayer } from "./nesplayer";
import { KEYS, frameLines, framesDue, modelSteps, padFromGamepad, padFromKeys, scriptFrom } from "./playfold";
import { ModelPreview } from "./modelpreview";
import {
  type Ladder,
  type LadderRow,
  type ScenarioInput,
  type ScenariosState,
  RECORD_SECONDS,
  ScenarioClient,
  byTier,
  inputHint,
  playedLog,
  recordSays,
  headline,
  jobSays,
  nextAction,
  nextRow,
  rowMark,
  tellingFrame,
  tierName,
} from "./scenariofold";

const W = 256;
const H = 240;
const POLL_MS = 400;

function picture(rgba: Uint8ClampedArray<ArrayBuffer>): string {
  const c = document.createElement("canvas");
  c.width = W;
  c.height = H;
  c.getContext("2d")?.putImageData(new ImageData(rgba, W, H), 0, 0);
  return c.toDataURL();
}

export interface ScenarioLadderProps {
  ladder: Ladder;
  state?: ScenariosState;
  selected?: string;
  onSelect?: (row: LadderRow) => void;
  /** A picture of the original for a row, made when asked. */
  thumb?: (row: LadderRow) => Promise<string>;
}

/** The ladder by tier: each sequence a line with its mark, what differs, its score and what it needs. */
export function ScenarioLadder({ ladder, state, selected, onSelect, thumb }: ScenarioLadderProps) {
  const [thumbs, setThumbs] = useState<Record<string, string>>({});
  // One picture at a time, in ladder order.
  useEffect(() => {
    if (!thumb) return;
    let live = true;
    (async () => {
      for (const r of ladder.scenarios) {
        if (!live) return;
        const key = `${r.scenario.id}@${tellingFrame(r)}`;
        if (thumbs[key]) continue;
        try {
          const url = await thumb(r);
          if (live) setThumbs((t) => ({ ...t, [key]: url }));
        } catch {
          return;
        }
      }
    })();
    return () => {
      live = false;
    };
  }, [ladder, thumb]); // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <div className="scenario-ladder" data-testid="scenario-ladder">
      {byTier(ladder).map((t) => (
        <div key={t.tier} className="scenario-tier">
          <h4>{`${tierName(t.tier)}: ${t.exact} of ${t.total} match`}</h4>
          <ol>
            {t.rows.map((r) => {
              const key = `${r.scenario.id}@${tellingFrame(r)}`;
              const here = ladder.start_here === r.scenario.id;
              return (
                <li key={r.scenario.id} data-testid="scenario-row" data-state={r.state} className={`scenario-row ${r.state}${selected === r.scenario.id ? " selected" : ""}`}>
                  <button type="button" onClick={() => onSelect?.(r)} disabled={!onSelect}>
                    {thumbs[key] && <img className="scenario-thumb" src={thumbs[key]} alt={`The original in ${r.scenario.name}`} width={64} height={60} />}
                    <span className="scenario-mark">{rowMark(r, state)}</span> <span className="scenario-says">{r.says}</span> <span className="scenario-headline">({headline(r.result)})</span>
                  </button>
                  {here && <strong className="scenario-here"> start here</strong>}
                  {r.trend && <div className="scenario-trend">{r.trend}</div>}
                  {r.needs && <div className="scenario-needs">{r.needs}</div>}
                  {r.unlocks && selected === r.scenario.id && <div className="scenario-unlocks">{r.unlocks}</div>}
                </li>
              );
            })}
          </ol>
        </div>
      ))}
    </div>
  );
}

export interface ScenarioSideBySideProps {
  row: LadderRow;
  /** The original's side: the input to the start and then the presses. */
  input?: ScenarioInput;
  player?: NesPlayer;
  rom?: Uint8Array;
  /** The run's preview in control mode, for the rebuilt game's side. */
  previewSrc?: string;
}

/** One sequence: the original playing it from its start, looped, and its scores by measure. */
export function ScenarioSideBySide({ row, input, player, rom, previewSrc }: ScenarioSideBySideProps) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const iframe = useRef<HTMLIFrameElement>(null);
  const model = useRef(new ModelPreview(() => iframe.current));
  const [frame, setFrame] = useState(0);
  const [modelError, setModelError] = useState<string>();
  useEffect(() => {
    const m = model.current;
    m.onError = setModelError;
    const on = (e: MessageEvent) => m.receive(e);
    window.addEventListener("message", on);
    return () => window.removeEventListener("message", on);
  }, []);
  // The rebuilt game: the original's state at the start, then the same presses.
  const lines = input ? frameLines(input.log) : [];
  useEffect(() => {
    if (!input || !previewSrc) return;
    model.current.loadState(input.state, scriptFrom(frameLines(input.log), input.start_frame), 0);
  }, [input, previewSrc]);
  useEffect(() => {
    if (input && previewSrc) model.current.seek(modelSteps(lines, input.start_frame, input.start_frame + frame));
  }, [frame, input, previewSrc]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    if (!input || !player || !rom) return;
    let live = true;
    let k = 0;
    const draw = () => {
      if (!live) return;
      if (k === 0) {
        player.replay(1, rom, input.log);
        player.runTo(1, input.start_frame);
      } else {
        player.advance(1);
      }
      canvas.current?.getContext("2d")?.putImageData(new ImageData(player.rgba(1), W, H), 0, 0);
      setFrame(k);
      k = k >= input.frames ? 0 : k + 1;
      timer = setTimeout(draw, 1000 / 60);
    };
    let timer = setTimeout(draw, 0);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [input, player, rom]);
  const r = row.result;
  return (
    <div className="scenario-side" data-testid="scenario-side">
      <p className="scenario-title">{row.says}</p>
      <div className="scenario-games">
        <figure>
          <canvas ref={canvas} width={W} height={H} aria-label={`The original playing ${row.scenario.name}`} />
          <figcaption>The original, frame {frame} of {row.scenario.frames}</figcaption>
        </figure>
        {previewSrc && !modelError ? (
          <figure>
            <iframe ref={iframe} title={`the rebuilt game playing ${row.scenario.name}`} src={previewSrc} width={W} height={H} />
            <figcaption>
              The rebuilt game, from the original's state at frame {input?.start_frame ?? "..."}, on the same presses
              {r.first && `. At frame ${r.first.frame} its ${r.first.path} is ${r.first.model}, the original's ${r.first.original}`}
            </figcaption>
          </figure>
        ) : (
          <figure className="scenario-model-missing">
            <figcaption>
              {modelError ? `The rebuilt game couldn't start from the original's state: ${modelError}. ` : "There is no rebuilt game to show; its values are in the scores below. "}
              {r.first && ` At frame ${r.first.frame} the rebuilt game's ${r.first.path} is ${r.first.model}, the original's ${r.first.original}.`}
            </figcaption>
          </figure>
        )}
      </div>
      <table className="scenario-measures" data-testid="scenario-measures">
        <thead>
          <tr>
            <th>Measure</th>
            <th>Frames together</th>
            <th>First apart</th>
            <th>Largest</th>
            <th>How far apart</th>
          </tr>
        </thead>
        <tbody>
          {r.measures.map((m) => {
            const most = Math.max(1, ...m.bins.map((b) => b.frames));
            return (
              <tr key={m.path} className={m.together === m.total ? "together" : "apart"}>
                <td>{m.path}</td>
                <td>
                  {m.together} of {m.total}
                </td>
                <td>{m.first ? `frame ${m.first.frame}: ${m.first.model} for ${m.first.original}` : "never"}</td>
                <td>{m.max}</td>
                <td>
                  <span className="scenario-bins" role="img" aria-label={`${m.path}: frames at each distance`}>
                    {m.bins.map((b, i) => (
                      <span key={i} title={`${i === 0 ? "within " + b.to : b.to ? `${b.from} to ${b.to}` : `over ${b.from}`}: ${b.frames} frames`} style={{ display: "inline-block", width: 6, height: Math.round((20 * b.frames) / most), background: i === 0 ? "var(--ok, #3a3)" : "var(--bad, #c33)", marginRight: 1 }} />
                    ))}
                  </span>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

export interface RefineOnScenarioProps {
  row: LadderRow;
  state?: ScenariosState;
  /** Starts the loop on the row; the state then streams in. */
  start: (row: LadderRow) => void;
  stop?: () => void;
}

/** "Refine on this sequence", with what the loop is trying, its rounds, and what it fixed. */
export function RefineOnScenario({ row, state, start, stop }: RefineOnScenarioProps) {
  const mine = state?.refine?.scenario === row.scenario.id ? state.refine : undefined;
  const running = state?.state === "running";
  return (
    <div className="scenario-refine" data-testid="scenario-refine">
      <button type="button" disabled={running || row.state === "passed"} onClick={() => start(row)}>
        Refine on this sequence
      </button>
      {running && stop && (
        <button type="button" onClick={stop}>
          Stop
        </button>
      )}
      {mine?.trying && (
        <p>{`Trying ${mine.trying.param} = ${mine.trying.value}`}</p>
      )}
      {mine && mine.rounds.length > 0 && (
        <ol>
          {mine.rounds.map((r) => (
            <li key={r.round}>{`${r.param}: ${r.old} -> ${r.new}`}</li>
          ))}
        </ol>
      )}
      {mine?.result && <p data-testid="scenario-refine-result">{mine.result.says}</p>}
    </div>
  );
}

export interface RecordSequenceProps {
  /** The original's frame to start from. */
  at: number;
  /** The recording's input to play to `at`. */
  log: () => Promise<string>;
  /** An emulator of its own, so the section's others go on. */
  open: () => Promise<{ player: NesPlayer; rom: Uint8Array }>;
  save: (log: string) => Promise<unknown>;
  seconds?: number;
}

/** Play it yourself: the original from its state at `at`, live, the presses recorded frame by frame for up to N seconds, then saved as a sequence. */
export function RecordSequence({ at, log, open, save, seconds = RECORD_SECONDS }: RecordSequenceProps) {
  const total = Math.round(seconds * 60);
  const canvas = useRef<HTMLCanvasElement>(null);
  const [phase, setPhase] = useState<"ready" | "count" | "recording" | "saving" | "saved">("ready");
  const [n, setN] = useState(0);
  const [pad, setPad] = useState(false);
  const [error, setError] = useState<string>();
  const down = useRef(new Set<string>());
  // While the countdown and the take run, the game's keys are the game's:
  // they don't scroll the page or pause the two games above (Space is A).
  const live = useRef(false);
  live.current = phase === "count" || phase === "recording";
  useEffect(() => {
    const game = new Set(KEYS.flat());
    const kd = (e: KeyboardEvent) => {
      down.current.add(e.code);
      if (live.current && game.has(e.code)) {
        e.preventDefault();
        e.stopImmediatePropagation();
      }
    };
    const ku = (e: KeyboardEvent) => {
      down.current.delete(e.code);
      if (live.current && game.has(e.code)) e.preventDefault();
    };
    window.addEventListener("keydown", kd, { capture: true });
    window.addEventListener("keyup", ku, { capture: true });
    const t = setInterval(() => setPad(!!navigator.getGamepads?.()[0]), 500);
    return () => {
      window.removeEventListener("keydown", kd, { capture: true });
      window.removeEventListener("keyup", ku, { capture: true });
      clearInterval(t);
    };
  }, []);
  const start = async () => {
    try {
      const [{ player: p, rom: r }, l] = await Promise.all([open(), log()]);
      p.replay(1, r, l);
      p.runTo(1, at);
      const draw = () => canvas.current?.getContext("2d")?.putImageData(new ImageData(p.rgba(1), W, H), 0, 0);
      setPhase("count");
      await new Promise((ok) => requestAnimationFrame(ok));
      canvas.current?.scrollIntoView({ block: "center", behavior: "smooth" });
      draw();
      for (let c = 3; c > 0; c--) {
        setN(c);
        await new Promise((ok) => setTimeout(ok, 700));
      }
      setPhase("recording");
      const frames: { pad: number; polled: boolean }[] = [];
      let acc = 0;
      let last = performance.now();
      await new Promise<void>((done) => {
        const tick = (now: number) => {
          const due = framesDue(acc, now - last);
          acc = due.acc;
          last = now;
          for (let i = 0; i < due.frames && frames.length < total; i++) {
            const bits = padFromKeys(down.current) | padFromGamepad(navigator.getGamepads?.()[0]);
            frames.push({ pad: bits, polled: p.step(1, bits) });
          }
          draw();
          setN(frames.length);
          if (frames.length >= total) done();
          else requestAnimationFrame(tick);
        };
        requestAnimationFrame(tick);
      });
      setPhase("saving");
      await save(playedLog(frames));
      setPhase("saved");
    } catch (e) {
      setError((e as Error).message);
      setPhase("ready");
    }
  };
  return (
    <div className="scenario-record" data-testid="scenario-record">
      <button type="button" disabled={phase === "count" || phase === "recording" || phase === "saving"} onClick={() => void start()}>
        Play it yourself from here
      </button>
      <span>{` ${recordSays(phase, n, total)} ${inputHint(pad)}`}</span>
      {phase !== "ready" && <canvas ref={canvas} width={W} height={H} className="scenario-live" aria-label="The original, live" />}
      {error && <p className="error">{error}</p>}
    </div>
  );
}

export interface ScenariosSectionProps {
  api: Api;
  run: string;
  recording?: string;
  /** Sets the view's focus to the sequence's frames of the original. */
  onFocus?: (from: number, to: number) => void;
  /** Sets the view's frame. */
  seek?: (frame: number) => void;
  /** The paused frame, the original's: a sequence is recorded from it. */
  paused?: number;
  /** Whether the run has a rebuilt game to show beside the original. */
  preview?: boolean;
}

/** The Scenarios section: the one next step, the ladder, and the sequence picked. */
export function ScenariosSection({ api, run, recording, onFocus, seek, paused, preview }: ScenariosSectionProps) {
  const previewSrc = preview ? api.capturePreview(run, "index.html?control=1") : undefined;
  const client = useRef(new ScenarioClient(run));
  useEffect(() => {
    client.current = new ScenarioClient(run);
  }, [run]);
  const [st, setSt] = useState<ScenariosState>();
  const [error, setError] = useState<string>();
  const [picked, setPicked] = useState<LadderRow>();
  const [input, setInput] = useState<ScenarioInput>();
  const [player, setPlayer] = useState<NesPlayer>();
  const [rom, setRom] = useState<Uint8Array>();
  const inputs = useRef(new Map<string, ScenarioInput>());

  const read = useCallback(() => (recording ? client.current.state(recording).then(setSt) : Promise.resolve()), [recording]);
  useEffect(() => {
    read().catch((e: Error) => setError(e.message));
  }, [read]);
  useEffect(() => {
    if (st?.state !== "running") return;
    const t = setTimeout(() => read().catch((e: Error) => setError(e.message)), POLL_MS);
    return () => clearTimeout(t);
  }, [st, read]);

  const opened = useCallback(async () => {
    if (player && rom) return { player, rom };
    const [p, r] = await Promise.all([NesPlayer.load(api.playWasm()), api.captureRom(run)]);
    setPlayer(p);
    setRom(r);
    return { player: p, rom: r };
  }, [api, run, player, rom]);

  const inputOf = useCallback(
    async (id: string) => {
      let i = inputs.current.get(id);
      if (!i && recording) {
        i = await client.current.input(recording, id);
        inputs.current.set(id, i);
      }
      return i;
    },
    [recording],
  );

  const thumb = useCallback(
    async (row: LadderRow) => {
      const { player: p, rom: r } = await opened();
      const i = await inputOf(row.scenario.id);
      if (!i) throw new Error("no input");
      // Slot 0: the side by side plays in slot 1.
      p.replay(0, r, i.log);
      p.runTo(0, tellingFrame(row));
      return picture(p.rgba(0));
    },
    [opened, inputOf],
  );

  const pick = useCallback(
    (row: LadderRow) => {
      setPicked(row);
      onFocus?.(row.result.original_from, row.result.original_to);
      seek?.(row.result.original_from);
      Promise.all([opened(), inputOf(row.scenario.id)])
        .then(([, i]) => setInput(i))
        .catch((e: Error) => setError(e.message));
    },
    [onFocus, seek, opened, inputOf],
  );

  const act = (p: Promise<{ started: boolean; why?: string }>) =>
    p
      .then((r) => (r.started ? read() : setError(r.why)))
      .catch((e: Error) => setError(e.message));
  const ladder = st?.ladder;
  const next = nextRow(ladder);
  const action = nextAction(st);
  if (!recording) return null;
  return (
    <section className="scenarios" data-testid="scenarios">
      <h3>Scenarios</h3>
      <p className="scenarios-says" data-testid="scenarios-says">
        {jobSays(st)}
        {st?.progress && ` (${st.progress.k} of ${st.progress.n})`}
      </p>
      {ladder && <p className="scenarios-start">Start: {ladder.start.says}. Passing means {ladder.bar_says}.</p>}
      <div className="scenarios-next">
        {action === "run" && (
          <button type="button" onClick={() => act(client.current.score(recording))}>
            Run the scenarios
          </button>
        )}
        {action === "refine" && next && (
          <>
            <button type="button" onClick={() => act(client.current.refine(recording, next.scenario.id))}>
              Fix: {next.scenario.name}
            </button>
            <button type="button" onClick={() => act(client.current.auto(recording))}>
              Work through them on its own
            </button>
            <button type="button" onClick={() => act(client.current.score(recording))}>
              Run again
            </button>
          </>
        )}
        {action === "wait" && (
          <button type="button" onClick={() => client.current.stop().then(read)}>
            Stop
          </button>
        )}
        {action === "record" && (
          <button type="button" onClick={() => act(client.current.score(recording))}>
            Run again
          </button>
        )}
      </div>
      {error && <p className="error">{error}</p>}
      {(paused ?? ladder?.start.original_frame) !== undefined && (
        <RecordSequence
          at={(paused ?? ladder?.start.original_frame)!}
          log={() => api.captureInputLog(run, recording)}
          open={async () => ({ player: await NesPlayer.load(api.playWasm()), rom: rom ?? (await api.captureRom(run)) })}
          save={(log) => client.current.played(recording, (paused ?? ladder?.start.original_frame)!, log).then(read)}
        />
      )}
      {ladder && <ScenarioLadder ladder={ladder} state={st} selected={picked?.scenario.id} onSelect={pick} thumb={thumb} />}
      {picked && (
        <>
          <ScenarioSideBySide row={ladder?.scenarios.find((r) => r.scenario.id === picked.scenario.id) ?? picked} input={input} player={player} rom={rom} previewSrc={previewSrc} />
          <RefineOnScenario row={picked} state={st} start={(r) => act(client.current.refine(recording, r.scenario.id))} stop={() => client.current.stop().then(read)} />
        </>
      )}
    </section>
  );
}
