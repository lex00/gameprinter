//! The published gp: `$GP_HOME/bin/gp`, a link to `$GP_HOME/bin/gp-<commit>`,
//! which the tool gate moves as it lands patches (docs/gate.md, "Published
//! builds"), and `gp self path`, which names the file behind the link.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::Subcommand;
use gp_evidence::Paths;
use serde_json::json;

use crate::Outcome;
use crate::gate::git;

/// How many earlier builds are kept beside the linked one.
const KEEP: usize = 8;

#[derive(Subcommand)]
pub enum SelfCmd {
    /// Print the newest published build: the file $GP_HOME/bin/gp points
    /// at, which the tool gate keeps at the newest commit it built gp for.
    /// A worker that isn't changing Rust runs this build instead of
    /// building gp. Exits 1 when none is published yet.
    Path,
    /// Publish this binary as the build of the commit checked out here, as
    /// the tool gate does after a landing. For a first setup, before any
    /// gate has run: `just publish`. Refuses a tree with uncommitted changes.
    Publish,
    /// Write the command list, CLI.md, from the commands gp has. With
    /// --check, fail if the file differs.
    Docs {
        /// The file to write or check (default CLI.md here).
        #[arg(default_value = "CLI.md")]
        file: PathBuf,
        #[arg(long)]
        check: bool,
    },
    /// Write every format's JSON Schema into DIR, one file per format with
    /// no version in its name, as schema/README.md says. `just docs` runs
    /// it on the schema/ folder.
    Schemas {
        /// The folder to write or check (default schema here).
        #[arg(default_value = "schema")]
        dir: PathBuf,
        /// Write nothing; exit 1 if a file is stale or missing, or a JSON
        /// file in DIR is no format's.
        #[arg(long)]
        check: bool,
    },
    /// Print the named features this build knows as Markdown (the source
    /// of docs/features.md, which `just docs` writes).
    Features,
    /// Print every command this gp has, from its own command tree: each
    /// one's words, help, and arguments and flags with their kinds, for
    /// `gp mcp`'s gp_commands and gp_run (docs/mcp.md, "Every command").
    Commands,
    /// Write the runtime shell's generated files (docs/shell.md): the
    /// storage interface's bindings from wit/shell/storage.wit (a Rust
    /// trait in gp-present and the JavaScript host's check) and the web
    /// preview's copy of the key table (scripts/web/page/shell-keys.js).
    /// Run in the repo; `just docs` runs it.
    Shell {
        /// Write nothing; exit 1 if a file is stale.
        #[arg(long)]
        check: bool,
    },
    /// Say what works on this machine and what doesn't, in plain words:
    /// gp's home, an agent (whether one connected to gp mcp this week,
    /// which agents this machine has, and the line that connects one), your
    /// ROMs, a games folder, curl, git, Rust, the build sandbox, Node, a
    /// headless browser and MAME, each with what needs it and how to get
    /// it; then it uses what a released gp carries, from its bundle alone
    /// with the checkout gp was built from hidden: the page's files, the
    /// templates, the module kit, a module built from it (when Rust and the
    /// sandbox are here) and the page's server answering on a free port,
    /// each ready, failed with the path it tried, or skipped and why; then
    /// whether this machine is ready to make a theme, make a game, or port
    /// one. Writes only under $GP_HOME. Exits 1 when gp can't write its
    /// home folder or a use of what it carries failed. Example: gp self
    /// check
    Check,
}

pub(crate) fn run(cmd: &SelfCmd, paths: &Paths) -> Result<Outcome> {
    match cmd {
        SelfCmd::Check => crate::self_check::run(paths),
        SelfCmd::Path => self_path(paths),
        SelfCmd::Publish => self_publish(paths),
        SelfCmd::Docs { file, check } => crate::cli_doc::run(file, *check),
        SelfCmd::Schemas { dir, check } => crate::schemas::run(dir, *check),
        SelfCmd::Commands => crate::cli_tree::run(),
        SelfCmd::Features => {
            let md = crate::schemas::features_markdown();
            Ok(Outcome::ok(
                serde_json::json!({ "markdown": md }),
                md.trim_end().to_owned(),
            ))
        }
        SelfCmd::Shell { check } => {
            let here = std::env::current_dir()?;
            let Some(repo) = here.ancestors().find(|d| d.join("wit/shell").is_dir()) else {
                anyhow::bail!(
                    "run gp self shell inside the gameprinter repo (it reads wit/shell/)"
                );
            };
            crate::docs_shell::run(repo, *check)
        }
    }
}

