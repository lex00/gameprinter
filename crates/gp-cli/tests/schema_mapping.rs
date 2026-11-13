//! `.taplo.toml` points each kind of TOML file at its JSON Schema for
//! typed editing. This checks the mapping: every schema it names exists,
//! and every file it covers validates against its schema, in this repo
//! (templates/ and games/) and in the games checkout beside it
//! ($GP_GAMES_DIR, or `games` beside the repo), which is skipped when
//! there is none.

use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The games checkout, when this machine has one.
fn games() -> Option<PathBuf> {
    let dir = std::env::var_os("GP_GAMES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo().join("../games"));
    dir.join("donkey-kong").is_dir().then_some(dir)
}

fn toml_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name();
        let name = name.to_string_lossy();
        if p.is_dir() {
            if !matches!(name.as_ref(), "target" | "node_modules" | ".git" | "ext") {
                toml_files(&p, out);
            }
        } else if name.ends_with(".toml") {
            out.push(p);
        }
    }
}

/// The files `.taplo.toml`'s patterns cover. Its patterns are
/// `**/NAME` and `**/DIR/NAME`.
fn matches(pattern: &str, file: &Path) -> bool {
    let tail = pattern.trim_start_matches("**/");
    let f = file.to_string_lossy();
    f.ends_with(&format!("/{tail}"))
}

fn files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    toml_files(&repo().join("templates"), &mut files);
    toml_files(&repo().join("games"), &mut files);
    if let Some(g) = games() {
        toml_files(&g, &mut files);
    }
    files
}

fn rules() -> Vec<(String, Vec<String>)> {
    let config: toml::Value =
        toml::from_str(&std::fs::read_to_string(repo().join(".taplo.toml")).unwrap()).unwrap();
    config["rule"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["schema"]["path"].as_str().unwrap().to_owned(),
                r["include"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| p.as_str().unwrap().to_owned())
                    .collect(),
            )
        })
        .collect()
}

#[test]
fn every_file_fits_its_schema() {
    let root = repo();
    let files = files();
    let mut checked = 0;
    for (schema_rel, patterns) in rules() {
        let schema_text = std::fs::read_to_string(root.join(&schema_rel))
            .unwrap_or_else(|_| panic!("{schema_rel} is named in .taplo.toml but missing"));
        let schema: serde_json::Value = serde_json::from_str(&schema_text).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        for f in &files {
            if !patterns.iter().any(|p| matches(p, f)) {
                continue;
            }
            let value: toml::Value = toml::from_str(&std::fs::read_to_string(f).unwrap())
                .unwrap_or_else(|e| panic!("{}: {e}", f.display()));
            let json = serde_json::to_value(&value).unwrap();
            let errors: Vec<String> = validator
                .iter_errors(&json)
                .take(5)
                .map(|e| format!("  {} at {}", e, e.instance_path()))
                .collect();
            assert!(
                errors.is_empty(),
                "{} doesn't fit {schema_rel}:\n{}",
                f.display(),
                errors.join("\n")
            );
            checked += 1;
        }
    }
    // Moss Hop's template alone has a model, contract, mapping, profiles,
    // three themes and a stack.
    assert!(checked >= 8, "only {checked} files matched");
}

/// Each rule's files are found: a rule that matches nothing is a typo.
/// The games checkout's kinds (package, roms, corpus) are checked only when
/// it is here; clips.toml and slots.toml have no file in either yet.
#[test]
fn every_rule_matches_a_file() {
    let files = files();
    let have_games = games().is_some();
    for (schema, patterns) in rules() {
        let games_only = ["corpus.json", "package.json", "roms.json"]
            .iter()
            .any(|s| schema.ends_with(s));
        let none_yet = ["clips.json", "slots.json"]
            .iter()
            .any(|s| schema.ends_with(s));
        if none_yet || (games_only && !have_games) {
            continue;
        }
        assert!(
            files.iter().any(|f| patterns.iter().any(|p| matches(p, f))),
            "{patterns:?} matches nothing"
        );
    }
}

/// The merged model schema has teeth: an unknown component is refused, and
/// Moss Hop's model is accepted.
#[test]
fn the_model_schema_refuses_typos() {
    let text = std::fs::read_to_string(repo().join("schema/model-platformer.json")).unwrap();
    let validator = jsonschema::validator_for(&serde_json::from_str(&text).unwrap()).unwrap();
    let model: toml::Value = toml::from_str(
        &std::fs::read_to_string(repo().join("templates/mosshop/model.toml")).unwrap(),
    )
    .unwrap();
    let mut json = serde_json::to_value(&model).unwrap();
    assert!(validator.is_valid(&json));
    let entity = json["entities"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .next()
        .unwrap();
    entity["jumpp"] = serde_json::json!({});
    assert!(!validator.is_valid(&json));
}
