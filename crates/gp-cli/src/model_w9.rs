//! `gp model diff`, as game-printer's model.rs had it (wave 9 of the port).
//! It reads model files only, so it works the same for an NES game and an
//! arcade game. `gp model schema` is crate::schemas (wave 10).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use gp_model::Model;
use serde_json::json;

use crate::Outcome;

fn load(arg: &Path) -> Result<(PathBuf, Model)> {
    let file = crate::mechanics::model_file(arg);
    let text = fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
    let model = Model::parse(&text).with_context(|| file.display().to_string())?;
    Ok((file, model))
}

pub(crate) fn diff(a: &Path, b: &Path) -> Result<Outcome> {
    let (fa, ma) = load(a)?;
    let (fb, mb) = load(b)?;
    let changes = gp_model::diff::diff(&ma.to_tree(), &mb.to_tree());
    let text = if changes.is_empty() {
        format!("{} and {}: the same model", fa.display(), fb.display())
    } else {
        let lines: Vec<String> = changes.iter().map(ToString::to_string).collect();
        lines.join("\n")
    };
    Ok(Outcome {
        json: json!({
            "a": fa,
            "b": fb,
            "same": changes.is_empty(),
            "changes": changes,
        }),
        text,
        exit: if changes.is_empty() { 0 } else { 1 },
    })
}
