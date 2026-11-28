//! Made-up sets for tests: no game's bytes, only the programs a test
//! writes. They run on the MCR board like any set, so the tools that read
//! an arcade run can be tested without a romset on the machine.

use crate::Romset;

/// A set whose main and sound programs are `main` and `ssio`, from address
/// 0, with zeros after them and blank graphics.
pub fn romset(main: &[u8], ssio: &[u8]) -> Romset {
    let mut maincpu = vec![0u8; 0x10000];
    maincpu[..main.len()].copy_from_slice(main);
    let mut ssiocpu = vec![0u8; 0x10000];
    ssiocpu[..ssio.len()].copy_from_slice(ssio);
    Romset {
        name: "test",
        identity: "0".repeat(64),
        maincpu,
        ssiocpu,
        gfx1: vec![0; 0x8000],
        gfx2: vec![0; 0x20000],
        extras: Vec::new(),
        ssio_prom: None,
    }
}

/// A made-up sound program: turns on AY 0's tone A and both chips'
/// ports (so the SSIO's duty volumes are the loudest and it isn't
/// muted), then writes the latch-0 byte's low half to tone A's volume
/// over and over.
pub const SOUND: [u8; 46] = [
    0x3E, 0x07, 0x32, 0x00, 0xA0, 0x3E, 0x7E, 0x32, 0x02, 0xA0, // AY 0 reg 7 = 7e
    0x3E, 0x00, 0x32, 0x00, 0xA0, 0x3E, 0x80, 0x32, 0x02, 0xA0, // AY 0 reg 0 = 80
    0x3E, 0x07, 0x32, 0x00, 0xB0, 0x3E, 0xBF, 0x32, 0x02, 0xB0, // AY 1 reg 7 = bf
    0x3E, 0x08, 0x32, 0x00, 0xA0, // loop: AY 0 reg 8 =
    0x3A, 0x00, 0x90, 0xE6, 0x0F, 0x32, 0x02, 0xA0, // latch 0 & 0f
    0xC3, 0x1E, 0x00, // jp loop
];

/// A made-up main program: counts in A, writing it to latch 0 and
/// feeding the watchdog, with a wait between.
pub const MAIN: [u8; 12] = [
    0x3C, // inc a
    0xD3, 0x1C, // out (1c),a
    0xD3, 0xE0, // out (e0),a
    0x06, 0xFF, // ld b,ff
    0x10, 0xFE, // djnz $
    0x18, 0xF5, // jr 0
    0x00,
];

/// A set that sounds: [`MAIN`] counts into latch 0 and [`SOUND`] plays it
/// as AY 0's tone A's volume, with a made-up PROM of alternate bits for
/// the SSIO's duty volumes.
pub fn sounding() -> Romset {
    let mut rom = romset(&MAIN, &SOUND);
    rom.ssio_prom = Some(vec![0xAA; 32]);
    rom
}

/// Both programs loop at 0 (`JR $`).
pub fn looping() -> Romset {
    romset(&[0x18, 0xfe], &[0x18, 0xfe])
}

