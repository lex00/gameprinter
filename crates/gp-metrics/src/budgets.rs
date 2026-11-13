//! Budgets: limits on metrics that fail the check suite when crossed.
//!
//! ```toml
//! [metrics."gp.noop.duration"]
//! max = 200
//! unit = "ms"
//! note = "startup cost of every command"
//! ```

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::MetricsError;
use crate::db::MetricsDb;

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Budgets {
    #[serde(default)]
    pub metrics: BTreeMap<String, Budget>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Budget {
    pub max: Option<f64>,
    pub min: Option<f64>,
    /// Operating systems this budget applies on (`macos`, `linux`, ...), for
    /// checks that only run on some platforms. Absent means everywhere.
    pub os: Option<Vec<String>>,
    /// Timing budgets mean nothing under CPU emulation, so these are skipped
    /// when `GP_EMULATED=1` (set by `just docker-check-amd64` on an ARM host).
    #[serde(default)]
    pub native_only: bool,
    pub unit: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BudgetStatus {
    Pass,
    Fail,
    /// Never recorded at all. Reported, not a failure.
    NoData,
    /// Not recorded in the run group being checked: the step that records it
    /// didn't run. A failure, so a stale passing value can't hide it.
    Missing,
    /// Doesn't apply here: another OS, or a timing budget under emulation.
    /// Listed with its reason so a skip is never silent.
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BudgetResult {
    pub name: String,
    pub value: Option<f64>,
    pub unit: Option<String>,
    pub max: Option<f64>,
    pub min: Option<f64>,
    pub status: BudgetStatus,
    /// Why a budget was skipped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Whether this run is under CPU emulation, as the environment declares.
pub fn emulated() -> bool {
    std::env::var("GP_EMULATED").is_ok_and(|v| v == "1")
}

impl Budgets {
    pub fn load(path: &Path) -> Result<Self, MetricsError> {
        let text = std::fs::read_to_string(path).map_err(|source| MetricsError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        toml::from_str(&text).map_err(|source| MetricsError::Budgets {
            path: path.to_path_buf(),
            source,
        })
    }

    pub fn check(&self, name: &str, value: f64) -> BudgetStatus {
        match self.metrics.get(name) {
            Some(b) if b.max.is_some_and(|m| value > m) || b.min.is_some_and(|m| value < m) => {
                BudgetStatus::Fail
            }
            _ => BudgetStatus::Pass,
        }
    }

    /// Checks every budgeted metric. With a run group, only values recorded in
    /// that group count, and a metric the group didn't record fails. Without
    /// one, the latest value ever recorded is checked.
    pub fn evaluate(
        &self,
        db: &MetricsDb,
        group: Option<&str>,
    ) -> Result<Vec<BudgetResult>, MetricsError> {
        let mut out = Vec::new();
        for (name, b) in &self.metrics {
            let skip = if b
                .os
                .as_ref()
                .is_some_and(|os| !os.iter().any(|o| o == std::env::consts::OS))
            {
                Some(format!(
                    "runs only on {}",
                    b.os.as_deref().unwrap_or_default().join(", ")
                ))
            } else if b.native_only && emulated() {
                Some("timing budget, skipped under CPU emulation".to_owned())
            } else {
                None
            };
            if let Some(reason) = skip {
                out.push(BudgetResult {
                    name: name.clone(),
                    value: None,
                    unit: b.unit.clone(),
                    max: b.max,
                    min: b.min,
                    status: BudgetStatus::Skipped,
                    reason: Some(reason),
                });
                continue;
            }
            let latest = match group {
                Some(g) => db.latest_in_group(name, g)?,
                None => db.latest(name)?,
            };
            let status = match (&latest, group) {
                (None, Some(_)) => BudgetStatus::Missing,
                (None, None) => BudgetStatus::NoData,
                (Some(row), _) => self.check(name, row.value),
            };
            out.push(BudgetResult {
                name: name.clone(),
                value: latest.as_ref().map(|r| r.value),
                unit: latest.map(|r| r.unit).or_else(|| b.unit.clone()),
                max: b.max,
                min: b.min,
                status,
                reason: None,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_checks_limits() {
        let b: Budgets = toml::from_str(
            r#"
            [metrics."a"]
            max = 10
            [metrics."b"]
            min = 0.9
            "#,
        )
        .unwrap();
        assert_eq!(b.check("a", 9.0), BudgetStatus::Pass);
        assert_eq!(b.check("a", 11.0), BudgetStatus::Fail);
        assert_eq!(b.check("b", 0.5), BudgetStatus::Fail);
        assert_eq!(b.check("unbudgeted", 1e9), BudgetStatus::Pass);
    }

    #[test]
    fn budgets_for_other_platforms_are_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let db = MetricsDb::open(&tmp.path().join("m.sqlite")).unwrap();
        let b: Budgets = toml::from_str(
            "[metrics.\"elsewhere\"]\nmax = 0\nos = [\"plan9\"]\n[metrics.\"here\"]\nmax = 0\n",
        )
        .unwrap();
        let results = b.evaluate(&db, Some("g")).unwrap();
        let elsewhere = results.iter().find(|r| r.name == "elsewhere").unwrap();
        assert_eq!(elsewhere.status, BudgetStatus::Skipped);
        assert_eq!(elsewhere.reason.as_deref(), Some("runs only on plan9"));
        let here = results.iter().find(|r| r.name == "here").unwrap();
        assert_eq!(here.status, BudgetStatus::Missing);
    }

    #[test]
    fn a_run_group_ignores_stale_values() {
        use crate::run::{RunRecorder, RunStatus, Tool};
        let tmp = tempfile::tempdir().unwrap();
        let db = MetricsDb::open(&tmp.path().join("m.sqlite")).unwrap();
        let record = |group: &str, value: f64| {
            let mut rec = RunRecorder::start(
                "t",
                vec![],
                Tool {
                    name: "gp".into(),
                    version: "0".into(),
                },
            )
            .with_run_group(Some(group.into()));
            rec.metric("step.exit", value, "code");
            db.record_run(&rec.finish(RunStatus::Ok), Path::new("x"))
                .unwrap();
        };
        let b: Budgets = toml::from_str("[metrics.\"step.exit\"]\nmax = 0\n").unwrap();

        record("old-check", 0.0);
        // A later check where the step never ran: the old pass must not count.
        let results = b.evaluate(&db, Some("new-check")).unwrap();
        assert_eq!(results[0].status, BudgetStatus::Missing);
        assert_eq!(b.evaluate(&db, None).unwrap()[0].status, BudgetStatus::Pass);

        record("new-check", 1.0);
        assert_eq!(
            b.evaluate(&db, Some("new-check")).unwrap()[0].status,
            BudgetStatus::Fail
        );
        assert_eq!(
            b.evaluate(&db, Some("old-check")).unwrap()[0].status,
            BudgetStatus::Pass
        );
    }
}
