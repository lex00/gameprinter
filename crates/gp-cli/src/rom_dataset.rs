//! The ROM dataset (docs/formats/rom-dataset.md): what stage 0 extracts
//! from one ROM, as tables a data scientist loads with pandas, DuckDB or
//! Hugging Face `datasets`. One fixed schema for every game, so the
//! datasets of many ROMs union into one table each. Every row names the
//! ROM (`rom`, its headerless SHA-256), where it came from (`src`), the
//! rule that made it (`rule`) and the tool (`tool`).
//!
//! [`TABLES`] is the schema: the Parquet files, the card, Croissant and
//! `schema/rom-dataset.json` are all made from it. Nothing here reads a
//! ROM but to check that the written files hold none of it.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};

use crate::dataset_tables::{self as ds, Col, Field, Table};

/// The format's version: its tables and columns. A column added is minor,
/// one removed or changed in meaning is major.
pub(crate) const FORMAT: &str = "0.1.0";

/// A column's type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ty {
    /// Text, null allowed.
    Str,
    /// A whole number, never null.
    Int,
    /// A whole number, null allowed.
    OptInt,
    Bool,
    /// A float, null allowed.
    Num,
}

pub(crate) struct ColSpec {
    pub name: &'static str,
    pub ty: Ty,
    pub unit: &'static str,
    pub about: &'static str,
}

pub(crate) struct TableSpec {
    pub name: &'static str,
    pub about: &'static str,
    pub key: &'static [&'static str],
    pub cols: &'static [ColSpec],
}

const fn c(name: &'static str, ty: Ty, unit: &'static str, about: &'static str) -> ColSpec {
    ColSpec {
        name,
        ty,
        unit,
        about,
    }
}

const ROM: ColSpec = c(
    "rom",
    Ty::Str,
    "",
    "The ROM's headerless SHA-256 (PRG then CHR), the same in every table: the column datasets of many ROMs are joined on",
);
const SRC: ColSpec = c(
    "src",
    Ty::Str,
    "",
    "Where the row came from: `read:BANK:ADDR` (an instruction the reader read; the bank is left out on an unbanked board), `run:RECIPE@FRAME` (a run's recipe hash and frame), `asset-map:ENTRY`, or a JSON list of these",
);
const RULE: ColSpec = c(
    "rule",
    Ty::Str,
    "",
    "The rule of the tool that made the row",
);
const TOOL: ColSpec = c("tool", Ty::Str, "", "The tool and its version (`gp 0.1.0`)");

