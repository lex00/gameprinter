// share.js under Node (`node --test`): the address read as untrusted, held
// to the game's own names, a share link and its round trip, and a game's
// page facts from its package. No browser and no ROM.
import { test } from "node:test";
import assert from "node:assert/strict";
import { gamePageOf, linkSettings, packageRef, profileNames, readLink, registryAddress, rootKey, shareLink, sourcesOf, themesSource } from "./share.js";

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
  assert.deepEqual(packageRef("lex00/made-up-game"), { name: "lex00/made-up-game", version: null });
  assert.deepEqual(packageRef("lex00/made-up-game@1.0.9"), { name: "lex00/made-up-game", version: "1.0.9" });
  for (const bad of ["made-up-game", "../x/y", "lex00/../catalog", "a/b/c", "Lex00/mg", "lex00/mg@latest", "lex00/mg@1.0", "lex00/<b>", "lex00/mg@1.0.0@2", "", "a/" + "b".repeat(80)]) {
    assert.equal(packageRef(bad), null, bad);
  }
});

test("a link's keys are read, unknown or misshapen ones left out by name only", () => {
  const l = readLink("?g=lex00/made-up-game&c=lex00/turnips&mode=normal&on=smooth_walk,quick_restart&rom=AAAA&g2=x");
  assert.deepEqual(l.game, { name: "lex00/made-up-game", version: null });
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
  const a = shareLink(base, { game: "lex00/made-up-game", content: "lex00/turnips", profile: "normal", names: n });
  assert.equal(a, "https://play.example/p?g=lex00/made-up-game&c=lex00/turnips&mode=normal");
  const b = shareLink(base, { game: "lex00/made-up-game", look: "original", profile: "smooth_walk+quick_restart", names: n });
  assert.equal(b, "https://play.example/p?g=lex00/made-up-game&look=original&on=smooth_walk,quick_restart");
  // A switch the game hasn't, or a profile that isn't one, is left out.
  const c = shareLink(base, { game: "lex00/made-up-game", profile: "smooth_walk+<x>+gone", names: n });
  assert.equal(c, "https://play.example/p?g=lex00/made-up-game&on=smooth_walk");
  for (const [url, profile] of [[a, "normal"], [b, "smooth_walk+quick_restart"]]) {
    const back = readLink(new URL(url).search);
    assert.deepEqual(back.ignored, []);
    assert.equal(back.game.name, "lex00/made-up-game");
    assert.equal(linkSettings(back, n).profile, profile);
  }
  assert.throws(() => shareLink(base, { game: "not a name" }));
  // Only these keys: nothing of the player.
  for (const url of [a, b, c]) assert.ok([...new URL(url).searchParams.keys()].every((k) => ["g", "c", "look", "mode", "on", "r", "rk"].includes(k)), url);
});

// A made-up key body of the right shape (not anyone's key).
const RK = "AAAAC3NzaC1lZDI1NTE5AAAAI" + "Ab+/".repeat(10) + "xyz";
const REG = "https://raw.githubusercontent.com/acme/harbor-registry/main";

test("a registry by address: https only, held to its shape", () => {
  assert.equal(registryAddress(REG), REG);
  assert.equal(registryAddress(`${REG}/`), REG, "a trailing slash is taken off");
  assert.equal(registryAddress("https://acme.github.io/harbor"), "https://acme.github.io/harbor");
  for (const bad of [
    "http://raw.githubusercontent.com/acme/r/main",
    "http://127.0.0.1:8377",
    "ftp://acme.example/r",
    "javascript:alert(1)",
    "data:text/plain,x",
    "//acme.example/r",
    "acme.example/r",
    "https://user:pw@acme.example/r",
    "https://acme.example/r?x=1",
    "https://acme.example/r#x",
    "https://" + "a".repeat(400) + ".example",
    "",
  ]) {
    assert.equal(registryAddress(bad), null, bad);
  }
});

test("a root key in a link is an ssh-ed25519 key's body, a + read back from a space", () => {
  assert.equal(RK.length, 68);
  assert.equal(rootKey(RK), `ssh-ed25519 ${RK}`);
  assert.equal(rootKey(RK.replace(/\+/g, " ")), `ssh-ed25519 ${RK}`);
  for (const bad of ["", "ssh-ed25519 " + RK, RK.slice(1), RK + "A", "AAAAB3NzaC1yc2EAAAADAQABAAABAQ" + "A".repeat(38), RK.replace("x", "<")]) assert.equal(rootKey(bad), null, bad);
});

