//! `gp corpus score` and `gp game score`: the corpus of a game (its corpus
//! plan, or an older `clips.toml` and `corpus.toml`) scored on a model. A
//! plan's curves are derived from the ROM each time; an older set's are
//! checked against the ones it records.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use clap::Subcommand;
use gp_evidence::Paths;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::Outcome;
use crate::clip_score::{Open, rom_of};
use crate::clipset::{self as cs, ClipSet};
use crate::hosted::Source;

/// A game's manifest, where `[scored_against]` is.
const MANIFEST: &str = "template.toml";
/// Score records inside a template.
const EVIDENCE_DIR: &str = "evidence";
/// The manifest beside a clip set.
pub(crate) const FILE: &str = "corpus.toml";
/// The score record's own format.
pub(crate) const RECORD_FORMAT: &str = "0.1.0";
/// The record's predicate type.
pub(crate) const PREDICATE: &str = "urn:game-printer:score-record:0";

#[derive(Subcommand)]
pub enum CorpusCmd {
    /// Write corpus.toml beside DIR/clips.toml, so an older clip set becomes
    /// a scoring corpus with a name, a version (1.0.0 unless given), the ROM
    /// and mapping from the set and the content hash of its recipes. A game
    /// started by `gp game new` has a corpus plan already.
    Init {
        dir: PathBuf,
        /// The corpus's name: lowercase letters, digits, `-` and `_`.
        #[arg(long)]
        name: String,
        #[arg(long, default_value = "1.0.0")]
        version: String,
        /// A title for people.
        #[arg(long)]
        title: Option<String>,
        /// For a hack: the patch's SHA-256, applied to the base ROM.
        #[arg(long)]
        patch: Option<String>,
        /// The changelog line (default: "First version: N clips.").
        #[arg(long)]
        says: Option<String>,
        /// The changelog date, YYYY-MM-DD (default: today, UTC).
        #[arg(long)]
        date: Option<String>,
    },
    /// The next version from the clips' changes since the version corpus.toml
    /// records. A clip added is minor; a clip removed, a recipe changed, or
    /// another ROM or mapping is major; metadata only is a patch. A plan's
    /// clips are compared with the plan git has at HEAD. `--write` records
    /// it with a changelog entry.
    Bump {
        dir: PathBuf,
        #[arg(long)]
        write: bool,
        /// The changelog line.
        #[arg(long)]
        says: Option<String>,
        /// The changelog date, YYYY-MM-DD (default: today, UTC).
        #[arg(long)]
        date: Option<String>,
    },
    /// Turn GAME's mechanic sessions ($GP_HOME/mechanics/GAME) into the
    /// clips of its corpus plan: every clip of every session, as its presses
    /// from power-on to its start and its own, in inputs/. Sealed sessions
    /// stay out, and so do clips whose inputs are a sealed run's
    /// ($GP_HOME/sealed/GAME). Rows out of scope in the quest log stay out
    /// too, as does what the plan's [import] excludes. Prints what changed
    /// and the version step; `--write` writes the clips and records the
    /// version.
    Import {
        game: String,
        /// The game's folder (default: GAME in the games checkout).
        #[arg(long)]
        template: Option<PathBuf>,
        /// The home holding the sessions, their paths and the sealed runs
        /// (default: $GP_HOME).
        #[arg(long)]
        from: Option<PathBuf>,
        #[arg(long)]
        write: bool,
        /// The changelog line (default: the clips added, changed and
        /// removed).
        #[arg(long)]
        says: Option<String>,
        /// The changelog date, YYYY-MM-DD (default: today, UTC).
        #[arg(long)]
        date: Option<String>,
        /// A clip to remove from the plan: its ID, or a pattern with `*`
        /// (`dk.climb-sweep.*`). Repeat it for each. No other clip is
        /// removed: a clip whose session isn't in the home, or that the
        /// sessions now leave out, is kept as it is.
        #[arg(long, value_name = "ID")]
        remove: Vec<String>,
        /// Import only this session's clips (repeat it for each), so a
        /// worker importing from a shared home adds its own sessions and
        /// nobody else's. The plan's other clips are left as they are.
        #[arg(long, value_name = "NAME")]
        session: Vec<String>,
    },
    /// Score a game's corpus on a model: every clip made again from the
    /// ROM, played through the model and checked against its curve. Prints
    /// the clips exact and the fidelity and, with --json, the score record.
    Score {
        /// The game's package folder (games/donkey-kong), with its corpus
        /// plan (corpus.toml) or, for a game not moved yet, clips.toml too.
        game: PathBuf,
        /// The run's name: its starts and caches are kept under $GP_HOME.
        #[arg(long)]
        run: String,
        /// The game's built module. Needed for a model with hooks.
        #[arg(long)]
        module: Option<PathBuf>,
        /// The model to score; by default the game's own.
        #[arg(long)]
        model: Option<PathBuf>,
        /// The model's lifted assets (default: $GP_HOME/work/assets/SOURCE,
        /// lifted there first if missing).
        #[arg(long)]
        assets: Option<PathBuf>,
        /// The ROM; by default the one in $GP_ROM_DIR the package names.
        #[arg(long)]
        rom: Option<PathBuf>,
        /// Score only these clips, by ID. Repeatable.
        #[arg(long)]
        only: Vec<String>,
        /// Against an earlier `gp --json corpus score` output in this file:
        /// score only the clips the change can move, and keep the others' rows.
        #[arg(long, conflicts_with = "only")]
        reuse: Option<PathBuf>,
        /// Write the score record into this template folder's evidence/, and
        /// point its template.toml's [scored_against] at this corpus.
        #[arg(long)]
        write: Option<PathBuf>,
        /// Measure the original's curves again under the model's mapping
        /// (the model in this folder): they become the corpus's, as its next
        /// version, and the record is written for it.
        #[arg(long, conflicts_with = "only")]
        remeasure: Option<PathBuf>,
    },
}

pub(crate) fn run(cmd: &CorpusCmd, paths: &Paths) -> Result<Outcome> {
    match cmd {
        CorpusCmd::Init {
            dir,
            name,
            version,
            title,
            patch,
            says,
            date,
        } => init(
            dir,
            name,
            version,
            title.as_deref(),
            patch.as_deref(),
            says.as_deref(),
            date.as_deref(),
        ),
        CorpusCmd::Bump {
            dir,
            write,
            says,
            date,
        } => bump(dir, *write, says.as_deref(), date.as_deref()),
        CorpusCmd::Import {
            game,
            template,
            from,
            write,
            says,
            date,
            remove,
            session,
        } => crate::corpus_import::run(
            &crate::corpus_import::Args {
                game,
                template: template.as_deref(),
                from: from.as_deref(),
                write: *write,
                says: says.as_deref(),
                date: date.as_deref(),
                remove,
                sessions: session,
            },
            paths,
        ),
        CorpusCmd::Score {
            game,
            run,
            module,
            model,
            assets,
            rom,
            only,
            reuse,
            write,
            remeasure,
        } => score(
            paths,
            game,
            run,
            &Open {
                module: module.as_deref(),
                model: model.as_deref(),
                assets: assets.as_deref(),
                rom: rom.as_deref(),
            },
            only,
            reuse.as_deref(),
            write.as_deref(),
            remeasure.as_deref(),
        ),
    }
}

