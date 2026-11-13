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

  let drawn = "";
  function render() {
    if (!answer) return;
    const now = performance.now();
    const c = clue && clue.until > now && !BUSY.includes(page.mode()) ? clue : null;
    // Drawn again only when it changes: a button redrawn under the mouse
    // loses the click.
    const key = JSON.stringify([c?.says, answer.stands, answer.stage, answer.next, answer.also, page.tab()]);
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
    el.hidden = false;
  }

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
    } catch (e) {
      return;
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
