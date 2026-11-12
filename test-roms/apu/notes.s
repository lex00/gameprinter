; A tiny NES program that plays a few notes, for gp sound's open tier
; (docs/sound.md): a made-up eight-note phrase on pulse 1 with a falling
; volume, a bass note on the triangle every four notes, and a noise beat
; on every note, 12 frames a note, looping forever. Each NMI steps the
; phrase, the way a game's sound engine runs once a frame. It holds no
; game's data: the pitches are equal-tempered periods computed for
; NTSC (CPU clock / (16 * f) - 1).

.segment "HEADER"
    .byte "NES", $1A, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0

.segment "ZEROPAGE"
note:   .res 1          ; the phrase's note, 0 to 7
timer:  .res 1          ; frames left of the note

.segment "CODE"
NOTE_FRAMES = 12

; C4 E4 G4 C5 B4 G4 E4 D4 on pulse 1 (timer periods).
lo:     .byte <427, <339, <285, <213, <225, <285, <339, <380
hi:     .byte >427, >339, >285, >213, >225, >285, >339, >380
; C2 and G2 on the triangle (it sounds an octave below a pulse's period).
tlo:    .byte <855, <570
thi:    .byte >855, >570

reset:
    sei
    cld
    ldx #$FF
    txs
    inx
    stx $2000           ; NMI off while setting up
    stx $2001
    stx note
    lda #1
    sta timer
    lda #$40
    sta $4017           ; 4-step frame counter, no IRQ
    lda #$0F
    sta $4015           ; pulses, triangle, noise
    bit $2002
vblank1:
    bit $2002
    bpl vblank1
vblank2:
    bit $2002
    bpl vblank2
    lda #$80
    sta $2000           ; NMI on
loop:
    jmp loop

nmi:
    pha
    txa
    pha
    dec timer
    bne envelope
    ; A new note.
    lda #NOTE_FRAMES
    sta timer
    ldx note
    lda #$BC            ; duty 2, constant volume 12
    sta $4000
    lda #$08            ; sweep off
    sta $4001
    lda lo,x
    sta $4002
    lda hi,x
    ora #$08
    sta $4003
    lda #$34            ; noise: constant volume 4
    sta $400C
    lda #$05
    sta $400E
    lda #$08
    sta $400F
    txa
    and #3
    bne next
    txa
    lsr
    lsr
    tax
    lda #$FF            ; triangle on, linear counter held
    sta $4008
    lda tlo,x
    sta $400A
    lda thi,x
    ora #$08
    sta $400B
next:
    ldx note
    inx
    txa
    and #7
    sta note
    jmp done
envelope:
    ; The volume falls by one every other frame; the noise stops after 3.
    lda timer
    cmp #NOTE_FRAMES - 3
    bne vol
    lda #$30
    sta $400C
vol:
    lda timer
    lsr
    ora #$B0            ; duty 2, constant volume timer / 2
    sta $4000
done:
    pla
    tax
    pla
    rti

irq:
    rti

.segment "VECTORS"
    .word nmi, reset, irq

.segment "CHARS"
    .res $2000, 0
