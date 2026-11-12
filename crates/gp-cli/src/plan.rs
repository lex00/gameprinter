//! The corpus plan: `corpus.toml` at format 1.0.0, the recipes that make a
//! game's clips. The clips themselves, their curves and coverage are made
//! from it and the ROM on the machine that has the ROM, and published only
//! as attestations ([`crate::attest`]).
//!
//! Each clip is a start and presses from there, as canonical `.presses`
//! files in `inputs/` (`gp-presses 0`, `frames N`, a span a line), a frame
//! count, and metadata that isn't hashed. A plan names no probes: Donkey
//! Kong's clips don't use them yet, so a clip made by a probe is refused.
//!
//! A plan loads as the same [`ClipSet`] the older `clips.toml` does, so
//! everything after loading is the same code; only curves differ, being
//! derived rather than compared.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context as _, bail};
use serde::Deserialize;
use serde_json::json;

use crate::clipset::{self as cs, ClipSet, Header, Recipe, StartRecipe};
use crate::corpus_score::{Change, CorpusFile, FILE, Head, Needs, Target};

/// The plan format this gp reads.
pub(crate) const FORMAT: &str = "1.0.0";
/// What a recipe-set hash starts with.
const PREFIX: &str = "b3:";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanHead {
    format: String,
    name: String,
    version: String,
    #[serde(default)]
    title: String,
    /// The recipe-set hash ([`recipe_set_hash`]).
    recipes: String,
}

