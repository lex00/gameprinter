//! An arcade model drawn as its board draws its picture
//! (docs/formats/board-art.md): a level's `present.board` gives the
//! background tile layer (a whole screen of the board's tiles) and the
//! board's 64 colours, and each body whose entity has a `board_sprite` is
//! an entry of the board's sprite RAM, its picture and flags read from the
//! body's words or fixed by the model, placed where the board draws an
//! entry whose bytes are the body's position. The picture is then made by
//! the board's own drawing, `gp_arcade::video::draw`, with the tiles and
//! sprites the player's lift holds: its layering, priorities, colour banks
//! and placement are the board's, and nothing here decodes a pixel again.
//!
//! Everything comes from the model and the lifted folder; nothing is read
//! from the original's RAM. A model that names none of this draws as
//! before (by tile kind on an arcade lift).

use std::collections::BTreeMap;
use std::path::Path;

use gp_arcade::video::{self, Gfx};
use gp_assets::arcade::{ArcadeAsset, ArcadeMap, Format};
use gp_model::Model;
use gp_model::model::{AssetKind, BoardByte};
use gp_sim::{Spec, World};

/// The sprite RAM's entries on the 91490 board's sprite board (0x200
/// bytes of four).
pub const ENTRIES: usize = 128;

/// The board's picture's size: [`Picture::pens`] is this wide and high.
pub const WIDTH: usize = video::WIDTH;
pub const HEIGHT: usize = video::HEIGHT;

/// A model's board art, read once from its lifted folder.
#[derive(Clone)]
pub struct BoardArt {
    gfx: Gfx,
    /// By the spec's level index: its look, or None for a level with no
    /// `present.board` (drawn as before).
    levels: Vec<Option<LevelBoard>>,
    /// By the spec's kind index: its board sprite, or None.
    kinds: Vec<Option<KindSprite>>,
    /// The spec's kind names, by index.
    kind_names: Vec<String>,
    /// The spec's counter names, by index, as a theme's `counter_off`
    /// names them.
    counter_names: Vec<String>,
    /// Whether the model's positions are the board's own units (its state
    /// mapping's `board_units`): top left, y down, each unit
    /// [`video::UNIT`] screen pixels, so a body's sprite entry holds the
    /// bytes that draw it there ([`video::sprite_bytes`]). Otherwise they
    /// are the sprite RAM's bytes as they are.
    board_units: bool,
}

#[derive(Clone)]
struct LevelBoard {
    /// The level's own look.
    base: Look,
    /// Its other looks (`looks`), each with the counter tests that choose
    /// it: the first whose tests all hold is drawn.
    looks: Vec<(Vec<Test>, Look)>,
    /// Counters drawn as digits (`counters`).
    numbers: Vec<Number>,
    /// Counters drawn as one tile repeated (`repeats`).
    repeats: Vec<Repeat>,
    /// Strings drawn while counters say so (`texts`).
    texts: Vec<Text>,
}

/// A screen and its colours.
#[derive(Clone)]
struct Look {
    /// The `board-screen` slot its screen is, None for a look with none.
    screen: Option<String>,
    /// For a screen several of the level's looks show (a title lit letter
    /// by letter by its colours): this look's own name, the slot and its
    /// place among them from 1 (`screen_title-3`), the base look first.
    stage: Option<String>,
    /// Video RAM as the board reads it: 960 cells of two bytes, low first.
    videoram: Vec<u8>,
    /// The 64 colours as 0xRRGGBB.
    palette: [u32; 64],
    /// No sprite drawn (`sprites_off`).
    sprites_off: bool,
}

/// A counter test (a `Guard`), its counter by its index in the spec.
#[derive(Clone, Copy)]
struct Test {
    counter: usize,
    cmp: gp_model::model::Cmp,
    value: i64,
}

/// A comparison as a rule writes it (`==`), as the model's `when` reads it.
fn cmp_of(s: &str) -> Option<gp_model::model::Cmp> {
    use gp_model::model::Cmp;
    Some(match s {
        "<" => Cmp::Lt,
        "<=" => Cmp::Le,
        "==" => Cmp::Eq,
        "!=" => Cmp::Ne,
        ">=" => Cmp::Ge,
        ">" => Cmp::Gt,
        _ => return None,
    })
}

impl Test {
    fn all(tests: &[Test], w: &World) -> bool {
        tests.iter().all(|t| {
            w.counters
                .get(t.counter)
                .is_some_and(|v| t.cmp.holds(*v, t.value))
        })
    }
}

/// A counter drawn as digits: the cell of its first digit, each digit's
/// tile code and the cells' high byte, lifted.
#[derive(Clone)]
struct Number {
    counter: usize,
    /// The counter's name, as a theme names its digits.
    name: String,
    plus: i64,
    cell: usize,
    digits: usize,
    codes: [u8; 10],
    high: u8,
    zeros: bool,
    when: Vec<Test>,
}

impl Number {
    /// The name a theme gives the cell this counter draws for digit `d`:
    /// [`counter_name`], except that a one-digit counter holding the
    /// character code of a letter it draws through that letter's tile
    /// (a high score's initials: the counter is 65 for A, its `plus`
    /// takes it to the table's first cell, and the tile drawn is A's) is
    /// named by the letter, as a text's letters are ([`letter_name`]), so
    /// a theme's letter pictures draw it too.
    fn digit_name(&self, d: u8) -> String {
        let code = self.codes[usize::from(d) % 10];
        let word = [code, self.high];
        if self.digits == 1
            && i64::from(code) == i64::from(d) - self.plus
            && (word[1] & 3) == 0
            && code.is_ascii_uppercase()
        {
            return letter_name(word);
        }
        counter_name(&self.name, Some(d))
    }
}

/// A string drawn while counters say so: each character's cell and word.
#[derive(Clone)]
struct Text {
    /// Its slot, as a theme's `text_say` names it.
    name: String,
    /// Each character's cell and its two bytes.
    cells: Vec<(usize, [u8; 2])>,
    when: Vec<Test>,
}

/// A counter drawn as a tile repeated: the cells it takes, in order.
#[derive(Clone)]
struct Repeat {
    counter: usize,
    /// The counter's name, as a theme names its tile.
    name: String,
    plus: i64,
    cells: Vec<usize>,
    word: [u8; 2],
    when: Vec<Test>,
}

#[derive(Clone, Copy)]
enum Byte {
    /// A byte of a body's `ext` word: (word, byte).
    Word(usize, u32),
    Number(u8),
}

impl Byte {
    fn of(b: &BoardByte) -> Self {
        match (b.word, b.number) {
            (Some(w), _) => Byte::Word(w as usize, b.byte),
            (None, n) => Byte::Number(n.unwrap_or(0) as u8),
        }
    }

    fn read(self, body: &gp_sim::world::Body) -> u8 {
        match self {
            Byte::Word(w, k) => body
                .ext
                .get(w)
                .map_or(0, |v| ((*v >> (8 * k.min(7))) & 0xff) as u8),
            Byte::Number(n) => n,
        }
    }
}

#[derive(Clone)]
struct KindSprite {
    picture: Byte,
    flags: Option<Byte>,
    flip_when_facing_left: bool,
    /// Its other sprites (`board_sprite.extra`).
    extra: Vec<ExtraSprite>,
    /// The frames none of them is drawn (`board_sprite.hidden`): a byte
    /// of the body's words, and the bit of it that hides them (None: any).
    hidden: Option<(Byte, Option<u32>)>,
    /// The frames after which the game empties the object
    /// (`board_sprite.gone_next`): read as `hidden` on a frame drawn from
    /// the frame before's bodies, ignored on one drawn from its own
    /// ([`BoardArt::own_frame`]). As `hidden`, (word, byte) and its bit.
    gone_next: Option<(usize, u32, Option<u32>)>,
}

/// One of a body's other sprites: its place from the body's, its picture
/// and flags.
#[derive(Clone, Copy)]
struct ExtraSprite {
    offset: [i32; 2],
    picture: Byte,
    flags: Option<Byte>,
    flip_when_facing_left: bool,
}

impl KindSprite {
    /// Whether the board leaves the body's sprites out this frame.
    fn hides(&self, body: &gp_sim::world::Body) -> bool {
        let set = |byte: Byte, bit: Option<u32>| {
            let v = byte.read(body);
            bit.map_or(v != 0, |k| v >> k.min(7) & 1 == 1)
        };
        self.hidden.is_some_and(|(byte, bit)| set(byte, bit))
            || self
                .gone_next
                .is_some_and(|(w, k, bit)| set(Byte::Word(w, k), bit))
    }

    /// The body with its `gone_next` cleared: as a frame drawn from its own
    /// bodies shows it. A byte with no bit is cleared whole.
    fn clear_gone_next(&self, body: &mut gp_sim::world::Body) -> bool {
        let Some((w, k, bit)) = self.gone_next else {
            return false;
        };
        let Some(v) = body.ext.get_mut(w) else {
            return false;
        };
        let mask = bit.map_or(0xff, |b| 1 << b.min(7)) << (8 * k.min(7));
        let had = *v & mask != 0;
        *v &= !mask;
        had
    }

    /// The entry's position bytes for a body at (x, y) in its own units.
    fn bytes(board_units: bool, x: i64, y: i64) -> (i64, i64) {
        if board_units {
            let (x, y) = video::sprite_bytes(x as i32, y as i32);
            (i64::from(x), i64::from(y))
        } else {
            (x.rem_euclid(256), y.rem_euclid(256))
        }
    }
}

impl LevelBoard {
    /// The look the world's counters choose: the first of `looks` whose
    /// tests all hold, else the level's own.
    fn look(&self, w: &World) -> &Look {
        self.looks
            .iter()
            .find(|(t, _)| Test::all(t, w))
            .map_or(&self.base, |(_, l)| l)
    }

    /// The look's video RAM with the counters drawn into it: the screen's
    /// own cells where a counter draws nothing.
    #[cfg(test)]
    fn cells<'a>(&self, videoram: &'a [u8], w: &World) -> std::borrow::Cow<'a, [u8]> {
        self.cells_with(videoram, w, None).0
    }

    /// [`LevelBoard::cells`], and each cell a counter drew, in order: those
    /// `omit` names (by that order) left as the screen has them.
    fn cells_with<'a>(
        &self,
        videoram: &'a [u8],
        w: &World,
        omit: Option<&[bool]>,
    ) -> (std::borrow::Cow<'a, [u8]>, Vec<CellDrawn>) {
        if self.numbers.is_empty() && self.repeats.is_empty() && self.texts.is_empty() {
            return (std::borrow::Cow::Borrowed(videoram), Vec::new());
        }
        let mut v = videoram.to_vec();
        // Every cell written this frame, in the order written: a counter's
        // digits, a repeat's tiles, a text's letters.
        let mut puts: Vec<(usize, [u8; 2], CellDrawn)> = Vec::new();
        let mut put = |cell: usize,
                       word: [u8; 2],
                       name: String,
                       text: Option<(usize, usize)>,
                       counter: Option<usize>| {
            puts.push((
                cell,
                word,
                CellDrawn {
                    name,
                    cell,
                    text,
                    counter,
                },
            ));
        };
        let value = |counter: usize, plus: i64| {
            w.counters
                .get(counter)
                .copied()
                .unwrap_or(0)
                .saturating_add(plus)
                .max(0)
        };
        for n in self.numbers.iter().filter(|n| Test::all(&n.when, w)) {
            let mut x = value(n.counter, n.plus) as u64;
            // The last digits, least first.
            let mut digits = vec![0u8; n.digits];
            for d in digits.iter_mut().rev() {
                *d = (x % 10) as u8;
                x /= 10;
            }
            let first = if n.zeros {
                0
            } else {
                digits[..n.digits - 1]
                    .iter()
                    .position(|d| *d != 0)
                    .unwrap_or(n.digits - 1)
            };
            for (k, d) in digits.iter().enumerate().skip(first) {
                put(
                    n.cell + k,
                    [n.codes[*d as usize], n.high],
                    n.digit_name(*d),
                    None,
                    Some(n.counter),
                );
            }
        }
        for r in self.repeats.iter().filter(|r| Test::all(&r.when, w)) {
            let count = value(r.counter, r.plus) as usize;
            for &cell in r.cells.iter().take(count) {
                put(
                    cell,
                    r.word,
                    counter_name(&r.name, None),
                    None,
                    Some(r.counter),
                );
            }
        }
        for (ti, t) in self
            .texts
            .iter()
            .enumerate()
            .filter(|(_, t)| Test::all(&t.when, w))
        {
            for (ci, &(cell, word)) in t.cells.iter().enumerate() {
                put(cell, word, letter_name(word), Some((ti, ci)), None);
            }
        }
        // A cell written again shows its last word, as the board's video
        // RAM does: only that write is drawn and named, in the order of
        // those last writes. `omit` goes by that order, so it is asked of
        // the last write alone: an earlier write to the cell is never laid
        // in the video RAM, where a later write left to the theme would
        // otherwise leave the earlier one's word showing.
        let mut last = std::collections::HashMap::new();
        for (k, (cell, ..)) in puts.iter().enumerate() {
            last.insert(*cell, k);
        }
        let mut drawn = Vec::new();
        for (k, (cell, word, d)) in puts.into_iter().enumerate() {
            if last.get(&cell) != Some(&k) {
                continue;
            }
            let left =
                cell < 960 && omit.is_some_and(|o| o.get(drawn.len()).copied().unwrap_or(false));
            if !left && let Some(c) = v.get_mut(cell * 2..cell * 2 + 2) {
                c.copy_from_slice(&word);
            }
            if cell < 960 {
                drawn.push(d);
            }
        }
        (std::borrow::Cow::Owned(v), drawn)
    }
}

/// A counter's cell drawn this frame: the name a theme gives its picture
/// ([`counter_name`]) and the cell (row times 32 plus column).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellDrawn {
    pub name: String,
    pub cell: usize,
    /// For a text's letter: the text (its place in the level's texts) and
    /// the letter's place in it.
    pub text: Option<(usize, usize)>,
    /// For a counter's digit or tile: the counter (the spec's index).
    pub counter: Option<usize>,
}

/// The name a theme gives a sprite of the board: its entity and its
/// picture code (0 to 511) in hex, `sprite-mug-4d`.
#[must_use]
pub fn sprite_name(entity: &str, code: u16) -> String {
    format!("sprite-{entity}-{code:02x}")
}

/// The name a theme gives a counter's cell: `counter-score-7` for a digit
/// of a counter drawn as digits, `counter-lives` for the tile a counter
/// drawn as a tile repeated repeats.
#[must_use]
pub fn counter_name(counter: &str, digit: Option<u8>) -> String {
    match digit {
        Some(d) => format!("counter-{counter}-{d}"),
        None => format!("counter-{counter}"),
    }
}

/// The name a theme gives a text's letter: the tile a text draws, by the
/// character it stands for when it is a letter or a digit (`letter-A`,
/// `letter-7`, the text's byte being the tile's code), else by the tile's
/// code in hex (`letter-x2e`). One picture a letter redraws every text the
/// game writes with it.
#[must_use]
pub fn letter_name(word: [u8; 2]) -> String {
    let c = char::from(word[0]);
    if (word[1] & 3) == 0 && (c.is_ascii_uppercase() || c.is_ascii_digit()) {
        format!("letter-{c}")
    } else {
        format!(
            "letter-x{:03x}",
            u16::from(word[0]) | (u16::from(word[1] & 3) << 8)
        )
    }
}

/// A screen's, a sprite's and a counter cell's size in the board's
/// picture's pixels: what a theme's sizes and places are in.
pub const SCREEN_SIZE: [u32; 2] = [WIDTH as u32, HEIGHT as u32];
pub const SPRITE_SIZE: [u32; 2] = [32, 32];
pub const CELL_SIZE: [u32; 2] = [16, 16];

/// One body the board draws: its index, its kind (the spec's index), its
/// rectangle on the screen (left, top, width, height), the picture code
/// its sprite entry draws (0 to 511: byte 2, and bit 3 of byte 1) and the
/// entry's flags (byte 1: colour bank, flips), and the entry of sprite
/// RAM it takes (0 to 127: a later entry is drawn over an earlier one).
/// `extra` is which of the body's other sprites it is
/// (`board_sprite.extra`), None for the body's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drawn {
    pub body: usize,
    pub extra: Option<usize>,
    pub kind: usize,
    pub entry: usize,
    pub rect: [i64; 4],
    pub code: u16,
    pub flags: u8,
}

/// The board's picture of a frame: each pixel's pen (0 to 63, with
/// `gp_arcade::video::SPRITE_BIT` where a sprite drew it, or
/// [`TRANSPARENT`] where a theme draws the screen), the 64 colours, the
/// bodies drawn, the look's screen slot and the counters' cells drawn.
pub struct Picture {
    pub pens: Vec<u8>,
    pub palette: [u32; 64],
    pub drawn: Vec<Drawn>,
    pub screen: Option<String>,
    /// The look's own name for its screen when several looks show it
    /// (`screen_title-3`), which a theme can draw in place of the screen's.
    pub stage: Option<String>,
    pub cells: Vec<CellDrawn>,
    /// Pixels a theme's `#+ recolor` draws in its own colours: each by its
    /// place in `pens` and its colour, None for nothing (what is under it
    /// shows).
    pub recolor: Vec<(u32, Option<[u8; 3]>)>,
}

/// A pen in [`Picture::pens`] that draws nothing: the screen's pixels
/// where a theme draws the screen itself (no board pen is 0x40).
pub const TRANSPARENT: u8 = 0x40;

/// What a theme draws itself, which the board then leaves out of its
/// picture: bodies by index, the screen, the counters' cells by their
/// order in [`Picture::cells`].
#[derive(Debug, Clone, Default)]
pub struct Omit {
    pub bodies: Vec<bool>,
    pub screen: bool,
    pub cells: Vec<bool>,
    /// Single sprites of a body left out, the rest of it drawn: `(body,
    /// None)` its own sprite, `(body, Some(n))` its extra sprite `n` (a mug
    /// a customer holds, which a theme draws while the board draws the
    /// customer).
    pub parts: Vec<(usize, Option<usize>)>,
}

/// Whether the model names any board art: a level's `present.board` or an
/// entity's `board_sprite`.
pub fn names_board(model: &Model) -> bool {
    model
        .entities
        .values()
        .any(|e| e.core.board_sprite.is_some())
        || model
            .levels
            .values()
            .any(|l| l.core.present.as_ref().is_some_and(|p| p.board.is_some()))
}

/// The lifted arcade asset a slot of `kind` names, from the map.
fn slot_asset<'a>(
    model: &Model,
    map: &'a ArcadeMap,
    slot: &str,
    kind: AssetKind,
) -> Result<&'a ArcadeAsset, String> {
    let s = model
        .assets
        .get(slot)
        .ok_or(format!("the model has no asset slot `{slot}`"))?;
    if s.kind != kind {
        return Err(format!("asset slot `{slot}` isn't of kind {}", kind.name()));
    }
    let from = s.from.as_ref().ok_or(format!(
        "asset slot `{slot}` has no `from`, so nothing fills it"
    ))?;
    if from.source != map.source {
        return Err(format!(
            "asset slot `{slot}` is filled from `{}`, but the lifted assets are `{}`'s",
            from.source, map.source
        ));
    }
    let a = map
        .assets
        .iter()
        .find(|a| a.name == from.asset)
        .ok_or(format!(
            "asset slot `{slot}` names `{}`, which the asset map doesn't have",
            from.asset
        ))?;
    if !kind.takes(a.format.name()) {
        return Err(format!(
            "asset slot `{slot}` names `{}`, which is {}, not {}",
            from.asset,
            a.format.name(),
            kind.map_format()
        ));
    }
    Ok(a)
}

