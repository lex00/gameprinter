//! `gp dataset export` (docs/research/datasets.md): a game's corpus scored
//! on its model as it stands, written as a data set of tables a person
//! can load in a few lines (pandas, polars, DuckDB, Hugging Face
//! `datasets`). One table a kind of row:
//!
//! | Table | One row a |
//! |---|---|
//! | `clips` | clip: its recipe (start, presses, frames), its score and its curve hashes |
//! | `measures` | clip, frame and measure: the original's and the model's value |
//! | `events` | body event: a turn or change of a body, in one game, on a frame |
//! | `splits` | track's first wrong frame in a clip |
//!
//! Beside them it writes a dataset card (`README.md`, with the YAML the
//! Hub reads), Croissant metadata (`croissant.json`) and a score record
//! (`record.json`): everything the scores were made from, by hash, shaped
//! as an unsigned in-toto statement so a later signature can wrap it.
//!
//! Nothing here reads a frame, a snapshot's bytes or RAM beyond the
//! mapping's measures, and nothing is published: the tables go to `--out`.
//! Whether the `measures` table may be shared is an open question
//! (docs/research/datasets.md); the rest are recipes, hashes and scores.
//!
//! Ported from game-printer's serve/dataset.rs. There `--run` named a
//! capture run, whose clips and model the export scored; gameprinter has
//! no capture runs, so the clips are a game package's corpus (`--game`),
//! made again from the ROM into run RUN and scored on its model, as `gp
//! corpus score` does (crate::clipset::regen_with). The columns are the
//! old ones; a clip's `start_input` is its start's presses file and
//! `presses_log_blake3` the BLAKE3 of its presses file, since every corpus
//! clip is a recipe. Frames count at the original's rate: an NES's 60.0988
//! Hz, an arcade board's (Tapper's 30 Hz) on a romset.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use anyhow::Context as _;
use serde_json::{Value, json};

use crate::clipset::Recipe;
pub(crate) use crate::dataset_tables::{Col, Field, Table, croissant_for, sha256, write_parquet};
use gp_interp::scenario::Outcome as Ran;

/// The data set format's version: the tables' columns and the files
/// beside them. A column added is a minor change, one removed or changed
/// in meaning a major one.
pub(crate) const DATASET_FORMAT: &str = "0.1.0";

/// NTSC NES frames a second, the unit every `frame` column counts in.
const NTSC_HZ: &str = "60.0988";

/// Midway MCR boards (Tapper) run 30 frames a second, the unit on a romset.
const ARCADE_HZ: &str = "30";

impl Col {
    fn cell(&self, i: usize) -> String {
        match self {
            Col::Str(v) => v[i].clone().unwrap_or_default(),
            Col::Int(v) => v[i].to_string(),
            Col::OptInt(v) => v[i].map(|x| x.to_string()).unwrap_or_default(),
            Col::Bool(v) => v[i].to_string(),
            Col::Num(v) => v[i].map(|x| x.to_string()).unwrap_or_default(),
        }
    }
}

impl Table {
    fn col(&mut self, name: &str) -> &mut Col {
        &mut self
            .fields
            .iter_mut()
            .find(|f| f.name == name)
            .expect("a column of the table")
            .col
    }

    pub(crate) fn push_str(&mut self, name: &str, v: impl Into<Option<String>>) {
        if let Col::Str(c) = self.col(name) {
            c.push(v.into());
        }
    }

    pub(crate) fn push_int(&mut self, name: &str, v: i64) {
        if let Col::Int(c) = self.col(name) {
            c.push(v);
        }
    }

    pub(crate) fn push_opt_int(&mut self, name: &str, v: Option<i64>) {
        if let Col::OptInt(c) = self.col(name) {
            c.push(v);
        }
    }

    pub(crate) fn push_bool(&mut self, name: &str, v: bool) {
        if let Col::Bool(c) = self.col(name) {
            c.push(v);
        }
    }

    pub(crate) fn push_num(&mut self, name: &str, v: Option<f64>) {
        if let Col::Num(c) = self.col(name) {
            c.push(v);
        }
    }
}

