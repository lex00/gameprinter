// REFINE's queue (game-printer's editor/src/Dashboard.tsx, with the clip
// card's agent lines from Agent.tsx; docs/show.md, "The main screen", and
// docs/agents.md): the clips in progress, the one worked on now first,
// then the ones put in the queue by hand, then those whose score just
// changed, then those that don't match yet. Each clip has its score in
// words, a small line of its score over time, "fix this clip" (the queue
// on, that clip first; on a clip no numbers fix, the clip agent takes
// it), "add to the queue", "let the agent fix it", and the clip agent's
// line and attempts with keep. Above them, a clip can be added to the
// queue by name.

import { addScores, cardLine, clipState, css, h, hub, inProgress, openClip, opts, readTrends, sparkline } from "./fold.js";

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
    if (el.contains(document.activeElement) && document.activeElement.tagName === "SELECT") return;
    const s = x.state;
    if (why === "state" && s) {
      const r = addScores(trends, s.scores);
      trends = r.trends;
      for (const id of r.moved) changed[id] = Date.now();
    }
    el.textContent = "";
    const head = h("div", "rf-head");
    head.append(h("h3", "", "The queue"));
    el.append(head);
    if (!s) { el.append(h("p", "rf-muted", "Checking the background work.")); return; }
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
  const off = x.subscribe(draw);
  draw();
  return { stop() { off(); x.stop(); el.textContent = ""; } };
}

export default refineQueue;
