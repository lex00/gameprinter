//! Sprites from recordings: on each frame, the state mapping places every
//! entity the game has on screen, and each sprite of the sprite table
//! (OAM) is given to the entity whose hitbox it's on (or that it touches
//! through sprites already given). An entity's sprites, made relative to
//! its position and to the way it faces, are the frame it shows, recorded
//! with its model state, the conditions clips choose by, its level and
//! its palettes' colors.

use std::collections::{BTreeMap, HashMap};

use gp_assets::sprite_set::Piece;
use gp_interp::mapping::{Field, Mapping};
use gp_model::Model;
use gp_runtime::Value;
use gp_runtime::select::Condition;

use crate::chr::{Banks, TileBank};
use crate::config::Config;

/// An instance's last positions, by frame.
type Trail = std::collections::VecDeque<(u64, [i64; 2])>;

/// Frames over which moving and rising are measured.
const TRAIL: usize = 5;

/// The clip conditions capture measures, in the order a key holds them.
pub const CONDITIONS: [&str; 7] = [
    "ground",
    "moving",
    "rising",
    "jumped",
    "skidding",
    "climbing",
    "invulnerable",
];

/// A sprite on screen: x, y, its lifted tile, palette and flips.
type Sprite = (i32, i32, Option<(usize, u32)>, u8, bool, bool);

/// One frame of a recording, as the console drew it.
pub struct Shot<'a> {
    pub frame: u64,
    /// Internal RAM, 2048 bytes, of the frame the sprites were made from.
    pub ram: &'a [u8],
    /// The sprite table drawn, 256 bytes.
    pub oam: &'a [u8],
    /// Palette RAM, 32 bytes.
    pub palette: &'a [u8],
    /// PPUCTRL: bit 3 is the 8x8 sprites' pattern table, bit 5 8x16.
    pub ctrl: u8,
    pub banks: Banks<'a>,
}

/// A frame as an entity showed it: its sprites from its position, sorted.
pub type Layout = Vec<Piece>;

/// Values of the conditions, by [`CONDITIONS`]; None where not known.
pub type Cond = [Option<bool>; 7];

/// One entity on one frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Obs {
    pub frame: u64,
    /// Which instance: the mapping's index for a fixed one, 1000 plus the
    /// slot for one in a slot.
    pub instance: u32,
    pub entity: String,
    pub state: String,
    pub cond: Cond,
    /// An index into [`Observer::layouts`].
    pub layout: u32,
    /// Where the layout's top left corner was, from the position.
    pub anchor: [i32; 2],
    pub facing_left: bool,
    /// Its sprite palettes' colors, by palette in the model's numbering.
    pub colors: Vec<(u8, [u8; 4])>,
    pub level: String,
    /// The game's frame counter, when the mapping has one.
    pub clock: Option<u8>,
}

struct Fixed {
    entity: String,
    index: u32,
    x: Field,
    /// Without one, the y of the entity's spawn in the start level.
    y: Option<Field>,
    spawn_y: i64,
    facing: Option<Field>,
    ground: Option<Field>,
    alive: Option<Field>,
    vx: Option<Field>,
}

#[derive(Clone)]
struct Inst {
    instance: u32,
    entity: String,
    slot: Option<u16>,
    /// The screen position of the entity's position, and its hitbox on
    /// screen (x0, y0, x1, y1).
    at: [i32; 2],
    /// Its position in the level.
    world: [i64; 2],
    bx: [i32; 4],
    mirror: i32,
    facing: Option<bool>,
    ground: Option<bool>,
    /// Its sideways speed, raw fixed point, when the mapping has it.
    vx: Option<i64>,
    /// A slot whose state has a bit the mapping says ends a life (a
    /// defeated enemy falling): its sprites are its, but not its frames.
    ended: bool,
}

