//! JSON Schemas generated from the Rust types, for tools outside the
//! workspace (the editor, agents). `gp model schema` prints them and
//! `just docs` writes them to schema/, where a test keeps them current.
//!
//! The types are the definition: parsing goes through them, and the JSON
//! Schema can only say what they say. Some checks are beyond JSON Schema
//! (references between parts, evidence for every fact), and
//! [`crate::validate`] does those.

use crate::model::{CORE_VERSION, Model};
use crate::platformer;
use crate::topdown;

fn finish(mut schema: schemars::Schema, id: &str) -> String {
    schema.insert("$id".into(), id.into());
    let mut text = serde_json::to_string_pretty(&schema).expect("schemas serialize");
    text.push('\n');
    text
}

/// The model core schema.
pub fn core() -> String {
    finish(
        schemars::schema_for!(Model),
        &format!("game-printer:model-core@{CORE_VERSION}"),
    )
}

/// The platformer extension's schema: the components it adds to entities
/// and the fields it adds to levels.
pub fn platformer() -> String {
    finish(
        schemars::schema_for!(platformer::Schema),
        &format!("game-printer:platformer@{}", platformer::VERSION),
    )
}

/// The top-down extension's schema: the components it adds to entities
/// and the fields it adds to levels.
pub fn topdown() -> String {
    finish(
        schemars::schema_for!(topdown::Schema),
        &format!("game-printer:topdown@{}", topdown::VERSION),
    )
}

/// The schema of a `model.toml` that uses the platformer extension: the
/// core with the extension's entity components and level fields merged
/// into `Entity` and `Level`, so an editor checks a whole model file with
/// one schema. For editors only (`.taplo.toml`); `core` and `platformer`
/// stay the pieces' own schemas.
pub fn model_platformer() -> String {
    let mut core: serde_json::Value =
        serde_json::from_str(&core()).expect("the core schema is JSON");
    let ext: serde_json::Value =
        serde_json::from_str(&platformer()).expect("the platformer schema is JSON");
    let ext_defs = ext["$defs"].as_object().expect("the extension has $defs");
    let defs = core["$defs"].as_object_mut().expect("the core has $defs");
    for (name, def) in ext_defs {
        if name != "EntityComponents" && name != "LevelFields" {
            defs.entry(name.clone()).or_insert_with(|| def.clone());
        }
    }
    for (into, from) in [("Entity", "EntityComponents"), ("Level", "LevelFields")] {
        let add = ext_defs[from]["properties"]
            .as_object()
            .expect("components have properties")
            .clone();
        let target = defs
            .get_mut(into)
            .and_then(|d| d["properties"].as_object_mut())
            .expect("the core type has properties");
        target.extend(add);
        defs.get_mut(into)
            .and_then(serde_json::Value::as_object_mut)
            .expect("an object")
            .insert("additionalProperties".into(), false.into());
    }
    core["$id"] = format!(
        "game-printer:model-platformer@{CORE_VERSION}+{}",
        platformer::VERSION
    )
    .into();
    core["title"] = "Model with the platformer extension".into();
    let mut text = serde_json::to_string_pretty(&core).expect("schemas serialize");
    text.push('\n');
    text
}

/// A template's physics profiles, `profiles.toml` (docs/formats/profiles.md).
pub fn profiles() -> String {
    finish(
        schemars::schema_for!(crate::profile::Profiles),
        &format!("game-printer:profiles@{}", crate::profile::FORMAT),
    )
}

/// The file name each schema has in schema/: one file per format, with
/// no version in its name (the `$id` inside names the version).
pub fn file_name(which: &str) -> String {
    format!("{which}.json")
}
