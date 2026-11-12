// What the workspace says to a person, in game terms (docs/inspector.md,
// "The game view"). Every sentence is a template filled from the model's
// own words: its entity names and roles, its levels and counters, the
// mechanic catalog's sentences and units, and what capture found. Nothing
// here names a game; a new game gets the same sentences with its names.
// Model paths, labels, routines and metric names stay in the details.

import type { GameWords, MechanicWords } from "./api";
import type { Found, LiveState, Metric, Question } from "./livefold";
import type { Explanation, PlayState } from "./playfold";

/** A name as words: `cheep_cheep_red` -> `cheep cheep red`. */
export function words(name: string): string {
  return name.replace(/\[\d+\]/g, "").replace(/[_.-]+/g, " ").trim();
}

const cap = (s: string) => (s ? s[0].toUpperCase() + s.slice(1) : s);

/** The entity a role names first, such as the player. */
export function byRole(g: GameWords | undefined, role: string): string | undefined {
  return g?.vocab.entities.find((e) => e.role === role)?.name;
}

/**
 * An entity as a person says it: the player's own name when it has one
 * ("Mario"), otherwise "the" and its name ("the slug", "the player").
 */
export function thing(g: GameWords | undefined, name: string): string {
  const e = g?.vocab.entities.find((x) => x.name === name);
  if (e && e.role === "player" && e.name !== "player") return cap(words(e.name));
  return `the ${words(name)}`;
}

/** The player as a person says it. */
export function player(g: GameWords | undefined): string {
  const p = byRole(g, "player");
  return p ? thing(g, p) : "the player";
}

/** A level: "the level" when the game has one, else its name. */
export function level(g: GameWords | undefined, name: string): string {
  if (!g || g.vocab.levels.length <= 1) return "the level";
  return `level ${words(name)}`;
}

/** What a model path is about, as words: "the slug's patrol period", "the deaths count". */
export function subject(g: GameWords | undefined, path: string): { owner: string; field: string; ownerName?: string } {
  const parts = path.split(".");
  const rest = (n: number) => words(parts.slice(n).join(" "));
  switch (parts[0]) {
    case "entities":
      return { owner: thing(g, parts[1] ?? ""), field: rest(2), ownerName: parts[1] };
    case "levels":
      return { owner: level(g, parts[1] ?? ""), field: rest(2), ownerName: parts[1] };
    case "counters":
      return { owner: `the ${words(parts[1] ?? "")} count`, field: rest(2) };
    case "rules":
      return parts[2] === "states" ? { owner: `the game's ${words(parts[3] ?? "")} state`, field: rest(4) } : { owner: "the game's state", field: rest(2) };
    case "cameras":
      return { owner: "the camera", field: rest(2) };
    case "assets":
      return { owner: `the ${words(parts[1] ?? "")} graphics`, field: rest(2) };
    case "input":
      return { owner: "the controls", field: rest(1) };
    default:
      return { owner: words(parts[0] ?? ""), field: rest(1) };
  }
}

/** The path as one phrase: "the slug's patrol period", or "the deaths count". */
export function pathPhrase(g: GameWords | undefined, path: string): string {
  const s = subject(g, path);
  return s.field ? `${s.owner}'s ${s.field}` : s.owner;
}

/** The mechanic a path belongs to, and the parameter it is. */
export function mechanicOf(g: GameWords | undefined, path: string): { mechanic?: MechanicWords; param?: MechanicWords["params"][number] } {
  const row = g?.mechanics.paths.find((p) => p.path === path);
  const mechanic = row?.mechanic ? g?.mechanics.mechanics.find((m) => m.name === row.mechanic) : undefined;
  const last = path.split(".").pop() ?? "";
  const param = mechanic?.params.find((p) => p.name === last);
  return { mechanic, param };
}

/** How a question's picture shows a value: a line across the screen at that height, or none. */
export type Picture = "row" | "screen-column" | null;

/**
 * Questions in game terms, keyed by mechanic and parameter. The braces
 * are filled from the model: `{player}` is the player entity's name,
 * `{owner}` the entity or level the path is under, `{level}` its level.
 * A mechanic or parameter not listed is asked from its unit (below).
 */
