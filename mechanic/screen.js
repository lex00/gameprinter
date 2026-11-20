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
// Under the screens, the quest log (the rows `gp mechanic queue GAME`
// prints), grouped NEEDS YOU, AGENT WORKING, UP NEXT, DONE, and the clips
// of every row. A recording goes to the row outlined: the open one when it
// starts. A row's RECORD INTO THIS ROW moves it there (api/pick when not
// recording); a click elsewhere on the row only opens its details, the
// RAM tests, addresses and hashes. A clip clicked shows on both screens.

import { NesPlayer } from "./nesplayer.js";

const $ = (id) => document.getElementById(id);

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
    footer: "5 drops a coin and 1 starts. Arrows move, X or Space taps. On a controller: Select is the coin, Start starts, A taps. R records.",
  },
};
let LAYOUT = LAYOUTS.nes;
let FPS = LAYOUT.fps;
let W = 256, H = 240;

let player = null, rom = null;
let session = null; // api/session: the game and the mechanic open
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
};

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
    if (b(0) || b(1) || b(2) || b(3)) pad |= 64;
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
  else if (m?.gp === "error") note(`The model: ${m.message}`, 6000);
});

/** The theme's part of the model's address: large art is drawn bigger than
 * the board, so it is fitted. */
function lookParam() {
  return show.pictures !== "game" ? `&theme=${encodeURIComponent(show.pictures)}&scale=fit` : "";
}

/** The rebuild built once at least: builds it first when it isn't, saying
 * how long it has taken. */
async function ensureBuilt() {
  let st = await api("api/rebuild/status");
  if (st.built) return st;
  if (!st.model) throw new Error("this game has no model yet");
  await api("api/rebuild/build", {});
  while ((st = await api("api/rebuild/status")).building !== null) {
    model.say = "BUILDING THE MODEL";
    line(`Building the playable model: ${Math.round(st.building)} s. The first build takes a few minutes. Play on meanwhile.`);
    await new Promise((r) => setTimeout(r, 2000));
  }
  if (st.error || !st.built) throw new Error(st.error ?? "the build didn't finish");
  return st;
}

/** Loads the model's page into its frame (again), then starts it where
 * the screen is: live at the original's frame, or at the clip's. The last
 * picture of the model stays on screen until the new one is on. */
