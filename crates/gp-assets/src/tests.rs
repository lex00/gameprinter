//! Lifting and packing a made-up NROM image whose bytes come from a fixed
//! generator, so every format, every edit and every refusal is covered
//! without a game. Moss Hop's own round trip is in gp-cli's tests and in
//! `just check`.

use std::collections::BTreeMap;

use super::*;

/// A 16KB PRG + 8KB CHR iNES image of pseudo-random bytes, with a level
/// grid at PRG 0x100 whose cells are 0 to 3.
fn rom() -> Vec<u8> {
    let mut rom = b"NES\x1a\x01\x01\x01\x00".to_vec();
    rom.resize(16, 0);
    let mut x: u32 = 0x1234_5678;
    for _ in 0..(16384 + 8192) {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        rom.push((x >> 24) as u8);
    }
    for i in 0..48 {
        rom[16 + 0x100 + i] &= 3;
    }
    rom
}

fn sha(rom: &[u8]) -> String {
    RomLayout::of(rom).unwrap().sha256
}

fn map_text(rom: &[u8]) -> String {
    format!(
        r#"
schema = 1
source = "test"
sha256 = "{}"

[[asset]]
name = "tiles"
format = "chr-2bpp"
region = "chr"
offset = 0x10
tiles = 20
tiles_per_row = 8

[[asset]]
name = "palettes"
format = "palette"
region = "prg"
offset = 0x20
count = 8
label = "palette"

[[asset]]
name = "metatiles"
format = "metatiles"
region = "prg"
offset = 0x40
count = 4
names = ["sky", "ground", "block", "goal"]
property = [{{ name = "solid", region = "prg", offset = 0x50 }}]

[[asset]]
name = "level"
format = "level-grid"
region = "prg"
offset = 0x100
width = 8
height = 6
legend = ".#B)"
metatiles = "metatiles"

[[asset]]
name = "enemies"
format = "table"
count = 3
column = [
  {{ name = "x", region = "prg", offset = 0x200, hi_offset = 0x210 }},
  {{ name = "kind", region = "prg", offset = 0x220 }},
]
"#,
        sha(rom)
    )
}

fn lifted(rom: &[u8]) -> (AssetMap, BTreeMap<String, Vec<u8>>) {
    let text = map_text(rom);
    let map = AssetMap::parse(&text).unwrap();
    let (files, _) = lift(rom, &map, &text).unwrap();
    let files = files.into_iter().map(|f| (f.name, f.bytes)).collect();
    (map, files)
}

fn pack(map: &AssetMap, files: &BTreeMap<String, Vec<u8>>) -> Result<Vec<Encoded>, Error> {
    encode(map, &|name| {
        files
            .get(name)
            .cloned()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, name.to_owned()))
    })
}

/// File offsets where two ROMs differ.
fn changed(a: &[u8], b: &[u8]) -> Vec<usize> {
    (0..a.len()).filter(|&i| a[i] != b[i]).collect()
}

fn text_of(files: &BTreeMap<String, Vec<u8>>, name: &str) -> String {
    String::from_utf8(files[name].clone()).unwrap()
}

#[test]
fn round_trip_is_byte_exact() {
    let rom = rom();
    let (map, files) = lifted(&rom);
    assert_eq!(
        files.keys().cloned().collect::<Vec<_>>(),
        [
            "assets.toml",
            "enemies.toml",
            "level.toml",
            "metatiles.toml",
            "palettes.toml",
            "tiles.png"
        ]
    );
    let encoded = pack(&map, &files).unwrap();
    assert!(differences(&rom, &map, &encoded).unwrap().is_empty());
    assert_eq!(apply(&rom, &map, &encoded).unwrap(), rom);
    // 20 tiles, 8 palettes, 4 metatiles and their flags, a 48-cell level,
    // and three rows of a split 16-bit column and a byte column.
    assert_eq!(map.bytes_covered(), 320 + 32 + 16 + 4 + 48 + 9);
}

