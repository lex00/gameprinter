//! `gp cycles`: measuring an original's per-frame CPU work, for a model of
//! a random generator stepped in an idle loop (docs/cycles.md). These are
//! analysis commands: they run the original in gp-nes and the cost-only CPU
//! of gp-cycles on a slice of the user's ROM kept under $GP_HOME/work. No
//! rebuild runs any of this. On an arcade romset they read the main Z80
//! from gp-arcade's trace instead (crate::cycles_z80).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use serde_json::json;

use gp_evidence::manifest::PowerOn;
use gp_evidence::{Hash, Paths};
use gp_nes::Nes;

use crate::Outcome;
use crate::run_input::InputArgs;

#[derive(Subcommand)]
pub enum CyclesCmd {
    /// Write the slice gp-cycles runs, the cartridge's program ROM, from the
    /// user's ROM to $GP_HOME/work/cycles/<rom blake3>/: `prg.bin` and
    /// `slice.json` (the ROM's hash, where the program sits). Mapper 0
    /// (NROM) only.
    Slice {
        #[arg(long)]
        rom: PathBuf,
    },
    /// Each call of ROUTINE inside the NMI handler on an input: its cycles
    /// and the RAM bytes it reads before writing them (not the stack). A
    /// routine's cost is a function of these: `distinct read-states` with no
    /// `ambiguous` one shows it.
    Readset {
        #[arg(long)]
        rom: PathBuf,
        #[command(flatten)]
        input: InputArgs,
        #[arg(long)]
        from: u64,
        #[arg(long)]
        to: u64,
        /// The routine's address, hex.
        routine: String,
    },
    /// Each call of each ROUTINE on an input against gp-cycles' cost of it,
    /// run on the RAM, registers and controller at the call.
    Check {
        #[arg(long)]
        rom: PathBuf,
        #[command(flatten)]
        input: InputArgs,
        #[arg(long)]
        from: u64,
        #[arg(long)]
        to: u64,
        /// Routine addresses, hex.
        routines: Vec<String>,
    },
    /// gp-cycles runs the CPU on its own from the NMI at FROM to TO: it
    /// takes each NMI where the vblank grid puts it (89342 PPU dots a
    /// frame, 89341 on an odd frame whose pre-render line has rendering on)
    /// after the right instruction, counts the idle loop's passes at
    /// `--idle`, and keeps the `--keep` bytes as its own while taking the
    /// rest of RAM from the original at each NMI. Reports the frames whose
    /// pass count and NMI entry match, and the first frame the kept bytes
    /// differ.
    Chain {
        #[arg(long)]
        rom: PathBuf,
        #[command(flatten)]
        input: InputArgs,
        #[arg(long)]
        from: u64,
        #[arg(long)]
        to: u64,
        /// The idle loop's first instruction, hex (Donkey Kong: c7e1).
        #[arg(long)]
        idle: String,
        /// RAM bytes the machine keeps, as `lo-hi` hex (Donkey Kong's random
        /// bytes: 18-1f).
        #[arg(long, default_value = "")]
        keep: String,
        /// The RAM the model holds, as comma-separated `lo-hi` or `addr`
        /// hex, or `@FILE` with one such entry per line. Given this or
        /// `--mapping`, each frame takes only these bytes from the
        /// original and holds every other byte at its value at FROM, so
        /// the exact count shows how much of the frame's work the model's
        /// state decides.
        #[arg(long)]
        known: Option<String>,
        /// A state mapping whose fields' `ram` bytes the model holds, added
        /// to `--known`.
        #[arg(long)]
        mapping: Option<PathBuf>,
        /// Start every frame from the original's NMI entry, registers and
        /// `--keep` bytes, so a frame's count is judged on its own RAM and
        /// one wrong frame doesn't carry into the next.
        #[arg(long)]
        resync: bool,
        /// Write the original's RAM at each frame's NMI entry, FROM to TO,
        /// 2048 bytes a frame, to this file. It's game data: keep it under
        /// $GP_HOME/work.
        #[arg(long)]
        ram_out: Option<PathBuf>,
    },
    /// Each frame's NMI handler from FROM to TO split by routine: the
    /// handler's cycles from its entry to its RTI, and each routine it
    /// calls at any depth with the cycles of its calls that frame, callees
    /// included. One JSON line a frame to `--out`, for finding which
    /// routines move the frame's cost.
    Split {
        #[arg(long)]
        rom: PathBuf,
        #[command(flatten)]
        input: InputArgs,
        #[arg(long)]
        from: u64,
        #[arg(long)]
        to: u64,
        #[arg(long)]
        out: PathBuf,
        /// Also write each frame's instructions to this file: for each
        /// address the handler ran, the times it ran and their cycles
        /// (a branch's taken cycles and a page crossing's included), for
        /// finding the branches that move a routine's cost. Little-endian:
        /// per frame the frame (u32) and the count of addresses (u32), then
        /// each address (u16), its runs (u16) and its cycles (u32).
        #[arg(long)]
        pcs: Option<PathBuf>,
        /// Also write the original's RAM at each frame's NMI entry, 2048
        /// bytes a frame. It's game data: keep it under $GP_HOME/work.
        #[arg(long)]
        ram_out: Option<PathBuf>,
    },
}

