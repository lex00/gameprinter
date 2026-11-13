//! `gp agent integrate`: the coordinator's helper for merging the clip
//! agents' approved attempts (docs/agents.md, "Merging attempts"). It picks
//! them onto a new branch, checks the features their notes name, bumps a
//! version only for a note that says `breaking:`, regenerates schema/,
//! writes the notes into the ROADMAP and the change log, and runs `gp
//! model check` on the games. It never merges into main: in gameprinter
//! the tool gate (`gp tool gate`) lands the branch. Changed from
//! game-printer: the schema comes from `gp self schemas` (was `gp docs
//! schemas`), the model check runs on the games of the games checkout
//! (was templates/), and `--check` runs `just check` on the branch (was
//! scripts/merge-check.sh, which gameprinter doesn't have).
//!
//! An added option is a named feature (docs/model.md, "Versions"), so
//! adding one bumps nothing and two lanes collide only on the same name.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, bail};
use gp_evidence::Paths;
use serde_json::{Value, json};

use crate::Outcome;
use crate::serve::work;

pub(crate) struct Args<'a> {
    pub repo: &'a Path,
    pub branch: &'a str,
    pub gp: Option<&'a Path>,
    pub skip: &'a [u64],
    pub date: Option<&'a str>,
    pub dry_run: bool,
    /// Run `just check` on the branch at the end.
    pub check: bool,
    /// The games checkout the model check runs on (default:
    /// $GP_GAMES_DIR, else one beside the repo).
    pub games: Option<&'a Path>,
}

/// One approved attempt not on main.
#[derive(Debug, Clone)]
struct Pick {
    run: String,
    attempt: u64,
    clip: String,
    title: String,
    commit: String,
    approved_ms: u64,
}

/// One attempt's `CHANGE.md`, as filed in `changes/`.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct Note {
    pub file: String,
    /// The features the change adds, by name.
    pub features: BTreeSet<String>,
    /// The pieces the change breaks: core, platformer or mapping. Only
    /// these bump a version.
    pub breaking: BTreeSet<String>,
    /// The old `bumps:` line, from before features: read so the
    /// integrator can say it's ignored, never acted on.
    pub bumps: BTreeSet<String>,
    pub roadmap: String,
    pub log: String,
    pub plugins: String,
}

/// Reads `key: text` lines; unknown keys and other lines are ignored.
pub(crate) fn parse_note(file: &str, text: &str) -> Note {
    let mut n = Note {
        file: file.to_owned(),
        ..Note::default()
    };
    for l in text.lines() {
        let Some((k, v)) = l.split_once(':') else {
            continue;
        };
        let v = v.trim();
        let list = |v: &str| -> BTreeSet<String> { note_list(v).into_iter().collect() };
        match k.trim().to_ascii_lowercase().as_str() {
            "bumps" => n.bumps = list(v),
            "features" => n.features = list(v),
            "breaking" => n.breaking = list(v),
            "roadmap" => n.roadmap = v.to_owned(),
            "log" => n.log = v.to_owned(),
            "plugins" => n.plugins = v.to_owned(),
            _ => {}
        }
    }
    n
}

/// A list line's items in order, as `parse_note` reads them: split at
/// commas and spaces, backticks dropped, lowercased, `none` skipped.
fn note_list(v: &str) -> Vec<String> {
    v.split([',', ' '])
        .map(|x| x.trim().trim_matches('`').to_ascii_lowercase())
        .filter(|x| !x.is_empty() && x != "none")
        .collect()
}

/// `CHANGE.md` merged from two changes made on `base`, for a three-way
/// merge that conflicts in it: each list line (`features:`, `breaking:`,
/// `bumps:`) is ours then theirs' items not in it, in order; each text line
/// (`roadmap:`, `log:`, `plugins:`) is ours with what theirs added to the
/// base's after it. A key only theirs has is appended; other lines are
/// ours.
// game-printer's gate merged CHANGE.md conflicts with this; gameprinter's
// gates don't read CHANGE.md yet, so only the tests call it.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn merge_notes(base: &str, ours: &str, theirs: &str) -> String {
    let lines = |text: &str| -> Vec<(String, String)> {
        text.lines()
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned()))
            .collect()
    };
    let find = |ls: &[(String, String)], k: &str| -> Option<String> {
        ls.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone())
    };
    let is_list = |k: &str| matches!(k, "features" | "breaking" | "bumps");
    let known = |k: &str| is_list(k) || matches!(k, "roadmap" | "log" | "plugins");
    let (b, t) = (lines(base), lines(theirs));
    let merged = |k: &str, ov: &str| -> String {
        let Some(tv) = find(&t, k) else {
            return ov.to_owned();
        };
        if is_list(k) {
            let mut items = note_list(ov);
            for x in note_list(&tv) {
                if !items.contains(&x) {
                    items.push(x);
                }
            }
            return items.join(", ");
        }
        let bv = find(&b, k).unwrap_or_default();
        let added = tv.strip_prefix(bv.as_str()).unwrap_or(&tv).trim();
        if added.is_empty() || ov.contains(added) {
            ov.to_owned()
        } else if ov.is_empty() {
            added.to_owned()
        } else {
            format!("{ov} {added}")
        }
    };
    let mut out = String::new();
    let mut seen = BTreeSet::new();
    for l in ours.lines() {
        match l.split_once(':') {
            Some((k, v))
                if known(&k.trim().to_ascii_lowercase())
                    && !seen.contains(&k.trim().to_ascii_lowercase()) =>
            {
                let key = k.trim().to_ascii_lowercase();
                out.push_str(&format!("{}: {}\n", k.trim(), merged(&key, v.trim())));
                seen.insert(key);
            }
            _ => {
                out.push_str(l);
                out.push('\n');
            }
        }
    }
    for l in theirs.lines() {
        if let Some((k, v)) = l.split_once(':') {
            let key = k.trim().to_ascii_lowercase();
            if known(&key) && seen.insert(key) {
                out.push_str(&format!("{}: {}\n", k.trim(), v.trim()));
            }
        }
    }
    out
}

