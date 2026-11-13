//! Mesen 2 as the oracle: which build, how it's configured from a
//! recording's power-on state, and how its capture becomes states with a
//! field table of their own.
//!
//! The field table uses the canonical state's names (docs/formats/
//! recording.md) for every field Mesen shows with the same meaning, after
//! the conversions in [`encode`], and leaves out the ones it doesn't show.
//! `gp oracle diff` compares fields by name, so a field one side lacks is
//! listed as not compared rather than reported as a difference.

use gp_evidence::events::{Event, EventKind};
use gp_evidence::manifest::{Bf909Revision, FourPlayer, Mmc3Revision, PowerOn, RamInit, Region};
use gp_evidence::trace::{FieldDef, FieldTable, Width};

use crate::capture::{Frame, RawEvent};

/// SourMesen/Mesen2 at its last commit (the repository is archived at
/// version 2.1.1; work continues as nesdev-org/MesenCE). docker/mesen/
/// Dockerfile builds the same commit; a test checks they agree.
pub const COMMIT: &str = "b9fa69ddc6d0a331fb103fdb5eef6904305703c2";
pub const VERSION: &str = "2.1.1";
/// The runtime name Mesen recordings carry.
pub const RUNTIME: &str = "mesen2";

/// The Docker image tag for one architecture (`arm64` or `amd64`).
pub fn image(arch: &str) -> String {
    format!("game-printer-mesen:{}-{arch}", &COMMIT[..8])
}

/// Dots to move Mesen's PPU position by at the frame boundary. Both
/// runtimes end a frame at the first instruction boundary at or after their
/// own PPU's scanline 241, dot 1, so the positions compare as they are
/// (docs/oracle.md says why this alignment and not one by CPU cycle).
pub const BOUNDARY_DOTS: i64 = 0;

pub const STATE_MAGIC: [u8; 4] = *b"GPMS";
/// Version of the Mesen field table below. 2 (2026-10-27) holds the whole
/// 64-bit cycle count as `cpu.cycles`, matching gp-nes's field table 3,
/// where 1 had its low 32 bits there and the rest in `cpu.cycles64`.
pub const STATE_VERSION: u32 = 2;

/// The `--nes.*` switches that set Mesen up as `power_on` describes, or
/// what it can't reproduce. Video and audio are always on in Mesen, so
/// `render_video` and `mix_audio` are noted, not refused ([`notes`]).
pub fn switches(power_on: &PowerOn) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    out.push(
        match power_on.region {
            Region::Ntsc => "--nes.region=Ntsc",
            Region::Pal => "--nes.region=Pal",
        }
        .to_owned(),
    );
    out.push(
        match power_on.ram_init {
            RamInit::Zeros => "--nes.ramPowerOnState=AllZeros",
            RamInit::Ones => "--nes.ramPowerOnState=AllOnes",
            // The capture script writes FCEUX's fill into internal RAM
            // before frame 0 (scripts/oracle/capture.lua, `ram_fill`).
            RamInit::Fceux => "--nes.ramPowerOnState=AllZeros",
            RamInit::Random { .. } => {
                return Err(
                    "Mesen's random power-on RAM can't be seeded, so it can't reproduce ram_init random"
                        .into(),
                );
            }
        }
        .to_owned(),
    );
    // Mesen fixes the alignment at 1 unless it randomizes it (NesCpu.cpp,
    // RandomizeCpuPpuAlignment), the same value tetanes-core uses.
    if power_on.cpu_ppu_offset != 1 {
        return Err(format!(
            "Mesen runs a CPU-PPU offset of 1 or a random one, not {}",
            power_on.cpu_ppu_offset
        ));
    }
    out.push("--nes.randomizeCpuPpuAlignment=false".into());
    out.push(format!(
        "--nes.restrictPpuAccessOnFirstFrame={}",
        power_on.ppu_warmup
    ));
    if power_on.mapper_revisions.mmc3 != Mmc3Revision::default()
        || power_on.mapper_revisions.bf909 != Bf909Revision::default()
    {
        return Err(
            "the harness doesn't set Mesen's MMC3 or BF909 revision yet; only the defaults run"
                .into(),
        );
    }
    if power_on.four_player != FourPlayer::None || power_on.zapper {
        return Err("the harness plugs in standard controllers only".into());
    }
    // Mesen's first-run wizard is what plugs in controllers; the image
    // skips it, so both ports are set here.
    out.push("--nes.port1.type=NesController".into());
    out.push("--nes.port2.type=NesController".into());
    out.push(format!(
        "--nes.allowInvalidInput={}",
        power_on.concurrent_dpad
    ));
    out.push("--nes.randomizeMapperPowerOnState=false".into());
    Ok(out)
}

