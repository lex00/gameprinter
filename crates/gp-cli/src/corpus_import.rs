//! `gp corpus import GAME`: a game's mechanic sessions
//! (`$GP_HOME/mechanics/GAME/`) as the clips of its corpus plan
//! (docs/formats/scoring-corpus.md, "Import"). Each clip of a session is
//! written as a recipe: its presses from power-on to its start in
//! `inputs/ID.start.presses`, and its own presses in `inputs/ID.presses`.
//! The version is recorded by the corpus's rules.
//!
//! | Left out | How it is known |
//! |---|---|
//! | Sealed runs | A session named `sealed*`; a clip whose inputs are a file's under `$GP_HOME/sealed/GAME/`, or that plays a sealed run's own presses |
//! | Rows out of scope | `scope = "out"` on the quest log's row |
//! | Anything else | `[import] exclude` in corpus.toml, a session or a clip by name, with why |
//!
//! A clip already in the plan whose recipe is unchanged is kept as it is,
//! with its name, so an import changes only what the sessions changed.
//!
//! An import removes only the clips it is told to (`--remove ID`, or a
//! pattern with `*`). A clip whose session isn't in the home, or that the
//! sessions now leave out, stays in the plan as it is.
//!
//! `--write` edits corpus.toml in place ([`edit_in_place`]): new clips
//! after the last clip, changed and removed clips' blocks only, every other
//! byte kept; then the version and the recipe-set hash, with a changelog
//! entry. `--session NAME` imports only the named sessions.
//!
//! Ported from game-printer's corpus_import.rs, which wrote an older
//! `clips.toml` and measured new clips' curves in a capture run (`--run`).
//! A plan keeps no curves (they are derived from the ROM when the corpus is
//! scored), so there is no `--run`, and the version is recorded on the same
//! `--write`. A folder with an older `clips.toml` and no plan is refused.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use gp_evidence::Paths;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::Outcome;
use crate::clipset::{self as cs, Recipe};
use crate::corpus_score::FILE;

/// What `gp corpus import` was asked.
pub(crate) struct Args<'a> {
    pub game: &'a str,
    /// The game's folder (default: GAME in the games checkout).
    pub template: Option<&'a Path>,
    /// The home holding the sessions, their paths and the sealed runs
    /// (default: `$GP_HOME`).
    pub from: Option<&'a Path>,
    pub write: bool,
    pub says: Option<&'a str>,
    pub date: Option<&'a str>,
    /// The clips to remove: IDs, or patterns with `*`. No other clip is
    /// removed.
    pub remove: &'a [String],
    /// The sessions to import (`--session`); empty: every session. A clip
    /// of a session not named is left as it is.
    pub sessions: &'a [String],
}

/// The plan's `[import]` table: how the import names and leaves out clips.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ImportRules {
    /// The clip IDs' first part: `dk` in `dk.walk.001`. Every clip whose ID
    /// starts with it and a dot, and is `played`, is the import's to add,
    /// change or remove. Blank: the game's name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prefix: String,
    /// A new clip's `why`, with `{session}` for its session's name. Blank:
    /// "a clip of the {session} mechanic session".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
    /// Sessions (`title`) and clips (`walk.006`) left out by name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<Excluded>,
}

/// A session or a clip `gp corpus import` leaves out, and why.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Excluded {
    /// A session (`title`) or a session's clip (`walk.006`).
    pub what: String,
    /// Why, in a few words.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
}

/// Frames and their spans.
type Spans = (u64, Vec<String>);

/// One clip a session gives.
#[derive(Debug, Clone, PartialEq)]
struct Taken {
    id: String,
    session: String,
    n: u64,
    note: String,
    /// Frames from power-on to the start, and their spans.
    lead: Spans,
    /// The clip's own frames and spans.
    body: Spans,
}

/// A session or clip left out, and why.
#[derive(Debug, Clone, PartialEq)]
struct Left {
    what: String,
    why: String,
}

