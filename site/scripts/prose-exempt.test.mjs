import { test } from "node:test";
import assert from "node:assert/strict";
import { isDomainTermRepeat } from "./prose-exempt.mjs";

const f = (ruleId, message) => ({ ruleId, message });

test("a repeated domain term is exempt", () => {
  assert.ok(isDomainTermRepeat(f("dead-metaphor/rare-lemma", "“clip/clips” recurs 58 times (5% of the document's content words)")));
  assert.ok(isDomainTermRepeat(f("dead-metaphor/rare-lemma", "“model/models” recurs 30 times (4% of the document's content words)")));
  assert.ok(isDomainTermRepeat(f("dead-metaphor/rare-lemma", "“mapping/mappings” recurs 12 times (6% of the document's content words)")));
  assert.ok(isDomainTermRepeat(f("dead-metaphor/rare-lemma", "“run/runs” recurs 12 times (6% of the document's content words)")));
});

test("other words are not exempt", () => {
  assert.ok(!isDomainTermRepeat(f("dead-metaphor/rare-lemma", "“slot/slots” recurs 21 times (6% of the document's content words)")));
  assert.ok(!isDomainTermRepeat(f("dead-metaphor/rare-lemma", "“clip/frames” recurs 3 times (1% of the document's content words)")));
});

test("other rules are not exempt", () => {
  assert.ok(!isDomainTermRepeat(f("repetition/near-duplicate", "“clip/clips” recurs 58 times")));
});
