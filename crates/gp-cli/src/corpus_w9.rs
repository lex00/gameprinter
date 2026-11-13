//! `gp corpus show`, `diff` and `verify`, as game-printer's
//! scoring_corpus.rs had them (wave 9 of the port), over gameprinter's
//! corpus plans and its `gp corpus score` remaking of clips.
//!
//! What gameprinter's layout changed: a corpus is a game's package folder
//! (a corpus plan) or a clip set; `diff`'s REV:DIR takes the folder out of
//! git into a scratch folder and reads it as a folder, so a plan reads the
//! same at a revision as on disk. `verify` makes the clips again through
//! the game's module as `gp corpus score` does (game-printer used a capture
//! run's model), and a plan's curves are derived, not stored, so for a plan
//! it checks that every clip is made.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use gp_evidence::Paths;
use serde_json::{Value, json};

use crate::clipset as cs;
use crate::clipset::Recipe;
use crate::corpus_score::{
    self as sc, Diff, EVIDENCE_DIR, FILE, Read, ScoredAgainst, Step, Version, or_none, short,
};
use crate::{Outcome, hosted::Source};

/// A corpus named on the command line: a folder, or REV:DIR.
enum Src {
    Dir(PathBuf),
    Git { rev: String, dir: PathBuf },
}

impl Src {
    fn of(spec: &str) -> Src {
        let p = Path::new(spec);
        if p.exists() {
            return Src::Dir(p.to_owned());
        }
        match spec.split_once(':') {
            Some((rev, dir)) if !rev.is_empty() => Src::Git {
                rev: rev.to_owned(),
                dir: PathBuf::from(if dir.is_empty() { "." } else { dir }),
            },
            _ => Src::Dir(p.to_owned()),
        }
    }

    fn name(&self) -> String {
        match self {
            Src::Dir(d) => d.display().to_string(),
            Src::Git { rev, dir } => format!("{rev}:{}", dir.display()),
        }
    }

    /// The corpus read: a folder as it is, a revision's through `git
    /// archive` into a scratch folder (kept until the read is done).
    fn load(&self) -> Result<Read> {
        match self {
            Src::Dir(d) => sc::load(d),
            Src::Git { rev, dir } => {
                let tmp = tempfile::tempdir()?;
                // git archive takes paths from here and writes them from the
                // top of the work tree.
                let top = Command::new("git")
                    .args(["rev-parse", "--show-prefix"])
                    .output()
                    .context("running git")?;
                let prefix = String::from_utf8_lossy(&top.stdout).trim().to_owned();
                let tar = Command::new("git")
                    .args(["archive", "--format=tar", rev, "--"])
                    .arg(dir)
                    .output()
                    .context("running git archive")?;
                if !tar.status.success() {
                    bail!(
                        "git archive {rev} {}: {}",
                        dir.display(),
                        String::from_utf8_lossy(&tar.stderr).trim()
                    );
                }
                let mut child = Command::new("tar")
                    .arg("-x")
                    .arg("-C")
                    .arg(tmp.path())
                    .stdin(std::process::Stdio::piped())
                    .spawn()
                    .context("running tar")?;
                {
                    use std::io::Write as _;
                    child
                        .stdin
                        .take()
                        .context("tar's input")?
                        .write_all(&tar.stdout)?;
                }
                child.wait()?;
                let inner = tmp.path().join(prefix).join(dir);
                sc::load(&inner).with_context(|| self.name())
            }
        }
    }
}

/// What differs between two recipes of one clip.
fn what_changed(a: &Recipe, b: &Recipe) -> Vec<&'static str> {
    let mut w = Vec::new();
    if a.curve != b.curve {
        w.push("curve");
    }
    if a.coverage != b.coverage {
        w.push("coverage");
    }
    if a.presses != b.presses || a.presses_file != b.presses_file {
        w.push("presses");
    }
    if a.start.recording != b.start.recording
        || a.start.presses_from_power_on != b.start.presses_from_power_on
        || a.start.frame != b.start.frame
        || a.start.model_frame != b.start.model_frame
    {
        w.push("start");
    }
    if a.frames != b.frames {
        w.push("frames");
    }
    if w.is_empty() {
        // The same keys with a different hash: a .presses file changed.
        w.push("presses");
    }
    w
}

/// The target and needs of a clip set: its header's, which say what its
/// curves were measured with, and the manifest's where the header is
/// silent.
fn target_of(r: &Read) -> (String, String, String, String) {
    let h = &r.set.clipset;
    let m = r.manifest.as_ref();
    let pick = |a: &str, b: Option<&str>| {
        if a.is_empty() {
            b.unwrap_or_default().to_owned()
        } else {
            a.to_owned()
        }
    };
    (
        pick(&h.rom, m.map(|m| m.target.rom.as_str())),
        m.map(|m| m.target.patch.clone()).unwrap_or_default(),
        pick(&h.mapping, m.map(|m| m.needs.mapping.as_str())),
        pick(&h.generator, m.map(|m| m.needs.generator.as_str())),
    )
}

