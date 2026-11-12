//! `gp model propose` and the recording-driven `gp model refine` (without
//! `--lockstep`): agent extraction's measurement and refinement loop over
//! store recordings (docs/mechanics.md). The measuring is gp-extract's
//! `feel` and `propose`, over `gp track`'s tracks; between rounds the loop
//! runs `gp validate`.
//!
//! Ported from game-printer's refine.rs. The run recorder's metrics
//! (`--metrics`) are gone with the recorder. Recordings are read through
//! `gp track`, which reads an NES recording's sprite table, and refuses an
//! arcade recording in words until gp-arcade's sprite adapter lands (wave
//! 6.5).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use gp_extract::apply::{Fact, apply, at};
use gp_extract::propose::{Proposal, Proposed};
use gp_extract::questions::{Candidate, QUESTIONS_FILE, Question, Questions};
use gp_interp::mapping::{MAPPING_FILE, Mapping};
use gp_model::{MODEL_FILE, Model};

use super::{answer_refs, read_model, read_questions};
use crate::Outcome;

/// The most rounds `gp model refine` runs by default without `--lockstep`.
pub const DEFAULT_ROUNDS: u32 = 4;

fn write_questions(dir: &Path, q: &Questions) -> Result<()> {
    let file = dir.join(QUESTIONS_FILE);
    fs::write(&file, q.to_toml()).with_context(|| format!("writing {}", file.display()))
}

/// The entity of each object ID, from the mapping's slot kinds.
fn ids_of(dir: &Path) -> BTreeMap<u8, String> {
    let Ok(text) = fs::read_to_string(dir.join(MAPPING_FILE)) else {
        return BTreeMap::new();
    };
    let Ok(m) = Mapping::parse(&text) else {
        return BTreeMap::new();
    };
    let mut out = BTreeMap::new();
    if let Some(s) = &m.slots {
        for k in &s.kinds {
            for id in &k.ids {
                out.insert(*id, k.entity.clone());
            }
        }
    }
    out
}

/// Every recording's proposals, merged by path: the same measured value
/// from each is one proposal with their samples added; different values
/// are a question.
pub(crate) struct Merged {
    pub per_recording: Vec<Proposed>,
    pub proposals: Vec<Proposal>,
    pub questions: Vec<Question>,
}

pub(crate) fn measure(
    dir: &Path,
    model: &Model,
    recordings: &[String],
    frames: &str,
    paths: &gp_evidence::Paths,
) -> Result<Merged> {
    let player = gp_extract::propose::player_of(model).unwrap_or_default();
    let pad = gp_extract::feel::Pad::of(model, player);
    let jump = model
        .entities
        .get(player)
        .and_then(|e| e.platformer.jump.as_ref());
    let move_first = jump.is_some_and(|j| j.move_first);
    let held = jump.is_some_and(|j| {
        j.gravity_held.is_some() || j.bands.iter().any(|b| b.gravity_held.is_some())
    });
    let ids = ids_of(dir);
    let mapping = fs::read_to_string(dir.join(MAPPING_FILE))
        .ok()
        .and_then(|t| Mapping::parse(&t).ok());
    let slots = mapping.as_ref().and_then(crate::track::SlotIds::of);
    let player_at = mapping
        .as_ref()
        .and_then(|m| crate::track::PlayerAt::of(m, player));
    let mut per = Vec::new();
    for id in recordings {
        let mut t = crate::track::tracked(paths, id, frames, 1)?;
        // The mapping's slots name each enemy track's kind where the
        // tracker's own slot search found no ID byte.
        if let Some(s) = &slots {
            crate::track::ids_from_slots(&mut t, s)?;
        }
        // The mapping's player fields stand in for a player the sprite
        // tracker didn't find.
        if let Some(p) = &player_at {
            crate::track::player_from_mapping(&mut t, p)?;
        }
        let feel = gp_extract::feel::measure(
            &t.report.tracks,
            t.range.start,
            &t.pads,
            pad,
            move_first,
            held,
        );
        per.push(gp_extract::propose::propose(
            model,
            &feel,
            &t.opened.entry.id.to_hex()[..12],
            &ids,
        ));
    }
    let mut by: BTreeMap<String, Vec<Proposal>> = BTreeMap::new();
    let mut order = Vec::new();
    let mut questions: Vec<Question> = Vec::new();
    for p in &per {
        for q in &p.questions {
            if !questions.iter().any(|o| o.id == q.id) {
                questions.push(q.clone());
            }
        }
        for x in &p.proposals {
            if !by.contains_key(&x.path) {
                order.push(x.path.clone());
            }
            by.entry(x.path.clone()).or_default().push(x.clone());
        }
    }
    let mut proposals = Vec::new();
    for path in order {
        let mut list = by.remove(&path).expect("added");
        let first = list.remove(0);
        if list.iter().all(|o| o.measured == first.measured) {
            let mut p = first;
            for o in &list {
                p.samples += o.samples;
                p.settled &= o.settled;
                p.recordings.extend(o.recordings.iter().cloned());
            }
            proposals.push(p);
        } else {
            let mut values: Vec<Candidate> = vec![Candidate {
                value: first.measured.clone(),
                from: first.note.clone(),
                refs: first
                    .recordings
                    .iter()
                    .map(|r| at(format!("recording {r}")))
                    .collect(),
            }];
            for o in &list {
                if !values.iter().any(|c| c.value == o.measured) {
                    values.push(Candidate {
                        value: o.measured.clone(),
                        from: o.note.clone(),
                        refs: o
                            .recordings
                            .iter()
                            .map(|r| at(format!("recording {r}")))
                            .collect(),
                    });
                }
            }
            questions.push(Question {
                id: format!("recordings:{path}"),
                kind: "conflict".into(),
                path: path.clone(),
                text: format!("Recordings measure {path} differently. Which is the mechanic's?"),
                settle: "a recording made to exercise this mechanic alone".into(),
                candidates: values,
                answer: None,
            });
            let mut p = first;
            p.settled = false;
            proposals.push(p);
        }
    }
    Ok(Merged {
        per_recording: per,
        proposals,
        questions,
    })
}

