//! The format modules (`byte-rows`, `ppu-strings`) on a
//! made-up NROM image: none of its bytes are a game's. Each round-trips,
//! an edit changes exactly the bytes it should, and bad files and maps are
//! refused.

use std::collections::BTreeMap;

use super::*;

/// A 16KB PRG + 8KB CHR image with `prg` placed at PRG offset 0x100.
fn rom(prg: &[u8]) -> Vec<u8> {
    let mut rom = b"NES\x1a\x01\x01\x00\x00".to_vec();
    rom.resize(16, 0);
    let mut data = vec![0u8; 16384 + 8192];
    data[0x100..0x100 + prg.len()].copy_from_slice(prg);
    rom.extend(data);
    rom
}

const CHARSET: &str = r#"charset = [[0x00, "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ "], [0x28, "-"]]"#;

/// Text "HI" is tiles 0x11, 0x12.
fn ppu_bytes() -> Vec<u8> {
    vec![
        // "first": "HI" at 0x2043, then 5 x tile 0xAA down a column, end.
        0x20, 0x43, 0x02, 0x11, 0x12, 0x23, 0xC0, 0xC5, 0xAA, 0xFF,
        // "open": one update and no end byte; tiles that aren't text.
        0x21, 0x00, 0x02, 0x24, 0x77,
        // "palette": its data is the palette asset's 4 bytes.
        0x3F, 0x00, 0x04, 0x0F, 0x16, 0x30, 0x27, 0xFF,
    ]
}

fn ppu_map(rom: &[u8], size: u32) -> String {
    format!(
        r#"
schema = 1
source = "test"
sha256 = "{}"

[[asset]]
name = "pal"
format = "palette"
region = "prg"
offset = 0x0112
count = 1

[[asset]]
name = "text"
format = "ppu-strings"
region = "prg"
offset = 0x0100
size = {size}
end = 0xFF
{CHARSET}
entry = [{{ name = "first" }}, {{ name = "open", records = 1 }}, {{ name = "palette" }}]
embed = [{{ asset = "pal", offset = 0x0112, size = 4 }}]
"#,
        RomLayout::of(rom).unwrap().sha256
    )
}

type Files = BTreeMap<String, Vec<u8>>;

fn lift_files(rom: &[u8], map_text: &str) -> (AssetMap, Files) {
    let map = AssetMap::parse(map_text).unwrap();
    let (files, _) = lift(rom, &map, map_text).unwrap();
    (map, files.into_iter().map(|f| (f.name, f.bytes)).collect())
}

fn pack(map: &AssetMap, files: &Files) -> Result<Vec<Encoded>, Error> {
    encode(map, &|name| {
        files
            .get(name)
            .cloned()
            .ok_or_else(|| std::io::Error::other(format!("no {name}")))
    })
}

fn diffs(rom: &[u8], map: &AssetMap, files: &Files) -> Vec<(u32, u8, u8)> {
    differences(rom, map, &pack(map, files).unwrap())
        .unwrap()
        .into_iter()
        .map(|d| (d.offset, d.rom, d.encoded))
        .collect()
}

fn edit(files: &mut Files, name: &str, from: &str, to: &str) {
    let text = String::from_utf8(files[name].clone()).unwrap();
    assert!(text.contains(from), "{from:?} not in {text}");
    files.insert(name.into(), text.replacen(from, to, 1).into_bytes());
}

fn pack_err(map: &AssetMap, files: &Files) -> String {
    pack(map, files).unwrap_err().to_string()
}

#[test]
fn ppu_strings_round_trip_and_edit() {
    let rom = rom(&ppu_bytes());
    let (map, files) = lift_files(&rom, &ppu_map(&rom, 23));
    let text = String::from_utf8(files["text.toml"].clone()).unwrap();
    for line in [
        "{ ppu = 0x2043, text = \"HI\" },",
        "{ ppu = 0x23C0, vertical = true, repeat = 5, tile = 0xAA },",
        "{ ppu = 0x2100, tiles = [0x24, 0x77] },",
        "{ ppu = 0x3F00, data = \"pal\" },",
        "# 1 update and no end byte",
    ] {
        assert!(text.contains(line), "{line} not in {text}");
    }
    // The palette's bytes are its own asset's; the strings own the rest.
    let spans: Vec<(u32, u32)> = map
        .asset("text")
        .unwrap()
        .spans()
        .iter()
        .map(|s| (s.offset, s.len))
        .collect();
    assert_eq!(spans, [(0x100, 0x12), (0x116, 1)]);
    assert_eq!(diffs(&rom, &map, &files), []);

    // Text in place of text of the same length changes those bytes only.
    let mut edited = files.clone();
    edit(&mut edited, "text.toml", "text = \"HI\"", "text = \"OK\"");
    assert_eq!(
        diffs(&rom, &map, &edited),
        [(0x103, 0x11, 0x18), (0x104, 0x12, 0x14)]
    );
}