/// What differs between `power_on` and how Mesen runs, without stopping it.
pub fn notes(power_on: &PowerOn) -> Vec<String> {
    let mut out = Vec::new();
    if !power_on.render_video {
        out.push("Mesen always renders video, so its sprite 0 hits come from the rendering path; the recording ran headless".into());
    }
    if !power_on.mix_audio {
        out.push("Mesen always mixes audio; that changes no console state".into());
    }
    out
}

fn f(name: &str, n: u32) -> FieldDef {
    FieldDef {
        name: name.into(),
        width: Width::Fixed(n),
    }
}

fn region(name: &str) -> FieldDef {
    FieldDef {
        name: name.into(),
        width: Width::Region,
    }
}

/// The prefix of fields Mesen counts another way than `tetanes-core`, kept
/// in the recording but under names the diff doesn't pair with a `gp-nes`
/// field: the APU's timer counters and sequencer positions. At power-on,
/// before either has run an APU cycle of the game's, they already differ
/// (pulse duty step 0 against 6, noise shift register $2000 against $4000,
/// DMC timer 419 against 426, the frame counter's cycle), so they count
/// from other origins or in other directions. Comparing them needs each
/// emulator's convention worked out first (docs/oracle.md).
pub const PHASE: &str = "mesen.";

/// The Mesen field table: canonical names where Mesen shows the same thing,
/// and [`PHASE`] names where it counts differently.
pub fn field_table() -> FieldTable {
    let mut fields = vec![
        f("cpu.a", 1),
        f("cpu.x", 1),
        f("cpu.y", 1),
        f("cpu.s", 1),
        f("cpu.p", 1),
        f("cpu.pc", 2),
        f("cpu.cycles", 8),
        f("ram", 2048),
        f("ppu.scanline", 2),
        f("ppu.dot", 2),
        f("ppu.odd_frame", 1),
        f("ppu.ctrl", 1),
        f("ppu.mask", 1),
        f("ppu.status", 1),
        f("ppu.oamaddr", 1),
        f("ppu.v", 2),
        f("ppu.t", 2),
        f("ppu.x", 1),
        f("ppu.w", 1),
        f("ppu.read_buffer", 1),
        f("oam", 256),
        f("palette", 32),
        region("nametables"),
        region("chr_ram"),
        region("prg_ram"),
    ];
    for ch in ["apu.pulse1", "apu.pulse2"] {
        fields.extend(timer(ch));
        fields.push(f(&format!("{ch}.duty"), 1));
        fields.push(f(&format!("{PHASE}{ch}.duty_step"), 1));
        fields.extend(length(ch));
        fields.extend(envelope(ch));
        for s in ["enabled", "negate", "reload", "shift", "period", "divider"] {
            fields.push(f(&format!("{ch}.sweep.{s}"), 1));
        }
    }
    fields.extend(timer("apu.triangle"));
    fields.push(f(&format!("{PHASE}apu.triangle.step"), 1));
    fields.extend(length("apu.triangle"));
    for s in ["reload", "control", "reload_value", "counter"] {
        fields.push(f(&format!("apu.triangle.linear.{s}"), 1));
    }
    fields.extend(timer("apu.noise"));
    fields.push(f("apu.noise.mode", 1));
    fields.push(f(&format!("{PHASE}apu.noise.shift"), 2));
    fields.extend(length("apu.noise"));
    fields.extend(envelope("apu.noise"));
    fields.extend(timer("apu.dmc"));
    fields.extend([
        f("apu.dmc.irq_enabled", 1),
        f("apu.dmc.loop", 1),
        f("apu.dmc.sample_addr", 2),
        f("apu.dmc.sample_length", 2),
        f("apu.dmc.addr", 2),
        f("apu.dmc.bytes_remaining", 2),
        f("apu.dmc.sample_buffer", 1),
        f("apu.dmc.buffer_empty", 1),
        f("apu.dmc.output_level", 1),
        f("apu.dmc.shift", 1),
        f(&format!("{PHASE}apu.dmc.bits_remaining"), 1),
        f("apu.dmc.silence", 1),
        f("apu.frame_counter.mode", 1),
        f("apu.frame_counter.step", 1),
        f(&format!("{PHASE}apu.frame_counter.cycle"), 4),
        f("apu.frame_counter.irq_inhibit", 1),
        f("apu.frame_counter.irq_pending", 1),
        f("apu.frame_counter.write_pending", 1),
        f("apu.frame_counter.write_value", 1),
        f("apu.frame_counter.write_delay", 1),
        f("pad1.strobe", 1),
        f("pad2.strobe", 1),
    ]);
    FieldTable {
        state_magic: STATE_MAGIC,
        state_version: STATE_VERSION,
        fields,
    }
}

