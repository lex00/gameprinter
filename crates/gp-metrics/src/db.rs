//! Metric history in SQLite: one row per run and one per recorded metric.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::MetricsError;
use crate::run::RunManifest;

pub struct MetricsDb {
    conn: Connection,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MetricRow {
    pub run_id: String,
    pub commit: Option<String>,
    pub dirty: bool,
    pub arch: String,
    pub name: String,
    pub value: f64,
    pub unit: String,
    pub recorded_at: String,
    pub run_group: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunRow {
    pub run_id: String,
    pub command: String,
    pub commit: Option<String>,
    pub dirty: bool,
    pub started_at: String,
    pub duration_ms: f64,
    pub ok: bool,
    pub manifest_path: String,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS runs (
    run_id        TEXT PRIMARY KEY,
    command       TEXT NOT NULL,
    commit_sha    TEXT,
    dirty         INTEGER NOT NULL,
    os            TEXT NOT NULL,
    arch          TEXT NOT NULL,
    started_at    TEXT NOT NULL,
    duration_ms   REAL NOT NULL,
    ok            INTEGER NOT NULL,
    manifest_path TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS metrics (
    run_id      TEXT NOT NULL REFERENCES runs(run_id),
    commit_sha  TEXT,
    dirty       INTEGER NOT NULL,
    arch        TEXT NOT NULL,
    name        TEXT NOT NULL,
    value       REAL NOT NULL,
    unit        TEXT NOT NULL,
    recorded_at TEXT NOT NULL,
    run_group   TEXT
);
CREATE INDEX IF NOT EXISTS metrics_by_name ON metrics(name, recorded_at);
";

impl MetricsDb {
    pub fn open(path: &Path) -> Result<Self, MetricsError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| MetricsError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(SCHEMA)?;
        // Databases from before run groups get the column added in place.
        let has_group: bool = conn
            .prepare("SELECT 1 FROM pragma_table_info('metrics') WHERE name = 'run_group'")?
            .exists([])?;
        if !has_group {
            conn.execute_batch("ALTER TABLE metrics ADD COLUMN run_group TEXT;")?;
        }
        Ok(Self { conn })
    }

    pub fn record_run(&self, m: &RunManifest, manifest_path: &Path) -> Result<(), MetricsError> {
        let commit = m.git.as_ref().map(|g| g.commit.as_str());
        let dirty = m.git.as_ref().is_some_and(|g| g.dirty);
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT OR REPLACE INTO runs VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                m.run_id,
                m.command,
                commit,
                dirty,
                m.host.os,
                m.host.arch,
                m.started_at,
                m.duration_ms,
                matches!(m.status, crate::RunStatus::Ok),
                manifest_path.to_string_lossy(),
            ],
        )?;
        for metric in &m.metrics {
            tx.execute(
                "INSERT INTO metrics (run_id, commit_sha, dirty, arch, name, value, unit, recorded_at, run_group)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    m.run_id,
                    commit,
                    dirty,
                    m.host.arch,
                    metric.name,
                    metric.value,
                    metric.unit,
                    m.finished_at,
                    m.run_group,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn names(&self) -> Result<Vec<String>, MetricsError> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT name FROM metrics ORDER BY name")?;
        let names = stmt
            .query_map([], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        Ok(names)
    }

    pub fn latest(&self, name: &str) -> Result<Option<MetricRow>, MetricsError> {
        Ok(self.history(name, 1)?.into_iter().next())
    }

    /// The most recent value of a metric recorded in one run group.
    pub fn latest_in_group(
        &self,
        name: &str,
        group: &str,
    ) -> Result<Option<MetricRow>, MetricsError> {
        let mut stmt = self.conn.prepare(
            "SELECT run_id, commit_sha, dirty, arch, name, value, unit, recorded_at, run_group
             FROM metrics WHERE name = ?1 AND run_group = ?2 ORDER BY recorded_at DESC, rowid DESC LIMIT 1",
        )?;
        let row = stmt
            .query_map(params![name, group], row_to_metric)?
            .next()
            .transpose()?;
        Ok(row)
    }

    /// The most recent values of a metric, newest first.
    pub fn history(&self, name: &str, limit: usize) -> Result<Vec<MetricRow>, MetricsError> {
        let mut stmt = self.conn.prepare(
            "SELECT run_id, commit_sha, dirty, arch, name, value, unit, recorded_at, run_group
             FROM metrics WHERE name = ?1 ORDER BY recorded_at DESC, rowid DESC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![name, limit as i64], row_to_metric)?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    pub fn recent_runs(&self, limit: usize) -> Result<Vec<RunRow>, MetricsError> {
        let mut stmt = self.conn.prepare(
            "SELECT run_id, command, commit_sha, dirty, started_at, duration_ms, ok, manifest_path
             FROM runs ORDER BY started_at DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit as i64], |r| {
                Ok(RunRow {
                    run_id: r.get(0)?,
                    command: r.get(1)?,
                    commit: r.get(2)?,
                    dirty: r.get(3)?,
                    started_at: r.get(4)?,
                    duration_ms: r.get(5)?,
                    ok: r.get(6)?,
                    manifest_path: r.get(7)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    pub fn run_count(&self) -> Result<u64, MetricsError> {
        let n: Option<i64> = self
            .conn
            .query_row("SELECT COUNT(*) FROM runs", [], |r| r.get(0))
            .optional()?;
        Ok(n.unwrap_or(0) as u64)
    }
}

fn row_to_metric(r: &rusqlite::Row<'_>) -> rusqlite::Result<MetricRow> {
    Ok(MetricRow {
        run_id: r.get(0)?,
        commit: r.get(1)?,
        dirty: r.get(2)?,
        arch: r.get(3)?,
        name: r.get(4)?,
        value: r.get(5)?,
        unit: r.get(6)?,
        recorded_at: r.get(7)?,
        run_group: r.get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::{RunRecorder, RunStatus, Tool};

    fn manifest(value: f64) -> RunManifest {
        let mut rec = RunRecorder::start(
            "test",
            vec![],
            Tool {
                name: "gp".into(),
                version: "0".into(),
            },
        );
        rec.metric("noop.duration", value, "ms");
        rec.finish(RunStatus::Ok)
    }

    #[test]
    fn records_and_reads_back_newest_first() {
        let tmp = tempfile::tempdir().unwrap();
        let db = MetricsDb::open(&tmp.path().join("m.sqlite")).unwrap();
        db.record_run(&manifest(3.0), Path::new("a.json")).unwrap();
        db.record_run(&manifest(5.0), Path::new("b.json")).unwrap();
        let h = db.history("noop.duration", 10).unwrap();
        assert_eq!(h.iter().map(|r| r.value).collect::<Vec<_>>(), [5.0, 3.0]);
        assert_eq!(db.latest("noop.duration").unwrap().unwrap().value, 5.0);
        assert_eq!(db.names().unwrap(), ["noop.duration"]);
        assert_eq!(db.run_count().unwrap(), 2);
        assert_eq!(db.recent_runs(1).unwrap().len(), 1);
    }
}
