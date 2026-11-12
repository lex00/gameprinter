// The watch view end to end, in a headless browser: runs `gp watch start`
// on a ROM into a scratch GP_HOME, starts `gp serve` on it, opens
// ?watch=NAME and checks the view draws what the session found: the PRG
// map's lit pages (each `--bank B` given with code lit), the stops a run
// went past (`--stops-reached N`), the game replaying the last kept clip in
// gp_play.wasm to a last frame whose hash equals gp-nes's in the session
// (`end_hash`), the instructions of a frame shown on a click of the feed,
// and no page errors.
//
//   node e2e/watch.mjs --gp GP --rom ROM --browser HEADLESS_SHELL [--bank 1] [--stops-reached 1] [-- WATCH_ARGS...]
//
// `just check-watch-e2e ROM` runs it, with the browser from
// scripts/inspector/fetch-browser.sh and the player from
// scripts/web/build-play.sh. Exit codes: 0 passed, 1 a check failed, 2
// couldn't run.

import { spawn, spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright-core";

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, "../..");

function arg(name, fallback) {
  const i = process.argv.indexOf(`--${name}`);
  if (i < 0 || !process.argv[i + 1]) {
    if (fallback !== undefined) return fallback;
    console.error(`watch e2e: give --${name}`);
    process.exit(2);
  }
  return process.argv[i + 1];
}
function args(name) {
  const out = [];
  for (let i = 0; i < process.argv.length; i++) if (process.argv[i] === `--${name}` && process.argv[i + 1]) out.push(process.argv[i + 1]);
  return out;
}

const gp = resolve(arg("gp"));
const rom = resolve(arg("rom"));
const browserPath = resolve(arg("browser"));
const banks = args("bank").map(Number);
const stopsReached = Number(arg("stops-reached", "0"));
const dash = process.argv.indexOf("--");
const watchArgs = dash < 0 ? ["--budget", "100", "--seed", "1"] : process.argv.slice(dash + 1);

const scratch = mkdtempSync(join(process.env.TMPDIR ?? tmpdir(), "gp-watch-e2e-"));
const env = { ...process.env, GP_HOME: scratch, GP_STORE: join(scratch, "store") };
delete env.GP_ROM_DIR;
mkdirSync(join(scratch, "store"), { recursive: true });

const failures = [];
function check(ok, what) {
  console.log(`${ok ? "ok  " : "FAIL"}  ${what}`);
  if (!ok) failures.push(what);
}

