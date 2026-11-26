// Records a corpus clip from the screen page (gp serve's
// /mechanic/GAME/screen.html) as one video with sound: the original on the
// left and the model on the right, in the pictures asked for, each frame of
// both drawn into one picture so the two halves can't drift apart. A demo
// video's side by side, its fade between the two and its wipe are all cut
// from this one file (docs/demo-video.md).
//
//   node scripts/web/demo-record.mjs SCREEN_URL CLIP FROM TO PICTURES OUT.webm
//
// SCREEN_URL  http://127.0.0.1:7796/mechanic/GAME/screen.html
// CLIP        a clip of the game's corpus, by its id
// FROM TO     the clip's frames to record, from 0 at its start
// PICTURES    `game` for the game's own pictures, or a theme's id
//
// GP_LEAD=N takes the clip from N frames before its start (FROM and TO then
// count from there), for a tune that begins just before the clip does.
//
// It opens the page with ?work=CLIP&from=FROM&to=TO, waits for the clip to
// be up and for its loop to come round to FROM, and records one pass of it
// in the page (MediaRecorder on a canvas it draws both screens into, with
// the sound tapped where it reaches the speakers). GP_FPS is the game's
// frames a second (30). The file shows the person's ROM's pictures and
// sound: never commit it.
import { existsSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { homedir } from "node:os";

const here = dirname(fileURLToPath(import.meta.url));
const { chromium } = await import(join(here, "node_modules/playwright-core/index.mjs"));

const [url, clip, from, to, pictures, out] = process.argv.slice(2);
if (!out) {
  console.error("usage: node scripts/web/demo-record.mjs SCREEN_URL CLIP FROM TO PICTURES OUT.webm");
  process.exit(2);
}
const fps = Number(process.env.GP_FPS || 30);
const [lo, hi] = [Number(from), Number(to)];
const ms = Math.round(((hi - lo) / fps) * 1000);

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

const b = await chromium.launch({ executablePath: browser(), args: ["--autoplay-policy=no-user-gesture-required"] });
const ctx = await b.newContext({ viewport: { width: 1600, height: 1400 } });
// Whatever a frame connects to its speakers is connected to a stream too.
await ctx.addInitScript(() => {
  const connect = AudioNode.prototype.connect;
  AudioNode.prototype.connect = function (to, ...rest) {
    if (to instanceof AudioDestinationNode) {
      const c = to.context;
      c.__tap ??= c.createMediaStreamDestination();
      (window.__taps ??= new Set()).add(c.__tap);
      connect.call(this, c.__tap);
    }
    return connect.call(this, to, ...rest);
  };
});
const p = await ctx.newPage();
const said = [];
p.on("pageerror", (e) => said.push(String(e).slice(0, 200)));
const q = new URLSearchParams({ work: clip, from: String(lo), to: String(hi), pictures, view: "side" });
if (Number(process.env.GP_LEAD) > 0) q.set("lead", String(Number(process.env.GP_LEAD)));
await p.goto(`${url}?${q}`, { waitUntil: "load" });
try {
  await p.waitForFunction((id) => document.body.dataset.work === id, clip, { timeout: 180000 });
} catch (e) {
  console.error(`the page didn't set the clip up: ${(await p.evaluate(() => document.getElementById("line")?.textContent)) ?? ""} ${said.join(" ; ")}`);
  await b.close();
  process.exit(1);
}
// The model's first frames in a theme arrive after its pictures do.
await p.waitForTimeout(Number(process.env.GP_SETTLE_MS || 6000));

const started = await p.evaluate(async ({ ms, lo }) => {
  const a = document.getElementById("a");
  const frame = [...document.querySelectorAll("iframe")].find((f) => f.contentDocument?.getElementById("game"));
  const m = frame?.contentDocument.getElementById("game") ?? document.getElementById("b");
  if (!a || !m) return "the page has no two screens";
  const [w, h] = [m.width, m.height];
  const both = document.createElement("canvas");
  both.width = w * 2;
  both.height = h;
  const g = both.getContext("2d");
  g.imageSmoothingEnabled = false;
  const draw = () => {
    g.drawImage(a, 0, 0, w, h);
    g.drawImage(m, w, 0, w, h);
    requestAnimationFrame(draw);
  };
  draw();
  const stream = both.captureStream(30);
  // The model's sound when it plays its own, else the page's.
  const taps = [...(frame?.contentWindow.__taps ?? [])];
  const from = taps.length ? "the model's" : "the page's";
  for (const t of taps.length ? taps : [...(window.__taps ?? [])]) for (const tr of t.stream.getAudioTracks()) stream.addTrack(tr);
  // One pass of the loop: from the clip's frame coming back to its first.
  const at = () => Number(document.body.innerText.match(/frame\s+(\d+)/i)?.[1] ?? -1);
  let last = at();
  await new Promise((res) => {
    const tick = () => {
      const f = at();
      if (f >= 0 && f < last && f <= lo + 2) return res();
      last = f;
      requestAnimationFrame(tick);
    };
    tick();
    // A page that shows no frame number records from now.
    setTimeout(res, ms + 5000);
  });
  const rec = new MediaRecorder(stream, { mimeType: "video/webm;codecs=vp9,opus", videoBitsPerSecond: 20_000_000 });
  const chunks = [];
  rec.ondataavailable = (e) => chunks.push(e.data);
  window.__recDone = new Promise((res) => {
    rec.onstop = async () => {
      const buf = new Uint8Array(await new Blob(chunks).arrayBuffer());
      let s = "";
      for (let i = 0; i < buf.length; i += 0x8000) s += String.fromCharCode.apply(null, buf.subarray(i, i + 0x8000));
      res(btoa(s));
    };
  });
  rec.start(1000);
  setTimeout(() => rec.stop(), ms);
  return `recording ${both.width}x${both.height} for ${ms} ms with ${stream.getAudioTracks().length} sound track(s), ${from}`;
}, { ms, lo });
console.log(started);
if (!started.startsWith("recording")) {
  await b.close();
  process.exit(1);
}
await p.waitForTimeout(ms + 2000);
const bytes = Buffer.from(await p.evaluate(() => window.__recDone), "base64");
writeFileSync(out, bytes);
console.log(`wrote ${out}: ${bytes.length} bytes, ${clip} frames ${lo} to ${hi} in ${pictures}`);
if (said.length) console.log(`the page said: ${said.slice(0, 4).join(" ; ")}`);
await b.close();
