//! The static harvest (docs/plan-extraction-process.md, stage 0): every
//! part of a model the ROM's code settles without running it, and the rest
//! as open facts naming the routine and bytes that decide them. It reads the
//! ROM's code ([`crate::codemap`]) and the asset map's tables only, never
//! labels or source.
//!
//! [`absint`] reads every routine from the vectors with each call followed
//! in its caller's state; [`cond`] turns branch outcomes into conditions on
//! places; the rules turn both into elements, dispatch, state machines,
//! constants, mapping rows and model facts, each with the instruction it
//! comes from. The output is the same bytes for the same ROM and asset map.
//!
//! Boards: the code map reads the CPU's view at power-on ([`crate::codemap::cpu_image`]):
//! mapper 0 (NROM-128 mirrored, NROM-256) in full; on a banked board only
//! the first and last 16KB, so code in other banks isn't read (an open
//! fact says so).

pub mod absint;
pub mod cond;
mod rules;
pub mod unread;

use serde::Serialize;
use serde_json::Value;

use crate::codemap::{Board, RomCode, Va, cpu_image};
use absint::{Ev, Reader};

/// The shape of the output. A change to it bumps this.
pub const SCHEMA: &str = "gp model static 1";

/// A table the asset map places in PRG ROM.
#[derive(Debug, Clone, Serialize)]
pub struct Table {
    pub asset: String,
    /// The asset's column or property (`kind`, `solid`, `start`), or the
    /// asset's own name for its main span.
    pub column: String,
    /// The asset's format (`metatiles`, `level-grid`, `palette`, `table`).
    pub asset_kind: String,
    /// Offset in PRG ROM and length in bytes (0 when unknown).
    pub offset: usize,
    pub len: usize,
}

