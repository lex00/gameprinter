// The inspector end to end, in a headless browser: records two Moss Hop
// runs into a scratch store, starts `gp serve` on them, opens the inspector
// and checks every view draws, seeks to frames across the recording, and
// opens the two runs side by side from their first difference. On the way it
// checks the workspace's parts: a box selection names the player's model
// paths, the lifted tiles are drawn in the frame's colors, and an annotation
// added on the timeline is written beside the recording. The slowest
// seek, from request to drawn frame, is recorded as
// inspector.seek.render.max_ms through `gp metrics record`.
//
//   node e2e/inspector.mjs --gp GP --rom MOSSHOP_ROM --browser HEADLESS_SHELL
//
// `just check-inspector-e2e` runs it (part of `just check-cycle`), with the
// browser from scripts/inspector/fetch-browser.sh. Exit codes: 0 passed, 1 a
// check failed, 2 couldn't run.

import { spawn, spawnSync, execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync, mkdirSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright-core";

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, "../..");

function arg(name) {
  const i = process.argv.indexOf(`--${name}`);
  if (i < 0 || !process.argv[i + 1]) {
    console.error(`inspector e2e: give --${name}`);
    process.exit(2);
  }
  return resolve(process.argv[i + 1]);
}

const gp = arg("gp");
const rom = arg("rom");
const browserPath = arg("browser");
// Scratch GP_HOME and store; metrics go to the caller's GP_HOME.
const scratch = mkdtempSync(join(process.env.TMPDIR ?? tmpdir(), "gp-inspector-e2e-"));
const env = { ...process.env, GP_HOME: scratch, GP_STORE: join(scratch, "store") };
delete env.GP_ROM_DIR;
mkdirSync(join(scratch, "store"), { recursive: true });

const failures = [];
function check(ok, what) {
  console.log(`${ok ? "ok  " : "FAIL"}  ${what}`);
  if (!ok) failures.push(what);
}

// `gp trace diff` exits 1 when the runs differ; its JSON is still the answer.
function gpJson(args, okCodes = [0]) {
  const r = spawnSync(gp, ["--json", ...args], { env, cwd: repo, encoding: "utf8" });
  if (!okCodes.includes(r.status)) throw new Error(`gp ${args.join(" ")} exited ${r.status}: ${r.stderr}`);
  return JSON.parse(r.stdout);
}

function readText(path) {
  try {
    return readFileSync(path, "utf8");
  } catch {
    return "";
  }
}

function record(baseline, name) {
  const log = join(scratch, `${name}.log`);
  gpJson(["input-log", "generate", "--baseline", join(repo, "baselines/mosshop", baseline), "--out", log]);
  return gpJson(["record", "--rom", rom, "--input", log, "--name", name]).id;
}

// Setting up (recording, starting gp serve) failing means the test couldn't
// run: exit 2, with the server stopped.
let a, b, frames, server, url;
try {
  // Moss Hop's tiles and palettes, lifted into the scratch GP_HOME, for the
  // Tiles view.
  gpJson(["assets", "lift", rom]);
  a = record("finish-level.toml", "mosshop-finish-level");
  b = record("stomp-then-pit.toml", "mosshop-stomp-then-pit");
  frames = gpJson(["recordings", "show", a]).recording.frames;
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
  console.error(`inspector e2e: couldn't set up: ${e.message ?? e}`);
  server?.kill();
  process.exit(2);
}

