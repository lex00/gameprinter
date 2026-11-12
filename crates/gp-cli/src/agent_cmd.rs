//! `gp agent` (game-printer's agent_work.rs; docs/agents.md, "Working on a
//! clip in the loop"): writes and reads a run's work record, the notes and
//! attempts the REFINE tab shows while Claude works on a clip. The driver
//! posts its lanes' work with `--by driver`; the clip agent `gp serve`
//! starts posts with `--by "clip agent"`.
//!
//! A run is a game of the games checkout (`--games`, else $GP_GAMES_DIR),
//! named by its folder, as in `gp serve`. Ported: `note`, `attempt`,
//! `done`, `approve`, `state` and `score`. Left out: `brief` (`gp clip
//! score` and `gp element inputs` give its core), `tell` and `tells` (the
//! mechanic page's talk), `bench`, `clips` and `integrate` (the
//! coordinator's merge helper; gameprinter's tool gate lands branches).

use std::path::PathBuf;

use anyhow::Context as _;
use clap::{Args, Subcommand};
use gp_evidence::Paths;
use serde_json::json;

use crate::Outcome;
use crate::serve::Ctx;
use crate::serve::game::run_named;
use crate::serve::work::{self, Entry};

const WORKFLOW: &str = "\
Working on a clip, in order (replace R with the run, a game's folder name, and C with the clip):

  1. gp clip score and gp element inputs on the clip: its first wrong frame
       and the code that wrote the measure there.
  2. Change the code in your worktree W and the model in your copy M of
       the run's model, one change at a time.
  3. gp agent attempt --run R --clip C --by \"clip agent\" --worktree W --model-from M --commit \\
       --title \"the barrel rolls off the edge\" --says \"The game drops it a frame later.\"
       Commits, builds the worktree's gp, scores every active clip and
       prints the score before and after. It can take minutes.
  4. gp agent note --run R --clip C --by \"clip agent\" --on \"the barrel\" \"reading the roll code\"
       Post it as you go: the person reads it on the clip.
  5. gp agent done --run R --clip C --by \"clip agent\" \"the clip matches\"

Also: `gp agent state --run R` lists the attempts and who is working on what.";

#[derive(Args)]
#[command(after_help = WORKFLOW)]
pub struct AgentArgs {
    /// The games checkout whose games are the runs (default: $GP_GAMES_DIR,
    /// else a folder of games at or above the working directory).
    #[arg(long, global = true)]
    pub games: Option<PathBuf>,
    #[command(subcommand)]
    pub cmd: AgentCmd,
}

