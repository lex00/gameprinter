//! The NES runtime: runs a cartridge on the vendored `tetanes-core` from a
//! fixed power-on state, one frame per input frame, and reports what a
//! recording needs to replay and check it.
//!
//! A frame ends at the start of vertical blank: the first instruction
//! boundary at which the PPU is at or past scanline 241, dot 1, the dot that
//! sets the vblank flag and raises NMI. The CPU runs whole instructions, so
//! the PPU is a few dots past that point when the frame ends, always the same
//! number for the same run. Frame 0 is the power-on state, with the PPU at
//! scanline 0, so the first frame is short: 241 scanlines instead of 262.
//! docs/formats/recording.md has the details.

pub mod accuracy;
pub mod baseline;
pub mod board;
pub mod canonical;
pub mod codemap;
pub mod coverage;
pub mod harvest;
pub mod instructions;
pub mod poke;
pub mod protocol;
pub mod ramrole;
pub mod recording;
pub mod seek;
pub mod slice;
pub mod testrom;
pub mod video;

use gp_evidence::Hash;
use gp_evidence::input_log::InputFrame;
use gp_evidence::manifest::{
    Bf909Revision, FourPlayer, Mmc3Revision, PowerOn, RamInit, Region, RuntimeRef,
};
use tetanes_core::apu::filter::FilterChain;
use tetanes_core::bus::Bus;
use tetanes_core::control_deck::{Config, ControlDeck, HeadlessMode, MapperRevisionsConfig};
use tetanes_core::input::{JoypadBtnState, Player};
use tetanes_core::mapper::{self, MapperRevision};
use tetanes_core::prelude::{NesRegion, RamState, ResetKind};

/// The version of gp-nes's own behaviour: how it powers on, steps a frame,
/// hashes the console's state and restores a checkpoint. Bump it with every
/// change to any of those, including one that keeps every test passing. It is
/// part of the runtime identity in every recording, and replay uses that
/// identity to tell a runtime change from an emulation divergence.
// 2 (2026-10-27): frames end at the start of vblank, and frame hashes cover
// a canonical state (canonical.rs) instead of tetanes-core's save state.
// 3 (2026-10-27): field table version 2, with the MMC1 and MMC3 registers.
// 4 (2026-10-28): the PPU open bus decays bit by bit as Mesen 2's does, and
// field table version 3 (64-bit cycle count, open bus ages, no PRG RAM on
// NROM boards without a battery).
// 5 (2026-10-28): field table version 4 (the open bus as held, ages 0 to 4,
// NROM's PRG RAM back in the state).
// 6 (2026-10-28): the console loads board::runnable's image: the header
// from the registry's board facts for a registered source, archaic iNES
// headers read with bytes 7 to 15 as zero, trainers and trailing bytes
// dropped, and a header that contradicts the registry refused.
// 7 (2026-11-03): field table version 5 (CNROM's CHR bank register).
pub const BEHAVIOUR_VERSION: u32 = 7;

/// Identifies this runtime in recording manifests. Two builds with the same
/// identity must replay every recording identically. The version names
/// [`BEHAVIOUR_VERSION`], the upstream `tetanes-core` release and a hash of
/// our patch to it (third_party/tetanes-core.patch); `commit` is the upstream
/// commit that release was published from. Our fork has no commit of its own
/// (it's vendored, with no remote), so the patch hash stands in for one.
/// build.rs derives the `tetanes-core` parts.
pub fn runtime_ref() -> RuntimeRef {
    RuntimeRef {
        name: "gp-nes".into(),
        version: format!(
            "{BEHAVIOUR_VERSION} (tetanes-core {} + patch {})",
            env!("GP_NES_TETANES_VERSION"),
            env!("GP_NES_PATCH_BLAKE3")
        ),
        commit: Some(env!("GP_NES_TETANES_COMMIT").into()),
    }
}

/// Samples a second of the audio [`Nes::set_output`] mixes: tetanes-core's
/// default, which gp-nes never changes.
pub const AUDIO_RATE: u32 = tetanes_core::apu::Apu::DEFAULT_SAMPLE_RATE as u32;

/// The dot on the vblank scanline that sets the vblank flag.
const VBLANK_DOT: u16 = tetanes_core::ppu::cycle::VBLANK;