/// What capture saw, frame by frame.
pub struct Observer<'a> {
    model: &'a Model,
    mapping: Option<&'a Mapping>,
    config: &'a Config,
    bank: &'a TileBank,
    fixed: Vec<Fixed>,
    cam: [Option<Field>; 2],
    rules: BTreeMap<String, Vec<(String, Condition)>>,
    whens: BTreeMap<String, Vec<(usize, Condition)>>,
    levels: Vec<(String, Condition)>,
    /// The game machines' state fields, and the states that freeze the
    /// world, when a sprite can't be chosen by them.
    frozen: Vec<(Field, Vec<String>)>,
    skips: BTreeMap<String, Condition>,
    last: HashMap<(u32, String), ([i64; 2], bool)>,
    facings: BTreeMap<String, Condition>,
    trail: HashMap<(u32, String), Trail>,
    slot_facing: Option<Condition>,
    skip_all: Option<Condition>,
    interned: HashMap<Layout, u32>,
    pub layouts: Vec<Layout>,
    pub obs: Vec<Obs>,
    /// Sprite palettes (16 colors) seen on each frame, by level, with
    /// their counts.
    pub level_colors: BTreeMap<String, BTreeMap<Vec<u8>, u32>>,
    /// Sprites whose tile isn't in any lifted tiles asset.
    pub unresolved: u64,
    /// Entity frames left out: while a game state froze the world, or
    /// while the entity's `skip` held.
    pub skipped: u64,
    /// Frames in no level of the model, by the mapping's levels.
    pub outside: u64,
    /// Each 8x8 sprite tile number, as the sprite table gives it, and the
    /// lifted tiles it showed (slot or asset, tile), counted: how a table
    /// that names pattern table numbers finds its tiles.
    pub pattern: HashMap<u32, BTreeMap<(String, u32), u32>>,
    pub frames: u64,
}

fn fixed_raw(v: &Value) -> Option<i64> {
    match v {
        Value::Fixed(r) => Some(r >> 16),
        Value::Int(n) => Some(*n),
        Value::Flag(b) => Some(i64::from(*b)),
        _ => None,
    }
}

fn read(f: &Field, ram: &[u8], slot: u16) -> Value {
    let bytes: Vec<u8> = f
        .ram
        .iter()
        .chain(&f.frac)
        .map(|a| ram.get(usize::from(a + slot)).copied().unwrap_or(0))
        .collect();
    f.value(&bytes)
}

fn eval(c: &Condition, ram: &[u8], slot: u16) -> bool {
    c.eval(&mut |s| {
        let name = s.segments.first().map(|g| g.name.as_str()).unwrap_or("");
        let idx = match s.segments.first().and_then(|g| g.index.as_ref()) {
            Some(gp_runtime::select::Index::At(n)) => *n,
            _ => return Err(format!("`{}` needs one index", s.name())),
        };
        let at = match name {
            "ram" => idx,
            "slot" => idx + i64::from(slot),
            _ => return Err(format!("`{name}` isn't ram or slot")),
        };
        Ok(i64::from(ram.get(at as usize).copied().unwrap_or(0)))
    })
    .unwrap_or(false)
}

