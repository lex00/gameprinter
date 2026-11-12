// license:BSD-3-Clause
// copyright-holders:Juergen Buchmueller
//
// A port of MAME 0.276's Zilog Z80 (src/devices/cpu/z80/z80.cpp, z80.h and
// z80.inc). The instructions themselves are in ops.rs, generated from
// MAME's z80.lst by tools/gen_z80.py. This file holds the registers, the
// flag tables and the helpers the instructions call, as z80.cpp has them.
// Only the NMOS Z80 that both of Tapper's boards use is here.

//! MAME's Z80, step for step.
//!
//! MAME runs a CPU for a number of cycles (`icount`) and stops it at the
//! first memory or I/O cycle boundary where the count runs out, which may
//! fall inside an instruction. The next timeslice carries on from there.
//! `m_ref` says where: the prefix, the opcode and the step within it, as in
//! MAME. That's what makes this core match MAME at every frame boundary,
//! and why it's a port rather than a crate: no Rust Z80 crate stops between
//! the memory cycles of an instruction.

#[rustfmt::skip]
mod ops;

/// What a Z80 is wired to. Every call that can reach a device gets the
/// CPU's cycle counter, so the device can work out the CPU's local time
/// (MAME's `local_time()`) and end the timeslice (`abort_timeslice()`, which
/// sets the counter to 0).
pub trait Z80Bus {
    /// A memory read on the data bus.
    fn read(&mut self, addr: u16, icount: &mut i32) -> u8;
    /// A memory write.
    fn write(&mut self, addr: u16, value: u8, icount: &mut i32);
    /// An opcode or operand fetch. MAME reads these through a cache that
    /// only sees ROM and RAM, so they have no side effects.
    fn fetch(&mut self, addr: u16) -> u8;
    fn io_read(&mut self, port: u16, icount: &mut i32) -> u8;
    fn io_write(&mut self, port: u16, value: u8, icount: &mut i32);
    /// The interrupt acknowledge cycle: the daisy chain's vector, or the
    /// CPU's own IRQ callback when it has no chain.
    fn irq_ack(&mut self, icount: &mut i32) -> i32;
    /// RETI, passed to the daisy chain.
    fn reti(&mut self, icount: &mut i32);
    /// Called before each instruction (MAME's debugger hook), for tracing:
    /// the CPU as it is before the instruction, `pc` at its first byte and
    /// `icount` the cycles left in the timeslice.
    #[inline(always)]
    fn instruction(&mut self, _cpu: &Z80) {}
}

pub const CF: u8 = 0x01;
pub const NF: u8 = 0x02;
pub const PF: u8 = 0x04;
pub const VF: u8 = PF;
pub const XF: u8 = 0x08;
pub const HF: u8 = 0x10;
pub const YF: u8 = 0x20;
pub const ZF: u8 = 0x40;
pub const SF: u8 = 0x80;

/// The flag tables z80.cpp builds in device_start.
pub struct Tables {
    pub sz: [u8; 256],
    pub sz_bit: [u8; 256],
    pub szp: [u8; 256],
    pub szhv_inc: [u8; 256],
    pub szhv_dec: [u8; 256],
    pub szhvc_add: [u8; 2 * 256 * 256],
    pub szhvc_sub: [u8; 2 * 256 * 256],
}

