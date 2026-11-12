//! `gp interp scenarios --refine` and `--auto`: the refinement loop on one
//! scenario of the ladder, or on each open one in turn (game-printer's
//! scenario.rs `refine_one` and `auto`). The loop is `gp model refine
//! --lockstep`'s ([`crate::refine::lockstep::run_scoped`]); a fit is
//! written only if no other scenario gets worse, and the solvers
//! ([`crate::solve`]) read the scenario's own score.
//!
//! What gameprinter's layout changed: the working copy is refused inside a
//! git checkout (as gp model refine refuses it), where game-printer refused
//! a template in its repo; each model a fit tries is scored through the
//! bench's module, edited (`Source::edited`); the original's random draws
//! for the loop are recorded here (game-printer's loop recorded them
//! itself); and there is no run recorder or stop flag.

use std::collections::BTreeMap;

use anyhow::{Result, bail};
use gp_interp::scenario::{self, Bar, Outcome as Ran, Scenario, State};
use gp_model::Model;
use serde_json::{Value as Json, json};

use crate::refine::lockstep;
use crate::scenario::{Bench, Started, ladder_json, record_history, scenarios, score};

/// How one model's results compare with another's, scenario by scenario.
pub(crate) struct Change {
    pub fixed: Vec<String>,
    pub better: Vec<String>,
    pub worse: Vec<String>,
}

pub(crate) fn change(
    list: &[Scenario],
    before: &[Ran],
    after: &[Ran],
    bar: Bar,
    except: &str,
) -> Change {
    let mut c = Change {
        fixed: Vec::new(),
        better: Vec::new(),
        worse: Vec::new(),
    };
    for ((s, b), a) in list.iter().zip(before).zip(after) {
        if s.id == except {
            continue;
        }
        if a.score > b.score || (bar.passes(b) && !bar.passes(a)) {
            c.worse.push(s.name.clone());
        } else if !bar.passes(b) && bar.passes(a) {
            c.fixed.push(s.name.clone());
        } else if a.score < b.score {
            c.better.push(s.name.clone());
        }
    }
    c
}

fn change_says(c: &Change) -> String {
    let n = c.fixed.len();
    let fixes = match n {
        0 => "fixes no other scenario".to_owned(),
        1 => "also fixes 1 other scenario".to_owned(),
        _ => format!("also fixes {n} other scenarios"),
    };
    let worse = match c.worse.len() {
        0 => "none worse".to_owned(),
        k => format!("{k} worse: {}", c.worse.join("; ")),
    };
    format!("{fixes}, {worse}")
}

/// What a refine on one scenario needs besides the bench.
pub(crate) struct RefineArgs<'a> {
    pub at: Option<u64>,
    pub tier: Option<u8>,
    pub played: &'a [Scenario],
    pub bar: Bar,
    /// The list is `played` alone (the background queue's pool of clips),
    /// not the generated ladder, and needs no start of the bench's own.
    pub pool_only: bool,
}

impl RefineArgs<'_> {
    fn list(&self, model: &Model, exts: &gp_sim::ext::Exts) -> Vec<Scenario> {
        if self.pool_only {
            self.played.to_vec()
        } else {
            scenarios(model, exts, self.played, self.tier)
        }
    }

    fn started(&self, b: &Bench, model: &Model, text: &str) -> Result<Started> {
        if self.pool_only {
            Ok(Started::none())
        } else {
            b.started(model, text, self.at)
        }
    }
}

