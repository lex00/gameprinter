use super::*;

/// 64 KB of RAM and nothing else; I/O reads return the port's low byte.
struct Flat {
    mem: Vec<u8>,
    io_writes: Vec<(u16, u8)>,
}

impl Flat {
    fn new(program: &[u8]) -> Self {
        let mut mem = vec![0u8; 0x10000];
        mem[..program.len()].copy_from_slice(program);
        Self {
            mem,
            io_writes: Vec::new(),
        }
    }
}

impl Z80Bus for Flat {
    fn read(&mut self, addr: u16, _: &mut i32) -> u8 {
        self.mem[usize::from(addr)]
    }
    fn write(&mut self, addr: u16, value: u8, _: &mut i32) {
        self.mem[usize::from(addr)] = value;
    }
    fn fetch(&mut self, addr: u16) -> u8 {
        self.mem[usize::from(addr)]
    }
    fn io_read(&mut self, port: u16, _: &mut i32) -> u8 {
        port as u8
    }
    fn io_write(&mut self, port: u16, value: u8, _: &mut i32) {
        self.io_writes.push((port, value));
    }
    fn irq_ack(&mut self, _: &mut i32) -> i32 {
        0xff
    }
    fn reti(&mut self, _: &mut i32) {}
}

fn run(z: &mut Z80, bus: &mut Flat, cycles: i32) -> i32 {
    z.icount = cycles;
    z.execute_run(bus);
    z.icount
}

// MAME's Z80 stops as soon as a step uses up the cycles it was given, so
// an instruction's fetch can end one run and its work start the next. The
// tests below run one cycle at a time where that matters.

#[test]
fn runs_a_small_program_with_mames_cycle_counts() {
    // LD A,5; LD B,3; loop: ADD A,B; DJNZ loop; LD (8000),A; HALT
    let program = [
        0x3e, 0x05, 0x06, 0x03, 0x80, 0x10, 0xfd, 0x32, 0x00, 0x80, 0x76,
    ];
    let mut bus = Flat::new(&program);
    let mut z = Z80::new();
    z.reset();
    // 7 + 7 + 3 * 4 + 13 + 13 + 8 = 60 cycles before LD (nn),A, whose write
    // is its fourth memory cycle, after 4 + 3 + 3: the step that ends on
    // cycle 73.
    let mut spent = 0;
    while bus.mem[0x8000] == 0 {
        spent += 1 - run(&mut z, &mut bus, 1);
        assert!(spent < 1000);
    }
    assert_eq!(spent, 73);
    assert_eq!(bus.mem[0x8000], 5 + 3 + 2 + 1);
    run(&mut z, &mut bus, 20);
    assert_eq!(z.halt, 1);
    assert_eq!(z.bc >> 8, 0);
}

#[test]
fn stopping_mid_instruction_and_going_on_gives_the_same_result() {
    // LD HL,1234; LD SP,9000; PUSH HL; LD IX,8000; LD (IX+5),77;
    // LD A,(IX+5); SLA A; LD BC,3; LD DE,8100; LD HL,0; LDIR; JR $
    let program = [
        0x21, 0x34, 0x12, 0x31, 0x00, 0x90, 0xe5, 0xdd, 0x21, 0x00, 0x80, 0xdd, 0x36, 0x05, 0x77,
        0xdd, 0x7e, 0x05, 0xcb, 0x27, 0x01, 0x03, 0x00, 0x11, 0x00, 0x81, 0x21, 0x00, 0x00, 0xed,
        0xb0, 0x18, 0xfe,
    ];
    let mut whole_bus = Flat::new(&program);
    let mut whole = Z80::new();
    whole.reset();
    let total = 300 - run(&mut whole, &mut whole_bus, 300);

    let mut bus = Flat::new(&program);
    let mut z = Z80::new();
    z.reset();
    let mut spent = 0;
    while spent < total {
        spent += 1 - run(&mut z, &mut bus, 1);
    }
    assert_eq!(spent, total, "both stop on the same step boundary");
    z.icount = whole.icount;
    assert_eq!(z, whole);
    assert_eq!(whole_bus.mem, bus.mem);
    assert_eq!(bus.mem[0x8005], 0x77);
    assert_eq!(&bus.mem[0x8100..0x8103], &program[..3]);
    assert_eq!((bus.mem[0x8ffe], bus.mem[0x8fff]), (0x34, 0x12));
}

#[test]
fn flags_of_sub_and_daa() {
    // LD A,0x15; SUB 0x27; DAA; HALT
    let program = [0x3e, 0x15, 0xd6, 0x27, 0x27, 0x76];
    let mut bus = Flat::new(&program);
    let mut z = Z80::new();
    z.reset();
    run(&mut z, &mut bus, 40);
    // 0x15 - 0x27 = 0xee, borrow; DAA with N and C set gives 0x88.
    assert_eq!(z.af >> 8, 0x88);
    assert_eq!(z.af as u8 & (CF | NF), CF | NF);
}

#[test]
fn takes_an_im1_interrupt_after_ei_and_one_more_instruction() {
    // LD SP,F000; IM 1; EI; NOP; NOP ... with an RST 38 handler that
    // writes a marker.
    let mut program = vec![0x31, 0x00, 0xf0, 0xed, 0x56, 0xfb, 0x00, 0x00, 0x00, 0x00];
    program.resize(0x38, 0x00);
    program.extend_from_slice(&[0x3e, 0xaa, 0x32, 0x00, 0x80, 0x76]);
    let mut bus = Flat::new(&program);
    let mut z = Z80::new();
    z.reset();
    z.set_irq_line(true, false);
    run(&mut z, &mut bus, 200);
    assert_eq!(bus.mem[0x8000], 0xaa);
    // The NOP after EI ran, so the return address is 7.
    assert_eq!((bus.mem[0xeffe], bus.mem[0xefff]), (7, 0));
}

#[test]
fn a_shift_into_a_keeps_the_flags_it_sets() {
    // LD A,0x80; SLA A; HALT. The generated `A = sla(A)` once stored A
    // with the F from before the call, losing the carry (frame 125 of the
    // first capture, in the sound board's RAM test).
    let program = [0x3e, 0x80, 0xcb, 0x27, 0x76];
    let mut bus = Flat::new(&program);
    let mut z = Z80::new();
    z.reset();
    run(&mut z, &mut bus, 40);
    assert_eq!(z.af >> 8, 0);
    assert_eq!(z.af as u8 & (CF | ZF), CF | ZF);
}