fn git(dir: &Path, args: &[&str]) -> anyhow::Result<String> {
    let o = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("running git {}", args.join(" ")))?;
    if !o.status.success() {
        bail!(
            "git {} failed: {}{}",
            args.join(" "),
            String::from_utf8_lossy(&o.stderr).trim(),
            String::from_utf8_lossy(&o.stdout).trim()
        );
    }
    Ok(String::from_utf8_lossy(&o.stdout).trim().to_owned())
}

fn git_succeeds(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// The approved attempts not on main (read from git, never a stored
/// flag), in the order they were kept. Of
/// the attempts on one branch whose commits are squashed (each holds the
/// whole change so far), only the latest counts.
fn picks(repo: &Path, home: &Path, skip: &[u64]) -> anyhow::Result<(Vec<Pick>, Vec<String>)> {
    let file = work::approved_file(home);
    let v: Value = serde_json::from_slice(
        &fs::read(&file).with_context(|| format!("reading {}", file.display()))?,
    )?;
    let mut left: Vec<String> = Vec::new();
    let mut all: Vec<(Option<String>, Pick, bool)> = Vec::new();
    for e in v["approved"].as_array().cloned().unwrap_or_default() {
        let n = e["attempt"].as_u64().unwrap_or(0);
        // On main is read from git: the commit, or an equal patch, is there.
        // An old approval with a `merged` flag git can't place counts too.
        if work::entry_on_main_in(repo, &e) {
            left.push(format!("attempt {n}: already on main"));
            continue;
        }
        let Some(commit) = e["commit"].as_str().map(str::to_owned) else {
            left.push(format!("attempt {n}: no commit recorded"));
            continue;
        };
        if skip.contains(&n) {
            left.push(format!("attempt {n}: skipped as asked"));
            continue;
        }
        if !git_succeeds(repo, &["cat-file", "-e", &format!("{commit}^{{commit}}")]) {
            left.push(format!("attempt {n}: commit {commit} isn't in this repo"));
            continue;
        }
        if git_succeeds(repo, &["merge-base", "--is-ancestor", &commit, "main"]) {
            left.push(format!("attempt {n}: already on main"));
            continue;
        }
        let cherry = git(repo, &["cherry", "main", &commit]).unwrap_or_default();
        if cherry
            .lines()
            .any(|l| l.starts_with("- ") && l.ends_with(&commit))
        {
            left.push(format!("attempt {n}: an equal change is on main"));
            continue;
        }
        let squash = e["squash"].as_bool().unwrap_or(false);
        let key = squash.then(|| {
            format!(
                "{}|{}",
                e["worktree"].as_str().unwrap_or(""),
                e["branch"].as_str().unwrap_or("")
            )
        });
        all.push((
            key,
            Pick {
                run: e["run"].as_str().unwrap_or("").to_owned(),
                attempt: n,
                clip: e["clip"].as_str().unwrap_or("").to_owned(),
                title: e["title"].as_str().unwrap_or("").to_owned(),
                commit,
                approved_ms: e["approved_ms"].as_u64().unwrap_or(0),
            },
            squash,
        ));
    }
    let mut out: Vec<Pick> = Vec::new();
    for (i, (key, p, _)) in all.iter().enumerate() {
        if let Some(k) = key
            && let Some((_, later, _)) = all
                .iter()
                .enumerate()
                .filter(|(j, (k2, ..))| *j != i && k2.as_ref() == Some(k))
                .map(|(_, x)| x)
                .find(|(_, q, _)| (q.approved_ms, q.attempt) > (p.approved_ms, p.attempt))
        {
            left.push(format!(
                "attempt {}: inside attempt {}, which is on the same branch",
                p.attempt, later.attempt
            ));
            continue;
        }
        out.push(p.clone());
    }
    out.sort_by_key(|p| (p.approved_ms, p.attempt));
    Ok((out, left))
}

/// Replaces `from` with `to` in a file's text, once per match.
fn replace_in(repo: &Path, rel: &str, f: impl Fn(&str) -> String, touched: &mut Vec<String>) {
    let p = repo.join(rel);
    let Ok(text) = fs::read_to_string(&p) else {
        return;
    };
    let new = f(&text);
    if new != text && fs::write(&p, new).is_ok() {
        touched.push(rel.to_owned());
    }
}

/// `pub const NAME: ty = "x.y.z";` or `= N;` in a file: its value.
fn const_value(repo: &Path, rel: &str, decl: &str) -> Option<String> {
    let text = fs::read_to_string(repo.join(rel)).ok()?;
    let line = text.lines().find(|l| l.trim_start().starts_with(decl))?;
    let v = line.split_once('=')?.1.trim().trim_end_matches(';').trim();
    Some(v.trim_matches('"').to_owned())
}

/// The version after a breaking change: the minor while the major is 0
/// ("0.4" -> "0.5"), else the major. A patch from before features is
/// dropped.
fn next_breaking(v: &str) -> anyhow::Result<String> {
    let mut it = v.split('.').map(|p| p.parse::<u32>());
    match (it.next(), it.next()) {
        (Some(Ok(0)), Some(Ok(b))) => Ok(format!("0.{}", b + 1)),
        (Some(Ok(a)), Some(Ok(_))) => Ok(format!("{}.0", a + 1)),
        _ => bail!("{v:?} isn't a version x.y"),
    }
}

/// A piece with a version in a Rust constant and the lines that name it.
/// The state version isn't one: it's computed from the layout
/// (`gp_sim::layout`).
struct Piece {
    key: &'static str,
    /// The constant's file and the start of its declaration.
    decl: (&'static str, &'static str),
    /// Constants and strings that name the version: (file, before, after
    /// the number).
    spots: &'static [(&'static str, &'static str, &'static str)],
    integer: bool,
}

