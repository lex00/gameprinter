//! A capture written into a model: the sprite-set slot and any tiles
//! slots its frames use, each captured entity's sprite and each state's,
//! and evidence naming the recordings and tables. A sprite the model
//! already has is kept: its frames move into the set (a row of a `rows`
//! slot, or tiles, become the same layout as a set frame), its clips stay
//! first, and captured clips for conditions it doesn't name are added
//! before them, so the most specific holds first.

use std::collections::BTreeMap;
use std::path::Path;

use gp_assets::sprite_set::{Piece, SetFrame, SpriteSet};
use gp_model::Model;
use gp_model::model::{
    AssetKind, AssetSlot, AssetSource, Cell, Clip, ClipFrame, Evidence, Ref, RowRef, Source, Sprite,
};

use crate::build::Capture;
use crate::chr::TileBank;

/// What a write changed, for the report.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Written {
    pub entities: Vec<String>,
    pub states: Vec<String>,
    pub slots_added: Vec<String>,
    pub clips_added: usize,
    /// Sprites the capture agreed with, left as they were.
    pub agreed: Vec<String>,
}

/// A sprite's frame as a set frame: its tiles from a row or a list, laid
/// out by its cells (or rows of `width`), stood on the entity's bottom
/// edge as a sprite without a set is drawn.
fn as_layout(
    sprite: &Sprite,
    nums: &[u32],
    cells: &[Cell],
    height: i32,
    bank: &TileBank,
) -> Vec<Piece> {
    let w = sprite.width.max(1);
    let place = |i: usize| -> [u32; 2] {
        cells
            .get(i)
            .and_then(|c| c.at)
            .unwrap_or([i as u32 % w, i as u32 / w])
    };
    let rows = (0..nums.len()).map(|i| place(i)[1] + 1).max().unwrap_or(1) as i32;
    let blank = |t: u32| {
        bank.assets
            .iter()
            .find(|a| a.slot.as_deref() == Some(&sprite.slot))
            .and_then(|a| a.tiles.get(t as usize))
            .is_some_and(|px| px.iter().all(|&c| c == 0))
    };
    let mut out: Vec<Piece> = nums
        .iter()
        .enumerate()
        .filter(|(_, t)| !blank(**t))
        .map(|(i, &t)| {
            let c = cells.get(i).cloned().unwrap_or_default();
            let [cx, cy] = place(i);
            Piece {
                tile: t,
                slot: Some(sprite.slot.clone()),
                x: cx as i32 * 8,
                y: cy as i32 * 8 + height - rows * 8,
                palette: c.palette.unwrap_or(sprite.palette) as u8,
                flip_x: c.flip_x,
                flip_y: c.flip_y,
            }
        })
        .collect();
    out.sort();
    out
}

/// A layout from its top left corner: what two placements of one frame
/// share.
fn shape(l: &[Piece]) -> Vec<Piece> {
    let x0 = l.iter().map(|p| p.x).min().unwrap_or(0);
    let y0 = l.iter().map(|p| p.y).min().unwrap_or(0);
    let mut v: Vec<Piece> = l
        .iter()
        .map(|p| Piece {
            x: p.x - x0,
            y: p.y - y0,
            ..p.clone()
        })
        .collect();
    v.sort();
    v
}

/// The set frame for a layout, added when the set doesn't have it. A frame
/// of the same entity with the same shape is the same frame, placed as
/// capture placed it.
fn frame_in(set: &mut SpriteSet, entity: &str, state: &str, layout: Vec<Piece>) -> u32 {
    let want = shape(&layout);
    // The same tiles in the same places, flips aside: a hand-written frame
    // that missed a mirrored tile is the frame capture saw. Only a frame
    // capture has no match for is added, after capture's own, so the
    // set's numbering stays what a capture from the tables and recordings
    // alone gives.
    let loose = |l: &[Piece]| -> Vec<(Option<String>, u32, i32, i32)> {
        let mut v: Vec<_> = shape(l)
            .into_iter()
            .map(|p| (p.slot, p.tile, p.x, p.y))
            .collect();
        v.sort();
        v
    };
    let found = set
        .frames
        .iter()
        .position(|f| f.colors.is_empty() && (f.pieces == layout || shape(&f.pieces) == want))
        .or_else(|| {
            let w = loose(&layout);
            set.frames.iter().position(|f| {
                f.colors.is_empty()
                    && f.entity == entity
                    && !f.pieces.is_empty()
                    && loose(&f.pieces) == w
            })
        });
    if let Some(i) = found {
        return i as u32;
    }
    set.frames.push(SetFrame {
        entity: entity.into(),
        state: state.into(),
        from: vec!["model".into()],
        pieces: layout,
        colors: Vec::new(),
    });
    (set.frames.len() - 1) as u32
}