fn diff_reads(a: &Read, b: &Read) -> Diff {
    let mut d = Diff::default();
    let by_id = |r: &Read| -> std::collections::BTreeMap<String, Recipe> {
        r.set
            .clips
            .iter()
            .map(|c| (c.id.clone(), c.clone()))
            .collect()
    };
    let (ca, cb) = (by_id(a), by_id(b));
    for (id, h) in &b.recipes {
        match a.recipes.get(id) {
            None => d.added.push(id.clone()),
            Some(ha) if ha != h => d.changed.push((id.clone(), what_changed(&ca[id], &cb[id]))),
            _ => {}
        }
    }
    for id in a.recipes.keys() {
        if !b.recipes.contains_key(id) {
            d.removed.push(id.clone());
        }
    }
    let (ta, tb) = (target_of(a), target_of(b));
    for (what, x, y) in [
        ("rom", &ta.0, &tb.0),
        ("patch", &ta.1, &tb.1),
        ("mapping", &ta.2, &tb.2),
    ] {
        if x != y {
            d.target
                .push(format!("{what} {} -> {}", or_none(x), or_none(y)));
        }
    }
    if ta.3 != tb.3 {
        d.metadata.push(format!(
            "generator {} -> {}",
            or_none(&ta.3),
            or_none(&tb.3)
        ));
    }
    if a.metadata != b.metadata
        && d.added.is_empty()
        && d.removed.is_empty()
        && d.changed.is_empty()
    {
        d.metadata
            .push("clips.toml's names, reasons, parts or guards".into());
    }
    if let (Some(ma), Some(mb)) = (&a.manifest, &b.manifest) {
        let (ha, hb) = (&ma.corpus, &mb.corpus);
        if (ha.title.as_str(), &ha.authors, ha.license.as_str())
            != (hb.title.as_str(), &hb.authors, hb.license.as_str())
        {
            d.metadata
                .push("the corpus's title, authors or license".into());
        }
    }
    d
}

pub(crate) fn diff_cmd(a: &str, b: &str) -> Result<Outcome> {
    let (sa, sb) = (Src::of(a), Src::of(b));
    let (ra, rb) = (sa.load()?, sb.load()?);
    let d = diff_reads(&ra, &rb);
    let from = ra.manifest.as_ref().map(|m| m.corpus.version.clone());
    let suggests = from
        .as_deref()
        .and_then(|v| Version::parse(v).ok())
        .map(|v| v.next(d.step()).to_string());
    let mut text = d.lines();
    text.push(format!(
        "{} added, {} removed, {} changed: a {} step{}.",
        d.added.len(),
        d.removed.len(),
        d.changed.len(),
        d.step().name(),
        match (&from, &suggests) {
            (Some(f), Some(s)) if d.step() != Step::None => format!(", {f} -> {s}"),
            _ => String::new(),
        }
    ));
    let mut v = d.json();
    v["a"] = json!({"from": sa.name(), "hash": ra.hash, "version": from});
    v["b"] = json!({"from": sb.name(), "hash": rb.hash,
        "version": rb.manifest.as_ref().map(|m| m.corpus.version.clone())});
    v["suggests"] = json!(suggests);
    Ok(Outcome::ok(v, text.join("\n")))
}

/// The problems with a template's `[scored_against]`: the corpus is
/// there, is the named version and its clips are that version's.
fn check_scored_against(template: &Path, s: &ScoredAgainst) -> Vec<String> {
    let dir = template.join(&s.path);
    let mut out = Vec::new();
    match sc::checked(&dir) {
        Err(e) => out.push(format!("scored_against: {e:#}")),
        Ok((_, m, problems)) => {
            if m.corpus.name != s.corpus || m.corpus.version != s.version || m.corpus.hash != s.hash
            {
                out.push(format!(
                    "scored_against names corpus {} {} ({}), {} is {} {} ({})",
                    s.corpus,
                    s.version,
                    short(&s.hash),
                    dir.join(FILE).display(),
                    m.corpus.name,
                    m.corpus.version,
                    short(&m.corpus.hash)
                ));
            }
            out.extend(problems);
        }
    }
    out
}

