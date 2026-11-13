//! The static harvest on Tapper's main CPU, against what was found by
//! hand (the games checkout's tapper/labels.toml): the interrupt handlers,
//! the frame counter and its wait, the random routine and its wrapper, the
//! record tables, the main loop's jump table and the per-customer state
//! machine. Skipped without tapper.zip (gp_arcade::romset::local_romset).

use gp_arcade::Romset;
use gp_arcade::harvest::{Board, Harvest, Value, harvest};

fn fact<'a>(h: &'a Harvest, path: &str) -> Option<&'a Value> {
    h.facts.iter().find(|f| f.path == path).map(|f| &f.value)
}

fn s(v: &str) -> Option<Value> {
    Some(Value::Str(v.into()))
}

#[test]
fn tapper_reads_as_found_by_hand() {
    let Some(zip) = gp_arcade::romset::local_romset("tapper") else {
        eprintln!("skipped: no tapper romset");
        return;
    };
    let set = Romset::load_zip(&zip).unwrap();
    let h = harvest(&Board::mcr_main(&set.maincpu));
    assert_eq!(fact(&h, "hardware.interrupt_mode"), Some(&Value::Int(2)));
    assert_eq!(fact(&h, "hardware.vector_table").cloned(), s("00D0"));
    for (ch, irq) in [
        (0, "irq_0144"),
        (1, "irq_0158"),
        (2, "irq_0584"),
        (3, "irq_00D8"),
    ] {
        assert_eq!(
            fact(&h, &format!("hardware.interrupts.ctc{ch}")).cloned(),
            s(irq)
        );
    }
    assert_eq!(fact(&h, "hardware.frame_counter").cloned(), s("E16C"));
    assert!(
        h.facts
            .iter()
            .any(|f| f.path == "hardware.frame_wait" && f.value == Value::Str("sub_60F6".into()))
    );
    assert_eq!(fact(&h, "random.byte").cloned(), s("E390"));
    assert_eq!(fact(&h, "random.routine").cloned(), s("sub_5FBD"));
    assert!(fact(&h, "random.wrappers.sub_5FAC").is_some());
    let Some(Value::List(callers)) = fact(&h, "random.callers") else {
        panic!("no callers")
    };
    for site in ["sub_09C7 at 09DE", "sub_1048 at 106D", "sub_10A7 at 10C6"] {
        assert!(
            callers
                .iter()
                .any(|c| matches!(c, Value::Str(x) if x.starts_with(site))),
            "{site} in {callers:?}"
        );
    }
    for (base, stride, count) in [("E262", 8, 17), ("E2FE", 8, 9), ("E4B2", 32, 17)] {
        let p = format!("entities.slots_{base}");
        assert_eq!(
            fact(&h, &format!("{p}.stride")),
            Some(&Value::Int(stride)),
            "{p}"
        );
        assert_eq!(
            fact(&h, &format!("{p}.count")),
            Some(&Value::Int(count)),
            "{p}"
        );
    }
    // The taps: four of 4 bytes (the walk's count isn't always read).
    assert_eq!(fact(&h, "entities.slots_E346.stride"), Some(&Value::Int(4)));
    assert!(fact(&h, "input.bytes.E38E").is_some());
    let table = h
        .dispatch
        .iter()
        .find(|d| d.at == "60C0" && d.subject.starts_with("E397"))
        .expect("the main loop's table");
    assert_eq!(table.cases.len(), 16);
    let m = h
        .machines
        .iter()
        .find(|m| m.byte == "E4B2+20k")
        .expect("the customer's machine");
    let t: Vec<(&str, &str, &str)> = m
        .transitions
        .iter()
        .map(|t| (t.from.as_str(), t.to.as_str(), t.routine.as_str()))
        .collect();
    for want in [
        ("s0", "s5", "sub_1016"),
        ("s1", "s2", "sub_1048"),
        ("s4", "s0", "sub_1148"),
        ("s5", "s0", "sub_1167"),
    ] {
        assert!(t.contains(&want), "{want:?} in {t:?}");
    }
}
