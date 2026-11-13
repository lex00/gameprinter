//! `gp progress`: how far along printing a game is, as a ladder of nine
//! stages from the ROM to the whole game played identically
//! (docs/progress.md). Each stage has one number from 0 to 100 and a line
//! that says when it is done. A stage that can't be computed from what
//! exists says "not measured" and names the command that would measure it.
//!
//! [`ladder`] is the computation, from [`Facts`] alone, so the tests give it
//! made-up inputs. [`gather`] reads the facts from a template, its capture
//! run and the cached measure ([`crate::progress_measure`]), and never
//! writes. The heavy numbers (the RAM a playthrough changes, the code it
//! runs, the whole-game replay) come from `gp progress --measure`.
//!
//! Ported from game-printer's progress.rs. What gameprinter's layout
//! changed: the template is a game of the games checkout (its package
//! folder, or a folder holding model.toml), and a run is gp serve's run of
//! a game, whose model is its working copy once there is one; the source
//! is the game's roms.toml and template.toml `[source]`, found in the ROM
//! folder on either platform; the labels are the game's labels.toml; the
//! scores are those gp serve's background work keeps (behaviour alone:
//! gameprinter's serve has no picture score yet), and there is no `gp agent
//! bench`, so no effort lines. `--module` gives the game's module for the
//! whole-game replay of a model with hooks. The commands the stages name
//! are gameprinter's. An arcade game's stages read through its own mapping
//! on gp-arcade (crate::progress_measure); its RAM is the board's work
//! RAM at E000, counted by its index from there.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Args;
use serde::Serialize;
use serde_json::{Value, json};

use crate::Outcome;
use crate::progress_measure::Measured;

/// The ladder's version: a change to what a stage counts bumps it.
pub(crate) const VERSION: &str = "progress 2";

#[derive(Args, Debug, Clone)]
pub struct ProgressArgs {
    /// The game: a name in the games checkout, or a folder holding
    /// model.toml (game-printer's template).
    #[arg(long)]
    pub template: String,
    /// The gp serve run whose model, clips and scores to read: a run is a
    /// game of the games checkout, by its folder's name. Without it only
    /// the game's folder is read.
    #[arg(long)]
    pub run: Option<String>,
    /// Measure the heavy stages and keep the result: the playthrough played
    /// on the original (the RAM it changes, the object kinds and the code it
    /// runs), the run's clips on the original (the code they run), and the
    /// whole playthrough replayed on the rebuilt game. Needs `--run`.
    #[arg(long)]
    pub measure: bool,
    /// With `--measure`, the full playthrough to measure against: a recording
    /// by ID or prefix, or a baseline file. Default: the longest of the
    /// run's recordings and baselines.
    #[arg(long)]
    pub playthrough: Option<String>,
    /// With `--measure`, leave out the run's clips on the original (stage 6's
    /// code coverage).
    #[arg(long)]
    pub no_clips: bool,
    /// With `--measure`, leave out the whole-game replay (stage 9).
    #[arg(long)]
    pub no_whole: bool,
    /// Write the RAM bytes the last measure's trace says aren't game state
    /// (input, counters, scratch, sound, pointers) into this mapping file as
    /// its [[ignore]] list, replacing the list written before. Bytes the
    /// mapping names are left out of it. Needs `--run` and a measure.
    #[arg(long, value_name = "MAPPING")]
    pub write_ignore: Option<PathBuf>,
    /// With `--measure`, the game's built module, for a model with hooks
    /// (gameprinter's hooks live in each game's module).
    #[arg(long)]
    pub module: Option<PathBuf>,
}

/// One stage of the ladder.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct Stage {
    pub n: u8,
    pub key: &'static str,
    pub name: &'static str,
    /// The stage's number, 0 to 100, rounded down so that 100 means done
    /// on that count. None when it isn't measured.
    pub percent: Option<u64>,
    /// What the number counts: `with` of `of`.
    pub with: Option<u64>,
    pub of: Option<u64>,
    pub done: bool,
    pub done_when: &'static str,
    /// The number in words: "142 of 300 RAM bytes the game changes in play
    /// are named".
    pub says: String,
    /// The stage's other measures, in words.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub also: Vec<String>,
    /// A table of the stage's parts (stage 9: each level).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub table: Vec<Row>,
    /// The command that measures what isn't measured yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measure: Option<String>,
}

/// One row of a stage's table.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct Row {
    pub name: String,
    pub with: u64,
    pub of: u64,
    pub says: String,
}

impl Stage {
    fn new(n: u8, key: &'static str, name: &'static str, done_when: &'static str) -> Self {
        Self {
            n,
            key,
            name,
            percent: None,
            with: None,
            of: None,
            done: false,
            done_when,
            says: "not measured".into(),
            also: Vec::new(),
            table: Vec::new(),
            measure: None,
        }
    }

    fn count(mut self, with: u64, of: u64, says: String) -> Self {
        self.with = Some(with);
        self.of = Some(of);
        self.percent = Some(percent(with, of));
        self.says = says;
        self
    }

    fn unmeasured(mut self, says: impl Into<String>, cmd: impl Into<String>) -> Self {
        self.says = says.into();
        self.measure = Some(cmd.into());
        self
    }
}

/// `with` of `of` as a percent, rounded down: 100 only when all are.
pub(crate) fn percent(with: u64, of: u64) -> u64 {
    (with.min(of) * 100).checked_div(of).unwrap_or(0)
}

/// The source's facts, stage 1.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct SourceFacts {
    /// The ROM's ID, as the template's `[source]` names it.
    pub id: String,
    /// Registered in `sources/<platform>.toml`, or a game built from source
    /// whose hash the template accepts.
    pub registered: bool,
    /// The registry says `verified`, or the source is built here.
    pub verified: bool,
    /// A ROM with an accepted hash was found.
    pub rom: Option<PathBuf>,
    /// The labels file (from a disassembly or the game's source), when it
    /// is there.
    pub labels: Option<PathBuf>,
    /// The command that makes the labels when they aren't there.
    pub labels_recipe: String,
}

/// What the clips' scores say, stages 7 and 8.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Scores {
    pub clips: u64,
    pub exact: u64,
    /// Behaviour fidelity, 0 to 100.
    pub behaviour: u64,
    /// The mean of the scored clips' frames together, in thousandths.
    pub together_permille: u64,
    /// The fidelity score, 0 to 100.
    pub fidelity: u64,
    /// Picture fidelity, 0 to 100, when any clip has a screen score.
    pub picture: Option<u64>,
    /// The entity kinds a scored clip measures (a body paired across the
    /// two games, or the player).
    pub measured_kinds: BTreeSet<String>,
}

/// Everything the ladder reads.
#[derive(Debug, Clone, Default)]
pub(crate) struct Facts {
    pub template: String,
    pub run: Option<String>,
    pub source: SourceFacts,
    /// The cached measure, if `gp progress --measure` has run.
    pub measured: Option<Measured>,
    /// The model's hash now, to say when the measure is older.
    pub model_now: String,
    /// The RAM addresses the state mapping names, the level grid its
    /// `[cells]` reads among them.
    pub named_ram: BTreeSet<u16>,
    pub cell_ram: BTreeSet<u16>,
    /// The mapping's ignore list: each byte's role and whether it is
    /// flagged for review.
    pub ignored: BTreeMap<u16, (String, bool)>,
    /// RAM labels by address, for naming the bytes the mapping doesn't.
    pub labels: BTreeMap<u16, String>,
    /// Whether the mapping has `[slots]`, and each slot table's kinds by ID.
    pub slots: bool,
    pub slot_kinds: BTreeMap<(u8, u8), String>,
    /// The slot tables whose ID byte is their state byte too.
    pub state_ids: BTreeSet<u8>,
    /// The model's entity kinds, the player's first.
    pub entities: BTreeSet<String>,
    pub player: Option<String>,
    /// The kinds a mapped field measures (for a game without slots).
    pub mapped_kinds: BTreeSet<String>,
    /// The game's levels, by ID, and the model's.
    pub game_levels: BTreeSet<String>,
    pub model_levels: BTreeSet<String>,
    /// Levels in game words ("World 1-1"), by ID.
    pub level_names: BTreeMap<String, String>,
    /// The run's starts: (the rebuilt game can begin at, all).
    pub starts: Option<(u64, u64)>,
    /// Facts with evidence from the game's code, and all facts.
    pub code_facts: (u64, u64),
    /// The open missing-mechanic findings (distinct parts), when a run says.
    pub missing: Option<u64>,
    /// Mutants a kept clip catches, and all the clip set names.
    pub mutants: Option<(u64, u64)>,
    pub scores: Option<Scores>,
    /// The agents' efficiency (`gp agent bench`), when the run has a ledger.
    pub effort: Option<Value>,
}