fn proposal_line(p: &Proposal) -> String {
    let state = match (p.agrees, &p.stated) {
        (Some(true), _) => "agrees".to_owned(),
        (Some(false), Some(s)) => format!("differs from {s}"),
        _ => "new".to_owned(),
    };
    format!(
        "  {:<44} {:>10} {:<11} {} samples, {}{}",
        p.path,
        p.measured,
        p.unit,
        p.samples,
        state,
        if p.settled { "" } else { ", asked" }
    )
}

fn accuracy(per: &[Proposed]) -> (usize, usize, f64) {
    let (mut c, mut a, mut e) = (0, 0, 0.0);
    for p in per {
        c += p.accuracy.compared;
        a += p.accuracy.agree;
        e += p.accuracy.mean_relative_error * p.accuracy.compared as f64;
    }
    (c, a, if c > 0 { e / c as f64 } else { 0.0 })
}

/// `gp model propose`.
pub(crate) fn propose(
    dir: &Path,
    recordings: &[String],
    frames: &str,
    write: bool,
    paths: &gp_evidence::Paths,
) -> Result<Outcome> {
    let (_, model) = read_model(dir)?;
    let m = measure(dir, &model, recordings, frames, paths)?;
    let (compared, agree, err) = accuracy(&m.per_recording);
    let mut asked = 0;
    if write {
        let mut qs = read_questions(dir)?;
        for q in &m.questions {
            asked += usize::from(qs.ask(q.clone()));
        }
        write_questions(dir, &qs)?;
    }
    let mut t = format!(
        "{}: {} proposals from {} recordings; against the model's facts {agree} of {compared} agree (mean relative error {:.3})\n",
        model.model.name,
        m.proposals.len(),
        recordings.len(),
        err
    );
    for p in &m.proposals {
        t.push_str(&proposal_line(p));
        t.push('\n');
    }
    for q in &m.questions {
        let _ = writeln!(t, "question {}: {}", q.id, q.text);
    }
    if write {
        let _ = writeln!(
            t,
            "{asked} new questions in {}",
            dir.join(QUESTIONS_FILE).display()
        );
    }
    Ok(Outcome::ok(
        json!({
            "model": model.model.name,
            "proposals": m.proposals,
            "questions": m.questions,
            "recordings": m.per_recording,
            "accuracy": { "compared": compared, "agree": agree, "mean_relative_error": err },
        }),
        t.trim_end().to_owned(),
    ))
}

/// What `gp model refine` validates against, and with what.
pub(crate) struct RefineArgs<'a> {
    pub dir: &'a Path,
    pub recordings: &'a [String],
    pub against: &'a [String],
    pub rom: Option<&'a PathBuf>,
    pub assets: Option<&'a PathBuf>,
    pub module: Option<&'a PathBuf>,
    pub rounds: u32,
    pub frames: &'a str,
}