/// The lifted files are the same bytes on every platform: this digest is
/// checked natively, under Rosetta and in the Linux container.
#[test]
fn lifted_files_are_deterministic() {
    let rom = rom();
    let (_, files) = lifted(&rom);
    let mut h = blake3::Hasher::new();
    for (name, bytes) in &files {
        if name == MAP_FILE {
            continue;
        }
        h.update(name.as_bytes());
        h.update(&(bytes.len() as u64).to_le_bytes());
        h.update(bytes);
    }
    assert_eq!(
        h.finalize().to_hex().as_str(),
        "85c9e3f423101532eb79f21d735dbe89bcb2f9849107851f80b747fb6764e037"
    );
}

#[test]
fn text_files_read_like_the_data() {
    let rom = rom();
    let (_, files) = lifted(&rom);
    let level = text_of(&files, "level.toml");
    assert!(level.contains("#   #  1 ground\n"), "{level}");
    assert!(level.contains("rows = [\n  \""), "{level}");
    let palettes = text_of(&files, "palettes.toml");
    assert!(palettes.contains(", # sprites 3\n"), "{palettes}");
    let metatiles = text_of(&files, "metatiles.toml");
    assert!(
        metatiles.contains("# 3: goal, level character \")\"\n[[metatile]]\ntiles = [0x"),
        "{metatiles}"
    );
    assert!(text_of(&files, "enemies.toml").contains("[[row]]\nx = "));
}

/// Changing one pixel changes only the bits of that pixel's row in the
/// tile's two planes.
#[test]
fn one_pixel_edit_changes_only_its_bytes() {
    let rom = rom();
    let (map, mut files) = lifted(&rom);
    let (w, h) = chr::image_size(20, 8);
    let mut pixels = chr::read_png(&files["tiles.png"], (w, h)).unwrap();
    // Tile 9 is the second row's second tile; pixel (3, 5) inside it.
    let (x, y) = (8 + 3, 8 + 5);
    let old = pixels[(y * w + x) as usize];
    let new = old ^ 3; // flips both planes' bits
    pixels[(y * w + x) as usize] = new;
    files.insert("tiles.png".into(), chr::write_png(w, h, &pixels));

    let packed = apply(&rom, &map, &pack(&map, &files).unwrap()).unwrap();
    let tile = 16 + 16384 + 0x10 + 9 * 16;
    assert_eq!(changed(&rom, &packed), [tile + 5, tile + 8 + 5]);
    assert_eq!(rom[tile + 5] ^ packed[tile + 5], 1 << (7 - 3));
}

#[test]
fn one_level_cell_edit_changes_one_byte() {
    let rom = rom();
    let (map, mut files) = lifted(&rom);
    let level = text_of(&files, "level.toml");
    // Row 2, column 5, set to the block.
    let start = level.find("rows = [\n").unwrap() + "rows = [\n".len();
    let row2 = start + 2 * ("  \"".len() + 8 + "\", # 0\n".len()) + "  \"".len();
    let mut bytes = level.into_bytes();
    let was = bytes[row2 + 5];
    let now = if was == b'B' { b'#' } else { b'B' };
    bytes[row2 + 5] = now;
    files.insert("level.toml".into(), bytes);

    let packed = apply(&rom, &map, &pack(&map, &files).unwrap()).unwrap();
    let cell = 16 + 0x100 + 2 * 8 + 5;
    assert_eq!(changed(&rom, &packed), [cell]);
    assert_eq!(packed[cell], ".#B)".find(now as char).unwrap() as u8);
}

