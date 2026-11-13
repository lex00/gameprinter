// The mechanic page (docs/mechanic-page.md): the game, played live with the
// keyboard or a gamepad, and the agent's requests over it, in three tabs.
// HELP WITH MECHANICS plays the original: the window is the live game, a
// clip being recorded, a clip just recorded to hand off, or the agent's
// words (with choices to pick, or a clip replayed under them), which pause
// the game until they are answered. REVIEW REBUILD shows a clip on the
// original beside the game's model, and plays the model live beside the
// original (PLAY THE REBUILD). FREE PLAY is the model alone. Everything the
// page writes goes to this machine's `gp mechanic start`.
//
// Ported from game-printer's page with all three tabs. game-printer's
// REVIEW REBUILD card was refine.js; that name is the refinement view's now
// (gp model refine and gp rule, /refine), so the card is rebuild-card.js,
// and #rule-card is the slot it draws in.

import { NesPlayer } from "./nesplayer.js";
import { RefineView } from "./rebuild-card.js";
import { startShell } from "./shell.js";

// What the machine is: the screen, the frame rate and the buttons, taken
// from the player module (gp_play_info) when it loads. The NES's until then.
// `keys` and `keys2` are each button's keys in the order of the player's
// bits; `bit` names the game bits the page's own commands read as presses.
const LAYOUTS = {
  nes: {
    fps: 60.0988,
    // A, B, Select, Start, Up, Down, Left, Right (the input log's bits).
    keys: [["KeyX", "Space"], ["KeyZ", "ShiftLeft"], ["ShiftRight", "Tab"], ["Enter"], ["ArrowUp", "KeyW"], ["ArrowDown", "KeyS"], ["ArrowLeft", "KeyA"], ["ArrowRight", "KeyD"]],
    // Controller 2 from the keyboard, as the preview and the Bevy build map
    // it: M and N for A and B, H Select, Y Start, I J K L the d-pad. A second
    // gamepad is controller 2 too.
    keys2: [["KeyM"], ["KeyN"], ["KeyH"], ["KeyY"], ["KeyI"], ["KeyK"], ["KeyJ"], ["KeyL"]],
    bit: { a: 1, b: 2, select: 4, start: 8, up: 16, down: 32, left: 64, right: 128 },
    footer: "Arrows move, X is A, Z is B, Enter is Start. F5 or the left trigger saves, F8 or the right trigger loads. R or the right bumper records. U turns the sound off and on. Player 2: I J K L move, M is A, N is B, H is Select, Y is Start, or a second controller.",
  },
  // An arcade cabinet, as MAME keys it: 5 drops a coin, 1 is the one-player
  // start. Coin, start, right, left, down, up, button 1 (Tapper's tap).
  arcade: {
    fps: 30,
    keys: [["Digit5"], ["Digit1"], ["ArrowRight", "KeyD"], ["ArrowLeft", "KeyA"], ["ArrowDown", "KeyS"], ["ArrowUp", "KeyW"], ["KeyX", "Space"]],
    keys2: [],
    bit: { a: 64, b: 0, select: 1, start: 2, up: 32, down: 16, left: 8, right: 4 },
    footer: "5 drops a coin and 1 starts, as in MAME. Arrows move, X or Space taps. On a controller: Select is the coin, Start starts, A taps. F5 or the left trigger saves, F8 or the right trigger loads. R or the right bumper records. U turns the sound off and on.",
  },
};
let LAYOUT = LAYOUTS.nes;
let FPS = LAYOUT.fps;
let KEYS = LAYOUT.keys;
let KEYS2 = LAYOUT.keys2;
let W = 256, H = 240;

/** Takes the machine's numbers from the player's `gp_play_info`. */
function useMachine(info) {
  LAYOUT = LAYOUTS[info.platform] ?? LAYOUTS.nes;
  FPS = info.fps ? info.fps[0] / info.fps[1] : LAYOUT.fps;
  KEYS = LAYOUT.keys;
  KEYS2 = LAYOUT.keys2;
  W = info.width ?? 256;
  H = info.height ?? 240;
  for (const c of [canvas, $("game2"), ...off, picCanvas, rbOff]) { c.width = W; c.height = H; c.style.aspectRatio = `${W} / ${H}`; }
  $("keys").textContent = LAYOUT.footer;
  document.body.dataset.platform = info.platform ?? "nes";
}
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
let mode = "live"; // live | recording | review | sending | agent | idle | watch | refine
// REFINE or THEMES shows in the game's place (shell.js): the games hold and
// the keys and the controller are theirs.
let paneOpen = false;
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
let measure = null; // the shown clip's measurements, drawn over it
// Two clips shown together: the older in console 0 (the live game saved
// aside), the newer in console 1. view: overlay | side | diff.
let compare = null; // {old: {log, start, end}, saved}
// The workspace game view's three layouts (editor/src/GameView.tsx): side
// by side, the rebuilt game on top of the original at a transparency, and
// their difference, black where the two match. Kept in the browser.
const VIEWS = ["side", "top", "diff"];
const VIEW_WORDS = { side: "side by side", top: "the model on top", diff: "the differences", solo: "the model only" };
// Playing the rebuild live adds the rebuild by itself to the cycle.
const LIVE_VIEWS = ["side", "top", "diff", "solo"];
let view = "side";
let opacity = 0.5;
let showBoxes = false; // the rebuild's entities outlined, a layer over its picture
try {
  const v = JSON.parse(localStorage.getItem("gp-mechanic-view") ?? "null");
  if (v && LIVE_VIEWS.includes(v.view)) { view = v.view; opacity = Math.min(1, Math.max(0, Number(v.opacity) || 0.5)); showBoxes = !!v.boxes; }
} catch (e) { /* no storage: the defaults */ }
function keepView() {
  try { localStorage.setItem("gp-mechanic-view", JSON.stringify({ view, opacity, boxes: showBoxes })); } catch (e) { /* not kept */ }
  syncReviewBar();
}
// The agent's model playing, watched as it happens: its demo's start, the
// presses still to play, the decisions seen and the overlay's words.
let watch = null; // {key, seen, queue: [pad], end, last}

// Buttons still down from closing a panel: Enter or A answered it, and
// mustn't also reach the game as Start or A (Start pauses it).
let mask = 0;
/** Both controllers for the game: controller 1's bits in the low byte,
 * controller 2's in the next, as gp_play_step and the rebuild take them. */
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

/** Controller 2: its keys and the second gamepad. An arcade board has one
 * player's controls here. */
function rawPad2() {
  if (!KEYS2.length) return 0;
  let pad = 0;
  KEYS2.forEach((ks, bit) => { if (ks.some((k) => down.has(k))) pad |= 1 << bit; });
  return pad | gamepadBits(secondPad());
}

/** A gamepad's buttons as the machine's bits, its left stick as the
 * d-pad. On an arcade board: Select is the coin, Start the start and A or
 * B the button. */
