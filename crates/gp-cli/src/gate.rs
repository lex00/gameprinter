//! `gp model gate`: a patch to a game in the `games` repo, held to the
//! corpus. A worktree of the repo in the gate's home is put at the branch
//! tip and scored; the patch is applied there, the game's module built for
//! its hooks, the package scanned for what it must not hold, and the corpus
//! scored again on the patched game through the patched module. A patch that
//! leaves any clip worse fails, and what passes lands on the branch. The
//! old gate's queue, build of gp per commit and repo-wide guards are gone:
//! gp is the running binary, and a patch only ever reaches the game's files.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use clap::Args;
use gp_evidence::Paths;
use serde_json::{Value, json};

use crate::Outcome;
use crate::clip_score::rom_of;
use crate::corpus_score::{CorpusCmd, lift_into};

/// Tries before giving up on a tip that keeps moving.
const TIP_TRIES: usize = 5;

#[derive(Args)]
pub struct GateArgs {
    /// The game's package folder in a checkout of the games repo.
    pub game: PathBuf,
    /// The patch: `git diff` output, or a `git format-patch` mail, which
    /// keeps its own message and author. It may only change the game's files.
    #[arg(long)]
    pub patch: PathBuf,
    /// The run in the gate's home the corpus's clips are made in.
    #[arg(long, default_value = "gate")]
    pub run: String,
    /// The branch to land on (default: the one checked out where GAME is).
    #[arg(long)]
    pub onto: Option<String>,
    /// The gate's own home (default: $GP_GATE_HOME, else ~/.gameprinter/gate).
    /// Its runs, scores, modules and worktree are kept there.
    #[arg(long)]
    pub home: Option<PathBuf>,
    /// The commit message, for a patch that isn't a format-patch mail.
    #[arg(long, short)]
    pub message: Option<String>,
    /// Say PASS or FAIL and land nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// A clip the patch removes from the corpus on purpose: its ID, or a
    /// pattern with `*`. A patch that removes a clip it doesn't name fails,
    /// since no score compares a clip that is gone.
    #[arg(long = "removes", value_name = "ID")]
    pub removes: Vec<String>,
}

/// What every try reads.
struct Ctx {
    repo: PathBuf,
    /// The game's folder inside the repo.
    rel: PathBuf,
    /// The gate's worktree.
    tree: PathBuf,
    home: PathBuf,
    onto: String,
    run: String,
    /// Paths with `home` the gate's, so its runs stay out of the person's.
    paths: Paths,
    /// `gp game kit` output for this run.
    kit: PathBuf,
    kit_hash: String,
}

/// `Err` with git's own words.
fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let o = Command::new("git").arg("-C").arg(dir).args(args).output()?;
    if !o.status.success() {
        bail!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&o.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&o.stdout).trim().to_owned())
}

/// `gp` itself as a command, with the gate's home as its $GP_HOME.
fn self_cmd(c: &Path) -> Result<Command> {
    let mut cmd = Command::new(std::env::current_exe()?);
    cmd.env("GP_HOME", c);
    Ok(cmd)
}

/// The module for the game in folder `game`, built once for its hook code
/// and the kit; `None` for a game with no hooks.
fn module_at(c: &Ctx, game: &Path, lines: &mut Vec<String>) -> Result<Option<PathBuf>> {
    let Some(ext) = crate::package::hook_code(game)? else {
        return Ok(None);
    };
    let key = blake3::hash(format!("{ext}\n{}", c.kit_hash).as_bytes()).to_hex();
    let out = c.home.join("modules").join(format!("{key}.wasm"));
    if out.is_file() {
        lines.push(format!("module for hook code {} reused", &ext[..12]));
        return Ok(Some(out));
    }
    fs::create_dir_all(out.parent().expect("a parent"))?;
    let started = std::time::Instant::now();
    let tmp = out.with_extension("tmp");
    let o = self_cmd(&c.home)?
        .args(["game", "build"])
        .arg(game)
        .arg("--kit")
        .arg(&c.kit)
        .arg("-o")
        .arg(&tmp)
        .output()?;
    if !o.status.success() {
        bail!(
            "gp game build: {}{}",
            String::from_utf8_lossy(&o.stdout).trim(),
            String::from_utf8_lossy(&o.stderr).trim()
        );
    }
    fs::rename(&tmp, &out)?;
    lines.push(format!(
        "module built for hook code {} in {} s",
        &ext[..12],
        started.elapsed().as_secs()
    ));
    Ok(Some(out))
}

