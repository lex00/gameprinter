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
// failing to compile the 404 as wasm. A picture the agent proposes for
// a made-up theme of the person's is accepted, put back and rejected on
// the page (issue #4). "What is this?" on the player in REFINE's game
// view: the question's turn names the gp element explain call, the
// check answers from it as the agent, and the answer shows under the box
// in plain words and fits at 360 px (issue #10). The first screen's three choices, a made-up theme
// and a new game from the Moss Hop template made there, each at its step
// with whose turn it is, and the copies with CARRY ON (issue #5); a theme
// beside the original, moment by moment along the game's route (issue #5,
// BESIDE THE ORIGINAL); the tabs following the open copy, on a theme, a
// game from nothing and a port (issue #5). Every tab is checked at 360 px
// wide too: nothing past the right edge, every control in the window, the
// agent's log a full-width panel that closes, REVIEW REBUILD's two
// pictures one over the other. Any error the page throws fails the
// check. With --shots DIR a screenshot of each mode goes to DIR.
//
// With --arcade-rom ZIP (the owner's rbtapper romset) and --arcade-wasm
// WASM (gp_arcade_play.wasm), an arcade run follows: gp roms add and gp
// game new on the romset, then the page on it at 512x480 and 30 frames a
// second with MAME's keys. It drops a coin (5), starts (1), records a clip
// of moves and taps with a note and hands it off, checks the clip's arcade
// log and state hash in the session, that gp mechanic say shows, and that
// REVIEW REBUILD plays the clip on the model gp game new wrote, drawn by
// tile kind and saying so, that FREE PLAY plays the model at 512x480, and
// that gp mechanic demo says in words why it has nothing to play yet.
// Without the
// two options the arcade run is skipped, as other ROM checks are.
//
// Playwright comes from scripts/web/node_modules (`npm ci --prefix
// scripts/web`), or PLAYWRIGHT_FROM names another package.json whose
// node_modules has it. The browser is the one
// scripts/inspector/fetch-browser.sh fetches.
//
// Exit codes: 0 passed, 1 a check failed, 2 couldn't run.
import { spawn, spawnSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { createServer } from "node:http";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { crc32 as zlibCrc32, deflateSync } from "node:zlib";

const here = dirname(fileURLToPath(import.meta.url));
const opt = (n) => {
  const i = process.argv.indexOf(`--${n}`);
  return i < 0 || !process.argv[i + 1] ? null : resolve(process.argv[i + 1]);
};
const gp = opt("gp"), romFile = opt("rom"), wasm = opt("wasm"), browserPath = opt("browser");
const arcadeRom = opt("arcade-rom"), arcadeWasm = opt("arcade-wasm");
// --only arcade: the arcade run alone (it needs no Moss Hop run before it).
// --only journeys: the first screen, the copies and their tabs alone.
const only = process.argv.includes("--only") ? process.argv[process.argv.indexOf("--only") + 1] : null;
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

/** Whether `page` shows the game's own mechanic page, open: its #title
 * names the game and it has the tabs. gp serve's waiting page has the same
 * #title but no tabs, and reloads itself once the game is ready, so a look
 * made then would land in that reload (as on CI's slower machines). */
async function gamePageOpen(page) {
  if (!(await text(page, "#title").catch(() => "")).startsWith(GAME)) return false;
  return Boolean(await page.$("#tabs").catch(() => null)) && !(await page.$("#wait").catch(() => true));
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

/** REVIEW REBUILD's two pictures side by side (#game, #game2): their
 * size and place, whether both are inside the window, and the picture's
 * own size. */
const pair = (page) => page.evaluate(() => {
  const r = (id) => document.getElementById(id).getBoundingClientRect();
  const a = r("game"), b = r("game2");
  return {
    w: [a.width, b.width], h: [a.height, b.height], native: [document.getElementById("game").width, document.getElementById("game").height],
    inside: [a, b].every((q) => q.left >= -1 && q.right <= innerWidth + 1), stacked: b.top >= a.bottom - 1,
    window: [innerWidth, innerHeight],
  };
});
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
  // The logo opens a page this server has: here, a server of the page's
  // own, that is the game's page, not gp serve's first screen.
  const logo = await page.evaluate(() => document.getElementById("logo").href);
  const logoAt = await fetch(logo);
  if (logoAt.status !== 200 || new URL(logo).pathname !== "/") fail(`the logo on gp mechanic start --own-server opens ${logo}: ${logoAt.status}`);
  ok(`the logo on gp mechanic start --own-server opens ${new URL(logo).pathname}, which this server answers (${logoAt.status})`);

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
  // Side by side at 1280x720, both pictures at a whole scale of the
  // game's 256x240, the same size, inside the window (issue #28: they
  // were shown at about half the height the window has room for).
  const sbs = await until("the pair fitted", async () => { const q = await pair(page); return q.w[1] > 0 ? q : null; });
  const k = sbs.w[0] / sbs.native[0];
  if (sbs.w[0] !== sbs.w[1] || sbs.h[0] !== sbs.h[1] || k < 2 || k !== Math.round(k) || sbs.h[0] !== sbs.native[1] * k || !sbs.inside || sbs.stacked) fail(`the pictures side by side at 1280x720: ${JSON.stringify(sbs)}`);
  ok(`side by side at 1280x720, each picture is ${sbs.w[0]}x${sbs.h[0]}, ${k} times the game's, both in the window`);
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
  // Counts the sound the page starts: every buffer source started on an
  // AudioContext, with its channels and rate.
  await page.addInitScript(() => {
    window.__gpSound = { started: 0, channels: 0, rate: 0 };
    const make = BaseAudioContext.prototype.createBufferSource;
    BaseAudioContext.prototype.createBufferSource = function () {
      const src = make.call(this);
      const start = src.start.bind(src);
      src.start = (...a) => {
        const s = window.__gpSound;
        s.started++;
        s.channels = src.buffer?.numberOfChannels ?? 0;
        s.rate = src.buffer?.sampleRate ?? 0;
        return start(...a);
      };
      return src;
    };
  });
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

  // A coin, start, then a recording of moves and taps in play (issue
  // #28: the clip was the title screen). The board boots for about 146
  // frames and drops a coin put in before then, so the coin waits for
  // that; a game's first bar is set up about 140 frames after start.
  await until("the board booted", async () => (await frame(page)) >= 180, 30000);
  await page.keyboard.down("Digit5");
  await page.waitForTimeout(250);
  await page.keyboard.up("Digit5");
  await page.waitForTimeout(1500);
  await page.keyboard.down("Digit1");
  await page.waitForTimeout(250);
  await page.keyboard.up("Digit1");
  const started = await frame(page);
  await until("play under way", async () => (await frame(page)) >= started + 200, 30000);
  // The coin's key started the board's sound (the browser lets a page's
  // audio start on a key): stereo frames at 48 kHz, from the two AYs, and
  // nothing went wrong on the page.
  const snd = await page.evaluate(() => window.__gpSound);
  if (!(snd.started > 30) || snd.channels !== 2 || snd.rate !== 48000) fail(`the arcade page's sound didn't start: ${JSON.stringify(snd)}`);
  if (errors.length) fail(`the arcade page's errors with the sound on: ${errors.join("; ")}`);
  ok(`the arcade page's sound started on the coin's key: ${snd.started} frames of stereo at 48 kHz queued, no page errors`);
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
  // The clip is play, read from the board's RAM: the clip's whole log
  // from power-on recorded in the run's store, and on every frame of the
  // clip the game's mode byte says a game is on and the bartender's bar
  // moves with the presses. The addresses are rbtapper's (games/tapper's
  // labels.toml: mode E3BD, 0 in a game; bar E6F9, the bartender's bar).
  const clipAt = { start: Number(meta.match(/^start = (\d+)/m)?.[1]), frames: Number(meta.match(/^frames = (\d+)/m)?.[1]) };
  const full = join(work, "tapper-clip-1.log");
  writeFileSync(full, Buffer.from(await (await page.request.get(`${url}api/clips/1.log`)).body()));
  const recorded = JSON.parse(gpRun(["--json", "record", "--rom", zip, "--input", full, "--name", "check-tapper-clip-1", "--no-trace", "--no-events"]));
  const rows = JSON.parse(gpRun(["--json", "query", recorded.id, "ram[0xE3BD]", "ram[0xE6F9]", "--frames", `${clipAt.start}..${clipAt.start + clipAt.frames}`, "--rom", zip])).rows;
  const modes = new Set(rows.map((r) => r.entries[0].value)), bars = new Set(rows.map((r) => r.entries[1].value));
  if (!(clipAt.start > 0) || rows.length !== clipAt.frames || modes.size !== 1 || !modes.has(0) || bars.size < 2 || [...bars].some((b) => b > 3)) fail(`Tapper's clip 1 isn't play: frames ${clipAt.start} to ${clipAt.start + clipAt.frames}, mode ${[...modes]}, bar ${[...bars]}`);
  ok(`Tapper's clip 1 is play: on its ${rows.length} frames from frame ${clipAt.start} the board's mode is a game's and the bartender moves between bars ${[...bars].sort().join(", ")}`);
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
  // (the backdrop): the original's scene (here a bar in play) fills a
  // tenth of the screen at least, 24 sprites' worth, in 4 colors
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
  // Tapper's 512x480 pictures side by side at full size in 1280x720.
  const tsb = await until("Tapper's pair fitted", async () => { const q = await pair(page); return q.w[1] > 0 ? q : null; });
  if (tsb.w.some((w) => w !== 512) || tsb.h.some((h) => h !== 480) || !tsb.inside || tsb.stacked) fail(`Tapper's pictures side by side at 1280x720 aren't both 512x480 in the window: ${JSON.stringify(tsb)}`);
  ok("Tapper's two 512x480 pictures show side by side at full size in 1280x720");
  await page.keyboard.press("KeyC");
  if (!(await page.evaluate(() => document.getElementById("rv-boxes").checked))) fail("C didn't outline Tapper's entities");
  await page.keyboard.press("ArrowRight");
  await until("a step on Tapper", async () => /frame \d+ of/.test(await text(page, "#rv-at")));
  ok(`REVIEW REBUILD shows Tapper's clip 1: ${rv.match(/\d+ of \d+ frames the same/)[0]}, the original's scene and the model's player where the board draws it (on the screen on ${heroes.filter((b) => b).length} of ${heroes.length} frames) at frame ${shown}, in all three layouts, with the strip, the outlines, the frame's measures and "drawn by tile kind"`);
  // FREE PLAY plays the arcade game's model in the interpreter preview,
  // keyed as a cabinet (issue #3): the rebuild's frame loads the model,
  // reads the arcade layout and draws a 512x480 picture, never the
  // original. The skeleton names no board art, so the frame draws its
  // scene; a model that names some is drawn by gp_board.wasm.
  await page.click("#tab-play");
  await until("Tapper's FREE PLAY", async () => {
    const t = (await text(page, "#toast")) + (await text(page, "#state"));
    if (/draws nothing yet|couldn't start/.test(t)) fail(`Tapper's FREE PLAY: ${t}`);
    return (await text(page, "#modebadge")) === "FREE PLAY: THE MODEL";
  }, 600000);
  const rb = await until("the arcade model's first frame", async () => {
    const f = page.frames().find((x) => x.url().includes("/rebuild/"));
    const g = f && (await f.evaluate(() => {
      const t = window.__gp;
      if (!t) return null;
      if (t.error) return { error: t.error };
      const c = document.getElementById("game");
      return t.marks["first-frame"] ? { w: c.width, h: c.height, board: t.board ?? null, frame: t.state().frame } : null;
    }).catch(() => null));
    return g;
  }, 120000);
  if (rb.error) fail(`Tapper's model in FREE PLAY: ${rb.error}`);
  if (rb.w !== 512 || rb.h !== 480) fail(`Tapper's model in FREE PLAY draws ${rb.w}x${rb.h}, not 512x480`);
  const fsrc = await page.evaluate(() => document.body.dataset.frameSource);
  if (fsrc === "original") fail("Tapper's FREE PLAY showed the original");
  await shot(page, "tapper-free-play");
  ok(`FREE PLAY plays Tapper's model: a 512x480 picture from the preview (board art ${rb.board ? "drawn" : "not named"}), never the original`);
  await page.keyboard.press("Escape");
  await until("CLIPS after Tapper's FREE PLAY", async () => page.evaluate(() => document.getElementById("tab-mech").classList.contains("on")));
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
  await arcadeOriginalFreePlay(game);
}

/** FREE PLAY of the original on an arcade game, in REFINE's game view
 * (gp serve's editor view, /index.html?embed=game): the board in
 * gp_arcade_play.wasm at 512x480, a coin and a start, a save point the
 * server rebuilds natively to the same hashes, back to it with F8 (a new
 * branch), and a clip from it whose log is the board's. Needs the
 * editor's build (`just editor`); without it the step says so. */
async function arcadeOriginalFreePlay(game) {
  const base = await gpServe(games);
  const index = await fetch(`${base}index.html`);
  if (!index.ok || !(index.headers.get("content-type") ?? "").includes("text/html")) {
    console.log("skip FREE PLAY of the arcade original: no editor build (just editor)");
    return;
  }
  const page = await browser.newPage({ viewport: { width: 1400, height: 1100 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(`${base}index.html?embed=game&run=${game}`);
  await until("the game view's free play button", async () => page.isEnabled("[data-testid=game-free]").catch(() => false), 120000);
  await page.click("[data-testid=game-free]");
  await until("the arcade original in free play", async () => !/Starting the original/.test(await text(page, "[data-testid=free-says]")), 60000);
  const size = await page.evaluate(() => { const c = document.querySelector("[data-testid=game-original]"); return [c.width, c.height]; });
  if (size[0] !== 512 || size[1] !== 480) fail(`the arcade original in free play is ${size.join("x")}, not 512x480`);
  const hint = await text(page, ".free-play");
  if (!hint.includes("5 drops a coin")) fail(`free play on the arcade original doesn't name the cabinet's keys: ${hint}`);
  // The board boots for about 146 of its 30 frames a second before a coin counts.
  await page.waitForTimeout(6500);
  for (const k of ["Digit5", "Digit1"]) {
    await page.keyboard.down(k);
    await page.waitForTimeout(250);
    await page.keyboard.up(k);
    await page.waitForTimeout(1500);
  }
  await page.waitForTimeout(3000);
  await page.keyboard.press("F5");
  const sessions = async () => (await (await fetch(`${base}api/play/free?run=${game}`)).json()).sessions;
  const saved = await until("the arcade save point rebuilt natively", async () => {
    const s = (await sessions())[0];
    const sp = s?.saves?.[0];
    if (sp?.error) fail(`the arcade save point couldn't be rebuilt: ${sp.error}`);
    return sp?.native ? { s, sp } : null;
  }, 120000);
  if (!saved.sp.native.matches) fail(`the arcade save point rebuilt to other hashes: ${JSON.stringify(saved.sp)}`);
  await until("the save point's words on the page", async () => /rebuilt the same/.test(await text(page, "[data-testid=save-points]")), 15000);
  ok(`FREE PLAY of the arcade original: 512x480 with the cabinet's keys, a coin and a start, and save point 1 at frame ${saved.sp.frame} rebuilt natively to the page's hashes`);
  await shot(page, "tapper-free-play-original");
  await page.waitForTimeout(1500);
  await page.keyboard.press("F8");
  await until("a branch from the save point", async () => ((await sessions())[0]?.branches?.length ?? 0) >= 2, 30000);
  await page.click("[data-testid=clip-make]");
  await until("the take recording", async () => /Recording the clip/.test(await text(page, "[data-testid=free-says]")), 15000);
  for (const k of ["ArrowDown", "KeyX", "ArrowUp", "KeyX"]) {
    await page.keyboard.down(k);
    await page.waitForTimeout(300);
    await page.keyboard.up(k);
    await page.waitForTimeout(150);
  }
  await page.click("[data-testid=clip-stop]");
  await until("the arcade clip saved", async () => page.isVisible("[data-testid=clip-saved]"), 30000);
  const clip = JSON.parse(readFileSync(join(home, "play", "free", saved.s.id, "clips", "0001.json"), "utf8"));
  if (!clip.log?.startsWith("gp-arcade-input 0\nsystem rbtapper\ncolumns CSRLDUT\n") || !/T\n/.test(clip.log)) fail(`the arcade free play clip's log: ${clip.log}`);
  if (clip.start?.frame !== saved.sp.frame) fail(`the arcade clip doesn't start at its save point: ${JSON.stringify(clip.start)}`);
  ok(`F8 went back to the save point on a new branch, and a clip from it (${clip.frames} frames) holds the board's log with taps from frame ${clip.start.frame}`);
  if (errors.length) fail(`free play on the arcade original threw: ${errors.join("; ")}`);
  await page.close();
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
  const logo = await page.evaluate(() => document.getElementById("logo").href);
  const logoAt = await fetch(logo);
  if (logoAt.status !== 200 || new URL(logo).pathname !== "/start/") fail(`the logo on gp serve opens ${logo}: ${logoAt.status}`);
  ok(`the logo on gp serve opens the first screen, ${new URL(logo).pathname} (${logoAt.status})`);
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
/** gp serve on a port of its own, with a stand-in decision server as its
 * Laya ($GP_LAYA_URL), so what asks for Laya (LET THE TOOLS TRY through
 * gp_run) doesn't depend on the person's own Laya at 127.0.0.1:7720.
 * CHECK_LAYA_URL (an address where nothing listens, with GP_SERVE_URL one
 * too) shows instead what the page says when no Laya runs. */
let standInLaya;
async function gpServe(gamesDir) {
  standInLaya ??= await fakeDecider();
  const p = spawn(gp, ["serve", "--port", "0", "--games", gamesDir], { env: { ...env, GP_LAYA_URL: process.env.CHECK_LAYA_URL ?? standInLaya } });
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

/** Opens the game's page on gp serve at `base` once the game is ready.
 * Before then gp serve answers with a waiting page that reloads itself when
 * the game is ready, and a read of the page in that reload fails with
 * "Execution context was destroyed". */
async function openGame(page, base, query) {
  await until("the game's page is made", async () => (await (await fetch(`${base}api/mechanic/${GAME}/ready`)).json()).ready, 240000);
  await page.goto(`${base}mechanic/${GAME}/${query}`);
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
  await until("the page", () => gamePageOpen(page), 60000);
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
  await openGame(page, base, "?tab=work");
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

/** "What is this?" (issue #10) in REFINE's game view (THE CLIP MEASURED),
 * on main()'s clip in the game's tests: paused, a box dragged round the
 * player on the original says what it is, and "what is this?" sends the
 * question to the open mechanic's talk with the gp element explain call
 * that answers it. The check, as the agent, runs that call and says what
 * it found with gp mechanic say. The answer shows under the box in plain
 * words naming the element pointed at, and still fits at 360 px. */
async function whatIsThis() {
  const base = await gpServe(games);
  const page = await browser.newPage({ viewport: { width: 1400, height: 1000 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await openGame(page, base, "?tab=work");
  const view = await until("the game view in THE CLIP MEASURED", async () => {
    const h = await page.$("#measured iframe.details-frame");
    const f = h && (await h.contentFrame());
    if (f && (await f.$("[data-testid=point-original]"))) return f;
    return /editor's build/.test(await text(page, "#measured")) ? "none" : null;
  }, 60000);
  if (view === "none") {
    console.log("skip WHAT IS THIS?: no editor build");
    await page.close();
    return;
  }
  // The clip in the view, and the frame it is paused on.
  // (The picker shows with two clips or more; with one, it is the tests'.)
  const picker = await view.$("[data-testid=game-recording]");
  const rec = picker ? await picker.inputValue() : (await (await fetch(`${base}api/capture/${GAME}/background`)).json()).active?.[0];
  // A frame near the clip's end, where the player has walked in (at its
  // first frames the game hasn't placed it yet).
  const at = await view.evaluate(() => {
    const s = document.querySelector("[data-testid=game-scrubber]");
    const f = Math.max(1, Number(s.max) - 10);
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set.call(s, String(f));
    s.dispatchEvent(new Event("input", { bubbles: true }));
    return f;
  });
  if (!rec || !(at > 1)) fail(`the game view's clip and frame: ${rec} ${at}`);
  await until("the view on the frame", async () => Number(await view.inputValue("[data-testid=game-scrubber]")) === at);
  // Where the player is on that frame, as the pointer itself asks: what
  // the whole screen holds, and the original's object the model names.
  const whole = await (await fetch(`${base}api/capture/${GAME}/point?recording=${encodeURIComponent(rec)}&frame=${at}&rect=0,0,256,240`)).json();
  const obj = (whole.original?.entities ?? []).find((e) => e.box && e.box.w > 0 && (e.box.x > 0 || e.box.y > 0));
  const thing = obj && (whole.things ?? []).find((t) => t.what === "entity" && t.kind === obj.kind);
  if (!thing) fail(`nothing of the model's on the screen at frame ${at} of ${rec}: ${JSON.stringify(whole.original?.entities ?? whole).slice(0, 300)}`);
  const { x: bx, y: by, w: bw, h: bh } = obj.box;
  // Drag the box on the original's picture, in its own 256x240 pixels.
  const layer = await view.$("[data-testid=point-original]");
  await layer.scrollIntoViewIfNeeded();
  const r = await layer.boundingBox();
  const px = (x, y) => [r.x + ((x + 0.5) * r.width) / 256, r.y + ((y + 0.5) * r.height) / 240];
  await page.mouse.move(...px(bx, by));
  await page.mouse.down();
  await page.mouse.move(...px(bx + bw - 1, by + bh - 1), { steps: 4 });
  await page.mouse.up();
  const pointed = await until("what the box covers", async () => {
    const t = await view.textContent("[data-testid=tell-pointed]", { timeout: 1000 }).catch(() => "");
    return /^You pointed at /.test(t) ? t : /error|couldn't|can't/i.test(t) ? fail(`the box round the ${thing.label}: ${t}`) : null;
  }, 60000);
  if (!pointed.includes(thing.label)) fail(`the box round ${thing.label} says: ${pointed}`);
  // The question: the person's turn in the open mechanic's talk, with the
  // call that answers it.
  const talkFile = join(session, "talk.ndjson");
  // (A line being written as it is read is left for the next read.)
  const turns = () => readFileSync(talkFile, "utf8").split("\n").flatMap((l) => { try { return [JSON.parse(l)]; } catch { return []; } });
  const before = turns().length;
  await view.click("[data-testid=tell-ask]");
  const q = await until("the question in the talk", () => turns().slice(before).find((t) => t.by === "player" && t.text === "what is this?"));
  const call = q.point?.explain;
  if (q.point?.ask !== "explain" || !/^gp element explain --run \S+ --clip \S+ --log-frame \d+ --box \d+,\d+,\d+,\d+ --side original$/.test(call ?? "")) fail(`the question's turn: ${JSON.stringify(q)}`);
  await until("the wait for the answer under the box", async () => /Waiting for your agent's answer/.test(await view.textContent("[data-testid=tell-answer]").catch(() => "")));
  // The agent answers from the call, as `gp mechanic show` hands it over.
  const found = JSON.parse(gpRun(["--json", ...call.split(" ").slice(1)]));
  const name = found.object?.label ?? found.model?.things?.[0]?.name;
  if (!name) fail(`gp element explain named nothing under the box: ${JSON.stringify(found).slice(0, 400)}`);
  const sprites = found.drawn?.sprites?.length ?? 0;
  const said = `That is ${thing.label}: the model's ${name}, drawn with ${sprites} sprite${sprites === 1 ? "" : "s"} there on this frame.`;
  gpRun(["mechanic", "say", GAME, MECH, said]);
  const answer = await until("the answer under the box", async () => {
    const t = await view.textContent("[data-testid=tell-answer]").catch(() => "");
    return t.includes(said) && t;
  }, 20000);
  if (/\$[0-9A-Fa-f]{2}|0x[0-9a-f]|ram\[|\bgp [a-z]/.test(answer)) fail(`the answer isn't in plain words: ${answer}`);
  ok(`WHAT IS THIS? on ${thing.label} at frame ${at} of ${rec}: the box says "${pointed.trim()}", the question's turn carries ${call.split(" --")[0]}, and the answer shows under the box: "${said}"`);
  await shot(page, "what-is-this");
  // At 360 px wide the answer is whole inside the game view's frame.
  await page.setViewportSize({ width: 360, height: 780 });
  await page.waitForTimeout(1000);
  const fit = await page.evaluate(() => {
    const f = document.querySelector("#measured iframe.details-frame");
    const d = f.contentDocument;
    const a = d.querySelector("[data-testid=tell-answer]");
    const ar = a.getBoundingClientRect(), fr = f.getBoundingClientRect();
    return { frame: [Math.round(fr.left), Math.round(fr.right)], doc: d.documentElement.scrollWidth, inner: f.clientWidth, answer: [Math.round(ar.left), Math.round(ar.right)], shown: ar.width > 0 && ar.height > 0, W: innerWidth };
  });
  if (!fit.shown || fit.frame[0] < -1 || fit.frame[1] > fit.W + 1 || fit.doc > fit.inner + 1 || fit.answer[0] < -1 || fit.answer[1] > fit.inner + 1) fail(`the answer at 360 px: ${JSON.stringify(fit)}`);
  await shot(page, "what-is-this-360");
  ok(`at 360 px the answer is whole inside the game view (${fit.answer[1] - fit.answer[0]} px wide in its ${fit.inner} px frame)`);
  if (errors.length) fail(`the page threw: ${errors.join("; ")}`);
  await page.close();
}

/** A PNG of a disc of one colour on a clear ground, w by h, made here
 * (nothing from a ROM): for the made-up theme of pictureProposals(). */
function discPng(w, h, rgb) {
  const raw = Buffer.alloc((w * 4 + 1) * h);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const at = y * (w * 4 + 1) + 1 + x * 4;
      if ((x - w / 2) ** 2 + (y - h / 2) ** 2 < (w * 0.4) ** 2) raw.set([...rgb, 255], at);
    }
  }
  const chunk = (type, data) => {
    const len = Buffer.alloc(4); len.writeUInt32BE(data.length);
    const crc = Buffer.alloc(4); crc.writeUInt32BE(zlibCrc32(Buffer.concat([Buffer.from(type), data])));
    return Buffer.concat([len, Buffer.from(type), data, crc]);
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4); ihdr.set([8, 6, 0, 0, 0], 8);
  return Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), chunk("IHDR", ihdr), chunk("IDAT", deflateSync(raw)), chunk("IEND", Buffer.alloc(0))]);
}

/** A picture the agent proposes for the person's own theme (issue #4):
 * on Moss Hop with a made-up large-art theme under the home's
 * work/themes/, the agent proposes over MCP (a wrong shape is refused
 * and never reaches the page; applying is refused to it), the card shows
 * the picture now and the new one at game size and enlarged with square
 * pixels, two proposals queue, ACCEPT changes the file, PUT THE OLD ONE
 * BACK puts it back, REJECT with words returns the words to the agent,
 * and the card is whole at 360 px. */
async function pictureProposals() {
  const theme = join(home, "work", "themes", "check-look");
  mkdirSync(join(theme, "theme", "art"), { recursive: true });
  writeFileSync(join(theme, "package.toml"), '[package]\nname = "check/check-look"\nkind = "content"\nversion = "0.1.0"\ntitle = "Check Look"\nlicense = "CC0-1.0"\n');
  writeFileSync(join(theme, "theme", "look.txt"), "gp-look 1\nid check-look\ntitle Check Look\nart art.txt\n");
  writeFileSync(join(theme, "theme", "art.txt"), "gp-art 1\nscale 4\nlimit off\npic frame-0.png art/hero.png 16 16 0 0\n");
  const hero = join(theme, "theme", "art", "hero.png");
  const green = discPng(64, 64, [40, 160, 60]);
  writeFileSync(hero, green);
  const red = join(work, "hero-red.png"), blue = join(work, "hero-blue.png"), wide = join(work, "hero-wide.png");
  writeFileSync(red, discPng(64, 64, [220, 40, 40]));
  writeFileSync(blue, discPng(64, 64, [40, 80, 220]));
  writeFileSync(wide, discPng(128, 64, [220, 40, 40]));
  const base = await gpServe(games);
  await until("the game's page is made", async () => (await (await fetch(`${base}api/mechanic/${GAME}/ready`)).json()).ready, 240000);
  const page = await browser.newPage({ viewport: { width: 1400, height: 1000 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(`${base}mechanic/${GAME}/?tab=themes`);
  await until("the page", () => gamePageOpen(page), 60000);
  const agent = mcpClient();
  const propose = (png, why) => agent.call("gp_run", { command: "theme picture propose", args: { dir: theme, game: GAME, picture: "art/hero.png", png, why } });
  const bad = await propose(wide, "Wider.");
  if (bad.exit !== 1 || !/stretched/.test(bad.json?.refused ?? "")) fail(`a picture of the wrong shape wasn't refused: ${JSON.stringify(bad)}`);
  const one = await propose(red, "A red hero, as you asked.");
  if (one.exit !== 0 || !one.json?.id) fail(`the proposal wasn't made: ${JSON.stringify(one)}`);
  const id1 = one.json.id;
  if (!readFileSync(hero).equals(green)) fail("proposing changed the theme");
  ok(`the agent proposes over gp_run, checked first: a picture of the wrong shape is refused in words (${bad.json.refused.slice(0, 60)}...), and the theme is unchanged until the person answers`);
  const card = `#asks .pic-card[data-picture="${id1}"]`;
  await until("the picture card", async () => page.isVisible(card), 15000);
  const seen = await until("both pictures drawn", () => page.evaluate((sel) => {
    const imgs = [...document.querySelectorAll(`${sel} img`)];
    if (imgs.length !== 4 || imgs.some((i) => !i.complete || !i.naturalWidth)) return null;
    return imgs.map((i) => ({ src: i.src.split("/").pop(), w: i.getBoundingClientRect().width, natural: i.naturalWidth, look: getComputedStyle(i).imageRendering, cls: i.className }));
  }, card));
  const big = seen.filter((i) => i.cls === "pic-big"), small = seen.filter((i) => i.cls === "pic-small");
  if (big.map((i) => i.src).join() !== "old.png,new.png" || small.some((i) => i.w !== 16) || big.some((i) => i.w <= 16 || i.w % 16)) fail(`the pictures on the card: ${JSON.stringify(seen)}`);
  if (seen.some((i) => !/pixelated|crisp-edges/.test(i.look))) fail(`the pictures are smoothed: ${JSON.stringify(seen)}`);
  const words = await text(page, card);
  if (!/YOUR AGENT PROPOSES A PICTURE/.test(words) || !/A red hero, as you asked/.test(words) || !/ACCEPT/.test(words) || !/REJECT/.test(words)) fail(`the card says: ${words}`);
  await until("THEMES lists it", async () => page.isVisible(`#th-pictures .pic-item[data-picture="${id1}"]`));
  ok(`the proposal is a card on the page and on THEMES: now and new, ${small[0].w} px at game size and ${big[0].w} px enlarged with square pixels, the agent's sentence, ACCEPT and REJECT`);
  await shot(page, "picture-card");
  const refusedApply = await agent.call("gp_run", { command: "theme picture accept", args: { id: id1 } });
  if (!/only the person decides/.test(refusedApply.refused ?? "") || !readFileSync(hero).equals(green)) fail(`the agent could apply it: ${JSON.stringify(refusedApply)}`);
  ok("an agent call that applies a proposal is refused: only the person decides");
  const two = await propose(blue, "A blue hero instead.");
  const id2 = two.json?.id;
  if (!id2) fail(`the second proposal: ${JSON.stringify(two)}`);
  await until("the queue count", async () => /1 of 2 waiting/.test(await text(page, card)));
  if (await page.isVisible(`#asks .pic-card[data-picture="${id2}"]`)) fail("the second proposal shows before the first is answered");
  ok("two proposals queue: the card says 1 of 2 waiting and shows the first");
  await page.click(`${card} .pic-accept`);
  await until("ACCEPT applied", async () => readFileSync(hero).equals(readFileSync(red)));
  await until("YOU ACCEPTED", async () => /YOU ACCEPTED/.test(await text(page, card)));
  const heard = await agent.call("next", { game: GAME });
  if (!heard.waits_for_yes?.some((y) => y.id === id1 && y.state === "accepted")) fail(`next doesn't carry the answer: ${JSON.stringify(heard.waits_for_yes)}`);
  ok("ACCEPT puts the new picture in the theme's file, and next carries the answer to the agent");
  await shot(page, "picture-accepted");
  await page.click(`${card} .pic-undo`);
  await until("the old one back", async () => readFileSync(hero).equals(green));
  ok("PUT THE OLD ONE BACK puts the old picture back in the file");
  const card2 = `#asks .pic-card[data-picture="${id2}"]`;
  await until("the second card", async () => page.isVisible(card2));
  for (const width of [360, 1400]) {
    await page.setViewportSize({ width, height: 800 });
    await page.waitForTimeout(300);
    await page.evaluate((sel) => document.querySelector(sel).scrollIntoView({ block: "start" }), card2);
    const fit = await page.evaluate((sel) => {
      const c = document.querySelector(sel);
      const r = c.getBoundingClientRect();
      const out = [...c.querySelectorAll("*")].filter((e) => { const q = e.getBoundingClientRect(); return q.width && (q.left < -1 || q.right > innerWidth + 1); }).map((e) => e.className || e.tagName);
      const hit = (b) => { const q = b.getBoundingClientRect(); const at = document.elementFromPoint(q.left + q.width / 2, q.top + q.height / 2); return at === b || b.contains(at); };
      return { inside: r.left >= -1 && r.right <= innerWidth + 1, out, press: [...c.querySelectorAll("button, input")].map(hit), page: document.documentElement.scrollWidth <= innerWidth + 1 };
    }, card2);
    await shot(page, `picture-card-${width}`, false);
    if (!fit.inside || fit.out.length || fit.press.includes(false) || !fit.page) fail(`the picture card at ${width} px: ${JSON.stringify(fit)}`);
  }
  ok("the picture card is whole at 360 and 1400 px, with its box, ACCEPT and REJECT pressable and nothing past the edge");
  await page.fill(`${card2} .pic-words`, "Keep him green, but give him a hat.");
  await page.click(`${card2} .pic-reject`);
  await until("YOU REJECTED", async () => /YOU REJECTED/.test(await text(page, card2)));
  if (!readFileSync(hero).equals(green)) fail("REJECT changed the theme");
  const told = await agent.call("gp_run", { command: "theme picture list", args: {} });
  const back = told.json?.answered?.find((p) => p.id === id2);
  if (back?.state !== "rejected" || back.words !== "Keep him green, but give him a hat.") fail(`the agent didn't get the words: ${JSON.stringify(told)}`);
  ok(`REJECT keeps the theme, and the person's words reach the agent: "${back.words}"`);
  await until("the read answers leave the cards", async () => !(await page.isVisible("#asks .pic-card")), 15000);
  await shot(page, "picture-themes");
  if (errors.length) fail(`the page threw: ${errors.join("; ")}`);
  await page.close();
}

/** A sound change the agent proposes for the person's copy (issue #4), on
 * a copy of Moss Hop with a made-up tune (one held note, written here,
 * nothing from a ROM): sound_edit proposes by default and its direct
 * write is refused while the page is open, gp progress --next and next
 * list it as waiting for the person, the card plays the tune now and the
 * new one, ACCEPT writes it into the copy, PUT THE OLD ONE BACK puts it
 * back, REJECT leaves the sound as it was with the words reaching the
 * agent, and the card is whole at 360 px. */
async function soundProposals() {
  const agent = mcpClient();
  const forked = await agent.call("template_fork", { template: "mosshop", copy: "check_tune" });
  const copy = join(home, "work", "mcp", "check_tune"), model = join(copy, "model.toml");
  if (!existsSync(model)) fail(`template_fork made no copy: ${JSON.stringify(forked).slice(0, 300)}`);
  writeFileSync(model, readFileSync(model, "utf8").replace("[assets.font]\n", '[assets.music]\nkind = "songs"\nrole = "music.level"\ncapture = { recording = "made", frames = "1..", name = "tune" }\n\n[assets.font]\n'));
  const tune = join(home, "work", "sound", "made0000", "tune");
  mkdirSync(tune, { recursive: true });
  writeFileSync(join(tune, "songs.json"), JSON.stringify({
    format: "gp-songs", version: "0.1.0", region: "ntsc", samples: [],
    instruments: [{ name: "lead", voice: "pulse", role: "lead", volume: [12, 10, 8], duty: [2], pitch: [], notes: 0 }],
    songs: [{ name: "theme", kind: "music", origin: "log", confidence: 0.95, frames: 32, loop_frame: 0, level: "main", notes: [],
      tempo: { frames_per_row: 8, rows_per_pattern: 4 },
      tracks: [{ channel: "pulse1", order: [0], patterns: [{ notes: [{ at: 0, len: 30, pitch: 72, fine: 0, instrument: 0 }] }] }] }],
  }));
  const own = join(copy, "sound", "music.songs.json");
  const base = await gpServe(games);
  await until("the game's page is made", async () => (await (await fetch(`${base}api/mechanic/${GAME}/ready`)).json()).ready, 240000);
  const page = await browser.newPage({ viewport: { width: 1400, height: 1000 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(`${base}mechanic/${GAME}/?tab=clips`);
  await until("the page", () => gamePageOpen(page), 60000);
  const lower = [{ op: "transpose", song: "theme", semitones: -12 }];
  const direct = await agent.call("sound_edit", { copy: "check_tune", ops: lower, write: true, confirmed_by_person: true });
  if (!/ACCEPT/.test(direct?.refused ?? "") || existsSync(own)) fail(`a direct write with the page open: ${JSON.stringify(direct).slice(0, 300)}`);
  const one = await agent.call("sound_edit", { copy: "check_tune", ops: lower, why: "A lower tune, as you asked." });
  const id1 = one?.id;
  if (!id1 || one.proposed !== true || existsSync(own)) fail(`the proposal: ${JSON.stringify(one).slice(0, 300)}`);
  ok("sound_edit proposes by default and the copy is unchanged; its direct write is refused while the page is open, even with confirmed_by_person");
  for (const at of [["--template", GAME], ["--copy", "check_tune"]]) {
    const line = gpRun(["progress", "--next", ...at]);
    if (!line.includes(`Waits for your ACCEPT or REJECT (sound ${id1})`)) fail(`gp progress --next ${at.join(" ")}: ${line}`);
  }
  const told = await agent.call("next", { game: GAME });
  if (!told.waits_for_yes?.some((y) => y.what === "sound" && y.id === id1 && y.state === "open")) fail(`next: ${JSON.stringify(told.waits_for_yes)}`);
  ok("gp progress --next, for the port and for the copy, and the MCP tool next list it as waiting for the person (what: sound)");
  const card = `#asks .snd-card[data-sound="${id1}"]`;
  await until("the sound card", async () => page.isVisible(card), 15000);
  const words = await text(page, card);
  if (!/YOUR AGENT PROPOSES A NEW SOUND/.test(words) || !/A lower tune, as you asked/.test(words) || !/ACCEPT/.test(words) || !/REJECT/.test(words)) fail(`the card says: ${words}`);
  const clips = {};
  for (const which of ["old", "new"]) {
    const r = await fetch(`${base}api/sounds/${id1}/${which}.wav`);
    const b = Buffer.from(await r.arrayBuffer());
    if (r.status !== 200 || r.headers.get("content-type") !== "audio/wav" || b.subarray(0, 4).toString() !== "RIFF") fail(`the ${which} clip: ${r.status} ${r.headers.get("content-type")}`);
    clips[which] = b;
    await page.click(`${card} .snd-play-${which}`);
    await until(`the ${which} clip plays`, () => page.evaluate(([sel, w]) => { const a = document.querySelector(`${sel} audio.snd-${w}`); return a && !a.paused && a.currentTime > 0.05; }, [card, which]), 10000);
  }
  if (clips.old.equals(clips.new)) fail("the two clips are the same");
  const playing = await page.evaluate((sel) => [...document.querySelectorAll(`${sel} audio`)].filter((a) => !a.paused).length, card);
  if (playing !== 1) fail(`${playing} clips play at once`);
  ok("the card plays the tune as it is now and with the change, one at a time, with the agent's sentence, ACCEPT and REJECT");
  await shot(page, "sound-card");
  await page.click(`${card} .pic-accept`);
  await until("ACCEPT applied", async () => existsSync(own) && readFileSync(own, "utf8").includes('"pitch": 60'));
  await until("YOU ACCEPTED", async () => /YOU ACCEPTED/.test(await text(page, card)));
  const heard = await agent.call("next", { game: GAME });
  if (!heard.waits_for_yes?.some((y) => y.id === id1 && y.state === "accepted")) fail(`next doesn't carry the answer: ${JSON.stringify(heard.waits_for_yes)}`);
  ok("ACCEPT writes the lower tune into the copy's songs, and next carries the answer to the agent");
  await page.click(`${card} .pic-undo`);
  await until("the old tune back", async () => !existsSync(own));
  ok("PUT THE OLD ONE BACK puts the copy's tune back as it was");
  const two = await agent.call("sound_edit", { copy: "check_tune", ops: [{ op: "tempo", song: "theme", frames_per_row: 4 }], why: "Twice as fast." });
  const card2 = `#asks .snd-card[data-sound="${two?.id}"]`;
  await until("the second card", async () => page.isVisible(card2), 15000);
  for (const width of [360, 1400]) {
    await page.setViewportSize({ width, height: 800 });
    await page.waitForTimeout(300);
    await page.evaluate((sel) => document.querySelector(sel).scrollIntoView({ block: "start" }), card2);
    const fit = await page.evaluate((sel) => {
      const c = document.querySelector(sel);
      const r = c.getBoundingClientRect();
      const out = [...c.querySelectorAll("*")].filter((e) => { const q = e.getBoundingClientRect(); return q.width && (q.left < -1 || q.right > innerWidth + 1); }).map((e) => e.className || e.tagName);
      const hit = (b) => { const q = b.getBoundingClientRect(); const at = document.elementFromPoint(q.left + q.width / 2, q.top + q.height / 2); return at === b || b.contains(at); };
      return { inside: r.left >= -1 && r.right <= innerWidth + 1, out, press: [...c.querySelectorAll("button, input")].map(hit), page: document.documentElement.scrollWidth <= innerWidth + 1 };
    }, card2);
    await shot(page, `sound-card-${width}`, false);
    if (!fit.inside || fit.out.length || fit.press.includes(false) || !fit.page) fail(`the sound card at ${width} px: ${JSON.stringify(fit)}`);
  }
  ok("the sound card is whole at 360 and 1400 px, its two players, box, ACCEPT and REJECT pressable and nothing past the edge");
  await page.fill(`${card2} .pic-words`, "Keep it at the speed it was.");
  await page.click(`${card2} .pic-reject`);
  await until("YOU REJECTED", async () => /YOU REJECTED/.test(await text(page, card2)));
  if (existsSync(own)) fail("REJECT changed the copy's sound");
  const refused = await agent.call("gp_run", { command: "theme sound accept", args: { id: two.id } });
  if (!/only the person decides/.test(refused?.refused ?? "")) fail(`the agent could answer for the person: ${JSON.stringify(refused)}`);
  const back = (await agent.call("gp_run", { command: "theme sound list", args: {} })).json?.answered?.find((p) => p.id === two.id);
  if (back?.state !== "rejected" || back.words !== "Keep it at the speed it was.") fail(`the agent didn't get the words: ${JSON.stringify(back)}`);
  ok(`REJECT leaves the copy's sound as it was, the agent can't answer for the person, and the words reach it: "${back.words}"`);
  if (errors.length) fail(`the page threw: ${errors.join("; ")}`);
  await page.close();
}

/** A role's sound for a made-up large-art theme of the person's (issue
 * #30), on Moss Hop with one made-up sound role, `hop`, written into its
 * theme kit (Moss Hop has no sound of its own), and made-up square tones:
 * a second take proposed over gp_run is a card that says there is no
 * sound there yet and plays the new one, ACCEPT writes the take and its
 * look.txt line, PUT THE OLD ONE BACK takes both away; a new first file's
 * card plays the sound as it is and the new one, and REJECT's words reach
 * the agent. */
async function roleSoundProposals() {
  const tone = (period) => {
    const n = 8820, b = Buffer.alloc(44 + n * 2);
    b.write("RIFF", 0); b.writeUInt32LE(36 + n * 2, 4); b.write("WAVEfmt ", 8);
    b.writeUInt32LE(16, 16); b.writeUInt16LE(1, 20); b.writeUInt16LE(1, 22); b.writeUInt32LE(44100, 24); b.writeUInt32LE(88200, 28); b.writeUInt16LE(2, 32); b.writeUInt16LE(16, 34);
    b.write("data", 36); b.writeUInt32LE(n * 2, 40);
    for (let i = 0; i < n; i++) b.writeInt16LE(Math.floor(i / period) % 2 ? -3000 : 3000, 44 + i * 2);
    return b;
  };
  const kitDir = join(games, GAME, "themes", "kit");
  mkdirSync(kitDir, { recursive: true });
  const kitFile = join(kitDir, "kit.toml");
  const kit = existsSync(kitFile) ? readFileSync(kitFile, "utf8").replace("\nsound = []\n", "\n") : 'format = "0.1.0"\ngame = "moss-hop"\nmodel = "mosshop"\nscreen = [256, 240]\npixel_aspect = "1"\n';
  writeFileSync(kitFile, `${kit}\n[[sound]]\nrole = "hop"\nkind = "sfx"\ncue = "made up"\nloops = false\ndescription = "a made-up hop"\n`);
  const theme = join(home, "work", "themes", "check-tick"), look = join(theme, "theme", "look.txt");
  mkdirSync(join(theme, "theme", "sound", "roles"), { recursive: true });
  writeFileSync(join(theme, "package.toml"), '[package]\nname = "check/check-tick"\nkind = "content"\nversion = "0.1.0"\ntitle = "Check Tick"\nlicense = "CC0-1.0"\n');
  const lookText = "gp-look 1\nid check-tick\ntitle Check Tick\nart art.txt\nfile sound/roles/hop.wav\n";
  writeFileSync(look, lookText);
  writeFileSync(join(theme, "theme", "art.txt"), "gp-art 1\nscale 4\n");
  const first = join(theme, "theme", "sound", "roles", "hop.wav"), second = join(theme, "theme", "sound", "takes", "hop.2.wav");
  writeFileSync(first, tone(20));
  for (const [f, p] of [["hop-2.wav", 30], ["hop-soft.wav", 45]]) writeFileSync(join(work, f), tone(p));
  const base = await gpServe(games);
  await until("the game's page is made", async () => (await (await fetch(`${base}api/mechanic/${GAME}/ready`)).json()).ready, 240000);
  const page = await browser.newPage({ viewport: { width: 1400, height: 1000 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(`${base}mechanic/${GAME}/?tab=themes`);
  await until("the page", () => gamePageOpen(page), 60000);
  const agent = mcpClient();
  const propose = (take, wav, why) => agent.call("gp_run", { command: "theme sound propose", args: { copy: theme, game: GAME, role: "hop", take, wav: join(work, wav), why } });
  const gap = await propose(3, "hop-2.wav", "A third hop.");
  if (gap.exit !== 1 || !/has no take 2 yet/.test(gap.json?.refused ?? "")) fail(`a take past a gap wasn't refused: ${JSON.stringify(gap).slice(0, 300)}`);
  const one = await propose(2, "hop-2.wav", "A second hop, so it varies.");
  const id1 = one.json?.id;
  if (one.exit !== 0 || !id1 || existsSync(second) || readFileSync(look, "utf8") !== lookText) fail(`the proposal: ${JSON.stringify(one).slice(0, 300)}`);
  ok("the agent proposes a role's second take over gp_run, checked first (a take past a gap refused), and the theme is unchanged until the person answers");
  const card = `#asks .snd-card[data-sound="${id1}"]`;
  await until("the role's card", async () => page.isVisible(card), 15000);
  const words = await text(page, card);
  if (!/Check Tick, the sound "hop" \(take 2\)/.test(words) || !/NO SOUND THERE YET/.test(words) || !/A second hop, so it varies/.test(words)) fail(`the card says: ${words}`);
  if (!(await page.isDisabled(`${card} .snd-play-old`))) fail("the card offers to play a sound the theme hasn't got");
  const clip = await fetch(`${base}api/sounds/${id1}/new.wav`);
  if (clip.status !== 200 || !Buffer.from(await clip.arrayBuffer()).equals(tone(30)) || (await fetch(`${base}api/sounds/${id1}/old.wav`)).status !== 404) fail("the card's clips aren't the take proposed and none");
  await page.click(`${card} .snd-play-new`);
  await until("the new take plays", () => page.evaluate((sel) => { const a = document.querySelector(`${sel} audio.snd-new`); return a && !a.paused && a.currentTime > 0.02; }, card), 10000);
  ok(`the card names the theme and the role's take, says there is no sound there yet, and plays the new take ("${words.slice(0, 80)}...")`);
  await shot(page, "sound-card-role");
  await page.click(`${card} .pic-accept`);
  await until("ACCEPT applied", async () => existsSync(second) && readFileSync(look, "utf8").endsWith("file sound/takes/hop.2.wav\n"));
  await until("YOU ACCEPTED", async () => /YOU ACCEPTED/.test(await text(page, card)));
  await page.click(`${card} .pic-undo`);
  await until("the take taken away", async () => !existsSync(second) && readFileSync(look, "utf8") === lookText);
  ok("ACCEPT writes the take and its look.txt line, PUT THE OLD ONE BACK takes both away");
  const two = await propose(1, "hop-soft.wav", "A softer hop.");
  const card2 = `#asks .snd-card[data-sound="${two.json?.id}"]`;
  await until("the second card", async () => page.isVisible(card2), 15000);
  for (const which of ["old", "new"]) {
    await page.click(`${card2} .snd-play-${which}`);
    await until(`the ${which} sound plays`, () => page.evaluate(([sel, w]) => { const a = document.querySelector(`${sel} audio.snd-${w}`); return a && !a.paused && a.currentTime > 0.02; }, [card2, which]), 10000);
  }
  await page.fill(`${card2} .pic-words`, "Keep the sharp one.");
  await page.click(`${card2} .pic-reject`);
  await until("YOU REJECTED", async () => /YOU REJECTED/.test(await text(page, card2)));
  const back = (await agent.call("gp_run", { command: "theme sound list", args: {} })).json?.answered?.find((p) => p.id === two.json.id);
  if (!readFileSync(first).equals(tone(20)) || back?.words !== "Keep the sharp one.") fail(`REJECT: ${JSON.stringify(back)}`);
  ok(`a new first file's card plays the sound as it is and the new one; REJECT keeps it and the words reach the agent: "${back.words}"`);
  if (errors.length) fail(`the page threw: ${errors.join("; ")}`);
  await page.close();
}

/** The first screen and the two journeys besides porting (issue #5), on
 * Moss Hop: the three choices show; MAKE A THEME makes a made-up theme
 * of Moss Hop from the page and opens it at its step with whose turn it
 * is; a picture put in it and CHECK MY THEME move it to SEE IT IN THE
 * GAME, the person's turn; MAKE A NEW GAME copies the Moss Hop template
 * and opens it at BUILD IT TO PLAY, the tools' turn, and SHOW THE STAGE
 * draws its stage; gp progress --next --copy and the MCP tool next give
 * the same steps; the first screen lists both copies, and CARRY ON opens
 * one at its step; all of it fits at 360 px. */
async function journeys() {
  const base = await gpServe(games);
  const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(`${base}start/`);
  const choices = await until("the three choices", async () => {
    const t = await page.evaluate(() => [...document.querySelectorAll("#start-choices button b")].map((b) => b.textContent));
    return t.length === 3 ? t : null;
  });
  if (choices.join() !== "PORT A GAME I OWN,MAKE A THEME FOR A GAME,MAKE A NEW GAME") fail(`the three choices: ${choices}`);
  const intro = await text(page, "#intro");
  if (!/called the model/.test(intro) || !/A template is a game's rules without its pictures/.test(intro)) fail(`the first screen doesn't say what the model and a template are: ${intro}`);
  ok(`the first screen asks what the person wants to do: ${choices.join(", ")}, and says what the model and a template are`);
  await shot(page, "start-1400");
  const fitsHere = () => page.evaluate(() => {
    const W = innerWidth;
    const off = [...document.querySelectorAll("body *")].filter((e) => e.getClientRects().length && !e.closest("details:not([open]) > :not(summary)"))
      .filter((e) => { const r = e.getBoundingClientRect(); return r.width && (r.left < -1 || r.right > W + 1); })
      .map((e) => `${e.tagName.toLowerCase()}${e.id ? "#" + e.id : ""}.${String(e.className).split(" ")[0]}`);
    return { off: off.slice(0, 8), page: document.documentElement.scrollWidth <= W + 1 };
  });
  const narrowToo = async (what, name) => {
    await page.setViewportSize({ width: 360, height: 780 });
    await page.waitForTimeout(400);
    const f = await fitsHere();
    if (f.off.length || !f.page) fail(`${what} at 360 px runs past the edge: ${JSON.stringify(f)}`);
    await shot(page, `${name}-360`);
    await page.setViewportSize({ width: 1400, height: 900 });
    await page.waitForTimeout(200);
  };
  await narrowToo("the first screen", "start");

  // A theme, made on the page.
  await page.click('#start-choices button[data-journey="theme"]');
  await page.selectOption("#theme-game", `game:${GAME}`);
  await page.fill("#theme-title", "Moss Night");
  await page.fill("#theme-style", "night time, neon");
  await narrowToo("MAKE A THEME's form", "start-theme-form");
  await page.click("#theme-go");
  await until("the theme's journey", async () => page.url().includes("copy=theme/moss-night") && page.isVisible('#journey li[data-step="T2"]'), 30000);
  const steps = (prefix) => page.evaluate((p) => [...document.querySelectorAll(`#journey ol.jr-steps > li[data-step^="${p}"]`)].map((li) => [li.dataset.step, li.dataset.state, li.dataset.who]), prefix);
  const t2 = await steps("T");
  if (JSON.stringify(t2) !== JSON.stringify([["T1", "done", "person"], ["T2", "now", "agent"], ["T3", "later", "tools"], ["T4", "later", "person"], ["T5", "later", "person"], ["T6", "later", "person"]])) fail(`the theme's steps: ${JSON.stringify(t2)}`);
  if (!/NEXT, YOUR AGENT/.test(await text(page, "#jr-next"))) fail(`the theme's next step: ${await text(page, "#jr-next")}`);
  ok("MAKE A THEME makes Moss Night for Moss Hop and opens it at step 2 of 6, Make the pictures, your agent's turn");
  const theme = join(home, "work", "themes", "moss-night", "theme");
  mkdirSync(join(theme, "art"), { recursive: true });
  writeFileSync(join(theme, "art", "hero.png"), discPng(64, 64, [60, 40, 200]));
  writeFileSync(join(theme, "art.txt"), `${readFileSync(join(theme, "art.txt"), "utf8")}pic frame-0.png art/hero.png 16 16 0 0\n`);
  await until("the look check is next", async () => page.isVisible('#journey li[data-step="T3"][data-state="now"]'), 15000);
  await page.click('#journey [data-act="look-check"] button');
  await until("SEE IT IN THE GAME is next", async () => page.isVisible('#journey li[data-step="T4"][data-state="now"]'), 60000);
  const t4 = await steps("T");
  if (t4[3][2] !== "person" || !/NEXT, YOU/.test(await text(page, "#jr-next")) || !/SEE IT IN THE GAME/.test(await text(page, "#jr-next"))) fail(`after the look check: ${JSON.stringify(t4)} ${await text(page, "#jr-next")}`);
  await page.click('#journey li[data-step="T3"] .jr-step-head');
  if (!/no problems/.test(await text(page, '#journey li[data-step="T3"]'))) fail(`the look check's words: ${await text(page, '#journey li[data-step="T3"]')}`);
  ok("with a picture in it, CHECK MY THEME finds no problems and the theme moves to step 4, See it in the game, the person's turn");
  await shot(page, "journey-theme-1400");
  await narrowToo("the theme's journey", "journey-theme");
  const cliTheme = gpRun(["progress", "--next", "--copy", "moss-night"]);
  if (!/step 4 of 6: See it in the game/.test(cliTheme) || !/Next \(you\)/.test(cliTheme)) fail(`gp progress --next --copy moss-night: ${cliTheme}`);
  ok("gp progress --next --copy moss-night says the same step and turn");

  // A new game from the Moss Hop template.
  await page.click("#journey .jr-head button");
  await until("the first screen again", async () => !(await page.isHidden("#first")));
  await page.click('#start-choices button[data-journey="new"]');
  await page.check('#new-start input[name="template"][value="mosshop"]');
  await page.fill("#new-name", "check_hop");
  await narrowToo("MAKE A NEW GAME's form", "start-new-form");
  await page.click("#new-go");
  await until("the new game's journey", async () => page.url().includes("copy=new/check_hop") && page.isVisible('#journey li[data-step="N3"]'), 120000);
  const n3 = await steps("N");
  if (JSON.stringify(n3.map((s) => s[1])) !== JSON.stringify(["done", "done", "now", "later", "later", "later"]) || n3[2][2] !== "tools") fail(`the new game's steps: ${JSON.stringify(n3)}`);
  if (!/NEXT, THE TOOLS/.test(await text(page, "#jr-next")) || !(await page.isVisible('#jr-next button:text("BUILD IT TO PLAY")'))) fail(`the new game's next step: ${await text(page, "#jr-next")}`);
  ok("MAKE A NEW GAME copies the Moss Hop template as check_hop and opens it at step 3 of 6, See it play, the tools' turn, with BUILD IT TO PLAY");
  await page.click('#journey li[data-step="N4"] .jr-step-head');
  await page.click('#journey [data-act="render"] button');
  const stage = await until("the stage drawn", () => page.evaluate(() => { const i = document.querySelector("#journey img.jr-stage"); return i?.complete && i.naturalWidth ? i.naturalWidth : 0; }), 60000);
  ok(`SHOW THE STAGE draws check_hop's stage by kind (${stage} px wide)`);
  // CHECK THE STAGE on Moss Hop's tile stage: gp level verify plays it to
  // the burrow with the slugs in (the reach search and the playthrough).
  await page.click('#journey [data-act="verify"] button');
  const checked = await until("CHECK THE STAGE's answer", async () => {
    const t = await text(page, '#journey li[data-step="N4"] .jr-body');
    return /The last check/.test(t) ? t : null;
  }, 180000);
  if (!/the stage follows the rules and can be finished/.test(checked) || /couldn't check this stage/.test(checked)) fail(`CHECK THE STAGE on check_hop: ${checked}`);
  ok("CHECK THE STAGE checks check_hop's tile stage: it follows the rules and can be finished");
  await shot(page, "journey-new-1400");
  await narrowToo("the new game's journey", "journey-new");
  const agent = mcpClient();
  const told = await agent.call("next", { copy: "check_hop" });
  if (told.stage?.id !== "N3" || told.next?.who !== "tools" || told.steps?.length !== 6 || !told.copies?.some((c) => c.name === "moss-night")) fail(`the MCP next tool for the copy: ${JSON.stringify(told).slice(0, 400)}`);
  ok("the MCP tool next on the copy gives step N3, the tools' turn, the six steps and the person's copies");

  // The copies, and CARRY ON.
  await page.click("#journey .jr-head button");
  const listed = await until("the copies listed", () => page.evaluate(() => [...document.querySelectorAll("#copies li")].map((li) => li.dataset.copy)));
  if (!listed.includes("theme/moss-night") || !listed.includes("new/check_hop")) fail(`the copies: ${listed}`);
  const line = await text(page, '#copies li[data-copy="theme/moss-night"]');
  if (!/for moss-hop/.test(line) || !/changed/.test(line) || !/Step 4 of 6/.test(line)) fail(`the theme's line: ${line}`);
  if (await page.evaluate(() => [...document.querySelectorAll("#carry button, #carry a")].some((b) => /DELETE|REMOVE/i.test(b.textContent)))) fail("the copies list has a way to delete");
  await shot(page, "copies-1400");
  await narrowToo("the copies list", "copies");
  await page.click('#copies li[data-copy="theme/moss-night"] button');
  await until("CARRY ON opens the theme at its step", async () => page.url().includes("copy=theme/moss-night") && page.isVisible('#journey li[data-step="T4"][data-state="now"] .jr-body'), 15000);
  ok(`the first screen lists the copies (${listed.join(", ")}) with where each came from, when it changed and its step, nothing to delete, and CARRY ON opens one at its step`);
  // The root opens the copy opened last, at its step.
  await page.goto(base);
  await until("the root opens the copy opened last", async () => page.url().includes("copy=theme/moss-night"), 15000);
  ok("gp serve's root opens the copy opened last, at its step");
  if (errors.length) fail(`the first screen threw: ${errors.join("; ")}`);
  await page.close();
}

/** SEE IT IN THE GAME on both journeys (issue #6), after journeys():
 * Moss Night (a theme of Moss Hop, a game gp game new made, played from
 * its ROM) opens in the page that plays any game by name with the theme
 * chosen, and the theme moves on to step 5; then check_hop's look step:
 * BUILD IT TO PLAY, DONE WITH THE STAGE, MAKE A THEME FOR IT, a picture,
 * CHECK MY THEME and SEE IT IN THE GAME (a copy of the Moss Hop template,
 * played from its own files with no ROM), and DONE WITH THE LOOK finishes
 * the step. Each server SEE IT IN THE GAME starts is stopped here. */
async function seeItInTheGame() {
  const base = await gpServe(games);
  const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  const now = (step) => page.isVisible(`#journey li[data-step="${step}"][data-state="now"]`);
  // Presses SEE IT IN THE GAME on the open theme: its link, once served.
  const seeIt = async (what) => {
    await page.click('#journey [data-act="look-play"] button');
    const api = `${base}api/journeys/${new URL(page.url()).searchParams.get("copy")}`;
    const job = await until(`${what}: SEE IT IN THE GAME's run`, async () => {
      const j = await page.evaluate(async (u) => (await (await fetch(u)).json()).jobs?.["look-play"] ?? null, api);
      return j && j.state !== "running" ? j : null;
    }, 900000);
    if (job.state !== "done") fail(`${what}: SEE IT IN THE GAME ${job.state}: ${job.error ?? JSON.stringify(job).slice(0, 300)}`);
    const href = await until(`${what}: OPEN THE GAME`, () => page.evaluate(() => document.querySelector("#journey a.jr-link")?.href ?? null), 30000);
    return { href, pid: job.json?.pid ?? null };
  };
  const stop = (pid) => { try { if (pid) process.kill(pid); } catch { /* gone already */ } };

  // Moss Night: the page that plays any game by name, the ROM loaded.
  await page.goto(`${base}start/?copy=theme/moss-night`);
  await until("Moss Night at SEE IT IN THE GAME", () => now("T4"), 30000);
  const t = await seeIt("Moss Night");
  const game = await browser.newPage();
  game.on("pageerror", (e) => errors.push(`the game: ${e}`));
  try {
    await game.goto(t.href);
    if (!/\/p\?g=check\/moss-hop&c=local\/moss-night$/.test(t.href)) fail(`SEE IT IN THE GAME's link: ${t.href}`);
    await game.setInputFiles("#rom", romFile);
    await until("the page takes the ROM", () => game.evaluate(() => ["ok", "warn", "error"].includes(document.getElementById("message")?.dataset.kind)), 120000);
    if ((await game.evaluate(() => document.getElementById("message").dataset.kind)) === "error") fail(`the page refused Moss Hop's ROM: ${await text(game, "#message")}`);
    const frameOf = () => game.frame({ url: /\/game\/(index\.html)?(\?|$)/ });
    const st = await until("Moss Hop plays in Moss Night", async () => {
      const g = frameOf();
      const s = g ? await g.evaluate(() => ({ theme: window.__gp?.theme ?? null, frame: window.__gp?.state?.()?.frame ?? 0 })).catch(() => null) : null;
      return s && s.frame > 30 ? s : null;
    }, 120000);
    if (st.theme !== "moss-night") fail(`the game opened in look ${st.theme}, not moss-night`);
    ok(`SEE IT IN THE GAME serves Moss Hop (a game gp game new made) in the page that plays any game by name: ${t.href.replace(/^http:\/\/[^/]+/, "")}, the ROM loaded, playing in Moss Night (frame ${st.frame})`);
  } finally {
    await game.close();
    stop(t.pid);
  }
  await page.reload();
  await until("Moss Night moves on to step 5", () => now("T5"), 30000);
  ok("seen in the game, Moss Night moves on to step 5, Change pictures");

  // check_hop's look step: a theme of a copy of the Moss Hop template.
  await page.goto(`${base}start/?copy=new/check_hop`);
  await until("check_hop at BUILD IT TO PLAY", () => now("N3"), 30000);
  await page.click('#journey [data-act="build"] button');
  await until("check_hop built to play", () => now("N4"), 900000);
  await page.click('#journey [data-done="stage"]');
  await until("check_hop's look step", () => now("N5"), 30000);
  await page.fill("#jr-look-title", "Hop Night");
  await page.click('#journey button:text("MAKE A THEME FOR IT")');
  await until("Hop Night's journey", async () => page.url().includes("copy=theme/hop-night") && page.isVisible('#journey li[data-step="T1"]'), 30000);
  const said = await page.evaluate(async (u) => (await fetch(u, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ style: "night, moss" }) })).status, `${base}api/journeys/theme/hop-night/set`);
  if (said !== 200) fail(`Hop Night's style: HTTP ${said}`);
  const look = join(home, "work", "themes", "hop-night", "theme");
  writeFileSync(join(look, "art", "hero.png"), discPng(96, 96, [60, 40, 200]));
  writeFileSync(join(look, "art.txt"), `${readFileSync(join(look, "art.txt"), "utf8")}pic frame-0-0.png art/hero.png 16 16 0 0\npic frame-1-0.png art/hero.png 16 16 0 0\n`);
  await page.reload();
  await until("Hop Night's look check is next", () => now("T3"), 30000);
  await page.click('#journey [data-act="look-check"] button');
  await until("Hop Night at SEE IT IN THE GAME", () => now("T4"), 120000);
  const h = await seeIt("Hop Night");
  const hop = await browser.newPage();
  hop.on("pageerror", (e) => errors.push(`check_hop: ${e}`));
  try {
    if (!/\/index\.html\?files=here&theme=hop-night$/.test(h.href)) fail(`check_hop's SEE IT IN THE GAME link: ${h.href}`);
    await hop.goto(h.href);
    const st = await until("check_hop plays in Hop Night", async () => {
      const s = await hop.evaluate(() => ({ theme: window.__gp?.theme ?? null, frame: window.__gp?.state?.()?.frame ?? 0, error: window.__gp?.error ? String(window.__gp.error) : null })).catch(() => null);
      if (s?.error) fail(`check_hop's game: ${s.error}`);
      return s && s.frame > 30 ? s : null;
    }, 120000);
    if (st.theme !== "hop-night") fail(`check_hop opened in look ${st.theme}, not hop-night`);
    ok(`SEE IT IN THE GAME serves check_hop (a copy of the Moss Hop template) from its own files, no ROM asked for, playing in Hop Night (frame ${st.frame})`);
  } finally {
    await hop.close();
    stop(h.pid);
  }
  await page.goto(`${base}start/?copy=new/check_hop`);
  await until("check_hop back at its look step", () => now("N5"), 30000);
  await page.click('#journey [data-done="look"]');
  await until("check_hop's look step finished", () => now("N6"), 30000);
  ok("DONE WITH THE LOOK finishes check_hop's look step: step 6, Play and share");

  // Play and share: MAKE A SHARE LINK publishes check_hop into the
  // person's own registry here (gp game share, which compares it with
  // every ROM in $GP_ROM_DIR first) and shows the link; the link plays it
  // with no ROM asked for, from gp serve's /share/, under the public
  // page's own headers.
  const said6 = await text(page, "#journey");
  if (!/MAKE A SHARE LINK/.test(said6) || /isn't built yet/.test(said6)) fail(`step 6 doesn't offer a share link: ${said6.slice(0, 300)}`);
  await page.click('#journey [data-act="share"] button');
  const shareApi = `${base}api/journeys/new/check_hop`;
  const sj = await until("MAKE A SHARE LINK's run", async () => {
    const j = await page.evaluate(async (u) => (await (await fetch(u)).json()).jobs?.share ?? null, shareApi);
    return j && j.state !== "running" ? j : null;
  }, 1500000);
  if (sj.state !== "done") fail(`MAKE A SHARE LINK ${sj.state}: ${sj.error ?? JSON.stringify(sj).slice(0, 400)}`);
  await page.reload();
  const shareHref = await until("the share link on the page", () => page.evaluate(() => document.querySelector("#jr-share-link a")?.href ?? null), 30000);
  // Hop Night, the theme made in the look step, goes in the package and
  // the link opens in it.
  if (shareHref !== `${base}share/p?g=local/check-hop&look=hop-night`) fail(`the share link: ${shareHref}`);
  const others = await text(page, "#journey");
  if (!/for other people to open it, the page and your registry .* have to be put online together/.test(others)) fail(`step 6 doesn't say what it takes for other people: ${others.slice(0, 400)}`);
  const shared = await browser.newPage();
  shared.on("pageerror", (e) => errors.push(`the shared link: ${e}`));
  const violations = [];
  shared.on("console", (m) => /Content Security Policy|Refused to/i.test(m.text()) && violations.push(m.text()));
  try {
    const res = await shared.goto(shareHref);
    const csp = res.headers()["content-security-policy"] ?? "";
    if (!/default-src 'none'/.test(csp) || res.headers()["cross-origin-embedder-policy"] !== "require-corp") fail(`the shared page isn't served under the public page's headers: ${JSON.stringify(res.headers())}`);
    const frameOf = () => shared.frame({ url: /\/game\/(index\.html)?(\?|$)/ });
    const st = await until("check_hop plays from the share link", async () => {
      const g = frameOf();
      const s = g ? await g.evaluate(() => ({ frames: window.__gp?.hashes?.length ?? 0, theme: window.__gp?.theme ?? null, looks: window.__gp?.looks?.() ?? null, error: window.__gp?.error ? String(window.__gp.error) : null })).catch(() => null) : null;
      if (s?.error) fail(`check_hop from the share link: ${s.error}`);
      return s && s.frames > 30 ? s : null;
    }, 120000);
    if (st.theme !== "hop-night") fail(`the share link opened check_hop in look ${st.theme}, not Hop Night (hop-night)`);
    if (!st.looks?.includes("hop-night")) fail(`the shared game's Settings don't offer Hop Night: ${JSON.stringify(st.looks)}`);
    const fcsp = (await (await shared.request.get(new URL("game/index.html", shareHref).href)).headers())["content-security-policy"] ?? "";
    if (!/default-src 'none'/.test(fcsp)) fail("the shared game's frame isn't served under the public page's policy");
    if (await shared.isVisible("#load-label")) fail("the share link asks for a ROM");
    const mode = await text(shared, "#mode");
    if (!/it plays from its own files, with no ROM/.test(mode)) fail(`the share link's page says: ${mode}`);
    if (violations.length) fail(`the policy refused something on the shared page: ${violations[0]}`);
    ok(`MAKE A SHARE LINK puts check_hop in the person's own registry and shows ${shareHref.replace(base, "/")}, which plays it with no ROM asked for in Hop Night, the theme made for it, offered in Settings with ${st.looks.filter((l) => l !== "hop-night").join(", ")} (${st.frames} frames; "${mode}"), page and frame under the public page's Content-Security-Policy, and step 6 says what it takes for other people to open it`);
  } finally {
    await shared.close();
  }
  if (errors.length) fail(`SEE IT IN THE GAME threw: ${errors.join("; ")}`);
  await page.close();
}

/** The tabs follow the open copy (issue #5, the owner's decision), after
 * seeItInTheGame(): a theme copy (Moss Night) opens from CARRY ON at
 * THEMES, its journey with BESIDE THE ORIGINAL, and PLAY plays Moss Hop in
 * Moss Night in a frame of the page, the ROM loaded there; a game from
 * nothing, made on the page, opens at STEPS, and its PLAY offers BUILD IT
 * TO PLAY; check_hop, built, plays in PLAY with no ROM; CLIPS and REFINE
 * hide for all three; a port (Moss Hop, from the header's picker) has its
 * four tabs as before at CLIPS, and its picker opens a copy at that copy's
 * tab. Each copy kind fits at 360 px; with --shots each is kept at 1400 and
 * 360 px. */
async function copyTabs() {
  const base = await gpServe(games);
  const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  const shown = () => page.evaluate(() => [...document.querySelectorAll("#tabs button")].filter((b) => b.getClientRects().length).map((b) => `${b.textContent}${b.classList.contains("on") ? "*" : ""}`).join(" "));
  const fitsHere = () => page.evaluate(() => {
    const W = innerWidth;
    const off = [...document.querySelectorAll("header *, #tabs *, #journey *, #pane-play *")].filter((e) => e.getClientRects().length && !e.closest("details:not([open]) > :not(summary)"))
      .filter((e) => { const r = e.getBoundingClientRect(); return r.width && (r.left < -1 || r.right > W + 1); })
      .map((e) => `${e.tagName.toLowerCase()}${e.id ? "#" + e.id : ""}.${String(e.className).split(" ")[0]}`);
    return { off: off.slice(0, 8), page: document.documentElement.scrollWidth <= W + 1 };
  });
  // At 1400 and at 360 px: kept, and nothing past the edge at 360.
  const both = async (what, name) => {
    await shot(page, `${name}-1400`, false);
    await page.setViewportSize({ width: 360, height: 780 });
    await page.waitForTimeout(500);
    const f = await fitsHere();
    if (f.off.length || !f.page) fail(`${what} at 360 px runs past the edge: ${JSON.stringify(f)}`);
    await shot(page, `${name}-360`, false);
    await page.setViewportSize({ width: 1400, height: 900 });
    await page.waitForTimeout(200);
  };
  const tabIs = (t) => new URL(page.url()).searchParams.get("tab") === t;
  const playing = async (what, f) => until(what, async () => {
    const g = f.childFrames().find((c) => /\/game\/(index\.html)?(\?|$)/.test(c.url())) ?? f;
    const s = await g.evaluate(() => ({ theme: window.__gp?.theme ?? null, frame: window.__gp?.state?.()?.frame ?? window.__gp?.hashes?.length ?? 0, error: window.__gp?.error ? String(window.__gp.error) : null })).catch(() => null);
    if (s?.error) fail(`${what}: ${s.error}`);
    return s && s.frame > 30 ? s : null;
  }, 180000);

  // A theme copy: CARRY ON leads into THEMES, the theme's journey.
  await page.goto(`${base}start/`);
  await until("the copies listed", () => page.isVisible('#copies li[data-copy="theme/moss-night"] button'));
  await page.click('#copies li[data-copy="theme/moss-night"] button');
  await until("CARRY ON opens Moss Night at THEMES", async () => tabIs("themes") && page.isVisible('#journey li[data-step="T5"][data-state="now"] #jr-beside'), 30000);
  const themeTabs = await shown();
  if (themeTabs !== "PLAY THEMES*") fail(`Moss Night's tabs: ${themeTabs}`);
  if (!(await page.isHidden("#tab-clips")) || !(await page.isHidden("#tab-work"))) fail("CLIPS or REFINE shows for a theme");
  if ((await page.inputValue("#copy-pick")) !== "copy:theme/moss-night") fail(`the header's picker names ${await page.inputValue("#copy-pick")}`);
  ok(`CARRY ON opens Moss Night at THEMES, its journey with BESIDE THE ORIGINAL at step 5; the tabs are ${themeTabs} (CLIPS and REFINE hidden) and the header's picker names it`);
  await both("a theme's THEMES", "tabs-theme-themes");

  // PLAY: Moss Hop in Moss Night, in a frame of the page.
  await page.click("#tab-play");
  await until("Moss Night's PLAY", async () => tabIs("play") && (await page.isHidden("#journey")) && page.isVisible('#pane-play [data-act="look-play"] button, #pane-play #cp-game'), 30000);
  if (await page.isVisible('#pane-play [data-act="look-play"] button')) await page.click('#pane-play [data-act="look-play"] button');
  const ifr = await until("Moss Night's game in PLAY", () => page.$("#pane-play iframe#cp-game"), 900000);
  const pid = await page.evaluate(async (u) => (await (await fetch(u)).json()).jobs?.["look-play"]?.json?.pid ?? null, `${base}api/journeys/theme/moss-night`);
  try {
    const f = await ifr.contentFrame();
    await until("the any-game page in PLAY", () => f.$("#rom"), 60000);
    await f.setInputFiles("#rom", romFile);
    const st = await playing("Moss Hop plays in Moss Night in PLAY", f);
    if (st.theme !== "moss-night") fail(`PLAY opened Moss Hop in look ${st.theme}, not moss-night`);
    ok(`PLAY on Moss Night plays Moss Hop in Moss Night in the page, the ROM loaded there (frame ${st.frame})`);
    if (!(await page.isHidden("#carry")) || !(await page.isHidden("#first"))) fail("the first screen's lists show under a copy's PLAY");
    await both("a theme's PLAY", "tabs-theme-play");
    await page.click("#tab-themes");
    await until("THEMES again, the game stopped", async () => tabIs("themes") && !(await page.$("#cp-game")) && page.isVisible("#journey"));
    ok("leaving PLAY stops the game; THEMES shows the journey again");
  } finally {
    try { if (pid) process.kill(pid); } catch { /* gone already */ }
  }

  // A game from nothing, made on the page: STEPS, and PLAY to build it.
  await page.goto(`${base}start/`);
  await page.click('#start-choices button[data-journey="new"]');
  await page.check('#new-start input[name="template"][data-empty="1"]');
  await page.fill("#new-name", "check_blank");
  await page.click("#new-go");
  await until("check_blank at STEPS", async () => page.url().includes("copy=new/check_blank") && tabIs("steps") && page.isVisible('#journey li[data-step="N1"]'), 120000);
  const newTabs = await shown();
  if (newTabs !== "PLAY STEPS*") fail(`check_blank's tabs: ${newTabs}`);
  if (!/started from nothing/.test(await text(page, "#journey h2"))) fail(`check_blank's heading: ${await text(page, "#journey h2")}`);
  ok(`MAKE A NEW GAME from nothing opens check_blank at STEPS; the tabs are ${newTabs} (CLIPS, REFINE and THEMES hidden)`);
  await both("a new game's STEPS", "tabs-new-steps");
  await page.click("#tab-play");
  await until("check_blank's PLAY", async () => tabIs("play") && page.isVisible('#pane-play [data-act="build"] button'), 30000);
  if (!/isn't built to play yet/.test(await text(page, "#pane-play"))) fail(`check_blank's PLAY: ${await text(page, "#pane-play")}`);
  ok("PLAY on check_blank, not built yet, says so and offers BUILD IT TO PLAY");
  await both("a new game's PLAY", "tabs-new-play");

  // check_hop, built by seeItInTheGame: PLAY plays it, no ROM asked for.
  await page.selectOption("#copy-pick", "copy:new/check_hop");
  await until("the picker opens check_hop at STEPS", async () => page.url().includes("copy=new/check_hop") && tabIs("steps") && page.isVisible('#journey li[data-step="N6"]'), 30000);
  await page.click("#tab-play");
  const hop = await until("check_hop's game in PLAY", () => page.$("#pane-play iframe#cp-game"), 30000);
  const hs = await playing("check_hop plays in PLAY", await hop.contentFrame());
  ok(`the header's picker opens check_hop at STEPS, and PLAY plays it from its own files, no ROM asked for (frame ${hs.frame})`);
  await both("a built new game's PLAY", "tabs-new-built-play");

  // A port: the four tabs, as before.
  await page.selectOption("#copy-pick", `port:${GAME}`);
  await until("the picker opens the port", async () => page.url().includes(`/mechanic/${GAME}/`) && (await gamePageOpen(page)), 240000);
  const portTabs = await until("the port's tabs", async () => { const t = await shown().catch(() => ""); return t.split(" ").length === 4 ? t : null; });
  if (portTabs !== "CLIPS* REFINE PLAY THEMES") fail(`the port's tabs: ${portTabs}`);
  const picks = await until("the port's picker lists the copies", async () => {
    const v = await page.evaluate(() => [...document.querySelectorAll("#game-pick option")].map((o) => o.value));
    return v.includes("copy:theme/moss-night") ? v : null;
  });
  if (!picks.includes(GAME) || (await page.inputValue("#game-pick")) !== GAME) fail(`the port's picker: ${picks}`);
  ok(`a port (${GAME}) keeps its four tabs at CLIPS (${portTabs}), and its header's picker lists the copies too: ${picks.join(", ")}`);
  await shot(page, "tabs-port-1400", false);
  await page.setViewportSize({ width: 360, height: 780 });
  await page.waitForTimeout(500);
  if (!(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1))) fail("the port's page at 360 px runs past the edge");
  await shot(page, "tabs-port-360", false);
  await page.setViewportSize({ width: 1400, height: 900 });
  await page.selectOption("#game-pick", "copy:theme/moss-night");
  await until("the port's picker opens Moss Night at THEMES", async () => page.url().includes("copy=theme/moss-night") && tabIs("themes") && page.isVisible("#journey li"), 30000);
  ok("the port's picker opens Moss Night at its THEMES tab");
  // The root opens the copy opened last, at its first tab.
  await page.goto(base);
  await until("the root opens the copy opened last at its tab", async () => page.url().includes("copy=theme/moss-night") && tabIs("themes"), 15000);
  ok("gp serve's root opens the copy opened last at its THEMES tab");
  if (errors.length) fail(`the copies' tabs threw: ${errors.join("; ")}`);
  await page.close();
}

/** BESIDE THE ORIGINAL on Moss Night's step 5 (issue #5), after
 * seeItInTheGame(): gp theme look play --shots --against original plays
 * the game's route twice in headless Chrome, once in the theme and once
 * in the game's own pictures, and the step shows each moment's pair with
 * YOUR THEME and THE ORIGINAL over it, its words, and BEFORE and NEXT to
 * step through them; it fits at 360 px. */
async function besideTheOriginal() {
  // The run's Chrome, where gp looks for it in the home
  // (scripts/inspector/fetch-browser.sh's layout).
  const shell = join(home, "work", "chrome-headless-shell", "check");
  if (!existsSync(join(shell, "bin"))) {
    mkdirSync(shell, { recursive: true });
    symlinkSync(dirname(browserPath), join(shell, "bin"));
  }
  const base = await gpServe(games);
  const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(`${base}start/?copy=theme/moss-night`);
  await until("Moss Night at step 5", () => page.isVisible('#journey li[data-step="T5"][data-state="now"] #jr-beside'), 30000);
  await page.click('#journey [data-act="compare"] button');
  const api = `${base}api/journeys/theme/moss-night`;
  const job = await until("BESIDE THE ORIGINAL's run", async () => {
    const j = await page.evaluate(async (u) => (await (await fetch(u)).json()).jobs?.compare ?? null, api);
    return j && j.state !== "running" ? j : null;
  }, 1500000);
  if (job.state !== "done") fail(`BESIDE THE ORIGINAL ${job.state}: ${job.error ?? JSON.stringify(job).slice(0, 400)}`);
  const sides = job.json?.sides ?? [];
  if (!/the original/.test(sides[1] ?? "")) fail(`BESIDE THE ORIGINAL's sides: ${JSON.stringify(sides)}`);
  const pair = await until("the first pair shown", () => page.evaluate(() => {
    const i = document.getElementById("jr-pair-img");
    return i?.complete && i.naturalWidth ? { w: i.naturalWidth, h: i.naturalHeight, cap: document.querySelector("#jr-beside figcaption")?.textContent ?? "", heads: document.querySelector("#jr-beside .jr-pair-heads")?.textContent ?? "", n: document.querySelectorAll("#jr-moment option").length } : null;
  }), 30000);
  if (!/^1 of \d+: /.test(pair.cap) || pair.heads !== "YOUR THEMETHE ORIGINAL") fail(`the pair's words: ${JSON.stringify(pair)}`);
  if (pair.n > 1) {
    await page.click('#jr-beside button:text("NEXT")');
    const cap = await text(page, "#jr-beside figcaption");
    if (!/^2 of /.test(cap)) fail(`NEXT didn't step to the second moment: ${cap}`);
    await page.click('#jr-beside button:text("BEFORE")');
  }
  ok(`BESIDE THE ORIGINAL plays Moss Hop's route in Moss Night and in its own pictures and shows ${pair.n} moments side by side (${pair.w}x${pair.h}, "${pair.cap}"), with BEFORE and NEXT`);
  await page.evaluate(() => document.getElementById("jr-beside").scrollIntoView());
  await shot(page, "beside-original-1400", false);
  await page.setViewportSize({ width: 360, height: 780 });
  await page.waitForTimeout(400);
  const f = await page.evaluate(() => {
    const W = innerWidth;
    const off = [...document.querySelectorAll("#jr-beside, #jr-beside *")].filter((e) => e.getClientRects().length)
      .filter((e) => { const r = e.getBoundingClientRect(); return r.width && (r.left < -1 || r.right > W + 1); })
      .map((e) => `${e.tagName.toLowerCase()}${e.id ? "#" + e.id : ""}.${String(e.className).split(" ")[0]}`);
    return { off: off.slice(0, 8), page: document.documentElement.scrollWidth <= W + 1 };
  });
  if (f.off.length || !f.page) fail(`BESIDE THE ORIGINAL at 360 px runs past the edge: ${JSON.stringify(f)}`);
  await page.evaluate(() => document.getElementById("jr-beside").scrollIntoView());
  await shot(page, "beside-original-360", false);
  ok("at 360 px the pair, its words and BEFORE and NEXT fit the window");
  if (errors.length) fail(`BESIDE THE ORIGINAL threw: ${errors.join("; ")}`);
  await page.close();
}

/** Every tab at 360 px wide (issue #17), on a gp serve page with
 * main()'s clip: nothing runs past the window's right edge, the tabs and
 * every visible button, picker and field are inside the window, the game
 * fits the screen, the editor's views in REFINE fit their frame, the
 * agent's log is a full-width panel whose CLOSE closes it, and
 * REVIEW REBUILD's two pictures go one over the other, whole. */
async function narrow() {
  const base = await gpServe(games);
  const page = await browser.newPage({ viewport: { width: 360, height: 780 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(`${base}mechanic/${GAME}/?tab=clips`);
  await until("the page at 360 px", () => gamePageOpen(page), 60000);
  const fits = () => page.evaluate(() => {
    const W = innerWidth;
    // What sticks out past the window: the outermost element that does,
    // leaving out the hidden player frame (kept off screen on purpose)
    // and what a scrolling box holds.
    const out = [];
    const clipped = (el) => { for (let p = el.parentElement; p && p !== document.body; p = p.parentElement) { const o = getComputedStyle(p).overflowX; if (o === "auto" || o === "scroll" || o === "hidden") return true; } return false; };
    for (const el of document.body.querySelectorAll("*")) {
      if (el.id === "rb-frame" || el.closest("#rb-frame") || !el.getClientRects().length) continue;
      const r = el.getBoundingClientRect();
      if (r.width === 0 || r.height === 0 || (r.right <= W + 1 && r.left >= -1) || clipped(el)) continue;
      const q = el.parentElement.getBoundingClientRect();
      if (el.parentElement !== document.body && (q.right > W + 1 || q.left < -1)) continue;
      out.push(`${el.tagName.toLowerCase()}${el.id ? "#" + el.id : ""}${typeof el.className === "string" && el.className ? "." + el.className.split(" ")[0] : ""} ${Math.round(r.left)}..${Math.round(r.right)}`);
    }
    const shown = (el) => el.getClientRects().length && getComputedStyle(el).visibility !== "hidden";
    const off = [...document.querySelectorAll("#tabs button, header button, header select, main button, main select, main input, .pane button, .pane select, .pane input, #log-panel button, #log-panel select")]
      .filter(shown).filter((el) => !el.closest("#rb-frame")).filter((el) => { const r = el.getBoundingClientRect(); return r.left < -1 || r.right > W + 1; })
      .map((el) => `${el.tagName.toLowerCase()} ${el.id || el.textContent.trim().slice(0, 24)}`);
    const frames = [...document.querySelectorAll("iframe.details-frame")].filter(shown).map((f) => {
      const d = f.contentDocument?.documentElement;
      return d && d.scrollWidth > f.clientWidth + 1 ? `${f.dataset.view} ${d.scrollWidth} in ${f.clientWidth}` : null;
    }).filter(Boolean);
    const g = document.getElementById("game");
    const game = shown(g) && !g.closest("main[hidden]") ? Math.round(g.getBoundingClientRect().right) : 0;
    return { out: out.slice(0, 8), off: off.slice(0, 8), frames, game, W };
  });
  const tabs = {
    clips: async () => !(await page.isHidden("main")) && (await page.evaluate(() => document.getElementById("game").getBoundingClientRect().width > 0)),
    work: async () => !(await page.isHidden("#pane-work")) && /workers/.test(await text(page, "#work-body")),
    play: async () => (await page.evaluate(() => document.getElementById("tab-play").classList.contains("on"))) && !(await page.isHidden("main")),
    themes: async () => !(await page.isHidden("#pane-themes")) && (await text(page, "#themes-body")).includes("Theme work comes once"),
  };
  const seen = [];
  for (const [tab, ready] of Object.entries(tabs)) {
    await page.click(`#tab-${tab}`);
    await until(`${tab} at 360 px`, ready, 30000);
    await page.waitForTimeout(tab === "work" ? 3000 : 1000);
    const f = await fits();
    if (f.out.length || f.off.length || f.frames.length || f.game > f.W + 1) fail(`${tab} at 360 px wide: ${JSON.stringify(f)}`);
    seen.push(tab === "work" ? "REFINE" : tab.toUpperCase());
    await shot(page, `narrow-${tab}-360`);
  }
  ok(`at 360 px wide, ${seen.join(", ")} run past no edge, with every tab, button, picker and field inside the window, the game within the screen and REFINE's editor views within their frame`);
  await page.click("#tab-clips");
  await until("CLIPS again at 360 px", tabs.clips);
  await page.click("#log-toggle");
  await until("the log at 360 px", async () => !(await page.isHidden("#log-panel")));
  const log = await page.evaluate(() => {
    const r = document.getElementById("log-panel").getBoundingClientRect();
    const c = document.getElementById("log-close").getBoundingClientRect();
    const at = document.elementFromPoint(c.left + c.width / 2, c.top + c.height / 2);
    return { left: r.left, right: r.right, W: innerWidth, close: at?.id === "log-close" && c.right <= innerWidth && c.left >= 0 };
  });
  if (Math.abs(log.left) > 1 || Math.abs(log.right - log.W) > 1 || !log.close) fail(`the agent's log at 360 px: ${JSON.stringify(log)}`);
  const lf = await fits();
  if (lf.out.length || lf.off.length) fail(`the agent's log at 360 px runs past the edge: ${JSON.stringify(lf)}`);
  await shot(page, "narrow-log-360", false);
  await page.click("#log-close");
  if (!(await page.isHidden("#log-panel")) || (await page.evaluate(() => document.body.classList.contains("log-open")))) fail("CLOSE didn't close the agent's log");
  ok("at 360 px the agent's log is a panel the full width of the window, and its CLOSE closes it");
  await page.click("#clips .rv2-open");
  await until("REVIEW REBUILD at 360 px", async () => /\d+ of \d+ frames the same/.test(await text(page, "#busy-text")), 180000);
  await page.keyboard.press("KeyX");
  await page.selectOption("#rv-layout", "side");
  const two = await until("the pair at 360 px", async () => { const q = await pair(page); return q.w[0] > 0 && q.stacked ? q : null; });
  if (!two.inside || two.w[0] !== two.w[1] || two.w[0] < two.native[0]) fail(`the two pictures at 360 px: ${JSON.stringify(two)}`);
  const rf = await fits();
  if (rf.out.length || rf.off.length) fail(`REVIEW REBUILD at 360 px runs past the edge: ${JSON.stringify(rf)}`);
  await shot(page, "narrow-review-360");
  ok(`at 360 px REVIEW REBUILD shows the two pictures one over the other at ${two.w[0]}x${two.h[0]} each, with its bar inside the window`);
  if (errors.length) fail(`the page at 360 px threw: ${errors.join("; ")}`);
  await page.close();
}

let code = 0;
try {
  if (only === "arcade") {
    browser = await chromium.launch({ executablePath: browserPath, args: ["--no-sandbox"] });
    await arcade();
  } else if (only === "journeys") await journeysOnly();
  else await whole();
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

/** --only journeys: the first screen, the copies and their tabs, on Moss
 * Hop made here as main() makes it, without main()'s clip. */
async function journeysOnly() {
  gpRun(["roms", "add", romFile, "--games", games]);
  gpRun(["game", "new", GAME, "--rom", join(home, "roms", "nes", "mosshop.nes"), "--genre", "platformer", "--namespace", "check", "--games", games]);
  browser = await chromium.launch({ executablePath: browserPath, args: ["--no-sandbox"] });
  await journeys();
  await seeItInTheGame();
  await copyTabs();
}

/** Every run, in order. */
async function whole() {
  await main();
  await oneServer();
  await agentOnThePage();
  await editorFeatures();
  await whatIsThis();
  await pictureProposals();
  await soundProposals();
  await roleSoundProposals();
  await journeys();
  await seeItInTheGame();
  await besideTheOriginal();
  await copyTabs();
  await narrow();
  await arcade();
  await romSets();
}
