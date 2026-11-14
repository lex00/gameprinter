// The status line under the header, on every tab: where the port stands,
// the one next step and who does it, with its button, and a clue when the
// person seems lost. It reads gp serve's /api/next/GAME, the same answer
// as `gp progress --next` and the MCP tool `next`; it suggests, and starts
// no work. On a page with no such route (the page on a server of its own)
// the line stays hidden.
//
// The clues the page itself sees (docs/progress.md, "What next"): idle on
// CLIPS for 2 minutes while a row only the person can play is open; the
// same row's recording dropped or too short three times in 10 minutes;
// four tab changes in 2 minutes with nothing done; the agent's question
// waiting 5 minutes while another tab shows. The server picks the words.
// One clue at a time, none while recording or reviewing, and each at most
// once in 10 minutes; after the third time on one signal its words offer
// another way in.

const $ = (id) => document.getElementById(id);

const POLL_MS = 5000;
const IDLE_MS = 120000;
const AGAIN_MS = 600000;
const CLUE_SHOWN_MS = 60000;
const HOPS = 4;
const HOP_MS = 120000;
const DROPS = 3;
const DROP_MS = 600000;
const AWAY_MS = 300000;
const BUSY = ["recording", "review", "sealed", "sending"];
const WHO = { tools: "THE TOOLS", agent: "YOUR AGENT", person: "YOU" };

/** What LET THE TOOLS TRY's record says, in words, or null: a result
 * shows for ten minutes after it ended. */
function toolsWords(t) {
  if (!t || !["done", "failed"].includes(t.state)) return null;
  if (t.ended_ms && Date.now() - t.ended_ms > 600000) return null;
  const first = String(t.error ?? "").split("\n")[0].trim().replace(/[.:;]+$/, "");
  if (t.state === "failed") return `The tools couldn't start: ${first || "no reason given"}. Your agent can look at why.`;
  const made = t.json?.made?.length ?? 0;
  const you = t.json?.needs_you?.length ?? 0;
  const items = (n) => (n === 1 ? "1 quest item" : `${n} quest items`);
  const got = made ? `they got ${items(made)} on their own` : "they got none on their own";
  return `The tools tried the quest log: ${got}, and ${items(you)} ${you === 1 ? "needs" : "need"} you.`;
}

/** Starts the line. `page` is app.js's: {game(), mode(), tab(), target(),
 * waiting(), idleMs(), playFrom(row), show(tab)}. */