impl Facts {
    fn measure_cmd(&self) -> String {
        match &self.run {
            Some(r) => format!(
                "gp progress --template {} --run {r} --measure",
                self.template
            ),
            None => format!(
                "gp progress --template {} --run RUN --measure (a run is the game, as gp serve names it)",
                self.template
            ),
        }
    }

    /// A level in game words, or its ID.
    fn level_name(&self, id: &str) -> String {
        match self.level_names.get(id) {
            Some(n) => n.clone(),
            None if id.is_empty() => "the game".into(),
            None => id.to_owned(),
        }
    }

    /// This command again with a run, for a stage that needs one.
    fn with_run(&self) -> String {
        format!(
            "gp progress --template {} --run RUN (a run is the game, as gp serve names it)",
            self.template
        )
    }
}

/// The few names of a list, "a, b and 3 more".
fn few(names: &[String]) -> String {
    match names.len() {
        0 => String::new(),
        1 => names[0].clone(),
        2 => format!("{} and {}", names[0], names[1]),
        3 => format!("{}, {} and {}", names[0], names[1], names[2]),
        n => format!(
            "{}, {}, {} and {} more",
            names[0],
            names[1],
            names[2],
            n - 3
        ),
    }
}

fn stage_source(f: &Facts) -> Stage {
    let s = &f.source;
    let st = Stage::new(
        1,
        "source",
        "Source",
        "The ROM is registered, its hash verified, and its labels are there",
    );
    let checks = [
        (s.registered, "registered"),
        (s.verified, "verified"),
        (s.rom.is_some(), "the ROM found by its hash"),
        (s.labels.is_some(), "labels from its code"),
    ];
    let with = checks.iter().filter(|c| c.0).count() as u64;
    let missing: Vec<String> = checks
        .iter()
        .filter(|c| !c.0)
        .map(|c| c.1.to_owned())
        .collect();
    let says = if missing.is_empty() {
        format!("{}: registered, verified, found and labelled", s.id)
    } else {
        format!("{}: not yet {}", s.id, missing.join(", "))
    };
    let mut st = st.count(with, checks.len() as u64, says);
    st.done = with == checks.len() as u64;
    if s.labels.is_none() && !s.labels_recipe.is_empty() {
        st.measure = Some(s.labels_recipe.clone());
    }
    if s.rom.is_none() {
        st.measure = Some("gp roms add FILE (docs/sources.md)".into());
    }
    st
}

/// The game-state bytes of the RAM the playthrough changes in play: all
/// but those the mapping's ignore list leaves out, though a byte the
/// mapping names is game state whatever the list says.
fn game_state(f: &Facts, m: &Measured) -> BTreeSet<u16> {
    m.ram
        .iter()
        .copied()
        .filter(|a| f.named_ram.contains(a) || !f.ignored.contains_key(a))
        .collect()
}

fn stage_memory(f: &Facts) -> Stage {
    let st = Stage::new(
        2,
        "memory",
        "Memory map",
        "Every game-state RAM byte the game changes in a full playthrough is named by the state mapping, and every other one is in its ignore list with a reason",
    );
    let Some(m) = &f.measured else {
        return st.unmeasured(
            "not measured: needs a full playthrough played on the original",
            f.measure_cmd(),
        );
    };
    let changed: BTreeSet<u16> = m.ram.iter().copied().collect();
    let written = game_state(f, m);
    let named = written.intersection(&f.named_ram).count() as u64;
    let mut st = st.count(
        named,
        written.len() as u64,
        format!(
            "{named} of {} game-state RAM bytes the game changes in play are named",
            written.len()
        ),
    );
    st.done = named == written.len() as u64 && !written.is_empty();
    let cells = written.intersection(&f.cell_ram).count();
    if cells > 0 {
        st.also.push(format!(
            "{cells} of the named are the level grid the mapping's [cells] reads"
        ));
    }
    // The bytes left out on purpose, by role.
    let left: Vec<u16> = changed.difference(&written).copied().collect();
    if !left.is_empty() {
        let mut by: BTreeMap<&str, u64> = BTreeMap::new();
        for a in &left {
            *by.entry(f.ignored[a].0.as_str()).or_default() += 1;
        }
        let review = left.iter().filter(|a| f.ignored[a].1).count();
        st.also.push(format!(
            "{} of the {} bytes it changes are left out by the mapping's ignore list: {}{}",
            left.len(),
            changed.len(),
            by.iter()
                .map(|(r, n)| format!("{r} {n}"))
                .collect::<Vec<_>>()
                .join(", "),
            if review > 0 {
                format!("; {review} flagged for review")
            } else {
                String::new()
            }
        ));
    }
    // What the last trace says that the list doesn't.
    if !m.roles.roles.is_empty() {
        let not_state: BTreeSet<u16> = m
            .roles
            .roles
            .iter()
            .filter(|r| r.role != gp_nes::ramrole::Role::State)
            .map(|r| r.addr)
            .collect();
        let more = written
            .iter()
            .filter(|a| not_state.contains(a) && !f.named_ram.contains(a))
            .count();
        let stale = left.iter().filter(|a| !not_state.contains(a)).count();
        if more > 0 {
            st.also.push(format!(
                "the trace says {more} more aren't game state: gp progress --template {} --run {} --write-ignore MAPPING",
                f.template,
                f.run.as_deref().unwrap_or("RUN")
            ));
        }
        if stale > 0 {
            st.also.push(format!(
                "{stale} bytes in the ignore list are game state by the trace"
            ));
        }
    }
    // The bytes the labels name first (a disassembly's, or the game's own
    // source): they are the game's variables, where an unlabelled byte is
    // often a buffer or a scratch.
    let labelled: BTreeSet<u16> = written
        .iter()
        .copied()
        .filter(|a| f.labels.contains_key(a))
        .collect();
    if !labelled.is_empty() {
        st.also.push(format!(
            "{} of the {} that have a label are named",
            labelled.intersection(&f.named_ram).count(),
            labelled.len()
        ));
    }
    let unnamed: Vec<String> = labelled
        .difference(&f.named_ram)
        .chain(
            written
                .difference(&f.named_ram)
                .filter(|a| !labelled.contains(a)),
        )
        .take(6)
        .map(|a| match f.labels.get(a) {
            Some(l) => format!("{l} (${a:04X})"),
            None => format!("${a:04X}"),
        })
        .collect();
    if !unnamed.is_empty() {
        let n = written.len() - named as usize;
        let mut words = few(&unnamed[..unnamed.len().min(3)]);
        if n > 3 {
            words = format!("{} and {} more", unnamed[..3].join(", "), n - 3);
        }
        st.also.push(format!("not named yet: {words}"));
    }
    st
}

fn stage_objects(f: &Facts) -> Stage {
    let st = Stage::new(
        3,
        "objects",
        "Objects",
        "Every kind of object seen in a full playthrough is modelled, and a scored clip measures it",
    );
    let measured = f.scores.as_ref().map(|s| &s.measured_kinds);
    if !f.slots {
        // Without slots the ROM's kinds aren't known: count the model's.
        let kinds: Vec<&String> = f
            .entities
            .iter()
            .filter(|k| Some(*k) != f.player.as_ref())
            .collect();
        let with = kinds
            .iter()
            .filter(|k| f.mapped_kinds.contains(**k))
            .count() as u64;
        let mut st = st.count(
            with,
            kinds.len() as u64,
            format!(
                "{with} of the model's {} kinds of object are measured against the original",
                kinds.len()
            ),
        );
        st.also
            .push("the mapping has no [slots], so the kinds the ROM handles aren't counted".into());
        let in_clips = measured.map_or(0, |m| kinds.iter().filter(|k| m.contains(**k)).count());
        if measured.is_some() {
            st.also.push(format!(
                "{in_clips} of {} measured in a scored clip",
                kinds.len()
            ));
        }
        st.done = with == kinds.len() as u64 && in_clips == kinds.len();
        return st;
    }
    let Some(m) = &f.measured else {
        return st.unmeasured(
            "not measured: the kinds come from a full playthrough",
            f.measure_cmd(),
        );
    };
    // A kind is an entity the mapping names for the ID, counted once
    // however many IDs it has; an ID it doesn't name is a kind of its own,
    // except in a table whose ID byte is its state, where the values it
    // doesn't name are one kind ("group 2's other states").
    let mut seen: BTreeMap<String, bool> = BTreeMap::new();
    for (t, id) in &m.kinds {
        let (name, has) = match f.slot_kinds.get(&(*t, *id)) {
            Some(e) => (e.clone(), f.entities.contains(e)),
            None if f.state_ids.contains(t) => (format!("group {t}'s other states"), false),
            None if *t == 0 => (format!("ID ${id:02X}"), false),
            None => (format!("group {t} ID ${id:02X}"), false),
        };
        seen.insert(name, has);
    }
    let modelled: Vec<&String> = seen.iter().filter(|e| *e.1).map(|e| e.0).collect();
    let missing: Vec<String> = seen.iter().filter(|e| !*e.1).map(|e| e.0.clone()).collect();
    let with = modelled.len() as u64;
    let mut st = st.count(
        with,
        seen.len() as u64,
        format!(
            "{with} of {} kinds of object the playthrough shows are modelled",
            seen.len()
        ),
    );
    if !missing.is_empty() {
        st.also.push(format!("not modelled: {}", few(&missing)));
    }
    let in_clips = measured.map(|ms| modelled.iter().filter(|k| ms.contains(**k)).count());
    match in_clips {
        Some(n) => st
            .also
            .push(format!("{n} of {} measured in a scored clip", seen.len())),
        None => st
            .also
            .push("measured in a scored clip: not known without --run".into()),
    }
    st.done = with == seen.len() as u64 && in_clips == Some(seen.len()) && !seen.is_empty();
    st
}