pub(crate) fn hex(s: &str) -> Result<u16> {
    u16::from_str_radix(s.trim_start_matches("0x").trim_start_matches('$'), 16)
        .with_context(|| format!("`{s}` isn't a hex address"))
}

/// The ROM's program as a gp-cycles slice, and the bytes.
fn load(rom: &PathBuf) -> Result<(Vec<u8>, gp_cycles::Slice)> {
    let bytes = fs::read(rom).with_context(|| format!("reading {}", rom.display()))?;
    if bytes.len() < 16 || &bytes[..4] != b"NES\x1a" {
        bail!("{} isn't an iNES ROM", rom.display());
    }
    let mapper = (bytes[6] >> 4) | (bytes[7] & 0xF0);
    if mapper != 0 {
        bail!("mapper {mapper}: gp cycles reads NROM (mapper 0) only");
    }
    let prg_len = usize::from(bytes[4]) * 0x4000;
    let start = 16 + if bytes[6] & 4 != 0 { 512 } else { 0 };
    let prg = bytes
        .get(start..start + prg_len)
        .context("the ROM is shorter than its header says")?
        .to_vec();
    Ok((bytes.clone(), gp_cycles::Slice { base: 0x8000, prg }))
}

fn peek_ram(bus: &tetanes_core::bus::Bus) -> [u8; 0x800] {
    let mut ram = [0u8; 0x800];
    for (k, b) in ram.iter_mut().enumerate() {
        *b = bus.peek(k as u16);
    }
    ram
}

fn regs_of(bus: &tetanes_core::bus::Bus) -> gp_cycles::Regs {
    gp_cycles::Regs {
        a: bus.cpu.acc,
        x: bus.cpu.x,
        y: bus.cpu.y,
        p: bus.cpu.status.bits(),
        s: bus.cpu.sp,
    }
}

/// A controller read for gp-cycles: the pad's bits in serial order, then 1s.
fn pad_io(pad: u8) -> impl FnMut(u16, u64) -> u8 {
    let mut k = [0u32; 2];
    move |a, _| match a {
        0x4016 | 0x4017 => {
            let p = usize::from(a - 0x4016);
            let b = if k[p] < 8 && p == 0 {
                (pad >> k[p]) & 1
            } else if k[p] < 8 {
                0
            } else {
                1
            };
            k[p] += 1;
            0x40 | b
        }
        _ => 0,
    }
}

fn target_of(bus: &tetanes_core::bus::Bus, pc: u16) -> u16 {
    u16::from(bus.peek(pc.wrapping_add(1))) | u16::from(bus.peek(pc.wrapping_add(2))) << 8
}