#[test]
fn ppu_strings_refusals() {
    let rom = rom(&ppu_bytes());
    let (map, files) = lift_files(&rom, &ppu_map(&rom, 23));
    let cases = [
        // A longer string would move the palette after it.
        ("text = \"HI\"", "text = \"HIT\"", "put its data at"),
        // Strings that outgrow their span.
        (
            "{ ppu = 0x3F00, data = \"pal\" },",
            "{ ppu = 0x3F00, data = \"pal\" },\n  { ppu = 0x2000, tiles = [0x01] },",
            "exactly the 23",
        ),
        // A character outside the charset.
        (
            "text = \"HI\"",
            "text = \"H?\"",
            "'?' isn't in the map's charset",
        ),
        // An address starting with the end byte would end the string.
        ("ppu = 0x2043", "ppu = 0xFF43", "end byte"),
        // The open string has a set number of updates.
        (
            "{ ppu = 0x2100, tiles = [0x24, 0x77] },",
            "",
            "assets.toml says it has 1",
        ),
        // Data must be an embedded asset.
        (
            "data = \"pal\"",
            "data = \"text\"",
            "isn't one of the assets",
        ),
        ("repeat = 5, tile = 0xAA", "repeat = 5", "repeat with tile"),
        ("repeat = 5", "repeat = 64", "outside 0 to 63"),
        // The embed's bytes written as tiles would be dropped: the span
        // leaves them to the palette asset.
        (
            "data = \"pal\"",
            "tiles = [0x0F, 0x16, 0x30, 0x27]",
            "no update has data = \"pal\"",
        ),
    ];
    for (from, to, want) in cases {
        let mut bad = files.clone();
        edit(&mut bad, "text.toml", from, to);
        let err = pack_err(&map, &bad);
        assert!(err.contains(want), "{from} -> {to}: {err}");
    }
    // With end_anywhere, the end byte is refused in any byte.
    let anywhere = ppu_map(&rom, 23).replace("end = 0xFF", "end = 0xFF\nend_anywhere = true");
    let (amap, afiles) = lift_files(&rom, &anywhere);
    assert_eq!(diffs(&rom, &amap, &afiles), []);
    let mut bad = afiles.clone();
    edit(
        &mut bad,
        "text.toml",
        "tiles = [0x24, 0x77]",
        "tiles = [0x24, 0xFF]",
    );
    assert!(pack_err(&amap, &bad).contains("holds the end byte 0xff"));
    assert!(pack(&map, &bad).is_ok(), "without end_anywhere it packs");

    // Map checks: no updates, and spans past any ROM.
    for (from, to, want) in [
        ("records = 1", "records = 0", "records must be at least 1"),
        (
            "offset = 0x0100\nsize",
            "offset = 0xFFFFFF00\nsize",
            "more than any NES ROM",
        ),
    ] {
        let err = AssetMap::parse(&ppu_map(&rom, 23).replace(from, to))
            .unwrap_err()
            .to_string();
        assert!(err.contains(want), "{to}: {err}");
    }

    // A size that doesn't end where the strings do can't be lifted.
    let short = ppu_map(&rom, 24);
    let map = AssetMap::parse(&short).unwrap();
    let err = lift(&rom, &map, &short).unwrap_err().to_string();
    assert!(err.contains("is size right?"), "{err}");

    // An embed that isn't the named asset's span is refused with the map.
    let wrong = ppu_map(&rom, 23).replace("offset = 0x0112, size = 4", "offset = 0x0113, size = 3");
    let err = AssetMap::parse(&wrong).unwrap_err().to_string();
    assert!(err.contains("embed pal must name another asset"), "{err}");
}