/// A `byte-rows` table's lifted file read back: its rows.
fn read_rows(text: &str) -> Result<Vec<Vec<u8>>, String> {
    let t: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
    let bad = || "`rows` isn't a list of rows of bytes".to_owned();
    t.get("rows")
        .and_then(toml::Value::as_array)
        .ok_or_else(bad)?
        .iter()
        .map(|r| {
            r.as_array()
                .ok_or_else(bad)?
                .iter()
                .map(|b| {
                    b.as_integer()
                        .and_then(|b| u8::try_from(b).ok())
                        .ok_or_else(bad)
                })
                .collect()
        })
        .collect()
}

/// Keeps the one slot of a board region the model names: a second
/// differing one is refused.
fn one(have: &mut Option<String>, slot: &str, what: &str) -> Result<(), String> {
    match have {
        Some(s) if s != slot => Err(format!(
            "the board has one {what} region, and the model names two {what} slots, `{s}` and `{slot}`"
        )),
        _ => {
            *have = Some(slot.to_owned());
            Ok(())
        }
    }
}

/// Reads a lifted file by its name in the lifted folder: its bytes, or
/// why not. [`BoardArt::load`] reads the folder on disk; the page's player
/// hands over the files the lift stored in the browser
/// ([`BoardArt::load_with`]).
pub type Read<'a> = &'a dyn Fn(&str) -> Result<Vec<u8>, String>;

/// An `mcr-tiles` or `mcr-sprites` asset's pictures, by their index in
/// the board's region (those before `first` blank), at least `at_least`
/// of them.
fn pictures<const N: usize>(
    read: Read,
    a: &ArcadeAsset,
    side: u32,
    at_least: usize,
) -> Result<Vec<[u8; N]>, String> {
    let file = a.file();
    let bytes = read(&file)?;
    let (w, _) = a.image_size();
    let px =
        gp_assets::chr::read_png(&bytes, a.image_size()).map_err(|e| format!("{file}: {e}"))?;
    let first = a.first as usize;
    let mut out = vec![[0u8; N]; (first + a.count as usize).max(at_least)];
    for k in 0..a.count {
        let (cx, cy) = ((k % a.per_row) * side, (k / a.per_row) * side);
        let pic = &mut out[first + k as usize];
        for y in 0..side {
            for x in 0..side {
                pic[(y * side + x) as usize] = px[((cy + y) * w + cx + x) as usize] & 0x0f;
            }
        }
    }
    Ok(out)
}

impl BoardArt {
    /// The board art of `model` from the lifted folder `dir`, or None
    /// when the model names none or the folder isn't an arcade lift.
    pub fn load(model: &Model, spec: &Spec, dir: &Path) -> Result<Option<Self>, String> {
        Self::load_with(model, spec, &|name: &str| {
            let f = dir.join(name);
            std::fs::read(&f).map_err(|e| format!("{}: {e}", f.display()))
        })
    }

    /// The lifted files the board art of `model` reads, by their names in
    /// the lifted folder `dir` (the map first): what a player that has no
    /// folder (the page's) is handed for [`BoardArt::load_with`]. Empty
    /// when the model names no board art.
    pub fn files(model: &Model, spec: &Spec, dir: &Path) -> Result<Vec<String>, String> {
        let names = std::cell::RefCell::new(Vec::<String>::new());
        Self::load_with(model, spec, &|name: &str| {
            let f = dir.join(name);
            let bytes = std::fs::read(&f).map_err(|e| format!("{}: {e}", f.display()))?;
            let mut n = names.borrow_mut();
            if !n.iter().any(|x| x == name) {
                n.push(name.to_owned());
            }
            Ok(bytes)
        })?;
        Ok(names.into_inner())
    }

    /// [`BoardArt::load`] with the lifted files read by `read`, by their
    /// names in the lifted folder.
    pub fn load_with(model: &Model, spec: &Spec, read_file: Read) -> Result<Option<Self>, String> {
        if !names_board(model) {
            return Ok(None);
        }
        let map_file = gp_interp::lifted::MAP_FILE;
        let text =
            String::from_utf8(read_file(map_file)?).map_err(|e| format!("{map_file}: {e}"))?;
        if !gp_assets::arcade::is_arcade(&text) {
            return Err(format!(
                "{map_file}: board art is drawn from an arcade lift, and this one is an NES map"
            ));
        }
        let map = ArcadeMap::parse(&text).map_err(|e| format!("{map_file}: {e}"))?;
        let read = |a: &ArcadeAsset| {
            let file = a.file();
            String::from_utf8(read_file(&file)?).map_err(|e| format!("{file}: {e}"))
        };

        // One tiles slot and one sprites slot draw everything: the board
        // has one of each region.
        let (mut ts, mut ss) = (None::<String>, None::<String>);
        let mut levels = Vec::with_capacity(spec.levels.len());
        let mut screens: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let mut rows_of: BTreeMap<String, Vec<Vec<u8>>> = BTreeMap::new();
        // A `rows` slot's byte, read once a slot.
        let mut row_byte = |slot: &str, row: u32, column: u32| -> Result<u8, String> {
            if !rows_of.contains_key(slot) {
                let a = slot_asset(model, &map, slot, AssetKind::Rows)?;
                let rows = read_rows(&read(a)?).map_err(|e| format!("{}: {e}", a.file()))?;
                rows_of.insert(slot.to_owned(), rows);
            }
            rows_of[slot]
                .get(row as usize)
                .and_then(|r| r.get(column as usize))
                .copied()
                .ok_or_else(|| format!("`{slot}` has no byte {column} in row {row}"))
        };
        let mut screen_of = |sc: &str| -> Result<Vec<u8>, String> {
            if let Some(v) = screens.get(sc) {
                return Ok(v.clone());
            }
            let a = slot_asset(model, &map, sc, AssetKind::BoardScreen)?;
            let cells = gp_assets::arcade::read_screen(&read(a)?)
                .map_err(|e| format!("{}: {e}", a.file()))?;
            let mut v: Vec<u8> = cells.iter().flat_map(|c| c.to_le_bytes()).collect();
            v.resize(0x800, 0);
            screens.insert(sc.to_owned(), v.clone());
            Ok(v)
        };
        let counter = |name: &str| {
            spec.counters
                .index_of(name)
                .ok_or_else(|| format!("no counter named `{name}`"))
        };
        let tests = |when: &[gp_model::model::Guard]| -> Result<Vec<Test>, String> {
            when.iter()
                .map(|g| {
                    Ok(Test {
                        counter: counter(&g.counter)?,
                        cmp: g.cmp,
                        value: g.value,
                    })
                })
                .collect()
        };
        for l in &spec.levels {
            let look = model
                .levels
                .get(&l.name)
                .and_then(|m| m.core.present.as_ref())
                .and_then(|p| p.board.as_ref());
            let Some(look) = look else {
                levels.push(None);
                continue;
            };
            let at = |e: String| format!("levels.{}.present.board: {e}", l.name);
            if let Some(t) = &look.tiles {
                one(&mut ts, t, "tiles").map_err(at)?;
            }
            // A look: the palette and the writes over it, in order, and
            // the screen.
            let mut make = |palette: &str,
                            writes: &[&[gp_model::model::PaletteWrite]],
                            screen: Option<&str>|
             -> Result<Look, String> {
                let pal = slot_asset(model, &map, palette, AssetKind::BoardPalette)?;
                let mut nine: Vec<u16> = gp_assets::arcade::read_palette(&read(pal)?)
                    .map_err(|e| format!("{}: {e}", pal.file()))?;
                for (i, pw) in writes.iter().flat_map(|w| w.iter()).enumerate() {
                    let v = row_byte(&pw.slot, pw.row, pw.column)
                        .map_err(|e| format!("palette_writes[{i}]: {e}"))?;
                    // mcr_paletteram9_w: the byte, and the address's low
                    // bit as the ninth.
                    if let Some(c) = nine.get_mut(pw.at as usize / 2) {
                        *c = u16::from(v) | (u16::from(pw.at as u8 & 1) << 8);
                    }
                }
                let mut colors = [0u32; 64];
                for (p, c) in colors.iter_mut().zip(&nine) {
                    *p = video::color_9bit(*c);
                }
                let videoram = match screen {
                    Some(sc) => screen_of(sc)?,
                    None => vec![0; 0x800],
                };
                Ok(Look {
                    screen: screen.map(str::to_owned),
                    stage: None,
                    videoram,
                    palette: colors,
                    sprites_off: false,
                })
            };
            let base = make(
                &look.palette,
                &[&look.palette_writes],
                look.screen.as_deref(),
            )
            .map_err(at)?;
            let mut looks = Vec::with_capacity(look.looks.len());
            for (i, o) in look.looks.iter().enumerate() {
                let at_o = |e: String| at(format!("looks[{i}]: {e}"));
                // A look with a palette of its own loads it afresh, as a
                // program that loads its colours again: only its own
                // writes go over it.
                let writes: &[&[gp_model::model::PaletteWrite]] = match &o.palette {
                    Some(_) => &[&o.palette_writes],
                    None => &[&look.palette_writes, &o.palette_writes],
                };
                if o.clear && o.screen.is_some() {
                    return Err(at_o(
                        "a look that clears the tile layer names no screen".into(),
                    ));
                }
                // A look that clears the tile layer draws no screen: tile
                // 0 everywhere, as the level with none.
                let screen = if o.clear {
                    None
                } else {
                    o.screen.as_deref().or(look.screen.as_deref())
                };
                let mut made = make(
                    o.palette.as_deref().unwrap_or(&look.palette),
                    writes,
                    screen,
                )
                .map_err(at_o)?;
                made.sprites_off = o.sprites_off;
                looks.push((tests(&o.when).map_err(at_o)?, made));
            }
            let mut numbers = Vec::with_capacity(look.counters.len());
            for (i, n) in look.counters.iter().enumerate() {
                let at_n = |e: String| at(format!("counters[{i}]: {e}"));
                let mut codes = [0u8; 10];
                for (d, c) in codes.iter_mut().enumerate() {
                    *c = if n.table {
                        row_byte(&n.tile.slot, n.tile.row, n.tile.column + d as u32)
                            .map_err(at_n)?
                    } else {
                        row_byte(&n.tile.slot, n.tile.row, n.tile.column)
                            .map_err(at_n)?
                            .wrapping_add(d as u8)
                    };
                }
                numbers.push(Number {
                    counter: counter(&n.counter).map_err(at_n)?,
                    name: n.counter.clone(),
                    plus: n.plus,
                    cell: (n.at[1] * 32 + n.at[0]) as usize,
                    digits: n.digits.clamp(1, 10) as usize,
                    codes,
                    high: row_byte(&n.high.slot, n.high.row, n.high.column).map_err(at_n)?,
                    zeros: n.zeros,
                    when: tests(&n.when).map_err(at_n)?,
                });
            }
            let mut repeats = Vec::with_capacity(look.repeats.len());
            for (i, r) in look.repeats.iter().enumerate() {
                let at_r = |e: String| at(format!("repeats[{i}]: {e}"));
                let [sx, sy] = r.step.unwrap_or([1, 0]);
                let cells = (0..i64::from(r.most))
                    .map(|k| {
                        (
                            i64::from(r.at[0]) + i64::from(sx) * k,
                            i64::from(r.at[1]) + i64::from(sy) * k,
                        )
                    })
                    .filter(|(x, y)| (0..32).contains(x) && (0..30).contains(y))
                    .map(|(x, y)| (y * 32 + x) as usize)
                    .collect();
                repeats.push(Repeat {
                    counter: counter(&r.counter).map_err(at_r)?,
                    name: r.counter.clone(),
                    plus: r.plus,
                    cells,
                    word: [
                        row_byte(&r.tile.slot, r.tile.row, r.tile.column).map_err(at_r)?,
                        row_byte(&r.high.slot, r.high.row, r.high.column).map_err(at_r)?,
                    ],
                    when: tests(&r.when).map_err(at_r)?,
                });
            }
            let mut texts = Vec::with_capacity(look.texts.len());
            for (i, t) in look.texts.iter().enumerate() {
                let at_t = |e: String| at(format!("texts[{i}]: {e}"));
                let high = row_byte(&t.high.slot, t.high.row, t.high.column).map_err(at_t)?;
                let mut cells = Vec::new();
                for k in 0..32u32 {
                    let Ok(b) = row_byte(&t.text.slot, t.text.row, t.text.column + k) else {
                        break;
                    };
                    if b == 0 {
                        break;
                    }
                    let (x, y) = (t.at[0] + k, t.at[1]);
                    if x < 32 && y < 30 {
                        cells.push(((y * 32 + x) as usize, [if b == 0x20 { 0 } else { b }, high]));
                    }
                }
                texts.push(Text {
                    name: t.text.slot.clone(),
                    cells,
                    when: tests(&t.when).map_err(at_t)?,
                });
            }
            // A screen several looks show: each look's own name for it.
            let mut base = base;
            {
                let mut all: Vec<&mut Look> = std::iter::once(&mut base)
                    .chain(looks.iter_mut().map(|(_, l)| l))
                    .collect();
                let mut seen: BTreeMap<String, usize> = BTreeMap::new();
                for l in &all {
                    if let Some(sc) = &l.screen {
                        *seen.entry(sc.clone()).or_default() += 1;
                    }
                }
                let mut at: BTreeMap<String, usize> = BTreeMap::new();
                for l in &mut all {
                    if let Some(sc) = l.screen.clone()
                        && seen.get(&sc).copied().unwrap_or(0) > 1
                    {
                        let k = at.entry(sc.clone()).or_default();
                        *k += 1;
                        l.stage = Some(format!("{sc}-{k}"));
                    }
                }
            }
            levels.push(Some(LevelBoard {
                base,
                looks,
                numbers,
                repeats,
                texts,
            }));
        }
        let kinds = spec
            .kinds
            .iter()
            .map(|k| {
                let Some(b) = model
                    .entities
                    .get(&k.name)
                    .and_then(|e| e.core.board_sprite.as_ref())
                else {
                    return Ok(None);
                };
                one(&mut ss, &b.slot, "sprites")
                    .map_err(|e| format!("entities.{}.board_sprite: {e}", k.name))?;
                Ok(Some(KindSprite {
                    picture: Byte::of(&b.picture),
                    flags: b.flags.as_ref().map(Byte::of),
                    flip_when_facing_left: b.flip_when_facing_left,
                    extra: b
                        .extra
                        .iter()
                        .map(|e| ExtraSprite {
                            offset: e.offset,
                            picture: Byte::of(&e.picture),
                            flags: e.flags.as_ref().map(Byte::of),
                            flip_when_facing_left: e.flip_when_facing_left,
                        })
                        .collect(),
                    hidden: b
                        .hidden
                        .as_ref()
                        .map(|h| (Byte::Word(h.word as usize, h.byte), h.bit)),
                    gone_next: b
                        .gone_next
                        .as_ref()
                        .map(|h| (h.word as usize, h.byte, h.bit)),
                }))
            })
            .collect::<Result<Vec<_>, String>>()?;
        // The board indexes 1024 tiles; a sprite's number wraps at the
        // region's sprites.
        let tiles = match ts.as_deref() {
            Some(t) => {
                let a = slot_asset(model, &map, t, AssetKind::BoardTiles)?;
                if a.format != Format::McrTiles {
                    return Err(format!("slot `{t}` isn't mcr-tiles"));
                }
                pictures::<64>(read_file, a, 8, 1024)?
            }
            None => vec![[0; 64]; 1024],
        };
        let sprites = match ss.as_deref() {
            Some(s) => {
                let a = slot_asset(model, &map, s, AssetKind::BoardSprites)?;
                pictures::<1024>(read_file, a, 32, 1)?
            }
            None => vec![[0; 1024]; 1],
        };
        Ok(Some(Self {
            gfx: Gfx { tiles, sprites },
            levels,
            kinds,
            kind_names: spec.kinds.iter().map(|k| k.name.clone()).collect(),
            counter_names: spec.counters.names().map(str::to_owned).collect(),
            board_units: false,
        }))
    }

    /// The art for a model whose positions are the board's own units
    /// (`on`, its state mapping's `board_units`) or the sprite RAM's bytes.
    #[must_use]
    pub fn in_board_units(mut self, on: bool) -> Self {
        self.board_units = on;
        self
    }

    /// Whether it reads positions as the board's own units.
    #[must_use]
    pub fn board_units(&self) -> bool {
        self.board_units
    }

    /// Whether the board draws level `level` (the spec's index).
    pub fn draws_level(&self, level: usize) -> bool {
        self.levels.get(level).is_some_and(Option::is_some)
    }

    /// Whether bodies of kind `kind` are the board's sprites.
    pub fn draws_kind(&self, kind: usize) -> bool {
        self.kinds.get(kind).is_some_and(Option::is_some)
    }

    /// The world a frame drawn from its own bodies (no frame before: a
    /// clip's frame 0, a loaded state, a jump) is drawn with: its bodies'
    /// `board_sprite.gone_next` cleared, as the board's sprite RAM for that
    /// frame was copied before the game emptied them. None when no body has
    /// it set, to draw `w` as it is.
    #[must_use]
    pub fn own_frame(&self, w: &World) -> Option<World> {
        let mut out: Option<World> = None;
        for (i, b) in w.bodies.iter().enumerate() {
            let Some(Some(k)) = self.kinds.get(b.kind) else {
                continue;
            };
            let mut nb = *b;
            if k.clear_gone_next(&mut nb) {
                out.get_or_insert_with(|| w.clone()).bodies[i] = nb;
            }
        }
        out
    }

    /// The board's picture of the world's frame, or None when its level
    /// has no board look. `shown` says by body whether the body is drawn
    /// this frame (alive and in the level, as the scene has it); `rank`
    /// gives each body's entry of sprite RAM (`present.sprite_order`, as
    /// the scene ranks it; `u32::MAX` for none). A body with no entry, or
    /// whose entry another body has, takes the lowest free one, so it is
    /// drawn under the others.
    pub fn draw(&self, w: &World, shown: &[bool], rank: &[u32]) -> Option<Picture> {
        self.draw_omitting(w, shown, rank, &Omit::default())
    }

    /// [`BoardArt::draw`] with what a theme draws itself left out
    /// (`omit`): those bodies' sprite entries, the counters' cells (the
    /// screen's own words there), and with the screen every pixel no
    /// sprite drew as [`TRANSPARENT`], but for the counters' cells still
    /// drawn, whose pixels of colour 0 in their bank are transparent and
    /// the rest kept. `drawn` and `cells` still list everything, as the
    /// frame would draw it whole. Omitting nothing draws [`BoardArt::draw`]'s
    /// picture, pen for pen.
    pub fn draw_omitting(
        &self,
        w: &World,
        shown: &[bool],
        rank: &[u32],
        omit: &Omit,
    ) -> Option<Picture> {
        self.make(w, shown, rank, omit, true)
    }

    /// What [`BoardArt::draw`] would draw, without its pixels: `pens` is
    /// empty, and `drawn`, `screen` and `cells` are the frame's. For a tool
    /// that only asks which pictures a frame shows.
    pub fn layout(&self, w: &World, shown: &[bool], rank: &[u32]) -> Option<Picture> {
        self.make(w, shown, rank, &Omit::default(), false)
    }

