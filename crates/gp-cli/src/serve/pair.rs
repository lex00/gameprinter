//! Side-by-side playback (docs/inspector.md, "Original and model side by
//! side"): an original recording and a run of its model on the same input,
//! lined up frame by frame, with the state mapping's paths compared.
//!
//! The model run is made with `gp record --model ... --polled-of ID`, which
//! plays the frames of the original's input log in which its game read the
//! controller, as `gp interp compare` does. The lining up is the mapping's
//! `polled` alignment worked out from the two input logs (and, for a
//! mapping with a `start` condition, the original's trace), not by
//! replaying. A model run recorded on the whole log is lined up frame for
//! frame instead.

use std::collections::{BTreeMap, HashMap};

use gp_evidence::InputLog;
use gp_evidence::manifest::SourceRef;
use gp_interp::mapping::{Mapping, agrees, show};
use gp_interp::modeltrace;
use gp_runtime::{Entry, Value};
use serde_json::{Value as Json, json};

use super::select::{Hitboxes, MappedFrame, mapping_in};
use super::{Ctx, Fail, Params, Route};
use crate::mechanic::http::{Request, Response};
use crate::open::Opened;

pub(super) fn routes() -> Vec<Route> {
    vec![
        Route {
            path: "/api/pair",
            about: "`?original=ID&model=ID`: an original recording and a run of its model lined up, and how each mapped path agrees over the run",
            handler: summary,
        },
        Route {
            path: "/api/pair/frame",
            about: "`?original=ID&model=ID&frame=N`: the mapped paths of both at one original frame, which differ, and the entity boxes of both",
            handler: frame,
        },
    ]
}

/// How the model's frames line up with the original's.
pub(crate) struct Alignment {
    /// `polled` or `frame`.
    pub kind: &'static str,
    /// (model frame, original frame), by model frame.
    pub pairs: Vec<(u64, u64)>,
    /// Whether the model's input is the original's at every pair.
    pub input_matches: bool,
    /// The first model frame whose input isn't the original's.
    pub first_mismatch: Option<u64>,
}

impl Alignment {
    /// The pair at or before an original frame; the first before any.
    pub fn at(&self, original: u64) -> (u64, u64) {
        let n = self.pairs.partition_point(|p| p.1 <= original);
        self.pairs[n.saturating_sub(1)]
    }
}

pub(crate) struct Setup {
    pub original: Opened,
    pub model: Opened,
    pub mapping: Mapping,
    pub hitboxes: Hitboxes,
    pub file: String,
    pub game: String,
    pub align: Alignment,
}

fn input_of(o: &Opened) -> Result<InputLog, Fail> {
    let bytes = o
        .store
        .get(&o.manifest.input_log)
        .map_err(anyhow::Error::from)?;
    InputLog::parse(&String::from_utf8_lossy(&bytes))
        .map_err(|e| Fail::from(anyhow::Error::from(e)))
}

/// Opens both recordings and finds the mapping that joins them.
pub(crate) fn setup(ctx: &Ctx, original: &str, model: &str) -> Result<Setup, Fail> {
    let original = super::recordings::open(ctx, original)?;
    let model = super::recordings::open(ctx, model)?;
    if !matches!(original.manifest.source, SourceRef::NesRom { .. }) {
        return Err(Fail::bad_request(format!(
            "{} isn't a recording of a ROM; the original is the first recording",
            original.short()
        )));
    }
    let SourceRef::Model { name, .. } = &model.manifest.source else {
        return Err(Fail::bad_request(format!(
            "{} isn't a run of a model; make one with `gp record --model DIR --polled-of {}`",
            model.short(),
            &original.entry.id.to_hex()[..12]
        )));
    };
    let model_name = name.clone();
    let game = super::labels::load(ctx, original.sha256())?
        .game
        .ok_or_else(|| Fail::not_found("the original's game isn't known"))?;
    let Some((mapping, hitboxes, file)) =
        mapping_in(ctx, &game).map_err(|e| Fail::from(anyhow::anyhow!(e)))?
    else {
        return Err(Fail::not_found(format!(
            "no game in the games checkout has a state mapping for {game}"
        )));
    };
    if mapping.model != model_name {
        return Err(Fail::bad_request(format!(
            "the mapping for {game} is for the model `{}`, not `{model_name}`",
            mapping.model
        )));
    }
    let align = align(ctx, &original, &model, &mapping)?;
    Ok(Setup {
        original,
        model,
        mapping,
        hitboxes,
        file: file.display().to_string(),
        game,
        align,
    })
}

