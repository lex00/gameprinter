//! Generates the data the crate compiles in, into `OUT_DIR`:
//!
//! - `ntsc_palette.bin`: the NTSC filter's color lookup table. Generating it costs ~30 ms of `powf`
//!   and `sin_cos`. Paid lazily by whichever frame first uses the NTSC filter, that lands as a
//!   visible hitch the moment the filter is switched on; it depends on nothing but constants, so it
//!   is computed here instead.
//! - `ntscpalette.pal` and `game_db.dat`: the NTSC palette and the cartridge database, built from
//!   the text files `ntscpalette.txt` and `game_db.txt`. Each must come out byte for byte as the
//!   binary file upstream ships, which is checked against that file's SHA-256.

include!("src/video/ntsc_palette.rs");

use flate2::{Compression, write::DeflateEncoder};
use sha2::{Digest, Sha256};
use std::{io::Write, path::Path};

/// SHA-256 of upstream's `ntscpalette.pal` (tetanes-core 0.17.0).
const NTSC_PAL_SHA256: &str = "d99e88417a54d73d8627a70406c48368b36e01f166156c6dc395d3e65b273444";
/// SHA-256 of upstream's `game_db.dat` (tetanes-core 0.17.0).
const GAME_DB_SHA256: &str = "c42d7e14ec5c48b1393905f1341aed2fe89f5ee08db132b45982c339e89d913b";

fn main() {
    for path in [
        "src/video/ntsc_palette.rs",
        "build.rs",
        "ntscpalette.txt",
        "game_db.txt",
    ] {
        println!("cargo::rerun-if-changed={path}");
    }

    let out_dir = std::env::var_os("OUT_DIR").expect("OUT_DIR is set by cargo");
    let out_dir = Path::new(&out_dir);
    write(&out_dir.join("ntsc_palette.bin"), &generate_ntsc_palette());
    write(
        &out_dir.join("ntscpalette.pal"),
        &checked("ntscpalette.txt", ntsc_pal(), NTSC_PAL_SHA256),
    );
    write(
        &out_dir.join("game_db.dat"),
        &checked("game_db.txt", game_db(), GAME_DB_SHA256),
    );
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::write(path, bytes)
        .unwrap_or_else(|err| panic!("failed to write {}: {err}", path.display()));
}

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|err| panic!("failed to read {path}: {err}"))
}

/// The lines of a text file that hold data: not blank, not a `#` comment, with their line numbers.
fn data_lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.lines()
        .enumerate()
        .map(|(n, line)| (n + 1, line.trim()))
        .filter(|(_, line)| !line.is_empty() && !line.starts_with('#'))
}

fn checked(source: &str, bytes: Vec<u8>, want: &str) -> Vec<u8> {
    let got: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert!(
        got == want,
        "{source} builds {} bytes with SHA-256 {got}, but the file it replaces had SHA-256 {want}",
        bytes.len()
    );
    bytes
}

/// `ntscpalette.txt`: `emphasis colour rgb` per line, entry `emphasis * 64 + colour`, in order.
fn ntsc_pal() -> Vec<u8> {
    let text = read("ntscpalette.txt");
    let mut bytes = Vec::with_capacity(512 * 3);
    for (index, (n, line)) in data_lines(&text).enumerate() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [emphasis, colour, rgb] = fields[..] else {
            panic!("ntscpalette.txt:{n}: expected `emphasis colour rgb`, found {line:?}");
        };
        let emphasis = u32::from_str_radix(emphasis, 10);
        let colour = u32::from_str_radix(colour, 16);
        assert!(
            emphasis == Ok(index as u32 / 64) && colour == Ok(index as u32 % 64),
            "ntscpalette.txt:{n}: expected entry {} {:02X} here, found {line:?}",
            index / 64,
            index % 64
        );
        let rgb = u32::from_str_radix(rgb, 16)
            .ok()
            .filter(|_| rgb.len() == 6)
            .unwrap_or_else(|| panic!("ntscpalette.txt:{n}: {rgb:?} isn't RRGGBB hex"));
        bytes.extend_from_slice(&rgb.to_be_bytes()[1..]);
    }
    assert!(
        bytes.len() == 512 * 3,
        "ntscpalette.txt has {} entries, expected 512 (64 colours times 8 emphasis settings)",
        bytes.len() / 3
    );
    bytes
}

/// `game_db.txt`: `crc=HEX region=NAME mapper=N submapper=N` per line, sorted by CRC.
///
/// The output is what `fs::save_version(.., &Vec<GameInfo>, GAME_DB_VERSION)` writes: the save
/// header, then the bincode (legacy config) encoding deflate-compressed at the default level.
/// bincode's legacy encoding is a `u64` length, then per entry the `u32` CRC, the `NesRegion`
/// variant index as a `u32`, the `u16` mapper and the `u8` submapper, all little-endian.
fn game_db() -> Vec<u8> {
    let text = read("game_db.txt");
    let mut entries = Vec::new();
    let mut last_crc = None;
    for (n, line) in data_lines(&text) {
        let fields: Vec<(&str, &str)> = line
            .split_whitespace()
            .map(|field| field.split_once('=').unwrap_or((field, "")))
            .collect();
        let [
            ("crc", crc),
            ("region", region),
            ("mapper", mapper),
            ("submapper", submapper),
        ] = fields[..]
        else {
            panic!(
                "game_db.txt:{n}: expected `crc=.. region=.. mapper=.. submapper=..`, found {line:?}"
            );
        };
        let bad = |what: &str| -> ! { panic!("game_db.txt:{n}: bad {what} in {line:?}") };
        let crc = u32::from_str_radix(crc, 16).unwrap_or_else(|_| bad("crc"));
        // Variant indexes of `NesRegion` (src/common.rs), which is what bincode writes.
        let region: u32 = match region {
            "Auto" => 0,
            "Ntsc" => 1,
            "Pal" => 2,
            "Dendy" => 3,
            _ => bad("region"),
        };
        let mapper: u16 = mapper.parse().unwrap_or_else(|_| bad("mapper"));
        let submapper: u8 = submapper.parse().unwrap_or_else(|_| bad("submapper"));
        assert!(
            last_crc < Some(crc),
            "game_db.txt:{n}: entries must be sorted by crc with no repeats (the lookup is a binary search)"
        );
        last_crc = Some(crc);
        entries.push((crc, region, mapper, submapper));
    }

    let mut encoded = Vec::with_capacity(8 + entries.len() * 11);
    encoded.extend_from_slice(&(entries.len() as u64).to_le_bytes());
    for (crc, region, mapper, submapper) in entries {
        encoded.extend_from_slice(&crc.to_le_bytes());
        encoded.extend_from_slice(&region.to_le_bytes());
        encoded.extend_from_slice(&mapper.to_le_bytes());
        encoded.push(submapper);
    }

    // The header `fs::write_header` writes: magic, then `fs::GAME_DB_VERSION`.
    let mut bytes = b"TETANES\x1a1".to_vec();
    let mut encoder = DeflateEncoder::new(&mut bytes, Compression::default());
    encoder.write_all(&encoded).expect("writes to a Vec");
    encoder.finish().expect("writes to a Vec");
    bytes
}
