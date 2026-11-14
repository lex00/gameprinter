# Capture scorecard

> Brought from game-printer with wave 9 of the port. gameprinter keeps commercial games in the games checkout, so a GAME is a folder, an id under `templates/` or a game there (`gp scorecard donkey-kong`). Its hooks live in each game's module, so `--module` (with `--assets`) gives the module the mechanics column reads; without it, gp's built-in hooks, and a game's hooks show as debt. An arcade game's asset map names graphics regions, so its PRG/CHR column reads `gfx1 1024 gfx2 256` for Tapper. Every `gp` run is recorded again (gp-metrics, [run manifests](formats/run-manifest.md)), but the commands that fill the run columns (gaps, sound, track, validate, propose, chunks) don't record their `--metrics` numbers here yet, so those columns say "needs ROM" or "no run yet" until they do. The `just` recipes below are game-printer's; gameprinter has none for the scorecard yet.

The scorecard answers one question: how much of each game has capture found so far, and how much of that can be trusted. It has one row per game under `templates/` and one column per thing capture produces. It's Milestone 2 work ([the roadmap](../ROADMAP.md)).

## Run it

```sh
just scorecard                 # gp scorecard --record, then gp metrics report; prints the page's path
gp scorecard                   # the table, no metrics written
gp scorecard mosshop smb1-ntsc-prg0 --json
gp scorecard mosshop --recording 3f2a1c9e --record   # also run gp model propose on a store recording
```

The page is `$GP_HOME/reports/metrics.html`, the metrics report with a Scorecard section after the metric table. The section has the table, then one drop-down per game. Each drop-down holds every recorded number with its trend over the last 30 recorded runs as a small line. It also lists the hooks that are debt and the facts that differ from a measurement. Last come the slots and the presentation kinds the model uses, and the top items of each place `gp gaps` found (read from `$GP_HOME/reports/gaps/<template>/`). The page is plain HTML with CSS and inline SVG, and it loads nothing from the network. `gp metrics report` computes the static columns from the `templates/` folder in the current directory; elsewhere it draws the table the last `--record` saved in `$GP_HOME/reports/scorecard.json`.

`--record` writes each number as the metric `scorecard.<template>.<column>`, so `gp metrics report` has history to draw. Run it after a capture step and the trend shows the step's effect. Nothing needs a ROM: the static columns come from files in the repo, and the run columns say what is missing instead of failing.

## Columns

Every column's cell carries its source as a tooltip on the page. A cell that has no run behind it says `needs ROM` (a game whose template is not open tier, so its runs need your own copy) or `no run yet` (an open game nobody has run since the metric was added). `not built` marks a command this build does not have.

