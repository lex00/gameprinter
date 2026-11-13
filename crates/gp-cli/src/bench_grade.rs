//! The benches' grader: the only module that reads a game's answer key
//! (docs/plan-extraction-process.md, "Guards that keep the work honest",
//! "One grader"). The answer key is what the bench file's `[grade]` names:
//! the game's hand-written model and its state mapping (`model`, a package
//! folder under the bench's root), and, for a game with its source, the
//! source the evidence cites (`source`), where an uncaught mutant is placed.
//! A tool never reads any of it; `gp bench` grades what the tools produced
//! here, after they ran.
//!
//! Ported from game-printer. Its grader also graded `gp clips matrix` and
//! `gp capture grid --code` against a ground truth written from Moss Hop's
//! source (routines, their reads and the branch ways never taken) and RAM
//! labels. gameprinter has neither command nor Moss Hop, and Donkey Kong
//! and Tapper have no source, so those grades aren't here: the bench grades
//! the corpus's code coverage by `gp corpus reduce` instead (bench_clips.rs).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::Deserialize;
use serde_json::{Value, json};
use toml::Value as Toml;

#[derive(Deserialize)]
struct Grade {
    /// The real model and its state mapping: the answer key for the
    /// facts. A package folder.
    model: PathBuf,
    /// The source the real model's evidence cites, for a game that has it.
    #[serde(default)]
    source: Option<PathBuf>,
}

/// The answer key, loaded from a bench file's `[grade]`.
pub(crate) struct Grader {
    model_dir: PathBuf,
    source: Option<(String, Vec<String>)>,
    files: Vec<PathBuf>,
}

impl Grader {
    pub(crate) fn load(root: &Path, bench_file: &Path) -> Result<Self> {
        #[derive(Deserialize)]
        struct OnlyGrade {
            grade: Grade,
        }
        let g: OnlyGrade = toml::from_str(&fs::read_to_string(bench_file)?)
            .with_context(|| format!("{}: [grade]", bench_file.display()))?;
        let g = g.grade;
        let model_dir = root.join(&g.model);
        let mut files = vec![
            model_dir.join(gp_model::MODEL_FILE),
            model_dir.join(gp_interp::mapping::MAPPING_FILE),
        ];
        let source = match &g.source {
            Some(s) => {
                files.push(root.join(s));
                let text = fs::read_to_string(root.join(s))
                    .with_context(|| format!("the source {}", s.display()))?;
                Some((
                    s.display().to_string(),
                    text.lines().map(str::to_owned).collect(),
                ))
            }
            None => None,
        };
        Ok(Self {
            model_dir,
            source,
            files,
        })
    }

    /// The answer key's files, which no stage tool may read.
    pub(crate) fn files(&self) -> &[PathBuf] {
        &self.files
    }

    /// The real model's folder.
    pub(crate) fn model_dir(&self) -> &Path {
        &self.model_dir
    }

    /// The real model and its state mapping.
    pub(crate) fn answer(&self) -> Result<(Toml, Option<Toml>)> {
        let model: Toml = toml::from_str(&fs::read_to_string(
            self.model_dir.join(gp_model::MODEL_FILE),
        )?)?;
        let mapping = read_toml(&self.model_dir.join(gp_interp::mapping::MAPPING_FILE));
        Ok((model, mapping))
    }

    /// Where a model path's evidence points: the first ref's `at` of the
    /// path or its nearest parent with evidence, and the line's label in
    /// the source when the bench has it.
    pub(crate) fn lives_in(&self, path: &str, real: &Toml) -> String {
        let plain: String = {
            let mut out = String::new();
            let mut depth = 0;
            for c in path.chars() {
                match c {
                    '[' => depth += 1,
                    ']' => depth -= 1,
                    c if depth == 0 => out.push(c),
                    _ => {}
                }
            }
            out
        };
        let Some(ev) = real.get("evidence").and_then(Toml::as_table) else {
            return "no evidence".into();
        };
        let mut key = plain.as_str();
        let at = loop {
            if let Some(a) = ev
                .get(key)
                .and_then(|e| e.get("refs"))
                .and_then(Toml::as_array)
                .into_iter()
                .flatten()
                .find_map(|r| r.get("at").and_then(Toml::as_str))
            {
                break Some(a.to_owned());
            }
            match key.rsplit_once('.') {
                Some((k, _)) => key = k,
                None => break None,
            }
        };
        let Some(at) = at else {
            return "no evidence ref".into();
        };
        let Some((name, lines)) = &self.source else {
            return at;
        };
        let line = at
            .strip_prefix(name.as_str())
            .and_then(|x| x.strip_prefix(':'))
            .and_then(|n| n.parse::<usize>().ok());
        let Some(line) = line else { return at };
        // The nearest label at or above the line.
        let label = lines[..line.min(lines.len())].iter().rev().find_map(|l| {
            let w: String = l
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            (!w.is_empty() && l[w.len()..].starts_with(':')).then_some(w)
        });
        match label {
            Some(l) => format!("{l}, line {line}"),
            None => format!("line {line}, before any label"),
        }
    }
}

