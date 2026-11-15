// The page's address, read and written (docs/web.md, "One page, any game"):
// which game, which theme or look, and the settings a link starts with.
// Everything read from an address is untrusted: each value is checked
// against a strict shape here before any request names it, and against the
// game's own lists (the source's catalog and by-game list, the game's
// profiles.toml) by page.js once those are read. Nothing here builds HTML.
// Also the game's page facts read from its package (play/page.json, or the
// lift's files for a package from before page.json).
//
// No imports and no browser objects, so the tests run it under Node
// (share.test.mjs).

// A package name, namespace/name: lower-case letters, digits, `-` and `.`,
// each part starting with a letter or a digit (gp-package's split_name, and
// no `..`).
const NAME = /^[a-z0-9][a-z0-9.-]{0,63}\/[a-z0-9][a-z0-9.-]{0,63}$/;
const VERSION = /^\d{1,6}\.\d{1,6}\.\d{1,6}$/;
// A mode, a switch or a look's id.
const WORD = /^[A-Za-z0-9_-]{1,64}$/;
// The keys a link may have; any other is left out.
export const LINK_KEYS = ["g", "c", "look", "mode", "on"];
const MAX_SWITCHES = 64;

/** NAME or NAME@VERSION, checked, as { name, version }; null if it isn't one. */
export function packageRef(v) {
  if (typeof v !== "string" || v.length > 140) return null;
  const at = v.indexOf("@");
  const name = at < 0 ? v : v.slice(0, at);
  const version = at < 0 ? null : v.slice(at + 1);
  if (!NAME.test(name) || name.includes("..")) return null;
  if (version !== null && !VERSION.test(version)) return null;
  return { name, version };
}

/** A link's choices from `search` (location.search), each checked for its
 * shape only: { game, content, look, mode, on, ignored }. `ignored` names
 * the keys left out (unknown, given twice, or not of their shape), never
 * their values. */
export function readLink(search) {
  const q = new URLSearchParams(search);
  const out = { game: null, content: null, look: null, mode: null, on: null, ignored: [] };
  const seen = new Set();
  for (const key of q.keys()) {
    if (seen.has(key)) continue;
    seen.add(key);
    if (!LINK_KEYS.includes(key) || q.getAll(key).length > 1) {
      out.ignored.push(LINK_KEYS.includes(key) ? key : "an unknown key");
      continue;
    }
    const v = q.get(key);
    if (key === "g" || key === "c") {
      const ref = packageRef(v);
      if (ref) out[key === "g" ? "game" : "content"] = ref;
      else out.ignored.push(key);
    } else if (key === "look" || key === "mode") {
      if (WORD.test(v)) out[key] = v;
      else out.ignored.push(key);
    } else if (key === "on") {
      const parts = v.split(",").filter((s) => s !== "");
      if (parts.length <= MAX_SWITCHES && parts.every((s) => WORD.test(s))) out.on = [...new Set(parts)];
      else out.ignored.push(key);
    }
  }
  out.ignored = [...new Set(out.ignored)];
  return out;
}

/** The modes and switches a profiles.toml offers: { presets, switches },
 * each a Set of names. `retro` is always a mode and isn't listed. Reads
 * only the table headers and the switches' `name` lines, which is all the
 * format (docs/settings.md) puts names in. */
export function profileNames(text) {
  const presets = new Set();
  const switches = new Set();
  let table = "";
  for (const raw of String(text ?? "").split("\n")) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    let m;
    if ((m = /^\[\[\s*([A-Za-z0-9_.-]+)\s*\]\]$/.exec(line))) table = m[1];
    else if ((m = /^\[\s*([A-Za-z0-9_.-]+)\s*\]$/.exec(line))) {
      table = m[1];
      const p = /^presets\.([A-Za-z0-9_-]+)$/.exec(table);
      if (p) presets.add(p[1]);
    } else if (table === "groups.switches" && (m = /^name\s*=\s*"([A-Za-z0-9_-]+)"/.exec(line))) switches.add(m[1]);
  }
  return { presets, switches };
}

/** The settings a link asks for, held to the game's own names: { profile,
 * dropped }. `profile` is what the player takes as `profile=` (a mode, or
 * switches joined by `+`), null for none; `dropped` names each mode or
 * switch the game doesn't have, which are left out. `on` wins over
 * `mode`. */
export function linkSettings(link, names) {
  const dropped = [];
  if (link.on) {
    const on = link.on.filter((s) => {
      const ok = names.switches.has(s);
      if (!ok) dropped.push(s);
      return ok;
    });
    if (link.mode) dropped.push(link.mode);
    return { profile: on.length ? on.join("+") : link.on.length ? null : "retro", dropped };
  }
  if (link.mode) {
    if (link.mode === "retro" || names.presets.has(link.mode)) return { profile: link.mode, dropped };
    dropped.push(link.mode);
  }
  return { profile: null, dropped };
}