fn rows_map(rom: &[u8]) -> String {
    format!(
        r#"
schema = 1
source = "test"
sha256 = "{}"

[[asset]]
name = "rows"
format = "byte-rows"
label = "Rows"
end_label = "After-1"
region = "prg"
offset = 0x0100
count = 3
width = 2
names = ["first", "second", "third"]
{CHARSET}
"#,
        RomLayout::of(rom).unwrap().sha256
    )
}

#[test]
fn byte_rows_round_trip_and_edit() {
    let rom = rom(&[0x11, 0x12, 0x99, 0x01, 0x28, 0x24]);
    let (map, files) = lift_files(&rom, &rows_map(&rom));
    let text = String::from_utf8(files["rows.toml"].clone()).unwrap();
    assert!(text.contains("  \"HI\", # 0: first\n"), "{text}");
    assert!(text.contains("  [0x99, 0x01], # 1: second\n"), "{text}");
    assert!(text.contains("  \"- \", # 2: third\n"), "{text}");
    assert_eq!(diffs(&rom, &map, &files), []);

    let mut edited = files.clone();
    edit(&mut edited, "rows.toml", "[0x99, 0x01]", "\"10\"");
    assert_eq!(
        diffs(&rom, &map, &edited),
        [(0x102, 0x99, 0x01), (0x103, 0x01, 0x00)]
    );

    for (from, to, want) in [
        ("\"HI\"", "\"HIT\"", "has 3 characters, not 2"),
        ("[0x99, 0x01]", "[0x99]", "has 1 numbers, not 2"),
        ("[0x99, 0x01]", "[0x99, 0x100]", "isn't a byte"),
    ] {
        let mut bad = files.clone();
        edit(&mut bad, "rows.toml", from, to);
        let err = pack_err(&map, &bad);
        assert!(err.contains(want), "{to}: {err}");
    }
    // A row of more than 16 numbers is written 16 to a line.
    let wide = self::rom(&(0u8..20).collect::<Vec<_>>());
    let wide_map = rows_map(&wide)
        .replace("count = 3\nwidth = 2", "count = 1\nwidth = 20")
        .replace("names = [\"first\", \"second\", \"third\"]", "")
        .replace(CHARSET, "");
    let (wmap, wfiles) = lift_files(&wide, &wide_map);
    let text = String::from_utf8(wfiles["rows.toml"].clone()).unwrap();
    assert!(text.contains("  [ # 0\n    0x00, 0x01,"), "{text}");
    assert!(
        text.contains("0x0F,\n    0x10, 0x11, 0x12, 0x13,\n  ],\n"),
        "{text}"
    );
    assert_eq!(diffs(&wide, &wmap, &wfiles), []);
    // Without a charset, text can't be packed.
    let plain = rows_map(&rom).replace(CHARSET, "");
    let map = AssetMap::parse(&plain).unwrap();
    let err = pack_err(&map, &files);
    assert!(err.contains("gives no charset"), "{err}");
    // Names must match the rows.
    let bad = rows_map(&rom).replace("\"third\"", "");
    assert!(
        AssetMap::parse(&bad)
            .unwrap_err()
            .to_string()
            .contains("2 names for 3 rows")
    );
    let bad = rows_map(&rom).replace("After-1", "After-");
    assert!(
        AssetMap::parse(&bad)
            .unwrap_err()
            .to_string()
            .contains("end_label")
    );
}

#[test]
fn labels_of_every_format_are_checked() {
    let image = rom(&[]);
    let base = |extra: &str| {
        format!(
            r#"
schema = 1
source = "test"
sha256 = "{}"

[[asset]]
name = "pal"
format = "palette"
region = "prg"
offset = 0x0100
count = 1
{extra}
"#,
            RomLayout::of(&image).unwrap().sha256
        )
    };
    assert!(AssetMap::parse(&base("label = \"Pal+3\"\nend_label = \"Next-1\"")).is_ok());
    for extra in ["label = \"bad label\"", "end_label = \"Next-\""] {
        let err = AssetMap::parse(&base(extra)).unwrap_err().to_string();
        assert!(err.contains("label"), "{extra}: {err}");
    }
}