fn build_tables() -> Box<Tables> {
    let mut t = Box::new(Tables {
        sz: [0; 256],
        sz_bit: [0; 256],
        szp: [0; 256],
        szhv_inc: [0; 256],
        szhv_dec: [0; 256],
        szhvc_add: [0; 2 * 256 * 256],
        szhvc_sub: [0; 2 * 256 * 256],
    });
    let mut oldval: i32 = 0;
    while oldval < 256 {
        let mut newval: i32 = 0;
        while newval < 256 {
            let i = (oldval * 256 + newval) as usize;
            let j = (256 * 256 + oldval * 256 + newval) as usize;
            let sign_zero = if newval != 0 {
                if newval & 0x80 != 0 { SF } else { 0 }
            } else {
                ZF
            };
            let yx = (newval as u8) & (YF | XF);

            // add or adc without carry
            let val = newval - oldval;
            let mut f = sign_zero | yx;
            if (newval & 0x0f) < (oldval & 0x0f) {
                f |= HF;
            }
            if newval < oldval {
                f |= CF;
            }
            if (val ^ oldval ^ 0x80) & (val ^ newval) & 0x80 != 0 {
                f |= VF;
            }
            t.szhvc_add[i] = f;

            // adc with carry
            let val = newval - oldval - 1;
            let mut f = sign_zero | yx;
            if (newval & 0x0f) <= (oldval & 0x0f) {
                f |= HF;
            }
            if newval <= oldval {
                f |= CF;
            }
            if (val ^ oldval ^ 0x80) & (val ^ newval) & 0x80 != 0 {
                f |= VF;
            }
            t.szhvc_add[j] = f;

            // cp, sub or sbc without carry
            let val = oldval - newval;
            let mut f = NF | sign_zero | yx;
            if (newval & 0x0f) > (oldval & 0x0f) {
                f |= HF;
            }
            if newval > oldval {
                f |= CF;
            }
            if (val ^ oldval) & (oldval ^ newval) & 0x80 != 0 {
                f |= VF;
            }
            t.szhvc_sub[i] = f;

            // sbc with carry
            let val = oldval - newval - 1;
            let mut f = NF | sign_zero | yx;
            if (newval & 0x0f) >= (oldval & 0x0f) {
                f |= HF;
            }
            if newval >= oldval {
                f |= CF;
            }
            if (val ^ oldval) & (oldval ^ newval) & 0x80 != 0 {
                f |= VF;
            }
            t.szhvc_sub[j] = f;
            newval += 1;
        }
        oldval += 1;
    }
    let mut i = 0;
    while i < 256 {
        let b = i as u8;
        let p = b.count_ones();
        t.sz[i] = if b != 0 { b & SF } else { ZF } | (b & (YF | XF));
        t.sz_bit[i] = if b != 0 { b & SF } else { ZF | PF } | (b & (YF | XF));
        t.szp[i] = t.sz[i] | if p & 1 != 0 { 0 } else { PF };
        t.szhv_inc[i] = t.sz[i];
        if b == 0x80 {
            t.szhv_inc[i] |= VF;
        }
        if b & 0x0f == 0x00 {
            t.szhv_inc[i] |= HF;
        }
        t.szhv_dec[i] = t.sz[i] | NF;
        if b == 0x7f {
            t.szhv_dec[i] |= VF;
        }
        if b & 0x0f == 0x0f {
            t.szhv_dec[i] |= HF;
        }
        i += 1;
    }
    t
}

static TABLES: std::sync::LazyLock<Box<Tables>> = std::sync::LazyLock::new(build_tables);

#[inline(always)]
pub(crate) fn tables() -> &'static Tables {
    &TABLES
}

/// One Z80's state: MAME's registers plus the step it stopped at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Z80 {
    pub prvpc: u16,
    pub pc: u16,
    pub sp: u16,
    pub af: u16,
    pub bc: u16,
    pub de: u16,
    pub hl: u16,
    pub ix: u16,
    pub iy: u16,
    pub wz: u16,
    pub af2: u16,
    pub bc2: u16,
    pub de2: u16,
    pub hl2: u16,
    pub qt: u8,
    pub q: u8,
    pub r: u8,
    pub r2: u8,
    pub iff1: u8,
    pub iff2: u8,
    pub halt: u8,
    pub im: u8,
    pub i: u8,
    pub nmi_state: u8,
    pub nmi_pending: bool,
    pub irq_state: u8,
    pub wait_state: i32,
    pub busrq_state: i32,
    pub busack_state: u8,
    pub after_ei: bool,
    pub after_ldair: bool,
    pub ea: u16,
    pub icount: i32,
    pub tmp_irq_vector: i32,
    /// MAME's m_shared_addr (TADR), m_shared_data (TDAT) and
    /// m_shared_data2 (TDAT2): the values carried from one step to the next.
    pub tadr: u16,
    pub tdat: u16,
    pub tdat2: u16,
    pub rtemp: u8,
    /// Prefix << 16 | opcode << 8 | step. 0xffff00 is "check interrupts and
    /// fetch the next opcode".
    pub m_ref: u32,
}

impl Default for Z80 {
    fn default() -> Self {
        Self::new()
    }
}

