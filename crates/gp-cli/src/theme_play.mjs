// `gp theme look play --shots`: the page in headless Chrome, a ROM loaded,
// and the game frame played along a route to chosen frames, the game's
// canvas saved at each. gp writes this file to a scratch folder and runs
//   node theme_play.mjs CONFIG.json
// CONFIG: { origin, link, rom, browser, playwright, out, prefix, theme,
//           shots: [{ name, run, frame }] }
// The route files are served by gp under /game/runs/NAME.frames. With
// `rom` null the game plays from its own files and `link` is the player
// itself (the interpreter preview, runs/ beside it), opened at each moment
// with `run`, `from` and `to`. One JSON object a line goes to stdout:
// { shot } per picture, then { done }.
// The ways of the page used here are check-page.mjs's (playTo) and the
// hand scripts' that this command replaces.
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";

const cfg = JSON.parse(readFileSync(process.argv[2], "utf8"));
const { chromium } = createRequire(cfg.playwright)("playwright-core");
mkdirSync(cfg.out, { recursive: true });
const say = (o) => console.log(JSON.stringify(o));

const browser = await chromium.launch({
  executablePath: cfg.browser,
  args: ["--no-sandbox", "--autoplay-policy=no-user-gesture-required"],
});
const consoleLines = [];
let errors = 0;
try {
  const context = await browser.newContext({ viewport: { width: 1366, height: 1100 } });
  const p = await context.newPage();
  const note = (line, bad) => {
    consoleLines.push(line);
    if (bad) errors++;
  };
  // A game with no hooks has no module.wasm: the player asks for it and
  // runs its own interpreter, so that 404 is no error. A failed load is
  // counted once, by its response.
  const optional = (url) => /\/module\.wasm(\?|$)/.test(url);
  p.on("console", (m) => note(`[${m.type()}] ${m.text()}`, m.type() === "error" && !m.text().startsWith("Failed to load resource")));
  p.on("pageerror", (e) => note(`[pageerror] ${e.message}`, true));
  context.on("requestfailed", (r) => note(`[requestfailed] ${r.url()} ${r.failure()?.errorText}`, !optional(r.url())));
  context.on("response", (r) => {
    if (r.status() >= 400) note(`[http ${r.status()}] ${r.url()}`, !(r.status() === 404 && optional(r.url())));
  });
  const game = () => p.frame({ url: /\/game\/(index\.html)?(\?|$)/ });
  const gp = async (f, a) => (await game()).evaluate(f, a);

  if (!cfg.rom) {
    for (const s of cfg.shots) {
      await p.goto(`${cfg.origin}${cfg.link}&run=${s.run}&from=${s.frame}&to=${s.frame}`);
      const st = await p
        .waitForFunction((f) => { const st = window.__gp?.state?.(); return st && st.frame >= f ? st : null; }, s.frame, { timeout: 120000, polling: 100 })
        .then((h) => h.jsonValue())
        .catch(() => null);
      await p.waitForTimeout(250);
      const png = await p.evaluate(() => document.getElementById("game").toDataURL("image/png")).catch(() => null);
      const theme = await p.evaluate(() => window.__gp?.theme ?? null).catch(() => null);
      let file = null;
      if (png && st) {
        file = join(cfg.out, `${cfg.prefix}-${s.name}.png`);
        writeFileSync(file, Buffer.from(png.split(",")[1], "base64"));
      }
      say({ shot: s.name, file, frame: st?.frame ?? null, level: st?.level ?? null, theme });
    }
  } else {
    await p.goto(`${cfg.origin}${cfg.link}`);
    await p.waitForFunction(() => window.__page !== undefined, null, { timeout: 300000 }).catch(() => {});
    await p.setInputFiles("#rom", cfg.rom);
    await p.waitForFunction(() => ["ok", "warn", "error"].includes(document.getElementById("message").dataset.kind), null, {
      timeout: 300000,
    });
    const kind = await p.evaluate(() => document.getElementById("message").dataset.kind);
    const message = await p.locator("#message").innerText();
    if (kind === "error") {
      say({ failed: `the page refused the ROM: ${message}` });
      process.exitCode = 1;
    } else {
      await p.waitForFunction(() => document.getElementById("game").src.includes("files=idb:"), null, { timeout: 60000 });
      await p.waitForTimeout(2000);
      const romSrc = await p.locator("#game").getAttribute("src");
      say({ message, src: romSrc });
      const at = async (run, frame) => {
        await gp(() => (window.__checkOld = true)).catch(() => {});
        await p.locator("#game").evaluate((f, s) => (f.src = s), `${romSrc}&run=${run}&from=${frame}&to=${frame}`);
        await p.waitForFunction(() => document.getElementById("game").src.includes("run="), null, { timeout: 5000 });
        const t0 = Date.now();
        while (Date.now() - t0 < 120000) {
          const st = await gp(() => (window.__checkOld ? null : (window.__gp?.state?.() ?? null))).catch(() => null);
          if (st && st.frame >= frame) return st;
          await p.waitForTimeout(100);
        }
        return null;
      };
      for (const s of cfg.shots) {
        const st = await at(s.run, s.frame);
        await p.waitForTimeout(250);
        const png = await gp(() => document.getElementById("game").toDataURL("image/png")).catch(() => null);
        const theme = await gp(() => window.__gp.theme).catch(() => null);
        let file = null;
        if (png && st) {
          file = join(cfg.out, `${cfg.prefix}-${s.name}.png`);
          writeFileSync(file, Buffer.from(png.split(",")[1], "base64"));
        }
        say({ shot: s.name, file, frame: st?.frame ?? null, level: st?.level ?? null, theme });
      }
    }
  }
} finally {
  writeFileSync(join(cfg.out, `${cfg.prefix}-console.txt`), consoleLines.join("\n") + "\n");
  say({ done: true, console_lines: consoleLines.length, console_errors: errors });
  await browser.close();
}