    fn make(
        &self,
        w: &World,
        shown: &[bool],
        rank: &[u32],
        omit: &Omit,
        paint: bool,
    ) -> Option<Picture> {
        let level = self.levels.get(w.level)?.as_ref()?;
        let look = level.look(w);
        let mut ram = vec![0u8; ENTRIES * 4];
        let mut taken = [false; ENTRIES];
        let mut at: Vec<(usize, usize)> = Vec::new();
        let mut later = Vec::new();
        for (i, b) in w.bodies.iter().enumerate() {
            // A look with its sprites off draws none: the bodies stay in
            // play, the board shows only its tile layer. A body whose
            // sprite is hidden this frame takes no entry either, as a
            // program that copies nothing of an object it still places.
            if look.sprites_off
                || !shown.get(i).copied().unwrap_or(false)
                || !self.draws_kind(b.kind)
                || matches!(self.kinds.get(b.kind), Some(Some(k)) if k.hides(b))
            {
                continue;
            }
            match rank.get(i).copied().map(|r| r as usize) {
                Some(r) if r < ENTRIES && !taken[r] => {
                    taken[r] = true;
                    at.push((i, r));
                }
                _ => later.push(i),
            }
        }
        for i in later {
            if let Some(r) = taken.iter().position(|t| !t) {
                taken[r] = true;
                at.push((i, r));
            }
        }
        let mut drawn = Vec::new();
        // The bodies' other sprites, placed once every body has its own
        // entry: (body, which, the entry it wants, its entry's bytes).
        let mut extras: Vec<(usize, usize, usize, [u8; 4])> = Vec::new();
        for (i, r) in at {
            let b = &w.bodies[i];
            let Some(Some(k)) = self.kinds.get(b.kind) else {
                continue;
            };
            // The entry's bytes: the position's own, or in the board's
            // units those that draw the sprite there.
            let (x, y) = KindSprite::bytes(self.board_units, b.x.floor(), b.y.floor());
            let mut shown_extras = 0;
            for (n, e) in k.extra.iter().enumerate() {
                let picture = e.picture.read(b);
                if picture == 0 {
                    continue;
                }
                // The k-th one drawn wants the k-th entry after the body's.
                let want = r + 1 + shown_extras;
                shown_extras += 1;
                let mut flags = e.flags.map_or(0, |f| f.read(b));
                if e.flip_when_facing_left && b.facing_left {
                    flags ^= 0x10;
                }
                let (ex, ey) = KindSprite::bytes(
                    self.board_units,
                    b.x.floor() + i64::from(e.offset[0]),
                    b.y.floor() + i64::from(e.offset[1]),
                );
                extras.push((i, n, want, [ey as u8, flags, picture, ex as u8]));
            }
            let mut flags = k.flags.map_or(0, |f| f.read(b));
            if k.flip_when_facing_left && b.facing_left {
                flags ^= 0x10;
            }
            let picture = k.picture.read(b);
            if !omit.bodies.get(i).copied().unwrap_or(false) && !omit.parts.contains(&(i, None)) {
                ram[r * 4..r * 4 + 4].copy_from_slice(&[y as u8, flags, picture, x as u8]);
            }
            let (px, py) = video::sprite_place(x as i32, y as i32);
            let u = i64::from(video::UNIT);
            let code = (u16::from(picture) + 256 * u16::from((flags >> 3) & 1))
                % self.gfx.sprites.len().max(1) as u16;
            drawn.push(Drawn {
                body: i,
                extra: None,
                kind: b.kind,
                entry: r,
                rect: [i64::from(px) * u, i64::from(py) * u, 32, 32],
                code,
                flags,
            });
        }
        // The other sprites, each in the entry after its body's own (the
        // k-th drawn one k+1 after) as the program copies it there, else
        // in the lowest entry still free, in the bodies' order; with every
        // entry taken they aren't drawn.
        for (i, n, want, bytes) in extras {
            let r = if want < ENTRIES && !taken[want] {
                want
            } else if let Some(r) = taken.iter().position(|t| !t) {
                r
            } else {
                break;
            };
            taken[r] = true;
            if !omit.bodies.get(i).copied().unwrap_or(false) && !omit.parts.contains(&(i, Some(n)))
            {
                ram[r * 4..r * 4 + 4].copy_from_slice(&bytes);
            }
            let [y, flags, picture, x] = bytes;
            let (px, py) = video::sprite_place(i32::from(x), i32::from(y));
            let u = i64::from(video::UNIT);
            let code = (u16::from(picture) + 256 * u16::from((flags >> 3) & 1))
                % self.gfx.sprites.len().max(1) as u16;
            drawn.push(Drawn {
                body: i,
                extra: Some(n),
                kind: w.bodies[i].kind,
                entry: r,
                rect: [i64::from(px) * u, i64::from(py) * u, 32, 32],
                code,
                flags,
            });
        }
        drawn.sort_by_key(|d| (d.body, d.extra));
        let omit_cells = (!omit.cells.is_empty()).then_some(omit.cells.as_slice());
        let (videoram, cells) = level.cells_with(&look.videoram, w, omit_cells);
        if !paint {
            return Some(Picture {
                pens: Vec::new(),
                palette: look.palette,
                drawn,
                screen: look.screen.clone(),
                stage: look.stage.clone(),
                cells,
                recolor: Vec::new(),
            });
        }
        let mut pens = vec![0u8; video::WIDTH * video::HEIGHT];
        video::draw(&self.gfx, &videoram, &ram, false, &mut pens);
        if omit.screen {
            let mut kept = vec![false; 960];
            for (k, c) in cells.iter().enumerate() {
                if !omit.cells.get(k).copied().unwrap_or(false) {
                    kept[c.cell] = true;
                }
            }
            for (i, p) in pens.iter_mut().enumerate() {
                // A sprite's own pixels stay the board's; where a sprite's
                // mask hides what is under it, the theme's room shows (a
                // door the guests come out of).
                if *p & video::SPRITE_BIT != 0 {
                    continue;
                }
                let cell = (i / video::WIDTH / 16) * 32 + (i % video::WIDTH) / 16;
                if !(kept.get(cell).copied().unwrap_or(false) && *p & 0x0f != 0) {
                    *p = TRANSPARENT;
                }
            }
        }
        Some(Picture {
            pens,
            palette: look.palette,
            drawn,
            screen: look.screen.clone(),
            stage: look.stage.clone(),
            cells,
            recolor: Vec::new(),
        })
    }

    /// Every name a theme may give a picture the board draws, each with
    /// its size in the board's picture's pixels: each `board-screen` slot
    /// a level's look shows (by its slot name), each board-drawn entity's
    /// sprite at every picture code the lift's sprites hold
    /// ([`sprite_name`]), and each counter's digits or tile
    /// ([`counter_name`]). The names come from the model and the count of
    /// the lift's sprites only: nothing of a picture.
    #[must_use]
    pub fn names(&self) -> Vec<(String, [u32; 2])> {
        let mut out: Vec<(String, [u32; 2])> = Vec::new();
        let add = |n: String, size: [u32; 2], out: &mut Vec<(String, [u32; 2])>| {
            if !out.iter().any(|(x, _)| *x == n) {
                out.push((n, size));
            }
        };
        for l in self.levels.iter().flatten() {
            let looks = std::iter::once(&l.base).chain(l.looks.iter().map(|(_, x)| x));
            for x in looks {
                if let Some(sc) = &x.screen {
                    add(sc.clone(), SCREEN_SIZE, &mut out);
                }
                if let Some(st) = &x.stage {
                    add(st.clone(), SCREEN_SIZE, &mut out);
                }
            }
        }
        let codes = self.gfx.sprites.len() as u16;
        for (k, ks) in self.kinds.iter().enumerate() {
            if ks.is_none() {
                continue;
            }
            for c in 0..codes {
                add(sprite_name(&self.kind_names[k], c), SPRITE_SIZE, &mut out);
            }
        }
        for l in self.levels.iter().flatten() {
            for n in &l.numbers {
                for d in 0..10 {
                    add(n.digit_name(d), CELL_SIZE, &mut out);
                }
            }
            for r in &l.repeats {
                add(counter_name(&r.name, None), CELL_SIZE, &mut out);
            }
            for t in &l.texts {
                for &(_, word) in &t.cells {
                    add(letter_name(word), CELL_SIZE, &mut out);
                }
            }
        }
        out
    }

    /// The board's named pictures as a presentation a large-art look
    /// resolves its rules against ([`gp_present::art::Art::new`]): the
    /// pictures [`BoardArt::names`] gives, in its order, on a screen the
    /// size of the board's picture.
    #[must_use]
    pub fn present(&self) -> gp_present::Present {
        gp_present::Present {
            screen: gp_present::Screen {
                size: SCREEN_SIZE,
                aspect: [1, 1],
                overscan: [0; 4],
                sprite_limit: None,
                x_wraps: false,
            },
            pictures: self
                .names()
                .into_iter()
                .map(|(file, size)| gp_present::Picture { file, size })
                .collect(),
            ..gp_present::Present::default()
        }
    }

    /// A screen's cells as a theme's rooms read them
    /// (`gp_present::art::room`): its 32 by 30 cells of 16 pixels, each
    /// one's tile code, colour bank and priority from the screen's word,
    /// and the colours of its tile's 64 pixels in the colours of the first
    /// look that shows the screen (the level's own, then its `looks`). None
    /// for a screen no look shows. All of it from the player's lift.
    #[must_use]
    pub fn screen_cells(&self, screen: &str) -> Option<gp_present::art::room::Cells> {
        let look = self.levels.iter().flatten().find_map(|l| {
            std::iter::once(&l.base)
                .chain(l.looks.iter().map(|(_, x)| x))
                .find(|x| x.screen.as_deref() == Some(screen))
        })?;
        let rgb = |c: u32| [(c >> 16) as u8, (c >> 8) as u8, c as u8];
        let cells = (0..960)
            .map(|i| {
                let v = &look.videoram;
                let word = u16::from(v[i * 2]) | (u16::from(v[i * 2 + 1]) << 8);
                let bank = ((word >> 12) & 3) as u8;
                let tile = word & 0x3ff;
                let colors = self.gfx.tiles[usize::from(tile) % self.gfx.tiles.len()]
                    .iter()
                    .map(|&pix| rgb(look.palette[usize::from(bank * 16 + (pix & 0x0f)) & 0x3f]))
                    .collect();
                gp_present::art::room::Cell {
                    tile,
                    bank,
                    prio: ((word >> 14) & 3) as u8,
                    colors,
                    flip: [word & 0x400 != 0, word & 0x800 != 0],
                }
            })
            .collect();
        Some(gp_present::art::room::Cells {
            cols: 32,
            rows: 30,
            size: CELL_SIZE[0] as i64,
            cells,
        })
    }

    /// A picture of the board's, as the frame's sprite of `kind` with
    /// `code` and `flags` draws it on its own, 32 by 32 RGBA: for a
    /// person's own contact sheet of the codes an entity shows, from their
    /// own lift. Its colours are the level's first look's.
    #[must_use]
    pub fn sprite_rgba(&self, level: usize, code: u16, flags: u8) -> Option<Vec<[u8; 4]>> {
        let l = self.levels.get(level)?.as_ref()?;
        let sprite = self
            .gfx
            .sprites
            .get(code as usize % self.gfx.sprites.len())?;
        let color = ((!flags & 3) << 4) & 0x30;
        Some(
            sprite
                .iter()
                .map(|&v| {
                    let pix = color | v;
                    if pix & 0x07 == 0 {
                        [0, 0, 0, 0]
                    } else {
                        let c = l.base.palette[(pix & 0x3f) as usize];
                        [(c >> 16) as u8, (c >> 8) as u8, c as u8, 255]
                    }
                })
                .collect(),
        )
    }
}

/// The board's named pictures ([`BoardArt::present`]) of the model a
/// bundle's bytes hold (as `gp_load` takes them), its lifted files read by
/// `read`: what a large-art theme for an arcade game names. None for a
/// model that names no board art.
pub fn present_of(bundle: &[u8], read: Read) -> Result<Option<gp_present::Present>, String> {
    let b = gp_interp::lifted::Bundle::from_bytes(bundle)?;
    let model = b.parse_model()?;
    let spec = gp_interp::compile::compile(&model, &b.assets)?;
    Ok(BoardArt::load_with(&model, &spec, read)?.map(|a| a.present()))
}

/// An arcade model's board art and what drawing a frame needs besides the
/// world: for a player whose game runs somewhere else (the page's game
/// module, in its Worker) and hands over each frame's canonical state and
/// which bodies it shows. Each body's sprite entry comes from the
/// presentation (`present.sprite_order`) as the scene ranks it, the
/// world's own data, so it is the game's; whether a body shows this frame
/// depends on its animation, which only the game's player holds, so the
/// caller says.
pub struct Companion {
    spec: Spec,
    present: gp_present::Present,
    art: BoardArt,
    /// The board's one-frame sprite lag ([`SpriteLag`]), over the frames
    /// the page hands over in turn.
    lag: std::sync::Mutex<SpriteLag>,
}

/// The MCR board's one-frame sprite lag (issue #42): the game copies its
/// objects into sprite RAM as a frame starts, before it moves them, so the
/// board's picture of a frame shows the objects as they stood a frame
/// before, while its background is the frame's own. A model's state is
/// the game's, so its picture is drawn from a world whose bodies are the
/// frame before's and the rest this frame's; the rules never see it.
///
/// Kept by whoever draws the frames in turn. Each frame is told by the
/// world's `frame`: the next frame moves the lag on, the same frame again
/// draws the same (a page drawing a frame both bare and in a theme), and
/// any other (a jump, a loaded state, the first frame) draws the frame's
/// own bodies, there being no frame before, with their
/// `board_sprite.gone_next` cleared ([`BoardArt::own_frame`]): so a hook
/// that sets it on a loaded state shows the objects on that frame and
/// hides them on the next, as the board does when the game empties them
/// as the loaded frame ends.
#[derive(Debug, Default)]
pub struct SpriteLag {
    /// The last frame seen, its bodies and which of them it showed.
    now: Option<(u64, Vec<gp_sim::world::Body>, Vec<bool>)>,
    /// The frame before it, the same.
    before: Option<(u64, Vec<gp_sim::world::Body>, Vec<bool>)>,
}

impl SpriteLag {
    /// The world and shown bodies to draw `w`'s frame with on `art`: `w`
    /// with the frame before's bodies and their `shown`; with no frame
    /// before, `w` as [`BoardArt::own_frame`] gives it; None to draw `w`
    /// and `shown` as they are.
    pub fn lagged(
        &mut self,
        art: &BoardArt,
        w: &World,
        shown: &[bool],
    ) -> Option<(World, Vec<bool>)> {
        let seen = self.now.as_ref().map(|n| n.0);
        if seen != Some(w.frame) {
            self.before = if seen.is_some_and(|f| f + 1 == w.frame) {
                self.now.take()
            } else {
                None
            };
            self.now = Some((w.frame, w.bodies.clone(), shown.to_vec()));
        }
        let Some((_, bodies, shown)) = self.before.as_ref() else {
            return art.own_frame(w).map(|w| (w, shown.to_vec()));
        };
        let mut l = w.clone();
        l.bodies.clone_from(bodies);
        Some((l, shown.clone()))
    }
}

impl Companion {
    /// From the model bundle the game was loaded from (its bytes, as
    /// `gp_load` takes them, profile switches and all), the presentation
    /// text (`assets/present.txt`) and the lifted files `read` gives by
    /// name. None for a model that names no board art.
    pub fn open(bundle: &[u8], present_text: &str, read: Read) -> Result<Option<Self>, String> {
        let b = gp_interp::lifted::Bundle::from_bytes(bundle)?;
        let model = b.parse_model()?;
        let spec = gp_interp::compile::compile(&model, &b.assets)?;
        let present = gp_present::text::read(present_text)?;
        Ok(BoardArt::load_with(&model, &spec, read)?.map(|art| Self {
            spec,
            present,
            art,
            lag: std::sync::Mutex::new(SpriteLag::default()),
        }))
    }

    /// The world and shown bodies the frame whose canonical state is
    /// `state` is drawn with: its sprites a frame late ([`SpriteLag`]).
    fn lagged(&self, state: &[u8], shown: &[bool]) -> Result<(World, Vec<bool>), String> {
        let w = World::from_canonical(&self.spec, state)?;
        let mut lag = self
            .lag
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(lag
            .lagged(&self.art, &w, shown)
            .unwrap_or_else(|| (w, shown.to_vec())))
    }

    /// The board's picture of the frame whose canonical state is `state`
    /// (`gp_sim::World::canonical`), the bodies `shown` by body (the game's
    /// scene: those it draws this frame), its sprites those of the frame
    /// before when that was the last handed over ([`SpriteLag`]). None on
    /// a level without board art.
    pub fn frame(&self, state: &[u8], shown: &[bool]) -> Result<Option<Picture>, String> {
        let (w, shown) = self.lagged(state, shown)?;
        let scene = gp_present::Animator::new().scene(&self.present, &self.spec, &w);
        Ok(self.art.draw(&w, &shown, &scene.rank))
    }

    /// The board's art, for its names and themes.
    #[must_use]
    pub fn art(&self) -> &BoardArt {
        &self.art
    }

    /// Reads the model's positions as the board's own units (`on`, its
    /// state mapping's `board_units`) or as the sprite RAM's bytes, which
    /// is how it opens.
    pub fn set_board_units(&mut self, on: bool) {
        self.art.board_units = on;
    }

    /// [`Companion::frame`] drawn in `theme` ([`BoardArt::themed`]).
    pub fn themed(
        &self,
        theme: &mut BoardTheme,
        state: &[u8],
        shown: &[bool],
    ) -> Result<Option<Themed>, String> {
        let (w, shown) = self.lagged(state, shown)?;
        let scene = gp_present::Animator::new().scene(&self.present, &self.spec, &w);
        Ok(self.art.themed(theme, &w, &shown, &scene.rank))
    }
}

/// A large-art theme over the board's pictures (docs/formats/board-art.md,
/// "Themes"): the rules of `gp_present::art`, the same reader and painter
/// an NES game's theme is drawn with, resolved against the board's named
/// pictures ([`BoardArt::present`]). It keeps the painter's memory from
/// frame to frame (a picture's age for `timed`, the props' bursts).
pub struct BoardTheme {
    pub art: gp_present::art::Art,
    present: gp_present::Present,
    index: BTreeMap<String, u32>,
    painter: gp_present::art::Painter,
    /// The game frame the painter last stepped on.
    last: Option<u64>,
    /// The rooms the theme built (`#+ room`), each over a screen the board
    /// shows, from the screen's cells in the player's lift.
    pub rooms: Vec<BuiltRoom>,
    /// The counters the theme leaves out (`#+ counter_off`), each with
    /// the tests that must hold, by the spec's counter indices. A rule
    /// naming a counter the model hasn't got is passed over.
    counter_off: Vec<(usize, Vec<Test>)>,
}

impl BoardTheme {
    /// The room rules (`#+ room_fill`, `room_run`, `room_box`, `room_at`) whose class
    /// found no cell on a screen `shown` takes, so they draw nothing
    /// there: each rule's line, its class and those screens, by line.
    #[must_use]
    pub fn empty_rooms(&self, shown: &dyn Fn(&str) -> bool) -> Vec<(usize, String, Vec<String>)> {
        let mut out: Vec<(usize, String, Vec<String>)> = Vec::new();
        for built in self.rooms.iter().filter(|b| shown(&b.screen)) {
            let room = &self.art.rules.rooms[built.room];
            for (place, cells) in room.places.iter().zip(&built.cells) {
                if *cells > 0 {
                    continue;
                }
                match out.iter_mut().find(|(l, _, _)| *l == place.line) {
                    Some(o) => o.2.push(built.screen.clone()),
                    None => out.push((place.line, place.class.clone(), vec![built.screen.clone()])),
                }
            }
        }
        out.sort();
        out
    }
}

/// A room a theme built over one of the board's screens: the screen's
/// slot and picture number, its room's number in the rules' `rooms`, and
/// the cells each of the room's rules found (by its place in the room's
/// `places`). A rule that found none draws nothing on that screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltRoom {
    pub screen: String,
    pub pic: u32,
    pub room: usize,
    pub cells: Vec<usize>,
}