async function loadModel(why) {
  const mine = ++model.token;
  // How many times the model was loaded, for a check of the page.
  document.body.dataset.loads = String(Number(document.body.dataset.loads ?? 0) + 1);
  model.ready = false;
  model.on = false;
  try {
    const st = await ensureBuilt();
    if (mine !== model.token) return;
    model.build = st.build ?? null;
    try {
      const pj = await api("rebuild/preview.json");
      model.visible = Array.isArray(pj.visible) ? pj.visible : null;
    } catch (e) { model.visible = null; }
    const f = $("model-frame");
    await new Promise((resolve, reject) => {
      const t = setTimeout(() => reject(new Error("the model didn't load")), 30000);
      model.onReady = () => { clearTimeout(t); resolve(); };
      // files=here: the preview reads the build beside it, not the
      // browser's store of installed games.
      f.src = `rebuild/index.html?control=1&files=here${lookParam()}&t=${Date.now()}`;
    });
    if (mine !== model.token) return;
    await startModel(mine);
    if (why && mine === model.token) note(why);
  } catch (e) {
    if (mine !== model.token) return;
    model.say = "THE MODEL COULDN'T START";
    note(`The model couldn't start: ${e.message ?? e}`, 10000);
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
  try {
    const path = await sendPath(player.inputLog(0).split("\n").slice(0, frame + 2).join("\n") + "\n");
    const r = await api("api/rebuild/start", { path, frame });
    if (mine !== model.token || mode !== "live" || show.view === "solo") return;
    const frames = "gp-frames 1\n" + model.queue.map(framesLine).join("\n");
    model.queue = [];
    modelPost({ gp: "load-state", state: r.state, text: frames + "\n", frame: 1e9 });
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
    const r = await api("api/rebuild/start", { path: c.path, frame: c.start });
    if (mine !== model.token || clip !== c) return;
    const text = "gp-frames 1\n" + c.polled.map(framesLine).join("\n") + "\n";
    modelPost({ gp: "load-state", state: r.state, text, frame: c.upTo[c.f] ?? 0 });
    model.on = true;
  } catch (e) {
    if (mine !== model.token || clip !== c) return;
    model.say = "THE MODEL CAN'T START AT THIS CLIP";
    note(`The model can't start at the clip's start: ${String(e.message ?? e).slice(0, 160)}`, 8000);
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
  $("cap-a").textContent = side ? "THE ORIGINAL" : show.view === "top" ? `THE MODEL OVER THE ORIGINAL, ${Math.round(show.opacity * 100)}%` : show.view === "solo" ? `THE MODEL ALONE${pics}` : "THE DIFFERENCE: BLACK WHERE THEY MATCH";
  $("cap-b").textContent = `THE MODEL${pics}`;
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

/** Shows clip `n` of `mechanic` at frame `f`: the original replayed in the
 * player's second console to the clip's start, every frame of the clip
 * played once to learn which frames read the pad, then both games stood at
 * `f`. */
async function openClip(n, mechanic, f) {
  if (recording) stopRecording(false);
  const key = `${mechanic ?? ""}/${n}`;
  if (clip && clip.key === key) { seekClip(f); return; }
  const q = mechanic ? `&mechanic=${encodeURIComponent(mechanic)}` : "";
  const meta = await api(`api/screen/clip?n=${n}${q}`);
  if (mode === "clip" && clip) backToClipless();
  const r = await fetch(`api/clips/${n}.log?mechanic=${encodeURIComponent(meta.mechanic)}`);
  if (!r.ok) throw new Error(`clip ${n}: ${r.status}`);
  const log = await r.text();
  await showClipLog({ key, n, mechanic: meta.mechanic, work: null, name: `CLIP ${n} OF ${meta.mechanic.toUpperCase()}`, says: `clip ${n} of ${meta.mechanic}`, note: meta.note ?? "", start: meta.start, frames: meta.frames, log }, f);
}

/** Shows the corpus clip a refine works on (api/screen/work-clip), looping
 * the window's frames [lo, hi] from the clip's start. */
async function openWorkClip(id, lo, hi) {
  const meta = await api(`api/screen/work-clip?id=${encodeURIComponent(id)}`);
  if (mode === "clip" && clip) backToClipless();
  await showClipLog({ key: `work/${id}`, n: null, mechanic: null, work: id, name: `THE CLIP YOUR AGENT REFINES: ${id.toUpperCase()}`, says: `corpus clip ${id}`, note: meta.name ?? "", start: meta.start, frames: meta.frames, log: meta.log }, lo);
  if (clip?.work === id) { clip.loop = [lo, Math.max(lo, hi)]; playClip(true); }
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
  player.replay(1, rom, log);
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
  const polled = [], upTo = [];
  for (let i = 0; i < length; i++) {
    if (player.step(1, pads[i])) polled.push(pads[i]);
    upTo.push(polled.length);
    if ((i + 1) % SNAP_EVERY === 0) snaps.set(i + 1, player.save(1));
  }
  clip = { key: c.key, n, mechanic, work: c.work, loop: null, note: c.note, start, frames: length, log, pads, polled, upTo, snaps, f: 0, pos: length, playing: false, path: null };
  $("clip-scrub").max = String(length - 1);
  $("clip-name").textContent = c.name;
  $("clipbar").hidden = false;
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
    note(`Couldn't show the clip your agent refines (${wk.clip}): ${e.message ?? e}`, 8000);
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
  // The play so far goes up as the recording starts, so sending later
  // waits on no replay (app.js's warmPath).
  warming = sendPath(player.inputLog(0), recording.start).catch(() => undefined);
  note(`Recording into ${clipFor ?? "the open row"}. Press RECORD again (or R) to send it to your agent.`, 3000);
}

/** Ends the recording; with `send`, hands the clip to the open mechanic
 * as CLIPS's hand-off does: the play as a path, the clip on it (POST
 * api/clips), then a turn in the talk naming it (POST api/talk). */
async function stopRecording(send = true) {
  const r = recording;
  recording = null;
  $("record").classList.remove("on");
  $("record").textContent = "RECORD";
  if (!r || !send) { markTarget(); return; }
  const log = player.inputLog(0);
  const frames = player.frame(0) - r.start;
  if (frames < 10) { note("Too short to keep: under 10 frames."); return; }
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
    const { n, again } = await api("api/clips", { path, start: r.start, inputs, note: "", mechanic });
    await api("api/talk", { text: `Here is clip ${n}.`, clips: [n], mechanic });
    saved = { mechanic, n };
    note(again
      ? `Already saved: this is clip ${n} of ${mechanic}, under ${String(mechanic).toUpperCase()} in CLIPS below.`
      : `Saved as clip ${n} of ${mechanic}: it is under ${String(mechanic).toUpperCase()} in CLIPS below, and your agent has it.`, 15000);
    await refreshLists(true);
  } catch (e) {
    note(`The clip didn't reach your agent: ${e.message ?? e}`, 10000);
  } finally {
    sending = false;
    clipFor = null;
    markTarget();
    $("record").disabled = mode !== "live" || show.view === "solo";
  }
}

// The quest log and the clips --------------------------------------------

// What the queue's statuses show as (app.js's LABELS).
const LABELS = { done: "MEASURED", "asking you": "YOUR MOVE", "agent working": "WORKING ON IT", "in line": "IN LINE", next: "NEXT" };
let rows = []; // the quest log, as api/session gives it
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
        await api("api/talk", { text, clips, mechanic });
        f.remove();
        note(clips.length ? `Your comment on clip ${clips[0]} of ${mechanic} went to your agent.` : `Your comment on ${mechanic} went to your agent.`, 5000);
        await refreshLists(true);
      } catch (err) {
        go.disabled = false;
        note(`Your comment didn't save: ${err.message ?? err}`, 8000);
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
const GROUPS = [
  ["you", "NEEDS YOU"],
  ["working", "AGENT WORKING"],
  ["next", "UP NEXT"],
  ["done", "DONE"],
];
function groupOf(r) {
  if (r.triage?.by === "you" || r.status === "asking you") return "you";
  if (r.status === "agent working") return "working";
  if (r.status === "done") return "done";
  return "next";
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
  const key = JSON.stringify([session?.mechanic, rows, clipsBy.map((g) => [g.mechanic, (g.talk ?? []).length])]);
  if (q.dataset.key === key) { markTarget(); return; }
  // Details opened stay open when the log is drawn again.
  const opened = new Set([...q.querySelectorAll("li[data-mechanic]")].filter((li) => [...li.querySelectorAll("details")].some((d) => d.open)).map((li) => li.dataset.mechanic));
  q.dataset.key = key;
  q.textContent = "";
  try { q.classList.toggle("done-open", localStorage.getItem("gp-done-open") === "1"); } catch (e) { /* no storage */ }
  const shown = rows.filter((r) => r.mechanic && !String(r.mechanic).startsWith("sealed"));
  for (const [g, title] of GROUPS) {
    const of = shown.filter((r) => groupOf(r) === g);
    if (!of.length) continue;
    const head = el("li", `qgroup qgroup-${g}`, `${title} (${of.length})`);
    if (g === "done") {
      head.title = "Finished rows, archived here";
      head.addEventListener("click", () => {
        q.classList.toggle("done-open");
        try { localStorage.setItem("gp-done-open", q.classList.contains("done-open") ? "1" : ""); } catch (e) { /* no storage */ }
      });
    }
    q.append(head);
    for (const r of of) q.append(queueRow(r, g, opened.has(r.mechanic)));
  }
  if (!q.children.length) q.append(el("li", "", "No rows yet: tell your agent what to put in the quest log."));
  markTarget();
}

/** A quest-log row. A click opens its details; only RECORD INTO THIS ROW
 * moves where the next recording goes. */
function queueRow(r, g, open) {
  const li = el("li", `qrow-${g}`);
  li.dataset.mechanic = r.mechanic;
  const label = r.triage?.by === "you" ? "NEEDS YOU" : r.triage?.by === "tools" ? "THE TOOLS TRY" : LABELS[r.status] ?? String(r.status ?? "").toUpperCase();
  li.append(el("span", `st st-${String(r.status ?? "").replace(/ /g, "-")}`, label), el("span", "qname", r.mechanic));
  if (g === "working") {
    li.classList.add("active");
    li.append(el("span", "qworker", r.worker ? `ACTIVE: ${String(r.worker).toUpperCase()}` : "ACTIVE"));
  }
  li.append(el("span", "qto", ""));
  const here = el("button", "qhere", "RECORD INTO THIS ROW");
  here.type = "button";
  here.title = "Recordings go to this row from now on";
  here.addEventListener("click", (e) => { e.stopPropagation(); e.currentTarget.blur(); void pickRow(r.mechanic); });
  li.append(here);
  if (r.scores) li.append(noteLine("SCORES", r.scores, "qscore"));
  // Who wrote it: the person's words from the page (by: person) stay theirs
  // (asked) once the agent's note takes the row over.
  if (r.asked) li.append(noteLine("YOU ASKED", r.asked));
  if (r.note) li.append(noteLine(r.by === "person" && !r.asked ? "YOU WROTE" : "AGENT WROTE", r.note));
  const turns = talkOf(r.mechanic);
  if (turns.length) li.append(talkLines(turns));
  li.append(commentBox(li, r.mechanic, [], `A comment on ${r.mechanic} for your agent`));
  if (r.mechanic === session?.mechanic) li.classList.add("open");
  for (const d of li.querySelectorAll("details")) d.open = open;
  li.title = "Click: show or hide its details (RAM tests, addresses, hashes)";
  li.addEventListener("click", () => {
    const ds = [...li.querySelectorAll("details")];
    const to = !ds.some((d) => d.open);
    for (const d of ds) d.open = to;
  });
  return li;
}

function drawClips() {
  const box = $("clips");
  if (pressing) return;
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
        void openClip(c.n, g.mechanic, 0).then(() => { drawClips(); report(); }).catch((e) => note(`Couldn't open clip ${c.n}: ${e.message ?? e}`, 8000));
      });
      ul.append(li);
    }
    box.append(ul);
  }
  if (!box.children.length) box.append(el("p", "empty", "No clips yet. RECORD makes one; your agent's clips show here too."));
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
  } catch (e) { note(`Couldn't open ${m}: ${e.message ?? e}`, 8000); }
}

