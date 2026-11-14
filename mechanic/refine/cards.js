// REFINE's live cards (game-printer's editor/src/ControlRoom.tsx; docs/show.md,
// "The live cards"): a card for each job the background work runs now,
// with bars that move toward the goal while you watch, then the cards of
// the jobs just done with their final score and the change since last
// time, and the stepper for how many jobs run at once. A refine round's
// bars move with its progress events on the run's stream (gp model
// refine --progress-json, through gp serve), not with the poll.
//
// The user asked for it on 2026-10-30: "a live render of stats of the ones
// in progress with bars showing moving toward the goal".

import { bar, clipPicture, css, doneLine, h, hub, jobBars, liveCards, openClip, opts } from "./fold.js";

function stepper(label, value, min, max, set, title) {
  const w = h("span", "rf-stepper");
  w.title = title;
  const minus = h("button", "", "-");
  const plus = h("button", "", "+");
  minus.disabled = value <= min;
  plus.disabled = value >= max;
  minus.addEventListener("click", () => set(value - 1));
  plus.addEventListener("click", () => set(value + 1));
  w.append(h("span", "rf-label", label), minus, h("b", "", value), plus);
  return w;
}

function card(x, c, pics) {
  const b = h("button", `rf-card ${c.state === "running" ? c.job.kind : "done"}`);
  b.dataset.testid = "live-card";
  b.dataset.state = c.state;
  b.dataset.clip = c.clip;
  b.title = "open this clip on the screen";
  b.addEventListener("click", () => openClip(x.run, c.clip));
  // The picture is made once a clip, so a card redrawn each second keeps it.
  let pic = pics.get(c.clip);
  if (!pic) { pic = clipPicture(x, c.clip); pics.set(c.clip, pic); }
  b.append(pic, h("span", "rf-name", c.name ?? c.clip));
  if (c.state === "running") {
    const j = c.job;
    const doing = h("span", "rf-doing");
    doing.append(h("span", "rf-busy"), document.createTextNode(j.kind === "score" ? "scoring" : `fixing numbers, round ${j.round ?? "?"}`));
    b.append(doing);
    for (const r of jobBars(j)) {
      const row = h("span", "rf-row");
      row.dataset.testid = `live-bar-${r.id}`;
      row.append(h("span", "rf-label", r.label));
      if (r.value !== undefined) row.append(bar(r.value, r.label));
      row.append(h("span", "rf-says", r.says));
      b.append(row);
    }
    if (j.trying) b.append(h("span", "rf-trying", `trying ${j.trying.words ?? j.trying.param} = ${j.trying.value}`));
    else if (j.target?.says) b.append(h("span", "rf-trying", j.target.says));
    return b;
  }
  const d = c.done;
  const l = doneLine(d);
  b.append(h("span", "rf-doing", d.kind === "refine" ? "a round of fixing numbers" : "scored"));
  const row = h("span", "rf-row");
  row.dataset.testid = "live-done";
  if (d.kind === "refine") {
    // A round's line is a sentence: under the bar, not beside it.
    row.append(h("span", "rf-label", "frames together"), bar((d.after ?? 0) / 1000, "frames together"));
    b.append(row, h("span", "rf-trying", l.says));
  } else {
    row.append(h("span", "rf-label", "frames together"), bar((d.after ?? 0) / 1000, "frames together"), h("span", `rf-says ${l.change ?? ""}`, l.says));
    b.append(row);
  }
  if (d.exact) b.classList.add("matches");
  return b;
}

/** Mounts the live cards on `el`; returns {stop()}. */
export function refineCards(el, a, b, c) {
  css();
  const x = hub(opts(a, b, c));
  el.classList.add("rf", "rf-cards");
  const pics = new Map();
  const draw = () => {
    const s = x.state;
    el.textContent = "";
    const head = h("div", "rf-head");
    head.append(h("h3", "", "In progress now"));
    if (s) {
      head.append(stepper("workers", s.workers ?? 1, 1, s.max_workers ?? 1, (n) => void x.post("/background/workers", { workers: n }), "How many clips the background work scores or fixes at the same time. More is faster and uses more of the machine."));
      head.append(stepper("agents", s.agents ?? 0, 0, s.max_agents ?? 8, (n) => void x.post("/background/agents", { agents: n }), "How many clip agents the stuck clips are handed to at once (0 hands none off)."));
    }
    el.append(head);
    if (!s) { el.append(h("p", "rf-muted", x.error ? `No background work here: ${x.error}` : "Checking the background work.")); return; }
    if (s.error) el.append(h("p", "rf-error", s.error));
    const cards = liveCards(s.jobs, s.done, s.workers ?? 1);
    if (!cards.length) {
      const p = h("p", "rf-muted", s.enabled ? s.says || "Nothing running now." : "The background work is paused: start it to score and fix clips.");
      p.dataset.testid = "live-none";
      el.append(p);
      return;
    }
    const row = h("div", "rf-live-row");
    row.dataset.testid = "live-cards";
    for (const c of cards) row.append(card(x, c, pics));
    el.append(row);
  };
  const off = x.subscribe(draw);
  draw();
  // The seconds on a scoring card count up between events.
  const t = setInterval(() => { if (x.state?.working) for (const j of x.state.jobs ?? []) j.secs = (j.secs ?? 0) + 1; if (x.state?.working) draw(); }, 1000);
  return { stop() { clearInterval(t); off(); x.stop(); el.textContent = ""; } };
}

export default refineCards;