/// The lifted assets for the game in `game`, by its asset map: a patch that
/// changes the map changes the lift.
fn assets_for(c: &Ctx, game: &Path, rom: &[u8]) -> Result<PathBuf> {
    let (_, map) = crate::corpus_score::asset_map(game, rom)?;
    let out = c
        .home
        .join("assets")
        .join(blake3::hash(map.as_bytes()).to_hex().as_str());
    lift_into(game, rom, &out)?;
    Ok(out)
}

/// A score's rows, in the corpus's order, and the file holding its whole
/// JSON.
fn score(
    c: &Ctx,
    game: &Path,
    module: Option<&Path>,
    assets: &Path,
    reuse: Option<&Path>,
    remeasure: bool,
    name: &str,
) -> Result<(Rows, PathBuf)> {
    let cmd = CorpusCmd::Score {
        game: game.to_owned(),
        run: c.run.clone(),
        module: module.map(Path::to_owned),
        model: None,
        assets: Some(assets.to_owned()),
        rom: None,
        only: Vec::new(),
        reuse: reuse.map(Path::to_owned),
        write: None,
        remeasure: remeasure.then(|| game.to_owned()),
    };
    let o = crate::corpus_score::run(&cmd, &c.paths)?;
    let dir = c.home.join("scores");
    fs::create_dir_all(&dir)?;
    let f = dir.join(format!("{name}-{}.json", c.run));
    fs::write(&f, serde_json::to_vec(&o.json)?)?;
    let clips = o.json["record"]["predicate"]["clips"]
        .as_array()
        .context("gp corpus score's record has no clips")?;
    let rows = clips
        .iter()
        .map(|v| (v["id"].as_str().unwrap_or("").to_owned(), Row::of(v)))
        .collect();
    Ok((rows, f))
}

/// The files of the package scan's needs-rewrite items, by count.
fn rewrites(v: &Value) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    for part in v["parts"].as_array().into_iter().flatten() {
        for f in part["files"].as_array().into_iter().flatten() {
            let n = f["rewrite"].as_u64().unwrap_or(0);
            if n > 0 {
                out.insert(f["file"].as_str().unwrap_or("").to_owned(), n);
            }
        }
    }
    out
}

/// The tree at `commit`, clean.
fn checkout(c: &Ctx, commit: &str) -> Result<()> {
    let tree = &c.tree;
    if tree.join(".git").exists() {
        let _ = git(tree, &["am", "--abort"]);
        let _ = git(tree, &["reset", "-q", "--hard"]);
        git(tree, &["checkout", "--detach", "--force", commit])?;
        // The gate's own tree: what an earlier patch added goes, in the
        // template or in gp's code.
        git(tree, &["clean", "-fdq"])?;
    } else {
        git(
            &c.repo,
            &[
                "worktree",
                "add",
                "--detach",
                &tree.to_string_lossy(),
                commit,
            ],
        )?;
    }
    Ok(())
}

/// A failure's reason and the phase it failed in.
struct Failed {
    why: String,
    phase: &'static str,
}

/// How a try went.
enum Trial {
    Fail(Failed),
    Pass { commit: String },
}

fn fail(why: String, phase: &'static str) -> Result<Trial> {
    Ok(Trial::Fail(Failed { why, phase }))
}

