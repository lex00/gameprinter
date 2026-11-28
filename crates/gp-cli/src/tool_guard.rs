//! What the tool gate guards beside its checks (issue 51). The model gate
//! guards what judges a game change: a clip removed must be named and
//! carded. This is the same for what judges a tool change, and for the
//! files that steer later workers.
//!
//! A patch that deletes a test, marks one ignored or takes assertions out
//! of a file passes format, the build, `just check` and the tests. So the
//! gate reads the patch: each test it removes or ignores, and each file it
//! leaves with fewer assertions, must be named with `--removes-test`, or
//! the gate fails. A patch that names one, or that touches a file that
//! steers workers ([`STEERING`]), is not landed on the gate's own answer:
//! its card waits for the person (serve/runner.rs, run_yeses). The one
//! steering change the gate answers itself is new rows of the permission
//! table for commands it doesn't have yet (issue 58): the guard holds only
//! what loosens, and a new command's row loosens no rule there is.

use std::collections::{BTreeMap, BTreeSet};

/// The files that steer later workers: the agent's settings, hooks and
/// instructions, the worker's brief and command line (the runner), the
/// permission table, and this guard and its gate. A folder ends with `/`.
pub(crate) const STEERING: &[&str] = &[
    ".claude/",
    "AGENTS.md",
    "CLAUDE.md",
    "crates/gp-cli/src/serve/runner.rs",
    "crates/gp-mcp/src/policy.rs",
    "crates/gp-cli/src/tool_guard.rs",
    "crates/gp-cli/src/tool_gate.rs",
];

/// The files of `files` that steer later workers ([`STEERING`]).
pub(crate) fn steering(files: &[String]) -> Vec<String> {
    files
        .iter()
        .filter(|f| {
            STEERING.iter().any(|s| {
                if s.ends_with('/') {
                    f.starts_with(s) || f.contains(&format!("/{s}"))
                } else {
                    f.as_str() == *s
                }
            })
        })
        .cloned()
        .collect()
}

/// The files a patch's text changes, from its `---` and `+++` lines (both,
/// so a file it deletes is among them).
pub(crate) fn files_of(diff: &str) -> Vec<String> {
    let mut out = BTreeSet::new();
    for l in diff.lines() {
        let path = l
            .strip_prefix("+++ b/")
            .or_else(|| l.strip_prefix("--- a/"));
        if let Some(p) = path {
            out.insert(p.trim().to_owned());
        }
    }
    out.into_iter().collect()
}

/// One way a patch weakens what judges it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Weak {
    /// It removes test `name` of `file`.
    Removed { file: String, name: String },
    /// It marks test `name` of `file` ignored.
    Ignored { file: String, name: String },
    /// It leaves `file` with `n` fewer assertions and removes no test of
    /// it.
    Fewer { file: String, n: usize },
}

impl Weak {
    /// The name `--removes-test` takes for it: the test's, or the file's
    /// for assertions.
    pub(crate) fn name(&self) -> &str {
        match self {
            Weak::Removed { name, .. } | Weak::Ignored { name, .. } => name,
            Weak::Fewer { file, .. } => file,
        }
    }

    pub(crate) fn says(&self) -> String {
        match self {
            Weak::Removed { file, name } => format!("removes the test {name} ({file})"),
            Weak::Ignored { file, name } => format!("marks the test {name} ignored ({file})"),
            Weak::Fewer { file, n } => format!("leaves {file} with {n} fewer assertion(s)"),
        }
    }
}

/// The function a line of Rust declares (`fn NAME(` or `fn NAME<`).
fn fn_name(line: &str) -> Option<&str> {
    let at = line.find("fn ")?;
    if at > 0 && !line[..at].ends_with([' ', '\t']) {
        return None;
    }
    let rest = &line[at + 3..];
    let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))?;
    (end > 0 && rest[end..].starts_with(['(', '<'])).then(|| &rest[..end])
}