impl<'a> Observer<'a> {
    pub fn new(
        model: &'a Model,
        mapping: Option<&'a Mapping>,
        config: &'a Config,
        bank: &'a TileBank,
    ) -> Result<Self, String> {
        let mut fixed: BTreeMap<(String, u32), [Option<Field>; 6]> = BTreeMap::new();
        let mut cam = [None, None];
        let mut frozen_fields = Vec::new();
        if let Some(m) = mapping {
            for f in &m.fields {
                let p = &f.path;
                if let Some(rest) = p.strip_prefix("rules.")
                    && let Some(machine) = rest.strip_suffix(".state")
                    && let Some(m) = model.rules.get(machine)
                {
                    let frozen: Vec<String> = m
                        .states
                        .iter()
                        .filter(|(_, st)| st.freeze)
                        .map(|(n, _)| n.clone())
                        .collect();
                    if !frozen.is_empty() {
                        frozen_fields.push((f.clone(), frozen));
                    }
                    continue;
                }
                if let Some(rest) = p.strip_prefix("cameras.") {
                    if rest.ends_with(".x") && cam[0].is_none() && rest.matches('.').count() == 1 {
                        cam[0] = Some(f.clone());
                    } else if rest.ends_with(".y")
                        && cam[1].is_none()
                        && rest.matches('.').count() == 1
                    {
                        cam[1] = Some(f.clone());
                    }
                    continue;
                }
                let Some(rest) = p.strip_prefix("entities.") else {
                    continue;
                };
                let Some((head, field)) = rest.split_once("].") else {
                    continue;
                };
                let Some((name, ix)) = head.split_once('[') else {
                    continue;
                };
                let Ok(ix) = ix.parse::<u32>() else { continue };
                let slot = match field {
                    "x" => 0,
                    "y" => 1,
                    "facing" => 2,
                    "on_ground" => 3,
                    "alive" => 4,
                    "vx" => 5,
                    _ => continue,
                };
                fixed.entry((name.to_owned(), ix)).or_default()[slot] = Some(f.clone());
            }
        }
        // The spawns of the start level, by entity, for an entity whose y
        // the mapping doesn't give (it never changes).
        let spawns: Vec<(String, i64)> = model
            .model
            .start_level
            .as_ref()
            .and_then(|l| model.levels.get(l))
            .or_else(|| model.levels.values().next())
            .map(|l| {
                l.core
                    .spawn
                    .iter()
                    .filter_map(|s| Some((s.entity.clone(), s.y?.raw() >> 16)))
                    .collect()
            })
            .unwrap_or_default();
        let fixed = fixed
            .into_iter()
            .filter_map(|((entity, index), [x, y, facing, ground, alive, vx])| {
                let spawn_y = spawns.iter().find(|(e, _)| *e == entity).map_or(0, |s| s.1);
                Some(Fixed {
                    entity,
                    index,
                    x: x?,
                    y,
                    spawn_y,
                    facing,
                    ground,
                    alive,
                    vx,
                })
            })
            .collect();
        let mut rules = BTreeMap::new();
        let mut whens = BTreeMap::new();
        let mut skips = BTreeMap::new();
        let mut facings = BTreeMap::new();
        for (name, e) in &config.entities {
            if let Some(w) = &e.skip {
                skips.insert(name.clone(), Condition::parse(w)?);
            }
            if let Some(w) = &e.facing_left {
                facings.insert(name.clone(), Condition::parse(w)?);
            }
            let mut rs = Vec::new();
            for r in &e.states {
                rs.push((r.name.clone(), Condition::parse(&r.when)?));
            }
            rules.insert(name.clone(), rs);
            let mut ws = Vec::new();
            for (k, w) in &e.when {
                let i = CONDITIONS.iter().position(|c| c == k).expect("checked");
                ws.push((i, Condition::parse(w)?));
            }
            whens.insert(name.clone(), ws);
        }
        let mut levels = Vec::new();
        if let Some(s) = mapping.and_then(|m| m.slots.as_ref()) {
            for l in &s.levels {
                levels.push((l.level.clone(), Condition::parse(&l.when)?));
            }
        }
        Ok(Observer {
            model,
            mapping,
            config,
            bank,
            fixed,
            cam,
            rules,
            whens,
            levels,
            frozen: frozen_fields,
            skips,
            facings,
            trail: HashMap::new(),
            skip_all: config
                .skip
                .as_ref()
                .map(|c| Condition::parse(c))
                .transpose()?,
            slot_facing: config
                .facing_left
                .as_ref()
                .map(|c| Condition::parse(c))
                .transpose()?,
            last: HashMap::new(),
            interned: HashMap::new(),
            layouts: Vec::new(),
            obs: Vec::new(),
            level_colors: BTreeMap::new(),
            unresolved: 0,
            skipped: 0,
            outside: 0,
            pattern: HashMap::new(),
            frames: 0,
        })
    }