/// Applies the patch on the clean tree: as it is, else merged three ways.
fn apply(tree: &Path, patch: &Path, mail: bool) -> std::result::Result<(), String> {
    let p = patch.to_string_lossy();
    if mail {
        if git(tree, &["am", "--3way", &p]).is_ok() {
            return Ok(());
        }
        let _ = git(tree, &["am", "--abort"]);
        return Err("the patch doesn't apply on the tip".into());
    }
    if git(tree, &["apply", "--check", &p]).is_ok() {
        git(tree, &["apply", "--index", &p]).map_err(|e| format!("{e:#}"))?;
        return Ok(());
    }
    git(tree, &["apply", "--3way", &p]).map_err(|_| {
        let _ = git(tree, &["reset", "-q", "--hard"]);
        "the patch doesn't apply on the tip, and a three-way merge conflicts".to_owned()
    })?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn trial(
    c: &Ctx,
    a: &GateArgs,
    tip: &str,
    (tip_rows, tip_json): (&Rows, &Path),
    tip_model: &str,
    scan_before: &(BTreeMap<String, u64>, u8),
    rom: &[u8],
    lines: &mut Vec<String>,
) -> Result<Trial> {
    let tree = &c.tree;
    let game = tree.join(&c.rel);
    let text = fs::read(&a.patch).with_context(|| a.patch.display().to_string())?;
    let mail = text.starts_with(b"From ");
    if let Err(why) = apply(tree, &a.patch, mail) {
        return fail(why, "patch");
    }
    // A plain patch stays staged until its checks pass, so what a check
    // refuses (a ROM's bytes in the package) never becomes a commit.
    let files: Vec<String> = git(tree, &["diff", "--cached", "--name-only", tip])?
        .lines()
        .map(str::to_owned)
        .collect();
    let rel = c.rel.to_string_lossy().into_owned();
    if let Some(f) = files.iter().find(|f| !f.starts_with(&format!("{rel}/"))) {
        return fail(format!("the patch changes {f}, outside the game"), "patch");
    }
    let removed = removed_clips(tree, &files, |f| {
        git(tree, &["show", &format!("{tip}:{f}")]).ok()
    });
    if let Some(why) = unnamed_removals(&removed, &a.removes) {
        return fail(why, "removed clips");
    }
    let module = match module_at(c, &game, lines) {
        Ok(m) => m,
        Err(e) => return fail(format!("{e:#}"), "module"),
    };
    let wasm: Vec<PathBuf> = module.iter().cloned().collect();
    let after = crate::package::scan::run(&game, &[], &wasm, &c.paths)?;
    let now = rewrites(&after.json);
    let new: Vec<String> = now
        .iter()
        .filter(|(f, n)| scan_before.0.get(*f).is_none_or(|b| *b < **n))
        .map(|(f, n)| format!("{f} ({n})"))
        .collect();
    if !new.is_empty() || (after.exit != 0 && scan_before.1 == 0) {
        let found: Vec<&str> = after
            .text
            .lines()
            .filter(|l| l.contains("NEEDS REWRITE"))
            .collect();
        return fail(
            format!(
                "gp package scan: new needs-rewrite items: {}{}",
                new.join("; "),
                found.join("; ")
            ),
            "package scan",
        );
    }
    let assets = assets_for(c, &game, rom)?;
    let check = crate::model::check(&game, Some(&assets))?;
    if check.exit != 0 {
        return fail(
            format!("gp model check: {}", check.text.trim()),
            "model check",
        );
    }
    let (rows, _) = score(
        c,
        &game,
        module.as_deref(),
        &assets,
        Some(tip_json),
        true,
        "patched",
    )?;
    let text_now = fs::read_to_string(game.join(gp_model::MODEL_FILE)).unwrap_or_default();
    let parts = changed_parts(tip_model, &text_now);
    // A patch to the model's presentation alone is held to the picture too.
    let picture = files
        .iter()
        .all(|f| f == &format!("{rel}/model.toml") || f.starts_with(&format!("{rel}/themes/")))
        && parts.as_ref().is_some_and(|p| {
            p.iter()
                .all(|x| x == "present" || x.starts_with("present."))
        });
    if picture && let Some(w) = picture_worse(tip_rows, &rows) {
        return fail(w, "picture");
    }
    if let Some(w) = worse(tip_rows, &rows) {
        return fail(w, "corpus");
    }
    if !mail {
        let msg = a
            .message
            .clone()
            .unwrap_or_else(|| "A patch, by gp model gate".into());
        git(tree, &["commit", "-q", "-m", &msg])?;
    }
    // The curves measured again under a mapping the patch changed: the
    // corpus's new curves, version and record land with it, in a commit of
    // their own.
    let dirty = !git(tree, &["status", "--porcelain", "--", &rel])?.is_empty();
    if dirty {
        let corpus = fs::read_to_string(game.join("corpus.toml")).unwrap_or_default();
        let version = corpus
            .lines()
            .find_map(|l| l.trim().strip_prefix("version = "))
            .unwrap_or("")
            .trim_matches('"');
        let msg = format!(
            "{rel} corpus {version}: the curves measured again under the mapping as it now is (gp model gate)"
        );
        git(tree, &["add", "-A", "--", &rel])?;
        git(tree, &["commit", "-q", "-m", &msg])?;
        lines.push(msg);
    }
    Ok(Trial::Pass {
        commit: git(tree, &["rev-parse", "HEAD"])?,
    })
}

pub(crate) fn run(a: &GateArgs, paths: &Paths) -> Result<Outcome> {
    let game = fs::canonicalize(&a.game).with_context(|| a.game.display().to_string())?;
    let repo = PathBuf::from(git(&game, &["rev-parse", "--show-toplevel"])?);
    let rel = game.strip_prefix(&repo)?.to_owned();
    let onto = match &a.onto {
        Some(b) => b.clone(),
        None => git(&repo, &["rev-parse", "--abbrev-ref", "HEAD"])?,
    };
    let home = match &a.home {
        Some(h) => h.clone(),
        None => match std::env::var_os("GP_GATE_HOME") {
            Some(h) => PathBuf::from(h),
            None => dirs_home()?.join(".gameprinter").join("gate"),
        },
    };
    let home = std::path::absolute(&home)?;
    fs::create_dir_all(&home)?;
    if let Some(r) = gp_evidence::paths::enclosing_git_work_tree(&home) {
        bail!(
            "the gate's home is inside the git work tree at {}",
            r.display()
        );
    }
    // One gate at a time on a home: they share the worktree.
    let lock = fs::File::create(home.join("gate.lock"))?;
    lock.lock()?;
    let kit = home.join("kit");
    let o = self_cmd(&home)?
        .args(["--json", "game", "kit", "-o"])
        .arg(&kit)
        .output()?;
    if !o.status.success() {
        bail!("gp game kit: {}", String::from_utf8_lossy(&o.stderr).trim());
    }
    let kit_hash = serde_json::from_slice::<Value>(&o.stdout)
        .context("gp game kit gave no JSON")?["hash"]
        .as_str()
        .context("gp game kit gave no hash")?
        .to_owned();
    let c = Ctx {
        repo,
        rel,
        tree: home.join("tree"),
        paths: Paths {
            home: home.clone(),
            store: home.join("store"),
            roms: paths.roms.clone(),
        },
        home,
        onto,
        run: a.run.clone(),
        kit,
        kit_hash,
    };
    let rom = rom_of(&c.paths, &game, None)?;
    let mut lines = Vec::new();
    let mut last_fail = None;
    for _ in 0..TIP_TRIES {
        let tip = git(&c.repo, &["rev-parse", &c.onto])?;
        checkout(&c, &tip)?;
        let started = std::time::Instant::now();
        let tree_game = c.tree.join(&c.rel);
        let module = match module_at(&c, &tree_game, &mut lines) {
            Ok(m) => m,
            Err(e) => bail!("the tip's module doesn't build: {e:#}"),
        };
        let assets = assets_for(&c, &tree_game, &rom)?;
        let (tip_rows, tip_json) = score(
            &c,
            &tree_game,
            module.as_deref(),
            &assets,
            None,
            false,
            "tip",
        )?;
        lines.push(format!(
            "tip {}: {} clips scored in {} s",
            &tip[..8],
            tip_rows.len(),
            started.elapsed().as_secs()
        ));
        let wasm: Vec<PathBuf> = module.iter().cloned().collect();
        let scan = crate::package::scan::run(&tree_game, &[], &wasm, &c.paths)?;
        let scan_before = (rewrites(&scan.json), scan.exit);
        let model_text =
            fs::read_to_string(tree_game.join(gp_model::MODEL_FILE)).unwrap_or_default();
        let t = trial(
            &c,
            a,
            &tip,
            (&tip_rows, &tip_json),
            &model_text,
            &scan_before,
            &rom,
            &mut lines,
        )?;
        let (commit, moved_rows) = match t {
            Trial::Fail(f) => {
                last_fail = Some(f);
                break;
            }
            Trial::Pass { commit } => (commit, tip_rows.len()),
        };
        lines.push(format!(
            "{commit}: model check valid, package scan clean, corpus no worse ({moved_rows} clips held)"
        ));
        if a.dry_run {
            lines.push("--dry-run: PASS, nothing lands".into());
            let v = json!({"verdict": "PASS", "commit": commit, "tip": tip,
                "tip_score": tip_json, "lines": lines});
            return Ok(Outcome {
                json: v,
                text: lines.join("\n"),
                exit: 0,
            });
        }
        match land(&c, &tip, &commit) {
            Ok(()) => {
                lines.push(format!("PASS: landed {commit} on {}", c.onto));
                return Ok(Outcome {
                    json: json!({"verdict": "PASS", "commit": commit, "tip": tip,
                        "tip_score": tip_json, "lines": lines}),
                    text: lines.join("\n"),
                    exit: 0,
                });
            }
            Err((why, moved)) if moved => lines.push(format!("the tip moved: {why}; again")),
            Err((why, _)) => bail!("couldn't land {commit}: {why}"),
        }
    }
    let f = last_fail.unwrap_or(Failed {
        why: format!("the tip moved {TIP_TRIES} times"),
        phase: "tip",
    });
    lines.push(format!("FAIL ({}): {}", f.phase, f.why));
    Ok(Outcome {
        json: json!({"verdict": "FAIL", "phase": f.phase, "why": f.why, "lines": lines}),
        text: lines.join("\n"),
        exit: 1,
    })
}

fn dirs_home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("no home directory; give --home")
}

