//! Every format's JSON Schema, one file each in `schema/`, generated from
//! the Rust type that reads it (schema/README.md). `gp self schemas DIR`
//! writes them all and `--check` compares; `just docs` runs it, and the
//! test below compares the repo's `schema/` with this list on every run.
//!
//! A file's name has no version in it: the `$id` inside names the format
//! and its version, which bumps only for a breaking change. An optional
//! part is a named feature (docs/model.md, "Versions").
//!
//! game-printer's command was `gp docs schemas`; gameprinter's `gp docs cli`
//! became `gp self docs`, so this is `gp self schemas`. game-printer's
//! loop.json (gp loop, dropped) and chunks.json (gp-chunks, not ported) are
//! gone; corpus.json (the corpus plan), package.json and roms.json are new.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::Deserialize;
use serde_json::json;

use crate::Outcome;

/// One format: the file in `schema/`, what it describes, and how its
/// schema is made.
pub(crate) struct Format {
    pub file: &'static str,
    pub what: &'static str,
    pub make: fn() -> String,
}

fn with_id(mut schema: schemars::Schema, id: String, title: &str) -> String {
    schema.insert("$id".into(), id.into());
    schema.insert("title".into(), title.into());
    let mut text = serde_json::to_string_pretty(&schema).expect("schemas serialize");
    text.push('\n');
    text
}

fn clips() -> String {
    with_id(
        schemars::schema_for!(crate::clipset::ClipSet),
        format!("game-printer:clips@{}", crate::clipset::FORMAT),
        "Clip set, `clips.toml`",
    )
}

fn scoring_corpus() -> String {
    with_id(
        schemars::schema_for!(crate::corpus_score::CorpusFile),
        format!("game-printer:scoring-corpus@{}", crate::clipset::FORMAT),
        "Scoring corpus manifest, `corpus.toml` beside a clip set",
    )
}

fn package() -> String {
    with_id(
        schemars::schema_for!(PackageFile),
        "game-printer:package@1".into(),
        "A package, `package.toml`",
    )
}

fn capture_events() -> String {
    let mut t = gp_capture::schema_text();
    if !t.ends_with('\n') {
        t.push('\n');
    }
    t
}

fn watch_events() -> String {
    let mut t = gp_events::watch::schema_text();
    if !t.ends_with('\n') {
        t.push('\n');
    }
    t
}

/// Every format with a schema, in the order schema/README.md lists them.
pub(crate) const FORMATS: &[Format] = &[
    Format {
        file: "model-core.json",
        what: "Model core schema, `model.toml` (fixed core)",
        make: gp_model::schema::core,
    },
    Format {
        file: "platformer.json",
        what: "Platformer extension",
        make: gp_model::schema::platformer,
    },
    Format {
        file: "topdown.json",
        what: "Top-down extension",
        make: gp_model::schema::topdown,
    },
    Format {
        file: "model-platformer.json",
        what: "The core with the platformer extension merged in, for typed editing of `model.toml`",
        make: gp_model::schema::model_platformer,
    },
    Format {
        file: "contract.json",
        what: "Fidelity contract, `fidelity.toml` (fixed core)",
        make: gp_contract::schema,
    },
    Format {
        file: "mapping.json",
        what: "State mapping, `mapping.toml`",
        make: gp_interp::mapping::schema,
    },
    Format {
        file: "profiles.json",
        what: "Physics profiles, `profiles.toml`",
        make: gp_model::schema::profiles,
    },
    Format {
        file: "corpus.json",
        what: "Corpus plan, a game's `corpus.toml` (format 1.0.0)",
        make: crate::plan::schema,
    },
    Format {
        file: "chunks.json",
        what: "Chunk catalog, `gp chunks`'s `chunks.json`",
        make: gp_chunks::format::schema,
    },
    Format {
        file: "clips.json",
        what: "Clip set, `clips.toml`",
        make: clips,
    },
    Format {
        file: "scoring-corpus.json",
        what: "Scoring corpus manifest, the older `corpus.toml` beside a clip set",
        make: scoring_corpus,
    },
    Format {
        file: "package.json",
        what: "A package, `package.toml`",
        make: package,
    },
    Format {
        file: "roms.json",
        what: "A game's ROM sources, `roms.toml`",
        make: gp_evidence::registry::schema,
    },
    Format {
        file: "clip-corpus.json",
        what: "Clip corpus, a run's `corpus/corpus.json`",
        make: gp_interp::corpus::schema,
    },
    Format {
        file: "recording.json",
        what: "Recording manifest, the store's `manifest.json` of a recording",
        make: gp_evidence::manifest::RecordingManifest::json_schema,
    },
    Format {
        file: "slots.json",
        what: "Asset slot state, `slots.toml`",
        make: gp_mcp::catalog::schema,
    },
    Format {
        file: "theme.json",
        what: "A theme, `theme.toml`",
        make: gp_theme::format::theme_schema,
    },
    Format {
        file: "theme-stack.json",
        what: "A theme stack, `stack.toml`",
        make: gp_theme::format::stack_schema,
    },
    Format {
        file: "capture-events.json",
        what: "A capture run's event and input lines",
        make: capture_events,
    },
    Format {
        file: "rom-dataset.json",
        what: "A ROM dataset's rows, one schema a table (docs/formats/rom-dataset.md)",
        make: crate::rom_dataset::json_schema,
    },
    Format {
        file: "watch-events.json",
        what: "A watch session's event lines",
        make: watch_events,
    },
    Format {
        file: "image-config.json",
        what: "An image format config layer",
        make: gp_imagefmt::config::schema,
    },
];

