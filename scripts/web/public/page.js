// The public page (docs/web.md, "The public page"): a visitor's own ROM,
// checked and lifted in this browser (lift.js), played in the original's
// look and layout or in a theme. Nothing plays until a ROM is loaded; a
// ROM kept from an earlier visit plays at once. No game data is on the
// server, and the ROM's bytes never leave the browser: this file sends no
// request with them, and lift.js only reads the module and the template's
// own files.
//
// The folder holds index.html and this file, page.json (the titles and
// the game the page takes ROMs of), game/ (the preview: gp_web.wasm,
// preview.js, the authored stage's bundle and pictures, themes/ with the
// looks the preview offers in its settings screen, and lift/ with
// gp-web-lift.wasm and Donkey Kong's templates). Any dump of the game is
// taken: the lift finds its assets by content and picks the model its
// program matches; a ROM whose game has less than the model (Donkey Kong
// (JU) has no 50m) leaves blank what only that part uses (docs/web.md,
// "The ROM, in the browser").
//
// Nothing here waits before the ROM picker is handled: boot.js (a classic
// script, which runs first) takes the picked file, says "Checking NAME..."
// at once and hands it to `liftAndPlay` through `__gpBoot.take`, set
// below before any `await`. The lift is imported when it is needed, not
// before this file runs, so a lift.js that fails to load is reported as
// the ROM's message rather than stopping the page with no word.

const $ = (id) => document.getElementById(id);
const frame = $("game");
const say = (text, kind = "") => {
  $("message").textContent = text;
  $("message").dataset.kind = kind;
};

// The page's files, each fetched once; a step that needs one awaits it.
let liftModule = null;
const lift = () => (liftModule ??= import("./game/lift.js"));
const pageFile = fetch("page.json").then((r) => {
  if (!r.ok) throw new Error(`page.json didn't load (${r.status})`);
  return r.json();
});
let page = null;
let rom = null;
let link = null;
// A play link's game and content: `g` and `c`, each NAME or NAME@VERSION.
// With no link, the page's own game and its own content (page.json's
// `content`, a theme installed with the game). The page has one source,
// its own.
function gameLink(p) {
  const q = new URLSearchParams(location.search);
  const part = (v) => {
    if (!v) return null;
    const [name, version = null] = v.split("@");
    return { name, version };
  };
  const game = part(q.get("g")) ?? { name: p.game, version: null };
  if (game.name !== p.game) throw new Error(`this page plays ${p.game}, not ${game.name}`);
  return { game, content: part(q.get("c")) ?? part(p.content), direct: q.has("g") };
}
// page.json read into `page` and `rom`: what a ROM needs before it can
// play. A file picked earlier waits for it in liftAndPlay.
const ready = pageFile.then((p) => {
  page = p;
  rom = p.rom;
  link = gameLink(p);
  $("rom-help").textContent =
    `Load your own ${rom.name} ROM (${rom.file}) to play. ` + "It is checked and read in this browser and never uploaded.";
});
ready.catch(() => {});

// The game, installed from the page's source or already in this browser,
// and the link's content. Everything that plays waits for this.
let installer = null;
const install = () => (installer ??= import("./game/install.js"));
let notices = [];
const installing = ready.then(async () => {
  const x = await install();
  const progress = (n, of) => say(`Getting the game: ${n} of ${of} files. It stays in this browser so it plays offline.`);
  const g = await x.ensure(page.source, link.game, progress);
  if (!g.record.files.some((f) => f.path.startsWith("play/"))) throw new Error(`${g.record.name}@${g.record.version} has no files this player can run`);
  await x.setCurrent(g.record);
  if (g.notice) notices.push(g.notice);
  // The themes: the link's (the page's own, Turnips say), which the game
  // opens in, and every other theme the source offers for the game, each
  // a choice in Settings, Theme. A theme that won't install is named and
  // left out; the rest still play.
  const asked = link.content ? [{ ...link.content, linked: true }] : [];
  const list = await x.offered(page.source, g.record.name).catch(() => []);
  for (const o of list) if (!asked.some((a) => a.name === o.name)) asked.push({ name: o.name, version: null });
  let theme = null;
  const themes = [];
  for (const a of asked) {
    const c = await x.ensure(page.source, { name: a.name, version: a.version ?? null, for_game: g.record.name }).catch((err) => {
      notices.push(
        a.linked
          ? `${a.name} couldn't be used: ${err.message ?? err}. Playing in the game's own look.`
          : `The theme ${a.name} couldn't be installed: ${err.message ?? err}.`,
      );
      return null;
    });
    if (!c) continue;
    if (c.notice) notices.push(c.notice);
    const t = { id: c.record.name.split("/")[1], files: await x.filesOf(c.record, "theme") };
    if (themes.some((x) => x.id === t.id)) continue;
    themes.push(t);
    if (a.linked) theme = t;
  }
  say("");
  return { game: g.record, theme, themes, files: await x.filesOf(g.record, "play") };
});
installing.catch(() => {});

