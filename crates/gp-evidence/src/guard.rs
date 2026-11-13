//! One guard that every reader of a recording or an input log passes
//! through (docs/contributing-mechanics.md, "Sealed runs").
//!
//! A program installs a [`Guard`] once at start. [`crate::recordings::load_manifest`],
//! the store's one way to open a recording, asks it about each recording it
//! opens, and the input-log readers of the tools ask it about each log they
//! read ([`input`]). A refusal is an error with the guard's message. With no
//! guard installed (a library test, a program that has no sealed runs)
//! nothing is refused.
//!
//! The guard is the place a rule about what no tool may read lives, so a new
//! command that opens recordings or input logs the usual way is held to it
//! without a check of its own. A reader that must see what the guard
//! refuses (the coordinator's scorer, the listing of leaks) calls
//! [`crate::recordings::load_manifest_unguarded`] and says why.

use std::sync::OnceLock;

use crate::hash::Hash;
use crate::input_log::InputLog;
use crate::manifest::RecordingManifest;
use crate::store::Store;

/// What decides whether a recording or an input log may be read.
pub trait Guard: Send + Sync {
    /// A stored recording, by its ID and manifest. `Err` holds the refusal.
    fn recording(
        &self,
        store: &Store,
        id: &Hash,
        manifest: &RecordingManifest,
    ) -> Result<(), String>;
    /// An input log from power-on, named `what` in a refusal, whose own
    /// frames start at `start`, played on the game `of` says.
    fn input(&self, what: &str, of: &Of, log: &InputLog, start: usize) -> Result<(), String>;
}

/// What game a recording or an input log is of, as far as its reader
/// knows. A sealed run applies only to plays of the same game: by
/// registered game ID when both are registered, by ROM hash when either
/// isn't, and by name when neither side knows a ROM. A play that says
/// nothing at all (the default) is held to every sealed run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Of {
    /// The game's name: its source registry ID, or for a game the registry
    /// doesn't list, the name its template or sealed folder goes by.
    pub game: Option<String>,
    /// The headerless SHA-256 of the ROM it plays on
    /// ([`crate::source::identify_nes`]).
    pub rom: Option<String>,
}

impl Of {
    /// A game by name.
    pub fn game(name: &str) -> Self {
        Self {
            game: Some(name.to_owned()).filter(|g| !g.is_empty()),
            rom: None,
        }
    }

    /// A ROM by its headerless SHA-256.
    pub fn rom(sha256: &str) -> Self {
        Self {
            game: None,
            rom: Some(sha256.to_owned()).filter(|r| !r.is_empty()),
        }
    }
}

static GUARD: OnceLock<Box<dyn Guard>> = OnceLock::new();

/// Installs the guard. Only the first call counts; it returns whether this
/// one did.
pub fn install(guard: Box<dyn Guard>) -> bool {
    GUARD.set(guard).is_ok()
}

/// Whether a guard is installed.
pub fn installed() -> bool {
    GUARD.get().is_some()
}

/// Asks the guard about a recording.
pub fn recording(store: &Store, id: &Hash, manifest: &RecordingManifest) -> Result<(), String> {
    GUARD
        .get()
        .map_or(Ok(()), |g| g.recording(store, id, manifest))
}

/// Asks the guard about an input log ([`Guard::input`]).
pub fn input(what: &str, of: &Of, log: &InputLog, start: usize) -> Result<(), String> {
    GUARD
        .get()
        .map_or(Ok(()), |g| g.input(what, of, log, start))
}
