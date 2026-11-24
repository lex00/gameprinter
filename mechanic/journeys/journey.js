// A copy's journey on the first screen (start.js): a theme for a game, or
// a new game from nothing or from a template. Its six steps, whose turn
// each is (you, your agent, or the tools), where it stands and the one
// next step, from gp serve's /api/journeys/KIND/NAME: the same answer
// `gp progress --next --copy` and the MCP tool `next` give the agent. Each step opens to what
// it needs: words to type, a button that runs a free gp command (through
// the agent's own gp_run, marked as the person's), or what to say to the
// agent. A step with no tool yet says so. Nothing here publishes or
// deletes: the share step shows what to send, and sends nothing.

import { pictureCard, picturesSection } from "../picture-card.js";
import { soundCard } from "../sound-card.js";
import { spriteCard } from "../sprite-card.js";

export const WHO = { person: "YOU", agent: "YOUR AGENT", tools: "THE TOOLS" };

const el = (tag, cls, text) => {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
};

/** When, in words: "just now", "5 minutes ago", "on 14 Nov". */
export function when(ms) {
  if (!ms) return "never";
  const s = (Date.now() - ms) / 1000;
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.round(s / 60)} minute${Math.round(s / 60) === 1 ? "" : "s"} ago`;
  if (s < 86400) return `${Math.round(s / 3600)} hour${Math.round(s / 3600) === 1 ? "" : "s"} ago`;
  return `on ${new Date(ms).toLocaleDateString(undefined, { day: "numeric", month: "short" })}`;
}

const whoTag = (w) => el("span", `jr-who who-${w}`, WHO[w] ?? w);

/** What each button is called, and what it does, in words. */
const BUTTONS = {
  "look-check": ["CHECK MY THEME", "Reads your theme the way the game does and lists what is wrong. Free, under a minute."],
  "look-play": ["SEE IT IN THE GAME", "Opens the game's screen with your theme picked, on this computer only."],
  shots: ["PICTURES OF IT IN THE GAME", "Plays the game with your theme from your copy of it and keeps pictures of each stage here. A few minutes."],
  compare: ["BESIDE THE ORIGINAL", "Plays the game's route twice from your copy of it, once in your theme and once in the game's own pictures, and keeps the two pictures of each moment side by side here. A few minutes."],
  build: ["BUILD IT TO PLAY", "Builds your game to play in this browser. A few minutes the first time."],
  render: ["SHOW THE STAGE", "Draws the stage, each piece coloured by what it is."],
  verify: ["CHECK THE STAGE", "Checks the stage still follows the game's rules and can be finished. Free."],
  undo: ["UNDO THE LAST CHANGE", "Puts the stage back as it was before the last change."],
  share: ["MAKE A SHARE LINK", "Puts your game in your own registry on this computer, checked to hold nothing of anyone's ROM, and makes the link that plays it. A few minutes the first time."],
};

/**
 * Mounts the journey of copy KIND/NAME in `box`; `nav` is start.js's
 * {open(key), home(), api(path, body)}. Returns a function that stops it.
 */
export function mountJourney(box, kind, name, nav) {
  const base = `/api/journeys/${encodeURIComponent(kind)}/${encodeURIComponent(name)}`;
  let v = null;
  let drawn = "";
  let timer = null;
  let stopped = false;
  // Which steps the person opened or closed, kept across redraws.
  const opened = new Map();
  // Words the person is typing, kept across redraws.
  const typed = new Map();
  const errors = new Map();
  // The pictures list is kept across redraws: it draws itself.
  let pictures = null;
  // The moment shown beside the original, kept across redraws.
  let moment = 0;

  async function load() {
    try {
      // Through start.js's wait for a server moving onto a new gp.
      v = await nav.api(base);
    } catch (e) {
      if (stopped) return;
      box.textContent = "";
      box.append(el("p", "jr-err", `This copy can't be read: ${e.message ?? e}`));
      const back = el("button", "jr-btn", "BACK TO THE FIRST SCREEN");
      back.type = "button";
      back.addEventListener("click", nav.home);
      box.append(back);
      return;
    }
    // Not drawn again while the person types in a box of this view, so
    // the box keeps its focus; the next look draws it.
    const busy = box.contains(document.activeElement) && /^(INPUT|TEXTAREA|SELECT)$/.test(document.activeElement.tagName);
    const key = JSON.stringify([v, [...errors]]);
    if (key !== drawn && !busy) {
      drawn = key;
      draw();
    }
  }

  function again() {
    clearTimeout(timer);
    if (stopped) return;
    const running = Object.values(v?.jobs ?? {}).some((j) => j.state === "running");
    timer = setTimeout(async () => { if (!document.hidden) await load(); again(); }, running ? 1500 : 5000);
  }

  async function press(which) {
    errors.delete(which);
    try {
      await nav.api(`${base}/do/${which}`, {});
    } catch (e) {
      errors.set(which, e.message ?? String(e));
    }
    await load();
    again();
  }

  async function set(body, errKey) {
    errors.delete(errKey);
    try { await nav.api(`${base}/set`, body); } catch (e) { errors.set(errKey, e.message ?? String(e)); }
    drawn = "";
    await load();
  }

  /** A button that runs one of the copy's gp commands, with its last
   * run under it in words. */
  function button(which, opts = {}) {
    const wrap = el("div", "jr-act");
    wrap.dataset.act = which;
    const [label, about] = BUTTONS[which];
    const job = v.jobs?.[which] ?? { state: "never" };
    const b = el("button", `jr-btn${opts.go ? " go" : ""}`, job.state === "running" ? `${label}: RUNNING` : label);
    b.type = "button";
    b.title = about;
    b.disabled = job.state === "running";
    b.addEventListener("click", () => void press(which));
    wrap.append(b);
    const said = el("span", "muted", "");
    if (job.state === "running") said.textContent = `Started ${when(job.started_ms)}. ${about}`;
    else if (job.state === "failed") { said.className = "jr-err"; said.textContent = `It didn't work, ${when(job.ended_ms)}. Your agent can look at why. The tools said: ${firstLine(job.error)}`; }
    else if (job.state === "done") { said.className = "jr-ok"; said.textContent = `Done ${when(job.ended_ms)}.`; }
    else said.textContent = about;
    if (errors.has(which)) { said.className = "jr-err"; said.textContent = errors.get(which); }
    wrap.append(" ", said);
    return wrap;
  }

  function doneButton(step, label, title) {
    const done = (v.answer.facts.steps_done ?? []).includes(step);
    const b = el("button", `jr-btn${done ? "" : " go"}`, done ? `${label}: DONE (PRESS TO UNDO)` : label);
    b.type = "button";
    b.title = title;
    b.dataset.done = step;
    b.addEventListener("click", () => void set({ step, done: !done }, `done-${step}`));
    const wrap = el("div", "jr-act");
    wrap.append(b);
    if (errors.has(`done-${step}`)) wrap.append(" ", el("span", "jr-err", errors.get(`done-${step}`)));
    return wrap;
  }

  /** What to say to the agent, in a quote. */
  function say(words) {
    const p = el("p");
    p.append("You can say to your agent: ", el("q", "", words));
    return p;
  }

  // -------------------------------------------------------------------
  // The theme's steps.

  function themeBody(id, b) {
    const f = v.answer.facts;
    const game = f.from?.title ?? "its game";
    switch (id) {
      case "T1": {
        b.append(el("p", "", f.from ? `This theme is for ${game}${f.from.kind === "new" ? ", your new game" : ""}. A theme is the game's pictures drawn again in a style of your own: the game plays the same, only how it looks changes.` : "This theme doesn't say which game it is for. Tell your agent which game."));
        const t = el("textarea");
        t.id = "jr-style";
        t.maxLength = 300;
        t.placeholder = "like night time, neon, or paper cutouts";
        t.value = typed.get("style") ?? f.style ?? "";
        t.addEventListener("input", () => typed.set("style", t.value));
        const l = el("label", "jr-field");
        l.append(el("span", "", "The style, in a few words"), t);
        const save = el("button", "jr-btn go", "SAVE THE STYLE");
        save.type = "button";
        save.addEventListener("click", () => { typed.delete("style"); void set({ style: t.value }, "style"); });
        b.append(l, save);
        if (errors.has("style")) b.append(el("p", "jr-err", errors.get("style")));
        break;
      }
      case "T2": {
        b.append(el("p", "", f.pictures ? `Your theme has ${f.pictures} picture${f.pictures === 1 ? "" : "s"}${f.to_draw ? `, and ${f.to_draw} still to draw` : ""}.` : "Your theme has no pictures yet."));
        b.append(el("p", "", "Your agent can make them in your style with an image model. That costs money, so it asks you first, on a card at the top of this page, before it spends anything. Or bring pictures of your own."));
        b.append(say(`Make the pictures for my theme ${f.title}, in the style ${f.style ?? "I described"}.`));
        const p = el("p", "muted jr-link");
        p.append("Your own pictures go in this folder, then tell your agent: ", el("code", "", `${f.dir}/theme/art`));
        b.append(p);
        break;
      }
      case "T3": {
        b.append(el("p", "", "The tools read your theme the way the game does: every picture there, the right size and shape, and nothing copied from the original game."));
        b.append(button("look-check", { go: true }));
        const ck = v.files.check;
        if (ck?.at_ms) {
          const p = ck.problems ?? 0, w = ck.warnings ?? 0;
          b.append(el("p", p ? "jr-err" : "jr-ok", `The last check, ${when(ck.at_ms)}${f.check && !f.check.current ? " (your theme changed since)" : ""}: ${p ? `${p} problem${p === 1 ? "" : "s"}` : "no problems"}, ${w} warning${w === 1 ? "" : "s"}.`));
          const list = el("ul", "jr-findings");
          const shown = (ck.findings ?? []).filter((x) => x.level === "problem" || x.level === "warning");
          for (const x of shown.slice(0, 12)) {
            const li = el("li");
            li.append(el("span", `lv lv-${x.level}`, x.level === "problem" ? "PROBLEM" : "WARNING"), `${x.message}${x.file ? ` (${x.file}${x.line ? ` line ${x.line}` : ""})` : ""}`);
            list.append(li);
          }
          if (shown.length > 12) list.append(el("li", "muted", `and ${shown.length - 12} more: your agent has the whole list.`));
          if (shown.length) b.append(list);
          const notes = (ck.findings ?? []).filter((x) => x.level === "note");
          if (notes.length) {
            const d = el("details");
            d.append(el("summary", "", `${notes.length} note${notes.length === 1 ? "" : "s"} for your agent`));
            const ul = el("ul", "jr-findings");
            for (const x of notes) ul.append(el("li", "", x.message));
            d.append(ul);
            b.append(d);
          }
          if (p) b.append(say("Fix the problems the look check found in my theme."));
        }
        break;
      }
      case "T4": {
        b.append(el("p", "", "See your theme in the game, on this computer only. Load your own copy of the game in it and play every stage."));
        b.append(button("look-play", { go: true }));
        const pl = v.jobs?.["look-play"];
        const links = pl?.state === "done" ? (pl.json?.links ?? []) : [];
        links.forEach((l, i) => {
          const a = el("a", "jr-btn go jr-link", links.length > 1 ? `OPEN THE GAME (${i + 1})` : "OPEN THE GAME");
          a.href = l;
          a.target = "_blank";
          a.rel = "noopener";
          b.append(a);
        });
        if (f.rom) b.append(button("shots"));
        else b.append(el("p", "muted", "Pictures of your theme in the game need your own copy of the game on this computer. Your agent can bring it in."));
        const shots = v.files.shots ?? [];
        if (shots.length) {
          const row = el("div", "jr-pics");
          for (const s of shots.slice(0, 12)) {
            const fig = el("figure");
            const img = el("img");
            img.src = s;
            img.alt = s.split("/").pop();
            img.loading = "lazy";
            fig.append(img, el("figcaption", "", s.split("/").slice(-2).join(" ")));
            row.append(fig);
          }
          b.append(row);
        }
        break;
      }
      case "T5": {
        b.append(el("p", "", "Play it and tell your agent which pictures to change. A new picture shows as a card with the one you have beside it, and nothing changes until you press ACCEPT."));
        b.append(beside(f));
        b.append(say("Make the hero's hat bigger."));
        pictures ??= picturesSection();
        b.append(pictures);
        b.append(doneButton("pictures", "READY TO SHARE", "You like your theme as it is: on to sharing it"));
        break;
      }
      case "T6": {
        const s = f.share ?? {};
        b.append(el("p", "", "Other people play your theme from the public page once it is published there. Publishing is done by the people who keep the list of themes: you send them a request. Nothing is published from this page."));
        if (s.link) {
          const p = el("p", "jr-link");
          p.append("The share link, once it is published: ");
          const a = el("a", "", s.link);
          a.href = s.link;
          a.target = "_blank";
          a.rel = "noopener";
          p.append(a);
          b.append(p);
        } else if (s.why) b.append(el("p", "muted", `No share link yet: ${s.why}.`));
        if (s.request) {
          b.append(el("h3", "", "The request to send"));
          const p = el("p", "jr-link");
          p.append("Open ");
          const a = el("a", "", s.request.where);
          a.href = s.request.where;
          a.target = "_blank";
          a.rel = "noopener";
          p.append(a, ", put this as its title, and the words under it as its text. Your theme's folder needs to be online first, where people can see it (your agent can help).");
          b.append(p, el("pre", "jr-said", s.request.title));
          const t = el("textarea", "jr-request");
          t.readOnly = true;
          t.value = s.request.body;
          t.setAttribute("aria-label", "The request's text");
          b.append(t);
          const c = el("button", "jr-btn", "COPY THE TEXT");
          c.type = "button";
          c.addEventListener("click", async () => {
            try { await navigator.clipboard.writeText(`${s.request.title}\n\n${s.request.body}`); c.textContent = "COPIED"; } catch (e) { t.select(); }
          });
          b.append(c);
        }
        break;
      }
    }
  }

  /** Your theme beside the game in its own pictures, moment by moment
   * along the game's route: the pairs the last BESIDE THE ORIGINAL kept. */
  function beside(f) {
    const box = el("section", "jr-beside");
    box.id = "jr-beside";
    box.append(el("h3", "", "Your theme beside the original"));
    const pairs = v.files.compare ?? [];
    if (!f.rom && !pairs.length) {
      box.append(el("p", "muted", "Seeing your theme beside the original needs your own copy of the game on this computer. Your agent can bring it in."));
      return box;
    }
    box.append(button("compare", { go: !pairs.length }));
    if (!pairs.length) {
      box.append(el("p", "muted", "Each moment of the game's route shows twice, your theme on the left and the game's own pictures on the right, so you can see what changed and what to ask for."));
      return box;
    }
    moment = Math.max(0, Math.min(pairs.length - 1, moment));
    const fig = el("figure", "jr-pair");
    const heads = el("div", "jr-pair-heads");
    heads.append(el("span", "", "YOUR THEME"), el("span", "", "THE ORIGINAL"));
    const img = el("img");
    img.id = "jr-pair-img";
    const cap = el("figcaption");
    const show = () => {
      const p = pairs[moment];
      img.src = p.src;
      img.alt = `Your theme and the original at ${p.moment}`;
      cap.textContent = `${moment + 1} of ${pairs.length}: ${p.shows || p.moment}${pairs.some((x) => x.rom !== p.rom) ? ` (${p.rom})` : ""}, frame ${p.frame}`;
      sel.value = String(moment);
      prev.disabled = moment === 0;
      next.disabled = moment === pairs.length - 1;
    };
    const go = (d) => { moment = Math.max(0, Math.min(pairs.length - 1, moment + d)); show(); };
    const prev = el("button", "jr-btn", "BEFORE");
    prev.type = "button";
    prev.title = "The moment before";
    prev.addEventListener("click", () => go(-1));
    const next = el("button", "jr-btn", "NEXT");
    next.type = "button";
    next.title = "The moment after";
    next.addEventListener("click", () => go(1));
    const sel = el("select");
    sel.id = "jr-moment";
    sel.setAttribute("aria-label", "The moment");
    pairs.forEach((p, i) => {
      const o = el("option", "", `${i + 1}. ${p.shows || p.moment}`);
      o.value = String(i);
      sel.append(o);
    });
    sel.addEventListener("change", () => { moment = Number(sel.value); show(); });
    const bar = el("div", "jr-pair-bar");
    bar.append(prev, sel, next);
    fig.append(heads, img, cap);
    box.append(bar, fig);
    box.append(el("p", "muted", "These pictures show your own copy of the game: they stay on this computer. Press BESIDE THE ORIGINAL again after a change."));
    show();
    return box;
  }

  // -------------------------------------------------------------------
  // The new game's steps.

  function playLink(label = "PLAY IT") {
    if (!v.files.play) return null;
    const a = el("a", "jr-btn go", label);
    a.href = v.files.play;
    a.target = "_blank";
    a.rel = "noopener";
    a.title = "Your game, in a new tab";
    return a;
  }

  function newBody(id, b) {
    const f = v.answer.facts;
    const from = f.from?.title ?? "a template";
    switch (id) {
      case "N1":
        b.append(el("p", "", `Your game starts from the template ${from}. A template is a game's rules without its pictures, that a new game is built on. Your copy is yours to change; the template stays as it is.`));
        break;
      case "N2": {
        b.append(el("p", "", `Its name is ${name}. It lives in its own folder on this computer:`));
        b.append(el("pre", "jr-said", f.dir));
        break;
      }
      case "N3": {
        if (!f.assets) {
          b.append(el("p", "", "Its pictures and levels aren't ready yet. They are made from your own copy of the original game, or built for a game made here."));
          b.append(say(`Get the pictures and levels of ${name} ready.`));
        }
        b.append(button("build", { go: f.assets }));
        if (f.built_ms && !f.built_current) b.append(el("p", "jr-err", "You changed it since the last build: build it again to play the change."));
        const a = playLink();
        if (a) b.append(a);
        break;
      }
      case "N4": {
        b.append(el("p", "", "Ask your agent for a change to the stage, in your own words. The tools check every change against the game's rules before anything is written, and keep each one so it can be undone."));
        b.append(say("Move the start a little to the right."));
        const pa = playLink();
        if (pa) b.append(pa);
        b.append(button("render"));
        if (v.files.stage) {
          const img = el("img", "jr-stage");
          img.src = v.files.stage;
          img.alt = `The stage of ${name}, each piece coloured by what it is`;
          b.append(img);
          if (!v.files.stage_current) b.append(el("p", "muted", "The stage changed since this picture: press SHOW THE STAGE again."));
        }
        b.append(el("p", "muted", f.edits ? `You changed the stage ${f.edits} time${f.edits === 1 ? "" : "s"}.` : "No changes yet."));
        if (f.no_check) b.append(el("p", "muted", `The last CHECK THE STAGE couldn't check this stage. The tools said: ${f.no_check}`));
        b.append(button("verify"));
        if (f.verify) b.append(el("p", f.verify.ok ? "jr-ok" : "jr-err", `The last check, ${when(f.verify.at_ms)}${f.verify.current ? "" : " (before your last change)"}: ${f.verify.ok ? "the stage follows the rules and can be finished" : lastLine(f.verify.said)}.`));
        if (f.edits) b.append(button("undo"));
        b.append(doneButton("stage", "DONE WITH THE STAGE", "The stage is how you want it: on to its look"));
        break;
      }
      case "N5": {
        b.append(el("p", "", "Give your game pictures of its own with a theme: the same steps as making a theme for any game."));
        for (const t of f.themes ?? []) {
          const c = el("button", "jr-btn go", `CARRY ON WITH THE THEME ${t}`);
          c.type = "button";
          c.addEventListener("click", () => nav.open(`theme/${t}`));
          b.append(c);
        }
        const title = el("input");
        title.id = "jr-look-title";
        title.maxLength = 60;
        title.placeholder = `like ${name} at night`;
        title.value = typed.get("look-title") ?? "";
        title.addEventListener("input", () => typed.set("look-title", title.value));
        const l = el("label", "jr-field");
        l.append(el("span", "", "A name for its theme"), title);
        const go = el("button", "jr-btn go", "MAKE A THEME FOR IT");
        go.type = "button";
        go.addEventListener("click", async () => {
          errors.delete("look");
          try {
            const r = await nav.api("/api/journeys/start", { journey: "theme", game: name, game_kind: "new", title: title.value || `${name} look`, style: "" });
            typed.delete("look-title");
            await nav.open(`theme/${r.name}`);
          } catch (e) {
            errors.set("look", e.message ?? String(e));
            drawn = "";
            await load();
          }
        });
        b.append(l, go);
        if (errors.has("look")) b.append(el("p", "jr-err", errors.get("look")));
        b.append(doneButton("look", f.themes?.length ? "DONE WITH THE LOOK" : "SKIP: KEEP THE TEMPLATE'S PICTURES", "On to playing it"));
        break;
      }
      case "N6": {
        const a = playLink("PLAY IT");
        if (a) b.append(a);
        else b.append(button("build", { go: true }));
        // The share link: gp game share puts the game in the person's own
        // registry here and the page beside it, which gp serve serves at
        // /share/. Nothing is sent anywhere.
        b.append(el("p", "", "Share it: MAKE A SHARE LINK puts your game in your own registry on this computer, with a page that plays it by its name, and gives you the link. Nothing is sent anywhere."));
        b.append(button("share", { go: true }));
        const sh = v.jobs?.share;
        const out = sh?.state === "done" ? sh.json : null;
        if (out?.link) {
          const href = new URL(out.link, location.origin).href;
          const p = el("p", "jr-link");
          p.id = "jr-share-link";
          p.append("Your share link: ");
          const l = el("a", "", href);
          l.href = href;
          l.target = "_blank";
          l.rel = "noopener";
          p.append(l);
          b.append(p);
          const c = el("button", "jr-btn", "COPY THE LINK");
          c.type = "button";
          c.addEventListener("click", async () => {
            try { await navigator.clipboard.writeText(href); c.textContent = "COPIED"; } catch (e) { c.textContent = "COPY IT FROM ABOVE"; }
          });
          b.append(c);
          b.append(el("p", "", out.for_others));
          b.append(el("pre", "jr-said", `the page      ${out.page}\nthe registry  ${out.registry}`));
          if (out.version) b.append(el("p", "muted", `It is ${out.name}, version ${out.version}. Press MAKE A SHARE LINK again after a change to share the new version.`));
        }
        break;
      }
    }
  }

  // -------------------------------------------------------------------

  /** The cards of what waits for the person: the agent's asks (YES, NO)
   * and the pictures, sound changes and sprite edits it proposes
   * (ACCEPT, REJECT). */
  function asks() {
    const d = el("div");
    d.id = "asks";
    d.setAttribute("aria-live", "polite");
    const list = (v.answer.waits_for_yes ?? []).filter((y) => y.id && (!["picture", "sound", "sprite"].includes(y.what) || y.state !== "open" || ((y.picture ?? y.sound ?? y.sprite)?.place ?? 1) === 1));
    for (const y of list) {
      if (y.what === "picture" && y.picture) {
        if (y.state === "open") d.append(pictureCard(y.picture, () => { drawn = ""; void load(); }));
        continue;
      }
      if (y.what === "sound" && y.sound) {
        if (y.state === "open") d.append(soundCard(y.sound, () => { drawn = ""; void load(); }));
        continue;
      }
      if (y.what === "sprite" && y.sprite) {
        if (y.state === "open") d.append(spriteCard(y.sprite, () => { drawn = ""; void load(); }));
        continue;
      }
      if (y.state !== "open" && y.state !== "yes") continue;
      const card = el("div", "ask-card");
      card.dataset.ask = y.id;
      card.append(el("span", "ask-head", y.state === "yes" ? "YOU SAID YES" : "YOUR AGENT ASKS"), el("span", "ask-says", y.says));
      if (y.state === "yes") card.append(el("span", "muted", "Your agent can go ahead, once."));
      else {
        // The answer box, open on the card: its words go with YES or NO.
        const t = el("input", "ask-words");
        t.placeholder = "Say anything about this (goes with YES or NO)";
        t.setAttribute("aria-label", "Your answer on this card");
        t.addEventListener("keydown", (ev) => ev.stopPropagation());
        card.append(t);
        for (const [word, val] of [["YES", "yes"], ["NO", "no"]]) {
          const b = el("button", `ask-${val}`, word);
          b.type = "button";
          b.title = val === "yes" ? "Let your agent do this, once" : "Your agent won't do this";
          b.addEventListener("click", async () => {
            for (const x of card.querySelectorAll("button")) x.disabled = true;
            try { await nav.api(`/api/asks/${encodeURIComponent(y.id)}`, t.value.trim() ? { answer: val, words: t.value.trim() } : { answer: val }); } catch (e) { errors.set("ask", e.message ?? String(e)); }
            drawn = "";
            await load();
          });
          card.append(b);
        }
      }
      d.append(card);
    }
    return d;
  }

  function draw() {
    const a = v.answer;
    const c = v.copy;
    box.textContent = "";
    const head = el("div", "jr-head");
    const back = el("button", "jr-btn", "FIRST SCREEN");
    back.type = "button";
    back.title = "What you want to do, and your copies";
    back.addEventListener("click", nav.home);
    head.append(back);
    box.append(head);
    const kindWords = kind === "theme" ? "Your theme" : "Your new game";
    const from = c.from ? (c.from.kind === "template" ? ` from the template ${c.from.title}` : c.from.kind === "empty" ? " started from nothing" : c.from.kind === "new" ? ` for your new game ${c.from.title}` : ` for ${c.from.title}`) : "";
    box.append(el("h2", "", `${kindWords} ${a.facts.title ?? name}${from}`));
    box.append(el("p", "muted", `Changed ${when(c.touched_ms)}. Step ${a.stage.n} of ${a.stage.of}: ${a.stage.name}.`));
    // Where it stands and the next step, as the game page's status line.
    const line = el("div", "jr-line");
    line.id = "jr-next";
    line.append(el("span", "nx-stands", a.stands));
    const next = el("span", "nx-next");
    next.append(el("span", "nx-who", `NEXT, ${WHO[a.next.who] ?? a.next.who}:`), ` ${a.next.says}`);
    line.append(next);
    if (a.next.act?.do === "start" && BUTTONS[a.next.act.action]) {
      const j = v.jobs?.[a.next.act.action];
      const b = el("button", "jr-btn go", j?.state === "running" ? `${BUTTONS[a.next.act.action][0]}: RUNNING` : BUTTONS[a.next.act.action][0]);
      b.type = "button";
      b.disabled = j?.state === "running";
      b.addEventListener("click", () => void press(a.next.act.action));
      line.append(b);
    }
    for (const r of a.running ?? []) line.append(el("span", "nx-run", `Running: ${r.what}`));
    box.append(line);
    box.append(asks());
    if (errors.has("ask")) box.append(el("p", "jr-err", errors.get("ask")));
    // The steps.
    const ol = el("ol", "jr-steps");
    ol.setAttribute("aria-label", "The steps");
    for (const s of a.steps) {
      const li = el("li");
      li.dataset.step = s.id;
      li.dataset.state = s.state;
      li.dataset.who = s.who;
      const h = el("button", "jr-step-head");
      h.type = "button";
      h.setAttribute("aria-expanded", "false");
      h.append(el("span", "jr-n", `${s.n}.`), el("span", "jr-name", s.name), whoTag(s.who), el("span", "jr-state", s.state === "now" ? "NOW" : s.state === "done" ? "DONE" : "LATER"));
      const body = el("div", "jr-body");
      const open = opened.get(s.id) ?? s.state === "now";
      body.hidden = !open;
      h.setAttribute("aria-expanded", String(open));
      h.addEventListener("click", () => {
        const o = body.hidden;
        opened.set(s.id, o);
        body.hidden = !o;
        h.setAttribute("aria-expanded", String(o));
      });
      if (kind === "theme") themeBody(s.id, body);
      else newBody(s.id, body);
      li.append(h, body);
      ol.append(li);
    }
    box.append(ol);
    // A theme for a new game: the way back to the game.
    if (kind === "theme" && c.from?.kind === "new") {
      const g = el("button", "jr-btn", `BACK TO YOUR GAME ${c.from.name}`);
      g.type = "button";
      g.addEventListener("click", () => nav.open(`new/${c.from.name}`));
      box.append(g);
    }
  }

  void load().then(again);
  return () => { stopped = true; clearTimeout(timer); };
}

function firstLine(t) {
  return String(t ?? "no reason given").split("\n")[0].trim();
}

function lastLine(t) {
  const lines = String(t ?? "").split("\n").map((x) => x.trim()).filter(Boolean);
  return lines[lines.length - 1] ?? "it didn't pass";
}