/// Lines the model's frames up with the original's.
fn align(ctx: &Ctx, orig: &Opened, model: &Opened, mapping: &Mapping) -> Result<Alignment, Fail> {
    let (oi, mi) = (input_of(orig)?, input_of(model)?);
    let pad = |l: &InputLog, i: usize| l.frames.get(i).map(|f| f.pads[0]);
    // The whole log played as it is: frame for frame.
    if orig.manifest.frames == model.manifest.frames
        && (0..oi.frames.len()).all(|i| pad(&oi, i) == pad(&mi, i))
    {
        return Ok(Alignment {
            kind: "frame",
            pairs: (0..=orig.manifest.frames).map(|f| (f, f)).collect(),
            input_matches: true,
            first_mismatch: None,
        });
    }
    let start = mapping
        .start
        .as_deref()
        .map(gp_runtime::select::Condition::parse)
        .transpose()
        .map_err(|e| Fail::bad_request(format!("the mapping's start: {e}")))?;
    let holds = |frame: u64| -> Result<bool, Fail> {
        let Some(cond) = &start else {
            return Ok(false);
        };
        let (state, _) = orig.state(frame, None, &ctx.paths)?;
        gp_nes::protocol::holds_in_state(&state, frame, cond)
            .map_err(|e| Fail::from(anyhow::anyhow!(e)))
    };
    let mut pairs = Vec::new();
    let mut started = false;
    if start.is_some() && holds(0)? {
        started = true;
        pairs.push((0, 0));
    }
    let mut next = 0u64;
    for (i, f) in oi.frames.iter().enumerate() {
        let after = i as u64 + 1;
        if start.is_some() && !started {
            if holds(after)? {
                started = true;
                pairs.push((0, after));
            }
            continue;
        }
        if !f.polled {
            continue;
        }
        if !started {
            started = true;
            pairs.push((0, i as u64));
        }
        next += 1;
        if next > model.manifest.frames {
            break;
        }
        pairs.push((next, after));
    }
    if pairs.is_empty() {
        pairs.push((0, 0));
    }
    let mut first_mismatch = None;
    for &(k, of) in pairs.iter().filter(|p| p.0 > 0) {
        if pad(&mi, (k - 1) as usize) != pad(&oi, (of - 1) as usize) {
            first_mismatch = Some(k);
            break;
        }
    }
    Ok(Alignment {
        kind: "polled",
        pairs,
        input_matches: first_mismatch.is_none(),
        first_mismatch,
    })
}

fn name_of(o: &Opened) -> String {
    o.entry.name.clone()
}

fn original_ram(ctx: &Ctx, orig: &Opened, of: u64) -> Result<Vec<u8>, Fail> {
    let (state, _) = orig.state(of, None, &ctx.paths)?;
    let table = crate::recordings::field_table_of(&orig.store, &orig.manifest)?;
    let fields = table.split(&state).map_err(anyhow::Error::from)?;
    Ok(fields
        .into_iter()
        .find(|(n, _)| *n == "ram")
        .map(|(_, b)| b.to_vec())
        .unwrap_or_default())
}

fn rows(
    s: &Setup,
    ram: &[u8],
    model: &HashMap<String, Value>,
    labels: &super::labels::Loaded,
) -> Vec<Json> {
    let frame = MappedFrame {
        mapping: &s.mapping,
        hitboxes: &s.hitboxes,
        ram,
    };
    s.mapping
        .fields
        .iter()
        .map(|f| {
            let o = frame.query(&f.path).unwrap_or(Value::Int(0));
            let m = model.get(&f.path);
            let differs = m.is_none_or(|m| !agrees(f, &o, m));
            let addrs: Vec<u16> = f.ram.iter().chain(&f.frac).copied().collect();
            json!({
                "path": f.path,
                "original": show(&o),
                "model": m.map(show),
                "differs": differs,
                "note": f.note,
                "ram": addrs,
                "labels": addrs.iter().flat_map(|a| labels.at(*a)).collect::<Vec<_>>(),
            })
        })
        .collect()
}

/// A box on the picture.
fn boxj(x: i64, y: Option<i64>, w: i64, h: Option<i64>) -> Json {
    json!({ "x": x, "y": y, "w": w, "h": h })
}

fn original_entities(s: &Setup, ram: &[u8]) -> (Vec<Json>, Option<(String, i64, i64)>) {
    let frame = MappedFrame {
        mapping: &s.mapping,
        hitboxes: &s.hitboxes,
        ram,
    };
    let camera = frame.camera();
    let (cx, cy) = camera
        .as_ref()
        .map_or((0, 0), |(_, x, y)| (x.unwrap_or(0), y.unwrap_or(0)));
    let mut out = Vec::new();
    for inst in frame.instances() {
        let alive = frame.query(&format!("{}.alive", inst.path)) != Some(Value::Flag(false));
        if let (true, Some((x, y, w, h))) = (alive, frame.hitbox(&inst)) {
            out.push(json!({ "path": inst.path, "kind": inst.kind, "box": boxj(x - cx, y.map(|y| y - cy), w, h) }));
        }
    }
    (out, camera.map(|(n, _, _)| (n, cx, cy)))
}

