// The first screen's calls to gp serve (start.js, journey.js, tabs.js),
// through a server that moves onto a newly published gp in place
// (crates/gp-cli/src/serve/reload.rs). In that moment a request is either
// refused with 503 and {"error": "gp serve is moving ..."} (serve/mod.rs
// moving), which the server never acted on, or can't reach the server at all
// (fetch's TypeError). Both are waited through, the way the screens page's
// reach does (screen.js), with the wait told to `away` so the page can say
// so in words; once the server answers, `back` is told. A call that can't
// reach the server is sent again only for a GET: a POST may have been taken.
// Checked without a browser by mechanic/journeys/reach.test.mjs.

/** How the page says it waits, `ms` into the wait. */
export function awayWords(ms) {
  return `gp serve is moving onto a newly published gp: waiting for it (${Math.round(ms / 1000)} s). This page carries on by itself.`;
}

/** `path` as JSON: a GET, or a POST of `body` as JSON. Throws the server's
 * own words for any other refusal, and once it has waited `within` ms. */
export async function call(path, body, {
  away = () => {},
  back = () => {},
  within = 60000,
  every = 1000,
  sleep = (ms) => new Promise((r) => setTimeout(r, ms)),
  fetch: get = (...a) => globalThis.fetch(...a),
} = {}) {
  const post = body !== undefined;
  let waited = 0;
  for (;;) {
    let r = null;
    try {
      r = await get(path, post ? { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) } : {});
    } catch (e) {
      if (post || !(e instanceof TypeError) || waited >= within) throw e;
    }
    if (r && (r.status !== 503 || waited >= within)) {
      if (waited) back();
      const v = await r.json().catch(() => ({}));
      if (!r.ok) throw new Error(v.error ?? `the page's server answered ${r.status}`);
      return v;
    }
    away(waited);
    await sleep(every);
    waited += every;
  }
}