pub(crate) fn s(name: &'static str, unit: &'static str, about: &'static str) -> Field {
    Field {
        name,
        unit,
        about,
        col: Col::Str(Vec::new()),
    }
}
pub(crate) fn i(name: &'static str, unit: &'static str, about: &'static str) -> Field {
    Field {
        name,
        unit,
        about,
        col: Col::Int(Vec::new()),
    }
}
pub(crate) fn b(name: &'static str, about: &'static str) -> Field {
    Field {
        name,
        unit: "",
        about,
        col: Col::Bool(Vec::new()),
    }
}
pub(crate) fn oi(name: &'static str, unit: &'static str, about: &'static str) -> Field {
    Field {
        name,
        unit,
        about,
        col: Col::OptInt(Vec::new()),
    }
}
pub(crate) fn n(name: &'static str, unit: &'static str, about: &'static str) -> Field {
    Field {
        name,
        unit,
        about,
        col: Col::Num(Vec::new()),
    }
}

fn tables() -> [Table; 4] {
    [
        Table {
            name: "clips",
            about: "One row a clip: its recipe, its score on the model, and the hashes of both games' curves",
            key: &["clip_id"],
            fields: vec![
                s(
                    "clip_id",
                    "",
                    "The clip's ID, stable for the same model and corpus",
                ),
                s("name", "", "The clip in plain words"),
                s(
                    "source",
                    "",
                    "played (by a person) or generated (by the tool)",
                ),
                s(
                    "start_input",
                    "",
                    "The recording or input log the start is taken from, by name",
                ),
                i(
                    "start_frame",
                    "frames",
                    "The original's frame the clip starts at, from power-on",
                ),
                i("frames", "frames", "Frames the clip runs"),
                s(
                    "presses",
                    "",
                    "Its presses as button spans (a@0-14), JSON list; empty for a played clip",
                ),
                s(
                    "presses_log_blake3",
                    "",
                    "BLAKE3 of a played clip's input log, else null",
                ),
                b("exact", "Every measure within its tolerance on every frame"),
                i(
                    "score",
                    "splits",
                    "How far from exact: one a measure split on a frame",
                ),
                i(
                    "measures_exact",
                    "count",
                    "Measures exact over the whole clip",
                ),
                i("measures", "count", "Measures compared"),
                i(
                    "together_permille",
                    "1/1000",
                    "Frames together as a share of every measure's frames",
                ),
                s(
                    "curve_original",
                    "",
                    "BLAKE3 of the original's values of every measure on every frame: what regenerating the clip from the ROM must give",
                ),
                s(
                    "curve_model",
                    "",
                    "The same for the model; changes with every fix",
                ),
                s("says", "", "The score in plain words"),
            ],
        },
        Table {
            name: "measures",
            about: "One row a clip, frame and measure: the original's and the model's value, from the state mapping",
            key: &["clip_id", "frame", "path"],
            fields: vec![
                s("clip_id", "", "The clip, as in clips"),
                i("frame", "frames", "Frames from the clip's start"),
                s("path", "", "The model path measured (entities.player[0].x)"),
                s(
                    "owner",
                    "",
                    "The most specific mechanic that owns the measure",
                ),
                s(
                    "original",
                    "model units",
                    "The original's value as exact decimal text; empty for a name",
                ),
                s(
                    "model",
                    "model units",
                    "The model's value as exact decimal text; empty for a name",
                ),
                n(
                    "original_f64",
                    "model units",
                    "original as a float, for plotting",
                ),
                n("model_f64", "model units", "model as a float, for plotting"),
                s("distance", "model units", "How far apart the two are"),
            ],
        },
        Table {
            name: "events",
            about: "One row a turn or change of a body other than the player, in one game",
            key: &["clip_id", "body", "game", "frame", "what"],
            fields: vec![
                s("clip_id", "", "The clip, as in clips"),
                s("body", "", "The body in words (the second goomba)"),
                s("kind", "", "The model entity kind"),
                s("game", "", "original or model"),
                i("frame", "frames", "Frames from the clip's start"),
                s("what", "", "The event in words (turns left)"),
            ],
        },
        Table {
            name: "splits",
            about: "One row a track's first wrong frame in a clip that isn't exact",
            key: &["clip_id", "track"],
            fields: vec![
                s("clip_id", "", "The clip, as in clips"),
                s("track", "", "The entity kind, camera or game"),
                i("frame", "frames", "Frames from the clip's start"),
                i(
                    "original_frame",
                    "frames",
                    "The original's frame, from power-on",
                ),
                s("path", "", "The first measure wrong there"),
                s("original", "model units", "The original's value"),
                s("model", "model units", "The model's value"),
                s("says", "", "In plain words"),
            ],
        },
    ]
}

