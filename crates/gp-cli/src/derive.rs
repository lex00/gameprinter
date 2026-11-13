//! `gp model derive`: derives a model's hand-written facts from what their
//! evidence cites (docs/derivation.md). The matching is `gp-derive`; this
//! module loads the sources the refs name (the repo's files), the lifted
//! tables through the asset map, reruns the generator scripts the evidence
//! names over the lifted assets, measures recordings through `gp model
//! propose`'s code, merges it all per fact, scores it and asks where a
//! derived value differs from the model's.
//!
//! Ported from game-printer's derive.rs. Its pinned downloads
//! (game-printer's pinned.rs) were other games' disassemblies (Super Mario
//! Bros., Mega Man 2, Super Mario Bros. 2), cut with those games, so a ref
//! `in` a pinned download is reported as not read. A model in a game's
//! package (a folder with package.toml) reads its refs from the package,
//! where game-printer read them from the repo. The run recorder's metrics
//! (`--metrics`) are gone with the recorder.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::json;

use gp_assets::AssetMap;
use gp_derive::asm::Source;
use gp_derive::derive::{Derived, From, Inputs, Score, Status, derive, merge, number};
use gp_extract::questions::{Candidate, Question};
use gp_model::validate::file_ref;
use gp_model::{MODEL_FILE, Model};

use crate::Outcome;

/// What `gp model derive` takes.
pub(crate) struct DeriveArgs<'a> {
    pub dir: &'a Path,
    pub from: &'a [String],
    pub assets: Option<&'a Path>,
    pub recordings: &'a [String],
    pub frames: &'a str,
    pub root: Option<&'a Path>,
    pub questions: bool,
    pub list: bool,
}

/// Source files read in full, so constants defined in one resolve in
/// another: 6502 source, and Z80 source for an arcade game (gp-derive's
/// asm reads either, by the file's mnemonics). Other cited files (TOML, Markdown) are read for their cited
/// lines only.
fn is_asm(file: &str) -> bool {
    let lower = file.to_ascii_lowercase();
    [
        ".s", ".asm", ".inc", ".a65", ".65s", ".z80", ".s80", ".z80s",
    ]
    .iter()
    .any(|e| lower.ends_with(e))
}

/// The sources every ref of the model names, by pin (`None`: the repo).
/// gameprinter has no pinned downloads, so a ref `in` one is missing.
fn load_sources(model: &Model, repo: &Path) -> (BTreeMap<Option<String>, Source>, Vec<String>) {
    let mut out: BTreeMap<Option<String>, Source> = BTreeMap::new();
    let mut loaded: BTreeSet<(Option<String>, String)> = BTreeSet::new();
    let mut missing = BTreeSet::new();
    for ev in model.evidence.values() {
        for r in &ev.refs {
            let Some((file, _)) = file_ref(&r.at) else {
                continue;
            };
            if let Some(w) = &r.within {
                missing.insert(w.clone());
                continue;
            }
            let key: Option<String> = None;
            if loaded.contains(&(key.clone(), file.to_owned())) {
                continue;
            }
            let dir = repo.to_path_buf();
            let src = out.entry(key.clone()).or_default();
            // A pin's whole file list, so its constants are all there; a
            // repo file with the assembly files beside it.
            let mut files: Vec<String> = vec![file.to_owned()];
            if is_asm(file)
                && let Some(parent) = Path::new(file).parent()
                && let Ok(rd) = fs::read_dir(dir.join(parent))
            {
                for e in rd.flatten() {
                    let n = e.file_name().to_string_lossy().into_owned();
                    if is_asm(&n) {
                        files.push(parent.join(n).to_string_lossy().into_owned());
                    }
                }
            }
            for f in files {
                if !loaded.insert((key.clone(), f.clone())) {
                    continue;
                }
                if let Ok(text) = fs::read_to_string(dir.join(&f)) {
                    if is_asm(&f) {
                        src.add(&f, &text);
                    } else {
                        src.add_lines_only(&f, &text);
                    }
                }
            }
        }
    }
    let missing = missing.into_iter().collect();
    (out, missing)
}

