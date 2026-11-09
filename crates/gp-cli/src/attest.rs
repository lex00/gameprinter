//! `gp game attest`: signed statements about a published version of a game,
//! in-toto Statements under `attest/<id>/` in the registry. Three kinds:
//! the derivation of its corpus, the build of its module, and its score.
//! Each holds hashes and scores only, nothing of a ROM.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use gp_evidence::Paths;
use serde_json::{Value, json};

use crate::Outcome;

const STATEMENT: &str = "https://in-toto.io/Statement/v1";

fn hash(bytes: &[u8]) -> String {
    gp_evidence::Hash::of(bytes).to_string()
}

/// What `gp game attest` was given.
pub(crate) struct Given<'a> {
    pub dir: &'a Path,
    pub score: &'a Path,
    pub kit: &'a Path,
    pub module: Option<&'a Path>,
    pub id: &'a str,
    pub to: &'a str,
    pub key: &'a Path,
}

/// A JSON value as TOML, nulls left out (TOML has none).
fn to_toml(v: &Value) -> Result<toml::Value> {
    Ok(match v {
        Value::Bool(b) => toml::Value::Boolean(*b),
        Value::Number(n) => match n.as_i64() {
            Some(i) => toml::Value::Integer(i),
            None => toml::Value::Float(n.as_f64().context("a number TOML can't hold")?),
        },
        Value::String(s) => toml::Value::String(s.clone()),
        Value::Array(a) => toml::Value::Array(
            a.iter()
                .filter(|x| !x.is_null())
                .map(to_toml)
                .collect::<Result<_>>()?,
        ),
        Value::Object(m) => {
            let mut t = toml::Table::new();
            for (k, x) in m {
                if !x.is_null() {
                    t.insert(k.clone(), to_toml(x)?);
                }
            }
            toml::Value::Table(t)
        }
        Value::Null => bail!("a null where none is allowed"),
    })
}

/// A Statement about the package, as the file the registry holds.
fn statement(kind: &str, name: &str, id: &str, predicate: Value) -> Result<String> {
    let v = json!({
        "_type": STATEMENT,
        "subject": [{"name": name, "digest": {"blake3": id}}],
        "predicateType": format!("urn:game-printer:{kind}:1"),
        "predicate": predicate,
    });
    Ok(toml::to_string(&to_toml(&v)?)?)
}

/// The module's hash as the package's manifest names it: the published
/// package must be the one the module and the score are for.
fn manifest_module(to: &str, id: &str) -> Result<Option<String>> {
    let target = crate::registry::open(to);
    let text = target
        .read(&gp_package::package_key(id))
        .map_err(|e| anyhow::anyhow!(e))?
        .with_context(|| format!("the registry has no package {id}"))?;
    let text = String::from_utf8(text)?;
    Ok(text.lines().find_map(|l| {
        let mut p = l.split_whitespace();
        (p.next() == Some("file")).then_some(())?;
        let hash = p.next()?;
        p.next()?;
        (p.next() == Some("play/module.wasm")).then(|| hash.to_owned())
    }))
}

