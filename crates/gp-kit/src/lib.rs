//! The module kit: the tool's sources for the crates a game's Wasm module is
//! built from (gp-web and its path dependencies, cut to what a browser build
//! uses), its lock, its toolchain and every crate it needs vendored, so
//! `gp game build` compiles it with no network. `gp game kit` makes it from
//! a source build; the release build makes it with this crate's own binary
//! (`gp-kit SRC OUT`), which compiles in seconds, and carries it in gp.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

/// The crate that exports the page's calls, and what the closure starts from.
pub const TOP: &str = "gp-web";
/// The target a module is built for.
pub const TARGET: &str = "wasm32-unknown-unknown";

/// Runs `cmd`, failing with its output when it fails.
pub fn run(cmd: &mut Command, what: &str) -> Result<()> {
    let out = cmd.output().with_context(|| format!("starting {what}"))?;
    if !out.status.success() {
        bail!(
            "{what} failed:\n{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(())
}

fn table(text: &str, file: &Path) -> Result<toml::Table> {
    text.parse()
        .with_context(|| format!("parsing {}", file.display()))
}

/// A TOML file as a table.
pub fn read_table(file: &Path) -> Result<toml::Table> {
    let text = fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    table(&text, file)
}

/// Every file under `dir` by relative path, skipping build output and links.
pub fn walk(dir: &Path) -> Result<BTreeMap<String, PathBuf>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).with_context(|| d.display().to_string())? {
            let e = e?;
            let name = e.file_name().to_string_lossy().into_owned();
            // A crate's build output; a folder of source can be named target too.
            let output = name == "target" && (d == dir || d.join("Cargo.toml").exists());
            if output || matches!(name.as_str(), ".git" | ".DS_Store" | "node_modules") {
                continue;
            }
            let (p, t) = (e.path(), e.file_type()?);
            if t.is_dir() {
                stack.push(p);
            } else if t.is_file() {
                let rel = p.strip_prefix(dir)?.to_string_lossy().replace('\\', "/");
                out.insert(rel, p);
            }
        }
    }
    Ok(out)
}

/// Writes only what differs and removes what isn't wanted, so cargo sees
/// unchanged files as unchanged.
pub fn write_tree(files: &BTreeMap<String, Vec<u8>>, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    let old = walk(to)?;
    for (rel, p) in &old {
        if !files.contains_key(rel) {
            fs::remove_file(p)?;
        }
    }
    for (rel, bytes) in files {
        let p = to.join(rel);
        if old.contains_key(rel) && fs::read(&p)? == *bytes {
            continue;
        }
        fs::create_dir_all(p.parent().context("a file has a folder")?)?;
        fs::write(&p, bytes)?;
    }
    Ok(())
}

/// Every file under `dir` with its bytes, by relative path.
pub fn read_all(dir: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    walk(dir)?
        .into_iter()
        .map(|(rel, p)| Ok((rel, fs::read(&p)?)))
        .collect()
}

/// A hash of a folder's paths and contents.
pub fn tree_hash(dir: &Path) -> Result<String> {
    let mut h = blake3::Hasher::new();
    for (rel, bytes) in read_all(dir)? {
        h.update(format!("{} {} {rel}\n", blake3::hash(&bytes).to_hex(), bytes.len()).as_bytes());
    }
    Ok(h.finalize().to_hex().to_string())
}

fn path_deps(crates: &Path, name: &str) -> Result<Vec<String>> {
    let m = read_table(&crates.join(name).join("Cargo.toml"))?;
    let mut out = Vec::new();
    for key in ["dependencies", "build-dependencies"] {
        for (dep, v) in m
            .get(key)
            .and_then(toml::Value::as_table)
            .into_iter()
            .flatten()
        {
            if v.get("path").is_some() {
                out.push(dep.clone());
            }
        }
    }
    Ok(out)
}

/// A kit crate's manifest with everything a browser build doesn't use taken
/// out, so the kit needs neither those crates nor what they depend on.
pub fn browser_manifest(file: &Path) -> Result<String> {
    let mut m = read_table(file)?;
    for key in ["dev-dependencies", "bin", "test", "bench", "example"] {
        m.remove(key);
    }
    if let Some(t) = m.get_mut("target").and_then(toml::Value::as_table_mut) {
        t.retain(|k, _| !k.contains("not(target_arch"));
    }
    if let Some(p) = m.get_mut("package").and_then(toml::Value::as_table_mut) {
        p.insert("autobins".into(), false.into());
    }
    Ok(toml::to_string(&m)?)
}

/// Whether `src` holds the kit's top crate.
pub fn has_kit_crates(src: &Path) -> bool {
    src.join("crates").join(TOP).join("Cargo.toml").is_file()
}

/// A kit [`make`] wrote: where, its [`tree_hash`] and how many crates of
/// the tool's it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Made {
    pub kit: PathBuf,
    pub hash: String,
    pub crates: usize,
}

