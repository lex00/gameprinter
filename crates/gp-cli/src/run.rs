//! Runs of the original and of a model: baselines, and the comparison of
//! the two through the model's state mapping.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use gp_evidence::baseline::Baseline;
use gp_evidence::source::identify_nes;
use gp_evidence::{InputLog, Store};
use gp_interp::mapping::{MAPPING_FILE, Mapping};
use gp_interp::{Bundle, Interp, Lifted};
use gp_model::Model;
use gp_runtime::InputFrame;
use serde_json::json;

use crate::Outcome;

pub(crate) fn frames(input: &InputLog) -> Vec<InputFrame> {
    input
        .frames
        .iter()
        .map(|f| InputFrame {
            buttons: f.buttons_word(),
            axes: Vec::new(),
            reset: f.reset,
        })
        .collect()
}

/// A model's run through its module, each frame's state hash written out.
pub fn module_run(
    dir: &Path,
    module: &Path,
    assets: &Path,
    input: &Path,
    hashes: &Path,
) -> Result<Outcome> {
    let file = dir.join("model.toml");
    let text = fs::read_to_string(&file).with_context(|| file.display().to_string())?;
    let model = Model::parse(&text).with_context(|| file.display().to_string())?;
    let lifted = Lifted::read_dir(assets, &model).map_err(|e| anyhow!("{e}"))?;
    let bundle = Bundle::new(&model, lifted);
    let wasm = fs::read(module).with_context(|| module.display().to_string())?;
    let mut rt =
        gp_module::ModuleRuntime::load(&wasm, &bundle.to_bytes()).map_err(|e| anyhow!("{e}"))?;
    let log =
        InputLog::parse(&fs::read_to_string(input).with_context(|| input.display().to_string())?)
            .map_err(|e| anyhow!("{}: {e}", input.display()))?;
    let hex = |rt: &gp_module::ModuleRuntime| -> Result<String> {
        Ok(hex::encode(rt.world_hash().map_err(|e| anyhow!("{e}"))?))
    };
    let mut out = String::new();
    out.push_str(&hex(&rt)?);
    out.push('\n');
    let start = std::time::Instant::now();
    let inputs = frames(&log);
    for f in &inputs {
        gp_runtime::Runtime::step(&mut rt, std::slice::from_ref(f)).map_err(|e| anyhow!("{e}"))?;
        out.push_str(&hex(&rt)?);
        out.push('\n');
    }
    let secs = start.elapsed().as_secs_f64();
    fs::write(hashes, out).with_context(|| hashes.display().to_string())?;
    Ok(Outcome {
        json: json!({ "frames": inputs.len(), "seconds": secs, "hooks": rt.label }),
        text: format!(
            "ran {} frames through the module in {secs:.2} s; hooks: {}",
            inputs.len(),
            rt.label
        ),
        exit: 0,
    })
}

pub fn baseline_check(files: &[PathBuf], rom_path: &Path) -> Result<Outcome> {
    let mut list = Vec::new();
    for f in files {
        if f.is_dir() {
            list.extend(gp_evidence::baseline::list(f)?);
        } else {
            list.push(f.clone());
        }
    }
    if list.is_empty() {
        bail!("no baselines to check");
    }
    let rom = fs::read(rom_path).with_context(|| rom_path.display().to_string())?;
    let sha = identify_nes(&rom)?.sha256;
    let (mut lines, mut failed) = (Vec::new(), 0u64);
    for file in &list {
        let baseline = Baseline::load(file)?;
        if baseline.sha256 != sha {
            bail!(
                "{} has SHA-256 {sha}, but {} expects {}",
                rom_path.display(),
                file.display(),
                baseline.sha256
            );
        }
        let name = file
            .file_stem()
            .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
        let tmp = tempfile::tempdir()?;
        let store = Store::open(tmp.path())?;
        let run = gp_nes::baseline::run(&store, &baseline, &name, &rom, false)?;
        let failures = run.failures(&baseline);
        if failures.is_empty() {
            lines.push(format!(
                "ok     {}  {} frames",
                file.display(),
                baseline.frames
            ));
        } else {
            failed += 1;
            lines.push(format!("FAILED {}", file.display()));
            lines.extend(failures.iter().map(|f| format!("         {f}")));
        }
    }
    lines.push(format!("{} baselines, {failed} failed", list.len()));
    Ok(Outcome {
        json: json!({ "baselines": list.len(), "failed": failed }),
        text: lines.join("\n"),
        exit: u8::from(failed > 0),
    })
}

pub fn compare(dir: &Path, rom_path: &Path, baseline: &Path, assets: &Path) -> Result<Outcome> {
    let file = dir.join("model.toml");
    let text = fs::read_to_string(&file).with_context(|| file.display().to_string())?;
    let model = Model::parse(&text).with_context(|| file.display().to_string())?;
    let lifted = Lifted::read_dir(assets, &model).map_err(|e| anyhow!("{e}"))?;
    let bundle = Bundle::new(&model, lifted);
    let mapping_file = dir.join(MAPPING_FILE);
    let mapping = Mapping::parse(
        &fs::read_to_string(&mapping_file).with_context(|| mapping_file.display().to_string())?,
    )
    .map_err(|e| anyhow!("{}: {e}", mapping_file.display()))?;
    let input = Baseline::load(baseline)?.input()?;
    let rom = fs::read(rom_path).with_context(|| rom_path.display().to_string())?;
    let mut original = crate::original::OriginalRuntime::of_rom(&rom)?;
    let interp = Interp::from_bundle(&bundle).map_err(|e| anyhow!("{e}"))?;
    let (exts, _) = gp_exts::registry(&interp.spec().name);
    let mut interp = interp.with_exts(std::sync::Arc::new(exts));
    let missing = interp.ext_hooks();
    if !missing.is_empty() {
        bail!(
            "the model names hooks this gp has no crate for: {}",
            missing.join(", ")
        );
    }
    let report = gp_interp::compare::compare(&mut original, &mut interp, &mapping, &frames(&input))
        .map_err(|e| anyhow!("{e}"))?;
    let agreeing = report.fields.iter().filter(|f| f.first.is_none()).count();
    let mut lines: Vec<String> = report
        .fields
        .iter()
        .filter_map(|f| {
            f.first
                .as_ref()
                .map(|d| format!("DIFFERS  {}  first at model frame {}", f.path, d.frame))
        })
        .collect();
    lines.push(format!(
        "{agreeing} of {} mapped paths agree on every frame",
        report.fields.len()
    ));
    Ok(Outcome {
        json: json!({ "agrees": report.agrees(), "paths": report.fields.len(), "agreeing": agreeing }),
        text: lines.join("\n"),
        exit: u8::from(!report.agrees()),
    })
}
