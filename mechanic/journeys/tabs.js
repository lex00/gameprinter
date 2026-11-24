// The tabs follow the open copy (issue #5, the owner's decision of how the
// three journeys share the page's four tabs). A port opens the game's page
// at /mechanic/GAME/ with CLIPS, REFINE, PLAY and THEMES as before. A copy
// with nothing to port (no original to match clip by clip) opens here, at
// /start/?copy=KIND/NAME&tab=TAB, with CLIPS and REFINE hidden:
//
// | Copy | Tabs | First tab |
// |---|---|---|
// | A theme | PLAY (the game in that look), THEMES (the theme's journey, BESIDE THE ORIGINAL among its steps) | THEMES |
// | A new game, from nothing or a template | PLAY (the game built to play), STEPS (its journey) | STEPS |
//
// The journey itself is journey.js's; this file only chooses what shows and
// draws PLAY. Nothing here publishes or deletes.

import { mountJourney, when } from "./journey.js";

/** Each copy's tabs, in the order the bar shows them. */
export const TABS = { theme: ["play", "themes"], new: ["play", "steps"] };
/** The tab a copy opens at: CARRY ON, the first screen and the root lead there. */
export const FIRST_TAB = { theme: "themes", new: "steps" };
const ALL = ["clips", "work", "play", "themes", "steps"];

const $ = (id) => document.getElementById(id);
const el = (tag, cls, text) => {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
};

/**
 * Mounts copy KIND/NAME's tabs: the bar (#tabs), its journey in #journey
 * and PLAY in #pane-play, at `tab` (else the copy's first tab). `nav` is
 * start.js's {open(key), home(), api(path, body)}. Returns a function that
 * stops it.
 */
export function mountCopy(kind, name, tab, nav) {
  const mine = TABS[kind] ?? [];
  const bar = $("tabs");
  const buttons = Object.fromEntries(ALL.map((t) => [t, $(`tab-${t}`)]));
  const stopJourney = mountJourney($("journey"), kind, name, nav);
  const play = playPane($("pane-play"), kind, name, nav);
  let at = null;
  const handlers = [];

  function show(t) {
    if (!mine.includes(t)) t = FIRST_TAB[kind];
    at = t;
    bar.hidden = false;
    for (const [k, b] of Object.entries(buttons)) {
      b.hidden = !mine.includes(k);
      b.classList.toggle("on", k === t);
      b.setAttribute("aria-pressed", String(k === t));
    }
    $("journey").hidden = t === "play";
    if (t === "play") play.start(); else play.stop();
    // Written by hand: URLSearchParams would spell the copy's / as %2F.
    history.replaceState(history.state, "", `/start/?copy=${kind}/${name}&tab=${t}`);
  }
  for (const k of mine) {
    const f = () => show(k);
    buttons[k].addEventListener("click", f);
    handlers.push([buttons[k], f]);
  }
  show(tab);
  return () => {
    stopJourney();
    play.stop();
    for (const [b, f] of handlers) b.removeEventListener("click", f);
    bar.hidden = true;
    $("pane-play").hidden = true;
    return at;
  };
}

/** PLAY for a copy: a theme's game in that look (`gp theme look play`, the
 * copy's SEE IT IN THE GAME), or a new game built to play (BUILD IT TO
 * PLAY). The game plays in a frame here; leaving PLAY stops it. */
