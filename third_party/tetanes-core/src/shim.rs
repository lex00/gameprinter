//! Game Printer: the cartridge shim, a collector's view of the console from inside it
//! (docs/research/rom-event-model.md in Game Printer, section 5).
//!
//! The CPU marks each instruction's PRG ROM bytes as it starts it (the opcode and its operands,
//! by offset into PRG ROM), and each CPU write on its way to the bus: internal RAM and PRG RAM
//! bytes written, and writes to watched addresses with their value and writer. A collector is
//! called out to only where it asked: before an instruction whose opcode it set in
//! [`Shim::opcodes`], at a PRG offset it set in [`Shim::hooks`], after an interrupt, when it
//! set [`Shim::follow`] to see the next instruction, or at the instruction after one whose
//! opcode it set in [`Shim::transfers`], with that one in [`Shim::arrived`]. So a transfer of
//! control costs one call, where it went. Everything else is a bit set inline.
//!
//! The shim only observes. It is never serialized and never copied into a snapshot, so a
//! console with one reaches the same states as one without.

use crate::cpu::Cpu;

/// A CPU write the shim saw to a watched address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShimWrite {
    /// The address as the CPU wrote it.
    pub addr: u16,
    /// The value written.
    pub value: u8,
    /// The writing instruction's opcode.
    pub opcode: u8,
    /// Its CPU address.
    pub pc: u16,
    /// Its PRG ROM offset, if it ran from PRG ROM.
    pub offset: Option<usize>,
    /// The instruction's number since the shim was attached ([`Shim::instructions`] when it
    /// ran), so two writes by one read-modify-write tell apart from two instructions' writes.
    pub instruction: u64,
    /// A push to the stack (by `PHA`, `PHP`, `JSR`, `BRK` or an interrupt), not a write the
    /// instruction's operand names.
    pub push: bool,
}

/// An instruction whose opcode is in [`Shim::transfers`], kept until the next one starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transfer {
    /// Its opcode.
    pub opcode: u8,
    /// Its PRG ROM offset, if it ran from PRG ROM.
    pub offset: Option<usize>,
    /// Its CPU address.
    pub pc: u16,
    /// The stack pointer before it ran.
    pub sp: u8,
}

/// What the shim records, and where it calls out.
#[derive(Debug, Clone)]
pub struct Shim {
    /// PRG ROM's and PRG RAM's place in the memory arena (offsets [`Memory::prg_offset`]
    /// gives) and their lengths.
    ///
    /// [`Memory::prg_offset`]: crate::memory::Memory::prg_offset
    rom_start: usize,
    rom_len: usize,
    ram_start: usize,
    ram_len: usize,
    /// Every byte of every instruction run from PRG ROM, by PRG ROM offset, bit `i % 64` of
    /// word `i / 64`.
    pub executed: Vec<u64>,
    /// The opcode byte of each.
    pub starts: Vec<u64>,
    /// For each PRG ROM offset an instruction started at: its length, 0 for none yet, and its
    /// opcode. ROM doesn't change, so an offset seen once needs no decoding again.
    lens: Vec<u8>,
    ops: Vec<u8>,
    /// For each 1 KiB page of PRG ROM, the CPU pages (bit `n` for `$n*0x400`) it ran at.
    pub windows: Vec<u64>,
    /// Internal RAM bytes written ($0000-$07FF, mirrors folded).
    pub ram_written: [u64; 32],
    /// PRG RAM bytes written, by PRG RAM offset.
    pub prg_ram_written: Vec<u64>,
    /// Instructions started.
    pub instructions: u64,
    /// Instructions started anywhere but PRG ROM.
    pub outside_rom: u64,
    /// Opcodes to call out before, wherever they run: bit `op % 64` of word `op / 64`.
    pub opcodes: [u64; 4],
    /// PRG ROM offsets to call out before, as [`Shim::executed`] is laid out. Empty: none.
    pub hooks: Vec<u64>,
    /// Opcodes whose next instruction is called out, with them in [`Shim::arrived`].
    pub transfers: [u64; 4],
    /// The last instruction with an opcode in [`Shim::transfers`], until the next starts.
    pending: Option<Transfer>,
    /// At a call out: the instruction just run, when its opcode is in [`Shim::transfers`].
    pub arrived: Option<Transfer>,
    /// Call out before the next instruction, whatever it is. Cleared by the call.
    pub follow: bool,
    /// The PRG ROM offset of the instruction about to run, set as it starts.
    pub here: Option<usize>,
    /// Its CPU address.
    pub pc: u16,
    /// Its opcode.
    pub opcode: u8,
    /// The instruction before it.
    pub last: (Option<usize>, u16),
    /// Addresses whose writes are kept, as CPU addresses: bit `a % 64` of word `a / 64`. Empty:
    /// none.
    pub watch: Vec<u64>,
    /// Writes to watched addresses since the collector last took them.
    pub writes: Vec<ShimWrite>,
    pushing: bool,
}