/// Whether a line of Rust asserts: a macro whose name starts `assert` or
/// `debug_assert`.
fn asserts(line: &str) -> bool {
    line.match_indices("assert").any(|(i, _)| {
        let after = &line[i + 6..];
        let name_end = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(after.len());
        after[name_end..].starts_with('!')
    })
}

/// How a patch's text weakens what judges it: the tests it removes (a
/// removed `#[test]` with its function, when no line the patch adds
/// declares a function of that name, so a test moved or rewritten is not
/// one), the tests it marks ignored (an added `#[ignore`), and each Rust
/// file it leaves with fewer assertions while removing no test of it.
pub(crate) fn weakened(diff: &str) -> Vec<Weak> {
    let mut file = String::new();
    // Per file: removed and added assertion lines.
    let mut counts: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut removed: Vec<(String, String)> = Vec::new();
    let mut ignored: Vec<(String, String)> = Vec::new();
    let mut added_fns: BTreeSet<String> = BTreeSet::new();
    // A removed `#[test]` waits for its function among the removed lines
    // after it; an added `#[ignore]` for the next function of any line.
    let (mut test_pending, mut ignore_pending) = (false, false);
    // A `#[test]` kept or added waits for the function it is on.
    let mut test_kept = false;
    for l in diff.lines() {
        // The file from either header line, so a file the patch deletes
        // (`+++ /dev/null`) keeps its name.
        if l.starts_with("--- ") || l.starts_with("+++ ") {
            if let Some(p) = l
                .strip_prefix("--- a/")
                .or_else(|| l.strip_prefix("+++ b/"))
            {
                file = p.trim().to_owned();
            }
            (test_pending, ignore_pending) = (false, false);
            continue;
        }
        if l.starts_with("diff ") || l.starts_with("index ") {
            continue;
        }
        if l.starts_with("@@") {
            (test_pending, ignore_pending) = (false, false);
            continue;
        }
        if !file.ends_with(".rs") {
            continue;
        }
        let (sign, body) = match l.chars().next() {
            Some(s @ ('+' | '-' | ' ')) => (s, &l[1..]),
            _ => continue,
        };
        let text = body.trim();
        if text.starts_with("//") {
            continue;
        }
        if sign != ' ' && asserts(text) {
            let c = counts.entry(file.clone()).or_default();
            if sign == '-' {
                c.0 += 1;
            } else {
                c.1 += 1;
            }
        }
        if sign == '-' {
            if text.starts_with("#[test]") || text.starts_with("#[tokio::test") {
                test_pending = true;
            } else if let Some(n) = fn_name(text) {
                if test_pending {
                    removed.push((file.clone(), n.to_owned()));
                }
                test_pending = false;
            }
        } else {
            // A line kept or added between a removed #[test] and a
            // function: the attribute went, the test's function stays as
            // no test. The next function is the one it was on.
            if text.starts_with("#[test]") || text.starts_with("#[tokio::test") {
                test_kept = true;
            }
            if let Some(n) = fn_name(text) {
                // Added as a test: a function added with no #[test] on it
                // doesn't make up for a test removed under its name.
                if sign == '+' && test_kept {
                    added_fns.insert(n.to_owned());
                }
                test_kept = false;
                if test_pending && sign == ' ' {
                    removed.push((file.clone(), n.to_owned()));
                }
                test_pending = false;
                if ignore_pending {
                    ignored.push((file.clone(), n.to_owned()));
                    ignore_pending = false;
                }
            }
            if sign == '+' && text.starts_with("#[ignore") {
                ignore_pending = true;
            }
        }
    }
    let mut out = Vec::new();
    let mut lost: BTreeSet<String> = BTreeSet::new();
    // Files a test left for another place in the patch: their assertions
    // are counted over the whole patch, where the test went.
    let mut moved: BTreeSet<String> = BTreeSet::new();
    for (file, name) in removed {
        if added_fns.contains(&name) {
            moved.insert(file);
        } else {
            lost.insert(file.clone());
            out.push(Weak::Removed { file, name });
        }
    }
    for (file, name) in ignored {
        out.push(Weak::Ignored { file, name });
    }
    let (all_minus, all_plus) = counts
        .values()
        .fold((0, 0), |(m, p), (a, b)| (m + a, p + b));
    for (file, (minus, plus)) in counts {
        if moved.contains(&file) && all_minus <= all_plus {
            continue;
        }
        if minus > plus && !lost.contains(&file) {
            out.push(Weak::Fewer {
                file,
                n: minus - plus,
            });
        }
    }
    out
}

