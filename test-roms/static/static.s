; A test program for `gp model static` (crates/gp-cli/tests/model_static.rs),
; laid out unlike Moss Hop so its rules are tested on other code: UxROM
; (mapper 2) with every routine in the fixed bank, RAM at other addresses,
; the controller shifted in with ROR (A in bit 0), the game's mode tested
; with CMP and BNE, and the objects' routines picked by an RTS jump table
; on each slot's kind. Nothing is drawn; the program only runs its rules.
; Written for this project and licensed with it (Apache-2.0).

PPUCTRL  = $2000
PPUSTATUS = $2002
JOY1     = $4016

FRAME    = $40   ; the NMI adds 1
VBLANK   = $41   ; set by the NMI, cleared by the loop
BUTTONS  = $42   ; controller 1, A in bit 0 ... Right in bit 7
LASTBTN  = $43   ; the frame before
PRESSED  = $44   ; down now and up the frame before
MODE     = $45   ; 0 title, 1 play, 2 lost
WAIT     = $46   ; frames left in the lost mode
HERO_X   = $50
HERO_Y   = $51
HERO_YF  = $52   ; fraction of HERO_Y, 256ths
HERO_VY  = $53   ; vertical speed, whole pixels, signed
HERO_VYF = $54   ; its fraction
STANDING = $55

; Four objects, one byte each per field.
OBJ_KIND  = $0400   ; 0 crawler, 1 hopper
OBJ_X     = $0404
OBJ_STATE = $0408   ; 0 none, 1 active, 2 stunned
OBJ_TIME  = $040C   ; frames left stunned

WALK     = 1
JUMP_VY  = $FC80    ; -3.5 pixels a frame
FALL     = $20      ; 0.125 pixels a frame, every frame
TOP_FALL = 3
LOST_FRAMES = 75
STUN_FRAMES = 45

; Buttons, as ROR leaves them.
BTN_A     = $01
BTN_LEFT  = $40
BTN_RIGHT = $80

.segment "HEADER"
    .byte "NES", $1A, 4, 1, $20, $00, 0, 0, 0, 0, 0, 0, 0, 0

.segment "BANK0"
    .byte $55        ; data only

.segment "CODE"
reset:
    sei
    ldx #$FF
    txs
    lda #0
    sta MODE
    jsr spawn
    lda #$80
    sta PPUCTRL
frame:
    lda VBLANK
    beq frame
    lda #0
    sta VBLANK
    jsr pad
    lda MODE
    cmp #1
    bne @not_play
    jsr hero
    jsr objects
    jmp frame
@not_play:
    cmp #2
    bne @title
    dec WAIT
    bne @still
    lda #0
    sta MODE
@still:
    jmp frame
@title:
    lda PRESSED
    and #BTN_A
    beq @wait
    lda #1
    sta MODE
@wait:
    jmp frame

nmi:
    inc FRAME
    lda #1
    sta VBLANK
    rti

irq:
    rti

; The controller, A first: each bit goes in at the top and moves down.
pad:
    lda BUTTONS
    sta LASTBTN
    lda #1
    sta JOY1
    lda #0
    sta JOY1
    ldy #8
@bit:
    lda JOY1
    lsr a
    ror BUTTONS
    dey
    bne @bit
    lda LASTBTN
    eor #$FF
    and BUTTONS
    sta PRESSED
    rts

spawn:
    lda #32
    sta HERO_X
    lda #160
    sta HERO_Y
    lda #1
    sta STANDING
    ldx #3
@obj:
    lda start_kind,x
    sta OBJ_KIND,x
    lda start_x,x
    sta OBJ_X,x
    lda #1
    sta OBJ_STATE,x
    lda #0
    sta OBJ_TIME,x
    dex
    bpl @obj
    rts

hero:
    lda BUTTONS
    and #BTN_RIGHT
    beq @no_right
    inc HERO_X
@no_right:
    lda BUTTONS
    and #BTN_LEFT
    beq @no_left
    dec HERO_X
@no_left:
    lda STANDING
    beq @air
    lda PRESSED
    and #BTN_A
    beq @air
    lda #<JUMP_VY
    sta HERO_VYF
    lda #>JUMP_VY
    sta HERO_VY
    lda #0
    sta STANDING
@air:
    clc
    lda HERO_VYF
    adc #FALL
    sta HERO_VYF
    lda HERO_VY
    adc #0
    sta HERO_VY
    bmi @move
    cmp #TOP_FALL
    bcc @move
    lda #TOP_FALL
    sta HERO_VY
    lda #0
    sta HERO_VYF
@move:
    clc
    lda HERO_YF
    adc HERO_VYF
    sta HERO_YF
    lda HERO_Y
    adc HERO_VY
    sta HERO_Y
    rts

; Each active object runs its kind's routine, through an RTS jump.
objects:
    ldx #0
@obj:
    lda OBJ_STATE,x
    cmp #2
    bne @not_stunned
    dec OBJ_TIME,x
    bne @next
    lda #1
    sta OBJ_STATE,x
    jmp @next
@not_stunned:
    cmp #1
    bne @next
    jsr run_kind
@next:
    inx
    cpx #4
    bne @obj
    rts

run_kind:
    ldy OBJ_KIND,x
    lda kind_hi,y
    pha
    lda kind_lo,y
    pha
    rts

; A crawler walks right one pixel every fourth frame.
crawler:
    lda FRAME
    and #3
    bne @done
    inc OBJ_X,x
@done:
    rts

; A hopper is stunned when the hero is level with it, and the game is lost.
hopper:
    lda HERO_X
    cmp OBJ_X,x
    bne @done
    lda #2
    sta OBJ_STATE,x
    lda #STUN_FRAMES
    sta OBJ_TIME,x
    lda #2
    sta MODE
    lda #LOST_FRAMES
    sta WAIT
@done:
    rts

.segment "RODATA"
kind_lo: .byte <(crawler - 1), <(hopper - 1)
kind_hi: .byte >(crawler - 1), >(hopper - 1)
start_kind: .byte 0, 1, 0, 1
start_x: .byte 40, 80, 120, 200

.segment "VECTORS"
    .word nmi, reset, irq