/// The tables, in the order the card lists them.
pub(crate) const TABLES: &[TableSpec] = &[
    TableSpec {
        name: "rom",
        about: "One row: the ROM and the extraction that made the dataset",
        key: &["rom"],
        cols: &[
            ROM,
            c("mapper", Ty::Int, "", "The iNES mapper number"),
            c(
                "board",
                Ty::Str,
                "",
                "The board as the reader models it (nrom, uxrom, mmc1, mmc3, or another name for one it reads only in part)",
            ),
            c("prg_bytes", Ty::Int, "bytes", "PRG ROM size"),
            c(
                "chr_bytes",
                Ty::Int,
                "bytes",
                "CHR ROM size (0 for CHR RAM)",
            ),
            c(
                "asset_map",
                Ty::Str,
                "",
                "BLAKE3 of the asset map read beside the ROM; null when the ROM alone was read",
            ),
            c(
                "runs",
                Ty::Int,
                "count",
                "Runs of the ROM the extraction made (the runs table)",
            ),
            c("frames", Ty::Int, "frames", "Frames run, over all runs"),
            c("format", Ty::Str, "", "This format's version"),
            TOOL,
        ],
    },
    TableSpec {
        name: "routines",
        about: "One row a routine of the ROM's code: where it is, what calls it, the element it belongs to",
        key: &["rom", "routine"],
        cols: &[
            ROM,
            c(
                "routine",
                Ty::Str,
                "",
                "Its entry as `BANK:ADDR` (hex; the bank left out on an unbanked board): the name every other table uses",
            ),
            c(
                "bank",
                Ty::OptInt,
                "",
                "The PRG bank it is in, null on an unbanked board",
            ),
            c("addr", Ty::Int, "", "Its entry's CPU address"),
            c(
                "size",
                Ty::Int,
                "bytes",
                "Bytes of code from its entry to its last instruction",
            ),
            c(
                "element",
                Ty::Str,
                "",
                "The element it belongs to (the elements table), null if none",
            ),
            c("read", Ty::Bool, "", "The reader read its instructions"),
            c("ran", Ty::Bool, "", "A run entered it"),
            c(
                "callers",
                Ty::Str,
                "",
                "The routines that call or jump to it, a JSON list",
            ),
            c(
                "callees",
                Ty::Str,
                "",
                "The routines it calls or jumps to, a JSON list",
            ),
            SRC,
            RULE,
            TOOL,
        ],
    },
    TableSpec {
        name: "branches",
        about: "One row a conditional branch: the bytes its test reads, the value compared, and which ways runs took it",
        key: &["rom", "pc"],
        cols: &[
            ROM,
            c("pc", Ty::Str, "", "The branch instruction, `BANK:ADDR`"),
            c("routine", Ty::Str, "", "The routine it is in"),
            c("op", Ty::Str, "", "Its mnemonic (beq, bne, bcc, ...)"),
            c(
                "tests",
                Ty::Str,
                "",
                "The RAM places its flags come from, a JSON list (`$05`, `$20,x`)",
            ),
            c(
                "compare",
                Ty::OptInt,
                "",
                "The constant the test compares with, null if it compares with none",
            ),
            c(
                "taken",
                Ty::OptInt,
                "count",
                "Times runs branched here; null if no run reached it",
            ),
            c(
                "not_taken",
                Ty::OptInt,
                "count",
                "Times runs fell through here; null if no run reached it",
            ),
            SRC,
            RULE,
            TOOL,
        ],
    },
    TableSpec {
        name: "ram",
        about: "One row a RAM byte the game writes: its role, the element and slot it belongs to, the quantity it holds, and the routines that read and write it",
        key: &["rom", "addr"],
        cols: &[
            ROM,
            c(
                "addr",
                Ty::Int,
                "",
                "The CPU address ($0000 to $07FF, or $6000 to $7FFF for PRG RAM)",
            ),
            c(
                "role",
                Ty::Str,
                "",
                "state, counter, timer, pointer, input, sound, scratch or level; null if not known (then an open row says why)",
            ),
            c(
                "element",
                Ty::Str,
                "",
                "The element it belongs to, null if none",
            ),
            c(
                "slot",
                Ty::OptInt,
                "",
                "For an element with slots: the slot it is in, from 0",
            ),
            c(
                "quantity",
                Ty::Str,
                "",
                "What it holds, in a vocabulary shared by every game: x, y, speed_x, speed_y, state, timer, facing, kind, flag, count, pointer, buttons, cell, other; null if not known",
            ),
            c(
                "part",
                Ty::Str,
                "",
                "Which byte of a wider value: whole, lo, hi or fraction; null if not known",
            ),
            c("written", Ty::Bool, "", "A run wrote it"),
            c("readers", Ty::Str, "", "Routines that read it, a JSON list"),
            c(
                "writers",
                Ty::Str,
                "",
                "Routines that write it, a JSON list",
            ),
            SRC,
            RULE,
            TOOL,
        ],
    },
    TableSpec {
        name: "elements",
        about: "One row a thing in the game: the player, a group of object slots, the camera, the level, the HUD, the game's own state",
        key: &["rom", "element"],
        cols: &[
            ROM,
            c(
                "element",
                Ty::Str,
                "",
                "Its name, made by the tool from the ROM (`player`, `slots_20`): the name every other table uses",
            ),
            c(
                "kind",
                Ty::Str,
                "",
                "player, slots, camera, level, hud, game, sound, input or other",
            ),
            c(
                "slots",
                Ty::OptInt,
                "count",
                "For a group of object slots, how many",
            ),
            c("routines", Ty::Int, "count", "Routines that belong to it"),
            c("ram", Ty::Int, "count", "RAM bytes that belong to it"),
            c(
                "tile_kinds",
                Ty::Str,
                "",
                "The level tile kinds it acts on, a JSON list, null if none",
            ),
            SRC,
            RULE,
            TOOL,
        ],
    },
    TableSpec {
        name: "states",
        about: "One row a state of a state machine: the byte it lives in and the value that means it",
        key: &["rom", "machine", "value"],
        cols: &[
            ROM,
            c(
                "machine",
                Ty::Str,
                "",
                "The machine's name, made by the tool (`state_05`)",
            ),
            c("byte", Ty::Int, "", "The RAM address that holds the state"),
            c("element", Ty::Str, "", "The element the machine belongs to"),
            c("value", Ty::Int, "", "The byte's value in this state"),
            c(
                "state",
                Ty::Str,
                "",
                "The state's name, made by the tool (`s0`)",
            ),
            c("initial", Ty::Bool, "", "The state the machine starts in"),
            SRC,
            RULE,
            TOOL,
        ],
    },
    TableSpec {
        name: "transitions",
        about: "One row a change of state: from, to, the instruction that writes it and what must hold",
        key: &["rom", "machine", "from_state", "to_state", "pc"],
        cols: &[
            ROM,
            c("machine", Ty::Str, "", "The machine (the states table)"),
            c("from_state", Ty::Str, "", "The state before"),
            c("to_state", Ty::Str, "", "The state after"),
            c(
                "pc",
                Ty::Str,
                "",
                "The instruction that writes the new state, `BANK:ADDR`",
            ),
            c("routine", Ty::Str, "", "The routine it is in"),
            c(
                "guards",
                Ty::Str,
                "",
                "What must hold on the path to it, a JSON list of conditions (`$12 >= #$C0`)",
            ),
            c(
                "after",
                Ty::OptInt,
                "frames",
                "When the change waits on a timer: the frames it waits",
            ),
            SRC,
            RULE,
            TOOL,
        ],
    },
    TableSpec {
        name: "constants",
        about: "One row a value the code uses, at a compare, a load or a ROM table entry: its raw value, what it is, its fixed-point format and its value as a number",
        key: &["rom", "pc", "entry"],
        cols: &[
            ROM,
            c(
                "pc",
                Ty::Str,
                "",
                "The instruction, or the table's first byte, `BANK:ADDR`",
            ),
            c(
                "entry",
                Ty::Int,
                "",
                "The entry of a table, 0 for an instruction's own operand",
            ),
            c("routine", Ty::Str, "", "The routine that uses it"),
            c(
                "op",
                Ty::Str,
                "",
                "The use: cmp, cpx, cpy, lda, ldx, ldy, adc, sbc, and, ora, or table",
            ),
            c(
                "raw",
                Ty::Int,
                "",
                "The value as stored: a byte, or two bytes as one 16-bit number",
            ),
            c(
                "format",
                Ty::Str,
                "",
                "How the code reads it: u8, s8, u16, s16, or a fixed-point form as INT.FRAC bits (`8.8`, `4.4`, `0.8`); null if not known",
            ),
            c(
                "number",
                Ty::Num,
                "",
                "The value as a number in its format (-3.5); null if the format is not known",
            ),
            c(
                "quantity",
                Ty::Str,
                "",
                "What it is, in a vocabulary shared by every game: gravity, jump_speed, max_speed, accel, decel, threshold, period, duration, count, offset, size, limit, kind, mask, other; null if not known (then an open row says why)",
            ),
            c(
                "element",
                Ty::Str,
                "",
                "The element it belongs to, null if none",
            ),
            c(
                "mechanic",
                Ty::Str,
                "",
                "The mechanic part it is a parameter of (docs/mechanics.md: platformer.jump), null if none",
            ),
            c(
                "parameter",
                Ty::Str,
                "",
                "The parameter of that part (gravity), null if none",
            ),
            c(
                "compared",
                Ty::Str,
                "",
                "For a compare: the RAM place compared with it (`$12`), null otherwise",
            ),
            SRC,
            RULE,
            TOOL,
        ],
    },
    TableSpec {
        name: "mechanics",
        about: "One row a mechanic part an element uses, with one of its options (docs/mechanics.md)",
        key: &["rom", "element", "mechanic", "option"],
        cols: &[
            ROM,
            c("element", Ty::Str, "", "The element"),
            c(
                "mechanic",
                Ty::Str,
                "",
                "The mechanic part (platformer.jump)",
            ),
            c(
                "option",
                Ty::Str,
                "",
                "The option, or `*` for the part itself",
            ),
            c(
                "value",
                Ty::Str,
                "",
                "The option's value as JSON, null for the part itself",
            ),
            SRC,
            RULE,
            TOOL,
        ],
    },
    TableSpec {
        name: "facts",
        about: "One row a fact of the game's model the ROM settles, by model path in the tool's own names",
        key: &["rom", "path"],
        cols: &[
            ROM,
            c(
                "path",
                Ty::Str,
                "",
                "The model path (entities.slots_20.walk.speed)",
            ),
            c("value", Ty::Str, "", "The value as JSON"),
            c(
                "number",
                Ty::Num,
                "",
                "The value as a number, null if it isn't one",
            ),
            c(
                "element",
                Ty::Str,
                "",
                "The element it is about, null if none",
            ),
            c(
                "settled_by",
                Ty::Str,
                "",
                "read (the code), run (a run), or asset-map",
            ),
            SRC,
            RULE,
            TOOL,
        ],
    },
    TableSpec {
        name: "open",
        about: "One row a question the ROM leaves: what is not known, the routine and bytes that decide it, and what would settle it",
        key: &["rom", "what", "pc"],
        cols: &[
            ROM,
            c(
                "what",
                Ty::Str,
                "",
                "What is not known: a table and key (`ram:$0507`, `constants:C4CE`) or a model path",
            ),
            c(
                "element",
                Ty::Str,
                "",
                "The element it is about, null if none",
            ),
            c(
                "routine",
                Ty::Str,
                "",
                "The routine that decides it, null if none is known",
            ),
            c(
                "pc",
                Ty::Str,
                "",
                "The instruction that decides it, `BANK:ADDR`; empty if none is known",
            ),
            c(
                "bytes",
                Ty::Str,
                "",
                "The RAM places it depends on, a JSON list",
            ),
            c("why", Ty::Str, "", "Why the tool can't settle it"),
            c(
                "settle_by",
                Ty::Str,
                "",
                "What would settle it: run (a run that reaches the instruction), trace (values from a clip), bank (a bank the reader can't place), name (a choice no code makes)",
            ),
            SRC,
            RULE,
            TOOL,
        ],
    },
    TableSpec {
        name: "links",
        about: "One row a routine's part in an element: the routine that draws it, or one that updates it, found by following the element's sprites back to the code that wrote them",
        key: &["rom", "element", "routine", "link"],
        cols: &[
            ROM,
            c("element", Ty::Str, "", "The element"),
            c("routine", Ty::Str, "", "The routine, `BANK:ADDR`"),
            c(
                "link",
                Ty::Str,
                "",
                "draws (it writes the element's sprites from its state) or updates (it writes the element's position)",
            ),
            c(
                "pc",
                Ty::Str,
                "",
                "For draws: the instruction that writes the sprite's x, `BANK:ADDR`",
            ),
            c(
                "bytes",
                Ty::Str,
                "",
                "The RAM bytes involved, a JSON list: for draws those it reads, for updates those it writes",
            ),
            SRC,
            RULE,
            TOOL,
        ],
    },
    TableSpec {
        name: "estimates",
        about: "One row a value measured from the runs, close but not exact: a fact to refine, with how it was measured, how much the samples spread and what would make it exact",
        key: &["rom", "element", "mechanic", "parameter"],
        cols: &[
            ROM,
            c("element", Ty::Str, "", "The element it is about"),
            c(
                "mechanic",
                Ty::Str,
                "",
                "The mechanic part (docs/mechanics.md: platformer.jump), or a pattern for an element no part fits",
            ),
            c(
                "parameter",
                Ty::Str,
                "",
                "The parameter of that part (gravity)",
            ),
            c(
                "quantity",
                Ty::Str,
                "",
                "What it is, in the constants table's vocabulary; null if not one of them",
            ),
            c(
                "value",
                Ty::Num,
                "",
                "The estimate: the median of the samples",
            ),
            c(
                "spread",
                Ty::Num,
                "",
                "Half the distance between the samples' lowest and highest values; 0 when they agree",
            ),
            c(
                "samples",
                Ty::Int,
                "count",
                "Measurements the estimate is made from",
            ),
            c(
                "unit",
                Ty::Str,
                "",
                "pixels, pixels a frame, pixels a frame a frame, frames, or count",
            ),
            c(
                "method",
                Ty::Str,
                "",
                "The measurer: feel (gp-extract's player and enemy feel), pattern (gp-track's behaviour classes), timer (a byte's count steps)",
            ),
            c(
                "refine",
                Ty::Str,
                "",
                "What would make it exact: the code that writes the value, or clips that isolate it",
            ),
            SRC,
            RULE,
            TOOL,
        ],
    },
    TableSpec {
        name: "runs",
        about: "One row a run of the ROM the extraction made: its recipe and what it reached",
        key: &["rom", "run"],
        cols: &[
            ROM,
            c(
                "run",
                Ty::Str,
                "",
                "The recipe's hash: the ROM, the start and the presses",
            ),
            c("kind", Ty::Str, "", "idle, start, random, clip or explore"),
            c(
                "seed",
                Ty::OptInt,
                "",
                "The seed of a random or explored run",
            ),
            c("frames", Ty::Int, "frames", "Frames run"),
            c("routines", Ty::Int, "count", "Routines it entered"),
            c("prg_bytes", Ty::Int, "bytes", "PRG bytes it executed"),
            c("ram_written", Ty::Int, "bytes", "RAM bytes it wrote"),
            RULE,
            TOOL,
        ],
    },
];

