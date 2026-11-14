// The play panel (docs/show.md): play the original live, drop marks on
// the moments the model gets wrong ("hit a brick"), stop, and read what
// the original did there that the model didn't. The game runs in
// gp_play.wasm on a ROM gp serve reads from $GP_ROM_DIR; the page keeps it
// in memory only. The page paces frames at the NES's rate and the console
// never reads a clock, so the input log replays to the same frames
// natively, which gp serve checks when it records the session.
//
// Keys: arrows (or WASD) for the d-pad, X jump (A), Z run (B), Enter
// Start, right Shift Select; a gamepad in the standard mapping works too.
// M drops a mark on the last second and pauses for its note. P pauses to
// point at the screen and tell the agent what's wrong there
// (docs/point-and-tell.md).

import { useEffect, useRef, useState } from "react";
import type { Api, Box, PlayRom } from "./api";
import { ScreenPointer, TellPanel } from "./PointTell";
import { NesPlayer } from "./nesplayer";
import {
  MARK_KEY,
  UPLOAD_FRAMES,
  emptyPlay,
  foldPlay,
  frameAt,
  frameLines,
  framesDue,
  KEYS,
  markBox,
  modelFrame,
  padFromGamepad,
  padFromKeys,
  replayWindow,
  type PlayState,
} from "./playfold";

/** Frames a mark dropped while playing reaches back: the last second and a half. */
export const MARK_BACK = 90;

const GAME_KEYS = new Set(KEYS.flat());

/** The key that pauses live play to point and tell. Space is the A button here. */
export const PAUSE_KEY = "KeyP";

function draw(canvas: HTMLCanvasElement | null, rgba: Uint8ClampedArray<ArrayBuffer>) {
  const ctx = canvas?.getContext("2d");
  if (!ctx) return;
  ctx.putImageData(new ImageData(rgba, 256, 240), 0, 0);
}

/** The marks on the session's timeline, the explanations and the log. Drawn from the folded events alone. */
export function PlayReport({
  state,
  frames,
  pending,
  onTimeline,
  onReplay,
}: {
  state: PlayState;
  frames: number;
  /** A timeline mark's first frame, chosen and waiting for its last. */
  pending?: number;
  onTimeline?: (frame: number) => void;
  onReplay?: (mark: number) => void;
}) {
  return (
    <div className="play-report" data-testid="play-report">
      <p className="status" data-testid="play-status">
        {state.run ?? "no session"}: {frames} frames
        {state.recording ? `, recorded as ${state.recording.slice(0, 12)}${state.realStore ? " in the real store" : " in the scratch store"}` : state.stopped ? ", recording" : ""}
        {state.digestMatches === true ? ", every frame hash agrees" : state.digestMatches === false ? ", FRAME HASHES DIFFER" : ""}
        {state.explaining ? ", explaining" : ""}
      </p>
      <div
        className="timeline play-timeline"
        data-testid="play-timeline"
        title={onTimeline ? "click a mark's first frame, then its last" : undefined}
        onClick={(e) => {
          if (!onTimeline) return;
          const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
          onTimeline(frameAt(e.clientX - r.left, r.width, frames));
        }}
      >
        {pending !== undefined && <div className="timeline-now" style={{ left: `${markBox({ from: pending, to: pending }, frames).left}%` }} />}
        {state.marks.map((m) => {
          const b = markBox(m, frames);
          return <span key={m.id} className="timeline-mark" style={{ left: `${b.left}%`, width: `${b.width}%`, top: 2 }} title={`mark ${m.id}: ${m.from}..=${m.to} ${m.note}`} />;
        })}
      </div>
      <ol className="play-marks" data-testid="play-marks">
        {state.marks.map((m) => {
          const x = state.explanations[m.id];
          return (
            <li key={m.id} data-mark={m.id}>
              <b>
                mark {m.id}, frames {m.from} to {m.to}
              </b>
              {m.note ? ` "${m.note}"` : ""} ({m.by === "key" ? "while playing" : "on the timeline"}
              {m.annotation !== undefined ? `, annotation ${m.annotation}` : ""})
              {x ? (
                <div className="explanation">
                  <p>What the original did that the model didn't:</p>
                  {x.missing.length ? (
                    <ul>
                      {x.missing.map((l, i) => (
                        <li key={i}>{l}</li>
                      ))}
                    </ul>
                  ) : (
                    <p>{x.summary}</p>
                  )}
                  <p className="model-did">
                    The model, frames {x.model.frames ? `${x.model.frames[0]} to ${x.model.frames[1]}` : "none"}:{" "}
                    {x.model.events?.length ? x.model.events.map(([n, c]) => `${n} x${c}`).join(", ") : "no events"}
                  </p>
                </div>
              ) : state.recording ? (
                <p className="hint">explaining</p>
              ) : null}
              {onReplay && state.recording && <button onClick={() => onReplay(m.id)}>replay beside the model</button>}
            </li>
          );
        })}
      </ol>
      {state.preview && state.preview.status !== "ok" && (
        <p className="hint">
          model preview: {state.preview.status}
          {state.preview.why ? ` (${state.preview.why})` : ""}
        </p>
      )}
      {state.errors.map((e, i) => (
        <p key={i} className="error">
          {e}
        </p>
      ))}
    </div>
  );
}

