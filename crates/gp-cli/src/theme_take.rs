//! `gp theme look take`: named pictures from one large-art theme folder
//! (the person's copy, `$GP_HOME/work/themes/NAME/repo`) brought into
//! another in a games checkout (`GAME/themes/NAME`), so a change that lands
//! a theme's rules can carry its pictures too.
//!
//! Each file under the named paths of the copy's `theme/` folder is written
//! at the same path under the target's `theme/` folder. A file the target
//! repo keeps in Git LFS (its `.gitattributes`, as `git check-attr filter`
//! reads them) is also put in the repo's LFS store (`lfs/objects/` of its
//! common git folder, shared by every worktree), as `git add` would, so the
//! pointer a patch carries names a picture the gate and a card's look can
//! find. Each written file is marked to add (`git add --intent-to-add`), so
//! `git diff` in the repo shows it, as its LFS pointer.

use std::fmt::Write as _;
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde_json::json;

use crate::Outcome;

/// The first line of a Git LFS pointer.
const LFS_VERSION: &str = "version https://git-lfs.github.com/spec/v1";

/// A PNG file's first eight bytes.
const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

/// A theme folder's `theme/` folder: `dir` itself when it holds look.txt
/// and no theme/ folder, else `dir/theme`.
fn theme_of(dir: &Path) -> PathBuf {
    if dir.join(gp_present::looks::LOOK_FILE).is_file() && !dir.join("theme").is_dir() {
        dir.to_owned()
    } else {
        dir.join("theme")
    }
}

/// A path under the copy's theme/ folder as given (`pieces/punk`,
/// `theme/pieces/punk/sw`), checked to stay inside it.
fn clean_rel(s: &str) -> Result<PathBuf> {
    let s = s.trim().trim_end_matches('/');
    let s = s.strip_prefix("theme/").unwrap_or(s);
    let p = Path::new(s);
    let ok = !s.is_empty() && p.components().all(|c| matches!(c, Component::Normal(_)));
    if !ok {
        bail!(
            "--only {s:?} isn't a path under the copy's theme/ folder: give one like pieces/punk, with no .. and not from /"
        );
    }
    Ok(p.to_owned())
}

