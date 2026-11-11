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
// conversation, answers them, then plays in FREE PLAY, where the agent's
// next words wait until Esc goes back. Last, a second server with no player
// built: the page must say so, in the server's words, instead of failing
// to compile the 404 as wasm. Any error the page throws fails the check.
//
// Playwright comes from scripts/web/node_modules (`npm ci --prefix
// scripts/web`), or PLAYWRIGHT_FROM names another package.json whose
// node_modules has it. The browser is the one
// scripts/inspector/fetch-browser.sh fetches.
//
// Exit codes: 0 passed, 1 a check failed, 2 couldn't run.
import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const opt = (n) => {
  const i = process.argv.indexOf(`--${n}`);
  return i < 0 || !process.argv[i + 1] ? null : resolve(process.argv[i + 1]);
};
const gp = opt("gp"), romFile = opt("rom"), wasm = opt("wasm"), browserPath = opt("browser");
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
const env = { ...process.env, GP_HOME: home, GP_ROM_DIR: join(home, "roms"), GP_GAMES_DIR: games, GP_PLAY_WASM: wasm };
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
function serve(extra = {}) {
  const p = spawn(gp, ["mechanic", "start", GAME, MECH, "--port", "0", "--games", games], { env: { ...env, ...extra } });
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

  // FREE PLAY: the game plays on its own and the agent's words wait.
  await page.click("#tab-free");
  await until("FREE PLAY", async () => (await text(page, "#modebadge")) === "FREE PLAY");
  const fa = await frame(page);
  await page.keyboard.down("ArrowLeft");
  await page.waitForTimeout(500);
  await page.keyboard.up("ArrowLeft");
  const fb = await frame(page);
  if (fb - fa < 15) fail(`FREE PLAY played ${fb - fa} frames in half a second`);
  const later = "One more when you're ready.";
  gpRun(["mechanic", "say", GAME, MECH, later]);
  await until("the waiting message", async () => (await text(page, "#busy-text")).includes("Your agent has a message"));
  if (!(await page.isHidden("#overlay"))) fail("the agent's words showed over FREE PLAY");
  ok(`FREE PLAY plays (${fb - fa} frames in half a second) and the agent's words wait`);
  await page.keyboard.press("Escape");
  await until("the words after Esc", async () => (await text(page, "#say")).startsWith(later) && !(await page.isHidden("#overlay")));
  if (!(await page.evaluate(() => document.getElementById("tab-mech").classList.contains("on")))) fail("Esc didn't go back to HELP WITH MECHANICS");
  ok("Esc goes back to HELP WITH MECHANICS, where the words show");
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

let code = 0;
try {
  await main();
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
