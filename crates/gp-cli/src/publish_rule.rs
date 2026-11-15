//! The rule for publishing an arcade game (the owner's decisions on #3):
//! `gp registry publish` takes one only when every clip of its corpus
//! scores exact on state and every screen frame each clip compares is the
//! same on picture (the record's per-clip `screen` counts: same equal to
//! compared, and more than none compared). It doesn't score: it reads the score record `gp corpus
//! score --write` keeps in the package's own `evidence/`, the one for the
//! package's model and corpus as they are (`evidence/MODEL/CORPUS.json`),
//! and checks that the record's mapping, contract and ROM are the
//! package's too. A record kept in the package is committed with it, so
//! the pass travels with the package's commit and into the registry.
//!
//! A game with hooks (package.toml's `[hooks]`) must have been scored
//! through the module it ships: the record's `module.hash` (record format
//! 0.2.0 on) is play/module.wasm's. A record from before records named the
//! module is refused for such a game and taken for one without hooks.

use std::fs;
use std::path::Path;

use anyhow::{Result, anyhow};
use serde_json::Value;

use crate::corpus_score::{Picture, model_key, record_path};

/// Whether the package folder is an arcade game: its package.toml names
/// an arcade romset, its template's source is an arcade board, or its
/// play/page.json plays it from a romset zip.
pub(crate) fn is_arcade_game(dir: &Path) -> bool {
    let toml_of =
        |f: &str| -> Option<toml::Table> { fs::read_to_string(dir.join(f)).ok()?.parse().ok() };
    let arcade = |v: Option<&toml::Value>| v.and_then(toml::Value::as_str) == Some("arcade");
    let entries = toml_of("package.toml")
        .and_then(|t| t.get("rom")?.get("entry")?.as_array().cloned())
        .unwrap_or_default();
    if entries.iter().any(|e| arcade(e.get("platform"))) {
        return true;
    }
    if toml_of("template.toml")
        .is_some_and(|t| arcade(t.get("source").and_then(|s| s.get("platform"))))
    {
        return true;
    }
    let page: Value = fs::read(dir.join("play").join("page.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    page["player"] == "arcade" || page["rom"]["kind"] == "zip"
}

/// What a person does to get an arcade game published.
const HOW: &str = "Score it in the game's folder with `gp corpus score GAME_DIR --run publish --write GAME_DIR` (for a game with hooks, add `--module PLAY_DIR/module.wasm`, the module the package ships, which build-page.sh or `gp game build` makes) until every clip is exact on state and every screen frame is the same on picture. That writes the record into GAME_DIR/evidence/; commit it with the game, and publish the folder again";

/// Stops the publish of an arcade game (`kind` game) unless its
/// evidence/ holds a score record for its model and corpus as they are,
/// with every clip exact on state, none that couldn't be scored, every
/// clip's compared screen frames the same on picture, and the record's
/// mapping, contract and ROM the package's. Returns a line saying what it
/// read, or None for a package that isn't an arcade game.
pub(crate) fn arcade_exact_guard(dir: &Path, kind: &str) -> Result<Option<String>> {
    if kind != "game" || !is_arcade_game(dir) {
        return Ok(None);
    }
    let what = package_name(dir);
    let stop = |why: String| {
        anyhow!(
            "{what} is an arcade game, and an arcade game is published only when every clip of its corpus scores exact on state and every screen frame is the same on picture, but {why}. {HOW}. Nothing was published"
        )
    };
    let model = fs::read_to_string(dir.join(gp_model::MODEL_FILE))
        .map_err(|_| stop(format!("it has no {} to score", gp_model::MODEL_FILE)))?;
    let corpus = crate::corpus_score::load(dir)
        .map_err(|e| stop(format!("its corpus can't be read ({e:#})")))?;
    let clips = corpus.set.clips.len();
    if clips == 0 {
        return Err(stop("its corpus has no clips".into()));
    }
    let model_hash = model_key(&model);
    let file = record_path(dir, &model_hash, &corpus.hash);
    let rel = file
        .strip_prefix(dir)
        .unwrap_or(&file)
        .display()
        .to_string();
    let Ok(bytes) = fs::read(&file) else {
        let others = crate::corpus_w9::records(dir).len();
        let since = match others {
            0 => String::new(),
            1 => {
                " (its one record is for another model or corpus: one of them changed since)".into()
            }
            n => format!(" (its {n} records are for other models or corpora: they changed since)"),
        };
        return Err(stop(format!(
            "it holds no score record for its model and corpus as they are ({rel} isn't there){since}"
        )));
    };
    let v: Value = serde_json::from_slice(&bytes)
        .map_err(|e| stop(format!("its score record {rel} isn't JSON ({e})")))?;
    if v["predicateType"] != crate::corpus_score::PREDICATE {
        return Err(stop(format!("{rel} isn't a score record")));
    }
    let p = &v["predicate"];
    let r = &p["results"];
    let (n, exact, errors) = (
        r["clips"].as_u64().unwrap_or(0),
        r["exact"].as_u64().unwrap_or(0),
        r["errors"].as_u64().unwrap_or(0),
    );
    if n != clips as u64 {
        return Err(stop(format!(
            "its score record {rel} scored {n} clips and its corpus has {clips}"
        )));
    }
    let rows = p["clips"].as_array().cloned().unwrap_or_default();
    // The picture apart from the state: each clip's compared screen frames.
    let pictures: Vec<(&str, Picture)> = rows
        .iter()
        .filter(|c| c.get("error").is_none_or(Value::is_null))
        .map(|c| (c["id"].as_str().unwrap_or("?"), Picture::of(c)))
        .collect();
    let (same, frames) = pictures
        .iter()
        .fold((0, 0), |(s, f), (_, pc)| (s + pc.same, f + pc.frames));
    let picture = format!("picture: {same} of {frames} screen frames the same");
    let named = |ids: Vec<&str>, what: &str| {
        if ids.is_empty() {
            return String::new();
        }
        let more = if ids.len() > 5 { ", and more" } else { "" };
        let shown: Vec<&str> = ids.into_iter().take(5).collect();
        format!(" ({what}: {}{more})", shown.join(", "))
    };
    if exact != n || errors != 0 {
        let not = rows
            .iter()
            .filter(|c| c["exact"] != true)
            .filter_map(|c| c["id"].as_str())
            .collect::<Vec<_>>();
        return Err(stop(format!(
            "its score record {rel} has {exact} of {n} clips exact on state and {errors} that couldn't be scored{}; {picture}",
            named(not, "not exact")
        )));
    }
    let off: Vec<&str> = pictures
        .iter()
        .filter(|(_, pc)| !pc.exact())
        .map(|(id, _)| *id)
        .collect();
    if rows.len() as u64 != n {
        return Err(stop(format!(
            "its score record {rel} has {} clip rows for its {n} clips",
            rows.len()
        )));
    }
    if !off.is_empty() {
        let none = pictures.iter().filter(|(_, pc)| pc.frames == 0).count();
        let unseen = if none > 0 {
            format!(", {none} of them with no screen frames compared")
        } else {
            String::new()
        };
        return Err(stop(format!(
            "its score record {rel} has {n} of {n} clips exact on state; {picture}, {} of {n} clips differ on picture{unseen}{}",
            off.len(),
            named(off.clone(), "differ on picture")
        )));
    }
    let h = |b: &[u8]| gp_evidence::Hash::of(b).to_string();
    let mapping = fs::read(dir.join("mapping.toml")).unwrap_or_default();
    if p["mapping"]["hash"] != h(&mapping).as_str() {
        return Err(stop(format!(
            "its mapping.toml isn't the one its score record {rel} was scored with"
        )));
    }
    let contract = fs::read(dir.join(gp_contract::CONTRACT_FILE)).unwrap_or_default();
    if p["contract"] != h(&contract).as_str() {
        return Err(stop(format!(
            "its {} isn't the one its score record {rel} was scored with",
            gp_contract::CONTRACT_FILE
        )));
    }
    if p["corpus"]["hash"] != corpus.hash.as_str() || p["model"]["hash"] != model_hash.as_str() {
        return Err(stop(format!(
            "its score record {rel} names another model or corpus than its folder"
        )));
    }
    if let Some(why) = module_differs(dir, p) {
        return Err(stop(format!("its score record {rel} {why}")));
    }
    let scored = scored_rom(dir);
    if let Some(sha) = &scored
        && p["rom"]["sha256"]
            .as_str()
            .is_none_or(|s| !s.eq_ignore_ascii_case(sha))
    {
        return Err(stop(format!(
            "its score record {rel} is for ROM {}, and its package.toml's scored romset is {sha}",
            p["rom"]["sha256"].as_str().unwrap_or("none")
        )));
    }
    Ok(Some(format!(
        "{what} is an arcade game: all {n} clips of its corpus {} {} score exact on state and all {frames} of their screen frames are the same on picture, on its model as it is (score record {rel}, by {})",
        p["corpus"]["name"].as_str().unwrap_or("?"),
        p["corpus"]["version"].as_str().unwrap_or("?"),
        p["gp"]["build"]
            .as_str()
            .or(p["gp"]["version"].as_str())
            .unwrap_or("gp")
    )))
}

fn package_name(dir: &Path) -> String {
    fs::read_to_string(dir.join("package.toml"))
        .ok()
        .and_then(|t| t.parse::<toml::Table>().ok())
        .and_then(|t| Some(t.get("package")?.get("name")?.as_str()?.to_owned()))
        .unwrap_or_else(|| "this game".into())
}

/// The sha256 of the package's scored romset (`[[rom.entry]]` with role
/// `scored`), when it names one.
fn scored_rom(dir: &Path) -> Option<String> {
    let t: toml::Table = fs::read_to_string(dir.join("package.toml"))
        .ok()?
        .parse()
        .ok()?;
    t.get("rom")?
        .get("entry")?
        .as_array()?
        .iter()
        .find(|e| e.get("role").and_then(toml::Value::as_str) == Some("scored"))
        .and_then(|e| Some(e.get("sha256")?.as_str()?.to_owned()))
}

/// Why the record's hooks module isn't the package's, or None when it is:
/// a game with hooks needs a record naming a module and play/module.wasm
/// with that hash; a module the package ships must be the one scored.
fn module_differs(dir: &Path, p: &Value) -> Option<String> {
    let hooks = fs::read_to_string(dir.join("package.toml"))
        .ok()
        .and_then(|t| t.parse::<toml::Table>().ok())
        .is_some_and(|t| t.contains_key("hooks"));
    let recorded = p["module"]["hash"].as_str();
    let shipped = fs::read(dir.join("play").join("module.wasm"))
        .ok()
        .map(|b| gp_evidence::Hash::of(&b).to_string());
    match (hooks, recorded, shipped.as_deref()) {
        (true, None, _) if p.get("module").is_none() => Some(format!(
            "is format {}, from before score records named the hooks module, and the game has hooks: score it again",
            p["record_format"].as_str().unwrap_or("?")
        )),
        (true, None, _) => Some("was scored with no hooks module, and the game has hooks".into()),
        (true, Some(_), None) => {
            Some("names a hooks module, and the package ships none (play/module.wasm)".into())
        }
        (_, Some(r), Some(s)) if r != s => Some(format!(
            "was scored through module {r}, and the package ships {s} (play/module.wasm)"
        )),
        _ => None,
    }
}