/// Moves a sprite that isn't in the set yet into it.
#[allow(clippy::too_many_arguments)]
fn into_set(
    sprite: &Sprite,
    set_slot: &str,
    set: &mut SpriteSet,
    rows: &dyn Fn(&str, u32) -> Option<Vec<u32>>,
    entity: &str,
    state: &str,
    height: i32,
    bank: &TileBank,
) -> Option<Sprite> {
    let in_set = |s: &Sprite| s.frame.as_ref().is_some_and(|f| f.slot == set_slot);
    if in_set(sprite) {
        return Some(sprite.clone());
    }
    let nums = |row: Option<u32>, tiles: &[u32]| -> Option<Vec<u32>> {
        if !tiles.is_empty() {
            return Some(tiles.to_vec());
        }
        match (&sprite.frame, row) {
            (Some(f), Some(r)) => rows(&f.slot, r),
            (Some(f), None) => rows(&f.slot, f.row),
            (None, _) => Some(sprite.tiles.clone()),
        }
    };
    let base_nums = nums(None, &[])?;
    let base = frame_in(
        set,
        entity,
        state,
        as_layout(sprite, &base_nums, &sprite.cells, height, bank),
    );
    let mut out = sprite.clone();
    out.tiles.clear();
    out.cells.clear();
    out.width = 1;
    out.frame = Some(RowRef {
        slot: set_slot.into(),
        row: base,
    });
    for c in &mut out.clips {
        for f in &mut c.frames {
            let n = if f.row.is_none() && f.tiles.is_empty() {
                base_nums.clone()
            } else {
                nums(f.row, &f.tiles)?
            };
            let cells = if f.cells.is_empty() {
                &sprite.cells
            } else {
                &f.cells
            };
            let id = frame_in(
                set,
                entity,
                state,
                as_layout(sprite, &n, cells, height, bank),
            );
            *f = ClipFrame {
                row: Some(id),
                flip_x: f.flip_x,
                flip_y: f.flip_y,
                ..ClipFrame::default()
            };
        }
    }
    Some(out)
}

