//! The run recorder (gp-metrics), as game-printer kept it: every `gp`
//! command writes a run manifest under `$GP_HOME/runs/` and a row in
//! `$GP_HOME/metrics.sqlite`, with any metric the command recorded and its
//! own `gp.<command>.duration` (docs/formats/run-manifest.md).
//!
//! game-printer passed a `&mut RunRecorder` to every command. Here the
//! recorder is one per process, behind [`metric`], [`tool`] and [`output`],
//! so commands that record nothing keep their signatures. A run that can't
//! be written (a read-only `$GP_HOME`) is not an error: the command's own
//! output stands.

use std::sync::Mutex;

use clap::ArgMatches;
use gp_metrics::{MetricsDb, RunRecorder, RunStatus, SpanCollector, run::Tool};

static REC: Mutex<Option<RunRecorder>> = Mutex::new(None);

/// The command's name as game-printer recorded it: its subcommand path,
/// such as `metrics time` or `corpus score`.
pub fn command_name(m: &ArgMatches) -> String {
    let mut parts = Vec::new();
    let mut cur = m;
    while let Some((name, sub)) = cur.subcommand() {
        parts.push(name.to_owned());
        cur = sub;
    }
    parts.join(" ")
}

/// Starts the run. Installs the span timer (a `tracing` layer) once.
pub fn start(name: &str) {
    let spans = SpanCollector::default();
    {
        use tracing_subscriber::layer::SubscriberExt as _;
        use tracing_subscriber::util::SubscriberInitExt as _;
        let _ = tracing_subscriber::registry()
            .with(spans.layer())
            .try_init();
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let tool = Tool {
        name: "gp".into(),
        version: env!("CARGO_PKG_VERSION").into(),
    };
    let run_group = std::env::var("GP_RUN_GROUP").ok().filter(|g| !g.is_empty());
    let rec = RunRecorder::start(name, args, tool)
        .with_spans(spans)
        .with_run_group(run_group);
    *REC.lock().unwrap_or_else(|e| e.into_inner()) = Some(rec);
}

fn with(f: impl FnOnce(&mut RunRecorder)) {
    if let Some(r) = REC.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        f(r);
    }
}

/// Records one metric in this run.
pub fn metric(name: impl Into<String>, value: f64, unit: impl Into<String>) {
    with(|r| r.metric(name, value, unit));
}

/// Names another program this run depended on.
pub fn tool(name: &str, version: &str) {
    with(|r| r.tool(name, version));
}

/// Names something this run wrote.
pub fn output(name: &str, hash: String, bytes: Option<u64>) {
    with(|r| r.output(name, hash, bytes));
}

/// Ends the run: writes the manifest and the database row. Returns the
/// manifest's path, or None when there was no run or it couldn't be written.
pub fn finish(name: &str, exit: u8, error: Option<String>) -> Option<std::path::PathBuf> {
    let rec = REC.lock().unwrap_or_else(|e| e.into_inner()).take()?;
    let status = match (exit, error) {
        (_, Some(message)) => RunStatus::Failed { message },
        (0, None) => RunStatus::Ok,
        (n, None) => RunStatus::Failed {
            message: format!("exit code {n}"),
        },
    };
    let mut manifest = rec.finish(status);
    manifest.metrics.push(gp_metrics::Metric {
        name: format!("gp.{}.duration", name.replace(' ', ".")),
        value: manifest.duration_ms,
        unit: "ms".into(),
    });
    let paths = gp_evidence::Paths::from_env().ok()?;
    let path = manifest.write(&paths.runs_dir()).ok()?;
    MetricsDb::open(&paths.metrics_db())
        .and_then(|db| db.record_run(&manifest, &path))
        .ok()?;
    Some(path)
}
