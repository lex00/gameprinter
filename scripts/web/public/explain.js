// "What is this?" on the public page, answered in this browser from the
// game's own package (docs/web.md, "What is this?"). The mechanic page's
// button asks the agent, who runs `gp element explain` with the ROM and the
// tools (issue #10); the public page has neither, so it says what the
// package's own files say about the thing under the spot:
//
// - the game's model (model.toml: the one the lift played, under
//   play/lift/TEMPLATE/, else the package's own): what each kind of thing
//   is, its role, its box, what touching it does, the parts or the hook
//   that move it, and how it was worked out (its evidence note);
// - the hooks' own table (ext/src/*.rs, the doc lines `| \`hook\` | kind |
//   words |`): what the hook that moves it does, in words;
// - the state mapping (mapping.toml): where the original keeps it in RAM,
//   as addresses and the mapping's own notes;
// - the spot itself, from the model running in the page (preview.js,
//   `__gp.spot`): the bodies drawn there with their kind, place and facing,
//   the HUD field under it, or the level.
//
// None of it is read from the ROM: these are the package's published
// files, which hold addresses and words, never the game's bytes. What only
// the original's run can say (its sprite table, the routine that drew the
// sprite and the inputs it read) the answer leaves to the tools, and says
// so. A package without a model, or a model that doesn't describe the
// thing, answers that the game can't explain itself yet.
//
// Pure: the tests (explain.test.mjs) drive it with made-up files.

/** The words for "this game can't explain itself yet", with why. */
export const CANNOT = "This game can't explain itself yet";

/** A HUD's cell, in pixels: the NES's and the board's character size. */
const CELL = 8;

// ---- A TOML reader, for the package's model.toml and mapping.toml ----

/** Reads TOML text into plain objects: tables, arrays of tables, dotted
 * and quoted keys, strings of each kind, numbers, booleans, arrays and
 * inline tables; a date stays its text. Throws with the line on text it
 * can't read. */
