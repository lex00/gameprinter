//! The collector: the levels a run keeps.
//!
//! It watches a run one of two ways. Through the cartridge shim
//! (`tetanes_core::shim`, docs/research/rom-event-model.md, section 5), the
//! fork marks each instruction's PRG bytes and each CPU write inline, and
//! calls the collector only before a transfer of control, after an
//! interrupt or a reset, and at the instruction a transfer went to: L0, L1
//! and L2's watched writes need nothing more. As a host observer it looks
//! at every instruction before it runs, which the code-activity and
//! RAM-role observers need. [`Collector::step`] runs a frame the first way
//! when it can and the second when they are on; the levels are the same
//! either way.

use gp_evidence::input_log::InputFrame;
use gp_nes::{FrameOutcome, Nes, NesError};
use tetanes_core::bus::Bus;
use tetanes_core::debug::FrameKind;
use tetanes_core::shim::Shim;

use crate::activity::{Activity, FrameActivity};
use crate::edges::Edges;
use crate::footprint::Footprint;
use crate::layout::Layout;

/// L0 and, when asked for, L1, watched together.
#[derive(Debug, Clone)]
pub struct Collector {
    pub layout: Layout,
    pub footprint: Footprint,
    pub edges: Option<Edges>,
    /// Code activity (who wrote what, routines entered), for `gp gaps`.
    pub activity: Option<Activity>,
    /// The RAM-role classifier's trace (`gp_nes::ramrole`).
    pub ramrole: Option<gp_nes::ramrole::Tracer>,
    /// Hooked instructions run since they were last taken (through the
    /// shim's hook bitmap).
    pub hooked: Vec<crate::frames::Hooked>,
    /// A soft reset happens before the next instruction.
    reset: bool,
}

impl Collector {
    /// A collector of L0, and of L1 when `edges`.
    pub fn new(layout: Layout, edges: bool) -> Self {
        Self {
            layout,
            footprint: Footprint::new(layout),
            edges: edges.then(Edges::new),
            activity: None,
            ramrole: None,
            hooked: Vec::new(),
            reset: false,
        }
    }

    /// A collector of L0 with `activity` watching too (and L1 when
    /// `edges`).
    pub fn with_activity(layout: Layout, edges: bool, activity: Activity) -> Self {
        Self {
            activity: Some(activity),
            ..Self::new(layout, edges)
        }
    }

    /// Clears a stale `interrupt_taken` before the first frame.
    pub fn start(&mut self, bus: &mut Bus) {
        bus.interrupt_taken = None;
    }

    /// Call before stepping a frame whose input presses reset: the CPU
    /// jumps to the reset handler with no transfer, no push and no
    /// interrupt, and the collector records it as one edge.
    pub fn will_reset(&mut self) {
        self.reset = true;
    }

    /// Whether the run goes through the shim: no observer needs every
    /// instruction.
    pub fn shimmed(&self) -> bool {
        self.activity.is_none() && self.ramrole.is_none()
    }

    /// Attaches a shim to the console, when the run can go through one,
    /// asking it for the transfers L1 reads. Call once, after any restore
    /// (a restored state has no shim) and before the first frame.
    pub fn attach(&mut self, bus: &mut Bus) {
        if !self.shimmed() {
            return;
        }
        let (rom_start, rom_len, ram_start, ram_len) = self.layout.arena();
        let mut s = Shim::new(rom_start, rom_len, ram_start, ram_len);
        if self.edges.is_some() {
            for op in 0..=255u8 {
                if crate::edges::transfers(op) {
                    s.follow_opcode(op);
                }
            }
        }
        bus.shim = Some(Box::new(s));
    }

    /// Runs one frame with `input`, watched, and ends it: through the
    /// shim when one is attached ([`Collector::attach`]), else calling
    /// [`Collector::before`] before each instruction. A reset in the input
    /// is recorded as one edge.
    pub fn step(&mut self, nes: &mut Nes, input: InputFrame) -> Result<FrameOutcome, NesError> {
        if input.reset {
            self.will_reset();
        }
        let out = if nes.bus().shim.is_some() {
            if self.reset
                && let Some(s) = nes.bus_mut_for_observers().shim.as_deref_mut()
            {
                s.follow = true;
            }
            nes.step_with(input, |bus| {
                if bus.shim_before() {
                    self.hook(bus);
                }
            })?
        } else {
            nes.step_with(input, |bus| self.before(bus))?
        };
        self.end_frame();
        self.sync(nes.bus());
        Ok(out)
    }

