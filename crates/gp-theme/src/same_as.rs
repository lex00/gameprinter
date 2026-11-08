//! A theme folder that points at another (docs/formats/theme.md, "One
//! theme for two templates"). A template's `themes/<id>/` holds either the
//! theme (`theme.toml` and its files) or only `same_as.toml`:
//!
//! ```toml
//! theme = "../../../donkey-kong-oe/themes/paper"
//! ```
//!
//! The path is relative to the pointing folder and names a theme folder of
//! another template in the same templates folder. The pointer is followed
//! once: its target must hold `theme.toml` itself, not another pointer.
//! Whatever reads the folder reads the target's files.

use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

/// The pointer's file name.
pub const SAME_AS_FILE: &str = "same_as.toml";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SameAs {
    theme: String,
}

/// True when `dir` is a theme folder: it holds `theme.toml` or
/// `same_as.toml`.
pub fn is_theme_dir(dir: &Path) -> bool {
    dir.join(crate::format::THEME_FILE).is_file() || dir.join(SAME_AS_FILE).is_file()
}

/// True when `dir` holds a pointer and no theme of its own.
pub fn is_pointer(dir: &Path) -> bool {
    !dir.join(crate::format::THEME_FILE).is_file() && dir.join(SAME_AS_FILE).is_file()
}

/// The folder whose files are the theme in `dir`: `dir` itself when it
/// holds `theme.toml`, else the target its `same_as.toml` names. A pointer
/// is refused when its target isn't a theme folder of a template beside
/// this one, is missing, or is itself a pointer (a chain or a loop).
pub fn target(dir: &Path) -> Result<PathBuf, String> {
    if dir.join(crate::format::THEME_FILE).is_file() {
        return Ok(dir.to_owned());
    }
    let file = dir.join(SAME_AS_FILE);
    let text = match std::fs::read_to_string(&file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!(
                "{}: no {} or {SAME_AS_FILE} here, so it isn't a theme",
                dir.display(),
                crate::format::THEME_FILE
            ));
        }
        Err(e) => return Err(format!("{}: {e}", file.display())),
    };
    let s: SameAs = toml::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
    let rel = Path::new(&s.theme);
    if rel.is_absolute() {
        return Err(format!(
            "{}: `{}` is an absolute path; name the theme relative to this folder, such as `../../../OTHER-TEMPLATE/themes/ID`",
            file.display(),
            s.theme
        ));
    }
    let here = normal(dir);
    let to = normal(&dir.join(rel));
    // themes/<id>/ of a template: three folders up is the templates folder.
    let root = |p: &Path| p.ancestors().nth(3).map(Path::to_owned);
    let in_themes = |p: &Path| {
        p.parent()
            .and_then(Path::file_name)
            .is_some_and(|n| n == "themes")
    };
    if !in_themes(&to) || root(&to).is_none() || root(&to) != root(&here) {
        return Err(format!(
            "{}: `{}` isn't a theme folder of a template beside this one (TEMPLATE/themes/ID)",
            file.display(),
            s.theme
        ));
    }
    if to == here {
        return Err(format!(
            "{}: the pointer names its own folder, a loop",
            file.display()
        ));
    }
    if to.join(crate::format::THEME_FILE).is_file() {
        return Ok(to);
    }
    if to.join(SAME_AS_FILE).is_file() {
        return Err(format!(
            "{}: `{}` is a pointer too; a pointer must name the folder that holds theme.toml (no chains or loops)",
            file.display(),
            s.theme
        ));
    }
    Err(format!(
        "{}: `{}` has no {}; the theme it points at is missing",
        file.display(),
        s.theme,
        crate::format::THEME_FILE
    ))
}

/// `p` with each `.` dropped and each `..` taking off the folder before it,
/// without reading the disk (a WASI host may not resolve `..`).
fn normal(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A templates folder with `a/themes/paper/` holding a made-up theme.
    fn templates() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let paper = d.path().join("a/themes/paper");
        fs::create_dir_all(&paper).unwrap();
        fs::write(
            paper.join(crate::format::THEME_FILE),
            "format = \"0.1.3\"\nid = \"paper\"\ntitle = \"Paper\"\nlicense = \"CC0-1.0\"\nfor = [\"m_a\", \"m_b\"]\n",
        )
        .unwrap();
        d
    }

    fn point(dir: &Path, to: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(SAME_AS_FILE), format!("theme = \"{to}\"\n")).unwrap();
    }

    #[test]
    fn a_pointer_resolves_to_its_target() {
        let d = templates();
        let b = d.path().join("b/themes/paper");
        point(&b, "../../../a/themes/paper");
        assert!(is_theme_dir(&b) && is_pointer(&b));
        assert_eq!(target(&b).unwrap(), d.path().join("a/themes/paper"));
        // A theme folder is its own target.
        let a = d.path().join("a/themes/paper");
        assert!(!is_pointer(&a));
        assert_eq!(target(&a).unwrap(), a);
        // stack::load reads the target, and the loaded theme's files are
        // the target's.
        let l = crate::stack::load(&b, crate::Scope::Baked).unwrap();
        assert_eq!(l.theme.id, "paper");
        assert_eq!(l.dir, a);
    }

    #[test]
    fn a_missing_target_is_refused() {
        let d = templates();
        let b = d.path().join("b/themes/paper");
        point(&b, "../../../a/themes/night");
        let e = target(&b).unwrap_err();
        assert!(
            e.contains("has no theme.toml; the theme it points at is missing"),
            "{e}"
        );
    }

    #[test]
    fn a_loop_or_chain_is_refused() {
        let d = templates();
        let b = d.path().join("b/themes/paper");
        let c = d.path().join("c/themes/paper");
        point(&b, "../../../c/themes/paper");
        point(&c, "../../../b/themes/paper");
        let e = target(&b).unwrap_err();
        assert!(e.contains("is a pointer too"), "{e}");
        let own = d.path().join("d/themes/paper");
        point(&own, "../paper");
        let e = target(&own).unwrap_err();
        assert!(e.contains("names its own folder, a loop"), "{e}");
    }

    #[test]
    fn a_target_outside_the_templates_folder_is_refused() {
        let d = templates();
        let b = d.path().join("b/themes/paper");
        point(&b, "../../../../elsewhere/themes/paper");
        assert!(target(&b).unwrap_err().contains("beside this one"));
        point(&b, "../../a/paper");
        assert!(target(&b).unwrap_err().contains("beside this one"));
        point(&b, "/a/themes/paper");
        assert!(target(&b).unwrap_err().contains("absolute path"));
        // Not a theme folder at all.
        let e = target(&d.path().join("a/themes")).unwrap_err();
        assert!(e.contains("isn't a theme"), "{e}");
    }
}
