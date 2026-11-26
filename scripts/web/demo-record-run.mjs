// Records a run of a game on the play page (`gp theme look play
// --background`) as one video with sound: the game in its own pictures on
// the left and in a theme on the right, both stepped frame by frame from
// the same presses, so the two halves can't drift apart. For a game whose
// corpus clips the screen page can't show in a theme, or for a whole play
// from power-on in one take (docs/demo-video.md, "Recording a run").
//
//   node scripts/web/demo-record-run.mjs PAGE_URL MOMENTS.toml RUN FROM TO THEME OUT.webm
//
// PAGE_URL  the link `gp theme look play --background` printed, with the
//           person's ROM on this computer
// MOMENTS   a TOML file of [[run]] tables (the game's theme-shots.toml);
//           RUN is the name of one: its `frames` and `holds` are the play
// FROM TO   the run's frames to record, from 0 at power-on; the frames
//           before FROM are stepped at once and not heard
// THEME     the theme's id on that page
//
// It opens the page, takes the address of its game frame, and puts two
// frames of it in the page instead, each in control mode (`control=1`,
// scripts/web/page/preview.js): the one in the game's own look
// (`own-look`), the other in the theme. Both get the run as a frames file
// and are stepped to the same frame on each tick of the game's clock
// (GP_FPS, 60 a second), and the picture is drawn once both say they are
// there. The sound is the themed frame's, tapped where it reaches the
// speakers. The file shows the person's ROM's pictures and plays its
// sound: never commit it.
import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { homedir } from "node:os";

const here = dirname(fileURLToPath(import.meta.url));
const { chromium } = await import(join(here, "node_modules/playwright-core/index.mjs"));

const [url, momentsFile, runName, from, to, theme, out] = process.argv.slice(2);
if (!out) {
  console.error("usage: node scripts/web/demo-record-run.mjs PAGE_URL MOMENTS.toml RUN FROM TO THEME OUT.webm");
  process.exit(2);
}
const fps = Number(process.env.GP_FPS || 60);
const [lo, hi] = [Number(from), Number(to)];

/** Chrome Headless Shell under $GP_HOME/work, as gp theme look play fetches it. */
function browser() {
  if (process.env.GP_BROWSER) return process.env.GP_BROWSER;
  const root = join(process.env.GP_HOME || join(homedir(), ".gameprinter"), "work/chrome-headless-shell");
  for (const v of existsSync(root) ? readdirSync(root).sort().reverse() : []) {
    for (const d of readdirSync(join(root, v))) {
      const f = join(root, v, d, "chrome-headless-shell");
      if (existsSync(f)) return f;
    }
  }
  throw new Error(`no Chrome Headless Shell under ${root}: gp theme look play --shots fetches one, or name one in GP_BROWSER`);
}

