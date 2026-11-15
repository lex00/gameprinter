// A picture the agent proposes for the person's own theme (issue #4,
// crates/gp-cli/src/theme_picture.rs): the card in the agent's asks on
// every tab (next.js draws it from the status line's waits_for_yes, the
// way it draws a yes card), and the list on THEMES (themes/pane.js). The
// card shows the picture the theme has now beside the new one, each at
// game size and enlarged with its pixels kept square, the agent's
// sentence, ACCEPT, REJECT and a box for what the agent should change.
// An accepted picture can be put back. Answers go to gp serve's
// /api/pictures/ID, and the agent hears them through `next`.

const LINK = "picture-card-css";
/** The largest side of the enlarged pictures, in CSS pixels, so two fit
 * side by side at 360 px. */
const BIG = 120;

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

function css() {
  if (document.getElementById(LINK)) return;
  const l = document.createElement("link");
  l.id = LINK;
  l.rel = "stylesheet";
  l.href = new URL("./picture-card.css", import.meta.url).href;
  document.head.append(l);
}

/** What the person typed in a card's box, by proposal, so a redraw of the
 * cards doesn't lose it. */
const typed = new Map();

/** Tells the other views an answer was given, so they look again. */
function answered() {
  window.dispatchEvent(new CustomEvent("gp-pictures"));
}

/** Sends an answer: "accept", "reject" or "undo". The proposal as stored
 * now, or throws with the server's words. */