/// A clip's own presses: a file in `inputs/`, or spans inline.
#[derive(Deserialize)]
#[serde(untagged)]
enum Presses {
    File(String),
    Spans(Vec<String>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanClip {
    id: String,
    /// Metadata, not hashed.
    name: String,
    #[serde(default = "generated")]
    source: String,
    /// A `.presses` file from power-on; its frames are where the clip starts.
    start: String,
    presses: Presses,
    frames: u64,
    #[serde(default)]
    why: String,
    #[serde(default)]
    level: String,
}

fn generated() -> String {
    "generated".into()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    corpus: PlanHead,
    target: Target,
    #[serde(default)]
    needs: Needs,
    #[serde(rename = "clip", default)]
    clips: Vec<PlanClip>,
    #[serde(default)]
    changelog: Vec<Change>,
    /// Carried through unread, as `clips.toml`'s was.
    #[serde(default)]
    import: Option<toml::Value>,
}

/// A plan read and checked: the clips as a [`ClipSet`], each clip's recipe
/// hash, the recipe-set hash measured now, and the plan as a [`CorpusFile`]
/// with the recipe-set hash it records.
pub(crate) struct Loaded {
    pub set: ClipSet,
    pub recipes: BTreeMap<String, String>,
    pub hash: String,
    pub manifest: CorpusFile,
}

/// The plan in `dir`, or `None` when its `corpus.toml` is the older kind or
/// there is none.
pub(crate) fn load(dir: &Path) -> anyhow::Result<Option<Loaded>> {
    let read = |rel: &str| -> anyhow::Result<String> {
        fs::read_to_string(dir.join(rel)).with_context(|| format!("reading {rel}"))
    };
    load_with(&read, &dir.join(FILE))
}

/// [`load`] with each file of the game's folder, by its path in the
/// folder, read by `read`: a folder as a git revision has it, for `gp
/// corpus bump`. `file` names the plan in messages.
pub(crate) fn load_with(
    read: &dyn Fn(&str) -> anyhow::Result<String>,
    file: &Path,
) -> anyhow::Result<Option<Loaded>> {
    let Ok(text) = read(FILE) else {
        return Ok(None);
    };
    let major = toml::from_str::<toml::Value>(&text)
        .ok()
        .and_then(|v| v.get("corpus")?.get("format")?.as_str().map(str::to_owned))
        .unwrap_or_default();
    if !major.starts_with("1.") {
        return Ok(None);
    }
    let plan: Plan =
        toml::from_str(&text).with_context(|| format!("{} isn't a corpus plan", file.display()))?;
    if plan.corpus.format != FORMAT {
        bail!(
            "{} is plan format {}, this gp reads {FORMAT}",
            file.display(),
            plan.corpus.format
        );
    }
    let mut clips = Vec::with_capacity(plan.clips.len());
    let mut recipes = BTreeMap::new();
    let mut seen = std::collections::BTreeSet::new();
    for c in &plan.clips {
        if !seen.insert(c.id.as_str()) {
            bail!("{}: clip {} is listed twice", file.display(), c.id);
        }
        if c.source == cs::DENSE {
            bail!(
                "{}: clip {} is a dense clip, which a plan can't name yet",
                file.display(),
                c.id
            );
        }
        let (recipe, hash) = recipe(read, c).with_context(|| format!("clip {}", c.id))?;
        recipes.insert(c.id.clone(), hash);
        clips.push(recipe);
    }
    let hash = recipe_set_hash(&recipes);
    let manifest = CorpusFile {
        corpus: Head {
            format: plan.corpus.format.clone(),
            name: plan.corpus.name.clone(),
            version: plan.corpus.version.clone(),
            title: plan.corpus.title.clone(),
            hash: plan.corpus.recipes.clone(),
            ..Head::default()
        },
        target: plan.target.clone(),
        needs: plan.needs.clone(),
        recipes: BTreeMap::new(),
        changelog: plan.changelog.clone(),
    };
    let set = ClipSet {
        clipset: Header {
            format: FORMAT.into(),
            game: plan.target.game.clone(),
            rom: plan.target.rom.clone(),
            ..Header::default()
        },
        clips,
        import: plan.import,
        plan: true,
        ..ClipSet::default()
    };
    Ok(Some(Loaded {
        set,
        recipes,
        hash,
        manifest,
    }))
}

/// A `.presses` file of the plan: its frames and spans.
fn presses(
    read: &dyn Fn(&str) -> anyhow::Result<String>,
    rel: &str,
) -> anyhow::Result<(u64, Vec<String>)> {
    if !cs::inside(rel) {
        bail!("{rel} isn't inside the game's folder");
    }
    let text = read(rel)?;
    cs::parse_presses(&text).with_context(|| rel.to_owned())
}

/// A clip as a recipe, and its recipe hash: the BLAKE3 of its canonical
/// JSON, over its ID, its start's input, its presses and its frames.
fn recipe(
    read: &dyn Fn(&str) -> anyhow::Result<String>,
    c: &PlanClip,
) -> anyhow::Result<(Recipe, String)> {
    let (start_frames, start_spans) = presses(read, &c.start)?;
    let (own, presses_file) = match &c.presses {
        Presses::Spans(spans) => (spans.clone(), None),
        Presses::File(rel) => {
            let (frames, spans) = presses(read, rel)?;
            if frames != c.frames {
                bail!("{rel} has {frames} frames, the plan says {}", c.frames);
            }
            (spans, Some(rel.clone()))
        }
    };
    let start = cs::presses_text(start_frames, &start_spans);
    let v = json!({
        "id": c.id,
        "start": {"input": gp_evidence::Hash::of(start.as_bytes()).to_string()},
        "presses": own,
        "frames": c.frames,
    });
    let hash = gp_evidence::Hash::of(&serde_json::to_vec(&v)?).to_string();
    let recipe = Recipe {
        id: c.id.clone(),
        name: c.name.clone(),
        source: c.source.clone(),
        presses: if presses_file.is_some() {
            Vec::new()
        } else {
            own
        },
        presses_file,
        frames: c.frames,
        why: c.why.clone(),
        start: StartRecipe {
            presses_from_power_on: Some(c.start.clone()),
            frame: start_frames,
            context: "save_point".into(),
            level: c.level.clone(),
            ..StartRecipe::default()
        },
        ..Recipe::default()
    };
    Ok((recipe, hash))
}

/// The recipe-set hash: `gp-corpus-recipes 1` and a line `ID RECIPE-HASH`
/// per clip, sorted by ID. It names the corpus as shipped.
pub(crate) fn recipe_set_hash(recipes: &BTreeMap<String, String>) -> String {
    let mut s = String::from("gp-corpus-recipes 1\n");
    for (id, h) in recipes {
        s.push_str(id);
        s.push(' ');
        s.push_str(h);
        s.push('\n');
    }
    format!("{PREFIX}{}", gp_evidence::Hash::of(s.as_bytes()))
}

/// The curve-set hash: `gp-curves 1` and a line `ID CURVE COVERAGE` per
/// clip, sorted by ID, `-` for a clip with no coverage hash. It says what
/// the original gives on a recipe set, under one mapping and ROM.
pub(crate) fn curve_set_hash(curves: &BTreeMap<String, (String, String)>) -> String {
    let mut s = String::from("gp-curves 1\n");
    for (id, (curve, coverage)) in curves {
        s.push_str(id);
        s.push(' ');
        s.push_str(curve);
        s.push(' ');
        s.push_str(if coverage.is_empty() { "-" } else { coverage });
        s.push('\n');
    }
    format!("{PREFIX}{}", gp_evidence::Hash::of(s.as_bytes()))
}

/// The name of the table a TOML line heads (`corpus`, `clip` for
/// `[[clip]]`, `import.exclude`), if it heads one.
fn header(line: &str) -> Option<&str> {
    let l = line.split_once('#').map_or(line, |(a, _)| a).trim();
    let inner = l
        .strip_prefix("[[")
        .and_then(|x| x.strip_suffix("]]"))
        .or_else(|| l.strip_prefix('[').and_then(|x| x.strip_suffix(']')))?
        .trim();
    (inner.starts_with(|c: char| c.is_ascii_alphabetic())
        && inner
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c)))
    .then_some(inner)
}