/// A run's folder under `$GP_HOME/capture`, its name checked.
pub(crate) fn run_dir(home: &Path, run: &str) -> Result<PathBuf> {
    if run.is_empty()
        || run.len() > 80
        || run == "cache"
        || !run
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!("a run name is letters, digits, - and _, and not `cache` (got {run:?})");
    }
    Ok(home.join("capture").join(run))
}

const HEAD_COMMENT: &str = "# A scoring corpus (docs/formats/scoring-corpus.md): the clip set beside it\n\
    # (clips.toml) with a name, a version and the hash of its recipes. Recipes,\n\
    # hashes and metadata only: `gp corpus verify` makes every clip again from\n\
    # your own ROM. `gp corpus bump` suggests the next version.\n";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CorpusFile {
    pub corpus: Head,
    pub target: Target,
    #[serde(default)]
    pub needs: Needs,
    /// Each clip's recipe hash at this version, by ID: what `bump`
    /// compares the clip set with.
    #[serde(default)]
    pub recipes: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changelog: Vec<Change>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Head {
    /// The clip set format this manifest is part of, [`cs::FORMAT`].
    pub format: String,
    pub name: String,
    /// Semver of the corpus's content.
    pub version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// The content hash over the clips' recipes ([`cs::content_hash`]).
    pub hash: String,
    /// The BLAKE3 of `clips.toml` itself: a change here with the same
    /// `hash` is metadata only.
    #[serde(default)]
    pub metadata: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub license: String,
}

/// The game a corpus is for.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Target {
    /// The template or source ID.
    #[serde(default)]
    pub game: String,
    /// The base ROM's headerless SHA-256, as `sources/<platform>.toml`
    /// registers it.
    pub rom: String,
    /// For a hack: the patch's SHA-256.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub patch: String,
}

/// What a corpus needs of the tools that make its clips again.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Needs {
    /// gp's version and the corpus generator's, as the clip set says.
    #[serde(default)]
    pub generator: String,
    /// The state mapping's `schema` the curves were measured with.
    #[serde(default)]
    pub mapping: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Change {
    pub version: String,
    pub date: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub says: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed: Vec<String>,
}

pub(crate) fn read_manifest(text: &str, what: &str) -> Result<CorpusFile> {
    let c: CorpusFile = toml::from_str(text).with_context(|| format!("{what} isn't a corpus"))?;
    if !cs::readable(&c.corpus.format) {
        bail!(
            "{what} is clip set format {}, this gp reads {}",
            c.corpus.format,
            cs::FORMAT
        );
    }
    if !valid_name(&c.corpus.name) {
        bail!(
            "{what}: corpus.name {:?} isn't lowercase letters, digits, - and _",
            c.corpus.name
        );
    }
    Version::parse(&c.corpus.version)
        .with_context(|| format!("{what}: corpus.version {:?}", c.corpus.version))?;
    Ok(c)
}

fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Version(u64, u64, u64);

impl Version {
    pub(crate) fn parse(s: &str) -> Result<Version> {
        let p: Vec<&str> = s.split('.').collect();
        let n = |x: &str| x.parse::<u64>().ok();
        match p.as_slice() {
            [a, b, c] => match (n(a), n(b), n(c)) {
                (Some(a), Some(b), Some(c)) => Ok(Version(a, b, c)),
                _ => bail!("{s} isn't MAJOR.MINOR.PATCH"),
            },
            _ => bail!("{s} isn't MAJOR.MINOR.PATCH"),
        }
    }

    pub(crate) fn next(self, step: Step) -> Version {
        let Version(a, b, c) = self;
        match step {
            Step::None => self,
            Step::Patch => Version(a, b, c + 1),
            Step::Minor => Version(a, b + 1, 0),
            Step::Major => Version(a + 1, 0, 0),
        }
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// The version step a change makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Step {
    None,
    Patch,
    Minor,
    Major,
}

impl Step {
    fn name(self) -> &'static str {
        match self {
            Step::None => "none",
            Step::Patch => "patch",
            Step::Minor => "minor",
            Step::Major => "major",
        }
    }
}

/// A corpus as read from its folder.
struct Read {
    set: ClipSet,
    recipes: BTreeMap<String, String>,
    hash: String,
    metadata: String,
    manifest: Option<CorpusFile>,
}

fn load(dir: &Path) -> Result<Read> {
    if let Some(p) = crate::plan::load(dir)? {
        return Ok(Read {
            metadata: p.manifest.corpus.metadata.clone(),
            set: p.set,
            recipes: p.recipes,
            hash: p.hash,
            manifest: Some(p.manifest),
        });
    }
    let file = dir.join(cs::FILE);
    let text = fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
    let set = cs::parse(&text, &file)?;
    let reader = |p: &str| -> Result<String> {
        fs::read_to_string(dir.join(p)).with_context(|| format!("reading {p}"))
    };
    let mut recipes = BTreeMap::new();
    for r in &set.clips {
        let h = cs::recipe_hash(r, &reader).with_context(|| format!("clip {}'s recipe", r.id))?;
        recipes.insert(r.id.clone(), h);
    }
    let manifest = match fs::read_to_string(dir.join(FILE)) {
        Ok(t) => Some(read_manifest(&t, &format!("{}/{FILE}", dir.display()))?),
        Err(_) => None,
    };
    Ok(Read {
        hash: cs::content_hash(&recipes),
        metadata: gp_evidence::Hash::of(text.as_bytes()).to_string(),
        set,
        recipes,
        manifest,
    })
}

#[derive(Default)]
pub(crate) struct Diff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    /// Each changed clip with what changed in it, when both recipes are
    /// at hand (`curve`, `coverage`, `presses`, `start`, `frames`), else `recipe`.
    pub changed: Vec<(String, Vec<&'static str>)>,
    /// The ROM, patch or mapping differ: the clips are for another game,
    /// or their curves were measured another way.
    pub target: Vec<String>,
    /// Anything else differs: names, reasons, parts, guards, the
    /// generator, the manifest's title.
    pub metadata: Vec<String>,
}

impl Diff {
    pub(crate) fn step(&self) -> Step {
        if !self.removed.is_empty() || !self.changed.is_empty() || !self.target.is_empty() {
            Step::Major
        } else if !self.added.is_empty() {
            Step::Minor
        } else if !self.metadata.is_empty() {
            Step::Patch
        } else {
            Step::None
        }
    }