const PIECES: &[Piece] = &[
    Piece {
        key: "core",
        decl: ("crates/gp-model/src/model.rs", "pub const CORE_VERSION"),
        spots: &[
            (
                "crates/gp-model/src/model.rs",
                "CORE_VERSION: &str = \"",
                "\"",
            ),
            ("crates/gp-model/src/model.rs", "model core schema ", "\")]"),
            (
                "crates/gp-sim/src/mechanic.rs",
                "CORE_VERSION: &str = \"",
                "\"",
            ),
            (
                "crates/gp-model/src/tests.rs",
                "game-printer:model-core@",
                "\"",
            ),
        ],
        integer: false,
    },
    Piece {
        key: "platformer",
        decl: ("crates/gp-model/src/platformer.rs", "pub const VERSION"),
        spots: &[
            (
                "crates/gp-model/src/platformer.rs",
                "VERSION: &str = \"",
                "\"",
            ),
            (
                "crates/gp-model/src/platformer.rs",
                "platformer = \"",
                "\" }",
            ),
            (
                "crates/gp-sim/src/mechanic.rs",
                "PLATFORMER_VERSION: &str = \"",
                "\"",
            ),
            (
                "crates/gp-model/src/tests.rs",
                "game-printer:platformer@",
                "\"",
            ),
        ],
        integer: false,
    },
    Piece {
        key: "mapping",
        decl: ("crates/gp-interp/src/mapping.rs", "pub const SCHEMA"),
        spots: &[("crates/gp-interp/src/mapping.rs", "SCHEMA: u32 = ", ";")],
        integer: true,
    },
];

/// Sets `<before>OLD<after>` to `<before>NEW<after>` wherever it is.
fn swap(text: &str, before: &str, old: &str, after: &str, new: &str) -> String {
    text.replace(
        &format!("{before}{old}{after}"),
        &format!("{before}{new}{after}"),
    )
}

/// Adds `new` to the `READS` list of the mapping schemas a build reads.
fn add_read(text: &str, new: &str) -> String {
    let Some(at) = text.find("pub const READS: [u32; ") else {
        return text.to_owned();
    };
    let Some(end) = text[at..].find("];").map(|i| at + i) else {
        return text.to_owned();
    };
    let line = &text[at..end];
    let Some((head, list)) = line.split_once("] = [") else {
        return text.to_owned();
    };
    let Some((decl, count)) = head.rsplit_once("; ") else {
        return text.to_owned();
    };
    if list.split(", ").any(|v| v == new) {
        return text.to_owned();
    }
    let n: usize = count.parse().unwrap_or(0) + 1;
    format!(
        "{}{decl}; {n}] = [{list}, {new}{}",
        &text[..at],
        &text[end..]
    )
}

/// The version a piece goes to, and what changed in the files.
fn bump(repo: &Path, p: &Piece, touched: &mut Vec<String>) -> anyhow::Result<(String, String)> {
    let old = const_value(repo, p.decl.0, p.decl.1)
        .with_context(|| format!("no `{}` in {}", p.decl.1, p.decl.0))?;
    let new = if p.integer {
        (old.parse::<u64>()
            .with_context(|| format!("{old:?} isn't a number"))?
            + 1)
        .to_string()
    } else {
        next_breaking(&old)?
    };
    for (file, before, after) in p.spots {
        replace_in(repo, file, |t| swap(t, before, &old, after, &new), touched);
    }
    if p.key == "mapping" {
        replace_in(
            repo,
            "crates/gp-interp/src/mapping.rs",
            |t| add_read(t, &new),
            touched,
        );
    }
    Ok((old, new))
}

