// share.js under Node (`node --test`): the address read as untrusted, held
// to the game's own names, a share link and its round trip, and a game's
// page facts from its package. No browser and no ROM.
import { test } from "node:test";
import assert from "node:assert/strict";
import { gamePageOf, linkSettings, packageRef, profileNames, readLink, shareLink, sourcesOf } from "./share.js";

const PROFILES = `format = "0.2.0"
# A comment with name = "not_a_switch"
[[groups]]
name = "fixes"
title = "Engine fixes"

[[groups.switches]]
description = "A # in the text"
name = "smooth_walk"

[[groups.switches]]
name = "quick_restart"

[groups.switches.set]
name = "inside_a_set_table"

[presets.normal]
description = "Every fix"
switches = ["smooth_walk", "quick_restart"]
`;

test("a profiles.toml's modes and switches, and nothing else", () => {
  const n = profileNames(PROFILES);
  assert.deepEqual([...n.presets], ["normal"]);
  assert.deepEqual([...n.switches].sort(), ["quick_restart", "smooth_walk"]);
});

test("names and versions are held to their shape", () => {
  assert.deepEqual(packageRef("lex00/donkey-kong"), { name: "lex00/donkey-kong", version: null });
  assert.deepEqual(packageRef("lex00/donkey-kong@1.0.9"), { name: "lex00/donkey-kong", version: "1.0.9" });
  for (const bad of ["donkey-kong", "../x/y", "lex00/../catalog", "a/b/c", "Lex00/dk", "lex00/dk@latest", "lex00/dk@1.0", "lex00/<b>", "lex00/dk@1.0.0@2", "", "a/" + "b".repeat(80)]) {
    assert.equal(packageRef(bad), null, bad);
  }
});

test("a link's keys are read, unknown or misshapen ones left out by name only", () => {
  const l = readLink("?g=lex00/donkey-kong&c=lex00/turnips&mode=normal&on=smooth_walk,quick_restart&rom=AAAA&g2=x");
  assert.deepEqual(l.game, { name: "lex00/donkey-kong", version: null });
  assert.deepEqual(l.content, { name: "lex00/turnips", version: null });
  assert.equal(l.mode, "normal");
  assert.deepEqual(l.on, ["smooth_walk", "quick_restart"]);
  assert.deepEqual(l.ignored, ["an unknown key"]);
  const bad = readLink("?g=../../etc&c=x&look=<img src=x>&mode=a b&on=ok,no%20way");
  assert.equal(bad.game, null);
  assert.equal(bad.content, null);
  assert.equal(bad.look, null);
  assert.equal(bad.mode, null);
  assert.equal(bad.on, null);
  assert.deepEqual(bad.ignored, ["g", "c", "look", "mode", "on"]);
  assert.ok(!bad.ignored.some((k) => k.includes("<")), "never the value");
  // A key given twice is not guessed at.
  assert.deepEqual(readLink("?g=a/b&g=c/d").ignored, ["g"]);
  assert.equal(readLink("?g=a/b&g=c/d").game, null);
});

test("the settings a link asks for are held to the game's own", () => {
  const n = profileNames(PROFILES);
  assert.deepEqual(linkSettings(readLink("?mode=normal"), n), { profile: "normal", dropped: [] });
  assert.deepEqual(linkSettings(readLink("?mode=retro"), n), { profile: "retro", dropped: [] });
  assert.deepEqual(linkSettings(readLink("?mode=turbo"), n), { profile: null, dropped: ["turbo"] });
  assert.deepEqual(linkSettings(readLink("?on=quick_restart,fly"), n), { profile: "quick_restart", dropped: ["fly"] });
  assert.deepEqual(linkSettings(readLink("?on=fly"), n), { profile: null, dropped: ["fly"] });
  assert.deepEqual(linkSettings(readLink("?on=&mode=normal"), n), { profile: "retro", dropped: ["normal"] });
  assert.deepEqual(linkSettings(readLink(""), n), { profile: null, dropped: [] });
});

test("a share link carries the game, the theme or look and the settings, and reads back the same", () => {
  const n = profileNames(PROFILES);
  const base = "https://play.example/p?g=old/game&rom=x#saves";
  const a = shareLink(base, { game: "lex00/donkey-kong", content: "lex00/turnips", profile: "normal", names: n });
  assert.equal(a, "https://play.example/p?g=lex00/donkey-kong&c=lex00/turnips&mode=normal");
  const b = shareLink(base, { game: "lex00/donkey-kong", look: "original", profile: "smooth_walk+quick_restart", names: n });
  assert.equal(b, "https://play.example/p?g=lex00/donkey-kong&look=original&on=smooth_walk,quick_restart");
  // A switch the game hasn't, or a profile that isn't one, is left out.
  const c = shareLink(base, { game: "lex00/donkey-kong", profile: "smooth_walk+<x>+gone", names: n });
  assert.equal(c, "https://play.example/p?g=lex00/donkey-kong&on=smooth_walk");
  for (const [url, profile] of [[a, "normal"], [b, "smooth_walk+quick_restart"]]) {
    const back = readLink(new URL(url).search);
    assert.deepEqual(back.ignored, []);
    assert.equal(back.game.name, "lex00/donkey-kong");
    assert.equal(linkSettings(back, n).profile, profile);
  }
  assert.throws(() => shareLink(base, { game: "not a name" }));
  // Only these keys: nothing of the player.
  for (const url of [a, b, c]) assert.ok([...new URL(url).searchParams.keys()].every((k) => ["g", "c", "look", "mode", "on"].includes(k)), url);
});

