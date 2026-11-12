// The mechanic page in a headless browser, on Moss Hop (the repo's own test
// ROM), in a GP_HOME and a games folder made here for the run:
//   node scripts/web/check-mechanic.mjs --gp GP --rom MOSSHOP --wasm WASM --browser BROWSER
//
// `gp roms add` puts the ROM in the run's $GP_ROM_DIR and `gp game new`
// makes the game and seeds its quest log. Then `gp mechanic start` serves
// the page. The check plays some frames, records a clip with a note and
// hands it off, and checks the clip is in the session folder (with its
// state hash, added in the background) and in the page's clip list. It
// runs `gp mechanic say` and checks the words show over the game and in the
// conversation, answers them, and checks the watch panel. Then the three
// tabs: REVIEW REBUILD from the clip's ON THE MODEL (the clip on the model
// in the game window, all three layouts, the strip, the outlines and the
// frame's measures), PLAY THE REBUILD (the preview built on the way, the
// model beside the original, a run recorded there and sealed with Tab into
// the sealed store), and FREE PLAY (the rebuild alone, never the original,
// with its settings; the agent's next words wait until Esc goes back).
// Then `gp mechanic demo` against a stand-in decision server made here,
// watched in the page, and `say --judge`. Last, a second server with no
// player built: the page must say so, in the server's words, instead of
// failing to compile the 404 as wasm. Any error the page throws fails the
// check. With --shots DIR a screenshot of each mode goes to DIR.
//
// With --arcade-rom ZIP (the owner's rbtapper romset) and --arcade-wasm
// WASM (gp_arcade_play.wasm), an arcade run follows: gp roms add and gp
// game new on the romset, then the page on it at 512x480 and 30 frames a
// second with MAME's keys. It drops a coin (5), starts (1), records a clip
// of moves and taps with a note and hands it off, checks the clip's arcade
// log and state hash in the session, that gp mechanic say shows, and that
// REVIEW REBUILD, FREE PLAY and gp mechanic demo say in words why they
// have nothing to play yet. Without the
// two options the arcade run is skipped, as other ROM checks are.
//
// Playwright comes from scripts/web/node_modules (`npm ci --prefix
// scripts/web`), or PLAYWRIGHT_FROM names another package.json whose
// node_modules has it. The browser is the one
// scripts/inspector/fetch-browser.sh fetches.
//
// Exit codes: 0 passed, 1 a check failed, 2 couldn't run.
import { spawn, spawnSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { createServer } from "node:http";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const opt = (n) => {
  const i = process.argv.indexOf(`--${n}`);
  return i < 0 || !process.argv[i + 1] ? null : resolve(process.argv[i + 1]);
};
const gp = opt("gp"), romFile = opt("rom"), wasm = opt("wasm"), browserPath = opt("browser");
const arcadeRom = opt("arcade-rom"), arcadeWasm = opt("arcade-wasm");
// --arcade-games DIR: a games checkout with tapper/ (its roms.toml names
// tapper, rbtapper and the rest), for the ROM set picker, with the sets
// beside --arcade-rom.
const arcadeGames = opt("arcade-games");
// --shots DIR: a screenshot of each mode goes there (never a repo: they
// show the game).
const shots = opt("shots");
if (!gp || !romFile || !wasm || !browserPath) {
  console.error("check-mechanic: give --gp GP --rom MOSSHOP --wasm WASM --browser BROWSER");
  process.exit(2);
}
for (const f of [gp, romFile, wasm, browserPath]) {
  if (!existsSync(f)) { console.error(`check-mechanic: no ${f}`); process.exit(2); }
}
const from = process.env.PLAYWRIGHT_FROM ?? join(here, "package.json");
let chromium;
try {
  ({ chromium } = createRequire(from)("playwright-core"));
} catch (e) {
  console.error(`check-mechanic: no playwright-core beside ${from}: run npm ci --prefix scripts/web (${e.message})`);
  process.exit(2);
}

const t0 = Date.now();
const work = mkdtempSync(join(tmpdir(), "gp-mechanic-page-"));
const home = join(work, "home"), games = join(work, "games");
mkdirSync(games, { recursive: true });
const env = { ...process.env, GP_HOME: home, GP_ROM_DIR: join(home, "roms"), GP_GAMES_DIR: games, GP_PLAY_WASM: wasm, GP_ARCADE_PLAY_WASM: arcadeWasm ?? "" };
const GAME = "moss-hop", MECH = "walk";
const servers = [];
let browser;

function ok(line) { console.log(`ok   ${line}`); }
function fail(line) { throw new Error(line); }

/** Runs gp to the end; its stdout, or a failure with its stderr. */
function gpRun(args, extra = {}) {
  const r = spawnSync(gp, args, { env: { ...env, ...extra }, encoding: "utf8" });
  if (r.status !== 0) fail(`gp ${args.join(" ")} exited ${r.status}: ${r.stderr.trim()}`);
  return r.stdout;
}

/** Starts `gp mechanic start` on a free port; its address once it prints it. */
function serve(extra = {}, game = GAME, mech = MECH, gamesDir = games, more = []) {
  const p = spawn(gp, ["mechanic", "start", game, mech, "--own-server", "--port", "0", "--games", gamesDir, ...more], { env: { ...env, ...extra } });
  servers.push(p);
  let err = "";
  p.stderr.on("data", (d) => { err += d; });
  return new Promise((res, rej) => {
    let out = "";
    const t = setTimeout(() => rej(new Error(`gp mechanic start printed no address: ${out}${err}`)), 20000);
    p.stdout.on("data", (d) => {
      out += d;
      const m = out.match(/http:\/\/127\.0\.0\.1:\d+\//);
      if (m) { clearTimeout(t); res(m[0]); }
    });
    p.on("exit", (c) => { clearTimeout(t); rej(new Error(`gp mechanic start exited ${c}: ${err}`)); });
  });
}

/** Polls `f` until it gives something truthy; that, or a failure naming `what`. */
async function until(what, f, ms = 15000) {
  const end = Date.now() + ms;
  for (;;) {
    const v = await f();
    if (v) return v;
    if (Date.now() > end) fail(`timed out: ${what}`);
    await new Promise((r) => setTimeout(r, 100));
  }
}

/** A screenshot of the page into --shots, when given. */
async function shot(page, name) {
  if (!shots) return;
  mkdirSync(shots, { recursive: true });
  await page.screenshot({ path: join(shots, `${GAME}-${name}.png`), fullPage: true });
}

/** A stand-in decision server on a free port, as Jev's wire format has
 * it: every question is answered with its first choice (the chord "right"
 * for a demo), at 80%. Its address. */
function fakeDecider() {
  const srv = createServer((req, res) => {
    let body = "";
    req.on("data", (d) => { body += d; });
    req.on("end", () => {
      const q = JSON.parse(body).questions;
      const answers = {};
      for (const [k, v] of Object.entries(q)) {
        const names = Object.keys(v.criteria);
        const choice = k === "buttons" ? (names.includes("right_jump") ? "right_jump" : names[0]) : names[0];
        answers[k] = { choice, probabilities: Object.fromEntries(names.map((n) => [n, n === choice ? 0.8 : 0.2 / Math.max(1, names.length - 1)])), answer_confidence: 0.8 };
      }
      const text = JSON.stringify({ model: "laya", answers, usage: {} });
      res.writeHead(200, { "Content-Type": "application/json", "Content-Length": Buffer.byteLength(text), Connection: "close" });
      res.end(text);
    });
  });
  servers.push({ kill: () => srv.close() });
  return new Promise((r) => srv.listen(0, "127.0.0.1", () => r(`http://127.0.0.1:${srv.address().port}`)));
}

const frame = (page) => page.evaluate(() => Number(document.body.dataset.frame ?? -1));
const text = (page, sel) => page.evaluate((s) => document.querySelector(s)?.textContent ?? "", sel);
const session = join(home, "mechanics", GAME, MECH);

async function main() {
  gpRun(["roms", "add", romFile, "--games", games]);
  gpRun(["game", "new", GAME, "--rom", join(home, "roms", "nes", "mosshop.nes"), "--genre", "platformer", "--namespace", "check", "--games", games]);
  const queue = JSON.parse(readFileSync(join(home, "mechanics", GAME, "queue.json"), "utf8"));
  if (!queue.some((r) => r.mechanic === MECH && r.status === "next")) fail(`gp game new seeded no ${MECH} row: ${JSON.stringify(queue)}`);
  ok(`gp game new seeded the quest log: ${queue.length} rows, ${MECH} among them`);

  const url = await serve();
  browser = await chromium.launch({ executablePath: browserPath, args: ["--no-sandbox"] });
  const page = await browser.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(url);
  await until("the page shows the session", async () => (await text(page, "#title")) === `${GAME} / ${MECH}`);
  const f0 = await until("the game plays 60 frames", async () => ((await frame(page)) > 60 ? frame(page) : 0));
  ok(`the page loaded ${GAME} / ${MECH} and plays (frame ${f0})`);

  // Start the game, then record a walk to the right with a jump.
  await page.keyboard.press("Enter");
  await page.waitForTimeout(500);
  await page.keyboard.press("KeyR");
  await until("recording", async () => (await text(page, "#modebadge")) === "RECORDING");
  await page.keyboard.down("ArrowRight");
  await page.waitForTimeout(700);
  await page.keyboard.down("KeyX");
  await page.waitForTimeout(300);
  await page.keyboard.up("KeyX");
  await page.keyboard.up("ArrowRight");
  await page.keyboard.press("KeyR");
  await until("the review panel", async () => (await text(page, "#modebadge")) === "YOUR CLIP");
  const note = "a walk right with a hop";
  await page.fill("#note", note);
  await page.keyboard.press("Enter");
  const clipToml = join(session, "clips", "001.toml");
  await until("clip 1 in the session folder", () => existsSync(clipToml) && existsSync(join(session, "clips", "001.log")));
  const meta = await until("clip 1's state hash", () => {
    const t = readFileSync(clipToml, "utf8");
    return /state = "[0-9a-f]{64}"/.test(t) ? t : null;
  });
  if (!meta.includes(`note = "${note}"`) || !meta.includes('by = "player"')) fail(`clip 1's toml: ${meta}`);
  ok(`clip 1 is in ${session}/clips with its note, path, start and state hash`);
  await until("clip 1 in the page's list", async () => (await text(page, "#clips")).includes(note));
  await until("the hand-off in the conversation", async () => (await text(page, "#talk")).includes(`player: ${note}`));
  ok("the page lists clip 1 and the hand-off turn");

  // The agent answers with a choice; the person picks the second one.
  const said = "Thanks. Is that the full height of a held jump?";
  gpRun(["mechanic", "say", GAME, MECH, said, "--choice", "Yes", "--choice", "No, a tap", "--show", "1"]);
  await until("the agent's words over the game", async () => (await text(page, "#say")).startsWith(said) && !(await page.isHidden("#overlay")));
  await until("the agent's words in the conversation", async () => (await text(page, "#talk")).includes(`agent: ${said}`));
  ok("gp mechanic say shows over the game and in the conversation");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  await until("the answer in the session", () => readFileSync(join(session, "talk.ndjson"), "utf8").includes('"text":"No, a tap"'));
  ok("the answer, picked with the arrows and Enter, is the person's turn");

  // The watch panel: the agent's log, a pinned picture and a problem show
  // beside the game while the person plays.
  gpRun(["mechanic", "log", GAME, MECH, "Measuring the hop in clip 1"]);
  const sheet = join(work, "hop sheet.png");
  // A one-pixel PNG, made here.
  writeFileSync(sheet, Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg==", "base64"));
  gpRun(["mechanic", "pin", GAME, MECH, sheet, "--title", "the hop, frame by frame", "--keep"]);
  gpRun(["mechanic", "problem", GAME, MECH, "--where", "the first floor", "--what", "the hop lands a pixel low"]);
  gpRun(["mechanic", "problem", GAME, MECH, "--n", "1", "--state", "fixed", "--fix", "land on the floor's top row"]);
  await until("the agent's status", async () => (await text(page, "#w-status")).includes("Measuring the hop in clip 1"));
  await until("the pinned picture", async () => (await text(page, "#w-pins")).includes("the hop, frame by frame") && (await page.evaluate(() => document.querySelector("#w-pins img")?.naturalWidth)) === 1);
  await until("the problem row", async () => (await text(page, "#w-problems")).includes("the hop lands a pixel low") && (await text(page, "#w-problems")).includes("fixed"));
  ok("the watch panel shows the agent's status, its pinned picture and its problem table");

  // game-printer's layout beneath the game: two columns, the quest log at
  // half the width, the clips and the conversation in the second; the
  // score, SOUND ON and AGENT'S LOG at the header's right. The agent's log
  // is a panel at the right, opened from the header (or L), on every tab.
  const beneath = await page.evaluate(() => {
    const cols = [...document.querySelectorAll("main > aside > .col")];
    const r = (id) => document.getElementById(id).getBoundingClientRect();
    return {
      cols: cols.length,
      talkIn: cols.findIndex((c) => c.contains(document.getElementById("talk"))),
      queue: r("queue").width / document.querySelector("main > aside").getBoundingClientRect().width,
      header: r("sound").right <= r("log-toggle").left && r("score").left > window.innerWidth / 2,
      tabs: [...document.querySelectorAll("#tabs button")].map((b) => b.textContent).join(","),
    };
  });
  if (beneath.cols !== 2 || beneath.talkIn !== 1 || beneath.queue < 0.4 || !beneath.header || beneath.tabs !== "CLIPS,REFINE,PLAY,THEMES") fail(`the layout: ${JSON.stringify(beneath)}`);
  ok("four tabs (CLIPS, REFINE, PLAY, THEMES), two columns beneath the game, the score, SOUND ON and AGENT'S LOG at the right");
  await page.click("#log-toggle");
  await until("the agent's log panel", async () => !(await page.isHidden("#log-panel")) && (await text(page, "#log-panel")).includes("Measuring the hop in clip 1"));
  const beside = await page.evaluate(() => document.getElementById("log-panel").getBoundingClientRect().left >= document.querySelector("main").getBoundingClientRect().right - 1);
  if (!beside) fail("the agent's log covers the game");
  await page.click("#log-toggle");
  if (!(await page.isHidden("#log-panel"))) fail("AGENT'S LOG didn't close the panel");
  ok("AGENT'S LOG opens the agent's log beside the game and closes it");

  // The quest log takes the gate's scores ($GP_HOME/work/refine/GAME/
  // status.json, which gp model gate writes): the row shows MEASURED and
  // its score line, and the header counts it.
  const refineDir = join(home, "work", "refine", GAME);
  mkdirSync(refineDir, { recursive: true });
  writeFileSync(join(refineDir, "status.json"), JSON.stringify({
    clips: [{ id: `check.${MECH}.001`, mechanic: MECH, clip: 1, start: 0, frames: 60, bars: { together: true, longest_stretch: null }, straight: { of: 60, together: 60, pct: 100.0 }, first_split: null, resync: null, longest_exact: null, draws: null }],
  }));
  const row = await until("the row's gate scores", async () => {
    const r = await page.evaluate((m) => document.querySelector(`#queue li[data-mechanic="${m}"]`)?.textContent ?? "", MECH);
    return r.startsWith("MEASURED") && r.includes("1 of 1 clips meet the bars") ? r : null;
  });
  const measured = await text(page, "#score");
  if (!/^1 \/ \d+ MEASURED$/.test(measured)) fail(`the header's score: ${measured}`);
  ok(`the quest log takes the gate's scores: "${row.slice(0, 80)}", ${measured}`);

  // REVIEW REBUILD, from clip 1's ON THE MODEL: the clip played through
  // the original and the game's model (the skeleton gp game new wrote) as
  // the score plays it, in the game window, both pictures of each frame
  // kept and shown in every layout, with the strip of frames that agree,
  // the outlines and the right margin's frame.
  await page.click("#clips .rv2-open");
  await until("REVIEW REBUILD", async () => (await page.evaluate(() => document.getElementById("tab-refine").classList.contains("on"))));
  const same = await until("the review", async () => {
    const t = await text(page, "#busy-text");
    return /\d+ of \d+ frames the same/.test(t) ? t.match(/\d+ of \d+ frames the same/)[0] : /couldn't|No clips/.test(t) ? fail(t) : null;
  }, 180000);
  const reviewed = join(session, "clips", "001.review", "meta.json");
  if (!existsSync(reviewed)) fail(`the review isn't kept in ${reviewed}`);
  const review = JSON.parse(readFileSync(reviewed, "utf8"));
  if (!Array.isArray(review.diffs) || !Array.isArray(review.drawn)) fail(`the review has no diffs or drawn things: ${Object.keys(review)}`);
  const shown = review.have.indexOf(true);
  if (shown < 0) fail("the review kept no pictures");
  await until("the badge", async () => (await text(page, "#modebadge")) === "REVIEWING THE REBUILD");
  await page.keyboard.press("KeyX"); // A pauses
  await page.evaluate((k) => { const s = document.getElementById("rv-scrub"); s.value = String(k); s.dispatchEvent(new Event("input")); }, shown);
  const lit = () => page.evaluate(() => {
    const c = document.getElementById("game");
    const d = c.getContext("2d").getImageData(0, 0, c.width, c.height).data;
    for (let i = 0; i < d.length; i += 4) if (d[i] || d[i + 1] || d[i + 2]) return true;
    return false;
  });
  for (const v of ["side", "top", "diff"]) {
    await page.selectOption("#rv-layout", v);
    await until(`the ${v} layout drawn`, lit);
    await shot(page, `review-rebuild-${v}`);
  }
  if ((await page.evaluate(() => document.body.dataset.frameSource)) !== "clip+rebuild") fail("REVIEW REBUILD didn't draw the clip beside the rebuild");
  if (await page.isHidden("#rv-strip")) fail("no strip of the frames that agree");
  if (!/Frame \d+ of \d+/.test(await text(page, "#ctx-frame"))) fail(`the right margin: ${await text(page, "#ctx-frame")}`);
  if (!(await text(page, "#rule-card")).includes("moss-hop") && !(await text(page, "#rule-card")).includes(`${MECH} 1`)) fail(`the clip picker: ${await text(page, "#rule-card")}`);
  await page.selectOption("#rv-layout", "side");
  await page.keyboard.press("KeyC");
  if (!(await page.evaluate(() => document.getElementById("rv-boxes").checked))) fail("C didn't outline the entities");
  await page.keyboard.press("ArrowRight");
  await until("a step on", async () => /frame \d+ of/.test(await text(page, "#rv-at")));
  ok(`REVIEW REBUILD shows clip 1 in the game window: ${same}, in all three layouts, with the strip, the outlines and the frame's measures`);

  // PLAY THE REBUILD: the model played live beside the original on the
  // same presses. The first build of the preview takes minutes.
  await page.keyboard.press("KeyR");
  await until("the rebuild playing beside the original", async () => (await text(page, "#modebadge")) === "PLAYING THE REBUILD" && (await page.evaluate(() => document.body.dataset.frameSource)) === "original+rebuild", 900000);
  await page.keyboard.down("ArrowRight");
  await page.waitForTimeout(400);
  await page.keyboard.up("ArrowRight");
  await shot(page, "play-the-rebuild");
  const layout = () => page.evaluate(() => (document.getElementById("game2").hidden ? "one" : "side"));
  const before = await layout();
  await page.keyboard.press("KeyV");
  await until("V changes the layout", async () => (await layout()) !== before || (await text(page, "#toast")).includes("Playing the rebuild:"));
  // Recording beside the rebuild, then sealing the run with Tab.
  await page.keyboard.press("KeyR");
  await until("recording beside the rebuild", async () => (await text(page, "#modebadge")) === "RECORDING BESIDE THE REBUILD");
  await page.keyboard.down("ArrowRight");
  await page.waitForTimeout(500);
  await page.keyboard.up("ArrowRight");
  await page.keyboard.press("KeyR");
  await until("the review panel", async () => (await text(page, "#modebadge")) === "YOUR CLIP");
  await page.keyboard.press("Tab");
  await until("sealed", async () => (await text(page, "#modebadge")) === "SEALED");
  const sealedToml = join(home, "sealed", GAME, "sealed-run", "clips", "001.toml");
  await until("the sealed run in the sealed store", () => existsSync(sealedToml));
  if (existsSync(join(session, "clips", "002.toml"))) fail("the sealed run went into the session");
  await page.keyboard.press("Enter");
  ok("PLAY THE REBUILD plays the model beside the original; a run recorded there and sealed with Tab goes to the sealed store");
  await page.keyboard.press("Escape");
  await until("back to the review", async () => (await text(page, "#modebadge")).startsWith("REVIEW"));

  // FREE PLAY: the rebuild alone, never the original, and the agent's words
  // wait until Esc goes back.
  await page.click("#tab-play");
  await until("FREE PLAY", async () => (await text(page, "#modebadge")) === "FREE PLAY: THE REBUILD", 120000);
  if (!(await page.evaluate(() => document.getElementById("tab-play").classList.contains("on") && new URL(location.href).searchParams.get("tab") === "play"))) fail("PLAY isn't the tab shown or in the address");
  const sources = new Set();
  for (let i = 0; i < 10; i++) {
    sources.add(await page.evaluate(() => document.body.dataset.frameSource));
    await page.waitForTimeout(100);
  }
  if (sources.has("original")) fail(`FREE PLAY showed the original: ${[...sources]}`);
  if (await page.isHidden("#settings")) fail("FREE PLAY has no settings panel");
  await shot(page, "free-play");
  const later = "One more when you're ready.";
  gpRun(["mechanic", "say", GAME, MECH, later]);
  await until("the waiting message", async () => (await text(page, "#busy-text")).includes("Your agent has a message"));
  if (!(await page.isHidden("#overlay"))) fail("the agent's words showed over FREE PLAY");
  ok(`FREE PLAY is the rebuild alone (${[...sources].join(", ")}), with its settings, and the agent's words wait`);
  await page.keyboard.press("Escape");
  await until("the words after Esc", async () => (await text(page, "#say")).startsWith(later) && !(await page.isHidden("#overlay")));
  await until("CLIPS after Esc", async () => page.evaluate(() => document.getElementById("tab-mech").classList.contains("on") && document.getElementById("tab-clips").classList.contains("on")));
  ok("Esc goes back to CLIPS on the original, where the words show");
  await page.keyboard.press("Enter");

  // The decision model at the controller, played as it comes: a stand-in
  // decision server here answers every question with a chord, and gp
  // mechanic demo plays from clip 1's start; the page watches it, then
  // loops the clip it made under the words. Then say --judge: the model's
  // pick shows beside the choices.
  const decide = await fakeDecider();
  // The demo reads the player's place from the game's mapping; the
  // skeleton's is empty, so the check maps Moss Hop's player bytes
  // (templates/mosshop/mapping.toml) as a person would.
  const mapping = join(games, GAME, "mapping.toml");
  writeFileSync(mapping, readFileSync(mapping, "utf8").replace(/^field = \[\]$/m, "") + '\n[[field]]\npath = "entities.player[0].x"\nram = [0x10, 0x11]\n\n[[field]]\npath = "entities.player[0].y"\nram = [0x12, 0x1C]\n');
  const demo = spawn(gp, ["mechanic", "demo", GAME, MECH, "--hint", "hop once", "--times", "1", "--frames", "120", "--every", "6", "--from", "1", "--decide", decide, "--games", games], { env });
  let demoErr = "", demoCode = null;
  demo.stderr.on("data", (d) => { demoErr += d; });
  demo.on("exit", (c) => { demoCode = c; });
  await until("the page watching the demo", async () => {
    if (demoCode !== null && demoCode !== 0) fail(`gp mechanic demo exited ${demoCode}: ${demoErr}`);
    return (await text(page, "#modebadge")) === "WATCHING YOUR AGENT PLAY";
  }, 60000);
  await shot(page, "watching-the-demo");
  await until("the demo's end", () => demoCode !== null, 60000);
  if (demoCode !== 0) fail(`gp mechanic demo exited ${demoCode}: ${demoErr}`);
  await until("the demo's clip under the words", async () => (await text(page, "#say")).includes("The model's clip"), 30000);
  if (!existsSync(join(session, "clips", "002.decisions.ndjson"))) fail("the demo kept no decisions");
  ok("gp mechanic demo plays from clip 1's start and the page watches it, then shows its clip");
  await page.keyboard.press("Enter");
  // Not gpRun: the stand-in server answers from this process, which a
  // spawnSync would hold still.
  const judged = await new Promise((r) => {
    const p = spawn(gp, ["mechanic", "say", GAME, MECH, "Was that a hop?", "--choice", "yes", "--choice", "no", "--show", "2", "--judge", "--decide", decide, "--games", games], { env });
    let err = "";
    p.stderr.on("data", (d) => { err += d; });
    p.on("exit", (c) => r({ c, err }));
  });
  if (judged.c !== 0) fail(`gp mechanic say --judge exited ${judged.c}: ${judged.err}`);
  await until("Laya's pick beside the choices", async () => (await text(page, "#choices")).includes("Laya's pick"));
  await page.keyboard.press("Enter");
  await until("the agreement kept", () => readFileSync(join(session, "talk.ndjson"), "utf8").includes('"agreed":true'));
  ok("say --judge shows the model's pick, and the answer keeps whether the person agreed");
  if (errors.length) fail(`the page threw: ${errors.join("; ")}`);

  // No player built: the page says so instead of a CompileError.
  const bare = await serve({ GP_PLAY_WASM: join(work, "no-player", "gp_play.wasm") });
  const p2 = await browser.newPage();
  const errors2 = [];
  p2.on("pageerror", (e) => errors2.push(String(e)));
  await p2.goto(bare);
  const why = await until("the page's message with no player", () => p2.evaluate(() => document.body.dataset.failed ?? ""));
  if (!why.includes("isn't built: run scripts/web/build-play.sh")) fail(`with no player the page says: ${why}`);
  if (!(await text(p2, "#busy-text")).includes("isn't built")) fail("the message isn't on the page");
  if (/CompileError/.test(why) || errors2.length) fail(`with no player: ${why} ${errors2.join("; ")}`);
  ok("with no player built the page says how to build it");
}

/** The arcade run: the owner's romset in the page (see the head). */
async function arcade() {
  if (!arcadeRom || !arcadeWasm) {
    console.log("skip the arcade run: no --arcade-rom and --arcade-wasm (the owner's rbtapper romset isn't here)");
    return;
  }
  const game = "tapper", mech = "walk";
  gpRun(["roms", "add", arcadeRom, "--games", games]);
  const zip = join(home, "roms", "arcade", arcadeRom.split("/").pop());
  gpRun(["game", "new", game, "--rom", zip, "--genre", "other", "--namespace", "check", "--games", games]);
  ok(`gp roms add and gp game new made ${game} from the romset`);
  const url = await serve({}, game, mech);
  const page = await browser.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(url);
  await until("the arcade page shows the session", async () => (await text(page, "#title")) === `${game} / ${mech}`);
  const shape = await page.evaluate(() => ({ p: document.body.dataset.platform, w: document.getElementById("game").width, h: document.getElementById("game").height }));
  if (shape.p !== "arcade" || shape.w !== 512 || shape.h !== 480) fail(`the arcade page: ${JSON.stringify(shape)}`);
  if (!(await text(page, "#keys")).includes("5 drops a coin")) fail(`the footer: ${await text(page, "#keys")}`);
  const a0 = await frame(page);
  await page.waitForTimeout(1000);
  const a1 = await frame(page);
  if (a1 - a0 < 20 || a1 - a0 > 40) fail(`the arcade page played ${a1 - a0} frames in a second, not about 30`);
  ok(`the arcade page is 512x480 at ${a1 - a0} frames a second, with MAME's keys in the footer`);

  // A coin, start, then a recording of moves and taps.
  await page.keyboard.down("Digit5");
  await page.waitForTimeout(250);
  await page.keyboard.up("Digit5");
  await page.waitForTimeout(1500);
  await page.keyboard.down("Digit1");
  await page.waitForTimeout(250);
  await page.keyboard.up("Digit1");
  await page.waitForTimeout(3000);
  await page.keyboard.press("KeyR");
  await until("recording", async () => (await text(page, "#modebadge")) === "RECORDING");
  for (const k of ["ArrowDown", "KeyX", "ArrowUp", "KeyX", "ArrowLeft"]) {
    await page.keyboard.down(k);
    await page.waitForTimeout(300);
    await page.keyboard.up(k);
    await page.waitForTimeout(150);
  }
  await page.keyboard.press("KeyR");
  await until("the review panel", async () => (await text(page, "#modebadge")) === "YOUR CLIP");
  const note = "a tap at the second bar";
  await page.fill("#note", note);
  await page.keyboard.press("Enter");
  const dir = join(home, "mechanics", game, mech);
  const toml = join(dir, "clips", "001.toml");
  const meta = await until("the arcade clip's state hash", () => {
    if (!existsSync(toml)) return null;
    const t = readFileSync(toml, "utf8");
    return /state = "[0-9a-f]{64}"/.test(t) ? t : null;
  }, 60000);
  if (!meta.includes(`note = "${note}"`)) fail(`the arcade clip's toml: ${meta}`);
  const log = readFileSync(join(dir, "clips", "001.log"), "utf8");
  if (!log.startsWith("gp-arcade-input 0\nsystem rbtapper\ncolumns CSRLDUT\n") || !/T\n/.test(log)) fail(`the arcade clip's log: ${log}`);
  ok("the arcade clip is in the session: an arcade input log with taps, its note, path and state hash");
  await until("the arcade hand-off in the conversation", async () => (await text(page, "#talk")).includes(`player: ${note}`));
  const said = "Got it. Which bar was that?";
  gpRun(["mechanic", "say", game, mech, said]);
  await until("the agent's words over the arcade game", async () => (await text(page, "#say")).startsWith(said) && !(await page.isHidden("#overlay")));
  ok("gp mechanic say shows over the arcade game");
  await page.keyboard.press("Enter");
  await page.click("#clips .rv2-open");
  const says = await until("REVIEW REBUILD's words", async () => {
    const t = await text(page, "#busy-text");
    return t.includes("an arcade game has none yet") ? t : null;
  });
  ok(`REVIEW REBUILD refuses in words: ${says.slice(0, 80)}...`);
  await page.click("#tab-play");
  const free = await until("FREE PLAY's words", async () => {
    const t = (await text(page, "#toast")) + (await text(page, "#state"));
    return t.includes("an arcade game has none yet") ? t : null;
  });
  if (!(await page.evaluate(() => document.getElementById("tab-mech").classList.contains("on")))) fail("the arcade page stayed in FREE PLAY with nothing to play");
  ok(`FREE PLAY refuses in words: ${free.slice(0, 80)}...`);
  const r = spawnSync(gp, ["mechanic", "demo", game, mech, "--hint", "tap", "--games", games], { env, encoding: "utf8" });
  if (r.status === 0 || !r.stderr.includes("arcade game")) fail(`gp mechanic demo on an arcade game: ${r.status} ${r.stderr}`);
  ok("gp mechanic demo refuses, in words, an arcade game whose mapping names no player");
  if (errors.length) fail(`the arcade page threw: ${errors.join("; ")}`);
}

/** One server: gp serve hosts the page at /mechanic/GAME/ (its root
 * opens it), gp mechanic start opens CLIPS of a gp serve already running,
 * REFINE shows gp serve's background work, THEMES the game's themes, and
 * gp mechanic watch opens a session's log at /watch/GAME/TOPIC/. */
async function oneServer() {
  const p = spawn(gp, ["serve", "--port", "0", "--games", games], { env });
  servers.push(p);
  const base = await new Promise((res, rej) => {
    let out = "";
    const t = setTimeout(() => rej(new Error(`gp serve printed no address: ${out}`)), 20000);
    p.stdout.on("data", (d) => {
      out += d;
      const m = out.match(/http:\/\/127\.0\.0\.1:(\d+)\//);
      if (m) { clearTimeout(t); res(m); }
    });
  });
  const port = base[1];
  const opened = gpRun(["mechanic", "start", GAME, MECH, "--port", port, "--games", games]);
  if (!opened.includes(`/mechanic/${GAME}/?tab=clips`)) fail(`gp mechanic start didn't open CLIPS of gp serve: ${opened}`);
  ok("gp mechanic start opens CLIPS of the gp serve running");
  const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  // gp mechanic start opened nothing in a browser, so no game was opened
  // yet: the root lists the games.
  await page.goto(base[0]);
  const listed = await until("gp serve's root lists the games", async () => page.evaluate(() => [...document.querySelectorAll("#games a")].map((a) => a.textContent)));
  if (!listed.includes(GAME)) fail(`gp serve's root lists ${listed}`);
  await page.click(`#games a:text("${GAME}")`);
  await until("the game's page", async () => page.url().includes(`/mechanic/${GAME}/`) && (await text(page, "#title").catch(() => "")) === `${GAME} / ${MECH}`);
  await page.goto(base[0]);
  await until("gp serve's root opens the last game", async () => page.url().includes(`/mechanic/${GAME}/`) && (await text(page, "#title").catch(() => "")) === `${GAME} / ${MECH}`);
  ok(`gp serve's root lists the games (${listed.join(", ")}), then opens the last one opened: ${page.url()}`);
  const picks = await until("the game picker", async () => (!(await page.isHidden("#game-pick")) ? page.evaluate(() => [...document.querySelectorAll("#game-pick option")].map((o) => o.value)) : null));
  if (!picks.includes(GAME)) fail(`the game picker: ${picks}`);
  const last = readFileSync(join(home, "work", "serve", "last-game"), "utf8").trim();
  if (last !== GAME) fail(`gp serve kept ${last} as the last game`);
  ok(`the header picks among the games (${picks.join(", ")}), and gp serve keeps the last one opened for its root`);
  await page.click("#tab-work");
  await until("REFINE's background work", async () => !(await page.isHidden("#pane-work")) && (await page.isHidden("main")) && /workers/.test(await text(page, "#work-body")));
  const holds = await frame(page);
  await page.waitForTimeout(500);
  if ((await frame(page)) !== holds) fail("the game kept playing behind REFINE");
  ok(`REFINE shows gp serve's background work in the game's place, and the game holds: ${(await text(page, "#work-body")).slice(0, 70)}...`);
  await shot(page, "refine");
  await page.click("#tab-themes");
  await until("THEMES", async () => !(await page.isHidden("#pane-themes")) && (await text(page, "#themes-body")).includes("Theme work comes once"));
  ok("THEMES says what's there and what's coming");
  await shot(page, "themes");
  await page.click("#tab-clips");
  await until("CLIPS again", async () => !(await page.isHidden("main")) && (await frame(page)) > holds);
  ok("CLIPS plays the original again");
  const w = gpRun(["mechanic", "watch", GAME, "emulator", "--port", port]);
  if (!w.includes(`/watch/${GAME}/emulator/`)) fail(`gp mechanic watch didn't open gp serve's log page: ${w}`);
  gpRun(["mechanic", "log", GAME, "emulator", "reading the board"]);
  await page.goto(`${base[0]}watch/${GAME}/emulator/`);
  await until("the topic's log", async () => (await text(page, "#w-status")).includes("reading the board"));
  ok("gp mechanic watch opens the session's log on gp serve");
  if (errors.length) fail(`the page under gp serve threw: ${errors.join("; ")}`);
  await page.close();
}

/** The ROM set picker, on Tapper: the games checkout's tapper/ (five sets
 * in its roms.toml) with the owner's sets beside --arcade-rom. The page
 * lists the sets that are here, plays the scored one (tapper) at first,
 * and choosing rbtapper plays it, keeps it for the game and loads the page
 * again; gp serve --rom tapper=FILE chooses it too. */
async function romSets() {
  if (!arcadeRom || !arcadeWasm || !arcadeGames || !existsSync(join(arcadeGames, "tapper", "roms.toml"))) {
    console.log("skip the ROM sets: no --arcade-rom, --arcade-wasm and --arcade-games with tapper/");
    return;
  }
  const games2 = join(work, "games2");
  cpSync(join(arcadeGames, "tapper"), join(games2, "tapper"), { recursive: true, filter: (p) => !p.includes("/target") });
  const romDir = dirname(arcadeRom);
  const RB = "05c721e1f53c60355abc0f38c4db05040b305118630b86e67721ced1bda22657";
  const url = await serve({ GP_ROM_DIR: romDir }, "tapper", "sets", games2);
  const page = await browser.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(url);
  await until("Tapper's page", async () => (await text(page, "#title")) === "tapper / sets");
  const listed = await until("the set picker", async () => (!(await page.isHidden("#rom-pick")) ? page.evaluate(() => [...document.querySelectorAll("#rom-pick option")].map((o) => o.value)) : null));
  if (!listed.includes("rbtapper") || listed.length < 2) fail(`the set picker lists ${listed}`);
  const first = await page.evaluate(() => fetch("api/rom").then((r) => r.json()));
  ok(`the set picker lists the sets here (${listed.join(", ")}), playing ${first.set}`);
  await page.selectOption("#rom-pick", "rbtapper");
  await until("the page again on rbtapper", async () => {
    const r = await page.evaluate(() => fetch("api/rom").then((x) => x.json()).catch(() => null)).catch(() => null);
    return r?.set === "rbtapper" && r?.sha256 === RB && (await text(page, "#title").catch(() => "")) === "tapper / sets";
  }, 30000);
  const f0 = await frame(page);
  await page.waitForTimeout(800);
  if ((await frame(page)) <= f0) fail("the page doesn't play after the set changed");
  const kept = readFileSync(join(home, "mechanics", "tapper", "rom-set"), "utf8").trim();
  if (kept !== "rbtapper") fail(`the set kept for tapper: ${kept}`);
  ok("choosing rbtapper plays it (its identity 05c721e1...), keeps it for the game, and the page plays on");
  await page.close();

  // gp serve --rom tapper=FILE: the set chosen from the command line.
  writeFileSync(join(home, "mechanics", "tapper", "rom-set"), "tapper\n");
  const rbZip = join(romDir, "rbtapper.zip");
  const p = spawn(gp, ["serve", "--port", "0", "--games", games2, "--rom", `tapper=${rbZip}`], { env: { ...env, GP_ROM_DIR: romDir } });
  servers.push(p);
  const base = await new Promise((res, rej) => {
    let out = "", err = "";
    const t = setTimeout(() => rej(new Error(`gp serve printed no address: ${out}${err}`)), 20000);
    p.stderr.on("data", (d) => { err += d; });
    p.stdout.on("data", (d) => {
      out += d;
      const m = out.match(/http:\/\/127\.0\.0\.1:\d+\//);
      if (m) { clearTimeout(t); res(m[0]); }
    });
  });
  const viaServe = await (await fetch(`${base}mechanic/tapper/api/rom`)).json();
  if (viaServe.set !== "rbtapper") fail(`gp serve --rom tapper=rbtapper.zip played ${viaServe.set}`);
  ok("gp serve --rom tapper=FILE plays that set");
  if (errors.length) fail(`the set picker's page threw: ${errors.join("; ")}`);
}

let code = 0;
try {
  await main();
  await oneServer();
  await arcade();
  await romSets();
  console.log(`mechanic-page: passed in ${((Date.now() - t0) / 1000).toFixed(1)} s`);
} catch (e) {
  console.error(`FAIL ${e.message}`);
  code = 1;
} finally {
  await browser?.close().catch(() => undefined);
  for (const p of servers) p.kill();
  rmSync(work, { recursive: true, force: true });
}
process.exit(code);
