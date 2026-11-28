//! `.taplo.toml` points each kind of TOML file at its JSON Schema for
//! typed editing. This checks the mapping: every schema it names exists,
//! and every file it covers validates against its schema, in this repo
//! (templates/ and games/). A games checkout's files are checked from the
//! games repo, which reads no tool test.

use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
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
        let validator = Schema::new(schema);
        for f in &files {
            if !patterns.iter().any(|p| matches(p, f)) {
                continue;
            }
            let value: toml::Value = toml::from_str(&std::fs::read_to_string(f).unwrap())
                .unwrap_or_else(|e| panic!("{}: {e}", f.display()));
            let json = serde_json::to_value(&value).unwrap();
            let errors = validator.errors(&json, 5);
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
/// The games checkout's kinds (package, roms, corpus) have no file in this
/// repo, and clips.toml and slots.toml have none anywhere yet.
#[test]
fn every_rule_matches_a_file() {
    let files = files();
    for (schema, patterns) in rules() {
        let games_only = ["corpus.json", "package.json", "roms.json"]
            .iter()
            .any(|s| schema.ends_with(s));
        let none_yet = ["clips.json", "slots.json"]
            .iter()
            .any(|s| schema.ends_with(s));
        if none_yet || games_only {
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
    let validator = Schema::new(serde_json::from_str(&text).unwrap());
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

/// A mapping that breaks its schema is refused, each way the checker
/// knows, with where it breaks: Moss Hop's mapping fits, and each change
/// below makes it not fit.
#[test]
fn a_mapping_that_breaks_its_schema_is_refused() {
    let text = std::fs::read_to_string(repo().join("schema/mapping.json")).unwrap();
    let schema = Schema::new(serde_json::from_str(&text).unwrap());
    let mapping: toml::Value = toml::from_str(
        &std::fs::read_to_string(repo().join("templates/mosshop/mapping.toml")).unwrap(),
    )
    .unwrap();
    let good = serde_json::to_value(&mapping).unwrap();
    assert_eq!(schema.errors(&good, 5), Vec::<String>::new());
    type Break = fn(&mut Value);
    let cases: &[(&str, &str, Break)] = &[
        ("a key the mapping doesn't take", "/aling", |m| {
            m["aling"] = "polled".into()
        }),
        ("a required key left out", "\"model\" is required", |m| {
            m.as_object_mut().unwrap().remove("model");
        }),
        (
            "a value no oneOf case takes",
            "oneOf, not one at /align",
            |m| m["align"] = "frames".into(),
        ),
        ("a string where an array goes", "at /field/0/ram", |m| {
            m["field"][0]["ram"] = "0x10".into()
        }),
        (
            "a number past its maximum",
            "maximum 65535 at /field/1/frac",
            |m| m["field"][1]["frac"] = 70000.into(),
        ),
        ("too few items", "minItems is 2 at /field/0/bits", |m| {
            m["field"][0]["bits"] = serde_json::json!([3])
        }),
        ("a misspelt key in a field", "/field/2/sigend", |m| {
            m["field"][2]["sigend"] = true.into()
        }),
    ];
    for (what, says, change) in cases {
        let mut m = good.clone();
        change(&mut m);
        let errors = schema.errors(&m, 5).join("\n");
        assert!(
            errors.contains(says),
            "{what}: wanted {says:?}, got {errors:?}"
        );
    }
}

/// The schema check needs no JSON Schema crate: gp-cli depends on no
/// jsonschema, the lock holds none, and docs/workers.md says what the
/// workspace already has for the common jobs.
#[test]
fn the_check_needs_no_schema_crate() {
    let manifest: toml::Value =
        toml::from_str(&std::fs::read_to_string(repo().join("crates/gp-cli/Cargo.toml")).unwrap())
            .unwrap();
    for table in ["dependencies", "dev-dependencies", "build-dependencies"] {
        let has = manifest
            .get(table)
            .and_then(|t| t.get("jsonschema"))
            .is_some();
        assert!(!has, "crates/gp-cli/Cargo.toml has jsonschema in [{table}]");
    }
    let lock = std::fs::read_to_string(repo().join("Cargo.lock")).unwrap();
    assert!(
        !lock.contains("name = \"jsonschema\""),
        "Cargo.lock still holds jsonschema"
    );
    let workers = std::fs::read_to_string(repo().join("docs/workers.md")).unwrap();
    assert!(
        workers.contains("## Dependencies"),
        "docs/workers.md has no Dependencies section"
    );
}

use serde_json::Value;

/// A JSON Schema (draft 2020-12) checked against values with serde_json
/// alone, for the keywords schema/'s files use (schemars writes them):
/// `$ref` to the file's own `$defs`, `oneOf`, `anyOf`, `const`, `enum`,
/// `type`, `properties`, `additionalProperties`, `required`, `items`,
/// `prefixItems`, `minItems`, `maxItems`, `minimum`, `maximum` and
/// `pattern`. `format` (schemars' `uint8`, `double` and the like) is an
/// annotation in 2020-12, as the bounds beside it say the same, and so are
/// the rest of [`ANNOTATIONS`]. Any other keyword stops the test, so a
/// schema that starts using one fails here until the checker learns it,
/// never passing unchecked.
struct Schema {
    root: Value,
}

const ANNOTATIONS: &[&str] = &[
    "$schema",
    "$id",
    "$defs",
    "$comment",
    "title",
    "description",
    "default",
    "examples",
    "format",
    "deprecated",
    "readOnly",
    "writeOnly",
];

impl Schema {
    fn new(root: Value) -> Schema {
        Schema { root }
    }

    /// Up to `limit` of the ways `value` breaks the schema, each with
    /// where: `  MESSAGE at /PATH`.
    fn errors(&self, value: &Value, limit: usize) -> Vec<String> {
        let mut out = Vec::new();
        self.check(&self.root, value, "", &mut out);
        out.truncate(limit);
        out
    }

    fn is_valid(&self, value: &Value) -> bool {
        self.errors(value, 1).is_empty()
    }

    fn check(&self, schema: &Value, v: &Value, at: &str, out: &mut Vec<String>) {
        let s = match schema {
            Value::Bool(true) => return,
            Value::Bool(false) => {
                out.push(format!("  nothing is allowed here at {at}"));
                return;
            }
            Value::Object(s) => s,
            other => panic!("a schema is an object or a boolean, not {other}"),
        };
        for (k, x) in s {
            match k.as_str() {
                "$ref" => {
                    let r = x.as_str().expect("$ref is a string");
                    let target = r
                        .strip_prefix('#')
                        .and_then(|p| self.root.pointer(p))
                        .unwrap_or_else(|| panic!("$ref {r} isn't in this file's $defs"));
                    self.check(target, v, at, out);
                }
                "anyOf" | "oneOf" => {
                    let each = x.as_array().expect("anyOf and oneOf are arrays");
                    let fits = each
                        .iter()
                        .filter(|sub| {
                            let mut e = Vec::new();
                            self.check(sub, v, at, &mut e);
                            e.is_empty()
                        })
                        .count();
                    if k == "anyOf" && fits == 0 {
                        out.push(format!("  {v} fits none of anyOf at {at}"));
                    }
                    if k == "oneOf" && fits != 1 {
                        out.push(format!("  {v} fits {fits} of oneOf, not one at {at}"));
                    }
                }
                "const" => {
                    if !same(x, v) {
                        out.push(format!("  {v} isn't {x} at {at}"));
                    }
                }
                "enum" => {
                    let all = x.as_array().expect("enum is an array");
                    if !all.iter().any(|e| same(e, v)) {
                        out.push(format!("  {v} isn't one of {x} at {at}"));
                    }
                }
                "type" => {
                    let fits = match x {
                        Value::String(t) => is_type(t, v),
                        Value::Array(ts) => ts.iter().any(|t| is_type(t.as_str().unwrap(), v)),
                        _ => panic!("type is a string or an array"),
                    };
                    if !fits {
                        out.push(format!("  {v} isn't of type {x} at {at}"));
                    }
                }
                "properties" | "additionalProperties" | "required" => {
                    let Value::Object(o) = v else { continue };
                    let props = s.get("properties").and_then(Value::as_object);
                    match k.as_str() {
                        "properties" => {
                            for (name, sub) in props.unwrap() {
                                if let Some(val) = o.get(name) {
                                    self.check(sub, val, &format!("{at}/{name}"), out);
                                }
                            }
                        }
                        "additionalProperties" => {
                            for (name, val) in o {
                                if props.is_some_and(|p| p.contains_key(name)) {
                                    continue;
                                }
                                let path = format!("{at}/{name}");
                                if x == &Value::Bool(false) {
                                    out.push(format!(
                                        "  {name:?} isn't a key this takes at {path}"
                                    ));
                                } else {
                                    self.check(x, val, &path, out);
                                }
                            }
                        }
                        _ => {
                            for name in x.as_array().expect("required is an array") {
                                let name = name.as_str().unwrap();
                                if !o.contains_key(name) {
                                    out.push(format!("  {name:?} is required at {at}"));
                                }
                            }
                        }
                    }
                }
                "items" | "prefixItems" | "minItems" | "maxItems" => {
                    let Value::Array(a) = v else { continue };
                    let prefix = s.get("prefixItems").and_then(Value::as_array);
                    match k.as_str() {
                        "prefixItems" => {
                            for (i, (sub, val)) in prefix.unwrap().iter().zip(a).enumerate() {
                                self.check(sub, val, &format!("{at}/{i}"), out);
                            }
                        }
                        "items" => {
                            let skip = prefix.map_or(0, Vec::len);
                            for (i, val) in a.iter().enumerate().skip(skip) {
                                self.check(x, val, &format!("{at}/{i}"), out);
                            }
                        }
                        _ => {
                            let n = x.as_u64().expect("minItems and maxItems are counts") as usize;
                            if (k == "minItems" && a.len() < n) || (k == "maxItems" && a.len() > n)
                            {
                                out.push(format!("  {} items, {k} is {n} at {at}", a.len()));
                            }
                        }
                    }
                }
                "minimum" | "maximum" => {
                    let Some(n) = v.as_f64() else { continue };
                    let bound = x.as_f64().expect("a bound is a number");
                    if (k == "minimum" && n < bound) || (k == "maximum" && n > bound) {
                        out.push(format!("  {v} is past its {k} {x} at {at}"));
                    }
                }
                "pattern" => {
                    let Value::String(text) = v else { continue };
                    let p = x.as_str().expect("pattern is a string");
                    if !fits_pattern(p, text) {
                        out.push(format!("  {v} doesn't match {p} at {at}"));
                    }
                }
                k if ANNOTATIONS.contains(&k) => {}
                k => panic!(
                    "schema keyword {k} is new to this checker (tests/schema_mapping.rs): teach it the keyword"
                ),
            }
        }
    }
}

/// Values equal as JSON: a number equals another of the same value
/// whether written as an integer or a float.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(x, y)| same(x, y))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, x)| y.get(k).is_some_and(|y| same(x, y)))
        }
        _ => a == b,
    }
}

fn is_type(t: &str, v: &Value) -> bool {
    match t {
        "null" => v.is_null(),
        "boolean" => v.is_boolean(),
        "string" => v.is_string(),
        "array" => v.is_array(),
        "object" => v.is_object(),
        "number" => v.is_number(),
        "integer" => v.is_i64() || v.is_u64() || v.as_f64().is_some_and(|f| f.fract() == 0.0),
        _ => panic!("unknown type {t}"),
    }
}

/// The patterns schema/'s mapped files use, each matched by hand. Another
/// pattern stops the test until it is added here.
fn fits_pattern(pattern: &str, s: &str) -> bool {
    fn digits(s: &str) -> bool {
        !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
    }
    /// `[0-9]+(\.[0-9]{1,16})?`
    fn decimal(s: &str) -> bool {
        match s.split_once('.') {
            Some((whole, frac)) => digits(whole) && digits(frac) && frac.len() <= 16,
            None => digits(s),
        }
    }
    match pattern {
        r"^-?[0-9]+(\.[0-9]{1,16})?$" => decimal(s.strip_prefix('-').unwrap_or(s)),
        r"^[0-9]+(\.[0-9]{1,16})?%?$" => decimal(s.strip_suffix('%').unwrap_or(s)),
        r"^[0-9]+(/[0-9]+)?$" => match s.split_once('/') {
            Some((a, b)) => digits(a) && digits(b),
            None => digits(s),
        },
        _ => panic!(
            "pattern {pattern} is new to this checker (tests/schema_mapping.rs): teach it the pattern"
        ),
    }
}