#[derive(Debug, thiserror::Error)]
pub enum NesError {
    #[error("power-on configuration not supported: {0}")]
    Unsupported(String),
    #[error("tetanes-core: {0}")]
    Core(#[from] tetanes_core::control_deck::Error),
    #[error("ROM: {0}")]
    Rom(#[from] gp_evidence::dump::DumpError),
}

/// The `tetanes-core` configuration for a power-on state. Settings that only
/// affect presentation (video filter, APU channel mute) keep their defaults;
/// run-ahead, Game Genie codes and battery files stay off.
fn config(power_on: PowerOn) -> Result<Config, NesError> {
    let unsupported = |why: &str| Err(NesError::Unsupported(why.into()));
    let ram_state = match power_on.ram_init {
        RamInit::Zeros => RamState::AllZeros,
        RamInit::Ones => RamState::AllOnes,
        // Zeros everywhere, then internal RAM is filled in power_on.
        RamInit::Fceux => RamState::AllZeros,
        RamInit::Random { .. } => {
            return unsupported(
                "random RAM init can't be seeded in tetanes-core; use zeros or ones",
            );
        }
    };
    if power_on.cpu_ppu_offset != 1 {
        return unsupported("tetanes-core fixes the CPU-PPU clock offset at 1");
    }
    if power_on.four_player != FourPlayer::None {
        return unsupported("gp-nes applies controllers 1 and 2 only; four_player must be none");
    }
    if power_on.zapper {
        return unsupported("input logs carry no zapper data; zapper must be false");
    }
    if !power_on.concurrent_dpad {
        // gp-nes sets the buttons as logged; tetanes-core's rule against
        // opposing directions applies only to presses made through its own
        // input calls, so turning it off here would change nothing.
        return unsupported("gp-nes applies input as logged; concurrent_dpad must be true");
    }
    let region = match power_on.region {
        Region::Ntsc => NesRegion::Ntsc,
        Region::Pal => NesRegion::Pal,
    };
    let mut revisions = MapperRevisionsConfig::default();
    revisions.set(MapperRevision::Mmc3(match power_on.mapper_revisions.mmc3 {
        Mmc3Revision::A => mapper::Mmc3Revision::A,
        Mmc3Revision::BC => mapper::Mmc3Revision::BC,
        Mmc3Revision::Acclaim => mapper::Mmc3Revision::Acc,
    }));
    revisions.set(MapperRevision::Bf909(
        match power_on.mapper_revisions.bf909 {
            Bf909Revision::Bf909x => mapper::Bf909Revision::Bf909x,
            Bf909Revision::Bf9097 => mapper::Bf909Revision::Bf9097,
        },
    ));
    let mut headless = HeadlessMode::empty();
    if !power_on.render_video {
        headless |= HeadlessMode::NO_VIDEO;
    }
    if !power_on.mix_audio {
        headless |= HeadlessMode::NO_AUDIO;
    }
    Ok(Config::default()
        .with_region(region)
        .with_ram_state(ram_state)
        .with_emulate_ppu_warmup(power_on.ppu_warmup)
        .with_mapper_revisions(revisions)
        .with_concurrent_dpad(power_on.concurrent_dpad)
        .with_headless_mode(headless)
        .with_sram_dir(None))
}

/// What one frame produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameOutcome {
    /// Whether the game read either controller during the frame.
    pub polled: bool,
}

/// One running console. A clone is an independent copy of it, which is how
/// [`protocol::NesRuntime`] forks.
#[derive(Clone)]
pub struct Nes {
    deck: ControlDeck,
    frame: u64,
    /// Whether the event log is on; kept here because a restore replaces the
    /// PPU, and the log with it.
    events: bool,
}

impl Nes {
    /// Powers on with the ROM loaded, configured from `power_on` alone, so a
    /// replay powers on exactly as its recording did. Refuses what gp-nes
    /// can't reproduce (see [`config`]). The console loads
    /// [`board::runnable`]'s image of `rom`, so junk around the game data
    /// changes nothing, and a registered source runs with its registry
    /// header.
    pub fn power_on(rom: &[u8], power_on: PowerOn) -> Result<Self, NesError> {
        let config = config(power_on)?;
        let rom = board::runnable(rom)?;
        let mut deck = ControlDeck::with_config(config);
        deck.load_rom("game", &mut &rom[..])?;
        if power_on.ram_init == RamInit::Fceux {
            for (addr, byte) in deck.bus_mut().wram.iter_mut().take(0x800).enumerate() {
                *byte = RamInit::fceux_byte(addr);
            }
        }
        Ok(Self {
            deck,
            frame: 0,
            events: false,
        })
    }

    /// Turns video rendering on or off; it's off after power-on. With it on,
    /// the PPU draws every pixel and finds sprite-0 hits while drawing; off,
    /// it finds them on a separate headless path. Neither changes the
    /// canonical state unless the two paths disagree, which is what turning
    /// it on checks. Audio stays off.
    pub fn set_video(&mut self, on: bool) {
        self.set_output(on, false);
    }

    /// Turns video rendering and audio mixing on or off together. Mixing,
    /// like drawing, changes no canonical state: the APU's channels are
    /// clocked the same either way, and what mixing adds (the mixed level,
    /// the filters' history, the samples) is left out of the state hash and
    /// of [`Nes::snapshot`]. On, each frame's samples are added to
    /// [`Nes::audio_samples`], mono at [`AUDIO_RATE`], until
    /// [`Nes::clear_audio`].
    pub fn set_output(&mut self, video: bool, audio: bool) {
        let mut mode = HeadlessMode::empty();
        if !video {
            mode |= HeadlessMode::NO_VIDEO;
        }
        if !audio {
            mode |= HeadlessMode::NO_AUDIO;
        }
        self.deck.set_headless_mode(mode);
    }

    /// The samples mixed since the last [`Nes::clear_audio`]: mono, at
    /// [`AUDIO_RATE`], about 800 a frame. Empty with mixing off.
    pub fn audio_samples(&self) -> &[f32] {
        self.deck.bus().audio_samples()
    }

