//! `gp author`: a prototype of the authoring API of
//! docs/research/authoring-api.md. A proposer turns a request (a short
//! description and a base template) into an edit of the base model's
//! mechanic parameters; gp's own deterministic checks verify it; failed
//! checks become feedback for the next proposal; and the whole exchange is
//! written as one log row, the shape later training data would take.
//!
//! The proposer here is scripted: a fixed table from request words to
//! edits, so no LLM is called and two runs give the same row. An LLM
//! proposer would implement [`Proposer`] and read the same feedback.
//! Only `--dry-run` is built: nothing is written but the log row.
//!
//! Ported from game-printer's author.rs with its flags, checks and log
//! row. The model's hooks come from gp-exts' registry, as `gp interp`'s
//! did; with no run recorder, the row's gp build and hash are computed
//! here.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Args;
use serde_json::{Value, json};

use gp_evidence::Paths;
use gp_interp::variant::NativeSim;
use gp_interp::{Bundle, Interp, Lifted};
use gp_model::profile::{Group, Profiles, Switch};
use gp_model::{Fixed, Model};
use gp_sim::Exts;

use crate::Outcome;
use crate::corpus_score::build;
use crate::mechanics::model_file;
use crate::static_grade::toml_json;

/// The model's hooks, as game-printer's `interp::hooks_for` gave them.
fn hooks_for(model: &str) -> (std::sync::Arc<Exts>, String) {
    let (exts, label) = gp_exts::registry(model);
    (std::sync::Arc::new(exts), label)
}

/// The hash of a value's JSON.
fn hash_json(v: &impl serde::Serialize) -> String {
    gp_evidence::Hash::of(&serde_json::to_vec(v).unwrap_or_default()).to_string()
}

/// The log row's format.
pub(crate) const LOG_SCHEMA: &str = "gp author log 1";

#[derive(Args)]
pub struct AuthorArgs {
    /// What the person asked for, in their words ("a floatier jump").
    #[arg(long)]
    pub prompt: String,
    /// The template to start from: a directory holding model.toml (and
    /// template.toml).
    #[arg(long)]
    pub base: PathBuf,
    /// The level difficulty wanted, in bits summed over the model's levels
    /// (gp level difficulty). A proposal outside a quarter of it either
    /// way fails, with "too hard" or "too easy" as its feedback.
    #[arg(long, value_name = "BITS")]
    pub target_bits: Option<f64>,
    /// The most proposals tried.
    #[arg(long, default_value_t = 3)]
    pub attempts: usize,
    /// Verify and log only; write no variant. The only mode built.
    #[arg(long)]
    pub dry_run: bool,
    /// The log to add the row to (default: $GP_HOME/author/log.jsonl).
    #[arg(long, value_name = "FILE")]
    pub log: Option<PathBuf>,
    /// The model's lifted assets (default:
    /// $GP_HOME/work/assets/<model.source>).
    #[arg(long, value_name = "DIR")]
    pub assets: Option<PathBuf>,
}

/// One proposed edit: new values for mechanic parameters by model path,
/// as a profile switch's `set` holds them, and why.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Proposal {
    pub set: BTreeMap<String, toml::Value>,
    pub why: String,
    /// What the edit should do to the game, measured by [`feel`].
    pub expect: Expect,
}

/// A measurable direction a request names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Expect {
    /// The jump stays in the air longer.
    MoreAirtime,
    /// A running jump carries further.
    FurtherRun,
    /// The levels score fewer bits.
    Easier,
    /// The levels score more bits.
    Harder,
}

impl Expect {
    fn words(self) -> &'static str {
        match self {
            Expect::MoreAirtime => "the jump stays in the air longer",
            Expect::FurtherRun => "a running jump carries further",
            Expect::Easier => "the levels score fewer bits",
            Expect::Harder => "the levels score more bits",
        }
    }
}