impl Z80 {
    /// The state after MAME's device_start.
    pub fn new() -> Self {
        let mut z = Self {
            prvpc: 0,
            pc: 0,
            sp: 0,
            af: 0,
            bc: 0,
            de: 0,
            hl: 0,
            ix: 0xffff,
            iy: 0xffff,
            wz: 0,
            af2: 0,
            bc2: 0,
            de2: 0,
            hl2: 0,
            qt: 0,
            q: 0,
            r: 0,
            r2: 0,
            iff1: 0,
            iff2: 0,
            halt: 0,
            im: 0,
            i: 0,
            nmi_state: 0,
            nmi_pending: false,
            irq_state: 0,
            wait_state: 0,
            busrq_state: 0,
            busack_state: 0,
            after_ei: false,
            after_ldair: false,
            ea: 0,
            icount: 0,
            tmp_irq_vector: 0,
            tadr: 0,
            tdat: 0,
            tdat2: 0,
            rtemp: 0,
            m_ref: 0xffff00,
        };
        z.f_set_f(ZF as i64);
        z
    }

    /// MAME's device_reset.
    pub fn reset(&mut self) {
        self.halt = 0;
        self.m_ref = 0xffff00;
        self.pc = 0;
        self.wz = self.pc;
        self.i = 0;
        self.r = 0;
        self.r2 = 0;
        self.nmi_pending = false;
        self.after_ei = false;
        self.after_ldair = false;
        self.iff1 = 0;
        self.iff2 = 0;
    }

    /// MAME's execute_run: runs until `icount` runs out.
    pub fn execute_run<B: Z80Bus>(&mut self, bus: &mut B) {
        if self.wait_state != 0 {
            self.icount = 0;
            return;
        }
        while self.icount > 0 {
            self.do_op(bus);
        }
    }

    /// The IRQ line as execute_set_input sets it, after the daisy chain
    /// has had its say (`chain_asserts`, false without a chain).
    pub fn set_irq_line(&mut self, asserted: bool, chain_asserts: bool) {
        self.irq_state = u8::from(asserted || chain_asserts);
    }

    fn a(&self) -> u8 {
        (self.af >> 8) as u8
    }

    fn f(&self) -> u8 {
        self.af as u8
    }

    fn set_a(&mut self, v: u8) {
        self.af = (self.af & 0x00ff) | (u16::from(v) << 8);
    }

    fn set_f_raw(&mut self, v: u8) {
        self.af = (self.af & 0xff00) | u16::from(v);
    }

    fn f_set_f(&mut self, f: i64) {
        self.qt = 0;
        self.set_f_raw(f as u8);
    }

    fn f_halt(&mut self) {
        if self.halt == 0 {
            self.halt = 1;
        }
    }

    fn f_leave_halt(&mut self) {
        if self.halt != 0 {
            self.halt = 0;
        }
    }

    fn f_inc(&mut self, r: i64) -> i64 {
        let r = (r as u8).wrapping_add(1);
        self.f_set_f(i64::from((self.f() & CF) | tables().szhv_inc[r as usize]));
        i64::from(r)
    }

    fn f_dec(&mut self, r: i64) -> i64 {
        let r = (r as u8).wrapping_sub(1);
        self.f_set_f(i64::from((self.f() & CF) | tables().szhv_dec[r as usize]));
        i64::from(r)
    }

    fn f_rlca(&mut self) {
        let a = self.a().rotate_left(1);
        self.set_a(a);
        self.f_set_f(i64::from(
            (self.f() & (SF | ZF | PF)) | (a & (YF | XF | CF)),
        ));
    }

    fn f_rrca(&mut self) {
        let a0 = self.a();
        self.f_set_f(i64::from((self.f() & (SF | ZF | PF)) | (a0 & CF)));
        let a = a0.rotate_right(1);
        self.set_a(a);
        let f = self.f() | (a & (YF | XF));
        self.set_f_raw(f);
    }

    fn f_rla(&mut self) {
        let a = self.a();
        let res = (a << 1) | (self.f() & CF);
        let c = if a & 0x80 != 0 { CF } else { 0 };
        self.f_set_f(i64::from(
            (self.f() & (SF | ZF | PF)) | c | (res & (YF | XF)),
        ));
        self.set_a(res);
    }