fn stage_levels(f: &Facts) -> Stage {
    let st = Stage::new(
        4,
        "levels",
        "Levels",
        "Every level of the game is in the model, and the rebuilt game can begin at every start",
    );
    let game = &f.game_levels;
    if game.is_empty() {
        return st.unmeasured(
            "not measured: the game's levels come from the mapping's slots or the run's recordings",
            f.with_run(),
        );
    }
    let with = game.intersection(&f.model_levels).count() as u64;
    let missing: Vec<String> = game
        .difference(&f.model_levels)
        .map(|l| f.level_name(l))
        .collect();
    let mut st = st.count(
        with,
        game.len() as u64,
        format!(
            "{with} of the game's {} levels are in the model",
            game.len()
        ),
    );
    if !missing.is_empty() {
        st.also.push(format!("not in the model: {}", few(&missing)));
    }
    match f.starts {
        Some((ok, all)) => {
            st.also.push(format!(
                "the rebuilt game can begin at {ok} of {all} starts ({}%)",
                percent(ok, all)
            ));
            st.done = with == game.len() as u64 && ok == all && all > 0;
        }
        None => st.also.push(match &f.run {
            Some(r) => format!("starts: not counted yet for run {r} (gp corpus score)"),
            None => "starts: not known without --run".into(),
        }),
    }
    st
}

fn stage_mechanics(f: &Facts) -> Stage {
    let st = Stage::new(
        5,
        "mechanics",
        "Mechanics",
        "No missing-mechanic finding is open",
    );
    let (code, all) = f.code_facts;
    let mut st = st.count(
        code,
        all,
        format!("{code} of the model's {all} facts cite the game's code"),
    );
    match f.missing {
        Some(n) => {
            st.also.push(format!(
                "{n} missing-mechanic findings open (a part no mechanic writes, or numbers no mechanic gives)"
            ));
            st.done = n == 0;
        }
        None => {
            st.also
                .push("missing-mechanic findings: not measured".into());
            st.measure = Some(match &f.run {
                Some(r) => format!("gp solve --run {r} --missing"),
                None => "gp solve --run GAME --missing".into(),
            });
        }
    }
    st
}

fn stage_clips(f: &Facts) -> Stage {
    let st = Stage::new(
        6,
        "clips",
        "Clips",
        "The clips run all the code a full playthrough runs, and a kept clip catches every fix's mutant",
    );
    let corpus = f.measured.as_ref().and_then(|m| m.clips.as_ref());
    let mut st = match (corpus, &f.measured) {
        (Some(c), Some(m)) => st.count(
            c.shared,
            m.code,
            format!(
                "the {} clips run {} of the {} code addresses a full playthrough runs",
                c.clips, c.shared, m.code
            ),
        ),
        _ => st.unmeasured(
            "code coverage not measured: needs the playthrough and the clips played on the original",
            f.measure_cmd(),
        ),
    };
    if let Some(c) = corpus
        && c.only > 0
    {
        st.also.push(format!(
            "{} addresses the clips run aren't in the playthrough",
            c.only
        ));
    }
    match f.mutants {
        Some((caught, all)) => {
            st.also
                .push(format!("{caught} of {all} mutants caught by a kept clip"));
            st.done = st.percent == Some(100) && caught == all;
        }
        None => st.also.push("mutants: the game's corpus names none".into()),
    }
    st
}

fn stage_behaviour(f: &Facts) -> Stage {
    let st = Stage::new(
        7,
        "behaviour",
        "Behaviour",
        "Every scored clip plays exactly as the original",
    );
    let Some(s) = &f.scores else {
        return st.unmeasured("not measured: no run's clips are scored", f.with_run());
    };
    if s.clips == 0 {
        return st.unmeasured(
            "not measured: no clip is scored yet",
            match &f.run {
                Some(r) => format!("gp serve, then start {r}'s background work in REFINE"),
                None => "gp serve, then start the game's background work in REFINE".into(),
            },
        );
    }
    let mut st = st.count(
        s.exact,
        s.clips,
        format!("{} of {} scored clips are exact", s.exact, s.clips),
    );
    st.also.push(format!("behaviour fidelity {}", s.behaviour));
    st.also.push(format!(
        "frames together {}.{}%",
        s.together_permille / 10,
        s.together_permille % 10
    ));
    st.done = s.exact == s.clips;
    st
}

fn stage_picture(f: &Facts) -> Stage {
    let st = Stage::new(
        8,
        "picture",
        "Picture and sound",
        "The pictures match on every clip and no picture-gap group is open",
    );
    let picture = f.scores.as_ref().and_then(|s| s.picture);
    let mut st = match picture {
        Some(p) => {
            let mut st = st.count(p, 100, format!("picture fidelity {p}"));
            st.percent = Some(p);
            st
        }
        None => st.unmeasured(
            "picture not measured: no clip has a screen score",
            "gp picture GAME --run RUN (gameprinter's serve keeps no screen score yet)",
        ),
    };
    st.also.push(format!(
        "picture-gap groups: not measured (gp picture {} --run RUN)",
        f.run.as_deref().unwrap_or("GAME")
    ));
    st.also
        .push("sound: not measured (gp sound fidelity)".into());
    st.done = false;
    st
}

fn stage_whole(f: &Facts) -> Stage {
    let st = Stage::new(
        9,
        "whole",
        "Whole game",
        "The whole playthrough plays identically on the rebuilt game, started from the original's state",
    );
    let Some(w) = f.measured.as_ref().and_then(|m| m.whole.as_ref()) else {
        return st.unmeasured(
            "not measured: the playthrough isn't replayed on the rebuilt game yet",
            f.measure_cmd(),
        );
    };
    let together = w.first.as_ref().map_or(w.compared, |s| s.at);
    let mut st = st.count(
        together,
        w.compared,
        match &w.first {
            Some(s) => format!(
                "{together} of {} frames play identically from the original's state at its frame {}, then they part at its frame {} in {}: {}",
                w.compared,
                w.from,
                s.original_frame,
                f.level_name(&s.level),
                split_words(&s.paths)
            ),
            None => format!(
                "all {} frames play identically from the original's state at its frame {}",
                w.compared, w.from
            ),
        },
    );
    let who = f
        .player
        .as_deref()
        .map_or_else(|| "the player".to_owned(), capital);
    match &w.player {
        Some(p) => st.also.push(format!(
            "{who}'s own measures hold for {} frames ({}%), then part at the original's frame {} in {}: {}",
            p.at,
            percent(p.at, w.compared),
            p.original_frame,
            f.level_name(&p.level),
            split_words(&p.paths)
        )),
        None if w.compared > 0 && f.player.is_some() => st.also.push(format!(
            "{who}'s own measures hold for the whole playthrough"
        )),
        None => {}
    }
    if !w.levels.is_empty() {
        let ran: Vec<&crate::progress_measure::LevelRun> =
            w.levels.iter().filter(|l| l.cant.is_none()).collect();
        let with: u64 = ran.iter().map(|l| l.together()).sum();
        let of: u64 = ran.iter().map(|l| l.compared).sum();
        let whole = ran.iter().filter(|l| l.first.is_none()).count();
        st.also.push(format!(
            "level by level, each from the original's state at its start: {with} of {of} frames ({}%) before each level's first part; {whole} of {} levels play through identically{}",
            percent(with, of),
            w.levels.len(),
            match w.levels.len() - ran.len() {
                0 => String::new(),
                n => format!("; {n} can't start"),
            }
        ));
        for l in &w.levels {
            let name = f.level_name(&l.level);
            st.table.push(match (&l.cant, &l.first) {
                (Some(why), _) => Row {
                    name,
                    with: 0,
                    of: 0,
                    says: format!("can't start at frame {}: {why}", l.from),
                },
                (None, Some(s)) => Row {
                    name,
                    with: s.at,
                    of: l.compared,
                    says: format!(
                        "{} of {} frames, then part at the original's frame {}: {}",
                        s.at,
                        l.compared,
                        s.original_frame,
                        split_words(&s.paths)
                    ),
                },
                (None, None) => Row {
                    name,
                    with: l.compared,
                    of: l.compared,
                    says: format!("all {} frames", l.compared),
                },
            });
        }
    }
    if let Some((within, of)) = w.resync {
        st.also.push(format!(
            "put back after each part, {within} of {of} frames agree on every measure ({}%)",
            percent(within, of)
        ));
    }
    st.done = w.first.is_none() && w.compared > 0;
    st
}