/// The command on an arcade romset (crate::cycles_z80), or `None` for an
/// NES image.
fn arcade(cmd: &CyclesCmd, paths: &Paths) -> Result<Option<Outcome>> {
    let rom = match cmd {
        CyclesCmd::Slice { rom }
        | CyclesCmd::Readset { rom, .. }
        | CyclesCmd::Check { rom, .. }
        | CyclesCmd::Chain { rom, .. }
        | CyclesCmd::Split { rom, .. } => rom,
    };
    let bytes = fs::read(rom).with_context(|| format!("reading {}", rom.display()))?;
    if crate::original::Platform::of_file(&bytes) != crate::original::Platform::Arcade {
        return Ok(None);
    }
    use crate::cycles_z80 as z;
    Ok(Some(match cmd {
        CyclesCmd::Slice { .. } => z::slice(&bytes, &paths.home)?,
        CyclesCmd::Readset {
            input,
            from,
            to,
            routine,
            ..
        } => z::readset(&bytes, &input.read(paths)?, (*from, *to), routine)?,
        CyclesCmd::Check {
            input,
            from,
            to,
            routines,
            ..
        } => z::check(&bytes, &input.read(paths)?, (*from, *to), routines)?,
        CyclesCmd::Split {
            input,
            from,
            to,
            out,
            pcs,
            ram_out,
            ..
        } => z::split(
            &bytes,
            &input.read(paths)?,
            (*from, *to),
            out,
            pcs.as_deref(),
            ram_out.as_deref(),
        )?,
        CyclesCmd::Chain {
            input,
            from,
            to,
            idle,
            ..
        } => {
            let passes = z::idle_passes(&bytes, &input.read(paths)?, (*from, *to), hex(idle)?)?;
            let counts: BTreeSet<u32> = passes.iter().map(|p| p.1).collect();
            Outcome {
                json: json!({
                    "frames": passes.len(),
                    "passes": passes.iter().map(|(f, n)| json!({"frame": f, "passes": n})).collect::<Vec<_>>(),
                    "predicted": false,
                }),
                text: format!(
                    "the original's idle loop at {idle}: {} frames, passes a frame {}\nnot predicted: the MCR board's interrupt comes from its CTC counting the video timing, which a Z80 run alone doesn't model (gp cycles check runs calls alone, which need no interrupt)",
                    passes.len(),
                    counts
                        .iter()
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(" ")
                ),
                exit: 0,
            }
        }
    }))
}