/// A template's score records, each with the corpus it is from, sorted by
/// model then corpus.
pub(crate) fn records(template: &Path) -> Vec<Value> {
    let mut out = Vec::new();
    let Ok(models) = fs::read_dir(template.join(EVIDENCE_DIR)) else {
        return out;
    };
    let mut models: Vec<PathBuf> = models.flatten().map(|e| e.path()).collect();
    models.sort();
    for mdir in models.into_iter().filter(|p| p.is_dir()) {
        let Ok(files) = fs::read_dir(&mdir) else {
            continue;
        };
        let mut files: Vec<PathBuf> = files
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        files.sort();
        for f in files {
            let v: Value = fs::read(&f)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or(Value::Null);
            let p = &v["predicate"];
            out.push(json!({
                "file": f,
                "model": p["model"]["hash"],
                "corpus": p["corpus"],
                "results": p["results"],
                "gp": p["gp"],
            }));
        }
    }
    out
}

/// What `show` says of a template folder, or `None` when it isn't one:
/// the corpus it is scored against and its records.
pub(crate) fn template_scores(template: &Path) -> Option<Value> {
    let text = fs::read_to_string(template.join("template.toml")).ok()?;
    let v: toml::Value = toml::from_str(&text).ok()?;
    let sa = v
        .get("scored_against")
        .and_then(|s| s.clone().try_into::<ScoredAgainst>().ok());
    let problems = sa
        .as_ref()
        .map(|s| check_scored_against(template, s))
        .unwrap_or_default();
    Some(json!({"scored_against": sa, "problems": problems, "records": records(template)}))
}

fn record_line(r: &Value) -> String {
    let f = r["results"]["fidelity_permille"].as_u64().unwrap_or(0);
    format!(
        "model {}: corpus {} {} ({}): {} of {} exact, fidelity {}.{}% (gp {})",
        short(r["model"].as_str().unwrap_or("")),
        r["corpus"]["name"].as_str().unwrap_or("?"),
        r["corpus"]["version"].as_str().unwrap_or("?"),
        short(r["corpus"]["hash"].as_str().unwrap_or("")),
        r["results"]["exact"],
        r["results"]["clips"],
        f / 10,
        f % 10,
        r["gp"]["version"].as_str().unwrap_or("?"),
    )
}

pub(crate) fn show(dir: &Path) -> Result<Outcome> {
    let mut text = Vec::new();
    let mut v = json!({});
    if dir.join(FILE).is_file() {
        let (now, m, problems) = sc::checked(dir)?;
        text.push(format!(
            "Corpus {} {}{}: {} clips, hash {}, for ROM {}{}, mapping {}.",
            m.corpus.name,
            m.corpus.version,
            if m.corpus.title.is_empty() {
                String::new()
            } else {
                format!(" ({})", m.corpus.title)
            },
            now.recipes.len(),
            short(&m.corpus.hash),
            short(&m.target.rom),
            if m.target.patch.is_empty() {
                String::new()
            } else {
                format!(" with patch {}", short(&m.target.patch))
            },
            or_none(&m.needs.mapping),
        ));
        for c in &m.changelog {
            text.push(format!(
                "  {} {}: {}{}",
                c.version,
                c.date,
                c.says,
                if c.added.is_empty() && c.removed.is_empty() && c.changed.is_empty() {
                    String::new()
                } else {
                    format!(
                        " ({} added, {} removed, {} changed)",
                        c.added.len(),
                        c.removed.len(),
                        c.changed.len()
                    )
                }
            ));
        }
        for p in &problems {
            text.push(format!("problem: {p}"));
        }
        v["corpus"] = json!({"name": m.corpus.name, "version": m.corpus.version,
            "hash": m.corpus.hash, "clips": now.recipes.len(), "target": m.target,
            "needs": m.needs, "changelog": m.changelog, "current": now.hash == m.corpus.hash,
            "problems": problems});
    }
    if let Some(t) = template_scores(dir) {
        if let Some(sa) = t["scored_against"].as_object() {
            text.push(format!(
                "Scored against corpus {} {} ({}), in folder {}.",
                sa["corpus"].as_str().unwrap_or(""),
                sa["version"].as_str().unwrap_or(""),
                short(sa["hash"].as_str().unwrap_or("")),
                sa["path"].as_str().unwrap_or("."),
            ));
        }
        for p in t["problems"].as_array().into_iter().flatten() {
            text.push(format!("problem: {}", p.as_str().unwrap_or("")));
        }
        for r in t["records"].as_array().into_iter().flatten() {
            text.push(record_line(r));
        }
        v["template"] = t;
    }
    if text.is_empty() {
        bail!("{} has neither a {FILE} nor a template.toml", dir.display());
    }
    Ok(Outcome::ok(v, text.join("\n")))
}

