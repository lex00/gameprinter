//! Resolution cards for the workspace (docs/inspector.md, "Resolution
//! cards"): what the model says about a thing the person selected, how
//! sure it is, what the contract demands of it, what the original and the
//! model measured, and the paths to open it by.
//!
//! A card is for a model path such as `entities.slug[1]` (a box selection's
//! entity) or `rules.game.state`. Its part is the first two names, here
//! `entities.slug`; the facts are those the part holds.

use std::path::{Path, PathBuf};

use gp_contract::{CONTRACT_FILE, Contract};
use gp_evidence::manifest::SourceRef;
use gp_model::Model;
use gp_model::path::{self, Step};
use gp_runtime::Value;
use serde_json::{Value as Json, json};

use super::select::{mapping_in, model_dirs};
use super::{Ctx, Fail, Params, Route};
use crate::mechanic::http::{Request, Response};
use crate::open::Opened;
use crate::static_grade::toml_json;

/// The most facts a card lists.
const MAX_FACTS: usize = 80;

pub(super) fn routes() -> Vec<Route> {
    vec![
        Route {
            path: "/api/recordings/{id}/card",
            about: "`?path=P[&frame=N][&results=ID]`: the model's facts under a path with their evidence and contract level, the values at a frame, and the validation results measured against the original",
            handler: card,
        },
        Route {
            path: "/api/recordings/{id}/facts",
            about: "The model facts of the recording's game or model: path, value and contract level, for choosing what to change",
            handler: facts,
        },
    ]
}

/// A model found under `templates/`, with its contract.
pub(crate) struct Found {
    pub dir: PathBuf,
    pub model: Model,
    pub contract: Option<Contract>,
}

fn read(dir: &Path) -> Result<Found, Fail> {
    let text = std::fs::read_to_string(dir.join(gp_model::MODEL_FILE))
        .map_err(|e| Fail::from(anyhow::anyhow!("reading {}: {e}", dir.display())))?;
    let model =
        Model::parse(&text).map_err(|e| Fail::from(anyhow::anyhow!("{}: {e}", dir.display())))?;
    let contract = std::fs::read_to_string(dir.join(CONTRACT_FILE))
        .ok()
        .and_then(|t| Contract::parse(&t).ok());
    Ok(Found {
        dir: dir.to_path_buf(),
        model,
        contract,
    })
}

/// The model a recording belongs to: a model run's, by name, or an
/// original's, by the state mapping for its game.
pub(crate) fn model_of(ctx: &Ctx, opened: &Opened) -> Result<Found, Fail> {
    match &opened.manifest.source {
        SourceRef::Model { name, .. } => {
            for dir in model_dirs(ctx) {
                if let Ok(f) = read(&dir)
                    && &f.model.model.name == name
                {
                    return Ok(f);
                }
            }
            Err(Fail::not_found(format!(
                "no model named {name} in the games checkout or templates/"
            )))
        }
        SourceRef::NesRom { .. } | SourceRef::Romset { .. } => {
            let game = super::labels::load(ctx, opened.sha256())?
                .game
                .ok_or_else(|| Fail::not_found("the recording's game isn't known"))?;
            match mapping_in(ctx, &game).map_err(|e| Fail::from(anyhow::anyhow!(e)))? {
                Some((_, _, file)) => read(file.parent().expect("a file has a folder")),
                None => Err(Fail::not_found(format!(
                    "no game in the games checkout has a state mapping for {game}"
                ))),
            }
        }
    }
}

/// `entities.slug[1].x` -> `entities.slug`.
pub(crate) fn part_of(text: &str) -> Result<String, String> {
    let p = gp_model::Path::parse(text)?;
    Ok(p.segments
        .iter()
        .take(2)
        .map(|s| s.name.as_str())
        .collect::<Vec<_>>()
        .join("."))
}

/// Whether the fact (as a path text) is under the part.
fn under(fact: &str, part: &str) -> bool {
    fact == part
        || fact
            .strip_prefix(part)
            .is_some_and(|rest| rest.starts_with('.') || rest.starts_with('['))
}

