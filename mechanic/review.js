// The review of a clip on the model: the clip played through the original
// and the game's module as `gp clip score` plays it, each frame's two
// pictures kept by the server (crates/gp-cli/src/mechanic/review.rs). It
// opens over the page from a clip's ON THE MODEL button and shows the two
// side by side, the model over the original, or their differences, with
// the frames where the mapped state disagrees marked on the strip under
// them. Esc closes it; while it's open its keys don't reach the game.

const $ = (id) => document.getElementById(id);

const state = {
  open: false,
  clip: 0,
  mechanic: "",
  meta: null,
  at: 0,
  playing: false,
  timer: null,
  cache: new Map(),
};

function pictureUrl(kind, k) {
  const name = `${kind}-${String(k).padStart(5, "0")}.png`;
  return `/api/clips/${state.clip}/review/${name}?mechanic=${encodeURIComponent(state.mechanic)}`;
}

/** A frame's picture, loaded once. */
function picture(kind, k) {
  const key = `${state.clip}/${kind}/${k}`;
  let p = state.cache.get(key);
  if (!p) {
    p = new Promise((resolve, reject) => {
      const img = new Image();
      img.onload = () => resolve(img);
      img.onerror = () => reject(new Error(`frame ${k} has no picture`));
      img.src = pictureUrl(kind, k);
    });
    state.cache.set(key, p);
  }
  return p;
}

function say(text) {
  $("rv2-says").textContent = text;
}

/** The strip under the pictures: a mark for each frame, red where the
 * original's and the model's mapped state disagree. */
function drawStrip() {
  const m = state.meta;
  const c = $("rv2-strip");
  const w = c.clientWidth || 600;
  c.width = w;
  const g = c.getContext("2d");
  g.fillStyle = "#1c1c1c";
  g.fillRect(0, 0, w, c.height);
  const n = m.frames;
  for (let k = 0; k < n; k++) {
    const x = Math.floor((k / n) * w);
    const x2 = Math.max(x + 1, Math.floor(((k + 1) / n) * w));
    g.fillStyle = m.together[k] ? "#00a800" : "#f83800";
    g.fillRect(x, 0, x2 - x, c.height);
  }
  g.fillStyle = "#fcfcfc";
  g.fillRect(Math.floor((state.at / n) * w), 0, 2, c.height);
}

async function draw() {
  const m = state.meta;
  if (!m) return;
  const k = state.at;
  $("rv2-scrub").value = String(k);
  const agree = m.together[k];
  $("rv2-at").textContent = `frame ${k + 1} of ${m.frames} (the original's ${m.original_frames[k] ?? "?"}): ${agree ? "the same" : "they differ"}`;
  $("rv2-at").className = agree ? "same" : "differ";
  drawStrip();
  const layout = $("rv2-layout").value;
  $("rv2-mix-row").hidden = layout !== "top";
  const [co, cm] = [$("rv2-o"), $("rv2-m")];
  cm.hidden = layout !== "side";
  for (const c of [co, cm]) {
    c.width = m.w;
    c.height = m.h;
  }
  if (!m.have[k]) {
    for (const c of [co, cm]) c.getContext("2d").clearRect(0, 0, m.w, m.h);
    say("No picture on this frame: the model has none yet for the frame the original shows.");
    return;
  }
  let o, md;
  try {
    [o, md] = await Promise.all([picture("o", k), picture("m", k)]);
  } catch (e) {
    say(String(e.message ?? e));
    return;
  }
  if (state.at !== k) return;
  say("");
  const go = co.getContext("2d");
  if (layout === "side") {
    go.drawImage(o, 0, 0);
    cm.getContext("2d").drawImage(md, 0, 0);
  } else if (layout === "top") {
    go.globalAlpha = 1;
    go.drawImage(o, 0, 0);
    go.globalAlpha = Number($("rv2-mix").value) / 100;
    go.drawImage(md, 0, 0);
    go.globalAlpha = 1;
  } else {
    // The differences: the original dimmed where the two agree, magenta
    // where a pixel differs, inside the part both draw (meta.visible, as
    // the screen score compares); outside it the original only, dimmed.
    go.drawImage(o, 0, 0);
    const a = go.getImageData(0, 0, m.w, m.h);
    go.drawImage(md, 0, 0);
    const b = go.getImageData(0, 0, m.w, m.h);
    const [vl, vt, vw, vh] = m.visible ?? [0, 0, m.w, m.h];
    let diff = 0;
    for (let i = 0; i < a.data.length; i += 4) {
      const x = (i / 4) % m.w;
      const y = Math.floor(i / 4 / m.w);
      const inside = x >= vl && x < vl + vw && y >= vt && y < vt + vh;
      const same = !inside || (a.data[i] === b.data[i] && a.data[i + 1] === b.data[i + 1] && a.data[i + 2] === b.data[i + 2]);
      if (same) {
        a.data[i] >>= 2;
        a.data[i + 1] >>= 2;
        a.data[i + 2] >>= 2;
      } else {
        diff++;
        a.data[i] = 255;
        a.data[i + 1] = 0;
        a.data[i + 2] = 255;
      }
    }
    go.putImageData(a, 0, 0);
    say(diff ? `${diff} pixels differ` : "every pixel the same");
  }
  // The next few frames load while this one shows.
  for (let j = k + 1; j < Math.min(m.frames, k + 8); j++) if (m.have[j]) void picture("o", j).catch(() => {}), void picture("m", j).catch(() => {});
}

