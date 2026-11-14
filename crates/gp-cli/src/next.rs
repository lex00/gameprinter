//! `gp progress --next`: where the port of a game stands and the one next
//! thing, for the person and their agent (docs/progress.md, "What next").
//! The same answer is the page's status line (`GET /api/next`,
//! serve/next.rs) and the MCP tool `next`.
//!
//! It only reads: the game's folder, the quest log and its sessions, the
//! gate's scores, gp serve's background state, the last `gp progress
//! --measure` and a published record. It runs nothing (no score, no ROM, no
//! build), so a measure that is missing is named as the next step, and a new
//! conversation gets the same answer from the same files. It suggests; it
//! never starts work.
//!
//! [`answer`] is the computation, from [`Facts`] alone, so the tests give it
//! made-up inputs; [`gather`] reads the facts. The twelve stages are the
//! porting road of the design note (guided-loop.md, section 2a), P1 (no
//! game file) to P12 (online). The next step is chosen in this order, the
//! first that applies: something that stops all work; a step that costs
//! money or publishes, which waits for the person's yes; a question only
//! the person can answer; a hand-off the agent hasn't answered; free work
//! for the tools; work for the agent; play only the person can give; the
//! stage's own step. When the next step is the tools' or the agent's, the
//! person's own item comes first among the others.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Result, bail};
use serde::Serialize;
use serde_json::{Value, json};

use crate::Outcome;

/// The answer's version: a change to its shape or stages bumps it.
pub(crate) const VERSION: &str = "next 1";
/// The porting stages.
const STAGES: u8 = 12;
/// How old a question on REFINE is (seconds) before it earns a clue.
const QUESTION_CLUE_S: u64 = 600;
/// How many other steps the answer offers.
const ALSO: usize = 2;

/// Who does a step: the tools (free, unattended; the agent may start it
/// without asking), the agent, or the person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Who {
    Tools,
    Agent,
    Person,
}

impl Who {
    fn words(self) -> &'static str {
        match self {
            Who::Tools => "the tools",
            Who::Agent => "your agent",
            Who::Person => "you",
        }
    }
}

/// Where a step is done: a button on the page, a command, an MCP tool.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub(crate) struct How {
    pub page: Option<String>,
    pub cli: Option<String>,
    pub mcp: Option<String>,
}

/// What the page's button does for a step.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "do", rename_all = "snake_case")]
pub(crate) enum Act {
    /// PLAY FROM HERE on a quest log row.
    PlayFrom { row: String },
    /// Open a tab.
    Tab { tab: &'static str },
    /// Start one of gp serve's page actions (serve/jobs.rs) by its name:
    /// LET THE TOOLS TRY is `triage`.
    Start { action: &'static str },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Step {
    pub who: Who,
    pub says: String,
    pub how: How,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub act: Option<Act>,
}

fn step(who: Who, says: impl Into<String>) -> Step {
    Step {
        who,
        says: says.into(),
        how: How::default(),
        act: None,
    }
}

impl Step {
    fn page(mut self, p: impl Into<String>) -> Self {
        self.how.page = Some(p.into());
        self
    }
    fn cli(mut self, c: impl Into<String>) -> Self {
        self.how.cli = Some(c.into());
        self
    }
    fn act(mut self, a: Act) -> Self {
        self.act = Some(a);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Running {
    pub what: String,
    pub look_again_s: u64,
}

/// A step that costs money or makes something public.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Yes {
    pub what: String,
    pub says: String,
    /// The ask `gp_run` left for it (gp_mcp::asks), which the page's card
    /// answers: its id, and `open` or `yes` (said yes, not used yet).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Clue {
    pub signal: &'static str,
    pub says: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct StageId {
    pub id: String,
    pub n: u8,
    pub of: u8,
    pub name: &'static str,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Answer {
    pub version: &'static str,
    pub journey: &'static str,
    pub subject: Option<String>,
    pub stage: StageId,
    pub stands: String,
    pub next: Step,
    pub also: Vec<Step>,
    pub running: Vec<Running>,
    pub waits_for_yes: Vec<Yes>,
    pub clue: Option<Clue>,
    /// A count the agent's changes to the game raise, so the page knows
    /// what it shows may be stale.
    pub changes: u64,
    /// The same for a button the person pressed (LET THE TOOLS TRY): the
    /// page refreshes its views with no note.
    pub page_changes: u64,
    pub facts: Value,
}

/// A quest log row as the loop reads it.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Row {
    pub mechanic: String,
    pub done: bool,
    pub triaged: bool,
    /// The triage's request when the row needs the person.
    pub you: Option<Request>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Request {
    pub start: String,
    pub todo: String,
    pub stop: String,
}

/// The clips' scores on the model as it stands.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Scores {
    pub clips: u64,
    pub exact: u64,
    /// The clip that goes wrong first, in words.
    pub first_wrong: Option<String>,
    /// Clips waiting to be scored on the model as it stands.
    pub to_score: u64,
    /// The scores are from an older model.
    pub stale: bool,
    /// The fidelity number: over the clips scored, the mean of how much of
    /// each plays the same as the original (an exact clip counts 1000).
    /// Behaviour only; pictures and sound aren't measured. The page's
    /// REFINE line computes the same from the same scores (refine/fold.js,
    /// fidelitySentence).
    pub fidelity_permille: u64,
}

/// The mean of some clips' permilles, rounded down (as the page does).
fn mean_permille(each: impl Iterator<Item = u64>) -> u64 {
    let (sum, n) = each.fold((0u64, 0u64), |(s, n), p| (s + p.min(1000), n + 1));
    sum.checked_div(n).unwrap_or(0)
}

/// A permille as a whole percent, rounded half up (as the page does).
fn percent(permille: u64) -> u64 {
    (permille + 5) / 10
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Question {
    pub clip: String,
    pub choices: usize,
    pub age_s: u64,
}

/// The whole-game measure (`gp progress --measure`, stage 9).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) enum Whole {
    #[default]
    NotMeasured,
    Stale,
    Parts {
        together: u64,
        of: u64,
        level: String,
    },
    Same,
}

/// gp serve's background work, as its page or its record says.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Background {
    pub on: bool,
    /// The clips a job has now.
    pub jobs: Vec<String>,
    pub error: Option<String>,
    /// Read from the server itself, so a page is open on it.
    pub page_open: bool,
}

/// What the page saw the person do, for a clue: one signal at a time.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct PageSignal {
    /// `idle`, `drops`, `hops` or `away`.
    pub signal: String,
    /// The row it is about (drops).
    pub row: Option<String>,
    /// How many times this signal's clue was shown before.
    pub shown: u32,
}

/// Stage 1's step: the person says where their copy of the game is. The
/// first screen says it in its own words already (serve's first_screen).
pub(crate) const BRING_YOUR_COPY: &str =
    "Tell your agent where your copy of the game is: a .nes file, or a .zip for an arcade game.";

/// Everything the answer reads.
#[derive(Debug, Clone, Default)]
pub(crate) struct Facts {
    pub game: Option<String>,
    pub title: String,
    /// The games in the games checkout.
    pub games: Vec<String>,
    /// The ROM files in the ROM folder, by name (read before any game).
    pub roms: Vec<String>,
    pub rom_found: bool,
    pub player: Option<String>,
    pub player_mapped: bool,
    pub rows: Vec<Row>,
    /// The game has a triage.toml.
    pub triage_plan: bool,
    pub session_clips: u64,
    /// Session clips the corpus doesn't hold yet (`walk 3`).
    pub not_kept: Vec<String>,
    pub corpus_clips: u64,
    pub scores: Option<Scores>,
    pub questions: Vec<Question>,
    /// Rows the person handed to the agent that it hasn't answered.
    pub handoffs: Vec<String>,
    /// What the person told the agent about a clip on REFINE that it
    /// hasn't answered (serve/tell.rs): (tell number, clip, words), oldest
    /// first.
    pub tells: Vec<(u64, String, String)>,
    /// Rows where the agent asked the person and waits: (row, words).
    pub asked: Vec<(String, String)>,
    pub whole: Whole,
    pub published: Option<String>,
    pub background: Background,
    /// Words the person typed on the page, newest last.
    pub notes: Vec<String>,
    pub page: Option<PageSignal>,
    /// The asks `gp_run` left that wait for the person's yes, and the
    /// yeses not used yet (gp_mcp::asks), of every game.
    pub asks: Vec<Value>,
    /// The agent's calls running now (gp_mcp::activity): what, in words.
    pub agent_runs: Vec<(String, u64)>,
    /// The same calls started by a button on the page (serve/jobs.rs),
    /// which run in gp serve's own process.
    pub page_runs: Vec<String>,
    /// How many times the agent changed the game through gp_run.
    pub changes: u64,
    /// How many times a button the person pressed changed it (the page
    /// refreshes quietly on these).
    pub page_changes: u64,
}

impl Facts {
    fn you_rows(&self) -> Vec<&Row> {
        self.rows
            .iter()
            .filter(|r| !r.done && r.you.is_some())
            .collect()
    }