/// A cell of a row.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Cell {
    S(Option<String>),
    I(i64),
    OI(Option<i64>),
    B(bool),
    N(Option<f64>),
}

impl Cell {
    #[allow(dead_code)]
    pub(crate) fn json(&self) -> Value {
        match self {
            Cell::S(v) => json!(v),
            Cell::I(v) => json!(v),
            Cell::OI(v) => json!(v),
            Cell::B(v) => json!(v),
            Cell::N(v) => json!(v),
        }
    }
}

/// A row: each column of its table by name.
pub(crate) type Row = BTreeMap<String, Cell>;

pub(crate) fn spec(table: &str) -> &'static TableSpec {
    TABLES
        .iter()
        .find(|t| t.name == table)
        .unwrap_or_else(|| panic!("no table {table}"))
}

/// A dataset: rows by table.
#[derive(Default, Debug)]
pub(crate) struct Dataset {
    pub tables: BTreeMap<String, Vec<Row>>,
}

impl Dataset {
    /// Adds a row from `(column, value)` pairs given as JSON; a column
    /// left out is null (or 0 and false for a column that can't be).
    pub(crate) fn push(&mut self, table: &str, cells: Value) -> Result<()> {
        let t = spec(table);
        let obj = cells.as_object().context("a row is a JSON object")?;
        for k in obj.keys() {
            if !t.cols.iter().any(|c| c.name == k) {
                bail!("table {table} has no column {k}");
            }
        }
        let mut row = Row::new();
        for c in t.cols {
            let v = obj.get(c.name).unwrap_or(&Value::Null);
            let cell = match c.ty {
                Ty::Str => Cell::S(match v {
                    Value::Null => None,
                    Value::String(s) => Some(s.clone()),
                    other => Some(other.to_string()),
                }),
                Ty::Int => Cell::I(v.as_i64().unwrap_or(0)),
                Ty::OptInt => Cell::OI(v.as_i64()),
                Ty::Bool => Cell::B(v.as_bool().unwrap_or(false)),
                Ty::Num => Cell::N(v.as_f64()),
            };
            row.insert(c.name.to_owned(), cell);
        }
        self.tables.entry(table.to_owned()).or_default().push(row);
        Ok(())
    }