/// A main program that does a little of everything a tool reads, every
/// frame, with no interrupts: it reads input port 1 into work RAM at
/// `e000`, counts frames at `e001`, steps a shift register at `e002` (a
/// made-up random generator: `ld a,(e002) / rlca / xor 1Ch` when the
/// carry is set; an even constant keeps it from ever reaching zero),
/// writes the sound latch and feeds the watchdog, then
/// jumps through a table with `jp (hl)`. The sound program writes AY-8910
/// register 7 with whatever is in latch 0.
pub fn busy() -> Romset {
    let main: &[u8] = &[
        // 0000: ld sp,e7f0
        0x31, 0xf0, 0xe7, //
        // 0003: in a,(01) ; ld (e000),a
        0xdb, 0x01, 0x32, 0x00, 0xe0, //
        // 0008: ld hl,e001 ; inc (hl)
        0x21, 0x01, 0xe0, 0x34, //
        // 000c: ld a,(e002) ; rlca ; jr nc,+2 ; xor 1c ; ld (e002),a
        0x3a, 0x02, 0xe0, 0x07, 0x30, 0x02, 0xee, 0x1c, 0x32, 0x02, 0xe0, //
        // 0017: out (1c),a ; out (e0),a
        0xd3, 0x1c, 0xd3, 0xe0, //
        // 001b: and 1 ; add a,a ; ld e,a ; ld d,0 ; ld hl,0040 ; add hl,de
        0xe6, 0x01, 0x87, 0x5f, 0x16, 0x00, 0x21, 0x40, 0x00, 0x19, //
        // 0025: ld a,(hl) ; inc hl ; ld h,(hl) ; ld l,a ; jp (hl)
        0x7e, 0x23, 0x66, 0x6f, 0xe9, //
        // 002a..003f: nops, then the table at 0040: 0050, 0058
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, //
        0x50, 0x00, 0x58, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, //
        // 0050: ld a,1 ; ld (e003),a ; jr 0003 (back to the loop)
        0x3e, 0x01, 0x32, 0x03, 0xe0, 0x18, 0xac, 0, //
        // 0058: ld a,2 ; ld (e003),a ; jr 0003
        0x3e, 0x02, 0x32, 0x03, 0xe0, 0x18, 0xa4,
    ];
    let ssio: &[u8] = &[
        // 0000: ld a,07 ; ld (a000),a  (select register 7 of the first AY)
        0x3e, 0x07, 0x32, 0x00, 0xa0, //
        // 0005: ld a,(9000) ; ld (a002),a ; jr 0005
        0x3a, 0x00, 0x90, 0x32, 0x02, 0xa0, 0x18, 0xf8,
    ];
    romset(main, ssio)
}

/// A main program that draws one random number and stops: it calls a
/// made-up random routine at 0010 (`ld a,(e002) / rlca / ld (e002),a /
/// ret`), stores the number it returns at `e000` and loops on itself. The
/// call is at 0003, so the number lands in A at 0006. Work RAM starts at
/// 0xff, so the routine returns 0xff.
pub fn drawing() -> Romset {
    let main: &[u8] = &[
        // 0000: ld sp,e7f0 ; call 0010
        0x31, 0xf0, 0xe7, 0xcd, 0x10, 0x00, //
        // 0006: ld (e000),a ; jr $
        0x32, 0x00, 0xe0, 0x18, 0xfe, //
        // 000b..000f: nops
        0, 0, 0, 0, 0, //
        // 0010: ld a,(e002) ; rlca ; ld (e002),a ; ret
        0x3a, 0x02, 0xe0, 0x07, 0x32, 0x02, 0xe0, 0xc9,
    ];
    romset(main, &[0x18, 0xfe])
}

