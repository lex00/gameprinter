// The REFINE parts' shared side: the pure functions game-printer's editor
// had in controlfold.ts, backfold.ts and Dashboard.tsx (the live cards'
// bars, the status sentence, the rounds in words, the clips in progress),
// and one hub per run that holds the background work's state, the work
// record and the rounds, kept current from the run's event stream
// (gp serve's /api/capture/RUN/stream) with a slow poll under it.
//
// No framework: each part (cards.js, queue.js, line.js, strip.js) takes an
// element and {api, run, stream?} and returns {stop()}.

const enc = encodeURIComponent;

// ------------------------------------------------------------ the bars

const frac = (k, n) => (n > 0 ? Math.min(1, Math.max(0, k / n)) : 0);
const pct = (v) => `${Math.round(v * 100)}%`;

/**
 * A job's bars (controlfold.ts, jobBars). A score: the clips scored so far
 * toward the pool. A refine: the impact pass's trials, the fine pass's,
 * and the best split count so far against the start's, toward none.
 * Each is {id, label, value?, says}; no value is a line without a bar.
 */
export function jobBars(j) {
  if (j.kind === "score") {
    const bars = [];
    if (j.n) bars.push({ id: "pool", label: "clips scored", value: frac(j.k ?? 0, j.n), says: `clip ${j.k ?? 0} of ${j.n}` });
    bars.push({ id: "time", label: "this clip", says: `${j.secs ?? 0} s` });
    return bars;
  }
  if (j.kind === "refine") {
    const base = j.base ?? null;
    const best = j.best ?? base;
    const bars = [
      { id: "impact", label: "impact pass", value: frac(j.impact_k ?? 0, j.impact_n ?? 0), says: j.impact_n ? `trial ${Math.min(j.impact_k ?? 0, j.impact_n)} of ${j.impact_n}` : "finding where they part" },
      j.pass === "fine"
        ? { id: "fine", label: "fine pass", value: j.fine_knobs ? frac(Math.max(0, (j.fine_on ?? 0) - 1) + 0.5, j.fine_knobs) : 1, says: j.fine_knobs ? `number ${j.fine_on ?? 0} of ${j.fine_knobs}, ${j.fine_k ?? 0} trials` : "no number moves it toward the original" }
        : { id: "fine", label: "fine pass", says: "after the impact pass" },
    ];
    if (base !== null && best !== null)
      bars.push({ id: "best", label: "best so far", value: base > 0 ? frac(base - best, base) : 1, says: `${best} wrong, from ${base} at the start${j.kept ? `; ${j.kept} kept` : ""}` });
    return bars;
  }
  return [];
}

/**
 * The top row (controlfold.ts, liveCards): every job running, by worker,
 * then the jobs just done (newest first, one card a clip, none for a clip
 * running again) to fill as many places as there are workers.
 */
export function liveCards(jobs, done, workers) {
  const running = [...(jobs ?? [])].sort((a, b) => (a.slot ?? 0) - (b.slot ?? 0));
  const out = running.map((j, i) => ({ key: `run-${j.slot ?? i}-${j.kind}-${j.clip}`, clip: j.clip, name: j.name, state: "running", job: j }));
  const busy = new Set(running.map((j) => j.clip));
  const seen = new Set();
  for (const d of [...(done ?? [])].reverse()) {
    if (out.length >= Math.max(workers, running.length)) break;
    if (busy.has(d.clip) || seen.has(d.clip)) continue;
    seen.add(d.clip);
    out.push({ key: `done-${d.seq}`, clip: d.clip, name: d.name, state: "done", done: d });
  }
  return out;
}

/** A done card's line: its final score and the change since last time. */
export function doneLine(d) {
  if (d.error) return { says: `can't be scored: ${d.error}`.slice(0, 160) };
  if (d.kind === "refine") return { says: d.says ?? "round done" };
  const after = d.after ?? 0;
  const now = d.exact ? "matches exactly" : `${Math.round(after / 10)}% together`;
  if (d.before == null) return { says: `${now} (first score)` };
  const delta = Math.round(after / 10) - Math.round(d.before / 10);
  if (delta === 0) return { says: `${now}, as before`, change: "same" };
  return { says: `${now}, was ${Math.round(d.before / 10)}% (${delta > 0 ? "+" : ""}${delta})`, change: delta > 0 ? "up" : "down" };
}

// ------------------------------------------------------------ the line