    fn game(&self) -> &str {
        self.game.as_deref().unwrap_or("GAME")
    }
}

/// The names of a list: "a, b and 3 more".
fn few(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [a] => a.clone(),
        [a, b] => format!("{a} and {b}"),
        [a, b, c] => format!("{a}, {b} and {c}"),
        [a, b, rest @ ..] => format!("{a}, {b} and {} more", rest.len()),
    }
}

/// "1 clip", "3 clips".
fn n_of(n: u64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Words cut at about `max` characters, at a space.
fn cut(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let head: String = s.chars().take(max).collect();
    let at = head.rfind(' ').unwrap_or(head.len());
    format!("{}...", head[..at].trim_end_matches([',', ';', ':', '.']))
}

fn stage_name(n: u8) -> &'static str {
    match n {
        0 => "Which game",
        1 => "No game file",
        2 => "A game file, no game yet",
        3 => "Finding the player",
        4 => "First clips",
        5 => "Keeping the clips",
        6 => "Scoring",
        7 => "Fixing the model",
        8 => "Play only you can give",
        9 => "A full run",
        10 => "The full run parts",
        11 => "Ready to go online",
        _ => "Online",
    }
}

/// The stage the port is at: the first of the road not passed.
fn stage(f: &Facts) -> u8 {
    if f.game.is_none() {
        return if !f.games.is_empty() {
            0
        } else if f.roms.is_empty() {
            1
        } else {
            2
        };
    }
    if !f.player_mapped {
        return 3;
    }
    if f.session_clips == 0 && f.corpus_clips == 0 {
        return 4;
    }
    if !f.not_kept.is_empty() {
        return 5;
    }
    let Some(s) = &f.scores else { return 6 };
    if s.stale || s.to_score > 0 || s.clips == 0 {
        return 6;
    }
    if s.exact < s.clips {
        return if f.you_rows().is_empty() { 7 } else { 8 };
    }
    match f.whole {
        Whole::NotMeasured | Whole::Stale => 9,
        Whole::Parts { .. } => 10,
        Whole::Same if f.published.is_none() => 11,
        Whole::Same => 12,
    }
}