/// Makes proposals. It sees the request and every earlier attempt's
/// feedback, and returns None when it has nothing more to try.
pub(crate) trait Proposer {
    fn name(&self) -> &'static str;
    fn propose(&mut self, prompt: &str, base: &Model, feedback: &[String]) -> Option<Proposal>;
}

/// The scripted proposer: request words pick a list of edits, tried in
/// order. It ignores the feedback's content, which an LLM would read.
pub(crate) struct Scripted {
    queue: Option<Vec<Proposal>>,
}

impl Scripted {
    pub(crate) fn new() -> Self {
        Self { queue: None }
    }
}

/// The first entity with this role, by name order.
fn with_role(tree: &toml::Value, role: &str) -> Option<String> {
    tree.get("entities")?
        .as_table()?
        .iter()
        .find(|(_, e)| e.get("role").and_then(toml::Value::as_str) == Some(role))
        .map(|(n, _)| n.clone())
}

fn at<'a>(tree: &'a toml::Value, path: &str) -> Option<&'a toml::Value> {
    path.split('.').try_fold(tree, |v, k| v.get(k))
}

fn one(path: String, v: toml::Value, why: String, expect: Expect) -> Proposal {
    Proposal {
        set: BTreeMap::from([(path, v)]),
        why,
        expect,
    }
}

/// The edits for a request, from the base model's own paths: the player
/// is the entity whose role is `player`, an enemy the first whose role is
/// `enemy`, so the table works on any model with those parts.
pub(crate) fn scripted_edits(prompt: &str, base: &Model) -> Vec<Proposal> {
    let tree = base.to_tree();
    let p = prompt.to_ascii_lowercase();
    let has = |ws: &[&str]| ws.iter().any(|w| p.contains(w));
    let player = with_role(&tree, "player");
    let enemy = with_role(&tree, "enemy");
    let mut out = Vec::new();
    if let Some(pl) = &player {
        let g = format!("entities.{pl}.jump.gravity");
        if has(&["float", "moon", "glide"])
            && let Some(f) = at(&tree, &g)
                .and_then(toml::Value::as_str)
                .and_then(|s| Fixed::parse(s).ok())
        {
            for (num, den) in [(3, 4), (1, 2)] {
                let v = Fixed::from_raw(f.raw() * num / den);
                out.push(one(
                    g.clone(),
                    toml::Value::String(v.to_string()),
                    format!("jump gravity {f} -> {v} ({num}/{den})"),
                    Expect::MoreAirtime,
                ));
            }
        }
        let r = format!("entities.{pl}.run.run_speed");
        if has(&["fast", "speed", "quick", "zoom"])
            && let Some(s) = at(&tree, &r).and_then(toml::Value::as_integer)
        {
            for n in [s + 1, s + 2] {
                out.push(one(
                    r.clone(),
                    toml::Value::Integer(n),
                    format!("run speed {s} -> {n} px a frame"),
                    Expect::FurtherRun,
                ));
            }
        }
    }
    if let Some(en) = &enemy {
        let per = format!("entities.{en}.patrol.period");
        if let Some(n) = at(&tree, &per).and_then(toml::Value::as_integer) {
            if has(&["easy", "easier", "gentle", "kid"]) {
                for m in [n * 2, n * 4] {
                    out.push(one(
                        per.clone(),
                        toml::Value::Integer(m),
                        format!("{en} steps every {m} frames, not {n}"),
                        Expect::Easier,
                    ));
                }
            }
            if has(&["hard", "harder", "tough", "kaizo"]) && n > 1 {
                out.push(one(
                    per.clone(),
                    toml::Value::Integer(1),
                    format!("{en} steps every frame, not every {n}"),
                    Expect::Harder,
                ));
            }
        }
    }
    out
}

impl Proposer for Scripted {
    fn name(&self) -> &'static str {
        "scripted 1"
    }

    fn propose(&mut self, prompt: &str, base: &Model, _feedback: &[String]) -> Option<Proposal> {
        let q = self.queue.get_or_insert_with(|| {
            let mut v = scripted_edits(prompt, base);
            v.reverse();
            v
        });
        q.pop()
    }
}