fn timer(ch: &str) -> [FieldDef; 2] {
    [
        f(&format!("{ch}.timer.period"), 2),
        f(&format!("{PHASE}{ch}.timer.counter"), 2),
    ]
}

fn length(ch: &str) -> [FieldDef; 3] {
    [
        f(&format!("{ch}.length.enabled"), 1),
        f(&format!("{ch}.length.halt"), 1),
        f(&format!("{ch}.length.counter"), 1),
    ]
}

fn envelope(ch: &str) -> [FieldDef; 6] {
    [
        f(&format!("{ch}.envelope.start"), 1),
        f(&format!("{ch}.envelope.loop"), 1),
        f(&format!("{ch}.envelope.constant"), 1),
        f(&format!("{ch}.envelope.volume"), 1),
        f(&format!("{ch}.envelope.divider"), 1),
        f(&format!("{ch}.envelope.decay"), 1),
    ]
}

/// Scanlines per frame.
fn lines(region: Region) -> i64 {
    match region {
        Region::Ntsc => 262,
        Region::Pal => 312,
    }
}

/// Mesen's PPU position moved back by `dots`, in `gp-nes`'s numbering:
/// scanlines 0 to 261 (311 on PAL), where Mesen calls the pre-render line
/// -1.
pub fn position(region: Region, scanline: i64, dot: i64, dots: i64) -> (u16, u16) {
    let lines = lines(region);
    let line = if scanline < 0 { lines - 1 } else { scanline };
    let total = lines * 341;
    let at = (line * 341 + dot - dots).rem_euclid(total);
    ((at / 341) as u16, (at % 341) as u16)
}

struct Writer<'a> {
    frame: &'a Frame,
    out: Vec<u8>,
    missing: Vec<String>,
}