/// Where the work stands, in a sentence or two.
fn stands(f: &Facts, n: u8) -> String {
    let t = &f.title;
    let mut s = match n {
        0 => format!(
            "{} are here: {}. None was opened on the page yet.",
            n_of(f.games.len() as u64, "game", "games"),
            few(&f.games)
        ),
        1 => "No game file is on this computer yet.".into(),
        2 if f.roms.len() == 1 => format!(
            "Your game file {} is here, but no game is set up from it yet.",
            f.roms[0]
        ),
        2 => format!(
            "{} game files are here, but no game is set up from them yet.",
            f.roms.len()
        ),
        3 => format!("{t} is set up. The tools haven't found the player in the game's memory yet."),
        4 => format!("The tools know where the player is in {t}. No play is recorded yet."),
        5 => {
            let k = f.not_kept.len() as u64;
            let verb = if k == 1 { "is" } else { "are" };
            match f.scores.as_ref().filter(|s| s.clips > 0) {
                Some(sc) => format!(
                    "{} of {} clips of {t} play the same on the model as on the original{}, and {k} more {verb} recorded but not in the game's tests yet.",
                    sc.exact,
                    sc.clips,
                    if sc.stale {
                        " (scored on an older model)"
                    } else {
                        ""
                    }
                ),
                None => format!(
                    "{} of {t} {verb} recorded but not in the game's tests yet.",
                    n_of(k, "clip", "clips")
                ),
            }
        }
        6 => match &f.scores {
            Some(s) if !f.background.jobs.is_empty() => format!(
                "The tools are scoring {} of {t} on the model.",
                n_of(s.to_score.max(1), "clip", "clips")
            ),
            Some(s) if s.stale => format!(
                "The model of {t} changed since its clips were last scored. Then, {} of {} played the same as the original.",
                s.exact, s.clips
            ),
            Some(s) if s.to_score > 0 => format!(
                "{} of {t} haven't been scored on the model as it stands.",
                n_of(s.to_score, "clip", "clips")
            ),
            _ => format!(
                "{t} has {} in its tests, and none is scored yet.",
                n_of(f.corpus_clips.max(f.session_clips), "clip", "clips")
            ),
        },
        7 | 8 => {
            let s = f.scores.clone().unwrap_or_default();
            format!(
                "On average, {}% of a clip of {t} plays the same on the model as on the original. {} of {} clips play the same all the way.",
                percent(s.fidelity_permille),
                s.exact,
                s.clips
            )
        }
        9 => format!(
            "Every clip of {t} plays the same on the model. A full run hasn't been checked {}.",
            if f.whole == Whole::Stale {
                "since the last change"
            } else {
                "yet"
            }
        ),
        10 => match &f.whole {
            Whole::Parts {
                together,
                of,
                level,
            } => format!(
                "A full run of {t} plays the same for {together} of {of} frames, then the two part in {level}."
            ),
            _ => String::new(),
        },
        11 => {
            format!("A full run of {t} plays the same from start to end. It's ready to go online.")
        }
        _ => format!(
            "{t} is online at {}.",
            f.published.as_deref().unwrap_or("its link")
        ),
    };
    let you = f.you_rows().len() as u64;
    if (3..=10).contains(&n) && you > 0 {
        s.push_str(&format!(
            " {} {} you: play the tools couldn't get.",
            n_of(you, "quest item", "quest items"),
            if you == 1 { "needs" } else { "need" }
        ));
    } else if n > 3 && !f.handoffs.is_empty() {
        s.push_str(&format!(
            " Your agent hasn't answered your hand-off on {} yet.",
            few(&f.handoffs)
        ));
    }
    s
}

/// The person's play for a row that needs them.
fn play_step(f: &Facts, r: &Row) -> Step {
    let q = r.you.clone().unwrap_or_default();
    let mut says = format!("Press PLAY FROM HERE on {}.", r.mechanic);
    if !q.todo.is_empty() {
        says.push_str(&format!(" {}", sentence(&q.todo)));
    }
    if !q.stop.is_empty() {
        says.push_str(&format!(" Stop: {}", sentence(&q.stop)));
    }
    step(Who::Person, says)
        .page(format!("PLAY FROM HERE on {}", r.mechanic))
        .cli(format!("gp mechanic start {} {}", f.game(), r.mechanic))
        .act(Act::PlayFrom {
            row: r.mechanic.clone(),
        })
}

/// The tools' quest-log step: free, so the page's LET THE TOOLS TRY starts
/// it (serve/jobs.rs, `triage`) and the agent may start it unasked.
fn triage_step(g: &str, what: &str) -> Step {
    step(
        Who::Tools,
        format!("{what} Press LET THE TOOLS TRY, or your agent can start it."),
    )
    .page("LET THE TOOLS TRY on the status line")
    .cli(format!("gp mechanic triage {g}"))
    .act(Act::Start { action: "triage" })
}

/// Words as a sentence: a capital first and a stop at the end.
fn sentence(s: &str) -> String {
    let s = s.trim();
    let mut c = s.chars();
    let mut out: String = c
        .next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default();
    if !out.ends_with(['.', '!', '?']) {
        out.push('.');
    }
    out
}

/// The questions only the person can answer.
fn person_answers(f: &Facts) -> Vec<Step> {
    let mut out = Vec::new();
    for (row, words) in &f.asked {
        out.push(
            step(
                Who::Person,
                format!(
                    "Your agent asked about {row}: \"{}\" Answer it on CLIPS, over the game.",
                    cut(words, 140)
                ),
            )
            .page(format!("CLIPS, the open row {row}"))
            .act(Act::Tab { tab: "clips" }),
        );
    }
    if let Some(q) = f.questions.first() {
        out.push(
            step(
                Who::Person,
                format!(
                    "The tools are stuck on {} and need you to pick one of {} answers. It takes a minute, on REFINE.",
                    q.clip,
                    q.choices.max(2)
                ),
            )
            .page("REFINE, the questions")
            .act(Act::Tab { tab: "refine" }),
        );
    }
    out
}