/// Writes every format's schema into schema/ with `gp self schemas`.
fn write_schemas(repo: &Path, gp: &Path, touched: &mut Vec<String>) -> anyhow::Result<()> {
    if !repo.join("schema").is_dir() {
        return Ok(());
    }
    let o = Command::new(gp)
        .current_dir(repo)
        .args(["--json", "self", "schemas", "schema"])
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("running {}", gp.display()))?;
    if !o.status.success() {
        bail!(
            "`gp self schemas schema` failed: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        );
    }
    let v: Value = serde_json::from_slice(&o.stdout).unwrap_or_default();
    for f in v["formats"].as_array().into_iter().flatten() {
        if let Some(name) = f["file"].as_str() {
            touched.push(format!("schema/{name}"));
        }
    }
    Ok(())
}

/// The registries a feature name is listed in.
const REGISTRIES: &[&str] = &[
    "crates/gp-model/src/feature_list.rs",
    "crates/gp-interp/src/mapping_features.rs",
];

/// The features the notes add: a name two notes both add, or one that
/// isn't a feature name, is an error; one no registry on the branch lists
/// is a line to read.
fn check_features(
    repo: &Path,
    notes: &[Note],
    lines: &mut Vec<String>,
) -> anyhow::Result<Vec<String>> {
    let mut seen: Vec<(String, String)> = Vec::new();
    for n in notes {
        for f in &n.features {
            if !gp_model::features::is_name(f) {
                bail!(
                    "{} names {f:?}, which isn't a feature name (lowercase words joined by `.` and `_`)",
                    n.file
                );
            }
            if let Some((_, other)) = seen.iter().find(|(x, _)| x == f) {
                bail!(
                    "{} and {other} both add the feature `{f}`: one of them takes another name",
                    n.file
                );
            }
            seen.push((f.clone(), n.file.clone()));
        }
        if !n.bumps.is_empty() && n.breaking.is_empty() {
            lines.push(format!(
                "{}: `bumps:` is ignored; an option is a feature (`features:`), and a version bumps only for `breaking:`.",
                n.file
            ));
        }
    }
    let listed: String = REGISTRIES
        .iter()
        .filter_map(|r| fs::read_to_string(repo.join(r)).ok())
        .collect();
    for (f, file) in &seen {
        if !listed.contains(&format!("\"{f}\"")) {
            lines.push(format!(
                "{file} adds the feature `{f}`, which no registry on the branch lists."
            ));
        }
    }
    Ok(seen.into_iter().map(|(f, _)| f).collect())
}

fn sentence(s: &str) -> String {
    s.trim().trim_end_matches('.').to_owned()
}

fn today() -> String {
    Command::new("date")
        .arg("+%F")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "undated".into())
}