/// The module kit in `out`, made from the tool's source in `src`. Runs
/// cargo metadata and cargo vendor offline: every crate the lock names
/// must be in cargo's cache (`cargo fetch --locked`).
pub fn make(src: &Path, out: &Path) -> Result<Made> {
    let crates = src.join("crates");
    let mut seen = BTreeSet::new();
    let mut todo = vec![TOP.to_owned()];
    while let Some(c) = todo.pop() {
        if seen.insert(c.clone()) {
            todo.extend(path_deps(&crates, &c)?);
        }
    }
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for c in &seen {
        let dir = crates.join(c);
        for (rel, p) in walk(&dir)? {
            if rel.starts_with("src/bin/")
                || rel.starts_with("tests/")
                || rel.starts_with("benches/")
            {
                continue;
            }
            let bytes = if rel == "Cargo.toml" {
                browser_manifest(&p)?.into_bytes()
            } else {
                fs::read(&p)?
            };
            files.insert(format!("crates/{c}/{rel}"), bytes);
        }
    }
    let mut root = read_table(&src.join("Cargo.toml"))?;
    let members: Vec<toml::Value> = seen.iter().map(|c| format!("crates/{c}").into()).collect();
    root.get_mut("workspace")
        .and_then(toml::Value::as_table_mut)
        .context("the tool's Cargo.toml has no [workspace]")?
        .insert("members".into(), members.into());
    files.insert("Cargo.toml".into(), toml::to_string(&root)?.into_bytes());
    files.insert("Cargo.lock".into(), fs::read(src.join("Cargo.lock"))?);
    let channel = read_table(&src.join("rust-toolchain.toml"))?
        .get("toolchain")
        .and_then(|t| t.get("channel"))
        .and_then(toml::Value::as_str)
        .context("rust-toolchain.toml has no channel")?
        .to_owned();
    files.insert(
        "rust-toolchain.toml".into(),
        format!("[toolchain]\nchannel = \"{channel}\"\ntargets = [\"{TARGET}\"]\n").into_bytes(),
    );
    write_tree(&files, out)?;
    let out = out.canonicalize()?;
    // Drops the lock's entries the kit doesn't use, keeping every version.
    run(
        Command::new("cargo")
            .args(["metadata", "--format-version", "1", "--offline"])
            .current_dir(&out),
        "cargo metadata",
    )?;
    let vendored = Command::new("cargo")
        .args([
            "vendor",
            "--locked",
            "--offline",
            "--versioned-dirs",
            "vendor",
        ])
        .current_dir(&out)
        .output()
        .context("starting cargo vendor")?;
    if !vendored.status.success() {
        bail!(
            "cargo vendor failed:\n{}",
            String::from_utf8_lossy(&vendored.stderr)
        );
    }
    fs::create_dir_all(out.join(".cargo"))?;
    fs::write(
        out.join(".cargo/config.toml"),
        "[source.crates-io]\nreplace-with = \"vendored-sources\"\n\n[source.vendored-sources]\ndirectory = \"vendor\"\n",
    )?;
    let hash = tree_hash(&out)?;
    Ok(Made {
        kit: out,
        hash,
        crates: seen.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The kit's crates are gp-web and the path dependencies it reaches,
    /// and never a crate only the native tool uses: gp-kit itself, gp-cli.
    #[test]
    fn the_kit_closure_is_gp_web_and_its_path_dependencies() {
        let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut seen = BTreeSet::new();
        let mut todo = vec![TOP.to_owned()];
        while let Some(c) = todo.pop() {
            if seen.insert(c.clone()) {
                todo.extend(path_deps(&crates, &c).unwrap());
            }
        }
        assert!(
            seen.contains("gp-web") && seen.contains("gp-sim"),
            "{seen:?}"
        );
        assert!(
            !seen.contains("gp-kit") && !seen.contains("gp-cli"),
            "{seen:?}"
        );
        assert!(has_kit_crates(&crates.join("..")));
    }

    #[test]
    fn the_tree_hash_follows_paths_and_bytes() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("kit");
        let files = |pairs: &[(&str, &str)]| -> BTreeMap<String, Vec<u8>> {
            pairs
                .iter()
                .map(|(p, b)| ((*p).to_owned(), b.as_bytes().to_vec()))
                .collect()
        };
        write_tree(&files(&[("x/a", "1"), ("b", "2")]), &a).unwrap();
        let first = tree_hash(&a).unwrap();
        write_tree(&files(&[("x/a", "1"), ("b", "2")]), &a).unwrap();
        assert_eq!(tree_hash(&a).unwrap(), first);
        write_tree(&files(&[("x/a", "1"), ("c", "2")]), &a).unwrap();
        assert_ne!(tree_hash(&a).unwrap(), first);
        assert!(
            !a.join("b").exists(),
            "write_tree keeps a file it wasn't given"
        );
    }
}
