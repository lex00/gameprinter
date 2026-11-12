//! `registry_search` (docs/mcp.md): the templates in the repo's
//! `templates/` and the sources registered in `sources/*.toml`, searched by
//! words, platform, genre (a model extension), entity roles and systems.
//! Each template says what it needs before it can be forked and played:
//! the source ROM (found by hash in `$GP_ROM_DIR`, or the one the server
//! was given) and its lifted assets.
//!
//! The mechanic index (Milestone 5, docs/originals.md) is worked out from
//! each model when it's searched, so it can't go stale: [`mechanics`]
//! names every mechanic a model uses by its carrier's role (`enemy:walk`,
//! `player:jump.bands`, `game:timer`). A search can require mechanics, ask
//! for the templates inspired by another (the manifests' `inspired_by`
//! provenance), and rank by likeness to an example template, the share of
//! mechanics two templates have in common.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Config, Reply};

/// The most results a search returns when it isn't given a limit.
pub const DEFAULT_LIMIT: usize = 20;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchArgs {
    /// Words, every one of which must appear in a template's or source's
    /// id, title, description, entity names or roles. Case doesn't matter.
    #[serde(default)]
    pub query: Option<String>,
    /// A source platform, such as `nes`.
    #[serde(default)]
    pub platform: Option<String>,
    /// A genre: the name of a model extension, such as `platformer`.
    #[serde(default)]
    pub genre: Option<String>,
    /// Entity roles every result must have, such as `boss`.
    #[serde(default)]
    pub roles: Vec<String>,
    /// Systems (mechanics) every result must run, such as
    /// `platformer.enemies`.
    #[serde(default)]
    pub systems: Vec<String>,
    /// Mechanics every result must use, from the mechanic index: a role
    /// and a mechanic (`enemy:contact.kick`, `player:jump.coyote`), or a
    /// mechanic alone for any role (`path.wait_near`).
    #[serde(default)]
    pub mechanics: Vec<String>,
    /// Only templates that took mechanics from this template, by its
    /// directory under templates/ (their manifests' `inspired_by`).
    #[serde(default)]
    pub inspired_by: Option<String>,
    /// Search by example: a template directory. Results get a
    /// `similarity`, the share of mechanics they and the example have in
    /// common (Jaccard, 0 to 1), and come most alike first.
    #[serde(default)]
    pub like: Option<String>,
    /// The most templates and sources to return (default 20).
    #[serde(default)]
    pub limit: Option<usize>,
}