/// Whether `named` (a `--removes-test` value) names `w`: its test's name,
/// or for assertions its file.
fn names(named: &str, w: &Weak) -> bool {
    let n = named.trim();
    n == w.name() || n.rsplit("::").next() == Some(w.name())
}

/// Why the gate fails a patch that weakens what judges it without naming
/// each: None when every one of `weak` is named in `named`.
pub(crate) fn unnamed(weak: &[Weak], named: &[String]) -> Option<String> {
    let missing: Vec<&Weak> = weak
        .iter()
        .filter(|w| !named.iter().any(|n| names(n, w)))
        .collect();
    if missing.is_empty() {
        return None;
    }
    Some(format!(
        "the patch {}, and doesn't name {}: a test taken out of the checks is named, as a clip removed from a corpus is. If it is meant, give --removes-test {} (through gp_run: removes_test), and its card waits for the person's yes; else put the test back",
        missing
            .iter()
            .map(|w| w.says())
            .collect::<Vec<_>>()
            .join("; "),
        if missing.len() == 1 { "it" } else { "them" },
        missing
            .iter()
            .map(|w| w.name())
            .collect::<Vec<_>>()
            .join(" --removes-test "),
    ))
}

/// The permission table: gp_run's rule for each command ([`STEERING`]).
pub(crate) const TABLE_FILE: &str = "crates/gp-mcp/src/policy.rs";

/// The rows a table entry is written with (`r(` and `laya(`, policy.rs).
const ROW_CALLS: &[&str] = &["r(", "laya("];

/// The commands whose rows a patch adds to the permission table, when that
/// is all it changes there (issue 58): every line it changes in
/// [`TABLE_FILE`] is an added one, the added lines, comments aside, are
/// whole rows (`r("PATH", ...),` or `laya("PATH", ...),`), and no row's
/// PATH is a command `has` says the table already holds, or one the patch
/// adds twice. Else why not, naming the first line or command that isn't.
/// A patch that doesn't touch the table adds none.
pub(crate) fn new_rows(diff: &str, has: &dyn Fn(&str) -> bool) -> Result<Vec<String>, String> {
    let mut in_table = false;
    // The added lines of the table, each run of them (between context or
    // hunk headers) read as one piece.
    let mut runs: Vec<String> = Vec::new();
    let mut run = String::new();
    let end = |run: &mut String, runs: &mut Vec<String>| {
        if !run.trim().is_empty() {
            runs.push(std::mem::take(run));
        }
        run.clear();
    };
    for l in diff.lines() {
        if l.starts_with("diff ") {
            end(&mut run, &mut runs);
            in_table = false;
            continue;
        }
        if let Some(p) = l
            .strip_prefix("--- a/")
            .or_else(|| l.strip_prefix("+++ b/"))
        {
            end(&mut run, &mut runs);
            in_table = p.trim() == TABLE_FILE;
            continue;
        }
        if l.starts_with("--- ") || l.starts_with("+++ ") || l.starts_with("index ") {
            continue;
        }
        if !in_table {
            continue;
        }
        if l.starts_with("@@") || l.starts_with(' ') || l.is_empty() {
            end(&mut run, &mut runs);
            continue;
        }
        if let Some(body) = l.strip_prefix('-') {
            return Err(format!(
                "it changes or removes a line of the permission table: {}",
                body.trim()
            ));
        }
        if let Some(body) = l.strip_prefix('+') {
            let t = body.trim();
            if !t.starts_with("//") {
                run.push_str(t);
                run.push(' ');
            }
            continue;
        }
        if l.starts_with('\\') {
            continue;
        }
        return Err(format!("it changes the permission table: {}", l.trim()));
    }
    end(&mut run, &mut runs);
    let mut out: Vec<String> = Vec::new();
    for r in runs {
        for path in rows_of(&r)? {
            if has(&path) {
                return Err(format!(
                    "it adds a row for {path}, a command the permission table already has"
                ));
            }
            if out.contains(&path) {
                return Err(format!("it adds two rows for {path}"));
            }
            out.push(path);
        }
    }
    Ok(out)
}

