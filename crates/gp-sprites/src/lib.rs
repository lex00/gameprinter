//! Full sprite capture (docs/capture.md, "Sprites"): every sprite frame a
//! game's sprite tables define and every frame its recordings show, merged
//! into one sprite set per game with each entity's clips by model state,
//! their timing, palette variants and coverage.
//!
//! - [`config`] reads a template's `sprites.toml`: the tables' shapes and
//!   where they are, and the RAM conditions for each model state.
//! - [`tables`] decodes the tables from the lifted files.
//! - [`chr`] finds which lifted tile a sprite shows, through CHR banks or
//!   CHR RAM.
//! - [`observe`] gives each sprite of each recorded frame to the entity
//!   the state mapping places under it.
//! - [`build`] merges the two into a [`gp_assets::sprite_set::SpriteSet`],
//!   clips and coverage.
//! - [`emit`] writes it into a model.
//!
//! Nothing in it is particular to one game: a game's specifics are its
//! `sprites.toml`, its state mapping and its asset map.

pub mod build;
pub mod chr;
pub mod config;
pub mod emit;
pub mod observe;
pub mod tables;

use std::collections::HashMap;

/// True when version `v` is before `at` (both `major.minor.patch`).
pub fn version_below(v: &str, at: &str) -> bool {
    !gp_model::validate::version_at_least(v, at)
}

/// The most seen lifted tile for each sprite tile number.
pub fn pattern_map(
    seen: &HashMap<u32, std::collections::BTreeMap<(String, u32), u32>>,
) -> HashMap<u32, (String, u32)> {
    seen.iter()
        .filter_map(|(k, v)| {
            v.iter()
                .max_by_key(|(t, n)| (**n, std::cmp::Reverse((*t).clone())))
                .map(|(t, _)| (*k, t.clone()))
        })
        .collect()
}

#[cfg(test)]
mod tests;
