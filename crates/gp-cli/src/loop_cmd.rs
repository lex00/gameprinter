//! `gp loop`: the loop's steps as the engine emits them (crate::steps,
//! docs/loop.md). `gp loop steps` lists the open steps of every game and
//! copy in the home, each with its id, who does it, what it touches and
//! the commands it runs. It reads files only, as `next` does. `gp loop
//! set` changes the home's loop settings (`$GP_HOME/loop.toml`,
//! gp_mcp::steps): whether gp_run needs a step for a write, and gp serve's
//! runner. `gp loop wrong` marks an open step wrong, which halts the loop,
//! and `gp loop resume` clears the halt once a landed change names the
//! objection (crate::wrong). Only the person runs these three: gp_run
//! refuses them.

use anyhow::Result;
use clap::Subcommand;
use serde_json::{Value, json};

use crate::Outcome;

#[derive(Subcommand)]
pub enum LoopCmd {
    /// The open steps of every game and copy in the home (docs/loop.md):
    /// each with its id (a hash of its command and the facts it was
    /// decided from), who does it, what it says, what it touches and the gp
    /// commands it runs. No two open steps touch the same thing. gp_run's
    /// `step` cites one of these ids. Reads files only.
    Steps,
    /// Change the home's loop settings ($GP_HOME/loop.toml) and print them.
    /// With no flag, print them. Only the person runs this: gp_run refuses
    /// it, so an agent can't turn the loop's rules off.
    Set {
        /// on: gp_run runs a call that writes, lands or publishes only
        /// citing an open step that runs its command (off by default).
        #[arg(long, value_parser = ["on", "off"])]
        steps_required: Option<String>,
        /// on: gp serve's runner starts a worker on each open step of the
        /// agent's, up to --max-workers (off by default; each worker is an
        /// agent, which costs).
        #[arg(long, value_parser = ["on", "off"])]
        runner: Option<String>,
        /// The most workers the runner has at once (default 2).
        #[arg(long)]
        max_workers: Option<u32>,
        /// A note on spending the page shows beside the runner ("about 5
        /// dollars a day"); the runner keeps no count of money yet.
        #[arg(long)]
        spending: Option<String>,
        /// The games whose quest-log items the loop works on, by name,
        /// comma-separated ("tapper"); "all" for every game's.
        #[arg(long)]
        games: Option<String>,
    },
    /// Mark an open step wrong, with why: the loop halts (next emits no
    /// steps, gp_run runs none, the runner starts no worker) until a change
    /// to the tool that names the objection's id lands through gp tool gate.
    /// The page's WRONG does the same. Only the person runs this.
    Wrong {
        /// The step's id, from gp loop steps or next.
        step: String,
        /// What is wrong with it, in the person's words.
        #[arg(long)]
        why: String,
    },
    /// Clear objection ID once a commit on the tool's branch adds a line
    /// naming it (a test that pins the rule change). The tool gate does this
    /// itself for each patch it lands; this is for one that landed another
    /// way or before. Run in the gameprinter checkout. Only the person runs
    /// this.
    Resume {
        /// The objection's id (w and 12 hex digits), from gp loop steps.
        id: String,
        /// The branch to look on (default: main).
        #[arg(long, default_value = "main")]
        onto: String,
    },
}

fn settings_text(s: &gp_mcp::steps::Settings) -> String {
    let w = |b: bool| if b { "on" } else { "off" };
    format!(
        "steps required by gp_run: {}\nquest-log items of: {}\nrunner: {}, at most {} workers{}",
        w(s.steps_required),
        if s.games.is_empty() {
            "every game".to_owned()
        } else {
            s.games.join(", ")
        },
        w(s.runner.on),
        s.runner.max_workers,
        if s.runner.spending.is_empty() {
            String::new()
        } else {
            format!("; spending: {}", s.runner.spending)
        }
    )
}

/// The steps as JSON, for gp_run's check and the page.
pub(crate) fn steps_json(paths: &gp_evidence::Paths, games: Option<&std::path::Path>) -> Value {
    let answers = crate::steps::answers(paths, games);
    // The runner's workers, newest first, with the live ones' steps.
    let mut workers = gp_mcp::steps::workers(&paths.home);
    workers.reverse();
    match crate::wrong::halt(&paths.home) {
        Some(h) => json!({"open": [], "halted": crate::wrong::halted_json(&h), "workers": workers}),
        None => json!({
            "open": open_with_rows(paths, games, &answers),
            "halted": null,
            "workers": workers,
        }),
    }
}

