//! Variant previews for the workspace (docs/inspector.md, "Describing a
//! change"): the model run changed in a few ways, each run through the
//! same input in the interpreter, so the workspace can draw them over the
//! original's frames as ghosts.
//!
//! A change is structured: a model path and a new value, the edits
//! `model_edit` takes (docs/mcp.md). Nothing is written: each variant is
//! the model recording's model with its edits applied in memory, checked as
//! `gp model check` checks a model, and run with the recording's input.
//! The server stays read-only, so the edits come in the query.

use std::sync::Arc;

use gp_interp::{Bundle, Interp};
use gp_mcp::edit::{Edit, apply};
use gp_model::Model;
use serde::Deserialize;
use serde_json::{Value as Json, json};

use super::{Ctx, Fail, Params, Route};
use crate::mechanic::http::{Request, Response};

/// The most variants one request runs.
pub(crate) const MAX_VARIANTS: usize = 8;
/// The most frames one request runs.
pub(crate) const MAX_FRAMES: u64 = 4000;
/// The most entities a variant reports.
const MAX_ENTITIES: usize = 64;

pub(super) fn routes() -> Vec<Route> {
    vec![Route {
        path: "/api/recordings/{id}/variants",
        about: "`?variants=JSON`: the model run's model changed by each variant's edits, run on the recording's input; the entities' positions by frame",
        handler: variants,
    }]
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Variant {
    #[serde(default)]
    pub name: String,
    pub edits: Vec<Edit>,
}

/// The variants a request names.
pub(crate) fn parse(text: &str) -> Result<Vec<Variant>, String> {
    let v: Vec<Variant> = serde_json::from_str(text).map_err(|e| format!("variants: {e}"))?;
    if v.len() > MAX_VARIANTS {
        return Err(format!(
            "at most {MAX_VARIANTS} variants a request, not {}",
            v.len()
        ));
    }
    Ok(v)
}

/// The bundle of a model with `edits` applied, or why they can't be.
pub(crate) fn edited(bundle: &Bundle, edits: &[Edit]) -> Result<Bundle, String> {
    let model = bundle.parse_model()?;
    let mut tree = model.to_tree();
    for e in edits {
        apply(&mut tree, e)?;
    }
    let text = toml::to_string(&tree).map_err(|e| e.to_string())?;
    let new = Model::parse(&text).map_err(|e| e.to_string())?;
    let problems = gp_model::validate::validate(&new);
    if !problems.is_empty() {
        return Err(problems
            .iter()
            .take(3)
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; "));
    }
    Ok(Bundle {
        model: new.to_toml(),
        ..bundle.clone()
    })
}

/// One run: the entities' positions by frame.
struct Run {
    frames: usize,
    /// Per frame, the first camera's position.
    camera: Vec<[i64; 2]>,
    /// Per body: its path, kind, hitbox offset and size, and by frame
    /// x, y and whether it's in play.
    bodies: Vec<Traced>,
    hashes: Vec<[u8; 32]>,
}

struct Traced {
    path: String,
    kind: String,
    hitbox: [i64; 4],
    x: Vec<i64>,
    y: Vec<i64>,
    live: Vec<bool>,
}

fn run(
    bundle: &Bundle,
    input: &gp_evidence::InputLog,
    exts: &Arc<gp_sim::Exts>,
    upto: usize,
) -> Result<Run, String> {
    let mut rt = Interp::from_bundle(bundle)?.with_exts(Arc::clone(exts));
    let mut out = Run {
        frames: 0,
        camera: Vec::new(),
        bodies: Vec::new(),
        hashes: Vec::new(),
    };
    let record = |rt: &Interp, out: &mut Run| {
        let w = rt.world();
        out.camera.push(
            w.cameras
                .first()
                .map_or([0, 0], |c| [c[0].floor(), c[1].floor()]),
        );
        out.hashes.push(w.hash());
        let frame = out.frames;
        let mut counts = vec![0usize; rt.spec().kinds.len()];
        for b in &w.bodies {
            let n = counts[b.kind];
            counts[b.kind] += 1;
            let kind = &rt.spec().kinds[b.kind];
            let path = format!("entities.{}[{n}]", kind.name);
            let at = match out.bodies.iter().position(|t| t.path == path) {
                Some(i) => i,
                None => {
                    if out.bodies.len() >= MAX_ENTITIES {
                        continue;
                    }
                    out.bodies.push(Traced {
                        path,
                        kind: kind.name.clone(),
                        hitbox: [
                            kind.hitbox.x.floor(),
                            kind.hitbox.y.floor(),
                            kind.hitbox.w.floor(),
                            kind.hitbox.h.floor(),
                        ],
                        x: vec![0; frame],
                        y: vec![0; frame],
                        live: vec![false; frame],
                    });
                    out.bodies.len() - 1
                }
            };
            let t = &mut out.bodies[at];
            t.x.resize(frame, 0);
            t.y.resize(frame, 0);
            t.live.resize(frame, false);
            t.x.push(b.x.floor());
            t.y.push(b.y.floor());
            t.live.push(b.alive && !b.waiting);
        }
        out.frames += 1;
        for t in &mut out.bodies {
            t.x.resize(out.frames, 0);
            t.y.resize(out.frames, 0);
            t.live.resize(out.frames, false);
        }
    };
    record(&rt, &mut out);
    for f in input.frames.iter().take(upto) {
        rt.advance(&gp_interp::recording::input_frame(f))?;
        record(&rt, &mut out);
    }
    Ok(out)
}

fn run_json(name: &str, edits: &[Edit], r: &Run, base: Option<&Run>) -> Json {
    let first =
        base.and_then(|b| (0..r.frames.min(b.frames)).find(|&f| r.hashes[f] != b.hashes[f]));
    json!({
        "name": name,
        "ok": true,
        "edits": edits.iter().map(|e| json!({ "path": e.path, "value": e.value, "remove": e.remove })).collect::<Vec<_>>(),
        "frames": r.frames,
        "first_difference": first,
        "camera": r.camera,
        "entities": r.bodies.iter().map(|t| json!({
            "path": t.path,
            "kind": t.kind,
            "hitbox": t.hitbox,
            "x": t.x,
            "y": t.y,
            "live": t.live.iter().map(|l| u8::from(*l)).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

fn variants(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    let opened = super::recordings::open(ctx, params.get("id").map_or("", String::as_str))?;
    let bundle = gp_interp::recording::bundle(&opened.store, &opened.manifest).map_err(|_| {
        Fail::bad_request(format!(
            "{} isn't a run of a model; variants run a model on a recording's input",
            opened.short()
        ))
    })?;
    let list = parse(req.param("variants").unwrap_or("[]")).map_err(Fail::bad_request)?;
    let frames = match req.param("frames") {
        None => opened.manifest.frames,
        Some(t) => t
            .parse::<u64>()
            .map_err(|_| Fail::bad_request(format!("frames must be a whole number, not {t:?}")))?
            .min(opened.manifest.frames),
    };
    if frames > MAX_FRAMES {
        return Err(Fail::bad_request(format!(
            "a request runs at most {MAX_FRAMES} frames; give ?frames=N"
        )));
    }
    let input = gp_evidence::InputLog::parse(&String::from_utf8_lossy(
        &opened
            .store
            .get(&opened.manifest.input_log)
            .map_err(anyhow::Error::from)?,
    ))
    .map_err(|e| Fail::from(anyhow::Error::from(e)))?;
    let (exts, _) = gp_exts::registry(&bundle.parse_model().map_err(Fail::bad_request)?.model.name);
    let exts = std::sync::Arc::new(exts);
    let upto = frames as usize;
    let base = run(&bundle, &input, &exts, upto).map_err(|e| Fail::from(anyhow::anyhow!(e)))?;
    let mut out = vec![run_json("model", &[], &base, None)];
    for (i, v) in list.iter().enumerate() {
        let name = if v.name.is_empty() {
            format!("variant {}", i + 1)
        } else {
            v.name.clone()
        };
        match edited(&bundle, &v.edits).and_then(|b| run(&b, &input, &exts, upto)) {
            Ok(r) => out.push(run_json(&name, &v.edits, &r, Some(&base))),
            Err(e) => out.push(json!({ "name": name, "ok": false, "error": e })),
        }
    }
    Ok(Response::json(&json!({
        "id": opened.entry.id,
        "frames": frames,
        "variants": out,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_parse_and_are_capped() {
        let v = parse(
            r#"[{"name":"a","edits":[{"path":"entities.hero.jump.gravity","value":"0.5"}]}]"#,
        )
        .unwrap();
        assert_eq!(v[0].edits[0].path, "entities.hero.jump.gravity");
        assert!(parse("[]").unwrap().is_empty());
        assert!(parse("nope").is_err());
        assert!(parse(r#"[{"edits":[],"extra":1}]"#).is_err());
        let many = format!("[{}]", [r#"{"edits":[]}"#; MAX_VARIANTS + 1].join(","));
        assert!(parse(&many).is_err());
    }
}
