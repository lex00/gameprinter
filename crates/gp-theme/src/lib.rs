//! Themes (docs/formats/theme.md): a theme restyles a game by role without
//! touching its model. This crate has the theme and stack formats
//! ([`format`]), stacking ([`stack`]), the slot-name check ([`slots`]),
//! theme folders that point at another ([`same_as`]),
//! applying a stack to pictures whose pixels carry roles, the readability
//! check ([`readability`]), the Style
//! metrics ([`style`]), a frame compositor for runtimes without a GPU
//! ([`compose`]) and a role-based painter for generated placeholder art
//! ([`paint`]).
//!
//! Everything here is presentation. The simulation never sees a theme, so a
//! state hash can't depend on one; floats are fine.

pub mod color;
pub mod compose;
pub mod format;
pub mod paint;
pub mod particles;
pub mod pix;
pub mod readability;
pub mod same_as;
pub mod sfx;
pub mod slots;
pub mod stack;
pub mod style;
pub mod tune;

pub use format::{FORMAT_VERSION, Scope, Stack, Theme};
pub use stack::{Places, Resolved, RoleImage};

/// Every role a pixel can have: the entity roles, then the world roles not
/// already among them. A [`RoleImage`] stores a role as its index here.
pub const ROLES: &[&str] = &[
    "player",
    "enemy",
    "boss",
    "projectile",
    "npc",
    "collectible",
    "power-up",
    "hazard",
    "interactive",
    "goal",
    "terrain",
    "platform",
    "background",
    "sky",
    "water",
    "hud",
    "menu",
    "font",
];

/// A pixel with no role (transparent, or art the model gives no role).
pub const NO_ROLE: u8 = u8::MAX;

/// A role's index in [`ROLES`], or [`NO_ROLE`].
pub fn role_id(role: &str) -> u8 {
    ROLES
        .iter()
        .position(|r| *r == role)
        .map_or(NO_ROLE, |i| i as u8)
}

/// True when a reader of [`FORMAT_VERSION`] reads `v`: the same major and
/// minor, a patch no newer.
pub fn version_supported(v: &str) -> bool {
    gp_model::validate::version_supported(v, FORMAT_VERSION)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gp_model::validate::{ENTITY_ROLES, WORLD_ROLES};

    #[test]
    fn roles_are_the_models() {
        for r in ENTITY_ROLES.iter().chain(WORLD_ROLES) {
            assert_ne!(role_id(r), NO_ROLE, "{r}");
        }
        assert_eq!(ROLES.len(), ENTITY_ROLES.len() + WORLD_ROLES.len());
    }
}