/// A main program for the static harvest, as code at each address. The
/// reset sets up IM 2 with I = 0 and the CTC vector $80; channel 0's
/// handler counts frames in E000; the main loop waits for it, walks 4
/// records of 8 bytes at E100 with IX (and their 32-byte partners at E200
/// with IY), calling a per-record routine that jumps through a table by
/// the record's state (IY+0, doubled).
pub const RECORDS: &[(u16, &[u8])] = &[
    // 0000: ld sp,$e800; di; ld a,$00; ld i,a; ld a,$80; out ($f0),a;
    //       im 2; ei
    (
        0x0000,
        &[
            0x31, 0x00, 0xe8, 0xf3, 0x3e, 0x00, 0xed, 0x47, 0x3e, 0x80, 0xd3, 0xf0, 0xed, 0x5e,
            0xfb,
        ],
    ),
    // 000F: call $0200 (wait); call $0300 (walk); jr $000F
    (0x000f, &[0xcd, 0x00, 0x02, 0xcd, 0x00, 0x03, 0x18, 0xf8]),
    // 0080 vector table: channel 0 -> $0100, channel 1 -> $0120
    (0x0080, &[0x00, 0x01, 0x20, 0x01]),
    // 0100: push af; ld a,($e000); inc a; ld ($e000),a; pop af; ei; reti
    (
        0x0100,
        &[
            0xf5, 0x3a, 0x00, 0xe0, 0x3c, 0x32, 0x00, 0xe0, 0xf1, 0xfb, 0xed, 0x4d,
        ],
    ),
    // 0120: ei; reti
    (0x0120, &[0xfb, 0xed, 0x4d]),
    // 0200: ld a,($e000); or a; jr z,$0200; xor a; ld ($e000),a; ret
    (
        0x0200,
        &[
            0x3a, 0x00, 0xe0, 0xb7, 0x28, 0xfa, 0xaf, 0x32, 0x00, 0xe0, 0xc9,
        ],
    ),
    // 0300: ld ix,$e100; ld iy,$e200; ld b,$04
    //       030A: call $0400; ld de,$0008; add ix,de; ld de,$0020;
    //       add iy,de; djnz $0309; ret
    (
        0x0300,
        &[
            0xdd, 0x21, 0x00, 0xe1, 0xfd, 0x21, 0x00, 0xe2, 0x06, 0x04, 0xcd, 0x00, 0x04, 0x11,
            0x08, 0x00, 0xdd, 0x19, 0x11, 0x20, 0x00, 0xfd, 0x19, 0x10, 0xf1, 0xc9,
        ],
    ),
    // 0400: push bc; ld hl,$0480; ld a,(iy+$00); rlca; call $0500;
    //       ld a,(hl); inc hl; ld h,(hl); ld l,a; jp (hl)
    (
        0x0400,
        &[
            0xc5, 0x21, 0x80, 0x04, 0xfd, 0x7e, 0x00, 0x07, 0xcd, 0x00, 0x05, 0x7e, 0x23, 0x66,
            0x6f, 0xe9,
        ],
    ),
    // 0480: the table: state 0 -> $0600, 1 -> $0620, 2 -> $0640
    (0x0480, &[0x00, 0x06, 0x20, 0x06, 0x40, 0x06]),
    // 0500: add a,l; ld l,a; ret nc; inc h; ret (HL += A)
    (0x0500, &[0x85, 0x6f, 0xd0, 0x24, 0xc9]),
    // 0600: state 0: dec (iy+$04); jr nz,$060D; ld (iy+$00),$01;
    //       ld (iy+$04),$20; 060D: pop bc; ret
    (
        0x0600,
        &[
            0xfd, 0x35, 0x04, 0x20, 0x08, 0xfd, 0x36, 0x00, 0x01, 0xfd, 0x36, 0x04, 0x20,
        ],
    ),
    (0x060d, &[0xc1, 0xc9]),
    // 0620: state 1: ld a,(ix+$03); add a,(iy+$06); ld (ix+$03),a;
    //       cp $80; jr c,$060D; ld (iy+$00),$02; jr $060D
    (
        0x0620,
        &[
            0xdd, 0x7e, 0x03, 0xfd, 0x86, 0x06, 0xdd, 0x77, 0x03, 0xfe, 0x80, 0x38, 0xe0, 0xfd,
            0x36, 0x00, 0x02, 0x18, 0xda,
        ],
    ),
    // 0640: state 2: ld a,$03; call $0700; ld (iy+$02),a;
    //       ld (iy+$00),$00; jr $060D
    (
        0x0640,
        &[
            0x3e, 0x03, 0xcd, 0x00, 0x07, 0xfd, 0x77, 0x02, 0xfd, 0x36, 0x00, 0x00, 0x18, 0xbf,
        ],
    ),
    // 0700: random below A: push bc; ld b,a; call $0710; cp b;
    //       jr c,$070E; sub b; jr $0705; 070E: pop bc; ret
    (
        0x0700,
        &[
            0xc5, 0x47, 0xcd, 0x10, 0x07, 0xb8, 0x38, 0x06, 0x90, 0x18, 0xfa, 0x00, 0x00, 0x00,
            0xc1, 0xc9,
        ],
    ),
    // 0710: random: ld a,($e010); rlca; jr nc,$0717; xor $2b;
    //       0718: ld c,a; ld a,r; xor c; ld ($e010),a; ret
    (
        0x0710,
        &[
            0x3a, 0x10, 0xe0, 0x07, 0x30, 0x02, 0xee, 0x2b, 0x4f, 0xed, 0x5f, 0xa9, 0x32, 0x10,
            0xe0, 0xc9,
        ],
    ),
];

/// [`RECORDS`] as a set's main program, with a sound program that loops.
pub fn records() -> Romset {
    let mut main = vec![0xffu8; 0x0800];
    for (at, bytes) in RECORDS {
        let o = usize::from(*at);
        main[o..o + bytes.len()].copy_from_slice(bytes);
    }
    romset(&main, &[0x18, 0xfe])
}