    /// The step when a changed clip's curve or coverage hash is all that
    /// changed because the original was measured again under a mapping
    /// that measures more: minor, as a clip added is. Against a recorded
    /// version only the recipe hash is at hand (`recipe`); `score
    /// --remeasure` calls this only when the clips were that version's
    /// before it wrote the curves, so the curves are all that moved.
    pub(crate) fn step_measured_again(&self) -> Step {
        let measured = self.changed.iter().all(|(_, w)| {
            w.iter()
                .all(|x| matches!(*x, "curve" | "coverage" | "recipe"))
        });
        if self.removed.is_empty() && self.target.is_empty() && measured && !self.changed.is_empty()
        {
            Step::Minor
        } else {
            self.step()
        }
    }

    fn json(&self) -> Value {
        json!({
            "added": self.added,
            "removed": self.removed,
            "changed": self.changed.iter().map(|(id, w)| json!({"id": id, "what": w})).collect::<Vec<_>>(),
            "target": self.target,
            "metadata": self.metadata,
            "step": self.step().name(),
        })
    }

    fn lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        for id in &self.added {
            out.push(format!("added    {id}"));
        }
        for id in &self.removed {
            out.push(format!("removed  {id}"));
        }
        for (id, w) in &self.changed {
            out.push(format!("changed  {id}  ({})", w.join(", ")));
        }
        for t in &self.target {
            out.push(format!("target   {t}"));
        }
        for m in &self.metadata {
            out.push(format!("metadata {m}"));
        }
        out
    }
}

fn diff_recorded(m: &CorpusFile, now: &Read) -> Diff {
    let mut d = Diff::default();
    for (id, h) in &now.recipes {
        match m.recipes.get(id) {
            None => d.added.push(id.clone()),
            Some(x) if x != h => d.changed.push((id.clone(), vec!["recipe"])),
            _ => {}
        }
    }
    for id in m.recipes.keys() {
        if !now.recipes.contains_key(id) {
            d.removed.push(id.clone());
        }
    }
    let h = &now.set.clipset;
    if !h.rom.is_empty() && h.rom != m.target.rom {
        d.target
            .push(format!("rom {} -> {}", or_none(&m.target.rom), h.rom));
    }
    if !h.mapping.is_empty() && h.mapping != m.needs.mapping {
        d.target.push(format!(
            "mapping {} -> {}",
            or_none(&m.needs.mapping),
            h.mapping
        ));
    }
    if !h.generator.is_empty() && h.generator != m.needs.generator {
        d.metadata.push(format!(
            "generator {} -> {}",
            or_none(&m.needs.generator),
            h.generator
        ));
    }
    if now.metadata != m.corpus.metadata
        && d.added.is_empty()
        && d.removed.is_empty()
        && d.changed.is_empty()
    {
        d.metadata
            .push("clips.toml's names, reasons, parts or guards".into());
    }
    d
}

fn or_none(s: &str) -> &str {
    if s.is_empty() { "none" } else { s }
}

fn short(h: &str) -> &str {
    &h[..h.len().min(12)]
}

fn checked(dir: &Path) -> Result<(Read, CorpusFile, Vec<String>)> {
    let now = load(dir)?;
    let Some(m) = now.manifest.clone() else {
        bail!("{} has no {FILE}; gp corpus init makes one", dir.display());
    };
    let mut problems = Vec::new();
    if now.set.plan {
        if now.hash != m.corpus.hash {
            problems.push(format!(
                "{}'s recipes aren't version {}'s (hash {}, recorded {}): a changed recipe is a new corpus version",
                dir.display(),
                m.corpus.version,
                now.hash,
                m.corpus.hash,
            ));
        }
    } else if now.hash != m.corpus.hash {
        let d = diff_recorded(&m, &now);
        problems.push(format!(
            "{}'s clips aren't version {}'s (hash {}, recorded {}); gp corpus bump suggests {}",
            dir.display(),
            m.corpus.version,
            short(&now.hash),
            short(&m.corpus.hash),
            Version::parse(&m.corpus.version)?.next(d.step())
        ));
    }
    if !now.set.clipset.rom.is_empty() && now.set.clipset.rom != m.target.rom {
        problems.push(format!(
            "clips.toml is for ROM {}, corpus.toml for {}",
            now.set.clipset.rom, m.target.rom
        ));
    }
    Ok((now, m, problems))
}

fn write_manifest(dir: &Path, c: &CorpusFile) -> Result<PathBuf> {
    let file = dir.join(FILE);
    fs::write(&file, format!("{HEAD_COMMENT}{}", toml::to_string(c)?))
        .with_context(|| format!("writing {}", file.display()))?;
    Ok(file)
}

fn today(date: Option<&str>) -> Result<String> {
    if let Some(d) = date {
        let ok = d.len() == 10
            && d.chars().enumerate().all(|(i, c)| {
                if i == 4 || i == 7 {
                    c == '-'
                } else {
                    c.is_ascii_digit()
                }
            });
        if !ok {
            bail!("--date {d} isn't YYYY-MM-DD");
        }
        return Ok(d.to_owned());
    }
    // The civil date from days since 1970 (Howard Hinnant's algorithm).
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let z = (secs / 86400) as i64 + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    Ok(format!("{y:04}-{m:02}-{d:02}"))
}

// ---- Commands ----

/// `gp corpus init`: a manifest beside an older `clips.toml`.
pub(crate) fn init(
    dir: &Path,
    name: &str,
    version: &str,
    title: Option<&str>,
    patch: Option<&str>,
    says: Option<&str>,
    date: Option<&str>,
) -> Result<Outcome> {
    if !valid_name(name) {
        bail!("--name {name:?} isn't lowercase letters, digits, - and _");
    }
    let v = Version::parse(version)?;
    if dir.join(FILE).is_file() {
        bail!(
            "{} is there already; gp corpus bump records a new version",
            dir.join(FILE).display()
        );
    }
    let now = load(dir)?;
    let h = &now.set.clipset;
    if h.rom.is_empty() {
        bail!(
            "{}'s [clipset] names no rom; a corpus is for one ROM",
            cs::FILE
        );
    }
    let c = CorpusFile {
        corpus: Head {
            format: cs::FORMAT.into(),
            name: name.into(),
            version: v.to_string(),
            title: title.unwrap_or_default().into(),
            hash: now.hash.clone(),
            metadata: now.metadata.clone(),
            ..Head::default()
        },
        target: Target {
            game: h.game.clone(),
            rom: h.rom.clone(),
            patch: patch.unwrap_or_default().into(),
        },
        needs: Needs {
            generator: h.generator.clone(),
            mapping: h.mapping.clone(),
        },
        recipes: now.recipes.clone(),
        changelog: vec![Change {
            version: v.to_string(),
            date: today(date)?,
            says: says.map_or_else(
                || format!("First version: {} clips.", now.recipes.len()),
                str::to_owned,
            ),
            ..Change::default()
        }],
    };
    let file = write_manifest(dir, &c)?;
    Ok(Outcome::ok(
        json!({"file": file, "name": name, "version": c.corpus.version, "hash": c.corpus.hash,
            "clips": now.recipes.len()}),
        format!(
            "Corpus {name} {} with {} clips, hash {}: {}.",
            c.corpus.version,
            now.recipes.len(),
            short(&c.corpus.hash),
            file.display()
        ),
    ))
}

