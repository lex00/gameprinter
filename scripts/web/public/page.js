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

import { gamePageOf, linkSettings, profileNames, readLink, shareLink } from "./share.js";
import { whatIsThis } from "./what-is-this.js";

const $ = (id) => document.getElementById(id);
const frame = $("game");
const say = (text, kind = "") => {
  $("message").textContent = text;
  $("message").dataset.kind = kind;
};

// The files this page takes, by a game's ROM kind (its play/page.json's
// rom.kind; docs/web.md, "Players"): the picker's file types (null for
// any, as the NES picker always was), where the lift reads the file in its
// memory, and the word the page's lines call it by. One lift module reads
// both (it knows a zip by its bytes) and one frame plays both: an arcade
// model is keyed as a cabinet and drawn by game/gp_board.wasm, which the
// frame fetches only for a model with board art.
const KINDS = {
  nes: { accept: null, romPath: "/rom.nes", noun: "ROM", file: "ROM file" },
  zip: { accept: ".zip,application/zip,application/x-zip-compressed", romPath: "/rom.zip", noun: "romset", file: "romset zip" },
  // A game that plays from its own files (a new game, made with pictures
  // of its own): no file is asked for, nothing is lifted, and the frame
  // opens on the installed package's play/ files.
  none: { own: true },
};
// The players by the name a package gives (`player`, kept for packages
// already published: lex00/donkey-kong@1.0.9 names `nes`), each with the
// ROM kinds it takes. An arcade game sounds through its board's own sound
// CPU and chips, run in this browser from the visitor's romset and fed the
// requests the game's module makes (game/gp_arcade_play.wasm, preview.js
// BoardSound; docs/web.md, "Arcade games"); a module built before it makes
// none, and plays silent.
const PLAYERS = {
  nes: { kinds: ["nes"] },
  arcade: { kinds: ["zip"], sound: "The board's own sound, played from your romset in this browser. It starts on your first key or click." },
  own: { kinds: ["none"] },
};
const LIFT_MODULE = "game/lift/gp-web-lift.wasm";
const FRAME = "game/index.html";
// The kind of file the game being played takes (an NES ROM until a
// game's facts say otherwise).
const kindOf = () => KINDS[rom?.kind] ?? KINDS.nes;
// Whether the game being played plays from its own files.
const ownFiles = () => !!kindOf().own;
// An own game's lines: how to play it when its package says nothing (the
// NES keys, preview.js's NES_LAYOUT; the settings screen), and the About
// line saying what it is and who made it, from its package's page facts.
const OWN_HOWTO = [
  "Controls: the arrows move, X is the A button, Z the B button and Enter is Start. A controller works too.",
  "Settings: press Esc or the Settings button (top right) to switch theme and mode.",
];
const ownAbout = (f) =>
  `${f.about ?? `${f.title} is a game made with Game Printer.`} It plays from its own pictures, so there is no ROM to load.${f.madeBy ? ` Made by ${f.madeBy}.` : ""}`;