/// The command paths of `text`, when it is nothing but table rows, each
/// `r(` or `laya(` with a string first, its parentheses closed and a comma
/// after. Else why not.
fn rows_of(text: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut rest = text.trim();
    while !rest.is_empty() {
        let Some(call) = ROW_CALLS.iter().find(|c| rest.starts_with(**c)) else {
            return Err(format!(
                "it adds to the permission table what is not a row: {rest}"
            ));
        };
        let args = rest[call.len()..].trim_start();
        let Some(quoted) = args.strip_prefix('"') else {
            return Err(format!("it adds a row with no command: {rest}"));
        };
        let Some(q) = quoted.find('"') else {
            return Err(format!("it adds a row with no command: {rest}"));
        };
        let path = &quoted[..q];
        if path.is_empty() || path.contains('\\') {
            return Err(format!("it adds a row with no command: {rest}"));
        }
        // Close the call's parentheses, strings read whole.
        let (mut depth, mut in_str, mut esc, mut close) = (1usize, false, false, None);
        let from = rest.len() - args.len();
        for (i, c) in rest[from..].char_indices() {
            if in_str {
                match c {
                    _ if esc => esc = false,
                    '\\' => esc = true,
                    '"' => in_str = false,
                    _ => {}
                }
                continue;
            }
            match c {
                '"' => in_str = true,
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(from + i);
                        break;
                    }
                }
                '{' | '}' | ';' => {
                    return Err(format!(
                        "it adds to the permission table what is not a row: {rest}"
                    ));
                }
                _ => {}
            }
        }
        let Some(close) = close else {
            return Err(format!("it adds a row that isn't closed: {rest}"));
        };
        let Some(after) = rest[close + 1..].trim_start().strip_prefix(',') else {
            return Err(format!("it adds a row with no comma after it: {rest}"));
        };
        out.push(path.to_owned());
        rest = after.trim_start();
    }
    Ok(out)
}

/// Why a tool gate card's patch is the person's to answer and not the
/// gate's own (the runner's land_tools): it names a test it removes
/// (`removes_test` not empty), or it changes a file that steers later
/// workers, except a patch whose only change to one is new rows of the
/// permission table for commands it doesn't have yet (issue 58: the guard
/// holds only what loosens). None for any other patch.
pub(crate) fn needs_person(diff: &str, removes_test: &[String]) -> Option<String> {
    needs_person_with(diff, removes_test, &|p| gp_mcp::policy::row(p).is_some())
}

