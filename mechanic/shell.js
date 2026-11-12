// The page's four tabs by phase, and the agent's log beside every one.
//
// CLIPS is capture and review together: the quest log, the game pane on
// the original (THE ORIGINAL: play, record with R, hand off, talk) or on
// the model (ON THE MODEL: a clip beside the game's model, the strip and
// the layouts), and a line of the background work under it. REFINE is the
// background work itself (gp serve's /api/capture/RUN/background): the jobs
// running with their bars, what they finished, the clips in progress, the
// questions. PLAY is the rebuild (FREE PLAY, with its settings). THEMES is
// the game's themes. The tab is in the URL (?tab=clips|refine|play|themes).
//
// app.js keeps the game and its modes; this file only chooses what shows.
// REFINE and THEMES take the game's place, and the game holds while they
// show. The REFINE pane's parts come from mechanic/refine/ when they are
// there (cards.js, queue.js, line.js, strip.js: the editor's background
// work as plain modules); until then a plain view of the same state.

const $ = (id) => document.getElementById(id);

/** Starts the tabs. `page` is app.js's: {setTab(t), mode(), liveKind(),
 * pause(on), toast(text), api(path, body?), game()}. */
export function startShell(page) {
  const tabs = { clips: $("tab-clips"), refine: $("tab-work"), play: $("tab-play"), themes: $("tab-themes") };
  let phase = "clips";
  const busy = () => ["recording", "review", "sealed", "sending"].includes(page.mode());

  function show(p, fromUrl) {
    if (p !== phase && busy()) { page.toast("Finish the recording first."); return; }
    const wasPlay = phase === "play";
    phase = p;
    for (const [k, b] of Object.entries(tabs)) b.classList.toggle("on", k === p);
    const pane = p === "refine" || p === "themes";
    $("pane-work").hidden = p !== "refine";
    $("pane-themes").hidden = p !== "themes";
    document.querySelector("main").hidden = pane;
    $("views").hidden = p !== "clips";
    if (pane) {
      // The rebuild stops; the original holds where it is.
      if (page.liveKind()) page.setTab("mech");
      page.pause(true);
      if (p === "refine") work.start(); else themes.load();
    } else {
      page.pause(false);
      work.stop();
      if (p === "play" && page.liveKind() !== "free") page.setTab("free");
      if (p === "clips" && (wasPlay || page.liveKind() === "free")) page.setTab("mech");
    }
    if (!fromUrl) {
      const u = new URL(location.href);
      u.searchParams.set("tab", p);
      history.replaceState(null, "", u);
    }
  }
  for (const [k, b] of Object.entries(tabs)) b.addEventListener("click", () => show(k));
  // FREE PLAY started or left from inside the game (Esc) moves the tab.
  setInterval(() => {
    if (phase === "play" && page.liveKind() !== "free" && !busy()) show("clips");
    else if (phase === "clips" && page.liveKind() === "free") show("play");
  }, 500);

  // The agent's log at the right, on every tab; L or the header's button.
  const toggle = $("log-toggle");
  const setLog = (on) => {
    $("log-panel").hidden = !on;
    document.body.classList.toggle("log-open", on);
    toggle.setAttribute("aria-pressed", String(on));
    toggle.classList.remove("news");
    try { localStorage.setItem("gp-log-open", on ? "1" : ""); } catch (e) { /* not kept */ }
  };
  toggle.addEventListener("click", (e) => { setLog($("log-panel").hidden); e.currentTarget.blur(); });
  window.addEventListener("keydown", (e) => {
    if (e.code !== "KeyL" || e.ctrlKey || e.metaKey || e.altKey || document.activeElement?.tagName === "INPUT") return;
    if (page.mode() === "review") return; // the note may be typed
    setLog($("log-panel").hidden);
  });
  let open = false;
  try { open = localStorage.getItem("gp-log-open") === "1"; } catch (e) { /* closed */ }
  if (new URLSearchParams(location.search).get("log") === "1") open = true;
  setLog(open);
  // New lines while it's closed light the button.
  new MutationObserver(() => { if ($("log-panel").hidden) toggle.classList.add("news"); })
    .observe($("w-status"), { childList: true, characterData: true, subtree: true });

  const work = refineWork(page);
  const themes = themesPane(page);
  strip(page);

  const asked = new URLSearchParams(location.search).get("tab");
  // The old names: mech and refine were CLIPS's two views, free was PLAY.
  const p = { mech: "clips", free: "play", refine: "clips", work: "refine" }[asked] ?? asked;
  if (p && tabs[p]) show(p, true);
  if (asked === "refine" && new URLSearchParams(location.search).get("view") !== "work") page.setTab("refine");
  return { show };
}