export function openQuestions(s) {
  return (s?.questions ?? []).filter((q) => !q.answer);
}

/** Rounds since local midnight that fixed a clip. */
export function fixedToday(history, now) {
  const d = new Date(now);
  const midnight = new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  return history.filter((r) => r.when_ms >= midnight && r.round !== undefined).reduce((n, r) => n + (r.fixed ?? 0), 0);
}

/** A round in words: "Round 12: jumpman.gravity 0.1875 -> 0.15625, 6 clips fixed, none worse." */
export function roundWords(r) {
  if (r.round === undefined) return r.says;
  const clips = (n) => (n === 1 ? "1 clip" : `${n} clips`);
  if (!r.changes?.length) return r.stuck ? `Round ${r.round}: no numbers make ${r.clip_name ?? "the clip"} match; a question for the clip agent.` : r.says;
  const worse = r.worse?.length ? `${clips(r.worse.length)} worse` : "none worse";
  return `Round ${r.round}: ${r.changes.map((c) => `${c.param} ${c.old} -> ${c.new}`).join(", ")}, ${clips(r.fixed ?? 0)} fixed, ${worse}.`;
}

/**
 * The status line (backfold.ts, statusSentence): "Background: scoring
 * clip 14 of 52; 3 fixed today; 1 question". Paused or off says so with
 * the next thing to do; why no clip agent works comes last.
 */
export function statusSentence(s, history, now) {
  if (!s) return "Background: checking.";
  const parts = [];
  const jobs = s.jobs ?? [];
  const j = s.working ? jobs[0] : undefined;
  if (s.error && !j) parts.push(`stopped on an error: ${s.error}`);
  else if (!s.enabled && !j) parts.push(history.length ? "paused" : "off; turn it on to score your clips and fix numbers while you play");
  else if (jobs.length > 1) parts.push(`${jobs.length} jobs at once: ${jobs.map((x) => (x.kind === "score" ? `scoring ${x.name}` : `fixing ${x.name}`)).join(", ")}`);
  else if (j?.kind === "score") parts.push(`scoring clip ${j.k} of ${j.n}`);
  else if (j?.kind === "refine") parts.push(`round ${j.round}: fixing numbers on ${j.name}${j.trying ? ` (trying ${j.trying.param} = ${j.trying.value})` : ""}`);
  else {
    const scored = Object.values(s.scores ?? {});
    const differ = scored.filter((x) => !x.exact).length;
    parts.push(scored.length ? (differ ? `waiting; ${differ} of ${scored.length} clips differ` : `all ${scored.length} clips match`) : "waiting for clips");
  }
  const fixed = fixedToday(history, now);
  if (fixed) parts.push(`${fixed} fixed today`);
  const q = openQuestions(s).length;
  if (q) parts.push(q === 1 ? "1 question" : `${q} questions`);
  if (s.clip_agents?.length) parts.push(s.clip_agents.length === 1 ? "1 clip agent working" : `${s.clip_agents.length} clip agents working`);
  else if (s.agents_idle) parts.push(String(s.agents_idle).replace(/\.$/, ""));
  return `Background: ${parts.join("; ")}`;
}

// ------------------------------------------------------------ the number

/** What the fidelity number means, for its hover. */
export const FIDELITY_MEANS = "The average, over the clips scored, of how much of each clip plays the same as the original. Pictures and sound are not measured yet.";

/**
 * The fidelity number, as the agent's `next` has it (crates/gp-cli/src/
 * next.rs, Scores.fidelity_permille), so the status line, this page and
 * the agent say the same: over the active clips scored on the model as it
 * stands, the mean of how much of each plays the same as the original, an
 * exact clip counting 1000, rounded down. Where the background work has no
 * score on this model, `next` reads the gate's scores, and so does this
 * (`nx`, the status line's last answer, which next.js passes on as
 * "gp-next"); then the background's older scores, marked so.
 * {permille, scored, exact, active, older}, or null with no score at all.
 */
export function fidelity(s, nx) {
  const active = s?.active ?? Object.keys(s?.scores ?? {});
  const all = active.map((id) => s?.scores?.[id]).filter((x) => x && !x.error);
  const now = all.filter((x) => !s.model || x.model === s.model);
  const pm = (x) => (x.exact ? 1000 : Math.min(1000, x.permille ?? 0));
  const of = (use, older) => ({
    permille: Math.floor(use.reduce((a, x) => a + pm(x), 0) / use.length),
    scored: use.length,
    exact: use.filter((x) => x.exact).length,
    active: active.length,
    older,
  });
  if (now.length) return of(now, false);
  const g = nx?.facts?.scores;
  if (g?.clips && typeof g.fidelity_permille === "number") {
    return { permille: g.fidelity_permille, scored: g.clips, exact: g.exact ?? 0, active: g.clips + (g.to_score ?? 0), older: !!g.stale };
  }
  return all.length ? of(all, true) : null;
}

