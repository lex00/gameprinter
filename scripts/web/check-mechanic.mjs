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
// REVIEW REBUILD plays the clip on the model gp game new wrote, drawn by
// tile kind and saying so, and that FREE PLAY and gp mechanic demo say in
// words why they have nothing to play yet. Without the
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
    // A fresh home builds the game's module from the kit first (about a
    // minute for Tapper's hook crate), so the address can be slow to come.
    const t = setTimeout(() => rej(new Error(`gp mechanic start printed no address: ${out}${err}`)), 240000);
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

/** A screenshot of the page into --shots, when given: the whole page, or
 * what the window shows (`fullPage` false), where fixed panels are. */
async function shot(page, name, fullPage = true) {
  if (!shots) return;
  mkdirSync(shots, { recursive: true });
  await page.screenshot({ path: join(shots, `${GAME}-${name}.png`), fullPage });
}

/** A stand-in decision server on a free port, as Jev's wire format has
 * it: every question is answered with its first choice (the chord "right"
 * for a demo), at 80%. Its address. */
function fakeDecider() {
  const srv = createServer((req, res) => {
    // As serve.mjs answers anything but a POST: gp asks this way whether
    // a decision server is there (crate::laya::probe).
    if (req.method !== "POST") {
      const text = JSON.stringify({ error: "POST /v1/systemone only" });
      res.writeHead(404, { "Content-Type": "application/json", "Content-Length": Buffer.byteLength(text), Connection: "close" });
      res.end(text);
      return;
    }
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

  // The quest log follows the files under it with no click: the session
  // poll redraws a row when its turn changes (the agent's turn makes it
  // YOUR MOVE, the person's WORKING ON IT) and when queue.json is written
  // by anything else.
  const badge = (m) => page.evaluate((x) => document.querySelector(`#queue li[data-mechanic="${x}"] .st`)?.textContent ?? "", m);
  await until("the row back with the agent after the answer", async () => (await badge(MECH)) === "WORKING ON IT");
  gpRun(["mechanic", "say", GAME, MECH, "One more: was that on the first floor?", "--choice", "Yes", "--choice", "No"]);
  await until("the row the person's move, with no click", async () => (await badge(MECH)) === "YOUR MOVE");
  await until("the second question over the game", async () => (await text(page, "#say")).startsWith("One more") && !(await page.isHidden("#overlay")));
  await page.keyboard.press("Enter");
  await until("the row the agent's again, with no click", async () => (await badge(MECH)) === "WORKING ON IT");
  const qf = join(home, "mechanics", GAME, "queue.json");
  const other = JSON.parse(readFileSync(qf, "utf8")).find((r) => r.mechanic !== MECH)?.mechanic;
  if (!other) fail("the quest log has one row only");
  const shownBefore = await badge(other);
  const edited = JSON.parse(readFileSync(qf, "utf8")).map((r) => (r.mechanic === other ? { ...r, status: "asking you" } : r));
  writeFileSync(qf, JSON.stringify(edited, null, 2));
  await until(`${other} redrawn from queue.json, with no click`, async () => (await badge(other)) === "YOUR MOVE");
  ok(`the quest log follows the turns and queue.json with no click: ${MECH} YOUR MOVE then WORKING ON IT, ${other} ${shownBefore} to YOUR MOVE`);

  // A click only opens a row: its status and its conversation stay as
  // they were. HAND TO THE AGENT on the open row hands it over, once.
  const third = JSON.parse(readFileSync(qf, "utf8")).find((r) => r.mechanic !== MECH && r.mechanic !== other && r.status === "next")?.mechanic;
  if (!third) fail("no row at NEXT to open");
  await page.click(`#queue li[data-mechanic="${third}"]`);
  await until(`${third} open`, async () => (await text(page, "#title")) === `${GAME} / ${third}`);
  await page.waitForTimeout(2000); // a poll or more
  const thirdTalk = join(home, "mechanics", GAME, third, "talk.ndjson");
  const turnsOf = () => (existsSync(thirdTalk) ? readFileSync(thirdTalk, "utf8").trim() : "");
  if ((await badge(third)) !== "NEXT" || turnsOf()) fail(`opening ${third} changed it: ${await badge(third)}, talk ${turnsOf()}`);
  await page.click(`#queue li[data-mechanic="${third}"] .qgive`);
  await until(`${third} handed over`, async () => (await badge(third)) === "WORKING ON IT" && turnsOf().includes(`Let's do ${third}`));
  if (await page.$(`#queue li[data-mechanic="${third}"] .qgive`)) fail("HAND TO THE AGENT still offered once handed over");
  await page.click(`#queue li[data-mechanic="${MECH}"]`);
  await until(`${MECH} open again`, async () => (await text(page, "#title")) === `${GAME} / ${MECH}`);
  if (turnsOf().split("\n").length !== 1) fail(`${third}'s talk: ${turnsOf()}`);
  ok(`a click opens ${third} and leaves it NEXT with no turn; HAND TO THE AGENT makes it WORKING ON IT with its one "Let's do" turn`);

  // The background work on a row's clips (gp serve's, as the strip's hub
  // reads it; stood in for here) shows the row WORKING ON IT while it runs.
  const fourth = JSON.parse(readFileSync(qf, "utf8")).find((r) => ![MECH, other, third].includes(r.mechanic) && r.status === "next")?.mechanic;
  if (!fourth) fail("no fourth row at NEXT");
  const bg = `**/api/capture/${GAME}/background`;
  await page.route(bg, (r) => r.fulfill({ contentType: "application/json", body: JSON.stringify({ jobs: [{ kind: "score", clip: `check.${fourth}.001`, name: `${fourth} 001` }], clip_agents: [] }) }));
  await until(`${fourth} WORKING ON IT while a job runs on its clip`, async () => (await badge(fourth)) === "WORKING ON IT");
  await page.unroute(bg);
  await page.route(bg, (r) => r.fulfill({ contentType: "application/json", body: JSON.stringify({ jobs: [], clip_agents: [] }) }));
  await until(`${fourth} NEXT again once the job is done`, async () => (await badge(fourth)) === "NEXT");
  await page.unroute(bg);
  ok(`a background job on ${fourth}'s clip shows it WORKING ON IT, and NEXT again when it ends`);

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

  // A log written to another topic of the game (as agents do with gp
  // mechanic log GAME emulator) shows in the panel too, labelled with its
  // topic, beside the open mechanic's own lines, which are marked; the
  // topic picker shows one topic. The topic is made as agents make one
  // outside the quest log, with gp mechanic watch (on a server of its own
  // here, stopped once it answers).
  await new Promise((res, rej) => {
    const w = spawn(gp, ["mechanic", "watch", GAME, "emulator", "--own-server", "--port", "0"], { env });
    let out = "";
    const t = setTimeout(() => { w.kill(); rej(new Error(`gp mechanic watch printed no address: ${out}`)); }, 20000);
    w.stdout.on("data", (d) => { out += d; if (/watch at http/.test(out)) { clearTimeout(t); w.kill(); res(); } });
    w.on("exit", (c) => { clearTimeout(t); if (!/watch at http/.test(out)) rej(new Error(`gp mechanic watch exited ${c}`)); });
  });
  gpRun(["mechanic", "log", GAME, "emulator", "Booting the emulator core"]);
  gpRun(["mechanic", "problem", GAME, "emulator", "--where", "the title", "--what", "the logo is a line low"]);
  await until("another topic's line in the panel", async () => {
    const lines = await page.$$eval("#w-log li", (ls) => ls.map((l) => [l.querySelector(".topic")?.textContent, l.textContent, l.classList.contains("mine")]));
    const other = lines.find((l) => l[1].includes("Booting the emulator core"));
    const mine = lines.find((l) => l[1].includes("Measuring the hop in clip 1"));
    return other && other[0] === "emulator" && !other[2] && mine && mine[0] === MECH && mine[2];
  });
  await until("another topic's open problem", async () => (await text(page, "#w-problems")).includes("the logo is a line low"));
  if (!(await text(page, "#w-status")).includes("Booting the emulator core")) fail(`the newest line of every topic isn't the status: ${await text(page, "#w-status")} / ${await text(page, "#w-log")}`);
  await page.click("#log-toggle");
  await page.selectOption("#w-topic", MECH);
  await until("the topic picked", async () => {
    const log = await text(page, "#w-log");
    return log.includes("Measuring the hop in clip 1") && !log.includes("Booting the emulator core");
  });
  await page.selectOption("#w-topic", "");
  await until("every topic again", async () => (await text(page, "#w-log")).includes("Booting the emulator core"));
  await shot(page, "log-every-topic");
  await page.click("#log-toggle");
  ok("a log written to another topic shows in the panel with its topic, and the picker shows one topic");

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
  if (/corpus|\(gate/.test(row)) fail(`the quest log says corpus or a tool's note to the person: ${row}`);
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
  await until("the badge", async () => (await text(page, "#modebadge")) === "REVIEWING THE MODEL");
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
  await until("the rebuild playing beside the original", async () => (await text(page, "#modebadge")) === "PLAYING THE MODEL" && (await page.evaluate(() => document.body.dataset.frameSource)) === "original+rebuild", 900000);
  await page.keyboard.down("ArrowRight");
  await page.waitForTimeout(400);
  await page.keyboard.up("ArrowRight");
  await shot(page, "play-the-rebuild");
  const layout = () => page.evaluate(() => (document.getElementById("game2").hidden ? "one" : "side"));
  const before = await layout();
  await page.keyboard.press("KeyV");
  await until("V changes the layout", async () => (await layout()) !== before || (await text(page, "#toast")).includes("Playing the model:"));
  // Recording beside the rebuild, then sealing the run with Tab.
  await page.keyboard.press("KeyR");
  await until("recording beside the rebuild", async () => (await text(page, "#modebadge")) === "RECORDING BESIDE THE MODEL");
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
  await until("FREE PLAY", async () => (await text(page, "#modebadge")) === "FREE PLAY: THE MODEL", 120000);
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
  // REVIEW REBUILD, as on Moss Hop: the clip played through the original
  // and the model gp game new wrote, both pictures of a frame kept and
  // shown in every layout. The model names no picture slot, so its side is
  // drawn by tile kind, and the page says so in a sentence.
  await page.click("#clips .rv2-open");
  await until("Tapper's REVIEW REBUILD", async () => (await page.evaluate(() => document.getElementById("tab-refine").classList.contains("on"))));
  const rv = await until("Tapper's review", async () => {
    const t = await text(page, "#busy-text");
    return /\d+ of \d+ frames the same/.test(t) ? t : /couldn't|No clips|draws nothing yet/.test(t) ? fail(t) : null;
  }, 300000);
  if (!rv.includes("drawn by tile kind")) fail(`REVIEW REBUILD doesn't say the arcade model is drawn by tile kind: ${rv}`);
  const reviewed = join(dir, "clips", "001.review", "meta.json");
  if (!existsSync(reviewed)) fail(`Tapper's review isn't kept in ${reviewed}`);
  const review = JSON.parse(readFileSync(reviewed, "utf8"));
  if (!Array.isArray(review.diffs) || !Array.isArray(review.drawn)) fail(`Tapper's review has no diffs or drawn things: ${Object.keys(review)}`);
  // A frame some way in, where the game is under way, if it has pictures.
  const have = review.have.map((h, k) => (h ? k : -1)).filter((k) => k >= 0);
  if (!have.length) fail("Tapper's review kept no pictures");
  const shown = have[Math.floor(have.length / 2)];
  // The model's one thing, its player, as the board places a sprite: 32
  // by 32 pixels for its 16 by 16 of the game's units (drawn as pixels, as
  // before #28, it was 16 wide). The skeleton maps nothing, so the player
  // stays at its spawn, 0, 0: bytes the board draws below the screen, as
  // the model now does (before #28 it was a box at the top left). On any
  // frame it is drawn, its box is the board's size.
  const heroes = review.drawn.map((d) => (d.boxes ?? []).find((b) => b[4] === "player") ?? null);
  const wrong = heroes.find((b) => b && (b[2] !== 32 || b[3] !== 32));
  if (wrong) fail(`Tapper's player on the model isn't the board's 32 by 32: ${JSON.stringify(wrong)}`);
  const hero = heroes[shown];
  // Both pictures of that frame, by the pixels not in its commonest color
  // (the backdrop): the original's scene (here the attract mode's title)
  // fills a tenth of the screen at least, 24 sprites' worth, in 4 colors
  // or more; the model's is exactly the player's box as far
  // as it is on the screen, or nothing. One lit box can't pass for the
  // original, nor a stray picture for the model.
  const heroArea = hero ? Math.max(0, Math.min(512, hero[0] + 32) - Math.max(0, hero[0])) * Math.max(0, Math.min(480, hero[1] + 32) - Math.max(0, hero[1])) : 0;
  for (const side of ["o", "m"]) {
    const px = await page.evaluate(async (u) => {
      const b = await createImageBitmap(await (await fetch(u)).blob());
      const c = new OffscreenCanvas(b.width, b.height);
      const x = c.getContext("2d");
      x.drawImage(b, 0, 0);
      const d = x.getImageData(0, 0, b.width, b.height).data;
      const count = new Map();
      for (let i = 0; i < d.length; i += 4) {
        const c = (d[i] << 16) | (d[i + 1] << 8) | d[i + 2];
        count.set(c, (count.get(c) ?? 0) + 1);
      }
      const n = d.length / 4;
      return { n, other: n - Math.max(...count.values()), colors: count.size };
    }, `api/clips/1/review/${side}-${String(shown).padStart(5, "0")}.png?mechanic=${mech}`);
    const bad = side === "o" ? px.other < px.n / 10 || px.colors < 4 : px.other !== heroArea;
    if (bad) fail(`Tapper's ${side === "o" ? "original" : "model"} picture at frame ${shown}: ${JSON.stringify(px)}`);
  }
  await until("the badge on Tapper", async () => (await text(page, "#modebadge")) === "REVIEWING THE MODEL");
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
    await until(`Tapper's ${v} layout drawn`, lit);
    if (shots) {
      mkdirSync(shots, { recursive: true });
      await page.screenshot({ path: join(shots, `${game}-review-rebuild-${v}.png`), fullPage: true });
    }
  }
  if ((await page.evaluate(() => document.body.dataset.frameSource)) !== "clip+rebuild") fail("Tapper's REVIEW REBUILD didn't draw the clip beside the rebuild");
  if (await page.isHidden("#rv-strip")) fail("no strip of the frames that agree on Tapper");
  if (!/Frame \d+ of \d+/.test(await text(page, "#ctx-frame"))) fail(`Tapper's right margin: ${await text(page, "#ctx-frame")}`);
  await page.selectOption("#rv-layout", "side");
  await page.keyboard.press("KeyC");
  if (!(await page.evaluate(() => document.getElementById("rv-boxes").checked))) fail("C didn't outline Tapper's entities");
  await page.keyboard.press("ArrowRight");
  await until("a step on Tapper", async () => /frame \d+ of/.test(await text(page, "#rv-at")));
  ok(`REVIEW REBUILD shows Tapper's clip 1: ${rv.match(/\d+ of \d+ frames the same/)[0]}, the original's scene and the model's player where the board draws it (on the screen on ${heroes.filter((b) => b).length} of ${heroes.length} frames) at frame ${shown}, in all three layouts, with the strip, the outlines, the frame's measures and "drawn by tile kind"`);
  await page.click("#tab-play");
  const free = await until("FREE PLAY's words", async () => {
    const t = (await text(page, "#toast")) + (await text(page, "#state"));
    return t.includes("draws nothing yet") ? t : null;
  });
  if (!(await page.evaluate(() => document.getElementById("tab-mech").classList.contains("on")))) fail("the arcade page stayed in FREE PLAY with nothing to play");
  ok(`FREE PLAY refuses in words: ${free.slice(0, 80)}...`);
  // The stand-in decision server, as the Moss Hop run uses: without
  // --decide, demo asks for Laya at 127.0.0.1:7720 before it reads the
  // game, and the check would pass only on a machine with Laya running.
  // Not spawnSync: the stand-in answers from this process.
  const decide = await fakeDecider();
  const r = await new Promise((res) => {
    const p = spawn(gp, ["mechanic", "demo", game, mech, "--hint", "tap", "--games", games, "--decide", decide], { env });
    let stderr = "";
    p.stderr.on("data", (d) => { stderr += d; });
    p.on("exit", (status) => res({ status, stderr }));
  });
  if (r.status === 0 || !r.stderr.includes("whose state mapping doesn't name the player")) fail(`gp mechanic demo on an arcade game: ${r.status} ${r.stderr}`);
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
  // The old inspector's views (mechanic/details/, wave 8): REFINE's THE
  // CLIP MEASURED and the header's DETAILS drawer of the trace inspector,
  // each the editor's view in the page (or, with no editor build, words
  // saying so).
  const measured = await until("REFINE's clip measured", async () => {
    const n = await page.evaluate(() => {
      const f = document.querySelector("#measured iframe.details-frame");
      const doc = f?.contentDocument;
      if (doc?.querySelector("[data-testid=game-view]")) return "the game view";
      return /editor's build/.test(document.querySelector("#measured")?.textContent ?? "") ? "no editor build" : null;
    });
    return n;
  }, 30000);
  ok(`REFINE has THE CLIP MEASURED under the work: ${measured}`);
  await page.click("#details-toggle");
  const drawer = await until("the DETAILS drawer", async () => {
    if (await page.isHidden("#details-drawer")) return null;
    return page.evaluate(() => {
      const doc = document.querySelector("#details-drawer iframe.details-frame")?.contentDocument;
      if (doc && /Trace inspector/i.test(doc.body?.textContent ?? "")) return "the trace inspector";
      return /editor's build/.test(document.querySelector("#details-drawer")?.textContent ?? "") ? "no editor build" : null;
    });
  }, 30000);
  if (!page.url().includes("details=1")) fail(`the drawer isn't in the URL: ${page.url()}`);
  await page.click("#details-drawer .details-head button");
  if (!(await page.isHidden("#details-drawer"))) fail("the DETAILS drawer didn't close");
  ok(`DETAILS opens a drawer with ${drawer} and closes`);
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
  // As the page does: ask whether the game is ready (asking starts making
  // it) and wait for it; its routes would wait too, but the page never asks
  // them before.
  await until("tapper made ready", async () => {
    const r = await (await fetch(`${base}api/mechanic/tapper/ready`)).json();
    if (r.failed) fail(`gp serve couldn't make tapper ready: ${r.failed}`);
    return r.ready;
  }, 120000);
  const romRes = await fetch(`${base}mechanic/tapper/api/rom`);
  if (!romRes.ok) fail(`gp serve --rom: /api/rom answered ${romRes.status}: ${await romRes.text()}`);
  const viaServe = await romRes.json();
  if (viaServe.set !== "rbtapper") fail(`gp serve --rom tapper=rbtapper.zip played ${viaServe.set}`);
  ok("gp serve --rom tapper=FILE plays that set");
  if (errors.length) fail(`the set picker's page threw: ${errors.join("; ")}`);
}

/** A gp serve on a free port with these games; its address. */
async function gpServe(gamesDir) {
  const p = spawn(gp, ["serve", "--port", "0", "--games", gamesDir], { env });
  servers.push(p);
  return new Promise((res, rej) => {
    let out = "";
    const t = setTimeout(() => rej(new Error(`gp serve printed no address: ${out}`)), 20000);
    p.stdout.on("data", (d) => {
      out += d;
      const m = out.match(/http:\/\/127\.0\.0\.1:\d+\//);
      if (m) { clearTimeout(t); res(m[0]); }
    });
  });
}

/** gp mcp on this home, as an agent talks to it: call(tool, args). */
function mcpClient() {
  const p = spawn(gp, ["mcp", "--games", games], { env });
  servers.push(p);
  let buf = "", id = 0;
  const waiting = new Map();
  p.stdout.on("data", (d) => {
    buf += d;
    let i;
    while ((i = buf.indexOf("\n")) >= 0) {
      const line = buf.slice(0, i);
      buf = buf.slice(i + 1);
      try { const m = JSON.parse(line); waiting.get(m.id)?.(m); } catch (e) { /* not a reply */ }
    }
  });
  const send = (m) => p.stdin.write(`${JSON.stringify(m)}\n`);
  const rpc = (method, params) => new Promise((res) => { id += 1; waiting.set(id, res); send({ jsonrpc: "2.0", id, method, params }); });
  const ready = rpc("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "check-mechanic", version: "0" } })
    .then(() => send({ jsonrpc: "2.0", method: "notifications/initialized" }));
  return {
    call: async (name, args) => {
      await ready;
      const r = await rpc("tools/call", { name, arguments: args });
      return r.result?.structuredContent ?? r.result;
    },
  };
}

/** What the agent does shows on the page (ui-vs-mcp.md items 2 to 5): the
 * yes card and its one use, the agent's calls in the log, the change note
 * on PLAY, a point-and-tell turn in the session, and the first screen with
 * no game. */
async function agentOnThePage() {
  const base = await gpServe(games);
  const ready = await until("the game's page is made", async () => (await (await fetch(`${base}api/mechanic/${GAME}/ready`)).json()).ready, 240000);
  if (!ready) fail("the game's page never got ready");
  const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(`${base}mechanic/${GAME}/?tab=play&log=1`);
  await until("the page", async () => (await text(page, "#title").catch(() => "")).startsWith(GAME));
  ok("gp serve answers /api/mechanic/GAME/ready, and the page opens once the game is ready");
  const agent = mcpClient();
  const paid = { command: "capture background", args: { run: GAME, agents: 1 } };
  const asked = await agent.call("gp_run", paid);
  if (!asked.ask_id || !asked.page_open) fail(`paid clip agents weren't asked for: ${JSON.stringify(asked)}`);
  const card = await until("the yes card", async () => (await page.isVisible("#asks .ask-card")) && text(page, "#asks .ask-card"), 15000);
  if (!/clip agent/.test(card) || !/YES/.test(card)) fail(`the card says: ${card}`);
  ok(`gp_run's ask is a card on the page: ${card.slice(0, 80)}...`);
  await shot(page, "yes-card");
  // The card is whole at every width from 360 px, with the log open: its
  // words not cut, YES and NO on the screen and not under the log panel.
  for (const width of [360, 700, 1400]) {
    await page.setViewportSize({ width, height: 800 });
    await page.waitForTimeout(400);
    await page.evaluate(() => document.querySelector("#asks .ask-card").scrollIntoView({ block: "start" }));
    await page.waitForTimeout(200);
    const seen = await page.evaluate(() => {
      const card = document.querySelector("#asks .ask-card");
      const says = card.querySelector(".ask-says");
      const r = card.getBoundingClientRect();
      const hit = (b) => {
        const q = b.getBoundingClientRect();
        const at = document.elementFromPoint(q.left + q.width / 2, q.top + q.height / 2);
        return at === b || b.contains(at);
      };
      return {
        inside: r.left >= -1 && r.right <= innerWidth + 1 && r.top >= -1 && r.bottom <= innerHeight + 1,
        cut: says.scrollWidth > says.clientWidth + 1 || says.scrollHeight > says.clientHeight + 1,
        buttons: [...card.querySelectorAll("button")].map(hit),
        logOpen: document.body.classList.contains("log-open"),
        rect: [r.left, r.top, r.right, r.bottom].map(Math.round),
        window: [innerWidth, innerHeight],
      };
    });
    await shot(page, `yes-card-${width}`, false);
    if (!seen.logOpen) fail(`the log isn't open at ${width} px`);
    if (!seen.inside || seen.cut) fail(`the yes card isn't whole at ${width} px: ${JSON.stringify(seen)}`);
    if (seen.buttons.length !== 2 || seen.buttons.includes(false)) fail(`YES or NO can't be pressed at ${width} px: ${JSON.stringify(seen)}`);
    await page.evaluate(() => scrollTo(0, 0));
    await shot(page, `top-${width}`, false);
  }
  ok("the yes card is whole with YES and NO pressable at 360, 700 and 1400 px, with the log open");
  // The status line wraps its sentences rather than cutting them, beside
  // the card, at a narrow and a wide window; narrow, the next step starts
  // on a line of its own.
  for (const width of [700, 1600]) {
    await page.setViewportSize({ width, height: 1000 });
    await page.waitForTimeout(400);
    const line = await page.evaluate(() => {
      const spans = [...document.querySelectorAll("#next-line > span")];
      const top = (c) => document.querySelector(`#next-line .${c}`)?.getBoundingClientRect().top ?? 0;
      return {
        cut: spans.filter((x) => getComputedStyle(x).textOverflow === "ellipsis" || x.scrollWidth > x.clientWidth + 1).map((x) => x.className),
        ownLine: top("nx-next") > (top("nx-stands") || top("nx-clue")),
        height: document.getElementById("next-line").offsetHeight,
      };
    });
    if (line.cut.length) fail(`the status line cuts ${line.cut.join(", ")} at ${width} px`);
    if (width === 700 && !line.ownLine) fail("at 700 px the next step doesn't start on a line of its own");
    await shot(page, `status-${width}`);
    await page.waitForTimeout(2500);
    const after = await page.evaluate(() => document.getElementById("next-line").offsetHeight);
    if (after < line.height) fail(`the status line shrank from ${line.height} to ${after} px between polls at ${width} px`);
  }
  ok("the status line wraps its sentences beside the card and holds its height between polls; at 700 px the next step has a line of its own");
  const early = await agent.call("gp_run", { ...paid, confirmed_by_person: true });
  if (!early.ask_id || early.exit !== undefined) fail(`confirmed_by_person ran it while the page was open: ${JSON.stringify(early)}`);
  await page.click("#asks .ask-yes");
  await until("the yes recorded", async () => /YOU SAID YES/.test(await text(page, "#asks")));
  const ran = await agent.call("gp_run", { ...paid, ask_id: asked.ask_id });
  if (ran.exit !== 0 || !ran.yes) fail(`the yes didn't run it: ${JSON.stringify(ran)}`);
  const again = await agent.call("gp_run", paid);
  if (!again.ask_id || again.ask_id === asked.ask_id) fail(`a used yes ran it again: ${JSON.stringify(again)}`);
  ok("YES on the card runs that exact call once; confirmed_by_person alone doesn't, and the next run asks again");
  await until("the next ask's card", async () => page.isVisible(`#asks .ask-card[data-ask="${again.ask_id}"] .ask-no`));
  await page.click(`#asks .ask-card[data-ask="${again.ask_id}"] .ask-no`);
  await until("the no recorded", async () => !(await page.isVisible(`#asks .ask-card[data-ask="${again.ask_id}"]`)));
  const no = await agent.call("gp_run", { ...paid, ask_id: again.ask_id });
  if (!/said no/.test(no.refused ?? "")) fail(`a no didn't refuse: ${JSON.stringify(no)}`);
  ok("NO keeps refusing, and says so to the agent");
  // Two calls more: one done, one that fails with its reason.
  await agent.call("gp_run", { command: "progress", args: { template: GAME } });
  await agent.call("gp_run", { command: "corpus show", args: { dir: join(work, "no-such-game") } });
  const logged = await until("the agent's calls in the log", async () => {
    const t = await text(page, "#w-log");
    return /Asked to start the clip agents: waiting for your yes/.test(t) && /Started the clip agents: done/.test(t) && /Asked to start the clip agents: you said no/.test(t)
      && /Checked how far the game is: /.test(t) && /Looked up the game's tests: failed/.test(t) && t;
  });
  if (/\(capture background\)|corpus show\)|clip agents in the background/.test(logged)) fail(`a log line names the command or its help: ${logged}`);
  const shownAt = await page.evaluate(() => document.querySelector("#w-log .at")?.textContent ?? "");
  const local = new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  if (shownAt.slice(0, 2) !== local.slice(0, 2)) fail(`the log's clock says ${shownAt}, the person's is ${local}`);
  ok(`the agent's calls show in the log panel in a few words, at the person's own time: ${logged.replace(/\s+/g, " ").slice(0, 120)}...`);
  await shot(page, "agent-log");
  await agent.call("gp_run", { command: "capture background", args: { run: GAME, workers: 1 } });
  await until("the change note on PLAY", async () => /Your agent changed the game/.test(await text(page, "#next-line")), 20000);
  ok("a change the agent makes shows on PLAY as \"Your agent changed the game.\" with RELOAD");
  await shot(page, "changed");
  const told = await (await fetch(`${base}api/capture/${GAME}/input`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ v: 1, actor: "person", kind: "request", text: "this should bounce", data: { frame: 30, recording: "r1", label: "the player", rect: [10, 20, 16, 16], point: { things: [{ label: "the player" }] } } }),
  })).json();
  if (!/^request:\d+$/.test(told.input?.item ?? "")) fail(`point and tell: ${JSON.stringify(told)}`);
  const shown = gpRun(["mechanic", "show", GAME, told.mechanic]);
  if (!shown.includes("this should bounce [pointing at the player in r1 at frame 30")) fail(`gp mechanic show: ${shown}`);
  ok("point and tell is the person's talk turn with its point, as gp mechanic show prints it");
  if (errors.length) fail(`the page threw: ${errors.join("; ")}`);
  await page.close();
  // The first screen with no game: what Game Printer does, what to bring
  // and what to say, with no terminal command.
  const empty = join(work, "no-games");
  mkdirSync(empty, { recursive: true });
  const first = await gpServe(empty);
  const fp = await browser.newPage({ viewport: { width: 1000, height: 800 } });
  await fp.goto(first);
  const words = await until("the first screen", async () => text(fp, "#first"));
  for (const w of ["plain-text rules file", "your own copy of the game", "What to say to your agent"]) {
    if (!words.toLowerCase().includes(w.toLowerCase())) fail(`the first screen doesn't say ${w}: ${words}`);
  }
  if (/\bgp [a-z]/.test(words)) fail(`the first screen names a command: ${words}`);
  ok("with no game, the first screen says what Game Printer does, what to bring and what to say to the agent");
  await shot(fp, "first-screen");
  await fp.close();
}