fn validate(a: &RefineArgs<'_>, paths: &gp_evidence::Paths) -> Result<Value> {
    let against = if a.against.is_empty() {
        a.recordings.to_vec()
    } else {
        a.against.to_vec()
    };
    let args = crate::validate::ValidateArgs {
        model: a.dir.to_path_buf(),
        against,
        store: false,
        runtime: crate::validate::RuntimeArg::Interp,
        module: a.module.cloned(),
        assets: a.assets.cloned(),
        rom: a.rom.cloned(),
        mapping: None,
        no_save: true,
        // The loop proposes facts from what the model does on its own,
        // its start values too.
        power_on: true,
    };
    let out = crate::validate::run(&args, paths)?;
    if out.exit == 2 {
        bail!("gp validate couldn't run: {}", out.text);
    }
    Ok(out.json)
}

/// The failing checks' parts.
fn failing(v: &Value) -> Vec<String> {
    v["checks"]
        .as_array()
        .map(|c| {
            c.iter()
                .filter(|c| c["status"] == "fail")
                .filter_map(|c| c["part"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// `gp model refine`: propose, validate, question, apply, until the
/// checks pass with nothing left to apply, or only questions remain, or
/// `rounds` rounds have run. A measured fact that disagrees with the
/// model's is written when a failing check measures it (the recording and
/// validation agree the model is wrong); otherwise it's asked. Answered
/// questions are written as stated facts.
pub(crate) fn refine(a: &RefineArgs<'_>, paths: &gp_evidence::Paths) -> Result<Outcome> {
    if a.recordings.is_empty() {
        bail!("gp model refine needs at least one --recording to measure");
    }
    let mut log: Vec<Value> = Vec::new();
    let mut applied_all: Vec<String> = Vec::new();
    let mut text = String::new();
    let mut rounds = 0;
    let (mut last, mut open);
    loop {
        rounds += 1;
        let v = validate(a, paths)?;
        let fails = failing(&v);
        let (_, model) = read_model(a.dir)?;
        let m = measure(a.dir, &model, a.recordings, a.frames, paths)?;
        let mut qs = read_questions(a.dir)?;
        let mut facts = Vec::new();
        // Answered questions first, then measurements a failing check backs.
        for q in qs
            .questions
            .iter()
            .filter(|q| !matches!(q.kind.as_str(), "which-entity" | "shape"))
        {
            if let Some(n) = q.answer {
                let value = &q.candidates[n].value;
                let stated = m
                    .proposals
                    .iter()
                    .find(|p| p.path == q.path)
                    .and_then(|p| p.stated.clone());
                if stated.as_deref() != Some(value.as_str()) {
                    facts.push(Fact {
                        path: q.path.clone(),
                        value: value.clone(),
                        source: "stated",
                        confidence: 100,
                        note: format!("Answered by the person: {}", q.text),
                        refs: answer_refs(&model, q, n),
                    });
                }
            }
        }
        let mut asked = 0;
        for p in &m.proposals {
            if facts.iter().any(|f| f.path == p.path) || qs.answered(&p.path).is_some() {
                continue;
            }
            let backed = fails
                .iter()
                .any(|f| p.path == *f || p.path.starts_with(&format!("{f}.")));
            if p.agrees == Some(false) && backed && p.samples >= 1 {
                facts.push(Fact {
                    path: p.path.clone(),
                    value: p.measured.clone(),
                    source: "observed",
                    confidence: p.confidence,
                    note: format!(
                        "{}; written by gp model refine because validation failed on {}",
                        p.note,
                        fails
                            .iter()
                            .find(|f| p.path.starts_with(f.as_str()))
                            .cloned()
                            .unwrap_or_default()
                    ),
                    refs: p
                        .recordings
                        .iter()
                        .map(|r| at(format!("recording {r}")))
                        .collect(),
                });
            }
        }
        for q in &m.questions {
            // A disagreement a failing check backs is written, not asked.
            if q.kind == "disagree" && facts.iter().any(|f| f.path == q.path) {
                continue;
            }
            asked += usize::from(qs.ask(q.clone()));
        }
        write_questions(a.dir, &qs)?;
        open = qs.open().count();
        let applied: Vec<String> = facts
            .iter()
            .map(|f| format!("{} = {}", f.path, f.value))
            .collect();
        if !facts.is_empty() {
            let (old, _) = read_model(a.dir)?;
            let new = apply(&old, &facts).map_err(anyhow::Error::msg)?;
            fs::write(a.dir.join(MODEL_FILE), new)?;
        }
        let _ = writeln!(
            text,
            "round {rounds}: {} checks pass, {} fail; {} proposals; {} applied{}; {} new questions, {open} open",
            v["passed"],
            v["failed"],
            m.proposals.len(),
            applied.len(),
            if applied.is_empty() {
                String::new()
            } else {
                format!(" ({})", applied.join(", "))
            },
            asked,
        );
        log.push(json!({
            "round": rounds,
            "passed": v["passed"],
            "failed": v["failed"],
            "failing": fails,
            "applied": applied,
            "asked": asked,
            "open": open,
        }));
        applied_all.extend(applied.iter().cloned());
        last = v;
        if facts.is_empty() || rounds >= a.rounds {
            break;
        }
    }
    // A last validation when the final round wrote facts.
    let wrote_last = log
        .last()
        .and_then(|l| l["applied"].as_array())
        .is_some_and(|a| !a.is_empty());
    if wrote_last {
        last = validate(a, paths)?;
        let _ = writeln!(
            text,
            "after round {rounds}: {} checks pass, {} fail",
            last["passed"], last["failed"]
        );
    }
    let ok = last["ok"] == true;
    let stop = if ok && open == 0 {
        "the checks pass and no question is open"
    } else if ok {
        "the checks pass; questions remain for the person (gp model questions)"
    } else if open > 0 {
        "checks fail and questions remain for the person (gp model questions)"
    } else {
        "checks fail and nothing measured explains them"
    };
    let _ = writeln!(text, "{rounds} rounds: {stop}");
    Ok(Outcome {
        json: json!({
            "ok": ok,
            "iterations": rounds,
            "applied": applied_all,
            "open_questions": open,
            "stop": stop,
            "rounds": log,
            "passed": last["passed"],
            "failed": last["failed"],
        }),
        text: text.trim_end().to_owned(),
        exit: u8::from(!ok),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::refine::questions;

    /// Moss Hop's model with its kill line made wrong, and the question
    /// `gp model derive` asks about it, in a scratch folder.
    fn wrong_kill_line() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let text = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/mosshop/model.toml"),
        )
        .unwrap();
        fs::write(
            dir.path().join(MODEL_FILE),
            text.replacen("kill_below = 208\n", "kill_below = 200\n", 1),
        )
        .unwrap();
        let cited = gp_model::model::Ref {
            at: "games/mosshop/mosshop.s:54".into(),
            within: None,
            quote: Some("PIT_Y        = 208".into()),
        };
        let q = Question {
            id: "derive:levels.main.kill_below".into(),
            kind: "disagree".into(),
            path: "levels.main.kill_below".into(),
            text: "Keep the model's or take the derived value?".into(),
            settle: "read the cited line".into(),
            candidates: vec![
                Candidate {
                    value: "200".into(),
                    from: "the model".into(),
                    refs: Vec::new(),
                },
                Candidate {
                    value: "208".into(),
                    from: "gp model derive --from evidence".into(),
                    refs: vec![cited],
                },
            ],
            answer: None,
        };
        write_questions(dir.path(), &Questions { questions: vec![q] }).unwrap();
        dir
    }

    fn kill_line_refs(dir: &Path) -> (toml::Value, Vec<String>) {
        let (_, m) = read_model(dir).unwrap();
        let tree = m.to_tree();
        let v = tree["levels"]["main"]["kill_below"].clone();
        let refs = m.evidence["levels.main.kill_below"]
            .refs
            .iter()
            .map(|r| format!("{}|{}", r.at, r.quote.as_deref().unwrap_or("")))
            .collect();
        (v, refs)
    }

    #[test]
    fn an_answer_keeps_the_source_the_deriver_read() {
        let dir = wrong_kill_line();
        let out = questions(dir.path(), &["derive:levels.main.kill_below=2".into()]).unwrap();
        assert_eq!(out.json["applied"], 1);
        let (v, refs) = kill_line_refs(dir.path());
        assert_eq!(v, toml::Value::Integer(208));
        assert_eq!(
            refs,
            vec![
                "questions.toml derive:levels.main.kill_below|".to_owned(),
                "games/mosshop/mosshop.s:54|PIT_Y        = 208".to_owned(),
            ]
        );
    }

    #[test]
    fn keeping_the_model_s_value_keeps_its_evidence() {
        // The model's candidate carries no refs: the covering entry's are kept.
        let dir = wrong_kill_line();
        questions(dir.path(), &["derive:levels.main.kill_below=1".into()]).unwrap();
        let (v, refs) = kill_line_refs(dir.path());
        assert_eq!(v, toml::Value::Integer(200));
        assert_eq!(refs.len(), 2, "{refs:?}");
        assert!(refs[1].starts_with("games/mosshop/mosshop.s:54|"));
    }
}