#[test]
fn a_split_column_writes_both_halves() {
    let rom = rom();
    let (map, mut files) = lifted(&rom);
    let text = text_of(&files, "enemies.toml");
    let line = text.lines().find(|l| l.starts_with("x = ")).unwrap();
    let old: u32 = line[4..].parse().unwrap();
    let new = old ^ 0x0101;
    files.insert(
        "enemies.toml".into(),
        text.replacen(line, &format!("x = {new}"), 1).into_bytes(),
    );
    let packed = apply(&rom, &map, &pack(&map, &files).unwrap()).unwrap();
    assert_eq!(changed(&rom, &packed), [16 + 0x200, 16 + 0x210]);
}

#[test]
fn differences_name_the_asset_and_offset() {
    let rom = rom();
    let (map, files) = lifted(&rom);
    let encoded = pack(&map, &files).unwrap();
    let mut other = rom.clone();
    other[16 + 0x22] ^= 0x01; // palette 0, color 2
    // Another ROM (an edited one) is compared all the same: the map's
    // SHA-256 pins what lift reads, and a comparison can only find real
    // differences.
    assert!(!RomLayout::of(&other).unwrap().is_for(&map));
    let d = differences(&other, &map, &encoded).unwrap();
    assert_eq!(d.len(), 1);
    assert_eq!(
        (
            d[0].asset.as_str(),
            d[0].region,
            d[0].offset,
            d[0].file_offset
        ),
        ("palettes", "prg", 0x22, 16 + 0x22)
    );
}

#[test]
fn eight_bit_indexed_png_is_accepted() {
    let rom = rom();
    let (map, mut files) = lifted(&rom);
    let (w, h) = chr::image_size(20, 8);
    let pixels = chr::read_png(&files["tiles.png"], (w, h)).unwrap();
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Indexed);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_palette(vec![0u8; 3 * 16]);
    let mut wr = enc.write_header().unwrap();
    wr.write_image_data(&pixels).unwrap();
    wr.finish().unwrap();
    files.insert("tiles.png".into(), out.clone());
    let encoded = pack(&map, &files).unwrap();
    assert!(differences(&rom, &map, &encoded).unwrap().is_empty());

    // An index past 3 is refused, naming the pixel.
    let mut bad = pixels.clone();
    bad[w as usize + 2] = 5;
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Indexed);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_palette(vec![0u8; 3 * 16]);
    let mut wr = enc.write_header().unwrap();
    wr.write_image_data(&bad).unwrap();
    wr.finish().unwrap();
    files.insert("tiles.png".into(), out);
    let err = pack(&map, &files).unwrap_err().to_string();
    assert!(
        err.contains("tiles.png: pixel (2, 1) has color index 5"),
        "{err}"
    );
}

#[test]
fn bad_edits_are_refused_with_the_file_and_entry() {
    let rom = rom();
    let (map, files) = lifted(&rom);
    type Edit = dyn Fn(String) -> String;
    let cases: [(&str, &Edit, &str); 5] = [
        (
            "level.toml",
            &|t| t.replacen("rows = [\n  \"", "rows = [\n  \"x", 1),
            "row 0 has 9 characters",
        ),
        (
            "level.toml",
            &|t| {
                let i = t.find("rows = [\n  \"").unwrap() + 12;
                format!("{}Q{}", &t[..i], &t[i + 1..])
            },
            "row 0, column 0: 'Q' isn't in the legend",
        ),
        (
            "metatiles.toml",
            &|t| t.replacen("solid = ", "solid = 256 # ", 1),
            "metatile 0: solid = 256 is outside 0 to 255",
        ),
        (
            "palettes.toml",
            &|t| t.replace("schema = 1", "schema = 1\ncolour = 3"),
            "unknown key \"colour\"",
        ),
        (
            "enemies.toml",
            &|t| t.replacen("[[row]]", "[[row]]\nspeed = 2", 1),
            "entry 0: unknown key \"speed\"",
        ),
    ];
    for (file, edit, want) in cases {
        let mut files = files.clone();
        let text = text_of(&files, file);
        files.insert(file.into(), edit(text).into_bytes());
        let err = pack(&map, &files).unwrap_err().to_string();
        assert!(err.starts_with(&format!("{file}: ")), "{err}");
        assert!(err.contains(want), "{err}");
    }
}

