//! Finding the gameprinter checkout: the working folder or one above it
//! that `has` says is it, else the checkout gp was built from, else the
//! files a released gp carries, unpacked (src/bundle.rs). Each caller
//! names its own marker file and its own sentence when there is none.
//! The page's own files (mechanic/, editor/dist/) come first from those
//! published with the running gp ([`page_files`]).

use std::cell::Cell;
use std::path::{Path, PathBuf};

/// Set (to anything but empty), the checkout gp was built from is treated
/// as absent, as it is for a gp installed away from its source (issue 46):
/// what the release's build job sets when it checks the archive it built.
pub(crate) const HIDE_BUILT: &str = "GP_HIDE_BUILT_SOURCE";

thread_local! {
    /// [`hide_built`]'s switch, for this thread: `gp self check` runs on
    /// one, and a test of it doesn't hide the checkout from other tests.
    static HIDDEN: Cell<bool> = const { Cell::new(false) };
}

/// Whether [`built`] answers none: [`hide_built`] is on, or
/// GP_HIDE_BUILT_SOURCE is set.
pub(crate) fn built_hidden() -> bool {
    HIDDEN.get() || std::env::var_os(HIDE_BUILT).is_some_and(|v| !v.is_empty())
}

/// While the returned guard lives, every lookup of the checkout gp was
/// built from on this thread answers none: [`built`], so
/// [`find_repo`]'s fallback, and the module kit's source
/// (crate::module). `gp self check` holds it for its whole run.
pub(crate) fn hide_built() -> HideBuilt {
    HideBuilt(HIDDEN.replace(true))
}

/// [`hide_built`]'s guard: dropped, the switch is as it was.
pub(crate) struct HideBuilt(bool);

impl Drop for HideBuilt {
    fn drop(&mut self) {
        HIDDEN.set(self.0);
    }
}

/// The checkout gp was built from, by the path it had at build time:
/// None under [`hide_built`]. On an installed gp the path is the build
/// machine's, so callers check that what they want is there.
pub(crate) fn built() -> Option<PathBuf> {
    if built_hidden() {
        return None;
    }
    Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
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
    if let Some(built) = built().filter(|b| has(b)) {
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