function focusGame() {
  try {
    frame.contentWindow?.focus();
  } catch {
    // Not loaded yet.
  }
}
frame.addEventListener("load", focusGame);
// The game's sound plays through one AudioContext of this page's, which
// the game frame takes (preview.js, `shared`). Safari runs a context only
// when it is made or resumed while a click or a key is handled, and a
// gamepad's buttons are neither, so a context the game frame made as it
// loaded stayed held after the click on "Load your own ROM" (the user,
// 2026-11-05, Safari with a gamepad: no sound until a key). This one is
// made or resumed in every click and key on the page, the Load click
// first among them, and outlives the frames the page loads. `make` unset
// asks for it only if it is there.
let audioContext = null;
window.__gpAudio = (make = true) => {
  if (!make) return audioContext;
  try {
    audioContext ??= new AudioContext({ sampleRate: 44100 });
    if (audioContext.state !== "running") audioContext.resume().catch(() => {});
  } catch {
    // A browser with no audio output: the game plays on in silence.
  }
  return audioContext;
};
for (const e of ["keydown", "pointerdown", "click", "touchend"]) addEventListener(e, () => window.__gpAudio(true), true);
// A key pressed while the shell has the focus goes to the game.
addEventListener("keydown", (e) => {
  if (e.target === document.body) focusGame();
});

// The Settings button opens the game's own screen (docs/settings.md): the
// preview takes the same message the mechanic page sends, and the focus
// goes back to the game so the screen's keys work at once.
$("settings").addEventListener("click", () => {
  frame.contentWindow?.postMessage({ gp: "settings", key: "open" }, location.origin);
  $("settings").blur();
  focusGame();
});

// The how-to-play panel: open on a first visit, and the choice is kept in
// localStorage (which can be missing or refuse, so every use is guarded).
const HOWTO_KEY = "gp-public:howto";
function setHowto(open, save) {
  $("howto").hidden = !open;
  $("howto-toggle").setAttribute("aria-expanded", String(open));
  if (!save) return;
  try {
    localStorage.setItem(HOWTO_KEY, open ? "open" : "closed");
  } catch {
    // Not kept this time.
  }
}
let howtoOpen = true;
try {
  howtoOpen = localStorage.getItem(HOWTO_KEY) !== "closed";
} catch {
  // Open.
}
setHowto(howtoOpen, false);
$("howto-toggle").addEventListener("click", () => {
  setHowto($("howto").hidden, true);
  $("howto-toggle").blur();
  focusGame();
});

// Asset names in words: `kong_take` is "kong take".
const words = (names) => names.map((n) => n.replaceAll("_", " ")).join(", ");

// What the lift found, and what it left blank, in a sentence or two.
function foundText(found) {
  if (!found) return "the original's look and layout";
  const by = (k) => found.assets.filter((a) => a.from === k);
  const same = by("same");
  const changed = by("changed").map((a) => a.name);
  const blank = by("blank");
  const parts = [];
  if (!changed.length && !blank.length) parts.push(`all ${same.length} of its assets found`);
  else {
    parts.push(`${same.length + changed.length} of ${found.assets.length} assets found`);
    if (changed.length) parts.push(`with small changes: ${words(changed)}`);
    // Named in result.json; here only counted, as their names say little
    // to a player, with what of the game uses them.
    const without = [...new Set(blank.map((a) => a.without).filter(Boolean))];
    if (blank.length) parts.push(`${blank.length} left blank` + (without.length ? `, since your ROM's game has no ${without.join(" and no ")}` : ""));
  }
  return parts.join("; ");
}

function romName(found) {
  return found?.rom_name ?? (found?.template_name ? `${rom.title} (like ${found.template_name})` : rom.name);
}

// The ROM themes the lift couldn't offer (result.json's looks_skipped,
// crates/gp-web-lift), in a line under the mode line: each theme's title
// and the lift's reason, which is written to follow "because". Without
// it a theme the visitor's ROM can't take is missing from the settings
// with no word. Hidden when there are none and when no ROM plays.
function showSkipped(skipped = []) {
  const el = $("skipped");
  el.hidden = !skipped.length;
  el.textContent = skipped.length
    ? `${skipped.length === 1 ? "ROM theme" : "ROM themes"} not offered: ` +
      skipped.map((s) => `${s.title}, because ${s.reason}`).join("; ") +
      "."
    : "";
}

let linked = null;
// `rom` plays the lifted ROM; `none` shows no game and asks for a ROM.
function play(mode, found = null, skipped = []) {
  const romMode = mode === "rom";
  // A linked theme (the page's own, Turnips say) opens the game in it; a
  // ROM with none starts in its own art (own-look), whatever look an
  // earlier visit kept.
  const wanted = romMode && linked?.theme && !skipped.some((t) => t.id === linked.theme.id) ? `theme=${linked.theme.id}` : "own-look";
  if (romMode) frame.src = `game/index.html?files=idb:${rom.game}&${wanted}&scale=fit`;
  else frame.removeAttribute("src");
  frame.hidden = !romMode;
  if (page) {
    $("title").textContent = page.title;
    document.title = page.title;
  }
  $("mode").textContent = romMode
    ? `Playing your own ${romName(found)} ROM: ${foundText(found)}, kept in this browser only`
    : "No ROM loaded";
  $("back").hidden = !romMode;
  $("start").hidden = romMode;
  showSkipped(romMode ? skipped : []);
}

