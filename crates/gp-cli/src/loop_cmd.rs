//! `gp loop`: the loop's steps as the engine emits them (crate::steps,
//! docs/loop.md). `gp loop steps` lists the open steps of every game and
//! copy in the home, each with its id, who does it, what it touches and
//! the commands it runs. It reads files only, as `next` does. `gp loop
//! set` changes the home's loop settings (`$GP_HOME/loop.toml`,
//! gp_mcp::steps): whether gp_run needs a step for a write, and gp serve's
//! runner. Only the person runs it: gp_run refuses it.

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
    },
}

fn settings_text(s: &gp_mcp::steps::Settings) -> String {
    let w = |b: bool| if b { "on" } else { "off" };
    format!(
        "steps required by gp_run: {}\nrunner: {}, at most {} workers{}",
        w(s.steps_required),
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
    json!({"open": crate::steps::open_of(&answers)})
}

fn steps_text(v: &Value) -> String {
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
            let changed = steps_required.is_some()
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
    }
}