/// A made-up set named `system`, as a game's roms.toml would list it: three
/// chips of made-up bytes (a program chip, a tile chip and a sprite chip,
/// filled with a byte from the name), and those chips as a zip's files would hold them.
pub fn made_up_set(system: &str) -> (String, Vec<(String, Vec<u8>)>) {
    let fill = name_byte(system);
    set_of(
        system,
        vec![
            ("1a", "maincpu", vec![fill; 0x1000]),
            ("bg", "gfx1", vec![fill.wrapping_add(1); 0x2000]),
            ("fg", "gfx2", vec![fill.wrapping_add(2); 0x2000]),
        ],
    )
}

/// A byte made from a set's name, so two made-up sets differ.
fn name_byte(system: &str) -> u8 {
    system
        .bytes()
        .fold(7u8, |a, b| a.wrapping_mul(31).wrapping_add(b))
}

/// A roms.toml listing `chips` (each a file `system.SUFFIX`, its region, its
/// bytes at offset 0) as the arcade set `system`, and the chips' files.
fn set_of(system: &str, chips: Vec<(&str, &str, Vec<u8>)>) -> (String, Vec<(String, Vec<u8>)>) {
    let hex = |d: &[u8]| -> String {
        crate::sha1::sha1(d)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    };
    let sums: Vec<String> = chips.iter().map(|(_, _, d)| hex(d)).collect();
    let id = crate::romset::identity(sums.iter().map(String::as_str));
    let mut toml = format!(
        "[[source]]\nid = \"{system}\"\ntitle = \"Made Up\"\nplatform = \"arcade\"\nregion = \"us\"\n\
         revision = \"Made Up ({system})\"\nsystem = \"{system}\"\nsha256 = \"{id}\"\nstatus = \"unverified\"\n"
    );
    for ((suffix, region, d), sha1) in chips.iter().zip(&sums) {
        toml.push_str(&format!(
            "\n[[source.chip]]\nname = \"{system}.{suffix}\"\nregion = \"{region}\"\noffset = 0x0000\nsize = {:#06x}\nsha1 = \"{sha1}\"\n",
            d.len()
        ));
    }
    let files = chips
        .into_iter()
        .map(|(suffix, _, d)| (format!("{system}.{suffix}"), d))
        .collect();
    (toml, files)
}

/// [`made_up_set`], added to the sets known ([`crate::romset::known`]):
/// its files, which [`Romset::identify`] then knows as `system`.
pub fn known_made_up_set(system: &str) -> Vec<(String, Vec<u8>)> {
    let (toml, files) = made_up_set(system);
    crate::romset::add_roms_toml(&toml).expect("a made-up set's roms.toml reads");
    files
}