// The play files the lift reads: those under lift/, by their path there,
// and what its index aliases beside the folder.
function playTemplates(files) {
  const out = new Map();
  for (const [k, v] of files) {
    out.set(k.startsWith("lift/") ? k.slice(5) : k, v);
  }
  return out;
}

// Lifts a ROM (a File, or the ArrayBuffer kept from an earlier visit) and
// plays it, saying what was found. `name` is the file's name for the first
// message. Every way it can fail ends in a message in words.
async function liftAndPlay(file, name) {
  say(`Checking ${name}...`);
  try {
    await ready.catch((err) => {
      throw new Error(`the page's settings didn't load (${err.message ?? err}); reload the page`);
    });
    const { liftRom } = await lift().catch((err) => {
      throw new Error(`the page's ROM reader (game/lift.js) didn't load: ${err.message ?? err}`);
    });
    linked = await installing;
    const r = await liftRom(file, {
      moduleUrl: "game/lift/gp-web-lift.wasm",
      templateFiles: playTemplates(linked.files),
      extraThemes: linked.themes ?? (linked.theme ? [linked.theme] : []),
    });
    if (!r.ok) {
      say(r.error, "error");
      return;
    }
    const f = r.found;
    const model = f ? ` Playing as ${f.template_name ?? f.template}.` : "";
    say(
      `${romName(f)}: ${foundText(f)}. Read in ${Math.round(r.ms ?? 0)} ms, kept in this browser.${model}` +
        (f?.warning ? ` ${f.warning}` : ""),
      f?.warning ? "warn" : "ok",
    );
    play("rom", f, r.looks_skipped ?? []);
  } catch (err) {
    say(`The ROM couldn't be read: ${err.message ?? err}`, "error");
  }
}

$("back").addEventListener("click", () => {
  say("");
  play("none");
});

// The ROM picker, taken by boot.js; and if boot.js didn't run (a page
// that lacks it), by this file, the same way: the first of `change` and
// `input` takes the file and clears the input.
const boot = window.__gpBoot;
if (boot) boot.take = (file, name) => liftAndPlay(file, name);
else {
  const picked = (e) => {
    const file = e.target.files?.[0];
    if (!file) return;
    e.target.value = "";
    liftAndPlay(file, file.name);
  };
  $("rom").addEventListener("change", picked);
  $("rom").addEventListener("input", picked);
}

// A ROM file dropped on the start panel loads as if picked.
const start = $("start");
start.addEventListener("dragover", (e) => {
  e.preventDefault();
  start.classList.add("over");
});
start.addEventListener("dragleave", () => start.classList.remove("over"));
start.addEventListener("drop", (e) => {
  e.preventDefault();
  start.classList.remove("over");
  const file = e.dataTransfer?.files?.[0];
  if (file) liftAndPlay(file, file.name);
});

boot?.markReady();

let started = false;
try {
  await ready;
  linked = await installing;
  started = true;
  if (notices.length) say(notices.join(" "), "warn");
} catch (err) {
  say(`${page?.title ?? "The game"} couldn't be installed: ${err.message ?? err}.`, "error");
}
window.__page = { play, liftRom: async (...a) => (await lift()).liftRom(...a), showSkipped, installing };
// No game until a ROM: unless one picked while the page loaded already plays.
if (!frame.getAttribute("src")) play("none");

// Kept from an earlier visit: the ROM plays at once, lifted again, so it
// plays with this page's themes and fixes, not the ones of the page that
// first lifted it. Files an older page kept without the ROM can't be
// lifted again, so they are dropped and the visitor loads the file once
// more. IndexedDB can be refused (some private windows) or, in some Safari
// versions, never answer; either way the page waits for a ROM without the
// buttons, and nothing here holds up the picker.
const { storedRom, loadStored, clearStored } = started ? await lift().catch(() => ({})) : {};
const keptRom = storedRom ? await storedRom(rom.game).catch(() => null) : null;
if (!keptRom && loadStored && (await loadStored(rom.game).catch(() => null))) await clearStored(rom.game).catch(() => {});
if (keptRom) liftAndPlay(keptRom, "your ROM");
if (keptRom) {
  const again = document.createElement("button");
  again.className = "quiet";
  again.textContent = "Play my ROM";
  again.addEventListener("click", () => liftAndPlay(keptRom, "your ROM"));
  const forget = document.createElement("button");
  forget.className = "quiet";
  forget.textContent = "Forget my ROM data";
  forget.addEventListener("click", async () => {
    try {
      await clearStored(rom.game);
    } catch (err) {
      say(`Your ROM's data couldn't be removed from this browser: ${err.message ?? err}`, "error");
      return;
    }
    again.remove();
    forget.remove();
    play("none");
    say("Your ROM and its data are gone from this browser.", "ok");
  });
  $("back").after(again, forget);
}