/// A frame drawn in a theme: the board's picture with what the theme
/// draws itself left out ([`BoardArt::draw_omitting`]), and the theme's
/// draws, in the board's picture's pixels (in sixteenths, as
/// [`gp_present::art::ArtDraw`] has them). The board's picture is painted
/// before `frame.draws[board_at..]`: over the theme's screen, under its
/// sprites.
pub struct Themed {
    pub picture: Picture,
    pub frame: gp_present::art::ArtFrame,
    pub board_at: usize,
}

impl BoardArt {
    /// A large-art theme's rules (`gp-art 1`) read and resolved against the
    /// board's named pictures. A rule naming a picture the board hasn't
    /// got is passed over, as on the NES; rules that draw none of them are
    /// refused, as a theme made for another game.
    pub fn theme(&self, text: &str) -> Result<BoardTheme, String> {
        self.theme_seeded(text, 0)
    }

    /// [`BoardArt::theme`], its rooms' piece alternatives (`a|b`) picked
    /// by `seed` (the page gives a new one each time it loads a theme).
    pub fn theme_seeded(&self, text: &str, seed: u64) -> Result<BoardTheme, String> {
        let rules = gp_present::art::ArtRules::read(text)?;
        let present = self.present();
        let mut art = gp_present::art::Art::new(rules, &present)?;
        art.room_seed = seed;
        // Each room over its screen's cells, as the board has them.
        let mut rooms = Vec::new();
        for (pic, room) in art.room_screens(&present) {
            let screen = present.pictures[pic as usize].file.clone();
            if let Some(cells) = self.screen_cells(&screen) {
                let found = art.build_room(&present, pic, &cells)?;
                rooms.push(BuiltRoom {
                    screen,
                    pic,
                    room,
                    cells: found,
                });
            }
        }
        let index = present
            .pictures
            .iter()
            .enumerate()
            .map(|(i, p)| (p.file.clone(), i as u32))
            .collect();
        let counter = |name: &str| self.counter_names.iter().position(|n| n == name);
        let counter_off = art
            .rules
            .counter_off
            .iter()
            .filter_map(|r| {
                let when = r
                    .when
                    .iter()
                    .map(|(c, cmp, value)| {
                        Some(Test {
                            counter: counter(c)?,
                            cmp: cmp_of(cmp)?,
                            value: *value,
                        })
                    })
                    .collect::<Option<Vec<_>>>()?;
                Some((counter(&r.counter)?, when))
            })
            .collect();
        Ok(BoardTheme {
            art,
            present,
            index,
            painter: gp_present::art::Painter::new(),
            last: None,
            rooms,
            counter_off,
        })
    }

    /// The pixels `#+ recolor` gives its colours: of each sprite the board
    /// draws whose name a rule gives, each pixel of a colour number the
    /// rule names that the board drew from that sprite (its pen there).
    fn recolored(
        &self,
        rules: &[gp_present::art::Recolor],
        pic: &Picture,
        omit: &Omit,
    ) -> Vec<(u32, Option<[u8; 3]>)> {
        let mut out = Vec::new();
        if rules.is_empty() || pic.pens.is_empty() {
            return out;
        }
        for d in &pic.drawn {
            if omit.bodies.get(d.body).copied().unwrap_or(false) {
                continue;
            }
            let name = sprite_name(&self.kind_names[d.kind], d.code);
            let Some(sprite) = self.gfx.sprites.get(usize::from(d.code)) else {
                continue;
            };
            // Every rule for the picture, each in its own box.
            for rule in rules.iter().filter(|r| r.build == name) {
                let color = ((!d.flags & 3) << 4) & 0x30;
                let (hf, vf) = (d.flags & 0x10 != 0, d.flags & 0x20 != 0);
                for y in 0..32usize {
                    for x in 0..32usize {
                        let (tx, ty) = (if hf { 31 - x } else { x }, if vf { 31 - y } else { y });
                        let v = sprite[ty * 32 + tx];
                        if let Some([x0, y0, x1, y1]) = rule.area
                            && !((x0 as usize..=x1 as usize).contains(&tx)
                                && (y0 as usize..=y1 as usize).contains(&ty))
                        {
                            continue;
                        }
                        let Some((_, rgb)) = rule.colors.iter().find(|(n, _)| *n == v) else {
                            continue;
                        };
                        let (sx, sy) = (d.rect[0] + x as i64, d.rect[1] + y as i64);
                        if !(0..video::WIDTH as i64).contains(&sx)
                            || !(0..video::HEIGHT as i64).contains(&sy)
                        {
                            continue;
                        }
                        let at = sy as usize * video::WIDTH + sx as usize;
                        if pic.pens[at] == (color | v) | video::SPRITE_BIT {
                            out.push((at as u32, *rgb));
                        }
                    }
                }
            }
        }
        out
    }

    /// By body: whether every pixel its sprites draw has the colour the
    /// screen has there without sprites, so the board shows nothing of it
    /// (a body with no sprite drawn is not unseen, nor one with a mask
    /// pixel, which hides the sprites under it).
    fn unseen(&self, w: &World, full: &Picture, bare: Option<&Picture>) -> Vec<bool> {
        let n = w.bodies.len();
        let Some(bare) = bare else {
            return vec![false; n];
        };
        let rgb = |pen: u8| full.palette[usize::from(pen & 0x3f)];
        let mut seen = vec![false; n];
        let mut any = vec![false; n];
        for d in &full.drawn {
            let Some(sprite) = self.gfx.sprites.get(usize::from(d.code)) else {
                continue;
            };
            any[d.body] = true;
            let color = ((!d.flags & 3) << 4) & 0x30;
            let (hf, vf) = (d.flags & 0x10 != 0, d.flags & 0x20 != 0);
            for y in 0..32usize {
                for x in 0..32usize {
                    let (tx, ty) = (if hf { 31 - x } else { x }, if vf { 31 - y } else { y });
                    let pix = color | sprite[ty * 32 + tx];
                    if pix & 0x0f == 0 {
                        continue;
                    }
                    // A mask pixel draws nothing but hides the sprites
                    // under it (a door the guests come out of): it shows.
                    if pix & 0x07 == 0 {
                        seen[d.body] = true;
                        continue;
                    }
                    let (sx, sy) = (d.rect[0] + x as i64, d.rect[1] + y as i64);
                    if !(0..video::WIDTH as i64).contains(&sx)
                        || !(0..video::HEIGHT as i64).contains(&sy)
                    {
                        continue;
                    }
                    let under = bare.pens[sy as usize * video::WIDTH + sx as usize];
                    if rgb(pix) != rgb(under) {
                        seen[d.body] = true;
                    }
                }
            }
        }
        (0..n).map(|b| any[b] && !seen[b]).collect()
    }

    /// The frame drawn in `theme`, or None when its level has no board
    /// look. A frame whose pictures the theme draws none of is
    /// [`BoardArt::draw`]'s picture as it is, with `board_at` 0 and the
    /// theme's other draws (props, drift) after it. The painter steps once
    /// for each new game frame (`w.frame`), and starts over when the
    /// frame goes anywhere but one on (a seek, a load).
    pub fn themed(
        &self,
        theme: &mut BoardTheme,
        w: &World,
        shown: &[bool],
        rank: &[u32],
    ) -> Option<Themed> {
        use gp_present::art::{ArtDraw, Of, Src};
        let full = self.draw(w, shown, rank)?;
        let n = w.bodies.len();
        let pic = |name: &str| theme.index.get(name).copied();
        // A body the board draws in the colours already under it (a tap
        // kept in place on a screen that hides it) shows nothing there:
        // neither the theme nor the board draws it.
        // The screen with no sprites: what shows under a body, for the
        // bodies the board shows nothing of and for a recolour's nothing.
        let bare = self.draw_omitting(
            w,
            shown,
            rank,
            &Omit {
                bodies: vec![true; n],
                ..Omit::default()
            },
        );
        let unseen = self.unseen(w, &full, bare.as_ref());
        let mut bodies = vec![None; n];
        let mut order = vec![u32::MAX; n];
        // By body: which of its sprites the theme draws (the last named).
        let mut chosen: Vec<Option<Option<usize>>> = vec![None; n];
        for d in &full.drawn {
            if unseen.get(d.body).copied().unwrap_or(false) {
                continue;
            }
            if let (Some(p), Some(b)) = (
                pic(&sprite_name(&self.kind_names[d.kind], d.code)),
                bodies.get_mut(d.body),
            ) {
                chosen[d.body] = Some(d.extra);
                *b = Some(gp_present::Draw {
                    pic: p,
                    x: d.rect[0],
                    y: d.rect[1],
                    flip_x: d.flags & 0x10 != 0,
                    flip_y: d.flags & 0x20 != 0,
                });
                // A later entry is drawn over an earlier one: the scene's
                // paint order puts the lower rank last.
                order[d.body] = (ENTRIES - d.entry) as u32;
            }
        }
        // Every counter cell by its name; one the names lack is drawn by
        // the board. A text the theme words its own way (`#+ text_say`)
        // takes its letters from those words, and one it centres
        // (`#+ text_center`) moves as a line.
        let texts = self
            .levels
            .get(w.level)
            .and_then(Option::as_ref)
            .map_or(&[][..], |l| &l.texts[..]);
        let rules = &theme.art.rules;
        let said = |ti: usize| {
            texts
                .get(ti)
                .and_then(|t| rules.text_say.iter().find(|(slot, _)| *slot == t.name))
                .map(|(_, words)| words.as_bytes())
        };
        let names: Vec<(String, bool)> = full
            .cells
            .iter()
            .map(
                |c| match c.text.and_then(|(ti, ci)| said(ti).map(|w| (w, ci))) {
                    Some((w, ci)) => (letter_name([w.get(ci).copied().unwrap_or(b' '), 0]), true),
                    None => (c.name.clone(), false),
                },
            )
            .collect();
        // Each text line's move, in pixels: its letters (not its spaces)
        // centred between a rule's columns when all its cells lie there.
        let mut shift: BTreeMap<(usize, usize), i64> = BTreeMap::new();
        if !rules.text_center.is_empty() {
            let mut lines: BTreeMap<(usize, usize), (i64, i64, i64, i64)> = BTreeMap::new();
            for (c, (name, _)) in full.cells.iter().zip(&names) {
                let Some((ti, _)) = c.text else { continue };
                let (col, row) = ((c.cell % 32) as i64, c.cell / 32);
                let e = lines
                    .entry((ti, row))
                    .or_insert((col, col, i64::MAX, i64::MIN));
                e.0 = e.0.min(col);
                e.1 = e.1.max(col);
                if name != "letter-x020" {
                    e.2 = e.2.min(col);
                    e.3 = e.3.max(col);
                }
            }
            for (k, (lo, hi, first, last)) in lines {
                if first > last {
                    continue;
                }
                let row = k.1 as i64;
                if let Some([x0, x1, _, _]) = rules
                    .text_center
                    .iter()
                    .find(|[x0, x1, y0, y1]| *x0 <= lo && hi <= *x1 && (*y0..=*y1).contains(&row))
                {
                    shift.insert(k, ((x0 + x1) - (first + last)) * 8);
                }
            }
        }
        // The counters the theme leaves off this frame (`#+ counter_off`):
        // their cells drawn by neither, so the screen under them shows.
        let off: Vec<usize> = theme
            .counter_off
            .iter()
            .filter(|(_, when)| Test::all(when, w))
            .map(|(c, _)| *c)
            .collect();
        let mut cells = Vec::new();
        let mut cell_of = Vec::new();
        let mut unnamed = vec![false; full.cells.len()];
        for (k, (c, (name, own_words))) in full.cells.iter().zip(&names).enumerate() {
            if c.counter.is_some_and(|i| off.contains(&i)) {
                continue;
            }
            let Some(p) = pic(name) else {
                // A worded line's letter the theme has no picture for (a
                // space) is drawn by neither.
                unnamed[k] = !own_words;
                continue;
            };
            let dx = c
                .text
                .and_then(|(ti, _)| shift.get(&(ti, c.cell / 32)))
                .copied()
                .unwrap_or(0);
            cells.push(gp_present::Draw {
                pic: p,
                x: (c.cell % 32) as i64 * 16 + dx,
                y: (c.cell / 32) as i64 * 16,
                flip_x: false,
                flip_y: false,
            });
            cell_of.push(k);
        }
        // A look's own stage of its screen, when the theme draws that
        // stage; else the screen's.
        let level = full
            .stage
            .as_deref()
            .filter(|st| theme.art.rules.pics.iter().any(|p| p.build == *st))
            .or(full.screen.as_deref())
            .and_then(pic);
        let scene = gp_present::Scene {
            level,
            cells,
            behind: vec![false; n],
            front: vec![false; n],
            name_table: vec![false; n],
            bodies,
            rank: order,
            ..gp_present::Scene::default()
        };
        let one_on = theme.last.is_some_and(|l| l + 1 == w.frame);
        if theme.last != Some(w.frame) {
            if !one_on {
                theme.painter = gp_present::art::Painter::new();
            }
            theme.painter.step(&theme.art, &scene);
            theme.last = Some(w.frame);
        }
        let framed = gp_present::view::Framed {
            scene,
            window: gp_present::view::Window {
                x: 0,
                y: 0,
                w: SCREEN_SIZE[0],
                h: SCREEN_SIZE[1],
                view_x: 0,
                wall: 0,
                hud: [0, 0],
            },
            alpha: vec![255; n],
            ..gp_present::view::Framed::default()
        };
        let f = theme
            .painter
            .frame(&theme.art, &theme.present, &framed, SCREEN_SIZE[1]);
        // What the board still draws: what the theme left as the game's own.
        let mut keep_body = vec![false; n];
        let mut keep_cell = unnamed;
        let mut keep_screen = false;
        for d in &f.draws {
            if !matches!(d.src, Src::Own(_)) {
                continue;
            }
            match d.of {
                Of::Body(i) => keep_body[i] = true,
                Of::Cell(k) => keep_cell[cell_of[k]] = true,
                Of::Level => keep_screen = true,
                Of::Other => {}
            }
        }
        let mut omit = Omit {
            bodies: vec![false; n],
            screen: level.is_some() && !keep_screen,
            cells: keep_cell.iter().map(|k| !k).collect(),
            parts: Vec::new(),
        };
        // A body the theme names no picture for stays the board's. One the
        // theme draws is left out whole when that is its only sprite; with
        // more (a customer holding a mug) only the sprite the theme draws
        // is left out, and the board draws the rest.
        let mut sprites = vec![0usize; n];
        for d in &full.drawn {
            sprites[d.body] += 1;
        }
        for i in 0..n {
            let themed = framed_named(&framed.scene, i) && !keep_body[i];
            match chosen[i] {
                Some(part) if themed && sprites[i] > 1 => omit.parts.push((i, part)),
                _ => omit.bodies[i] = themed || unseen[i],
            }
        }
        let mut draws: Vec<ArtDraw> = Vec::with_capacity(f.draws.len());
        let mut board_at = 0;
        for d in f.draws.iter() {
            match (d.of, d.src) {
                (Of::Level, Src::Own(_)) => board_at = draws.len(),
                (Of::Level, _) => {
                    draws.push(d.clone());
                    board_at = draws.len();
                }
                // The room over the bodies in its front boxes, after the
                // board's picture.
                (_, Src::Stage(_)) if d.clip.is_some() => draws.push(d.clone()),
                // The board's own pictures are in its picture.
                (_, Src::Own(_) | Src::Tint(..) | Src::Stage(_) | Src::Backdrop) => {}
                _ => draws.push(d.clone()),
            }
        }
        let mut picture = if omit.screen
            || omit.bodies.iter().any(|b| *b)
            || omit.cells.iter().any(|c| *c)
            || !omit.parts.is_empty()
        {
            self.draw_omitting(w, shown, rank, &omit)?
        } else {
            full
        };
        picture.recolor = self.recolored(&theme.art.rules.recolor, &picture, &omit);
        // A recolour's nothing shows what is under it: the theme's room
        // where the theme draws the screen, else the board's own screen
        // there (a screen a theme leaves the board's, as a title's).
        if !omit.screen
            && let Some(bare) = &bare
        {
            for (at, c) in &mut picture.recolor {
                if c.is_none()
                    && let Some(&pen) = bare.pens.get(*at as usize)
                {
                    let v = bare.palette[usize::from(pen & 0x3f)];
                    *c = Some([(v >> 16) as u8, (v >> 8) as u8, v as u8]);
                }
            }
        }
        Some(Themed {
            picture,
            frame: gp_present::art::ArtFrame { draws, ..f },
            board_at,
        })
    }
}

