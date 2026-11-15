// The first screen (mechanic/start.html, served at /start/ by
// crates/gp-cli/src/serve/journeys.rs): what the person wants to do, in
// three choices (port a game they own, make a theme for a game, make a new
// game), and "carry on" with each copy they have and each game they are
// porting. A copy opens here at its step (journey.js), at /start/?copy=
// KIND/NAME. Nothing here deletes or publishes.

import { mountJourney, WHO, when } from "./journey.js";

const $ = (id) => document.getElementById(id);
const el = (tag, cls, text) => {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
};

async function api(path, body) {
  const r = await fetch(path, body === undefined ? {} : {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  const v = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(v.error ?? `the page's server answered ${r.status}`);
  return v;
}

let all = null;
let stopJourney = null;

function fromWords(f) {
  if (!f) return "";
  if (f.kind === "template") return `from the template ${f.title}`;
  if (f.kind === "new") return `for your new game ${f.title}`;
  return `for ${f.title}`;
}

/** One line of the copies list: name, where it came from, when it was
 * last touched, where it stands, whose turn, and CARRY ON. */
function copyItem(c) {
  const li = el("li");
  li.dataset.copy = `${c.journey}/${c.name}`;
  const kind = c.journey === "theme" ? "THEME" : "NEW GAME";
  li.append(
    el("span", "cp-kind jr-who who-tools", kind),
    el("span", "cp-name", c.name),
    el("span", "cp-from", fromWords(c.from)),
    el("span", "cp-when", `changed ${when(c.touched_ms)}`),
  );
  const at = el("span", "cp-at");
  const who = el("span", `jr-who who-${c.who}`, WHO[c.who] ?? c.who);
  at.append(`Step ${c.stage.n} of ${c.stage.of}, ${c.stage.name}. Next: `, who, ` ${c.says}`);
  const b = el("button", "jr-btn go", "CARRY ON");
  b.type = "button";
  b.title = `Open ${c.name} at its step`;
  b.addEventListener("click", () => openCopy(`${c.journey}/${c.name}`));
  li.append(at, b);
  return li;
}

/** A game being ported: its page, and where the port stands. */
function gameItem(g) {
  const li = el("li");
  const a = el("a", "", g.name);
  a.href = `/mechanic/${encodeURIComponent(g.name)}/`;
  li.append(el("span", "cp-kind jr-who who-tools", "PORT"), a);
  if (g.port) {
    const at = el("span", "cp-at");
    at.append(`Step ${g.port.stage.n} of ${g.port.stage.of}, ${g.port.stage.name}. Next: `, el("span", `jr-who who-${g.port.who}`, WHO[g.port.who] ?? g.port.who), ` ${g.port.says}`);
    li.append(el("span", "cp-when", g.touched_ms ? `changed ${when(g.touched_ms)}` : ""), at);
  }
  return li;
}

function drawCarry() {
  const copies = $("copies");
  const gl = $("games");
  copies.textContent = "";
  gl.textContent = "";
  for (const c of all.copies) copies.append(copyItem(c));
  for (const g of all.games.filter((g) => g.port)) gl.append(gameItem(g));
  $("carry").hidden = !copies.childElementCount && !gl.childElementCount;
}

// ---------------------------------------------------------------------
// The three choices.

function field(label, input) {
  const f = el("label", "jr-field");
  f.append(el("span", "", label), input);
  return f;
}

function themeForm(box) {
  box.textContent = "";
  box.append(el("h2", "", "Make a theme for a game"));
  box.append(el("p", "", "A theme is a game's pictures drawn again in a style of your own. The game plays the same; only how it looks changes. Your agent makes the pictures, or you bring your own, and the page walks you through checking them and seeing them in the game."));
  const pick = el("select");
  pick.id = "theme-game";
  for (const g of all.games) {
    const o = el("option", "", g.title === g.name ? g.name : `${g.title} (${g.name})`);
    o.value = `game:${g.name}`;
    pick.append(o);
  }
  for (const c of all.copies.filter((c) => c.journey === "new")) {
    const o = el("option", "", `${c.name} (your new game)`);
    o.value = `new:${c.name}`;
    pick.append(o);
  }
  const title = el("input");
  title.id = "theme-title";
  title.maxLength = 60;
  title.placeholder = "like Night Moss";
  const style = el("textarea");
  style.id = "theme-style";
  style.maxLength = 300;
  style.placeholder = "like night time, neon, or paper cutouts";
  if (!pick.options.length) {
    box.append(el("p", "muted", "There's no game here to make a theme for yet. Port a game first, or make a new game."));
    return;
  }
  box.append(field("The game", pick), field("A name for your theme", title), field("The style, in a few words", style));
  const err = el("p", "jr-err");
  const go = el("button", "jr-btn go", "MAKE MY THEME");
  go.type = "button";
  go.id = "theme-go";
  go.addEventListener("click", async () => {
    const [kind, game] = pick.value.split(/:(.*)/s);
    go.disabled = true;
    err.textContent = "";
    try {
      const v = await api("/api/journeys/start", { journey: "theme", game, game_kind: kind, title: title.value, style: style.value });
      await openCopy(`theme/${v.name}`);
    } catch (e) {
      err.textContent = `It wasn't made: ${e.message ?? e}`;
    }
    go.disabled = false;
  });
  box.append(go, err, el("p", "muted", `Your theme's folder goes under ${all.themes_dir}. Making it is free; nothing is sent anywhere.`));
}

function newForm(box) {
  box.textContent = "";
  box.append(el("h2", "", "Make a new game"));
  box.append(el("p", "", "A new game starts from a template: a game's rules without its pictures, that a new game is built on. You get a copy of your own to change. The template stays as it is."));
  const list = el("div", "jr-templates");
  list.setAttribute("role", "radiogroup");
  list.setAttribute("aria-label", "The template");
  const open = all.templates.filter((t) => t.open);
  const shown = open.length ? open : all.templates;
  shown.forEach((t, i) => {
    const l = el("label", "jr-template");
    const r = el("input");
    r.type = "radio";
    r.name = "template";
    r.value = t.name;
    r.checked = i === 0;
    l.append(r, ` ${t.title}`, el("span", "muted", t.kit ? " (its stages can be changed here)" : " (its stages can't be changed here yet)"));
    list.append(l);
  });
  const name = el("input");
  name.id = "new-name";
  name.maxLength = 40;
  name.placeholder = "like my_hop";
  box.append(el("h3", "", "Start from a template"), list, field("A name for your game: letters, digits and _, starting with a letter", name));
  const err = el("p", "jr-err");
  const go = el("button", "jr-btn go", "MAKE MY GAME");
  go.type = "button";
  go.id = "new-go";
  go.addEventListener("click", async () => {
    const template = list.querySelector("input:checked")?.value;
    go.disabled = true;
    err.textContent = "";
    try {
      const v = await api("/api/journeys/start", { journey: "new", template, name: name.value.trim() });
      await openCopy(`new/${v.name}`);
    } catch (e) {
      err.textContent = `It wasn't made: ${e.message ?? e}`;
    }
    go.disabled = false;
  });
  box.append(go, err, el("p", "muted", `Your game's folder goes under ${all.copies_dir}. Copying is free and takes a few seconds.`));
}

function choose(which) {
  for (const b of document.querySelectorAll("#start-choices button")) b.classList.toggle("on", b.dataset.journey === which);
  $("port-start").hidden = which !== "port";
  $("theme-start").hidden = which !== "theme";
  $("new-start").hidden = which !== "new";
  if (which === "theme") themeForm($("theme-start"));
  if (which === "new") newForm($("new-start"));
  $("port-have").hidden = !all.games.some((g) => g.port);
  const box = $(`${which}-start`);
  box?.scrollIntoView({ block: "nearest" });
}

// ---------------------------------------------------------------------
// One copy.

async function openCopy(key) {
  // KIND/NAME is letters, digits, - and _ with one /, kept as it is.
  history.pushState({ copy: key }, "", `/start/?copy=${key}`);
  await show();
}

function home() {
  history.pushState({}, "", "/start/");
  void show();
}

async function show() {
  stopJourney?.();
  stopJourney = null;
  try { all = await api("/api/journeys"); } catch (e) {
    $("title").textContent = `The first screen can't read its server: ${e.message ?? e}`;
    return;
  }
  const key = new URL(location.href).searchParams.get("copy");
  const [kind, name] = (key ?? "").split("/");
  const inJourney = Boolean(kind && name);
  $("first").hidden = inJourney;
  $("theme-start").hidden = true;
  $("new-start").hidden = true;
  $("journey").hidden = !inJourney;
  drawCarry();
  if (inJourney) {
    $("carry").hidden = true;
    $("title").textContent = name;
    stopJourney = mountJourney($("journey"), kind, name, { open: openCopy, home, api });
    window.scrollTo(0, 0);
    return;
  }
  $("title").textContent = "What do you want to do?";
  for (const b of document.querySelectorAll("#start-choices button")) b.classList.remove("on");
  $("port-start").hidden = true;
}

for (const b of document.querySelectorAll("#start-choices button")) {
  b.addEventListener("click", () => choose(b.dataset.journey));
}
$("logo").addEventListener("click", (e) => {
  if (new URL(location.href).searchParams.has("copy")) {
    e.preventDefault();
    home();
  }
});
window.addEventListener("popstate", () => void show());
// The copies list keeps up with the agent while the first screen shows.
setInterval(async () => {
  if (!$("journey").hidden || document.hidden) return;
  try { all = await api("/api/journeys"); drawCarry(); } catch (e) { /* the next look */ }
}, 8000);
void show();