/** REFINE's first sentence: "On average, 78% of a clip plays the same on
 * the model as on the original. 35 of 52 clips play the same all the way." */
export function fidelitySentence(s, nx) {
  if (!s) return "";
  const f = fidelity(s, nx);
  if (!f) return "Not measured yet: no clip is scored.";
  let t = `On average, ${Math.round(f.permille / 10)}% of a clip plays the same on the model as on the original${f.older ? " (scored on an older model)" : ""}. ${f.exact} of ${f.scored} clips play the same all the way.`;
  if (!f.older && f.scored < f.active) t += ` Only ${f.scored} of ${f.active} clips are scored so far.`;
  return t;
}

/** The same, short, for CLIPS's strip: "Clips: 78% the same as the
 * original, 35 of 52 all the way." */
export function fidelityShort(s, nx) {
  if (!s) return "";
  const f = fidelity(s, nx);
  if (!f) return "Clips: not measured yet.";
  return `Clips: ${Math.round(f.permille / 10)}% the same as the original, ${f.exact} of ${f.scored} all the way${f.older ? ", on an older model" : ""}.`;
}

// ------------------------------------------------------------ the buttons

/**
 * A work button's record (gp serve's /api/capture/RUN/do/WHICH,
 * serve/jobs.rs): the page names one of a fixed few actions, the server
 * runs its gp command through the agent's gp_run.
 */
export async function jobRecord(api, run, which) {
  return api(`/api/capture/${enc(run)}/do/${enc(which)}`);
}

/** Starts a work button's action; its record. */
export async function startJob(api, run, which) {
  return api(`/api/capture/${enc(run)}/do/${enc(which)}`, {});
}