#[test]
fn maps_are_checked() {
    let rom = rom();
    let text = map_text(&rom);
    let cases = [
        (
            text.replace("offset = 0x50", "offset = 0x4F"),
            "metatiles and metatiles.solid both claim prg offset 0x004f",
        ),
        (
            text.replace("legend = \".#B)\"", "legend = \".#.)\""),
            "repeats",
        ),
        (
            text.replace("name = \"level\"", "name = \"Level\""),
            "lowercase",
        ),
        (
            text.replace("names = [\"sky\", ", "names = ["),
            "3 names for 4",
        ),
        (text.replace("schema = 1", "schema = 2"), "schema 2"),
        (text.replace("tiles_per_row", "per_row"), "unknown field"),
    ];
    for (text, want) in cases {
        let err = AssetMap::parse(&text).unwrap_err().to_string();
        assert!(err.contains(want), "{want}: {err}");
    }
    // A span past the end of its region is refused when the ROM is known.
    let text = text.replace("offset = 0x10\ntiles = 20", "offset = 0x1F00\ntiles = 20");
    let map = AssetMap::parse(&text).unwrap();
    let err = lift(&rom, &map, &text).unwrap_err().to_string();
    assert!(err.contains("past the end of the ROM's chr data"), "{err}");
}

#[test]
fn level_bytes_outside_the_legend_are_refused() {
    let mut rom = rom();
    rom[16 + 0x100 + 9] = 7;
    let text = map_text(&rom);
    let map = AssetMap::parse(&text).unwrap();
    let err = lift(&rom, &map, &text).unwrap_err().to_string();
    assert!(
        err.contains("level: cell 9 (row 1, column 1) holds 7"),
        "{err}"
    );
}

/// A pixel going from index 0 to 1 sets only plane 0's bit, and 0 to 2
/// only plane 1's, so the planes can't be swapped unnoticed.
#[test]
fn each_plane_holds_its_own_bit() {
    let rom = rom();
    let (map, files) = lifted(&rom);
    let (w, h) = chr::image_size(20, 8);
    let base = chr::read_png(&files["tiles.png"], (w, h)).unwrap();
    // Tile 0, row 0: the pixel in column x, whichever index it has now.
    for (index, plane) in [(1u8, 0usize), (2, 8)] {
        let mut pixels = base.clone();
        let x = (0..8)
            .find(|&x| base[x] == 0)
            .expect("an index-0 pixel in row 0");
        pixels[x] = index;
        let mut files = files.clone();
        files.insert("tiles.png".into(), chr::write_png(w, h, &pixels));
        let packed = apply(&rom, &map, &pack(&map, &files).unwrap()).unwrap();
        let tile = 16 + 16384 + 0x10;
        assert_eq!(changed(&rom, &packed), [tile + plane]);
        assert_eq!(rom[tile + plane] ^ packed[tile + plane], 0x80 >> x);
    }
}

/// Sizes that would overflow, and names that would escape a directory or
/// break a comment, are refused when the map is read.
#[test]
fn hostile_maps_are_refused() {
    let rom = rom();
    let text = map_text(&rom);
    let cases = [
        (
            text.replace("tiles = 20\n", "tiles = 268435456\n"),
            "more than any NES ROM holds",
        ),
        (
            text.replace("count = 8\n", "count = 1073741824\n"),
            "more than any NES ROM holds",
        ),
        (
            text.replace("width = 8\nheight = 6", "width = 65536\nheight = 65536"),
            "more than any NES ROM holds",
        ),
        (
            text.replace("source = \"test\"", "source = \"../../escaped\""),
            "source \"../../escaped\" must be",
        ),
        (
            text.replace("source = \"test\"", "source = \"test\"\ntitle = \"a\\nb\""),
            "title must be one line",
        ),
    ];
    for (text, want) in cases {
        let err = AssetMap::parse(&text).unwrap_err().to_string();
        assert!(err.contains(want), "{want}: {err}");
    }
}

