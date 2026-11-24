// The model's preview loading in its frame, for the screen page
// (screen.js) and the mechanic page (app.js). Checked without a browser by
// mechanic/model-load.test.mjs.
//
// The preview tells each step of its load ({gp: "loading", step}, its
// mark() in scripts/web/page/preview.js), then {gp: "ready"}, or
// {gp: "failed", message} when its start throws. The time limit runs from
// the last word it sent, not from the start: a load that is slow but
// moving (a theme's large pictures, a server busy with a build or a gate)
// isn't called failed, and one that stops says the last step it reached.
// Before, the whole load had 30 s and the page said only "the model didn't
// load in 30 s" (a screen page, 2026-11-24).

/** Each step the preview tells, as what it had done. */
const STEPS = {
  script: "its script ran",
  instantiated: "it read its files",
  opened: "its module started",
  bundle: "it read the model",
  tested: "it ran its tests",
  board: "it read the board's art",
  "board-sound": "it set up the board's sound",
  "model-loaded": "the model was loaded",
  pictures: "its pictures were loaded",
  looks: "it found its themes",
  roles: "its pictures were placed",
  theme: "the theme was drawn",
  "assets-loaded": "its art was loaded",
  stills: "its second model started",
  "first-frame": "it drew its first frame",
};

/** What the page says when the load stopped for `ms` after `step` (null:
 * before the preview's script said anything). */
export function stoppedWords(step, ms) {
  const s = `${Math.round(ms / 1000)} s`;
  if (step === null) return `the model's page sent nothing for ${s}: its script didn't start`;
  return `the model stopped loading: nothing came for ${s} after ${STEPS[step] ?? step}`;
}

/** Waits for the model's preview. `start()` sets the frame going (its
 * src); returns `{ done, ready, failed, step }`: the page's message
 * listener calls ready(), failed(why) and step(name) as the preview's
 * words come, and `done` resolves on ready and rejects on a failure or
 * `within` ms with no word. */
export function waitForModel(start, { within = 30000, timer = setTimeout, clear = clearTimeout } = {}) {
  let last = null;
  let t = null;
  let over = false;
  let resolve;
  let reject;
  const done = new Promise((y, n) => { resolve = y; reject = n; });
  const end = (f) => {
    if (over) return;
    over = true;
    clear(t);
    f();
  };
  const arm = () => {
    clear(t);
    t = timer(() => end(() => reject(new Error(stoppedWords(last, within)))), within);
  };
  arm();
  try {
    start();
  } catch (e) {
    end(() => reject(e));
  }
  return {
    done,
    ready: () => end(resolve),
    failed: (why) => end(() => reject(new Error(`its preview didn't start: ${why}`))),
    step: (name) => {
      if (over) return;
      last = name;
      arm();
    },
  };
}