/** "14:02", the person's own clock. */
export function clock(ms) {
  const d = new Date(ms);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

/** The first line of some words, without a trailing stop. */
export function firstLine(t) {
  return String(t ?? "").split("\n")[0].trim().replace(/[.:;]+$/, "");
}

/** "a, b and 5 more", as parts: the names are clips to open. */
function named(ids) {
  const shown = ids.length > 3 ? ids.slice(0, 2) : ids;
  const out = [];
  shown.forEach((id, i) => {
    if (i > 0) out.push(i === shown.length - 1 && ids.length <= 3 ? " and " : ", ");
    out.push({ clip: id });
  });
  if (ids.length > 3) out.push(` and ${ids.length - 2} more`);
  return out;
}

/** What CHECK THE TESTS says (`gp corpus verify`'s JSON), as parts: text,
 * and {clip} for a clip's name the person can open. On today's plan-style
 * tests a clip's curves are derived, so the check can only say each clip
 * can be made again; it says that and no more. */
export function verifyWords(rec) {
  const st = rec?.state ?? "never";
  if (st === "never") return ["Not checked yet."];
  if (st === "running") return ["Checking the tests: making each clip again. This takes a few minutes. You can keep working."];
  if (st === "waits") return ["The check waits for your yes, on the card at the top."];
  if (st === "failed") return [`The check couldn't run: ${firstLine(rec.error) || "no reason given"}.`];
  const v = rec.json ?? {};
  const rows = v.clips ?? [];
  const at = `Checked at ${clock(rec.ended_ms ?? Date.now())}: `;
  const n = rows.length;
  const of = (k) => (k === 1 ? "1 clip" : `${k} clips`);
  const ids = (state) => rows.filter((r) => r.state === state).map((r) => r.id);
  const differ = ids("differ");
  const failed = ids("failed");
  const measured = ids("measured");
  const out = [];
  if (!differ.length && !failed.length && !measured.length) {
    const plan = n > 0 && rows.every((r) => r.says === "made");
    if (!n) return [`${at}the game's tests have no clip yet.`];
    const all = plan ? `all ${n} clips can be made again from your copy of the game` : `all ${n} clips come out the same`;
    // A problem with no clip to name: the tests themselves changed since
    // their version was recorded (gp corpus verify's content hash).
    if (!v.ok && v.problems?.length) return [`${at}${all}, but the game's tests changed since their last version was recorded. Your agent can look at why.`];
    return [`${at}${all}.`];
  }
  out.push(at);
  if (differ.length) out.push(`${differ.length} of ${n} clips come out different: `, ...named(differ), ". ");
  if (failed.length) out.push(`${of(failed.length)} couldn't be made again: `, ...named(failed), ". ");
  if (measured.length) out.push(`${of(measured.length)} had nothing saved to compare with, so ${measured.length === 1 ? "it was" : "they were"} only measured: `, ...named(measured), ". ");
  out.push("Your agent can look at why.");
  return out;
}

// ------------------------------------------------------------ telling

/** What the person told the agent about a clip (serve/tell.rs), newest
 * last, in words: [{text, state}], state "answered", "sent" or "waiting". */
export function tellLines(tells, clip) {
  const q = (t) => `"${String(t).trim()}"`;
  return (tells ?? []).filter((t) => t.clip === clip).slice(-2).flatMap((t) => {
    const reply = t.replies?.[t.replies.length - 1];
    if (reply) return [{ text: `You told your agent: ${q(t.text)}`, state: "told" }, { text: `Your agent answered: ${q(reply.text)}`, state: "answered" }];
    return [{ text: `You told your agent: ${q(t.text)}. ${t.agent ? "Sent to the agent working on this clip." : "Waiting for your agent."}`, state: t.agent ? "sent" : "waiting" }];
  });
}

/** Tells the agent something about a clip: the work record and, echoed,
 * the conversation. */
export async function tellAbout(api, run, clip, text) {
  return api(`/api/capture/${enc(run)}/clips/${enc(clip)}/tell`, { text, echo: true });
}

// ------------------------------------------------------------ the queue

/** A clip's state from its score: waiting, can't start, matches, fixed, not matching. */
export function clipState(score) {
  if (!score) return "waiting";
  if (score.error) return "can't start";
  if (score.exact) return score.fixed ? "fixed" : "matches";
  return "not matching";
}

/** A card's main line: "62% together; entities.barrel.x wrong from frame 9", or "matches exactly". */
export function cardLine(s, fps = 60) {
  if (!s) return "waiting to be scored";
  if (s.error) return `can't be scored: ${s.error}`.slice(0, 140);
  if (s.exact) return s.fixed ? "matches exactly (fixed)" : "matches exactly";
  const first = s.tracks?.[0];
  const at = (f) => `frame ${f} (${(f / fps).toFixed(1)} s)`;
  const where = first ? `; ${first.path ?? first.track} wrong from ${at(first.frame)}` : s.drift != null ? `; parts at ${at(s.drift)}` : "";
  return `${Math.round((s.permille ?? 0) / 10)}% together${where}`;
}

/**
 * The clips in progress, in order (Dashboard.tsx, inProgress): the ones
 * worked on now, then those in the queue by hand in its order, then those
 * whose score changed most recently on this page, then those that don't
 * match yet by score (furthest first), then those that match.
 */
export function inProgress(state, working, changed) {
  const ids = state?.active ?? Object.keys(state?.scores ?? {});
  const queued = state?.queued ?? [];
  const rank = (id) => {
    const s = state?.scores?.[id];
    if (working.has(id)) return [0, 0];
    const q = queued.indexOf(id);
    if (q >= 0) return [1, q];
    if (changed[id] !== undefined) return [2, -changed[id]];
    const st = clipState(s);
    if (st === "not matching") return [3, s?.permille ?? 0];
    if (st === "matches" || st === "fixed") return [5, 0];
    return [4, 0];
  };
  return [...ids].sort((a, b) => {
    const [ra, sa] = rank(a);
    const [rb, sb] = rank(b);
    return ra - rb || sa - sb || a.localeCompare(b);
  });
}

const TRENDS_KEY = "gp.refine.trends";
const KEEP = 24;

export function readTrends() {
  try {
    return JSON.parse(localStorage.getItem(TRENDS_KEY) ?? "{}");
  } catch {
    return {};
  }
}

/** The trends with each clip's current score added when it changed; and which changed. */
export function addScores(t, scores) {
  const out = { ...t };
  const moved = [];
  for (const [id, s] of Object.entries(scores ?? {})) {
    if (!s || s.error) continue;
    const v = s.exact ? 1000 : s.permille ?? 0;
    const was = t[id] ?? [];
    if (was[was.length - 1] === v) continue;
    if (was.length) moved.push(id);
    out[id] = [...was, v].slice(-KEEP);
  }
  try {
    localStorage.setItem(TRENDS_KEY, JSON.stringify(out));
  } catch {
    // Not kept; the trend starts again next time.
  }
  return { trends: out, moved };
}

/** A small line of a clip's score over time, as SVG. */
export function sparkline(values) {
  if (!values || values.length < 2) return null;
  const w = 80;
  const h = 18;
  const lo = Math.min(...values, 0);
  const pts = values.map((v, i) => `${((i / (values.length - 1)) * w).toFixed(1)},${(h - ((v - lo) / Math.max(1, 1000 - lo)) * h).toFixed(1)}`).join(" ");
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  svg.setAttribute("class", "rf-spark");
  svg.setAttribute("viewBox", `0 0 ${w} ${h}`);
  svg.setAttribute("width", w);
  svg.setAttribute("height", h);
  svg.setAttribute("role", "img");
  svg.setAttribute("aria-label", "its score over time");
  const pl = document.createElementNS(ns, "polyline");
  pl.setAttribute("points", pts);
  svg.append(pl);
  return svg;
}

// ------------------------------------------------------------ the page

/** A tag with a class and text. */
export function h(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined && text !== null) e.textContent = String(text);
  return e;
}

