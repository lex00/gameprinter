// The model's preview loading in its frame, for the screen page
// (screen.js). Checked without a browser by
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

/** What the page says when the frame loaded again and again for `ms` and
 * the preview's script never ran in it. */
export function neverRanWords(ms) {
  return `the model's page loaded but its script never ran (tried again for ${Math.round(ms / 1000)} s): preview.js or a file it imports didn't load`;
}

/** Waits for the model's preview. `start()` sets the frame going (its
 * src); returns `{ done, ready, failed, step, loaded }`: the page's
 * message listener calls ready(), failed(why) and step(name) as the
 * preview's words come, the frame's load event calls loaded(ran) with
 * whether the preview's script ran in it, and `done` resolves on ready and
 * rejects on a failure or `within` ms with no word.
 *
 * A frame that loads before the preview has said anything and holds no
 * running preview (the browser's page for a server it couldn't reach, the
 * server's refusal while it moves onto a new build, or a script file that
 * didn't come) is started again every `again` ms, for `retry` ms from the
 * first start; so is one that said nothing in `within` ms. The preview
 * says its first word only once every file it imports has loaded, so
 * until then the page can't tell a slow server from one that is away.
 * Before, a frame was loaded once, and one loaded while gp serve moved
 * onto a new build said "sent nothing for 30 s" (a screen page,
 * 2026-11-27). */
export function waitForModel(start, { within = 30000, again = 2000, retry = 60000, timer = setTimeout, clear = clearTimeout, now = Date.now } = {}) {
  let last = null;
  let t = null;
  let over = false;
  let waiting = false;
  const first = now();
  let resolve;
  let reject;
  const done = new Promise((y, n) => { resolve = y; reject = n; });
  const end = (f) => {
    if (over) return;
    over = true;
    clear(t);
    f();
  };
  const go = () => {
    try {
      start();
    } catch (e) {
      end(() => reject(e));
    }
  };
  // Starts the frame again after `again` ms, if there is time left.
  const restart = () => {
    if (now() + again - first > retry) return false;
    clear(t);
    waiting = true;
    t = timer(() => {
      waiting = false;
      if (over || last !== null) return;
      arm();
      go();
    }, again);
    return true;
  };
  const arm = () => {
    clear(t);
    t = timer(() => {
      if (last === null && restart()) return;
      end(() => reject(new Error(stoppedWords(last, within))));
    }, within);
  };
  arm();
  go();
  return {
    done,
    ready: () => end(resolve),
    failed: (why) => end(() => reject(new Error(`its preview didn't start: ${why}`))),
    step: (name) => {
      if (over) return;
      last = name;
      waiting = false;
      arm();
    },
    loaded: (ran) => {
      if (over || ran || last !== null || waiting) return;
      if (restart()) return;
      end(() => reject(new Error(neverRanWords(retry))));
    },
  };
}
