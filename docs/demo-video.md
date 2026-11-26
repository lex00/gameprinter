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

## Text

A title card, a banner under a stage's name, a line about the original: each is a picture (a PNG drawn with a bitmap font, no smoothing) laid over the video with ffmpeg's `overlay` for a span of time. An ffmpeg built without freetype has no `drawtext`, and pictures keep the letters' edges hard either way.
