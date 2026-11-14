//! `gp host`: the local runtime host (Milestone 3; docs/host.md). `serve`
//! runs it; `scenarios`, `record` and `speed` are the same pieces run once,
//! without a server.
//!
//! Brought from game-printer's host.rs. gameprinter has no run recorder,
//! so `scenarios` and `speed` no longer record metrics (their JSON is as
//! before), and `--json` comes from main.rs through [`JSON_OUT`], since
//! `serve` prints its URL line as JSON with it.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Subcommand};
use serde_json::{Value, json};

use gp_evidence::Paths;
use gp_host::local::{HostConfig, LocalHost};
use gp_host::{Host, scenario, speed};

use crate::Outcome;

/// Whether gp was given `--json`, set by main.rs before the command runs.
pub(crate) static JSON_OUT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[derive(Args, Clone)]
pub struct Where {
    /// The repo whose templates/ and baselines/ are read (default: the git
    /// work tree of the current directory).
    #[arg(long, value_name = "DIR")]
    repo: Option<PathBuf>,
    /// Where working copies are, outside any git work tree (default:
    /// $GP_HOME/work/mcp, the same as `gp mcp`).
    #[arg(long, value_name = "DIR")]
    work: Option<PathBuf>,
    /// The original's ROM, when it isn't found in $GP_ROM_DIR by hash.
    #[arg(long)]
    rom: Option<PathBuf>,
    /// The lifted assets (default: $GP_HOME/work/assets/<model.source>).
    #[arg(long, value_name = "DIR")]
    assets: Option<PathBuf>,
}

#[derive(Subcommand)]
pub enum HostCmd {
    /// Run the host on `127.0.0.1` for sessions and scenarios, background
    /// builds and playable links. It prints its URL, then serves until
    /// stopped.
    Serve {
        #[command(flatten)]
        at: Where,
        /// The port; 0 picks a free one.
        #[arg(long, default_value_t = 7710)]
        port: u16,
        /// Builds at once. They share one cargo target directory, so more
        /// than one mostly waits.
        #[arg(long, default_value_t = 1)]
        build_workers: usize,
    },
    /// Run a working copy's (or a template's) scenarios in parallel.
    Scenarios {
        #[command(flatten)]
        at: Where,
        /// The working copy, or with `--template` a template id.
        name: String,
        #[arg(long)]
        template: bool,
        /// Only these scenarios.
        #[arg(long = "scenario", value_name = "NAME")]
        names: Vec<String>,
    },
    /// Record a run of a working copy as a scenario in the copy: the
    /// variant's own baseline.
    Record {
        #[command(flatten)]
        at: Where,
        copy: String,
        /// The scenario's name.
        scenario: String,
        #[arg(long)]
        frames: Option<usize>,
        /// Button holds such as `right+b@10-`.
        #[arg(long = "hold", value_name = "SPAN")]
        holds: Vec<String>,
        #[arg(long)]
        baseline: Option<String>,
        /// State paths to pin (default: every counters. and rules. value).
        #[arg(long = "path", value_name = "PATH")]
        paths: Vec<String>,
        #[arg(long)]
        description: Option<String>,
    },
    /// Search for button holds that reach a goal in a working copy's own
    /// model, starting from a baseline's holds: a winning run for a
    /// variant, to record with `host record`.
    Search {
        #[command(flatten)]
        at: Where,
        copy: String,
        /// Start from this baseline of the model's source.
        #[arg(long)]
        baseline: Option<String>,
        /// Or from these holds (with `--frames`).
        #[arg(long = "hold", value_name = "SPAN")]
        holds: Vec<String>,
        #[arg(long)]
        frames: Option<usize>,
        /// The goal as PATH=VALUE, such as `rules.game.state=won`.
        #[arg(long, value_name = "PATH=VALUE")]
        goal: String,
        /// A number to push as far as it goes, such as `entities.player[0].x`.
        #[arg(long, value_name = "PATH")]
        progress: String,
        /// How far a hold's start may move either way, in frames.
        #[arg(long)]
        window: Option<i64>,
        #[arg(long)]
        rounds: Option<usize>,
    },
    /// Hold speed measurements to their budgets: concept plan time, time to
    /// the first playable link, tweak time.
    Speed {
        /// Seconds from the task start to a concept plan.
        #[arg(long)]
        plan: Option<f64>,
        /// Seconds from the fork to the first playable link answering.
        #[arg(long)]
        first_link: Option<f64>,
        /// Seconds from a model edit to its effect read back.
        #[arg(long)]
        tweak: Option<f64>,
        /// Seconds from a model edit to the rebuilt link answering.
        #[arg(long)]
        tweak_link: Option<f64>,
        /// Seconds from a ROM handed to `gp capture start` to the run's
        /// first interpreter preview (its `speed.rom_to_preview_s` event).
        #[arg(long)]
        rom_to_preview: Option<f64>,
    },
}