/// The answer, from the facts alone.
pub(crate) fn answer(f: &Facts) -> Answer {
    let n = stage(f);
    let g = f.game().to_owned();
    let t = &f.title;
    let mut running = Vec::new();
    let mut waits = Vec::new();
    let you = f.you_rows();
    let to_score = f
        .scores
        .as_ref()
        .map_or(0, |s| if s.stale { s.clips.max(1) } else { s.to_score });
    for (what, _secs) in &f.agent_runs {
        running.push(Running {
            what: format!("your agent: {what}"),
            look_again_s: 30,
        });
    }
    for what in &f.page_runs {
        running.push(Running {
            what: format!("started on the page: {what}"),
            look_again_s: 30,
        });
    }
    let scoring_now = f.background.on && (to_score > 0 || !f.background.jobs.is_empty());
    if scoring_now {
        let k = to_score.max(f.background.jobs.len() as u64);
        running.push(Running {
            what: format!("scoring {} on the model", n_of(k, "clip", "clips")),
            look_again_s: (k * 3).clamp(60, 600),
        });
    }

    // Candidates, in the order of the rules; the first is next.
    let mut steps: Vec<Step> = Vec::new();
    // 1. Something stops all work.
    if f.game.is_some() && !f.rom_found {
        steps.push(step(
            Who::Person,
            format!("Game Printer can't find your copy of {t} on this computer. Tell your agent where the file is: a .zip for an arcade game, a .nes file otherwise."),
        ));
    }
    if let Some(e) = &f.background.error {
        steps.push(
            step(
                Who::Agent,
                format!(
                    "The tools stopped scoring: {}. Your agent can look at why.",
                    cut(e, 120)
                ),
            )
            .page("REFINE")
            .act(Act::Tab { tab: "refine" }),
        );
    }
    // 2. A step that costs money or publishes: the agent's asks first.
    for a in &f.asks {
        let s = |k: &str| a[k].as_str().map(str::to_owned);
        let says = s("says").unwrap_or_default();
        waits.push(Yes {
            what: s("needs").unwrap_or_default(),
            says: says.clone(),
            id: s("id"),
            state: s("state"),
            game: s("game"),
        });
        steps.push(if a["state"] == json!("open") {
            step(
                Who::Person,
                format!("Your agent waits for your yes: {says} Press YES or NO on the card."),
            )
        } else {
            step(
                Who::Agent,
                format!("You said yes, so your agent can go ahead: {says}"),
            )
        });
    }
    if n == 11 {
        waits.push(Yes {
            what: "publish".into(),
            says: format!("Puts {t} online as a link. Anyone with the link can open it and play with their own copy of the game."),
            id: None,
            state: None,
            game: None,
        });
        steps.push(step(
            Who::Person,
            format!("Say yes and your agent will put {t} online as a link. Anyone with the link can open it."),
        ));
    }
    // 3. A question only the person can answer.
    steps.extend(person_answers(f));
    // 3b. A hand-off the agent hasn't answered.
    if !f.handoffs.is_empty() {
        let first = &f.handoffs[0];
        steps.push(
            step(
                Who::Agent,
                format!(
                    "Your agent can answer your hand-off on {}. If it hasn't yet, tell it you handed it over.",
                    few(&f.handoffs)
                ),
            )
            .cli(format!(
                "gp mechanic show {g} {first}, then gp mechanic say {g} {first} WORDS"
            )),
        );
    }
    // 3c. What the person told the agent about a clip, not answered.
    if let Some((id, clip, words)) = f.tells.first() {
        let more = f.tells.len() - 1;
        steps.push(
            step(
                Who::Agent,
                if more == 0 {
                    format!(
                        "Your agent can answer what you told it about {clip}: \"{}\" If it hasn't yet, ask it to read your note.",
                        cut(words, 120)
                    )
                } else {
                    format!(
                        "Your agent can answer what you told it about {clip} and {}. If it hasn't yet, ask it to read your notes.",
                        n_of(more as u64, "other clip", "other clips")
                    )
                },
            )
            .cli(format!(
                "gp agent tells --run {g} --open, then gp agent note --run {g} --clip {clip} --reply-to {id} WORDS"
            )),
        );
    }
    // 4. Free work for the tools that isn't running.
    if f.game.is_some() && f.player_mapped {
        if f.triage_plan && !f.rows.iter().any(|r| r.triaged) {
            steps.push(triage_step(
                &g,
                "The tools can try to get each quest item on their own, then list the few they can't.",
            ));
        }
        if n == 6 && !scoring_now && (to_score > 0 || f.corpus_clips > 0) {
            let k = if to_score > 0 {
                to_score
            } else {
                f.corpus_clips
            };
            steps.push(
                step(
                    Who::Tools,
                    format!(
                        "{} can be scored on the model as it stands. Press START THE WORK on REFINE and the tools score them.",
                        n_of(k, "clip", "clips")
                    ),
                )
                .page("START THE WORK on REFINE")
                .act(Act::Tab { tab: "refine" }),
            );
        }
        if n == 9 {
            steps.push(
                step(
                    Who::Tools,
                    "Your agent can start this now. The tools play a full run on the original and on the model and compare them. It takes 2 to 5 minutes.",
                )
                .cli(format!("gp progress --template {g} --run {g} --measure")),
            );
        }
    }
    // 5. Work for the agent.
    match n {
        3 => steps.push(
            step(
                Who::Agent,
                "Your agent can find where the game keeps the player. A short walk left and right on CLIPS helps.",
            )
            .cli(format!("gp model blind DIR --rom ROM, then the player's fields into games/{g}/mapping.toml")),
        ),
        5 => steps.push(
            step(
                Who::Agent,
                format!(
                    "Your agent can add {} to the game's tests ({}). Then the tools can score {}.",
                    n_of(f.not_kept.len() as u64, "new clip", "new clips"),
                    few(&f.not_kept),
                    if f.not_kept.len() == 1 { "it" } else { "them" }
                ),
            )
            .cli(format!("gp corpus import {g} --write")),
        ),
        7 | 8 => {
            let s = f.scores.clone().unwrap_or_default();
            if let Some(w) = &s.first_wrong {
                steps.push(
                    step(
                        Who::Agent,
                        format!("Your agent can read the game's code behind {w}, the clip that goes wrong first. You can watch on REFINE."),
                    )
                    .page("REFINE"),
                );
            }
        }
        10 => {
            if let Whole::Parts { together, level, .. } = &f.whole {
                steps.push(step(
                    Who::Agent,
                    format!("Your agent can find why the two part after {together} frames in {level}. It may ask you for a clip of that spot."),
                ));
            }
        }
        _ => {}
    }
    // 6. Play only the person can give.
    for r in &you {
        steps.push(play_step(f, r));
    }
    // 7. The stage's own step.
    match n {
        0 => steps.push(
            step(
                Who::Person,
                format!("Say which game to work on: {}.", few(&f.games)),
            )
            .cli("gp progress --next --template GAME"),
        ),
        1 => steps.push(step(Who::Person, BRING_YOUR_COPY)),
        2 if f.roms.len() == 1 => steps.push(
            step(
                Who::Agent,
                format!("Your agent can set up the game from {}. It takes about a minute and needs nothing from you.", f.roms[0]),
            )
            .cli("gp roms scan, then gp game new GAME --rom ROM --genre other"),
        ),
        2 => steps.push(step(
            Who::Person,
            format!("Say which of them to bring over: {}.", few(&f.roms)),
        )),
        4 => steps.push(triage_step(
            &g,
            "The tools can try to record each quest item on their own, then list the few they can't.",
        )),
        6 if scoring_now => steps.push(step(
            Who::Tools,
            format!(
                "Nothing for you right now. Look again in about {} min.",
                running.first().map_or(1, |r| r.look_again_s.div_ceil(60))
            ),
        )),
        12 => steps.push(step(
            Who::Person,
            "Share the link, or ask your agent for a new look on THEMES.",
        )),
        _ => {}
    }
    if steps.is_empty() {
        steps.push(step(
            Who::Agent,
            "Your agent can look at what the tools say on REFINE and pick the next fix.",
        ));
    }
    // Never two steps alike.
    let mut seen = BTreeSet::new();
    steps.retain(|s| seen.insert(s.says.clone()));
    let next = steps.remove(0);
    // The others, the person's first when the next isn't theirs.
    let mut also: Vec<Step> = Vec::new();
    if next.who != Who::Person {
        also.extend(steps.iter().filter(|s| s.who == Who::Person).cloned());
    }
    let rest: Vec<Step> = steps.into_iter().filter(|s| !also.contains(s)).collect();
    also.extend(rest);
    also.truncate(ALSO);

    let clue = clue(f, n, &next);
    let s = f.scores.clone().unwrap_or_default();
    Answer {
        version: VERSION,
        journey: "port",
        subject: f.game.clone(),
        stage: StageId {
            id: format!("P{n}"),
            n,
            of: STAGES,
            name: stage_name(n),
        },
        stands: stands(f, n),
        next,
        also,
        running,
        waits_for_yes: waits,
        clue,
        changes: f.changes,
        page_changes: f.page_changes,
        facts: json!({
            "title": f.title,
            "rom_found": f.rom_found,
            "player_mapped": f.player_mapped,
            "rows": f.rows.len(),
            "rows_done": f.rows.iter().filter(|r| r.done).count(),
            "needs_you": you.iter().map(|r| r.mechanic.clone()).collect::<Vec<_>>(),
            "handoffs": f.handoffs,
            "tells": f.tells.iter().map(|t| json!({"id": t.0, "clip": t.1})).collect::<Vec<_>>(),
            "asked": f.asked.iter().map(|a| a.0.clone()).collect::<Vec<_>>(),
            "session_clips": f.session_clips,
            "corpus_clips": f.corpus_clips,
            "not_kept": f.not_kept,
            "scores": f.scores.as_ref().map(|_| json!({
                "clips": s.clips, "exact": s.exact, "to_score": s.to_score,
                "stale": s.stale, "first_wrong": s.first_wrong,
                "fidelity_permille": s.fidelity_permille})),
            "questions": f.questions.len(),
            "whole": match &f.whole {
                Whole::NotMeasured => json!("not measured"),
                Whole::Stale => json!("on an older model"),
                Whole::Parts { together, of, level } => json!({"together": together, "of": of, "level": level}),
                Whole::Same => json!("the same"),
            },
            "published": f.published,
            "background": {"on": f.background.on, "jobs": f.background.jobs.len()},
        }),
    }
}

