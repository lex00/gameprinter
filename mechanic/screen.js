// The screen (screen.html, docs/mechanic-page.md, "The screen"): the
// original and the model on one screen, for a person who talks to their
// agent and watches. The original is the person's ROM in gp_play.wasm, as
// the mechanic page plays it (nesplayer.js); the model is the rebuild's
// preview in control mode in a frame off screen, as CLIPS's PLAY THE MODEL
// drives it (app.js's rbStart, rbPost and drawLiveRebuild): it starts from
// the original's state, translated by the server, and takes the frames the
// original read the pad on, so both get the same presses.
//
// Two ways to watch, three views. Live: the person plays the original with
// the keyboard or a gamepad and the model follows; RECORD sends what they
// played to the open mechanic's session as CLIPS's hand-off does. A clip:
// both games at a frame of a clip of the session, still or playing. The
// views are side by side, the model over the original at a transparency,
// their difference, and the model alone at its own size, which plays from
// its own start live (as the theme's play page did: SEE IT IN THE GAME
// opens it with the theme picked). The pictures are the game's own or one
// of the person's themes for the game.
//
// The agent sets any of it with screen_show (gp mechanic screen): the page
// reads the ask from /api/screen every second and applies it, and reports
// what it shows with POST /api/screen, saying when the person chose it.
// It is the developer's loop: a theme file changed shows within a few
// seconds, and a model changed is built again by itself while the old
// build plays, then swapped in at the same place.
//
// The screen page shows only the screens, YOUR TURN and one status line
// (owner, page-minimal). The loop's page, this same file at
// screen.html?page=loop (LOOP_PAGE), leads with PROGRESS (a line for each
// part on the game's scoreboard: its state and what is being done for it)
// and YOUR TURN, with everything below folded under WHAT THE AGENTS ARE
// DOING (owner, loop-page-progress-first). There it shows the quest log (the rows `gp
// mechanic queue GAME` prints), grouped NEEDS YOU, AGENT WORKING (a row
// marked so, or one a live worker of the runner's is on), UP NEXT, IN LINE
// and DONE (those two folded), the steps, the workers and the clips of every row, and the screens
// only while a clip is open on them. A recording goes to the row outlined: the open one when it
// starts. A row's RECORD INTO THIS ROW moves it there (api/pick when not
// recording); a click elsewhere on the row only opens its details, the
// RAM tests, addresses and hashes. A clip clicked shows on both screens.
//
// The random draws: with a mapping that scores with the original's draws
// (`score_draws` and its `[[draws]]` sites, docs/formats/draws.md), the
// player records each frame's draws at the sites as the original makes
// them, and the model takes them in place of its own generator, as a
// clip's score gives them, so the two make the same random choices live.
// A recording keeps the draws it saw (the clip's NNN.draws), and a clip
// shown gives the model its draws, kept or recorded again from its replay.

import { NesPlayer } from "./nesplayer.js";
import { InputTimeline, settings as inputSettings } from "./input-timeline.js";

const $ = (id) => document.getElementById(id);

/** Whether this is the loop's page (screen.html?page=loop), not the screen. */
const LOOP_PAGE = new URLSearchParams(location.search).get("page") === "loop";
document.body.classList.toggle("loop-page", LOOP_PAGE);
/** The loop's page, opened at row `m` when one is given. */
const loopHref = (m) => `screen.html?page=loop${m ? `&row=${encodeURIComponent(m)}` : ""}`;
// The loop's page leads with PROGRESS, then YOUR TURN (the screen page's,
// moved here), then WHAT THE AGENTS ARE DOING, folded.
if (LOOP_PAGE) $("progress").after($("your-turn"));

/** One plain sentence of `text`: the question it asks, else its first
 * sentence, at most `max` characters (cut at a word, with "..."). */
function oneSentence(text, max = 160) {
  const t = String(text ?? "").replace(/\s+/g, " ").trim();
  const sentences = t.match(/[^.!?]+(?:[.!?]+(?=\s|$)|$)/g) ?? [t];
  let s = (sentences.find((x) => isQuestion(x)) ?? sentences[0] ?? "").trim();
  if (s.length > max) s = `${s.slice(0, max - 3).replace(/\s+\S*$/, "")}...`;
  return s;
}

/** A span of `text` cut to one sentence, with MORE showing the rest. */
function shortText(cls, text) {
  const full = String(text ?? "").trim();
  const short = oneSentence(full);
  const s = el("span", cls, short);
  if (short === full) return s;
  s.title = full;
  const more = el("button", "more", "MORE");
  more.type = "button";
  more.title = "Show all of it";
  more.addEventListener("click", (e) => {
    e.stopPropagation();
    e.currentTarget.blur();
    const open = more.textContent === "MORE";
    s.firstChild.textContent = open ? full : short;
    more.textContent = open ? "LESS" : "MORE";
  });
  s.append(more);
  return s;
}

// The machine's keys, as app.js's LAYOUTS have them: each button's keys in
// the order of the player's bits.
const LAYOUTS = {
  nes: {
    fps: 60.0988,
    keys: [["KeyX", "Space"], ["KeyZ", "ShiftLeft"], ["ShiftRight", "Tab"], ["Enter"], ["ArrowUp", "KeyW"], ["ArrowDown", "KeyS"], ["ArrowLeft", "KeyA"], ["ArrowRight", "KeyD"]],
    keys2: [["KeyM"], ["KeyN"], ["KeyH"], ["KeyY"], ["KeyI"], ["KeyK"], ["KeyJ"], ["KeyL"]],
    footer: "Arrows move, X is A, Z is B, Enter is Start. R records.",
  },
  arcade: {
    fps: 30,
    keys: [["Digit5"], ["Digit1"], ["ArrowRight", "KeyD"], ["ArrowLeft", "KeyA"], ["ArrowDown", "KeyS"], ["ArrowUp", "KeyW"], ["KeyX", "Space"]],
    keys2: [],
    footer: "5 drops a coin and 1 starts. Arrows move, X or Space taps. On a controller: Select is the coin, Start starts, A or the right stick taps. R records.",
  },
};
let LAYOUT = LAYOUTS.nes;
let FPS = LAYOUT.fps;
let W = 256, H = 240;

let player = null, rom = null;
// The set the page's ROM is (api/rom: its id and MAME system), and the
// other sets' ROMs fetched for clips recorded on them (rom?set=SET).
let romSet = null;
const setRoms = new Map();
let session = null; // api/session: the game and the mechanic open

/** The set an input log was recorded on: an arcade log's `system` line. */
function setOf(log) {
  return /^system (\S+)/m.exec(log ?? "")?.[1] ?? null;
}

/** The ROM a log plays on: the set it was recorded on, whatever set the
 * picker holds; the page's own when it is that set or names none. */
async function romFor(log) {
  const want = setOf(log);
  if (!want || !romSet || want === romSet.set || want === romSet.system) return rom;
  if (!setRoms.has(want)) {
    const r = await fetch(`rom?set=${encodeURIComponent(want)}`);
    if (!r.ok) throw new Error(await r.text());
    setRoms.set(want, new Uint8Array(await r.arrayBuffer()));
  }
  return setRoms.get(want);
}

/** A clip's name on the clip bar, with the set it plays on. */
function clipName(name, log) {
  const set = setOf(log);
  return set ? `${name} (ON ${set.toUpperCase()})` : name;
}
const down = new Set();

// What is shown. `by` is who chose it last: the agent's ask or the person.
const show = { view: "side", opacity: 0.5, pictures: "game", by: "person" };
let mode = "live"; // live | clip
let clip = null; // the clip shown: {n, mechanic, start, frames, pads, polled, upTo, snaps, f, pos, playing, log}
let applied = 0; // the last ask applied
let recording = null; // {start} while recording
let sending = false;
let askError = null; // why the agent's last ask couldn't be shown
// The quest-log row a recording goes to: the open one when it starts; a
// row clicked while it records moves it there (app.js's clipFor).
let clipFor = null;

// The model in its frame.
const model = {
  ready: false, // the frame said it is loaded
  on: false, // it plays (live) or stands at the clip
  queue: [], // live: the presses for it since the last frame
  visible: null, // the part of the screen it draws
  build: null, // the build it loaded (live.rs's key)
  token: 0, // which start is current
  say: "", // what is drawn over it while it isn't on
  draws: null, // how it took the original's draws ({taken, of, desync}), or null
  // The gate card whose change it plays (ask_look.rs: the model as the
  // card's patch would leave it), or null for main's model.
  ask: null,
  // The card a gate card's LOOK AT IT set up, kept while BEFORE shows
  // main's, so AFTER goes back to it.
  card: null,
  loads: 0, // loads begun, so a switch of model knows one ran
};

// The original's random draws, recorded in the player's slot 0 at the
// mapping's draw sites once they are known (api/rebuild/start's
// `draw_sites`).
const draws = {
  sites: null, // the sites, a line each, or null: none to record
  from: null, // slot 0's frame when it began recording them
  log: [], // every draw since: [frame, site, value]
  live: null, // the live model's numbering ({base, lags, early, lines}), or null
};

/** The soft sites of a sites text (`... soft`), as a draws script names
 * them. */
function softLines(sites) {
  return (sites ?? "").split("\n").filter((l) => / soft$/.test(l.trim())).map((l) => `soft ${l.trim().split(/\s+/)[0]}\n`).join("");
}

/** One frame of the original's draws in the model's numbering from a
 * start at the original's frame `n.base`: a frame the game didn't read
 * the pad on, which the model doesn't step, puts the original a frame
 * ahead from then on, as a lockstep's draws.lag does. Draws on or before
 * the start are the start's own. */
function drawLines(n, f, polled, ds) {
  if (f <= n.base) return;
  if (!polled) n.lags++;
  for (const [site, v] of ds) n.lines.push(`${f - n.lags} ${site} ${v}\n`);
}

/** After a frame of live play: its draws kept, and handed to the model
 * following it. */
function liveDraws(polled) {
  if (draws.from === null) return;
  const f = player.frame(0);
  const ds = player.draws(0);
  for (const [site, v] of ds) draws.log.push([f, site, v]);
  const n = draws.live;
  if (!n) return;
  if (n.base === null) n.early.push([f, polled, ds]);
  else drawLines(n, f, polled, ds);
}

/** Starts recording the draws at `sites` in slot 0, once. */
function recordLiveDraws(sites) {
  if (!sites || draws.from !== null || !player.recordsDraws()) return false;
  player.drawSites(0, sites);
  draws.sites = sites;
  draws.from = player.frame(0);
  return true;
}

// The pictures of both, the size of the original's screen.
const offA = document.createElement("canvas");
const offB = document.createElement("canvas");

function line(text) { $("line").textContent = text; }
let lineUntil = 0;
/** A line that stays a few seconds, then the next line goes back to the usual. */
function note(text, ms = 4000) {
  line(text);
  lineUntil = performance.now() + ms;
  // The line sits under the screens; scrolled down to the lists, the person
  // wouldn't see it, so it shows at the foot of the window too.
  const r = $("line").getBoundingClientRect();
  if (r.bottom > 0 && r.top < innerHeight) return;
  let t = $("toast");
  if (!t) {
    t = document.createElement("p");
    t.id = "toast";
    t.setAttribute("aria-live", "polite");
    document.body.append(t);
  }
  t.textContent = text;
  t.hidden = false;
  clearTimeout(note.timer);
  note.timer = setTimeout(() => { t.hidden = true; }, Math.min(ms, 8000));
}

// An error shown to the person: noted as any line, and sent to the loop,
// which files it as a tools row of its own trouble (crates/gp-cli/src/
// trouble.rs; the same words within a minute are sent once).
const faultSent = new Map();
function fault(text, ms = 8000, where = "") {
  note(text, ms);
  const now = Date.now();
  if (now - (faultSent.get(text) ?? 0) < 60000) return;
  faultSent.set(text, now);
  fetch("api/trouble", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ text, where }) }).catch(() => { /* the server is away: nothing to file it in */ });
}