impl Writer<'_> {
    fn int(&mut self, key: &str) -> i64 {
        let Some(v) = self.frame.values.get(key) else {
            self.missing.push(key.to_owned());
            return 0;
        };
        match v.as_str() {
            "true" => 1,
            "false" => 0,
            v => v.parse::<i64>().unwrap_or_else(|_| {
                self.missing.push(format!("{key} (not a number: {v:?})"));
                0
            }),
        }
    }

    fn u8(&mut self, key: &str) {
        let v = self.int(key);
        self.out.push(v as u8);
    }

    fn u16(&mut self, key: &str) {
        let v = self.int(key) as u16;
        self.out.extend_from_slice(&v.to_le_bytes());
    }

    fn flag(&mut self, key: &str, bit: u8) -> u8 {
        u8::from(self.int(key) != 0) << bit
    }

    fn put(&mut self, bytes: &[u8]) {
        self.out.extend_from_slice(bytes);
    }

    fn memory(&mut self, name: &str) -> Vec<u8> {
        match self.frame.memories.get(name) {
            Some(m) => m.clone(),
            None => {
                self.missing.push(format!("memory {name}"));
                Vec::new()
            }
        }
    }

    fn region(&mut self, bytes: &[u8]) {
        self.out
            .extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        self.out.extend_from_slice(bytes);
    }

    fn timer(&mut self, mesen: &str) {
        self.u16(&format!("{mesen}.timer.period"));
        self.u16(&format!("{mesen}.timer.timer"));
    }

    fn length(&mut self, lc: &str) {
        self.u8(&format!("{lc}.enabled"));
        self.u8(&format!("{lc}.halt"));
        self.u8(&format!("{lc}.counter"));
    }

    fn envelope(&mut self, env: &str) {
        self.u8(&format!("{env}.start"));
        // The envelope loop flag and the length counter halt flag are the
        // same register bit; Mesen keeps only the length counter's.
        self.u8(&format!("{env}.lengthCounter.halt"));
        self.u8(&format!("{env}.constantVolume"));
        self.u8(&format!("{env}.volume"));
        self.u8(&format!("{env}.divider"));
        self.u8(&format!("{env}.counter"));
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error(
    "Mesen's state at frame {frame} lacks {missing}; this Mesen build names its state differently"
)]
pub struct EncodeError {
    pub frame: u64,
    pub missing: String,
}

