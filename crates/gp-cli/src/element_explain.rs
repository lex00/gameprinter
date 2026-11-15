//! `gp element explain`: what is at a spot of the screen on one frame of a
//! clip, from evidence (issue #10, docs/element.md, "What is at a spot").
//!
//! A person points at the game on the page and asks "what is this?"; the
//! agent answers by running this. Given a game, a corpus clip, a frame and
//! a point or a box in screen pixels, it says:
//!
//! - what the original draws there on that frame: the sprites under the
//!   box (their slot in the sprite table and picture number) and the
//!   background tile at its middle (its cell and picture number, and the
//!   asset map's name for its metatile when the model has lifted one);
//! - which game object that is, when the state mapping says: the sprite
//!   table's bytes, or the bytes the code that wrote them read (followed
//!   up to [`HOPS`] writes back), set against the mapping's fields and
//!   slots, with the field values on the frame;
//! - what code drew it (the routine that wrote the sprite's x byte) and
//!   what moves it (the routine that wrote the object's x byte, with every
//!   input that code read), both from `gp element inputs`' slice;
//! - what the model has there: the things its picture draws in the box,
//!   how many pixels of the two pictures differ in it, and, for the object,
//!   the lockstep's measures on the frame in both games, each within its
//!   tolerance or not;
//! - what the tools could not determine, said plainly.
//!
//! Every claim names its source (`from`), so an agent can run that tool
//! itself. Nothing is guessed: a part no tool answered is in `unknown`.
//!
//! It reads the original twice at most per platform's run of the clip (the
//! start cache keeps the NES's start; the arcade board runs from
//! power-on, as `gp element inputs` runs it) and the model once, straight
//! through as `gp clip score` runs it, from the clip's score tape when
//! there is one. `--no-model` leaves the model out. The output says how
//! long each part took.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use anyhow::{Context as _, Result, bail};
use clap::{Args, ValueEnum};
use gp_evidence::Paths;
use gp_interp::mapping::{Field, Mapping};
use gp_present::raster::Frame;
use serde_json::{Value, json};

use crate::element::InputsArgs;
use crate::original::Platform;

/// The JSON shape of `gp element explain`. A change to it bumps this.
pub const SCHEMA: &str = "gp element explain 1";

/// The note on an owner whose slot's flag reads empty.
const EMPTY: &str = "its slot reads empty on this frame";

/// How many writes back from the sprite table the code is followed to
/// find bytes the mapping names.
pub const HOPS: usize = 3;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum Side {
    /// The original's picture: what the ROM draws there.
    #[default]
    Original,
    /// The model's picture: what the rebuilt game draws there.
    Model,
}

impl Side {
    fn name(self) -> &'static str {
        match self {
            Side::Original => "original",
            Side::Model => "model",
        }
    }
}

#[derive(Args, Clone, Default)]
pub struct ExplainArgs {
    /// The game's package folder (games/donkey-kong).
    #[arg(long)]
    pub run: PathBuf,
    /// A clip of the game's corpus by its ID (dk.walk.001): the recordings
    /// the page's game view plays from the game's tests.
    #[arg(long)]
    pub clip: String,
    /// The clip's frame, counted from its start as `gp clip score` and
    /// `gp element inputs` count it (1 is the clip's first frame).
    #[arg(long, conflicts_with = "log_frame")]
    pub frame: Option<u64>,
    /// The frame counted from power-on, as the page's game view and a
    /// point-and-tell turn give it; the clip's frame is this less its start.
    #[arg(long)]
    pub log_frame: Option<u64>,
    /// A point on the picture in screen pixels, `X,Y` (the NES's picture is
    /// 256x240, an arcade board's as drawn, Tapper's 512x480). A point is
    /// a box of one pixel.
    #[arg(long, conflicts_with = "spot_box")]
    pub at: Option<String>,
    /// A box on the picture in screen pixels, `X,Y,W,H`, as point-and-tell
    /// sends it.
    #[arg(long = "box")]
    pub spot_box: Option<String>,
    /// Which picture the person pointed at. Both games are read either
    /// way; this says whose things lead the answer.
    #[arg(long, value_enum, default_value_t = Side::Original)]
    pub side: Side,
    /// Don't run the model: the original's side alone, faster.
    #[arg(long)]
    pub no_model: bool,
    /// The labels file, instead of the game's usual one.
    #[arg(long)]
    pub labels: Option<PathBuf>,
    /// The game's built module, for a model with hooks.
    #[arg(long)]
    pub module: Option<PathBuf>,
    /// The model folder (model.toml and mapping.toml); by default the
    /// game's own.
    #[arg(long)]
    pub model: Option<PathBuf>,
    /// The model's lifted assets (default: $GP_HOME/work/assets/SOURCE).
    #[arg(long)]
    pub assets: Option<PathBuf>,
    /// The ROM; by default the one in $GP_ROM_DIR the package names.
    #[arg(long)]
    pub rom: Option<PathBuf>,
}

/// A box on the picture, in screen pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spot {
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
}

impl Spot {
    fn mid(&self) -> (i64, i64) {
        (self.x + self.w / 2, self.y + self.h / 2)
    }

    fn touches(&self, r: [i64; 4]) -> bool {
        r[0] < self.x + self.w
            && self.x < r[0] + r[2]
            && r[1] < self.y + self.h
            && self.y < r[1] + r[3]
    }

    fn json(&self) -> Value {
        json!({"x": self.x, "y": self.y, "w": self.w, "h": self.h})
    }
}

/// The spot from `--at X,Y` or `--box X,Y,W,H`, checked against the
/// picture's size and clipped to it.
pub fn parse_spot(at: Option<&str>, b: Option<&str>, screen: (i64, i64)) -> Result<Spot, String> {
    let nums = |t: &str| -> Result<Vec<i64>, String> {
        t.split(',')
            .map(|p| p.trim().parse::<i64>())
            .collect::<Result<_, _>>()
            .map_err(|_| format!("{t:?} isn't whole pixels separated by commas"))
    };
    let (x, y, w, h) = match (at, b) {
        (Some(t), None) => match nums(t)?[..] {
            [x, y] => (x, y, 1, 1),
            _ => return Err(format!("--at is X,Y, two numbers, not {t:?}")),
        },
        (None, Some(t)) => match nums(t)?[..] {
            [x, y, w, h] => (x, y, w, h),
            _ => return Err(format!("--box is X,Y,W,H, four numbers, not {t:?}")),
        },
        _ => return Err("give the spot: --at X,Y or --box X,Y,W,H in screen pixels".into()),
    };
    if !(0..screen.0).contains(&x) || !(0..screen.1).contains(&y) || w < 1 || h < 1 {
        return Err(format!(
            "the spot {x},{y} ({w}x{h}) must start on the {}x{} picture and be at least 1x1",
            screen.0, screen.1
        ));
    }
    Ok(Spot {
        x,
        y,
        w: w.min(screen.0 - x),
        h: h.min(screen.1 - y),
    })
}

/// One sprite as the original's sprite table holds it, placed in screen
/// pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sprite {
    /// Its slot in the sprite table (OAM's 0 to 63, the board's sprite
    /// RAM's 0 to 127).
    pub index: u16,
    /// Left, top, width, height in screen pixels.
    pub rect: [i64; 4],
    /// The picture number: the tile (NES) or the sprite code (arcade).
    pub picture: u16,
    pub palette: u8,
    pub flip_x: bool,
    pub flip_y: bool,
    /// The bytes in CPU memory the code wrote its x and y into: the
    /// sprite table the NES copies to OAM, or the board's sprite RAM.
    pub x_addr: Option<u16>,
    pub y_addr: Option<u16>,
}

/// The NES's sprites from OAM: those on the screen, 8 wide and 8 or 16
/// tall. `page` is the sprite table's RAM page the code copies to OAM.
pub fn nes_sprites(oam: &[u8], tall: bool, page: Option<u8>) -> Vec<Sprite> {
    let h = if tall { 16 } else { 8 };
    (0..64u16)
        .filter_map(|i| {
            let b = oam.get(4 * usize::from(i)..4 * usize::from(i) + 4)?;
            if b[0] >= 0xEF {
                return None;
            }
            let at = |k: u16| page.map(|p| (u16::from(p) << 8) | (4 * i + k));
            Some(Sprite {
                index: i,
                rect: [i64::from(b[3]), i64::from(b[0]) + 1, 8, h],
                picture: u16::from(b[1]),
                palette: b[2] & 3,
                flip_x: b[2] & 0x40 != 0,
                flip_y: b[2] & 0x80 != 0,
                x_addr: at(3),
                y_addr: at(0),
            })
        })
        .collect()
}

/// The arcade board's sprites from its sprite RAM (at `base` in the main
/// CPU's memory), placed as gp-arcade draws them: each 16x16 of the game's
/// units, 32x32 screen pixels.
pub fn arcade_sprites(ram: &[u8], base: u16) -> Vec<Sprite> {
    let unit = i64::from(gp_arcade::video::UNIT);
    gp_arcade::video::sprite_list(ram)
        .into_iter()
        .map(|s| {
            let at = base + 4 * u16::from(s.slot);
            Sprite {
                index: u16::from(s.slot),
                rect: [
                    i64::from(s.x) * unit,
                    i64::from(s.y) * unit,
                    16 * unit,
                    16 * unit,
                ],
                picture: s.code,
                palette: s.color,
                flip_x: s.flip_x,
                flip_y: s.flip_y,
                x_addr: Some(at + 3),
                y_addr: Some(at),
            }
        })
        .collect()
}

/// The sprites the spot touches, the one nearest its middle first.
pub fn sprites_under(sprites: &[Sprite], spot: Spot) -> Vec<Sprite> {
    let (mx, my) = spot.mid();
    let mut v: Vec<Sprite> = sprites
        .iter()
        .filter(|s| spot.touches(s.rect))
        .cloned()
        .collect();
    let d = |s: &Sprite| {
        let (cx, cy) = (s.rect[0] + s.rect[2] / 2, s.rect[1] + s.rect[3] / 2);
        (cx - mx).abs() + (cy - my).abs()
    };
    v.sort_by_key(|s| (d(s), s.index));
    v
}