/// Words that show the person expects a ROM file or an install out of
/// this: Game Printer's result is a page that plays in a browser.
const OFF_TARGET: &[&str] = &[
    "download",
    "install",
    "compile",
    "emulator",
    "rom hack",
    "new rom",
    "edit the rom",
    "patch the rom",
    ".nes file",
    "cartridge",
];

/// The explanation of what Game Printer makes (PRODUCT-REVIEW.md, 2d).
const EXPLAIN: &str = "Game Printer writes down how a game plays: what moves, how fast, what happens when things touch. That written-down game is the model, and a web page plays it, so nothing gets installed. The pictures, sounds and level layouts come from your own copy of the game, read inside your browser and never sent anywhere. It never writes or changes a ROM file.";

/// One clue, when one applies: what the page saw first, then what the
/// files show.
fn clue(f: &Facts, n: u8, next: &Step) -> Option<Clue> {
    let mk = |signal, says: String| Some(Clue { signal, says });
    let you = f.you_rows();
    if let Some(p) = &f.page {
        let again = p.shown >= 3;
        match p.signal.as_str() {
            "idle" => {
                if let Some(r) = you.first() {
                    return mk(
                        "idle",
                        if again {
                            format!(
                                "If {} is hard to play, press HAND TO THE AGENT on it and your agent will look for another way, or pick another row.",
                                r.mechanic
                            )
                        } else if p.shown % 2 == 1 {
                            format!(
                                "{} still needs a bit of play. PLAY FROM HERE starts you at the spot and records; R stops.",
                                r.mechanic
                            )
                        } else {
                            format!(
                                "One bit of play would help: {}. PLAY FROM HERE puts you at the spot.",
                                r.mechanic
                            )
                        },
                    );
                }
            }
            "drops" => {
                let row = p.row.clone().unwrap_or_else(|| "this row".into());
                let stop = f
                    .rows
                    .iter()
                    .find(|r| r.mechanic == row)
                    .and_then(|r| r.you.as_ref())
                    .map(|q| q.stop.trim_end_matches('.').to_owned())
                    .filter(|s| !s.is_empty());
                return mk(
                    "drops",
                    if again {
                        format!(
                            "{row} is a hard one to get. Press HAND TO THE AGENT on it and your agent will look for another way."
                        )
                    } else {
                        match stop {
                            Some(s) => format!("The clip can be short. You only need to reach the stop: {s}. F5 saves a spot, F8 goes back to it."),
                            None => "The clip can be short: only the part that shows it. F5 saves a spot, F8 goes back to it.".into(),
                        }
                    },
                );
            }
            "hops" => {
                return mk(
                    "hops",
                    format!("Looking for where to start? Right now: {}", next.says),
                );
            }
            "away" => {
                if let Some((row, words)) = f.asked.first() {
                    return mk(
                        "away",
                        format!(
                            "Your agent asked about {row}: \"{}\" It's on CLIPS, over the game.",
                            cut(words, 100)
                        ),
                    );
                }
            }
            _ => {}
        }
    }
    if let Some(q) = f.questions.iter().find(|q| q.age_s >= QUESTION_CLUE_S)
        && f.background.jobs.is_empty()
    {
        return mk(
            "question",
            format!(
                "The tools are stuck on {} and need you to pick one of {} answers. It takes a minute, on REFINE.",
                q.clip,
                q.choices.max(2)
            ),
        );
    }
    if f.background.page_open && !f.background.on && n == 6 {
        return mk(
            "paused",
            "The tools are paused since the page started. Press START THE WORK on REFINE and they'll score your clips while you play.".into(),
        );
    }
    if let Some(note) = f.notes.last() {
        let low = note.to_lowercase();
        if OFF_TARGET.iter().any(|w| low.contains(w)) {
            return mk("words", EXPLAIN.into());
        }
    }
    None
}