/** The quest log and the clips, again from the server. */
let listing = false;
async function refreshLists(force) {
  if (listing && !force) return;
  listing = true;
  try {
    const [s, c] = await Promise.all([api("api/session"), api("api/screen/clips")]);
    session = s;
    rows = s.queue ?? [];
    clipsBy = c.mechanics ?? [];
    drawQueue();
    drawClips();
    drawNowWork();
    drawYourTurn();
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
  } catch (err) { note(`The item didn't save: ${err.message ?? err}`, 8000); }
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
  } catch (err) { note(`Your agent didn't get it: ${err.message ?? err}`, 8000); }
  finally { $("tell-send").disabled = false; }
}

// What the agent waits for the person's yes on -------------------------
//
// The asks gp_run left (gp_mcp::asks, GET /api/asks), as cards in YOUR
// TURN, the strip over the window's bottom right corner (out of the
// page's flow, so the screens never move): one line a card, its question
// and YES and NO (POST /api/asks/ID); a click on the question opens what
// it does, the command and its message. A yes'd card is one line in the
// runner's turn (RUNNING NOW, WAITS n OF m), and one the runner finished
// says LANDED, DONE or FAILED with why, then leaves a minute later.

/** How long a card the runner finished stays on the strip. */
const ASK_DONE_SHOWS_MS = 60 * 1000;
const asksOpen = new Set();
let asksDrawn = "";
let asksShown = 0; // the yes cards in YOUR TURN now
let asksNow = []; // those cards, as /api/asks last gave them
async function refreshAsks() {
  let list = [];
  const now = Date.now();
  try {
    const r = await fetch("/api/asks");
    if (r.ok) list = ((await r.json()).asks ?? []).filter((a) => a.ran
      ? a.ran.state === "running" || now - (a.ran.done_ms ?? a.ran.at_ms ?? 0) < ASK_DONE_SHOWS_MS
      : a.state === "open" || a.state === "yes");
  } catch (e) { /* the next look */ }
  asksShown = list.length;
  asksNow = list;
  drawNowWork();
  // The runner's turn: what it runs now, then the yeses in the order they
  // were said, which it runs one at a time.
  const turn = [
    ...list.filter((a) => a.ran?.state === "running").sort((x, y) => (x.ran.at_ms ?? 0) - (y.ran.at_ms ?? 0)),
    ...list.filter((a) => !a.ran && a.state === "yes").sort((x, y) => (x.answered_ms ?? 0) - (y.answered_ms ?? 0)),
  ];
  // Open questions first, then the runner's turn, then what it finished.
  const rank = (a) => (!a.ran && a.state === "open" ? 0 : turn.includes(a) ? 1 : 2);
  list.sort((x, y) => rank(x) - rank(y) || (rank(x) === 1 ? turn.indexOf(x) - turn.indexOf(y) : 0));
  const key = JSON.stringify(list.map((a) => [a.id, a.state, a.ran?.state, turn.indexOf(a), asksOpen.has(a.id)]));
  if (key === asksDrawn) { drawYourTurn(); return; }
  asksDrawn = key;
  const box = $("asks");
  box.textContent = "";
  box.hidden = list.length === 0;
  drawYourTurn();
  for (const a of list) {
    const card = el("div", `ask-card${a.ran || a.state === "yes" ? " ran" : ""}${asksOpen.has(a.id) ? " open" : ""}`);
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
    let question = what ? `${verb} in ${game || "the game"}: ${what}?` : `${a.says ?? ""}`;
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
    const step = a.step ? openSteps.find((x) => x.id === a.step) : null;
    const row = (step?.touches ?? []).find((t) => t.startsWith("row:"))?.split("/").slice(1).join("/") ?? null;
    if (row) {
      const go = el("button", "ask-row", `FOR ${row.toUpperCase()}`);
      go.type = "button";
      go.title = "Open this quest-log item";
      go.addEventListener("click", (e) => {
        e.currentTarget.blur();
        void pickRow(row);
        $("quest")?.scrollIntoView({ behavior: "smooth", block: "start" });
      });
      card.append(go);
    } else {
      card.append(el("span", "ask-detail", a.step
        ? `For step ${a.step}, no longer open.`
        : "Raised outside the loop: no quest-log item, no step."));
    }
    card.append(el("span", "ask-detail", `gp ${a.command ?? ""}`));
    if (a.ran) {
      for (const l of a.ran.lines ?? []) card.append(el("span", "ask-detail", l));
    } else if (a.state === "yes") {
      card.append(el("span", "ask-detail", "The loop's runner runs the yeses one at a time, in the order you said them, while it is on."));
    } else {
      for (const [word, val] of [["YES", "yes"], ["NO", "no"]]) {
        const b = el("button", `ask-${val}`, word);
        b.type = "button";
        b.title = val === "yes" ? "Let your agent do this, once" : "Your agent won't do this";
        b.addEventListener("click", async (e) => {
          e.currentTarget.blur();
          for (const x of card.querySelectorAll("button")) x.disabled = true;
          try {
            const r = await fetch(`/api/asks/${encodeURIComponent(a.id)}`, {
              method: "POST",
              headers: { "content-type": "application/json" },
              body: JSON.stringify({ answer: val }),
            });
            if (!r.ok) says.textContent = `${a.says} (Your answer didn't save: ${(await r.json().catch(() => ({}))).error ?? r.status}.)`;
          } catch (err) {
            says.textContent = `${a.says} (Your answer didn't save. Is the page's server still running?)`;
          }
          asksDrawn = "";
          void refreshAsks();
        });
        card.append(b);
      }
    }
    box.append(card);
  }
}