#[inline(always)]
fn set(words: &mut [u64], i: usize) {
    words[i / 64] |= 1 << (i % 64);
}

#[inline(always)]
fn has(words: &[u64], i: usize) -> bool {
    words.get(i / 64).is_some_and(|w| w & (1 << (i % 64)) != 0)
}

impl Shim {
    /// A shim for a cartridge whose PRG ROM is `rom_len` bytes at arena offset `rom_start` and
    /// whose PRG RAM is `ram_len` bytes at `ram_start`.
    pub fn new(rom_start: usize, rom_len: usize, ram_start: usize, ram_len: usize) -> Self {
        Self {
            rom_start,
            rom_len,
            ram_start,
            ram_len,
            executed: vec![0; rom_len.div_ceil(64)],
            starts: vec![0; rom_len.div_ceil(64)],
            lens: vec![0; rom_len],
            ops: vec![0; rom_len],
            windows: vec![0; rom_len.div_ceil(1024)],
            ram_written: [0; 32],
            prg_ram_written: vec![0; ram_len.div_ceil(64)],
            instructions: 0,
            outside_rom: 0,
            opcodes: [0; 4],
            transfers: [0; 4],
            pending: None,
            arrived: None,
            hooks: Vec::new(),
            follow: false,
            here: None,
            pc: 0,
            opcode: 0,
            last: (None, 0),
            watch: Vec::new(),
            writes: Vec::new(),
            pushing: false,
        }
    }

    /// Calls out before every instruction with `op`.
    pub fn hook_opcode(&mut self, op: u8) {
        self.opcodes[usize::from(op) / 64] |= 1 << (op % 64);
    }

    /// Calls out after every instruction with `op`, at the next one, with it in
    /// [`Shim::arrived`].
    pub fn follow_opcode(&mut self, op: u8) {
        self.transfers[usize::from(op) / 64] |= 1 << (op % 64);
    }

    /// Forgets a transfer waiting for its next instruction (a reset came between them).
    pub fn forget(&mut self) {
        self.pending = None;
        self.arrived = None;
    }

    /// Calls out before the instruction at PRG ROM offset `offset`.
    pub fn hook_offset(&mut self, offset: usize) {
        if offset < self.rom_len {
            if self.hooks.is_empty() {
                self.hooks = vec![0; self.rom_len.div_ceil(64)];
            }
            set(&mut self.hooks, offset);
        }
    }

    /// Whether the instruction at PRG ROM offset `offset` is hooked.
    pub fn hooked(&self, offset: usize) -> bool {
        has(&self.hooks, offset)
    }

    /// Keeps the writes to CPU address `addr`.
    pub fn watch_addr(&mut self, addr: u16) {
        if self.watch.is_empty() {
            self.watch = vec![0; 1024];
        }
        set(&mut self.watch, usize::from(addr));
    }

    /// The PRG ROM offset of an arena offset, if it is in PRG ROM.
    #[inline(always)]
    fn rom(&self, arena: Option<usize>) -> Option<usize> {
        let o = arena?.checked_sub(self.rom_start)?;
        (o < self.rom_len).then_some(o)
    }
}