    pub fn clear_audio(&mut self) {
        self.deck.bus_mut().clear_audio_samples();
    }

    /// Frames run since power-on.
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// Runs one frame with the given controller state.
    pub fn step(&mut self, input: InputFrame) -> Result<FrameOutcome, NesError> {
        self.step_with(input, |_| {})
    }

    /// Runs one frame like [`Nes::step`], calling `before` with the console
    /// before each instruction runs. It may read anything; the only thing it
    /// may change is `Bus::interrupt_taken`, which nothing else reads. With a
    /// closure that does nothing, this compiles to the same loop as `step`.
    pub fn step_with(
        &mut self,
        input: InputFrame,
        mut before: impl FnMut(&mut Bus),
    ) -> Result<FrameOutcome, NesError> {
        // A reset in the input log presses the reset button before the
        // frame runs, as FCEUX does for a movie frame's reset command.
        if input.reset {
            self.deck.reset(ResetKind::Soft);
        }
        self.set_buttons(input.pads);
        // Run until an instruction carries the PPU from before this frame's
        // vblank start to at or after it. A step begins just past the
        // previous one, so it first runs through the rest of vblank and
        // the next frame's visible scanlines.
        let mut was_before = self.before_vblank_start();
        loop {
            before(self.deck.bus_mut());
            self.deck.clock_instr()?;
            let now = self.before_vblank_start();
            if was_before && !now {
                break;
            }
            was_before = now;
        }
        // What `clock_frame` does at the end of each frame: brings the APU up
        // to date and rebases the master clocks, which would otherwise
        // overflow after a few minutes.
        self.deck.bus_mut().clock_sync();
        self.frame += 1;
        let polled =
            self.deck.joypad(Player::One).reads > 0 || self.deck.joypad(Player::Two).reads > 0;
        Ok(FrameOutcome { polled })
    }

    /// Whether the PPU is still before this frame's vblank start (scanline
    /// 241, dot 1). Scanlines run 0 to 261 (311 on PAL) and wrap to 0, so
    /// this turns true at the end of each frame's pre-render line.
    fn before_vblank_start(&self) -> bool {
        let ppu = &self.deck.bus().ppu;
        ppu.scanline < ppu.vblank_scanline
            || (ppu.scanline == ppu.vblank_scanline && ppu.cycle < VBLANK_DOT)
    }

    /// The console's full serialized state, as tetanes-core saves it. For
    /// checkpoints: only the same runtime can restore it.
    ///
    /// With audio mixing on ([`Nes::set_output`]) the snapshot is taken of
    /// a copy with mixing's own fields as a console that never mixed has
    /// them (the flag off, the mixed level 0, the filters fresh), so the
    /// bytes are the same with sound or without.
    pub fn snapshot(&self) -> Result<Vec<u8>, NesError> {
        if self.deck.bus().apu.skip_mixing {
            return Self::serialize(&self.deck);
        }
        let mut quiet = self.deck.clone();
        let apu = &mut quiet.bus_mut().apu;
        apu.skip_mixing = true;
        apu.mixed_level = 0.0;
        apu.filter_chain = FilterChain::new(apu.region, apu.sample_rate / apu.speed);
        apu.audio_samples = Vec::new();
        Self::serialize(&quiet)
    }

    fn serialize(deck: &ControlDeck) -> Result<Vec<u8>, NesError> {
        let mut buf = vec![0u8; deck.serialized_state_len()?];
        let n = deck.serialize_state_into(&mut buf)?;
        buf.truncate(n);
        Ok(buf)
    }

    /// Restores a snapshot taken at `frame`. `held` is the controller state
    /// that frame was run with (the input log's entry before `frame`):
    /// tetanes-core deliberately releases every button when it loads a state,
    /// so players don't resume with buttons stuck down, and putting them back
    /// makes the restored state identical to the one snapshotted.
    ///
    /// With audio mixing on, the mixed level the synthesiser last heard is
    /// kept, so the first sound after a restore is a step from what was
    /// playing rather than from silence (a click).
    pub fn restore(&mut self, snapshot: &[u8], frame: u64, held: [u8; 4]) -> Result<(), NesError> {
        let heard = self.deck.bus().apu.mixed_level;
        self.deck.deserialize_state(snapshot)?;
        let apu = &mut self.deck.bus_mut().apu;
        if !apu.skip_mixing {
            apu.mixed_level = heard;
        }
        self.set_buttons(held);
        self.frame = frame;
        self.set_events(self.events);
        Ok(())
    }

    /// The controller state the last frame ran with, ports 1 and 2 (3 and
    /// 4 are always 0): what [`Nes::restore`] needs to put back.
    pub fn held(&self) -> [u8; 4] {
        let pad = |player| self.deck.joypad(player).buttons.bits() as u8;
        [pad(Player::One), pad(Player::Two), 0, 0]
    }

    /// The console, for reading state that has no accessor of its own.
    pub fn bus(&self) -> &Bus {
        self.deck.bus()
    }

    /// The console, for observers between frames, such as coverage
    /// (`gp-coverage`). Like the closure [`Nes::step_with`] calls, it may
    /// read anything and change only `Bus::interrupt_taken`, which nothing
    /// else reads.
    pub fn bus_mut_for_observers(&mut self) -> &mut Bus {
        self.deck.bus_mut()
    }