/// A made-up game's main program, as code at each address: a little game
/// loop that does what a tool reads of a game's run, with no game's bytes.
///
/// The reset routine (0000) sets the stack, clears work RAM E000-E2FF
/// (0100), sets up the CTC (0120: vector $80 with I = 0, channel 3 counting
/// the board's once-a-frame trigger, so its interrupt comes each frame
/// through the word at 0086) and IM 2, then loops (000D): wait for the
/// frame (0140), read input ports 0 and 1 into E010 and E011 (0160), and
/// run the state E002 through a table (0180, `jp (hl)`).
///
/// The interrupt handler (0200) sets the frame flag E001, counts frames in
/// E003-E004 (0325), feeds the watchdog (0240) and, every 8th frame, writes
/// the frame count's next bits to sound latch 0 (0260).
///
/// State 0, attract (0300), calls ten object routines (0400, then 0600,
/// 0610 .. 0690, each counting in E100 + its number); a coin, or the
/// frame count reaching 100, moves to state 1. State 1, play (0340),
/// moves the player at E020 while port 1's bit 0 is held (0420) and walks
/// four enemies of 8 bytes at E200 with IX (0440, 0460), each moving right
/// and, at $80, wrapping with a new number from the random routine (0500).
pub const GAME_MAIN: &[(u16, &[u8])] = &[
    // 0000: ld sp,$e800; di; call $0100; call $0120; im 2; ei
    (
        0x0000,
        &[
            0x31, 0x00, 0xe8, 0xf3, 0xcd, 0x00, 0x01, 0xcd, 0x20, 0x01, 0xed, 0x5e, 0xfb,
        ],
    ),
    // 000D: call $0140; call $0160; call $0180; jr $000D
    (
        0x000d,
        &[
            0xcd, 0x40, 0x01, 0xcd, 0x60, 0x01, 0xcd, 0x80, 0x01, 0x18, 0xf5,
        ],
    ),
    // 0086: CTC channel 3's vector -> $0200
    (0x0086, &[0x00, 0x02]),
    // 0100: ld hl,$e000; ld c,3; 0105: ld b,0; 0107: ld (hl),0; inc hl;
    //       djnz $0107; dec c; jr nz,$0105; ret
    (
        0x0100,
        &[
            0x21, 0x00, 0xe0, 0x0e, 0x03, 0x06, 0x00, 0x36, 0x00, 0x23, 0x10, 0xfb, 0x0d, 0x20,
            0xf6, 0xc9,
        ],
    ),
    // 0120: ld a,0; ld i,a; ld a,$80; out ($f0),a; ld a,$c5; out ($f3),a;
    //       ld a,1; out ($f3),a; ret
    (
        0x0120,
        &[
            0x3e, 0x00, 0xed, 0x47, 0x3e, 0x80, 0xd3, 0xf0, 0x3e, 0xc5, 0xd3, 0xf3, 0x3e, 0x01,
            0xd3, 0xf3, 0xc9,
        ],
    ),
    // 0140: ld a,($e001); or a; jr z,$0140; xor a; ld ($e001),a; ret
    (
        0x0140,
        &[
            0x3a, 0x01, 0xe0, 0xb7, 0x28, 0xfa, 0xaf, 0x32, 0x01, 0xe0, 0xc9,
        ],
    ),
    // 0160: in a,($00); ld ($e010),a; in a,($01); ld ($e011),a; ret
    (
        0x0160,
        &[
            0xdb, 0x00, 0x32, 0x10, 0xe0, 0xdb, 0x01, 0x32, 0x11, 0xe0, 0xc9,
        ],
    ),
    // 0180: ld a,($e002); add a,a; ld e,a; ld d,0; ld hl,$01a0; add hl,de;
    //       ld a,(hl); inc hl; ld h,(hl); ld l,a; jp (hl)
    (
        0x0180,
        &[
            0x3a, 0x02, 0xe0, 0x87, 0x5f, 0x16, 0x00, 0x21, 0xa0, 0x01, 0x19, 0x7e, 0x23, 0x66,
            0x6f, 0xe9,
        ],
    ),
    // 01A0: the states: 0 -> $0300, 1 -> $0340
    (0x01a0, &[0x00, 0x03, 0x40, 0x03]),
    // 0200: push af/bc/de/hl; ld a,1; ld ($e001),a; call $0325;
    //       call $0240; call $0260; pop hl/de/bc/af; ei; reti
    (
        0x0200,
        &[
            0xf5, 0xc5, 0xd5, 0xe5, 0x3e, 0x01, 0x32, 0x01, 0xe0, 0xcd, 0x25, 0x03, 0xcd, 0x40,
            0x02, 0xcd, 0x60, 0x02, 0xe1, 0xd1, 0xc1, 0xf1, 0xfb, 0xed, 0x4d,
        ],
    ),
    // 0240: out ($e0),a; ret
    (0x0240, &[0xd3, 0xe0, 0xc9]),
    // 0260: ld a,($e003); and 7; ret nz; ld a,($e003); rrca x3; and $0f;
    //       out ($1c),a; ret
    (
        0x0260,
        &[
            0x3a, 0x03, 0xe0, 0xe6, 0x07, 0xc0, 0x3a, 0x03, 0xe0, 0x0f, 0x0f, 0x0f, 0xe6, 0x0f,
            0xd3, 0x1c, 0xc9,
        ],
    ),
    // 0300: call $0400; ld a,($e010); and 1; jr z,$0311; ld a,($e003);
    //       cp 100; ret c; nop; 0311: ld a,1; ld ($e002),a; ret
    (
        0x0300,
        &[
            0xcd, 0x00, 0x04, 0x3a, 0x10, 0xe0, 0xe6, 0x01, 0x28, 0x07, 0x3a, 0x03, 0xe0, 0xfe,
            0x64, 0xd8, 0x00, 0x3e, 0x01, 0x32, 0x02, 0xe0, 0xc9,
        ],
    ),
    // 0325: ld hl,($e003); inc hl; ld ($e003),hl; ret
    (0x0325, &[0x2a, 0x03, 0xe0, 0x23, 0x22, 0x03, 0xe0, 0xc9]),
    // 0340: call $0420; call $0440; ret
    (0x0340, &[0xcd, 0x20, 0x04, 0xcd, 0x40, 0x04, 0xc9]),
    // 0400: call $0600; call $0610; .. call $0690; ret
    (
        0x0400,
        &[
            0xcd, 0x00, 0x06, 0xcd, 0x10, 0x06, 0xcd, 0x20, 0x06, 0xcd, 0x30, 0x06, 0xcd, 0x40,
            0x06, 0xcd, 0x50, 0x06, 0xcd, 0x60, 0x06, 0xcd, 0x70, 0x06, 0xcd, 0x80, 0x06, 0xcd,
            0x90, 0x06, 0xc9,
        ],
    ),
    // 0420: ld a,($e011); and 1; jr nz,$042B; ld hl,$e020; inc (hl); ret
    (
        0x0420,
        &[
            0x3a, 0x11, 0xe0, 0xe6, 0x01, 0x20, 0x04, 0x21, 0x20, 0xe0, 0x34, 0xc9,
        ],
    ),
    // 0440: ld ix,$e200; ld b,4; 0446: call $0460; ld de,8; add ix,de;
    //       djnz $0446; ret
    (
        0x0440,
        &[
            0xdd, 0x21, 0x00, 0xe2, 0x06, 0x04, 0xcd, 0x60, 0x04, 0x11, 0x08, 0x00, 0xdd, 0x19,
            0x10, 0xf6, 0xc9,
        ],
    ),
    // 0460: inc (ix+1); ld a,(ix+1); cp $80; ret c; ld (ix+1),0;
    //       call $0500; ld (ix+2),a; ret
    (
        0x0460,
        &[
            0xdd, 0x34, 0x01, 0xdd, 0x7e, 0x01, 0xfe, 0x80, 0xd8, 0xdd, 0x36, 0x01, 0x00, 0xcd,
            0x00, 0x05, 0xdd, 0x77, 0x02, 0xc9,
        ],
    ),
    // 0500: ld a,($e030); rlca; jr nc,$0508; xor $1d; 0508: inc a;
    //       ld ($e030),a; ret
    (
        0x0500,
        &[
            0x3a, 0x30, 0xe0, 0x07, 0x30, 0x02, 0xee, 0x1d, 0x3c, 0x32, 0x30, 0xe0, 0xc9,
        ],
    ),
    // 0600 + 16k, k = 0..9: ld hl,$e100+k; inc (hl); ret
    (0x0600, &[0x21, 0x00, 0xe1, 0x34, 0xc9]),
    (0x0610, &[0x21, 0x01, 0xe1, 0x34, 0xc9]),
    (0x0620, &[0x21, 0x02, 0xe1, 0x34, 0xc9]),
    (0x0630, &[0x21, 0x03, 0xe1, 0x34, 0xc9]),
    (0x0640, &[0x21, 0x04, 0xe1, 0x34, 0xc9]),
    (0x0650, &[0x21, 0x05, 0xe1, 0x34, 0xc9]),
    (0x0660, &[0x21, 0x06, 0xe1, 0x34, 0xc9]),
    (0x0670, &[0x21, 0x07, 0xe1, 0x34, 0xc9]),
    (0x0680, &[0x21, 0x08, 0xe1, 0x34, 0xc9]),
    (0x0690, &[0x21, 0x09, 0xe1, 0x34, 0xc9]),
];

