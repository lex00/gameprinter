//! The harvest on made-up Z80 programs (no ROM): record tables walked by
//! IX and IY, a jump table by a doubled byte, IM 2 through the CTC, the
//! random routine and its callers.

use super::*;

/// A 16 KB ROM with `code` placed at each address; RAM at E000-E7FF.
fn board(code: &[(u16, &[u8])]) -> Board {
    let mut rom = vec![0xffu8; 0x4000];
    for (at, bytes) in code {
        let o = usize::from(*at);
        rom[o..o + bytes.len()].copy_from_slice(bytes);
    }
    Board::flat(&rom, (0xe000, 0xe800))
}

fn fact<'a>(h: &'a Harvest, path: &str) -> Option<&'a Value> {
    h.facts.iter().find(|f| f.path == path).map(|f| &f.value)
}

/// The reset sets up IM 2 with I = 0 and the CTC vector $80; channel 0's
/// handler counts frames in E000; the main loop waits for it, walks 4
/// records of 8 bytes at E100 with IX (and their 32-byte partners at E200
/// with IY), calling a per-record routine that jumps through a table by
/// the record's state (IY+0, doubled).
fn program() -> Board {
    board(&[
        // 0000: ld sp,$e800; di; ld a,$00; ld i,a; ld a,$80; out ($f0),a;
        //       im 2; ei
        (
            0x0000,
            &[
                0x31, 0x00, 0xe8, 0xf3, 0x3e, 0x00, 0xed, 0x47, 0x3e, 0x80, 0xd3, 0xf0, 0xed, 0x5e,
                0xfb,
            ],
        ),
        // 000F: call $0200 (wait); call $0300 (walk); jr $000F
        (0x000f, &[0xcd, 0x00, 0x02, 0xcd, 0x00, 0x03, 0x18, 0xf8]),
        // 0080 vector table: channel 0 -> $0100, channel 1 -> $0120
        (0x0080, &[0x00, 0x01, 0x20, 0x01]),
        // 0100: push af; ld a,($e000); inc a; ld ($e000),a; pop af; ei; reti
        (
            0x0100,
            &[
                0xf5, 0x3a, 0x00, 0xe0, 0x3c, 0x32, 0x00, 0xe0, 0xf1, 0xfb, 0xed, 0x4d,
            ],
        ),
        // 0120: ei; reti
        (0x0120, &[0xfb, 0xed, 0x4d]),
        // 0200: ld a,($e000); or a; jr z,$0200; xor a; ld ($e000),a; ret
        (
            0x0200,
            &[
                0x3a, 0x00, 0xe0, 0xb7, 0x28, 0xfa, 0xaf, 0x32, 0x00, 0xe0, 0xc9,
            ],
        ),
        // 0300: ld ix,$e100; ld iy,$e200; ld b,$04
        //       030A: call $0400; ld de,$0008; add ix,de; ld de,$0020;
        //       add iy,de; djnz $0309; ret
        (
            0x0300,
            &[
                0xdd, 0x21, 0x00, 0xe1, 0xfd, 0x21, 0x00, 0xe2, 0x06, 0x04, 0xcd, 0x00, 0x04, 0x11,
                0x08, 0x00, 0xdd, 0x19, 0x11, 0x20, 0x00, 0xfd, 0x19, 0x10, 0xf1, 0xc9,
            ],
        ),
        // 0400: push bc; ld hl,$0480; ld a,(iy+$00); rlca; call $0500;
        //       ld a,(hl); inc hl; ld h,(hl); ld l,a; jp (hl)
        (
            0x0400,
            &[
                0xc5, 0x21, 0x80, 0x04, 0xfd, 0x7e, 0x00, 0x07, 0xcd, 0x00, 0x05, 0x7e, 0x23, 0x66,
                0x6f, 0xe9,
            ],
        ),
        // 0480: the table: state 0 -> $0600, 1 -> $0620, 2 -> $0640
        (0x0480, &[0x00, 0x06, 0x20, 0x06, 0x40, 0x06]),
        // 0500: add a,l; ld l,a; ret nc; inc h; ret (HL += A)
        (0x0500, &[0x85, 0x6f, 0xd0, 0x24, 0xc9]),
        // 0600: state 0: dec (iy+$04); jr nz,$060D; ld (iy+$00),$01;
        //       ld (iy+$04),$20; 060D: pop bc; ret
        (
            0x0600,
            &[
                0xfd, 0x35, 0x04, 0x20, 0x08, 0xfd, 0x36, 0x00, 0x01, 0xfd, 0x36, 0x04, 0x20,
            ],
        ),
        (0x060d, &[0xc1, 0xc9]),
        // 0620: state 1: ld a,(ix+$03); add a,(iy+$06); ld (ix+$03),a;
        //       cp $80; jr c,$060D; ld (iy+$00),$02; jr $060D
        (
            0x0620,
            &[
                0xdd, 0x7e, 0x03, 0xfd, 0x86, 0x06, 0xdd, 0x77, 0x03, 0xfe, 0x80, 0x38, 0xe0, 0xfd,
                0x36, 0x00, 0x02, 0x18, 0xda,
            ],
        ),
        // 0640: state 2: ld a,$03; call $0700; ld (iy+$02),a;
        //       ld (iy+$00),$00; jr $060D
        (
            0x0640,
            &[
                0x3e, 0x03, 0xcd, 0x00, 0x07, 0xfd, 0x77, 0x02, 0xfd, 0x36, 0x00, 0x00, 0x18, 0xbf,
            ],
        ),
        // 0700: random below A: push bc; ld b,a; call $0710; cp b;
        //       jr c,$070E; sub b; jr $0705; 070E: pop bc; ret
        (
            0x0700,
            &[
                0xc5, 0x47, 0xcd, 0x10, 0x07, 0xb8, 0x38, 0x06, 0x90, 0x18, 0xfa, 0x00, 0x00, 0x00,
                0xc1, 0xc9,
            ],
        ),
        // 0710: random: ld a,($e010); rlca; jr nc,$0717; xor $2b;
        //       0718: ld c,a; ld a,r; xor c; ld ($e010),a; ret
        (
            0x0710,
            &[
                0x3a, 0x10, 0xe0, 0x07, 0x30, 0x02, 0xee, 0x2b, 0x4f, 0xed, 0x5f, 0xa9, 0x32, 0x10,
                0xe0, 0xc9,
            ],
        ),
    ])
}

