//! Finding the gameprinter checkout: the working folder or one above it
//! that `has` says is it, else the checkout gp was built from. Each caller
//! names its own marker file and its own sentence when there is none.

use std::path::{Path, PathBuf};

/// The checkout gp was built from.
pub(crate) fn built() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The working folder or the nearest one above it that `has` accepts,
/// else the checkout gp was built from if `has` accepts that.
pub(crate) fn find_repo(has: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    if let Ok(here) = std::env::current_dir()
        && let Some(d) = here.ancestors().find(|d| has(d))
    {
        return Some(d.to_path_buf());
    }
    let built = built();
    has(&built).then_some(built)
}