fn absolute(p: &Path) -> Result<PathBuf> {
    std::path::absolute(p).with_context(|| p.display().to_string())
}

fn config(at: &Where, paths: &Paths) -> Result<gp_mcp::Config> {
    let cwd = std::env::current_dir()?;
    let repo = match &at.repo {
        Some(r) => absolute(r)?,
        None => gp_evidence::paths::enclosing_git_work_tree(&cwd)
            .ok_or_else(|| anyhow!("not in a git work tree; give the repo with --repo"))?,
    };
    if !repo.join("templates").is_dir() {
        bail!("{} has no templates/", repo.display());
    }
    let work = match &at.work {
        Some(w) => absolute(w)?,
        None => paths.home.join("work").join("mcp"),
    };
    let existing = work
        .ancestors()
        .find(|p| p.exists())
        .unwrap_or(Path::new("/"));
    if let Some(tree) = gp_evidence::paths::enclosing_git_work_tree(existing) {
        bail!(
            "{} is inside the git work tree {}; working copies go outside it",
            work.display(),
            tree.display()
        );
    }
    std::fs::create_dir_all(&work).with_context(|| format!("creating {}", work.display()))?;
    Ok(gp_mcp::Config {
        gp: std::env::current_exe().context("finding the gp binary")?,
        repo,
        work,
        home: paths.home.clone(),
        rom: at.rom.as_deref().map(absolute).transpose()?,
        assets: at.assets.as_deref().map(absolute).transpose()?,
        games: crate::roms_cmd::games_dir(None),
    })
}