/// The game's quest log rows marked out of scope, with why.
fn out_of_scope(home: &Path, game: &str) -> BTreeMap<String, String> {
    let rows: Vec<serde_json::Value> =
        fs::read_to_string(home.join("mechanics").join(game).join("queue.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
    rows.iter()
        .filter(|r| r["scope"] == json!("out"))
        .filter_map(|r| {
            Some((
                r["mechanic"].as_str()?.to_owned(),
                r["scope_why"].as_str().unwrap_or("out of scope").to_owned(),
            ))
        })
        .collect()
}

/// Every sealed run's inputs from power-on, by hash: the logs under
/// `$GP_HOME/sealed/GAME/` and the page's sealed clips, each as a full log.
fn sealed_inputs(home: &Path, game: &str) -> HashSet<String> {
    let top = home.join("sealed").join(game);
    let mut out = HashSet::new();
    for e in fs::read_dir(&top).into_iter().flatten().flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "log")
            && let Ok(t) = fs::read_to_string(&p)
            && let Ok(l) = gp_evidence::InputLog::parse(&t)
        {
            out.insert(frames_key(&l.frames));
        }
    }
    let dir = top.join(crate::sealed::SEALED_ROW);
    for n in clip_numbers(&dir) {
        if let Ok((t, _)) = crate::mechanic::paths::full_log(home, &dir, n)
            && let Ok(l) = gp_evidence::InputLog::parse(&t)
        {
            out.insert(frames_key(&l.frames));
        }
    }
    out
}

fn frames_key(f: &[gp_evidence::InputFrame]) -> String {
    let mut h = blake3::Hasher::new();
    for x in f {
        h.update(&x.pads);
        h.update(&[u8::from(x.reset)]);
    }
    h.finalize().to_hex().to_string()
}

/// A session's clip numbers, from its `clips/NNN.toml` files, in order.
fn clip_numbers(dir: &Path) -> Vec<u64> {
    let mut v: Vec<u64> = fs::read_dir(dir.join("clips"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            name.strip_suffix(".toml")?.parse().ok()
        })
        .collect();
    v.sort_unstable();
    v
}

/// The clips the sessions give, and what was left out.
fn take(
    home: &Path,
    game: &str,
    prefix: &str,
    rules: &ImportRules,
    only: &[String],
) -> Result<(Vec<Taken>, Vec<Left>)> {
    let root = home.join("mechanics").join(game);
    if !root.is_dir() {
        bail!("no mechanic sessions for {game} in {}", root.display());
    }
    for s in only {
        if !root.join(s).join("clips").is_dir() {
            bail!("no session {s} with clips in {}", root.display());
        }
    }
    let excluded: BTreeMap<&str, &str> = rules
        .exclude
        .iter()
        .map(|e| (e.what.as_str(), e.why.as_str()))
        .collect();
    let scope = out_of_scope(home, game);
    let sealed_keys = crate::sealed::sealed_logs(home, game);
    let sealed_full = sealed_inputs(home, game);
    // Any clip that plays a sealed run's own frames, a part of one as well
    // as a copy.
    let sealed_runs = crate::sealed::runs(home, game);
    let mut sessions: Vec<String> = fs::read_dir(&root)?
        .flatten()
        .filter(|e| e.path().join("clips").is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|s| only.is_empty() || only.contains(s))
        .collect();
    sessions.sort();
    let (mut taken, mut left) = (Vec::new(), Vec::new());
    let why = |w: &str, d: &str| {
        if w.is_empty() {
            d.to_owned()
        } else {
            w.to_owned()
        }
    };
    for s in sessions {
        if s.starts_with("sealed") {
            left.push(Left {
                what: s,
                why: "a sealed run's session".into(),
            });
            continue;
        }
        if let Some(w) = scope.get(&s) {
            left.push(Left {
                what: s.clone(),
                why: format!("out of scope in the quest log: {w}"),
            });
            continue;
        }
        if let Some(w) = excluded.get(s.as_str()) {
            left.push(Left {
                what: s.clone(),
                why: why(w, "excluded by name"),
            });
            continue;
        }
        let dir = root.join(&s);
        for n in clip_numbers(&dir) {
            let what = format!("{s}.{n:03}");
            if let Some(w) = excluded.get(what.as_str()) {
                left.push(Left {
                    what,
                    why: why(w, "excluded by name"),
                });
                continue;
            }
            if crate::sealed::clip_key(&dir.join(format!("clips/{n:03}.log")))
                .is_some_and(|k| sealed_keys.contains(&k))
            {
                left.push(Left {
                    what,
                    why: "a sealed run's inputs".into(),
                });
                continue;
            }
            let meta: toml::Table = fs::read_to_string(dir.join(format!("clips/{n:03}.toml")))
                .ok()
                .and_then(|t| toml::from_str(&t).ok())
                .unwrap_or_default();
            let (log, start) = match crate::mechanic::paths::full_log(home, &dir, n) {
                Ok(x) => x,
                Err(e) => {
                    left.push(Left {
                        what,
                        why: format!("no input from power-on: {e:#}"),
                    });
                    continue;
                }
            };
            let log =
                gp_evidence::InputLog::parse(&log).map_err(|e| anyhow::anyhow!("{what}: {e}"))?;
            if sealed_full.contains(&frames_key(&log.frames)) {
                left.push(Left {
                    what,
                    why: "a sealed run's inputs".into(),
                });
                continue;
            }
            if crate::sealed::refuse_clip_in(home, &sealed_runs, &dir, n).is_err() {
                left.push(Left {
                    what,
                    why: "plays a sealed run's inputs".into(),
                });
                continue;
            }
            let start = start.min(log.frames.len());
            let own = log.frames.len() - start;
            let frames = meta
                .get("frames")
                .and_then(toml::Value::as_integer)
                .and_then(|f| usize::try_from(f).ok())
                .map_or(own, |f| f.min(own));
            if frames == 0 {
                left.push(Left {
                    what,
                    why: "no frames".into(),
                });
                continue;
            }
            let spans = |f: &[gp_evidence::InputFrame]| gp_evidence::input_log::spans_of(f);
            let (Some(lead), Some(body)) = (
                spans(&log.frames[..start]),
                spans(&log.frames[start..start + frames]),
            ) else {
                left.push(Left {
                    what,
                    why: "presses reset or a third or fourth controller".into(),
                });
                continue;
            };
            taken.push(Taken {
                id: format!("{prefix}.{s}.{n:03}"),
                session: s.clone(),
                n,
                note: meta
                    .get("note")
                    .and_then(toml::Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                lead: (start as u64, lead),
                body: (frames as u64, body),
            });
        }
    }
    Ok((taken, left))
}

/// A recipe's start presses and own presses as read from the game's folder.
fn recipe_inputs(dir: &Path, r: &Recipe) -> Option<(Spans, Spans)> {
    let read = |p: &str| cs::parse_presses(&fs::read_to_string(dir.join(p)).ok()?).ok();
    let lead = read(r.start.presses_from_power_on.as_deref()?)?;
    let body = match &r.presses_file {
        Some(p) => read(p)?.1,
        None => r.presses.clone(),
    };
    Some((lead, (r.frames, body)))
}

/// Whether a plan's clip is one the import makes: `played`, its ID the
/// prefix's.
fn owned(r: &Recipe, prefix: &str) -> bool {
    r.source == "played" && r.id.starts_with(&format!("{prefix}."))
}

/// The session an owned clip's ID names: `walk` in `dk.walk.001`.
fn session_of<'a>(id: &'a str, prefix: &str) -> Option<&'a str> {
    id.strip_prefix(prefix)?
        .strip_prefix('.')?
        .rsplit_once('.')
        .map(|(s, _)| s)
}

