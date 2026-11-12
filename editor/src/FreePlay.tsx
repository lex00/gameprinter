// Free play in the game view (docs/show.md, "Free play"): the left monitor
// becomes the original, live, played with the keyboard or a gamepad, and
// the right one the rebuilt game following from the latest save point: it
// loads the original's state there, translated, and gets the same presses
// live, frame by frame. F5 makes a save point and F8 goes back to the
// last one; the row of save points sits under the two monitors.
//
// The page keeps each save point's state bytes for fast loading. The
// server keeps the input from power-on that reaches each one, so every
// save point is rebuilt natively, and loading one starts a new branch of
// the session's input from its frame. Nothing here reads a clock to
// decide what a frame does; the clock only says when the next one runs.
//
// Any save point can become a clip: it loads, counts down 3-2-1, and the
// presses from there are recorded for up to the clip length (8 seconds
// unless the setting says otherwise; stop ends the take early). The take
// is saved as a played scenario whose start is the save point, scored
// with the run's scenarios, and can be watched on both games from the
// original's state at the save point, looping, or refined on.

import { useEffect, useRef, useState } from "react";
import type { Api } from "./api";
import { NesPlayer } from "./nesplayer";
import type { ModelPreview } from "./modelpreview";
import { KEYS, frameLines, framesDue, padFromGamepad, padFromKeys, triggerPresses } from "./playfold";
import type { Scenario } from "./scenariofold";
import { CLIP_SECONDS, FOLLOW_AFTER, FreePlayClient, UPLOAD_LINES, checkWords, clipCard, clipLength, clock, freeSays, latest, modelLine, takeSays, type FreeSave, type TakePhase } from "./freefold";

const W = 256;
const H = 240;

/** The keys free play takes for itself while it's on. */
export const SAVE_KEY = "F5";
export const LOAD_KEY = "F8";
const PAUSE = "KeyP";
const OWN = new Set([...KEYS.flat(), SAVE_KEY, LOAD_KEY, PAUSE]);

/** Tries at asking for the rebuilt game's start before saying it can't start there. */
const FOLLOW_TRIES = 8;

/** A save point's picture, half size, as a PNG data URL. */
function thumbnail(rgba: Uint8ClampedArray<ArrayBuffer>): string {
  const full = document.createElement("canvas");
  full.width = W;
  full.height = H;
  full.getContext("2d")?.putImageData(new ImageData(rgba, W, H), 0, 0);
  const half = document.createElement("canvas");
  half.width = W / 2;
  half.height = H / 2;
  half.getContext("2d")?.drawImage(full, 0, 0, W / 2, H / 2);
  return half.toDataURL("image/png");
}

/** A frame stepped live: its index from power-on, the pad, and whether the game read it. */
type Played = { f: number; pad: number; polled: boolean };

/** The rebuilt game following a save point: its frame and branch, and once taken, the original's frame it starts at. */
type Follow = { at: number; branch: number; askAt: number; tries: number; asking: boolean; start?: number; state?: string; fed: number; none?: boolean };

/** A clip being made from save point `save`: its phase, the frames recorded and the most allowed. */
type Take = { save: number; frame: number; phase: TakePhase; count: number; n: number; max: number; replace?: string; presses: Played[]; clip?: Scenario; start?: { at: number; state: string } };

const SETTING = "gp.freeplay.clipSeconds";

export interface FreePlayProps {
  api: Api;
  run: string;
  /** The recording the view shows, and its frame: free play starts there. */
  recording?: string;
  frame: number;
  /** The left monitor's canvas, the original's. */
  canvas: React.RefObject<HTMLCanvasElement | null>;
  /** The right monitor, the rebuilt game; absent when the run has none. */
  model?: ModelPreview;
}