    /// Turns the event log on or off; it's off after power-on. On, the
    /// console logs interrupts, sprite 0 hits, bank switches, controller
    /// reads and register writes as they happen, and [`Nes::take_events`]
    /// collects them. It changes nothing about what the console does.
    pub fn set_events(&mut self, on: bool) {
        self.events = on;
        let log = &mut self.deck.bus_mut().ppu.events;
        match (on, log.is_some()) {
            (true, false) => *log = Some(Box::default()),
            (false, true) => *log = None,
            _ => (),
        }
    }

    /// The events logged since the last call, as the frame they happened in
    /// (the frame now running, [`Nes::frame`] after the step that ran it).
    /// Empty when the log is off.
    pub fn take_events(&mut self) -> Vec<gp_evidence::events::Event> {
        use gp_evidence::events::{Event, EventKind as Kind};
        use tetanes_core::events::EventKind;
        let frame = self.frame;
        let Some(log) = self.deck.bus_mut().ppu.events.as_mut() else {
            return Vec::new();
        };
        log.drain(..)
            .map(|e| Event {
                frame,
                kind: match e.kind {
                    EventKind::Nmi => Kind::Nmi,
                    EventKind::Irq => Kind::Irq,
                    EventKind::Sprite0Hit => Kind::Sprite0Hit,
                    EventKind::BankSwitch => Kind::BankSwitch,
                    EventKind::ControllerRead => Kind::ControllerRead,
                    EventKind::ControllerWrite => Kind::ControllerWrite,
                    EventKind::PpuWrite => Kind::PpuWrite,
                    EventKind::ApuWrite => Kind::ApuWrite,
                },
                scanline: e.scanline,
                dot: e.dot,
                addr: e.addr,
                value: e.value,
            })
            .collect()
    }

    fn set_buttons(&mut self, pads: [u8; 4]) {
        for (player, pad) in [(Player::One, pads[0]), (Player::Two, pads[1])] {
            let joypad = self.deck.joypad_mut(player);
            joypad.buttons = JoypadBtnState::from_bits_truncate(u16::from(pad));
            joypad.reads = 0;
        }
    }

    /// The canonical console state (see [`canonical`]): CPU, RAM, PPU,
    /// mapper, APU and controller state, without runtime settings or held
    /// buttons.
    pub fn canonical_state(&self) -> Result<Vec<u8>, NesError> {
        canonical::encode(self.deck.bus())
    }

    /// Hash of the canonical state, the per-frame `state` hash.
    pub fn state_hash(&self) -> Result<Hash, NesError> {
        Ok(Hash::of(&self.canonical_state()?))
    }

    /// [`Nes::state_hash`] where the canonical state's layout defines the
    /// board ([`canonical::MAPPERS`]), else the hash of the same fields with
    /// the board's registers left out ([`canonical::encode_any`]). Two runs
    /// compare by it on any board gp-nes runs (the page's player against a
    /// native run, say); only the first is a recording's `state` hash.
    pub fn state_or_board_free_hash(&self) -> Result<Hash, NesError> {
        Ok(Hash::of(&canonical::encode_any(self.deck.bus())))
    }

    /// The 2KB of internal RAM, where game logic keeps its state.
    pub fn wram(&self) -> &[u8] {
        self.deck.wram()
    }

    /// The name table RAM (CIRAM), as the canonical state's `nametables`
    /// holds it.
    pub fn nametables(&self) -> &[u8] {
        self.deck
            .bus()
            .memory
            .region_ref(tetanes_core::memory::Src::CiRam)
    }

    /// The 2KB of internal RAM as the CPU takes its next NMI, read from a
    /// copy of the console run on an instruction at a time, so this one is
    /// left where it is. A game that runs its frame in the NMI handler
    /// can take longer than a frame: at the frame's end the handler is
    /// still running, and the RAM there is a frame half done (Donkey
    /// Kong's timer routine $F4AC, last in the handler, hasn't run yet on
    /// a ladder grab with seven barrels out). The next NMI is taken only
    /// once the handler lets it, so the RAM then is the whole frame's.
    /// `None` when the CPU is at the NMI's entry already (the RAM is that
    /// one) or takes none within a frame's cycles (the NMI is off).
    pub fn wram_at_next_nmi(&self) -> Option<Vec<u8>> {
        self.at_next_nmi().map(|n| n.wram().to_vec())
    }

    /// A copy of the console as the CPU takes its next NMI: its entry, the
    /// return address and status pushed, PC at the vector. As
    /// [`Self::wram_at_next_nmi`], `None` when the CPU is at the NMI's
    /// entry already or takes none within a frame's cycles.
    pub fn at_next_nmi(&self) -> Option<Nes> {
        // A frame's CPU cycles on NTSC, 89342 PPU dots over 3, and a margin.
        const MOST: u64 = 29_781 + 1_000;
        let bus = self.deck.bus();
        let vector = u16::from(bus.peek(0xFFFA)) | u16::from(bus.peek(0xFFFB)) << 8;
        if bus.cpu.pc == vector {
            return None;
        }
        let mut ahead = self.clone();
        ahead.set_events(false);
        ahead.deck.bus_mut().interrupt_taken = None;
        let start = ahead.deck.bus().cpu.cycle;
        loop {
            ahead.deck.clock_instr().ok()?;
            let bus = ahead.deck.bus();
            if bus.interrupt_taken == Some(tetanes_core::debug::FrameKind::Nmi) {
                return Some(ahead);
            }
            if bus.cpu.cycle.saturating_sub(start) > MOST {
                return None;
            }
        }
    }

