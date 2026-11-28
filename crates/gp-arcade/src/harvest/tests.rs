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

/// crate::testrom::RECORDS: IM 2 through the CTC, a frame counter and
/// its wait, two record tables walked by IX and IY, a jump table by a
/// record's state, and a random routine with its wrapper.
fn program() -> Board {
    board(crate::testrom::RECORDS)
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

/// Each immediate operand is a constant at its own instruction, as the
/// 6502's harvest has them: `cp $80` (0629) a threshold, `ld (iy+$00),$01`
/// (0605) stored by itself and cited by the machine's states, `ld a,$03`
/// before a call (0640) loaded, `xor $2b` (0716) an operand, and no
/// constant at the branch after the compare (062B).
#[test]
fn each_immediate_is_a_constant_at_its_own_instruction() {
    let h = harvest(&program());
    let at = |pc: &str| h.constants.iter().find(|c| c.pc == pc);
    let c = |pc: &str| at(pc).unwrap_or_else(|| panic!("no constant at {pc}"));
    assert_eq!(
        (c("0629").value, c("0629").what.as_str()),
        (0x80, "threshold")
    );
    assert!(at("062B").is_none());
    let s = c("0605");
    assert_eq!((s.value, s.what.as_str()), (1, "stored"));
    assert_eq!(s.then, s.insn);
    assert!(s.cited, "the machine's states cite the store");
    assert_eq!((c("0640").value, c("0640").what.as_str()), (3, "loaded"));
    assert_eq!(
        (c("0716").value, c("0716").what.as_str()),
        (0x2b, "operand")
    );
    assert_eq!(c("0308").routine, "sub_0300");
    // `ld a,n` then a store of A: stored, with the store's text.
    let h = harvest(&board(&[(
        0x0000,
        &[0x3e, 0x05, 0x32, 0x00, 0xe0, 0x18, 0xfe],
    )]));
    let k = h
        .constants
        .iter()
        .find(|c| c.pc == "0000")
        .expect("ld a,$05");
    assert_eq!((k.value, k.what.as_str()), (5, "stored"));
    assert!(
        k.then.starts_with("ld (") && k.then.ends_with(",a"),
        "{}",
        k.then
    );
}

/// Two loops walk records from E100, one 4 bytes apart and one 8: two
/// tables at one start, named apart by their strides.
#[test]
fn two_tables_at_one_start_have_two_names() {
    let h = harvest(&board(&[(
        0x0000,
        &[
            // 0000: ld sp,$e800
            0x31, 0x00, 0xe8, //
            // 0003: ld ix,$e100; ld b,$04
            0xdd, 0x21, 0x00, 0xe1, 0x06, 0x04, //
            // 0009: ld a,(ix+$00); inc a; ld (ix+$00),a; ld de,$0004;
            //       add ix,de; djnz $0009
            0xdd, 0x7e, 0x00, 0x3c, 0xdd, 0x77, 0x00, 0x11, 0x04, 0x00, 0xdd, 0x19, 0x10,
            0xf2, //
            // 0017: ld ix,$e100; ld b,$02
            0xdd, 0x21, 0x00, 0xe1, 0x06, 0x02, //
            // 001D: ld a,(ix+$01); inc a; ld (ix+$01),a; ld de,$0008;
            //       add ix,de; djnz $001D
            0xdd, 0x7e, 0x01, 0x3c, 0xdd, 0x77, 0x01, 0x11, 0x08, 0x00, 0xdd, 0x19, 0x10,
            0xf2, //
            // 002B: jr $0003
            0x18, 0xd6,
        ],
    )]));
    let names: Vec<&str> = h.elements.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"slots_E100_4"), "{names:?}");
    assert!(names.contains(&"slots_E100_8"), "{names:?}");
    assert_eq!(
        fact(&h, "entities.slots_E100_8.stride"),
        Some(&Value::Int(8))
    );
}

#[test]
fn the_same_program_gives_the_same_harvest() {
    let a = format!("{:?}", harvest(&program()));
    let b = format!("{:?}", harvest(&program()));
    assert_eq!(a, b);
}

#[test]
#[ignore = "prints"]
fn dump_program() {
    println!("{}", dump(&program(), &[0x0100, 0x0120]));
}

/// A made-up program with two indirect jumps: one through a constant
/// (followed) and one through a pointer read from RAM at E000, which the
/// reader can't tell: that one is the only stop, in the reset routine,
/// reached, and made from E000.
#[test]
fn an_indirect_jump_through_ram_is_where_the_reading_stops() {
    let b = board(&[
        // 0000: ld sp,$e800; call $0020; ld hl,($e000); jp (hl)
        (
            0x0000,
            &[0x31, 0x00, 0xe8, 0xcd, 0x20, 0x00, 0x2a, 0x00, 0xe0, 0xe9],
        ),
        // 0020: ld hl,$0030; jp (hl)
        (0x0020, &[0x21, 0x30, 0x00, 0xe9]),
        // 0030: ret
        (0x0030, &[0xc9]),
    ]);
    let (_, st) = harvest_stops(&b);
    assert_eq!(st.stops.len(), 1, "{st:?}");
    let s = &st.stops[0];
    assert_eq!((s.pc, s.routine, s.reached), (0x0009, 0x0000, true));
    assert!(
        s.places.iter().any(|p| p.contains("E000")),
        "{:?}",
        s.places
    );
    assert!(st.routines >= 2 && st.code_bytes > 0 && st.rom_bytes == 0x4000);
}
