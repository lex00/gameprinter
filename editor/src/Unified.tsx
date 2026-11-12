// The main screen (docs/show.md, "The main screen"), a control room for
// the background work: the game screen with the rebuilt game on top, one
// slim row of controls (the work on or paused, how many clips it works on
// at once, the rebuilt game, free play, generate clips), the background
// line, the live cards (a card for each job running now, its bars moving
// toward the goal, ControlRoom.tsx), the dashboard of every clip in
// progress, and the progress summary. Everything else is in the details
// drawer: the save state filmstrip, the background line's questions and
// rounds with their curves. A clip opened from a card is the clip view:
// both games loop it, with its score, its questions, keep, skip and star,
// and "fix this clip".
//
// Free play is one button. Off, the original stands still and the screen
// follows the clip the background work is on. On, the play controls open
// and the original plays live with the keyboard or a gamepad. Play is the
// natural loop of save states: F5 saves, F8 loads the save state chosen in
// the strip (the newest unless another is chosen). With clips on, every
// save or load starts a clip there that ends at the next save or load or
// at the cap (clipfold.ts), so playing makes clips with no start and stop.
// Each clip is saved with its save point, rebuilt natively from its input
// log and hash-checked by the server, and its presses.
//
// "generate clips" asks the server to make clips by itself from the
// run's recordings (genfold.ts, docs/show.md, "Clips made by the tool"):
// they join the strip after the save points, grouped by level, and the
// background work scores them beside the ones you played. A skipped clip
// leaves the background pool.
//
// The page keeps the state bytes of the save points made since it opened;
// the rest come from the server's native rebuild (`saves/<k>/state`), so
// hundreds of save points work across page loads. Nothing here reads a
// clock to decide what a frame does; the clock only says when the next
// one runs.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Api, Box, GameWords } from "./api";
import { ScreenPointer } from "./PointTell";
import { CLICK } from "./point";
import { NesPlayer } from "./nesplayer";
import { ModelPreview } from "./modelpreview";
import { PlayRebuild } from "./PlayRebuild";
import { ScreenBoxes } from "./ScreenBoxes";
import { boxesAt, screenLines } from "./screenfold";
import { KEYS, frameLines, framesDue, padFromGamepad, padFromKeys, triggerPresses } from "./playfold";
import { FOLLOW_AFTER, FreePlayClient, UPLOAD_LINES, clock, modelLine, secs, type FreeSave, type FreeSession } from "./freefold";
import { type ClipSettings, type CutState, type EndedClip, capSeconds, cutSays, emptyCut, frameOfLine, keepClip, onFrame, onManualStart, onMark, onStop, readSettings, saveSettings } from "./clipfold";
import { type Filter, type GenState, type Mark, type SourceFilter, type StripClip, type StripItem, activeItems, clipState, filterItems, madeItems, saveKey, stripItems, tally, tallyWords } from "./stripfold";
import { GenClient, generating } from "./genfold";
import { type Card, Sparkline, useTrends } from "./Dashboard";
import { Filmstrip } from "./Filmstrip";
import { BackgroundLine, useBackground } from "./Background";
import { type BgQuestion, type BgRound, type BgState, openQuestions, stripScores } from "./backfold";
import { LiveCards, WorkersControl } from "./ControlRoom";
import { AgentsControl } from "./AgentsControl";
import { FlightControl, GridHeat, useGrid } from "./GridHeat";
import { ClipCurve } from "./CurveChart";
import { framesScript } from "./playfold";
import { pathPhrase } from "./phrase";
import { AgentProgress, useWork } from "./Agent";
import { FidelityLine, useFidelity } from "./Fidelity";
import { ClipDetail, FidelityGauge, type Thumb, Thumbs } from "./Calm";
import { type Ctx, agentAsks, roundRows, clipBadges, clipQuestions, clipStatus, jobOn, matchPermille, needsYouClips, thumbOrder, whereItParts } from "./calmfold";
import { type WorkState, agentLine, clipAttempts, clipTells, fixReady, headerCounts, pinWords, tellBody, workingOn } from "./agentfold";
import type { ScreenScore } from "./screenfold";

const W = 256;
const H = 240;

export const SAVE_KEY = "F5";
export const LOAD_KEY = "F8";
const PAUSE = "KeyP";
const OWN = new Set([...KEYS.flat(), SAVE_KEY, LOAD_KEY, PAUSE]);

/** Tries at asking for the rebuilt game's start before saying it can't start there. */
const FOLLOW_TRIES = 8;

/** State bytes kept in the page, newest last; older ones come from the server. */
const KEPT_BYTES = 24;

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

/** The rebuilt game following from the last save or load. */
type Follow = { at: number; branch: number; askAt: number; tries: number; asking: boolean; start?: number; fed: number; none?: boolean };

export type Layout = "top" | "diff" | "side";

const VIEW_KEY = "gp.unified.view";

/** How the two games show: the rebuilt game on or off, how, and whether the boxes where the pictures differ are drawn. */
type View = { show: boolean; layout: Layout; opacity: number; boxes: boolean };

function readView(): View {
  try {
    const v = JSON.parse(localStorage.getItem(VIEW_KEY) ?? "null");
    if (v && ["top", "diff", "side"].includes(v.layout))
      return { show: v.show === true, layout: v.layout, opacity: Math.min(1, Math.max(0, Number(v.opacity) || 0.5)), boxes: v.boxes !== false };
  } catch {
    // Not kept; the defaults stand.
  }
  return { show: false, layout: "top", opacity: 0.5, boxes: true };
}

function saveView(v: View) {
  try {
    localStorage.setItem(VIEW_KEY, JSON.stringify(v));
  } catch {
    // Kept for this page only.
  }
}

export interface UnifiedProps {
  api: Api;
  run: string;
  words?: GameWords;
}

/** A clip open on the screen, and the question it was opened for. */
/** `auto` is a clip the screen follows by itself: it plays, but its drawer stays closed until asked for. */
/** Where the person clicked on the paused clip: the box, whether it was a click, the clip's frame and the original's. */
type Pin = { box: Box; click: boolean; frame: number; orig?: number };
type Watch = { clip: StripClip; item: StripItem & { kind: "save" | "made" }; question?: BgQuestion; auto?: boolean };

/** The most thumbnails drawn; the rest are counted. */
const MAX_THUMBS = 36;