/// The files in `dir` that differ from what [`FORMATS`] makes, and the
/// `.json` files there that no format makes.
pub(crate) fn stale(dir: &Path) -> (Vec<&'static str>, Vec<String>) {
    let stale = FORMATS
        .iter()
        .filter(|f| fs::read_to_string(dir.join(f.file)).ok().as_deref() != Some(&(f.make)()))
        .map(|f| f.file)
        .collect();
    let mut extra: Vec<String> = fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.ends_with(".json") && !FORMATS.iter().any(|f| f.file == n))
                .collect()
        })
        .unwrap_or_default();
    extra.sort();
    (stale, extra)
}

/// `gp self schemas DIR`: writes every schema, or with `check` writes
/// nothing and fails when one is stale or missing, or a file there is no
/// format's.
pub(crate) fn run(dir: &Path, check: bool) -> Result<Outcome> {
    if check {
        let (stale, extra) = stale(dir);
        let ok = stale.is_empty() && extra.is_empty();
        let mut lines: Vec<String> = Vec::new();
        for f in &stale {
            lines.push(format!(
                "{}/{f} is stale or missing; run just docs",
                dir.display()
            ));
        }
        for f in &extra {
            lines.push(format!("{}/{f} is no format's schema", dir.display()));
        }
        if ok {
            lines.push(format!("{} schemas are current.", FORMATS.len()));
        }
        return Ok(Outcome {
            json: json!({"ok": ok, "dir": dir, "stale": stale, "extra": extra}),
            text: lines.join("\n"),
            exit: u8::from(!ok),
        });
    }
    fs::create_dir_all(dir).with_context(|| format!("making {}", dir.display()))?;
    let mut written = Vec::new();
    for f in FORMATS {
        let p = dir.join(f.file);
        let text = (f.make)();
        if fs::read_to_string(&p).ok().as_deref() != Some(&text) {
            fs::write(&p, text).with_context(|| format!("writing {}", p.display()))?;
            written.push(f.file);
        }
    }
    Ok(Outcome::ok(
        json!({"dir": dir, "written": written, "formats": FORMATS.iter().map(|f| json!({"file": f.file, "what": f.what})).collect::<Vec<_>>()}),
        format!(
            "{} schemas in {}; {} rewritten.",
            FORMATS.len(),
            dir.display(),
            written.len()
        ),
    ))
}

/// The formats `gp model schema` prints, as game-printer's did.
#[derive(Clone, Copy, clap::ValueEnum)]
pub enum Which {
    Core,
    Platformer,
    /// The top-down extension.
    Topdown,
    /// The core with the platformer extension merged in, for editors.
    Model,
    Contract,
    Slots,
}

