//! The store's endpoints: recordings, their manifests, input logs, field
//! tables, frame states, events, trace chunks, and two-recording diffs.
//! Each one that has a matching command calls that command's own code, so
//! the JSON is the same by construction.
//!
//! Ported from game-printer's serve/api.rs (wave 8). What gameprinter's
//! layout changed: the list and a recording's names come from the
//! recordings log ([`crate::recindex`]) where game-printer kept gp-index's
//! SQLite file; a ROM is found by hash in $GP_ROM_DIR (gp serve takes no
//! `--rom` files); no run recorder.

use crate::mechanic::http::{Request, Response};
use gp_evidence::Store;

use super::{Ctx, Fail, Params, Route};
use crate::Outcome;
use crate::open::Opened;
use crate::recindex::{Filter, Index, Retired};

pub(super) fn routes() -> Vec<Route> {
    vec![
        Route {
            path: "/api/recordings",
            about: "Every recording; `?retired=include` or `?retired=only` as for --retired and --only-retired",
            handler: recordings,
        },
        Route {
            path: "/api/recordings/{id}",
            about: "A recording's names, manifest summary and objects",
            handler: show,
        },
        Route {
            path: "/api/recordings/{id}/manifest",
            about: "The stored manifest, byte for byte (its hash is the ID)",
            handler: manifest,
        },
        Route {
            path: "/api/recordings/{id}/input",
            about: "The input log, as text",
            handler: input,
        },
        Route {
            path: "/api/recordings/{id}/fields",
            about: "The field table the recording's states follow",
            handler: fields,
        },
        Route {
            path: "/api/recordings/{id}/frames/{frame}",
            about: "The state at a frame, by field, from the trace or by restoring a checkpoint and replaying; `?field=ram,oam` selects fields",
            handler: frame,
        },
        Route {
            path: "/api/recordings/{id}/events",
            about: "Events in `?frames=A..B` (default 1..), optionally only `?kind=nmi,ppu-write`",
            handler: events,
        },
        Route {
            path: "/api/recordings/{id}/trace/{chunk}",
            about: "Trace chunk N, the stored object byte for byte, for decoding in the browser",
            handler: chunk,
        },
        Route {
            path: "/api/diff",
            about: "`?a=ID&b=ID`: the first frame whose state differs, and the fields that differ there",
            handler: diff,
        },
    ]
}

fn id(params: &Params) -> &str {
    params.get("id").map_or("", String::as_str)
}

fn number(params: &Params, name: &str) -> Result<u64, Fail> {
    let text = params.get(name).map_or("", String::as_str);
    text.parse()
        .map_err(|_| Fail::bad_request(format!("{name} must be a whole number, not {text:?}")))
}