    pub(crate) fn rows(&self, table: &str) -> &[Row] {
        self.tables.get(table).map_or(&[], Vec::as_slice)
    }

    /// Each table's rows in key order, so the same rows give the same
    /// bytes whatever order they were pushed in.
    pub(crate) fn sort(&mut self) {
        for (name, rows) in self.tables.iter_mut() {
            let t = spec(name);
            rows.sort_by(|a, b| {
                let key = |r: &Row| -> Vec<String> {
                    t.cols
                        .iter()
                        .map(|c| match &r[c.name] {
                            Cell::S(v) => v.clone().unwrap_or_default(),
                            Cell::I(v) => format!("{v:020}"),
                            Cell::OI(v) => v.map(|x| format!("{x:020}")).unwrap_or_default(),
                            Cell::B(v) => v.to_string(),
                            Cell::N(v) => v.map(|x| format!("{x:e}")).unwrap_or_default(),
                        })
                        .collect()
                };
                let (ka, kb) = (key(a), key(b));
                let idx: Vec<usize> = t
                    .key
                    .iter()
                    .filter_map(|k| t.cols.iter().position(|c| c.name == *k))
                    .collect();
                let pick = |k: &Vec<String>| idx.iter().map(|&i| k[i].clone()).collect::<Vec<_>>();
                pick(&ka).cmp(&pick(&kb)).then(ka.cmp(&kb))
            });
        }
    }
}