/// What `$GP_HOME/bin/gp` points at, with its commit.
fn self_path(paths: &Paths) -> Result<Outcome> {
    let link = paths.home.join("bin").join("gp");
    let real = fs::canonicalize(&link).with_context(|| {
        format!(
            "no build is published at {}: the tool gate publishes one when it lands, or run `just publish`",
            link.display()
        )
    });
    let real = match real {
        Ok(r) => r,
        Err(e) => {
            return Ok(Outcome {
                json: json!({"path": null, "link": link, "error": format!("{e:#}")}),
                text: format!("{e:#}"),
                exit: 1,
            });
        }
    };
    let commit = commit_of(&real);
    Ok(Outcome::ok(
        json!({"path": real, "link": link, "commit": commit}),
        real.display().to_string(),
    ))
}

fn commit_of(real: &Path) -> Option<String> {
    real.file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix("gp-"))
        .map(str::to_owned)
}

fn self_publish(paths: &Paths) -> Result<Outcome> {
    let cwd = std::env::current_dir()?;
    let top = PathBuf::from(git(&cwd, &["rev-parse", "--show-toplevel"])?);
    if !git(&top, &["status", "--porcelain"])?.is_empty() {
        bail!(
            "{} has uncommitted changes, so a build of it has no commit to be named by",
            top.display()
        );
    }
    let commit = git(&top, &["rev-parse", "HEAD"])?;
    let line = publish(
        &paths.home,
        &std::env::current_exe()?,
        &commit,
        &top,
        &top,
        &mut Vec::new(),
    )?;
    Ok(Outcome::ok(json!({"published": line.clone()}), line))
}

/// Copies `binary` to `home/bin/gp-<commit>`, and the page's files from
/// `tree` (the checkout it was built in, at `commit`) beside it, and
/// points the link at it, unless the link's commit already holds
/// `commit`: the link only moves forward, so a slow gate finishing late
/// can't undo a newer landing. `repo` answers which commit is newer.
pub(crate) fn publish(
    home: &Path,
    binary: &Path,
    commit: &str,
    repo: &Path,
    tree: &Path,
    lines: &mut Vec<String>,
) -> Result<String> {
    let bin = home.join("bin");
    fs::create_dir_all(&bin)?;
    let short = &commit[..12];
    // The page's files first: a tree missing one publishes nothing, so no
    // build is ever published without them, and a gp serve that moves onto
    // the build finds them there.
    if let Some(l) = publish_pages(&bin, short, tree)? {
        lines.push(l);
    }
    let dest = bin.join(format!("gp-{short}"));
    // A new file renamed into place: rewriting a Mach-O in place keeps its
    // inode, and macOS kills its next launch for a stale signature.
    let tmp = bin.join(format!("gp-{short}.tmp{}", std::process::id()));
    fs::copy(binary, &tmp).with_context(|| format!("copying {}", binary.display()))?;
    fs::rename(&tmp, &dest)?;

    let link = bin.join("gp");
    let held = fs::read_link(&link).ok().and_then(|p| commit_of(&p));
    let ahead = match &held {
        None => true,
        // An unknown linked commit (a rebased history) doesn't block.
        Some(old) => {
            git(repo, &["merge-base", "--is-ancestor", old, commit]).is_ok()
                || git(repo, &["cat-file", "-e", old]).is_err()
        }
    };
    let line = if ahead {
        point_at(&bin, &link, short)?;
        format!("published {short} as {}", link.display())
    } else {
        format!(
            "published {short} at {}; the link stays on the newer {}",
            dest.display(),
            held.unwrap_or_default()
        )
    };
    lines.push(line.clone());
    prune(&bin, &link);
    Ok(line)
}

