// The public page in a headless browser (docs/web.md, "The public page"):
// with no ROM nothing plays and the page says so (its title, the
// how-to-play panel), a wrong ROM gets a message naming the right one, the
// visitor's own ROM (read from $GP_ROM_DIR by the browser, never copied)
// plays in the page's theme (Turnips, large art drawn by the game's
// module) and in the original's look and layout, through every stage in
// the real game's order, each shot in the ROM's art and in the theme, the
// settings screen's theme and mode changes play on, and "Unload ROM"
// leaves no game playing. Every request
// any frame makes is logged: each must be a GET with no body to the page's
// own server (or a blob: URL), and no WebSocket may open, so nothing of
// the ROM leaves the browser.
//   node scripts/web/public/check-page.mjs --dist OUT --browser BROWSER [--shots DIR] [--full] [--jobs N] [--only REGEX] [--accept-pictures]
// The picture sections (pictures-*) play fixed moments and compare the
// game canvas's pixels with the references in pictures.txt (below,
// "Pictures"); --accept-pictures records new ones.
// With no accepted ROM in $GP_ROM_DIR the ROM steps are skipped and it
// says so. Exit codes: 0 passed, 1 a check failed, 2 couldn't run.
//
// The checks are in sections (SECTIONS, below), each in a browser context
// of its own with its own server, so its storage, its requests and the
// files it reads are its own. Up to --jobs sections run at once (default
// $GP_PAGE_JOBS, else half the cores the load leaves free, at most 6). A
// section that passed is kept in the reuse store ($GP_PAGE_REUSE, default
// $GP_HOME/work/page-release/reuse; page-check/SECTION/) with the files it
// read (every file of the bundle its server sent, check-page.mjs, and what
// else it names) and their SHA-256s, its lines and its shots. A later run
// reuses it, printing its lines as kept, when every one of those files is
// unchanged and the browser is the same. --full runs every section.
// --only runs just the sections whose names match REGEX (rom-.*-play, say)
// and leaves the others out of the run and its lines; the release check
// never passes it.
import { createHash } from "node:crypto";
import { createServer } from "node:http";
import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, renameSync, statSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { cpus, homedir, loadavg } from "node:os";
import { dirname, extname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { offeredIn, themeLineWrong, wanted } from "./theme-line.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, "../../..");
// playwright-core comes from scripts/web/node_modules (`npm ci --prefix
// scripts/web`), or PLAYWRIGHT_FROM names another package.json whose
// node_modules has it.
const from = process.env.PLAYWRIGHT_FROM ?? join(here, "../package.json");
const require_ = createRequire(from);
const { chromium } = require_("playwright-core");
const playwrightVersion = JSON.parse(readFileSync(require_.resolve("playwright-core/package.json"), "utf8")).version;

const opt = (n) => {
  const i = process.argv.indexOf(`--${n}`);
  return i < 0 || !process.argv[i + 1] ? null : process.argv[i + 1];
};
const dist = opt("dist") && resolve(opt("dist"));
const browserPath = opt("browser") && resolve(opt("browser"));
if (!dist || !browserPath) {
  console.error("check-page: give --dist OUT --browser BROWSER");
  process.exit(2);
}
const shots = resolve(opt("shots") ?? join(dist, "..", "page-shots"));
mkdirSync(shots, { recursive: true });
// Every section runs: the game now comes from a registry, which a kept
// result's inputs can't name, so a kept pass could describe other files.
const full = true;
// The page holds no game: it installs one from its source. --play is the
// folder the package's play/ was made from, for what the checks read
// themselves, at /__check/play/, which the page never asks for.
const playDir = opt("play") ? resolve(opt("play")) : `${dist}.play`;
const entry = opt("entry") ?? "index.html";
// --registry-folder answers the page's source from a folder (a dry run, or
// the release check's snapshot of the registry, cloned when it starts, so
// the page and this check read the same files for the whole run);
// --registry-stopped installs, then refuses every request to the source.
const registryFolder = opt("registry-folder") && resolve(opt("registry-folder"));
// --live URL checks a page already deployed there: the page comes from that
// address, and the files a section reads of the game itself from --play. With
// --registry-folder too, a section's installs read that checkout of the source
// instead of the network, so a full run doesn't hit a host's rate limit.
const live = opt("live")?.replace(/\/$/, "") ?? null;
const registryStopped = process.argv.includes("--registry-stopped");
// The themes the source offers for the page's game, by id (a package's name
// after its namespace), from its by-game list: the registry folder's, or with
// no folder the one on the source the page reads (on a live registry anyone
// granted a namespace can publish a theme for the game, which is why the
// release check passes a snapshot), read once for the run. theme-line.mjs
// has the rule the theme line is held to. Only a source with no list for the game, or
// one that can't be reached, leaves the page's own theme.
let offeredOnce = null;
function offeredThemes() {
  offeredOnce ??= (async () => {
    const key = `by-game/${page.game}.toml`;
    let text = null;
    if (registryFolder) text = existsSync(join(registryFolder, key)) ? readFileSync(join(registryFolder, key), "utf8") : null;
    else if (page.source.url === "." && !live) text = existsSync(join(dist, key)) ? readFileSync(join(dist, key), "utf8") : null;
    else {
      const base = page.source.url === "." ? live : page.source.url.replace(/\/$/, "");
      text = await fetch(`${base}/${key}`, { cache: "no-store" })
        .then((r) => (r.ok ? r.text() : null))
        .catch(() => null);
    }
    if (text === null) return page.content ? [page.content.split("/")[1].split("@")[0]] : [];
    return offeredIn(text);
  })();
  return offeredOnce;
}
// A request to the page's source. A source of "." is on the page's own
// address, so only the registry's own keys count.
const REGISTRY_KEY = /^\/(registry\.toml|catalog\.toml|yanks\.toml|namespaces\/|index\/|packages\/|blobs\/|by-game\/)/;
const sourceOrigin = (origin) => new URL(page.source.url, `${origin}/`).origin;
// A source can be under a path, as a repo host's is.
const sourcePath = (origin) => new URL(page.source.url, `${origin}/`).pathname.replace(/\/$/, "");
const sourceKey = (origin, url) => (url.pathname.startsWith(`${sourcePath(origin)}/`) ? url.pathname.slice(sourcePath(origin).length) : null);
const isSourceUrl = (origin, u) => {
  const url = typeof u === "string" ? new URL(u) : u;
  return url.origin === sourceOrigin(origin) && REGISTRY_KEY.test(sourceKey(origin, url) ?? "");
};
const free = Math.max(1, cpus().length - loadavg()[0]);
const jobs = Math.max(1, Number(opt("jobs") ?? process.env.GP_PAGE_JOBS ?? Math.min(6, Math.floor(free / 2))) || 1);
const gpHome = process.env.GP_HOME ?? join(homedir(), ".gameprinter");
const store = join(process.env.GP_PAGE_REUSE ?? join(gpHome, "work", "page-release", "reuse"), "page-check");

const types = { ".html": "text/html", ".css": "text/css", ".js": "text/javascript", ".json": "application/json", ".wasm": "application/wasm", ".png": "image/png", ".toml": "text/plain", ".txt": "text/plain" };
// The bundle's _headers file (Cloudflare Pages format: a path pattern, then
// indented "Name: value" lines; * matches any run of characters), applied
// as Pages does, so the check runs under the Content-Security-Policy the
// deployed page gets. Rules that match add their headers together.
const rules = [];
try {
  let cur = null;
  for (const line of readFileSync(join(dist, "_headers"), "utf8").split("\n")) {
    if (!line.trim() || line.startsWith("#")) continue;
    if (/^\s/.test(line)) cur?.headers.push(line.trim().split(/:\s*(.*)/s).slice(0, 2));
    else rules.push((cur = { re: new RegExp("^" + line.trim().replace(/[.+?^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*") + "$"), headers: [] }));
  }
} catch {}
const headersFor = (path) => {
  const out = new Map();
  for (const r of rules) if (r.re.test(path)) for (const [k, v] of r.headers) out.set(k, out.has(k) ? `${out.get(k)}, ${v}` : v);
  return out;
};

// The accepted ROM, found by its headerless SHA-256 in $GP_ROM_DIR.
const page = JSON.parse(readFileSync(join(dist, "page.json"), "utf8"));
function findRom(sha) {
  const root = process.env.GP_ROM_DIR ?? join(homedir(), ".gameprinter", "roms");
  const stack = [root];
  while (stack.length) {
    const d = stack.pop();
    let names = [];
    try {
      names = readdirSync(d);
    } catch {
      continue;
    }
    for (const n of names) {
      const p = join(d, n);
      const st = statSync(p);
      if (st.isDirectory()) stack.push(p);
      else if (st.size > 16 && st.size < 4 << 20) {
        const b = readFileSync(p);
        if (b[0] !== 0x4e || b[1] !== 0x45 || b[2] !== 0x53 || b[3] !== 0x1a) continue;
        if (createHash("sha256").update(b.subarray(16)).digest("hex") === sha) return p;
      }
    }
  }
  return null;
}

// A model baseline's holds (docs/originals.md) as a frames file
// (`gp-frames 1`, docs/generator.md): "right+a@100-120" holds Right and A
// from frame 100 to 120, both included.
const BITS = { a: 1, b: 2, select: 4, start: 8, up: 16, down: 32, left: 64, right: 128 };
function framesOf(holds, n) {
  const pad = new Array(n).fill(0);
  for (const h of holds) {
    const [keys, span] = h.split("@");
    const [a, b] = span.split("-");
    const bits = keys.split("+").reduce((m, k) => m | BITS[k], 0);
    for (let f = Number(a); f <= (b === "" ? n - 1 : Number(b)); f++) pad[f] |= bits;
  }
  return "gp-frames 1\n" + pad.map((v) => v.toString(16).padStart(2, "0")).join("\n") + "\n";
}

// Each ROM's game played from power-on through every stage it has, in the
// real game's order, to 100m's ending and 25m a loop on. `plays` is that
// order, from the game, not from the model: Original Edition goes 25m,
// 50m, 75m, 100m; Donkey Kong (JU) has no 50m and goes 25m, 75m, 100m
// ($53 from 1 to 3). Each of `stages` is one stage of it: `level` the
// model's level that plays it, `start` a frame at its start with Mario
// standing at `mario`, `shot` a frame of it with `kinds` about, `clear`
// the frame Mario finishes it (the stage counter moves on: the goal on
// 25m, 50m and 75m, the last rivet on 100m), with Mario at a y of `top`
// or less. `ending` is a frame of 100m's ending with `kinds` about, and
// `again` a frame of the next loop's 25m, with Mario at his start;
// `frames` is the run's length. `leftOut` names the rules and entity kinds
// of the model the ROM's game hasn't, which the lifted model must not have
// and no frame may show.
//
// Original Edition's run: 25m by the route `gp level measure --clears`
// found (worker relfix, 2026-11-07), then the original's own presses
// from dk.m100-clear.001's start (power-on to 100m's last rivet on the
// real game, templates/donkey-kong-oe/clips) for 50m, dk.m75-clear.001's
// for 75m and dk.m100-clear.001's again with that clip's own for 100m
// and the ending, each moved to the frame the stage starts on in the
// model (gp interp run on the model, 2026-11-07).
const ROUTES = {
  "donkey-kong-oe": {
    plays: ["main", "m50", "m75", "m100"],
    stages: [
      { name: "25m", level: "main", start: 550, mario: [53, 204], shot: 1200, kinds: ["kong", "barrel", "oil_drum"], clear: 1754, top: 40 },
      { name: "50m", level: "m50", start: 2156, mario: [61, 204], shot: 2500, kinds: ["kong_50m", "pie", "conveyor"], clear: 2852, top: 60 },
      { name: "75m", level: "m75", start: 3256, mario: [21, 188], shot: 3600, kinds: ["kong_75m", "elevator", "spring"], clear: 4401, top: 40 },
      { name: "100m", level: "m100", start: 4807, mario: [61, 204], shot: 5600, kinds: ["kong_100m", "fire"], clear: 6633, top: 60 },
    ],
    ending: { frame: 6900, kinds: ["heart", "kong_fallen", "ending_mario"] },
    again: { frame: 7620, mario: [53, 204] },
    frames: 7700,
    holds: [
      "start@40-44", "right@561-777", "up@778-847", "left@848-986", "up@987-1072", "right@1073-1091",
      "up@1092-1177", "left@1178-1259", "right@1260-1277", "up@1278-1357", "right@1358-1520", "up@1521-1588",
      "left@1589-1679", "up@1680-1753", "right@2156-2209", "up@2210-2304", "left@2315-2322", "up@2323-2440",
      "left@2443-2455", "left+a@2456-2457", "left@2458-2499", "left@2532-2559", "up@2560-2678", "left@2681-2733",
      "up@2734-2851", "right@3257-3260", "up@3261-3385", "right@3386-3395", "up@3396-3567", "right@3577-3582",
      "right+a@3672-3672", "right@3673-3684", "right@3697-3714", "right+a@3715-3715", "right@3716-3755", "right@3952-3986",
      "right+a@3987-3987", "right@3988-4026", "right+a@4027-4027", "right@4028-4047", "right@4067-4098", "right+a@4099-4099",
      "right@4100-4119", "left@4141-4143", "up@4147-4257", "left@4287-4329", "up@4330-4407", "left@4808-4873",
      "up@4874-4968", "right@4969-5279", "up@5280-5397", "left@5402-5688", "up@5689-5806", "right@5815-6077",
      "up@6078-6195", "left@6196-6242", "down@6243-6340", "left@6341-6496", "up@6497-6572", "up@6573-6615",
      "left@6616-6632",
    ],
    // Deaths the route doesn't play (romDeaths, below): `holds` from
    // power-on, Mario dead by frame `hit` at `at` (his body still there,
    // the game holding), back at his start `mario` on `level` by frame
    // `restart`, the game stepping on after it.
    deaths: [
      {
        // The route to 50m and up its second ladder (x 81), stopped a
        // step short of girder 3; a fire coming down from the middle
        // conveyor meets him there (the user, 2026-11-08: "scaling the
        // top of the 2nd ladder, fireball hit me, then the game froze").
        name: "50m: a fire at the top of the ladder at x 81",
        level: "m50",
        holds: [
          "start@40-44", "right@561-777", "up@778-847", "left@848-986", "up@987-1072", "right@1073-1091",
          "up@1092-1177", "left@1178-1259", "right@1260-1277", "up@1278-1357", "right@1358-1520", "up@1521-1588",
          "left@1589-1679", "up@1680-1753", "right@2156-2209", "up@2210-2304", "left@2315-2322", "up@2323-2425",
        ],
        at: [81, 137],
        hit: 3660,
        restart: 4104,
        mario: [61, 204],
        frames: 4400,
      },
    ],
    // A game lost standing still at 25m's start in a profile (`gameOver`,
    // romDeaths): its last death ends in GAME OVER and the title, whose
    // demo then walks Mario to `mario` by `frame`. In Normal the last
    // death held the game for good until 2026-11-08 (quick_restart,
    // templates/donkey-kong-oe/profiles.toml). Since 2026-11-14 Normal's
    // deaths play their whole spin (games bd0815f), so the demo walks
    // Mario there 473 frames later than before.
    gameOver: { profile: "normal", holds: ["start@40-44"], frames: 3500, frame: 3273, mario: [195, 201] },
  },
  "donkey-kong-ju": {
    plays: ["main", "m75", "m100"],
    // Original Edition's stage-clear scene, Kong taking Pauline up: JU's
    // game goes on from the clear with the stage as it stands.
    leftOut: ["clear_scene", "kong_carry", "pauline_carry"],
    stages: [
      { name: "25m", level: "main", start: 550, mario: [53, 204], shot: 1200, kinds: ["kong", "barrel", "oil_drum"], clear: 1730, top: 40 },
      { name: "75m", level: "m75", start: 2134, mario: [21, 188], shot: 2478, kinds: ["kong_75m", "elevator", "spring"], clear: 3288, top: 40 },
      { name: "100m", level: "m100", start: 3685, mario: [61, 204], shot: 4478, kinds: ["kong_100m", "fire"], clear: 5511, top: 60 },
    ],
    ending: { frame: 5778, kinds: ["heart", "kong_fallen", "ending_mario"] },
    again: { frame: 6498, mario: [53, 204] },
    frames: 6578,
    holds: [
      "start@40-44", "right@561-777", "up@778-847", "left@848-986", "up@987-1072", "right@1073-1091", "up@1092-1177",
      "left@1178-1241", "up@1242-1321", "right@1322-1484", "up@1485-1552", "left@1553-1592", "left+a@1593-1593",
      "left@1594-1656", "up@1657-1729", "left+a@2134-2165", "up@2166-2173", "a@2182-2197", "up@2198-2206", "left+a@2255-2278",
      "up@2279-2424", "right@2425-2436", "right+a@2453-2500", "right@2501-2512", "down@2537-2540", "left@2541-2564",
      "right@2565-2570", "a@2571-2602", "up@2603-2604", "right+a@2605-2616", "left+a@2617-2632", "down@2633-2638",
      "a@2639-2639", "left@2640-2663", "left+a@2664-2669", "a@2670-2701", "right+a@2704-2707", "down@2708-2719",
      "down@2752-2763", "right+a@2764-2769", "a@2770-2793", "right@2794-2841", "right+a@2842-2873", "down@2874-2885",
      "right+a@2886-2893", "right@2926-2949", "right@2956-2961", "right+a@2962-2977", "right@2978-3000", "up@3001-3094",
      "left@3175-3218", "up@3219-3288", "left@3686-3751", "up@3752-3846", "right@3847-4157", "up@4158-4275",
      "left@4280-4566", "up@4567-4684", "right@4693-4955", "up@4956-5073", "left@5074-5120", "down@5121-5218",
      "left@5219-5374", "up@5375-5450", "up@5451-5493", "left@5494-5510",
    ],
  },
};
// A Donkey Kong (JU) ROM plays Original Edition's model (its template's
// [[loads]]) over JU's own lifted tables, which move its barrels and 75m
// otherwise, so it has a run of its own, looked up by the folder the lift
// names (result.json's `found.template`). Until 2026-11-06 JU had a model
// and a route of its own; from then until 2026-11-07 the check gave it
// Original Edition's run, which dies on JU's 25m. Its 25m is the route
// `gp level measure --clears 1` finds on JU's lift; its 75m is the one
// worker relfix's search of the page's module found, moved to the frame
// 75m starts on; its 100m and ending are Original Edition's presses from
// dk.m100-clear.001, moved so the first one lands on the first frame
// Mario moves on JU's 100m as on Original Edition's (3686 and 4808). Each
// start was read off the page's module (relfix's tools, gp_web.wasm under
// Node). Found again on 2026-11-07 when JU's map gained the code from
// $C707 on (code_c707, code_c735, code_c74e): it had been zeros, and JU's
// own code moves the barrels on 25m.

// A ROM a template only loads to play ([[loads]]; docs/web.md, "ROMs to
// play") has its own asset map and not every slot of the template's model:
// the assets of what its game does without ([[loads.without]]) are left
// blank, and nothing is filled from anywhere else (until 2026-11-11 they
// came from Rivet Rodeo, the retired open version). Its lift's counts, by
// the folder `found.template` names: `found` of `of` assets from the ROM,
// `blank` left blank, and `without` what the page names as missing from
// the ROM's game. A template's own ROM is held to every asset found.
const LOADS = {
  "donkey-kong-ju": { found: 57, of: 72, blank: 15, without: ["50m", "stage-clear scene"] },
};

// What the game frame gives its AudioContexts, read in every frame: each
// buffer's swing (a frame with sound swings, one level held doesn't) and
// how far ahead of the context's clock it is started. The public page
// keeps the context the game plays through (page.js, `__gpAudio`), so a
// game's buffers are then counted in the page's frame (`soundOf`, below).
// With `check_hold=MS` in a frame's URL, or `__checkHoldNow(MS)` called in
// it, its contexts are held suspended for MS ms whatever the page asks, as
// a browser holds one until its output starts or until a gesture it takes.
function audioProbe() {
  const a = (window.__check = { buffers: 0, loud: 0, lead: null, held: 0 });
  const AC = window.AudioContext;
  if (!AC) return;
  // A browser's rule for starting sound, in a visitor's own context
  // (`window.__checkGesture`), since a headless browser starts every
  // context: "chrome" runs one once this frame, the page around it or a
  // frame in it has had a click or a key, "safari" only when it is made or
  // resumed while one is handled. The gestures are counted here, as Chrome
  // Headless Shell says navigator.userActivation.isActive before any
  // (2026-11-05): a trusted key other than Esc, a press or a touch's end.
  const rule = window.__checkGesture;
  const gesture = { now: false, ever: false };
  window.__checkActivation = gesture;
  for (const type of ["keydown", "mousedown", "pointerdown", "pointerup", "touchend"]) {
    addEventListener(
      type,
      (e) => {
        if (!e.isTrusted || e.key === "Escape") return;
        gesture.now = gesture.ever = true;
        setTimeout(() => (gesture.now = false), 0);
      },
      true,
    );
  }
  const may = () =>
    [window, window.parent, ...Array.from({ length: window.frames.length }, (_, i) => window.frames[i])].some((w) => {
      try {
        const g = w.__checkActivation;
        return rule === "chrome" ? g.ever : g.now;
      } catch {
        return false;
      }
    });
  const made = [];
  let holdUntil = 0;
  window.__checkHoldNow = (ms) => {
    a.held = ms;
    holdUntil = performance.now() + ms;
    for (const c of made) c.suspend();
    setTimeout(() => {
      for (const c of made) c.resumeHeld();
    }, ms);
  };
  window.AudioContext = function (...args) {
    const c = new AC(...args);
    a.made = (a.made ?? 0) + 1;
    made.push(c);
    const resume = c.resume.bind(c);
    c.resumeHeld = resume;
    if ((rule && !may()) || performance.now() < holdUntil) c.suspend();
    c.resume = () => (performance.now() < holdUntil || (rule && !may()) ? Promise.resolve() : resume());
    return c;
  };
  const hold = Number(new URLSearchParams(location.search).get("check_hold") ?? 0);
  if (hold > 0) window.__checkHoldNow(hold);
  window.AudioContext.prototype = AC.prototype;
  const copy = AudioBuffer.prototype.copyToChannel;
  AudioBuffer.prototype.copyToChannel = function (f, ...rest) {
    let lo = 1;
    let hi = -1;
    for (const v of f) {
      if (v < lo) lo = v;
      if (v > hi) hi = v;
    }
    a.buffers++;
    if (hi - lo > 0.01) a.loud++;
    return copy.call(this, f, ...rest);
  };
  const start = AudioBufferSourceNode.prototype.start;
  AudioBufferSourceNode.prototype.start = function (when = 0, ...rest) {
    a.lead = when - this.context.currentTime;
    return start.call(this, when, ...rest);
  };
}

// A section's own server, browser context, page and the helpers its checks
// use. Every file of the bundle the server is asked for is noted in
// `S.read`; the section is reused only while those files are unchanged.
// `opts.preview === false` serves the page's preview opened with no ROM's
// files as an empty page, so a section about ROM mode never runs it; the
// page itself no longer opens it.
async function session(browser, name, opts = {}) {
  const S = { name, lines: [], failures: [], read: new Set(), files: [], requests: [], sockets: [], errors: [], violations: [], runs: new Map() };
  S.check = (ok, what) => {
    S.lines.push(`${ok ? "ok  " : "FAIL"}  ${what}`);
    if (!ok) S.failures.push(what);
  };
  // A file the section writes into the shots folder (a shot, a download):
  // kept with the section, so a reused section's shots are there too.
  S.file = (n) => {
    S.files.push(n);
    return join(shots, n);
  };
  // Input files the checks play in the preview (`run=NAME`), served at
  // game/runs/NAME.frames from here, not from the bundle: button holds only.
  S.server = createServer((req, res) => {
    const url = new URL(req.url, "http://x");
    if (url.pathname.startsWith("/__check/play/")) {
      try {
        const f = join(playDir, decodeURIComponent(url.pathname.slice(14)));
        res.setHeader("content-type", types[extname(f)] ?? "application/octet-stream");
        res.end(readFileSync(f));
      } catch {
        res.statusCode = 404;
        res.end();
      }
      return;
    }
    if (url.pathname === "/p") url.pathname = "/index.html"; // _redirects
    const run = /^\/game\/runs\/(.+)\.frames$/.exec(url.pathname);
    if (run && S.runs.has(run[1])) {
      res.setHeader("content-type", "text/plain");
      res.end(S.runs.get(run[1]));
      return;
    }
    try {
      const rel = decodeURIComponent(url.pathname).replace(/^\/+/, "");
      S.read.add(rel);
      for (const [k, v] of headersFor(url.pathname)) res.setHeader(k, v);
      let p = join(dist, rel);
      if (page.source.url === "." && registryFolder && REGISTRY_KEY.test(url.pathname)) p = join(registryFolder, rel);
      if (statSync(p).isDirectory()) throw new Error("dir");
      res.setHeader("content-type", types[extname(p)] ?? "application/octet-stream");
      res.end(readFileSync(p));
    } catch {
      res.statusCode = 404;
      res.end();
    }
  });
  await new Promise((r) => S.server.listen(0, "127.0.0.1", r));
  S.origin = live ?? `http://127.0.0.1:${S.server.address().port}`;
  const context = (S.context = await browser.newContext({ viewport: { width: 1366, height: 768 } }));
  if (live) {
    const type = (f) => types[extname(f)] ?? "application/octet-stream";
    await context.route((u) => u.origin === live && u.pathname.startsWith("/__check/play/"), (route) => {
      const f = join(playDir, decodeURIComponent(new URL(route.request().url()).pathname.slice(14)));
      return route.fulfill({ path: f, contentType: type(f) }).catch(() => route.abort());
    });
    await context.route((u) => u.origin === live && /^\/game\/runs\/.+\.frames$/.test(u.pathname), (route) => {
      const name = /^\/game\/runs\/(.+)\.frames$/.exec(new URL(route.request().url()).pathname)[1];
      if (!S.runs.has(name)) return route.fallback();
      return route.fulfill({ body: S.runs.get(name), contentType: "text/plain", headers: { "cross-origin-resource-policy": "same-origin" } });
    });
  }
  if (opts.gesture) await context.addInitScript((r) => (window.__checkGesture = r), opts.gesture);
  await context.addInitScript(audioProbe);
  if (opts.gamepad) {
    // A standard gamepad whose held buttons the page sets in window.__pad.
    await context.addInitScript(() => {
      const held = () => {
        try {
          return window.top.__pad ?? [];
        } catch {
          return [];
        }
      };
      navigator.getGamepads = () => [{ connected: true, mapping: "standard", axes: [0, 0, 0, 0], buttons: Array.from({ length: 17 }, (_, i) => ({ pressed: held().includes(i), value: held().includes(i) ? 1 : 0 })) }];
    });
  }
  if (opts.preview === false) {
    await context.route(
      (u) => /^\/game\/(index\.html)?$/.test(u.pathname) && !u.search.includes("files="),
      (r) => r.fulfill({ contentType: "text/html", body: "<!doctype html><title>The preview with no ROM, not loaded in this section</title>" }),
    );
  }
  const isSource = (u) => isSourceUrl(S.origin, u);
  if (registryFolder && page.source.url !== ".") await context.route(isSource, (r) => fromFolder(r));
  if (opts.source) await context.route(isSource, opts.source);
  context.on("request", (r) => S.requests.push({ method: r.method(), url: r.url(), body: r.postDataBuffer()?.length ?? 0 }));
  if (registryStopped) {
    const first = await context.newPage();
    await first.goto(`${S.origin}/${opts.firstLink ?? entry}`);
    await first.waitForFunction(() => window.__page);
    await first.evaluate(() => window.__page.installing);
    await first.close();
    await context.route(isSource, (r) => r.abort());
    S.requests.length = 0;
  }
  const p = (S.p = await context.newPage());
  p.on("websocket", (w) => S.sockets.push(w.url()));
  p.on("pageerror", (e) => S.errors.push(e.message));
  // A Content-Security-Policy refusal is logged to the console, in any frame.
  p.on("console", (m) => /Content Security Policy|Content-Security-Policy|Refused to/i.test(m.text()) && S.violations.push(m.text()));
  const check = S.check;
  const errors = S.errors;
  const game = (S.game = () => p.frame({ url: /\/game\/(index\.html)?(\?|$)/ }));
  const gp = (S.gp = async (f, arg) => (await game()).evaluate(f, arg));
  const waitFrames = (S.waitFrames = async (n, ms = 20000) => {
    const t0 = Date.now();
    while (Date.now() - t0 < ms) {
      const h = await gp(() => window.__gp?.hashes?.length ?? 0).catch(() => 0);
      if (h >= n) return true;
      await p.waitForTimeout(100);
    }
    return false;
  });
  // The game frame pointed at `src`, waited for until the new document
  // has stepped `n` frames. The old document is marked first, so its
  // hashes (long past n) aren't taken for the new one's while it loads:
  // under load the old one was read, and the next step ran against a
  // frame being replaced (rom-aa408f5a-play threw in the release check,
  // 2026-11-07).
  //
  // The new document is also known by its address: the old one, marked or
  // not (a mark that failed to land was passed over in silence before),
  // is never taken for it.
  S.reframe = async (src, n = 30, ms = 30000) => {
    const url = new URL(src, `${S.origin}/`).href;
    const t0 = Date.now();
    while (!(await gp(() => (window.__checkOld = true)).catch(() => false)) && Date.now() - t0 < ms) await p.waitForTimeout(50);
    await p.locator("#game").evaluate((f, s) => (f.src = s), src);
    while (Date.now() - t0 < ms) {
      // Cloudflare serves /game/index.html at /game/.
      const h = await gp((url) => (window.__checkOld || location.href !== url.replace("/game/index.html", location.pathname) ? -1 : (window.__gp?.hashes?.length ?? 0)), url).catch(() => -1);
      if (h >= n) return true;
      await p.waitForTimeout(100);
    }
    return false;
  };
  S.shot = (n) => p.screenshot({ path: S.file(n) });
  // What the game played, counted in the game frame and in the page: the
  // game plays through the page's AudioContext once the page has one
  // (page.js, `__gpAudio`), and its buffers are then the page's.
  S.soundOf = async (pg, fr) => {
    const [x, y] = await Promise.all([pg.mainFrame(), fr].map((f) => f.evaluate(() => ({ ...window.__check }))));
    return { buffers: x.buffers + y.buffers, loud: x.loud + y.loud, lead: y.lead ?? x.lead, held: Math.max(x.held, y.held) };
  };
  S.soundReset = (pg, fr) => Promise.all([pg.mainFrame(), fr].map((f) => f.evaluate(() => Object.assign(window.__check, { buffers: 0, loud: 0, lead: null }))));
  // Keys go to the game's frame, which the page focuses.
  const key = (S.key = async (k, hold = 80) => {
    const f = await game();
    await f.locator("canvas").first().focus().catch(() => {});
    await p.keyboard.down(k);
    await p.waitForTimeout(hold);
    await p.keyboard.up(k);
  });
  // Until `f(arg)`, evaluated in the game frame, is true or `ms` pass;
  // whether it came true. A frame being replaced counts as not yet.
  const until = (S.until = async (f, arg, ms = 15000) => {
    const t0 = Date.now();
    while (Date.now() - t0 < ms) {
      if (await gp(f, arg).catch(() => false)) return true;
      await p.waitForTimeout(50);
    }
    return false;
  });
  // Two turns of the game frame's animation frames, at most `ms`: the
  // game's loop reads the keys pressed so far once a frame (preview.js,
  // settingsFrame), so a key sent before this is taken when it returns.
  const turns = (ms = 15000) =>
    gp((ms) => new Promise((ok) => (requestAnimationFrame(() => requestAnimationFrame(() => ok(true))), setTimeout(() => ok(false), ms))), ms).catch(() => false);
  // The settings screen driven by keys: open, the keys, close; then
  // whether the game steps on, with no error, once it is closed.
  //
  // The keys reach the frame's document at once, but the game's loop
  // takes them only at its next animation frame, and what a close starts
  // (a new mode's restart, a theme's pictures) ends later still. Sent and
  // read on a fixed clock (250 ms, 150 ms, then 1.5 s and 1 s), a frame
  // whose animation frames stalled a few seconds left the screen out of
  // step with the keys: the close's Esc was still waiting at the read
  // ("theme original -> original, 0 frames stepped in 1 s, screen open"),
  // the next step's first Esc closed the screen and its last opened it
  // again, so every later line failed too: the lines rom-45440d8e-play
  // failed with in 2 of 7 runs under load (2026-11-07), which come back
  // each time the frame's animation frames are held 4 s after the first
  // Esc. A renderer slowed 14 times drew the chosen theme after the read
  // ("theme original -> original", 6 of 6 sections).
  // Each key now waits on the screen: open before the keys, each key
  // taken, then closed, no restart under way and `settled` (the mode and
  // theme the close asked for, each [field, value] or [field, "!=",
  // value]), each wait at most 15 s; the game's steps are then counted
  // over 1 s as before.
  S.settings = async (keys, close = "Escape", settled = []) => {
    await key("Escape");
    await until(() => window.__gp.settingsOpen === true);
    for (const k of keys) {
      await key(k);
      await turns();
    }
    if (close === "button") await p.locator("#settings").click();
    else await key(close);
    const want = settled.map((w) => (w.length === 2 ? [w[0], "==", w[1]] : w));
    await until((want) => !window.__gp.settingsOpen && !window.__gp.restarting && want.every(([k, op, v]) => (window.__gp[k] === v) === (op === "==")), want);
    // A page built before `__gp.restarting`: a restart the close started
    // is given the 1.5 s it always had.
    if (!(await gp(() => "restarting" in (window.__gp ?? {})))) await p.waitForTimeout(1500);
    const a = await gp(() => window.__gp.hashes.length);
    await p.waitForTimeout(1000);
    return gp(
      (a) => ({ open: window.__gp.settingsOpen, steps: window.__gp.hashes.length - a, profile: window.__gp.profile, theme: window.__gp.theme, error: window.__gp.error }),
      a,
    );
  };
  S.runs_ = (r) => `${r.profile}, ${r.steps} frames stepped in 1 s, ${r.open ? "screen open" : "screen closed"}${r.error ? `, error ${r.error}` : ""}`;
  // The layer order where a body meets the HUD's background part (its
  // layer and fields): each canvas pixel only one body covers, on a HUD
  // tile, must be the body's, as the NES draws its sprites over the
  // status row. Counts those pixels and the ones that aren't.
  S.overHud = () =>
    gp(async () => {
      // A preview built before __gp.scene can't be asked.
      if (typeof window.__gp.scene !== "function") return { over: 0, under: NaN };
      const s = window.__gp.scene();
      const cfg = await (await fetch("/__check/play/preview.json")).json();
      const canvas = document.getElementById("game");
      const W = canvas.width;
      const shown = canvas.getContext("2d").getImageData(0, 0, W, canvas.height).data;
      const cache = new Map();
      const pic = async (i) => {
        if (!cache.has(i)) {
          const im = new Image();
          im.src = `/__check/play/assets/${cfg.pictures[i]}`;
          await im.decode();
          const c = document.createElement("canvas");
          [c.width, c.height] = [im.width, im.height];
          const x = c.getContext("2d");
          x.drawImage(im, 0, 0);
          cache.set(i, x.getImageData(0, 0, im.width, im.height));
        }
        return cache.get(i);
      };
      const each = async ([n, x0, y0, fx, fy], f) => {
        const d = await pic(n);
        for (let y = 0; y < d.height; y++)
          for (let x = 0; x < d.width; x++) {
            const sx = fx ? d.width - 1 - x : x;
            const sy = fy ? d.height - 1 - y : y;
            const k = (sy * d.width + sx) * 4;
            if (d.data[k + 3] >= 128) f(x0 + x, y0 + y, [d.data[k], d.data[k + 1], d.data[k + 2]]);
          }
      };
      const win = s.window;
      const [hx, hy] = [win.hud[0] - win.x, win.hud[1] - win.y];
      const bg = s.hud.length - (s.hud_sprites ?? 0);
      const hud = new Set();
      const top = new Set();
      for (const d of s.hud.slice(0, bg)) await each(d, (x, y) => hud.add((y + hy) * W + x + hx));
      for (const d of s.hud.slice(bg)) await each(d, (x, y) => top.add((y + hy) * W + x + hx));
      const cover = new Map();
      for (const [i, b] of s.bodies.entries()) {
        if (!b || s.behind?.[i]) continue;
        await each(b, (x, y, c) => {
          const k = (y - win.y) * W + x - win.x;
          cover.set(k, cover.has(k) ? null : c);
        });
      }
      let over = 0;
      let under = 0;
      for (const [k, c] of cover) {
        if (!c || !hud.has(k) || top.has(k)) continue;
        over++;
        if (shown[k * 4] !== c[0] || shown[k * 4 + 1] !== c[1] || shown[k * 4 + 2] !== c[2]) under++;
      }
      return { over, under };
    });

  // The seven save features (docs/shell.md, "Saves on the settings
  // screen"), in the game playing in the frame now, from its title: `tag`
  // names the shots and `who` the lines. The Saves line lists the slots
  // with Resume, the autosave of the level start, and saves, loads and
  // deletes; Download hands a slot over as a file and Load from a file
  // takes it back, refusing a damaged one and another version's with the
  // same sentences; Console only turns off saves, loads and rewind; the
  // quick save buttons act only once turned on, and rewind is on the left
  // trigger; slots kept in localStorage are moved into IndexedDB, and the
  // preview's options are kept by the game, not the build.
  const screenLines = () => gp(() => window.__gp.settingsLines?.() ?? null);
  const hashNow = () => gp(() => window.__gp.hashNow());
  const frameNow = () => gp(() => window.__gp.state().frame);
  const isOpen = () => gp(() => window.__gp.settingsOpen);
  const said = async () => ((await screenLines()) ?? []).at(-1)?.slice(2) ?? "";
  const keptFile = (S.keptFile = (name) =>
    gp(async (name) => {
      const db = await new Promise((ok, no) => {
        const r = indexedDB.open("gp-saves");
        r.onsuccess = () => ok(r.result);
        r.onerror = () => no(r.error);
      });
      const v = await new Promise((ok) => {
        const r = db.transaction("files").objectStore("files").get(name);
        r.onsuccess = () => ok(r.result);
        r.onerror = () => ok(null);
      });
      db.close();
      return v ? Array.from(new Uint8Array(v)) : null;
    }, name));
  // A record's state's frame and its digest's place (gp_sim::save).
  const recordParts = (b) => {
    const buf = Buffer.from(b);
    let at = 8;
    const strs = [];
    for (let i = 0; i < 3; i++) {
      const n = buf.readUInt16LE(at);
      strs.push([at + 2, buf.subarray(at + 2, at + 2 + n).toString()]);
      at += 2 + n;
    }
    return { model: strs[0][1], digestAt: strs[1][0], profile: strs[2][1], state: buf.subarray(at), frame: Number(buf.readBigUInt64LE(at + 8)) };
  };
  // The screen's cursor to the first line starting with `start`.
  const cursorTo = async (start) => {
    for (let i = 0; i < 80; i++) {
      const l = (await screenLines()) ?? [];
      const at = l.findIndex((x) => x.startsWith("> "));
      if (at >= 0 && l[at].slice(2).startsWith(start)) return true;
      await key("ArrowDown", 40);
      await p.waitForTimeout(40);
    }
    return false;
  };
  const openScreen = async () => {
    if (!(await isOpen())) await key("Escape");
    await p.waitForTimeout(250);
  };
  const closeScreen = async () => {
    for (let i = 0; i < 3 && (await isOpen()); i++) {
      await key("Escape");
      await p.waitForTimeout(250);
    }
  };
  const pause = async (on) => {
    const bar = await (await game()).evaluate(() => document.getElementById("game").nextElementSibling?.textContent ?? "");
    const before = await gp(() => window.__gp.hashes.length);
    await p.waitForTimeout(150);
    const moving = (await gp(() => window.__gp.hashes.length)) > before;
    if (moving === on) await key("F2");
    await p.waitForTimeout(150);
    return bar;
  };
  const play = async (n) => {
    await pause(false);
    await waitFrames((await gp(() => window.__gp.hashes.length)) + n);
    await pause(true);
  };
  const pad = (buttons) => gp((b) => (window.__pad = b), buttons);
  // Holds a key or a gamepad button for 400 ms while the game plays: how
  // far the frame went, negative when it rewound. (A count of states seen
  // before can't tell: after a load, plain play passes the same states.)
  const heldFor = async (what) => {
    const f0 = await frameNow();
    if (typeof what === "number") {
      await pad([what]);
      await p.waitForTimeout(400);
      await pad([]);
    } else await key(what, 400);
    return (await frameNow()) - f0;
  };
  S.savesCheck = async (tag, who) => {
    const model = await gp(() => window.__gp.game);
    const slot = (n) => `${model}/saves/slot-${n}.gpsave`;
    const when = (n) => `${model}/saves/slot-${n}.when`;
    await closeScreen();
    // A gamepad the check holds buttons on (window.__pad), in the frame.
    await gp(() => {
      window.__pad = [];
      navigator.getGamepads = () => [{ index: 0, connected: true, mapping: "standard", axes: [0, 0, 0, 0], buttons: Array.from({ length: 17 }, (_, i) => ({ pressed: window.__pad.includes(i), value: window.__pad.includes(i) ? 1 : 0 })) }];
    });
    // 3. Start: the level start autosaves into slot 0, listed as Resume.
    await key("Enter");
    // The stage's start takes about seven seconds, longer while the
    // machine is busy: the autosave is waited for, up to 25 s.
    let auto = null;
    for (const t0 = Date.now(); !auto && Date.now() - t0 < 25000; ) {
      await p.waitForTimeout(500);
      auto = await keptFile(slot(0));
    }
    for (let i = 0; i < 2; i++) await key("ArrowRight", 300);
    check(auto !== null && (await keptFile(when(0))) !== null, `${who}: the level start autosaves into slot 0 with its stamp (frame ${auto ? recordParts(auto).frame : "none"})`);
    // 1. The Saves line: slot 1 saved from it, listed with its time, mode
    // and age, and loaded from it onto the hash of the frame saved.
    await pause(true);
    const h1 = await hashNow();
    const f1 = await frameNow();
    await openScreen();
    check(await cursorTo("Slot 1"), `${who}: the settings screen has a Saves line with Slot 1`);
    let l = await screenLines();
    check(l.some((x) => /^ {2}Resume +\d+:\d\d {2}\w+/.test(x)), `${who}: Resume is listed above the slots (${l.find((x) => x.includes("Resume"))?.trim()})`);
    await key("Enter");
    await p.waitForTimeout(200);
    l = await screenLines();
    check(l[0].trim() === "Slot 1" && l.some((x) => x.trim() === "Save here" || x.trim() === "> Save here" || /Save here/.test(x)), `${who}: choosing Slot 1 asks what to do (${l.slice(0, -1).map((x) => x.trim()).join(", ")})`);
    await cursorTo("Save here");
    await key("Enter");
    await p.waitForTimeout(600);
    l = await screenLines();
    const s1 = await keptFile(slot(1));
    check((await said()) === "Saved to slot 1" && s1 !== null && recordParts(s1).frame === f1 && (await keptFile(when(1))) !== null, `${who}: Save here keeps slot 1 and its stamp in IndexedDB at frame ${f1} ("${await said()}")`);
    const line1 = l.find((x) => x.includes("Slot 1"))?.slice(2) ?? "";
    check(/^Slot 1 {2}\d+:\d\d {2}(Retro|Normal) {2}just now$/.test(line1), `${who}: Slot 1's line has its play time, mode and age ("${line1}")`);
    check(await gp(() => !!window.__gp.persistAsked), `${who}: the first save asks the browser to keep the saves (persisted: ${await gp(() => window.__gp.persisted)})`);
    await S.shot(`${tag}-saves.png`);
    await (await game()).locator("canvas").first().screenshot({ path: S.file(`${tag}-saves-screen.png`) });
    await closeScreen();
    await play(60);
    await openScreen();
    await cursorTo("Slot 1");
    await key("Enter");
    await p.waitForTimeout(150);
    await cursorTo("Load");
    await key("Enter");
    await p.waitForTimeout(800);
    check(!(await isOpen()) && (await hashNow()) === h1, `${who}: Load on the Saves line closes the screen onto the hash of the frame saved`);
    // Delete: slot 2 saved, then deleted with its stamp.
    await openScreen();
    await cursorTo("Slot 2");
    await key("Enter");
    await p.waitForTimeout(150);
    await cursorTo("Save here");
    await key("Enter");
    await p.waitForTimeout(500);
    const s2 = await keptFile(slot(2));
    await key("Enter");
    await p.waitForTimeout(150);
    await cursorTo("Delete");
    await key("Enter");
    await p.waitForTimeout(600);
    l = await screenLines();
    check(s2 !== null && (await keptFile(slot(2))) === null && (await keptFile(when(2))) === null && l.some((x) => /Slot 2 {2}empty/.test(x)), `${who}: Delete removes slot 2 and its stamp ("${await said()}")`);
    // Resume loads the autosave: the newest, since a stage's start can
    // start a level more than once.
    const autoNow = await keptFile(slot(0));
    await cursorTo("Resume");
    await key("Enter");
    await p.waitForTimeout(150);
    await cursorTo("Load");
    await key("Enter");
    await p.waitForTimeout(800);
    const fr = await frameNow();
    check(!(await isOpen()) && autoNow && fr === recordParts(autoNow).frame, `${who}: Resume loads the autosave (frame ${fr})`);
    // 7. Download as a file, and Load from a file after playing on.
    await openScreen();
    await cursorTo("Slot 1");
    await key("Enter");
    await p.waitForTimeout(150);
    await cursorTo("Download");
    const [dl] = await Promise.all([p.waitForEvent("download", { timeout: 10000 }), key("Enter")]);
    const file = S.file(`${tag}-${dl.suggestedFilename()}`);
    await dl.saveAs(file);
    const got = readFileSync(file);
    check(dl.suggestedFilename() === `${model}-slot1.gpsave` && Buffer.compare(got, Buffer.from(s1 ?? [])) === 0, `${who}: Download hands slot 1 over as ${dl.suggestedFilename()}, the bytes kept (${got.length} bytes)`);
    await closeScreen();
    await play(30);
    const fromFile = async (path) => {
      await openScreen();
      await cursorTo("Load from a file");
      const [chooser] = await Promise.all([p.waitForEvent("filechooser", { timeout: 10000 }), key("Enter")]);
      await chooser.setFiles(path);
      await p.waitForTimeout(800);
    };
    await fromFile(file);
    check(!(await isOpen()) && (await hashNow()) === h1, `${who}: Load from a file takes the download back onto the hash of the frame saved`);
    const here = await hashNow();
    const damaged = S.file(`${tag}-damaged.gpsave`);
    writeFileSync(damaged, got.subarray(0, 10));
    await fromFile(damaged);
    const sDamaged = await said();
    const other = Buffer.from(got);
    other.fill("0".charCodeAt(0), recordParts(got).digestAt, recordParts(got).digestAt + 64);
    const otherFile = S.file(`${tag}-other-version.gpsave`);
    writeFileSync(otherFile, other);
    await closeScreen();
    await fromFile(otherFile);
    const sOther = await said();
    check(sDamaged === "Can't load this save: it is damaged" && sOther === "Can't load a save from another version" && (await hashNow()) === here, `${who}: a damaged file and another version's are refused with the same sentences, the game left where it was ("${sDamaged}", "${sOther}")`);
    await closeScreen();
    // 5. Console only: the Saves line says so, and Shift+1, 1 and
    // rewind do nothing.
    await openScreen();
    await cursorTo("[ ] Console only");
    await key("Enter");
    await p.waitForTimeout(200);
    l = await screenLines();
    check(l.some((x) => x.includes("Off: console only is on")) && !l.some((x) => x.includes("Slot 1")), `${who}: with Console only on, the Saves line says saves are off`);
    await (await game()).locator("canvas").first().screenshot({ path: S.file(`${tag}-console-only-screen.png`) });
    await closeScreen();
    const shellFile = await keptFile(`${model}/settings/shell.txt`);
    check(shellFile && Buffer.from(shellFile).toString().includes("console_only = true") && (await gp(() => window.__gp.shellSettings().console_only)), `${who}: Console only is kept through the shell's storage`);
    const s1before = await keptFile(slot(1));
    await p.keyboard.down("Shift");
    await key("1");
    await p.keyboard.up("Shift");
    await p.waitForTimeout(400);
    const hc = await hashNow();
    await key("1");
    await p.waitForTimeout(300);
    const loadedNothing = (await hashNow()) === hc;
    await pause(false);
    await waitFrames((await gp(() => window.__gp.hashes.length)) + 60);
    let went = await heldFor("Backspace");
    check(Buffer.compare(Buffer.from(await keptFile(slot(1))), Buffer.from(s1before)) === 0 && loadedNothing && went > 0, `${who}: under Console only Shift+1 saves nothing, 1 loads nothing and Backspace rewinds nothing (the game went on ${went} frames while it was held)`);
    await openScreen();
    await cursorTo("[X] Console only");
    await key("Enter");
    await closeScreen();
    // 2. The quick save buttons: off, RB saves nothing; on, RB saves to
    // the last slot used and LB loads it; held LT rewinds.
    await pause(true);
    const press = async (b) => {
      await pad([b]);
      await p.waitForTimeout(150);
      await pad([]);
      await p.waitForTimeout(400);
    };
    const s1q = await keptFile(slot(1));
    await press(5);
    check(Buffer.compare(Buffer.from(await keptFile(slot(1))), Buffer.from(s1q)) === 0, `${who}: with Quick save buttons off (the default), RB saves nothing`);
    await openScreen();
    await cursorTo("[ ] Quick save buttons");
    await key("Enter");
    await closeScreen();
    const hq = await hashNow();
    const fq = await frameNow();
    await press(5);
    const s1r = await keptFile(slot(1));
    check(s1r && recordParts(s1r).frame === fq && (await gp(() => window.__gp.lastSlot())) === 1, `${who}: with them on, RB saves to the last slot used, slot 1 (frame ${s1r ? recordParts(s1r).frame : "none"})`);
    await play(60);
    await press(4);
    check((await hashNow()) === hq, `${who}: and LB loads it onto the hash of the frame saved`);
    await pause(false);
    await waitFrames((await gp(() => window.__gp.hashes.length)) + 60);
    went = await heldFor(6);
    check(went < -6, `${who}: held LT rewinds (the frame went back ${-went})`);
    await openScreen();
    await cursorTo("[X] Quick save buttons");
    await key("Enter");
    await closeScreen();
    // 6. The preview's options are kept by the game: F10's flash dimming
    // under gp-view:<game>:, no key by the build's digest.
    await key("F10");
    await p.waitForTimeout(200);
    const views = await gp(() => Object.keys(localStorage).filter((k) => k.startsWith("gp-view:") || k.startsWith("gp-shell:")));
    check(views.includes(`gp-view:${model}:dim`) && !views.some((k) => k.startsWith("gp-shell:")), `${who}: flash dimming is kept by the game (${views.join(", ")})`);
    await key("F10");
    // 4 and 6 after a page update: an option and slots kept the older
    // ways, moved by the next visit (the frame loaded again).
    const digest = await gp(() => window.__gp.cfgDigest);
    const profile = await gp(() => window.__gp.profile);
    const rec = recordParts(s1r);
    await gp(
      ([model, digest, b64, json]) => {
        localStorage.setItem(`gp-save:${model}:slot3`, b64);
        if (digest) {
          localStorage.setItem(`gp-shell:${digest}:slot4`, json);
          localStorage.setItem(`gp-shell:${digest}:mute`, "dmc");
        }
      },
      [model, digest, Buffer.from(s1r).toString("base64"), JSON.stringify({ profile, state: Buffer.from(rec.state).toString("base64") })],
    );
    await (await game()).evaluate(() => location.reload());
    await p.waitForTimeout(500);
    await waitFrames(30);
    const t0 = Date.now();
    while (Date.now() - t0 < 5000 && (await gp(() => window.__gp.movedSlots).catch(() => undefined)) === undefined) await p.waitForTimeout(100);
    const moved = await gp(() => window.__gp.movedSlots);
    const left = await gp(() => Object.keys(localStorage).filter((k) => /^gp-save:|^gp-shell:/.test(k)));
    const s3 = await keptFile(slot(3));
    const s4 = await keptFile(slot(4));
    check(moved === (digest ? 2 : 1) && s3 && (!digest || s4) && left.length === 0, `${who}: slots kept in localStorage before are moved into IndexedDB and their keys removed (${moved} moved, ${left.length} left)`);
    const mute = await gp((m) => localStorage.getItem(`gp-view:${m}:mute`), model);
    check(!digest || mute === "dmc", `${who}: an option kept by the build's digest is kept by the game after an update (mute ${mute})`);
    await pause(true);
    await key("3");
    await p.waitForTimeout(300);
    check((await hashNow()) === hq, `${who}: the moved slot 3 loads onto the hash it was saved at`);
    if (digest) {
      await play(30);
      await key("4");
      await p.waitForTimeout(300);
      check((await hashNow()) === hq, `${who}: and slot 4, kept as a state before saves were records, loads too`);
      await key("9");
      await p.waitForTimeout(200);
    }
    await pause(false);
    await gp(() => delete navigator.getGamepads);
    check(errors.length === 0, `${who}: no page errors in the save features (${errors.join("; ")})`);
  };

  // A ROM picked on the page (as a visitor picks it, read where it lies),
  // waited for until the page has lifted it and loads the ROM version.
  // Returns what the page said.
  S.loadRom = async (rom) => {
    await p.evaluate(() => (document.getElementById("message").dataset.kind = ""));
    await p.setInputFiles("#rom", rom);
    await p.waitForFunction(() => ["ok", "warn"].includes(document.getElementById("message").dataset.kind), null, { timeout: 60000 });
    const said = await p.locator("#message").innerText();
    await p.waitForFunction(() => document.getElementById("game").src.includes("files=idb:"), null, { timeout: 5000 });
    return said;
  };
  // The page done installing (page.js sets __page once the game and its
  // content are in this browser, or couldn't be): true when the game is.
  S.installed = async (ms = 60000) => {
    await p.waitForFunction(() => window.__page !== undefined, null, { timeout: ms }).catch(() => {});
    return (await installedOf(p)) !== null;
  };
  // Canvas pixels to a game pixel in the game's frame: more than 1 while a
  // large-art look (gp_present::art) is drawn.
  S.artScale = async () => (await game()).evaluate(() => document.getElementById("game").gpScale ?? 1);
  return S;
}

// ---- The sections ----

// The page with no ROM: installed with no click, its title, nothing
// playing and the header saying so, the line naming the ROM it takes, the
// how-to-play panel, and a theme the lift couldn't offer named in words.
async function noRomPage(S) {
  const { p, check, errors } = S;
  await p.goto(`${S.origin}/${entry}`);
  check(await S.installed(), "the game is installed with no click");
  const title = await p.title();
  check(title === page.title, `the page is titled ${page.title} (${title})`);
  check(!/mario|pauline|jumpman/i.test(await p.locator("header").innerText()), "the header names no Nintendo character");
  check(!(await p.locator("#game").getAttribute("src")) && !(await p.locator("#game").isVisible()), "nothing plays until a ROM is loaded");
  check((await p.locator("#mode").innerText()) === "No ROM loaded", "and the header says so");
  check(await p.locator("#load-label").isVisible(), "Load your own ROM is offered");
  {
    // In the middle of the empty page, large, not a black screen.
    const b = await p.locator("#load-label").boundingBox();
    const m = await p.locator("main").boundingBox();
    const off = Math.abs(b.x + b.width / 2 - (m.x + m.width / 2)) + Math.abs(b.y + b.height / 2 - (m.y + m.height / 2));
    check(b.height >= 50 && off < m.height / 4, `and it is a large button in the middle of the page (${Math.round(b.width)}x${Math.round(b.height)}, ${Math.round(off)} px off centre)`);
  }
  const help = await p.locator("#rom-help").innerText();
  check(help.includes(page.rom.name) && /never uploaded/.test(help), `the line under it names the ROM it takes and says it isn't uploaded (${help.slice(0, 80)}...)`);
  await S.shot("no-rom.png");
  // The how-to-play panel: open on a first visit, collapses, and stays
  // collapsed after a reload (localStorage).
  const panel = p.locator("#howto");
  check(await panel.isVisible(), "the how-to-play panel is open on a first visit");
  check(/arrows move, X jumps, Enter starts/.test(await panel.innerText()) && /Esc or the Settings button/.test(await panel.innerText()), "it names the controls and the settings");
  check(await p.locator("#settings").isVisible(), "the Settings button is visible");
  await p.locator("#howto-toggle").click();
  check(!(await panel.isVisible()), "the toggle collapses it");
  check((await p.locator("#howto-toggle").getAttribute("aria-expanded")) === "false", "and says so (aria-expanded)");
  await p.reload();
  check(!(await p.locator("#howto").isVisible()), "it stays collapsed after a reload");
  await p.locator("#howto-toggle").click();
  check(await p.locator("#howto").isVisible(), "the toggle opens it again");
  await p.reload();
  check(await p.locator("#howto").isVisible(), "and it stays open after a reload");
  // A ROM theme the lift couldn't offer is named under the mode line, in
  // words a player reads. The lift's result is made up here (its reasons
  // are crates/gp-web-lift's), since a real one needs the person's ROM;
  // gp-web-lift's own test lifts a theme that can't load and one that
  // can't draw.
  await S.installed();
  check(!(await p.locator("#skipped").isVisible()), "with no ROM no theme is named as not offered");
  await p.evaluate(() =>
    window.__page.showSkipped([
      { id: "small", title: "Small Tiles", reason: "it doesn't fit the art read from your ROM", detail: "themes/small/tiles.pix: a restyle of `tiles` is 8 by 8, not 128 by 128" },
      { id: "unread", title: "Unread", reason: "its files have a mistake in them", detail: "restyle[0].sha256: tiles.pix has SHA-256 ..." },
    ]),
  );
  const skippedLine = p.locator("#skipped");
  const skippedText = await skippedLine.innerText();
  check(
    (await skippedLine.isVisible()) &&
      skippedText === "ROM themes not offered: Small Tiles, because it doesn't fit the art read from your ROM; Unread, because its files have a mistake in them.",
    `a theme the lift couldn't offer is named with its reason: "${skippedText}"`,
  );
  check(!/[`_]|\.pix|sha-?256|restyle/i.test(skippedText), "in plain words, with no file names or code terms");
  const mb = await p.locator("#mode").boundingBox();
  const sb = await skippedLine.boundingBox();
  check(sb.y >= mb.y + mb.height - 1, `the line sits under the mode line (mode ends ${Math.round(mb.y + mb.height)}, line ${Math.round(sb.y)})`);
  await S.shot("theme-not-offered.png");
  await p.evaluate(() => window.__page.play("none"));
  check(!(await skippedLine.isVisible()), "and no ROM playing hides it again");
  check(errors.length === 0, `no page errors (${errors.join("; ")})`);
}

// A wrong ROM: a made-up image, no game's.
async function wrongRom(S) {
  const { p, check } = S;
  await p.goto(`${S.origin}/${entry}`);
  const fake = S.file("not-a-game.nes");
  const body = Buffer.alloc(16 + 16384 + 8192);
  body.set([0x4e, 0x45, 0x53, 0x1a, 1, 1]);
  for (let i = 16; i < body.length; i++) body[i] = (i * 7919) & 0xff;
  writeFileSync(fake, body);
  await p.setInputFiles("#rom", fake);
  await p.waitForFunction(() => document.getElementById("message").dataset.kind === "error", null, { timeout: 30000 });
  const msg = await p.locator("#message").innerText();
  check(msg.includes(page.rom.name), `a wrong ROM is refused naming ${page.rom.name}: "${msg.slice(0, 120)}..."`);
  await S.shot("wrong-rom.png");
}

// The visitor's own ROM, read by the browser where it lies: lifted with
// every asset found (a ROM loaded only to play, with the counts LOADS
// gives) and every ROM theme offered, opened in the page's theme (Turnips,
// its large art drawn by the module), the title, sound from Start and with
// the context held 4 s, the settings screen and its button, then Theme to
// Original, Normal and back to Retro, and Theme back to Turnips, each
// closed with the game playing on. (A theme change failed with "present
// text doesn't start with `gp-present 15`" and the next restart froze the
// game, 2026-11-04.)
async function romPlay(S, { name, sha, rom }) {
  const { p, check, gp, key, waitFrames, settings, runs_ } = S;
  await p.goto(`${S.origin}/${entry}`);
  const said = await S.loadRom(rom);
  const tag = sha.slice(0, 8);
  check(true, `${name} is lifted in the browser: ${said}`);
  const notOffered = (await p.locator("#skipped").isVisible()) ? await p.locator("#skipped").innerText() : "";
  check(!notOffered, `every ROM theme the page ships is offered with ${name}${notOffered ? `: "${notOffered}"` : ""}`);
  const lifted = await p.evaluate(async (game) => (await (await import("./game/lift.js")).storedMeta(game))?.found ?? null, page.rom.game);
  const load = LOADS[lifted?.template];
  if (!load) check(/all \d+ of its assets found/.test(said), `every asset of ${name} is found`);
  else {
    // A ROM loaded only to play: the page says how many of the model's
    // assets it found and counts those left blank with what of the game
    // uses them, as the lift's result has them. Nothing is filled.
    const by = (k) => (lifted.assets ?? []).filter((a) => a.from === k);
    const without = [...new Set(by("blank").map((a) => a.without))];
    const got = { found: by("same").length + by("changed").length, of: lifted.assets?.length ?? 0, blank: by("blank").length, without };
    check(JSON.stringify(got) === JSON.stringify(load), `${name}, a ROM ${lifted.plays} loads to play, lifts with ${load.found} of ${load.of} assets found and ${load.blank} left blank for ${load.without.join(" and ")} (${JSON.stringify(got)})`);
    const others = (lifted.assets ?? []).filter((a) => !["same", "changed", "blank"].includes(a.from));
    check(!others.length && !/stand-in|filled/.test(said), `none of ${name}'s assets is filled from anywhere else, and the page names no stand-in (${others.map((a) => `${a.name} ${a.from}`).join(", ") || "none"})`);
    const told = `${load.blank} left blank, since your ROM's game has no ${load.without.join(" and no ")}`;
    check(said.includes(`${load.found} of ${load.of} assets found`) && said.includes(told), `and the page says so ("${said}")`);
  }
  check(await waitFrames(30), "the ROM version starts");
  check((await gp(() => window.__gp.error)) === null, "and loads without an error");
  await p.waitForTimeout(800);
  await S.shot(`rom-${tag}-title.png`);
  // The page's theme, with its large art (gp_present::art): a finished look
  // the lift passed through, opened by the page.
  const kept = await p.evaluate(async (game) => {
    const files = await (await import("./game/lift.js")).loadStored(game);
    const text = (n) => (files?.get(n) ? new TextDecoder().decode(files.get(n)) : "");
    return { look: text("themes/turnips/look.txt"), art: text("themes/turnips/art.txt").slice(0, 8) };
  }, page.rom.game);
  check(/^art art\.txt$/m.test(kept.look) && kept.art === "gp-art 1", `${name}: the lifted game holds the page's theme, Turnips, as a finished look`);
  check((await gp(() => window.__gp.theme)) === "turnips", `${name}: the ROM opens in it`);
  const scale = await S.artScale();
  check(scale > 1, `${name}: drawn in its large art, ${scale} canvas pixels to a game pixel (rom-${tag}-title.png)`);
  check((await p.title()) === page.title, `with a ROM loaded the page is still titled ${page.title}`);
  check((await p.textContent("#title")) === page.title, `with a ROM loaded the header still says ${page.title}`);
  await S.soundReset(p, await S.game());
  await key("Enter");
  // Sound from the game's start, before any death: the stage's start
  // takes about six seconds and Mario can't die in it.
  await p.waitForTimeout(3000);
  const early = await S.soundOf(p, await S.game());
  check(early.loud >= 60, `${name}: sound plays within 3 s of Start, before any death (${early.loud} of ${early.buffers} frames with sound)`);
  check(early.lead !== null && early.lead < 0.3, `${name}: and it is played at once (${early.lead === null ? "nothing played" : early.lead.toFixed(2) + " s ahead"})`);
  await p.waitForTimeout(4000);
  for (let i = 0; i < 3; i++) await key("ArrowRight", 400);
  await p.waitForTimeout(400);
  await S.shot(`rom-${tag}-play.png`);
  // A browser that starts the AudioContext late (its output starting,
  // or its autoplay rules waiting for another gesture) must not leave
  // the sound behind the game once it runs.
  const romSrc = await p.locator("#game").getAttribute("src");
  // The page's context is held too, and what the page counted so far set
  // aside: the new frame counts from nothing.
  await p.evaluate(() => (window.__checkHoldNow(4000), Object.assign(window.__check, { buffers: 0, loud: 0, lead: null })));
  check(await S.reframe(`${romSrc}&check_hold=4000`), `${name}: the ROM version starts again with its AudioContext held 4 s`);
  await key("Enter");
  await p.waitForTimeout(6000);
  const late = await S.soundOf(p, await S.game());
  check(late.held === 4000 && late.loud > 0 && late.lead !== null && late.lead < 0.3, `${name}: with the context held 4 s, the sound is played at once when it runs (${late.lead === null ? "nothing played" : late.lead.toFixed(2) + " s ahead"}, ${late.loud} frames with sound)`);
  // The settings screen in ROM mode: Esc, F1 and the top bar's button open
  // it, and the focus goes back to the game, so Esc closes it with no click.
  check(await S.reframe(romSrc), `${name}: the ROM version plays again`);
  await key("Escape");
  await p.waitForTimeout(300);
  check(await gp(() => window.__gp.settingsOpen), "Esc opens the in-game settings screen");
  await S.shot(`rom-${tag}-settings.png`);
  await key("Escape");
  await p.waitForTimeout(300);
  check(!(await gp(() => window.__gp.settingsOpen)), "Esc closes it");
  await key("F1");
  await p.waitForTimeout(300);
  check(await gp(() => window.__gp.settingsOpen), "F1 still opens it");
  await key("Escape");
  await p.waitForTimeout(300);
  await p.locator("#settings").click();
  await p.waitForTimeout(300);
  check(await gp(() => window.__gp.settingsOpen), "the Settings button opens the in-game screen");
  check(await p.evaluate(() => document.activeElement === document.getElementById("game")), "the focus is back on the game");
  await key("Escape");
  await p.waitForTimeout(300);
  check(!(await gp(() => window.__gp.settingsOpen)), "Esc closes it after the button");
  await p.locator("#settings").click();
  await p.waitForTimeout(300);
  await p.locator("#settings").click();
  await p.waitForTimeout(300);
  check(!(await gp(() => window.__gp.settingsOpen)), "the Settings button pressed again closes the screen");
  // Theme: Original and Turnips, and nothing else.
  const lookWas = await gp(() => window.__gp.theme);
  const offered = await gp(() => window.__gp.looks?.() ?? null).catch(() => null);
  // Steps along the theme line to one theme: with more themes offered,
  // Original isn't always the next one.
  const offeredBySource = await offeredThemes();
  const want = wanted(offeredBySource);
  const toTheme = async (id, key) => {
    let r;
    for (let i = 0; i < want.length; i++) {
      r = await settings(["ArrowDown", key], "Escape", [["theme", "!=", "?"]]);
      if (r.theme === id) break;
    }
    return r;
  };
  let rs = await toTheme("original", "ArrowLeft");
  const bar = async () => (await S.game()).evaluate(() => document.getElementById("game").nextElementSibling?.textContent ?? "");
  const barSaid = await bar();
  await S.shot(`rom-${tag}-settings-original.png`);
  check(rs.theme === "original" && !/not changed/i.test(barSaid) && rs.steps > 30 && !rs.error, `${name}: Theme Original chosen on the settings screen is drawn and the game plays (theme ${lookWas} -> ${rs.theme}, "${barSaid}", ${runs_(rs)})`);
  check((await S.artScale()) === 1, `${name}: in the original's pixels, one canvas pixel to a game pixel`);
  rs = await settings(["ArrowRight"], "Escape", [["profile", "normal"]]);
  check(rs.profile === "normal" && rs.theme === "original" && rs.steps > 30 && !rs.error, `${name}: then Normal, closed: the game plays (${runs_(rs)})`);
  rs = await settings(["ArrowLeft"], "Escape", [["profile", "retro"]]);
  check(rs.profile === "retro" && rs.theme === "original" && rs.steps > 30 && !rs.error, `${name}: then back to Retro, closed: the game plays (${runs_(rs)})`);
  await S.shot(`rom-${tag}-retro-again.png`);
  rs = await toTheme("turnips", "ArrowRight");
  check(rs.theme === "turnips" && rs.steps > 30 && !rs.error && (await S.artScale()) > 1, `${name}: and Theme back to Turnips, in its large art (${runs_(rs)})`);
  await S.shot(`rom-${tag}-turnips-again.png`);
  // The theme line offers Original and every theme the source offers for
  // the game (its by-game list), and nothing else: stepped right until a
  // theme comes round again.
  const seen = new Set(["turnips"]);
  for (let i = 0; i <= want.length; i++) {
    rs = await settings(["ArrowDown", "ArrowRight"], "Escape", [["theme", "!=", "?"]]);
    if (seen.has(rs.theme)) break;
    seen.add(rs.theme);
  }
  const got = [...seen].sort();
  const wrong = themeLineWrong(got, offeredBySource);
  check(!wrong, `${name}: ${wrong ? `${wrong}; ` : ""}the theme line offers Original and each theme the source offers for the game, and nothing else (${got.join(", ")}; the source offers ${want.slice().filter((t) => t !== "original").join(", ")}${offered ? `, offered ${JSON.stringify(offered)}` : ""})`);
  const stored = await p.evaluate(async (game) => {
    const { loadStored } = await import("./game/lift.js");
    return (await loadStored(game))?.size ?? 0;
  }, page.rom.game);
  check(stored > 10, `the lifted game is in this browser's IndexedDB (${stored} files)`);
}

// The ROM's game from power-on through every stage it has, in the real
// game's order (ROUTES' `plays`): the lifted model's stages list plays
// that order, and a run clears each stage in turn. Each stage starts with
// Mario at his start, is shown in the ROM's art and in Turnips with its big
// foe and what it has about (rom-TAG-25m.png, rom-TAG-25m-turnips.png, and
// so on) and is cleared; the level played after each clear is the next
// of `plays`, so a stage that plays another level fails. After 100m, its
// ending (rom-TAG-ending.png) and the next loop's 25m. Until 2026-11-07
// the page played 25m in 50m's and 100m's places (build-page.sh's
// GP_LIFT_LEVELS=main,m75), and this section held it to that.
async function romRoute(S, { name, sha, rom }) {
  const { p, check, gp } = S;
  await p.goto(`${S.origin}/${entry}`);
  await S.loadRom(rom);
  check(await S.waitFrames(30), `${name}: the ROM version starts`);
  const tag = sha.slice(0, 8);
  // In the ROM's own art: the page opens it in its theme.
  const romSrc = (await p.locator("#game").getAttribute("src")).replace(/theme=[^&]*/, "own-look");
  const found = await p.evaluate(async (game) => (await (await import("./game/lift.js")).storedMeta(game))?.found ?? null, page.rom.game);
  const template = found?.template;
  const c = ROUTES[template];
  check(!!c, `${name}: the check has a route through its game for its model (${template})`);
  if (!c) return;
  const names = (levels) => levels.map((l) => c.stages.find((st) => st.level === l)?.name ?? l).join(", ");
  // The stages list the lift gave the model, less the stages it passes
  // over (stages.skip): the order a player meets them in.
  const list = await p.evaluate(async (game) => {
    const files = await (await import("./game/lift.js")).loadStored(game);
    const model = JSON.parse(new TextDecoder().decode(files.get("bundle.json"))).model;
    const table = /\n\[stages\]\n((?:[^[\n][^\n]*\n)*)/.exec(model)?.[1] ?? "";
    const list = (key) => {
      const m = new RegExp(`^${key} = (\\[[^\\]]*\\])`, "m").exec(table);
      return m ? JSON.parse(m[1]) : [];
    };
    return { order: list("order"), skip: list("skip") };
  }, page.rom.game);
  const played = list.order.filter((l) => !list.skip.includes(l));
  check(JSON.stringify(played) === JSON.stringify(c.plays), `${name}: the lifted model plays the game's stages in its order, ${names(c.plays)} (order ${JSON.stringify(list.order)}, skipped ${JSON.stringify(list.skip)})`);
  if (c.leftOut) {
    const still = await p.evaluate(
      async ({ game, parts }) => {
        const files = await (await import("./game/lift.js")).loadStored(game);
        const model = JSON.parse(new TextDecoder().decode(files.get("bundle.json"))).model;
        return parts.filter((n) => model.includes(`[rules.${n}]`) || model.includes(`[entities.${n}]`));
      },
      { game: page.rom.game, parts: c.leftOut },
    );
    check(!still.length, `${name}: the lifted model leaves out what the game hasn't, ${c.leftOut.join(", ")}${still.length ? ` (still there: ${still.join(", ")})` : ""}`);
  }
  const run = `route-${template}`;
  S.runs.set(run, framesOf(c.holds, c.frames));
  const at = async (frame, extra = "") => {
    // The frame's old document is marked, so its state isn't taken for the
    // new one's while the new one loads (two calls in a row at the same
    // frame, or at an earlier one, took the old state and read the new
    // document's theme before it was set: "look undefined" under load,
    // 2026-11-05).
    await gp(() => (window.__checkOld = true)).catch(() => {});
    await p.locator("#game").evaluate((f, s) => (f.src = s), `${romSrc}${extra}&run=${run}&from=${frame}&to=${frame}`);
    await p.waitForFunction(() => document.getElementById("game").src.includes("run="), null, { timeout: 5000 });
    const t0 = Date.now();
    while (Date.now() - t0 < 60000) {
      const st = await gp(() => (window.__checkOld ? null : (window.__gp?.state?.() ?? null))).catch(() => null);
      if (st && st.frame >= frame) return st;
      await p.waitForTimeout(100);
    }
    return null;
  };
  const shot = async (frame, file, extra = "") => {
    const st = await at(frame, extra);
    await p.waitForTimeout(300);
    await S.shot(file);
    return st;
  };
  const player = (st) => st?.bodies.find((b) => b[0] === "player" && b[4]);
  const kinds = (st) => [...new Set(st?.bodies.filter((b) => b[4]).map((b) => b[0]))];
  const where = (st) => `level ${st?.level ?? "none"}, Mario at ${player(st)?.slice(1, 3) ?? "none"}`;
  // The page's theme drawn over this ROM's art, at a fixed frame of
  // the same run, so the picture compares with an earlier page's.
  const paper = await shot(c.stages[0].clear - 120, `rom-${tag}-turnips.png`, "&theme=turnips");
  const look = await gp(() => window.__gp.theme);
  check(!!paper && look === "turnips", `${name}: the page's theme, Turnips, is drawn over its art (look ${look}, frame ${paper?.frame})`);
  for (const [i, st] of c.stages.entries()) {
    const want = c.plays[i];
    const what = `${name}, stage ${i + 1} (${st.name})`;
    check(st.level === want, `${what}: the route plays the game's stage ${i + 1} here (${want}; the route has ${st.level})`);
    const begun = await at(st.start);
    const [x, y] = player(begun)?.slice(1, 3) ?? [];
    check(begun?.level === want && x === st.mario[0] && y === st.mario[1], `${what} is played by its own level from frame ${st.start}, Mario at his start ${st.mario} (${where(begun)})`);
    const mid = await shot(st.shot, `rom-${tag}-${st.name}.png`);
    const missing = st.kinds.filter((k) => !kinds(mid).includes(k));
    check(mid?.level === want && missing.length === 0, `${what} in the ROM's art at frame ${st.shot}: ${st.kinds.join(", ")} there${missing.length ? ` (missing: ${missing.join(", ")})` : ""} (level ${mid?.level}, rom-${tag}-${st.name}.png)`);
    const midPaper = await shot(st.shot, `rom-${tag}-${st.name}-turnips.png`, "&theme=turnips");
    const lookNow = await gp(() => window.__gp.theme);
    check(midPaper?.level === want && lookNow === "turnips", `${what}: and in Turnips (level ${midPaper?.level}, look ${lookNow}, rom-${tag}-${st.name}-turnips.png)`);
    const done = await at(st.clear + 1);
    check(done?.level === want && player(done)?.[2] <= st.top, `${what}: the run finishes it at frame ${st.clear} (${where(done)})`);
    if (i === 0) {
      // Through the clear: shots a second, two and a bit over three in (by
      // then the screen is off for the next stage's setup), and none of
      // what the game leaves out on any of them.
      const frames = [st.clear + 60, st.clear + 120, st.clear + 200];
      const after = [];
      for (const f of frames) after.push(await shot(f, f === frames[0] ? `rom-${tag}-clear.png` : `rom-${tag}-clear-${f - st.clear}.png`));
      const shown = [...new Set(after.flatMap((a) => kinds(a)))].filter((k) => (c.leftOut ?? []).includes(k));
      check(after.every((a) => a?.level === want) && !shown.length, `${what}: the clear plays on ${st.name} (frames ${frames.join(", ")}${shown.length ? `; shows ${shown.join(", ")}` : ""}, rom-${tag}-clear.png)`);
    }
  }
  const last = c.plays[c.plays.length - 1];
  const end = await shot(c.ending.frame, `rom-${tag}-ending.png`);
  const gone = c.ending.kinds.filter((k) => !kinds(end).includes(k));
  check(end?.level === last && gone.length === 0, `${name}: 100m's ending plays at frame ${c.ending.frame}: ${c.ending.kinds.join(", ")} there${gone.length ? ` (missing: ${gone.join(", ")})` : ""} (level ${end?.level}, rom-${tag}-ending.png)`);
  const endPaper = await shot(c.ending.frame, `rom-${tag}-ending-turnips.png`, "&theme=turnips");
  check(endPaper?.level === last, `${name}: and in Turnips (level ${endPaper?.level}, rom-${tag}-ending-turnips.png)`);
  const again = await shot(c.again.frame, `rom-${tag}-loop-2.png`);
  const [ax, ay] = player(again)?.slice(1, 3) ?? [];
  check(again?.level === c.plays[0] && ax === c.again.mario[0] && ay === c.again.mario[1], `${name}: then the game goes round, the next loop's first stage played by ${c.plays[0]} with Mario at his start ${c.again.mario} at frame ${c.again.frame} (${where(again)}, rom-${tag}-loop-2.png)`);
}

// Deaths off the route (ROUTES' `deaths`), each played from power-on in
// the ROM's art: Mario dies where it says, the game holds through the
// death and starts the stage again with him at his start, and it steps on
// from there in real time with no error. Until 2026-11-07 the page never
// played 50m, so no death there had been played in the browser.
async function romDeaths(S, { name, rom }) {
  const { p, check, gp } = S;
  await p.goto(`${S.origin}/${entry}`);
  await S.loadRom(rom);
  check(await S.waitFrames(30), `${name}: the ROM version starts`);
  const romSrc = await p.locator("#game").getAttribute("src");
  const found = await p.evaluate(async (game) => (await (await import("./game/lift.js")).storedMeta(game))?.found ?? null, page.rom.game);
  const c = ROUTES[found?.template];
  check(!!c, `${name}: the check has a route through its game for its model (${found?.template})`);
  if (!c?.deaths?.length && !c?.gameOver) return;
  const player = (st) => st?.bodies.find((b) => b[0] === "player" && b[4]);
  const where = (st) => `frame ${st?.frame ?? "none"}, level ${st?.level ?? "none"}, Mario at ${player(st)?.slice(1, 3) ?? "none"}`;
  // The run played to `from`, then on in real time to `to`: the state once
  // it reaches `want` (null if it doesn't in 30 s), and the error the
  // game frame met.
  const play = async (run, from, to, want, extra = "") => {
    await gp(() => (window.__checkOld = true)).catch(() => {});
    await p.locator("#game").evaluate((f, s) => (f.src = s), `${romSrc}${extra}&run=${run}&from=${from}&to=${to}`);
    const t0 = Date.now();
    let st = null;
    let error = null;
    while (Date.now() - t0 < 30000) {
      const r = await gp(() => (window.__checkOld ? null : { st: window.__gp?.state?.() ?? null, error: window.__gp?.error ?? null })).catch(() => null);
      if (r) {
        st = r.st;
        error = r.error;
        if (error || (st && st.frame >= want)) break;
      }
      await p.waitForTimeout(100);
    }
    return { st: st && st.frame >= want ? st : null, last: st, error };
  };
  const o = c.gameOver;
  if (o) {
    S.runs.set("game-over", framesOf(o.holds, o.frames));
    const what = `${name}: a game lost at 25m's start in ${o.profile}`;
    const end = await play("game-over", o.frame, o.frame, o.frame, `&profile=${o.profile}`);
    const profile = await gp(() => window.__gp.profile).catch(() => null);
    const [ox, oy] = player(end.st)?.slice(1, 3) ?? [];
    check(profile === o.profile && ox === o.mario[0] && oy === o.mario[1] && !end.error, `${what}: the last death ends in GAME OVER and the title's demo, Mario walked to ${o.mario} by frame ${o.frame} (${profile}, ${where(end.last)}${end.error ? `, error ${end.error}` : ""})`);
  }
  for (const [i, d] of (c.deaths ?? []).entries()) {
    const run = `death-${i}`;
    S.runs.set(run, framesOf(d.holds, d.frames));
    const what = `${name}: ${d.name}`;
    const hit = await play(run, d.hit, d.hit, d.hit);
    const [hx, hy] = player(hit.st)?.slice(1, 3) ?? [];
    check(hit.st?.level === d.level && hx === d.at[0] && hy === d.at[1] && !hit.error, `${what}: dead at ${d.at} by frame ${d.hit} (${where(hit.last)}${hit.error ? `, error ${hit.error}` : ""})`);
    // From the death on in real time, as a player meets it.
    const on = await play(run, d.hit, d.frames, d.restart + 60);
    check(!!on.st && !on.error, `${what}: the game steps on through the death to frame ${d.restart + 60} with no error (${where(on.last)}${on.error ? `, error ${on.error}` : ""})`);
    const back = await play(run, d.restart, d.restart, d.restart);
    const [bx, by] = player(back.st)?.slice(1, 3) ?? [];
    check(back.st?.level === d.level && bx === d.mario[0] && by === d.mario[1] && !back.error, `${what}: the stage starts again by frame ${d.restart}, Mario at his start ${d.mario} (${where(back.last)}${back.error ? `, error ${back.error}` : ""})`);
  }
}

// The seven save features in the ROM's game.
// The runtime shell's saves and rewind (docs/shell.md) in the ROM's game:
// Shift+1 saves while paused, into IndexedDB under the standard's slot
// name; 1 loads it back onto the hash of the frame saved; held Backspace
// steps back onto states the run passed through. Then the seven save
// features.
async function romSaves(S, { name, sha, rom }) {
  const { p, check, gp, key, waitFrames, errors } = S;
  await p.goto(`${S.origin}/${entry}`);
  await S.loadRom(rom);
  check(await waitFrames(30), `${name}: the ROM version starts`);
  const game = await gp(() => window.__gp.game);
  const last = () => gp(() => window.__gp.hashes[window.__gp.hashes.length - 1]);
  await key("F2");
  await p.waitForTimeout(300);
  const savedHash = await last();
  await p.keyboard.down("Shift");
  await key("1");
  await p.keyboard.up("Shift");
  await p.waitForTimeout(500);
  const keptSave = await S.keptFile(`${game}/saves/slot-1.gpsave`);
  check(keptSave !== null && String.fromCharCode(...keptSave.slice(0, 4)) === "GPSV", `Shift+1 keeps a save record in IndexedDB (${keptSave ? keptSave.length : 0} bytes)`);
  await key("F2");
  check(await waitFrames((await gp(() => window.__gp.hashes.length)) + 60), "the game plays on after a save");
  await key("F2");
  await key("1");
  await p.waitForTimeout(300);
  check((await last()) === savedHash, "1 loads slot 1 onto the hash of the frame saved");
  await key("F2");
  await waitFrames((await gp(() => window.__gp.hashes.length)) + 60);
  const before = await gp(() => window.__gp.hashes.length);
  await key("Backspace", 400);
  const back = await gp((n) => {
    const h = window.__gp.hashes;
    const seen = new Set(h.slice(0, n));
    return h.slice(n).filter((x) => seen.has(x)).length;
  }, before);
  check(back > 3, `held Backspace rewinds onto states the run passed through (${back} steps)`);
  check(errors.length === 0, `no page errors after saves and rewind (${errors.join("; ")})`);
  await S.savesCheck(`rom-${sha.slice(0, 8)}`, name);
}

// "Unload ROM" leaves no game playing, and the page asks for a ROM again.
async function romUnload(S, { name, rom }) {
  const { p, check, waitFrames } = S;
  await p.goto(`${S.origin}/${entry}`);
  await S.loadRom(rom);
  check(await waitFrames(30), `${name}: the ROM version starts`);
  await p.locator("#back").click();
  await p.waitForTimeout(300);
  check(!(await p.locator("#game").getAttribute("src")) && !(await p.locator("#game").isVisible()), "Unload ROM leaves no game playing");
  check((await p.locator("#mode").innerText()) === "No ROM loaded" && (await p.locator("#load-label").isVisible()), "and the page asks for a ROM again");
}

// Sound from the game's start in ROM mode, as a new visitor plays it:
// the ROM loaded by a click on the page, then Start and play from the
// keyboard or a gamepad, under Chrome's rule and Safari's for starting
// sound. With a gamepad there was none at all (the user, 2026-11-04:
// "no sound until Mario's first death"): the sound started on a key or a
// click in the game only, and a gamepad's buttons are neither. Under
// Safari's rule the click on Load was no help until 2026-11-05 (the
// user: no sound until a key), as the game's frame made its context
// after the click; the page now keeps one, made in the click. A ROM
// loaded with no click at all (`setInputFiles`, as no visitor can) has
// no context under Safari's rule: the game asks a gamepad player for a
// click or a key before it plays, and one click starts the sound.
async function visitor(S, { rule, input, load, rom }) {
  const { check } = S;
  const v = S.p;
  const vgame = () => v.frame({ url: /\/game\/(index\.html)?\?files=idb/ });
  const press = async (k, ms = 100) => {
    if (input === "gamepad") {
      const b = { Enter: 9, ArrowRight: 15 }[k];
      await v.evaluate((b) => (window.__pad = [b]), b);
      await v.waitForTimeout(ms);
      await v.evaluate(() => (window.__pad = []));
    } else {
      await v.keyboard.down(k);
      await v.waitForTimeout(ms);
      await v.keyboard.up(k);
    }
  };
  const heard = async (more = {}) => ({ ...(await S.soundOf(v, vgame())), state: await vgame().evaluate(() => window.__gp.audioState?.() ?? "?"), ...more });
  const asked = () => vgame().evaluate(() => !!document.getElementById("sound-prompt"));
  const who = `a new visitor under ${rule === "chrome" ? "Chrome's" : "Safari's"} rule for sound, playing with ${input === "gamepad" ? "a gamepad" : "the keyboard"}${load === "none" ? ", the ROM loaded with no click" : ""}`;
  await v.goto(`${S.origin}/${entry}`);
  await v.waitForTimeout(1500);
  if (load === "click") {
    const [chooser] = await Promise.all([v.waitForEvent("filechooser"), v.locator("#load-label").click()]);
    await chooser.setFiles(rom);
  } else await v.setInputFiles("#rom", rom);
  await v.waitForFunction(() => ["ok", "warn"].includes(document.getElementById("message").dataset.kind), null, { timeout: 60000 });
  await v.waitForFunction(() => document.getElementById("game").src.includes("files=idb:"), null, { timeout: 5000 });
  const t0 = Date.now();
  while (Date.now() - t0 < 20000 && !((await vgame()?.evaluate(() => window.__gp?.hashes?.length ?? 0).catch(() => 0)) >= 60)) await v.waitForTimeout(100);
  await S.soundReset(v, vgame());
  await press("Enter");
  await v.waitForTimeout(500);
  let prompt = await asked();
  if (load === "none") {
    // No gesture has started the sound: the game holds and asks, and
    // one click on it starts the sound.
    const s = await heard();
    check(prompt && s.loud === 0, `${who}: the game asks for a click or a key before it plays (${prompt ? "asked" : "not asked"}, ${s.loud} frames with sound, context ${s.state})`);
    // A click on the game, where the question is, as a person makes it.
    const box = await v.locator("#game").boundingBox();
    await v.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
    await v.waitForTimeout(300);
    check(!(await asked()), `${who}: one click takes the question away`);
    await S.soundReset(v, vgame());
    await press("Enter");
    await v.waitForTimeout(3000);
    const after = await heard();
    check(after.loud >= 60, `${who}: and the sound plays within 3 s of Start (${after.loud} of ${after.buffers} frames with sound, context ${after.state})`);
  } else {
    await v.waitForTimeout(2500);
    let s = await heard();
    check(s.loud >= 60 && !prompt, `${who}: sound plays within 3 s of Start, before any death, with no question asked (${s.loud} of ${s.buffers} frames with sound, context ${s.state}${prompt ? ", asked" : ""})`);
    for (let i = 0; i < 5; i++) await press("ArrowRight", 400);
    await S.soundReset(v, vgame());
    await v.waitForTimeout(2000);
    s = await heard({ frame: await vgame().evaluate(() => window.__gp.state().frame) });
    prompt = await asked();
    check(s.loud >= 20 && !prompt, `${who}: and on through play (${s.loud} of ${s.buffers} frames with sound to frame ${s.frame})`);
  }
}

// A later visit: "Play my ROM version" lifts the kept ROM again, so its
// files are this page's (on 2026-11-04 the user played files an older
// page kept, with no themes, and saw no Theme line). Files kept with no
// ROM, as a page from before then kept them, are dropped.
// A returning visitor who chose Paper on an earlier visit (kept in
// localStorage as gp-theme:MODEL) picks their ROM: the game must open in
// the page's theme. A kept Paper drew the ROM's game in the paper look,
// and the page looked as if the ROM never loaded (2026-11-07, reported
// from Safari and missed here, since every section starts in a fresh
// browser).
async function keptLook(S, { name, rom }) {
  const { p, check } = S;
  await p.goto(`${S.origin}/${entry}`);
  await S.installed();
  await p.evaluate(() => {
    for (const m of ["donkey_kong_oe", "donkey_kong_ju"]) localStorage.setItem(`gp-theme:${m}`, "paper");
  });
  await S.loadRom(rom);
  // The ROM's frame, and never the one it replaces: on a host that redirects
  // the frame's address, the old document is still there for a moment.
  const romFrame = () => p.frames().find((f) => /[?&]files=idb:/.test(f.url()));
  await p.waitForFunction(() => document.getElementById("game").src.includes("files=idb:"), null, { timeout: 5000 });
  const t0 = Date.now();
  while (!romFrame() && Date.now() - t0 < 30000) await p.waitForTimeout(50);
  const g = romFrame();
  await g.waitForFunction(() => window.__gp && window.__gp.theme !== undefined && window.__gp.hashes?.length > 30, null, { timeout: 60000 });
  const theme = await g.evaluate(() => window.__gp.theme);
  await S.shot("kept-look.png");
  check(theme === "turnips", `${name}: with Paper kept from an earlier visit, a ROM picked opens in the page's theme, Turnips (theme ${JSON.stringify(theme)}, kept-look.png)`);
}

// Theme picks that overlap. A look's pictures are decoded before it is
// drawn, and the player can pick again, or a new mode can restart the
// game, in that wait. On the live page (2026-11-14) a large-art theme picked
// with the JU ROM ended twice in 14 runs in Original, the bar saying
// "Theme not changed: no large-art look loaded": a second pick cleared the
// first one's art in the module while its pictures were decoded. Here the
// game frame's picture decoding (createImageBitmap) waits until the check
// lets it go, batch by batch, so each overlap happens every run: a pick
// made while another loads, a restart while one loads, and an older pick
// whose pictures come in after a newer one is drawn. The newest pick is
// drawn each time, no pick says it failed, and the settings screen always
// opens on the theme drawn.
async function themePicks(S, { name, sha, rom }) {
  const { p, check, gp, key, until } = S;
  await p.goto(`${S.origin}/${entry}`);
  await S.loadRom(rom);
  check((await S.waitFrames(30)) && (await gp(() => window.__gp.theme)) === "turnips", `${name}: the ROM opens in Turnips`);
  const index = await p.evaluate(async (game) => {
    const t = (await (await import("./game/lift.js")).loadStored(game))?.get("themes/index.json");
    return t ? JSON.parse(new TextDecoder().decode(t)).themes : [];
  }, page.rom.game);
  const titles = new Map([["original", "Original"], ...index.map((x) => [x.id, x.title])]);
  const others = index.map((x) => x.id).filter((id) => id !== "turnips");
  if (others.length < 2) {
    S.lines.push(`skip  ${name}: overlapping theme picks need two themes besides Turnips; the lift offers ${[...titles.keys()].join(", ")}`);
    return;
  }
  const [late, slow] = others.slice(-2);
  await gp(() => {
    const decode = window.createImageBitmap;
    const gate = (window.__artGate = []);
    window.createImageBitmap = (...a) => {
      const b = gate.at(-1);
      if (!b || b.open) return decode.apply(window, a);
      b.held++;
      return b.wait.then(() => decode.apply(window, a));
    };
    // Every message the bar shows, in order.
    const bar = document.getElementById("game").nextElementSibling;
    window.__said = [];
    new MutationObserver(() => window.__said.at(-1) !== bar.textContent && window.__said.push(bar.textContent)).observe(bar, { childList: true, characterData: true, subtree: true });
  });
  const hold = () =>
    gp(() => {
      const b = { held: 0, open: false };
      b.wait = new Promise((ok) => (b.go = ok));
      return window.__artGate.push(b) - 1;
    });
  const holding = (n) => until((n) => window.__artGate[n].held > 0, n);
  const release = (n) => gp((n) => ((window.__artGate[n].open = true), window.__artGate[n].go()), n);
  const turns = () => gp(() => new Promise((ok) => (requestAnimationFrame(() => requestAnimationFrame(() => ok(true))), setTimeout(() => ok(false), 15000)))).catch(() => false);
  const lines = async () => (await gp(() => window.__gp.settingsLines?.() ?? null)) ?? [];
  const cursor = async () => (await lines()).find((l) => l.startsWith("> ")) ?? "";
  const themeLine = async () => ((await lines()).find((l) => /^(> | {2})Theme /.test(l)) ?? "").replace(/^(> | {2})Theme\s+<\s*/, "").replace(/\s*>\s*$/, "");
  const step = async (k) => {
    await key(k);
    await turns();
  };
  const cursorTo = async (start) => {
    for (let i = 0; i < 40 && !(await cursor()).slice(2).startsWith(start); i++) await step("ArrowDown");
  };
  // The screen opened, the Theme line set to `id` (and the Mode line to
  // `mode`), closed: what the screen opened on and what was drawn then.
  // The pick itself isn't waited for.
  const pick = async (id, mode = null) => {
    const drawn = await gp(() => window.__gp.theme);
    await key("Escape");
    await until(() => window.__gp.settingsOpen === true);
    const shown = await themeLine();
    if (mode) {
      await cursorTo("Mode");
      for (let i = 0; i < 6 && !(await cursor()).includes(`< ${mode} >`); i++) await step("ArrowRight");
    }
    await cursorTo("Theme");
    for (let i = 0; i <= titles.size && (await themeLine()) !== titles.get(id); i++) await step("ArrowRight");
    const set = (await themeLine()) === titles.get(id);
    await key("Escape");
    await until(() => !window.__gp.settingsOpen);
    return { drawn, shown, set };
  };
  const opened = [];
  const picked = async (id, mode) => {
    const r = await pick(id, mode);
    opened.push(r);
    return r.set;
  };
  const now = () => gp(() => ({ theme: window.__gp.theme, profile: window.__gp.profile, restarting: window.__gp.restarting, scale: document.getElementById("game").gpScale ?? 1, said: window.__said.slice() }));
  const failed = (said) => said.filter((x) => /not changed|not applied/i.test(x));
  const told = (said) => JSON.stringify(said.filter((x) => /^(Theme|Settings: \w+\.)/.test(x)));
  const settle = async (id) => {
    await until((id) => window.__gp.theme === id && !window.__gp.restarting, id);
    for (let i = 0; i < 10; i++) await turns();
    return now();
  };

  // 1. A pick while another's pictures are decoded.
  let b = await hold();
  let ok = await picked(late);
  check(ok && (await holding(b)), `${name}: ${titles.get(late)} picked, its pictures held`);
  ok = await picked("original");
  await release(b);
  let n = await settle("original");
  check(ok && n.theme === "original" && n.scale === 1 && !failed(n.said).length, `${name}: Original picked while ${titles.get(late)} loads is drawn, and neither pick says it failed (theme ${n.theme}, scale ${n.scale}, the bar said ${told(n.said)})`);

  // 2. A new mode's restart while a pick's pictures are decoded.
  b = await hold();
  ok = await picked(late, "normal");
  check(ok && (await holding(b)), `${name}: ${titles.get(late)} and Normal picked together, its pictures held`);
  await until(() => window.__gp.profile === "normal" && !window.__gp.restarting);
  await release(b);
  n = await settle(late);
  check(n.theme === late && n.profile === "normal" && n.scale > 1 && !failed(n.said).length, `${name}: the game restarts in Normal while ${titles.get(late)} loads, and it is drawn after (theme ${n.theme}, ${n.profile}, scale ${n.scale}, the bar said ${told(n.said)})`);

  // 3. An older pick whose pictures come in after a newer one is drawn.
  const first = await hold();
  ok = await picked(slow);
  check(ok && (await holding(first)), `${name}: ${titles.get(slow)} picked, its pictures held`);
  const second = await hold();
  ok = await picked("turnips");
  check(ok && (await holding(second)), `${name}: then Turnips, its pictures held too`);
  await release(second);
  n = await settle("turnips");
  check(n.theme === "turnips", `${name}: Turnips, picked last, is drawn first (theme ${n.theme})`);
  await release(first);
  for (let i = 0; i < 20; i++) await turns();
  n = await now();
  await S.shot(`theme-picks-${sha.slice(0, 8)}.png`);
  check(n.theme === "turnips" && n.scale > 1 && !failed(n.said).length, `${name}: and stays drawn when ${titles.get(slow)}'s pictures come in after it (theme ${n.theme}, scale ${n.scale}, the bar said ${told(n.said)}, theme-picks-${sha.slice(0, 8)}.png)`);

  // The screen opened each time on the theme drawn.
  const off = opened.filter((o) => o.shown !== titles.get(o.drawn));
  await key("Escape");
  await until(() => window.__gp.settingsOpen === true);
  const last = await themeLine();
  check(!off.length && last === titles.get(n.theme), `${name}: the settings screen opens on the theme drawn, each of ${opened.length + 1} times (${off.map((o) => `"${o.shown}" over ${o.drawn}`).join(", ") || `last "${last}"`})`);
  await key("Escape");
}

async function laterVisit(S, { rom }) {
  const { p, check, waitFrames } = S;
  await p.goto(`${S.origin}/${entry}`);
  await S.installed();
  await S.loadRom(rom);
  await waitFrames(30);
  const stamp = async () => p.evaluate(async (game) => (await (await import("./game/lift.js")).storedMeta(game))?.at ?? 0, page.rom.game);
  const at0 = await stamp();
  await p.evaluate(() => sessionStorage.setItem("check-reloaded", "1"));
  await p.reload();
  await p.waitForFunction(() => ["ok", "warn", "error"].includes(document.getElementById("message").dataset.kind) && document.getElementById("game").src.includes("files=idb:"), null, { timeout: 60000 }).catch(() => {});
  check((await stamp()) > at0, "a later visit lifts the kept ROM again, with no click");
  check(await waitFrames(30), "and plays it");
  check(await p.getByRole("button", { name: "Play my ROM" }).isVisible(), "with Play my ROM there to lift it again");
  const looks = await p.evaluate(async (game) => {
    const files = await (await import("./game/lift.js")).loadStored(game);
    return JSON.parse(new TextDecoder().decode(files.get("preview.json"))).looks === true && files.has("themes/index.json");
  }, page.rom.game);
  check(looks, "so the ROM version it plays has this page's themes");
  await p.evaluate(async (game) => {
    const db = await new Promise((ok, fail) => {
      const r = indexedDB.open("gp-public", 1);
      r.onsuccess = () => ok(r.result);
      r.onerror = () => fail(r.error);
    });
    const tx = db.transaction("files", "readwrite");
    tx.objectStore("files").delete(`${game}/.rom`);
    await new Promise((ok) => (tx.oncomplete = ok));
    db.close();
  }, page.rom.game);
  await p.reload();
  await S.installed();
  await p.waitForTimeout(1000);
  check(!(await p.getByRole("button", { name: "Play my ROM" }).isVisible()) && !(await p.locator("#game").getAttribute("src")), "files an older page kept with no ROM aren't played or offered");
  const left = await p.evaluate(async (game) => (await (await import("./game/lift.js")).loadStored(game))?.size ?? 0, page.rom.game);
  check(left === 0, `and are dropped (${left} files left)`);
}

// ---- Pictures: the game's canvas at fixed frames, against references ----

// The references (scripts/web/public/pictures.txt): for each moment, the
// run and frame it is played to, what a person sees there, and the
// SHA-256 of the game canvas's RGBA pixels at that frame, all text (the
// repo holds no pictures). A moment with `pending` is played and its
// picture written out, but not compared: its reference is recorded once
// the reason is gone (remove the line and accept). A moment with `look`
// is played in that look (`&theme=`, the Turnips look a player picks in
// Settings), and fails if the frame shows another. A ROM moment with
// `press` plays the route with those holds added (`a@1512-1513`, more
// apart by spaces), for a moment the route itself doesn't have. `--accept-pictures`
// runs only the picture sections (unless --only names others) and writes
// each moment's new hash; the accepted pictures are kept by hash in
// $GP_HOME/work/page-pictures/, so a later difference shows the old one.
// A difference fails its section naming the moment, with old, new and
// diff pictures in $GP_HOME/work/page-shots/.
const picturesFile = join(here, "pictures.txt");
const acceptPictures = process.argv.includes("--accept-pictures");
const pictureStore = join(gpHome, "work", "page-pictures");
const pictureShots = join(gpHome, "work", "page-shots");
function readPictures() {
  const out = [];
  let cur = null;
  for (const raw of readFileSync(picturesFile, "utf8").split("\n")) {
    const line = raw.trimEnd();
    if (!line.trim() || line.trimStart().startsWith("#")) continue;
    const head = /^moment (\S+)$/.exec(line);
    if (head) {
      out.push((cur = { id: head[1] }));
      continue;
    }
    const kv = /^\s+(\w+)\s+(.*)$/.exec(line);
    if (!kv || !cur) throw new Error(`${picturesFile}: can't read "${line}"`);
    cur[kv[1]] = kv[1] === "frame" ? Number(kv[2]) : kv[2];
  }
  return out;
}
function writePictures(moments) {
  const head = readFileSync(picturesFile, "utf8")
    .split("\n")
    .filter((l, i, a) => a.slice(0, i + 1).every((x) => !x.trim() || x.startsWith("#")))
    .join("\n");
  const keys = ["section", "frame", "look", "press", "shows", "state", "size", "sha256", "pending"];
  const body = moments.map((m) => [`moment ${m.id}`, ...keys.filter((k) => m[k] !== undefined && m[k] !== "").map((k) => `  ${k.padEnd(7)} ${m[k]}`)].join("\n"));
  writeFileSync(picturesFile, `${head.trimEnd()}\n\n${body.join("\n\n")}\n`);
}
const pictureMoments = existsSync(picturesFile) ? readPictures() : [];
// What each section found, by moment, for --accept-pictures.
const pictureResults = new Map();

// The game canvas now: its size, the SHA-256 of its RGBA pixels and a PNG.
async function canvasNow(S) {
  const got = await S.gp(() => {
    const c = document.getElementById("game");
    const d = c.getContext("2d").getImageData(0, 0, c.width, c.height).data;
    let bin = "";
    for (let i = 0; i < d.length; i += 0x8000) bin += String.fromCharCode(...d.subarray(i, i + 0x8000));
    return { w: c.width, h: c.height, rgba: btoa(bin), png: c.toDataURL("image/png") };
  });
  const rgba = Buffer.from(got.rgba, "base64");
  return { size: `${got.w}x${got.h}`, sha256: createHash("sha256").update(rgba).digest("hex"), png: Buffer.from(got.png.split(",")[1], "base64"), w: got.w, h: got.h };
}

// The old picture against the new, in the game frame: a diff picture (the
// new one dimmed, each pixel that differs in red) and the count.
async function diffOf(S, oldPng, newPng) {
  const r = await S.gp(
    async ([a, b]) => {
      const load = async (u) => {
        const im = new Image();
        im.src = u;
        await im.decode();
        const c = document.createElement("canvas");
        [c.width, c.height] = [im.width, im.height];
        const x = c.getContext("2d");
        x.drawImage(im, 0, 0);
        return x.getImageData(0, 0, im.width, im.height);
      };
      const [o, n] = [await load(a), await load(b)];
      const c = document.createElement("canvas");
      [c.width, c.height] = [n.width, n.height];
      const x = c.getContext("2d");
      const d = x.createImageData(n.width, n.height);
      let differ = 0;
      for (let i = 0; i < n.data.length; i += 4) {
        const same = o.width === n.width && o.data[i] === n.data[i] && o.data[i + 1] === n.data[i + 1] && o.data[i + 2] === n.data[i + 2] && o.data[i + 3] === n.data[i + 3];
        if (!same) differ++;
        d.data.set(same ? [n.data[i] >> 2, n.data[i + 1] >> 2, n.data[i + 2] >> 2, 255] : [255, 0, 0, 255], i);
      }
      x.putImageData(d, 0, 0);
      return { differ, png: c.toDataURL("image/png") };
    },
    [`data:image/png;base64,${oldPng.toString("base64")}`, `data:image/png;base64,${newPng.toString("base64")}`],
  );
  return { differ: r.differ, png: Buffer.from(r.png.split(",")[1], "base64") };
}

// A run played in the game frame to one frame: the frame's old document is
// marked so its state isn't taken for the new one's; then the state.
async function playTo(S, src, run, frame, extra = "") {
  const { p, gp } = S;
  await gp(() => (window.__checkOld = true)).catch(() => {});
  await p.locator("#game").evaluate((f, s) => (f.src = s), `${src}${src.includes("?") ? "&" : "?"}run=${run}&from=${frame}&to=${frame}${extra}`);
  const t0 = Date.now();
  while (Date.now() - t0 < 60000) {
    const st = await gp(() => (window.__checkOld ? null : (window.__gp?.state?.() ?? null))).catch(() => null);
    if (st && st.frame >= frame) {
      await p.waitForTimeout(300);
      return st;
    }
    await p.waitForTimeout(100);
  }
  return null;
}

// What a frame's state says, for the references' text: the level, the
// game's state and where the player stands.
const stateText = (st) => {
  const pl = st?.bodies?.find((b) => b[0] === "player" && b[4]);
  return `level ${st?.level ?? "?"}${pl ? `, player at ${pl[1]},${pl[2]}` : ", no player drawn"}`;
};

// Each moment of the section played and compared.
async function pictureMomentsCheck(S, section, src, runOf) {
  const { check } = S;
  const found = [];
  mkdirSync(pictureStore, { recursive: true });
  mkdirSync(pictureShots, { recursive: true });
  for (const m of pictureMoments.filter((x) => x.section === section)) {
    const st = await playTo(S, src, runOf(m), m.frame, m.look ? `&theme=${m.look}` : "");
    if (!st) {
      check(false, `picture ${m.id}: the run didn't reach frame ${m.frame}`);
      continue;
    }
    if (m.look) {
      const look = await S.gp(() => window.__gp.theme);
      if (look !== m.look) {
        check(false, `picture ${m.id}: played in look ${look}, not ${m.look}`);
        continue;
      }
    }
    const now = await canvasNow(S);
    const state = stateText(st);
    found.push({ ...m, size: now.size, sha256: now.sha256, state });
    writeFileSync(join(pictureStore, `${now.sha256}.png`), now.png);
    const newFile = join(pictureShots, `${m.id}-new.png`);
    if (m.pending) {
      writeFileSync(newFile, now.png);
      S.lines.push(`pend  picture ${m.id} (${m.shows}, frame ${m.frame}): not compared, ${m.pending}; now sha256 ${now.sha256.slice(0, 16)}, ${state} (${newFile})`);
      continue;
    }
    if (acceptPictures) {
      S.lines.push(`${m.sha256 === now.sha256 ? "ok  " : "new "}  picture ${m.id} (${m.shows}, frame ${m.frame}): sha256 ${now.sha256.slice(0, 16)}${m.sha256 && m.sha256 !== now.sha256 ? `, was ${m.sha256.slice(0, 16)}` : ""}`);
      continue;
    }
    const same = m.sha256 === now.sha256 && m.size === now.size;
    if (!same) {
      writeFileSync(newFile, now.png);
      const oldFile = join(pictureStore, `${m.sha256}.png`);
      let more = m.sha256 ? `no picture of the reference is kept on this machine (${oldFile}), so only ${m.id}-new.png is written in ${pictureShots}` : `the moment has no reference yet; ${m.id}-new.png is in ${pictureShots}`;
      if (m.sha256 && existsSync(oldFile)) {
        const old = readFileSync(oldFile);
        copyFileSync(oldFile, join(pictureShots, `${m.id}-old.png`));
        const d = await diffOf(S, old, now.png);
        writeFileSync(join(pictureShots, `${m.id}-diff.png`), d.png);
        more = `${d.differ} of ${now.w * now.h} pixels differ; ${m.id}-old.png, -new.png and -diff.png in ${pictureShots}`;
      }
      check(false, `picture ${m.id} (${m.shows}, frame ${m.frame}) is not its reference: sha256 ${now.sha256.slice(0, 16)}, the reference ${(m.sha256 ?? "none").slice(0, 16)} (now ${state}${m.state ? `; the reference's ${m.state}` : ""}); ${more}. If the change is meant: just web-page accept-pictures`);
    } else check(true, `picture ${m.id} (${m.shows}, frame ${m.frame}) is its reference (sha256 ${now.sha256.slice(0, 16)})`);
  }
  pictureResults.set(section, found);
}

// A ROM version's moments, on its route (ROUTES).
async function romPictures(S, { name, sha, rom }) {
  const { p, check } = S;
  await p.goto(`${S.origin}/${entry}`);
  await S.loadRom(rom);
  check(await S.waitFrames(30), `${name}: the ROM version starts`);
  // The references are in the ROM's own art unless a moment names a look.
  const src = (await p.locator("#game").getAttribute("src")).replace(/theme=[^&]*/, "own-look");
  const found = await p.evaluate(async (game) => (await (await import("./game/lift.js")).storedMeta(game))?.found ?? null, page.rom.game);
  const c = ROUTES[found?.template];
  check(!!c, `${name}: the check has a route for its model (${found?.template})`);
  if (!c) return;
  const run = `pictures-${found.template}`;
  S.runs.set(run, framesOf(c.holds, c.frames));
  const runOf = (m) => {
    if (!m.press) return run;
    const name = `pictures-${found.template}-${m.id}`;
    S.runs.set(name, framesOf([...c.holds, ...m.press.split(/\s+/)], c.frames));
    return name;
  };
  await pictureMomentsCheck(S, `pictures-rom-${sha.slice(0, 8)}`, src, runOf);
}

// The sections, in the order their lines are printed. `est` is about how
// long one takes alone (seconds, 2026-11-05), so the longest start first;
// `files` are inputs beside the bundle files its server sends; `rom` is
// the SHA-256 of the ROM it plays.

// ---- The registry: the deep link, the install, and what is refused ----

const LINK = `p?g=${page.game}`;
// The theme the page installs with the game (page.json's `content`), a
// large-art look; --art-theme names another.
const ART_THEME = opt("art-theme") ?? page.content ?? "lex00/turnips";
const installedOf = (p, name = page.game, v = null) => p.evaluate(async ([n, ver]) => (await import("./game/install.js")).installed(n, ver), [name, v]);
const romPlaying = (p) => p.waitForFunction(() => document.getElementById("game").src.includes("files=idb:"), null, { timeout: 60000 }).then(() => true, () => false);

// A first visit by the link: the game comes from the source, whole, and the
// page's own server holds none of it. Another visit asks for no file again.
async function registryLink(S) {
  const { p, check } = S;
  await p.addInitScript(() => {
    window.__persistAsked = 0;
    const was = navigator.storage.persist.bind(navigator.storage);
    navigator.storage.persist = () => (window.__persistAsked++, was());
  });
  await p.goto(`${S.origin}/${LINK}`);
  check(await S.installed(), "by the link the game is installed with no click");
  const rec = await installedOf(p);
  // The package holds no game files the page plays from since 2026-11-11
  // (Rivet Rodeo's went): its play files are the lift's folder and the
  // game's module.
  const has = (path) => rec?.files.some((f) => f.path === path);
  check(rec?.name === page.game && has("play/module.wasm") && has("play/lift/index.json"), `it is installed in this browser (${rec?.name}@${rec?.version}, ${rec?.files.length} files, the module and the lift's folder among them)`);
  const isSource = (r) => isSourceUrl(S.origin, r.url);
  const own = S.requests.filter((r) => r.url.startsWith(S.origin) && !isSourceUrl(S.origin, r.url)).map((r) => r.url.slice(S.origin.length));
  check(!own.some((u) => /\/(bundle|preview)\.json|\/game\/assets\/|\/game\/lift\/(?!gp-web-lift)/.test(u)), `the page's server was asked for none of the game (${own.length} requests)`);
  if (registryStopped) {
    check(!S.requests.some((r) => isSource(r) && r.url.includes("/blobs/")), "with the registry stopped, the game played with no file fetched from it");
    return;
  }
  const blobs = S.requests.filter((r) => isSource(r) && r.url.includes("/blobs/")).length;
  check(blobs > 200, `the game's files came from the source (${blobs} files)`);
  check((await p.evaluate(() => window.__persistAsked)) === 1, "the first install asks once for storage that lasts");
  S.requests.length = 0;
  await p.reload();
  check(await S.installed(), "the link opens again");
  check(!S.requests.some((r) => isSource(r) && r.url.includes("/blobs/")), "a later visit fetches no file again");
  check((await p.evaluate(() => window.__persistAsked)) === 0, "a later visit doesn't ask for storage again");
}

// A person's ROM by the link: it is asked for once, and after that the link
// goes straight into the game.
async function registryRom(S, r) {
  const { p, check, waitFrames } = S;
  await p.goto(`${S.origin}/${LINK}`);
  check(await S.installed(), `${r.name}: the link opens`);
  await S.loadRom(r.rom);
  check(await waitFrames(30), `${r.name}: the ROM plays`);
  await p.goto(`${S.origin}/${LINK}`);
  check(await romPlaying(p), `${r.name}: on a later visit the link goes straight into the game`);
  check(await waitFrames(30), `${r.name}: and it runs`);
  await p.goto(`${S.origin}/index.html`);
  check(await romPlaying(p), `${r.name}: the page with no link plays the kept ROM too`);
  check(await waitFrames(30), `${r.name}: and it runs`);
}

// `c=`: a content package from the source, drawn from its own files over the
// person's ROM.
async function registryTheme(S, r) {
  const { p, check, gp, waitFrames } = S;
  const link = `${LINK}&c=${ART_THEME}`;
  await p.goto(`${S.origin}/${link}`);
  check(await S.installed(), `${r.name}: the link with a theme opens`);
  const c = await installedOf(p, ART_THEME);
  check(c?.kind === "content", `${r.name}: the theme is installed (${c?.name}@${c?.version})`);
  const id = ART_THEME.split("/")[1];
  await S.loadRom(r.rom);
  const frameSrc = await p.locator("#game").getAttribute("src");
  const skipped = (await p.locator("#skipped").isVisible()) ? await p.locator("#skipped").innerText() : "";
  check(frameSrc.includes(`theme=${id}`) && !skipped, `${r.name}: the ROM's game opens in the theme (${frameSrc}${skipped ? `; ${skipped}` : ""})`);
  check(await waitFrames(30), `${r.name}: it runs`);
  check((await gp(() => window.__gp.theme)) === id, `${r.name}: and draws it`);
  const scale = await S.artScale();
  check(scale > 1, `${r.name}: in its large art, drawn by the game's module (${scale} canvas pixels to a game pixel)`);
  await S.shot(`registry-theme-${r.sha.slice(0, 8)}.png`);
}

// A small Wasm module with the loader's calls, whose gp_step is `step`.
function testModule(step, maxPages) {
  const section = (id, body) => [id, body.length, ...body];
  const name = (n) => [n.length, ...Buffer.from(n)];
  const exp = (n, kind, i) => [...name(n), kind, i];
  const fn = (code) => [code.length + 1, 0, ...code];
  const limits = maxPages === null ? [0, 1] : [1, 1, maxPages & 0x7f | 0x80, maxPages >> 7];
  const exports_ = [5, ...exp("memory", 2, 0), ...exp("gp_alloc", 0, 0), ...exp("gp_out_ptr", 0, 1), ...exp("gp_out_len", 0, 2), ...exp("gp_step", 0, 3)];
  const zero = fn([0x41, 0, 0x0b]);
  const code = [4, ...zero, ...zero, ...zero, ...fn(step)];
  return [
    0, 0x61, 0x73, 0x6d, 1, 0, 0, 0,
    ...section(1, [3, 0x60, 1, 0x7f, 1, 0x7f, 0x60, 0, 1, 0x7f, 0x60, 2, 0x7f, 0x7f, 1, 0x7f]),
    ...section(3, [4, 0, 1, 1, 2]),
    ...section(5, [1, ...limits]),
    ...section(7, exports_),
    ...section(10, code),
  ];
}

// The game's own module runs in a Worker, the page's own build has no game's
// hooks, and a module that breaks a limit stops at it with a message.
async function moduleSection(S, withRom) {
  const { p, check, waitFrames, gp } = S;
  await p.goto(`${S.origin}/${LINK}`);
  await S.loadRom(withRom.rom);
  check(await waitFrames(60), "the game runs");
  check((await gp(() => window.__gp.module)) === "game", "it runs the game's own module, not the page's");
  await waitFrames(600, 30000);
  const timing = await gp(() => ({ ...window.__gp.timing }));
  check(timing.frames > 0, `the frames go through the Worker (${timing.frames})`);
  const each = (ms) => (ms / timing.frames).toFixed(2);
  console.log(`measure frame time over ${timing.frames} frames: ${each(timing.worker)} ms in the module, ${each(timing.round)} ms for the page's whole call; worst ${timing.workerMax.toFixed(1)} and ${timing.roundMax.toFixed(1)} ms`);
  check((await gp(() => self.crossOriginIsolated)) === true, "the frame is cross-origin isolated");
  const frame = p.frames().find((f) => f.url().includes("/game/"));
  check(!!frame, "the game's frame is there");
  const loop = testModule([0x03, 0x40, 0x0c, 0, 0x0b, 0x41, 0, 0x0b], 16);
  const grow = testModule([0x03, 0x40, 0x41, 1, 0x40, 0, 0x41, 0x7f, 0x46, 0x04, 0x40, 0, 0x0b, 0x0c, 0, 0x0b, 0x41, 0, 0x0b], 16);
  const r = await frame.evaluate(async ({ loop, grow, noMax, tooBig, game }) => {
    const { ModuleHost } = await import("./module-host.js");
    const refused = (b) => ModuleHost.open(new Uint8Array(b).buffer).then(() => null, (e) => e.message);
    const run = async (b) => {
      let told = null;
      const h = await ModuleHost.open(new Uint8Array(b).buffer, (m) => (told = m));
      const t0 = performance.now();
      let threw = null;
      try {
        h.call("gp_step", [0, 0]);
      } catch (e) {
        threw = e.message;
      }
      let after = null;
      try {
        h.call("gp_step", [0, 0]);
      } catch (e) {
        after = e.message;
      }
      return { told, threw, after, ms: performance.now() - t0 };
    };
    // The page's own build and the game's module, given the bundle the
    // ROM was lifted into.
    const bundle = new Uint8Array((await (await import("./lift.js")).loadStored(game)).get("bundle.json"));
    const load = async (bytes) => {
      const h = await ModuleHost.open(bytes);
      const { r, out } = h.call("gp_load", [bundle]);
      return { r, out: new TextDecoder().decode(out) };
    };
    const pageWasm = await (await fetch("gp_web.wasm")).arrayBuffer();
    const { playFiles } = await import("./install.js");
    const moduleWasm = (await playFiles()).get("module.wasm");
    return {
      loop: await run(loop),
      grow: await run(grow),
      noMax: await refused(noMax),
      tooBig: await refused(tooBig),
      own: await load(pageWasm),
      game: await load(moduleWasm),
    };
  }, { loop, grow, noMax: testModule([0x41, 0, 0x0b], null), tooBig: testModule([0x41, 0, 0x0b], 4097), game: page.rom.game });
  check(/one frame took more than/.test(r.loop.told ?? "") && r.loop.ms < 5000, `a module that loops forever stops at the deadline (${r.loop.told}, ${Math.round(r.loop.ms)} ms)`);
  check(/stopped/.test(r.loop.after ?? ""), "and answers nothing more");
  check(/module failed/.test(r.grow.told ?? ""), `a module that grows without end stops at its limit (${r.grow.told})`);
  check(/sets no memory limit/.test(r.noMax ?? ""), `a module with no memory limit is refused (${r.noMax})`);
  check(/allows 256 MB/.test(r.tooBig ?? ""), `a module asking for more than 256 MB is refused (${r.tooBig})`);
  check(r.own.r !== 0 && /hook/i.test(r.own.out), `the page's own build has no game's hooks: it refuses the game's model (${r.own.out.slice(0, 120)})`);
  check(r.game.r === 0 && r.game.out.length > 0, `the game's module runs the same model with its hooks (${r.game.out})`);
}

async function registryLinks(S) {
  const { p, check } = S;
  const refused = async (link, re, what) => {
    await p.goto(`${S.origin}/${link}`);
    await p.waitForFunction(() => document.getElementById("message").dataset.kind === "error", null, { timeout: 30000 }).catch(() => {});
    const said = await p.locator("#message").innerText();
    check(re.test(said) && !(await p.locator("#game").getAttribute("src")), `${what} (${said})`);
  };
  await refused("p?g=someone/else", /isn't in this source's catalog/i, "a link to a game the source doesn't have is refused in words");
  await refused(`p?g=${page.game}@9.9.9`, /couldn't be installed/i, "a link to a version that doesn't exist is refused in words");
  // Neither registry holds 1.0.0.
  await refused(`p?g=${page.game}@1.0.0`, /isn't there/i, "a version this source doesn't hold is refused in words");
}

// A request to a source on the network answered from --registry-folder, the
// file at its key (`change` applied) or 403 as a repo host answers a file it
// doesn't have.
function fromFolder(route, change = (b) => b) {
  const url = new URL(route.request().url());
  const f = join(registryFolder, sourceKey(url.origin, url));
  if (!existsSync(f)) return route.fulfill({ status: 403, headers: { "access-control-allow-origin": "*" }, body: "" });
  return route.fulfill({ status: 200, headers: { "access-control-allow-origin": "*" }, body: change(readFileSync(f)) });
}

// What a source can do wrong, each on a first visit. `r` fetches the real
// answer (from --registry-folder when the source is on the network and the
// run has one, so the case changes the run's copy, not the live source);
// each case changes it, and nothing may be installed.
const broken = (match, change) => async (route) => {
  const url = route.request().url();
  if (!match(url)) return route.fallback();
  try {
    if (registryFolder && page.source.url !== ".") return await fromFolder(route, change);
    const real = await route.fetch();
    await route.fulfill({ response: real, body: change(await real.body()) });
  } catch {
    // The section ended, and its page with it, while this file was on its way.
  }
};
const flip = (b) => {
  const c = Buffer.from(b);
  c[c.length >> 1] ^= 1;
  return c;
};
const source_ = (k) => (u) => new URL(u).pathname.endsWith(`/${k}`);
const refusedInstall = (re, what, { version } = {}) => async (S) => {
  const { p, check } = S;
  await p.goto(`${S.origin}/${LINK}${version ? `@${version}` : ""}`);
  await p.waitForFunction(() => document.getElementById("message").dataset.kind === "error", null, { timeout: 60000 }).catch(() => {});
  const said = await p.locator("#message").innerText();
  check(re.test(said), `${what}: the page says so (${said})`);
  check(!(await p.locator("#game").getAttribute("src")), `${what}: no game starts`);
  check((await installedOf(p)) === null, `${what}: nothing is installed`);
};

// Installed, then the source goes bad: what is installed keeps playing and
// the page says why it isn't refreshed.
async function registryGoesBad(S) {
  const { p, check } = S;
  await p.goto(`${S.origin}/${LINK}`);
  check(await S.installed(), "installed");
  await p.context().route((u) => isSourceUrl(S.origin, u) && sourceKey(S.origin, u) === "/registry.toml", broken(() => true, flip));
  await p.goto(`${S.origin}/${LINK}`);
  check(await S.installed(), "with the source's root signature broken, what is installed is used");
  const said = await p.locator("#message").innerText();
  check(/Using the copy in this browser/.test(said), `and the page says so (${said})`);
}

// A file changed in this browser's own storage is caught before it is used.
async function registryLocalChange(S) {
  const { p, check } = S;
  await p.goto(`${S.origin}/${LINK}`);
  check(await S.installed(), "installed");
  await p.evaluate(async () => {
    const db = await new Promise((ok) => (indexedDB.open("gp-installs").onsuccess = (e) => ok(e.target.result)));
    // The game's own record, the one under `current`: the store holds a
    // record for each theme installed with it too, and one from a namespace
    // that sorts before the game's (acme/ before lex00/) comes first.
    const games = db.transaction("games").objectStore("games");
    const get = (k) => new Promise((ok) => (games.get(k).onsuccess = (e) => ok(e.target.result)));
    const rec = await get(await get("current"));
    const f = rec.files.find((x) => x.path === "play/lift/index.json");
    const tx = db.transaction("blobs", "readwrite");
    tx.objectStore("blobs").put(new TextEncoder().encode("{}").buffer, f.hash);
    await new Promise((ok) => (tx.oncomplete = ok));
    db.close();
  });
  await p.goto(`${S.origin}/${LINK}`);
  await p.waitForFunction(() => document.getElementById("message").dataset.kind === "error", null, { timeout: 30000 }).catch(() => {});
  const said = await p.locator("#message").innerText();
  check(/changed in this browser/.test(said), `a play file changed in this browser is refused (${said})`);
}

// ---- Any game by its name, and the share link (docs/web.md, "One page, any game") ----

// A made-up second game in the registry folder (scripts/web/test-registry.sh
// puts test/made-up and its theme test/made-up-look there): its sections run
// only when --registry-folder holds it.
const MADE_UP = "test/made-up";
const MADE_UP_LOOK = "test/made-up-look";
const madeUpHere = !!registryFolder && existsSync(join(registryFolder, "index", MADE_UP));
// The made-up game's manifest id and a blob of its, from the folder.
function madeUpFiles() {
  const cat = readFileSync(join(registryFolder, "catalog.toml"), "utf8");
  const id = new RegExp(`name = "${MADE_UP}"[^\\[]*?id = "([0-9a-f]{64})"`).exec(cat)[1];
  const hashes = new Map();
  for (const l of readFileSync(join(registryFolder, "packages", id), "utf8").split("\n")) {
    const m = /^file ([0-9a-f]{64}) \d+ (.+)$/.exec(l);
    if (m) hashes.set(m[2], m[1]);
  }
  return { id, hashes };
}
// The page's message once it says something of `kinds`, at most `ms`.
const messageOf = async (p, kinds = ["error"], ms = 60000) => {
  await p.waitForFunction((k) => k.includes(document.getElementById("message").dataset.kind), kinds, { timeout: ms }).catch(() => {});
  return p.locator("#message").innerText();
};
const madeUpRom = (S) => {
  const fake = S.file("made-up-game.nes");
  const body = Buffer.alloc(16 + 16384 + 8192);
  body.set([0x4e, 0x45, 0x53, 0x1a, 1, 1]);
  for (let i = 16; i < body.length; i++) body[i] = (i * 104729) & 0xff;
  writeFileSync(fake, body);
  return fake;
};

// The page's own game by its registry name, and a second game by its name:
// each is installed from the source, and the page takes its title, its ROM
// line, its how-to-play lines and its About line from the game, as text.
async function gameByName(S) {
  const { p, check, errors } = S;
  await p.goto(`${S.origin}/p?g=${page.game}`);
  check(await S.installed(), `${page.game} by its name is installed`);
  check((await p.title()) === page.title && (await p.locator("#rom-help").innerText()).includes(page.rom.name), `and the page is its own: ${page.title}, the ROM line naming ${page.rom.name}`);
  check(/arrows move, X jumps, Enter starts/.test(await p.locator("#howto").innerText()), "with the page's own how-to-play lines");
  if (!madeUpHere) {
    S.lines.push(`skip  no ${MADE_UP} in the registry folder (scripts/web/test-registry.sh makes one): a second game isn't checked`);
    return;
  }
  await p.goto(`${S.origin}/p?g=${MADE_UP}`);
  check(await S.installed(), `${MADE_UP} by its name is installed with no click`);
  const rec = await installedOf(p, MADE_UP);
  check(rec?.kind === "game" && rec.files.some((f) => f.path === "play/page.json"), `from its package (${rec?.name}@${rec?.version}, ${rec?.files.length} files, play/page.json among them)`);
  check((await p.title()) === "Made Up Game" && (await p.locator("#title").innerText()) === "Made Up Game", `the page is titled for it (${await p.title()})`);
  const help = await p.locator("#rom-help").innerText();
  check(help.includes("Made Up Game") && help.includes("(A) or (B)") && !/Donkey Kong/.test(help), `the ROM line is its package's (${help.slice(0, 90)}...)`);
  const howto = p.locator("#howto");
  check(/Made-up controls: the page check reads this line/.test(await howto.innerText()), "the how-to-play lines are its package's");
  check((await howto.innerText()).includes("<b>Not HTML</b>") && (await howto.locator("b").count()) === 0, "and its text is shown as text, never read as HTML");
  check((await p.locator("#about-mark").textContent()) === "Made Up Game is made up for the page check." && !/Nintendo|Donkey/.test(await p.locator("#about").textContent()), "the About lines are its own and name no other game");
  check((await p.evaluate(() => window.__page !== undefined)) && !(await p.locator("#game").getAttribute("src")), "nothing plays until a ROM is loaded");
  await S.shot("game-made-up.png");
  // A ROM that isn't its: refused by the lift, naming the game.
  await p.setInputFiles("#rom", madeUpRom(S));
  const said = await messageOf(p);
  check(/Made Up Game/.test(said) && !(await p.locator("#game").getAttribute("src")), `a ROM that isn't its is refused naming it, and no game starts (${said.slice(0, 120)})`);
  // Its theme by the link, and one offered for another game refused.
  await p.goto(`${S.origin}/p?g=${MADE_UP}&c=${MADE_UP_LOOK}`);
  check(await S.installed(), "with its own theme the link opens");
  check((await installedOf(p, MADE_UP_LOOK))?.kind === "content", `and the theme is installed (${MADE_UP_LOOK})`);
  await p.goto(`${S.origin}/p?g=${MADE_UP}&c=${ART_THEME}`);
  check(await S.installed(), "with another game's theme the link still opens");
  const warned = await messageOf(p, ["warn", "error"], 10000);
  check(new RegExp(`${ART_THEME}.*isn't a theme the source offers for Made Up Game`).test(warned), `and that theme is left out in words (${warned})`);
  // The page's own game, after another: its own facts again.
  await p.goto(`${S.origin}/${entry}`);
  check(await S.installed(), "the plain address installs the page's own game again");
  check((await p.title()) === page.title && /arrows move, X jumps/.test(await p.locator("#howto").innerText()), "with its own title and how-to-play lines");
  check(errors.length === 0, `no page errors (${errors.join("; ")})`);
}

// Names that aren't games, or aren't names: each refused in words before
// anything of them is fetched, or left out with the page's own game playing.
async function gameBadName(S) {
  const { p, check } = S;
  const refused = async (link, re, what) => {
    await p.goto(`${S.origin}/${link}`);
    const said = await messageOf(p, ["error"], 30000);
    check(re.test(said) && !(await p.locator("#game").getAttribute("src")), `${what} (${said})`);
  };
  await refused("p?g=nobody/nothing", /nobody\/nothing couldn't be installed: .*isn't in this source's catalog/, "a game the source doesn't have is refused in words");
  await refused(`p?g=${ART_THEME}`, /isn't a game on this page's source/, "a theme named as the game is refused in words");
  const before = S.requests.length;
  for (const bad of ["p?g=../../registry", "p?g=lex00/%3Cimg%20src%3Dx%3E", "p?g=lex00/donkey-kong@latest", "p?g=a/b&g=c/d"]) {
    await p.goto(`${S.origin}/${bad}`);
    check(await S.installed(), `${decodeURIComponent(bad)}: not a name, so the page's own game plays`);
    const said = await messageOf(p, ["warn"], 10000);
    check(/The link's g wasn't understood and was left out/.test(said), `and says the link's g was left out (${said})`);
    check((await p.title()) === page.title && (await p.locator("img").count()) === 0, "with its own title, and nothing of the address made into the page");
  }
  const asked = S.requests.slice(before).filter((r) => isSourceUrl(S.origin, r.url)).map((r) => r.url);
  check(!asked.some((u) => /\.\.|%3C|<|\/c\/d|@latest/i.test(u)), `no request named what the address said (${asked.length} requests to the source)`);
  await p.goto(`${S.origin}/p?g=${page.game}&mode=turbo&on=smooth_walk,not_a_switch&rom=x`);
  check(await S.installed(), "a link with settings the game hasn't opens");
  const said = await messageOf(p, ["warn"], 10000);
  check(/link's an unknown key wasn't understood/.test(said) && /no setting not_a_switch, turbo/.test(said), `and names the settings and keys it left out (${said})`);
}

// A tampered package of the second game: a changed file, or a changed
// manifest (its content id no longer its hash), and nothing is installed.
async function gameTampered(S, which) {
  const { p, check } = S;
  await p.goto(`${S.origin}/p?g=${MADE_UP}`);
  const said = await messageOf(p, ["error"], 60000);
  const re = which === "blob" ? /doesn't match what its maker signed/ : /couldn't be installed/;
  check(re.test(said), `${which === "blob" ? "its play/page.json's bytes changed" : "its manifest changed"}: the page says so (${said})`);
  check(!(await p.locator("#game").getAttribute("src")), "no game starts");
  check((await installedOf(p, MADE_UP)) === null, "nothing is installed");
}

// The share link: made from what plays, read back, and opened in a fresh
// browser, where it asks for the ROM and then starts in those choices. It
// carries the game, the theme and the settings and nothing else.
async function shareSection(S, withRom) {
  const { p, check, gp } = S;
  await S.context.grantPermissions(["clipboard-read", "clipboard-write"]).catch(() => {});
  const shared = async () => {
    await p.evaluate(() => delete document.getElementById("message").dataset.share);
    await p.locator("#share").click();
    await p.waitForFunction(() => document.getElementById("message").dataset.share, null, { timeout: 30000 }).catch(() => {});
    return p.evaluate(() => document.getElementById("message").dataset.share ?? null);
  };
  const keys = (u) => [...new URL(u).searchParams.keys()];
  await p.goto(`${S.origin}/p?g=${page.game}&c=${ART_THEME}&on=smooth_walk,not_a_switch&evil=1`);
  check(await S.installed(), "a link with a theme and switches opens");
  check(await p.locator("#share").isVisible(), "the Share button is there");
  let link = await shared();
  const q = link ? new URL(link).searchParams : new URLSearchParams();
  check(link?.startsWith(`${S.origin}/p?`) && q.get("g") === page.game && q.get("c") === ART_THEME && q.get("on") === "smooth_walk" && keys(link).length === 3, `with no ROM, the link is the game, the theme and the switches the game has (${link})`);
  const clip = await p.evaluate(() => navigator.clipboard.readText()).catch(() => null);
  check(clip === null || clip === link, `and it is what was copied (${clip === null ? "the clipboard can't be read here" : "the same"})`);
  if (madeUpHere) {
    await p.goto(`${S.origin}/p?g=${MADE_UP}&c=${MADE_UP_LOOK}&mode=retro`);
    check(await S.installed(), `${MADE_UP} with its theme and a mode opens`);
    link = await shared();
    check(link === `${S.origin}/p?g=${MADE_UP}&c=${MADE_UP_LOOK}&mode=retro`, `its share link reads back the same (${link})`);
  }
  if (!withRom) {
    S.lines.push("skip  no ROM in $GP_ROM_DIR: the share link isn't played");
    return;
  }
  // Played: the settings and theme the game runs in go in the link.
  await p.goto(`${S.origin}/p?g=${page.game}&c=${ART_THEME}&mode=normal`);
  await S.loadRom(withRom.rom);
  check(await S.waitFrames(30), "the ROM plays from the link");
  check((await gp(() => window.__gp.profile)) === "normal" && (await gp(() => window.__gp.theme)) === ART_THEME.split("/")[1], "in the link's mode and theme");
  link = await shared();
  check(keys(link).join(",") === "g,c,mode" && new URL(link).searchParams.get("mode") === "normal", `the share link carries them (${link})`);
  check(!/rom|save|sha|idb|blob/i.test(new URL(link).search), "and nothing of the ROM, the saves or the player");
  // A fresh browser: nothing kept, so the ROM is asked for, then the game
  // starts in the shared choices.
  const fresh = await p.context().browser().newContext({ viewport: { width: 1366, height: 768 } });
  try {
    fresh.on("request", (r) => S.requests.push({ method: r.method(), url: r.url(), body: r.postDataBuffer()?.length ?? 0 }));
    if (registryFolder && page.source.url !== ".") await fresh.route((u) => isSourceUrl(S.origin, u), (r) => fromFolder(r));
    const q2 = await fresh.newPage();
    await q2.goto(link);
    await q2.waitForFunction(() => window.__page !== undefined, null, { timeout: 60000 });
    check((await q2.locator("#load-label").isVisible()) && !(await q2.locator("#game").getAttribute("src")), "the shared link in a fresh browser asks for the ROM");
    await q2.setInputFiles("#rom", withRom.rom);
    await q2.waitForFunction(() => document.getElementById("game").src.includes("files=idb:"), null, { timeout: 60000 });
    const fr = () => q2.frame({ url: /\/game\/(index\.html)?(\?|$)/ });
    const t0 = Date.now();
    let got = null;
    while (Date.now() - t0 < 30000) {
      got = await fr()?.evaluate(() => (window.__gp?.hashes?.length > 30 ? { profile: window.__gp.profile, theme: window.__gp.theme } : null)).catch(() => null);
      if (got) break;
      await q2.waitForTimeout(200);
    }
    check(got?.profile === "normal" && got?.theme === ART_THEME.split("/")[1], `then starts in the shared mode and theme (${JSON.stringify(got)})`);
  } finally {
    await fresh.close();
  }
}

// Themes the source lists that the page leaves out are named with why
// (gp_web_registry::choose; page.js notOffered): in About from the start,
// in the line under the header while a ROM plays, and the details in the
// log. The game's list gets three made-up lines here, which the page
// leaves out before it installs them: one whose name can't name a theme
// (`zz/8bit`), one withdrawn for safety (yanks.toml gets its line too), and
// one whose name ends as the first listed theme's does. A link to a theme
// left out says why. And a theme drawn in part is named too (a made-up
// looks_partial, since a real one needs a theme that names a picture the
// game hasn't got: gp-web-lift's own test makes one). Kept apart from the
// theme-picks sections. Only on an unsigned source: a signed list changed
// here would be refused whole.
const madeUpId = (n) => createHash("sha256").update(n).digest("hex");
let firstListed = null;
const notOfferedSource = async (route) => {
  const url = new URL(route.request().url());
  const key = sourceKey(url.origin, url);
  const byGame = `/by-game/${page.game}.toml`;
  if (key !== byGame && key !== "/yanks.toml") return route.fallback();
  try {
    let body;
    if (registryFolder && page.source.url !== ".") {
      const f = join(registryFolder, key);
      body = existsSync(f) ? readFileSync(f, "utf8") : null;
    } else {
      const real = await route.fetch();
      body = real.ok() ? (await real.body()).toString() : null;
    }
    if (body === null) return route.fulfill({ status: 403, headers: { "access-control-allow-origin": "*" }, body: "" });
    if (key === byGame) {
      const m = /^package = "([^"]+)"[\s\S]*?^title = "([^"]*)"/m.exec(body);
      firstListed = m ? { name: m[1], title: m[2] } : null;
      const line = (pkg, title) => `\n[[line]]\npackage = "${pkg}"\nversion = "1.0.0"\nid = "${madeUpId(pkg)}"\ntitle = "${title}"\n`;
      if (firstListed) body += line(`zz/${firstListed.name.split("/")[1]}`, "A Copy");
      body += line("zz/8bit", "Eight Bit") + line("zz/unsafe", "Unsafe");
    } else {
      body = body.replace(/^yank = \[\]\n?/m, "");
      body += `\n[[yank]]\nname = "zz/unsafe"\nversion = "1.0.0"\nid = "${madeUpId("zz/unsafe")}"\nreason = "made up for the page check"\nseverity = "security"\n`;
    }
    await route.fulfill({ status: 200, headers: { "access-control-allow-origin": "*" }, body });
  } catch {
    // The section ended while this file was on its way.
  }
};
async function themesNotOffered(S) {
  const { p, check } = S;
  const logged = [];
  p.on("console", (m) => logged.push(m.text()));
  await p.goto(`${S.origin}/${LINK}`);
  check(await S.installed(), "installed, with three made-up lines on the game's theme list");
  await p.evaluate(() => window.__page.installing);
  const about = p.locator("#about-themes");
  const said = (await about.textContent()) ?? "";
  const parts = ["Eight Bit, because its name isn't one this page can list", "Unsafe, because it was withdrawn for safety: made up for the page check"];
  if (firstListed) parts.push(`A Copy, because another theme, ${firstListed.title}, has the same name here, and Settings, Theme lists one of each name`);
  check(said.startsWith("Themes not offered: ") && parts.every((x) => said.includes(x)), `About names each theme left out with why: "${said}"`);
  check(!(await about.isVisible()), "in About, shut until it is opened");
  await p.locator("#about-box summary").click();
  check(await about.isVisible(), "and there when About is opened");
  check(!(await p.locator("#skipped").isVisible()), "with no ROM, the line under the header stays hidden");
  check(
    logged.some((l) => l.startsWith("gp: theme zz/8bit not offered: its name isn't one this page can list (zz/8bit: the page names a theme by the part of its package name after the namespace")),
    `the log has the detail (${logged.filter((l) => l.startsWith("gp: theme")).join(" | ")})`,
  );
  const settings = await p.evaluate(() => window.__page.installing.then((x) => x.themes.map((t) => t.name)));
  check(!settings.some((n) => n.startsWith("zz/")), `none of them is installed or offered (${settings.join(", ")})`);
  // While a ROM plays: the same in the line under the header, with a
  // theme drawn in part (looks_partial's shape) after it.
  await p.evaluate(() =>
    window.__page.showSkipped(
      [],
      [{ id: "gappy", title: "Gappy", reason: "some of its pictures are for parts of the game this version hasn't got, so they aren't drawn", detail: "art.txt names frame-999-9.png this build hasn't got" }],
    ),
  );
  const line = (await p.locator("#skipped").innerText()) ?? "";
  check(
    (await p.locator("#skipped").isVisible()) && line.startsWith("Themes not offered: ") && line.endsWith("Theme drawn in part: Gappy, because some of its pictures are for parts of the game this version hasn't got, so they aren't drawn."),
    `the line under the header names them, and a theme drawn in part: "${line}"`,
  );
  check(!/frame-|\.txt|zz\//.test(line), "in plain words, with no file or package names");
  await S.shot("themes-not-offered.png");
  // A link to a theme left out says why, and the game plays in its own look.
  await p.goto(`${S.origin}/${LINK}&c=zz/8bit`);
  await S.installed();
  await p.evaluate(() => window.__page.installing.catch(() => {}));
  const msg = await p.locator("#message").innerText();
  check(msg.includes("zz/8bit isn't offered for ") && msg.includes(", because its name isn't one this page can list. Playing in the game's own look."), `a link to one says why: "${msg}"`);
  check(S.errors.length === 0, `no page errors (${S.errors.join("; ")})`);
}

// ---- A theme's HUD drawn over the bodies (gp_present::art, `#+ hud_over`) ----

// Moss Hop (the project's own game, from its own ROM) with its HUD's
// counters in the background and two made-up large-art themes, as
// scripts/web/mosshop-hud.sh writes it (--mosshop DIR, default DIST.mosshop).
// Its files are served beside the page's own player files (game/index.html,
// preview.js, gp_web.wasm), so the page's own code draws the frame: `tall`
// draws the hero 200 pixels tall, standing across the HUD's row, and
// `tallover` is the same theme with the line `#+ hud_over`. With the line
// the HUD's text is painted over the hero, where without it the hero's
// picture covers it, and nothing else in the frame differs.
const mosshopDir = resolve(opt("mosshop") ?? `${dist}.mosshop`);
const mosshopHere = existsSync(join(mosshopDir, "bundle.json")) && existsSync(join(mosshopDir, "themes", "tallover", "art.txt"));
// The made-up Moss Hop build's files, served beside the page's own player
// files; the page's own files, and the runs a section plays, come from its
// server.
async function serveMosshop(S) {
  const own = (u) => existsSync(join(dist, decodeURIComponent(u.pathname))) || /^\/game\/runs\/.+\.frames$/.test(u.pathname);
  await S.context.route(
    (u) => u.origin === S.origin && u.pathname.startsWith("/game/") && !own(u),
    (route) => {
      const path = new URL(route.request().url()).pathname;
      const f = join(mosshopDir, decodeURIComponent(path.slice("/game/".length)));
      if (!f.startsWith(mosshopDir) || !existsSync(f) || statSync(f).isDirectory()) return route.fulfill({ status: 404, body: "" });
      const headers = Object.fromEntries(headersFor(path));
      headers["content-type"] = types[extname(f)] ?? "application/octet-stream";
      return route.fulfill({ status: 200, headers, body: readFileSync(f) });
    },
  );
}
async function themeHudOver(S) {
  const { p, check } = S;
  await serveMosshop(S);
  // The game in a theme, held 60 frames in (a run of no buttons, which
  // stops there): the canvas's pixels.
  S.runs.set("hud-over", framesOf([], 60));
  const drawn = async (theme) => {
    await p.goto(`${S.origin}/game/index.html?files=here&run=hud-over&theme=${theme}`);
    check(await S.waitFrames(61), `Moss Hop plays 60 frames in the made-up theme ${theme}`);
    await p.waitForTimeout(300);
    check((await S.gp(() => window.__gp.theme ?? null).catch(() => null)) === theme, `and is drawn in it (${theme})`);
    await S.shot(`theme-hud-over-${theme}.png`);
    const got = await S.gp(() => {
      const c = document.getElementById("game");
      const d = c.getContext("2d").getImageData(0, 0, c.width, c.height).data;
      let bin = "";
      for (let i = 0; i < d.length; i += 0x8000) bin += String.fromCharCode(...d.subarray(i, i + 0x8000));
      return { w: c.width, h: c.height, rgba: btoa(bin) };
    });
    return { ...got, rgba: Buffer.from(got.rgba, "base64") };
  };
  const under = await drawn("tall");
  const over = await drawn("tallover");
  check(under.w === over.w && under.h === over.h, `the two themes draw the same canvas (${under.w} by ${under.h})`);
  const hero = [200, 40, 160];
  const at = (img, x, y) => [...img.rgba.subarray((y * img.w + x) * 4, (y * img.w + x) * 4 + 3)];
  const isHero = (img, x, y) => at(img, x, y).every((v, i) => Math.abs(v - hero[i]) <= 2);
  // Where the pictures differ: the HUD's text, painted over the hero.
  const diff = [];
  for (let y = 0; y < under.h; y++) for (let x = 0; x < under.w; x++) if (!at(under, x, y).every((v, i) => v === at(over, x, y)[i])) diff.push([x, y]);
  const box = diff.reduce((b, [x, y]) => [Math.min(b[0], x), Math.min(b[1], y), Math.max(b[2], x + 1), Math.max(b[3], y + 1)], [Infinity, Infinity, -Infinity, -Infinity]);
  check(diff.length > 0, `with #+ hud_over the frame differs where the hero stands across the HUD's row (${diff.length} canvas pixels, in x ${box[0]} to ${box[2]}, y ${box[1]} to ${box[3]})`);
  if (diff.length) {
    check(diff.every(([x, y]) => isHero(under, x, y)), "without it, the hero's picture covers every one of those pixels (the HUD's text can't be read)");
    check(diff.every(([x, y]) => !isHero(over, x, y)), "with it, the HUD's text is painted there instead, over the hero (readable)");
    // The hero's picture spans the HUD's text box: the text is within it.
    const heroCols = [];
    for (let x = 0; x < under.w; x++) if (isHero(under, x, box[1])) heroCols.push(x);
    check(box[0] >= heroCols[0] && box[2] <= heroCols.at(-1) + 1, `and all of it is inside the hero's picture (its columns ${heroCols[0]} to ${heroCols.at(-1) + 1} on that row)`);
  }
  check(S.errors.length === 0, `no page errors (${S.errors.join("; ")})`);
}

// ---- A theme's picture in a region of the stage (gp_present::art, `#+ in REGION`) ----

// Moss Hop's level kit names one region, `start` (where the hero starts:
// templates/mosshop/level-kit/rules.toml), which gp-web-prep writes into
// preview.json and preview.js gives the module (gp_set_regions). The
// made-up theme `instart` (scripts/web/mosshop-hud.sh) draws the standing
// hero 200 pixels tall in one colour, and in `start` 16 pixels tall in
// another (`#+ in start pic ...`): standing at the start the short picture
// is drawn and the tall one isn't; walked right out of it and standing
// again, the tall one is and the short one isn't.
const mosshopRegionHere = mosshopHere && existsSync(join(mosshopDir, "themes", "instart", "art.txt"));
async function themeInRegion(S) {
  const { p, check } = S;
  await serveMosshop(S);
  const cfg = JSON.parse(readFileSync(join(mosshopDir, "preview.json"), "utf8"));
  check(JSON.stringify(cfg.regions) === JSON.stringify([["start", "level-main.png", 0, 160, 64, 32]]), `the build's preview.json gives Moss Hop's one region from its level kit (${JSON.stringify(cfg.regions)})`);
  S.runs.set("in-start", framesOf([], 60));
  S.runs.set("off-start", framesOf(["right@0-40"], 120));
  const tall = [200, 40, 160];
  const short = [40, 200, 220];
  const counts = async (run, frames) => {
    await p.goto(`${S.origin}/game/index.html?files=here&run=${run}&theme=instart`);
    check(await S.waitFrames(frames + 1), `Moss Hop plays the run ${run} (${frames} frames) in the made-up theme instart`);
    await p.waitForTimeout(300);
    check((await S.gp(() => window.__gp.theme ?? null).catch(() => null)) === "instart" && (await S.gp(() => window.__gp.regions ?? null)) === true, "drawn in it, the module given the regions");
    await S.shot(`theme-in-region-${run}.png`);
    return S.gp(
      ([a, b]) => {
        const c = document.getElementById("game");
        const d = c.getContext("2d").getImageData(0, 0, c.width, c.height).data;
        const near = (i, k) => k.every((v, j) => Math.abs(d[i + j] - v) <= 2);
        let n = [0, 0];
        for (let i = 0; i < d.length; i += 4) {
          if (near(i, a)) n[0]++;
          else if (near(i, b)) n[1]++;
        }
        return { tall: n[0], short: n[1] };
      },
      [tall, short],
    );
  };
  const at = await counts("in-start", 60);
  check(at.short > 0 && at.tall === 0, `standing at the start, in the region, the hero is drawn in the region's shorter picture only (${at.short} pixels of it, ${at.tall} of the tall one)`);
  const off = await counts("off-start", 120);
  check(off.tall > 0 && off.short === 0, `walked right out of it, the hero is drawn in its full picture only (${off.tall} pixels of it, ${off.short} of the region's)`);
  check(S.errors.length === 0, `no page errors (${S.errors.join("; ")})`);
}

// ---- A sound role's takes (issue #30) ----

// Moss Hop with a made-up sound role, `tick`, cued every 40 frames, and a
// made-up large-art theme `takes` that gives it three files
// (sound/roles/tick.wav, sound/takes/tick.2.wav and tick.3.wav), as
// scripts/web/mosshop-takes.sh writes them (--mosshop-takes DIR, default
// DIST.mosshop-takes), served beside the page's own player files. A run of
// 400 frames in the theme: the page reads all three, each start of the role
// plays the next take (1, 2, 3, 1, ...), and the same run played again
// starts the same takes at the same frames.
const takesDir = resolve(opt("mosshop-takes") ?? `${dist}.mosshop-takes`);
const takesHere = existsSync(join(takesDir, "bundle.json")) && existsSync(join(takesDir, "themes", "takes", "look.txt"));
async function themeSoundTakes(S) {
  const { p, check } = S;
  const own = (u) => existsSync(join(dist, decodeURIComponent(u.pathname))) || /^\/game\/runs\/.+\.frames$/.test(u.pathname);
  await S.context.route(
    (u) => u.origin === S.origin && u.pathname.startsWith("/game/") && !own(u),
    (route) => {
      const path = new URL(route.request().url()).pathname;
      const f = join(takesDir, decodeURIComponent(path.slice("/game/".length)));
      if (!f.startsWith(takesDir) || !existsSync(f) || statSync(f).isDirectory()) return route.fulfill({ status: 404, body: "" });
      const headers = Object.fromEntries(headersFor(path));
      headers["content-type"] = types[extname(f)] ?? "application/octet-stream";
      return route.fulfill({ status: 200, headers, body: readFileSync(f) });
    },
  );
  S.runs.set("takes", framesOf([], 400));
  const files = ["sound/roles/tick.wav", "sound/takes/tick.2.wav", "sound/takes/tick.3.wav"];
  const played = async (n) => {
    const before = S.requests.length;
    await p.goto(`${S.origin}/game/index.html?files=here&run=takes&theme=takes`);
    check(await S.waitFrames(401, 40000), `run ${n}: Moss Hop plays the 400 frames in the made-up theme`);
    await p.waitForTimeout(300);
    const asked = S.requests.slice(before).map((r) => new URL(r.url).pathname);
    const log = await S.gp(() => window.__gp.soundRoleLog?.() ?? null).catch(() => null);
    return { log, asked };
  };
  const one = await played(1);
  check(files.every((f) => one.asked.includes(`/game/themes/takes/${f}`)), `the page read the role's three files (${files.join(", ")})`);
  const log = one.log ?? [];
  check(one.log !== null, "the page's module says which take each start played (gp_sound_role_log)");
  const takes = log.map(([, , k]) => k);
  check(log.length >= 9 && log.every(([, r]) => r === "tick"), `the role started ${log.length} times in the run (every 40 frames)`);
  check(
    [1, 2, 3].every((k) => takes.includes(k)) && takes.every((k, i) => k === (i % 3) + 1),
    `each start played the next take, all three over the run (${takes.join(" ")})`,
  );
  const gaps = log.slice(1).map(([f], i) => f - log[i][0]);
  check(gaps.length > 0 && gaps.every((g) => g === 40), `40 frames apart (${[...new Set(gaps)].join(", ")})`);
  const two = await played(2);
  const same = JSON.stringify(two.log) === JSON.stringify(one.log);
  check(same, `played again, the same run starts the same takes at the same frames (${(two.log ?? []).map(([f, , k]) => `${f}:${k}`).slice(0, 6).join(" ")} ...)`);
  check(S.errors.length === 0, `no page errors (${S.errors.join("; ")})`);
}

// ---- A game with its own pictures, by its name: no ROM (docs/web.md, "A game with its own pictures") ----

// A made-up game that plays from its own files (scripts/web/test-registry.sh
// puts test/own-hop there: a copy of the Moss Hop template, built by
// build-page.sh's road for such a game): its sections run only when
// --registry-folder holds it.
const OWN = "test/own-hop";
const ownHere = !!registryFolder && existsSync(join(registryFolder, "index", OWN));
function ownFiles() {
  const cat = readFileSync(join(registryFolder, "catalog.toml"), "utf8");
  const id = new RegExp(`name = "${OWN}"[^\\[]*?id = "([0-9a-f]{64})"`).exec(cat)[1];
  const hashes = new Map();
  for (const l of readFileSync(join(registryFolder, "packages", id), "utf8").split("\n")) {
    const m = /^file ([0-9a-f]{64}) \d+ (.+)$/.exec(l);
    if (m) hashes.set(m[2], m[1]);
  }
  return { id, hashes };
}
// The lift's files: a game that plays from its own files fetches none.
const liftFetched = (S) => S.requests.some((r) => /\/game\/lift(\.js|\/)/.test(r.url));

// It installs by its name and plays at once, with no ROM asked for and
// nothing lifted: the frame reads the package's own play/ files. The page
// says what the game is and who made it from the package's page facts, as
// text, and its share link reads back.
async function ownGame(S) {
  const { p, check, errors } = S;
  await S.context.grantPermissions(["clipboard-read", "clipboard-write"]).catch(() => {});
  const installed = async () => {
    await p.waitForFunction(() => window.__page !== undefined, null, { timeout: 60000 }).catch(() => {});
    return (await installedOf(p, OWN)) !== null;
  };
  await p.goto(`${S.origin}/p?g=${OWN}`);
  check(await installed(), `${OWN} by its name is installed with no click`);
  const rec = await installedOf(p, OWN);
  const paths = rec?.files.map((f) => f.path) ?? [];
  check(
    rec?.kind === "game" && ["play/page.json", "play/bundle.json", "play/preview.json"].every((f) => paths.includes(f)) && !paths.some((f) => f.startsWith("play/lift/")),
    `from its package (${rec?.name}@${rec?.version}, ${paths.length} files: play/page.json, bundle.json and preview.json, and no lift)`,
  );
  const frameSrc = async () => {
    await p.waitForFunction(() => document.getElementById("game").getAttribute("src"), null, { timeout: 30000 }).catch(() => {});
    return p.locator("#game").getAttribute("src");
  };
  const src = await frameSrc();
  check(!!src && !/files=/.test(src), `it plays at once, the frame reading the installed package (${src})`);
  check(await S.waitFrames(60), "the game runs (60 frames)");
  // Its own large-art theme (gp game share puts a person's in): in the
  // package's play/themes and offered in Settings, the game opening in its
  // own pictures with no look asked for.
  check(["play/themes/index.json", "play/themes/hopglow/look.txt", "play/themes/hopglow/art/glow.png"].every((f) => paths.includes(f)), "its own theme, Hop Glow, is in the package's play/themes");
  const offeredOwn = await S.gp(() => window.__gp.looks?.() ?? null);
  check(Array.isArray(offeredOwn) && offeredOwn.includes("hopglow") && (await S.gp(() => window.__gp.theme)) !== "hopglow", `Settings offers it (${JSON.stringify(offeredOwn)}), and with no look asked for the game opens in its own pictures`);
  check(!(await p.locator("#start").isVisible()) && !(await p.locator("#load-label").isVisible()) && !(await p.locator("#back").isVisible()), "no ROM is asked for: no picker, no Unload ROM");
  check(!liftFetched(S), "nothing is lifted (no request for the lift)");
  check((await S.gp(() => window.__gp?.module)) === "page", "it runs in the page's own interpreter (it has no module of its own)");
  const mode = await p.locator("#mode").innerText();
  check(mode === "Playing Own Hop, made by the page check: it plays from its own files, with no ROM", `the line under the title says what plays ("${mode}")`);
  check((await p.title()) === "Own Hop" && (await p.locator("#title").innerText()) === "Own Hop", `the page is titled for it (${await p.title()})`);
  const howto = await p.locator("#howto").innerText();
  check(/X is the A button/.test(howto) && !/Load your own ROM|ROM/.test(howto), `the how-to-play lines name its keys and no ROM ("${howto.replace(/\n+/g, " / ")}")`);
  const about = await p.locator("#about").textContent();
  check(
    about.includes("Own Hop is made up for the page check: a copy of the Moss Hop template, with its pictures and no ROM. It plays from its own pictures, so there is no ROM to load. Made by the page check.") && !/Nintendo|Donkey|accepts a/.test(about),
    "About says what it is and who made it, from its package, and names no ROM",
  );
  await S.shot("own-game.png");
  // A file dropped on the page is no ROM to it.
  await p.evaluate(() => window.__gpBoot?.take?.(new File([new Uint8Array(4)], "x.nes"), "x.nes"));
  const said = await messageOf(p, ["warn", "error"], 5000);
  check(/Own Hop plays from its own files: there is no ROM to load/.test(said), `a file given to it anyway isn't read ("${said}")`);
  // The share link: the game by its name, read back.
  await p.evaluate(() => delete document.getElementById("message").dataset.share);
  await p.locator("#share").click();
  await p.waitForFunction(() => document.getElementById("message").dataset.share, null, { timeout: 30000 }).catch(() => {});
  const link = await p.evaluate(() => document.getElementById("message").dataset.share ?? null);
  const q = link ? new URL(link).searchParams : new URLSearchParams();
  check(!!link && q.get("g") === OWN && new URL(link).pathname === "/p" && [...q.keys()].every((k) => ["g", "look", "mode"].includes(k)), `its share link names the game, its look and its mode and nothing else (${link})`);
  if (link) {
    await p.goto(link);
    check(await installed(), "and opened, the link installs it again");
    const again = await frameSrc();
    check(!!again && !/files=/.test(again) && (await S.reframe(again, 30)), "and it plays at once again");
    await p.evaluate(() => delete document.getElementById("message").dataset.share);
    await p.locator("#share").click();
    await p.waitForFunction(() => document.getElementById("message").dataset.share, null, { timeout: 30000 }).catch(() => {});
    const second = await p.evaluate(() => document.getElementById("message").dataset.share ?? null);
    check(second === link, `and its share link reads back the same (${second})`);
  }
  // In a fresh browser, nothing kept: the link alone plays it.
  const fresh = await p.context().browser().newContext({ viewport: { width: 1366, height: 768 } });
  try {
    fresh.on("request", (r) => S.requests.push({ method: r.method(), url: r.url(), body: r.postDataBuffer()?.length ?? 0 }));
    if (registryFolder && page.source.url !== ".") await fresh.route((u) => isSourceUrl(S.origin, u), (r) => fromFolder(r));
    const q2 = await fresh.newPage();
    await q2.goto(link ?? `${S.origin}/p?g=${OWN}`);
    await q2.waitForFunction(() => window.__page !== undefined, null, { timeout: 60000 });
    await q2.waitForFunction(() => document.getElementById("game").getAttribute("src"), null, { timeout: 30000 }).catch(() => {});
    const fr = () => q2.frame({ url: /\/game\/(index\.html)?(\?|$)/ });
    const t0 = Date.now();
    let n = 0;
    while (Date.now() - t0 < 30000 && n < 30) {
      n = (await fr()?.evaluate(() => window.__gp?.hashes?.length ?? 0).catch(() => 0)) ?? 0;
      await q2.waitForTimeout(200);
    }
    check(n >= 30 && !(await q2.locator("#load-label").isVisible()), `the shared link in a fresh browser plays it with no ROM asked for (${n} frames)`);
    // A link that names its theme (gp game share's names the look the
    // person chose) opens in it.
    await q2.goto(`${S.origin}/p?g=${OWN}&look=hopglow`);
    await q2.waitForFunction(() => document.getElementById("game").getAttribute("src"), null, { timeout: 30000 }).catch(() => {});
    let drawn = null;
    const t1 = Date.now();
    while (Date.now() - t1 < 30000 && !drawn) {
      drawn = (await fr()?.evaluate(() => (window.__gp?.hashes?.length > 30 ? window.__gp.theme : null)).catch(() => null)) ?? null;
      if (!drawn) await q2.waitForTimeout(200);
    }
    check(drawn === "hopglow", `and ?g=${OWN}&look=hopglow opens it in its own theme, Hop Glow (${drawn})`);
  } finally {
    await fresh.close();
  }
  check(errors.length === 0, `no page errors (${errors.join("; ")})`);
}

// A tampered package of it: a changed play/ file (its bundle), or a changed
// manifest, and nothing is installed or played.
async function ownGameTampered(S, which) {
  const { p, check } = S;
  await p.goto(`${S.origin}/p?g=${OWN}`);
  const said = await messageOf(p, ["error"], 60000);
  const re = which === "blob" ? /doesn't match what its maker signed/ : /couldn't be installed/;
  check(re.test(said), `${which === "blob" ? "its play/bundle.json's bytes changed" : "its manifest changed"}: the page says so (${said})`);
  check(!(await p.locator("#game").getAttribute("src")), "no game starts");
  check((await installedOf(p, OWN)) === null, "nothing is installed");
}

// ---- A theme on an arcade game's board (gp_board.wasm, gp_present::art) ----

// Tapper's preview from the player's own romset with one made-up theme,
// `flat`, as scripts/web/tapper-theme.sh writes it (--tapper-theme DIR,
// default DIST.tapper-theme): the bartender's sprite (every code of the
// entity `player`) as a flat square of one colour, nothing else named. Its
// files are served beside the page's own player files (game/index.html,
// preview.js, gp_web.wasm, gp_board.wasm), so the page's own code draws the
// frame. The same run in the board's own look and in the theme: the square
// is drawn where the bartender's sprite was, 32 by 32, and every pixel
// outside it is the board's as before (the customers, the bars, the
// screen).
const tapperThemeDir = resolve(opt("tapper-theme") ?? `${dist}.tapper-theme`);
const tapperThemeHere = existsSync(join(tapperThemeDir, "bundle.json")) && existsSync(join(tapperThemeDir, "themes", "flat", "art.txt"));
async function arcadeTheme(S) {
  const { p, check } = S;
  const own = (u) => existsSync(join(dist, decodeURIComponent(u.pathname))) || /^\/game\/runs\/.+\.frames$/.test(u.pathname);
  await S.context.route(
    (u) => u.origin === S.origin && u.pathname.startsWith("/game/") && !own(u),
    (route) => {
      const path = new URL(route.request().url()).pathname;
      const f = join(tapperThemeDir, decodeURIComponent(path.slice("/game/".length)));
      if (!f.startsWith(tapperThemeDir) || !existsSync(f) || statSync(f).isDirectory()) return route.fulfill({ status: 404, body: "" });
      const headers = Object.fromEntries(headersFor(path));
      headers["content-type"] = types[extname(f)] ?? "application/octet-stream";
      return route.fulfill({ status: 200, headers, body: readFileSync(f) });
    },
  );
  // A coin and the start (the cabinet's columns: coin is bit 0, start bit
  // 1), as the corpus's start of play has them, then 500 frames of play.
  const n = 700;
  const pad = new Array(n).fill(0);
  for (let f = 124; f <= 134; f++) pad[f] |= 1;
  for (let f = 195; f <= 201; f++) pad[f] |= 2;
  S.runs.set("arcade-theme", "gp-frames 1\n" + pad.map((v) => v.toString(16).padStart(2, "0")).join("\n") + "\n");
  const drawn = async (theme) => {
    await p.goto(`${S.origin}/game/index.html?files=here&run=arcade-theme${theme ? `&theme=${theme}` : ""}`);
    // The run plays in time (about 23 s at 30 frames a second) and stops
    // at its end, so both pictures are of the same frame.
    check(await S.waitFrames(n + 1, 120000), `Tapper plays ${n} frames from a coin and the start ${theme ? `in the made-up theme ${theme}` : "in the board's own look"}`);
    await p.waitForTimeout(300);
    const t = await S.gp(() => ({ theme: window.__gp.theme ?? null, board: window.__gp.board, error: window.__gp.error, frames: window.__gp.hashes.length }));
    check(t.frames === n + 1, `and stops at the run's end (${t.frames - 1} frames)`);
    check(t.board === true && t.error === null, `drawn by the board's art (gp_board.wasm), with no error (${t.error})`);
    if (theme) check(t.theme === theme, `and in the theme (${t.theme})`);
    await S.shot(`arcade-theme-${theme || "original"}.png`);
    const got = await S.gp(() => {
      const c = document.getElementById("game");
      const d = c.getContext("2d").getImageData(0, 0, c.width, c.height).data;
      let bin = "";
      for (let i = 0; i < d.length; i += 0x8000) bin += String.fromCharCode(...d.subarray(i, i + 0x8000));
      // The bartender (the entity `player`) where the board puts his
      // sprite: x - 3 and 241 - y in the game's units, two pixels each.
      const b = window.__gp.state().bodies.find(([k, , , , alive]) => k === "player" && alive);
      const rect = b && [(((b[1] % 256) + 256) % 256 - 3) * 2, (241 - (((b[2] % 256) + 256) % 256)) * 2];
      return { w: c.width, h: c.height, rgba: btoa(bin), rect };
    });
    return { ...got, rgba: Buffer.from(got.rgba, "base64") };
  };
  const plain = await drawn(null);
  const themed = await drawn("flat");
  check(plain.w === 512 && plain.h === 480 && themed.w === 512 && themed.h === 480, `both are the board's 512 by 480 picture (${plain.w} by ${plain.h}, ${themed.w} by ${themed.h})`);
  const at = (img, x, y) => [...img.rgba.subarray((y * img.w + x) * 4, (y * img.w + x) * 4 + 3)];
  const same = (a, b) => a.every((v, i) => v === b[i]);
  const square = [200, 40, 160];
  const box = [Infinity, Infinity, -Infinity, -Infinity];
  let ink = 0;
  for (let y = 0; y < themed.h; y++)
    for (let x = 0; x < themed.w; x++)
      if (same(at(themed, x, y), square)) {
        ink++;
        box[0] = Math.min(box[0], x);
        box[1] = Math.min(box[1], y);
        box[2] = Math.max(box[2], x + 1);
        box[3] = Math.max(box[3], y + 1);
      }
  const [bw, bh] = [box[2] - box[0], box[3] - box[1]];
  check(ink === 32 * 32 && bw === 32 && bh === 32, `the theme's square is drawn once, 32 by 32 of the board's pixels (${ink} pixels in ${bw} by ${bh} at ${box[0]}, ${box[1]})`);
  const inBox = (x, y) => x >= box[0] && x < box[2] && y >= box[1] && y < box[3];
  let outside = 0;
  let was = 0;
  for (let y = 0; y < plain.h; y++)
    for (let x = 0; x < plain.w; x++) {
      const differ = !same(at(plain, x, y), at(themed, x, y));
      if (differ && !inBox(x, y)) outside++;
      if (differ && inBox(x, y)) was++;
    }
  check(outside === 0, `every pixel outside the square is the board's own look's: the unthemed sprites (customers, mugs, the bartender's other parts) and the screen are unchanged (${outside} differ)`);
  check(!!plain.rect && plain.rect[0] === box[0] && plain.rect[1] === box[1], `and it stands where the board draws the bartender's sprite, from his place in the game (${plain.rect?.join(", ")})`);
  let his = 0;
  for (let y = box[1]; y < box[3]; y++) for (let x = box[0]; x < box[2]; x++) if (!same(at(plain, x, y), at(plain, box[0], box[3] - 1))) his++;
  check(was > 0 && his > 64, `where the board's own look has his picture (${his} of its pixels differ from the box's corner; ${was} differ from the theme's)`);
  // The theme's three files and the page's player, nothing else of it.
  const asked = S.requests.map((r) => new URL(r.url).pathname).filter((u) => u.startsWith("/game/themes/"));
  const want = ["/game/themes/index.json", "/game/themes/flat/look.txt", "/game/themes/flat/art.txt", "/game/themes/flat/art/square.png"];
  check(asked.every((u) => want.includes(u)) && want.slice(1).every((u) => asked.includes(u)), `the page fetched the theme's look, rules and one picture and nothing else of it (${[...new Set(asked)].join(", ")})`);
  check(S.requests.every((r) => r.url.startsWith(`${S.origin}/game/`) || r.url.startsWith("blob:") || r.url.startsWith("data:")), "and every request is for the game's own files on the page's server");
  check(S.errors.length === 0, `no page errors (${S.errors.join("; ")})`);
}

// ---- An arcade game by its name: played from the visitor's romset zip ----

// The arcade game the sections play (--arcade-game, default lex00/tapper)
// from the registry folder, and its romsets in $GP_ROM_DIR by their MAME
// names: the zips are read by the browser where they lie, never copied, and
// a zip with a chip left out is made in memory and handed to the picker as
// bytes, never written to disk.
const ARCADE = opt("arcade-game") ?? "lex00/tapper";
const arcadeHere = !!registryFolder && existsSync(join(registryFolder, "index", ARCADE));
const ARCADE_LINK = `p?g=${ARCADE}`;
// A copy of the arcade game whose maps record no place of the Root Beer
// set's tables (scripts/web/test-registry.sh makes it from the game's
// package): the set the lift refuses as not drawn right yet.
const UNPLACED = "test/tapper-unplaced";
const unplacedHere = !!registryFolder && existsSync(join(registryFolder, "index", UNPLACED));
// The arcade game named in the link, installed (page.js sets __page once
// it is, or couldn't be).
const arcadeInstalled = async (p, name = ARCADE) => {
  await p.waitForFunction(() => window.__page !== undefined, null, { timeout: 60000 }).catch(() => {});
  return (await installedOf(p, name)) !== null;
};
function findZip(name) {
  const root = process.env.GP_ROM_DIR ?? join(homedir(), ".gameprinter", "roms");
  for (const p of [join(root, "arcade", `${name}.zip`), join(root, `${name}.zip`)]) if (existsSync(p) && statSync(p).isFile()) return p;
  return null;
}
// A zip's entries as stored, from its central directory: each entry's
// local header and data copied as they are.
function zipWithout(bytes, drop) {
  const b = Buffer.from(bytes);
  let end = b.length - 22;
  while (end >= 0 && b.readUInt32LE(end) !== 0x06054b50) end--;
  if (end < 0) throw new Error("not a zip");
  const count = b.readUInt16LE(end + 10);
  let at = b.readUInt32LE(end + 16);
  const locals = [];
  const centrals = [];
  const names = [];
  for (let i = 0; i < count; i++) {
    const nameLen = b.readUInt16LE(at + 28);
    const len = 46 + nameLen + b.readUInt16LE(at + 30) + b.readUInt16LE(at + 32);
    const name = b.toString("latin1", at + 46, at + 46 + nameLen);
    const off = b.readUInt32LE(at + 42);
    const size = b.readUInt32LE(at + 20);
    const localLen = 30 + b.readUInt16LE(off + 26) + b.readUInt16LE(off + 28) + size;
    if (name !== drop) {
      names.push(name);
      locals.push(b.subarray(off, off + localLen));
      centrals.push(Buffer.from(b.subarray(at, at + len)));
    }
    at += len;
  }
  let o = 0;
  for (let i = 0; i < locals.length; i++) {
    centrals[i].writeUInt32LE(o, 42);
    o += locals[i].length;
  }
  const cd = Buffer.concat(centrals);
  const eocd = Buffer.alloc(22);
  eocd.writeUInt32LE(0x06054b50, 0);
  eocd.writeUInt16LE(locals.length, 8);
  eocd.writeUInt16LE(locals.length, 10);
  eocd.writeUInt32LE(cd.length, 12);
  eocd.writeUInt32LE(o, 16);
  return { zip: Buffer.concat([...locals, cd, eocd]), names };
}
function zipNames(bytes) {
  return zipWithout(bytes, null).names;
}
// A zip of made-up files (stored, no compression): no chip of any game.
function madeUpZip(files) {
  const crcTable = Array.from({ length: 256 }, (_, n) => {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    return c >>> 0;
  });
  const crc = (d) => {
    let c = 0xffffffff;
    for (const x of d) c = crcTable[(c ^ x) & 0xff] ^ (c >>> 8);
    return (c ^ 0xffffffff) >>> 0;
  };
  const locals = [];
  const centrals = [];
  let o = 0;
  for (const [name, data] of files) {
    const n = Buffer.from(name, "latin1");
    const h = Buffer.alloc(30);
    h.writeUInt32LE(0x04034b50, 0);
    h.writeUInt16LE(20, 4);
    h.writeUInt32LE(crc(data), 14);
    h.writeUInt32LE(data.length, 18);
    h.writeUInt32LE(data.length, 22);
    h.writeUInt16LE(n.length, 26);
    const c = Buffer.alloc(46);
    c.writeUInt32LE(0x02014b50, 0);
    c.writeUInt16LE(20, 4);
    c.writeUInt16LE(20, 6);
    c.writeUInt32LE(crc(data), 16);
    c.writeUInt32LE(data.length, 20);
    c.writeUInt32LE(data.length, 24);
    c.writeUInt16LE(n.length, 28);
    c.writeUInt32LE(o, 42);
    locals.push(h, n, data);
    centrals.push(c, n);
    o += 30 + n.length + data.length;
  }
  const cd = Buffer.concat(centrals);
  const eocd = Buffer.alloc(22);
  eocd.writeUInt32LE(0x06054b50, 0);
  eocd.writeUInt16LE(files.length, 8);
  eocd.writeUInt16LE(files.length, 10);
  eocd.writeUInt32LE(cd.length, 12);
  eocd.writeUInt32LE(o, 16);
  return Buffer.concat([...locals, cd, eocd]);
}
// A file handed to the picker as bytes, then the page's message once it
// says something of `kinds`.
const pickBytes = async (p, name, buffer, kinds = ["error"]) => {
  await p.evaluate(() => (document.getElementById("message").dataset.kind = ""));
  await p.setInputFiles("#rom", { name, mimeType: "application/zip", buffer });
  return messageOf(p, kinds);
};
const boardFetched = (S) => S.requests.some((r) => /\/gp_board\.wasm(\?|$)/.test(r.url));

// "What is this?" on Donkey Kong by the link (what-is-this.js,
// explain.js): the button holds the game, a click on Mario names him from
// the package's own model with a box around him and his place from the
// state, a click on the stage says it is the stage, "Play on" lets the game
// go on; then the same with a tap at 360 pixels wide, the answer under the
// game and the page not scrolling sideways. Nothing is asked of a server:
// every request stays the page's own or the source's.
async function whatIsThisSection(S, r) {
  const { p, check, gp, key, waitFrames, errors } = S;
  await p.setViewportSize({ width: 1400, height: 900 });
  await p.goto(`${S.origin}/${LINK}`);
  check(await S.installed(), "the link opens");
  check(!(await p.locator("#what").isVisible()), "with no game playing there is no What is this? button");
  await S.loadRom(r.rom);
  check(await waitFrames(30), `${r.name} plays`);
  check(await p.locator("#what").isVisible(), "with a game playing the header has a What is this? button");
  // Into 25m, past the stage's start, Mario standing at the bottom.
  await key("Enter");
  const inPlay = await S.until(() => window.__gp.state().level === "main" && window.__gp.state().frame > 900, null, 30000);
  check(inPlay, "Start takes the game into 25m");
  await p.waitForTimeout(4500);
  // Where Mario is drawn, as fractions of the canvas: the first spot of a
  // grid the frame's own spot names him at.
  const findOn = () =>
    gp(() => {
      for (let y = 0.02; y < 1; y += 0.02) for (let x = 0.01; x < 1; x += 0.01) {
        const h = window.__gp.spot(x, y, 0).hits[0];
        if (h?.kind === "player") return { x, y, at: [h.x, h.y] };
      }
      return null;
    });
  // The canvas's box, which Playwright gives in the page's own pixels.
  const pageAt = async (f) => {
    const c = await (await S.game()).locator("#game").boundingBox();
    return { x: c.x + f.x * c.width, y: c.y + f.y * c.height };
  };
  const requestsBefore = S.requests.length;
  await p.locator("#what").click();
  check((await p.locator("#what").getAttribute("aria-pressed")) === "true" && (await p.locator("#what-panel").isVisible()), "pressed, it opens a panel that asks to point at something");
  // The hold is a message to the frame, taken at its next animation frame.
  await p.waitForTimeout(300);
  const h0 = await gp(() => window.__gp.hashes.length);
  await p.waitForTimeout(600);
  const h1held = await gp(() => window.__gp.hashes.length);
  check(h1held === h0, `and the game holds while it is asked (${h1held - h0} frames stepped in 0.6 s)`);
  const mario = await findOn();
  check(!!mario, `Mario is drawn somewhere on the picture (${JSON.stringify(mario)})`);
  if (!mario) return;
  const pt = await pageAt(mario);
  await p.mouse.click(pt.x, pt.y);
  await p.waitForFunction(() => document.getElementById("what-panel").dataset.answer, null, { timeout: 15000 }).catch(() => {});
  const said = await p.locator("#what-panel").innerText();
  check((await p.locator("#what-panel").getAttribute("data-answer")) === "player" && /^Mario\b/.test(said), `a click on Mario names him ("${said.replace(/\s+/g, " ").slice(0, 300)}")`);
  check(said.includes(`at x ${mario.at[0]}, y ${mario.at[1]}`), `with his place in the model's state (x ${mario.at[0]}, y ${mario.at[1]})`);
  check(/the one you play/.test(said) && /state mapping reads it from RAM/.test(said) && /How it was worked out/.test(said), "from the package's model and mapping: his role, how he was worked out, where the original keeps him");
  check(/gp element explain/.test(said) && /no server/.test(said), "and it says what only the tools can add");
  const box = await p.locator("#what-layer .what-box").boundingBox();
  check(!!box && pt.x >= box.x && pt.x <= box.x + box.width && pt.y >= box.y && pt.y <= box.y + box.height, `a box is drawn around him (${JSON.stringify(box)})`);
  await S.shot("what-is-this-1400.png");
  // The stage: a spot with no body or HUD field on it.
  const empty = await gp(() => {
    for (let y = 0.5; y < 0.95; y += 0.05) for (let x = 0.3; x < 0.9; x += 0.05) {
      const s = window.__gp.spot(x, y, 0);
      if (!s.hits.length) return { x, y };
    }
    return null;
  });
  if (empty) {
    const e = await pageAt(empty);
    await p.mouse.click(e.x, e.y);
    await p.waitForFunction(() => document.getElementById("what-panel").dataset.answer === "place", null, { timeout: 5000 }).catch(() => {});
    const t = await p.locator("#what-panel").innerText();
    check(/^The stage/.test(t) && /25m/.test(t), `a click on the stage says it is the stage ("${t.split("\n").slice(0, 2).join(" ")}")`);
  } else check(false, "a spot with nothing on it was found");
  check(
    S.requests.slice(requestsBefore).every((q) => q.method === "GET" && (q.url.startsWith(S.origin) || isSourceUrl(S.origin, q.url) || q.url.startsWith("blob:"))),
    "the answers asked nothing of any server",
  );
  await p.locator('#what-panel [data-what="done"]').click();
  check(!(await p.locator("#what-panel").isVisible()) && (await p.locator("#what").getAttribute("aria-pressed")) === "false", "Play on closes it");
  const h1 = await gp(() => window.__gp.hashes.length);
  await p.waitForTimeout(800);
  check((await gp(() => window.__gp.hashes.length)) > h1 + 10, "and the game goes on");
  // A phone: 360 pixels wide, a tap.
  await p.setViewportSize({ width: 360, height: 760 });
  await p.waitForTimeout(800);
  await p.locator("#what").click();
  await p.waitForTimeout(300);
  const m2 = await findOn();
  check(!!m2, "at 360 pixels wide Mario is still found on the picture");
  if (m2) {
    const q = await pageAt(m2);
    await p.locator("#what-layer").dispatchEvent("pointerup", { clientX: q.x, clientY: q.y, pointerType: "touch", bubbles: true });
    await p.waitForFunction(() => document.getElementById("what-panel").dataset.answer === "player", null, { timeout: 10000 }).catch(() => {});
    check((await p.locator("#what-panel").getAttribute("data-answer")) === "player", "a tap on Mario names him at 360 pixels wide");
    const pb = await p.locator("#what-panel").boundingBox();
    const gb = await p.locator("#game").boundingBox();
    check(!!pb && !!gb && pb.y >= gb.y + gb.height - 1 && pb.width <= 360, `the answer is under the game (game ends at ${Math.round(gb?.y + gb?.height)}, answer starts at ${Math.round(pb?.y ?? 0)})`);
  }
  const wide = await p.evaluate(() => document.documentElement.scrollWidth);
  check(wide <= 360, `and the page doesn't scroll sideways (${wide} wide)`);
  await S.shot("what-is-this-360.png");
  await p.keyboard.press("Escape");
  check(!(await p.locator("#what-panel").isVisible()), "Esc closes it");
  check(errors.length === 0, `no page errors (${errors.join("; ")})`);
}

// The arcade game by its name, played from the visitor's own zip: its
// facts from its package (the romset line, the cabinet's keys and the line
// about its sound), the picker taking a zip, the lift, and the game in
// the page's frame: the board's 512 by 480 picture from gp_board.wasm, the
// coin and the start taken. Shots of it at 1400 and at 360 pixels wide.
async function arcadePlay(S, { zip }) {
  const { p, check, gp, waitFrames, errors } = S;
  await p.goto(`${S.origin}/${ARCADE_LINK}`);
  check(await arcadeInstalled(p), `${ARCADE} by its name is installed with no click`);
  const rec = await installedOf(p, ARCADE);
  check(rec?.kind === "game" && rec.files.some((f) => f.path === "play/page.json"), `from its package (${rec?.name}@${rec?.version}, ${rec?.files.length} files)`);
  const title = await p.title();
  check(title === "Tapper" && !/Donkey|Nintendo/.test(await p.locator("#about").textContent()), `the page is titled for it (${title}) and names no other game`);
  const help = await p.locator("#rom-help").innerText();
  check(/romset/.test(help) && /zip/.test(help) && !/\.nes/.test(help) && /never uploaded/.test(help), `the line under Load names a romset zip, not an .nes file ("${help}")`);
  check(/\.zip/.test((await p.locator("#rom").getAttribute("accept")) ?? ""), `the picker takes a zip (accept ${await p.locator("#rom").getAttribute("accept")})`);
  const howto = await p.locator("#howto").innerText();
  check(/5 drops a coin and 1 starts/.test(howto), "the how-to-play lines give the cabinet's keys");
  const sound = (await p.locator("#howto p").allInnerTexts()).filter((t) => /^Sound:/.test(t));
  check(
    sound.length === 1 && sound[0] === "Sound: The board's own sound, played from your romset in this browser. It starts on your first key or click.",
    `and one plain line says where its sound comes from and when it starts ("${sound.join(" | ")}")`,
  );
  check(!boardFetched(S), "no gp_board.wasm is fetched before a game plays");
  await p.setViewportSize({ width: 1400, height: 900 });
  const said = await S.loadRom(zip);
  check(/Tapper \(Budweiser, 1\/27\/84\)/.test(said) && (await p.locator("#message").getAttribute("data-kind")) === "ok", `the zip is lifted and named ("${said}")`);
  const mode = await p.locator("#mode").innerText();
  check(/^Playing your own Tapper \(Budweiser, 1\/27\/84\) romset: all \d+ of its assets found/.test(mode), `the header says what plays ("${mode}")`);
  check(await waitFrames(30), "the game plays in the page's frame");
  check(await S.until(() => window.__gp.board === true), "with the board's art (gp_board.wasm loaded the lifted files)");
  check(boardFetched(S), "gp_board.wasm was fetched for it");
  const size = await gp(() => [document.getElementById("game").width, document.getElementById("game").height]);
  check(size[0] === 512 && size[1] === 480, `the canvas is the board's size (${size.join(" by ")})`);
  // The key hint under the game comes from the cabinet's layout: 5 and 1
  // are the coin and the start, so the slots load on 2-4 and the mute keys
  // are 6-9, and the NES hint's keys aren't named.
  const hint = await gp(() => window.__gp.keyHint?.() ?? null);
  check(
    !!hint && hint.startsWith("5 coin, 1 start, ") && /\b2-4 load\b/.test(hint) && !/\b1-4 load\b|\b5-9\b/.test(hint) && (!/mute a channel/.test(hint) || /\b6-9 mute a channel\b/.test(hint)),
    `the key hint under the game is the cabinet's ("${hint}")`,
  );
  const shown = await S.until(() => [...document.querySelectorAll("div")].some((d) => d.textContent === window.__gp.keyHint()));
  check(shown, "and it is the line shown under the game");
  // A coin and the start: the frames go on and the picture changes.
  const before = await gp(() => window.__gp.hashes.length);
  await S.key("Digit5", 200);
  await p.waitForTimeout(800);
  await S.key("Digit1", 200);
  await p.waitForTimeout(3000);
  const after = await gp(() => window.__gp.hashes.length);
  check(after - before > 30, `after a coin and a start it plays on (${after - before} frames)`);
  check((await gp(() => window.__gp.error)) === null, `with no error in the frame: ${await gp(() => window.__gp.error)}`);
  const lit = await gp(() => {
    const c = document.getElementById("game");
    const d = c.getContext("2d").getImageData(0, 0, c.width, c.height).data;
    const seen = new Set();
    for (let i = 0; i < d.length; i += 4 * 7) seen.add((d[i] << 16) | (d[i + 1] << 8) | d[i + 2]);
    return seen.size;
  });
  check(lit >= 8, `and the board's picture has colours in it (${lit} seen)`);
  // The shots a few seconds into the first bar, the bartender and the
  // customers out.
  await p.waitForTimeout(5000);
  await S.shot("arcade-play-1400.png");
  await p.setViewportSize({ width: 360, height: 760 });
  await p.waitForTimeout(800);
  const wide = await p.evaluate(() => document.documentElement.scrollWidth);
  const box = await p.locator("#game").boundingBox();
  check(wide <= 360 && box && box.width <= 360, `at 360 pixels wide the page doesn't scroll sideways (page ${wide}, game ${Math.round(box?.width ?? 0)} wide)`);
  await S.shot("arcade-play-360.png");
  check(errors.length === 0, `no page errors (${errors.join("; ")})`);
}

// The arcade game's own sound on the public page, under Safari's rule for
// sound (a context runs only when made or resumed during a key or a click)
// and with the zip loaded with no click: the game's module gives each
// frame's sound requests (gp_sound_requests), the frame powers the board's
// sound on from the zip in game/gp_arcade_play.wasm and feeds it every
// frame, and nothing is heard until the first key, which starts it; then
// a coin and a start, and the board's sound plays with no error.
const arcadePlayFetched = (S) => S.requests.some((r) => /\/gp_arcade_play\.wasm(\?|$)/.test(r.url));
async function arcadeSound(S, { zip }) {
  const { p, check, gp, waitFrames, errors } = S;
  await p.goto(`${S.origin}/${ARCADE_LINK}`);
  check(await arcadeInstalled(p), `${ARCADE} by its name is installed with no click`);
  check(!arcadePlayFetched(S), "no gp_arcade_play.wasm is fetched before a game plays");
  await S.loadRom(zip);
  check(await waitFrames(60), "the game plays in the page's frame");
  check(await S.until(() => window.__gp.boardSound === true), "its frame powered the board's sound on from the zip (the module gives sound requests)");
  check(arcadePlayFetched(S), "gp_arcade_play.wasm was fetched for it");
  const stepped = await gp(() => window.__gp.boardSoundFrames());
  const before = await S.soundOf(p, await S.game());
  const state = await gp(() => window.__gp.audioState());
  check(stepped.frames > 0 && before.buffers === 0 && state !== "running", `the board runs from the game's first frames but nothing is heard before a key (${stepped.frames} frames stepped, ${before.buffers} buffers queued, context ${state})`);
  await S.soundReset(p, await S.game());
  await S.key("Digit5", 200);
  await p.waitForTimeout(800);
  await S.key("Digit1", 200);
  await p.waitForTimeout(3000);
  const after = await S.soundOf(p, await S.game());
  const heard = await gp(() => window.__gp.boardSoundFrames());
  const running = await gp(() => window.__gp.audioState());
  check(running === "running" && heard.heard > 30 && after.buffers > 60, `the first key starts it: the board's sound is queued from then on (context ${running}, ${heard.heard} frames heard, ${after.buffers} buffers)`);
  check(after.loud > 0, `and a coin and a start sound (${after.loud} of ${after.buffers} buffers with sound)`);
  check(after.lead !== null && after.lead < 0.3, `played at once (${after.lead === null ? "nothing played" : after.lead.toFixed(2) + " s ahead"})`);
  check((await gp(() => window.__gp.error)) === null && (await gp(() => window.__gp.boardSound)) === true, `the sound runs on with no error in the frame: ${await gp(() => window.__gp.error)}`);
  check(errors.length === 0, `no page errors (${errors.join("; ")})`);
}

// Zips the arcade game's lift refuses, each in words on the page with no
// game started: a file that isn't a zip, a zip of made-up files and a zip
// of the set with a chip left out (made in memory).
async function arcadeRefused(S, { zip }) {
  const { p, check, errors } = S;
  await p.goto(`${S.origin}/${ARCADE_LINK}`);
  check(await arcadeInstalled(p), `${ARCADE} by its name is installed`);
  const none = async () => !(await p.locator("#game").getAttribute("src"));
  let said = await pickBytes(p, "tapper.zip", Buffer.from("This is not a zip, only some words.\n"));
  check(/^This file isn't a zip\. This page takes a MAME romset zip of Tapper: tapper/.test(said) && (await none()), `a file that isn't a zip is refused in words ("${said}")`);
  said = await pickBytes(p, "tapper.zip", madeUpZip([["made-up.bin", Buffer.from(Array.from({ length: 4096 }, (_, i) => (i * 31) & 0xff))]]));
  check(/^This zip holds no chip of a Tapper romset\./.test(said) && (await none()), `a zip of no chip of the game is refused in words ("${said}")`);
  if (zip) {
    const bytes = readFileSync(zip);
    const first = zipNames(bytes)[0];
    said = await pickBytes(p, "tapper.zip", zipWithout(bytes, first).zip);
    check(new RegExp(`^This zip isn't a whole Tapper romset: it has \\d+ of the \\d+ chips of tapper .*lacks ${first.replace(/[.]/g, "\\.")}`).test(said) && (await none()), `a zip with a chip left out is refused naming it ("${said}")`);
  } else S.lines.push("skip  no tapper.zip in $GP_ROM_DIR: a zip with a chip left out isn't checked");
  check(!boardFetched(S), "nothing of a refused zip is drawn (no gp_board.wasm fetched)");
  await S.shot("arcade-refused.png");
  check(errors.length === 0, `no page errors (${errors.join("; ")})`);
}

// The owner's notice for an arcade game (#3) and the sets to bring: the
// page says by the picker, and again in About, in one short line that a person needs their own romset and that the
// game's names and trademarks belong to their owners (the package's
// trademark line, or the page's own for an arcade game), and names the MAME
// sets the page takes with the files each zip must hold; a refused zip's
// message ends saying which set to bring. Reads no zip.
async function arcadeNoticeSection(S) {
  const { p, check, errors } = S;
  await p.goto(`${S.origin}/${ARCADE_LINK}`);
  check(await arcadeInstalled(p), `${ARCADE} by its name is installed`);
  const title = await p.title();
  const mark = (await p.locator("#about-mark").textContent()).trim();
  check(
    mark === `You need your own ${title} romset to play; ${title}'s names and trademarks belong to their owners.` && mark.length <= 160 && !/Nintendo/.test(mark),
    `About's notice is the one short line ("${mark}")`,
  );
  // The same line by the picker, visible with About closed, at 1400 and at
  // 360 pixels wide.
  for (const [width, height] of [[1400, 900], [360, 760]]) {
    await p.setViewportSize({ width, height });
    await p.waitForTimeout(300);
    const shown = await p.locator("#rom-notice").isVisible();
    const line = (await p.locator("#rom-notice").innerText()).trim();
    const wide = await p.evaluate(() => document.documentElement.scrollWidth);
    const closed = !(await p.locator("#about-box").evaluate((d) => d.open));
    check(shown && closed && line === mark && wide <= width, `at ${width} pixels wide the notice shows by the picker, with About closed and no sideways scroll ("${line}", page ${wide} wide)`);
    await S.shot(`arcade-notice-${width}.png`);
  }
  const sets = (await p.locator("#about-sets p").allTextContents()).slice(1);
  check(
    (await p.locator("#about-sets").getAttribute("hidden")) === null && sets.length > 0 && sets.every((t) => /^[a-z0-9_]+, .+: \d+ files: .+ \(\d+ bytes, SHA-1 [0-9a-f]{40}\)/.test(t)),
    `About names the sets the page takes with each zip's files (${sets.map((t) => t.split(":")[0]).join(" | ")})`,
  );
  const said = await pickBytes(p, "tapper.zip", Buffer.from("This is not a zip, only some words.\n"));
  check(
    /^This file isn't a zip\./.test(said) && / Bring (MAME's [a-z0-9_]+ \(.+\) set: a zip of its \d+ files|one of MAME's sets .+ or .+: About lists the files each zip must hold)/.test(said),
    `a refused zip's message says which set to bring ("${said}")`,
  );
  check(errors.length === 0, `no page errors (${errors.join("; ")})`);
}

// A set whose tables the game's maps record no place for (the owner's
// decision: refused in words, never offered and never hidden): the Root
// Beer set, rbtapper.zip, on the copy of the game with its places taken
// out. Refused by name, saying it isn't drawn right yet; nothing plays.
async function arcadeUnplaced(S, { garbled }) {
  const { p, check, errors } = S;
  await p.goto(`${S.origin}/p?g=${UNPLACED}`);
  check(await arcadeInstalled(p, UNPLACED), `${UNPLACED} by its name is installed`);
  await p.evaluate(() => (document.getElementById("message").dataset.kind = ""));
  await p.setInputFiles("#rom", garbled);
  const said = await messageOf(p);
  check(
    /^This is rbtapper \(Tapper \(Root Beer\)\), which this page doesn't draw right yet/.test(said) && !(await p.locator("#game").getAttribute("src")),
    `rbtapper.zip is refused in words naming the set and saying it isn't drawn right yet, and nothing plays ("${said}")`,
  );
  check(!boardFetched(S), "nothing of it is drawn (no gp_board.wasm fetched)");
  await S.shot("arcade-unplaced.png");
  check(errors.length === 0, `no page errors (${errors.join("; ")})`);
}

const SECTIONS = [];
const add = (name, est, run, extra = {}) => SECTIONS.push({ name, est, run, ...extra });
add("no-rom", 15, noRomPage);
add("wrong-rom", 3, wrongRom);
const skips = [];
const roms = [];
for (const [name, sha] of Object.entries(page.rom.known_sha256 ?? {})) {
  const rom = findRom(sha);
  if (!rom) {
    skips.push(`skip  no ${name} ROM in $GP_ROM_DIR: its ROM mode isn't checked`);
    continue;
  }
  const r = { name, sha, rom };
  roms.push(r);
  const tag = sha.slice(0, 8);
  add(`rom-${tag}-play`, 35, (S) => romPlay(S, r), { rom: sha, preview: false });
  add(`rom-${tag}-route`, 10, (S) => romRoute(S, r), { rom: sha, preview: false });
  add(`rom-${tag}-deaths`, 10, (S) => romDeaths(S, r), { rom: sha, preview: false });
  add(`rom-${tag}-saves`, 52, (S) => romSaves(S, r), { rom: sha, preview: false });
  add(`rom-${tag}-unload`, 6, (S) => romUnload(S, r), { rom: sha });
  if (pictureMoments.some((m) => m.section === `pictures-rom-${tag}`)) add(`pictures-rom-${tag}`, 15, (S) => romPictures(S, r), { rom: sha, preview: false, files: [picturesFile] });
}
if (roms.length) {
  const first = roms[0];
  for (const [rule, input, load] of [["chrome", "keys", "click"], ["chrome", "gamepad", "click"], ["safari", "keys", "click"], ["safari", "gamepad", "click"], ["safari", "gamepad", "none"]]) {
    add(`visitor-${rule}-${input}${load === "none" ? "-no-click" : ""}`, 11, (S) => visitor(S, { rule, input, load, rom: first.rom }), { rom: first.sha, gesture: rule, gamepad: true });
  }
  add("later-visit", 6, (S) => laterVisit(S, first), { rom: first.sha });
  add("kept-look", 8, (S) => keptLook(S, first), { rom: first.sha });
  add("what-is-this", 25, (S) => whatIsThisSection(S, first), { rom: first.sha });
  for (const r of roms) add(`theme-picks-${r.sha.slice(0, 8)}`, 20, (S) => themePicks(S, r), { rom: r.sha });
}

add("registry-link", 15, registryLink);
if (!page.source.root) add("themes-not-offered", 10, themesNotOffered, { source: notOfferedSource });
else skips.push("skip  themes-not-offered: the page's source is signed, so its theme list can't be changed for the check");
if (mosshopHere) add("theme-hud-over", 10, themeHudOver);
else skips.push(`skip  theme-hud-over: no Moss Hop build with the made-up themes at ${mosshopDir} (scripts/web/mosshop-hud.sh DIR makes one; give it with --mosshop DIR)`);
if (mosshopRegionHere) add("theme-in-region", 10, themeInRegion);
else skips.push(`skip  theme-in-region: no Moss Hop build with the made-up theme instart at ${mosshopDir} (scripts/web/mosshop-hud.sh DIR makes one; give it with --mosshop DIR)`);
if (takesHere) add("theme-sound-takes", 10, themeSoundTakes);
else skips.push(`skip  theme-sound-takes: no Moss Hop build with the made-up sound role at ${takesDir} (scripts/web/mosshop-takes.sh DIR makes one; give it with --mosshop-takes DIR)`);
if (tapperThemeHere) add("arcade-theme", 20, arcadeTheme, { arcade: true });
else skips.push(`skip  arcade-theme: no Tapper build with the made-up theme at ${tapperThemeDir} (scripts/web/tapper-theme.sh DIR makes one from $GP_ROM_DIR/arcade/tapper.zip; give it with --tapper-theme DIR)`);
if (roms.length) add("module", 15, (S) => moduleSection(S, roms[0]), { rom: roms[0].sha });
for (const r of roms) {
  const tag = r.sha.slice(0, 8);
  add(`registry-rom-${tag}`, 15, (S) => registryRom(S, r), { rom: r.sha });
  add(`registry-theme-${tag}`, 15, (S) => registryTheme(S, r), { rom: r.sha, firstLink: `${LINK}&c=${ART_THEME}` });
}
if (!registryStopped) {
  add("registry-links", 15, registryLinks);
  add("registry-goes-bad", 15, registryGoesBad);
  add("registry-local-change", 15, registryLocalChange);
  const bad = (name, what, re, match, change) => add(`registry-bad-${name}`, 10, refusedInstall(re, what), { source: broken(match, change) });
  // A source with no root isn't signed, so only hashes can catch a change.
  if (page.source.root) {
    bad("root-sig", "registry.toml's signature broken", /couldn't be installed/i, source_("registry.toml.sig"), flip);
    bad("root-text", "registry.toml changed", /couldn't be installed/i, source_("registry.toml"), (b) => Buffer.from(b.toString().replace("gp-staging", "gp-stagiing")));
    bad("namespace", "the namespace's keys changed", /couldn't be installed/i, source_("namespaces/lex00.toml"), (b) => Buffer.from(b.toString() + "\n# changed\n"));
    bad("catalog", "the catalog's latest changed", /couldn't be installed/i, source_("catalog.toml"), (b) => Buffer.from(b.toString().replace(/latest = "\d+\.\d+\.\d+"/, 'latest = "1.0.0"')));
    bad("manifest-sig", "the manifest's signature broken", /couldn't be installed/i, (u) => /\/packages\/[0-9a-f]{64}\.sig$/.test(u), flip);
  }
  bad("manifest", "the manifest changed", /couldn't be installed/i, (u) => /\/packages\/[0-9a-f]{64}$/.test(u), flip);
  bad("blob", "one file's bytes changed", /doesn't match what its maker signed/i, (u) => /\/blobs\//.test(u), flip);
  add("game-by-name", 20, gameByName);
  add("game-bad-name", 20, gameBadName);
  if (madeUpHere) {
    const { id, hashes } = madeUpFiles();
    add("game-tampered-blob", 10, (S) => gameTampered(S, "blob"), { source: broken((u) => new URL(u).pathname.endsWith(`/blobs/${hashes.get("play/page.json")}`), flip) });
    add("game-tampered-manifest", 10, (S) => gameTampered(S, "manifest"), { source: broken((u) => new URL(u).pathname.endsWith(`/packages/${id}`), flip) });
  } else skips.push(`skip  no ${MADE_UP} in the registry folder (scripts/web/test-registry.sh makes one): a tampered second game isn't checked`);
}
add("share-link", 25, (S) => shareSection(S, roms[0] ?? null), roms.length ? { rom: roms[0].sha } : {});
// The game with its own pictures (own-game*): it plays with no ROM, and a
// tampered play/ file or manifest is refused.
if (ownHere && !registryStopped) {
  const { id, hashes } = ownFiles();
  add("own-game", 25, ownGame);
  add("own-game-tampered-blob", 10, (S) => ownGameTampered(S, "blob"), { source: broken((u) => new URL(u).pathname.endsWith(`/blobs/${hashes.get("play/bundle.json")}`), flip) });
  add("own-game-tampered-manifest", 10, (S) => ownGameTampered(S, "manifest"), { source: broken((u) => new URL(u).pathname.endsWith(`/packages/${id}`), flip) });
} else if (!ownHere) skips.push(`skip  own-game: no ${OWN} in the registry folder (scripts/web/test-registry.sh makes one when Moss Hop's pictures are lifted): a game with its own pictures isn't checked`);
// The arcade game by its name (arcade-*), each in its own section: it plays
// from its zip, and the zips its lift refuses are refused in words. Left out
// with a line without the game in the registry folder or its zip.
{
  const zip = findZip("tapper");
  const garbled = findZip("rbtapper");
  if (!arcadeHere) skips.push(`skip  arcade-play, arcade-refused, arcade-notice: no ${ARCADE} in the registry folder (publish one into a copy of the registry; docs/web.md, "Arcade games")`);
  else {
    if (zip) add("arcade-play", 30, (S) => arcadePlay(S, { zip }), { arcade: true, rom: "tapper.zip" });
    else skips.push("skip  arcade-play: no tapper.zip in $GP_ROM_DIR/arcade: the arcade game isn't played");
    if (zip) add("arcade-sound", 20, (S) => arcadeSound(S, { zip }), { arcade: true, rom: "tapper.zip", gesture: "safari" });
    else skips.push("skip  arcade-sound: no tapper.zip in $GP_ROM_DIR/arcade: the arcade game's sound isn't played");
    add("arcade-refused", 20, (S) => arcadeRefused(S, { zip }), { arcade: true, rom: "tapper.zip" });
    add("arcade-notice", 10, arcadeNoticeSection, { arcade: true });
  }
  if (!unplacedHere) skips.push(`skip  arcade-unplaced: no ${UNPLACED} in the registry folder (scripts/web/test-registry.sh makes one when the registry has ${ARCADE})`);
  else if (!garbled) skips.push("skip  arcade-unplaced: no rbtapper.zip in $GP_ROM_DIR/arcade: the refusal of a set not drawn right yet isn't checked");
  else add("arcade-unplaced", 15, (S) => arcadeUnplaced(S, { garbled }), { arcade: true, rom: "rbtapper.zip" });
}

if (opt("only") || acceptPictures) {
  const re = new RegExp(opt("only") ?? "^pictures-");
  const left = SECTIONS.filter((sec) => re.test(sec.name));
  SECTIONS.splice(0, SECTIONS.length, ...left);
  console.log(`check-page: --only ${opt("only")}: ${left.map((sec) => sec.name).join(", ") || "no section"}`);
}

// ---- Reuse: a section's result, kept with the files it read ----

const sha256 = (b) => createHash("sha256").update(b).digest("hex");
const digests = new Map();
// An input as kept: "file bundle:PATH" for a file of the bundle, "file
// repo:PATH" for one of the repo (so a store shared by worktrees reads
// each one's own), else "file " and the absolute path.
const keptName = (abs) => {
  const r = relative(repo, abs);
  return r.startsWith("..") ? `file ${abs}` : `file repo:${r}`;
};
const pathOf = (kept) => {
  const p = kept.replace(/^file /, "");
  if (p.startsWith("bundle:")) return join(dist, p.slice(7));
  if (p.startsWith("repo:")) return join(repo, p.slice(5));
  return p;
};
const digestOf = (kept) => {
  if (!digests.has(kept)) {
    let d = "missing";
    try {
      const p = pathOf(kept);
      if (statSync(p).isFile()) d = sha256(readFileSync(p));
    } catch {}
    digests.set(kept, d);
  }
  return digests.get(kept);
};
const extraOf = (sec) => [`section ${sec.name}`, `browser ${browserPath}`, `playwright-core ${playwrightVersion}`, `rom ${sec.rom ?? "none"}`].join("\n");
// Read by every section: the check itself, and the bundle's files the
// check reads without a browser.
const always = [keptName(fileURLToPath(import.meta.url)), "file bundle:page.json", "file bundle:_headers"];

function findKept(sec) {
  const d = join(store, sec.name);
  let entries = [];
  try {
    entries = readdirSync(d)
      .filter((n) => !n.endsWith(".part"))
      .map((n) => ({ n, t: statSync(join(d, n)).mtimeMs }))
      .sort((a, b) => b.t - a.t)
      .slice(0, 40);
  } catch {
    return null;
  }
  const extra = extraOf(sec);
  for (const { n } of entries) {
    let meta;
    try {
      meta = JSON.parse(readFileSync(join(d, n, "inputs.json"), "utf8"));
    } catch {
      continue;
    }
    if (meta.extra !== extra) continue;
    if (Object.entries(meta.inputs).every(([k, v]) => digestOf(k) === v)) return { dir: join(d, n), meta };
  }
  return null;
}

function keep(sec, S, seconds) {
  const inputs = {};
  for (const k of [...always, ...(sec.files ?? []).map(keptName), ...[...S.read].map((r) => `file bundle:${r}`)]) inputs[k] = digestOf(k);
  const extra = extraOf(sec);
  const key = sha256(extra + JSON.stringify(Object.entries(inputs).sort())).slice(0, 32);
  const final = join(store, sec.name, key);
  if (existsSync(final)) return;
  const work = `${final}.${process.pid}.part`;
  mkdirSync(join(work, "shots"), { recursive: true });
  for (const f of new Set(S.files)) if (existsSync(join(shots, f))) copyFileSync(join(shots, f), join(work, "shots", f));
  const meta = { kind: `page-check/${sec.name}`, extra, made: new Date().toLocaleString("sv").replace(" ", "T"), seconds, requests: S.requests.length, lines: S.lines, inputs };
  writeFileSync(join(work, "inputs.json"), JSON.stringify(meta, null, 0));
  try {
    renameSync(work, final);
  } catch {}
}

// ---- Run ----

const failures = [];
const results = new Map();
let exit = 0;
let browser;
const t00 = Date.now();
try {
  const todo = [];
  for (const sec of SECTIONS) {
    const kept = full || (acceptPictures && sec.name.startsWith("pictures-")) ? null : findKept(sec);
    if (kept) {
      for (const f of readdirSync(join(kept.dir, "shots"))) copyFileSync(join(kept.dir, "shots", f), join(shots, f));
      results.set(sec.name, { kept: kept.meta });
    } else todo.push(sec);
  }
  if (todo.length) {
    browser = await chromium.launch({ executablePath: browserPath, args: ["--no-sandbox", "--autoplay-policy=no-user-gesture-required"] });
    const queue = [...todo].sort((a, b) => b.est - a.est);
    const worker = async () => {
      for (let sec; (sec = queue.shift()); ) {
        const t0 = Date.now();
        // A section that throws (a timeout under load, a frame replaced
        // while it was read) is run once more in a fresh context, and its
        // lines say so; a second throw fails it with its error printed.
        let S;
        let error = null;
        let retried = null;
        for (let attempt = 0; attempt < 2; attempt++) {
          if (attempt) console.log(`retry ${sec.name}: it threw ${String(error).split("\n")[0]}`);
          ({ S, error } = await runOnce(sec));
          if (!error) break;
          if (!attempt) retried = error;
        }
        if (retried) S.lines.unshift(`note  retried after: ${String(retried.stack ?? retried).split("\n").slice(0, 3).join(" | ")}`);
        const seconds = (Date.now() - t0) / 1000;
        console.log(`time  ${sec.name}: ${seconds.toFixed(1)} s${error || S.failures.length ? " (failed)" : ""}${retried ? " (retried)" : ""}`);
        results.set(sec.name, { S, error, seconds, retried });
        if (!error && !S.failures.length) keep(sec, S, seconds);
      }
    };
    const runOnce = async (sec) => {
        const S = await session(browser, sec.name, sec);
        let error = null;
        try {
          await sec.run(S);
          // The network log of every frame of the section.
          const origin = S.origin;
          const off = S.requests.filter((r) => !(r.url.startsWith(origin) || isSourceUrl(origin, r.url) || r.url.startsWith("blob:") || r.url.startsWith("data:")));
          const posts = S.requests.filter((r) => r.method !== "GET" || r.body > 0);
          S.check(off.length === 0, `every request is to the page's own server or its source (${S.requests.length} requests; others: ${off.map((r) => r.url).join(", ")})`);
          S.check(posts.length === 0, `every request is a GET with no body (${posts.map((r) => `${r.method} ${r.url}`).join(", ")})`);
          S.check(S.sockets.length === 0, "no WebSocket opens");
          // An arcade model's drawing (2.4 MB) is fetched for an arcade game only.
          if (!sec.arcade) S.check(!boardFetched(S), "no request for gp_board.wasm (only an arcade game's board art needs it)");
          if (!sec.arcade) S.check(!arcadePlayFetched(S), "no request for gp_arcade_play.wasm (only an arcade game's sound needs it)");
          if (rules.length) S.check(S.violations.length === 0, `the policy refused nothing (${S.violations.length}${S.violations[0] ? ": " + S.violations[0].slice(0, 200) : ""})`);
          writeFileSync(S.file(`requests-${sec.name}.txt`), S.requests.map((r) => `${r.method} ${r.body} ${r.url.replace(origin, "")}`).join("\n") + "\n");
        } catch (e) {
          error = e;
        } finally {
          await S.context.close().catch(() => {});
          S.server.close();
        }
        return { S, error };
    };
    await Promise.all(Array.from({ length: Math.min(jobs, todo.length) }, worker));
  }
} catch (e) {
  console.log(`FAIL  check-page threw: ${e.stack ?? e}`);
  failures.push(String(e));
  exit = 2;
} finally {
  await browser?.close();
}

// The lines, section by section in a fixed order.
let ran = 0;
let reused = 0;
let requests = 0;
for (const sec of SECTIONS) {
  const r = results.get(sec.name);
  if (!r) continue;
  if (r.kept) {
    reused++;
    requests += r.kept.requests;
    console.log(`\n-- ${sec.name}: kept from ${r.kept.made} (${r.kept.seconds.toFixed(0)} s then); none of the ${Object.keys(r.kept.inputs).length} files it read changed`);
    for (const l of r.kept.lines) console.log(l.replace(/^ok {2}/, "kept"));
    continue;
  }
  ran++;
  requests += r.S.requests.length;
  console.log(`\n-- ${sec.name}: ran in ${r.seconds.toFixed(1)} s, read ${r.S.read.size} files of the bundle`);
  for (const l of r.S.lines) console.log(l);
  failures.push(...r.S.failures.map((f) => `${sec.name}: ${f}`));
  if (r.error) {
    // On stdout under the section's heading, so a log of the run holds it.
    console.log(`FAIL  ${sec.name} threw${r.retried ? " again after a retry" : ""}: ${r.error.stack ?? r.error}`);
    failures.push(`${sec.name}: threw ${String(r.error).split("\n")[0]}`);
    exit = 2;
  }
}
console.log();
for (const s of skips) console.log(s);
if (acceptPictures) {
  // Each moment a section played takes its new hash; a pending one keeps
  // its pending line and no hash; a moment no section played is kept.
  const byId = new Map([...pictureResults.values()].flat().map((m) => [m.id, m]));
  let changed = 0;
  const next = pictureMoments.map((m) => {
    const f = byId.get(m.id);
    if (!f || m.pending) return m;
    if (f.sha256 !== m.sha256) changed++;
    return { ...m, size: f.size, sha256: f.sha256, state: f.state };
  });
  const errored = [...results.values()].some((r) => r.error);
  if (errored) console.log("check-page: --accept-pictures: a section threw, so no reference is written");
  else {
    writePictures(next);
    console.log(`check-page: --accept-pictures: ${changed} reference(s) changed of ${byId.size} played, written to ${relative(repo, picturesFile)}`);
  }
}
if (rules.length) {
  const csp = headersFor("/index.html").get("Content-Security-Policy");
  const ok = !!csp && !/unsafe-inline|unsafe-eval/.test(csp.replace("'wasm-unsafe-eval'", ""));
  console.log(`${ok ? "ok  " : "FAIL"}  the page is served under a Content-Security-Policy with no inline or eval allowance`);
  if (!ok) failures.push("the Content-Security-Policy");
} else console.log("skip  no _headers in the bundle, so no policy was applied");
console.log(`check-page: ${SECTIONS.length} sections, ${ran} ran (up to ${jobs} at once) and ${reused} kept; ${requests} requests, each checked in the run of its section; ${((Date.now() - t00) / 1000).toFixed(1)} s`);
if (failures.length) {
  console.log(`check-page: ${failures.length} failed`);
  for (const f of failures) console.log(`  - ${f}`);
} else console.log(`check-page: passed; shots in ${shots}`);
process.exitCode = exit || (failures.length ? 1 : 0);