/// One lifted asset's numbers as rows: a `rows` table, each item of a list
/// of tables, or a list of numbers.
fn lifted_rows(v: &toml::Value) -> Vec<Vec<Option<i64>>> {
    let mut out = Vec::new();
    let Some(t) = v.as_table() else {
        return out;
    };
    let ints = |v: &toml::Value| -> Vec<Option<i64>> {
        match v {
            toml::Value::Integer(i) => vec![Some(*i)],
            toml::Value::Array(a) => a.iter().map(toml::Value::as_integer).collect(),
            _ => Vec::new(),
        }
    };
    for (k, v) in t {
        if k == "schema" {
            continue;
        }
        let Some(items) = v.as_array() else { continue };
        for item in items {
            match item {
                toml::Value::Array(_) => out.push(ints(item)),
                toml::Value::Table(row) => {
                    let r: Vec<Option<i64>> = row.values().flat_map(ints).collect();
                    if !r.is_empty() {
                        out.push(r);
                    }
                }
                toml::Value::Integer(_) => {}
                _ => {}
            }
        }
        if items.iter().all(toml::Value::is_integer) && !items.is_empty() {
            out.push(ints(v));
        }
    }
    out
}

/// A TOML document as a value.
fn doc(text: &str) -> Result<toml::Value, String> {
    text.parse::<toml::Table>()
        .map(toml::Value::Table)
        .map_err(|e| e.to_string())
}

/// Lifted tables by disassembly label, as rows.
type Tables = BTreeMap<String, Vec<Vec<Option<i64>>>>;

/// The lifted tables of `dir` by their disassembly label, through the
/// asset map it was lifted with: each `[[asset]]`'s `label` (and each
/// column's, for a table of columns) names the file `<name>.toml`.
fn load_tables(dir: &Path) -> Result<(AssetMap, Tables)> {
    let map_file = dir.join(gp_assets::MAP_FILE);
    let text =
        fs::read_to_string(&map_file).with_context(|| format!("reading {}", map_file.display()))?;
    let map = AssetMap::parse(&text).with_context(|| map_file.display().to_string())?;
    let raw = doc(&text)
        .map_err(anyhow::Error::msg)
        .with_context(|| map_file.display().to_string())?;
    let mut out = BTreeMap::new();
    for a in raw
        .get("asset")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
    {
        let (Some(name), Some(label)) = (
            a.get("name").and_then(toml::Value::as_str),
            a.get("label").and_then(toml::Value::as_str),
        ) else {
            continue;
        };
        let Ok(t) = fs::read_to_string(dir.join(format!("{name}.toml"))) else {
            continue;
        };
        let Ok(v) = doc(&t) else {
            continue;
        };
        let rows = lifted_rows(&v);
        if !rows.is_empty() {
            out.insert(label.to_owned(), rows);
        }
    }
    Ok((map, out))
}

fn agree(path: &str, value: &str, from: From, confidence: u8, at: String) -> Derived {
    Derived {
        path: path.to_owned(),
        value: Some(value.to_owned()),
        status: Status::Agree,
        from: Some(from),
        also: vec![from],
        proposed: None,
        encoding: None,
        confidence,
        refs: vec![at],
        could: None,
        reason: None,
        against: Vec::new(),
        source: String::new(),
        class: String::new(),
    }
}

/// The asset slots against the asset map: the source, the asset and the
/// kind each slot names.
fn slots(model: &Model, map: &AssetMap, map_at: &str) -> Vec<Derived> {
    let mut out = Vec::new();
    for (name, slot) in &model.assets {
        let Some(from) = slot.from.as_ref().filter(|f| f.source == map.source) else {
            continue;
        };
        let Some(a) = map.asset(&from.asset) else {
            continue;
        };
        let at = format!("{map_at} ({})", a.name());
        out.push(agree(
            &format!("assets.{name}.from.source"),
            &from.source,
            From::Data,
            100,
            at.clone(),
        ));
        out.push(agree(
            &format!("assets.{name}.from.asset"),
            &from.asset,
            From::Data,
            100,
            at.clone(),
        ));
        if a.format() == slot.kind.map_format() {
            out.push(agree(
                &format!("assets.{name}.kind"),
                slot.kind.name(),
                From::Data,
                90,
                at,
            ));
        }
    }
    out
}

