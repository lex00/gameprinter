; Moss Hop: the Game Printer reference game. A small NES platformer, written
; for this project and licensed with it (Apache-2.0). Mapper 0 (NROM-128),
; 16KB PRG, 8KB CHR, vertical mirroring.
;
; The player runs and jumps through a level two screens wide, past two
; slugs of one kind, to a burrow, which clears the level. On the way are a
; ? block that gives a coin when bumped from below and floating coins. A
; slug touched from above is squashed; touched any other way, or a fall
; into the pit, kills the player, who hops and falls off the screen. Then
; the level starts over, or the game is over when no lives are left.
; docs/reference-game.md describes the mechanics and the RAM map; ram.inc
; holds the addresses.

.include "ram.inc"

PPUCTRL   = $2000
PPUMASK   = $2001
PPUSTATUS = $2002
OAMADDR   = $2003
PPUSCROLL = $2005
PPUADDR   = $2006
PPUDATA   = $2007
DMC_FREQ  = $4010
OAMDMA    = $4014
JOY1      = $4016
APU_FRAME = $4017

BUTTON_A     = $80
BUTTON_B     = $40
BUTTON_LEFT  = $02
BUTTON_RIGHT = $01

; The game's states, in STATE.
PLAYING   = 0
DYING     = 1
CLEAR     = 2
GAME_OVER = 3

; A slug's states, in ENEMY_STATE.
GONE     = 0
WALKING  = 1
SQUASHED = 2

