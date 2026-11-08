; Moss Hop: RAM layout. Every variable has a fixed address, so recordings,
; baselines and selectors such as ram[0x10] can name it. labels.toml lists
; the same names and addresses for tools, and a test in gp-cli checks that
; the two agree. docs/reference-game.md describes what each one means.

FRAMES       = $00 ; NMI count, wraps at 256
NMI_DONE     = $01 ; set by the NMI handler, cleared by the main loop
PAD          = $02 ; controller 1 this frame: A B Select Start Up Down Left Right, A in bit 7
PAD_PREV     = $03 ; controller 1 the frame before
PAD_NEW      = $04 ; buttons down this frame and up the frame before
STATE        = $05 ; 0 playing, 1 dying, 2 level clear, 3 game over
TIMER        = $06 ; frames left in the dying or game over state
DEATHS       = $07 ; deaths since the game started
STOMPS       = $08 ; enemies stomped since the game started
COINS        = $09 ; coins taken since the game started
LIVES        = $0A ; lives left after this one

PLAYER_X     = $10 ; player left edge in the level, pixels, low byte
PLAYER_X_HI  = $11 ; high byte (0 or 1: the level is 512 pixels wide)
PLAYER_Y     = $12 ; player top edge, pixels from the top of the screen, low byte
PLAYER_YSUB  = $13 ; fraction of a pixel, in 256ths
PLAYER_VY    = $14 ; vertical speed, whole pixels per frame, signed (down is positive)
PLAYER_VYSUB = $15 ; fraction of the speed, in 256ths
ON_GROUND    = $16 ; 1 while standing on something solid
FACING       = $17 ; 0 right, 1 left
JUMP_START_Y = $18 ; PLAYER_Y when the last jump started
PEAK_Y       = $19 ; smallest PLAYER_Y since the last jump started
CAMERA_X     = $1A ; left edge of the screen in the level, low byte
CAMERA_X_HI  = $1B ; high byte
PLAYER_Y_HI  = $1C ; high byte of PLAYER_Y: 1 once a dying player falls below the screen

ENEMY_X      = $20 ; two enemies: low bytes of their left edges ($20, $21)
ENEMY_X_HI   = $22 ; high bytes ($22, $23)
ENEMY_DIR    = $24 ; 0 moving right, 1 moving left ($24, $25)
ENEMY_STATE  = $26 ; 0 gone, 1 walking, 2 squashed ($26, $27)
ENEMY_TIMER  = $28 ; frames left squashed ($28, $29)

; Scratch, valid only inside one routine.
TMP_XL       = $30
TMP_XH       = $31
TMP_Y        = $32
TMP_COL      = $33
TMP_PTR      = $34 ; two bytes
TMP_SPEED    = $36
TMP_SX       = $37
TMP_SY       = $38
TMP_TILE     = $39
TMP_ATTR     = $3A
TMP_MASK     = $3B
TMP_ROW      = $3C
TMP_HALF     = $3D
TMP_COUNT    = $3E
TMP_NT       = $3F
TMP_CELL     = $40 ; a level cell's index in its page: row * 16 + column
TMP_PAGE     = $41 ; its page: 0 for columns 0-15, 1 for 16-31
TMP_DST      = $42 ; two bytes
TMP_QTILE    = $44
TMP_QCELL    = $45
TMP_QCOL     = $46

; Metatiles to draw again in the next NMI, after a cell of the level
; changes: up to REDRAW_MAX of them, each a nametable address and a metatile.
REDRAW_COUNT = $50
REDRAW_HI    = $51 ; eight bytes
REDRAW_LO    = $59 ; eight bytes
REDRAW_TILE  = $61 ; eight bytes

OAM          = $0200 ; sprite table, copied to the PPU by DMA in every NMI
LEVEL_LEFT   = $0300 ; the level's cells, columns 0-15: 15 rows of 16 metatiles
LEVEL_RIGHT  = $03F0 ; columns 16-31, the same way
