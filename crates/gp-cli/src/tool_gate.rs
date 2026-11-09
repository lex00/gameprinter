//! `gp tool gate`: a patch to the tool repo (`gameprinter`), held to the
//! tool's own checks and a Donkey Kong canary. A worktree in the gate's
//! home is put at the branch tip, the patch applied and built, `just check`
//! and the tests run, and the game gate's no-op scored through the patched
//! gp and the tip's: no row may get worse. What passes lands on the branch
//! and is published at $GP_HOME/bin/gp. Keeps the shape of `gate.rs` and
//! shares its apply, hook, tree and landing code.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use clap::Args;
use gp_evidence::Paths;
use serde_json::{Value, json};

use crate::Outcome;
use crate::clip_score::rom_of;
use crate::gate::{self, Failed, Patch, Row, Rows, git};
use crate::publish;

/// The canary game's folder in the games repo.
const CANARY: &str = "donkey-kong";
/// The profile the gate builds and publishes.
const PROFILE: &str = "fastdev";
const TIP_TRIES: usize = 5;

#[derive(Args)]
pub struct ToolGateArgs {
    /// The patch: `git diff` output, or a one-commit `git format-patch`
    /// mail, which keeps its own message and author.
    #[arg(long)]
    pub patch: PathBuf,
    /// The commit message, for a patch that isn't a format-patch mail.
    #[arg(long, short)]
    pub message: Option<String>,
    /// The branch to land on (default: the one checked out here).
    #[arg(long)]
    pub onto: Option<String>,
    /// The gate's home (default: $GP_GATE_HOME, else ~/.gameprinter/gate).
    /// Its worktree, target folder and canary runs are kept there.
    #[arg(long)]
    pub home: Option<PathBuf>,
    /// The games checkout the canary reads (default: the folder `games`
    /// beside this repo, else $GP_GAMES_DIR).
    #[arg(long)]
    pub games: Option<PathBuf>,
    /// Say PASS or FAIL and land nothing.
    #[arg(long)]
    pub dry_run: bool,
}

struct Cx {
    repo: PathBuf,
    onto: String,
    /// The gate's worktree of the tool, under the gate's home.
    tree: PathBuf,
    /// One target folder for every try, `<home>/tool/target`.
    target: PathBuf,
    canary_dir: PathBuf,
    /// The person's $GP_HOME: where builds are published.
    home: PathBuf,
    games_repo: PathBuf,
    games_tree: PathBuf,
    paths: Paths,
}

fn progress(s: impl AsRef<str>) {
    eprintln!("tool gate: {}", s.as_ref());
}

/// `cargo`, through the machine's build lock when it's installed.
fn cargo(c: &Cx) -> Command {
    let locked = c.home.join("bin/locked/cargo");
    let program = match std::env::var_os("GP_CARGO") {
        Some(p) => PathBuf::from(p),
        None if locked.is_file() => locked,
        None => PathBuf::from("cargo"),
    };
    Command::new(program)
}