/** The background work's state from gp serve, or null where there's none
 * (the page on a server of its own). */
async function background(page) {
  const game = page.game();
  if (!game) return null;
  const r = await fetch(`/api/capture/${encodeURIComponent(game)}/background`);
  if (!r.ok) return null;
  return r.json();
}

/** "####......" for k of n. */
function bars(k, n, w = 10) {
  const f = n > 0 ? Math.max(0, Math.min(w, Math.round((k / n) * w))) : 0;
  return "#".repeat(f) + ".".repeat(w - f);
}

/** One line for a job running: "round 3 on dk.m75.037 ####......". */
function jobLine(j) {
  const what = j.kind === "refine" ? `round ${j.round ?? "?"} on ${j.name ?? j.clip}` : `scoring ${j.name ?? j.clip}`;
  return `${what} ${bars(j.k ?? 0, j.n ?? 0)}${j.target?.says ? "  " + j.target.says : ""}`;
}

/** CLIPS's line under the game: the background work as it goes. */
function strip(page) {
  const el = $("refine-strip");
  let parts = null;
  import("./refine/strip.js").then((m) => { parts = m; }).catch(() => undefined);
  let mounted = null;
  const tick = async () => {
    if (parts && !mounted) { el.textContent = ""; mounted = (parts.refineStrip ?? parts.default)(el, { api: page.api, run: page.game() }); return; }
    if (mounted) return;
    const s = await background(page).catch(() => null);
    if (!s) { el.textContent = ""; return; }
    const jobs = s.jobs ?? [];
    el.textContent = jobs.length ? jobs.map(jobLine).join("   ") : (s.says ?? "");
  };
  void tick();
  setInterval(() => void tick(), 2000);
}