    /// Called by the shim before an instruction it was asked about: the
    /// same as [`Collector::before`] but with the instruction's bytes
    /// already marked.
    fn hook(&mut self, bus: &mut Bus) {
        let Some(s) = bus.shim.as_deref_mut() else {
            return;
        };
        if self.reset {
            s.forget();
        }
        if let Some(o) = s.here.filter(|&o| s.hooked(o)) {
            let cpu = &bus.cpu;
            self.hooked.push(crate::frames::Hooked {
                offset: o as u32,
                cpu: cpu.pc,
                a: cpu.acc,
                x: cpu.x,
                y: cpu.y,
                s: cpu.sp,
                p: cpu.status.bits(),
            });
        }
        let Some(s) = bus.shim.as_deref_mut() else {
            return;
        };
        let (here, pc, last, arrived) = (s.here, s.pc, s.last, s.arrived.take());
        let Some(e) = &mut self.edges else {
            // L0 alone: the shim marked everything, and an interrupt or a
            // reset only needs its record cleared.
            self.reset = false;
            bus.interrupt_taken = None;
            return;
        };
        e.set_last(last.0, last.1);
        // The transfer the last instruction made, its target unknown until now.
        if let Some(t) = arrived {
            e.begin(bus, t.opcode, t.offset, t.pc, t.sp);
        }
        if self.reset {
            self.reset = false;
            bus.interrupt_taken = None;
            e.reset(bus, here);
        }
        let interrupted = matches!(
            bus.interrupt_taken.take(),
            Some(FrameKind::Nmi | FrameKind::Irq)
        );
        if interrupted {
            let sp = bus.cpu.sp;
            let ret = u16::from_le_bytes([
                bus.peek(0x100 | u16::from(sp.wrapping_add(2))),
                bus.peek(0x100 | u16::from(sp.wrapping_add(3))),
            ]);
            let resumed = (self.layout.rom_offset(bus, ret), ret);
            e.interrupt(bus, here, resumed);
        }
        e.arrive(here, pc);
    }

    /// Where the interrupted code goes on: the address an interrupt pushed,
    /// just above the flags, and its PRG offset.
    fn resumed(&self, bus: &Bus) -> (Option<usize>, u16) {
        let sp = bus.cpu.sp;
        let ret = u16::from_le_bytes([
            bus.peek(0x100 | u16::from(sp.wrapping_add(2))),
            bus.peek(0x100 | u16::from(sp.wrapping_add(3))),
        ]);
        (self.layout.rom_offset(bus, ret), ret)
    }

    /// Brings L0 up to date with what the shim marked.
    pub fn sync(&mut self, bus: &Bus) {
        if let Some(s) = bus.shim.as_deref() {
            self.footprint.from_shim(s);
        }
    }

    /// Call before each instruction, through `Nes::step_with`.
    #[inline]
    pub fn before(&mut self, bus: &mut Bus) {
        if self.reset {
            self.reset = false;
            // Whatever the reset left as the last vector taken isn't one.
            bus.interrupt_taken = None;
            if let Some(e) = &mut self.edges {
                let here = self.layout.rom_offset(bus, bus.cpu.pc);
                e.reset(bus, here);
            }
        }
        let interrupted = matches!(
            bus.interrupt_taken.take(),
            Some(FrameKind::Nmi | FrameKind::Irq)
        );
        if let Some(a) = &mut self.activity {
            a.observe(bus, interrupted);
        }
        if let Some(t) = &mut self.ramrole {
            t.observe(bus, interrupted);
        }
        if interrupted {
            self.footprint.interrupted(bus);
            let resumed = self.resumed(bus);
            if let Some(e) = &mut self.edges {
                let here = self.layout.rom_offset(bus, bus.cpu.pc);
                e.interrupt(bus, here, resumed);
            }
        }
        let (here, opcode) = self.footprint.instruction(bus);
        if let Some(e) = &mut self.edges {
            e.instruction(bus, here, opcode);
        }
    }

    /// Ends a frame and gives the activity observer's frame, when it has
    /// one.
    pub fn end_frame_activity(&mut self, bus: &Bus) -> Option<FrameActivity> {
        self.end_frame();
        self.activity.as_mut().map(|a| a.end_frame(bus))
    }

    /// Ends a frame.
    pub fn end_frame(&mut self) {
        if let Some(e) = &mut self.edges {
            e.end_frame();
        }
    }

    /// Adds another run's levels (of the same ROM).
    pub fn merge(&mut self, other: &Collector) -> Result<(), String> {
        self.footprint.merge(&other.footprint)?;
        match (&mut self.edges, &other.edges) {
            (Some(a), Some(b)) => a.merge(b),
            (None, None) => {}
            _ => return Err("can't merge a run with edges and one without".into()),
        }
        Ok(())
    }
}