pub(crate) fn run(paths: &Paths, g: &Given) -> Result<Outcome> {
    let score: Value =
        serde_json::from_slice(&fs::read(g.score).with_context(|| g.score.display().to_string())?)
            .with_context(|| format!("{} isn't a gp game score output", g.score.display()))?;
    if score["ok"] != true {
        bail!(
            "the score has problems, so it isn't attested: {}",
            score["problems"]
        );
    }
    let r = &score["record"]["predicate"];
    if r["results"]["errors"] != 0 {
        bail!("the score has clips that couldn't be scored");
    }
    let pkg: toml::Table = fs::read_to_string(g.dir.join("package.toml"))
        .with_context(|| g.dir.display().to_string())?
        .parse()
        .context("package.toml")?;
    let package = pkg
        .get("package")
        .context("package.toml has no [package]")?;
    let field = |k: &str| {
        package
            .get(k)
            .and_then(toml::Value::as_str)
            .with_context(|| format!("package.toml has no package.{k}"))
    };
    let name = format!("{}@{}", field("name")?, field("version")?);

    // The module: the one scored, the one the package ships, the one named.
    let module_file = match g.module {
        Some(m) => m.to_owned(),
        None => g.dir.join("play/module.wasm"),
    };
    let module_bytes = fs::read(&module_file).with_context(|| module_file.display().to_string())?;
    let module = hash(&module_bytes);
    if score["module"] != json!(module) {
        bail!(
            "{} is module {module}, the score was through {}",
            module_file.display(),
            score["module"]
        );
    }
    if let Some(shipped) = manifest_module(g.to, g.id)?
        && shipped != module
    {
        bail!("the package ships module {shipped}, this is {module}");
    }

    // What the clips' curves say: a set hash over every clip.
    let rows = r["clips"].as_array().context("the score has no clips")?;
    let mut curves = BTreeMap::new();
    for c in rows {
        let f = |k: &str| c[k].as_str().unwrap_or("").to_owned();
        curves.insert(f("id"), (f("curve"), f("coverage")));
    }
    let curve_set = crate::plan::curve_set_hash(&curves);
    if score["curves"] != json!(curve_set) {
        bail!("the score's curve-set hash isn't its rows'");
    }
    let by = crate::registry::public_of(g.key)?;
    let at = crate::registry::utc(0)?;
    let common = |runtime: Value| json!({"by": by, "at": at, "gp": r["gp"], "runtime": runtime});
    let merged = |mut base: Value, extra: Value| {
        base.as_object_mut()
            .expect("an object")
            .extend(extra.as_object().expect("an object").clone());
        base
    };

    let original = json!({"id": "original/nes/gp-nes", "version": env!("CARGO_PKG_VERSION")});
    let derivation = merged(
        common(original.clone()),
        json!({
            "recipes": r["corpus"]["hash"],
            "rom": r["rom"]["sha256"],
            "mapping": r["mapping"],
            "original": original,
            "curves": curve_set,
            "clips": curves.iter().map(|(id, (curve, coverage))| {
                let mut v = json!({"id": id, "curve": curve});
                if !coverage.is_empty() {
                    v["coverage"] = json!(coverage);
                }
                v
            }).collect::<Vec<_>>(),
        }),
    );

    let kit_hash = crate::module::tree_hash(g.kit)?;
    let scan = crate::package::scan::run(g.dir, &[], std::slice::from_ref(&module_file), paths)?;
    if scan.exit != 0 {
        bail!("gp package scan fails: {}", scan.text);
    }
    let cargo_lock = fs::read(g.dir.join("Cargo.lock")).context("the package has no Cargo.lock")?;
    let rom_hashes = crate::package::Package::load(g.dir)?.rom_hashes();
    let build = merged(
        common(json!({"id": "module/wasm", "version": r["runtime"]["scenario"]})),
        json!({
            "source": g.id,
            "toolchain": crate::module::kit_channel(g.kit)?,
            "module_kit": kit_hash,
            "cargo_lock": hash(&cargo_lock),
            "sandbox": {"platform": format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)},
            "module": module,
            "data_scan": {"pass": true, "roms": rom_hashes},
        }),
    );

    // Per clip: the hashes and the score, not the frames' details.
    let clips: Vec<Value> = rows
        .iter()
        .map(|c| {
            let mut v = serde_json::Map::new();
            for k in [
                "id",
                "recipe",
                "curve",
                "model_curve",
                "exact",
                "permille",
                "frames",
                "frames_together",
            ] {
                v.insert(k.to_owned(), c[k].clone());
            }
            Value::Object(v)
        })
        .collect();
    let scored = merged(
        common(json!({"id": "module/wasm", "version": r["runtime"]["scenario"]})),
        json!({
            "derivation": curve_set,
            "model": r["model"]["hash"],
            "module": module,
            "corpus": {"name": r["corpus"]["name"], "version": r["corpus"]["version"],
                       "recipes": r["corpus"]["hash"]},
            "results": r["results"],
            "clips": clips,
        }),
    );

    let statements = [
        statement("derivation", &name, g.id, derivation)?,
        statement("module-build", &name, g.id, build)?,
        statement("score", &name, g.id, scored)?,
    ];
    crate::registry::attest_cmd(g.to, g.id, &statements, g.key)
}
