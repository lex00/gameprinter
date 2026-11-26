# A demo video of a theme

A short video of a game in a theme, with its sound, made by a script from the clips the game's corpus already holds: nobody plays it live. The owner asked for it on 2026-11-25: "i need the ability to record a demo video with sound", about 20 seconds of play on each stage set and one special scene, with a few seconds of the original beside the theme and a fade from the one to the other.

The video shows the person's ROM's pictures and plays its sound. It is theirs to publish or not, and it is never committed.

## Recording a clip

```
node scripts/web/demo-record.mjs http://127.0.0.1:7796/mechanic/GAME/screen.html CLIP FROM TO PICTURES OUT.webm
```

`CLIP` is a corpus clip's id, `FROM` and `TO` its frames (0 at its start), `PICTURES` a theme's id or `game`. `gp serve` must be running, with the person's ROM chosen on its page.

The script opens the screen page with `?work=CLIP&from=FROM&to=TO&pictures=PICTURES&view=side`. That link option sets the corpus clip up on the screens, its frames played in a loop, as a gate card's LOOK AT IT does, and sets `document.body.dataset.work` to the clip's id once it is up. The script then draws the original's screen and the model's into one canvas, the original on the left, waits for the loop to come round to its first frame, and records one pass with `MediaRecorder`, the sound tapped where the page sends it to the speakers.

Both screens are in each picture of the one file, so they can't drift apart. Everything a demo shows of a clip is cut from it:

| Shot | From the file |
|---|---|
| The theme alone | its right half (`crop=iw/2:ih:iw/2:0`) |
| The original beside the theme | the whole picture |
| A fade from the one to the other and back | the halves blended, the share a function of time (`blend=all_expr='A*(0.5+0.5*sin(PI*T/2))+B*(0.5-0.5*sin(PI*T/2))'`) |
| A wipe | `blend=all_expr='if(lt(X,W*(0.5+0.4*sin(PI*T/2))),A,B)'` |

A recording made in a browser has uneven time stamps: bring each to a steady rate first (`-vf fps=30,settb=AVTB -af aresample=async=1`).

A corpus clip often starts just after the sound of its scene does (a round's tune begins as the round is set up, a few frames before the clip's first). `GP_LEAD=N` before the command takes the clip from N frames before its start, as far back as its play reaches, and FROM and TO then count from there; the link option is `&lead=N`. A TO past the clip's end stops at its last frame.

## Cutting the video

```
python3 -I scripts/web/demo-cut.py PLAN.toml OUT.mp4
```

The plan lists the video's parts in order, and the script makes each with ffmpeg and joins them (1920 by 1080, 30 frames a second, unless the plan's `size` and `fps` say otherwise). It needs Pillow.

```toml
letters = "GAME/themes/NAME/theme/art/font"   # letter-A.png, letter-0.png, letter-x021.png
title_height = 96
letter_height = 32

[[part]]
kind = "card"
length = 7
lines = ["A TITLE", "A LINE UNDER IT.", { plain = "example.org/address" }]

[[part]]
kind = "shot"
file = "saloon.webm"     # a recording, beside the plan
show = "theme"           # theme, original, both, fade or wipe
start = 0
length = 8
```

A card's first line is large and the rest small, in the theme's own letters. A line written `{ plain = "..." }` is drawn as typed in a font file (`plain_font`, Menlo unless the plan names one): an address needs its small letters and its slash, which a theme's lettering may not have. With no `ground` in the plan, the frame takes the colour of the letters' own tile.

A shot's `swing` is the seconds a fade or a wipe takes to go from the theme to the original and back. A shot can also carry `[[part.bubble]]` tables (`at`, `for`, `x`, `y`, `text`, `tail`): a few words that pop up over the picture with a blip. One bubble on a shot works; two on one shot fail in ffmpeg and are not fixed yet.

## Text

A title card, a banner under a stage's name, a line about the original: each is a picture (a PNG drawn with a bitmap font, no smoothing) laid over the video with ffmpeg's `overlay` for a span of time. An ffmpeg built without freetype has no `drawtext`, and pictures keep the letters' edges hard either way.