let server, url;
try {
  const w = spawnSync(gp, ["watch", "start", "e2e", rom, ...watchArgs], { env, cwd: repo, encoding: "utf8" });
  if (w.status !== 0) throw new Error(`gp watch start exited ${w.status}: ${w.stderr}`);
  console.log(w.stdout.trim());
  server = spawn(gp, ["serve", "--port", "0", "--rom", rom, "--editor", join(repo, "editor/dist")], {
    env,
    cwd: repo,
    stdio: ["ignore", "pipe", "inherit"],
  });
  url = await new Promise((ok, fail) => {
    let out = "";
    server.stdout.on("data", (d) => {
      out += d;
      const m = out.match(/http:\/\/127\.0\.0\.1:\d+\//);
      if (m) ok(m[0]);
    });
    server.on("exit", (code) => fail(new Error(`gp serve exited with ${code}: ${out}`)));
    setTimeout(() => fail(new Error("gp serve printed no URL within 10 s")), 10_000);
  });
} catch (e) {
  console.error(`watch e2e: couldn't set up: ${e.message ?? e}`);
  server?.kill();
  process.exit(2);
}

let browser;
let exit = 0;
try {
  try {
    browser = await chromium.launch({ executablePath: browserPath, headless: true });
  } catch (e) {
    console.error(`watch e2e: can't start the browser at ${browserPath}: ${e.message}`);
    exit = 2;
    throw e;
  }
  const page = await browser.newPage({ viewport: { width: 1400, height: 1000 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  await page.goto(`${url}?watch=e2e`);
  await page.waitForFunction(() => document.querySelector('[data-testid="watch-ticker"]')?.textContent?.includes("Done."), null, { timeout: 30_000 });
  check(true, "the view follows the session to its end");
  const lit = Number(await page.getByTestId("watch-banks").getAttribute("data-lit"));
  check(lit > 0, `the PRG map lights ${lit} pages`);
  for (const b of banks) {
    const n = Number(await page.locator(`[data-bank="${b}"]`).getAttribute("data-lit"));
    check(n > 0, `bank ${b}'s row has ${n} pages lit`);
  }
  const reached = await page.locator('[data-testid="watch-stop"][data-reached="yes"]').count();
  check(reached >= stopsReached, `${reached} stops marked reached (at least ${stopsReached})`);
  await page.waitForFunction(() => document.querySelector('[data-testid="watch-game-status"]')?.getAttribute("data-hash"), null, { timeout: 60_000 });
  const game = page.getByTestId("watch-game-status");
  const matches = await game.getAttribute("data-matches");
  check(matches === "true", `the replayed clip's last frame hash equals the session's gp-nes hash (${await game.textContent()})`);
  // A line of the feed from a run asks for that frame's instructions.
  await page.locator('[data-testid="watch-feed"] li[data-kind="bank"] button, [data-testid="watch-feed"] li[data-kind="new-code"] button').first().click();
  await page.waitForFunction(() => document.querySelector('[data-testid="watch-detail"] pre')?.textContent?.length > 0, null, { timeout: 30_000 });
  const lines = (await page.locator('[data-testid="watch-detail"] pre').textContent()).split("\n").length;
  check(lines > 10, `a click on the feed shows the frame's instructions (${lines} lines)`);
  // A click on a sprite names the routine that wrote it. Where a sprite is
  // comes from gp watch point over the whole screen at the shown frame.
  const shownFrame = Number((await game.textContent()).match(/frame (\d+)/)[1]);
  const recipe = await page.evaluate(async () => {
    const r = await fetch("/api/watch/e2e/input");
    const inputs = (await r.json()).inputs;
    return inputs.filter((i) => i.kind === "view").at(-1)?.data?.recipe;
  });
  check(!!recipe, "the page posts what it shows as a view input");
  const all = JSON.parse(
    spawnSync(gp, ["--json", "watch", "point", "e2e", "--recipe", recipe, "--frame", String(shownFrame), "--rect", "0,0,256,240"], { env, cwd: repo, encoding: "utf8" }).stdout,
  );
  const sprites = (all.data ?? all).sprites.filter((s) => s.bytes.length > 0 && s.y < 239);
  if (sprites.length) {
    const sp = sprites[0];
    // The feed's click above scrolled the page; bring the game back.
    await page.getByTestId("watch-canvas").scrollIntoViewIfNeeded();
    const box = await page.getByTestId("watch-canvas").boundingBox();
    await page.mouse.click(box.x + ((sp.x + 4) / 256) * box.width, box.y + ((sp.y + 5) / 240) * box.height);
    const said = await page
      .waitForFunction(() => /written by|no sprite|can't point/.test(document.querySelector('[data-testid="watch-point"]')?.textContent ?? ""), null, { timeout: 60_000 })
      .then(() => page.getByTestId("watch-point").textContent())
      .catch(async () => `no answer within 60 s; the page says: ${(await page.locator('[data-testid="watch-point"]').count()) ? await page.getByTestId("watch-point").textContent() : "nothing"}`);
    check(said.includes(`sprite ${sp.index}, written by`), `a click on sprite ${sp.index} names its writer (${said})`);
  } else {
    check(true, "no sprite on screen at the shown frame to click");
  }
  check(errors.length === 0, `no page errors${errors.length ? `: ${errors.join("; ")}` : ""}`);
} catch (e) {
  if (!exit) {
    console.error(`watch e2e: ${e.message ?? e}`);
    exit = 1;
  }
} finally {
  await browser?.close();
  server?.kill();
}
if (!exit && failures.length) exit = 1;
console.log(exit === 0 ? "watch e2e: passed" : `watch e2e: ${failures.length} checks failed`);
process.exit(exit);