/// A TOML file's tables: each one's name and its lines, from its header to
/// its last line that isn't blank or a comment. The lines between
/// (comments, blank lines) are no table's.
pub(crate) fn tables(lines: &[&str]) -> Vec<(String, usize, usize)> {
    let heads: Vec<(usize, &str)> = lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| header(l).map(|h| (i, h)))
        .collect();
    let mut out = Vec::new();
    for (k, &(i, h)) in heads.iter().enumerate() {
        let to = heads.get(k + 1).map_or(lines.len(), |&(j, _)| j);
        let end = (i..to)
            .rev()
            .find(|&j| {
                let l = lines[j].trim();
                !l.is_empty() && !l.starts_with('#')
            })
            .map_or(i + 1, |j| j + 1);
        out.push((h.to_owned(), i, end));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gate::recipe_ids;

    const PLAN: &str = r#"
[corpus]
format = "1.0.0"
name = "test"
version = "7.0.0"
recipes = "RECIPES"

[target]
game = "test"
rom = "aa"

[needs]
mapping = "22"

[[clip]]
id = "a.1"
name = "first"
source = "played"
start = "inputs/a.start.presses"
presses = "inputs/a.presses"
frames = 10

[[clip]]
id = "b.1"
name = "second"
source = "generated"
start = "inputs/b.start.presses"
presses = []
frames = 5
"#;

    fn folder() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join("inputs")).unwrap();
        for (f, t) in [
            (
                "inputs/a.start.presses",
                "gp-presses 0\nframes 4\nstart@1-2\n",
            ),
            ("inputs/a.presses", "gp-presses 0\nframes 10\nright@0-5\n"),
            ("inputs/b.start.presses", "gp-presses 0\nframes 8\n"),
        ] {
            fs::write(d.path().join(f), t).unwrap();
        }
        fs::write(d.path().join(FILE), PLAN).unwrap();
        d
    }

    #[test]
    fn a_plan_loads_as_a_clip_set_and_hashes_its_recipes() {
        let d = folder();
        let p = load(d.path()).unwrap().unwrap();
        assert!(p.set.plan);
        assert_eq!(p.set.clips.len(), 2);
        let a = &p.set.clips[0];
        assert_eq!(a.presses_file.as_deref(), Some("inputs/a.presses"));
        assert_eq!(a.start.frame, 4);
        assert_eq!(a.curve, "", "a plan holds no curve");
        // The recorded hash is carried, the measured one is the set's.
        assert_eq!(p.manifest.corpus.hash, "RECIPES");
        assert_eq!(p.hash, recipe_set_hash(&p.recipes));
        assert!(p.hash.starts_with(PREFIX));
    }

    #[test]
    fn one_press_changed_changes_the_recipe_hash() {
        let d = folder();
        let before = load(d.path()).unwrap().unwrap().hash;
        fs::write(
            d.path().join("inputs/a.presses"),
            "gp-presses 0\nframes 10\nright@0-6\n",
        )
        .unwrap();
        let after = load(d.path()).unwrap().unwrap().hash;
        assert_ne!(before, after);
    }

    #[test]
    fn a_presses_file_with_other_frames_is_refused() {
        let d = folder();
        fs::write(
            d.path().join("inputs/a.presses"),
            "gp-presses 0\nframes 11\nright@0-5\n",
        )
        .unwrap();
        assert!(load(d.path()).is_err());
    }

    #[test]
    fn a_path_leaving_the_folder_is_refused() {
        let d = folder();
        fs::write(
            d.path().join(FILE),
            PLAN.replace("inputs/a.presses", "../outside.presses"),
        )
        .unwrap();
        assert!(load(d.path()).is_err());
    }

    #[test]
    fn an_older_corpus_file_is_not_a_plan() {
        let d = tempfile::tempdir().unwrap();
        fs::write(
            d.path().join(FILE),
            "[corpus]\nformat = \"0.5.0\"\nname = \"x\"\nversion = \"1.0.0\"\nhash = \"\"\n",
        )
        .unwrap();
        assert!(load(d.path()).unwrap().is_none());
        assert!(load(Path::new("/nonexistent/x")).unwrap().is_none());
    }

    #[test]
    fn the_gate_reads_a_plans_clip_ids() {
        let got: Vec<String> = recipe_ids(PLAN).into_iter().collect();
        assert_eq!(got, ["a.1", "b.1"]);
        let older = "[recipes]\n\"a.1\" = \"x\"\n";
        assert_eq!(
            recipe_ids(older).into_iter().collect::<Vec<_>>(),
            ["a.1"],
            "an older file's [recipes] still read"
        );
    }

    #[test]
    fn the_curve_set_hash_follows_every_curve() {
        let mut m = BTreeMap::new();
        m.insert("a".to_owned(), ("c1".to_owned(), String::new()));
        let one = curve_set_hash(&m);
        m.get_mut("a").unwrap().0 = "c2".to_owned();
        assert_ne!(one, curve_set_hash(&m));
    }
}
