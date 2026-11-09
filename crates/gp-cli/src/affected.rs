//! The clips a change can move. A clip's score reads the rest of the corpus
//! score's inputs whole (this gp, the mapping, the contract, the ROM, the
//! tolerances, the mechanic catalog, the model's pictures and the spec's own
//! fields), and of the spec's kinds, levels, machines, counters and cameras
//! and of the hooks only what its run noted (`gp_sim::reads`, on the host and
//! inside the module). So a score keeps a fingerprint of each, and a later
//! score against it scores again only the clips whose reads meet an element
//! or hook whose hash changed; when anything read whole changed, every clip.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::Result;
use gp_interp::reads::ReadSet;
use gp_model::Model;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::scenario::Bench;

/// What a corpus score read, as hashes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Fingerprint {
    /// Everything a clip reads whole, as one hash.
    pub global: String,
    /// What `global` is made of, each as its hash, so a reader can say
    /// which changed.
    pub whole: BTreeMap<String, String>,
    /// The state mapping's hash: a clip's curve stays the corpus's only
    /// while it is the same.
    pub mapping: String,
    /// Each element's hash by list (`kinds`, `levels`, ...) and name.
    pub lists: BTreeMap<String, BTreeMap<String, String>>,
    /// Each hook's hash by name: the hash of the game's hook code as a
    /// whole. gp links no game's crate, so it can't hash each hook's reach
    /// at build time; an edit anywhere in the crate rescans every clip that
    /// calls a hook, which costs time and misses nothing.
    pub hooks: BTreeMap<String, String>,
    /// The hash of the game's `ext/` tree, which `module` is built from.
    #[serde(default)]
    pub ext: String,
    /// The built module's hash: with `ext` the same and this different,
    /// the module was built from another kit.
    #[serde(default)]
    pub module: String,
}

fn h(bytes: &[u8]) -> String {
    gp_evidence::Hash::of(bytes).to_string()
}

/// The fingerprint of scoring on bench `b` with `model`, whose hook code is
/// in the game folder `game`.
pub(crate) fn fingerprint(b: &Bench, model: &Model, game: &Path) -> Result<Fingerprint> {
    let spec = b.hosted.spec.clone();
    let mut whole = BTreeMap::new();
    let mut put = |k: &str, v: String| {
        whole.insert(k.to_owned(), v);
    };
    put("gp", crate::corpus_score::build());
    put("mapping", h(b.mapping_text.as_bytes()));
    put(
        "contract",
        h(&std::fs::read(b.dir.join(gp_contract::CONTRACT_FILE)).unwrap_or_default()),
    );
    put("rom", h(&b.rom));
    put("input", h(b.log_text.as_bytes()));
    let (tolerance, _): (Vec<gp_sim::Fx>, _) =
        crate::lockstep::tolerances(&b.dir, model, &b.mapping);
    put("tolerances", h(format!("{tolerance:?}").as_bytes()));
    put(
        "catalog",
        h(format!("{:?}", gp_interp::mechanics::catalog(model, &b.hosted.exts)).as_bytes()),
    );
    put(
        "pictures",
        h(match gp_look::present::art(model, &spec, &b.assets) {
            Ok(a) => format!("{a:?}"),
            Err(e) => format!("error {e}"),
        }
        .as_bytes()),
    );
    // The spec but its lists' elements, with the lists' names in order.
    let mut rest = spec.clone();
    rest.kinds = Default::default();
    rest.levels = Default::default();
    rest.machines = Default::default();
    rest.counters = Default::default();
    rest.cameras = Default::default();
    let names: Vec<Vec<&str>> = vec![
        spec.kinds.names().collect(),
        spec.levels.names().collect(),
        spec.machines.names().collect(),
        spec.counters.names().collect(),
        spec.cameras.names().collect(),
    ];
    // What the simulation reads across every kind, and each kind's role,
    // which it reads without noting the kind.
    let roles: Vec<Option<gp_sim::spec::Role>> = (0..spec.kinds.len())
        .map(|k| spec.kinds.role_of(k))
        .collect();
    put(
        "spec",
        h(format!("{rest:?}\n{names:?}\n{:?}\n{roles:?}", spec.across()).as_bytes()),
    );
    let mut lists = BTreeMap::new();
    fn each<T: gp_sim::reads::Element + std::fmt::Debug>(
        p: &gp_sim::reads::Parts<T>,
    ) -> BTreeMap<String, String> {
        p.unnoted()
            .iter()
            .map(|e| (e.name().to_owned(), h(format!("{e:?}").as_bytes())))
            .collect()
    }
    lists.insert("kinds".to_owned(), each(&spec.kinds));
    lists.insert("levels".to_owned(), each(&spec.levels));
    lists.insert("machines".to_owned(), each(&spec.machines));
    lists.insert("counters".to_owned(), each(&spec.counters));
    lists.insert("cameras".to_owned(), each(&spec.cameras));
    // A game with hooks hashes them by its `ext/` tree; with none, `gp`'s
    // own label stands for them.
    let ext = crate::package::hook_code(game)?.unwrap_or_default();
    let hooks: BTreeMap<String, String> = b
        .hosted
        .exts
        .keys()
        .map(|k| {
            (
                k.clone(),
                if ext.is_empty() {
                    h(b.hosted.hooks.as_bytes())
                } else {
                    ext.clone()
                },
            )
        })
        .collect();
    let global = h(serde_json::to_string(&whole)?.as_bytes());
    Ok(Fingerprint {
        global,
        mapping: whole["mapping"].clone(),
        whole,
        lists,
        hooks,
        ext,
        module: b.hosted.hooks.clone(),
    })
}

