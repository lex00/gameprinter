; An MMC1 (mapper 1) test program of our own, for `gp testroms run`
; (docs/test-roms.md). The pinned test ROM collection has no MMC1 test that
; reports a result, and Mega Man 2 is MMC1.
;
; The cartridge is SKROM-shaped: 256KB PRG ROM in 16 banks of 16KB, 128KB
; CHR ROM in 32 banks of 4KB, and 8KB PRG RAM at $6000. The first byte of
; each PRG bank is its bank number, and every byte of each CHR bank is its
; bank number, so a read says which bank is mapped.
;
; It reports with blargg's $6000 protocol: $6001-$6003 hold DE B0 61, $6000
; is $80 while running and then the result code (0 passed, otherwise the
; number of the check that failed), and $6004 on holds the text. RESULT
; ($00) holds the same code, so the open baseline baselines/mmc1 can
; assert it.
;
; Checks, numbered as the result codes:
;  2  after the reset stub's bit 7 write: $C000 holds the last bank (this
;     repeats check 6; MMC1's state at power-on itself isn't tested)
;  3  PRG mode 3: $8000 switches through every bank, $C000 stays the last
;  4  PRG mode 2: $8000 fixed to bank 0, $C000 switches
;  5  PRG 32KB mode: the low bank bit is ignored, $C000 is the next bank
;  6  a write with bit 7 set empties the shift register and restores mode 3
;  7  CHR 8KB mode: the low bank bit is ignored, $1000 is the next bank
;  8  CHR 4KB mode: $0000 and $1000 switch independently
;  9  mirroring: one-screen lower, one-screen upper, vertical, horizontal
; 10  the second write of a read-modify-write (on the next cycle) is ignored
; 11  PRG RAM disable (bit 4 of $E000, MMC1B and later): reads don't see the
;     RAM and writes don't reach it
;
; After the checks it runs on: each frame (by the vblank flag; NMI stays
; off) it counts frames in FRAMES ($01), reads controller 1 into BUTTONS
; ($02), switches the PRG bank at $8000 and CHR bank 1 to the frame count,
; and sets the mirroring from A and B, so a recording holds three MMC1
; registers changing every frame.
;
; Build: ca65 mmc1.s && ld65 -C mmc1.cfg mmc1.o -o mmc1.nes (see the justfile
; recipe build-testroms).

.segment "HEADER"
    .byte "NES", $1A
    .byte 16            ; PRG ROM in 16KB units: 256KB
    .byte 16            ; CHR ROM in 8KB units: 128KB
    .byte $10           ; mapper 1 (low nibble), horizontal mirroring, no battery
    .byte $00           ; mapper high nibble 0, iNES 1.0
    .byte 0, 0, 0, 0, 0, 0, 0, 0

.import __RAMCODE_LOAD__, __RAMCODE_RUN__, __RAMCODE_SIZE__

STATUS   = $6000
TEXT     = $6004

PPUCTRL   = $2000
PPUMASK   = $2001
PPUSTATUS = $2002
PPUADDR   = $2006
PPUDATA   = $2007

JOY1     = $4016

.zeropage
result:   .res 1        ; $00
frames:   .res 1        ; $01
buttons:  .res 1        ; $02
lo_mark:  .res 1        ; bank number read at $8000
hi_mark:  .res 1        ; bank number read at $C000
count:    .res 1
want:     .res 1
textptr:  .res 2

; Five writes to one MMC1 register, bit 0 of A first. STA and LSR take six
; cycles between writes, so none falls on the cycle after the last.
.macro serial addr
    sta addr
    lsr a
    sta addr
    lsr a
    sta addr
    lsr a
    sta addr
    lsr a
    sta addr
.endmacro

; A branch on not-equal with a 16-bit reach.
.macro jne target
    beq :+
    jmp target
:
.endmacro

; Every bank but the last: its number, then a reset stub and vectors at the
; top, so the console starts in a known mode whichever bank is at $C000.
.macro bank n
.segment .sprintf("BANK%d", n)
    .byte n
    .res $4000 - 1 - 16 - 6, $FF
    stub
.endmacro

; 16 bytes at $FFEA, then the vectors at $FFFA.
.macro stub
    .local reset, irq
reset:
    sei                 ; $FFEA
    ldx #$FF
    txs
    lda #$80            ; empty the shift register, PRG mode 3: the last bank at $C000
    sta $8000
    jmp main            ; $FFF3
irq:
    rti                 ; $FFF6
    .byte $FF, $FF, $FF
    .word irq, reset, irq
.endmacro

bank 0
bank 1
bank 2
bank 3
bank 4
bank 5
bank 6
bank 7
bank 8
bank 9
bank 10
bank 11
bank 12
bank 13
bank 14

.segment "BANK15"
    .byte 15

; Run from RAM ($0300), since they switch out the bank at $C000.
.segment "RAMCODE"