// Your turn ------------------------------------------------------------
//
// At the top, only what the person can do now, each with a button that
// does it: the yes cards above (#asks), the loop's steps for this game
// that are the person's and have a button (PLAY FROM HERE: the row's start
// point, recording into the row), and the quest-log rows asking them
// (ANSWER: their words to the row's talk). With none of them, NOTHING
// NEEDS YOU.

let turnDrawn = "";
function drawYourTurn() {
  const list = $("turn");
  // A box being typed in stays until it is sent or closed.
  if (list.querySelector("form.comment")) return;
  const game = session?.game ?? "";
  const plays = openSteps.filter((s) => s.who === "person" && s.act?.do === "play_from"
    && (!game || (s.touches ?? []).includes(`row:${game}/${s.act.row}`)));
  const played = new Set(plays.map((s) => s.act.row));
  const asking = rows.filter((r) => r.status === "asking you" && r.mechanic && !played.has(r.mechanic));
  const key = JSON.stringify([plays.map((s) => [s.id, s.says]), asking.map((r) => [r.mechanic, r.note]), asksShown]);
  if (key === turnDrawn) return;
  turnDrawn = key;
  list.textContent = "";
  for (const s of plays) {
    const li = el("li", "turn-item");
    li.dataset.step = s.id;
    li.append(el("span", "turn-row", s.act.row.toUpperCase()), el("span", "turn-says", s.says));
    const b = el("button", "turn-play", "PLAY FROM HERE");
    b.type = "button";
    b.title = `Plays from ${s.act.row}'s start point and records into it: play the stretch, then RECORD (or R) sends it`;
    b.addEventListener("click", (e) => { e.currentTarget.blur(); void playFrom(s.act.row); });
    li.append(b);
    list.append(li);
  }
  for (const r of asking) {
    const li = el("li", "turn-item");
    li.dataset.mechanic = r.mechanic;
    li.append(el("span", "turn-row", r.mechanic.toUpperCase()), el("span", "turn-says", r.note || "Your agent asks you about this row."));
    li.append(commentBox(li, r.mechanic, [], `Your answer on ${r.mechanic} for your agent`, "ANSWER"));
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
      player.replay(0, rom, await res.text());
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
    note(`Couldn't load the start point: ${e.message ?? e}`, 8000);
  } finally {
    if (mode === "loading") mode = "live";
    starting = null;
  }
}