/// Writes the capture into the model. `rows` reads a row of a `rows` slot
/// from the lifted assets; `source` is the model's source ID; `refs` are
/// the evidence's references (recordings, the template's sprites.toml).
#[allow(clippy::too_many_arguments)]
pub fn write(
    model: &mut Model,
    cap: &mut Capture,
    set_slot: &str,
    source: &str,
    rows: &dyn Fn(&str, u32) -> Option<Vec<u32>>,
    bank: &TileBank,
    refs: &[Ref],
    lifted: &Path,
) -> Written {
    let _ = lifted;
    let mut w = Written::default();
    let captured_total = cap.set.frames.len();
    model.assets.entry(set_slot.to_owned()).or_insert_with(|| {
        w.slots_added.push(set_slot.to_owned());
        AssetSlot {
            kind: AssetKind::SpriteSet,
            role: "graphics".into(),
            description: Some(
                "Every sprite frame of every entity, from the sprite tables and the recordings (gp sprites capture, docs/formats/sprite-set.md)".into(),
            ),
            from: Some(AssetSource {
                source: source.to_owned(),
                asset: cap.set.asset.clone(),
            }),
            ..default_slot()
        }
    });
    // Tiles slots the frames name.
    let mut names: Vec<String> = cap
        .set
        .frames
        .iter()
        .flat_map(|f| f.pieces.iter().filter_map(|p| p.slot.clone()))
        .collect();
    names.sort();
    names.dedup();
    for n in names {
        if model.assets.contains_key(&n) {
            continue;
        }
        w.slots_added.push(n.clone());
        model.assets.insert(
            n.clone(),
            AssetSlot {
                kind: AssetKind::Tiles,
                role: "graphics".into(),
                description: Some(
                    "Sprite tiles a captured frame shows (gp sprites capture)".into(),
                ),
                from: Some(AssetSource {
                    source: source.to_owned(),
                    asset: n.clone(),
                }),
                ..default_slot()
            },
        );
    }
    let first_tiles = |cap: &Capture, frame: u32| -> Option<String> {
        cap.set
            .frames
            .get(frame as usize)?
            .pieces
            .first()?
            .slot
            .clone()
    };
    for (name, es) in &cap.entities {
        let Some(entity) = model.entities.get(name).cloned() else {
            continue;
        };
        let height = (entity.core.size[1].raw() >> 16) as i32;
        let mut touched = false;
        for (state, (base, clips)) in &es.states {
            // The sprite the model gives this state itself, if any.
            let own = if state.is_empty() {
                entity.core.sprite.clone()
            } else {
                let has = entity
                    .core
                    .machine
                    .as_ref()
                    .and_then(|m| m.states.get(state))
                    .is_some_and(|st| st.set.contains_key("sprite"));
                if !has {
                    None
                } else {
                    entity.in_state(state).ok().and_then(|e| e.core.sprite)
                }
            };
            let own_in_set = own
                .as_ref()
                .is_some_and(|s| s.frame.as_ref().is_some_and(|f| f.slot == set_slot));
            let mut current = own.and_then(|s| {
                into_set(&s, set_slot, &mut cap.set, rows, name, state, height, bank)
            });
            // A hand-written frame no recording or table showed (half of a
            // boss drawn in two halves) gives way to the frame capture saw
            // most.
            if let Some(c) = &mut current
                && let Some(f) = &mut c.frame
                && f.row as usize >= captured_total
                && (*base as usize) < captured_total
            {
                f.row = *base;
            }
            let inherited = entity.core.sprite.as_ref();
            let mut sprite = current.clone().unwrap_or_else(|| Sprite {
                slot: first_tiles(cap, *base)
                    .or_else(|| inherited.map(|s| s.slot.clone()))
                    .unwrap_or_default(),
                tiles: Vec::new(),
                frame: Some(RowRef {
                    slot: set_slot.into(),
                    row: *base,
                }),
                width: 1,
                palette: inherited.map_or(4, |s| s.palette),
                flip_when_facing_left: inherited.map_or(es.faces_both_ways, |s| {
                    s.flip_when_facing_left || es.faces_both_ways
                }),
                copy: inherited.and_then(|s| s.copy.clone()),
                cells: Vec::new(),
                colors: inherited.and_then(|s| s.colors.clone()),
                clips: Vec::new(),
                blink: None,
                offset: None,
                behind: false,
                behind_when: None,
                split: None,
                replaced: None,
                leaves: None,
                front: false,
                name_table: false,
                flip_in_place: inherited.is_some_and(|s| s.flip_in_place),
            });
            if sprite.slot.is_empty() {
                continue;
            }
            if state.is_empty() && sprite.blink.is_none() {
                sprite.blink = es
                    .blink
                    .clone()
                    .map(|(when, every)| gp_model::model::Blink { when, every });
            }
            let seen = es.seen.get(state).cloned().unwrap_or_default();
            let (before, after) =
                merge_clips(&sprite.clips, clips, &seen, current.is_some(), &cap.set);
            // A sprite the capture agrees with stays as the model has it:
            // nothing new to show, so nothing moves into the set.
            let agrees = current.is_some()
                && before.is_empty()
                && after.is_empty()
                && sprite.blink.is_none()
                && sprite
                    .frame
                    .as_ref()
                    .and_then(|f| cap.set.frames.get(f.row as usize))
                    .map(|f| &f.pieces)
                    == cap.set.frames.get(*base as usize).map(|f| &f.pieces);
            if agrees && !own_in_set {
                w.agreed.push(if state.is_empty() {
                    name.clone()
                } else {
                    format!("{name}.{state}")
                });
                continue;
            }
            w.clips_added += before.len() + after.len();
            let own = std::mem::take(&mut sprite.clips);
            sprite.clips = before.into_iter().chain(own).chain(after).collect();
            let e = model.entities.get_mut(name).expect("checked");
            if state.is_empty() {
                e.core.sprite = Some(sprite);
            } else if let Some(m) = &mut e.core.machine
                && let Some(st) = m.states.get_mut(state)
            {
                // What the entity's own sprite already gives, the state
                // inherits: its `set` merges into it.
                if sprite.blink.is_some()
                    && sprite.blink == e.core.sprite.as_ref().and_then(|s| s.blink.clone())
                {
                    sprite.blink = None;
                }
                let v = toml::Value::try_from(&sprite).expect("a sprite serializes");
                st.set.insert("sprite".into(), v);
                w.states.push(format!("{name}.{state}"));
            }
            touched = true;
        }
        if touched {
            w.entities.push(name.clone());
            let c = &es.coverage;
            let mut paths = vec![format!("entities.{name}.sprite")];
            for st in &w.states {
                if let Some(s) = st.strip_prefix(&format!("{name}.")) {
                    paths.push(format!("entities.{name}.machine.states.{s}.set.sprite"));
                }
            }
            for path in paths {
                model.evidence.insert(
                path,
                Evidence {
                    source: Source::Observed,
                    confidence: 80,
                    refs: refs.to_vec(),
                    note: Some(format!(
                        "gp sprites capture: {} frames from {} observations; states with frames: {}{}",
                        c.frames,
                        c.observations,
                        if c.covered.is_empty() { "none".into() } else { c.covered.join(", ") },
                        if c.unobserved.is_empty() {
                            String::new()
                        } else {
                            format!("; {} table frames never seen", c.unobserved.len())
                        }
                    )),
                },
            );
            }
        }
    }
    prune(model, cap, set_slot, captured_total);
    if w.entities.is_empty() {
        // Nothing uses the slots: the model stays as it was.
        for n in w.slots_added.drain(..) {
            model.assets.remove(&n);
        }
        return w;
    }
    for n in &w.slots_added {
        model.evidence.insert(
            format!("assets.{n}"),
            Evidence {
                source: Source::Observed,
                confidence: 90,
                refs: refs.to_vec(),
                note: Some(
                    "gp sprites capture: a slot its frames use, filled from the asset of its name"
                        .into(),
                ),
            },
        );
    }
    if crate::version_below(&model.model.schema, "0.4.2") {
        model.model.schema = "0.4.2".into();
    }
    w
}