/// Made-up blobs at PRG 0x10A (CPU 0x810A) and three pointer tables into
/// them: split, words, and one-byte offsets from 0x8100.
fn blob_bytes() -> Vec<u8> {
    let mut b = vec![0u8; 0x17];
    // At 0x100: the split table, low then high bytes: blob 0, blob 2, a
    // pointer outside the blobs.
    b[0x00..0x06].copy_from_slice(&[0x0A, 0x14, 0xFF, 0x81, 0x81, 0xFF]);
    // At 0x106: words: blob 1, blob 0.
    b[0x06..0x0A].copy_from_slice(&[0x10, 0x81, 0x0A, 0x81]);
    // Blob 0: a head of two, then two rows of two.
    b[0x0A..0x10].copy_from_slice(&[0x02, 0x00, 0x11, 0x12, 0x13, 0x14]);
    // Blob 1: a head and one row.
    b[0x10..0x14].copy_from_slice(&[0x01, 0x05, 0x21, 0x22]);
    // Blob 2, and the fill after it.
    b[0x14..0x17].copy_from_slice(&[0x99, 0xFF, 0xFF]);
    b
}

fn blob_map(rom: &[u8], extra: &str) -> String {
    format!(
        r#"
schema = 1
source = "test"
sha256 = "{}"

[[asset]]
name = "frames"
format = "blobs"
label = "Frames"
end_label = "After"
region = "prg"
offset = 0x010A
size = 13
cpu_base = 0x8000
head = 2
width = 2
fill = 0xFF
notes = ["standing"]
{extra}
table = [
  {{ name = "split", offset = 0x0100, hi_offset = 0x0103, count = 3, label = "Lo", hi_label = "Hi" }},
  {{ name = "words", offset = 0x0106, count = 2 }},
  {{ name = "near", offset = 0x00F0, count = 2, base = 0x8100 }},
]
"#,
        RomLayout::of(rom).unwrap().sha256
    )
}

fn blob_rom() -> Vec<u8> {
    let mut image = rom(&blob_bytes());
    // The offsets table at PRG 0xF0: blob 0 and blob 1, from 0x8100.
    image[16 + 0xF0] = 0x0A;
    image[16 + 0xF1] = 0x10;
    image
}

#[test]
fn blobs_round_trip_and_move() {
    let image = blob_rom();
    let (map, files) = lift_files(&image, &blob_map(&image, ""));
    let text = String::from_utf8(files["frames.toml"].clone()).unwrap();
    for line in [
        "split = [\n  0, 2, \"0xFFFF\", # 0x00\n]",
        "words = [\n  1, 0, # 0x00\n]",
        "near = [\n  0, 1, # 0x00\n]",
        "# 0: standing (split 0x00, words 0x01, near 0x00), 6 bytes at 0x810A",
        "head = [0x02, 0x00]\nrows = [\n  [0x11, 0x12],\n  [0x13, 0x14],\n]",
        "# 2 (split 0x01), 3 bytes at 0x8114\n[[blob]]\nhead = [0x99, 0xFF]\nbytes = [0xFF]",
    ] {
        assert!(text.contains(line), "{line:?} not in {text}");
    }
    assert_eq!(diffs(&image, &map, &files), []);

    // A row less in blob 0 moves blobs 1 and 2 down two bytes, rewrites
    // their pointers, and fills the end.
    let mut edited = files.clone();
    edit(&mut edited, "frames.toml", "  [0x13, 0x14],\n", "");
    let mut moved = diffs(&image, &map, &edited);
    moved.sort_unstable();
    assert_eq!(
        moved,
        [
            (0xF1, 0x10, 0x0E),
            (0x101, 0x14, 0x12),
            (0x106, 0x10, 0x0E),
            (0x10E, 0x13, 0x01),
            (0x10F, 0x14, 0x05),
            (0x110, 0x01, 0x21),
            (0x111, 0x05, 0x22),
            (0x112, 0x21, 0x99),
            (0x113, 0x22, 0xFF),
            (0x114, 0x99, 0xFF),
        ]
    );

    for (from, to, want) in [
        (
            "  0, 2, \"0xFFFF\"",
            "  0, 3, \"0xFFFF\"",
            "there's no blob 3",
        ),
        ("\"0xFFFF\"", "\"0x8110\"", "points into the blobs"),
        ("\"0xFFFF\"", "\"FFFF\"", "isn't an address"),
        (
            "  1, 0, # 0x00",
            "  1, # 0x00",
            "1 pointers, but assets.toml says 2",
        ),
        (
            "bytes = [0xFF]",
            "bytes = [0xFF, 0, 0]",
            "take more than the 13 bytes",
        ),
        ("bytes = [0xFF]", "rows = [[0xFF]]", "has 1 numbers, not 2"),
        (
            "bytes = [0xFF]",
            "address = 0x8114",
            "only for blobs that can't move",
        ),
    ] {
        let mut bad = files.clone();
        edit(&mut bad, "frames.toml", from, to);
        let err = pack_err(&map, &bad);
        assert!(err.contains(want), "{to}: {err}");
    }
}

