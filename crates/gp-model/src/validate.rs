//! Checks a parsed model beyond its types: names, references between parts,
//! roles, events and actions, the extensions it uses, and evidence for
//! every fact. Each problem names the model path it's about.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path as FsPath;

use crate::model::{AssetKind, CORE_SYSTEMS, CORE_VERSION, Machine, Model, Source};
use crate::path::{self, Concrete, Path};
use crate::platformer::{self, Extension};

/// Roles an entity can have.
pub const ENTITY_ROLES: &[&str] = &[
    "player",
    "enemy",
    "boss",
    "projectile",
    "npc",
    "collectible",
    "power-up",
    "hazard",
    "interactive",
    "goal",
];

/// Roles of parts of the world and the interface, for items within an
/// asset (each metatile of a tileset). Entity roles are allowed too.
pub const WORLD_ROLES: &[&str] = &[
    "terrain",
    "platform",
    "background",
    "sky",
    "water",
    "hud",
    "menu",
    "font",
];

/// Roles an asset slot can have, besides `music.<mood>` and `sfx.<event>`.
pub const SLOT_ROLES: &[&str] = &[
    "graphics", "palettes", "tileset", "layout", "spawns", "font", "behavior", "hud",
];

/// The role groups themes and the readability check key on (ARCHITECTURE.md,
/// "Themes"), each with its entity and world roles. Sound roles
/// (`music.<mood>`, `sfx.<event>`) are the group `sound`.
pub const ROLE_GROUPS: &[(&str, &[&str])] = &[
    ("actors", &["player", "enemy", "boss", "projectile", "npc"]),
    (
        "things",
        &["collectible", "power-up", "hazard", "interactive", "goal"],
    ),
    (
        "world",
        &["terrain", "platform", "background", "sky", "water"],
    ),
    ("interface", &["hud", "menu", "font"]),
];

/// Feature `clip.when_x`: a condition's x range, when both bounds are
/// given, holds some x.
fn x_range(c: &mut Checker, at: &str, w: &crate::model::When) {
    if let (Some(a), Some(b)) = (w.x_at_least, w.x_below)
        && a >= b
    {
        c.add(
            format!("{at}.x_below"),
            "is above x_at_least, or no x is in the range",
        );
    }
}

/// The group of a role, or None for a slot role that isn't in one
/// (`graphics`, `layout` and the like).
pub fn role_group(role: &str) -> Option<&'static str> {
    if role.starts_with("music.") || role.starts_with("sfx.") {
        return Some("sound");
    }
    ROLE_GROUPS
        .iter()
        .find(|(_, roles)| roles.contains(&role))
        .map(|(g, _)| *g)
}

/// Kinds of slot whose items each need a role from core 0.3.1 on: each
/// metatile of a tileset, and each palette of a palettes slot. These are
/// what a theme recolors and the readability check measures, so a model
/// that leaves them out can't be themed by role.
pub const ITEM_ROLE_KINDS: &[AssetKind] = &[AssetKind::Metatiles, AssetKind::Palettes];

/// The core version from which [`ITEM_ROLE_KINDS`] slots must give
/// `item_roles`. A model naming 0.3.0 is read as before.
pub const ITEM_ROLES_FROM: &str = "0.3.1";

/// True when version `v` is `at` or later (`major.minor`, with a patch
/// read as 0 when it's left out).
pub fn version_at_least(v: &str, at: &str) -> bool {
    let parts = |v: &str| -> Option<(u32, u32, u32)> {
        let mut it = v.split('.').map(|p| p.parse::<u32>().ok());
        Some((it.next()??, it.next()??, it.next().unwrap_or(Some(0))?))
    };
    matches!((parts(v), parts(at)), (Some(a), Some(b)) if a >= b)
}

/// Actions the core defines for transitions.
pub const CORE_ACTIONS: &[&str] = &["restart-level", "next-level", "reset-game", "enter-level"];

/// Actions only an entity's own state machine can take.
pub const ENTITY_ACTIONS: &[&str] = &["remove"];

/// The entity action that replaces an instance by one of another entity:
/// `become:<entity>`.
pub const BECOME: &str = "become:";

/// The action that changes a counter: `change:<counter>:<by>`, `by` a whole
/// number, such as `change:lives:-1`.
pub const CHANGE: &str = "change:";

/// The action that fires an event (feature `action.emit`): `emit:<event>`, an event
/// of the core, a counter, a declared extension or the model's own.
pub const EMIT: &str = "emit:";

/// The action that adds one counter's value to another (feature
/// `action.add`): `add:<to>:<from>`, as Donkey Kong adds the bonus timer to
/// the score when a stage is cleared.
pub const ADD: &str = "add:";

/// An `add:` action's two counters, the one changed and the one added.
pub fn add_action(a: &str) -> Option<(&str, &str)> {
    let (to, from) = a.strip_prefix(ADD)?.split_once(':')?;
    (!to.is_empty() && !from.is_empty()).then_some((to, from))
}

/// The action that sets a counter (feature `action.set`):
/// `set:<counter>:<value>`, as Donkey Kong's stage setup puts the bonus
/// timer back at 5000 during the clear scene.
pub const SET: &str = "set:";

/// The action that takes every active body of an entity out of the level
/// at once, with no event (feature `action.clear`): `clear:<entity>`, as
/// Donkey Kong's death wait redraws the screen and its barrels are gone.
pub const CLEAR: &str = "clear:";

/// The action that puts every active body of an entity back at its spawn's
/// place, at rest, its state kept (feature `action.respawn`):
/// `respawn:<entity>`, as Donkey Kong draws Mario at his start late in a
/// death's wait, before the restart.
pub const RESPAWN: &str = "respawn:";
/// The action that takes a body out of the level by a counter (feature
/// `action.remove_body`): `remove-body:<counter>`, the body whose ID plus
/// one the counter holds, as Donkey Kong takes away the barrel the hammer
/// smashed when the smash ends.
pub const REMOVE_BODY: &str = "remove-body:";

/// A `set:` action's counter and value.
pub fn set_action(a: &str) -> Option<(&str, i64)> {
    let (counter, v) = a.strip_prefix(SET)?.split_once(':')?;
    Some((counter, v.parse().ok()?))
}

/// A `change:` action's counter and amount.
pub fn change_action(a: &str) -> Option<(&str, i64)> {
    let (counter, by) = a.strip_prefix(CHANGE)?.split_once(':')?;
    Some((counter, by.parse().ok()?))
}

/// Events the core fires. Counters add `<name>.reached_min` and
/// `<name>.wrapped` (see [`counter_events`]).
pub const CORE_EVENTS: &[&str] = &["level.started"];

/// The events a model's counters fire: `<name>.reached_min` for one with a
/// `min`, `<name>.wrapped` for one that wraps.
pub fn counter_events(model: &Model) -> Vec<String> {
    let mut out = Vec::new();
    for (name, c) in &model.counters {
        if c.min.is_some() {
            out.push(format!("{name}.reached_min"));
        }
        if c.wrap {
            out.push(format!("{name}.wrapped"));
        }
    }
    out
}

/// Extensions this build knows.
pub const EXTENSIONS: &[Extension] = &[platformer::EXTENSION, crate::topdown::EXTENSION];

/// One thing wrong with a model.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Problem {
    /// The model path it's about.
    pub path: String,
    pub message: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

struct Checker {
    problems: Vec<Problem>,
}

impl Checker {
    fn add(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.problems.push(Problem {
            path: path.into(),
            message: message.into(),
        });
    }

    fn name(&mut self, at: &str, name: &str) {
        if !path::is_name(name) {
            self.add(
                at,
                format!("`{name}` can't be a name in a model path: use letters, digits and `_`, not starting with a digit"),
            );
        }
    }
}

/// True when a model naming version `have` (such as "0.4" or, from
/// before features, "0.4.23") can be read by a reader of `want` (such as
/// "0.4"): the same major and minor version. A version bumps only for a
/// breaking change, so the patch says nothing a reader needs; an optional
/// part is a feature ([`crate::features`]).
pub fn version_supported(have: &str, want: &str) -> bool {
    let parts = |v: &str| -> Option<(u32, u32)> {
        let mut it = v.split('.').map(|p| p.parse::<u32>().ok());
        let r = (it.next()??, it.next()??);
        match (it.next(), it.next()) {
            (None, _) | (Some(Some(_)), None) => Some(r),
            _ => None,
        }
    };
    match (parts(have), parts(want)) {
        (Some(h), Some(w)) => h == w,
        _ => false,
    }
}

/// `major.minor` of a version such as "0.1.0".
fn minor(v: &str) -> String {
    v.split('.').take(2).collect::<Vec<_>>().join(".")
}

/// The model's features against the ones it lists ([`crate::features`]).
fn features(c: &mut Checker, model: &Model, has_platformer: bool) {
    use crate::features::{Piece, is_name};
    let list = &model.model.features;
    for (i, name) in list.iter().enumerate() {
        let at = format!("model.features[{i}]");
        if !is_name(name) {
            c.add(
                &at,
                format!("`{name}` isn't a feature name: lowercase words joined by `.` and `_`"),
            );
            continue;
        }
        match crate::feature_list::FEATURES.iter().find(|f| f.name == name) {
            None => c.add(&at, crate::features::refusal("the model", &[name.as_str()])),
            Some(f) if f.piece == Piece::Platformer && !has_platformer => c.add(
                &at,
                format!("`{name}` is a platformer feature, and the model doesn't declare the platformer extension"),
            ),
            Some(_) => {}
        }
        if list[..i].contains(name) {
            c.add(&at, format!("`{name}` is listed twice"));
        }
    }
    if !list.is_empty() {
        for f in model.features_used() {
            if !list.iter().any(|n| n == f) {
                c.add(
                    "model.features",
                    format!("the model uses `{f}` and doesn't list it"),
                );
            }
        }
    }
}

/// True when `part` is a fact or holds one, so a rule or evidence entry
/// naming it covers something. Parts inside a list of plain values, and
/// descriptions, aren't facts.
pub fn holds_facts(facts: &[Concrete], part: &[path::Step]) -> bool {
    facts.iter().any(|f| f.starts_with(part))
}