/** The old editor's five features on the page (BRIEF-editor-features.md,
 * issue #2), on Moss Hop with main()'s clip in the game's tests: the
 * fidelity number in REFINE's first line and on CLIPS's strip, the same
 * number `next` gives; CHECK THE TESTS on REFINE; TELL YOUR AGENT on a
 * queue row; the queue AS PICTURES; LET THE TOOLS TRY on the status line. */
async function editorFeatures() {
  gpRun(["corpus", "import", GAME, "--write"]);
  const base = await gpServe(games);
  const api = async (path, body) => {
    const r = await fetch(`${base}${path}`, body === undefined ? {} : { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
    return r.json();
  };
  const bg = await api(`api/capture/${GAME}/background`);
  if (!bg.active?.length) fail(`the game's tests have no clip after corpus import: ${JSON.stringify(bg).slice(0, 200)}`);
  // Scores as the background work leaves them: the first clip 62% together
  // on the model as it stands, the others exact.
  const scores = Object.fromEntries(bg.active.map((id, i) => [id, i === 0
    ? { permille: 620, exact: false, model: bg.model }
    : { permille: 1000, exact: true, model: bg.model }]));
  const stateFile = join(home, "work", "serve", GAME, "background", "state.json");
  const saved = existsSync(stateFile) ? JSON.parse(readFileSync(stateFile, "utf8")) : {};
  writeFileSync(stateFile, JSON.stringify({ ...saved, scores }));
  const n = bg.active.length;
  const pct = Math.round(Math.floor((620 + 1000 * (n - 1)) / n) / 10);
  const page = await browser.newPage({ viewport: { width: 1400, height: 1000 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(`${base}mechanic/${GAME}/?tab=work`);
  const said = await until("the fidelity number on REFINE", async () => {
    const t = await text(page, "[data-testid=fidelity]");
    return t.startsWith(`On average, ${pct}% of a clip plays the same`) && t.includes(`${n - 1} of ${n} clips play the same all the way`) && t;
  }, 30000);
  const hover = await page.getAttribute("[data-testid=fidelity]", "title");
  if (!/Pictures and sound are not measured/.test(hover ?? "")) fail(`the number's hover: ${hover}`);
  const nx = await api(`api/next/${GAME}`);
  if (nx.facts?.scores?.fidelity_permille !== Math.floor((620 + 1000 * (n - 1)) / n)) fail(`next's fidelity_permille: ${JSON.stringify(nx.facts?.scores)}`);
  ok(`REFINE's first line says the fidelity number, as next has it (${nx.facts.scores.fidelity_permille} permille): "${said}"`);
  await shot(page, "fidelity");
  await page.click("#tab-clips");
  const strip = await until("the number on CLIPS's strip", async () => {
    const t = await text(page, "[data-testid=strip-fidelity]");
    return t.startsWith(`Clips: ${pct}% the same as the original`) && t;
  });
  ok(`CLIPS's strip says it short: "${strip}"`);
  await page.click("#tab-work");

  // CHECK THE TESTS on REFINE: gp corpus verify through gp_run (gp serve's
  // fixed table of actions, serve/jobs.rs), what it found in words, and
  // the agent's log line its call leaves.
  await page.click("[data-testid=check-tests]");
  await until("the check running", async () => /Checking the tests: making each clip again/.test(await text(page, "[data-testid=check-says]")));
  const checked = await until("the check's result", async () => {
    const t = await text(page, "[data-testid=check-says]");
    return /^Checked at \d\d:\d\d: all \d+ clips (can be made again from your copy of the game|come out the same)\.$/.test(t) && t;
  }, 300000);
  const rec = await api(`api/capture/${GAME}/do/verify`);
  // The person pressed it: the log says so, and a check that only reads
  // changes nothing, so no note says the agent changed the game.
  if (rec.state !== "done" || rec.command !== "corpus verify") fail(`the check's record: ${JSON.stringify(rec).slice(0, 300)}`);
  await page.click("#log-toggle");
  const before = await api(`api/next/${GAME}`);
  await until("the check in the agent's log, as the person's", async () => /You checked the game's tests: done/.test(await text(page, "#w-log")));
  if (/Checked the game's tests/.test(await text(page, "#w-log"))) fail("the check's log line reads as the agent's");
  await page.click("#log-toggle");
  await page.waitForTimeout(6000); // a poll of the status line or more
  const nowNx = await api(`api/next/${GAME}`);
  if (nowNx.changes !== before.changes || nowNx.page_changes !== before.page_changes) fail(`the check counted a change: ${before.changes}/${before.page_changes} then ${nowNx.changes}/${nowNx.page_changes}`);
  if (/Your agent changed the game/.test(await text(page, "#next-line"))) fail("after the person's check the status line says the agent changed the game");
  ok(`CHECK THE TESTS runs gp corpus verify through gp_run and says what it found: "${checked}"; the log says "You checked the game's tests: done", and no change is counted`);
  await shot(page, "check-tests");
  const nope = await fetch(`${base}api/capture/${GAME}/do/shell`, { method: "POST", body: "{}" });
  if (nope.status !== 404) fail(`an action outside the table answered ${nope.status}`);
  ok("an action outside the page's fixed table is refused");
  // One clip's presses changed under the tests: the check says the tests
  // changed. On plan-style tests a clip's curves are derived, so no clip
  // can come out different; the corpus's content hash is what moves.
  const plan = readFileSync(join(games, GAME, "corpus.toml"), "utf8");
  const first = plan.match(/presses = "([^"]+)"/)?.[1];
  const firstId = plan.match(/id = "([^"]+)"/)?.[1];
  if (!first || !firstId) fail(`no clip's presses in corpus.toml: ${plan.slice(0, 300)}`);
  const pressFile = join(games, GAME, first);
  const kept = readFileSync(pressFile, "utf8");
  if (!kept.includes("right")) fail(`clip 1's presses hold no right to change: ${kept}`);
  writeFileSync(pressFile, kept.replace(/right/g, "left"));
  await page.click("[data-testid=check-tests]");
  await until("the check running again", async () => (await page.getAttribute("[data-testid=check-says]", "data-state")) === "running");
  const bad = await until("the check's finding", async () => {
    const t = await text(page, "[data-testid=check-says]");
    return /^Checked at \d\d:\d\d: all \d+ clips can be made again from your copy of the game, but the game's tests changed since their last version was recorded\. Your agent can look at why\.$/.test(t) && t;
  }, 300000);
  if ((await api(`api/capture/${GAME}/do/verify`)).exit !== 1) fail("the check's record doesn't say a check failed");
  ok(`a clip's presses changed under the tests: "${bad}"`);
  writeFileSync(pressFile, kept);

  // TELL YOUR AGENT on a row of REFINE's queue: the work record (gp agent
  // tells lists it), the conversation (the open mechanic's talk), a step in
  // next for the agent, and the agent's answer under the row.
  const row = `[data-testid=queue] li[data-clip="${firstId}"]`;
  await until("the clip's row in the queue", async () => page.isVisible(`${row} [data-testid=tell-agent]`));
  await page.click(`${row} [data-testid=tell-agent]`);
  if (!(await page.isDisabled(`${row} [data-testid=tell-send]`))) fail("SEND can be pressed with nothing written");
  const words = "the hop is too floaty at the top";
  await page.fill(`${row} [data-testid=tell-text]`, words);
  await page.click(`${row} [data-testid=tell-send]`);
  const told = await until("the tell under the row", async () => {
    const t = await text(page, `${row} [data-testid=told]`);
    return t.includes(`You told your agent: "${words}". Waiting for your agent.`) && t;
  });
  const open = JSON.parse(gpRun(["--json", "agent", "tells", "--run", GAME, "--open"]));
  const tell = (open.tells ?? []).find((t) => t.clip === firstId && t.text === words);
  if (!tell) fail(`gp agent tells --open: ${JSON.stringify(open).slice(0, 400)}`);
  const talk = readFileSync(join(session, "talk.ndjson"), "utf8").trim().split("\n").map((l) => JSON.parse(l)).pop();
  if (talk.by !== "player" || talk.text !== `About clip ${firstId}: ${words}` || talk.tell?.clip !== firstId) fail(`the conversation's last turn: ${JSON.stringify(talk)}`);
  const asks = await api(`api/next/${GAME}`);
  const step = [asks.next, ...(asks.also ?? [])].find((x) => x.says.includes(`what you told it about ${firstId}`));
  if (!step || step.who !== "agent" || !step.how.cli.includes(`--reply-to ${tell.id}`)) fail(`next has no step for the tell: ${JSON.stringify(asks.next)} ${JSON.stringify(asks.also)}`);
  gpRun(["agent", "note", "--run", GAME, "--clip", firstId, "--reply-to", String(tell.id), "lowered the gravity at the top of the hop"]);
  await until("the agent's answer under the row", async () => (await text(page, `${row}`)).includes('Your agent answered: "lowered the gravity at the top of the hop"'));
  const after = await api(`api/next/${GAME}`);
  if ([after.next, ...(after.also ?? [])].some((x) => x.says.includes("what you told it"))) fail("next still has the tell's step once answered");
  ok(`TELL YOUR AGENT on a queue row: "${told}"; in the work record, the conversation and next, and the answer shows under the row`);
  // THE TWO GAMES, MEASURED APART under THE CLIP MEASURED says what the
  // state mapping doesn't carry over once: the start's line and the
  // put-back's line don't both say the same list.
  const apart = await until("THE TWO GAMES, MEASURED APART", async () =>
    page.evaluate(() => {
      const doc = document.querySelector("#measured iframe.details-frame")?.contentDocument;
      const sec = doc?.querySelector("[data-testid=lockstep]");
      if (sec?.querySelector("[data-testid=lock-summary]")) return { text: sec.textContent };
      if (sec?.querySelector(".error")) return { error: sec.querySelector(".error").textContent };
      return /editor's build/.test(document.querySelector("#measured")?.textContent ?? "") ? { none: true } : null;
    }),
  120000);
  if (apart.error) fail(`THE TWO GAMES, MEASURED APART couldn't measure: ${apart.error}`);
  if (apart.none) console.log("skip THE TWO GAMES, MEASURED APART: no editor build");
  else {
    const said = apart.text.match(/Not carried over[^.]*\./g) ?? [];
    const twice = said.filter((s, i) => said.indexOf(s) !== i);
    if (twice.length) fail(`THE TWO GAMES, MEASURED APART says this twice: "${twice[0]}"`);
    ok(`THE TWO GAMES, MEASURED APART ${said.length === 0 ? "has nothing left out of the state to say" : `says what isn't carried over ${said.length === 1 ? "once" : `${said.length} times, each a different list`}`}`);
  }
  await shot(page, "tell");
  await page.click("#tab-clips");
  await until("the tell in the conversation column", async () => (await text(page, "#talk")).includes(`About clip ${firstId}: ${words}`));
  ok("the tell is echoed into the conversation column on CLIPS");
  await page.click("#tab-work");

  // AS PICTURES: every clip of the tests as a picture, grouped by its
  // mechanic, a state word on each, and a click opens it on CLIPS.
  await until("the queue's switch", async () => page.isVisible("[data-testid=view-pictures]"));
  await page.click("[data-testid=view-pictures]");
  const cards = await until("the pictures", async () => {
    const c = await page.$$eval("[data-testid=picture-card]", (cs) => cs.map((x) => [x.dataset.clip, x.querySelector(".rf-state")?.textContent, x.querySelector("img")?.getAttribute("src")]));
    return c.length === n && c;
  });
  if (cards.some((c) => !c[1] || !c[2]?.includes("/picture.png"))) fail(`a picture card without its state or picture: ${JSON.stringify(cards)}`);
  const says = await text(page, "[data-testid=pictures-says]");
  if (!says.startsWith(`${n} clips.`) || !says.includes("Click a picture to open the clip")) fail(`the pictures' count: ${says}`);
  await until("the pictures loaded", async () => page.$$eval("[data-testid=picture-card] img", (is) => is.every((i) => i.complete && i.naturalWidth > 0)), 60000);
  await shot(page, "pictures");
  await page.click(`[data-testid=picture-card][data-clip="${firstId}"]`);
  await until("the clip opened on CLIPS", async () => !(await page.isHidden("main")) && (await page.getAttribute("#tab-clips", "class"))?.includes("on") && (await page.getAttribute("#tab-refine", "class"))?.includes("on"));
  ok(`AS PICTURES shows the ${n} clips, a state word on each (${cards.map((c) => c[1]).join(", ")}): "${says}"; a click opens the clip on CLIPS, on the model`);
  await page.click("#tab-work");
  await page.click("[data-testid=view-list]");

  // LET THE TOOLS TRY on the status line: gp mechanic triage through the
  // same fixed table, free, so no yes card; while it runs the line says
  // so, and after, what it found (or why it couldn't start).
  // A quest item for the tools to try: Moss Hop's player x (its mapping's
  // PLAYER_X) past 120, on a quest log row gp game new made.
  writeFileSync(join(games, GAME, "triage.toml"), '[watch]\npx = 0x10\n\n[[item]]\nrow = "climb"\ngoal = "px >= 120"\ndo = "Walk right along the ground"\nstop = "After a few steps"\n');
  const tn = await until("the tools' step in next", async () => {
    const r = await api(`api/next/${GAME}`);
    return [r.next, ...(r.also ?? [])].some((x) => x.act?.do === "start") && r;
  });
  const triage = [tn.next, ...(tn.also ?? [])].find((x) => x.act?.do === "start");
  if (!triage || triage.act.action !== "triage" || !/LET THE TOOLS TRY/.test(triage.says)) fail(`next has no LET THE TOOLS TRY step: ${JSON.stringify(tn.next)}`);
  if (tn.next.act?.do === "start") {
    await until("LET THE TOOLS TRY on the status line", async () => page.isVisible("[data-testid=tools-try]"));
    await page.click("[data-testid=tools-try]");
  } else {
    // Not the next step here: started as the button starts it.
    await api(`api/capture/${GAME}/do/triage`, {});
  }
  if (await page.isVisible("#asks .ask-card")) fail("LET THE TOOLS TRY asked for a yes");
  const atStart = tn;
  const tried = await until("what the tools found", async () => {
    const r = await api(`api/capture/${GAME}/do/triage`);
    return r.state !== "running" && r;
  }, 900000);
  const shown = await until("the status line says it", async () => text(page, "[data-testid=tools-said]"), 20000);
  if (tried.state === "done" ? !/^The tools tried the quest log: they got (none|\d+ quest items?) on their own, and \d+ quest items? needs? you\.$/.test(shown) : !/^The tools couldn't start: .+\. Your agent can look at why\.$/.test(shown)) fail(`the status line says: ${shown}`);
  if (/\bgp [a-z]/.test(shown)) fail(`the status line names a command: ${shown}`);
  await page.click("#log-toggle");
  if (tried.state === "done") {
    await until("the triage in the agent's log, as the person's", async () => /You sorted the quest log: /.test(await text(page, "#w-log")));
    // The tools changed the quest log at the person's press: the views
    // refresh quietly, with no note that the agent changed the game.
    const after = await until("the person's change counted", async () => {
      const r = await api(`api/next/${GAME}`);
      return r.page_changes > atStart.page_changes && r;
    });
    if (after.changes !== atStart.changes) fail(`the person's triage counted as the agent's change: ${atStart.changes} then ${after.changes}`);
    await page.waitForTimeout(6000);
    if (/Your agent changed the game/.test(await text(page, "#next-line"))) fail("after the person's LET THE TOOLS TRY the status line says the agent changed the game");
  }
  await page.click("#log-toggle");
  ok(`LET THE TOOLS TRY runs gp mechanic triage through gp_run with no yes card, and the status line says: "${shown}"`);
  await shot(page, "tools-try");
  if (errors.length) fail(`the page threw: ${errors.join("; ")}`);
  await page.close();
}

let code = 0;
try {
  await main();
  await oneServer();
  await agentOnThePage();
  await editorFeatures();
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
