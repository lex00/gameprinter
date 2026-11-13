use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::MetricsError;
use crate::spans::{SpanCollector, SpanTiming};

/// Run manifest format version.
pub const RUN_SCHEMA: u32 = 0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunManifest {
    pub schema: u32,
    pub run_id: String,
    pub command: String,
    pub args: Vec<String>,
    /// Shared by every run from one check (`$GP_RUN_GROUP`), so budgets can
    /// be checked against that check's values only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_group: Option<String>,
    pub started_at: String,
    pub finished_at: String,
    pub duration_ms: f64,
    pub status: RunStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<GitInfo>,
    pub host: Host,
    pub tool: Tool,
    /// Other programs the run depended on, with their versions, such as the
    /// wasmtime that ran a WASM check.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<Tool>,
    #[serde(default)]
    pub inputs: Vec<Artifact>,
    #[serde(default)]
    pub outputs: Vec<Artifact>,
    #[serde(default)]
    pub metrics: Vec<Metric>,
    #[serde(default)]
    pub spans: Vec<SpanTiming>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum RunStatus {
    Ok,
    Failed { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitInfo {
    pub commit: String,
    pub dirty: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Host {
    pub os: String,
    pub arch: String,
}

impl Host {
    pub fn current() -> Self {
        Self {
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub version: String,
}

/// Something a run read or wrote, named by a prefixed hash such as
/// `blake3:<hex>` or `sha256:<hex>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artifact {
    pub name: String,
    pub hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metric {
    pub name: String,
    pub value: f64,
    pub unit: String,
}

/// Collects a run manifest while a command runs.
pub struct RunRecorder {
    command: String,
    args: Vec<String>,
    tool: Tool,
    run_group: Option<String>,
    started: Instant,
    started_at: OffsetDateTime,
    spans: Option<SpanCollector>,
    tools: Vec<Tool>,
    inputs: Vec<Artifact>,
    outputs: Vec<Artifact>,
    metrics: Vec<Metric>,
}

impl RunRecorder {
    pub fn start(command: impl Into<String>, args: Vec<String>, tool: Tool) -> Self {
        Self {
            command: command.into(),
            args,
            tool,
            run_group: None,
            started: Instant::now(),
            started_at: OffsetDateTime::now_utc(),
            spans: None,
            tools: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            metrics: Vec::new(),
        }
    }

    pub fn with_run_group(mut self, group: Option<String>) -> Self {
        self.run_group = group;
        self
    }

    pub fn with_spans(mut self, spans: SpanCollector) -> Self {
        self.spans = Some(spans);
        self
    }

    /// Notes another program the run depended on, and its version.
    pub fn tool(&mut self, name: impl Into<String>, version: impl Into<String>) {
        self.tools.push(Tool {
            name: name.into(),
            version: version.into(),
        });
    }

    pub fn input(&mut self, name: impl Into<String>, hash: impl Into<String>, bytes: Option<u64>) {
        self.inputs.push(Artifact {
            name: name.into(),
            hash: hash.into(),
            bytes,
        });
    }

    pub fn output(&mut self, name: impl Into<String>, hash: impl Into<String>, bytes: Option<u64>) {
        self.outputs.push(Artifact {
            name: name.into(),
            hash: hash.into(),
            bytes,
        });
    }

    pub fn metric(&mut self, name: impl Into<String>, value: f64, unit: impl Into<String>) {
        self.metrics.push(Metric {
            name: name.into(),
            value,
            unit: unit.into(),
        });
    }

    pub fn finish(self, status: RunStatus) -> RunManifest {
        let elapsed = self.started.elapsed();
        let finished_at = self.started_at + elapsed;
        RunManifest {
            schema: RUN_SCHEMA,
            run_id: uuid::Uuid::now_v7().to_string(),
            command: self.command,
            args: self.args,
            run_group: self.run_group,
            started_at: rfc3339(self.started_at),
            finished_at: rfc3339(finished_at),
            duration_ms: elapsed.as_secs_f64() * 1000.0,
            status,
            git: git_info(),
            host: Host::current(),
            tool: self.tool,
            tools: self.tools,
            inputs: self.inputs,
            outputs: self.outputs,
            metrics: self.metrics,
            spans: self.spans.map(|s| s.drain()).unwrap_or_default(),
        }
    }
}

/// RFC 3339 in UTC with exactly six fractional digits, so that comparing two
/// timestamps as strings (as the metrics database's ORDER BY does) gives the
/// same order as comparing the times. Plain RFC 3339 formatting trims trailing
/// zeros, which breaks that: "…:23.79Z" sorts after "…:23.796707Z".
pub fn rfc3339(t: OffsetDateTime) -> String {
    let format = time::macros::format_description!(
        "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:6]Z"
    );
    t.to_offset(time::UtcOffset::UTC)
        .format(&format)
        .expect("UTC timestamps always format")
}

impl RunManifest {
    /// Writes the manifest to `<runs_dir>/<YYYY-MM-DD>/<run_id>.json`.
    pub fn write(&self, runs_dir: &Path) -> Result<PathBuf, MetricsError> {
        let day = self.started_at.get(..10).unwrap_or("unknown");
        let dir = runs_dir.join(day);
        std::fs::create_dir_all(&dir).map_err(|source| MetricsError::Io {
            path: dir.clone(),
            source,
        })?;
        let path = dir.join(format!("{}.json", self.run_id));
        let json = serde_json::to_string_pretty(self).expect("manifests always serialize");
        std::fs::write(&path, json).map_err(|source| MetricsError::Io {
            path: path.clone(),
            source,
        })?;
        Ok(path)
    }
}

/// The current directory's git state, if it's in a work tree.
pub fn git_info() -> Option<GitInfo> {
    let run = |args: &[&str]| -> Option<String> {
        let out = Command::new("git").args(args).output().ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
    };
    // Two git processes, not three: each spawn costs about 20 ms, and every
    // command pays this.
    let refs = run(&["rev-parse", "HEAD", "--abbrev-ref", "HEAD"])?;
    let mut lines = refs.lines();
    let commit = lines.next()?.to_owned();
    let branch = lines.next().filter(|b| *b != "HEAD").map(str::to_owned);
    let dirty =
        run(&["status", "--porcelain", "--untracked-files=no"]).is_some_and(|s| !s.is_empty());
    Some(GitInfo {
        commit,
        dirty,
        branch,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool() -> Tool {
        Tool {
            name: "gp".into(),
            version: "0.1.0".into(),
        }
    }

    #[test]
    fn finish_fills_times_and_host() {
        let mut rec = RunRecorder::start("noop", vec![], tool());
        rec.metric("x", 1.5, "ms");
        rec.input("rom", "sha256:00", Some(40976));
        let m = rec.finish(RunStatus::Ok);
        assert_eq!(m.schema, RUN_SCHEMA);
        assert_eq!(m.metrics.len(), 1);
        assert!(m.duration_ms >= 0.0);
        assert!(m.started_at <= m.finished_at);
        assert_eq!(m.host, Host::current());
        assert_eq!(m.run_id.len(), 36);
    }

    #[test]
    fn timestamps_sort_as_strings() {
        let base = time::macros::datetime!(2026-10-27 07:00:23.79 UTC);
        let later = base + time::Duration::microseconds(6707);
        let (a, b) = (rfc3339(base), rfc3339(later));
        assert_eq!(a, "2026-10-27T07:00:23.790000Z");
        assert!(a < b, "{a} should sort before {b}");
        let rfc = time::format_description::well_known::Rfc3339;
        assert!(OffsetDateTime::parse(&a, &rfc).is_ok());
    }

    #[test]
    fn writes_under_the_start_date() {
        let tmp = tempfile::tempdir().unwrap();
        let m = RunRecorder::start("noop", vec![], tool()).finish(RunStatus::Ok);
        let path = m.write(tmp.path()).unwrap();
        assert!(path.starts_with(tmp.path().join(&m.started_at[..10])));
        let back: RunManifest = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(back, m);
    }
}