/// A clip as the plan writes it.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct PlanClip {
    id: String,
    name: String,
    source: String,
    start: String,
    presses: String,
    frames: u64,
    #[serde(skip_serializing_if = "String::is_empty")]
    why: String,
}

/// The plan's clips after the import: each kept one by ID, the new or
/// changed ones' blocks, the files to write and the IDs added, changed and
/// removed.
struct Merged {
    /// The clip IDs in the plan's order after the import.
    order: Vec<String>,
    /// New and changed clips, to write.
    written: BTreeMap<String, PlanClip>,
    files: Vec<(String, String)>,
    added: Vec<String>,
    changed: Vec<String>,
    removed: Vec<String>,
    kept: usize,
    /// The plan's own clips no session gives, kept as they are.
    unsessioned: Vec<String>,
}

/// Whether `id` is named by one of the `--remove` patterns.
fn named(remove: &[String], id: &str) -> bool {
    remove.iter().any(|p| crate::gate::matches(p, id))
}

fn merge(
    dir: &Path,
    old: &[Recipe],
    prefix: &str,
    why: &str,
    taken: &[Taken],
    remove: &[String],
    only: &[String],
) -> Merged {
    let by_id: BTreeMap<&str, &Taken> = taken.iter().map(|t| (t.id.as_str(), t)).collect();
    let mut m = Merged {
        order: Vec::new(),
        written: BTreeMap::new(),
        files: Vec::new(),
        added: Vec::new(),
        changed: Vec::new(),
        removed: Vec::new(),
        kept: 0,
        unsessioned: Vec::new(),
    };
    let clip = |t: &Taken, files: &mut Vec<(String, String)>| -> PlanClip {
        let key = cs::file_key(&t.id);
        let lead = format!("inputs/{key}.start.presses");
        files.push((lead.clone(), cs::presses_text(t.lead.0, &t.lead.1)));
        let presses = format!("inputs/{key}.presses");
        files.push((presses.clone(), cs::presses_text(t.body.0, &t.body.1)));
        let note = if t.note.trim().is_empty() {
            t.session.as_str()
        } else {
            t.note.trim()
        };
        let name: String = format!("{} {:03}: {note}", t.session, t.n)
            .chars()
            .take(160)
            .collect();
        PlanClip {
            id: t.id.clone(),
            name,
            source: "played".into(),
            start: lead,
            presses,
            frames: t.body.0,
            why: why.replace("{session}", &t.session),
        }
    };
    let mut done = BTreeSet::new();
    for r in old {
        if named(remove, &r.id) {
            done.insert(r.id.clone());
            m.removed.push(r.id.clone());
            continue;
        }
        m.order.push(r.id.clone());
        // Another session's clip, when the import names its sessions, is
        // none of its business.
        if !owned(r, prefix)
            || (!only.is_empty()
                && !session_of(&r.id, prefix).is_some_and(|s| only.iter().any(|o| o == s)))
        {
            continue;
        }
        match by_id.get(r.id.as_str()) {
            None => m.unsessioned.push(r.id.clone()),
            Some(t) => {
                done.insert(r.id.clone());
                if recipe_inputs(dir, r) == Some((t.lead.clone(), t.body.clone())) {
                    m.kept += 1;
                } else {
                    let c = clip(t, &mut m.files);
                    m.written.insert(r.id.clone(), c);
                    m.changed.push(r.id.clone());
                }
            }
        }
    }
    // A new clip goes after the last one, so the file's other bytes stay
    // where they are.
    for t in taken {
        if done.contains(&t.id) || named(remove, &t.id) {
            continue;
        }
        let c = clip(t, &mut m.files);
        m.written.insert(t.id.clone(), c);
        m.order.push(t.id.clone());
        m.added.push(t.id.clone());
    }
    m
}