/// `gp model schema WHICH`.
pub(crate) fn print(which: Which) -> Result<Outcome> {
    let text = match which {
        Which::Core => gp_model::schema::core(),
        Which::Platformer => gp_model::schema::platformer(),
        Which::Topdown => gp_model::schema::topdown(),
        Which::Model => gp_model::schema::model_platformer(),
        Which::Contract => gp_contract::schema(),
        Which::Slots => gp_mcp::catalog::schema(),
    };
    let json: serde_json::Value = serde_json::from_str(&text)?;
    Ok(Outcome::ok(json, text.trim_end().to_owned()))
}

// package.toml, for editors. The readers (crate::package, gp-web-registry,
// the page) take what they need from the table, so these types only give
// an editor the known keys and their types; a key not listed here is
// allowed, as the readers allow it.

/// A package's `package.toml`.
#[allow(dead_code)]
#[derive(Deserialize, schemars::JsonSchema)]
struct PackageFile {
    package: PackageHead,
    #[serde(default)]
    tool: Option<PackageTool>,
    #[serde(default)]
    play: Option<PackagePlay>,
    #[serde(default)]
    rom: Option<PackageRom>,
    #[serde(default)]
    hooks: Option<PackageHooks>,
    /// The corpus plan's name, version and recipe-set hash, as corpus.toml
    /// records them.
    #[serde(default)]
    corpus: Option<PackageCorpus>,
    /// An open version kept beside the game: its model's path in the package.
    #[serde(default)]
    open: Option<BTreeMap<String, String>>,
}

#[allow(dead_code)]
#[derive(Deserialize, schemars::JsonSchema)]
struct PackageHead {
    /// `owner/name`.
    name: String,
    /// Semver.
    version: String,
    /// `game`, `content` (a theme) or `model`.
    kind: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    license: Option<String>,
    /// `commercial` (the package holds no game data) or `open`.
    #[serde(default)]
    tier: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, schemars::JsonSchema)]
struct PackageTool {
    /// The runtime interface the package was built against, such as
    /// `game-printer:runtime@0.4.0`.
    #[serde(rename = "runtime-wit", default)]
    runtime_wit: Option<String>,
    #[serde(rename = "sim-api", default)]
    sim_api: Option<String>,
    /// The gp versions it works with, a semver range.
    #[serde(default)]
    gp: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, schemars::JsonSchema)]
struct PackagePlay {
    /// The files and folders a player downloads.
    files: Vec<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, schemars::JsonSchema)]
struct PackageRom {
    /// How a person gets the ROM themselves, shown on the page.
    #[serde(default)]
    how_to_get: Option<String>,
    #[serde(default)]
    entry: Vec<PackageRomEntry>,
}

#[allow(dead_code)]
#[derive(Deserialize, schemars::JsonSchema)]
struct PackageRomEntry {
    id: String,
    /// `scored` (the corpus is scored on it) or `play`.
    role: String,
    #[serde(default)]
    title: Option<String>,
    /// The headerless SHA-256 of an NES ROM, or a romset's identity.
    sha256: String,
    /// `nes` (the default) or `arcade`.
    #[serde(default)]
    platform: Option<String>,
    /// An arcade entry's MAME set name.
    #[serde(default)]
    system: Option<String>,
    #[serde(default)]
    mapper: Option<u16>,
    #[serde(default)]
    submapper: Option<u8>,
    #[serde(default)]
    prg_rom_bytes: Option<u64>,
    #[serde(default)]
    chr_rom_bytes: Option<u64>,
    /// `unverified` or `verified`.
    #[serde(default)]
    board_status: Option<String>,
    /// The asset map, relative to the package folder.
    asset_map: String,
    asset_map_sha256: String,
    /// What lifting the ROM with the map gives, so a person's lift is checked.
    #[serde(default)]
    lifted_sha256: Option<String>,
    /// The model's levels this ROM plays, when not all.
    #[serde(default)]
    levels: Option<Vec<String>>,
    /// Model asset names to the map's names for this ROM.
    #[serde(default)]
    assets: Option<BTreeMap<String, String>>,
    /// What the scored ROM's model has that this ROM's game hasn't.
    #[serde(default)]
    without: Vec<PackageWithout>,
}

#[allow(dead_code)]
#[derive(Deserialize, schemars::JsonSchema)]
struct PackageWithout {
    what: String,
    #[serde(default)]
    levels: Vec<String>,
    #[serde(default)]
    parts: Vec<String>,
    /// Assets left blank.
    #[serde(default)]
    blank: Vec<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, schemars::JsonSchema)]