export function readToml(text) {
  const s = String(text ?? "");
  const root = {};
  let cur = root;
  let i = 0;
  const line = () => s.slice(0, i).split("\n").length;
  const fail = (m) => {
    throw new Error(`line ${line()}: ${m}`);
  };
  const blank = () => {
    while (i < s.length && (s[i] === " " || s[i] === "\t")) i++;
  };
  const gap = () => {
    for (;;) {
      blank();
      if (s[i] === "#") while (i < s.length && s[i] !== "\n") i++;
      if (s[i] === "\n" || s[i] === "\r") i++;
      else return;
    }
  };
  const ESC = { b: "\b", t: "\t", n: "\n", f: "\f", r: "\r", '"': '"', "\\": "\\" };
  const basic = (end) => {
    let out = "";
    while (i < s.length) {
      if (s.startsWith(end, i)) {
        i += end.length;
        return out;
      }
      if (s[i] === "\n" && end === '"') fail("a string runs past its line");
      const c = s[i++];
      if (c !== "\\") {
        out += c;
        continue;
      }
      const e = s[i++];
      if (e in ESC) out += ESC[e];
      else if (e === "u" || e === "U") {
        const n = e === "u" ? 4 : 8;
        out += String.fromCodePoint(parseInt(s.slice(i, i + n), 16));
        i += n;
      } else if (end === '"""' && /[ \t\r\n]/.test(e)) {
        while (/[ \t\r\n]/.test(s[i] ?? "")) i++;
      } else fail(`an unknown escape \\${e}`);
    }
    fail("a string never ends");
  };
  const literal = (end) => {
    const at = s.indexOf(end, i);
    if (at < 0) fail("a string never ends");
    const out = s.slice(i, at);
    i = at + end.length;
    return out;
  };
  const str = () => {
    for (const q of ['"""', "'''"]) {
      if (s.startsWith(q, i)) {
        i += 3;
        if (s[i] === "\r") i++;
        if (s[i] === "\n") i++;
        return q === '"""' ? basic(q) : literal(q);
      }
    }
    const q = s[i++];
    return q === '"' ? basic('"') : literal("'");
  };
  const key = () => {
    const parts = [];
    for (;;) {
      blank();
      if (s[i] === '"' || s[i] === "'") parts.push(str());
      else {
        const m = /^[A-Za-z0-9_-]+/.exec(s.slice(i, i + 256));
        if (!m) fail("a key was expected");
        parts.push(m[0]);
        i += m[0].length;
      }
      blank();
      if (s[i] !== ".") return parts;
      i++;
    }
  };
  const value = () => {
    blank();
    const c = s[i];
    if (c === '"' || c === "'") return str();
    if (c === "[") {
      i++;
      const out = [];
      for (;;) {
        gap();
        if (s[i] === "]") {
          i++;
          return out;
        }
        out.push(value());
        gap();
        if (s[i] === ",") i++;
        else if (s[i] !== "]") fail("an array needs , or ]");
      }
    }
    if (c === "{") {
      i++;
      const out = {};
      blank();
      if (s[i] === "}") {
        i++;
        return out;
      }
      for (;;) {
        const k = key();
        if (s[i++] !== "=") fail("an inline table needs =");
        put(out, k, value());
        blank();
        const d = s[i++];
        if (d === "}") return out;
        if (d !== ",") fail("an inline table needs , or }");
      }
    }
    const m = /^[^\s,\]}#]+/.exec(s.slice(i, i + 64));
    if (!m) fail("a value was expected");
    i += m[0].length;
    const w = m[0];
    if (w === "true") return true;
    if (w === "false") return false;
    const n = w.replace(/_/g, "");
    if (/^0x[0-9a-f]+$/i.test(n)) return parseInt(n.slice(2), 16);
    if (/^0o[0-7]+$/.test(n)) return parseInt(n.slice(2), 8);
    if (/^0b[01]+$/.test(n)) return parseInt(n.slice(2), 2);
    if (/^[+-]?inf$/.test(n)) return n.startsWith("-") ? -Infinity : Infinity;
    if (/^[+-]?nan$/.test(n)) return NaN;
    if (/^[+-]?(\d+\.?\d*|\.\d+)(e[+-]?\d+)?$/i.test(n)) return Number(n);
    return w;
  };
  // The table a path names: the last of an array of tables.
  const walk = (obj, parts) => {
    let o = obj;
    for (const p of parts) {
      if (!(p in o)) o[p] = {};
      o = o[p];
      if (Array.isArray(o)) o = o[o.length - 1];
      if (typeof o !== "object" || o === null) fail(`${parts.join(".")} isn't a table`);
    }
    return o;
  };
  const put = (obj, parts, v) => {
    const o = walk(obj, parts.slice(0, -1));
    o[parts[parts.length - 1]] = v;
  };
  for (;;) {
    gap();
    if (i >= s.length) return root;
    if (s[i] === "[") {
      const many = s[i + 1] === "[";
      i += many ? 2 : 1;
      const parts = key();
      if (s[i] !== "]" || (many && s[i + 1] !== "]")) fail("a table header needs ]");
      i += many ? 2 : 1;
      if (many) {
        const o = walk(root, parts.slice(0, -1));
        const last = parts[parts.length - 1];
        if (!Array.isArray(o[last])) o[last] = [];
        o[last].push({});
        cur = o[last][o[last].length - 1];
      } else cur = walk(root, parts);
      continue;
    }
    const k = key();
    if (s[i++] !== "=") fail("a key needs =");
    put(cur, k, value());
    blank();
    if (s[i] === "#") while (i < s.length && s[i] !== "\n") i++;
    if (i < s.length && s[i] !== "\n" && s[i] !== "\r") fail("one key a line");
  }
}

// ---- The package's hooks, in words ----

/** What each hook does, from the hooks' own table in their source's doc
 * lines (`//! | \`dk_barrel\` | barrel | Held, then rolls ... |`), by the
 * hook's name: { file, kinds, words }. `sources` is a Map of path to text. */
