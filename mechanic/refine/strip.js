// CLIPS's slim strip under the game (game-printer's editor/src/Background.tsx
// status line, slimmed): the background work in one line, and a small bar
// for each job running now, moving with the run's events, so the work
// shows while the person records and reviews clips. REFINE has all of it.

import { bar, css, h, hub, jobBars, opts, statusSentence } from "./fold.js";

/** Mounts the strip on `el`; returns {stop()}. */
export function refineStrip(el, a, b, c) {
  css();
  const x = hub(opts(a, b, c));
  el.classList.add("rf", "rf-strip");
  const draw = () => {
    const s = x.state;
    el.textContent = "";
    const jobs = s?.jobs ?? [];
    if (!jobs.length) {
      el.append(h("span", "rf-says", statusSentence(s, x.history, Date.now())));
      return;
    }
    for (const j of jobs) {
      const one = h("span", "rf-strip-job");
      one.dataset.testid = "strip-job";
      one.append(h("span", "rf-name", j.kind === "score" ? `scoring ${j.name}` : `round ${j.round ?? "?"} on ${j.name}`));
      // The bar that moves most: the impact pass's or the fine pass's
      // trials on a round, the pool on a score.
      const bs = jobBars(j).filter((r) => r.value !== undefined);
      const r = j.kind === "refine" ? (j.pass === "fine" ? bs.find((r) => r.id === "fine") : bs.find((r) => r.id === "impact")) ?? bs[0] : bs[0];
      if (r) one.append(bar(r.value, r.label), h("span", "rf-says", r.says));
      el.append(one);
    }
    const ag = s?.clip_agents ?? [];
    for (const g of ag) el.append(h("span", "rf-strip-agent", g.row ?? g.says ?? ""));
  };
  const off = x.subscribe(draw);
  draw();
  return { stop() { off(); x.stop(); el.textContent = ""; } };
}

export default refineStrip;