export const ASK: { mechanic: string; param?: string; ask: string; picture?: Picture }[] = [
  { mechanic: "platformer.pit", param: "kill_below", ask: "Where does {player} fall out of {level}?", picture: "row" },
  { mechanic: "platformer.goal", ask: "Where does {level} end?" },
  { mechanic: "platformer.jump", param: "speed", ask: "How hard does {owner} jump?" },
  { mechanic: "platformer.jump", param: "gravity", ask: "How fast does {owner} come back down?" },
  { mechanic: "platformer.jump", param: "max_fall", ask: "How fast can {owner} fall at most?" },
  { mechanic: "platformer.run", param: "walk_speed", ask: "How fast does {owner} walk?" },
  { mechanic: "platformer.run", param: "run_speed", ask: "How fast does {owner} run?" },
  { mechanic: "platformer.run", param: "acceleration", ask: "How quickly does {owner} speed up?" },
  { mechanic: "platformer.patrol", ask: "How does {owner} pace back and forth?" },
  { mechanic: "platformer.contact", param: "stomp_depth", ask: "How far onto {owner} must {player} land to stomp it?" },
  { mechanic: "platformer.contact", param: "bounce", ask: "How high does {player} bounce off {owner}?" },
  { mechanic: "platformer.contact", ask: "How big is {owner} when it touches things?" },
  { mechanic: "core.camera", param: "lead", ask: "Where on the screen does the camera keep {player}?", picture: "screen-column" },
  { mechanic: "core.camera", ask: "How does the camera follow {player}?" },
  { mechanic: "core.flow", ask: "How long does the game stay in each state?" },
  { mechanic: "core.counters", ask: "When does {owner} change?" },
];

/** A question from a unit, for a parameter no template covers. */
function askByUnit(unit: string | undefined, what: string): string {
  switch (unit) {
    case "px":
      return `Where is ${what}?`;
    case "px/frame":
    case "px/frame^2":
      return `How fast is ${what}?`;
    case "frames":
      return `How long is ${what}?`;
    default:
      return `What is ${what}?`;
  }
}

/** A unit after a number, as a person reads it. */
export function unitWords(unit: string | undefined): string {
  switch (unit) {
    case "px":
      return "px";
    case "px/frame":
      return "px a frame";
    case "px/frame^2":
      return "px a frame, each frame";
    case "frames":
      return "frames";
    default:
      return unit ?? "";
  }
}

/** Where a candidate answer came from, in plain words. */
export function fromWords(from: string): string {
  const f = from.toLowerCase();
  if (f.includes("derive")) return "what the game's code says";
  if (f.includes("propose") || f.includes("measure") || f.includes("refine")) return "what the recordings show";
  if (f.includes("model")) return "what the model has now";
  return from;
}

export interface PlainQuestion {
  id: string;
  ask: string;
  /** The mechanic's own sentence, as a hint. */
  about?: string;
  picture: Picture;
  unit?: string;
  options: { choice: number; value: string; number?: number; label: string; from: string }[];
  /** Kept for the details: the model path and the pipeline's own text. */
  path?: string;
  text: string;
}

/** A question as a person reads it, with what each answer would show. */
export function plainQuestion(g: GameWords | undefined, q: Question): PlainQuestion {
  const path = q.path ?? q.id.replace(/^[a-z]+:/, "");
  const { mechanic, param } = mechanicOf(g, path);
  const s = subject(g, path);
  const t =
    ASK.find((a) => mechanic && a.mechanic === mechanic.name && a.param === param?.name) ??
    ASK.find((a) => mechanic && a.mechanic === mechanic.name && !a.param);
  const lvl = path.startsWith("levels.") ? s.owner : level(g, g?.vocab.levels[0]?.name ?? "");
  const fill = (tpl: string) => cap(tpl.replace(/\{player\}/g, player(g)).replace(/\{owner\}/g, s.owner).replace(/\{level\}/g, lvl));
  const ask = t ? fill(t.ask) : askByUnit(param?.unit, pathPhrase(g, path));
  const unit = param?.unit;
  return {
    id: q.id,
    ask,
    // The mechanic's sentence, its parameter named as the picture shows it.
    about: mechanic?.about && param ? mechanic.about.split(param.name).join((t?.picture ?? null) === "row" ? "the line" : words(param.name)) : mechanic?.about,
    picture: t?.picture ?? null,
    unit,
    options: q.candidates.map((c, i) => {
      const n = Number(c.value);
      return {
        choice: i + 1,
        value: c.value,
        number: Number.isFinite(n) ? n : undefined,
        label: unit && Number.isFinite(n) ? `${c.value} ${unitWords(unit)}` : c.value,
        from: fromWords(c.from),
      };
    }),
    path,
    text: q.text,
  };
}

