// REFINE's background line (game-printer's editor/src/Background.tsx and
// backfold.ts; docs/show.md, "The background work"): the fidelity number
// in a sentence (fold.js, fidelitySentence), CHECK THE TESTS (gp corpus
// verify, through serve/jobs.rs) with what it found, one sentence for the
// background queue, start and pause, the questions the loop asked with
// their choices, and its rounds in words when asked for.

import { css, FIDELITY_MEANS, fidelitySentence, h, hub, jobRecord, openClip, openQuestions, opts, roundWords, startJob, statusSentence, verifyWords } from "./fold.js";

const CHECK_MEANS = "Makes every clip in the game's tests again from your copy of the game and checks each still comes out as saved. This takes a few minutes.";

/** Mounts the line on `el`; returns {stop()}. */
export function refineLine(el, a, b, c) {
  css();
  const x = hub(opts(a, b, c));
  el.classList.add("rf", "rf-line");
  let showHistory = false;
  let showQuestions = false;
  let busy = false;
  // CHECK THE TESTS: its record from gp serve (serve/jobs.rs), read with
  // the state, every second while it runs.
  let check = null;
  let reading = false;
  const readCheck = async () => {
    if (reading) return;
    reading = true;
    try {
      const r = await jobRecord(x.api, x.run, "verify");
      const was = JSON.stringify(check);
      check = r;
      if (JSON.stringify(r) !== was) draw();
    } catch { /* a server with no work buttons: none shows */ } finally { reading = false; }
  };
  const checkTimer = setInterval(() => { if (check?.state === "running") void readCheck(); }, 1000);
  const act = async (f) => {
    if (busy) return;
    busy = true;
    try { await f(); } catch (e) { x.error = String(e.message ?? e); } finally { busy = false; draw(); }
  };
  const draw = (_, why) => {
    if (why === "progress") return;
    const s = x.state;
    el.textContent = "";
    // The fidelity number first: how close the model is, in one sentence.
    if (s) {
      const fid = h("p", "rf-fidelity", fidelitySentence(s, x.next));
      fid.dataset.testid = "fidelity";
      fid.title = FIDELITY_MEANS;
      el.append(fid);
    }
    const top = h("div", "rf-line-top");
    const says = h("span", "rf-says", statusSentence(s, x.history, Date.now()));
    says.dataset.testid = "bg-says";
    top.append(says);
    if (s) {
      const t = h("button", "rf-btn", s.enabled ? "pause" : x.history.length ? "resume" : "start the work");
      t.dataset.testid = "bg-toggle";
      t.addEventListener("click", () => act(() => x.post(s.enabled ? "/background/stop" : "/background/start", {})));
      top.append(t);
      if (check) {
        const v = h("button", "rf-btn", "check the tests");
        v.dataset.testid = "check-tests";
        v.title = CHECK_MEANS;
        v.disabled = check.state === "running";
        v.addEventListener("click", () => act(async () => { check = await startJob(x.api, x.run, "verify"); }));
        top.append(v);
      }
      const qs = openQuestions(s);
      if (qs.length) {
        const q = h("button", "rf-badge", qs.length === 1 ? "1 question" : `${qs.length} questions`);
        q.dataset.testid = "bg-questions";
        q.title = qs[0].ask;
        q.addEventListener("click", () => { showQuestions = !showQuestions; draw(); });
        top.append(q);
      }
      if (x.history.length) {
        const l = h("button", "rf-link", showHistory ? "hide what it did" : "what it did");
        l.dataset.testid = "bg-history-toggle";
        l.addEventListener("click", () => { showHistory = !showHistory; draw(); });
        top.append(l);
      }
    }
    el.append(top);
    if (check) {
      const said = h("p", "rf-muted rf-checked");
      said.dataset.testid = "check-says";
      said.dataset.state = check.state;
      // What the check printed, word for word, for the curious.
      if (check.json?.problems?.length) said.title = check.json.problems.join("\n");
      for (const part of verifyWords(check)) {
        if (typeof part === "string") said.append(part);
        else {
          const l = h("button", "rf-link", part.clip);
          l.title = `${part.clip}: open this clip on the screen`;
          l.addEventListener("click", () => openClip(x.run, part.clip));
          said.append(l);
        }
      }
      el.append(said);
    }
    if (x.error) el.append(h("p", "rf-error", x.error));
    if (s && showQuestions) {
      const box = h("div", "rf-questions");
      for (const q of openQuestions(s)) {
        const d = h("div", "rf-q");
        d.append(h("p", "", `${q.clip_name || q.clip || "a clip"}: ${q.ask}`));
        if (q.detail) d.append(h("p", "rf-muted", q.detail));
        for (const ch of q.choices ?? []) {
          const cb = h("button", "rf-btn", ch);
          cb.addEventListener("click", () => act(() => x.post("/background/answer", { question: q.id, choice: ch })));
          d.append(cb);
        }
        box.append(d);
      }
      el.append(box);
    }
    if (showHistory) {
      const ol = h("ol", "rf-history");
      ol.dataset.testid = "bg-history";
      for (const r of [...x.history].reverse().slice(0, 20)) ol.append(h("li", "", roundWords(r)));
      el.append(ol);
    }
  };
  const off = x.subscribe((_, why) => { draw(_, why); if (why === "state") void readCheck(); });
  draw();
  return { stop() { clearInterval(checkTimer); off(); x.stop(); el.textContent = ""; } };
}

export default refineLine;