fn number(entries: &HashMap<String, Value>, path: &str) -> Option<i64> {
    match entries.get(path)? {
        Value::Fixed(raw) => Some(raw >> 16),
        Value::Int(n) => Some(*n),
        _ => None,
    }
}

/// The model's living entities as boxes on the picture, from its trace
/// state: each one's position plus its kind's hitbox, less the first
/// camera.
fn model_entities(
    hitboxes: &Hitboxes,
    entries: &[Entry],
    map: &HashMap<String, Value>,
) -> Vec<Json> {
    let camera = entries
        .iter()
        .find_map(|e| e.path.strip_prefix("cameras.")?.strip_suffix(".x"))
        .map(|n| {
            (
                number(map, &format!("cameras.{n}.x")).unwrap_or(0),
                number(map, &format!("cameras.{n}.y")).unwrap_or(0),
            )
        })
        .unwrap_or((0, 0));
    let mut out = Vec::new();
    for e in entries {
        let Some(inst) = e
            .path
            .strip_suffix(".x")
            .filter(|p| p.starts_with("entities."))
        else {
            continue;
        };
        let kind = inst
            .strip_prefix("entities.")
            .and_then(|r| r.split('[').next())
            .unwrap_or("");
        let flag = |f: &str| map.get(&format!("{inst}.{f}")) == Some(&Value::Flag(true));
        if !flag("alive") || flag("waiting") {
            continue;
        }
        let Some([hx, hy, hw, hh]) = hitboxes.get(kind) else {
            continue;
        };
        let (Some(x), Some(y)) = (
            number(map, &format!("{inst}.x")),
            number(map, &format!("{inst}.y")),
        ) else {
            continue;
        };
        let raw = |v: gp_model::Fixed| v.raw() >> 16;
        out.push(json!({
            "path": inst, "kind": kind,
            "box": boxj(x + raw(*hx) - camera.0, Some(y + raw(*hy) - camera.1), raw(*hw), Some(raw(*hh))),
        }));
    }
    out
}

fn frame(ctx: &Ctx, req: &Request, _: &Params) -> Result<Response, Fail> {
    let (Some(o), Some(m)) = (req.param("original"), req.param("model")) else {
        return Err(Fail::bad_request("give ?original=ID&model=ID&frame=N"));
    };
    let frame_text = req.param("frame").unwrap_or("");
    let want: u64 = frame_text.parse().map_err(|_| {
        Fail::bad_request(format!("frame must be a whole number, not {frame_text:?}"))
    })?;
    let s = setup(ctx, o, m)?;
    if want > s.original.manifest.frames {
        return Err(Fail::bad_request(format!(
            "the original has {} frames",
            s.original.manifest.frames
        )));
    }
    let (k, of) = s.align.at(want);
    let ram = original_ram(ctx, &s.original, of)?;
    let entries = super::select::model_entries(&s.model, k)?;
    let map: HashMap<String, Value> = entries
        .iter()
        .map(|e| (e.path.clone(), e.value.clone()))
        .collect();
    let labels = super::labels::load(ctx, s.original.sha256())?;
    let rows = rows(&s, &ram, &map, &labels);
    let (oe, camera) = original_entities(&s, &ram);
    Ok(Response::json(&json!({
        "original": { "id": s.original.entry.id, "frame": of },
        "model": { "id": s.model.entry.id, "frame": k },
        "asked": want,
        "aligned": of == want,
        "alignment": s.align.kind,
        "paths": rows,
        "differing": rows.iter().filter(|r| r["differs"] == true).count(),
        "original_entities": oe,
        "model_entities": model_entities(&s.hitboxes, &entries, &map),
        "camera": camera.map(|(n, x, y)| json!({ "path": format!("cameras.{n}"), "x": x, "y": y })),
    })))
}

/// Ranges `[a, b]` (inclusive) of the sorted frames.
fn ranges(frames: &[u64]) -> Vec<[u64; 2]> {
    let mut out: Vec<[u64; 2]> = Vec::new();
    for &f in frames {
        match out.last_mut() {
            Some(r) if r[1] + 1 == f => r[1] = f,
            _ => out.push([f, f]),
        }
    }
    out
}