/// `Err` carrying the command's last lines.
fn run_step(cmd: &mut Command, what: &str) -> std::result::Result<(), String> {
    let o = cmd.output().map_err(|e| format!("running {what}: {e}"))?;
    if o.status.success() {
        return Ok(());
    }
    let all = format!(
        "{}\n{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    let tail: Vec<&str> = all
        .lines()
        .filter(|l| !l.trim().is_empty())
        .rev()
        .take(25)
        .collect();
    Err(format!(
        "{what} failed:\n{}",
        tail.into_iter().rev().collect::<Vec<_>>().join("\n")
    ))
}

fn path_with_locked(c: &Cx) -> Result<std::ffi::OsString> {
    let locked = c.home.join("bin/locked");
    let path = std::env::var_os("PATH").unwrap_or_default();
    Ok(if locked.is_dir() {
        std::env::join_paths(std::iter::once(locked).chain(std::env::split_paths(&path)))?
    } else {
        path
    })
}

/// Builds gp in the gate's tree, in the gate's profile.
fn build(c: &Cx) -> std::result::Result<(), String> {
    progress("building gp");
    run_step(
        cargo(c)
            .args(["build", "-q", "--profile", PROFILE, "-p", "gp-cli"])
            .current_dir(&c.tree)
            .env("CARGO_TARGET_DIR", &c.target),
        "cargo build",
    )
}

fn built_gp(c: &Cx) -> PathBuf {
    c.target.join(PROFILE).join("gp")
}

/// `just check`, then the tests on their own: the patch can't remove them
/// by changing the justfile. Both from the tree, in the gate's profile.
fn check_and_test(c: &Cx) -> std::result::Result<Option<Failed>, String> {
    let path = path_with_locked(c).map_err(|e| e.to_string())?;
    progress("just check");
    let r = run_step(
        Command::new("just")
            .arg("check")
            .current_dir(&c.tree)
            .env("CARGO_TARGET_DIR", &c.target)
            .env("GP_PROFILE", PROFILE)
            .env("PATH", &path),
        "just check",
    );
    if let Err(why) = r {
        return Ok(Some(Failed {
            why,
            phase: "just check",
        }));
    }
    progress("cargo test");
    let r = run_step(
        cargo(c)
            .args(["test", "-q", "--workspace", "--profile", PROFILE])
            .current_dir(&c.tree)
            .env("CARGO_TARGET_DIR", &c.target),
        "cargo test",
    );
    if let Err(why) = r {
        return Ok(Some(Failed {
            why,
            phase: "tests",
        }));
    }
    Ok(None)
}

/// The rows of the corpus scored by `gp`: `gp model gate --score-only`
/// with a home of its own (`label`), since a home's caches are the build's.
fn scored(c: &Cx, gp: &Path, label: &str) -> std::result::Result<Value, String> {
    let game = c.games_tree.join(CANARY);
    let home = c.canary_dir.join(label);
    let o = Command::new(gp)
        .args(["--json", "model", "gate"])
        .arg(&game)
        .arg("--score-only")
        .args(["--run", "canary"])
        .arg("--home")
        .arg(&home)
        .output()
        .map_err(|e| format!("running {}: {e}", gp.display()))?;
    if !o.status.success() {
        return Err(format!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout).trim(),
            String::from_utf8_lossy(&o.stderr).trim()
        ));
    }
    serde_json::from_slice(&o.stdout).map_err(|e| format!("gp gave no JSON: {e}"))
}

fn rows_of(v: &Value) -> Rows {
    v["clips"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| (c["id"].as_str().unwrap_or("").to_owned(), Row::of(c)))
        .collect()
}

/// The canary, against the tip's scores (kept for this tip and games tip,
/// so a second patch on the same tip doesn't pay for them again).
fn canary(
    c: &Cx,
    tip: &str,
    tip_gp: &Path,
    patched_gp: &Path,
    patch_hash: &str,
    lines: &mut Vec<String>,
) -> std::result::Result<Option<Failed>, String> {
    let game = c.games_tree.join(CANARY);
    if rom_of(&c.paths, &game, None).is_err() {
        lines.push(format!(
            "canary skipped: no ROM for {CANARY} in {}",
            c.paths.roms.display()
        ));
        return Ok(None);
    }
    let games_head = git(&c.games_tree, &["rev-parse", "HEAD"]).map_err(|e| format!("{e:#}"))?;
    let base_file = c
        .canary_dir
        .join(format!("base-{}-{}.json", &tip[..12], &games_head[..12]));
    let base: Value = match fs::read(&base_file) {
        Ok(b) => serde_json::from_slice(&b).map_err(|e| format!("{e}"))?,
        Err(_) => {
            progress(format!("canary: scoring {CANARY} through the tip's gp"));
            let v = scored(c, tip_gp, &format!("tip-{}", &tip[..12]))?;
            fs::create_dir_all(&c.canary_dir).map_err(|e| e.to_string())?;
            fs::write(&base_file, serde_json::to_vec(&v).unwrap_or_default())
                .map_err(|e| e.to_string())?;
            v
        }
    };
    progress(format!("canary: scoring {CANARY} through the patched gp"));
    let new = match scored(c, patched_gp, &format!("patch-{patch_hash}")) {
        Ok(v) => v,
        Err(why) => {
            return Ok(Some(Failed {
                why: format!("the patched gp can't score {CANARY}: {why}"),
                phase: "canary",
            }));
        }
    };
    let (base_rows, new_rows) = (rows_of(&base), rows_of(&new));
    if base_rows.is_empty() {
        return Err("the tip's canary score has no rows".into());
    }
    for (id, _) in &base_rows {
        if !new_rows.iter().any(|(i, _)| i == id) {
            return Ok(Some(Failed {
                why: format!("{id}: no row through the patched gp"),
                phase: "canary",
            }));
        }
    }
    if let Some(why) = gate::worse(&base_rows, &new_rows) {
        return Ok(Some(Failed {
            why,
            phase: "canary",
        }));
    }
    lines.push(format!(
        "canary: {} clips of {CANARY} held through the patched gp",
        base_rows.len()
    ));
    Ok(None)
}