/// An image of the wrong size is refused from its header, before its
/// pixels are read, so a small file can't make gp allocate gigabytes.
#[test]
fn a_huge_png_is_refused_from_its_header() {
    let rom = rom();
    let (map, mut files) = lifted(&rom);
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, 4096, 4096);
        enc.set_color(png::ColorType::Indexed);
        enc.set_depth(png::BitDepth::One);
        enc.set_palette(vec![0u8; 6]);
        let mut wr = enc.write_header().unwrap();
        wr.write_image_data(&vec![0u8; 4096 / 8 * 4096]).unwrap();
        wr.finish().unwrap();
    }
    files.insert("tiles.png".into(), out);
    let err = pack(&map, &files).unwrap_err().to_string();
    assert!(err.contains("the image is 4096x4096"), "{err}");
}

/// `map_text` with its content section, hashed from `rom`.
fn hashed(text: &str, rom: &[u8], code: bool) -> String {
    let map = AssetMap::parse(text).unwrap();
    let l = RomLayout::of(rom).unwrap();
    let (content, code) =
        content::compute(&map, &rom[l.prg.clone()], &rom[l.chr.clone()], code).unwrap();
    content::write_section(text, &content, code.as_ref())
}

/// A made-up re-release of `rom()`: mapper 3, 32KB of PRG with the old
/// 16KB in the upper half and new bytes below, and 16KB of CHR with the
/// old 8KB in bank 1. `edit` flips bytes at these old CHR offsets.
fn rereleased(edit: &[usize]) -> Vec<u8> {
    let old = rom();
    let mut out = b"NES\x1a\x02\x02\x30\x00".to_vec();
    out.resize(16, 0);
    let mut x: u32 = 0x0BAD_F00D;
    let mut noise = |n: usize| -> Vec<u8> {
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x >> 24) as u8
            })
            .collect()
    };
    out.extend(noise(0x4000));
    out.extend(&old[16..16 + 0x4000]);
    out.extend(noise(0x2000));
    let mut chr = old[16 + 0x4000..].to_vec();
    for &e in edit {
        chr[e] ^= 0x5A;
    }
    out.extend(chr);
    out
}

#[test]
fn content_is_written_checked_and_replaced() {
    let r = rom();
    let text = hashed(&map_text(&r), &r, true);
    let map = AssetMap::parse(&text).unwrap();
    // One entry per span: tiles, palettes, metatiles and its property, the
    // level, and the table's three columns (one split in two).
    assert_eq!(map.content.len(), 8);
    assert_eq!(map.content[0].part, "tiles");
    assert_eq!(
        content::parse_prints(&map.content[0].chunks).unwrap().len(),
        20
    );
    assert!(map.code.is_some());
    // Hashing again replaces the section and changes nothing else.
    assert_eq!(hashed(&text, &r, true), text);
    // The section holds hashes, never the bytes: no 4-byte run of the
    // span's data written as hex appears in it.
    let tiles = &r[16 + 0x4000 + 0x10..16 + 0x4000 + 0x10 + 320];
    let section = &text[text.find(content::MARKER).unwrap()..].to_lowercase();
    for w in tiles.windows(4) {
        let hex: String = w.iter().map(|b| format!("{b:02x}")).collect();
        assert!(
            !section.contains(&hex) || w.iter().all(|&b| b == w[0]),
            "{hex}"
        );
    }
    // An entry that doesn't match a span is refused.
    let wrong = text.replacen("len = 320", "len = 336", 1);
    let e = AssetMap::parse(&wrong).unwrap_err().to_string();
    assert!(e.contains("isn't a span of the map"), "{e}");
    let wrong = text.replacen("part = \"tiles\"", "part = \"palettes\"", 1);
    assert!(AssetMap::parse(&wrong).is_err());
}

