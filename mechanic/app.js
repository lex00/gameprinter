// The mechanic page: the game, played live with the keyboard or a gamepad,
// and the agent's requests over it. The window is one of four things: the
// live game, a clip being recorded, a clip just recorded to hand off, or
// the agent's words (with choices to pick, or a clip replayed under them),
// which pause the game until they are answered. Everything the page writes
// goes to this machine's `gp mechanic start`.
//
// Ported from game-printer's mechanic page with two tabs, HELP WITH
// MECHANICS and FREE PLAY. REVIEW REBUILD (the original beside the model),
// sealed runs and the measurement overlays wait until their server side
// comes over.

import { NesPlayer } from "/nesplayer.js";

const NES_FPS = 60.0988;
// Keys for A, B, Select, Start, Up, Down, Left, Right (the input log's bits).
const KEYS = [["KeyX", "Space"], ["KeyZ", "ShiftLeft"], ["ShiftRight", "Tab"], ["Enter"], ["ArrowUp", "KeyW"], ["ArrowDown", "KeyS"], ["ArrowLeft", "KeyA"], ["ArrowRight", "KeyD"]];
// Controller 2 from the keyboard, as the preview and the Bevy build map it:
// M and N for A and B, H Select, Y Start, I J K L the d-pad. A second
// gamepad is controller 2 too.
const KEYS2 = [["KeyM"], ["KeyN"], ["KeyH"], ["KeyY"], ["KeyI"], ["KeyK"], ["KeyJ"], ["KeyL"]];
// Gamepad buttons, standard mapping.
const PAD = { a: 0, b: 1, lb: 4, rb: 5, lt: 6, rt: 7, select: 8, start: 9, up: 12, down: 13 };

const $ = (id) => document.getElementById(id);

/** The first connected controller, in whatever slot the browser put it. */
function firstPad() {
  for (const g of navigator.getGamepads?.() ?? []) if (g && g.connected) return g;
  return null;
}

/** The second connected controller: controller 2, as on the console. */
function secondPad() {
  let n = 0;
  for (const g of navigator.getGamepads?.() ?? []) if (g && g.connected && n++ === 1) return g;
  return null;
}
const canvas = $("game");
const ctx = canvas.getContext("2d");
const down = new Set();
let player, rom;
let save = null; // the last save point's bytes
let mode = "live"; // live | recording | review | sending | agent | idle | watch
let recStart = 0; // the frame a recording started on
let pendingClip = null; // {log, start, frames}
// The quest-log row a recording goes to: the open one when recording
// starts; a click on another row while recording or reviewing moves it
// there. Fixed when A or Enter hands it off, and sent with the clip, so the
// server files it there whatever is open by the time it arrives.
let clipFor = null;
let seenTurns = 0; // agent turns already shown
let agentTurn = null; // the turn on screen
let choice = 0;
let showClip = null; // a clip replayed under the agent's words: {lines, i}
let held = {}; // gamepad buttons down last read, for presses
let session = null;
// Two clips shown together: the older in console 0 (the live game saved
// aside), the newer in console 1. view: overlay | side | diff.
let compare = null; // {old: {log, start, end}, saved}
// Three layouts: side by side, the newer on top of the older at a
// transparency, and their difference, black where the two match. Kept in
// the browser.
const VIEWS = ["side", "top", "diff"];
const VIEW_WORDS = { side: "side by side", top: "the newer on top", diff: "the differences" };
let view = "side";
let opacity = 0.5;
try {
  const v = JSON.parse(localStorage.getItem("gp-mechanic-view") ?? "null");
  if (v && VIEWS.includes(v.view)) { view = v.view; opacity = Math.min(1, Math.max(0, Number(v.opacity) || 0.5)); }
} catch (e) { /* no storage: the defaults */ }
function keepView() {
  try { localStorage.setItem("gp-mechanic-view", JSON.stringify({ view, opacity })); } catch (e) { /* not kept */ }
}
// The agent's model playing, watched as it happens: its demo's start, the
// presses still to play, the decisions seen and the overlay's words.
let watch = null; // {key, seen, queue: [pad], end, last}

// Buttons still down from closing a panel: Enter or A answered it, and
// mustn't also reach the game as Start or A (Start pauses it).
let mask = 0;
/** Both controllers for the game: controller 1's bits in the low byte,
 * controller 2's in the next, as gp_play_step takes them. */
function padNow() {
  const pad = rawPad();
  mask &= pad;
  return (pad & ~mask) | (rawPad2() << 8);
}

/** Controller 1: the keyboard and the first gamepad. */
function rawPad() {
  let pad = 0;
  KEYS.forEach((ks, bit) => { if (ks.some((k) => down.has(k))) pad |= 1 << bit; });
  return pad | gamepadBits(firstPad());
}

/** Controller 2: its keys and the second gamepad. */
function rawPad2() {
  let pad = 0;
  KEYS2.forEach((ks, bit) => { if (ks.some((k) => down.has(k))) pad |= 1 << bit; });
  return pad | gamepadBits(secondPad());
}

