//! The grid from the model (docs/research/clip-corpus.md, section 4): each
//! action leads to the mechanics it drives, each mechanic to its numbers
//! and the contexts it acts in, and each (mechanic, context) to the press
//! patterns its class allows, with hold lengths from [`super::HOLDS`] cut
//! at the original's anchors. A genre adds probe cells for moves the model
//! lacks. Four rules prune: a context the model's records rule out, a
//! button no mechanic reads in that context, a hold past the anchor where
//! the mechanic stops reading it, and cells whose outcome on the original
//! is the same frame for frame ([`prune_identical`]).
//!
//! Nothing here reads a number the model computes to place a press: a
//! timing is an anchor measured on the original, so refining the model
//! never moves a clip.

use std::collections::{BTreeMap, BTreeSet};

use gp_model::Model;

use super::{
    Anchor, Cell, Context, EventAt, HOLDS, Lead, MAX_FRAMES, Pattern, SETTLE, Status, span,
};
use crate::mapping::Mapping;
use crate::mechanics::{Catalog, Mechanic};

/// A number the refinement loop can change, as the model sets it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fitted {
    /// `entities.mario.jump.bands[0].below`.
    pub path: String,
    pub mechanic: String,
    /// Its value in 16.16 fixed point.
    pub raw: i64,
    /// A frame count or a count: whole numbers.
    pub whole: bool,
    pub range: String,
}

fn table_at<'a>(tree: &'a toml::Value, path: &str) -> Option<&'a toml::Value> {
    let mut node = tree;
    for part in path.split('.') {
        node = node.get(part)?;
    }
    Some(node)
}

/// The table a mechanic's parameters are in for one of its uses.
fn params_table<'a>(tree: &'a toml::Value, used_by: &str) -> Option<(String, &'a toml::Value)> {
    match table_at(tree, used_by) {
        Some(t @ toml::Value::Table(_)) => Some((used_by.to_owned(), t)),
        _ => {
            let (base, _) = used_by.rsplit_once('.')?;
            Some((base.to_owned(), table_at(tree, base)?))
        }
    }
}

/// A number's value: an integer or a fixed-point string.
pub fn raw_of(v: &toml::Value) -> Option<i64> {
    match v {
        toml::Value::Integer(i) => i.checked_mul(1 << 16),
        toml::Value::String(s) => gp_model::Fixed::parse(s).ok().map(|f| f.raw()),
        _ => None,
    }
}

/// The mechanics a part of the model carries (`entities.mario`), each with
/// the table its parameters are in.
pub fn carried<'a>(
    catalog: &'a Catalog,
    tree: &'a toml::Value,
    part: &str,
) -> Vec<(&'a Mechanic, String, &'a toml::Value)> {
    let pre = format!("{part}.");
    let mut out = Vec::new();
    for m in &catalog.mechanics {
        for u in &m.used_by {
            if (u.starts_with(&pre) || u == part)
                && let Some((base, t)) = params_table(tree, u)
            {
                out.push((m, base, t));
                break;
            }
        }
    }
    out
}