/// The mechanics a model uses, read as plain TOML so any schema version is
/// indexed: each entity's components and their notable options under its
/// role, and the game's own (timers, conditions, level chains, exits,
/// spawn tables). Names follow the platformer extension's components
/// (docs/model.md).
pub fn mechanics(model: &toml::Table) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let has = |t: &toml::Value, k: &str| t.get(k).is_some();
    let entity = |role: &str, e: &toml::Value, out: &mut BTreeSet<String>| {
        let mut add = |m: &str| {
            out.insert(format!("{role}:{m}"));
        };
        let options: &[(&str, &[(&str, &str)])] = &[
            (
                "run",
                &[
                    ("acceleration", "run.acceleration"),
                    ("deceleration", "run.deceleration"),
                    ("skid_deceleration", "run.skid"),
                    ("run_action", "run.run_button"),
                    ("air_control", "run.air_control"),
                ],
            ),
            (
                "jump",
                &[
                    ("gravity_held", "jump.variable_height"),
                    ("bands", "jump.bands"),
                    ("coyote_frames", "jump.coyote"),
                    ("buffer_frames", "jump.buffer"),
                    ("move_first", "jump.move_first"),
                ],
            ),
            ("walk", &[("turn_at_ledges", "walk.turn_at_ledges")]),
            ("fall", &[]),
            ("fly", &[]),
            ("hop", &[]),
            ("path", &[("wait_near", "path.wait_near")]),
            ("patrol", &[]),
            (
                "contact",
                &[
                    ("stomp_depth", "contact.stomp"),
                    ("keep_on_stomp", "contact.keep_on_stomp"),
                ],
            ),
        ];
        for (comp, opts) in options {
            let Some(c) = e.get(*comp) else { continue };
            add(comp);
            for (k, m) in *opts {
                if has(c, k) {
                    add(m);
                }
            }
        }
        if e.get("fly")
            .and_then(|f| f.get("heading"))
            .and_then(toml::Value::as_str)
            .is_none_or(|h| h == "player")
            && has(e, "fly")
        {
            add("fly.aimed");
        }
        let mut touches: Vec<&str> = Vec::new();
        if let Some(o) = e.get("contact").and_then(|c| c.get("otherwise")) {
            touches.extend(o.as_str());
        }
        if let Some(m) = e.get("machine") {
            add("machine");
            if let Some(states) = m.get("states").and_then(toml::Value::as_table) {
                for st in states.values() {
                    if let Some(o) = st
                        .get("set")
                        .and_then(|s| s.get("contact"))
                        .and_then(|c| c.get("otherwise"))
                    {
                        touches.extend(o.as_str());
                    }
                    for tr in st
                        .get("transition")
                        .and_then(toml::Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        for d in tr
                            .get("do")
                            .and_then(toml::Value::as_array)
                            .into_iter()
                            .flatten()
                            .filter_map(toml::Value::as_str)
                        {
                            if d.starts_with("become:") {
                                add("become");
                            }
                        }
                    }
                }
            }
        }
        for t in touches {
            add(&format!("contact.{t}"));
        }
        if has(e, "ext") {
            add("ext");
        }
    };
    if let Some(es) = model.get("entities").and_then(toml::Value::as_table) {
        for e in es.values() {
            let role = e
                .get("role")
                .and_then(toml::Value::as_str)
                .unwrap_or("none");
            entity(role, e, &mut out);
        }
    }
    let mut game = |m: &str| {
        out.insert(format!("game:{m}"));
    };
    if let Some(cs) = model.get("counters").and_then(toml::Value::as_table) {
        for c in cs.values() {
            let every = c
                .get("change")
                .and_then(toml::Value::as_array)
                .into_iter()
                .flatten()
                .any(|ch| has(ch, "every"));
            if every {
                game("timer");
            }
        }
    }
    let mut actions = BTreeSet::new();
    if let Some(rules) = model.get("rules").and_then(toml::Value::as_table) {
        for r in rules.values() {
            for st in r
                .get("states")
                .and_then(toml::Value::as_table)
                .into_iter()
                .flat_map(|t| t.values())
            {
                for tr in st
                    .get("transition")
                    .and_then(toml::Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if has(tr, "if") {
                        actions.insert("conditions".to_owned());
                    }
                    for d in tr
                        .get("do")
                        .and_then(toml::Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(toml::Value::as_str)
                    {
                        match d {
                            "restart-level" => actions.insert("restart".to_owned()),
                            "next-level" => actions.insert("level_chain".to_owned()),
                            "reset-game" => actions.insert("reset".to_owned()),
                            _ => false,
                        };
                    }
                }
            }
        }
    }
    for a in &actions {
        game(a);
    }
    if let Some(ls) = model.get("levels").and_then(toml::Value::as_table) {
        for l in ls.values() {
            if has(l, "exit") {
                game("exits");
            }
            if has(l, "spawn_ahead") {
                game("spawn_ahead");
            }
            if has(l, "goal") {
                game("goal");
            }
            if has(l, "kill_below") {
                game("pits");
            }
            if l.get("spawn")
                .and_then(toml::Value::as_array)
                .into_iter()
                .flatten()
                .any(|sp| has(sp, "table"))
            {
                game("spawn_table");
            }
        }
    }
    if let Some(cs) = model.get("cameras").and_then(toml::Value::as_table)
        && cs
            .values()
            .any(|c| c.get("scroll_back").and_then(toml::Value::as_bool) == Some(false))
    {
        game("camera.no_scroll_back");
    }
    out
}