/// The made-up game's sound program, as code at each address: the reset
/// (0000) sets the stack and sets up the AYs (0050: AY 0's tone A and both
/// chips' ports, as [`SOUND`] does), then waits in IM 1. Its interrupt
/// (0038, the SSIO's timer) clears the timer, reads latch 0 into sound RAM
/// 8000 (0070) and writes its low half to AY 0's tone A volume (0090), so
/// a latch byte is heard at the interrupt after it is written.
pub const GAME_SOUND: &[(u16, &[u8])] = &[
    // 0000: ld sp,$8400; call $0050; im 1; ei; 0009: jr $0009
    (
        0x0000,
        &[
            0x31, 0x00, 0x84, 0xcd, 0x50, 0x00, 0xed, 0x56, 0xfb, 0x18, 0xfe,
        ],
    ),
    // 0038: push af; ld a,($e000); call $0070; call $0090; pop af; ei; ret
    (
        0x0038,
        &[
            0xf5, 0x3a, 0x00, 0xe0, 0xcd, 0x70, 0x00, 0xcd, 0x90, 0x00, 0xf1, 0xfb, 0xc9,
        ],
    ),
    // 0050: AY 0 reg 7 = 7e, reg 0 = 80; AY 1 reg 7 = bf; ret
    (
        0x0050,
        &[
            0x3e, 0x07, 0x32, 0x00, 0xa0, 0x3e, 0x7e, 0x32, 0x02, 0xa0, 0x3e, 0x00, 0x32, 0x00,
            0xa0, 0x3e, 0x80, 0x32, 0x02, 0xa0, 0x3e, 0x07, 0x32, 0x00, 0xb0, 0x3e, 0xbf, 0x32,
            0x02, 0xb0, 0xc9,
        ],
    ),
    // 0070: ld a,($9000); ld ($8000),a; ret
    (0x0070, &[0x3a, 0x00, 0x90, 0x32, 0x00, 0x80, 0xc9]),
    // 0090: ld a,8; ld ($a000),a; ld a,($8000); and $0f; ld ($a002),a; ret
    (
        0x0090,
        &[
            0x3e, 0x08, 0x32, 0x00, 0xa0, 0x3a, 0x00, 0x80, 0xe6, 0x0f, 0x32, 0x02, 0xa0, 0xc9,
        ],
    ),
];