fn fact_rows(found: &Found, keep: impl Fn(&str) -> bool) -> (Vec<Json>, usize) {
    let tree = found.model.to_tree();
    let evidence: std::collections::HashMap<String, Option<String>> =
        gp_model::validate::evidence_for(&found.model)
            .into_iter()
            .map(|(p, k)| (path::display(&p), k))
            .collect();
    let bound = found.contract.as_ref().map(|c| c.bind(&found.model));
    let mut rows = Vec::new();
    let mut total = 0;
    for (fact, value) in gp_model::facts(&tree) {
        let shown = path::display(&fact);
        if !keep(&shown) {
            continue;
        }
        total += 1;
        if rows.len() >= MAX_FACTS {
            continue;
        }
        let key = evidence.get(&shown).cloned().flatten();
        let ev = key.as_ref().and_then(|k| found.model.evidence.get(k));
        let level = bound.as_ref().map(|b| b.resolve(&fact, None));
        rows.push(json!({
            "path": shown,
            "value": toml_json(value),
            "evidence": ev.map(|e| json!({
                "entry": key,
                "source": e.source.name(),
                "confidence": e.confidence,
                "note": e.note,
                "refs": e.refs.iter().map(|r| json!({ "at": r.at, "in": r.within })).collect::<Vec<_>>(),
            })),
            "level": level.as_ref().map(|l| l.level.name()),
            "rule": level.as_ref().and_then(|l| l.rule),
            "tolerance": level.as_ref().map(|l| serde_json::to_value(&l.tolerance).unwrap_or(Json::Null)),
        }));
    }
    (rows, total)
}

fn facts(ctx: &Ctx, _: &Request, params: &Params) -> Result<Response, Fail> {
    let opened = super::recordings::open(ctx, params.get("id").map_or("", String::as_str))?;
    let found = model_of(ctx, &opened)?;
    let tree = found.model.to_tree();
    let bound = found.contract.as_ref().map(|c| c.bind(&found.model));
    let rows: Vec<Json> = gp_model::facts(&tree)
        .into_iter()
        .map(|(fact, value)| {
            json!({
                "path": path::display(&fact),
                "value": toml_json(value),
                "level": bound.as_ref().map(|b| b.resolve(&fact, None).level.name()),
            })
        })
        .collect();
    Ok(Response::json(&json!({
        "model": found.model.model.name,
        "dir": found.dir.display().to_string(),
        "facts": rows,
    })))
}

/// The recording's values at a frame under a path, with the RAM that holds
/// them when the recording is of the original.
fn values_at(ctx: &Ctx, opened: &Opened, frame: u64, text: &str) -> Result<Vec<Json>, Fail> {
    let prefix = text.split('.').take(2).collect::<Vec<_>>().join(".");
    // `entities.slug[1].x` selects the instance; `entities.slug` every one.
    let wanted = gp_model::Path::parse(text).map_err(Fail::bad_request)?;
    let narrow: Option<String> = wanted
        .segments
        .get(1)
        .filter(|s| s.index.is_some())
        .map(|_| {
            wanted
                .segments
                .iter()
                .take(2)
                .enumerate()
                .map(|(i, s)| match (&s.index, i) {
                    (Some(path::Index::At(n)), _) => format!("{}[{n}]", s.name),
                    _ => s.name.clone(),
                })
                .collect::<Vec<_>>()
                .join(".")
        });
    let keep = |p: &str| match &narrow {
        Some(n) => under(p, n),
        None => under(p, &prefix),
    };
    match &opened.manifest.source {
        SourceRef::Model { .. } => Ok(super::select::model_entries(opened, frame)?
            .into_iter()
            .filter(|e| keep(&e.path))
            .map(|e| json!({ "path": e.path, "value": gp_interp::mapping::show(&e.value) }))
            .collect()),
        SourceRef::NesRom { .. } | SourceRef::Romset { .. } => {
            let labels = super::labels::load(ctx, opened.sha256())?;
            let Some(game) = &labels.game else {
                return Ok(Vec::new());
            };
            let Some((mapping, hitboxes, _)) =
                mapping_in(ctx, game).map_err(|e| Fail::from(anyhow::anyhow!(e)))?
            else {
                return Ok(Vec::new());
            };
            let (state, _) = opened.state(frame, None, &ctx.paths)?;
            let table = crate::recordings::field_table_of(&opened.store, &opened.manifest)?;
            let ram = table
                .split(&state)
                .map_err(anyhow::Error::from)?
                .into_iter()
                .find(|(n, _)| *n == "ram")
                .map(|(_, b)| b.to_vec())
                .unwrap_or_default();
            let mapped = super::select::MappedFrame {
                mapping: &mapping,
                hitboxes: &hitboxes,
                ram: &ram,
            };
            Ok(mapping
                .fields
                .iter()
                .filter(|f| keep(&f.path))
                .map(|f| {
                    let v: Value = mapped.query(&f.path).unwrap_or(Value::Int(0));
                    let addrs: Vec<u16> = f.ram.iter().chain(&f.frac).copied().collect();
                    json!({
                        "path": f.path,
                        "value": gp_interp::mapping::show(&v),
                        "ram": addrs,
                        "labels": addrs.iter().flat_map(|a| labels.at(*a)).collect::<Vec<_>>(),
                        "note": f.note,
                    })
                })
                .collect())
        }
    }
}