/// Whether an index holds a mechanic asked for as `role:mechanic` or as a
/// mechanic alone, for any role.
fn uses(index: &BTreeSet<String>, want: &str) -> bool {
    if want.contains(':') {
        index.contains(want)
    } else {
        index
            .iter()
            .any(|m| m.split_once(':').is_some_and(|(_, x)| x == want))
    }
}

/// The share of two indexes' mechanics they have in common.
fn jaccard(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f64 {
    let union = a.union(b).count();
    if union == 0 {
        return 0.0;
    }
    a.intersection(b).count() as f64 / union as f64
}

/// One manifest's `inspired_by` entries, as JSON, and the templates named.
fn provenance(manifest: &toml::Table) -> (Value, Vec<String>) {
    let entries: Vec<Value> = manifest
        .get("inspired_by")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .map(|i| {
            json!({
                "template": i.get("template").and_then(toml::Value::as_str),
                "took": strs(i.get("took")),
                "into": strs(i.get("into")),
                "note": i.get("note").and_then(toml::Value::as_str),
            })
        })
        .collect();
    let names = entries
        .iter()
        .filter_map(|e| e["template"].as_str().map(str::to_owned))
        .collect();
    (json!(entries), names)
}

/// A template directory's manifest and model, as plain TOML.
fn read_template(dir: &Path, name: &str) -> Result<(toml::Table, toml::Table), String> {
    let manifest: toml::Table = fs::read_to_string(dir.join("template.toml"))
        .map_err(|e| format!("reading templates/{name}/template.toml: {e}"))?
        .parse()
        .map_err(|e| format!("templates/{name}/template.toml: {e}"))?;
    let model_file = manifest
        .get("template")
        .and_then(|t| t.get("model"))
        .and_then(toml::Value::as_str)
        .unwrap_or(gp_model::MODEL_FILE);
    // Plain TOML, as gp template check reads it, so a template whose
    // model is on another schema version is still listed.
    let model: toml::Table = fs::read_to_string(dir.join(model_file))
        .map_err(|e| format!("reading templates/{name}/{model_file}: {e}"))?
        .parse()
        .map_err(|e| format!("templates/{name}/{model_file}: {e}"))?;
    Ok((manifest, model))
}

/// A registered source, as `sources/<platform>.toml` lists it.
struct Source {
    platform: String,
    entry: toml::Table,
}

impl Source {
    fn get(&self, key: &str) -> Option<&str> {
        self.entry.get(key).and_then(toml::Value::as_str)
    }
}

fn sources(repo: &Path) -> Result<Vec<Source>, String> {
    let dir = repo.join("sources");
    let mut files: Vec<_> = match fs::read_dir(&dir) {
        Ok(d) => d
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "toml"))
            .collect(),
        Err(_) => return Ok(Vec::new()),
    };
    files.sort();
    let mut out = Vec::new();
    for f in files {
        let text = fs::read_to_string(&f).map_err(|e| format!("reading {}: {e}", f.display()))?;
        let t: toml::Table = text.parse().map_err(|e| format!("{}: {e}", f.display()))?;
        let stem = f
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        for s in t
            .get("source")
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(entry) = s.as_table() {
                let platform = entry
                    .get("platform")
                    .and_then(toml::Value::as_str)
                    .unwrap_or(&stem)
                    .to_owned();
                out.push(Source {
                    platform,
                    entry: entry.clone(),
                });
            }
        }
    }
    Ok(out)
}

/// The SHA-256 of a NES image's PRG and CHR, header and trainer removed,
/// as `gp roms scan` prints it.
pub fn headerless_sha256(bytes: &[u8]) -> String {
    let data = if bytes.starts_with(b"NES\x1a") && bytes.len() >= 16 {
        let skip = 16 + if bytes[6] & 0x04 != 0 { 512 } else { 0 };
        bytes.get(skip..).unwrap_or(&[])
    } else {
        bytes
    };
    crate::catalog::sha256_hex(data)
}