/** The run's holds as a frames file (`gp-frames 1`), as the page check makes them. */
function framesOf() {
  const runs = readFileSync(momentsFile, "utf8").split(/^\[\[run\]\]\s*$/m).slice(1);
  const mine = runs.map((r) => r.split(/^\[\[/m)[0]).find((r) => new RegExp(`^name = "${runName}"\\s*$`, "m").test(r));
  if (!mine) throw new Error(`${momentsFile} has no [[run]] named ${runName}`);
  const n = Number(/^frames = (\d+)/m.exec(mine)?.[1]);
  const body = /^holds = \[([\s\S]*?)\]/m.exec(mine)?.[1];
  if (!n || body === undefined) throw new Error(`${momentsFile}: the run ${runName} needs frames and holds`);
  const bits = { a: 1, b: 2, select: 4, start: 8, up: 16, down: 32, left: 64, right: 128 };
  const pad = new Array(n).fill(0);
  for (const [, h] of body.matchAll(/"([^"]+)"/g)) {
    const [keys, span] = h.split("@");
    const [a, b] = span.split("-");
    const m = keys.split("+").reduce((acc, k) => {
      if (!(k in bits)) throw new Error(`a hold names a key the run hasn't: ${h}`);
      return acc | bits[k];
    }, 0);
    for (let f = Number(a); f <= Math.min(n - 1, Number(b)); f++) pad[f] |= m;
  }
  return { text: "gp-frames 1\n" + pad.map((v) => v.toString(16).padStart(2, "0")).join("\n") + "\n", n };
}

const run = framesOf();
if (!(lo >= 0 && hi > lo && hi < run.n)) {
  console.error(`the run ${runName} has frames 0 to ${run.n - 1}: FROM ${from} and TO ${to} don't fit`);
  process.exit(2);
}
const b = await chromium.launch({ executablePath: browser(), args: ["--autoplay-policy=no-user-gesture-required"] });
const ctx = await b.newContext({ viewport: { width: 1700, height: 900 } });
// Whatever a frame connects to its speakers is connected to a stream too.
await ctx.addInitScript(() => {
  const connect = AudioNode.prototype.connect;
  AudioNode.prototype.connect = function (to, ...rest) {
    if (to instanceof AudioDestinationNode) {
      const c = to.context;
      c.__tap ??= c.createMediaStreamDestination();
      connect.call(this, c.__tap);
    }
    return connect.call(this, to, ...rest);
  };
});
const p = await ctx.newPage();
const said = [];
p.on("pageerror", (e) => said.push(String(e).slice(0, 200)));
const stop = async (why) => {
  console.error(`${why}${said.length ? ` (the page said: ${said.slice(0, 4).join(" ; ")})` : ""}`);
  await b.close();
  process.exit(1);
};
await p.goto(url, { waitUntil: "load" });
try {
  await p.waitForFunction(() => document.getElementById("game")?.src.includes("files="), null, { timeout: 120000 });
} catch {
  await stop("the page didn't start the game: it plays at once only with the game's ROM on this computer");
}
const base = new URL(await p.locator("#game").getAttribute("src"), url);
const frameOf = (look) => {
  const u = new URL(base);
  for (const k of ["theme", "own-look", "keys"]) u.searchParams.delete(k);
  if (look) u.searchParams.set("theme", look);
  else u.searchParams.set("own-look", "1");
  u.searchParams.set("control", "1");
  return u.href;
};
// A click, which the browser wants before a page may sound.
await p.mouse.click(5, 5);

const setup = await p.evaluate(async ({ own, themed, text, lo }) => {
  document.body.innerHTML = "";
  document.body.style.cssText = "margin:0;background:#000;display:flex;gap:8px";
  const got = { ready: new Set(), at: new Map(), failed: [] };
  const fr = {};
  window.__run = { got, fr };
  addEventListener("message", (e) => {
    const who = e.source === fr.a?.contentWindow ? "a" : e.source === fr.b?.contentWindow ? "b" : null;
    const m = e.data;
    if (!who || !m || typeof m.gp !== "string") return;
    if (m.gp === "ready") got.ready.add(who);
    if (m.gp === "failed" || m.gp === "error") got.failed.push(`${who}: ${m.message}`);
    if (m.gp === "at") got.at.set(who, m.frame);
  });
  for (const [k, s] of [["a", own], ["b", themed]]) {
    const f = document.createElement("iframe");
    f.src = s;
    f.style.cssText = "width:820px;height:780px;border:0";
    document.body.append(f);
    fr[k] = f;
  }
  const t0 = performance.now();
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  while (got.ready.size < 2 && !got.failed.length && performance.now() - t0 < 120000) await wait(100);
  if (got.ready.size < 2) return `the two game frames didn't come up: ${got.failed.join(" ; ") || "no word from them in two minutes"}`;
  // The original's half is seen, not heard.
  fr.a.contentWindow.postMessage({ gp: "sound", on: false }, location.origin);
  for (const k of ["a", "b"]) fr[k].contentWindow.postMessage({ gp: "frames", text, frame: lo }, location.origin);
  while ((got.at.get("a") !== lo || got.at.get("b") !== lo) && !got.failed.length && performance.now() - t0 < 240000) await wait(100);
  if (got.at.get("a") !== lo || got.at.get("b") !== lo) return `the frames didn't reach frame ${lo}: ${got.failed.join(" ; ")}`;
  const size = (k) => {
    const c = fr[k].contentDocument.getElementById("game");
    return `${c.width} by ${c.height}`;
  };
  return `ready: the game's own look ${size("a")}, the theme ${size("b")}`;
}, { own: frameOf(null), themed: frameOf(theme), text: run.text, lo });
if (!setup.startsWith("ready")) await stop(setup);
console.log(setup);
// A theme's pictures arrive after its model is up.
await p.waitForTimeout(Number(process.env.GP_SETTLE_MS || 8000));
await p.mouse.click(5, 5);

const started = await p.evaluate(async ({ lo, hi, fps }) => {
  const { got, fr } = window.__run;
  const post = (n) => {
    for (const k of ["a", "b"]) fr[k].contentWindow.postMessage({ gp: "seek", frame: n }, location.origin);
  };
  const there = (n) => got.at.get("a") === n && got.at.get("b") === n;
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  // A frame's sound wakes on the first frame stepped one at a time: a few
  // are stepped so before the recording, from a little before its start.
  for (let n = Math.max(0, lo - 12); n <= lo; n++) {
    post(n);
    const t = performance.now();
    while (!there(n) && performance.now() - t < 5000) await wait(4);
    await wait(16);
  }
  await wait(400);
  const ca = fr.a.contentDocument.getElementById("game");
  const cb = fr.b.contentDocument.getElementById("game");
  const [w, h] = [cb.width, cb.height];
  const both = document.createElement("canvas");
  both.width = w * 2;
  both.height = h;
  const g = both.getContext("2d");
  g.imageSmoothingEnabled = false;
  const paint = () => {
    g.drawImage(ca, 0, 0, w, h);
    g.drawImage(cb, w, 0, w, h);
  };
  paint();
  const stream = both.captureStream(fps);
  // The sound's context is the one the page keeps for its game
  // (`__gpAudio`, scripts/web/public/page.js). Its tap is made here,
  // before the first sound, with silence running into it: a stretch of
  // silence connects nothing, a recording takes no track once it has
  // begun, and one with a track that has had nothing writes nothing.
  let c = null;
  try {
    c = typeof window.__gpAudio === "function" ? window.__gpAudio(true) : null;
  } catch {
    c = null;
  }
  if (c) {
    c.__tap ??= c.createMediaStreamDestination();
    const hush = c.createGain();
    hush.gain.value = 0;
    const osc = c.createOscillator();
    osc.connect(hush);
    hush.connect(c.__tap);
    osc.start();
    for (const tr of c.__tap.stream.getAudioTracks()) stream.addTrack(tr);
  }
  const rec = new MediaRecorder(stream, { mimeType: "video/webm;codecs=vp9,opus", videoBitsPerSecond: 20_000_000 });
  const chunks = [];
  const stats = (window.__run.stats = { late: 0, ticks: 0, done: false, ms: 0, error: null });
  rec.ondataavailable = (e) => chunks.push(e.data);
  rec.onerror = (e) => (stats.error = String(e.error ?? e));
  window.__run.file = new Promise((res) => {
    rec.onstop = async () => {
      const buf = new Uint8Array(await new Blob(chunks).arrayBuffer());
      let s = "";
      for (let i = 0; i < buf.length; i += 0x8000) s += String.fromCharCode.apply(null, buf.subarray(i, i + 0x8000));
      res(btoa(s));
    };
  });
  // One frame of the game a tick of its clock: both halves are asked for
  // it, and the picture is drawn once both are there. A tick that comes
  // late steps on by the clock, at most four frames, which a frame still
  // sounds (preview.js, HEARD_STEPS).
  let asked = lo;
  const t0 = performance.now();
  rec.start(1000);
  const tick = () => {
    if (there(asked)) {
      paint();
      if (asked >= hi) {
        stats.done = true;
        stats.ms = performance.now() - t0;
        setTimeout(() => rec.stop(), 300);
        return;
      }
      const want = lo + Math.floor(((performance.now() - t0) * fps) / 1000) + 1;
      const next = Math.min(hi, Math.max(asked + 1, Math.min(asked + 4, want)));
      if (next - asked > 1) stats.late++;
      stats.ticks++;
      asked = next;
      post(asked);
    }
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
  return `recording ${both.width} by ${both.height}, frames ${lo} to ${hi}, with ${stream.getAudioTracks().length} sound track(s)`;
}, { lo, hi, fps });
console.log(started);
const ms = Math.round(((hi - lo) / fps) * 1000);
try {
  await p.waitForFunction(() => window.__run.stats.done, null, { timeout: ms * 3 + 60000 });
} catch {
  await stop("the recording didn't reach its last frame");
}
const stats = await p.evaluate(() => window.__run.stats);
const bytes = Buffer.from(await p.evaluate(() => window.__run.file), "base64");
if (stats.error || !bytes.length) await stop(`the recorder wrote nothing${stats.error ? `: ${stats.error}` : ""}`);
writeFileSync(out, bytes);
console.log(`wrote ${out}: ${bytes.length} bytes, ${runName} frames ${lo} to ${hi} in ${theme}; ${Math.round(stats.ms)} ms for ${ms} ms of play, ${stats.late} of ${stats.ticks} ticks stepped more than one frame`);
if (said.length) console.log(`the page said: ${said.slice(0, 4).join(" ; ")}`);
await b.close();