/// Who in the state mapping holds a byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owner {
    /// The object: `entities.player[0]`, `bodies.barrel` with a slot, or a
    /// field's whole path for one that is no entity's (`counters.bonus`).
    pub thing: String,
    /// The model entity, when the byte is an entity's.
    pub entity: Option<String>,
    /// The original's slot, for a slot object.
    pub slot: Option<u16>,
    /// The field: `x`, `y`, `ext[1]`.
    pub field: String,
    /// The mapping's path for it, as the lockstep names it.
    pub path: String,
    /// How the slot's kind was told, or why it couldn't be.
    pub note: Option<String>,
}

fn holds(f: &Field, addr: u16, slot: Option<u16>) -> bool {
    if f.machine.is_some() || !f.oam.is_empty() || !f.nametable.is_empty() || f.oam_tile.is_some() {
        return false;
    }
    match slot {
        None => f.ram.contains(&addr) || f.frac == Some(addr),
        Some(s) => f.slot_addresses(s).contains(&addr),
    }
}

/// Whether a condition over the original's RAM holds (the runtime
/// protocol's grammar, `ram[N]` selectors read by `byte`). One that can't
/// be read holds not.
fn holds_when(text: &str, byte: &dyn Fn(u16) -> Option<u8>) -> bool {
    use gp_runtime::select::{Condition, Index};
    let Ok(c) = Condition::parse(text) else {
        return false;
    };
    c.eval(
        &mut |s: &gp_runtime::select::Selector| match s.segments.as_slice() {
            [seg] if seg.name == "ram" => match seg.index {
                Some(Index::At(n)) => u16::try_from(n)
                    .ok()
                    .and_then(byte)
                    .map(i64::from)
                    .ok_or_else(|| "unread".to_owned()),
                _ => Err("not a byte".into()),
            },
            _ => Err("only ram[N]".into()),
        },
    )
    .unwrap_or(false)
}

/// Why slot `s` of `g` reads free on this frame, if it does: the group's
/// `vacant` (or its slots') holds, or its `vacant_from`.
fn vacant(
    base: &gp_interp::mapping::Slots,
    g: &gp_interp::mapping::Slots,
    s: u16,
    byte: &dyn Fn(u16) -> Option<u8>,
) -> Option<String> {
    let when = g.vacant.as_ref().or(base.vacant.as_ref());
    if let Some(w) = when
        && holds_when(w, byte)
    {
        return Some("the mapping's vacant condition holds: these slots hold none of their objects on this frame".into());
    }
    if let Some(v) = &g.vacant_from
        && s >= v.slot
        && holds_when(&v.when, byte)
    {
        return Some(format!(
            "the mapping's vacant_from holds for slots from {}",
            v.slot
        ));
    }
    None
}

/// The mapping's fields and slot fields that read `addr`. A slot whose
/// group holds more than one kind is told by its ID byte (read by `byte`).
/// A slot group the mapping says is vacant on the frame is passed over;
/// an empty slot (its flag 0) answers only when no other does, with a
/// note.
pub fn owners(m: &Mapping, addr: u16, byte: &dyn Fn(u16) -> Option<u8>) -> Vec<Owner> {
    let mut out: Vec<Owner> = Vec::new();
    // A presentation field (one no run compares, as a hook's kept bytes)
    // names nothing a person sees.
    for f in &m.fields {
        if !holds(f, addr, None) || f.presentation {
            continue;
        }
        let (thing, field) = split_path(&f.path);
        out.push(Owner {
            entity: entity_of(&thing),
            thing,
            slot: None,
            field,
            path: f.path.clone(),
            note: None,
        });
    }
    let groups = m.slot_groups();
    let mut empty: Vec<Owner> = Vec::new();
    let mut base = 0u16;
    for g in &groups {
        for s in 0..g.count {
            let mut kinds: Vec<&gp_interp::mapping::SlotKind> = g
                .kinds
                .iter()
                .filter(|k| g.fields_of(k).iter().any(|f| holds(f, addr, Some(s))))
                .collect();
            if kinds.is_empty() || vacant(groups[0], g, s, byte).is_some() {
                continue;
            }
            let mut note = None;
            if kinds.len() > 1 {
                match g.id_of(s, |a| byte(a).ok_or_else(|| "unread".to_owned())) {
                    Ok(id) => {
                        let fit: Vec<_> = kinds
                            .iter()
                            .copied()
                            .filter(|k| k.ids.contains(&id))
                            .collect();
                        if fit.is_empty() {
                            note = Some(format!(
                                "its ID byte reads {id}, which names none of {}",
                                names(&kinds)
                            ));
                        } else {
                            kinds = fit;
                            note = Some(format!("told by its ID byte, {id}"));
                        }
                    }
                    Err(_) => {
                        note = Some(format!(
                            "the slot holds one of {}; its ID byte wasn't read",
                            names(&kinds)
                        ))
                    }
                }
            }
            let free = matches!(
                g.flag_of(s, |a| byte(a).ok_or_else(|| "unread".to_owned())),
                Ok(0)
            );
            if free {
                note = Some(EMPTY.to_owned());
            }
            for k in kinds {
                for f in g.fields_of(k) {
                    if holds(f, addr, Some(s)) {
                        let o = Owner {
                            thing: format!("{}.{}", gp_interp::bodies::PREFIX, k.entity),
                            entity: Some(k.entity.clone()),
                            slot: Some(base + s),
                            field: f.path.clone(),
                            path: format!(
                                "{}.{}[slot {}].{}",
                                gp_interp::bodies::PREFIX,
                                k.entity,
                                base + s,
                                f.path
                            ),
                            note: note.clone(),
                        };
                        if free { empty.push(o) } else { out.push(o) }
                    }
                }
            }
        }
        base += g.count;
    }
    if out.is_empty() {
        out = empty;
    }
    // An entity's place first, then its other fields, then the rest.
    out.sort_by_key(|o| (o.entity.is_none(), !matches!(o.field.as_str(), "x" | "y")));
    out
}

/// When no code read leads to a named byte: the slot whose block of bytes
/// holds `addr`, for a slot group whose fields step by a stride (Donkey
/// Kong's barrels: 16 bytes of the sprite buffer a barrel, the x and y the
/// mapping reads at the block's first sprite). The block runs from the
/// slot's lowest field byte for one stride. Said as found by place, not
/// by the code.
pub fn owners_by_block(m: &Mapping, addr: u16, byte: &dyn Fn(u16) -> Option<u8>) -> Vec<Owner> {
    let mut base = 0u16;
    let groups = m.slot_groups();
    for g in groups.iter().copied() {
        let stride = g
            .fields
            .iter()
            .filter_map(|f| f.stride)
            .max()
            .or(g.active_stride)
            .unwrap_or(1);
        if stride > 1 {
            for s in 0..g.count {
                let low = g
                    .fields
                    .iter()
                    .filter(|f| f.stride == Some(stride) && !f.shared)
                    .flat_map(|f| f.slot_addresses(s))
                    .chain(std::iter::once(g.active_at(s)))
                    .min();
                let Some(low) = low else { continue };
                if !(low..low.saturating_add(stride)).contains(&addr)
                    || vacant(groups[0], g, s, byte).is_some()
                {
                    continue;
                }
                let Some(field) = g
                    .fields
                    .iter()
                    .find(|f| f.path == "x" && f.stride == Some(stride))
                else {
                    continue;
                };
                let x = field.slot_addresses(s).first().copied().unwrap_or(low);
                let mut found: Vec<Owner> = owners(m, x, byte)
                    .into_iter()
                    .filter(|o| o.slot.is_some())
                    .collect();
                for o in &mut found {
                    o.note = Some(format!(
                        "found by place: {} is in slot {}'s block of {stride} bytes ({} to {}), whose x the mapping reads at {}{}",
                        hex(addr),
                        base + s,
                        hex(low),
                        hex(low + stride - 1),
                        hex(x),
                        o.note.as_ref().map_or(String::new(), |n| format!("; {n}"))
                    ));
                }
                if !found.is_empty() {
                    return found;
                }
            }
        }
        base += g.count;
    }
    Vec::new()
}