pub(crate) fn run(cmd: &CyclesCmd, paths: &Paths) -> Result<Outcome> {
    if let Some(out) = arcade(cmd, paths)? {
        return Ok(out);
    }
    match cmd {
        CyclesCmd::Slice { rom } => {
            let (bytes, slice) = load(rom)?;
            let hash = Hash::of(&bytes).to_string();
            let dir = paths.home.join("work").join("cycles").join(&hash);
            fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
            fs::write(dir.join("prg.bin"), &slice.prg)?;
            let meta = json!({"rom_blake3": hash, "base": slice.base, "prg_len": slice.prg.len()});
            fs::write(dir.join("slice.json"), serde_json::to_string_pretty(&meta)?)?;
            Ok(Outcome {
                json: meta,
                text: format!("wrote {}", dir.display()),
                exit: 0,
            })
        }
        CyclesCmd::Readset {
            rom,
            input,
            from,
            to,
            routine,
        } => {
            use gp_events::footprint::{reads_memory, target};
            use tetanes_core::cpu::Cpu;
            let (bytes, _) = load(rom)?;
            let log = input.read(paths)?;
            let r = hex(routine)?;
            let mut nes = Nes::power_on(&bytes, PowerOn::default()).map_err(anyhow::Error::msg)?;
            let mut costs: BTreeMap<Vec<(u16, u8)>, BTreeSet<u64>> = BTreeMap::new();
            let mut addrs: BTreeSet<u16> = BTreeSet::new();
            let mut calls = 0u64;
            for (i, fr) in log.frames.iter().enumerate().take(*to as usize) {
                if (i as u64) + 1 < *from {
                    nes.step(*fr).map_err(anyhow::Error::msg)?;
                    continue;
                }
                let mut open: Option<OpenCall> = None;
                let mut done: Vec<(u64, Vec<(u16, u8)>)> = vec![];
                nes.step_with(*fr, |bus| {
                    let pc = bus.cpu.pc;
                    let c = bus.cpu.cycle;
                    let op = bus.peek(pc);
                    match &mut open {
                        None => {
                            if op == 0x20 && target_of(bus, pc) == r {
                                open =
                                    Some((c, bus.cpu.sp.wrapping_sub(2), vec![], BTreeSet::new()));
                            }
                        }
                        Some((c0, sp, reads, seen)) => {
                            let ins = Cpu::INSTR_REF[usize::from(op)];
                            if reads_memory(ins.instr, ins.addr_mode)
                                && let Some(t) = target(bus, pc, ins.instr, ins.addr_mode)
                                && t < 0x800
                                && !(0x100..0x200).contains(&t)
                                && seen.insert(t)
                            {
                                reads.push((t, bus.peek(t)));
                            }
                            if op == 0x60 && bus.cpu.sp == *sp {
                                done.push((c + 6 - *c0, std::mem::take(reads)));
                                open = None;
                            }
                        }
                    }
                })
                .map_err(anyhow::Error::msg)?;
                for (cyc, reads) in done {
                    calls += 1;
                    addrs.extend(reads.iter().map(|(a, _)| *a));
                    costs.entry(reads).or_default().insert(cyc);
                }
            }
            let ambiguous = costs.values().filter(|v| v.len() > 1).count();
            let list: Vec<String> = addrs.iter().map(|a| format!("{a:04x}")).collect();
            Ok(Outcome {
                json: json!({"calls": calls, "read_states": costs.len(), "ambiguous": ambiguous, "bytes": list}),
                text: format!(
                    "{calls} calls, {} distinct read-states, {ambiguous} ambiguous; {} bytes read: {}",
                    costs.len(),
                    list.len(),
                    list.join(" ")
                ),
                exit: 0,
            })
        }
        CyclesCmd::Check {
            rom,
            input,
            from,
            to,
            routines,
        } => {
            let (bytes, slice) = load(rom)?;
            let log = input.read(paths)?;
            let rs: Vec<u16> = routines.iter().map(|s| hex(s)).collect::<Result<_>>()?;
            let mut nes = Nes::power_on(&bytes, PowerOn::default()).map_err(anyhow::Error::msg)?;
            let mut stats: BTreeMap<u16, (u64, u64)> = BTreeMap::new();
            for (i, fr) in log.frames.iter().enumerate().take(*to as usize) {
                if (i as u64) + 1 < *from {
                    nes.step(*fr).map_err(anyhow::Error::msg)?;
                    continue;
                }
                let pad = fr.pads[0];
                let mut open: Vec<(u16, u64, u64, u8)> = vec![];
                let mut done: Vec<(u16, u64, u64)> = vec![];
                nes.step_with(*fr, |bus| {
                    let pc = bus.cpu.pc;
                    let c = bus.cpu.cycle;
                    let op = bus.peek(pc);
                    if op == 0x20 {
                        let t = target_of(bus, pc);
                        if rs.contains(&t) {
                            let mut regs = regs_of(bus);
                            regs.s = regs.s.wrapping_sub(2);
                            let mut io = pad_io(pad);
                            let pred = gp_cycles::cost_io(
                                &slice,
                                t,
                                &peek_ram(bus),
                                regs,
                                1_000_000,
                                &mut io,
                            )
                            .unwrap_or(0);
                            open.push((t, c, pred, regs.s));
                        }
                    } else if op == 0x60
                        && let Some(&(t, c0, pred, sp)) = open.last()
                        && bus.cpu.sp == sp
                    {
                        open.pop();
                        done.push((t, c + 6 - c0, pred));
                    }
                })
                .map_err(anyhow::Error::msg)?;
                for (t, real, pred) in done {
                    let e = stats.entry(t).or_default();
                    e.0 += 1;
                    e.1 += u64::from(real == pred);
                }
            }
            let lines: Vec<String> = stats
                .iter()
                .map(|(t, (n, ok))| format!("{t:04x}: {ok} of {n} calls exact"))
                .collect();
            let j: Vec<_> = stats
                .iter()
                .map(|(t, (n, ok))| json!({"routine": format!("{t:04x}"), "calls": n, "exact": ok}))
                .collect();
            Ok(Outcome {
                json: json!({"routines": j}),
                text: lines.join("\n"),
                exit: 0,
            })
        }
        CyclesCmd::Chain {
            rom,
            input,
            from,
            to,
            idle,
            keep,
            known,
            mapping,
            resync,
            ram_out,
        } => {
            let known = known_bytes(known.as_deref(), mapping.as_deref())?;
            let log = input.read(paths)?;
            let opts = ChainOpts {
                keep,
                known: known.as_ref(),
                resync: *resync,
                ram_out: ram_out.as_deref(),
            };
            chain(rom, &log, (*from, *to), hex(idle)?, &opts)
        }
        CyclesCmd::Split {
            rom,
            input,
            from,
            to,
            out,
            pcs,
            ram_out,
        } => {
            let log = input.read(paths)?;
            split(
                rom,
                &log,
                (*from, *to),
                out,
                pcs.as_deref(),
                ram_out.as_deref(),
            )
        }
    }
}