fn num(v: &str) -> Option<f64> {
    v.parse::<f64>().ok()
}

/// A clip's row in `clips`, and its rows in the other tables. `dir` is the
/// corpus folder its presses files are read from.
fn fill(t: &mut [Table; 4], dir: &Path, r: &Recipe, o: &Ran) {
    let [clips, measures, events, splits] = t;
    let s = r;
    let st = &r.start;
    clips.push_str("clip_id", s.id.clone());
    clips.push_str("name", s.name.clone());
    clips.push_str("source", s.source.clone());
    clips.push_str(
        "start_input",
        st.presses_from_power_on.clone().or(st.recording.clone()),
    );
    clips.push_int("start_frame", st.frame as i64);
    clips.push_int("frames", s.frames as i64);
    clips.push_str(
        "presses",
        serde_json::to_string(&s.presses).unwrap_or_default(),
    );
    clips.push_str(
        "presses_log_blake3",
        s.presses_file
            .as_ref()
            .and_then(|p| fs::read(dir.join(p)).ok())
            .map(|b| gp_evidence::Hash::of(&b).to_string()),
    );
    clips.push_bool("exact", o.exact);
    clips.push_int("score", o.score as i64);
    clips.push_int("measures_exact", o.headline.exact as i64);
    clips.push_int("measures", o.headline.measures as i64);
    clips.push_int("together_permille", o.headline.together_permille as i64);
    clips.push_str("curve_original", crate::scenario::curve_hash(o, false));
    clips.push_str("curve_model", crate::scenario::curve_hash(o, true));
    clips.push_str("says", o.says.clone());

    for m in &o.measures {
        let owner = m.owners.first().cloned();
        let frames = m.original.len().max(m.model.len());
        for f in 0..frames {
            let a = m.original.get(f).cloned().unwrap_or_default();
            let z = m.model.get(f).cloned().unwrap_or_default();
            measures.push_str("clip_id", s.id.clone());
            measures.push_int("frame", f as i64);
            measures.push_str("path", m.path.clone());
            measures.push_str("owner", owner.clone());
            measures.push_num("original_f64", num(&a));
            measures.push_num("model_f64", num(&z));
            measures.push_str("original", a);
            measures.push_str("model", z);
            measures.push_str("distance", m.distance.get(f).cloned());
        }
    }
    for body in &o.bodies {
        for e in &body.events {
            events.push_str("clip_id", s.id.clone());
            events.push_str("body", body.name.clone());
            events.push_str("kind", body.kind.clone());
            events.push_str("game", e.game.clone());
            events.push_int("frame", e.frame as i64);
            events.push_str("what", e.what.clone());
        }
    }
    for tr in &o.tracks {
        splits.push_str("clip_id", s.id.clone());
        splits.push_str("track", tr.track.clone());
        splits.push_int("frame", tr.frame as i64);
        splits.push_int("original_frame", tr.original_frame as i64);
        splits.push_str("path", tr.path.clone());
        splits.push_str("original", tr.original.clone());
        splits.push_str("model", tr.model.clone());
        splits.push_str("says", tr.says.clone());
    }
}

fn csv_cell(c: &str) -> String {
    if c.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", c.replace('"', "\"\""))
    } else {
        c.to_owned()
    }
}

fn write_csv(t: &Table, path: &Path) -> anyhow::Result<()> {
    let mut out = String::new();
    let head: Vec<&str> = t.fields.iter().map(|f| f.name).collect();
    out.push_str(&head.join(","));
    out.push('\n');
    for r in 0..t.rows() {
        let row: Vec<String> = t.fields.iter().map(|f| csv_cell(&f.col.cell(r))).collect();
        out.push_str(&row.join(","));
        out.push('\n');
    }
    fs::write(path, out).with_context(|| format!("writing {}", path.display()))
}