/// The numbers of a part's mechanics: each fixed-point number, frame count
/// and count it sets, and each number in a list parameter's items (a
/// jump's bands), in the records' order.
pub fn fitted(catalog: &Catalog, tree: &toml::Value, part: &str) -> Vec<Fitted> {
    let mut out = Vec::new();
    for (m, base, t) in carried(catalog, tree, part) {
        for p in &m.params {
            match t.get(p.name) {
                Some(v) if matches!(p.ty, "fixed" | "frames" | "count") => {
                    if let Some(raw) = raw_of(v) {
                        out.push(Fitted {
                            path: format!("{base}.{}", p.name),
                            mechanic: m.name.to_owned(),
                            raw,
                            whole: p.ty != "fixed",
                            range: p.range.to_owned(),
                        });
                    }
                }
                Some(toml::Value::Array(items)) if p.ty == "list" => {
                    for (i, item) in items.iter().enumerate() {
                        let Some(it) = item.as_table() else { continue };
                        for (k, v) in it {
                            let Some(raw) = raw_of(v) else { continue };
                            let like = m.params.iter().find(|q| q.name == k.as_str());
                            out.push(Fitted {
                                path: format!("{base}.{}[{i}].{k}", p.name),
                                mechanic: m.name.to_owned(),
                                raw,
                                whole: like.is_some_and(|q| q.ty != "fixed"),
                                range: like.map_or("", |q| q.range).to_owned(),
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let mut seen = BTreeSet::new();
    out.retain(|f| seen.insert(f.path.clone()));
    out
}

/// The model with one number set to `raw`, for a perturbation.
pub fn with_value(tree: &toml::Value, path: &str, raw: i64) -> Result<Model, String> {
    let mut tree = tree.clone();
    let p = gp_model::Path::parse(path).map_err(|e| e.to_string())?;
    let (last, parents) = p.segments.split_last().ok_or("an empty path")?;
    let mut node = &mut tree;
    for s in parents {
        node = node
            .get_mut(s.name.as_str())
            .ok_or_else(|| format!("{path}: no {}", s.name))?;
        if let Some(gp_model::path::Index::At(i)) = s.index {
            node = usize::try_from(i)
                .ok()
                .and_then(|i| node.get_mut(i))
                .ok_or_else(|| format!("{path}: no item {i}"))?;
        }
    }
    let t = node.as_table_mut().ok_or("not a table")?;
    let fx = gp_sim::Fx::from_raw(raw).to_string();
    let v = match fx.parse::<i64>() {
        Ok(i) => toml::Value::Integer(i),
        Err(_) => toml::Value::String(fx),
    };
    t.insert(last.name.clone(), v);
    let text = toml::to_string(&tree).map_err(|e| e.to_string())?;
    Model::parse(&text).map_err(|e| format!("{path}: {e}"))
}

/// An action and what it drives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drive {
    pub action: String,
    pub button: String,
    /// Mechanics it is the main action of.
    pub mechanics: Vec<String>,
    /// Mechanics it only modifies (a run button, read with a direction).
    pub modifies: Vec<String>,
    /// Level or game mechanics that read it in a context of their own:
    /// `core.exits` for an exit's held action.
    pub elsewhere: Vec<String>,
}

/// Each action of `[input.actions]`, one a button, and the mechanics it
/// drives: the player's by an `action` parameter or a read of it, and the
/// level's exits that hold it.
pub fn drives(model: &Model, catalog: &Catalog, player: &str) -> Vec<Drive> {
    let tree = model.to_tree();
    let Some(table) = tree
        .get("input")
        .and_then(|i| i.get("actions"))
        .and_then(|a| a.as_table())
    else {
        return Vec::new();
    };
    let mechs = carried(catalog, &tree, &format!("entities.{player}"));
    let mut exit_holds = BTreeSet::new();
    if let Some(levels) = tree.get("levels").and_then(|l| l.as_table()) {
        for l in levels.values() {
            for e in l
                .get("exit")
                .and_then(|e| e.as_array())
                .into_iter()
                .flatten()
            {
                if let Some(h) = e
                    .get("take")
                    .and_then(|t| t.get("hold"))
                    .and_then(|h| h.as_str())
                {
                    exit_holds.insert(h.to_owned());
                }
            }
        }
    }
    let mut out: Vec<Drive> = Vec::new();
    for (name, button) in table {
        let Some(button) = button.as_str() else {
            continue;
        };
        let mut d = Drive {
            action: name.clone(),
            button: button.to_ascii_lowercase(),
            mechanics: Vec::new(),
            modifies: Vec::new(),
            elsewhere: Vec::new(),
        };
        for (m, _, t) in &mechs {
            let mut main = m.reads.iter().any(|r| *r == format!("action.{name}"));
            let mut modifier = false;
            for p in m.params.iter().filter(|p| p.ty == "action") {
                let set = match t.get(p.name).and_then(|v| v.as_str()) {
                    Some(v) => v == name,
                    None => !p.optional && p.name == name,
                };
                if set {
                    if p.name.ends_with("_action") {
                        modifier = true;
                    } else {
                        main = true;
                    }
                }
            }
            // A modifier read is a modifier, even if the record reads it.
            if modifier {
                main = false;
                d.modifies.push(m.name.to_owned());
            } else if main {
                d.mechanics.push(m.name.to_owned());
            }
        }
        if exit_holds.contains(name.as_str()) && catalog.get("core.exits").is_some() {
            d.elsewhere.push("core.exits".into());
        }
        out.push(d);
    }
    let mut seen = BTreeSet::new();
    out.retain(|a| seen.insert(a.button.clone()));
    out
}

/// What the derivation reads from the model and mapping.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub player: String,
    pub jump: Option<String>,
    pub jump_button: String,
    pub run: bool,
    pub run_action: Option<String>,
    pub run_button: String,
    pub air_control: bool,
    pub gravity_held: bool,
    pub buffer: bool,
    pub coyote: bool,
    pub double_jump: bool,
    /// The jump's speed bands' `below`, in fixed point.
    pub bands: Vec<i64>,
    pub tiles: bool,
    /// Kinds whose touch the player meets, with their movers.
    pub enemies: Vec<String>,
    /// Actions an exit holds: name, button, and whether no mechanic of the
    /// player reads it too.
    pub exit_actions: Vec<(String, String, bool)>,
    pub water: bool,
    pub climb: bool,
    pub carry: Option<(String, String)>,
    pub shoot: bool,
    /// The player machine's states other than its first.
    pub states: Vec<String>,
    pub vx_mapped: bool,
    pub vy_mapped: bool,
    pub state_mapped: bool,
    pub genre: Vec<String>,
    /// Actions no mechanic reads, by name and button.
    pub unread: Vec<(String, String)>,
}

fn bool_at(t: &toml::Value, k: &str) -> bool {
    t.get(k).and_then(|v| v.as_bool()).unwrap_or(false)
}

/// The facts of a model, its catalog and mapping.
pub fn facts(model: &Model, catalog: &Catalog, mapping: &Mapping) -> Option<Facts> {
    let player = crate::scenario::player(model)?;
    let tree = model.to_tree();
    let part = format!("entities.{player}");
    let mechs = carried(catalog, &tree, &part);
    let ds = drives(model, catalog, &player);
    let mut f = Facts {
        player: player.clone(),
        genre: model.model.extensions.keys().cloned().collect(),
        ..Facts::default()
    };
    let mapped = |field: &str| {
        mapping
            .fields
            .iter()
            .any(|x| x.path == format!("entities.{player}[0].{field}"))
    };
    f.vx_mapped = mapped("vx");
    f.vy_mapped = mapped("vy");
    f.state_mapped = mapped("state");
    for (m, _, t) in &mechs {
        match m.name {
            "platformer.jump" => {
                let a = t.get("action").and_then(|v| v.as_str()).unwrap_or("jump");
                f.jump = Some(a.to_owned());
                f.gravity_held = t.get("gravity_held").is_some()
                    || t.get("bands")
                        .and_then(|b| b.as_array())
                        .is_some_and(|b| b.iter().any(|x| x.get("gravity_held").is_some()));
                f.buffer = t.get("buffer_frames").is_some();
                f.coyote = t.get("coyote_frames").is_some();
                f.bands = t
                    .get("bands")
                    .and_then(|b| b.as_array())
                    .map(|b| {
                        b.iter()
                            .filter_map(|x| x.get("below").and_then(raw_of))
                            .collect()
                    })
                    .unwrap_or_default();
            }
            "platformer.run" => {
                f.run = true;
                f.air_control = bool_at(t, "air_control");
                f.run_action = t
                    .get("run_action")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned);
            }
            "platformer.climb" => f.climb = true,
            "platformer.carry" => {
                let a = t.get("action").and_then(|v| v.as_str()).unwrap_or("carry");
                f.carry = Some((a.to_owned(), String::new()));
            }
            "platformer.shoot" => f.shoot = true,
            n if n.contains("double") => f.double_jump = true,
            _ => {}
        }
        if m.params.iter().any(|p| {
            (p.name.contains("air_jump") || p.name.contains("double")) && t.get(p.name).is_some()
        }) {
            f.double_jump = true;
        }
    }
    let button_of = |a: &str| {
        ds.iter()
            .find(|d| d.action == a)
            .map(|d| d.button.clone())
            .unwrap_or_default()
    };
    if let Some(j) = &f.jump {
        f.jump_button = button_of(j);
        if f.jump_button.is_empty() {
            f.jump = None;
        }
    }
    if let Some(r) = &f.run_action {
        f.run_button = button_of(r);
        if f.run_button.is_empty() {
            f.run_action = None;
        }
    }
    if let Some((a, b)) = &mut f.carry {
        *b = button_of(a);
    }
    f.tiles = tree
        .get("entities")
        .and_then(|e| e.get(&player))
        .is_some_and(|p| bool_at(p, "tile_collision"));
    if let Some(ents) = tree.get("entities").and_then(|e| e.as_table()) {
        for (k, e) in ents {
            if *k != player && e.get("contact").is_some() {
                f.enemies.push(k.clone());
            }
        }
    }
    for d in &ds {
        if d.elsewhere.iter().any(|m| m == "core.exits") {
            let only = d.mechanics.is_empty() && d.modifies.is_empty();
            f.exit_actions
                .push((d.action.clone(), d.button.clone(), only));
        }
        if d.mechanics.is_empty() && d.modifies.is_empty() && d.elsewhere.is_empty() {
            f.unread.push((d.action.clone(), d.button.clone()));
        }
    }
    f.water = catalog.get("platformer.water").is_some();
    if let Some(m) = tree
        .get("entities")
        .and_then(|e| e.get(&player))
        .and_then(|p| p.get("machine"))
    {
        let initial = m.get("initial").and_then(|v| v.as_str()).unwrap_or("");
        if let Some(st) = m.get("states").and_then(|s| s.as_table()) {
            f.states = st.keys().filter(|k| *k != initial).cloned().collect();
        }
    }
    Some(f)
}

/// The context table's order (docs/research/clip-corpus.md, section 3).
pub fn context_order(id: &str) -> (usize, String) {
    let head = id.split('[').next().unwrap_or(id);
    let rank = [
        "ground.still",
        "ground.walk_top",
        "ground.run_top",
        "ground.band",
        "ground.edge",
        "ground.ledge",
        "air.rising",
        "air.falling",
        "enemy.near",
        "exit",
        "water",
        "ladder",
        "carrying",
        "state",
    ]
    .iter()
    .position(|h| *h == head)
    .unwrap_or(99);
    (rank, id.to_owned())
}

/// The pattern table's order (section 2).
fn pattern_order(name: &str) -> usize {
    [
        "check", "idle", "tap", "hold", "release", "taps", "chord", "through", "early", "again",
        "late", "doublet", "turn", "air", "stomp", "mixed",
    ]
    .iter()
    .position(|p| *p == name)
    .unwrap_or(99)
}

/// A context's anchor for its band edge `i` (`jump.edge[1]`): the frames
/// a hold from standing takes on the original to reach that takeoff speed.
pub fn edge_anchor(i: usize) -> String {
    format!("jump.edge[{i}]")
}

/// The anchor of band `i`'s middle.
pub fn band_anchor(i: usize) -> String {
    format!("jump.band[{i}]")
}

/// The anchor of the top of a held jump from standing.
pub const TOP: &str = "jump.top";

fn context(id: &str, words: &str, predicate: &str, needs: &[&str], lead: Option<Lead>) -> Context {
    Context {
        id: id.to_owned(),
        words: words.to_owned(),
        predicate: predicate.to_owned(),
        needs: needs.iter().map(|s| (*s).to_owned()).collect(),
        lead,
        starts: Vec::new(),
        missing: None,
    }
}

fn fx_words(raw: i64) -> String {
    gp_sim::Fx::from_raw(raw).to_string()
}

/// The contexts the model's mechanics name, in the table's order, without
/// starts.
pub fn contexts(f: &Facts) -> Vec<Context> {
    let mut out = vec![context(
        "ground.still",
        "standing still",
        "on_ground and not moving for 10 frames, nothing alive within 80 px, every rule in its first state",
        &["platformer.jump", "platformer.run"],
        None,
    )];
    if f.run {
        out.push(context(
            "ground.walk_top",
            "walking at top speed",
            "on_ground, a direction held without run, and the speed across the same nonzero value for 4 frames",
            &["platformer.run"],
            None,
        ));
        if f.run_action.is_some() {
            out.push(context(
                "ground.run_top",
                "running at top speed",
                "on_ground, a direction and run held, and the speed across the same nonzero value for 4 frames",
                &["platformer.run"],
                None,
            ));
        }
    }
    if f.jump.is_some() && !f.bands.is_empty() && f.run {
        let n = f.bands.len();
        for i in 1..=n {
            let lo = fx_words(f.bands[i - 1]);
            let range = match f.bands.get(i) {
                Some(hi) => format!("{lo} <= abs(vx) < {}", fx_words(*hi)),
                None => format!("abs(vx) >= {lo}"),
            };
            out.push(context(
                &format!("ground.band[{i}]"),
                &if i == n {
                    "taking off above the last speed band".to_owned()
                } else {
                    format!("taking off in speed band {}", i + 1)
                },
                &format!(
                    "on_ground, {range}, reached from standing by a hold measured on the original"
                ),
                &["platformer.jump"],
                Some(Lead {
                    from: "ground.still".into(),
                    hold: vec!["fwd".into()],
                    anchor: band_anchor(i),
                    back: 0,
                }),
            ));
        }
        for (i, b) in f.bands.iter().enumerate() {
            for (side, back) in [("under", 3u64), ("over", 0u64)] {
                out.push(context(
                    &format!("ground.edge[{i}].{side}"),
                    &format!("taking off just {side} the speed {}", fx_words(*b)),
                    &format!(
                        "on_ground, abs(vx) just {} {}: the frame a hold from standing reaches it on the original{}",
                        if side == "under" { "under" } else { "at or over" },
                        fx_words(*b),
                        if back > 0 { ", less 3" } else { "" }
                    ),
                    &["platformer.jump"],
                    Some(Lead {
                        from: "ground.still".into(),
                        hold: vec!["fwd".into()],
                        anchor: edge_anchor(i),
                        back,
                    }),
                ));
            }
        }
    }
    if f.tiles {
        out.push(context(
            "ground.ledge",
            "at a ledge",
            "on_ground, with nothing to stand on 2 to 12 px ahead of the leading foot",
            &["platformer.jump", "platformer.run"],
            None,
        ));
    }
    if f.jump.is_some() {
        let lead = Some(Lead {
            from: "ground.still".into(),
            hold: Vec::new(),
            anchor: String::new(),
            back: 0,
        });
        out.push(context(
            "air.rising",
            "in the air, rising",
            "not on_ground, vy < 0; the jump's own state isn't carried over, so its clips start standing and jump",
            &["platformer.jump"],
            lead.clone(),
        ));
        out.push(context(
            "air.falling",
            "in the air, falling",
            "not on_ground, vy > 0; its clips start standing and jump, as for rising",
            &["platformer.jump"],
            lead,
        ));
    }
    for e in &f.enemies {
        out.push(context(
            &format!("enemy.near[{e}]"),
            &format!("near a {}", e.replace('_', " ")),
            &format!("on_ground, a living {e} 16 to 64 px ahead"),
            &["platformer.contact"],
            None,
        ));
    }
    if !f.exit_actions.is_empty() {
        out.push(context(
            "exit",
            "in an exit's box",
            "the player's hitbox inside an exit's take box that holds an action",
            &["core.exits"],
            None,
        ));
    }
    if f.water || (f.genre.iter().any(|g| g == "platformer") && f.jump.is_some()) {
        out.push(context(
            "water",
            "in water",
            "the player's middle in a water tile",
            &["platformer.water"],
            None,
        ));
    }
    if f.climb {
        out.push(context(
            "ladder",
            "on a ladder",
            "climbing, or the player's middle in a ladder tile",
            &["platformer.climb"],
            None,
        ));
    }
    if f.carry.is_some() {
        out.push(context(
            "carrying",
            "carrying something",
            "carrying set",
            &["platformer.carry"],
            None,
        ));
    }
    for s in &f.states {
        out.push(context(
            &format!("state[{s}]"),
            &format!("as {s}"),
            &format!("on_ground, the player's machine in state {s}"),
            &["core.modes"],
            None,
        ));
    }
    out.sort_by_key(|c| context_order(&c.id));
    out
}

struct Gen<'a> {
    anchors: &'a BTreeMap<String, Anchor>,
    contexts: &'a [Context],
    out: Vec<Cell>,
}

/// A pattern's part of a cell ID: `jump.hold-16`, `fwd+run.hold-64`.
fn key_of(p: &Pattern) -> String {
    let mut k = String::new();
    if !p.action.is_empty() {
        k.push_str(&p.action);
        for w in &p.with {
            k.push('+');
            k.push_str(w);
        }
        k.push('.');
    }
    k.push_str(&p.name);
    if let Some(n) = p.frames {
        k.push_str(&format!("-{n}"));
    }
    if let Some(a) = &p.anchor {
        k.push_str(&format!("-{}", a.rsplit('.').next().unwrap_or(a)));
    }
    k
}

fn pat(name: &str, action: &str, with: &[&str], frames: Option<u64>) -> Pattern {
    Pattern {
        name: name.to_owned(),
        action: action.to_owned(),
        with: with.iter().map(|s| (*s).to_owned()).collect(),
        frames,
        anchor: None,
        event: None,
    }
}

impl Gen<'_> {
    fn has(&self, ctx: &str) -> bool {
        self.contexts.iter().any(|c| c.id == ctx)
    }

    fn words(&self, ctx: &str) -> String {
        self.contexts
            .iter()
            .find(|c| c.id == ctx)
            .map(|c| c.words.clone())
            .unwrap_or_else(|| ctx.to_owned())
    }

    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        mechanic: &str,
        p: Pattern,
        ctx: &str,
        tier: u8,
        name: &str,
        presses: Vec<String>,
        status: Status,
        reason: Option<String>,
    ) {
        if !self.has(ctx) {
            return;
        }
        let id = format!("{mechanic}:{}:{ctx}", key_of(&p));
        if self.out.iter().any(|c| c.id == id) {
            return;
        }
        let end = super::end_of(&presses);
        let frames = if p.event.is_some() {
            MAX_FRAMES
        } else if presses.is_empty() {
            p.frames.unwrap_or(60)
        } else {
            (end + SETTLE).min(MAX_FRAMES).max(end)
        };
        let name = format!("{name}, {}", self.words(ctx));
        self.out.push(Cell {
            id,
            mechanic: mechanic.to_owned(),
            pattern: p,
            context: ctx.to_owned(),
            tier,
            name: capital(&name),
            presses,
            frames,
            status,
            reason,
            requires: Vec::new(),
            excites: Vec::new(),
            enters: Vec::new(),
            check: None,
        });
    }

    fn anchor(&self, name: &str) -> Option<u64> {
        self.anchors.get(name).map(|a| a.frames)
    }
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// A pseudo-random generator for the `mixed` cells: xorshift64*, seeded.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// Seeded pseudo-random presses over `buttons` for `frames` frames, each
/// press held 4 to 16 frames with gaps of 0 to 8.
fn mixed(seed: u64, buttons: &[String], frames: u64) -> Vec<String> {
    let mut r = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let mut out = Vec::new();
    let mut at = 0;
    while at + 4 < frames && !buttons.is_empty() {
        let b = &buttons[r.below(buttons.len() as u64) as usize];
        let len = 4 + r.below(13);
        let len = len.min(frames - at);
        out.push(span(b, at, len));
        at += len + r.below(9);
    }
    out
}

/// The cells of the model, sorted, with their `requires`: every cell the
/// rules make, the pruned ones with their reason. `anchors` are the
/// original's timings measured so far; a cell whose hold needs one not
/// measured yet leaves it out.
pub fn cells(f: &Facts, contexts: &[Context], anchors: &BTreeMap<String, Anchor>) -> Vec<Cell> {
    let mut g = Gen {
        anchors,
        contexts,
        out: Vec::new(),
    };
    let fwd = "right";
    let back = "left";
    let jb = f.jump_button.clone();
    let rb = f.run_button.clone();
    let top = g.anchor(TOP);

    // Mapping checks, on the original alone: a wrong measure makes every
    // fit wrong, so these come first.
    if f.run && f.vx_mapped {
        g.add(
            "check.mapping",
            pat("check", "x-by-vx", &[], None),
            "ground.walk_top",
            0,
            "Check the mapping: x moves by vx every frame at walking top speed",
            vec![span(fwd, 0, 32)],
            Status::Check,
            None,
        );
    }
    if f.jump.is_some() && f.vy_mapped {
        let n = top.unwrap_or(16);
        g.add(
            "check.mapping",
            pat("check", "y-by-vy", &[], None),
            "ground.still",
            0,
            "Check the mapping: y moves by vy every frame of a held jump",
            vec![span(&jb, 0, n)],
            Status::Check,
            None,
        );
        g.add(
            "check.mapping",
            pat("check", "ground-vy", &[], None),
            "ground.still",
            0,
            "Check the mapping: vy is 0 whenever on_ground",
            vec![span(&jb, 0, n)],
            Status::Check,
            None,
        );
    }

    // Nothing pressed, in each context a recording can give.
    for c in contexts.iter().filter(|c| c.lead.is_none()) {
        if matches!(c.id.as_str(), "ground.walk_top" | "ground.run_top") {
            continue;
        }
        let tier = if c.id == "ground.still" { 1 } else { 2 };
        g.add(
            "core.idle",
            pat("idle", "", &[], Some(60)),
            &c.id,
            tier,
            "Nothing pressed for 60 frames",
            Vec::new(),
            Status::Active,
            None,
        );
    }

    if let Some(j) = &f.jump {
        let m = "platformer.jump";
        g.add(
            m,
            pat("tap", "jump", &[], None),
            "ground.still",
            1,
            &format!("{}, tapped for 1 frame", capital(j)),
            vec![span(&jb, 0, 1)],
            Status::Active,
            None,
        );
        for n in HOLDS {
            let (status, reason) = match top {
                Some(t) if n > t => (
                    Status::Pruned,
                    Some(format!(
                        "dominated: held past the top of the rise (the original's is at {t} frames held), the same as held to the top until the landing"
                    )),
                ),
                _ => (Status::Active, None),
            };
            g.add(
                m,
                pat("hold", "jump", &[], Some(n)),
                "ground.still",
                1,
                &format!("{}, held {n} frames", capital(j)),
                vec![span(&jb, 0, n)],
                status,
                reason,
            );
        }
        if let Some(t) = top
            && !HOLDS.contains(&t)
        {
            let mut p = pat("hold", "jump", &[], None);
            p.anchor = Some(TOP.into());
            g.add(
                m,
                p,
                "ground.still",
                1,
                &format!(
                    "{}, held to the top of the rise ({t} frames on the original)",
                    capital(j)
                ),
                vec![span(&jb, 0, t)],
                Status::Active,
                None,
            );
        }
        g.add(
            m,
            pat("taps", "jump", &[], Some(2)),
            "ground.still",
            1,
            &format!(
                "{}, tapped, released for 5 frames, tapped again",
                capital(j)
            ),
            vec![span(&jb, 0, 1), span(&jb, 6, 1)],
            Status::Active,
            None,
        );
        g.add(
            m,
            pat("through", "jump", &[], Some(80)),
            "ground.still",
            2,
            &format!("{}, held 80 frames, through the landing", capital(j)),
            vec![span(&jb, 0, 80)],
            Status::Active,
            None,
        );
        let mut p = pat("again", "jump", &[], None);
        p.event = Some(EventAt {
            event: "land".into(),
            offset: 0,
            button: jb.clone(),
            len: 5,
        });
        g.add(
            m,
            p,
            "ground.still",
            2,
            &format!(
                "{}, tapped, then again on the original's landing frame",
                capital(j)
            ),
            vec![span(&jb, 0, 1)],
            Status::Active,
            None,
        );
        let mut p = pat("again", "jump", &[], None);
        p.event = Some(EventAt {
            event: "leave".into(),
            offset: 6,
            button: jb.clone(),
            len: 5,
        });
        let (st, why) = if f.double_jump {
            (Status::Active, None)
        } else {
            (
                Status::Check,
                Some(
                    "no mechanic of the player jumps in the air: checks that nothing happens"
                        .to_owned(),
                ),
            )
        };
        g.add(
            m,
            p,
            "air.rising",
            2,
            &format!("{}, then pressed again 6 frames after takeoff", capital(j)),
            vec![span(&jb, 0, 5)],
            st,
            why,
        );
        let mut p = pat("early", "jump", &[], Some(3));
        p.event = Some(EventAt {
            event: "land".into(),
            offset: -3,
            button: jb.clone(),
            len: 5,
        });
        let (st, why) = if f.buffer {
            (Status::Active, None)
        } else {
            (
                Status::Check,
                Some(
                    "the jump has no buffer_frames: checks that an early press is ignored"
                        .to_owned(),
                ),
            )
        };
        g.add(
            m,
            p,
            "air.falling",
            2,
            &format!(
                "{}, tapped, then pressed 3 frames before the original lands",
                capital(j)
            ),
            vec![span(&jb, 0, 1)],
            st,
            why,
        );
        if f.run {
            for k in [1u64, 3] {
                let mut p = pat("late", "jump", &[], Some(k));
                p.event = Some(EventAt {
                    event: "leave".into(),
                    offset: k as i64,
                    button: jb.clone(),
                    len: 5,
                });
                let (st, why) = if f.coyote {
                    (Status::Active, None)
                } else {
                    (
                        Status::Check,
                        Some(
                            "the jump has no coyote_frames: checks that a late press does nothing"
                                .to_owned(),
                        ),
                    )
                };
                g.add(
                    m,
                    p,
                    "ground.ledge",
                    2,
                    &format!("Walk off, then {j} {k} frames after leaving the ground"),
                    vec![span(fwd, 0, 64)],
                    st,
                    why,
                );
            }
        }
        // Each speed band, and either side of each band edge.
        let n = f.bands.len();
        for i in 1..=n {
            let ctx = format!("ground.band[{i}]");
            g.add(
                m,
                pat("tap", "jump", &[], None),
                &ctx,
                3,
                &format!("{}, tapped", capital(j)),
                vec![span(&jb, 0, 1)],
                Status::Active,
                None,
            );
            for h in [12u64, 64] {
                g.add(
                    m,
                    pat("hold", "jump", &[], Some(h)),
                    &ctx,
                    3,
                    &format!("{}, held {h} frames", capital(j)),
                    vec![span(&jb, 0, h)],
                    Status::Active,
                    None,
                );
            }
        }
        for i in 0..n {
            for side in ["under", "over"] {
                g.add(
                    m,
                    pat("hold", "jump", &[], Some(64)),
                    &format!("ground.edge[{i}].{side}"),
                    3,
                    &format!("{}, held 64 frames", capital(j)),
                    vec![span(&jb, 0, 64)],
                    Status::Active,
                    None,
                );
            }
        }
    }

    if f.run {
        let m = "platformer.run";
        for n in HOLDS {
            g.add(
                m,
                pat("hold", "fwd", &[], Some(n)),
                "ground.still",
                1,
                &format!("Forward, held {n} frames"),
                vec![span(fwd, 0, n)],
                Status::Active,
                None,
            );
        }
        g.add(
            m,
            pat("hold", "back", &[], Some(16)),
            "ground.still",
            1,
            "Back, held 16 frames (the mirror check)",
            vec![span(back, 0, 16)],
            Status::Active,
            None,
        );
        if let Some(r) = &f.run_action {
            for n in [8u64, 16, 32, 64] {
                g.add(
                    m,
                    pat("hold", "fwd", &[r], Some(n)),
                    "ground.still",
                    1,
                    &format!("Forward with {r}, held {n} frames"),
                    vec![span(&format!("{fwd}+{rb}"), 0, n)],
                    Status::Active,
                    None,
                );
            }
            g.add(
                m,
                pat("tap", r, &[], None),
                "ground.still",
                1,
                &format!("{}, tapped alone", capital(r)),
                vec![span(&rb, 0, 1)],
                Status::Check,
                Some(format!("{r} is read only with a direction held (run_action): checks that nothing happens")),
            );
            for n in [16u64, 64] {
                g.add(
                    m,
                    pat("hold", r, &[], Some(n)),
                    "ground.still",
                    1,
                    &format!("{}, held {n} frames alone", capital(r)),
                    vec![span(&rb, 0, n)],
                    Status::Pruned,
                    Some(format!("no effect by the record: {r} is read only with a direction held (run_action)")),
                );
            }
        }
        g.add(
            m,
            pat("release", "", &[], Some(60)),
            "ground.walk_top",
            2,
            "Everything released",
            Vec::new(),
            Status::Active,
            None,
        );
        g.add(
            m,
            pat("hold", "fwd", &[], Some(32)),
            "ground.walk_top",
            2,
            "Forward held on, 32 frames",
            vec![span(fwd, 0, 32)],
            Status::Active,
            None,
        );
        g.add(
            m,
            pat("doublet", "back", &[], Some(32)),
            "ground.walk_top",
            2,
            "Back held 32 frames: a skid and turn",
            vec![span(back, 0, 32)],
            Status::Active,
            None,
        );
        g.add(
            m,
            pat("tap", "back", &[], None),
            "ground.walk_top",
            2,
            "Back tapped for 1 frame",
            vec![span(back, 0, 1)],
            Status::Active,
            None,
        );
        if let Some(r) = &f.run_action {
            g.add(
                m,
                pat("release", r, &["fwd"], Some(32)),
                "ground.run_top",
                2,
                &format!("Forward held, {r} released"),
                vec![span(fwd, 0, 32)],
                Status::Active,
                None,
            );
            g.add(
                m,
                pat("release", "", &[], Some(60)),
                "ground.run_top",
                2,
                "Everything released",
                Vec::new(),
                Status::Active,
                None,
            );
            g.add(
                m,
                pat("doublet", "back", &[r], Some(32)),
                "ground.run_top",
                2,
                &format!("Back with {r}, held 32 frames: a skid and turn"),
                vec![span(&format!("{back}+{rb}"), 0, 32)],
                Status::Active,
                None,
            );
        }
        if f.jump.is_some() && f.air_control {
            g.add(
                m,
                pat("turn", "back", &["jump"], None),
                "air.rising",
                2,
                "Forward and jump, then back from frame 20",
                vec![span(fwd, 0, 20), span(&jb, 0, 15), span(back, 20, 20)],
                Status::Active,
                None,
            );
            g.add(
                m,
                pat("air", "fwd", &["jump"], None),
                "air.falling",
                2,
                "Jump straight up, then forward from frame 20",
                vec![span(&jb, 0, 15), span(fwd, 20, 30)],
                Status::Active,
                None,
            );
        }
        g.add(
            m,
            pat("hold", "fwd", &[], Some(64)),
            "ground.ledge",
            2,
            "Forward held 64 frames, off the ledge",
            vec![span(fwd, 0, 64)],
            Status::Active,
            None,
        );
        for s in &f.states {
            g.add(
                m,
                pat("hold", "fwd", &[], Some(32)),
                &format!("state[{s}]"),
                2,
                "Forward held 32 frames",
                vec![span(fwd, 0, 32)],
                Status::Active,
                None,
            );
        }
    }
    if let Some(j) = &f.jump {
        for s in &f.states {
            g.add(
                "platformer.jump",
                pat("hold", "jump", &[], Some(32)),
                &format!("state[{s}]"),
                2,
                &format!("{}, held 32 frames", capital(j)),
                vec![span(&jb, 0, 32)],
                Status::Active,
                None,
            );
        }
    }

    for e in &f.enemies {
        let ctx = format!("enemy.near[{e}]");
        let m = "platformer.contact";
        g.add(
            m,
            pat("hold", "fwd", &[], Some(64)),
            &ctx,
            2,
            "Forward held 64 frames, into it",
            vec![span(fwd, 0, 64)],
            Status::Active,
            None,
        );
        if f.jump.is_some() {
            for at in [0u64, 8] {
                g.add(
                    m,
                    pat("stomp", "jump", &["fwd"], Some(at)),
                    &ctx,
                    2,
                    &format!("Forward held, jump held from frame {at}: onto it"),
                    vec![span(fwd, 0, 64), span(&jb, at, 32)],
                    Status::Active,
                    None,
                );
            }
        }
    }

    for (a, b, only) in &f.exit_actions {
        g.add(
            "core.exits",
            pat("hold", a, &[], Some(32)),
            "exit",
            2,
            &format!("{}, held 32 frames", capital(a)),
            vec![span(b, 0, 32)],
            Status::Active,
            None,
        );
        if !only {
            continue;
        }
        g.add(
            "core.exits",
            pat("hold", a, &[], Some(16)),
            "ground.still",
            1,
            &format!("{}, held 16 frames", capital(a)),
            vec![span(b, 0, 16)],
            Status::Pruned,
            Some(format!(
                "no effect by the record: no mechanic reads {a} outside an exit's box"
            )),
        );
    }
    for (a, b) in &f.unread {
        g.add(
            "core.none",
            pat("hold", a, &[], Some(16)),
            "ground.still",
            1,
            &format!("{}, held 16 frames", capital(a)),
            vec![span(b, 0, 16)],
            Status::Pruned,
            Some(format!("no effect by the record: no mechanic reads {a}")),
        );
    }

    // Two buttons together.
    if let (Some(j), Some(r)) = (&f.jump, &f.run_action) {
        g.add(
            "platformer.jump",
            pat("chord", "jump", &[r], Some(16)),
            "ground.still",
            2,
            &format!("{} and {r} together, held 16 frames", capital(j)),
            vec![span(&format!("{jb}+{rb}"), 0, 16)],
            Status::Active,
            None,
        );
    }

    // Probes for moves the genre has and the model lacks: run on both
    // games, never fitted.
    if f.genre.iter().any(|g| g == "platformer") {
        if f.jump.is_some() && !f.water {
            for k in [1u64, 3, 6] {
                let presses = (0..k).map(|i| span(&jb, i * 8, 1)).collect();
                g.add(
                    "probe.swim",
                    pat("taps", "jump", &[], Some(k)),
                    "water",
                    4,
                    &format!(
                        "Jump tapped {k} times (the model has no swimming: does the original swim?)"
                    ),
                    presses,
                    Status::Probe,
                    Some("the model has no water mechanic".into()),
                );
            }
        }
        if !f.shoot
            && let Some(r) = &f.run_action
        {
            for s in &f.states {
                g.add(
                    "probe.shoot",
                    pat("tap", r, &[], None),
                    &format!("state[{s}]"),
                    4,
                    &format!(
                        "{} tapped (the model shoots nothing: does the original?)",
                        capital(r)
                    ),
                    vec![span(&rb, 0, 1)],
                    Status::Probe,
                    Some("the model has no shoot mechanic".into()),
                );
            }
        }
        if let Some((a, b, _)) = f
            .exit_actions
            .iter()
            .find(|(_, b, only)| b == "down" && *only)
        {
            for ctx in std::iter::once("ground.still".to_owned())
                .chain(f.states.iter().map(|s| format!("state[{s}]")))
            {
                g.add(
                    "probe.crouch",
                    pat("hold", a, &[], Some(32)),
                    &ctx,
                    4,
                    &format!(
                        "{} held 32 frames (the model has no crouch: does the original?)",
                        capital(a)
                    ),
                    vec![span(b, 0, 32)],
                    Status::Probe,
                    Some("the model has no crouch mechanic".into()),
                );
            }
        }
    }

    // Seeded pseudo-random presses over every button a mechanic reads.
    let mut buttons: Vec<String> = Vec::new();
    if f.run {
        buttons.extend([fwd.to_owned(), back.to_owned()]);
        if !rb.is_empty() {
            buttons.push(format!("{fwd}+{rb}"));
        }
    }
    if !jb.is_empty() {
        buttons.push(jb.clone());
        if f.run {
            buttons.push(format!("{fwd}+{jb}"));
        }
    }
    if !buttons.is_empty() {
        for k in 1..=2u64 {
            g.add(
                "mixed",
                pat("mixed", "", &[], Some(k)),
                "ground.still",
                4,
                &format!("Seeded presses {k} (seed {})", super::SEED),
                mixed(super::SEED * 1000 + k, &buttons, MAX_FRAMES - SETTLE),
                Status::Active,
                None,
            );
        }
    }

    let mut out = g.out;
    even_lengths(&mut out);
    out.sort_by(|a, b| {
        (
            a.tier,
            &a.mechanic,
            context_order(&a.context),
            pattern_order(&a.pattern.name),
            a.pattern.frames.unwrap_or(0),
            &a.id,
        )
            .cmp(&(
                b.tier,
                &b.mechanic,
                context_order(&b.context),
                pattern_order(&b.pattern.name),
                b.pattern.frames.unwrap_or(0),
                &b.id,
            ))
    });
    link(&mut out, f);
    out
}

/// Gives every cell of one mechanic in one context the same length, the
/// longest of them, so two cells' outcomes on the original compare frame
/// for frame.
fn even_lengths(cells: &mut [Cell]) {
    let mut longest: BTreeMap<(String, String), u64> = BTreeMap::new();
    for c in cells.iter().filter(|c| c.pattern.event.is_none()) {
        let e = longest
            .entry((c.mechanic.clone(), c.context.clone()))
            .or_insert(0);
        *e = (*e).max(c.frames);
    }
    for c in cells.iter_mut().filter(|c| c.pattern.event.is_none()) {
        if let Some(n) = longest.get(&(c.mechanic.clone(), c.context.clone())) {
            c.frames = *n;
        }
    }
}

/// Fills `requires`: nothing pressed while standing first, then each
/// action's own cell from standing: a tap of the jump, a hold forward, a
/// hold forward with run.
fn link(cells: &mut [Cell], f: &Facts) {
    let ids: BTreeSet<String> = cells
        .iter()
        .filter(|c| c.status != Status::Pruned)
        .map(|c| c.id.clone())
        .collect();
    let idle = "core.idle:idle-60:ground.still".to_owned();
    let mut base: Vec<(String, String)> = Vec::new();
    if let Some(j) = &f.jump {
        let _ = j;
        base.push((
            "jump".into(),
            "platformer.jump:jump.tap:ground.still".into(),
        ));
    }
    if f.run {
        base.push((
            "fwd".into(),
            "platformer.run:fwd.hold-16:ground.still".into(),
        ));
        if let Some(r) = &f.run_action {
            base.push((
                r.clone(),
                format!("platformer.run:fwd+{r}.hold-16:ground.still"),
            ));
        }
    }
    for c in cells.iter_mut() {
        if c.tier == 0 || c.status == Status::Pruned || c.id == idle {
            continue;
        }
        let mut req = Vec::new();
        if ids.contains(&idle) {
            req.push(idle.clone());
        }
        let mut acts: Vec<String> = Vec::new();
        if !c.pattern.action.is_empty() {
            acts.push(c.pattern.action.clone());
        }
        acts.extend(c.pattern.with.iter().cloned());
        if c.context.starts_with("ground.band") || c.context.starts_with("ground.edge") {
            acts.push("fwd".into());
        }
        let alone = acts.len() == 1;
        for a in acts {
            for (ba, id) in &base {
                if *ba == a && *id != c.id && ids.contains(id) && !req.contains(id) {
                    // A single action's own cells from standing need only
                    // nothing pressed.
                    if alone && c.context == "ground.still" {
                        continue;
                    }
                    req.push(id.clone());
                }
            }
        }
        c.requires = req;
    }
}

/// The identical-outcome rule: of active cells in one context whose
/// outcome on the original (`digest`, by cell, from the context's first
/// start) is the same frame for frame, the first in the corpus's order
/// (tier, mechanic, context, pattern, length) stays and the rest are
/// pruned with the reason. So a jump tapped alone stays, and the same
/// jump with the run button, which changes nothing standing still, goes.
pub fn prune_identical(cells: &mut [Cell], digest: &BTreeMap<String, String>) {
    let mut first: BTreeMap<(String, String), String> = BTreeMap::new();
    for c in cells.iter_mut() {
        if c.status != Status::Active {
            continue;
        }
        let Some(d) = digest.get(&c.id) else { continue };
        let k = (c.context.clone(), d.clone());
        match first.get(&k) {
            Some(keep) => {
                c.status = Status::Pruned;
                c.reason = Some(format!(
                    "the same on the original, frame for frame, as {keep}"
                ));
            }
            None => {
                first.insert(k, c.id.clone());
            }
        }
    }
}

/// The contexts and cells of a model, before any start is found:
/// [`facts`], [`contexts`], [`cells`].
pub fn derive(
    model: &Model,
    catalog: &Catalog,
    mapping: &Mapping,
    anchors: &BTreeMap<String, Anchor>,
) -> Option<(Facts, Vec<Context>, Vec<Cell>)> {
    let f = facts(model, catalog, mapping)?;
    let cx = contexts(&f);
    let cs = cells(&f, &cx, anchors);
    Some((f, cx, cs))
}