/// `gp corpus bump`: a plan's by [`plan_bump`], an older clip set's against
/// the recipes its manifest records.
pub(crate) fn bump(
    dir: &Path,
    write: bool,
    says: Option<&str>,
    date: Option<&str>,
) -> Result<Outcome> {
    if crate::plan::load(dir)?.is_some() {
        return plan_bump(dir, write, says, date, None);
    }
    bump_as(dir, write, says, date, false)
}

/// A plan as git has it at HEAD, by each file's path in the game's folder.
fn plan_at_head(dir: &Path) -> Option<crate::plan::Loaded> {
    let read = |rel: &str| -> Result<String> {
        let o = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["show", &format!("HEAD:./{rel}")])
            .output()
            .context("running git")?;
        if !o.status.success() {
            bail!(
                "git show HEAD:{rel}: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            );
        }
        Ok(String::from_utf8(o.stdout)?)
    };
    crate::plan::load_with(&read, &dir.join(FILE))
        .ok()
        .flatten()
}

/// The version a plan's recipes make since the version it records, and with
/// `write`, that version and its recipe-set hash written in place with a
/// changelog entry. A plan records only the recipe-set hash, so each clip's
/// recorded recipe is `known` (the import's, when the plan was as recorded
/// before it), else the plan git has at HEAD when that is the recorded
/// version. Without either, a changed hash is a major step.
pub(crate) fn plan_bump(
    dir: &Path,
    write: bool,
    says: Option<&str>,
    date: Option<&str>,
    known: Option<&BTreeMap<String, String>>,
) -> Result<Outcome> {
    let Some(now) = crate::plan::load(dir)? else {
        bail!("{} has no corpus plan", dir.display());
    };
    let m = &now.manifest;
    let recorded = &m.corpus.hash;
    let head = if known.is_none() && now.hash != *recorded {
        plan_at_head(dir).filter(|h| h.hash == *recorded)
    } else {
        None
    };
    let mut d = Diff::default();
    if now.hash != *recorded {
        match known.or(head.as_ref().map(|h| &h.recipes)) {
            Some(was) => {
                for (id, h) in &now.recipes {
                    match was.get(id) {
                        None => d.added.push(id.clone()),
                        Some(x) if x != h => d.changed.push((id.clone(), vec!["recipe"])),
                        _ => {}
                    }
                }
                for id in was.keys() {
                    if !now.recipes.contains_key(id) {
                        d.removed.push(id.clone());
                    }
                }
            }
            None => d.target.push(format!(
                "recipes {} -> {}: no recorded version to compare clip by clip (the plan at git HEAD isn't it)",
                short(recorded),
                short(&now.hash)
            )),
        }
    }
    if let Some(h) = &head {
        let (a, b) = (&h.manifest, m);
        if a.target.rom != b.target.rom {
            d.target.push(format!(
                "rom {} -> {}",
                or_none(&a.target.rom),
                b.target.rom
            ));
        }
        if a.needs.mapping != b.needs.mapping {
            d.target.push(format!(
                "mapping {} -> {}",
                or_none(&a.needs.mapping),
                b.needs.mapping
            ));
        }
    }
    let from = Version::parse(&m.corpus.version)?;
    let step = d.step();
    let to = from.next(step);
    let mut text = d.lines();
    let mut v = d.json();
    v["step"] = json!(step.name());
    v["name"] = json!(m.corpus.name);
    v["version"] = json!(m.corpus.version);
    v["suggests"] = json!(to.to_string());
    if step == Step::None {
        text.push(format!(
            "Corpus {} {} is as recorded: nothing to bump.",
            m.corpus.name, m.corpus.version
        ));
        v["written"] = json!(false);
        return Ok(Outcome::ok(v, text.join("\n")));
    }
    text.push(format!(
        "{} added, {} removed, {} changed since {}: a {} step, {from} -> {to}.",
        d.added.len(),
        d.removed.len(),
        d.changed.len(),
        m.corpus.version,
        step.name()
    ));
    if write {
        let file = dir.join(FILE);
        let before = fs::read_to_string(&file)?;
        let change = Change {
            version: to.to_string(),
            date: today(date)?,
            says: says.unwrap_or_default().into(),
            added: d.added.clone(),
            removed: d.removed.clone(),
            changed: d.changed.iter().map(|(id, _)| id.clone()).collect(),
        };
        fs::write(
            &file,
            plan_versioned(&before, &to.to_string(), &now.hash, &change)?,
        )
        .with_context(|| format!("writing {}", file.display()))?;
        text.push(format!("Recorded {to} in {}.", file.display()));
    }
    v["written"] = json!(write);
    Ok(Outcome::ok(v, text.join("\n")))
}

