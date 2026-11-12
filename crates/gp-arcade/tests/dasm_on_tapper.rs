//! The disassembler against the CPU itself: on Tapper, every instruction
//! either CPU runs is followed by the one its length and flow say, unless
//! an interrupt comes between. Skipped without tapper.zip
//! (gp_arcade::romset::local_romset).

use gp_arcade::dasm::{Flow, decode};
use gp_arcade::{Controls, Romset, Tapper};

#[test]
fn lengths_and_flow_match_what_the_cpus_run() {
    let Some(zip) = gp_arcade::romset::local_romset("tapper") else {
        eprintln!("skipped: no tapper romset");
        return;
    };
    let set = Romset::load_zip(&zip).unwrap();
    let mut t = Tapper::power_on(&set);
    let mut checked = 0u64;
    for frame in 1..=900u32 {
        let coin = (600..606).contains(&frame);
        let start = (666..672).contains(&frame);
        let traced = frame % 30 == 0 || frame > 870;
        t.set_trace(traced);
        t.step(Controls {
            coin,
            start,
            ..Controls::default()
        });
        let all = t.take_trace();
        for cpu in 0..2u8 {
            let entries: Vec<_> = all.iter().filter(|e| e.cpu == cpu).collect();
            for w in entries.windows(2) {
                let (a, b) = (w[0], w[1]);
                if a.interrupt.is_some() || b.interrupt.is_some() {
                    continue;
                }
                let i = decode(&a.bytes, a.pc);
                let next = a.pc.wrapping_add(u16::from(i.len));
                // ldir and the other repeating block instructions run
                // again from their own address until BC is done.
                let repeats = i.op == gp_arcade::dasm::Op::Block && i.text.ends_with('r');
                let ok = match i.flow {
                    Flow::Next if repeats => b.pc == next || b.pc == a.pc,
                    Flow::Next => b.pc == next,
                    Flow::Jump(t) => b.pc == t,
                    Flow::JumpIf(t) | Flow::Djnz(t) => b.pc == t || b.pc == next,
                    Flow::Call(t) | Flow::Rst(t) => b.pc == t,
                    Flow::CallIf(t) => b.pc == t || b.pc == next,
                    Flow::Halt => b.pc == a.pc || b.pc == next,
                    // Returns and jp (hl) go where the stack or a register says.
                    Flow::Ret | Flow::RetIf | Flow::Indirect(_) => true,
                };
                assert!(
                    ok,
                    "cpu {cpu} frame {frame}: {:04x} {:02x?} `{}` ({} bytes, {:?}) was followed by {:04x}",
                    a.pc, a.bytes, i.text, i.len, i.flow, b.pc
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 100_000, "{checked}");
}
