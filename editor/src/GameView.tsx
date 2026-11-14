// The game view (docs/inspector.md, "The game view"): the top of the
// workspace when a capture run or a play session is open. The original
// (gp_play.wasm replaying a recording's input on the run's ROM) and the
// game rebuilt from the model (the run's interpreter preview, driven
// frame by frame) side by side at the same moment, with nothing above
// them. The rebuilt game never plays from its own power-on: it starts
// from the original's state at the view's start, translated, and gets
// the same button presses from there ("The same commands to both"). Under them the controls, the page's toolbar, a line saying in game
// terms where they differ, and the two measured apart (the lockstep).
// Below that: the questions as pictures,
// what the model misses as sentences with before and after pictures, and
// a few numbers with what they mean. Every word comes from phrase.ts,
// filled from the model; the raw data is in the details below.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Api, CaptureInput, GameWords, StartState } from "./api";
import type { LiveState } from "./livefold";
import { openGaps, openQuestions } from "./livefold";
import { LockstepPanel } from "./Lockstep";
import { LockCurve } from "./CurveChart";
import { ScreenBoxes } from "./ScreenBoxes";
import { lockBoxes, lockIndex } from "./screenfold";
import { RefinePanel } from "./Refine";
import { MomentsInFocus, MomentsStrip, RefineThisMoment } from "./Moments";
import { type Moment, type Moments, inFocus, momentName, momentsPhase } from "./momentfold";
import { momentRefinePhase } from "./refinefold";
import { ScenariosSection } from "./Scenarios";
import { type DriftAt, type Focus, type Lockstep, type StartPhase, driftAt, driftWords, firstFocus, focusMisses, focusWords, missLine, seconds, startPhase } from "./lockfold";
import type { RefineState } from "./refinefold";
import { ScreenPointer, TellPanel } from "./PointTell";
import { isPauseKey } from "./point";
import type { Box } from "./api";
import { NesPlayer } from "./nesplayer";
import { ModelPreview as Model } from "./modelpreview";
import { FreePlay } from "./FreePlay";
import {
  EXPLAINER,
  EXPLAINER_PLAYING,
  checkSentence,
  checkWhere,
  failingChecks,
  gapGroups,
  headline,
  momentSentence,
  plainQuestion,
  statusLine,
  words,
  type PlainQuestion,
} from "./phrase";
import { frameLines, framesDue, modelSteps, playStep, pressedWords, scriptFrom, type PlayState } from "./playfold";

const W = 256;
const H = 240;

/** A clip around a moment: a second before it to half a second after. */
export function clipAround(from: number, to: number, frames: number): { from: number; to: number } {
  return { from: Math.max(0, from - 60), to: Math.min(frames, to + 30) };
}

/** Where the view starts: the first failing check's recording and frame, else the first recording. */
export function startAt(words: GameWords | undefined, live: LiveState, play?: PlayState): { recording?: string; frame: number } {
  const m = play?.marks[0];
  if (m && words?.recordings.some((r) => r.name === "session")) return { recording: "session", frame: Math.max(1, m.from - 60) };
  const c = failingChecks(live).find((x) => checkWhere(x).recording);
  if (c) {
    const w = checkWhere(c);
    if (words?.recordings.some((r) => r.name === w.recording)) return { recording: w.recording, frame: Math.max(1, (w.frame ?? 0) - 60) };
  }
  return { recording: words?.recordings.find((r) => r.input)?.name, frame: 1 };
}

/** The original side: the run's ROM in gp_play.wasm, replaying one input log per slot. */
class Original {
  private logs = new Map<string, string>();
  private loaded: (string | undefined)[] = [];
  private constructor(
    private readonly api: Api,
    private readonly run: string,
    readonly p: NesPlayer,
    private readonly rom: Uint8Array,
  ) {}
  static async open(api: Api, run: string): Promise<Original> {
    const [p, rom] = await Promise.all([NesPlayer.load(api.playWasm()), api.captureRom(run)]);
    return new Original(api, run, p, rom);
  }
  async log(name: string): Promise<string> {
    let l = this.logs.get(name);
    if (l === undefined) {
      l = await this.api.captureInputLog(this.run, name);
      this.logs.set(name, l);
    }
    return l;
  }
  /** The picture at `frame` of `name` in `slot`, as RGBA. Forward is a step at a time; back starts over. */
  async at(slot: number, name: string, frame: number): Promise<Uint8ClampedArray<ArrayBuffer>> {
    const log = await this.log(name);
    if (this.loaded[slot] !== name || frame < this.p.frame(slot)) {
      this.p.replay(slot, this.rom, log);
      this.loaded[slot] = name;
    }
    if (frame === this.p.frame(slot) + 1) this.p.advance(slot);
    else if (frame > this.p.frame(slot)) this.p.runTo(slot, frame);
    return this.p.rgba(slot);
  }
}