; A = control, Y = PRG bank. Reads the bank numbers at $8000 and $C000 into
; lo_mark and hi_mark, then goes back to mode 3 (control $0E: vertical, CHR
; 8KB) so the last bank is at $C000 again.
probe:
    serial $8000
    tya
    serial $E000
    lda $8000
    sta lo_mark
    lda $C000
    sta hi_mark
    lda #$0E
    serial $8000
    rts

; From 32KB mode with bank 4, a write with bit 7 set must bring the last
; bank back to $C000 by itself. Leaves the bank at $C000 in hi_mark.
reset_from_32k:
    lda #$02            ; 32KB mode, vertical
    serial $8000
    lda #4
    serial $E000
    lda #$80
    sta $8000
    lda $C000
    sta hi_mark
    lda #$0E
    serial $8000
    rts

.segment "CODE"

main:
    sei
    cld
    ldx #$FF
    txs
    lda #0
    sta PPUCTRL
    sta PPUMASK

    ; Copy the RAM routines to $0300.
    ldx #0
@copy:
    lda __RAMCODE_LOAD__, x
    sta __RAMCODE_RUN__, x
    inx
    cpx #<__RAMCODE_SIZE__
    .assert __RAMCODE_SIZE__ < 256, error, "the RAM routines must fit the 8-bit copy loop"
    bne @copy

    ; The report: status running, then the signature.
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

    ; Wait two vblanks for the PPU to warm up before touching VRAM.
    bit PPUSTATUS
@vbl1:
    bit PPUSTATUS
    bpl @vbl1
@vbl2:
    bit PPUSTATUS
    bpl @vbl2

; 2: the last bank is at $C000 after the reset stub's write of $80.
    lda $C000
    cmp #15
    beq :+
    lda #2
    jmp fail
:

; 3: mode 3 (control $0E): $8000 is bank Y, $C000 stays 15.
    lda #0
    sta count
@mode3:
    ldy count
    lda #$0E
    jsr probe
    lda lo_mark
    cmp count
    bne @mode3_fail
    lda hi_mark
    cmp #15
    bne @mode3_fail
    inc count
    lda count
    cmp #16
    bne @mode3
    jmp @mode3_ok
@mode3_fail:
    lda #3
    jmp fail
@mode3_ok:

; 4: mode 2 (control $0A): $8000 is bank 0, $C000 is bank Y.
    lda #0
    sta count
@mode2:
    ldy count
    lda #$0A
    jsr probe
    lda lo_mark
    bne @mode2_fail
    lda hi_mark
    cmp count
    bne @mode2_fail
    inc count
    lda count
    cmp #16
    bne @mode2
    jmp @mode2_ok
@mode2_fail:
    lda #4
    jmp fail
@mode2_ok:

; 5: 32KB mode (control $02): $8000 is bank Y with bit 0 clear, $C000 the next.
    lda #0
    sta count
@mode32:
    ldy count
    lda #$02
    jsr probe
    lda count
    and #$FE
    sta want
    cmp lo_mark
    bne @mode32_fail
    inc want
    lda want
    cmp hi_mark
    bne @mode32_fail
    inc count
    lda count
    cmp #16
    bne @mode32
    jmp @mode32_ok
@mode32_fail:
    lda #5
    jmp fail
@mode32_ok:

; 6: bit 7 restores mode 3, and empties a half-filled shift register.
    jsr reset_from_32k
    lda hi_mark
    cmp #15
    bne @reset_fail
    lda #1              ; three bits of a value that never completes
    sta $E000
    nop
    sta $E000
    nop
    sta $E000
    lda #$80
    sta $8000
    nop
    lda #6
    serial $E000
    lda $8000
    cmp #6
    beq @reset_ok
@reset_fail:
    lda #6
    jmp fail
@reset_ok:

; 7: CHR 8KB mode (control $0E): $0000 is bank Y with bit 0 clear, $1000 the next.
    lda #0
    sta count
@chr8:
    lda count
    serial $A000
    lda count
    and #$1E
    sta want
    lda #$00
    jsr read_chr
    cmp want
    bne @chr8_fail
    inc want
    lda #$10
    jsr read_chr
    cmp want
    bne @chr8_fail
    inc count
    lda count
    cmp #32
    bne @chr8
    jmp @chr8_ok
@chr8_fail:
    lda #7
    jmp fail
@chr8_ok:

; 8: CHR 4KB mode (control $1E): $0000 is bank Y, $1000 is bank 31 - Y.
    lda #$1E
    serial $8000
    lda #0
    sta count
@chr4:
    lda count
    serial $A000
    lda #31
    sec
    sbc count
    sta want
    serial $C000
    lda #$00
    jsr read_chr
    cmp count
    bne @chr4_fail
    lda #$10
    jsr read_chr
    cmp want
    bne @chr4_fail
    inc count
    lda count
    cmp #32
    bne @chr4
    jmp @chr4_ok
@chr4_fail:
    lda #8
    jmp fail
@chr4_ok:
    lda #$0E
    serial $8000