/// The ROMs at hand by headerless SHA-256: every NES image `gp roms scan`
/// finds in `$GP_ROM_DIR`, and the one the server was given. A scan that
/// can't run leaves a note instead.
fn roms_at_hand(config: &Config) -> (BTreeMap<String, String>, Option<String>) {
    let mut out = BTreeMap::new();
    let mut note = None;
    match crate::run_gp(config, &["roms".as_ref(), "scan".as_ref()]) {
        Ok((_, v)) => {
            for r in v["roms"].as_array().into_iter().flatten() {
                if let (Some(h), Some(p)) = (r["image"]["sha256"].as_str(), r["path"].as_str()) {
                    out.entry(h.to_owned()).or_insert_with(|| p.to_owned());
                }
            }
        }
        Err(e) => note = Some(format!("gp roms scan didn't run: {e}")),
    }
    if let Some(r) = &config.rom
        && let Ok(bytes) = fs::read(r)
    {
        out.insert(headerless_sha256(&bytes), r.display().to_string());
    }
    (out, note)
}

fn strs(v: Option<&toml::Value>) -> Vec<String> {
    v.and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(str::to_owned))
        .collect()
}

/// The lifted assets for a source, and whether their digest is the one
/// the template pins: where the tools read them (`--assets`, or
/// `$GP_HOME/work/assets/<source>` as `gp assets lift` writes them), or
/// else where `gp template extract` put them.
fn extracted(config: &Config, source: &str, template_id: &str, pin: Option<&str>) -> Value {
    let candidates = [
        config.assets_for(Some(source)),
        crate::is_source_name(template_id).then(|| {
            config
                .home
                .join("work/templates")
                .join(template_id)
                .join("assets")
        }),
    ];
    let mut first = None;
    for dir in candidates.into_iter().flatten() {
        let v = lifted_at(&dir, source, pin);
        if v["present"] == true {
            return v;
        }
        first.get_or_insert(v);
    }
    first.unwrap_or(json!({ "present": false }))
}

fn lifted_at(dir: &Path, source: &str, pin: Option<&str>) -> Value {
    let map_path = dir.join(gp_assets::MAP_FILE);
    let Some(map) = fs::read_to_string(&map_path)
        .ok()
        .and_then(|t| gp_assets::AssetMap::parse(&t).ok())
        .filter(|m| m.source == source)
    else {
        return json!({ "dir": dir, "present": false });
    };
    let mut files = BTreeMap::new();
    let mut names = vec![gp_assets::MAP_FILE.to_owned()];
    for a in &map.assets {
        names.extend(a.files());
    }
    for n in names {
        match fs::read(dir.join(&n)) {
            Ok(b) => {
                files.insert(n, b);
            }
            Err(_) => return json!({ "dir": dir, "present": false, "missing": n }),
        }
    }
    let digest = crate::catalog::files_digest(&files);
    json!({
        "dir": dir,
        "present": true,
        "lifted_sha256": digest,
        "matches_pin": pin.map(|p| p == digest),
    })
}

