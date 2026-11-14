// The page check's theme line rule (theme-line.mjs), as check-page.mjs
// uses it: since the check reads the themes the source offers from the
// source, a page that offers a theme the source doesn't must still fail.
//   node --test scripts/web/public/theme-line.test.mjs
import assert from "node:assert/strict";
import { test } from "node:test";
import { offeredIn, themeLineWrong, wanted } from "./theme-line.mjs";

// A by-game list as lex00/gp-registry signs it, two themes for the game.
const byGame = `format = "1"
registry = "gp-registry"

[[line]]
package = "lex00/turnips"
version = "1.0.2"
title = "Turnips"

[[line]]
package = "ddmoney420/orangutan-arcade"
version = "0.9.0"
title = "Orangutan Arcade"

[[line]]
package = "lex00/turnips"
version = "1.0.1"
title = "Turnips"
`;

test("reads each theme the source offers once, by its name after the namespace", () => {
  assert.deepEqual(offeredIn(byGame), ["turnips", "orangutan-arcade"]);
  assert.deepEqual(wanted(offeredIn(byGame)), ["orangutan-arcade", "original", "turnips"]);
});

test("passes a theme line of Original and every theme the source offers, in any order", () => {
  assert.equal(themeLineWrong(["turnips", "original", "orangutan-arcade"], offeredIn(byGame)), null);
});

test("fails a page that offers a theme the source doesn't", () => {
  const said = themeLineWrong(["original", "turnips", "orangutan-arcade"], ["turnips"]);
  assert.equal(said, "the page offers orangutan-arcade, which the source doesn't");
  assert.notEqual(themeLineWrong(["original", "turnips", "somebody-else"], offeredIn(byGame)), null);
});

test("fails a page that leaves out a theme the source offers, or Original", () => {
  assert.equal(themeLineWrong(["original", "turnips"], offeredIn(byGame)), "the page doesn't offer orangutan-arcade, which the source does");
  assert.equal(themeLineWrong(["turnips", "orangutan-arcade"], offeredIn(byGame)), "the page doesn't offer original, which the source does");
});
