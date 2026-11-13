//! `build_publish` (docs/mcp.md): build a working copy for the browser into
//! a local directory with a manifest saying what's in it. The project has
//! no remote, so publishing is local: the directory is what a hosted
//! runtime would serve.
//!
//! It refuses before building when the model check fails, when any slot
//! still holds an original asset of a commercial source (docs/stubs.md),
//! or when a slot's files no longer match the digest `slots.toml` records.
//! The builds are the repo's scripts (docs/web.md), run as child
//! processes.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use gp_model::Model;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::catalog::{self, SlotState, State};
use crate::{Config, Reply, model_check};

/// The manifest's format version.
///
/// 0.2.0 adds `parent` (the template the copy was forked from, by its
/// files' hashes) and `changes` (the model facts, contract rules and slots
/// that differ from it), and `wasm` (whether the preview module was shrunk).
pub const FORMAT: &str = "0.2.0";

/// The manifest's name in the output directory.
pub const MANIFEST: &str = "manifest.json";

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PublishArgs {
    pub copy: String,
    /// `web-preview` (default): the interpreter preview, which plays any
    /// model with one prebuilt module. `web` adds a generated Bevy game; it
    /// runs `gp gen bevy`, which this gp doesn't have, so it fails today.
    #[serde(default)]
    pub target: Option<String>,
    /// The output directory, outside any git work tree, new, empty or an
    /// earlier build's. Default: `<work>/.publish/<copy>/<target>`.
    #[serde(default)]
    pub out: Option<PathBuf>,
    /// Run every check and return the manifest without building.
    #[serde(default)]
    pub dry_run: bool,
    /// Fetch the web tools (binaryen's wasm-opt, and for `web` also
    /// wasm-bindgen) if they aren't there, so the preview module is shrunk.
    /// Without it the preview uses them when `$WEB_TOOLS` names them or an
    /// earlier fetch left them, and else ships the unshrunk module (1.4 MB
    /// against 1.2 MB), saying so in `wasm`. The fetch is pinned and hash
    /// checked (docs/web.md) and builds wasm-bindgen-cli, so it can take
    /// minutes the first time.
    #[serde(default)]
    pub fetch_tools: bool,
}

/// The directory of the web tools: `$WEB_TOOLS`, or what an earlier fetch
/// left in `$GP_HOME`, or (with `fetch`) a fetch.
fn web_tools(config: &Config, fetch: bool) -> Result<Option<PathBuf>, String> {
    if let Some(t) = std::env::var_os("WEB_TOOLS") {
        return Ok(Some(PathBuf::from(t)));
    }
    let have = config.home.join("work").join("web-tools").join("bin");
    if have.join("wasm-opt").exists() && have.join("wasm-bindgen").exists() {
        return Ok(Some(have));
    }
    if !fetch {
        return Ok(None);
    }
    let mut cmd = Command::new("bash");
    cmd.arg(config.repo.join("scripts/web/fetch-tools.sh"))
        .arg(&config.home);
    let lines = run_step(config, &mut cmd, "scripts/web/fetch-tools.sh")?;
    lines
        .last()
        .map(|l| Some(PathBuf::from(l)))
        .ok_or_else(|| "fetch-tools.sh printed no directory".to_owned())
}