; Speeds and positions, in pixels unless noted.
START_X      = 24
START_Y      = 176     ; standing on the ground, whose top is at 192
WALK_SPEED   = 1       ; per frame
RUN_SPEED    = 2       ; per frame, with B held
JUMP_SPEED   = $FB00   ; -5.0 pixels per frame, 8.8 fixed point
BOUNCE_SPEED = $FD00   ; -3.0, after a stomp
DEATH_HOP    = $FC00   ; -4.0, as the player dies
GRAVITY      = $40     ; 0.25 pixels per frame, added every frame
FALL_LIMIT   = 4       ; whole pixels per frame
PIT_Y        = 208     ; a player this low has fallen into a pit
GOAL_X       = 468     ; reaching this PLAYER_X (the player in the burrow's doorway) clears the level
CAMERA_LEAD  = 120     ; the camera keeps the player this far from its left edge
CAMERA_MAX   = 256     ; the level is two screens wide
WORLD_MAX_X  = 496     ; the player's left edge stays at or below this
DYING_FRAMES = 90
GAME_OVER_FRAMES = 120
START_LIVES  = 2       ; lives after the first, so three tries
SQUASH_FRAMES = 30     ; a stomped slug lies flat this long, then goes
ENEMY_COUNT  = 2
ENEMY_Y      = 176     ; enemies walk on the ground
ENEMY_TOP    = ENEMY_Y + 4  ; the slug's eye stalks start 4 pixels down its tile
STOMP_BELOW  = 172     ; a falling player with PLAYER_Y below this stomps: its
                       ; bottom row is in the slug's top 7 rows (180-186)
HEAD_X       = 8       ; the head point, which bumps blocks: PLAYER_X + 8, PLAYER_Y
PICKUP_X     = 8       ; the pickup point, which takes coins: PLAYER_X + 8, PLAYER_Y + 8
PICKUP_Y     = 8
HUD_Y        = 15      ; the counters' sprites, one pixel above row 16
REDRAW_MAX   = 8

; Metatiles (see metatile_tiles), and their kinds (metatile_kind).
SKY        = 0
USED_BLOCK = 7
KIND_COIN_BLOCK = 2
KIND_COIN  = 5

.segment "HEADER"
    .byte "NES", $1A
    .byte 1             ; 16KB PRG
    .byte 1             ; 8KB CHR
    .byte %00000001     ; mapper 0, vertical mirroring
    .byte %00000000
    .byte 0, 0, 0, 0, 0, 0, 0, 0

.segment "CODE"

reset:
    sei
    cld
    ldx #$40
    stx APU_FRAME       ; no APU frame interrupt
    ldx #$FF
    txs
    inx
    stx PPUCTRL         ; no NMI yet
    stx PPUMASK         ; rendering off
    stx DMC_FREQ
    bit PPUSTATUS
@vblank1:
    bit PPUSTATUS
    bpl @vblank1

    ; Clear RAM, then hide every sprite.
    txa
@clear:
    sta $0000,x
    sta $0100,x
    sta $0200,x
    sta $0300,x
    sta $0400,x
    sta $0500,x
    sta $0600,x
    sta $0700,x
    inx
    bne @clear
    jsr hide_sprites

@vblank2:
    bit PPUSTATUS
    bpl @vblank2

    jsr load_palette
    jsr draw_level
    jsr new_game
    lda #0              ; draw_level drew every cell already
    sta REDRAW_COUNT
    jsr draw_sprites

    bit PPUSTATUS
    lda #0
    sta PPUSCROLL
    sta PPUSCROLL
    lda #%10000000      ; NMI on, nametable $2000, both pattern tables at $0000
    sta PPUCTRL
    lda #%00011110      ; background and sprites, including the left 8 pixels
    sta PPUMASK

; One pass per frame: wait for the NMI, read the controller, update, draw.
main:
    lda NMI_DONE
    beq main
    lda #0
    sta NMI_DONE
    jsr read_pad
    jsr tick_squashed   ; a squashed slug's time runs in every state
    lda STATE
    beq @playing
    cmp #DYING
    beq @dying
    cmp #GAME_OVER
    beq @game_over
    jmp @draw           ; level clear: everything stays where it is
@playing:
    jsr move_player
    jsr move_enemies
    jsr touch_enemies
    jsr follow_player
    jmp @draw
@dying:
    jsr update_dying
    jmp @draw
@game_over:
    dec TIMER
    bne @draw
    jsr new_game
@draw:
    jsr draw_sprites
    jmp main

nmi:
    pha
    txa
    pha
    tya
    pha
    inc FRAMES
    lda #0
    sta OAMADDR
    lda #>OAM
    sta OAMDMA
    jsr redraw_cells
    lda CAMERA_X_HI
    and #1
    ora #%10000000
    sta PPUCTRL
    bit PPUSTATUS
    lda CAMERA_X
    sta PPUSCROLL
    lda #0
    sta PPUSCROLL
    lda #1
    sta NMI_DONE
    pla
    tay
    pla
    tax
    pla
irq:
    rti

; In the NMI: draws the metatiles queued by queue_redraw into the
; nametables, two tiles over two.
redraw_cells:
    bit PPUSTATUS
    ldx #0
@cell:
    cpx REDRAW_COUNT
    beq @done
    lda REDRAW_HI,x
    sta PPUADDR
    lda REDRAW_LO,x
    sta PPUADDR
    lda REDRAW_TILE,x
    asl a
    asl a
    tay
    lda metatile_tiles,y
    sta PPUDATA
    lda metatile_tiles+1,y
    sta PPUDATA
    lda REDRAW_LO,x     ; the row of tiles below: 32 tiles on
    clc
    adc #32
    pha
    lda REDRAW_HI,x
    adc #0
    sta PPUADDR
    pla
    sta PPUADDR
    lda metatile_tiles+2,y
    sta PPUDATA
    lda metatile_tiles+3,y
    sta PPUDATA
    inx
    jmp @cell
@done:
    lda #0
    sta REDRAW_COUNT
    rts

; A new game: three lives and every counter at 0, then the level.
new_game:
    lda #START_LIVES
    sta LIVES
    lda #0
    sta DEATHS
    sta STOMPS
    sta COINS
    jmp start_level

; Puts the player, the enemies and the camera at the start of the level,
; and the level's cells as they were.
start_level:
    lda #START_X
    sta PLAYER_X
    lda #0
    sta PLAYER_X_HI
    sta PLAYER_Y_HI
    sta PLAYER_YSUB
    sta PLAYER_VY
    sta PLAYER_VYSUB
    sta FACING
    sta STATE
    sta CAMERA_X
    sta CAMERA_X_HI
    lda #START_Y
    sta PLAYER_Y
    lda #1
    sta ON_GROUND
    ldx #ENEMY_COUNT - 1
@enemy:
    lda enemy_start_lo,x
    sta ENEMY_X,x
    lda enemy_start_hi,x
    sta ENEMY_X_HI,x
    lda #0
    sta ENEMY_DIR,x
    sta ENEMY_TIMER,x
    lda #WALKING
    sta ENEMY_STATE,x
    dex
    bpl @enemy
    jmp restore_level

; Copies the level from ROM into LEVEL_LEFT and LEVEL_RIGHT, queueing a
; redraw of each cell that changed (a coin taken, a block bumped).
restore_level:
    lda #0
    sta TMP_ROW
@row:
    ldx TMP_ROW
    lda level_rows_lo,x
    sta TMP_PTR
    lda level_rows_hi,x
    sta TMP_PTR+1
    lda TMP_ROW
    asl a
    asl a
    asl a
    asl a
    sta TMP_CELL        ; the row's first cell in each page
    clc
    adc #<LEVEL_LEFT
    sta TMP_DST
    lda #>LEVEL_LEFT
    adc #0
    sta TMP_DST+1
    lda #0
    sta TMP_PAGE
    jsr restore_half
    lda TMP_PTR         ; the row's right half: 16 metatiles on
    clc
    adc #16
    sta TMP_PTR
    lda TMP_PTR+1
    adc #0
    sta TMP_PTR+1
    lda TMP_CELL
    clc
    adc #<LEVEL_RIGHT
    sta TMP_DST
    lda #>LEVEL_RIGHT
    adc #0
    sta TMP_DST+1
    lda #1
    sta TMP_PAGE
    jsr restore_half
    inc TMP_ROW
    lda TMP_ROW
    cmp #15
    bne @row
    rts

; Copies 16 metatiles from TMP_PTR to TMP_DST, the cells TMP_CELL on in
; page TMP_PAGE, queueing a redraw of each that differs.
restore_half:
    ldy #0
@cell:
    lda (TMP_PTR),y
    cmp (TMP_DST),y
    beq @same
    sta (TMP_DST),y
    pha
    tya
    ora TMP_CELL
    tax
    pla
    jsr queue_redraw
@same:
    iny
    cpy #16
    bne @cell
    rts

; Reads controller 1 into PAD, and the newly pressed buttons into PAD_NEW.
read_pad:
    lda PAD
    sta PAD_PREV
    lda #1
    sta JOY1
    lda #0
    sta JOY1
    ldx #8
@bit:
    lda JOY1
    lsr a
    rol PAD
    dex
    bne @bit
    lda PAD_PREV
    eor #$FF
    and PAD
    sta PAD_NEW
    rts

; Runs and jumps, applies gravity, lands or bumps its head, takes coins,
; and checks for a pit or the goal.
move_player:
    lda #WALK_SPEED
    sta TMP_SPEED
    lda PAD
    and #BUTTON_B
    beq @right
    lda #RUN_SPEED
    sta TMP_SPEED
@right:
    lda PAD
    and #BUTTON_RIGHT
    beq @not_right
    lda #0
    sta FACING
@step_right:
    jsr step_right
    dec TMP_SPEED
    bne @step_right
    jmp @jump
@not_right:
    lda PAD
    and #BUTTON_LEFT
    beq @jump
    lda #1
    sta FACING
@step_left:
    jsr step_left
    dec TMP_SPEED
    bne @step_left

@jump:
    lda ON_GROUND
    beq @gravity
    lda PAD_NEW
    and #BUTTON_A
    beq @gravity
    lda #<JUMP_SPEED
    sta PLAYER_VYSUB
    lda #>JUMP_SPEED
    sta PLAYER_VY
    lda PLAYER_Y
    sta JUMP_START_Y
    sta PEAK_Y
    lda #0
    sta ON_GROUND

@gravity:
    jsr apply_gravity
    jsr move_vertically

    lda PLAYER_VY
    bmi @rising
    ; Moving down or not at all: is there ground under the feet?
    lda PLAYER_Y
    clc
    adc #16
    sta TMP_Y
    jsr solid_under_player
    bcc @in_air
    lda TMP_Y
    and #$F0
    sec
    sbc #16
    sta PLAYER_Y
    lda #0
    sta PLAYER_YSUB
    sta PLAYER_VY
    sta PLAYER_VYSUB
    lda #1
    sta ON_GROUND
    jmp @checks
@rising:
    jsr bump_head
@in_air:
    lda #0
    sta ON_GROUND
    lda PLAYER_Y
    cmp PEAK_Y
    bcs @checks
    sta PEAK_Y

@checks:
    jsr take_coin
    lda PLAYER_Y
    cmp #PIT_Y
    bcc @goal
    jmp kill_player
@goal:
    lda PLAYER_X_HI
    beq @done
    lda PLAYER_X
    cmp #<GOAL_X
    bcc @done
    lda #CLEAR
    sta STATE
@done:
    rts

; Adds gravity to the vertical speed, up to the fall limit.
apply_gravity:
    clc
    lda PLAYER_VYSUB
    adc #GRAVITY
    sta PLAYER_VYSUB
    lda PLAYER_VY
    adc #0
    sta PLAYER_VY
    bmi @done           ; still rising
    cmp #FALL_LIMIT
    bcc @done
    lda #FALL_LIMIT
    sta PLAYER_VY
    lda #0
    sta PLAYER_VYSUB
@done:
    rts

; Moves the player by its vertical speed: PLAYER_Y_HI, PLAYER_Y and
; PLAYER_YSUB as one number, the speed sign-extended.
move_vertically:
    clc
    lda PLAYER_YSUB
    adc PLAYER_VYSUB
    sta PLAYER_YSUB
    lda PLAYER_Y
    adc PLAYER_VY
    sta PLAYER_Y
    lda #0
    ldx PLAYER_VY
    bpl @down
    lda #$FF
@down:
    adc PLAYER_Y_HI
    sta PLAYER_Y_HI
    rts

; Rising: the cell at the head point decides. A solid cell ends the rise
; where the player is; a ? block in it becomes a used block and gives a coin.
bump_head:
    lda PLAYER_X
    clc
    adc #HEAD_X
    sta TMP_XL
    lda PLAYER_X_HI
    adc #0
    sta TMP_XH
    lda PLAYER_Y
    sta TMP_Y
    jsr cell_at
    tax
    lda metatile_solid,x
    beq @done           ; open: the rise goes on
    lda #0
    sta PLAYER_VY
    sta PLAYER_VYSUB
    lda metatile_kind,x
    cmp #KIND_COIN_BLOCK
    bne @done
    lda #USED_BLOCK
    jsr set_cell
    inc COINS
@done:
    rts

; A coin in the cell at the pickup point is taken: the cell becomes sky.
take_coin:
    lda PLAYER_X
    clc
    adc #PICKUP_X
    sta TMP_XL
    lda PLAYER_X_HI
    adc #0
    sta TMP_XH
    lda PLAYER_Y
    clc
    adc #PICKUP_Y
    sta TMP_Y
    jsr cell_at
    tax
    lda metatile_kind,x
    cmp #KIND_COIN
    bne @done
    lda #SKY
    jsr set_cell
    inc COINS
@done:
    rts

; One pixel right, unless a block or the end of the level is in the way.
step_right:
    lda PLAYER_X_HI
    beq @inside
    lda PLAYER_X
    cmp #<WORLD_MAX_X
    bcs @blocked
@inside:
    lda PLAYER_X
    clc
    adc #14
    sta TMP_XL
    lda PLAYER_X_HI
    adc #0
    sta TMP_XH
    jsr solid_beside_player
    bcs @blocked
    inc PLAYER_X
    bne @blocked
    inc PLAYER_X_HI
@blocked:
    rts

; One pixel left, unless a block or the start of the level is in the way.
step_left:
    lda PLAYER_X
    ora PLAYER_X_HI
    beq @blocked
    lda PLAYER_X
    clc
    adc #1
    sta TMP_XL
    lda PLAYER_X_HI
    adc #0
    sta TMP_XH
    jsr solid_beside_player
    bcs @blocked
    lda PLAYER_X
    bne @low
    dec PLAYER_X_HI
@low:
    dec PLAYER_X
@blocked:
    rts

; Carry set if the column at TMP_XL/TMP_XH is solid at the player's top or
; bottom row of pixels.
solid_beside_player:
    lda PLAYER_Y
    sta TMP_Y
    jsr solid_at
    bcs @done
    lda PLAYER_Y
    clc
    adc #15
    sta TMP_Y
    jmp solid_at
@done:
    rts

; Carry set if the row TMP_Y is solid under the player's left or right edge
; (PLAYER_X + 2 and PLAYER_X + 13: the player is 12 pixels wide).
solid_under_player:
    lda PLAYER_X
    clc
    adc #2
    sta TMP_XL
    lda PLAYER_X_HI
    adc #0
    sta TMP_XH
    jsr solid_at
    bcs @done
    lda PLAYER_X
    clc
    adc #13
    sta TMP_XL
    lda PLAYER_X_HI
    adc #0
    sta TMP_XH
    jmp solid_at
@done:
    rts

; Carry set if the level pixel at (TMP_XL/TMP_XH, TMP_Y) is in a solid
; metatile.
solid_at:
    jsr cell_at
    tax
    lda metatile_solid,x
    lsr a               ; bit 0 into carry
    rts

; The metatile at the level pixel (TMP_XL/TMP_XH, TMP_Y), in A, from the
; level's cells, with its place in TMP_CELL and TMP_PAGE for set_cell.
; Anything below the screen or past the level is sky.
cell_at:
    lda TMP_Y
    cmp #240
    bcs @open
    lda TMP_XH
    cmp #2
    bcs @open
    sta TMP_PAGE        ; the high byte is the page: 16 columns of 16 pixels
    lda TMP_Y
    and #$F0            ; the row, times 16
    sta TMP_CELL
    lda TMP_XL
    lsr a
    lsr a
    lsr a
    lsr a               ; the column in the page
    ora TMP_CELL
    sta TMP_CELL
    tay
    lda TMP_PAGE
    bne @right
    lda LEVEL_LEFT,y
    rts
@right:
    lda LEVEL_RIGHT,y
    rts
@open:
    lda #SKY
    rts

; Sets the cell cell_at last found to the metatile in A, and queues its redraw.
set_cell:
    ldx TMP_CELL
    ldy TMP_PAGE
    bne @right
    sta LEVEL_LEFT,x
    jmp queue_redraw
@right:
    sta LEVEL_RIGHT,x
    jmp queue_redraw

; Queues metatile A for cell X of page TMP_PAGE, for the next NMI to draw.
; Leaves Y as it was. A full queue drops it.
queue_redraw:
    sta TMP_QTILE
    stx TMP_QCELL
    ldx REDRAW_COUNT
    cpx #REDRAW_MAX
    bcs @full
    lda TMP_QTILE
    sta REDRAW_TILE,x
    ; The nametable address: $2000 (or $2400 for page 1), plus the row
    ; times 64, plus the column times 2.
    lda TMP_QCELL
    and #$0F
    asl a
    sta TMP_QCOL
    lda TMP_QCELL
    and #$F0
    asl a
    asl a
    ora TMP_QCOL
    sta REDRAW_LO,x
    lda TMP_QCELL
    lsr a
    lsr a
    lsr a
    lsr a
    lsr a
    lsr a
    sta TMP_QCOL
    lda TMP_PAGE
    beq @left
    lda #$04
@left:
    ora #$20
    ora TMP_QCOL
    sta REDRAW_HI,x
    inc REDRAW_COUNT
@full:
    rts

; The player dies: it hops up and falls off the screen while nothing else
; moves (update_dying).
kill_player:
    lda #DYING
    sta STATE
    lda #DYING_FRAMES
    sta TIMER
    inc DEATHS
    lda #<DEATH_HOP
    sta PLAYER_VYSUB
    lda #>DEATH_HOP
    sta PLAYER_VY
    lda #0
    sta ON_GROUND
    rts

; Each frame of the dying state: the player falls through everything. When
; the time is up, a life is spent and the level starts over, or with no
; lives left the game is over.
update_dying:
    dec TIMER
    beq @time_up
    jsr apply_gravity
    jmp move_vertically
@time_up:
    lda LIVES
    beq @no_lives
    dec LIVES
    jmp start_level
@no_lives:
    lda #GAME_OVER
    sta STATE
    lda #GAME_OVER_FRAMES
    sta TIMER
    rts

; A squashed slug's time runs out, and it goes.
tick_squashed:
    ldx #0
@enemy:
    lda ENEMY_STATE,x
    cmp #SQUASHED
    bne @next
    dec ENEMY_TIMER,x
    bne @next
    lda #GONE
    sta ENEMY_STATE,x
@next:
    inx
    cpx #ENEMY_COUNT
    bne @enemy
    rts

; Each walking enemy walks one pixel every other frame and turns at the ends
; of its patrol.
move_enemies:
    lda FRAMES
    and #1
    bne @done
    ldx #0
@enemy:
    lda ENEMY_STATE,x
    cmp #WALKING
    bne @next
    lda ENEMY_DIR,x
    bne @left
    inc ENEMY_X,x
    bne @right_end
    inc ENEMY_X_HI,x
@right_end:
    lda ENEMY_X,x
    cmp enemy_max_lo,x
    bne @next
    lda ENEMY_X_HI,x
    cmp enemy_max_hi,x
    bne @next
    lda #1
    sta ENEMY_DIR,x
    jmp @next
@left:
    lda ENEMY_X,x
    bne @left_low
    dec ENEMY_X_HI,x
@left_low:
    dec ENEMY_X,x
    lda ENEMY_X,x
    cmp enemy_start_lo,x
    bne @next
    lda ENEMY_X_HI,x
    cmp enemy_start_hi,x
    bne @next
    lda #0
    sta ENEMY_DIR,x
@next:
    inx
    cpx #ENEMY_COUNT
    bne @enemy
@done:
    rts

; A player touching a walking enemy squashes it when falling onto its top,
; and dies otherwise.
touch_enemies:
    lda STATE
    bne @done           ; already fell into a pit or reached the goal
    ldx #0
@enemy:
    lda ENEMY_STATE,x
    cmp #WALKING
    bne @next
    lda PLAYER_Y
    cmp #ENEMY_TOP - 15
    bcc @next
    cmp #ENEMY_Y + 16
    bcs @next
    lda PLAYER_X
    sec
    sbc ENEMY_X,x
    sta TMP_XL
    lda PLAYER_X_HI
    sbc ENEMY_X_HI,x
    beq @right_of
    cmp #$FF
    bne @next
    lda TMP_XL
    cmp #256 - 11       ; player at most 11 pixels left of the enemy
    bcc @next
    jmp @touch
@right_of:
    lda TMP_XL
    cmp #12             ; player at most 11 pixels right of the enemy
    bcs @next
@touch:
    lda PLAYER_VY
    bmi @hurt
    ora PLAYER_VYSUB
    beq @hurt
    lda PLAYER_Y
    cmp #STOMP_BELOW
    bcs @hurt
    lda #SQUASHED
    sta ENEMY_STATE,x
    lda #SQUASH_FRAMES
    sta ENEMY_TIMER,x
    lda #<BOUNCE_SPEED
    sta PLAYER_VYSUB
    lda #>BOUNCE_SPEED
    sta PLAYER_VY
    inc STOMPS
@next:
    inx
    cpx #ENEMY_COUNT
    bne @enemy
@done:
    rts
@hurt:
    jmp kill_player

; The camera keeps the player CAMERA_LEAD pixels from the screen's left edge,
; within the level.
follow_player:
    lda PLAYER_X
    sec
    sbc #CAMERA_LEAD
    sta CAMERA_X
    lda PLAYER_X_HI
    sbc #0
    sta CAMERA_X_HI
    bcs @inside
    lda #0              ; before the start of the level
    sta CAMERA_X
    sta CAMERA_X_HI
    rts
@inside:
    beq @done
    lda CAMERA_X
    beq @done
    lda #<CAMERA_MAX    ; past the end of the level
    sta CAMERA_X
    lda #>CAMERA_MAX
    sta CAMERA_X_HI
@done:
    rts

hide_sprites:
    lda #$FF
    ldx #0
@sprite:
    sta OAM,x
    inx
    bne @sprite
    rts

; Writes the player, the enemies and the counters into the sprite table.
draw_sprites:
    jsr hide_sprites
    ldy #0
    ; The player: flipped when facing left, upside down while dying, and
    ; not drawn once it is below the screen.
    lda PLAYER_Y_HI
    bne @enemies
    lda PLAYER_Y
    cmp #239
    bcs @enemies
    lda PLAYER_X
    sec
    sbc CAMERA_X
    sta TMP_SX
    ldx PLAYER_Y
    dex
    stx TMP_SY
    lda #$10
    sta TMP_TILE
    lda #0
    ldx FACING
    beq @facing_right
    lda #%01000000
@facing_right:
    ldx STATE
    cpx #DYING
    bne @upright
    ora #%10000000
@upright:
    sta TMP_ATTR
    jsr draw_metasprite

@enemies:
    lda #0
    sta TMP_ROW         ; enemy index; draw_metasprite uses X
@enemy:
    ldx TMP_ROW
    lda ENEMY_STATE,x
    beq @next           ; gone
    lda ENEMY_X,x
    sec
    sbc CAMERA_X
    sta TMP_SX
    lda ENEMY_X_HI,x
    sbc CAMERA_X_HI
    bne @next           ; off screen
    lda TMP_SX
    cmp #241
    bcs @next           ; would wrap around the right edge
    lda #ENEMY_Y - 1
    sta TMP_SY
    lda ENEMY_STATE,x
    cmp #SQUASHED
    beq @flat
    lda #$14            ; walking
    jmp @tile
@flat:
    lda #$24            ; squashed flat
@tile:
    sta TMP_TILE
    lda ENEMY_DIR,x
    beq @enemy_right
    lda #%01000000      ; walking left: flipped
@enemy_right:
    ora #%00000001      ; sprite palette 1
    sta TMP_ATTR
    jsr draw_metasprite
@next:
    inc TMP_ROW
    lda TMP_ROW
    cmp #ENEMY_COUNT
    bne @enemy
    jmp draw_hud

; The counters along the top of the screen: a coin and the coins taken,
; then a life and the lives left. Y is where the sprite table goes on.
draw_hud:
    lda #16
    sta TMP_SX
    lda #$3A            ; coin
    jsr hud_sprite
    lda COINS
    jsr hud_number
    lda #200
    sta TMP_SX
    lda #$3C            ; life
    jsr hud_sprite
    lda #$3B            ; x
    jsr hud_sprite
    lda LIVES
    jmp hud_number

; Two digits of A, from 0 to 99, at TMP_SX on.
hud_number:
    ldx #0
@tens:
    cmp #10
    bcc @ones
    sbc #10
    inx
    jmp @tens
@ones:
    pha
    txa
    clc
    adc #$30            ; the digits' tiles start at $30
    jsr hud_sprite
    pla
    clc
    adc #$30
    jmp hud_sprite

; An 8x8 sprite of tile A at (TMP_SX, HUD_Y), into the sprite table at Y;
; then Y and TMP_SX move on by one sprite.
hud_sprite:
    sta OAM+1,y
    lda #HUD_Y
    sta OAM,y
    lda #0
    sta OAM+2,y
    lda TMP_SX
    sta OAM+3,y
    clc
    adc #8
    sta TMP_SX
    iny
    iny
    iny
    iny
    rts

; A 16x16 sprite from four tiles (TMP_TILE, +1 for the right half, +2 for
; the bottom half) at TMP_SX, TMP_SY with attributes TMP_ATTR, into the
; sprite table from offset Y. Leaves Y after the last sprite written.
draw_metasprite:
    lda TMP_ATTR
    and #%01000000      ; horizontal flip swaps left and right tiles
    beq @no_hflip
    lda #1
@no_hflip:
    sta TMP_MASK
    lda TMP_ATTR
    and #%10000000      ; vertical flip swaps top and bottom tiles
    beq @no_vflip
    lda #2
    ora TMP_MASK
    sta TMP_MASK
@no_vflip:
    ldx #0
@sprite:
    txa
    and #2
    asl a
    asl a
    clc
    adc TMP_SY
    sta OAM,y
    txa
    eor TMP_MASK
    clc
    adc TMP_TILE
    sta OAM+1,y
    lda TMP_ATTR
    sta OAM+2,y
    txa
    and #1
    asl a
    asl a
    asl a
    clc
    adc TMP_SX
    sta OAM+3,y
    iny
    iny
    iny
    iny
    inx
    cpx #4
    bne @sprite
    rts

load_palette:
    bit PPUSTATUS
    lda #$3F
    sta PPUADDR
    lda #$00
    sta PPUADDR
    ldx #0
@color:
    lda palette,x
    sta PPUDATA
    inx
    cpx #32
    bne @color
    rts

; Draws the whole level once, from ROM: metatile columns 0-15 into the
; nametable at $2000 and 16-31 into $2400. After that only the cells that
; change are drawn again (redraw_cells); scrolling only moves the view.
draw_level:
    lda #$20
    sta TMP_NT
    lda #0
    sta TMP_COL
@nametable:
    bit PPUSTATUS
    lda TMP_NT
    sta PPUADDR
    lda #0
    sta PPUADDR
    sta TMP_ROW
@row:
    ldx TMP_ROW
    lda level_rows_lo,x
    sta TMP_PTR
    lda level_rows_hi,x
    sta TMP_PTR+1
    lda #0
    sta TMP_HALF
@half:
    ldy TMP_COL
    lda #16
    sta TMP_COUNT
@metatile:
    lda (TMP_PTR),y
    asl a
    asl a
    ora TMP_HALF
    tax
    lda metatile_tiles,x
    sta PPUDATA
    lda metatile_tiles+1,x
    sta PPUDATA
    iny
    dec TMP_COUNT
    bne @metatile
    lda TMP_HALF
    bne @next_row
    lda #2
    sta TMP_HALF
    jmp @half
@next_row:
    inc TMP_ROW
    lda TMP_ROW
    cmp #15
    bne @row
    ; Attribute table: background palette 0 everywhere.
    lda #0
    ldx #64
@attribute:
    sta PPUDATA
    dex
    bne @attribute
    lda TMP_NT
    cmp #$24
    beq @done
    lda #$24
    sta TMP_NT
    lda #16
    sta TMP_COL
    jmp @nametable
@done:
    rts

.segment "RODATA"

palette:
    ; background: sky, moss green, soil brown, tan
    .byte $21, $1A, $17, $37
    .byte $21, $1A, $17, $37
    .byte $21, $1A, $17, $37
    .byte $21, $1A, $17, $37
    ; sprites: the player (outline, moss, eye), the slug (outline, body, eyes)
    .byte $21, $0F, $2A, $30
    .byte $21, $0F, $28, $30
    .byte $21, $0F, $2A, $30
    .byte $21, $0F, $2A, $30

; Metatiles, 16x16 pixels: four tile numbers each (top left, top right,
; bottom left, bottom right), whether the player can stand on them, and
; their kind: 2 a block with a coin in it, 5 a coin, 0 anything else.
;   0 sky, 1 moss over soil, 2 soil, 3 stone block, 4 burrow left,
;   5 burrow right, 6 ? block, 7 used block, 8 coin
metatile_tiles:
    .byte $00, $00, $00, $00
    .byte $01, $02, $03, $04
    .byte $03, $04, $04, $03
    .byte $05, $06, $07, $08
    .byte $00, $09, $00, $0B
    .byte $0A, $00, $0C, $00
    .byte $18, $19, $1A, $1B
    .byte $1C, $1D, $1E, $1F
    .byte $20, $21, $22, $23
metatile_solid:
    .byte 0, 1, 1, 1, 0, 0, 1, 1, 0
metatile_kind:
    .byte 0, 0, 0, 0, 0, 0, 2, 0, 5

; The level, 32 metatiles wide and 15 high: two screens. One string per row:
; '.' sky, 'M' moss, 'S' soil, 'B' block, '(' and ')' the burrow, '?' a ?
; block, 'o' a coin.
.macro level_row text
    .assert .strlen(text) = 32, error, "level rows are 32 metatiles wide"
    .repeat 32, i
        .byte (.strat(text, i) = 'M') * 1 + (.strat(text, i) = 'S') * 2 + (.strat(text, i) = 'B') * 3 + (.strat(text, i) = '(') * 4 + (.strat(text, i) = ')') * 5 + (.strat(text, i) = '?') * 6 + (.strat(text, i) = 'o') * 8
    .endrepeat
.endmacro

level:
row0:  level_row "................................"
row1:  level_row "................................"
row2:  level_row "................................"
row3:  level_row "................................"
row4:  level_row "................................"
row5:  level_row "................................"
row6:  level_row "................................"
row7:  level_row "................................"
row8:  level_row "....?..........................."
row9:  level_row ".................ooo............"
row10: level_row "............oo...BBB.....B......"
row11: level_row "......B..................B...()."
row12: level_row "MMMMMMMMMMMM..MMMMMMMMMMMMMMMMMM"
row13: level_row "SSSSSSSSSSSS..SSSSSSSSSSSSSSSSSS"
row14: level_row "SSSSSSSSSSSS..SSSSSSSSSSSSSSSSSS"

level_rows_lo:
    .byte <row0, <row1, <row2, <row3, <row4, <row5, <row6, <row7
    .byte <row8, <row9, <row10, <row11, <row12, <row13, <row14
level_rows_hi:
    .byte >row0, >row1, >row2, >row3, >row4, >row5, >row6, >row7
    .byte >row8, >row9, >row10, >row11, >row12, >row13, >row14

; Enemy patrols: each walks right from its start to its end and back.
enemy_start_lo: .byte <128, <256
enemy_start_hi: .byte >128, >256
enemy_max_lo:   .byte <176, <352
enemy_max_hi:   .byte >176, >352

.segment "VECTORS"
    .word nmi, reset, irq

.include "chr.s"