/// Where `verify` finds the model it makes the clips through.
pub(crate) struct VerifyOpen<'a> {
    pub module: Option<&'a Path>,
    pub assets: Option<&'a Path>,
    pub rom: Option<&'a Path>,
}

pub(crate) fn verify(
    paths: &Paths,
    dir: &Path,
    run: &str,
    accept: bool,
    how: &VerifyOpen,
) -> Result<Outcome> {
    let (_, m, mut problems) = sc::checked(dir)?;
    let started = std::time::Instant::now();
    let rom = crate::clip_score::rom_of(paths, dir, how.rom)?;
    let assets = sc::assets_of(paths, dir, dir, how.assets, &rom)?;
    let (source, _) = Source::open_dir(dir, Some(&assets), paths, how.module)?;
    let rdir = sc::run_dir(&paths.home, run)?;
    fs::create_dir_all(&rdir)?;
    let work = cs::Work {
        paths,
        run_dir: &rdir,
        run,
        mdir: dir,
        assets: &assets,
        rom,
        source: Arc::new(source),
    };
    let g = cs::regen_with(&work, dir, None, &cs::Scoring::default())?;
    let make_secs = started.elapsed().as_secs_f64();
    let plan = g.set.plan;
    let accepted = if accept && !plan && g.measured + g.differ > 0 && g.failed == 0 {
        Some(sc::accept_curves(dir, &g)?)
    } else {
        None
    };
    if !m.needs.mapping.is_empty() && g.mapping != m.needs.mapping {
        problems.push(format!(
            "the run's state mapping is schema {}, the corpus's curves were measured with {}",
            g.mapping, m.needs.mapping
        ));
    }
    let same_mapping = g.set.clipset.mapping.is_empty() || g.set.clipset.mapping == g.mapping;
    let mut rows = Vec::new();
    let mut same = 0;
    for r in &g.set.clips {
        let (state, says) = match (g.outcomes.get(&r.id), g.curves.get(&r.id)) {
            (None, _) => ("failed", "not made".to_owned()),
            (Some(Err(e)), _) => ("failed", e.clone()),
            (Some(Ok(_)), None) => ("failed", "no curve".to_owned()),
            (Some(Ok(_)), Some(_)) if plan => ("same", "made".to_owned()),
            (Some(Ok(_)), Some(c)) if !same_mapping || r.curve.is_empty() => {
                ("measured", format!("curve {}", short(c)))
            }
            (Some(Ok(_)), Some(c)) if *c != r.curve => (
                "differ",
                format!("curve {} (recorded {})", short(c), short(&r.curve)),
            ),
            (Some(Ok(_)), Some(_)) => ("same", String::new()),
        };
        same += usize::from(state == "same");
        rows.push(json!({"id": r.id, "state": state, "says": says,
            "curve": g.curves.get(&r.id), "coverage": r.coverage}));
    }
    let mut text: Vec<String> = rows
        .iter()
        .filter(|r| r["state"] != "same")
        .map(|r| {
            format!(
                "{:8}  {}  {}",
                r["state"].as_str().unwrap_or(""),
                r["id"].as_str().unwrap_or(""),
                r["says"].as_str().unwrap_or("")
            )
        })
        .collect();
    if g.differ > 0 || g.failed > 0 || g.measured > 0 {
        problems.push(format!(
            "{} differ, {} measured again, {} couldn't be made",
            g.differ, g.measured, g.failed
        ));
    }
    for p in &problems {
        text.push(format!("problem: {p}"));
    }
    if let Some(n) = accepted {
        text.push(format!(
            "Wrote the curves measured under mapping {} into {}: {n} changed. gp corpus bump suggests the version.",
            g.mapping,
            dir.join(cs::FILE).display()
        ));
    }
    text.push(format!(
        "Corpus {} {} ({}): {} of {} clips made again {}{}.",
        m.corpus.name,
        m.corpus.version,
        short(&m.corpus.hash),
        same,
        g.set.clips.len(),
        if plan {
            "(a plan: its curves are derived, so each is only made)"
        } else {
            "with the same curve"
        },
        if problems.is_empty() {
            "; verified"
        } else {
            ""
        }
    ));
    let v = json!({"ok": problems.is_empty(),
        "corpus": {"name": m.corpus.name, "version": m.corpus.version, "hash": m.corpus.hash},
        "run": run, "same": same, "differ": g.differ, "measured": g.measured,
        "failed": g.failed, "make_seconds": make_secs, "problems": problems,
        "accepted": accepted, "clips": rows});
    let mut out = Outcome::ok(v, text.join("\n"));
    if !problems.is_empty() {
        out.exit = 1;
    }
    Ok(out)
}