/// The page's own files a published gp serves, from the tree it was
/// built in. They are copied beside the build because that tree is the
/// tool gate's, which holds every patch it tries, landed or not.
pub(crate) const PAGES: [&str; 2] = ["mechanic", "editor/dist"];

/// The file in a built editor/dist naming the editor sources it was built
/// from: git's hash of editor/ (its sources and its lock; dist and
/// node_modules are ignored, so they aren't in it). Inside dist, which
/// git ignores, so the gate's `git clean -fd` keeps it with the build.
const EDITOR_BUILT_FROM: &str = "editor/dist/.built-from";

/// Makes sure `tree`'s editor/dist is the build of its editor/ as it is at
/// `source` (git's hash of the folder): a dist whose [`EDITOR_BUILT_FROM`]
/// names `source` is kept; else `build` runs on editor/ (the justfile's
/// `editor` recipe's npm ci and vite build), which must leave
/// editor/dist/index.html, and the new dist is marked with `source`.
/// The line saying which, or None when `tree` has no editor
/// (editor/package.json).
pub(crate) fn ensure_editor(
    tree: &Path,
    source: &str,
    build: impl FnOnce(&Path) -> std::result::Result<(), String>,
) -> std::result::Result<Option<String>, String> {
    let editor = tree.join("editor");
    if !editor.join("package.json").is_file() {
        return Ok(None);
    }
    let mark = tree.join(EDITOR_BUILT_FROM);
    let index = tree.join("editor/dist/index.html");
    let short = &source[..source.len().min(12)];
    if index.is_file() && fs::read_to_string(&mark).is_ok_and(|s| s.trim() == source) {
        return Ok(Some(format!(
            "the editor's build kept: editor/dist is built from these sources ({short})"
        )));
    }
    // A build that stops half way leaves no mark, so the next one builds.
    let _ = fs::remove_file(&mark);
    build(&editor)?;
    if !index.is_file() {
        return Err(format!("the editor's build left no {}", index.display()));
    }
    fs::write(&mark, format!("{source}\n")).map_err(|e| format!("{}: {e}", mark.display()))?;
    Ok(Some(format!(
        "the editor built into editor/dist from its sources ({short})"
    )))
}

/// The parts of [`PAGES`] `tree` must publish: mechanic/ when it has one,
/// and editor/dist when it has the editor (editor/package.json) or a
/// built one. The tool's own tree has both; a tree with neither has no page.
fn page_parts(tree: &Path) -> Vec<&'static str> {
    PAGES
        .into_iter()
        .filter(|rel| {
            let there = tree.join(rel).is_dir();
            there || (*rel == "editor/dist" && tree.join("editor/package.json").is_file())
        })
        .collect()
}