/// One frame of a capture as a state of [`field_table`]. The conversions:
/// the PPU position is moved back [`BOUNDARY_DOTS`] dots and renumbered
/// ([`position`]); `cpu.cycles` is Mesen's 64-bit count;
/// `ppu.ctrl`, `ppu.mask` and `ppu.status` are assembled from Mesen's
/// flags (PPUCTRL's master/slave bit isn't shown, so it's 0); `prg_ram` is
/// Mesen's work RAM then its save RAM. `cpu.p` is Mesen's as is, with bit
/// 5 clear; the diff ignores bits 4 and 5 ([`crate::diff`]).
pub fn encode(frame: &Frame, region_: Region) -> Result<Vec<u8>, EncodeError> {
    let mut w = Writer {
        frame,
        out: Vec::with_capacity(8 + 2048 + 2048 + 512),
        missing: Vec::new(),
    };
    w.put(&STATE_MAGIC);
    w.put(&STATE_VERSION.to_le_bytes());
    w.u8("cpu.a");
    w.u8("cpu.x");
    w.u8("cpu.y");
    w.u8("cpu.sp");
    w.u8("cpu.ps");
    w.u16("cpu.pc");
    let cycles = w.int("cpu.cycleCount") as u64;
    w.put(&cycles.to_le_bytes());
    let ram = w.memory("ram");
    w.put(&ram);
    let scanline = w.int("ppu.scanline");
    let dot = w.int("ppu.cycle");
    let (line, dot) = position(region_, scanline, dot, BOUNDARY_DOTS);
    w.put(&line.to_le_bytes());
    w.put(&dot.to_le_bytes());
    // Mesen counts PPU frames from 1 at power-on; tetanes-core's first
    // frame is even.
    let frames = w.int("ppu.frameCount");
    w.out.push(u8::from(frames % 2 == 0));
    let ctrl = w.flag("ppu.control.verticalWrite", 2)
        | u8::from(w.int("ppu.control.spritePatternAddr") == 0x1000) << 3
        | u8::from(w.int("ppu.control.backgroundPatternAddr") == 0x1000) << 4
        | w.flag("ppu.control.largeSprites", 5)
        | w.flag("ppu.control.nmiOnVerticalBlank", 7);
    w.out.push(ctrl);
    let mask = w.flag("ppu.mask.grayscale", 0)
        | w.flag("ppu.mask.backgroundMask", 1)
        | w.flag("ppu.mask.spriteMask", 2)
        | w.flag("ppu.mask.backgroundEnabled", 3)
        | w.flag("ppu.mask.spritesEnabled", 4)
        | w.flag("ppu.mask.intensifyRed", 5)
        | w.flag("ppu.mask.intensifyGreen", 6)
        | w.flag("ppu.mask.intensifyBlue", 7);
    w.out.push(mask);
    let status = w.flag("ppu.statusFlags.spriteOverflow", 5)
        | w.flag("ppu.statusFlags.sprite0Hit", 6)
        | w.flag("ppu.statusFlags.verticalBlank", 7);
    w.out.push(status);
    w.u8("ppu.spriteRamAddr");
    w.u16("ppu.videoRamAddr");
    w.u16("ppu.tmpVideoRamAddr");
    w.u8("ppu.xScroll");
    w.u8("ppu.writeToggle");
    w.u8("ppu.memoryReadBuffer");
    let oam = w.memory("oam");
    w.put(&oam);
    let palette = w.memory("palette");
    w.put(&palette);
    for name in ["nametables", "chr_ram"] {
        let m = w.memory(name);
        w.region(&m);
    }
    let mut prg = w.memory("work_ram");
    prg.extend(w.memory("save_ram"));
    w.region(&prg);

    for ch in ["apu.square1", "apu.square2"] {
        w.timer(ch);
        w.u8(&format!("{ch}.duty"));
        w.u8(&format!("{ch}.dutyPos"));
        w.length(&format!("{ch}.envelope.lengthCounter"));
        w.envelope(&format!("{ch}.envelope"));
        w.u8(&format!("{ch}.sweepEnabled"));
        w.u8(&format!("{ch}.sweepNegate"));
        w.u8(&format!("{ch}.reloadSweep"));
        w.u8(&format!("{ch}.sweepShift"));
        w.u8(&format!("{ch}.sweepPeriod"));
        w.u8(&format!("{ch}.sweepDivider"));
    }
    w.timer("apu.triangle");
    w.u8("apu.triangle.sequencePosition");
    w.length("apu.triangle.lengthCounter");
    w.u8("apu.triangle.linearReloadFlag");
    w.u8("apu.triangle.linearControlFlag");
    w.u8("apu.triangle.linearCounterReload");
    w.u8("apu.triangle.linearCounter");
    w.timer("apu.noise");
    w.u8("apu.noise.modeFlag");
    w.u16("apu.noise.shiftRegister");
    w.length("apu.noise.envelope.lengthCounter");
    w.envelope("apu.noise.envelope");
    w.timer("apu.dmc");
    w.u8("apu.dmc.irqEnabled");
    w.u8("apu.dmc.loopFlag");
    w.u16("apu.dmc.sampleAddr");
    w.u16("apu.dmc.sampleLength");
    w.u16("apu.dmc.currentAddr");
    w.u16("apu.dmc.bytesRemaining");
    w.u8("apu.dmc.readBuffer");
    w.u8("apu.dmc.bufferEmpty");
    w.u8("apu.dmc.outputLevel");
    w.u8("apu.dmc.shiftRegister");
    w.u8("apu.dmc.bitsRemaining");
    w.u8("apu.dmc.silenceFlag");
    w.u8("apu.frameCounter.stepMode");
    w.u8("apu.frameCounter.currentStep");
    let cycle = w.int("apu.frameCounter.previousCycle") as u32;
    w.put(&cycle.to_le_bytes());
    w.u8("apu.frameCounter.inhibitIRQ");
    w.u8("apu.frameCounter.irqFlag");
    // -1 means no write is waiting.
    let pending = w.int("apu.frameCounter.newValue");
    w.out.push(u8::from(pending >= 0));
    w.out.push(pending.max(0) as u8);
    let delay = w.int("apu.frameCounter.writeDelayCounter");
    w.out.push(delay.max(0) as u8);
    // Mesen's device 0 is the console itself; 1 and 2 are the ports.
    w.u8("controlManager.controlDevices[1].strobe");
    w.u8("controlManager.controlDevices[2].strobe");

    if !w.missing.is_empty() {
        return Err(EncodeError {
            frame: frame.frame,
            missing: w.missing.join(", "),
        });
    }
    Ok(w.out)
}

