// explain.js under Node (`node --test`): the TOML it reads, the hooks'
// words, and the answer for a body, a HUD field, the stage and a package
// that can't answer, all from made-up files. No browser and no ROM.
import { test } from "node:test";
import assert from "node:assert/strict";
import { CANNOT, explainSpot, hookWords, pickModel, readToml, wantedFile } from "./explain.js";

const MODEL = `# A made-up game.
[model]
name = "hop_test"
title = 'Hop Test'

[entities.hero]
description = "The hero"
role = "player"
size = [16, 16]

[entities.hero.hitbox]
x = 2
y = 0
w = 12
h = 16

[entities.hero.run]
speed = "1.5"

[entities.slug]
description = "A slug"
ext = "slug_crawl"
role = "enemy"
size = [16, 8]

[entities.slug.contact]
otherwise = "kill-player"

[entities.ghost]
role = "enemy"
size = [8, 8]

[counters.score]
description = "The score, made up"

[levels.one]
description = """
A flat field"""

[present.hud]
size = [32, 2]

[[present.hud.field]]
at = [2, 1]
counter = "score"
digits = 6

[evidence."entities.slug"]
source = "observed"
confidence = 70
note = "It crawls a pixel a frame at $C000. It turns at walls. A third sentence stays in the note."
`;

const MAPPING = `model = "hop_test"

[[field]]
note = "The hero's x"
path = "entities.hero[0].x"
ram = [0x0203]

[slots]
active = 0x0230
active_stride = 16
count = 4

[[slots.field]]
note = "The slug's x"
path = "x"
ram = [563]
stride = 16

[[slots.field]]
path = "ext[0]"
ram = [94]

[[slots.kind]]
entity = "slug"
ids = [0]
`;

const HOOKS = `//! | Hook | Entity | What it does |
//! |---|---|---|
//! | \`slug_crawl\` | slug | Crawls along the floor and turns at a wall ([\`crawl\`]) |
pub fn nothing() {}
`;

const files = () =>
  new Map([
    ["model.toml", MODEL],
    ["mapping.toml", MAPPING],
    ["ext/src/lib.rs", HOOKS],
  ]);
const spot = (hits, more = {}) => ({ frame: 120, level: "one", camera: [0, 0], world: [50, 50], hud: [50, 50], hits, ...more });
const hit = (kind, extra = {}) => ({ body: 0, kind, x: 40, y: 48, facing: "left", alive: true, off: 0, top: 1, box: [0.1, 0.2, 0.05, 0.05], ...extra });

test("the TOML a package's model and mapping are written in", () => {
  const t = readToml(`a = 1 # a comment
b = "x # not a comment"
c = 'lit\\eral'
d = [1, 2,
  3, # inside
]
e = { f = true, "g.h" = 0x10 }
i.j = 1_000
k = """
two
lines"""
[t."a.b"]
x = 1.5
[[arr]]
n = 1
[[arr]]
n = 2
[arr.sub]
m = 3
`);
  assert.equal(t.a, 1);
  assert.equal(t.b, "x # not a comment");
  assert.equal(t.c, "lit\\eral");
  assert.deepEqual(t.d, [1, 2, 3]);
  assert.deepEqual(t.e, { f: true, "g.h": 16 });
  assert.equal(t.i.j, 1000);
  assert.equal(t.k, "two\nlines");
  assert.equal(t.t["a.b"].x, 1.5);
  assert.deepEqual(t.arr, [{ n: 1 }, { n: 2, sub: { m: 3 } }]);
  assert.throws(() => readToml('a = "never ends\n'), /line 1/);
  assert.throws(() => readToml("a = 1 b = 2"), /one key a line/);
});

test("the hooks' own table, in words", () => {
  const h = hookWords(new Map([["ext/src/lib.rs", HOOKS]]));
  assert.deepEqual(h.get("slug_crawl"), { file: "ext/src/lib.rs", kinds: "slug", words: "Crawls along the floor and turns at a wall (crawl)" });
  assert.equal(h.has("Hook"), false);
});