/// A corpus score's rows by clip, in the corpus's order.
type Rows = Vec<(String, Row)>;

struct Row {
    frames: u64,
    together: u64,
    error: Option<String>,
    bar_together: Option<bool>,
    bar_longest: Option<bool>,
    longest: Option<u64>,
    first: Option<(u64, u64, String)>,
    /// The row as gp corpus score gave it, for the status file.
    raw: Value,
}

impl Row {
    fn of(v: &Value) -> Self {
        let bar = |k: &str| v["bars"][k].as_bool();
        Self {
            frames: v["frames"].as_u64().unwrap_or(0),
            together: v["frames_together"].as_u64().unwrap_or(0),
            error: v["error"].as_str().map(str::to_owned),
            bar_together: bar("together"),
            bar_longest: bar("longest_stretch"),
            longest: v["stretches"]["longest_any"].as_u64(),
            first: v["first"].as_object().map(|f| {
                (
                    f.get("frame").and_then(Value::as_u64).unwrap_or(0),
                    f.get("original_frame").and_then(Value::as_u64).unwrap_or(0),
                    f.get("path")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                )
            }),
            raw: v.clone(),
        }
    }

    fn full(&self) -> bool {
        self.error.is_none() && self.frames > 0 && self.together == self.frames
    }
}