/// The refinement loop on one scenario: its input is the input to the
/// start and then the scenario's presses, every run starts from the
/// original's state at the start, only the scenario's mechanic's numbers
/// change, and a fit is written only if no other scenario gets worse.
/// Afterwards every scenario is scored again and the history gets the new
/// model.
pub(crate) fn refine_one(
    b: &Bench,
    a: &RefineArgs<'_>,
    id: &str,
    progress: &mut dyn FnMut(&Json),
) -> Result<Json> {
    crate::refine::refuse_checkout(&b.dir)?;
    let (model, text) = b.model()?;
    let started = a.started(b, &model, &text)?;
    let list = a.list(&model, &b.hosted.exts);
    let Some(target) = list.iter().find(|s| s.id == id).cloned() else {
        bail!("the ladder has no scenario `{id}`");
    };
    let before = score(b, &model, &text, &started, &list, true, &mut |_, _, _| {})?;
    record_history(b, &started, &text, "before refine", &before);
    let own_input = b.for_start(target.start.as_ref())?;
    let tb = own_input.as_ref().unwrap_or(b);
    let st = match &target.start {
        Some(own) if own_input.is_some() || own.frame != started.original_frame => {
            tb.started_at(&model, &text, own)?
        }
        _ => started.clone(),
    };
    let s = usize::try_from(st.original_frame)
        .unwrap_or(usize::MAX)
        .min(tb.inputs.len());
    let mut inputs = tb.inputs[..s].to_vec();
    inputs.extend(target.inputs().map_err(anyhow::Error::msg)?);
    let others: Vec<Scenario> = list.iter().filter(|s| s.id != target.id).cloned().collect();
    let others_before: Vec<Ran> = list
        .iter()
        .zip(&before)
        .filter(|(s, _)| s.id != target.id)
        .map(|(_, r)| r.clone())
        .collect();
    let bar = a.bar;
    let accept = |m: &Model| -> Result<Option<String>> {
        let t = toml::to_string(&m.to_tree())?;
        let after = score(
            &on(b, m)?,
            m,
            &t,
            &started,
            &others,
            false,
            &mut |_, _, _| {},
        )?;
        let c = change(&others, &others_before, &after, bar, "");
        Ok((!c.worse.is_empty()).then(|| format!("makes worse: {}", c.worse.join("; "))))
    };
    // The solvers read the target's own score on the model as it stands
    // (crate::solve): its two series per measure, run free.
    let solve = |m: &Model| -> Result<Vec<crate::solve::Proposal>> {
        let t = toml::to_string(&m.to_tree())?;
        let r = score(
            &on(b, m)?,
            m,
            &t,
            &started,
            std::slice::from_ref(&target),
            true,
            &mut |_, _, _| {},
        )?;
        let catalog = gp_interp::mechanics::catalog(m, &b.hosted.exts);
        Ok(r.first()
            .map(|o| crate::solve::solve(o, &target.name, m, &catalog).proposals)
            .unwrap_or_default())
    };
    let scope = lockstep::Scope {
        start_at: Some(st.original_frame),
        mechanics: if target.mechanic.is_empty() {
            Vec::new()
        } else {
            vec![target.mechanic.clone()]
        },
        accept: Some(&accept),
        solve: Some(&solve),
        ..Default::default()
    };
    let source = format!(
        "scenario {} ({}) from {} at frame {}",
        target.id, target.name, b.input, st.original_frame
    );
    // The original's random draws from the start on, with the mapping's
    // `score_draws`, as a clip's refine takes them.
    let draws = if b.mapping.score_draws {
        let frames: Vec<gp_evidence::input_log::InputFrame> = inputs
            .iter()
            .map(|f| gp_evidence::input_log::InputFrame {
                polled: false,
                pads: f.buttons.to_le_bytes(),
                reset: f.reset,
            })
            .collect();
        Some(crate::lockstep::record_draws(
            &b.rom,
            &b.mapping,
            &frames,
            st.original_frame,
        )?)
    } else {
        None
    };
    let args = lockstep::LockstepArgs {
        dir: &b.dir,
        rom: &b.rom,
        inputs: &inputs,
        source: &source,
        model: &b.hosted,
        resume: None,
        draws,
        rounds: lockstep::DEFAULT_ROUNDS,
        window: target.frames + 1,
        from: Some(st.original_frame),
    };
    let mut round = 0u32;
    let out = lockstep::run_scoped(&args, &scope, &mut |p: &Json| {
        if p.get("round").is_some_and(Json::is_object) {
            round += 1;
            // Each round changes the model: score the ladder again for
            // the history.
            if let Ok((m, t)) = b.model()
                && let Ok(bm) = on(b, &m)
                && let Ok(r) = score(&bm, &m, &t, &started, &list, true, &mut |_, _, _| {})
            {
                let r0 = &p["round"];
                record_history(
                    b,
                    &started,
                    &t,
                    &format!(
                        "refine round {round} on {}: {} {} -> {}",
                        target.name,
                        r0["param"].as_str().unwrap_or(""),
                        r0["old"].as_str().unwrap_or(""),
                        r0["new"].as_str().unwrap_or("")
                    ),
                    &r,
                );
            }
        }
        progress(p);
    })?;
    let (model2, text2) = b.model()?;
    let after = score(
        &on(b, &model2)?,
        &model2,
        &text2,
        &started,
        &list,
        true,
        &mut |_, _, _| {},
    )?;
    let history = record_history(
        b,
        &started,
        &text2,
        &format!("after refine on {}", target.name),
        &after,
    );
    let c = change(&list, &before, &after, a.bar, &target.id);
    let i = list.iter().position(|s| s.id == target.id).unwrap_or(0);
    let passes = a.bar.passes(&after[i]);
    let changed = text2 != text;
    let says = if passes {
        format!("{} now passes; {}.", target.name, change_says(&c))
    } else if changed {
        format!(
            "{} is closer ({} -> {}) but doesn't pass yet; {}.",
            target.name,
            before[i].headline.says,
            after[i].headline.says,
            change_says(&c)
        )
    } else {
        format!("No numbers make {} match: a question for you.", target.name)
    };
    Ok(json!({
        "scenario": target.id,
        "name": target.name,
        "says": says,
        "passes": passes,
        "changed": changed,
        "before": before[i],
        "after": after[i],
        "also_fixed": c.fixed,
        "better": c.better,
        "worse": c.worse,
        "refine": out.json,
        "ladder": ladder_json(&model2, &started, &list, &after, a.bar, &[], &history, &b.input),
    }))
}

