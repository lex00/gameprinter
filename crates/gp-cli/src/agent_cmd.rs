//! `gp agent` (game-printer's agent_work.rs; docs/agents.md, "Working on a
//! clip in the loop"): writes and reads a run's work record, the notes and
//! attempts the REFINE tab shows while Claude works on a clip. The driver
//! posts its lanes' work with `--by driver`; the clip agent `gp serve`
//! starts posts with `--by "clip agent"`.
//!
//! A run is a game of the games checkout (`--games`, else $GP_GAMES_DIR),
//! named by its folder, as in `gp serve`. Every command of game-printer's
//! `gp agent` is here: `note`, `tell`, `tells`, `attempt`, `integrate`,
//! `done`, `approve`, `state`, `bench`, `brief`, `clips` and `score`
//! (agent_brief.rs, agent_bench.rs, agent_integrate.rs, serve/tell.rs).

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

  1. gp agent brief --run R --clip C --json
       The clip, its first wrong frame and the values around it, the
       mechanics that own them, the code the evidence cites, what the
       original's code read to write the measure (gp element inputs), the
       person's tells, the attempts and the next commands, whole. Run it
       before reading anything else.
  2. Change the code in your worktree W and the model in your copy M of
       the run's model, one change at a time.
  3. gp agent attempt --run R --clip C --by \"clip agent\" --worktree W --model-from M --commit \\
       --title \"the barrel rolls off the edge\" --says \"The game drops it a frame later.\"
       Commits, builds the worktree's gp, scores every active clip and
       prints the score before and after. It can take minutes.
  4. gp agent note --run R --clip C --by \"clip agent\" --on \"the barrel\" \"reading the roll code\"
       Post it as you go: the person reads it on the clip.
  5. gp agent done --run R --clip C --by \"clip agent\" \"the clip matches\"