/// One `[[clip]]` block, with one newline at its end.
fn render(c: &PlanClip) -> Result<String> {
    #[derive(Serialize)]
    struct Clips<'a> {
        clip: [&'a PlanClip; 1],
    }
    let s = toml::to_string(&Clips { clip: [c] })?;
    Ok(format!("{}\n", s.trim_end()))
}

/// `text`, a plan, with the import made in place: a changed clip's block
/// written again, a removed one's block gone, and new clips after the last
/// clip (or before the first changelog entry, or at the end). Every other
/// byte stays as it was.
fn edit_in_place(text: &str, m: &Merged) -> Result<String> {
    #[derive(Deserialize)]
    struct Clips {
        clip: Vec<toml::Table>,
    }
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let tabs = crate::plan::tables(&lines);
    let new: Vec<&PlanClip> = m.added.iter().filter_map(|id| m.written.get(id)).collect();
    // Where new clips go: after the last clip block, else before the first
    // changelog entry, else at the end.
    let insert_at = tabs
        .iter()
        .filter(|(n, _, _)| n == "clip")
        .map(|(_, _, e)| *e)
        .next_back()
        .or_else(|| {
            tabs.iter()
                .find(|(n, _, _)| n == "changelog")
                .map(|(_, s, _)| *s)
        });
    let mut added = String::new();
    for c in &new {
        added.push('\n');
        added.push_str(&render(c)?);
    }
    let (mut out, mut at) = (String::new(), 0);
    let push_added = |out: &mut String, before_table: bool| {
        if added.is_empty() {
            return;
        }
        if before_table {
            // Before the changelog: the clips, then a blank line.
            out.push_str(added.trim_start_matches('\n'));
            out.push('\n');
        } else {
            out.push_str(&added);
        }
    };
    let mut inserted = false;
    for (name, from, to) in &tabs {
        if insert_at == Some(*from) && !inserted && name != "clip" {
            out.extend(lines[at..*from].iter().copied());
            at = *from;
            push_added(&mut out, true);
            inserted = true;
        }
        out.extend(lines[at..*from].iter().copied());
        at = *to;
        let block: String = lines[*from..*to].concat();
        if name == "clip" {
            let old: Clips = toml::from_str(&block).with_context(|| format!("reading {block}"))?;
            let id = old
                .clip
                .first()
                .and_then(|t| t.get("id"))
                .and_then(toml::Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if m.removed.contains(&id) {
                // A blank line beside it goes with it.
                if lines.get(at).is_some_and(|l| l.trim().is_empty()) {
                    at += 1;
                } else if out.ends_with("\n\n") {
                    out.pop();
                }
            } else if let Some(c) = m.written.get(&id).filter(|_| m.changed.contains(&id)) {
                out.push_str(&render(c)?);
            } else {
                out.push_str(&block);
            }
        } else {
            out.push_str(&block);
        }
        if insert_at == Some(*to) && !inserted && name == "clip" {
            push_added(&mut out, false);
            inserted = true;
        }
    }
    out.extend(lines[at..].iter().copied());
    if !inserted && !added.is_empty() {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&added);
    }
    Ok(out)
}