#[test]
fn im2_through_the_ctc_finds_the_handlers_and_the_frame_counter() {
    let h = harvest(&program());
    assert_eq!(fact(&h, "hardware.interrupt_mode"), Some(&Value::Int(2)));
    assert_eq!(
        fact(&h, "hardware.vector_table"),
        Some(&Value::Str("0080".into()))
    );
    assert_eq!(
        fact(&h, "hardware.interrupts.ctc0"),
        Some(&Value::Str("irq_0100".into()))
    );
    assert_eq!(
        fact(&h, "hardware.interrupts.ctc1"),
        Some(&Value::Str("irq_0120".into()))
    );
    assert_eq!(
        fact(&h, "hardware.frame_counter"),
        Some(&Value::Str("E000".into()))
    );
    assert_eq!(
        fact(&h, "hardware.frame_wait"),
        Some(&Value::Str("sub_0200".into()))
    );
}

#[test]
fn records_walked_by_ix_and_iy_are_tables_with_stride_and_count() {
    let h = harvest(&program());
    assert_eq!(
        fact(&h, "entities.slots_E200.stride"),
        Some(&Value::Int(0x20))
    );
    assert_eq!(fact(&h, "entities.slots_E200.count"), Some(&Value::Int(4)));
    assert_eq!(fact(&h, "entities.slots_E100.stride"), Some(&Value::Int(8)));
    assert_eq!(
        fact(&h, "entities.slots_E200.fields"),
        Some(&Value::List(vec![
            Value::Int(0),
            Value::Int(2),
            Value::Int(4),
            Value::Int(6)
        ]))
    );
    assert!(h.facts.iter().any(|f| f.path == "positions.p_E103+8k"));
    assert!(h.facts.iter().any(|f| f.path == "timers.t_E204+20k"));
}

#[test]
fn a_jump_table_by_a_doubled_field_gives_its_dispatch_and_machine() {
    let h = harvest(&program());
    let d = h
        .dispatch
        .iter()
        .find(|d| d.kind == "jump table")
        .expect("the table");
    assert_eq!(d.at, "040F");
    assert!(d.subject.starts_with("E200+20k"), "{}", d.subject);
    assert_eq!(d.cases["0"], vec!["sub_0600".to_string()]);
    assert_eq!(d.cases["2"], vec!["sub_0640".to_string()]);
    let m = h
        .machines
        .iter()
        .find(|m| m.byte == "E200+20k")
        .expect("the machine");
    assert_eq!(m.path, "entities.slots_E200.machine_00");
    let t: Vec<(String, String)> = m
        .transitions
        .iter()
        .map(|t| (t.from.clone(), t.to.clone()))
        .collect();
    for want in [("s0", "s1"), ("s1", "s2"), ("s2", "s0")] {
        assert!(
            t.contains(&(want.0.to_string(), want.1.to_string())),
            "{want:?} in {t:?}"
        );
    }
}

#[test]
fn the_random_routine_its_wrapper_and_callers() {
    let h = harvest(&program());
    assert_eq!(fact(&h, "random.byte"), Some(&Value::Str("E010".into())));
    assert_eq!(
        fact(&h, "random.routine"),
        Some(&Value::Str("sub_0710".into()))
    );
    assert!(fact(&h, "random.wrappers.sub_0700").is_some());
    let Some(Value::List(c)) = fact(&h, "random.callers") else {
        panic!("callers")
    };
    assert_eq!(c, &vec![Value::Str("sub_0640 at 0642 (sub_0700)".into())]);
}

#[test]
fn the_same_program_gives_the_same_harvest() {
    let a = format!("{:?}", harvest(&program()));
    let b = format!("{:?}", harvest(&program()));
    assert_eq!(a, b);
}

/// Tapper's activations, printed (`cargo test -p gp-arcade dump_tapper --
/// --ignored --nocapture`), when the romset is on the machine.
#[test]
#[ignore = "prints; reads the romset"]
fn dump_tapper() {
    let Some(zip) = crate::romset::local_romset("tapper") else {
        return;
    };
    let set = crate::Romset::load_zip(&zip).expect("the romset");
    let b = Board::mcr_main(&set.maincpu);
    println!("{}", dump(&b, &[0x0144, 0x0158, 0x0584, 0x00d8]));
}

#[test]
#[ignore = "prints"]
fn dump_program() {
    println!("{}", dump(&program(), &[0x0100, 0x0120]));
}