export function hookWords(sources) {
  const out = new Map();
  for (const [file, text] of sources ?? []) {
    for (const raw of String(text).split("\n")) {
      const m = /^\s*\/\/[/!]\s*\|\s*`([A-Za-z0-9_]+)`\s*\|([^|]*)\|(.*)\|\s*$/.exec(raw);
      if (m && !out.has(m[1])) out.set(m[1], { file, kinds: m[2].trim(), words: plain(m[3].trim()) });
    }
  }
  return out;
}

// Markdown's marks off a table cell: `code` and [`links`] read as words.
const plain = (t) => t.replace(/\[`?([^\]`]+)`?\]/g, "$1").replace(/`([^`]+)`/g, "$1");

// ---- The answer ----

const hex = (n) => `$${n.toString(16).toUpperCase().padStart(4, "0")}`;
const words = (name) => String(name).replace(/[_-]+/g, " ");
const sentence = (t) => {
  const x = String(t ?? "").trim();
  if (!x) return "";
  return /[.!?]$/.test(x) ? x : `${x}.`;
};
// The first `n` sentences of a long note; the rest goes under "more".
const firstSentences = (t, n) => {
  const parts = String(t ?? "").match(/[^.]+(?:\.(?!\d)|$)+/g) ?? [];
  return parts.slice(0, n).join("").trim();
};
// What touching a kind does, in words, by the model's action.
const CONTACT = { "kill-player": "costs the player a life", "hurt-player": "hurts the player" };
const ROLES = {
  player: "the one you play",
  enemy: "an enemy",
  hazard: "a hazard",
  npc: "a character the game moves",
  "power-up": "a power-up",
  item: "an item",
  behavior: "a part of the game's workings, not a thing you see",
};
// The model's parts that say how a kind moves, by their table's name.
const NOT_PARTS = new Set(["sprite", "hitbox", "contact", "ext_params", "ext_rows", "ext_tables", "copy", "spawn", "states"]);

/** The model whose [model] name is `name` among the package's model files
 * (a Map of path to text), else the package's own model.toml, else the
 * first. { path, model } or null when the package has none. */
export function pickModel(files, name) {
  const texts = [...(files ?? [])].filter(([p]) => p === "model.toml" || /^play\/lift\/[^/]+\/model\.toml$/.test(p));
  if (!texts.length) return null;
  const read = texts.map(([path, text]) => {
    try {
      return { path, model: readToml(text) };
    } catch (err) {
      return { path, error: String(err.message ?? err) };
    }
  });
  const ok = read.filter((r) => r.model);
  return ok.find((r) => r.model.model?.name === name) ?? ok.find((r) => r.path === "model.toml") ?? ok[0] ?? read[0];
}

// A mapping field's name in words: a hook's word `ext[N]` is "its hook's word N".
const fieldName = (n) => String(n).replace(/^ext\[(\d+)\]$/, "its hook's word $1");

// Where the original keeps a kind, from the state mapping: its own fields
// (`entities.KIND[n].FIELD`) and the slots that hold it ([slots] and
// [[slots.group]] whose kind is it), each with its RAM addresses.
function originalOf(mapping, kind) {
  if (!mapping) return null;
  const fields = [];
  const at = (f) => (Array.isArray(f.ram) ? f.ram : []).map(hex);
  for (const f of mapping.field ?? []) {
    const m = new RegExp(`^entities\\.${kind}(?:\\[\\d+\\])?\\.(.+)$`).exec(f.path ?? "");
    if (m) fields.push({ name: fieldName(m[1]), at: at(f), note: f.note ?? "" });
  }
  let slot = null;
  const groups = [mapping.slots, ...(mapping.slots?.group ?? [])].filter(Boolean);
  for (const g of groups) {
    if (!(g.kind ?? []).some((k) => k.entity === kind)) continue;
    slot = { name: g.name ?? "slots", count: g.count, stride: g.active_stride, first: typeof g.active === "number" ? hex(g.active) : null, note: g.note ?? "" };
    for (const f of g.field ?? []) fields.push({ name: fieldName(f.path), at: at(f), stride: f.stride, note: f.note ?? "" });
    break;
  }
  return fields.length || slot ? { fields, slot } : null;
}

function bodyAnswer(hit, model, mapping, hooks, spot) {
  const e = model.entities?.[hit.kind];
  if (!e || !e.description) {
    return { can: false, why: `${CANNOT}: its model has no words for the ${words(hit.kind)} drawn here (model.toml, [entities.${hit.kind}] description).`, kind: hit.kind };
  }
  const lines = [];
  const more = [];
  const role = ROLES[e.role] ?? (e.role ? words(e.role) : null);
  const box = e.hitbox ? [e.hitbox.w, e.hitbox.h] : e.size;
  // The heading is the description; a short one isn't said twice.
  const title = sentence(e.description).replace(/\.$/, "");
  const firstLine = e.description.length > 60 ? sentence(e.description) : "";
  lines.push(
    [firstLine, role ? `It is ${role}.` : "", Array.isArray(box) ? `Its touch box is ${box[0]} by ${box[1]} pixels${e.hitbox && e.size ? `, in a ${e.size[0]} by ${e.size[1]} picture` : ""}.` : ""]
      .filter(Boolean)
      .join(" "),
  );
  lines.push(`Now it is at x ${hit.x}, y ${hit.y}, facing ${hit.facing}${spot.level ? `, on level ${spot.level}` : ""}${hit.alive === false ? ", out of play" : ""} (frame ${spot.frame}).`);
  if (e.contact && typeof e.contact === "object") {
    const c = Object.entries(e.contact).map(([k, v]) => (k === "otherwise" ? `touching it ${CONTACT[v] ?? `does ${words(v)}`}` : `${words(k)}: ${words(Array.isArray(v) ? v.join(", ") : v)}`));
    if (c.length) lines.push(sentence(c.join("; ").replace(/^./, (x) => x.toUpperCase())));
  }
  if (e.ext) {
    const h = hooks.get(e.ext);
    lines.push(h ? `It moves by the game's own code, the hook ${e.ext} (${h.file}): ${sentence(h.words)}` : `It moves by the game's own code, the hook ${e.ext} (in the package's ext/ folder).`);
  } else {
    const parts = Object.entries(e)
      .filter(([k, v]) => v && typeof v === "object" && !Array.isArray(v) && !NOT_PARTS.has(k))
      .map(([k]) => words(k));
    if (parts.length) lines.push(`The model's own parts move it: ${parts.join(", ")}.`);
  }
  const ev = model.evidence?.[`entities.${hit.kind}`];
  if (ev?.note) {
    lines.push(`How it was worked out (${ev.source ?? "the model's evidence"}${typeof ev.confidence === "number" ? `, ${ev.confidence} in 100 sure` : ""}): ${sentence(firstSentences(ev.note, 2))}`);
    more.push({ head: "The whole evidence note", text: ev.note });
  }
  const o = originalOf(mapping, hit.kind);
  if (o) {
    const at = o.fields.filter((f) => f.at.length).slice(0, 4).map((f) => `${words(f.name)} at ${f.at.join(", ")}${f.stride ? ` (every ${f.stride} bytes a slot)` : ""}`);
    const slot = o.slot ? `It is one of ${o.slot.count ?? "the"} slots${o.slot.first ? ` from ${o.slot.first}` : ""}${o.slot.stride ? `, ${o.slot.stride} bytes apart` : ""}. ` : "";
    lines.push(`In the original, the state mapping reads it from RAM. ${slot}${at.length ? `${sentence(at.join("; ").replace(/^./, (x) => x.toUpperCase()))}` : ""}`.trim());
    const notes = [o.slot?.note, ...o.fields.map((f) => (f.note ? `${words(f.name)}: ${f.note}` : ""))].filter(Boolean);
    if (notes.length) more.push({ head: "The mapping's notes", text: notes.join("\n\n") });
  }
  return { can: true, title, kind: hit.kind, lines, more };
}

function hudAnswer(spot, model) {
  const hud = model.present?.hud;
  if (!hud) return null;
  const [x, y] = spot.hud;
  const [cw, ch] = hud.size ?? [0, 0];
  if (x < 0 || y < 0 || x >= cw * CELL || y >= ch * CELL) return null;
  const under = (hud.field ?? []).filter((f) => {
    if (!Array.isArray(f.at)) return false;
    const n = f.digits ?? String(f.text ?? "").length ?? 1;
    return x >= f.at[0] * CELL && x < (f.at[0] + Math.max(1, n)) * CELL && y >= f.at[1] * CELL && y < (f.at[1] + 1) * CELL;
  });
  const counters = [...new Set(under.map((f) => f.counter).filter(Boolean))];
  if (!counters.length) {
    return { can: true, title: "The HUD", kind: null, lines: ["This is the HUD's own picture, drawn behind its numbers. Nothing the game counts is shown at this spot."], more: [] };
  }
  const lines = counters.map((c) => {
    const d = model.counters?.[c]?.description;
    return `${words(c)}: ${d ? sentence(d) : "the model says no more of it."}`;
  });
  if (counters.length > 1) lines.unshift(`The HUD shows one of these here, by the game's state (${counters.map(words).join(", ")}).`);
  return { can: true, title: `The HUD's ${words(counters[0])}`, kind: null, lines, more: [] };
}

/**
 * The answer for a spot (`__gp.spot`'s result) from the package's files
 * (a Map of path to text: model.toml or play/lift/TEMPLATE/model.toml,
 * mapping.toml, ext/src/*.rs). `modelName` is the model the page plays
 * (`__gp.game`). Either { can: true, title, lines, more, note, kind, box,
 * model } or
 * { can: false, why }.
 */
export function explainSpot(spot, files, modelName) {
  if (!spot || !Array.isArray(spot.hits)) return { can: false, why: `${CANNOT}: its player can't say what it draws where.` };
  const picked = pickModel(files, modelName);
  if (!picked) return { can: false, why: `${CANNOT}: its package has no model.toml, the file that says what each thing in the game is.` };
  if (!picked.model) return { can: false, why: `${CANNOT}: its ${picked.path} couldn't be read (${picked.error}).` };
  const model = picked.model;
  let mapping = null;
  if (files.has("mapping.toml")) {
    try {
      mapping = readToml(files.get("mapping.toml"));
    } catch {
      mapping = null;
    }
  }
  const sources = new Map([...files].filter(([p]) => /^ext\/src\/.+\.rs$/.test(p)));
  const hooks = hookWords(sources);
  const seen = new Set();
  const hits = spot.hits.filter((h) => h.kind && !seen.has(h.kind) && seen.add(h.kind));
  const also = hits.slice(1).map((h) => model.entities?.[h.kind]?.description ?? words(h.kind));
  let a = null;
  if (hits.length) a = bodyAnswer(hits[0], model, mapping, hooks, spot);
  else a = hudAnswer(spot, model);
  if (!a) {
    const lv = model.levels?.[spot.level];
    a = {
      can: true,
      title: "The stage",
      kind: null,
      lines: [`Nothing the game moves is drawn here: this is the stage${spot.level ? `, ${spot.level}` : ""}.${lv?.description ? ` ${sentence(lv.description)}` : ""}`, "Pick a character, an enemy or a number to hear about it."],
      more: [],
    };
  }
  if (!a.can) return a;
  if (also.length) a.lines.push(`Also under the spot: ${also.join(", ")}.`);
  a.note =
    "Answered in this browser from the game's own package, with no server. What the original's code does at this moment (its sprite table, the routine that drew it and what that routine read) is for the tools: gp element explain reads it from your ROM.";
  a.model = picked.path;
  a.box = spot.hits.find((h) => h.kind === a.kind)?.box ?? null;
  return a;
}

/** The files of an installed package the answer reads, by path. */
export const wantedFile = (path) => path === "model.toml" || path === "mapping.toml" || /^play\/lift\/[^/]+\/model\.toml$/.test(path) || /^ext\/src\/.+\.rs$/.test(path);
