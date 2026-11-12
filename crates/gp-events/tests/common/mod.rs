//! The small program the tests run: each kind of transfer, and NMIs.

#[rustfmt::skip]
const MAIN: &[u8] = &[
    0x78,             // C000 SEI
    0xA2, 0xFF,       // C001 LDX #$FF
    0x9A,             // C003 TXS
    0xA9, 0x80,       // C004 LDA #$80
    0x8D, 0x00, 0x20, // C006 STA $2000     NMI on
    0x20, 0x60, 0xC0, // C009 JSR $C060     loop
    0xA9, 0x30,       // C00C LDA #$30
    0x85, 0x10,       // C00E STA $10
    0xA9, 0xC0,       // C010 LDA #$C0
    0x85, 0x11,       // C012 STA $11       pointer $10 = $C030
    0x6C, 0x10, 0x00, // C014 JMP ($0010)
];

#[rustfmt::skip]
const AT_C030: &[u8] = &[
    0xA9, 0xC0,       // C030 LDA #$C0
    0x48,             // C032 PHA
    0xA9, 0x37,       // C033 LDA #$37
    0x48,             // C035 PHA          $C037 pushed
    0x60,             // C036 RTS          to $C038: a jump
    0xEA,             // C037 NOP
    0xA2, 0x01,       // C038 LDX #$01
    0xCA,             // C03A DEX          Z set
    0xF0, 0x02,       // C03B BEQ $C03F    taken
    0xEA, 0xEA,       // C03D
    0xD0, 0x02,       // C03F BNE $C043    not taken
    0xEA, 0xEA,       // C041
    0x4C, 0x09, 0xC0, // C043 JMP $C009
];

pub const SUB: u16 = 0xC060; // RTS
pub const NMI: u16 = 0xC070; // RTI

pub fn rom() -> Vec<u8> {
    let mut prg = vec![0xEAu8; 0x4000];
    prg[..MAIN.len()].copy_from_slice(MAIN);
    prg[0x30..0x30 + AT_C030.len()].copy_from_slice(AT_C030);
    prg[usize::from(SUB - 0xC000)] = 0x60;
    prg[usize::from(NMI - 0xC000)] = 0x40;
    prg[0x3FFA..0x3FFC].copy_from_slice(&NMI.to_le_bytes());
    prg[0x3FFC..0x3FFE].copy_from_slice(&0xC000u16.to_le_bytes());
    prg[0x3FFE..0x4000].copy_from_slice(&NMI.to_le_bytes());
    let mut rom = b"NES\x1a\x01\x00\x00\x00".to_vec();
    rom.resize(16, 0);
    rom.extend(prg);
    rom
}
