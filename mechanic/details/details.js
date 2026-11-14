// The old inspector's views on the one page (wave 8). gp serve's editor
// build draws them (/index.html?embed=VIEW&run=GAME, editor/src/Embed.tsx)
// in the page's colors, and this file puts each where it belongs:
//
// - REFINE, under the background work: THE CLIP MEASURED, a clip or a
//   recording of the game against the rebuilt game (the lockstep panel
//   and its curves, the moments, the scenario ladder, pause, point and
//   tell).
// - DETAILS in the header, on every tab: a drawer with the trace
//   inspector (a recording's RAM, OAM, nametables, tiles, events,
//   coverage, box selection and cards, annotations, compare two).
//
// Each view loads the first time it shows. With no editor build (`just
// editor`) or on a server of the page's own, each says so instead.

const $ = (id) => document.getElementById(id);

/** Starts both. `page` is app.js's (shell.js passes it): game(). */
export function startDetails(page) {
  const css = document.createElement("link");
  css.rel = "stylesheet";
  css.href = "details/details.css";
  document.head.append(css);
  const ready = editorBuilt();
  const game = () => page.game();
  drawer(game, ready);
  measured(game, ready);
}

/** Whether gp serve serves the editor's build here: its index.html. */
async function editorBuilt() {
  try {
    const r = await fetch("/index.html", { method: "GET" });
    const kind = r.headers.get("content-type") ?? "";
    return r.ok && kind.includes("text/html");
  } catch (e) {
    return false;
  }
}

/** The page's game once app.js knows it. */
async function gameKnown(game) {
  for (let i = 0; i < 600 && !game(); i++) await new Promise((r) => setTimeout(r, 100));
  return game();
}

/** An iframe of one embedded view that grows to its content's height. */
function frame(view, run, title, min) {
  const f = document.createElement("iframe");
  f.className = "details-frame";
  f.title = title;
  f.dataset.view = view;
  f.src = `/index.html?embed=${encodeURIComponent(view)}&run=${encodeURIComponent(run)}`;
  f.style.height = `${min}px`;
  const fit = () => {
    try {
      const h = f.contentDocument?.documentElement?.scrollHeight ?? 0;
      if (h > 0) f.style.height = `${Math.max(min, h + 4)}px`;
    } catch (e) {
      /* not ours to read */
    }
  };
  setInterval(fit, 700);
  return f;
}

function line(parent, text, cls) {
  const p = document.createElement("p");
  if (cls) p.className = cls;
  p.textContent = text;
  parent.append(p);
  return p;
}

const NO_EDITOR =
  "These views need the editor's build, which isn't on this computer yet. Ask your agent to build the editor, then reload the page.";

/** The header's DETAILS button and its drawer of the trace inspector. */
function drawer(game, ready) {
  const button = document.createElement("button");
  button.id = "details-toggle";
  button.type = "button";
  button.tabIndex = -1;
  button.textContent = "DETAILS";
  button.title = "The trace inspector: a recording's RAM, OAM, nametables, tiles, events and coverage, frame by frame";
  button.setAttribute("aria-pressed", "false");
  const state = $("state");
  state?.parentElement?.insertBefore(button, state);

  const panel = document.createElement("aside");
  panel.id = "details-drawer";
  panel.hidden = true;
  panel.setAttribute("aria-label", "Details: the trace inspector");
  const head = document.createElement("div");
  head.className = "details-head";
  const h = document.createElement("h2");
  h.textContent = "Details: the trace inspector";
  const close = document.createElement("button");
  close.type = "button";
  close.textContent = "CLOSE";
  close.title = "Close the details";
  head.append(h, close);
  const body = document.createElement("div");
  body.className = "details-body";
  panel.append(head, body);
  document.body.append(panel);

  let loaded = false;
  const set = async (on) => {
    panel.hidden = !on;
    document.body.classList.toggle("details-open", on);
    button.setAttribute("aria-pressed", String(on));
    const u = new URL(location.href);
    if (on) u.searchParams.set("details", "1");
    else u.searchParams.delete("details");
    history.replaceState(null, "", u);
    if (!on || loaded) return;
    loaded = true;
    const run = await gameKnown(game);
    if (!(await ready) || !run) {
      line(body, NO_EDITOR, "muted");
      return;
    }
    line(body, "Pick a recording of the game (your agent can make one), move to a frame, and each view draws it. A second recording compares the two.", "muted");
    body.append(frame("inspector", run, "the trace inspector", 480));
  };
  button.addEventListener("click", (e) => {
    void set(panel.hidden);
    e.currentTarget.blur();
  });
  close.addEventListener("click", () => void set(false));
  if (new URLSearchParams(location.search).get("details") === "1") void set(true);
}

/** REFINE's THE CLIP MEASURED, under the background work; loads when
 * REFINE first shows. */
function measured(game, ready) {
  const pane = $("pane-work");
  if (!pane) return;
  const sec = document.createElement("section");
  sec.id = "measured";
  sec.setAttribute("aria-label", "A clip measured against the rebuilt game");
  const h = document.createElement("h2");
  h.textContent = "The clip measured";
  sec.append(h);
  line(
    sec,
    "A clip of the game (or a recording of its ROM) on the original and on the rebuilt game from the same moment: where the two part, measure by measure, the game's own moments, the scenario ladder, and pause to point at the screen and tell your agent what is wrong.",
    "muted",
  );
  pane.append(sec);
  let loaded = false;
  const load = async () => {
    if (loaded || pane.hidden) return;
    loaded = true;
    const run = await gameKnown(game);
    if (!(await ready) || !run) {
      line(sec, NO_EDITOR, "muted");
      return;
    }
    sec.append(frame("game", run, "a clip measured against the rebuilt game", 640));
  };
  new MutationObserver(() => void load()).observe(pane, { attributes: true, attributeFilter: ["hidden"] });
  void load();
}