/// What the harvest reads besides the code.
#[derive(Debug, Clone, Default)]
pub struct Input {
    pub prg_len: usize,
    pub tables: Vec<Table>,
    pub level_width_px: u32,
    pub metatile_names: Vec<String>,
    /// Each metatile's kind property, read from ROM through the asset map.
    pub metatile_kind: Vec<u8>,
    /// Each metatile's solid property, the same way.
    pub metatile_solid: Vec<u8>,
    /// Routine starts runs of the ROM entered (calls, interrupt handlers,
    /// jumps through a pointer or a pushed address), at banked addresses:
    /// the code map starts from them as well as from the vectors.
    pub seed_starts: Vec<Va>,
    /// Transfers runs took that the code alone can't name: (the jump's
    /// address, where it went), for a `jmp (ptr)` and an `rts` used as a
    /// jump. Kept on every pass, as a table's entries are.
    pub seed_jumps: Vec<(Va, Va)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Evidence {
    pub routine: String,
    pub pc: String,
    pub insn: String,
}

/// One model fact, by its path in a model whose names the harvest chose
/// (`entities.player`, `entities.slots_20`, `rules.state_05`, states `s0`
/// to `sN` by value, counters `c_XX` by address).
#[derive(Debug, Clone, Serialize)]
pub struct Fact {
    pub path: String,
    pub value: Value,
    pub rule: String,
    pub evidence: Vec<Evidence>,
}

/// A fact the code leaves open, with what decides it.
#[derive(Debug, Clone, Serialize)]
pub struct Open {
    pub what: String,
    pub routine: String,
    pub pc: String,
    pub bytes: Vec<String>,
    pub why: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RoutineRow {
    pub name: String,
    pub addr: String,
    pub size: u32,
    pub element: String,
    pub why: String,
    pub calls: Vec<String>,
}

/// Code that picks routines by a byte: a jump table or a compare chain.
#[derive(Debug, Clone, Serialize)]
pub struct Dispatch {
    pub kind: String,
    pub at: String,
    pub subject: String,
    /// Each value (or list of values) and the routines that run for it.
    pub cases: std::collections::BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MapRow {
    pub byte: String,
    pub role: String,
    pub read_by: Vec<String>,
    pub written_by: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Element {
    pub name: String,
    pub kind: String,
    pub routines: Vec<String>,
    pub mapping: Vec<MapRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct State {
    pub value: u8,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Transition {
    pub from: String,
    pub to: String,
    pub at: String,
    pub routine: String,
    /// The other conditions on the path, as text.
    pub guards: Vec<String>,
    pub after: Option<u32>,
    pub counter: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Machine {
    pub byte: String,
    pub name: String,
    /// The state the reset code writes before the frame loop.
    pub initial: Option<String>,
    /// Where its facts are in the model (`rules.state_05`,
    /// `entities.slots_20.machine`).
    pub path: String,
    pub states: Vec<State>,
    pub transitions: Vec<Transition>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Constant {
    pub routine: String,
    pub pc: String,
    pub insn: String,
    pub value: u8,
    /// threshold, mask, offset, stored, loaded or operand.
    pub what: String,
    /// The store that follows a load.
    pub then: String,
    /// Whether a fact cites it.
    pub cited: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Harvest {
    pub schema: String,
    pub routines: Vec<RoutineRow>,
    pub dispatch: Vec<Dispatch>,
    pub elements: Vec<Element>,
    pub machines: Vec<Machine>,
    pub facts: Vec<Fact>,
    pub open: Vec<Open>,
    pub constants: Vec<Constant>,
}

/// Harvests a CPU image ($8000 to $FFFF, from [`cpu_image`]) with the
/// asset map's tables. Jump tables found on a pass add their entries as
/// routine starts, and the code is read again until a pass finds the same
/// tables as the one before.
pub fn harvest(image: &[u8], inp: &Input) -> Harvest {
    harvest_board(&Board::flat(image), inp)
}

/// Passes before the harvest stops even if a pass still found something new:
/// each can add the routines of a bank or a table the one before reached.
const PASSES: usize = 8;

/// [`harvest`] on a board whose banks a mapper switches ([`Board`]).
pub fn harvest_board(board: &Board, inp: &Input) -> Harvest {
    passes(board, inp, false).0
}

/// [`harvest_board`] with the report of where the reader stopped
/// ([`unread`]).
pub fn harvest_board_unread(board: &Board, inp: &Input) -> (Harvest, unread::Unread) {
    let (h, u) = passes(board, inp, true);
    (h, u.expect("asked for"))
}

fn passes(board: &Board, inp: &Input, want: bool) -> (Harvest, Option<unread::Unread>) {
    let mut jumps: Vec<(Va, Va)> = inp.seed_jumps.clone();
    jumps.sort_unstable();
    jumps.dedup();
    let mut extra: Vec<Va> = inp.seed_starts.clone();
    extra.sort_unstable();
    extra.dedup();
    let mut hints: std::collections::BTreeMap<Va, std::collections::BTreeSet<Va>> =
        Default::default();
    let mut interrupts: Option<std::collections::BTreeSet<absint::Loc>> = None;
    let mut frames: Option<std::collections::BTreeSet<absint::Loc>> = None;
    let mut named: std::collections::BTreeSet<Va> = Default::default();
    let mut values: std::collections::BTreeMap<Va, std::collections::BTreeSet<u8>> =
        Default::default();
    for pass in 0..PASSES {
        let code = RomCode::of_board(board, &extra, &jumps);
        let mut r = Reader::new(&code);
        r.hints = hints.clone();
        r.interrupt_writes = interrupts.clone();
        r.frame_writes = frames.clone();
        r.named = named.clone();
        r.frame_values = values.clone();
        r.run_vectors();
        // Then what runs entered that no path from the vectors read.
        r.run_entries(&inp.seed_starts);
        let mut h = rules::H::new(&code, &r, inp);
        rules::run(&mut h);
        // The next pass reads with this pass's tables: a site read again
        // takes its new entries (a table bounded once more is known, by the
        // next site's table or its index's values, drops the entries it
        // read past its end); a site this pass didn't reach keeps its own.
        let read: std::collections::BTreeSet<Va> = h.tables.iter().map(|p| p.0).collect();
        let mut more: Vec<(Va, Va)> = jumps
            .iter()
            .copied()
            .filter(|p| !read.contains(&p.0))
            .chain(h.tables.iter().copied())
            .chain(r.hinted.iter().copied())
            .chain(inp.seed_jumps.iter().copied())
            .collect();
        more.sort_unstable();
        more.dedup();
        // Routines in a bank the reader went into start routines too.
        let mut banked: Vec<Va> = extra.iter().chain(r.banked.iter()).copied().collect();
        banked.sort_unstable();
        banked.dedup();
        let next_hints = absint::pointer_hints(&code, &r.acts);
        let next_interrupts = Some(r.interrupt_writes_now());
        let next_frames = Some(r.frame_writes_now());
        let next_named = r.named_now();
        let next_values = r.frame_values_now();
        if (more == jumps
            && banked == extra
            && next_hints == hints
            && next_interrupts == interrupts
            && next_frames == frames
            && next_named == named
            && next_values == values)
            || pass == PASSES - 1
        {
            let u = want.then(|| unread::report(&code, &r));
            return (finish(h, &code, &r), u);
        }
        jumps = more;
        extra = banked;
        hints = next_hints;
        interrupts = next_interrupts;
        frames = next_frames;
        named = next_named;
        values = next_values;
    }
    unreachable!("the last pass returns")
}

/// A pass's harvest, with an open fact for each call into a switched bank
/// the reader couldn't follow.
fn finish(h: rules::H, code: &RomCode, r: &Reader) -> Harvest {
    let mut out = h.out;
    let mut seen = std::collections::BTreeSet::new();
    for &(pc, t) in &r.unresolved {
        if !seen.insert(pc) {
            continue;
        }
        let routine = code
            .routines
            .at(pc)
            .map(|i| code.routines.names[i].clone())
            .unwrap_or_default();
        out.open.push(Open {
            what: format!("call into a switched bank at {}", crate::codemap::va_hex(pc)),
            routine,
            pc: crate::codemap::va_hex(pc),
            bytes: Vec::new(),
            why: format!("${t:04X} is in a switched region and the bank mapped there isn't known on this path"),
        });
    }
    out.open.sort_by(|a, b| a.what.cmp(&b.what));
    out.schema = SCHEMA.into();
    out
}

/// The PRG ROM of an iNES file: after the header and the trainer, if the
/// flags say there is one. The mapper number with it.
pub fn ines_prg(rom: &[u8]) -> Option<(&[u8], u16)> {
    if rom.len() < 16 || &rom[..4] != b"NES\x1A" {
        return None;
    }
    let banks = usize::from(rom[4]);
    let trainer = if rom[6] & 0x04 != 0 { 512 } else { 0 };
    let start = 16 + trainer;
    let end = start + banks * 0x4000;
    let mapper = u16::from(rom[6] >> 4) | u16::from(rom[7] & 0xF0);
    rom.get(start..end).map(|p| (p, mapper))
}

/// The CPU image of an iNES file, and its mapper.
pub fn ines_image(rom: &[u8]) -> Option<(Vec<u8>, u16, usize)> {
    let (prg, mapper) = ines_prg(rom)?;
    Some((cpu_image(prg), mapper, prg.len()))
}

/// Every activation's events, as text, for reading by hand while the rules
/// are written.
pub fn dump(image: &[u8]) -> String {
    let code = RomCode::of(image);
    let mut r = Reader::new(&code);
    let v = r.run_vectors();
    let mut out = format!("vectors {v:?}\nloops {:04X?}\n", r.loops);
    for (i, a) in r.acts.iter().enumerate() {
        out += &format!(
            "act {i} {:04X} general={} callers={:?} returns={}\n",
            a.start, a.general, a.callers, a.returns
        );
        for e in &a.events {
            if !matches!(e, Ev::Read { .. }) {
                out += &format!("  {e:?}\n");
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An NROM-128 image with `code` at $C000 and the vectors pointing at
    /// `reset` and `nmi`.
    fn image(code: &[(u16, &[u8])], reset: u16, nmi: u16) -> Vec<u8> {
        let mut prg = vec![0xFFu8; 0x4000];
        for (at, bytes) in code {
            let o = (*at as usize - 0xC000) % 0x4000;
            prg[o..o + bytes.len()].copy_from_slice(bytes);
        }
        prg[0x3FFA..0x3FFC].copy_from_slice(&nmi.to_le_bytes());
        prg[0x3FFC..0x3FFE].copy_from_slice(&reset.to_le_bytes());
        prg[0x3FFE..0x4000].copy_from_slice(&nmi.to_le_bytes());
        cpu_image(&prg)
    }

    /// A jump through a pointer no code sets (a run fills it) reaches a
    /// routine the code alone never finds; seeded with what a run did, the
    /// code map has it and the reader reads it.
    #[test]
    fn run_seeds_add_what_the_code_alone_cant_reach() {
        let img = image(
            &[
                // C000: JMP ($0010), to whatever a run put there
                (0xC000, &[0x6C, 0x10, 0x00]),
                // C100: INC $20; JMP C000
                (0xC100, &[0xE6, 0x20, 0x4C, 0x00, 0xC0]),
                (0xC080, &[0x40]),
            ],
            0xC000,
            0xC080,
        );
        let names = |h: &Harvest| {
            h.routines
                .iter()
                .map(|r| r.name.clone())
                .collect::<Vec<_>>()
        };
        let plain = harvest(&img, &Input::default());
        assert!(!names(&plain).contains(&"sub_C100".to_string()));
        let seeded = harvest(
            &img,
            &Input {
                seed_starts: vec![0xC100],
                seed_jumps: vec![(0xC000, 0xC100)],
                ..Input::default()
            },
        );
        assert!(names(&seeded).contains(&"sub_C100".to_string()));
        // And the reader reads it: from the run's entry, not only from the
        // vectors.
        let board = Board::flat(&img);
        let (_, u) = harvest_board_unread(
            &board,
            &Input {
                seed_starts: vec![0xC100],
                seed_jumps: vec![(0xC000, 0xC100)],
                ..Input::default()
            },
        );
        assert_eq!(
            u.read, u.routines,
            "every routine read: {} of {}",
            u.read, u.routines
        );
        // Seeds change nothing else in a program the code reaches whole.
        let whole = table_rom();
        let a = serde_json::to_string(&harvest(&whole, &Input::default())).unwrap();
        let b = serde_json::to_string(&harvest(
            &whole,
            &Input {
                seed_starts: vec![0xC010, 0xC050],
                ..Input::default()
            },
        ))
        .unwrap();
        assert_eq!(a, b);
    }

    /// A frame loop that calls a dispatcher on the byte at $10 through a
    /// table of three words; each entry steps a byte of its own.
    fn table_rom() -> Vec<u8> {
        image(
            &[
                // C000: JSR C010; JMP C000
                (0xC000, &[0x20, 0x10, 0xC0, 0x4C, 0x00, 0xC0]),
                // C010: LDA $10; ASL A; TAX; LDA C040,X; STA $00;
                //       LDA C041,X; STA $01; JMP ($0000)
                (
                    0xC010,
                    &[
                        0xA5, 0x10, 0x0A, 0xAA, 0xBD, 0x40, 0xC0, 0x85, 0x00, 0xBD, 0x41, 0xC0,
                        0x85, 0x01, 0x6C, 0x00, 0x00,
                    ],
                ),
                // C040: the table
                (0xC040, &[0x50, 0xC0, 0x60, 0xC0, 0x70, 0xC0]),
                // C050, C060, C070: INC $20 (or $21, $22); RTS
                (0xC050, &[0xE6, 0x20, 0x60]),
                (0xC060, &[0xE6, 0x21, 0x60]),
                (0xC070, &[0xE6, 0x22, 0x60]),
                // C080: RTI
                (0xC080, &[0x40]),
            ],
            0xC000,
            0xC080,
        )
    }

    #[test]
    fn a_jump_table_gives_one_element_per_entry() {
        let h = harvest(&table_rom(), &Input::default());
        let d = h
            .dispatch
            .iter()
            .find(|d| d.kind == "jump table")
            .expect("the table");
        assert_eq!(d.at, "C01E");
        assert_eq!(d.subject, "$10");
        let targets: Vec<&str> = d.cases.values().flatten().map(String::as_str).collect();
        assert_eq!(targets, ["sub_C050", "sub_C060", "sub_C070"]);
        for t in targets {
            let r = h
                .routines
                .iter()
                .find(|r| r.name == t)
                .expect("the entry is a routine");
            assert!(
                r.element.starts_with("dispatch_C01E_"),
                "{t}: {}",
                r.element
            );
        }
        assert_eq!(
            h.elements
                .iter()
                .filter(|e| e.kind == "dispatch entry")
                .count(),
            3
        );
    }

    /// A frame loop that jumps through a table of words by the byte at
    /// $10, shifted left, after the reset sets it to 0. The entries are code
    /// the loop runs on into; each calls a routine that sets the byte to
    /// the other value and jumps back to the loop.
    fn inline_table_rom() -> Vec<u8> {
        image(
            &[
                // C000: LDA #0; STA $10
                (0xC000, &[0xA9, 0x00, 0x85, 0x10]),
                // C004: LDA $10; ASL A; TAX; LDA C030,X; STA $00;
                //       LDA C031,X; STA $01; JMP ($0000)
                (
                    0xC004,
                    &[
                        0xA5, 0x10, 0x0A, 0xAA, 0xBD, 0x30, 0xC0, 0x85, 0x00, 0xBD, 0x31, 0xC0,
                        0x85, 0x01, 0x6C, 0x00, 0x00,
                    ],
                ),
                // C015: JSR C040; JMP C004
                (0xC015, &[0x20, 0x40, 0xC0, 0x4C, 0x04, 0xC0]),
                // C01B: JSR C050; JMP C004
                (0xC01B, &[0x20, 0x50, 0xC0, 0x4C, 0x04, 0xC0]),
                // C030: the table
                (0xC030, &[0x15, 0xC0, 0x1B, 0xC0]),
                // C040: LDA #1; STA $10; RTS
                (0xC040, &[0xA9, 0x01, 0x85, 0x10, 0x60]),
                // C050: LDA #0; STA $10; RTS
                (0xC050, &[0xA9, 0x00, 0x85, 0x10, 0x60]),
                // C060: RTI
                (0xC060, &[0x40]),
            ],
            0xC000,
            0xC060,
        )
    }

    #[test]
    fn a_jump_table_in_the_frame_loop_dispatches_on_the_index_byte() {
        let h = harvest(&inline_table_rom(), &Input::default());
        let t = h
            .dispatch
            .iter()
            .find(|d| d.kind == "jump table")
            .expect("the table");
        assert_eq!(t.at, "C012");
        assert_eq!(t.subject, "$10");
        let targets: Vec<&str> = t.cases.values().flatten().map(String::as_str).collect();
        assert_eq!(targets, ["sub_C015", "sub_C01B"]);
        let names: Vec<&str> = h.routines.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"sub_C004"), "{names:?}");
        // Each entry runs for the byte's value that indexes it: entry 1 is
        // at offset 2, the byte shifted left once.
        let c = h
            .dispatch
            .iter()
            .find(|d| d.kind == "compare chain")
            .expect("the dispatch by value");
        assert_eq!(c.subject, "$10");
        assert_eq!(c.cases.get("0"), Some(&vec!["sub_C040".to_owned()]));
        assert_eq!(c.cases.get("1"), Some(&vec!["sub_C050".to_owned()]));
        let m = h
            .machines
            .iter()
            .find(|m| m.byte == "$10")
            .expect("the state byte's machine");
        let states: Vec<u8> = m.states.iter().map(|s| s.value).collect();
        assert_eq!(states, [0, 1]);
    }

    /// A dispatcher that pushes an address from two tables (low bytes,
    /// then high bytes) at the byte at $10, unshifted, and returns to it.
    /// Each entry sets the byte to the next value.
    fn rts_table_rom() -> Vec<u8> {
        image(
            &[
                // C000: JSR C010; JMP C000
                (0xC000, &[0x20, 0x10, 0xC0, 0x4C, 0x00, 0xC0]),
                // C010: LDX $10; LDA C043,X; PHA; LDA C040,X; PHA; RTS
                (
                    0xC010,
                    &[
                        0xA6, 0x10, 0xBD, 0x43, 0xC0, 0x48, 0xBD, 0x40, 0xC0, 0x48, 0x60,
                    ],
                ),
                // C040: the low bytes, then the high bytes, of each entry
                // less one
                (0xC040, &[0x4F, 0x5F, 0x6F, 0xC0, 0xC0, 0xC0]),
                // C050, C060, C070: LDA #1 (2, 0); STA $10; RTS
                (0xC050, &[0xA9, 0x01, 0x85, 0x10, 0x60]),
                (0xC060, &[0xA9, 0x02, 0x85, 0x10, 0x60]),
                (0xC070, &[0xA9, 0x00, 0x85, 0x10, 0x60]),
                // C080: RTI
                (0xC080, &[0x40]),
            ],
            0xC000,
            0xC080,
        )
    }

    #[test]
    fn a_pushed_jump_table_tests_its_index_byte() {
        let h = harvest(&rts_table_rom(), &Input::default());
        let t = h
            .dispatch
            .iter()
            .find(|d| d.kind == "jump table")
            .expect("the table");
        assert_eq!(t.subject, "$10");
        let targets: Vec<&str> = t.cases.values().flatten().map(String::as_str).collect();
        assert_eq!(targets, ["sub_C050", "sub_C060", "sub_C070"]);
        let m = h
            .machines
            .iter()
            .find(|m| m.byte == "$10")
            .expect("the index byte is a state byte");
        let states: Vec<u8> = m.states.iter().map(|s| s.value).collect();
        assert_eq!(states, [0, 1, 2]);
    }

    /// A frame loop that calls a routine writing $0600,X, and an NMI that
    /// copies page `dma` to the PPU.
    fn sprite_page_rom(dma: u8) -> Vec<u8> {
        image(
            &[
                // C000: JSR C010; JMP C000
                (0xC000, &[0x20, 0x10, 0xC0, 0x4C, 0x00, 0xC0]),
                // C010: LDX #0; LDA $10; STA $0600,X; RTS
                (0xC010, &[0xA2, 0x00, 0xA5, 0x10, 0x9D, 0x00, 0x06, 0x60]),
                // C020: LDA #dma; STA $4014; RTI
                (0xC020, &[0xA9, dma, 0x8D, 0x14, 0x40, 0x40]),
            ],
            0xC000,
            0xC020,
        )
    }

    #[test]
    fn the_sprite_table_is_the_page_the_code_copies_by_dma() {
        let element = |dma: u8| {
            let h = harvest(&sprite_page_rom(dma), &Input::default());
            h.routines
                .iter()
                .find(|r| r.name == "sub_C010")
                .map(|r| r.element.clone())
                .expect("the routine")
        };
        assert_eq!(element(0x06), "draw");
        assert_ne!(element(0x02), "draw");
    }

    /// A frame loop that calls a jump engine with the byte at $10; the two
    /// words after the call are its table. Each entry calls a routine that
    /// sets the byte to the other value and jumps back to the loop.
    fn jump_engine_rom() -> Vec<u8> {
        image(
            &[
                // C000: LDA #0; STA $10
                (0xC000, &[0xA9, 0x00, 0x85, 0x10]),
                // C004: LDA $10; JSR C080; .word C00D, C013
                (
                    0xC004,
                    &[0xA5, 0x10, 0x20, 0x80, 0xC0, 0x0D, 0xC0, 0x13, 0xC0],
                ),
                // C00D: JSR C050; JMP C004
                (0xC00D, &[0x20, 0x50, 0xC0, 0x4C, 0x04, 0xC0]),
                // C013: JSR C060; JMP C004
                (0xC013, &[0x20, 0x60, 0xC0, 0x4C, 0x04, 0xC0]),
                // C050: LDA #1; STA $10; RTS
                (0xC050, &[0xA9, 0x01, 0x85, 0x10, 0x60]),
                // C060: LDA #0; STA $10; RTS
                (0xC060, &[0xA9, 0x00, 0x85, 0x10, 0x60]),
                // C080: ASL A; TAY; PLA; STA $00; PLA; STA $01; INY;
                //       LDA ($00),Y; STA $02; INY; LDA ($00),Y; STA $03;
                //       JMP ($0002)
                (
                    0xC080,
                    &[
                        0x0A, 0xA8, 0x68, 0x85, 0x00, 0x68, 0x85, 0x01, 0xC8, 0xB1, 0x00, 0x85,
                        0x02, 0xC8, 0xB1, 0x00, 0x85, 0x03, 0x6C, 0x02, 0x00,
                    ],
                ),
                // C0A0: RTI
                (0xC0A0, &[0x40]),
            ],
            0xC000,
            0xC0A0,
        )
    }

    #[test]
    fn a_jump_engine_reads_the_table_after_its_call() {
        let h = harvest(&jump_engine_rom(), &Input::default());
        let t = h
            .dispatch
            .iter()
            .find(|d| d.kind == "jump table")
            .expect("the table");
        assert_eq!(t.at, "C092");
        assert_eq!(t.subject, "$10");
        let targets: Vec<&str> = t.cases.values().flatten().map(String::as_str).collect();
        assert_eq!(targets, ["sub_C00D", "sub_C013"]);
        // The words after the call are data, not code.
        assert!(h.routines.iter().all(|r| r.addr != "C009"));
        let c = h
            .dispatch
            .iter()
            .find(|d| d.kind == "compare chain")
            .expect("the dispatch by value");
        assert_eq!(c.subject, "$10");
        assert_eq!(c.cases.get("0"), Some(&vec!["sub_C050".to_owned()]));
        assert_eq!(c.cases.get("1"), Some(&vec!["sub_C060".to_owned()]));
        let m = h
            .machines
            .iter()
            .find(|m| m.byte == "$10")
            .expect("the state byte's machine");
        let states: Vec<u8> = m.states.iter().map(|s| s.value).collect();
        assert_eq!(states, [0, 1]);
    }

    /// The engine above called from a routine the frame loop calls; each
    /// entry is a routine that sets the byte and returns, to the loop.
    fn returning_engine_rom() -> Vec<u8> {
        image(
            &[
                // C000: JSR C020; JMP C000
                (0xC000, &[0x20, 0x20, 0xC0, 0x4C, 0x00, 0xC0]),
                // C020: LDA $10; JSR C080; .word C050, C060
                (
                    0xC020,
                    &[0xA5, 0x10, 0x20, 0x80, 0xC0, 0x50, 0xC0, 0x60, 0xC0],
                ),
                // C050: LDA #1; STA $10; RTS
                (0xC050, &[0xA9, 0x01, 0x85, 0x10, 0x60]),
                // C060: LDA #0; STA $10; RTS
                (0xC060, &[0xA9, 0x00, 0x85, 0x10, 0x60]),
                // C080: the engine
                (
                    0xC080,
                    &[
                        0x0A, 0xA8, 0x68, 0x85, 0x00, 0x68, 0x85, 0x01, 0xC8, 0xB1, 0x00, 0x85,
                        0x02, 0xC8, 0xB1, 0x00, 0x85, 0x03, 0x6C, 0x02, 0x00,
                    ],
                ),
                // C0A0: RTI
                (0xC0A0, &[0x40]),
            ],
            0xC000,
            0xC0A0,
        )
    }

    #[test]
    fn a_jump_engines_entries_return_for_the_routine_that_called_it() {
        let h = harvest(&returning_engine_rom(), &Input::default());
        let t = h
            .dispatch
            .iter()
            .find(|d| d.kind == "jump table")
            .expect("the table");
        assert_eq!(t.subject, "$10");
        let targets: Vec<&str> = t.cases.values().flatten().map(String::as_str).collect();
        assert_eq!(targets, ["sub_C050", "sub_C060"]);
        assert!(h.routines.iter().all(|r| r.addr != "C025"));
        let m = h
            .machines
            .iter()
            .find(|m| m.byte == "$10")
            .expect("the state byte's machine");
        let states: Vec<u8> = m.states.iter().map(|s| s.value).collect();
        assert_eq!(states, [0, 1]);
    }

    #[test]
    fn a_table_ends_at_its_index_bytes_last_value() {
        // The inline table's two words are followed by two more that point
        // at code; $10 is only ever 0 or 1, so the table has two entries.
        let mut img = inline_table_rom();
        img[0xC034 - 0x8000..0xC038 - 0x8000].copy_from_slice(&[0x40, 0xC0, 0x50, 0xC0]);
        let h = harvest(&img, &Input::default());
        let t = h
            .dispatch
            .iter()
            .find(|d| d.kind == "jump table")
            .expect("the table");
        let targets: Vec<&str> = t.cases.values().flatten().map(String::as_str).collect();
        assert_eq!(targets, ["sub_C015", "sub_C01B"]);
    }

    #[test]
    fn a_table_read_at_an_index_a_branch_chose_reads_one_entry_per_path() {
        let img = image(
            &[
                // C000: JSR C010; JMP C000
                (0xC000, &[0x20, 0x10, 0xC0, 0x4C, 0x00, 0xC0]),
                // C010: LDX #0; LDA $10; AND #$40; BEQ +1; INX;
                //       LDA C030,X; STA $20; RTS
                (
                    0xC010,
                    &[
                        0xA2, 0x00, 0xA5, 0x10, 0x29, 0x40, 0xF0, 0x01, 0xE8, 0xBD, 0x30, 0xC0,
                        0x85, 0x20, 0x60,
                    ],
                ),
                // C030: the table
                (0xC030, &[0x01, 0x02]),
                // C040: RTI
                (0xC040, &[0x40]),
            ],
            0xC000,
            0xC040,
        );
        let code = RomCode::of(&img);
        let mut r = Reader::new(&code);
        r.run_vectors();
        let written: std::collections::BTreeSet<u8> = r
            .acts
            .iter()
            .flat_map(|a| a.events.iter())
            .filter_map(|e| match e {
                absint::Ev::Write { loc, val, .. } if *loc == absint::Loc::at(0x20) => val.konst(),
                _ => None,
            })
            .collect();
        assert_eq!(written.into_iter().collect::<Vec<_>>(), [1, 2]);
    }

    /// A 64 KB board: `select` in the last bank at $C000 (the reset code,
    /// which then calls $8000 and loops), a routine at $8000 in each other
    /// 16 KB bank writing the bank's number plus 4 to $10 (8 KB banks on
    /// MMC3: at the start of each), and an RTI for the NMI.
    fn banked_board(mapper: u16, select: &[u8]) -> crate::codemap::Board {
        let mut prg = vec![0xFFu8; 0x10000];
        let bank = if mapper == 4 { 0x2000 } else { 0x4000 };
        for b in 0..(0xC000 / bank) {
            // LDA #(b+4); STA $10; RTS
            prg[b * bank..b * bank + 5].copy_from_slice(&[0xA9, b as u8 + 4, 0x85, 0x10, 0x60]);
        }
        let fixed = 0xC000;
        let mut code = select.to_vec();
        // JSR $8000; JMP to itself
        let here = 0xC000 + code.len() as u16 + 3;
        code.extend([0x20, 0x00, 0x80, 0x4C]);
        code.extend(here.to_le_bytes());
        prg[fixed..fixed + code.len()].copy_from_slice(&code);
        prg[0xFF00] = 0x40;
        prg[0xFFFA..0xFFFC].copy_from_slice(&[0x00, 0xFF]);
        prg[0xFFFC..0xFFFE].copy_from_slice(&[0x00, 0xC0]);
        prg[0xFFFE..0x10000].copy_from_slice(&[0x00, 0xFF]);
        crate::codemap::Board::of(&prg, mapper)
    }

    /// The constants the reader saw written to $10, and the harvest.
    fn written_10(board: &crate::codemap::Board) -> (Vec<u8>, Harvest) {
        let h = harvest_board(board, &Input::default());
        let code = RomCode::of_board(board, &[0x2_8000, 0x3_8000, 0x4_8000], &[]);
        let mut r = Reader::new(&code);
        r.run_vectors();
        let w: std::collections::BTreeSet<u8> = r
            .acts
            .iter()
            .flat_map(|a| a.events.iter())
            .filter_map(|e| match e {
                absint::Ev::Write { loc, val, .. } if *loc == absint::Loc::at(0x10) => val.konst(),
                _ => None,
            })
            .collect();
        (w.into_iter().collect(), h)
    }

    #[test]
    fn uxrom_calls_go_to_the_bank_the_code_selected() {
        // LDA #1; STA $C100 (the bank register: any write to ROM)
        let (w, h) = written_10(&banked_board(2, &[0xA9, 0x01, 0x8D, 0x00, 0xC1]));
        assert_eq!(w, [5], "bank 1's routine, not bank 0's");
        assert!(h.routines.iter().any(|r| r.name == "sub_01_8000"));
    }

    #[test]
    fn mmc1_takes_five_serial_writes_to_e000() {
        // LDA #1; STA $E000; LSR A; STA $E000 (three more times)
        let mut sel = vec![0xA9, 0x01];
        for _ in 0..5 {
            sel.extend([0x8D, 0x00, 0xE0, 0x4A]);
        }
        let (w, h) = written_10(&banked_board(1, &sel));
        assert_eq!(w, [5]);
        assert!(h.routines.iter().any(|r| r.name == "sub_01_8000"));
    }

    #[test]
    fn mmc3_register_6_maps_8000() {
        // LDA #6; STA $8000; LDA #2; STA $8001 (8 KB bank 2 at $8000)
        let (w, h) = written_10(&banked_board(
            4,
            &[0xA9, 0x06, 0x8D, 0x00, 0x80, 0xA9, 0x02, 0x8D, 0x01, 0x80],
        ));
        assert_eq!(w, [6]);
        assert!(h.routines.iter().any(|r| r.name == "sub_02_8000"));
    }

    #[test]
    fn a_call_into_a_bank_not_known_is_an_open_fact_and_not_read() {
        // Nothing selected: the bank at $8000 isn't known.
        let (w, h) = written_10(&banked_board(2, &[0xEA]));
        assert!(w.is_empty(), "{w:?}");
        assert!(
            h.open
                .iter()
                .any(|o| o.what.starts_with("call into a switched bank") && o.pc == "C001"),
            "{:?}",
            h.open.iter().map(|o| &o.what).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_jump_through_a_ram_pointer_goes_where_the_program_points_it() {
        let img = image(
            &[
                // C000: LDA #$40; STA $20; LDA #$C0; STA $21; JMP C008
                (
                    0xC000,
                    &[
                        0xA9, 0x40, 0x85, 0x20, 0xA9, 0xC0, 0x85, 0x21, 0x4C, 0x08, 0xC0,
                    ],
                ),
                // C020: JMP ($0020), the NMI
                (0xC020, &[0x6C, 0x20, 0x00]),
                // C040: LDA #7; STA $10; RTI
                (0xC040, &[0xA9, 0x07, 0x85, 0x10, 0x40]),
            ],
            0xC000,
            0xC020,
        );
        let h = harvest(&img, &Input::default());
        assert!(
            h.routines.iter().any(|r| r.name == "sub_C040"),
            "{:?}",
            h.routines.iter().map(|r| &r.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_frame_starts_from_the_ram_only_the_reset_code_writes() {
        // The reset code puts the bank in $20; each frame selects the bank
        // from $20 and calls $8000. Nothing else writes $20.
        let mut board = banked_board(2, &[]);
        let code: &[u8] = &[
            0xA9, 0x01, 0x85, 0x20, // C000: LDA #1; STA $20
            0xA5, 0x20, 0x8D, 0x00, 0xC1, // C004: LDA $20; STA $C100
            0x20, 0x00, 0x80, 0x4C, 0x04, 0xC0, // JSR $8000; JMP C004
        ];
        board.prg[0xC000..0xC000 + code.len()].copy_from_slice(code);
        board.image = crate::codemap::cpu_image(&board.prg);
        // The harvest's later passes know what the frames write.
        let h = harvest_board(&board, &Input::default());
        let names: Vec<&str> = h.routines.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"sub_01_8000"), "{names:?}");
        assert!(!names.contains(&"sub_00_8000"), "{names:?}");
    }

    #[test]
    fn a_bank_a_ram_byte_chooses_goes_to_each_bank_the_byte_takes() {
        // Each frame selects the bank $20 names (through a table of the
        // bank numbers, as UxROM's bus conflicts ask) and calls $8000.
        // Bank 1's routine sets $20 to 2, bank 2's sets it to 1.
        let mut board = banked_board(2, &[]);
        let code: &[u8] = &[
            0xA9, 0x01, 0x85, 0x20, // C000: LDA #1; STA $20
            0xA6, 0x20, 0xBD, 0x00, 0xC1, 0x9D, 0x00,
            0xC1, // C004: LDX $20; LDA C100,X; STA C100,X
            0x20, 0x00, 0x80, 0x4C, 0x04, 0xC0, // JSR $8000; JMP C004
        ];
        board.prg[0xC000..0xC000 + code.len()].copy_from_slice(code);
        board.prg[0xC100..0xC104].copy_from_slice(&[0, 1, 2, 3]);
        for (bank, next) in [(1usize, 2u8), (2, 1)] {
            board.prg[bank * 0x4000..bank * 0x4000 + 5]
                .copy_from_slice(&[0xA9, next, 0x85, 0x20, 0x60]);
        }
        board.image = crate::codemap::cpu_image(&board.prg);
        let h = harvest_board(&board, &Input::default());
        let names: Vec<&str> = h.routines.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"sub_01_8000"), "{names:?}");
        assert!(names.contains(&"sub_02_8000"), "{names:?}");
        assert!(!names.contains(&"sub_00_8000"), "{names:?}");
    }

    #[test]
    fn the_unread_report_names_a_call_into_a_bank_not_known() {
        let (_, u) = harvest_board_unread(&banked_board(2, &[0xEA]), &Input::default());
        let s = u
            .stops
            .iter()
            .find(|s| s.reason == unread::BANK)
            .expect("the stop");
        assert_eq!(s.pc, "C001");
        assert_eq!(s.target, "8000");
        assert!(s.behind >= 1);
        assert!(u.unread >= 1 && u.read + u.unread == u.routines);
    }

    #[test]
    fn the_unread_report_names_a_call_no_path_reaches() {
        let img = image(
            &[
                // C000: JSR C010; JSR C020; RTS
                (0xC000, &[0x20, 0x10, 0xC0, 0x20, 0x20, 0xC0, 0x60]),
                // C010: JMP C010 (never returns)
                (0xC010, &[0x4C, 0x10, 0xC0]),
                // C020: LDA #1; STA $10; RTS
                (0xC020, &[0xA9, 0x01, 0x85, 0x10, 0x60]),
                // C030: RTI
                (0xC030, &[0x40]),
            ],
            0xC000,
            0xC030,
        );
        let (_, u) = harvest_board_unread(&crate::codemap::Board::flat(&img), &Input::default());
        let s = &u.stops[0];
        assert_eq!(
            (s.reason.as_str(), s.pc.as_str(), s.target.as_str()),
            (unread::NOT_REACHED, "C003", "C020")
        );
        assert_eq!(s.examples, ["sub_C020"]);
    }

    #[test]
    fn the_same_rom_gives_the_same_bytes() {
        let a =
            serde_json::to_string(&harvest(&table_rom(), &Input::default())).unwrap_or_default();
        let b =
            serde_json::to_string(&harvest(&table_rom(), &Input::default())).unwrap_or_default();
        assert!(!a.is_empty());
        assert_eq!(a, b);
    }

    #[test]
    fn ines_reads_past_the_trainer_and_names_the_mapper() {
        let mut rom = vec![0u8; 16 + 512 + 0x4000];
        rom[..4].copy_from_slice(b"NES\x1A");
        rom[4] = 1;
        rom[6] = 0x04 | 0x20; // trainer, mapper low nibble 2
        rom[7] = 0x10; // mapper high nibble 1: mapper 18
        rom[16 + 512] = 0xA9;
        let (prg, mapper) = ines_prg(&rom).expect("an iNES file");
        assert_eq!(mapper, 0x12);
        assert_eq!(prg.len(), 0x4000);
        assert_eq!(prg[0], 0xA9);
        assert!(ines_prg(b"not a rom").is_none());
    }

    #[test]
    fn the_cpu_image_mirrors_nrom_128_and_keeps_the_last_bank_of_a_banked_board() {
        let small: Vec<u8> = (0..0x4000u32).map(|i| (i / 0x100) as u8).collect();
        let img = cpu_image(&small);
        assert_eq!(img[0x0100], img[0x4100], "16KB is mirrored at $C000");
        let big: Vec<u8> = (0..0x20000u32).map(|i| (i / 0x4000) as u8).collect();
        let img = cpu_image(&big);
        assert_eq!(img[0], 0, "the first bank at $8000");
        assert_eq!(img[0x4000], 7, "the last bank at $C000");
        assert!(crate::codemap::banked(&big));
        assert!(!crate::codemap::banked(&small));
    }
}