Also: `gp agent clips --run R` lists the run's clips and scores;
`gp agent state --run R` lists the attempts and who is working on what;
`gp agent tells --run R --open` lists what the person told you.";

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
        /// The tell this note answers, by number (`gp agent tells`).
        #[arg(long)]
        reply_to: Option<u64>,
        text: String,
    },
    /// Tell the agent something about a clip, as the person does from the
    /// clip. It goes to the inbox of the clip agent working on the clip, or
    /// waits for the next one.
    Tell {
        #[arg(long)]
        run: String,
        #[arg(long)]
        clip: String,
        /// What is wrong, in plain words.
        #[arg(long)]
        says: String,
        /// The clip's frame it is about.
        #[arg(long)]
        frame: Option<u64>,
        /// The original's frame.
        #[arg(long)]
        orig_frame: Option<u64>,
        /// A spot on the screen, in game pixels: X,Y.
        #[arg(long)]
        at: Option<String>,
        /// A region on the screen: X,Y,W,H.
        #[arg(long)]
        region: Option<String>,
    },
    /// The tells and their replies; `--open` for those without a reply.
    Tells {
        #[arg(long)]
        run: String,
        #[arg(long)]
        open: bool,
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
    /// Run by the coordinator, it picks the approved attempts not on main,
    /// in order, onto a new branch of this checkout, numbers the versions
    /// once, regenerates the schema, writes the attempts' CHANGE.md notes
    /// into the ROADMAP, and runs `gp model check` on the games. Stops at
    /// the first conflict. It doesn't merge into main or score: the tool
    /// gate lands the branch (docs/agents.md, "Merging attempts").
    Integrate {
        /// The new branch, made from main in the checkout, which must be
        /// clean.
        #[arg(long)]
        branch: String,
        /// The repo checkout to integrate in.
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        /// Use this gp for the schema and the model check instead of
        /// building the checkout's.
        #[arg(long)]
        gp: Option<PathBuf>,
        /// Attempts to leave out, by number.
        #[arg(long)]
        skip: Vec<u64>,
        /// The date for the ROADMAP line (default today).
        #[arg(long)]
        date: Option<String>,
        /// Only list what would be picked.
        #[arg(long)]
        dry_run: bool,
        /// At the end, run `just check` on the branch and exit 1 on a
        /// failure.
        #[arg(long)]
        check: bool,
    },
    /// The loop's efficiency in one table (docs/agents.md, "Loop
    /// efficiency") covers rounds and fixes per hour by source, and seconds
    /// per round with LLM calls per fix. It also gives exact and fidelity
    /// over a window, and is read-only.
    Bench {
        #[arg(long)]
        run: String,
        /// Where the window starts: a round number, a UTC time such as
        /// "2026-10-30 08:00", or an age before the last round (2h).
        #[arg(long)]
        since: Option<String>,
        /// Also append the result as one dated row to the notes log of
        /// docs/research/deterministic-inference.md.
        #[arg(long)]
        log: bool,
        /// The note to append to (default: that file, in the current repo).
        #[arg(long)]
        note: Option<PathBuf>,
    },
    /// Everything an agent needs to start on a clip, in one call: the clip,
    /// its first wrong frame and each wrong measure's values around it, and
    /// the mechanics that own those measures with their parameters. It also
    /// gives the code the evidence cites (file, line and routine), what the
    /// original's code read to write the first wrong measure (the Z80's on
    /// an arcade game), the open questions and the person's tells, with the
    /// attempts so far and the next commands written out in full. It is
    /// read-only and, with `--json`, has a fixed shape (`schema`).
    Brief {
        #[arg(long)]
        run: String,
        #[arg(long)]
        clip: String,
        /// Skip playing the clip for the values around its first wrong
        /// frame and the writer slice.
        #[arg(long)]
        no_curve: bool,
    },
    /// The run's active clips with their scores, those that don't match
    /// first. Read-only; the brief lists the first 20.
    Clips {
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

/// `X,Y` (n = 2) or `X,Y,W,H` (n = 4).
fn numbers<const N: usize>(s: &Option<String>, what: &str) -> anyhow::Result<Option<[u32; N]>> {
    let Some(s) = s else { return Ok(None) };
    let v: Vec<u32> = s
        .split(',')
        .map(|x| x.trim().parse::<u32>())
        .collect::<Result<_, _>>()
        .map_err(|_| anyhow::anyhow!("--{what} is {N} numbers split by commas, not {s:?}"))?;
    let a: [u32; N] = v
        .try_into()
        .map_err(|_| anyhow::anyhow!("--{what} is {N} numbers split by commas, not {s:?}"))?;
    Ok(Some(a))
}

pub fn run(args: &AgentArgs, paths: &Paths) -> anyhow::Result<Outcome> {
    // The merge helper reads no run.
    if let AgentCmd::Integrate {
        branch,
        repo,
        gp,
        skip,
        date,
        dry_run,
        check,
    } = &args.cmd
    {
        return crate::agent_integrate::run(
            &crate::agent_integrate::Args {
                repo,
                branch,
                gp: gp.as_deref(),
                skip,
                date: date.as_deref(),
                dry_run: *dry_run,
                check: *check,
                games: args.games.as_deref(),
            },
            paths,
        );
    }
    let ctx = Ctx::for_cli(paths, args.games.as_deref())?;
    let run_of = |r: &str| run_named(&ctx, r).map_err(|f| anyhow::anyhow!(f.message));
    match &args.cmd {
        AgentCmd::Note {
            run,
            clip,
            by,
            on,
            reply_to,
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
                    reply_to: *reply_to,
                    ..Entry::default()
                },
            )?;
            Ok(Outcome::ok(json!(e), format!("Posted: {text}")))
        }
        AgentCmd::Tell {
            run,
            clip,
            says,
            frame,
            orig_frame,
            at,
            region,
        } => {
            let r = run_of(run)?;
            let (e, inbox) = crate::serve::tell::add(
                &r,
                crate::serve::tell::New {
                    clip: clip.clone(),
                    text: says.clone(),
                    frame: *frame,
                    orig_frame: *orig_frame,
                    at: numbers::<2>(at, "at")?,
                    region: numbers::<4>(region, "region")?,
                },
            )?;
            let how = if inbox.is_some() {
                "It is in the agent's inbox."
            } else {
                "No agent is on the clip: it is queued for the next one."
            };
            Ok(Outcome::ok(
                json!({"tell": e, "delivered": inbox.is_some()}),
                format!("Tell {}: {} {how}", e.id.unwrap_or(0), e.text),
            ))
        }
        AgentCmd::Tells { run, open } => {
            let r = run_of(run)?;
            let v = crate::serve::tell::open_json(&r, *open);
            let mut lines: Vec<String> = Vec::new();
            for t in v["tells"].as_array().into_iter().flatten() {
                lines.push(format!(
                    "{} (tell {}{})",
                    t["line"].as_str().unwrap_or(""),
                    t["id"],
                    if t["open"] == true { ", open" } else { "" }
                ));
                for rp in t["replies"].as_array().into_iter().flatten() {
                    let by = match rp["by"].as_str().unwrap_or("") {
                        "driver" => "the driver",
                        "clip agent" => "the clip agent",
                        "user" => "you",
                        other => other,
                    };
                    lines.push(format!(
                        "  reply, {by}: {}",
                        rp["text"].as_str().unwrap_or("")
                    ));
                }
            }
            if lines.is_empty() {
                lines.push(if *open {
                    "No open tells.".into()
                } else {
                    "No tells.".into()
                });
            }
            Ok(Outcome::ok(v, lines.join("\n")))
        }
        AgentCmd::Integrate { .. } => unreachable!("handled above"),
        AgentCmd::Bench {
            run,
            since,
            log,
            note,
        } => {
            let r = run_of(run)?;
            crate::agent_bench::run(&paths.home, &r.dir, since.as_deref(), *log, note.clone())
        }
        AgentCmd::Brief {
            run,
            clip,
            no_curve,
        } => crate::agent_brief::run(
            &ctx,
            paths,
            &crate::agent_brief::BriefArgs {
                run,
                clip,
                curve: !no_curve,
            },
        ),
        AgentCmd::Clips { run } => crate::agent_brief::clips(&ctx, run),
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