/** A picture as a data URL, from RGBA. */
function pictureUrl(rgba: Uint8ClampedArray<ArrayBuffer>): string {
  const c = document.createElement("canvas");
  c.width = W;
  c.height = H;
  c.getContext("2d")?.putImageData(new ImageData(rgba, W, H), 0, 0);
  return c.toDataURL();
}

/** The pictures the page asks for, made one at a time on slot 1 and the model's snap. */
type StillKey = { recording: string; frame: number; side: "original" | "model" };

export interface GameViewProps {
  api: Api;
  run: string;
  words?: GameWords;
  live: LiveState;
  play?: PlayState;
  send: (i: Omit<CaptureInput, "v" | "actor">) => void;
  /** The page's own bar (run picker, buttons), drawn under the controls so the two games are first. */
  toolbar?: React.ReactNode;
}

export function GameView({ api, run, words: g, live, play, send, toolbar }: GameViewProps) {
  const start = useMemo(() => startAt(g, live, play), [g, live.stages.validate?.round, play?.marks.length]); // eslint-disable-line react-hooks/exhaustive-deps
  const [recording, setRecording] = useState<string>();
  const [frame, setFrameState] = useState(0);
  const [playing, setPlaying] = useState(false);
  // Free play (docs/show.md): the left monitor is the original, live.
  const [free, setFree] = useState(false);
  const [lines, setLines] = useState<string[]>([]);
  const [error, setError] = useState<string>();
  const [stills, setStills] = useState<Record<string, string>>({});
  const original = useRef<Original | undefined>(undefined);
  const opening = useRef<Promise<Original> | undefined>(undefined);
  const canvas = useRef<HTMLCanvasElement>(null);
  const iframe = useRef<HTMLIFrameElement>(null);
  const model = useRef(new Model(() => iframe.current));
  const clipEnd = useRef<number | undefined>(undefined);
  const stillQueue = useRef<Promise<unknown>>(Promise.resolve());
  const rec = recording ?? start.recording;
  const total = lines.length;
  // Pause and point: the box on the paused frame. What the person says
  // about it goes to their agent as a turn in the mechanic's talk.
  const [box, setBox] = useState<Box>();
  // Bumped when the refinement loop changes the model, so the lockstep is measured again.
  const [modelRound, setModelRound] = useState(0);
  const paused = !playing && total > 0;
  // Side by side, or the rebuilt game drawn over the original with a
  // transparency slider, or their difference (where they match is black).
  // Kept in the browser, a convenience.
  const [layout, setLayout] = useState<Layout>(() => readLayout().layout);
  const [opacity, setOpacity] = useState(() => readLayout().opacity);
  useEffect(() => saveLayout(layout, opacity), [layout, opacity]);
  // The stretch the two games are compared over (docs/inspector.md, "The
  // focus"): the refinement loop's window while it works, a stretch the
  // person picks, or a second before the first split. Both games loop it,
  // the slider covers it and the charts zoom to it.
  const [ls, setLs] = useState<Lockstep>();
  const [refine, setRefine] = useState<RefineState>();
  const [picked, setPicked] = useState<Focus | null>();
  // The whole run measured from the start, for where the view looks first.
  const [whole, setWhole] = useState<Lockstep>();
  // The original's state at the view's start, as the rebuilt game starts from it.
  const [st, setSt] = useState<StartState>();
  const [startError, setStartError] = useState<string>();
  // The game's own moments (docs/inspector.md, "Moments"), and the one picked.
  const [moments, setMoments] = useState<Moments>();
  const [momentsError, setMomentsError] = useState<string>();
  const [moment, setMoment] = useState<Moment>();
  // Where the two drift apart from the view's start, run free as the
  // picture is (docs/inspector.md, "Where they drift apart"): play stops
  // there, and with "fix on its own" the scoped loop starts there.
  const [freeLs, setFreeLs] = useState<Lockstep>();
  const [driftStopped, setDriftStopped] = useState<DriftAt>();
  // Outlines where the two pictures differ, from the free run's screen measure.
  const [showBoxes, setShowBoxes] = useState(true);
  const [autoFix, setAutoFix] = useState(false);
  // Bumped when a loop is started outside the Refining section, so it reads it.
  const [refineKick, setRefineKick] = useState(0);
  const loopWindow = refine?.current?.window ?? (refine?.state === "stuck" ? refine.result?.questions[0]?.window : undefined);
  const focus: Focus | undefined =
    refine?.running && loopWindow ? { from: loopWindow.original_from, to: loopWindow.original_to } : picked === null ? undefined : (picked ?? (loopWindow ? { from: loopWindow.original_from, to: loopWindow.original_to } : whole ? firstFocus(whole, FOCUS_LEN) : undefined));
  // Where both games start: the focus's first frame, or the run's start.
  const viewFrom = focus?.from ?? 1;
  const drift = freeLs ? driftAt(freeLs, viewFrom, focus?.to) : undefined;
  const driftRef = useRef(drift);
  driftRef.current = drift;
  const focusRef = useRef(focus);
  focusRef.current = focus;
  useEffect(() => setPicked(undefined), [recording]);
  // Into the stretch when it changes.
  useEffect(() => {
    if (focus && (frame < focus.from || frame > focus.to)) setFrameState(focus.from);
  }, [focus?.from, focus?.to]); // eslint-disable-line react-hooks/exhaustive-deps

  // Frame 0 is power-on, before anything is drawn, so the view starts at 1.
  const setFrame = useCallback((f: number) => setFrameState(Math.max(1, Math.min(f, total || f))), [total]);

  useEffect(() => {
    if (recording === undefined && start.recording) setFrameState(start.frame);
  }, [start.recording, start.frame, recording]);

  const opened = useCallback(() => {
    if (!g?.rom) return Promise.reject(new Error("the run has no ROM to play the original from"));
    opening.current ??= Original.open(api, run).then((o) => (original.current = o));
    return opening.current;
  }, [api, run, g?.rom]);

  // The model's messages.
  useEffect(() => {
    model.current.onError = setStartError;
    const on = (e: MessageEvent) => model.current.receive(e);
    window.addEventListener("message", on);
    return () => window.removeEventListener("message", on);
  }, []);

  // A recording chosen: its input for both sides.
  useEffect(() => {
    if (!rec || !g) return;
    let live = true;
    opened()
      .then((o) => o.log(rec))
      .then((log) => {
        if (!live) return;
        setLines(frameLines(log));
      })
      .catch((e: Error) => live && setError(e.message));
    return () => {
      live = false;
    };
  }, [rec, g, opened]); // eslint-disable-line react-hooks/exhaustive-deps

  // The whole run from the start, measured: where the view looks first.
  useEffect(() => {
    if (!rec || !g?.rom) return;
    let live = true;
    api
      .captureLockstep(run, rec, "resync", 1)
      .then((j) => live && setWhole(j))
      .catch(() => live && setWhole(undefined));
    return () => {
      live = false;
    };
  }, [api, run, rec, g?.rom, modelRound]);

  // The free run from the view's start, for where the two drift apart.
  useEffect(() => {
    if (!rec || !g?.rom) return;
    let live = true;
    setFreeLs(undefined);
    setDriftStopped(undefined);
    api
      .captureLockstep(run, rec, "free", viewFrom)
      .then((j) => live && setFreeLs(j))
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, [api, run, rec, g?.rom, viewFrom, modelRound]);
  const fixFromDrift = () => {
    if (moment && focus && moment.to >= focus.from && moment.from <= focus.to) return refineMoment(moment);
    if (!rec) return;
    api
      .captureRefineStart(run, rec, viewFrom)
      .then((r) => (r.started ? setRefineKick((n) => n + 1) : setError(r.why ?? "the loop didn't start")))
      .catch((e: Error) => setError(e.message));
  };
  useEffect(() => {
    if (driftStopped && autoFix && !refine?.running) fixFromDrift();
  }, [driftStopped]); // eslint-disable-line react-hooks/exhaustive-deps

  // The moments: found once a recording and the model are there.
  useEffect(() => {
    if (!rec || !g?.rom) return;
    let live = true;
    setMoments(undefined);
    setMomentsError(undefined);
    api
      .captureMoments(run, rec)
      .then((m) => live && setMoments(m))
      .catch((e: Error) => live && setMomentsError(e.message));
    return () => {
      live = false;
    };
  }, [api, run, rec, g?.rom, modelRound]);
  useEffect(() => setMoment(undefined), [rec]);
  const pickMoment = (m: Moment) => {
    setPlaying(false);
    setMoment(m);
    setPicked({ from: m.focus.from, to: Math.min(total || m.focus.to, m.focus.to) });
  };
  const refineMoment = (m: Moment) => {
    if (!rec) return;
    api
      .captureRefineStart(run, rec, undefined, { moment: { kind: m.kind, n: m.n } })
      .then((r) => (r.started ? setRefineKick((n) => n + 1) : setError(r.why ?? "the loop didn't start")))
      .catch((e: Error) => setError(e.message));
  };

  // The start: the original's state at the view's start, translated, and
  // the rebuilt game loaded with it and given the same presses from there.
  useEffect(() => {
    if (!rec || !g?.rom || !g.preview) return;
    let live = true;
    setSt(undefined);
    setStartError(undefined);
    api
      .captureStartState(run, rec, viewFrom)
      .then((s) => live && setSt(s))
      .catch((e: Error) => live && setStartError(e.message));
    return () => {
      live = false;
    };
  }, [api, run, rec, g?.rom, g?.preview, viewFrom, modelRound]);
  useEffect(() => {
    if (!st || !lines.length || free) return;
    model.current.loadState(st.state, scriptFrom(lines, st.original_frame), modelSteps(lines, st.original_frame, frame));
  }, [st, lines, free]); // eslint-disable-line react-hooks/exhaustive-deps

  // The frame: both sides there.
  useEffect(() => {
    if (!rec || !lines.length || free) return;
    let live = true;
    opened()
      .then((o) => o.at(0, rec, frame))
      .then((rgba) => {
        if (!live) return;
        canvas.current?.getContext("2d")?.putImageData(new ImageData(rgba, W, H), 0, 0);
      })
      .catch((e: Error) => live && setError(e.message));
    if (st) model.current.seek(modelSteps(lines, st.original_frame, frame));
    return () => {
      live = false;
    };
  }, [rec, lines, frame, opened, st, free]);

  // Playing: the shared frame moves at the NES's rate; the clock decides when, never what.
  useEffect(() => {
    if (!playing) return;
    let acc = 0;
    let last = performance.now();
    let raf = 0;
    const loop = (now: number) => {
      const due = framesDue(acc, now - last, 2);
      acc = due.acc;
      last = now;
      if (due.frames) {
        setFrameState((f) => {
          // In a focus play loops the stretch, and both stop where they drift apart.
          const s = playStep(f, due.frames, { total, focus: focusRef.current, clipEnd: clipEnd.current, stopAt: driftRef.current?.frame });
          if (s.stop) {
            clipEnd.current = undefined;
            setPlaying(false);
            if (s.frame === driftRef.current?.frame) setDriftStopped(driftRef.current);
          }
          return s.frame;
        });
      }
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(raf);
  }, [playing, total]);

  // Space or P pauses and resumes both games; a new frame drops the box.
  useEffect(() => {
    const kd = (e: KeyboardEvent) => {
      if (!total || !isPauseKey(e)) return;
      e.preventDefault();
      setPlaying((p) => !p);
    };
    window.addEventListener("keydown", kd);
    return () => window.removeEventListener("keydown", kd);
  }, [total]);
  useEffect(() => setBox(undefined), [frame, rec, playing]);

  const watch = (name: string | undefined, from: number, to: number) => {
    if (name && name !== rec) setRecording(name);
    const c = clipAround(from, to, total || to + 30);
    setFrameState(Math.max(1, c.from));
    clipEnd.current = c.to;
    setPlaying(true);
  };

  // Pictures for the questions and the moments: asked for while drawing,
  // made after, one at a time.
  const wanted = useRef(new Map<string, StillKey>());
  const asked = useRef(new Set<string>());
  const still = (k: StillKey) => {
    const key = `${k.side}:${k.recording}:${k.frame}`;
    if (!(key in stills) && !asked.current.has(key)) wanted.current.set(key, k);
    return stills[key] ?? "";
  };
  useEffect(() => {
    for (const [key, k] of wanted.current) {
      if (asked.current.has(key)) continue;
      if (k.side === "model" && (k.recording !== rec || !lines.length || !st)) continue;
      if (k.side === "original" && !g?.rom) continue;
      asked.current.add(key);
      stillQueue.current = stillQueue.current.then(async () => {
        try {
          const url = k.side === "original" ? pictureUrl(await (await opened()).at(1, k.recording, k.frame)) : await model.current.snap(modelSteps(lines, st!.original_frame, k.frame));
          setStills((s) => ({ ...s, [key]: url }));
        } catch {
          // No picture; the sentence stands alone.
        }
      });
    }
    wanted.current.clear();
  });

  const questions = openQuestions(live).map((q) => plainQuestion(g, q));
  const checks = failingChecks(live);
  const gaps = gapGroups(openGaps(live));
  const numbers = headline(live.metrics);
  const marks = play?.marks ?? [];
  const recName = (i?: number) => (i === undefined ? rec : g?.recordings[i]?.name);
  const levelFits = (() => {
    const l = g?.vocab.levels[0];
    return !!l?.size && !!l.tile_size && l.size[1] * l.tile_size <= H;
  })();
  const status = statusLine(g, live, play);

  return (
    <section className="game-view" data-testid="game-view">
      <div className={`games ${layout}`} data-layout={layout}>
        <figure>
          <ScreenPointer active={paused} box={box} onBox={setBox} testid="point-original">
            <canvas ref={canvas} width={W} height={H} className="game-screen" data-testid="game-original" />
            {showBoxes && freeLs?.screen && !free && <ScreenBoxes boxes={lockBoxes(freeLs.screen, lockIndex(freeLs.frames, frame))} w={W} h={H} />}
          </ScreenPointer>
          <figcaption>The original{g?.vocab.title ? `: ${g.vocab.title}` : ""}</figcaption>
        </figure>
        <figure className="model-side" style={layout === "top" ? { opacity } : undefined}>
          <ScreenPointer active={paused} box={box} onBox={setBox} testid="point-model">
            {g?.preview ? (
              <iframe ref={iframe} title="the game played from its model" src={api.capturePreview(run, "index.html?control=1")} className="game-screen" data-testid="game-model" />
            ) : (
              <div className="game-screen missing" data-testid="game-model">
                No playable model yet{g?.kind === "capture" ? " (start the run with --preview)" : ""}
              </div>
            )}
          </ScreenPointer>
          <figcaption>Played from the model</figcaption>
        </figure>
      </div>
      {free && <FreePlay api={api} run={run} recording={rec} frame={frame} canvas={canvas} model={g?.preview ? model.current : undefined} />}
      {total > 0 && !free && <SameCommands pressed={pressedWords(lines, frame, g?.vocab.actions)} phase={startPhase(g, st, frame, startError)} />}
      {total > 0 && g?.rom && !free && (
        <MomentsStrip g={g} moments={moments?.moments ?? []} lo={focus?.from ?? 1} hi={Math.min(focus?.to ?? total, total)} picked={moment} phase={momentsPhase(moments, !moments && !momentsError, momentsError)} pick={pickMoment} />
      )}
      <div className="game-controls">
        <label>
          show{" "}
          <select value={layout} onChange={(e) => setLayout(e.target.value as Layout)} data-testid="game-layout">
            <option value="side">side by side</option>
            <option value="top">the model on top</option>
            <option value="diff">the differences</option>
          </select>
        </label>
        {layout === "top" && (
          <label title="How much of the model shows over the original">
            original{" "}
            <input type="range" min={0} max={100} value={Math.round(opacity * 100)} onChange={(e) => setOpacity(Number(e.target.value) / 100)} data-testid="game-opacity" /> model
          </label>
        )}
        {freeLs?.screen && (
          <label title="Outline the places the two games' pictures differ on this frame: the characters in red, the background dashed">
            <input type="checkbox" checked={showBoxes} onChange={(e) => setShowBoxes(e.target.checked)} data-testid="game-boxes" /> show where the pictures differ
          </label>
        )}
        <button onClick={() => setPlaying(!playing)} data-testid="game-play" disabled={!total || free} title="Space or P">
          {playing ? "pause" : "play"}
        </button>
        <button onClick={() => (setPlaying(false), setFree(!free))} data-testid="game-free" disabled={!g?.rom} title="play the original yourself, with save points">
          {free ? "back to the recording" : "free play"}
        </button>
        <input
          type="range"
          min={focus?.from ?? 1}
          max={Math.min(focus?.to ?? total, total)}
          value={Math.min(frame, total)}
          onChange={(e) => (setPlaying(false), setFrame(Number(e.target.value)))}
          data-testid="game-scrubber"
        />
        {drift && (
          <span className="drift-mark" data-testid="drift-mark" title={driftWords(g, drift)}>
            drifts apart at {seconds(drift.frame)}
          </span>
        )}
        <span className="hint">
          {seconds(frame)}, frame {frame} {focus ? `(the stretch is frames ${focus.from} to ${focus.to})` : `of ${total}`}
        </span>
        {g && g.recordings.length > 1 && (
          <label>
            watching{" "}
            <select value={rec ?? ""} onChange={(e) => (setPlaying(false), setRecording(e.target.value), setFrameState(1))} data-testid="game-recording">
              {g.recordings
                .filter((r) => r.input)
                .map((r) => (
                  <option key={r.name} value={r.name}>
                    {words(r.name)}
                  </option>
                ))}
            </select>
          </label>
        )}
      </div>
      <FocusBar
        g={g}
        focus={focus}
        ls={ls}
        total={total}
        loop={refine?.running ? refineStatusWords(refine) : undefined}
        playing={playing}
        play={() => (focus && setFrameState(focus.from), setPlaying(true))}
        pick={(f) => (setPlaying(false), setPicked(f))}
        seek={(f) => (setPlaying(false), setFrame(f))}
        moments={focus && moments ? inFocus(moments.moments, focus) : []}
        pickMoment={pickMoment}
      >
        {drift && (
          <p className="drift-line" data-testid="drift-line">
            {driftWords(g, drift)}. <button onClick={() => (setPlaying(false), setFrame(drift.frame))}>go there</button>{" "}
            <button onClick={fixFromDrift} disabled={refine?.running} data-testid="fix-from-here">
              fix from here
            </button>{" "}
            <label title="When both games stop where they drift apart, start the refinement loop there by itself">
              <input type="checkbox" checked={autoFix} onChange={(e) => setAutoFix(e.target.checked)} /> fix on its own
            </label>
          </p>
        )}
        {moment && focus && moment.to >= focus.from && moment.from <= focus.to && (
          <RefineThisMoment
            moment={moment}
            phase={momentRefinePhase(refine?.moment ? refine : undefined, momentName(moment), moment.mechanic)}
            state={refine}
            start={refineMoment}
            stop={() => api.captureRefineStop(run).then(() => setRefineKick((n) => n + 1))}
            seek={(f) => (setPlaying(false), setFrame(f))}
          />
        )}
      </FocusBar>
      {toolbar}
      <p className="game-status" data-testid="game-status">
        {status}
      </p>
      {error && <p className="error">{error}</p>}
      {g?.rom && total > 0 && (
        <LockstepPanel
          key={modelRound}
          from={viewFrom}
          drift={drift?.frame}
          startHere={(f) => setPicked({ from: f, to: Math.min(total, f + FOCUS_LEN - 1) })}
          focus={focus}
          onResult={setLs}
          api={api}
          run={run}
          words={g}
          recording={rec}
          frame={frame}
          total={total}
          paused={paused}
          seek={(f) => {
            setPlaying(false);
            setFrame(f);
          }}
        />
      )}
      {g?.rom && total > 0 && <LockCurve ls={ls} frame={frame} />}
      {g?.rom && total > 0 && <RefinePanel api={api} run={run} words={g} recording={rec} frame={frame} paused={paused} seek={(f) => (setPlaying(false), setFrame(f))} watch={(f, t) => watch(rec, f, t)} changed={() => setModelRound((n) => n + 1)} send={send} onState={setRefine} kick={refineKick} />}
      {g?.rom && total > 0 && <ScenariosSection api={api} run={run} recording={rec} preview={!!g?.preview} onFocus={(from, to) => setPicked({ from, to })} seek={(f) => (setPlaying(false), setFrame(f))} paused={paused ? frame : undefined} />}
      {paused && <TellPanel api={api} run={run} recording={rec} frame={frame} box={box} />}

      {/* Capture's questions are about the whole run, not the stretch in
          focus, so they wait folded away; the questions about the stretch
          are the refinement loop's, above. */}
      {questions.length > 0 && (
        <details className="whole-run-questions">
          <summary>{questions.length === 1 ? "1 question" : `${questions.length} questions`} about the whole run, not the stretch above (mostly for agents)</summary>
          <section className="plain-questions" data-testid="plain-questions">
            {questions.map((q) => (
              <QuestionCard key={q.id} q={q} at={checks[0] ? { ...checkWhere(checks[0]) } : undefined} rec={rec} fits={levelFits} still={still} send={send} />
            ))}
          </section>
        </details>
      )}

      {(checks.length > 0 || gaps.length > 0 || marks.length > 0) && (
        <section className="misses" data-testid="misses">
          <h2>What the model misses</h2>
          <ul>
            {marks.map((m) => {
              const x = play?.explanations[m.id];
              const s = x ? momentSentence(g, x, m.note) : undefined;
              return (
                <li key={`m${m.id}`} data-testid="miss">
                  <p>{s ? s.sentence : `${m.note || `Moment ${m.id}`}: explaining...`}</p>
                  <BeforeAfter name={rec} from={m.from} to={m.to} still={still} />
                  <button onClick={() => watch(rec, m.from, m.to)}>watch it</button>
                </li>
              );
            })}
            {checks.length > 0 && (
              <li data-testid="miss">
                <p>
                  {checks.length === 1 ? "A check fails" : `${checks.length} checks fail`}: {checks.map((c) => checkSentence(g, c)).join("; ")}
                  {checkWhere(checks[0]).text ? `, ${checkWhere(checks[0]).text}` : ""}.
                </p>
                {checkWhere(checks[0]).frame !== undefined && (
                  <>
                    <BeforeAfter name={checkWhere(checks[0]).recording} from={checkWhere(checks[0]).frame!} to={checkWhere(checks[0]).frame!} still={still} model={checkWhere(checks[0]).recording === rec} />
                    <button onClick={() => watch(checkWhere(checks[0]).recording, checkWhere(checks[0]).frame!, checkWhere(checks[0]).frame!)}>watch it</button>
                  </>
                )}
              </li>
            )}
            {gaps.map((gp) => (
              <li key={gp.place} data-testid="miss">
                <p>
                  {gp.sentence}.{" "}
                  <button onClick={() => gp.items.forEach((i) => send({ kind: "pin", path: i.id }))} title="won't model: leave these out of the counts">
                    leave {gp.items.length === 1 ? "it" : "them"} out
                  </button>
                </p>
                {gp.frame !== undefined && recName(gp.recording) && (
                  <>
                    <BeforeAfter name={recName(gp.recording)} from={gp.frame} to={gp.frame} still={still} model={recName(gp.recording) === rec} />
                    <button onClick={() => watch(recName(gp.recording), gp.frame!, gp.frame!)}>watch it</button>
                  </>
                )}
              </li>
            ))}
          </ul>
        </section>
      )}

      {numbers.length > 0 && (
        <section className="numbers" data-testid="plain-numbers">
          {numbers.map((n) => (
            <div key={n.key} className={n.changed ? "number changed" : "number"} title={n.key}>
              <b>{Number.isInteger(n.value) ? n.value : n.value.toFixed(1)}</b> {n.label}
              <span className="hint">{n.means}</span>
            </div>
          ))}
        </section>
      )}
    </section>
  );
}

/** The buttons both games get on this frame and where the rebuilt game started, for the view and a guide that drives it. */
export function SameCommands({ pressed, phase }: { pressed: string; phase: StartPhase }) {
  return (
    <div className={`same-commands ${phase.state}`} data-testid="same-commands">
      <p>
        Both games: <b data-testid="pressed">{pressed}</b>
      </p>
      <p className="hint" data-testid="model-start">
        {phase.says} {phase.next}
      </p>
    </div>
  );
}

type Layout = "side" | "top" | "diff";
const LAYOUT_KEY = "gp.game.layout";

/** The two games' layout as last chosen in this browser; storage can be missing or refuse. */
function readLayout(): { layout: Layout; opacity: number } {
  try {
    const v = JSON.parse(localStorage.getItem(LAYOUT_KEY) ?? "null");
    if (v && ["side", "top", "diff"].includes(v.layout)) return { layout: v.layout, opacity: Math.min(1, Math.max(0, Number(v.opacity) || 0.5)) };
  } catch {
    // Not kept; the default stands.
  }
  return { layout: "side", opacity: 0.5 };
}

function saveLayout(layout: Layout, opacity: number): void {
  try {
    localStorage.setItem(LAYOUT_KEY, JSON.stringify({ layout, opacity }));
  } catch {
    // Not kept.
  }
}

/** The stretch the view compares, in the original's frames: 5 seconds. */
const FOCUS_LEN = 300;

function refineStatusWords(r: RefineState): string {
  const w = r.current?.window;
  return w ? `The refinement loop is working on this stretch (round ${r.current!.round}).` : "The refinement loop is running.";
}

/**
 * The stretch the two games are compared over, said first and large:
 * "Comparing 5 seconds: 3.6 to 8.6 seconds", what doesn't match there, and
 * buttons to move it or see the whole run.
 */
function FocusBar({
  g,
  focus,
  ls,
  total,
  loop,
  playing,
  play,
  pick,
  seek,
  moments = [],
  pickMoment,
  children,
}: {
  moments?: Moment[];
  pickMoment?: (m: Moment) => void;
  children?: React.ReactNode;
  g?: GameWords;
  focus?: Focus;
  ls?: Lockstep;
  total: number;
  loop?: string;
  playing: boolean;
  play: () => void;
  pick: (f: Focus | null) => void;
  seek: (f: number) => void;
}) {
  if (!total) return null;
  if (!focus) {
    return (
      <section className="focus-bar whole" data-testid="focus-bar">
        <h2>Showing the whole run ({seconds(total)})</h2>
        <button onClick={() => pick((ls && firstFocus(ls, FOCUS_LEN)) || { from: 1, to: Math.min(total, FOCUS_LEN) })}>compare 5 seconds at a time</button>
      </section>
    );
  }
  const len = focus.to - focus.from + 1;
  const misses = ls ? focusMisses(ls, focus) : [];
  const move = (by: number) => {
    const from = Math.max(1, Math.min(total - len + 1, focus.from + by));
    pick({ from, to: from + len - 1 });
  };
  return (
    <section className="focus-bar" data-testid="focus-bar">
      <h2>Comparing {focusWords(focus)}</h2>
      <p className="hint">
        Frames {focus.from} to {focus.to} of the original. Both games {playing ? "loop" : "play"} this stretch from the same moment with the same button presses. {loop}
      </p>
      <div className="focus-buttons">
        <button onClick={() => move(-len)} disabled={focus.from <= 1}>
          earlier
        </button>
        <button onClick={play}>play the stretch</button>
        <button onClick={() => move(len)} disabled={focus.to >= total}>
          later
        </button>
        <button className="link" onClick={() => pick(null)}>
          the whole run
        </button>
      </div>
      {pickMoment && <MomentsInFocus g={g} moments={moments} pick={pickMoment} />}
      {children}
      {ls && (
        <div className="focus-misses" data-testid="focus-misses">
          {misses.length ? (
            <>
              <h3>Not matching in these {len === FOCUS_LEN ? "5 seconds" : "frames"}</h3>
              <ul>
                {misses.map((m) => (
                  <li key={m.path}>
                    {missLine(g, m)}, from {seconds(m.first)} <button onClick={() => seek(m.first)}>go there</button>
                  </li>
                ))}
              </ul>
            </>
          ) : (
            <p>Everything measured matches in this stretch.</p>
          )}
        </div>
      )}
    </section>
  );
}

/** A still of the original and the model at a moment: before it and at it. */
function BeforeAfter({ name, from, to, still, model = true }: { name?: string; from: number; to: number; still: (k: StillKey) => string; model?: boolean }) {
  if (!name) return null;
  const before = Math.max(1, from - 20);
  const after = to + 10;
  const pic = (side: "original" | "model", frame: number, label: string) => {
    const url = still({ recording: name, frame, side });
    return (
      <figure key={`${side}${frame}`}>
        {url ? <img src={url} alt={`${label}, frame ${frame}`} width={W / 2} height={H / 2} /> : <div className="still-empty" style={{ width: W / 2, height: H / 2 }} />}
        <figcaption>{label}</figcaption>
      </figure>
    );
  };
  return (
    <div className="before-after" data-testid="before-after">
      {pic("original", before, "the original, before")}
      {pic("original", after, "the original, after")}
      {model && pic("model", before, "the model, before")}
      {model && pic("model", after, "the model, after")}
    </div>
  );
}

/** A question as pictures: the original at the moment, with each answer drawn on it where it can be. */
function QuestionCard({
  q,
  at,
  rec,
  fits,
  still,
  send,
}: {
  q: PlainQuestion;
  at?: { frame?: number; recording?: string };
  rec?: string;
  fits: boolean;
  still: (k: StillKey) => string;
  send: (i: Omit<CaptureInput, "v" | "actor">) => void;
}) {
  const name = at?.recording ?? rec;
  const frame = at?.frame ?? 0;
  const url = name && q.picture === "row" && fits ? still({ recording: name, frame, side: "original" }) : "";
  return (
    <div className="plain-question" data-testid="plain-question">
      <p className="ask">{q.ask}</p>
      {q.about && <p className="hint">{q.about}.</p>}
      <div className="options">
        {q.options.map((o) => (
          <button key={o.choice} className="option" onClick={() => send({ kind: "answer", question: q.id, choice: o.choice })} data-testid="plain-option">
            {q.picture === "row" && fits && o.number !== undefined && name ? (
              <svg viewBox={`0 0 ${W} ${H}`} width={W} height={H} className="option-picture">
                {url ? <image href={url} width={W} height={H} /> : <rect width={W} height={H} className="screen-bg" />}
                <line x1={0} x2={W} y1={o.number} y2={o.number} className="option-line" />
                <text x={4} y={Math.max(10, o.number - 3)} className="option-label">
                  {o.label}
                </text>
              </svg>
            ) : null}
            <span>
              {o.label} <span className="hint">({o.from})</span>
            </span>
          </button>
        ))}
      </div>
      <p className="hint">
        {at?.frame !== undefined && name ? `The picture is frame ${frame} of "${words(name)}", where the two games part. ` : ""}
        <button className="link" onClick={() => send({ kind: "pin", path: q.path ?? q.id })} title="won't model: stop asking and leave it out of the counts">
          the model can leave this out
        </button>
      </p>
    </div>
  );
}

/** The first-run explainer: what each area is, a line each. Dismissed once, it stays closed in this browser. */
export function Explainer({ open, onClose, playing = false }: { open: boolean; onClose: () => void; /** The live play panel is up and no run is shown beside it. */ playing?: boolean }) {
  if (!open) return null;
  const lines = playing ? EXPLAINER_PLAYING : EXPLAINER;
  return (
    <section className="explainer" data-testid="explainer">
      <h2>What am I looking at?</h2>
      <ul>
        {lines.map((e) => (
          <li key={e.area}>
            {e.area}: {e.line}
          </li>
        ))}
      </ul>
      <button onClick={onClose} data-testid="explainer-close">
        got it
      </button>
    </section>
  );
}

const EXPLAINED_KEY = "gp.explainer.dismissed";

/** Whether the explainer was dismissed in this browser; storage can be missing or refuse. */
export function explainerDismissed(): boolean {
  try {
    return typeof localStorage !== "undefined" && localStorage.getItem(EXPLAINED_KEY) === "1";
  } catch {
    return false;
  }
}

export function dismissExplainer(): void {
  try {
    localStorage.setItem(EXPLAINED_KEY, "1");
  } catch {
    // Not kept; it shows again next time.
  }
}