/** The checks that failed in the latest validation, or none once one passed. */
export function failingChecks(s: LiveState): Found[] {
  const v = s.stages.validate;
  if (v && (v.status === "ok" || (v.status === "cached" && v.result === "ok"))) return [];
  const checks = Object.values(s.found).filter((f) => f.what === "check" && f.status === "fail");
  const last = Math.max(0, ...checks.map((c) => c.round));
  return checks.filter((c) => c.round === last);
}

const sv = (v: unknown) => (v === undefined || v === null ? "" : String(v));

/** A failing check as a sentence: what the original does there and what the model does. */
export function checkSentence(g: GameWords | undefined, c: Found): string {
  const d = c.data ?? {};
  const part = sv(d.part) || c.id.split(" ")[0];
  const quantity = sv(d.quantity) || c.id.split(" ").slice(1).join(" ");
  const expected = sv(d.expected);
  const actual = sv(d.actual);
  const kind = sv(d.kind);
  const both = expected !== "" || actual !== "";
  if (kind === "machine" && both) return `the model is ${words(actual)} while the original is ${words(expected)}`;
  const what = part.startsWith("counters.") ? pathPhrase(g, part) : `${pathPhrase(g, part)} ${words(quantity)}`.trim();
  if (both) return `${what} is ${actual || "nothing"} in the model and ${expected || "nothing"} in the original`;
  return `${what} doesn't match the original`;
}

/** Where a failing check failed: "at frame 272 of stomp then pit". */
export function checkWhere(c: Found): { frame?: number; recording?: string; text: string } {
  const d = c.data ?? {};
  const frame = typeof d.frame === "number" ? d.frame : undefined;
  const recording = typeof d.from === "string" ? d.from : undefined;
  const text = frame !== undefined ? `at frame ${frame}${recording ? ` of "${words(recording)}"` : ""}` : recording ? `in "${words(recording)}"` : "";
  return { frame, recording, text };
}