test("a body: what the model says of it, how it moves, where the original keeps it", () => {
  const a = explainSpot(spot([hit("slug"), hit("hero")]), files(), "hop_test");
  assert.equal(a.can, true);
  assert.equal(a.title, "A slug");
  assert.equal(a.kind, "slug");
  assert.deepEqual(a.box, [0.1, 0.2, 0.05, 0.05]);
  const text = a.lines.join("\n");
  assert.match(text, /^It is an enemy\. Its touch box is 16 by 8 pixels\./);
  assert.match(text, /at x 40, y 48, facing left, on level one \(frame 120\)/);
  assert.match(text, /Touching it costs the player a life\./);
  assert.match(text, /the hook slug_crawl \(ext\/src\/lib\.rs\): Crawls along the floor and turns at a wall \(crawl\)\./);
  assert.match(text, /How it was worked out \(observed, 70 in 100 sure\): It crawls a pixel a frame at \$C000\. It turns at walls\./);
  assert.doesNotMatch(text, /third sentence/);
  assert.match(text, /one of 4 slots from \$0230, 16 bytes apart/);
  assert.match(text, /X at \$0233 \(every 16 bytes a slot\); its hook's word 0 at \$005E/);
  assert.match(text, /Also under the spot: The hero\./);
  assert.equal(a.more[0].text, "It crawls a pixel a frame at $C000. It turns at walls. A third sentence stays in the note.");
  assert.match(a.note, /gp element explain/);
});

test("the player, moved by the model's own parts", () => {
  const a = explainSpot(spot([hit("hero", { facing: "right" })]), files(), "hop_test");
  const text = a.lines.join("\n");
  assert.match(text, /^It is the one you play\. Its touch box is 12 by 16 pixels, in a 16 by 16 picture\./);
  assert.match(text, /The model's own parts move it: run\./);
  assert.match(text, /X at \$0203/);
});

test("a HUD field, the stage, and a thing the model has no words for", () => {
  const h = explainSpot(spot([], { hud: [20, 10] }), files(), "hop_test");
  assert.equal(h.title, "The HUD's score");
  assert.deepEqual(h.lines, ["score: The score, made up."]);
  const s = explainSpot(spot([], { hud: [20, 100] }), files(), "hop_test");
  assert.equal(s.title, "The stage");
  assert.match(s.lines[0], /this is the stage, one\. A flat field\./);
  const g = explainSpot(spot([hit("ghost")]), files(), "hop_test");
  assert.equal(g.can, false);
  assert.ok(g.why.startsWith(`${CANNOT}: its model has no words for the ghost`), g.why);
});

test("a package that can't explain itself says so", () => {
  const none = explainSpot(spot([hit("hero")]), new Map([["mapping.toml", MAPPING]]), "hop_test");
  assert.equal(none.can, false);
  assert.equal(none.why, `${CANNOT}: its package has no model.toml, the file that says what each thing in the game is.`);
  const broken = explainSpot(spot([hit("hero")]), new Map([["model.toml", "[model\n"]]), "hop_test");
  assert.equal(broken.can, false);
  assert.match(broken.why, /model\.toml couldn't be read \(line 1/);
  assert.match(explainSpot(null, files(), "hop_test").why, /its player can't say what it draws where/);
});

test("the model the lift played is the one read", () => {
  const lifted = MODEL.replace('name = "hop_test"', 'name = "hop_lifted"').replace('"The hero"', '"The lifted hero"');
  const f = new Map([...files(), ["play/lift/hop-lifted/model.toml", lifted]]);
  assert.equal(pickModel(f, "hop_lifted").path, "play/lift/hop-lifted/model.toml");
  assert.equal(pickModel(f, "something_else").path, "model.toml");
  assert.equal(explainSpot(spot([hit("hero")]), f, "hop_lifted").title, "The lifted hero");
  assert.ok(["model.toml", "mapping.toml", "play/lift/x/model.toml", "ext/src/fire.rs"].every(wantedFile));
  assert.ok(!["play/assets/tiles.png", "corpus.toml", "truth/labels.toml", "play/lift/x/asset-map.toml"].some(wantedFile));
});
