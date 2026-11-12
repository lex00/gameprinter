//! Playable links: a published build's directory, served by the host at
//! `/play/<id>/`. The ID is the first 12 hex digits of the SHA-256 of the
//! build's `manifest.json`, so the same build has the same link, and a
//! rebuilt one has a new link. A link holds the directory's path, not a copy:
//! the host serves what is there.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Link {
    pub id: String,
    pub dir: PathBuf,
    /// The path the host serves it at.
    pub path: String,
    /// The full URL, once the host is listening.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The manifest's copy, target and template, for a listing.
    #[serde(default)]
    pub copy: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub template: Option<String>,
}

pub fn link_for(dir: &Path) -> Result<Link, String> {
    let dir = std::fs::canonicalize(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let manifest = dir.join("manifest.json");
    let bytes = std::fs::read(&manifest).map_err(|_| {
        format!(
            "{} has no manifest.json, so it isn't a published build",
            dir.display()
        )
    })?;
    if !dir.join("index.html").is_file() {
        return Err(format!("{} has no index.html", dir.display()));
    }
    let id: String = Sha256::digest(&bytes)
        .iter()
        .take(6)
        .map(|b| format!("{b:02x}"))
        .collect();
    let m: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
    Ok(Link {
        path: format!("/play/{id}/"),
        id,
        dir,
        url: None,
        copy: m["copy"].as_str().map(str::to_owned),
        target: m["target"].as_str().map(str::to_owned),
        template: m["template"]["id"].as_str().map(str::to_owned),
    })
}

/// The registry, kept in a JSON file so links survive a restart.
pub fn load(file: &Path) -> BTreeMap<String, Link> {
    std::fs::read(file)
        .ok()
        .and_then(|b| serde_json::from_slice::<Vec<Link>>(&b).ok())
        .map(|v| v.into_iter().map(|l| (l.id.clone(), l)).collect())
        .unwrap_or_default()
}

pub fn save(file: &Path, links: &BTreeMap<String, Link>) -> Result<(), String> {
    let v: Vec<&Link> = links.values().collect();
    let tmp = file.with_extension("tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(&v).expect("links serialize"),
    )
    .and_then(|()| std::fs::rename(&tmp, file))
    .map_err(|e| format!("writing {}: {e}", file.display()))
}

/// The file inside a link's directory a request path names, never outside
/// it.
pub fn resolve(link: &Link, rest: &str) -> Option<PathBuf> {
    let mut p = link.dir.clone();
    let rest = rest.trim_start_matches('/');
    if rest.is_empty() {
        p.push("index.html");
    } else {
        for part in rest.split('/') {
            if part.is_empty()
                || part == "."
                || part == ".."
                || part.contains('\\')
                || part.contains('\0')
            {
                return None;
            }
            p.push(part);
        }
    }
    let real = std::fs::canonicalize(&p).ok()?;
    (real.starts_with(&link.dir) && real.is_file()).then_some(real)
}

pub fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("css") => "text/css; charset=utf-8",
        Some("toml" | "txt") => "text/plain; charset=utf-8",
        Some("br") => "application/octet-stream",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_serves_only_files_inside_its_build() {
        let root = tempfile::tempdir().unwrap();
        let build = root.path().join("build");
        std::fs::create_dir_all(build.join("sub")).unwrap();
        std::fs::write(
            build.join("manifest.json"),
            r#"{"copy":"c","target":"web-preview","template":{"id":"t"}}"#,
        )
        .unwrap();
        std::fs::write(build.join("index.html"), "<p>hi").unwrap();
        std::fs::write(build.join("sub/a.wasm"), "x").unwrap();
        std::fs::write(root.path().join("secret.txt"), "no").unwrap();
        let l = link_for(&build).unwrap();
        assert_eq!(l.id.len(), 12);
        assert_eq!(l.copy.as_deref(), Some("c"));
        assert_eq!(l.template.as_deref(), Some("t"));
        assert!(resolve(&l, "").unwrap().ends_with("index.html"));
        assert!(resolve(&l, "sub/a.wasm").is_some());
        assert_eq!(
            content_type(&resolve(&l, "sub/a.wasm").unwrap()),
            "application/wasm"
        );
        for bad in [
            "../secret.txt",
            "sub/../../secret.txt",
            "/etc/passwd",
            "sub//a.wasm",
            "nope",
            "sub",
        ] {
            assert!(resolve(&l, bad).is_none(), "{bad}");
        }
        assert_eq!(
            l.id,
            link_for(&build).unwrap().id,
            "the same build, the same link"
        );
    }

    #[test]
    fn a_directory_without_a_manifest_is_not_a_build() {
        let d = tempfile::tempdir().unwrap();
        assert!(link_for(d.path()).is_err());
    }
}