    /// Hash of internal RAM only.
    pub fn wram_hash(&self) -> Hash {
        Hash::of(self.deck.wram())
    }

    /// Reads a byte as the CPU would at `addr`, without the side effects a
    /// read can have (clearing the vblank flag, advancing a controller's
    /// shift register). Reaches what [`Nes::wram`] doesn't, such as PRG RAM
    /// at $6000-$7FFF, where test ROMs report their results.
    pub fn peek(&self, addr: u16) -> u8 {
        self.deck.bus().peek(addr)
    }

    /// Presses the reset button now. Test ROMs that ask for it use this
    /// ([`accuracy`]); recordings say it in the input log instead
    /// (`InputFrame::reset`), which [`Nes::step`] applies.
    pub fn reset(&mut self) {
        self.deck.reset(ResetKind::Soft);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gp_evidence::input_log::button;

    fn input(pad: u8) -> InputFrame {
        InputFrame {
            polled: true,
            pads: [pad, 0, 0, 0],
            reset: false,
        }
    }

    /// Runs the test ROM, pressing A on frames divisible by 7, and returns
    /// each frame's (state hash, polled).
    fn run(frames: u64) -> Vec<(Hash, bool)> {
        let mut nes = Nes::power_on(&testrom::build(), PowerOn::default()).unwrap();
        (0..frames)
            .map(|f| {
                let pad = if f % 7 == 0 { button::A } else { 0 };
                let out = nes.step(input(pad)).unwrap();
                (nes.state_hash().unwrap(), out.polled)
            })
            .collect()
    }

    #[test]
    fn the_fceux_fill_sets_internal_ram_only() {
        let rom = testrom::build();
        let zeros = Nes::power_on(&rom, PowerOn::default()).unwrap();
        let fceux = Nes::power_on(
            &rom,
            PowerOn {
                ram_init: RamInit::Fceux,
                ..PowerOn::default()
            },
        )
        .unwrap();
        let ram = fceux.wram();
        assert_eq!(
            &ram[..16],
            &[
                0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff
            ]
        );
        assert_eq!(ram[0x7ff], 0xff);
        assert!(zeros.wram()[..0x800].iter().all(|&b| b == 0));
        // Everything but internal RAM powers on as with zeros.
        let (a, b) = (
            zeros.canonical_state().unwrap(),
            fceux.canonical_state().unwrap(),
        );
        assert_eq!(a.len(), b.len());
        let differing = a.iter().zip(&b).filter(|(x, y)| x != y).count();
        assert_eq!(differing, 0x400, "half of the 2KB is $FF");
    }

    #[test]
    fn replays_identically() {
        assert_eq!(run(300), run(300));
    }

    #[test]
    fn polls_on_alternate_frames() {
        let frames = run(120);
        let polled: Vec<bool> = frames.iter().skip(20).map(|(_, p)| *p).collect();
        assert!(polled.iter().any(|p| *p) && polled.iter().any(|p| !*p));
        for pair in polled.windows(2) {
            assert_ne!(pair[0], pair[1], "the test ROM polls every other frame");
        }
    }

    #[test]
    fn input_reaches_ram() {
        let mut held = Nes::power_on(&testrom::build(), PowerOn::default()).unwrap();
        let mut idle = Nes::power_on(&testrom::build(), PowerOn::default()).unwrap();
        for _ in 0..60 {
            held.step(input(button::A | button::RIGHT)).unwrap();
            idle.step(input(0)).unwrap();
        }
        assert_eq!(idle.wram()[testrom::addr::BUTTONS], 0);
        // The program shifts buttons in A-first, so A lands in bit 7 and Right in bit 0.
        assert_eq!(held.wram()[testrom::addr::BUTTONS], 0b1000_0001);
        assert_ne!(held.wram_hash(), idle.wram_hash());
        assert!(held.wram()[testrom::addr::FRAMES] > 50);
    }

    #[test]
    fn each_button_reaches_its_own_bit() {
        // The NES returns A first and Right last; the test program shifts
        // them in, so A ends in bit 7. Pins the whole mapping, not just a
        // pattern that reads the same reversed.
        let order = [
            button::A,
            button::B,
            button::SELECT,
            button::START,
            button::UP,
            button::DOWN,
            button::LEFT,
            button::RIGHT,
        ];
        for (i, b) in order.into_iter().enumerate() {
            let mut nes = Nes::power_on(&testrom::build(), PowerOn::default()).unwrap();
            for _ in 0..10 {
                nes.step(input(b)).unwrap();
            }
            assert_eq!(
                nes.wram()[testrom::addr::BUTTONS],
                0x80 >> i,
                "button {b:#04x}"
            );
        }
    }

    #[test]
    fn restoring_a_snapshot_resumes_identically() {
        let mut nes = Nes::power_on(&testrom::build(), PowerOn::default()).unwrap();
        for _ in 0..100 {
            nes.step(input(0)).unwrap();
        }
        let snap = nes.snapshot().unwrap();
        let tail = |nes: &mut Nes| -> Vec<Hash> {
            (0..100)
                .map(|f| {
                    nes.step(input(if f % 3 == 0 { button::B } else { 0 }))
                        .unwrap();
                    nes.state_hash().unwrap()
                })
                .collect()
        };
        let first = tail(&mut nes);
        nes.restore(&snap, 100, [0; 4]).unwrap();
        assert_eq!(nes.frame(), 100);
        assert_eq!(tail(&mut nes), first);
    }

    #[test]
    fn restore_reproduces_held_buttons() {
        let mut nes = Nes::power_on(&testrom::build(), PowerOn::default()).unwrap();
        for _ in 0..50 {
            nes.step(input(button::RIGHT | button::B)).unwrap();
        }
        let snap = nes.snapshot().unwrap();
        let mut fresh = Nes::power_on(&testrom::build(), PowerOn::default()).unwrap();
        fresh
            .restore(&snap, 50, [button::RIGHT | button::B, 0, 0, 0])
            .unwrap();
        assert_eq!(
            fresh.snapshot().unwrap(),
            snap,
            "restored state must be byte-identical"
        );
    }

    /// A field of the canonical state, as a little-endian integer.
    fn field(nes: &Nes, name: &str) -> u64 {
        let state = nes.canonical_state().unwrap();
        let fields = canonical::decode(&state).unwrap();
        let (_, bytes) = fields.iter().find(|(n, _)| *n == name).unwrap();
        bytes
            .iter()
            .rev()
            .fold(0, |acc, b| (acc << 8) | u64::from(*b))
    }

    #[test]
    fn frames_end_at_vblank_start() {
        let mut nes = Nes::power_on(&testrom::build(), PowerOn::default()).unwrap();
        assert_eq!(field(&nes, "ppu.scanline"), 0, "power-on is at scanline 0");
        let mut cycles = field(&nes, "cpu.cycles");
        for frame in 1..=20 {
            nes.step(input(0)).unwrap();
            assert_eq!(field(&nes, "ppu.scanline"), 241, "frame {frame}");
            // The instruction that crosses dot 1 finishes a few dots later,
            // or a few more if an NMI sequence follows it.
            let dot = field(&nes, "ppu.dot");
            assert!((1..=45).contains(&dot), "frame {frame} ends at dot {dot}");
            let now = field(&nes, "cpu.cycles");
            let ran = now - cycles;
            cycles = now;
            if frame == 1 {
                // From power-on at scanline 0: 241 of the 262 scanlines.
                assert!(
                    (27_350..27_420).contains(&ran),
                    "first frame ran {ran} cycles"
                );
            } else {
                // 262 scanlines of 341 dots, 3 dots a cycle: 29,780.67.
                assert!(
                    (29_765..29_795).contains(&ran),
                    "frame {frame} ran {ran} cycles"
                );
            }
            if frame > 4 {
                // Past the program's warmup nothing reads $2002, so the
                // flag set at dot 1 is still up.
                assert_eq!(field(&nes, "ppu.status") & 0x80, 0x80, "frame {frame}");
            }
        }
    }

    #[test]
    fn held_buttons_are_input_not_state() {
        let mut nes = Nes::power_on(&testrom::build(), PowerOn::default()).unwrap();
        for _ in 0..30 {
            nes.step(input(button::A)).unwrap();
        }
        let (state, snapshot) = (nes.state_hash().unwrap(), nes.snapshot().unwrap());
        nes.set_buttons([button::START, button::LEFT, 0, 0]);
        assert_eq!(nes.state_hash().unwrap(), state);
        assert_ne!(nes.snapshot().unwrap(), snapshot, "the save state has them");
    }

    #[test]
    fn runtime_settings_are_not_state() {
        let mut nes = Nes::power_on(&testrom::build(), PowerOn::default()).unwrap();
        let state = nes.state_hash().unwrap();
        nes.set_video(true);
        assert_eq!(nes.state_hash().unwrap(), state);
    }

    #[test]
    fn canonical_state_decodes_into_its_fields() {
        let mut nes = Nes::power_on(&testrom::build(), PowerOn::default()).unwrap();
        for _ in 0..10 {
            nes.step(input(0)).unwrap();
        }
        let state = nes.canonical_state().unwrap();
        let fields = canonical::decode(&state).unwrap();
        let names: Vec<_> = fields.iter().map(|(n, _)| *n).collect();
        let expected: Vec<_> = canonical::FIELDS.iter().map(|f| f.name).collect();
        assert_eq!(names, expected);
        let get = |name| fields.iter().find(|(n, _)| *n == name).unwrap().1;
        assert_eq!(get("ram"), nes.wram());
        assert_eq!(get("nametables").len(), 2048);
        assert_eq!(get("chr_ram").len(), 8192, "the test ROM has CHR RAM");
        assert_eq!(get("oam").len(), 256);
        assert_eq!(
            canonical::decode(&state[..state.len() - 1]),
            Err(canonical::DecodeError::Truncated("pad2.strobe"))
        );
        let mut longer = state.clone();
        longer.push(0);
        assert_eq!(
            canonical::decode(&longer),
            Err(canonical::DecodeError::Trailing(1))
        );
        let mut other = state;
        other[4] = 9;
        assert_eq!(
            canonical::decode(&other),
            Err(canonical::DecodeError::Version(9))
        );
    }

    #[test]
    fn refuses_what_it_cant_reproduce() {
        let refused = [
            PowerOn {
                ram_init: RamInit::Random { seed: 1 },
                ..PowerOn::default()
            },
            PowerOn {
                cpu_ppu_offset: 2,
                ..PowerOn::default()
            },
            PowerOn {
                four_player: FourPlayer::FourScore,
                ..PowerOn::default()
            },
            PowerOn {
                zapper: true,
                ..PowerOn::default()
            },
            PowerOn {
                concurrent_dpad: false,
                ..PowerOn::default()
            },
        ];
        for p in refused {
            assert!(
                matches!(
                    Nes::power_on(&testrom::build(), p),
                    Err(NesError::Unsupported(_))
                ),
                "{p:?}"
            );
        }
    }

    #[test]
    fn defaults_are_the_schema_0_configuration() {
        // Schema 0 recordings were made with exactly this, and load with
        // `PowerOn::default()` (gp_evidence::manifest).
        let c = config(PowerOn::default()).unwrap();
        assert_eq!(c.region, NesRegion::Ntsc);
        assert_eq!(c.ram_state, RamState::AllZeros);
        assert!(c.concurrent_dpad && !c.emulate_ppu_warmup);
        assert_eq!(
            c.headless_mode,
            HeadlessMode::NO_AUDIO | HeadlessMode::NO_VIDEO
        );
        assert_eq!(c.mapper_revisions, MapperRevisionsConfig::default());
        assert_eq!(c.run_ahead, 0);
        assert!(c.genie_codes.is_empty() && !c.zapper && c.sram_dir.is_none());
    }

    #[test]
    fn honours_each_power_on_field() {
        let rom = testrom::build();
        let on = |p: PowerOn| Nes::power_on(&rom, p).unwrap();
        let ones = on(PowerOn {
            ram_init: RamInit::Ones,
            ..PowerOn::default()
        });
        // The test program clears only its own variables, so the rest of RAM
        // shows how it was initialised.
        assert_eq!(ones.wram()[0x700], 0xff);
        assert_eq!(on(PowerOn::default()).wram()[0x700], 0);

        let pal = on(PowerOn {
            region: Region::Pal,
            ..PowerOn::default()
        });
        assert_eq!(pal.deck.region(), NesRegion::Pal);

        let warm = on(PowerOn {
            ppu_warmup: true,
            ..PowerOn::default()
        });
        assert!(warm.deck.bus().ppu.emulate_warmup);
        assert!(!on(PowerOn::default()).deck.bus().ppu.emulate_warmup);

        let av = on(PowerOn {
            render_video: true,
            mix_audio: true,
            ..PowerOn::default()
        });
        assert!(!av.deck.bus().ppu.skip_rendering && !av.deck.bus().apu.skip_mixing);
        let headless = on(PowerOn::default());
        assert!(headless.deck.bus().ppu.skip_rendering && headless.deck.bus().apu.skip_mixing);

        // Mapper revisions only reach MMC3 and BF909 carts, which the test
        // program isn't, so check the configuration they produce.
        let c = config(PowerOn {
            mapper_revisions: gp_evidence::manifest::MapperRevisions {
                mmc3: Mmc3Revision::Acclaim,
                bf909: Bf909Revision::Bf9097,
            },
            ..PowerOn::default()
        })
        .unwrap();
        assert_eq!(c.mapper_revisions.mmc3, mapper::Mmc3Revision::Acc);
        assert_eq!(c.mapper_revisions.bf909, mapper::Bf909Revision::Bf9097);
    }

    #[test]
    fn the_event_log_changes_nothing_and_logs_what_happened() {
        use gp_evidence::events::EventKind;
        let mut plain = Nes::power_on(&testrom::build(), PowerOn::default()).unwrap();
        let mut logged = Nes::power_on(&testrom::build(), PowerOn::default()).unwrap();
        logged.set_events(true);
        let mut by_frame = Vec::new();
        for f in 0..40 {
            let pad = if f % 3 == 0 { button::A } else { 0 };
            let a = plain.step(input(pad)).unwrap();
            let b = logged.step(input(pad)).unwrap();
            assert_eq!(a, b);
            assert_eq!(
                plain.snapshot().unwrap(),
                logged.snapshot().unwrap(),
                "frame {f}"
            );
            assert!(plain.take_events().is_empty());
            by_frame.push((b.polled, logged.take_events()));
        }
        let all: Vec<_> = by_frame.iter().flat_map(|(_, e)| e).collect();
        assert!(all.iter().all(|e| e.frame >= 1 && e.frame <= 40));
        // The program enables NMI once with $2000 = $80.
        let enable = all
            .iter()
            .filter(|e| e.kind == EventKind::PpuWrite && e.addr == 0x2000)
            .collect::<Vec<_>>();
        assert_eq!(enable.len(), 1);
        assert_eq!(enable[0].value, 0x80);
        for (polled, events) in &by_frame[5..] {
            let count = |k: EventKind| events.iter().filter(|e| e.kind == k).count();
            // The NMI raised at the vblank that ends a frame is taken either
            // in the step that ends it or at the start of the next, so a
            // frame holds none, one or two (counted over all frames below).
            assert!(count(EventKind::Nmi) <= 2, "{events:?}");
            for nmi in events.iter().filter(|e| e.kind == EventKind::Nmi) {
                assert_eq!(nmi.addr, 0xC017, "the handler's address");
                assert_eq!((nmi.scanline, nmi.dot < 60), (241, true));
            }
            // A polled frame strobes once and reads each port eight times.
            let (writes, reads) = if *polled { (2, 16) } else { (0, 0) };
            assert_eq!(count(EventKind::ControllerWrite), writes);
            assert_eq!(count(EventKind::ControllerRead), reads);
            assert_eq!(
                count(EventKind::BankSwitch),
                0,
                "NROM has no banks to switch"
            );
        }
        let nmis = by_frame[5..]
            .iter()
            .flat_map(|(_, e)| e)
            .filter(|e| e.kind == EventKind::Nmi)
            .count();
        assert!((34..=36).contains(&nmis), "{nmis} NMIs in 35 frames");
        // Ports 1 and 2 are read in turn; A (held on frame 6, the input of
        // polled frame 7) comes first, in bit 0.
        let reads: Vec<_> = by_frame[6]
            .1
            .iter()
            .filter(|e| e.kind == EventKind::ControllerRead)
            .map(|e| (e.addr, e.value & 1))
            .collect();
        let mut expected = vec![(0x4016, 1), (0x4017, 0)];
        expected.extend([(0x4016, 0), (0x4017, 0)].repeat(7));
        assert_eq!(reads, expected);
    }

    #[test]
    fn the_event_log_survives_a_restore() {
        let mut nes = Nes::power_on(&testrom::build(), PowerOn::default()).unwrap();
        nes.set_events(true);
        for _ in 0..10 {
            nes.step(input(0)).unwrap();
        }
        let snap = nes.snapshot().unwrap();
        nes.take_events();
        nes.step(input(0)).unwrap();
        let first = nes.take_events();
        nes.restore(&snap, 10, [0; 4]).unwrap();
        nes.step(input(0)).unwrap();
        assert!(!first.is_empty());
        assert_eq!(nes.take_events(), first);
        nes.set_events(false);
        nes.step(input(0)).unwrap();
        assert!(nes.take_events().is_empty());
    }

    /// A 32KB UxROM (mapper 2) program that selects bank 1 at $8000 on
    /// reset, then idles.
    fn uxrom_bank_switch() -> Vec<u8> {
        let mut rom = b"NES\x1a\x02\x00\x20\x00".to_vec();
        rom.resize(16, 0);
        let mut prg = vec![0xFFu8; 32 * 1024];
        #[rustfmt::skip]
        let code = [
            0xA9, 0x01,       // $C000 LDA #1
            0x8D, 0x00, 0x80, //       STA $8000   select bank 1
            0x4C, 0x05, 0xC0, // $C005 JMP $C005
        ];
        prg[0x4000..0x4000 + code.len()].copy_from_slice(&code);
        // Reset vector -> $C000; NMI and IRQ point at the idle loop.
        prg[0x7FFA..].copy_from_slice(&[0x05, 0xC0, 0x00, 0xC0, 0x05, 0xC0]);
        rom.extend(prg);
        rom
    }

    #[test]
    fn logs_bank_switches() {
        use gp_evidence::events::EventKind;
        let mut nes = Nes::power_on(&uxrom_bank_switch(), PowerOn::default()).unwrap();
        nes.set_events(true);
        nes.step(input(0)).unwrap();
        let switches: Vec<_> = nes
            .take_events()
            .into_iter()
            .filter(|e| e.kind == EventKind::BankSwitch)
            .map(|e| (e.addr, e.value))
            .collect();
        assert_eq!(switches, [(0x8000, 0x01)]);
    }

    #[test]
    fn runtime_identity_names_the_vendored_core_and_patch() {
        let third_party =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../third_party");
        let patch = std::fs::read(third_party.join("tetanes-core.patch")).unwrap();
        let upstream =
            std::fs::read_to_string(third_party.join("tetanes-core.upstream.sha256")).unwrap();
        let runtime = runtime_ref();
        let hash = Hash::of(&patch).to_hex();
        assert!(
            runtime
                .version
                .ends_with(&format!("+ patch {})", &hash[..12])),
            "{} doesn't name the current patch",
            runtime.version
        );
        assert!(
            runtime
                .version
                .starts_with(&format!("{BEHAVIOUR_VERSION} (tetanes-core "))
        );
        let commit = runtime.commit.unwrap();
        assert_eq!(commit.len(), 40);
        assert!(upstream.lines().next().unwrap().ends_with(&commit));
    }
}
