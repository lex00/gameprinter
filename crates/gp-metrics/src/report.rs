//! A self-contained HTML report of every metric's recent history.

use std::fmt::Write as _;

use crate::MetricsError;
use crate::budgets::{BudgetStatus, Budgets};
use crate::db::{MetricRow, MetricsDb};

const HISTORY: usize = 30;

pub fn html(db: &MetricsDb, budgets: &Budgets, title: &str) -> Result<String, MetricsError> {
    html_with(db, budgets, title, "")
}

/// The report with `extra` (HTML) after the metrics table.
pub fn html_with(
    db: &MetricsDb,
    budgets: &Budgets,
    title: &str,
    extra: &str,
) -> Result<String, MetricsError> {
    let mut rows = String::new();
    for name in db.names()? {
        let history = db.history(&name, HISTORY)?;
        let Some(latest) = history.first() else {
            continue;
        };
        let previous = history.get(1);
        let delta = previous
            .filter(|p| p.value != 0.0)
            .map(|p| (latest.value - p.value) / p.value * 100.0);
        let status = match budgets.metrics.get(&name) {
            None => "",
            Some(_) => match budgets.check(&name, latest.value) {
                BudgetStatus::Fail => "<span class=\"fail\">over budget</span>",
                _ => "<span class=\"pass\">within budget</span>",
            },
        };
        let limit = budgets
            .metrics
            .get(&name)
            .map(|b| match (b.min, b.max) {
                (Some(lo), Some(hi)) => format!("{lo} to {hi}"),
                (None, Some(hi)) => format!("max {hi}"),
                (Some(lo), None) => format!("min {lo}"),
                (None, None) => String::new(),
            })
            .unwrap_or_default();
        let _ = write!(
            rows,
            "<tr><td><code>{}</code></td><td class=\"num\">{} {}</td><td class=\"num\">{}</td>\
             <td>{}</td><td>{} {}</td><td>{}</td></tr>",
            esc(&name),
            fmt_value(latest.value),
            esc(&latest.unit),
            delta
                .map(|d| format!("{d:+.1}%"))
                .unwrap_or_else(|| "-".into()),
            sparkline(&history),
            esc(&limit),
            status,
            commit_label(latest),
        );
    }
    if rows.is_empty() {
        rows.push_str(
            "<tr><td colspan=\"6\">No metrics recorded yet. Run <code>just check</code>.</td></tr>",
        );
    }

    let runs = db.run_count()?;
    Ok(format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<style>
:root {{ --bg: #ffffff; --fg: #1d1d1f; --muted: #6e6e73; --line: #d2d2d7; --pass: #1a7f37; --fail: #cf222e; --spark: #0969da; }}
@media (prefers-color-scheme: dark) {{
  :root {{ --bg: #111214; --fg: #e8e8ed; --muted: #a1a1a6; --line: #3a3a3c; --pass: #3fb950; --fail: #f85149; --spark: #58a6ff; }}
}}
body {{ background: var(--bg); color: var(--fg); font: 14px/1.5 -apple-system, system-ui, sans-serif; margin: 0 auto; max-width: 1100px; padding: 24px 16px; }}
h1 {{ font-size: 20px; margin: 0 0 4px; }}
p {{ color: var(--muted); margin: 0 0 20px; }}
.wrap {{ overflow-x: auto; }}
table {{ border-collapse: collapse; width: 100%; }}
th, td {{ border-bottom: 1px solid var(--line); padding: 6px 10px; text-align: left; white-space: nowrap; }}
th {{ color: var(--muted); font-weight: 500; }}
.num {{ font-variant-numeric: tabular-nums; text-align: right; }}
.pass {{ color: var(--pass); }} .fail {{ color: var(--fail); font-weight: 600; }}
svg polyline {{ fill: none; stroke: var(--spark); stroke-width: 1.5; }}
code {{ font: 13px ui-monospace, monospace; }}
</style>
</head>
<body>
<h1>{title}</h1>
<p>{runs} runs recorded. Trend shows up to the last {HISTORY} values, oldest on the left. Change is against the previous value.</p>
<div class="wrap"><table>
<thead><tr><th>Metric</th><th class="num">Latest</th><th class="num">Change</th><th>Trend</th><th>Budget</th><th>Commit</th></tr></thead>
<tbody>{rows}</tbody>
</table></div>
{extra}
</body>
</html>
"#,
        title = esc(title),
        extra = extra,
    ))
}

pub fn sparkline(history: &[MetricRow]) -> String {
    if history.len() < 2 {
        return String::new();
    }
    let values: Vec<f64> = history.iter().rev().map(|r| r.value).collect();
    let (lo, hi) = values
        .iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    let span = if hi > lo { hi - lo } else { 1.0 };
    let (w, h) = (120.0, 24.0);
    let step = w / (values.len() - 1) as f64;
    let points: Vec<String> = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            format!(
                "{:.1},{:.1}",
                i as f64 * step,
                h - 2.0 - (v - lo) / span * (h - 4.0)
            )
        })
        .collect();
    format!(
        "<svg width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\" role=\"img\" aria-label=\"trend\"><polyline points=\"{}\"/></svg>",
        points.join(" ")
    )
}

fn commit_label(row: &MetricRow) -> String {
    match &row.commit {
        Some(c) => format!(
            "<code>{}</code>{}",
            esc(&c[..c.len().min(8)]),
            if row.dirty { " (dirty)" } else { "" }
        ),
        None => "-".into(),
    }
}

pub fn fmt_value(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{v:.0}")
    } else if v.abs() >= 100.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.3}")
    }
}

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::{RunRecorder, RunStatus, Tool};

    #[test]
    fn renders_metrics_and_budgets() {
        let tmp = tempfile::tempdir().unwrap();
        let db = MetricsDb::open(&tmp.path().join("m.sqlite")).unwrap();
        for v in [10.0, 12.0, 30.0] {
            let mut rec = RunRecorder::start(
                "t",
                vec![],
                Tool {
                    name: "gp".into(),
                    version: "0".into(),
                },
            );
            rec.metric("check.test.duration", v, "ms");
            db.record_run(&rec.finish(RunStatus::Ok), std::path::Path::new("x"))
                .unwrap();
        }
        let budgets: Budgets =
            toml::from_str("[metrics.\"check.test.duration\"]\nmax = 20\n").unwrap();
        let page = html(&db, &budgets, "Check <report>").unwrap();
        assert!(page.contains("check.test.duration"));
        assert!(page.contains("over budget"));
        assert!(page.contains("+150.0%"));
        assert!(page.contains("<polyline"));
        assert!(page.contains("Check &lt;report&gt;"));
    }

    #[test]
    fn empty_database() {
        let tmp = tempfile::tempdir().unwrap();
        let db = MetricsDb::open(&tmp.path().join("m.sqlite")).unwrap();
        let page = html(&db, &Budgets::default(), "r").unwrap();
        assert!(page.contains("No metrics recorded yet"));
    }
}