function gamepadBits(g) {
  let pad = 0;
  if (g && LAYOUT === LAYOUTS.arcade) {
    const b = (i) => g.buttons[i]?.pressed === true;
    const ax = g.axes[0] ?? 0, ay = g.axes[1] ?? 0;
    if (b(8)) pad |= 1;
    if (b(9)) pad |= 2;
    if (b(15) || ax > 0.5) pad |= 4;
    if (b(14) || ax < -0.5) pad |= 8;
    if (b(13) || ay > 0.5) pad |= 16;
    if (b(12) || ay < -0.5) pad |= 32;
    if (b(0) || b(1) || b(2) || b(3)) pad |= 64;
  } else if (g) {
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

/** A pad as a frames file's line: controller 1's bits in hex, then
 * controller 2's when it holds any (docs/generator.md). */
function framesLine(p) {
  const hex = (b) => b.toString(16).padStart(2, "0");
  return p >> 8 ? `${hex(p & 0xff)} ${hex(p >> 8)}` : hex(p & 0xff);
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
for (const c of off) { c.width = W; c.height = H; }

function draw(slot) {
  const img = new ImageData(player.rgba(slot), W, H);
  ctx.putImageData(img, 0, 0);
}

/** The two clips of a comparison in the chosen view: the newer (console 1)
 * over the older (console 0), side by side, or their difference. */
function drawCompare() {
  if (view === "solo") view = "side";
  drawPair(player.rgba(0), player.rgba(1));
}

/** Two pictures in the chosen view: side by side (a left), `b` over `a` at
 * the chosen transparency, or their difference. `marks`, when given, draws
 * over `b` in every view. With `visible` (left, top, width, height: the
 * part both games draw, which the screen score compares), the differences
 * are ON THE MODEL's: the original dimmed where the two agree, magenta
 * where a pixel differs, and the count goes to `pixelsDiffer`; without it,
 * each color's distance, black where they match. */
let pixelsDiffer = null;
function drawPair(a, b, marks, visible) {
  $("screen").classList.toggle("side", view === "side");
  $("game2").hidden = view !== "side";
  pixelsDiffer = null;
  if (view === "side") {
    ctx.putImageData(new ImageData(a, W, H), 0, 0);
    const c2 = $("game2").getContext("2d");
    c2.putImageData(new ImageData(b, W, H), 0, 0);
    marks?.(c2);
  } else if (view === "solo") {
    ctx.putImageData(new ImageData(b, W, H), 0, 0);
    marks?.(ctx);
  } else if (view === "top") {
    off[0].getContext("2d").putImageData(new ImageData(a, W, H), 0, 0);
    off[1].getContext("2d").putImageData(new ImageData(b, W, H), 0, 0);
    ctx.globalAlpha = 1;
    ctx.drawImage(off[0], 0, 0);
    ctx.globalAlpha = opacity;
    ctx.drawImage(off[1], 0, 0);
    ctx.globalAlpha = 1;
    marks?.(ctx);
  } else if (visible) {
    const out = new Uint8ClampedArray(a);
    const [vl, vt, vw, vh] = visible;
    let n = 0;
    for (let i = 0; i < a.length; i += 4) {
      const x = (i / 4) % W, y = Math.floor(i / 4 / W);
      const inside = x >= vl && x < vl + vw && y >= vt && y < vt + vh;
      if (!inside || (a[i] === b[i] && a[i + 1] === b[i + 1] && a[i + 2] === b[i + 2])) {
        out[i] >>= 2; out[i + 1] >>= 2; out[i + 2] >>= 2;
      } else {
        n++;
        out[i] = 255; out[i + 1] = 0; out[i + 2] = 255;
      }
      out[i + 3] = 255;
    }
    pixelsDiffer = n;
    ctx.putImageData(new ImageData(out, W, H), 0, 0);
    marks?.(ctx);
  } else {
    const out = new Uint8ClampedArray(a.length);
    for (let i = 0; i < a.length; i += 4) {
      out[i] = Math.abs(a[i] - b[i]); out[i + 1] = Math.abs(a[i + 1] - b[i + 1]); out[i + 2] = Math.abs(a[i + 2] - b[i + 2]); out[i + 3] = 255;
    }
    ctx.putImageData(new ImageData(out, W, H), 0, 0);
    marks?.(ctx);
  }
}

// The rebuild beside the original: a clip played by the server through the
// original and the game's model, as `gp clip score` plays it straight
// through with the original's random draws (/api/clips/N.rebuild.json, the
// same review as ON THE MODEL keeps beside the clip). One entry a compared
// frame, in order: the original's frame it stands at, the two pictures'
// addresses, whether the mapped state agrees, the model's entities and
// what differs.
let rebuild = null; // {id, frames: [entry], byF: Map original frame -> k, visible, note, draws, drawsDesync}

/** Loads a clip's rebuild for the shown clip; the hint counts the wait. */
async function loadRebuild(n, mechanic, still) {
  const q = mechanic ? `?mechanic=${encodeURIComponent(mechanic)}` : "";
  return loadRebuildFrom(`api/clips/${n}.rebuild.json${q}`, still);
}

/** Loads the rebuild of the clip shown from `url`; the hint counts the wait. */
async function loadRebuildFrom(url, still) {
  const t0 = performance.now();
  const line = mode === "refine" ? { get: () => "", set: (t) => { refineMsg = t; $("busy-text").textContent = t; } }
    : { get: () => $("hint").textContent, set: (t) => { $("hint").textContent = t; } };
  const base = line.get();
  const tick = setInterval(() => {
    if (!still()) return;
    line.set(`${base} Playing the clip on the original and the model, ${((performance.now() - t0) / 1000).toFixed(1)} s (a long clip takes a while the first time).`.trim());
  }, 100);
  try {
    const r = await api(url);
    if (!still()) return;
    const frames = r.frames ?? [];
    rebuild = {
      id: url, frames, byF: new Map(frames.map((f) => [f.f, f.k])), visible: r.visible ?? null,
      note: r.note ?? [], draws: r.draws?.list ?? [], drawsDesync: r.draws?.desync ?? null,
      same: r.together_count ?? 0,
    };
    if (mode === "refine") drawStrip();
    line.set(`${base} ${mode === "refine" ? `${rebuild.same} of ${frames.length} frames the same. A plays and pauses, left and right step, up and down set the transparency, Select changes the layout.` : "Left and right: side by side, on top, the differences."}${mode === "refine" && rebuild.note.length ? " The model: " + rebuild.note.join("; ") + "." : ""}`.trim());
    if (mode !== "refine" && rebuild.note.length) $("say").textContent += `\n\nThe model: ${rebuild.note.join("; ")}.`;
    cycleView(0);
  } catch (e) {
    if (still()) line.set(`${base} The model couldn't play it: ${String(e.message ?? e).replace(/^[^ ]+: \d+ /, "")}`.trim());
  } finally {
    clearInterval(tick);
  }
}

// The pictures as pixels, loaded when first asked for and the last few
// dozen kept: a long clip has thousands, too many to hold.
const picCache = new Map(); // url -> pixels, or null while loading
const picCanvas = document.createElement("canvas");
picCanvas.width = W; picCanvas.height = H;
function picData(url) {
  if (!url) return null;
  if (picCache.has(url)) return picCache.get(url);
  picCache.set(url, null);
  const img = new Image();
  img.onload = () => {
    const g = picCanvas.getContext("2d", { willReadFrequently: true });
    g.clearRect(0, 0, W, H);
    g.drawImage(img, 0, 0);
    picCache.set(url, g.getImageData(0, 0, W, H).data);
    if (picCache.size > 64) picCache.delete(picCache.keys().next().value);
  };
  img.onerror = () => picCache.delete(url);
  img.src = url;
  return null;
}

/** The compared frame the clip view stands at: REVIEW REBUILD's clips step
 * by the review's own frames; a clip under the agent's words plays in
 * console 1, and stands at the nearest compared frame at or before it. */
function clipAt() {
  return showClip?.server ? review.f : player.frame(1);
}
function entryAt(at) {
  if (!rebuild) return null;
  if (showClip?.server) return rebuild.frames[at] ?? null;
  for (let f = at; f >= (showClip?.start ?? 0) - 1; f--) {
    const k = rebuild.byF.get(f);
    if (k !== undefined) return rebuild.frames[k];
  }
  return rebuild.frames[0] ?? null;
}

function drawRebuild() {
  if (view === "solo") view = "side";
  const at = clipAt();
  const e = entryAt(at);
  const blank = new Uint8ClampedArray(W * H * 4);
  // The original: the server's picture in REVIEW REBUILD, console 1's
  // under the agent's words.
  const o = showClip?.server ? picData(e?.o) : player.rgba(1);
  const m = picData(e?.m);
  // The next frames load while this one shows.
  if (showClip?.server) for (let j = at + 1; j < Math.min(rebuild.frames.length, at + 8); j++) { picData(rebuild.frames[j].o); picData(rebuild.frames[j].m); }
  drawPair(o ?? blank, m ?? blank, (g) => {
    g.save();
    g.lineWidth = 1;
    g.font = "8px monospace";
    if (showBoxes) {
      for (const [x, y, w, h, name] of e?.boxes ?? []) {
        g.strokeStyle = "rgb(252,160,68)";
        g.strokeRect(x + 0.5, y + 0.5, Math.max(1, w - 1), Math.max(1, h - 1));
        g.fillStyle = "rgb(252,160,68)";
        g.fillText(name, x, y - 2);
      }
    }
    if (!e?.m) {
      g.fillStyle = "rgba(0,0,0,0.75)";
      g.fillRect(2, 14, 240, 11);
      g.fillStyle = "rgb(252,160,68)";
      g.fillText("NO PICTURE OF THE MODEL ON THIS FRAME", 5, 22);
    }
    if (mode === "refine" && refineSplit && Math.abs((e?.f ?? 0) - refineSplit.frame) <= 45) {
      const t = `FIRST SPLIT${refineSplit.what ? ": " + refineSplit.what : ""}`;
      g.fillStyle = "rgba(0,0,0,0.75)";
      g.fillRect(2, 2, Math.min(W - 4, g.measureText(t).width + 6), 11);
      g.fillStyle = "rgb(248,56,0)";
      g.fillText(t, 5, 10);
    }
    const label = view === "side" ? "MODEL" : view === "top" ? `MODEL ON TOP, ${Math.round(opacity * 100)}%`
      : pixelsDiffer === null ? "DIFFERENCE: MAGENTA WHERE THEY DIFFER" : `DIFFERENCE: ${pixelsDiffer ? pixelsDiffer + " PIXELS DIFFER" : "EVERY PIXEL THE SAME"}`;
    g.fillStyle = "rgba(0,0,0,0.7)";
    g.fillRect(2, H - 10, g.measureText(label).width + 6, 10);
    g.fillStyle = "rgb(252,160,68)";
    g.fillText(label, 4, H - 2);
    g.restore();
  }, rebuild.visible ?? [0, 0, W, H]);
}

/** The strip under the game in REVIEW REBUILD (ON THE MODEL's): a mark a
 * compared frame, green where the original's and the model's mapped state
 * agree and red where they differ, and a white line where the review
 * stands. A click goes there. */
function drawStrip() {
  const c = $("rv-strip");
  if (!c) return;
  c.hidden = mode !== "refine" || !rebuild || !showClip?.server;
  if (c.hidden) return;
  const w = c.clientWidth || 600;
  if (c.width !== w) c.width = w;
  const g = c.getContext("2d");
  const n = rebuild.frames.length;
  g.fillStyle = "#1c1c1c";
  g.fillRect(0, 0, w, c.height);
  for (let k = 0; k < n; k++) {
    const x = Math.floor((k / n) * w);
    g.fillStyle = rebuild.frames[k].together ? "#00a800" : "#f83800";
    g.fillRect(x, 0, Math.max(1, Math.floor(((k + 1) / n) * w) - x), c.height);
  }
  g.fillStyle = "#fcfcfc";
  g.fillRect(Math.floor((review.f / Math.max(1, n)) * w), 0, 2, c.height);
}

/** The next compared frame where the mapped state differs, after (`dir`
 * 1) or before (-1) the one shown. */
function nextDiffer(dir) {
  if (!rebuild || !showClip?.server) return;
  for (let k = review.f + dir; k >= 0 && k < rebuild.frames.length; k += dir) {
    if (!rebuild.frames[k].together) { review.playing = false; seekReview(k); return; }
  }
  toast(dir > 0 ? "No difference after this frame." : "No difference before this frame.");
}

/** The measurements over the clip as it plays: every sprite box, the
 * player's in blue with the path its top has taken, and each hit marked for
 * a second after it with what it did. */
function drawMeasure(at) {
  const fr = measure.frames;
  if (at < 0 || at >= fr.length) return;
  ctx.save();
  ctx.lineWidth = 1;
  ctx.strokeStyle = "rgba(255,255,255,0.45)";
  for (const [x, y, w, h] of fr[at].boxes) ctx.strokeRect(x + 0.5, y + 0.5, w, h);
  ctx.strokeStyle = "rgba(108,180,255,0.9)";
  ctx.beginPath();
  let pen = false;
  for (let i = 0; i <= at; i++) {
    const p = fr[i].player;
    if (p === null || p === undefined) { pen = false; continue; }
    const [x, y, w] = fr[i].boxes[p];
    if (pen) ctx.lineTo(x + w / 2, y); else ctx.moveTo(x + w / 2, y);
    pen = true;
  }
  ctx.stroke();
  const p = fr[at].player;
  if (p !== null && p !== undefined) {
    const [x, y, w, h] = fr[at].boxes[p];
    ctx.lineWidth = 2;
    ctx.strokeRect(x, y, w, h);
  }
  ctx.font = "8px monospace";
  for (const r of measure.rests ?? []) {
    if (at < r.from) continue;
    // The height it stood at, as a line where the player's box stood.
    const q = fr[r.from].player;
    const b = q === null || q === undefined ? null : fr[r.from].boxes[q];
    if (!b) continue;
    const y = b[1] + b[3];
    ctx.strokeStyle = "rgba(120,230,140,0.9)";
    ctx.lineWidth = 1;
    ctx.beginPath(); ctx.moveTo(b[0] - 12, y + 0.5); ctx.lineTo(b[0] + b[2] + 12, y + 0.5); ctx.stroke();
    if (at <= r.to + 30) {
      ctx.fillStyle = "rgb(140,240,160)";
      ctx.fillText(r.up_px ? `stood ${r.up_px} px up` : "ground", b[0] - 12, y + 9);
    }
    if (r.then && at > r.to && at <= r.to + r.then.frames + 40) {
      const label = `${r.then.how === "jump" ? "jumped" : r.then.how === "fall" ? "walked off, fell" : "rose"} ${Math.abs(r.then.to_up_px - r.up_px)} px in ${r.then.frames} frames`;
      ctx.fillStyle = "rgba(0,0,0,0.7)";
      ctx.fillRect(4, 18, ctx.measureText(label).width + 6, 11);
      ctx.fillStyle = "rgb(140,240,160)";
      ctx.fillText(label, 7, 26);
    }
  }
  for (const w of measure.walls ?? []) {
    if (at < w.frame || at > w.frame + 60) continue;
    const q = fr[w.frame]?.player;
    const b = q === null || q === undefined ? null : fr[w.frame].boxes[q];
    if (!b) continue;
    const x = w.side === "right" ? b[0] + b[2] + 1 : b[0] - 1;
    ctx.strokeStyle = "rgb(255,80,80)";
    ctx.lineWidth = 2;
    ctx.beginPath(); ctx.moveTo(x, b[1] - 6); ctx.lineTo(x, b[1] + b[3] + 6); ctx.stroke();
    const label = `ran into something on the ${w.side} at ${w.speed_px_a_frame} px a frame, ${w.in_air ? "in the air" : "on the ground"}`;
    ctx.fillStyle = "rgba(0,0,0,0.7)";
    ctx.fillRect(4, 32, ctx.measureText(label).width + 6, 11);
    ctx.fillStyle = "rgb(255,120,120)";
    ctx.fillText(label, 7, 40);
  }
  const banner = (text, y, color) => {
    ctx.fillStyle = "rgba(0,0,0,0.7)";
    ctx.fillRect(4, y - 8, ctx.measureText(text).width + 6, 11);
    ctx.fillStyle = color;
    ctx.fillText(text, 7, y);
  };
  // Items: the path of what came out of a block, drawn as it goes, with
  // its box in yellow and a mark where it touched the player.
  for (const it of measure.items ?? []) {
    if (at < it.frame) continue;
    const pts = it.path.filter(([f]) => f <= at);
    if (!pts.length) continue;
    ctx.strokeStyle = "rgba(248,216,0,0.8)";
    ctx.lineWidth = 1;
    ctx.beginPath();
    pts.forEach(([, x, y], n) => (n ? ctx.lineTo(x + 8, y + 8) : ctx.moveTo(x + 8, y + 8)));
    ctx.stroke();
    const [, lx, ly] = pts[pts.length - 1];
    const touched = it.touch_frame !== undefined && at >= it.touch_frame;
    if (!touched) {
      ctx.lineWidth = 2;
      ctx.strokeRect(lx, ly, it.size[0], it.size[1]);
    }
    const leg = (it.legs ?? []).filter((l) => l.from <= at && l.dx_px !== 0).pop();
    banner(touched
      ? `item touched the player: box ${it.player_h[0]} to ${it.player_h[1]} px tall, ${it.frozen_frames} frames frozen`
      : `item out of the block${leg ? `, moving ${leg.px_a_frame} px a frame` : ""}`, 82, "rgb(248,216,0)");
  }
  // Stomps: the spot under the feet, boxed in orange, and what the thing
  // there did.
  for (const st of measure.stomps ?? []) {
    if (at < st.frame || at > st.frame + 60) continue;
    const [x, y, w, h] = st.box;
    ctx.strokeStyle = "rgb(252,152,56)";
    ctx.lineWidth = 2;
    ctx.strokeRect(x, y, w, h);
    banner(`landed on something: bounced ${st.bounced_px} px, the thing under him ${st.under}`, 54, "rgb(252,180,100)");
  }
  // Hurts: a change to the player's box (shrank, grew, blinked, or left
  // the screen at the bottom).
  for (const hu of measure.hurts ?? []) {
    if (at < hu.frame || at > hu.frame + 90) continue;
    const q = fr[Math.max(0, hu.frame - 1)]?.player;
    const b = q === null || q === undefined ? null : fr[Math.max(0, hu.frame - 1)].boxes[q];
    if (b) {
      ctx.strokeStyle = "rgb(248,56,0)";
      ctx.setLineDash([3, 2]);
      ctx.strokeRect(b[0] - 2, b[1] - 2, b[2] + 4, b[3] + 4);
      ctx.setLineDash([]);
    }
    const what = hu.kind === "blinked" ? `blinked for ${hu.frames} frames`
      : hu.kind === "fell off the screen" ? "fell off the bottom of the screen"
      : `${hu.kind}: ${hu.from_h} to ${hu.to_h} px tall`;
    banner(`the player's box ${what}`, 68, "rgb(255,120,90)");
  }
  measure.hits.forEach((h, n) => {
    if (at < h.frame || at > h.frame + 60) return;
    const q = fr[h.frame].player;
    const b = q === null || q === undefined ? null : fr[h.frame].boxes[q];
    ctx.strokeStyle = ctx.fillStyle = "rgb(255,80,80)";
    ctx.lineWidth = 2;
    if (b) { ctx.beginPath(); ctx.moveTo(b[0] - 6, b[1] - 1); ctx.lineTo(b[0] + b[2] + 6, b[1] - 1); ctx.stroke(); }
    const lasting = [...h.at_hit, ...h.after].filter((c) => c.lasting).length;
    const label = `hit ${n + 1}: rose ${h.rose_px} px, ${h.sprites_added >= 0 ? "+" : ""}${h.sprites_added} sprites, ${lasting} bytes kept`;
    ctx.fillStyle = "rgba(0,0,0,0.7)";
    ctx.fillRect(4, 4, ctx.measureText(label).width + 6, 11);
    ctx.fillStyle = "rgb(255,120,120)";
    ctx.fillText(label, 7, 12);
  });
  ctx.restore();
}

function endCompare() {
  rebuild = null;
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
  if (liveRb) liveRb.labelUntil = performance.now() + 3000;
  const list = liveRb?.kind === "beside" ? LIVE_VIEWS : VIEWS;
  const i = list.indexOf(view);
  view = list[((i < 0 ? 0 : i) + d + list.length) % list.length];
  keepView();
  toast(`${liveRb ? "Playing the model" : rebuild ? "Original and model" : "Two clips"}: ${VIEW_WORDS[view]}.`, 1500);
}

function setOpacity(o) {
  opacity = Math.min(1, Math.max(0, Math.round(o * 10) / 10));
  if (liveRb) liveRb.labelUntil = performance.now() + 3000;
  if (view !== "top") view = "top";
  keepView();
  toast(`The model on top at ${Math.round(opacity * 100)}%.`, 1200);
}

/** A route's JSON; a failure throws the server's own words (its `error`). */
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
  $("bar-time").textContent = of === undefined ? `${(at / FPS).toFixed(1)} s` : `${(at / FPS).toFixed(1)} of ${(of / FPS).toFixed(1)} s`;
}

/** The banner while it's the agent's turn, with what it says it is doing. */
let busySince = null;
function updateBusy() {
  // Always says whose turn it is and what you can do now.
  $("busy").hidden = !session;
  if (!session || mode === "idle") return;
  if (mode === "refine") { $("busy").classList.add("theirs"); $("busy-text").textContent = refineMsg ?? refine.statusText(); return; }
  const agents = session.waiting_for === "agent" && session.talk.length > 0;
  $("busy").classList.toggle("theirs", agents);
  const waiting = session.talk.filter((t) => t.by === "agent").length > seenTurns;
  if (liveRb && (mode === "live" || mode === "recording")) {
    const what = liveRb.kind === "free"
      ? "Free play: the model alone, with its own random numbers. The original follows your presses unseen, so a clip you record can be reviewed against it."
      : `The model beside the original, ${VIEW_WORDS[view]}. V or the left bumper changes the layout, [ and ] the transparency.`;
    const rec = mode === "recording" ? " Recording: R or the right bumper stops." : " R or the right bumper records; Esc stops playing the model.";
    if (liveRb.on) $("busy-text").textContent = what + rec + (waiting ? " Your agent has a message: Esc to see it." : "");
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

// The margins: the clip loaded, and this frame ----------------------------

// Left of the game, the clip loaded: its row, note and scores, and its
// first split. Right of it, while a clip plays beside the rebuild, what the
// two games disagree on at this frame (measure, original, rebuild) and the
// random draws taken so far. Both are empty when nothing is loaded.

/** A clip's context: its scores from the refinement status, its note
 * from the open session's clips. */
function ctxFor(mechanic, n) {
  const scored = refine?.data?.clips?.find((x) => x.mechanic === mechanic && x.clip === n) ?? {};
  const meta = mechanic === session?.mechanic ? session?.clips?.find((x) => x.n === n) : null;
  return { title: `${mechanic} clip ${n}`, row: mechanic, note: meta?.note, start: meta?.start ?? scored.start, ...scored };
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
  const viewing = showClip && (mode === "agent" || mode === "idle" || mode === "refine") ? showClip : null;
  const live = (mode === "live" || mode === "recording") && liveRb?.on;
  const at = viewing ? clipAt() : -1;
  // Redrawn every frame when paused, every sixth while playing.
  const tick = mode === "refine" && !review.playing ? at : Math.floor(at / 6);
  const key = viewing ? `${viewing.ctx?.title}|${tick}|${rebuild ? 1 : 0}` : live ? "live" : "";
  if (key === ctxKey) return;
  const left = $("ctx-clip"), right = $("ctx-frame");
  if (!left || !right) return;
  const clipChanged = key.split("|")[0] !== ctxKey.split("|")[0] || key.split("|")[2] !== ctxKey.split("|")[2];
  ctxKey = key;
  if (clipChanged) {
    left.innerHTML = "";
    if (viewing) {
      const c = viewing.ctx ?? {};
      const pct = (x) => (x && x.pct !== null && x.pct !== undefined ? `${x.pct.toFixed(1)}%` : "not scored");
      const secs = (f) => `${(f / FPS).toFixed(1)} s`;
      el(left, "h3", c.title ?? "This clip");
      const t = el(left, "table");
      const row = (k, v) => { const tr = el(t, "tr"); el(tr, "td", k); el(tr, "td", v, "v"); };
      row("row", c.row ?? "");
      if (c.straight !== undefined) row("straight through", pct(c.straight));
      if (c.resync !== undefined) row("resync", pct(c.resync));
      if (c.longest_exact !== undefined) row("longest exact", c.longest_exact ? secs(c.longest_exact.frames) : "not measured");
      row("length", secs(viewing.end - viewing.start));
      if (c.note) el(left, "p", `"${c.note}"`, "muted");
      const sp = c.first_split;
      if (sp) el(left, "p", `First split at ${secs(sp.clip_frame ?? sp.frame - (c.start ?? viewing.start))}: ${sp.what ?? "a measure differs"}`, "bad");
      else if (c.straight) el(left, "p", "No lasting split.", "muted");
      if (rebuild?.drawsDesync) el(left, "p", `The model's random draws leave the original's at frame ${rebuild.drawsDesync.frame}.`, "bad");
    } else if (live) {
      el(left, "h3", "Playing the model");
      el(left, "p", "You play the model; the original takes the same presses. Measures aren't taken while playing live: record a clip and review it to see them.", "muted");
    }
  }
  right.innerHTML = "";
  if (!viewing || !rebuild) return;
  const e = entryAt(at);
  const k = e?.k ?? 0;
  el(right, "h3", `Frame ${k + 1} of ${rebuild.frames.length} (the original's ${e?.f ?? "?"}): ${e?.together ? "the same" : "what differs"}`);
  const diffs = e?.diffs ?? [];
  if (!diffs.length) el(right, "p", e?.together === false ? "The mapped state differs, but no measure is out of its tolerance." : "Nothing measured differs.", "muted");
  else {
    const t = el(right, "table");
    const h = el(t, "tr"); el(h, "td", "measure", "muted"); el(h, "td", "original", "v muted"); el(h, "td", "model", "v muted");
    for (const [path, o, m] of diffs) { const tr = el(t, "tr"); el(tr, "td", path); el(tr, "td", o, "v"); el(tr, "td", m, "v bad"); }
  }
  const first = rebuild.frames[0]?.f ?? 0;
  const taken = rebuild.draws.filter((d) => d[0] <= (e?.f ?? 0));
  el(right, "h3", `Random draws so far: ${taken.length}`);
  if (taken.length) {
    const t = el(right, "table");
    for (const [f, site, v] of taken.slice(-8)) { const tr = el(t, "tr"); el(tr, "td", `${((f - first) / FPS).toFixed(2)} s`, "muted"); el(tr, "td", site); el(tr, "td", v, "v"); }
  }
}

// Playing the rebuild ---------------------------------------------------

// The model played live beside the original (REVIEW REBUILD's PLAY THE
// REBUILD): the interpreter preview of the model (/rebuild/, built on
// request) in control mode in a hidden frame. It starts from the original's
// state where the person is, translated by the server, and takes each
// frame the original reads the pad on, so both games get the same presses.
// Recording and handing off work as in plain play: the clip is the
// original's play.
let liveRb = null; // {kind, on, queue: [pad] or null, ready, visible, labelUntil}
// The game drawn on the last frame: original, rebuild, original+rebuild,
// clip, clip+rebuild or none.
let frameSource = "original";

async function rbPlay(kind = "beside") {
  if (mode === "recording" || mode === "review" || mode === "sending" || mode === "sealed") { toast("Finish the recording first."); return; }
  if (liveRb) rbStop(true);
  if (mode === "refine") leaveRefine();
  if (mode === "idle") leaveIdle();
  if (mode === "agent") { toast("Answer your agent first."); return; }
  if (mode !== "live") { toast("Back to the game first."); return; }
  // FREE PLAY is the rebuild alone, full size; beside, the last layout.
  liveRb = { kind, on: false, queue: null, ready: false };
  if (kind === "free") view = "solo";
  else if (view === "solo") view = "side";
  syncTabs();
  try {
    let st = await api("api/rebuild/status");
    if (st.built && (st.stale || st.building !== null)) {
      // The model moved on since the last build: play that build now and
      // make a newer one beside it; the next PLAY uses it.
      await api("api/rebuild/build", {});
      liveRb.olderBuild = true;
      toast("A newer build of the model is building; you're playing the previous one.", 5000);
    } else if (!st.built) {
      // No build yet: the first one has to finish before anything plays.
      await api("api/rebuild/build", {});
      while (liveRb && (st = await api("api/rebuild/status")).building !== null) {
        $("busy-text").textContent = `Building the playable model: ${Math.round(st.building)} s (the first build takes a few minutes). Keep playing.`;
        toast(`Building the playable model, ${Math.round(st.building)} s.`, 2500);
        await new Promise((r) => setTimeout(r, 2000));
      }
      if (!liveRb) return;
      if (st.error || !st.built) throw new Error(st.error ?? "the build didn't finish");
    }
    try {
      const pj = await api("rebuild/preview.json");
      if (liveRb && Array.isArray(pj.visible)) liveRb.visible = pj.visible;
    } catch (e) { /* drawn from the corner */ }
    const f = $("rb-frame");
    f.onload = null;
    // FREE PLAY runs the model in the viewer's settings; beside the
    // original it is the model as written, which the clips measure.
    if (kind === "free") {
      if (!profiles) profiles = await api("api/rebuild/profiles").catch(() => null);
      if (!liveRb) return;
      liveRb.profile = profileName();
    }
    await new Promise((resolve, reject) => {
      const t = setTimeout(() => reject(new Error("the model didn't load")), 30000);
      liveRb.onReady = () => { clearTimeout(t); resolve(); };
      const prof = kind === "free" && liveRb.profile !== (profiles?.base ?? "retro") ? `&profile=${encodeURIComponent(liveRb.profile)}` : "";
      // files=here: the preview reads the build beside it, not the
      // browser's store of installed games.
      f.src = `rebuild/index.html?control=1&files=here${prof}&t=${Date.now()}`;
    });
    if (kind === "free") rbStartFree(); else await rbStart();
  } catch (e) {
    const why = `The model couldn't start: ${e.message ?? e}`;
    toast(why, 8000);
    setState(why);
    rbStop(true);
    if (mode === "live") setTab(kind === "beside" ? "refine" : "mech");
  }
}

/** Loads the original's state here into the model, then lets presses through. */
/** FREE PLAY: the rebuild from its own start, owing nothing to the
 * original's state, so it never waits on the original (on its title screen,
 * or in a stage the rebuild hasn't got) and never shows it. */
function rbStartFree() {
  if (!liveRb) return;
  liveRb.queue = [];
  rbPost({ gp: "frames", text: "gp-frames 1\n", frame: 0 });
  liveRb.on = true;
  liveRb.labelUntil = performance.now() + 3000;
  const older = liveRb.olderBuild ? " A newer build of the model is building; you're playing the previous one." : "";
  toast("Free play: the model alone, from its own start. R records, Esc stops." + older, older ? 7000 : 4000);
}

async function rbStart() {
  if (!liveRb) return;
  if (liveRb.kind === "free") { rbStartFree(); return; }
  const frame = player.frame(0);
  liveRb.on = false;
  liveRb.queue = [];
  try {
    const path = await sendPath(player.inputLog(0).split("\n").slice(0, frame + 2).join("\n") + "\n");
    const r = await api("api/rebuild/start", { path, frame });
    if (!liveRb) return;
    const frames = "gp-frames 1\n" + liveRb.queue.map(framesLine).join("\n");
    liveRb.queue = [];
    rbPost({ gp: "load-state", state: r.state, text: frames + "\n", frame: 1e9 });
    liveRb.on = true;
    liveRb.labelUntil = performance.now() + 3000;
    const older = liveRb.olderBuild ? " A newer build of the model is building; you're playing the previous one." : "";
    toast((liveRb.kind === "free"
      ? "Free play: the model alone. R records, Esc stops."
      : `Playing the model from here${r.missing ? ` (${r.missing} parts of the state not carried over)` : ""}. V or the left bumper changes the layout, [ and ] the transparency, Esc stops.`) + older, older ? 7000 : 4000);
  } catch (e) {
    // Not in play yet (the title screen): try again in a second.
    if (!liveRb) return;
    liveRb.queue = null;
    $("busy-text").textContent = `The model joins once the game is in play: ${String(e).slice(0, 120)}`;
    setTimeout(() => { if (liveRb && !liveRb.on) void rbStart(); }, 1000);
  }
}

function rbPost(m) {
  $("rb-frame").contentWindow?.postMessage(m, location.origin);
}

/** Stops playing the rebuild. Esc from beside the original goes back to
 * REVIEW REBUILD; from FREE PLAY, back to the game. `quiet` when another
 * mode takes over at once. */
function rbStop(quiet = false) {
  const kind = liveRb?.kind;
  liveRb = null;
  $("rb-frame").src = "about:blank";
  $("screen").classList.remove("side");
  $("game2").hidden = true;
  if (view === "solo") view = "side";
  if (!quiet && kind === "beside" && mode === "live") { setTab("refine"); return; }
  syncTabs();
  if (!quiet) toast("Playing the original.");
}

/** The tab lit for the mode: REVIEW REBUILD for its clip review and for
 * playing the rebuild beside the original, FREE PLAY for the rebuild
 * alone, HELP WITH MECHANICS for the rest. */
function syncTabs() {
  const t = mode === "refine" || liveRb?.kind === "beside" ? "refine" : liveRb?.kind === "free" ? "free" : "mech";
  $("tab-mech").classList.toggle("on", t === "mech");
  $("tab-refine").classList.toggle("on", t === "refine");
  $("tab-free")?.classList.toggle("on", t === "free");
  $("settings").hidden = t !== "free";
  if (t === "free") void showSettings();
}

// FREE PLAY's settings ---------------------------------------------------

// The rebuild's own settings screen (docs/settings.md) is the source: F1 or
// the right stick's press in FREE PLAY opens it inside the game, where the
// preview draws it (gp_present::settings), and the page hands it the keys
// until it closes. The panel under the game only says what the rebuild
// plays and opens the same screen. The setting is kept per viewer and game
// in the browser (`gp-settings:MODEL`, the preview's own key). The rebuild
// applies it to its model as it loads (preview.js, `profile=`), so a change
// starts FREE PLAY again from power-on; the original, which follows the
// presses unseen, is the game as it is.
let profiles = null; // {base, groups, presets} from /api/rebuild/profiles
let modelName = null; // the model's name, from the build's preview.json
let settingsMode = ""; // the page's state when the panel was last drawn
const settingsKey = () => `gp-settings:${modelName ?? session?.game ?? ""}`;
/** The kept setting: a mode's name (retro, normal, a preset) or switches
 * joined by `+`. A list kept by the page before the in-game screen is read
 * as its switches. */
function profileName() {
  try {
    const v = localStorage.getItem(settingsKey());
    if (v) return v;
    const old = JSON.parse(localStorage.getItem(`gp-mechanic-profile:${session?.game ?? ""}`) ?? "[]");
    if (Array.isArray(old) && old.length) return old.filter((n) => typeof n === "string").sort().join("+");
  } catch (e) { /* no storage: retro */ }
  return profiles?.base ?? "retro";
}
function keepProfile(name) {
  try { localStorage.setItem(settingsKey(), name); } catch (e) { /* not kept */ }
}
async function showSettings() {
  if (!profiles) {
    try { profiles = await api("api/rebuild/profiles"); } catch (e) { $("set-now").textContent = `No settings: ${e}`; return; }
  }
  if (!modelName) {
    try { modelName = (await api("rebuild/preview.json")).model ?? null; } catch (e) { /* the game's name */ }
  }
  const b = $("set-open");
  if (b && !b.dataset.wired) {
    b.dataset.wired = "1";
    b.addEventListener("click", () => { b.blur(); openSettings(); });
  }
  syncSettings();
}
function syncSettings() {
  const busy = mode !== "live";
  const b = $("set-open");
  if (b) b.disabled = busy || liveRb?.kind !== "free";
  const name = profileName();
  const playing = liveRb?.profile ?? name;
  $("set-now").textContent = `Playing: ${name.split("+").join(", ")}${playing !== name ? " (starting)" : ""}. The button below, F1 or the right stick (R3) opens the settings in the game (Esc only closes them here, since it also leaves FREE PLAY): the mode, retro or normal, and every switch. A change starts the model again from power-on.`;
}
/** Opens the settings screen inside the rebuild: FREE PLAY only, since
 * beside the original the rebuild is always the model as written. */
function openSettings() {
  if (liveRb?.kind !== "free" || !liveRb.on) { toast("The settings open in FREE PLAY."); return; }
  if (mode !== "live") { toast("Finish the recording first."); return; }
  rbPost({ gp: "settings", key: "open", profile: profileName() });
}
// The settings screen's keys while it is open in the rebuild.
const SETTINGS_KEYS = { ArrowUp: "up", KeyW: "up", ArrowDown: "down", KeyS: "down", ArrowLeft: "left", KeyA: "left", ArrowRight: "right", KeyD: "right", Enter: "choose", Space: "choose", KeyX: "choose", Escape: "back", Backspace: "back", KeyZ: "back", F1: "open" };
const SETTINGS_PAD = [[12, "up"], [13, "down"], [14, "left"], [15, "right"], [0, "choose"], [1, "back"], [11, "open"]];
const settingsPadWas = new Set();
/** The settings keys pressed on the first gamepad since the last look. */
function settingsPadKeys() {
  const g = firstPad();
  const out = [];
  if (!g) return out;
  for (const [i, key] of SETTINGS_PAD) {
    const on = !!g.buttons[i]?.pressed;
    if (on && !settingsPadWas.has(i)) out.push(key);
    if (on) settingsPadWas.add(i); else settingsPadWas.delete(i);
  }
  return out;
}
/** A settings message from the rebuild: open, or closed with the setting
 * to keep and whether it changes the switches. */
function onSettings(m) {
  if (!liveRb) return;
  liveRb.settingsOpen = !!m.open;
  if (m.open) { down.clear(); return; }
  mask = 0xff;
  if (typeof m.profile !== "string") return;
  keepProfile(m.profile);
  syncSettings();
  if (m.restart && liveRb.kind === "free") {
    toast(`Settings: ${m.profile.split("+").join(", ")}. The model starts again from power-on.`, 4000);
    void rbPlay("free");
  }
}

window.addEventListener("message", (e) => {
  if (!liveRb || e.origin !== location.origin || e.source !== $("rb-frame").contentWindow) return;
  const m = e.data;
  if (m?.gp === "ready") { liveRb.ready = true; liveRb.onReady?.(); }
  else if (m?.gp === "settings") onSettings(m);
  else if (m?.gp === "error") toast(`The model: ${m.message}`, 6000);
});

const rbOff = document.createElement("canvas");
rbOff.width = W; rbOff.height = H;
/** The original (console 0) and the rebuild's last picture in the chosen layout. */
function drawLiveRebuild() {
  if (liveRb.on && liveRb.queue.length) {
    rbPost({ gp: "push", text: "gp-frames 1\n" + liveRb.queue.map(framesLine).join("\n") + "\n" });
    liveRb.queue = [];
  }
  const c = $("rb-frame").contentDocument?.getElementById("game");
  if (!liveRb.on || !c || !c.width) {
    if (liveRb.kind === "free") {
      // Never the original in FREE PLAY: black, and why.
      frameSource = "none";
      ctx.fillStyle = "#000";
      ctx.fillRect(0, 0, W, H);
      ctx.font = "8px monospace";
      ctx.fillStyle = "rgb(252,160,68)";
      ctx.fillText("THE MODEL IS STARTING", 80, 120);
      return;
    }
    frameSource = "original";
    draw(0);
    ctx.save();
    ctx.font = "8px monospace";
    const t = "THE MODEL JOINS ONCE THE GAME IS IN PLAY";
    ctx.fillStyle = "rgba(0,0,0,0.75)";
    ctx.fillRect(2, 2, ctx.measureText(t).width + 6, 11);
    ctx.fillStyle = "rgb(252,160,68)";
    ctx.fillText(t, 5, 10);
    ctx.restore();
    return;
  }
  // The preview's canvas is the visible part of the screen (256 by 224 on
  // Donkey Kong, from y 8): copied pixel for pixel to its place, never
  // stretched, so the picture stays sharp.
  const g = rbOff.getContext("2d", { willReadFrequently: true });
  g.imageSmoothingEnabled = false;
  g.fillStyle = "#000";
  g.fillRect(0, 0, W, H);
  const [vl, vt] = liveRb.visible ?? [0, 0];
  g.drawImage(c, vl, vt);
  frameSource = view === "solo" ? "rebuild" : "original+rebuild";
  drawPair(player.rgba(0), g.getImageData(0, 0, W, H).data, (k) => {
    // The layout's name, for a few seconds after it changes; the badge
    // names the mode the rest of the time.
    if (performance.now() > (liveRb.labelUntil ?? 0)) return;
    const label = view === "solo" ? (liveRb.kind === "free" ? "FREE PLAY: THE MODEL ALONE" : "THE MODEL ONLY") : view === "side" ? "SIDE BY SIDE: ORIGINAL, MODEL" : view === "top" ? `MODEL ON TOP, ${Math.round(opacity * 100)}%` : "DIFFERENCE: BLACK WHERE THEY MATCH";
    k.font = "8px monospace";
    k.fillStyle = "rgba(0,0,0,0.7)";
    k.fillRect(2, H - 10, k.measureText(label).width + 6, 10);
    k.fillStyle = "rgb(252,160,68)";
    k.fillText(label, 4, H - 2);
  });
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
  // The rebuild starts again from the loaded moment.
  if (liveRb?.kind === "beside") void rbStart();
  setState(mode === "recording" ? "Loaded. Still recording." : "Loaded.");
  toast(mode === "recording" ? "Loaded. Still recording." : "Loaded.");
}

function stopRecording() {
  const log = player.inputLog(0);
  const frames = player.frame(0) - recStart;
  $("rec").hidden = true;
  if (frames < 10) { mode = "live"; clipFor = null; markTarget(); toast("Too short to keep: under 10 frames."); return; }
  pendingClip = { log, start: recStart, frames, stage: "checking" };
  mode = "review";
  reviewPanel();
  void checkStage(pendingClip);
}

// A run that spans a stage (the game's stage moves on during it, as a
// clear does) is a held-out run: review offers to seal it, and sealing is
// the chosen answer. Until the server has said, A waits for it.
let sealChoice = 0; // 0 seal, 1 hand off, while a stage run is offered for sealing
async function checkStage(c) {
  try {
    const { head, frames } = splitLog(c.log);
    if (warming) { await warming; warming = null; }
    const path = await sendPath(head + frames.slice(0, c.start).map((l) => l + "\n").join(""));
    const r = await api("api/clips/stage", { path, start: c.start, inputs: head + frames.slice(c.start).join("\n") + "\n" });
    c.stage = r.spans ? "spans" : "within";
    c.stages = (r.stages ?? []).map((x) => x.stage);
  } catch (e) {
    c.stage = "unknown";
  }
  if (pendingClip !== c || mode !== "review") return;
  if (c.stage === "spans") {
    sealChoice = 0;
    reviewPanel();
  } else if (c.goOnce) {
    void handOff();
  }
}

function reviewPanel() {
  const c = pendingClip;
  if (c.stage === "spans") {
    sealChoice = Math.min(1, Math.max(0, sealChoice));
    showPanel(`Clip of ${(c.frames / FPS).toFixed(1)} s. This run spans a stage (${c.stages.join(" to ")}). Seal this run?`,
      ["Seal it: no worker will see it", `Hand it off to ${clipFor}`], true,
      "Up and down to choose, A or Enter to do it. B or Esc: discard it. Right bumper or R: record it again.");
    [...$("choices").children].forEach((li, i) => { li.className = i === sealChoice ? "on" : ""; });
    showTo(sealChoice === 0 ? "sealed" : clipFor);
    return;
  }
  showPanel(`Clip of ${(c.frames / FPS).toFixed(1)} s. Hand off to ${clipFor}?`, [], true,
    "A or Enter: hand it off to that row. Select or Tab: seal this run instead (no worker sees it). B or Esc: discard it. Right bumper or R: record it again. Click another quest-log row to hand it to that one.");
  showTo(clipFor === SEALED ? "sealed" : clipFor);
}

/** The row a clip goes to, big over the panel: yellow for the open row,
 * orange for another, red for a sealed run. */
const SEALED = "sealed-run";
function showTo(row) {
  const to = $("to");
  if (!row) { to.hidden = true; return; }
  to.hidden = false;
  to.className = row === "sealed" ? "sealed" : row === session?.mechanic ? "" : "other";
  to.textContent = row === "sealed" ? "SEALED RUN: NO WORKER WILL SEE IT" : `TO: ${row}`;
}

/** The row a recording goes to, outlined in the quest log while it is
 * recorded, reviewed and sent. */
function markTarget() {
  const on = mode === "recording" || mode === "review" || mode === "sending" ? clipFor : null;
  for (const li of $("queue").querySelectorAll("li[data-mechanic]")) li.classList.toggle("target", li.dataset.mechanic === on);
}

/** An input log's header, and its frames one line each: an NES log's frame
 * lines as they are; an arcade log's runs (`gp-arcade-input 0`, a count of
 * frames and the buttons held) spelt out a frame a line, `1 C......`, which
 * reads back the same. */
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

// The play log the server already holds as a path, so a clip only sends
// what was played since: the path is the play from power-on, the clip is
// a frame on it and the buttons pressed from there.
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

async function sendClip(c, note, mechanic, phase) {
  const text = c.log;
  const { head, frames } = splitLog(text);
  const tail = sentPath && text.startsWith(sentPath.text) ? text.length - sentPath.text.length : text.length;
  if (warming) { phase("Sending your play"); await warming; warming = null; }
  phase(`Sending your play (${Math.max(1, Math.round(tail / 1024))} KB)`);
  const path = await sendPath(text);
  phase("Saving the clip");
  const inputs = head + frames.slice(c.start).join("\n") + "\n";
  return api("api/clips", { path, start: c.start, inputs, note, mechanic });
}

/** A sealed run: only the play up to its start goes to the shared paths;
 * its own presses go to the sealed store with the clip. */
async function sendSealed(c, note, phase) {
  const { head, frames } = splitLog(c.log);
  if (warming) { phase("Sealing your run"); await warming; warming = null; }
  phase("Sealing your run");
  const path = await sendPath(head + frames.slice(0, c.start).map((l) => l + "\n").join(""));
  const inputs = head + frames.slice(c.start).join("\n") + "\n";
  return api("api/clips", { path, start: c.start, inputs, note, sealed: true });
}

/** Hands the reviewed clip to the agent. The page is in the sending state
 * from the first line until the server has the clip and the turn: no
 * button does anything there, and the panel says what is happening. */
async function handOff(seal = false) {
  if (mode !== "review" || !pendingClip) return;
  mode = "sending";
  mask = 0xff;
  // The row is fixed here, at the press, not when the server answers.
  const target = seal ? SEALED : clipFor;
  const c = pendingClip;
  const note = $("note").value.trim();
  const t0 = performance.now();
  let what = seal ? "Sealing your run" : `Sending your clip to ${target}`;
  const show = () => {
    $("say").textContent = `${what}, ${((performance.now() - t0) / 1000).toFixed(1)} s`;
  };
  showPanel("", [], false, "Buttons wait until it's sent.");
  showTo(seal ? "sealed" : target);
  show();
  const timer = setInterval(show, 100);
  let n, again;
  if (seal) {
    let kept;
    try {
      ({ again, sealed: kept } = await sendSealed(c, note, (w) => { what = w; show(); }));
    } catch (e) {
      clearInterval(timer);
      mode = "review";
      showPanel(`The run wasn't sealed: ${e}`, [], true, "Select or Tab: try again. A or Enter: hand it off instead. B or Esc: discard it.");
      showTo("sealed");
      return;
    }
    clearInterval(timer);
    pendingClip = null;
    clipFor = null;
    $("note").value = "";
    mode = "sealed";
    markTarget();
    showPanel(again ? "Sealed already: no worker will see this." : "Sealed: no worker will see this.", [], false,
      `Kept as ${kept}, outside every mechanic's clips. A or Enter goes back to the game.`);
    showTo("sealed");
    setState(`Sealed as ${kept}.`);
    return;
  }
  try {
    ({ n, again } = await sendClip(c, note, target, (w) => { what = `${w} to ${target}`; show(); }));
    what = `Telling your agent (${target})`;
    show();
    await api("api/talk", { text: note || `Here is clip ${n}.`, clips: [n], mechanic: target });
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
  // The model's pick, already chosen: A confirms it, another choice disputes it.
  const j = turn.judge;
  if (j && choices.includes(j.choice)) choice = choices.indexOf(j.choice);
  const hint = choices.length
    ? "Up and down to choose, A or Enter to pick." + (turn.show ? " B replays the clip." : "")
    : "A or Enter when you're ready.";
  showPanel(turn.text, choices, false, hint, j);
  if (j) {
    const pct = Math.round((j.confidence ?? 0) * 100);
    const sure = pct >= 80 ? "fairly sure" : pct >= 50 ? "unsure" : "guessing";
    $("hint").textContent = `Laya's pick is chosen (${pct}%, ${sure}): A confirms it, another choice disputes it. ` + hint;
  }
  if (!turn.show) return;
  const still = () => agentTurn === turn && mode === "agent";
  try {
    progress("Loading the clip", 0, 1);
    const q = turn.mechanic ? `?mechanic=${encodeURIComponent(turn.mechanic)}` : "";
    const r = await fetch(`api/clips/${turn.show}.log${q}`);
    if (!r.ok) throw new Error(`clip ${turn.show}: ${r.status}`);
    const text = await r.text();
    if (!still()) return;
    const meta = session.clips.find((c) => c.n === turn.show);
    const start = meta ? meta.start : 0;
    const clip = { start, end: meta ? start + meta.frames : start + 600, log: text };
    measure = null;
    if (turn.measure) {
      const r3 = await fetch(`api/clips/${turn.show}.measure.json${q}`);
      if (r3.ok) measure = await r3.json();
    }
    if (turn.against) {
      const r2 = await fetch(`api/clips/${turn.against}.log${q}`);
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
    if (turn.rebuild) await loadRebuild(turn.show, turn.mechanic, still);
  } catch (e) {
    showClip = null;
    setBar(0, null);
    setState(`Couldn't load the clip: ${e}`);
  }
}

/** A clip from the refinement card beside the rebuild, looping from two
 * seconds before its first lasting split. */
let refineSplit = null; // {frame, what} of the clip shown in refine
let refineMsg = null; // the busy line while refine loads a clip
let refineToken = null;
async function refineShow(c) {
  if (mode !== "refine") return;
  endCompare();
  showClip = null;
  rebuild = null;
  measure = null;
  refineSplit = null;
  drawStrip();
  const mine = {};
  refineToken = mine;
  const still = () => mode === "refine" && refineToken === mine;
  const name = c.sealed ? `sealed run ${c.sealed}` : `clip ${c.clip} of ${c.mechanic}`;
  refineMsg = `Loading ${name}.`;
  // The server plays both games from the clip's start (its start cache has
  // the original there) and keeps both pictures, so nothing is replayed
  // here from power-on, however late in a long play the clip starts.
  const q = `?mechanic=${encodeURIComponent(c.mechanic ?? "")}`;
  await loadRebuildFrom(c.sealed ? `api/sealed/${c.sealed}.rebuild.json` : `api/clips/${c.clip}.rebuild.json${q}`, still);
  if (!still()) return;
  if (!rebuild) { refineMsg = $("busy-text").textContent || `Couldn't load ${name}.`; return; }
  // The review's own frames: one a compared frame of the original.
  const clip = { start: 0, end: Math.max(0, rebuild.frames.length - 1), server: true };
  clip.ctx = c.sealed ? { title: `Sealed run ${c.sealed}`, row: "sealed (private)", note: c.note, start: c.start }
    : { ...ctxFor(c.mechanic, c.clip), ...c, title: `${c.mechanic} clip ${c.clip}`, row: c.mechanic };
  showClip = clip;
  review.f = 0;
  refineSplit = c.first_split ?? null;
  // Picked from the list, a clip opens a second before its first split;
  // with no split from the scores, on its first frame that differs (as ON
  // THE MODEL opens).
  const k = refineSplit?.frame ? frameAt(refineSplit.frame - 60) : rebuild.frames.findIndex((f) => !f.together);
  if (k > 0) seekReview(k);
  review.playing = !(k > 0) || !!refineSplit;
  syncReviewBar();
  drawStrip();
}

/** The review's frame standing at the original's frame `f`, or the
 * nearest after it. */
function frameAt(f) {
  const i = rebuild?.frames.findIndex((e) => e.f >= f) ?? -1;
  return i < 0 ? 0 : i;
}

// Reviewing the rebuild: the clip plays or stands still, and steps and
// scrubs by its compared frames (the server's pictures, a lookup each). A
// clip under the agent's words plays in console 1, which only runs
// forward, so states are kept every 120 frames as it plays, and a step back
// restores the nearest one before it.
const review = { playing: true, f: 0 };
const SNAP_EVERY = 120;

function seekReview(f) {
  if (!showClip) return;
  const target = Math.max(showClip.start, Math.min(showClip.end, Math.round(f)));
  if (showClip.server) { review.f = target; syncReviewBar(); drawStrip(); return; }
  let best = showClip.start, state = showClip.startState;
  for (const [k, st] of showClip.snaps ?? []) if (k <= target && k > best) { best = k; state = st; }
  flushAudio();
  if (player.frame(1) > target || player.frame(1) < best) {
    if (state) player.restore(1, state); else rewindClip();
  }
  while (player.frame(1) < target && player.advance(1)) keepSnap();
  syncReviewBar();
}

function keepSnap() {
  const f = player.frame(1);
  if (showClip?.snaps && f % SNAP_EVERY === 0 && !showClip.snaps.has(f)) {
    try { showClip.snaps.set(f, player.save(1)); } catch (e) { /* stepping still works */ }
  }
}

/** The bar under the game in REVIEW REBUILD: play, step, the differences
 * before and after, scrub, layout, transparency and the entity boxes, the
 * same as the keys. */
function syncReviewBar() {
  const bar = $("review-bar");
  if (!bar) return;
  bar.hidden = mode !== "refine";
  if (bar.hidden) { $("rv-strip").hidden = true; return; }
  $("rv-play").textContent = review.playing ? "PAUSE" : "PLAY";
  $("rv-layout").value = view === "solo" ? "side" : view;
  $("rv-opacity").value = String(Math.round(opacity * 100));
  $("rv-opacity-row").hidden = view !== "top";
  $("rv-boxes").checked = showBoxes;
  const s = $("rv-scrub");
  if (showClip) {
    s.min = String(showClip.start);
    s.max = String(showClip.end);
    s.value = String(clipAt());
    const e = entryAt(clipAt());
    const at = clipAt() - showClip.start;
    const of = showClip.end - showClip.start + 1;
    $("rv-at").textContent = `frame ${at + 1} of ${of} (${(at / FPS).toFixed(2)} s, the original's ${e?.f ?? "?"}): ${e?.together ? "the same" : "they differ"}`;
    $("rv-at").className = e?.together ? "same" : "differ";
  } else {
    $("rv-at").textContent = "pick a clip";
    $("rv-at").className = "";
  }
}

function reviewBar() {
  $("rv-play").addEventListener("click", () => doAction("rvPlay"));
  $("rv-back").addEventListener("click", () => doAction("rvBack"));
  $("rv-fwd").addEventListener("click", () => doAction("rvFwd"));
  $("rv-prev-diff").addEventListener("click", () => doAction("rvPrevDiff"));
  $("rv-next-diff").addEventListener("click", () => doAction("rvNextDiff"));
  $("rv-layout").addEventListener("change", (e) => { view = e.target.value; keepView(); e.target.blur(); });
  $("rv-opacity").addEventListener("input", (e) => { opacity = Number(e.target.value) / 100; keepView(); });
  $("rv-boxes").addEventListener("change", (e) => { showBoxes = e.target.checked; keepView(); e.target.blur(); });
  $("rv-scrub").addEventListener("input", (e) => { review.playing = false; seekReview(Number(e.target.value)); });
  $("rv-strip").addEventListener("click", (e) => {
    if (!rebuild || !showClip?.server) return;
    const c = e.currentTarget;
    review.playing = false;
    seekReview(Math.floor((e.offsetX / c.clientWidth) * rebuild.frames.length));
  });
}

/** A clip of the open mechanic beside the rebuild, from the clip list, the
 * open row's COMPARE, or C and the left bumper (its newest clip). */
async function openClipView(n) {
  if (mode === "idle") leaveIdle();
  if (mode === "agent") { agentTurn = null; showClip = null; endCompare(); hidePanel(); mode = "live"; }
  if (mode !== "live") { toast(mode === "sending" ? "Wait for the clip to send." : "Finish the recording first."); return; }
  const c = session.clips.find((x) => x.n === n);
  toast(`Clip ${n} beside the model.`);
  // Shown as an agent turn of the page's own, which answers nothing.
  seenTurns = session.talk.filter((t) => t.by === "agent").length;
  await showAgent({ text: `Clip ${n}${c?.note ? ": " + c.note : ""}. The original, and the model on the same presses from the original's state at the clip's start.`, choices: [], show: n, rebuild: true, mine: true });
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
    if (t.judge) body.laya = t.judge.choice;
    try { await api("api/talk", body); } catch (e) { setState(String(e)); }
  }
  setState("");
}

function showPanel(text, choices, withNote, hint, judge) {
  showTo(null);
  $("say").textContent = text;
  // What was measured in the clip, so the person judges from the same facts.
  const facts = agentTurn?.facts ?? [];
  if (facts.length) $("say").textContent += "\n\n" + facts.join("\n");
  const m = judge?.measured ?? [];
  if (m.length) {
    $("say").textContent += "\n\nMeasured: " + m.map((j, i) => `jump ${i + 1} rose ${j.rose_px} px, A held ${j.a_held_frames} frames, took off at ${j.takeoff_px_a_frame} px a frame`).join("; ") + ".";
  }
  const ul = $("choices");
  ul.innerHTML = "";
  choices.forEach((c, i) => {
    const li = document.createElement("li");
    li.textContent = c;
    const p = judge?.probabilities?.[c];
    if (p !== undefined) {
      const tag = document.createElement("span");
      tag.className = "laya";
      tag.textContent = c === judge.choice ? ` Laya's pick, ${Math.round(p * 100)}%` : ` Laya ${Math.round(p * 100)}%`;
      li.appendChild(tag);
    }
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
// What the agents are doing in every topic of the game, from /api/watch:
// each topic's log (gp mechanic log GAME TOPIC) merged newest first with
// its topic, the pins and the problem rows still to do in every topic, and
// all of the open mechanic's own, which are marked. A picker shows one
// topic. Redrawn only when it changes, so a playing clip isn't restarted
// every poll.

/** An element with a class and text. */
function wEl(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

/** A log line: its time, its topic and what it says. */
function timed(line, open) {
  const li = wEl("li", line.topic === open ? "mine" : "");
  li.append(wEl("span", "at", line.at ?? ""), wEl("span", "topic", line.topic ?? ""), document.createTextNode(line.text ?? ""));
  return li;
}

let watchTopic = ""; // "" is every topic
// Blurred once picked, so the arrows go back to the game.
$("w-topic").addEventListener("change", (e) => { watchTopic = e.target.value; e.target.blur(); void refreshWatch(); });

async function refreshWatch() {
  if (!session) return;
  const w = await api(`api/watch?mechanic=${encodeURIComponent(session.mechanic)}`);
  const open = w.open ?? session.mechanic;
  const key = JSON.stringify([session.mechanic, watchTopic, w]);
  const col = $("watch-col");
  if (col.dataset.key === key) return;
  col.dataset.key = key;
  const topics = w.topics ?? [];
  if (watchTopic && !topics.some((t) => t.topic === watchTopic)) watchTopic = "";
  const pick = $("w-topic");
  pick.innerHTML = "";
  pick.append(new Option(`All topics (${topics.length})`, ""));
  for (const t of topics) {
    const todo = t.problems ? `, ${t.problems} to do` : "";
    pick.append(new Option(`${t.topic}${t.topic === open ? " (open here)" : ""}: ${t.lines} lines${todo}`, t.topic));
  }
  pick.value = watchTopic;
  const shown = (x) => !watchTopic || x.topic === watchTopic;
  const lines = (w.log ?? []).filter(shown);
  const status = $("w-status");
  status.innerHTML = "";
  if (lines.length) {
    status.className = lines[0].topic === open ? "mine" : "";
    status.append(wEl("span", "at", lines[0].at ?? ""), wEl("span", "topic", lines[0].topic ?? ""), document.createTextNode(lines[0].text ?? ""));
  } else {
    status.className = "muted";
    const where = watchTopic ? `in ${watchTopic}` : `in any topic of ${session.game}`;
    status.textContent = `Nothing yet ${where}. What your agent is doing shows here, every topic of the game.`;
  }
  const pins = $("w-pins");
  pins.innerHTML = "";
  for (const p of (w.pins ?? []).filter(shown)) {
    const fig = wEl("figure", [p.keep ? "keep" : "", p.topic === open ? "mine" : ""].join(" ").trim());
    const src = `api/watch/files/${encodeURIComponent(p.file)}?topic=${encodeURIComponent(p.topic)}`;
    let media;
    if (p.kind === "video") {
      media = wEl("video");
      Object.assign(media, { src, muted: true, loop: true, autoplay: true, controls: true, playsInline: true });
    } else {
      media = wEl("img");
      Object.assign(media, { src, alt: p.title || p.file });
    }
    const cap = wEl("figcaption");
    cap.append(wEl("span", "topic", p.topic), document.createTextNode(`${p.n}. ${p.title || p.file}`));
    fig.append(media, cap);
    pins.append(fig);
  }
  // Every topic's rows still to do and all of the open one's; a topic
  // picked shows all of its own.
  const rows = (w.problems ?? []).filter((r) => (watchTopic ? r.topic === watchTopic : r.todo || r.topic === open));
  const table = $("w-problems");
  const body = table.querySelector("tbody");
  body.innerHTML = "";
  for (const r of rows) {
    const tr = wEl("tr", r.topic === open ? "mine" : "");
    for (const [v, cls] of [[r.topic, "topic"], [r.n], [r.where], [r.what], [r.fix], [r.state, `st-${r.state}`]]) tr.append(wEl("td", cls ?? "", String(v ?? "")));
    body.append(tr);
  }
  table.hidden = !rows.length;
  const log = $("w-log");
  log.innerHTML = "";
  for (const line of lines) log.append(timed(line, open));
}

async function refresh() {
  session = await api("api/session");
  $("title").textContent = `${session.game} / ${session.mechanic}`;
  const all = session.queue ?? [];
  $("score").textContent = all.length ? `${all.filter((q) => q.status === "done").length} / ${all.length} MEASURED` : "";
  // Rebuilt only when it changes: a list redrawn under the mouse every poll
  // loses the click.
  const queue = $("queue");
  const qkey = JSON.stringify([session.mechanic, all, [...toolsOn], handable()]);
  if (queue.dataset.key !== qkey) {
    queue.dataset.key = qkey;
    queue.innerHTML = "";
    // gp mechanic triage's rows: what needs the person first, with its
    // request; what the tools made, collapsed at the end with its clips.
    const ordered = [...all.filter((q) => q.triage?.by === "you"), ...all.filter((q) => !q.triage?.by), ...all.filter((q) => q.triage?.by === "tools")];
    const toolsList = toolsGroup(all.filter((q) => q.triage?.by === "tools").length);
    for (const q of ordered) {
      const li = document.createElement("li");
      const st = document.createElement("span");
      const status = shownStatus(q);
      st.className = `st st-${status.replace(/ /g, "-")}`;
      st.textContent = triageLabel(q) ?? LABELS[status] ?? status.toUpperCase();
      if (q.triage?.by) st.className = `st st-${q.triage.by === "you" ? "needs-you" : "by-tools"}`;
      li.appendChild(st);
      const name = document.createElement("span");
      name.className = "qname";
      name.textContent = q.mechanic;
      li.appendChild(name);
      // The gate's scores first (status.json), then the typed note.
      for (const [k, cls] of [["scores", "qnote qscore"], ["note", "qnote"]]) {
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
        cmp.textContent = "COMPARE";
        cmp.title = "Its newest clip beside the model (C, or the left bumper)";
        cmp.addEventListener("click", (e) => { e.stopPropagation(); doAction("compareNewest"); });
        li.appendChild(cmp);
        // Opening a row hands nothing over; this does.
        if (handable()) {
          const give = document.createElement("button");
          give.className = "qcmp qgive";
          give.textContent = "HAND TO THE AGENT";
          give.title = "Ask your agent to work on this mechanic now";
          give.addEventListener("click", (e) => { e.stopPropagation(); void handTo(q.mechanic); });
          li.appendChild(give);
        }
      }
      triageLines(li, q);
      li.dataset.mechanic = q.mechanic;
      li.addEventListener("click", () => (mode === "refine" ? void refine.enter(q.mechanic) : void pick(q.mechanic)));
      (q.triage?.by === "tools" ? toolsList : queue).appendChild(li);
    }
    if (toolsList) queue.appendChild(toolsList.closest("li"));
    markTarget();
    showFindings(all.find((q) => q.mechanic === session.mechanic));
    if (mode === "refine") $("findings").hidden = true;
  }
  const clips = $("clips");
  const ckey = JSON.stringify([session.mechanic, session.clips.map((c) => [c.n, c.frames, c.note])]);
  if (clips.dataset.key !== ckey) {
    clips.dataset.key = ckey;
    clips.innerHTML = "";
    for (const c of session.clips) {
      const li = document.createElement("li");
      li.value = c.n;
      li.textContent = `${(c.frames / FPS).toFixed(1)} s, by ${c.by}${c.note ? ": " + c.note : ""}`;
      li.title = "Click to watch it beside the model";
      li.addEventListener("click", () => void openClipView(c.n));
      // The clip on the model beside the original, in REVIEW REBUILD.
      const rv = document.createElement("button");
      rv.type = "button";
      rv.className = "rv2-open";
      rv.textContent = "ON THE MODEL";
      rv.title = "Review this clip on the model beside the original, as the score plays it (ON THE MODEL)";
      rv.addEventListener("click", (e) => {
        e.stopPropagation();
        void reviewOnModel(c.n);
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
  // Over plain play only; while the rebuild plays live the turn waits,
  // and the line under the game says so.
  if (agentTurns.length > seenTurns && mode === "live" && liveRb) {
    $("busy-text").textContent = "Your agent has a message. Esc goes back to playing the original, where it shows.";
  } else if (agentTurns.length > seenTurns && mode === "live") {
    seenTurns = agentTurns.length;
    await showAgent(agentTurns[agentTurns.length - 1]);
  }
  if (mode === "live") setState("");
  updateBusy();
}

// gp mechanic triage's marks ------------------------------------------

/** A triaged row's badge: the person's to play, or made by the tools. */
function triageLabel(q) {
  if (q.triage?.by === "you") return "NEEDS YOU";
  if (q.triage?.by === "tools") return "BY THE TOOLS";
  return null;
}

/** The collapsed group the tools' rows go in: an <ol> inside a <details>
 * row at the end of the quest log; none when the tools made nothing. */
function toolsGroup(n) {
  if (!n) return null;
  const li = document.createElement("li");
  li.className = "qgroup";
  const d = document.createElement("details");
  const sum = document.createElement("summary");
  sum.textContent = `Done by the tools (${n})`;
  sum.title = "gp mechanic triage: the tools made a clip of each of these on their own, in its session, scored like any clip";
  d.appendChild(sum);
  const ol = document.createElement("ol");
  ol.className = "qtools";
  d.appendChild(ol);
  li.appendChild(d);
  return ol;
}

/** A triaged row's lines: for the person, the request (where it starts,
 * what to do, when to stop) and PLAY FROM HERE; for the tools, which tool
 * and its clips. Why the tools couldn't is the badge's tooltip. */
function triageLines(li, q) {
  const t = q.triage;
  if (!t?.by) return;
  const line = (text, cls) => {
    const s = document.createElement("span");
    s.className = `qnote ${cls}`;
    s.textContent = text;
    li.appendChild(s);
    return s;
  };
  if (t.by === "tools") {
    const clips = (t.clips ?? []).map((n) => `clip ${n}`).join(", ");
    line(`${t.why}${clips ? ` (${clips})` : ""}`, "qtool");
    return;
  }
  const r = t.request ?? {};
  line(`Starts: ${r.where ?? r.from ?? "power-on"}`, "qreq");
  if (r.do) line(`Do: ${r.do}`, "qreq");
  if (r.stop) line(`Stop: ${r.stop}`, "qreq");
  line(`The tools: ${t.why}`, "qwhy");
  const b = document.createElement("button");
  b.className = "qplay";
  b.textContent = "PLAY FROM HERE";
  b.title = "Opens this row at the request's start, and starts recording: play only the hard stretch, then R";
  b.addEventListener("click", (e) => { e.stopPropagation(); void playFrom(q.mechanic); });
  li.appendChild(b);
}

// What the queue's statuses show as.
const LABELS = { done: "MEASURED", "asking you": "YOUR MOVE", "agent working": "WORKING ON IT", "in line": "IN LINE", next: "NEXT" };

// The mechanics the background work (gp serve's, streamed to the page's
// hub in refine/fold.js) has a job or a clip agent on now, from its clip
// IDs (GAME.MECHANIC.NNN).
let toolsOn = new Set();
function mechanicOfClip(id) {
  const parts = String(id ?? "").split(".");
  return parts.length >= 3 ? parts.slice(1, -1).join(".") : null;
}
function readToolsOn(s) {
  const on = new Set();
  for (const j of s?.jobs ?? []) on.add(mechanicOfClip(j.clip));
  for (const g of s?.clip_agents ?? []) for (const c of [g.clip, ...(g.clips ?? [])]) on.add(mechanicOfClip(c));
  on.delete(null);
  return on;
}

/** A row's status as shown: WORKING ON IT while the background work is on
 * its clips, otherwise the server's (the agent's turn, the gate's scores). */
function shownStatus(q) {
  return q.status !== "done" && toolsOn.has(q.mechanic) ? "agent working" : q.status;
}

/** Whether the open row can be handed to the agent: no turn of the
 * person's is waiting for it, and it isn't measured. */
function handable() {
  const row = (session?.queue ?? []).find((q) => q.mechanic === session.mechanic);
  const last = session?.talk?.[session.talk.length - 1];
  return !!row && row.status !== "done" && last?.by !== "player";
}

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

// The two big modes ----------------------------------------------------

let refine = null;

/** HELP WITH MECHANICS is the page as it always was; REVIEW REBUILD
 * pauses the game and loops the original with the rule drawn over it. */
function setTab(t) {
  // A clip being recorded, reviewed or sent keeps its tab until it's done.
  if (mode === "recording" || mode === "review" || mode === "sealed") { toast("Finish the recording first."); return; }
  if (mode === "sending") { toast("Wait for the clip to send."); return; }
  if (t === "refine") {
    if (mode === "refine") return;
    if (mode === "recording" || mode === "review") { toast("Finish the recording first."); return; }
    if (mode === "sending") { toast("Wait for the clip to send."); return; }
    if (mode === "idle") leaveIdle();
    if (mode === "agent") { agentTurn = null; showClip = null; endCompare(); hidePanel(); seenTurns = Math.max(0, seenTurns - 1); }
    if (liveRb) rbStop(true);
    showClip = null;
    refineMsg = null;
    mode = "refine";
    $("findings").hidden = true;
    $("rule-card").hidden = false;
    // The tab opens on a clip beside the rebuild, never on the bare game.
    void refine.enter(session?.mechanic ?? null).then(() => {
      if (mode === "refine" && !showClip && !refineToken && !refine.showFirst()) refineMsg = "No clips yet: record one in CLIPS.";
    });
    syncReviewBar();
  } else if (t === "free") {
    void rbPlay("free");
    return;
  } else {
    if (liveRb) rbStop(true);
    if (mode === "refine") leaveRefine();
  }
  syncTabs();
}

/** A clip's ON THE MODEL button: REVIEW REBUILD, on that clip. */
async function reviewOnModel(n) {
  if (mode !== "refine") setTab("refine");
  if (mode !== "refine") return;
  const c = session.clips.find((x) => x.n === n);
  refine.picked = `${session.mechanic}/${n}`;
  refine.card();
  await refineShow({ mechanic: session.mechanic, clip: n, start: c?.start ?? 0, frames: c?.frames ?? 0 });
}

/** REVIEW REBUILD's clip review closed, back to the live game. */
function leaveRefine() {
  if (mode !== "refine") return;
  refine.leave();
  endCompare();
  showClip = null;
  refineSplit = null;
  refineMsg = null;
  refineToken = null;
  setBar(0, null);
  mode = "live";
  syncReviewBar();
  mask = 0xff;
  lastInput = performance.now();
  $("rule-card").hidden = true;
  void refresh();
}

/** HAND TO THE AGENT on the open row: its "Let's do" turn. */
async function handTo(m) {
  try {
    const r = await api("api/handoff", { mechanic: m });
    toast(r.again ? `${m} is with your agent already.` : `${m} handed to your agent.`);
    await refresh();
  } catch (e) { toast(`Couldn't hand ${m} over: ${e.message ?? e}`); }
}

/** Opens another mechanic from the queue in the page; nothing goes to the
 * agent (HAND TO THE AGENT does that). The game keeps running where it is. */
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
    await api("api/pick", { mechanic: m });
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

/** A request's PLAY FROM HERE (gp mechanic triage): opens the row, plays
 * the request's path to its frame unseen, makes that the quick save (F8
 * goes back to it) and starts recording into the row, so the person plays
 * only the hard stretch. */
let starting = null;
async function playFrom(m) {
  if (starting) { toast("Still loading the start point."); return; }
  const row = (session?.queue ?? []).find((q) => q.mechanic === m);
  const r = row?.triage?.request;
  if (!r) return;
  if (m !== session.mechanic) await pick(m);
  if (session.mechanic !== m) return;
  if (mode === "idle") leaveIdle();
  if (mode !== "live") { toast("Finish what's on the game first."); return; }
  starting = m;
  mode = "loading";
  try {
    if (r.path) {
      const res = await fetch(`api/paths/${r.path}`);
      if (!res.ok) throw new Error(await res.text());
      player.replay(0, rom, await res.text());
      if (!(await runToSliced(0, r.frame ?? 0, "Loading the start point", () => mode === "loading"))) return;
    } else {
      player.power(0, rom);
    }
    setBar(0, null);
    flushAudio();
    save = player.save(0);
    mode = "live";
    startRecording();
    clipFor = m;
    markTarget();
    toast(`Recording from the start point. ${r.stop ? "Stop: " + r.stop : "R stops."}`, 6000);
  } catch (e) {
    toast(`Couldn't load the start point: ${e.message ?? e}`);
  } finally {
    if (mode === "loading") mode = "live";
    $("bar").hidden = true;
    starting = null;
  }
}

// Input ----------------------------------------------------------------

// <actions> What each input does in each of the page's states; an input
// missing from a state does nothing there (docs/contributing-mechanics.md
// has this as a table). The game's own buttons reach the game only while
// live or recording. Each action checks the state it was meant for before
// it acts, so a second press (A after Enter, both read in one poll) finds
// the state already moved on and does nothing.
// While the rebuild plays live, live and recording have their own rows:
// "live:beside" (REVIEW REBUILD's PLAY THE REBUILD, the original beside it)
// and "live:free" (FREE PLAY, the rebuild alone); see modeKey().
const ACTIONS = {
  live: { save: "save", load: "load", rec: "startRec", cmp: "compareNewest" },
  recording: { save: "save", load: "load", rec: "stopRec" },
  "live:beside": { save: "save", load: "load", rec: "startRec", cmp: "rbLayout", lay: "rbLayout", less: "rbLess", more: "rbMore", esc: "rbStop" },
  "recording:beside": { save: "save", load: "load", rec: "stopRec", cmp: "rbLayout", lay: "rbLayout", less: "rbLess", more: "rbMore" },
  // FREE PLAY's save and load would move only the unseen original, so they
  // do nothing there.
  "live:free": { rec: "startRec", esc: "rbStop" },
  "recording:free": { rec: "stopRec" },
  review: { a: "handOff", start: "handOff", select: "seal", up: "reviewUp", down: "reviewDown", b: "discard", esc: "discard", rec: "recordAgain" },
  sending: {},
  sealed: { a: "back", start: "back", b: "back", esc: "back" },
  agent: { a: "answer", start: "answer", up: "choiceUp", down: "choiceDown", b: "replayClip", left: "viewLeft", right: "viewRight", cmp: "toggleRebuild" },
  idle: {},
  watch: {},
  refine: { a: "rvPlay", start: "rvPlay", left: "rvBack", right: "rvFwd", up: "rvMore", down: "rvLess", select: "rvLayout", b: "rvSplit", cmp: "rvBoxes", save: "rvBack60", load: "rvFwd60", less: "rvPrevDiff", more: "rvNextDiff", rec: "rbPlay" },
};
// The state an action leaves the page in, at once (before any reply).
const NEXT = { handOff: "sending", seal: "sending", back: "live", discard: "live", recordAgain: "recording", answer: "live", startRec: "recording", stopRec: "review" };
const KEY_INPUT = { Enter: "start", KeyX: "a", Space: "a", KeyZ: "b", ShiftLeft: "b", ArrowUp: "up", ArrowDown: "down", ArrowLeft: "left", ArrowRight: "right", Escape: "esc", Tab: "select", ShiftRight: "select", KeyR: "rec", F5: "save", F8: "load", KeyC: "cmp", KeyV: "lay", BracketLeft: "less", BracketRight: "more" };
const TYPING_KEYS = ["start", "esc", "select", "up", "down"];
/** The row of ACTIONS for a mode, with the live rebuild's kind. */
function modeKey(mode, kind) {
  return kind && (mode === "live" || mode === "recording") ? `${mode}:${kind}` : mode;
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
    case "handOff":
      if (mode !== "review" || !pendingClip) break;
      // Not yet known whether it spans a stage: A goes on once it is, or
      // the question to seal comes up instead.
      if (pendingClip.stage === "checking") { pendingClip.goOnce = true; toast("Checking whether this run spans a stage."); break; }
      void handOff(pendingClip.stage === "spans" && sealChoice === 0);
      break;
    case "reviewUp": case "reviewDown":
      if (mode === "review" && pendingClip?.stage === "spans") { sealChoice = 1 - sealChoice; reviewPanel(); }
      break;
    case "seal": void handOff(true); break;
    case "back": if (mode === "sealed") { hidePanel(); mode = "live"; mask = 0xff; } break;
    case "discard": if (mode === "review") { pendingClip = null; clipFor = null; hidePanel(); mode = "live"; mask = 0xff; markTarget(); toast("Clip dropped."); } break;
    case "recordAgain": if (mode === "review") { const keep = clipFor; pendingClip = null; hidePanel(); startRecording(); clipFor = keep; markTarget(); } break;
    case "answer": void answerAgent(); break;
    case "choiceUp": moveChoice(-1); break;
    case "choiceDown": moveChoice(1); break;
    case "replayClip": if (showClip && !showClip.server) { rewindClip(); if (compare) rewindCompare(); } break;
    case "rvPlay": review.playing = !review.playing; syncReviewBar(); break;
    case "rvBack": review.playing = false; seekReview(clipAt() - 1); break;
    case "rvFwd": review.playing = false; seekReview(clipAt() + 1); break;
    case "rvBack60": review.playing = false; seekReview(clipAt() - Math.round(FPS)); break;
    case "rvFwd60": review.playing = false; seekReview(clipAt() + Math.round(FPS)); break;
    case "rvPrevDiff": nextDiffer(-1); break;
    case "rvNextDiff": nextDiffer(1); break;
    case "rvMore": setOpacity(opacity + 0.1); break;
    case "rvLess": setOpacity(opacity - 0.1); break;
    case "rvLayout": cycleView(1); break;
    case "rvBoxes": showBoxes = !showBoxes; keepView(); toast(showBoxes ? "The model's entities outlined." : "No outlines."); break;
    case "rvSplit": review.playing = false; seekReview(refineSplit?.frame && showClip?.server ? frameAt(refineSplit.frame - 60) : showClip?.start ?? 0); break;
    case "viewLeft": if (compare || rebuild) cycleView(-1); break;
    case "viewRight": if (compare || rebuild) cycleView(1); break;
    case "rbPlay": void rbPlay("beside"); break;
    case "rbLayout": if (liveRb?.kind === "beside") cycleView(1); break;
    case "rbLess": if (liveRb) setOpacity(opacity - 0.1); break;
    case "rbMore": if (liveRb) setOpacity(opacity + 0.1); break;
    case "rbStop": if (liveRb && mode === "live") rbStop(); break;
    case "compareNewest": {
      const last = session?.clips?.[session.clips.length - 1];
      if (last) void openClipView(last.n); else toast(`No clips of ${session?.mechanic} yet: R records one.`);
      break;
    }
    case "toggleRebuild":
      if (!agentTurn?.show) { toast("No clip shown to compare."); break; }
      if (rebuild) { rebuild = null; $("screen").classList.remove("side"); $("game2").hidden = true; toast("The original alone."); }
      else { const t = agentTurn; void loadRebuild(t.show, t.mechanic, () => agentTurn === t && mode === "agent"); }
      break;
  }
}

window.addEventListener("keydown", (e) => {
  if (paneOpen) return;
  const typing = document.activeElement === $("note");
  // U turns the sound off and on in every state, the rebuild's settings
  // screen and the note being typed aside.
  if (e.code === SOUND_KEY_CODE && !typing && !e.ctrlKey && !e.metaKey && !e.altKey && !(liveRb?.kind === "free" && liveRb.settingsOpen)) {
    e.preventDefault();
    if (!e.repeat) setSound(!soundOn);
    return;
  }
  const input = KEY_INPUT[e.code];
  // FREE PLAY's settings screen, in the rebuild: F1 opens it, and while it
  // is open its keys go to it and nothing else.
  if (liveRb?.kind === "free" && liveRb.on && mode === "live" && !typing) {
    if (liveRb.settingsOpen) {
      e.preventDefault();
      if (!e.repeat && SETTINGS_KEYS[e.code]) rbPost({ gp: "settings", key: SETTINGS_KEYS[e.code] });
      return;
    }
    if (e.code === "F1") { e.preventDefault(); if (!e.repeat) openSettings(); return; }
  }
  if (mode === "live" || mode === "recording") {
    const a = input && actionFor(modeKey(mode, liveRb?.kind), input, false);
    if (a) { e.preventDefault(); if (!e.repeat) doAction(a); return; }
    if (KEYS.flat().includes(e.code) || KEYS2.flat().includes(e.code)) e.preventDefault();
    down.add(e.code);
    return;
  }
  // Every other state: the game is paused and only the table's inputs act.
  if (!input || (typing && !TYPING_KEYS.includes(input))) return;
  e.preventDefault();
  const a = actionFor(modeKey(mode, liveRb?.kind), input, typing);
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
  const on = (name) => !!(fresh & LAYOUT.bit[name]);
  return { a: on("a"), b: on("b"), select: on("select"), start: on("start"), up: on("up"), down: on("down"), left: on("left"), right: on("right") };
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
  if (liveRb?.kind === "free" && liveRb.on && mode === "live") {
    const keys = settingsPadKeys();
    if (liveRb.settingsOpen) {
      for (const key of keys) rbPost({ gp: "settings", key });
      padPresses();
      return;
    }
    if (keys.includes("open")) openSettings();
  }
  const p = padPresses();
  const g = gamePresses();
  const pressed = { a: p.a || g.a, b: p.b || g.b, start: p.start || g.start, select: p.select || g.select, up: p.up || g.up, down: p.down || g.down, left: g.left, right: g.right, rec: p.rb, save: p.lt, load: p.rt, cmp: p.lb };
  // In live play the game's own buttons go to the game, not the table.
  const game = mode === "live" || mode === "recording";
  for (const [input, on] of Object.entries(pressed)) {
    if (!on || (game && !["rec", "save", "load", "cmp", "select"].includes(input))) continue;
    // Looked up again for each press: an action that moves the state
    // leaves the next press of the same poll nothing to do.
    const a = actionFor(modeKey(mode, liveRb?.kind), input, false);
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
 * the original while the rebuild is played live, which has its own sound. */
function heardSlot() {
  if (!soundOn || liveRb) return null;
  if (mode === "live" || mode === "recording") return 0;
  if (mode === "watch") return watch && !watch.old && watch.ready ? 1 : null;
  if (mode === "agent" && showClip && !showClip.server) return 1;
  if (mode === "refine" && showClip && !showClip.server && review.playing) return 1;
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
  const ch = player.audioChannels();
  const n = f.length / ch;
  const b = audio.createBuffer(ch, n, rate);
  if (ch === 1) {
    b.copyToChannel(f, 0);
  } else {
    for (let c = 0; c < ch; c++) {
      const one = new Float32Array(n);
      for (let i = 0; i < n; i++) one[i] = f[i * ch + c];
      b.copyToChannel(one, c);
    }
  }
  const src = audio.createBufferSource();
  src.buffer = b;
  src.connect(audioOut);
  src.start(audioAt);
  audioAt += n / rate;
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
  acc = Math.min(acc + (now - last), (1000 / FPS) * 4);
  last = now;
  if (paneOpen) { acc = 0; requestAnimationFrame(frame); return; }
  try {
    gamepadCommands();
    const heard = heardSlot();
    if (heard !== audioSlot) { flushAudio(); audioSlot = heard; }
    while (acc >= 1000 / FPS) {
      acc -= 1000 / FPS;
      if (liveRb?.settingsOpen) {
        // The rebuild's settings screen is open: both games hold.
      } else if (mode === "live" || mode === "recording") {
        const pad = padNow();
        // The rebuild takes the frames the original read the pad on.
        const polled = player.step(0, pad);
        playFrame(0);
        // Beside the original the rebuild takes the frames the original
        // read the pad on, so the two stay in step. In FREE PLAY it is its
        // own game and takes every frame: it doesn't wait while the unseen
        // original plays a death or a title screen.
        if (liveRb && liveRb.queue && (polled || liveRb.kind === "free")) liveRb.queue.push(pad);
      } else if (mode === "watch") {
        watchFrame();
      } else if (mode === "refine" && showClip) {
        if (review.playing && showClip.server) {
          review.f = review.f >= showClip.end ? showClip.start : review.f + 1;
        } else if (review.playing) {
          if (player.frame(1) >= showClip.end || !player.advance(1)) rewindClip();
          else playFrame(1);
          keepSnap();
        }
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
    if (liveRb && ["live", "recording", "review", "sending", "sealed"].includes(mode)) drawLiveRebuild();
    else if ((mode === "agent" || mode === "refine") && showClip && rebuild) { frameSource = "clip+rebuild"; drawRebuild(); }
    else if (mode === "agent" && showClip && compare) drawCompare();
    else {
      const slot = ((mode === "agent" || mode === "idle" || mode === "refine") && showClip) || mode === "watch" ? 1 : 0;
      frameSource = slot ? "clip" : "original";
      draw(slot);
    }
    if (mode === "refine" && !showClip) {
      ctx.fillStyle = "rgba(0,0,0,0.75)";
      ctx.fillRect(0, H / 2 - 16, W, 24);
      ctx.fillStyle = "#fff";
      ctx.font = "8px monospace";
      ctx.fillText(refineMsg ?? "Pick a clip in the card to watch it beside the model.", 8, H / 2 - 1);
    }
    if ((mode === "agent" || mode === "idle") && showClip && measure) drawMeasure(player.frame(1) - showClip.start);
    if (mode === "refine" && showClip && review.playing) { syncReviewBar(); drawStrip(); }
    // Which game the picture is of, and the frame the live game is on, for
    // the page's check (scripts/web/check-mechanic.mjs): never "original"
    // in FREE PLAY.
    if (document.body.dataset.frameSource !== frameSource) document.body.dataset.frameSource = frameSource;
    document.body.dataset.frame = String(player.frame(0));
    renderCtx();
    idleCheck();
    showMode();
    // The settings can't open while a clip is recorded or reviewed.
    if (profiles && !$("settings").hidden && settingsMode !== mode) { settingsMode = mode; syncSettings(); }
    if (mode === "recording") $("rec-time").textContent = `${((player.frame(0) - recStart) / FPS).toFixed(1)} s`;
  } catch (e) {
    setState(String(e));
  }
  requestAnimationFrame(frame);
}

// Idle: the agent's work on screen --------------------------------------

// After 30 s with no button, the game pauses and the screen loops the clip
// the agent is working on, with its measurements so far. Any button returns.
const IDLE_MS = 30000;
let lastInput = performance.now();
let idleFetching = false;
function idleCheck() {
  if (rawPad() !== 0 || rawPad2() !== 0) {
    lastInput = performance.now();
    if (mode === "idle") leaveIdle();
    return;
  }
  // Only in plain play: never over the rebuild played live.
  if (mode !== "live" || liveRb || idleFetching || performance.now() - lastInput < IDLE_MS) return;
  void enterViewer(null);
}

/** Watching the agent: the newest clip of a mechanic (by default the one
 * it is working on) loops with what it has measured so far. Any button
 * goes back to playing. */
async function enterViewer(m) {
  if (idleFetching) return;
  idleFetching = true;
  try {
    const w = await api("api/working" + (m ? `?mechanic=${encodeURIComponent(m)}` : ""));
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
    measure = w.measure && w.measure.frames ? w.measure : null;
    $("busy-text").textContent = `Watching your agent on ${w.mechanic}${w.busy ? ": " + w.busy : ""}${measure ? ". The marks are what it has measured so far" : ""}. Any button goes back to playing.`;
  } catch (e) {
    lastInput = performance.now();
  } finally {
    idleFetching = false;
  }
}

function leaveIdle() {
  mode = "live";
  showClip = null;
  measure = null;
  mask = 0xff;
  setBar(0, null);
  lastInput = performance.now();
  showMode();
}

/** The badge over the game and the screen's border: watching the agent,
 * playing, recording, or the agent talking. */
function showMode() {
  const [cls, text] = mode === "refine" ? ["m-watch", !showClip ? "ON THE MODEL: PICK A CLIP" : refine.picked?.startsWith("sealed/") ? "REVIEWING A SEALED RUN" : "REVIEWING THE MODEL"]
    : mode === "idle" ? ["m-watch", "WATCHING YOUR AGENT"]
    : mode === "watch" ? ["m-watch", "WATCHING YOUR AGENT PLAY"]
    : mode === "recording" ? ["m-rec", liveRb?.kind === "free" ? "RECORDING IN FREE PLAY" : liveRb ? "RECORDING BESIDE THE MODEL" : "RECORDING"]
    : mode === "sending" ? ["m-talk", "SENDING"]
    : mode === "sealed" ? ["m-rec", "SEALED"]
    : mode === "review" ? ["m-talk", "YOUR CLIP"]
    : mode === "agent" ? ["m-talk", agentTurn?.mine ? "A CLIP BESIDE THE MODEL" : "YOUR AGENT SAYS"]
    : liveRb?.kind === "free" ? ["m-play", liveRb.on ? "FREE PLAY: THE MODEL" : "FREE PLAY: STARTING"]
    : liveRb ? ["m-play", liveRb.on ? "PLAYING THE MODEL" : "THE MODEL: STARTING"]
    : ["m-play", "PLAYING"];
  const b = $("modebadge");
  if (b.textContent !== text) b.textContent = text;
  $("screen").dataset.mode = cls;
}
window.addEventListener("keydown", () => { lastInput = performance.now(); if (mode === "idle") leaveIdle(); }, { capture: true });

// Watching the model play ------------------------------------------------

async function pollLive() {
  const { lines } = await api("api/live");
  // No demo yet: the first one to come is new, and plays.
  if (!lines.length) {
    watch ??= { key: null, seen: 0, queue: [], end: null, old: true };
    return;
  }
  const start = lines[0];
  const key = `${start.start}:${start.hint}:${start.log.length}`;
  // A demo that was already there when the page opened isn't replayed.
  if (watch === null) {
    watch = { key, seen: lines.length, queue: [], end: null, old: true };
    return;
  }
  if (watch.key !== key) {
    // A new demo waits while the person records, reviews or answers.
    if (mode !== "live" || liveRb) return;
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
  player = await NesPlayer.load("gp_play.wasm");
  const r = await fetch("rom");
  if (!r.ok) throw new Error(`the ROM: ${await r.text()}`);
  rom = new Uint8Array(await r.arrayBuffer());
  player.power(0, rom);
  useMachine(player.info(0));
  showSound();
  $("sound").addEventListener("click", (e) => { setSound(!soundOn); e.currentTarget.blur(); });
  refine = new RefineView({ $, api, fps: () => FPS, show: (c) => void refineShow(c), sessionClips: () => (session?.clips ?? []).map((c) => ({ mechanic: session.mechanic, clip: c.n, start: c.start, frames: c.frames })) });
  reviewBar();
  $("tab-mech").addEventListener("click", () => setTab("mech"));
  $("rv-live").addEventListener("click", () => void rbPlay("beside"));
  $("tab-refine").addEventListener("click", () => setTab("refine"));
  // Inside the editor (gp serve's /mechanic/GAME/?embedded=1): the
  // editor's header and tabs are the page's, so its own logo, link and
  // tabs hide, and the editor's tabs choose this page's tab by message.
  const asked = new URLSearchParams(location.search);
  if (asked.has("embedded")) document.body.classList.add("embedded");
  window.addEventListener("message", (e) => {
    if (e.origin === location.origin && e.source === window.parent && e.data?.gp === "tab") setTab(e.data.tab);
  });
  setInterval(() => { if (mode === "refine") void refine.refresh(); }, 2000);
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
  // The background work's jobs, from the hub the strip under the game
  // shares (no poll of its own while the strip has one).
  import("./refine/fold.js").then((f) => {
    f.hub(f.opts({ api, run: session.game })).subscribe((x, why) => {
      if (why === "state") toolsOn = readToolsOn(x.state);
    });
  }).catch(() => undefined);
  const shell = startShell({
    setTab, api, toast,
    mode: () => mode,
    liveKind: () => liveRb?.kind ?? null,
    pause: (on) => { paneOpen = on; if (on) flushAudio(); },
    game: () => session?.game ?? null,
  });
  // A clip named in REFINE (mechanic/refine/'s "gp-open-clip", a corpus
  // clip's ID such as dk.walk.003): CLIPS, on the model, at that clip of
  // its mechanic's session.
  window.addEventListener("gp-open-clip", async (e) => {
    const id = String(e.detail?.clip ?? "");
    const st = await api("api/rebuild/scores").catch(() => null);
    const c = (st?.status?.clips ?? []).find((x) => x.id === id);
    const parts = id.split(".");
    const n = c?.clip ?? Number(parts.pop());
    const mechanic = c?.mechanic ?? parts.slice(1).join(".");
    if (!mechanic || !Number.isFinite(n)) { toast(`No session clip for ${id}.`); return; }
    shell.show("clips");
    if (mode !== "refine") setTab("refine");
    if (mode !== "refine") return;
    refine.picked = `${mechanic}/${n}`;
    refine.card();
    await refineShow({ mechanic, clip: n, start: c?.start ?? 0, frames: c?.frames ?? 0 });
  });
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