/// Closes the open calls a routine left by pulling its return address
/// (Donkey Kong's $E0F1 at $E107: PLA, PLA, then an RTS to its caller's
/// caller): at an RTS or RTI with the stack pointer at `sp`, each call
/// whose return address sat below it has ended, its cycles to the end of
/// this instruction counted. Without it the handler's call stack never
/// empties and the frame reads as no handler at all.
fn close_left(open: &mut Vec<(u16, u64, u8)>, by: &mut BTreeMap<u16, u64>, sp: u8, c: u64) {
    while let Some(&(t, c1, s)) = open.last()
        && s < sp
    {
        open.pop();
        *by.entry(t).or_default() += c + 6 - c1;
    }
}

/// `gp cycles split`: each frame's handler cycles by routine.
fn split(
    rom: &PathBuf,
    log: &gp_evidence::InputLog,
    (from, to): (u64, u64),
    out: &std::path::Path,
    pcs: Option<&std::path::Path>,
    ram_out: Option<&std::path::Path>,
) -> Result<Outcome> {
    let (bytes, slice) = load(rom)?;
    let mut pcs_bytes: Vec<u8> = vec![];
    let mut ram_bytes: Vec<u8> = vec![];
    let vector = {
        let w = |a: u16| slice.prg[usize::from(a - slice.base) % slice.prg.len()];
        u16::from(w(0xFFFA)) | u16::from(w(0xFFFB)) << 8
    };
    let mut nes = Nes::power_on(&bytes, PowerOn::default()).map_err(anyhow::Error::msg)?;
    let mut lines = String::new();
    let mut n = 0u64;
    for (i, fr) in log.frames.iter().enumerate().take(to as usize) {
        if (i as u64) + 1 < from {
            nes.step(*fr).map_err(anyhow::Error::msg)?;
            continue;
        }
        let mut entry: Option<(u64, u8)> = None;
        let mut whole = 0u64;
        let mut open: Vec<(u16, u64, u8)> = vec![];
        let mut by: BTreeMap<u16, u64> = BTreeMap::new();
        let mut hist: BTreeMap<u16, (u32, u64)> = BTreeMap::new();
        let mut last: Option<(u16, u64)> = None;
        let mut ram_at: Option<[u8; 0x800]> = None;
        let want_ram = ram_out.is_some();
        nes.step_with(*fr, |bus| {
            let pc = bus.cpu.pc;
            let c = bus.cpu.cycle;
            if pc == vector && entry.is_none() {
                entry = Some((c, bus.cpu.sp));
                if want_ram {
                    ram_at = Some(peek_ram(bus));
                }
            }
            let Some((c0, _)) = entry else { return };
            if whole != 0 {
                return;
            }
            if let Some((lp, lc)) = last.take() {
                let h = hist.entry(lp).or_default();
                h.0 += 1;
                h.1 += c - lc;
            }
            last = Some((pc, c));
            match bus.peek(pc) {
                0x20 => open.push((target_of(bus, pc), c, bus.cpu.sp.wrapping_sub(2))),
                0x60 => {
                    close_left(&mut open, &mut by, bus.cpu.sp, c);
                    if let Some(&(t, c1, sp)) = open.last()
                        && bus.cpu.sp == sp
                    {
                        open.pop();
                        *by.entry(t).or_default() += c + 6 - c1;
                    }
                }
                0x40 if {
                    close_left(&mut open, &mut by, bus.cpu.sp, c);
                    open.is_empty()
                } =>
                {
                    whole = c + 6 - c0;
                    let h = hist.entry(pc).or_default();
                    h.0 += 1;
                    h.1 += 6;
                    last = None;
                }
                _ => {}
            }
        })
        .map_err(anyhow::Error::msg)?;
        if pcs.is_some() {
            pcs_bytes.extend_from_slice(&((i + 1) as u32).to_le_bytes());
            pcs_bytes.extend_from_slice(&(hist.len() as u32).to_le_bytes());
            for (pc, (k, cy)) in &hist {
                pcs_bytes.extend_from_slice(&pc.to_le_bytes());
                pcs_bytes.extend_from_slice(&(*k as u16).to_le_bytes());
                pcs_bytes.extend_from_slice(&(*cy as u32).to_le_bytes());
            }
        }
        if want_ram {
            ram_bytes.extend_from_slice(&ram_at.unwrap_or([0u8; 0x800]));
        }
        let routines: BTreeMap<String, u64> =
            by.iter().map(|(t, c)| (format!("{t:04x}"), *c)).collect();
        lines.push_str(&serde_json::to_string(
            &json!({"frame": i + 1, "handler": whole, "routines": routines}),
        )?);
        lines.push('\n');
        n += 1;
    }
    fs::write(out, lines).with_context(|| format!("writing {}", out.display()))?;
    if let Some(p) = pcs {
        fs::write(p, &pcs_bytes).with_context(|| format!("writing {}", p.display()))?;
    }
    if let Some(p) = ram_out {
        fs::write(p, &ram_bytes).with_context(|| format!("writing {}", p.display()))?;
    }
    Ok(Outcome {
        json: json!({"frames": n, "out": out.display().to_string()}),
        text: format!("{n} frames written to {}", out.display()),
        exit: 0,
    })
}