let browser;
let exit = 0;
try {
  try {
    browser = await chromium.launch({ executablePath: browserPath, headless: true });
  } catch (e) {
    console.error(`inspector e2e: can't start the browser at ${browserPath}: ${e.message}`);
    exit = 2;
    throw e;
  }
  const page = await browser.newPage({ viewport: { width: 1400, height: 1000 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));

  const status = page.getByTestId("status");
  // A frame is shown once the status names it, and its seek is timed once
  // the browser has painted it, a moment later.
  let timed = 0;
  const waitFrame = async (n) => {
    await page.waitForFunction(
      (n) => document.querySelector('[data-testid="status"]')?.textContent?.startsWith(`frame ${n} from`),
      n,
      { timeout: 10_000 },
    );
    timed += 1;
    await page.waitForFunction((k) => (window.__gpSeeks ?? []).length >= k, timed, { timeout: 10_000 });
  };

  await page.goto(`${url}?a=${a}&frame=0`);
  await waitFrame(0);
  check((await status.textContent()).includes("from the trace"), "frame 0 is read from the trace");
  for (const view of ["ram-heatmap", "ram-watch", "oam", "nametables", "event-lanes"]) {
    check((await page.locator(`[data-view="${view}"]`).count()) === 1, `the ${view} view is drawn`);
  }
  check((await page.getByTestId("ram-watch").locator("tbody tr").count()) > 10, "the RAM watch lists Moss Hop's labels");

  // Seek: the frame input, the keyboard and the event window all move it.
  const targets = [1, 59, 60, 119, 150, 200, 239, frames - 1, frames];
  for (const n of targets) {
    await page.getByTestId("frame-input").fill(String(n));
    await page.getByTestId("frame-input").press("Enter");
    await waitFrame(n);
  }
  await page.locator("body").click({ position: { x: 5, y: 5 } });
  await page.keyboard.press("ArrowLeft");
  await waitFrame(frames - 1);
  check(true, `seeked to ${targets.length + 1} frames, up to the last (${frames})`);
  const playerX = page.getByTestId("ram-watch").locator("tr", { hasText: "PLAYER_X" }).first();
  check((await playerX.count()) === 1, "PLAYER_X is in the watch list");
  check((await page.locator('[data-testid^="lanes-"] .ev').count()) > 0, "the frame's events are on the lanes");
  check((await page.locator('[data-testid^="oam-"] rect.sprite, [data-testid^="oam-"] rect.sprite0').count()) > 0, "sprites are drawn");
  // Coverage over Moss Hop's source: measured by replay with the --rom
  // file, and drawn over the listing from target/mosshop/mosshop.dbg.
  await page.getByTestId("coverage-listing").waitFor({ timeout: 20_000 });
  await page.getByTestId("coverage").locator('select[aria-label="lines"]').selectOption("code");
  check((await page.locator('[data-testid="coverage-listing"] .run-all').count()) > 0, "the coverage view marks source lines that ran");
  check((await page.getByTestId("coverage").textContent()).includes("PRG ROM bytes ran"), "the coverage view sums the executed bytes");

  const seeks = await page.evaluate(() => window.__gpSeeks ?? []);
  const slowest = seeks.reduce((m, s) => Math.max(m, s.ms), 0);
  check(seeks.length >= targets.length + 2, `${seeks.length} seeks timed; the slowest took ${slowest.toFixed(1)} ms`);
  check(slowest < 1000, "every seek drew its frame within a second");

  // The workspace. Box selection: a box around the player names its model
  // paths and the RAM that holds them.
  await page.goto(`${url}?a=${a}&frame=150`);
  timed = 0;
  await waitFrame(150);
  const sel = await (await fetch(`${url}api/recordings/${a}/select?frame=150&rect=0,0,256,240`)).json();
  const player = sel.entities.find((e) => e.path === "entities.player[0]");
  check(player !== undefined, "the select endpoint finds the player through Moss Hop's state mapping");
  const screen = page.getByTestId("select-screen");
  await screen.scrollIntoViewIfNeeded();
  const box = await screen.boundingBox();
  const sx = box.width / 256;
  const sy = box.height / 240;
  const px = player.box.x + player.box.w / 2;
  const py = player.box.y + player.box.h / 2;
  await page.mouse.move(box.x + (px - 6) * sx, box.y + (py - 6) * sy);
  await page.mouse.down();
  await page.mouse.move(box.x + (px + 6) * sx, box.y + (py + 6) * sy, { steps: 4 });
  await page.mouse.up();
  await page.getByTestId("selection").waitFor({ timeout: 10_000 });
  const selected = await page.getByTestId("selection").textContent();
  check(selected.includes("entities.player[0].x") && selected.includes("PLAYER_X"), "a dragged box lists the player's model paths and RAM labels");
  // Tiles: drawn through the frame's palette RAM, so not in grays.
  await page.getByTestId("tiles-canvas").waitFor({ timeout: 10_000 });
  const colored = await page.getByTestId("tiles-canvas").evaluate((c) => {
    const d = c.getContext("2d").getImageData(0, 0, c.width, c.height).data;
    for (let i = 0; i < d.length; i += 4) if (d[i] !== d[i + 1] || d[i + 1] !== d[i + 2]) return true;
    return false;
  });
  check(colored, "the lifted tiles are drawn in the game's colors");
  // Annotations: add one on the timeline, and it's in the file beside the recording.
  await page.getByTestId("annotate").click();
  await page.getByTestId("note-text").fill("the e2e test was here");
  await page.getByTestId("note-text").press("Enter");
  await page.locator(".timeline-mark").first().waitFor({ timeout: 10_000 });
  check((await page.getByTestId("annotations").textContent()).includes("the e2e test was here"), "an annotation added on the timeline is listed at its frame");
  const notes = readText(join(scratch, "annotations", `${a}.toml`));
  check(notes.includes("the e2e test was here") && notes.includes("from = 150"), "the annotation is written to $GP_HOME/annotations");

  // The model beside the original: a model run on the original's polled
  // input, lined up through the state mapping, with a resolution card for
  // an entity and a ghost preview of a lower gravity.
  const m = gpJson(["record", "--model", "templates/mosshop", "--polled-of", a, "--name", "mosshop-model"]).id;
  gpJson(["validate", "templates/mosshop", "--against", a, "--rom", rom], [0, 1]);
  await page.goto(`${url}?a=${a}&m=${m}&frame=60`);
  timed = 0;
  await waitFrame(60);
  await page.getByTestId("pair-frame").waitFor({ timeout: 10_000 });
  check((await page.getByTestId("pair-summary").textContent()).includes("every mapped path agrees"), "the model run agrees with the original on every mapped path");
  check((await page.locator('[data-testid="pair-paths"] tbody tr').count()) >= 10, "the pair view lists the mapped paths of both sides");
  check((await page.locator('[data-testid="model-entity"]').count()) > 0, "the model's entities are drawn beside the original's");
  await page.locator('[data-testid="original-entity"]').first().click();
  await page.getByTestId("card").waitFor({ timeout: 10_000 });
  check((await page.getByTestId("card-facts").locator("tr").count()) > 0, "a clicked entity opens a card with the model's facts");
  check((await page.getByTestId("card-checks").locator("tr").count()) > 0, "the card lists what gp validate measured");
  await page.getByTestId("change-path").fill("entities.player.jump.gravity");
  await page.getByTestId("change-value").fill("0.125");
  await page.getByTestId("change-add").click();
  await page.getByTestId("change-preview").click();
  await page.getByTestId("ghost-screen").waitFor({ timeout: 20_000 });
  check((await page.locator('[data-testid="ghost"]').count()) > 0, "a preview draws the changed runs as ghosts over the original");
  check((await page.getByTestId("variant-runs").textContent()).includes("parts from the model"), "the preview says where the changed run parts from the model");

  // Diff mode: the two runs side by side from their first difference.
  const diff = gpJson(["trace", "diff", a, b], [1]);
  await page.goto(`${url}?a=${a}&b=${b}&frame=0`);
  timed = 0;
  await waitFrame(diff.frame);
  check((await page.getByTestId("diff-fields").textContent()).includes(`First difference at frame ${diff.frame}`), `diff mode opens at the first difference, frame ${diff.frame}`);
  check((await page.locator('[data-testid^="heatmap-"]').count()) === 2, "both recordings' RAM is drawn side by side");
  check((await page.locator("tr.differs").count()) > 0, "the watch list marks labels that differ");
  await page.screenshot({ path: join(scratch, "inspector.png"), fullPage: true });
  console.log(`screenshot: ${join(scratch, "inspector.png")}`);

  check(errors.length === 0, `no errors in the page${errors.length ? ": " + errors.join("; ") : ""}`);
  execFileSync(gp, ["metrics", "record", "--name", "inspector.seek.render.max_ms", "--value", slowest.toFixed(2), "--unit", "ms"], {
    stdio: "inherit",
    cwd: repo,
  });
  exit = failures.length ? 1 : 0;
} catch (e) {
  // Once the browser runs, anything that throws (a frame that never
  // shows, say) is a failed check.
  if (exit !== 2) {
    console.error(`inspector e2e: ${e.stack ?? e}`);
    failures.push(String(e.message ?? e));
    exit = 1;
  }
} finally {
  await browser?.close();
  server?.kill();
}
writeFileSync(join(scratch, "result.txt"), failures.length ? failures.join("\n") : "passed\n");
console.log(exit === 0 ? "inspector e2e: passed" : `inspector e2e: ${failures.length} checks failed`);
process.exit(exit);
