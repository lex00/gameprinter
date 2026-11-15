// A sprite edit the agent proposes for the person's copy with
// `assets_edit` (issue #4, crates/gp-mcp/src/sprite_proposal.rs): an NES
// copy's tiles or an arcade copy's board sprites. The card is in the
// agent's asks on every tab, drawn by next.js from the status line's
// waits_for_yes as the picture card is (picture-card.js, whose look it
// shares). It shows the edit's own pictures: the sprite before and after,
// close up (previews/edit-SEQ.png), and the recorded frames before and
// after when there are any (edit-SEQ-context.png), each with its pixels
// kept square; the agent's sentence, ACCEPT, REJECT and a box for what
// the agent should change. An accepted edit can be put back. Answers go
// to gp serve's /api/sprites/ID, and the agent hears them through `next`
// and reads them with assets_history.

const LINK = "picture-card-css";
/** The widest the pictures are drawn, in CSS pixels, so they fit at 360 px. */
const WIDE = 320;

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

/** What the person typed in a card's box, by proposal. */
const typed = new Map();

/** Sends an answer: "accept", "reject" or "undo". The proposal as shown
 * now, or throws with the server's words. */
export async function answerSprite(id, what, words) {
  const r = await fetch(`/api/sprites/${encodeURIComponent(id)}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ answer: what, words: words || undefined }),
  });
  const v = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(v.error ?? `the page's server answered ${r.status}`);
  typed.delete(id);
  window.dispatchEvent(new CustomEvent("gp-sprites"));
  return v;
}

/** One of the edit's pictures, at a whole number of screen pixels to a
 * picture pixel when it fits, else shrunk to fit; square pixels both ways. */
function shot(p, which, label) {
  const f = el("figure", "pic-side");
  const img = el("img", "pic-big");
  img.alt = label;
  img.src = `/api/sprites/${encodeURIComponent(p.id)}/${which}.png`;
  img.addEventListener("load", () => {
    const w = img.naturalWidth;
    const h = img.naturalHeight;
    const k = w <= WIDE ? Math.max(1, Math.floor(WIDE / w)) : WIDE / w;
    img.width = Math.round(w * k);
    img.height = Math.round(h * k);
  });
  const frame = el("div", "pic-frame");
  frame.append(img);
  f.append(frame, el("figcaption", "", label));
  return f;
}

/** The pictures the edit has: close up, and in the recorded frames. */
function pictures(p) {
  const d = el("div", "pic-pair spr-pair");
  if (p.preview) d.append(shot(p, "edit", p.state === "accepted" ? "BEFORE AND AFTER" : "NOW AND NEW"));
  if (p.context) d.append(shot(p, "context", "IN THE GAME'S FRAMES"));
  if (!p.preview && !p.context) d.append(el("span", "muted", "This edit has no pictures to show. Ask your agent to propose it again."));
  return d;
}

function what(p) {
  const s = p.subject ? `the ${p.subject}` : "a sprite";
  return `${s} in ${p.copy ?? "your game"}`;
}

/**
 * The card for one proposal, as `next` lists it (`waits_for_yes` with
 * what "sprite", its `sprite` the proposal). `done` runs after an answer
 * is saved, to draw the cards again.
 */
export function spriteCard(p, done = () => {}) {
  css();
  const card = el("div", "ask-card pic-card spr-card");
  card.dataset.sprite = p.id;
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
    head.textContent = "YOUR AGENT PROPOSES A SPRITE CHANGE";
    if (p.of > 1) card.append(el("span", "pic-count", `${p.place ?? 1} of ${p.of} waiting`));
    says.textContent = `A change to ${what(p)}.${p.why ? ` ${p.why}` : ""}`;
    card.append(says, pictures(p));
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
      btn("ACCEPT", "pic-accept ask-yes", "Put the change in your game. You can put the old sprite back after.", () => answerSprite(p.id, "accept", words.value.trim())),
      btn("REJECT", "pic-reject ask-no", "Keep the sprite you have. Your agent hears what you wrote in the box.", () => answerSprite(p.id, "reject", words.value.trim())),
    );
    card.append(row);
  } else {
    const heads = { accepted: "YOU ACCEPTED", rejected: "YOU REJECTED", undone: "YOU PUT THE OLD ONE BACK" };
    head.textContent = heads[p.state] ?? String(p.state ?? "").toUpperCase();
    const tail = {
      accepted: "It is in your game now.",
      rejected: "Your game keeps the sprite it had.",
      undone: "Your game has the sprite it had before.",
    }[p.state] ?? "";
    says.textContent = `The change to ${what(p)}. ${tail}${p.words ? ` You said: "${p.words}"` : ""}`;
    card.append(says);
    if (p.state === "accepted") {
      card.append(pictures(p));
      card.append(btn("PUT THE OLD ONE BACK", "pic-undo", "Put the sprite your game had before back", () => answerSprite(p.id, "undo")));
    }
    if (p.told_ms == null) card.append(el("span", "muted pic-heard", "Your agent hears this when it looks next."));
  }
  card.append(err);
  return card;
}
