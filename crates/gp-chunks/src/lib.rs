//! The level chunk catalog: the groups of cells a game's levels are built
//! from, captured in two layers (the format's own object vocabulary, and
//! patterns mined from the decoded grids of any format), with what each
//! does for the player and how the pieces sit together (docs/chunks.md,
//! docs/formats/chunks.md). It detects and catalogs; a level generator that
//! uses the catalog comes later.

pub mod demand;
pub mod format;
pub mod level;
pub mod mine;
pub mod source;

#[cfg(test)]
mod tests;

pub use format::{Catalog, Chunk, FORMAT, Layer, VERSION};
pub use level::{LevelGrid, Object, Placed};
pub use mine::{Interactive, Mined, Options, Tester, mine};

/// The interactive element kinds of a gaps catalog (`interactive.json`,
/// docs/formats/interactive.md) whose cell has tiles before it changes.
pub fn interactive_kinds(json: &serde_json::Value) -> Vec<Interactive> {
    let mut out = Vec::new();
    for k in json["kinds"].as_array().into_iter().flatten() {
        let (Some(kind), Some(before)) = (k["kind"].as_str(), k["cell"]["before"].as_str()) else {
            continue;
        };
        // Eight hex digits, the four tile numbers.
        let t: Vec<u8> = (0..before.len() / 2)
            .filter_map(|i| u8::from_str_radix(before.get(2 * i..2 * i + 2)?, 16).ok())
            .collect();
        if before.len() == 8
            && let Ok(before) = <[u8; 4]>::try_from(t.as_slice())
        {
            out.push(Interactive {
                kind: kind.to_owned(),
                before,
            });
        }
    }
    out
}