fn games_dir(a: &ToolGateArgs, repo: &Path) -> Result<PathBuf> {
    if let Some(g) = &a.games {
        return fs::canonicalize(g).with_context(|| g.display().to_string());
    }
    if let Some(g) = std::env::var_os("GP_GAMES_DIR") {
        return Ok(PathBuf::from(g));
    }
    // Beside this repo's main checkout, where a worktree's own parent isn't.
    let common = git(
        repo,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let g = Path::new(&common)
        .parent()
        .and_then(Path::parent)
        .map(|p| p.join("games"));
    match g {
        Some(g) if g.join(CANARY).is_dir() => Ok(g),
        _ => bail!(
            "no games checkout beside this repo for the canary; pass --games or set GP_GAMES_DIR"
        ),
    }
}

/// The tree's patch, or why it was refused. The tip's own build stays put
/// in the shared target folder and, as a published build, is the base of
/// the canary and a build every worker can use.
#[allow(clippy::too_many_arguments)]
fn trial(
    c: &Cx,
    a: &ToolGateArgs,
    patch: &Patch,
    tip: &str,
    tip_gp: &Path,
    patch_hash: &str,
    lines: &mut Vec<String>,
) -> Result<std::result::Result<String, Failed>> {
    macro_rules! refuse {
        ($why:expr, $phase:expr) => {
            return Ok(Err(Failed {
                why: $why,
                phase: $phase,
            }))
        };
    }
    let tree = &c.tree;
    if let Err(why) = gate::apply(tree, &patch.diff) {
        refuse!(why, "patch");
    }
    let files = git(tree, &["diff", "--cached", "--name-only", tip])?;
    if files.is_empty() {
        refuse!("the patch changes nothing".into(), "patch");
    }
    if let Some(f) = files
        .lines()
        .find(|f| f.starts_with(".githooks/") || f.starts_with("scripts/romguard/"))
    {
        refuse!(
            format!("the patch changes {f}, the ROM-blocking hook, which only the person edits"),
            "patch"
        );
    }
    let gate_gp = std::env::current_exe()?;
    if let Err(why) = gate::hook(tree, &gate_gp) {
        refuse!(format!("the ROM-blocking hook refuses it: {why}"), "hook");
    }
    if let Err(why) = build(c) {
        refuse!(why, "build");
    }
    match check_and_test(c) {
        Ok(None) => {}
        Ok(Some(f)) => return Ok(Err(f)),
        Err(e) => bail!(e),
    }
    let patched = built_gp(c);
    match canary(c, tip, tip_gp, &patched, patch_hash, lines) {
        Ok(None) => {}
        Ok(Some(f)) => return Ok(Err(f)),
        Err(e) => bail!(e),
    }
    let msg = a
        .message
        .clone()
        .unwrap_or_else(|| "A patch, by gp tool gate".into());
    if let Err(e) = gate::commit(tree, &msg, patch, &gate_gp) {
        refuse!(format!("{e:#}"), "commit");
    }
    Ok(Ok(git(tree, &["rev-parse", "HEAD"])?))
}

pub(crate) fn run(a: &ToolGateArgs, paths: &Paths) -> Result<Outcome> {
    let cwd = std::env::current_dir()?;
    let repo = PathBuf::from(git(&cwd, &["rev-parse", "--show-toplevel"])?);
    let onto = match &a.onto {
        Some(b) => b.clone(),
        None => git(&repo, &["rev-parse", "--abbrev-ref", "HEAD"])?,
    };
    let home = gate::gate_home(a.home.as_deref())?;
    let lock = fs::File::create(home.join("gate.lock"))?;
    lock.lock()?;
    let tool = home.join("tool");
    fs::create_dir_all(&tool)?;
    let games_repo = games_dir(a, &repo)?;
    let patch_bytes = fs::read(&a.patch).with_context(|| a.patch.display().to_string())?;
    let patch_hash = blake3::hash(&patch_bytes).to_hex()[..12].to_owned();
    let patch = match gate::read_patch(&a.patch, &tool)? {
        Ok(p) => p,
        Err(why) => {
            return Ok(fail(
                Failed {
                    why,
                    phase: "patch",
                },
                Vec::new(),
            ));
        }
    };
    let c = Cx {
        repo,
        onto,
        tree: tool.join("tree"),
        target: tool.join("target"),
        canary_dir: tool.join("canary"),
        home: paths.home.clone(),
        games_repo,
        games_tree: tool.join("games-tree"),
        paths: paths.clone(),
    };
    let games_head = git(&c.games_repo, &["rev-parse", "HEAD"])?;
    gate::checkout(&c.games_repo, &c.games_tree, &games_head)?;

    let mut lines = Vec::new();
    let mut failed = None;
    for _ in 0..TIP_TRIES {
        let tip = git(&c.repo, &["rev-parse", &c.onto])?;
        gate::checkout(&c.repo, &c.tree, &tip)?;
        let published = paths.home.join("bin").join(format!("gp-{}", &tip[..12]));
        let tip_gp = if published.is_file() {
            lines.push(format!("tip {}: using its published build", &tip[..12]));
            published
        } else {
            if let Err(why) = build(&c) {
                bail!("the tip doesn't build: {why}");
            }
            publish::publish(&c.home, &built_gp(&c), &tip, &c.repo, &mut lines)?;
            published
        };
        match trial(&c, a, &patch, &tip, &tip_gp, &patch_hash, &mut lines)? {
            Err(f) => {
                failed = Some(f);
                gate::clean(&c.tree);
                break;
            }
            Ok(commit) => {
                if a.dry_run {
                    lines.push("--dry-run: PASS, nothing lands".into());
                    return Ok(Outcome::ok(
                        json!({"verdict": "PASS", "commit": commit, "tip": tip, "lines": lines}),
                        lines.join("\n"),
                    ));
                }
                match gate::land(&c.repo, &c.onto, &tip, &commit) {
                    Ok(()) => {
                        lines.push(format!("PASS: landed {commit} on {}", c.onto));
                        publish::publish(&c.home, &built_gp(&c), &commit, &c.repo, &mut lines)?;
                        return Ok(Outcome::ok(
                            json!({"verdict": "PASS", "commit": commit, "tip": tip, "lines": lines}),
                            lines.join("\n"),
                        ));
                    }
                    Err((why, moved)) if moved => {
                        lines.push(format!("the tip moved: {why}; again"));
                    }
                    Err((why, _)) => bail!("couldn't land {commit}: {why}"),
                }
            }
        }
    }
    let f = failed.unwrap_or(Failed {
        why: format!("the tip moved {TIP_TRIES} times"),
        phase: "tip",
    });
    Ok(fail(f, lines))
}

fn fail(f: Failed, mut lines: Vec<String>) -> Outcome {
    lines.push(format!("FAIL ({}): {}", f.phase, f.why));
    Outcome {
        json: json!({"verdict": "FAIL", "phase": f.phase, "why": f.why, "lines": lines}),
        text: lines.join("\n"),
        exit: 1,
    }
}