struct PackageHooks {
    /// The hook crate's folder in the package.
    #[serde(rename = "crate")]
    krate: String,
    /// The models the hooks are for.
    #[serde(default)]
    models: Vec<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, schemars::JsonSchema)]
struct PackageCorpus {
    name: String,
    version: String,
    recipes: String,
}

/// The named features as Markdown: docs/features.md (`gp self features`,
/// game-printer's `gp docs features`).
pub(crate) fn features_markdown() -> String {
    use gp_model::features::{Feature, Piece};
    let mut out = String::from(
        "# Features\n\n\
         Generated by `gp self features` from the registries in `crates/gp-model/src/feature_list.rs` (the model, the contract and the asset map) and `crates/gp-interp/src/mapping_features.rs`; `just docs` writes it. Don't edit it by hand.\n\n\
         A feature is an optional part of a format with a name. A model lists the ones it uses in `model.features`, a state mapping in `features`, or they're derived from the fields the file sets; a reader refuses a file that lists one it doesn't know, by name (docs/model.md, \"Versions\"). Adding one bumps no version. \"Since\" is the version it appeared in, from before versions stopped counting options, kept as a note; it's empty for a feature added after. A feature with no paths is found only by being listed.\n",
    );
    let all: Vec<&Feature> = gp_model::feature_list::FEATURES
        .iter()
        .chain(gp_interp::mapping_features::FEATURES)
        .chain(gp_model::feature_list::CONTRACT_FEATURES)
        .chain(gp_model::feature_list::ASSET_MAP_FEATURES)
        .collect();
    for (piece, title) in [
        (Piece::Core, "Core"),
        (Piece::Platformer, "Platformer extension"),
        (Piece::Mapping, "State mapping"),
        (Piece::Contract, "Fidelity contract"),
        (Piece::AssetMap, "Asset map"),
    ] {
        let rows: Vec<&&Feature> = all.iter().filter(|f| f.piece == piece).collect();
        out.push_str(&format!(
            "\n## {title}\n\n{} features.\n\n| Feature | Since | What | Paths |\n|---|---|---|---|\n",
            rows.len()
        ));
        for f in rows {
            let paths = f
                .paths
                .iter()
                .map(|p| format!("`{p}`"))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!(
                "| `{}` | {} | {} | {} |\n",
                f.name,
                f.since,
                f.about.replace('|', "\\|"),
                paths
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_schema() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schema")
    }

    /// The one schema test: every format's file is what its type makes,
    /// and `schema/` holds no other JSON.
    #[test]
    fn every_schema_file_is_current() {
        let (stale, extra) = stale(&repo_schema());
        assert!(
            stale.is_empty(),
            "stale or missing: {stale:?}; run just docs"
        );
        assert!(extra.is_empty(), "no format makes {extra:?}");
    }

    #[test]
    fn every_schema_is_json_with_no_version_in_its_name() {
        for f in FORMATS {
            let v: serde_json::Value = serde_json::from_str(&(f.make)()).unwrap();
            assert!(v.is_object(), "{}", f.file);
            assert!(
                !f.file.chars().any(|c| c.is_ascii_digit()),
                "{} has a version in its name",
                f.file
            );
        }
    }

    /// The schemas of the files a game's folder holds name their formats.
    #[test]
    fn the_schemas_name_their_formats() {
        for (file, id) in [
            ("mapping.json", "game-printer:mapping@"),
            ("profiles.json", "game-printer:profiles@"),
            ("corpus.json", "game-printer:corpus@"),
            ("clips.json", "game-printer:clips@"),
            ("scoring-corpus.json", "game-printer:scoring-corpus@"),
            ("package.json", "game-printer:package@"),
            ("roms.json", "game-printer:roms@"),
            ("clip-corpus.json", "game-printer:clip-corpus@"),
            ("recording.json", "game-printer:recording@"),
        ] {
            let f = FORMATS.iter().find(|f| f.file == file).unwrap();
            let v: serde_json::Value = serde_json::from_str(&(f.make)()).unwrap();
            assert!(v["$id"].as_str().unwrap().starts_with(id), "{file}");
            assert!(v.get("properties").is_some(), "{file} has no properties");
        }
    }
}