impl crate::bus::Bus {
    /// Game Printer: the shim's look at the instruction about to run. Marks its bytes and
    /// returns whether the collector asked to be called before it. Call it before each
    /// [`Bus::clock_instr`](crate::bus::Bus::clock_instr) while a shim is attached.
    #[inline(always)]
    pub fn shim_before(&mut self) -> bool {
        if self.shim.is_none() {
            return false;
        }
        let pc = self.cpu.pc;
        let interrupted = self.interrupt_taken.is_some();
        let sp = self.cpu.sp;
        // A board that serves PRG reads itself, or a cheat, can make a fetch read something other
        // than the ROM byte, so then each instruction is decoded as it is fetched.
        let constant = self.patches.is_empty()
            && !self
                .mapper_ops
                .intersects(crate::mapper::MapperOps::SERVES_PRG_READS);
        let here = if pc < 0x4020 {
            None
        } else {
            self.shim
                .as_deref()
                .and_then(|s| s.rom(self.memory.prg_offset(pc)))
        };
        let known = match (here, self.shim.as_deref()) {
            (Some(o), Some(s)) if constant && s.lens[o] != 0 => Some((s.ops[o], s.lens[o])),
            _ => None,
        };
        let (opcode, len) = match known {
            Some(k) => k,
            None => {
                let opcode = self.peek(pc);
                (
                    opcode,
                    1 + Cpu::INSTR_REF[usize::from(opcode)].addr_mode.operand_len(),
                )
            }
        };
        let memory = &self.memory;
        let Some(s) = self.shim.as_deref_mut() else {
            return false;
        };
        s.last = (s.here, s.pc);
        s.instructions += 1;
        let within = (usize::from(pc) & 0x3FF) + usize::from(len) <= 0x400;
        match here {
            Some(o) => {
                s.windows[o / 1024] |= 1 << (pc >> 10);
                if known.is_none() {
                    set(&mut s.starts, o);
                    if constant {
                        s.lens[o] = len;
                        s.ops[o] = opcode;
                    }
                }
                // Within one 1 KiB page (the page tables' unit) the bytes are consecutive in
                // PRG ROM, and were marked when the offset was first seen; past it, each is
                // looked up.
                if known.is_none() || !within {
                    set(&mut s.executed, o);
                    for i in 1..len {
                        let addr = pc.wrapping_add(u16::from(i));
                        if within {
                            set(&mut s.executed, o + usize::from(i));
                        } else if addr >= 0x4020
                            && let Some(x) = s.rom(memory.prg_offset(addr))
                        {
                            set(&mut s.executed, x);
                        }
                    }
                }
            }
            None => {
                s.outside_rom += 1;
                for i in 1..len {
                    let addr = pc.wrapping_add(u16::from(i));
                    if addr >= 0x4020
                        && let Some(x) = s.rom(memory.prg_offset(addr))
                    {
                        set(&mut s.executed, x);
                    }
                }
            }
        }
        s.here = here;
        s.pc = pc;
        s.opcode = opcode;
        s.arrived = s.pending.take();
        if has(&s.transfers, usize::from(opcode)) {
            s.pending = Some(Transfer {
                opcode,
                offset: here,
                pc,
                sp,
            });
        }
        let want = interrupted
            || s.arrived.is_some()
            || std::mem::take(&mut s.follow)
            || has(&s.opcodes, usize::from(opcode))
            || here.is_some_and(|o| has(&s.hooks, o));
        want
    }

    /// Game Printer: a CPU write, as the shim sees it on its way to the bus.
    #[inline(always)]
    pub(crate) fn shim_write(&mut self, addr: u16, val: u8) {
        let memory = &self.memory;
        let Some(s) = self.shim.as_deref_mut() else {
            return;
        };
        if addr < 0x2000 {
            set(&mut s.ram_written, usize::from(addr) & 0x7FF);
        } else if addr >= 0x4020
            && s.ram_len > 0
            && let Some(o) = memory
                .prg_offset(addr)
                .and_then(|o| o.checked_sub(s.ram_start))
            && o < s.ram_len
        {
            set(&mut s.prg_ram_written, o);
        }
        if has(&s.watch, usize::from(addr)) {
            s.writes.push(ShimWrite {
                addr,
                value: val,
                opcode: s.opcode,
                pc: s.pc,
                offset: s.here,
                instruction: s.instructions,
                push: s.pushing,
            });
        }
    }

    /// Game Printer: marks the writes while `on` as pushes to the stack.
    #[inline(always)]
    pub(crate) fn shim_pushing(&mut self, on: bool) {
        if let Some(s) = self.shim.as_deref_mut() {
            s.pushing = on;
        }
    }
}