/// The dataset card: the Hub's YAML (one config a table), then what it
/// is, where it came from, what it holds and how to load it.
fn card(
    run: &str,
    rec: &Value,
    tables: &[Table; 4],
    ext: &str,
    files: &[(String, String, usize)],
    hz: &str,
) -> String {
    let mut c = String::from("---\n");
    let _ = writeln!(c, "pretty_name: \"Game Printer clips: {run}\"");
    c.push_str("license: other\n");
    c.push_str("tags:\n- game-printer\n- tabular\n- timeseries\n");
    c.push_str("configs:\n");
    for (k, t) in tables.iter().enumerate() {
        let _ = writeln!(c, "- config_name: {}", t.name);
        let _ = writeln!(c, "  data_files: \"data/{}.{ext}\"", t.name);
        if k == 0 {
            c.push_str("  default: true\n");
        }
    }
    c.push_str("---\n\n");
    let _ = writeln!(c, "# Game Printer clips: {run}\n");
    let p = &rec["predicate"];
    let _ = writeln!(
        c,
        "The clips of run `{run}`, each scored on the rebuilt game's model against the original, frame by frame. Made by `gp dataset export` (data set format {DATASET_FORMAT}).\n"
    );
    c.push_str("Nothing of the game is in it: no ROM, frames, sprites, level data or emulator snapshots. Each clip is a recipe (a start, presses and a frame count) that anyone with their own copy of the ROM regenerates, and `curve_original` is the hash the regenerated clip must give.\n\n");
    c.push_str("## What it was made from\n\n| Input | Value |\n|---|---|\n");
    for (k, v) in [
        ("Source", &p["source"]),
        ("ROM SHA-256", &p["rom"]["sha256"]),
        ("Model hash", &p["model"]["hash"]),
        ("Mapping hash", &p["mapping"]["hash"]),
        ("Mapping schema", &p["mapping"]["schema"]),
        ("Contract hash", &p["contract"]),
        ("Runtime", &p["runtime"]["name"]),
        ("Scenario version", &p["runtime"]["scenario"]),
        ("gp version", &p["gp"]["version"]),
        ("gp build", &p["gp"]["build"]),
    ] {
        let v = v.as_str().map_or_else(|| v.to_string(), str::to_owned);
        let _ = writeln!(c, "| {k} | `{v}` |");
    }
    let r = &p["results"];
    let _ = writeln!(
        c,
        "\nClips scored: {}, exact: {}, failed to start: {}.\n",
        r["clips"], r["exact"], r["errors"]
    );
    c.push_str("## Tables\n\n");
    let _ = writeln!(
        c,
        "Frames are the original's, {hz} a second. Measure values are in the model's units: pixels for places, pixels a frame for speeds, written as exact decimals (the original's 1/256 subpixels are the fraction).\n"
    );
    for t in tables {
        let rows = files
            .iter()
            .find(|f| f.0.contains(&format!("/{}.", t.name)))
            .map_or(0, |f| f.2);
        let _ = writeln!(
            c,
            "### {}\n\n{}. Key: {}. Rows: {rows}.\n",
            t.name,
            t.about,
            t.key.join(", ")
        );
        c.push_str("| Column | Type | Unit | Meaning |\n|---|---|---|---|\n");
        for f in &t.fields {
            let ty = f.col.croissant_type().trim_start_matches("sc:");
            let _ = writeln!(c, "| `{}` | {ty} | {} | {} |", f.name, f.unit, f.about);
        }
        c.push('\n');
    }
    c.push_str("## Load it\n\n```python\nimport duckdb\nduckdb.sql(\"SELECT path, count(*) FROM 'data/measures.parquet' WHERE original <> model GROUP BY path ORDER BY 2 DESC\").show()\n\nimport polars as pl\nclips = pl.read_parquet(\"data/clips.parquet\")\n\nfrom datasets import load_dataset\nm = load_dataset(\"parquet\", data_files=\"data/measures.parquet\")[\"train\"]\n```\n\n");
    c.push_str("## Files\n\n| File | SHA-256 | Rows |\n|---|---|---|\n");
    for (f, h, rows) in files {
        let _ = writeln!(c, "| `{f}` | `{h}` | {rows} |");
    }
    c.push_str("\n## Use and license\n\nThe recipes, scores and hashes are Game Printer's. The per-frame measures are numbers read through the state mapping from the original game's memory; whether they may be shared is an open question (docs/research/datasets.md), so keep this data set local until it is decided. Names and marks belong to their owners. `record.json` lists every input by hash.\n");
    c
}