/// A plan's text with its `[corpus]` version and recipe-set hash set and a
/// changelog entry added at its end; every other byte kept.
fn plan_versioned(text: &str, version: &str, recipes: &str, change: &Change) -> Result<String> {
    #[derive(Serialize)]
    struct Log<'a> {
        changelog: [&'a Change; 1],
    }
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let tabs = crate::plan::tables(&lines);
    let Some((_, from, to)) = tabs.iter().find(|(n, _, _)| n == "corpus") else {
        bail!("the plan has no [corpus] table");
    };
    let mut out = String::new();
    let (mut set_version, mut set_recipes) = (false, false);
    for (i, l) in lines.iter().enumerate() {
        let key = l.split('=').next().unwrap_or("").trim();
        if i > *from && i < *to && key == "version" {
            out.push_str(&format!("version = {}\n", toml::Value::from(version)));
            set_version = true;
        } else if i > *from && i < *to && key == "recipes" {
            out.push_str(&format!("recipes = {}\n", toml::Value::from(recipes)));
            set_recipes = true;
        } else {
            out.push_str(l);
        }
    }
    if !(set_version && set_recipes) {
        bail!("the plan's [corpus] has no version or recipes line");
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push('\n');
    out.push_str(
        toml::to_string(&Log {
            changelog: [change],
        })?
        .trim_end(),
    );
    out.push('\n');
    Ok(out)
}

fn bump_as(
    dir: &Path,
    write: bool,
    says: Option<&str>,
    date: Option<&str>,
    measured_again: bool,
) -> Result<Outcome> {
    let now = load(dir)?;
    let Some(mut m) = now.manifest.clone() else {
        bail!("{} has no {FILE}; gp corpus init makes one", dir.display());
    };
    let d = diff_recorded(&m, &now);
    let from = Version::parse(&m.corpus.version)?;
    let step = if measured_again {
        d.step_measured_again()
    } else {
        d.step()
    };
    let to = from.next(step);
    let mut text = d.lines();
    let mut v = d.json();
    v["step"] = json!(step.name());
    v["name"] = json!(m.corpus.name);
    v["version"] = json!(m.corpus.version);
    v["suggests"] = json!(to.to_string());
    if step == Step::None {
        text.push(format!(
            "Corpus {} {} is as recorded: nothing to bump.",
            m.corpus.name, m.corpus.version
        ));
        v["written"] = json!(false);
        return Ok(Outcome::ok(v, text.join("\n")));
    }
    text.push(format!(
        "{} added, {} removed, {} changed since {}: a {} step, {from} -> {to}.",
        d.added.len(),
        d.removed.len(),
        d.changed.len(),
        m.corpus.version,
        step.name()
    ));
    if write {
        let h = &now.set.clipset;
        m.corpus.version = to.to_string();
        m.corpus.format = cs::FORMAT.into();
        m.corpus.hash = now.hash.clone();
        m.corpus.metadata = now.metadata.clone();
        if !h.rom.is_empty() {
            m.target.rom = h.rom.clone();
        }
        if !h.mapping.is_empty() {
            m.needs.mapping = h.mapping.clone();
        }
        if !h.generator.is_empty() {
            m.needs.generator = h.generator.clone();
        }
        m.recipes = now.recipes.clone();
        m.changelog.push(Change {
            version: to.to_string(),
            date: today(date)?,
            says: says.unwrap_or_default().into(),
            added: d.added.clone(),
            removed: d.removed.clone(),
            changed: d.changed.iter().map(|(id, _)| id.clone()).collect(),
        });
        let file = write_manifest(dir, &m)?;
        text.push(format!("Recorded {to} in {}.", file.display()));
    }
    v["written"] = json!(write);
    Ok(Outcome::ok(v, text.join("\n")))
}

fn accept_curves(dir: &Path, g: &cs::Regen) -> Result<usize> {
    let file = dir.join(cs::FILE);
    let text = fs::read_to_string(&file)?;
    let head: String = text
        .lines()
        .take_while(|l| l.starts_with('#'))
        .map(|l| format!("{l}\n"))
        .collect();
    let mut set = g.set.clone();
    let mut changed = 0;
    for c in &mut set.clips {
        if let Some(m) = g.curves.get(&c.id)
            && c.curve != *m
        {
            c.curve.clone_from(m);
            changed += 1;
        }
    }
    set.clipset.mapping = g.mapping.clone();
    set.clipset.format = cs::FORMAT.into();
    fs::write(&file, format!("{head}{}", toml::to_string(&set)?))?;
    Ok(changed)
}

pub(crate) fn record_path(template: &Path, model_hash: &str, corpus_hash: &str) -> PathBuf {
    template
        .join(EVIDENCE_DIR)
        .join(model_hash)
        .join(format!("{corpus_hash}.json"))
}

/// The version template `t`'s `[scored_against]` names when it names
/// corpus folder `dir` at another version or hash than `c`, else None (no
/// section, another corpus, or current).
fn behind(t: &Path, dir: &Path, c: &Head) -> Option<String> {
    let text = fs::read_to_string(t.join(MANIFEST)).ok()?;
    let v: toml::Value = toml::from_str(&text).ok()?;
    let sa: ScoredAgainst = v.get("scored_against")?.clone().try_into().ok()?;
    let same_dir = t.join(&sa.path).canonicalize().ok()? == dir.canonicalize().ok()?;
    (same_dir && sa.corpus == c.name && (sa.version != c.version || sa.hash != c.hash))
        .then_some(sa.version)
}

/// `to` relative to `from`, both folders: `.` when the same.
fn relative(from: &Path, to: &Path) -> String {
    let (Ok(a), Ok(b)) = (from.canonicalize(), to.canonicalize()) else {
        return to.display().to_string();
    };
    let (ac, bc): (Vec<_>, Vec<_>) = (a.components().collect(), b.components().collect());
    let common = ac.iter().zip(&bc).take_while(|(x, y)| x == y).count();
    let mut p = PathBuf::new();
    for _ in common..ac.len() {
        p.push("..");
    }
    for c in &bc[common..] {
        p.push(c);
    }
    if p.as_os_str().is_empty() {
        ".".into()
    } else {
        p.display().to_string()
    }
}

/// Sets `[scored_against]` in a template manifest: an existing section's
/// keys are rewritten where they stand, so its comments and the sections
/// around it stay as they are; a new section goes at the end.
fn set_scored_against(manifest: &Path, c: &Head, path: &str) -> Result<()> {
    let text = fs::read_to_string(manifest)?;
    let values = [
        ("corpus", c.name.as_str()),
        ("version", c.version.as_str()),
        ("hash", c.hash.as_str()),
        ("path", path),
    ];
    let line_of = |k: &str, v: &str| format!("{k} = {v:?}");
    let mut out = String::new();
    if text.lines().any(|l| l.trim() == "[scored_against]") {
        let has = |k: &str| {
            let mut inside = false;
            text.lines().any(|l| {
                let t = l.trim();
                if t.starts_with('[') {
                    inside = t == "[scored_against]";
                }
                inside && t.split('=').next().unwrap_or("").trim() == k
            })
        };
        let mut inside = false;
        for line in text.lines() {
            let t = line.trim();
            if t.starts_with('[') {
                inside = t == "[scored_against]";
            }
            let key = t.split('=').next().unwrap_or("").trim();
            match values.iter().find(|(k, _)| inside && *k == key) {
                Some((k, v)) => out.push_str(&line_of(k, v)),
                None => out.push_str(line),
            }
            out.push('\n');
            // Keys the section lacks go right after its header.
            if t == "[scored_against]" {
                for (k, v) in values.iter().filter(|(k, _)| !has(k)) {
                    out.push_str(&line_of(k, v));
                    out.push('\n');
                }
            }
        }
    } else {
        out.push_str(text.trim_end());
        out.push_str(
            "\n\n# The scoring corpus this game's scores are against\n\
             # (docs/formats/scoring-corpus.md); the records are in evidence/.\n[scored_against]\n",
        );
        for (k, v) in values {
            out.push_str(&line_of(k, v));
            out.push('\n');
        }
    }
    fs::write(manifest, out)?;
    Ok(())
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScoredAgainst {
    pub corpus: String,
    pub version: String,
    pub hash: String,
    /// The corpus's folder, relative to the template (default `.`).
    #[serde(default = "here")]
    pub path: String,
}

fn here() -> String {
    ".".into()
}

/// The gp build: the hash of this binary, or `GP_BUILD` when set.
pub(crate) fn build() -> String {
    if let Ok(b) = std::env::var("GP_BUILD") {
        return b;
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| fs::read(p).ok())
        .map(|b| gp_evidence::Hash::of(&b).to_string())
        .unwrap_or_else(|| format!("gp {}", env!("CARGO_PKG_VERSION")))
}

/// The hash of a model's text as the record names it: its canonical tree's,
/// else the text's, so a reformatted model gets the same record.
pub(crate) fn model_key(text: &str) -> String {
    gp_model::Model::parse(text)
        .ok()
        .and_then(|m| toml::to_string(&m.to_tree()).ok())
        .map_or_else(
            || gp_evidence::Hash::of(text.as_bytes()).to_string(),
            |t| gp_evidence::Hash::of(t.as_bytes()).to_string(),
        )
}

/// `gp game score`: the corpus derived and scored in this `$GP_HOME`, from
/// the package in `game` and the ROM alone. The score record goes to `out`
/// with the corpus's curve-set hash and the module's hash beside it, for an
/// attestation to name. Without `--module` a package's own
/// `play/module.wasm` is used.
pub(crate) fn game_score(
    paths: &Paths,
    game: &Path,
    module: Option<&Path>,
    out: &Path,
) -> Result<Outcome> {
    let game = fs::canonicalize(game).with_context(|| game.display().to_string())?;
    let own = game.join("play/module.wasm");
    let module = module
        .map(Path::to_owned)
        .or_else(|| own.is_file().then_some(own));
    let scored = score(
        paths,
        &game,
        "game-score",
        &Open {
            module: module.as_deref(),
            model: None,
            assets: None,
            rom: None,
        },
        &[],
        None,
        None,
        None,
    )?;
    let mut v = scored.json;
    let rows = v["record"]["predicate"]["clips"]
        .as_array()
        .context("the score record has no clips")?;
    let curves: BTreeMap<String, (String, String)> = rows
        .iter()
        .map(|r| {
            let f = |k: &str| r[k].as_str().unwrap_or("").to_owned();
            (f("id"), (f("curve"), f("coverage")))
        })
        .collect();
    v["curves"] = json!(crate::plan::curve_set_hash(&curves));
    v["module"] = match &module {
        Some(m) => json!(gp_evidence::Hash::of(&fs::read(m)?).to_string()),
        None => Value::Null,
    };
    if let Some(p) = out.parent() {
        fs::create_dir_all(p)?;
    }
    fs::write(out, format!("{}\n", serde_json::to_string_pretty(&v)?))
        .with_context(|| out.display().to_string())?;
    let text = format!("{}\nThe score record is in {}.", scored.text, out.display());
    let exit = scored.exit;
    Ok(Outcome {
        json: v,
        text,
        exit,
    })
}

/// The game's asset map for this ROM: its path and text.
pub(crate) fn asset_map(game: &Path, rom: &[u8]) -> Result<(PathBuf, String)> {
    let pkg: toml::Table = fs::read_to_string(game.join("package.toml"))
        .with_context(|| format!("reading {}/package.toml", game.display()))?
        .parse()
        .with_context(|| game.display().to_string())?;
    let entries = pkg
        .get("rom")
        .and_then(|r| r.get("entry"))
        .and_then(|e| e.as_array())
        .cloned()
        .unwrap_or_default();
    let sha = crate::roms::sha256_of(rom)?;
    let entry = entries
        .iter()
        .find(|e| e.get("sha256").and_then(|v| v.as_str()) == Some(sha.as_str()))
        .context("the package names no asset map for this ROM")?;
    let map_rel = entry
        .get("asset_map")
        .and_then(|v| v.as_str())
        .context("the package's ROM entry has no asset_map")?;
    let path = game.join(map_rel);
    let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    Ok((path, text))
}

/// Lifts the game's assets for `rom` into `out`, unless already there.
pub(crate) fn lift_into(game: &Path, rom: &[u8], out: &Path) -> Result<()> {
    if out.join(gp_assets::MAP_FILE).is_file() {
        return Ok(());
    }
    let (path, text) = asset_map(game, rom)?;
    let (files, _) =
        crate::assets::lift_files(rom, &text).with_context(|| path.display().to_string())?;
    for f in &files {
        let p = out.join(&f.name);
        if let Some(d) = p.parent() {
            fs::create_dir_all(d)?;
        }
        fs::write(&p, &f.bytes)?;
    }
    Ok(())
}

/// The model's lifted assets: `--assets`, else `$GP_HOME/work/assets/SOURCE`,
/// lifted there from the game's asset map and the ROM if not there yet.
pub(crate) fn assets_of(
    paths: &Paths,
    game: &Path,
    mdir: &Path,
    given: Option<&Path>,
    rom: &[u8],
) -> Result<PathBuf> {
    if let Some(a) = given {
        return Ok(a.to_owned());
    }
    let file = mdir.join(gp_model::MODEL_FILE);
    let text = fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
    let model = gp_model::Model::parse(&text).with_context(|| file.display().to_string())?;
    let Some(source) = &model.model.source else {
        bail!("the model names no source, so give its lifted assets with --assets");
    };
    let out = paths.home.join("work").join("assets").join(source);
    lift_into(game, rom, &out)?;
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn score(
    paths: &Paths,
    game: &Path,
    run: &str,
    how: &Open,
    only: &[String],
    reuse: Option<&Path>,
    write: Option<&Path>,
    remeasure: Option<&Path>,
) -> Result<Outcome> {
    let (mut now, mut m, mut problems) = checked(game)?;
    let only: Option<BTreeSet<String>> = (!only.is_empty()).then(|| only.iter().cloned().collect());
    let started = std::time::Instant::now();
    let rom = rom_of(paths, game, how.rom)?;
    let mdir = how.model.or(write).or(remeasure).unwrap_or(game).to_owned();
    let assets = assets_of(paths, game, &mdir, how.assets, &rom)?;
    let (source, _) = Source::open_dir(&mdir, Some(&assets), paths, how.module)?;
    let source = Arc::new(source);
    let dir = run_dir(&paths.home, run)?;
    fs::create_dir_all(&dir)?;
    let work = cs::Work {
        paths,
        run_dir: &dir,
        run,
        mdir: &mdir,
        assets: &assets,
        rom,
        source: source.clone(),
    };
    // Against an earlier score: only the clips the change can move.
    let earlier = match reuse {
        Some(f) => {
            let before: Value = serde_json::from_slice(
                &fs::read(f).with_context(|| format!("reading {}", f.display()))?,
            )
            .with_context(|| format!("{} isn't a gp corpus score --json output", f.display()))?;
            let b = cs::bench_for(&work)?;
            let (mm, _) = b.model()?;
            let fp = crate::affected::fingerprint(&b, &mm, game)?;
            let ids: Vec<String> = now.set.clips.iter().map(|r| r.id.clone()).collect();
            let sel = crate::affected::select(&before, &fp, &ids, &now.recipes);
            Some((fp, sel))
        }
        None => None,
    };
    let scoring = cs::Scoring {
        reads: true,
        stop: earlier
            .as_ref()
            .map(|(_, s)| s.stop.clone())
            .unwrap_or_default(),
    };
    let only = match &earlier {
        Some((_, sel)) => Some(sel.score.clone()),
        None => only,
    };
    let mut g = cs::regen_with(&work, game, only.as_ref(), &scoring)?;
    if g.stopped > 0 {
        problems.push(format!(
            "{} clips the earlier score had exact stopped at their first split",
            g.stopped
        ));
    }
    // Measured again: the original's curves under the model's mapping
    // become the corpus's, as the next minor version, and the record is
    // written for it.
    let mut write = write;
    let mut remeasured = Value::Null;
    if let Some(t) = remeasure
        && problems.is_empty()
        && g.failed == 0
        && g.stopped == 0
        && g.differ + g.measured > 0
    {
        let changed = accept_curves(game, &g)?;
        let says = format!(
            "{changed} curves measured again under the model's state mapping (schema {}), by gp corpus score --remeasure",
            g.mapping
        );
        let b = bump_as(game, true, Some(&says), None, true)?;
        remeasured =
            json!({"changed": changed, "from": m.corpus.version, "to": b.json["suggests"]});
        (now, m, problems) = checked(game)?;
        g.differ = 0;
        g.measured = 0;
        write = Some(t);
    }
    // A template whose `[scored_against]` names another version of this
    // corpus gets the record and the section brought to the corpus as it is.
    let mut caught_up = Value::Null;
    if let Some(t) = remeasure
        && write.is_none()
        && let Some(was) = behind(t, game, &m.corpus)
    {
        caught_up = json!({"from": was, "to": m.corpus.version});
        write = Some(t);
    }
    if !m.needs.mapping.is_empty() && g.mapping != m.needs.mapping {
        problems.push(format!(
            "the model's state mapping is schema {}, the corpus's curves were measured with {}",
            g.mapping, m.needs.mapping
        ));
    }
    if g.differ > 0 || g.failed > 0 || g.measured > 0 {
        problems.push(format!(
            "{} clips differ from the corpus's curve, {} measured again, {} couldn't be made",
            g.differ, g.measured, g.failed
        ));
    }
    let b = &g.bench;
    let model_hash = model_key(&g.model_text);
    // The contract's bars for the corpus's clips, when it sets them.
    let bars = fs::read_to_string(b.dir.join(gp_contract::CONTRACT_FILE))
        .ok()
        .and_then(|t| gp_contract::Contract::parse(&t).ok())
        .and_then(|c| c.corpus);
    let bar_together = bars
        .as_ref()
        .map(|c| c.together_permille())
        .transpose()
        .map_err(anyhow::Error::msg)?
        .flatten();
    let bar_longest = bars
        .as_ref()
        .map(|c| c.longest_frames())
        .transpose()
        .map_err(anyhow::Error::msg)?
        .flatten();
    let mut clips = Vec::new();
    let mut reads = serde_json::Map::new();
    for r in &g.set.clips {
        if earlier.is_none() && only.as_ref().is_some_and(|o| !o.contains(&r.id)) {
            continue;
        }
        // A clip the change can't move keeps the earlier score's row.
        if let Some((row, rd)) = earlier.as_ref().and_then(|(_, s)| s.kept.get(&r.id)) {
            reads.insert(r.id.clone(), rd.clone());
            clips.push(row.clone());
            continue;
        }
        let row = match g.outcomes.get(&r.id) {
            Some(Ok(o)) => {
                reads.insert(r.id.clone(), json!(o.reads));
                // Frames on which every measure agrees, of those measured.
                let all = (o.together_frames * 1000)
                    .checked_div(o.measured)
                    .unwrap_or(0);
                let together_ok = bar_together.map(|t| all >= t);
                // A long run is held to the longest stretch only when it
                // has that much play: a game over or a clear scene has
                // none.
                let longest_ok = match (&o.stretches, bar_longest) {
                    (Some(st), Some(t)) if st.play >= t => Some(st.longest_any() >= t),
                    _ => None,
                };
                let mut row = json!({"id": r.id, "recipe": now.recipes[&r.id],
                    "curve": crate::scenario::curve_hash(o, false),
                    "model_curve": crate::scenario::curve_hash(o, true),
                    "exact": o.exact, "permille": o.headline.together_permille, "score": o.score,
                    "frames": o.measured, "frames_together": o.together_frames,
                    "first": o.first.as_ref().map(|f| json!({"frame": f.frame, "original_frame": f.original_frame, "path": f.path})),
                    "moved": o.moved, "start": o.original_from,
                    "pairing": o.pairing, "out_of_scope": o.out_of_scope});
                if o.stopped {
                    row["stopped"] = json!(true);
                }
                if let Some(st) = &o.stretches {
                    row["stretches"] = json!({
                        "frames": st.frames, "together": st.together, "play": st.play,
                        "split_events": st.events,
                        "whole": st.whole.len(), "open_end": st.open_end,
                        "longest": st.longest(), "longest_any": st.longest_any(),
                        "median": st.median(), "p10": st.percentile(10),
                        "shortest": st.whole.first().copied().unwrap_or(0),
                        "at_least_5s": st.at_least(5), "at_least_10s": st.at_least(10),
                        "at_least_30s": st.at_least(30),
                    });
                }
                // The screen measure, for the gate's picture lane.
                if let Some(sc) = &o.screen {
                    row["screen"] = json!({
                        "sprites": {"same": sc.sprites.same, "total": sc.sprites.total},
                        "background": {"same": sc.background.same, "total": sc.background.total},
                    });
                }
                if together_ok.is_some() || longest_ok.is_some() {
                    row["bars"] = json!({"together": together_ok, "longest_stretch": longest_ok});
                }
                row
            }
            Some(Err(e)) => json!({"id": r.id, "recipe": now.recipes[&r.id], "error": e}),
            None => json!({"id": r.id, "recipe": now.recipes[&r.id], "error": "not made"}),
        };
        clips.push(row);
    }
    // The totals, from the rows: those scored now and those kept alike.
    let mut bar_lines = Vec::new();
    let mut pair_lines: Vec<String> = Vec::new();
    let (mut met, mut held) = (0usize, 0usize);
    let (mut exact, mut errors, mut permille_sum) = (0usize, 0usize, 0u64);
    for row in &clips {
        let id = row["id"].as_str().unwrap_or("");
        if row.get("error").is_some_and(|e| !e.is_null()) {
            errors += 1;
            continue;
        }
        if row["exact"] == true {
            exact += 1;
        }
        permille_sum += row["permille"].as_u64().unwrap_or(0);
        let together_ok = row["bars"]["together"].as_bool();
        let longest_ok = row["bars"]["longest_stretch"].as_bool();
        if together_ok.is_some() || longest_ok.is_some() {
            held += 1;
            if together_ok != Some(false) && longest_ok != Some(false) {
                met += 1;
            } else {
                let (t, f) = (
                    row["frames_together"].as_u64().unwrap_or(0),
                    row["frames"].as_u64().unwrap_or(0),
                );
                let all = (t * 1000).checked_div(f).unwrap_or(0);
                let mut why = vec![format!(
                    "{t}/{f} frames together ({}.{}%)",
                    all / 10,
                    all % 10
                )];
                if let Some(l) = row["stretches"]["longest_any"].as_u64() {
                    why.push(format!(
                        "longest stretch {} s",
                        gp_interp::lockstep::seconds(l)
                    ));
                }
                bar_lines.push(format!("  below the bars: {id} ({})", why.join(", ")));
            }
        }
        // A clip whose frames don't pair one to one from its start, or
        // that runs past the template's scope, says so.
        let mut said = Vec::new();
        if let Ok(p) =
            serde_json::from_value::<gp_interp::lockstep::Pairing>(row["pairing"].clone())
            && !p.lag.is_empty()
        {
            said.push(p.says());
        }
        if let Some(x) = row["out_of_scope"].as_object() {
            said.push(format!(
                "out of scope from the original's frame {} (the clip's {}), {} frames left out",
                x.get("original_frame").and_then(Value::as_u64).unwrap_or(0),
                x.get("frame").and_then(Value::as_u64).unwrap_or(0),
                x.get("frames").and_then(Value::as_u64).unwrap_or(0)
            ));
        }
        if !said.is_empty() {
            pair_lines.push(format!("  {id}: {}", said.join("; ")));
        }
    }
    let n = clips.len();
    // Every clip counts: one that can't be scored counts as 0.
    let fidelity = if n == 0 { 0 } else { permille_sum / n as u64 };
    let model_name = gp_model::Model::parse(&g.model_text)
        .map(|x| x.model.name)
        .unwrap_or_default();
    let contract = fs::read(b.dir.join(gp_contract::CONTRACT_FILE)).unwrap_or_default();
    let h = |x: &[u8]| gp_evidence::Hash::of(x).to_string();
    let record = json!({
        "_type": "https://in-toto.io/Statement/v1",
        "subject": [
            {"name": format!("corpus {} {}", m.corpus.name, m.corpus.version), "digest": {"blake3": m.corpus.hash}},
            {"name": format!("model {model_name}"), "digest": {"blake3": model_hash}},
        ],
        "predicateType": PREDICATE,
        "predicate": {
            "record_format": RECORD_FORMAT,
            "corpus": {"name": m.corpus.name, "version": m.corpus.version, "hash": m.corpus.hash},
            "rom": {"sha256": m.target.rom, "patch": if m.target.patch.is_empty() { Value::Null } else { json!(m.target.patch) }},
            "model": {"hash": model_hash, "short": crate::scenario::model_hash(&g.model_text), "name": model_name},
            "mapping": {"hash": h(b.mapping_text.as_bytes()), "schema": g.mapping},
            "contract": h(&contract),
            "profile": Value::Null,
            "runtime": {"name": "gp-interp", "scenario": gp_interp::scenario::SCENARIO_VERSION},
            "gp": {"version": env!("CARGO_PKG_VERSION"), "build": build()},
            "results": {"clips": n, "exact": exact, "errors": errors, "fidelity_permille": fidelity,
                "bars": if held > 0 { json!({"held": held, "met": met}) } else { Value::Null }},
            "clips": clips,
        },
    });
    let mut text = vec![format!(
        "On corpus {} {} ({}): {exact} of {n} clips exact, fidelity {}.{}%, {errors} couldn't be scored. Model {}.",
        m.corpus.name,
        m.corpus.version,
        short(&m.corpus.hash),
        fidelity / 10,
        fidelity % 10,
        short(&model_hash),
    )];
    if held > 0 {
        text.push(format!(
            "The contract's corpus bars{}{}: {met} of {held} clips meet them.",
            bar_together
                .map(|t| format!(", {}.{}% together", t / 10, t % 10))
                .unwrap_or_default(),
            bar_longest
                .map(|f| format!(
                    ", a longest stretch of {} s on a long run",
                    gp_interp::lockstep::seconds(f)
                ))
                .unwrap_or_default(),
        ));
        text.extend(bar_lines);
    }
    let offsets: BTreeSet<i64> = clips
        .iter()
        .filter_map(|c| c["pairing"]["offset"].as_i64())
        .collect();
    if !offsets.is_empty() {
        text.push(format!(
            "Pairing: model frame n is the original's n+k at each clip's start, k {}; {} clips skip a lag frame or leave the template's scope{}",
            offsets
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(", "),
            pair_lines.len(),
            if pair_lines.is_empty() { "." } else { ":" }
        ));
        text.extend(pair_lines);
    }

    let mut written = Value::Null;
    if let Some(t) = write {
        let tmodel = fs::read_to_string(t.join(gp_model::MODEL_FILE))
            .with_context(|| format!("reading {}'s model", t.display()))?;
        if model_key(&tmodel) != model_hash {
            problems.push(format!(
                "{}'s model isn't the one scored; the record would be for another model",
                t.display()
            ));
        }
        if problems.is_empty() {
            let file = record_path(t, &model_hash, &m.corpus.hash);
            fs::create_dir_all(file.parent().expect("a parent"))?;
            fs::write(
                &file,
                format!("{}\n", serde_json::to_string_pretty(&record)?),
            )?;
            let manifest = t.join(MANIFEST);
            if manifest.is_file() {
                let rel = relative(t, game);
                set_scored_against(&manifest, &m.corpus, &rel)?;
            }
            text.push(format!("Wrote the record to {}.", file.display()));
            written = json!(file);
        } else {
            text.push("Nothing written: a record is only for the corpus as it is.".into());
        }
    }
    for p in &problems {
        text.push(format!("problem: {p}"));
    }
    if let Some(r) = remeasured.as_object() {
        text.push(format!(
            "Measured again: {} curves, corpus {} -> {}.",
            r["changed"],
            r["from"].as_str().unwrap_or(""),
            r["to"].as_str().unwrap_or("")
        ));
    }
    if let Some(was) = caught_up["from"].as_str()
        && !written.is_null()
    {
        text.push(format!(
            "template.toml's [scored_against] brought from {was} to {}.",
            m.corpus.version
        ));
    }
    // What a later score can be measured against: the fingerprint of what
    // this one read, and each clip's reads. Not in the record: they describe
    // this build's run.
    let fingerprint = match &earlier {
        Some((fp, _)) => fp.clone(),
        None => {
            let mm = gp_model::Model::parse(&g.model_text)?;
            crate::affected::fingerprint(b, &mm, game)?
        }
    };
    let affected = match &earlier {
        Some((_, sel)) => {
            text.push(format!(
                "Against the earlier score: {} clips scored, {} kept ({}), in {:.1} s.",
                sel.score.len(),
                sel.kept.len(),
                sel.says,
                started.elapsed().as_secs_f64()
            ));
            json!({"scored": sel.score, "kept": sel.kept.len(), "says": sel.says,
                "changed": sel.changed, "stopped_at_split": g.stopped,
                "seconds": started.elapsed().as_secs_f64()})
        }
        None => Value::Null,
    };
    let v = json!({"ok": problems.is_empty(), "record": record, "written": written,
        "problems": problems, "remeasured": remeasured,
        "scored_against_from": caught_up["from"],
        "fingerprint": fingerprint, "reads": reads, "affected": affected});
    let mut out = Outcome::ok(v, text.join("\n"));
    if !problems.is_empty() {
        out.exit = 1;
    }
    Ok(out)
}