/// Copies the page's files ([`page_parts`]) from `tree` to
/// bin/gp-<short>.pages/, whole or not at all (crate::repo::page_files
/// reads them): one missing, such as an editor/dist never built, fails
/// the publish naming it, and nothing is copied. None when `tree` has no
/// page.
fn publish_pages(bin: &Path, short: &str, tree: &Path) -> Result<Option<String>> {
    let parts = page_parts(tree);
    if parts.is_empty() {
        return Ok(None);
    }
    let missing: Vec<&str> = parts
        .iter()
        .copied()
        .filter(|rel| match *rel {
            "editor/dist" => !tree.join("editor/dist/index.html").is_file(),
            _ => !tree.join(rel).is_dir(),
        })
        .collect();
    if !missing.is_empty() {
        bail!(
            "the page's files aren't all in {}: {} missing (`just editor` builds editor/dist), so nothing is published",
            tree.display(),
            missing.join(" and ")
        );
    }
    let dest = bin.join(format!("gp-{short}.pages"));
    let tmp = bin.join(format!("gp-{short}.pages.tmp{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    let mut copied = Vec::new();
    for rel in parts {
        let from = tree.join(rel);
        crate::serve::game::copy_tree(&from, &tmp.join(rel))
            .with_context(|| format!("copying {}", from.display()))?;
        copied.push(rel);
    }
    // A build of this commit published before: its files are the same
    // commit's, so the new copy replaces them.
    let _ = fs::remove_dir_all(&dest);
    fs::rename(&tmp, &dest)?;
    Ok(Some(format!(
        "the page's files ({}) published in {}",
        copied.join(", "),
        dest.display()
    )))
}

/// Points `link` (bin/gp) at bin/gp-<short>: a symlink, renamed into
/// place so the swap is whole.
#[cfg(unix)]
fn point_at(bin: &Path, link: &Path, short: &str) -> Result<()> {
    let tmp = bin.join(format!("gp.new{}", std::process::id()));
    std::os::unix::fs::symlink(format!("gp-{short}"), &tmp)?;
    fs::rename(&tmp, link)?;
    Ok(())
}

/// On Windows a symlink needs developer mode or an administrator, so
/// bin/gp.exe is a copy of the build. The copy holds no commit, so the
/// next publish always moves it.
#[cfg(not(unix))]
fn point_at(bin: &Path, _link: &Path, short: &str) -> Result<()> {
    let tmp = bin.join(format!("gp.new{}.exe", std::process::id()));
    fs::copy(bin.join(format!("gp-{short}")), &tmp)?;
    fs::rename(&tmp, bin.join("gp.exe"))
        .context("replacing bin/gp.exe (is a gp from it still running?)")?;
    Ok(())
}

/// Keeps the newest [`KEEP`] builds, the linked one and the ones a game's
/// port run is on (crate::port_run), which its workers still start on.
fn prune(bin: &Path, link: &Path) {
    let mut keep = bin
        .parent()
        .map(crate::port_run::builds)
        .unwrap_or_default();
    keep.extend(
        fs::read_link(link)
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_owned())),
    );
    let Ok(rd) = fs::read_dir(bin) else {
        return;
    };
    let mut builds: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let n = e.file_name();
            let n = n.to_str()?;
            let build = n.starts_with("gp-") && !n.contains(".tmp") && !n.ends_with(".pages");
            build.then_some(())?;
            Some((e.metadata().ok()?.modified().ok()?, e.path()))
        })
        .collect();
    builds.sort();
    for (_, p) in builds.iter().rev().skip(KEEP) {
        if p.file_name().is_none_or(|n| !keep.contains(n)) {
            let _ = fs::remove_file(p);
        }
    }
    // A build's page files go with it.
    let Ok(rd) = fs::read_dir(bin) else {
        return;
    };
    for e in rd.flatten() {
        let n = e.file_name();
        let orphan = n
            .to_str()
            .and_then(|n| n.strip_suffix(".pages"))
            .is_some_and(|b| b.starts_with("gp-") && !bin.join(b).is_file());
        if orphan {
            let _ = fs::remove_dir_all(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init(dir: &Path) {
        let run = |args: &[&str]| {
            let o = std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .output()
                .unwrap();
            assert!(o.status.success(), "{o:?}");
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "test@example.invalid"]);
        run(&["config", "user.name", "publish test"]);
        run(&["commit", "-q", "--allow-empty", "-m", "one"]);
        run(&["commit", "-q", "--allow-empty", "-m", "two"]);
    }

    /// The link moves forward and never back; a build published out of
    /// order stays beside it and `self path` still names the linked file.
    #[test]
    fn the_link_only_moves_forward() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir(&repo).unwrap();
        init(&repo);
        let exe = dir.path().join("exe");
        fs::write(&exe, b"binary").unwrap();
        let home = dir.path().join("home");
        let commit = |n: &str| git(&repo, &["rev-parse", n]).unwrap();
        let older = commit("HEAD~1");
        let newer = commit("HEAD");
        let paths = Paths {
            home: home.clone(),
            store: home.join("store"),
            roms: home.join("roms"),
        };
        let none = self_path(&paths).unwrap();
        assert_eq!(none.exit, 1, "{}", none.text);
        let mut lines = Vec::new();
        publish(&home, &exe, &newer, &repo, &repo, &mut lines).unwrap();
        let published = home.join("bin").join(format!("gp-{}", &newer[..12]));
        assert!(published.is_file());
        assert!(
            crate::repo::pages_beside(&published).is_none(),
            "a tree with no page publishes none"
        );
        assert_eq!(
            fs::canonicalize(home.join("bin/gp")).unwrap(),
            fs::canonicalize(&published).unwrap()
        );
        let said = publish(&home, &exe, &older, &repo, &repo, &mut lines).unwrap();
        assert!(said.contains("stays"), "{said}");
        assert_eq!(
            fs::canonicalize(home.join("bin/gp")).unwrap(),
            fs::canonicalize(&published).unwrap(),
            "an older commit doesn't take the link"
        );
        assert!(
            home.join("bin")
                .join(format!("gp-{}", &older[..12]))
                .is_file()
        );
        assert_eq!(self_path(&paths).unwrap().exit, 0);
    }

    #[cfg(unix)]
    #[test]
    fn prune_keeps_the_linked_build() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir(&bin).unwrap();
        for i in 0..12 {
            let p = bin.join(format!("gp-{i:012}"));
            fs::write(&p, b"x").unwrap();
            let t =
                std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000 + i as u64);
            fs::File::options()
                .write(true)
                .open(&p)
                .unwrap()
                .set_modified(t)
                .unwrap();
        }
        let link = bin.join("gp");
        std::os::unix::fs::symlink(format!("gp-{:012}", 0), &link).unwrap();
        // A port run on an old build keeps it too.
        let run_on = bin.join(format!("gp-{:012}", 2));
        crate::port_run::gp_for(dir.path(), "rooftop", &run_on, 1);
        prune(&bin, &link);
        assert!(
            bin.join(format!("gp-{:012}", 0)).is_file(),
            "the linked one stays"
        );
        assert!(run_on.is_file(), "the port run's stays");
        assert!(bin.join(format!("gp-{:012}", 11)).is_file());
        assert!(!bin.join(format!("gp-{:012}", 1)).exists());
    }

    /// A tree with the editor's sources and no editor/dist (the tool
    /// gate's new tree) has it built; the build is kept while editor/'s
    /// sources stay, built again when they change, and a build that fails
    /// or leaves no index.html says so and is built again next time.
    #[test]
    fn a_tree_with_no_dist_has_its_editor_built_and_kept_while_its_sources_stay() {
        let dir = tempfile::tempdir().unwrap();
        let tree = dir.path();
        let builds = std::cell::Cell::new(0);
        let vite = |editor: &Path| -> std::result::Result<(), String> {
            builds.set(builds.get() + 1);
            fs::create_dir_all(editor.join("dist")).unwrap();
            fs::write(editor.join("dist/index.html"), "editor").unwrap();
            Ok(())
        };
        // No editor at all: nothing to build.
        assert_eq!(ensure_editor(tree, "a1", vite), Ok(None));
        assert_eq!(builds.get(), 0);

        fs::create_dir_all(tree.join("editor")).unwrap();
        fs::write(tree.join("editor/package.json"), "{}").unwrap();
        let said = ensure_editor(tree, "a1", vite).unwrap().unwrap();
        assert!(said.contains("built"), "{said}");
        assert_eq!(builds.get(), 1);
        assert!(tree.join("editor/dist/index.html").is_file());
        let said = ensure_editor(tree, "a1", vite).unwrap().unwrap();
        assert!(said.contains("kept"), "{said}");
        assert_eq!(builds.get(), 1, "the same sources build nothing");
        ensure_editor(tree, "b2", vite).unwrap();
        assert_eq!(builds.get(), 2, "changed sources build again");

        let failed = ensure_editor(tree, "c3", |_| Err("npm ci failed".into()));
        assert_eq!(failed, Err("npm ci failed".into()));
        ensure_editor(tree, "c3", vite).unwrap();
        assert_eq!(builds.get(), 3, "after a failed build the next one builds");
        fs::remove_dir_all(tree.join("editor/dist")).unwrap();
        let empty = ensure_editor(tree, "d4", |_| Ok(()));
        assert!(empty.unwrap_err().contains("index.html"));
    }

    /// Publishing is all or nothing: a tree with mechanic/ and the editor's
    /// sources but no built editor/dist publishes no build, no page and no
    /// link, saying what is missing; once dist is built it publishes both.
    #[test]
    fn a_tree_with_no_dist_publishes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (home, tree) = (dir.path().join("home"), dir.path().join("tree"));
        fs::create_dir_all(tree.join("mechanic")).unwrap();
        fs::write(tree.join("mechanic/screen.js"), "page").unwrap();
        fs::create_dir_all(tree.join("editor")).unwrap();
        fs::write(tree.join("editor/package.json"), "{}").unwrap();
        let exe = dir.path().join("exe");
        fs::write(&exe, b"binary").unwrap();
        let commit = "0123456789ab0123456789ab0123456789ab0123";
        let bin = home.join("bin");
        let err = publish(&home, &exe, commit, &tree, &tree, &mut Vec::new()).unwrap_err();
        let err = format!("{err:#}");
        assert!(err.contains("editor/dist missing"), "{err}");
        assert!(
            !bin.join("gp-0123456789ab").exists(),
            "no build without its page"
        );
        assert!(!bin.join("gp-0123456789ab.pages").exists());
        assert!(
            fs::symlink_metadata(bin.join("gp")).is_err(),
            "the link didn't move"
        );

        fs::create_dir_all(tree.join("editor/dist")).unwrap();
        fs::write(tree.join("editor/dist/index.html"), "editor").unwrap();
        let mut lines = Vec::new();
        publish(&home, &exe, commit, &tree, &tree, &mut lines).unwrap();
        assert!(lines[0].contains("mechanic, editor/dist"), "{lines:?}");
        let pages = crate::repo::pages_beside(&bin.join("gp-0123456789ab")).unwrap();
        assert!(pages.join("editor/dist/index.html").is_file());
        assert!(pages.join("mechanic/screen.js").is_file());
    }

    /// The page's files are published beside the build, are not a build
    /// of their own when pruning, and go with their build.
    #[cfg(unix)]
    #[test]
    fn the_page_is_published_beside_its_build_and_pruned_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        let tree = dir.path().join("tree");
        fs::create_dir_all(tree.join("mechanic")).unwrap();
        fs::create_dir_all(tree.join("editor/dist")).unwrap();
        fs::write(tree.join("mechanic/screen.js"), "landed").unwrap();
        fs::write(tree.join("editor/dist/index.html"), "editor").unwrap();
        fs::create_dir_all(&bin).unwrap();
        let said = publish_pages(&bin, "000000000000", &tree).unwrap();
        let said = said.unwrap();
        assert!(said.contains("mechanic, editor/dist"), "{said}");
        let build = bin.join("gp-000000000000");
        fs::write(&build, b"x").unwrap();
        let pages = crate::repo::pages_beside(&build);
        let pages = pages.unwrap();
        assert_eq!(pages, bin.join("gp-000000000000.pages"));
        // The tree moving on (a gate's next try) leaves the copy alone.
        fs::write(tree.join("mechanic/screen.js"), "tried").unwrap();
        let read = |rel: &str| fs::read_to_string(pages.join(rel)).unwrap();
        assert_eq!(read("mechanic/screen.js"), "landed");
        assert_eq!(read("editor/dist/index.html"), "editor");
        // A build whose binary was pruned loses its page; the linked
        // one's stays, and the folder counts as no build.
        fs::create_dir_all(bin.join("gp-111111111111.pages/mechanic")).unwrap();
        let link = bin.join("gp");
        std::os::unix::fs::symlink("gp-000000000000", &link).unwrap();
        prune(&bin, &link);
        assert!(pages.join("mechanic/screen.js").is_file());
        assert!(!bin.join("gp-111111111111.pages").exists());
        assert!(crate::repo::pages_beside(&dir.path().join("gp")).is_none());
    }
}
