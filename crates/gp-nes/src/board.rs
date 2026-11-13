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

    /// How many frames a mapper-set mirroring bit may take to stop mattering:
    /// Super Mario Bros. 2 writes MMC3's mirroring register by frame 4.
    const MIRRORING_SETTLES_BY: usize = 8;

    /// A copy of `raw` with junk a dump can carry: archaic bytes in the
    /// header, a trainer, or bytes after the game data.
    fn junk(raw: &[u8], kind: &str) -> Vec<u8> {
        let (header, data) = raw.split_at(16);
        let mut h = header.to_vec();
        let mut out = Vec::new();
        match kind {
            "diskdude" => h[7..16].copy_from_slice(b"DiskDude!"),
            "trainer" => {
                h[6] |= 0x04;
                out.extend(std::iter::repeat_n(0xEE, 512));
            }
            "trailing" => {}
            other => panic!("{other}"),
        }
        let mut rom = h;
        rom.extend(out);
        rom.extend_from_slice(data);
        if kind == "trailing" {
            rom.extend(std::iter::repeat_n(0xFF, 4096));
        }
        rom
    }

    /// The state hash after each frame of `input` from power-on.
    fn hashes(nes: &mut crate::Nes, input: &gp_evidence::input_log::InputLog) -> Vec<String> {
        let mut out = vec![format!("{:?}", nes.state_hash().unwrap())];
        for f in &input.frames {
            nes.step(*f).unwrap();
            out.push(format!("{:?}", nes.state_hash().unwrap()));
        }
        out
    }

    /// ROM tier (`just check-roms`, which `just check` runs, sets
    /// GP_CHECK_ROM_BOOT): each NES ROM a game in the games checkout names
    /// ($GP_GAMES_DIR, or `games` beside the repo), found in $GP_ROM_DIR,
    /// boots in gp-nes exactly as tetanes-core boots the file as it is, frame
    /// by frame, and a copy of it with each kind of junk around the dump
    /// records the same hashes as the file. Skips, with a notice, what this
    /// machine doesn't have. game-printer's test read its compiled-in
    /// registry; the games checkout holds the sources now, and the junk
    /// copies were scripts/romjunk/check.py's half.
    #[test]
    fn registered_roms_boot_as_their_files_do() {
        if std::env::var_os("GP_CHECK_ROM_BOOT").is_none() {
            return;
        }
        let Some(dir) = std::env::var_os("GP_ROM_DIR") else {
            eprintln!("GP_ROM_DIR isn't set; skipped");
            return;
        };
        let games = std::env::var_os("GP_GAMES_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games"));
        let Ok(sources) = Registry::load_games(&games) else {
            eprintln!("no games checkout at {}; skipped", games.display());
            return;
        };
        let index = gp_evidence::source::index_nes_roms(Path::new(&dir)).unwrap_or_default();
        let input = gp_evidence::input_log::generate(
            600,
            &["start@60-65", "start@200-205", "a+right@300-"],
        )
        .unwrap();
        let power_on = gp_evidence::manifest::PowerOn::default();
        let mut checked = 0;
        for source in &sources.sources {
            let Some(path) = index.get(&source.sha256) else {
                continue;
            };
            let raw = std::fs::read(path).unwrap();
            if dump::runnable(&raw, source.board_facts().as_ref()).is_err() {
                eprintln!(
                    "{}: skipped, its header contradicts the registry",
                    source.id
                );
                continue;
            }
            let mut ours = crate::Nes::power_on(&raw, power_on).unwrap();
            let mut deck = tetanes_core::control_deck::ControlDeck::with_config(
                crate::config(power_on).unwrap(),
            );
            if deck.load_rom("game", &mut &raw[..]).is_err() {
                eprintln!(
                    "{}: skipped, tetanes-core refuses the file as it is",
                    source.id
                );
                continue;
            }
            let mut file = crate::Nes {
                deck,
                frame: 0,
                events: false,
            };
            let registry_flags6 = source
                .board_facts()
                .and_then(|f| dump::header_from_facts(&f).ok())
                .map_or(raw[6], |h| h[6]);
            let settle = if raw[6] & 1 != registry_flags6 & 1 {
                MIRRORING_SETTLES_BY
            } else {
                0
            };
            let mine = hashes(&mut ours, &input);
            let theirs = hashes(&mut file, &input);
            for frame in settle..mine.len() {
                assert_eq!(
                    mine[frame], theirs[frame],
                    "{} differs from the file as tetanes-core boots it at frame {frame}",
                    source.id
                );
            }
            for kind in ["diskdude", "trainer", "trailing"] {
                let copy = junk(&raw, kind);
                let mut nes = crate::Nes::power_on(&copy, power_on).unwrap();
                assert_eq!(
                    hashes(&mut nes, &input),
                    mine,
                    "{} with {kind} junk records other hashes than the file",
                    source.id
                );
            }
            eprintln!(
                "{}: identical to the file from frame {settle} to frame {}, and so are its junk copies",
                source.id,
                input.frames.len()
            );
            checked += 1;
        }
        if checked == 0 {
            eprintln!("no NES ROM a game names in $GP_ROM_DIR to check; skipped");
        }
    }
}
