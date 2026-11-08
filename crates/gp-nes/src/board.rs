//! Which image the console loads: the registry's board facts for a
//! registered source, the file's header for anything else (docs/sources.md,
//! "Junk around a dump", and ARCHITECTURE.md).
//!
//! The registry is compiled in from roms/nes.toml, so a build of gp-nes
//! boots a given ROM one way whatever registry `gp` is pointed at. A file
//! whose header contradicts its source's facts is refused; `gp roms clean`
//! writes a copy with the registry's header. Junk the header rules ignore
//! (archaic iNES bytes 7 to 15, a trainer, trailing bytes) is dropped before
//! tetanes-core sees the file, so a registered game boots the same from a
//! junk dump as from a clean one.

use std::borrow::Cow;
use std::path::Path;
use std::sync::LazyLock;

use gp_evidence::dump::{self, DumpError};
use gp_evidence::registry::{BoardFacts, Registry};

static REGISTRY: LazyLock<Registry> = LazyLock::new(|| {
    Registry::from_toml(
        include_str!("../../../roms/nes.toml"),
        Path::new("roms/nes.toml"),
    )
    .unwrap_or_else(|e| panic!("the compiled-in source registry doesn't load: {e}"))
});

/// The compiled-in registry (roms/nes.toml as of this build).
pub fn registry() -> &'static Registry {
    &REGISTRY
}

/// The board facts of the registered source with this headerless SHA-256.
pub fn facts(sha256: &str) -> Option<BoardFacts> {
    REGISTRY.by_sha256(sha256)?.board_facts()
}

/// The image to load for `rom`: header, PRG and CHR, with the header from
/// the registry for a registered source and from the file otherwise. A
/// clean file of an unregistered ROM comes back unchanged.
pub fn runnable(rom: &[u8]) -> Result<Cow<'_, [u8]>, DumpError> {
    let image = gp_evidence::source::identify_nes(rom)?;
    dump::runnable(rom, facts(&image.sha256).as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tool lists no game's board facts: a game's live in its package.
    #[test]
    fn the_tool_lists_no_games_board() {
        assert!(registry().sources.is_empty());
    }
}
