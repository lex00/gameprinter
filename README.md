# Game Printer

gameprinter is the tool repo of Game Printer. It turns a game you own into a model: text files that say how the game behaves, plus hook code for what text can't say. The model is checked by playing a set of recorded clips, the corpus, on the original and on the model and comparing them frame by frame. Then it is published as a package that a web page plays from the visitor's own ROM.

It came from game-printer, the old one-repo tool. The games moved to their own repo, `lex00/games`, and the Bevy generator, `gp loop` and the other games' code were left behind.

Today there are two games: Donkey Kong (NES), playable at [nes-donkey-kong.intentius.workers.dev](https://nes-donkey-kong.intentius.workers.dev) from a ROM you load, and Tapper (arcade, MAME's `rbtapper`), whose model is in progress. Moss Hop, a small NES game written for testing, lives here and is what the checks run on.

## What's in the repo

| Folder | What |
|---|---|
| `crates/` | 38 Rust crates: the `gp` command, the NES and arcade emulators, the model, simulation and interpreter, packages, the browser builds |
| `wit/` | The runtime protocol and a game module's interface |
| `mechanic/`, `editor/` | The page `gp serve` hosts |
| `scripts/web/` | The public page's build, checks and deploy |
| `games/mosshop`, `templates/mosshop`, `baselines/` | Moss Hop's source, model and recorded runs |
| `third_party/` | The vendored NES emulator core |
| `docs/`, `site/` | Docs, and the docs site |

[ARCHITECTURE.md](ARCHITECTURE.md) has the crates, the data flow and where state lives. [ROADMAP.md](ROADMAP.md) has what's done and what's left.

## Build and check

You need rustup (the repo pins Rust 1.99.1 and installs it on the first build), `just`, cc65 to build Moss Hop, and on Linux bubblewrap to build a game's module.

```sh
just setup     # once per clone: the ROM-blocking git hook and the locked cargo
just check     # what every change has to pass; needs no ROM
just publish   # an optimized gp in ~/.gameprinter/bin
export PATH="$HOME/.gameprinter/bin:$PATH"
gp --help
```

`just check` builds gp, assembles Moss Hop and runs formatting, clippy, the tests and the Moss Hop checks. Changes land on `main` only through `gp tool gate` ([docs/gate.md](docs/gate.md)).

ROMs never go in a repo. They live in `$GP_ROM_DIR` (default `~/.gameprinter/roms`), and the hook refuses them. Read the [ROM rules](site/src/content/docs/rom-rules.md) before you use one.

## Where the docs are

The docs site's pages, in `site/src/content/docs/`, are for people using the tool:

| Page | For |
|---|---|
| [install.md](site/src/content/docs/install.md) | Building `gp` |
| [bring.md](site/src/content/docs/bring.md) | Bringing in a game from a ROM you own |
| [improve.md](site/src/content/docs/improve.md) | Changing a published game and sending the change |
| [make.md](site/src/content/docs/make.md) | Making your own game from Moss Hop |
| [themes.md](site/src/content/docs/themes.md) | Themes and levels |
| [registry.md](site/src/content/docs/registry.md) | Running a registry of your own |
| [contribute.md](site/src/content/docs/contribute.md) | Changing the tool |
| [rom-rules.md](site/src/content/docs/rom-rules.md) | What may and may not happen with a ROM |

The files in [docs/](docs/) are for contributors and agents. Start with [CLI.md](CLI.md), the list of every `gp` command, generated from the code; [docs/workers.md](docs/workers.md) and [docs/gate.md](docs/gate.md) for taking a task and landing it; and [docs/modules.md](docs/modules.md), [docs/web.md](docs/web.md) and [docs/mechanic-page.md](docs/mechanic-page.md) for the page and game modules.

## Licence

The code is licensed under [Apache-2.0](LICENSE). Moss Hop is the only game in it; other games, themes and ROMs live elsewhere.