#[test]
fn the_source_itself_is_found_unchanged() {
    let r = rom();
    let map = AssetMap::parse(&hashed(&map_text(&r), &r, true)).unwrap();
    let rep = find::find(&r, &map).unwrap();
    assert!(rep.is_source);
    assert_eq!(rep.count("same"), 5, "{:?}", rep.assets);
    for b in &rep.blocks {
        assert_eq!(b.found.place().unwrap().offset, b.offset, "{}", b.part);
    }
    let code = rep.code.unwrap();
    assert!(code.chunks > 900 && code.found == code.chunks, "{code:?}");
}

#[test]
fn a_rerelease_is_found_at_its_new_offsets_with_its_changes() {
    let r = rom();
    let text = hashed(&map_text(&r), &r, true);
    let map = AssetMap::parse(&text).unwrap();
    // Two bytes changed in tile 2 (old CHR 0x10 + 32 + 5 and + 40).
    let new = rereleased(&[0x10 + 37, 0x10 + 72]);
    let rep = find::find(&new, &map).unwrap();
    assert!(!rep.is_source);
    assert_eq!(rep.mapper, 3);
    assert_eq!(rep.count("same"), 4, "{:?}", rep.assets);
    assert_eq!(rep.count("changed"), 1);
    for b in &rep.blocks {
        let at = b
            .found
            .place()
            .unwrap_or_else(|| panic!("{} missing", b.part));
        match b.region {
            Region::Prg => {
                assert_eq!(at.offset, b.offset + 0x4000, "{}", b.part);
                assert_eq!(at.cpu, Some(0xC000 + b.offset), "{}", b.part);
            }
            Region::Chr => {
                assert_eq!((at.offset, at.bank), (b.offset + 0x2000, Some(1)));
                let find::Found::Changed {
                    same_bytes, differ, ..
                } = &b.found
                else {
                    panic!("tiles: {:?}", b.found);
                };
                assert_eq!(*same_bytes, 320 - 32);
                assert_eq!(differ, &vec![[32, 16], [64, 16]]);
            }
        }
    }
    assert_eq!(rep.shifts[0], (Region::Prg, 0x4000, 7));
    let code = rep.code.as_ref().unwrap();
    assert_eq!(code.found, code.chunks, "{code:?}");

    // The image in the map's layout lifts to the same files as the
    // source, but for the changed tiles.
    let (image, found) = find::assemble(&new, &map, &rep).unwrap();
    assert_eq!(found.len(), 5);
    let (a, _) = lift(&r, &map, &text).unwrap();
    let (b, _) = lift_any(&image, &map, &text).unwrap();
    for (x, y) in a.iter().zip(&b) {
        assert_eq!(x.name, y.name);
        assert_eq!(x.bytes == y.bytes, x.name != "tiles.png", "{}", x.name);
    }
}

#[test]
fn another_game_finds_nothing() {
    let r = rom();
    let map = AssetMap::parse(&hashed(&map_text(&r), &r, true)).unwrap();
    let mut other = rereleased(&[]);
    // Only the new bytes: the old program and tiles are written over.
    let n = other.len();
    other.copy_within(16..16 + 0x2000, 16 + 0x4000);
    other.copy_within(16 + 0x2000..16 + 0x4000, 16 + 0x6000);
    other.copy_within(16 + 0x8000..16 + 0xA000, n - 0x2000);
    let rep = find::find(&other, &map).unwrap();
    assert_eq!(rep.count("missing"), 5, "{:?}", rep.assets);
    let code = rep.code.as_ref().unwrap();
    assert_eq!(code.found, 0, "{code:?}");
    let (_, found) = find::assemble(&other, &map, &rep).unwrap();
    assert!(found.is_empty());
}