/// Every file under `p` (or `p` itself), in name order, leaving out hidden
/// ones (`.DS_Store`).
fn files_under(p: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let hidden = p
        .file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with('.'));
    if hidden {
        return Ok(());
    }
    if p.is_file() {
        out.push(p.to_owned());
        return Ok(());
    }
    let mut es: Vec<PathBuf> = fs::read_dir(p)
        .with_context(|| format!("reading {}", p.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    es.sort();
    for e in es {
        files_under(&e, out)?;
    }
    Ok(())
}

/// The nearest folder at or above `p` that exists.
fn existing_above(p: &Path) -> Option<&Path> {
    p.ancestors().find(|a| a.is_dir())
}

struct Taken {
    /// From the repo's top, with `/`.
    path: String,
    state: &'static str,
    lfs: bool,
    bytes: usize,
}

pub(crate) fn take(copy: &Path, into: &Path, only: &[String]) -> Result<Outcome> {
    let from = theme_of(copy);
    if !from.is_dir() {
        bail!(
            "{} has no theme/ folder: give the theme copy's folder (holding package.toml and theme/), or its theme/ folder",
            copy.display()
        );
    }
    let to = theme_of(into);
    let above = existing_above(&to).with_context(|| format!("no folder above {}", to.display()))?;
    let top = crate::gate::git(above, &["rev-parse", "--show-toplevel"]).map_err(|_| {
        anyhow::anyhow!(
            "{} isn't in a git checkout: give the theme's folder in a games checkout or a worktree of one (GAME/themes/NAME)",
            into.display()
        )
    })?;
    let top = PathBuf::from(top);
    let common = crate::gate::git(&top, &["rev-parse", "--git-common-dir"])?;
    let common = top.join(common);
    if only.is_empty() {
        bail!(
            "name what to take with --only (pieces/punk, say): a file or folder under the copy's theme/ folder"
        );
    }

    // What to take: each file under each named path, once.
    let mut files: Vec<PathBuf> = Vec::new();
    for o in only {
        let rel = clean_rel(o)?;
        let p = from.join(&rel);
        if !p.exists() {
            bail!(
                "the copy has no {} (looked for {})",
                rel.display(),
                p.display()
            );
        }
        let mut found = Vec::new();
        files_under(&p, &mut found)?;
        for f in found {
            if !files.contains(&f) {
                files.push(f);
            }
        }
    }
    if files.is_empty() {
        bail!("nothing to take: the paths given hold no files");
    }

    // Read every file first, so a bad one writes nothing.
    let mut read = Vec::new();
    for f in &files {
        let rel = f.strip_prefix(&from).unwrap_or(f).to_owned();
        let bytes = fs::read(f).with_context(|| format!("reading {}", f.display()))?;
        if bytes.starts_with(LFS_VERSION.as_bytes()) {
            bail!(
                "{} in the copy is a Git LFS pointer, not its picture: run `git lfs pull` in the copy first",
                f.display()
            );
        }
        let png = rel
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("png"));
        if png && !bytes.starts_with(PNG_MAGIC) {
            bail!("{} isn't a PNG picture", f.display());
        }
        read.push((rel, bytes));
    }

    fs::create_dir_all(&to).with_context(|| format!("creating {}", to.display()))?;
    let to_real = fs::canonicalize(&to).unwrap_or_else(|_| to.clone());
    let top_real = fs::canonicalize(&top).unwrap_or_else(|_| top.clone());
    let base = to_real
        .strip_prefix(&top_real)
        .with_context(|| format!("{} isn't under {}", to.display(), top.display()))?
        .to_owned();
    let repo_rel = |rel: &Path| base.join(rel).to_string_lossy().replace('\\', "/");

    // Which of them the repo keeps in Git LFS.
    let rels: Vec<String> = read.iter().map(|(r, _)| repo_rel(r)).collect();
    let mut args = vec!["check-attr", "-z", "filter", "--"];
    args.extend(rels.iter().map(String::as_str));
    let attrs = crate::gate::git(&top, &args)?;
    let parts: Vec<&str> = attrs.split('\0').collect();
    let lfs_of = |path: &str| {
        parts
            .chunks(3)
            .any(|c| c.len() == 3 && c[0] == path && c[2] == "lfs")
    };

    let mut taken = Vec::new();
    for ((rel, bytes), path) in read.iter().zip(&rels) {
        use sha2::Digest as _;
        let dest = to.join(rel);
        let state = match fs::read(&dest) {
            Ok(old) if old == *bytes => "same",
            Ok(_) => "changed",
            Err(_) => "new",
        };
        let lfs = lfs_of(path);
        if lfs {
            let oid = hex::encode(sha2::Sha256::digest(bytes));
            let dir = common.join("lfs/objects").join(&oid[..2]).join(&oid[2..4]);
            let obj = dir.join(&oid);
            if !obj.is_file() {
                fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
                let tmp = dir.join(format!("{oid}.gp-take"));
                fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
                fs::rename(&tmp, &obj).with_context(|| format!("writing {}", obj.display()))?;
            }
        }
        if state != "same" {
            if let Some(d) = dest.parent() {
                fs::create_dir_all(d).with_context(|| format!("creating {}", d.display()))?;
            }
            fs::write(&dest, bytes).with_context(|| format!("writing {}", dest.display()))?;
        }
        taken.push(Taken {
            path: path.clone(),
            state,
            lfs,
            bytes: bytes.len(),
        });
    }

    // New files marked to add, so `git diff` shows them.
    let fresh: Vec<&str> = taken
        .iter()
        .filter(|t| t.state == "new")
        .map(|t| t.path.as_str())
        .collect();
    if !fresh.is_empty() {
        let mut args = vec!["add", "--intent-to-add", "--"];
        args.extend(fresh.iter().copied());
        crate::gate::git(&top, &args)?;
    }

    let count = |s: &str| taken.iter().filter(|t| t.state == s).count();
    let (new, changed, same) = (count("new"), count("changed"), count("same"));
    let not_lfs: Vec<&str> = taken
        .iter()
        .filter(|t| !t.lfs)
        .map(|t| t.path.as_str())
        .collect();
    let mut text = format!(
        "Took {} file{} from {} into {}: {new} new, {changed} changed, {same} the same.",
        taken.len(),
        if taken.len() == 1 { "" } else { "s" },
        from.display(),
        to.display()
    );
    for t in &taken {
        let _ = write!(
            text,
            "\n  {} {}{}",
            t.state,
            t.path,
            if t.lfs { "" } else { " (not in Git LFS)" }
        );
    }
    if !not_lfs.is_empty() {
        text.push_str(
            "\nThe repo keeps some of them out of Git LFS: make the patch with `git diff --binary` so they apply.",
        );
    }
    let _ = write!(
        text,
        "\nNext: `git diff{} > FILE` in {} (the pictures show as their LFS pointers), then gp model gate GAME --patch FILE --dry-run.",
        if not_lfs.is_empty() { "" } else { " --binary" },
        top.display()
    );
    let json = json!({
        "from": from.display().to_string(),
        "into": to.display().to_string(),
        "repo": top.display().to_string(),
        "new": new,
        "changed": changed,
        "same": same,
        "files": taken.iter().map(|t| json!({
            "path": t.path, "state": t.state, "lfs": t.lfs, "bytes": t.bytes,
        })).collect::<Vec<_>>(),
    });
    Ok(Outcome::ok(json, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) -> String {
        crate::gate::git(dir, args).unwrap()
    }

    fn png(n: u8) -> Vec<u8> {
        let mut b = PNG_MAGIC.to_vec();
        b.extend([n; 20]);
        b
    }

    fn put(p: &Path, b: &[u8]) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b).unwrap();
    }

    /// A games checkout with a theme folder whose pictures are in Git LFS,
    /// and a theme copy beside it with pictures the checkout lacks.
    fn setup(d: &Path) -> (PathBuf, PathBuf) {
        let games = d.join("games");
        fs::create_dir_all(&games).unwrap();
        git(&games, &["init", "-q"]);
        put(
            &games.join(".gitattributes"),
            b"*.png filter=lfs diff=lfs merge=lfs -text\n",
        );
        put(
            &games.join("hilltop/themes/neon/theme/look.txt"),
            b"id neon\n",
        );
        put(
            &games.join("hilltop/themes/neon/theme/pieces/old.png"),
            &png(1),
        );
        let copy = d.join("copy/repo");
        put(&copy.join("package.toml"), b"[package]\n");
        put(&copy.join("theme/look.txt"), b"id neon\n");
        put(&copy.join("theme/pieces/old.png"), &png(2));
        put(&copy.join("theme/pieces/punk/wall.png"), &png(3));
        put(&copy.join("theme/pieces/punk/sw/wall.png"), &png(4));
        put(&copy.join("theme/pieces/punk/.DS_Store"), b"junk");
        put(&copy.join("theme/pieces/space/star.png"), &png(5));
        (games, copy)
    }

    fn object(games: &Path, b: &[u8]) -> PathBuf {
        use sha2::Digest as _;
        let oid = hex::encode(sha2::Sha256::digest(b));
        games
            .join(".git/lfs/objects")
            .join(&oid[..2])
            .join(&oid[2..4])
            .join(&oid)
    }

    #[test]
    fn a_theme_copys_named_pictures_are_taken_into_a_games_checkout_and_its_lfs_store() {
        let d = tempfile::tempdir().unwrap();
        let (games, copy) = setup(d.path());
        let into = games.join("hilltop/themes/neon");
        let o = take(
            &copy,
            &into,
            &["theme/pieces/punk".into(), "pieces/old.png".into()],
        )
        .unwrap();
        assert_eq!(o.exit, 0);
        let t = into.join("theme/pieces");
        assert_eq!(fs::read(t.join("punk/wall.png")).unwrap(), png(3));
        assert_eq!(fs::read(t.join("punk/sw/wall.png")).unwrap(), png(4));
        assert_eq!(fs::read(t.join("old.png")).unwrap(), png(2));
        // Only what was named: no hidden files, nothing from space/.
        assert!(!t.join("punk/.DS_Store").exists());
        assert!(!t.join("space").exists());
        // Each picture is in the LFS store, by its sha256.
        for n in [2, 3, 4] {
            assert_eq!(fs::read(object(&games, &png(n))).unwrap(), png(n));
        }
        assert_eq!(
            (o.json["new"].clone(), o.json["changed"].clone()),
            (json!(2), json!(1))
        );
        let paths: Vec<&str> = o.json["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["path"].as_str().unwrap())
            .collect();
        assert_eq!(
            paths,
            [
                "hilltop/themes/neon/theme/pieces/punk/sw/wall.png",
                "hilltop/themes/neon/theme/pieces/punk/wall.png",
                "hilltop/themes/neon/theme/pieces/old.png",
            ]
        );
        assert!(
            o.json["files"]
                .as_array()
                .unwrap()
                .iter()
                .all(|f| f["lfs"] == true)
        );
        // The new ones are marked to add, so `git diff` shows them.
        let listed = git(&games, &["ls-files"]);
        assert!(listed.contains("hilltop/themes/neon/theme/pieces/punk/wall.png"));
        // Taken again, nothing changes.
        let again = take(&copy, &into, &["pieces/punk".into()]).unwrap();
        assert_eq!(again.json["same"], json!(2));
    }

    #[test]
    fn a_take_refuses_paths_outside_the_copy_and_pointers_and_writes_nothing() {
        let d = tempfile::tempdir().unwrap();
        let (games, copy) = setup(d.path());
        let into = games.join("hilltop/themes/neon");
        for bad in ["../package.toml", "/etc", "pieces/none"] {
            assert!(take(&copy, &into, &[bad.into()]).is_err(), "{bad}");
        }
        assert!(take(&copy, &into, &[]).is_err());
        // A pointer in the copy (its picture never pulled) is refused, and
        // the good picture beside it isn't written either.
        put(
            &copy.join("theme/pieces/punk/ptr.png"),
            format!("{LFS_VERSION}\noid sha256:{}\nsize 9\n", "a".repeat(64)).as_bytes(),
        );
        let Err(e) = take(&copy, &into, &["pieces/punk".into()]) else {
            panic!("a pointer in the copy was taken");
        };
        assert!(format!("{e:#}").contains("LFS pointer"), "{e:#}");
        assert!(!into.join("theme/pieces/punk").exists());
        // Not a git checkout.
        let loose = d.path().join("loose/neon");
        assert!(take(&copy, &loose, &["pieces/old.png".into()]).is_err());
    }
}
