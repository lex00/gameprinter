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
    /// [`git_info`], asked on a thread of its own as the run starts
    /// ([`RunRecorder::with_git_cache`]), so the command doesn't wait on
    /// git; without one, asked as the run ends.
    git: Option<std::thread::JoinHandle<Option<GitInfo>>>,
    /// The answer once [`RunRecorder::git`] has waited for it.
    git_known: Option<Option<GitInfo>>,
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
            git: None,
            git_known: None,
        }
    }

    /// The run's git state, the one its manifest records: the answer of
    /// the thread [`RunRecorder::with_git_cache`] started (so a command
    /// that shows it asks git no more than the record does), or
    /// [`git_info`] without one.
    pub fn git(&mut self) -> Option<GitInfo> {
        if self.git_known.is_none() {
            self.git_known = Some(match self.git.take() {
                Some(asked) => asked.join().ok().flatten(),
                None => git_info(),
            });
        }
        self.git_known.clone().flatten()
    }

    /// Asks git for the run's [`GitInfo`] now, on a thread of its own,
    /// answered from `cache` (`$GP_HOME/runs/git-state.json`) when the
    /// work tree is as git last saw it ([`crate::gitstate`]).
    pub fn with_git_cache(mut self, cache: PathBuf) -> Self {
        let dir = std::env::current_dir();
        self.git = std::thread::Builder::new()
            .name("gp-run-git".into())
            .spawn(move || crate::gitstate::git_info_cached(&dir.ok()?, &cache))
            .ok();
        self
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
            git: match (self.git_known, self.git) {
                (Some(known), _) => known,
                (None, Some(asked)) => asked.join().ok().flatten(),
                (None, None) => git_info(),
            },
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

/// The current directory's git state, if it's in a work tree: what
/// `git status --porcelain=v2 --branch --untracked-files=no` says, one git
/// process (two cost about 40 ms on macOS, where `/usr/bin/git` is a shim
/// that runs xcrun first, and every command paid it). It takes no optional
/// locks, so it never holds the index while another git writes it.
pub fn git_info() -> Option<GitInfo> {
    git_info_in(None)
}

/// [`git_info`] of `dir`, or of the current directory.
pub(crate) fn git_info_in(dir: Option<&Path>) -> Option<GitInfo> {
    let mut git = Command::new(git_program());
    if let Some(dir) = dir {
        git.current_dir(dir);
    }
    let out = git
        .args([
            "--no-optional-locks",
            "status",
            "--porcelain=v2",
            "--branch",
            "--untracked-files=no",
        ])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_status(&String::from_utf8_lossy(&out.stdout))
}

/// [`GitInfo`] from `git status --porcelain=v2 --branch`'s text: the
/// commit from `branch.oid` (none before the first commit), the branch
/// from `branch.head` (none when detached), and dirty when any tracked
/// file's line follows.
fn parse_status(text: &str) -> Option<GitInfo> {
    let (mut commit, mut branch, mut dirty) = (None, None, false);
    for line in text.lines() {
        if let Some(oid) = line.strip_prefix("# branch.oid ") {
            commit = Some(oid.trim().to_owned()).filter(|c| c != "(initial)");
        } else if let Some(head) = line.strip_prefix("# branch.head ") {
            branch = Some(head.trim().to_owned()).filter(|b| b != "(detached)");
        } else if !line.starts_with('#') && !line.is_empty() {
            dirty = true;
        }
    }
    Some(GitInfo {
        commit: commit?,
        dirty,
        branch,
    })
}

/// The git to run: the first `git` on PATH, except that on macOS the
/// `/usr/bin/git` shim is passed over for the git it would run (the
/// selected developer folder's, by `DEVELOPER_DIR` or `xcode-select`'s
/// link), which saves the shim's xcrun lookup, about 10 ms a call.
fn git_program() -> PathBuf {
    let on_path = std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|d| d.join("git"))
            .find(|g| g.is_file())
    });
    match on_path {
        Some(g) if cfg!(target_os = "macos") && g == Path::new("/usr/bin/git") => {
            developer_git().unwrap_or(g)
        }
        Some(g) => g,
        None => PathBuf::from("git"),
    }
}

/// The git inside macOS's selected developer folder, if there is one.
fn developer_git() -> Option<PathBuf> {
    let dir = std::env::var_os("DEVELOPER_DIR")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::fs::read_link("/var/db/xcode_select_link").ok())
        .unwrap_or_else(|| PathBuf::from("/Library/Developer/CommandLineTools"));
    let git = dir.join("usr/bin/git");
    git.is_file().then_some(git)
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

    /// A command that shows the run's git state (gp env) gets the answer
    /// the record's own thread gave, and the manifest records the same.
    #[test]
    fn the_run_git_state_is_asked_once_and_recorded_as_shown() {
        let info = GitInfo {
            commit: "made-up".into(),
            dirty: true,
            branch: Some("work".into()),
        };
        let mut rec = RunRecorder::start("env", vec![], tool());
        let given = info.clone();
        rec.git = Some(std::thread::spawn(move || Some(given)));
        assert_eq!(rec.git(), Some(info.clone()));
        assert_eq!(rec.git(), Some(info.clone()), "asked again, the same");
        assert_eq!(rec.finish(RunStatus::Ok).git, Some(info));
    }

    #[test]
    fn git_status_names_the_commit_the_branch_and_any_change() {
        let oid = "f699af1d38edf802c7edec44fcdf04717db5b116";
        let clean = format!("# branch.oid {oid}\n# branch.head main\n");
        assert_eq!(
            parse_status(&clean),
            Some(GitInfo {
                commit: oid.into(),
                dirty: false,
                branch: Some("main".into()),
            })
        );
        let changed = format!(
            "# branch.oid {oid}\n# branch.head (detached)\n1 .M N... 100644 100644 100644 {oid} {oid} src/lib.rs\n"
        );
        assert_eq!(
            parse_status(&changed),
            Some(GitInfo {
                commit: oid.into(),
                dirty: true,
                branch: None,
            })
        );
        assert_eq!(
            parse_status("# branch.oid (initial)\n# branch.head main\n"),
            None
        );
        assert_eq!(parse_status(""), None);
    }

    /// In a work tree of its own, the state git itself reports: the
    /// commit, the branch, and dirty only once a tracked file changes.
    #[test]
    fn git_info_reads_a_work_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let ok = Command::new("git")
                .args([
                    "-c",
                    "user.name=t",
                    "-c",
                    "user.email=t@t",
                    "-c",
                    "commit.gpgsign=false",
                ])
                .args(args)
                .current_dir(tmp.path())
                .output()
                .unwrap();
            assert!(ok.status.success(), "{args:?}: {ok:?}");
            String::from_utf8(ok.stdout).unwrap().trim().to_owned()
        };
        git(&["init", "-q", "-b", "trunk"]);
        std::fs::write(tmp.path().join("a"), "one").unwrap();
        git(&["add", "a"]);
        git(&["commit", "-q", "-m", "one"]);
        std::fs::write(tmp.path().join("untracked"), "x").unwrap();
        let head = git(&["rev-parse", "HEAD"]);
        let at = |dir: &Path| git_info_in(Some(dir));
        assert_eq!(
            at(tmp.path()),
            Some(GitInfo {
                commit: head.clone(),
                dirty: false,
                branch: Some("trunk".into()),
            })
        );
        std::fs::write(tmp.path().join("a"), "two").unwrap();
        assert!(at(tmp.path()).unwrap().dirty);
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
