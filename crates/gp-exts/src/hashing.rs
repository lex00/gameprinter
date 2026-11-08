// What a hook crate's hash covers, shared by build.rs (through `include!`)
// and the crate's tests: its Cargo.toml, its build.rs if it has one, and
// every `.rs` file under src/, in path order. Untracked files an editor or
// a file browser leaves (dotfiles, backups) and target/ are left out, and
// symlinks aren't followed, so the hash changes when the crate's code
// changes and not otherwise.

/// The files a hook crate's hash covers, sorted.
pub fn hook_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    fn rs_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            let hidden = p
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'));
            let Ok(meta) = std::fs::symlink_metadata(&p) else {
                continue;
            };
            if hidden || meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                rs_files(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    for top in ["Cargo.toml", "build.rs"] {
        let p = dir.join(top);
        if p.is_file() {
            out.push(p);
        }
    }
    rs_files(&dir.join("src"), &mut out);
    out.sort();
    out
}

/// The BLAKE3 hash of each file's path (relative to `dir`) and bytes, in
/// order. An unreadable file is an error, never hashed as empty.
pub fn hook_hash(dir: &std::path::Path, files: &[std::path::PathBuf]) -> String {
    let mut h = blake3::Hasher::new();
    for p in files {
        let rel = p.strip_prefix(dir).unwrap_or(p);
        h.update(rel.to_string_lossy().replace('\\', "/").as_bytes());
        h.update(&[0]);
        let bytes = std::fs::read(p).unwrap_or_else(|e| panic!("reading {}: {e}", p.display()));
        h.update(&bytes);
        h.update(&[0]);
    }
    h.finalize().to_hex().to_string()
}

/// The `version` of the `[package]` table of a Cargo.toml, following
/// `version.workspace = true` to the workspace root's `[workspace.package]`.
pub fn package_version(manifest: &str, workspace: &str) -> String {
    fn table_value(text: &str, table: &str, key: &str) -> Option<String> {
        let mut inside = false;
        for line in text.lines() {
            let l = line.trim();
            if l.starts_with('[') {
                inside = l == format!("[{table}]");
                continue;
            }
            if !inside {
                continue;
            }
            let Some(rest) = l.strip_prefix(key) else {
                continue;
            };
            let rest = rest.trim_start();
            if let Some(v) = rest.strip_prefix('=') {
                return Some(v.trim().trim_matches('"').to_owned());
            }
            if rest.starts_with(".workspace") {
                return Some("(workspace)".into());
            }
        }
        None
    }
    match table_value(manifest, "package", "version").as_deref() {
        Some("(workspace)") => table_value(workspace, "workspace.package", "version")
            .unwrap_or_else(|| panic!("the workspace has no [workspace.package] version")),
        Some(v) => v.to_owned(),
        None => panic!("a hook crate's Cargo.toml has no [package] version"),
    }
}
