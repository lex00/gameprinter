//! L1 on a small program that makes each kind of transfer: a call and its
//! return, a jump through a pointer in RAM, a return to a pushed address
//! (a jump, not a return), a branch taken and one not, a plain jump, and
//! NMIs with their return. Each edge is found with the right kind, ends
//! and pointer, and nothing else is.

use gp_events::{EdgeKind, NO_ROM, run};
use gp_evidence::input_log::InputFrame;
use gp_evidence::manifest::PowerOn;

mod common;
use common::{NMI, SUB, rom};

fn o(a: u16) -> u32 {
    u32::from(a - 0xC000)
}

#[test]
fn finds_each_kind_of_transfer() {
    let inputs = vec![InputFrame::default(); 12];
    let r = run(&rom(), PowerOn::default(), &inputs, true).unwrap();
    let e = r.collector.edges.as_ref().unwrap();
    let has = |k: EdgeKind, from: u16, to: u16| e.table.contains_key(&(k, o(from), o(to)));
    assert!(has(EdgeKind::Jsr, 0xC009, SUB));
    assert!(has(EdgeKind::Rts, SUB, 0xC00C));
    assert!(has(EdgeKind::JmpIndirect, 0xC014, 0xC030));
    assert!(has(EdgeKind::RtsJump, 0xC036, 0xC038));
    assert!(has(EdgeKind::Branch, 0xC03B, 0xC03F));
    assert!(has(EdgeKind::Fall, 0xC03F, 0xC041));
    assert!(has(EdgeKind::Jmp, 0xC043, 0xC009));
    let ptr = &e.table[&(EdgeKind::JmpIndirect, o(0xC014), o(0xC030))].pointers;
    assert_eq!(
        ptr.iter().copied().collect::<Vec<_>>(),
        vec![(0x0010, 0xC030)]
    );

    // NMIs enter the handler, and each returns where it was taken.
    let ints: Vec<_> = e
        .table
        .keys()
        .filter(|k| k.0 == EdgeKind::Interrupt)
        .collect();
    assert!(!ints.is_empty());
    assert!(ints.iter().all(|k| k.2 == o(NMI) && k.1 != NO_ROM));
    let rtis: Vec<_> = e.table.keys().filter(|k| k.0 == EdgeKind::Rti).collect();
    assert!(!rtis.is_empty() && rtis.iter().all(|k| k.1 == o(NMI) && k.2 != NO_ROM));

    // Nothing else moves control in this program.
    let kinds: std::collections::BTreeSet<EdgeKind> = e.table.keys().map(|k| k.0).collect();
    use EdgeKind::*;
    let want: std::collections::BTreeSet<EdgeKind> = [
        Jsr,
        Rts,
        RtsJump,
        Rti,
        Jmp,
        JmpIndirect,
        Branch,
        Fall,
        Interrupt,
    ]
    .into();
    assert_eq!(kinds, want);
    assert_eq!(
        e.entries(),
        [o(SUB), o(0xC030), o(0xC038), o(NMI)].into_iter().collect()
    );

    // Two runs of the same recipe give the same edges.
    let again = run(&rom(), PowerOn::default(), &inputs, true).unwrap();
    assert_eq!(again.collector.edges.unwrap().table, e.table);
}