/// The bytes `--known` and `--mapping` name, or none when neither is given.
fn known_bytes(
    known: Option<&str>,
    mapping: Option<&std::path::Path>,
) -> Result<Option<Vec<bool>>> {
    if known.is_none() && mapping.is_none() {
        return Ok(None);
    }
    let mut set = vec![false; 0x800];
    let mut add = |lo: u16, hi: u16| {
        for a in lo..=hi {
            if let Some(b) = set.get_mut(usize::from(a)) {
                *b = true;
            }
        }
    };
    if let Some(k) = known {
        let text = match k.strip_prefix('@') {
            Some(f) => fs::read_to_string(f).with_context(|| format!("reading {f}"))?,
            None => k.replace(',', "\n"),
        };
        for item in text
            .lines()
            .map(|l| l.split('#').next().unwrap_or("").trim())
        {
            if item.is_empty() {
                continue;
            }
            match item.split_once('-') {
                Some((a, b)) => add(hex(a)?, hex(b)?),
                None => add(hex(item)?, hex(item)?),
            }
        }
    }
    if let Some(m) = mapping {
        let text = fs::read_to_string(m).with_context(|| format!("reading {}", m.display()))?;
        let doc: toml::Value = toml::from_str(&text).with_context(|| format!("{}", m.display()))?;
        let mut stack = vec![&doc];
        while let Some(v) = stack.pop() {
            match v {
                toml::Value::Table(t) => {
                    if let Some(toml::Value::Array(a)) = t.get("ram") {
                        for x in a.iter().filter_map(toml::Value::as_integer) {
                            let x = u16::try_from(x).context("a `ram` byte out of range")?;
                            add(x, x);
                        }
                    }
                    stack.extend(t.values());
                }
                toml::Value::Array(a) => stack.extend(a.iter()),
                _ => {}
            }
        }
    }
    Ok(Some(set))
}

/// A call being watched: its first cycle, the stack pointer inside it, the
/// bytes read so far in order, and the addresses seen.
type OpenCall = (u64, u8, Vec<(u16, u8)>, BTreeSet<u16>);

/// `gp cycles chain`'s options past the run and the idle loop.
#[derive(Clone, Copy)]
struct ChainOpts<'a> {
    keep: &'a str,
    known: Option<&'a Vec<bool>>,
    resync: bool,
    ram_out: Option<&'a std::path::Path>,
}

struct FrameData {
    entry: u64,
    ram: [u8; 0x800],
    regs: gp_cycles::Regs,
    pad: u8,
    passes: u32,
    /// The vblank in thirds of a CPU cycle, when the trace shows it (the
    /// NMI came right after the instruction it fell in), else 0.
    vblank: u64,
}

