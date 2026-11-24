// The screen page's model start through a server that moves onto a new
// build, checked without a browser: the block between "// <reach>" and
// "// </reach>" in screen.js is run on its own. Run with
// `node --test mechanic/reach.test.mjs`; `cargo test` runs it too.
import fs from "node:fs";
import test from "node:test";
import assert from "node:assert/strict";

const src = fs.readFileSync(new URL("./screen.js", import.meta.url), "utf8");
const block = src.slice(src.indexOf("// <reach>"), src.indexOf("// </reach>"));
const { reach, buildModel, sendTalk, unsaved } = new Function(`${block}; return { reach, buildModel, sendTalk, unsaved };`)();

const sleep = async () => {};
const away = () => { throw new TypeError("Failed to fetch"); };

/** A page's server: `answer(path, body, n)` answers the nth request; every
 * request is kept in `sent`. */
function server(answer) {
  const sent = [];
  const api = async (path, body) => {
    sent.push(body === undefined ? path : `POST ${path}`);
    return answer(path, body, sent.length);
  };
  return { api, sent };
}

test("a model building while the server moves onto a new build is built again and starts", async () => {
  // Built? No. Build. Building 5 s. Then the server is away for three
  // requests and comes back having forgotten the build.
  let restarted = false;
  let builds = 0;
  const s = server((path, body, n) => {
    if (path === "api/rebuild/build") { builds++; return {}; }
    if (n === 1) return { model: true, built: false, building: null };
    if (n === 3) return { model: true, built: false, building: 5 };
    if (n >= 4 && n <= 6) away();
    if (!restarted) { restarted = true; return { model: true, built: false, building: null, error: null }; }
    if (builds < 2) throw new Error("the build wasn't asked for again");
    return n < 10 ? { model: true, built: false, building: 1 } : { model: true, built: true, building: null, build: "k2" };
  });
  const waited = [];
  const st = await buildModel(s.api, null, { sleep, away: (w) => waited.push(w) });
  assert.equal(st.built, true);
  assert.equal(st.build, "k2");
  assert.equal(builds, 2);
  assert.equal(waited.length, 3);
});

test("a card's look asks for its own build again", async () => {
  const bodies = [];
  const s = server((path, body, n) => {
    if (path === "api/rebuild/build") { bodies.push(body); return {}; }
    if (n === 3) away();
    if (n === 6) return { model: true, built: true, building: null };
    return { model: true, built: false, building: null };
  });
  await buildModel(s.api, "a1", { sleep });
  assert.deepEqual(s.sent, [
    "api/rebuild/status?ask=a1",
    "POST api/rebuild/build",
    "api/rebuild/status?ask=a1",
    "api/rebuild/status?ask=a1",
    "POST api/rebuild/build",
    "api/rebuild/status?ask=a1",
  ]);
  assert.deepEqual(bodies, [{ ask: "a1" }, { ask: "a1" }]);
});

test("a build that failed isn't asked for again", async () => {
  const s = server((path, body, n) => {
    if (path === "api/rebuild/build") return {};
    if (n === 1) return { model: true, built: false, building: null };
    if (n === 3) away();
    return { model: true, built: false, building: null, error: "cargo failed" };
  });
  await assert.rejects(buildModel(s.api, null, { sleep }), /cargo failed/);
  assert.equal(s.sent.filter((p) => p.startsWith("POST")).length, 1);
});

test("an answer from the server is thrown at once; a server away too long is given up on", async () => {
  let n = 0;
  await assert.rejects(reach(async () => { n++; throw new Error("409 no clip"); }, { sleep }), /409 no clip/);
  assert.equal(n, 1);
  n = 0;
  await assert.rejects(reach(async () => { n++; away(); }, { sleep, within: 5000, every: 1000 }), TypeError);
  assert.equal(n, 6);
  assert.equal(await reach(async () => (++n < 9 ? away() : "ok"), { sleep }), "ok");
});

test("a 503 from a server moving onto a new gp is waited through as a server away", async () => {
  let n = 0;
  const moving = () => { throw Object.assign(new Error("gp serve is moving onto a newly published gp; try again in a second"), { away: true }); };
  const waited = [];
  assert.equal(await reach(async () => (++n < 4 ? moving() : "ok"), { sleep, away: (w) => waited.push(w) }), "ok");
  assert.deepEqual(waited, [0, 1000, 2000]);
});

test("the person's words sent while the server moves onto a new build are sent again, the same", async () => {
  // Away for two tries, then the server keeps the turn.
  const s = server((path, body, n) => (n <= 2 ? away() : { turn: 4, again: false }));
  const body = { text: "the mug floats", clips: [], mechanic: "pour" };
  const waited = [];
  assert.deepEqual(await sendTalk(s.api, body, { sleep, away: (w) => waited.push(w) }), { turn: 4, again: false });
  assert.deepEqual(s.sent, ["POST api/talk", "POST api/talk", "POST api/talk"]);
  assert.deepEqual(waited, [0, 1000]);
  // A refusal from the server is said at once, in its words.
  const r = server(() => { throw new Error("no row pour"); });
  await assert.rejects(sendTalk(r.api, body, { sleep }), /no row pour/);
  assert.equal(r.sent.length, 1);
  assert.equal(unsaved(new Error("no row pour")), "no row pour");
  // A server that never comes back is given up on in plain words.
  const gone = server(() => away());
  const err = await sendTalk(gone.api, body, { sleep, within: 3000 }).catch((e) => e);
  assert.match(unsaved(err), /didn't answer for a minute. Your words are still in the box/);
  assert.doesNotMatch(unsaved(err), /Failed to fetch/);
});