/// The answer as text, for a terminal.
pub(crate) fn text_of(a: &Answer) -> String {
    let mut out = vec![
        format!(
            "{}stage {} of {}: {}",
            a.subject
                .as_deref()
                .map(|s| format!("{s}, "))
                .unwrap_or_default(),
            a.stage.n,
            a.stage.of,
            a.stage.name
        ),
        a.stands.clone(),
        String::new(),
    ];
    let line = |s: &Step, lead: &str| {
        let mut l = vec![format!("{lead} ({}): {}", s.who.words(), s.says)];
        if let Some(c) = &s.how.cli {
            l.push(format!("  in a terminal: {c}"));
        }
        l
    };
    out.extend(line(&a.next, "Next"));
    for s in &a.also {
        out.extend(line(s, "Or"));
    }
    for r in &a.running {
        out.push(format!(
            "Running: {}. Look again in about {} min.",
            r.what,
            r.look_again_s.div_ceil(60)
        ));
    }
    for y in &a.waits_for_yes {
        match (&y.id, y.state.as_deref()) {
            (Some(id), Some("yes")) => {
                out.push(format!("Said yes, not used yet (ask {id}): {}", y.says));
            }
            (Some(id), _) => out.push(format!("Waits for your yes (ask {id}): {}", y.says)),
            _ => out.push(format!("Waits for your yes: {}", y.says)),
        }
    }
    if let Some(c) = &a.clue {
        out.push(format!("Clue: {}", c.says));
    }
    out.join("\n")
}

/// The answer as JSON, with its text.
pub(crate) fn json_of(a: &Answer) -> Value {
    let mut v = serde_json::to_value(a).expect("the answer serializes");
    v["text"] = json!(text_of(a));
    v
}

// Reading the facts.

/// The games of the games checkout: folders holding package.toml or
/// model.toml.
fn games_in(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            let p = e.path();
            p.join("package.toml").is_file() || p.join(gp_model::MODEL_FILE).is_file()
        })
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .filter(|n| !n.starts_with('.'))
        .collect();
    out.sort();
    out
}

/// The ROM files in the ROM folder and its folders, by name; not the
/// sealed or patched ones.
fn rom_files(dir: &Path) -> Vec<String> {
    fn walk(d: &Path, depth: u8, out: &mut Vec<String>) {
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if p.is_dir() {
                let skip = ["sealed", "patched", "patches"]
                    .iter()
                    .any(|s| name.eq_ignore_ascii_case(s));
                if depth < 2 && !skip && !name.starts_with('.') {
                    walk(&p, depth + 1, out);
                }
            } else if crate::roms::is_rom_file(&p) {
                out.push(name);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, 0, &mut out);
    out.sort();
    out
}

fn read_json(p: &Path) -> Value {
    std::fs::read(p)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null)
}

fn age_s(p: &Path) -> u64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .map_or(0, |d| d.as_secs())
}