fn chain(
    rom: &PathBuf,
    log: &gp_evidence::InputLog,
    (from, to): (u64, u64),
    idle: u16,
    opts: &ChainOpts<'_>,
) -> Result<Outcome> {
    let ChainOpts {
        keep,
        known,
        resync,
        ram_out,
    } = *opts;
    let (bytes, slice) = load(rom)?;
    let keep: Option<(usize, usize)> = match keep.split_once('-') {
        Some((a, b)) => Some((usize::from(hex(a)?), usize::from(hex(b)?))),
        None => None,
    };
    let mut nes = Nes::power_on(&bytes, PowerOn::default()).map_err(anyhow::Error::msg)?;
    let vector = {
        let w = |a: u16| slice.prg[usize::from(a - slice.base) % slice.prg.len()];
        u16::from(w(0xFFFA)) | u16::from(w(0xFFFB)) << 8
    };
    let mut frames: Vec<FrameData> = vec![];
    let mut after_last = 0u32;
    for (i, fr) in log.frames.iter().enumerate().take(to as usize) {
        if (i as u64) + 1 < from {
            nes.step(*fr).map_err(anyhow::Error::msg)?;
            continue;
        }
        let mut snap = None;
        let (mut after, mut before) = (0u32, 0u32);
        let mut prev = (0u64, 0u16, 0u16);
        let mut v3 = 0u64;
        nes.step_with(*fr, |bus| {
            let pc = bus.cpu.pc;
            if pc == vector && snap.is_none() {
                snap = Some((bus.cpu.cycle, peek_ram(bus), regs_of(bus)));
                if prev.1 == 241 {
                    v3 = 3 * prev.0 - (u64::from(prev.2).saturating_sub(1));
                }
            }
            if pc == idle {
                if snap.is_some() {
                    after += 1;
                } else {
                    before += 1;
                }
            }
            prev = (bus.cpu.cycle, bus.ppu.scanline, bus.ppu.cycle);
        })
        .map_err(anyhow::Error::msg)?;
        if let Some(last) = frames.last_mut() {
            last.passes = after_last + before;
        }
        after_last = after;
        let Some((entry, ram, regs)) = snap else {
            bail!("frame {}: no NMI", i + 1);
        };
        frames.push(FrameData {
            entry,
            ram,
            regs,
            pad: fr.pads[0],
            passes: 0,
            vblank: v3,
        });
    }
    if let Some(out) = ram_out {
        let bytes: Vec<u8> = frames.iter().flat_map(|f| f.ram).collect();
        fs::write(out, bytes).with_context(|| format!("writing {}", out.display()))?;
    }
    let Some((k0, v0)) = frames
        .iter()
        .enumerate()
        .find(|(_, d)| d.vblank != 0)
        .map(|(k, d)| (k, d.vblank))
    else {
        bail!("no frame shows the vblank's place");
    };
    // The grid back to the first frame: the odd frame comes first when the
    // first frame's number is even, as from power-on.
    let mut odd = from.is_multiple_of(2);
    let mut v_cur = v0;
    for j in (0..k0).rev() {
        // Frame j is odd when j's parity matches the first frame's.
        let odd_j = (j % 2 == 0) == odd;
        v_cur -= if odd_j { 89341 } else { 89342 };
    }
    let (mut ok, mut entries_ok, mut differ) = (0u64, 0u64, None);
    let held = frames[0].ram;
    let mut ram = frames[0].ram;
    let mut regs = frames[0].regs;
    let (mut entry, mut pc) = (frames[0].entry, vector);
    let mut render = true;
    for (k, frame) in frames.iter().enumerate().take(frames.len() - 1) {
        let mine = ram;
        ram = frame.ram;
        if let Some(known) = known {
            for (a, b) in ram.iter_mut().enumerate() {
                if !known[a] {
                    *b = held[a];
                }
            }
        }
        if resync {
            regs = frame.regs;
            entry = frame.entry;
            pc = vector;
            // The stack above S is the CPU's too: the NMI's pushed status
            // and return address, which say where in the idle loop the
            // frame resumes.
            let top = 0x100 + usize::from(regs.s) + 1;
            ram[top..0x200].copy_from_slice(&frame.ram[top..0x200]);
            if let Some((lo, hi)) = keep {
                ram[lo..=hi].copy_from_slice(&frame.ram[lo..=hi]);
            }
        } else if k > 0 {
            if let Some((lo, hi)) = keep {
                if differ.is_none() && mine[lo..=hi] != ram[lo..=hi] {
                    differ = Some(from + k as u64);
                }
                ram[lo..=hi].copy_from_slice(&mine[lo..=hi]);
            }
            entries_ok += u64::from(entry == frame.entry);
        }
        let pre = v_cur + 20 * 341;
        let writes = std::cell::RefCell::new(Vec::<(u64, u8)>::new());
        let v_next = std::cell::Cell::new(0u64);
        let mut vread = false;
        let mut pad = pad_io(frame.pad);
        let mut io = |a: u16, c: u64| match a {
            0x2002 => {
                let v = v_next.get();
                if !vread && v != 0 && 3 * c + 3 >= v {
                    vread = true;
                    0x80
                } else {
                    0
                }
            }
            _ => pad(a, c),
        };
        let mut on_write = |a: u16, v: u8, c: u64| {
            if a == 0x2001 {
                writes.borrow_mut().push((c, v));
            }
            if a == 0x4014 { 513 + ((c + 1) & 1) } else { 0 }
        };
        let mut m = gp_cycles::Machine::new(&slice, &ram, regs, pc, entry, &mut io, &mut on_write);
        let (mut passes, mut in_handler) = (0u32, true);
        for _ in 0..1_000_000 {
            if m.pc() == idle && !in_handler {
                passes += 1;
            }
            let at = m.pc();
            if m.step().is_err() {
                break;
            }
            if in_handler && slice.prg[usize::from(at - slice.base) % slice.prg.len()] == 0x40 {
                in_handler = false;
            }
            if v_next.get() == 0 && 3 * m.cycle() >= pre {
                let mut on = render;
                for &(c, v) in writes.borrow().iter() {
                    if 3 * c <= pre {
                        on = v & 0x18 != 0;
                    }
                }
                render = on;
                v_next.set(v_cur + if odd && on { 89341 } else { 89342 });
            }
            let v = v_next.get();
            // An NMI is taken after the instruction its vblank fell in
            // before that instruction's last cycle.
            if v != 0 && !in_handler && 3 * m.cycle() > v + 3 {
                m.nmi(vector);
                break;
            }
        }
        ok += u64::from(passes == frame.passes);
        ram = *m.ram();
        regs = m.regs();
        entry = m.cycle();
        pc = m.pc();
        for &(_, v) in writes.borrow().iter() {
            render = v & 0x18 != 0;
        }
        v_cur = v_next.get();
        odd = !odd;
    }
    let n = frames.len() as u64 - 1;
    let n_known = known.map(|k| k.iter().filter(|b| **b).count());
    Ok(Outcome {
        json: json!({"frames": n, "passes_exact": ok, "nmi_exact": entries_ok, "kept_differ_at": differ, "known_bytes": n_known}),
        text: format!(
            "{ok} of {n} frames' idle passes exact; {entries_ok} NMI entries exact; kept bytes first differ at {}{}",
            differ.map_or("none".to_string(), |f| f.to_string()),
            n_known.map_or(String::new(), |k| format!(
                "; {k} bytes known, the rest held"
            ))
        ),
        exit: u8::from(ok != n),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A call left by pulling its return address ends at its caller's RTS,
    /// with the caller, and the RTI then finds the stack empty.
    #[test]
    fn a_call_left_by_its_return_address_closes_with_its_caller() {
        // JSR $E0AE at cycle 100 (S $FB after the push), then JSR $E0F1 at
        // 110 (S $F9); $E0F1's PLA PLA takes S back to $FB, and the RTS at
        // 130 returns from $E0AE.
        let mut open = vec![(0xE0AE, 100, 0xFB), (0xE0F1, 110, 0xF9)];
        let mut by = BTreeMap::new();
        close_left(&mut open, &mut by, 0xFB, 130);
        assert_eq!(open, vec![(0xE0AE, 100, 0xFB)]);
        assert_eq!(by.get(&0xE0F1), Some(&26));
        // A balanced call is left for the RTS to match.
        close_left(&mut open, &mut by, 0xFB, 130);
        assert_eq!(open.len(), 1);
    }

    #[test]
    fn known_bytes_take_ranges_and_a_mappings_ram() {
        assert!(known_bytes(None, None).unwrap().is_none());
        let k = known_bytes(Some("18-1f,f0"), None).unwrap().unwrap();
        let on: Vec<usize> = (0..0x800).filter(|a| k[*a]).collect();
        assert_eq!(
            on,
            vec![0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0xf0]
        );
        let dir = std::env::temp_dir().join(format!("gp-known-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let m = dir.join("mapping.toml");
        fs::write(&m, "[[field]]\npath = \"x\"\nram = [0x203, 0x0102]\n").unwrap();
        let k = known_bytes(None, Some(&m)).unwrap().unwrap();
        assert!(k[0x203] && k[0x102] && !k[0x18]);
    }
}