    fn f_rra(&mut self) {
        let a = self.a();
        let res = (a >> 1) | (self.f() << 7);
        let c = if a & 0x01 != 0 { CF } else { 0 };
        self.f_set_f(i64::from(
            (self.f() & (SF | ZF | PF)) | c | (res & (YF | XF)),
        ));
        self.set_a(res);
    }

    fn f_add_a(&mut self, value: i64) {
        let ah = u32::from(self.af & 0xff00);
        let res = u32::from(((ah >> 8) as u8).wrapping_add(value as u8));
        self.f_set_f(i64::from(tables().szhvc_add[(ah | res) as usize]));
        self.set_a(res as u8);
    }

    fn f_adc_a(&mut self, value: i64) {
        let ah = u32::from(self.af & 0xff00);
        let c = u32::from(self.af & 1);
        let res = u32::from(
            ((ah >> 8) as u8)
                .wrapping_add(value as u8)
                .wrapping_add(c as u8),
        );
        self.f_set_f(i64::from(
            tables().szhvc_add[((c << 16) | ah | res) as usize],
        ));
        self.set_a(res as u8);
    }

    fn f_sub(&mut self, value: i64) {
        let ah = u32::from(self.af & 0xff00);
        let res = u32::from(((ah >> 8) as u8).wrapping_sub(value as u8));
        self.f_set_f(i64::from(tables().szhvc_sub[(ah | res) as usize]));
        self.set_a(res as u8);
    }

    fn f_sbc_a(&mut self, value: i64) {
        let ah = u32::from(self.af & 0xff00);
        let c = u32::from(self.af & 1);
        let res = u32::from(
            ((ah >> 8) as u8)
                .wrapping_sub(value as u8)
                .wrapping_sub(c as u8),
        );
        self.f_set_f(i64::from(
            tables().szhvc_sub[((c << 16) | ah | res) as usize],
        ));
        self.set_a(res as u8);
    }

    fn f_neg(&mut self) {
        let value = self.a();
        self.set_a(0);
        self.f_sub(i64::from(value));
    }

    fn f_daa(&mut self) {
        let a0 = self.a();
        let f = self.f();
        let mut a = a0;
        if f & NF != 0 {
            if (f & HF) != 0 || (a0 & 0xf) > 9 {
                a = a.wrapping_sub(6);
            }
            if (f & CF) != 0 || a0 > 0x99 {
                a = a.wrapping_sub(0x60);
            }
        } else {
            if (f & HF) != 0 || (a0 & 0xf) > 9 {
                a = a.wrapping_add(6);
            }
            if (f & CF) != 0 || a0 > 0x99 {
                a = a.wrapping_add(0x60);
            }
        }
        let nf = (f & (CF | NF)) | u8::from(a0 > 0x99) | ((a0 ^ a) & HF) | tables().szp[a as usize];
        self.f_set_f(i64::from(nf));
        self.set_a(a);
    }

    fn f_and_a(&mut self, value: i64) {
        let a = self.a() & (value as u8);
        self.set_a(a);
        self.f_set_f(i64::from(tables().szp[a as usize] | HF));
    }

    fn f_or_a(&mut self, value: i64) {
        let a = self.a() | (value as u8);
        self.set_a(a);
        self.f_set_f(i64::from(tables().szp[a as usize]));
    }

    fn f_xor_a(&mut self, value: i64) {
        let a = self.a() ^ (value as u8);
        self.set_a(a);
        self.f_set_f(i64::from(tables().szp[a as usize]));
    }

    fn f_cp(&mut self, value: i64) {
        let val = u32::from(value as u8);
        let ah = u32::from(self.af & 0xff00);
        let res = u32::from(((ah >> 8) as u8).wrapping_sub(val as u8));
        self.f_set_f(i64::from(
            (tables().szhvc_sub[(ah | res) as usize] & !(YF | XF)) | ((val as u8) & (YF | XF)),
        ));
    }

    fn f_exx(&mut self) {
        std::mem::swap(&mut self.bc, &mut self.bc2);
        std::mem::swap(&mut self.de, &mut self.de2);
        std::mem::swap(&mut self.hl, &mut self.hl2);
    }

    fn rot_flags(&mut self, res: u8, c: u8) -> i64 {
        self.f_set_f(i64::from(tables().szp[res as usize] | c));
        i64::from(res)
    }