#[derive(Subcommand)]
pub enum AgentCmd {
    /// Post a progress note, shown on the clip's card and the REFINE tab.
    /// `--on` says in plain words what the work is on until another `--on`
    /// or done.
    Note {
        #[arg(long)]
        run: String,
        #[arg(long)]
        clip: Option<String>,
        /// Who writes it: `driver` or `clip agent`.
        #[arg(long, default_value = "driver")]
        by: String,
        #[arg(long)]
        on: Option<String>,
        text: String,
    },
    /// Record an attempt at a clip: with `--worktree`, builds that
    /// worktree's gp and trial preview, scores the clip and every active
    /// clip with them, and records the score before and after, the clips
    /// better and worse, the preview and the commit.
    Attempt {
        #[arg(long)]
        run: String,
        #[arg(long)]
        clip: String,
        #[arg(long, default_value = "driver")]
        by: String,
        #[arg(long)]
        title: String,
        #[arg(long, default_value = "")]
        says: String,
        /// The git worktree holding the code change.
        #[arg(long)]
        worktree: Option<PathBuf>,
        /// Score this model folder as it is.
        #[arg(long)]
        model: Option<PathBuf>,
        /// A changed copy of the run's model: its changes are merged into
        /// the run's working copy file by file (against DIR.base, the copy
        /// it started from, when there is one).
        #[arg(long, value_name = "DIR")]
        model_from: Option<PathBuf>,
        /// Score with this gp instead of building the worktree's.
        #[arg(long)]
        gp: Option<PathBuf>,
        /// The worktree's cargo target folder (default: its target/).
        #[arg(long)]
        target: Option<PathBuf>,
        #[arg(long)]
        no_preview: bool,
        /// It changes how a mechanic works: keep says it has the person's OK.
        #[arg(long)]
        shape: bool,
        /// Commit the worktree's changes before scoring.
        #[arg(long)]
        commit: bool,
    },
    /// Say the work on a clip stopped, and why.
    Done {
        #[arg(long)]
        run: String,
        #[arg(long)]
        clip: Option<String>,
        #[arg(long, default_value = "driver")]
        by: String,
        #[arg(default_value = "")]
        text: String,
    },
    /// Keep an attempt (what the page's keep does): marks it approved and
    /// writes it to $GP_HOME/work/agents/approved.json for the driver.
    Approve {
        #[arg(long)]
        run: String,
        #[arg(long)]
        attempt: u64,
    },
    /// Who is working on what, and the attempts, in words.
    State {
        #[arg(long)]
        run: String,
    },
    /// Score every active clip of a run on a model folder with this build,
    /// as JSON: what an attempt reads from a worktree's gp.
    Score {
        #[arg(long)]
        run: String,
        #[arg(long)]
        model: PathBuf,
        /// The clip whose first wrong measure to give.
        #[arg(long)]
        clip: Option<String>,
        /// Where each clip's score files go (default: a folder under the
        /// run's agent/).
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

pub fn run(args: &AgentArgs, paths: &Paths) -> anyhow::Result<Outcome> {
    let ctx = Ctx::for_cli(paths, args.games.as_deref())?;
    let run_of = |r: &str| run_named(&ctx, r).map_err(|f| anyhow::anyhow!(f.message));
    match &args.cmd {
        AgentCmd::Note {
            run,
            clip,
            by,
            on,
            text,
        } => {
            let r = run_of(run)?;
            let e = work::append(
                &r,
                Entry {
                    kind: "note".into(),
                    by: work::parse_by(by)?.into(),
                    clip: clip.clone(),
                    text: text.clone(),
                    on: on.clone(),
                    ..Entry::default()
                },
            )?;
            Ok(Outcome::ok(json!(e), format!("Posted: {text}")))
        }
        AgentCmd::Done {
            run,
            clip,
            by,
            text,
        } => {
            let r = run_of(run)?;
            let e = work::append(
                &r,
                Entry {
                    kind: "done".into(),
                    by: work::parse_by(by)?.into(),
                    clip: clip.clone(),
                    text: text.clone(),
                    ..Entry::default()
                },
            )?;
            Ok(Outcome::ok(json!(e), "Done.".into()))
        }
        AgentCmd::Attempt {
            run,
            clip,
            by,
            title,
            says,
            worktree,
            model,
            model_from,
            gp,
            target,
            no_preview,
            shape,
            commit,
        } => {
            let a = work::run_attempt(
                &ctx,
                &work::AttemptArgs {
                    run,
                    clip,
                    by: work::parse_by(by)?,
                    title,
                    says,
                    worktree: worktree.as_deref(),
                    model: model.as_deref(),
                    model_from: model_from.as_deref(),
                    gp: gp.as_deref(),
                    target: target.as_deref(),
                    preview: !no_preview,
                    shape: *shape,
                    commit: *commit,
                },
            )?;
            let line = work::attempt_line(&a);
            let mut v = json!(a);
            v["line"] = json!(line);
            v["keepable"] = json!(a.keepable());
            Ok(Outcome::ok(v, line))
        }
        AgentCmd::Approve { run, attempt } => {
            let r = run_of(run)?;
            let a = work::approve_attempt_as(&paths.home, &r, *attempt, None)?;
            Ok(Outcome::ok(
                json!(a),
                format!(
                    "Kept attempt {}: written to {} for the driver to merge.",
                    a.n,
                    work::approved_file(&paths.home).display()
                ),
            ))
        }
        AgentCmd::State { run } => {
            let r = run_of(run)?;
            let v = work::state_json(&r);
            let mut text = Vec::new();
            for w in v["working"].as_array().into_iter().flatten() {
                text.push(w["says"].as_str().unwrap_or("").to_owned());
            }
            if text.is_empty() {
                text.push("No one is working on a clip.".into());
            }
            for a in v["attempts"].as_array().into_iter().flatten() {
                text.push(a["line"].as_str().unwrap_or("").to_owned());
            }
            Ok(Outcome::ok(v, text.join("\n")))
        }
        AgentCmd::Score {
            run,
            model,
            clip,
            out,
        } => {
            let r = run_of(run)?;
            let model = model
                .canonicalize()
                .with_context(|| format!("no model folder {}", model.display()))?;
            let out = out.clone().unwrap_or_else(|| {
                work::folder(&r)
                    .join("scores")
                    .join(work::now_ms().to_string())
            });
            let s = work::score_clips(
                &ctx,
                &r,
                Some(&model),
                &out,
                clip.as_deref(),
                &serde_json::Value::Null,
            )?;
            let exact = s
                .scores
                .values()
                .filter(|v| v["exact"] == json!(true))
                .count();
            let text = format!(
                "{} clips scored on {} (model {}): {exact} exact.",
                s.scores.len(),
                model.display(),
                s.model
            );
            Ok(Outcome::ok(work::scored_json(&s), text))
        }
    }
}
