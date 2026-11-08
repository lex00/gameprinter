//! Where Game Printer keeps data on this machine.
//!
//! ROMs and the evidence store always live outside any git work tree, so
//! nothing copied from or derived from a game can be committed by accident.

use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Paths {
    /// `$GP_HOME`, default `~/.game-printer`.
    pub home: PathBuf,
    /// `$GP_STORE`, default `$GP_HOME/store`.
    pub store: PathBuf,
    /// `$GP_ROM_DIR`, default `$GP_HOME/roms`.
    pub roms: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum PathsError {
    #[error("no home directory; set GP_HOME")]
    NoHome,
    #[error("{name} must be an absolute path, got {path}")]
    NotAbsolute { name: &'static str, path: PathBuf },
    #[error("{name} ({path}) is inside the git work tree at {repo}; keep it outside any repo")]
    InsideGitRepo {
        name: &'static str,
        path: PathBuf,
        repo: PathBuf,
    },
}

impl Paths {
    pub fn from_env() -> Result<Self, PathsError> {
        let var = |k: &str| {
            std::env::var_os(k)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        Self::resolve(var("GP_HOME"), var("GP_STORE"), var("GP_ROM_DIR"))
    }

    pub fn resolve(
        home: Option<PathBuf>,
        store: Option<PathBuf>,
        roms: Option<PathBuf>,
    ) -> Result<Self, PathsError> {
        let home = match home {
            Some(h) => h,
            None => dirs::home_dir()
                .ok_or(PathsError::NoHome)?
                .join(".game-printer"),
        };
        let store = store.unwrap_or_else(|| home.join("store"));
        let roms = roms.unwrap_or_else(|| home.join("roms"));

        for (name, path) in [
            ("GP_HOME", &home),
            ("GP_STORE", &store),
            ("GP_ROM_DIR", &roms),
        ] {
            if !path.is_absolute() {
                return Err(PathsError::NotAbsolute {
                    name,
                    path: path.clone(),
                });
            }
            if let Some(repo) = enclosing_git_work_tree(path) {
                return Err(PathsError::InsideGitRepo {
                    name,
                    path: path.clone(),
                    repo,
                });
            }
        }
        Ok(Self { home, store, roms })
    }

    pub fn runs_dir(&self) -> PathBuf {
        self.home.join("runs")
    }

    pub fn reports_dir(&self) -> PathBuf {
        self.home.join("reports")
    }

    pub fn metrics_db(&self) -> PathBuf {
        self.home.join("metrics.sqlite")
    }
}

/// The nearest ancestor of `path` (or `path` itself) that contains a `.git`
/// entry. The path doesn't need to exist yet. Symlinks are resolved first, so
/// a link that points into a repo counts as inside it.
pub fn enclosing_git_work_tree(path: &Path) -> Option<PathBuf> {
    real_path(path)
        .ancestors()
        .find(|dir| dir.join(".git").exists())
        .map(Path::to_path_buf)
}

/// `path` made absolute (against the current directory), with symlinks
/// resolved in the deepest part of it that exists; the rest, which doesn't
/// exist yet, is appended unchanged. A relative path is made absolute first,
/// so a bare file name in a repo's subdirectory still finds the repo.
pub fn real_path(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let path = absolute.as_path();
    let mut rest = Vec::new();
    let mut existing = path;
    loop {
        if let Ok(real) = existing.canonicalize() {
            return rest.iter().rev().fold(real, |p, part| p.join(part));
        }
        match (existing.file_name(), existing.parent()) {
            (Some(name), Some(parent)) => {
                rest.push(name.to_owned());
                existing = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_hang_off_home() {
        let tmp = tempfile::tempdir().unwrap();
        let p = Paths::resolve(Some(tmp.path().to_path_buf()), None, None).unwrap();
        assert_eq!(p.store, tmp.path().join("store"));
        assert_eq!(p.roms, tmp.path().join("roms"));
        assert_eq!(p.metrics_db(), tmp.path().join("metrics.sqlite"));
    }

    #[test]
    fn refuses_paths_inside_a_git_work_tree() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join(".git")).unwrap();
        let roms = tmp.path().join("assets").join("roms");
        let home = tempfile::tempdir().unwrap();
        let err = Paths::resolve(Some(home.path().to_path_buf()), None, Some(roms)).unwrap_err();
        assert!(matches!(
            err,
            PathsError::InsideGitRepo {
                name: "GP_ROM_DIR",
                ..
            }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn refuses_a_symlink_into_a_git_work_tree() {
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir(repo.path().join(".git")).unwrap();
        std::fs::create_dir(repo.path().join("roms")).unwrap();
        let outside = tempfile::tempdir().unwrap();
        let link = outside.path().join("roms-link");
        std::os::unix::fs::symlink(repo.path().join("roms"), &link).unwrap();
        let home = tempfile::tempdir().unwrap();
        let err = Paths::resolve(
            Some(home.path().to_path_buf()),
            None,
            Some(link.join("nes")),
        )
        .unwrap_err();
        assert!(matches!(
            err,
            PathsError::InsideGitRepo {
                name: "GP_ROM_DIR",
                ..
            }
        ));
    }

    #[test]
    fn real_path_keeps_parts_that_dont_exist_yet() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("not").join("yet");
        assert_eq!(
            real_path(&p),
            tmp.path().canonicalize().unwrap().join("not").join("yet")
        );
    }

    #[test]
    fn a_relative_name_in_a_repo_subdirectory_is_inside_it() {
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir(repo.path().join(".git")).unwrap();
        let sub = repo.path().join("a").join("b");
        std::fs::create_dir_all(&sub).unwrap();
        // A path relative to `sub`, as a bare `-o out.nes` run from it would be.
        let rel = pathdiff(&sub.join("out.nes"));
        assert_eq!(
            enclosing_git_work_tree(&rel).map(|p| p.canonicalize().unwrap()),
            Some(repo.path().canonicalize().unwrap())
        );
    }

    /// `p` relative to the current directory, through `..` components.
    fn pathdiff(p: &Path) -> PathBuf {
        let cwd = std::env::current_dir().unwrap().canonicalize().unwrap();
        let p = p.canonicalize().unwrap_or_else(|_| {
            p.parent()
                .unwrap()
                .canonicalize()
                .unwrap()
                .join(p.file_name().unwrap())
        });
        let common = cwd.ancestors().find(|a| p.starts_with(a)).unwrap();
        let ups = cwd.strip_prefix(common).unwrap().components().count();
        let mut out = PathBuf::new();
        for _ in 0..ups {
            out.push("..");
        }
        out.join(p.strip_prefix(common).unwrap())
    }

    #[test]
    fn refuses_relative_paths() {
        let err = Paths::resolve(Some(PathBuf::from("relative")), None, None).unwrap_err();
        assert!(matches!(
            err,
            PathsError::NotAbsolute {
                name: "GP_HOME",
                ..
            }
        ));
    }
}
