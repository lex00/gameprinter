//! Finding the gameprinter checkout: the working folder or one above it
//! that `has` says is it, else the checkout gp was built from, else the
//! files a released gp carries, unpacked (src/bundle.rs). Each caller
//! names its own marker file and its own sentence when there is none.
//! The page's own files (mechanic/, editor/dist/) come first from those
//! published with the running gp ([`page_files`]).

use std::path::{Path, PathBuf};

/// The checkout gp was built from.
pub(crate) fn built() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The working folder or the nearest one above it that `has` accepts,
/// else the checkout gp was built from if `has` accepts that, else the
/// carried files' folder if `has` accepts that.
pub(crate) fn find_repo(has: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    if let Ok(here) = std::env::current_dir()
        && let Some(d) = here.ancestors().find(|d| has(d))
    {
        return Some(d.to_path_buf());
    }
    let built = built();
    if has(&built) {
        return Some(built);
    }
    crate::bundle::root().filter(|r| has(r))
}

/// The page's files published with the running gp: `bin/gp-<commit>.pages/`
/// beside the build `bin/gp-<commit>` this process runs (crate::publish
/// copies them there when the tool gate lands). None for a gp that isn't
/// a published build, or one published without them.
pub(crate) fn published_pages() -> Option<PathBuf> {
    let exe = std::fs::canonicalize(std::env::current_exe().ok()?).ok()?;
    pages_beside(&exe)
}

/// `exe`'s published page files, when there are some.
pub(crate) fn pages_beside(exe: &Path) -> Option<PathBuf> {
    let stem = exe.file_stem()?.to_str()?;
    if !stem.starts_with("gp-") {
        return None;
    }
    let dir = exe.with_file_name(format!("{stem}.pages"));
    dir.join("mechanic").is_dir().then_some(dir)
}

/// The folder `rel` of the page's files (`mechanic`, `editor/dist`): the
/// one published with this gp when it has it, else `repo`'s. A published
/// gp was built in the tool gate's tree, which holds whatever patch the
/// gate is trying, so its page changes only when a landing publishes.
pub(crate) fn page_files(repo: &Path, rel: &str) -> PathBuf {
    published_pages()
        .map(|d| d.join(rel))
        .filter(|d| d.is_dir())
        .unwrap_or_else(|| repo.join(rel))
}
