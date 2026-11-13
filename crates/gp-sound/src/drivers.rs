//! Sound driver decoders and their registry (docs/sound.md, "Drivers").
//!
//! A driver is a sound engine several games share. Where a pinned
//! disassembly documents its data format, a decoder reads songs straight
//! from the game's music data (lifted through the asset map, never stored
//! in the repo) into the note representation, with `origin = "driver"`.
//! Log capture then cross-checks it, and the decoder's notes are trusted
//! where they agree. A new game whose engine is a registered driver
//! reuses the decoder: its asset map lifts the same asset format, and the
//! registry finds the driver by that format.
//!
//! The decoders take bytes, as the game's routines read them; the caller
//! encodes the lifted files back to bytes (`gp_assets::encode`). An arcade
//! driver whose music data isn't mapped takes the sound chips' register
//! log instead (format `ay-log`): its decoder reads the songs from what
//! the sound program wrote (gp-cli's sound_ay), so it works on any game
//! the board runs.
//!
//! game-printer's registry held Super Mario Bros.'s decoder and Mega Man
//! 2's and Super Mario Bros. 2's drivers; gameprinter's games are Donkey
//! Kong and Tapper, so those were cut and these two entered.

/// What the registry knows about one driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Driver {
    pub name: &'static str,
    /// The asset map format (or `blobs` asset name) its music data lifts
    /// as; `ay-log` for a driver decoded from an AY-8910 register log.
    pub format: &'static str,
    /// Games it's known in, by source ID.
    pub games: &'static [&'static str],
    /// Whether a decoder is written.
    pub decoder: bool,
    pub about: &'static str,
}

/// The format of a driver whose decoder reads an AY-8910 register log.
pub const AY_LOG: &str = "ay-log";

pub const REGISTRY: &[Driver] = &[
    Driver {
        name: "nintendo-dk",
        format: "dk-music",
        games: &["donkey-kong-ju", "donkey-kong-oe"],
        decoder: false,
        about: "Nintendo's sound engine of Donkey Kong (NES): no asset map lifts its music data yet, so there is no decoder; gp sound capture reads its songs from the APU log",
    },
    Driver {
        name: "midway-ssio",
        format: AY_LOG,
        games: &["tapper", "rbtapper", "tapperg", "sutapper", "tappera"],
        decoder: true,
        about: "Bally Midway's SSIO board: a sound Z80 driving two AY-8910s on the main CPU's sound latch. Its music data isn't mapped, so the decoder reads the songs from an AY register log (gp sound capture's ay.log): each run of sound between silences is a song",
    },
];

pub fn by_name(name: &str) -> Option<&'static Driver> {
    REGISTRY.iter().find(|d| d.name == name)
}

pub fn by_format(format: &str) -> Option<&'static Driver> {
    REGISTRY.iter().find(|d| d.format == format)
}

/// The driver a game (by source ID) is known to use.
pub fn by_game(source: &str) -> Option<&'static Driver> {
    REGISTRY.iter().find(|d| d.games.contains(&source))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registry_finds_drivers_by_name_format_and_game() {
        assert_eq!(by_game("tapper").map(|d| d.name), Some("midway-ssio"));
        assert!(by_format(AY_LOG).is_some_and(|d| d.decoder));
        assert!(by_name("nintendo-dk").is_some_and(|d| !d.decoder));
        assert!(by_game("smb1-ntsc-prg0").is_none());
        let mut names: Vec<_> = REGISTRY.iter().map(|d| d.name).collect();
        names.dedup();
        assert_eq!(names.len(), REGISTRY.len());
    }
}