pub(crate) fn jaccard<T: Ord>(a: &BTreeSet<T>, b: &BTreeSet<T>) -> f64 {
    let u = a.union(b).count();
    if u == 0 {
        1.0
    } else {
        a.intersection(b).count() as f64 / u as f64
    }
}

pub(crate) fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// Leaves that are documentation, not facts.
const DOC_KEYS: &[&str] = &["description", "note", "title"];

/// Every leaf under `v`, by its path with list positions: a scalar, or a
/// list of scalars as one value.
pub(crate) fn leaves(v: &Toml, at: &str, out: &mut BTreeMap<String, Toml>) {
    match v {
        Toml::Table(t) => {
            for (k, x) in t {
                if DOC_KEYS.contains(&k.as_str()) || (at.is_empty() && k == "evidence") {
                    continue;
                }
                let p = if at.is_empty() {
                    k.clone()
                } else {
                    format!("{at}.{k}")
                };
                leaves(x, &p, out);
            }
        }
        Toml::Array(a) if a.iter().any(Toml::is_table) => {
            for (i, x) in a.iter().enumerate() {
                leaves(x, &format!("{at}[{i}]"), out);
            }
        }
        x => {
            out.insert(at.to_owned(), x.clone());
        }
    }
}

/// A model's and a mapping's facts, the mapping's under `mapping.`.
pub(crate) fn facts_of(model: &Toml, mapping: Option<&Toml>) -> BTreeMap<String, Toml> {
    let mut out = BTreeMap::new();
    leaves(model, "", &mut out);
    if let Some(m) = mapping {
        let mut mm = BTreeMap::new();
        leaves(m, "", &mut mm);
        for (k, v) in mm {
            out.insert(format!("mapping.{k}"), v);
        }
    }
    out
}

/// A number as a value to compare: `"0.25"` and `0.25` are the same.
pub(crate) fn same(a: &Toml, b: &Toml) -> bool {
    let num = |v: &Toml| match v {
        Toml::Integer(i) => Some(*i as f64),
        Toml::Float(f) => Some(*f),
        Toml::String(s) => s.parse().ok(),
        _ => None,
    };
    match (num(a), num(b)) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

/// Each real fact in `have`: right, wrong or missing; and the facts
/// `have` states that the real model doesn't.
pub(crate) fn grade(real: &BTreeMap<String, Toml>, have: &BTreeMap<String, Toml>) -> Value {
    let (mut right, mut wrong, mut missing) = (Vec::new(), Vec::new(), 0);
    for (p, v) in real {
        match have.get(p) {
            None => missing += 1,
            Some(h) if same(h, v) => right.push(p.clone()),
            Some(_) => wrong.push(p.clone()),
        }
    }
    let extra = have.keys().filter(|p| !real.contains_key(*p)).count();
    json!({"total": real.len(), "right": right.len(), "wrong": wrong.len(), "missing": missing,
        "extra": extra, "right_paths": right, "wrong_paths": wrong})
}

/// The elements a model names: its entity kinds, tile kinds and counters.
pub(crate) fn elements(model: &Toml) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (sec, prefix) in [("entities", "entity"), ("counters", "counter")] {
        if let Some(t) = model.get(sec).and_then(Toml::as_table) {
            out.extend(t.keys().map(|k| format!("{prefix}:{k}")));
        }
    }
    for l in model
        .get("levels")
        .and_then(Toml::as_table)
        .into_iter()
        .flat_map(|t| t.values())
    {
        if let Some(k) = l.get("tile_kinds").and_then(Toml::as_table) {
            out.extend(
                k.values()
                    .filter_map(Toml::as_str)
                    .map(|v| format!("tile:{v}")),
            );
        }
    }
    out
}

pub(crate) fn read_toml(p: &Path) -> Option<Toml> {
    toml::from_str(&fs::read_to_string(p).ok()?).ok()
}