/// Whether the scene has a picture for body `i` (its sprite named).
fn framed_named(s: &gp_present::Scene, i: usize) -> bool {
    s.bodies.get(i).is_some_and(Option::is_some)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up arcade lift: 1024 tiles, tile 1 all value 5; 4 sprites,
    /// sprite 0 all value 5, sprite 1 a mask on its left half, sprite 2 all value 3, sprite 3 all value 2; a screen of tile 1 in colour bank 1; a
    /// palette whose entry n is n. No game's bytes.
    fn lift(dir: &Path) {
        let map = "schema = 1\nplatform = \"arcade\"\nsource = \"cab\"\nsets = [\"cab\"]\n\n\
                   [[asset]]\nname = \"tiles\"\nformat = \"mcr-tiles\"\nregion = \"gfx1\"\ncount = 1024\nper_row = 32\n\n\
                   [[asset]]\nname = \"sprites\"\nformat = \"mcr-sprites\"\nregion = \"gfx2\"\ncount = 4\nper_row = 4\n\n\
                   [[asset]]\nname = \"stage\"\nformat = \"mcr-screen\"\nregion = \"maincpu\"\noffset = 0\n\n\
                   [[asset]]\nname = \"colors\"\nformat = \"mcr-palette\"\nregion = \"maincpu\"\noffset = 0\n";
        std::fs::write(dir.join("assets.toml"), map).unwrap();
        let png = |w: u32, h: u32, px: &[u8]| {
            let mut out = Vec::new();
            let mut e = png::Encoder::new(&mut out, w, h);
            e.set_color(png::ColorType::Indexed);
            e.set_depth(png::BitDepth::Eight);
            e.set_palette((0..=255u8).flat_map(|i| [i; 3]).collect::<Vec<_>>());
            let mut wr = e.write_header().unwrap();
            wr.write_image_data(px).unwrap();
            wr.finish().unwrap();
            out
        };
        let (tw, th) = (32 * 8, 32 * 8);
        let mut t = vec![0u8; (tw * th) as usize];
        for y in 0..8 {
            for x in 8..16 {
                t[y * tw as usize + x] = 5;
            }
        }
        std::fs::write(dir.join("tiles.png"), png(tw, th, &t)).unwrap();
        let (sw, sh) = (4 * 32, 32);
        let mut s = vec![0u8; (sw * sh) as usize];
        for y in 0..32 {
            for x in 64..96 {
                s[y * sw as usize + x] = 3;
            }
            // Sprite 1: its left half a mask (value 8: it draws nothing
            // and shows the tile through).
            for x in 32..48 {
                s[y * sw as usize + x] = 8;
            }
            // Sprite 3: value 2, for a second body.
            for x in 96..128 {
                s[y * sw as usize + x] = 2;
            }
            // Sprite 0: value 5, the screen's own colour in bank 1 (with
            // flags 2): drawn there, it shows nothing.
            for x in 0..32 {
                s[y * sw as usize + x] = 5;
            }
        }
        std::fs::write(dir.join("sprites.png"), png(sw, sh, &s)).unwrap();
        let row = vec!["0x1001"; 32].join(", ");
        let rows = vec![format!("[{row}]"); 30].join(",\n");
        std::fs::write(dir.join("stage.toml"), format!("rows = [{rows}]\n")).unwrap();
        let colors: Vec<String> = (0..64).map(|n| n.to_string()).collect();
        std::fs::write(
            dir.join("colors.toml"),
            format!("colors = [{}]\n", colors.join(", ")),
        )
        .unwrap();
    }

    const MODEL: &str = r#"
[model]
schema = "0.4"
name = "cab"
title = "Cab"
description = "A made-up arcade game"
source = "cab"
view = "side"
frame_rate = "30"
systems = ["rules", "entity_states", "counters", "spawn", "platformer.player", "platformer.enemies", "platformer.contact", "exits", "camera"]
start_level = "main"

[model.extensions]
platformer = "0.4"

[input]
device = "arcade"

[input.actions]
left = "left"

[assets.tiles]
kind = "board-tiles"
role = "graphics"
[assets.tiles.from]
source = "cab"
asset = "tiles"

[assets.sprites]
kind = "board-sprites"
role = "graphics"
[assets.sprites.from]
source = "cab"
asset = "sprites"

[assets.stage]
kind = "board-screen"
role = "layout"
[assets.stage.from]
source = "cab"
asset = "stage"

[assets.colors]
kind = "board-palette"
role = "palettes"
[assets.colors.from]
source = "cab"
asset = "colors"

[entities.player]
role = "player"
size = [16, 16]

[entities.player.hitbox]
x = 0
y = 0
w = 16
h = 16

[entities.player.board_sprite]
slot = "sprites"
picture = { word = 1, byte = 1 }
flags = { word = 1 }

[levels.main]
description = "A screen"
empty = true
size = [32, 30]
tile_size = 16

[levels.main.present.board]
palette = "colors"
tiles = "tiles"
screen = "stage"

[[levels.main.spawn]]
entity = "player"
x = 53
y = 191
"#;

    /// The board's own drawing makes the picture: the screen's tiles in
    /// their colour bank, and the body as a sprite entry (its picture and
    /// flags from its word) where gp-arcade's test draws one with these
    /// bytes, over the tiles, in the board's colour bank.
    #[test]
    fn a_body_and_its_stage_are_the_boards() {
        let dir = tempfile::tempdir().unwrap();
        lift(dir.path());
        let model = Model::parse(MODEL).unwrap();
        // Every check but evidence, which a made-up model hasn't got.
        let errs: Vec<_> = gp_model::validate::validate(&model)
            .into_iter()
            .filter(|p| p.message != "no evidence covers this fact")
            .collect();
        assert!(errs.is_empty(), "{errs:?}");
        assert!(names_board(&model));
        let lifted = gp_interp::Lifted::read_dir(dir.path(), &model).unwrap();
        let spec = gp_interp::compile::compile(&model, &lifted).unwrap();
        let art = BoardArt::load(&model, &spec, dir.path())
            .unwrap()
            .expect("the model names board art");
        let mut w = World::default();
        // Flags 0x03 (colour bank !3 & 3 = 0, pens 0x00 to 0x0f),
        // picture 2: ext[1] = flags | picture << 8.
        w.bodies.push(gp_sim::world::Body {
            x: gp_sim::Fx::from_int(53),
            y: gp_sim::Fx::from_int(191),
            alive: true,
            ext: [0, 0x0203, 0, 0],
            ..Default::default()
        });
        assert!(art.draws_kind(0) && art.draws_level(0));
        let p = art.draw(&w, &[true], &[u32::MAX]).unwrap();
        assert_eq!(
            p.drawn,
            vec![Drawn {
                body: 0,
                extra: None,
                kind: 0,
                entry: 0,
                rect: [100, 100, 32, 32],
                code: 2,
                flags: 3,
            }]
        );
        let at = |x: usize, y: usize| p.pens[y * video::WIDTH + x];
        // The tile layer: tile 1, value 5, colour bank 1.
        assert_eq!(at(0, 0), 16 + 5);
        // The sprite: value 3 in colour bank 0 (flags 3: !3 & 3 = 0).
        assert_eq!(at(100, 100), 0x03 | video::SPRITE_BIT);
        assert_eq!(at(99, 100), 16 + 5);
        assert_eq!(p.palette[21], video::color_9bit(21));
        // Not shown: only the tiles.
        let p = art.draw(&w, &[false], &[u32::MAX]).unwrap();
        assert_eq!(at_of(&p, 100, 100), 16 + 5);
    }

    /// The files the art reads are named, and the art read from them in
    /// memory (as the page's player hands them over) draws the same.
    #[test]
    fn the_art_read_from_its_files_in_memory_draws_the_same() {
        let dir = tempfile::tempdir().unwrap();
        lift(dir.path());
        let model = Model::parse(MODEL).unwrap();
        let lifted = gp_interp::Lifted::read_dir(dir.path(), &model).unwrap();
        let spec = gp_interp::compile::compile(&model, &lifted).unwrap();
        let files = BoardArt::files(&model, &spec, dir.path()).unwrap();
        assert_eq!(
            files,
            [
                "assets.toml",
                "colors.toml",
                "stage.toml",
                "tiles.png",
                "sprites.png"
            ]
        );
        let held: BTreeMap<String, Vec<u8>> = files
            .iter()
            .map(|f| (f.clone(), std::fs::read(dir.path().join(f)).unwrap()))
            .collect();
        let mem = BoardArt::load_with(&model, &spec, &|n: &str| {
            held.get(n).cloned().ok_or(format!("{n}: not handed over"))
        })
        .unwrap()
        .unwrap();
        let disk = BoardArt::load(&model, &spec, dir.path()).unwrap().unwrap();
        let mut w = World::default();
        w.bodies.push(gp_sim::world::Body {
            x: gp_sim::Fx::from_int(53),
            y: gp_sim::Fx::from_int(191),
            alive: true,
            ext: [0, 0x0203, 0, 0],
            ..Default::default()
        });
        let (a, b) = (
            mem.draw(&w, &[true], &[u32::MAX]).unwrap(),
            disk.draw(&w, &[true], &[u32::MAX]).unwrap(),
        );
        assert!(a.pens == b.pens && a.palette == b.palette);
        // A file not handed over is named.
        let e = BoardArt::load_with(&model, &spec, &|n: &str| {
            if n == "tiles.png" {
                Err(format!("{n}: not handed over"))
            } else {
                Ok(held[n].clone())
            }
        })
        .err()
        .unwrap();
        assert!(e.contains("tiles.png"), "{e}");
    }

    /// The companion draws from the bundle, the presentation and a
    /// world's canonical state what the art draws from the world.
    #[test]
    fn the_companion_draws_a_frame_from_its_state_as_the_art_does() {
        let dir = tempfile::tempdir().unwrap();
        lift(dir.path());
        let model = Model::parse(MODEL).unwrap();
        let lifted = gp_interp::Lifted::read_dir(dir.path(), &model).unwrap();
        let spec = gp_interp::compile::compile(&model, &lifted).unwrap();
        let bundle = gp_interp::lifted::Bundle::new(&model, lifted).to_bytes();
        let present = crate::present::art(&model, &spec, dir.path())
            .unwrap()
            .present_text();
        let read = |n: &str| std::fs::read(dir.path().join(n)).map_err(|e| e.to_string());
        let c = Companion::open(&bundle, &present, &read).unwrap().unwrap();
        let mut w = World::power_on(&spec);
        w.bodies.push(gp_sim::world::Body {
            x: gp_sim::Fx::from_int(53),
            y: gp_sim::Fx::from_int(191),
            alive: true,
            ext: [0, 0x0203, 0, 0],
            ..Default::default()
        });
        let shown = vec![true; w.bodies.len()];
        let art = BoardArt::load(&model, &spec, dir.path()).unwrap().unwrap();
        let want = art.draw(&w, &shown, &vec![u32::MAX; shown.len()]).unwrap();
        let got = c.frame(&w.canonical(), &shown).unwrap().unwrap();
        assert!(got.pens == want.pens && got.palette == want.palette);
        assert_eq!(got.drawn, want.drawn);
        // Not shown by the game: not drawn.
        let none = c
            .frame(&w.canonical(), &vec![false; shown.len()])
            .unwrap()
            .unwrap();
        assert!(none.drawn.is_empty());
        // In the board's units, the same body at (50, 50) draws the same
        // frame once the companion is told so.
        let mut c = c;
        c.set_board_units(true);
        for b in &mut w.bodies {
            (b.x, b.y) = (gp_sim::Fx::from_int(50), gp_sim::Fx::from_int(50));
        }
        let marked = c.frame(&w.canonical(), &shown).unwrap().unwrap();
        assert!(marked.pens == want.pens && marked.drawn == want.drawn);
    }

    /// The board's sprite lag (issue #42): over frames handed over in
    /// turn, a frame's sprites are those of the frame before, drawn the
    /// same however often the frame is asked for; a jump draws the frame's
    /// own, and so does a frame with no frame before.
    #[test]
    fn the_companion_draws_each_frames_sprites_as_they_were_a_frame_before() {
        let dir = tempfile::tempdir().unwrap();
        lift(dir.path());
        let model = Model::parse(MODEL).unwrap();
        let lifted = gp_interp::Lifted::read_dir(dir.path(), &model).unwrap();
        let spec = gp_interp::compile::compile(&model, &lifted).unwrap();
        let bundle = gp_interp::lifted::Bundle::new(&model, lifted).to_bytes();
        let present = crate::present::art(&model, &spec, dir.path())
            .unwrap()
            .present_text();
        let read = |n: &str| std::fs::read(dir.path().join(n)).map_err(|e| e.to_string());
        let c = Companion::open(&bundle, &present, &read).unwrap().unwrap();
        let art = BoardArt::load(&model, &spec, dir.path()).unwrap().unwrap();
        let mut w = World::power_on(&spec);
        w.bodies.push(gp_sim::world::Body {
            x: gp_sim::Fx::from_int(53),
            y: gp_sim::Fx::from_int(191),
            alive: true,
            ext: [0, 0x0203, 0, 0],
            ..Default::default()
        });
        let at = |w: &World, x: i64, frame: u64| {
            let mut w = w.clone();
            w.frame = frame;
            w.bodies[0].x = gp_sim::Fx::from_int(x);
            w
        };
        let own = |w: &World| art.draw(w, &[true], &[u32::MAX]).unwrap();
        let (f10, f11, f40) = (at(&w, 53, 10), at(&w, 93, 11), at(&w, 133, 40));
        // No frame before: the frame's own.
        let got = c.frame(&f10.canonical(), &[true]).unwrap().unwrap();
        assert_eq!(got.drawn, own(&f10).drawn);
        // The next frame: the body where it stood a frame before, twice.
        for _ in 0..2 {
            let got = c.frame(&f11.canonical(), &[true]).unwrap().unwrap();
            assert_eq!(got.drawn, own(&f10).drawn);
            assert!(got.pens == own(&f10).pens);
        }
        assert_ne!(own(&f10).drawn, own(&f11).drawn);
        // A jump: the frame's own again.
        let got = c.frame(&f40.canonical(), &[true]).unwrap().unwrap();
        assert_eq!(got.drawn, own(&f40).drawn);
        // A body the game didn't show a frame before isn't drawn now.
        let mut lag = SpriteLag::default();
        assert!(lag.lagged(&art, &f10, &[false]).is_none());
        let (lw, shown) = lag.lagged(&art, &f11, &[true]).unwrap();
        assert_eq!((lw.bodies[0].x, shown), (f10.bodies[0].x, vec![false]));
    }

    /// The same made-up scene drawn twice: once with its positions the
    /// sprite RAM's bytes (an unmarked model, as one arcade port's is today), once
    /// in the board's own units (a mapping's `board_units`), top left and
    /// y down. The board's picture, what it lists as drawn and a theme's
    /// draw of a sprite are the same, pen for pen and pixel for pixel.
    #[test]
    fn a_scene_in_board_units_draws_as_the_same_scene_in_the_rams_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let (_, raw, raw_w) = two_bodies(dir.path());
        assert!(!raw.board_units(), "unmarked: the RAM's bytes");
        let units = raw.clone().in_board_units(true);
        // The bytes (53, 191) and (153, 191) are the board's (50, 50) and
        // (150, 50): x - 3 and 241 - y.
        let mut w = raw_w.clone();
        for (b, x) in w.bodies.iter_mut().zip([50, 150]) {
            b.x = gp_sim::Fx::from_int(x);
            b.y = gp_sim::Fx::from_int(50);
        }
        let (shown, rank) = (&[true, true], &[u32::MAX; 2]);
        let a = raw.draw(&raw_w, shown, rank).unwrap();
        let b = units.draw(&w, shown, rank).unwrap();
        assert!(
            a.pens == b.pens && a.palette == b.palette,
            "the same picture"
        );
        assert_eq!(a.drawn, b.drawn);
        assert_eq!(b.drawn[0].rect, [100, 100, 32, 32]);
        assert_eq!(b.drawn[1].rect, [300, 100, 32, 32]);
        // Each unit is two screen pixels: the body's place doubled.
        assert_eq!(at_of(&b, 100, 100), 0x03 | video::SPRITE_BIT);
        let rules = "gp-art 1\npic sprite-player-02 mug.png 40 40 -4 -4\n";
        let (mut ta, mut tb) = (raw.theme(rules).unwrap(), units.theme(rules).unwrap());
        let ta = raw.themed(&mut ta, &raw_w, shown, rank).unwrap();
        let tb = units.themed(&mut tb, &w, shown, rank).unwrap();
        assert!(ta.picture.pens == tb.picture.pens);
        assert_eq!(ta.frame.draws.len(), 1);
        let (da, db) = (&ta.frame.draws[0], &tb.frame.draws[0]);
        assert_eq!([da.x, da.y, da.w, da.h], [db.x, db.y, db.w, db.h]);
        // A body at the board's edges wraps as the entry's bytes do: (-3,
        // 241) is the bytes (0, 0), as the bytes (0, 0) are placed.
        let mut edge = w.clone();
        edge.bodies[0].x = gp_sim::Fx::from_int(-3);
        edge.bodies[0].y = gp_sim::Fx::from_int(241);
        let mut edge_raw = raw_w.clone();
        edge_raw.bodies[0].x = gp_sim::Fx::from_int(0);
        edge_raw.bodies[0].y = gp_sim::Fx::from_int(0);
        let (a, b) = (
            raw.draw(&edge_raw, shown, rank).unwrap(),
            units.draw(&edge, shown, rank).unwrap(),
        );
        assert!(a.pens == b.pens && a.drawn == b.drawn);
    }

    fn at_of(p: &Picture, x: usize, y: usize) -> u8 {
        p.pens[y * video::WIDTH + x]
    }

    /// The made-up model with another look chosen by a counter and three
    /// counters drawn as tiles, and the lift with the `rows` table they
    /// read: digit 0's code 0x30, the high byte 0x30, a colour byte 0x77
    /// and the lives' tile 0x10. No game's bytes.
    fn with_looks_and_counters(dir: &Path) -> (Model, Spec, BoardArt) {
        with_looks_and(dir, "")
    }

    /// [`with_looks_and_counters`] with `more` (more of the level's board
    /// tables) before its spawns.
    fn with_looks_and(dir: &Path, more: &str) -> (Model, Spec, BoardArt) {
        lift(dir);
        let map = std::fs::read_to_string(dir.join("assets.toml")).unwrap()
            + "\n[[asset]]\nname = \"codes\"\nformat = \"byte-rows\"\nregion = \"maincpu\"\noffset = 0\ncount = 1\nwidth = 4\n\n\
               [[asset]]\nname = \"stage2\"\nformat = \"mcr-screen\"\nregion = \"maincpu\"\noffset = 0\n\n\
               [[asset]]\nname = \"words\"\nformat = \"byte-rows\"\nregion = \"maincpu\"\noffset = 0\ncount = 1\nwidth = 6\n";
        std::fs::write(dir.join("assets.toml"), map).unwrap();
        std::fs::write(dir.join("codes.toml"), "rows = [[48, 48, 119, 16]]\n").unwrap();
        std::fs::write(dir.join("words.toml"), "rows = [[72, 73, 32, 74, 0, 75]]\n").unwrap();
        let row = vec!["0x0001"; 32].join(", ");
        let rows = vec![format!("[{row}]"); 30].join(",\n");
        std::fs::write(dir.join("stage2.toml"), format!("rows = [{rows}]\n")).unwrap();
        let text = MODEL.replace(
            "[[levels.main.spawn]]",
            &(more.to_owned()
                + r#"[[levels.main.present.board.looks]]
when = [{ counter = "bar", cmp = "==", value = 1 }]
screen = "stage2"
palette_writes = [{ at = 0x2A, slot = "codes", column = 2 }]

[[levels.main.present.board.counters]]
counter = "score"
at = [2, 1]
digits = 4
tile = { slot = "codes" }
high = { slot = "codes", column = 1 }

[[levels.main.present.board.counters]]
counter = "cleared"
plus = 1
at = [29, 27]
digits = 2
zeros = true
tile = { slot = "codes" }
high = { slot = "codes", column = 1 }
when = [{ counter = "bar", cmp = "<", value = 3 }]

[[levels.main.present.board.texts]]
text = { slot = "words" }
at = [4, 5]
high = { slot = "codes", column = 1 }
when = [{ counter = "bar", cmp = "==", value = 2 }]

[[levels.main.present.board.repeats]]
counter = "lives"
plus = -1
at = [8, 2]
step = [-1, 0]
most = 3
tile = { slot = "codes", column = 3 }
high = { slot = "codes", column = 1 }

[[levels.main.spawn]]"#),
        ) + r#"
[assets.codes]
kind = "rows"
role = "graphics"
[assets.codes.from]
source = "cab"
asset = "codes"

[assets.words]
kind = "rows"
role = "graphics"
[assets.words.from]
source = "cab"
asset = "words"

[assets.stage2]
kind = "board-screen"
role = "layout"
[assets.stage2.from]
source = "cab"
asset = "stage2"

[counters.bar]
[counters.score]
[counters.cleared]
[counters.lives]
"#;
        let model = Model::parse(&text).unwrap();
        let errs: Vec<_> = gp_model::validate::validate(&model)
            .into_iter()
            .filter(|p| p.message != "no evidence covers this fact")
            .collect();
        assert!(errs.is_empty(), "{errs:?}");
        let used = model.features_used();
        assert!(used.contains("level.board.looks") && used.contains("level.board.counters"));
        let lifted = gp_interp::Lifted::read_dir(dir, &model).unwrap();
        let spec = gp_interp::compile::compile(&model, &lifted).unwrap();
        let art = BoardArt::load(&model, &spec, dir).unwrap().unwrap();
        (model, spec, art)
    }

    /// A look is chosen by the counters as each frame is drawn: the level
    /// stays and its bodies with it.
    #[test]
    fn a_counter_chooses_the_screen_and_colours_each_frame() {
        let dir = tempfile::tempdir().unwrap();
        let (_, spec, art) = with_looks_and_counters(dir.path());
        let bar = spec.counters.index_of("bar").unwrap();
        let mut w = World {
            counters: vec![0; spec.counters.len()],
            ..Default::default()
        };
        let p = art.draw(&w, &[], &[]).unwrap();
        // The level's own: tile 1 in colour bank 1, colour 21 as loaded.
        assert_eq!(at_of(&p, 0, 0), 16 + 5);
        assert_eq!(p.palette[21], video::color_9bit(21));
        w.counters[bar] = 1;
        let p = art.draw(&w, &[], &[]).unwrap();
        // The other look: its screen's tile 1 in colour bank 0, and the
        // byte 0x77 written at 0x2A (colour 21, ninth bit 0).
        assert_eq!(at_of(&p, 0, 0), 5);
        assert_eq!(p.palette[21], video::color_9bit(0x77));
        w.counters[bar] = 2;
        assert_eq!(at_of(&art.draw(&w, &[], &[]).unwrap(), 0, 0), 16 + 5);
    }

    /// A look that clears the tile layer draws tile 0 everywhere, in
    /// colour bank 0, while its counters choose it; a look can't both
    /// clear and name a screen.
    #[test]
    fn a_look_that_clears_draws_no_tiles() {
        let dir = tempfile::tempdir().unwrap();
        lift(dir.path());
        let text = MODEL.replace(
            "[[levels.main.spawn]]",
            "[[levels.main.present.board.looks]]\nwhen = [{ counter = \"bar\", cmp = \"==\", value = 1 }]\nclear = true\n\n[[levels.main.spawn]]",
        ) + "\n[counters.bar]\n";
        let model = Model::parse(&text).unwrap();
        assert!(model.features_used().contains("level.board.looks.clear"));
        let lifted = gp_interp::Lifted::read_dir(dir.path(), &model).unwrap();
        let spec = gp_interp::compile::compile(&model, &lifted).unwrap();
        let art = BoardArt::load(&model, &spec, dir.path()).unwrap().unwrap();
        let mut w = World {
            counters: vec![0; spec.counters.len()],
            ..Default::default()
        };
        assert_eq!(at_of(&art.draw(&w, &[], &[]).unwrap(), 0, 0), 16 + 5);
        w.counters[spec.counters.index_of("bar").unwrap()] = 1;
        let p = art.draw(&w, &[], &[]).unwrap();
        assert_eq!(at_of(&p, 0, 0), 0);
        assert_eq!(p.screen, None);
        let both = Model::parse(&text.replace("clear = true", "clear = true\nscreen = \"stage\""))
            .unwrap();
        let err = BoardArt::load(&both, &spec, dir.path()).err().unwrap();
        assert!(err.contains("clears the tile layer"), "{err}");
    }

    /// A look with its sprites off draws no sprite while its counters
    /// choose it, the body still in the world, and its tile layer as
    /// before; the next frame its counters don't hold draws the sprite.
    #[test]
    fn a_look_with_its_sprites_off_draws_none() {
        let dir = tempfile::tempdir().unwrap();
        lift(dir.path());
        let text = MODEL.replace(
            "[[levels.main.spawn]]",
            "[[levels.main.present.board.looks]]\nwhen = [{ counter = \"bar\", cmp = \"==\", value = 1 }]\nsprites_off = true\n\n[[levels.main.spawn]]",
        ) + "\n[counters.bar]\n";
        let model = Model::parse(&text).unwrap();
        assert!(
            model
                .features_used()
                .contains("level.board.looks.sprites_off")
        );
        let said: Vec<_> = gp_model::validate::validate(&model)
            .into_iter()
            .filter(|p| p.message != "no evidence covers this fact")
            .collect();
        assert!(said.is_empty(), "{said:?}");
        let lifted = gp_interp::Lifted::read_dir(dir.path(), &model).unwrap();
        let spec = gp_interp::compile::compile(&model, &lifted).unwrap();
        let art = BoardArt::load(&model, &spec, dir.path()).unwrap().unwrap();
        let mut w = World {
            counters: vec![0; spec.counters.len()],
            ..Default::default()
        };
        w.bodies.push(gp_sim::world::Body {
            x: gp_sim::Fx::from_int(53),
            y: gp_sim::Fx::from_int(191),
            alive: true,
            ext: [0, 0x0203, 0, 0],
            ..Default::default()
        });
        let with = art.draw(&w, &[true], &[u32::MAX]).unwrap();
        assert_eq!(with.drawn.len(), 1);
        w.counters[spec.counters.index_of("bar").unwrap()] = 1;
        let off = art.draw(&w, &[true], &[u32::MAX]).unwrap();
        assert!(off.drawn.is_empty());
        assert!(off.pens.iter().all(|p| p & video::SPRITE_BIT == 0));
        assert_eq!(at_of(&off, 0, 0), at_of(&with, 0, 0));
        assert_eq!(off.screen, with.screen);
        assert_eq!(w.bodies.len(), 1);
    }

    /// A body's other sprite takes the entry after the body's own, else
    /// the lowest entry free after every body's own, at its offset, while
    /// its picture isn't 0.
    #[test]
    fn a_bodys_other_sprite_is_drawn_while_its_picture_is_set() {
        let dir = tempfile::tempdir().unwrap();
        lift(dir.path());
        let text = MODEL.replace(
            "flags = { word = 1 }\n",
            "flags = { word = 1 }\n\n[[entities.player.board_sprite.extra]]\noffset = [16, 0]\npicture = { word = 2 }\nflags = { number = 3 }\n",
        );
        let model = Model::parse(&text).unwrap();
        let errs: Vec<_> = gp_model::validate::validate(&model)
            .into_iter()
            .filter(|p| p.message != "no evidence covers this fact")
            .collect();
        assert!(errs.is_empty(), "{errs:?}");
        assert!(model.features_used().contains("sprite.board.extra"));
        let lifted = gp_interp::Lifted::read_dir(dir.path(), &model).unwrap();
        let spec = gp_interp::compile::compile(&model, &lifted).unwrap();
        let art = BoardArt::load(&model, &spec, dir.path()).unwrap().unwrap();
        let mut w = World::default();
        w.bodies.push(gp_sim::world::Body {
            x: gp_sim::Fx::from_int(53),
            y: gp_sim::Fx::from_int(191),
            alive: true,
            ext: [0, 0x0203, 0, 0],
            ..Default::default()
        });
        assert_eq!(art.draw(&w, &[true], &[u32::MAX]).unwrap().drawn.len(), 1);
        w.bodies[0].ext[2] = 1;
        let p = art.draw(&w, &[true], &[u32::MAX]).unwrap();
        let (px, py) = video::sprite_place(69, 191);
        let u = i64::from(video::UNIT);
        assert_eq!(
            p.drawn[1],
            Drawn {
                body: 0,
                extra: Some(0),
                kind: 0,
                entry: 1,
                rect: [i64::from(px) * u, i64::from(py) * u, 32, 32],
                code: 1,
                flags: 3,
            }
        );
        assert_eq!(p.drawn[0].extra, None);
        // A theme naming only the other sprite's picture (1) draws that
        // one; the board still draws the body's own (2), not the other.
        let mut theme = art
            .theme("gp-art 1\nscale 2\npic sprite-player-01 mug.png 32 32 0 0\n")
            .unwrap();
        let t = art.themed(&mut theme, &w, &[true], &[u32::MAX]).unwrap();
        let own = video::SPRITE_BIT | 3;
        assert!(
            t.picture.pens.contains(&own),
            "the body's own sprite is drawn"
        );
        let arts = t
            .frame
            .draws
            .iter()
            .filter(|d| matches!(d.src, gp_present::art::Src::Art(_)))
            .count();
        assert_eq!(arts, 1);
        let other = art.draw(&w, &[true], &[u32::MAX]).unwrap();
        let (x, y) = (
            other.drawn[1].rect[0] as usize + 31,
            other.drawn[1].rect[1] as usize + 31,
        );
        assert_ne!(
            t.picture.pens[y * video::WIDTH + x] & video::SPRITE_BIT,
            video::SPRITE_BIT,
            "the other sprite is left to the theme"
        );
        // A body in entry 5 has its other sprite in entry 6, the one after
        // its own, above every entry below; with 6 taken by another body
        // it goes to the lowest free one.
        let p = art.draw(&w, &[true], &[5]).unwrap();
        assert_eq!((p.drawn[0].entry, p.drawn[1].entry), (5, 6));
        w.bodies.push(w.bodies[0]);
        w.bodies[1].ext[2] = 0;
        let p = art.draw(&w, &[true, true], &[5, 6]).unwrap();
        let entries: Vec<_> = p.drawn.iter().map(|d| (d.body, d.extra, d.entry)).collect();
        assert_eq!(entries, [(0, None, 5), (0, Some(0), 0), (1, None, 6)]);
    }

    /// A body whose `hidden` bit is set draws none of its sprites, its
    /// own or its other ones, and takes no entry, while it stays where it
    /// is; another body keeps its entry; with the bit clear it is drawn
    /// as before.
    #[test]
    fn a_body_hidden_by_its_word_draws_no_board_sprite_that_frame() {
        let dir = tempfile::tempdir().unwrap();
        lift(dir.path());
        let text = MODEL.replace(
            "flags = { word = 1 }\n",
            "flags = { word = 1 }\nhidden = { word = 3, byte = 1, bit = 2 }\n\n[[entities.player.board_sprite.extra]]\noffset = [16, 0]\npicture = { word = 2 }\n",
        );
        let model = Model::parse(&text).unwrap();
        let errs: Vec<_> = gp_model::validate::validate(&model)
            .into_iter()
            .filter(|p| p.message != "no evidence covers this fact")
            .collect();
        assert!(errs.is_empty(), "{errs:?}");
        assert!(model.features_used().contains("sprite.board.hidden"));
        let lifted = gp_interp::Lifted::read_dir(dir.path(), &model).unwrap();
        let spec = gp_interp::compile::compile(&model, &lifted).unwrap();
        let art = BoardArt::load(&model, &spec, dir.path()).unwrap().unwrap();
        let body = gp_sim::world::Body {
            x: gp_sim::Fx::from_int(53),
            y: gp_sim::Fx::from_int(191),
            alive: true,
            ext: [0, 0x0203, 1, 0],
            ..Default::default()
        };
        let mut w = World::default();
        w.bodies.push(body);
        w.bodies.push(body);
        w.bodies[1].ext[2] = 0;
        let seen = |w: &World| {
            let p = art.draw(w, &[true, true], &[0, 1]).unwrap();
            p.drawn
                .iter()
                .map(|d| (d.body, d.extra, d.entry))
                .collect::<Vec<_>>()
        };
        assert_eq!(seen(&w), [(0, None, 0), (0, Some(0), 2), (1, None, 1)]);
        // Another bit of the byte, or the bit in another byte, hides nothing.
        w.bodies[0].ext[3] = 0x0300 | 0x04;
        assert_eq!(seen(&w).len(), 3);
        w.bodies[0].ext[3] = 0x0400;
        assert_eq!(seen(&w), [(1, None, 1)]);
        let hidden = art.draw(&w, &[true, true], &[0, 1]).unwrap();
        let lone = art.draw(&w, &[false, true], &[0, 1]).unwrap();
        assert_eq!(hidden.pens, lone.pens, "drawn as if the body weren't shown");
        assert_eq!((w.bodies[0].x, w.bodies[0].y), (body.x, body.y));
        // Without `bit`, any bit of the byte hides it.
        let text = text.replace(
            "hidden = { word = 3, byte = 1, bit = 2 }",
            "hidden = { word = 3, byte = 1 }",
        );
        let model = Model::parse(&text).unwrap();
        let art2 = BoardArt::load(&model, &spec, dir.path()).unwrap().unwrap();
        w.bodies[0].ext[3] = 0x0100;
        let p = art2.draw(&w, &[true, true], &[0, 1]).unwrap();
        assert_eq!(p.drawn.len(), 1);
        w.bodies[0].ext[3] = 0x00ff;
        let p = art2.draw(&w, &[true, true], &[0, 1]).unwrap();
        assert_eq!(p.drawn.len(), 3);
        // A bit past 7 is refused.
        let bad = Model::parse(&text.replace(
            "hidden = { word = 3, byte = 1 }",
            "hidden = { word = 3, bit = 8 }",
        ))
        .unwrap();
        assert!(
            gp_model::validate::validate(&bad)
                .iter()
                .any(|p| p.message == "`bit` is 0 to 7")
        );
    }

    /// A body whose `gone_next` bit is set is left out of the next frame's
    /// picture, drawn from this frame's bodies, but still drawn on a frame
    /// drawn from its own: a loaded state whose hook sets it shows the
    /// body on the loaded frame and none of it on the next (a clip
    /// starting on the game's last frame before it empties its objects).
    /// `hidden` beside it still hides the loaded frame.
    #[test]
    fn a_body_gone_next_shows_on_the_loaded_frame_and_not_the_next() {
        let dir = tempfile::tempdir().unwrap();
        lift(dir.path());
        let text = MODEL.replace(
            "flags = { word = 1 }\n",
            "flags = { word = 1 }\nhidden = { word = 3, byte = 4, bit = 0 }\ngone_next = { word = 3, byte = 7, bit = 6 }\n",
        );
        let model = Model::parse(&text).unwrap();
        let errs: Vec<_> = gp_model::validate::validate(&model)
            .into_iter()
            .filter(|p| p.message != "no evidence covers this fact")
            .collect();
        assert!(errs.is_empty(), "{errs:?}");
        assert!(model.features_used().contains("sprite.board.gone_next"));
        // The bundle gp theme look sheet builds reads the model back with
        // it: the bundle's reader is this same board sprite.
        let read = crate::bundle(&model, dir.path())
            .unwrap()
            .parse_model()
            .unwrap();
        assert_eq!(read, model);
        let lifted = gp_interp::Lifted::read_dir(dir.path(), &model).unwrap();
        let spec = gp_interp::compile::compile(&model, &lifted).unwrap();
        let art = BoardArt::load(&model, &spec, dir.path()).unwrap().unwrap();
        const GONE: i64 = 1 << 62;
        const HIDDEN: i64 = 1 << 32;
        let at = |frame: u64, ext3: i64| World {
            frame,
            bodies: vec![gp_sim::world::Body {
                x: gp_sim::Fx::from_int(53),
                y: gp_sim::Fx::from_int(191),
                alive: true,
                ext: [0, 0x0203, 0, ext3],
                ..Default::default()
            }],
            ..Default::default()
        };
        // Frames drawn in turn, as a lockstep draws them: how many sprites
        // each one's picture has.
        let run = |frames: &[World]| {
            let mut lag = SpriteLag::default();
            frames
                .iter()
                .map(|w| {
                    let (dw, shown) = lag
                        .lagged(&art, w, &[true])
                        .unwrap_or_else(|| (w.clone(), vec![true]));
                    art.draw(&dw, &shown, &[u32::MAX]).unwrap().drawn.len()
                })
                .collect::<Vec<_>>()
        };
        // Loaded on frame 10 with gone_next set (the hook's loaded), the
        // step of 11 clearing it: shown on 10, gone on 11, back on 12.
        assert_eq!(run(&[at(10, GONE), at(11, 0), at(12, 0)]), [1, 0, 1]);
        // In play the same: set on 11, gone on 12.
        assert_eq!(run(&[at(10, 0), at(11, GONE), at(12, 0)]), [1, 1, 0]);
        // A jump is drawn from its own bodies too.
        assert_eq!(run(&[at(10, 0), at(40, GONE), at(41, 0)]), [1, 1, 0]);
        // `hidden` on the loaded frame hides it there and on the next.
        assert_eq!(run(&[at(10, HIDDEN), at(11, HIDDEN), at(12, 0)]), [0, 0, 0]);
        assert_eq!(run(&[at(10, HIDDEN), at(11, 0), at(12, 0)]), [0, 0, 1]);
        // Drawn alone, a frame with gone_next set is hidden: own_frame is
        // what clears it, and leaves a world with none set as it is.
        assert_eq!(
            art.draw(&at(10, GONE), &[true], &[u32::MAX])
                .unwrap()
                .drawn
                .len(),
            0
        );
        assert!(art.own_frame(&at(10, HIDDEN)).is_none());
        assert_eq!(
            art.own_frame(&at(10, GONE | HIDDEN)).unwrap().bodies[0].ext[3],
            HIDDEN
        );
        // A byte past 7 is refused.
        let bad = Model::parse(&text.replace(
            "gone_next = { word = 3, byte = 7, bit = 6 }",
            "gone_next = { word = 3, byte = 8 }",
        ))
        .unwrap();
        assert!(
            gp_model::validate::validate(&bad)
                .iter()
                .any(|p| p.message == "`byte` is 0 to 7" && p.path.ends_with("gone_next"))
        );
    }

    /// A string drawn while its counter test holds, a byte a cell up to
    /// its zero byte, a space as tile 0, as a game's text routine writes
    /// one.
    #[test]
    fn a_text_is_drawn_while_its_counters_say_so() {
        let dir = tempfile::tempdir().unwrap();
        let (_, spec, art) = with_looks_and_counters(dir.path());
        let mut w = World {
            counters: vec![0; spec.counters.len()],
            ..Default::default()
        };
        let level = art.levels[0].as_ref().unwrap();
        let cell = |v: &[u8], x: usize, y: usize| {
            let i = (y * 32 + x) * 2;
            u16::from_le_bytes([v[i], v[i + 1]])
        };
        let screen = level.look(&w).videoram.clone();
        let v = level.cells(&level.look(&w).videoram, &w).into_owned();
        assert_eq!(
            cell(&v, 4, 5),
            cell(&screen, 4, 5),
            "not drawn while bar is 0"
        );
        w.counters[spec.counters.index_of("bar").unwrap()] = 2;
        let v = level.cells(&level.look(&w).videoram, &w).into_owned();
        assert_eq!(cell(&v, 4, 5), 0x3048);
        assert_eq!(cell(&v, 5, 5), 0x3049);
        assert_eq!(cell(&v, 6, 5), 0x3000, "a space is tile 0");
        assert_eq!(cell(&v, 7, 5), 0x304A);
        assert_eq!(cell(&v, 8, 5), cell(&screen, 8, 5), "the zero byte ends it");
    }

    /// Counters drawn as digits and as a tile repeated, over the screen's
    /// own cells, as a game's vblank writes its score and lives.
    #[test]
    fn counters_are_drawn_as_tiles_over_the_screen() {
        let dir = tempfile::tempdir().unwrap();
        let (_, spec, art) = with_looks_and_counters(dir.path());
        let c = |n: &str| spec.counters.index_of(n).unwrap();
        let mut w = World {
            counters: vec![0; spec.counters.len()],
            ..Default::default()
        };
        w.counters[c("score")] = 120;
        w.counters[c("cleared")] = 0;
        w.counters[c("lives")] = 3;
        let level = art.levels[0].as_ref().unwrap();
        let cell = |v: &[u8], x: usize, y: usize| {
            let i = (y * 32 + x) * 2;
            u16::from_le_bytes([v[i], v[i + 1]])
        };
        let v = level.cells(&level.look(&w).videoram, &w).into_owned();
        // " 120": the leading zero left as the screen has it.
        assert_eq!(cell(&v, 2, 1), 0x1001);
        assert_eq!(
            [cell(&v, 3, 1), cell(&v, 4, 1), cell(&v, 5, 1)],
            [0x3031, 0x3032, 0x3030]
        );
        // The level number: cleared plus 1, with its zero.
        assert_eq!([cell(&v, 29, 27), cell(&v, 30, 27)], [0x3030, 0x3031]);
        // Two lives besides the one in play, leftward from (8, 2).
        assert_eq!(
            [cell(&v, 8, 2), cell(&v, 7, 2), cell(&v, 6, 2)],
            [0x3010, 0x3010, 0x1001]
        );
        // A score of 0 draws its last digit; too many digits shows the last.
        w.counters[c("score")] = 0;
        w.counters[c("bar")] = 3;
        let v = level.cells(&level.look(&w).videoram, &w).into_owned();
        assert_eq!([cell(&v, 4, 1), cell(&v, 5, 1)], [0x1001, 0x3030]);
        // `when` no longer holds: the level number isn't drawn.
        assert_eq!(cell(&v, 30, 27), 0x1001);
        w.counters[c("score")] = 123_456;
        let v = level.cells(&level.look(&w).videoram, &w).into_owned();
        assert_eq!(cell(&v, 2, 1), 0x3033);
        // No counters drawn: the screen as it is, not copied.
        let mut bare = level.clone();
        bare.numbers.clear();
        bare.repeats.clear();
        bare.texts.clear();
        assert!(matches!(
            bare.cells(&bare.base.videoram, &w),
            std::borrow::Cow::Borrowed(_)
        ));
    }

    /// Wrong uses are named in plain words by the model's check.
    #[test]
    fn a_look_or_counter_used_wrongly_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let (model, _, _) = with_looks_and_counters(dir.path());
        let mut bad = model.clone();
        let b = bad
            .levels
            .get_mut("main")
            .unwrap()
            .core
            .present
            .as_mut()
            .unwrap();
        let b = b.board.as_mut().unwrap();
        b.looks[0].when.clear();
        b.looks.push(gp_model::model::BoardLookWhen {
            when: vec![gp_model::model::Guard {
                counter: "nope".into(),
                cmp: gp_model::model::Cmp::Eq,
                value: 0,
            }],
            palette: None,
            screen: None,
            clear: false,
            sprites_off: false,
            palette_writes: Vec::new(),
        });
        b.counters[0].at = [30, 1];
        b.repeats[0].step = Some([-5, 0]);
        let said: Vec<String> = gp_model::validate::validate(&bad)
            .into_iter()
            .filter(|p| p.message != "no evidence covers this fact")
            .map(|p| format!("{}: {}", p.path, p.message))
            .collect();
        let has = |s: &str| said.iter().any(|m| m.contains(s));
        assert!(
            has("looks[0].when: a look is chosen by its counter tests"),
            "{said:?}"
        );
        assert!(has("looks[1].when: no counter named `nope`"), "{said:?}");
        assert!(has("looks[1]: the look changes nothing"), "{said:?}");
        assert!(
            has("counters[0].digits: cell (33, 1) is off the board"),
            "{said:?}"
        );
        assert!(
            has("repeats[0].most: cell (-2, 2) is off the board"),
            "{said:?}"
        );
    }

    /// The made-up model's art and a world of two bodies: picture 2 at
    /// (53, 191) and picture 3 at (153, 191), both in colour bank 0.
    fn two_bodies(dir: &Path) -> (Spec, BoardArt, World) {
        lift(dir);
        let model = Model::parse(MODEL).unwrap();
        let lifted = gp_interp::Lifted::read_dir(dir, &model).unwrap();
        let spec = gp_interp::compile::compile(&model, &lifted).unwrap();
        let art = BoardArt::load(&model, &spec, dir).unwrap().unwrap();
        let mut w = World::default();
        for (x, ext) in [(53, 0x0203), (153, 0x0303)] {
            w.bodies.push(gp_sim::world::Body {
                x: gp_sim::Fx::from_int(x),
                y: gp_sim::Fx::from_int(191),
                alive: true,
                ext: [0, ext, 0, 0],
                ..Default::default()
            });
        }
        (spec, art, w)
    }

    /// The names a theme may use: the screen by its slot, each sprite
    /// code of the board-drawn entity, each counter's digits and tile.
    #[test]
    fn a_theme_names_the_boards_pictures_from_the_model() {
        let dir = tempfile::tempdir().unwrap();
        let (_, art, _) = two_bodies(dir.path());
        let names: Vec<String> = art.names().into_iter().map(|(n, _)| n).collect();
        assert_eq!(
            names,
            [
                "stage",
                "sprite-player-00",
                "sprite-player-01",
                "sprite-player-02",
                "sprite-player-03"
            ]
        );
        let dir = tempfile::tempdir().unwrap();
        let (_, _, art) = with_looks_and_counters(dir.path());
        let names = art.names();
        let has = |n: &str, size: [u32; 2]| names.iter().any(|(x, s)| x == n && *s == size);
        assert!(has("stage", SCREEN_SIZE) && has("stage2", SCREEN_SIZE));
        assert!(has("sprite-player-03", SPRITE_SIZE));
        assert!(has("counter-score-0", CELL_SIZE) && has("counter-score-9", CELL_SIZE));
        assert!(has("counter-cleared-5", CELL_SIZE) && has("counter-lives", CELL_SIZE));
        assert!(!names.iter().any(|(n, _)| n == "counter-lives-0"));
        assert_eq!(sprite_name("mug", 0x4d), "sprite-mug-4d");
        assert_eq!(sprite_name("mug", 0x14d), "sprite-mug-14d");
    }

    /// A theme that replaces nothing the frame shows draws the board's
    /// picture as it is, pen for pen, and only that.
    #[test]
    fn a_theme_that_replaces_nothing_draws_the_board_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let (_, art, w) = two_bodies(dir.path());
        let plain = art.draw(&w, &[true, true], &[u32::MAX; 2]).unwrap();
        // It names a code no body shows, and the other screen.
        let mut theme = art
            .theme("gp-art 1\npic sprite-player-01 a.png 32 32 0 0\n")
            .unwrap();
        let t = art
            .themed(&mut theme, &w, &[true, true], &[u32::MAX; 2])
            .unwrap();
        assert!(t.picture.pens == plain.pens && t.picture.palette == plain.palette);
        assert_eq!(t.picture.drawn, plain.drawn);
        assert!(t.frame.draws.is_empty() && t.board_at == 0);
        assert_eq!(t.frame.size, SCREEN_SIZE);
        // A theme for another game's pictures is refused, as on the NES.
        assert!(
            art.theme("gp-art 1\npic frame-1-0.png a.png 8 8 0 0\n")
                .is_err()
        );
    }

    /// A themed sprite is drawn by the theme where the board drew it, and
    /// left out of the board's picture; an unthemed one is unchanged.
    #[test]
    fn a_themed_sprite_is_the_themes_where_the_boards_was() {
        let dir = tempfile::tempdir().unwrap();
        let (_, art, w) = two_bodies(dir.path());
        let plain = art.draw(&w, &[true, true], &[u32::MAX; 2]).unwrap();
        let mut theme = art
            .theme("gp-art 1\npic sprite-player-02 mug.png 40 40 -4 -4\n")
            .unwrap();
        let t = art
            .themed(&mut theme, &w, &[true, true], &[u32::MAX; 2])
            .unwrap();
        let u = gp_present::art::ART_SUB;
        // The board's picture first (the screen is the board's), then the art
        // at the first body's rectangle (100, 100), moved and sized.
        assert_eq!(t.board_at, 0);
        assert_eq!(t.frame.draws.len(), 1);
        let d = &t.frame.draws[0];
        assert_eq!(d.src, gp_present::art::Src::Art(0));
        assert_eq!([d.x, d.y, d.w, d.h], [96 * u, 96 * u, 40 * u, 40 * u]);
        let at = |p: &Picture, x: usize, y: usize| p.pens[y * video::WIDTH + x];
        // Where the first sprite was, the screen's tile.
        assert_eq!(at(&plain, 100, 100), 0x03 | video::SPRITE_BIT);
        assert_eq!(at(&t.picture, 100, 100), 16 + 5);
        // The second body (picture 3, at 300) is the board's, unchanged.
        let second = plain.drawn[1].rect;
        assert_eq!(second, [300, 100, 32, 32]);
        assert_eq!(at(&t.picture, 300, 100), 0x02 | video::SPRITE_BIT);
        for y in 0..video::HEIGHT {
            for x in 0..video::WIDTH {
                if !(100..132).contains(&x) || !(100..132).contains(&y) {
                    assert_eq!(at(&t.picture, x, y), at(&plain, x, y), "{x},{y}");
                }
            }
        }
        // `drawn` still lists both, as the frame has them.
        assert_eq!(t.picture.drawn, plain.drawn);
    }

    /// A themed screen: the board's picture keeps only its sprites, over
    /// the theme's screen, and a body the theme names nothing for stays.
    #[test]
    fn a_themed_screen_is_drawn_under_the_boards_sprites() {
        let dir = tempfile::tempdir().unwrap();
        let (_, art, w) = two_bodies(dir.path());
        let mut theme = art
            .theme("gp-art 1\nscale 2\npic stage saloon.png 512 480 0 0\n")
            .unwrap();
        let t = art
            .themed(&mut theme, &w, &[true, true], &[u32::MAX; 2])
            .unwrap();
        assert_eq!(t.frame.scale, 2);
        assert_eq!(t.frame.draws.len(), 1);
        assert_eq!(t.frame.draws[0].of, gp_present::art::Of::Level);
        assert_eq!(t.board_at, 1);
        let at = |x: usize, y: usize| t.picture.pens[y * video::WIDTH + x];
        assert_eq!(at(0, 0), TRANSPARENT);
        assert_eq!(at(100, 100), 0x03 | video::SPRITE_BIT);
        assert_eq!(at(300, 100), 0x02 | video::SPRITE_BIT);
    }

    /// Under a themed screen, a board sprite's mask (it draws nothing and
    /// hides the sprites under it, as a door hides the guests coming out
    /// of it) shows the theme's room, never the board's screen.
    #[test]
    fn a_sprites_mask_shows_the_themes_room() {
        let dir = tempfile::tempdir().unwrap();
        let (_, art, mut w) = two_bodies(dir.path());
        w.bodies.truncate(1);
        w.bodies[0].ext[1] = 0x0103;
        let mut theme = art
            .theme("gp-art 1\nscale 2\npic stage saloon.png 512 480 0 0\n")
            .unwrap();
        let t = art.themed(&mut theme, &w, &[true], &[u32::MAX]).unwrap();
        assert!(t.picture.pens.iter().all(|p| *p == TRANSPARENT));
    }

    /// A body the board draws in the colours the screen already has under
    /// it shows nothing, so a theme naming its picture doesn't draw it
    /// either; drawn in other colours, the theme does.
    #[test]
    fn a_body_the_board_shows_nothing_of_isnt_themed() {
        let dir = tempfile::tempdir().unwrap();
        let (_, art, mut w) = two_bodies(dir.path());
        w.bodies.truncate(1);
        let mut theme = art
            .theme("gp-art 1\nscale 2\npic sprite-player-00 tap.png 32 32 0 0\npic sprite-player-03 hero.png 32 32 0 0\n")
            .unwrap();
        let arts = |t: &Themed| {
            t.frame
                .draws
                .iter()
                .filter(|d| matches!(d.src, gp_present::art::Src::Art(_)))
                .count()
        };
        // Picture 0 in bank 1 (flags 2): the screen's own pen 21 at every pixel.
        w.bodies[0].ext[1] = 0x0002;
        let t = art.themed(&mut theme, &w, &[true], &[u32::MAX]).unwrap();
        assert_eq!(arts(&t), 0);
        assert_eq!(
            t.picture.pens,
            art.draw(&w, &[false], &[u32::MAX]).unwrap().pens
        );
        // Picture 3: value 2, other than the screen's.
        w.bodies[0].ext[1] = 0x0302;
        let t = art.themed(&mut theme, &w, &[true], &[u32::MAX]).unwrap();
        assert_eq!(arts(&t), 1);
        // Picture 1: only mask pixels, which draw nothing but hide the
        // sprites under them, so it shows: the board keeps it.
        let mut theme = art
            .theme("gp-art 1\nscale 2\npic stage saloon.png 512 480 0 0\n")
            .unwrap();
        w.bodies[0].ext[1] = 0x0102;
        let t = art.themed(&mut theme, &w, &[true], &[u32::MAX]).unwrap();
        let plain = art.draw(&w, &[true], &[u32::MAX]).unwrap();
        assert_eq!(t.picture.drawn.len(), plain.drawn.len());
        let bare = art.draw(&w, &[false], &[u32::MAX]);
        assert!(!art.unseen(&w, &plain, bare.as_ref())[0]);
    }

    /// `#+ recolor` gives a picture's pixels of a colour number the theme's
    /// colour, where the board drew them, and leaves the rest the board's.
    #[test]
    fn a_recolor_gives_a_pictures_colour_the_themes() {
        let dir = tempfile::tempdir().unwrap();
        let (_, art, mut w) = two_bodies(dir.path());
        w.bodies.truncate(1);
        // Picture 3: all value 2.
        w.bodies[0].ext[1] = 0x0302;
        let mut theme = art
            .theme("gp-art 1\nscale 2\npic stage saloon.png 512 480 0 0\n#+ recolor sprite-player-03 2=c08040 7=ffffff\n")
            .unwrap();
        assert_eq!(theme.art.rules.recolor[0].build, "sprite-player-03");
        assert_eq!(
            theme.art.rules.recolor[0].colors,
            vec![(2, Some([0xc0, 0x80, 0x40])), (7, Some([255, 255, 255]))]
        );
        let t = art.themed(&mut theme, &w, &[true], &[u32::MAX]).unwrap();
        assert_eq!(t.picture.recolor.len(), 32 * 32);
        assert!(
            t.picture
                .recolor
                .iter()
                .all(|(_, c)| *c == Some([0xc0, 0x80, 0x40]))
        );
        // `2=-`: drawn as nothing.
        let mut theme = art
            .theme("gp-art 1\nscale 2\npic stage saloon.png 512 480 0 0\n#+ recolor sprite-player-03 2=-\n")
            .unwrap();
        let t = art.themed(&mut theme, &w, &[true], &[u32::MAX]).unwrap();
        assert!(t.picture.recolor.iter().all(|(_, c)| c.is_none()));
        // With the screen left the board's (no picture for it), nothing
        // shows the board's screen there: tile 1 in bank 1, pen 21.
        let mut theme = art
            .theme("gp-art 1\nscale 2\npic sprite-player-02 x.png 32 32 0 0\n#+ recolor sprite-player-03 2=-\n")
            .unwrap();
        let t = art.themed(&mut theme, &w, &[true], &[u32::MAX]).unwrap();
        let screen = art.draw(&w, &[false], &[u32::MAX]).unwrap();
        let v = screen.palette[usize::from(screen.pens[0] & 0x3f)];
        assert!(!t.picture.recolor.is_empty());
        assert!(
            t.picture
                .recolor
                .iter()
                .all(|(_, c)| *c == Some([(v >> 16) as u8, (v >> 8) as u8, v as u8]))
        );
        // `box=0,0,7,3`: only those of its pixels, 8 by 4.
        let mut theme = art
            .theme("gp-art 1\nscale 2\npic stage saloon.png 512 480 0 0\n#+ recolor sprite-player-03 box=0,0,7,3 2=-\n")
            .unwrap();
        let t = art.themed(&mut theme, &w, &[true], &[u32::MAX]).unwrap();
        assert_eq!(t.picture.recolor.len(), 32);
        // Two rules for one picture, each in its box: both hold.
        let mut theme = art
            .theme("gp-art 1\nscale 2\npic stage saloon.png 512 480 0 0\n#+ recolor sprite-player-03 box=0,0,7,3 2=-\n#+ recolor sprite-player-03 box=0,4,7,7 2=ffffff\n")
            .unwrap();
        let t = art.themed(&mut theme, &w, &[true], &[u32::MAX]).unwrap();
        assert_eq!(t.picture.recolor.len(), 64);
        // Another picture: no rule names it.
        w.bodies[0].ext[1] = 0x0202;
        let t = art.themed(&mut theme, &w, &[true], &[u32::MAX]).unwrap();
        assert!(t.picture.recolor.is_empty());
    }

    /// A board game's model has no palettes of its own, so its look is
    /// drawn by tile kind; its levels still keep the sprite table's order,
    /// which ranks the board's sprites (a body takes its entry from it).
    #[test]
    fn a_board_games_look_keeps_the_sprite_order() {
        let dir = tempfile::tempdir().unwrap();
        lift(dir.path());
        let text = MODEL.replace(
            "[[levels.main.spawn]]",
            "[levels.main.present.sprite_order.player]\nat = 87\nstep = 1\n\n[[levels.main.spawn]]",
        );
        let model = Model::parse(&text).unwrap();
        let lifted = gp_interp::Lifted::read_dir(dir.path(), &model).unwrap();
        let spec = gp_interp::compile::compile(&model, &lifted).unwrap();
        let art = crate::present::art(&model, &spec, dir.path()).unwrap();
        let player = spec.kinds.iter().position(|k| k.name == "player").unwrap();
        let o = art.present.levels[0].order[player].as_ref().unwrap();
        assert_eq!((o.at, o.step), (87, 1));
    }

    /// A counter's digit drawn by the theme: the cell is the screen's own
    /// in the board's picture, and the theme's digit goes there.
    #[test]
    fn a_themed_counter_digit_is_drawn_in_its_cell() {
        let dir = tempfile::tempdir().unwrap();
        let (_, spec, art) = with_looks_and_counters(dir.path());
        let c = |n: &str| spec.counters.index_of(n).unwrap();
        let mut w = World {
            counters: vec![0; spec.counters.len()],
            ..Default::default()
        };
        w.counters[c("score")] = 120;
        let plain = art.draw(&w, &[], &[]).unwrap();
        assert!(
            plain
                .cells
                .iter()
                .any(|x| x.name == "counter-score-2" && x.cell == 32 + 4)
        );
        let mut theme = art
            .theme("gp-art 1\npic counter-score-2 two.png 16 16 0 0\n")
            .unwrap();
        let t = art.themed(&mut theme, &w, &[], &[]).unwrap();
        let u = gp_present::art::ART_SUB;
        assert_eq!(t.frame.draws.len(), 1);
        let d = &t.frame.draws[0];
        assert_eq!([d.x, d.y], [4 * 16 * u, 16 * u]);
        // The digit 2's cell (4, 1): the screen's tile 1 in bank 1 now, the
        // digit 1 beside it still the board's.
        let at = |p: &Picture, x: usize, y: usize| p.pens[y * video::WIDTH + x];
        assert_eq!(at(&t.picture, 4 * 16, 16), 16 + 5);
        assert_eq!(at(&t.picture, 3 * 16, 16), at(&plain, 3 * 16, 16));
    }

    /// `#+ counter_off COUNTER when ...` leaves a counter's cells to the
    /// screen, drawn by neither the theme nor the board, while its tests
    /// hold (a second player's score while there is none), and draws them
    /// as before when they don't. A rule naming a counter the model hasn't
    /// got is passed over; one that can't be read is skipped.
    #[test]
    fn a_counter_the_theme_turns_off_is_drawn_by_neither() {
        let dir = tempfile::tempdir().unwrap();
        let (_, spec, art) = with_looks_and_counters(dir.path());
        let c = |n: &str| spec.counters.index_of(n).unwrap();
        let mut w = World {
            counters: vec![0; spec.counters.len()],
            ..Default::default()
        };
        w.counters[c("score")] = 120;
        let plain = art.draw(&w, &[], &[]).unwrap();
        let rules = "gp-art 1\npic counter-score-2 two.png 16 16 0 0\n\
                     #+ counter_off score when lives == 0\n\
                     #+ counter_off nothing_like_it\n\
                     #+ counter_off score when lives =\n";
        let mut theme = art.theme(rules).unwrap();
        assert_eq!(theme.art.rules.counter_off.len(), 2);
        assert_eq!(theme.art.rules.skipped.len(), 1);
        let at = |p: &Picture, x: usize, y: usize| p.pens[y * video::WIDTH + x];
        // No lives: the score's three digits are the screen's tile 1 in
        // bank 1, and the theme draws no digit.
        let t = art.themed(&mut theme, &w, &[], &[]).unwrap();
        assert!(t.frame.draws.is_empty());
        for x in 3..=5 {
            assert_eq!(at(&t.picture, x * 16, 16), 16 + 5, "cell ({x}, 1)");
        }
        assert_ne!(at(&plain, 3 * 16, 16), 16 + 5);
        // Lives again: the theme's 2 and the board's 1 and 0.
        w.counters[c("lives")] = 3;
        w.frame = 1;
        let t = art.themed(&mut theme, &w, &[], &[]).unwrap();
        assert_eq!(t.frame.draws.len(), 1);
        assert_eq!(at(&t.picture, 3 * 16, 16), at(&plain, 3 * 16, 16));
        assert_eq!(at(&t.picture, 5 * 16, 16), at(&plain, 5 * 16, 16));
    }

    /// `#+ text_center` moves a text line so its letters sit centred
    /// between the rule's columns (only in its rows when it names them); `#+ text_say` words a text the theme's
    /// way, in its own cells, a space drawn by neither theme nor board.
    #[test]
    fn a_theme_centres_and_words_a_text() {
        let dir = tempfile::tempdir().unwrap();
        let (_, spec, art) = with_looks_and_counters(dir.path());
        let mut w = World {
            counters: vec![0; spec.counters.len()],
            ..Default::default()
        };
        w.counters[spec.counters.index_of("bar").unwrap()] = 2;
        let font = "gp-art 1\npic letter-H h.png 16 16 0 0\npic letter-I i.png 16 16 0 0\npic letter-J j.png 16 16 0 0\n";
        let u = gp_present::art::ART_SUB;
        let at = |t: &Themed| -> Vec<(i64, i64)> {
            t.frame.draws.iter().map(|d| (d.x / u, d.y / u)).collect()
        };
        // "HI J" from column 4: letters 4 to 7, centred between 0 and 31.
        let mut theme = art.theme(&format!("{font}#+ text_center 0 31\n")).unwrap();
        let t = art.themed(&mut theme, &w, &[], &[]).unwrap();
        let shift = (31 - (4 + 7)) * 8;
        assert!(at(&t).contains(&(4 * 16 + shift, 5 * 16)), "{:?}", at(&t));
        // With rows given, a line outside them stays where the game put it
        // (the text is in row 5); one inside them moves.
        let mut theme = art
            .theme(&format!("{font}#+ text_center 0 31 6 9\n"))
            .unwrap();
        let t = art.themed(&mut theme, &w, &[], &[]).unwrap();
        assert!(at(&t).contains(&(4 * 16, 5 * 16)), "{:?}", at(&t));
        let mut theme = art
            .theme(&format!("{font}#+ text_center 0 31 3 5\n"))
            .unwrap();
        let t = art.themed(&mut theme, &w, &[], &[]).unwrap();
        assert!(at(&t).contains(&(4 * 16 + shift, 5 * 16)), "{:?}", at(&t));
        // Worded "IJ": I and J in the first two cells, the rest drawn by
        // neither (the board's own letters left out).
        let mut theme = art.theme(&format!("{font}#+ text_say words IJ\n")).unwrap();
        let t = art.themed(&mut theme, &w, &[], &[]).unwrap();
        let files: Vec<String> = t
            .frame
            .draws
            .iter()
            .filter_map(|d| match d.src {
                gp_present::art::Src::Art(f) => Some(theme.art.files[f as usize].clone()),
                _ => None,
            })
            .collect();
        assert_eq!(files, ["i.png", "j.png"]);
        let px = |x: usize, y: usize| t.picture.pens[y * video::WIDTH + x];
        let plain = art.draw(&w, &[], &[]).unwrap();
        // The H's cell isn't the board's H any more.
        assert_ne!(
            px(4 * 16 + 5, 5 * 16 + 7),
            plain.pens[(5 * 16 + 7) * video::WIDTH + 4 * 16 + 5]
        );
    }

    /// A screen two looks show has a name for each look, and a theme that
    /// draws one of those stages draws it on that look only.
    #[test]
    fn a_screen_shown_by_two_looks_has_a_stage_each() {
        let dir = tempfile::tempdir().unwrap();
        let (_, spec, art) = with_looks_and(
            dir.path(),
            "[[levels.main.present.board.looks]]\nwhen = [{ counter = \"bar\", cmp = \"==\", value = 3 }]\nscreen = \"stage2\"\n\n",
        );
        let names: Vec<String> = art.names().into_iter().map(|(n, _)| n).collect();
        assert!(
            names.contains(&"stage2-1".to_owned()) && names.contains(&"stage2-2".to_owned()),
            "{names:?}"
        );
        let mut w = World {
            counters: vec![0; spec.counters.len()],
            ..Default::default()
        };
        let bar = spec.counters.index_of("bar").unwrap();
        let text = "gp-art 1\npic stage2 any.png 512 480 0 0\npic stage2-2 two.png 512 480 0 0\n";
        let file = |w: &World| {
            let mut theme = art.theme(text).unwrap();
            let t = art.themed(&mut theme, w, &[], &[]).unwrap();
            t.frame
                .draws
                .iter()
                .find_map(|d| match d.src {
                    gp_present::art::Src::Art(f) => Some(theme.art.files[f as usize].clone()),
                    _ => None,
                })
                .unwrap()
        };
        w.counters[bar] = 3;
        let first = file(&w);
        w.counters[bar] = 1;
        let second = file(&w);
        assert_ne!(first, second);
        assert!([first.as_str(), second.as_str()].contains(&"two.png"));
    }

    /// Two texts on the same cells (one message giving way to the next):
    /// a cell lists only the letter written last, as the board shows it.
    #[test]
    fn a_cell_written_twice_lists_its_last_letter() {
        let dir = tempfile::tempdir().unwrap();
        let (_, spec, art) = with_looks_and_counters(dir.path());
        let mut w = World {
            counters: vec![0; spec.counters.len()],
            ..Default::default()
        };
        w.counters[spec.counters.index_of("bar").unwrap()] = 2;
        let level = art.levels[0].as_ref().unwrap();
        let mut twice = level.clone();
        let again = twice.texts[0].clone();
        twice.texts.push(Text {
            cells: again
                .cells
                .iter()
                .map(|&(c, _)| (c, [b'Q', 0x30]))
                .collect(),
            ..again
        });
        let (_, cells) = twice.cells_with(&level.look(&w).videoram, &w, None);
        let at = 5 * 32 + 4;
        let here: Vec<&str> = cells
            .iter()
            .filter(|c| c.cell == at)
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(here, ["letter-Q"]);
    }

    /// A later text's space blanks an earlier text's letter in a themed
    /// draw too: the cells a theme draws are left out of the video RAM by
    /// the order of the last writes, so an earlier write to a cell is never
    /// laid there when the later one is the theme's or a space.
    #[test]
    fn a_later_space_blanks_an_earlier_letter_when_a_theme_draws_the_letters() {
        let dir = tempfile::tempdir().unwrap();
        let (_, spec, art) = with_looks_and_counters(dir.path());
        let mut w = World {
            counters: vec![0; spec.counters.len()],
            ..Default::default()
        };
        w.counters[spec.counters.index_of("bar").unwrap()] = 2;
        let level = art.levels[0].as_ref().unwrap();
        let mut twice = level.clone();
        let again = twice.texts[0].clone();
        let at = again.cells[0].0;
        // The same line again, later, with a space where its first letter was.
        twice.texts.push(Text {
            cells: again
                .cells
                .iter()
                .enumerate()
                .map(|(k, &(c, w))| (c, if k == 0 { [b' ', 0x30] } else { w }))
                .collect(),
            ..again
        });
        let ram = &level.look(&w).videoram;
        let (_, cells) = twice.cells_with(ram, &w, None);
        let space = letter_name([b' ', 0x30]);
        // A theme with a picture for every letter and none for a space.
        let omit: Vec<bool> = cells.iter().map(|c| c.name != space).collect();
        let (v, drawn) = twice.cells_with(ram, &w, Some(&omit));
        assert_eq!(drawn, cells);
        assert_eq!(
            &v[at * 2..at * 2 + 2],
            &[b' ', 0x30],
            "the space, not the earlier letter"
        );
        // And each letter the theme draws is left as the screen has it.
        for c in cells.iter().filter(|c| c.name != space) {
            assert_eq!(
                &v[c.cell * 2..c.cell * 2 + 2],
                &ram[c.cell * 2..c.cell * 2 + 2],
                "{}",
                c.name
            );
        }
    }

    /// A text's letters are named by the character each stands for, so one
    /// picture a letter redraws every text written with it; a theme that
    /// draws a letter draws it in that letter's cell.
    #[test]
    fn a_theme_draws_a_texts_letters_by_name() {
        assert_eq!(letter_name([b'H', 0x30]), "letter-H");
        assert_eq!(letter_name([b'7', 0x30]), "letter-7");
        assert_eq!(letter_name([b'.', 0x30]), "letter-x02e");
        assert_eq!(letter_name([b'H', 0x31]), "letter-x148");
        let dir = tempfile::tempdir().unwrap();
        let (_, spec, art) = with_looks_and_counters(dir.path());
        let names: Vec<String> = art.names().into_iter().map(|(n, _)| n).collect();
        assert!(names.contains(&"letter-H".to_owned()) && names.contains(&"letter-J".to_owned()));
        let mut w = World {
            counters: vec![0; spec.counters.len()],
            ..Default::default()
        };
        w.counters[spec.counters.index_of("bar").unwrap()] = 2;
        let mut theme = art
            .theme("gp-art 1\npic letter-H h.png 16 16 0 0\n")
            .unwrap();
        let t = art.themed(&mut theme, &w, &[], &[]).unwrap();
        let u = gp_present::art::ART_SUB;
        assert!(
            t.frame
                .draws
                .iter()
                .any(|d| [d.x, d.y] == [4 * 16 * u, 5 * 16 * u])
        );
    }

    /// A one-digit counter that holds a letter's character code and draws
    /// it through that letter's tile (a high score's initials: 65 for
    /// A, plus -65, a table of A to J) is named by the letter, as a text's
    /// letters are, so a theme's letter pictures draw it; one whose value
    /// isn't the tile's code keeps its digit names.
    #[test]
    fn a_counter_of_letters_is_named_by_its_letters() {
        let dir = tempfile::tempdir().unwrap();
        let (_, spec, mut art) = with_looks_and_counters(dir.path());
        let bar = spec.counters.index_of("bar").unwrap();
        let mut codes = [0u8; 10];
        for (d, c) in codes.iter_mut().enumerate() {
            *c = b'A' + d as u8;
        }
        let letters = Number {
            counter: bar,
            name: "table_1_a".into(),
            plus: -65,
            cell: 2 * 32 + 10,
            digits: 1,
            codes,
            high: 0x30,
            zeros: false,
            when: Vec::new(),
        };
        // The same tiles counted from 0: not the letters' codes.
        let digits = Number {
            name: "level".into(),
            plus: 0,
            cell: 3 * 32 + 10,
            ..letters.clone()
        };
        {
            let level = art.levels[0].as_mut().unwrap();
            level.numbers.push(letters);
            level.numbers.push(digits);
        }
        let mut w = World {
            counters: vec![0; spec.counters.len()],
            ..Default::default()
        };
        w.counters[bar] = i64::from(b'C');
        let level = art.levels[0].as_ref().unwrap();
        let (_, cells) = level.cells_with(&level.look(&w).videoram, &w, None);
        let at = |cell: usize| {
            cells
                .iter()
                .find(|c| c.cell == cell)
                .map(|c| c.name.clone())
        };
        assert_eq!(at(2 * 32 + 10).as_deref(), Some("letter-C"));
        let names: Vec<String> = art.names().into_iter().map(|(n, _)| n).collect();
        for n in ["letter-A", "letter-J", "counter-level-0", "counter-level-9"] {
            assert!(names.contains(&n.to_owned()), "{n}: {names:?}");
        }
        assert!(
            !names.iter().any(|n| n.starts_with("counter-table_1_a")),
            "{names:?}"
        );
        // The theme's letter C draws the initial's cell.
        let mut theme = art
            .theme("gp-art 1\npic letter-C c.png 16 16 0 0\n")
            .unwrap();
        let t = art.themed(&mut theme, &w, &[], &[]).unwrap();
        let u = gp_present::art::ART_SUB;
        assert!(
            t.frame
                .draws
                .iter()
                .any(|d| [d.x, d.y] == [10 * 16 * u, 2 * 16 * u])
        );
    }

    /// A theme that draws other cells, and none of a text's letters,
    /// leaves the letters the board's: the words still show.
    #[test]
    fn a_text_stays_the_boards_under_a_theme_of_other_cells() {
        let dir = tempfile::tempdir().unwrap();
        let (_, spec, art) = with_looks_and_counters(dir.path());
        let c = |n: &str| spec.counters.index_of(n).unwrap();
        let mut w = World {
            counters: vec![0; spec.counters.len()],
            ..Default::default()
        };
        w.counters[c("score")] = 120;
        w.counters[c("bar")] = 2;
        let plain = art.draw(&w, &[], &[]).unwrap();
        assert!(plain.cells.iter().any(|x| x.name.starts_with("letter-")));
        let mut theme = art
            .theme("gp-art 1\npic counter-score-2 two.png 16 16 0 0\n")
            .unwrap();
        let t = art.themed(&mut theme, &w, &[], &[]).unwrap();
        let at = |p: &Picture, x: usize, y: usize| p.pens[y * video::WIDTH + x];
        for col in 4..8 {
            for (dx, dy) in [(0, 0), (5, 7), (11, 3)] {
                let (x, y) = (col * 16 + dx, 5 * 16 + dy);
                assert_eq!(at(&t.picture, x, y), at(&plain, x, y), "cell ({col}, 5)");
            }
        }
    }

    /// A `timed` rule counts the frames a body has shown its picture, one
    /// step a game frame however often the frame is drawn, and a seek
    /// starts it over.
    #[test]
    fn a_timed_rule_counts_game_frames() {
        let dir = tempfile::tempdir().unwrap();
        let (_, art, mut w) = two_bodies(dir.path());
        let mut theme = art
            .theme("gp-art 1\npic sprite-player-02 b.png 32 32 0 0\ntimed sprite-player-02 0 2 a.png 32 32 0 0\n")
            .unwrap();
        let file = |theme: &mut BoardTheme, w: &World| {
            let t = art.themed(theme, w, &[true, true], &[u32::MAX; 2]).unwrap();
            match t.frame.draws[0].src {
                gp_present::art::Src::Art(f) => theme.art.files[f as usize].clone(),
                s => panic!("{s:?}"),
            }
        };
        let mut seen = Vec::new();
        for f in 0..4 {
            w.frame = f;
            seen.push(file(&mut theme, &w));
            // Drawn twice: the same frame.
            assert_eq!(file(&mut theme, &w), *seen.last().unwrap());
        }
        assert_eq!(seen, ["a.png", "a.png", "b.png", "b.png"]);
        w.frame = 40;
        assert_eq!(file(&mut theme, &w), "a.png");
    }

    /// A room built from the theme's pieces on the screen's own cells: a
    /// counter where the lift's screen has cells of colour bank 2, the
    /// back wall on the rest, drawn as the stage under the board's
    /// sprites, and the board's own screen left out.
    #[test]
    fn a_room_is_built_on_the_screens_own_cells() {
        let dir = tempfile::tempdir().unwrap();
        let (_, _, w) = two_bodies(dir.path());
        // Row 10, columns 4 to 19: tile 0 in colour bank 2.
        let mut rows = Vec::new();
        for y in 0..30 {
            let row: Vec<&str> = (0..32)
                .map(|x| {
                    if y == 10 && (4..20).contains(&x) {
                        "0x2000"
                    } else {
                        "0x1001"
                    }
                })
                .collect();
            rows.push(format!("[{}]", row.join(", ")));
        }
        std::fs::write(
            dir.path().join("stage.toml"),
            format!("rows = [{}]\n", rows.join(",\n")),
        )
        .unwrap();
        let model = Model::parse(MODEL).unwrap();
        let lifted = gp_interp::Lifted::read_dir(dir.path(), &model).unwrap();
        let spec = gp_interp::compile::compile(&model, &lifted).unwrap();
        let art = BoardArt::load(&model, &spec, dir.path()).unwrap().unwrap();
        let cells = art.screen_cells("stage").unwrap();
        let wall = &cells.cells[0];
        assert_eq!((wall.tile, wall.bank, wall.colors.len()), (1, 1, 64));
        let c = art.levels[0].as_ref().unwrap().base.palette[16 + 5];
        assert_eq!(wall.colors[0], [(c >> 16) as u8, (c >> 8) as u8, c as u8]);
        assert_eq!(cells.cells[10 * 32 + 4].bank, 2);
        assert!(art.screen_cells("nothing").is_none());
        let text = "gp-art 1\nscale 2\nstage 2 0 0 0 0 0 0\n\
                    stage_piece top top.png 64 32\nstage_piece brick brick.png 64 64\n\
                    #+ room stage\n#+ room_cells counter bank=2\n#+ room_cells back !counter\n\
                    #+ room_cells sky b>r+200\n\
                    #+ room_fill back brick\n#+ room_run counter - top -\n#+ room_fill sky brick\n\
                    #+ room_front 0 0 3 2\n";
        let mut theme = art.theme(text).unwrap();
        assert!(theme.art.rules.skipped.is_empty());
        assert_eq!(theme.rooms.len(), 1);
        assert_eq!(theme.rooms[0].screen, "stage");
        // The back wall, the counter, and nothing of the sky.
        assert_eq!(theme.rooms[0].cells, [960 - 16, 16, 0]);
        // The check's list: the sky's rule, line 12, on the screen shown.
        assert_eq!(
            theme.empty_rooms(&|_| true),
            [(12, "sky".to_owned(), vec!["stage".to_owned()])]
        );
        assert!(theme.empty_rooms(&|s| s != "stage").is_empty());
        let t = art
            .themed(&mut theme, &w, &[true, true], &[u32::MAX; 2])
            .unwrap();
        let pic = theme.rooms[0].pic;
        assert_eq!(t.frame.draws[0].src, gp_present::art::Src::Stage(pic));
        assert_eq!(t.frame.draws[0].of, gp_present::art::Of::Level);
        assert_eq!(t.board_at, 1);
        // `#+ room_front 0 0 3 2`: the room again after the board's
        // picture (its sprites), cut to columns 0 to 3 and rows 0 to 2.
        let front = &t.frame.draws[1];
        assert_eq!(front.src, gp_present::art::Src::Stage(pic));
        let u = gp_present::art::ART_SUB;
        assert_eq!(front.clip, Some([0, 0, 4 * 16 * u, 3 * 16 * u]));
        let at = |x: usize, y: usize| t.picture.pens[y * video::WIDTH + x];
        assert_eq!(at(0, 0), TRANSPARENT);
        assert_eq!(at(100, 100), 0x03 | video::SPRITE_BIT);
        // The counter's run: the top (64 by 32, so 64 wide at the row's 32
        // stage pixels) across columns 4 to 20, cut there.
        let st = theme.art.stage(pic).unwrap();
        let tops: Vec<_> = st
            .draws
            .iter()
            .filter(|d| d.src == gp_present::art::stage::StageSrc::Piece(0))
            .collect();
        assert_eq!(tops.len(), 8);
        assert!(
            tops.iter()
                .all(|d| d.clip == Some([4 * 32, 10 * 32, 16 * 32, 32]))
        );
    }

    #[test]
    fn a_model_with_no_board_art_has_none() {
        let dir = tempfile::tempdir().unwrap();
        let model = Model::parse(
            &MODEL
                .replace("[entities.player.board_sprite]\nslot = \"sprites\"\npicture = { word = 1, byte = 1 }\nflags = { word = 1 }\n", "")
                .replace("[levels.main.present.board]\npalette = \"colors\"\ntiles = \"tiles\"\nscreen = \"stage\"\n", ""),
        )
        .unwrap();
        assert!(!names_board(&model));
        lift(dir.path());
        let lifted = gp_interp::Lifted::read_dir(dir.path(), &model).unwrap();
        let spec = gp_interp::compile::compile(&model, &lifted).unwrap();
        assert!(BoardArt::load(&model, &spec, dir.path()).unwrap().is_none());
    }
}