fn table_of(spec: &TableSpec, rows: &[Row]) -> Table {
    let fields = spec
        .cols
        .iter()
        .map(|c| {
            let col = match c.ty {
                Ty::Str => Col::Str(
                    rows.iter()
                        .map(|r| match &r[c.name] {
                            Cell::S(v) => v.clone(),
                            _ => None,
                        })
                        .collect(),
                ),
                Ty::Int => Col::Int(
                    rows.iter()
                        .map(|r| match &r[c.name] {
                            Cell::I(v) => *v,
                            _ => 0,
                        })
                        .collect(),
                ),
                Ty::OptInt => Col::OptInt(
                    rows.iter()
                        .map(|r| match &r[c.name] {
                            Cell::OI(v) => *v,
                            _ => None,
                        })
                        .collect(),
                ),
                Ty::Bool => Col::Bool(
                    rows.iter()
                        .map(|r| matches!(&r[c.name], Cell::B(true)))
                        .collect(),
                ),
                Ty::Num => Col::Num(
                    rows.iter()
                        .map(|r| match &r[c.name] {
                            Cell::N(v) => *v,
                            _ => None,
                        })
                        .collect(),
                ),
            };
            Field {
                name: c.name,
                unit: c.unit,
                about: c.about,
                col,
            }
        })
        .collect();
    Table {
        name: spec.name,
        about: spec.about,
        key: spec.key,
        fields,
    }
}