/// `code` laid out in a chip of `size` bytes, the rest 0xff.
fn chip_of(code: &[(u16, &[u8])], size: usize) -> Vec<u8> {
    let mut out = vec![0xffu8; size];
    for (at, bytes) in code {
        let o = usize::from(*at);
        out[o..o + bytes.len()].copy_from_slice(bytes);
    }
    out
}

/// A made-up game named `system`, as a game's roms.toml would list it: a
/// main program chip of [`GAME_MAIN`], a sound program chip of
/// [`GAME_SOUND`], and a tile chip and a sprite chip of made-up bytes; and
/// those chips as a zip's files would hold them.
pub fn made_up_game(system: &str) -> (String, Vec<(String, Vec<u8>)>) {
    let fill = name_byte(system);
    // Made-up pictures: a byte that changes along the chip, so no run of
    // one byte (as a text file's spaces would be) is in it, after the
    // set's name, so two names never give the same chips.
    let gfx = |k: u8| -> Vec<u8> {
        let mut out: Vec<u8> = (0..0x2000u32)
            .map(|i| fill ^ k ^ (i as u8).wrapping_mul(0x9b) ^ (i >> 8) as u8)
            .collect();
        let name = system.as_bytes();
        out[..name.len()].copy_from_slice(name);
        out
    };
    set_of(
        system,
        vec![
            ("main", "maincpu", chip_of(GAME_MAIN, 0x1000)),
            ("snd", "ssio:cpu", chip_of(GAME_SOUND, 0x1000)),
            ("bg", "gfx1", gfx(0x00)),
            ("fg", "gfx2", gfx(0x5a)),
        ],
    )
}

/// [`made_up_game`], added to the sets known, as a MAME zip's bytes, which
/// [`Romset::from_zip`] then loads as `system`.
pub fn known_made_up_game(system: &str) -> Vec<u8> {
    let (toml, files) = made_up_game(system);
    crate::romset::add_roms_toml(&toml).expect("a made-up game's roms.toml reads");
    zip(&files)
}

