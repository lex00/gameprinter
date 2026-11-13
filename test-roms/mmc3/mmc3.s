; An MMC3 (mapper 4) test program of our own, for `gp testroms run` and the
; open baseline baselines/mmc3 (docs/test-roms.md). Kirby's Adventure is
; MMC3. The pinned collection's MMC3 tests cover the IRQ counter in depth;
; this one covers banking and mirroring, and runs the scanline IRQ the way
; a game does, so a recording holds bank registers changing every frame.
;
; The cartridge is TKROM-shaped without the battery: 128KB PRG ROM in 16
; banks of 8KB, 128KB CHR ROM in 128 banks of 1KB, and 8KB PRG RAM at
; $6000. The first byte of each PRG bank is its bank number, and every byte
; of each CHR bank is its bank number, so a read says which bank is mapped.
;
; It reports with blargg's $6000 protocol: $6001-$6003 hold DE B0 61, $6000
; is $80 while running and then the result code (0 passed, otherwise the
; number of the check that failed), and $6004 on holds the text. RESULT
; ($00) holds the same code, so a baseline can assert it.
;
; Checks, numbered as the result codes:
;  2  at power-on $E000 holds the last bank
;  3  PRG mode 0: R6 switches $8000 and R7 $A000 through every bank, $C000
;     holds the second-last bank
;  4  PRG mode 1: R6 switches $C000, $8000 holds the second-last bank
;  5  CHR mode 0: R0 and R1 switch 2KB at $0000 and $0800 (low bit
;     ignored), R2 to R5 switch 1KB each at $1000 to $1C00
;  6  CHR mode 1: the two halves swap
;  7  mirroring ($A000): 0 vertical, 1 horizontal
;  8  scanline IRQ: with the counter reloaded in vblank and a latch of 99,
;     sprites at $1000, two IRQs a frame (at the ends of scanlines 98 and 198)
;  9  writing $E000 disables the IRQ: none in a frame (check 8 already
;     relies on $E000 acknowledging: the handler would loop otherwise)
;
; After the checks it runs on: each frame it reads controller 1 into
; BUTTONS ($02), counts frames in FRAMES ($01), switches R6 and R2 to the
; frame count, sets the mirroring from the A button, and keeps the scanline
; IRQ going, with the IRQs of the last frame in IRQ_FRAME ($04).
;
; Build: ca65 mmc3.s && ld65 -C mmc3.cfg mmc3.o -o mmc3.nes (see the justfile
; recipe build-testroms).

.segment "HEADER"
    .byte "NES", $1A
    .byte 8             ; PRG ROM in 16KB units: 128KB
    .byte 16            ; CHR ROM in 8KB units: 128KB
    .byte $40           ; mapper 4 (low nibble), horizontal mirroring, no battery
    .byte $00           ; mapper high nibble 0, iNES 1.0
    .byte 0, 0, 0, 0, 0, 0, 0, 0

STATUS   = $6000
TEXT     = $6004

PPUCTRL   = $2000
PPUMASK   = $2001
PPUSTATUS = $2002
PPUADDR   = $2006
PPUDATA   = $2007
JOY1      = $4016
APUFRAME  = $4017

BANK_SELECT = $8000
BANK_DATA   = $8001
MIRRORING   = $A000
IRQ_LATCH   = $C000
IRQ_RELOAD  = $C001
IRQ_DISABLE = $E000
IRQ_ENABLE  = $E001

LATCH = 99              ; an IRQ every 100 scanlines

.zeropage
result:    .res 1       ; $00
frames:    .res 1       ; $01
buttons:   .res 1       ; $02
irqs:      .res 1       ; $03, IRQs so far this frame
irq_frame: .res 1       ; $04, IRQs in the last frame
m8:        .res 1       ; bank numbers read at $8000, $A000 and $C000
mA:        .res 1
mC:        .res 1
count:     .res 1
mode:      .res 1
textptr:   .res 2

; Banks 0 to 14: each starts with its number.
.segment "BANKS"
.repeat 15, n
    .byte n
    .res $2000 - 1, $FF
.endrepeat

.segment "CODE"
    .byte 15            ; bank 15's number, at $E000

