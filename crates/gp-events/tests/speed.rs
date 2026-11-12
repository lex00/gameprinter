//! How long a collector costs, against plain stepping, on any ROM run from
//! power-on with no input: `GP_EVENTS_ROM=rom.nes cargo test --release -p
//! gp-events --test speed -- --ignored --nocapture` (`GP_EVENTS_FRAMES`,
//! default 3600). Each mode runs three times and the fastest is kept. The
//! collector runs as a host observer (`before` on every instruction) and
//! through the cartridge shim, which `gp_events::run` uses.

use std::time::Instant;

use gp_events::{Collector, Layout, run};
use gp_evidence::input_log::InputFrame;
use gp_evidence::manifest::PowerOn;
use gp_nes::Nes;

#[test]
#[ignore = "needs a ROM; see the file's comment"]
fn compare_plain_footprint_and_edges() {
    let Ok(path) = std::env::var("GP_EVENTS_ROM") else {
        eprintln!("set GP_EVENTS_ROM");
        return;
    };
    let rom = std::fs::read(path).unwrap();
    let frames: usize = std::env::var("GP_EVENTS_FRAMES")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(3600);
    let inputs = vec![InputFrame::default(); frames];
    let best = |f: &dyn Fn()| {
        (0..3)
            .map(|_| {
                let t = Instant::now();
                f();
                frames as f64 / t.elapsed().as_secs_f64()
            })
            .fold(0.0, f64::max)
    };
    let plain = best(&|| {
        let mut nes = Nes::power_on(&rom, PowerOn::default()).unwrap();
        for i in &inputs {
            nes.step(*i).unwrap();
        }
    });
    let host = |edges: bool| {
        let mut nes = Nes::power_on(&rom, PowerOn::default()).unwrap();
        let mut c = Collector::new(Layout::of(nes.bus()).unwrap(), edges);
        c.start(nes.bus_mut_for_observers());
        for i in &inputs {
            c.step(&mut nes, *i).unwrap();
        }
    };
    let h0 = best(&|| host(false));
    let h1 = best(&|| host(true));
    let l0 = best(&|| {
        run(&rom, PowerOn::default(), &inputs, false).unwrap();
    });
    let l1 = best(&|| {
        run(&rom, PowerOn::default(), &inputs, true).unwrap();
    });
    let over = |x: f64| (plain / x - 1.0) * 100.0;
    println!("plain step:          {plain:>8.0} frames/s");
    println!(
        "L0, host observer:   {h0:>8.0} frames/s ({:+.1}%)",
        over(h0)
    );
    println!(
        "L0, shim:            {l0:>8.0} frames/s ({:+.1}%)",
        over(l0)
    );
    println!(
        "L0 and L1, host:     {h1:>8.0} frames/s ({:+.1}%)",
        over(h1)
    );
    println!(
        "L0 and L1, shim:     {l1:>8.0} frames/s ({:+.1}%)",
        over(l1)
    );
}
