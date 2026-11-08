//! The static harvest's reader events for one iNES file, as text, for
//! reading by hand: `cargo run -p gp-nes --example harvest_dump -- ROM`.
//! `-- --testroms DIR` writes the crate's own test programs there instead,
//! for running `gp model static` on them.
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--testroms") {
        let dir = std::path::PathBuf::from(args.get(1).expect("usage: --testroms DIR"));
        std::fs::create_dir_all(&dir).expect("the folder");
        use gp_nes::testrom;
        for (name, rom) in [
            ("testrom.nes", testrom::build()),
            ("testrom-port2.nes", testrom::build_port2_alone()),
            ("nmi-rewrite.nes", testrom::nmi::build_rewrite()),
            ("nmi-toggle.nes", testrom::nmi::build_toggle()),
        ] {
            std::fs::write(dir.join(name), rom).expect("writing");
        }
        return;
    }
    let p = args.first().expect("usage: harvest_dump ROM");
    let rom = std::fs::read(p).expect("reading the ROM");
    let (img, mapper, _) = gp_nes::harvest::ines_image(&rom).expect("an iNES file");
    println!("mapper {mapper}");
    if args.get(1).map(String::as_str) == Some("--extra") {
        let extra: Vec<u16> = args[2..]
            .iter()
            .map(|a| u16::from_str_radix(a, 16).expect("hex"))
            .collect();
        let code = gp_nes::codemap::RomCode::of_with(&img, &extra);
        for (i, n) in code.routines.names.iter().enumerate() {
            println!("{n} size {}", code.routines.size[i]);
        }
        let mut r = gp_nes::harvest::absint::Reader::new(&code);
        r.run_vectors();
        for (i, a) in r.acts.iter().enumerate() {
            for e in &a.events {
                if let gp_nes::harvest::absint::Ev::Indirect { .. } = e {
                    println!("act {i} {:04X} {e:?}", a.start);
                }
            }
        }
        return;
    }
    print!("{}", gp_nes::harvest::dump(&img));
}
