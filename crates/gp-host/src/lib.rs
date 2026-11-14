//! The local host (Milestone 3, docs/host.md): interpreter sessions per
//! working copy, parallel scenario runs, a queue of background builds with
//! status, and playable links on localhost.
//!
//! [`Host`] is the interface; [`local::LocalHost`] is the implementation
//! that runs everything in this process and on this machine. A hosted
//! runtime would implement the same trait over the network, and callers
//! (the CLI, the POC loop, an agent through `gp host`) would not change.
//! Everything crossing the trait is a serde type or JSON, so it can.

pub mod http;
pub mod links;
pub mod local;
pub mod session;
pub mod speed;

// Scenarios and the input search live in gp-mcp, so `validate` can run
// them; the host serves and schedules them.
pub use gp_mcp::{scenario, search};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A background job, by what it does.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum JobSpec {
    /// Build a snapshot of a working copy for the browser, as
    /// `build_publish` does, and register a link for it.
    Build {
        copy: String,
        /// `web-preview` (default), the only target.
        #[serde(default)]
        target: Option<String>,
        #[serde(default)]
        fetch_tools: bool,
    },
    /// Run a copy's (or a template's) scenarios in parallel.
    Scenarios {
        copy: String,
        #[serde(default)]
        template: bool,
        #[serde(default)]
        names: Vec<String>,
        #[serde(default)]
        jobs: Option<usize>,
    },
}

impl JobSpec {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Build { .. } => "build",
            Self::Scenarios { .. } => "scenarios",
        }
    }
    pub fn copy(&self) -> &str {
        match self {
            Self::Build { copy, .. } | Self::Scenarios { copy, .. } => copy,
        }
    }
}

/// Where a job is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed,
}

/// A job as a caller sees it.
#[derive(Debug, Clone, Serialize)]
pub struct JobView {
    pub id: String,
    pub kind: &'static str,
    pub copy: String,
    pub state: JobState,
    /// Milliseconds spent waiting for a worker.
    pub wait_ms: u128,
    /// Milliseconds running (so far, while it runs).
    pub run_ms: u128,
    /// What it produced: a build's manifest and link, a scenario report.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// For a build that finished, the link to play it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<links::Link>,
}

/// What a step of a session is asked for.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepRequest {
    /// Frames to run (default 1).
    #[serde(default)]
    pub frames: Option<usize>,
    /// Buttons held for every frame of the step, such as `["right", "b"]`.
    #[serde(default)]
    pub buttons: Vec<String>,
    /// Holds relative to the step's first frame, as `run_preview` takes
    /// them (`a@0-10`).
    #[serde(default)]
    pub holds: Vec<String>,
    /// State paths to read after the step.
    #[serde(default)]
    pub paths: Vec<String>,
}

/// The interface a runtime host offers. Methods block; a remote
/// implementation would turn each into a request.
pub trait Host: Send + Sync {
    /// Starts an interpreter session on a working copy at frame 0.
    fn open_session(&self, copy: &str) -> Result<Value, String>;
    /// Runs frames. If the copy's model changed since the session last ran,
    /// the session reloads and replays its input first.
    fn step_session(&self, id: &str, step: &StepRequest) -> Result<Value, String>;
    fn query_session(&self, id: &str, paths: &[String]) -> Result<Value, String>;
    fn close_session(&self, id: &str) -> Result<(), String>;
    fn sessions(&self) -> Vec<Value>;

    /// Runs scenarios now, in parallel, and returns the report.
    fn run_scenarios(&self, req: &scenario::Request) -> Result<Value, String>;
    /// Records a run of the copy as a scenario in the copy.
    fn record_scenario(&self, req: &scenario::Record) -> Result<Value, String>;

    /// Looks for holds that reach a goal in the copy's own model.
    fn search_inputs(&self, req: &search::Search) -> Result<Value, String>;

    /// Queues a background job and returns its ID.
    fn submit(&self, spec: JobSpec) -> Result<String, String>;
    fn job(&self, id: &str) -> Option<JobView>;
    fn jobs(&self) -> Vec<JobView>;

    /// Gives a published directory a playable link.
    fn register_link(&self, dir: &std::path::Path) -> Result<links::Link, String>;
    fn links(&self) -> Vec<links::Link>;

    /// Speed measurements so far, held to their budgets.
    fn speed(&self) -> Value;
    /// Adds a measurement from a caller (the POC loop's plan time).
    fn record_speed(&self, name: &str, seconds: f64) -> Result<Value, String>;
}

#[cfg(test)]
mod tests;
