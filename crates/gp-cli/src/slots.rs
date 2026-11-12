//! `gp interp slots`: the model's enemies (their components and `ext`
//! hooks) measured against the original's enemy slots, one slot life at a
//! time (gp_interp::compare::compare_slots; docs/formats/state-mapping.md,
//! "Slots").
//!
//! Ported from game-printer's interp.rs (`slots`). The original starts
//! through [`OriginalRuntime`], so any platform's original runs; the
//! bodies are stepped by the model's hooks wherever they are: in gp for a
//! model whose hooks gp has, or in the game's module (`--module`, its
//! `step-hooks` call) for one whose hooks are only there. The run recorder's
//! metrics are gone with the recorder.

use std::path::PathBuf;

use anyhow::{Result, bail};
use gp_evidence::Paths;
use serde_json::json;

use crate::Outcome;
use crate::interp_args::{self as ia, InputArgs};
use crate::original::OriginalRuntime;

#[derive(clap::Args)]
pub struct SlotsArgs {
    /// A model directory holding model.toml and mapping.toml.
    pub model: PathBuf,
    /// The original game's ROM.
    #[arg(long)]
    pub rom: PathBuf,
    #[command(flatten)]
    pub input: InputArgs,
    /// The game's built module. Needed for a model with hooks.
    #[arg(long)]
    pub module: Option<PathBuf>,
    /// The model's lifted assets (default:
    /// $GP_HOME/work/assets/<model.source>).
    #[arg(long, value_name = "DIR")]
    pub assets: Option<PathBuf>,
    /// The state mapping, instead of the model directory's mapping.toml.
    #[arg(long, value_name = "FILE")]
    pub mapping: Option<PathBuf>,
    /// Run only this many frames of the input.
    #[arg(long)]
    pub frames: Option<usize>,
    /// Print every compared value of this entity's lives to stderr, a
    /// line a frame: the frame, the value, both sides and the body's
    /// ext words.
    #[arg(long, value_name = "ENTITY")]
    pub trace: Option<String>,
}

pub(crate) fn run(a: &SlotsArgs, paths: &Paths) -> Result<Outcome> {
    let mut input = a.input.read()?;
    if let Some(n) = a.frames {
        input.frames.truncate(n);
    }
    let (source, _) = ia::source(&a.model, a.assets.as_deref(), a.module.as_deref(), paths)?;
    let (mapping_file, mapping) = ia::load_mapping(&a.model, a.mapping.as_deref())?;
    let rom = ia::read_rom(&a.rom)?;
    let mut original = OriginalRuntime::of_rom(&rom)?;
    let spec = &source.spec;
    // Every mapped entity's hook must run: a missing one is refused here.
    if let Some(slots) = &mapping.slots {
        for k in &slots.kinds {
            let kind = spec.kinds.iter().find(|x| x.name == k.entity);
            if let Some(name) = kind.and_then(|x| x.ext.as_ref())
                && !source.exts.contains_key(name)
            {
                bail!(
                    "the slots map `{}`, whose ext hook `{name}` neither gp nor the module has",
                    k.entity
                );
            }
        }
    }
    ia::same_model(&mapping_file, &mapping, &spec.name)?;
    let stepper = source.open()?;
    let trace = a.trace.as_deref();
    let reports = gp_interp::compare::compare_slots_with(
        &mut original,
        &mapping,
        &crate::run::frames(&input),
        spec,
        &mut |w| stepper.step_hooks(w).map_err(|e| format!("{e:#}")),
        &mut |row| {
            if trace == Some(row.entity) {
                eprintln!(
                    "{} {} {}: original {}, model {}, words {:x?}",
                    row.frame, row.entity, row.field, row.original, row.model, row.words
                );
            }
        },
    )
    .map_err(anyhow::Error::msg)?;
    let mut lines = vec![format!(
        "{} frames of the original; each slot life beside one body of its entity",
        input.frames.len()
    )];
    let mut entities = Vec::new();
    for r in &reports {
        lines.push(format!(
            "{}: {} lives, {} frames",
            r.entity, r.lives, r.frames
        ));
        let measures = r.measures();
        let mut fields = Vec::new();
        for f in &r.fields {
            let m: Vec<_> = measures.iter().filter(|m| m.field == f.path).collect();
            let mut text = format!(
                "  {:6} {} of {} frames within a pixel, at most {} off",
                f.path,
                f.agreed,
                f.compared,
                gp_sim::Fx::from_raw(f.max_error)
            );
            for m in &m {
                text.push_str(&format!(
                    "; {} {} vs {}{}",
                    m.what,
                    m.original,
                    m.model,
                    m.off_bp()
                        .map(|bp| format!(" ({}.{:02}% off)", bp / 100, bp % 100))
                        .unwrap_or_default()
                ));
            }
            lines.push(text);
            fields.push(json!({
                "path": f.path,
                "compared": f.compared,
                "agreed": f.agreed,
                "max_error": gp_sim::Fx::from_raw(f.max_error).to_string(),
                "measures": m.iter().map(|m| json!({
                    "what": m.what,
                    "original": m.original.to_string(),
                    "model": m.model.to_string(),
                    "off_bp": m.off_bp(),
                })).collect::<Vec<_>>(),
            }));
        }
        entities.push(json!({
            "entity": r.entity,
            "lives": r.lives,
            "frames": r.frames,
            "fields": fields,
        }));
    }
    lines.push(format!("ext hooks: {}", source.hooks));
    Ok(Outcome::ok(
        json!({
            "model": spec.name,
            "frames": input.frames.len(),
            "ext_hooks": source.hooks,
            "entities": entities,
        }),
        lines.join("\n"),
    ))
}
