// Words that name a defined thing in Game Printer's docs. A page that is about
// clips says "clip" every time it means one; replacing it with "it" or "one"
// makes the reader guess, so the repeated-word rule (dead-metaphor/rare-lemma)
// does not apply to these.
export const DOMAIN_TERMS = new Set(["clip", "model", "mapping", "run"]);

// The message reads: "clip/clips" recurs 58 times (5% of the document's content words)
export function isDomainTermRepeat(finding) {
  if (finding.ruleId !== "dead-metaphor/rare-lemma") return false;
  const m = /^[“"]([^”"]*)[”"] recurs/.exec(finding.message);
  if (m === null) return false;
  const forms = m[1].split("/").map((w) => w.toLowerCase());
  return forms.every((w) => DOMAIN_TERMS.has(w) || DOMAIN_TERMS.has(w.replace(/s$/, "")));
}