    fn f_rlc(&mut self, v: i64) -> i64 {
        let v = v as u8;
        let c = if v & 0x80 != 0 { CF } else { 0 };
        self.rot_flags(v.rotate_left(1), c)
    }

    fn f_rrc(&mut self, v: i64) -> i64 {
        let v = v as u8;
        let c = if v & 0x01 != 0 { CF } else { 0 };
        self.rot_flags(v.rotate_right(1), c)
    }

    fn f_rl(&mut self, v: i64) -> i64 {
        let v = v as u8;
        let c = if v & 0x80 != 0 { CF } else { 0 };
        self.rot_flags((v << 1) | (self.f() & CF), c)
    }

    fn f_rr(&mut self, v: i64) -> i64 {
        let v = v as u8;
        let c = if v & 0x01 != 0 { CF } else { 0 };
        self.rot_flags((v >> 1) | (self.f() << 7), c)
    }

    fn f_sla(&mut self, v: i64) -> i64 {
        let v = v as u8;
        let c = if v & 0x80 != 0 { CF } else { 0 };
        self.rot_flags(v << 1, c)
    }

    fn f_sra(&mut self, v: i64) -> i64 {
        let v = v as u8;
        let c = if v & 0x01 != 0 { CF } else { 0 };
        self.rot_flags((v >> 1) | (v & 0x80), c)
    }

    fn f_sll(&mut self, v: i64) -> i64 {
        let v = v as u8;
        let c = if v & 0x80 != 0 { CF } else { 0 };
        self.rot_flags((v << 1) | 0x01, c)
    }

    fn f_srl(&mut self, v: i64) -> i64 {
        let v = v as u8;
        let c = if v & 0x01 != 0 { CF } else { 0 };
        self.rot_flags(v >> 1, c)
    }

    fn f_bit(&mut self, bit: i64, v: i64) {
        let v = v as u8;
        let f = (self.f() & CF)
            | HF
            | (tables().sz_bit[(v & (1u8 << bit)) as usize] & !(YF | XF))
            | (v & (YF | XF));
        self.f_set_f(i64::from(f));
    }

    fn f_bit_hl(&mut self, bit: i64, v: i64) {
        let v = v as u8;
        let wz_h = (self.wz >> 8) as u8;
        let f = (self.f() & CF)
            | HF
            | (tables().sz_bit[(v & (1u8 << bit)) as usize] & !(YF | XF))
            | (wz_h & (YF | XF));
        self.f_set_f(i64::from(f));
    }

    fn f_bit_xy(&mut self, bit: i64, v: i64) {
        let v = v as u8;
        let f = (self.f() & CF)
            | HF
            | (tables().sz_bit[(v & (1u8 << bit)) as usize] & !(YF | XF))
            | (((self.ea >> 8) as u8) & (YF | XF));
        self.f_set_f(i64::from(f));
    }

    fn f_res(&mut self, bit: i64, v: i64) -> i64 {
        v & !(1i64 << bit)
    }

    fn f_set(&mut self, bit: i64, v: i64) -> i64 {
        v | (1i64 << bit)
    }

    fn f_block_io_interrupted_flags(&mut self) {
        let mut f = self.f();
        let b = (self.bc >> 8) as u8;
        let tdat8 = self.tdat as u8;
        f &= !(YF | XF);
        f |= ((self.pc >> 8) as u8) & (YF | XF);
        if f & CF != 0 {
            f &= !HF;
            if tdat8 & 0x80 != 0 {
                f ^= (tables().szp[(b.wrapping_sub(1) & 0x07) as usize] ^ PF) & PF;
                if b & 0x0f == 0x00 {
                    f |= HF;
                }
            } else {
                f ^= (tables().szp[(b.wrapping_add(1) & 0x07) as usize] ^ PF) & PF;
                if b & 0x0f == 0x0f {
                    f |= HF;
                }
            }
        } else {
            f ^= (tables().szp[(b & 0x07) as usize] ^ PF) & PF;
        }
        self.set_f_raw(f);
    }

    fn f_ei(&mut self) {
        self.iff1 = 1;
        self.iff2 = 1;
        self.after_ei = true;
    }

    fn f_illegal_1(&mut self) {}

    fn f_illegal_2(&mut self) {}
}

#[cfg(test)]
mod tests;