/// What one verifier said about one proposal.
#[derive(Debug, Clone)]
struct Check {
    name: &'static str,
    /// `pass`, `fail` or `skipped`.
    status: &'static str,
    detail: String,
}

impl Check {
    fn pass(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            status: "pass",
            detail: detail.into(),
        }
    }
    fn fail(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            status: "fail",
            detail: detail.into(),
        }
    }
    fn skipped(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            status: "skipped",
            detail: detail.into(),
        }
    }
    fn json(&self) -> Value {
        json!({ "check": self.name, "status": self.status, "detail": self.detail })
    }
}

/// What the game measures as: per level, its difficulty in millibits
/// (None when no run was found), and the jump's envelope on the first
/// level.
#[derive(Debug, Clone, PartialEq)]
struct Measures {
    millibits: BTreeMap<String, Option<u32>>,
    airtime: Option<u32>,
    rise: Option<i64>,
    run_distance: Option<i64>,
}

impl Measures {
    fn total(&self) -> Option<u32> {
        self.millibits.values().copied().sum()
    }
    fn json(&self) -> Value {
        json!({
            "millibits": self.millibits,
            "total_millibits": self.total(),
            "airtime": self.airtime,
            "rise": self.rise,
            "run_distance": self.run_distance,
        })
    }
}

fn measure(spec: &gp_sim::Spec, exts: &Exts) -> Result<Measures> {
    let mut millibits = BTreeMap::new();
    for (i, l) in spec.levels.iter().enumerate() {
        let d = gp_interp::difficulty::score(spec, i, &NativeSim(exts), Default::default())
            .map_err(anyhow::Error::msg)?;
        millibits.insert(l.name.clone(), d.cleared.then_some(d.millibits));
    }
    let env = if spec.levels.is_empty() {
        None
    } else {
        gp_interp::reach::envelope(spec, 0, &NativeSim(exts)).ok()
    };
    Ok(Measures {
        millibits,
        airtime: env.map(|e| e.airtime),
        rise: env.map(|e| e.rise),
        run_distance: env.and_then(|e| e.run_distance),
    })
}

fn spec_of(m: &Model, lifted: &Lifted) -> Result<gp_sim::Spec> {
    let bundle = Bundle::new(m, lifted.clone());
    let rt = Interp::from_bundle(&bundle).map_err(anyhow::Error::msg)?;
    Ok(rt.spec().clone())
}

fn opt<T: std::fmt::Display>(v: Option<T>) -> String {
    v.map_or("-".into(), |v| v.to_string())
}

fn bits(mb: u32) -> String {
    format!("{}.{:03}", mb / 1000, mb % 1000)
}

/// The cheap part of `gp template check --public` (proposed in
/// docs/research/user-paths.md, 4.6): the tier and the source. The rest
/// (slots, names, likeness) isn't built.
fn public_check(base: &Path) -> Check {
    let name = "public";
    let Ok(text) = fs::read_to_string(base.join("template.toml")) else {
        return Check::skipped(name, "no template.toml beside the model");
    };
    let Ok(t) = text.parse::<toml::Table>() else {
        return Check::fail(name, "template.toml doesn't parse");
    };
    let tier = t
        .get("template")
        .and_then(|v| v.get("tier"))
        .and_then(toml::Value::as_str)
        .unwrap_or("");
    let how = t
        .get("source")
        .and_then(|v| v.get("how"))
        .and_then(toml::Value::as_str)
        .unwrap_or("");
    if tier != "open" {
        return Check::fail(
            name,
            format!("the base's tier is `{tier}`, not `open`: a variant of it can't be served"),
        );
    }
    if how == "rom" {
        return Check::fail(name, "the base's source is a ROM dump");
    }
    Check::pass(
        name,
        format!(
            "partial: tier `{tier}`, source built `{how}`; slots, names and likeness wait for gp template check --public"
        ),
    )
}

