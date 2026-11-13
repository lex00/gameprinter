// Prose lint for the site, on the `sentences` trope ruleset (adapted from
// plunge's site/scripts/lint-prose.mjs). It reads every page under
// src/content/docs, which includes the pages generated from the repo's docs,
// and the repo README.md.
//
// A ratchet: findings are counted per (file, rule) in prose-baseline.json
// and the lint fails only when a count grows. When counts drop, rerun with
// --update-baseline and commit the smaller file.
//
//   node scripts/lint-prose.mjs                    # gate (CI)
//   node scripts/lint-prose.mjs --verbose          # every finding, file:line
//   node scripts/lint-prose.mjs --update-baseline
//
// medium and high severities gate; `reframe` ("It is not X. It is Y.")
// gates at any severity, as in chant.
import { readFileSync, writeFileSync, readdirSync, statSync } from "node:fs";
import { join, relative, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { RULES } from "sentences/lint/registry";
import { runRules } from "sentences/lint/engine";
import { buildDocAnalysis } from "sentences/lint/build-doc";
import { extractProse } from "sentences/lint/markdown-prose";
import { isDomainTermRepeat } from "./prose-exempt.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const REPO = join(here, "..", "..");
const DOCS_ROOT = join(here, "..", "src", "content", "docs");
const README = join(REPO, "README.md");
const BASELINE = join(here, "prose-baseline.json");
const GATED_SEVERITIES = new Set(["medium", "high"]);
const ALWAYS_GATED_RULES = new Set(["reframe"]);

const verbose = process.argv.includes("--verbose");
const updateBaseline = process.argv.includes("--update-baseline");

function docFiles(dir) {
  const out = [];
  for (const entry of readdirSync(dir)) {
    const p = join(dir, entry);
    if (statSync(p).isDirectory()) out.push(...docFiles(p));
    else if (/\.(md|mdx)$/.test(entry)) out.push(p);
  }
  return out.sort();
}

function lineOf(text, offset) {
  return text.slice(0, offset).split("\n").length;
}

// Anaphora fires on markup: consecutive bullets, link lists and headings
// "open" with "-", "[", "###" or ",". An opening with no letter in it is not
// a word, so those findings are dropped.
function markupAnaphora(f) {
  if (f.ruleId !== "anaphora/repeated-opening") return false;
  const m = /“([^”]*)”/.exec(f.message);
  return m !== null && !/\p{L}/u.test(m[1]);
}

const sources = docFiles(DOCS_ROOT).map((file) => {
  const prose = extractProse(readFileSync(file, "utf8"));
  return { rel: relative(REPO, file), prose, line: (o) => lineOf(prose, o) };
});
{
  const prose = extractProse(readFileSync(README, "utf8"));
  sources.push({ rel: "README.md", prose, line: (o) => lineOf(prose, o) });
}

const current = {}; // file -> rule -> gated count
let total = 0;
let gated = 0;
const detail = [];

for (const { rel, prose, line } of sources) {
  const { findings, errors } = runRules(RULES, buildDocAnalysis(prose));
  for (const e of errors) detail.push(`${rel}: rule ${e.ruleId} errored: ${e.message}`);
  for (const f of findings) {
    if (markupAnaphora(f) || isDomainTermRepeat(f)) continue;
    total++;
    if (verbose) detail.push(`${rel}:${line(f.span.start)} [${f.severity}] ${f.ruleId}: ${f.message}`);
    if (!GATED_SEVERITIES.has(f.severity) && !ALWAYS_GATED_RULES.has(f.ruleId)) continue;
    gated++;
    const perFile = (current[rel] ??= {});
    perFile[f.ruleId] = (perFile[f.ruleId] ?? 0) + 1;
  }
}

for (const l of detail) console.log(l);

if (updateBaseline) {
  writeFileSync(BASELINE, JSON.stringify(current, null, 1) + "\n");
  console.log(`prose lint: baseline written, ${gated} gated finding(s) in ${Object.keys(current).length} file(s) (${total} incl. low tiers)`);
  process.exit(0);
}

let baseline;
try {
  baseline = JSON.parse(readFileSync(BASELINE, "utf8"));
} catch {
  console.error(`prose lint: no baseline at ${relative(process.cwd(), BASELINE)}; run with --update-baseline and commit it`);
  process.exit(1);
}

const regressions = [];
for (const [file, rules] of Object.entries(current)) {
  for (const [rule, count] of Object.entries(rules)) {
    const allowed = baseline[file]?.[rule] ?? 0;
    if (count > allowed) regressions.push(`${file}: ${rule} ${allowed} -> ${count}`);
  }
}
const baselineTotal = Object.values(baseline).reduce(
  (n, rules) => n + Object.values(rules).reduce((m, c) => m + c, 0),
  0,
);

if (regressions.length > 0) {
  console.error(`prose lint: ${regressions.length} regression(s) against the baseline:`);
  for (const r of regressions) console.error(`  ${r}`);
  console.error("Fix the prose (--verbose gives file:line), or for a deliberate choice run --update-baseline and commit.");
  process.exit(1);
}

if (gated < baselineTotal) {
  console.log(`prose lint: ${gated} gated finding(s), no regressions, ${baselineTotal - gated} below the baseline of ${baselineTotal}; tighten it with --update-baseline`);
} else {
  console.log(`prose lint: ${gated} gated finding(s), no regressions (${baselineTotal} baselined)`);
}