export async function answer(id, what, words) {
  const r = await fetch(`/api/pictures/${encodeURIComponent(id)}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ answer: what, words: words || undefined }),
  });
  const v = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(v.error ?? `the page's server answered ${r.status}`);
  typed.delete(id);
  answered();
  return v;
}

/** One side: the picture enlarged, and under it at game size. */
function side(p, which, label) {
  const f = el("figure", "pic-side");
  const [w, h] = p.size ?? p[`${which}_px`] ?? [16, 16];
  const px = p[`${which}_px`] ?? [w, h];
  const src = `/api/pictures/${encodeURIComponent(p.id)}/${which}.png`;
  // Enlarged: a whole number of screen pixels to a game pixel when the
  // picture is small, else shrunk to fit; both keep their pixels square.
  const k = Math.max(w, h) <= BIG ? Math.max(1, Math.floor(BIG / Math.max(w, h))) : BIG / Math.max(w, h);
  const big = el("img", "pic-big");
  big.src = src;
  big.alt = `${label}, enlarged`;
  big.width = Math.round(w * k);
  big.height = Math.round(h * k);
  const small = el("img", "pic-small");
  small.src = src;
  small.alt = `${label}, at game size`;
  small.width = w;
  small.height = h;
  small.title = `At game size: ${w} by ${h} game pixels (the file is ${px[0]} by ${px[1]})`;
  const cap = el("figcaption", "", label);
  f.append(el("div", "pic-frame"), cap);
  f.firstChild.append(big);
  const at = el("div", "pic-at");
  at.append(small, el("span", "muted", "game size"));
  f.append(at);
  return f;
}

/** The two pictures, now and new. */
function pair(p) {
  const d = el("div", "pic-pair");
  d.append(side(p, "old", p.state === "accepted" ? "BEFORE" : "NOW"), side(p, "new", "NEW"));
  return d;
}

function who(p) {
  const title = p.title ?? "your theme";
  return `${title}, ${p.picture}`;
}

/**
 * The card for one proposal, as `next` lists it (`waits_for_yes` with
 * what "picture", its `picture` the proposal). `done` runs after an
 * answer is saved, to draw the cards again.
 */
export function pictureCard(p, done = () => {}) {
  css();
  const card = el("div", "ask-card pic-card");
  card.dataset.picture = p.id;
  card.dataset.state = p.state;
  const head = el("span", "ask-head");
  const says = el("span", "ask-says");
  card.append(head);
  const err = el("span", "pic-err");
  const btn = (text, cls, title, on) => {
    const b = el("button", cls, text);
    b.type = "button";
    b.tabIndex = -1;
    b.title = title;
    b.addEventListener("click", async (e) => {
      e.currentTarget.blur();
      for (const x of card.querySelectorAll("button, input")) x.disabled = true;
      try {
        await on();
        err.textContent = "";
      } catch (x) {
        err.textContent = `It didn't save: ${x.message ?? x}`;
        for (const y of card.querySelectorAll("button, input")) y.disabled = false;
        return;
      }
      done();
    });
    return b;
  };
  if (p.state === "open") {
    head.textContent = "YOUR AGENT PROPOSES A PICTURE";
    if (p.of > 1) card.append(el("span", "pic-count", `${p.place ?? 1} of ${p.of} waiting`));
    says.textContent = `${p.game ? `${p.game}: ` : ""}${who(p)}. ${p.why ?? ""}`;
    card.append(says, pair(p));
    const words = el("input", "pic-words");
    words.type = "text";
    words.placeholder = "tell your agent what to change";
    words.setAttribute("aria-label", "Tell your agent what to change");
    words.maxLength = 2000;
    words.value = typed.get(p.id) ?? "";
    words.addEventListener("input", () => typed.set(p.id, words.value));
    const row = el("div", "pic-row");
    row.append(
      words,
      btn("ACCEPT", "pic-accept ask-yes", "Put the new picture in your theme. You can put the old one back after.", () => answer(p.id, "accept", words.value.trim())),
      btn("REJECT", "pic-reject ask-no", "Keep the picture you have. Your agent hears what you wrote in the box.", () => answer(p.id, "reject", words.value.trim())),
    );
    card.append(row);
  } else {
    const heads = { accepted: "YOU ACCEPTED", rejected: "YOU REJECTED", undone: "YOU PUT THE OLD ONE BACK", stale: "NOT USED" };
    head.textContent = heads[p.state] ?? p.state.toUpperCase();
    const tail = {
      accepted: "It is in your theme now.",
      rejected: "Your theme keeps the picture it had.",
      undone: "Your theme has the picture it had before.",
      stale: "The picture changed in the theme first. Your agent can propose it again.",
    }[p.state] ?? "";
    says.textContent = `${who(p)}. ${tail}${p.words ? ` You said: "${p.words}"` : ""}`;
    card.append(says);
    if (p.state === "accepted") {
      card.append(pair(p));
      card.append(btn("PUT THE OLD ONE BACK", "pic-undo", "Put the picture your theme had before back in it", () => answer(p.id, "undo")));
    }
    if (p.told_ms == null && p.state !== "stale") card.append(el("span", "muted pic-heard", "Your agent hears this when it looks next."));
  }
  card.append(err);
  return card;
}

/**
 * THEMES's list: every open proposal (answered on the card at the top),
 * and the last answered, each accepted one with PUT THE OLD ONE BACK.
 * Returns a section that draws itself again when an answer is given or
 * every few seconds while THEMES shows.
 */
export function picturesSection() {
  css();
  const s = el("section", "th-box pic-list");
  s.id = "th-pictures";
  let drawn = "";
  async function load() {
    let v;
    try {
      const r = await fetch("/api/pictures");
      if (!r.ok) { s.hidden = true; return; }
      v = await r.json();
    } catch (e) { s.hidden = true; return; }
    const key = JSON.stringify([...v.open, ...v.recent].map((p) => [p.id, p.state, p.told_ms]));
    if (key === drawn) return;
    drawn = key;
    s.hidden = false;
    s.textContent = "";
    s.append(el("h3", "", "Pictures your agent proposed"));
    const n = v.open.length;
    s.append(el("p", n ? "" : "muted", n
      ? `${n === 1 ? "1 picture waits" : `${n} pictures wait`} for your answer. ACCEPT or REJECT ${n === 1 ? "it" : "the first"} on the card at the top of the page.`
      : "Nothing waits for your answer. Ask your agent to change a picture of your theme, and it shows here before anything changes."));
    for (const p of v.open) {
      const row = el("div", "pic-item");
      row.dataset.picture = p.id;
      row.append(el("p", "", `${who(p)}: ${p.why ?? ""}`), pair(p));
      s.append(row);
    }
    if (v.recent.length) s.append(el("h4", "", "Answered"));
    for (const p of v.recent.slice(0, 8)) {
      s.append(pictureCard({ ...p, told_ms: p.told_ms ?? 0 }, () => { drawn = ""; void load(); }));
    }
  }
  window.addEventListener("gp-pictures", () => { drawn = ""; void load(); });
  setInterval(() => { if (s.isConnected && !s.closest("[hidden]")) void load(); }, 5000);
  void load();
  return s;
}