fn changed_parts(a: &str, b: &str) -> Option<Vec<String>> {
    let a: toml::Value = toml::from_str(a).ok()?;
    let b: toml::Value = toml::from_str(b).ok()?;
    let mut out = Vec::new();
    fn walk(
        a: Option<&toml::Value>,
        b: Option<&toml::Value>,
        at: &str,
        depth: usize,
        out: &mut Vec<String>,
    ) {
        if a == b {
            return;
        }
        match (a, b) {
            (Some(toml::Value::Table(x)), Some(toml::Value::Table(y))) if depth < 3 => {
                let keys: BTreeSet<&String> = x.keys().chain(y.keys()).collect();
                for k in keys {
                    let p = if at.is_empty() {
                        k.clone()
                    } else {
                        format!("{at}.{k}")
                    };
                    walk(x.get(k), y.get(k), &p, depth + 1, out);
                }
            }
            _ => out.push(at.to_owned()),
        }
    }
    walk(Some(&a), Some(&b), "", 0, &mut out);
    Some(out)
}

fn pairing_of(r: &Row) -> String {
    serde_json::from_value::<gp_interp::lockstep::Pairing>(r.raw["pairing"].clone())
        .map(|p| format!("; {}", p.says()))
        .unwrap_or_default()
}

fn worse(base: &Rows, new: &Rows) -> Option<String> {
    for (id, b) in base {
        let Some((_, n)) = new.iter().find(|(i, _)| i == id) else {
            continue;
        };
        let at = |r: &Row| match &r.first {
            Some((f, of, p)) => format!(
                ", first wrong at frame {f} of the clip (the original's {of}), {p}{}",
                pairing_of(r)
            ),
            None => String::new(),
        };
        if b.error.is_none()
            && let Some(e) = &n.error
        {
            return Some(format!("{id}: scored before, now can't be: {e}"));
        }
        if b.full() && !n.full() && n.raw["stopped"] == true {
            return Some(format!(
                "{id}: was 100% ({0}/{0} frames together), now splits{1}; its run stopped there",
                b.frames,
                at(n)
            ));
        }
        if b.full() && !n.full() {
            return Some(format!(
                "{id}: was 100% ({0}/{0} frames together), now {1}/{2}{3}",
                b.frames,
                n.together,
                n.frames,
                at(n)
            ));
        }
        if b.bar_together == Some(true) && n.bar_together == Some(false) {
            return Some(format!(
                "{id}: met the contract's together bar, now {}/{} frames{}",
                n.together,
                n.frames,
                at(n)
            ));
        }
        if b.bar_longest == Some(true) && n.bar_longest != Some(true) {
            return Some(format!(
                "{id}: met the contract's longest stretch ({} frames), now {} frames",
                b.longest.unwrap_or(0),
                n.longest.unwrap_or(0)
            ));
        }
    }
    None
}