/** A link to `base` (the page's /p) for what is playing: the game, the
 * theme package (`content`, NS/NAME) or another look's id (`look`, such as
 * `original`), and the profile the player runs (a mode, or switches joined
 * by `+`), each held to its shape and to the game's names first. Nothing
 * else goes in it: no ROM, no save, nothing of the player. */
export function shareLink(base, { game, content = null, look = null, profile = null, names = null }) {
  const g = packageRef(game);
  if (!g || g.version) throw new Error("a share link needs the game's name");
  const u = new URL(base);
  u.search = "";
  u.hash = "";
  const q = new URLSearchParams();
  q.set("g", g.name);
  const c = content ? packageRef(content) : null;
  if (c && !c.version) q.set("c", c.name);
  else if (look && WORD.test(look)) q.set("look", look);
  if (profile && names) {
    const parts = String(profile).split("+");
    if (parts.length === 1 && (parts[0] === "retro" || names.presets.has(parts[0]))) q.set("mode", parts[0]);
    else {
      const on = parts.filter((s) => WORD.test(s) && names.switches.has(s));
      if (on.length) q.set("on", on.join(","));
      else q.set("mode", "retro");
    }
  }
  // Commas and slashes are kept as they are, so a link reads plainly.
  u.search = q.toString().replace(/%2F/gi, "/").replace(/%2C/gi, ",");
  return u.href;
}

const str = (v, max) => typeof v === "string" && v.length > 0 && v.length <= max;
// The id the lift keeps a game's files under in this browser.
const GAME_ID = /^[a-z0-9][a-z0-9_-]{0,63}$/;

/** A source list's [[source]] entries (sources/nes.toml): { id, title,
 * revision, sha256 } each, strings only. */
export function sourcesOf(text) {
  const out = [];
  let cur = null;
  for (const raw of String(text ?? "").split("\n")) {
    const line = raw.trim();
    if (line === "[[source]]") out.push((cur = {}));
    else if (/^\[/.test(line)) cur = null;
    else if (cur) {
      const m = /^(id|title|revision|sha256)\s*=\s*"([^"\\]*)"\s*$/.exec(line);
      if (m) cur[m[1]] = m[2];
    }
  }
  return out;
}

const text_ = (b) => new TextDecoder().decode(b instanceof Uint8Array ? b : new Uint8Array(b));

/** The facts of a game that plays from its own files (play/page.json's
 * rom.kind `none`: a new game, made with its own pictures): its title,
 * player and id, who made it (`made_by`) and what it is (`about`), each
 * held to its shape. Such a package holds the files the player reads
 * (bundle.json, preview.json) and no lift: it is never played from a ROM.
 * Throws an error in words for one the page can't read. */
function ownGameOf(j, files) {
  const r = j.rom ?? {};
  if (!str(j.title, 120) || !WORD.test(j.player ?? "") || !GAME_ID.test(r.game ?? "")) throw new Error("its play/page.json is missing a title, a player or the game's id");
  if ([...files.keys()].some((k) => k.startsWith("lift/"))) throw new Error("its play/page.json says it plays from its own files, but it holds a ROM's lift");
  if (!files.has("bundle.json") || !files.has("preview.json")) throw new Error("its package has no bundle.json and preview.json to play");
  const howto = Array.isArray(j.howto) ? j.howto : [];
  if (howto.length > 8 || !howto.every((h) => str(h, 400))) throw new Error("its play/page.json's howto isn't a few lines of text");
  for (const k of ["made_by", "about"]) if (j[k] !== undefined && !str(j[k], k === "about" ? 600 : 120)) throw new Error(`its play/page.json's ${k} isn't a line of text`);
  return { title: j.title, player: j.player, rom: { kind: "none", game: r.game }, howto, trademark: null, madeBy: j.made_by ?? null, about: j.about ?? null };
}

/** An arcade game's MAME sets from its play/page.json's `rom.sets` (what
 * `gp roms sets` says of each: its name, title, identity and the files its
 * zip must hold, by name, size and SHA-1), each held to its shape; null
 * when it has none. Throws an error in words for a list the page can't
 * read. */
