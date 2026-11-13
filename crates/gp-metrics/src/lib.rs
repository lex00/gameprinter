//! Run manifests, metrics and their history. Every `gp` command records what
//! it read, what it wrote, how long each traced step took and any metrics, so
//! any run can be reproduced and every number can be tracked per commit.
//! See docs/formats/run-manifest.md.

pub mod budgets;
pub mod db;
pub mod report;
pub mod run;
pub mod spans;

pub use budgets::{BudgetStatus, Budgets};
pub use db::MetricsDb;
pub use run::{Artifact, Metric, RunManifest, RunRecorder, RunStatus};
pub use spans::SpanCollector;

#[derive(Debug, thiserror::Error)]
pub enum MetricsError {
    #[error("metrics database: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("{path}: {source}")]
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("budgets file {path}: {source}")]
    Budgets {
        path: std::path::PathBuf,
        source: toml::de::Error,
    },
}