#[test]
fn fixed_blobs_keep_their_addresses() {
    let image = blob_rom();
    let (map, files) = lift_files(&image, &blob_map(&image, "fixed = true"));
    let text = String::from_utf8(files["frames.toml"].clone()).unwrap();
    assert!(text.contains("[[blob]]\naddress = 0x810A\nhead"), "{text}");
    assert_eq!(diffs(&image, &map, &files), []);
    // A blob's bytes may change, but not its length.
    let mut edited = files.clone();
    edit(&mut edited, "frames.toml", "[0x13, 0x14]", "[0x13, 0x15]");
    assert_eq!(diffs(&image, &map, &edited), [(0x10F, 0x14, 0x15)]);
    let mut moved = files.clone();
    edit(&mut moved, "frames.toml", "  [0x13, 0x14],\n", "");
    let err = pack_err(&map, &moved);
    assert!(err.contains("none may change length"), "{err}");
}

#[test]
fn blob_maps_are_checked() {
    let image = blob_rom();
    for (from, to, want) in [
        ("size = 13", "size = 0", "size must be at least 1"),
        ("region = \"prg\"", "region = \"chr\"", "read from prg"),
        ("cpu_base = 0x8000", "cpu_base = 0xFFF8", "64KB"),
        ("name = \"words\"", "name = \"split\"", "not repeat"),
        ("count = 2 }", "count = 0 }", "count must be at least 1"),
        (
            "count = 2 }",
            "count = 4294967295 }",
            "no bigger than any NES ROM",
        ),
        ("base = 0x8100", "base = 0xFF80", "base + 255"),
        (
            "base = 0x8100",
            "base = 0x8100, hi_offset = 0x0108",
            "not both",
        ),
        ("label = \"Lo\"", "label = \"Lo-1\"", "label"),
    ] {
        let text = blob_map(&image, "").replacen(from, to, 1);
        let err = AssetMap::parse(&text).unwrap_err().to_string();
        assert!(err.contains(want), "{to}: {err}");
    }
}

/// A pointer table whose bank maps `window` bytes refuses a target that
/// has moved past them.
#[test]
fn a_pointer_past_its_window_is_refused() {
    let image = rom(&[0x1A, 0xC0, 0x20, 0x21]);
    let map_text = |window: &str| {
        format!(
            r#"
schema = 1
source = "test"
sha256 = "{}"

[[asset]]
name = "ptrs"
format = "pointers"
region = "prg"
offset = 0x0100
count = 1
cpu_base = 0xC000
prg_base = 0x0100
{window}

[[asset]]
name = "rows"
format = "byte-rows"
region = "prg"
offset = 0x011A
count = 1
width = 1
"#,
            RomLayout::of(&image).unwrap().sha256
        )
    };
    let (map, files) = lift_files(&image, &map_text("window = 0x20"));
    assert_eq!(diffs(&image, &map, &files), []);
    let narrow = AssetMap::parse(&map_text("window = 0x10")).unwrap();
    let err = pack_err(&narrow, &files);
    assert!(err.contains("past the 0x10 bytes the bank maps"), "{err}");
    // Lifting with that window names it too.
    let err = lift(&image, &narrow, &map_text("window = 0x10"))
        .unwrap_err()
        .to_string();
    assert!(err.contains("past the 0x10 bytes the bank maps"), "{err}");
    let err = AssetMap::parse(&map_text("window = 0"))
        .unwrap_err()
        .to_string();
    assert!(err.contains("window"), "{err}");
}