/// Lowercase words of a text.
fn words(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// True when every query word starts a word of the haystack.
fn matches(query: &[String], hay: &BTreeSet<String>) -> bool {
    query
        .iter()
        .all(|q| hay.iter().any(|w| w.starts_with(q.as_str())))
}

pub fn search(config: &Config, a: SearchArgs) -> Result<Reply, String> {
    let query: Vec<String> = a
        .query
        .as_deref()
        .map(|q| words(q).into_iter().collect())
        .unwrap_or_default();
    let limit = a.limit.unwrap_or(DEFAULT_LIMIT).max(1);
    let registered = sources(&config.repo)?;
    let (roms, rom_note) = roms_at_hand(config);

    let mut dirs: Vec<_> = fs::read_dir(config.repo.join("templates"))
        .map_err(|e| format!("reading templates/: {e}"))?
        .filter_map(Result::ok)
        .filter(|e| e.path().join("template.toml").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    dirs.sort();

    // Every template's index and provenance first: search by example
    // needs the example's, and each result lists who took from it.
    let mut read = BTreeMap::new();
    let mut taken_by: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for name in &dirs {
        let dir = config.repo.join("templates").join(name);
        let (manifest, model) = read_template(&dir, name)?;
        let (_, from) = provenance(&manifest);
        for f in from {
            taken_by.entry(f).or_default().push(name.clone());
        }
        let index = mechanics(&model);
        read.insert(name.clone(), (manifest, model, index));
    }
    let example = match &a.like {
        Some(l) => Some(
            read.get(l)
                .map(|(_, _, i)| i.clone())
                .ok_or(format!("`{l}` isn't a template under templates/"))?,
        ),
        None => None,
    };

    let mut templates = Vec::new();
    let mut by_source: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for name in &dirs {
        let (manifest, model, index) = &read[name];
        let t = manifest.get("template").and_then(toml::Value::as_table);
        let s = manifest.get("source").and_then(toml::Value::as_table);
        let get = |tbl: Option<&toml::Table>, k: &str| {
            tbl.and_then(|t| t.get(k))
                .and_then(toml::Value::as_str)
                .map(str::to_owned)
        };
        let header = model.get("model").and_then(toml::Value::as_table);
        let id = get(t, "id").unwrap_or_else(|| name.clone());
        let title = get(t, "title").or_else(|| get(header, "title"));
        let tier = get(t, "tier");
        let platform = get(s, "platform");
        let source_id = get(s, "id");
        let how = get(s, "how");
        let accepts = strs(s.and_then(|s| s.get("accepts")));
        let genre: Vec<String> = header
            .and_then(|h| h.get("extensions"))
            .and_then(toml::Value::as_table)
            .map(|e| e.keys().cloned().collect())
            .unwrap_or_default();
        let systems = strs(header.and_then(|h| h.get("systems")));
        let mut roles: BTreeMap<String, usize> = BTreeMap::new();
        let mut entity_names = Vec::new();
        if let Some(es) = model.get("entities").and_then(toml::Value::as_table) {
            for (n, e) in es {
                entity_names.push(n.clone());
                if let Some(r) = e.get("role").and_then(toml::Value::as_str) {
                    *roles.entry(r.to_owned()).or_default() += 1;
                }
            }
        }
        let levels = model
            .get("levels")
            .and_then(toml::Value::as_table)
            .map_or(0, toml::Table::len);
        let slots: BTreeMap<String, String> = manifest
            .get("slots")
            .and_then(toml::Value::as_table)
            .map(|t| {
                t.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_owned()))
                    .collect()
            })
            .unwrap_or_default();
        let hooks = strs(manifest.get("ext").and_then(|e| e.get("hooks")));

        let mut hay = words(&format!(
            "{name} {id} {} {} {} {}",
            title.as_deref().unwrap_or(""),
            get(header, "description").unwrap_or_default(),
            source_id.as_deref().unwrap_or(""),
            entity_names.join(" "),
        ));
        hay.extend(roles.keys().flat_map(|r| words(r)));
        let (inspired, inspired_names) = provenance(manifest);
        hay.extend(index.iter().flat_map(|m| words(m)));
        let keep = matches(&query, &hay)
            && a.mechanics.iter().all(|m| uses(index, m))
            && a.inspired_by
                .as_ref()
                .is_none_or(|t| inspired_names.contains(t))
            && a.platform
                .as_ref()
                .is_none_or(|p| platform.as_ref() == Some(p))
            && a.genre.as_ref().is_none_or(|g| genre.contains(g))
            && a.roles.iter().all(|r| roles.contains_key(r))
            && a.systems.iter().all(|x| systems.contains(x));
        if let Some(sid) = &source_id {
            by_source.entry(sid.clone()).or_default().push(name.clone());
        }
        if !keep {
            continue;
        }

        let found = accepts.iter().find_map(|h| roms.get(h));
        let registered_entry = source_id.as_ref().and_then(|sid| {
            registered
                .iter()
                .find(|r| r.get("id") == Some(sid.as_str()))
        });
        let pin = manifest
            .get("recipe")
            .and_then(|r| r.get("lifted_sha256"))
            .and_then(toml::Value::as_str);
        let ex = source_id
            .as_deref()
            .filter(|s| crate::is_source_name(s))
            .map_or(json!({ "present": false }), |s| {
                extracted(config, s, &id, pin)
            });
        let rom_ok = found.is_some() || ex["present"] == true;
        let lifted_ok = ex["present"] == true && ex["matches_pin"] != false;
        let next = match (lifted_ok, found.is_some(), how.as_deref()) {
            (true, _, _) => "ready: template_fork it".to_owned(),
            (false, _, Some("authored")) => format!(
                "build its assets from its authored.toml: gp template build templates/{name}"
            ),
            (false, true, _) => {
                format!("lift its assets: gp template extract templates/{name} (or gp assets lift)")
            }
            (false, false, Some("build")) => format!(
                "build its source ({}), lift its assets, and start the server with --rom and --assets",
                get(s, "build").unwrap_or_else(|| "see the template".into())
            ),
            (false, false, _) => format!(
                "it needs your own copy of {}: gp roms add it to $GP_ROM_DIR (docs/sources.md), then gp template extract templates/{name}",
                source_id.as_deref().unwrap_or("the source")
            ),
        };
        templates.push(json!({
            "template": name,
            "id": id,
            "title": title,
            "description": get(header, "description"),
            "tier": tier,
            "platform": platform,
            "genre": genre,
            "systems": systems,
            "roles": roles,
            "entities": entity_names.len(),
            "levels": levels,
            "slots": slots,
            "ext_hooks": hooks.len(),
            "mechanics": index,
            "inspired_by": inspired,
            "taken_by": taken_by.get(name).cloned().unwrap_or_default(),
            "similarity": example.as_ref().map(|e| (jaccard(e, index) * 1000.0).round() / 1000.0),
            "source": {
                "id": source_id,
                "how": how,
                "build": get(s, "build"),
                "accepts": accepts,
                "registered": registered_entry.map(|r| json!({
                    "title": r.get("title"),
                    "status": r.get("status"),
                    "region": r.get("region"),
                    "revision": r.get("revision"),
                })),
            },
            "needs": {
                "rom": {
                    "present": found.is_some(),
                    "path": found,
                    "needed": !rom_ok,
                },
                "extracted": ex,
            },
            "ready": lifted_ok,
            "next": next,
        }));
    }
    if example.is_some() {
        templates.sort_by(|x, y| {
            let s = |v: &Value| v["similarity"].as_f64().unwrap_or(0.0);
            s(y).total_cmp(&s(x))
        });
    } else if !query.is_empty() {
        // A template the query names (its directory or ID) comes before
        // one whose description or mechanics only mention it: "smb1" is
        // Super Mario Bros. before a hack of it.
        templates.sort_by_key(|t| {
            let named = words(&format!(
                "{} {}",
                t["template"].as_str().unwrap_or(""),
                t["id"].as_str().unwrap_or("")
            ));
            !matches(&query, &named)
        });
    }
    let total_templates = templates.len();
    templates.truncate(limit);

    // Registered sources match on words and platform only: they have no
    // model, so a genre, role or system filter leaves them out.
    let model_filters = a.genre.is_some()
        || !a.roles.is_empty()
        || !a.systems.is_empty()
        || !a.mechanics.is_empty()
        || a.inspired_by.is_some()
        || a.like.is_some();
    let mut found_sources = Vec::new();
    if !model_filters {
        for r in &registered {
            let id = r.get("id").unwrap_or("");
            let hay = words(&format!(
                "{id} {} {} {} {}",
                r.get("title").unwrap_or(""),
                r.get("region").unwrap_or(""),
                r.get("revision").unwrap_or(""),
                r.platform
            ));
            if !matches(&query, &hay) || a.platform.as_ref().is_some_and(|p| *p != r.platform) {
                continue;
            }
            let sha = r.get("sha256").unwrap_or("");
            found_sources.push(json!({
                "id": id,
                "title": r.get("title"),
                "platform": r.platform,
                "region": r.get("region"),
                "revision": r.get("revision"),
                "status": r.get("status"),
                "rom_present": roms.contains_key(sha),
                "templates": by_source.get(id).cloned().unwrap_or_default(),
            }));
        }
    }
    let total_sources = found_sources.len();
    found_sources.truncate(limit);
    let mut out = json!({
        "templates": templates,
        "templates_found": total_templates,
        "sources": found_sources,
        "sources_found": total_sources,
    });
    if let Some(n) = rom_note {
        out["note"] = json!(n);
    }
    Ok(Reply::ok(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hash_skips_the_header_and_trainer() {
        let data = [7u8; 32];
        let mut ines = b"NES\x1a".to_vec();
        ines.extend([0u8; 12]);
        ines.extend(data);
        assert_eq!(headerless_sha256(&ines), crate::catalog::sha256_hex(&data));
        let mut trained = b"NES\x1a\x01\x00\x04".to_vec();
        trained.extend([0u8; 9]);
        trained.extend([9u8; 512]);
        trained.extend(data);
        assert_eq!(
            headerless_sha256(&trained),
            crate::catalog::sha256_hex(&data)
        );
        assert_eq!(headerless_sha256(&data), crate::catalog::sha256_hex(&data));
    }

    #[test]
    fn the_index_names_mechanics_by_role() {
        let model: toml::Table = r#"
[entities.hero]
role = "player"
run = { walk_speed = 1, acceleration = "0.25" }
jump = { action = "jump", speed = -4, gravity = "0.25", max_fall = 4, coyote_frames = 4 }

[entities.shell]
role = "enemy"
walk = { speed = "0.5", turn_at_ledges = true }
contact = { stomp_depth = 6, bounce = -3, keep_on_stomp = true, otherwise = "kill-player" }

[entities.shell.machine]
initial = "walking"

[entities.shell.machine.states.still]
set = { contact = { otherwise = "kick" } }

[[entities.shell.machine.states.walking.transition]]
on = "enemy.stomped"
to = "still"
do = ["become:shell"]

[entities.ad]
role = "projectile"
fly = { speed = 1 }

[counters.time]
change = [{ every = 60, by = -1 }]

[[rules.game.states.playing.transition]]
on = "goal.reached"
to = "won"
if = { counter = "time", cmp = ">", value = 0 }
do = ["next-level"]
"#
        .parse()
        .unwrap();
        let m = mechanics(&model);
        for want in [
            "player:run.acceleration",
            "player:jump.coyote",
            "enemy:walk.turn_at_ledges",
            "enemy:contact.stomp",
            "enemy:contact.kick",
            "enemy:contact.kill-player",
            "enemy:become",
            "projectile:fly.aimed",
            "game:timer",
            "game:conditions",
            "game:level_chain",
        ] {
            assert!(m.contains(want), "{want} not in {m:?}");
        }
        assert!(!m.contains("player:jump.bands"));
        assert!(uses(&m, "contact.kick"));
        assert!(uses(&m, "enemy:contact.kick"));
        assert!(!uses(&m, "player:contact.kick"));
    }

    #[test]
    fn likeness_is_the_share_in_common() {
        let a: BTreeSet<String> = ["x:a", "x:b", "x:c"].map(String::from).into();
        let b: BTreeSet<String> = ["x:b", "x:c", "x:d"].map(String::from).into();
        assert_eq!(jaccard(&a, &b), 0.5);
        assert_eq!(jaccard(&a, &a), 1.0);
        assert_eq!(jaccard(&BTreeSet::new(), &BTreeSet::new()), 0.0);
    }

    #[test]
    fn query_words_start_words() {
        let hay = words("Donkey Kong donkey-kong-ju boss");
        assert!(matches(&["donkey".into(), "bo".into()], &hay));
        assert!(!matches(&["pauline".into()], &hay));
        assert!(matches(&[], &hay));
    }
}