pub(crate) fn run(args: &Args, paths: &Paths) -> anyhow::Result<Outcome> {
    let repo = args.repo.canonicalize()?;
    let (list, left) = picks(&repo, &paths.home, args.skip)?;
    let mut lines: Vec<String> = Vec::new();
    for l in &left {
        lines.push(format!("Left out: {l}."));
    }
    if list.is_empty() {
        lines.push("No approved attempt is waiting.".into());
        return Ok(Outcome::ok(
            json!({"picked": [], "left_out": left}),
            lines.join("\n"),
        ));
    }
    lines.push(format!(
        "Picking {} on {}: {}.",
        list.len(),
        args.branch,
        list.iter()
            .map(|p| format!("attempt {} ({})", p.attempt, p.title))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    if args.dry_run {
        return Ok(Outcome::ok(
            json!({"picked": list.iter().map(|p| p.attempt).collect::<Vec<_>>(), "left_out": left, "dry_run": true}),
            lines.join("\n"),
        ));
    }
    if !git(&repo, &["status", "--porcelain", "--untracked-files=no"])?.is_empty() {
        bail!(
            "{} has changes not committed; integrate from a clean checkout",
            repo.display()
        );
    }
    git(&repo, &["switch", "-q", "-c", args.branch, "main"])?;

    // The picks, in order, stopping at the first conflict.
    let mut done: Vec<&Pick> = Vec::new();
    let mut union_log: Vec<String> = Vec::new();
    for p in &list {
        let o = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["cherry-pick", &p.commit])
            .stdin(Stdio::null())
            .output()?;
        if o.status.success() {
            done.push(p);
            continue;
        }
        let files = git(&repo, &["diff", "--name-only", "--diff-filter=U"]).unwrap_or_default();
        if let Some(unions) = union_resolve(&repo, &files) {
            for u in &unions {
                lines.push(format!(
                    "Attempt {}: kept both sides of {} ({} places).",
                    p.attempt, u.0, u.1
                ));
            }
            let cont = Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(["cherry-pick", "--continue"])
                .env("GIT_EDITOR", "true")
                .stdin(Stdio::null())
                .output()?;
            if cont.status.success() {
                union_log.extend(
                    unions
                        .iter()
                        .map(|u| format!("{}: {} ({})", p.attempt, u.0, u.1)),
                );
                done.push(p);
                continue;
            }
        }
        let clean = git(&repo, &["status", "--porcelain", "--untracked-files=no"])
            .unwrap_or_default()
            .is_empty();
        let _ = git(&repo, &["cherry-pick", "--abort"]);
        if files.is_empty() && clean {
            let _ = git(&repo, &["reset", "-q", "--hard", "HEAD"]);
            lines.push(format!(
                "Attempt {} changes nothing on this branch (its change is already here); left out.",
                p.attempt
            ));
            continue;
        }
        lines.push(format!(
            "Stopped: attempt {} ({}, clip {}) conflicts in {}.",
            p.attempt,
            p.title,
            p.clip,
            files.lines().collect::<Vec<_>>().join(", ")
        ));
        lines.push(format!(
            "{} attempts are picked on {}. The conflict is undone. Either leave attempt {} out (`--skip {}` on a new branch) and have its agent merge main and record it again, or pick {} yourself and resolve it.",
            done.len(), args.branch, p.attempt, p.attempt, p.commit
        ));
        return Ok(Outcome {
            json: json!({"picked": done.iter().map(|d| d.attempt).collect::<Vec<_>>(), "conflict": {"attempt": p.attempt, "files": files.lines().collect::<Vec<_>>()}, "left_out": left}),
            text: lines.join("\n"),
            exit: 1,
        });
    }

    // The notes the picks brought.
    let mut notes: Vec<Note> = Vec::new();
    let changes = repo.join(work::CHANGES_DIR);
    let mut files: Vec<PathBuf> = fs::read_dir(&changes)
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    files.sort();
    for f in files
        .iter()
        .filter(|f| f.extension().is_some_and(|x| x == "md"))
    {
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        notes.push(parse_note(&name, &fs::read_to_string(f)?));
    }
    let mut touched: Vec<String> = Vec::new();
    let mut versions: Vec<(String, String)> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let features = check_features(&repo, &notes, &mut warnings)?;
    lines.extend(warnings.iter().cloned());
    let wanted: BTreeSet<&str> = notes
        .iter()
        .flat_map(|n| n.breaking.iter().map(String::as_str))
        .collect();
    for k in &wanted {
        if *k == "state" {
            lines.push(
                "`breaking: state` is ignored: the state version is computed from its layout."
                    .into(),
            );
        } else if !PIECES.iter().any(|p| p.key == *k) {
            bail!("a CHANGE.md says breaking {k:?}; the pieces are core, platformer and mapping");
        }
    }
    for p in PIECES {
        if wanted.contains(p.key) {
            let (old, new) = bump(&repo, p, &mut touched)?;
            lines.push(format!("{} (breaking): {old} -> {new}.", p.key));
            versions.push((p.key.to_owned(), new));
        }
    }
    if !features.is_empty() {
        lines.push(format!(
            "Features added: {}; no version bumps for them.",
            features
                .iter()
                .map(|f| format!("`{f}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    // The schema, from a build of the picked code. A gp built from
    // another checkout doesn't know the picked attempts' options, so
    // `--gp` is used only for a checkout with no Cargo workspace to build
    // (the tests' small stand-in repos).
    if notes.is_empty() && done.is_empty() {
        bail!("nothing to build");
    }
    let gp = match args.gp {
        Some(g) if !repo.join("Cargo.toml").is_file() => g.to_owned(),
        given => {
            if let Some(g) = given {
                lines.push(format!(
                    "Ignoring --gp {}: the schema and the model check use a gp built from this branch.",
                    g.display()
                ));
            }
            lines.push("Building gp to regenerate the schema.".into());
            work::build_gp(
                &repo,
                &repo.join("target"),
                &repo.join("target").join("integrate-build.log"),
            )?
        }
    };
    write_schemas(&repo, &gp, &mut touched)?;

    // The ROADMAP line and the change log.
    let date = args.date.map_or_else(today, str::to_owned);
    let all_labels: Vec<String> = versions
        .iter()
        .map(|(k, v)| match k.as_str() {
            "core" => format!("core {v}"),
            "platformer" => format!("platformer {v}"),
            _ => format!("mapping schema {v}"),
        })
        .collect();
    let feature_label = if features.is_empty() {
        String::new()
    } else {
        format!(
            "feature{} {}, ",
            if features.len() == 1 { "" } else { "s" },
            features
                .iter()
                .map(|f| format!("`{f}`"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let runs: BTreeSet<&str> = done.iter().map(|p| p.run.as_str()).collect();
    let ns: Vec<String> = done.iter().map(|p| p.attempt.to_string()).collect();
    let said: Vec<String> = notes
        .iter()
        .filter(|n| !n.roadmap.is_empty())
        .map(|n| sentence(&n.roadmap))
        .collect();
    let numbered = if all_labels.is_empty() {
        format!("{feature_label}{date}, {}", args.branch)
    } else {
        format!(
            "{feature_label}breaking: {}, {date}, {}",
            all_labels.join(", "),
            args.branch
        )
    };
    let road = format!(
        "- The clip agents' attempts {} on run{} {} are merged ({numbered}): {}.",
        ns.join(", "),
        if runs.len() == 1 { "" } else { "s" },
        runs.iter()
            .map(|r| format!("`{r}`"))
            .collect::<Vec<_>>()
            .join(", "),
        if said.is_empty() {
            "see the commits".to_owned()
        } else {
            said.join("; ")
        },
    );
    replace_in(
        &repo,
        "ROADMAP.md",
        |t| insert_after(t, "## Decided\n", &road),
        &mut touched,
    );
    let mut head: Vec<String> = Vec::new();
    if !features.is_empty() {
        head.push(format!(
            "Feature{} {}",
            if features.len() == 1 { "" } else { "s" },
            features
                .iter()
                .map(|f| format!("`{f}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for (k, v) in &versions {
        head.push(match k.as_str() {
            "core" => format!("Core {v} (breaking)"),
            "platformer" => format!("Platformer {v} (breaking)"),
            _ => format!("Mapping schema {v} (breaking)"),
        });
    }
    if !head.is_empty() {
        let logs: Vec<String> = notes
            .iter()
            .filter(|n| !n.log.is_empty())
            .map(|n| sentence(&n.log))
            .collect();
        let mut plugins: Vec<String> = Vec::new();
        for n in &notes {
            let t = sentence(&n.plugins);
            if !t.is_empty() && !plugins.contains(&t) {
                plugins.push(t);
            }
        }
        let row = format!(
            "| {} ({date}) | {}. | {}. |",
            head.join(", "),
            logs.join(". "),
            if plugins.is_empty() {
                "Nothing: optional and off by default".to_owned()
            } else {
                plugins.join(". ")
            }
        );
        replace_in(
            &repo,
            "docs/model.md",
            |t| match t.find("## Changes") {
                Some(i) => format!(
                    "{}{}",
                    &t[..i],
                    insert_after(&t[i..], "|---|---|---|\n", &row)
                ),
                None => t.to_owned(),
            },
            &mut touched,
        );
    }

    // Stage what was touched, remove the notes, commit.
    touched.sort();
    touched.dedup();
    for t in &touched {
        git(&repo, &["add", "--", t])?;
    }
    for f in files
        .iter()
        .filter(|f| f.extension().is_some_and(|x| x == "md"))
    {
        let rel = f
            .strip_prefix(&repo)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        git(&repo, &["rm", "-q", "--", &rel])?;
    }
    if !git(&repo, &["status", "--porcelain", "--untracked-files=no"])?.is_empty() {
        let msg = format!(
            "Integrate approved attempts {}{}",
            ns.join(", "),
            if all_labels.is_empty() {
                String::new()
            } else {
                format!(": {}", all_labels.join(", "))
            }
        );
        git(&repo, &["commit", "-q", "-m", &msg])?;
    }

    // gp model check on every game of the games checkout.
    let mut failed: Vec<String> = Vec::new();
    let mut checked: Vec<String> = Vec::new();
    let games = crate::roms_cmd::games_dir(args.games)
        .or_else(|| Some(repo.parent()?.join("games")).filter(|g| g.is_dir()));
    if let Some(Ok(d)) = games.as_ref().map(fs::read_dir) {
        let mut dirs: Vec<PathBuf> = d
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.join("model.toml").is_file())
            .collect();
        dirs.sort();
        for t in dirs {
            let name = t.file_name().unwrap().to_string_lossy().into_owned();
            let o = Command::new(&gp)
                .current_dir(&repo)
                .args(["model", "check"])
                .arg(&t)
                .stdin(Stdio::null())
                .output()?;
            checked.push(name.clone());
            if !o.status.success() {
                failed.push(format!(
                    "{name}: {}",
                    String::from_utf8_lossy(&o.stdout)
                        .lines()
                        .chain(String::from_utf8_lossy(&o.stderr).lines())
                        .take(6)
                        .collect::<Vec<_>>()
                        .join(" | ")
                ));
            }
        }
    }
    if failed.is_empty() {
        lines.push(format!(
            "gp model check passes on {} game(s).",
            checked.len()
        ));
    } else {
        for f in &failed {
            lines.push(format!("gp model check fails: {f}"));
        }
    }
    lines.push(format!(
        "Branch {} holds {} attempt(s). Run `just check` there and land it through the tool gate.",
        args.branch,
        done.len()
    ));
    let mut exit = u8::from(!failed.is_empty());
    let mut merge_check = Value::Null;
    if args.check {
        if failed.is_empty() {
            let o = Command::new("just")
                .arg("check")
                .current_dir(&repo)
                .stdin(Stdio::null())
                .output()?;
            let out = if o.status.success() {
                format!("just check passes on {}.", args.branch)
            } else {
                format!(
                    "just check fails on {}: {}",
                    args.branch,
                    String::from_utf8_lossy(&o.stderr)
                        .lines()
                        .rev()
                        .take(6)
                        .collect::<Vec<_>>()
                        .join(" | ")
                )
            };
            merge_check = json!({"passed": o.status.success(), "says": out});
            lines.push(out);
            if !o.status.success() {
                exit = 1;
            }
        } else {
            lines.push("just check skipped: gp model check failed.".into());
        }
    }
    Ok(Outcome {
        json: json!({
            "merge_check": merge_check,
            "branch": args.branch,
            "picked": done.iter().map(|d| d.attempt).collect::<Vec<_>>(),
            "left_out": left,
            "unions": union_log,
            "features": features,
            "warnings": warnings,
            "versions": versions.iter().map(|(k, v)| (k.clone(), json!(v))).collect::<serde_json::Map<_, _>>(),
            "notes": notes.iter().map(|n| n.file.clone()).collect::<Vec<_>>(),
            "model_check": {"checked": checked, "failed": failed},
        }),
        text: lines.join("\n"),
        exit,
    })
}

/// Docs that only ever get lines appended. A conflict in one of these is
/// resolved by keeping both sides. docs/model.md counts only when every
/// conflicting line is a table row (its change log).
const APPEND_ONLY: &[(&str, bool)] = &[
    ("ROADMAP.md", false),
    ("docs/research/deterministic-inference.md", false),
    ("docs/research/README.md", false),
    ("docs/open-questions.md", false),
    ("docs/model.md", true),
];

/// Keeps ours, then theirs, in every conflict hunk. None when a marker is
/// malformed or, with `table_only`, a hunk line isn't a table row.
fn union_text(text: &str, table_only: bool) -> Option<(String, usize)> {
    let mut out = String::new();
    let mut hunks = 0;
    // 0 outside, 1 ours, 2 base, 3 theirs
    let mut state = 0;
    for line in text.split_inclusive('\n') {
        let t = line.trim_end_matches(['\n', '\r']);
        if t.starts_with("<<<<<<< ") && state == 0 {
            state = 1;
            hunks += 1;
        } else if t.starts_with("||||||| ") && state == 1 {
            state = 2;
        } else if t == "=======" && (state == 1 || state == 2) {
            state = 3;
        } else if t.starts_with(">>>>>>> ") && state == 3 {
            state = 0;
        } else if state == 2 {
            // the base side is dropped
        } else {
            if state != 0 && table_only && !t.trim_start().starts_with('|') {
                return None;
            }
            out.push_str(line);
            if state != 0 && !line.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    (state == 0 && hunks > 0).then_some((out, hunks))
}

/// When every conflicting file is append-only or a generated schema,
/// writes the union of both sides (or, for a schema, theirs, since every
/// schema is written again after the picks) and stages it. Returns each
/// file and its hunk count; None (with nothing changed) when any file is
/// not on the list or cannot be unioned.
fn union_resolve(repo: &Path, files: &str) -> Option<Vec<(String, usize)>> {
    let names: Vec<&str> = files.lines().filter(|l| !l.is_empty()).collect();
    if names.is_empty() {
        return None;
    }
    let mut results = Vec::new();
    let mut generated = Vec::new();
    for n in &names {
        // A generated schema: take either side, since every schema is
        // written again from the picked code after the picks.
        if n.starts_with("schema/") && n.ends_with(".json") {
            generated.push(*n);
            continue;
        }
        let (_, table_only) = APPEND_ONLY.iter().find(|(f, _)| f == n)?;
        let text = fs::read_to_string(repo.join(n)).ok()?;
        let (merged, hunks) = union_text(&text, *table_only)?;
        results.push((n.to_string(), merged, hunks));
    }
    for (n, merged, _) in &results {
        fs::write(repo.join(n), merged).ok()?;
        git(repo, &["add", n]).ok()?;
    }
    for n in &generated {
        git(repo, &["checkout", "--theirs", "--", n]).ok()?;
        git(repo, &["add", n]).ok()?;
    }
    Some(
        results
            .into_iter()
            .map(|(n, _, h)| (n, h))
            .chain(generated.iter().map(|n| (n.to_string(), 0)))
            .collect(),
    )
}

/// Inserts `line` as the first entry after the first `marker` (and a blank
/// line for a heading, not for a table header).
fn insert_after(text: &str, marker: &str, line: &str) -> String {
    let Some(i) = text.find(marker) else {
        return text.to_owned();
    };
    let at = i + marker.len();
    let rest = &text[at..];
    if marker.starts_with('#') {
        // "## Decided\n\n- ..." : after the blank line.
        let skip = rest.len() - rest.trim_start_matches('\n').len();
        let at = at + skip;
        return format!("{}{line}\n{}", &text[..at], &text[at..]);
    }
    format!("{}{line}\n{}", &text[..at], &text[at..])
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_new_mapping_schema_joins_the_reads_list() {
        let t = "x\npub const READS: [u32; 3] = [1, 2, 3];\ny\n";
        let u = super::add_read(t, "4");
        assert_eq!(u, "x\npub const READS: [u32; 4] = [1, 2, 3, 4];\ny\n");
        assert_eq!(super::add_read(&u, "4"), u);
    }

    use super::*;

    #[test]
    fn two_notes_on_one_base_merge_by_key() {
        let base =
            "features: a.one, a.two\nroadmap: Did one.\nlog: Added a.one.\nplugins: Nothing.\n";
        let ours = "features: a.one, a.two, b.ours\nroadmap: Did one. Did ours.\nlog: Added a.one.\nplugins: Nothing.\n";
        let theirs = "features: a.one, a.two, `c.theirs`, b.ours\nbreaking: mapping\nroadmap: Did one. Did theirs.\nlog: Added a.one. Added c.theirs.\nplugins: Nothing.\n";
        assert_eq!(
            merge_notes(base, ours, theirs),
            "features: a.one, a.two, b.ours, c.theirs\nroadmap: Did one. Did ours. Did theirs.\nlog: Added a.one. Added c.theirs.\nplugins: Nothing.\nbreaking: mapping\n"
        );
    }

    #[test]
    fn a_note_is_key_lines() {
        let n = parse_note(
            "r-3.md",
            "features: shoot.in_slots, `shoot.turn`\nbreaking: mapping\nbumps: platformer, state\nroadmap: shots in slots.\nlog: Added `shoot.in_slots`.\nplugins: Nothing.\nthe rest is ignored\n",
        );
        assert_eq!(
            n.features.iter().collect::<Vec<_>>(),
            ["shoot.in_slots", "shoot.turn"]
        );
        assert_eq!(n.breaking.iter().collect::<Vec<_>>(), ["mapping"]);
        assert_eq!(n.bumps.iter().collect::<Vec<_>>(), ["platformer", "state"]);
        assert_eq!(n.roadmap, "shots in slots.");
        assert_eq!(n.plugins, "Nothing.");
        assert!(parse_note("x", "bumps: none").bumps.is_empty());
    }

    #[test]
    fn versions_count_up_and_lines_are_inserted_after_their_marker() {
        assert_eq!(next_breaking("0.4").unwrap(), "0.5");
        assert_eq!(next_breaking("0.4.23").unwrap(), "0.5");
        assert_eq!(next_breaking("1.2").unwrap(), "2.0");
        assert!(next_breaking("15").is_err());
        let t = "# R\n\n## Decided\n\n- old\n";
        assert_eq!(
            insert_after(t, "## Decided\n", "- new"),
            "# R\n\n## Decided\n\n- new\n- old\n"
        );
        let m = "| V | C | P |\n|---|---|---|\n| old | a | b |\n";
        assert!(
            insert_after(m, "|---|---|---|\n", "| new |").contains("|---|---|---|\n| new |\n| old")
        );
    }

    #[test]
    fn two_notes_adding_one_feature_collide_and_bumps_is_ignored() {
        let dir = std::env::temp_dir();
        let a = parse_note("a.md", "features: rope.swing\nbumps: platformer\n");
        let b = parse_note("b.md", "features: rope.swing\n");
        let mut lines = Vec::new();
        let err = check_features(&dir, &[a.clone(), b], &mut lines).unwrap_err();
        assert!(err.to_string().contains("`rope.swing`"), "{err}");
        let mut lines = Vec::new();
        let got = check_features(&dir, &[a], &mut lines).unwrap();
        assert_eq!(got, ["rope.swing"]);
        assert!(
            lines.iter().any(|l| l.contains("`bumps:` is ignored")),
            "{lines:?}"
        );
        assert!(lines.iter().any(|l| l.contains("no registry")), "{lines:?}");
        let bad = parse_note("c.md", "features: 1up\n");
        assert!(check_features(&dir, &[bad], &mut Vec::new()).is_err());
    }

    #[test]
    fn a_union_keeps_both_sides_and_refuses_other_text() {
        let t = "a\n<<<<<<< HEAD\n| one |\n=======\n| two |\n>>>>>>> x\nb\n";
        assert_eq!(
            union_text(t, false).unwrap(),
            ("a\n| one |\n| two |\nb\n".into(), 1)
        );
        assert!(union_text(t, true).is_some());
        let prose = "<<<<<<< HEAD\nfoo\n=======\nbar\n>>>>>>> x\n";
        assert!(union_text(prose, true).is_none());
        assert!(union_text("<<<<<<< HEAD\nfoo\n", false).is_none());
    }

    #[test]
    fn two_attempts_appending_at_one_place_in_the_roadmap_integrate() {
        let dir = std::env::temp_dir().join(format!("gp-union-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let g = |a: &[&str]| git(&dir, a).unwrap();
        g(&["init", "-q", "-b", "main"]);
        g(&["config", "user.email", "t@t"]);
        g(&["config", "user.name", "t"]);
        fs::write(dir.join("ROADMAP.md"), "# R\n\n| a |\n|---|\n| first |\n").unwrap();
        g(&["add", "."]);
        g(&["commit", "-q", "-m", "base"]);
        for (b, row) in [("one", "| moss |"), ("two", "| hop |")] {
            g(&["switch", "-q", "-c", b, "main"]);
            let mut t = fs::read_to_string(dir.join("ROADMAP.md")).unwrap();
            t.push_str(row);
            t.push('\n');
            fs::write(dir.join("ROADMAP.md"), t).unwrap();
            g(&["commit", "-qam", b]);
        }
        g(&["switch", "-q", "-c", "int", "main"]);
        g(&["cherry-pick", "one"]);
        let two = g(&["rev-parse", "two"]);
        assert!(git(&dir, &["cherry-pick", &two]).is_err());
        let files = g(&["diff", "--name-only", "--diff-filter=U"]);
        let u = union_resolve(&dir, &files).unwrap();
        assert_eq!(u, [("ROADMAP.md".to_owned(), 1)]);
        let ok = Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(["cherry-pick", "--continue"])
            .env("GIT_EDITOR", "true")
            .output()
            .unwrap();
        assert!(ok.status.success());
        let t = fs::read_to_string(dir.join("ROADMAP.md")).unwrap();
        assert!(t.ends_with("| first |\n| moss |\n| hop |\n"), "{t}");
    }
}