export function PlayPanel({ api, onRecorded }: { api: Api; /** Called once the session is recorded, to show it beside the model. */ onRecorded?: (run: string) => void }) {
  const [roms, setRoms] = useState<PlayRom[]>([]);
  const [wasmBuilt, setWasmBuilt] = useState(true);
  const [stores, setStores] = useState<{ scratch: string; real: string }>();
  const [rom, setRom] = useState("");
  const [notes, setNotes] = useState("");
  const [realStore, setRealStore] = useState(false);
  const [run, setRun] = useState<string>();
  const [state, setState] = useState<PlayState>(emptyPlay);
  const [frames, setFrames] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [notePrompt, setNotePrompt] = useState<{ from: number; to: number; by: "key" | "timeline" }>();
  const [note, setNote] = useState("");
  const [pending, setPending] = useState<number>();
  const [replay, setReplay] = useState<{ mark: number; model?: string }>();
  const [error, setError] = useState<string>();
  const [paused, setPaused] = useState(false);
  const [box, setBox] = useState<Box>();

  const player = useRef<NesPlayer>(undefined);
  const romBytes = useRef<Uint8Array>(undefined);
  const uploaded = useRef(0);
  const uploading = useRef<Promise<unknown>>(Promise.resolve());
  const down = useRef(new Set<string>());
  const reset = useRef(false);
  const canvas = useRef<HTMLCanvasElement>(null);
  const replayCanvas = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    api
      .playRoms()
      .then((r) => {
        setRoms(r.roms);
        setWasmBuilt(r.wasm);
        setStores({ scratch: r.scratch_store, real: r.real_store });
        if (r.roms[0]) setRom((x) => x || r.roms[0].sha256);
      })
      .catch((e: Error) => setError(e.message));
  }, [api]);

  // The session's events, polled as the live panel polls them.
  useEffect(() => {
    setState(emptyPlay());
    if (!run) return;
    let live = true;
    let seq = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const poll = async () => {
      try {
        const { events } = await api.captureEvents(run, seq);
        if (!live) return;
        if (events.length) {
          seq = events[events.length - 1].seq;
          setState((s) => foldPlay(s, events));
        }
      } catch (e) {
        if (live) setError((e as Error).message);
      }
      if (live) timer = setTimeout(poll, 500);
    };
    void poll();
    return () => {
      live = false;
      if (timer) clearTimeout(timer);
    };
  }, [api, run]);

  // Recorded: the game view takes the session over (docs/inspector.md).
  useEffect(() => {
    if (run && state.recording) onRecorded?.(run);
  }, [run, state.recording]); // eslint-disable-line react-hooks/exhaustive-deps

  // Sends the frames played since the last upload, in pieces.
  const upload = (all: boolean) => {
    const p = player.current;
    if (!p || !run) return uploading.current;
    uploading.current = uploading.current.then(async () => {
      const lines = frameLines(p.inputLog(0));
      while (lines.length - uploaded.current >= (all ? 1 : UPLOAD_FRAMES)) {
        const piece = lines.slice(uploaded.current, uploaded.current + UPLOAD_FRAMES);
        const r = await api.playLog(run, uploaded.current, piece.join("\n"));
        uploaded.current = r.frames;
      }
    });
    return uploading.current;
  };

  // Paused: the frames so far go up, so the server can show what's under
  // a box at this frame; resuming drops the box.
  useEffect(() => {
    setBox(undefined);
    if (paused) void upload(true).catch((e: Error) => setError(e.message));
  }, [paused]); // eslint-disable-line react-hooks/exhaustive-deps

  // The game loop: a fixed timestep at the NES's rate while playing.
  useEffect(() => {
    if (!playing || notePrompt || paused) return;
    const p = player.current;
    if (!p) return;
    let acc = 0;
    let last = performance.now();
    let raf = 0;
    const loop = (now: number) => {
      const due = framesDue(acc, now - last);
      acc = due.acc;
      last = now;
      try {
        for (let i = 0; i < due.frames; i++) {
          const pad = padFromKeys(down.current) | padFromGamepad(navigator.getGamepads?.()[0]);
          p.step(0, pad, reset.current);
          reset.current = false;
        }
        if (due.frames) {
          draw(canvas.current, p.rgba(0));
          const f = p.frame(0);
          setFrames(f);
          if (f - uploaded.current >= UPLOAD_FRAMES) void upload(false).catch((e: Error) => setError(e.message));
        }
      } catch (e) {
        setError((e as Error).message);
        setPlaying(false);
        return;
      }
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(raf);
  }, [playing, notePrompt, paused]);

  // Keys go to the game while it plays; M drops a mark and pauses for its note.
  useEffect(() => {
    if (!playing) return;
    const kd = (e: KeyboardEvent) => {
      if (notePrompt) return;
      const typing = ["INPUT", "TEXTAREA"].includes((e.target as HTMLElement | null)?.tagName ?? "");
      if (e.code === PAUSE_KEY && !typing) {
        setPaused((p) => !p);
        e.preventDefault();
        return;
      }
      if (paused) return;
      if (e.code === MARK_KEY) {
        const f = player.current?.frame(0) ?? 0;
        setNotePrompt({ from: Math.max(0, f - MARK_BACK), to: f, by: "key" });
        setNote("");
        e.preventDefault();
        return;
      }
      if (e.code === "F5") reset.current = true;
      if (GAME_KEYS.has(e.code) || e.code === "F5") {
        down.current.add(e.code);
        e.preventDefault();
      }
    };
    const ku = (e: KeyboardEvent) => down.current.delete(e.code);
    window.addEventListener("keydown", kd);
    window.addEventListener("keyup", ku);
    return () => {
      window.removeEventListener("keydown", kd);
      window.removeEventListener("keyup", ku);
    };
  }, [playing, notePrompt, paused]);

  const start = async () => {
    setError(undefined);
    try {
      const r = roms.find((x) => x.sha256 === rom);
      if (!r) throw new Error("choose a ROM");
      const [p, bytes] = await Promise.all([player.current ? Promise.resolve(player.current) : NesPlayer.load(api.playWasm()), api.playRom(r.sha256)]);
      player.current = p;
      romBytes.current = bytes;
      p.power(0, bytes);
      const s = await api.playStart({ rom: r.sha256, notes, real_store: realStore });
      uploaded.current = 0;
      setFrames(0);
      setReplay(undefined);
      setRun(s.session.run);
      setPlaying(true);
      canvas.current?.focus();
    } catch (e) {
      setError((e as Error).message);
    }
  };

  const stop = async () => {
    const p = player.current;
    if (!p || !run) return;
    setPlaying(false);
    setPaused(false);
    try {
      await upload(true);
      await api.playStop(run, p.frame(0), p.digest(0));
    } catch (e) {
      setError((e as Error).message);
    }
  };

  const sendMark = async () => {
    if (!notePrompt || !run) return;
    const m = notePrompt;
    setNotePrompt(undefined);
    try {
      await api.playMark(run, { from: m.from, to: m.to, note: note.trim(), by: m.by });
    } catch (e) {
      setError((e as Error).message);
    }
    canvas.current?.focus();
  };

  const timeline = (frame: number) => {
    if (playing) return;
    if (pending === undefined) {
      setPending(frame);
      return;
    }
    setPending(undefined);
    setNotePrompt({ from: Math.min(pending, frame), to: Math.max(pending, frame), by: "timeline" });
    setNote("");
  };

  // A mark's moment again: the original in slot 1 from the session's
  // input log, and the model's preview on the same polled frames.
  const startReplay = async (mark: number) => {
    const p = player.current;
    const m = state.marks.find((x) => x.id === mark);
    if (!p || !run || !m || !romBytes.current) return;
    try {
      const log = await api.playInputLog(run);
      const lines = frameLines(log);
      const w = replayWindow(m, lines.length);
      p.replay(1, romBytes.current, log);
      p.runTo(1, w.from);
      draw(replayCanvas.current, p.rgba(1));
      const model =
        state.preview?.status === "ok" ? api.capturePreview(run, `index.html?run=session&from=${modelFrame(lines, w.from)}&to=${modelFrame(lines, w.to)}`) : undefined;
      setReplay({ mark, model });
      let acc = 0;
      let last = performance.now();
      const loop = (now: number) => {
        const due = framesDue(acc, now - last);
        acc = due.acc;
        last = now;
        for (let i = 0; i < due.frames && p.frame(1) < w.to; i++) p.advance(1);
        draw(replayCanvas.current, p.rgba(1));
        if (p.frame(1) < w.to) requestAnimationFrame(loop);
      };
      requestAnimationFrame(loop);
    } catch (e) {
      setError((e as Error).message);
    }
  };

  return (
    <aside className="play-panel" data-testid="play-panel">
      <h2>Play the original</h2>
      {!wasmBuilt && <p className="error">The player isn't built: run `just play-web` (scripts/web/build-play.sh).</p>}
      {!playing && !state.stopped && (
        <form
          className="play-start"
          onSubmit={(e) => {
            e.preventDefault();
            void start();
          }}
        >
          <select data-testid="play-rom" value={rom} onChange={(e) => setRom(e.target.value)}>
            {roms.length === 0 && <option value="">no ROM in $GP_ROM_DIR or --rom</option>}
            {roms.map((r) => (
              <option key={r.sha256} value={r.sha256}>
                {r.name}
                {r.template ? ` (model ${r.template})` : " (no model)"}
              </option>
            ))}
          </select>
          <input value={notes} onChange={(e) => setNotes(e.target.value)} placeholder="notes for the recording" />
          <label title={stores ? `scratch: ${stores.scratch}; real: ${stores.real}` : undefined}>
            <input type="checkbox" data-testid="play-real-store" checked={realStore} onChange={(e) => setRealStore(e.target.checked)} /> record into the real store
            (default: the scratch store)
          </label>
          <button type="submit" disabled={!rom}>
            play
          </button>
        </form>
      )}
      <ScreenPointer active={playing && paused} box={box} onBox={setBox} testid="point-play">
        <canvas ref={canvas} width={256} height={240} tabIndex={0} className="play-screen" data-testid="play-screen" />
      </ScreenPointer>
      {playing && (
        <p className="hint">
          Arrows move, X jumps, Z runs, Enter is Start. M marks the last second. P pauses to point at what's wrong.{" "}
          <button onClick={() => setPaused(!paused)} data-testid="play-pause">
            {paused ? "go on playing" : "pause"}
          </button>{" "}
          <button onClick={() => void stop()} data-testid="play-stop">
            stop and record
          </button>
        </p>
      )}
      {playing && paused && run && (
        <TellPanel
          api={api}
          run={run}
          recording="session"
          frame={frames}
          box={box}
        />
      )}
      {notePrompt && (
        <form
          className="play-note"
          onSubmit={(e) => {
            e.preventDefault();
            void sendMark();
          }}
        >
          mark frames {notePrompt.from} to {notePrompt.to}:{" "}
          <input autoFocus value={note} onChange={(e) => setNote(e.target.value)} placeholder="what's missing (hit a brick)" />
          <button type="submit">mark</button>
          <button type="button" onClick={() => setNotePrompt(undefined)}>
            cancel
          </button>
        </form>
      )}
      {error && <p className="error">{error}</p>}
      {run && <PlayReport state={state} frames={state.frames ?? frames} pending={pending} onTimeline={playing ? undefined : timeline} onReplay={(m) => void startReplay(m)} />}
      {replay && (
        <section className="play-replay" data-testid="play-replay">
          <h3>Mark {replay.mark} again</h3>
          <div className="side-by-side">
            <figure>
              <canvas ref={replayCanvas} width={256} height={240} className="play-screen" />
              <figcaption>the original</figcaption>
            </figure>
            {replay.model ? (
              <figure>
                <iframe title="the model at the mark" src={replay.model} width={512} height={480} />
                <figcaption>the model, regenerated</figcaption>
              </figure>
            ) : (
              <p className="hint">no model preview for this session</p>
            )}
          </div>
        </section>
      )}
    </aside>
  );
}