/** A bar 0 to 1, as a progressbar. */
export function bar(value, label) {
  const b = h("span", "rf-bar");
  b.setAttribute("role", "progressbar");
  b.setAttribute("aria-valuemin", "0");
  b.setAttribute("aria-valuemax", "100");
  b.setAttribute("aria-valuenow", String(Math.round(value * 100)));
  if (label) b.setAttribute("aria-label", label);
  const fill = h("span");
  fill.style.width = `${(value * 100).toFixed(1)}%`;
  b.append(fill);
  return b;
}

/** refine.css, once, next to this file. */
export function css() {
  const href = new URL("./refine.css", import.meta.url).href;
  if ([...document.querySelectorAll("link[rel=stylesheet]")].some((l) => l.href === href)) return;
  const l = document.createElement("link");
  l.rel = "stylesheet";
  l.href = href;
  document.head.append(l);
}

/** The page's api, or a plain fetch when none is given. */
function apiOf(api) {
  if (api) return api;
  return async (path, body) => {
    const r = await fetch(path, body === undefined ? {} : { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
    if (!r.ok) {
      const t = await r.text();
      let why = `${path}: ${r.status} ${t}`;
      try { why = JSON.parse(t).error ?? why; } catch { /* the status says it */ }
      throw new Error(why);
    }
    return r.json();
  };
}

/** The mount's options, given as {api, run, stream} or as (api, run, stream). */
export function opts(a, b, c) {
  if (a && typeof a === "object" && !(typeof a === "function")) return { api: apiOf(a.api), run: a.run, stream: a.stream };
  return { api: apiOf(a), run: b, stream: c };
}

/** A clip's picture: the original at its start. */
export function pictureOf(run, clip) {
  return `/api/capture/${enc(run)}/generated/starts/${enc(clip)}/picture.png`;
}

/** A clip's picture as an image, 128 across, as game-printer's NES cards
 * were, at the machine's own shape (Tapper's 512 by 480 as much as the
 * NES's 256 by 240); loaded when it scrolls into view. */
export function clipPicture(x, clip) {
  const { width, height } = x.screen;
  const img = document.createElement("img");
  img.className = "rf-pic";
  img.alt = "";
  img.width = 128;
  img.height = Math.round((128 * height) / width);
  img.style.aspectRatio = `${width} / ${height}`;
  img.loading = "lazy";
  img.src = pictureOf(x.run, clip);
  // No picture (the start can't be replayed): a black frame of the same size.
  img.addEventListener("error", () => { img.src = "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7"; img.classList.add("rf-nopic"); });
  return img;
}

const hubs = new Map();

/**
 * The run's hub: {state, record, tells, history, screen, next, api, run, subscribe(fn),
 * refresh(), post(path, body)}. One per run however many parts mount;
 * it lets go of the stream when the last part stops.
 */
export function hub(o) {
  const key = o.run;
  let x = hubs.get(key);
  if (x) { x.users += 1; return x; }
  const base = `/api/capture/${enc(o.run)}`;
  x = {
    users: 1,
    run: o.run,
    api: o.api,
    state: null,
    record: null,
    tells: [],
    history: [],
    screen: { width: 256, height: 240, fps: 60, buttons: [] },
    subs: new Set(),
    seq: 0,
    last: null,
  };
  const tell = (why) => { for (const f of x.subs) { try { f(x, why); } catch (e) { console.error(e); } } };
  let pending = null;
  async function refresh() {
    try {
      const [s, rec, tells] = await Promise.all([o.api(`${base}/background`), o.api(`${base}/agent`).catch(() => null), o.api(`${base}/tells`).catch(() => null)]);
      x.state = s;
      if (rec) x.record = rec;
      if (tells) x.tells = tells.tells ?? [];
      if ((s.rounds ?? 0) !== x.history.length) x.history = (await o.api(`${base}/background/history`).catch(() => ({ rounds: [] }))).rounds ?? [];
      tell("state");
    } catch (e) {
      x.error = String(e.message ?? e);
      tell("error");
    }
  }
  x.refresh = refresh;
  x.soon = () => { clearTimeout(pending); pending = setTimeout(() => void refresh(), 250); };
  x.post = async (path, body) => {
    const r = await o.api(`${base}${path}`, body ?? {});
    void refresh();
    return r;
  };
  x.subscribe = (f) => { x.subs.add(f); if (x.state) f(x, "state"); return () => x.subs.delete(f); };
  // The status line's answer (next.js), for the fidelity number's scores
  // when the background work has none: heard when its scores change.
  x.next = typeof window !== "undefined" ? window.gpNext ?? null : null;
  const onNext = (e) => {
    const was = JSON.stringify(x.next?.facts?.scores ?? null);
    x.next = e.detail;
    if (JSON.stringify(x.next?.facts?.scores ?? null) !== was) tell("next");
  };
  if (typeof window !== "undefined") window.addEventListener("gp-next", onNext);
  // An event: a refine round's progress moves its card's bars at once;
  // anything else asks for the state again.
  const onEvent = (e) => {
    x.seq = Math.max(x.seq, e.seq ?? 0);
    x.last = e;
    if (e.kind === "refine.progress" && x.state?.jobs) {
      const j = x.state.jobs.find((j) => j.clip === e.data?.clip && j.kind === "refine");
      if (j) {
        if (e.data.says) j.target = { says: e.data.says };
        // The queue's round number stays; the loop's own (its window's
        // round within this clip) isn't the queue's.
        const skip = new Set(["clip", "says", "name", "round", "slot", "window", "fine_params"]);
        for (const [k, v] of Object.entries(e.data)) if (!skip.has(k)) j[k] = v;
        tell("progress");
        return;
      }
    }
    tell("event");
    x.soon();
  };
  const listen = (es) => es.addEventListener("message", (m) => { try { onEvent(JSON.parse(m.data)); } catch { /* not ours */ } });
  if (o.stream) { listen(o.stream); x.ownStream = null; }
  else if (typeof EventSource !== "undefined") { x.ownStream = new EventSource(`${base}/stream`); listen(x.ownStream); }
  // The poll under the stream: every second while a job runs, every three
  // seconds otherwise.
  const tick = () => {
    x.timer = setTimeout(async () => { await refresh(); tick(); }, x.state?.working ? 1000 : 3000);
  };
  void o.api(`${base}/game`).then((g) => {
    const sc = g.screen ?? {};
    x.screen = { width: sc.width ?? 256, height: sc.height ?? 240, fps: sc.fps ? sc.fps[0] / sc.fps[1] : 60, buttons: sc.buttons ?? [], platform: g.platform ?? sc.platform ?? "nes" };
    tell("screen");
  }).catch(() => undefined);
  void refresh().then(tick);
  x.stop = () => {
    x.users -= 1;
    if (x.users > 0) return;
    clearTimeout(x.timer);
    clearTimeout(pending);
    if (typeof window !== "undefined") window.removeEventListener("gp-next", onNext);
    x.ownStream?.close();
    hubs.delete(key);
  };
  hubs.set(key, x);
  return x;
}

/** "open this clip": the page listens for it (gp-open-clip on window). */
export function openClip(run, clip) {
  window.dispatchEvent(new CustomEvent("gp-open-clip", { detail: { run, clip } }));
}

/** A clip's name from the state or the record, else its ID. */
export function nameOf(x, id) {
  const j = (x.state?.jobs ?? []).find((j) => j.clip === id) ?? (x.state?.done ?? []).find((d) => d.clip === id);
  return j?.name ?? id;
}