/// Every problem with the model, sorted by path. Empty when it's valid.
pub fn validate(model: &Model) -> Vec<Problem> {
    let mut c = Checker {
        problems: Vec::new(),
    };
    let header = &model.model;
    if !version_supported(&header.schema, CORE_VERSION) {
        c.add(
            "model.schema",
            format!(
                "core schema {} isn't supported; this build reads {CORE_VERSION}",
                header.schema
            ),
        );
    }
    c.name("model.name", &header.name);
    if let Some(p) = &header.profile {
        // The switches on, joined by `+` (profiles 0.2.0).
        for part in p.split(crate::profile::JOIN) {
            c.name("model.profile", part);
        }
        if p == crate::profile::BASE {
            c.add(
                "model.profile",
                "the base profile is the model without `profile`",
            );
        }
    }

    // Extensions: known, at a version this build reads.
    let mut declared: Vec<&Extension> = Vec::new();
    for (name, version) in &header.extensions {
        let at = format!("model.extensions.{name}");
        match EXTENSIONS.iter().find(|e| e.name == name) {
            None => c.add(at, format!("no extension named `{name}` is known")),
            Some(e) if !version_supported(version, e.version) => c.add(
                at,
                format!(
                    "{name} {version} isn't supported; this build reads {}",
                    e.version
                ),
            ),
            Some(e)
                if !version_supported(
                    &format!("{}.0", e.core),
                    &format!("{}.0", minor(&header.schema)),
                ) =>
            {
                c.add(
                    at,
                    format!(
                        "{name} {} works with core {}, and the model is core {}",
                        e.version, e.core, header.schema
                    ),
                )
            }
            Some(e) => declared.push(e),
        }
    }
    let has_platformer = declared.iter().any(|e| e.name == platformer::NAME);
    let has_topdown = declared.iter().any(|e| e.name == crate::topdown::NAME);
    topdown_checks(&mut c, model, has_topdown);

    // Features: each listed one known, of a piece the model uses, and,
    // when there's a list, every feature the model uses on it.
    features(&mut c, model, has_platformer);
    let counter_events = counter_events(model);
    let builtin: BTreeSet<&str> = CORE_EVENTS
        .iter()
        .copied()
        .chain(declared.iter().flat_map(|e| e.events.iter().copied()))
        .chain(counter_events.iter().map(String::as_str))
        .collect();
    // The model's own events: dotted names, each once, none of them
    // another event's name.
    let mut own = BTreeSet::new();
    for (i, e) in header.events.iter().enumerate() {
        let ok = e.split('.').count() >= 2 && e.split('.').all(path::is_name);
        if !ok {
            c.add(
                format!("model.events[{i}]"),
                format!("`{e}` isn't a dotted event name such as `ladder.grabbed`"),
            );
        } else if builtin.contains(e.as_str()) {
            c.add(
                format!("model.events[{i}]"),
                format!("`{e}` is already an event of the core, a counter or an extension"),
            );
        } else if !own.insert(e.as_str()) {
            c.add(
                format!("model.events[{i}]"),
                format!("`{e}` is listed twice"),
            );
        }
    }
    let events: BTreeSet<&str> = builtin.iter().copied().chain(own).collect();
    let actions: BTreeSet<&str> = CORE_ACTIONS
        .iter()
        .copied()
        .chain(declared.iter().flat_map(|e| e.actions.iter().copied()))
        .collect();
    let params: BTreeSet<&str> = declared
        .iter()
        .flat_map(|e| e.spawn_params.iter().copied())
        .collect();

    // System order: every system of the core and the declared extensions,
    // once each.
    let known: Vec<&str> = CORE_SYSTEMS
        .iter()
        .copied()
        .chain(declared.iter().flat_map(|e| e.systems.iter().copied()))
        .collect();
    let mut seen = BTreeSet::new();
    for s in &header.systems {
        if !known.contains(&s.as_str()) {
            c.add(
                "model.systems",
                format!("`{s}` isn't a system of the core or a declared extension"),
            );
        } else if !seen.insert(s.as_str()) {
            c.add("model.systems", format!("`{s}` is listed twice"));
        }
    }
    let missing: Vec<&str> = known
        .iter()
        .copied()
        .filter(|s| !seen.contains(s))
        .collect();
    if !missing.is_empty() {
        c.add(
            "model.systems",
            format!(
                "every system runs in a stated order; add {}",
                missing
                    .iter()
                    .map(|s| format!("`{s}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
    }
    match &header.start_level {
        Some(l) if !model.levels.contains_key(l) => {
            c.add("model.start_level", format!("no level named `{l}`"))
        }
        None if model.levels.len() > 1 => c.add(
            "model.start_level",
            "a model with more than one level says which it starts in",
        ),
        _ => {}
    }

    // Input actions.
    let input_actions: BTreeSet<&str> = model
        .input
        .iter()
        .flat_map(|i| i.actions.keys().map(String::as_str))
        .collect();
    for a in &input_actions {
        c.name(&format!("input.actions.{a}"), a);
    }
    if let Some(i) = &model.input {
        for (a, b) in &i.actions {
            if i.device.bit(*b).is_none() {
                c.add(
                    format!("input.actions.{a}"),
                    format!("the input device has no button {b:?}"),
                );
            }
        }
    }
    if let Some(t) = model.input.as_ref().and_then(|i| i.turn.as_ref()) {
        if !model.counters.contains_key(&t.counter) {
            c.add(
                "input.turn.counter",
                format!("no counter named `{}`", t.counter),
            );
        }
        for a in &t.actions {
            if !input_actions.contains(a.as_str()) {
                c.add(
                    "input.turn.actions",
                    format!("the input has no action `{a}`"),
                );
            }
        }
    }
    if let Some(st) = &model.stages {
        if st.order.is_empty() {
            c.add("stages.order", "lists no stage");
        }
        if !model.levels.contains_key(&st.stand_in) {
            c.add(
                "stages.stand_in",
                format!("no level named `{}`", st.stand_in),
            );
        }
        if !model.counters.contains_key(&st.counter) {
            c.add(
                "stages.counter",
                format!("no counter named `{}`", st.counter),
            );
        }
        for s in &st.skip {
            if !st.order.contains(s) {
                c.add(
                    "stages.skip",
                    format!("`{s}` isn't a stage of stages.order"),
                );
            }
        }
        if !st.order.is_empty() && st.order.iter().all(|s| st.skip.contains(s)) {
            c.add("stages.skip", "skips every stage");
        }
    }

    // Asset slots.
    let slot_kind = |name: &str| model.assets.get(name).map(|s| s.kind);
    for (name, slot) in &model.assets {
        let at = format!("assets.{name}");
        c.name(&at, name);
        let role_ok = SLOT_ROLES.contains(&slot.role.as_str())
            || slot
                .role
                .strip_prefix("music.")
                .or_else(|| slot.role.strip_prefix("sfx."))
                .is_some_and(path::is_name);
        if !role_ok {
            c.add(
                format!("{at}.role"),
                format!("`{}` isn't a slot role", slot.role),
            );
        }
        if !slot.entities.is_empty() && slot.kind != AssetKind::SpawnObjects {
            c.add(
                format!("{at}.entities"),
                "only a spawn-objects slot places entities",
            );
        }
        let mut ids = BTreeSet::new();
        for (e, list) in &slot.entities {
            if !model.entities.contains_key(e) {
                c.add(
                    format!("{at}.entities.{e}"),
                    format!("no entity named `{e}`"),
                );
            }
            for id in list {
                if !ids.insert(*id) {
                    c.add(
                        format!("{at}.entities.{e}"),
                        format!("object kind {id} places more than one entity"),
                    );
                }
            }
        }
        if let Some(t) = &slot.tiles {
            // A screen-text slot (feature `assets.screen_tiles`), or a
            // level-objects slot a level's layout names (feature
            // `assets.layout_tiles`).
            if !matches!(slot.kind, AssetKind::ScreenText | AssetKind::LevelObjects) {
                c.add(
                    format!("{at}.tiles"),
                    "only a screen-text or level-objects slot names its tiles",
                );
            }
            if model
                .assets
                .get(t)
                .is_none_or(|s| s.kind != AssetKind::Tiles)
            {
                c.add(format!("{at}.tiles"), format!("`{t}` isn't a tiles slot"));
            }
        }
        if slot.item_roles.is_empty()
            && ITEM_ROLE_KINDS.contains(&slot.kind)
            && version_at_least(&header.schema, ITEM_ROLES_FROM)
        {
            c.add(
                format!("{at}.item_roles"),
                format!(
                    "a {} slot gives each item a role from core {ITEM_ROLES_FROM} on, for themes and the readability check",
                    slot.kind.map_format()
                ),
            );
        }
        for (i, r) in slot.item_roles.iter().enumerate() {
            if !WORLD_ROLES.contains(&r.as_str()) && !ENTITY_ROLES.contains(&r.as_str()) {
                c.add(
                    format!("{at}.item_roles"),
                    format!("item {i}: `{r}` isn't a world or entity role"),
                );
            }
        }
    }
    let want_slot = |c: &mut Checker, at: String, name: &str, kind: AssetKind| match slot_kind(name)
    {
        None => c.add(at, format!("no asset slot named `{name}`")),
        Some(k) if k != kind => c.add(
            at,
            format!("slot `{name}` is a {} slot, not {}", k.name(), kind.name()),
        ),
        Some(_) => {}
    };

    // Entities.
    for (name, e) in &model.entities {
        let at = format!("entities.{name}");
        c.name(&at, name);
        if !ENTITY_ROLES.contains(&e.core.role.as_str()) {
            c.add(
                format!("{at}.role"),
                format!("`{}` isn't an entity role", e.core.role),
            );
        }
        let [w, h] = e.core.size;
        if w <= crate::Fixed::ZERO || h <= crate::Fixed::ZERO {
            c.add(format!("{at}.size"), "width and height must be above zero");
        }
        if e.core.hook_frame_start && e.core.ext.is_none() {
            c.add(format!("{at}.hook_frame_start"), "needs an `ext` hook");
        }
        if e.core.hook_first && e.core.ext.is_none() {
            c.add(format!("{at}.hook_first"), "needs an `ext` hook");
        }
        if e.core.hook_last && e.core.ext.is_none() {
            c.add(format!("{at}.hook_last"), "needs an `ext` hook");
        }
        if e.core.hook_last && e.core.hook_first {
            c.add(
                format!("{at}.hook_last"),
                "a hook runs first or last, not both",
            );
        }
        if e.core.hook_through_stops && e.core.ext.is_none() {
            c.add(
                format!("{at}.hook_through_stops"),
                "only an entity with an `ext` hook has a hook to run through stops",
            );
        }
        if !e.core.ext_params.is_empty() && e.core.ext.is_none() {
            c.add(
                format!("{at}.ext_params"),
                "only an entity with an `ext` hook has ext_params",
            );
        }
        for k in e.core.ext_params.keys() {
            c.name(&format!("{at}.ext_params.{k}"), k);
        }
        if let Some(d) = e.core.despawn_behind
            && d < crate::Fixed::ZERO
        {
            c.add(format!("{at}.despawn_behind"), "must be zero or above");
        }
        if e.core
            .despawn_behind_wrap
            .is_some_and(|[from, _]| from > 255)
        {
            c.add(
                format!("{at}.despawn_behind_wrap"),
                "from is a place in a 256-pixel page",
            );
        }
        if e.core.despawn_ahead.is_some_and(|d| d < crate::Fixed::ZERO) {
            c.add(format!("{at}.despawn_ahead"), "must be zero or above");
        }
        // Since platformer 0.4.1 a component can read lifted tables too
        // (`orbit_part`'s quarter-turn table).
        if !e.core.ext_tables.is_empty()
            && e.core.ext.is_none()
            && e.platformer.orbit_part.is_none()
            && e.platformer.stream.is_none()
        {
            c.add(
                format!("{at}.ext_tables"),
                "only an entity with an `ext` hook, an `orbit_part` or a `stream` has ext_tables",
            );
        }
        if !e.core.ext_rows.is_empty() && e.core.ext.is_none() {
            c.add(
                format!("{at}.ext_rows"),
                "only an entity with an `ext` hook has ext_rows",
            );
        }
        for (k, v) in &e.core.ext_rows {
            let here = format!("{at}.ext_rows.{k}");
            c.name(&here, k);
            if e.core.ext_tables.contains_key(k) {
                c.add(&here, format!("`{k}` is in ext_tables too"));
            }
            want_slot(&mut c, here, v, AssetKind::Rows);
        }
        for (k, v) in &e.core.ext_tables {
            let here = format!("{at}.ext_tables.{k}");
            c.name(&here, k);
            match v.split_once('.') {
                Some((slot, column)) if !column.is_empty() => {
                    want_slot(&mut c, here.clone(), slot, AssetKind::Table)
                }
                _ => c.add(&here, format!("`{v}` isn't `<slot>.<column>`")),
            }
        }
        let hb = e.core.hitbox;
        if hb.w <= crate::Fixed::ZERO || hb.h <= crate::Fixed::ZERO {
            c.add(
                format!("{at}.hitbox"),
                "width and height must be above zero",
            );
        }
        if let Some(s) = &e.core.sprite {
            want_slot(
                &mut c,
                format!("{at}.sprite.slot"),
                &s.slot,
                AssetKind::Tiles,
            );
            match &s.frame {
                Some(f) => {
                    if !s.tiles.is_empty() {
                        c.add(format!("{at}.sprite"), "give tiles or frame, not both");
                    }
                    // A frame is a row of tile numbers, or (since core
                    // 0.4.2) a whole layout of a sprite-set.
                    let kind = match slot_kind(&f.slot) {
                        Some(AssetKind::SpriteSet) => AssetKind::SpriteSet,
                        _ => AssetKind::Rows,
                    };
                    want_slot(&mut c, format!("{at}.sprite.frame.slot"), &f.slot, kind);
                    if s.width == 0 {
                        c.add(format!("{at}.sprite.width"), "must be at least 1");
                    }
                }
                None => {
                    // A sprite that draws the body it replaced may have no
                    // tiles of its own: it draws nothing without one.
                    let none = s.tiles.is_empty() && s.replaced.is_some();
                    if s.width == 0
                        || (s.tiles.is_empty() && !none)
                        || s.tiles.len() % s.width as usize != 0
                    {
                        c.add(
                            format!("{at}.sprite.tiles"),
                            "there must be some tiles (none only with `replaced`), a whole number of rows of `width`",
                        );
                    }
                }
            }
            if let Some(p) = &s.colors {
                want_slot(
                    &mut c,
                    format!("{at}.sprite.colors.slot"),
                    &p.slot,
                    AssetKind::Palettes,
                );
            }
            for (i, clip) in s.clips.iter().enumerate() {
                let here = format!("{at}.sprite.clips[{i}]");
                for (j, f) in clip.frames.iter().enumerate() {
                    if let Some(p) = &f.colors {
                        want_slot(
                            &mut c,
                            format!("{here}.frames[{j}].colors.slot"),
                            &p.slot,
                            AssetKind::Palettes,
                        );
                    }
                }
                if clip.frames.is_empty() {
                    c.add(format!("{here}.frames"), "a clip needs at least one frame");
                }
                if clip.every == Some(0) || clip.by_speed.iter().any(|b| b.every == 0) {
                    c.add(&here, "a picture shows for at least one frame");
                }
                if clip.every.is_some() && !clip.by_speed.is_empty() {
                    c.add(&here, "give every or by_speed, not both");
                }
                if clip.frames.len() > 1
                    && clip.every.is_none()
                    && clip.by_speed.is_empty()
                    && !clip.hold
                {
                    c.add(
                        &here,
                        "a clip of more than one frame needs every or by_speed, or hold",
                    );
                }
                if !clip.by_speed.is_empty() && clip.clock == crate::model::Clock::Game {
                    c.add(&here, "by_speed needs the `own` or `body` clock");
                }
                for (j, f) in clip.frames.iter().enumerate() {
                    if f.row.is_some() && s.frame.is_none() {
                        c.add(
                            format!("{here}.frames[{j}].row"),
                            "a row needs the sprite's frame to name its slot",
                        );
                    }
                    if f.row.is_some() && !f.tiles.is_empty() {
                        c.add(format!("{here}.frames[{j}]"), "give row or tiles, not both");
                    }
                }
            }
        }
        if let Some(hb) = e.core.tile_hitbox
            && (hb.w <= crate::Fixed::ZERO || hb.h <= crate::Fixed::ZERO)
        {
            c.add(
                format!("{at}.tile_hitbox"),
                "width and height must be above zero",
            );
        }
        if let Some(m) = &e.core.machine {
            for (sname, st) in &m.states {
                let sat = format!("{at}.machine.states.{sname}.set");
                for key in st.set.keys() {
                    if matches!(key.as_str(), "machine" | "role" | "ext" | "description") {
                        c.add(&sat, format!("a state can't set `{key}`"));
                    }
                }
                if let Err(msg) = e.in_state(sname) {
                    c.add(&sat, msg);
                }
            }
        }
        components(&mut c, &at, e, has_platformer, &input_actions);
        if let Some(d) = e.platformer.leap.as_ref().and_then(|l| l.draw.as_ref()) {
            match model
                .counters
                .get(&d.counter)
                .and_then(|k| k.shift.as_ref())
            {
                None => c.add(
                    format!("{at}.leap.draw.counter"),
                    format!("`{}` isn't a counter with a `shift`", d.counter),
                ),
                Some(s) if d.at + 8 > s.bits => c.add(
                    format!("{at}.leap.draw.at"),
                    "the byte isn't within the shift's bits",
                ),
                Some(_) => {}
            }
        }
        // Platformer 0.4.28's launcher.
        if let Some(l) = &e.platformer.launcher {
            let lat = format!("{at}.launcher");
            match model
                .counters
                .get(&l.counter)
                .and_then(|k| k.shift.as_ref())
            {
                None => c.add(
                    format!("{lat}.counter"),
                    format!("`{}` isn't a counter with a `shift`", l.counter),
                ),
                Some(s) if l.at + 8 > s.bits => c.add(
                    format!("{lat}.at"),
                    "the byte isn't within the shift's bits",
                ),
                Some(_) => {}
            }
            if !model.counters.contains_key(&l.next) {
                c.add(format!("{lat}.next"), format!("no counter `{}`", l.next));
            }
            match model.entities.get(&l.fires) {
                None => c.add(format!("{lat}.fires"), format!("no entity `{}`", l.fires)),
                Some(f) => {
                    if f.platformer.fly.is_none() {
                        c.add(
                            format!("{lat}.fires"),
                            format!("`{}` flies from the launcher: give it `fly`", l.fires),
                        );
                    }
                    if f.core.ext.is_some() || f.platformer.launcher.is_some() {
                        c.add(
                            format!("{lat}.fires"),
                            format!(
                                "`{}` keeps \"aimed\" in its mechanic word 0: no ext hook or launcher on it",
                                l.fires
                            ),
                        );
                    }
                }
            }
            if l.table == 0 || l.slots == 0 {
                c.add(lat.clone(), "table and slots must be at least 1");
            }
            if e.core.ext.is_some() || !e.core.no_slot {
                c.add(
                    lat.clone(),
                    "a launcher keeps its timer and entry in the body's mechanic words and takes no enemy slot: give it `no_slot` and no ext hook",
                );
            }
        }
        if let Some(s) = &e.platformer.stream {
            match model
                .counters
                .get(&s.draw.counter)
                .and_then(|k| k.shift.as_ref())
            {
                None => c.add(
                    format!("{at}.stream.draw.counter"),
                    format!("`{}` isn't a counter with a `shift`", s.draw.counter),
                ),
                Some(sh) if s.draw.at + 8 > sh.bits => c.add(
                    format!("{at}.stream.draw.at"),
                    "the byte isn't within the shift's bits",
                ),
                Some(_) => {}
            }
            for t in ["heights", "times"] {
                if !e.core.ext_tables.contains_key(t) {
                    c.add(
                        format!("{at}.ext_tables"),
                        format!("a stream reads the table `{t}` from its ext_tables"),
                    );
                }
            }
            if !e.core.no_slot {
                c.add(format!("{at}.stream"), "a stream's marker needs `no_slot`");
            }
            if !model.entities.contains_key(&s.entity) {
                c.add(
                    format!("{at}.stream.entity"),
                    format!("no entity `{}`", s.entity),
                );
            }
            if let Some(u) = &s.until
                && !model.entities.contains_key(u)
            {
                c.add(format!("{at}.stream.until"), format!("no entity `{u}`"));
            }
        }
        if let Some(beat) = e.platformer.leap.as_ref().and_then(|l| l.beat.as_ref())
            && !model
                .counters
                .get(beat)
                .is_some_and(|k| k.change.iter().any(|ch| ch.every.is_some()))
        {
            c.add(
                format!("{at}.leap.beat"),
                "must name a counter with an `every` change",
            );
        }
        if let Some(inv) = &e.platformer.invincible {
            if !model.counters.contains_key(&inv.while_.counter) {
                c.add(
                    format!("{at}.invincible.while.counter"),
                    format!("no counter `{}`", inv.while_.counter),
                );
            }
            if e.core.role != "player" {
                c.add(format!("{at}.invincible"), "only a player is invincible");
            }
        }
        if let Some(lk) = &e.platformer.locks {
            if !model.counters.contains_key(&lk.while_.counter) {
                c.add(
                    format!("{at}.locks.while.counter"),
                    format!("no counter `{}`", lk.while_.counter),
                );
            }
            if e.core.role != "player" {
                c.add(format!("{at}.locks"), "only a player's actions are locked");
            }
            for a in &lk.actions {
                if !model
                    .input
                    .as_ref()
                    .is_some_and(|i| i.actions.contains_key(a))
                {
                    c.add(format!("{at}.locks.actions"), format!("no action `{a}`"));
                }
            }
        }
        let p = &e.platformer;
        if let Some(h) = &p.health {
            if let Some(ctr) = &h.counter
                && !model.counters.contains_key(ctr)
            {
                c.add(
                    format!("{at}.health.counter"),
                    format!("no counter named `{ctr}`"),
                );
            }
            match (&h.counter, h.points, e.core.role == "player") {
                (Some(_), None, true) | (None, Some(_), false) => {}
                (_, _, true) => c.add(
                    format!("{at}.health"),
                    "the player's hit points are a counter: give `counter`, not `points`",
                ),
                _ => c.add(
                    format!("{at}.health"),
                    "an entity other than the player keeps its own hit points: give `points`, not `counter`",
                ),
            }
            for k in h.damage_from.keys() {
                if !model.entities.contains_key(k) {
                    c.add(
                        format!("{at}.health.damage_from.{k}"),
                        format!("no entity named `{k}`"),
                    );
                }
            }
            if let Some(d) = &h.defeat
                && !model.entities.contains_key(&d.becomes)
            {
                c.add(
                    format!("{at}.health.defeat.becomes"),
                    format!("no entity named `{}`", d.becomes),
                );
            }
        }
        if let Some(sh) = &p.shoot {
            match model.entities.get(&sh.entity) {
                None => c.add(
                    format!("{at}.shoot.entity"),
                    format!("no entity named `{}`", sh.entity),
                ),
                Some(t) if t.core.role == "player" => c.add(
                    format!("{at}.shoot.entity"),
                    "an entity can't spawn the player",
                ),
                Some(_) => {}
            }
        }
        if let Some(h) = &p.hits {
            for r in &h.roles {
                if !ENTITY_ROLES.contains(&r.as_str()) {
                    c.add(
                        format!("{at}.hits.roles"),
                        format!("`{r}` isn't an entity role"),
                    );
                }
            }
        }
        if p.carry.is_some() && e.core.role != "player" {
            c.add(format!("{at}.carry"), "only the player carries");
        }
        if (p.climb.is_some() || p.knockback.is_some()) && e.core.role != "player" {
            c.add(
                at.clone(),
                "climb and knockback are the player's components",
            );
        }
        // Each state's entity: what its `set` makes, checked the same way
        // for problems the entity itself doesn't have.
        if let Some(m) = &e.core.machine {
            let mut base = Checker {
                problems: Vec::new(),
            };
            components(&mut base, &at, e, has_platformer, &input_actions);
            let known: BTreeSet<String> = base.problems.into_iter().map(|p| p.message).collect();
            for sname in m.states.keys() {
                let Ok(se) = e.in_state(sname) else { continue };
                let mut st = Checker {
                    problems: Vec::new(),
                };
                components(&mut st, &at, &se, has_platformer, &input_actions);
                for p in st.problems {
                    // A state that turns `keep_fall_gravity` off leaves the
                    // entity's `start_fall_gravity` unread in that state
                    // only, which is what it means to (Super Mario Bros.'
                    // death fall sets its gravity every frame).
                    let unread_start = p.path.ends_with(".jump.start_fall_gravity")
                        && p.message.starts_with("is read only with keep_fall_gravity");
                    if !known.contains(&p.message) && !unread_start {
                        c.add(
                            format!("{at}.machine.states.{sname}.set"),
                            format!("{}: {}", p.path, p.message),
                        );
                    }
                }
            }
        }
    }

    // Levels.
    for (name, l) in &model.levels {
        let at = format!("levels.{name}");
        c.name(&at, name);
        let core = &l.core;
        match (&core.grid, &core.layout) {
            (Some(g), None) => {
                want_slot(&mut c, format!("{at}.grid"), g, AssetKind::LevelGrid);
                if core.tileset.is_none() || core.solid_property.is_none() {
                    c.add(&at, "a level with a grid needs tileset and solid_property");
                }
            }
            (None, Some(lay)) => want_slot(
                &mut c,
                format!("{at}.layout.slot"),
                &lay.slot,
                AssetKind::LevelObjects,
            ),
            (None, None) if core.empty => {
                if core.tileset.is_some() || core.solid_property.is_some() {
                    c.add(&at, "an empty level has no tileset and no solid_property");
                }
            }
            _ if core.empty => c.add(&at, "an empty level has no grid and no layout"),
            _ => c.add(
                &at,
                "give exactly one of grid and layout, or say `empty = true`",
            ),
        }
        if let Some(t) = &core.tileset {
            want_slot(&mut c, format!("{at}.tileset"), t, AssetKind::Metatiles);
        }
        if core.kind_property.is_some() && core.grid.is_none() {
            c.add(
                format!("{at}.kind_property"),
                "a tileset property is read from a grid's tileset; give a grid",
            );
        }
        for k in core.tile_kinds.keys() {
            if k.parse::<u32>().is_err() {
                c.add(
                    format!("{at}.tile_kinds"),
                    format!("`{k}` isn't a whole number, a property value or collision class"),
                );
            }
        }
        if let Some(cam) = &core.camera
            && cam.hold_player
            && cam.step.is_none()
        {
            c.add(
                format!("{at}.camera.hold_player"),
                "holding the player waits for a camera that moves in steps; give step",
            );
        }
        if let Some(cam) = &core.camera
            && cam.step.is_some_and(|s| s <= crate::Fixed::ZERO)
        {
            c.add(format!("{at}.camera.step"), "must be above zero");
        }
        if core.respawn && core.despawn_behind.is_none() && core.despawn_ahead.is_none() {
            c.add(
                format!("{at}.respawn"),
                "only an entity that leaves by the view respawns; give despawn_behind or despawn_ahead",
            );
        }
        let spawn_rules = core.spawn_slots.is_some()
            || core.spawn_in_order
            || core.appear_still
            || core.draw.is_some()
            || core.spawn.iter().any(|s| s.drawn);
        if core.slot_rest && core.spawn_slots.is_none() {
            c.add(
                format!("{at}.slot_rest"),
                "numbers the spawn slots: give spawn_slots",
            );
        }
        if let Some(l) = &core.loop_back {
            match &core.draw {
                None => c.add(format!("{at}.loop_back"), "tests as a draw goes: give draw"),
                Some(d) if d.step == 0 || l.back % d.step != 0 || l.back % core.tile_size != 0 => c
                    .add(
                        format!("{at}.loop_back.back"),
                        "must be a whole number of the draw's steps and of tiles",
                    ),
                Some(_) => {}
            }
        }
        if spawn_rules && core.spawn_ahead.is_none() {
            c.add(
                format!("{at}.spawn_ahead"),
                "spawn_slots, spawn_in_order, appear_still and draw shape spawning by the view; give spawn_ahead",
            );
        }
        if core.despawn_at.is_some()
            && core.despawn_behind.is_none()
            && core.despawn_ahead.is_none()
        {
            c.add(
                format!("{at}.despawn_at"),
                "says what despawning measures; give despawn_behind or despawn_ahead",
            );
        }
        if core.despawn_after_move && core.despawn_behind.is_none() && core.despawn_ahead.is_none()
        {
            c.add(
                format!("{at}.despawn_after_move"),
                "says when despawning tests; give despawn_behind or despawn_ahead",
            );
        }
        if core.spawn_slots == Some(0) {
            c.add(format!("{at}.spawn_slots"), "must be above zero");
        }
        if let Some(d) = &core.draw {
            if d.step == 0 || core.tile_size == 0 || d.step % core.tile_size != 0 {
                c.add(
                    format!("{at}.draw.step"),
                    "must be a whole number of tiles, above zero",
                );
            }
            if d.every == 0 {
                c.add(format!("{at}.draw.every"), "must be above zero");
            }
            if d.ahead < crate::Fixed::ZERO {
                c.add(format!("{at}.draw.ahead"), "must be zero or above");
            }
        }
        for (i, s) in core.spawn.iter().enumerate() {
            if s.drawn && core.draw.is_none() {
                c.add(
                    format!("{at}.spawn[{i}].drawn"),
                    "a drawn spawn appears as the level's draw reaches it; give the level a draw",
                );
            }
        }
        if let Some(pr) = &core.present {
            if let Some(pal) = &pr.palettes {
                want_slot(
                    &mut c,
                    format!("{at}.present.palettes"),
                    pal,
                    AssetKind::Palettes,
                );
            }
            if let Some(bg) = &pr.background {
                want_slot(
                    &mut c,
                    format!("{at}.present.background.slot"),
                    &bg.slot,
                    AssetKind::Table,
                );
            }
            // Feature `present.effect_order`.
            for event in pr.effect_order.keys() {
                let named = model
                    .present
                    .as_ref()
                    .is_some_and(|p| p.effects.iter().any(|e| &e.event == event));
                if !named {
                    c.add(
                        format!("{at}.present.effect_order.{event}"),
                        format!("no effect starts on `{event}`"),
                    );
                }
            }
            // Feature `present.sprite_order`.
            for (name, o) in &pr.sprite_order {
                let at = format!("{at}.present.sprite_order.{name}");
                if !model.entities.contains_key(name) {
                    c.add(at.clone(), format!("no entity named `{name}`"));
                }
                if let Some(t) = &o.turn {
                    if !model.counters.contains_key(&t.counter) {
                        c.add(
                            format!("{at}.turn.counter"),
                            format!("no counter named `{}`", t.counter),
                        );
                    }
                    if t.count == 0 {
                        c.add(format!("{at}.turn.count"), "must be above zero");
                    }
                }
            }
        }
        if let Some(so) = &core.spawn_objects {
            want_slot(
                &mut c,
                format!("{at}.spawn_objects.slot"),
                &so.slot,
                AssetKind::SpawnObjects,
            );
        }
        for (k, v) in [
            ("spawn_ahead", core.spawn_ahead),
            ("despawn_behind", core.despawn_behind),
            ("despawn_ahead", core.despawn_ahead),
        ] {
            if v.is_some_and(|v| v < crate::Fixed::ZERO) {
                c.add(format!("{at}.{k}"), "must be zero or above");
            }
        }
        for (name, v) in &core.counters {
            match model.counters.get(name) {
                None => c.add(
                    format!("{at}.counters.{name}"),
                    format!("no counter named `{name}`"),
                ),
                Some(ctr) if ctr.min.is_some_and(|m| *v < m) || ctr.max.is_some_and(|m| *v > m) => {
                    c.add(
                        format!("{at}.counters.{name}"),
                        format!("{v} is outside the counter's min and max"),
                    )
                }
                Some(_) => {}
            }
        }
        if let Some(n) = &core.next
            && !model.levels.contains_key(n)
        {
            c.add(format!("{at}.next"), format!("no level named `{n}`"));
        }
        for (i, x) in core.exit.iter().enumerate() {
            let xat = format!("{at}.exit[{i}]");
            let Some(to) = model.levels.get(&x.to) else {
                c.add(format!("{xat}.to"), format!("no level named `{}`", x.to));
                continue;
            };
            if let Some(t) = &x.take {
                if t.at.w <= crate::Fixed::ZERO || t.at.h <= crate::Fixed::ZERO {
                    c.add(
                        format!("{xat}.take.at"),
                        "width and height must be above zero",
                    );
                }
                if let Some(h) = &t.hold
                    && !input_actions.contains(h.as_str())
                {
                    c.add(format!("{xat}.take.hold"), format!("no input action `{h}`"));
                }
                if let Some(g) = &t.guard
                    && !model.counters.contains_key(&g.counter)
                {
                    c.add(
                        format!("{xat}.take.if.counter"),
                        format!("no counter named `{}`", g.counter),
                    );
                }
            }
            if x.start && (x.keep.is_some() || x.scroll.is_some()) {
                c.add(
                    &xat,
                    "an exit that starts its level puts the player at its spawn, so it neither keeps a coordinate nor scrolls",
                );
            }
            if x.scroll == Some(0) {
                c.add(format!("{xat}.scroll"), "must be at least 1 frame");
            }
            if x.start && x.enter.is_some() {
                c.add(
                    format!("{xat}.enter"),
                    "an exit that starts its level puts the player at the level's spawn",
                );
            }
            if let Some(e) = &x.enter {
                let px = |n: u32| {
                    crate::Fixed::from_int(i64::from(n) * i64::from(to.core.tile_size))
                        .unwrap_or_default()
                };
                let (w, h) = (px(to.core.size[0]), px(to.core.size[1]));
                if e.x < crate::Fixed::ZERO || e.x >= w || e.y < crate::Fixed::ZERO || e.y >= h {
                    c.add(
                        format!("{xat}.enter"),
                        format!(
                            "({}, {}) is outside `{}` (0 to {w}, 0 to {h})",
                            e.x, e.y, x.to
                        ),
                    );
                }
            }
        }
        if l.core.tile_size == 0 || l.core.size.contains(&0) {
            c.add(&at, "tile_size and size must be above zero");
        }
        if let Some(h) = &l.platformer.hazard
            && !matches!(
                h.touch,
                platformer::Touch::KillPlayer | platformer::Touch::HurtPlayer
            )
        {
            c.add(
                format!("{at}.hazard.touch"),
                "a hazard kills or hurts the player: kill-player or hurt-player",
            );
        }
        if let Some(ev) = &l.platformer.pit_event {
            if !model.model.events.iter().any(|e| e == ev) {
                c.add(
                    format!("{at}.pit_event"),
                    format!("`{ev}` isn't one of model.events"),
                );
            }
            if l.platformer.kill_below.is_none() {
                c.add(format!("{at}.pit_event"), "needs kill_below");
            }
        }
        if l.platformer.hard_spawns && l.core.spawn_objects.is_none() {
            c.add(
                format!("{at}.hard_spawns"),
                "picks from a level's object list: give spawn_objects",
            );
        }
        if !l.platformer.is_empty() && !has_platformer {
            c.add(
                &at,
                "uses platformer fields without declaring the platformer extension",
            );
        }
        let mut players = 0;
        for (i, s) in l.core.spawn.iter().enumerate() {
            let sat = format!("{at}.spawn[{i}]");
            let Some(e) = model.entities.get(&s.entity) else {
                c.add(
                    format!("{sat}.entity"),
                    format!("no entity named `{}`", s.entity),
                );
                continue;
            };
            if e.core.role == "player" {
                players += 1;
            }
            if let Some(t) = &s.table {
                want_slot(&mut c, format!("{sat}.table"), t, AssetKind::Table);
            } else if !s.columns.is_empty() {
                c.add(format!("{sat}.columns"), "columns need a table");
            }
            for field in s.columns.keys().chain(s.params.keys()) {
                if field != "x" && field != "y" && !params.contains(field.as_str()) {
                    c.add(
                        &sat,
                        format!(
                            "`{field}` isn't a spawn field or a parameter of a declared extension"
                        ),
                    );
                }
            }
            for k in s.params.keys() {
                if k == "x" || k == "y" {
                    c.add(
                        format!("{sat}.params"),
                        format!("write `{k}` as a field, not a parameter"),
                    );
                }
            }
            for xy in ["x", "y"] {
                let given = if xy == "x" {
                    s.x.is_some()
                } else {
                    s.y.is_some()
                };
                if given == s.columns.contains_key(xy) {
                    c.add(
                        &sat,
                        format!("give `{xy}` either as a number or as a column, once"),
                    );
                }
            }
            if e.platformer.patrol.is_some()
                && !s.params.contains_key("patrol_end")
                && !s.columns.contains_key("patrol_end")
            {
                c.add(
                    &sat,
                    format!("`{}` patrols, so it needs a patrol_end", s.entity),
                );
            }
            // Literal positions lie inside the level, and a patrol runs
            // rightward from its start.
            let px = |n: u32| {
                crate::Fixed::from_int(i64::from(n) * i64::from(l.core.tile_size))
                    .unwrap_or_default()
            };
            let bounds = [px(l.core.size[0]), px(l.core.size[1])];
            for (v, bound, name) in [(s.x, bounds[0], "x"), (s.y, bounds[1], "y")] {
                if let Some(v) = v
                    && (v < crate::Fixed::ZERO || v >= bound)
                {
                    c.add(
                        format!("{sat}.{name}"),
                        format!("{v} is outside the level (0 to {bound})"),
                    );
                }
            }
            if let (Some(x), Some(end)) = (s.x, s.params.get("patrol_end"))
                && *end < x
            {
                c.add(
                    format!("{sat}.params.patrol_end"),
                    "a patrol ends at or right of where it starts",
                );
            }
            if s.params.keys().any(|k| s.columns.contains_key(k)) {
                c.add(
                    &sat,
                    "a parameter is given both as a number and as a column",
                );
            }
        }
        if players != 1 {
            c.add(
                format!("{at}.spawn"),
                format!("needs one player spawn, has {players}"),
            );
        }
    }

    // Cameras.
    for (name, cam) in &model.cameras {
        if cam.keep_in_view_touch && cam.keep_in_view.is_none() {
            c.add(
                format!("cameras.{name}.keep_in_view_touch"),
                "needs keep_in_view",
            );
        }
        if !cam.keep_in_view_rows.is_empty() && cam.keep_in_view.is_none() {
            c.add(
                format!("cameras.{name}.keep_in_view_rows"),
                "needs keep_in_view",
            );
        }
        for (k, r) in cam.keep_in_view_rows.iter().enumerate() {
            if r.rows[0] > r.rows[1] || r.bounds[0] > r.bounds[1] {
                c.add(
                    format!("cameras.{name}.keep_in_view_rows[{k}]"),
                    "each range's first number is at most its second",
                );
            }
        }
        if cam.keep_in_view_turn && cam.keep_in_view.is_none() {
            c.add(
                format!("cameras.{name}.keep_in_view_turn"),
                "needs keep_in_view",
            );
        }
        if let Some(k) = cam.keep_in_view
            && k[0] > k[1]
        {
            c.add(
                format!("cameras.{name}.keep_in_view"),
                "the first must be at most the second",
            );
        }
        let at = format!("cameras.{name}");
        c.name(&at, name);
        if !model.entities.contains_key(&cam.follow) {
            c.add(
                format!("{at}.follow"),
                format!("no entity named `{}`", cam.follow),
            );
        }
        if cam.size.contains(&0) {
            c.add(format!("{at}.size"), "must be above zero");
        }
        for (i, lead) in cam.lead.iter().enumerate() {
            let size = crate::Fixed::from_int(i64::from(cam.size[i])).unwrap_or_default();
            if *lead < crate::Fixed::ZERO || *lead > size {
                c.add(
                    format!("{at}.lead"),
                    format!("{lead} is outside the view (0 to {size})"),
                );
            }
        }
        if let Some(m) = &cam.by_move {
            let size = cam.size.iter().copied().max().unwrap_or(0);
            let size = crate::Fixed::from_int(i64::from(size)).unwrap_or_default();
            if m.from < crate::Fixed::ZERO || m.from > m.full || m.full > size {
                c.add(
                    format!("{at}.by_move"),
                    format!(
                        "from {} and full {} must go in order inside the view",
                        m.from, m.full
                    ),
                );
            }
            if m.short_by < crate::Fixed::ZERO {
                c.add(format!("{at}.by_move.short_by"), "can't be below zero");
            }
            if cam.scroll_back {
                c.add(
                    format!("{at}.by_move"),
                    "moves only forward: needs scroll_back = false",
                );
            }
        }
    }

    // Presentation.
    if let Some(pr) = &model.present {
        if let Some(sc) = &pr.screen {
            let [w, h] = sc.size;
            let [l, t, r, b] = sc.overscan.unwrap_or_default();
            if w == 0 || h == 0 || l + r >= w || t + b >= h {
                c.add(
                    "present.screen",
                    "the size must be above zero and more than the overscan hides",
                );
            }
            if let Some(sl) = &sc.sprite_limit
                && (sl.per_line == 0 || sl.sprite_width == Some(0))
            {
                c.add(
                    "present.screen.sprite_limit",
                    "per_line and sprite_width must be above zero",
                );
            }
        }
        for st in pr.screen.iter().flat_map(|s| &s.sprites_off) {
            let known = st.split_once('.').is_some_and(|(m, s)| {
                model
                    .rules
                    .get(m)
                    .is_some_and(|r| r.states.contains_key(s) || r.initial == s)
            });
            if !known {
                c.add(
                    "present.screen.sprites_off",
                    format!("`{st}` isn't a rule state, <machine>.<state>"),
                );
            }
        }
        for (i, h) in pr.hidden.iter().enumerate() {
            let at = format!("present.hidden[{i}]");
            if h.states.is_empty() {
                c.add(format!("{at}.states"), "names at least one rule state");
            }
            for st in &h.states {
                let known = st.split_once('.').is_some_and(|(m, s)| {
                    model
                        .rules
                        .get(m)
                        .is_some_and(|r| r.states.contains_key(s) || r.initial == s)
                });
                if !known {
                    c.add(
                        format!("{at}.states"),
                        format!("`{st}` isn't a rule state, <machine>.<state>"),
                    );
                }
            }
            for t in &h.when {
                if !model.counters.contains_key(&t.name) {
                    c.add(
                        format!("{at}.when"),
                        format!("the model has no counter `{}`", t.name),
                    );
                }
                if t.at_least.is_none() && t.below.is_none() {
                    c.add(
                        format!("{at}.when"),
                        "a counter test gives at_least, below or both",
                    );
                }
            }
            for k in &h.kinds {
                if !model.entities.contains_key(k) {
                    c.add(format!("{at}.kinds"), format!("no entity type `{k}`"));
                }
            }
        }
        for (i, sc) in pr.screens.iter().enumerate() {
            let at = format!("present.screens[{i}]");
            if let Some(layout) = &sc.layout {
                want_slot(
                    &mut c,
                    format!("{at}.layout.slot"),
                    &layout.slot,
                    AssetKind::ScreenText,
                );
            }
            if sc.layout.is_some() == sc.blank {
                c.add(
                    format!("{at}.layout"),
                    "a screen gives a layout or blank = true, not both",
                );
            }
            if sc.from_line.is_some() && !sc.blank {
                c.add(
                    format!("{at}.from_line"),
                    "from_line goes with blank = true",
                );
            }
            for (k, t) in sc.text.iter().enumerate() {
                if pr.hud.is_none() {
                    c.add(
                        format!("{at}.text[{k}]"),
                        "a screen's text is drawn in the HUD's font: give present.hud",
                    );
                }
                if t.palette.is_some_and(|p| p > 3) {
                    c.add(
                        format!("{at}.text[{k}].palette"),
                        "a background palette, 0 to 3",
                    );
                }
                if t.at[0] as usize + t.text.chars().count() > 32 || t.at[1] >= 30 {
                    c.add(
                        format!("{at}.text[{k}].at"),
                        "the text goes past the name table's 32 columns and 30 rows",
                    );
                }
            }
            for t in &sc.when {
                if !model.counters.contains_key(&t.name) {
                    c.add(
                        format!("{at}.when"),
                        format!("the model has no counter `{}`", t.name),
                    );
                }
                if t.at_least.is_none() && t.below.is_none() {
                    c.add(
                        format!("{at}.when"),
                        "a counter test gives at_least, below or both",
                    );
                }
            }
            if sc.states.is_empty() {
                c.add(format!("{at}.states"), "names at least one rule state");
            }
            for st in &sc.states {
                let known = st.split_once('.').is_some_and(|(m, s)| {
                    model
                        .rules
                        .get(m)
                        .is_some_and(|r| r.states.contains_key(s) || r.initial == s)
                });
                if !known {
                    c.add(
                        format!("{at}.states"),
                        format!("`{st}` isn't a rule state, <machine>.<state>"),
                    );
                }
            }
            for (k, sp) in sc.sprite.iter().enumerate() {
                let sat = format!("{at}.sprite[{k}]");
                want_slot(&mut c, format!("{sat}.tiles"), &sp.tiles, AssetKind::Tiles);
                if sp.palette.is_some_and(|p| p > 3) {
                    c.add(format!("{sat}.palette"), "a sprite palette is 0 to 3");
                }
                if let Some(k) = &sp.counter
                    && !model.counters.contains_key(k)
                {
                    c.add(format!("{sat}.counter"), format!("no counter named `{k}`"));
                }
            }
        }
        if let Some(hud) = &pr.hud {
            want_slot(
                &mut c,
                "present.hud.tiles".into(),
                &hud.tiles,
                AssetKind::Tiles,
            );
            want_slot(
                &mut c,
                "present.hud.font".into(),
                &hud.font,
                AssetKind::Font,
            );
            if let Some(l) = &hud.layer {
                want_slot(
                    &mut c,
                    "present.hud.layer.slot".into(),
                    &l.slot,
                    AssetKind::ScreenText,
                );
            }
            for (i, sp) in hud.sprite.iter().enumerate() {
                let at = format!("present.hud.sprite[{i}]");
                want_slot(&mut c, format!("{at}.tiles"), &sp.tiles, AssetKind::Tiles);
                if sp.palette.is_some_and(|p| p > 3) {
                    c.add(format!("{at}.palette"), "a sprite palette is 0 to 3");
                }
            }
            for (i, f) in hud.field.iter().enumerate() {
                let at = format!("present.hud.field[{i}]");
                let given = usize::from(f.counter.is_some())
                    + usize::from(f.level_label)
                    + usize::from(f.text.is_some());
                if given != 1 {
                    c.add(&at, "give exactly one of counter, level_label and text");
                }
                if let Some(k) = &f.counter
                    && !model.counters.contains_key(k)
                {
                    c.add(format!("{at}.counter"), format!("no counter named `{k}`"));
                }
                if f.divide == Some(0) {
                    c.add(format!("{at}.divide"), "must be above zero");
                }
                if f.pad.as_ref().is_some_and(|p| p.chars().count() != 1) {
                    c.add(format!("{at}.pad"), "must be one character");
                }
                if f.palette.is_some_and(|p| p > 3) {
                    c.add(format!("{at}.palette"), "a background palette is 0 to 3");
                }
                for g in &f.when {
                    if !model.counters.contains_key(&g.counter) {
                        c.add(
                            format!("{at}.when"),
                            format!("no counter named `{}`", g.counter),
                        );
                    }
                }
            }
        }
    }
    // Sound (core 0.4.3): slots of the right kinds, levels, states and
    // events that exist.
    if let Some(snd) = model.present.as_ref().and_then(|p| p.sound.as_ref()) {
        for (i, m) in snd.music.iter().enumerate() {
            let at = format!("present.sound.music[{i}]");
            want_slot(&mut c, format!("{at}.slot"), &m.song.slot, AssetKind::Songs);
            match (&m.level, &m.state) {
                (Some(l), None) if !model.levels.contains_key(l) => {
                    c.add(format!("{at}.level"), format!("no level named `{l}`"))
                }
                (None, Some(st)) => {
                    let ok = st.split_once('.').is_some_and(|(r, s)| {
                        model.rules.get(r).is_some_and(|m| m.states.contains_key(s))
                    });
                    if !ok {
                        c.add(format!("{at}.state"), format!("no rule state `{st}`"));
                    }
                }
                (Some(_), Some(_)) | (None, None) => {
                    c.add(&at, "give exactly one of level and state")
                }
                _ => {}
            }
        }
        let mut names = std::collections::BTreeSet::new();
        for (i, r) in snd.roles.iter().enumerate() {
            let at = format!("present.sound.roles[{i}]");
            let ok = r.name.starts_with(|c: char| c.is_ascii_lowercase())
                && r.name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            if !ok {
                c.add(
                    format!("{at}.name"),
                    format!(
                        "`{}` isn't a sound role name: lowercase letters, digits and _",
                        r.name
                    ),
                );
            }
            if !names.insert(r.name.as_str()) {
                c.add(format!("{at}.name"), format!("`{}` is named twice", r.name));
            }
            if !model.counters.contains_key(&r.counter) {
                c.add(
                    format!("{at}.counter"),
                    format!("no counter named `{}`", r.counter),
                );
            }
            if r.description.trim().is_empty() {
                c.add(format!("{at}.description"), "say when it plays");
            }
        }
        for (i, t) in snd.silent_while.iter().enumerate() {
            if !model.counters.contains_key(&t.counter) {
                c.add(
                    format!("present.sound.silent_while[{i}].counter"),
                    format!("no counter named `{}`", t.counter),
                );
            }
        }
        for (i, e) in snd.sfx.iter().enumerate() {
            let at = format!("present.sound.sfx[{i}]");
            want_slot(
                &mut c,
                format!("{at}.slot"),
                &e.slot,
                AssetKind::SoundEffects,
            );
            if !events.contains(e.event.as_str()) {
                c.add(
                    format!("{at}.event"),
                    format!("`{}` isn't an event of the model", e.event),
                );
            }
        }
    }
    for (name, slot) in &model.assets {
        let sound = matches!(slot.kind, AssetKind::Songs | AssetKind::SoundEffects);
        if slot.capture.is_some() && !sound {
            c.add(
                format!("assets.{name}.capture"),
                "only a songs or sound-effects slot is captured from a recording",
            );
        }
        if sound && !(slot.role.starts_with("music.") || slot.role.starts_with("sfx.")) {
            c.add(
                format!("assets.{name}.role"),
                "a sound slot's role is music.<mood> or sfx.<event>",
            );
        }
    }
    let cycles = model
        .present
        .iter()
        .flat_map(|p| {
            p.cycles
                .iter()
                .enumerate()
                .map(|(i, cy)| (format!("present.cycles[{i}]"), cy))
        })
        .chain(model.levels.iter().flat_map(|(n, l)| {
            l.core.present.iter().flat_map(move |p| {
                p.cycles
                    .iter()
                    .enumerate()
                    .map(move |(i, cy)| (format!("levels.{n}.present.cycles[{i}]"), cy))
            })
        }));
    for (at, cy) in cycles {
        want_slot(&mut c, format!("{at}.colors"), &cy.colors, AssetKind::Table);
        if cy.palette > 3 || cy.index == 0 || cy.index > 3 || cy.every == 0 {
            c.add(&at, "palette is 0 to 3, index 1 to 3, and every above zero");
        }
        if cy.counter.is_some() || cy.shift.is_some() {
            match &cy.counter {
                Some(k) if !model.counters.contains_key(k) => {
                    c.add(format!("{at}.counter"), format!("no counter named `{k}`"))
                }
                None => c.add(format!("{at}.shift"), "needs a counter"),
                _ => {}
            }
        }
    }

    // Head hits (platformer 0.4.5): their events, on the entity and as
    // each of its states makes it, and the tile kinds core 0.4.6 added.
    if let Some(p) = &model.present {
        if let Some(cells) = &p.cells {
            for k in cells.bump.iter().flat_map(|b| b.sprites.keys()) {
                if crate::model::TileKind::parse(k).is_none() {
                    c.add(
                        format!("present.cells.bump.sprites.{k}"),
                        "isn't a tile kind",
                    );
                }
            }
            for k in cells.kinds.keys() {
                if crate::model::TileKind::parse(k).is_none() {
                    c.add(format!("present.cells.kinds.{k}"), "isn't a tile kind");
                }
            }
        }
        for (i, e) in p.effects.iter().enumerate() {
            if e.phases.is_empty() {
                c.add(format!("present.effects[{i}].phases"), "is empty");
            }
            for (j, ph) in e.phases.iter().enumerate() {
                let at = format!("present.effects[{i}].phases[{j}]");
                if !model
                    .assets
                    .get(&ph.slot)
                    .is_some_and(|s| s.kind == crate::model::AssetKind::Tiles)
                {
                    c.add(
                        format!("{at}.slot"),
                        format!("`{}` isn't a tiles slot", ph.slot),
                    );
                }
                if ph.frames.is_empty() {
                    c.add(format!("{at}.frames"), "is empty");
                }
                if let Some(n) = &ph.until_counter
                    && !model.counters.contains_key(n)
                {
                    c.add(
                        format!("{at}.until_counter"),
                        format!("`{n}` isn't one of the model's counters"),
                    );
                }
                for n in &ph.until_counters {
                    if !model.counters.contains_key(n) {
                        c.add(
                            format!("{at}.until_counters"),
                            format!("`{n}` isn't one of the model's counters"),
                        );
                    }
                }
                if ph.until_gone && e.at == crate::model::EffectAt::Hit {
                    c.add(
                        format!("{at}.until_gone"),
                        "needs an effect at its entity, not at a hit",
                    );
                }
            }
        }
    }
    for (name, e) in &model.entities {
        let states: Vec<(String, crate::model::Entity)> = e
            .core
            .machine
            .iter()
            .flat_map(|m| m.states.keys())
            .filter_map(|s| Some((format!(".machine.states.{s}"), e.in_state(s).ok()?)))
            .collect();
        for (part, v) in std::iter::once((String::new(), e.clone())).chain(states) {
            let Some(s) = &v.core.sprite else { continue };
            // A sprite-set frame brings its own layout, so cells meant for
            // a row of tiles do nothing there: a state that names only a
            // row keeps the sprite's sprite-set slot and draws that set's
            // frame of the number instead.
            if s.frame
                .as_ref()
                .is_some_and(|f| slot_kind(&f.slot) == Some(AssetKind::SpriteSet))
                && !s.cells.is_empty()
            {
                c.add(
                    format!("entities.{name}{part}.sprite.cells"),
                    "cells don't apply to a frame of a sprite-set slot, which brings its own layout; name the frame's slot",
                );
            }
            // Feature `sprite.split`: a kept part with a size, and counters
            // the model has.
            if let Some(sp) = &s.split {
                let at = format!("entities.{name}{part}.sprite.split");
                if sp.keep[2] == 0 || sp.keep[3] == 0 {
                    c.add(
                        format!("{at}.keep"),
                        "the kept part needs a width and a height",
                    );
                }
                for n in sp.x.iter().chain(&sp.y) {
                    if !model.counters.contains_key(n) {
                        c.add(at.clone(), format!("the model has no counter `{n}`"));
                    }
                }
            }
            // Feature `sprite.behind_when`: its condition, as a clip's.
            if let Some(w) = &s.behind_when {
                let at = format!("entities.{name}{part}.sprite.behind_when");
                for t in w.word.iter().chain(&w.words) {
                    if t.index > 3 || (t.at_least.is_none() && t.below.is_none()) {
                        c.add(
                            at.clone(),
                            "a word test names a word 0 to 3 and gives at_least, below or both",
                        );
                    }
                }
                for t in &w.counters {
                    if !model.counters.contains_key(&t.name) {
                        c.add(
                            format!("{at}.counters"),
                            format!("the model has no counter `{}`", t.name),
                        );
                    }
                }
                for t in &w.slot_counters {
                    if !model.counters.contains_key(&format!("{}0", t.name)) {
                        c.add(
                            format!("{at}.slot_counters"),
                            format!("the model has no counter `{}0`, the first slot's", t.name),
                        );
                    }
                }
                x_range(&mut c, &at, w);
                if s.behind {
                    c.add(
                        at.clone(),
                        "the sprite is always behind (`behind`); give one or the other",
                    );
                }
            }
            for (i, clip) in s.clips.iter().enumerate() {
                let at = format!("entities.{name}{part}.sprite.clips[{i}]");
                for (k, w) in clip.when.word.iter().chain(&clip.when.words).enumerate() {
                    if w.index > 3 || (w.at_least.is_none() && w.below.is_none()) {
                        let path = match k {
                            0 if clip.when.word.is_some() => format!("{at}.when.word"),
                            _ => format!("{at}.when.words"),
                        };
                        c.add(
                            path,
                            "a word test names a word 0 to 3 and gives at_least, below or both",
                        );
                    }
                }
                for t in &clip.when.slot_counters {
                    if !model.counters.contains_key(&format!("{}0", t.name)) {
                        c.add(
                            format!("{at}.when.slot_counters"),
                            format!("the model has no counter `{}0`, the first slot's", t.name),
                        );
                    }
                    if t.at_least.is_none() && t.below.is_none() {
                        c.add(
                            format!("{at}.when.slot_counters"),
                            "a counter test gives at_least, below or both",
                        );
                    }
                }
                for t in &clip.when.counters {
                    if !model.counters.contains_key(&t.name) {
                        c.add(
                            format!("{at}.when.counters"),
                            format!("the model has no counter `{}`", t.name),
                        );
                    }
                    if t.at_least.is_none() && t.below.is_none() {
                        c.add(
                            format!("{at}.when.counters"),
                            "a counter test gives at_least, below or both",
                        );
                    }
                }
                x_range(&mut c, &format!("{at}.when"), &clip.when);
                let timed = clip.frames.iter().filter(|f| f.every.is_some()).count();
                if timed != 0 && timed != clip.frames.len() {
                    c.add(
                        format!("{at}.frames"),
                        "every frame of the clip gives its own every, or none does",
                    );
                }
                for (k, f) in clip.frames.iter().enumerate() {
                    if let Some(slot) = &f.slot
                        && (slot_kind(slot) != Some(AssetKind::Rows) || f.row.is_none())
                    {
                        c.add(
                            format!("{at}.frames[{k}].slot"),
                            "a frame's slot is a rows slot, and the frame gives its row",
                        );
                    }
                }
                if clip.frames.iter().any(|f| f.every == Some(0)) {
                    c.add(format!("{at}.frames"), "a frame's every is at least 1");
                }
            }
        }
    }
    for (name, e) in &model.entities {
        let at = format!("entities.{name}");
        let variants = std::iter::once((at.clone(), e.clone())).chain(
            e.core
                .machine
                .iter()
                .flat_map(|m| m.states.keys())
                .filter_map(|s| {
                    e.in_state(s)
                        .ok()
                        .map(|v| (format!("{at}.machine.states.{s}.set"), v))
                }),
        );
        for (vat, v) in variants {
            if let Some(ev) = v
                .platformer
                .jump
                .as_ref()
                .and_then(|j| j.pickup.as_ref())
                .and_then(|p| p.event.as_ref())
                && !events.contains(ev.as_str())
            {
                c.add(
                    format!("{vat}.jump.pickup.event"),
                    format!("`{ev}` isn't an event of the core, a counter, a declared extension or the model's own"),
                );
            }
            let Some(hd) = v.platformer.jump.as_ref().and_then(|j| j.head.as_ref()) else {
                continue;
            };
            for (i, h) in hd.hits.iter().enumerate() {
                let hat = format!("{vat}.jump.head.hits[{i}]");
                if h.tile == crate::model::TileKind::Coin {
                    c.add(&hat, "a head hit is on a cell that stops movement");
                }
                if let Some(ev) = &h.event
                    && !events.contains(ev.as_str())
                {
                    c.add(
                        format!("{hat}.event"),
                        format!("`{ev}` isn't an event of the core, a counter, a declared extension or the model's own"),
                    );
                }
            }
        }
    }

    // Rules, and entities' own state machines.
    for (name, m) in &model.rules {
        let at = format!("rules.{name}");
        c.name(&at, name);
        machine(&mut c, model, &at, m, &events, &actions, false);
    }
    for (name, e) in &model.entities {
        if let Some(m) = &e.core.machine {
            let entity_actions: BTreeSet<&str> = actions
                .iter()
                .copied()
                .chain(ENTITY_ACTIONS.iter().copied())
                .collect();
            machine(
                &mut c,
                model,
                &format!("entities.{name}.machine"),
                m,
                &events,
                &entity_actions,
                true,
            );
        }
    }
    for (name, ctr) in &model.counters {
        let at = format!("counters.{name}");
        c.name(&at, name);
        for (i, ch) in ctr.change.iter().enumerate() {
            let cat = format!("{at}.change[{i}]");
            match (&ch.on, ch.every) {
                (Some(ev), None) => {
                    if !events.contains(ev.as_str()) {
                        c.add(
                            format!("{cat}.on"),
                            format!("`{ev}` isn't an event of the core, a counter or a declared extension"),
                        );
                    }
                }
                (None, Some(0)) => c.add(format!("{cat}.every"), "must be at least 1 frame"),
                (None, Some(_)) => {}
                _ => c.add(&cat, "give exactly one of `on` and `every`"),
            }
            if (ch.after > 0 || !ch.of.is_empty()) && ch.on.is_none() {
                c.add(&cat, "`after` and `of` go with `on`");
            }
            for k in &ch.of {
                if !model.entities.contains_key(k) {
                    c.add(format!("{cat}.of"), format!("no entity `{k}`"));
                }
            }
        }
        if let (Some(lo), Some(hi)) = (ctr.min, ctr.max)
            && lo > hi
        {
            c.add(&at, "min is above max");
        }
        if ctr.wrap && ctr.max.is_none() {
            c.add(format!("{at}.wrap"), "wrapping needs a max");
        }
        if ctr.runs_when_frozen
            && ctr.shift.is_none()
            && !ctr.change.iter().any(|ch| ch.every.is_some())
        {
            c.add(
                format!("{at}.runs_when_frozen"),
                "needs an `every` change or a `shift`, the only kinds a freeze stops",
            );
        }
        if let Some(s) = &ctr.shift
            && (s.bits == 0
                || s.bits > 63
                || s.taps.is_empty()
                || s.taps.iter().any(|t| *t >= s.bits))
        {
            c.add(
                format!("{at}.shift"),
                "needs 1 to 63 bits and at least one tap below them",
            );
        }
        for (what, v) in [("min", ctr.min), ("max", ctr.max)] {
            let out = match what {
                "min" => v.is_some_and(|v| ctr.start < v),
                _ => v.is_some_and(|v| ctr.start > v),
            };
            if out {
                c.add(format!("{at}.start"), format!("is outside its {what}"));
            }
        }
    }

    check_evidence(model, &mut c);
    c.problems.sort();
    c.problems.dedup();
    c.problems
}

/// Platformer 0.4.2's run options: each needs the parts it works with.
fn run_options(c: &mut Checker, at: &str, r: &crate::platformer::Run) {
    use crate::platformer::SkidBy;
    let zero = crate::Fixed::ZERO;
    if r.move_step.is_some_and(|s| s <= zero) {
        c.add(format!("{at}.move_step"), "must be above zero");
    }
    if r.cap_whole && (r.move_step.is_none() || r.acceleration.is_none()) {
        c.add(
            format!("{at}.cap_whole"),
            "caps whole steps: needs move_step and acceleration",
        );
    }
    if r.skid_by == SkidBy::LastMotion && r.skid_deceleration.is_some() {
        c.add(
            format!("{at}.skid_deceleration"),
            "skid_by = \"last_motion\" doubles the rate in use; leave skid_deceleration out",
        );
    }
    if r.skid_turn_below.is_some_and(|s| s <= zero) {
        c.add(format!("{at}.skid_turn_below"), "must be above zero");
    }
    if !r.skid_turn_with.is_empty() && r.skid_turn_below.is_none() {
        c.add(
            format!("{at}.skid_turn_with"),
            "steers for the quick turn: needs skid_turn_below",
        );
    }
    for (i, b) in r.decel_bands.iter().enumerate() {
        if b.from <= zero {
            c.add(format!("{at}.decel_bands[{i}].from"), "must be above zero");
        }
        if b.rate <= zero {
            c.add(format!("{at}.decel_bands[{i}].rate"), "must be above zero");
        }
        if i > 0 && b.from <= r.decel_bands[i - 1].from {
            c.add(
                format!("{at}.decel_bands[{i}].from"),
                "bands go in order of from, each above the last",
            );
        }
    }
    if r.decel_hold_from.is_some() && r.decel_bands.is_empty() {
        c.add(
            format!("{at}.decel_hold_from"),
            "holds the last of decel_bands: needs decel_bands",
        );
    }
    if r.decel_hold_from.is_some_and(|s| s <= zero) {
        c.add(format!("{at}.decel_hold_from"), "must be above zero");
    }
    if r.run_speed.is_none() {
        if r.run_hold_frames > 0 {
            c.add(
                format!("{at}.run_hold_frames"),
                "needs run_speed and run_action",
            );
        }
        if r.run_with_motion {
            c.add(
                format!("{at}.run_with_motion"),
                "needs run_speed and run_action",
            );
        }
        if r.air_run_from.is_some() {
            c.add(
                format!("{at}.air_run_from"),
                "needs run_speed and run_action",
            );
        }
    }
    // Platformer 0.4.3's air options.
    if r.air_run_from.is_some_and(|s| s <= zero) {
        c.add(format!("{at}.air_run_from"), "must be above zero");
    }
    if r.air_run_from.is_some() && !r.air_control {
        c.add(
            format!("{at}.air_run_from"),
            "picks the rate in the air: needs air_control",
        );
    }
}

/// Checks an entity's platformer components.
/// The top-down extension's parts: declared when used, their actions
/// defined, and a class select naming player entities with a class.
fn topdown_checks(c: &mut Checker, model: &Model, has_topdown: bool) {
    let actions: BTreeSet<&str> = model
        .input
        .iter()
        .flat_map(|i| i.actions.keys())
        .map(String::as_str)
        .collect();
    for (name, e) in &model.entities {
        let at = format!("entities.{name}");
        let t = &e.topdown;
        if t.is_empty() {
            continue;
        }
        if !has_topdown {
            c.add(
                &at,
                "uses top-down components without declaring the topdown extension",
            );
        }
        if let Some(m) = &t.move8 {
            if m.speed <= crate::Fixed::ZERO {
                c.add(format!("{at}.move8.speed"), "the speed is more than 0");
            }
            for a in m.action_names() {
                if !actions.contains(a) {
                    c.add(
                        format!("{at}.move8"),
                        format!("move8 reads the input action `{a}`, which isn't defined"),
                    );
                }
            }
        }
        if t.class.is_some() && e.core.role != "player" {
            c.add(format!("{at}.class"), "only a player entity has a class");
        }
    }
    for (name, l) in &model.levels {
        let at = format!("levels.{name}");
        let Some(sel) = &l.topdown.class_select else {
            continue;
        };
        if !has_topdown {
            c.add(
                &at,
                "uses top-down fields without declaring the topdown extension",
            );
        }
        if sel.classes.is_empty() {
            c.add(
                format!("{at}.class_select.classes"),
                "names at least one class",
            );
        }
        for (i, cl) in sel.classes.iter().enumerate() {
            let ok = model
                .entities
                .get(cl)
                .is_some_and(|e| e.topdown.class.is_some());
            if !ok {
                c.add(
                    format!("{at}.class_select.classes[{i}]"),
                    format!("`{cl}` isn't an entity with a `class`"),
                );
            }
        }
        for (k, a) in [("next", &sel.next), ("pick", &sel.pick)] {
            if !actions.contains(a.as_str()) {
                c.add(
                    format!("{at}.class_select.{k}"),
                    format!("no input action `{a}`"),
                );
            }
        }
    }
}

fn components(
    c: &mut Checker,
    at: &str,
    e: &crate::model::Entity,
    has_platformer: bool,
    input_actions: &BTreeSet<&str>,
) {
    let p = &e.platformer;
    if !p.is_empty() && !has_platformer {
        c.add(
            at,
            "uses platformer components without declaring the platformer extension",
        );
    }
    if let Some(run) = &p.run {
        for a in ["left", "right"] {
            if !input_actions.contains(a) {
                c.add(
                    format!("{at}.run"),
                    format!("run reads the input action `{a}`, which isn't defined"),
                );
            }
        }
        match (&run.run_speed, &run.run_action) {
            (Some(_), Some(a)) if !input_actions.contains(a.as_str()) => c.add(
                format!("{at}.run.run_action"),
                format!("no input action `{a}`"),
            ),
            (Some(_), None) | (None, Some(_)) => {
                c.add(format!("{at}.run"), "run_speed and run_action go together")
            }
            _ => {}
        }
        for (i, a) in run.skid_turn_with.iter().enumerate() {
            if !input_actions.contains(a.as_str()) {
                c.add(
                    format!("{at}.run.skid_turn_with[{i}]"),
                    format!("no input action `{a}`"),
                );
            } else if ["left", "right"].contains(&a.as_str()) {
                c.add(
                    format!("{at}.run.skid_turn_with[{i}]"),
                    "the directions steer already; name other actions",
                );
            }
        }
    }
    if let Some(run) = &p.run {
        let hook = e.core.ext.is_some();
        if !run.step_cycle.is_empty() && hook {
            c.add(
                format!("{at}.run.step_cycle"),
                "a step cycle counts in the body's mechanic words, which the entity's ext hook keeps; give one of them",
            );
        }
        if !run.air_step_cycle.is_empty() {
            if run.step_cycle.is_empty() {
                c.add(
                    format!("{at}.run.air_step_cycle"),
                    "goes with run.step_cycle",
                );
            }
            if run.air_control {
                c.add(
                    format!("{at}.run.air_step_cycle"),
                    "is read only with air_control = false",
                );
            }
        }
    }
    if let Some(j) = &p.jump {
        if let Some(f) = j.takeoff_fraction
            && !(0..1 << crate::fixed::FRAC_BITS).contains(&f.raw())
        {
            c.add(
                format!("{at}.jump.takeoff_fraction"),
                "a fraction: from 0, below 1",
            );
        }
        if j.fatal_drop.is_some() && e.core.ext.is_some() {
            c.add(
                format!("{at}.jump.fatal_drop"),
                "the takeoff height is kept in the body's mechanic word 3, which the entity's ext hook keeps; give one of them",
            );
        }
        if j.land_hold > 0 && e.core.ext.is_some() {
            c.add(
                format!("{at}.jump.land_hold"),
                "a landing hold counts in the body's mechanic words, which the entity's ext hook keeps; give one of them",
            );
        }
        if !input_actions.contains(j.action.as_str()) {
            c.add(
                format!("{at}.jump.action"),
                format!("no input action `{}`", j.action),
            );
        }
        if j.max_fall <= crate::Fixed::ZERO {
            c.add(
                format!("{at}.jump.max_fall"),
                "must be above zero (down is positive)",
            );
        }
    }
    if let Some(feet) = p.jump.as_ref().and_then(|j| j.feet.as_ref())
        && (feet.is_empty() || feet.iter().any(|x| x.to_int().is_none()))
    {
        c.add(
            format!("{at}.jump.feet"),
            "one or more columns, in whole pixels",
        );
    }
    if let Some(feet) = p.fall.as_ref().and_then(|f| f.feet.as_ref())
        && (feet.is_empty() || feet.iter().any(|x| x.to_int().is_none()))
    {
        c.add(
            format!("{at}.fall.feet"),
            "one or more columns, in whole pixels",
        );
    }
    if let Some([from, to]) = p.jump.as_ref().and_then(|j| j.collide_y)
        && from >= to
    {
        c.add(
            format!("{at}.jump.collide_y"),
            "the first must be below the second",
        );
    }
    if let Some(j) = &p.jump
        && j.start_fall_gravity.is_some()
        && !j.keep_fall_gravity
    {
        c.add(
            format!("{at}.jump.start_fall_gravity"),
            "is read only with keep_fall_gravity = true",
        );
    }
    if let Some(g) = p.jump.as_ref().and_then(|j| j.start_fall_gravity)
        && g <= crate::Fixed::ZERO
    {
        c.add(
            format!("{at}.jump.start_fall_gravity"),
            "must be above zero (down is positive)",
        );
    }
    if let Some(pk) = p.jump.as_ref().and_then(|j| j.pickup.as_ref())
        && pk.points.iter().flatten().any(|v| v.to_int().is_none())
    {
        c.add(format!("{at}.jump.pickup.points"), "are in whole pixels");
    }
    if let Some(hd) = p.jump.as_ref().and_then(|j| j.head.as_ref()) {
        let hat = format!("{at}.jump.head");
        if hd.point.iter().any(|v| v.to_int().is_none()) {
            c.add(format!("{hat}.point"), "is in whole pixels");
        }
        for (i, h) in hd.hits.iter().enumerate() {
            use crate::model::TileKind as K;
            if !matches!(h.tile, K::Solid | K::CoinBlock | K::ItemBlock | K::Brick) {
                c.add(
                    format!("{hat}.hits[{i}].tile"),
                    "a head hit is on a cell that stops movement: solid, coin_block, item_block or brick",
                );
            }
            if h.speed.to_int().is_none() {
                c.add(
                    format!("{hat}.hits[{i}].speed"),
                    "is the speed's whole part: must be whole",
                );
            }
        }
    }
    if let Some(j) = &p.jump {
        let mut last = crate::Fixed::ZERO;
        for (i, b) in j.bands.iter().enumerate() {
            if b.below <= last {
                c.add(
                    format!("{at}.jump.bands[{i}].below"),
                    "bands go in order of `below`, each above the last and above zero",
                );
            }
            last = b.below;
        }
    }
    if let Some(pt) = &p.patrol
        && pt.period == 0
    {
        c.add(format!("{at}.patrol.period"), "must be at least 1");
    }
    if let Some(w) = &p.walk
        && w.speed < crate::Fixed::ZERO
    {
        c.add(
            format!("{at}.walk.speed"),
            "a speed without its sign: zero or above",
        );
    }
    if let Some(f) = &p.fall
        && f.max_fall_from_half
        && f.max_fall.to_int().is_none()
    {
        c.add(
            format!("{at}.fall.max_fall"),
            "with max_fall_from_half the limit compares whole parts: must be whole",
        );
    }
    if let Some(j) = &p.jump
        && j.max_fall_from_half
        && j.max_fall.to_int().is_none()
    {
        c.add(
            format!("{at}.jump.max_fall"),
            "with max_fall_from_half the limit compares whole parts: must be whole",
        );
    }
    if let Some(r) = &p.run
        && r.takeoff_in_air
        && p.jump.is_none()
    {
        c.add(
            format!("{at}.run.takeoff_in_air"),
            "is about the frame a jump starts: needs jump",
        );
    }
    if let Some(f) = &p.fall {
        if f.max_fall <= crate::Fixed::ZERO {
            c.add(
                format!("{at}.fall.max_fall"),
                "must be above zero (down is positive)",
            );
        }
        if p.jump.is_some() {
            c.add(
                format!("{at}.fall"),
                "an entity that jumps falls by its jump's gravity; give one of jump and fall",
            );
        }
    }
    let movers = [
        ("walk", p.walk.is_some()),
        ("patrol", p.patrol.is_some()),
        ("fly", p.fly.is_some()),
        ("path", p.path.is_some()),
        ("oscillate", p.oscillate.is_some()),
        ("shuttle", p.shuttle.is_some()),
    ];
    let given: Vec<&str> = movers.iter().filter(|m| m.1).map(|m| m.0).collect();
    if given.len() > 1 {
        c.add(
            at,
            format!(
                "give one of walk, patrol, fly, path, oscillate and shuttle, not {}",
                given.join(" and ")
            ),
        );
    }
    if let Some(f) = &p.fly
        && f.speed < crate::Fixed::ZERO
    {
        c.add(
            format!("{at}.fly.speed"),
            "a speed without its sign: zero or above",
        );
    }
    if p.hop.is_some() && p.fall.is_none() {
        c.add(format!("{at}.hop"), "an entity that hops needs fall");
    }
    if let Some(h) = &p.hop {
        if h.speed >= crate::Fixed::ZERO {
            c.add(format!("{at}.hop.speed"), "a hop goes up: below zero");
        }
        for (i, s) in h.pattern.iter().enumerate() {
            if *s >= crate::Fixed::ZERO {
                c.add(
                    format!("{at}.hop.pattern[{i}]"),
                    "a hop goes up: below zero",
                );
            }
        }
        if h.every == Some(0) {
            c.add(format!("{at}.hop.every"), "must be at least 1");
        }
        if !h.pattern.is_empty() && e.core.ext.is_some() {
            c.add(
                format!("{at}.hop.pattern"),
                "a hop pattern counts in the body's mechanic words, which the entity's ext hook keeps; give one of them",
            );
        }
    }
    if let Some(o) = &p.oscillate {
        if o.force <= crate::Fixed::ZERO {
            c.add(format!("{at}.oscillate.force"), "must be above zero");
        }
        if o.max_speed <= crate::Fixed::ZERO {
            c.add(format!("{at}.oscillate.max_speed"), "must be above zero");
        }
        if o.low_at.is_some() != o.low_center.is_some() {
            c.add(
                format!("{at}.oscillate"),
                "give low_at and low_center together",
            );
        }
        if p.fall.is_some() && o.axis == crate::platformer::PathAxis::Y {
            c.add(
                format!("{at}.fall"),
                "oscillate on y moves without gravity; leave out fall",
            );
        }
    }
    if let Some(l) = &p.leap {
        if l.gravity <= crate::Fixed::ZERO {
            c.add(format!("{at}.leap.gravity"), "must be above zero");
        }
        if l.every == 0 || l.interval == 0 {
            c.add(
                format!("{at}.leap"),
                "every and interval must be at least 1",
            );
        }
        if l.beat.is_some() && (l.waits.is_empty() || l.rise.is_some()) {
            c.add(
                format!("{at}.leap.beat"),
                "a wait on a beat needs `waits`, and no `rise`",
            );
        }
        if l.draw.is_some() && l.beat.is_none() {
            c.add(format!("{at}.leap.draw"), "needs `beat`");
        }
        if l.max_fall_from_half && l.max_fall.is_none_or(|m| m.to_int().is_none()) {
            c.add(
                format!("{at}.leap.max_fall"),
                "with max_fall_from_half the limit compares whole parts: must be whole",
            );
        }
        if let Some(r) = &l.rise
            && (r.speed >= crate::Fixed::ZERO || r.height <= crate::Fixed::ZERO)
        {
            c.add(
                format!("{at}.leap.rise"),
                "a rise goes up (speed below zero) by a height above zero",
            );
        }
        if p.fall.is_some()
            || p.path
                .as_ref()
                .is_some_and(|pa| pa.axis == crate::platformer::PathAxis::Y)
            || p.oscillate
                .as_ref()
                .is_some_and(|o| o.axis == crate::platformer::PathAxis::Y)
        {
            c.add(
                format!("{at}.leap"),
                "a leap moves y by itself; leave out fall, and path and oscillate on y",
            );
        }
        if e.core.ext.is_some() {
            c.add(
                format!("{at}.leap"),
                "a leap keeps its wait and phase in the body's mechanic words, which the entity's ext hook keeps; give one of them",
            );
        }
    }
    if let Some(sh) = &p.shuttle {
        if sh.every == 0 || sh.sway.as_ref().is_some_and(|w| w.half == 0) {
            c.add(
                format!("{at}.shuttle"),
                "every and sway.half must be at least 1",
            );
        }
        if p.fall.is_some() {
            c.add(
                format!("{at}.fall"),
                "a shuttle moves without gravity; leave out fall",
            );
        }
        if e.core.ext.is_some()
            || p.leap.is_some()
            || p.hop.as_ref().is_some_and(|h| !h.pattern.is_empty())
        {
            c.add(
                format!("{at}.shuttle"),
                "a shuttle keeps its count and way in the body's mechanic words; no ext hook, leap or hop pattern beside it",
            );
        }
    }
    // Platformer 0.4.4's springboard.
    if let Some(sb) = &p.springboard {
        let at = format!("{at}.springboard");
        let zero = crate::Fixed::ZERO;
        if sb.steps.is_empty() {
            c.add(format!("{at}.steps"), "give at least one step");
        }
        if sb.steps.iter().any(|st| st.frames == 0) {
            c.add(format!("{at}.steps"), "each step lasts at least 1 frame");
        }
        if sb.speed >= zero {
            c.add(format!("{at}.speed"), "a throw goes up: below zero");
        }
        if let Some(b) = &sb.boost {
            if b.speed >= zero {
                c.add(format!("{at}.boost.speed"), "a throw goes up: below zero");
            }
            if !input_actions.contains(b.action.as_str()) {
                c.add(
                    format!("{at}.boost.action"),
                    format!("no input action `{}`", b.action),
                );
            }
            if b.from_step as usize >= sb.steps.len().max(1) {
                c.add(
                    format!("{at}.boost.from_step"),
                    "counts steps from 0: must name one of the steps",
                );
            }
        }
        if e.core.role == "player" {
            c.add(
                at.clone(),
                "a springboard holds the player: give it to another entity",
            );
        }
        if e.core.ext.is_some() {
            c.add(
                at.clone(),
                "a springboard keeps its press in the body's mechanic words; give it no ext hook",
            );
        }
        let movers = [
            ("fall", p.fall.is_some()),
            ("walk", p.walk.is_some()),
            ("patrol", p.patrol.is_some()),
            ("fly", p.fly.is_some()),
            ("path", p.path.is_some()),
            ("oscillate", p.oscillate.is_some()),
            ("leap", p.leap.is_some()),
            ("shuttle", p.shuttle.is_some()),
            ("hop", p.hop.is_some()),
        ];
        for (name, given) in movers {
            if given {
                c.add(
                    at.clone(),
                    format!("a springboard stays where it's placed; leave out {name}"),
                );
            }
        }
    }
    if let Some(o) = &p.orbit {
        if o.angles < 4 || o.angles % 4 != 0 {
            c.add(format!("{at}.orbit.angles"), "a multiple of 4, at least 4");
        }
        if e.core.ext.is_some() {
            c.add(
                format!("{at}.orbit"),
                "an orbit keeps its angle in the body's mechanic words; give it no ext hook",
            );
        }
    }
    if p.orbit_part.is_some() {
        for t in ["offsets", "rows", "signs"] {
            if !e.core.ext_tables.contains_key(t) {
                c.add(
                    format!("{at}.ext_tables"),
                    format!("an orbit part reads the table `{t}` from its ext_tables"),
                );
            }
        }
        if e.core.ext.is_some() {
            c.add(
                format!("{at}.orbit_part"),
                "an orbit part keeps its pivot in the body's mechanic words; give it no ext hook",
            );
        }
    }
    if let Some(l) = &p.leave_view
        && l.margin < crate::Fixed::ZERO
    {
        c.add(format!("{at}.leave_view.margin"), "must be zero or above");
    }
    if let Some(l) = &p.leave_view
        && l.bottom_margin.is_some_and(|m| m < crate::Fixed::ZERO)
    {
        c.add(
            format!("{at}.leave_view.bottom_margin"),
            "must be zero or above",
        );
    }
    // Since platformer 0.4.10 a flier may fall too: a projectile thrown
    // in an arc, as Super Mario Bros.' fireball.
    if p.path.is_some() && p.fall.is_some() {
        c.add(
            format!("{at}.fall"),
            "a path moves without gravity; leave out fall",
        );
    }
    if let Some(pa) = &p.path {
        if pa.speed <= crate::Fixed::ZERO {
            c.add(format!("{at}.path.speed"), "must be above zero");
        }
        if pa.distance == crate::Fixed::ZERO {
            c.add(format!("{at}.path.distance"), "must not be zero");
        }
        if pa.wait_near.is_some_and(|n| n < crate::Fixed::ZERO) {
            c.add(format!("{at}.path.wait_near"), "must be zero or above");
        }
        if pa.phase > 0 && pa.phase >= pa.every.max(1) {
            c.add(format!("{at}.path.phase"), "must be below every");
        }
    }
    if let Some(r) = &p.run
        && r.run_acceleration.is_some()
        && r.run_speed.is_none()
    {
        c.add(
            format!("{at}.run.run_acceleration"),
            "needs run_speed and run_action",
        );
    }
    let want_action = |c: &mut Checker, at: String, a: &str| {
        if !input_actions.contains(a) {
            c.add(at, format!("no input action `{a}`"));
        }
    };
    if let Some(cl) = &p.climb {
        want_action(c, format!("{at}.climb.up"), &cl.up);
        want_action(c, format!("{at}.climb.down"), &cl.down);
        for (k, a) in [("release", &cl.release), ("hold_still", &cl.hold_still)] {
            if let Some(a) = a {
                want_action(c, format!("{at}.climb.{k}"), a);
            }
        }
        if cl.speed <= crate::Fixed::ZERO {
            c.add(format!("{at}.climb.speed"), "must be above zero");
        }
        if !cl.up_cycle.is_empty()
            && !cl.down_cycle.is_empty()
            && cl.up_cycle.len() != cl.down_cycle.len()
        {
            c.add(
                format!("{at}.climb.down_cycle"),
                "must be as long as up_cycle: both count the same step counter",
            );
        }
        if cl.down_through.is_some() && !cl.by_feet {
            c.add(format!("{at}.climb.down_through"), "needs by_feet");
        }
        if cl.down_through == Some(0) {
            c.add(format!("{at}.climb.down_through"), "must be above 0");
        }
        let walk = p.run.as_ref().map_or(0, |r| r.step_cycle.len());
        for (k, cycle) in [("up_cycle", &cl.up_cycle), ("down_cycle", &cl.down_cycle)] {
            if walk > 0 && !cycle.is_empty() && cycle.len() != walk {
                c.add(
                    format!("{at}.climb.{k}"),
                    "must be as long as run.step_cycle: both count the same step counter",
                );
            }
        }
        if (!cl.up_cycle.is_empty() || !cl.down_cycle.is_empty()) && e.core.ext.is_some() {
            c.add(
                format!("{at}.climb"),
                "a step cycle keeps its count in mechanic word 0, which an ext hook owns",
            );
        }
    }
    if let Some(sh) = &p.shoot {
        match (&sh.action, sh.every) {
            (Some(a), None) => want_action(c, format!("{at}.shoot.action"), a),
            (None, Some(n)) if n > 0 => {}
            _ => c.add(
                format!("{at}.shoot"),
                "give one of `action` and a nonzero `every`",
            ),
        }
        if sh.max == 0 {
            c.add(format!("{at}.shoot.max"), "must be at least 1");
        }
    }
    if let Some(cr) = &p.carry {
        want_action(c, format!("{at}.carry.action"), &cr.action);
        if cr.throw_moving.is_some() != cr.moving_above.is_some() {
            c.add(
                format!("{at}.carry"),
                "throw_moving and moving_above go together",
            );
        }
    }
    if let Some(k) = &p.knockback
        && k.speed < crate::Fixed::ZERO
    {
        c.add(
            format!("{at}.knockback.speed"),
            "a speed without its sign: zero or above",
        );
    }
    if let Some(r) = &p.run
        && r.start.as_ref().is_some_and(|s| s.frames == 0)
    {
        c.add(format!("{at}.run.start.frames"), "must be at least 1");
    }
    if let Some(r) = &p.run {
        run_options(c, &format!("{at}.run"), r);
    }
    if let Some(ct) = &p.contact {
        if ct.stand && ct.stomp_depth.is_none() {
            c.add(
                format!("{at}.contact.stand"),
                "standing lands within stomp_depth of the top; give stomp_depth",
            );
        }
        if ct.ride.is_some() && !ct.stand {
            c.add(format!("{at}.contact.ride"), "riding needs stand");
        }
        if ct.stomp_depth.is_some() != ct.bounce.is_some() && !ct.stand {
            c.add(
                format!("{at}.contact"),
                "stomp_depth and bounce go together",
            );
        }
        if ct.every == Some(0) {
            c.add(format!("{at}.contact.every"), "must be at least 1 frame");
        }
        if ct.phase > 0 && ct.phase >= ct.every.unwrap_or(1) {
            c.add(format!("{at}.contact.phase"), "must be below `every`");
        }
        if ct.keep_on_stomp && ct.stomp_depth.is_none() {
            c.add(
                format!("{at}.contact.keep_on_stomp"),
                "the entity can't be stomped without stomp_depth",
            );
        }
    }
    if let Some(h) = &p.hits {
        if h.every == Some(0) {
            c.add(format!("{at}.hits.every"), "must be at least 1 frame");
        }
        if h.phase > 0 && h.phase >= h.every.unwrap_or(1) {
            c.add(format!("{at}.hits.phase"), "must be below `every`");
        }
    }
    if let Some(b) = &p.bounce
        && b.speed >= crate::Fixed::ZERO
    {
        c.add(
            format!("{at}.bounce.speed"),
            "a bounce goes up: must be below zero",
        );
    }
}

/// Checks one state machine: the game's (`rules.<name>`) or an entity's.
fn machine(
    c: &mut Checker,
    model: &Model,
    at: &str,
    m: &Machine,
    events: &BTreeSet<&str>,
    actions: &BTreeSet<&str>,
    entity: bool,
) {
    if !m.states.contains_key(&m.initial) {
        c.add(
            format!("{at}.initial"),
            format!("no state named `{}`", m.initial),
        );
    }
    for (sname, s) in &m.states {
        let sat = format!("{at}.states.{sname}");
        c.name(&sat, sname);
        if !entity && !s.set.is_empty() {
            c.add(format!("{sat}.set"), "only an entity's state sets fields");
        }
        if s.ground.is_some() && !entity {
            c.add(
                format!("{sat}.ground"),
                "only an entity's state sets its standing",
            );
        }
        if s.enter_speed.is_some() {
            if !entity {
                c.add(
                    format!("{sat}.enter_speed"),
                    "only an entity's state sets its speed",
                );
            }
        } else if s.enter_keeps_fraction {
            c.add(
                format!("{sat}.enter_keeps_fraction"),
                "is for a state with `enter_speed`",
            );
        }
        for (key, list) in [
            ("stops", &s.stops),
            ("stops_at_once", &s.stops_at_once),
            ("frees_at_once", &s.frees_at_once),
        ] {
            for name in list {
                if !model.model.systems.iter().any(|x| x == name)
                    || matches!(name.as_str(), "rules" | "entity_states")
                {
                    c.add(
                        format!("{sat}.{key}"),
                        format!("`{name}` isn't a system of model.systems that moves things"),
                    );
                }
            }
        }
        for name in &s.stops_at_once {
            if !s.freeze && !s.stops.contains(name) {
                c.add(
                    format!("{sat}.stops_at_once"),
                    format!("`{name}` must be one of the systems the state stops"),
                );
            }
        }
        for name in &s.frees_at_once {
            if entity {
                c.add(
                    format!("{sat}.frees_at_once"),
                    "only a game machine's state frees systems as it is left",
                );
            } else if !s.freeze && !s.stops.contains(name) {
                c.add(
                    format!("{sat}.frees_at_once"),
                    format!("`{name}` must be one of the systems the state stops"),
                );
            }
        }
        if s.step_word == Some(0) {
            c.add(format!("{sat}.step_word"), "must be at least 1");
        }
        if !entity && s.step_word.is_some() {
            c.add(
                format!("{sat}.step_word"),
                "only an entity's state has step_word",
            );
        }
        if s.seen_every == Some(0) {
            c.add(format!("{sat}.seen_every"), "must be at least 1 frame");
        }
        if !entity && s.seen_every.is_some() {
            c.add(
                format!("{sat}.seen_every"),
                "only an entity's state has seen_every",
            );
        }
        for name in &s.holds {
            if !model.counters.contains_key(name) {
                c.add(format!("{sat}.holds"), format!("no counter named `{name}`"));
            }
        }
        for (i, t) in s.transition.iter().enumerate() {
            let tat = format!("{sat}.transition[{i}]");
            match (t.align, t.after) {
                (Some(0), _) => c.add(format!("{tat}.align"), "must be at least 1 frame"),
                (Some(_), None) => c.add(format!("{tat}.align"), "needs `after`"),
                _ => {}
            }
            if !m.states.contains_key(&t.to) {
                c.add(format!("{tat}.to"), format!("no state named `{}`", t.to));
            }
            if let Some(name) = &t.align_to {
                if t.after.is_none() {
                    c.add(format!("{tat}.align_to"), "needs `after`");
                }
                match model.counters.get(name) {
                    None => c.add(
                        format!("{tat}.align_to"),
                        format!("no counter named `{name}`"),
                    ),
                    Some(k) if !k.change.iter().any(|ch| ch.every.is_some()) => c.add(
                        format!("{tat}.align_to"),
                        format!("counter `{name}` has no `every` change, so no clock"),
                    ),
                    Some(_) => {}
                }
            }
            if t.below.is_some() {
                if !entity {
                    c.add(format!("{tat}.below"), "belongs to an entity's machine");
                }
                if t.on.is_some() {
                    c.add(&tat, "give `below` or `on`, not both");
                }
                if t.after == Some(0) {
                    c.add(format!("{tat}.after"), "must be at least 1 frame");
                }
            } else if let Some(a) = t.pressed.as_ref().or(t.held.as_ref()) {
                let key = if t.pressed.is_some() {
                    "pressed"
                } else {
                    "held"
                };
                if t.on.is_some() {
                    c.add(format!("{tat}.{key}"), "goes without `on`");
                }
                if t.pressed.is_some() && t.held.is_some() {
                    c.add(&tat, "give `pressed` or `held`, not both");
                }
                if !model
                    .input
                    .as_ref()
                    .is_some_and(|i| i.actions.contains_key(a))
                {
                    c.add(
                        format!("{tat}.{key}"),
                        format!("the input has no action `{a}`"),
                    );
                }
            } else {
                match (&t.on, t.after) {
                    (Some(ev), None) => {
                        if !events.contains(ev.as_str()) {
                            c.add(
                            format!("{tat}.on"),
                            format!("`{ev}` isn't an event of the core, a counter or a declared extension"),
                        );
                        }
                    }
                    (None, Some(0)) => c.add(format!("{tat}.after"), "must be at least 1 frame"),
                    (None, Some(_)) => {}
                    _ => c.add(&tat, "give exactly one of `on` and `after`"),
                }
            }
            if let Some(g) = &t.guard
                && !model.counters.contains_key(&g.counter)
            {
                c.add(
                    format!("{tat}.if.counter"),
                    format!("no counter named `{}`", g.counter),
                );
            }
            for a in &t.actions {
                if let Some(e) = a.strip_prefix(BECOME)
                    && entity
                {
                    match model.entities.get(e) {
                        None => c.add(format!("{tat}.do"), format!("`{a}`: no entity named `{e}`")),
                        Some(target) if target.core.role == "player" => c.add(
                            format!("{tat}.do"),
                            format!("`{a}`: an entity can't become the player"),
                        ),
                        Some(_) => {}
                    }
                    continue;
                }
                if let Some(ev) = a.strip_prefix(EMIT) {
                    if !events.contains(ev) {
                        c.add(
                            format!("{tat}.do"),
                            format!("`{a}`: `{ev}` isn't an event of the core, a counter, a declared extension or the model"),
                        );
                    }
                    continue;
                }
                if let Some(name) = a.strip_prefix(CLEAR).or_else(|| a.strip_prefix(RESPAWN)) {
                    if !model.entities.contains_key(name) {
                        c.add(
                            format!("{tat}.do"),
                            format!("`{a}`: no entity named `{name}`"),
                        );
                    }
                    continue;
                }
                if let Some(name) = a.strip_prefix(REMOVE_BODY) {
                    if !model.counters.contains_key(name) {
                        c.add(
                            format!("{tat}.do"),
                            format!("`{a}`: no counter named `{name}`"),
                        );
                    }
                    continue;
                }
                if a.starts_with(SET) {
                    match set_action(a) {
                        Some((name, _)) if model.counters.contains_key(name) => {}
                        Some((name, _)) => c.add(
                            format!("{tat}.do"),
                            format!("`{a}`: no counter named `{name}`"),
                        ),
                        None => c.add(
                            format!("{tat}.do"),
                            format!("`{a}` isn't `set:<counter>:<value>`"),
                        ),
                    }
                    continue;
                }
                if a.starts_with(ADD) {
                    match add_action(a) {
                        Some((to, from)) => {
                            for name in [to, from] {
                                if !model.counters.contains_key(name) {
                                    c.add(
                                        format!("{tat}.do"),
                                        format!("`{a}`: no counter named `{name}`"),
                                    );
                                }
                            }
                        }
                        None => c.add(
                            format!("{tat}.do"),
                            format!("`{a}` isn't `add:<counter>:<counter>`"),
                        ),
                    }
                    continue;
                }
                if a.starts_with(CHANGE) {
                    match change_action(a) {
                        Some((name, _)) if model.counters.contains_key(name) => {}
                        Some((name, _)) => c.add(
                            format!("{tat}.do"),
                            format!("`{a}`: no counter named `{name}`"),
                        ),
                        None => c.add(
                            format!("{tat}.do"),
                            format!("`{a}` isn't `change:<counter>:<by>`"),
                        ),
                    }
                    continue;
                }
                if !actions.contains(a.as_str()) {
                    c.add(
                        format!("{tat}.do"),
                        format!("`{a}` isn't an action of the core or a declared extension"),
                    );
                }
            }
            if t.actions
                .iter()
                .any(|a| a == "next-level" || a == "enter-level")
                && model.stages.is_none()
                && model.levels.values().all(|l| l.core.next.is_none())
            {
                c.add(
                    format!("{tat}.do"),
                    "next-level needs a level with `next`, or `[stages]`",
                );
            }
        }
    }
}

/// The evidence entry that covers each fact: the most specific entry whose
/// path names the fact or a part it's in. Facts no entry covers map to
/// `None`.
pub fn evidence_for(model: &Model) -> Vec<(Concrete, Option<String>)> {
    let tree = model.to_tree();
    let mut covered: BTreeMap<Concrete, ((usize, usize), String)> = BTreeMap::new();
    for key in model.evidence.keys() {
        let Ok(p) = Path::parse(key) else { continue };
        let spec = p.specificity();
        for at in p.resolve(&tree) {
            let slot = covered.entry(at).or_insert((spec, key.clone()));
            if spec > slot.0 {
                *slot = (spec, key.clone());
            }
        }
    }
    crate::facts(&tree)
        .into_iter()
        .map(|(fact, _)| {
            let best = (0..=fact.len())
                .rev()
                .find_map(|n| covered.get(&fact[..n]))
                .map(|(_, k)| k.clone());
            (fact, best)
        })
        .collect()
}

fn check_evidence(model: &Model, c: &mut Checker) {
    let tree = model.to_tree();
    let facts: Vec<Concrete> = crate::facts(&tree).into_iter().map(|(f, _)| f).collect();
    for (key, ev) in &model.evidence {
        let at = format!("evidence.\"{key}\"");
        match Path::parse(key) {
            Err(e) => c.add(&at, e),
            Ok(p) if p.resolve(&tree).is_empty() => c.add(&at, "names no part of the model"),
            Ok(p)
                if p.segments
                    .first()
                    .is_some_and(|s| s.name == "model" || s.name == "evidence") =>
            {
                c.add(&at, "the header and the evidence table aren't facts")
            }
            Ok(p)
                if !p
                    .resolve(&tree)
                    .iter()
                    .any(|part| holds_facts(&facts, part)) =>
            {
                c.add(
                    &at,
                    "names no fact (descriptions and items of a plain list aren't facts)",
                )
            }
            Ok(_) => {}
        }
        if ev.confidence > 100 {
            c.add(&at, "confidence is a percentage, 0 to 100");
        }
        for (i, r) in ev.refs.iter().enumerate() {
            if let Some(w) = &r.within
                && pinned(w).is_none()
            {
                c.add(
                    format!("{at}.refs[{i}].in"),
                    format!("`{w}` isn't `<name>@<revision>`"),
                );
            }
        }
        if matches!(ev.source, Source::Observed | Source::Stated) && ev.refs.is_empty() {
            c.add(
                &at,
                format!("{} evidence needs at least one ref", ev.source.name()),
            );
        }
    }
    for (fact, by) in evidence_for(model) {
        if by.is_none() {
            c.add(path::display(&fact), "no evidence covers this fact");
        }
    }
}

/// A ref's `in`, `<name>@<revision>`, split. The name is a name in the path
/// grammar and the revision letters and digits.
pub fn pinned(within: &str) -> Option<(&str, &str)> {
    let (name, rev) = within.split_once('@')?;
    (path::is_name(name) && !rev.is_empty() && rev.bytes().all(|b| b.is_ascii_alphanumeric()))
        .then_some((name, rev))
}

/// A ref of the form `path:line`: a path with a `/` or a `.` in it (so
/// `recording:68fdf735` isn't one), then a line number.
pub fn file_ref(at: &str) -> Option<(&str, usize)> {
    let (file, line) = at.rsplit_once(':')?;
    let line = line.parse().ok()?;
    (file.contains('/') || file.contains('.')).then_some((file, line))
}

/// Where a ref's file is read from, as [`check_refs_in`] asks.
pub enum RefRoot {
    /// Read it under this directory.
    Dir(std::path::PathBuf),
    /// Don't check it (a pinned download that isn't there).
    Skip,
    /// It can't be checked, for this reason (an unknown download).
    Bad(String),
}

/// Checks each evidence ref of the form `file:line` against the files under
/// `root`: the file exists, has that line, and the line contains the quote.
/// Refs of other forms (recording IDs) aren't checked, and neither are refs
/// into a pinned download (`in`); [`check_refs_in`] checks those.
pub fn check_refs(model: &Model, root: &FsPath) -> Vec<Problem> {
    check_refs_in(model, |within, _| match within {
        None => RefRoot::Dir(root.to_path_buf()),
        Some(_) => RefRoot::Skip,
    })
    .0
}

/// Checks every `file:line` ref, reading each from the directory `root_for`
/// gives for its `in` (`None` for the repo) and file. Returns the problems and the
/// number of refs skipped.
pub fn check_refs_in(
    model: &Model,
    mut root_for: impl FnMut(Option<&str>, &str) -> RefRoot,
) -> (Vec<Problem>, usize) {
    let mut out = Vec::new();
    let mut skipped = 0;
    let mut texts: BTreeMap<std::path::PathBuf, Result<String, String>> = BTreeMap::new();
    for (key, ev) in &model.evidence {
        for r in &ev.refs {
            let at = format!("evidence.\"{key}\"");
            let Some((file, line)) = file_ref(&r.at) else {
                // Something with a `/` is meant as a file and needs a line.
                if r.at.contains('/') || r.within.is_some() {
                    out.push(Problem {
                        path: at,
                        message: format!("{}: a file ref is `path:line`", r.at),
                    });
                }
                continue;
            };
            let mut bad = |m: String| {
                out.push(Problem {
                    path: at.clone(),
                    message: format!("{}: {m}", r.at),
                })
            };
            if file.starts_with('/') || file.split('/').any(|p| p == "..") {
                bad("refs are relative to their root and stay inside it".into());
                continue;
            }
            let root = match root_for(r.within.as_deref(), file) {
                RefRoot::Dir(d) => d,
                RefRoot::Skip => {
                    skipped += 1;
                    continue;
                }
                RefRoot::Bad(m) => {
                    bad(m);
                    continue;
                }
            };
            let full = root.join(file);
            let text = texts
                .entry(full.clone())
                .or_insert_with(|| std::fs::read_to_string(&full).map_err(|e| e.to_string()));
            let text = match text {
                Ok(t) => t,
                Err(e) => {
                    bad(format!("can't read it ({e})"));
                    continue;
                }
            };
            match text.lines().nth(line.wrapping_sub(1)) {
                None => bad(format!("the file has {} lines", text.lines().count())),
                Some(l) => {
                    if let Some(q) = &r.quote
                        && !l.contains(q.as_str())
                    {
                        bad(format!(
                            "the line doesn't contain `{q}`; it reads `{}`",
                            l.trim()
                        ));
                    }
                }
            }
        }
    }
    (out, skipped)
}
