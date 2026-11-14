// The page check's rule for the in-game theme line (check-page.mjs, the
// rom-TAG-saves section): it offers Original and each theme the page's
// source offers for the game, and nothing else. Apart so its own test
// (theme-line.test.mjs) can hold it to that without a browser. Not part
// of the page: build-page.sh copies the page's files by name.

/** The themes a source's by-game list offers, by id (a package's name
 * after its namespace), each once. */
export function offeredIn(text) {
  const names = [...text.matchAll(/^package = "([^"]+)"/gm)].map((m) => m[1].split("/")[1]);
  return [...new Set(names)];
}

/** What the theme line should hold: Original and the offered themes, sorted. */
export function wanted(offered) {
  return [...new Set(["original", ...offered])].sort();
}

/** What is wrong with the theme line the page showed (`got`, theme ids)
 * against what the source offers, in words, or null when it holds exactly
 * Original and those themes. */
export function themeLineWrong(got, offered) {
  const want = wanted(offered);
  const have = [...new Set(got)].sort();
  const extra = have.filter((t) => !want.includes(t));
  const missing = want.filter((t) => !have.includes(t));
  if (!extra.length && !missing.length) return null;
  const says = [];
  if (extra.length) says.push(`the page offers ${extra.join(", ")}, which the source doesn't`);
  if (missing.length) says.push(`the page doesn't offer ${missing.join(", ")}, which the source does`);
  return says.join("; ");
}
