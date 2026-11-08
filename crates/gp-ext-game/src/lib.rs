//! No game's hooks. A game's hook crate lives with the game, so this repo
//! holds none: the page's build puts one here, in a copy of the repo
//! (scripts/web/build-page.sh).

use gp_cycles as _;

/// No hooks.
pub fn exts() -> gp_sim::Exts {
    gp_sim::Exts::new()
}
