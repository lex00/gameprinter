//! Plays a MAME oracle capture's input log in gp-arcade's Tapper and
//! compares every frame's RAM, input ports and picture hash with MAME's.
//!
//!   cargo run --release -p gp-arcade --example compare -- \
//!       --rom ~/Downloads/rbtapper.zip \
//!       --capture ~/.gameprinter/work/oracle/tapper-first-a \
//!       --log scripts/oracle/inputs/tapper-first.input \
//!       [--all] [--png DIR] [--at N] [--dump N]...
//!
//! It prints the first frame that differs and how, or that every frame
//! matches. `--all` goes on to the end and counts the frames that differ in
//! each way. `--png DIR` writes our picture beside MAME's for the first
//! differing frame that has a MAME shot (or frame `--at N`): ours on the
//! left. The pictures show the game's art, so DIR must be outside any repo.
//! `--dump N` prints, for frame N, every 16-byte row of the RAM record
//! that differs, ours above MAME's, and both CPUs' registers. `--trace N`
//! prints every instruction both CPUs start during frame N.

use std::path::PathBuf;
use std::process::ExitCode;

use gp_arcade::oracle::{Capture, FrameDiff, InputLog, compare};
use gp_arcade::video::{HEIGHT, WIDTH};
use gp_arcade::{Romset, Tapper};

struct Args {
    rom: PathBuf,
    capture: PathBuf,
    log: PathBuf,
    all: bool,
    png: Option<PathBuf>,
    at: Option<usize>,
    dump: Vec<usize>,
    trace: Option<usize>,
}

fn args() -> Result<Args, String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut a = Args {
        rom: PathBuf::from(&home).join("Downloads/rbtapper.zip"),
        capture: PathBuf::new(),
        log: PathBuf::new(),
        all: false,
        png: None,
        at: None,
        dump: Vec::new(),
        trace: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--rom" => a.rom = value()?.into(),
            "--capture" => a.capture = value()?.into(),
            "--log" => a.log = value()?.into(),
            "--all" => a.all = true,
            "--png" => a.png = Some(value()?.into()),
            "--at" => a.at = Some(value()?.parse().map_err(|e| format!("--at: {e}"))?),
            "--trace" => a.trace = Some(value()?.parse().map_err(|e| format!("--trace: {e}"))?),
            "--dump" => a
                .dump
                .push(value()?.parse().map_err(|e| format!("--dump: {e}"))?),
            _ => return Err(format!("unknown argument {arg}")),
        }
    }
    if a.capture.as_os_str().is_empty() || a.log.as_os_str().is_empty() {
        return Err("--capture and --log are needed".into());
    }
    Ok(a)
}

fn read_png(path: &std::path::Path) -> Result<Vec<u32>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut dec = png::Decoder::new(std::io::BufReader::new(file));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("png too big")?];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let ch = info.color_type.samples();
    if info.width as usize != WIDTH || info.height as usize != HEIGHT || ch < 3 {
        return Err(format!(
            "{}: not a {WIDTH}x{HEIGHT} RGB picture",
            path.display()
        ));
    }
    Ok(buf[..info.buffer_size()]
        .chunks(ch)
        .map(|p| (u32::from(p[0]) << 16) | (u32::from(p[1]) << 8) | u32::from(p[2]))
        .collect())
}

/// Ours on the left, MAME's on the right, and a third panel marking the
/// pixels that differ in red.
fn write_side_by_side(
    path: &std::path::Path,
    ours: &[u32],
    theirs: &[u32],
) -> Result<usize, String> {
    let w = WIDTH * 3;
    let mut rgb = vec![0u8; w * HEIGHT * 3];
    let mut differ = 0;
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let a = ours[y * WIDTH + x];
            let b = theirs[y * WIDTH + x];
            let diff = if a == b {
                (a >> 2) & 0x3f3f3f
            } else {
                differ += 1;
                0xff0000
            };
            for (panel, c) in [a, b, diff].into_iter().enumerate() {
                let at = (y * w + panel * WIDTH + x) * 3;
                rgb[at] = (c >> 16) as u8;
                rgb[at + 1] = (c >> 8) as u8;
                rgb[at + 2] = c as u8;
            }
        }
    }
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, HEIGHT as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().map_err(|e| e.to_string())?;
    wr.write_image_data(&rgb).map_err(|e| e.to_string())?;
    Ok(differ)
}