/// Croissant 1.0 metadata: one FileObject and one RecordSet a table.
fn croissant(
    run: &str,
    tables: &[Table; 4],
    ext: &str,
    files: &[(String, String, usize)],
) -> Value {
    croissant_for(
        &format!("game-printer-{run}"),
        &format!(
            "Clips of run {run} scored on a Game Printer model against the original, frame by frame. No game data: recipes, scores, hashes and mapped measures."
        ),
        DATASET_FORMAT,
        tables,
        ext,
        files,
    )
}

/// `gp dataset export --run RUN --game DIR --out OUT`: the game's corpus
/// made into RUN, scored on the model and written as tables.
pub(crate) fn cli(
    paths: &gp_evidence::Paths,
    game: &Path,
    run: &str,
    how: &crate::clip_score::Open,
    out: &Path,
    format: &str,
) -> anyhow::Result<crate::Outcome> {
    let ext = match format {
        "parquet" => "parquet",
        "csv" => "csv",
        other => anyhow::bail!("--format is parquet or csv, not {other}"),
    };
    let (t, record, clips, exact, errors, hz) = scored(paths, game, run, how)?;
    write_run(run, out, ext, t, record, clips, exact, errors, hz)
}

/// A game's corpus made into run `run` and scored on its model: the four
/// tables, the score record (its subject and files not filled in yet), the
/// clips, exact and failed counts, and the frame rate.
#[allow(clippy::type_complexity)]
pub(crate) fn scored(
    paths: &gp_evidence::Paths,
    game: &Path,
    run: &str,
    how: &crate::clip_score::Open,
) -> anyhow::Result<([Table; 4], Value, usize, usize, usize, &'static str)> {
    use crate::clipset as cs;
    let rom = crate::clip_score::rom_of(paths, game, how.rom)?;
    let mdir = how.model.unwrap_or(game).to_owned();
    let assets = crate::corpus_score::assets_of(paths, game, &mdir, how.assets, &rom)?;
    let (source, _) = crate::hosted::Source::open_dir(&mdir, Some(&assets), paths, how.module)?;
    let dir = crate::corpus_score::run_dir(&paths.home, run)?;
    fs::create_dir_all(&dir)?;
    let work = cs::Work {
        paths,
        run_dir: &dir,
        run,
        mdir: &mdir,
        assets: &assets,
        rom: rom.clone(),
        source: std::sync::Arc::new(source),
    };
    let g = cs::regen_with(&work, game, None, &cs::Scoring::default())?;
    let b = &g.bench;

    let mut t = tables();
    let (mut exact, mut errors) = (0usize, Vec::new());
    for r in &g.set.clips {
        match g.outcomes.get(&r.id) {
            Some(Ok(o)) => {
                exact += usize::from(o.exact);
                fill(&mut t, game, r, o);
            }
            Some(Err(e)) => errors.push(json!({"clip_id": r.id, "error": e})),
            None => errors.push(json!({"clip_id": r.id, "error": "not made"})),
        }
    }

    // The ROM's identity as roms.toml registers it: an NES image's PRG and
    // CHR without the header, a romset's identity. The whole file's hash
    // and BLAKE3 go beside it.
    let arcade = crate::original::Platform::of_file(&rom) == crate::original::Platform::Arcade;
    let rom_id = crate::roms::sha256_of(&rom).unwrap_or_else(|_| sha256(&rom));
    let hash = |b: &[u8]| gp_evidence::Hash::of(b).to_string();
    let contract = fs::read(b.dir.join(gp_contract::CONTRACT_FILE)).unwrap_or_default();
    let model = b.hosted.parse_model()?;
    let record = json!({
        "_type": "https://in-toto.io/Statement/v1",
        "subject": [{"name": format!("{run}.clips"), "digest": {"blake3": ""}}],
        "predicateType": "urn:game-printer:score-record:0",
        "predicate": {
            "dataset_format": DATASET_FORMAT,
            "run": run,
            "source": b.mapping.source,
            "rom": {"sha256": rom_id, "file_sha256": sha256(&rom), "blake3": hash(&rom), "patch": Value::Null},
            "model": {"hash": crate::corpus_score::model_key(&g.model_text), "name": model.model.name},
            "mapping": {"hash": hash(b.mapping_text.as_bytes()), "schema": b.mapping.schema},
            "contract": hash(&contract),
            "hooks": how.module.and_then(|m| fs::read(m).ok()).map(|m| hash(&m)),
            "assets": hash(&b.lifted_bytes),
            "profile": Value::Null,
            "runtime": {"name": "gp-interp", "scenario": gp_interp::scenario::SCENARIO_VERSION},
            "gp": {"version": env!("CARGO_PKG_VERSION"), "build": crate::corpus_score::build(), "commit": Value::Null},
            "clips": g.set.clips.iter().map(|r| json!({"id": r.id, "spec": cs::recipe_hash(r, &|p: &str| fs::read_to_string(game.join(p)).map_err(anyhow::Error::from)).ok()})).collect::<Vec<_>>(),
            "results": {"clips": g.set.clips.len(), "exact": exact, "errors": errors.len(), "failed": errors},
        },
    });

    let n = errors.len();
    let hz = if arcade { ARCADE_HZ } else { NTSC_HZ };
    Ok((t, record, g.set.clips.len(), exact, n, hz))
}