// Now ------------------------------------------------------------------
//
// At the top of the page: the quest-log items being worked on (status
// "agent working"), by whom, with the open row's status line (its busy
// text), and the loop's switches: the runner, its workers, whether
// gp_run needs a step, and a halt. With no worker running and no step
// able to start, it says why in plain words: THE LOOP IS WAITING ON YOU
// (with each card and row asking the person, a click away), THE LOOP IS
// STOPPED: THE RUNNER IS OFF, THE LOOP IS HALTED, or THE LOOP HAS
// NOTHING TO DO.

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
    waits.push({ label: r.mechanic.toUpperCase(), title: `${r.note ?? ""}\nClick to open this item`, go: () => {
      void pickRow(r.mechanic);
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
  const working = rows.filter((r) => r.status === "agent working");
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
  const key = JSON.stringify([lines.map((l) => [l.r.mechanic, l.r.worker, l.r.note, l.there, l.last]), session?.mechanic, session?.busy, wk && [wk.clip, wk.from, wk.to, wk.says],
    idle && [idle.head, idle.why, idle.waits.map((x) => x.label)]]);
  if (box.dataset.key === key) return;
  box.dataset.key = key;
  box.textContent = "";
  // A refine running: the clip and the frames it fits, which the screens
  // follow (FOLLOW THE WORK).
  if (wk) {
    const line = el("div", "now-item");
    const [lo, hi] = workRange(wk);
    line.append(el("span", "now-label", "REFINING"), el("span", "now-row", `${wk.clip}, frames ${lo} to ${hi}`));
    if (wk.says) line.append(el("span", "now-status", wk.says));
    box.append(line);
  }
  // No worker runs and no step can start: say so, and why, so a quiet
  // loop never looks like one that stopped by itself.
  if (idle && !wk) {
    const line = el("div", `now-item now-stopped now-stopped-${idle.kind}`);
    line.append(el("span", "now-label", idle.head));
    if (idle.why) line.append(el("span", "now-who", idle.why));
    for (const x of idle.waits) {
      const b = el("button", "now-row now-wait", x.label);
      b.type = "button";
      b.title = x.title;
      b.addEventListener("click", (e) => { e.currentTarget.blur(); x.go(); });
      line.append(b);
    }
    box.append(line);
  }
  if (!working.length) {
    if (wk || idle) return;
    box.append(el("span", "now-idle", "NOBODY IS WORKING ON AN ITEM NOW"));
    return;
  }
  for (const { r, w, there, last } of lines) {
    const line = el("div", `now-item${w ? "" : " now-absent"}`);
    const name = el("button", "now-row", r.mechanic.toUpperCase());
    name.type = "button";
    name.title = "Open this item in the quest log";
    name.addEventListener("click", (e) => { e.currentTarget.blur(); void pickRow(r.mechanic); $("quest")?.scrollIntoView({ behavior: "smooth", block: "start" }); });
    line.append(el("span", "now-label", w ? "WORKING ON" : "MARKED WORKING"), name, el("span", "now-who", `${there}, ${last}`));
    const status = r.mechanic === session?.mechanic && session?.busy ? session.busy : r.note ?? "";
    if (status) line.append(el("span", "now-status", status));
    box.append(line);
  }
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
      note(`Couldn't show clip ${n} your agent named: ${e.message ?? e}`, 8000);
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
// each with who does it, its words and its id, and WRONG: the person's
// words on why, sent as POST /api/steps/ID/wrong, which halts the loop
// until a change to the tool that names the objection lands. While it is
// halted the list is empty and the halt says why.

let stepsDrawn = "";
let openSteps = []; // the open steps as /api/steps last gave them
let stepsWriting = false; // a WRONG box is open: don't redraw under it
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
  drawNowWork();
  drawYourTurn();
  const key = JSON.stringify([v.halted?.id ?? null, (v.open ?? []).map((s) => s.id)]);
  if (key === stepsDrawn) return;
  stepsDrawn = key;
  const halt = $("halt");
  halt.hidden = !v.halted;
  halt.textContent = v.halted?.says ?? "";
  const list = $("steps");
  list.textContent = "";
  const WHO = { person: "YOU", agent: "YOUR AGENT", tools: "THE TOOLS" };
  const game = session?.game ?? "";
  const mine = (v.open ?? []).filter((s) => !game || (s.touches ?? []).some((t) => t === `game:${game}` || t.startsWith(`row:${game}/`)) || s.subject === game);
  for (const s of mine) {
    const li = el("li");
    const row = (s.touches ?? []).find((t) => t.startsWith(`row:${game}/`))?.slice(`row:${game}/`.length) ?? null;
    li.append(el("span", `swho swho-${s.who}`, WHO[s.who] ?? (s.who ?? "").toUpperCase()));
    li.append(el("span", "ssays", s.says));
    if (row) {
      li.append(el("span", "srow", `for ${row}`));
      li.classList.add("srow-link");
      li.title = `Open ${row} in the quest log`;
      li.addEventListener("click", (e) => {
        if (e.target.closest("form, button")) return;
        void pickRow(row);
        $("quest")?.scrollIntoView({ behavior: "smooth", block: "start" });
      });
    } else {
      li.title = `Step ${s.id}`;
    }
    const b = el("button", "swrong", "WRONG");
    b.type = "button";
    b.title = "This step is wrong: say why, and the loop halts until a change to the tool fixes the rule";
    b.addEventListener("click", () => {
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
        } catch (err) { note(`It didn't save: ${err.message ?? err}`, 8000); }
        stepsWriting = false;
        stepsDrawn = "";
        void refreshSteps();
      });
      f.append(why, send, keep);
      li.append(f);
      why.focus();
    });
    li.append(b);
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
  if (key === workersDrawn) return;
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
        catch (err) { note(`It didn't stop: ${err.message ?? err}`, 8000); }
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
    else if (typeof a.clip === "number") { agentHeldUntil = performance.now() + AGENT_HOLD_MS; await openClip(a.clip, a.mechanic, a.frame ?? 0); }
    else if (typeof a.frame === "number" && clip) seekClip(a.frame, true);
  } catch (e) {
    console.error(e);
    askError = String(e.message ?? e);
    // A clip that failed half way: back to live play.
    if (mode === "clip" && !clip) backToLive();
    note(`Couldn't show what your agent asked: ${e.message ?? e}`, 15000);
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
    await api("api/rebuild/build", {}).catch((e) => note(`The model couldn't be built again: ${e.message ?? e}`, 8000));
    return;
  }
  if (st.building === null && st.build && model.build && st.build !== model.build && model.ready) {
    model.build = st.build;
    await loadModel("The new version of the model is showing.");
  }
  if (st.error && st.building === null && st.stale) note(`The new model didn't build: ${st.error.split("\n").pop()}`, 8000);
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
        if (model.on && model.free) model.queue.push(padNow());
      } else if (mode === "live") {
        const pad = padNow();
        if (pad) lastPlay = now;
        // The model takes the frames the original read the pad on; while
        // it starts they wait in the queue.
        if (player.step(0, pad)) model.queue.push(pad);
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
      if (model.on && model.queue.length) {
        modelPost({ gp: "push", text: "gp-frames 1\n" + model.queue.map(framesLine).join("\n") + "\n" });
        model.queue = [];
      }
      if (show.view !== "solo") putOriginal(0);
    }
    const have = grabModel();
    drawPair(have);
    usualLine();
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
window.addEventListener("keyup", (e) => down.delete(e.code));
window.addEventListener("blur", () => down.clear());

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
  $("clip-play").addEventListener("click", (e) => { e.currentTarget.blur(); playClip(!clip?.playing); });
  $("clip-scrub").addEventListener("input", (e) => { playClip(false); seekClip(Number(e.target.value), true); });
  $("clip-scrub").addEventListener("change", (e) => { e.target.blur(); show.by = "person"; report(); });
  $("clip-live").addEventListener("click", (e) => { e.currentTarget.blur(); show.by = "person"; backToLive(); drawClips(); });
  $("follow-on").addEventListener("click", (e) => { e.currentTarget.blur(); setFollow(!follow.on); report(); });
  $("tell").addEventListener("submit", (e) => void tell(e));
  $("tell-row").addEventListener("click", (e) => { e.currentTarget.blur(); void newRow(); });
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
  session = await api("api/session").catch(() => null);
  $("title").textContent = session?.game ?? "";
  document.title = `${session?.game ?? "Game"} screen`;
  await setUpPictures();
  await refreshLists(true);
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
}

main().catch((e) => {
  const why = e instanceof Error ? e.message : String(e);
  line(`The screen couldn't start: ${why}`);
  document.body.dataset.failed = why;
});