export function Unified({ api, run, words: g }: UnifiedProps) {
  const client = useRef(new FreePlayClient(api));
  const [sessions, setSessions] = useState<FreeSession[]>([]);
  const [frame, setFrame] = useState(0);
  const [paused, setPaused] = useState(false);
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string>();
  const [note, setNote] = useState<string>();
  const [follow, setFollowState] = useState<"waiting" | "asking" | "following" | "none">();
  const [view, setView] = useState(readView);
  useEffect(() => saveView(view), [view]);
  // Free play is one button (docs/show.md, "The main screen"): off, the
  // original stands still and the screen shows what the background work
  // does; on, the play controls open and the keys play the game.
  const [freePlay, setFreePlay] = useState(() => readFlag(FREEPLAY_KEY, false));
  useEffect(() => saveFlag(FREEPLAY_KEY, freePlay), [freePlay]);
  const freePlayRef = useRef(freePlay);
  freePlayRef.current = freePlay;
  // The rebuilt game's physics (docs/show.md, "Physics: retro or normal"):
  // the profiles the run's preview has, and the one shown, kept in the
  // browser. It changes only the rebuilt game on this screen; scores and
  // the background work always run retro.
  const [profiles, setProfiles] = useState<string[]>([]);
  const [physics, setPhysicsState] = useState(() => readText(PHYSICS_KEY, "retro"));
  const setPhysics = (p: string) => {
    saveText(PHYSICS_KEY, p);
    setPhysicsState(p);
  };
  // "Play the rebuild" (PlayRebuild.tsx): the screen is the playable rebuilt game; the clip loop waits.
  const [playRebuild, setPlayRebuild] = useState(false);
  const [drawer, setDrawer] = useState(() => readFlag(DRAWER_KEY, false));
  useEffect(() => saveFlag(DRAWER_KEY, drawer), [drawer]);
  const [settings, setSettingsState] = useState<ClipSettings>(readSettings);
  const setSettings = (s: ClipSettings) => {
    saveSettings(s);
    setSettingsState(s);
    settingsRef.current = s;
  };
  const settingsRef = useRef(settings);
  const [cut, setCutState] = useState<CutState>(emptyCut);
  const cutRef = useRef<CutState>(cut);
  const setCut = (c: CutState) => {
    cutRef.current = c;
    setCutState(c);
  };
  const [filter, setFilter] = useState<Filter>("all");
  const [source, setSource] = useState<SourceFilter>("all");
  const gclient = useRef(new GenClient(api));
  const [gen, setGen] = useState<GenState>();
  const [current, setCurrent] = useState<string>();
  const [playing, setPlaying] = useState<string>();
  const [pictures, setPictures] = useState<Record<string, string>>({});
  const bg = useBackground(api, run);
  const scores = useMemo(() => stripScores(bg.state), [bg.state]);
  const [watch, setWatch] = useState<Watch>();
  // Claude's work on the run's clips, and the attempt whose trial preview is on the screen.
  const work = useWork(api, run);
  const grid = useGrid(api, run);
  const [trial, setTrial] = useState<number>();
  const all = useMemo(() => [...stripItems(sessions, scores), ...madeItems(gen, scores)], [sessions, scores, gen]);
  const cards = useMemo(() => all.filter((i): i is Card => i.kind !== "group" && !!i.clip), [all]);
  const { trends, changed } = useTrends(cards);
  const fid = useFidelity(api, run);

  const player = useRef<NesPlayer>(undefined);
  const rom = useRef<Uint8Array>(undefined);
  const sidRef = useRef<string>(undefined);
  const branch = useRef({ id: 1, uploaded: 0 });
  const uploading = useRef<Promise<unknown>>(Promise.resolve());
  const played = useRef<Played[]>([]);
  const fol = useRef<Follow | undefined>(undefined);
  const last = useRef<{ at: number; branch: number } | undefined>(undefined);
  const bytes = useRef(new Map<string, Uint8Array>());
  const down = useRef(new Set<string>());
  const busy = useRef(false);
  // A save or load's number -> its save point ("SESSION/K"), once the server has it.
  const marks = useRef(new Map<number, Promise<string>>());
  const canvas = useRef<HTMLCanvasElement>(null);
  const iframe = useRef<HTMLIFrameElement>(null);
  const model = useRef(new ModelPreview(() => iframe.current));
  const showModel = (view.show || !!watch) && (!!g?.preview || (!!watch && trial !== undefined));
  // A trial is watched only on its clip.
  useEffect(() => setTrial(undefined), [watch?.clip.id]);
  // The trial preview is another page in the iframe: it says ready again.
  useEffect(() => model.current.reset(), [trial]);
  const modelSrc = watch && trial !== undefined ? work.client.previewUrl(run, trial) : api.capturePreview(run, "index.html?control=1");

  const keep = (key: string, b: Uint8Array) => {
    const m = bytes.current;
    m.delete(key);
    m.set(key, b);
    while (m.size > KEPT_BYTES) m.delete(m.keys().next().value as string);
  };

  const draw = () => {
    const p = player.current;
    if (p) canvas.current?.getContext("2d")?.putImageData(new ImageData(p.rgba(0), W, H), 0, 0);
  };

  const refresh = useCallback(async () => {
    const r = await client.current.sessions(run);
    setSessions(r.sessions);
    return r.sessions;
  }, [run]); // eslint-disable-line react-hooks/exhaustive-deps

  // The clips the tool made, polled: often while it makes them.
  const making = generating(gen);
  useEffect(() => {
    let live = true;
    const get = () =>
      gclient.current
        .state(run)
        .then((x) => live && setGen(x))
        .catch(() => undefined);
    void get();
    const t = setInterval(get, making ? 2000 : 10000);
    return () => {
      live = false;
      clearInterval(t);
    };
  }, [run, making]);
  const generate = async () => {
    try {
      setGen(await gclient.current.start(run));
    } catch (e) {
      setError((e as Error).message);
    }
  };

  // The physics profiles the run's preview was built with (preview.json).
  useEffect(() => {
    if (!g?.preview) return;
    let live = true;
    fetch(api.capturePreview(run, "preview.json"))
      .then((r) => (r.ok ? r.json() : null))
      .then((c: { profiles?: string[] } | null) => live && setProfiles(c?.profiles ?? []))
      .catch(() => live && setProfiles([]));
    return () => {
      live = false;
    };
  }, [run, g?.preview]); // eslint-disable-line react-hooks/exhaustive-deps
  const shownPhysics = profiles.includes(physics) ? physics : "retro";
  useEffect(() => model.current.profile(shownPhysics), [shownPhysics]);

  // The model's messages.
  useEffect(() => {
    const on = (e: MessageEvent) => model.current.receive(e);
    window.addEventListener("message", on);
    return () => window.removeEventListener("message", on);
  }, []);

  // Sends the branch's frames played since the last upload, in pieces.
  const upload = (all: boolean) => {
    uploading.current = uploading.current.then(async () => {
      const p = player.current;
      const id = sidRef.current;
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

  // A save point's bytes: in the page, from the server's rebuild, or replayed here.
  const restoreSave = async (session: string, sp: FreeSave) => {
    const p = player.current!;
    const key = saveKey(session, sp.id);
    let b = bytes.current.get(key);
    if (!b) {
      try {
        b = await client.current.state(session, sp.id);
        keep(key, b);
      } catch {
        b = undefined;
      }
    }
    if (b) p.restore(0, b);
    else {
      p.replay(0, rom.current!, await client.current.saveLog(session, sp.id));
      p.runTo(0, sp.frame);
    }
  };

  const followFrom = (at: number, b: number) => {
    last.current = { at, branch: b };
    fol.current = { at, branch: b, askAt: at + FOLLOW_AFTER, tries: 0, asking: false, fed: 0 };
    setFollowState("waiting");
  };

  // Opens: the newest session of the run, at its newest save point; or a new one from power-on.
  useEffect(() => {
    let live = true;
    (async () => {
      const [p, r] = await Promise.all([NesPlayer.load(api.playWasm()), api.captureRom(run)]);
      if (!live) return;
      player.current = p;
      rom.current = r;
      const list = await refresh();
      const newest = list[0];
      const sp = newest?.saves.reduce<FreeSave | undefined>((a, b) => (!a || b.id > a.id ? b : a), undefined);
      p.power(0, r);
      if (newest && sp) {
        sidRef.current = newest.id;
        await restoreSave(newest.id, sp);
        const nb = await client.current.branch(newest.id, sp.id);
        branch.current = { id: nb.branch.id, uploaded: nb.branch.frames };
        setCurrent(saveKey(newest.id, sp.id));
        setPlaying(saveKey(newest.id, sp.id));
        followFrom(sp.frame, nb.branch.id);
        await refresh();
      } else if (newest && newest.recording === null && newest.frame === 0) {
        // A session with no save point yet: a new branch of it from power-on.
        sidRef.current = newest.id;
        const nb = await client.current.branch(newest.id);
        branch.current = { id: nb.branch.id, uploaded: nb.branch.frames };
      } else {
        const s = await client.current.start(run, undefined, 0);
        sidRef.current = s.session.id;
        branch.current = { id: s.session.branches[0].id, uploaded: s.session.branches[0].frames };
        await refresh();
      }
      if (!live) return;
      played.current = [];
      setFrame(p.frame(0));
      draw();
      setReady(true);
    })().catch((e: Error) => live && setError(e.message));
    return () => {
      live = false;
    };
  }, [api, run]); // eslint-disable-line react-hooks/exhaustive-deps

  // The rebuilt game's start: asked for once the original is a few frames past the save point.
  const ask = async (f: Follow) => {
    const id = sidRef.current;
    if (!id) return;
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
      f.fed = played.current.length;
      model.current.loadState(s.state, `gp-frames 1\n${from.map((x) => modelLine(x.pad)).join("\n")}\n`, from.length);
      setFollowState("following");
    } catch (e) {
      setError((e as Error).message);
    } finally {
      f.asking = false;
    }
  };

  // A clip ended: kept by the rules, it's saved with its save point and presses.
  const finish = (e: EndedClip | undefined) => {
    const p = player.current;
    if (!e || !p) return;
    const text = p.inputLog(0);
    const lines = frameLines(text).slice(e.from, e.to);
    const k = keepClip(lines, settingsRef.current, e.manual);
    if (!k.keep) {
      setNote(`Left out a clip: ${k.why}.`);
      return;
    }
    const head = text.split("\n").slice(0, 2).join("\n");
    const ref = marks.current.get(e.mark);
    if (!ref) return;
    void ref
      .then((save) => client.current.clip(run, save, `${head}\n${lines.join("\n")}\n`, undefined, { branch: e.branch, from: e.from, cut: e.manual ? "manual" : "auto", ended: e.why }))
      .then(() => {
        setNote(`Clip saved: ${(lines.length / 60.1).toFixed(1)} s.`);
        return refresh();
      })
      .catch((x: Error) => setError(x.message));
  };

  // The game loop: a fixed timestep at the NES's rate.
  useEffect(() => {
    if (!ready || paused || watch || !freePlay) return;
    const p = player.current;
    if (!p) return;
    let acc = 0;
    let lastT = performance.now();
    let raf = 0;
    const loop = (now: number) => {
      const due = framesDue(acc, now - lastT);
      acc = due.acc;
      lastT = now;
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
            const c = onFrame(cutRef.current, p.frame(0), settingsRef.current);
            if (c.ended) {
              setCut(c.state);
              finish(c.ended);
            }
          }
          if (due.frames) {
            draw();
            const f = p.frame(0);
            setFrame(f);
            const fo = fol.current;
            if (showModelRef.current && fo) {
              if (fo.start !== undefined) {
                const more = played.current.slice(fo.fed).filter((x) => x.polled && x.f >= fo.start!);
                fo.fed = played.current.length;
                if (more.length) model.current.push(`gp-frames 1\n${more.map((x) => modelLine(x.pad)).join("\n")}\n`);
              } else if (!fo.asking && !fo.none && f >= fo.askAt) void ask(fo);
            }
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
  }, [ready, paused, watch, freePlay]); // eslint-disable-line react-hooks/exhaustive-deps
  const showModelRef = useRef(showModel);
  showModelRef.current = showModel;

  // The rebuilt game shown, or back from a clip: it follows from the last save or load.
  useEffect(() => {
    if (showModel && !watch && last.current) followFrom(last.current.at, last.current.branch);
  }, [showModel, !!watch]); // eslint-disable-line react-hooks/exhaustive-deps

  const saveNow = async (manual = false) => {
    const p = player.current;
    const id = sidRef.current;
    if (!p || !id || busy.current) return;
    const n = p.frame(0);
    const b = branch.current.id;
    const c = manual ? onManualStart(cutRef.current, n, b) : onMark(cutRef.current, "save", n, n, b, settingsRef.current);
    finish(c.ended);
    setCut(c.state);
    let done: (k: string) => void = () => {};
    let fail: (e: Error) => void = () => {};
    marks.current.set(
      c.mark,
      new Promise<string>((ok, no) => {
        done = ok;
        fail = no;
      }),
    );
    try {
      const state = p.save(0);
      const hash = p.hash(0);
      const digest = p.digest(0);
      const picture = thumbnail(p.rgba(0));
      await upload(true);
      const r = await client.current.save(id, { branch: b, frame: n, hash, digest, picture });
      const key = saveKey(id, r.save.id);
      keep(key, state);
      setPictures((x) => ({ ...x, [key]: picture }));
      setCurrent(key);
      setPlaying(key);
      done(key);
      if (branch.current.id === b) followFrom(n, b);
      await refresh();
    } catch (e) {
      fail(e as Error);
      setError((e as Error).message);
    }
  };

  const findSave = (key: string): { session: string; sp: FreeSave } | undefined => {
    for (const s of sessions) for (const sp of s.saves) if (saveKey(s.id, sp.id) === key) return { session: s.id, sp };
    return undefined;
  };

  const load = async (key: string | undefined) => {
    const p = player.current;
    const at = key ? findSave(key) : undefined;
    if (!p || !at || busy.current || !rom.current) return;
    busy.current = true;
    try {
      const end = p.frame(0);
      // The clip being cut ends where play was, before the load moves it.
      const pre = onStop(cutRef.current, end);
      finish(pre.ended ? { ...pre.ended, why: "load" } : undefined);
      await upload(true);
      await restoreSave(at.session, at.sp);
      sidRef.current = at.session;
      const nb = await client.current.branch(at.session, at.sp.id);
      branch.current = { id: nb.branch.id, uploaded: nb.branch.frames };
      played.current = [];
      const c = onMark(pre.state, "load", end, at.sp.frame, nb.branch.id, settingsRef.current);
      marks.current.set(c.mark, Promise.resolve(key!));
      setCut(c.state);
      setCurrent(key);
      setPlaying(key);
      followFrom(at.sp.frame, nb.branch.id);
      draw();
      setFrame(p.frame(0));
    } catch (e) {
      setError((e as Error).message);
    } finally {
      busy.current = false;
    }
  };

  // A clip open: both games loop it from the original's state at its start.
  const [watchError, setWatchError] = useState<string>();
  const [watchAt, setWatchAt] = useState(0);
  // Pause, point and tell: the clip paused on the screen, and where the person clicked.
  const [clipPaused, setClipPaused] = useState(false);
  const pausedRef = useRef(false);
  pausedRef.current = clipPaused;
  const [pin, setPin] = useState<Pin>();
  useEffect(() => {
    setClipPaused(false);
    setPin(undefined);
  }, [watch?.clip.id]);
  useEffect(() => {
    const w = watch;
    const p = player.current;
    if (!w || !p || !rom.current) return;
    let live = true;
    let raf = 0;
    setWatchError(undefined);
    const rec = bg.state?.recording ?? g?.recordings.find((r) => r.input)?.name;
    const it = w.item;
    // A played clip starts from its save point; one the tool made from its start.
    const baseFrame = it.kind === "save" ? it.save.frame : it.start.frame;
    (async () => {
      if (!rec) throw new Error("the run has no recording for the clip's bench");
      const inp = await api.json<{ log: string; start_frame: number; state: string }>(
        `/api/capture/${encodeURIComponent(run)}/scenarios/input?recording=${encodeURIComponent(rec)}&scenario=${encodeURIComponent(w.clip.id)}`,
      );
      // The frames from the save point on: to the start, then the clip's presses.
      const lines = frameLines(inp.log).slice(baseFrame);
      const frames = lines.map(frameOfLine);
      const lead = Math.max(0, inp.start_frame - baseFrame);
      const begin = async () => {
        const key = it.key;
        let b = bytes.current.get(key);
        if (!b) {
          b = await (it.kind === "save" ? client.current.state(it.session, it.save.id) : gclient.current.startState(run, it.start.id)).catch(() => undefined);
          if (b) keep(key, b);
        }
        if (b) {
          p.power(1, rom.current!);
          p.restore(1, b);
        } else if (it.kind === "save") {
          p.replay(1, rom.current!, await client.current.saveLog(it.session, it.save.id));
          p.runTo(1, it.save.frame);
        } else {
          throw new Error(`the original's state at ${it.start.name} couldn't be had from the server`);
        }
        if (showModelRef.current) model.current.loadState(inp.state, framesScript(lines.slice(lead)), 0);
      };
      let i = 0;
      let steps = 0;
      let acc = 0;
      let lastT = performance.now();
      const loop = (now: number) => {
        if (!live) return;
        if (pausedRef.current) {
          acc = 0;
          lastT = now;
          raf = requestAnimationFrame(loop);
          return;
        }
        const due = framesDue(acc, now - lastT);
        acc = due.acc;
        lastT = now;
        for (let k = 0; k < due.frames && i < frames.length; k++, i++) {
          const f = frames[i];
          p.step(1, f.pad, f.reset);
          if (f.polled && i >= lead) steps += 1;
        }
        if (due.frames) {
          canvas.current?.getContext("2d")?.putImageData(new ImageData(p.rgba(1), W, H), 0, 0);
          if (showModelRef.current) model.current.seek(steps);
          setWatchAt(Math.max(0, i - lead));
        }
        if (i >= frames.length) {
          // A second's rest on the last frame, then again from the start.
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
      await begin();
      if (live) raf = requestAnimationFrame(loop);
    })().catch((e: Error) => live && setWatchError(e.message));
    return () => {
      live = false;
      cancelAnimationFrame(raf);
      draw();
    };
  }, [watch?.clip.id, trial]); // eslint-disable-line react-hooks/exhaustive-deps

  const cardOf = (id: string) => all.find((i) => i.kind !== "group" && i.clip?.id === id) as (StripItem & { kind: "save" | "made" }) | undefined;
  const openClip = (c: StripClip, question?: BgQuestion) => {
    const it = cardOf(c.id);
    if (it) setWatch({ clip: it.clip!, item: it, question });
  };
  // Follow the background work (the user, 2026-10-30: "show me the
  // progress somehow and a live preview too"): while you aren't playing,
  // the screen shows the clip it scores or fixes now, looping, the rebuilt
  // game on top, and moves on with it. Kept in this browser.
  const [followWork, setFollowWork] = useState(() => readFollow());
  useEffect(() => saveFollow(followWork), [followWork]);
  const jobClip = bg.state?.job?.clip || undefined;
  useEffect(() => {
    // Out of free play you're watching, not playing, so it follows.
    // A clip you opened stays until you close it.
    if (playRebuild || !followWork || !jobClip || (freePlay && !paused && !watch) || watch?.clip.id === jobClip || (watch && !watch.auto)) return;
    const it = cardOf(jobClip);
    if (it?.clip) setWatch({ clip: it.clip, item: it, auto: true });
  }, [followWork, jobClip, paused, freePlay, playRebuild]); // eslint-disable-line react-hooks/exhaustive-deps
  const openQuestion = (q: BgQuestion) => {
    const it = cardOf(q.clip);
    if (it?.clip) setWatch({ clip: it.clip, item: it, question: q });
  };
  const markClip = async (m: Mark) => {
    if (!watch) return;
    try {
      if (watch.clip.source === "made") setGen(await gclient.current.mark(run, watch.clip.id, m));
      else await client.current.mark(watch.clip.session, watch.clip.id, m);
      setWatch({ ...watch, clip: { ...watch.clip, mark: m } });
      await refresh();
    } catch (e) {
      setError((e as Error).message);
    }
  };

  const stopClip = () => {
    const p = player.current;
    if (!p) return;
    const c = onStop(cutRef.current, p.frame(0));
    setCut(c.state);
    finish(c.ended);
  };

  // The game's keys and F5, F8 and P, while the page has them: they don't scroll it.
  const triggers = useRef({ save: false, load: false });
  const keys = useRef({ save: () => saveNow(), load: () => load(current) });
  keys.current = { save: () => saveNow(), load: () => load(current) };
  useEffect(() => {
    const kd = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      if (!freePlayRef.current || ["INPUT", "TEXTAREA", "SELECT"].includes(t?.tagName ?? "") || !OWN.has(e.code)) return;
      // The strip steps with the arrow keys while it has focus.
      if (t?.closest?.(".strip-scroll") && !["F5", "F8"].includes(e.code)) return;
      if (t?.closest?.(".details")) return;
      e.preventDefault();
      e.stopImmediatePropagation();
      if (e.repeat && (e.code === SAVE_KEY || e.code === LOAD_KEY || e.code === PAUSE)) return;
      if (e.code === SAVE_KEY) void keys.current.save();
      else if (e.code === LOAD_KEY) void keys.current.load();
      else if (e.code === PAUSE) setPaused((x) => !x);
      else down.current.add(e.code);
    };
    const ku = (e: KeyboardEvent) => {
      down.current.delete(e.code);
      if (OWN.has(e.code) && !(e.target as HTMLElement | null)?.closest?.(".details")) e.preventDefault();
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

  // The server's check of each save point, polled while any is unchecked.
  useEffect(() => {
    if (!sessions.some((s) => s.saves.some((x) => !x.native && !x.error))) return;
    const t = setTimeout(() => void refresh().catch(() => undefined), 2000);
    return () => clearTimeout(t);
  }, [sessions, refresh]);

  const items = useMemo(() => filterItems(all, filter, source), [all, filter, source]);
  const total = all.filter((i) => i.kind === "save").length;
  const made = all.filter((i) => i.kind === "made" && i.clip).length;
  const picture = (i: StripItem & { kind: "save" }) => pictures[i.key] ?? (i.save.picture ? client.current.picture(i.session, i.save.id) : "");

  const says = !ready
    ? "Starting the original."
    : paused
      ? "Paused. Press P to go on."
      : `${cutSays(cut, frame, settings)}${follow === "none" && showModel ? " The rebuilt game can't start from this save state yet; play on and save again." : ""}`;

  // The thumbnails of the clips in progress (calmfold.ts), needs you first.
  const ctx: Ctx = { bg: bg.state, work: work.state, changed, now: Date.now() };
  const needs = needsYouClips(bg.state, work.state);
  const ordered = thumbOrder(ctx, cards, trends);
  const thumbPicture = (c: Card) => (c.kind === "made" ? gclient.current.picture(run, c.start.id) : picture(c));
  const thumbs: Thumb[] = ordered.slice(0, MAX_THUMBS).map((c) => ({
    id: c.clip.id,
    name: c.clip.name ?? (c.kind === "save" ? `Clip from ${clock(c.save.frame)}` : c.clip.id),
    picture: thumbPicture(c),
    permille: matchPermille(c.clip.score),
    badges: clipBadges(ctx, c.clip, trends[c.clip.id], needs),
  }));
  const firstNeeds = ordered.find((c) => needs.has(c.clip.id));
  const openId = (id: string) => {
    const it = cardOf(id);
    if (it?.clip) setWatch({ clip: it.clip, item: it });
  };
  const detail = watch && !watch.auto ? watch : undefined;
  const strip = (
    <Filmstrip
      items={items}
      total={total}
      current={current}
      playing={playing}
      picture={picture}
      choose={setCurrent}
      load={(k) => void load(k)}
      openClip={(c) => openClip(c)}
      filter={filter}
      setFilter={setFilter}
      source={source}
      setSource={setSource}
      made={made}
      madePicture={(i) => gclient.current.picture(run, i.start.id)}
    />
  );
  const recorder = freePlay && !watch;
  const counts = headerCounts(work.state);

  return (
    <section className={`unified${detail ? " with-drawer" : ""}`} data-testid="unified">
      <header className="calm-head" data-testid="controls">
        <FidelityGauge f={fid} />
        <div className="head-group">
          <button className={bg.state?.enabled ? undefined : "primary"} onClick={() => void bg.toggle()} disabled={!bg.state} data-testid="ctl-background" title="Score every kept clip and fix numbers on the ones that differ, in the background">
            {bg.state?.enabled ? "pause" : "start"}
          </button>
          <WorkersControl state={bg.state} set={(n) => void bg.setWorkers(n)} />
          <AgentsControl api={api} run={run} />
          <FlightControl api={api} run={run} />
        </div>
        <div className="head-group head-right">
          {(counts.ready || counts.notes) && (
            <span className="head-quiet hint" data-testid="head-counts">
              {counts.ready && <span data-testid="fixes-ready">{counts.ready}</span>}
              {counts.notes && <span data-testid="notes-fixed">{counts.notes}</span>}
            </span>
          )}
          {needs.size > 0 && (
            <button className="chip needs-you loud" onClick={() => firstNeeds && openId(firstNeeds.clip.id)} data-testid="needs-you" title="Open the first clip that needs you">
              ? {needs.size} need you
            </button>
          )}
          <button className={freePlay ? "on" : undefined} aria-pressed={freePlay} onClick={() => (setFreePlay(!freePlay), setWatch(undefined))} disabled={!ready} data-testid="ctl-freeplay" title="Open the recorder: play the original yourself, F5 and F8 save and load, and every save or load starts a clip">
            {freePlay ? "close the recorder" : "record"}
          </button>
          <button className={playRebuild ? "on" : undefined} aria-pressed={playRebuild} onClick={() => (setPlayRebuild(!playRebuild), setFreePlay(false), setClipPaused(!playRebuild))} disabled={!g?.preview} data-testid="ctl-play-rebuild" title="Play the rebuilt game yourself, with the keyboard or a gamepad">
            {playRebuild ? "back to clips" : "play the rebuild"}
          </button>
          <button onClick={() => void generate()} disabled={making} title="Make clips by itself from the run's recordings" data-testid="ctl-generate">
            {making ? "generating..." : "generate clips"}
          </button>
          <button className={drawer ? "on" : undefined} aria-pressed={drawer} onClick={() => setDrawer(!drawer)} data-testid="ctl-more" title="Progress in numbers, the work's log and every clip">
            more
          </button>
        </div>
      </header>
      {playRebuild && <PlayRebuild api={api} run={run} back={() => (setPlayRebuild(false), setClipPaused(false))} />}
      {!playRebuild && <div className={`games ${showModel ? view.layout : "solo"}`} data-layout={showModel ? view.layout : "solo"}>
        <figure className="screen-host">
          <ScreenPointer
            active={!!detail}
            box={pin?.box}
            testid="screen-point"
            onBox={(box) => {
              if (!detail) return;
              const base = detail.item.kind === "save" ? detail.item.save.frame : undefined;
              setClipPaused(true);
              setPin({ box, click: box.w === CLICK && box.h === CLICK, frame: watchAt, orig: base === undefined ? undefined : base + watchAt });
            }}
          >
            <canvas ref={canvas} width={W} height={H} className="game-screen" data-testid="screen-original" />
          </ScreenPointer>
          {watch && view.boxes && <ScreenBoxes boxes={boxesAt(scores[watch.clip.id]?.screen ?? undefined, watchAt)} w={W} h={H} />}
        </figure>
        {showModel && (
          <figure className="model-side" style={view.layout === "top" ? { opacity: view.opacity } : undefined}>
            <iframe key={modelSrc} ref={iframe} title="the game rebuilt from the model" src={modelSrc} className="game-screen" data-testid="screen-model" />
            <figcaption>{watch && trial !== undefined ? `Rebuilt with attempt ${trial}` : "Rebuilt from the model"}</figcaption>
          </figure>
        )}
      </div>}
      {!playRebuild && <div className="view-row" data-testid="view-row">
        <label title="Show the game rebuilt from the model on top of the original">
          <input type="checkbox" checked={view.show} disabled={!g?.preview} onChange={(e) => setView({ ...view, show: e.target.checked })} data-testid="ctl-rebuilt" /> rebuilt game
        </label>
        {showModel && (
          <select value={view.layout} onChange={(e) => setView({ ...view, layout: e.target.value as Layout })} data-testid="ctl-layout">
            <option value="top">on top</option>
            <option value="diff">the differences</option>
            <option value="side">side by side</option>
          </select>
        )}
        {showModel && view.layout === "top" && (
          <input type="range" min={0} max={100} value={Math.round(view.opacity * 100)} onChange={(e) => setView({ ...view, opacity: Number(e.target.value) / 100 })} data-testid="ctl-opacity" title="How much of the rebuilt game shows over the original" />
        )}
        <label title="While you aren't recording, the screen loops the clip the background work is on">
          <input type="checkbox" checked={followWork} onChange={(e) => setFollowWork(e.target.checked)} data-testid="progress-follow" /> follow the work
        </label>
        {watch?.auto && (
          <button className="link one-line" onClick={() => setWatch({ ...watch, auto: false })} data-testid="showing" title="Open this clip">
            showing {watch.clip.name ?? watch.clip.id}
          </button>
        )}
      </div>}
      {recorder && !playRebuild && (
        <section className="recorder" data-testid="recorder">
          <div className="controls play-controls" data-testid="play-controls">
            <button onClick={() => setPaused(!paused)} disabled={!ready} title="P" data-testid="ctl-play">
              {paused ? "play" : "pause"}
            </button>
            <button onClick={() => void saveNow()} disabled={!ready} title="F5, or the left trigger" data-testid="ctl-save">
              save (F5)
            </button>
            <button onClick={() => void load(current)} disabled={!ready || !current} title="F8 or the right trigger: the save state chosen in the strip" data-testid="ctl-load">
              load (F8)
            </button>
            <label title="Every save or load starts a clip that ends at the next save or load, or after the cap">
              <input type="checkbox" checked={settings.on} onChange={(e) => setSettings({ ...settings, on: e.target.checked })} data-testid="ctl-clip-on" /> clips
            </label>
            {cut.open ? (
              <button onClick={stopClip} className="rec-on" data-testid="ctl-clip-stop">
                stop clip
              </button>
            ) : (
              <button onClick={() => void saveNow(true)} disabled={!ready} title="make a save state here and record from it until stop" data-testid="ctl-clip-start">
                start clip
              </button>
            )}
            <details className="clip-settings">
              <summary>clip settings</summary>
              <label>
                clips run up to{" "}
                <input type="number" min={1} max={30} value={settings.seconds} onChange={(e) => setSettings({ ...settings, seconds: capSeconds(Number(e.target.value)) })} data-testid="ctl-clip-seconds" style={{ width: "3.5em" }} /> seconds
              </label>
              <label>
                <input type="checkbox" checked={settings.dropIdle} onChange={(e) => setSettings({ ...settings, dropIdle: e.target.checked })} data-testid="ctl-drop-idle" /> leave out clips where nothing is played
              </label>
            </details>
            {showModel && profiles.length > 1 && (
              <span className="physics-switch" role="group" aria-label="physics" data-testid="ctl-physics" title="How the rebuilt game moves: retro is the original's physics, quirks and all; normal turns the NES quirks off. Only the rebuilt game here changes; scores always use retro.">
                physics:{" "}
                {profiles.map((p, i) => (
                  <span key={p}>
                    {i > 0 && " | "}
                    <button className={p === shownPhysics ? "on" : undefined} aria-pressed={p === shownPhysics} onClick={() => setPhysics(p)} data-testid={`ctl-physics-${p}`}>
                      {p}
                    </button>
                  </span>
                ))}
              </span>
            )}
            <span className="play-line one-line" data-testid="play-line" title={note}>
              {says}
            </span>
          </div>
          {strip}
        </section>
      )}
      <Thumbs thumbs={thumbs} open={openId} selected={watch?.clip.id} more={Math.max(0, ordered.length - MAX_THUMBS)} />
      <GridHeat grid={grid} bg={bg.state} work={work.state} open={openId} />
      {drawer && (
        <section className="more-panel details-drawer" data-testid="details">
          <Progress gen={gen} making={making} counts={tally(activeItems(all, bg.state?.active))}>
            {fid && <FidelityLine f={fid} />}
            <AgentProgress state={work.state} />
          </Progress>
          <div className="status-line" data-testid="status-line">
            <BackgroundLine g={g} state={bg.state} history={bg.history} toggle={() => void bg.toggle()} openQuestion={openQuestion} />
            <button onClick={() => void bg.verify()} disabled={!bg.state} data-testid="bg-verify" title="Compute every result in the run's ledger again and check each one comes out bit for bit the same">
              verify the results
            </button>
          </div>
          <LiveCards
            state={bg.state}
            picture={(id) => {
              const c = cardOf(id);
              return c ? thumbPicture(c as Card) : undefined;
            }}
            open={openId}
          />
          {!recorder && strip}
        </section>
      )}
      {detail && (
        <ClipDrawerFor
          w={detail}
          g={g}
          ctx={ctx}
          score={bg.state?.scores?.[detail.clip.id] ?? scores[detail.clip.id]}
          trend={trends[detail.clip.id]}
          history={bg.history}
          at={watchAt}
          boxes={view.boxes}
          setBoxes={(b) => setView({ ...view, boxes: b })}
          error={watchError ?? work.error}
          mark={(m) => void markClip(m)}
          fix={() => {
            void bg.fix(detail.clip.id);
            // A clip whose loop is stuck on a question needs a change to how a mechanic works: the clip agent starts on it.
            if (stuckOnQuestion(bg.state, detail.clip.id)) void work.start(detail.clip.id);
          }}
          working={bg.state?.focus === detail.clip.id && !!bg.state?.enabled && !bg.state?.error}
          doing={clipDoing(bg.state, detail.clip.id)}
          answer={(q, c) => void bg.answer(q, c).then(() => setWatch({ ...detail, question: undefined }))}
          close={() => setWatch(undefined)}
          playOn={detail.item.kind === "save" ? () => (setWatch(undefined), setFreePlay(true), void load(detail.item.key)) : undefined}
          watching={trial}
          watch={setTrial}
          pin={pin}
          clearPin={() => setPin(undefined)}
          clipPaused={clipPaused}
          setClipPaused={setClipPaused}
          sendTell={async (text) => {
            await work.tell(detail.clip.id, tellBody(text, pin));
            setPin(undefined);
          }}
          keep={(n) => void work.approve(n)}
          startAgent={scores[detail.clip.id]?.exact ? undefined : () => void work.start(detail.clip.id)}
          stopAgent={() => void work.stop()}
          curve={
            <ClipCurve
              api={api}
              run={run}
              clip={detail.clip.id}
              at={watchAt}
              job={bg.state?.job}
              model={bg.state?.scores?.[detail.clip.id]?.model}
              attempt={trial !== undefined ? { n: trial, path: work.client.curvePath(run, trial) } : undefined}
            />
          }
        />
      )}
      {error && <p className="error">{error}</p>}
    </section>
  );

}

/**
 * What the background work is doing for one clip, in words: working on
 * it now (the queue's own sentence, with what it's trying), waiting
 * behind other clips, or stopped with why. Nothing while the queue is off
 * and the clip wasn't picked.
 */
export function clipDoing(s: BgState | undefined, clip: string): { says: string; failed: boolean; busy: boolean } | undefined {
  if (!s) return undefined;
  const mine = s.focus === clip;
  if (s.error && (mine || s.enabled)) return { says: `The background work stopped: ${s.error}.`, failed: true, busy: false };
  if (!s.enabled) return mine ? { says: "The background work is paused; press fix this clip to go on.", failed: false, busy: false } : undefined;
  const job = s.job;
  if (job?.clip === clip) {
    const trying = job.trying ? ` Trying ${job.trying.param.split(".").slice(-2).join(" ")} = ${job.trying.value}.` : "";
    return { says: `${job.kind === "score" ? "Scoring this clip now." : `Working on this clip now (round ${job.round ?? "?"}).`}${trying}`, failed: false, busy: true };
  }
  if (mine) return { says: job ? `Waiting: ${s.says ?? "the background work is busy with another clip"}` : (s.says ?? "Starting."), failed: false, busy: !!s.working };
  return undefined;
}

/** Whether the background work is stuck on this clip: an open question about it, or no numbers made it match. */
export function stuckOnQuestion(s: BgState | undefined, clip: string): boolean {
  return openQuestions(s).some((q) => q.clip === clip) || !!(s as (BgState & { stuck?: Record<string, string> }) | undefined)?.stuck?.[clip];
}

/**
 * Each track's first wrong frame in words, the bodies first and the camera
 * apart: "Mario: wrong from frame 1 (Mario's x speed); the camera: wrong
 * from frame 40 (the camera's position)".
 */
export function trackWords(g: GameWords | undefined, tracks: { track: string; frame: number; path: string }[]): string {
  return tracks
    .map((t) => {
      const who = t.track === "camera" ? "the camera" : t.track === "game" ? "the game" : t.track.charAt(0).toUpperCase() + t.track.slice(1);
      return `${who}: wrong from frame ${t.frame} (${pathPhrase(g, t.path)})`;
    })
    .join("; ")
    .replace(/^./, (c) => c.toUpperCase());
}

/** A clip's state in words, for its details: what it is and how close the two games are. */
export function clipWords(w: Watch, g: GameWords | undefined, score: ClipCardScore | undefined): string {
  const c = w.clip;
  const st = clipState({ ...c, score: score ?? c.score });
  const what = w.item.kind === "save" ? `Clip from ${clock(w.item.save.frame)}, ${secs(c.frames)}. Played by you.` : `${c.name ?? "Clip"}, ${secs(c.frames)}. Made by the tool from ${w.item.start.name}.`;
  const words =
    st === "waiting"
      ? "Not scored yet; the background work scores it soon."
      : st === "waits"
        ? "Not scored yet: it waits until the clips it needs match (the presses it combines, each alone)."
        : st === "no start"
          ? "The rebuilt game can't start from the original's state here (the game isn't under way yet), so the background work leaves it out."
          : score?.exact
            ? `The rebuilt game matches the original all the way${score.fixed ? " (fixed by the background work)" : ""}.`
            : score
              ? score.tracks?.length
                ? `${trackWords(g, score.tracks)}. ${Math.round(score.permille / 10)}% of it together.`
                : `They drift apart at ${secs(score.drift ?? 0)}${score.drift_path ? `: ${pathPhrase(g, score.drift_path)}` : ""}. ${Math.round(score.permille / 10)}% of it together.`
              : "Skipped: the background work leaves it out.";
  return `${what} ${words}`;
}

type ClipCardScore = { exact: boolean; permille: number; drift?: number | null; drift_path?: string | null; fixed?: boolean; error?: string | null; says?: string; tracks?: { track: string; frame: number; path: string; says: string }[]; screen?: ScreenScore | null };

/** The one-line agent row for a clip: the clip agent handed it (alone or in a group), or a writer on it. */
export function agentRowFor(s: BgState | undefined, work: WorkState | undefined, clip: string): { says: string; busy: boolean } | undefined {
  const a = (s?.clip_agents ?? []).find((x) => x.clip === clip || (x.clips ?? []).includes(clip));
  if (a) return { says: a.row ?? a.says ?? `Agent on ${a.about ?? "this clip"}`, busy: a.state === "starting" || a.state === "working" };
  const own = agentLine(work?.clip_agent, clip);
  if (own) return own;
  const w = workingOn(work, clip).find((x) => x.clip === clip);
  return w ? { says: w.says, busy: true } : undefined;
}

/** The detail drawer of an open clip, from the page's state (Calm.tsx's ClipDetail). */
export function ClipDrawerFor({
  w,
  g,
  ctx,
  score,
  trend,
  history = [],
  at,
  boxes = true,
  setBoxes = () => {},
  error,
  mark,
  fix,
  working,
  doing,
  answer,
  close,
  playOn,
  watching,
  watch = () => {},
  keep = () => {},
  startAgent,
  stopAgent,
  curve,
  pin,
  clearPin,
  clipPaused,
  setClipPaused,
  sendTell,
}: {
  w: Watch;
  g?: GameWords;
  ctx: Ctx;
  score?: ClipCardScore;
  trend?: number[];
  history?: BgRound[];
  at: number;
  boxes?: boolean;
  setBoxes?: (b: boolean) => void;
  error?: string;
  mark: (m: Mark) => void;
  fix: () => void;
  working: boolean;
  doing?: { says: string; failed: boolean; busy: boolean };
  answer: (question: string, choice: string) => void;
  close: () => void;
  playOn?: () => void;
  watching?: number;
  watch?: (n: number | undefined) => void;
  keep?: (n: number) => void;
  startAgent?: () => void;
  stopAgent?: () => void;
  curve?: React.ReactNode;
  pin?: Pin;
  clearPin?: () => void;
  clipPaused?: boolean;
  setClipPaused?: (p: boolean) => void;
  sendTell?: (text: string) => Promise<void>;
}) {
  const c = w.clip;
  const s = score ?? c.score;
  const st = clipState({ ...c, score: s });
  const qs = clipQuestions(ctx.bg, c.id);
  // The one it was opened for first.
  const first = (l: BgQuestion[]) => (w.question ? [...l.filter((x) => x.id === w.question!.id), ...l.filter((x) => x.id !== w.question!.id)] : l);
  const attempts = [...clipAttempts(ctx.work, c.id)].sort((a, b) => b.n - a.n);
  const rounds = roundRows(history.filter((r) => r.clip === c.id).reverse());
  return (
    <ClipDetail
      g={g}
      name={c.name ?? (w.item.kind === "save" ? `Clip from ${clock(w.item.save.frame)}` : c.id)}
      sub={`${w.item.kind === "save" ? "Played by you" : "Made by the tool"}, ${secs(c.frames)}`}
      status={clipStatus(ctx, { id: c.id, score: s })}
      permille={matchPermille(s)}
      frames={c.frames}
      at={at}
      parts={whereItParts(g, s)}
      job={jobOn(ctx.bg, c.id)}
      agent={agentRowFor(ctx.bg, ctx.work, c.id)}
      attempts={attempts}
      rounds={rounds}
      watching={watching}
      watch={watch}
      keep={keep}
      startAgent={startAgent}
      stopAgent={stopAgent}
      needsYou={first(qs.needsYou)}
      other={first(qs.other)}
      asks={agentAsks(ctx.work, c.id)}
      tells={clipTells(ctx.work, c.id)}
      fixReady={fixReady(ctx.work, c.id)}
      pointed={pin ? pinWords(pin) : undefined}
      clearPoint={clearPin}
      paused={clipPaused}
      setPaused={setClipPaused}
      tell={sendTell}
      answer={answer}
      fix={!s?.exact && c.mark !== "skipped" && st !== "no start" ? fix : undefined}
      fixing={working}
      mark={c.mark}
      setMark={mark}
      playOn={playOn}
      close={close}
      error={error}
      more={
        <div className="clip-more" data-testid="clip-more">
          <p className="clip-what">{clipWords(w, g, s)}</p>
          {doing && (
            <p className={doing.failed ? "clip-doing error" : "clip-doing"} data-testid="clip-doing">
              {doing.busy && <span className="busy-dot" aria-hidden="true" />}
              {doing.says}
            </p>
          )}
          {trend && trend.length > 1 && (
            <p className="clip-trend">
              its score on this page <Sparkline values={trend} />
            </p>
          )}
          {s?.screen && (
            <div className="clip-screen" data-testid="clip-screen">
              {screenLines(s.screen).map((l) => (
                <p key={l}>{l}</p>
              ))}
              <label title="Outline the places the two games' pictures differ, frame by frame: the characters in red, the background dashed">
                <input type="checkbox" checked={boxes} onChange={(e) => setBoxes(e.target.checked)} data-testid="clip-boxes" /> show where the pictures differ
              </label>
            </div>
          )}
          {curve}
        </div>
      }
    />
  );
}

const FOLLOW_KEY = "gp.follow";

function readFollow(): boolean {
  try {
    return localStorage.getItem(FOLLOW_KEY) !== "0";
  } catch {
    return true;
  }
}

function saveFollow(on: boolean): void {
  try {
    localStorage.setItem(FOLLOW_KEY, on ? "1" : "0");
  } catch {
    // Not kept.
  }
}

/**
 * Where the work stands, always on screen: making clips (with a bar), the
 * clips scored and how they stand (with a bar), and whether the screen
 * follows the background work.
 */
export function Progress({ gen, making, counts, children }: { gen?: GenState; making: boolean; counts: ReturnType<typeof tally>; children?: React.ReactNode }) {
  const job = gen?.job;
  const pool = counts.total - counts.skipped;
  const pct = (k: number, n: number) => `${n ? Math.round((100 * k) / n) : 0}%`;
  const matchPct = pool ? Math.round((100 * counts.matches) / pool) : 0;
  return (
    <div className="progress" data-testid="progress">
      <div className="progress-headline" data-testid="progress-headline">
        <span className="progress-big">{counts.matches}</span>
        <span>
          of {pool} clips match the original exactly ({matchPct}%)
          <span className="hint">
            {" "}
            {counts.close} close, {counts.differs} differ, {counts.waiting} not scored yet
          </span>
        </span>
      </div>
      <div className="progress-goal" aria-hidden="true">
        <span style={{ width: `${matchPct}%` }} />
      </div>
      {(making || gen?.finished) && (
        <div className="progress-row" data-testid="progress-making">
          <span className="progress-label">Making clips</span>
          <span className="progress-bar">
            <span style={{ width: making ? pct(job?.k ?? 0, job?.n ?? 1) : "100%" }} />
          </span>
          <span className="one-line" title={gen?.says}>
            {gen?.says ?? (making ? "Starting." : "Done.")}
          </span>
        </div>
      )}
      <div className="progress-row" data-testid="progress-scored">
        <span className="progress-label">Scored</span>
        <span className="progress-bar">
          <span className="match" style={{ width: pct(counts.matches, pool) }} />
          <span className="close" style={{ width: pct(counts.close, pool) }} />
          <span className="differ" style={{ width: pct(counts.differs, pool) }} />
          <span className="nostart" style={{ width: pct(counts.noStart, pool) }} />
        </span>
        <span className="one-line" title={tallyWords(counts)}>
          {tallyWords(counts)}
        </span>
      </div>
      {children}
    </div>
  );
}

const FREEPLAY_KEY = "gp.freeplay";
const PHYSICS_KEY = "gp.physics";

function readText(key: string, fallback: string): string {
  try {
    return localStorage.getItem(key) ?? fallback;
  } catch {
    return fallback;
  }
}

function saveText(key: string, v: string): void {
  try {
    localStorage.setItem(key, v);
  } catch {
    // Kept for this page only.
  }
}
const DRAWER_KEY = "gp.details";

function readFlag(key: string, fallback: boolean): boolean {
  try {
    const v = localStorage.getItem(key);
    return v === null ? fallback : v === "1";
  } catch {
    return fallback;
  }
}

function saveFlag(key: string, on: boolean): void {
  try {
    localStorage.setItem(key, on ? "1" : "0");
  } catch {
    // Kept for this page only.
  }
}