/** A count and a noun: "1 check", "2 checks". */
export function count(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n === 1 ? one : many}`;
}

/** A list joined as a person writes it: "a, b and c". */
export function list(items: string[]): string {
  if (items.length <= 1) return items[0] ?? "";
  return `${items.slice(0, -1).join(", ")} and ${items[items.length - 1]}`;
}

/** What a gap means, by where the original keeps it. */
export const GAP_PLACES: Record<string, { one: string; many: string }> = {
  ram: { one: "The original keeps a value in its memory that the model has no name for", many: "The original keeps {n} values in its memory that the model has no name for" },
  oam: { one: "The original draws a sprite the model has nothing for", many: "The original draws {n} kinds of sprite the model has nothing for" },
  background: { one: "A piece of the background changes in the original that the model never changes", many: "{n} kinds of background change happen in the original that the model never makes" },
  sound: { one: "The original plays a sound the model doesn't", many: "The original plays {n} sounds the model doesn't" },
  apu: { one: "The original plays a sound the model doesn't", many: "The original plays {n} sounds the model doesn't" },
  code: { one: "The original runs a routine the model doesn't account for", many: "The original runs {n} routines the model doesn't account for" },
};

export interface GapGroup {
  place: string;
  sentence: string;
  items: Found[];
  /** A frame to show it at: the first sample's, in the run's recording. */
  frame?: number;
  recording?: number;
}

/** The open gaps as a few sentences, one per place, with a moment to show each. */
export function gapGroups(gaps: Found[]): GapGroup[] {
  const by = new Map<string, Found[]>();
  for (const g of gaps) {
    const place = sv(g.data?.place) || g.text || g.id.split(":")[0];
    by.set(place, [...(by.get(place) ?? []), g]);
  }
  return [...by.entries()].map(([place, items]) => {
    const t = GAP_PLACES[place.split(".")[0]] ?? { one: `The original has something in its ${words(place)} the model doesn't`, many: `The original has {n} things in its ${words(place)} the model doesn't` };
    const n = items.length;
    let sentence = (n === 1 ? t.one : t.many).replace("{n}", String(n));
    const first = items[0].data ?? {};
    const size = Array.isArray(first.size) ? (first.size as number[]) : undefined;
    if (place === "oam" && n === 1 && size) sentence = `The original draws a ${size[0]} by ${size[1]} sprite the model has nothing for`;
    const samples = Array.isArray(first.samples) ? (first.samples as number[]) : [];
    return { place, sentence, items, frame: samples[0], recording: typeof first.run === "number" ? first.run : undefined };
  });
}

/** An event name as words: `player.died (mario)` -> "Mario died". */
export function eventWords(g: GameWords | undefined, e: string): string {
  const m = /^([a-z_]+)\.([a-z_]+)(?: \(([^)]+)\))?$/.exec(e.trim());
  if (!m) return words(e);
  const [, who, verb, kind] = m;
  const actor = kind ? thing(g, kind) : who === "player" ? player(g) : `the ${words(who)}`;
  return `${actor} ${words(verb).replace(/^hurt player$/, `hurt ${player(g)}`)}`;
}

/** A counter change as words: "score +20". */
function counterWords(name: string, d: number): string {
  return `${words(name)} ${d > 0 ? "+" : ""}${d}`;
}

interface Side {
  counters?: { name: string; before: number; after: number }[];
}

/** What a mark's explanation says, as one sentence: the original does this; the model does that. */
export function momentSentence(g: GameWords | undefined, x: Explanation & { original?: unknown }, note?: string): { original: string[]; model: string[]; sentence: string } {
  const o = (x.original ?? {}) as Side & { cells?: { before: string; after: string; count: number }[]; interactions?: { interaction?: { requests?: string[]; spawned?: string[] } }[] };
  const m = (x.model ?? {}) as Side & { tiles_changed?: number; spawned?: unknown[]; events?: [string, number][] };
  const delta = (s: Side | undefined) => new Map((s?.counters ?? []).map((c) => [c.name, c.after - c.before]));
  const od = delta(o);
  const md = delta(m);
  const orig: string[] = [];
  for (const [name, d] of od) if (d !== (md.get(name) ?? 0) && d !== 0) orig.push(counterWords(name, d));
  const cells = (o.cells ?? []).filter((c) => c.before !== c.after).reduce((n, c) => n + (c.count ?? 1), 0);
  if (cells && !m.tiles_changed) orig.push(cells === 1 ? "a piece of the background changes" : `the background changes in ${cells} places`);
  const sounds = new Set((o.interactions ?? []).flatMap((i) => i.interaction?.requests ?? []));
  if (sounds.size) orig.push(sounds.size === 1 ? "a sound plays" : `${sounds.size} sounds play`);
  const spawned = (o.interactions ?? []).reduce((n, i) => n + (i.interaction?.spawned?.length ?? 0), 0);
  if (spawned && !(m.spawned ?? []).length) orig.push(spawned === 1 ? "something new appears" : `${spawned} new things appear`);
  if (!orig.length && x.missing.length) orig.push(`${count(x.missing.length, "thing")} happen that the model misses`);
  const model: string[] = [];
  for (const [e, n] of m.events ?? []) model.push(n > 1 ? `${eventWords(g, e)} (${n} times)` : eventWords(g, e));
  for (const [name, d] of md) if (d !== (od.get(name) ?? 0) && d !== 0) model.push(counterWords(name, d));
  const lead = note?.trim() ? `${cap(note.trim())}` : `At this moment`;
  const said = orig.length ? `the original: ${list(orig)}` : "the original does nothing the model misses";
  const did = model.length ? `the model: ${list(model)}` : "the model does nothing";
  return { original: orig, model, sentence: `${lead}: ${said}; ${did}.` };
}

/** The status line over the two games: what's wrong, in game terms. */
export function statusLine(g: GameWords | undefined, live: LiveState, play?: PlayState): string {
  const parts: string[] = [];
  if (live.running) parts.push(`Capture is working on round ${live.round}`);
  if (play && play.marks.length) {
    const explained = play.marks.map((m) => ({ m, x: play.explanations[m.id] })).filter((r) => r.x);
    if (!explained.length) parts.push(play.explaining ? `Explaining your ${count(play.marks.length, "mark")}` : `You marked ${count(play.marks.length, "moment")}`);
    for (const { m, x } of explained.slice(0, 1)) parts.push(momentSentence(g, x!, m.note).sentence.replace(/\.$/, ""));
    if (explained.length > 1) parts.push(`${count(explained.length - 1, "more moment")} below`);
  }
  const checks = failingChecks(live);
  if (checks.length) {
    const by = new Map<string, Found[]>();
    for (const c of checks) {
      const w = checkWhere(c).text;
      by.set(w, [...(by.get(w) ?? []), c]);
    }
    const said = [...by.entries()].map(([w, cs]) => `${list(cs.map((c) => checkSentence(g, c)))}${w ? ` ${w}` : ""}`);
    parts.push(`${count(checks.length, "check")} ${checks.length === 1 ? "fails" : "fail"}: ${said.join("; ")}`);
  }
  const q = Object.values(live.questions).filter((x) => !x.answered && !live.pins.includes(x.id) && !(x.path && live.pins.includes(x.path)));
  if (q.length) parts.push(`${q.length === 1 ? "A question for you" : `${q.length} questions for you`}: ${plainQuestion(g, q[0]).ask}`);
  if (!parts.length && live.stages.validate) parts.push("The model matches the original on every check");
  if (!parts.length) parts.push(g?.vocab.title ? `${g.vocab.title}: the original and the model` : "The original and the model");
  return parts.map((p) => (/[.?!]$/.test(p) ? p : `${p}.`)).join(" ");
}

/** The numbers worth a glance, each with a line saying what it means. Keys are scorecard metric names (docs/scorecard.md). */
export const MEANINGS: Record<string, { label: string; means: string }> = {
  "fidelity.interp.passed": { label: "checks pass", means: "Behaviours the model gets the same as the original's recordings" },
  "fidelity.interp.failed": { label: "checks fail", means: "Behaviours where the model and the original's recordings differ" },
  "fidelity.interp.unexercised": { label: "not tried", means: "Behaviours no recording exercises yet" },
  "questions.open": { label: "questions for you", means: "Things capture isn't sure of; pick the answer that looks right" },
  "gaps.open": { label: "things the model misses", means: "Things the original does that the model has nothing for yet" },
  "facts.total": { label: "facts", means: "Numbers and rules in the model, each with where it came from" },
  "facts.derived": { label: "read from the code", means: "Facts capture read from the game's own code" },
  "facts.derived_differ": { label: "code disagrees", means: "Facts where the model and the game's code say different things" },
  "facts.compared": { label: "measured", means: "Facts checked against what the recordings show" },
  "facts.agree": { label: "measurements agree", means: "Measured facts that match the recordings" },
  "assets.count": { label: "graphics and data found", means: "Pieces of the game's graphics, levels and tables capture lifted from the ROM" },
  "fidelity.track.recall": { label: "objects followed", means: "Share of the original's moving objects capture can follow" },
  "mechanics.debt": { label: "hand-written parts", means: "Behaviours still hand-written for this game instead of general parts every game can use" },
  "speed.rom_to_preview_s": { label: "seconds to a playable model", means: "From the ROM to the first rebuilt game you can watch" },
};

/** The few numbers the summary shows, in this order. */
export const HEADLINE_PLAIN = ["fidelity.interp.passed", "fidelity.interp.failed", "questions.open", "gaps.open", "facts.total", "speed.rom_to_preview_s"];

/** The summary's numbers: those the run has, with meanings. */
export function headline(metrics: Record<string, Metric>): { key: string; value: number; label: string; means: string; changed: boolean }[] {
  return HEADLINE_PLAIN.filter((k) => k in metrics).map((k) => {
    const m = metrics[k];
    return { key: k, value: m.value, label: MEANINGS[k].label, means: MEANINGS[k].means, changed: m.prev !== null && m.prev !== m.value };
  });
}

/** The explainer while the original is being played live: there is one picture until the session is recorded. */
export const EXPLAINER_PLAYING: { area: string; line: string }[] = [
  { area: "The picture", line: "The original game, playing now with your keys. The game rebuilt from the model isn't shown while you play." },
  { area: "Mark", line: "Press M when the game does something the model should know about (\"hit a brick\") and say what happened." },
  { area: "Stop and record", line: "Ends the session and records it. The original and the rebuilt game then show side by side, with the marked moments explained under them." },
  { area: "For agents", line: "At the bottom: the raw data under all of this, for agents and power users." },
];

/** The first-run explainer: each area of the page in a line. */
export const EXPLAINER: { area: string; line: string }[] = [
  { area: "The screen", line: "The original game, played live with your keys or a gamepad. Tick \"rebuilt game\" to show the game rebuilt from the model on top of it, or the differences, or side by side." },
  { area: "Save and load", line: "F5 saves, F8 loads the save state chosen in the strip. With clips on, each save or load starts a short clip that ends at the next save or load, or after 8 seconds." },
  { area: "The filmstrip", line: "Every save state, the newest on the right. Click one to load it. Its small mark is its clip: click the mark to watch the clip on both games and keep, skip or star it." },
  { area: "The background line", line: "What the background work is doing with your clips: scoring them, fixing numbers, and a question for you when it has one." },
  { area: "Details", line: "At the bottom, closed: the recording measured against the rebuilt game (charts, the scenario ladder, moments) and the raw data, for agents and power users." },
];