/// What differs between the copy and the template it came from.
fn parent_and_changes(
    config: &Config,
    dir: &Path,
    model: &Model,
    state: &BTreeMap<String, Value>,
) -> (Value, Value) {
    let table = |p: &Path| {
        fs::read_to_string(p)
            .ok()
            .and_then(|t| t.parse::<toml::Table>().ok())
    };
    let id = table(&dir.join("template.toml"))
        .and_then(|t| t.get("template")?.get("id")?.as_str().map(str::to_owned));
    let recorded = table(&dir.join("template.toml")).and_then(|t| {
        t.get("parent")?
            .get("content_sha256")?
            .as_str()
            .map(str::to_owned)
    });
    let slot_changes: Vec<Value> = state
        .iter()
        .filter(|(_, v)| v["status"] != "original")
        .map(|(k, v)| json!({ "slot": k, "status": v["status"], "origin": v["origin"] }))
        .collect();
    let Some(id) = id.filter(|i| gp_model::path::is_name(&i.replace('-', "_"))) else {
        return (
            Value::Null,
            json!({ "against": null, "slots": slot_changes }),
        );
    };
    let pdir = config.repo.join("templates").join(&id);
    let ptext = fs::read_to_string(pdir.join(gp_model::MODEL_FILE)).ok();
    let Some(pmodel) = ptext.as_deref().and_then(|t| Model::parse(t).ok()) else {
        return (
            json!({ "template": id, "found": false, "content_sha256": recorded }),
            json!({ "against": null, "slots": slot_changes }),
        );
    };
    let model_changes: Vec<Value> = gp_model::diff::diff(&pmodel.to_tree(), &model.to_tree())
        .iter()
        .filter(|c| !c.path().starts_with("evidence"))
        .map(|c| serde_json::to_value(c).expect("a change serializes"))
        .collect();
    let rules = |p: &Path| -> Vec<(String, Option<String>, String, Value)> {
        fs::read_to_string(p)
            .ok()
            .and_then(|t| gp_contract::Contract::parse(&t).ok())
            .map(|c| {
                c.rules
                    .iter()
                    .map(|r| {
                        let v = serde_json::to_value(r).expect("a rule serializes");
                        (
                            r.path.clone(),
                            r.aspect.clone(),
                            format!("{:?}", r.level),
                            v,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let (old, new) = (
        rules(&pdir.join(gp_contract::CONTRACT_FILE)),
        rules(&dir.join(gp_contract::CONTRACT_FILE)),
    );
    let mut contract = Vec::new();
    let mut free = Vec::new();
    for (path, aspect, level, v) in &new {
        match old.iter().find(|(p, a, _, _)| p == path && a == aspect) {
            None => contract.push(json!({ "change": "added", "path": path, "aspect": aspect, "level": level.to_lowercase() })),
            Some((_, _, l, ov)) if l != level || ov != v => {
                contract.push(json!({ "change": "changed", "path": path, "aspect": aspect, "from": l.to_lowercase(), "to": level.to_lowercase() }));
            }
            _ => {}
        }
        if level == "Free"
            && !old
                .iter()
                .any(|(p, a, l, _)| p == path && a == aspect && l == "Free")
        {
            free.push(json!(path));
        }
    }
    for (path, aspect, level, _) in &old {
        if !new.iter().any(|(p, a, _, _)| p == path && a == aspect) {
            contract.push(json!({ "change": "removed", "path": path, "aspect": aspect, "level": level.to_lowercase() }));
        }
    }
    let sha = |p: PathBuf| fs::read(p).ok().map(|b| catalog::sha256_hex(&b));
    (
        json!({
            "template": id,
            "found": true,
            "content_sha256": recorded,
            "model_sha256": ptext.as_deref().map(|t| catalog::sha256_hex(t.as_bytes())),
            "contract_sha256": sha(pdir.join(gp_contract::CONTRACT_FILE)),
        }),
        json!({
            "against": id,
            "model": model_changes,
            "contract_rules": contract,
            "contract_set_free": free,
            "slots": slot_changes,
        }),
    )
}

fn refused(stage: &str, problems: Value) -> Reply {
    Reply::refused(json!({ "ok": false, "written": false, "stage": stage, "problems": problems }))
}

fn sha256_file(path: &Path) -> Result<String, String> {
    fs::read(path)
        .map(|b| catalog::sha256_hex(&b))
        .map_err(|e| format!("reading {}: {e}", path.display()))
}

/// Every file under `dir`, by path with `/`, sorted.
fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) -> Result<(), String> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(|e| format!("reading {}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .collect();
    entries.sort_by_key(fs::DirEntry::file_name);
    for e in entries {
        let p = e.path();
        let t = e.file_type().map_err(|e| e.to_string())?;
        if t.is_dir() {
            walk(&p, base, out)?;
        } else if t.is_file() {
            let rel = p.strip_prefix(base).map_err(|e| e.to_string())?;
            out.push(
                rel.components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/"),
            );
        }
    }
    Ok(())
}

/// Runs a build step in the repo with stdin closed, and returns its
/// output's lines. A step that fails is an error carrying the end of what
/// it printed.
fn run_step(config: &Config, cmd: &mut Command, what: &str) -> Result<Vec<String>, String> {
    let out = cmd
        .current_dir(&config.repo)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{what}: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = stderr.lines().rev().take(15).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        return Err(format!(
            "{what} failed ({}):\n{}",
            out.status,
            tail.join("\n")
        ));
    }
    Ok(stdout.lines().map(str::to_owned).collect())
}

pub fn publish(config: &Config, a: PublishArgs) -> Result<Reply, String> {
    let target = a.target.as_deref().unwrap_or("web-preview");
    if !matches!(target, "web-preview" | "web") {
        return Err(format!("target `{target}` isn't `web-preview` or `web`"));
    }
    let dir = config.existing_copy(&a.copy)?;
    let model_text = fs::read_to_string(dir.join(gp_model::MODEL_FILE))
        .map_err(|e| format!("reading {}: {e}", gp_model::MODEL_FILE))?;
    let model = Model::parse(&model_text).map_err(|e| format!("{}: {e}", gp_model::MODEL_FILE))?;
    let source = model.model.source.as_deref();

    let (ok, report) = model_check(config, &dir, source)?;
    if !ok {
        return Ok(refused("model check", report["problems"].clone()));
    }

    let assets = config
        .assets_of(&dir, source)
        .filter(|a| a.join(gp_assets::MAP_FILE).is_file())
        .ok_or("no assets for this copy: start the server with --assets, or lift them with gp assets lift")?;
    let map_text = fs::read_to_string(assets.join(gp_assets::MAP_FILE))
        .map_err(|e| format!("reading {}: {e}", gp_assets::MAP_FILE))?;
    let map = gp_assets::AssetMap::parse(&map_text).map_err(|e| e.to_string())?;
    let open = catalog::is_open_tier(&dir);
    let state = match State::read(&assets)? {
        Some(s) => s,
        None => crate::place::original_state(&model, &map, &assets, &dir)?,
    };
    let by_slot = catalog::slot_assets(&model, &map)?;

    let mut problems = Vec::new();
    let mut slots = BTreeMap::new();
    for name in model.assets.keys() {
        let Some(asset) = by_slot.get(name.as_str()) else {
            problems.push(json!({
                "slot": name,
                "message": "the slot has no `from`, so there's nothing to build it from",
            }));
            continue;
        };
        let now = catalog::asset_digest(&assets, asset)?;
        let s: SlotState = match state.slots.get(name) {
            Some(s) => s.clone(),
            None => {
                problems.push(json!({
                    "slot": name,
                    "message": format!("{} has no entry for the slot, so its provenance is unknown", catalog::STATE_FILE),
                }));
                continue;
            }
        };
        if s.sha256 != now {
            problems.push(json!({
                "slot": name,
                "message": format!("the slot's files changed since {} recorded them (sha256 {} now, {} recorded); place them with assets_place so their provenance is known", catalog::STATE_FILE, now, s.sha256),
            }));
        }
        if catalog::holds_commercial_original(&s, open) {
            problems.push(json!({
                "slot": name,
                "status": s.status.name(),
                "license": s.license,
                "message": "the slot still holds an original asset of a commercial source, which comes from the person's own copy and is never shared; replace all of it (assets_generate or assets_place without `entity`, or a new level through model_edit)",
            }));
        }
        let mut v = serde_json::to_value(&s).expect("a slot state serializes");
        v["reads"] = json!(
            if gp_interp::lifted::simulation_slots(&model).contains(&name) {
                "simulation"
            } else {
                "presentation"
            }
        );
        slots.insert(name.clone(), v);
    }

    let manifest_template = fs::read_to_string(dir.join("template.toml"))
        .ok()
        .and_then(|t| t.parse::<toml::Table>().ok())
        .and_then(|t| t.get("template").and_then(toml::Value::as_table).cloned())
        .map(|t| {
            json!({
                "id": t.get("id").and_then(toml::Value::as_str),
                "title": t.get("title").and_then(toml::Value::as_str),
                "tier": t.get("tier").and_then(toml::Value::as_str),
            })
        });
    let contract_sha = fs::read(dir.join(gp_contract::CONTRACT_FILE))
        .ok()
        .map(|b| catalog::sha256_hex(&b));
    let asset_licenses: BTreeSet<String> = slots
        .values()
        .filter_map(|v| v["license"].as_str().map(str::to_owned))
        .collect();
    let baselines: Vec<PathBuf> = match source.filter(|s| crate::is_source_name(s)) {
        Some(s) => {
            let mut v: Vec<PathBuf> = fs::read_dir(config.repo.join("baselines").join(s))
                .map(|d| {
                    d.filter_map(Result::ok)
                        .map(|e| e.path())
                        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
                        .collect()
                })
                .unwrap_or_default();
            v.sort();
            v
        }
        None => Vec::new(),
    };
    let (parent, changes) = parent_and_changes(config, &dir, &model, &slots);
    let mut manifest = json!({
        "format": FORMAT,
        "parent": parent,
        "changes": changes,
        "copy": a.copy,
        "target": target,
        "template": manifest_template,
        "model": {
            "name": model.model.name,
            "title": model.model.title,
            "source": model.model.source,
            "sha256": catalog::sha256_hex(model_text.as_bytes()),
            "canonical_sha256": catalog::sha256_hex(model.to_toml().as_bytes()),
            "contract_sha256": contract_sha,
        },
        "assets": {
            "map_sha256": catalog::sha256_hex(map_text.as_bytes()),
            "slots": slots,
        },
        "licenses": {
            "runtime": env!("CARGO_PKG_LICENSE"),
            "template": catalog::TEMPLATE_LICENSE,
            "assets": asset_licenses,
        },
        "baselines": baselines.iter().filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned())).collect::<Vec<_>>(),
    });
    if target == "web" {
        manifest["licenses"]["runtime_note"] = json!(
            "the Bevy build also links the crates its lockfile pins, each under its own license"
        );
    }
    if !problems.is_empty() {
        let mut r = refused("slots", json!(problems));
        r.json["manifest"] = manifest;
        return Ok(r);
    }

    let out = match &a.out {
        Some(o) => std::path::absolute(o).map_err(|e| format!("{}: {e}", o.display()))?,
        None => config.work.join(".publish").join(&a.copy).join(target),
    };
    let existing = out
        .ancestors()
        .find(|p| p.exists())
        .unwrap_or(Path::new("/"));
    if let Some(tree) = gp_evidence::paths::enclosing_git_work_tree(existing) {
        return Err(format!(
            "{} is inside the git work tree {}; a build goes outside it",
            out.display(),
            tree.display()
        ));
    }
    if out.is_dir()
        && !out.join(MANIFEST).is_file()
        && fs::read_dir(&out)
            .map_err(|e| e.to_string())?
            .next()
            .is_some()
    {
        return Err(format!(
            "{} holds files and no {MANIFEST}, so it isn't an earlier build; give an empty or new directory",
            out.display()
        ));
    }
    if a.dry_run {
        return Ok(Reply::ok(json!({
            "ok": true,
            "written": false,
            "dry_run": true,
            "out": out,
            "manifest": manifest,
        })));
    }

    fs::create_dir_all(&out).map_err(|e| format!("creating {}: {e}", out.display()))?;
    let mut steps = Vec::new();
    let (tools, tools_note) = match web_tools(config, a.fetch_tools || target == "web") {
        Ok(t) => (t, None),
        Err(e) if target == "web" => return Err(e),
        Err(e) => (None, Some(e)),
    };
    let mut preview = Command::new("bash");
    if let Some(t) = &tools {
        preview.env("WEB_TOOLS", t);
    }
    preview
        .arg(config.repo.join("scripts/web/build-preview.sh"))
        .arg(&out)
        .arg(&dir)
        .arg(&assets)
        .args(&baselines);
    steps.push(json!({
        "step": "preview",
        "output": run_step(config, &mut preview, "scripts/web/build-preview.sh")?,
    }));
    if target == "web" {
        let tools = tools.clone().ok_or("no web tools")?;
        let gen_dir = config.work.join(".publish").join(&a.copy).join("gen");
        let (code, v) = crate::run_gp(
            config,
            &[
                "gen".as_ref(),
                "bevy".as_ref(),
                dir.as_os_str(),
                "-o".as_ref(),
                gen_dir.as_os_str(),
                "--assets".as_ref(),
                assets.as_os_str(),
            ],
        )?;
        if code != 0 {
            return Err(format!("gp gen bevy failed: {v}"));
        }
        let mut bevy = Command::new("bash");
        bevy.arg(config.repo.join("scripts/web/build-bevy.sh"))
            .arg(&gen_dir)
            .arg(&out)
            .arg("webgl2")
            .arg("size")
            .env("WEB_TOOLS", &tools)
            .env(
                "CARGO_TARGET_DIR",
                config.work.join(".publish").join("bevy-target"),
            );
        steps.push(json!({
            "step": "bevy",
            "output": run_step(config, &mut bevy, "scripts/web/build-bevy.sh")?,
        }));
    }

    let mut names = Vec::new();
    walk(&out, &out, &mut names)?;
    let mut files = BTreeMap::new();
    for n in names.iter().filter(|n| n.as_str() != MANIFEST) {
        files.insert(n.clone(), sha256_file(&out.join(n))?);
    }
    manifest["wasm"] = json!({
        "bytes": fs::metadata(out.join("gp_web.wasm")).map(|m| m.len()).ok(),
        "optimized": tools.is_some(),
        "note": match (&tools, &tools_note) {
            (Some(_), _) => "shrunk with wasm-opt".to_owned(),
            (None, Some(e)) => format!("unshrunk: fetching the web tools failed: {e}"),
            (None, None) => "unshrunk: no web tools found; set WEB_TOOLS, or pass fetch_tools to fetch them".to_owned(),
        },
    });
    manifest["files"] = json!(files);
    manifest["build"] = json!(steps);
    let text = serde_json::to_string_pretty(&manifest).expect("JSON serializes") + "\n";
    let tmp = out.join(format!("{MANIFEST}.tmp"));
    fs::write(&tmp, &text).map_err(|e| format!("writing {}: {e}", tmp.display()))?;
    fs::rename(&tmp, out.join(MANIFEST)).map_err(|e| format!("writing {MANIFEST}: {e}"))?;
    Ok(Reply::ok(json!({
        "ok": true,
        "written": true,
        "out": out,
        "index": out.join("index.html"),
        "manifest": manifest,
        "serve": format!("python3 -I -m http.server --bind 127.0.0.1 --directory {} 8765", out.display()),
    })))
}