fn picture_worse(base: &Rows, new: &Rows) -> Option<String> {
    for (id, b) in base {
        let Some((_, n)) = new.iter().find(|(i, _)| i == id) else {
            continue;
        };
        for layer in ["sprites", "background"] {
            let (bs, bt) = (
                &b.raw["screen"][layer]["same"],
                &b.raw["screen"][layer]["total"],
            );
            let (ns, nt) = (
                &n.raw["screen"][layer]["same"],
                &n.raw["screen"][layer]["total"],
            );
            if let (Some(bs), Some(ns)) = (bs.as_u64(), ns.as_u64())
                && bt == nt
                && ns < bs
            {
                return Some(format!(
                    "{id}: the picture's {layer} is the same on {ns} of {} frames, {bs} before",
                    nt.as_u64().unwrap_or(0)
                ));
            }
        }
    }
    None
}

fn recipe_ids(text: &str) -> BTreeSet<String> {
    toml::from_str::<toml::Value>(text)
        .ok()
        .and_then(|v| {
            v.get("recipes")?
                .as_table()
                .map(|t| t.keys().cloned().collect())
        })
        .unwrap_or_default()
}

pub(crate) fn matches(pattern: &str, id: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == id;
    }
    let mut rest = id;
    for (i, p) in parts.iter().enumerate() {
        if i == 0 {
            let Some(r) = rest.strip_prefix(p) else {
                return false;
            };
            rest = r;
        } else if i == parts.len() - 1 {
            return rest.ends_with(p);
        } else {
            let Some(at) = rest.find(p) else {
                return false;
            };
            rest = &rest[at + p.len()..];
        }
    }
    true
}