fn names(kinds: &[&gp_interp::mapping::SlotKind]) -> String {
    kinds
        .iter()
        .map(|k| k.entity.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// `entities.player[0].x` as (`entities.player[0]`, `x`).
fn split_path(p: &str) -> (String, String) {
    if let Some(rest) = p.strip_prefix("entities.")
        && let Some(i) = rest.find("].")
    {
        let cut = "entities.".len() + i + 1;
        return (p[..cut].to_owned(), p[cut + 1..].to_owned());
    }
    (p.to_owned(), String::new())
}

/// The entity of `entities.player[0]`.
fn entity_of(thing: &str) -> Option<String> {
    thing
        .strip_prefix("entities.")
        .and_then(|r| r.split('[').next())
        .map(str::to_owned)
}

/// The bytes the mapping reads for the owner's position field `field`
/// (`x`), in the same object.
fn field_addr(m: &Mapping, o: &Owner, field: &str) -> Option<u16> {
    match o.slot {
        None => {
            let p = format!("{}.{field}", o.thing);
            m.fields
                .iter()
                .find(|f| f.path == p)
                .and_then(|f| f.ram.first().copied())
        }
        Some(slot) => {
            let mut local = slot;
            for g in m.slot_groups() {
                if local >= g.count {
                    local -= g.count;
                    continue;
                }
                let k = g
                    .kinds
                    .iter()
                    .find(|k| Some(&k.entity) == o.entity.as_ref())?;
                return g
                    .fields_of(k)
                    .into_iter()
                    .find(|f| f.path == field)
                    .and_then(|f| f.slot_addresses(local).first().copied());
            }
            None
        }
    }
}

/// The fields of the owner's object with their values from the original's
/// memory on the frame (`byte`), as the mapping reads them.
fn field_values(m: &Mapping, o: &Owner, byte: &dyn Fn(u16) -> Option<u8>) -> Vec<Value> {
    let read = |f: &Field, slot: u16| -> Option<String> {
        let bytes: Option<Vec<u8>> = f.slot_addresses(slot).into_iter().map(byte).collect();
        Some(shown(&f.slot_value(slot, &bytes?)))
    };
    let addrs = |f: &Field, slot: u16| -> Vec<String> {
        f.slot_addresses(slot).iter().map(|a| hex(*a)).collect()
    };
    match o.slot {
        None => m
            .fields
            .iter()
            .filter(|f| f.path == o.thing || f.path.starts_with(&format!("{}.", o.thing)))
            .filter(|f| {
                !f.ram.is_empty()
                    && f.machine.is_none()
                    && f.oam.is_empty()
                    && f.nametable.is_empty()
            })
            .map(|f| json!({"path": f.path, "ram": addrs(f, 0), "original": read(f, 0)}))
            .collect(),
        Some(slot) => {
            let mut local = slot;
            for g in m.slot_groups() {
                if local >= g.count {
                    local -= g.count;
                    continue;
                }
                let Some(k) = g
                    .kinds
                    .iter()
                    .find(|k| Some(&k.entity) == o.entity.as_ref())
                else {
                    return Vec::new();
                };
                return g
                    .fields_of(k)
                    .into_iter()
                    .filter(|f| !f.index)
                    .map(|f| json!({"path": f.path, "ram": addrs(f, local), "original": read(f, local)}))
                    .collect();
            }
            Vec::new()
        }
    }
}

fn shown(v: &gp_runtime::Value) -> String {
    match v {
        gp_runtime::Value::Fixed(r) => {
            let x = *r as f64 / 65536.0;
            let t = format!("{x:.2}");
            t.trim_end_matches('0').trim_end_matches('.').to_owned()
        }
        other => other.to_string(),
    }
}

fn hex(a: u16) -> String {
    format!("${a:04X}")
}

/// A `$0203`, `0x203` or number from the slices' JSON.
fn addr_of(v: &Value) -> Option<u16> {
    if let Some(n) = v.as_u64() {
        return u16::try_from(n).ok();
    }
    let s = v.as_str()?;
    let h = s.strip_prefix('$').or_else(|| s.strip_prefix("0x"))?;
    u16::from_str_radix(h, 16).ok()
}

/// One write followed: a byte's slice from `gp element inputs`, the parts
/// this command reads of it.
#[derive(Debug, Clone)]
struct Hop {
    addr: u16,
    written: bool,
    routine: String,
    pc: String,
    frame_written: Option<u64>,
    sentence: String,
    /// The RAM bytes its value came from, nearest first.
    from: Vec<u16>,
    raw: Value,
}

/// A slice of either platform's `gp element inputs` as a [`Hop`]. `ram`
/// says which addresses are RAM the code keeps state in.
fn hop_of(addr: u16, b: &Value, ram: &dyn Fn(u16) -> bool) -> Hop {
    let written = b["written"] == true;
    let w = &b["writer"];
    let routine = w["routine_name"]
        .as_str()
        .or(w["routine"].as_str())
        .unwrap_or("")
        .to_owned();
    let pc = match &w["pc"] {
        Value::Number(n) => hex(n.as_u64().unwrap_or(0) as u16),
        Value::String(s) => s.clone(),
        _ => String::new(),
    };
    let mut from: Vec<u16> = Vec::new();
    let mut take = |reads: &Value| {
        for r in reads.as_array().into_iter().flatten() {
            if r["how"] != "value" {
                continue;
            }
            if let Some(a) = addr_of(&r["addr"])
                && a != addr
                && ram(a)
                && !from.contains(&a)
            {
                from.push(a);
            }
        }
    };
    take(&b["reads"]);
    for f in b["feeds"].as_array().into_iter().flatten() {
        take(&f["reads"]);
    }
    for e in b["earlier"].as_array().into_iter().flatten() {
        take(&e["reads"]);
    }
    Hop {
        addr,
        written,
        routine,
        pc,
        frame_written: b["frame_written"].as_u64(),
        sentence: b["sentence"].as_str().unwrap_or("").to_owned(),
        from,
        raw: b.clone(),
    }
}

/// Follows the code back from `starts` (a sprite's x and y bytes) through
/// the bytes each write's value came from, at most [`HOPS`] writes deep,
/// until a byte the mapping names: the writes followed (the starts' first,
/// in order), the owners found and the byte they were found at.
fn follow(
    starts: &[u16],
    slice: &mut dyn FnMut(&[u16]) -> Result<Vec<Hop>>,
    owned: &dyn Fn(u16) -> Vec<Owner>,
) -> Result<(Vec<Hop>, Vec<Owner>, Option<u16>)> {
    let mut hops: Vec<Hop> = Vec::new();
    let mut want = starts.to_vec();
    let mut seen: BTreeSet<u16> = starts.iter().copied().collect();
    // A start the mapping names: sliced for who wrote it, and owned at once.
    if let Some(&a) = starts.iter().find(|&&a| !owned(a).is_empty()) {
        return Ok((slice(starts)?, owned(a), Some(a)));
    }
    for _ in 0..HOPS {
        let got = slice(&want)?;
        let mut next: Vec<u16> = Vec::new();
        // An occupied slot's byte before an empty one's: a byte read from
        // an empty slot names nothing on the screen.
        let mut found: Option<(Vec<Owner>, u16)> = None;
        let mut weak: Option<(Vec<Owner>, u16)> = None;
        for h in &got {
            for &a in &h.from {
                let o = owned(a);
                if o.is_empty() {
                    if seen.insert(a) {
                        next.push(a);
                    }
                } else if o.iter().any(|x| x.note.as_deref() == Some(EMPTY)) {
                    weak.get_or_insert((o, a));
                } else {
                    found.get_or_insert((o, a));
                }
            }
        }
        hops.extend(got);
        if let Some((o, a)) = found.or(weak) {
            return Ok((hops, o, Some(a)));
        }
        if next.is_empty() {
            break;
        }
        next.truncate(4);
        want = next;
    }
    Ok((hops, Vec::new(), None))
}

/// What the model's picture draws under the spot: its things (bodies by
/// kind, HUD fields) whose box the spot touches.
pub fn things_under(things: &[(String, [i64; 4])], spot: Spot) -> Vec<Value> {
    // A HUD text field is named by its text in quotes; an empty one names
    // nothing.
    let mut seen: Vec<(&str, [i64; 4])> = Vec::new();
    things
        .iter()
        .filter(|(n, r)| !n.is_empty() && n != "\"\"" && r[2] > 0 && r[3] > 0 && spot.touches(*r))
        .filter(|(n, r)| {
            let new = !seen.contains(&(n.as_str(), *r));
            seen.push((n.as_str(), *r));
            new
        })
        .map(|(n, r)| json!({"name": n, "rect": r}))
        .collect()
}

/// The two pictures in the spot: pixels compared, pixels that differ, and
/// sprite pixels in each.
pub fn pixels_in(o: &Frame, m: &Frame, spot: Spot) -> Value {
    let (w, h) = (i64::from(o.w.min(m.w)), i64::from(o.h.min(m.h)));
    let (mut n, mut differ, mut os, mut ms) = (0u64, 0u64, 0u64, 0u64);
    for y in spot.y.max(0)..(spot.y + spot.h).min(h) {
        for x in spot.x.max(0)..(spot.x + spot.w).min(w) {
            let (io, im) = (
                (y * i64::from(o.w) + x) as usize,
                (y * i64::from(m.w) + x) as usize,
            );
            n += 1;
            if o.rgb.get(io) != m.rgb.get(im) {
                differ += 1;
            }
            os += u64::from(o.sprite.get(io).copied().unwrap_or(false));
            ms += u64::from(m.sprite.get(im).copied().unwrap_or(false));
        }
    }
    json!({"compared": n, "differ": differ, "original_sprite": os, "model_sprite": ms})
}

/// Per compared frame, what the model's run kept for the spot.
#[derive(Default, Clone)]
struct Seen {
    things: Vec<(String, [i64; 4])>,
    pixels: Value,
    size: (u32, u32),
}

/// `gp element explain` as JSON ([`SCHEMA`]).
pub fn explain(paths: &Paths, a: &ExplainArgs) -> Result<Value> {
    let t0 = Instant::now();
    let game = &a.run;
    let clip = crate::clip_score::corpus_clip(game, &a.clip)
        .with_context(|| format!("no clip {} in {}'s corpus", a.clip, game.display()))?;
    let (log, start) = clip.log()?;
    let rom = crate::clip_score::rom_of(paths, game, a.rom.as_deref())?;
    let platform = Platform::of_file(&rom);
    let frame = match (a.frame, a.log_frame) {
        (Some(f), _) => f,
        (None, Some(lf)) => {
            anyhow::ensure!(
                lf > start as u64,
                "frame {lf} from power-on is at or before the clip's start ({start}); the clip's frames are {} to {}",
                start + 1,
                log.frames.len()
            );
            lf - start as u64
        }
        (None, None) => bail!(
            "give the frame: --frame N (from the clip's start) or --log-frame N (from power-on, as the page counts)"
        ),
    };
    let clip_frames = log.frames.len().saturating_sub(start) as u64;
    anyhow::ensure!(
        (1..=clip_frames).contains(&frame),
        "frame {frame} isn't one of the clip's frames, 1 to {clip_frames}"
    );
    let of = start as u64 + frame;
    let screen: (i64, i64) = match platform {
        Platform::Nes => (256, 240),
        Platform::Arcade => (
            gp_arcade::video::UNIT as i64 * 256,
            gp_arcade::video::UNIT as i64 * 240,
        ),
    };
    let spot =
        parse_spot(a.at.as_deref(), a.spot_box.as_deref(), screen).map_err(anyhow::Error::msg)?;
    let model_dir = a.model.clone().unwrap_or_else(|| game.clone());
    let mfile = model_dir.join(gp_interp::mapping::MAPPING_FILE);
    let mapping = Mapping::parse(
        &std::fs::read_to_string(&mfile).with_context(|| format!("reading {}", mfile.display()))?,
    )
    .map_err(|e| anyhow::anyhow!("{}: {e}", mfile.display()))?;
    let model = crate::mechanics::load_model(&model_dir).ok();
    let (names, labels_file, labels_missing) = crate::element::names_for(game, a.labels.as_deref());
    let mut unknown: Vec<String> = Vec::new();
    if let Some(why) = &labels_missing {
        unknown.push(format!(
            "routine and byte names: {why}, so they are addresses"
        ));
    }

    // The original on the frame: its sprites, the tile at the spot's
    // middle and its memory.
    let t_orig = Instant::now();
    let orig = match platform {
        Platform::Nes => nes_frame(paths, &rom, &log, start, of)?,
        Platform::Arcade => arcade_frame(&rom, &log, of)?,
    };
    let byte = |addr: u16| {
        orig.mem
            .get(usize::from(addr))
            .copied()
            .filter(|_| orig.readable(addr))
    };
    let under = sprites_under(&orig.sprites, spot);
    let (mx, my) = spot.mid();
    let tile = orig.tile_at(mx, my, model.as_ref(), paths);
    let hud: Vec<Value> = model
        .as_ref()
        .map(|m| {
            crate::serve::pointing::hud_under(
                m,
                gp_runtime::Rect {
                    x: spot.x,
                    y: spot.y,
                    w: spot.w as u64,
                    h: spot.h as u64,
                },
            )
            .into_iter()
            .map(|(what, b)| json!({"counter": what, "box": b}))
            .collect()
        })
        .unwrap_or_default();
    if let Some(n) = &orig.note {
        unknown.push(n.clone());
    }
    let orig_ms = t_orig.elapsed().as_millis();

    // The code: who wrote the sprite's bytes, and whose bytes those were.
    let t_code = Instant::now();
    let ram = |addr: u16| orig.ram(addr);
    let raw_slice = |addrs: &[u16]| -> Result<Vec<Hop>> {
        match platform {
            Platform::Nes => addrs
                .iter()
                .map(|&addr| {
                    let v = crate::element::inputs(
                        paths,
                        &InputsArgs {
                            run: game.clone(),
                            clip: a.clip.clone(),
                            frame: Some(frame),
                            byte: Some(hex(addr)),
                            depth: 1,
                            back: true,
                            labels: a.labels.clone(),
                            model: a.model.clone(),
                            rom: a.rom.clone(),
                            ..InputsArgs::default()
                        },
                    )?;
                    Ok(hop_of(addr, &v["bytes"][0], &ram))
                })
                .collect(),
            Platform::Arcade => {
                let set = gp_arcade::Romset::from_zip(&rom)?;
                let v = crate::element_z80::inputs(&set, &log.frames, start, frame, addrs, &names)?;
                Ok(addrs
                    .iter()
                    .zip(&v)
                    .map(|(&ad, b)| hop_of(ad, b, &ram))
                    .collect())
            }
        }
    };
    // Each byte sliced once; an arcade slice runs the board from power-on,
    // so the bytes known to be wanted go in one run.
    let mut memo: std::collections::BTreeMap<u16, Hop> = std::collections::BTreeMap::new();
    let mut slice = |addrs: &[u16]| -> Result<Vec<Hop>> {
        let missing: Vec<u16> = addrs
            .iter()
            .copied()
            .filter(|a| !memo.contains_key(a))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if !missing.is_empty() {
            for h in raw_slice(&missing)? {
                memo.insert(h.addr, h);
            }
        }
        Ok(addrs.iter().filter_map(|a| memo.get(a).cloned()).collect())
    };
    if platform == Platform::Arcade {
        let mut first: Vec<u16> = under
            .first()
            .map(|s| s.x_addr.into_iter().chain(s.y_addr).collect())
            .unwrap_or_default();
        first.extend(
            tile.as_ref()
                .and_then(|t| t["bytes"][0].as_str())
                .and_then(|b| addr_of(&json!(b))),
        );
        if !first.is_empty() {
            slice(&first)?;
        }
    }
    let owned = |addr: u16| owners(&mapping, addr, &byte);
    let mut drawn_by: Option<Hop> = None;
    let mut hops: Vec<Hop> = Vec::new();
    let mut found: Vec<Owner> = Vec::new();
    let mut found_at: Option<u16> = None;
    let mut found_how = "code";
    let lead = under.first().cloned();
    let mut n_starts = 0;
    if let Some(s) = &lead {
        let starts: Vec<u16> = s.x_addr.into_iter().chain(s.y_addr).collect();
        n_starts = starts.len();
        if starts.is_empty() {
            unknown.push(format!(
                "who wrote sprite {}: the sprite table's page in RAM wasn't seen copied to OAM on the frame",
                s.index
            ));
        } else {
            let (h, o, at) = follow(&starts, &mut slice, &owned)?;
            // Who drew it: the latest write of its x or y byte.
            drawn_by = h
                .iter()
                .take(starts.len())
                .filter(|h| h.written)
                .max_by_key(|h| (h.frame_written, std::cmp::Reverse(h.addr != starts[0])))
                .or(h.first())
                .cloned();
            hops = h;
            found = o;
            found_at = at;
            if found.is_empty() {
                for &a in starts.iter().chain(hops.iter().flat_map(|h| h.from.iter())) {
                    found = owners_by_block(&mapping, a, &byte);
                    if !found.is_empty() {
                        found_at = Some(a);
                        found_how = "place";
                        break;
                    }
                }
            }
            if found.is_empty() {
                unknown.push(format!(
                    "which game object sprite {} is: the mapping names none of the bytes its x and y came from ({} write{} followed back from {})",
                    s.index,
                    hops.len(),
                    if hops.len() == 1 { "" } else { "s" },
                    starts.iter().map(|a| hex(*a)).collect::<Vec<_>>().join(" and ")
                ));
            }
        }
    } else if hud.is_empty() {
        unknown.push("which game object is here: no sprite is drawn in the spot on this frame, and the mapping's fields don't name background cells".to_owned());
    }
    // A HUD counter's bytes, when the spot covers one and no sprite does.
    if found.is_empty()
        && lead.is_none()
        && let Some(c) = hud.first()
    {
        let counter = c["counter"].as_str().unwrap_or("");
        for f in &mapping.fields {
            if (f.path.rsplit('.').next() == Some(counter)
                || f.path == format!("counters.{counter}"))
                && let Some(&addr) = f.ram.first()
                && found.is_empty()
            {
                found.extend(owned(addr).into_iter().filter(|o| o.path == f.path));
                found_at = Some(addr);
                found_how = "hud";
            }
        }
        if found.is_empty() {
            unknown.push(format!(
                "which RAM holds the {counter}: the mapping has no field for it"
            ));
        }
    }
    let owner = found.first().cloned();
    // What moves it: the writers of its x and y bytes (or the counter's).
    let mut moved_by: Vec<Hop> = Vec::new();
    if let Some(o) = &owner {
        let want: Vec<u16> = if o.slot.is_some() || o.thing.starts_with("entities.") {
            ["x", "y"]
                .iter()
                .filter_map(|f| field_addr(&mapping, o, f))
                .collect()
        } else {
            mapping
                .fields
                .iter()
                .find(|f| f.path == o.path)
                .and_then(|f| f.ram.first().copied())
                .into_iter()
                .collect()
        };
        if want.is_empty() {
            unknown.push(format!(
                "what moves the {}: the mapping has no x or y for it",
                o.entity.as_deref().unwrap_or(&o.thing)
            ));
        }
        let todo: Vec<u16> = want
            .iter()
            .copied()
            .filter(|a| !hops.iter().any(|h| h.addr == *a))
            .collect();
        let fresh = if todo.is_empty() {
            Vec::new()
        } else {
            slice(&todo)?
        };
        for a in want {
            if let Some(h) = hops.iter().chain(&fresh).find(|h| h.addr == a) {
                moved_by.push(h.clone());
            }
        }
        if !moved_by.is_empty() && !moved_by.iter().any(|h| h.written) {
            unknown.push(format!(
                "what moves it: its bytes {} weren't written from the clip's start to this frame, so it held still since the start, and the code that set them ran before the clip",
                moved_by.iter().map(|h| hex(h.addr)).collect::<Vec<_>>().join(" and ")
            ));
        }
    }
    // An arcade tile's bytes are in video RAM the CPU writes: who wrote
    // the cell, as for a sprite.
    let mut tile = tile;
    if platform == Platform::Arcade
        && let Some(t) = tile.as_mut()
        && let Some(addr) = t["bytes"][0].as_str().and_then(|b| addr_of(&json!(b)))
        && let Some(h) = slice(&[addr])?.into_iter().next()
    {
        t["writer"] = if h.written {
            json!({"routine": h.routine, "pc": h.pc, "frame_written": h.frame_written, "sentence": h.sentence,
                   "from": format!("gp element inputs --run {} --clip {} --frame {frame} --byte '{}'", game.display(), a.clip, hex(addr))})
        } else {
            json!({"written": false, "note": format!("{} wasn't written from the clip's start to this frame: the cell came with the start", hex(addr))})
        };
    }
    let code_ms = t_code.elapsed().as_millis();

    // The model: its picture's things in the spot and the lockstep's
    // measures on the frame.
    let t_model = Instant::now();
    let mut model_v = Value::Null;
    if a.no_model {
        unknown.push("the model's side: not run (--no-model)".to_owned());
    } else {
        match model_side(paths, a, game, &log, start, of, spot, owner.as_ref()) {
            Ok(v) => model_v = v,
            Err(e) => unknown.push(format!("the model's side: {e:#}")),
        }
    }
    let model_ms = t_model.elapsed().as_millis();

    let object = owner.as_ref().map(|o| {
        let label = o.entity.as_deref().map_or_else(|| o.thing.replace('_', " "), |e| e.replace('_', " "));
        json!({
            "thing": o.thing, "entity": o.entity, "label": label, "slot": o.slot, "field": o.field, "path": o.path,
            "note": o.note,
            "fields": field_values(&mapping, o, &byte),
            "found_by": found_how,
            "from": if found_how == "place" {
                format!("the state mapping ({}) reads {} for {}; the spot's byte {} is in that slot's block", mfile.display(), field_addr(&mapping, o, "x").map_or_else(|| "its bytes".to_owned(), hex), o.path.replace(&format!(".{}", o.field), ".x"), found_at.map(hex).unwrap_or_default())
            } else {
                format!("the state mapping ({}) reads {} for {}", mfile.display(), found_at.map_or_else(|| "its bytes".to_owned(), hex), o.path)
            },
            "also": found.iter().skip(1).map(|x| x.path.clone()).collect::<Vec<_>>(),
        })
    });
    let hop_json = |h: &Hop| {
        json!({
            "byte": hex(h.addr), "written": h.written, "routine": h.routine, "pc": h.pc,
            "frame_written": h.frame_written, "sentence": h.sentence,
            "from": format!("gp element inputs --run {} --clip {} --frame {frame} --byte '{}'", game.display(), a.clip, hex(h.addr)),
            "slice": h.raw,
        })
    };
    if let Some(h) = &drawn_by
        && !h.written
    {
        unknown.push(format!("who drew sprite {}: {} wasn't written from the clip's start to this frame, so its value came with the start", lead.as_ref().map_or(0, |s| s.index), hex(h.addr)));
    }
    if let Some(t) = &tile
        && platform == Platform::Nes
        && t["name"].is_null()
    {
        unknown.push(format!(
            "what the background tile is called: the model's lifted metatiles name none with the tiles {}",
            t["metatile_tiles"].as_str().unwrap_or("")
        ));
    }
    if let Some(t) = &tile
        && t["writer"].is_null()
        && platform == Platform::Nes
    {
        unknown.push("who drew the background tile: the NES writes its nametables through the PPU's $2007 port, and the slice doesn't follow that port to a cell".to_owned());
    }
    let total = t0.elapsed().as_millis();
    let mut v = json!({
        "schema": SCHEMA,
        "platform": match platform { Platform::Nes => "nes", Platform::Arcade => "arcade" },
        "run": game,
        "clip": a.clip,
        "frame": frame,
        "original_frame": of,
        "start": start,
        "side": a.side.name(),
        "screen": [screen.0, screen.1],
        "spot": spot.json(),
        "labels": labels_file,
        "drawn": {
            "sprites": under.iter().take(8).map(sprite_json).collect::<Vec<_>>(),
            "more_sprites": under.len().saturating_sub(8),
            "tile": tile,
            "hud": hud,
            "from": orig.from,
        },
        "object": object,
        "drawn_by": drawn_by.as_ref().map(hop_json),
        "followed": hops.iter().skip(n_starts).map(hop_json).collect::<Vec<_>>(),
        "moved_by": moved_by.iter().map(hop_json).collect::<Vec<_>>(),
        "model": model_v,
        "unknown": unknown,
        "took_ms": {"total": total, "original": orig_ms, "code": code_ms, "model": model_ms},
    });
    v["words"] = json!(words(&v));
    Ok(v)
}

fn sprite_json(s: &Sprite) -> Value {
    json!({
        "index": s.index, "rect": s.rect, "picture": s.picture, "palette": s.palette,
        "flip_x": s.flip_x, "flip_y": s.flip_y,
        "x_byte": s.x_addr.map(hex), "y_byte": s.y_addr.map(hex),
    })
}

/// The original as one frame ended.
struct OrigFrame {
    platform: Platform,
    sprites: Vec<Sprite>,
    /// The main CPU's memory as a 64K image (the NES's 2KB of RAM at 0).
    mem: Vec<u8>,
    /// NES: the nametables, scroll and mirroring; arcade: video RAM.
    nt: Vec<u8>,
    scroll: [i64; 2],
    mirroring: u8,
    note: Option<String>,
    from: String,
}

impl OrigFrame {
    /// Whether the mapping's reads of `addr` are answered here.
    fn readable(&self, addr: u16) -> bool {
        match self.platform {
            Platform::Nes => addr < 0x800,
            Platform::Arcade => true,
        }
    }

    /// Whether `addr` is RAM the code keeps state in.
    fn ram(&self, addr: u16) -> bool {
        match self.platform {
            Platform::Nes => addr < 0x800 && !(0x100..0x200).contains(&addr),
            Platform::Arcade => (0xE000..0xF800).contains(&addr),
        }
    }

    /// The background tile under `x`, `y` (screen pixels).
    fn tile_at(
        &self,
        x: i64,
        y: i64,
        model: Option<&gp_model::Model>,
        paths: &Paths,
    ) -> Option<Value> {
        match self.platform {
            Platform::Nes => {
                let (tiles, at) =
                    crate::serve::pointing::cell_at(&self.nt, self.mirroring, self.scroll, x, y)?;
                let q = usize::from(y - at[1] >= 8) * 2 + usize::from(x - at[0] >= 8);
                let metatiles = model
                    .map(|m| {
                        let source = m.model.source.clone().unwrap_or_default();
                        crate::serve::pointing::model_metatiles(
                            m,
                            &paths.home.join("work/assets").join(source),
                        )
                    })
                    .unwrap_or_default();
                let mt = metatiles.iter().find(|m| m.tiles == tiles);
                let (lx, ly) = (
                    (x + self.scroll[0]).rem_euclid(512),
                    (y + self.scroll[1]).rem_euclid(480),
                );
                Some(json!({
                    "nametable": (lx / 256 + 2 * (ly / 240)),
                    "cell": [(lx % 256) / 8, (ly % 240) / 8],
                    "tile_rect": [at[0] + 8 * (q as i64 % 2), at[1] + 8 * (q as i64 / 2), 8, 8],
                    "metatile_rect": [at[0], at[1], 16, 16],
                    "picture": tiles[q],
                    "metatile_tiles": gp_gaps::sig(&tiles),
                    "name": mt.map(|m| m.name.clone()),
                    "role": mt.and_then(|m| m.role.clone()),
                    "asset": mt.map(|m| format!("{}[{}]", m.asset, m.index)),
                    "writer": Value::Null,
                    "from": format!("the nametables at the frame's scroll ({}, {}), mirroring {}{}", self.scroll[0], self.scroll[1], self.mirroring,
                        if mt.is_some() { ", named by the model's lifted metatiles" } else { "" }),
                }))
            }
            Platform::Arcade => {
                let (col, row) = (x / 16, y / 16);
                if !(0..32).contains(&col) || !(0..30).contains(&row) {
                    return None;
                }
                let i = (row * 32 + col) as usize;
                let t = gp_arcade::video::tilemap(&self.nt).get(i).copied()?;
                Some(json!({
                    "cell": [col, row],
                    "tile_rect": [col * 16, row * 16, 16, 16],
                    "picture": t.code,
                    "palette": t.color,
                    "flip_x": t.flip_x, "flip_y": t.flip_y, "category": t.category,
                    "bytes": [hex(0xF000 + 2 * i as u16), hex(0xF001 + 2 * i as u16)],
                    "writer": Value::Null,
                    "from": "video RAM ($F000) as the frame ended, read as gp-arcade's tilemap draws it",
                }))
            }
        }
    }
}

/// The NES's frame `of`: from the clip's start (the start cache keeps it),
/// the last two frames traced to see the sprite table's page copied to OAM.
fn nes_frame(
    paths: &Paths,
    rom: &[u8],
    log: &gp_evidence::InputLog,
    start: usize,
    of: u64,
) -> Result<OrigFrame> {
    use gp_nes::slice::Recorder;
    let mut nes = crate::code::start_state(
        &paths.home,
        rom,
        gp_evidence::manifest::PowerOn::default(),
        &log.frames,
        start,
    )?;
    nes.set_events(false);
    let mut page: Option<u8> = None;
    for (k, i) in log.frames.iter().enumerate().take(of as usize).skip(start) {
        let f = k as u64 + 1;
        let input = gp_evidence::input_log::InputFrame {
            polled: false,
            ..*i
        };
        if f + 2 > of {
            let mut rec = Recorder::new();
            rec.start(nes.bus_mut_for_observers());
            nes.step_with(input, |bus| rec.before(bus))
                .map_err(|e| anyhow::anyhow!("the original, frame {f}: {e}"))?;
            let t = rec.finish();
            for s in &t.steps {
                if s.addr == Some(0x4014) {
                    match s.op {
                        0x8D => page = Some(s.a),
                        0x8E => page = Some(s.x),
                        0x8C => page = Some(s.y),
                        _ => {}
                    }
                }
            }
        } else {
            nes.step(input)
                .map_err(|e| anyhow::anyhow!("the original, frame {f}: {e}"))?;
        }
    }
    let oam = nes.oam();
    let tall = nes.tall_sprites();
    let mut mem = vec![0u8; 0x10000];
    mem[..nes.wram().len()].copy_from_slice(nes.wram());
    let state = nes.canonical_state()?;
    let table = gp_nes::canonical::field_table();
    let fields: std::collections::BTreeMap<String, Vec<u8>> = table
        .split(&state)
        .map_err(anyhow::Error::from)?
        .into_iter()
        .map(|(n, b)| (n.to_owned(), b.to_vec()))
        .collect();
    let get = |n: &str| fields.get(n).cloned().unwrap_or_default();
    let t = get("ppu.t");
    let t_reg = if t.len() >= 2 {
        u16::from_le_bytes([t[0], t[1]])
    } else {
        0
    };
    let fine = get("ppu.x").first().map_or(0, |b| u16::from(b & 7));
    let scroll = [
        i64::from((t_reg & 31) * 8 + fine + ((t_reg >> 10) & 1) * 256),
        i64::from(((t_reg >> 5) & 31) * 8 + ((t_reg >> 12) & 7) + ((t_reg >> 11) & 1) * 240),
    ];
    let note = page.is_none().then(|| {
        "the sprite table's page: no write to $4014 (the copy to OAM) on the frame or the one before, so the sprites' bytes in RAM aren't known".to_owned()
    });
    Ok(OrigFrame {
        platform: Platform::Nes,
        sprites: nes_sprites(&oam, tall, page),
        mem,
        nt: get("nametables"),
        scroll,
        mirroring: get("mapper.mirroring").first().copied().unwrap_or(1),
        note,
        from: format!(
            "the original in gp-nes from the clip's start (the start cache) to frame {of}: OAM as the frame ended{}",
            page.map_or(String::new(), |p| format!(
                ", copied from the sprite table at ${p:02X}00 (the write to $4014)"
            ))
        ),
    })
}

/// The arcade board's frame `of`, from power-on as `gp element inputs`
/// runs it. The board draws its picture at vertical blank, and the code
/// moves its sprites after, so the sprite RAM a frame drew from is the one
/// that draws its picture again: the RAM as the frame before ended, or as
/// this one ended. Each is drawn and set against the frame's own picture,
/// and the one that matches is read; when neither does, that is said.
fn arcade_frame(rom: &[u8], log: &gp_evidence::InputLog, of: u64) -> Result<OrigFrame> {
    use gp_arcade::video;
    let set = gp_arcade::Romset::from_zip(rom)?;
    let mut t = gp_arcade::Tapper::power_on(&set);
    let mut before: Option<(Vec<u8>, Vec<u8>)> = None;
    let ram_of = |t: &gp_arcade::Tapper| -> (Vec<u8>, Vec<u8>) {
        (
            (0xE800..0xEA00u16).map(|a| t.peek(a)).collect(),
            (0xF000..0xF800u16).map(|a| t.peek(a)).collect(),
        )
    };
    for (k, f) in log.frames.iter().enumerate().take(of as usize) {
        if k + 1 == of as usize {
            before = Some(ram_of(&t));
        }
        t.step(gp_arcade::Controls::from_bits(f.buttons_word() & 0xff));
    }
    let mem: Vec<u8> = (0..=0xFFFFu16).map(|a| t.peek(a)).collect();
    let after = ram_of(&t);
    let drawn = t.pens().0.to_vec();
    let gfx = video::Gfx::decode(&set.gfx1, &set.gfx2);
    let draws = |(sprites, tiles): &(Vec<u8>, Vec<u8>)| {
        let mut pens = vec![0u8; video::WIDTH * video::HEIGHT];
        video::draw(&gfx, tiles, sprites, false, &mut pens);
        pens == drawn
    };
    let (pick, note, which) = if before.as_ref().is_some_and(draws) {
        (
            before.clone().unwrap_or_default(),
            None,
            format!(
                "as frame {} ended (the RAM the board drew frame {of} from at its vertical blank)",
                of - 1
            ),
        )
    } else if draws(&after) {
        (
            after.clone(),
            None,
            format!("as frame {of} ended, which draws its picture"),
        )
    } else {
        (
            after.clone(),
            Some("the sprites' places: neither the sprite RAM as the frame began nor as it ended draws the frame's own picture, so a sprite may have moved while it was drawn; the places are the RAM as the frame ended".to_owned()),
            format!("as frame {of} ended"),
        )
    };
    Ok(OrigFrame {
        platform: Platform::Arcade,
        sprites: arcade_sprites(&pick.0, 0xE800),
        nt: pick.1,
        mem,
        scroll: [0, 0],
        mirroring: 0,
        note,
        from: format!(
            "the original in gp-arcade from power-on to frame {of}: sprite RAM ($E800) and video RAM ($F000) {which}"
        ),
    })
}

/// The model's side: the clip's lockstep straight through with both
/// pictures, as `gp clip score` and REVIEW REBUILD run it.
#[allow(clippy::too_many_arguments)]
fn model_side(
    paths: &Paths,
    a: &ExplainArgs,
    game: &Path,
    log: &gp_evidence::InputLog,
    start: usize,
    of: u64,
    spot: Spot,
    owner: Option<&Owner>,
) -> Result<Value> {
    let ctx = crate::clip_score::Ctx::open(
        paths,
        game,
        &crate::clip_score::Open {
            module: a.module.as_deref(),
            model: a.model.as_deref(),
            assets: a.assets.as_deref(),
            rom: a.rom.as_deref(),
        },
    )?;
    let seen: Rc<RefCell<Vec<Seen>>> = Rc::default();
    let keep = {
        let seen = seen.clone();
        Box::new(move |pair: Option<(&Frame, &Frame)>| {
            let s = match pair {
                None => Seen::default(),
                Some((o, m)) => Seen {
                    things: m.things.iter().map(|t| (t.name.clone(), t.rect)).collect(),
                    pixels: pixels_in(o, m, spot),
                    size: (o.w, o.h),
                },
            };
            seen.borrow_mut().push(s);
        }) as crate::screen::Keep
    };
    // Only up to the frame: the rest of the clip says nothing here. (The
    // clip's score tape is keyed by its whole input, so the original is
    // run from the start cache for these frames instead.)
    let mut upto = log.clone();
    upto.frames.truncate(of as usize + 2);
    let (ls, _report, _draws) = crate::clip_score::pictures(&ctx, &upto, start, keep, true)?;
    let k = ls
        .frames
        .iter()
        .position(|f| f.1 == of)
        .with_context(|| format!("the lockstep compared no frame at the original's {of} (a lag frame, or past the run's scope)"))?;
    let (model_frame, _) = ls.frames[k];
    let seen = seen.borrow();
    let at = seen.get(k).cloned().unwrap_or_default();
    let things = things_under(&at.things, spot);
    // The object's measures on the frame, both games.
    let measure = |i: usize| -> Option<Value> {
        let p = ls.paths.get(i)?;
        let (o, m) = ls.values.get(k)?;
        let unmeasured = ls
            .unmeasured
            .get(k)
            .and_then(|u| u.get(i))
            .copied()
            .unwrap_or(false);
        let d = p.distance.get(k)?;
        Some(json!({
            "path": p.path, "original": o.get(i).map(shown), "model": m.get(i).map(shown),
            "agree": unmeasured || *d <= p.tolerance,
            "unmeasured": unmeasured,
        }))
    };
    let mut measures: Vec<Value> = Vec::new();
    let mut body_v = Value::Null;
    if let Some(o) = owner {
        match o.slot {
            Some(slot) => {
                let alive = |b: &gp_interp::bodies::Body| {
                    b.paths
                        .first()
                        .and_then(|&i| ls.values.get(k).and_then(|(ov, _)| ov.get(i)))
                        .is_some_and(|v| !shown(v).is_empty())
                };
                let body = ls
                    .bodies
                    .iter()
                    .filter(|b| b.slot == Some(slot) && Some(&b.kind) == o.entity.as_ref())
                    .find(|b| alive(b))
                    .or_else(|| {
                        ls.bodies
                            .iter()
                            .find(|b| b.slot == Some(slot) && Some(&b.kind) == o.entity.as_ref())
                    });
                if let Some(b) = body {
                    measures = b.paths.iter().filter_map(|&i| measure(i)).collect();
                    let model_has = b
                        .paths
                        .first()
                        .and_then(|&i| ls.values.get(k).and_then(|(_, mv)| mv.get(i)))
                        .is_some_and(|v| !shown(v).is_empty());
                    body_v = json!({
                        "name": b.name(), "nth": b.nth, "slot": b.slot, "paired": b.paired.is_some() || b.mapped,
                        "one_sided": b.one_sided(), "model_has_it": model_has,
                    });
                }
            }
            None => {
                let pre = format!("{}.", o.thing);
                measures = ls
                    .paths
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| p.path.starts_with(&pre) || p.path == o.path)
                    .filter_map(|(i, _)| measure(i))
                    .collect();
            }
        }
    }
    let agree = (!measures.is_empty()).then(|| measures.iter().all(|m| m["agree"] == true));
    // An arcade model that names none of its board's art draws its bodies
    // by tile kind, so its pixels aren't the board's to compare.
    let board_art =
        (crate::original::Platform::of_file(ctx.rom()) == Platform::Arcade).then(|| {
            crate::mechanics::load_model(a.model.as_deref().unwrap_or(game))
                .ok()
                .is_some_and(|m| gp_look::board::names_board(&m))
        });
    Ok(json!({
        "model_frame": model_frame,
        "compared_index": k,
        "things": things,
        "pixels": at.pixels,
        "picture": [at.size.0, at.size.1],
        "body": body_v,
        "measures": measures,
        "agree": agree,
        "board_art": board_art,
        "from": "the clip's lockstep straight through with both pictures, as gp clip score and REVIEW REBUILD run it (from the clip's score tape when there is one)",
    }))
}

/// The answer in plain words, from the JSON.
/// A sentence cut to a readable length, saying so.
fn short(s: &str) -> String {
    const MAX: usize = 320;
    if s.chars().count() <= MAX {
        return s.to_owned();
    }
    let cut: String = s.chars().take(MAX).collect();
    format!("{cut}... (cut; the whole sentence is in the JSON)")
}

pub fn words(v: &Value) -> String {
    let mut out: Vec<String> = Vec::new();
    let spot = &v["spot"];
    let at = if spot["w"] == 1 && spot["h"] == 1 {
        format!("the point {},{}", spot["x"], spot["y"])
    } else {
        format!(
            "the box {},{} {}x{}",
            spot["x"], spot["y"], spot["w"], spot["h"]
        )
    };
    out.push(format!(
        "{} at frame {} (the original's {}), {} on the {} picture:",
        v["clip"].as_str().unwrap_or(""),
        v["frame"],
        v["original_frame"],
        at,
        v["side"].as_str().unwrap_or("original")
    ));
    let d = &v["drawn"];
    let sprites = d["sprites"].as_array().cloned().unwrap_or_default();
    if let Some(s) = sprites.first() {
        let r = &s["rect"];
        out.push(format!(
            "- Drawn there in the original: sprite {} (picture {}, palette {}) at {},{} {}x{}{}.",
            s["index"],
            s["picture"],
            s["palette"],
            r[0],
            r[1],
            r[2],
            r[3],
            if sprites.len() > 1 {
                format!(
                    ", with {} more sprite{} in the spot ({})",
                    sprites.len() - 1,
                    if sprites.len() == 2 { "" } else { "s" },
                    sprites
                        .iter()
                        .skip(1)
                        .map(|s| format!("{} picture {}", s["index"], s["picture"]))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            } else {
                String::new()
            }
        ));
    } else {
        out.push("- No sprite of the original is in the spot on this frame.".to_owned());
    }
    if let Some(t) = d["tile"].as_object() {
        let name = t
            .get("name")
            .and_then(Value::as_str)
            .map(|n| format!(", {}", crate::serve::pointing::a(n)))
            .unwrap_or_default();
        out.push(format!(
            "- The background tile at its middle: cell {},{}, picture {}{name}.",
            t["cell"][0], t["cell"][1], t["picture"]
        ));
        if let Some(w) = t.get("writer").filter(|w| w.is_object()) {
            match w["sentence"].as_str() {
                Some(sn) => out.push(format!(
                    "  Its cell was written by {} on frame {}: {}",
                    w["routine"].as_str().unwrap_or("?"),
                    w["frame_written"],
                    short(sn)
                )),
                None => out.push(format!("  {}.", w["note"].as_str().unwrap_or(""))),
            }
        }
    }
    for h in d["hud"].as_array().into_iter().flatten() {
        out.push(format!(
            "- It covers the HUD's {}.",
            h["counter"].as_str().unwrap_or("").replace('_', " ")
        ));
    }
    if let Some(o) = v["object"].as_object() {
        {
            let slot = o
                .get("slot")
                .and_then(Value::as_u64)
                .map(|s| format!(" in slot {s}"))
                .unwrap_or_default();
            let fields: Vec<String> = o
                .get("fields")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|f| {
                    Some(format!(
                        "{} {}",
                        f["path"].as_str()?,
                        f["original"].as_str()?
                    ))
                })
                .take(8)
                .collect();
            out.push(format!(
                "- It is the {}{slot} ({}){}{}.",
                o["label"].as_str().unwrap_or(""),
                o["from"].as_str().unwrap_or(""),
                if fields.is_empty() {
                    String::new()
                } else {
                    format!("; the original's values: {}", fields.join(", "))
                },
                o.get("note")
                    .and_then(Value::as_str)
                    .map(|n| format!("; {n}"))
                    .unwrap_or_default()
            ));
        }
    }
    if let Some(h) = v["drawn_by"].as_object().filter(|h| h["written"] == true) {
        out.push(format!(
            "- Drawn by {} (it wrote the sprite's byte {} at {}, frame {}): {}",
            h["routine"].as_str().unwrap_or("?"),
            h["byte"].as_str().unwrap_or(""),
            h["pc"].as_str().unwrap_or(""),
            h["frame_written"],
            short(h["sentence"].as_str().unwrap_or(""))
        ));
    }
    // The first write followed back, when the drawing copied the object's
    // bytes from elsewhere; the rest are in the JSON's `followed`.
    let followed: Vec<&Value> = v["followed"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|h| h["written"] == true)
        .collect();
    if let Some(h) = followed.first() {
        out.push(format!(
            "  Followed back from {}: {}{}",
            h["byte"].as_str().unwrap_or(""),
            short(h["sentence"].as_str().unwrap_or("")),
            if followed.len() > 1 {
                format!(
                    " ({} more writes followed back in the JSON)",
                    followed.len() - 1
                )
            } else {
                String::new()
            }
        ));
    }
    for h in v["moved_by"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|h| h["written"] == true)
    {
        let mut reads: Vec<String> = std::iter::once(&h["slice"]["reads"])
            .chain(
                h["slice"]["feeds"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|f| &f["reads"]),
            )
            .chain(
                h["slice"]["earlier"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|f| &f["reads"]),
            )
            .flat_map(|r| r.as_array().cloned().unwrap_or_default())
            .filter_map(|r| {
                let name = r["name"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| r["addr"].as_u64().map(|a| hex(a as u16)))?;
                Some(format!("{name} ({})", r["how"].as_str().unwrap_or("")))
            })
            .collect();
        let mut seen = BTreeSet::new();
        reads.retain(|r| seen.insert(r.clone()));
        reads.truncate(12);
        out.push(format!(
            "- Moved by {} (it wrote {} at {}, frame {}){}.",
            h["routine"]
                .as_str()
                .filter(|r| *r != "?")
                .unwrap_or("code outside any call the trace saw"),
            h["byte"].as_str().unwrap_or(""),
            h["pc"].as_str().unwrap_or(""),
            h["frame_written"],
            if reads.is_empty() {
                String::new()
            } else {
                format!("; that code read {}", reads.join(", "))
            }
        ));
    }
    let m = &v["model"];
    if m.is_object() {
        let things: Vec<String> = m["things"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| t["name"].as_str().map(str::to_owned))
            .collect();
        let px = &m["pixels"];
        out.push(format!(
            "- The model (its frame {}): {}; {} of the {} pixels in the spot differ between the two pictures.",
            m["model_frame"],
            if things.is_empty() { "its picture draws no body or HUD field in the spot".to_owned() } else { format!("its picture draws {} there", things.join(", ")) },
            px["differ"], px["compared"]
        ));
        if let Some(b) = m["body"].as_object() {
            out.push(format!(
                "- The lockstep pairs this object with a body of the model's: {}{}.",
                b["name"].as_str().unwrap_or("?"),
                if b["model_has_it"] == false {
                    ", which the model doesn't have on this frame"
                } else {
                    ""
                }
            ));
        }
        if m["board_art"] == false {
            out.push("  (This arcade model names none of the board's art, so it draws its bodies by tile kind: its pixels differ from the board's wherever it draws, and say nothing here.)".to_owned());
        }
        let ms = m["measures"].as_array().cloned().unwrap_or_default();
        if !ms.is_empty() {
            let off: Vec<String> = ms
                .iter()
                .filter(|x| x["agree"] == false)
                .map(|x| {
                    format!(
                        "{} original {} model {}",
                        x["path"].as_str().unwrap_or(""),
                        x["original"].as_str().unwrap_or("-"),
                        x["model"].as_str().unwrap_or("-")
                    )
                })
                .collect();
            if off.is_empty() {
                out.push(format!(
                    "- The two games agree on its {} measures on this frame ({}).",
                    ms.len(),
                    ms.iter()
                        .map(|x| x["path"]
                            .as_str()
                            .unwrap_or("")
                            .rsplit('.')
                            .next()
                            .unwrap_or("")
                            .to_owned())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                if px["differ"].as_u64().unwrap_or(0) > 0 && m["board_art"] != false {
                    out.push("  Their pictures still differ in the spot: the lockstep measures the mapped fields, not which picture is drawn.".to_owned());
                }
                let pictured = ms.iter().any(|x| {
                    let p = x["path"].as_str().unwrap_or("");
                    ["pic", "frame", "anim", "pose", "sprite"]
                        .iter()
                        .any(|w| p.contains(w))
                });
                if !pictured && let Some(s) = v["drawn"]["sprites"][0].as_object() {
                    out.push(format!(
                        "- Not determined: whether the model shows the same picture: no measure of the lockstep is the picture (the original draws picture {} here), so only its place and state are compared.",
                        s["picture"]
                    ));
                }
            } else {
                out.push(format!(
                    "- They disagree on this frame: {}.",
                    off.join("; ")
                ));
            }
        } else if v["object"].is_object() {
            out.push("- The lockstep measures nothing of this object on this frame.".to_owned());
        }
    }
    for u in v["unknown"].as_array().into_iter().flatten() {
        out.push(format!(
            "- Not determined: {}.",
            u.as_str().unwrap_or("").trim_end_matches('.')
        ));
    }
    let t = &v["took_ms"];
    out.push(format!(
        "Took {:.1} s (the original {:.1} s, the code {:.1} s, the model {:.1} s).",
        ms_s(&t["total"]),
        ms_s(&t["original"]),
        ms_s(&t["code"]),
        ms_s(&t["model"])
    ));
    out.join("\n")
}

fn ms_s(v: &Value) -> f64 {
    v.as_u64().unwrap_or(0) as f64 / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapping(text: &str) -> Mapping {
        Mapping::parse(text).unwrap()
    }

    #[test]
    fn a_spot_is_a_point_or_a_box_on_the_picture() {
        let s = parse_spot(Some("10,20"), None, (256, 240)).unwrap();
        assert_eq!(
            s,
            Spot {
                x: 10,
                y: 20,
                w: 1,
                h: 1
            }
        );
        let b = parse_spot(None, Some("250,230,16,16"), (256, 240)).unwrap();
        assert_eq!((b.w, b.h), (6, 10));
        assert!(parse_spot(None, None, (256, 240)).is_err());
        assert!(parse_spot(Some("300,1"), None, (256, 240)).is_err());
        assert!(parse_spot(None, Some("1,2,3"), (256, 240)).is_err());
        assert!(parse_spot(Some("300,1"), None, (512, 480)).is_ok());
    }

    #[test]
    fn nes_sprites_are_placed_and_know_their_table_bytes() {
        let mut oam = [0xFFu8; 256];
        // Sprite 2 at x 100, OAM y 49 (drawn from 50), tile 7, palette 1
        // flipped across.
        oam[8..12].copy_from_slice(&[49, 7, 0x41, 100]);
        // Sprite 5 a little right.
        oam[20..24].copy_from_slice(&[49, 9, 0, 106]);
        let s = nes_sprites(&oam, false, Some(2));
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].rect, [100, 50, 8, 8]);
        assert_eq!(s[0].x_addr, Some(0x020B));
        assert_eq!(s[0].y_addr, Some(0x0208));
        assert!(s[0].flip_x && !s[0].flip_y);
        assert_eq!(s[0].palette, 1);
        // The one nearest the middle of the spot comes first.
        let u = sprites_under(
            &s,
            Spot {
                x: 104,
                y: 50,
                w: 8,
                h: 8,
            },
        );
        assert_eq!(u.iter().map(|s| s.index).collect::<Vec<_>>(), [5, 2]);
        assert!(
            sprites_under(
                &s,
                Spot {
                    x: 0,
                    y: 0,
                    w: 8,
                    h: 8
                }
            )
            .is_empty()
        );
        // No page seen: no bytes.
        assert_eq!(nes_sprites(&oam, true, None)[0].x_addr, None);
        assert_eq!(nes_sprites(&oam, true, None)[0].rect[3], 16);
    }

    #[test]
    fn arcade_sprites_are_in_screen_pixels() {
        let mut ram = vec![0u8; 0x200];
        // Slot 3: y byte 200, x byte 50, code 12: placed at 47, 41 units.
        ram[12..16].copy_from_slice(&[200, 0, 12, 50]);
        let s: Vec<Sprite> = arcade_sprites(&ram, 0xE800)
            .into_iter()
            .filter(|s| s.index == 3)
            .collect();
        assert_eq!(s[0].rect, [94, 82, 32, 32]);
        assert_eq!(s[0].x_addr, Some(0xE80F));
        assert_eq!(s[0].picture, 12);
    }

    const MAP: &str = r#"
schema = 22
align = "polled"
source = "made-up"
model = "made_up"

[[field]]
path = "entities.hero[0].x"
ram = [0x0203]

[[field]]
path = "entities.hero[0].y"
ram = [0x0200]

[[field]]
path = "counters.bonus"
ram = [0x0040]

[slots]
count = 2
active = 0x0300
id = 0x0310

[[slots.field]]
path = "x"
ram = [0x0320]

[[slots.kind]]
entity = "rock"
ids = [1]

[[slots.kind]]
entity = "bird"
ids = [2]
"#;

    #[test]
    fn a_byte_is_named_by_the_mapping_s_fields_and_slots() {
        let m = mapping(MAP);
        let mem = |a: u16| -> Option<u8> {
            Some(match a {
                0x0300 | 0x0301 => 1,
                0x0310 => 1,
                0x0311 => 2,
                _ => 0,
            })
        };
        let o = owners(&m, 0x0203, &mem);
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].thing, "entities.hero[0]");
        assert_eq!(o[0].entity.as_deref(), Some("hero"));
        assert_eq!(o[0].field, "x");
        assert_eq!(field_addr(&m, &o[0], "y"), Some(0x0200));
        // Slot 1's x: its ID byte says a bird.
        let o = owners(&m, 0x0321, &mem);
        assert_eq!(o.len(), 1, "{o:?}");
        assert_eq!(o[0].entity.as_deref(), Some("bird"));
        assert_eq!(o[0].slot, Some(1));
        assert_eq!(o[0].note.as_deref(), Some("told by its ID byte, 2"));
        assert_eq!(field_addr(&m, &o[0], "x"), Some(0x0321));
        let fv = field_values(&m, &o[0], &mem);
        assert_eq!(fv[0]["path"], "x");
        assert_eq!(fv[0]["ram"], json!(["$0321"]));
        // A counter is no entity.
        let o = owners(&m, 0x0040, &mem);
        assert_eq!(o[0].entity, None);
        assert_eq!(o[0].thing, "counters.bonus");
        // A byte nobody reads.
        assert!(owners(&m, 0x0500, &mem).is_empty());
        // An empty slot says so.
        let empty = |a: u16| -> Option<u8> { Some(if a == 0x0310 { 1 } else { 0 }) };
        let o = owners(&m, 0x0320, &empty);
        assert_eq!(
            o[0].note.as_deref(),
            Some("its slot reads empty on this frame")
        );
    }

    const BLOCKS: &str = r#"