/// [`needs_person`], with `has` telling the commands the permission table
/// already holds (gp_mcp's table as this gp was built: main's).
fn needs_person_with(
    diff: &str,
    removes_test: &[String],
    has: &dyn Fn(&str) -> bool,
) -> Option<String> {
    if !removes_test.is_empty() {
        return Some(format!(
            "it takes out of the checks: {}",
            removes_test.join(", ")
        ));
    }
    let steers = steering(&files_of(diff));
    if steers.is_empty() {
        return None;
    }
    let table = if steers.iter().any(|f| f == TABLE_FILE) {
        match new_rows(diff, has) {
            Ok(rows) if steers.len() == 1 && !rows.is_empty() => return None,
            Ok(_) => None,
            Err(why) => Some(why),
        }
    } else {
        None
    };
    Some(format!(
        "it changes what steers later workers: {}{}",
        steers.join(", "),
        table.map(|w| format!(" ({w})")).unwrap_or_default()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = "diff --git a/crates/x/src/lib.rs b/crates/x/src/lib.rs\nindex 111..222 100644\n--- a/crates/x/src/lib.rs\n+++ b/crates/x/src/lib.rs\n";

    fn patch(hunk: &str) -> String {
        format!("{HEAD}@@ -10,9 +10,3 @@ mod tests {{\n{hunk}")
    }

    /// Issue 51: a patch that deletes a test, marks one ignored or takes
    /// assertions out is read from its text.
    #[test]
    fn a_patch_that_weakens_the_tests_is_read() {
        let deleted = patch(
            " use super::*;\n-    #[test]\n-    fn it_adds() {\n-        assert_eq!(add(1, 1), 2);\n-    }\n",
        );
        assert_eq!(
            weakened(&deleted),
            [Weak::Removed {
                file: "crates/x/src/lib.rs".into(),
                name: "it_adds".into()
            }]
        );
        // The attribute alone removed: the function stays, as no test.
        let untested = patch("-    #[test]\n     fn it_adds() {\n");
        assert_eq!(weakened(&untested)[0].name(), "it_adds");
        let ignored = patch("     #[test]\n+    #[ignore]\n     fn it_adds() {\n");
        assert_eq!(
            weakened(&ignored),
            [Weak::Ignored {
                file: "crates/x/src/lib.rs".into(),
                name: "it_adds".into()
            }]
        );
        let loosened = patch(
            "     fn it_adds() {\n-        assert_eq!(add(1, 1), 2);\n-        assert!(add(2, 2) == 4, \"four\");\n+        let _ = add(1, 1);\n",
        );
        assert_eq!(
            weakened(&loosened),
            [Weak::Fewer {
                file: "crates/x/src/lib.rs".into(),
                n: 2
            }]
        );
    }

    /// A patch that rewrites, moves or adds tests weakens nothing, and
    /// files that are not Rust are not read for tests.
    #[test]
    fn a_patch_that_rewrites_or_moves_a_test_is_not_one_that_removes_it() {
        let rewritten = patch(
            "-    #[test]\n-    fn it_adds() {\n-        assert_eq!(add(1, 1), 2);\n-    }\n+    #[test]\n+    fn it_adds() {\n+        assert_eq!(add(1, 1), 2);\n+        assert_eq!(add(2, 2), 4);\n+    }\n",
        );
        assert_eq!(weakened(&rewritten), []);
        let moved = format!(
            "{}{}",
            patch("-    #[test]\n-    fn it_adds() {\n-        assert!(true);\n-    }\n"),
            "diff --git a/crates/x/src/sum.rs b/crates/x/src/sum.rs\n--- a/crates/x/src/sum.rs\n+++ b/crates/x/src/sum.rs\n@@ -1,1 +1,5 @@\n+    #[test]\n+    fn it_adds() {\n+        assert!(true);\n+    }\n"
        );
        assert_eq!(weakened(&moved), [], "{moved}");
        // Rewritten as a plain function, the test is gone.
        let untested = patch(
            "-    #[test]\n-    fn it_adds() {\n-        assert!(true);\n-    }\n+    fn it_adds() {}\n",
        );
        assert_eq!(weakened(&untested)[0].name(), "it_adds");
        let added = patch(
            "+    #[test]\n+    fn it_subtracts() {\n+        assert_eq!(sub(2, 1), 1);\n+    }\n",
        );
        assert_eq!(weakened(&added), []);
        let words = "--- a/docs/a.md\n+++ b/docs/a.md\n@@ -1,2 +1,1 @@\n-#[test]\n-fn gone() {\n-assert!(x);\n";
        assert_eq!(weakened(words), []);
        // A comment that says assert is no assertion.
        let comment = patch("-        // assert!(old) was here\n+        let x = 1;\n");
        assert_eq!(weakened(&comment), []);
        assert!(asserts("debug_assert_eq!(a, b);") && asserts("assert!(x)"));
        assert!(!asserts("let asserted = 1;") && !asserts("fn assert_ok() {"));
        assert_eq!(fn_name("    pub(crate) fn a_b<T>(x: T) {"), Some("a_b"));
        assert_eq!(fn_name("let often = 1;"), None);
    }

    /// Issue 51: the gate fails a weakening the patch doesn't name, and a
    /// named one, or a file that steers workers, is the person's to answer.
    #[test]
    fn an_unnamed_removal_fails_and_a_named_one_waits_for_the_person() {
        let deleted = patch("-    #[test]\n-    fn it_adds() {\n-        assert!(true);\n-    }\n");
        let weak = weakened(&deleted);
        let why = unnamed(&weak, &[]).unwrap();
        assert!(why.contains("removes the test it_adds"), "{why}");
        assert!(why.contains("--removes-test it_adds"), "{why}");
        assert_eq!(unnamed(&weak, &["it_adds".into()]), None);
        assert_eq!(unnamed(&weak, &["tests::it_adds".into()]), None);
        assert!(unnamed(&weak, &["another".into()]).is_some());
        let loosened = patch("-        assert!(a);\n");
        let weak = weakened(&loosened);
        assert!(
            unnamed(&weak, &[])
                .unwrap()
                .contains("1 fewer assertion(s)")
        );
        assert_eq!(unnamed(&weak, &["crates/x/src/lib.rs".into()]), None);
        assert_eq!(unnamed(&[], &[]), None);
        // The person's: a named removal, or a steering file.
        assert!(
            needs_person(&deleted, &["it_adds".into()])
                .unwrap()
                .contains("it_adds")
        );
        assert_eq!(needs_person(&deleted, &[]), None);
        for f in [
            ".claude/settings.json",
            "AGENTS.md",
            "crates/gp-mcp/src/policy.rs",
            "crates/gp-cli/src/serve/runner.rs",
            "editor/.claude/settings.local.json",
        ] {
            let d = format!("--- a/{f}\n+++ b/{f}\n@@ -1 +1 @@\n-a\n+b\n");
            assert!(
                needs_person(&d, &[]).unwrap().contains(f),
                "{f} steers workers"
            );
        }
        // A file it adds or deletes is one it changes.
        let new = "--- /dev/null\n+++ b/.claude/hooks/x.sh\n@@ -0,0 +1 @@\n+echo\n";
        assert!(needs_person(new, &[]).is_some());
        let gone = "--- a/AGENTS.md\n+++ /dev/null\n@@ -1 +0,0 @@\n-a\n";
        assert!(needs_person(gone, &[]).is_some());
        let plain = "--- a/docs/loop.md\n+++ b/docs/loop.md\n@@ -1 +1 @@\n-a\n+b\n";
        assert_eq!(needs_person(plain, &[]), None);
    }

    /// A patch of the permission table, with `hunk` its hunk's lines.
    fn table(hunk: &str) -> String {
        format!(
            "diff --git a/{TABLE_FILE} b/{TABLE_FILE}\nindex 111..222 100644\n--- a/{TABLE_FILE}\n+++ b/{TABLE_FILE}\n@@ -300,4 +300,6 @@ pub const TABLE: &[Row] = &[\n     r(\"corpus show\", J, Read),\n{hunk}     r(\"corpus verify\", J, Write),\n"
        )
    }

    /// Issue 58: the guard holds only what loosens. A patch whose only
    /// change to a steering file is new rows of the permission table for
    /// commands it doesn't have is the gate's to answer; a row changed or
    /// removed, a row for a command the table has, anything in the table
    /// that isn't a row, or a new row beside a change to the runner, the
    /// gate or the guard is the person's.
    #[test]
    fn only_a_new_row_for_a_new_command_is_not_held() {
        let has = |p: &str| ["corpus show", "corpus verify", "rooftop sweep"].contains(&p);
        let held = |d: &str| needs_person_with(d, &[], &has);
        // Added: one line, several lines, a comment, and laya( rows.
        let added = table("+    r(\"hilltop climb\", S, Read),\n");
        assert_eq!(new_rows(&added, &has), Ok(vec!["hilltop climb".into()]));
        assert_eq!(held(&added), None);
        let multi = table(
            "+    // It climbs (a comment).\n+    r(\n+        \"hilltop climb\",\n+        D,\n+        Rule::Refuse(\"only the person (here) climbs.\"),\n+    ),\n+    laya(\"hilltop look\", S, Read, Laya::With(\"open\")),\n",
        );
        assert_eq!(
            new_rows(&multi, &has),
            Ok(vec!["hilltop climb".into(), "hilltop look".into()])
        );
        assert_eq!(held(&multi), None);
        // With a plain file beside it, still the gate's.
        let beside_docs = format!(
            "{added}diff --git a/docs/loop.md b/docs/loop.md\n--- a/docs/loop.md\n+++ b/docs/loop.md\n@@ -1 +1 @@\n-a\n+b\n"
        );
        assert_eq!(held(&beside_docs), None);
        // Changed: a row's rule loosened.
        let changed = format!(
            "--- a/{TABLE_FILE}\n+++ b/{TABLE_FILE}\n@@ -300,3 +300,3 @@\n     r(\"corpus show\", J, Read),\n-    r(\"corpus verify\", J, Rule::Refuse(\"no\")),\n+    r(\"corpus verify\", J, Write),\n"
        );
        let why = held(&changed).unwrap();
        assert!(
            why.contains(TABLE_FILE) && why.contains("changes or removes"),
            "{why}"
        );
        // Removed: a row taken out.
        let removed = format!(
            "--- a/{TABLE_FILE}\n+++ b/{TABLE_FILE}\n@@ -300,3 +300,2 @@\n     r(\"corpus show\", J, Read),\n-    r(\"corpus verify\", J, Write),\n"
        );
        assert!(held(&removed).unwrap().contains("corpus verify"));
        // A row for a command the table has, as a second row to shadow it,
        // or the same new command twice.
        let again = table("+    r(\"rooftop sweep\", J, Write),\n");
        assert!(held(&again).unwrap().contains("already has"));
        let twice =
            table("+    r(\"hilltop climb\", S, Read),\n+    r(\"hilltop climb\", S, Write),\n");
        assert!(held(&twice).unwrap().contains("two rows"));
        // Added lines that aren't whole rows: an argument into a row the
        // table has, a const, a row with no comma after it.
        for hunk in [
            "+        &[\"dry-run\"],\n",
            "+const CLIMB: &str = \"climbs\";\n",
            "+    r(\"hilltop climb\", S, Read)\n",
            "+    r(\"hilltop climb\", S, { Read }),\n",
        ] {
            assert!(held(&table(hunk)).is_some(), "{hunk}");
        }
        // A new row beside the runner, the gate or the guard is held.
        for f in [
            "crates/gp-cli/src/serve/runner.rs",
            "crates/gp-cli/src/tool_gate.rs",
            "crates/gp-cli/src/tool_guard.rs",
        ] {
            let d = format!(
                "{added}diff --git a/{f} b/{f}\n--- a/{f}\n+++ b/{f}\n@@ -1 +1,2 @@\n a\n+b\n"
            );
            let why = held(&d).unwrap();
            assert!(why.contains(f) && why.contains(TABLE_FILE), "{why}");
        }
        // A named test removal is the person's whatever it changes.
        assert!(needs_person_with(&added, &["it_adds".into()], &has).is_some());
        // Read against this gp's own table: a command it has is held.
        assert!(needs_person(&table("+    r(\"corpus show\", J, Write),\n"), &[]).is_some());
    }
}