pub(crate) fn run(cmd: &HostCmd, paths: &Paths) -> Result<Outcome> {
    let json_out = JSON_OUT.load(std::sync::atomic::Ordering::Relaxed);
    match cmd {
        HostCmd::Serve {
            at,
            port,
            build_workers,
        } => serve(at, *port, *build_workers, json_out, paths),
        HostCmd::Scenarios {
            at,
            name,
            template,
            names,
        } => {
            let cfg = config(at, paths)?;
            let host = LocalHost::start(HostConfig::new(cfg)).map_err(anyhow::Error::msg)?;
            let r = host
                .run_scenarios(&scenario::Request {
                    copy: name.clone(),
                    template: *template,
                    names: names.clone(),
                    // gp --jobs N or GP_JOBS; else the cores, at most 8.
                    jobs: crate::pool::explicit(),
                })
                .map_err(anyhow::Error::msg)?;
            let mut text = String::new();
            for x in r["results"].as_array().into_iter().flatten() {
                let _ = writeln!(
                    text,
                    "{} {}",
                    if x["ok"] == true { "pass" } else { "FAIL" },
                    x["name"].as_str().unwrap_or("")
                );
                if let Some(e) = x["error"].as_str() {
                    let _ = writeln!(text, "    {e}");
                }
                for c in x["checks"].as_array().into_iter().flatten() {
                    if c["ok"] != true {
                        let _ = writeln!(
                            text,
                            "    {} (got {})",
                            c["expect"].as_str().unwrap_or(""),
                            c["got"]
                        );
                    }
                }
            }
            let _ = write!(
                text,
                "{} scenarios, {} passed, {} failed; {} threads, {} ms (serial {} ms); object {}",
                r["scenarios"],
                r["passed"],
                r["failed"],
                r["jobs"],
                r["millis"]["wall"],
                r["millis"]["serial"],
                r["store_object"].as_str().unwrap_or("")
            );
            Ok(Outcome {
                exit: u8::from(r["ok"] != true),
                json: r,
                text,
            })
        }
        HostCmd::Record {
            at,
            copy,
            scenario: name,
            frames,
            holds,
            baseline,
            paths: state_paths,
            description,
        } => {
            let cfg = config(at, paths)?;
            let host = LocalHost::start(HostConfig::new(cfg)).map_err(anyhow::Error::msg)?;
            let r = host
                .record_scenario(&scenario::Record {
                    copy: copy.clone(),
                    name: name.clone(),
                    description: description.clone(),
                    frames: *frames,
                    holds: holds.clone(),
                    baseline: baseline.clone(),
                    paths: state_paths.clone(),
                    overwrite: false,
                })
                .map_err(anyhow::Error::msg)?;
            let text = format!(
                "recorded `{name}` ({} frames, state hash {}) in {}",
                r["frames"],
                r["state_hash"].as_str().unwrap_or(""),
                r["written"].as_str().unwrap_or("")
            );
            Ok(Outcome::ok(r, text))
        }
        HostCmd::Search {
            at,
            copy,
            baseline,
            holds,
            frames,
            goal,
            progress,
            window,
            rounds,
        } => {
            let (goal_path, goal_value) = goal
                .split_once('=')
                .ok_or_else(|| anyhow!("--goal is PATH=VALUE, such as rules.game.state=won"))?;
            let cfg = config(at, paths)?;
            let host = LocalHost::start(HostConfig::new(cfg)).map_err(anyhow::Error::msg)?;
            let r = host
                .search_inputs(&gp_host::search::Search {
                    copy: copy.clone(),
                    baseline: baseline.clone(),
                    holds: holds.clone(),
                    frames: *frames,
                    goal_path: goal_path.to_owned(),
                    goal_value: goal_value.to_owned(),
                    progress_path: progress.clone(),
                    window: *window,
                    rounds: *rounds,
                })
                .map_err(anyhow::Error::msg)?;
            let hold_args: Vec<String> = r["holds"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|h| h.as_str().map(|h| format!("--hold {h}")))
                .collect();
            let text = format!(
                "{} after {} tries; progress {}; {}\nrecord it: gp host record {copy} NAME --frames {} {}",
                if r["goal_met"] == true {
                    "goal met"
                } else {
                    "goal NOT met"
                },
                r["tries"],
                r["progress"],
                r["note"].as_str().unwrap_or(""),
                r["frames"],
                hold_args.join(" ")
            );
            Ok(Outcome {
                exit: u8::from(r["goal_met"] != true),
                json: r,
                text,
            })
        }
        HostCmd::Speed {
            plan,
            first_link,
            tweak,
            tweak_link,
            rom_to_preview,
        } => {
            let mut verdicts = Vec::new();
            for (name, v) in [
                ("plan_s", plan),
                ("first_link_s", first_link),
                ("tweak_s", tweak),
                ("tweak_link_s", tweak_link),
                ("rom_to_preview_s", rom_to_preview),
            ] {
                if let Some(s) = v {
                    let verdict = speed::judge(name, *s).map_err(anyhow::Error::msg)?;
                    verdicts.push(verdict);
                }
            }
            let mut text = String::new();
            for t in speed::TARGETS {
                match verdicts.iter().find(|v| v.name == t.name) {
                    Some(v) => {
                        let _ = writeln!(
                            text,
                            "{:<13} {:>7.2} s  budget {:>5.0} s  target {:>4.0} s  {}",
                            t.name,
                            v.seconds,
                            t.budget_s,
                            t.target_s,
                            if !v.within_budget {
                                "OVER BUDGET"
                            } else if v.within_target {
                                "on target"
                            } else {
                                "within budget"
                            }
                        );
                    }
                    None => {
                        let _ = writeln!(
                            text,
                            "{:<13} budget {:>5.0} s  target {:>4.0} s  ({})",
                            t.name, t.budget_s, t.target_s, t.what
                        );
                    }
                }
            }
            let over = verdicts.iter().any(|v| !v.within_budget);
            Ok(Outcome {
                exit: u8::from(over),
                json: json!({ "ok": !over, "measurements": verdicts, "targets": speed::TARGETS }),
                text: text.trim_end().to_owned(),
            })
        }
    }
}

fn serve(
    at: &Where,
    port: u16,
    build_workers: usize,
    json_out: bool,
    paths: &Paths,
) -> Result<Outcome> {
    let cfg = config(at, paths)?;
    let mut hc = HostConfig::new(cfg);
    hc.build_workers = build_workers;
    let shown = json!({ "repo": hc.mcp.repo, "work": hc.mcp.work, "dir": hc.dir });
    let host = LocalHost::start(hc).map_err(anyhow::Error::msg)?;
    gp_host::http::serve(host, port, |addr| {
        // The first line is the URL, for a script that started the host.
        if json_out {
            println!(
                "{}",
                json!({ "url": format!("http://{addr}"), "listening": true })
            );
        } else {
            println!("gp host listening on http://{addr}/");
        }
        let _ = std::io::Write::flush(&mut std::io::stdout());
    })
    .map_err(anyhow::Error::msg)?;
    Ok(Outcome::ok(shown as Value, String::new()))
}