fn list(req: &Request, name: &str) -> Vec<String> {
    req.param(name)
        .map(|v| {
            v.split(',')
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// A command's `--json` output as a response.
pub(super) fn outcome(o: Outcome) -> Response {
    Response::json(&o.json)
}

/// Opens a recording read-only: the store must be there already.
pub(crate) fn open(ctx: &Ctx, id: &str) -> anyhow::Result<Opened> {
    let store = Store::open_existing(&ctx.paths.store)?;
    let entry = gp_evidence::recordings::resolve(&store, id)?;
    let manifest = gp_evidence::recordings::load_manifest(&store, &entry.id)?;
    Ok(Opened {
        store,
        entry,
        manifest,
    })
}

/// The store and its recordings index, read from the log, so nothing is
/// written.
fn with_index<T>(
    ctx: &Ctx,
    f: impl FnOnce(&Store, &Index) -> anyhow::Result<T>,
) -> Result<T, Fail> {
    let store = Store::open_existing(&ctx.paths.store).map_err(anyhow::Error::from)?;
    let index = Index::read(&store)?;
    Ok(f(&store, &index)?)
}

fn recordings(ctx: &Ctx, req: &Request, _: &Params) -> Result<Response, Fail> {
    let retired = match req.param("retired") {
        None | Some("skip") => Retired::Skip,
        Some("include") => Retired::Include,
        Some("only") => Retired::Only,
        Some(other) => {
            return Err(Fail::bad_request(format!(
                "retired is skip, include or only, not {other:?}"
            )));
        }
    };
    let filter = Filter {
        retired,
        ..Default::default()
    };
    with_index(ctx, |_, index| {
        crate::recordings::recordings_list_in(filter, index)
    })
    .map(outcome)
}

fn show(ctx: &Ctx, _: &Request, params: &Params) -> Result<Response, Fail> {
    with_index(ctx, |store, index| {
        crate::recordings::recordings_show_in(id(params), store, index)
    })
    .map(outcome)
}

fn manifest(ctx: &Ctx, _: &Request, params: &Params) -> Result<Response, Fail> {
    let opened = super::recordings::open(ctx, id(params))?;
    let bytes = opened
        .store
        .get(&opened.entry.id)
        .map_err(anyhow::Error::from)?;
    Ok(Response::bytes("application/json", bytes))
}

fn input(ctx: &Ctx, _: &Request, params: &Params) -> Result<Response, Fail> {
    let opened = super::recordings::open(ctx, id(params))?;
    let bytes = opened
        .store
        .get(&opened.manifest.input_log)
        .map_err(anyhow::Error::from)?;
    Ok(Response::bytes("text/plain; charset=utf-8", bytes))
}

fn fields(ctx: &Ctx, _: &Request, params: &Params) -> Result<Response, Fail> {
    let opened = super::recordings::open(ctx, id(params))?;
    Ok(outcome(crate::recordings::trace_fields(&opened)?))
}

fn frame(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    let frame = number(params, "frame")?;
    let opened = super::recordings::open(ctx, id(params))?;
    let filters = list(req, "field");
    let table = crate::recordings::field_table_of(&opened.store, &opened.manifest)?;
    if let Some(f) = filters.iter().find(|f| {
        !table
            .fields
            .iter()
            .any(|d| crate::recordings::field_selected(&d.name, &[(*f).clone()]))
    }) {
        return Err(Fail::bad_request(format!(
            "no field matches {f:?}; GET .../fields lists them"
        )));
    }
    let o = crate::recordings::trace_frame_of(&opened, frame, &filters, None, &ctx.paths)?;
    Ok(outcome(o))
}

fn events(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    let opened = super::recordings::open(ctx, id(params))?;
    // A bad range or kind is the request's fault, so it's checked first.
    let frames = req.param("frames").unwrap_or("1..");
    crate::recordings::parse_frames(frames, opened.manifest.frames)
        .map_err(|e| Fail::bad_request(format!("{e:#}")))?;
    let kinds = list(req, "kind");
    let is_model = matches!(
        opened.manifest.source,
        gp_evidence::manifest::SourceRef::Model { .. }
    );
    if !is_model
        && !opened.is_arcade()
        && let Some(k) = kinds
            .iter()
            .find(|k| gp_evidence::events::EventKind::from_name(k).is_none())
    {
        return Err(Fail::bad_request(format!("unknown event kind {k:?}")));
    }
    let o = crate::recordings::events_of(&opened, frames, &kinds, None, &ctx.paths)?;
    Ok(outcome(o))
}

fn chunk(ctx: &Ctx, _: &Request, params: &Params) -> Result<Response, Fail> {
    let i = number(params, "chunk")?;
    let opened = super::recordings::open(ctx, id(params))?;
    let Some(trace) = &opened.manifest.trace else {
        return Err(Fail::not_found(format!(
            "recording {} has no trace; GET its frames instead",
            opened.short()
        )));
    };
    let Some(hash) = usize::try_from(i).ok().and_then(|i| trace.chunks.get(i)) else {
        return Err(Fail::not_found(format!(
            "the trace has {} chunks, numbered from 0",
            trace.chunks.len()
        )));
    };
    let bytes = opened.store.get(hash).map_err(anyhow::Error::from)?;
    Ok(Response::bytes("application/octet-stream", bytes))
}

fn diff(ctx: &Ctx, req: &Request, _: &Params) -> Result<Response, Fail> {
    let (Some(a), Some(b)) = (req.param("a"), req.param("b")) else {
        return Err(Fail::bad_request(
            "give two recordings: /api/diff?a=ID&b=ID",
        ));
    };
    let a = super::recordings::open(ctx, a)?;
    let b = super::recordings::open(ctx, b)?;
    Ok(outcome(crate::recordings::trace_diff_of(
        &a, &b, &ctx.paths,
    )?))
}
