// The public page's "What is this?" button (docs/web.md, "What is this?"),
// like the mechanic page's (editor/src/PointTell.tsx) but answered here,
// in this browser, from the game's own package (explain.js): no server,
// no agent. Pressing it holds the game; a click or a tap on the game's
// picture names what is drawn there and says what the package says of it,
// with a box around it; "Play on" (or the button again, or Esc) lets the
// game go on.
//
// Everything of it is here and in what-is-this.css, so page.js only calls
// `whatIsThis` once. It reads the game's frame (same origin): its canvas
// for where the spot is, and `__gp.spot` and `__gp.game` (preview.js) for
// what the model draws there and which model plays.

import { explainSpot, wantedFile } from "./explain.js";

// How far a body's picture may be from the spot, in game pixels, and still
// be what was pointed at: a finger is wider than a mouse's point.
const SLOP = { touch: 6, pen: 3, mouse: 2 };

/** Sets the button up. `frame` is the game's iframe; `installing` the
 * page's promise of the installed game ({ game: its record }). */
export function whatIsThis({ frame, installing }) {
  const link = document.createElement("link");
  link.rel = "stylesheet";
  link.href = "what-is-this.css";
  document.head.append(link);

  const button = Object.assign(document.createElement("button"), { id: "what", type: "button", className: "tube quiet violet", textContent: "What is this?" });
  button.setAttribute("aria-pressed", "false");
  button.title = "Pause the game and point at something to hear what it is";
  button.hidden = true;
  // With the page's lesser buttons, before Unload ROM.
  const back = document.getElementById("back");
  if (back) back.before(button);
  else (document.getElementById("share") ?? document.getElementById("settings"))?.after(button);

  const layer = Object.assign(document.createElement("div"), { id: "what-layer" });
  layer.hidden = true;
  layer.setAttribute("aria-hidden", "true");
  const mark = Object.assign(document.createElement("div"), { className: "what-box" });
  mark.hidden = true;
  layer.append(mark);
  document.body.append(layer);

  const panel = Object.assign(document.createElement("section"), { id: "what-panel" });
  panel.setAttribute("aria-label", "What is this?");
  panel.setAttribute("aria-live", "polite");
  panel.hidden = true;
  // After the game's box, not in it: under the game on a narrow page, and
  // in the left wing's place on a wide one (what-is-this.css).
  (frame.closest("main") ?? frame).after(panel);

  let on = false;
  let box = null;
  let files = null;

  const gp = () => {
    try {
      return frame.contentWindow?.__gp ?? null;
    } catch {
      return null;
    }
  };
  const canvas = () => {
    try {
      return frame.contentDocument?.getElementById("game") ?? null;
    } catch {
      return null;
    }
  };
  // The game's picture on this page: the frame's place plus the canvas's
  // place in the frame.
  const pictureRect = () => {
    const c = canvas();
    if (!c) return null;
    const f = frame.getBoundingClientRect();
    const r = c.getBoundingClientRect();
    return { left: f.left + r.left, top: f.top + r.top, width: r.width, height: r.height };
  };
  const place = () => {
    if (!on) return;
    const f = frame.getBoundingClientRect();
    Object.assign(layer.style, { left: `${f.left}px`, top: `${f.top}px`, width: `${f.width}px`, height: `${f.height}px` });
    const r = pictureRect();
    mark.hidden = !box || !r;
    if (box && r) {
      Object.assign(mark.style, {
        left: `${r.left - f.left + box[0] * r.width - 2}px`,
        top: `${r.top - f.top + box[1] * r.height - 2}px`,
        width: `${box[2] * r.width + 4}px`,
        height: `${box[3] * r.height + 4}px`,
      });
    }
  };
  const hold = (h) => frame.contentWindow?.postMessage({ gp: "hold", on: h }, location.origin);

  const show = (head, paras, extra = []) => {
    const h = Object.assign(document.createElement("h2"), { textContent: head });
    const ps = paras.map((t) => Object.assign(document.createElement("p"), { textContent: t }));
    const done = Object.assign(document.createElement("button"), { type: "button", className: "tube green", textContent: "Play on" });
    done.dataset.what = "done";
    done.addEventListener("click", () => stop());
    panel.replaceChildren(h, ...ps, ...extra, done);
  };
  const hint = () =>
    show("What is this?", ["The game is paused. Click or tap something in the game's picture: a character, an enemy, a number, the stage."]);

  function start() {
    if (on || !gp()) return;
    on = true;
    box = null;
    button.setAttribute("aria-pressed", "true");
    document.querySelector("main")?.classList.add("explaining");
    layer.hidden = false;
    panel.hidden = false;
    hold(true);
    hint();
    requestAnimationFrame(place);
  }
  function stop() {
    if (!on) return;
    on = false;
    box = null;
    button.setAttribute("aria-pressed", "false");
    document.querySelector("main")?.classList.remove("explaining");
    layer.hidden = true;
    mark.hidden = true;
    panel.hidden = true;
    panel.replaceChildren();
    hold(false);
    try {
      frame.contentWindow?.focus();
    } catch {
      // Not loaded.
    }
  }

  // The package's files the answer reads (model, mapping, hooks' source),
  // read once from this browser's store as text.
  const readFiles = async () => {
    if (files) return files;
    const { game } = await installing;
    const x = await import("./game/install.js");
    const got = await x.filesOf({ ...game, files: game.files.filter((f) => wantedFile(f.path)) }, "");
    const text = new TextDecoder();
    files = new Map([...got].map(([k, v]) => [k, text.decode(v)]));
    return files;
  };

  async function ask(e) {
    const r = pictureRect();
    const g = gp();
    if (!r || !g) return;
    const fx = (e.clientX - r.left) / r.width;
    const fy = (e.clientY - r.top) / r.height;
    if (fx < 0 || fy < 0 || fx > 1 || fy > 1) {
      box = null;
      place();
      show("What is this?", ["That is outside the game's picture. Click or tap inside it."]);
      return;
    }
    if (typeof g.spot !== "function") {
      show("What is this?", ["This game can't explain itself yet: its player is older than this button."]);
      return;
    }
    let a;
    try {
      const spot = g.spot(fx, fy, SLOP[e.pointerType] ?? SLOP.mouse);
      a = explainSpot(spot, await readFiles(), g.game);
    } catch (err) {
      a = { can: false, why: `This game can't explain itself yet: ${err.message ?? err}.` };
    }
    box = a.can ? a.box : null;
    place();
    if (!a.can) {
      show("What is this?", [a.why]);
      panel.dataset.answer = "cannot";
      return;
    }
    const extra = [];
    for (const m of a.more ?? []) {
      const d = document.createElement("details");
      d.append(Object.assign(document.createElement("summary"), { textContent: m.head }));
      for (const t of m.text.split("\n\n")) d.append(Object.assign(document.createElement("p"), { textContent: t }));
      extra.push(d);
    }
    if (a.note) extra.push(Object.assign(document.createElement("p"), { className: "what-note", textContent: a.note }));
    show(a.title, a.lines, extra);
    panel.dataset.answer = a.kind ?? "place";
  }

  layer.addEventListener("pointerup", (e) => {
    e.preventDefault();
    void ask(e);
  });
  button.addEventListener("click", () => (on ? stop() : start()));
  addEventListener("keydown", (e) => {
    if (on && e.key === "Escape") stop();
  });
  // The layer is fixed over the frame: it follows the frame as the page
  // scrolls (a phone's page scrolls to the answer) or changes size.
  for (const ev of ["resize", "scroll"]) addEventListener(ev, () => requestAnimationFrame(place), true);
  new ResizeObserver(() => place()).observe(frame);
  // Shown while a game plays in the frame; a new game, or none, ends a
  // question.
  const seen = () => {
    const playing = !frame.hidden && !!frame.getAttribute("src");
    button.hidden = !playing;
    if (!playing) stop();
  };
  new MutationObserver(() => {
    if (on) stop();
    seen();
  }).observe(frame, { attributes: true, attributeFilter: ["src", "hidden"] });
  seen();
  return { start, stop };
}