schema = 22
align = "polled"
source = "made-up"
model = "made_up"

[[field]]
path = "entities.hero[0].x"
ram = [0x0010]

[slots]
count = 2
active = 0x0230
active_stride = 16
free = 255
vacant = "ram[0x0050] == 3"

[[slots.field]]
path = "x"
ram = [0x0233]
stride = 16

[[slots.field]]
path = "y"
ram = [0x0230]
stride = 16

[[slots.kind]]
entity = "barrel"
ids = [0]
"#;

    #[test]
    fn a_sprite_in_a_slot_s_block_is_found_by_place_unless_vacant() {
        let m = mapping(BLOCKS);
        let mem = |a: u16| -> Option<u8> { Some(if a == 0x0240 { 255 } else { 0 }) };
        // $023B is slot 0's third sprite's x: no field reads it.
        assert!(owners(&m, 0x023B, &mem).is_empty());
        let o = owners_by_block(&m, 0x023B, &mem);
        assert_eq!(o.len(), 1, "{o:?}");
        assert_eq!(o[0].entity.as_deref(), Some("barrel"));
        assert_eq!(o[0].slot, Some(0));
        assert!(
            o[0].note.as_deref().unwrap().starts_with(
                "found by place: $023B is in slot 0's block of 16 bytes ($0230 to $023F)"
            ),
            "{o:?}"
        );
        // Slot 1 is free ($0240 is 255): its x answers with a note, its
        // block too.
        assert_eq!(
            owners(&m, 0x0243, &mem)[0].note.as_deref(),
            Some("its slot reads empty on this frame")
        );
        // While the mapping says the slots are vacant, nothing is a barrel.
        let vacant = |a: u16| -> Option<u8> { Some(if a == 0x0050 { 3 } else { 0 }) };
        assert!(owners(&m, 0x0233, &vacant).is_empty());
        assert!(owners_by_block(&m, 0x023B, &vacant).is_empty());
    }

    #[test]
    fn the_code_is_followed_back_to_a_named_byte() {
        // The sprite's x ($E803) was copied from $E18B, which was written
        // from the object's x at $E301, which the mapping names.
        let hop = |addr: u16, from: &[u16]| Hop {
            addr,
            written: true,
            routine: format!("r{addr:04X}"),
            pc: String::new(),
            frame_written: Some(1),
            sentence: String::new(),
            from: from.to_vec(),
            raw: Value::Null,
        };
        let mut calls: Vec<Vec<u16>> = Vec::new();
        let mut slice = |a: &[u16]| -> Result<Vec<Hop>> {
            calls.push(a.to_vec());
            Ok(a.iter()
                .map(|&x| match x {
                    0xE803 => hop(x, &[0xE18B]),
                    0xE18B => hop(x, &[0xE301]),
                    _ => hop(x, &[]),
                })
                .collect())
        };
        let owned = |a: u16| -> Vec<Owner> {
            if a == 0xE301 {
                vec![Owner {
                    thing: "bodies.mug".into(),
                    entity: Some("mug".into()),
                    slot: Some(0),
                    field: "x".into(),
                    path: "bodies.mug[slot 0].x".into(),
                    note: None,
                }]
            } else {
                Vec::new()
            }
        };
        let (hops, found, at) = follow(&[0xE803], &mut slice, &owned).unwrap();
        assert_eq!(at, Some(0xE301));
        assert_eq!(calls, [vec![0xE803], vec![0xE18B]]);
        assert_eq!(hops.len(), 2);
        assert_eq!(found[0].entity.as_deref(), Some("mug"));
        // A sprite byte the mapping names is sliced once and owned at once.
        let owned_self = |a: u16| -> Vec<Owner> {
            if a == 0xE803 {
                owned(0xE301)
            } else {
                Vec::new()
            }
        };
        let (hops, found, _) = follow(
            &[0xE803],
            &mut |a: &[u16]| Ok(a.iter().map(|&x| hop(x, &[])).collect()),
            &owned_self,
        )
        .unwrap();
        assert_eq!(hops.len(), 1);
        assert_eq!(found.len(), 1);
        // Nothing named within the hops: nothing found, said by the caller.
        let (hops, found, _) = follow(
            &[0xE803],
            &mut |a: &[u16]| Ok(a.iter().map(|&x| hop(x, &[x + 1])).collect()),
            &|_| Vec::new(),
        )
        .unwrap();
        assert_eq!(hops.len(), HOPS);
        assert!(found.is_empty());
    }

    #[test]
    fn a_slice_reads_as_a_hop_on_either_platform() {
        let nes = json!({"written": true, "writer": {"routine": "$EAE1", "pc": "$EAE4"}, "frame_written": 9,
            "reads": [{"addr": "$0203", "how": "value", "value": 3}, {"addr": "$C000", "how": "value"}, {"addr": "$0005", "how": "pointer"}],
            "feeds": [{"reads": [{"addr": "$0046", "how": "value"}]}], "sentence": "s"});
        let ram = |a: u16| a < 0x800;
        let h = hop_of(0x020B, &nes, &ram);
        assert_eq!(h.routine, "$EAE1");
        assert_eq!(h.from, [0x0203, 0x0046]);
        let z80 = json!({"written": true, "writer": {"routine_name": "$49B9", "pc": 18882}, "reads": [{"addr": 57739, "how": "value"}]});
        let h = hop_of(0xE803, &z80, &|a| a >= 0xE000);
        assert_eq!(h.pc, "$49C2");
        assert_eq!(h.from, [0xE18B]);
    }

    #[test]
    fn the_model_s_things_and_pixels_in_the_spot() {
        let things = vec![
            ("barrel".to_owned(), [10, 10, 16, 16]),
            ("the score".to_owned(), [100, 0, 48, 8]),
            (String::new(), [0, 0, 256, 240]),
        ];
        let t = things_under(
            &things,
            Spot {
                x: 20,
                y: 20,
                w: 1,
                h: 1,
            },
        );
        assert_eq!(t, vec![json!({"name": "barrel", "rect": [10, 10, 16, 16]})]);
        let frame = |c: u8, sprite: bool| Frame {
            w: 4,
            h: 4,
            rgb: vec![[c, 0, 0]; 16],
            sprite: vec![sprite; 16],
            visible: [0, 0, 4, 4],
            things: Vec::new(),
        };
        let p = pixels_in(
            &frame(1, true),
            &frame(2, false),
            Spot {
                x: 2,
                y: 2,
                w: 5,
                h: 5,
            },
        );
        assert_eq!(
            p,
            json!({"compared": 4, "differ": 4, "original_sprite": 4, "model_sprite": 0})
        );
    }

    #[test]
    fn the_words_say_what_is_known_and_what_is_not() {
        let v = json!({
            "clip": "c.001", "frame": 10, "original_frame": 110, "side": "original",
            "spot": {"x": 5, "y": 6, "w": 1, "h": 1},
            "drawn": {"sprites": [{"index": 4, "picture": 7, "palette": 1, "rect": [0, 0, 8, 8]}], "tile": {"cell": [0, 0], "picture": 36, "name": "girder"}, "hud": []},
            "object": {"label": "hero", "slot": null, "from": "the mapping reads $0203", "fields": [{"path": "entities.hero[0].x", "original": "5"}]},
            "drawn_by": {"written": true, "routine": "draw", "byte": "$0213", "pc": "$C000", "frame_written": 10, "sentence": "draw wrote $0213 from $0203"},
            "moved_by": [{"written": true, "routine": "move", "byte": "$0203", "pc": "$C100", "frame_written": 10, "slice": {"reads": [{"name": "$0016", "how": "value"}]}}],
            "model": {"model_frame": 106, "things": [{"name": "hero"}], "pixels": {"differ": 0, "compared": 1}, "measures": [{"path": "entities.hero[0].x", "agree": false, "original": "5", "model": "6"}]},
            "unknown": ["who drew the background tile: the port"],
            "took_ms": {"total": 1500, "original": 100, "code": 900, "model": 500},
        });
        let w = words(&v);
        assert!(w.contains("sprite 4 (picture 7, palette 1)"), "{w}");
        assert!(w.contains("picture 36, a girder"), "{w}");
        assert!(w.contains("It is the hero (the mapping reads $0203); the original's values: entities.hero[0].x 5"), "{w}");
        assert!(w.contains("Drawn by draw"), "{w}");
        assert!(
            w.contains("Moved by move") && w.contains("$0016 (value)"),
            "{w}"
        );
        assert!(w.contains("its picture draws hero there"), "{w}");
        assert!(
            w.contains("They disagree on this frame: entities.hero[0].x original 5 model 6"),
            "{w}"
        );
        assert!(
            w.contains("Not determined: who drew the background tile: the port."),
            "{w}"
        );
        assert!(w.contains("Took 1.5 s"), "{w}");
    }
}