async function api(path, body) {
  const r = await fetch(path, body === undefined ? {} : { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
  if (!r.ok) {
    const t = await r.text();
    let why = `${path}: ${r.status} ${t}`;
    try { why = JSON.parse(t).error ?? why; } catch (e) { /* not JSON: the status says it */ }
    throw new Error(why);
  }
  return r.json();
}

// Controls -------------------------------------------------------------

function firstPad() {
  for (const g of navigator.getGamepads?.() ?? []) if (g && g.connected) return g;
  return null;
}
function secondPad() {
  let n = 0;
  for (const g of navigator.getGamepads?.() ?? []) if (g && g.connected && n++ === 1) return g;
  return null;
}
/** The second stick's axes that have been seen at rest near the middle, by
 * gamepad: an axis that rests at an end (a trigger some controllers report
 * as an axis) never counts as a pull. */
const stickRested = new Map();
/** Whether the gamepad's second stick is pushed any way: on the arcade
 * layout that is the tap, as the cabinet's tap handle was a second stick. */
function secondStick(g) {
  let seen = stickRested.get(g.index);
  if (!seen) stickRested.set(g.index, (seen = new Set()));
  let pushed = false;
  for (const i of [2, 3]) {
    const v = g.axes[i];
    if (typeof v !== "number") continue;
    if (Math.abs(v) < 0.2) seen.add(i);
    else if (Math.abs(v) > 0.5 && seen.has(i)) pushed = true;
  }
  return pushed;
}
/** A gamepad as the machine's bits (app.js's gamepadBits). */
function gamepadBits(g) {
  let pad = 0;
  if (!g) return 0;
  const b = (i) => g.buttons[i]?.pressed === true;
  const ax = g.axes[0] ?? 0, ay = g.axes[1] ?? 0;
  if (LAYOUT === LAYOUTS.arcade) {
    if (b(8)) pad |= 1;
    if (b(9)) pad |= 2;
    if (b(15) || ax > 0.5) pad |= 4;
    if (b(14) || ax < -0.5) pad |= 8;
    if (b(13) || ay > 0.5) pad |= 16;
    if (b(12) || ay < -0.5) pad |= 32;
    if (b(0) || b(1) || b(2) || b(3) || secondStick(g)) pad |= 64;
  } else {
    if (b(0) || b(1)) pad |= 1;
    if (b(2) || b(3)) pad |= 2;
    if (b(8)) pad |= 4;
    if (b(9)) pad |= 8;
    if (b(12) || ay < -0.5) pad |= 16;
    if (b(13) || ay > 0.5) pad |= 32;
    if (b(14) || ax < -0.5) pad |= 64;
    if (b(15) || ax > 0.5) pad |= 128;
  }
  return pad;
}
/** Both controllers: controller 1 in the low byte, 2 in the next. */
function padNow() {
  let p1 = 0, p2 = 0;
  LAYOUT.keys.forEach((ks, bit) => { if (ks.some((k) => down.has(k))) p1 |= 1 << bit; });
  LAYOUT.keys2.forEach((ks, bit) => { if (ks.some((k) => down.has(k))) p2 |= 1 << bit; });
  p1 |= gamepadBits(firstPad());
  if (LAYOUT.keys2.length) p2 |= gamepadBits(secondPad());
  return p1 | (p2 << 8);
}
// The controls' timing (input-timeline.js): every change of the keys and
// the gamepad is kept with its time, and each game frame reads the buttons
// at its own time. The settings are the person's, kept in this browser.
const INPUT_KEY = "gp-input";
function savedInput() {
  let saved = null;
  try { saved = JSON.parse(localStorage.getItem(INPUT_KEY) ?? "null"); } catch (e) { /* none kept */ }
  const q = new URLSearchParams(location.search);
  const over = {};
  for (const [param, key] of [["lead", "leadMs"], ["assist", "assistMs"], ["poll", "pollMs"]]) if (q.has(param)) over[key] = q.get(param);
  return inputSettings({ ...(saved ?? {}), ...over });
}
const timeline = new InputTimeline(savedInput());
/** The controls changed, or may have: the timeline hears what they are at
 * time `t`. */
function noteInput(t) { timeline.set(padNow(), t); }
let padPoll = 0;
/** A gamepad can only be polled: this does, every pollMs. */
function pollPads() {
  clearInterval(padPoll);
  padPoll = setInterval(() => noteInput(performance.now()), timeline.opts.pollMs);
}
/** The CONTROLS' TIMING box under the screens: the three settings, and how
 * the last press was read. */
function wireInput() {
  const box = $("input-set");
  if (!box) return;
  const fields = { leadMs: $("input-lead"), assistMs: $("input-assist"), pollMs: $("input-poll") };
  for (const [k, el] of Object.entries(fields)) {
    if (!el) continue;
    el.value = String(timeline.opts[k]);
    el.addEventListener("change", () => {
      timeline.opts = inputSettings({ ...timeline.opts, [k]: el.value });
      el.value = String(timeline.opts[k]);
      try { localStorage.setItem(INPUT_KEY, JSON.stringify(timeline.opts)); } catch (e) { /* not kept */ }
      pollPads();
    });
    // A setting's box takes the keys typed in it, not the game.
    el.addEventListener("keydown", (ev) => ev.stopPropagation());
  }
  pollPads();
}
let inputShown = "";
/** Says how the last press was read, when it changed. */
function showLastPress() {
  const el = $("input-last");
  const p = timeline.lastPress();
  if (!el || !p) return;
  const says = `Last press: held ${p.ms} ms, read for ${p.frames} ${p.frames === 1 ? "frame" : "frames"} (a frame is ${Math.round(1000 / FPS)} ms).`;
  if (says !== inputShown) { inputShown = says; el.textContent = says; }
}

/** A pad as a frames file's line (app.js's framesLine). */
function framesLine(p) {
  const hex = (b) => b.toString(16).padStart(2, "0");
  return p >> 8 ? `${hex(p & 0xff)} ${hex(p >> 8)}` : hex(p & 0xff);
}

// Play logs ------------------------------------------------------------

/** An input log's header and its frames a line each (app.js's splitLog):
 * an arcade log's runs spelt out a frame a line, `1 C......`. */
function splitLog(text) {
  const lines = text.split("\n").filter((l) => l.length);
  if (lines[0] === "gp-arcade-input 0") {
    const head = lines.filter((l) => /^(gp-arcade-input|system|columns) /.test(l));
    const frames = [];
    for (const l of lines) {
      const m = /^(\d+)\s+(\S+)$/.exec(l.split("#")[0].trim());
      if (m) for (let k = 0; k < Number(m[1]); k++) frames.push(`1 ${m[2]}`);
    }
    return { head: head.join("\n") + "\n", frames };
  }
  return { head: lines.slice(0, 2).join("\n") + "\n", frames: lines.slice(2) };
}

/** A frame line of a log as the player's bits. The NES's fields spell
 * RLDUTSBA from bit 7 down (gp-evidence's input_log), controller 2 in the
 * next byte; an arcade log's letters are its columns from bit 0. */
function padOf(l, columns) {
  if (columns) {
    const f = l.split(/\s+/)[1] ?? "";
    let p = 0;
    for (let i = 0; i < f.length; i++) if (f[i] !== ".") p |= 1 << i;
    return p;
  }
  const fields = l.trim().split(/\s+/).slice(1);
  let p = 0;
  fields.slice(0, 2).forEach((f, c) => {
    for (let i = 0; i < 8 && i < f.length; i++) if (f[i] !== ".") p |= 1 << (7 - i + 8 * c);
  });
  return p;
}

// The play the server holds as a path, so a clip sends only what was
// played since (app.js's sendPath).
let sentPath = null; // {path, text}
async function sendPath(text, warm) {
  const { head } = splitLog(text);
  let p;
  if (sentPath && text === sentPath.text) {
    p = { path: sentPath.path };
    if (warm !== undefined) p = await api("api/paths", { base: sentPath.path, tail: head, warm });
  } else if (sentPath && text.startsWith(sentPath.text)) {
    p = await api("api/paths", { base: sentPath.path, tail: head + text.slice(sentPath.text.length), warm });
  } else {
    p = await api("api/paths", { log: text, warm });
  }
  sentPath = { path: p.path, text };
  return p.path;
}

// The model ------------------------------------------------------------

function modelPost(m) {
  $("model-frame").contentWindow?.postMessage(m, location.origin);
}

window.addEventListener("message", (e) => {
  if (e.origin !== location.origin || e.source !== $("model-frame").contentWindow) return;
  const m = e.data;
  if (m?.gp === "ready") { model.ready = true; model.onReady?.(); }
  else if (m?.gp === "failed") model.onFailed?.(m.message);
  else if (m?.gp === "at") {
    model.draws = m.draws ?? null;
    document.body.dataset.draws = m.draws ? JSON.stringify(m.draws) : "";
  }
  else if (m?.gp === "error") fault(`The model: ${m.message}`, 6000);
});

/** The theme's part of the model's address: large art is drawn bigger than
 * the board, so it is fitted. */
function lookParam() {
  return show.pictures !== "game" ? `&theme=${encodeURIComponent(show.pictures)}&scale=fit` : "";
}

/** The rebuild built once at least: builds it first when it isn't, saying
 * how long it has taken. With `ask`, a gate card's look: the model as the
 * card's patch would leave it (ask_look.rs). */
async function ensureBuilt(ask = model.ask) {
  const q = ask ? `?ask=${encodeURIComponent(ask)}` : "";
  let st = await api(`api/rebuild/status${q}`);
  if (st.built) return st;
  if (!st.model) throw new Error("this game has no model yet");
  await api("api/rebuild/build", ask ? { ask } : {});
  while ((st = await api(`api/rebuild/status${q}`)).building !== null) {
    model.say = ask ? "BUILDING THE MODEL AFTER THIS CHANGE" : "BUILDING THE MODEL";
    line(ask
      ? `Building the model as this change leaves it: ${Math.round(st.building)} s. Its module builds first, which takes a few minutes.`
      : `Building the playable model: ${Math.round(st.building)} s. The first build takes a few minutes. Play on meanwhile.`);
    await new Promise((r) => setTimeout(r, 2000));
  }
  if (st.error || !st.built) throw new Error(st.error ?? "the build didn't finish");
  return st;
}

/** Where the model's page is served: main's build, or a card's look's. */
function modelBase() {
  return model.ask ? `rebuild-ask/${encodeURIComponent(model.ask)}/` : "rebuild/";
}

/** The model the screen plays: card `ask`'s change, or main's (null).
 * Whether that changed; the caller loads it (a clip opening loads it). */
function useModel(ask) {
  ask = ask ?? null;
  const changed = model.ask !== ask;
  model.ask = ask;
  document.body.dataset.modelAsk = ask ?? "";
  beforeButton();
  return changed;
}

/** Set while a gate card's LOOK AT IT opens its clip, which keeps the
 * card's model; any other clip opened goes back to main's. */
let cardOpening = false;

/** Back to main's model, forgetting the card: whether it was on another. */
function toMainModel() {
  model.card = null;
  return useModel(null);
}

/** The clip bar's BEFORE (main's model) or AFTER (the card's change),
 * shown while a gate card's clip is on the screens. */
function beforeButton() {
  const b = $("clip-before");
  if (!b) return;
  b.hidden = !model.card || mode !== "clip";
  b.textContent = model.ask ? "BEFORE" : "AFTER";
  b.title = model.ask ? "The model on main, before this change" : "The model after this change, as the card would land it";
}

/** Loads the model's page into its frame (again), then starts it where
 * the screen is: live at the original's frame, or at the clip's. The last
 * picture of the model stays on screen until the new one is on. */
async function loadModel(why) {
  const mine = ++model.token;
  model.loads++;
  // How many times the model was loaded, for a check of the page.
  document.body.dataset.loads = String(Number(document.body.dataset.loads ?? 0) + 1);
  model.ready = false;
  model.on = false;
  const base = modelBase();
  try {
    const st = await ensureBuilt();
    if (mine !== model.token) return;
    model.build = st.build ?? null;
    try {
      const pj = await api(`${base}preview.json`);
      model.visible = Array.isArray(pj.visible) ? pj.visible : null;
    } catch (e) { model.visible = null; }
    const f = $("model-frame");
    await new Promise((resolve, reject) => {
      const t = setTimeout(() => reject(new Error("the model didn't load in 30 s")), 30000);
      model.onReady = () => { clearTimeout(t); resolve(); };
      model.onFailed = (why) => { clearTimeout(t); reject(new Error(`its preview didn't start: ${why}`)); };
      // files=here: the preview reads the build beside it, not the
      // browser's store of installed games.
      f.src = `${base}index.html?control=1&files=here${lookParam()}&t=${Date.now()}`;
    });
    if (mine !== model.token) return;
    await startModel(mine);
    if (why && mine === model.token) note(why);
  } catch (e) {
    if (mine !== model.token) return;
    model.say = model.ask ? "THE MODEL AFTER THIS CHANGE COULDN'T START" : "THE MODEL COULDN'T START";
    fault(model.ask
      ? `The model after this change couldn't start: ${e.message ?? e}. BEFORE shows main's.`
      : `The model couldn't start: ${e.message ?? e}`, 10000);
  }
}

/** Starts the loaded model where the screen is. */
async function startModel(mine = model.token) {
  model.on = false;
  if (mode === "clip") { await startModelAtClip(mine); return; }
  model.queue = [];
  if (show.view === "solo") {
    // The model alone, from its own start: it never waits on the
    // original's title screen (app.js's rbStartFree).
    modelPost({ gp: "frames", text: "gp-frames 1\n", frame: 0 });
    model.free = true;
    model.on = true;
    return;
  }
  model.free = false;
  const frame = player.frame(0);
  // The draws from here on wait for the start's frame.
  draws.live = draws.from !== null ? { base: null, lags: 0, early: [], lines: [] } : null;
  try {
    const path = await sendPath(player.inputLog(0).split("\n").slice(0, frame + 2).join("\n") + "\n");
    const r = await api("api/rebuild/start", { path, frame, ...(model.ask ? { ask: model.ask } : {}) });
    if (mine !== model.token || mode !== "live" || show.view === "solo") return;
    // The sites first heard of: recording starts now, so the start is
    // taken again from here, with every draw after it.
    if (recordLiveDraws(r.draw_sites ?? null)) { void startModel(mine); return; }
    let given;
    const n = draws.live;
    if (n) {
      n.base = r.original_frame ?? frame;
      for (const [f, polled, ds] of n.early) drawLines(n, f, polled, ds);
      n.early = [];
      given = { base: n.base, lines: softLines(draws.sites) + n.lines.join("") };
      n.lines = [];
    }
    const frames = "gp-frames 1\n" + model.queue.map(framesLine).join("\n");
    model.queue = [];
    modelPost({ gp: "load-state", state: r.state, text: frames + "\n", frame: 1e9, draws: given });
    model.on = true;
  } catch (e) {
    // Not in play yet (the title screen): again in a second.
    if (mine !== model.token || mode !== "live") return;
    model.say = "THE MODEL JOINS ONCE THE GAME IS IN PLAY";
    setTimeout(() => { if (mine === model.token && mode === "live" && !model.on) void startModel(mine); }, 1000);
  }
}

/** The model at the clip's start, with the clip's presses on the frames
 * the original read them, stood at the clip's frame. */
async function startModelAtClip(mine) {
  const c = clip;
  if (!c) return;
  try {
    c.path ??= await sendPath(c.log);
    const r = await api("api/rebuild/start", { path: c.path, frame: c.start, ...(model.ask ? { ask: model.ask } : {}) });
    if (mine !== model.token || clip !== c) return;
    const text = "gp-frames 1\n" + c.polled.map(framesLine).join("\n") + "\n";
    let given;
    if (c.draws) {
      const n = { base: r.original_frame ?? c.start, lags: 0, lines: [] };
      for (const [f, polled] of c.frameDraws) drawLines(n, f, polled, c.draws.get(f) ?? []);
      given = { base: n.base, lines: c.soft + n.lines.join("") };
    }
    modelPost({ gp: "load-state", state: r.state, text, frame: c.upTo[c.f] ?? 0, draws: given });
    model.on = true;
  } catch (e) {
    if (mine !== model.token || clip !== c) return;
    model.say = "THE MODEL CAN'T START AT THIS CLIP";
    fault(`The model can't start at the clip's start: ${String(e.message ?? e).slice(0, 160)}`, 8000);
  }
}

/** The model's last picture, in the board's place on its screen. A theme
 * drawn larger than the board is scaled down into it (app.js's
 * drawLiveRebuild). Kept as it was while the model isn't on. */
function grabModel() {
  const c = $("model-frame").contentDocument?.getElementById("game");
  const g = offB.getContext("2d", { willReadFrequently: true });
  if (!model.on || !c || !c.width) {
    if (!model.everOn) {
      g.fillStyle = "#000";
      g.fillRect(0, 0, W, H);
    }
    return false;
  }
  model.everOn = true;
  g.imageSmoothingEnabled = false;
  g.fillStyle = "#000";
  g.fillRect(0, 0, W, H);
  const [vl, vt, vw, vh] = model.visible ?? [0, 0, W, H];
  if (c.width > vw || c.height > vh) {
    g.imageSmoothingEnabled = true;
    g.drawImage(c, vl, vt, vw, vh);
  } else {
    g.drawImage(c, vl, vt);
  }
  return true;
}

// Drawing --------------------------------------------------------------

/** The screens' size: the largest that fits the window, two across side
 * by side, one otherwise. */
let fitKey = "";
function fit() {
  const [w, h] = show.view === "solo" ? [$("a").width, $("a").height] : [W, H];
  const key = `${innerWidth} ${innerHeight} ${w} ${h} ${show.view}`;
  if (key === fitKey) return;
  fitKey = key;
  const two = show.view === "side" && innerWidth > 640;
  const room = Math.min(innerWidth - 32, 1600) - (two ? 12 : 0) - (two ? 8 : 4);
  const high = innerHeight - $("screens").getBoundingClientRect().top - 130;
  const s = Math.max(0.1, Math.min((two ? room / 2 : room) / w, Math.max(120, high) / h));
  document.documentElement.style.setProperty("--cw", `${Math.floor(w * s)}px`);
  document.documentElement.style.setProperty("--ch", `${Math.floor(h * s)}px`);
}
window.addEventListener("resize", () => { fitKey = ""; fit(); });

/** A small label in the corner of a picture. */
function badge(g, text) {
  g.save();
  g.font = `${Math.max(8, Math.round(H / 30))}px monospace`;
  const h = Math.max(10, Math.round(H / 24));
  const y = g.canvas.height - h - 2;
  g.fillStyle = "rgba(0,0,0,0.75)";
  g.fillRect(2, y, g.measureText(text).width + 8, h);
  g.fillStyle = "rgb(252,160,68)";
  g.fillText(text, 6, y + h - 2);
  g.restore();
}

/** The original's picture (offA) and the model's (offB) in the chosen
 * view (app.js's drawPair): side by side, the model over the original at
 * the chosen transparency, or each colour's distance, black where they
 * match. */
function drawPair(haveModel) {
  const a = $("a").getContext("2d"), b = $("b").getContext("2d");
  const side = show.view === "side";
  $("screens").className = show.view;
  $("fig-b").hidden = !side;
  const pics = show.pictures === "game" ? "" : ` IN ${show.pictures.toUpperCase()}`;
  // A gate card's change on the screen says so; BEFORE (main's model) says
  // that it is main's while the card's clip is up.
  const which = model.ask ? "AFTER THIS CHANGE: " : model.card && mode === "clip" ? "BEFORE THIS CHANGE: " : "";
  $("cap-a").textContent = side ? "THE ORIGINAL" : show.view === "top" ? `${which}THE MODEL OVER THE ORIGINAL, ${Math.round(show.opacity * 100)}%` : show.view === "solo" ? `${which}THE MODEL ALONE${pics}` : `${which}THE DIFFERENCE: BLACK WHERE THEY MATCH`;
  $("cap-b").textContent = `${which}THE MODEL${pics}`;
  $("fig-b").classList.toggle("after", !!model.ask);
  if (show.view === "solo") {
    // The model at its own size: a theme's large art kept sharp.
    const c = $("model-frame").contentDocument?.getElementById("game");
    const ok = haveModel && c && c.width;
    const [cw, ch] = ok ? [c.width, c.height] : [W, H];
    if ($("a").width !== cw || $("a").height !== ch) { $("a").width = cw; $("a").height = ch; fitKey = ""; }
    fit();
    if (ok) a.drawImage(c, 0, 0);
    else { a.drawImage(offB, 0, 0); if (model.say) badge(a, model.say); }
    return;
  }
  if ($("a").width !== W || $("a").height !== H) { $("a").width = W; $("a").height = H; fitKey = ""; }
  fit();
  a.globalAlpha = 1;
  if (side) {
    a.drawImage(offA, 0, 0);
    b.drawImage(offB, 0, 0);
    if (!haveModel && model.say) badge(b, model.say);
  } else if (show.view === "top") {
    a.drawImage(offA, 0, 0);
    a.globalAlpha = show.opacity;
    a.drawImage(offB, 0, 0);
    a.globalAlpha = 1;
    if (!haveModel && model.say) badge(a, model.say);
  } else {
    const pa = offA.getContext("2d", { willReadFrequently: true }).getImageData(0, 0, W, H).data;
    const pb = offB.getContext("2d", { willReadFrequently: true }).getImageData(0, 0, W, H).data;
    const out = new Uint8ClampedArray(pa.length);
    for (let i = 0; i < pa.length; i += 4) {
      out[i] = Math.abs(pa[i] - pb[i]); out[i + 1] = Math.abs(pa[i + 1] - pb[i + 1]); out[i + 2] = Math.abs(pa[i + 2] - pb[i + 2]); out[i + 3] = 255;
    }
    a.putImageData(new ImageData(out, W, H), 0, 0);
    if (!haveModel && model.say) badge(a, model.say);
  }
  if (recording) badge(a, `RECORDING ${((player.frame(0) - recording.start) / FPS).toFixed(1)} S`);
}

function putOriginal(slot) {
  offA.getContext("2d").putImageData(new ImageData(player.rgba(slot), W, H), 0, 0);
}

// A clip ---------------------------------------------------------------

const SNAP_EVERY = 60;

/** A draws script's draws by the original's frame: [site, value] each. */
function readDraws(text) {
  const by = new Map();
  for (const l of text.split("\n")) {
    const m = /^(\d+)\s+(\S+)\s+(\d+)$/.exec(l.split("#")[0].trim());
    if (!m) continue;
    const f = Number(m[1]);
    if (!by.has(f)) by.set(f, []);
    by.get(f).push([m[2], Number(m[3])]);
  }
  return by;
}
/** A draws script's `soft SITE` lines. */
function softOf(text) {
  return text.split("\n").filter((l) => /^soft\s+\S+$/.test(l.trim())).map((l) => l.trim() + "\n").join("");
}

/** Shows clip `n` of `mechanic` at frame `f`: the original replayed in the
 * player's second console to the clip's start, every frame of the clip
 * played once to learn which frames read the pad, then both games stood at
 * `f`. */
async function openClip(n, mechanic, f) {
  if (recording) stopRecording(false);
  const key = `${mechanic ?? ""}/${n}`;
  // Any clip but a gate card's (judgeOn) plays on main's model.
  const back = cardOpening ? false : toMainModel();
  if (clip && clip.key === key) {
    seekClip(f);
    if (back) void loadModel();
    return;
  }
  const q = mechanic ? `&mechanic=${encodeURIComponent(mechanic)}` : "";
  const meta = await api(`api/screen/clip?n=${n}${q}`);
  if (mode === "clip" && clip) backToClipless();
  const r = await fetch(`api/clips/${n}.log?mechanic=${encodeURIComponent(meta.mechanic)}`);
  if (!r.ok) throw new Error(`clip ${n}: ${r.status}`);
  const log = await r.text();
  await showClipLog({ key, n, mechanic: meta.mechanic, work: null, name: `CLIP ${n} OF ${meta.mechanic.toUpperCase()}`, says: `clip ${n} of ${meta.mechanic}`, note: meta.note ?? "", start: meta.start, frames: meta.frames, log, draws: meta.draws ?? null, drawSites: meta.draw_sites ?? null }, f);
}

/** Shows the corpus clip a refine works on (api/screen/work-clip), looping
 * the window's frames [lo, hi] from the clip's start. */
async function openWorkClip(id, lo, hi, name) {
  // It loads the model again below, on main's unless a gate card opens it.
  if (!cardOpening) toMainModel();
  const meta = await api(`api/screen/work-clip?id=${encodeURIComponent(id)}`);
  if (mode === "clip" && clip) backToClipless();
  await showClipLog({ key: `work/${id}`, n: null, mechanic: null, work: id, name: name ?? `THE CLIP YOUR AGENT REFINES: ${id.toUpperCase()}`, says: `corpus clip ${id}`, note: meta.name ?? "", start: meta.start, frames: meta.frames, log: meta.log, draws: meta.draws ?? null, drawSites: meta.draw_sites ?? null }, lo);
  if (clip?.work === id) { clip.loop = [lo, Math.max(lo, hi)]; playClip(true); }
}

// A clip to judge on: a row asking the person may carry the clip that shows
// what they are to judge (gp mechanic queue --clip N --frames A-B --look
// THEME: row.clip, {n, mechanic?} or {id}, with from, to and look). A click
// on the row, on its item in YOUR TURN or on its card sets it up on the
// screens: the original beside the model, in that theme, at those frames
// (played in a loop, or stood still on one frame).

/** The words for a row's clip: "CLIP 1, FRAMES 200 TO 230, IN LAST-CALL". */
function judgeWords(r) {
  const c = r.clip;
  let s = c.id ? `CLIP ${c.id}` : c.mechanic && c.mechanic !== r.mechanic ? `CLIP ${c.n} OF ${c.mechanic}` : `CLIP ${c.n}`;
  if (typeof c.from === "number") s += c.to > c.from ? `, FRAMES ${c.from} TO ${c.to}` : `, FRAME ${c.from}`;
  if (c.look) s += c.look === "game" ? ", IN THE GAME'S PICTURES" : `, IN ${c.look}`;
  return s.toUpperCase();
}

/** Sets row `r`'s clip up on the screens for the person to judge. */
async function judgeOn(r) {
  const c = r?.clip;
  if (!c) return;
  if (recording || sending) { note("Send the recording first: RECORD (or R)."); return; }
  show.by = "person";
  setView("side");
  if (c.look && c.look !== show.pictures) setPictures(c.look);
  $("screens").scrollIntoView({ behavior: "smooth", block: "start" });
  const lo = typeof c.from === "number" ? c.from : 0;
  const hi = typeof c.to === "number" ? c.to : null;
  // A gate card's clip plays the card's change (r.ask, the card's id: the
  // model as its patch would leave it); a row's own clip plays main's.
  model.card = r.ask ?? null;
  const changed = useModel(r.ask ?? null);
  const loads = model.loads;
  note(`Setting up ${judgeWords(r).toLowerCase()} for ${r.mechanic}${r.ask ? ", the model after this change" : ""}.`, 60000);
  try {
    cardOpening = true;
    try {
      if (c.id) await openWorkClip(c.id, lo, hi ?? lo, `TO JUDGE FOR ${r.mechanic.toUpperCase()}: ${String(c.id).toUpperCase()}`);
      else {
        await openClip(c.n, c.mechanic ?? r.mechanic, lo);
        if (clip) $("clip-name").textContent = clipName(`TO JUDGE FOR ${r.mechanic.toUpperCase()}: CLIP ${c.n} OF ${String(c.mechanic ?? r.mechanic).toUpperCase()}`, clip.log);
      }
    } finally { cardOpening = false; }
    show.by = "person";
    if (!clip) return;
    // The same clip was up already (no load): the other model loads now.
    if (changed && model.loads === loads) void loadModel();
    beforeButton();
    seekClip(lo, true);
    if (hi !== null && hi > lo) { clip.loop = [lo, Math.min(hi, clip.frames - 1)]; playClip(true); }
    else { clip.loop = null; playClip(false); }
    document.body.dataset.judging = r.mechanic;
    note(r.ask
      ? `${r.mechanic}: ${judgeWords(r).toLowerCase()}, the original on the left, the model after this change on the right. BEFORE shows main's.`
      : `${r.mechanic}: ${judgeWords(r).toLowerCase()}, the original on the left. ${r.note ?? ""}`.trim(), 10000);
    drawClips();
    report();
  } catch (e) { fault(`Couldn't set up the clip for ${r.mechanic}: ${e.message ?? e}`, 8000, "LOOK AT IT on the row"); }
}

/** A LOOK AT IT button for row `r`'s clip; with `r.ask` (a gate card's),
 * the model beside the original is the card's change. */
function judgeButton(r) {
  const b = el("button", "judge", `LOOK AT IT: ${judgeWords(r)}`);
  b.type = "button";
  b.title = r.ask
    ? "Sets this clip up on the screens above: the original beside the model after this change, at the frames in question (BEFORE shows main's)"
    : "Sets this clip up on the screens above: the original beside the model, at the frames in question";
  b.addEventListener("click", (e) => { e.stopPropagation(); e.currentTarget.blur(); void judgeOn(r); });
  return b;
}

/** Whether row `r` asks the person to confirm a look a yes locks: asking
 * them, with a clip in the game's own pictures (crate::approved::confirmable). */
const confirmable = (r) => r?.status === "asking you" && !r.confirmed && r.clip?.look === "game"
  && (typeof r.clip.id === "string" || typeof r.clip.n === "number");

/** A CONFIRM button for row `r`, or null: the person's yes to the look,
 * which gp serve's runner locks (gp mechanic approve, through the model
 * gate) with no worker. */
function confirmButton(r) {
  if (!confirmable(r)) return null;
  const b = el("button", "judge confirm", "CONFIRM: IT LOOKS RIGHT");
  b.type = "button";
  b.title = "Say it looks right, so no later change can make it look worse";
  b.addEventListener("click", async (e) => {
    e.stopPropagation();
    const btn = e.currentTarget;
    btn.blur();
    btn.disabled = true;
    try {
      await api("api/confirm", { mechanic: r.mechanic });
      note(`${r.mechanic}: you said it looks right. Saving it now.`, 6000);
      turnDrawn = "";
      await refreshLists(true);
    } catch (err) {
      btn.disabled = false;
      fault(`Your yes didn't save: ${err.message ?? err}`, 8000);
    }
  });
  return b;
}

/** A clip's play from power-on on both screens, from `c.start` for
 * `c.frames`, stood at `f`. */
async function showClipLog(c, f) {
  const { log } = c;
  const { head, frames } = splitLog(log);
  const columns = /^columns /m.test(head);
  const start = c.start ?? 0;
  const length = Math.max(1, Math.min(c.frames ?? frames.length - start, frames.length - start));
  const pads = frames.slice(start, start + length).map((l) => padOf(l, columns));
  mode = "clip";
  model.on = false;
  model.say = "THE MODEL IS STARTING";
  const { n, mechanic } = c;
  note(`Loading ${c.says}.`, 60000);
  // The clip plays on the set it was recorded on.
  player.replay(1, await romFor(log), log);
  if (mode !== "clip") return;
  for (let at = player.frame(1); at < start;) {
    player.runTo(1, Math.min(start, at + 600));
    if (player.frame(1) === at) break;
    at = player.frame(1);
    line(`Loading ${c.says}: ${Math.round((100 * at) / Math.max(1, start))}% of the way to its start.`);
    await new Promise((res) => setTimeout(res, 0));
    // The person went back to playing while it loaded.
    if (mode !== "clip") return;
  }
  const snaps = new Map([[0, player.save(1)]]);
  // The original's draws over the clip: those kept with it when it was
  // recorded, else recorded from the replay at the mapping's sites.
  let clipDraws = c.draws ? readDraws(c.draws) : null;
  const sites = c.drawSites ?? draws.sites;
  const recordHere = !clipDraws && !!sites && player.recordsDraws();
  if (recordHere) { player.drawSites(1, sites); clipDraws = new Map(); }
  const polled = [], upTo = [], frameDraws = [];
  for (let i = 0; i < length; i++) {
    const read = player.step(1, pads[i]);
    if (read) polled.push(pads[i]);
    upTo.push(polled.length);
    const f = player.frame(1);
    frameDraws.push([f, read]);
    if (recordHere) {
      const ds = player.draws(1);
      if (ds.length) clipDraws.set(f, ds);
    }
    if ((i + 1) % SNAP_EVERY === 0) snaps.set(i + 1, player.save(1));
  }
  if (recordHere) player.drawSites(1, "");
  clip = {
    key: c.key, n, mechanic, work: c.work, loop: null, note: c.note, start, frames: length, log, pads, polled, upTo, snaps, f: 0, pos: length, playing: false, path: null,
    draws: clipDraws, frameDraws, soft: c.draws ? softOf(c.draws) : softLines(sites),
  };
  $("clip-scrub").max = String(length - 1);
  $("clip-name").textContent = clipName(c.name, log);
  $("clipbar").hidden = false;
  beforeButton();
  $("record").disabled = true;
  seekClip(f ?? 0, true);
  const says = c.says[0].toUpperCase() + c.says.slice(1);
  note(`${says}${clip.note ? `: ${clip.note}` : ""}.`, 6000);
  await loadModel();
}

/** Both games at frame `f` of the clip: the original from the snapshot at
 * or before it, the model seeked to the steps it had taken by then. */
function seekClip(f, quiet) {
  const c = clip;
  if (!c) return;
  f = Math.max(0, Math.min(c.frames - 1, Math.round(Number(f) || 0)));
  const want = f + 1;
  if (c.pos > want || want - c.pos > SNAP_EVERY) {
    const from = Math.floor(want / SNAP_EVERY) * SNAP_EVERY;
    const at = [...c.snaps.keys()].filter((k) => k <= from).reduce((x, y) => Math.max(x, y), 0);
    player.restore(1, c.snaps.get(at));
    c.pos = at;
  }
  while (c.pos < want) { player.step(1, c.pads[c.pos]); c.pos++; }
  c.f = f;
  putOriginal(1);
  if (model.on) modelPost({ gp: "seek", frame: c.upTo[f] ?? 0 });
  $("clip-scrub").value = String(f);
  $("clip-at").textContent = `frame ${f} of ${c.frames - 1}`;
  if (!quiet) report();
}

function playClip(on) {
  if (!clip) return;
  clip.playing = on;
  $("clip-play").textContent = on ? "STAND STILL" : "PLAY";
  report();
}

/** Forgets the clip shown, before another loads. */
function backToClipless() {
  clip = null;
  model.on = false;
}

/** Back to playing the game live, where it was. */
function backToLive() {
  lastPlay = performance.now();
  if (mode === "live") return;
  mode = "live";
  clip = null;
  // Live play is main's model.
  toMainModel();
  $("clipbar").hidden = true;
  $("record").disabled = show.view === "solo";
  model.say = "THE MODEL IS STARTING";
  void loadModel("Playing the game live. The model follows your presses.");
  report();
}

// Following the work ---------------------------------------------------
//
// While a refine runs (gp model refine writes screen-work.json; api/screen
// gives it as `work`), the screens show the clip it works on, the original
// beside the model, looping the frames of the window it fits. The person
// comes first: a recording is never cut off, a game button while it
// follows goes back to playing, and it follows again only after
// QUIET_MS with no press. A clip the person opened stays until they go
// back to playing; one the agent asked for with screen_show holds for
// AGENT_HOLD_MS. FOLLOW THE WORK turns it off and on.

const QUIET_MS = 20000;
const AGENT_HOLD_MS = 60000;
let lastPlay = -QUIET_MS; // the person's last game button
let agentHeldUntil = 0;
const follow = { on: true, work: null, opening: false };
try { follow.on = localStorage.getItem("gp-screen-follow") !== "off"; } catch (e) { /* on */ }

/** The window's frames from the clip's start: [lo, hi]. */
function workRange(w) {
  const last = Math.max(0, (w.frames ?? 1) - 1);
  const at = (x, d) => (typeof x === "number" ? Math.max(0, Math.min(last, x - (w.start ?? 0))) : d);
  const lo = at(w.from, 0);
  return [lo, Math.max(lo, at(w.to, last))];
}

/** Why the screen isn't following the work now, or null when it may. */
function holdBy() {
  if (!follow.on) return "off";
  if (recording || sending) return "recording";
  if (mode === "live" && performance.now() - lastPlay < QUIET_MS) return "playing";
  if (mode === "clip" && clip?.work && show.by === "person") return "looking";
  if (mode === "clip" && clip && !clip.work) {
    if (show.by === "person") return "your clip";
    if (performance.now() < agentHeldUntil) return "agent's clip";
  }
  return null;
}

/** The work from api/screen: open its clip, or move to its new window. */
async function followWork(w) {
  follow.work = w && w.live ? w : null;
  drawFollow();
  drawNowWork();
  const wk = follow.work;
  if (!wk || !wk.clip || follow.opening || holdBy()) return;
  const [lo, hi] = workRange(wk);
  if (mode === "clip" && clip?.work === wk.clip) {
    if (!clip.loop || clip.loop[0] !== lo || clip.loop[1] !== hi) {
      clip.loop = [lo, hi];
      seekClip(lo, true);
      playClip(true);
    }
    return;
  }
  follow.opening = true;
  try {
    show.by = "work";
    await openWorkClip(wk.clip, lo, hi);
    drawClips();
    report();
  } catch (e) {
    fault(`Couldn't show the clip your agent refines (${wk.clip}): ${e.message ?? e}`, 8000);
    if (mode === "clip" && !clip) backToLive();
  } finally {
    follow.opening = false;
  }
}

/** The line over the screens: which of the two the screen is doing, the
 * work or the person, and how it changes. */
function drawFollow() {
  const b = $("follow");
  const wk = follow.work;
  const held = holdBy();
  let text, cls;
  if (mode === "clip" && clip?.work) {
    const [lo, hi] = clip.loop ?? [0, clip.frames - 1];
    cls = "following";
    text = `FOLLOWING YOUR AGENT: ${clip.work}, frames ${lo} to ${hi}${wk?.says ? `. ${wk.says}` : wk ? "" : ". The refine has ended"}.${held === "looking" ? " Held where you stepped it." : ""} A game button goes back to playing.`;
  } else if (held === "recording") {
    cls = "you"; text = "RECORDING: the screen stays with you until you send it.";
  } else if (held === "playing") {
    const s = Math.ceil((QUIET_MS - (performance.now() - lastPlay)) / 1000);
    cls = "you"; text = wk ? `YOU ARE PLAYING. Your agent refines ${wk.clip}: the screen follows it after ${s} s with no press.` : "YOU ARE PLAYING.";
  } else if (held === "your clip" || held === "agent's clip") {
    cls = "you"; text = `SHOWING ${held === "your clip" ? "YOUR CHOICE" : "WHAT YOUR AGENT ASKED"}${wk ? `. Your agent refines ${wk.clip}: BACK TO PLAYING, then ${QUIET_MS / 1000} s with no press, follows it` : ""}.`;
  } else if (held === "off") {
    cls = "off"; text = wk ? `FOLLOW IS OFF. Your agent refines ${wk.clip}.` : "FOLLOW IS OFF.";
  } else {
    cls = "idle"; text = wk ? `Opening ${wk.clip}, the clip your agent refines.` : "NO REFINE RUNNING: the screen is yours. It follows your agent's next refine when you aren't playing.";
  }
  if (b.textContent !== text) b.textContent = text;
  b.className = cls;
  const t = $("follow-on");
  t.textContent = follow.on ? "FOLLOW THE WORK: ON" : "FOLLOW THE WORK: OFF";
  t.classList.toggle("on", follow.on);
}

function setFollow(on) {
  follow.on = on;
  try { localStorage.setItem("gp-screen-follow", on ? "on" : "off"); } catch (e) { /* not kept */ }
  if (!on && mode === "clip" && clip?.work) backToLive();
  drawFollow();
}

/** A game button while the screen follows the work: back to playing. */
function personPressed() {
  lastPlay = performance.now();
  if (mode === "clip" && clip?.work) { show.by = "person"; backToLive(); drawClips(); }
}

// Recording ------------------------------------------------------------

let warming = null;
// The last recording saved ({mechanic, n}): said beside RECORD and marked in
// the clips list until the next recording starts.
let saved = null;
function startRecording() {
  if (mode !== "live" || sending) return;
  if (show.view === "solo") { note("RECORD records the original: pick another view to record."); return; }
  recording = { start: player.frame(0) };
  saved = null;
  drawClips();
  clipFor = session?.mechanic ?? null;
  markTarget();
  $("record").classList.add("on");
  $("record").textContent = "STOP AND SEND";
  $("record-seal").hidden = false;
  // The play so far goes up as the recording starts, so sending later
  // waits on no replay (app.js's warmPath).
  warming = sendPath(player.inputLog(0), recording.start).catch(() => undefined);
  note(`Recording into ${clipFor ?? "the open row"}. Press RECORD again (or R) to send it to your agent.`, 3000);
}

/** Ends the recording; with `send`, hands the clip to the open mechanic
 * as CLIPS's hand-off does: the play as a path, the clip on it (POST
 * api/clips), then a turn in the talk naming it (POST api/talk). With
 * `seal`, it is a held-out run instead (docs/sealed.md): only the play up
 * to its start goes to the shared paths, the run itself to the sealed
 * store, no row and no agent hears of it, and the scoreboard scores it. */
async function stopRecording(send = true, seal = false) {
  const r = recording;
  recording = null;
  $("record").classList.remove("on");
  $("record").textContent = "RECORD";
  $("record-seal").hidden = true;
  if (!r || !send) { markTarget(); return; }
  const log = player.inputLog(0);
  r.end = player.frame(0);
  const frames = r.end - r.start;
  if (frames < 10) { note("Too short to keep: under 10 frames."); return; }
  if (seal) { await sealRecording(r, log); return; }
  sending = true;
  $("record").disabled = true;
  const mechanic = clipFor ?? session?.mechanic ?? null;
  try {
    const { head, frames: all } = splitLog(log);
    if (warming) { line("Sending your play."); await warming; warming = null; }
    line(`Sending your play (${Math.max(1, Math.round(log.length / 1024))} KB).`);
    const path = await sendPath(log);
    line("Saving the clip.");
    const inputs = head + all.slice(r.start).join("\n") + "\n";
    // The draws the original made over the clip, when they were recorded
    // from before it started: kept with it, so it replays the same.
    const kept = drawsOver(r);
    const { n, again } = await api("api/clips", { path, start: r.start, inputs, note: "", mechanic, draws: kept });
    await api("api/talk", { text: `Here is clip ${n}.`, clips: [n], mechanic });
    saved = { mechanic, n };
    note(again
      ? `Already saved: this is clip ${n} of ${mechanic}, under ${String(mechanic).toUpperCase()} in CLIPS below.`
      : `Saved as clip ${n} of ${mechanic}: it is under ${String(mechanic).toUpperCase()} in CLIPS below, and your agent has it.`, 15000);
    await refreshLists(true);
  } catch (e) {
    fault(`The clip didn't reach your agent: ${e.message ?? e}`, 10000);
  } finally {
    sending = false;
    clipFor = null;
    markTarget();
    $("record").disabled = mode !== "live" || show.view === "solo";
  }
}

/** The draws the original made over a recording, when they were recorded
 * from before it started, as a clip keeps them. */
function drawsOver(r) {
  if (draws.from === null || draws.from > r.start) return undefined;
  return "gp-draws 0\n" + softLines(draws.sites) + draws.log.filter(([f]) => f > r.start && f <= r.end).map(([f, site, v]) => `${f} ${site} ${v}\n`).join("");
}

/** STOP AND SEAL: the recording kept as a held-out run. The person names
 * the part it plays (level 1, say), which the scoreboard shows it as. */
async function sealRecording(r, log) {
  const name = (window.prompt("A held-out run no worker sees. Which part does it play (for example: level 1)?", "") ?? "").trim();
  sending = true;
  $("record").disabled = true;
  try {
    const { head, frames: all } = splitLog(log);
    if (warming) { line("Sealing your run."); await warming; warming = null; }
    line("Sealing your run.");
    const path = await sendPath(head + all.slice(0, r.start).map((l) => l + "\n").join(""));
    const inputs = head + all.slice(r.start).join("\n") + "\n";
    const { n, again } = await api("api/clips", { path, start: r.start, inputs, note: name, sealed: true, draws: drawsOver(r) });
    note(again
      ? `Sealed already as held-out run ${n}: no worker sees it.`
      : `Sealed as held-out run ${n}${name ? ` (${name})` : ""}: no worker sees it, and the scoreboard scores it on the model within a few minutes.`, 15000);
  } catch (e) {
    fault(`The run wasn't sealed: ${e.message ?? e}`, 10000);
  } finally {
    sending = false;
    clipFor = null;
    markTarget();
    $("record").disabled = mode !== "live" || show.view === "solo";
  }
}

// The quest log and the clips --------------------------------------------

// What the queue's statuses show as (app.js's LABELS).
const LABELS = { done: "DONE", "asking you": "YOUR MOVE", "agent working": "WORKING ON IT", "in line": "IN LINE", next: "NEXT" };
let rows = []; // the quest log, as api/session gives it
let rowsRead = false; // rows came from the server once
let clipsBy = []; // [{mechanic, clips: [{n, frames, by, note}]}]

const el = (tag, cls, text) => {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
};

/** The row a recording goes to, outlined, and said beside RECORD. */
function markTarget() {
  const to = recording || sending ? clipFor : session?.mechanic ?? null;
  for (const li of $("queue").querySelectorAll("li[data-mechanic]")) {
    const on = li.dataset.mechanic === to;
    li.classList.toggle("target", on);
    li.classList.toggle("recording", on && (!!recording || sending));
    const here = li.querySelector(".qhere");
    if (here) here.hidden = on;
    li.querySelector(".qto").textContent = on ?(recording ? "RECORDING INTO THIS ROW" : sending ? "SENDING TO THIS ROW" : "RECORD GOES HERE") : "";
  }
  const done = !recording && !sending && saved;
  $("rec-to").textContent = done
    ? `SAVED: CLIP ${saved.n} OF ${String(saved.mechanic).toUpperCase()}`
    : to ? `${recording ? "INTO" : "TO"}: ${to.toUpperCase()}` : "";
  $("rec-to").title = done
    ? `Your last recording is clip ${saved.n} of ${saved.mechanic}, in CLIPS below; the next recording goes to ${to ?? "the open row"}`
    : "The quest-log row a recording goes to: RECORD INTO THIS ROW on another row changes it";
  $("rec-to").classList.toggle("on", !!recording);
  $("rec-to").classList.toggle("saved", !!done);
}

/** Talk turns of a row (from api/screen/clips), oldest first. */
function talkOf(m) {
  return clipsBy.find((g) => g.mechanic === m)?.talk ?? [];
}

/** The person's words with what they were watching when they wrote them:
 * the clip on the screens and its frame, so "wrong here" says where. */
function withSeen(text) {
  if (mode !== "clip" || !clip) return text;
  const what = clip.work ? `corpus clip ${clip.work}` : `clip ${clip.n}${clip.mechanic ? ` of ${clip.mechanic}` : ""}`;
  return `${text} (written while watching ${what}, frame ${clip.f}${model.ask ? ", the card's change" : ""})`;
}

/** A COMMENT button that opens a line under `host`; what is typed goes to
 * the row's talk as the person's turn (POST api/talk), naming `clips`. */
function commentBox(host, mechanic, clips, label, word = "COMMENT") {
  const b = el("button", "comment-open", word);
  b.type = "button";
  b.title = label;
  b.addEventListener("click", (e) => {
    e.stopPropagation();
    e.currentTarget.blur();
    if (host.querySelector("form.comment")) return;
    const f = el("form", "comment");
    f.autocomplete = "off";
    const t = el("input", "comment-text");
    t.placeholder = label;
    const go = el("button", "", "SEND");
    go.type = "submit";
    f.append(t, go);
    for (const x of [f, t, go]) x.addEventListener("click", (ev) => ev.stopPropagation());
    t.addEventListener("keydown", (ev) => { ev.stopPropagation(); if (ev.code === "Escape") f.remove(); });
    f.addEventListener("submit", async (ev) => {
      ev.preventDefault();
      const text = t.value.trim();
      if (!text) return;
      go.disabled = true;
      try {
        await api("api/talk", { text: withSeen(text), clips, mechanic });
        f.remove();
        note(clips.length ? `Your comment on clip ${clips[0]} of ${mechanic} went to your agent.` : `Your comment on ${mechanic} went to your agent.`, 5000);
        await refreshLists(true);
      } catch (err) {
        go.disabled = false;
        fault(`Your comment didn't save: ${err.message ?? err}`, 8000);
      }
    });
    host.append(f);
    t.focus();
  });
  return b;
}

/** The last few turns of a row's talk, or those naming clip `n`. */
function talkLines(turns, n) {
  const box = el("div", "talk");
  const shown = turns.filter((t) => n === undefined || (t.clips ?? []).includes(n)).slice(-3);
  for (const t of shown) {
    const who = t.by === "player" ? "YOU" : "AGENT";
    box.append(el("div", `talk-line talk-${t.by === "player" ? "you" : "agent"}`, `${who}: ${t.text ?? ""}`));
  }
  return box;
}

/** A press held in the quest log or the clips: the lists aren't drawn
 * again until it is let go, since a row rebuilt between the press and the
 * release takes no click (the refresh every 3 s rebuilds them whenever a
 * status line changes). */
let pressing = false;
function holdWhilePressed(box) {
  box.addEventListener("pointerdown", () => { pressing = true; });
}
for (const ev of ["pointerup", "pointercancel"]) {
  addEventListener(ev, () => {
    if (!pressing) return;
    // After the click the release makes, which is dispatched next.
    setTimeout(() => { pressing = false; drawQueue(); drawClips(); }, 0);
  });
}

// The quest log's groups, in the order they show: what waits for the person
// first. A row's group is said in words in its heading and on its badge.
// IN LINE (parked behind a card or another row) and DONE stay folded until
// opened.
const GROUPS = [
  ["you", "NEEDS YOU"],
  ["working", "AGENT WORKING"],
  ["next", "UP NEXT"],
  ["inline", "IN LINE"],
  ["done", "DONE"],
];
const FOLDED = { inline: "Rows parked behind a card or another row", done: "Finished rows, archived here" };
// Whether a row is being worked: marked agent working, or a live worker of
// the runner's is on it while its status stays next (the runner's worker
// leaves the row's status as it was).
function worked(r) {
  return r.status === "agent working" || (r.status !== "done" && r.status !== "asking you" && !!workerOn(r));
}
// A row's group and badge come from its status and the runner's live
// workers; the triage's mark (NEEDS YOU, THE TOOLS TRY) only counts on a
// row that isn't done or worked.
function groupOf(r) {
  if (r.status === "done") return "done";
  if (r.status === "asking you" || (r.triage?.by === "you" && !worked(r))) return "you";
  if (worked(r)) return "working";
  if (r.status === "in line") return "inline";
  return "next";
}
function badgeOf(r) {
  if (worked(r)) return LABELS["agent working"];
  if (r.status !== "done") {
    if (r.triage?.by === "you") return "NEEDS YOU";
    if (r.triage?.by === "tools") return "THE TOOLS TRY";
  }
  return LABELS[r.status] ?? String(r.status ?? "").toUpperCase();
}

// The words in a note that are for the tools, not the person: RAM tests,
// addresses ($00A3, 0x1F2), hashes and commits. A sentence holding one is
// folded under DETAILS.
const TECH = /\bRAM\b|\$[0-9a-f]{2,}\b|\b0x[0-9a-f]+\b|\b(?=[0-9a-f]*\d)(?=[0-9a-f]*[a-f])[0-9a-f]{7,64}\b|\bsha\d*\b|\bblake3\b/i;
function splitNote(text) {
  const plain = [], tech = [];
  for (const s of String(text ?? "").split(/(?<=[.;])\s+/)) {
    if (!s.trim()) continue;
    (TECH.test(s) ? tech : plain).push(s);
  }
  return { plain: plain.join(" "), tech: tech.join(" ") };
}

/** One note of a row, with who wrote it, its tools' words folded. */
function noteLine(who, text, cls) {
  const box = el("div", `qnote ${cls ?? ""}`);
  const { plain, tech } = splitNote(text);
  box.append(el("span", "qwho", `${who}: `), plain || (tech ? "(details only)" : ""));
  if (tech) {
    const d = el("details", "qtech");
    d.append(el("summary", "", "DETAILS"), el("span", "", tech));
    d.addEventListener("click", (e) => e.stopPropagation());
    box.append(d);
  }
  return box;
}

function drawQueue() {
  const q = $("queue");
  if (pressing) return;
  const cardClips = asksNow.filter((a) => a.clip && !a.ran).map((a) => [askRow(a), a.clip]);
  const shown = rows.filter((r) => r.mechanic && !String(r.mechanic).startsWith("sealed"));
  const live = shown.map((r) => workerOn(r)?.worker ?? null);
  const key = JSON.stringify([session?.mechanic, rows, live, clipsBy.map((g) => [g.mechanic, (g.talk ?? []).length]), cardClips]);
  drawAgentsHead();
  if (q.dataset.key === key) { markTarget(); return; }
  // Held still while the person reads or types in it.
  if (agentsHeld()) { $("agents-held").hidden = false; return; }
  $("agents-held").hidden = true;
  // Cards opened stay open when the log is drawn again.
  const opened = new Set([...q.querySelectorAll("li.expanded[data-mechanic]")].map((li) => li.dataset.mechanic));
  q.dataset.key = key;
  q.textContent = "";
  for (const g of Object.keys(FOLDED)) {
    try { q.classList.toggle(`${g}-open`, localStorage.getItem(`gp-${g}-open`) === "1"); } catch (e) { /* no storage */ }
  }
  for (const [g, title] of GROUPS) {
    const of = shown.filter((r) => groupOf(r) === g);
    if (!of.length) continue;
    const head = el("li", `qgroup qgroup-${g}${FOLDED[g] ? " qgroup-folded" : ""}`, `${title} (${of.length})`);
    if (FOLDED[g]) {
      head.title = FOLDED[g];
      head.addEventListener("click", () => {
        q.classList.toggle(`${g}-open`);
        try { localStorage.setItem(`gp-${g}-open`, q.classList.contains(`${g}-open`) ? "1" : ""); } catch (e) { /* no storage */ }
      });
    }
    q.append(head);
    for (const r of of) q.append(queueRow(r, g, opened.has(r.mechanic)));
  }
  if (!q.children.length) q.append(el("li", "", "No rows yet: tell your agent what to put in the quest log."));
  markTarget();
}

/** A card's short title: a row's name in words, its trailing hash left
 * off (trouble-...-9e73e7). */
function cardTitle(name) {
  return String(name ?? "").replace(/-[0-9a-f]{6,}$/, "").replace(/[-_]+/g, " ");
}

/** A card's one line of what: the first sentence of its words, the loop's
 * "Proposed by the loop:" left off. */
function cardLine(text) {
  return oneSentence(String(text ?? "").replace(/^\s*Proposed by the loop:\s*/i, ""), 120);
}

/** A row's one line for a person: its `plain` sentence (gp mechanic queue
 * or propose --plain), else one made from the person's words or the note,
 * the loop's "Proposed by the loop:" and "(from ...)" and every sentence
 * for the tools (addresses, hashes, RAM tests) left off. The note itself
 * stays in the opened card. */
function plainLine(r) {
  if (r.plain) return oneSentence(r.plain, 160);
  const src = String(r.asked || r.note || r.target_says || "")
    .replace(/^\s*Proposed by the loop:\s*/i, "")
    .replace(/\s*\(from [^()]*\)\.?\s*$/, "");
  return cardLine(splitNote(src).plain);
}

/** A quest-log row as a small card: its badge, a short title, who is on it
 * and one line of what; a click shows the rest (its notes, target, talk,
 * RECORD INTO THIS ROW and COMMENT). Only RECORD INTO THIS ROW moves where
 * the next recording goes. LOOK AT IT and CONFIRM stay on the card. */
function queueRow(r, g, open) {
  const li = el("li", `qcard qrow-${g}`);
  li.dataset.mechanic = r.mechanic;
  const status = g === "working" ? "agent working" : String(r.status ?? "");
  const head = el("div", "qhead");
  const name = el("span", "qname", cardTitle(r.mechanic));
  name.title = r.mechanic;
  head.append(el("span", `st st-${status.replace(/ /g, "-")}`, badgeOf(r)), name);
  if (g === "working") {
    li.classList.add("active");
    const who = r.worker ?? workerOn(r)?.worker;
    head.append(el("span", "qworker", who ? String(who).toUpperCase() : "AGENT"));
  }
  head.append(el("span", "qto", ""));
  li.append(head);
  const what = plainLine(r);
  if (what) li.append(el("div", "qline", what));
  const judge = rowToJudge(r);
  const yes = confirmButton(r);
  // COMMENT is on the card's face, as it was before the cards: the person
  // can say something about any row without opening it.
  const face = el("div", "qacts qface");
  if (judge) face.append(judgeButton(judge));
  if (yes) face.append(yes);
  face.append(commentBox(face, r.mechanic, [], `A comment on ${r.mechanic} for your agent`));
  li.append(face);
  const more = el("div", "qmore");
  more.append(el("div", "qnote qfull", r.mechanic));
  if (r.scores) more.append(noteLine("SCORES", r.scores, "qscore"));
  // Its target closes it when the scoreboard's measure gets there.
  if (r.target_says) more.append(noteLine("TARGET", r.target_says, "qscore"));
  else if (r.measure_needed) more.append(noteLine("NO MEASURE YET", `what would measure it: ${r.measure_needed}`, "qscore"));
  // Parked on another row: the runner moves it to next when that one lands.
  if (r.waits_on && r.status === "in line") more.append(noteLine("WAITS ON", `${r.waits_on}: the loop takes it up when that lands or is done`, "qscore"));
  // Who wrote it: the person's words from the page (by: person) stay theirs
  // (asked) once the agent's note takes the row over.
  if (r.asked) more.append(noteLine("YOU ASKED", r.asked));
  if (r.note) more.append(noteLine(r.by === "person" && !r.asked ? "YOU WROTE" : "AGENT WROTE", r.note));
  const turns = talkOf(r.mechanic);
  if (turns.length) more.append(talkLines(turns));
  const acts = el("div", "qacts");
  const here = el("button", "qhere", "RECORD INTO THIS ROW");
  here.type = "button";
  here.title = "Recordings go to this row from now on";
  here.addEventListener("click", (e) => { e.stopPropagation(); e.currentTarget.blur(); void pickRow(r.mechanic); });
  acts.append(here);
  more.append(acts);
  // The person's words on the row, open once the card is: they go to its
  // talk as the person's turn, which its worker hears.
  more.append(answerBox(r.mechanic, [], {
    kind: "row",
    placeholder: `Your words on ${cardTitle(r.mechanic)}, for its agent`,
    word: "SEND",
    sent: `Your words went to ${cardTitle(r.mechanic)}'s talk.`,
  }));
  li.append(more);
  if (r.mechanic === session?.mechanic) li.classList.add("open");
  li.classList.toggle("expanded", open);
  for (const d of li.querySelectorAll("details")) d.open = open;
  li.title = r.clip
    ? `Click: set up ${judgeWords(r).toLowerCase()} on the screens above, and show or hide the rest`
    : "Click: show or hide the rest";
  li.addEventListener("click", (e) => {
    if (e.target.closest("form, button, summary")) return;
    const to = !li.classList.contains("expanded");
    li.classList.toggle("expanded", to);
    for (const d of li.querySelectorAll("details")) d.open = to;
    // A row with a clip to judge on sets it up on the screens.
    if (r.clip && to) void judgeOn(r);
  });
  return li;
}

function drawClips() {
  const box = $("clips");
  if (pressing || agentsHeld()) return;
  const shown = clip ? `${clip.mechanic}/${clip.n}` : "";
  const key = JSON.stringify([clipsBy, shown, saved]);  // talk included
  if (box.dataset.key === key) return;
  box.dataset.key = key;
  box.textContent = "";
  const fps = FPS || 60;
  for (const g of clipsBy) {
    if (!g.clips.length) continue;
    box.append(el("h3", "", g.mechanic.toUpperCase()));
    const ul = el("ul");
    for (const c of g.clips) {
      const just = saved && saved.mechanic === g.mechanic && Number(saved.n) === Number(c.n);
      const li = el("li", [`${g.mechanic}/${c.n}` === shown ? "on" : "", just ? "saved" : ""].join(" ").trim());
      if (just) li.append(el("span", "csaved", "JUST SAVED"));
      li.append(el("span", "cn", `CLIP ${c.n}`), `${((c.frames ?? 0) / fps).toFixed(1)} s `, el("span", "cby", `by ${c.by ?? "?"}${c.note ? `: ${c.note}` : ""}`));
      li.append(commentBox(li, g.mechanic, [c.n], `A comment on clip ${c.n} of ${g.mechanic}`));
      const said = (g.talk ?? []).filter((t) => (t.clips ?? []).includes(c.n) && !/^Here is clip \d+\.$/.test(t.text ?? ""));
      if (said.length) li.append(talkLines(said, c.n));
      li.title = "Show it on both screens: Left and Right step, Space plays, Esc goes back to playing";
      li.addEventListener("click", () => {
        show.by = "person";
        // Marked at once, and the screens it shows on brought into view.
        for (const o of box.querySelectorAll("li.on")) o.classList.remove("on");
        li.classList.add("on");
        $("screens").scrollIntoView({ behavior: "smooth", block: "start" });
        note(`Opening clip ${c.n} of ${g.mechanic}.`, 60000);
        void openClip(c.n, g.mechanic, 0).then(() => { drawClips(); report(); }).catch((e) => fault(`Couldn't open clip ${c.n}: ${e.message ?? e}`, 8000));
      });
      ul.append(li);
    }
    box.append(ul);
  }
  if (!box.children.length) box.append(el("p", "empty", "No clips yet. RECORD makes one; your agent's clips show here too."));
}

/** The quest log at row `m`: on the screen page, the loop's page opened
 * there; on the loop's page, the row opened and brought into view. */
function toQuestLog(m) {
  if (!LOOP_PAGE) { location.href = loopHref(m); return; }
  openAgents(true);
  if (m) void pickRow(m);
  $("quest").scrollIntoView({ behavior: "smooth", block: "start" });
}

/** A row's RECORD INTO THIS ROW: while recording (or sending) the clip goes
 * there; otherwise the row opens (api/pick), and the next recording goes to
 * it. */
async function pickRow(m) {
  if (sending) { note(`This clip goes to ${clipFor}. Wait for it to send.`); return; }
  if (recording) { clipFor = m; markTarget(); note(`This recording goes to ${m}.`); return; }
  if (m === session?.mechanic) { note(`Recordings go to ${m}.`); return; }
  // Marked at once; the server's answer then draws it for good.
  for (const li of $("queue").querySelectorAll("li[data-mechanic]")) li.classList.toggle("open", li.dataset.mechanic === m);
  note(`Opening ${m}.`);
  try {
    await api("api/pick", { mechanic: m });
    await refreshLists(true);
    note(`Recordings go to ${m} now.`);
  } catch (e) { fault(`Couldn't open ${m}: ${e.message ?? e}`, 8000); }
}

/** The quest log and the clips, again from the server. */
let listing = false;
async function refreshLists(force) {
  if (listing && !force) return;
  listing = true;
  try {
    const [s, c] = await Promise.all([api("api/session"), api("api/screen/clips")]);
    session = s;
    const noAgent = document.getElementById("no-agent");
    noAgent.hidden = !s.no_agent;
    noAgent.textContent = s.no_agent ?? "";
    rows = s.queue ?? [];
    rowsRead = true;
    clipsBy = c.mechanics ?? [];
    drawQueue();
    drawClips();
    drawNowWork();
    drawYourTurn();
    drawProgress();
    showNamedClip();
  } catch (e) { /* the next look */ }
  finally { listing = false; }
}

/** The box's words as a new quest-log row (POST api/screen/row). */
async function newRow() {
  const t = $("tell-text").value.trim();
  if (!t) { note("Say what the item is first."); return; }
  $("tell-row").disabled = true;
  try {
    const { mechanic } = await api("api/screen/row", { text: t });
    $("tell-text").value = "";
    $("tell-text").blur();
    note(`New item in the quest log: ${mechanic}.`, 6000);
    await refreshLists(true);
  } catch (err) { fault(`The item didn't save: ${err.message ?? err}`, 8000); }
  finally { $("tell-row").disabled = false; }
}

/** The box under the quest log: a turn to the open row's agent. */
async function tell(e) {
  e.preventDefault();
  const t = $("tell-text").value.trim();
  if (!t) return;
  $("tell-send").disabled = true;
  try {
    await api("api/talk", { text: t, clips: [], mechanic: session?.mechanic ?? null });
    $("tell-text").value = "";
    $("tell-text").blur();
    note("Sent to your agent.");
  } catch (err) { fault(`Your agent didn't get it: ${err.message ?? err}`, 8000); }
  finally { $("tell-send").disabled = false; }
}

// What the agent waits for the person's yes on -------------------------
//
// The asks gp_run left (gp_mcp::asks, GET /api/asks), as cards in YOUR
// TURN, the strip over the window's bottom right corner (out of the
// page's flow, so the screens never move): one line a card, its question
// and YES and NO (POST /api/asks/ID); a click on the question opens what
// it does, the command and its message. A yes'd card leaves YOUR TURN for
// CARDS YOU ANSWERED under WHAT THE AGENTS ARE DOING (#asks-said): one
// line in the runner's turn (RUNNING NOW, WAITS n OF m), and one the
// runner finished says LANDED, DONE or FAILED with why, then leaves a
// minute later.

/** How long a card the runner finished stays on the strip. */
const ASK_DONE_SHOWS_MS = 60 * 1000;
const asksOpen = new Set();
// The cards whose box for words is open, and what is typed in each, kept
// across redraws: the words go with YES or NO (POST /api/asks/ID, words)
// to the card's row, as the person's turn.
const asksWording = new Set();
const askWords = new Map();
let asksDrawn = "";
let asksShown = 0; // the yes cards in YOUR TURN now
let asksNow = []; // those cards, as /api/asks last gave them

/** The quest-log row card `a` is for: the server's `row` (GAME/ROW, from
 * the step's worker record, even after the step closed), else its open
 * step's row:GAME/ROW; null for a card raised outside the loop. */
function askRow(a) {
  if (a.row) return a.row.split("/").slice(1).join("/");
  const step = a.step ? openSteps.find((x) => x.id === a.step) : null;
  return (step?.touches ?? []).find((t) => t.startsWith("row:"))?.split("/").slice(1).join("/") ?? null;
}

/** The card whose change a LOOK AT IT on card `a` plays (its id), when it
 * lands a change to a game (a model gate's, ask_look.rs); null for any
 * other card, whose clip plays main's model. */
function cardAsk(a) {
  return a?.command === "model gate" && !a.ran ? a.id : null;
}

/** Row `r` with the clip to judge it on: its own (on main's model), else
 * that of an open card for it (a gate's clip and frames, on the card's
 * change), else null. */
function rowToJudge(r) {
  if (r.clip) return r;
  const a = asksNow.find((x) => x.clip && !x.ran && askRow(x) === r.mechanic);
  return a ? { ...r, clip: a.clip, ask: cardAsk(a) } : null;
}
async function refreshAsks() {
  let list = [];
  const now = Date.now();
  try {
    const r = await fetch("/api/asks");
    if (r.ok) list = ((await r.json()).asks ?? []).filter((a) => a.ran
      ? a.ran.state === "running" || now - (a.ran.done_ms ?? a.ran.at_ms ?? 0) < ASK_DONE_SHOWS_MS
      : a.state === "open" || a.state === "yes");
  } catch (e) { /* the next look */ }
  // YOUR TURN holds only the cards still asking; one said yes to, running
  // or finished is under WHAT THE AGENTS ARE DOING (#asks-said).
  const asking = (a) => !a.ran && a.state === "open";
  asksShown = list.filter(asking).length;
  asksNow = list;
  drawNowWork();
  drawProgress();
  // The runner's turn: what it runs now, then the yeses in the order they
  // were said, which it runs one at a time.
  const turn = [
    ...list.filter((a) => a.ran?.state === "running").sort((x, y) => (x.ran.at_ms ?? 0) - (y.ran.at_ms ?? 0)),
    ...list.filter((a) => !a.ran && a.state === "yes").sort((x, y) => (x.answered_ms ?? 0) - (y.answered_ms ?? 0)),
  ];
  // Open questions first, then the runner's turn, then what it finished.
  const rank = (a) => (!a.ran && a.state === "open" ? 0 : turn.includes(a) ? 1 : 2);
  list.sort((x, y) => rank(x) - rank(y) || (rank(x) === 1 ? turn.indexOf(x) - turn.indexOf(y) : 0));
  const key = JSON.stringify([list.map((a) => [a.id, a.state, a.row, a.ran?.state, turn.indexOf(a), asksOpen.has(a.id)]), rows.filter((r) => r.clip).map((r) => [r.mechanic, r.clip])]);
  if (key === asksDrawn) { drawYourTurn(); return; }
  asksDrawn = key;
  const box = $("asks");
  const said = $("asks-said");
  // The card being written on keeps the person's cursor through a redraw.
  const typing = document.activeElement?.classList?.contains("ask-words")
    ? document.activeElement.closest(".ask-card")?.dataset.ask : null;
  box.textContent = "";
  said.textContent = "";
  box.hidden = asksShown === 0;
  $("cards-said").hidden = asksShown === list.length;
  drawYourTurn();
  for (const a of list) {
    const card = el("div", `ask-card${a.ran || a.state === "yes" ? " ran" : ""}${asksOpen.has(a.id) ? " open" : ""}${asksWording.has(a.id) ? " wording" : ""}`);
    card.dataset.ask = a.id;
    const RAN = { landed: "LANDED", passed: "DONE", failed: "FAILED" };
    const at = turn.indexOf(a);
    const head = at >= 0
      ? (a.ran ? `RUNNING NOW, ${at + 1} OF ${turn.length}` : `YOU SAID YES: WAITS, ${at + 1} OF ${turn.length}`)
      : a.ran ? RAN[a.ran.state] ?? a.ran.state.toUpperCase() : "YOUR AGENT ASKS";
    card.append(el("span", `ask-head${a.ran ? ` ask-ran-${a.ran.state}` : ""}`, head));
    // The question, naming this change: from the command's own message
    // (a gate's patch message: its game, then what changes, up to the
    // first stop), else the ask's sentence.
    const argv = a.argv ?? [];
    const mi = argv.indexOf("--message");
    const msg = mi >= 0 ? String(argv[mi + 1] ?? "").trim() : "";
    const game = a.game ?? "";
    let what = msg.replace(new RegExp(`^${game}\\s*:\\s*`, "i"), "");
    what = what.split(/(?<=[a-z0-9)])[.;]\s|\s\(|: /)[0].trim();
    const verb = a.command === "model gate" ? "Land" : a.command === "registry publish" ? "Publish" : "Allow";
    // A card still asking asks; one answered says what it is.
    let question = !what ? `${a.says ?? ""}`
      : asking(a) ? `${verb} in ${game || "the game"}: ${what}?` : `${what}${/[.!?]$/.test(what) ? "" : "."}`;
    // A failed run says why on its one line: its last line.
    const why = a.ran?.state === "failed" ? String((a.ran.lines ?? []).filter((l) => String(l).trim()).at(-1) ?? "").trim() : "";
    if (why) question = `${why} (${question})`;
    const q = el("span", "ask-question", question);
    q.title = asksOpen.has(a.id) ? "Click to close" : `${question}\nClick for what it does`;
    q.addEventListener("click", () => {
      if (asksOpen.has(a.id)) asksOpen.delete(a.id); else asksOpen.add(a.id);
      void refreshAsks();
    });
    card.append(q);
    const says = el("span", "ask-says", msg || (a.says ?? ""));
    card.append(says);
    // The quest-log row it is for: its step's (row:GAME/ROW), which a
    // click opens; a card with no step was raised outside the loop.
    // The server names it (`row`, from the step's worker record) even after
    // the step closed; else an open step's.
    const row = askRow(a);
    if (row) {
      const go = el("button", "ask-row", `FOR ${row.toUpperCase()}`);
      go.type = "button";
      go.title = "Open this quest-log item";
      go.addEventListener("click", (e) => {
        e.currentTarget.blur();
        toQuestLog(row);
      });
      card.append(go);
      // The clip the card's change is seen in (a gate's clip and frames),
      // else the row's clip to judge on.
      // A gate's clip plays the card's change beside the original.
      const r = rows.find((x) => x.mechanic === row);
      if (a.clip) card.append(judgeButton({ ...(r ?? { mechanic: row }), clip: a.clip, ask: cardAsk(a) }));
      else if (r?.clip) card.append(judgeButton(r));
    } else if (a.clip?.id) {
      card.append(judgeButton({ mechanic: a.game ?? "the change", clip: a.clip, ask: cardAsk(a) }));
      card.append(el("span", "ask-detail", "Raised outside the loop: no quest-log item, no step."));
    } else {
      card.append(el("span", "ask-detail", a.step
        ? `For step ${a.step}, no longer open.`
        : "Raised outside the loop: no quest-log item, no step."));
    }
    card.append(el("span", "ask-detail", `gp ${a.command ?? ""}`));
    if (a.words) card.append(el("span", "ask-detail", `You wrote: ${a.words}`));
    if (a.ran) {
      for (const l of a.ran.lines ?? []) card.append(el("span", "ask-detail", l));
    } else if (a.state === "yes") {
      card.append(el("span", "ask-detail", "The loop's runner runs the yeses one at a time, in the order you said them, while it is on."));
    } else {
      // The answer box, always open on the card: what the person wants to
      // say about what the card shows. ANSWER sends the words alone to the
      // card's row (its talk, as the person's turn) and leaves the card
      // open; YES and NO send them with the answer.
      const t = el("input", "ask-words");
      t.placeholder = "Say anything about this (ANSWER sends it; YES and NO send it too)";
      t.setAttribute("aria-label", "Your answer on this card");
      t.value = askWords.get(a.id) ?? "";
      t.addEventListener("input", () => askWords.set(a.id, t.value));
      t.addEventListener("click", (ev) => ev.stopPropagation());
      const w = el("button", "ask-words-send", "ANSWER");
      w.type = "button";
      w.title = "Sends what you wrote to this card's row, for its agent to read; the card stays for your YES or NO";
      const sendWords = async () => {
        const text = t.value.trim();
        if (!text) { t.focus(); return; }
        const m = askRow(a);
        w.disabled = true;
        try {
          if (m) await api("api/talk", { text: withSeen(`On the card "${cardLine(a.says)}": ${text}`), clips: [], mechanic: m });
          else throw new Error("this card has no row to carry your words: press NO with them");
          t.value = "";
          askWords.delete(a.id);
          note(`Your answer went to ${m}'s agent.`, 5000);
        } catch (err) {
          fault(`Your answer didn't save: ${err.message ?? err}`, 8000);
        } finally { w.disabled = false; }
      };
      w.addEventListener("click", (e) => { e.stopPropagation(); e.currentTarget.blur(); void sendWords(); });
      t.addEventListener("keydown", (ev) => {
        ev.stopPropagation();
        if (ev.code === "Escape") t.blur();
        if (ev.code === "Enter") { ev.preventDefault(); void sendWords(); }
      });
      for (const [word, val] of [["YES", "yes"], ["NO", "no"]]) {
        const b = el("button", `ask-${val}`, word);
        b.type = "button";
        b.title = val === "yes"
          ? "Let your agent do this, once (with your words, if you wrote any)"
          : "Your agent won't do this; with words, its row goes back to its worker with them";
        b.addEventListener("click", async (e) => {
          e.currentTarget.blur();
          for (const x of card.querySelectorAll("button")) x.disabled = true;
          const words = t.value.trim() ? withSeen(t.value.trim()) : "";
          try {
            const r = await fetch(`/api/asks/${encodeURIComponent(a.id)}`, {
              method: "POST",
              headers: { "content-type": "application/json" },
              body: JSON.stringify(words ? { answer: val, words } : { answer: val }),
            });
            const out = await r.json().catch(() => ({}));
            if (!r.ok) says.textContent = `${a.says} (Your answer didn't save: ${out.error ?? r.status}.)`;
            else {
              askWords.delete(a.id);
              asksWording.delete(a.id);
              if (out.talk_error) fault(`Your answer saved, but your words didn't reach the row: ${out.talk_error}`, 8000);
              else if (words && out.row) note(`Your words went to ${out.row.split("/").slice(1).join("/")}.`, 5000);
            }
          } catch (err) {
            says.textContent = `${a.says} (Your answer didn't save. Is the page's server still running?)`;
          }
          asksDrawn = "";
          void refreshAsks();
        });
        card.append(b);
      }
      // DUPLICATE OF: a NO that names the other open card of the game this
      // one makes again. Its row then waits on that card's row instead of
      // going back to make the same card a third time (serve/runner.rs,
      // duplicate_of).
      const others = list.filter((x) => x.id !== a.id && !x.ran && (x.state === "open" || x.state === "yes") && (x.game ?? "") === (a.game ?? ""));
      if (others.length) {
        const d = el("select", "ask-duplicate");
        d.title = "Say NO because another card already makes this change: this card's row waits for that one";
        d.setAttribute("aria-label", "Refuse this card as a duplicate of another card");
        d.append(new Option("DUPLICATE OF...", ""));
        for (const x of others) {
          const xm = (x.argv ?? []).indexOf("--message");
          const xs = xm >= 0 ? String(x.argv[xm + 1] ?? "") : String(x.says ?? "");
          const xw = xs.replace(new RegExp(`^${x.game ?? ""}\\s*:\\s*`, "i"), "").split(" Expect:")[0].trim();
          const xr = askRow(x);
          d.append(new Option(`${xr ? `${xr}: ` : ""}${xw.length > 70 ? `${xw.slice(0, 70)}...` : xw}`, x.id));
        }
        d.addEventListener("click", (ev) => ev.stopPropagation());
        d.addEventListener("change", async () => {
          const other = d.value;
          if (!other) return;
          for (const x of card.querySelectorAll("button, select")) x.disabled = true;
          const words = t.value.trim() ? withSeen(t.value.trim()) : "";
          const body = { answer: "no", duplicate_of: other };
          if (words) body.words = words;
          try {
            const r = await fetch(`/api/asks/${encodeURIComponent(a.id)}`, {
              method: "POST",
              headers: { "content-type": "application/json" },
              body: JSON.stringify(body),
            });
            const out = await r.json().catch(() => ({}));
            if (!r.ok) says.textContent = `${a.says} (Your answer didn't save: ${out.error ?? r.status}.)`;
            else {
              askWords.delete(a.id);
              asksWording.delete(a.id);
              note("NO, as a duplicate: its row waits for the other card.", 5000);
            }
          } catch (err) {
            says.textContent = `${a.says} (Your answer didn't save. Is the page's server still running?)`;
          }
          asksDrawn = "";
          void refreshAsks();
        });
        card.append(d);
      }
      card.append(t, w);
    }
    (asking(a) ? box : said).append(card);
    if (typing === a.id) card.querySelector(".ask-words")?.focus();
  }
}

// Your turn ------------------------------------------------------------
//
// Right under the screens, only what the person can do now, each with a
// button that does it: the yes cards still asking (#asks; one answered is
// under WHAT THE AGENTS ARE DOING, #asks-said), the loop's steps for
// this game that are the person's and have a button (PLAY FROM HERE: the
// row's start point, recording into the row), and the questions of the
// rows (askOf: a worker's question in the talk or its status line, or a
// row asking them), each with its clip and an ANSWER box open, whose words
// go to the row's talk. A row that says to press YES shows only with its
// card; once the card is gone the row follows it (serve/runner.rs,
// settle_card_rows). With none of them, NOTHING NEEDS YOU.

/** Whether words ask something: a `?` ending a sentence. */
const isQuestion = (t) => /\?(?=$|[\s"')*_])/.test(String(t ?? ""));
/** Whether a row's note speaks of its card (press YES, the card is up). */
const ofCard = (t) => /\bcards?\b/i.test(String(t ?? "")) || /\bYES\b/.test(String(t ?? ""));

/** The cards (asksNow) for row `m` of the open game. */
function cardsOf(m) {
  const game = session?.game ?? "";
  return asksNow.filter((a) => a.row === `${game}/${m}`);
}

/** What a row asks the person now, or null: the question its agent said in
 * the talk (its newest turn, waiting on the person), its status line when
 * that asks, or its note when it is asking the person. A row whose note
 * talks of a card the page no longer has asks only what is left to do. */
function askOf(r) {
  if (!r.mechanic || r.status === "done") return null;
  const g = clipsBy.find((x) => x.mechanic === r.mechanic);
  const last = (g?.talk ?? []).at(-1);
  if (last && last.by === "agent" && g.waiting_for === "player" && isQuestion(last.text)) {
    const n = typeof last.show === "number" ? last.show : (last.clips ?? [])[0];
    return { says: last.text, from: "talk", clip: typeof n === "number" ? { n, frame: typeof last.frame === "number" ? last.frame : 0 } : null };
  }
  if (g?.busy && isQuestion(g.busy)) return { says: g.busy, from: "busy", clip: null };
  if (r.status !== "asking you") return null;
  // A look to confirm never had a card: it asks in its own words, with
  // LOOK AT IT, CONFIRM and the answer box (its note may say YES).
  if (confirmable(r)) return { says: confirmSays(r), from: "confirm", clip: null };
  if (ofCard(r.note)) {
    const cards = cardsOf(r.mechanic);
    // Its card is here (YOUR TURN shows it with YES and NO), or running.
    if (cards.some((a) => a.ran?.state === "running" || (!a.ran && (a.state === "open" || a.state === "yes")))) return null;
    // It just ran: its row follows it in a few seconds.
    if (cards.some((a) => a.ran)) return null;
    return { says: "Its card is no longer on your page. What should happen with it?", from: "gone", clip: null, was: r.note };
  }
  return { says: r.note || "Your agent asks you about this row.", from: "note", clip: null };
}

/** What a look to confirm asks: its plain sentence, else "Watch PART beside
 * the original. ..." with PART the scoreboard line it came from (a
 * confirm-LINE row's from) or its name. */
function confirmSays(r) {
  if (r.plain) return r.plain;
  const line = /^scoreboard line (.+), measured\b/.exec(String(r.from ?? ""))?.[1];
  const part = line ?? (cardTitle(r.mechanic).replace(/^confirm\s+/i, "") || "it");
  return `Watch ${part} beside the original. If it looks right, press CONFIRM; if not, say what is wrong.`;
}

/** What is typed in a words box not sent yet, by its row (and kind), kept
 * across the lists' redraws. */
const typedWords = new Map();

/** An ANSWER box, open: the person's words go into row `m`'s talk, which a
 * worker waiting on the row hears. `o` names it otherwise: its words
 * (`placeholder`), its button (`word`) and what the page says once sent
 * (`sent`); `o.kind` keeps its typed words apart from another box's. */
function answerBox(m, clips, o = {}) {
  const f = el("form", "comment answer");
  f.autocomplete = "off";
  f.dataset.mechanic = m;
  const keep = `${o.kind ?? "answer"}:${m}`;
  const t = el("input", "comment-text");
  t.placeholder = o.placeholder ?? `Your answer on ${cardTitle(m)}`;
  t.setAttribute("aria-label", t.placeholder);
  t.value = typedWords.get(keep) ?? "";
  t.addEventListener("input", () => { if (t.value) typedWords.set(keep, t.value); else typedWords.delete(keep); });
  const go = el("button", "", o.word ?? "ANSWER");
  go.type = "submit";
  f.append(t, go);
  for (const x of [f, t, go]) x.addEventListener("click", (ev) => ev.stopPropagation());
  t.addEventListener("keydown", (ev) => { ev.stopPropagation(); if (ev.code === "Escape") t.blur(); });
  f.addEventListener("submit", async (ev) => {
    ev.preventDefault();
    const text = t.value.trim();
    if (!text) { t.focus(); return; }
    go.disabled = true;
    try {
      await api("api/talk", { text: withSeen(text), clips, mechanic: m });
      t.value = "";
      typedWords.delete(keep);
      t.blur();
      note(o.sent ?? `Your answer on ${m} went to its agent.`, 5000);
      turnDrawn = "";
      await refreshLists(true);
    } catch (err) {
      fault(`Your answer didn't save: ${err.message ?? err}`, 8000);
    } finally { go.disabled = false; }
  });
  return f;
}

/** A LOOK AT IT button for a clip a turn of the talk names. */
function lookButton(m, c) {
  const b = el("button", "judge", `LOOK AT IT: CLIP ${c.n}${c.frame ? `, FRAME ${c.frame}` : ""}`);
  b.type = "button";
  b.title = "Shows this clip on the screens above: the original beside the model";
  b.addEventListener("click", (e) => {
    e.stopPropagation();
    e.currentTarget.blur();
    show.by = "person";
    setView("side");
    $("screens").scrollIntoView({ behavior: "smooth", block: "start" });
    void openClip(c.n, m, c.frame ?? 0).then(() => { drawClips(); report(); }).catch((err) => fault(`Couldn't open clip ${c.n}: ${err.message ?? err}`, 8000));
  });
  return b;
}

let turnDrawn = "";
// The row PROGRESS's READY FOR YOU pointed at: outlined in YOUR TURN.
let turnFocus = null;
function drawYourTurn() {
  const list = $("turn");
  // An answer being typed stays until it is sent or left empty.
  if ([...list.querySelectorAll("input")].some((i) => i.value.trim() || i === document.activeElement)) return;
  const game = session?.game ?? "";
  const plays = openSteps.filter((s) => s.who === "person" && s.act?.do === "play_from"
    && (!game || (s.touches ?? []).includes(`row:${game}/${s.act.row}`)));
  const played = new Set(plays.map((s) => s.act.row));
  const asking = rows.filter((r) => !played.has(r.mechanic)).map((r) => [r, askOf(r)]).filter(([, q]) => q);
  const key = JSON.stringify([plays.map((s) => [s.id, s.says]), asking.map(([r, q]) => [r.mechanic, q, r.clip ?? null]), asksShown, turnFocus]);
  if (key === turnDrawn) return;
  turnDrawn = key;
  list.textContent = "";
  for (const s of plays) {
    const li = el("li", "turn-item");
    li.dataset.step = s.id;
    li.append(el("span", "turn-row", s.act.row.toUpperCase()), shortText("turn-says", s.says));
    const b = el("button", "turn-play", "PLAY FROM HERE");
    b.type = "button";
    b.title = `Plays from ${s.act.row}'s start point and records into it: play the stretch, then RECORD (or R) sends it`;
    b.addEventListener("click", (e) => { e.currentTarget.blur(); void playFrom(s.act.row); });
    li.append(b);
    list.append(li);
  }
  for (const [r, q] of asking) {
    const li = el("li", "turn-item turn-ask");
    if (r.mechanic === turnFocus) li.classList.add("turn-focus");
    li.dataset.mechanic = r.mechanic;
    const says = shortText("turn-says", q.says);
    if (q.was) says.title = `The row said: ${q.was}`;
    const who = el("span", "turn-row", cardTitle(r.mechanic).toUpperCase());
    who.title = r.mechanic;
    li.append(who, says);
    // The clip it is about: the row's clip to judge, else the one its
    // agent's question names.
    let clips = [];
    if (r.clip) {
      li.append(judgeButton(r));
      const yes = confirmButton(r);
      if (yes) li.append(yes);
      li.title = `Click: set up ${judgeWords(r).toLowerCase()} on the screens above`;
      li.addEventListener("click", () => void judgeOn(r));
      if (!r.clip.id && !r.clip.mechanic) clips = [r.clip.n];
    } else if (q.clip) {
      li.append(lookButton(r.mechanic, q.clip));
      clips = [q.clip.n];
    }
    li.append(answerBox(r.mechanic, clips, q.from === "confirm" ? { placeholder: "Not right? Say what is wrong" } : {}));
    list.append(li);
  }
  $("turn-none").hidden = asksShown + plays.length + asking.length > 0;
}

/** A step's PLAY FROM HERE (app.js's playFrom): opens the row, plays its
 * request's path to its frame, starts the model there and records into
 * the row, so the person plays only the stretch it asks for. */
let starting = null;
async function playFrom(m) {
  if (starting) { note("Still loading the start point."); return; }
  if (recording || sending) { note("Send the recording first: RECORD (or R)."); return; }
  if (m !== session?.mechanic) {
    await pickRow(m);
    if (session?.mechanic !== m) return;
  }
  const r = rows.find((q) => q.mechanic === m)?.triage?.request ?? null;
  if (show.view === "solo") setView("side");
  if (mode === "clip") { clip = null; $("clipbar").hidden = true; drawClips(); }
  $("screens").scrollIntoView({ behavior: "smooth", block: "start" });
  starting = m;
  mode = "loading";
  model.on = false;
  model.say = "THE MODEL JOINS AT THE START POINT";
  try {
    if (r?.path) {
      const res = await fetch(`api/paths/${r.path}`);
      if (!res.ok) throw new Error(await res.text());
      // The start point plays on the set its path was recorded on.
      const text = await res.text();
      player.replay(0, await romFor(text), text);
      const target = r.frame ?? 0;
      while (player.frame(0) < target) {
        const before = player.frame(0);
        player.runTo(0, Math.min(target, before + 300));
        if (player.frame(0) === before) break;
        line(`Loading the start point: frame ${player.frame(0)} of ${target}.`);
        await new Promise((t) => setTimeout(t, 0));
      }
    } else {
      player.power(0, rom);
    }
    mode = "live";
    $("record").disabled = false;
    if (model.ready) void startModel(++model.token); else void loadModel();
    startRecording();
    clipFor = m;
    markTarget();
    note(`Recording into ${m} from its start point. ${r?.stop ? `Stop: ${r.stop}. ` : ""}RECORD (or R) sends it.`, 8000);
  } catch (e) {
    fault(`Couldn't load the start point: ${e.message ?? e}`, 8000);
  } finally {
    if (mode === "loading") mode = "live";
    starting = null;
  }
}

// Now ------------------------------------------------------------------
//
// One status line under YOUR TURN: the quest-log items being worked on
// (status "agent working"), by whom, with each row's status line (its busy
// text), then the loop: THE LOOP RUNS, or, with no worker running and no
// step able to start, why in plain words: THE LOOP WAITS ON YOU (a click
// goes to YOUR TURN), THE LOOP IS STOPPED: THE RUNNER IS OFF, THE LOOP IS
// HALTED, or THE LOOP HAS NOTHING TO DO. The loop's switches (runner,
// workers, steps required) are on the loop's page, which the line links.

/** "12 s ago", "4 min ago", "3 h ago", "2 days ago" from ms since the epoch. */
function ago(ms) {
  const s = Math.max(0, Math.round((Date.now() - ms) / 1000));
  if (s < 60) return `${s} s ago`;
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  return `${Math.floor(s / 86400)} days ago`;
}

let loopNow = null; // /api/steps as last seen: the runner and its workers

/** The live worker on row `r` of the open game, if one runs. */
function workerOn(r) {
  const game = session?.game ?? "";
  return (loopNow?.workers ?? []).find((w) => LIVE.includes(w.state)
    && ((r.worker && w.worker === r.worker) || (w.step?.touches ?? []).includes(`row:${game}/${r.mechanic}`))) ?? null;
}

/** Whether the loop is still: no live worker, no yes'd card running or
 * waiting for the runner, and no open step the runner could start. Then
 * the words for NOW (head, why) and what waits on the person, each with
 * a click to it: the open cards and this game's rows asking them, and
 * the steps that are theirs. null while anything moves or before the
 * first /api/steps. */
function loopIdle() {
  const v = loopNow;
  if (!v) return null;
  if ((v.workers ?? []).some((w) => LIVE.includes(w.state))) return null;
  if (asksNow.some((a) => a.ran?.state === "running" || (!a.ran && a.state === "yes"))) return null;
  const run = v.settings?.runner ?? {};
  const open = v.open ?? [];
  const theirs = open.filter((s) => s.who !== "person");
  if (!v.halted && run.on && theirs.length) return null;
  const waits = [];
  for (const a of asksNow.filter((x) => !x.ran && x.state === "open")) {
    const what = a.game ? `CARD: ${a.game.toUpperCase()} ${a.command ?? ""}` : `CARD: ${a.command ?? "a yes"}`;
    waits.push({ label: what.trim(), title: `${a.says ?? ""}\nClick to open the card`, go: () => {
      asksOpen.add(a.id);
      asksDrawn = "";
      void refreshAsks();
      $("your-turn")?.scrollIntoView({ behavior: "smooth", block: "start" });
    } });
  }
  for (const r of rows.filter((x) => x.status === "asking you" && x.mechanic)) {
    waits.push({ label: r.mechanic.toUpperCase(), title: `${r.note ?? ""}\nClick to open this item${r.clip ? ` and set up ${judgeWords(r).toLowerCase()} on the screens` : ""}`, go: () => {
      void pickRow(r.mechanic);
      if (r.clip) { void judgeOn(r); return; }
      $("your-turn")?.scrollIntoView({ behavior: "smooth", block: "start" });
    } });
  }
  const game = session?.game ?? "";
  for (const s of open.filter((x) => x.who === "person" && (!game || (x.touches ?? []).some((t) => t.startsWith(`row:${game}/`))))) {
    const row = (s.touches ?? []).find((t) => t.startsWith("row:"))?.split("/").slice(1).join("/") ?? null;
    if (row && waits.some((x) => x.label === row.toUpperCase())) continue;
    waits.push({ label: row ? row.toUpperCase() : `STEP ${s.id}`, title: `${s.says ?? ""}${row ? "\nClick to open this item" : ""}`, go: () => {
      if (row) void pickRow(row);
      $("your-turn")?.scrollIntoView({ behavior: "smooth", block: "start" });
    } });
  }
  const n = theirs.length;
  const steps = `${n} step${n === 1 ? "" : "s"}`;
  if (v.halted) {
    return { kind: "halted", head: "THE LOOP IS HALTED", why: `No step starts until a change to the tool answers your WRONG: ${v.halted.says ?? ""}`.trim(), waits };
  }
  if (!run.on && n) {
    return { kind: "off", head: "THE LOOP IS STOPPED: THE RUNNER IS OFF", why: `${steps} wait${n === 1 ? "s" : ""} for an agent, and no worker starts while the runner is off.`, waits };
  }
  if (waits.length) {
    return { kind: "you", head: "THE LOOP IS WAITING ON YOU", why: `No worker runs and no step can start until you answer${waits.length > 1 ? " one of these" : " this"}:`, waits };
  }
  return { kind: "nothing", head: "THE LOOP HAS NOTHING TO DO", why: "No worker runs, no step is open and nothing waits on you.", waits };
}

function drawNowWork() {
  const box = $("now-work");
  const working = rows.filter((r) => r.mechanic && worked(r));
  const wk = follow.work;
  // Whether an agent is there for each row, and when it last acted: a row
  // marked working with no live worker says so, with the runner's state.
  const lines = working.map((r) => {
    const w = workerOn(r);
    const times = [w ? w.last_ms ?? w.started_ms : null, r.mechanic === session?.mechanic ? session?.agent_ms : null].filter((t) => typeof t === "number");
    const last = times.length ? `last acted ${ago(Math.max(...times))}` : "no time of its last act";
    const there = w
      ? `YOUR AGENT IS THERE (${w.worker ?? w.step?.id ?? "a worker"})`
      : `NO AGENT IS RUNNING FOR IT${loopNow && !loopNow.settings?.runner?.on ? ": THE RUNNER IS OFF" : ""}`;
    return { r, w: !!w, there, last };
  });
  const idle = loopIdle();
  const trouble = rows.filter((r) => r.trouble && r.status !== "done");
  const busyOf =(m) => clipsBy.find((g) => g.mechanic === m)?.busy ?? (m === session?.mechanic ? session?.busy : null) ?? null;
  const key = JSON.stringify([lines.map((l) => [l.r.mechanic, l.r.worker, l.there, l.last, busyOf(l.r.mechanic)]), wk && [wk.clip, wk.from, wk.to, wk.says],
    idle && [idle.kind, idle.head, idle.waits.length], loopNow?.halted?.says ?? null, loopNow?.settings?.runner?.on ?? null, trouble.map((r) => r.mechanic)]);
  if (box.dataset.key === key) return;
  box.dataset.key = key;
  box.textContent = "";
  // One line (owner, page-one-design): what is being worked on and by
  // whom, then whether the loop runs or waits on the person. The rest is
  // on the loop's page, which the line ends with a link to.
  const line = el("div", "now-line");
  const part = (cls, text, title) => { const s = el("span", cls, text); if (title) s.title = title; line.append(s); return s; };
  const sep = () => part("now-sep", " · ");
  const work = [];
  if (wk) {
    const [lo, hi] = workRange(wk);
    work.push(() => { part("now-label", "REFINING "); part("now-what", `${wk.clip}, frames ${lo} to ${hi}`, wk.says ?? ""); });
  }
  for (const { r, w, there, last } of lines) {
    work.push(() => {
      part(`now-label${w ? "" : " now-absent"}`, w ? "WORKING ON " : "MARKED WORKING, NO AGENT: ");
      const name = el("button", "now-row", r.mechanic.toUpperCase());
      name.type = "button";
      name.title = `${there}, ${last}. Click to open it in the quest log`;
      name.addEventListener("click", (e) => { e.currentTarget.blur(); toQuestLog(r.mechanic); });
      line.append(name);
      const by = w ? ` by ${workerOn(r)?.worker ?? r.worker ?? "an agent"}, ${last}` : "";
      const b = busyOf(r.mechanic);
      part("now-who", `${by}${b ? `: ${oneSentence(b, 120)}` : ""}`, b ?? "");
    });
  }
  if (!work.length) part("now-idle", "NOBODY IS WORKING ON AN ITEM");
  work.forEach((f, i) => { if (i) sep(); f(); });
  sep();
  // The loop: runs, or why it is still, from loopIdle.
  if (idle) {
    const s = part(`now-loop-state now-stopped-${idle.kind}`, idle.kind === "you" ? `THE LOOP WAITS ON YOU (${idle.waits.length} IN YOUR TURN)` : idle.head, idle.why);
    if (idle.kind === "you") {
      s.classList.add("now-click");
      s.addEventListener("click", () => $("your-turn").scrollIntoView({ behavior: "smooth", block: "start" }));
    }
  } else if (loopNow) {
    part("now-loop-state now-runs", loopNow.halted ? "THE LOOP IS HALTED" : loopNow.settings?.runner?.on ? "THE LOOP RUNS" : "THE RUNNER IS OFF", loopNow.runner?.says ?? "");
  }
  // The loop's own trouble it filed as rows (crates/gp-cli/src/trouble.rs),
  // not done yet: how many, opening the first on the loop's page.
  if (trouble.length) {
    sep();
    const t = el("a", "now-trouble", `${trouble.length} ROW${trouble.length === 1 ? "" : "S"} OF THE LOOP'S OWN TROUBLE`);
    t.href = loopHref(trouble[0].mechanic);
    t.title = `Errors and hand fixes the loop filed itself: ${trouble.map((r) => r.mechanic).join(", ")}`;
    line.append(t);
  }
  // Everything else is on the loop's page.
  sep();
  const to = el("a", "now-page", "THE LOOP'S PAGE");
  to.href = loopHref(null);
  to.title = "The quest log, the steps, the workers and the clips";
  line.append(to);
  box.append(line);
}

// The agent's turns that name a clip (say --show N, with --frame F, or
// words like "clip 3, frame 120"): the newest new one opens on the screens
// there, held like an ask of the agent's. Turns from before the page opened
// aren't replayed.
let agentTurnSeen = null; // [mechanic, n] of the newest agent turn looked at
function showNamedClip() {
  const m = session?.mechanic;
  const talk = session?.talk ?? [];
  const t = [...talk].reverse().find((x) => x.by === "agent");
  if (!m) return;
  const seen = agentTurnSeen;
  agentTurnSeen = [m, t?.n ?? 0];
  if (!t || !seen || seen[0] !== m || (t.n ?? 0) <= seen[1]) return;
  const words = String(t.text ?? "");
  const n = typeof t.show === "number" ? t.show : Number(/\bclip\s+(\d+)/i.exec(words)?.[1] ?? NaN);
  if (!Number.isFinite(n)) return;
  const f = typeof t.frame === "number" ? t.frame : Number(/\bframe\s+(\d+)/i.exec(words)?.[1] ?? 0);
  if (recording || sending) return;
  show.by = "agent";
  agentHeldUntil = performance.now() + AGENT_HOLD_MS;
  note(`Your agent is showing clip ${n} of ${m}, frame ${f}.`, 8000);
  void openClip(n, m, f).then(() => { show.by = "agent"; drawClips(); drawFollow(); report(); })
    .catch((e) => {
      if (mode === "clip" && !clip) backToLive();
      fault(`Couldn't show clip ${n} your agent named: ${e.message ?? e}`, 8000);
    });
}

function drawNowLoop(v) {
  const box = $("now-loop");
  const st = v.settings ?? {};
  const run = st.runner ?? {};
  const parts = [
    v.halted ? `LOOP HALTED: ${v.halted.says ?? ""}` : "LOOP RUNNING",
    `RUNNER ${run.on ? `ON (at most ${run.max_workers ?? "?"} workers, ${(v.workers ?? []).length} now)` : "OFF"}`,
    `STEPS REQUIRED ${st.steps_required ? "ON" : "OFF"}`,
  ];
  const text = parts.join("  ·  ");
  if (box.textContent !== text) box.textContent = text;
  box.classList.toggle("halted", !!v.halted);
  box.title = v.runner?.says ?? "";
}

// The loop's steps -----------------------------------------------------
//
// The open steps of the home (GET /api/steps: crate::steps, docs/loop.md),
// each a small card with who does it and one line of its words; a click
// shows all its words, its id and OBJECT TO THIS STEP (the page's WRONG):
// the person's words on why, sent as POST /api/steps/ID/wrong, which halts the loop
// until a change to the tool that names the objection lands. While it is
// halted the list is empty and the halt says why.

let stepsDrawn = "";
let openSteps = []; // the open steps as /api/steps last gave them
let stepsWriting = false; // a WRONG box is open: don't redraw under it
// The scoreboard (docs/loop.md, "The scoreboard"): the game's parts, each
// with its game state, background and sprites as gp serve last measured
// them and how each moved since the measure before.
let scoreDrawn = "";
// The page's game from its address (/mechanic/GAME/screen.html), so the
// scoreboard is asked for before the session and the page's other polls.
const URL_GAME = decodeURIComponent(location.pathname.match(/\/mechanic\/([^/]+)\//)?.[1] ?? "") || null;
async function refreshScoreboard() {
  const game = session?.game ?? URL_GAME;
  if (!game) return;
  let v = null;
  try {
    const r = await fetch(`/api/scoreboard/${encodeURIComponent(game)}`);
    if (r.ok) v = await r.json();
  } catch (e) { /* the next look */ }
  const lines = v?.lines ?? [];
  if (v) { board = v; drawProgress(); drawAgentsHead(); }
  const key = JSON.stringify([v?.error ?? null, lines, v?.held_out ?? []]);
  if (key === scoreDrawn) return;
  scoreDrawn = key;
  $("scoreboard").hidden = !v || (lines.length === 0 && !v.error);
  const body = $("score-lines");
  body.textContent = "";
  if (v?.error) {
    const tr = body.insertRow();
    const td = tr.insertCell();
    td.colSpan = 5;
    td.textContent = `The scoreboard can't be read: ${v.error}`;
    return;
  }
  const cell = (tr, text, moved, title) => {
    const td = tr.insertCell();
    td.textContent = text;
    if (title) td.title = title;
    if (moved) {
      const m = document.createElement("span");
      m.className = `mv ${moved}`;
      m.textContent = moved;
      td.append(m);
    }
    return td;
  };
  const frames = (x) => (x ? `${x.matched} of ${x.total}` : "not measured yet");
  for (const l of lines) {
    const tr = body.insertRow();
    const when = l.measured_ms ? `measured ${ago(l.measured_ms)}` : "not measured yet";
    const why = l.error ? `; the last measure failed: ${l.error}` : l.measuring ? "; being measured now" : l.stale ? "; out of date, measured again soon" : "";
    cell(tr, l.name, "", `clip ${l.clip}${l.level ? `, level ${l.level}` : ""}; ${when}${why}`);
    cell(tr, l.state ? `${l.state.pct}%` : "no score", l.state?.moved ?? "", l.state ? `of ${l.state.frames} frames` : "the gate has no score for this clip yet");
    if (l.own_draws) {
      // The model on its own random draws: frames together straight
      // through, in place of the pictures.
      const td = cell(tr, `own draws: ${frames(l.own)}`, l.own?.moved ?? "", "frames together straight through with none of the original's random draws handed over (gp clip score --own-draws)");
      td.colSpan = 2;
      if (l.stale || l.measuring) td.classList.add("old");
    } else {
      for (const k of ["background", "sprites"]) {
        const td = cell(tr, frames(l[k]), l[k]?.moved ?? "", "frames matched of the clip's frames");
        if (l.stale || l.measuring) td.classList.add("old");
      }
    }
    cell(tr, l.approved?.length ? l.approved.join(", ") : "none", "");
  }
  // The held-out runs: sealed plays the model is never tuned on, each with
  // its game state straight through and with resync (gp sealed score).
  const held = v?.held_out ?? [];
  if (!held.length) {
    const td = body.insertRow().insertCell();
    td.colSpan = 5;
    td.className = "held-none";
    td.textContent = "HELD OUT: no sealed runs yet. While recording a whole level, STOP AND SEAL keeps it as one no worker sees.";
  }
  for (const r of held) {
    const tr = body.insertRow();
    tr.className = "held";
    const when = r.measured_ms ? `scored ${ago(r.measured_ms)}` : "not scored yet";
    const why = r.error ? `; the last score failed: ${r.error}` : r.measuring ? "; being scored now" : r.stale ? "; out of date, scored again soon" : "";
    cell(tr, `HELD OUT: ${r.name}`, "", `sealed run ${r.run}, never seen by a worker; ${when}${why}`);
    const st = cell(tr, r.straight ? `${r.straight.pct}%` : "not scored yet", r.straight?.moved ?? "", r.straight ? `${r.straight.together} of ${r.straight.frames} frames together straight through` : "");
    if (r.stale || r.measuring) st.classList.add("old");
    const rest = cell(tr, r.resync ? `with resync ${r.resync.pct}%${r.longest ? `, longest together ${r.longest} s` : ""}` : "", r.resync?.moved ?? "", "frames together when the model is put back on the original after each split; a sealed run's pictures aren't checked");
    rest.colSpan = 3;
  }
}

// Progress ---------------------------------------------------------------
//
// The top of the loop's page (owner, loop-page-progress-first): a line for
// each part of the game on its scoreboard, in its order: its name, its
// state (LOCKED with the day the person said yes; READY FOR YOU while its
// confirm row asks them; else the background and the sprites as bars, the
// frames matched of the clip's), and one plain sentence of what is being
// done for it now, from the open rows whose target names the line. A
// number that moved since the page was opened says by how much.

let board = null; // /api/scoreboard/GAME as last read
// Each line's numbers when the page first read them, by name.
const firstSeen = new Map();
let progressDrawn = "";

/** A row's cards (asksNow) by what they wait on: the person's YES, or the
 * runner landing it. */
function cardState(m) {
  const cards = cardsOf(m);
  if (cards.some((a) => !a.ran && a.state === "open")) return "yes";
  if (cards.some((a) => a.ran?.state === "running" || (!a.ran && a.state === "yes"))) return "landing";
  return null;
}

/** N and the word, a plural for any N but 1: "3 fixes", "1 fix". */
const count = (n, one, many) => `${n} ${n === 1 ? one : many}`;

/** The one sentence of what is being done for line `l` now. */
function partSays(l, state) {
  // The scoreboard can come before the quest log: say so, not "nothing".
  if (!rowsRead && state !== "locked") return "Reading what is being done for it.";
  const name = String(l.name ?? "").toLowerCase();
  const open = rows.filter((r) => r.mechanic && r.status !== "done" && r.target_line === name);
  // A row parked in line is worked only when it says so: a worker the
  // runner left on it isn't working it (every game row parked while the
  // game's work is held read as being worked on).
  const busy = (r) => (r.status === "in line" ? false : worked(r));
  const rest = open.filter((r) => !busy(r));
  const working = open.length - rest.length;
  const yes = rest.filter((r) => cardState(r.mechanic) === "yes").length;
  const landing = rest.filter((r) => cardState(r.mechanic) === "landing").length;
  const after = rest.filter((r) => !cardState(r.mechanic));
  const measuring = after.filter((r) => r.awaiting_measure).length;
  const idle = after.filter((r) => !r.awaiting_measure);
  // Only a row up next waits for an agent; one in line is parked, and one
  // asking the person waits for them.
  const parked = idle.filter((r) => r.status === "in line").length;
  const asking = idle.filter((r) => r.status === "asking you").length;
  const waiting = idle.length - parked - asking;
  const parts = [];
  if (working) parts.push(`${count(working, "fix is", "fixes are")} being worked on`);
  if (yes) parts.push(`${count(yes, "fix waits", "fixes wait")} for your YES`);
  if (landing) parts.push(`${count(landing, "fix is", "fixes are")} landing`);
  if (measuring) parts.push(`${count(measuring, "fix has", "fixes have")} landed and ${measuring === 1 ? "is" : "are"} being measured`);
  if (waiting) parts.push(`${count(waiting, "fix waits", "fixes wait")} for an agent`);
  if (asking) parts.push(`${count(asking, "fix asks", "fixes ask")} you to look`);
  if (parked) parts.push(`${count(parked, "fix is", "fixes are")} parked`);
  if (parts.length) return `${parts.join(", ")}.`.replace(/^./, (c) => c.toUpperCase());
  if (state === "locked") return "You said it looks right, so no change can make it look worse.";
  if (state === "ready") return "It measures near full: watch it beside the original and say if it looks right.";
  if (l.measuring) return "Being measured now.";
  return "Nothing is being done for it right now.";
}

/** A bar: what it measures, the frames matched of the total, and how far
 * it moved since the page was opened. */
function bar(label, x, since) {
  const box = el("div", "pbar");
  box.append(el("span", "pbar-label", label));
  const track = el("span", "pbar-track");
  const fill = el("span", "pbar-fill");
  const share = x && x.total ? x.matched / x.total : 0;
  fill.style.width = `${(share * 100).toFixed(1)}%`;
  if (x && x.total && x.matched >= x.total) fill.classList.add("full");
  track.append(fill);
  box.append(track, el("span", "pbar-n", x ? `${x.matched} of ${x.total} frames` : "not measured yet"));
  if (x && typeof since === "number" && x.matched !== since) {
    const d = x.matched - since;
    box.append(el("span", `pmoved ${d > 0 ? "up" : "down"}`, `${d > 0 ? "+" : ""}${d} since you opened this page`));
  }
  return box;
}

function drawProgress() {
  if (!LOOP_PAGE) return;
  const list = $("parts");
  const none = $("parts-none");
  const lines = board?.lines ?? [];
  for (const l of lines) {
    if (!firstSeen.has(l.name) && (l.background || l.sprites || l.own)) {
      firstSeen.set(l.name, { background: l.background?.matched, sprites: l.sprites?.matched, own: l.own?.matched });
    }
  }
  const of = (l) => {
    const confirm = rows.find((r) => r.mechanic === l.confirm_row && r.status === "asking you");
    const state = l.locked ? "locked" : confirm ? "ready" : "measure";
    return { l, state, confirm, says: partSays(l, state) };
  };
  const parts = lines.map(of);
  const key = JSON.stringify([board?.error ?? null, parts.map((p) => [p.l, p.state, p.says]), [...firstSeen]]);
  if (key === progressDrawn) return;
  progressDrawn = key;
  list.textContent = "";
  none.hidden = !(board?.error || !lines.length);
  // No answer yet is not "no scoreboard": that is said only once an answer
  // came with no lines.
  none.textContent = !board
    ? "Reading the game's progress."
    : board.error
      ? `The scoreboard can't be read: ${board.error}`
      : "This game has no scoreboard yet: its parts show here once it has one.";
  for (const { l, state, confirm, says } of parts) {
    const li = el("li", `part part-${state}`);
    const head = el("div", "phead");
    const name = String(l.name ?? "").replace(/^./, (c) => c.toUpperCase());
    head.append(el("span", "pname", name));
    if (state === "locked") {
      head.append(el("span", "pstate plocked", `LOCKED ${l.locked.day ?? ""}`.trim()));
    } else if (state === "ready") {
      const b = el("button", "pstate pready", "READY FOR YOU");
      b.type = "button";
      b.title = "It measures near full: it waits for you in YOUR TURN, below";
      b.addEventListener("click", (e) => {
        e.currentTarget.blur();
        turnFocus = confirm.mechanic;
        turnDrawn = "";
        drawYourTurn();
        $("your-turn").scrollIntoView({ behavior: "smooth", block: "start" });
      });
      head.append(b);
    }
    li.append(head);
    if (state !== "locked") {
      const seen = firstSeen.get(l.name) ?? {};
      const bars = el("div", "pbars");
      if (l.own_draws) bars.append(bar("ON ITS OWN", l.own, seen.own));
      else bars.append(bar("BACKGROUND", l.background, seen.background), bar("SPRITES", l.sprites, seen.sprites));
      if (l.stale || l.measuring) bars.classList.add("old");
      bars.title = l.measuring ? "Being measured now" : l.stale ? "Out of date: measured again soon" : l.measured_ms ? `Measured ${ago(l.measured_ms)}` : "";
      li.append(bars);
    }
    li.append(el("div", "psays", says));
    list.append(li);
  }
}

// What the agents are doing: the quest log, the steps, the workers and the
// clips, folded under one heading with counts. Open, it doesn't redraw or
// reorder while the person reads it (the pointer on it) or types in it,
// and catches up once they move away.

/** Whether the person reads or types in WHAT THE AGENTS ARE DOING. */
function agentsHeld() {
  const d = $("agents");
  if (!LOOP_PAGE || !d?.open) return false;
  const f = document.activeElement;
  return d.matches(":hover") || (!!f && f !== d && d.contains(f) && f.tagName !== "SUMMARY");
}

function openAgents(on) {
  const d = $("agents");
  if (d) d.open = on;
}

/** The heading's counts: rows in work, rows waiting, rows done today. */
function drawAgentsHead() {
  if (!LOOP_PAGE) return;
  const open = rows.filter((r) => r.mechanic && !String(r.mechanic).startsWith("sealed") && r.status !== "done");
  const working = open.filter((r) => worked(r)).length;
  const waiting = open.length - working;
  const midnight = new Date().setHours(0, 0, 0, 0);
  const done = (board?.done_ms ?? []).filter((t) => t >= midnight).length;
  const text = `(${working} in work, ${waiting} waiting, ${done} done today)`;
  if ($("agents-counts").textContent !== text) $("agents-counts").textContent = text;
}

/** The lists drawn again once the person moved away from them. */
function agentsCatchUp() {
  if (agentsHeld()) return;
  $("agents-held").hidden = true;
  drawQueue();
  drawClips();
  stepsDrawn = "";
  workersDrawn = "";
  void refreshSteps();
}

if (LOOP_PAGE) {
  const d = $("agents");
  try { d.open = localStorage.getItem("gp-agents-open") === "1"; } catch (e) { /* no storage */ }
  d.addEventListener("toggle", () => {
    try { localStorage.setItem("gp-agents-open", d.open ? "1" : ""); } catch (e) { /* no storage */ }
    if (d.open) agentsCatchUp();
  });
  d.addEventListener("pointerleave", () => setTimeout(agentsCatchUp, 0));
  d.addEventListener("focusout", () => setTimeout(agentsCatchUp, 0));
}

async function refreshSteps() {
  if (stepsWriting) return;
  let v = null;
  try {
    const r = await fetch("/api/steps");
    if (r.ok) v = await r.json();
  } catch (e) { /* the next look */ }
  if (!v) return;
  openSteps = v.open ?? [];
  loopNow = v;
  drawWorkers(v);
  drawQueue();
  drawNowWork();
  drawYourTurn();
  drawProgress();
  const key = JSON.stringify([v.halted?.id ?? null, (v.open ?? []).map((s) => s.id)]);
  if (key === stepsDrawn || agentsHeld()) return;
  stepsDrawn = key;
  const halt = $("halt");
  halt.hidden = !v.halted;
  halt.textContent = v.halted?.says ?? "";
  const list = $("steps");
  // Cards opened stay open when the steps are drawn again.
  const opened = new Set([...list.querySelectorAll("li.expanded[data-step]")].map((li) => li.dataset.step));
  list.textContent = "";
  const WHO = { person: "YOU", agent: "YOUR AGENT", tools: "THE TOOLS" };
  const game = session?.game ?? "";
  const mine = (v.open ?? []).filter((s) => !game || (s.touches ?? []).some((t) => t === `game:${game}` || t.startsWith(`row:${game}/`)) || s.subject === game);
  for (const s of mine) {
    // A small card: who, the row it is for and one line of what; a click
    // shows all its words, its id, OPEN THE ROW and OBJECT TO THIS STEP.
    const li = el("li", "scard");
    li.dataset.step = s.id;
    const row = (s.touches ?? []).find((t) => t.startsWith(`row:${game}/`))?.slice(`row:${game}/`.length) ?? null;
    const head = el("div", "shead");
    head.append(el("span", `swho swho-${s.who}`, WHO[s.who] ?? (s.who ?? "").toUpperCase()));
    if (row) {
      const name = el("span", "srow", cardTitle(row));
      name.title = row;
      head.append(name);
    }
    li.append(head);
    const says = String(s.says ?? "");
    // "Work on ROW: WORDS" says the row twice beside its title: the words.
    const lead = `Work on ${row ?? ""}`;
    const rest = row && says.startsWith(lead) ? says.slice(lead.length).replace(/^[.:]\s*/, "") : says;
    li.append(el("div", "ssays", cardLine(rest || "Work on it.")));
    const more = el("div", "smore");
    more.append(el("div", "sfull", says), el("div", "sid", `step ${s.id}`));
    const acts = el("div", "sacts");
    if (row) {
      const open = el("button", "sopen", "OPEN THE ROW");
      open.type = "button";
      open.title = `Open ${row} in the quest log`;
      open.addEventListener("click", (e) => { e.stopPropagation(); toQuestLog(row); });
      acts.append(open);
    }
    li.classList.toggle("expanded", opened.has(s.id));
    li.title = "Click: show or hide the rest";
    li.addEventListener("click", (e) => {
      if (e.target.closest("form, button")) return;
      li.classList.toggle("expanded");
    });
    const b = el("button", "swrong", "OBJECT TO THIS STEP");
    b.type = "button";
    b.title = "Say why this step is wrong: the loop halts until a change to the tool fixes the rule";
    b.addEventListener("click", (e) => {
      e.stopPropagation();
      if (li.querySelector("form")) return;
      stepsWriting = true;
      const f = el("form");
      const why = el("input");
      why.placeholder = "What is wrong with this step?";
      why.setAttribute("aria-label", "What is wrong with this step");
      const send = el("button", "", "HALT THE LOOP");
      send.type = "submit";
      const keep = el("button", "", "CANCEL");
      keep.type = "button";
      keep.addEventListener("click", () => { f.remove(); stepsWriting = false; });
      f.addEventListener("submit", async (e) => {
        e.preventDefault();
        if (!why.value.trim()) { why.focus(); return; }
        send.disabled = true;
        try {
          await api(`/api/steps/${encodeURIComponent(s.id)}/wrong`, { why: why.value });
          note("The loop is halted: no step starts until a change to the tool that names your objection lands.", 8000);
        } catch (err) { fault(`It didn't save: ${err.message ?? err}`, 8000); }
        stepsWriting = false;
        stepsDrawn = "";
        void refreshSteps();
      });
      f.append(why, send, keep);
      more.append(f);
      why.focus();
    });
    acts.append(b);
    more.append(acts);
    li.append(more);
    list.append(li);
  }
  if (mine.length === 0 && !v.halted) list.append(el("li", "", "Nothing open for this game."));
  drawNowLoop(v);
}

// The runner's workers (serve/runner.rs): each with its step, its state
// and its last log line, and STOP while it works.
const LIVE = ["working", "attempting", "waiting for a yes", "retrying"];
let workersDrawn = "";
function drawWorkers(v) {
  const shown = (v.workers ?? []).filter((w, i) => LIVE.includes(w.state) || i < 5);
  const key = JSON.stringify([v.runner?.says, shown.map((w) => [w.step?.id, w.state, w.last, w.why])]);
  if (key === workersDrawn || agentsHeld()) return;
  workersDrawn = key;
  $("runner").textContent = [v.runner?.says, v.runner?.spending ? `Spending: ${v.runner.spending}.` : ""].filter(Boolean).join(" ");
  const list = $("workers");
  list.textContent = "";
  for (const w of shown) {
    const live = LIVE.includes(w.state);
    const li = el("li", live ? "live" : "");
    li.append(el("span", "wstate", (w.state ?? "").toUpperCase()));
    const st = w.step ?? {};
    li.append(el("span", "wsays", st.subject ? `${st.subject}: ${st.says}` : st.says ?? ""));
    li.append(el("span", "sid", st.id ?? ""));
    if (live) {
      const b = el("button", "wstop", "STOP");
      b.type = "button";
      b.title = "Stop this worker and what it started";
      b.addEventListener("click", async () => {
        b.disabled = true;
        try { await api(`/api/loop/workers/${encodeURIComponent(st.id)}/stop`, {}); }
        catch (err) { fault(`It didn't stop: ${err.message ?? err}`, 8000); }
        workersDrawn = "";
        void refreshSteps();
      });
      li.append(b);
    }
    const last = [w.why, w.last].filter(Boolean).join(". ");
    if (last) li.append(el("span", "wlast", last));
    list.append(li);
  }
}

// What is shown, and the agent's asks -----------------------------------

function setView(v, by = "person") {
  if (!["side", "top", "diff", "solo"].includes(v)) return;
  const was = show.view;
  show.view = v;
  // Live, the model alone plays from its own start and beside the
  // original it follows the original's presses: a change starts it again.
  if (mode === "live" && model.ready && (was === "solo") !== (v === "solo")) {
    model.say = "THE MODEL IS STARTING";
    void startModel(++model.token);
  }
  $("record").disabled = mode !== "live" || v === "solo" || sending;
  show.by = by;
  for (const b of document.querySelectorAll("#views button")) b.classList.toggle("on", b.dataset.view === v);
  $("opacity-pick").hidden = v !== "top";
  fitKey = "";
  keep();
}
function setOpacity(o, by = "person") {
  show.opacity = Math.min(1, Math.max(0, Number(o)));
  show.by = by;
  $("opacity").value = String(Math.round(show.opacity * 100));
  $("opacity-at").textContent = `${Math.round(show.opacity * 100)}%`;
  keep();
}
/** The model drawn in the game's own pictures or a theme: at once, no restart. */
function setPictures(p, by = "person") {
  const sel = $("pictures");
  if (![...sel.options].some((o) => o.value === p)) { note(`No theme ${p} for this game.`); return false; }
  show.pictures = p;
  show.by = by;
  sel.value = p;
  themeSeen = themeTimes[p] ?? null;
  modelPost({ gp: "theme", id: p === "game" ? "original" : p });
  keep();
  return true;
}

function keep() {
  try { localStorage.setItem("gp-screen", JSON.stringify({ view: show.view, opacity: show.opacity, pictures: show.pictures })); } catch (e) { /* not kept */ }
}

/** Says what the screen shows (POST api/screen), for screen_show. */
let lastReport = 0;
function report() {
  lastReport = performance.now();
  const v = {
    applied, by: show.by, mode, view: show.view, opacity: show.opacity, pictures: show.pictures,
    clip: clip?.n ?? null, mechanic: clip?.mechanic ?? null, frame: clip ? clip.f : null,
    work_clip: clip?.work ?? null, follow: follow.on,
    playing: clip ? clip.playing : mode === "live", recording: !!recording, error: askError,
    model: model.on ? "showing" : "starting",
    // The gate card whose change the model plays (its ask id), or null
    // for main's model.
    model_ask: model.ask,
    // How the model took the original's random draws: taken of the
    // script's, and the first desync; null while it rolls its own.
    draws: model.draws,
  };
  void api("api/screen", v).catch(() => undefined);
}

/** Applies an ask of the agent's: only the parts it set. */
async function applyAsk(a) {
  applied = a.seq;
  askError = null;
  if (a.view) setView(a.view, "agent");
  if (typeof a.opacity === "number") setOpacity(a.opacity, "agent");
  if (a.pictures) setPictures(a.pictures, "agent");
  show.by = "agent";
  try {
    if ("clip" in a && a.clip === null) backToLive();
    else if (typeof a.clip === "number") {
      agentHeldUntil = performance.now() + AGENT_HOLD_MS;
      await openClip(a.clip, a.mechanic, a.frame ?? 0);
    }
    else if (typeof a.frame === "number" && clip) seekClip(a.frame, true);
  } catch (e) {
    console.error(e);
    askError = String(e.message ?? e);
    // A clip that failed half way: back to live play.
    if (mode === "clip" && !clip) backToLive();
    fault(`Couldn't show what your agent asked: ${e.message ?? e}`, 15000);
  }
  show.by = "agent";
  report();
}

// A theme's newest file time, from the server: a change reloads the model
// in the theme as it is now.
let themeTimes = {};
let themeSeen = null;
let themeReload = 0;
let asking = false;
async function poll() {
  if (asking) return;
  asking = true;
  try {
    const s = await api("api/screen");
    themeTimes = s.themes ?? {};
    const a = s.ask;
    if (a && typeof a.seq === "number" && a.seq > applied) await applyAsk(a);
    void followWork(s.work);
    const t = themeTimes[show.pictures];
    if (show.pictures !== "game" && t !== undefined) {
      if (themeSeen === null) themeSeen = t;
      else if (t !== themeSeen && performance.now() - themeReload > 2000) {
        themeSeen = t;
        themeReload = performance.now();
        note(`The theme ${show.pictures} changed: showing it again.`, 8000);
        void loadModel(`The theme ${show.pictures} as it is now.`);
      }
    }
  } catch (e) { /* the next poll */ }
  finally { asking = false; }
  if (performance.now() - lastReport > 5000) report();
}

// The model changed: built again by itself, the old build playing until
// the new one is ready, then swapped in where the screen is.
let building = null; // seconds, while a build runs
let buildAskedAt = 0;
async function watchBuild() {
  let st;
  try { st = await api("api/rebuild/status"); } catch (e) { return; }
  if (!st.built) return;
  building = st.building;
  if (st.stale && st.building === null && performance.now() - buildAskedAt > (st.error ? 60000 : 5000)) {
    buildAskedAt = performance.now();
    await api("api/rebuild/build", {}).catch((e) => fault(`The model couldn't be built again: ${e.message ?? e}`, 8000));
    return;
  }
  // A gate card's change on the screen is kept: main's new build shows
  // once the screen is back on main's model.
  if (!model.ask && st.building === null && st.build && model.build && st.build !== model.build && model.ready) {
    model.build = st.build;
    await loadModel("The new version of the model is showing.");
  }
  if (st.error && st.building === null && st.stale) fault(`The new model didn't build: ${st.error.split("\n").pop()}`, 8000);
}

/** The usual line under the screens. */
function usualLine() {
  if (performance.now() < lineUntil) return;
  if (building !== null) { line(`The model changed: building the new version, ${Math.round(building)} s. The old one shows until it is ready.`); return; }
  if (sending) return;
  if (mode === "clip") { line(""); return; }
  if (show.view === "solo") { line(model.on ? "Playing the model alone, from its own start." : ""); return; }
  line(recording ? "Recording. Press RECORD again (or R) to send it to your agent." : model.on ? "Playing live: the model follows your presses." : "");
}

// The loop -------------------------------------------------------------

let acc = 0, last = performance.now();
function frame(now) {
  acc = Math.min(acc + (now - last), (1000 / FPS) * 4);
  last = now;
  try {
    while (acc >= 1000 / FPS) {
      acc -= 1000 / FPS;
      if (mode === "live" && show.view === "solo") {
        // The model alone takes every frame; the original holds.
        const pad = timeline.take(now - acc, performance.now());
        if (model.on && model.free) model.queue.push(pad);
      } else if (mode === "live") {
        // The buttons at this game frame's own time, not at whatever
        // moment the browser ran it: a late frame's catch-up steps each
        // read their own.
        const pad = timeline.take(now - acc, performance.now());
        if (pad) lastPlay = now;
        // The model takes the frames the original read the pad on; while
        // it starts they wait in the queue. Their draws go with them.
        const read = player.step(0, pad);
        liveDraws(read);
        if (read) model.queue.push(pad);
      } else if (clip?.playing) {
        // A followed refine's window loops; any other clip loops whole.
        const [lo, hi] = clip.loop ?? [0, clip.frames - 1];
        seekClip(clip.f + 1 > hi || clip.f < lo ? lo : clip.f + 1, true);
      }
    }
    // A controller's button while the screen follows the work.
    if (mode === "clip" && clip?.work && (gamepadBits(firstPad()) || (LAYOUT.keys2.length && gamepadBits(secondPad())))) personPressed();
    if (mode === "live" && show.view === "solo" && padNow()) lastPlay = now;
    if (mode === "live") {
      const n = draws.live;
      const more = n && n.base !== null && n.lines.length ? n.lines.join("") : undefined;
      if (model.on && (model.queue.length || more !== undefined)) {
        if (more !== undefined) n.lines = [];
        modelPost({ gp: "push", text: "gp-frames 1\n" + model.queue.map(framesLine).join("\n") + "\n", draws: more });
        model.queue = [];
      }
      if (show.view !== "solo") putOriginal(0);
    }
    const have = grabModel();
    drawPair(have);
    usualLine();
    showLastPress();
    document.body.dataset.frame = String(mode === "live" ? player.frame(0) : clip?.f ?? 0);
    document.body.dataset.model = model.on ? "on" : "off";
  } catch (e) {
    line(String(e));
  }
  requestAnimationFrame(frame);
}

// Keys and buttons -----------------------------------------------------

window.addEventListener("keydown", (e) => {
  // Typing to the agent: the keys are the box's.
  if (e.target?.id === "tell-text") {
    if (e.code === "Escape") e.target.blur();
    return;
  }
  if (e.target instanceof HTMLInputElement || e.target instanceof HTMLSelectElement) {
    if (e.code !== "Escape") return;
  }
  if (e.ctrlKey || e.metaKey || e.altKey) return;
  if (e.code === "KeyR" && !e.repeat) {
    e.preventDefault();
    // Following the work, R takes the screen back and records.
    if (mode === "clip" && clip?.work) personPressed();
    if (mode !== "live") return;
    if (recording) void stopRecording(true); else startRecording();
    return;
  }
  // Following the work, a game button other than the clip's own keys
  // (Esc, Space, Enter, Left, Right) goes back to playing.
  const clipKeys = ["Escape", "Space", "Enter", "ArrowLeft", "ArrowRight"];
  if (mode === "clip" && clip?.work && !clipKeys.includes(e.code) && (LAYOUT.keys.flat().includes(e.code) || LAYOUT.keys2.flat().includes(e.code))) {
    e.preventDefault();
    personPressed();
    down.add(e.code);
    return;
  }
  if (mode === "clip") {
    // Stepping or pausing a followed clip: the person is looking at it.
    if (clip?.work && e.code !== "Escape") show.by = "person";
    if (e.code === "Escape") { e.preventDefault(); show.by = "person"; backToLive(); drawClips(); }
    else if (e.code === "Space" || e.code === "Enter") { e.preventDefault(); if (!e.repeat) playClip(!clip?.playing); }
    else if (e.code === "ArrowLeft" || e.code === "ArrowRight") { e.preventDefault(); playClip(false); seekClip((clip?.f ?? 0) + (e.code === "ArrowLeft" ? -1 : 1)); }
    return;
  }
  if (LAYOUT.keys.flat().includes(e.code) || LAYOUT.keys2.flat().includes(e.code)) e.preventDefault();
  down.add(e.code);
}, { capture: true });
window.addEventListener("keyup", (e) => { down.delete(e.code); noteInput(e.timeStamp); });
window.addEventListener("blur", () => { down.clear(); noteInput(performance.now()); });
// After the handler above has set the keys that are down, in the same turn.
window.addEventListener("keydown", (e) => noteInput(e.timeStamp));

function wire() {
  for (const b of document.querySelectorAll("#views button")) {
    b.addEventListener("click", () => { b.blur(); setView(b.dataset.view); report(); });
  }
  $("opacity").addEventListener("input", (e) => setOpacity(Number(e.target.value) / 100));
  $("opacity").addEventListener("change", (e) => { e.target.blur(); report(); });
  $("pictures").addEventListener("change", (e) => { e.target.blur(); if (setPictures(e.target.value)) report(); });
  $("record").addEventListener("click", (e) => {
    e.currentTarget.blur();
    if (recording) void stopRecording(true); else startRecording();
  });
  $("record-seal").addEventListener("click", (e) => {
    e.currentTarget.blur();
    if (recording) void stopRecording(true, true);
  });
  $("clip-play").addEventListener("click", (e) => { e.currentTarget.blur(); playClip(!clip?.playing); });
  $("clip-scrub").addEventListener("input", (e) => { playClip(false); seekClip(Number(e.target.value), true); });
  $("clip-scrub").addEventListener("change", (e) => { e.target.blur(); show.by = "person"; report(); });
  $("clip-live").addEventListener("click", (e) => { e.currentTarget.blur(); show.by = "person"; backToLive(); drawClips(); });
  // BEFORE: main's model at the card's clip; AFTER: the card's change again.
  $("clip-before").addEventListener("click", (e) => {
    e.currentTarget.blur();
    if (!model.card || mode !== "clip") return;
    show.by = "person";
    useModel(model.ask ? null : model.card);
    void loadModel(model.ask ? "The model after this change, as the card would land it." : "Main's model, before this change.");
  });
  $("follow-on").addEventListener("click", (e) => { e.currentTarget.blur(); setFollow(!follow.on); report(); });
  // The quest log's box: Enter (NEW ITEM) makes a new item; SEND comments.
  $("tell").addEventListener("submit", (e) => { e.preventDefault(); void newRow(); });
  $("tell-send").addEventListener("click", (e) => { e.currentTarget.blur(); void tell(e); });
  holdWhilePressed($("queue"));
  holdWhilePressed($("clips"));
}

async function setUpPictures() {
  let themes = [];
  try { themes = (await api("rebuild/themes/index.json")).themes ?? []; } catch (e) { /* the game's own only */ }
  const sel = $("pictures");
  for (const t of themes) {
    const o = document.createElement("option");
    o.value = t.id;
    o.textContent = `THEME: ${t.id.toUpperCase()}`;
    sel.append(o);
  }
}

async function main() {
  wire();
  // PROGRESS first (progress-shows-at-once): it says it is reading until
  // the scoreboard's first answer, asked for now, while the player, the
  // ROM and the session load.
  if (LOOP_PAGE) {
    drawProgress();
    void refreshScoreboard();
    setInterval(() => { if (!document.hidden) void refreshScoreboard(); }, 20000);
  }
  line("Loading the game.");
  player = await NesPlayer.load("gp_play.wasm");
  const r = await fetch("rom");
  if (!r.ok) throw new Error(`the ROM: ${await r.text()}`);
  rom = new Uint8Array(await r.arrayBuffer());
  // The model's frame sounds an arcade model through the board from the
  // ROM this page holds (preview.js asks parent.__gpRom).
  window.__gpRom = () => rom;
  if (player.x.gp_sets_load) {
    const sets = await fetch("roms.toml");
    if (sets.ok) player.sets(await sets.text());
  }
  player.power(0, rom);
  const info = player.info(0);
  LAYOUT = LAYOUTS[info.platform] ?? LAYOUTS.nes;
  FPS = info.fps ? info.fps[0] / info.fps[1] : LAYOUT.fps;
  W = info.width ?? 256;
  H = info.height ?? 240;
  for (const c of [$("a"), $("b"), offA, offB]) { c.width = W; c.height = H; }
  $("model-frame").style.width = `${W}px`;
  $("model-frame").style.height = `${H}px`;
  $("keys").textContent = LAYOUT.footer;
  wireInput();
  session = await api("api/session").catch(() => null);
  // The set the page plays live; a clip plays on its own (romFor).
  const sets = await api("api/rom").catch(() => null);
  if (sets?.set) romSet = { set: sets.set, system: (sets.sets ?? []).find((s) => s.id === sets.set)?.system ?? sets.set };
  $("title").textContent = session?.game ?? "";
  if (romSet && session?.game) {
    $("title").textContent = `${session.game} ON ${romSet.set}`;
    $("title").title = "The set the screens play live; a clip plays on the set it was recorded on, named on the clip bar";
  }
  document.title = `${session?.game ?? "Game"} ${LOOP_PAGE ? "loop" : "screen"}`;
  await setUpPictures();
  await refreshLists(true);
  // The loop's page opened at a row (the status line's or a card's link).
  const at = LOOP_PAGE ? new URLSearchParams(location.search).get("row") : null;
  if (at) toQuestLog(at);
  // The view kept in this browser, then the agent's last ask when no page
  // has shown it yet and it is recent (asked before the page opened); one
  // shown already or old is taken as seen.
  try {
    const k = JSON.parse(localStorage.getItem("gp-screen") ?? "null");
    if (k) {
      setView(k.view ?? "side");
      setOpacity(k.opacity ?? 0.5);
      if (k.pictures && [...$("pictures").options].some((o) => o.value === k.pictures)) { show.pictures = k.pictures; $("pictures").value = k.pictures; }
    } else setView("side");
  } catch (e) { setView("side"); }
  // A link's own choice wins (THEMES's SEE IT IN THE GAME opens the model
  // alone in the theme): ?pictures=ID&view=solo.
  const asked = new URLSearchParams(location.search);
  if (asked.get("view")) setView(asked.get("view"));
  if (asked.get("pictures") && [...$("pictures").options].some((o) => o.value === asked.get("pictures"))) {
    show.pictures = asked.get("pictures");
    $("pictures").value = show.pictures;
  }
  setOpacity(show.opacity);
  const first = await api("api/screen").catch(() => null);
  themeTimes = first?.themes ?? {};
  themeSeen = themeTimes[show.pictures] ?? null;
  drawFollow();
  const a = first?.ask;
  requestAnimationFrame(frame);
  if (a && typeof a.seq === "number") {
    const unseen = (first?.shown?.applied ?? 0) < a.seq;
    if (unseen && Date.now() - (a.at_ms ?? 0) < 10 * 60 * 1000) await applyAsk(a);
    else applied = a.seq;
  }
  if (mode === "live") await loadModel();
  report();
  setInterval(() => void poll(), 1000);
  setInterval(() => void watchBuild(), 4000);
  setInterval(() => { if (!document.hidden) void refreshLists(); }, 3000);
  void refreshAsks();
  setInterval(() => { if (!document.hidden) void refreshAsks(); }, 2000);
  void refreshSteps();
  setInterval(() => { if (!document.hidden) void refreshSteps(); }, 3000);
  // The loop's page asked at the start; one with no game in its address
  // asks again now the session names it.
  if (!LOOP_PAGE || !URL_GAME) void refreshScoreboard();
  if (!LOOP_PAGE) setInterval(() => { if (!document.hidden) void refreshScoreboard(); }, 20000);
}

main().catch((e) => {
  const why = e instanceof Error ? e.message : String(e);
  line(`The screen couldn't start: ${why}`);
  document.body.dataset.failed = why;
});