export function FreePlay({ api, run, recording, frame: startFrame, canvas, model }: FreePlayProps) {
  const client = useRef(new FreePlayClient(api));
  const [session, setSession] = useState<string>();
  const [saves, setSaves] = useState<FreeSave[]>([]);
  const [pictures, setPictures] = useState<Record<number, string>>({});
  const [frame, setFrame] = useState(0);
  const [paused, setPaused] = useState(false);
  const [follow, setFollowState] = useState<"waiting" | "asking" | "following" | "none">();
  const [current, setCurrent] = useState<number>();
  const [error, setError] = useState<string>();
  const [ready, setReady] = useState(false);
  const [seconds, setSeconds] = useState(() => {
    try {
      return clipLength(Number(localStorage.getItem(SETTING) ?? CLIP_SECONDS));
    } catch {
      return CLIP_SECONDS;
    }
  });
  const [take, setTakeState] = useState<Take>();
  const [watching, setWatching] = useState(false);
  const [refineSays, setRefineSays] = useState<string>();

  const player = useRef<NesPlayer>(undefined);
  const rom = useRef<Uint8Array>(undefined);
  const sid = useRef<string>(undefined);
  const branch = useRef({ id: 1, uploaded: 0 });
  const uploading = useRef<Promise<unknown>>(Promise.resolve());
  const played = useRef<Played[]>([]);
  const fol = useRef<Follow | undefined>(undefined);
  const bytes = useRef(new Map<number, Uint8Array>());
  const down = useRef(new Set<string>());
  const busy = useRef(false);
  const takeRef = useRef<Take | undefined>(undefined);
  const setTake = (t: Take | undefined) => {
    takeRef.current = t;
    setTakeState(t ? { ...t } : undefined);
  };

  const draw = () => {
    const p = player.current;
    if (p) canvas.current?.getContext("2d")?.putImageData(new ImageData(p.rgba(0), W, H), 0, 0);
  };

  // Sends the branch's frames played since the last upload, in pieces.
  const upload = (all: boolean) => {
    uploading.current = uploading.current.then(async () => {
      const p = player.current;
      const id = sid.current;
      if (!p || !id) return;
      const b = branch.current;
      const lines = frameLines(p.inputLog(0));
      while (lines.length - b.uploaded >= (all ? 1 : UPLOAD_LINES)) {
        const piece = lines.slice(b.uploaded, b.uploaded + UPLOAD_LINES);
        const r = await client.current.log(id, b.id, b.uploaded, piece.join("\n"));
        b.uploaded = r.frames;
      }
    });
    return uploading.current;
  };

  // Starts: the original at the view's frame of its recording, live.
  useEffect(() => {
    let live = true;
    (async () => {
      const [p, r] = await Promise.all([NesPlayer.load(api.playWasm()), api.captureRom(run)]);
      if (!live) return;
      player.current = p;
      rom.current = r;
      const at = recording ? startFrame : 0;
      if (recording && at > 0) {
        p.replay(0, r, await api.captureInputLog(run, recording));
        p.runTo(0, at);
      } else p.power(0, r);
      const s = await client.current.start(run, recording && at > 0 ? recording : undefined, p.frame(0));
      if (!live) return;
      sid.current = s.session.id;
      branch.current = { id: s.session.branches[0].id, uploaded: s.session.branches[0].frames };
      played.current = [];
      setSession(s.session.id);
      setFrame(p.frame(0));
      draw();
      setReady(true);
    })().catch((e: Error) => live && setError(e.message));
    return () => {
      live = false;
    };
  }, [api, run]); // eslint-disable-line react-hooks/exhaustive-deps

  // The rebuilt game's start: asked for once the original is a few frames
  // past the save point, again a second later while it isn't taken yet.
  const ask = async (f: Follow) => {
    const id = sid.current;
    if (!id || !model) return;
    f.asking = true;
    setFollowState("asking");
    try {
      await upload(true);
      const s = await client.current.startState(id, f.branch, f.at);
      if (fol.current !== f) return;
      if (!s.ready) {
        f.tries += 1;
        f.askAt = (player.current?.frame(0) ?? 0) + 60;
        f.none = f.tries >= FOLLOW_TRIES;
        setFollowState(f.none ? "none" : "waiting");
        return;
      }
      const from = played.current.filter((x) => x.f >= s.original_frame && x.polled);
      f.start = s.original_frame;
      f.state = s.state;
      f.fed = played.current.length;
      model.loadState(s.state, `gp-frames 1\n${from.map((x) => modelLine(x.pad)).join("\n")}\n`, from.length);
      setFollowState("following");
    } catch (e) {
      setError((e as Error).message);
    } finally {
      f.asking = false;
    }
  };

  const followFrom = (at: number, b: number) => {
    fol.current = { at, branch: b, askAt: at + FOLLOW_AFTER, tries: 0, asking: false, fed: 0 };
    setFollowState("waiting");
  };

  // The game loop: a fixed timestep at the NES's rate.
  useEffect(() => {
    if (!ready || paused || watching) return;
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
        // The controller's triggers save and load, as F5 and F8 do.
        const tp = triggerPresses(navigator.getGamepads?.()[0], triggers.current);
        triggers.current = tp.held;
        if (tp.save) void keys.current.save();
        if (tp.load) void keys.current.load();
        if (!busy.current) {
          for (let i = 0; i < due.frames; i++) {
            const pad = padFromKeys(down.current) | padFromGamepad(navigator.getGamepads?.()[0]);
            const f = p.frame(0);
            const polled = p.step(0, pad);
            played.current.push({ f, pad, polled });
            // A take ends by itself at the clip length.
            const t = takeRef.current;
            if (t?.phase === "recording") {
              t.n = played.current.length;
              if (t.n >= t.max) {
                void endTake();
                break;
              }
            }
          }
          if (due.frames) {
            draw();
            const f = p.frame(0);
            setFrame(f);
            if (takeRef.current?.phase === "recording") setTake(takeRef.current);
            const fo = fol.current;
            if (fo && fo.start !== undefined && model) {
              const more = played.current.slice(fo.fed).filter((x) => x.polled && x.f >= fo.start!);
              fo.fed = played.current.length;
              if (more.length) model.push(`gp-frames 1\n${more.map((x) => modelLine(x.pad)).join("\n")}\n`);
            } else if (fo && !fo.asking && !fo.none && f >= fo.askAt) void ask(fo);
            if (f - branch.current.uploaded >= UPLOAD_LINES) void upload(false).catch((e: Error) => setError(e.message));
          }
        }
      } catch (e) {
        setError((e as Error).message);
        return;
      }
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(raf);
  }, [ready, paused, watching]); // eslint-disable-line react-hooks/exhaustive-deps

  const saveNow = async () => {
    const p = player.current;
    const id = sid.current;
    if (!p || !id || busy.current || takeRef.current) return;
    try {
      const n = p.frame(0);
      const state = p.save(0);
      const hash = p.hash(0);
      const digest = p.digest(0);
      const picture = thumbnail(p.rgba(0));
      const b = branch.current.id;
      await upload(true);
      const r = await client.current.save(id, { branch: b, frame: n, hash, digest, picture });
      bytes.current.set(r.save.id, state);
      setPictures((x) => ({ ...x, [r.save.id]: picture }));
      setSaves((x) => [...x, r.save]);
      setCurrent(r.save.id);
      if (branch.current.id === b) followFrom(n, b);
    } catch (e) {
      setError((e as Error).message);
    }
  };

  const load = async (k: number | undefined, forTake = false) => {
    const p = player.current;
    const id = sid.current;
    const sp = saves.find((s) => s.id === k);
    if (!p || !id || !sp || busy.current || !rom.current) return;
    if (!forTake && takeRef.current && takeRef.current.phase !== "saved") return;
    busy.current = true;
    try {
      await upload(true);
      const b = bytes.current.get(sp.id);
      // The bytes in memory, or the save point's input from power-on.
      if (b) p.restore(0, b);
      else {
        p.replay(0, rom.current, await client.current.saveLog(id, sp.id));
        p.runTo(0, sp.frame);
      }
      const nb = await client.current.branch(id, sp.id);
      branch.current = { id: nb.branch.id, uploaded: nb.branch.frames };
      played.current = [];
      setCurrent(sp.id);
      followFrom(sp.frame, nb.branch.id);
      draw();
      setFrame(p.frame(0));
    } catch (e) {
      setError((e as Error).message);
    } finally {
      busy.current = false;
    }
  };

  // A clip from a save point: load it, count down, record the presses.
  const startTake = async (k: number, replace?: string) => {
    const sp = saves.find((s) => s.id === k);
    if (!sp || busy.current) return;
    setWatching(false);
    setRefineSays(undefined);
    setPaused(true);
    await load(k, true);
    const t: Take = { save: k, frame: sp.frame, phase: "count", count: 3, n: 0, max: Math.round(seconds * 60), replace, presses: [] };
    setTake(t);
    for (let c = 3; c > 0; c--) {
      t.count = c;
      setTake(t);
      await new Promise((ok) => setTimeout(ok, 700));
    }
    t.phase = "recording";
    played.current = [];
    setTake(t);
    canvas.current?.scrollIntoView({ block: "nearest" });
    setPaused(false);
  };

  // The take stops (by itself at the length, or with stop): saved as a clip, then scored.
  const endTake = async () => {
    const t = takeRef.current;
    const p = player.current;
    const id = sid.current;
    if (!t || t.phase !== "recording" || !p || !id) return;
    t.phase = "saving";
    t.presses = played.current.slice(0, t.max);
    t.n = t.presses.length;
    const fo = fol.current;
    if (fo?.state && fo.start !== undefined) t.start = { at: fo.start, state: fo.state };
    setTake(t);
    setPaused(true);
    try {
      if (t.n === 0) throw new Error("nothing was recorded");
      const text = p.inputLog(0);
      const head = text.split("\n").slice(0, 2).join("\n");
      const lines = frameLines(text).slice(t.frame, t.frame + t.n);
      const r = await client.current.clip(run, `${id}/${t.save}`, `${head}\n${lines.join("\n")}\n`, t.replace);
      t.clip = r.scenario;
      t.phase = "saved";
      setTake(t);
      // Scored with the run's scenarios, where it joins the strip.
      if (recording) void api.post(`/api/capture/${encodeURIComponent(run)}/scenarios`, { recording }).catch(() => undefined);
    } catch (e) {
      setError((e as Error).message);
      setTake(undefined);
    }
  };

  // The take on both games from the original's state at the save point, looping.
  useEffect(() => {
    const t = takeRef.current;
    const p = player.current;
    if (!watching || !t || !p || !rom.current) return;
    const b = bytes.current.get(t.save);
    const id = sid.current;
    let live = true;
    let raf = 0;
    const begin = async () => {
      // Slot 1, so the session's branch in slot 0 stays as played.
      if (b) {
        p.power(1, rom.current!);
        p.restore(1, b);
      } else if (id) {
        p.replay(1, rom.current!, await client.current.saveLog(id, t.save));
        p.runTo(1, t.frame);
      }
      const from = t.start ? t.presses.filter((x) => x.polled && x.f >= t.start!.at) : [];
      if (model && t.start) model.loadState(t.start.state, `gp-frames 1\n${from.map((x) => modelLine(x.pad)).join("\n")}\n`, 0);
    };
    let i = 0;
    let steps = 0;
    let acc = 0;
    let last = performance.now();
    const loop = (now: number) => {
      if (!live) return;
      const due = framesDue(acc, now - last);
      acc = due.acc;
      last = now;
      for (let k = 0; k < due.frames && i < t.presses.length; k++, i++) {
        const x = t.presses[i];
        p.step(1, x.pad);
        if (x.polled && t.start && x.f >= t.start.at) steps += 1;
      }
      if (due.frames) {
        canvas.current?.getContext("2d")?.putImageData(new ImageData(p.rgba(1), W, H), 0, 0);
        if (model && t.start) model.seek(steps);
      }
      if (i >= t.presses.length) {
        // A second's rest on the last frame, then again from the save point.
        setTimeout(() => {
          if (!live) return;
          i = 0;
          steps = 0;
          void begin().then(() => live && (raf = requestAnimationFrame(loop)));
        }, 1000);
        return;
      }
      raf = requestAnimationFrame(loop);
    };
    void begin()
      .then(() => live && (raf = requestAnimationFrame(loop)))
      .catch((e: Error) => setError(e.message));
    return () => {
      live = false;
      cancelAnimationFrame(raf);
    };
  }, [watching]); // eslint-disable-line react-hooks/exhaustive-deps

  const stopWatching = () => {
    setWatching(false);
    const t = takeRef.current;
    if (t) void load(t.save, true);
  };

  // The refinement loop on the clip, as the Scenarios section starts it; its sentence polled here.
  const refine = async () => {
    const t = takeRef.current;
    if (!t?.clip || !recording) return;
    const base = `/api/capture/${encodeURIComponent(run)}/scenarios`;
    try {
      // The clip's scores may still be running; the refine starts after them.
      let r = await api.post<{ started: boolean; why?: string }>(`${base}/refine`, { recording, scenario: t.clip.id });
      for (let i = 0; !r.started && i < 120; i++) {
        setRefineSays("Waiting for the clip's scores, then refining on it.");
        await new Promise((ok) => setTimeout(ok, 1000));
        r = await api.post(`${base}/refine`, { recording, scenario: t.clip.id });
      }
      if (!r.started) {
        setRefineSays(r.why ?? "It didn't start.");
        return;
      }
      for (;;) {
        await new Promise((ok) => setTimeout(ok, 1000));
        const st = await api.json<{ state: string; says?: string }>(`${base}?recording=${encodeURIComponent(recording)}`);
        setRefineSays(st.says);
        if (st.state !== "running") break;
      }
    } catch (e) {
      setError((e as Error).message);
    }
  };

  const doneWithTake = () => {
    setWatching(false);
    setRefineSays(undefined);
    setTake(undefined);
    setPaused(false);
  };

  // The keys are the game's and free play's while it's on: they don't
  // scroll the page or pause the recording's replay (Space is A).
  const triggers = useRef({ save: false, load: false });
  const keys = useRef({ save: saveNow, load: () => load(current ?? latest(saves)) });
  keys.current = { save: saveNow, load: () => load(current ?? latest(saves)) };
  useEffect(() => {
    const kd = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      if (["INPUT", "TEXTAREA", "SELECT"].includes(t?.tagName ?? "") || !OWN.has(e.code)) return;
      e.preventDefault();
      e.stopImmediatePropagation();
      if (e.repeat && (e.code === SAVE_KEY || e.code === LOAD_KEY || e.code === PAUSE)) return;
      if (e.code === SAVE_KEY) void keys.current.save();
      else if (e.code === LOAD_KEY) void keys.current.load();
      else if (e.code === PAUSE) {
        if (!takeRef.current) setPaused((x) => !x);
      }
      else down.current.add(e.code);
    };
    const ku = (e: KeyboardEvent) => {
      down.current.delete(e.code);
      if (OWN.has(e.code)) e.preventDefault();
    };
    const blur = () => down.current.clear();
    window.addEventListener("keydown", kd, { capture: true });
    window.addEventListener("keyup", ku, { capture: true });
    window.addEventListener("blur", blur);
    return () => {
      window.removeEventListener("keydown", kd, { capture: true });
      window.removeEventListener("keyup", ku, { capture: true });
      window.removeEventListener("blur", blur);
    };
  }, []);

  // The server's check of each save point, polled until every one is done.
  useEffect(() => {
    if (!session || saves.every((s) => s.native || s.error)) return;
    const t = setTimeout(() => {
      client.current
        .session(session)
        .then((r) => setSaves(r.session.saves))
        .catch(() => undefined);
    }, 1500);
    return () => clearTimeout(t);
  }, [session, saves]);

  return (
    <section className="free-play" data-testid="free-play">
      <p className="free-says" data-testid="free-says">
        {!ready
          ? "Starting the original here."
          : take
            ? take.phase === "saved" && take.clip
              ? clipCard(take.n, take.save, take.frame)
              : takeSays(take.phase, take.count, take.n, take.max)
            : freeSays({ saves: saves.length, follow, paused, model: !!model })}
      </p>
      {take && take.phase === "recording" && (
        <div className="free-actions">
          <button onClick={() => void endTake()} data-testid="clip-stop">
            stop
          </button>
        </div>
      )}
      {take?.phase === "saved" && take.clip && (
        <div className="free-actions" data-testid="clip-saved">
          <button onClick={() => (watching ? stopWatching() : setWatching(true))} data-testid="clip-watch">
            {watching ? "stop watching" : "watch it"}
          </button>
          <button onClick={() => void refine()} disabled={!recording} data-testid="clip-refine" title={recording ? undefined : "the view has no recording for the scenarios"}>
            refine it
          </button>
          <button onClick={() => void startTake(take.save, take.clip!.id)} data-testid="clip-again">
            record again
          </button>
          <button onClick={doneWithTake}>back to free play</button>
          {refineSays && <span className="hint">{refineSays}</span>}
        </div>
      )}
      <div className="free-actions">
        <button onClick={() => void saveNow()} disabled={!ready || !!take} data-testid="free-save" title="F5, or the left trigger">
          make a save point (F5)
        </button>
        <button onClick={() => void load(current ?? latest(saves))} disabled={!saves.length} data-testid="free-load" title="F8, or the right trigger">
          back to the save point (F8)
        </button>
        <button onClick={() => setPaused(!paused)} disabled={!ready || !!take} title="P">
          {paused ? "go on playing" : "pause"}
        </button>
        <span className="hint">
          {clock(frame)}. Arrows move, X jumps, Z runs, Enter is Start; a gamepad works too, its left trigger saving and its right trigger going back.
        </span>
        <label className="hint">
          clips run up to{" "}
          <input
            type="number"
            min={1}
            max={30}
            value={seconds}
            onChange={(e) => {
              const v = clipLength(Number(e.target.value));
              setSeconds(v);
              try {
                localStorage.setItem(SETTING, String(v));
              } catch {
                // The setting is kept for this page only.
              }
            }}
            data-testid="clip-seconds"
            style={{ width: "3.5em" }}
          />{" "}
          seconds
        </label>
      </div>
      {saves.length > 0 && (
        <ol className="save-points" data-testid="save-points">
          {saves.map((s) => (
            <li key={s.id} className={s.id === current ? "current" : undefined}>
              <button onClick={() => void load(s.id)} title="load this save point and play on from it">
                <img src={pictures[s.id] ?? (s.picture && session ? client.current.picture(session, s.id) : "")} width={W / 2} height={H / 2} alt={`save point ${s.id}`} />
                <span>
                  {s.id}. {clock(s.frame)}
                </span>
                <span className={s.native && !s.native.matches ? "error" : "hint"}>{checkWords(s)}</span>
              </button>
              <button onClick={() => void startTake(s.id)} disabled={!!take && take.phase !== "saved"} data-testid="clip-make">
                make a clip from here
              </button>
            </li>
          ))}
        </ol>
      )}
      {error && <p className="error">{error}</p>}
    </section>
  );
}