/// "mario" as "Mario".
fn capital(w: &str) -> String {
    let mut c = w.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

/// A measure path in words: the lockstep's words, and a timer's count
/// toward its next step (`ticks.star[0]`) as "the star timer's count".
fn path_words(p: &str) -> String {
    match p.strip_prefix("ticks.") {
        Some(rest) => format!(
            "the {} timer's count",
            rest.split('[').next().unwrap_or(rest).replace('_', " ")
        ),
        None => gp_interp::lockstep::words(p),
    }
}

/// The first few paths of a split in words.
fn split_words(paths: &[String]) -> String {
    let w: Vec<String> = paths.iter().map(|p| path_words(p)).collect();
    few(&w)
}

/// The ladder, stage by stage.
pub(crate) fn ladder(f: &Facts) -> Vec<Stage> {
    vec![
        stage_source(f),
        stage_memory(f),
        stage_objects(f),
        stage_levels(f),
        stage_mechanics(f),
        stage_clips(f),
        stage_behaviour(f),
        stage_picture(f),
        stage_whole(f),
    ]
}

/// The effort lines from `gp agent bench`'s JSON: how automatic the
/// progress was.
pub(crate) fn effort_lines(b: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let hours = b["work_s"].as_f64().unwrap_or(0.0) / 3600.0;
    let rounds = b["rounds"].as_u64().unwrap_or(0);
    let scores = b["scores_read"].as_u64().unwrap_or(0);
    out.push(if rounds == 0 {
        format!("no background rounds yet; {scores} clip scores read")
    } else {
        format!("{hours:.1} hours of work over {rounds} rounds; {scores} clip scores read")
    });
    let by = |k: &str| -> String {
        b[k].as_object()
            .filter(|m| !m.is_empty())
            .map(|m| {
                m.iter()
                    .map(|(s, n)| format!("{s} {}", n.as_f64().unwrap_or(0.0)))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_else(|| "none".into())
    };
    out.push(format!("changes kept by source: {}", by("kept_by_source")));
    out.push(format!("clips fixed by source: {}", by("fixed_by_source")));
    let llm = &b["llm"];
    let calls = llm["clip_agent_calls"].as_u64().unwrap_or(0)
        + llm["request_agent_calls"].as_u64().unwrap_or(0);
    let fixes = llm["fixes"].as_u64().unwrap_or(0);
    out.push(match llm["calls_per_fix"].as_f64() {
        Some(c) if calls >= fixes => format!(
            "{calls} LLM calls logged, {fixes} clips fixed by an LLM attempt ({c:.1} calls a fix)"
        ),
        Some(_) => format!(
            "{calls} LLM calls logged, {fixes} clips fixed by an LLM attempt (some calls weren't logged)"
        ),
        None => format!("{calls} LLM calls logged, no clip fixed by an LLM yet"),
    });
    out
}

/// The ladder and the rest as JSON.
pub(crate) fn json_of(f: &Facts) -> Value {
    let stages = ladder(f);
    let done = stages.iter().filter(|s| s.done).count();
    let next = stages.iter().find(|s| !s.done).map(|s| s.n);
    json!({
        "version": VERSION,
        "template": f.template,
        "run": f.run,
        "stages": stages,
        "done": done,
        "next": next,
        "measured": f.measured.as_ref().map(|m| json!({
            "playthrough": m.playthrough,
            "name": m.name,
            "frames": m.frames,
            "play_frames": m.play_frames,
            "model": m.model,
            "stale": !f.model_now.is_empty() && m.model != f.model_now,
            "seconds": m.seconds,
        })),
        "effort": f.effort.as_ref().map(|b| json!({
            "says": effort_lines(b),
            "bench": b,
        })),
    })
}

/// The ladder as text.
pub(crate) fn text_of(f: &Facts, v: &Value) -> String {
    let mut lines = vec![format!(
        "Progress on {}{}: {} of 9 stages done",
        f.template,
        f.run
            .as_ref()
            .map(|r| format!(" (run {r})"))
            .unwrap_or_default(),
        v["done"]
    )];
    for s in ladder(f) {
        let pct = s
            .percent
            .map_or_else(|| "  -".to_owned(), |p| format!("{p:3}%"));
        lines.push(format!(
            "{} {:18} {pct}  {}{}",
            s.n,
            s.name,
            s.says,
            if s.done { "  (done)" } else { "" }
        ));
        for a in &s.also {
            lines.push(format!("                         {a}"));
        }
        for r in &s.table {
            lines.push(format!(
                "                           {:12} {}",
                r.name, r.says
            ));
        }
        if let Some(c) = &s.measure {
            lines.push(format!("                         measure: {c}"));
        }
    }
    if let Some(m) = &f.measured {
        lines.push(format!(
            "Measured on {} ({}, {} frames){}.",
            m.name,
            if m.playthrough.starts_with("baseline:") {
                &m.playthrough
            } else {
                &m.playthrough[..m.playthrough.len().min(12)]
            },
            m.frames,
            if !f.model_now.is_empty() && m.model != f.model_now {
                ", on an older model: measure again"
            } else {
                ""
            }
        ));
    }
    if let Some(b) = &f.effort {
        lines.push("Effort:".into());
        lines.extend(effort_lines(b).into_iter().map(|l| format!("  {l}")));
    }
    lines.join("\n")
}

// Gathering the facts from files.

/// The game's folder: a path holding model.toml, or a game of the games
/// checkout by name.
pub(crate) fn template_dir(t: &str) -> Result<PathBuf> {
    let p = Path::new(t);
    if p.join("model.toml").is_file() {
        return Ok(p.to_owned());
    }
    let games = crate::roms_cmd::games_dir(None)
        .context("no games checkout found: set GP_GAMES_DIR, or give a folder with model.toml")?;
    let d = crate::labels::game_folder(&games, t)
        .with_context(|| format!("no game {t} in {}", games.display()))?;
    anyhow::ensure!(
        d.join("model.toml").is_file(),
        "{} has no model.toml",
        d.display()
    );
    Ok(d)
}

/// The RAM addresses a state mapping names: each field's bytes and
/// fraction, and each slot table's flag, ID, state, turn and fields for
/// every slot, with its clock.
pub(crate) fn named_ram(m: &gp_interp::mapping::Mapping) -> BTreeSet<u16> {
    let mut out = BTreeSet::new();
    let field = |f: &gp_interp::mapping::Field, out: &mut BTreeSet<u16>, n: u16| {
        for i in 0..n {
            out.extend(f.slot_addresses(i).into_iter().map(|a| a & 0x7FF));
        }
    };
    for f in &m.fields {
        field(f, &mut out, 1);
    }
    fn slots(
        s: &gp_interp::mapping::Slots,
        out: &mut BTreeSet<u16>,
        field: &dyn Fn(&gp_interp::mapping::Field, &mut BTreeSet<u16>, u16),
    ) {
        let n = s.count;
        for b in [Some(s.active), s.id, s.state, s.turn]
            .into_iter()
            .flatten()
        {
            for i in 0..n {
                out.insert((b + i) & 0x7FF);
            }
        }
        out.extend(s.clock.map(|c| c & 0x7FF));
        for f in s
            .fields
            .iter()
            .chain(&s.rest)
            .chain(s.kinds.iter().flat_map(|k| &k.fields))
        {
            field(f, out, n);
        }
        for g in &s.groups {
            slots(g, out, field);
        }
    }
    if let Some(s) = &m.slots {
        slots(s, &mut out, &field);
    }
    out.extend(cell_ram(m));
    out
}

/// The RAM bytes of the level grid the mapping's `[cells]` reads: each
/// page's columns times rows.
pub(crate) fn cell_ram(m: &gp_interp::mapping::Mapping) -> BTreeSet<u16> {
    let mut out = BTreeSet::new();
    if let Some(c) = &m.cells {
        let n = (c.page_columns * c.rows).clamp(0, 0x800) as u16;
        for p in &c.pages {
            for i in 0..n {
                out.insert(p.wrapping_add(i) & 0x7FF);
            }
        }
    }
    out
}

/// Each slot table's kinds by ID: table 0 is `[slots]`, then its groups.
pub(crate) fn slot_kinds(m: &gp_interp::mapping::Mapping) -> BTreeMap<(u8, u8), String> {
    let mut out = BTreeMap::new();
    if let Some(s) = &m.slots {
        let tables = std::iter::once(s).chain(&s.groups);
        for (t, table) in tables.enumerate() {
            for k in &table.kinds {
                for id in &k.ids {
                    out.insert((t as u8, *id), k.entity.clone());
                }
            }
        }
    }
    out
}

/// The slot tables whose ID byte is also their flag (`id == active`): the
/// value is the object's state, so values no kind names aren't kinds.
pub(crate) fn state_ids(m: &gp_interp::mapping::Mapping) -> BTreeSet<u8> {
    m.slots
        .iter()
        .flat_map(|s| std::iter::once(s).chain(&s.groups))
        .enumerate()
        .filter(|(_, t)| t.id == Some(t.active))
        .map(|(i, _)| i as u8)
        .collect()
}

/// Whether an evidence entry cites the game's code: a ref into a pinned
/// download (a disassembly), or a source or listing file.
fn cites_code(ev: &gp_model::model::Evidence) -> bool {
    ev.refs.iter().any(|r| {
        r.within.is_some() || {
            let file = r.at.split(':').next().unwrap_or("");
            [".s", ".asm", ".inc", ".a65", ".ca65"]
                .iter()
                .any(|e| file.ends_with(e))
        }
    })
}

/// Facts that cite the game's code, and all facts.
pub(crate) fn code_facts(model: &gp_model::Model) -> (u64, u64) {
    let all = gp_model::validate::evidence_for(model);
    let code = all
        .iter()
        .filter(|(_, k)| {
            k.as_ref()
                .and_then(|k| model.evidence.get(k))
                .is_some_and(cites_code)
        })
        .count();
    (code as u64, all.len() as u64)
}

/// The game's source: registered, verified, its ROM and its labels.
fn source_facts(tdir: &Path, paths: &gp_evidence::Paths) -> SourceFacts {
    let manifest: toml::Value = std::fs::read_to_string(tdir.join("template.toml"))
        .ok()
        .and_then(|t| toml::from_str(&t).ok())
        .unwrap_or(toml::Value::Table(Default::default()));
    let src = &manifest["source"];
    let id = src
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let built = src.get("how").and_then(|v| v.as_str()) == Some("build");
    let accepts: Vec<String> = src
        .get("accepts")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    // The game's own roms.toml, else the registry of every game.
    let reg = std::fs::read_to_string(tdir.join("roms.toml"))
        .ok()
        .and_then(|t| gp_evidence::registry::Registry::from_toml(&t, tdir).ok())
        .filter(|r| r.by_id(&id).is_some())
        .or_else(|| crate::open::load_registry().ok());
    let known = reg.as_ref().and_then(|r| r.by_id(&id));
    let registered = known.is_some_and(|k| accepts.is_empty() || accepts.contains(&k.sha256))
        || (built && !accepts.is_empty());
    let verified =
        built || known.is_some_and(|k| k.status == gp_evidence::registry::SourceStatus::Verified);
    // Found in the ROM folder by its hash, an NES image or a romset.
    let rom = crate::roms::find(&accepts, &[], &paths.roms)
        .ok()
        .filter(|f| !f.is_empty())
        .map(|_| paths.roms.clone());
    let (labels, labels_recipe) =
        match crate::labels::locate(tdir.file_name().and_then(|n| n.to_str()).unwrap_or(&id)) {
            Ok(p) => (Some(p), String::new()),
            Err(_) => (
                None,
                if src.get("platform").and_then(|v| v.as_str()) == Some("arcade") {
                    format!("gp labels propose {id}")
                } else {
                    String::new()
                },
            ),
        };
    SourceFacts {
        id,
        registered,
        verified,
        rom,
        labels,
        labels_recipe,
    }
}

fn ram_labels(path: &Path) -> BTreeMap<u16, String> {
    let mut out = BTreeMap::new();
    let Some(file) = std::fs::read_to_string(path)
        .ok()
        .and_then(|t| crate::labels::parse(&t).ok())
    else {
        return out;
    };
    for l in &file.label {
        for i in 0..l.size.max(1) {
            let mut a = l.addr.wrapping_add(i);
            // An arcade board's work RAM at E000, by its index.
            if (0xE000..0xE800).contains(&a) {
                a &= 0x7FF;
            }
            if a < 0x800 {
                out.entry(a).or_insert_with(|| {
                    if i == 0 {
                        l.name.clone()
                    } else {
                        format!("{}+{i}", l.name)
                    }
                });
            }
        }
    }
    out
}

/// The open missing-mechanic findings in the run's questions: unanswered
/// ones that carry a finding, counted once a part.
fn missing_open(dir: &Path) -> Option<u64> {
    let v: Value =
        serde_json::from_slice(&std::fs::read(dir.join("background").join("state.json")).ok()?)
            .ok()?;
    let parts: BTreeSet<String> = v["questions"]
        .as_array()?
        .iter()
        .filter(|q| q.get("missing").is_some_and(|m| !m.is_null()) && q["answer"].is_null())
        .map(|q| {
            q["path"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| q["id"].to_string())
        })
        .collect();
    Some(parts.len() as u64)
}

/// The mutants the game's corpus names, and those a kept clip catches
/// (its `guards`).
fn mutants(tdir: &Path) -> Option<(u64, u64)> {
    let set = crate::clipset::read_game(tdir).ok()?;
    if set.mutant.is_empty() {
        return None;
    }
    let caught: BTreeSet<&str> = set
        .clips
        .iter()
        .flat_map(|c| c.guards.iter().map(String::as_str))
        .collect();
    let all: BTreeSet<&str> = set
        .mutant
        .iter()
        .filter_map(|m| m.get("name").and_then(|n| n.as_str()))
        .collect();
    Some((all.intersection(&caught).count() as u64, all.len() as u64))
}

/// The scores gp serve's background work keeps for a run
/// (`background/state.json`): behaviour alone, each clip's frames together
/// and whether it is exact. gameprinter's serve keeps no picture score.
fn scores(state: &Value, player: Option<&str>) -> Scores {
    let scored: Vec<&Value> = state["scores"]
        .as_object()
        .into_iter()
        .flat_map(|m| m.values())
        .filter(|s| s["error"].is_null())
        .collect();
    let together = if scored.is_empty() {
        0
    } else {
        scored
            .iter()
            .map(|s| s["permille"].as_u64().unwrap_or(0).min(1000))
            .sum::<u64>()
            / scored.len() as u64
    };
    let mut measured_kinds = BTreeSet::new();
    if let Some(p) = player
        && !scored.is_empty()
    {
        measured_kinds.insert(p.to_owned());
    }
    Scores {
        clips: scored.len() as u64,
        exact: scored.iter().filter(|s| s["exact"] == json!(true)).count() as u64,
        behaviour: together / 10,
        together_permille: together,
        fidelity: together / 10,
        picture: None,
        measured_kinds,
    }
}

/// Reads the facts for `template`, and `run` when given, and the cached
/// measure. Writes nothing.
pub(crate) fn gather(
    paths: &gp_evidence::Paths,
    template: &str,
    run: Option<&str>,
) -> Result<Facts> {
    let tdir = template_dir(template)?;
    let rdir = run.map(|r| paths.home.join("work").join("serve").join(r));
    // The model and mapping: the run's working copy, else the game's.
    let mdir = model_dir(&tdir, rdir.as_deref());
    let model = crate::mechanics::load_model(&mdir)
        .with_context(|| format!("the model in {}", mdir.display()))?;
    let mapping_text = std::fs::read_to_string(mdir.join("mapping.toml"))
        .with_context(|| format!("reading {}", mdir.join("mapping.toml").display()))?;
    let mapping = gp_interp::mapping::Mapping::parse(&mapping_text).map_err(anyhow::Error::msg)?;
    let source = source_facts(&tdir, paths);
    let labels = source.labels.as_deref().map(ram_labels).unwrap_or_default();
    let mapped_kinds: BTreeSet<String> = mapping
        .fields
        .iter()
        .filter_map(|f| {
            f.path
                .strip_prefix("entities.")
                .and_then(|p| p.split(['[', '.']).next())
                .map(str::to_owned)
        })
        .collect();
    let mut game_levels = BTreeSet::new();
    if let Some(s) = &mapping.slots {
        game_levels.extend(s.levels.iter().map(|l| l.level.clone()));
    }
    if game_levels.is_empty() {
        game_levels.extend(model.levels.keys().cloned());
    }
    let player = gp_interp::scenario::player(&model);
    let mut sc = None;
    let mut missing = None;
    if let Some(d) = &rdir {
        let state: Value = std::fs::read(d.join("background").join("state.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(Value::Null);
        sc = Some(scores(&state, player.as_deref()));
        missing = missing_open(d);
    }
    let measured = crate::progress_measure::load(paths, &tdir, run);
    Ok(Facts {
        template: template.to_owned(),
        run: run.map(str::to_owned),
        source,
        measured,
        model_now: crate::progress_measure::model_key(&mdir),
        named_ram: named_ram(&mapping),
        cell_ram: cell_ram(&mapping),
        ignored: mapping
            .ignore
            .iter()
            .flat_map(|i| {
                i.addrs()
                    .map(move |a| (a & 0x7FF, (i.role.clone(), i.review)))
            })
            .collect(),
        labels,
        slots: mapping.slots.is_some(),
        slot_kinds: slot_kinds(&mapping),
        state_ids: state_ids(&mapping),
        entities: model.entities.keys().cloned().collect(),
        player,
        mapped_kinds,
        game_levels,
        model_levels: model.levels.keys().cloned().collect(),
        level_names: model
            .levels
            .keys()
            .map(|l| (l.clone(), gp_interp::scenario::level_words(&model, l)))
            .collect(),
        starts: None,
        code_facts: code_facts(&model),
        missing,
        mutants: mutants(&tdir),
        scores: sc,
        effort: None,
    })
}

/// The model a run plays: its working copy (gp serve's
/// `$GP_HOME/work/serve/GAME/model/`) once there is one, else the game's.
fn model_dir(game: &Path, run: Option<&Path>) -> PathBuf {
    match run.map(|d| d.join("model")) {
        Some(c) if c.join(gp_model::MODEL_FILE).is_file() => c,
        _ => game.to_owned(),
    }
}

/// The comment that starts the ignore list `--write-ignore` writes; the
/// list runs from it to the end of the file.
const IGNORE_MARK: &str = "# The RAM bytes the model needs no field for";

/// The ignore list from a measure's roles: every byte the playthrough
/// changes in play that isn't game state by the trace and that `mapping`
/// doesn't name, runs of one role and reason as one entry, as TOML.
pub(crate) fn ignore_list(
    m: &Measured,
    named: &BTreeSet<u16>,
    labels: &BTreeMap<u16, String>,
    code_labels: &BTreeMap<u16, String>,
) -> (String, usize) {
    let changed: BTreeSet<u16> = m.ram.iter().copied().collect();
    let rows: Vec<&gp_nes::ramrole::RamRole> = m
        .roles
        .roles
        .iter()
        .filter(|r| r.role != gp_nes::ramrole::Role::State)
        .filter(|r| changed.contains(&r.addr) && !named.contains(&r.addr))
        .collect();
    let routine = |r: &u32| match code_labels.get(&(*r as u16)) {
        Some(l) => format!("{l} (${r:04X})"),
        None => format!("${r:04X}"),
    };
    let mut out = format!(
        "\n{IGNORE_MARK}\n# (schema 21): input, counters, scratch, sound and pointers, from a\n# trace of every instruction of a playthrough on the original, by\n# `gp progress --write-ignore` (docs/progress.md). Written, not\n# hand-edited: write it again after a change. A byte marked review held\n# its rule on nearly every frame but not all; check it.\n# The playthrough: {} ({}).\n",
        m.name, m.playthrough
    );
    if !m.roles.sound_entries.is_empty() {
        let names: Vec<String> = m.roles.sound_entries.iter().map(routine).collect();
        out.push_str(&format!("# Sound routines from: {}.\n", names.join(", ")));
    }
    if !m.roles.input_routines.is_empty() {
        let names: Vec<String> = m.roles.input_routines.iter().map(routine).collect();
        out.push_str(&format!(
            "# The controller is read in: {}.\n",
            names.join(", ")
        ));
    }
    let mut n = 0;
    let mut i = 0;
    while i < rows.len() {
        let r = rows[i];
        let mut len = 1u16;
        while let Some(next) = rows.get(i + len as usize) {
            if next.addr == r.addr + len
                && next.role == r.role
                && next.why == r.why
                && next.review == r.review
            {
                len += 1;
            } else {
                break;
            }
        }
        let names: Vec<&str> = (r.addr..r.addr + len)
            .filter_map(|a| labels.get(&a).map(String::as_str))
            .filter(|l| !l.contains('+'))
            .collect();
        out.push_str("\n[[ignore]]\n");
        out.push_str(&format!("ram = 0x{:04X}", r.addr));
        if !names.is_empty() {
            out.push_str(&format!("  # {}", names.join(", ")));
        }
        out.push('\n');
        if len > 1 {
            out.push_str(&format!("len = {len}\n"));
        }
        out.push_str(&format!("role = \"{}\"\n", r.role.name()));
        out.push_str(&format!("why = {}\n", toml::Value::String(r.why.clone())));
        if r.review {
            out.push_str("review = true\n");
        }
        n += 1;
        i += len as usize;
    }
    (out, n)
}

/// Replaces `file`'s ignore list with the measure's, raising its schema
/// to 21. Returns the entries written.
fn write_ignore(file: &Path, m: &Measured, f: &Facts) -> Result<usize> {
    let text =
        std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let mut keep = match text.find(IGNORE_MARK) {
        Some(at) => text[..at].trim_end().to_owned(),
        None => {
            anyhow::ensure!(
                !text.contains("[[ignore]]"),
                "{} has an [[ignore]] list that --write-ignore didn't write: remove it first",
                file.display()
            );
            text.trim_end().to_owned()
        }
    };
    keep.push('\n');
    let mapping = gp_interp::mapping::Mapping::parse(&text).map_err(anyhow::Error::msg)?;
    if mapping.schema < 21 {
        let line = format!("schema = {}\n", mapping.schema);
        anyhow::ensure!(
            keep.contains(&line),
            "{}: no `schema = {}` line to raise to 21",
            file.display(),
            mapping.schema
        );
        keep = keep.replacen(&line, "schema = 21\n", 1);
    }
    let code_labels = code_labels(f.source.labels.as_deref());
    let (list, n) = ignore_list(m, &named_ram(&mapping), &f.labels, &code_labels);
    keep.push_str(&list);
    gp_interp::mapping::Mapping::parse(&keep)
        .map_err(|e| anyhow::anyhow!("the mapping with its ignore list: {e}"))?;
    std::fs::write(file, keep).with_context(|| format!("writing {}", file.display()))?;
    Ok(n)
}

/// The code's labels (its routines, and labels at or above $8000), by
/// address.
fn code_labels(path: Option<&Path>) -> BTreeMap<u16, String> {
    let Some(file) = path
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| crate::labels::parse(&t).ok())
    else {
        return BTreeMap::new();
    };
    let mut out = BTreeMap::new();
    for r in &file.routine {
        out.entry(r.addr).or_insert_with(|| r.name.clone());
    }
    for l in &file.label {
        if l.addr >= 0x8000 {
            out.entry(l.addr).or_insert_with(|| l.name.clone());
        }
    }
    out
}

/// `gp progress`.
pub(crate) fn run(args: &ProgressArgs, paths: &gp_evidence::Paths) -> Result<Outcome> {
    let mut notes = Vec::new();
    if args.measure {
        let run = args
            .run
            .as_deref()
            .context("--measure needs --run: the run's ROM, model and recordings")?;
        let game = template_dir(&args.template)?;
        let mdir = model_dir(
            &game,
            Some(&paths.home.join("work").join("serve").join(run)),
        );
        let m = crate::progress_measure::measure(
            paths,
            &game,
            &mdir,
            run,
            &crate::progress_measure::Opts {
                playthrough: args.playthrough.clone(),
                clips: !args.no_clips,
                whole: !args.no_whole,
                module: args.module.as_deref(),
            },
        )?;
        notes.push(format!("measured in {:.0} s", m.seconds));
    }
    let f = gather(paths, &args.template, args.run.as_deref())?;
    if let Some(file) = &args.write_ignore {
        let m = f
            .measured
            .as_ref()
            .context("--write-ignore needs a measure: run with --measure first")?;
        anyhow::ensure!(
            m.roles != gp_nes::ramrole::Classified::default() || m.ram.is_empty(),
            "--write-ignore needs the RAM roles from a trace of the 6502: gp has no Z80 counterpart of gp_nes::ramrole yet, so an arcade game's ignore list is written by hand"
        );
        let n = write_ignore(file, m, &f)?;
        notes.push(format!(
            "wrote {n} [[ignore]] entries to {}",
            file.display()
        ));
    }
    let f = if args.write_ignore.is_some() {
        gather(paths, &args.template, args.run.as_deref())?
    } else {
        f
    };
    let v = json_of(&f);
    let mut text = text_of(&f, &v);
    for n in notes {
        text.push('\n');
        text.push_str(&n);
    }
    Ok(Outcome::ok(v, text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress_measure::{ClipCode, FirstSplit, LevelRun, Whole};
    use gp_nes::ramrole::{Classified, RamRole, Role};

    fn facts() -> Facts {
        Facts {
            template: "t".into(),
            run: Some("r".into()),
            source: SourceFacts {
                id: "game".into(),
                registered: true,
                verified: true,
                rom: Some("game.nes".into()),
                labels: Some("labels.toml".into()),
                labels_recipe: String::new(),
            },
            named_ram: [0x10, 0x11, 0x12].into(),
            labels: [(0x13, "Timer".to_owned())].into(),
            slots: true,
            slot_kinds: [((0, 6), "goomba".to_owned()), ((0, 0), "koopa".to_owned())].into(),
            entities: ["mario", "goomba", "koopa"].map(String::from).into(),
            player: Some("mario".into()),
            game_levels: ["w1_1", "w1_2"].map(String::from).into(),
            level_names: [("w1_1".to_owned(), "World 1-1".to_owned())].into(),
            model_levels: ["w1_1"].map(String::from).into(),
            starts: Some((3, 4)),
            code_facts: (80, 100),
            missing: Some(2),
            mutants: Some((9, 10)),
            scores: Some(Scores {
                clips: 10,
                exact: 7,
                behaviour: 80,
                together_permille: 955,
                fidelity: 70,
                picture: Some(40),
                measured_kinds: ["mario", "goomba"].map(String::from).into(),
            }),
            ..Facts::default()
        }
    }

    fn measured() -> Measured {
        Measured {
            playthrough: "abc".into(),
            name: "tas".into(),
            frames: 1000,
            play_frames: 900,
            ram: vec![0x10, 0x11, 0x13, 0x14],
            kinds: vec![(0, 6), (0, 0), (0, 0x33)],
            code: 400,
            clips: Some(ClipCode {
                clips: 10,
                code: 330,
                shared: 300,
                only: 30,
            }),
            whole: Some(Whole {
                from: 60,
                compared: 800,
                first: Some(FirstSplit {
                    at: 200,
                    frame: 200,
                    original_frame: 260,
                    level: "w1_1".into(),
                    paths: vec!["entities.mario[0].x".into()],
                }),
                player: Some(FirstSplit {
                    at: 400,
                    frame: 400,
                    original_frame: 460,
                    level: "w1_1".into(),
                    paths: vec!["entities.mario[0].x".into()],
                }),
                resync: Some((700, 800)),
                levels: vec![
                    LevelRun {
                        level: "w1_1".into(),
                        from: 60,
                        compared: 500,
                        first: Some(FirstSplit {
                            at: 200,
                            frame: 200,
                            original_frame: 260,
                            level: "w1_1".into(),
                            paths: vec!["entities.mario[0].x".into()],
                        }),
                        cant: None,
                    },
                    LevelRun {
                        level: "w1_2".into(),
                        from: 600,
                        compared: 300,
                        first: None,
                        cant: None,
                    },
                ],
            }),
            ..Measured::default()
        }
    }

    fn stage(f: &Facts, key: &str) -> Stage {
        ladder(f).into_iter().find(|s| s.key == key).unwrap()
    }

    #[test]
    fn a_percent_is_rounded_down_so_100_means_all() {
        assert_eq!(percent(0, 0), 0);
        assert_eq!(percent(999, 1000), 99);
        assert_eq!(percent(1000, 1000), 100);
        assert_eq!(percent(1, 3), 33);
    }

    #[test]
    fn the_source_counts_four_checks() {
        let mut f = facts();
        let s = stage(&f, "source");
        assert_eq!((s.percent, s.done), (Some(100), true));
        f.source.labels = None;
        f.source.labels_recipe = "just smb-labels".into();
        let s = stage(&f, "source");
        assert_eq!((s.with, s.of, s.done), (Some(3), Some(4), false));
        assert_eq!(s.measure.as_deref(), Some("just smb-labels"));
        assert!(s.says.contains("labels from its code"), "{}", s.says);
    }

    #[test]
    fn the_heavy_stages_say_not_measured_with_the_command() {
        let f = facts();
        for key in ["memory", "objects", "whole"] {
            let s = stage(&f, key);
            assert_eq!(s.percent, None, "{key}");
            assert!(!s.done);
            assert_eq!(
                s.measure.as_deref(),
                Some("gp progress --template t --run r --measure"),
                "{key}"
            );
        }
        assert_eq!(stage(&f, "clips").percent, None);
    }

    #[test]
    fn the_memory_map_counts_the_bytes_the_playthrough_changes() {
        let mut f = facts();
        f.measured = Some(measured());
        let s = stage(&f, "memory");
        assert_eq!((s.with, s.of, s.percent), (Some(2), Some(4), Some(50)));
        assert_eq!(s.also[0], "0 of the 1 that have a label are named");
        assert_eq!(
            s.also[1], "not named yet: Timer ($0013) and $0014",
            "{:?}",
            s.also
        );
        f.named_ram.extend([0x13, 0x14]);
        assert!(stage(&f, "memory").done);
    }

    fn roles(list: &[(u16, Role)]) -> Classified {
        Classified {
            roles: list
                .iter()
                .map(|&(addr, role)| RamRole {
                    addr,
                    role,
                    why: format!("made up {}", role.name()),
                    review: addr == 0x14,
                })
                .collect(),
            sound_entries: vec![0x9000],
            input_routines: vec![],
        }
    }

    #[test]
    fn the_memory_map_counts_only_game_state_bytes() {
        let mut f = facts();
        let mut m = measured();
        m.roles = roles(&[
            (0x10, Role::Input),
            (0x11, Role::State),
            (0x13, Role::State),
            (0x14, Role::Scratch),
        ]);
        f.measured = Some(m.clone());
        // No ignore list yet: the trace's say so.
        let s = stage(&f, "memory");
        assert_eq!((s.with, s.of), (Some(2), Some(4)));
        assert!(
            s.also
                .iter()
                .any(|a| a.starts_with("the trace says 1 more aren't game state")),
            "{:?}",
            s.also
        );
        // The list leaves $14 out; $10 is named, so it counts whatever the
        // list says.
        f.ignored = [
            (0x14, ("scratch".to_owned(), true)),
            (0x10, ("input".to_owned(), false)),
        ]
        .into();
        let s = stage(&f, "memory");
        assert_eq!((s.with, s.of), (Some(2), Some(3)), "{s:?}");
        assert_eq!(
            s.says,
            "2 of 3 game-state RAM bytes the game changes in play are named"
        );
        assert!(
            s.also.iter().any(|a| a
                == "1 of the 4 bytes it changes are left out by the mapping's ignore list: scratch 1; 1 flagged for review"),
            "{:?}",
            s.also
        );
        f.named_ram.insert(0x13);
        assert!(stage(&f, "memory").done);
    }

    #[test]
    fn the_level_grid_the_cells_read_is_named() {
        let m = gp_interp::mapping::Mapping::parse(
            "schema = 14\nmodel = \"m\"\nsource = \"s\"\nalign = \"polled\"\nfield = []\n[cells]\npages = [0x500, 0x5D0]\npage_columns = 16\nrows = 13\ntop = 2\nview = 17\nkinds = [\"coin\"]\n",
        )
        .unwrap();
        let c = cell_ram(&m);
        assert_eq!(c.len(), 416);
        assert!(c.contains(&0x500) && c.contains(&0x69F) && !c.contains(&0x6A0));
        assert_eq!(named_ram(&m), c);
    }

    #[test]
    fn the_ignore_list_is_written_in_runs_with_labels() {
        let mut m = measured();
        m.name = "tas".into();
        m.ram = vec![0x10, 0x11, 0x12, 0x13, 0x14, 0x20];
        m.roles = roles(&[
            (0x10, Role::Sound),
            (0x11, Role::Sound),
            (0x12, Role::Sound),
            (0x13, Role::Input),
            (0x14, Role::Scratch),
            (0x20, Role::State),
        ]);
        let labels = [(0x10, "SfxTimer".to_owned()), (0x13, "Pad".to_owned())].into();
        let code = [(0x9000, "SoundEngine".to_owned())].into();
        let (text, n) = ignore_list(&m, &[0x12].into(), &labels, &code);
        assert_eq!(n, 3, "{text}");
        assert!(
            text.contains("# Sound routines from: SoundEngine ($9000)."),
            "{text}"
        );
        assert!(
            text.contains("[[ignore]]\nram = 0x0010  # SfxTimer\nlen = 2\nrole = \"sound\"\nwhy = \"made up sound\"\n"),
            "{text}"
        );
        assert!(
            text.contains(
                "ram = 0x0014\nrole = \"scratch\"\nwhy = \"made up scratch\"\nreview = true\n"
            ),
            "{text}"
        );
        assert!(
            !text.contains("0x0012") && !text.contains("0x0020"),
            "{text}"
        );
        // It parses as a mapping's list.
        let mapping = format!(
            "schema = 21\nmodel = \"m\"\nsource = \"s\"\nalign = \"polled\"\nfield = []\n{text}"
        );
        let parsed = gp_interp::mapping::Mapping::parse(&mapping).unwrap();
        assert_eq!(parsed.ignore.len(), 3);
        assert_eq!(
            parsed.ignore[0].addrs().collect::<Vec<_>>(),
            vec![0x10, 0x11]
        );
        // Schema 20 refuses it.
        assert!(
            gp_interp::mapping::Mapping::parse(&mapping.replace("schema = 21", "schema = 20"))
                .is_err()
        );
    }

    #[test]
    fn objects_are_the_slot_kinds_the_playthrough_shows() {
        let mut f = facts();
        f.measured = Some(measured());
        let s = stage(&f, "objects");
        assert_eq!((s.with, s.of), (Some(2), Some(3)));
        assert_eq!(s.also[0], "not modelled: ID $33", "{:?}", s.also);
        assert!(s.also[1].starts_with("1 of 3 measured"), "{:?}", s.also);
        assert!(!s.done);
        // A table whose ID byte is its state: one kind for the values no
        // kind names, and an entity counted once for all its IDs.
        let mut m = measured();
        m.kinds = vec![(2, 0x81), (2, 0x82), (2, 0x05), (2, 0x06)];
        f.measured = Some(m);
        f.state_ids = [2].into();
        f.slot_kinds.insert((2, 0x81), "hammer".into());
        f.slot_kinds.insert((2, 0x82), "hammer".into());
        f.entities.insert("hammer".into());
        let s = stage(&f, "objects");
        assert_eq!((s.with, s.of), (Some(1), Some(2)), "{s:?}");
        assert_eq!(s.also[0], "not modelled: group 2's other states");
        // Without slots: the model's kinds a field measures.
        f.slots = false;
        f.mapped_kinds = ["goomba"].map(String::from).into();
        let s = stage(&f, "objects");
        assert_eq!((s.with, s.of), (Some(1), Some(3)));
    }

    #[test]
    fn levels_count_the_model_and_the_starts() {
        let mut f = facts();
        let s = stage(&f, "levels");
        assert_eq!((s.with, s.of), (Some(1), Some(2)));
        assert!(
            s.also.iter().any(|a| a.contains("3 of 4 starts")),
            "{:?}",
            s.also
        );
        f.model_levels.insert("w1_2".into());
        f.starts = Some((4, 4));
        assert!(stage(&f, "levels").done);
    }

    #[test]
    fn mechanics_are_done_when_no_finding_is_open() {
        let mut f = facts();
        let s = stage(&f, "mechanics");
        assert_eq!((s.percent, s.done), (Some(80), false));
        f.missing = Some(0);
        assert!(stage(&f, "mechanics").done);
        f.missing = None;
        let s = stage(&f, "mechanics");
        assert_eq!(s.measure.as_deref(), Some("gp solve --run r --missing"));
        assert!(!s.done);
    }

    #[test]
    fn clips_count_the_playthroughs_code_and_the_mutants() {
        let mut f = facts();
        f.measured = Some(measured());
        let s = stage(&f, "clips");
        assert_eq!((s.with, s.of, s.percent), (Some(300), Some(400), Some(75)));
        assert!(
            s.also
                .iter()
                .any(|a| a == "9 of 10 mutants caught by a kept clip")
        );
        assert!(s.also.iter().any(|a| a.starts_with("30 addresses")));
        assert!(!s.done);
    }

    #[test]
    fn behaviour_and_picture_read_the_scores() {
        let f = facts();
        let s = stage(&f, "behaviour");
        assert_eq!((s.with, s.of, s.percent), (Some(7), Some(10), Some(70)));
        assert!(
            s.also.contains(&"frames together 95.5%".to_owned()),
            "{:?}",
            s.also
        );
        let p = stage(&f, "picture");
        assert_eq!(p.percent, Some(40));
        assert!(!p.done);
        let mut f = facts();
        f.scores = None;
        assert_eq!(stage(&f, "behaviour").percent, None);
    }

    #[test]
    fn the_whole_game_is_the_frames_before_the_first_part() {
        let mut f = facts();
        f.measured = Some(measured());
        let s = stage(&f, "whole");
        assert_eq!((s.with, s.of, s.percent), (Some(200), Some(800), Some(25)));
        assert_eq!(
            s.says,
            "200 of 800 frames play identically from the original's state at its frame 60, then they part at its frame 260 in World 1-1: Mario's place across"
        );
        // Level by level: each from its own start.
        assert!(
            s.also.iter().any(|a| a.starts_with(
                "level by level, each from the original's state at its start: 500 of 800 frames (62%) before each level's first part; 1 of 2 levels play through identically"
            )),
            "{:?}",
            s.also
        );
        assert_eq!(s.table.len(), 2);
        assert_eq!(
            (s.table[0].name.as_str(), s.table[0].with, s.table[0].of),
            ("World 1-1", 200, 500)
        );
        assert_eq!((s.table[1].with, s.table[1].of), (300, 300));
        assert_eq!(s.table[1].says, "all 300 frames");
        assert!(s.also.iter().any(|a| a.contains("700 of 800")));
        assert!(
            s.also
                .iter()
                .any(|a| a == "Mario's own measures hold for 400 frames (50%), then part at the original's frame 460 in World 1-1: Mario's place across"),
            "{:?}",
            s.also
        );
        let mut m = measured();
        m.whole.as_mut().unwrap().first = None;
        f.measured = Some(m);
        let s = stage(&f, "whole");
        assert_eq!((s.percent, s.done), (Some(100), true));
    }

    #[test]
    fn a_timers_count_is_said_in_words() {
        assert_eq!(path_words("ticks.star[0]"), "the star timer's count");
        assert_eq!(capital("mario"), "Mario");
        assert_eq!(capital(""), "");
    }

    #[test]
    fn the_json_names_the_next_stage() {
        let mut f = facts();
        f.measured = Some(measured());
        let v = json_of(&f);
        assert_eq!(v["stages"].as_array().unwrap().len(), 9);
        assert_eq!(v["done"], 1);
        assert_eq!(v["next"], 2);
        assert_eq!(v["version"], VERSION);
        let t = text_of(&f, &v);
        assert!(
            t.starts_with("Progress on t (run r): 1 of 9 stages done"),
            "{t}"
        );
    }

    #[test]
    fn effort_says_the_fixes_by_source_and_llm_calls() {
        let b = json!({"work_s": 7200.0, "rounds": 40, "scores_read": 300,
            "kept_by_source": {"solver:arith": 1.0}, "fixed_by_source": {},
            "llm": {"clip_agent_calls": 6, "request_agent_calls": 0, "calls_per_fix": null}});
        let l = effort_lines(&b);
        assert_eq!(
            l[0],
            "2.0 hours of work over 40 rounds; 300 clip scores read"
        );
        assert_eq!(l[1], "changes kept by source: solver:arith 1");
        assert_eq!(l[2], "clips fixed by source: none");
        assert_eq!(l[3], "6 LLM calls logged, no clip fixed by an LLM yet");
    }

    #[test]
    fn the_mapping_names_slot_bytes_for_every_slot() {
        let m = gp_interp::mapping::Mapping::parse(
            r#"schema = 9
model = "m"
source = "s"
align = "polled"

[[field]]
path = "entities.p[0].x"
ram = [0x86, 0x6D]
frac = 0x400

[slots]
count = 2
active = 0x0F
id = 0x16
clock = 0x09

[[slots.field]]
path = "x"
ram = [0x87]

[[slots.kind]]
entity = "goomba"
ids = [6]
"#,
        )
        .unwrap();
        let n = named_ram(&m);
        for a in [0x86, 0x6D, 0x400, 0x0F, 0x10, 0x16, 0x17, 0x09, 0x87, 0x88] {
            assert!(n.contains(&a), "{a:#x} in {n:?}");
        }
        assert_eq!(n.len(), 10);
        assert_eq!(few(&["a", "b", "c"].map(String::from)), "a, b and c");
        assert_eq!(
            slot_kinds(&m).get(&(0, 6)).map(String::as_str),
            Some("goomba")
        );
    }
}