#[allow(clippy::too_many_arguments)]
fn write_run(
    run: &str,
    out: &Path,
    ext: &str,
    t: [Table; 4],
    record: Value,
    clips: usize,
    exact: usize,
    errors: usize,
    hz: &str,
) -> anyhow::Result<crate::Outcome> {
    let data = out.join("data");
    fs::create_dir_all(&data).with_context(|| format!("making {}", data.display()))?;
    let meta = vec![
        ("gp.dataset.format".to_owned(), DATASET_FORMAT.to_owned()),
        ("gp.run".to_owned(), run.to_owned()),
        (
            "gp.model".to_owned(),
            record["predicate"]["model"]["hash"]
                .as_str()
                .unwrap_or("")
                .to_owned(),
        ),
        ("gp.frame_rate_hz".to_owned(), hz.to_owned()),
    ];
    let mut files = Vec::new();
    for tb in &t {
        let rel = format!("data/{}.{ext}", tb.name);
        let p = out.join(&rel);
        if ext == "parquet" {
            write_parquet(tb, &p, &meta)?;
        } else {
            write_csv(tb, &p)?;
        }
        files.push((rel, sha256(&fs::read(&p)?), tb.rows()));
    }
    // The record's subject is the clips table, by hash: what a signature
    // over the record vouches for.
    let mut record = record;
    record["subject"][0]["digest"] = json!({"sha256": files[0].1});
    record["predicate"]["files"] = json!(
        files
            .iter()
            .map(|(f, h, r)| json!({"path": f, "sha256": h, "rows": r}))
            .collect::<Vec<_>>()
    );
    fs::write(
        out.join("record.json"),
        serde_json::to_string_pretty(&record)?,
    )?;
    fs::write(
        out.join("README.md"),
        card(run, &record, &t, ext, &files, hz),
    )?;
    fs::write(
        out.join("croissant.json"),
        serde_json::to_string_pretty(&croissant(run, &t, ext, &files))?,
    )?;

    let mut text_out = format!(
        "{run}: {clips} clips scored, {exact} exact, {errors} failed to start; wrote {} to {}\n",
        ext,
        out.display()
    );
    for (f, _, r) in &files {
        let _ = writeln!(text_out, "  {f}: {r} rows");
    }
    text_out.push_str("  README.md, croissant.json, record.json\n");
    Ok(crate::Outcome::ok(
        json!({"run": run, "out": out, "format": ext, "files": record["predicate"]["files"],
               "record": record}),
        text_out,
    ))
}