reset:
    sei
    cld
    ldx #$FF
    txs
    lda #0
    sta PPUCTRL
    sta PPUMASK
    sta IRQ_DISABLE
    lda #$40            ; no APU frame IRQ
    sta APUFRAME

    lda #$80
    sta STATUS
    sta result
    lda #$DE
    sta STATUS + 1
    lda #$B0
    sta STATUS + 2
    lda #$61
    sta STATUS + 3
    lda #0
    sta TEXT
    sta frames
    sta irqs
    sta irq_frame

    ; Wait two vblanks for the PPU to warm up before touching VRAM.
    bit PPUSTATUS
@vbl1:
    bit PPUSTATUS
    bpl @vbl1
@vbl2:
    bit PPUSTATUS
    bpl @vbl2

; 2: $E000 holds the last bank.
    lda $E000
    cmp #15
    beq :+
    lda #2
    jmp fail
:

; 3: PRG mode 0. R6 = Y at $8000, R7 = 15 - Y at $A000, 14 at $C000.
    lda #$00
    sta mode
    ldy #0
@mode0:
    jsr prg_probe
    cpy m8
    bne @mode0_fail
    tya
    eor #$0F
    cmp mA
    bne @mode0_fail
    lda mC
    cmp #14
    bne @mode0_fail
    iny
    cpy #16
    bne @mode0
    jmp @mode0_ok
@mode0_fail:
    lda #3
    jmp fail
@mode0_ok:

; 4: PRG mode 1. R6 = Y at $C000, 14 at $8000.
    lda #$40
    sta mode
    ldy #0
@mode1:
    jsr prg_probe
    cpy mC
    bne @mode1_fail
    lda m8
    cmp #14
    bne @mode1_fail
    tya
    eor #$0F
    cmp mA
    bne @mode1_fail
    iny
    cpy #16
    bne @mode1
    jmp @mode1_ok
@mode1_fail:
    lda #4
    jmp fail
@mode1_ok:
    lda #$00            ; back to PRG mode 0
    sta BANK_SELECT

; 5: CHR mode 0.
    lda #$00
    jsr set_chr
    ldx #0
@chr0:
    lda chr_addr, x
    jsr chr_at
    cmp chr_mode0, x
    bne @chr0_fail
    inx
    cpx #8
    bne @chr0
    jmp @chr0_ok
@chr0_fail:
    lda #5
    jmp fail
@chr0_ok:

; 6: CHR mode 1.
    lda #$80
    jsr set_chr
    ldx #0
@chr1:
    lda chr_addr, x
    jsr chr_at
    cmp chr_mode1, x
    bne @chr1_fail
    inx
    cpx #8
    bne @chr1
    jmp @chr1_ok
@chr1_fail:
    lda #6
    jmp fail
@chr1_ok:
    lda #$00            ; back to CHR mode 0
    sta BANK_SELECT

; 7: mirroring. Vertical: $2800 overwrites $2000, $2C00 overwrites $2400.
    lda #0
    sta MIRRORING
    jsr write_four
    lda #$20
    jsr read_nt
    cmp #3
    bne @mirror_fail
    lda #$24
    jsr read_nt
    cmp #4
    bne @mirror_fail
    ; Horizontal: $2400 overwrites $2000, $2C00 overwrites $2800.
    lda #1
    sta MIRRORING
    jsr write_four
    lda #$20
    jsr read_nt
    cmp #2
    bne @mirror_fail
    lda #$28
    jsr read_nt
    cmp #4
    bne @mirror_fail
    jmp @mirror_ok
@mirror_fail:
    lda #7
    jmp fail
@mirror_ok:

; 8: two scanline IRQs a frame. NMI reloads the counter in vblank.
    jsr wait_vbl
    lda #$88            ; NMI on, sprites at $1000, background at $0000
    sta PPUCTRL
    lda #LATCH
    sta IRQ_LATCH
    sta IRQ_RELOAD
    sta IRQ_ENABLE
    cli
    jsr wait_frame
    lda #$18            ; background and sprites on
    sta PPUMASK
    jsr wait_frame
    jsr wait_frame
    jsr wait_frame
    lda irq_frame
    cmp #2
    beq :+
    lda #8
    jmp fail
:

; 9: $E000 disables: no IRQs in a whole frame.
    sta IRQ_DISABLE
    jsr wait_frame
    jsr wait_frame
    lda irq_frame
    beq :+
    lda #9
    jmp fail
:

    lda #<passed
    ldx #>passed
    jsr print
    lda #0
    sta STATUS
    sta result

; Run on, one pass a frame.
    sta IRQ_ENABLE