export function startNext(page) {
  const el = $("next-line");
  if (!el) return;
  let lastAct = performance.now();
  const hops = [];
  const drops = new Map(); // row -> times its recording was dropped
  let awaySince = null;
  const shown = new Map(); // signal -> {n, at}
  let clue = null; // {signal, says, until}
  let prevMode = page.mode();
  let answer = null;
  // The agent's changes to the game (gp_run counts them, answer.changes):
  // when the count goes up, the views that don't redraw on their own hear
  // it ("gp-changed"), and on PLAY and REFINE the line offers a reload.
  let changes = null;
  let pageChanges = null;
  let changedNote = false;
  // LET THE TOOLS TRY's record (serve/jobs.rs), read with each answer.
  let tools = null;

  window.addEventListener("keydown", () => { lastAct = performance.now(); }, { capture: true });
  window.addEventListener("pointerdown", (e) => {
    lastAct = performance.now();
    const t = e.target instanceof Element ? e.target : null;
    if (t?.closest("#tabs")) hops.push(performance.now());
    else if (t?.closest("button, input, select, #queue li")) hops.length = 0;
  }, { capture: true });

  // A recording dropped in review (B) or stopped too short goes back to
  // plain play without being sent.
  setInterval(() => {
    const m = page.mode();
    if ((prevMode === "review" || prevMode === "recording") && m === "live") {
      const row = page.target();
      if (row) drops.set(row, [...(drops.get(row) ?? []), performance.now()]);
    }
    if (m === "recording" || m === "sending") hops.length = 0;
    prevMode = m;
  }, 200);

  const ready = (s, now) => {
    const x = shown.get(s);
    return !x || now - x.at > AGAIN_MS;
  };

  /** The one signal to ask a clue for, if any. */
  function signal(now) {
    if (BUSY.includes(page.mode())) return null;
    if (awaySince !== null && now - awaySince > AWAY_MS && ready("away", now)) return { signal: "away" };
    for (const [row, list] of drops) {
      const recent = list.filter((t) => now - t < DROP_MS);
      drops.set(row, recent);
      if (recent.length >= DROPS && ready("drops", now)) return { signal: "drops", row };
    }
    while (hops.length && now - hops[0] > HOP_MS) hops.shift();
    if (hops.length >= HOPS && ready("hops", now)) return { signal: "hops" };
    const idle = Math.min(now - lastAct, page.idleMs());
    if (page.tab() === "clips" && idle > IDLE_MS && ready("idle", now)) return { signal: "idle" };
    return null;
  }

  function button(s) {
    const a = s.act;
    if (!a) return null;
    const b = document.createElement("button");
    b.type = "button";
    b.tabIndex = -1;
    if (a.do === "play_from") {
      b.textContent = "PLAY FROM HERE";
      b.title = `Opens ${a.row} at its start point and records`;
      b.addEventListener("click", (e) => { e.currentTarget.blur(); void page.playFrom(a.row); });
    } else if (a.do === "start" && a.action === "triage") {
      // LET THE TOOLS TRY: gp mechanic triage through gp serve's page
      // actions (serve/jobs.rs), the agent's gp_run underneath; free, so
      // no yes card. While it runs the line's RUNNING part says so.
      const busy = tools?.state === "running";
      b.textContent = busy ? "THE TOOLS ARE TRYING" : "LET THE TOOLS TRY";
      b.disabled = busy;
      b.dataset.testid = "tools-try";
      b.title = "The tools play each quest item themselves and save a clip when one works. Items they can't get stay for you.";
      b.addEventListener("click", async (e) => {
        e.currentTarget.blur();
        e.currentTarget.disabled = true;
        try {
          const r = await fetch(`/api/capture/${encodeURIComponent(page.game())}/do/triage`, { method: "POST", headers: { "content-type": "application/json" }, body: "{}" });
          const v = await r.json().catch(() => ({}));
          tools = r.ok ? v : { state: "failed", error: v.error ?? `the page's server answered ${r.status}` };
        } catch (err) {
          tools = { state: "failed", error: "the page's server didn't answer" };
        }
        drawn = "";
        render();
      });
    } else if (a.do === "tab") {
      if (page.tab() === a.tab) return null;
      b.textContent = a.tab.toUpperCase();
      b.title = `Opens ${a.tab.toUpperCase()}`;
      b.addEventListener("click", (e) => { e.currentTarget.blur(); page.show(a.tab); });
    } else {
      return null;
    }
    return b;
  }

  // The cards of what the agent waits for the person's yes on (the asks
  // gp_run left, gp_mcp::asks), under the line on every tab: what it does
  // in plain words, with YES and NO. A yes shows until the agent uses it.
  let asksDrawn = "";
  function renderAsks() {
    const list = (answer?.waits_for_yes ?? []).filter((y) => y.id);
    const key = JSON.stringify(list.map((y) => [y.id, y.state]));
    if (key === asksDrawn) return;
    asksDrawn = key;
    let box = $("asks");
    if (!box) {
      box = document.createElement("div");
      box.id = "asks";
      box.setAttribute("aria-live", "polite");
      el.after(box);
    }
    box.textContent = "";
    box.hidden = list.length === 0;
    for (const y of list) {
      const card = document.createElement("div");
      card.className = "ask-card";
      card.dataset.ask = y.id;
      const head = document.createElement("span");
      head.className = "ask-head";
      head.textContent = y.state === "yes" ? "YOU SAID YES" : "YOUR AGENT ASKS";
      const says = document.createElement("span");
      says.className = "ask-says";
      says.textContent = y.game && y.game !== page.game() ? `${y.game}: ${y.says}` : y.says;
      card.append(head, says);
      if (y.state === "yes") {
        const n = document.createElement("span");
        n.className = "muted";
        n.textContent = "Your agent can go ahead, once.";
        card.appendChild(n);
      } else {
        for (const [word, val] of [["YES", "yes"], ["NO", "no"]]) {
          const b = document.createElement("button");
          b.type = "button";
          b.tabIndex = -1;
          b.textContent = word;
          b.className = `ask-${val}`;
          b.title = val === "yes" ? "Let your agent do this, once" : "Your agent won't do this";
          b.addEventListener("click", async (e) => {
            e.currentTarget.blur();
            for (const x of card.querySelectorAll("button")) x.disabled = true;
            try {
              const r = await fetch(`/api/asks/${encodeURIComponent(y.id)}`, {
                method: "POST",
                headers: { "content-type": "application/json" },
                body: JSON.stringify({ answer: val }),
              });
              if (!r.ok) says.textContent = `${y.says} (Your answer didn't save: ${(await r.json().catch(() => ({}))).error ?? r.status}.)`;
            } catch (err) {
              says.textContent = `${y.says} (Your answer didn't save. Is the page's server still running?)`;
            }
            asksDrawn = "";
            void tick();
          });
          card.appendChild(b);
        }
      }
      box.appendChild(card);
    }
  }

  let drawn = "";
  function render() {
    if (!answer) return;
    renderAsks();
    const now = performance.now();
    const c = clue && clue.until > now && !BUSY.includes(page.mode()) ? clue : null;
    // Drawn again only when it changes: a button redrawn under the mouse
    // loses the click.
    const key = JSON.stringify([c?.says, answer.stands, answer.stage, answer.next, answer.also, answer.running, changedNote, page.tab(), tools?.state, tools?.ended_ms]);
    if (key === drawn) return;
    drawn = key;
    el.textContent = "";
    const lead = document.createElement("span");
    lead.className = c ? "nx-clue" : "nx-stands";
    lead.textContent = c ? c.says : answer.stands;
    lead.title = c ? c.says : `Stage ${answer.stage.n} of ${answer.stage.of}: ${answer.stage.name}. ${answer.stands}`;
    el.appendChild(lead);
    const next = document.createElement("span");
    next.className = "nx-next";
    const who = document.createElement("span");
    who.className = "nx-who";
    who.textContent = `NEXT, ${WHO[answer.next.who] ?? answer.next.who}:`;
    next.append(who, ` ${answer.next.says}`);
    next.title = [answer.next.says, ...(answer.also ?? []).map((s) => `Or (${WHO[s.who] ?? s.who}): ${s.says}`)].join("\n");
    el.appendChild(next);
    const b = button(answer.next);
    if (b) el.appendChild(b);
    // LET THE TOOLS TRY is free work the person may start whenever the
    // tools' step is among the others too, not only when it is next.
    const start = !answer.next.act || answer.next.act.do !== "start" ? (answer.also ?? []).find((x) => x.act?.do === "start") : null;
    const sb = start && button(start);
    if (sb) el.appendChild(sb);
    if (changedNote && ["play", "refine"].includes(page.tab())) {
      const n = document.createElement("span");
      n.className = "nx-changed";
      n.textContent = "Your agent changed the game.";
      const rb = document.createElement("button");
      rb.type = "button";
      rb.tabIndex = -1;
      rb.textContent = "RELOAD";
      rb.title = "Loads the page again, to show the game as it is now";
      rb.addEventListener("click", () => location.reload());
      el.append(n, rb);
    }
    // What LET THE TOOLS TRY found, for ten minutes after, or why it
    // couldn't start.
    const tw = toolsWords(tools);
    if (tw) {
      const n = document.createElement("span");
      n.className = tools.state === "failed" ? "nx-tools nx-failed" : "nx-tools";
      n.dataset.testid = "tools-said";
      n.textContent = tw;
      el.appendChild(n);
    }
    // What the agent runs now, and what a button on the page started, so
    // the person sees it working.
    const runs = (answer.running ?? []).filter((r) => /^(your agent|started on the page): /.test(r.what));
    if (runs.length) {
      const r = document.createElement("span");
      r.className = "nx-run";
      r.textContent = `RUNNING: ${runs.map((x) => x.what.replace(/^(your agent|started on the page): /, "")).join("; ")}`;
      r.title = runs.map((x) => x.what).join("\n");
      el.appendChild(r);
    }
    el.hidden = false;
    hold();
  }

  // The tallest the line has been at this width: it grows when its words
  // need more lines and doesn't shrink back on the next poll, so the game
  // pane under it stays put. A new width starts again.
  let held = 0;
  let heldWidth = 0;
  function hold() {
    const w = el.clientWidth;
    if (w !== heldWidth) {
      heldWidth = w;
      held = 0;
      el.style.minHeight = "";
    }
    const h = el.offsetHeight;
    if (h > held) {
      held = h;
      el.style.minHeight = `${h}px`;
    }
  }
  window.addEventListener("resize", () => { heldWidth = 0; if (!el.hidden) hold(); });

  async function tick() {
    const game = page.game();
    if (!game) return;
    const now = performance.now();
    if (page.waiting() && page.tab() !== "clips") awaySince ??= now;
    else awaySince = null;
    const s = signal(now);
    const q = new URLSearchParams();
    if (s) {
      q.set("signal", s.signal);
      if (s.row) q.set("row", s.row);
      q.set("shown", String(shown.get(s.signal)?.n ?? 0));
    }
    try {
      const r = await fetch(`/api/next/${encodeURIComponent(game)}${s ? `?${q}` : ""}`);
      if (!r.ok) { el.hidden = true; return; }
      answer = await r.json();
      // REFINE's fidelity number reads the same scores (refine/fold.js).
      window.gpNext = answer;
      window.dispatchEvent(new CustomEvent("gp-next", { detail: answer }));
    } catch (e) {
      return;
    }
    try {
      const r = await fetch(`/api/capture/${encodeURIComponent(game)}/do/triage`);
      if (r.ok) tools = await r.json();
    } catch (e) { /* no page actions here */ }
    // A change the person made with a button on the page (LET THE TOOLS
    // TRY): the views refresh, with no note that the agent did anything.
    if (typeof answer.page_changes === "number") {
      if (pageChanges !== null && answer.page_changes > pageChanges) {
        window.dispatchEvent(new CustomEvent("gp-changed", { detail: { changes: answer.changes, page: true } }));
      }
      pageChanges = answer.page_changes;
    }
    if (typeof answer.changes === "number") {
      if (changes !== null && answer.changes > changes) {
        changedNote = true;
        window.dispatchEvent(new CustomEvent("gp-changed", { detail: { changes: answer.changes } }));
      }
      changes = answer.changes;
    }
    const c = answer.clue;
    if (c && !(clue && clue.signal === c.signal && clue.until > now) && ready(c.signal, now) && !BUSY.includes(page.mode())) {
      const x = shown.get(c.signal) ?? { n: 0, at: 0 };
      shown.set(c.signal, { n: x.n + 1, at: now });
      clue = { ...c, until: now + CLUE_SHOWN_MS };
      if (c.signal === "drops") drops.delete(s?.row);
      if (c.signal === "hops") hops.length = 0;
    }
    render();
  }

  void tick();
  setInterval(() => void tick(), POLL_MS);
  setInterval(render, 1000);
}