/// Runs every verifier on one proposal. Returns the checks, the variant's
/// measures when it got that far, and its model hash.
fn verify(
    base: &Model,
    base_measures: &Measures,
    lifted: &Lifted,
    exts: &Exts,
    p: &Proposal,
    target: Option<u32>,
    public: &Check,
) -> (Vec<Check>, Option<Measures>, Option<String>) {
    let mut checks = Vec::new();
    let profiles = Profiles {
        format: gp_model::profile::FORMAT.into(),
        groups: vec![Group {
            name: "author".into(),
            title: None,
            description: None,
            switches: vec![Switch {
                name: "proposal".into(),
                description: p.why.clone(),
                set: p.set.clone(),
                off: Vec::new(),
            }],
        }],
        presets: BTreeMap::new(),
    };
    // Schema: every path a mechanic's parameter the base has, of its type,
    // and the model valid after (gp_model::profile's own rules).
    let mut variant = match profiles.apply_switches(base, &["proposal"]) {
        Ok(m) => {
            checks.push(Check::pass(
                "schema",
                "each path is a mechanic parameter the base has, and the model is valid",
            ));
            m
        }
        Err(e) => {
            checks.push(Check::fail("schema", e));
            return (checks, None, None);
        }
    };
    variant.model.profile = None;
    let hash = crate::scenario::model_hash(&variant.to_toml());
    let cat = gp_interp::mechanics::catalog(&variant, exts);
    if cat.errors() == 0 {
        checks.push(Check::pass(
            "mechanics",
            format!(
                "{} catalogue mechanics, none disagreeing with its record",
                cat.mechanics.len()
            ),
        ));
    } else {
        let first = cat
            .problems
            .iter()
            .find(|p| p.severity == "error")
            .map(|p| format!("{}: {}", p.path, p.message))
            .unwrap_or_default();
        checks.push(Check::fail("mechanics", first));
        return (checks, None, Some(hash));
    }
    checks.push(public.clone());
    let spec = match spec_of(&variant, lifted) {
        Ok(s) => s,
        Err(e) => {
            checks.push(Check::fail(
                "reach",
                format!("the interpreter refused it: {e}"),
            ));
            return (checks, None, Some(hash));
        }
    };
    let mut reach_fail = Vec::new();
    let mut through_fail = Vec::new();
    for (i, l) in spec.levels.iter().enumerate() {
        match gp_interp::reach::reach(&spec, i, &NativeSim(exts)) {
            Ok(r) if r.goal_path.is_some() => {}
            Ok(_) => reach_fail.push(l.name.clone()),
            Err(e) => reach_fail.push(format!("{} ({e})", l.name)),
        }
        match gp_interp::reach::playthrough(&spec, i, &NativeSim(exts)) {
            Ok(Some(_)) => {}
            Ok(None) => through_fail.push(l.name.clone()),
            Err(e) => through_fail.push(format!("{} ({e})", l.name)),
        }
    }
    let n = spec.levels.len();
    checks.push(if reach_fail.is_empty() {
        Check::pass(
            "reach",
            format!("the goal is reachable by the player's moves ({n} of {n} levels)"),
        )
    } else {
        Check::fail(
            "reach",
            format!("the goal can't be reached in {}", reach_fail.join(", ")),
        )
    });
    checks.push(if through_fail.is_empty() {
        Check::pass(
            "playthrough",
            format!("a run to the goal with the enemies found ({n} of {n} levels)"),
        )
    } else {
        Check::fail(
            "playthrough",
            format!(
                "no run with the enemies found in {}",
                through_fail.join(", ")
            ),
        )
    });
    let m = match measure(&spec, exts) {
        Ok(m) => m,
        Err(e) => {
            checks.push(Check::fail("difficulty", e.to_string()));
            return (checks, None, Some(hash));
        }
    };
    let (b, v) = (base_measures.total(), m.total());
    checks.push(match (target, v) {
        (_, None) => Check::fail("difficulty", "no level could be scored"),
        (None, Some(v)) => Check::pass(
            "difficulty",
            format!(
                "{} bits (base {}); no target given",
                bits(v),
                b.map_or("-".into(), bits)
            ),
        ),
        (Some(t), Some(v)) => {
            let lo = t - t / 4;
            let hi = t + t / 4;
            if v < lo {
                Check::fail(
                    "difficulty",
                    format!("too easy: {} bits, target {} (+-25%)", bits(v), bits(t)),
                )
            } else if v > hi {
                Check::fail(
                    "difficulty",
                    format!("too hard: {} bits, target {} (+-25%)", bits(v), bits(t)),
                )
            } else {
                Check::pass(
                    "difficulty",
                    format!(
                        "{} bits, within a quarter of the target {}",
                        bits(v),
                        bits(t)
                    ),
                )
            }
        }
    });
    checks.push(feel(p.expect, base_measures, &m));
    checks.push(Check::skipped(
        "conformance",
        "the interpreter and a generated build agreeing (game-printer's gp gen conform) needs the Bevy generator, which gameprinter dropped; not run",
    ));
    (checks, Some(m), Some(hash))
}