/** REFINE's pane. */
function refineWork(page) {
  const body = $("work-body");
  let timer = null;
  let parts = null;
  let mounted = [];
  const loadParts = Promise.all(["line", "cards", "queue"].map((n) => import(`./refine/${n}.js`)))
    .then((ms) => { parts = ms; })
    .catch(() => undefined);

  async function draw() {
    const s = await background(page).catch(() => null);
    body.innerHTML = "";
    if (!s) {
      p(body, "The background work runs in gp serve. Open this page from it: gp mechanic start GAME opens it there, starting gp serve when none runs.", "muted");
      return;
    }
    p(body, s.says ?? "", "");
    const ctl = document.createElement("p");
    const b = document.createElement("button");
    b.textContent = s.enabled ? "PAUSE THE WORK" : "START THE WORK";
    b.addEventListener("click", async () => { await page.api(`/api/capture/${page.game()}/background/${s.enabled ? "stop" : "start"}`, {}).catch((e) => page.toast(String(e.message ?? e))); void draw(); });
    ctl.append(b, document.createTextNode(`  ${s.workers ?? 0} of ${s.max_workers ?? 0} workers, ${s.agents ?? 0} of ${s.max_agents ?? 0} agents`));
    body.append(ctl);
    h(body, "Running now");
    if (!(s.jobs ?? []).length) p(body, "Nothing running.", "muted");
    for (const j of s.jobs ?? []) row(body, j.kind === "refine" ? `round ${j.round ?? "?"}` : "scoring", j.name ?? String(j.clip), j.k ?? 0, j.n ?? 0, j.target?.says ?? `${j.secs ?? 0} s`);
    const qs = (s.questions ?? []).filter((q) => q.answer == null);
    if (qs.length) {
      h(body, "Questions");
      for (const q of qs) {
        const d = document.createElement("div");
        d.className = "q";
        p(d, `${q.clip ?? ""}: ${q.ask ?? ""}`, "");
        if (q.detail) p(d, q.detail, "muted");
        for (const c of q.choices ?? []) {
          const cb = document.createElement("button");
          cb.textContent = String(c).toUpperCase();
          cb.addEventListener("click", async () => { await page.api(`/api/capture/${page.game()}/background/answer`, { question: q.id, choice: c }).catch((e) => page.toast(String(e.message ?? e))); void draw(); });
          d.append(cb, document.createTextNode(" "));
        }
        body.append(d);
      }
    }
    h(body, "Just done");
    if (!(s.done ?? []).length) p(body, "Nothing yet.", "muted");
    for (const j of s.done ?? []) row(body, j.kind === "refine" ? `round ${j.round ?? "?"}` : "scored", j.name ?? String(j.clip), 1, 1, j.target?.says ?? "");
    const scores = Object.entries(s.scores ?? {});
    h(body, `Clips scored on the model (${scores.length})`);
    for (const [clip, sc] of scores.slice(0, 200)) {
      const pm = sc.permille ?? 0;
      row(body, sc.exact ? "exact" : sc.error ? "error" : "", clip, pm, 1000, sc.says ?? sc.error ?? `${(pm / 10).toFixed(1)}%`);
    }
  }

  function start() {
    if (timer) return;
    void loadParts.then(() => {
      if (parts) {
        if (!mounted.length) {
          body.innerHTML = "";
          mounted = parts.map((m, i) => {
            const el = document.createElement("div");
            el.className = `work-${["line", "cards", "queue"][i]}`;
            body.append(el);
            const f = m.refineLine ?? m.refineCards ?? m.refineQueue ?? m.default;
            return f(el, { api: page.api, run: page.game() });
          });
        }
        return;
      }
      void draw();
      timer = setInterval(() => void draw(), 2000);
    });
  }
  function stop() {
    if (timer) { clearInterval(timer); timer = null; }
  }
  return { start, stop };
}

/** THEMES's pane: the game's themes from the registry, and what's coming. */
function themesPane(page) {
  const body = $("themes-body");
  let done = false;
  async function load() {
    if (done) return;
    let t;
    try { t = await page.api("api/themes"); } catch (e) { body.textContent = `No themes: ${e.message ?? e}`; return; }
    done = true;
    body.innerHTML = "";
    p(body, "Theme work comes once the game plays right with the original's pictures. Each theme then gets its own clip review and refinement here: the theme drawn over the same clips, held to the original's motion. For now, the themes there are:", "muted");
    h(body, `In the registry (${t.registry ?? "none"})`);
    if (!(t.themes ?? []).length) p(body, "No theme for this game in the registry yet.", "muted");
    for (const th of t.themes ?? []) p(body, `${th.title ?? th.name}: ${th.name} ${th.latest ?? ""}`, "");
    h(body, "The game's theme kit");
    if (!(t.kit ?? []).length) p(body, "The game has no theme kit (games/GAME/themes/kit).", "muted");
    else p(body, `games/${page.game()}/themes/kit: ${t.kit.join(", ")}. gp theme sheet makes a theme's starting grids from it.`, "");
  }
  return { load };
}

function p(parent, text, cls) {
  const e = document.createElement("p");
  if (cls) e.className = cls;
  e.textContent = text;
  parent.append(e);
  return e;
}
function h(parent, text) {
  const e = document.createElement("h3");
  e.textContent = text;
  parent.append(e);
}
function row(parent, kind, name, k, n, says) {
  const r = document.createElement("div");
  r.className = "row";
  const a = document.createElement("span");
  a.textContent = `${kind} ${name}`.trim();
  const bar = document.createElement("span");
  bar.className = "bar";
  bar.title = `${k} of ${n}`;
  const fill = document.createElement("span");
  fill.style.width = `${n > 0 ? Math.round((k / n) * 100) : 0}%`;
  bar.append(fill);
  const s = document.createElement("span");
  s.textContent = says;
  r.append(a, bar, s);
  parent.append(r);
}