function playPane(box, kind, name, nav) {
  const base = `/api/journeys/${encodeURIComponent(kind)}/${encodeURIComponent(name)}`;
  const which = kind === "theme" ? "look-play" : "build";
  let v = null;
  let timer = null;
  let on = false;
  let drawn = "";
  let err = "";

  async function load() {
    try {
      // Through start.js's wait for a server moving onto a new gp: the
      // game stays in its frame meanwhile.
      v = await nav.api(base);
    } catch (e) {
      v = null;
      err = `This copy can't be read: ${e.message ?? e}`;
    }
    const key = JSON.stringify([v?.jobs?.[which] ?? null, v?.files?.play ?? null, v?.answer?.facts?.built_current ?? null, gameLink(), err]);
    if (key !== drawn) {
      drawn = key;
      draw();
    }
  }

  function again() {
    clearTimeout(timer);
    if (!on) return;
    const running = v?.jobs?.[which]?.state === "running";
    timer = setTimeout(async () => { if (!document.hidden) await load(); again(); }, running ? 1500 : 6000);
  }

  /** The link that plays the copy, or null while there's none. A theme's
   * game is served by a process of its own for an hour: gp serve says
   * whether its link still answers (`serving`). */
  function gameLink() {
    if (!v) return null;
    if (kind === "new") return v.files?.play ?? null;
    const j = v.jobs?.["look-play"];
    if (j?.state !== "done" || !j.serving) return null;
    return (j.json?.links ?? [])[0] ?? null;
  }

  async function press() {
    err = "";
    try { await nav.api(`${base}/do/${which}`, {}); } catch (e) { err = e.message ?? String(e); }
    drawn = "";
    await load();
    again();
  }

  function button(label, about) {
    const job = v.jobs?.[which] ?? { state: "never" };
    const wrap = el("div", "jr-act");
    wrap.dataset.act = which;
    const b = el("button", "jr-btn go", job.state === "running" ? `${label}: RUNNING` : label);
    b.type = "button";
    b.title = about;
    b.disabled = job.state === "running";
    b.addEventListener("click", () => void press());
    const said = el("span", "muted", about);
    if (job.state === "running") said.textContent = `Started ${when(job.started_ms)}. ${about}`;
    else if (job.state === "failed") { said.className = "jr-err"; said.textContent = `It didn't work, ${when(job.ended_ms)}. Your agent can look at why. The tools said: ${String(job.error ?? "no reason given").split("\n")[0]}`; }
    wrap.append(b, " ", said);
    return wrap;
  }

  /** The game in a frame, and the same in a tab of its own. `src` is the
   * frame's: a theme's game comes through gp serve (/look/), since the
   * public page's headers let only its own origin hold it in a frame. */
  function frame(src, label, own = src) {
    const f = el("iframe", "cp-game");
    f.id = "cp-game";
    f.src = src;
    f.title = label;
    f.allow = "autoplay; gamepad; fullscreen";
    const open = el("a", "jr-btn jr-link", "OPEN IT IN A TAB OF ITS OWN");
    open.href = own;
    open.target = "_blank";
    open.rel = "noopener";
    return [f, open];
  }

  function draw() {
    box.textContent = "";
    if (!v) {
      box.append(el("p", "jr-err", err || "Loading."));
      return;
    }
    const f = v.answer?.facts ?? {};
    const title = f.title ?? name;
    const link = gameLink();
    if (kind === "theme") {
      const game = f.from?.title ?? null;
      box.append(el("h2", "", game ? `${game} in your theme ${title}` : `Your theme ${title}`));
      if (!game) {
        box.append(el("p", "muted", "This theme doesn't say which game it is for yet. Tell your agent which game, and it plays here."));
        return;
      }
      if (link) {
        const said = link.startsWith("/mechanic/")
          ? `${game}'s screen with your theme picked: the model alone in it, played from its own start. Pick another view at its top to see it beside the original. A change to your theme shows by itself.`
          : f.from?.kind === "new" ? "Your game, drawn in your theme. It plays from its own files, on this computer only." : `Load your own copy of ${game} in it to play. It plays on this computer only, for an hour from when it was opened. Leaving PLAY stops it.`;
        box.append(el("p", "muted", said));
        // The screen is gp serve's own page: framed as it is. A game a
        // server of its own plays comes through /look/.
        if (link.startsWith("/mechanic/")) box.append(...frame(link, `${game} in your theme ${title}`));
        else {
          const u = new URL(link);
          box.append(...frame(`${base}/look${u.pathname}${u.search}`, `${game} in your theme ${title}`, link));
        }
      } else {
        const stopped = v.jobs?.["look-play"]?.state === "done";
        box.append(el("p", "", stopped
          ? `The game you opened before has stopped: it plays for an hour from when it is opened. Open it again to play ${game} in your theme.`
          : `Play ${game} drawn in your theme, on this computer only.`));
        box.append(button("SEE IT IN THE GAME", "Opens the game with your theme here, on this computer only."));
      }
    } else {
      box.append(el("h2", "", `Play ${title}`));
      if (link) {
        if (f.built_ms && f.built_current === false) {
          box.append(el("p", "jr-err", "You changed it since the last build: build it again to play the change."));
          box.append(button("BUILD IT TO PLAY", "Builds your game to play in this browser."));
        }
        box.append(el("p", "muted", "Your game, built to play in this browser from its own files. Leaving PLAY stops it."));
        box.append(...frame(link, `${title}, your new game`));
      } else {
        box.append(el("p", "", "Your game isn't built to play yet. BUILD IT TO PLAY builds it in this browser; it takes a few minutes the first time."));
        box.append(button("BUILD IT TO PLAY", "Builds your game to play in this browser. A few minutes the first time."));
      }
    }
    if (err) box.append(el("p", "jr-err", err));
  }

  return {
    start() {
      if (on) return;
      on = true;
      box.hidden = false;
      drawn = "";
      void load().then(again);
    },
    stop() {
      on = false;
      clearTimeout(timer);
      box.hidden = true;
      // The game stops: no sound from a tab that isn't showing.
      box.textContent = "";
      drawn = "";
    },
  };
}
