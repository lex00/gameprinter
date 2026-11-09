//! `gp game kit` and `gp game build`: a game's hooks as one Wasm module.
//!
//! The module is the interpreter, the presentation and one game's hook crate
//! compiled together. It is built from the module kit (the tool's sources for
//! those crates and the crates they need) and the package's files, with cargo
//! in a sandbox that can read nothing else and has no network, so
//! `include_bytes!` can only reach files the package ships.
//!
//! The kit lives inside each tool release. A build needs its toolchain and
//! nothing else: same kit, same package and same lock make the same bytes.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::json;

use crate::Outcome;

/// The crate that exports the page's calls, and what the closure starts from.
const TOP: &str = "gp-web";
/// The placeholder a game's hook crate replaces.
const SLOT: &str = "gp-ext-game";
/// A hook crate may depend on these and nothing else.
const ALLOWED_DEPS: [&str; 2] = ["gp-sim", "gp-cycles"];
/// The largest memory a module may declare: 256 MB (the loader's limit too).
const MAX_MEMORY: u64 = 256 * 1024 * 1024;
const TARGET: &str = "wasm32-unknown-unknown";

fn run(cmd: &mut Command, what: &str) -> Result<()> {
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

fn read_table(file: &Path) -> Result<toml::Table> {
    let text = fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    table(&text, file)
}

/// Every file under `dir` by relative path, skipping build output and links.
fn walk(dir: &Path) -> Result<BTreeMap<String, PathBuf>> {
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
fn write_tree(files: &BTreeMap<String, Vec<u8>>, to: &Path) -> Result<()> {
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

fn read_all(dir: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    walk(dir)?
        .into_iter()
        .map(|(rel, p)| Ok((rel, fs::read(&p)?)))
        .collect()
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
fn browser_manifest(file: &Path) -> Result<String> {
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

/// The tool's source folder: GP_TOOL_SRC, or where this gp was built from.
fn tool_src() -> PathBuf {
    std::env::var_os("GP_TOOL_SRC").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."),
        PathBuf::from,
    )
}

pub fn kit(out: &Path) -> Result<Outcome> {
    let src = tool_src();
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
    Ok(Outcome {
        json: json!({ "kit": out, "hash": hash, "crates": seen.len() }),
        text: format!(
            "module kit in {}: {} crates, hash {hash}",
            out.display(),
            seen.len()
        ),
        exit: 0,
    })
}

/// A hash of a folder's paths and contents.
fn tree_hash(dir: &Path) -> Result<String> {
    let mut h = blake3::Hasher::new();
    for (rel, bytes) in read_all(dir)? {
        h.update(format!("{} {} {rel}\n", blake3::hash(&bytes).to_hex(), bytes.len()).as_bytes());
    }
    Ok(h.finalize().to_hex().to_string())
}

/// The rules a hook crate is held to: its code is only what it ships, and it
/// builds the same anywhere. Returns each rule it breaks.
pub fn hook_rules(ext: &Path) -> Result<Vec<String>> {
    let m = read_table(&ext.join("Cargo.toml"))?;
    let mut bad = Vec::new();
    if ext.join("build.rs").exists() || m.get("package").and_then(|p| p.get("build")).is_some() {
        bad.push("it has a build script".to_owned());
    }
    if m.get("lib").and_then(|l| l.get("proc-macro")).is_some()
        || m.get("lib").and_then(|l| l.get("proc_macro")).is_some()
    {
        bad.push("it is a proc macro".to_owned());
    }
    if m.get("target").is_some() {
        bad.push("it has target-specific dependencies".to_owned());
    }
    if m.get("build-dependencies").is_some() {
        bad.push("it has build dependencies".to_owned());
    }
    for (dep, _) in m
        .get("dependencies")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flatten()
    {
        if !ALLOWED_DEPS.contains(&dep.as_str()) {
            bad.push(format!(
                "it depends on `{dep}`; a hook crate may depend only on gp-sim and gp-cycles"
            ));
        }
    }
    Ok(bad)
}

/// The hook crate's manifest as the slot in the kit needs it.
fn slot_manifest(ext: &Path) -> Result<String> {
    let mut m = read_table(&ext.join("Cargo.toml"))?;
    m.remove("workspace");
    m.get_mut("package")
        .and_then(toml::Value::as_table_mut)
        .context("the hook crate has no [package]")?
        .insert("name".into(), SLOT.into());
    if let Some(d) = m
        .get_mut("dependencies")
        .and_then(toml::Value::as_table_mut)
    {
        for (dep, v) in d.iter_mut() {
            if let Some(t) = v.as_table_mut() {
                t.insert("path".into(), format!("../{dep}").into());
            }
        }
    }
    Ok(toml::to_string(&m)?)
}

fn toolchain_dir(kit: &Path) -> Result<PathBuf> {
    let channel = read_table(&kit.join("rust-toolchain.toml"))?
        .get("toolchain")
        .and_then(|t| t.get("channel"))
        .and_then(toml::Value::as_str)
        .context("the kit names no toolchain")?
        .to_owned();
    let out = Command::new("rustup")
        .args(["which", "--toolchain", &channel, "cargo"])
        .output()
        .context("starting rustup")?;
    if !out.status.success() {
        bail!(
            "toolchain {channel} isn't installed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let cargo = PathBuf::from(String::from_utf8(out.stdout)?.trim());
    Ok(cargo
        .ancestors()
        .nth(2)
        .context("cargo isn't in a toolchain folder")?
        .canonicalize()?)
}

/// `args` in the sandbox: it reads the work folder, the toolchain and the
/// system's own tools, writes only the work folder, and has no network.
/// There's no build without a sandbox.
fn sandboxed(work: &Path, toolchain: &Path, args: &[&str]) -> Result<Command> {
    sandboxed_program(work, toolchain, &toolchain.join("bin/cargo"), args)
}

fn sandboxed_program(
    work: &Path,
    toolchain: &Path,
    cargo: &Path,
    args: &[&str],
) -> Result<Command> {
    let cmd = if cfg!(target_os = "macos") {
        let quote = |p: &Path| format!("{:?}", p.display().to_string());
        let (w, t) = (quote(work), quote(toolchain));
        let profile = format!(
            r#"(version 1)
(allow default)
(deny network*)
(deny file-write*)
(deny file-read-data (subpath "/Users") (subpath "/Volumes") (subpath "/private/tmp") (subpath "/tmp") (subpath "/private/var/folders") (subpath "/opt") (subpath "/usr/local"))
(allow file-read-data (subpath {w}) (subpath {t}))
(allow file-write* (subpath {w}) (literal "/dev/null") (literal "/dev/dtracehelper"))
"#
        );
        let mut c = Command::new("/usr/bin/sandbox-exec");
        c.args(["-p", &profile]).arg(cargo);
        c
    } else if which("bwrap") {
        let mut c = Command::new("bwrap");
        c.args([
            "--unshare-all",
            "--die-with-parent",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--tmpfs",
            "/tmp",
        ]);
        c.args([
            "--ro-bind",
            "/usr",
            "/usr",
            "--symlink",
            "usr/lib",
            "/lib",
            "--symlink",
            "usr/bin",
            "/bin",
        ]);
        // The system's `cc` is a link through here on Debian and Ubuntu.
        if Path::new("/etc/alternatives").exists() {
            c.args(["--ro-bind", "/etc/alternatives", "/etc/alternatives"]);
        }
        if Path::new("/lib64").exists() {
            c.args(["--symlink", "usr/lib64", "/lib64"]);
        }
        c.arg("--ro-bind").arg(toolchain).arg(toolchain);
        c.arg("--bind").arg(work).arg(work);
        // The sandbox's own working folder; a build runs in its source.
        let start = work.join("src");
        c.arg("--chdir")
            .arg(if start.is_dir() {
                start.as_path()
            } else {
                work
            })
            .arg(cargo);
        c
    } else {
        bail!("no build sandbox here: gp game build needs sandbox-exec (macOS) or bwrap (Linux)");
    };
    let mut cmd = cmd;
    cmd.args(args).current_dir(work).env_clear();
    cmd.env(
        "PATH",
        format!("{}:/usr/bin:/bin", toolchain.join("bin").display()),
    )
    .env("HOME", work.join("home"))
    .env("CARGO_HOME", work.join("cargo-home"))
    .env("TMPDIR", work.join("tmp"))
    .env("RUSTC", toolchain.join("bin/rustc"))
    .env("RUSTDOC", toolchain.join("bin/rustdoc"))
    .env("CARGO_NET_OFFLINE", "true")
    .env("CARGO_INCREMENTAL", "0")
    .env("SOURCE_DATE_EPOCH", "0");
    Ok(cmd)
}

fn which(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(name).is_file()))
}

/// Where a build works: a fixed folder per game, so a rebuild reuses cargo's
/// output for what didn't change.
fn work_dir(name: &str) -> Result<PathBuf> {
    let home = std::env::var_os("GP_HOME").map_or_else(
        || std::env::var_os("HOME").map(|h| Path::new(&h).join(".gameprinter")),
        |h| Some(PathBuf::from(h)),
    );
    let work = home
        .context("no GP_HOME or HOME")?
        .join("work/module-build")
        .join(name.replace('/', "-"));
    for d in ["home", "cargo-home", "tmp"] {
        fs::create_dir_all(work.join(d))?;
    }
    Ok(work.canonicalize()?)
}

pub fn build(dir: &Path, kit_dir: &Path, out: &Path, update_lock: bool) -> Result<Outcome> {
    let dir = dir
        .canonicalize()
        .with_context(|| dir.display().to_string())?;
    let kit_dir = kit_dir
        .canonicalize()
        .with_context(|| format!("the module kit {}", kit_dir.display()))?;
    let manifest = read_table(&dir.join("package.toml"))?;
    let name = manifest
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(toml::Value::as_str)
        .context("package.toml names no package")?
        .to_owned();
    let hooks = manifest
        .get("hooks")
        .context("this package has no [hooks], so no module")?;
    let krate = hooks
        .get("crate")
        .and_then(toml::Value::as_str)
        .context("[hooks] names no crate")?;
    let models: Vec<&str> = hooks
        .get("models")
        .and_then(toml::Value::as_array)
        .context("[hooks] names no models")?
        .iter()
        .filter_map(toml::Value::as_str)
        .collect();
    let ext = dir.join(krate);
    let broken = hook_rules(&ext)?;
    if !broken.is_empty() {
        return Ok(Outcome {
            json: json!({ "refused": broken }),
            text: format!("{} refused: {}", ext.display(), broken.join("; ")),
            exit: 1,
        });
    }
    let lock_file = dir.join("Cargo.lock");
    if !update_lock && !lock_file.exists() {
        bail!(
            "{} is missing; make it once with --update-lock and commit it",
            lock_file.display()
        );
    }

    let toolchain = toolchain_dir(&kit_dir)?;
    let work = work_dir(&name)?;
    let src = work.join("src");
    let mut files = read_all(&kit_dir)?;
    let kit_lock = files
        .remove("Cargo.lock")
        .context("the kit has no Cargo.lock")?;
    files.retain(|rel, _| !rel.starts_with(&format!("crates/{SLOT}/")));
    for (rel, bytes) in read_all(&dir)? {
        // What a package may reach: its own files, and none is a ROM's.
        files.insert(format!("package/{rel}"), bytes);
    }
    for (rel, bytes) in read_all(&ext)? {
        if rel != "Cargo.toml" && rel != "Cargo.lock" {
            files.insert(format!("crates/{SLOT}/{rel}"), bytes);
        }
    }
    files.insert(
        format!("crates/{SLOT}/Cargo.toml"),
        slot_manifest(&ext)?.into_bytes(),
    );
    let lock = if update_lock {
        kit_lock
    } else {
        fs::read(&lock_file)?
    };
    files.insert("Cargo.lock".into(), lock);
    write_tree(&files, &src)?;

    if update_lock {
        let mut c = sandboxed(
            &work,
            &toolchain,
            &["metadata", "--format-version", "1", "--offline"],
        )?;
        c.current_dir(&src);
        run(&mut c, "cargo metadata in the sandbox")?;
        fs::write(&lock_file, fs::read(src.join("Cargo.lock"))?)?;
    }

    let flags = format!(
        "--remap-path-prefix={}=/gp-module -Clink-arg=--max-memory={MAX_MEMORY}",
        src.display()
    );
    let target_dir = work.join("target");
    let args = [
        "rustc",
        "-q",
        "--locked",
        "--offline",
        "-p",
        TOP,
        "--lib",
        "--release",
        "--target",
        TARGET,
        "--crate-type",
        "cdylib",
        "--config",
        "profile.release.lto=true",
        "--config",
        "profile.release.opt-level=\"s\"",
        "--config",
        "profile.release.codegen-units=1",
        "--config",
        "profile.release.strip=true",
        "--config",
        "profile.release.debug=false",
    ];
    let mut c = sandboxed(&work, &toolchain, &args)?;
    c.current_dir(&src)
        .env("RUSTFLAGS", flags)
        .env("CARGO_TARGET_DIR", &target_dir)
        .env("GP_HOOK_MODELS", models.join(","));
    run(&mut c, "the sandboxed build")?;

    let built = target_dir.join(format!("{TARGET}/release/gp_web.wasm"));
    let bytes = fs::read(&built).with_context(|| built.display().to_string())?;
    if let Some(p) = out.parent() {
        fs::create_dir_all(p)?;
    }
    fs::write(out, &bytes)?;
    let hash = blake3::hash(&bytes).to_hex().to_string();
    Ok(Outcome {
        json: json!({ "module": out, "hash": hash, "bytes": bytes.len(), "kit": tree_hash(&kit_dir)? }),
        text: format!("{}: {} bytes, hash {hash}", out.display(), bytes.len()),
        exit: 0,
    })
}

pub fn hash(file: &Path) -> Result<Outcome> {
    let bytes = fs::read(file).with_context(|| file.display().to_string())?;
    let h = blake3::hash(&bytes).to_hex().to_string();
    Ok(Outcome {
        json: json!({ "hash": h }),
        text: h,
        exit: 0,
    })
}

/// The bytes of each data segment and custom section of a Wasm file, by name.
pub fn data_parts(wasm: &[u8]) -> Result<Vec<(String, &[u8])>> {
    fn leb(b: &[u8], at: &mut usize) -> Result<u64> {
        let (mut v, mut s) = (0u64, 0);
        loop {
            let x = *b
                .get(*at)
                .ok_or_else(|| anyhow!("the file ends inside a number"))?;
            *at += 1;
            v |= u64::from(x & 0x7f) << s;
            if x & 0x80 == 0 {
                return Ok(v);
            }
            s += 7;
            if s > 63 {
                bail!("a number is too long");
            }
        }
    }
    /// Skips a constant expression, up to and including its `end`.
    fn skip_expr(b: &[u8], at: &mut usize) -> Result<()> {
        loop {
            let op = *b
                .get(*at)
                .ok_or_else(|| anyhow!("the file ends inside an expression"))?;
            *at += 1;
            match op {
                0x0b => return Ok(()),
                0x41 | 0x42 | 0x23 => {
                    leb(b, at)?;
                }
                _ => bail!(
                    "a data segment's offset uses instruction {op:#x}, which this scan doesn't read"
                ),
            }
        }
    }
    if wasm.get(..8) != Some(b"\0asm\x01\0\0\0") {
        bail!("not a Wasm module");
    }
    let mut out = Vec::new();
    let mut at = 8;
    while at < wasm.len() {
        let id = wasm[at];
        at += 1;
        let len = usize::try_from(leb(wasm, &mut at)?)?;
        let end = at
            .checked_add(len)
            .filter(|e| *e <= wasm.len())
            .context("a section runs past the file")?;
        let body = &wasm[at..end];
        match id {
            0 => {
                let mut p = 0;
                let n = usize::try_from(leb(body, &mut p)?)?;
                let name = body
                    .get(p..p + n)
                    .context("a custom section's name runs past it")?;
                out.push((
                    format!("custom section {}", String::from_utf8_lossy(name)),
                    &body[p + n..],
                ));
            }
            11 => {
                let mut p = 0;
                let count = leb(body, &mut p)?;
                for i in 0..count {
                    match leb(body, &mut p)? {
                        0 => skip_expr(body, &mut p)?,
                        1 => {}
                        2 => {
                            leb(body, &mut p)?;
                            skip_expr(body, &mut p)?;
                        }
                        k => bail!("a data segment of kind {k}"),
                    }
                    let n = usize::try_from(leb(body, &mut p)?)?;
                    let bytes = body
                        .get(p..p + n)
                        .context("a data segment runs past its section")?;
                    out.push((format!("data segment {i}"), bytes));
                    p += n;
                }
            }
            _ => {}
        }
        at = end;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wasm_with(data: &[u8]) -> Vec<u8> {
        let mut w = b"\0asm\x01\0\0\0".to_vec();
        // A custom section named "n" holding `abc`, then one active segment.
        w.extend([0, 5, 1, b'n', b'a', b'b', b'c']);
        let mut seg = vec![1, 0, 0x41, 0, 0x0b, data.len() as u8];
        seg.extend(data);
        w.push(11);
        w.push(seg.len() as u8);
        w.extend(seg);
        w
    }

    #[test]
    fn data_parts_reads_segments_and_custom_sections() {
        let w = wasm_with(b"hello");
        let parts = data_parts(&w).unwrap();
        assert_eq!(parts[0].1, b"abc");
        assert_eq!(parts[1].1, b"hello");
    }

    #[test]
    fn data_parts_refuses_a_cut_file_and_other_files() {
        let w = wasm_with(b"hello");
        assert!(data_parts(&w[..w.len() - 2]).is_err());
        assert!(data_parts(b"not wasm").is_err());
    }

    fn crate_with(toml_text: &str) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("Cargo.toml"), toml_text).unwrap();
        d
    }

    /// What the sandbox lets a build reach: its work folder, and not a file
    /// beside it in the home folder, nor the network.
    #[test]
    fn the_sandbox_reads_only_its_folder_and_has_no_network() {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        let outer = tempfile::tempdir_in(&home).unwrap();
        let outer = outer.path().canonicalize().unwrap();
        let work = outer.join("work");
        fs::create_dir_all(&work).unwrap();
        let toolchain = outer.join("toolchain");
        fs::create_dir_all(&toolchain).unwrap();
        fs::write(work.join("inside"), "yes").unwrap();
        fs::write(outer.join("outside"), "secret").unwrap();
        let try_it = |program: &str, arg: &str| {
            sandboxed_program(&work, &toolchain, Path::new(program), &[arg])
                .unwrap()
                .output()
                .unwrap()
        };
        let cat = "/bin/cat";
        let inside = try_it(cat, work.join("inside").to_str().unwrap());
        assert!(inside.status.success(), "{inside:?}");
        assert_eq!(inside.stdout, b"yes");
        let outside = try_it(cat, outer.join("outside").to_str().unwrap());
        assert!(
            !outside.status.success(),
            "read a file outside: {outside:?}"
        );
        let net = try_it("/usr/bin/curl", "https://example.com/");
        assert!(!net.status.success(), "reached the network: {net:?}");
    }

    const OK: &str = "[package]\nname = \"x\"\n[dependencies]\ngp-sim = { path = \"../gp-sim\" }\n";

    #[test]
    fn a_plain_hook_crate_passes() {
        assert!(hook_rules(crate_with(OK).path()).unwrap().is_empty());
    }

    #[test]
    fn each_broken_rule_is_named() {
        for (extra, word) in [
            ("reqwest = \"1\"\n", "reqwest"),
            ("[build-dependencies]\ncc = \"1\"\n", "build dependencies"),
            (
                "[target.'cfg(unix)'.dependencies]\nlibc = \"1\"\n",
                "target-specific",
            ),
            ("[lib]\nproc-macro = true\n", "proc macro"),
        ] {
            let bad = hook_rules(crate_with(&format!("{OK}{extra}")).path()).unwrap();
            assert!(bad.iter().any(|b| b.contains(word)), "{extra}: {bad:?}");
        }
        let d = crate_with(OK);
        fs::write(d.path().join("build.rs"), "fn main() {}").unwrap();
        assert!(hook_rules(d.path()).unwrap()[0].contains("build script"));
    }
}
