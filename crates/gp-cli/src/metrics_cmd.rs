//! `gp metrics time/record/check/report` and `gp runs list`, as game-printer
//! had them in main.rs. They read and write the run recorder's database
//! (crate::runrec, `$GP_HOME/metrics.sqlite`).

use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use clap::Subcommand;
use gp_evidence::Paths;
use gp_metrics::{BudgetStatus, Budgets, MetricsDb};
use serde_json::json;

use crate::{Outcome, runrec, scorecard};

#[derive(Subcommand)]
pub enum MetricsCmd {
    /// Run a command and record `<name>.duration` (ms) and `<name>.exit`.
    Time {
        /// Metric name prefix, e.g. `check.test.native`.
        #[arg(long)]
        name: String,
        /// Another program the command depends on, as NAME=VERSION (repeatable).
        /// Goes in the run manifest's `tools`.
        #[arg(long = "tool", value_name = "NAME=VERSION")]
        tools: Vec<String>,
        /// The command to run, after `--`.
        #[arg(trailing_var_arg = true, required = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// Record one metric value.
    Record {
        /// Dotted metric name, e.g. `gp.binary.release.bytes`.
        #[arg(long)]
        name: String,
        /// The value.
        #[arg(long)]
        value: f64,
        /// Its unit: `ms`, `bytes`, `count` or `code`.
        #[arg(long)]
        unit: String,
    },
    /// Check metrics against a budgets file; fails if any is over, or missing
    /// from the run group.
    Check {
        /// The budgets file.
        #[arg(long, default_value = "budgets.toml")]
        budgets: PathBuf,
        /// Only count values from this run group; defaults to $GP_RUN_GROUP.
        /// Without a group, the latest value ever recorded is checked.
        #[arg(long)]
        group: Option<String>,
    },
    /// Write the HTML trend report.
    Report {
        /// The budgets file, for showing limits. Optional.
        #[arg(long, default_value = "budgets.toml")]
        budgets: PathBuf,
        /// Defaults to $GP_HOME/reports/metrics.html.
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub enum RunsCmd {
    /// The most recent runs.
    List {
        /// How many runs to show.
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
}

pub fn runs(cmd: &RunsCmd, paths: &Paths) -> Result<Outcome> {
    let RunsCmd::List { limit } = cmd;
    let runs = MetricsDb::open(&paths.metrics_db())?.recent_runs(*limit)?;
    let text = runs
        .iter()
        .map(|r| {
            format!(
                "{}  {:<16} {:>9.1} ms  {}  {}",
                &r.started_at[..19.min(r.started_at.len())],
                r.command,
                r.duration_ms,
                if r.ok { "ok    " } else { "FAILED" },
                r.manifest_path
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Outcome::ok(json!({ "runs": runs }), text))
}

pub fn metrics(cmd: &MetricsCmd, paths: &Paths) -> Result<Outcome> {
    match cmd {
        MetricsCmd::Time {
            name,
            tools,
            command,
        } => {
            for tool in tools {
                let (tool, version) = tool
                    .split_once('=')
                    .filter(|(n, v)| !n.is_empty() && !v.is_empty())
                    .with_context(|| format!("--tool takes NAME=VERSION, got {tool:?}"))?;
                runrec::tool(tool, version);
            }
            let (program, rest) = command.split_first().expect("clap requires a command");
            let start = Instant::now();
            let status = {
                let _s = tracing::info_span!("metrics.time.child").entered();
                std::process::Command::new(program)
                    .args(rest)
                    .status()
                    .with_context(|| format!("running {program}"))?
            };
            let ms = start.elapsed().as_secs_f64() * 1000.0;
            let code = status.code().unwrap_or(1);
            runrec::metric(format!("{name}.duration"), ms, "ms");
            runrec::metric(format!("{name}.exit"), f64::from(code), "code");
            Ok(Outcome {
                json: json!({ "name": name, "duration_ms": ms, "exit": code }),
                text: String::new(),
                exit: u8::try_from(code.clamp(0, 255)).unwrap_or(1),
            })
        }
        MetricsCmd::Record { name, value, unit } => {
            runrec::metric(name.clone(), *value, unit.clone());
            Ok(Outcome::ok(
                json!({ "name": name, "value": value, "unit": unit }),
                String::new(),
            ))
        }
        MetricsCmd::Check { budgets, group } => {
            let b = Budgets::load(budgets)?;
            let group = group
                .clone()
                .or_else(|| std::env::var("GP_RUN_GROUP").ok().filter(|g| !g.is_empty()));
            let results = b.evaluate(&MetricsDb::open(&paths.metrics_db())?, group.as_deref())?;
            let failed = results
                .iter()
                .filter(|r| matches!(r.status, BudgetStatus::Fail | BudgetStatus::Missing))
                .count();
            let mut text = String::new();
            for r in &results {
                let limit = match (r.min, r.max) {
                    (Some(lo), Some(hi)) => format!("{lo}..{hi}"),
                    (None, Some(hi)) => format!("<= {hi}"),
                    (Some(lo), None) => format!(">= {lo}"),
                    (None, None) => "-".into(),
                };
                let value = match r.status {
                    BudgetStatus::Missing => "not recorded this run".into(),
                    BudgetStatus::Skipped => r.reason.clone().unwrap_or_default(),
                    _ => r
                        .value
                        .map(|v| format!("{v:.1}"))
                        .unwrap_or_else(|| "no data".into()),
                };
                let mark = match r.status {
                    BudgetStatus::Pass => "ok  ",
                    BudgetStatus::Fail => "OVER",
                    BudgetStatus::NoData => "--  ",
                    BudgetStatus::Missing => "MISS",
                    BudgetStatus::Skipped => "skip",
                };
                text.push_str(&format!(
                    "{mark} {:<32} {:>10} {:<5} budget {limit}\n",
                    r.name,
                    value,
                    r.unit.as_deref().unwrap_or("")
                ));
            }
            let skipped = results
                .iter()
                .filter(|r| r.status == BudgetStatus::Skipped)
                .count();
            text.push_str(&format!(
                "{} budgets checked, {} over or missing, {} skipped{}",
                results.len() - skipped,
                failed,
                skipped,
                group
                    .as_deref()
                    .map(|g| format!(" (run group {g})"))
                    .unwrap_or_default()
            ));
            runrec::metric("budgets.over", failed as f64, "count");
            Ok(Outcome {
                json: json!({ "results": results, "over": failed, "skipped": skipped, "group": group }),
                text,
                exit: u8::from(failed > 0),
            })
        }
        MetricsCmd::Report { budgets, out } => {
            let b = if budgets.exists() {
                Budgets::load(budgets)?
            } else {
                Budgets::default()
            };
            let db = MetricsDb::open(&paths.metrics_db())?;
            let extra = scorecard::report_section(&db, paths);
            let page = gp_metrics::report::html_with(&db, &b, "Game Printer metrics", &extra)?;
            let out = out
                .clone()
                .unwrap_or_else(|| paths.reports_dir().join("metrics.html"));
            if let Some(dir) = out.parent() {
                fs::create_dir_all(dir)?;
            }
            fs::write(&out, &page).with_context(|| format!("writing {}", out.display()))?;
            runrec::output(
                "report",
                format!("blake3:{}", blake3::hash(page.as_bytes()).to_hex()),
                Some(page.len() as u64),
            );
            Ok(Outcome::ok(
                json!({ "report": out }),
                format!("report: {}", out.display()),
            ))
        }
    }
}