pub(crate) fn removed_clips(
    tree: &Path,
    files: &[String],
    old: impl Fn(&str) -> Option<String>,
) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    let corpora: BTreeSet<&String> = files
        .iter()
        .filter(|f| f.ends_with("/corpus.toml") || f.as_str() == "corpus.toml")
        .collect();
    for f in corpora {
        let Some(before) = old(f) else {
            continue;
        };
        let after = recipe_ids(&fs::read_to_string(tree.join(f)).unwrap_or_default());
        let gone: Vec<String> = recipe_ids(&before)
            .into_iter()
            .filter(|id| !after.contains(id))
            .collect();
        if !gone.is_empty() {
            out.push((f.clone(), gone));
        }
    }
    out
}

pub(crate) fn unnamed_removals(
    removed: &[(String, Vec<String>)],
    allowed: &[String],
) -> Option<String> {
    let mut each = Vec::new();
    let mut n = 0;
    for (file, ids) in removed {
        let left: Vec<&String> = ids
            .iter()
            .filter(|id| !allowed.iter().any(|p| matches(p, id)))
            .collect();
        if left.is_empty() {
            continue;
        }
        n += left.len();
        let shown: Vec<&str> = left.iter().take(8).map(|s| s.as_str()).collect();
        let more = if left.len() > 8 {
            format!(" and {} more", left.len() - 8)
        } else {
            String::new()
        };
        each.push(format!(
            "{} from {file}: {}{more}",
            left.len(),
            shown.join(", ")
        ));
    }
    if each.is_empty() {
        return None;
    }
    Some(format!(
        "the patch removes {n} clip{} from the corpus without naming {}: {}. No score compares a clip that is gone, so a patch that means to remove clips names them: `--removes ID` (repeated, or a pattern such as `dk.climb-sweep.*`)",
        if n == 1 { "" } else { "s" },
        if n == 1 { "it" } else { "them" },
        each.join("; ")
    ))
}

/// Moves the branch from `tip` to `commit`: a fast-forward where it is
/// checked out, so that checkout follows; else the ref, if still at `tip`.
fn land(c: &Ctx, tip: &str, commit: &str) -> std::result::Result<(), (String, bool)> {
    let porcelain = git(&c.repo, &["worktree", "list", "--porcelain"])
        .map_err(|e| (format!("{e:#}"), false))?;
    let mut at: Option<PathBuf> = None;
    let mut cur: Option<PathBuf> = None;
    for l in porcelain.lines() {
        if let Some(p) = l.strip_prefix("worktree ") {
            cur = Some(PathBuf::from(p));
        } else if l == format!("branch refs/heads/{}", c.onto) {
            at = cur.clone();
        }
    }
    let landed = match &at {
        Some(w) => git(w, &["merge", "--ff-only", "-q", commit]),
        None => git(
            &c.repo,
            &["update-ref", &format!("refs/heads/{}", c.onto), commit, tip],
        ),
    };
    landed.map(|_| ()).map_err(|e| {
        let now = git(&c.repo, &["rev-parse", &c.onto]).unwrap_or_default();
        (format!("{e:#}"), now != tip)
    })
}