/// Whether a process is running, for the background's record.
fn alive(pid: u64) -> bool {
    if pid == u64::from(std::process::id()) {
        return true;
    }
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// The last turn of a session's talk: who and the words. A tell about a
/// clip echoed there (serve/tell.rs) is skipped: its own step says it.
fn last_turn(dir: &Path) -> Option<(String, String)> {
    let t = std::fs::read_to_string(dir.join("talk.ndjson")).ok()?;
    let v: Value = t
        .lines()
        .rev()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find(|v| v.get("tell").is_none())?;
    Some((
        v["by"].as_str()?.to_owned(),
        v["text"].as_str().unwrap_or("").to_owned(),
    ))
}

/// What the person typed (not the page's own words for a hand-off).
fn typed(text: &str) -> bool {
    !text.is_empty()
        && !["Let's do ", "Here is clip", "A sealed run"]
            .iter()
            .any(|p| text.starts_with(p))
}

/// The gate's scores (`$GP_HOME/work/refine/GAME/status.json`), stale
/// when the game's model or mapping changed since.
fn gate_scores(home: &Path, game: &str, gdir: &Path) -> Option<Scores> {
    let st = read_json(&home.join("work/refine").join(game).join("status.json"));
    let clips = st["clips"].as_array()?;
    if clips.is_empty() {
        return None;
    }
    let meets = |c: &Value| crate::mechanic::scores::meets(c);
    let exact = clips.iter().filter(|c| meets(c)).count() as u64;
    let first_wrong = clips.iter().find(|c| !meets(c)).map(|c| {
        let id = c["id"].as_str().unwrap_or("a clip");
        match c["first_split"]["clip_frame"].as_u64() {
            Some(f) => format!("{id} (at its frame {f})"),
            None => id.to_owned(),
        }
    });
    let mut b = std::fs::read(gdir.join(gp_model::MODEL_FILE)).unwrap_or_default();
    b.extend(std::fs::read(gdir.join(gp_interp::mapping::MAPPING_FILE)).unwrap_or_default());
    let now = gp_evidence::Hash::of(&b).to_string();
    let stale = st["model"]["hash"].as_str().is_some_and(|h| h != now);
    let fidelity_permille = mean_permille(clips.iter().map(|c| {
        let s = &c["straight"];
        match (s["together"].as_u64(), s["of"].as_u64()) {
            (Some(t), Some(of)) if of > 0 => t * 1000 / of,
            _ => 0,
        }
    }));
    Some(Scores {
        clips: clips.len() as u64,
        exact,
        first_wrong,
        to_score: 0,
        stale,
        fidelity_permille,
    })
}

/// gp serve's background scores for the run, on the model as it stands,
/// when it has any.
fn background_scores(paths: &gp_evidence::Paths, games: &Path, game: &str) -> Option<Scores> {
    let ctx = crate::serve::Ctx::for_cli(paths, Some(games)).ok()?;
    let run = crate::serve::game::run_named(&ctx, game).ok()?;
    let state = read_json(&run.dir.join("background").join("state.json"));
    let scores = state["scores"].as_object().filter(|s| !s.is_empty())?;
    let model = crate::serve::background::model_key(&ctx, &run);
    let active = crate::serve::background::active_now(&ctx, &run);
    let on_model: Vec<(&str, &Value)> = active
        .iter()
        .filter_map(|a| scores.get(&a.id).map(|s| (a.name.as_str(), s)))
        .filter(|(_, s)| s["model"].as_str() == Some(model.as_str()) && s["error"].is_null())
        .collect();
    if on_model.is_empty() {
        return None;
    }
    Some(Scores {
        clips: on_model.len() as u64,
        exact: on_model
            .iter()
            .filter(|s| s.1["exact"] == json!(true))
            .count() as u64,
        first_wrong: on_model
            .iter()
            .find(|s| s.1["exact"] != json!(true))
            .map(|s| s.0.to_owned()),
        to_score: (active.len() - on_model.len()) as u64,
        stale: false,
        fidelity_permille: mean_permille(on_model.iter().map(|s| {
            if s.1["exact"] == json!(true) {
                1000
            } else {
                s.1["permille"].as_u64().unwrap_or(0)
            }
        })),
    })
}

/// Reads the facts. `game` is the game asked for; without it, the game
/// whose page was opened last, or the only game. `live` is gp serve's
/// background as its server knows it; without it, its record.
pub(crate) fn gather(
    paths: &gp_evidence::Paths,
    games: Option<&Path>,
    game: Option<&str>,
    live: Option<Background>,
) -> Result<Facts> {
    let home = &paths.home;
    let games_dir = crate::roms_cmd::games_dir(games);
    let all = games_dir.as_deref().map(games_in).unwrap_or_default();
    let last = std::fs::read_to_string(home.join("work/serve/last-game"))
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|g| all.contains(g));
    let subject = match game {
        Some(g) if all.iter().any(|x| x == g) => Some(g.to_owned()),
        Some(g) => bail!(
            "no game {g} in {}",
            games_dir
                .as_deref()
                .map_or_else(|| "the games checkout".into(), |d| d.display().to_string())
        ),
        None => last.or_else(|| (all.len() == 1).then(|| all[0].clone())),
    };
    // A call a button on the page started (serve/jobs.rs) is the
    // person's; the others are the agent's.
    let runs: Vec<Value> = gp_mcp::activity::running(home)
        .into_iter()
        .filter(|r| game.is_none_or(|g| r["game"].as_str().is_none_or(|x| x == g)))
        .collect();
    let by_page = |r: &Value| r["by"] == json!("page");
    let mut f = Facts {
        games: all.clone(),
        asks: gp_mcp::asks::pending(home),
        agent_runs: runs
            .iter()
            .filter(|r| !by_page(r))
            .map(|r| {
                (
                    r["what"].as_str().unwrap_or("").to_owned(),
                    gp_mcp::asks::now_ms().saturating_sub(r["started_ms"].as_u64().unwrap_or(0))
                        / 1000,
                )
            })
            .collect(),
        page_runs: runs
            .iter()
            .filter(|r| by_page(r))
            .map(|r| r["what"].as_str().unwrap_or("").to_owned())
            .collect(),
        changes: gp_mcp::activity::changes(home, None),
        ..Facts::default()
    };
    let (Some(g), Some(gd)) = (subject, games_dir) else {
        if all.is_empty() {
            f.roms = rom_files(&paths.roms);
        }
        return Ok(f);
    };
    let gdir = gd.join(&g);
    f.game = Some(g.clone());
    f.changes = gp_mcp::activity::changes(home, Some(&g));
    f.page_changes = gp_mcp::activity::page_changes(home, Some(&g));
    let manifest: toml::Value = std::fs::read_to_string(gdir.join("template.toml"))
        .ok()
        .and_then(|t| toml::from_str(&t).ok())
        .unwrap_or(toml::Value::Table(Default::default()));
    f.title = manifest
        .get("template")
        .and_then(|t| t.get("title"))
        .and_then(|v| v.as_str())
        .unwrap_or(&g)
        .to_owned();
    let accepts: Vec<String> = manifest
        .get("source")
        .and_then(|s| s.get("accepts"))
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    f.rom_found = !accepts.is_empty()
        && crate::roms::find(&accepts, &[], &paths.roms).is_ok_and(|m| !m.is_empty());
    f.triage_plan = gdir.join("triage.toml").is_file();

    // The model and mapping the run plays: the working copy once there is one.
    let rdir = home.join("work/serve").join(&g);
    let mdir = if rdir.join("model").join(gp_model::MODEL_FILE).is_file() {
        rdir.join("model")
    } else {
        gdir.clone()
    };
    let model = crate::mechanics::load_model(&mdir).ok();
    f.player = model.as_ref().and_then(gp_interp::scenario::player);
    let mapping = std::fs::read_to_string(mdir.join(gp_interp::mapping::MAPPING_FILE))
        .ok()
        .and_then(|t| gp_interp::mapping::Mapping::parse(&t).ok());
    f.player_mapped = match (&f.player, &mapping) {
        (Some(p), Some(m)) => m.fields.iter().any(|x| {
            x.path
                .strip_prefix("entities.")
                .and_then(|r| r.strip_prefix(p.as_str()))
                .is_some_and(|r| r.starts_with(['[', '.']))
        }),
        _ => false,
    };

    // The quest log and its sessions.
    let mdirs = home.join("mechanics").join(&g);
    let queue = crate::mechanic::scores::scored_queue(home, &g);
    let out_of_scope: BTreeSet<String> = queue
        .iter()
        .filter(|r| r["scope"] == json!("out"))
        .filter_map(|r| r["mechanic"].as_str().map(str::to_owned))
        .collect();
    for r in &queue {
        let Some(m) = r["mechanic"].as_str() else {
            continue;
        };
        if out_of_scope.contains(m) {
            continue;
        }
        let t = &r["triage"];
        let req = &t["request"];
        let s = |v: &Value| v.as_str().unwrap_or("").to_owned();
        f.rows.push(Row {
            mechanic: m.to_owned(),
            done: r["status"] == json!("done"),
            triaged: t["by"].is_string(),
            you: (t["by"] == json!("you")).then(|| Request {
                start: if req["where"].is_string() {
                    s(&req["where"])
                } else {
                    s(&req["from"])
                },
                todo: s(&req["do"]),
                stop: s(&req["stop"]),
            }),
        });
        if m.starts_with("sealed") || r["status"] == json!("done") {
            continue;
        }
        match last_turn(&mdirs.join(m)) {
            Some((by, _)) if by == "player" => f.handoffs.push(m.to_owned()),
            Some((by, text)) if by == "agent" => f.asked.push((m.to_owned(), text)),
            _ => {}
        }
    }
    // Typed notes, oldest first, from the open mechanic's talk.
    if let Ok(cur) = std::fs::read_to_string(mdirs.join("current"))
        && let Ok(t) = std::fs::read_to_string(mdirs.join(cur.trim()).join("talk.ndjson"))
    {
        f.notes = t
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(|v| v["by"] == json!("player"))
            .filter_map(|v| v["text"].as_str().map(str::to_owned))
            .filter(|t| typed(t))
            .collect();
    }

    // Clips: the sessions' and the corpus's.
    let set = crate::clipset::read_game(&gdir).ok();
    let kept: BTreeSet<(String, u64)> = set
        .iter()
        .flat_map(|s| &s.clips)
        .filter_map(|c| {
            let p: Vec<&str> = c.id.split('.').collect();
            let n = p.last()?.parse().ok()?;
            (p.len() >= 3).then(|| (p[1..p.len() - 1].join("."), n))
        })
        .collect();
    f.corpus_clips = set.as_ref().map_or(0, |s| s.clips.len() as u64);
    let excluded: BTreeSet<String> = set
        .as_ref()
        .and_then(|s| s.import.as_ref())
        .and_then(|i| i.get("exclude"))
        .and_then(|e| e.as_array())
        .into_iter()
        .flatten()
        .filter_map(|e| e.get("what").and_then(|w| w.as_str()).map(str::to_owned))
        .collect();
    let mut sessions: Vec<PathBuf> = std::fs::read_dir(&mdirs)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("clips").is_dir())
        .collect();
    sessions.sort();
    for s in sessions {
        let name = s
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_owned();
        let mut nums: Vec<u64> = std::fs::read_dir(s.join("clips"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                (p.extension()? == "toml")
                    .then(|| p.file_stem()?.to_str()?.parse().ok())
                    .flatten()
            })
            .collect();
        nums.sort();
        f.session_clips += nums.len() as u64;
        if name.starts_with("sealed") || out_of_scope.contains(&name) || excluded.contains(&name) {
            continue;
        }
        for n in nums {
            if !kept.contains(&(name.clone(), n)) && !excluded.contains(&format!("{name}.{n:03}")) {
                f.not_kept.push(format!("{name} {n}"));
            }
        }
    }

    // Scores, questions, the background.
    f.scores = background_scores(paths, &gd, &g).or_else(|| gate_scores(home, &g, &gdir));
    let run = crate::serve::game::Run {
        name: g.clone(),
        game: gdir.clone(),
        dir: rdir.clone(),
    };
    f.tells = crate::serve::tell::open(&run)
        .into_iter()
        .filter_map(|t| Some((t.id?, t.clip?, t.text)))
        .collect();
    let qage = age_s(&run.copy_dir().join("questions.toml"));
    f.questions = crate::serve::background::open_questions(&run)
        .into_iter()
        .filter(|q| q["answer"].is_null())
        .map(|q| Question {
            clip: q["clip"]
                .as_str()
                .filter(|c| !c.is_empty())
                .or(q["path"].as_str())
                .unwrap_or("a clip")
                .to_owned(),
            choices: q["choices"].as_array().map_or(0, Vec::len),
            age_s: qage,
        })
        .collect();
    f.background = live.unwrap_or_else(|| {
        let l = &read_json(&rdir.join("background").join("state.json"))["live"];
        Background {
            on: l["on"] == json!(true) && l["pid"].as_u64().is_some_and(alive),
            ..Background::default()
        }
    });

    // The whole game, and online.
    if let Some(m) = crate::progress_measure::load(paths, &gdir, Some(&g))
        && let Some(w) = &m.whole
    {
        f.whole = if m.model != crate::progress_measure::model_key(&mdir) {
            Whole::Stale
        } else {
            match &w.first {
                None if w.compared > 0 => Whole::Same,
                None => Whole::NotMeasured,
                Some(s) => Whole::Parts {
                    together: s.at,
                    of: w.compared,
                    level: model.as_ref().map_or_else(
                        || s.level.clone(),
                        |md| gp_interp::scenario::level_words(md, &s.level),
                    ),
                },
            }
        };
    }
    f.published = read_json(&home.join("work/published").join(format!("{g}.json")))["link"]
        .as_str()
        .map(str::to_owned);
    Ok(f)
}

/// `gp progress --next`.
pub(crate) fn run(paths: &gp_evidence::Paths, game: Option<&str>) -> Result<Outcome> {
    let f = gather(paths, None, game, None)?;
    let a = answer(&f);
    Ok(Outcome::ok(json_of(&a), text_of(&a)))
}

#[cfg(test)]
mod tests;
