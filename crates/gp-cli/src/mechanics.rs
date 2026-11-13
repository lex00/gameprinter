//! `gp model mechanics`: a model's mechanic catalog (docs/mechanics.md),
//! generated from the records the simulation's components and the model's
//! hooks declare (`gp_interp::mechanics`).
//!
//! Ported from game-printer's mechanics.rs. gp links no game's hook crate,
//! so a model whose hooks are in its game's module gives their records
//! through the module (`--module`), as every other tool here runs them;
//! without it, its hooks are listed as having no record.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Args;
use gp_evidence::Paths;
use serde_json::json;

use gp_model::Model;

use crate::Outcome;

#[derive(Args)]
pub struct MechanicsArgs {
    /// A model folder (holding model.toml) or a model.toml file.
    pub dir: PathBuf,
    /// Also list every component record, used or not.
    #[arg(long)]
    pub all: bool,
    /// Also list every model path with its class and mechanic.
    #[arg(long)]
    pub paths: bool,
    /// The game's built module, whose hooks describe themselves. Needed
    /// for a model with hooks to have their records.
    #[arg(long)]
    pub module: Option<PathBuf>,
    /// With --module: the model's lifted assets (default:
    /// $GP_HOME/work/assets/SOURCE).
    #[arg(long, requires = "module")]
    pub assets: Option<PathBuf>,
}

pub(crate) fn model_file(arg: &Path) -> PathBuf {
    if arg.is_dir() {
        arg.join(gp_model::MODEL_FILE)
    } else {
        arg.to_path_buf()
    }
}

pub(crate) fn load_model(dir: &Path) -> Result<Model> {
    let file = model_file(dir);
    let text = fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
    Model::parse(&text).with_context(|| file.display().to_string())
}

/// Prints the catalog. Exits 1 when the model and a record disagree.
pub(crate) fn run(a: &MechanicsArgs, gp: &Paths) -> Result<Outcome> {
    let (all, paths) = (a.all, a.paths);
    let model = load_model(&a.dir)?;
    let (exts, label) = match &a.module {
        Some(m) => {
            let dir = if a.dir.is_dir() {
                a.dir.clone()
            } else {
                a.dir.parent().unwrap_or(Path::new(".")).to_owned()
            };
            let (source, _) =
                crate::hosted::Source::open_dir(&dir, a.assets.as_deref(), gp, Some(m))?;
            let label = format!("module {}", &source.hooks[..source.hooks.len().min(12)]);
            (source.exts, label)
        }
        None => {
            let (e, l) = gp_exts::registry(&model.model.name);
            (std::sync::Arc::new(e), l)
        }
    };
    let catalog = gp_interp::mechanics::catalog(&model, &exts);
    let mut t = String::new();
    let _ = writeln!(
        t,
        "{}: {} mechanics (catalog {}; hooks {})",
        model.model.name,
        catalog.mechanics.len(),
        catalog.catalog,
        if label.is_empty() { "none" } else { &label }
    );
    for m in &catalog.mechanics {
        let params: Vec<String> = m
            .params
            .iter()
            .map(|p| {
                if p.unit.is_empty() {
                    p.name.to_owned()
                } else {
                    format!("{} ({})", p.name, p.unit)
                }
            })
            .collect();
        let _ = writeln!(
            t,
            "  {:<28} {:<10} {:<20} {}{}",
            m.name,
            m.class,
            m.system,
            match m.used_by.len() {
                1 => m.used_by[0].clone(),
                n => format!("{n} uses"),
            },
            if m.like.is_empty() {
                String::new()
            } else {
                format!("  like {}", m.like)
            }
        );
        if !params.is_empty() {
            let _ = writeln!(t, "      params: {}", params.join(", "));
        }
        let _ = writeln!(t, "      built from: {}", m.built_from.join(", "));
    }
    let counts: Vec<String> = catalog
        .counts
        .iter()
        .map(|(c, n)| format!("{n} {c}"))
        .collect();
    let _ = writeln!(t, "paths: {}", counts.join(", "));
    if !catalog.debt.is_empty() {
        let _ = writeln!(
            t,
            "debt: {} hook records are game-specific code, not yet a general mechanic:",
            catalog.debt.len()
        );
        for d in &catalog.debt {
            let _ = writeln!(t, "  {} ({}): {}", d.hook, d.record, d.reason);
        }
    }
    for p in &catalog.problems {
        let _ = writeln!(t, "{}: {}: {}", p.severity, p.path, p.message);
    }
    if paths {
        for c in &catalog.paths {
            let _ = writeln!(
                t,
                "  {:<12} {:<50} {}",
                c.class.name(),
                c.path,
                c.mechanic.as_deref().unwrap_or("")
            );
        }
    }
    let mut json = serde_json::to_value(&catalog)?;
    if !paths {
        json.as_object_mut().map(|o| o.remove("paths"));
    }
    if all {
        json["components"] = json!(gp_interp::mechanics::all_components());
    }
    let exit = u8::from(catalog.errors() > 0);
    Ok(Outcome {
        json,
        text: t.trim_end().to_owned(),
        exit,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Moss Hop's model has no hooks: its catalog lists its mechanics and
    /// finds no problem.
    #[test]
    fn mosshops_catalog_has_no_errors() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/mosshop");
        let gp = Paths {
            home: dir.clone(),
            store: dir.join("store"),
            roms: dir.join("roms"),
        };
        let o = run(
            &MechanicsArgs {
                dir: dir.clone(),
                all: true,
                paths: true,
                module: None,
                assets: None,
            },
            &gp,
        )
        .unwrap();
        assert_eq!(o.exit, 0, "{}", o.text);
        assert!(!o.json["mechanics"].as_array().unwrap().is_empty());
        assert!(o.json["components"].is_array());
        assert!(o.json["paths"].is_array());
        assert!(o.text.contains("hooks none"), "{}", o.text);
    }
}