fn dump(t: &Tapper, cap: &Capture, n: usize) {
    let ours = t.ram_record();
    let theirs = cap.ram(n);
    println!("frame {n} dump (ours, then MAME's):");
    for row in (0..ours.len()).step_by(16) {
        if ours[row..row + 16] != theirs[row..row + 16] {
            let hex = |b: &[u8]| {
                b.iter()
                    .map(|x| format!("{x:02x}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            println!(
                "  {:<28} {}",
                gp_arcade::oracle::ram_address(row),
                hex(&ours[row..row + 16])
            );
            println!("  {:<28} {}", "", hex(&theirs[row..row + 16]));
        }
    }
    println!("  main {:?}", t.main);
    println!("  ssio {:?}", t.ssio);
    println!(
        "  MAME pcs: main {:04x} ssio {:04x}",
        cap.rows[n].main_pc, cap.rows[n].ssio_pc
    );
}

fn run() -> Result<bool, String> {
    let a = args()?;
    let rom = Romset::load_zip(&a.rom).map_err(|e| e.to_string())?;
    for extra in &rom.extras {
        println!("romset: {extra}");
    }
    let cap = Capture::load(&a.capture)?;
    let log = InputLog::parse(
        &std::fs::read_to_string(&a.log).map_err(|e| format!("{}: {e}", a.log.display()))?,
    )?;
    if log.system != rom.name {
        return Err(format!(
            "the log is for {}, the romset is {}",
            log.system, rom.name
        ));
    }
    let frames = cap.rows.len();
    println!("capture {}: {} frames", a.capture.display(), frames);

    let mut t = Tapper::power_on(&rom);
    let mut first: Option<(usize, FrameDiff)> = None;
    let (mut ram_frames, mut port_frames, mut pic_frames) = (0, 0, 0);
    let mut png_written = false;
    for n in 0..frames {
        if n > 0 {
            if a.trace == Some(n) {
                t.board.trace = Some(Vec::new());
            }
            t.step(log.controls(n));
            if let Some(tr) = t.board.trace.take() {
                for e in tr {
                    println!(
                        "trace {} {} pc {:04x} af {:04x} bc {:04x} de {:04x} hl {:04x}",
                        if e.cpu == 0 { "main" } else { "ssio" },
                        e.time,
                        e.pc,
                        e.af,
                        e.bc,
                        e.de,
                        e.hl
                    );
                }
            }
        }
        if t.watchdog_fired() && first.is_none() {
            println!("frame {n}: the watchdog ran out (MAME would reset here)");
        }
        let d = compare(&t, &cap, n);
        if a.dump.contains(&n) {
            dump(&t, &cap, n);
        }
        let wanted_png = a.at.map_or(!d.is_empty() && !png_written, |at| at == n);
        if let (Some(dir), true, Some(shot)) = (&a.png, wanted_png, &cap.rows[n].shot) {
            let theirs = read_png(&cap.dir.join(shot))?;
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            let path = dir.join(format!("frame-{n:06}-ours-mame-diff.png"));
            let differ = write_side_by_side(&path, &t.picture(), &theirs)?;
            println!(
                "frame {n}: wrote {} ({differ} pixels differ)",
                path.display()
            );
            png_written = true;
        }
        if !d.is_empty() {
            ram_frames += usize::from(!d.ram.is_empty());
            port_frames += usize::from(d.ports.is_some());
            pic_frames += usize::from(d.picture.is_some());
            if first.is_none() {
                println!(
                    "first difference at frame {n} (time {}):\n{}",
                    cap.rows[n].time,
                    d.describe(24)
                );
                println!(
                    "  pcs at the frame end: ours main {:04x} ssio {:04x}, MAME's main {:04x} ssio {:04x}",
                    t.main.pc, t.ssio.pc, cap.rows[n].main_pc, cap.rows[n].ssio_pc
                );
                first = Some((n, d));
                if !a.all && (a.png.is_none() || png_written) {
                    break;
                }
            }
        }
    }
    match first {
        None => {
            println!("all {frames} frames match MAME: RAM, input ports and picture hash");
            Ok(true)
        }
        Some((n, _)) => {
            if a.all {
                println!(
                    "{frames} frames from frame 0; first difference at frame {n}; frames differing: \
                     RAM {ram_frames}, ports {port_frames}, picture {pic_frames}"
                );
            }
            Ok(false)
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("compare: {e}");
            ExitCode::from(2)
        }
    }
}