/// The generator scripts the evidence names (`scripts/model/*.py`), rerun
/// over the lifted assets and compared with the model, fact by fact.
fn generators(
    model: &Model,
    tree: &toml::Value,
    repo: &Path,
    assets: &Path,
    notes: &mut Vec<String>,
) -> Vec<Derived> {
    let mut scripts = BTreeSet::new();
    for ev in model.evidence.values() {
        let texts = ev
            .refs
            .iter()
            .map(|r| r.at.as_str())
            .chain(ev.note.as_deref());
        for t in texts {
            for (i, _) in t.match_indices("scripts/model/") {
                let rest = &t[i..];
                if let Some(end) = rest.find(".py") {
                    scripts.insert(rest[..end + 3].to_owned());
                }
            }
        }
    }
    let facts: BTreeMap<String, &toml::Value> = gp_model::facts(tree)
        .into_iter()
        .map(|(p, v)| (gp_model::path::display(&p), v))
        .collect();
    let mut out = Vec::new();
    for s in scripts {
        let path = repo.join(&s);
        if !path.is_file() {
            continue;
        }
        let run = std::process::Command::new("python3")
            .args(["-I", "-B"])
            .arg(&path)
            .arg(assets)
            .output();
        let text = match run {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
            Ok(o) => {
                notes.push(format!(
                    "{s} failed: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                ));
                continue;
            }
            Err(e) => {
                notes.push(format!("{s} couldn't run: {e}"));
                continue;
            }
        };
        let Ok(written) = doc(&text) else {
            notes.push(format!("{s} wrote something that isn't TOML"));
            continue;
        };
        notes.push(format!("reran {s}"));
        for (p, v) in gp_model::facts(&written) {
            let p = gp_model::path::display(&p);
            let at = format!("{s} over {}", assets.display());
            match facts.get(&p) {
                Some(m) if same(m, v) => out.push(agree(&p, &text_of(m), From::Data, 85, at)),
                Some(m) => {
                    let mut d = agree(&p, &text_of(m), From::Data, 70, at);
                    d.status = Status::Differ;
                    d.proposed = Some(text_of(v));
                    out.push(d);
                }
                None => {
                    let mut d = agree(&p, "", From::Data, 70, at);
                    d.value = None;
                    d.status = Status::New;
                    d.proposed = Some(text_of(v));
                    out.push(d);
                }
            }
        }
    }
    out
}

fn text_of(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => s.clone(),
        o => o.to_string(),
    }
}

/// Two values the same, numbers compared as fixed-point.
fn same(a: &toml::Value, b: &toml::Value) -> bool {
    match (number(a), number(b)) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

/// `gp model propose`'s measurements as derivations.
fn recordings(
    args: &DeriveArgs<'_>,
    model: &Model,
    paths: &gp_evidence::Paths,
) -> Result<Vec<Derived>> {
    let m =
        crate::refine::recording::measure(args.dir, model, args.recordings, args.frames, paths)?;
    Ok(m.proposals
        .iter()
        .map(|p| {
            let at = format!("recording:{}", p.recordings.join(","));
            let mut d = agree(
                &p.path,
                p.stated.as_deref().unwrap_or(""),
                From::Recordings,
                p.confidence,
                at,
            );
            match p.agrees {
                Some(true) => {}
                Some(false) => {
                    d.status = Status::Differ;
                    d.proposed = Some(p.measured.clone());
                }
                None => {
                    d.value = None;
                    d.status = Status::New;
                    d.proposed = Some(p.measured.clone());
                }
            }
            d.encoding = Some(p.unit.to_owned());
            d
        })
        .collect())
}

fn parse_from(list: &[String]) -> Result<BTreeSet<From>> {
    let mut out = BTreeSet::new();
    for f in list {
        for f in f.split(',') {
            if f == "all" {
                out.extend([
                    From::Evidence,
                    From::Code,
                    From::Data,
                    From::Recordings,
                    From::Derived,
                ]);
            } else {
                out.insert(From::parse(f).with_context(|| {
                    format!("--from {f}: one of evidence, code, data, recordings, all")
                })?);
            }
        }
    }
    // Values built from others come with whichever reads them.
    out.insert(From::Derived);
    Ok(out)
}

pub(crate) fn run(args: &DeriveArgs<'_>, paths: &gp_evidence::Paths) -> Result<Outcome> {
    if !args.dir.is_dir() {
        bail!("{} isn't a directory", args.dir.display());
    }
    let file = args.dir.join(MODEL_FILE);
    let text = fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
    let model = Model::parse(&text).with_context(|| file.display().to_string())?;
    let from = parse_from(args.from)?;
    let repo: PathBuf = match args.root {
        Some(r) => r.to_path_buf(),
        // A game's package is its own root, as game-printer's repo was.
        None if args.dir.join("package.toml").is_file() => args.dir.to_path_buf(),
        None => gp_evidence::paths::enclosing_git_work_tree(args.dir)
            .unwrap_or_else(|| args.dir.to_path_buf()),
    };
    let mut notes = Vec::new();
    let (sources, missing) = load_sources(&model, &repo);
    if !missing.is_empty() {
        notes.push(format!(
            "pinned downloads gp has none of, so refs into them only read lifted tables: {}",
            missing.join(", ")
        ));
    }
    let lifted = match args.assets.filter(|_| from.contains(&From::Data)) {
        Some(d) => Some(load_tables(d)?),
        None => None,
    };
    let empty = BTreeMap::new();
    let mut all = derive(
        &model,
        &Inputs {
            sources: &sources,
            tables: lifted.as_ref().map_or(&empty, |(_, t)| t),
            from: from.clone(),
        },
    );
    if from.contains(&From::Data) {
        // The repo's asset map, or the lifted directory's.
        let map_file = repo.join("assets").join(format!(
            "{}.toml",
            model.model.source.as_deref().unwrap_or("")
        ));
        let map = match &lifted {
            Some((m, _)) => Some((m.clone(), args.assets.map(|d| d.join(gp_assets::MAP_FILE)))),
            None => fs::read_to_string(&map_file)
                .ok()
                .and_then(|t| AssetMap::parse(&t).ok())
                .map(|m| (m, Some(map_file.clone()))),
        };
        if let Some((m, at)) = map {
            let at = at.map_or_else(String::new, |p| {
                p.strip_prefix(&repo).unwrap_or(&p).display().to_string()
            });
            merge(&mut all, slots(&model, &m, &at));
        }
        if let Some(d) = args.assets {
            let rerun = generators(&model, &model.to_tree(), &repo, d, &mut notes);
            merge(&mut all, rerun);
        }
    }
    if from.contains(&From::Recordings) && !args.recordings.is_empty() {
        let r = recordings(args, &model, paths)?;
        merge(&mut all, r);
    }
    let score = Score::of(&all);

    let name = &model.model.name;
    let mut asked = 0;
    if args.questions {
        // The refs each fact's covering evidence entry cites: what the
        // deriver read, kept with either answer (docs/derivation.md).
        let covering: BTreeMap<String, Vec<gp_model::model::Ref>> =
            gp_model::validate::evidence_for(&model)
                .into_iter()
                .filter_map(|(f, key)| {
                    let refs = model.evidence.get(key.as_ref()?)?.refs.clone();
                    Some((gp_model::path::display(&f), refs))
                })
                .collect();
        let mut qs = crate::refine::read_questions(args.dir)?;
        for d in all.iter().filter(|d| d.status == Status::Differ) {
            let (Some(have), Some(got)) = (&d.value, &d.proposed) else {
                continue;
            };
            asked += usize::from(qs.ask(Question {
                id: format!("derive:{}", d.path),
                kind: "disagree".into(),
                path: d.path.clone(),
                text: format!(
                    "{} is {have} in the model, and {} reads {got}{}. Keep the model's or take the derived value?",
                    d.path,
                    d.refs.join(", "),
                    d.encoding
                        .as_deref()
                        .map_or(String::new(), |e| format!(" ({e})"))
                ),
                settle: "read the cited line and the code that uses it".into(),
                candidates: vec![
                    Candidate {
                        value: have.clone(),
                        from: "the model".into(),
                        refs: covering.get(&d.path).cloned().unwrap_or_default(),
                    },
                    Candidate {
                        value: got.clone(),
                        from: format!(
                            "gp model derive --from {}",
                            d.from.map_or("all", From::name)
                        ),
                        refs: covering.get(&d.path).cloned().unwrap_or_default(),
                    },
                ],
                answer: None,
            }));
        }
        let file = args.dir.join(gp_extract::questions::QUESTIONS_FILE);
        fs::write(&file, qs.to_toml()).with_context(|| format!("writing {}", file.display()))?;
    }

    let mut t = format!(
        "{name}: {} facts, {} hand-written: {} derived ({:.1}%), {} agree ({:.1}% of derived), {} differ, {} new, {} agreed but contradicted\n",
        score.facts,
        score.hand_written,
        score.derived,
        100.0 * score.derivable_share(),
        score.agree,
        100.0 * score.agree_share(),
        score.differ,
        score.new,
        score.contradicted
    );
    let by: Vec<String> = score
        .by_from
        .iter()
        .map(|(k, v)| format!("{k} {v}"))
        .collect();
    let _ = writeln!(t, "agree by deriver: {}", by.join(", "));
    let could: Vec<String> = score
        .could
        .iter()
        .map(|(k, v)| format!("({k}) {v}"))
        .collect();
    let _ = writeln!(
        t,
        "underivable by the class that could derive them: {}",
        could.join(", ")
    );
    for d in &all {
        let show =
            args.list || matches!(d.status, Status::Differ | Status::New) || !d.against.is_empty();
        if !show {
            continue;
        }
        let _ = writeln!(
            t,
            "  {:<11} {:<52} {:>12} {}{}",
            d.status.name(),
            d.path,
            d.value.as_deref().unwrap_or("-"),
            match (&d.proposed, d.status) {
                (Some(p), _) => format!("-> {p} "),
                _ => String::new(),
            },
            match (d.status, &d.reason) {
                (Status::Underivable, Some(r)) => format!("({}) {r}", d.could.unwrap_or('e')),
                _ => format!(
                    "{} {}% {}{}",
                    d.from.map_or("", From::name),
                    d.confidence,
                    d.refs.join(", "),
                    d.encoding
                        .as_deref()
                        .map_or(String::new(), |e| format!(" as {e}"))
                ),
            }
        );
        for (f, p) in &d.against {
            let _ = writeln!(t, "              but {} gives {p}", f.name());
        }
    }
    for n in &notes {
        let _ = writeln!(t, "note: {n}");
    }
    if args.questions {
        let _ = writeln!(
            t,
            "{asked} new questions in {}",
            args.dir
                .join(gp_extract::questions::QUESTIONS_FILE)
                .display()
        );
    }
    Ok(Outcome::ok(
        json!({
            "model": name,
            "score": score,
            "derivable_share": score.derivable_share(),
            "agree_share": score.agree_share(),
            "facts": all,
            "notes": notes,
        }),
        t.trim_end().to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifted_files_read_as_rows() {
        // Made-up lifted files: rows, a list of tables, a list of numbers.
        let v = doc("schema = 1\nasset = \"x\"\nrows = [[1, 2], [3, 4]]\n").unwrap();
        assert_eq!(
            lifted_rows(&v),
            vec![vec![Some(1), Some(2)], vec![Some(3), Some(4)]]
        );
        let v = doc("schema = 1\n[[row]]\nspeed = 5\nforce = [6, 7]\n").unwrap();
        assert_eq!(lifted_rows(&v), vec![vec![Some(6), Some(7), Some(5)]]);
        let v = doc("schema = 1\nstarts = [9, 8]\n").unwrap();
        assert_eq!(lifted_rows(&v), vec![vec![Some(9), Some(8)]]);
    }

    #[test]
    fn moss_hops_facts_come_back_from_its_source() {
        // The open tier's game: its model cites games/mosshop/mosshop.s.
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let text = fs::read_to_string(repo.join("templates/mosshop/model.toml")).unwrap();
        let model = Model::parse(&text).unwrap();
        let (sources, _) = load_sources(&model, &repo);
        let all = derive(
            &model,
            &Inputs {
                sources: &sources,
                tables: &BTreeMap::new(),
                from: parse_from(&["evidence,code".to_owned()]).unwrap(),
            },
        );
        let at = |p: &str| all.iter().find(|d| d.path == p).unwrap();
        assert_eq!(at("levels.main.kill_below").status, Status::Agree);
        assert_eq!(at("entities.player.jump.gravity").status, Status::Agree);
        assert_eq!(
            at("entities.player.jump.gravity").encoding.as_deref(),
            Some("256ths")
        );
        let s = Score::of(&all);
        assert!(s.agree >= 15, "{s:?}");
        assert_eq!(s.differ, 0);
    }
}