/// A capture's events in the event stream's terms. Positions are Mesen's
/// PPU position when its callback ran, renumbered and otherwise as they
/// are: over the Super Mario Bros. baseline the most common offset from
/// `gp-nes`'s (the position after the CPU cycle that made the event) is 0
/// for controller reads and register writes (docs/oracle.md).
pub fn events(frame: &Frame, region_: Region) -> Result<Vec<Event>, String> {
    frame
        .events
        .iter()
        .map(|e: &RawEvent| {
            let kind = EventKind::from_name(&e.kind)
                .ok_or_else(|| format!("unknown event kind {:?}", e.kind))?;
            let (scanline, dot) = position(region_, i64::from(e.scanline), i64::from(e.dot), 0);
            Ok(Event {
                frame: frame.frame,
                kind,
                scanline,
                dot,
                addr: e.addr,
                value: e.value,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_wrap_into_gp_nes_numbering() {
        assert_eq!(position(Region::Ntsc, 241, 8, 4), (241, 4));
        assert_eq!(position(Region::Ntsc, 241, 2, 4), (240, 339));
        assert_eq!(position(Region::Ntsc, -1, 0, 0), (261, 0));
        assert_eq!(position(Region::Pal, -1, 3, 4), (310, 340));
        assert_eq!(position(Region::Ntsc, 0, 1, 4), (261, 338));
    }

    #[test]
    fn power_on_defaults_map_to_switches() {
        let s = switches(&PowerOn::default()).unwrap();
        assert!(s.contains(&"--nes.ramPowerOnState=AllZeros".to_owned()));
        assert!(s.contains(&"--nes.allowInvalidInput=true".to_owned()));
        let ones = PowerOn {
            ram_init: RamInit::Ones,
            region: Region::Pal,
            ..PowerOn::default()
        };
        let s = switches(&ones).unwrap();
        assert!(s.contains(&"--nes.ramPowerOnState=AllOnes".to_owned()));
        assert!(s.contains(&"--nes.region=Pal".to_owned()));
        let random = PowerOn {
            ram_init: RamInit::Random { seed: 1 },
            ..PowerOn::default()
        };
        assert!(switches(&random).is_err());
    }

    #[test]
    fn the_justfile_tags_the_image_alike() {
        let justfile = include_str!("../../../justfile");
        let want = format!("mesen_image := \"{}\"", image("x").trim_end_matches("-x"));
        assert!(
            justfile.contains(&want),
            "the justfile's mesen_image isn't {want}"
        );
    }

    #[test]
    fn the_dockerfile_builds_this_commit() {
        let dockerfile = include_str!("../../../docker/mesen/Dockerfile");
        assert!(
            dockerfile.contains(&format!("ARG MESEN_COMMIT={COMMIT}")),
            "docker/mesen/Dockerfile and mesen::COMMIT name different commits"
        );
    }

    /// A frame with every value `encode` asks for gives a state the field
    /// table splits exactly, so the two list the same fields in order.
    #[test]
    fn encode_follows_the_field_table() {
        let mut frame = Frame::default();
        let missing = encode(&frame, Region::Ntsc).unwrap_err().missing;
        for key in missing.split(", ") {
            match key.strip_prefix("memory ") {
                Some(name) => {
                    let size = match name {
                        "ram" => 2048,
                        "oam" => 256,
                        "palette" => 32,
                        _ => 3,
                    };
                    frame.memories.insert(name.into(), vec![7; size]);
                }
                None => {
                    frame.values.insert(key.into(), "1".into());
                }
            }
        }
        let state = encode(&frame, Region::Ntsc).unwrap();
        let table = field_table();
        let fields = table.split(&state).unwrap();
        assert_eq!(fields.len(), table.fields.len());
        let prg = fields.iter().find(|(n, _)| *n == "prg_ram").unwrap().1;
        assert_eq!(prg.len(), 6, "work RAM then save RAM");
    }

    #[test]
    fn a_frame_missing_a_value_is_refused() {
        let frame = Frame::default();
        let err = encode(&frame, Region::Ntsc).unwrap_err();
        assert!(err.missing.contains("cpu.a"), "{err}");
    }
}
