// The first screen's calls through gp serve moving onto a newly published
// gp (reach.js), checked without a browser: a copy's PLAY that reads its
// copy while the server answers 503 says it waits in words, then shows the
// game once the server answers. Run with
// `node --test mechanic/journeys/reach.test.mjs`; `cargo test` runs it too.
import test from "node:test";
import assert from "node:assert/strict";
import { call, awayWords } from "./reach.js";

const MOVING = "gp serve is moving onto a newly published gp; try again in a second";
const sleep = async () => {};

/** A response as fetch gives it. */
const answer = (status, v) => ({ status, ok: status >= 200 && status < 300, json: async () => v });

/** A server: `reply(n, path, init)` answers the nth request, all kept in `sent`. */
function server(reply) {
  const sent = [];
  const fetch = async (path, init) => {
    sent.push(init?.method === "POST" ? `POST ${path}` : path);
    return reply(sent.length, path, init);
  };
  return { fetch, sent };
}

const COPY = "/api/journeys/theme/last-call";
const PLAYING = { jobs: { "look-play": { state: "done", serving: true, json: { links: ["/mechanic/rooftop/?theme=last-call"] } } } };

test("a copy's PLAY read while gp serve moves shows a waiting line, then the game", async () => {
  // Three 503s while it moves (the owner's 2026-11-23 paste), then the copy.
  const s = server((n) => (n <= 3 ? answer(503, { error: MOVING }) : answer(200, PLAYING)));
  const lines = [];
  let line = null;
  const v = await call(COPY, undefined, {
    sleep,
    fetch: s.fetch,
    away: (ms) => { line = awayWords(ms); lines.push(line); },
    back: () => { line = null; },
  });
  assert.deepEqual(v, PLAYING);
  assert.equal(s.sent.length, 4);
  assert.equal(lines.length, 3);
  assert.match(lines[0], /^gp serve is moving onto a newly published gp: waiting for it \(0 s\)\. This page carries on by itself\.$/);
  assert.match(lines[2], /\(2 s\)/);
  for (const l of lines) assert.doesNotMatch(l, /[{}"]/, "the line is words, not the server's JSON");
  assert.equal(line, null, "the line goes once the server answers");
});

test("a button pressed while gp serve moves is sent again once it answers: a 503 was never acted on", async () => {
  const s = server((n) => (n === 1 ? answer(503, { error: MOVING }) : answer(200, { started: true })));
  assert.deepEqual(await call(`${COPY}/do/look-play`, {}, { sleep, fetch: s.fetch }), { started: true });
  assert.deepEqual(s.sent, [`POST ${COPY}/do/look-play`, `POST ${COPY}/do/look-play`]);
});

test("a server that can't be reached is waited for on a GET, never a POST", async () => {
  const gone = () => { throw new TypeError("Failed to fetch"); };
  const s = server((n) => (n <= 2 ? gone() : answer(200, PLAYING)));
  assert.deepEqual(await call(COPY, undefined, { sleep, fetch: s.fetch }), PLAYING);
  assert.equal(s.sent.length, 3);
  const p = server(() => gone());
  await assert.rejects(call(`${COPY}/do/look-play`, {}, { sleep, fetch: p.fetch }), TypeError);
  assert.equal(p.sent.length, 1);
});

test("any other refusal is the server's own words at once; a move that lasts too long is given up on", async () => {
  const s = server(() => answer(404, { error: "no such copy" }));
  await assert.rejects(call(COPY, undefined, { sleep, fetch: s.fetch }), /^Error: no such copy$/);
  assert.equal(s.sent.length, 1);
  const m = server(() => answer(503, { error: MOVING }));
  await assert.rejects(call(COPY, undefined, { sleep, fetch: m.fetch, within: 3000 }), new RegExp(MOVING));
  assert.equal(m.sent.length, 4);
});