const enc = (o) => new TextEncoder().encode(typeof o === "string" ? o : JSON.stringify(o));
const SOURCES = `# Hashes and facts only
[[source]]
id = "made-up-a"
title = "Made Up"
revision = "A"
sha256 = "${"ab".repeat(32)}"
notes = "x"

[[source]]
id = "other"
title = "Other"
sha256 = "${"cd".repeat(32)}"
`;

test("a game's page facts from its play/page.json, each field held to its shape", () => {
  const facts = {
    format: 1,
    title: "Made Up",
    player: "nes",
    rom: { kind: "nes", game: "made-up", name: "Made Up", file: "an .nes file", known: ["Made Up (A)"], known_sha256: { "Made Up (A)": "ab".repeat(32) } },
    howto: ["Controls: arrows move."],
    trademark: "Made Up is made up.",
  };
  const files = new Map([["lift/index.json", enc({ game: "made-up", files: [] })], ["page.json", enc(facts)]]);
  const f = gamePageOf(files, "Made Up");
  assert.equal(f.title, "Made Up");
  assert.equal(f.player, "nes");
  assert.equal(f.rom.game, "made-up");
  assert.deepEqual(f.howto, ["Controls: arrows move."]);
  const bad = (change, re) => {
    const j = structuredClone(facts);
    change(j);
    assert.throws(() => gamePageOf(new Map([...files, ["page.json", enc(j)]]), "Made Up"), re);
  };
  bad((j) => (j.format = 2), /format 1/);
  bad((j) => (j.player = "<script>"), /player/);
  bad((j) => (j.rom.game = "../x"), /game/);
  bad((j) => (j.rom.game = "donkey-kong"), /its lift is for made-up/);
  bad((j) => (j.rom.known_sha256 = { a: "not a hash" }), /known_sha256/);
  bad((j) => (j.howto = [{}]), /howto/);
  assert.throws(() => gamePageOf(new Map([["page.json", enc(facts)]]), "x"), /lift\/index.json/);
});

test("a game that plays from its own files: no lift, no ROM, who made it and what it is", () => {
  const facts = { format: 1, title: "Own Hop", player: "own", rom: { kind: "none", game: "own-hop" }, made_by: "A. Maker", about: "A hop of my own.", howto: ["Controls: arrows move."] };
  const files = new Map([["page.json", enc(facts)], ["bundle.json", enc({})], ["preview.json", enc({})]]);
  const f = gamePageOf(files, "Own Hop");
  assert.deepEqual(f, { title: "Own Hop", player: "own", rom: { kind: "none", game: "own-hop" }, howto: ["Controls: arrows move."], trademark: null, madeBy: "A. Maker", about: "A hop of my own." });
  const bad = (change, re, more = []) => {
    const j = structuredClone(facts);
    change(j);
    assert.throws(() => gamePageOf(new Map([...files, ["page.json", enc(j)], ...more]), "x"), re);
  };
  bad((j) => (j.rom.game = "../x"), /game's id/);
  bad((j) => (j.made_by = { name: "x" }), /made_by/);
  bad((j) => (j.about = "x".repeat(601)), /about/);
  bad(() => {}, /holds a ROM's lift/, [["lift/index.json", enc({ game: "own-hop", files: [] })]]);
  assert.throws(() => gamePageOf(new Map([["page.json", enc(facts)]]), "x"), /no bundle.json and preview.json/);
});

test("a package from before page.json is read from its lift's files, as the NES player", () => {
  assert.deepEqual(sourcesOf(SOURCES).map((s) => s.id), ["made-up-a", "other"]);
  const files = new Map([
    ["lift/index.json", enc({ game: "made-up", files: [] })],
    ["lift/game.json", enc({ game: "made-up", title: "Made Up: First", templates: ["made-up-a"] })],
    ["lift/sources-nes.toml", enc(SOURCES)],
  ]);
  const f = gamePageOf(files, "Made Up");
  assert.equal(f.title, "Made Up");
  assert.equal(f.player, "nes");
  assert.equal(f.rom.kind, "nes");
  assert.equal(f.rom.name, "Made Up: First");
  assert.deepEqual(f.rom.known, ["Made Up (A)"]);
  assert.deepEqual(f.rom.known_sha256, { "Made Up (A)": "ab".repeat(32) });
});