/// Every sprite of the model that draws from the set, its own and its
/// states'.
fn set_sprites(model: &mut Model, set_slot: &str, each: &mut dyn FnMut(&mut Sprite)) {
    for e in model.entities.values_mut() {
        if let Some(s) = &mut e.core.sprite
            && s.frame.as_ref().is_some_and(|f| f.slot == set_slot)
        {
            each(s);
        }
        if let Some(m) = &mut e.core.machine {
            for st in m.states.values_mut() {
                let Some(v) = st.set.get("sprite") else {
                    continue;
                };
                let Ok(mut s) = v.clone().try_into::<Sprite>() else {
                    continue;
                };
                if s.frame.as_ref().is_some_and(|f| f.slot == set_slot) {
                    each(&mut s);
                    st.set.insert(
                        "sprite".into(),
                        toml::Value::try_from(&s).expect("a sprite serializes"),
                    );
                }
            }
        }
    }
}

/// Drops frames added for the model's own sprites that nothing names, and
/// numbers the rest after capture's.
fn prune(model: &mut Model, cap: &mut Capture, set_slot: &str, captured: usize) {
    let mut used = std::collections::BTreeSet::new();
    set_sprites(model, set_slot, &mut |s| {
        used.extend(s.frame.as_ref().map(|f| f.row));
        for c in &s.clips {
            used.extend(c.frames.iter().filter_map(|f| f.row));
        }
    });
    let mut map = BTreeMap::new();
    let mut kept = Vec::new();
    for (i, f) in cap.set.frames.iter().enumerate() {
        if i < captured || used.contains(&(i as u32)) {
            map.insert(i as u32, kept.len() as u32);
            kept.push(f.clone());
        }
    }
    if kept.len() == cap.set.frames.len() {
        return;
    }
    cap.set.frames = kept;
    set_sprites(model, set_slot, &mut |s| {
        if let Some(f) = &mut s.frame {
            f.row = map.get(&f.row).copied().unwrap_or(f.row);
        }
        for c in &mut s.clips {
            for f in &mut c.frames {
                if let Some(r) = &mut f.row {
                    *r = map.get(r).copied().unwrap_or(*r);
                }
            }
        }
    });
}