fn summary(ctx: &Ctx, req: &Request, _: &Params) -> Result<Response, Fail> {
    let (Some(o), Some(m)) = (req.param("original"), req.param("model")) else {
        return Err(Fail::bad_request("give ?original=ID&model=ID"));
    };
    let s = setup(ctx, o, m)?;
    let last_k = s.align.pairs.last().map_or(0, |p| p.0);
    let last_of = s.align.pairs.last().map_or(0, |p| p.1);
    // The mapped paths' model values at every model frame, from its trace.
    let mut model_at: BTreeMap<u64, HashMap<String, Value>> = BTreeMap::new();
    let table = gp_nes::recording::field_table(&s.model.store, &s.model.manifest)
        .map_err(anyhow::Error::from)?;
    let wanted: std::collections::HashSet<&str> =
        s.mapping.fields.iter().map(|f| f.path.as_str()).collect();
    let scanned =
        gp_nes::recording::scan_trace(&s.model.store, &s.model.manifest, 0..=last_k, |f, state| {
            if let Ok(entries) = modeltrace::entries(&table, state) {
                model_at.insert(
                    f,
                    entries
                        .into_iter()
                        .filter(|e| wanted.contains(e.path.as_str()))
                        .map(|e| (e.path, e.value))
                        .collect(),
                );
            }
            true
        })
        .map_err(anyhow::Error::from)?;
    if !scanned {
        return Err(Fail::bad_request(format!(
            "{} has no frame trace; record it again with `gp record --model`",
            s.model.short()
        )));
    }
    // The original's RAM at the paired frames.
    let wanted_frames: std::collections::HashSet<u64> = s.align.pairs.iter().map(|p| p.1).collect();
    let otable = gp_nes::recording::field_table(&s.original.store, &s.original.manifest)
        .map_err(anyhow::Error::from)?;
    let mut ram_at: HashMap<u64, Vec<u8>> = HashMap::new();
    let scanned = gp_nes::recording::scan_trace(
        &s.original.store,
        &s.original.manifest,
        0..=last_of,
        |f, state| {
            if wanted_frames.contains(&f)
                && let Ok(fields) = otable.split(state)
                && let Some((_, ram)) = fields.into_iter().find(|(n, _)| *n == "ram")
            {
                ram_at.insert(f, ram.to_vec());
            }
            true
        },
    )
    .map_err(anyhow::Error::from)?;
    if !scanned {
        return Err(Fail::bad_request(format!(
            "{} has no frame trace; record it again with a trace",
            s.original.short()
        )));
    }
    let mut compared = vec![0u64; s.mapping.fields.len()];
    let mut agreed = vec![0u64; s.mapping.fields.len()];
    let mut first: Vec<Option<Json>> = vec![None; s.mapping.fields.len()];
    let mut differing = Vec::new();
    for &(k, of) in &s.align.pairs {
        let (Some(ram), Some(model)) = (ram_at.get(&of), model_at.get(&k)) else {
            continue;
        };
        let frame = MappedFrame {
            mapping: &s.mapping,
            hitboxes: &s.hitboxes,
            ram,
        };
        let mut any = false;
        for (i, f) in s.mapping.fields.iter().enumerate() {
            let o = frame.query(&f.path).unwrap_or(Value::Int(0));
            let m = model.get(&f.path);
            compared[i] += 1;
            if m.is_some_and(|m| agrees(f, &o, m)) {
                agreed[i] += 1;
            } else {
                any = true;
                first[i].get_or_insert_with(|| {
                    json!({ "original_frame": of, "model_frame": k, "original": show(&o), "model": m.map(show) })
                });
            }
        }
        if any {
            differing.push(of);
        }
    }
    let paths: Vec<Json> = s
        .mapping
        .fields
        .iter()
        .enumerate()
        .map(|(i, f)| json!({ "path": f.path, "compared": compared[i], "agreed": agreed[i], "first": first[i] }))
        .collect();
    Ok(Response::json(&json!({
        "original": { "id": s.original.entry.id, "name": name_of(&s.original), "frames": s.original.manifest.frames },
        "model": { "id": s.model.entry.id, "name": name_of(&s.model), "frames": s.model.manifest.frames },
        "game": s.game,
        "mapping": s.file,
        "alignment": s.align.kind,
        "input_matches": s.align.input_matches,
        "first_input_mismatch": s.align.first_mismatch,
        "pairs": s.align.pairs.len(),
        // The model frame at each original frame, the last paired one at or
        // before it.
        "model_frame": (0..=s.original.manifest.frames).map(|f| s.align.at(f).0).collect::<Vec<_>>(),
        "paths": paths,
        "differing": ranges(&differing),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_join_runs() {
        assert_eq!(ranges(&[1, 2, 3, 7, 9, 10]), vec![[1, 3], [7, 7], [9, 10]]);
        assert!(ranges(&[]).is_empty());
    }

    #[test]
    fn the_pair_at_a_frame_is_the_last_at_or_before_it() {
        let a = Alignment {
            kind: "polled",
            pairs: vec![(0, 4), (1, 5), (2, 6), (3, 9)],
            input_matches: true,
            first_mismatch: None,
        };
        assert_eq!(a.at(0), (0, 4));
        assert_eq!(a.at(5), (1, 5));
        assert_eq!(a.at(8), (2, 6));
        assert_eq!(a.at(100), (3, 9));
    }
}