test("a link's registry: read, carried by a share link, and left out when it isn't one", () => {
  const l = readLink(`?g=lex00/made-up-game&c=acme/harbor-lights&r=${REG}`);
  assert.deepEqual(l.registry, { url: REG, root: null });
  assert.deepEqual(l.ignored, []);
  const signed = readLink(`?g=lex00/made-up-game&r=${encodeURIComponent(REG)}&rk=${encodeURIComponent(RK)}`);
  assert.deepEqual(signed.registry, { url: REG, root: `ssh-ed25519 ${RK}` });
  // A + not escaped arrives as a space, and is read back.
  assert.equal(readLink(`?r=${REG}&rk=${RK}`).registry.root, `ssh-ed25519 ${RK}`);
  const http = readLink("?g=lex00/made-up-game&r=http://raw.githubusercontent.com/acme/r/main");
  assert.equal(http.registry, null);
  assert.deepEqual(http.ignored, ["r"]);
  assert.deepEqual(readLink(`?rk=${encodeURIComponent(RK)}`).ignored, ["rk"], "a key with no registry is left out");
  assert.deepEqual(readLink(`?r=${REG}&rk=nope`).registry, { url: REG, root: null });
  assert.deepEqual(readLink(`?r=${REG}&rk=nope`).ignored, ["rk"]);
  assert.deepEqual(readLink(`?r=${REG}&r=https://other.example`).ignored, ["r"], "a registry given twice is not guessed at");

  const base = "https://play.example/p";
  const a = shareLink(base, { game: "lex00/made-up-game", content: "acme/harbor-lights", registry: l.registry });
  assert.equal(a, `https://play.example/p?g=lex00/made-up-game&c=acme/harbor-lights&r=${REG}`);
  assert.deepEqual(readLink(new URL(a).search).registry, l.registry);
  const b = shareLink(base, { game: "lex00/made-up-game", registry: signed.registry });
  assert.ok(b.includes("%2B"), "a key's + stays escaped");
  assert.deepEqual(readLink(new URL(b).search).registry, signed.registry);
  assert.equal(shareLink(base, { game: "lex00/made-up-game", registry: { url: "http://x.example" } }), "https://play.example/p?g=lex00/made-up-game");
  assert.equal(shareLink(base, { game: "lex00/made-up-game" }), "https://play.example/p?g=lex00/made-up-game");
});

test("the registry the page also takes themes from: the link's, else the one it was built with, never its own again", () => {
  const own = { name: "gp-registry", url: "https://raw.githubusercontent.com/lex00/gp-registry/main", root: "ssh-ed25519 OWN" };
  assert.equal(themesSource(own, readLink("?g=a/b")), null);
  assert.equal(themesSource(own, readLink(`?r=${own.url}/`)), null, "the page's own registry isn't a second one");
  assert.deepEqual(themesSource(own, readLink(`?r=${REG}`)), { name: REG, url: REG, root: null, byLink: true });
  assert.equal(themesSource(own, readLink(`?r=${REG}&rk=${encodeURIComponent(RK)}`)).root, `ssh-ed25519 ${RK}`);
  assert.equal(themesSource(own, readLink("?r=http://evil.example")), null);
  const built = { name: "harbor-registry", url: REG, root: null };
  assert.deepEqual(themesSource(own, readLink("?g=a/b"), built), { name: "harbor-registry", url: REG, root: null, byLink: false });
  assert.equal(themesSource(own, readLink("?r=https://other.example/r"), built).url, "https://other.example/r", "the link's wins");
  assert.equal(themesSource(own, readLink(""), { url: "http://x.example" }), null);
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
  bad((j) => (j.rom.game = "another-game"), /its lift is for made-up/);
  bad((j) => (j.rom.known_sha256 = { a: "not a hash" }), /known_sha256/);
  bad((j) => (j.howto = [{}]), /howto/);
  assert.throws(() => gamePageOf(new Map([["page.json", enc(facts)]]), "x"), /lift\/index.json/);
});

test("an arcade game's MAME sets from its play/page.json, each with the files its zip must hold", () => {
  const set = { name: "cab", title: "Cab (made up)", sha256: "ab".repeat(32), files: [{ name: "cab.1c", size: 4096, sha1: "cd".repeat(20) }] };
  const facts = { format: 1, title: "Cab", player: "arcade", rom: { kind: "zip", game: "cab", name: "Cab", file: "a zip", known: [], known_sha256: {}, sets: [set] }, howto: [] };
  const files = new Map([["lift/index.json", enc({ game: "cab", files: [] })], ["page.json", enc(facts)]]);
  assert.deepEqual(gamePageOf(files, "Cab").rom.sets, [set]);
  const without = structuredClone(facts);
  delete without.rom.sets;
  assert.equal(gamePageOf(new Map([...files, ["page.json", enc(without)]]), "Cab").rom.sets, undefined);
  for (const change of [(s) => (s.sha256 = "zz"), (s) => (s.files = []), (s) => (s.files[0].sha1 = "x"), (s) => (s.name = "<b>")]) {
    const j = structuredClone(facts);
    change(j.rom.sets[0]);
    assert.throws(() => gamePageOf(new Map([...files, ["page.json", enc(j)]]), "Cab"), /rom\.sets/);
  }
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