/// Works through the open scenarios on its own: the next open or failing
/// one is refined (as [`refine_one`]), everything is scored again, and it
/// moves on when it passes; it stops with the question when a scenario is
/// stuck, or when every open one passes.
pub(crate) fn auto(b: &Bench, a: &RefineArgs<'_>, progress: &mut dyn FnMut(&Json)) -> Result<Json> {
    let mut steps = Vec::new();
    let mut tried: BTreeMap<String, u32> = BTreeMap::new();
    loop {
        let (model, text) = b.model()?;
        let started = a.started(b, &model, &text)?;
        let list = a.list(&model, &b.hosted.exts);
        let results = score(
            &on(b, &model)?,
            &model,
            &text,
            &started,
            &list,
            true,
            &mut |_, _, _| {},
        )?;
        let history = record_history(b, &started, &text, "auto", &results);
        let ladder = ladder_json(
            &model,
            &started,
            &list,
            &results,
            a.bar,
            &[],
            &history,
            &b.input,
        );
        let refs: Vec<Option<&Ran>> = results.iter().map(Some).collect();
        let states = scenario::states(&list, &refs, a.bar, &[]);
        let passed = states.iter().filter(|s| **s == State::Passed).count();
        let locked = states.iter().filter(|s| **s == State::Locked).count();
        let open = states
            .iter()
            .filter(|s| matches!(s, State::Open | State::Failing))
            .count();
        let Some(i) = scenario::next(&states) else {
            let says = format!("Every open sequence passes: passed {passed}; locked {locked}.");
            return Ok(json!({"state": "done", "says": says, "steps": steps, "ladder": ladder}));
        };
        let s = &list[i];
        let n = tried.entry(s.id.clone()).or_default();
        *n += 1;
        if *n > 2 {
            let says = format!(
                "Stuck on {}: two refines didn't make it pass. Next: answer its question.",
                s.name
            );
            return Ok(
                json!({"state": "stuck", "says": says, "scenario": s.id, "steps": steps, "ladder": ladder}),
            );
        }
        let working = format!(
            "Working on: {} ({} of {open} open); passed {passed}; locked {locked}",
            s.name,
            states[..=i]
                .iter()
                .filter(|s| matches!(s, State::Open | State::Failing))
                .count()
        );
        progress(
            &json!({"working": {"scenario": s.id, "name": s.name, "says": working, "passed": passed, "locked": locked, "open": open}}),
        );
        let r = refine_one(b, a, &s.id.clone(), progress)?;
        let stuck = r["refine"]["stuck"] == true;
        let passes = r["passes"] == true;
        steps.push(json!({"scenario": r["scenario"], "says": r["says"], "passes": passes}));
        progress(&json!({"step": steps.last()}));
        if !passes && (stuck || r["changed"] == false) {
            let q = r["refine"]["questions"][0]["text"]
                .as_str()
                .unwrap_or("")
                .to_owned();
            let says = format!(
                "Stuck on {}: no numbers make it match. Next: answer its question{}",
                s.name,
                if q.is_empty() {
                    ".".to_owned()
                } else {
                    format!(": {q}")
                }
            );
            return Ok(
                json!({"state": "stuck", "says": says, "scenario": r["scenario"], "question": r["refine"]["questions"][0], "steps": steps, "ladder": r["ladder"]}),
            );
        }
    }
}

/// The bench with its model edited to `m`, run through the same module.
fn on(b: &Bench, m: &Model) -> Result<Bench> {
    let mut x = b.clone();
    x.hosted = std::sync::Arc::new(b.hosted.edited(m)?);
    Ok(x)
}
