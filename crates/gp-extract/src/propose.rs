//! Measured feel as proposed model facts (docs/mechanics.md, "Proposals"):
//! each measured parameter by its model path, rounded to its grid, with the
//! samples behind it, the fact the model states now, whether the two agree
//! within the quantity's tolerance, and the evidence it would be written
//! with. Where the samples disagree among themselves, or a confident
//! measurement disagrees with a stated fact, or a track can't be told to
//! be one entity, a question goes to the person instead.
//!
//! Only [`steps`] is ported yet; the proposals arrive with gp-track and
//! gp-extract's `feel` in a later wave.

use gp_model::path::{Path, Step};

/// The steps of a concrete path such as `entities.player.jump.bands[0].speed`.
pub fn steps(path: &str) -> Result<Vec<Step>, String> {
    let p = Path::parse(path)?;
    let mut out = Vec::new();
    for s in p.segments {
        out.push(Step::Key(s.name));
        match s.index {
            None => {}
            Some(gp_model::path::Index::At(i)) if i >= 0 => out.push(Step::Item(i as usize)),
            Some(_) => return Err(format!("`{path}` isn't a concrete path")),
        }
    }
    Ok(out)
}