/// `files` as a zip of stored entries, as [`crate::romset::read_zip`] and
/// MAME read one.
pub fn zip(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    let u16le = |v: &mut Vec<u8>, x: usize| v.extend_from_slice(&(x as u16).to_le_bytes());
    let u32le = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&x.to_le_bytes());
    for (name, data) in files {
        let mut crc = flate2::Crc::new();
        crc.update(data);
        let (crc, at, len) = (crc.sum(), out.len() as u32, data.len() as u32);
        out.extend_from_slice(b"PK\x03\x04");
        for x in [20, 0, 0, 0, 0] {
            u16le(&mut out, x);
        }
        for x in [crc, len, len] {
            u32le(&mut out, x);
        }
        u16le(&mut out, name.len());
        u16le(&mut out, 0);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        central.extend_from_slice(b"PK\x01\x02");
        for x in [20, 20, 0, 0, 0, 0] {
            u16le(&mut central, x);
        }
        for x in [crc, len, len] {
            u32le(&mut central, x);
        }
        for x in [name.len(), 0, 0, 0, 0] {
            u16le(&mut central, x);
        }
        u32le(&mut central, 0);
        u32le(&mut central, at);
        central.extend_from_slice(name.as_bytes());
    }
    let (cd_at, cd_len) = (out.len() as u32, central.len() as u32);
    out.extend_from_slice(&central);
    out.extend_from_slice(b"PK\x05\x06");
    for x in [0, 0, files.len(), files.len()] {
        u16le(&mut out, x);
    }
    u32le(&mut out, cd_len);
    u32le(&mut out, cd_at);
    u16le(&mut out, 0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Controls, Mcr};

    /// The busy program runs as its comments say.
    #[test]
    fn the_busy_program_runs() {
        let mut t = Mcr::power_on(&busy());
        t.step(Controls::default());
        t.step(Controls {
            tap: true,
            ..Controls::default()
        });
        assert_eq!(t.peek(0xe000), 0xef, "port 1 with tap held");
        assert!(t.peek(0xe001) > 2, "the loop ran many times a frame");
        assert!(matches!(t.peek(0xe003), 1 | 2), "the table jump landed");
    }

    /// The made-up game's zip loads as its set and runs as its comments
    /// say: an interrupt a frame, attract mode for 100 frames and then
    /// play, a coin starting play at once, and the sound program writing
    /// the latch's bytes to its RAM and the AYs.
    #[test]
    fn the_made_up_game_runs_from_its_zip() {
        let zip = known_made_up_game("madeupgame");
        let set = Romset::from_zip(&zip).unwrap();
        assert_eq!(set.name, "madeupgame");
        assert!(set.extras.is_empty(), "{:?}", set.extras);
        let mut t = Mcr::power_on(&set);
        t.set_events(true);
        for _ in 0..60 {
            t.step(Controls::default());
        }
        let frames = u16::from_le_bytes([t.peek(0xe003), t.peek(0xe004)]);
        assert!(
            (58..=60).contains(&frames),
            "one interrupt a frame: {frames}"
        );
        assert_eq!(t.peek(0xe002), 0, "attract");
        assert!(t.peek(0xe100) >= 55 && t.peek(0xe109) == t.peek(0xe100));
        assert_eq!(t.sound_ram()[0], ((frames >> 3) & 0x0f) as u8);
        let ev = t.take_events();
        use crate::events::Happened;
        for what in [Happened::SoundLatch, Happened::Watchdog, Happened::AyWrite] {
            assert!(ev.iter().any(|e| e.what == what), "{what:?}");
        }
        for _ in 0..200 {
            t.step(Controls::default());
        }
        assert_eq!(t.peek(0xe002), 1, "play after 100 frames");
        assert!(t.peek(0xe201) > 0, "play moves the enemies");
        assert_eq!(t.peek(0xe020), 0, "and not the player, with nothing held");
        assert_ne!(t.peek(0xe030), 0, "an enemy wrapped and drew a number");
        let mut c = Mcr::power_on(&set);
        for i in 0..20 {
            c.step(Controls {
                coin: i == 10,
                ..Controls::default()
            });
        }
        assert_eq!(c.peek(0xe002), 1, "a coin starts play");
    }

    /// With the event log on, a frame of the busy program logs each kind
    /// it does, in time order, and the log changes no state.
    #[test]
    fn the_busy_program_logs_its_events() {
        use crate::events::Happened;
        let mut quiet = Mcr::power_on(&busy());
        let mut t = Mcr::power_on(&busy());
        t.set_events(true);
        for _ in 0..3 {
            quiet.step(Controls::default());
            t.take_events();
            t.step(Controls::default());
        }
        assert_eq!(quiet.state_hash(), t.state_hash());
        let ev = t.take_events();
        for what in [
            Happened::PortRead,
            Happened::SoundLatch,
            Happened::Watchdog,
            Happened::AyWrite,
        ] {
            assert!(ev.iter().any(|e| e.what == what), "{what:?}");
        }
        let ay = ev.iter().find(|e| e.what == Happened::AyWrite).unwrap();
        assert_eq!(ay.addr, 7, "register 7 of the first chip");
        let lines: Vec<_> = ev
            .iter()
            .filter(|e| e.what == Happened::PortRead)
            .map(|e| (e.line, e.dot))
            .collect();
        assert!(lines.windows(2).all(|w| w[0] <= w[1]), "in time order");
        assert!(t.take_events().is_empty(), "taken");
    }
}