/// Observations a captured clip needs before it overrides a clip the model
/// already has.
pub const OVERRIDE_SEEN: usize = 30;

/// Whether clip `o`'s condition holds wherever `c`'s does: every condition
/// `o` gives, `c` gives the same.
fn covers(o: &gp_model::model::When, c: &gp_model::model::When) -> bool {
    let b = |x: Option<bool>, y: Option<bool>| x.is_none_or(|v| y == Some(v));
    b(o.ground, c.ground)
        && b(o.moving, c.moving)
        && b(o.skidding, c.skidding)
        && b(o.rising, c.rising)
        && b(o.jumped, c.jumped)
        && b(o.invulnerable, c.invulnerable)
        && b(o.climbing, c.climbing)
        && o.speed_at_least.is_none()
}

/// Captured clips merged with a sprite's own: those to put before its
/// clips and those to put after. With no clips of its own, all of them
/// go first in the order capture found them. Otherwise a captured clip
/// that no clip of the sprite covers goes after them (it shows only where
/// none of them holds); one that the first covering clip already shows is
/// left out; and one that shows other frames where a clip of the sprite
/// holds, seen often enough, goes before them: the sprite's clip is wrong
/// there (a jump frame where a fall off a ledge keeps the walk).
fn merge_clips(
    own: &[Clip],
    captured: &[Clip],
    seen: &[usize],
    had: bool,
    set: &SpriteSet,
) -> (Vec<Clip>, Vec<Clip>) {
    if !had || own.is_empty() {
        return (captured.to_vec(), Vec::new());
    }
    // Frames compare by their tiles and places: a state's own colors make
    // another frame of the same picture.
    let pieces = |row: Option<u32>| -> Option<Vec<Piece>> {
        Some(set.frames.get(row? as usize)?.pieces.clone())
    };
    let ids = |c: &Clip| -> std::collections::BTreeSet<Option<Vec<Piece>>> {
        c.frames.iter().map(|f| pieces(f.row)).collect()
    };
    let (mut before, mut after) = (Vec::new(), Vec::new());
    for (k, c) in captured.iter().enumerate() {
        match own.iter().find(|o| covers(&o.when, &c.when)) {
            None => {
                // Somewhere no clip of the sprite holds.
                if !own.iter().any(|o| o.when == c.when) {
                    after.push(c.clone());
                }
            }
            Some(o) => {
                // The frame the captured clip shows most (its first) is
                // one the sprite's clip shows: they agree.
                let more_specific = when_count(&c.when) > when_count(&o.when);
                let main = pieces(c.frames.first().and_then(|f| f.row));
                if ids(o).contains(&main) || !more_specific {
                    continue;
                }
                if seen.get(k).copied().unwrap_or(0) >= OVERRIDE_SEEN {
                    before.push(c.clone());
                }
            }
        }
    }
    (before, after)
}

fn when_count(w: &gp_model::model::When) -> usize {
    [
        w.ground.is_some(),
        w.moving.is_some(),
        w.skidding.is_some(),
        w.rising.is_some(),
        w.speed_at_least.is_some(),
        w.jumped.is_some(),
        w.invulnerable.is_some(),
        w.climbing.is_some(),
    ]
    .iter()
    .filter(|b| **b)
    .count()
}

fn default_slot() -> AssetSlot {
    let text = "kind = \"tiles\"\nrole = \"graphics\"\n";
    let mut s: AssetSlot = toml::from_str(text).expect("a minimal slot parses");
    s.description = None;
    s
}

/// Rows of a lifted `rows` slot, for [`write`].
pub fn rows_reader<'a>(
    model: &'a Model,
    dir: &'a Path,
) -> impl Fn(&str, u32) -> Option<Vec<u32>> + 'a {
    let cache: std::cell::RefCell<BTreeMap<String, Vec<Vec<i64>>>> = Default::default();
    move |slot: &str, row: u32| {
        let asset = model.assets.get(slot)?.from.as_ref()?.asset.clone();
        let mut c = cache.borrow_mut();
        if !c.contains_key(&asset) {
            c.insert(asset.clone(), crate::tables::byte_rows(dir, &asset).ok()?);
        }
        c.get(&asset)?
            .get(row as usize)
            .map(|r| r.iter().map(|&v| v as u32).collect())
    }
}