/// Writes the dataset to `out`: `data/TABLE.parquet` for every table
/// (empty ones too), `README.md` (the card), `croissant.json` and
/// `dataset.json`. With the ROM's bytes, fails if any written file holds
/// a run of them (the pre-commit hook's 16-byte window).
///
/// Writes the dataset's files for a ROM of `kind` (`NES ROM`, `arcade
/// romset`), which the card and the Croissant description name.
pub(crate) fn write(
    out: &Path,
    d: &Dataset,
    rom_sha: &str,
    title: &str,
    rom: Option<&[u8]>,
    kind: &str,
) -> Result<()> {
    let data = out.join("data");
    fs::create_dir_all(&data).with_context(|| format!("making {}", data.display()))?;
    let meta = vec![
        ("gp.format".to_owned(), format!("rom-dataset@{FORMAT}")),
        ("gp.rom".to_owned(), rom_sha.to_owned()),
    ];
    let mut tables = Vec::new();
    let mut files = Vec::new();
    for s in TABLES {
        let t = table_of(s, d.rows(s.name));
        let rel = format!("data/{}.parquet", s.name);
        let p = out.join(&rel);
        ds::write_parquet(&t, &p, &meta)?;
        files.push((rel, ds::sha256(&fs::read(&p)?), t.rows()));
        tables.push(t);
    }
    fs::write(
        out.join("README.md"),
        card(title, rom_sha, &tables, &files, kind),
    )?;
    let cr = ds::croissant_for(
        &format!("game-printer-rom-{}", &rom_sha[..rom_sha.len().min(12)]),
        &format!(
            "What Game Printer's stage 0 extracted from the {kind} with SHA-256 {rom_sha}: routines, branches, RAM, elements, state machines, constants, mechanics, model facts, open questions and runs. No bytes of the ROM."
        ),
        FORMAT,
        &tables,
        "parquet",
        &files,
    );
    fs::write(
        out.join("croissant.json"),
        serde_json::to_string_pretty(&cr)? + "\n",
    )?;
    let manifest = json!({
        "format": format!("rom-dataset@{FORMAT}"),
        "rom": rom_sha,
        "tables": files.iter().map(|f| json!({"file": f.0, "sha256": f.1, "rows": f.2})).collect::<Vec<_>>(),
    });
    fs::write(
        out.join("dataset.json"),
        serde_json::to_string_pretty(&manifest)? + "\n",
    )?;
    if let Some(rom) = rom {
        let leaks = crate::dataset_tables::rom_runs(out, rom)?;
        if !leaks.is_empty() {
            bail!(
                "the dataset holds bytes of the ROM: {}",
                leaks
                    .iter()
                    .map(|(f, at)| format!("{f} at byte {at}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
    Ok(())
}

fn card(
    title: &str,
    rom_sha: &str,
    tables: &[Table],
    files: &[(String, String, usize)],
    kind: &str,
) -> String {
    let mut c = String::from("---\n");
    let _ = writeln!(c, "pretty_name: \"{title}\"");
    c.push_str("license: other\nlicense_name: game-printer-recipes\n");
    let _ = write!(
        c,
        "tags:\n- game-printer\n- {}\n- reverse-engineering\n- game-mechanics\n- tabular\n",
        if kind == "NES ROM" { "nes" } else { "arcade" }
    );
    c.push_str("configs:\n");
    for (k, t) in tables.iter().enumerate() {
        let _ = writeln!(
            c,
            "- config_name: {}\n  data_files:\n  - split: train\n    path: \"data/{}.parquet\"",
            t.name, t.name
        );
        if k == 0 {
            c.push_str("  default: true\n");
        }
    }
    c.push_str("---\n\n");
    let _ = writeln!(c, "# {title}\n");
    let _ = writeln!(
        c,
        "What Game Printer's stage 0 extracted from one {kind} (SHA-256 `{rom_sha}`), from the ROM alone, by running it and reading its code. Format `rom-dataset` {FORMAT} (docs/formats/rom-dataset.md). Every table has the same columns for every game, so the datasets of many ROMs union into one table each, joined on `rom`.\n"
    );
    c.push_str("It holds no bytes of the ROM: addresses, names the tool made, numbers it read and the questions it left. Every row says where it came from (`src`) and which rule made it (`rule`).\n\n");
    c.push_str("## Tables\n\n");
    for t in tables {
        let rows = files
            .iter()
            .find(|f| f.0 == format!("data/{}.parquet", t.name))
            .map_or(0, |f| f.2);
        let _ = writeln!(
            c,
            "### {}\n\n{}. Key: {}. Rows: {rows}.\n",
            t.name,
            t.about,
            t.key.join(", ")
        );
        c.push_str("| Column | Type | Unit | Meaning |\n|---|---|---|---|\n");
        for f in &t.fields {
            let ty = f.col.croissant_type().trim_start_matches("sc:");
            let _ = writeln!(c, "| `{}` | {ty} | {} | {} |", f.name, f.unit, f.about);
        }
        c.push('\n');
    }
    c.push_str(QUERIES);
    c.push_str("\n## Files\n\n| File | SHA-256 | Rows |\n|---|---|---|\n");
    for (f, h, rows) in files {
        let _ = writeln!(c, "| `{f}` | `{h}` | {rows} |");
    }
    c
}

/// The card's queries, run across every dataset in a folder.
pub(crate) const QUERIES: &str = "## Load it\n\n```python\nimport duckdb\n# One ROM\nduckdb.sql(\"SELECT role, count(*) FROM 'data/ram.parquet' GROUP BY role ORDER BY 2 DESC\").show()\n# Every ROM in a folder of datasets\nduckdb.sql(\"SELECT rom, number FROM 'datasets/*/data/constants.parquet' WHERE quantity = 'gravity'\").show()\nduckdb.sql(\"SELECT rom, machine, count(*) AS states FROM 'datasets/*/data/states.parquet' GROUP BY ALL HAVING states > 4\").show()\nduckdb.sql(\"SELECT r.rom, count(*) FILTER (WHERE r.element IS NOT NULL) * 1.0 / count(*) AS placed FROM 'datasets/*/data/routines.parquet' r WHERE r.ran GROUP BY r.rom\").show()\n\nimport pandas as pd\nram = pd.read_parquet(\"data/ram.parquet\")\n\nfrom datasets import load_dataset\nconstants = load_dataset(\"parquet\", data_files=\"data/constants.parquet\")[\"train\"]\n```\n";

/// Reads a dataset's Parquet files back as rows.
#[allow(dead_code)]
pub(crate) fn read(dir: &Path) -> Result<Dataset> {
    use parquet::file::reader::{FileReader, SerializedFileReader};
    use parquet::record::Field as PF;
    let mut d = Dataset::default();
    for s in TABLES {
        let p = dir.join(format!("data/{}.parquet", s.name));
        let f = fs::File::open(&p).with_context(|| format!("reading {}", p.display()))?;
        let r = SerializedFileReader::new(f)?;
        let mut rows = Vec::new();
        for row in r.get_row_iter(None)? {
            let row = row?;
            let mut out = Row::new();
            for (name, v) in row.get_column_iter() {
                let Some(cs) = s.cols.iter().find(|c| c.name == name) else {
                    continue;
                };
                let cell = match (cs.ty, v) {
                    (Ty::Str, PF::Str(x)) => Cell::S(Some(x.clone())),
                    (Ty::Str, _) => Cell::S(None),
                    (Ty::Int, PF::Long(x)) => Cell::I(*x),
                    (Ty::Int, _) => Cell::I(0),
                    (Ty::OptInt, PF::Long(x)) => Cell::OI(Some(*x)),
                    (Ty::OptInt, _) => Cell::OI(None),
                    (Ty::Bool, PF::Bool(x)) => Cell::B(*x),
                    (Ty::Bool, _) => Cell::B(false),
                    (Ty::Num, PF::Double(x)) => Cell::N(Some(*x)),
                    (Ty::Num, _) => Cell::N(None),
                };
                out.insert(name.clone(), cell);
            }
            rows.push(out);
        }
        d.tables.insert(s.name.to_owned(), rows);
    }
    Ok(d)
}

/// `schema/rom-dataset.json`: one row schema a table, under `$defs`. The
/// committed file is checked against it, and `gp self schemas` writes it.
pub(crate) fn json_schema() -> String {
    let mut defs = serde_json::Map::new();
    for t in TABLES {
        let mut props = serde_json::Map::new();
        for c in t.cols {
            let ty = match c.ty {
                Ty::Str => json!(["string", "null"]),
                Ty::Int => json!("integer"),
                Ty::OptInt => json!(["integer", "null"]),
                Ty::Bool => json!("boolean"),
                Ty::Num => json!(["number", "null"]),
            };
            let mut about = c.about.to_owned();
            if !c.unit.is_empty() {
                about = format!("{about} (unit: {})", c.unit);
            }
            props.insert(c.name.into(), json!({"type": ty, "description": about}));
        }
        let required: Vec<&str> = t.cols.iter().map(|c| c.name).collect();
        defs.insert(
            t.name.into(),
            json!({"type": "object", "description": format!("{}. Key: {}.", t.about, t.key.join(", ")),
                   "properties": props, "required": required, "additionalProperties": false}),
        );
    }
    let s = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": format!("game-printer:rom-dataset@{FORMAT}"),
        "title": "ROM dataset",
        "description": "One row of each table of a ROM dataset (docs/formats/rom-dataset.md). The tables are Parquet files, data/TABLE.parquet; each row has these columns.",
        "$defs": Value::Object(defs),
    });
    serde_json::to_string_pretty(&s).expect("schemas serialize") + "\n"
}

/// The board name the `rom` table gives a mapper.
fn board_name(mapper: u16) -> String {
    match mapper {
        0 => "nrom".into(),
        1 => "mmc1".into(),
        2 => "uxrom".into(),
        4 => "mmc3".into(),
        m => format!("mapper{m}"),
    }
}

/// The `rom` table's row, with no runs.
pub(crate) fn rom_row(
    d: &mut Dataset,
    img: &gp_evidence::source::NesImage,
    runs: i64,
    frames: i64,
    asset_map: Option<String>,
) -> Result<()> {
    d.push(
        "rom",
        json!({
            "rom": img.sha256,
            "mapper": img.header.mapper,
            "board": board_name(img.header.mapper),
            "prg_bytes": img.header.prg_rom_bytes,
            "chr_bytes": img.header.chr_rom_bytes,
            "asset_map": asset_map,
            "runs": runs,
            "frames": frames,
            "format": FORMAT,
            "tool": tool(),
        }),
    )
}

/// The `tool` column's value.
pub(crate) fn tool() -> String {
    format!("gp {}", env!("CARGO_PKG_VERSION"))
}

/// `gp dataset rom`.
pub(crate) fn cli(
    rom: &Path,
    out: &Path,
    empty: bool,
    runs: usize,
    seed: u64,
    paths: &gp_evidence::Paths,
) -> Result<crate::Outcome> {
    let bytes = fs::read(rom).with_context(|| format!("reading {}", rom.display()))?;
    // An arcade romset's stage 0 reads the main Z80's code map
    // (crate::arcade_stage0); an NES image's reads 6502 code and its banks.
    let (sha, raw, platform, mut d) =
        if crate::original::Platform::of_file(&bytes) == crate::original::Platform::Arcade {
            let set = gp_arcade::Romset::from_zip(&bytes)?;
            let d = if empty {
                Dataset::default()
            } else {
                crate::arcade_stage0::extract(&set, runs, seed)?
            };
            (
                set.identity.clone(),
                crate::arcade_stage0::rom_bytes(&set),
                "arcade romset",
                d,
            )
        } else {
            let (img, prg, chr) = gp_evidence::source::nes_banks(&bytes)?;
            let d = if empty {
                Dataset::default()
            } else {
                crate::rom_extract::extract(rom, runs, seed, paths)?
            };
            let mut raw = prg.to_vec();
            raw.extend_from_slice(chr);
            (img.sha256, raw, "NES ROM", d)
        };
    d.sort();
    // Named by the ROM's hash: a file name can be text the ROM holds (a
    // test ROM's own name), which the ROM check would refuse.
    write(
        out,
        &d,
        &sha,
        &format!("ROM dataset {}", &sha[..12]),
        Some(&raw),
        platform,
    )?;
    let counts: Vec<String> = TABLES
        .iter()
        .map(|t| format!("{} {}", t.name, d.rows(t.name).len()))
        .collect();
    let rows: serde_json::Map<String, Value> = TABLES
        .iter()
        .map(|t| (t.name.to_owned(), json!(d.rows(t.name).len())))
        .collect();
    Ok(crate::Outcome::ok(
        json!({"rom": sha, "out": out.display().to_string(), "rows": rows}),
        format!(
            "{} -> {}: {}",
            rom.display(),
            out.display(),
            counts.join(", ")
        ),
    ))
}

/// The kind of an open row: its `what` up to the first address or name
/// (`ram:$0507` -> `ram`, `routines behind bank@C27D` -> `routines behind`).
fn open_kind(what: &str) -> String {
    let w = what.split(':').next().unwrap_or(what);
    w.split(|c: char| c == '$' || c.is_ascii_digit())
        .next()
        .unwrap_or(w)
        .split_whitespace()
        .take(3)
        .collect::<Vec<_>>()
        .join(" ")
}

/// `gp dataset gaps`.
pub(crate) fn gaps(dirs: &[std::path::PathBuf]) -> Result<crate::Outcome> {
    let mut all: BTreeMap<String, usize> = BTreeMap::new();
    let mut per: Vec<(String, BTreeMap<String, usize>)> = Vec::new();
    for dir in dirs {
        let d = read(dir)?;
        let mut c: BTreeMap<String, usize> = BTreeMap::new();
        let text = |r: &Row, k: &str| match r.get(k) {
            Some(Cell::S(Some(v))) => v.clone(),
            _ => String::new(),
        };
        for r in d.rows("open") {
            let why = text(r, "why");
            let suggest = why
                .split_once("the runs suggest ")
                .map(|(_, x)| x.split(',').next().unwrap_or(x).to_owned());
            let key = match suggest {
                Some(s) => format!(
                    "open {} (runs suggest {s}), by {}",
                    open_kind(&text(r, "what")),
                    text(r, "settle_by")
                ),
                None => format!(
                    "open {}, by {}",
                    open_kind(&text(r, "what")),
                    text(r, "settle_by")
                ),
            };
            *c.entry(key).or_default() += 1;
        }
        for r in d.rows("constants") {
            if matches!(r.get("number"), Some(Cell::N(None)) | None) {
                *c.entry(format!("constants with no unit ({})", text(r, "op")))
                    .or_default() += 1;
            }
        }
        for r in d.rows("routines") {
            if matches!(r.get("ran"), Some(Cell::B(true)))
                && matches!(r.get("element"), Some(Cell::S(None)) | None)
            {
                *c.entry("routines run with no element".to_owned())
                    .or_default() += 1;
            }
        }
        for (k, n) in &c {
            *all.entry(k.clone()).or_default() += n;
        }
        per.push((dir.display().to_string(), c));
    }
    fn ranked(m: &BTreeMap<String, usize>) -> Vec<(&String, &usize)> {
        let mut v: Vec<(&String, &usize)> = m.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        v
    }
    let mut text = String::from("Over all datasets\n\n| Gap | Rows |\n|---|---|\n");
    for (k, n) in ranked(&all).into_iter().take(25) {
        let _ = writeln!(text, "| {k} | {n} |");
    }
    for (name, m) in &per {
        let _ = writeln!(text, "\n{name}\n\n| Gap | Rows |\n|---|---|");
        for (k, n) in ranked(m).into_iter().take(10) {
            let _ = writeln!(text, "| {k} | {n} |");
        }
    }
    let json_all: serde_json::Map<String, Value> =
        all.iter().map(|(k, n)| (k.clone(), json!(n))).collect();
    Ok(crate::Outcome::ok(json!({"all": json_all}), text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_dataset_writes_every_table_and_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let d = Dataset::default();
        write(dir.path(), &d, "00", "empty", None, "NES ROM").unwrap();
        let back = read(dir.path()).unwrap();
        for t in TABLES {
            assert!(dir.path().join(format!("data/{}.parquet", t.name)).exists());
            assert!(back.rows(t.name).is_empty());
        }
    }

    #[test]
    fn rows_round_trip_and_the_same_rows_give_the_same_bytes() {
        let mk = |order: &[i64]| {
            let mut d = Dataset::default();
            for &a in order {
                d.push(
                    "ram",
                    json!({"rom": "ab", "addr": a, "role": "state", "slot": null, "written": true, "readers": "[\"C000\"]"}),
                )
                .unwrap();
            }
            d.push(
                "constants",
                json!({"rom": "ab", "pc": "C123", "raw": 200, "number": -3.5, "format": "4.4"}),
            )
            .unwrap();
            d.sort();
            d
        };
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        write(a.path(), &mk(&[5, 3, 9]), "ab", "t", None, "NES ROM").unwrap();
        write(b.path(), &mk(&[9, 5, 3]), "ab", "t", None, "NES ROM").unwrap();
        for t in TABLES {
            let f = format!("data/{}.parquet", t.name);
            assert_eq!(
                fs::read(a.path().join(&f)).unwrap(),
                fs::read(b.path().join(&f)).unwrap(),
                "{f}"
            );
        }
        let back = read(a.path()).unwrap();
        let ram = back.rows("ram");
        assert_eq!(ram.len(), 3);
        assert_eq!(ram[0]["addr"], Cell::I(3));
        assert_eq!(ram[0]["slot"], Cell::OI(None));
        assert_eq!(ram[0]["role"], Cell::S(Some("state".into())));
        assert_eq!(back.rows("constants")[0]["number"], Cell::N(Some(-3.5)));
    }

    #[test]
    fn a_column_the_table_lacks_is_refused() {
        let mut d = Dataset::default();
        assert!(d.push("ram", json!({"rom": "ab", "nope": 1})).is_err());
    }

    #[test]
    fn the_format_doc_names_every_table_and_column() {
        let doc = include_str!("../../../docs/formats/rom-dataset.md");
        for t in TABLES {
            assert!(doc.contains(&format!("`{}`", t.name)), "table {}", t.name);
            for c in t.cols {
                assert!(
                    doc.contains(&format!("`{}`", c.name)),
                    "{}.{}",
                    t.name,
                    c.name
                );
            }
        }
    }

    #[test]
    fn a_dataset_holding_rom_bytes_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let rom: Vec<u8> = (0..64u32).map(|x| ((x * 7) % 90 + 33) as u8).collect();
        let mut d = Dataset::default();
        // A string holding 16 of the ROM's bytes in a row.
        let leak: String = rom[8..24].iter().map(|&b| b as char).collect();
        d.push("open", json!({"rom": "ab", "what": leak})).unwrap();
        assert!(write(dir.path(), &d, "ab", "t", Some(&rom), "NES ROM").is_err());
    }

    /// The committed schema/rom-dataset.json is the one TABLES makes.
    #[test]
    fn the_committed_schema_is_current() {
        let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schema/rom-dataset.json");
        let text = fs::read_to_string(&file).unwrap();
        assert_eq!(
            text.trim_end(),
            json_schema().trim_end(),
            "rewrite {}",
            file.display()
        );
    }
}
