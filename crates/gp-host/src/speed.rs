//! The speed targets as tracked metrics (ROADMAP, Milestone 3; docs/host.md).
//!
//! | Metric | Measured from | To | Budget | Target |
//! |---|---|---|---|---|
//! | `plan_s` | The task starts | A concept plan: the template found, forked and its surface read, with the numbers to compare against taken | 30 s | 10 s |
//! | `first_link_s` | The fork starts | The first playable link answers | 120 s | 60 s |
//! | `tweak_s` | A model edit is sent | The edit's effect is read back from a preview run | 5 s | 1 s |
//! | `tweak_link_s` | A model edit is sent | The rebuilt playable link answers | 30 s | 10 s |
//! | `rom_to_preview_s` | `gp capture start` is given a ROM | The run's first interpreter preview is built | 60 s | 20 s |
//!
//! The POC loop measures the first three with its own clock around the MCP
//! calls (so they are the tools' time, not a model's thinking time) and the
//! fourth from the host's job record. `gp host speed` records them as run
//! metrics and fails over budget; `budgets-cycle.toml` holds the same limits
//! for the metrics database.

use serde::Serialize;

/// One tracked quantity.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Target {
    pub name: &'static str,
    pub what: &'static str,
    /// Over this is a failure.
    pub budget_s: f64,
    /// What a good run is.
    pub target_s: f64,
}

pub const TARGETS: [Target; 5] = [
    Target {
        name: "plan_s",
        what: "task start to a concept plan (template found, forked, surface read)",
        budget_s: 30.0,
        target_s: 10.0,
    },
    Target {
        name: "first_link_s",
        what: "fork start to the first playable link answering",
        budget_s: 120.0,
        target_s: 60.0,
    },
    Target {
        name: "tweak_s",
        what: "a model edit sent to its effect read back from a preview run",
        budget_s: 5.0,
        target_s: 1.0,
    },
    Target {
        name: "tweak_link_s",
        what: "a model edit sent to the rebuilt playable link answering",
        budget_s: 30.0,
        target_s: 10.0,
    },
    Target {
        name: "rom_to_preview_s",
        what: "a ROM handed to gp capture start to the run's first interpreter preview",
        budget_s: 60.0,
        target_s: 20.0,
    },
];

pub fn target(name: &str) -> Option<&'static Target> {
    TARGETS.iter().find(|t| t.name == name)
}

/// A measurement held to its budget.
#[derive(Debug, Clone, Serialize)]
pub struct Verdict {
    pub name: String,
    pub seconds: f64,
    pub budget_s: f64,
    pub target_s: f64,
    pub within_budget: bool,
    pub within_target: bool,
}

pub fn judge(name: &str, seconds: f64) -> Result<Verdict, String> {
    let t = target(name).ok_or(format!(
        "`{name}` isn't a speed metric; they are {:?}",
        TARGETS.iter().map(|t| t.name).collect::<Vec<_>>()
    ))?;
    Ok(Verdict {
        name: name.to_owned(),
        seconds,
        budget_s: t.budget_s,
        target_s: t.target_s,
        within_budget: seconds <= t.budget_s,
        within_target: seconds <= t.target_s,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budgets_follow_the_roadmap() {
        assert_eq!(target("plan_s").unwrap().budget_s, 30.0);
        assert_eq!(target("first_link_s").unwrap().budget_s, 120.0);
        assert!(judge("tweak_s", 0.8).unwrap().within_target);
        let v = judge("first_link_s", 90.0).unwrap();
        assert!(v.within_budget && !v.within_target);
        assert!(!judge("plan_s", 31.0).unwrap().within_budget);
        assert!(judge("nope", 1.0).is_err());
    }
}