/// Whether the folder's state mapping reads and maps no field and no slot,
/// as `gp game new` writes it.
fn measures_nothing(dir: &Path) -> bool {
    fs::read_to_string(dir.join("mapping.toml"))
        .ok()
        .and_then(|t| gp_interp::mapping::Mapping::parse(&t).ok())
        .is_some_and(|m| m.fields.is_empty() && m.slots.is_none())
}

/// The game's folder: `--template`, else GAME in the games checkout.
fn game_dir(a: &Args<'_>) -> Result<PathBuf> {
    if let Some(t) = a.template {
        return Ok(t.to_owned());
    }
    let games = crate::roms_cmd::games_dir(None)
        .context("no games checkout found: give --template, or set GP_GAMES_DIR")?;
    crate::labels::game_folder(&games, a.game)
        .with_context(|| format!("no game {} in {}: give --template", a.game, games.display()))
}

pub(crate) fn run(a: &Args<'_>, paths: &Paths) -> Result<Outcome> {
    let game = a.game;
    if !crate::mechanic::name_ok(game) {
        bail!("a game name is lower-case letters, digits and dashes: {game}");
    }
    let dir = game_dir(a)?;
    if !dir.is_dir() {
        bail!("no corpus folder {}", dir.display());
    }
    let home = a.from.unwrap_or(&paths.home);
    let Some(plan) = crate::plan::load(&dir)? else {
        bail!(
            "{} has no corpus plan ({FILE} at format {}): gp corpus import writes a plan's clips",
            dir.display(),
            crate::plan::FORMAT
        );
    };
    let rules: ImportRules = match &plan.set.import {
        Some(v) => v
            .clone()
            .try_into()
            .with_context(|| format!("{}/{FILE}: [import]", dir.display()))?,
        None => ImportRules::default(),
    };
    let prefix = if rules.prefix.is_empty() {
        game.to_owned()
    } else {
        rules.prefix.clone()
    };
    let (taken, left) = take(home, game, &prefix, &rules, a.sessions)?;
    let why = if rules.why.is_empty() {
        "a clip of the {session} mechanic session"
    } else {
        rules.why.as_str()
    };
    let m = merge(
        &dir,
        &plan.set.clips,
        &prefix,
        why,
        &taken,
        a.remove,
        a.sessions,
    );
    let step = if !m.removed.is_empty() || !m.changed.is_empty() {
        "major"
    } else if !m.added.is_empty() {
        "minor"
    } else {
        "none"
    };
    let mut text: Vec<String> = Vec::new();
    for id in &m.added {
        text.push(format!("added    {id}"));
    }
    for id in &m.changed {
        text.push(format!("changed  {id}  (its presses or start)"));
    }
    let left_of = |id: &str| {
        left.iter().find(|l| {
            id.ends_with(&format!(".{}", l.what)) || id.split('.').nth(1) == Some(l.what.as_str())
        })
    };
    for id in &m.removed {
        let why = left_of(id).map_or("named by --remove", |l| l.why.as_str());
        text.push(format!("removed  {id}  ({why}; --remove)"));
    }
    // The plan's clips the sessions don't give stay. Those the import rules
    // leave out (a sealed run, an exclude) are named, since only --remove
    // takes them out.
    for id in &m.unsessioned {
        if let Some(l) = left_of(id) {
            text.push(format!(
                "kept     {id}  (the sessions leave it out: {}; --remove {id} removes it)",
                l.why
            ));
        }
    }
    text.push(format!(
        "{} clips from the sessions: {} as they were, {} added, {} changed, {} removed; {} kept with no session; {} left out. A {step} step.",
        taken.len(),
        m.kept,
        m.added.len(),
        m.changed.len(),
        m.removed.len(),
        m.unsessioned.len(),
        left.len()
    ));
    let mut v = json!({
        "game": game, "dir": dir, "from": home, "prefix": prefix,
        "added": m.added, "changed": m.changed, "removed": m.removed, "kept": m.kept,
        "kept_without_session": m.unsessioned,
        "left_out": left.iter().map(|l| json!({"what": l.what, "why": l.why})).collect::<Vec<_>>(),
        "step": step, "written": false,
    });
    if a.write && step != "none" && measures_nothing(&dir) {
        bail!(
            "{}'s mapping.toml maps no field yet: a clip scored over nothing agrees on every frame, and the gate would hold every clip to that. Add the player's place first",
            dir.display()
        );
    }
    if !a.write {
        if step != "none" {
            text.push("Nothing written: --write writes the clips and records the version.".into());
        }
        return Ok(Outcome::ok(v, text.join("\n")));
    }
    if step == "none" {
        text.push("The corpus has every clip of the sessions already.".into());
        return Ok(Outcome::ok(v, text.join("\n")));
    }
    // The plan, edited in place, and the presses files.
    let file = dir.join(FILE);
    fs::create_dir_all(dir.join("inputs"))?;
    for (p, t) in &m.files {
        fs::write(dir.join(p), t)?;
    }
    // The presses files of removed clips go with them.
    for id in &m.removed {
        if let Some(r) = plan.set.clips.iter().find(|c| &c.id == id) {
            for p in [&r.start.presses_from_power_on, &r.presses_file]
                .into_iter()
                .flatten()
            {
                if !m.files.iter().any(|(q, _)| q == p) {
                    let _ = fs::remove_file(dir.join(p));
                }
            }
        }
    }
    let before = fs::read_to_string(&file)?;
    fs::write(&file, edit_in_place(&before, &m)?)?;
    v["written"] = json!(true);
    text.push(format!("Wrote {}.", file.display()));

    // The version: against the plan before the import when that was the
    // recorded version, else against the plan git has at HEAD.
    let known = (plan.hash == plan.manifest.corpus.hash).then_some(&plan.recipes);
    let says = a.says.map_or_else(
        || {
            let mut parts = Vec::new();
            for (n, w) in [
                (&m.added, "added"),
                (&m.changed, "changed"),
                (&m.removed, "removed"),
            ] {
                if !n.is_empty() {
                    parts.push(format!("{w} {}", n.join(", ")));
                }
            }
            format!(
                "gp corpus import from the mechanic sessions: {}.",
                parts.join("; ")
            )
        },
        str::to_owned,
    );
    let o = crate::corpus_score::plan_bump(&dir, true, Some(&says), a.date, known)?;
    text.push(o.text.clone());
    v["corpus"] = o.json;
    Ok(Outcome::ok(v, text.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gp_evidence::input_log::generate;

    fn paths(home: &Path) -> Paths {
        Paths {
            home: home.to_owned(),
            store: home.join("store"),
            roms: home.join("roms"),
        }
    }

    /// A session clip from power-on (no stored path): `start` frames of
    /// nothing, then the spans.
    fn clip(home: &Path, session: &str, n: u64, start: u64, spans: &[&str], frames: u64) {
        let dir = home.join("mechanics/g").join(session);
        fs::create_dir_all(dir.join("clips")).unwrap();
        fs::write(
            dir.join("session.toml"),
            format!("game = \"g\"\nmechanic = \"{session}\"\nrom = \"ab\"\n"),
        )
        .unwrap();
        let shifted: Vec<String> = spans
            .iter()
            .map(|s| {
                let (b, r) = s.split_once('@').unwrap();
                let (x, y) = r.split_once('-').unwrap();
                let (x, y): (u64, u64) = (x.parse().unwrap(), y.parse().unwrap());
                format!("{b}@{}-{}", x + start, y + start)
            })
            .collect();
        let refs: Vec<&str> = shifted.iter().map(String::as_str).collect();
        let log = generate((start + frames) as usize, &refs).unwrap();
        fs::write(dir.join(format!("clips/{n:03}.log")), log.to_text()).unwrap();
        fs::write(
            dir.join(format!("clips/{n:03}.toml")),
            format!("by = \"player\"\nstart = {start}\nframes = {frames}\nnote = \"clip {n}\"\n"),
        )
        .unwrap();
    }

    fn args<'a>(dir: &'a Path, home: &'a Path, write: bool) -> Args<'a> {
        Args {
            game: "g",
            template: Some(dir),
            from: Some(home),
            write,
            says: None,
            date: Some("2026-11-02"),
            remove: &[],
            sessions: &[],
        }
    }

    const MAPPING: &str = "schema = 22\nmodel = \"m\"\nsource = \"s\"\nalign = \"polled\"\n[[field]]\npath = \"entities.player[0].x\"\nram = [0x86]\n";

    /// A plan with no clips, as `gp game new` writes it, an `[import]`
    /// table laid out by hand, a changelog and a comment.
    fn plan(dir: &Path, import: &str) -> String {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("mapping.toml"), MAPPING).unwrap();
        let empty = crate::plan::recipe_set_hash(&BTreeMap::new());
        let text = format!(
            "# The corpus plan.\n\n[corpus]\nformat = \"1.0.0\"\nname = \"g\"\nversion = \"1.0.0\"\nrecipes = \"{empty}\"\n\n\
             [target]\ngame = \"g\"\nrom = \"ab\"\n\n{import}\
             [[changelog]]\nversion = \"1.0.0\"\ndate = \"2026-11-01\"\nsays = \"First version: 0 clips.\"\n"
        );
        fs::write(dir.join(FILE), &text).unwrap();
        text
    }

    /// Sealed sessions, a sealed run filed under the game, rows out of
    /// scope and clips excluded by name stay out; the rest become recipes
    /// in inputs/, and the version is recorded on the same --write.
    #[test]
    fn sessions_become_recipes_and_sealed_and_out_of_scope_clips_stay_out() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let tpl = d.path().join("tpl");
        clip(&home, "walk", 1, 10, &["right@0-19"], 30);
        clip(&home, "walk", 2, 12, &["left@2-5", "a@6-8"], 20);
        clip(&home, "walk", 3, 40, &["right@0-99"], 100);
        clip(&home, "jump", 1, 5, &["a@0-9"], 15);
        clip(&home, "sealed-run", 1, 3, &["right@0-9"], 10);
        clip(&home, "title", 1, 0, &["start@0-0"], 5);
        clip(&home, "blind-farm", 1, 0, &["a@0-3"], 5);
        // walk 3 was sealed after it was handed off.
        let sealed = home.join("sealed/g");
        fs::create_dir_all(&sealed).unwrap();
        fs::copy(
            home.join("mechanics/g/walk/clips/003.log"),
            sealed.join("run-1.log"),
        )
        .unwrap();
        fs::write(
            home.join("mechanics/g/queue.json"),
            r#"[{"mechanic": "blind-farm", "status": "next", "scope": "out", "scope_why": "the blind farm"}]"#,
        )
        .unwrap();
        plan(
            &tpl,
            "[import]\nprefix = \"g\"\n\n[[import.exclude]]\nwhat = \"title\"\nwhy = \"title demos\"\n\n",
        );
        let p = paths(&home);
        let o = run(&args(&tpl, &home, false), &p).unwrap();
        assert_eq!(
            o.json["added"],
            json!(["g.jump.001", "g.walk.001", "g.walk.002"])
        );
        assert_eq!(o.json["step"], "minor");
        assert_eq!(o.json["written"], false);
        let left: Vec<(String, String)> = o.json["left_out"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| {
                (
                    l["what"].as_str().unwrap().into(),
                    l["why"].as_str().unwrap().into(),
                )
            })
            .collect();
        assert!(left.contains(&("sealed-run".into(), "a sealed run's session".into())));
        assert!(left.contains(&("title".into(), "title demos".into())));
        assert!(left.contains(&("walk.003".into(), "a sealed run's inputs".into())));
        assert!(
            left.iter()
                .any(|(w, y)| w == "blind-farm" && y.contains("the blind farm"))
        );
        assert!(!tpl.join("inputs").exists(), "a dry run writes nothing");

        let o = run(&args(&tpl, &home, true), &p).unwrap();
        assert_eq!(o.json["written"], true);
        assert_eq!(o.json["corpus"]["suggests"], "1.1.0");
        let after = fs::read_to_string(tpl.join(FILE)).unwrap();
        assert!(after.starts_with("# The corpus plan.\n"), "{after}");
        assert!(
            after.contains("[[import.exclude]]\nwhat = \"title\""),
            "{after}"
        );
        let loaded = crate::plan::load(&tpl).unwrap().unwrap();
        assert_eq!(loaded.hash, loaded.manifest.corpus.hash, "{after}");
        assert_eq!(loaded.manifest.corpus.version, "1.1.0");
        let w2 = loaded
            .set
            .clips
            .iter()
            .find(|c| c.id == "g.walk.002")
            .unwrap();
        assert_eq!(w2.frames, 20);
        assert_eq!(w2.start.frame, 12);
        assert_eq!(
            fs::read_to_string(tpl.join("inputs/g.walk.002.start.presses")).unwrap(),
            "gp-presses 0\nframes 12\n"
        );
        assert_eq!(
            fs::read_to_string(tpl.join("inputs/g.walk.002.presses")).unwrap(),
            "gp-presses 0\nframes 20\nleft@2-5\na@6-8\n"
        );
        // The clips go before the changelog, which gets the new entry.
        let clips_at = after.find("[[clip]]").unwrap();
        assert!(clips_at < after.find("[[changelog]]").unwrap(), "{after}");
        assert!(after.trim_end().ends_with("\"g.walk.002\"]"), "{after}");

        // Nothing new: nothing changes.
        let o = run(&args(&tpl, &home, true), &p).unwrap();
        assert_eq!(o.json["step"], "none");
        assert_eq!(o.json["kept"], 3);
        assert_eq!(fs::read_to_string(tpl.join(FILE)).unwrap(), after);

        // A new clip goes after the last one: a minor step.
        clip(&home, "jump", 2, 7, &["a@0-4"], 9);
        let o = run(&args(&tpl, &home, true), &p).unwrap();
        assert_eq!(o.json["added"], json!(["g.jump.002"]));
        assert_eq!(o.json["corpus"]["suggests"], "1.2.0");
        let ids: Vec<String> = crate::plan::load(&tpl)
            .unwrap()
            .unwrap()
            .set
            .clips
            .iter()
            .map(|c| c.id.clone())
            .collect();
        assert_eq!(
            ids,
            ["g.jump.001", "g.walk.001", "g.walk.002", "g.jump.002"]
        );

        // A changed clip is a major step; --remove takes a clip and its
        // files out.
        clip(&home, "walk", 1, 10, &["right@0-25"], 30);
        let remove = ["g.jump.*".to_owned()];
        let o = run(
            &Args {
                remove: &remove,
                ..args(&tpl, &home, true)
            },
            &p,
        )
        .unwrap();
        assert_eq!(o.json["changed"], json!(["g.walk.001"]));
        assert_eq!(o.json["removed"], json!(["g.jump.001", "g.jump.002"]));
        assert_eq!(o.json["corpus"]["suggests"], "2.0.0");
        assert!(!tpl.join("inputs/g.jump.001.start.presses").exists());
        let loaded = crate::plan::load(&tpl).unwrap().unwrap();
        assert_eq!(loaded.hash, loaded.manifest.corpus.hash);
        assert_eq!(loaded.set.clips.len(), 2);
        let m = fs::read_to_string(tpl.join(FILE)).unwrap();
        assert!(
            m.contains("removed = [\"g.jump.001\", \"g.jump.002\"]"),
            "{m}"
        );
    }

    /// Clips whose sessions aren't in the home are kept, and an import of
    /// one session leaves the others' clips alone.
    #[test]
    fn clips_without_a_session_are_kept_unless_named() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let tpl = d.path().join("tpl");
        clip(&home, "walk", 1, 10, &["right@0-19"], 30);
        clip(&home, "jump", 1, 5, &["a@0-9"], 15);
        plan(&tpl, "");
        let p = paths(&home);
        let only = ["walk".to_owned()];
        let o = run(
            &Args {
                sessions: &only,
                ..args(&tpl, &home, true)
            },
            &p,
        )
        .unwrap();
        assert_eq!(o.json["added"], json!(["g.walk.001"]));
        // The walk session goes from the home: its clip stays.
        let moved = d.path().join("walk-elsewhere");
        fs::rename(home.join("mechanics/g/walk"), &moved).unwrap();
        let o = run(&args(&tpl, &home, true), &p).unwrap();
        assert_eq!(o.json["added"], json!(["g.jump.001"]));
        assert_eq!(o.json["kept_without_session"], json!(["g.walk.001"]));
        assert_eq!(o.json["removed"], json!([]));
    }

    #[test]
    fn an_older_clip_set_is_refused_and_a_mapping_with_no_field_measures_nothing() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        clip(&home, "walk", 1, 10, &["right@0-19"], 30);
        let tpl = d.path().join("tpl");
        fs::create_dir_all(&tpl).unwrap();
        let e = run(&args(&tpl, &home, false), &paths(&home)).err().unwrap();
        assert!(e.to_string().contains("no corpus plan"), "{e}");
        assert!(!measures_nothing(&tpl), "no mapping at all");
        let head = "schema = 22\nmodel = \"m\"\nsource = \"s\"\nalign = \"polled\"\n";
        fs::write(tpl.join("mapping.toml"), format!("{head}field = []\n")).unwrap();
        assert!(measures_nothing(&tpl));
    }
}