function go(k) {
  const m = state.meta;
  if (!m) return;
  state.at = Math.max(0, Math.min(m.frames - 1, k));
  void draw();
}

function setPlaying(on) {
  state.playing = on;
  $("rv2-play").textContent = on ? "PAUSE" : "PLAY";
  clearInterval(state.timer);
  if (on) {
    state.timer = setInterval(() => {
      if (state.at >= state.meta.frames - 1) return setPlaying(false);
      go(state.at + 1);
    }, 1000 / 30);
  }
}

/** The next frame where they differ, after the one shown. */
function nextDiffer(dir) {
  const m = state.meta;
  for (let k = state.at + dir; k >= 0 && k < m.frames; k += dir) if (!m.together[k]) return go(k);
  say(dir > 0 ? "No difference after this frame." : "No difference before this frame.");
}

export async function openReview(clip, mechanic) {
  Object.assign(state, { open: true, clip, mechanic, meta: null, at: 0 });
  setPlaying(false);
  $("rv2").hidden = false;
  $("rv2-title").textContent = `Clip ${clip} on the model`;
  $("rv2-body").hidden = true;
  say("Playing the clip on the original and on the model, as the score does. A long clip takes a while the first time.");
  let r;
  try {
    r = await fetch(`/api/clips/${clip}/review.json?mechanic=${encodeURIComponent(mechanic)}`);
  } catch (e) {
    say(`The review couldn't be asked for: ${e.message ?? e}`);
    return;
  }
  if (!state.open || state.clip !== clip) return;
  if (!r.ok) {
    const t = await r.text();
    let why = t;
    try {
      why = JSON.parse(t).error ?? t;
    } catch {}
    say(`No review: ${why}`);
    return;
  }
  const m = await r.json();
  state.meta = m;
  $("rv2-title").textContent = `Clip ${clip} on the model: ${m.together_count} of ${m.frames} frames the same`;
  $("rv2-scrub").max = String(m.frames - 1);
  $("rv2-body").hidden = false;
  const first = m.together.indexOf(false);
  go(first >= 0 ? first : 0);
}

function close() {
  state.open = false;
  setPlaying(false);
  $("rv2").hidden = true;
}

function setup() {
  $("rv2-close").addEventListener("click", close);
  $("rv2-play").addEventListener("click", () => setPlaying(!state.playing));
  $("rv2-back").addEventListener("click", () => go(state.at - 1));
  $("rv2-fwd").addEventListener("click", () => go(state.at + 1));
  $("rv2-prev-diff").addEventListener("click", () => nextDiffer(-1));
  $("rv2-next-diff").addEventListener("click", () => nextDiffer(1));
  $("rv2-scrub").addEventListener("input", (e) => go(Number(e.target.value)));
  $("rv2-layout").addEventListener("change", () => void draw());
  $("rv2-mix").addEventListener("input", () => void draw());
  $("rv2-strip").addEventListener("click", (e) => {
    const c = e.currentTarget;
    go(Math.floor((e.offsetX / c.clientWidth) * state.meta.frames));
  });
  // While the review is open its keys are its own, not the game's.
  window.addEventListener(
    "keydown",
    (e) => {
      if (!state.open) return;
      const keys = { Escape: close, ArrowLeft: () => go(state.at - 1), ArrowRight: () => go(state.at + 1), " ": () => setPlaying(!state.playing) };
      const f = keys[e.key];
      if (f && !(e.target instanceof HTMLSelectElement)) f();
      e.stopImmediatePropagation();
      if (f) e.preventDefault();
    },
    true,
  );
  window.addEventListener("keyup", (e) => state.open && e.stopImmediatePropagation(), true);
  // A clip's ON THE MODEL button (app.js) asks for it.
  window.addEventListener("gp-review", (e) => void openReview(e.detail.clip, e.detail.mechanic));
}

setup();