/// The stored validation results of a recording whose checks are about
/// the part: `gp validate`'s outcomes, newest first, one for each check
/// and subject.
fn checks(ctx: &Ctx, id: &gp_evidence::Hash, part: &str) -> Result<Vec<Json>, Fail> {
    let store = gp_evidence::Store::open_existing(&ctx.paths.store).map_err(anyhow::Error::from)?;
    let (lines, _) =
        gp_validate::cache::lines_checked(&store).map_err(|e| Fail::from(anyhow::anyhow!(e)))?;
    let want = id.to_hex();
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for line in lines.into_iter().rev().filter(|l| l.recording == want) {
        let Ok(bytes) = store.get(&line.object) else {
            continue;
        };
        let Ok(result) = serde_json::from_slice::<gp_validate::RecordingResult>(&bytes) else {
            continue;
        };
        for o in result.outcomes.iter().filter(|o| under(&o.part, part)) {
            let key = (result.subject.clone(), o.key());
            if !seen.insert(key) {
                continue;
            }
            out.push(json!({
                "part": o.part,
                "kind": o.kind,
                "quantity": o.quantity,
                "case": o.case,
                "against": o.against,
                "level": o.level,
                "status": o.status,
                "samples": o.samples,
                "expected": o.expected,
                "actual": o.actual,
                "tolerance": o.tolerance,
                "frame": o.frame,
                "detail": o.detail,
                "subject": result.subject,
                "validated_at": line.at,
            }));
        }
    }
    Ok(out)
}

fn card(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    let text = req
        .param("path")
        .ok_or_else(|| Fail::bad_request("give the model path: ?path=entities.slug[1]"))?;
    let part = part_of(text).map_err(Fail::bad_request)?;
    let opened = super::recordings::open(ctx, params.get("id").map_or("", String::as_str))?;
    let frame: u64 = match req.param("frame") {
        None => 0,
        Some(t) => t
            .parse()
            .map_err(|_| Fail::bad_request(format!("frame must be a whole number, not {t:?}")))?,
    };
    if frame > opened.manifest.frames {
        return Err(Fail::bad_request(format!(
            "the recording has {} frames",
            opened.manifest.frames
        )));
    }
    let found = model_of(ctx, &opened)?;
    let (facts, total) = fact_rows(&found, |p| under(p, &part));
    // Results are kept against the original recording.
    let results_id = match req.param("results") {
        Some(r) => super::recordings::open(ctx, r)?.entry.id,
        None => opened.entry.id,
    };
    let checks = checks(ctx, &results_id, &part)?;
    let level = found.contract.as_ref().map(|c| {
        let steps: Vec<Step> = part.split('.').map(|s| Step::Key(s.to_owned())).collect();
        c.bind(&found.model).resolve(&steps, None).level.name()
    });
    Ok(Response::json(&json!({
        "id": opened.entry.id,
        "frame": frame,
        "path": text,
        "part": part,
        "model": { "name": found.model.model.name, "dir": found.dir.display().to_string() },
        "level": level,
        "facts": facts,
        "facts_total": total,
        "values": values_at(ctx, &opened, frame, text)?,
        "checks": checks,
        "results_for": results_id,
        // Paths to open it by: the part, and each fact.
        "links": std::iter::once(json!({ "path": part, "kind": "part" }))
            .chain(facts.iter().map(|f| json!({ "path": f["path"], "kind": "fact" })))
            .collect::<Vec<_>>(),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_part_is_the_first_two_names() {
        assert_eq!(part_of("entities.slug[1].x").unwrap(), "entities.slug");
        assert_eq!(part_of("rules.game.state").unwrap(), "rules.game");
        assert_eq!(part_of("level").unwrap(), "level");
        assert!(part_of("entities.[").is_err());
    }

    #[test]
    fn facts_are_under_a_part_by_segment() {
        assert!(under("entities.slug.hitbox.w", "entities.slug"));
        assert!(under("entities.slug", "entities.slug"));
        assert!(!under("entities.slugger.hitbox", "entities.slug"));
        assert!(under("levels.main.spawn[0].x", "levels.main"));
    }
}