; 9: mirroring. One-screen lower and upper are two different pages.
    lda #$0C            ; one-screen lower
    serial $8000
    lda #$20
    ldx #$11
    jsr write_nt
    lda #$0D            ; one-screen upper
    serial $8000
    lda #$20
    ldx #$22
    jsr write_nt
    lda #$2C            ; upper, through the last nametable
    jsr read_nt
    cmp #$22
    jne @mirror_fail
    lda #$0C
    serial $8000
    lda #$24            ; lower, through the second nametable
    jsr read_nt
    cmp #$11
    jne @mirror_fail
    ; Vertical: $2000 = $2800 and $2400 = $2C00.
    lda #$0E
    serial $8000
    jsr write_four
    lda #$20
    jsr read_nt
    cmp #3
    jne @mirror_fail
    lda #$24
    jsr read_nt
    cmp #4
    jne @mirror_fail
    ; Horizontal: $2000 = $2400 and $2800 = $2C00.
    lda #$0F
    serial $8000
    jsr write_four
    lda #$20
    jsr read_nt
    cmp #2
    jne @mirror_fail
    lda #$28
    jsr read_nt
    cmp #4
    jne @mirror_fail
    lda #$0E
    serial $8000
    jmp @mirror_ok
@mirror_fail:
    lda #9
    jmp fail
@mirror_ok:

; 10: INC on a ROM byte of $FF writes $FF, then $00 on the next cycle. The
; first empties the shift register; the second must be ignored, or it would
; be the first bit of the next value and bank 6 would land as bank 12.
    inc ff_byte
    nop
    lda #6
    serial $E000
    lda $8000
    cmp #6
    beq :+
    lda #10
    jmp fail
:

; 11: PRG RAM disabled (bit 4 of $E000) doesn't read back or take writes.
    lda #$A5
    sta $6100
    lda #$10            ; bank 0, PRG RAM disabled
    serial $E000
    lda $6100
    cmp #$A5
    beq @ram_fail
    lda #$5A
    sta $6100
    lda #$00            ; bank 0, PRG RAM enabled
    serial $E000
    lda $6100
    cmp #$A5
    beq @ram_ok
@ram_fail:
    lda #$00            ; make sure the report can be written
    serial $E000
    lda #11
    jmp fail
@ram_ok:

    lda #<passed
    ldx #>passed
    jsr print
    lda #0
    sta STATUS
    sta result
    sta frames

; Run on, one pass a frame.
@run:
    bit PPUSTATUS
    bpl @run
    inc frames
    ; Controller 1 into buttons: A in bit 7 down to Right in bit 0.
    lda #1
    sta JOY1
    lda #0
    sta JOY1
    ldx #8
@pad:
    lda JOY1
    lsr a
    rol buttons
    dex
    bne @pad
    lda frames          ; PRG bank at $8000, PRG RAM on
    and #$0F
    serial $E000
    lda frames          ; CHR bank 1
    serial $C000
    lda buttons         ; mirroring from A (bit 1) and B (bit 0)
    lsr a
    lsr a
    lsr a
    lsr a
    lsr a
    lsr a
    ora #$1C            ; CHR 4KB, PRG mode 3
    serial $8000
    jmp @run

; A = failing check's number.
fail:
    pha
    lda #<failed
    ldx #>failed
    jsr print
    pla
    pha
    ora #'0'
    cmp #'9' + 1
    bcc @digit
    ; 10 and 11.
    pha
    lda #'1'
    jsr putc
    pla
    sec
    sbc #10
@digit:
    jsr putc
    pla
    sta STATUS
    sta result
@hang:
    jmp @hang

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

; A = high byte of a pattern table address. Returns the byte there.
read_chr:
    bit PPUSTATUS
    sta PPUADDR
    lda #0
    sta PPUADDR
    lda PPUDATA         ; the read buffer's old contents
    lda PPUDATA
    rts

; A = high byte of a nametable, X = the byte to write at its start.
write_nt:
    bit PPUSTATUS
    sta PPUADDR
    lda #0
    sta PPUADDR
    stx PPUDATA
    rts

; A = high byte of a nametable. Returns the byte at its start.
read_nt:
    bit PPUSTATUS
    sta PPUADDR
    lda #0
    sta PPUADDR
    lda PPUDATA
    lda PPUDATA
    rts

; Writes 1, 2, 3 and 4 to the starts of the four nametables, in order.
write_four:
    lda #$20
    ldx #1
    jsr write_nt
    lda #$24
    ldx #2
    jsr write_nt
    lda #$28
    ldx #3
    jsr write_nt
    lda #$2C
    ldx #4
    jsr write_nt
    rts

ff_byte:
    .byte $FF
passed:
    .byte "MMC1 checks passed", 0
failed:
    .byte "MMC1 check failed: #", 0

.segment "STUB15"
    stub

; Each 4KB CHR bank is filled with its own number.
.segment "CHR"
.repeat 32, n
    .res $1000, n
.endrepeat