/// The engine's open steps, then the quest-log items' steps that touch
/// nothing those do (crate::steps::row_steps).
fn open_with_rows(
    paths: &gp_evidence::Paths,
    games: Option<&std::path::Path>,
    answers: &[crate::next::Answer],
) -> Vec<Value> {
    let mut open = crate::steps::open_of(answers, &gp_mcp::steps::held(&paths.home));
    // A row set back to the agent (status next) is the agent's: the
    // engine's step asking the person to play it gives way to the row's.
    let rows = crate::steps::row_steps(paths, games);
    let row_touch: std::collections::BTreeSet<String> = rows
        .iter()
        .flat_map(|s| s["touches"].as_array().cloned().unwrap_or_default())
        .filter_map(|t| {
            t.as_str()
                .filter(|t| t.starts_with("row:"))
                .map(str::to_owned)
        })
        .collect();
    open.retain(|s| {
        s["who"] != json!("person")
            || !s["touches"].as_array().is_some_and(|t| {
                t.iter()
                    .filter_map(Value::as_str)
                    .any(|t| row_touch.contains(t))
            })
    });
    let taken: std::collections::BTreeSet<String> = open
        .iter()
        .flat_map(|s| s["touches"].as_array().cloned().unwrap_or_default())
        .filter_map(|t| t.as_str().map(str::to_owned))
        .collect();
    let free = |s: &Value| {
        s["touches"].as_array().is_some_and(|t| {
            t.iter()
                .filter_map(Value::as_str)
                .all(|t| !taken.contains(t))
        })
    };
    open.extend(
        crate::steps::row_steps(paths, games)
            .into_iter()
            .filter(free),
    );
    open
}

fn steps_text(v: &Value) -> String {
    if let Some(h) = v["halted"]["says"].as_str() {
        return h.to_owned();
    }
    let open = v["open"].as_array().cloned().unwrap_or_default();
    if open.is_empty() {
        return "No open steps.".into();
    }
    let mut out = vec![format!("{} open steps:", open.len())];
    for s in &open {
        let cmds = s["commands"]
            .as_array()
            .map(|c| {
                c.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        out.push(format!(
            "  {} ({}, {}): {}{}",
            s["id"].as_str().unwrap_or("?"),
            s["who"].as_str().unwrap_or("?"),
            s["subject"].as_str().unwrap_or("the home"),
            s["says"].as_str().unwrap_or(""),
            if cmds.is_empty() {
                String::new()
            } else {
                format!(" [runs {cmds}]")
            }
        ));
    }
    out.join("\n")
}

pub(crate) fn run(cmd: &LoopCmd, paths: &gp_evidence::Paths) -> Result<Outcome> {
    match cmd {
        LoopCmd::Steps => {
            let v = steps_json(paths, None);
            let text = steps_text(&v);
            Ok(Outcome::ok(v, text))
        }
        LoopCmd::Set {
            steps_required,
            runner,
            max_workers,
            spending,
            games,
        } => {
            let home = &paths.home;
            let mut s = gp_mcp::steps::settings(home);
            if let Some(v) = steps_required {
                s.steps_required = v == "on";
            }
            if let Some(v) = runner {
                s.runner.on = v == "on";
            }
            if let Some(n) = max_workers {
                s.runner.max_workers = *n;
            }
            if let Some(w) = spending {
                s.runner.spending = w.trim().to_owned();
            }
            if let Some(g) = games {
                s.games = if g.trim() == "all" {
                    Vec::new()
                } else {
                    g.split(',')
                        .map(|x| x.trim().to_owned())
                        .filter(|x| !x.is_empty())
                        .collect()
                };
            }
            let changed = games.is_some()
                || steps_required.is_some()
                || runner.is_some()
                || max_workers.is_some()
                || spending.is_some();
            if changed {
                gp_mcp::steps::save(home, &s).map_err(anyhow::Error::msg)?;
            }
            Ok(Outcome::ok(
                json!({"settings": s, "file": gp_mcp::steps::settings_file(home), "changed": changed}),
                settings_text(&s),
            ))
        }
        LoopCmd::Wrong { step, why } => {
            let v = crate::wrong::mark(paths, None, step, why)?;
            let text = crate::wrong::says(&v);
            Ok(Outcome::ok(v, text))
        }
        LoopCmd::Resume { id, onto } => {
            let cwd = std::env::current_dir()?;
            let repo = std::path::PathBuf::from(crate::gate::git(
                &cwd,
                &["rev-parse", "--show-toplevel"],
            )?);
            let v = crate::wrong::resume(&paths.home, &repo, onto, id)?;
            let text = format!(
                "The loop resumes: objection {id} is cleared by {}.",
                v["cleared"]["commit"].as_str().unwrap_or("?")
            );
            Ok(Outcome::ok(v, text))
        }
    }
}