/// Whether the edit did what the request named, by the tools' measures.
fn feel(expect: Expect, base: &Measures, v: &Measures) -> Check {
    let name = "feel";
    let (ok, was, now) = match expect {
        Expect::MoreAirtime => (
            v.airtime > base.airtime,
            format!("airtime {} frames", opt(base.airtime)),
            opt(v.airtime),
        ),
        Expect::FurtherRun => (
            v.run_distance > base.run_distance,
            format!("running jump {} px", opt(base.run_distance)),
            opt(v.run_distance),
        ),
        Expect::Easier => (
            v.total() < base.total(),
            format!("{} bits", opt(base.total().map(bits))),
            opt(v.total().map(bits)),
        ),
        Expect::Harder => (
            v.total() > base.total(),
            format!("{} bits", opt(base.total().map(bits))),
            opt(v.total().map(bits)),
        ),
    };
    let detail = format!("{}: {was} -> {now}", expect.words());
    if ok {
        Check::pass(name, detail)
    } else {
        Check::fail(name, format!("not met, {detail}"))
    }
}

pub(crate) fn run(a: &AuthorArgs, paths: &Paths) -> Result<Outcome> {
    if !a.dry_run {
        bail!(
            "only --dry-run is built: gp author verifies a proposal and logs it, but doesn't write a variant yet (docs/research/authoring-api.md)"
        );
    }
    let file = model_file(&a.base);
    let text = fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
    let base = Model::parse(&text).with_context(|| file.display().to_string())?;
    let dir = match &a.assets {
        Some(d) => d.clone(),
        None => {
            let Some(source) = &base.model.source else {
                bail!("the model names no source, so give its lifted assets with --assets");
            };
            paths.home.join("work").join("assets").join(source)
        }
    };
    let lifted = Lifted::read_dir(&dir, &base).map_err(|e| {
        anyhow::anyhow!(
            "{e}\n(lift the model's assets with gp assets lift, or name them with --assets)"
        )
    })?;
    let (exts, _) = hooks_for(&base.model.name);
    let base_spec = spec_of(&base, &lifted)?;
    let base_measures = measure(&base_spec, &exts)?;
    let base_hash = crate::scenario::model_hash(&base.to_toml());
    let base_dir = file.parent().unwrap_or(Path::new(".")).to_owned();
    let public = public_check(&base_dir);
    let target = a.target_bits.map(|b| (b * 1000.0).round().max(0.0) as u32);

    let mut proposer = Scripted::new();
    let mut feedback: Vec<String> = Vec::new();
    let mut attempts = Vec::new();
    let mut kept = None;
    let mut text = format!(
        "request: {:?} on {} (model {base_hash}); base {} bits, jump airtime {} frames\n",
        a.prompt,
        base.model.name,
        base_measures.total().map_or("-".into(), bits),
        opt(base_measures.airtime)
    );
    for n in 1..=a.attempts {
        let Some(p) = proposer.propose(&a.prompt, &base, &feedback) else {
            if n == 1 {
                text.push_str(
                    "the scripted proposer has no edit for these words (it knows float, fast, easy, hard)\n",
                );
            }
            break;
        };
        let (checks, m, hash) = verify(&base, &base_measures, &lifted, &exts, &p, target, &public);
        let failed: Vec<&Check> = checks.iter().filter(|c| c.status == "fail").collect();
        let ok = failed.is_empty();
        text.push_str(&format!("attempt {n}: {}\n", p.why));
        for c in &checks {
            text.push_str(&format!("  {:<12} {:<8} {}\n", c.name, c.status, c.detail));
        }
        let fb = failed
            .iter()
            .map(|c| format!("{}: {}", c.name, c.detail))
            .collect::<Vec<_>>()
            .join("; ");
        attempts.push(json!({
            "attempt": n,
            "proposer": proposer.name(),
            "proposal": {
                "set": p.set.iter().map(|(k, v)| (k.clone(), toml_json(v))).collect::<BTreeMap<_, _>>(),
                "why": p.why,
                "expect": p.expect.words(),
            },
            "model_hash": hash,
            "checks": checks.iter().map(Check::json).collect::<Vec<_>>(),
            "measures": m.as_ref().map(Measures::json),
            "passed": ok,
            "feedback": (!ok).then(|| fb.clone()),
        }));
        if ok {
            kept = Some(n);
            text.push_str(&format!("  kept (model {})\n", hash.unwrap_or_default()));
            break;
        }
        feedback.push(fb);
    }
    if kept.is_none() && !attempts.is_empty() {
        text.push_str("no proposal passed every check\n");
    }
    text.push_str("preview: not built in a dry run (gp print --web is proposed)\n");

    let request = json!({
        "prompt": a.prompt,
        "base": base.model.name,
        "base_model_hash": base_hash,
        "target_millibits": target,
        "profile": gp_model::profile::BASE,
    });
    let mut row = json!({
        "schema": LOG_SCHEMA,
        "request": request,
        "base_measures": base_measures.json(),
        "attempts": attempts,
        "kept": kept,
        "edits": null,
        "rating": null,
        "consent": null,
        "gp": { "version": env!("CARGO_PKG_VERSION"), "build": build() },
    });
    let id = hash_json(&row);
    row["id"] = json!(id[id.len() - 16..]);
    let log = a
        .log
        .clone()
        .unwrap_or_else(|| paths.home.join("author").join("log.jsonl"));
    if let Some(parent) = log.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .with_context(|| format!("opening {}", log.display()))?;
    writeln!(f, "{}", serde_json::to_string(&row)?)?;
    text.push_str(&format!("log: one row added to {}", log.display()));
    Ok(Outcome {
        json: json!({ "ok": kept.is_some(), "log": log, "row": row }),
        text,
        exit: if kept.is_some() { 0 } else { 1 },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mosshop() -> Model {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/mosshop/model.toml");
        Model::parse(&fs::read_to_string(p).unwrap()).unwrap()
    }

    #[test]
    fn request_words_pick_edits_from_the_base_models_own_paths() {
        let m = mosshop();
        let f = scripted_edits("Make the jump floatier", &m);
        assert_eq!(f.len(), 2);
        assert_eq!(
            f[0].set.get("entities.player.jump.gravity"),
            Some(&toml::Value::String("0.1875".into()))
        );
        assert_eq!(f[1].expect, Expect::MoreAirtime);
        let e = scripted_edits("easier please", &m);
        assert_eq!(
            e[0].set.get("entities.slug.patrol.period"),
            Some(&toml::Value::Integer(4))
        );
        assert!(scripted_edits("vaporwave it", &m).is_empty());
    }
}