/// Which clips to score against an earlier score (`gp corpus score
/// --reuse`): the rest keep that score's rows.
#[derive(Debug, Default)]
pub(crate) struct Selection {
    /// The clips to score; the others are taken from the earlier score.
    pub score: BTreeSet<String>,
    /// The earlier rows kept, and their reads, by clip.
    pub kept: BTreeMap<String, (Value, Value)>,
    /// Clips the earlier score had exact: a split decides them, so their
    /// runs stop at it.
    pub stop: BTreeSet<String>,
    /// What changed, in words.
    pub says: String,
    /// The elements and hooks that changed, by list.
    pub changed: BTreeMap<String, BTreeSet<String>>,
}

/// The clips of `ids` (with their recipe hashes `recipes`) that a change
/// from the score `before` (a `gp corpus score --json` output) to `now`
/// can move.
pub(crate) fn select(
    before: &Value,
    now: &Fingerprint,
    ids: &[String],
    recipes: &BTreeMap<String, String>,
) -> Selection {
    let mut sel = Selection::default();
    let rows: BTreeMap<&str, &Value> = before["record"]["predicate"]["clips"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| r["id"].as_str().map(|id| (id, r)))
                .collect()
        })
        .unwrap_or_default();
    let then: Option<Fingerprint> = serde_json::from_value(before["fingerprint"].clone()).ok();
    // A clip it had exact stops at its first split, while the mapping is
    // the same (a stopped run's curve would be measured short).
    if let Some(t) = &then
        && t.mapping == now.mapping
    {
        sel.stop = ids
            .iter()
            .filter(|id| rows.get(id.as_str()).is_some_and(|r| r["exact"] == true))
            .cloned()
            .collect();
    }
    let all = |sel: &mut Selection, why: String| {
        sel.score = ids.iter().cloned().collect();
        sel.says = why;
    };
    let Some(then) = then else {
        all(&mut sel, "the earlier score has no fingerprint".into());
        return sel;
    };
    // The same hooks in another module: the kit the module was built from
    // changed, so nothing about the clips can be assumed.
    if then.ext == now.ext && then.module != now.module {
        all(&mut sel, "the module was built from another kit".into());
        return sel;
    }
    if then.global != now.global {
        let what: Vec<&str> = now
            .whole
            .iter()
            .filter(|(k, v)| then.whole.get(*k) != Some(*v))
            .map(|(k, _)| k.as_str())
            .collect();
        all(
            &mut sel,
            format!(
                "what every clip reads changed ({})",
                if what.is_empty() {
                    "its parts".to_owned()
                } else {
                    what.join(", ")
                }
            ),
        );
        return sel;
    }
    for (list, names) in &now.lists {
        let was = then.lists.get(list);
        for (n, hash) in names {
            if was.and_then(|w| w.get(n)) != Some(hash) {
                sel.changed
                    .entry(list.clone())
                    .or_default()
                    .insert(n.clone());
            }
        }
    }
    let hooks: BTreeSet<&String> = now.hooks.keys().chain(then.hooks.keys()).collect();
    for k in hooks {
        if now.hooks.get(k) != then.hooks.get(k) {
            sel.changed
                .entry("hooks".to_owned())
                .or_default()
                .insert(k.clone());
        }
    }
    let reads = &before["reads"];
    let mut why: BTreeMap<&str, usize> = BTreeMap::new();
    for id in ids {
        let row = rows.get(id.as_str());
        let read: Option<ReadSet> = serde_json::from_value(reads[id].clone()).ok();
        let reason = match (row, &read) {
            (None, _) => Some("new"),
            (Some(r), _) if r.get("error").is_some_and(|e| !e.is_null()) => {
                Some("couldn't be scored")
            }
            (Some(r), _) if r["recipe"].as_str() != recipes.get(id).map(String::as_str) => {
                Some("its recipe changed")
            }
            // A run stopped at its split was never whole.
            (Some(r), _) if r["stopped"] == true => Some("stopped at a split before"),
            (Some(_), None) => Some("its reads unknown"),
            (Some(_), Some(rs)) if rs.meets(&sel.changed) => Some("reads what changed"),
            _ => None,
        };
        match (reason, row) {
            (Some(w), _) => {
                *why.entry(w).or_default() += 1;
                sel.score.insert(id.clone());
            }
            (None, Some(r)) => {
                sel.kept
                    .insert(id.clone(), ((*r).clone(), reads[id].clone()));
            }
            (None, None) => {}
        }
    }
    let changed: Vec<String> = sel
        .changed
        .iter()
        .map(|(l, n)| {
            format!(
                "{l} {}",
                n.iter().map(String::as_str).collect::<Vec<_>>().join(", ")
            )
        })
        .collect();
    sel.says = format!(
        "changed: {}; scored again: {}",
        if changed.is_empty() {
            "nothing a clip reads".to_owned()
        } else {
            changed.join("; ")
        },
        if why.is_empty() {
            "none".to_owned()
        } else {
            why.iter()
                .map(|(w, n)| format!("{n} {w}"))
                .collect::<Vec<_>>()
                .join(", ")
        }
    );
    sel
}
