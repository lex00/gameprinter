// A sound change the agent proposes for the person's copy (issue #4,
// crates/gp-mcp/src/sound_proposal.rs): the card in the agent's asks on
// every tab, drawn by next.js from the status line's waits_for_yes as the
// picture card is (picture-card.js, whose look it shares). The card plays
// the song as it sounds now and with the change, says the agent's
// sentence, and has ACCEPT, REJECT and a box for what the agent should
// change. An accepted change can be put back. Answers go to gp serve's
// /api/sounds/ID, and the agent hears them through `next`. A proposal of
// kind "role" (issue #30) is a sound for one role of a large-art theme:
// the card plays the role's sound as it is and the new one, or says there
// is none yet when the theme has no file there.

const LINK = "picture-card-css";

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

/** Stops every clip a sound card plays. */
function hush() {
  for (const a of document.querySelectorAll(".snd-card audio")) a.pause();
}

/** Sends an answer: "accept", "reject" or "undo". The proposal as stored
 * now, or throws with the server's words. */
export async function answerSound(id, what, words) {
  hush();
  const r = await fetch(`/api/sounds/${encodeURIComponent(id)}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ answer: what, words: words || undefined }),
  });
  const v = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(v.error ?? `the page's server answered ${r.status}`);
  typed.delete(id);
  window.dispatchEvent(new CustomEvent("gp-sounds"));
  return v;
}

/** One clip's button: plays it from the start, or stops it. */
function player(p, which, label) {
  if (which === "old" && p.kind === "role" && p.old_sha == null) {
    const b = el("button", "snd-play snd-play-old snd-none", p.state === "open" ? "NO SOUND THERE YET" : "THERE WAS NONE BEFORE");
    b.type = "button";
    b.tabIndex = -1;
    b.disabled = true;
    b.title = p.take > 1 ? "This take is new: the role plays its other takes now" : "The theme has no sound for this role yet";
    const f = el("span", "snd-side");
    f.append(b);
    return f;
  }
  const audio = el("audio", `snd-${which}`);
  audio.preload = "none";
  audio.src = `/api/sounds/${encodeURIComponent(p.id)}/${which}.wav`;
  const b = el("button", `snd-play snd-play-${which}`, label);
  b.type = "button";
  b.tabIndex = -1;
  b.title = which === "old" ? "Play the sound as it is in your game now" : "Play the sound with your agent's change";
  const show = () => { b.textContent = audio.paused ? label : "STOP"; b.classList.toggle("on", !audio.paused); };
  audio.addEventListener("play", show);
  audio.addEventListener("pause", show);
  audio.addEventListener("ended", show);
  b.addEventListener("click", (e) => {
    e.currentTarget.blur();
    if (!audio.paused) { audio.pause(); return; }
    hush();
    audio.currentTime = 0;
    audio.play().catch(() => { b.textContent = "IT DIDN'T PLAY"; });
  });
  const f = el("span", "snd-side");
  f.append(b, audio);
  return f;
}

/** The two players, now and new. */
function pair(p) {
  const d = el("div", "pic-pair snd-pair");
  d.append(
    player(p, "old", p.state === "accepted" ? "PLAY THE OLD ONE" : "PLAY WHAT YOU HAVE"),
    player(p, "new", "PLAY THE NEW ONE"),
  );
  if (p.kind === "role") {
    const takes = p.take > 1 ? ` It is take ${p.take}: each time the sound plays, the game plays the next take.` : "";
    d.append(el("span", "muted snd-about", `Each plays the whole sound (the new one is ${(p.seconds ?? 0).toFixed(1)} seconds).${takes}`));
    return d;
  }
  const secs = Math.round(p.seconds ?? 8);
  d.append(el("span", "muted snd-about", `Each plays the first ${secs} seconds of ${p.clip_song ? `the song "${p.clip_song}"` : "the song"}.`));
  return d;
}

/** The game by the person's name for it (the copy's), and the song. */
function who(p) {
  if (p.kind === "role") return `${p.title || p.copy || "your theme"}, the sound "${p.role}"${p.take > 1 ? ` (take ${p.take})` : ""}`;
  const title = p.copy || p.title || "your game";
  return p.clip_song ? `${title}, the song "${p.clip_song}"` : title;
}

/**
 * The card for one proposal, as `next` lists it (`waits_for_yes` with
 * what "sound", its `sound` the proposal). `done` runs after an answer
 * is saved, to draw the cards again.
 */
export function soundCard(p, done = () => {}) {
  css();
  const card = el("div", "ask-card pic-card snd-card");
  card.dataset.sound = p.id;
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
    head.textContent = "YOUR AGENT PROPOSES A NEW SOUND";
    if (p.of > 1) card.append(el("span", "pic-count", `${p.place ?? 1} of ${p.of} waiting`));
    says.textContent = `${who(p)}. ${p.why ?? ""}`;
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
      btn("ACCEPT", "pic-accept ask-yes", "Put the new sound in your game. You can put the old one back after.", () => answerSound(p.id, "accept", words.value.trim())),
      btn("REJECT", "pic-reject ask-no", "Keep the sound you have. Your agent hears what you wrote in the box.", () => answerSound(p.id, "reject", words.value.trim())),
    );
    card.append(row);
  } else {
    const heads = { accepted: "YOU ACCEPTED", rejected: "YOU REJECTED", undone: "YOU PUT THE OLD ONE BACK", stale: "NOT USED" };
    head.textContent = heads[p.state] ?? p.state.toUpperCase();
    const tail = {
      accepted: "It is in your game now.",
      rejected: "Your game keeps the sound it had.",
      undone: "Your game has the sound it had before.",
      stale: "The sound changed in your game first. Your agent can propose it again.",
    }[p.state] ?? "";
    says.textContent = `${who(p)}. ${tail}${p.words ? ` You said: "${p.words}"` : ""}`;
    card.append(says);
    if (p.state === "accepted") {
      card.append(pair(p));
      card.append(btn("PUT THE OLD ONE BACK", "pic-undo", "Put the sound your game had before back in it", () => answerSound(p.id, "undo")));
    }
    if (p.told_ms == null && p.state !== "stale") card.append(el("span", "muted pic-heard", "Your agent hears this when it looks next."));
  }
  card.append(err);
  return card;
}