// The page's files, each fetched once; a step that needs one awaits it.
let liftModule = null;
const lift = () => (liftModule ??= import("./game/lift.js"));
const pageFile = fetch("page.json").then((r) => {
  if (!r.ok) throw new Error(`page.json didn't load (${r.status})`);
  return r.json();
});
// `page` is the bundle's page.json (its source, its own game and theme);
// `facts` the game being played's title, player, ROM and help text: the
// bundle's for the page's own game, its package's for any other. `rom` is
// facts.rom.
let page = null;
let facts = null;
let rom = null;
let link = null;
let player = null;
// What the address asked for, each value held to its shape (share.js):
// the game `g` and theme `c`, each NAME or NAME@VERSION, a look `look`,
// and the settings `mode` and `on`. With no `g`, the page's own game in
// its own content (page.json's `content`, a theme installed with the
// game); `look=original` opens in the original's look instead. The page
// has one source, its own.
function gameLink(p) {
  const l = readLink(location.search);
  const game = l.game ?? { name: p.game, version: null };
  const own = game.name === p.game;
  const content = l.look ? null : (l.content ?? (own && p.content ? readLink(`?c=${encodeURIComponent(p.content)}`).content : null));
  return { ...l, game, own, content, linkedContent: !!l.content && !l.look, direct: !!l.game };
}
// The game's facts shown on the page: its title, the ROM line, and for a
// game that isn't the page's own its how-to-play lines and its About
// lines, each as text.
function showFacts(f) {
  rom = f.rom;
  const k = kindOf();
  if (k.own) {
    // Nothing to load: the picker is never shown, and the lines say what
    // the game is and who made it, as text.
    $("rom-help").textContent = "";
    $("start").hidden = true;
    $("title").textContent = f.title;
    document.title = f.title;
    const lines = f.howto?.length ? f.howto : OWN_HOWTO;
    $("howto").replaceChildren(...lines.map((t) => Object.assign(document.createElement("p"), { textContent: t })));
    $("about-what").textContent = "This page plays a game made playable in the browser with Game Printer. This one was made with pictures of its own.";
    $("about-rom").textContent = ownAbout(f);
    $("about-mark").textContent = "";
    return;
  }
  $("rom-help").textContent =
    `Load your own ${rom.name} ${k.noun} (${rom.file}) to play. ` + "It is checked and read in this browser and never uploaded.";
  $("title").textContent = f.title;
  document.title = f.title;
  // The how-to-play lines, and for a player whose sound needs saying a
  // line about it, as text.
  const sound = PLAYERS[f.player]?.sound;
  const lines = f.howto?.length ? [...f.howto] : sound ? [...$("howto").children].map((p) => p.textContent) : [];
  if (sound) lines.push(`Sound: ${sound}`);
  if (lines.length) $("howto").replaceChildren(...lines.map((t) => Object.assign(document.createElement("p"), { textContent: t })));
  if (!link.own || rom.kind !== "nes") {
    $("about-rom").textContent = `"Load your own ROM" accepts a ${rom.name} ${k.file} that you own. The page checks and reads it in your browser. It is never uploaded.`;
  }
  if (!link.own || f.trademark) {
    $("about-mark").textContent =
      f.trademark ??
      (rom.kind === "zip"
        ? arcadeNotice(f.title)
        : `This page is not affiliated with or endorsed by the makers of ${f.title}, which is named here only to say which file is accepted.`);
  }
  // An arcade game's notice shows by the picker too, not only in About.
  const notice = rom.kind === "zip" ? (f.trademark ?? arcadeNotice(f.title)) : "";
  $("rom-notice").textContent = notice;
  $("rom-notice").hidden = !notice;
  showSets(rom.sets);
}
// The owner's one-line notice for an arcade game (#3), when its package
// gives no trademark line of its own.
const arcadeNotice = (title) => `You need your own ${title} romset to play; ${title}'s names and trademarks belong to their owners.`;
// The MAME sets an arcade game takes (play/page.json's rom.sets) under
// About, each with the files its zip must hold, as text.
function showSets(sets) {
  const box = $("about-sets");
  if (!box) return;
  box.hidden = !sets?.length;
  if (!sets?.length) return box.replaceChildren();
  const p = (t) => Object.assign(document.createElement("p"), { textContent: t });
  box.replaceChildren(
    p("The romsets this page takes, by MAME's names, and the files each one's zip must hold (under any names: the page knows a file by its SHA-1):"),
    ...sets.map((s) => p(`${s.name}, ${s.title}: ${s.files.length} files: ${s.files.map((f) => `${f.name} (${f.size} bytes, SHA-1 ${f.sha1})`).join(", ")}.`)),
  );
}
// What to bring, after a romset zip the lift refused: the set or sets by
// MAME's names, from the package's facts.
function bringLine(r) {
  const sets = r?.kind === "zip" ? (r.sets ?? []) : [];
  if (!sets.length) return "";
  const named = sets.map((s) => `${s.name} (${s.title})`);
  if (named.length === 1) return ` Bring MAME's ${named[0]} set: a zip of its ${sets[0].files.length} files, listed with their SHA-1s under About.`;
  return ` Bring one of MAME's sets ${named.slice(0, -1).join(", ")} or ${named.at(-1)}: About lists the files each zip must hold, with their SHA-1s.`;
}
// page.json read into `page`, and for the page's own game into `facts`
// and `rom`: what a ROM needs before it can play. Another game's facts
// come with its package (installing, below). A file picked earlier waits
// for them in liftAndPlay.
const ready = pageFile.then((p) => {
  page = p;
  link = gameLink(p);
  if (link.own) {
    facts = { title: p.title, player: p.player ?? "nes", rom: { kind: "nes", ...p.rom }, howto: p.howto ?? [], trademark: p.trademark ?? null, madeBy: p.made_by ?? null, about: p.about ?? null };
    showFacts(facts);
  } else {
    // Nothing of the page's own game shows for another: its name from the
    // link (held to a package name's shape) until its package says more,
    // and help that names no game's keys.
    $("title").textContent = link.game.name;
    document.title = link.game.name;
    $("rom-help").textContent = `Getting ${link.game.name} from the page's source...`;
    $("about-rom").textContent = '"Load your own ROM" accepts a ROM file of the game that you own. The page checks and reads it in your browser. It is never uploaded.';
    $("about-mark").textContent = "This page is not affiliated with or endorsed by the makers of the games it plays.";
    $("howto").replaceChildren(
      ...[
        "Settings: press Esc or the Settings button (top right) to switch theme and mode.",
        "Load your own ROM to play. Under Settings, Theme switches between the original's art and a theme.",
      ].map((t) => Object.assign(document.createElement("p"), { textContent: t })),
    );
  }
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
  const g = await x.ensure(page.source, { name: link.game.name, version: link.game.version }, progress);
  if (g.record.kind !== "game") throw new Error(`${g.record.name} isn't a game on this page's source`);
  if (!g.record.files.some((f) => f.path.startsWith("play/"))) throw new Error(`${g.record.name}@${g.record.version} has no files this player can run`);
  const files = await x.filesOf(g.record, "play");
  // Another game's facts are its package's, checked before anything of
  // it runs: a player this page hasn't, or a ROM kind that player doesn't
  // take, is refused here, before a ROM is asked for.
  if (!link.own) facts = gamePageOf(files, g.record.title);
  player = PLAYERS[facts.player];
  if (!player) throw new Error(`${g.record.name} needs the ${facts.player} player, which this page doesn't have yet`);
  if (!player.kinds.includes(facts.rom.kind) || !KINDS[facts.rom.kind]) throw new Error(`${g.record.name} takes a ${facts.rom.kind} file, which the ${facts.player} player doesn't read`);
  if (!link.own) showFacts(facts);
  const accept = KINDS[facts.rom.kind].accept;
  // A game that plays from its own files has no lift, so its package's
  // own play/ files are the frame's (preview.js reads the installed game's
  // when it is opened with no `files`): they must hold what it plays.
  if (KINDS[facts.rom.kind].own && !["bundle.json", "preview.json"].every((n) => files.has(n))) throw new Error(`${g.record.name} has no bundle.json and preview.json to play`);
  if (accept) $("rom").setAttribute("accept", accept);
  await x.setCurrent(g.record);
  if (g.notice) notices.push(g.notice);
  if (link.ignored.length) notices.push(`The link's ${link.ignored.join(", ")} ${link.ignored.length === 1 ? "wasn't" : "weren't"} understood and ${link.ignored.length === 1 ? "was" : "were"} left out.`);
  // The settings the link asks for, held to the modes and switches the
  // game's own profiles.toml files offer (each template's in its lift).
  const names = { presets: new Set(), switches: new Set() };
  for (const [k, v] of files) {
    if (!/^lift\/[^/]+\/profiles\.toml$/.test(k) && k !== "profiles.toml") continue;
    const n = profileNames(new TextDecoder().decode(v));
    n.presets.forEach((s) => names.presets.add(s));
    n.switches.forEach((s) => names.switches.add(s));
  }
  const settings = linkSettings(link, names);
  if (settings.dropped.length) notices.push(`This game has no setting ${settings.dropped.join(", ")}, so the link's ${settings.dropped.length === 1 ? "is" : "are"} left out.`);
  // The themes: the link's (the page's own, Turnips say), which the game
  // opens in, and every other theme the source offers for the game, each
  // a choice in Settings, Theme. A theme that won't install is named and
  // left out; the rest still play. A theme the link names must be on the
  // source's list for the game when the list can be read (gp-registry.wasm
  // checks it again as it installs).
  // Every theme the source lists for the game and the page leaves out is
  // kept with why (`notOffered`: { name, title, reason, detail }, the
  // reason written to follow "because"), shown in the line under the
  // header while a ROM plays and in About, with the detail in the log.
  const asked = link.content ? [{ ...link.content, linked: true }] : [];
  const offer = await x.offer(page.source, g.record.name).then(
    (o) => o,
    () => null,
  );
  const listed = offer?.offered ?? null;
  const notOffered = (offer?.leftOut ?? []).map((l) => ({ name: l.name, title: l.title, reason: l.reason, detail: l.detail }));
  const list = listed ?? [];
  // A theme offered whose last recheck failed (its by-game line's
  // `recheck`, passed on by gp-registry.wasm only when it failed) is still
  // offered, and says when and why in the same line and in About.
  const rechecked = list.filter((o) => o.recheck).map((o) => ({ name: o.name, title: o.title, date: o.recheck.date, reason: o.recheck.reason }));
  if (listed && link.content && !listed.some((o) => o.name === link.content.name)) {
    const left = notOffered.find((l) => l.name === link.content.name);
    notices.push(
      left
        ? `${link.content.name} isn't offered for ${g.record.title}, because ${left.reason}. Playing in the game's own look.`
        : `${link.content.name} isn't a theme the source offers for ${g.record.title}. Playing in the game's own look.`,
    );
    asked.shift();
  }
  for (const o of list) if (!asked.some((a) => a.name === o.name)) asked.push({ name: o.name, version: null, title: o.title });
  // A game that plays from its own files offers the looks its package
  // holds (play/themes/); a theme from the source would reach the frame
  // only through a ROM's lift, so each is named and left out.
  if (KINDS[facts.rom.kind].own) {
    for (const a of asked.splice(0)) {
      if (a.linked) notices.push(`${a.name} can't be shown on ${g.record.title} yet: a theme from the source reaches only a game played from a ROM. Playing in the game's own look.`);
      else notOffered.push({ name: a.name, title: a.title ?? a.name, reason: "the page shows a theme from the source only on a game played from a ROM, and this one plays from its own files", detail: `${a.name} is listed for ${g.record.name}` });
    }
  }
  let theme = null;
  const themes = [];
  for (const a of asked) {
    const c = await x.ensure(page.source, { name: a.name, version: a.version ?? null, for_game: g.record.name }).catch((err) => {
      notices.push(
        a.linked
          ? `${a.name} couldn't be used: ${err.message ?? err}. Playing in the game's own look.`
          : `The theme ${a.name} couldn't be installed: ${err.message ?? err}.`,
      );
      if (!a.linked) notOffered.push({ name: a.name, title: a.title ?? a.name, reason: "it couldn't be installed", detail: String(err.message ?? err) });
      return null;
    });
    if (!c) continue;
    if (c.notice) notices.push(c.notice);
    const [ns, id] = c.record.name.split("/");
    const title = c.record.title || c.record.name;
    const t = { id, name: c.record.name, title, own: ns === g.record.name.split("/")[0], files: await x.filesOf(c.record, "theme") };
    if (!t.files.size) {
      notOffered.push({ name: t.name, title, reason: "its package holds no theme", detail: `${c.record.name}@${c.record.version} holds no file under theme/, where the page reads a theme's look.txt` });
      continue;
    }
    const same = themes.find((x) => x.id === t.id);
    if (same) {
      notOffered.push({ name: t.name, title, reason: `another theme, ${same.title}, has the same name here, and Settings, Theme lists one of each name`, detail: `${t.name} and ${same.name} both end in \`${t.id}\`` });
      continue;
    }
    themes.push(t);
    if (a.linked) theme = t;
  }
  for (const l of notOffered) console.info(`gp: theme ${l.name} not offered: ${l.reason} (${l.detail})`);
  showAboutThemes(notOffered, [], [], rechecked);
  // Settings, Theme lists them in this order after Original: the themes
  // from the game's own namespace first (for lex00/donkey-kong,
  // lex00/turnips before a theme someone else made), then the rest as the
  // source offered them. The games site orders a game's themes the same way.
  themes.sort((a, b) => b.own - a.own);
  say("");
  return { game: g.record, theme, themes, files, names, profile: settings.profile, notOffered, rechecked };
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
// The romset an arcade game plays from, for the game frame's sound board
// (preview.js, `parent.__gpRom`): the bytes the visitor loaded, which
// never leave this browser. Null until one is loaded, and for an NES game.
let romBytes = null;
window.__gpRom = () => romBytes;
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

// Share: a link to this page with the game, the theme or look, and the
// mode or switches being played (read from the game's frame, same origin),
// else the ones the link asked for. Nothing else: never the ROM, a save,
// or anything of the player. Copied when the browser lets the page, and
// shown in the message line either way.
function currentShare() {
  let theme = null;
  let profile = linked?.profile ?? null;
  try {
    const gp = frame.getAttribute("src") ? frame.contentWindow?.__gp : null;
    if (gp) {
      theme = typeof gp.theme === "string" ? gp.theme : null;
      profile = typeof gp.profile === "string" ? gp.profile : profile;
    }
  } catch {
    // The frame isn't readable yet.
  }
  let content = null;
  let look = null;
  if (theme === null) {
    content = linked?.theme?.name ?? null;
    look = !content && link?.look ? link.look : null;
  } else {
    const t = linked?.themes?.find((x) => x.id === theme);
    if (t) content = t.name;
    else look = theme;
  }
  return shareLink(new URL("p", document.baseURI).href, { game: linked?.game?.name ?? link.game.name, content, look, profile, names: linked?.names ?? null });
}
$("share").addEventListener("click", async () => {
  $("share").blur();
  let url;
  try {
    await installing;
    url = currentShare();
  } catch (err) {
    say(`No link to share: ${err.message ?? err}`, "error");
    return;
  }
  let copied = false;
  try {
    await navigator.clipboard.writeText(url);
    copied = true;
  } catch {
    // Not allowed here: the link is shown to copy by hand.
  }
  say(`${copied ? "Link copied" : "Link to share"}: ${url} (it holds the game, the theme and the settings, never your ROM or saves)`, "ok");
  $("message").dataset.share = url;
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

// The themes the page doesn't offer, in sentences: the ROM themes the
// lift couldn't offer (result.json's looks_skipped, crates/gp-web-lift),
// the themes the source lists that the page left out (`notOffered`:
// gp-registry.wasm's left_out and the installs that failed), and the
// themes offered with some of their lines passed over (looks_partial).
// Each names the theme's title and the reason, written to follow
// "because". Without them a theme is missing from the settings, or drawn
// in part, with no word. Last, the themes offered whose recheck failed
// (`rechecked`), each with the day and what failed.
function notOfferedText(skipped = [], left = [], partial = [], rechecked = linked?.rechecked ?? []) {
  const one = (list, single, many) =>
    list.length ? `${list.length === 1 ? single : many}: ` + list.map((s) => `${s.title}, because ${s.reason}`).join("; ") + "." : "";
  const failed = rechecked.length
    ? `${rechecked.length === 1 ? "Theme that failed a recheck" : "Themes that failed a recheck"}: ` +
      rechecked.map((s) => `${s.title}, on ${s.date}: ${s.reason.replace(/\.$/, "")}`).join("; ") +
      "."
    : "";
  return [one(skipped, "ROM theme not offered", "ROM themes not offered"), one(left, "Theme not offered", "Themes not offered"), one(partial, "Theme drawn in part", "Themes drawn in part"), failed]
    .filter(Boolean)
    .join(" ");
}

// In a line under the mode line while a ROM plays; hidden when there is
// nothing to say and when no ROM plays. The details go to the log.
function showSkipped(skipped = [], partial = []) {
  const el = $("skipped");
  const text = notOfferedText(skipped, linked?.notOffered ?? [], partial);
  el.hidden = !text;
  el.textContent = text;
  for (const s of [...skipped, ...partial]) console.info(`gp: theme ${s.id} (${s.title}): ${s.reason} (${s.detail})`);
  showAboutThemes(linked?.notOffered ?? [], skipped, partial);
}

// The same in About, so it can be found with or without a ROM: what the
// source's list leaves out from the start, and what the ROM's lift adds.
function showAboutThemes(left = [], skipped = [], partial = [], rechecked = linked?.rechecked ?? []) {
  const el = $("about-themes");
  if (!el) return;
  const text = notOfferedText(skipped, left, partial, rechecked);
  el.hidden = !text;
  el.textContent = text;
}

let linked = null;
// `rom` plays the lifted ROM; `none` shows no game and asks for a ROM.
function play(mode, found = null, skipped = [], partial = []) {
  if (mode === "own") return playOwn();
  const romMode = mode === "rom";
  // A linked theme (the page's own, Turnips say) opens the game in it; a
  // ROM with none starts in its own art (own-look), whatever look an
  // earlier visit kept.
  // A look the link names (`look=`, the original's or one of the ROM's own
  // themes) is asked of the player, which opens it only if it has one of
  // that id. The settings the link asks for, held to the game's names, go
  // as `profile=`. Every value is one share.js checked, and the address is
  // built by URLSearchParams.
  if (romMode) {
    const q = new URLSearchParams({ files: `idb:${rom.game}` });
    if (link?.look && link.look !== "original") q.set("theme", link.look);
    else if (linked?.theme && !skipped.some((t) => t.id === linked.theme.id)) q.set("theme", linked.theme.id);
    else q.set("own-look", "");
    if (linked?.profile) q.set("profile", linked.profile);
    q.set("scale", "fit");
    frame.src = `${FRAME}?${q.toString().replace(/%3A/gi, ":").replace(/own-look=(&|$)/, "own-look$1")}`;
  } else frame.removeAttribute("src");
  frame.hidden = !romMode;
  if (facts) {
    $("title").textContent = facts.title;
    document.title = facts.title;
  }
  $("mode").textContent = romMode
    ? `Playing your own ${romName(found)} ${kindOf().noun}: ${foundText(found)}, kept in this browser only`
    : "No ROM loaded";
  $("back").hidden = !romMode;
  $("start").hidden = romMode;
  if (romMode) showSkipped(skipped, partial);
  else {
    $("skipped").hidden = true;
    $("skipped").textContent = "";
    showAboutThemes(linked?.notOffered ?? []);
  }
}

// A game that plays from its own files, in the frame: no `files`, so the
// frame reads the installed package's play/ files; the link's look and
// settings as for a ROM. The picker and "Unload ROM" are never shown.
function playOwn() {
  const q = new URLSearchParams();
  if (link?.look && link.look !== "original") q.set("theme", link.look);
  else q.set("own-look", "");
  if (linked?.profile) q.set("profile", linked.profile);
  q.set("scale", "fit");
  frame.src = `${FRAME}?${q.toString().replace(/own-look=(&|$)/, "own-look$1")}`;
  frame.hidden = false;
  $("title").textContent = facts.title;
  document.title = facts.title;
  $("mode").textContent = `Playing ${facts.title}${facts.madeBy ? `, made by ${facts.madeBy}` : ""}: it plays from its own files, with no ROM`;
  $("back").hidden = true;
  $("start").hidden = true;
  showSkipped();
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
  if (ownFiles()) {
    say(`${facts.title} plays from its own files: there is no ROM to load.`, "warn");
    return;
  }
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
      moduleUrl: LIFT_MODULE,
      romPath: kindOf().romPath,
      templateFiles: playTemplates(linked.files),
      extraThemes: linked.themes ?? (linked.theme ? [linked.theme] : []),
    });
    if (!r.ok) {
      say(r.error + bringLine(facts?.rom), "error");
      return;
    }
    const f = r.found;
    const model = f ? ` Playing as ${f.template_name ?? f.template}.` : "";
    say(
      `${romName(f)}: ${foundText(f)}. Read in ${Math.round(r.ms ?? 0)} ms, kept in this browser.${model}` +
        (f?.warning ? ` ${f.warning}` : ""),
      f?.warning ? "warn" : "ok",
    );
    // An arcade game's frame powers its board's sound on from the romset
    // (preview.js BoardSound): its bytes, kept in this page only.
    romBytes = rom?.kind === "zip" ? (typeof file.arrayBuffer === "function" ? await file.arrayBuffer() : file) : null;
    play("rom", f, r.looks_skipped ?? [], r.looks_partial ?? []);
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
  const what = link && !link.own ? link.game.name : (page?.title ?? "The game");
  if (link && !link.own) $("rom-help").textContent = "";
  say(`${what} couldn't be installed: ${err.message ?? err}.`, "error");
}
window.__page = { play, liftRom: async (...a) => (await lift()).liftRom(...a), showSkipped, installing };
// "What is this?": answered in this browser from the game's package.
whatIsThis({ frame, installing });
// A game that plays from its own files plays at once; any other, no game
// until a ROM, unless one picked while the page loaded already plays.
if (started && ownFiles()) play("own");
else if (!frame.getAttribute("src")) play("none");

// Kept from an earlier visit: the ROM plays at once, lifted again, so it
// plays with this page's themes and fixes, not the ones of the page that
// first lifted it. Files an older page kept without the ROM can't be
// lifted again, so they are dropped and the visitor loads the file once
// more. IndexedDB can be refused (some private windows) or, in some Safari
// versions, never answer; either way the page waits for a ROM without the
// buttons, and nothing here holds up the picker.
const { storedRom, loadStored, clearStored } = started && !ownFiles() ? await lift().catch(() => ({})) : {};
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
