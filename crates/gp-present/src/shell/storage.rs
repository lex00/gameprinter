//! The shell's storage interface, `game-printer:shell@0.1.0`'s `storage`, as Rust:
//! generated from wit/shell/storage.wit by `gp docs shell`. Don't edit
//! it; change the WIT and run `just docs`.

/// The WIT package these bindings are of.
pub const WIT_PACKAGE: &str = "game-printer:shell@0.1.0";

/// A file the player handed over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picked {
    /// Its name as the player's system gave it.
    pub file_name: String,
    pub bytes: Vec<u8>,
}

/// What a target keeps for one game. Each function is the WIT's, with
/// its name in snake case.
pub trait Storage {
    /// The names kept that start with `prefix` (all of them for ""), sorted.
    fn list(&mut self, prefix: &str) -> Vec<String>;

    /// The file's bytes, or none when nothing is kept under the name.
    fn read(&mut self, name: &str) -> Option<Vec<u8>>;

    /// Keeps the bytes under the name, replacing what was there. Either the
    /// whole file is kept or none of it, so a crash never leaves half a save.
    fn write(&mut self, name: &str, bytes: &[u8]) -> Result<(), String>;

    /// Removes the file. A name that isn't kept is no error.
    fn delete(&mut self, name: &str) -> Result<(), String>;

    /// Asks that what is kept stays when the system wants room (a browser's
    /// navigator.storage.persist()). True when it is kept for good; a data
    /// folder always is.
    fn persist(&mut self) -> bool;

    /// Hands the kept file `name` to the player as a file called
    /// `file-name` (a browser downloads it; a native game writes it into its
    /// data folder's `exports/`), and says where it went.
    fn export(&mut self, name: &str, file_name: &str) -> Result<String, String>;

    /// A file the player picks (a browser's file chooser; a native game
    /// takes the newest file in its data folder's `imports/`), or none when
    /// there is none or the player cancels.
    fn import(&mut self) -> Result<Option<Picked>, String>;
}