| Column | Meaning | Source |
|---|---|---|
| PRG/CHR | Share of the ROM's PRG and CHR bytes the asset map covers, or the bytes when the source is not registered (Moss Hop, the originals). The numbers are `assets.prg_bytes` and `assets.chr_bytes` | `assets/<source>.toml` through gp-assets ([assets](assets.md)), totals from `sources/` |
| assets | Assets the map names (`assets.count`) | the same map |
| slots | Slots the model declares, how many an asset of the map fills (`assets.slots`, `assets.slots_filled`, and `assets.slots.<kind>` per slot kind), and how many `gp template check` found ready (`assets.slots_ready`) | `model.toml`; the metric `template.<id>.slots_ready` |
| art | Slots that are `original`, `stub` or `replaced` (`assets.art.<status>`) | `slots.toml` in `$GP_HOME/work/assets/<id>/` ([stubs](stubs.md)); "no slots.toml" until a stub or a placement wrote one |
| mechanics | Mechanics the model uses and how many are general components (`mechanics.used`, `mechanics.components`) | `gp model mechanics` ([mechanics](mechanics.md)) |
| hooks | Distinct `ext` hooks, plus hooks with no record (`mechanics.hooks`) | the same catalog, `Ext::describe` |
| debt | Hooks whose general mechanic no component runs yet (`mechanics.debt`). The page names them | the same catalog |
| paths m/p/c/pr | Model paths by class: mechanic, parameter, content, presentation (`mechanics.paths.<class>`) | the same catalog |
| present kinds | The presentation kinds the model uses, by name (`present.kind.<name>` counts the paths of each). A kind is the part of a presentation path after its owner: `entities.player.sprite.tiles` is `sprite`, and an asset slot is its asset kind. A new kind the catalog classes as presentation (animation, composite sprites, HUD, viewport) shows up with no change to the scorecard | the catalog's path classes |
| facts | Facts in the model (`facts.total`) | `model.toml` and its evidence table, as `gp model check` counts them |
| by source | Facts by evidence source: stated, inferred, observed, and those with none (`facts.source.<name>`, `facts.with_evidence`) | the same |
| measured | Facts `gp model propose` measured: agree and differ (`facts.compared`, `facts.agree`, `facts.differ`). The latest recorded run, or a run now with `--recording`, which also names the facts that differ | metrics `propose.<id>.compared` and `.agree` |
| derived | Facts `gp model derive` produced (`facts.derived`). Reads "not built" while the command is not in this build; the scorecard looks for it in the command tree at run time | metric `derive.<id>.derived` |
| validate | The latest `gp validate` result per runtime: pass, fail or unexercised, with the three counts (`fidelity.<runtime>.passed`, `.failed`, `.unexercised`) | metrics `validate.<id>.checks.*` (interpreter) and `validate.<id>.<runtime>.checks.*` |
| track P/R | The mean precision and recall of `gp track` against the game's mapping, over the recordings scored (`fidelity.track.precision`, `.recall`) | metrics `track.<id>.<recording>.precision` and `.recall` |
| gaps changes | Items `gp gaps changes` found per place: RAM, sprites (oam), background, sound and code (`gaps.<place>.count`) | metrics `gaps.<id>.<place>.count` from `gp gaps` ([gaps](gaps.md)) |
| gaps interactive | Interactive element kinds in the catalog (`gaps.interactive.count`) | metric `gaps.<id>.interactive.count` |
| chunks | Level chunk kinds by layer (objects, groups, gaps, patterns, compounds) and the mean share of each level's structure cells inside a known chunk (`chunks.kinds`, `chunks.<layer>`, `chunks.coverage`) | metrics `chunks.<id>.*` from `gp chunks` ([chunks](chunks.md)) |
| gaps drawn | Entity kinds the model draws with a systematic offset from the original's sprites, of the kinds checked (`gaps.drawn.count`) | metrics `gaps.<id>.drawn.count` and `.drawn.kinds` |
| sprites | Sprite capture's coverage: frames in the sprite set, model states with frames of all states, and table frames never seen in a recording (`sprites.frames`, `.states_covered`, `.states`, `.unobserved`) | metrics `sprites.<template>.*` from `gp sprites capture` ([capture](capture.md#sprites)) |
| sound captured | Songs and sound effects captured, over every capture of the template (`sound.songs`, `sound.sfx`) | metrics `sound.<id>.<capture>.songs` and `.sfx` from `gp sound capture --template` ([sound](sound.md)) |
| sound agreement | The captured songs' and effects' note onset F1 against their register logs, the mean over captures (`sound.f1`, `sound.sfx_f1`); warn below 0.95 | metrics `sound.<id>.<capture>.f1` and `.sfx_f1` |

`<id>` is the template's folder name, and the model's name is tried too (`mm2` for `mm2-usa`). A metric under neither name is not found.

## How capture raises each column

Capture fills the model and the asset map, and the columns follow from the files; nothing is entered by hand.

- PRG/CHR, assets and slots rise as the asset map names more of the ROM (`gp assets lift` and `pack --check` prove each addition byte for byte).
- art moves from `original` to `stub` or `replaced` when `gp assets stub` or `assets_place` writes `slots.toml`.
- debt falls when a hook's behaviour becomes a component, because the hook's `like` then names a mechanic a component runs. Each fix for one game is a general mechanic another game can use, so the number counts progress for every future game.
- present kinds grows when the model gains presentation primitives, and each new kind appears under its own name.
- facts and by source change as `gp model derive`, `gp model propose` and `gp model refine` write facts with evidence. The share with evidence should stay at all of them.
- measured improves as recordings with traces exist for the game; more recordings give more facts a measurement can check.
- sound rises as `gp sound capture` runs on more of a game's recordings (`just sound`, ROM tier), and its agreement as capture's note and effect rules improve.
- gaps falls as the model takes in what `gp gaps` finds (`just gaps`, ROM tier): a labelled byte, routine or sound effect the model cites leaves its list, and a kind drawn where the original draws it leaves the drawn count.
- chunks grows as `gp chunks` (`just chunks`) reads more of a game's levels, and coverage rises as the catalog explains more of their cells.
- validate and track rise as `just validate-*` and `just track-*` run with `--metrics`, and the commercial games need the ROM tier (`just check-roms`) for those.

## Limits

- The run columns show the latest value of a metric, whichever machine and commit recorded it. A trend mixes runs from different commits; the report's commit column on the main metric table says which.
- `gp scorecard --recording` runs `gp model propose` once per template shown with the same recordings, so it fits one template at a time.
- Per-asset-kind counts and the facts that differ are known only for the run that computed them. The page keeps the last `--record` run's detail in `scorecard.json`.