    fn intern(&mut self, l: Layout) -> u32 {
        if let Some(&i) = self.interned.get(&l) {
            return i;
        }
        let i = self.layouts.len() as u32;
        self.layouts.push(l.clone());
        self.interned.insert(l, i);
        i
    }

    fn place(&self, entity: &str, x: i64, y: i64, cam: [i64; 2], wrap: bool) -> Option<Inst> {
        let e = self.model.entities.get(entity)?;
        let hb = e.core.hitbox;
        let px = |f: gp_model::Fixed| (f.raw() >> 16) as i32;
        let mut sx = (x - cam[0]) as i32;
        if wrap {
            sx = sx.rem_euclid(256);
        }
        let sy = (y - cam[1]) as i32;
        let bx = [
            sx + px(hb.x),
            sy + px(hb.y),
            sx + px(hb.x) + px(hb.w),
            sy + px(hb.y) + px(hb.h),
        ];
        Some(Inst {
            instance: 0,
            entity: entity.to_owned(),
            slot: None,
            at: [sx, sy],
            world: [x, y],
            bx,
            mirror: ((2 * hb.x.raw() + hb.w.raw()) >> 16) as i32,
            facing: None,
            ground: None,
            vx: None,
            ended: false,
        })
    }

    /// Takes one frame.
    pub fn shot(&mut self, s: &Shot<'_>) {
        self.frames += 1;
        let ram = s.ram;
        let cam = [
            self.cam[0]
                .as_ref()
                .and_then(|f| fixed_raw(&read(f, ram, 0)))
                .unwrap_or(0),
            self.cam[1]
                .as_ref()
                .and_then(|f| fixed_raw(&read(f, ram, 0)))
                .unwrap_or(0),
        ];
        if self.skip_all.as_ref().is_some_and(|c| eval(c, ram, 0)) {
            self.outside += 1;
            return;
        }
        let found = self.levels.iter().find(|(_, c)| eval(c, ram, 0));
        // A mapping that names its levels and none of them holds: the game
        // is somewhere the model doesn't have (another stage, a menu).
        if found.is_none() && !self.levels.is_empty() {
            self.outside += 1;
            return;
        }
        let level = found.map(|(l, _)| l.clone()).unwrap_or_else(|| {
            self.model
                .model
                .start_level
                .clone()
                .or_else(|| self.model.levels.keys().next().cloned())
                .unwrap_or_default()
        });
        if s.palette.len() >= 32 {
            *self
                .level_colors
                .entry(level.clone())
                .or_default()
                .entry(s.palette[16..32].to_vec())
                .or_default() += 1;
        }

        // Where each entity is.
        let mut insts: Vec<Inst> = Vec::new();
        for f in &self.fixed {
            if let Some(a) = &f.alive
                && read(a, ram, 0) == Value::Flag(false)
            {
                continue;
            }
            let y = match &f.y {
                Some(y) => fixed_raw(&read(y, ram, 0)),
                None => Some(f.spawn_y),
            };
            let (Some(x), Some(y)) = (fixed_raw(&read(&f.x, ram, 0)), y) else {
                continue;
            };
            let Some(mut i) = self.place(&f.entity, x, y, cam, f.x.ram.len() == 1) else {
                continue;
            };
            i.instance = f.index;
            i.facing = f.facing.as_ref().map(|g| match read(g, ram, 0) {
                Value::Text(t) => t == "left",
                v => fixed_raw(&v).is_some_and(|n| n != 0),
            });
            i.ground = f.ground.as_ref().and_then(|g| match read(g, ram, 0) {
                Value::Flag(b) => Some(b),
                _ => None,
            });
            i.vx = f.vx.as_ref().and_then(|g| match read(g, ram, 0) {
                Value::Fixed(r) => Some(r),
                _ => None,
            });
            insts.push(i);
        }
        if let Some(m) = self.mapping
            && let Some(sl) = &m.slots
        {
            for k in 0..sl.count {
                let flag = sl
                    .flag_of(k, |a| Ok(ram.get(usize::from(a)).copied().unwrap_or(0)))
                    .unwrap_or(0);
                if flag == 0 {
                    continue;
                }
                let id = sl
                    .id_of(k, |a| Ok(ram.get(usize::from(a)).copied().unwrap_or(0)))
                    .unwrap_or(0);
                // The mapping's kind for the ID, else an entity the
                // config gives the ID, placed by the slots' own fields.
                let own = sl.kind_of(id);
                let entity = match own {
                    Some(kind) => {
                        if !sl.flag_fits(kind, flag) {
                            continue;
                        }
                        kind.entity.clone()
                    }
                    None if sl.is_part(flag) => continue,
                    None => match self
                        .config
                        .entities
                        .iter()
                        .find(|(_, e)| e.ids.contains(&id))
                    {
                        Some((name, _)) => name.clone(),
                        None => continue,
                    },
                };
                let fields: Vec<&Field> = match own {
                    Some(kind) => sl.fields_of(kind),
                    None => sl.fields.iter().collect(),
                };
                let get = |p: &str| fields.iter().find(|f| f.path == p).copied();
                let (Some(fx), Some(fy)) = (get("x"), get("y")) else {
                    continue;
                };
                let (Some(x), Some(y)) =
                    (fixed_raw(&read(fx, ram, k)), fixed_raw(&read(fy, ram, k)))
                else {
                    continue;
                };
                let Some(mut i) = self.place(&entity, x, y, cam, fx.ram.len() == 1) else {
                    continue;
                };
                i.instance = 1000 + u32::from(k);
                i.slot = Some(k);
                i.vx = get("vx").and_then(|g| match read(g, ram, k) {
                    Value::Fixed(r) => Some(r),
                    _ => None,
                });
                if let (Some(e), Some(at)) = (own.and_then(|k| k.ends).or(sl.ends), sl.state) {
                    i.ended = ram.get(usize::from(at + k)).is_some_and(|v| v & e != 0);
                }
                insts.push(i);
            }
        }
        insts.retain(|i| i.bx[2] > 0 && i.bx[0] < 256 && i.bx[3] > 0 && i.bx[1] < 240);
        if insts.is_empty() {
            return;
        }
        // A state that freezes the world (a death) shows sprites the model
        // can't choose by an entity's state yet: they're counted, not kept.
        let frozen = self.frozen.iter().any(|(f, states)| match read(f, ram, 0) {
            Value::Text(t) => states.contains(&t),
            _ => false,
        });

        // The sprites, 8x16 ones as two tiles.
        let tall = s.ctrl & 0x20 != 0;
        let base = if s.ctrl & 0x08 != 0 { 0x1000u16 } else { 0 };
        let mut sprites: Vec<Sprite> = Vec::new();
        for o in s.oam.as_chunks::<4>().0.iter().take(64) {
            let (y, tile, attr, x) = (o[0], o[1], o[2], o[3]);
            if y >= 0xEF {
                continue;
            }
            let (fx, fy) = (attr & 0x40 != 0, attr & 0x80 != 0);
            let (sx, sy) = (i32::from(x), i32::from(y) + 1);
            let parts: Vec<(u16, i32)> = if tall {
                let t = (u16::from(tile & 1) << 12) + u16::from(tile & 0xFE) * 16;
                let (top, bottom) = if fy { (t + 16, t) } else { (t, t + 16) };
                vec![(top, 0), (bottom, 8)]
            } else {
                vec![(base + u16::from(tile) * 16, 0)]
            };
            for (addr, dy) in parts {
                let t = s.banks.tile(self.bank, addr);
                if t.is_none() {
                    self.unresolved += 1;
                }
                if let (Some((a, n)), false) = (t, tall) {
                    let asset = &self.bank.assets[a];
                    let name = asset.slot.clone().unwrap_or_else(|| asset.name.clone());
                    *self
                        .pattern
                        .entry(u32::from(tile))
                        .or_default()
                        .entry((name, n))
                        .or_default() += 1;
                }
                if t.is_some_and(|t| self.bank.is_blank(t)) {
                    continue;
                }
                sprites.push((sx, sy + dy, t, 4 + (attr & 3), fx, fy));
            }
        }

        // Each sprite to the entity whose hitbox it's on, then sprites
        // touching those to the same one.
        let m = self.config.margin;
        let mut owner: Vec<Option<usize>> = vec![None; sprites.len()];
        for (k, sp) in sprites.iter().enumerate() {
            let (cx, cy) = (sp.0 + 4, sp.1 + 4);
            let mut best: Option<(i64, usize)> = None;
            for (j, i) in insts.iter().enumerate() {
                let b = i.bx;
                if cx >= b[0] - m && cx < b[2] + m && cy >= b[1] - m && cy < b[3] + m {
                    let (mx, my) = ((b[0] + b[2]) / 2, (b[1] + b[3]) / 2);
                    let d = i64::from(cx - mx).pow(2) + i64::from(cy - my).pow(2);
                    if best.is_none_or(|(bd, _)| d < bd) {
                        best = Some((d, j));
                    }
                }
            }
            owner[k] = best.map(|(_, j)| j);
        }
        for _ in 0..4 {
            let mut changed = false;
            for k in 0..sprites.len() {
                if owner[k].is_some() {
                    continue;
                }
                let a = &sprites[k];
                for q in 0..sprites.len() {
                    let Some(j) = owner[q] else { continue };
                    let b = &sprites[q];
                    let touch = (a.0 - b.0).abs() <= 8 && (a.1 - b.1).abs() <= 8 && a.3 == b.3;
                    let bx = insts[j].bx;
                    let near = a.0 + 8 > bx[0] - 32
                        && a.0 < bx[2] + 32
                        && a.1 + 8 > bx[1] - 32
                        && a.1 < bx[3] + 32;
                    if touch && near {
                        owner[k] = Some(j);
                        changed = true;
                        break;
                    }
                }
            }
            if !changed {
                break;
            }
        }

        let clock = self
            .mapping
            .and_then(|m| m.slots.as_ref())
            .and_then(|sl| sl.clock)
            .and_then(|at| ram.get(usize::from(at)).copied());
        for (j, inst) in insts.iter().enumerate() {
            let mut pieces = Vec::new();
            let mut unresolved = false;
            for (k, sp) in sprites.iter().enumerate() {
                if owner[k] != Some(j) {
                    continue;
                }
                let Some((a, t)) = sp.2 else {
                    unresolved = true;
                    continue;
                };
                let asset = &self.bank.assets[a];
                pieces.push(Piece {
                    tile: t,
                    slot: Some(asset.slot.clone().unwrap_or_else(|| asset.name.clone())),
                    x: sp.0 - inst.at[0],
                    y: sp.1 - inst.at[1],
                    palette: sp.3,
                    flip_x: sp.4,
                    flip_y: sp.5,
                });
            }
            if unresolved && pieces.is_empty() {
                continue;
            }
            // Near the screen's edges a game often draws an entity in part
            // or not at all: those frames aren't its frames. Its sprites
            // still were given to it, so they aren't another's.
            let b = inst.bx;
            if b[0] < 8 || b[2] > 248 || b[1] < 8 || b[3] > 232 {
                continue;
            }
            // Facing: the mapping's, else the way it last moved.
            let key = (inst.instance, inst.entity.clone());
            let key_t = key.clone();
            let prev = self.last.get(&key).copied();
            let moved = prev.map(|(p, _)| [inst.world[0] - p[0], inst.world[1] - p[1]]);
            // A slot object's slot, or for an entity the mapping names by
            // fixed paths (`entities.slug[1]`) its index, so `slot[a]` reads
            // its byte of a one-a-slug array.
            let slot = inst.slot.unwrap_or_else(|| {
                u16::try_from(inst.instance)
                    .ok()
                    .filter(|&i| i < 1000)
                    .unwrap_or(0)
            });
            let rule = self
                .facings
                .get(&inst.entity)
                .or(self.slot_facing.as_ref().filter(|_| inst.slot.is_some()))
                .map(|c| eval(c, ram, slot));
            let facing_left = inst.facing.or(rule).unwrap_or_else(|| match moved {
                Some([dx, _]) if dx < 0 => true,
                Some([dx, _]) if dx > 0 => false,
                _ => prev.is_some_and(|(_, f)| f),
            });
            self.last.insert(key, (inst.world, facing_left));
            if facing_left {
                for p in &mut pieces {
                    p.x = inst.mirror - p.x - 8;
                    p.flip_x = !p.flip_x;
                }
            }
            pieces.sort();
            let mut colors: Vec<(u8, [u8; 4])> = Vec::new();
            for p in &pieces {
                if !colors.iter().any(|(q, _)| *q == p.palette) && s.palette.len() >= 32 {
                    let at = 16 + usize::from(p.palette - 4) * 4;
                    colors.push((
                        p.palette,
                        [
                            s.palette[at],
                            s.palette[at + 1],
                            s.palette[at + 2],
                            s.palette[at + 3],
                        ],
                    ));
                }
            }
            colors.sort();
            if frozen
                || inst.ended
                || self
                    .skips
                    .get(&inst.entity)
                    .is_some_and(|c| eval(c, ram, slot))
            {
                self.skipped += 1;
                continue;
            }
            let state = self
                .rules
                .get(&inst.entity)
                .and_then(|rs| rs.iter().find(|(_, c)| eval(c, ram, slot)))
                .map(|(n, _)| n.clone())
                .or_else(|| {
                    // No rules: a slot's raw state byte, for the report.
                    let rules = self.rules.get(&inst.entity).is_some_and(|r| !r.is_empty());
                    let at = self.mapping?.slots.as_ref()?.state?;
                    (!rules && inst.slot.is_some()).then(|| {
                        let v = ram.get(usize::from(at + slot)).copied().unwrap_or(0);
                        format!("raw {v:#04x}")
                    })
                })
                .unwrap_or_default();
            let mut cond: Cond = [None; CONDITIONS.len()];
            cond[0] = inst.ground;
            // Moving and rising over the last few frames, so a speed of a
            // fraction of a pixel a frame still counts as moving.
            let trail = self.trail.entry(key_t.clone()).or_default();
            if trail.back().is_some_and(|(f, _)| s.frame != f + 1) {
                trail.clear();
            }
            trail.push_back((s.frame, inst.world));
            while trail.len() > TRAIL {
                trail.pop_front();
            }
            if trail.len() == TRAIL {
                let (_, old) = trail[0];
                cond[1] = Some(trail.iter().any(|(_, p)| p[0] != old[0]));
                if cond[0].is_none() {
                    cond[2] = Some(inst.world[1] < old[1]);
                }
            }
            let _ = moved;
            // The speed, when the mapping has it, is what a model's clip
            // reads: moving and skidding by it.
            if let Some(vx) = inst.vx {
                cond[1] = Some(vx != 0);
                cond[4] = Some(vx != 0 && (vx < 0) != facing_left);
            }
            if let Some(ws) = self.whens.get(&inst.entity) {
                for (i, c) in ws {
                    cond[*i] = Some(eval(c, ram, slot));
                }
            }
            // Frames are kept by their shape, from their top left corner;
            // where that corner is from the position is kept per frame, so
            // the pixel or two a game's drawing lags its positions by
            // doesn't make new frames.
            let x0 = pieces.iter().map(|p| p.x).min().unwrap_or(0);
            let y0 = pieces.iter().map(|p| p.y).min().unwrap_or(0);
            for p in &mut pieces {
                p.x -= x0;
                p.y -= y0;
            }
            let layout = self.intern(pieces);
            self.obs.push(Obs {
                frame: s.frame,
                instance: inst.instance,
                entity: inst.entity.clone(),
                state,
                cond,
                layout,
                anchor: [x0, y0],
                facing_left,
                colors,
                level: level.clone(),
                clock,
            });
        }
    }
}