/** A gamepad's NES buttons, its left stick as the d-pad. */
function gamepadBits(g) {
  let pad = 0;
  if (g) {
    const b = (i) => g.buttons[i]?.pressed === true;
    const ax = g.axes[0] ?? 0, ay = g.axes[1] ?? 0;
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

/** Gamepad buttons pressed since the last read: each counts once a press. */
function padPresses() {
  const g = firstPad();
  const out = {};
  if (!g) return out;
  for (const [name, i] of Object.entries(PAD)) {
    const b = g.buttons[i];
    const now = !!b && (b.pressed || (b.value ?? 0) > 0.5);
    if (now && !held[name]) out[name] = true;
    held[name] = now;
  }
  return out;
}

const off = [document.createElement("canvas"), document.createElement("canvas")];
for (const c of off) { c.width = 256; c.height = 240; }

function draw(slot) {
  const img = new ImageData(player.rgba(slot), 256, 240);
  ctx.putImageData(img, 0, 0);
}

/** The two clips of a comparison in the chosen view: the newer (console 1)
 * over the older (console 0), side by side, or their difference. */
function drawCompare() {
  drawPair(player.rgba(0), player.rgba(1));
}

/** Two pictures in the chosen view: side by side (a left), `b` over `a` at
 * the chosen transparency, or their difference (each color's distance,
 * black where they match). */
function drawPair(a, b) {
  $("screen").classList.toggle("side", view === "side");
  $("game2").hidden = view !== "side";
  if (view === "side") {
    ctx.putImageData(new ImageData(a, 256, 240), 0, 0);
    const c2 = $("game2").getContext("2d");
    c2.putImageData(new ImageData(b, 256, 240), 0, 0);
  } else if (view === "top") {
    off[0].getContext("2d").putImageData(new ImageData(a, 256, 240), 0, 0);
    off[1].getContext("2d").putImageData(new ImageData(b, 256, 240), 0, 0);
    ctx.globalAlpha = 1;
    ctx.drawImage(off[0], 0, 0);
    ctx.globalAlpha = opacity;
    ctx.drawImage(off[1], 0, 0);
    ctx.globalAlpha = 1;
  } else {
    const out = new Uint8ClampedArray(a.length);
    for (let i = 0; i < a.length; i += 4) {
      out[i] = Math.abs(a[i] - b[i]); out[i + 1] = Math.abs(a[i + 1] - b[i + 1]); out[i + 2] = Math.abs(a[i + 2] - b[i + 2]); out[i + 3] = 255;
    }
    ctx.putImageData(new ImageData(out, 256, 240), 0, 0);
  }
}

function endCompare() {
  $("screen").classList.remove("side");
  $("game2").hidden = true;
  if (!compare) return;
  try { player.restore(0, compare.saved); } catch (e) { setState(String(e)); }
  compare = null;
  $("screen").classList.remove("side");
  $("game2").hidden = true;
}

function rewindCompare() {
  if (compare.old.startState) {
    player.restore(0, compare.old.startState);
    return;
  }
  player.replay(0, rom, compare.old.log);
  player.runTo(0, compare.old.start);
  try { compare.old.startState = player.save(0); } catch (e) { compare.old.startState = null; }
}

function cycleView(d) {
  const i = VIEWS.indexOf(view);
  view = VIEWS[((i < 0 ? 0 : i) + d + VIEWS.length) % VIEWS.length];
  keepView();
  toast(`Two clips: ${VIEW_WORDS[view]}.`, 1500);
}

async function api(path, body) {
  const r = await fetch(path, body === undefined ? {} : { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
  if (!r.ok) throw new Error(`${path}: ${r.status} ${await r.text()}`);
  return r.json();
}

function setState(text) { $("state").textContent = text; }

/** A line over the game for a few seconds: what a press just did. */
let toastTimer = null;
function toast(text, ms = 3000) {
  $("toast").textContent = text;
  $("toast").hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { $("toast").hidden = true; }, ms);
}

/** Work the page does itself, a known number of frames: the bar along the
 * game fills and says what it is. */
function progress(label, done, of) {
  $("bar").hidden = false;
  $("bar-fill").style.width = `${Math.max(0, Math.min(1, done / Math.max(1, of))) * 100}%`;
  $("bar-time").textContent = `${label} ${Math.round((done / Math.max(1, of)) * 100)}%`;
}

/** Replays a slot's log up to `target` in slices of about 30 ms, so the
 * page keeps drawing and answering: a clip late in a long play is tens of
 * thousands of frames from power-on, and the player hashes each one.
 * Returns false when `still()` turns false (the person moved on). */
async function runToSliced(slot, target, label, still) {
  const from = player.frame(slot);
  while (player.frame(slot) < target) {
    const t0 = performance.now();
    while (player.frame(slot) < target && performance.now() - t0 < 30) {
      const before = player.frame(slot);
      player.runTo(slot, Math.min(target, before + 20));
      if (player.frame(slot) === before) return still();
    }
    progress(label, player.frame(slot) - from, target - from);
    await new Promise((r) => setTimeout(r, 0));
    if (!still()) return false;
  }
  return still();
}

/** The bar along the bottom of the game: how far into a clip the picture
 * is, and its length, so the loop's start is plain. */
function setBar(at, of) {
  if (of === null) {
    $("bar").hidden = true;
    return;
  }
  $("bar").hidden = false;
  $("bar-fill").style.width = `${Math.max(0, Math.min(1, at / Math.max(1, of))) * 100}%`;
  $("bar-time").textContent = of === undefined ? `${(at / NES_FPS).toFixed(1)} s` : `${(at / NES_FPS).toFixed(1)} of ${(of / NES_FPS).toFixed(1)} s`;
}

/** The banner while it's the agent's turn, with what it says it is doing. */
let busySince = null;
function updateBusy() {
  // Always says whose turn it is and what you can do now.
  $("busy").hidden = !session;
  if (!session || mode === "idle") return;
  const agents = session.waiting_for === "agent" && session.talk.length > 0;
  $("busy").classList.toggle("theirs", agents);
  const waiting = session.talk.filter((t) => t.by === "agent").length > seenTurns;
  if (free && (mode === "live" || mode === "recording")) {
    const rec = mode === "recording" ? " Recording: R or the right bumper stops." : " R or the right bumper records; Esc goes back to HELP WITH MECHANICS.";
    $("busy-text").textContent = "Free play: the game on its own." + rec + (waiting ? " Your agent has a message: Esc to see it." : "");
    return;
  }
  if (mode === "recording") {
    busySince = null;
    $("busy-text").textContent = "Recording. Right bumper (or R) stops.";
  } else if (mode === "review") {
    $("busy-text").textContent = "A hands the clip to your agent, B drops it.";
  } else if (mode === "sending") {
    $("busy-text").textContent = "Sending your clip. Buttons wait until it's sent.";
  } else if (mode === "agent") {
    $("busy-text").textContent = "Your agent is talking to you: the note below. A goes back to the game.";
  } else if (agents) {
    busySince ??= performance.now();
    const secs = Math.round((performance.now() - busySince) / 1000);
    $("busy-text").textContent = `Your agent is working${session.busy ? ": " + session.busy : ""} (${secs} s). Keep playing; the right bumper records another clip if you want.`;
  } else {
    busySince = null;
    $("busy-text").textContent = "Your turn. Play, and the right bumper (or R) records a clip; stop it with the right bumper again.";
  }
}

// The margin: the clip loaded ---------------------------------------------

// Left of the game, the clip loaded: its row, length and note. Empty when
// nothing is loaded. (The right margin held the rebuild's differences at
// this frame; it waits for REVIEW REBUILD.)

/** A clip's context: its note from the open session's clips. */
function ctxFor(mechanic, n) {
  const meta = mechanic === session?.mechanic ? session?.clips?.find((x) => x.n === n) : null;
  return { title: `${mechanic} clip ${n}`, row: mechanic, note: meta?.note, start: meta?.start };
}

function el(parent, tag, text, cls) {
  const e = document.createElement(tag);
  if (text !== undefined && text !== null) e.textContent = String(text);
  if (cls) e.className = cls;
  parent.appendChild(e);
  return e;
}

let ctxKey = "";
function renderCtx() {
  const viewing = showClip && (mode === "agent" || mode === "idle") ? showClip : null;
  const key = viewing ? viewing.ctx?.title ?? "clip" : "";
  if (key === ctxKey) return;
  const left = $("ctx-clip");
  if (!left) return;
  ctxKey = key;
  left.innerHTML = "";
  if (!viewing) return;
  const c = viewing.ctx ?? {};
  const secs = (f) => `${(f / NES_FPS).toFixed(1)} s`;
  el(left, "h3", c.title ?? "This clip");
  const t = el(left, "table");
  const row = (k, v) => { const tr = el(t, "tr"); el(tr, "td", k); el(tr, "td", v, "v"); };
  row("row", c.row ?? "");
  row("length", secs(viewing.end - viewing.start));
  if (c.note) el(left, "p", `"${c.note}"`, "muted");
}

// FREE PLAY ------------------------------------------------------------

// The original on its own: the agent's turns wait (the line under the game
// says one came) and the idle screen stays away. Recording and handing off
// work as in HELP WITH MECHANICS. In game-printer FREE PLAY was the rebuild
// alone; the rebuild comes back with the REVIEW REBUILD tab.
let free = false;

/** The tab lit: FREE PLAY or HELP WITH MECHANICS. */
function syncTabs() {
  $("tab-mech").classList.toggle("on", !free);
  $("tab-free").classList.toggle("on", free);
}

// Recording ------------------------------------------------------------

function startRecording() {
  recStart = player.frame(0);
  clipFor = picking ?? session?.mechanic ?? null;
  mode = "recording";
  markTarget();
  $("rec").hidden = false;
  warmPath();
}

/** The play so far goes to the server as the recording starts, and the
 * server replays it to the start in the background, so handing the clip
 * off later sends only the frames since and waits on no replay. */
function warmPath() {
  const text = player.inputLog(0);
  warming = sendPath(text, recStart).catch(() => undefined);
}
let warming = null; // the play's upload started with the recording

/** Goes back to the quick save. The save holds the play log up to its
 * frame, so the clip stays a path from power-on and drops the tries after
 * it. Loading back to before a recording started moves its start there. */
function quickLoad() {
  if (!save) { toast("No save yet: F5 or the left trigger saves."); return; }
  player.restore(0, save);
  flushAudio();
  if (mode === "recording" && player.frame(0) < recStart) recStart = player.frame(0);
  setState(mode === "recording" ? "Loaded. Still recording." : "Loaded.");
  toast(mode === "recording" ? "Loaded. Still recording." : "Loaded.");
}

function stopRecording() {
  const log = player.inputLog(0);
  const frames = player.frame(0) - recStart;
  $("rec").hidden = true;
  if (frames < 10) { mode = "live"; clipFor = null; markTarget(); toast("Too short to keep: under 10 frames."); return; }
  pendingClip = { log, start: recStart, frames };
  mode = "review";
  reviewPanel();
}

function reviewPanel() {
  const c = pendingClip;
  showPanel(`Clip of ${(c.frames / NES_FPS).toFixed(1)} s. Hand off to ${clipFor}?`, [], true,
    "A or Enter: hand it off to that row. B or Esc: discard it. Right bumper or R: record it again. Click another quest-log row to hand it to that one.");
  showTo(clipFor);
}

/** The row a clip goes to, big over the panel: yellow for the open row,
 * orange for another. */
function showTo(row) {
  const to = $("to");
  if (!row) { to.hidden = true; return; }
  to.hidden = false;
  to.className = row === session?.mechanic ? "" : "other";
  to.textContent = `TO: ${row}`;
}

/** The row a recording goes to, outlined in the quest log while it is
 * recorded, reviewed and sent. */
function markTarget() {
  const on = mode === "recording" || mode === "review" || mode === "sending" ? clipFor : null;
  for (const li of $("queue").children) li.classList.toggle("target", li.dataset.mechanic === on);
}

// The play log the server already holds as a path, so a clip only sends
// what was played since: the path is the play from power-on, the clip is
// a frame on it and the buttons pressed from there.
let sentPath = null; // {path, text}
async function sendPath(text, warm) {
  const head = text.split("\n", 2).join("\n") + "\n";
  let p;
  if (sentPath && text === sentPath.text) {
    p = { path: sentPath.path };
    if (warm !== undefined) p = await api("/api/paths", { base: sentPath.path, tail: head, warm });
  } else if (sentPath && text.startsWith(sentPath.text)) {
    p = await api("/api/paths", { base: sentPath.path, tail: head + text.slice(sentPath.text.length), warm });
  } else {
    p = await api("/api/paths", { log: text, warm });
  }
  sentPath = { path: p.path, text };
  return p.path;
}

async function sendClip(c, note, mechanic, phase) {
  const text = c.log;
  const lines = text.split("\n");
  const head = lines.slice(0, 2).join("\n") + "\n";
  const frames = lines.slice(2).filter((l) => l.length);
  const tail = sentPath && text.startsWith(sentPath.text) ? text.length - sentPath.text.length : text.length;
  if (warming) { phase("Sending your play"); await warming; warming = null; }
  phase(`Sending your play (${Math.max(1, Math.round(tail / 1024))} KB)`);
  const path = await sendPath(text);
  phase("Saving the clip");
  const inputs = head + frames.slice(c.start).join("\n") + "\n";
  return api("/api/clips", { path, start: c.start, inputs, note, mechanic });
}

/** Hands the reviewed clip to the agent. The page is in the sending state
 * from the first line until the server has the clip and the turn: no
 * button does anything there, and the panel says what is happening. */
async function handOff() {
  if (mode !== "review" || !pendingClip) return;
  mode = "sending";
  mask = 0xff;
  // The row is fixed here, at the press, not when the server answers.
  const target = clipFor;
  const c = pendingClip;
  const note = $("note").value.trim();
  const t0 = performance.now();
  let what = `Sending your clip to ${target}`;
  const show = () => {
    $("say").textContent = `${what}, ${((performance.now() - t0) / 1000).toFixed(1)} s`;
  };
  showPanel("", [], false, "Buttons wait until it's sent.");
  showTo(target);
  show();
  const timer = setInterval(show, 100);
  let n, again;
  try {
    ({ n, again } = await sendClip(c, note, target, (w) => { what = `${w} to ${target}`; show(); }));
    what = `Telling your agent (${target})`;
    show();
    await api("/api/talk", { text: note || `Here is clip ${n}.`, clips: [n], mechanic: target });
  } catch (e) {
    clearInterval(timer);
    // The clip stays: A tries again, B drops it.
    mode = "review";
    showPanel(`The clip didn't reach your agent: ${e}`, [], true, "A or Enter: try again. B or Esc: discard it.");
    return;
  }
  clearInterval(timer);
  pendingClip = null;
  clipFor = null;
  $("note").value = "";
  hidePanel();
  mode = "live";
  markTarget();
  const ms = Math.round(performance.now() - t0);
  const done = again ? `Clip ${n} of ${target} was already handed off.` : `Clip ${n} handed off to ${target} in ${(ms / 1000).toFixed(1)} s. The agent is working on it.`;
  setState(done);
  try { await refresh(); } catch (e) { setState(String(e)); }
  // A clip for another row opens that row, so the agent's answer shows.
  if (target && target !== session?.mechanic) await pick(target);
  toast(done, 5000);
}

// The agent's words ----------------------------------------------------

/** The agent's turn over the paused game. The words show at once; a clip
 * under them loads after, with the bar showing how far, and A answers
 * whether or not it has finished loading. */
async function showAgent(turn) {
  // The answer and the clips shown are this mechanic's, whatever opens later.
  turn.mechanic ??= session?.mechanic;
  agentTurn = turn;
  choice = 0;
  mode = "agent";
  showClip = null;
  const choices = turn.choices ?? [];
  const hint = choices.length
    ? "Up and down to choose, A or Enter to pick." + (turn.show ? " B replays the clip." : "")
    : "A or Enter when you're ready.";
  showPanel(turn.text, choices, false, hint);
  if (!turn.show) return;
  const still = () => agentTurn === turn && mode === "agent";
  try {
    progress("Loading the clip", 0, 1);
    const q = turn.mechanic ? `?mechanic=${encodeURIComponent(turn.mechanic)}` : "";
    const r = await fetch(`/api/clips/${turn.show}.log${q}`);
    if (!r.ok) throw new Error(`clip ${turn.show}: ${r.status}`);
    const text = await r.text();
    if (!still()) return;
    const meta = session.clips.find((c) => c.n === turn.show);
    const start = meta ? meta.start : 0;
    const clip = { start, end: meta ? start + meta.frames : start + 600, log: text };
    if (turn.against) {
      const r2 = await fetch(`/api/clips/${turn.against}.log${q}`);
      if (!r2.ok) throw new Error(`clip ${turn.against}: ${r2.status}`);
      const m2 = session.clips.find((c) => c.n === turn.against);
      const s2 = m2 ? m2.start : 0;
      const old = { log: await r2.text(), start: s2, end: m2 ? s2 + m2.frames : s2 + 600 };
      if (!still()) return;
      // The live game is saved aside while console 0 shows the older clip.
      compare = { old, saved: player.save(0) };
      if (!(await loadClip(0, old, "Loading the older clip", still))) return;
    }
    if (!(await loadClip(1, clip, "Loading the clip", still))) return;
    clip.ctx = ctxFor(turn.mechanic ?? session?.mechanic, turn.show);
    showClip = clip;
    if (compare) { rewindCompare(); cycleView(0); }
  } catch (e) {
    showClip = null;
    setBar(0, null);
    setState(`Couldn't load the clip: ${e}`);
  }
}

/** A clip of the open mechanic on a loop, from the clip list, the open
 * row's NEWEST CLIP, or C and the left bumper (its newest clip). */
async function openClipView(n) {
  if (mode === "idle") leaveIdle();
  if (mode === "agent") { agentTurn = null; showClip = null; endCompare(); hidePanel(); mode = "live"; }
  if (mode !== "live") { toast(mode === "sending" ? "Wait for the clip to send." : "Finish the recording first."); return; }
  const c = session.clips.find((x) => x.n === n);
  toast(`Clip ${n}.`);
  // Shown as an agent turn of the page's own, which answers nothing.
  seenTurns = session.talk.filter((t) => t.by === "agent").length;
  await showAgent({ text: `Clip ${n}${c?.note ? ": " + c.note : ""}. It plays on a loop.`, choices: [], show: n, mine: true });
}

/** Powers a slot on to replay a clip's log and runs it to the clip's start
 * a slice at a time, with the bar showing how far. The state there is kept,
 * so each loop restores it instead of replaying thousands of frames again.
 * False when `still()` turned false on the way. */
async function loadClip(slot, c, label, still) {
  player.replay(slot, rom, c.log);
  if (!(await runToSliced(slot, c.start, label, still))) return false;
  try { c.startState = player.save(slot); } catch (e) { c.startState = null; }
  setBar(0, null);
  return true;
}

/** The clip under the agent's words, from its start again. */
function rewindClip() {
  if (audioSlot === 1) flushAudio();
  if (showClip.startState) {
    player.restore(1, showClip.startState);
    return;
  }
  player.replay(1, rom, showClip.log);
  player.runTo(1, showClip.start);
  try { showClip.startState = player.save(1); } catch (e) { showClip.startState = null; }
}

async function answerAgent() {
  if (mode !== "agent" || !agentTurn) return;
  const t = agentTurn;
  const choices = t.choices ?? [];
  agentTurn = null;
  showClip = null;
  mask = 0xff;
  endCompare();
  hidePanel();
  mode = "live";
  if (choices.length) {
    toast(`Answered: ${choices[choice]}.`);
    const body = { text: choices[choice], choice, mechanic: t.mechanic };
    try { await api("/api/talk", body); } catch (e) { setState(String(e)); }
  }
  setState("");
}

function showPanel(text, choices, withNote, hint) {
  showTo(null);
  $("say").textContent = text;
  // What was measured in the clip, so the person judges from the same facts.
  const facts = agentTurn?.facts ?? [];
  if (facts.length) $("say").textContent += "\n\n" + facts.join("\n");
  const ul = $("choices");
  ul.innerHTML = "";
  choices.forEach((c, i) => {
    const li = document.createElement("li");
    li.textContent = c;
    if (i === choice) li.className = "on";
    ul.appendChild(li);
  });
  $("note-row").hidden = !withNote;
  $("hint").textContent = hint;
  $("overlay").hidden = false;
  if (withNote) $("note").focus();
}

function hidePanel() {
  showTo(null);
  setBar(0, null);
  $("overlay").hidden = true;
  $("note").blur();
}

function moveChoice(d) {
  const n = agentTurn?.choices?.length ?? 0;
  if (!n) return;
  choice = (choice + d + n) % n;
  [...$("choices").children].forEach((li, i) => { li.className = i === choice ? "on" : ""; });
}

// The session ----------------------------------------------------------

// The watch panel -------------------------------------------------------
// What the agent is doing, from /api/watch: its status (the newest log
// line), what it pinned, its problem table and the log. Redrawn only when
// it changes, so a playing clip isn't restarted every poll.

/** An element with a class and text. */
function wEl(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

function timed(line) {
  const li = wEl("li");
  li.append(wEl("span", "at", line.at ?? ""), document.createTextNode(line.text ?? ""));
  return li;
}

async function refreshWatch() {
  if (!session) return;
  const q = `?mechanic=${encodeURIComponent(session.mechanic)}`;
  const w = await api(`/api/watch${q}`);
  const key = JSON.stringify([session.mechanic, w]);
  const col = $("watch-col");
  if (col.dataset.key === key) return;
  col.dataset.key = key;
  const status = $("w-status");
  status.innerHTML = "";
  if (w.status) {
    status.className = "";
    status.append(wEl("span", "at", w.status.at ?? ""), document.createTextNode(w.status.text ?? ""));
  } else {
    status.className = "muted";
    status.textContent = "Nothing yet. Your agent writes here with gp mechanic log, pin and problem.";
  }
  const pins = $("w-pins");
  pins.innerHTML = "";
  for (const p of w.pins ?? []) {
    const fig = wEl("figure", p.keep ? "keep" : "");
    const src = `/api/watch/files/${encodeURIComponent(p.file)}${q}`;
    let media;
    if (p.kind === "video") {
      media = wEl("video");
      Object.assign(media, { src, muted: true, loop: true, autoplay: true, controls: true, playsInline: true });
    } else {
      media = wEl("img");
      Object.assign(media, { src, alt: p.title || p.file, loading: "lazy" });
    }
    fig.append(media, wEl("figcaption", "", `${p.n}. ${p.title || p.file}`));
    pins.append(fig);
  }
  const table = $("w-problems");
  const body = table.querySelector("tbody");
  body.innerHTML = "";
  for (const r of w.problems ?? []) {
    const tr = wEl("tr");
    for (const [v, cls] of [[r.n], [r.where], [r.what], [r.fix], [r.state, `st-${r.state}`]]) tr.append(wEl("td", cls ?? "", String(v ?? "")));
    body.append(tr);
  }
  table.hidden = !(w.problems ?? []).length;
  const log = $("w-log");
  log.innerHTML = "";
  for (const line of [...(w.log ?? [])].reverse()) log.append(timed(line));
}

async function refresh() {
  session = await api("/api/session");
  $("title").textContent = `${session.game} / ${session.mechanic}`;
  const all = session.queue ?? [];
  $("score").textContent = all.length ? `${all.filter((q) => q.status === "done").length} / ${all.length} MEASURED` : "";
  // Rebuilt only when it changes: a list redrawn under the mouse every poll
  // loses the click.
  const queue = $("queue");
  const qkey = JSON.stringify([session.mechanic, all]);
  if (queue.dataset.key !== qkey) {
    queue.dataset.key = qkey;
    queue.innerHTML = "";
    for (const q of all) {
      const li = document.createElement("li");
      const st = document.createElement("span");
      st.className = `st st-${q.status.replace(/ /g, "-")}`;
      st.textContent = LABELS[q.status] ?? q.status.toUpperCase();
      li.appendChild(st);
      const name = document.createElement("span");
      name.className = "qname";
      name.textContent = q.mechanic;
      li.appendChild(name);
      // The typed note.
      for (const [k, cls] of [["note", "qnote"]]) {
        if (!q[k]) continue;
        const note = document.createElement("span");
        note.className = cls;
        note.textContent = q[k];
        li.appendChild(note);
      }
      if (q.mechanic === session.mechanic) {
        li.classList.add("open");
        const cmp = document.createElement("button");
        cmp.className = "qcmp";
        cmp.textContent = "NEWEST CLIP";
        cmp.title = "Its newest clip on a loop (C, or the left bumper)";
        cmp.addEventListener("click", (e) => { e.stopPropagation(); doAction("compareNewest"); });
        li.appendChild(cmp);
      }
      li.dataset.mechanic = q.mechanic;
      li.addEventListener("click", () => void pick(q.mechanic));
      queue.appendChild(li);
    }
    markTarget();
    showFindings(all.find((q) => q.mechanic === session.mechanic));
  }
  const clips = $("clips");
  const ckey = JSON.stringify([session.mechanic, session.clips.map((c) => [c.n, c.frames, c.note])]);
  if (clips.dataset.key !== ckey) {
    clips.dataset.key = ckey;
    clips.innerHTML = "";
    for (const c of session.clips) {
      const li = document.createElement("li");
      li.value = c.n;
      li.textContent = `${(c.frames / NES_FPS).toFixed(1)} s, by ${c.by}${c.note ? ": " + c.note : ""}`;
      li.title = "Click to watch it";
      li.addEventListener("click", () => void openClipView(c.n));
      // The clip on the model, beside the original (review.js).
      const rv = document.createElement("button");
      rv.type = "button";
      rv.className = "rv2-open";
      rv.textContent = "ON THE MODEL";
      rv.title = "Play this clip on the model beside the original, as the score does";
      rv.addEventListener("click", (e) => {
        e.stopPropagation();
        window.dispatchEvent(new CustomEvent("gp-review", { detail: { clip: c.n, mechanic: session.mechanic } }));
      });
      li.appendChild(rv);
      clips.appendChild(li);
    }
  }
  const talk = $("talk");
  talk.innerHTML = "";
  for (const t of session.talk) {
    const li = document.createElement("li");
    li.value = t.n;
    li.className = t.by === "agent" ? "by-agent" : "";
    li.textContent = `${t.by}: ${t.text}`;
    talk.appendChild(li);
  }
  // A new turn from the agent pauses the game and shows over it.
  const agentTurns = session.talk.filter((t) => t.by === "agent");
  // Over HELP WITH MECHANICS only; in FREE PLAY the turn waits, and the
  // line under the game says so.
  if (agentTurns.length > seenTurns && mode === "live" && free) {
    $("busy-text").textContent = "Your agent has a message. Esc goes back to HELP WITH MECHANICS, where it shows.";
  } else if (agentTurns.length > seenTurns && mode === "live") {
    seenTurns = agentTurns.length;
    await showAgent(agentTurns[agentTurns.length - 1]);
  }
  if (mode === "live") setState("");
  updateBusy();
}

// What the queue's statuses show as.
const LABELS = { done: "MEASURED", "asking you": "YOUR MOVE", "agent working": "WORKING ON IT", "in line": "IN LINE", next: "NEXT" };

/** The open mechanic's findings: what was measured, what the agent read
 * into it, and what isn't found yet. */
function showFindings(q) {
  const box = $("findings");
  box.innerHTML = "";
  const parts = [["known", "Measured"], ["read", "My reading (check it by eye)"], ["open", "Not found yet"]];
  let any = false;
  for (const [k, title] of parts) {
    const items = q?.[k] ?? [];
    if (!items.length) continue;
    any = true;
    const h = document.createElement("h3");
    h.className = `f-${k}`;
    h.textContent = title;
    box.appendChild(h);
    const ul = document.createElement("ul");
    ul.className = `f-${k}`;
    for (const t of items) {
      const li = document.createElement("li");
      li.textContent = t;
      ul.appendChild(li);
    }
    box.appendChild(ul);
  }
  box.hidden = !any;
}

// The two tabs ---------------------------------------------------------

/** HELP WITH MECHANICS is the page as it always was; FREE PLAY is the game
 * on its own, the agent's turns waiting until you come back. */
function setTab(t) {
  // A clip being recorded, reviewed or sent keeps its tab until it's done.
  if (mode === "recording" || mode === "review") { toast("Finish the recording first."); return; }
  if (mode === "sending") { toast("Wait for the clip to send."); return; }
  if (t === "free" && !free) {
    if (mode === "idle") leaveIdle();
    if (mode === "agent") { toast("Answer your agent first."); return; }
    if (mode !== "live") { toast("Back to the game first."); return; }
    free = true;
    toast("Free play: the game on its own. R records, Esc goes back.", 4000);
  } else if (t !== "free" && free) {
    free = false;
    toast("Help with mechanics.");
  }
  syncTabs();
}

/** Opens another mechanic from the queue and hands it to the agent. The
 * game keeps running where it is. */
let picking = null; // the mechanic being opened
async function pick(m) {
  if (picking) { toast(`Still opening ${picking}.`); return; }
  if (mode === "sending") { toast(`This clip goes to ${clipFor}. Wait for it to send.`); return; }
  if (mode === "recording" || mode === "review") {
    clipFor = m;
    markTarget();
    if (mode === "review") reviewPanel();
    toast(`This clip goes to ${m}.`);
    return;
  }
  if (m === session?.mechanic) {
    const row = (session.queue ?? []).find((q) => q.mechanic === m);
    if (mode === "idle") leaveIdle();
    if (mode === "live" && row && (row.status === "agent working" || row.status === "in line")) { toast(`Loading what your agent has on ${m}.`); void enterViewer(m); }
    else toast(`Already on ${m}.`);
    return;
  }
  if (mode === "idle") leaveIdle();
  if (mode === "agent") { agentTurn = null; showClip = null; endCompare(); hidePanel(); mode = "live"; }
  if (mode !== "live") { toast(`Can't switch while ${mode === "watch" ? "your agent's model plays" : "busy"}.`); return; }
  picking = m;
  toast(`Opening ${m}.`);
  $("queue-hint").textContent = `Opening ${m}...`;
  try {
    await api("/api/pick", { mechanic: m });
    seenTurns = 1e9;
    watch = null;
    await refresh();
    const mine = session.talk.filter((t) => t.by === "agent").length;
    seenTurns = session.waiting_for === "player" ? mine - 1 : mine;
    $("queue-hint").textContent = `Now on ${m}.`;
    toast(`Now on ${m}.`);
  } catch (e) { setState(String(e)); toast(`Couldn't open ${m}.`); } finally { picking = null; }
  const row = (session.queue ?? []).find((q) => q.mechanic === m);
  if (row && (row.status === "agent working" || row.status === "in line")) void enterViewer(m);
}

// Input ----------------------------------------------------------------

// <actions> What each input does in each of the page's states; an input
// missing from a state does nothing there. The game's own buttons reach the
// game only while live or recording. Each action checks the state it was
// meant for before it acts, so a second press (A after Enter, both read in
// one poll) finds the state already moved on and does nothing.
// FREE PLAY has its own live and recording rows, "live:free" and
// "recording:free"; see modeKey().
const ACTIONS = {
  live: { save: "save", load: "load", rec: "startRec", cmp: "compareNewest" },
  recording: { save: "save", load: "load", rec: "stopRec" },
  "live:free": { save: "save", load: "load", rec: "startRec", esc: "freeStop" },
  "recording:free": { save: "save", load: "load", rec: "stopRec" },
  review: { a: "handOff", start: "handOff", b: "discard", esc: "discard", rec: "recordAgain" },
  sending: {},
  agent: { a: "answer", start: "answer", up: "choiceUp", down: "choiceDown", b: "replayClip", left: "viewLeft", right: "viewRight" },
  idle: {},
  watch: {},
};
// The state an action leaves the page in, at once (before any reply).
const NEXT = { handOff: "sending", discard: "live", recordAgain: "recording", answer: "live", startRec: "recording", stopRec: "review" };
const KEY_INPUT = { Enter: "start", KeyX: "a", Space: "a", KeyZ: "b", ShiftLeft: "b", ArrowUp: "up", ArrowDown: "down", ArrowLeft: "left", ArrowRight: "right", Escape: "esc", Tab: "select", ShiftRight: "select", KeyR: "rec", F5: "save", F8: "load", KeyC: "cmp" };
const TYPING_KEYS = ["start", "esc", "select", "up", "down"];
/** The row of ACTIONS for a mode, with the tab when it is FREE PLAY. */
function modeKey(mode, tab) {
  return tab && (mode === "live" || mode === "recording") ? `${mode}:${tab}` : mode;
}
function actionFor(mode, input, typing) {
  // While the note is being typed in, only Enter, Esc, Tab and the up and
  // down arrows (no use in a one-line note) are commands.
  if (typing && !TYPING_KEYS.includes(input)) return null;
  return ACTIONS[mode]?.[input] ?? null;
}
// </actions>

function doAction(name) {
  switch (name) {
    case "save": save = player.save(0); setState("Saved."); toast("Saved."); break;
    case "load": quickLoad(); break;
    case "startRec": if (mode === "live") startRecording(); break;
    case "stopRec": if (mode === "recording") stopRecording(); break;
    case "handOff": void handOff(); break;
    case "discard": if (mode === "review") { pendingClip = null; clipFor = null; hidePanel(); mode = "live"; mask = 0xff; markTarget(); toast("Clip dropped."); } break;
    case "recordAgain": if (mode === "review") { const keep = clipFor; pendingClip = null; hidePanel(); startRecording(); clipFor = keep; markTarget(); } break;
    case "answer": void answerAgent(); break;
    case "choiceUp": moveChoice(-1); break;
    case "choiceDown": moveChoice(1); break;
    case "replayClip": if (showClip) { rewindClip(); if (compare) rewindCompare(); } break;
    case "viewLeft": if (compare) cycleView(-1); break;
    case "viewRight": if (compare) cycleView(1); break;
    case "freeStop": if (free && mode === "live") setTab("mech"); break;
    case "compareNewest": {
      const last = session?.clips?.[session.clips.length - 1];
      if (last) void openClipView(last.n); else toast(`No clips of ${session?.mechanic} yet: R records one.`);
      break;
    }
  }
}

window.addEventListener("keydown", (e) => {
  const typing = document.activeElement === $("note");
  // U turns the sound off and on in every state, the note being typed
  // aside.
  if (e.code === SOUND_KEY_CODE && !typing && !e.ctrlKey && !e.metaKey && !e.altKey) {
    e.preventDefault();
    if (!e.repeat) setSound(!soundOn);
    return;
  }
  const input = KEY_INPUT[e.code];
  if (mode === "live" || mode === "recording") {
    const a = input && actionFor(modeKey(mode, free ? "free" : null), input, false);
    if (a) { e.preventDefault(); if (!e.repeat) doAction(a); return; }
    if (KEYS.flat().includes(e.code) || KEYS2.flat().includes(e.code)) e.preventDefault();
    down.add(e.code);
    return;
  }
  // Every other state: the game is paused and only the table's inputs act.
  if (!input || (typing && !TYPING_KEYS.includes(input))) return;
  e.preventDefault();
  const a = actionFor(modeKey(mode, free ? "free" : null), input, typing);
  if (a && !e.repeat) doAction(a);
}, { capture: true });
window.addEventListener("keyup", (e) => down.delete(e.code));
window.addEventListener("blur", () => down.clear());

/** The game buttons pressed since the last read, by the same mapping the
 * game uses (a d-pad that reports as stick axes included). */
let padPrev = 0;
function gamePresses() {
  const now = rawPad();
  const fresh = now & ~padPrev;
  padPrev = now;
  return { a: !!(fresh & 1), b: !!(fresh & 2), select: !!(fresh & 4), start: !!(fresh & 8), up: !!(fresh & 16), down: !!(fresh & 32), left: !!(fresh & 64), right: !!(fresh & 128) };
}

/** The controller the browser sees, in the footer, to tell why a button does nothing. */
function showPad() {
  const g = firstPad();
  $("keys").dataset.pad = g ? `${g.id} (mapping: ${g.mapping || "none"})` : "";
  $("keys").title = $("keys").dataset.pad;
  const line = $("pad-line");
  if (line) line.textContent = g ? `Controller: ${g.id}, mapping ${g.mapping || "not standard"}` : "No controller seen yet: press a button on it.";
}

function gamepadCommands() {
  const p = padPresses();
  const g = gamePresses();
  const pressed = { a: p.a || g.a, b: p.b || g.b, start: p.start || g.start, select: p.select || g.select, up: p.up || g.up, down: p.down || g.down, left: g.left, right: g.right, rec: p.rb, save: p.lt, load: p.rt, cmp: p.lb };
  // In live play the game's own buttons go to the game, not the table.
  const game = mode === "live" || mode === "recording";
  for (const [input, on] of Object.entries(pressed)) {
    if (!on || (game && !["rec", "save", "load", "cmp", "select"].includes(input))) continue;
    // Looked up again for each press: an action that moves the state
    // leaves the next press of the same poll nothing to do.
    const a = actionFor(modeKey(mode, free ? "free" : null), input, false);
    if (a) doAction(a);
  }
}

// Sound ------------------------------------------------------------------

// The original's sound, from gp_play's samples (a frame's at a time),
// played through one AudioContext. A browser runs one only after the person
// acts on the page, and Safari only when it is made or resumed while a click
// or a key is handled, so it is made or resumed in every click and key
// (docs/web.md, the public page's `__gpAudio`). A frame is queued at most
// 0.25 s ahead of the context's clock and dropped past that, and dropped
// while the context isn't running, so the sound never trails the picture.
// U or the button turns it off and on; the choice is kept in the browser.
const SOUND_KEY = "gp-mechanic-sound";
const SOUND_KEY_CODE = "KeyU";
let soundOn = true;
try { soundOn = localStorage.getItem(SOUND_KEY) !== "off"; } catch (e) { /* no storage: on */ }
let audio = null; // the AudioContext, once a click or a key made it
let audioOut = null; // the gain the queued frames go through; dropped to flush them
let audioAt = 0; // the context time the next frame's sound starts at
let audioSlot = null; // the console whose frames are heard

function unlockAudio() {
  if (!soundOn) return;
  try {
    audio ??= new AudioContext();
    if (audio.state !== "running") audio.resume().catch(() => {});
  } catch (e) {
    // A browser with no audio output: the game plays on in silence.
  }
}
for (const ev of ["keydown", "pointerdown", "click", "touchend"]) addEventListener(ev, unlockAudio, true);

/** Drops the sound queued and not yet heard: after a restore, a rewind or
 * a pause, so no stale frames play over the picture. */
function flushAudio() {
  if (audioOut) {
    const g = audioOut;
    g.gain.setTargetAtTime(0, audio.currentTime, 0.004);
    setTimeout(() => g.disconnect(), 50);
    audioOut = null;
  }
  audioAt = 0;
}

/** The console heard now, or null for silence. Live play is heard, and a
 * clip the page plays a frame at a time at the game's speed: the model
 * playing, the clip under the agent's words, and REVIEW REBUILD's PLAY.
 * Not the clip looped while the person is away (idle), not a seek, and not
 * the clip shown in REVIEW REBUILD, which isn't here yet. */
function heardSlot() {
  if (!soundOn) return null;
  if (mode === "live" || mode === "recording") return 0;
  if (mode === "watch") return watch && !watch.old && watch.ready ? 1 : null;
  if (mode === "agent" && showClip) return 1;
  return null;
}

/** Queues the frame `slot` just played. */
function playFrame(slot) {
  if (slot !== audioSlot) return;
  const f = player.audio(slot);
  if (!audio || !f.length || audio.state !== "running") return;
  const now = audio.currentTime;
  audioAt = Math.max(audioAt, now + 0.05);
  if (audioAt > now + 0.25) return;
  if (!audioOut) {
    audioOut = audio.createGain();
    audioOut.connect(audio.destination);
  }
  const rate = player.audioRate();
  const b = audio.createBuffer(1, f.length, rate);
  b.copyToChannel(f, 0);
  const src = audio.createBufferSource();
  src.buffer = b;
  src.connect(audioOut);
  src.start(audioAt);
  audioAt += f.length / rate;
}

function setSound(on) {
  soundOn = on;
  try { localStorage.setItem(SOUND_KEY, on ? "on" : "off"); } catch (e) { /* not kept */ }
  if (on) unlockAudio(); else flushAudio();
  showSound();
  toast(on ? "Sound on (U turns it off)." : "Sound off (U turns it on).");
}

function showSound() {
  const b = $("sound");
  b.textContent = soundOn ? "SOUND ON" : "SOUND OFF";
  b.setAttribute("aria-pressed", String(soundOn));
}

// The loop ---------------------------------------------------------------

let acc = 0, last = performance.now();
function frame(now) {
  acc = Math.min(acc + (now - last), (1000 / NES_FPS) * 4);
  last = now;
  try {
    gamepadCommands();
    const heard = heardSlot();
    if (heard !== audioSlot) { flushAudio(); audioSlot = heard; }
    while (acc >= 1000 / NES_FPS) {
      acc -= 1000 / NES_FPS;
      if (mode === "live" || mode === "recording") {
        player.step(0, padNow());
        playFrame(0);
      } else if (mode === "watch") {
        watchFrame();
      } else if ((mode === "agent" || mode === "idle") && showClip) {
        // The clip under the agent's words loops from its start.
        if (compare) {
          // Both clips from their starts, in step; both again once the longer ends.
          const doneB = player.frame(1) >= showClip.end || !player.advance(1);
          if (!doneB) playFrame(1);
          const doneA = player.frame(0) >= compare.old.end || !player.advance(0);
          const len = Math.max(showClip.end - showClip.start, compare.old.end - compare.old.start);
          const at = player.frame(1) - showClip.start;
          if ((doneA && doneB) || at >= len) { rewindClip(); rewindCompare(); }
          setBar(player.frame(1) - showClip.start, len);
        } else {
          if (player.frame(1) >= showClip.end || !player.advance(1)) rewindClip();
          else playFrame(1);
          setBar(player.frame(1) - showClip.start, showClip.end - showClip.start);
        }
      }
    }
    let frameSource;
    if (mode === "agent" && showClip && compare) {
      frameSource = "clips";
      drawCompare();
    } else {
      const slot = ((mode === "agent" || mode === "idle") && showClip) || mode === "watch" ? 1 : 0;
      frameSource = slot ? "clip" : "original";
      draw(slot);
    }
    // Which game the picture is of, and the frame the live game is on, for
    // the page's check (scripts/web/check-mechanic.mjs).
    if (document.body.dataset.frameSource !== frameSource) document.body.dataset.frameSource = frameSource;
    document.body.dataset.frame = String(player.frame(0));
    renderCtx();
    idleCheck();
    showMode();
    if (mode === "recording") $("rec-time").textContent = `${((player.frame(0) - recStart) / NES_FPS).toFixed(1)} s`;
  } catch (e) {
    setState(String(e));
  }
  requestAnimationFrame(frame);
}

// Idle: the agent's work on screen --------------------------------------

// After 30 s with no button, the game pauses and the screen loops the clip
// the agent is working on. Any button returns.
const IDLE_MS = 30000;
let lastInput = performance.now();
let idleFetching = false;
function idleCheck() {
  if (rawPad() !== 0 || rawPad2() !== 0) {
    lastInput = performance.now();
    if (mode === "idle") leaveIdle();
    return;
  }
  // Only in HELP WITH MECHANICS: FREE PLAY is the game on its own.
  if (mode !== "live" || free || idleFetching || performance.now() - lastInput < IDLE_MS) return;
  void enterViewer(null);
}

/** Watching the agent: the newest clip of a mechanic (by default the one
 * it is working on) loops. Any button goes back to playing. */
async function enterViewer(m) {
  if (idleFetching) return;
  idleFetching = true;
  try {
    const w = await api("/api/working" + (m ? `?mechanic=${encodeURIComponent(m)}` : ""));
    if (mode !== "live" || !w.mechanic || !w.log) {
      lastInput = performance.now();
      if (m) toast(`Nothing to watch on ${m} yet.`);
      return;
    }
    mode = "idle";
    showMode();
    $("busy-text").textContent = `Watching your agent on ${w.mechanic}${w.busy ? ": " + w.busy : ""}. Any button goes back to playing.`;
    const clip = { start: w.clip.start, end: w.clip.start + w.clip.frames, log: w.log };
    // Any button while it loads leaves idle; the load then stops.
    if (!(await loadClip(1, clip, "Loading the clip", () => mode === "idle"))) return;
    clip.ctx = ctxFor(w.mechanic, w.clip.n);
    showClip = clip;
  } catch (e) {
    lastInput = performance.now();
  } finally {
    idleFetching = false;
  }
}

function leaveIdle() {
  mode = "live";
  showClip = null;
  mask = 0xff;
  setBar(0, null);
  lastInput = performance.now();
  showMode();
}

/** The badge over the game and the screen's border: watching the agent,
 * playing, recording, or the agent talking. */
function showMode() {
  const [cls, text] = mode === "idle" ? ["m-watch", "WATCHING YOUR AGENT"]
    : mode === "watch" ? ["m-watch", "WATCHING YOUR AGENT PLAY"]
    : mode === "recording" ? ["m-rec", free ? "RECORDING IN FREE PLAY" : "RECORDING"]
    : mode === "sending" ? ["m-talk", "SENDING"]
    : mode === "review" ? ["m-talk", "YOUR CLIP"]
    : mode === "agent" ? ["m-talk", agentTurn?.mine ? "A CLIP OF THIS MECHANIC" : "YOUR AGENT SAYS"]
    : free ? ["m-play", "FREE PLAY"]
    : ["m-play", "PLAYING"];
  const b = $("modebadge");
  if (b.textContent !== text) b.textContent = text;
  $("screen").dataset.mode = cls;
}
window.addEventListener("keydown", () => { lastInput = performance.now(); if (mode === "idle") leaveIdle(); }, { capture: true });

// Watching the model play ------------------------------------------------

async function pollLive() {
  const { lines } = await api("/api/live");
  if (!lines.length) return;
  const start = lines[0];
  const key = `${start.start}:${start.hint}:${start.log.length}`;
  // A demo that was already there when the page opened isn't replayed.
  if (watch === null) {
    watch = { key, seen: lines.length, queue: [], end: null, old: true };
    return;
  }
  if (watch.key !== key) {
    // A new demo waits while the person records, reviews or answers.
    if (mode !== "live" || free) return;
    const w = { key, seen: 1, queue: [], end: null, last: null, played: 0, start, ready: false };
    watch = w;
    player.replay(1, rom, start.log);
    // Run to the demo's start a slice at a time; its presses queue meanwhile.
    void runToSliced(1, start.start, "Loading the demo", () => watch === w).then((ok) => { if (ok) { w.ready = true; setBar(0, null); } });
    mode = "watch";
    hidePanel();
    $("watch").hidden = false;
    $("watch-goal").textContent = `Your agent's model is playing: ${start.hint}` + (start.steps?.length ? `  [steps: ${start.steps.join(", then ")}]` : "");
    $("watch-pick").textContent = "Thinking...";
  }
  for (const l of lines.slice(watch.seen)) {
    if (l.kind === "decide") {
      for (let i = 0; i < l.frames; i++) watch.queue.push(l.pad);
      const top = Object.entries(l.probabilities ?? {}).sort((a, b) => b[1] - a[1]).slice(0, 3);
      const hs = (l.heights ?? []).map((h, i) => `jump ${i + 1}: ${h} px`).join(", ");
      watch.last = `${l.choice}   (${top.map(([k, p]) => `${k} ${(p * 100).toFixed(0)}%`).join(", ")})   ${l.done} of ${start.times}${l.step ? ", now: " + l.step : ""}${hs ? "   " + hs : ""}`;
    } else if (l.kind === "end") {
      watch.end = l;
    }
  }
  watch.seen = lines.length;
}

function watchFrame() {
  if (!watch || watch.old || !watch.ready) return;
  if (watch.queue.length) {
    player.step(1, watch.queue.shift());
    playFrame(1);
    watch.played += 1;
    setBar(watch.played, undefined);
    if (watch.last) $("watch-pick").textContent = watch.last;
  } else if (watch.end) {
    const e = watch.end;
    watch.end = null;
    $("watch").hidden = true;
    mode = "agent";
    setBar(0, null);
    const hs = (e.heights ?? []).map((h, i) => `jump ${i + 1} rose ${h} px with A held ${e.held?.[i] ?? "?"} frames`).join("; ");
    // The clip it made, replayed under the message until the person goes on.
    const turn = { text: `The model's clip ${e.clip} is done: ${e.done} of ${e.times} shown.${hs ? " " + hs + "." : ""} It plays on a loop above.`, choices: [], show: e.clip };
    void refresh().then(() => showAgent(turn)).catch((err) => setState(String(err)));
  }
}

async function main() {
  player = await NesPlayer.load("/gp_play.wasm");
  const r = await fetch("/rom");
  if (!r.ok) throw new Error(`the ROM: ${await r.text()}`);
  rom = new Uint8Array(await r.arrayBuffer());
  player.power(0, rom);
  showSound();
  $("sound").addEventListener("click", (e) => { setSound(!soundOn); e.currentTarget.blur(); });
  $("tab-mech").addEventListener("click", () => setTab("mech"));
  $("tab-free").addEventListener("click", () => setTab("free"));
  seenTurns = 1e9; // which turns to show is settled just below
  await refresh();
  // Turns already answered before this page opened aren't shown again,
  // except the last one when the agent has the floor.
  const agentTurns = session.talk.filter((t) => t.by === "agent");
  seenTurns = session.waiting_for === "player" ? agentTurns.length - 1 : agentTurns.length;
  if (session.waiting_for === "player" && agentTurns.length) {
    seenTurns = agentTurns.length;
    await showAgent(agentTurns[agentTurns.length - 1]);
  }
  setInterval(() => void refresh().catch((e) => setState(String(e))), 1500);
  setInterval(() => void refreshWatch().catch(() => undefined), 1000);
  setInterval(() => void pollLive().catch(() => undefined), 300);
  setInterval(updateBusy, 1000);
  setInterval(showPad, 1000);
  requestAnimationFrame(frame);
}

// A page that can't start says why where the game would be: the player
// not built, say, with the server's words on how to build it.
main().catch((e) => {
  const why = e instanceof Error ? e.message : String(e);
  setState(why);
  $("title").textContent = "The page couldn't start";
  $("busy").hidden = false;
  $("busy-text").textContent = why;
  document.body.dataset.failed = why;
});
