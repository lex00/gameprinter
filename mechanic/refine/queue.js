// REFINE's queue (game-printer's editor/src/Dashboard.tsx, with the clip
// card's agent lines from Agent.tsx; docs/show.md, "The main screen", and
// docs/agents.md): the clips in progress, the one worked on now first,
// then the ones put in the queue by hand, then those whose score just
// changed, then those that don't match yet. Each clip has its score in
// words, a small line of its score over time, "fix this clip" (the queue
// on, that clip first; on a clip no numbers fix, the clip agent takes
// it), "add to the queue", "let the agent fix it", "tell your agent" (a
// line about the clip for the agent: serve/tell.rs, echoed into the
// conversation, with the agent's answer under it), and the clip agent's
// line and attempts with keep. Above them, a clip can be added to the
// queue by name. AS PICTURES shows every clip of the tests as a picture
// instead (pictures.js).

import { drawPictures } from "./pictures.js";
import { addScores, cardLine, clipState, css, h, hub, inProgress, openClip, opts, readTrends, sparkline, tellAbout, tellLines } from "./fold.js";

const SHOWN = 40;

/** Mounts the queue on `el`; returns {stop()}. */
export function refineQueue(el, a, b, c) {
  css();
  const x = hub(opts(a, b, c));
  el.classList.add("rf", "rf-queue");
  let trends = readTrends();
  const changed = {};
  let note = "";
  let all = false;
  // TELL YOUR AGENT: the clip being told about, what is typed so far, and
  // why the last send failed.
  let telling = null;
  let draft = "";
  let tellError = "";
  // AS A LIST or AS PICTURES (pictures.js), kept for the next visit.
  let view = "list";
  try { if (localStorage.getItem("gp.refine.view") === "pictures") view = "pictures"; } catch { /* a list */ }
  const pics = new Map();
  const act = async (f) => {
    try {
      const r = await f();
      note = r?.agent?.why ?? r?.why ?? "";
      if (r?.agent?.started || r?.started) note = "The clip agent started on it.";
    } catch (e) {
      note = String(e.message ?? e);
    }
    draw();
  };
  const enc = encodeURIComponent;
  const draw = (_, why) => {
    if (why === "progress") return;
    // Not while a clip is being picked from the list.
    if (el.contains(document.activeElement) && ["SELECT", "INPUT"].includes(document.activeElement.tagName)) return;
    const s = x.state;
    if (why === "state" && s) {
      const r = addScores(trends, s.scores);
      trends = r.trends;
      for (const id of r.moved) changed[id] = Date.now();
    }
    el.textContent = "";
    const head = h("div", "rf-head");
    head.append(h("h3", "", "The queue"));
    const sw = h("span", "rf-switch");
    for (const [v, words, title] of [["list", "as a list", "The clips in progress, with what each needs"], ["pictures", "as pictures", "Every clip of the game's tests as a picture of its start"]]) {
      const b = h("button", `rf-btn${view === v ? " on" : ""}`, words);
      b.dataset.testid = `view-${v}`;
      b.title = title;
      b.setAttribute("aria-pressed", String(view === v));
      b.addEventListener("click", () => {
        view = v;
        try { localStorage.setItem("gp.refine.view", v); } catch { /* not kept */ }
        draw();
      });
      sw.append(b);
    }
    head.append(sw);
    el.append(head);
    if (!s) { el.append(h("p", "rf-muted", "Checking the background work.")); return; }
    if (view === "pictures") { drawPictures(el, x, pics); return; }
    const queued = s.queued ?? [];
    const rec = x.record ?? {};
    const agents = s.clip_agents ?? [];
    const working = new Set([...(s.jobs ?? []).map((j) => j.clip), ...agents.flatMap((g) => [g.clip, ...(g.clips ?? [])])]);
    // Add a clip to the queue by name.
    const add = h("form", "rf-add");
    const pick = document.createElement("select");
    pick.dataset.testid = "queue-pick";
    pick.append(new Option("add a clip to the queue...", ""));
    for (const id of s.active ?? []) if (!queued.includes(id)) pick.append(new Option(`${s.names?.[id] ?? id}: ${cardLine(s.scores?.[id], x.screen.fps)}`, id));
    const go = h("button", "rf-btn", "add to the queue");
    go.type = "submit";
    go.dataset.testid = "queue-add";
    add.append(pick, go);
    add.addEventListener("submit", (e) => { e.preventDefault(); if (pick.value) void act(() => x.post("/background/queue", { clip: pick.value })); });
    el.append(add);
    if (note) el.append(h("p", "rf-note", note));
    // The clip agents working now, each with its row.
    for (const g of agents) {
      const row = h("div", "rf-agent");
      row.dataset.testid = "agent-row";
      row.append(h("span", "rf-who", "Claude"), h("span", "rf-says", g.row ?? g.says ?? ""));
      const st = h("button", "rf-btn", "stop");
      st.addEventListener("click", () => void act(() => x.post("/agent/stop", { clip: g.clip })));
      row.append(st);
      el.append(row);
    }
    if (!agents.length && s.agents_idle) el.append(h("p", "rf-muted", s.agents_idle));
    const atts = rec.attempts ?? [];
    const ids = inProgress(s, working, changed);
    const list = h("ol", "rf-list");
    list.dataset.testid = "queue";
    for (const id of all ? ids : ids.slice(0, SHOWN)) {
      const sc = s.scores?.[id];
      const li = h("li", `rf-clip ${clipState(sc).replace(/[^a-z]/g, "-")}`);
      li.dataset.clip = id;
      const top = h("div", "rf-clip-top");
      const name = h("button", "rf-link rf-name", s.names?.[id] ?? id);
      name.title = `${id}: open this clip on the screen`;
      name.addEventListener("click", () => openClip(x.run, id));
      top.append(name);
      const q = queued.indexOf(id);
      if (working.has(id)) top.append(h("span", "rf-tag now", "now"));
      if (q >= 0) top.append(h("span", "rf-tag", `queued ${q + 1}`));
      if (s.stuck?.[id] && s.stuck[id] === s.model) top.append(h("span", "rf-tag stuck", "no numbers fix it"));
      const sp = sparkline(trends[id]);
      if (sp) top.append(sp);
      li.append(top, h("div", "rf-line-says", cardLine(sc, x.screen.fps)));
      // The clip agent and the attempts on it.
      const mine = atts.filter((t) => t.clip === id).slice(-3);
      for (const t of mine) {
        const ar = h("div", "rf-attempt");
        ar.append(h("span", "rf-who", t.by === "clip agent" ? "Claude (the clip agent)" : "Claude (the driver)"), h("span", "", t.line));
        if (t.approved) ar.append(h("span", "rf-tag", "kept; waiting to be merged"));
        else if (t.keepable) {
          const k = h("button", "rf-btn keep", "keep");
          k.title = "Better on this clip and no active clip worse: keep it, for the driver to merge into the game.";
          k.addEventListener("click", () => void act(() => x.post("/agent/approve", { attempt: t.n })));
          ar.append(k);
        }
        if (t.preview) {
          const pv = document.createElement("a");
          pv.href = `/api/capture/${enc(x.run)}/agent/attempts/${t.n}/preview/index.html`;
          pv.target = "_blank";
          pv.textContent = "trial preview";
          pv.className = "rf-link";
          ar.append(pv);
        }
        li.append(ar);
      }
      for (const t of tellLines(x.tells, id)) {
        const tl = h("div", `rf-told ${t.state}`, t.text);
        tl.dataset.testid = "told";
        li.append(tl);
      }
      if (telling === id) {
        li.append(tellForm(id));
        list.append(li);
        continue;
      }
      const btns = h("div", "rf-btns");
      if (clipState(sc) !== "matches" && clipState(sc) !== "fixed") {
        const fix = h("button", "rf-btn", "fix this clip");
        fix.dataset.testid = "fix-clip";
        fix.title = "The background work on, this clip first. On a clip no numbers fix, the clip agent takes it.";
        fix.addEventListener("click", () => void act(() => x.post("/background/start", { clip: id })));
        btns.append(fix);
      }
      const qb = h("button", "rf-btn", q >= 0 ? "take out of the queue" : "add to the queue");
      qb.addEventListener("click", () => void act(() => x.post("/background/queue", q >= 0 ? { clip: id, remove: true } : { clip: id })));
      btns.append(qb);
      if (s.stuck?.[id] && !working.has(id)) {
        const ag = h("button", "rf-btn", "let the agent fix it");
        ag.addEventListener("click", () => void act(() => x.post(`/clips/${enc(id)}/agent`, {})));
        btns.append(ag);
      }
      const tb = h("button", "rf-btn", "tell your agent");
      tb.dataset.testid = "tell-agent";
      tb.title = "Write what is wrong with this clip. Your agent gets it, and its answer shows here.";
      tb.addEventListener("click", () => { telling = id; draft = ""; tellError = ""; draw(); el.querySelector("[data-testid=tell-text]")?.focus(); });
      btns.append(tb);
      li.append(btns);
      list.append(li);
    }
    el.append(list);
    if (ids.length > SHOWN) {
      const more = h("button", "rf-link", all ? "fewer" : `all ${ids.length} clips`);
      more.addEventListener("click", () => { all = !all; draw(); });
      el.append(more);
    }
  };
  /** The one-line box that takes a button row's place. */
  function tellForm(id) {
    const f = h("form", "rf-tell");
    f.dataset.testid = "tell-form";
    const box = h("input");
    box.dataset.testid = "tell-text";
    box.placeholder = "What is wrong with this clip?";
    box.value = draft;
    box.setAttribute("aria-label", `What is wrong with ${id}`);
    const send = h("button", "rf-btn", "send");
    send.type = "submit";
    send.dataset.testid = "tell-send";
    send.disabled = !draft.trim();
    box.addEventListener("input", () => { draft = box.value; send.disabled = !draft.trim(); });
    box.addEventListener("keydown", (e) => { if (e.key === "Escape") { telling = null; box.blur(); draw(); } });
    const no = h("button", "rf-link", "cancel");
    no.type = "button";
    no.addEventListener("click", () => { telling = null; draw(); });
    f.append(box, send, no);
    if (tellError) f.append(h("span", "rf-error", tellError));
    f.addEventListener("submit", async (e) => {
      e.preventDefault();
      if (!draft.trim()) return;
      send.disabled = true;
      try {
        await tellAbout(x.api, x.run, id, draft.trim());
        telling = null;
        draft = "";
        tellError = "";
      } catch (err) {
        tellError = `Couldn't send: ${String(err.message ?? err).replace(/\.$/, "")}.`;
      }
      box.blur();
      await x.refresh();
      draw();
    });
    return f;
  }
  const off = x.subscribe(draw);
  draw();
  return { stop() { off(); x.stop(); el.textContent = ""; } };
}

export default refineQueue;