@run:
    jsr wait_frame
    jsr read_pad
    lda #$06            ; R6 = frame count, at $8000
    sta BANK_SELECT
    lda frames
    sta BANK_DATA
    lda #$02            ; R2 = frame count, at $1000
    sta BANK_SELECT
    lda frames
    sta BANK_DATA
    lda buttons         ; A held: horizontal, else vertical
    asl a
    lda #0
    rol a
    sta MIRRORING
    jmp @run

; A failing check's number in A: report it and stop.
fail:
    pha
    sta result
    lda #<failed
    ldx #>failed
    jsr print
    pla
    pha
    ora #'0'
    jsr putc
    pla
    sta STATUS
@hang:
    jmp @hang

; Y = R6, 15 - Y = R7, with the PRG mode in `mode`. Reads the bank numbers
; at $8000, $A000 and $C000 into m8, mA and mC.
prg_probe:
    lda #$06
    ora mode
    sta BANK_SELECT
    sty BANK_DATA
    lda #$07
    ora mode
    sta BANK_SELECT
    tya
    eor #$0F
    sta BANK_DATA
    lda $8000
    sta m8
    lda $A000
    sta mA
    lda $C000
    sta mC
    rts

; A = CHR mode ($00 or $80). Sets R0 to R5 from chr_regs.
set_chr:
    sta mode
    ldx #0
@reg:
    txa
    ora mode
    sta BANK_SELECT
    lda chr_regs, x
    sta BANK_DATA
    inx
    cpx #6
    bne @reg
    rts

; A = high byte of a PPU address. Returns the byte there.
chr_at:
    bit PPUSTATUS
    sta PPUADDR
    lda #0
    sta PPUADDR
    lda PPUDATA
    lda PPUDATA
    rts

; A = high byte of a nametable. Returns the byte at its start.
read_nt = chr_at

; Writes 1, 2, 3 and 4 to the starts of the four nametables, in order.
write_four:
    ldx #0
@one:
    bit PPUSTATUS
    lda nt_addr, x
    sta PPUADDR
    lda #0
    sta PPUADDR
    inx
    stx PPUDATA
    cpx #4
    bne @one
    rts

; Waits for the vblank flag, with NMI off.
wait_vbl:
    bit PPUSTATUS
@wait:
    bit PPUSTATUS
    bpl @wait
    rts

; Waits for the next NMI.
wait_frame:
    lda frames
@wait:
    cmp frames
    beq @wait
    rts

; Reads controller 1 into buttons: A in bit 7 down to Right in bit 0.
read_pad:
    lda #1
    sta JOY1
    lda #0
    sta JOY1
    ldx #8
@bit:
    lda JOY1
    lsr a
    rol buttons
    dex
    bne @bit
    rts

nmi:
    pha
    lda irqs
    sta irq_frame
    lda #0
    sta irqs
    sta IRQ_RELOAD      ; reload on the next A12 rise, at the pre-render line
    inc frames
    pla
    rti

irq:
    pha
    sta IRQ_DISABLE     ; acknowledge
    sta IRQ_ENABLE
    inc irqs
    pla
    rti

; A = character, appended to the text at TEXT, zero-terminated.
putc:
    ldy #0
@find:
    ldx TEXT, y
    beq @here
    iny
    bne @find
@here:
    sta TEXT, y
    lda #0
    sta TEXT + 1, y
    rts

; A/X = address of a zero-terminated string, appended to the text.
print:
    sta textptr
    stx textptr + 1
    ldy #0
@next:
    lda (textptr), y
    beq @end
    sty count
    jsr putc
    ldy count
    iny
    bne @next
@end:
    rts

; R0 to R5 for checks 5 and 6.
chr_regs:
    .byte $0B, $21, 40, 41, 50, 60
; The 1KB windows' high address bytes, and the bank expected in each.
chr_addr:
    .byte $00, $04, $08, $0C, $10, $14, $18, $1C
chr_mode0:
    .byte 10, 11, 32, 33, 40, 41, 50, 60
chr_mode1:
    .byte 40, 41, 50, 60, 10, 11, 32, 33
nt_addr:
    .byte $20, $24, $28, $2C

passed:
    .byte "MMC3 checks passed", 0
failed:
    .byte "MMC3 check failed: #", 0

.segment "VECTORS"
    .word nmi, reset, irq

; Each 1KB CHR bank is filled with its own number.
.segment "CHR"
.repeat 128, n
    .res $400, n
.endrepeat