function setsOf(v) {
  if (v === undefined) return null;
  const file = (f) => str(f?.name, 200) && Number.isInteger(f.size) && f.size > 0 && /^[0-9a-f]{40}$/.test(f.sha1 ?? "");
  const ok = (x) => WORD.test(x?.name ?? "") && str(x.title, 200) && /^[0-9a-f]{64}$/.test(x.sha256 ?? "") && Array.isArray(x.files) && x.files.length > 0 && x.files.length <= 64 && x.files.every(file);
  if (!Array.isArray(v) || v.length > 20 || !v.every(ok)) throw new Error("its play/page.json's rom.sets isn't a list of MAME sets and their files");
  return v.map((x) => ({ name: x.name, title: x.title, sha256: x.sha256, files: x.files.map((f) => ({ name: f.name, size: f.size, sha1: f.sha1 })) }));
}

/** The game's page facts from its play files (`files`, path under play/ to
 * bytes): play/page.json when the package has one, else the lift's
 * game.json and sources-nes.toml (a package from before page.json, played
 * by the NES player). `title` is the package's title from the source, the
 * fallback name. Each field is held to its shape, and `rom.game` to the
 * lift's own index.json; a game that plays from its own files (rom.kind
 * `none`) has no lift and is read by ownGameOf. Throws an error in words
 * for a package the page can't read. */
export function gamePageOf(files, title) {
  const own = files.get("page.json");
  let j = null;
  if (own) {
    try {
      j = JSON.parse(text_(own));
    } catch {
      throw new Error("its play/page.json isn't JSON");
    }
    if (j?.format !== 1) throw new Error("its play/page.json isn't format 1");
    if (j.rom?.kind === "none") return ownGameOf(j, files);
  }
  const lift = files.get("lift/index.json");
  if (!lift) throw new Error("its package has no lift/index.json");
  const liftGame = JSON.parse(text_(lift)).game;
  let facts;
  if (own) {
    const r = j.rom ?? {};
    const known = Array.isArray(r.known) ? r.known : [];
    const sha = r.known_sha256 && typeof r.known_sha256 === "object" ? r.known_sha256 : {};
    if (!str(j.title, 120) || !WORD.test(j.player ?? "") || !WORD.test(r.kind ?? "") || !GAME_ID.test(r.game ?? "") || !str(r.name, 200) || !str(r.file, 600))
      throw new Error("its play/page.json is missing a title, a player or the ROM's kind, game, name or file");
    if (known.length > 50 || !known.every((k) => str(k, 200))) throw new Error("its play/page.json's rom.known isn't a list of names");
    if (Object.keys(sha).length > 50 || !Object.entries(sha).every(([k, v]) => str(k, 200) && /^[0-9a-f]{64}$/.test(v))) throw new Error("its play/page.json's rom.known_sha256 isn't names and hashes");
    const howto = Array.isArray(j.howto) ? j.howto : [];
    if (howto.length > 8 || !howto.every((h) => str(h, 400))) throw new Error("its play/page.json's howto isn't a few lines of text");
    if (j.trademark !== undefined && !str(j.trademark, 600)) throw new Error("its play/page.json's trademark isn't a line of text");
    const sets = setsOf(r.sets);
    facts = {
      title: j.title,
      player: j.player,
      rom: { kind: r.kind, game: r.game, title: r.name, name: r.name, known: known, file: r.file, known_sha256: sha, ...(sets ? { sets } : {}) },
      howto,
      trademark: j.trademark ?? null,
    };
  } else {
    const g = files.get("lift/game.json");
    const s = files.get("lift/sources-nes.toml");
    if (!g || !s) throw new Error("its package has no play/page.json, and no lift/game.json and sources-nes.toml to read in its place");
    const game = JSON.parse(text_(g));
    const sources = sourcesOf(text_(s));
    const names = [];
    const known = {};
    for (const tid of Array.isArray(game.templates) ? game.templates : []) {
      const src = sources.find((x) => x.id === tid);
      if (!src || !src.title) continue;
      const n = src.revision ? `${src.title} (${src.revision})` : src.title;
      names.push(n);
      if (/^[0-9a-f]{64}$/.test(src.sha256 ?? "")) known[n] = src.sha256;
    }
    const name = str(game.title, 200) ? game.title : title;
    facts = {
      title: str(title, 120) ? title : name,
      player: "nes",
      rom: {
        kind: "nes",
        game: game.game,
        title: name,
        name,
        known: names,
        file: `an .nes file of any dump: ${names.join(", ")}, or another revision`,
        known_sha256: known,
      },
      howto: [],
      trademark: null,
    };
  }
  if (!GAME_ID.test(facts.rom.game ?? "")) throw new Error("its ROM's game id isn't a plain name");
  if (facts.rom.game !== liftGame) throw new Error(`its page facts are for ${facts.rom.game}, but its lift is for ${liftGame}`);
  return facts;
}