/// What a model folder holds, graded: its facts against the real ones,
/// and its elements.
pub(crate) fn grade_dir(
    dir: &Path,
    real: &BTreeMap<String, Toml>,
    real_elements: &BTreeSet<String>,
) -> Value {
    let model =
        read_toml(&dir.join(gp_model::MODEL_FILE)).unwrap_or(Toml::Table(Default::default()));
    let mapping = read_toml(&dir.join(gp_interp::mapping::MAPPING_FILE));
    let have = facts_of(&model, mapping.as_ref());
    let els = elements(&model);
    let found = els.intersection(real_elements).count();
    json!({
        "facts": grade(real, &have),
        "elements": {"real": real_elements.len(), "named": els.len(), "found": found,
            "recall": round3(found as f64 / real_elements.len().max(1) as f64),
            "precision": if els.is_empty() { Value::Null } else { json!(round3(found as f64 / els.len() as f64)) }},
        "mapping": mapping.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n != "target") {
                    rust_files(&p, out);
                }
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }

    /// One grader (docs/plan-extraction-process.md, "Guards"): only the
    /// graders read an answer key, and the from-scratch bench reads the real
    /// model only through this one.
    #[test]
    fn only_the_grader_reads_the_answer_key() {
        let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut files = Vec::new();
        rust_files(&crates, &mut files);
        let graders = ["bench_grade.rs", "static_grade.rs", "rom_grade.rs"];
        let mut bad = Vec::new();
        for f in &files {
            let name = f.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            if graders.contains(&name) && f.parent().is_some_and(|p| p.ends_with("gp-cli/src")) {
                continue;
            }
            let t = fs::read_to_string(f).unwrap();
            for marker in ["struct Grade ", "games/keys"] {
                if t.contains(marker) {
                    bad.push(format!("{} names {marker}", f.display()));
                }
            }
            if f.ends_with("bench_scratch.rs") {
                for marker in ["bench.template", "MAPPING_FILE)?"] {
                    if t.contains(marker) {
                        bad.push(format!("{} names {marker}", f.display()));
                    }
                }
            }
        }
        assert!(bad.is_empty(), "{bad:#?}");
    }

    /// A made-up model: the oracle tool scores every fact; the null tool
    /// none; a wrong value is wrong, and a number written as text counts as
    /// the number.
    fn made_up() -> Toml {
        toml::from_str(
            r#"
[model]
schema = "0.1"
name = "made-up"
title = "Made up"
frame_rate = 60

[entities.hero]
speed = 2
jump = { height = 40, gravity = "0.25" }

[counters.lives]
start = 3
"#,
        )
        .unwrap()
    }

    #[test]
    fn the_oracle_scores_all_the_null_none_and_a_change_is_wrong() {
        let m = made_up();
        let real = facts_of(&m, None);
        let all = grade(&real, &real);
        assert_eq!(all["right"].as_u64(), Some(real.len() as u64));
        let none = grade(&real, &BTreeMap::new());
        assert_eq!(
            (none["right"].as_u64(), none["missing"].as_u64()),
            (Some(0), Some(real.len() as u64))
        );
        let mut changed = real.clone();
        changed.insert("entities.hero.speed".into(), Toml::Integer(3));
        let g = grade(&real, &changed);
        assert_eq!(
            (g["wrong"].as_u64(), g["right"].as_u64()),
            (Some(1), Some(real.len() as u64 - 1))
        );
        assert!(same(&Toml::String("0.25".into()), &Toml::Float(0.25)));
        assert!(!same(
            &Toml::String("walk".into()),
            &Toml::String("run".into())
        ));
        assert_eq!(
            elements(&m),
            ["counter:lives", "entity:hero"]
                .iter()
                .map(|s| (*s).to_owned())
                .collect()
        );
    }

    #[test]
    fn the_start_holds_only_the_header_and_reads() {
        let r = made_up();
        let text = crate::bench_scratch::start_model(&r).unwrap();
        let v: Toml = toml::from_str(&text).unwrap();
        let keys: Vec<&String> = v.as_table().unwrap().keys().collect();
        assert_eq!(keys, ["model"]);
        assert!(elements(&v).is_empty());
        let g = grade(&facts_of(&r, None), &facts_of(&v, None));
        assert!(g["right"].as_u64().unwrap() > 0);
        assert_eq!(g["wrong"].as_u64(), Some(0));
    }
}
