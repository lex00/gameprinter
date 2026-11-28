# Game Printer

Game Printer lets you create original games that run in your browser with nothing to install. It has powerful tools to help you port mechanics from your old ROMs and breathe new life into old games. Most people use it by talking to an AI agent, which runs the command-line tool, `gp`, for them. This repo holds that tool and its code, and the docs for people using it are on [the docs site](https://lex00.github.io/gameprinter/).

Each game's rules and numbers live in `model.toml`, a TOML file that's read when the game loads and never compiled. Some games add custom code (Rust hooks in the game's `ext/` crate) for what the model can't say; Donkey Kong has some and Moss Hop has none. A game's folder with a `template.toml`, its model and no ROM data is a template, which anyone can copy to start a game. Every ported game's folder in the games checkout is one, and so is each starter under `templates/`.

To check a port, the tools play its corpus (a set of recorded clips) through both the original and the model and compare them frame by frame. Then the port is published to a registry. A web page plays it and takes the art and levels from the visitor's own ROM, inside their browser. It never makes a new ROM.

The repo grew out of game-printer, the older tool that kept everything in one repo. The games moved to their own repo, `lex00/games`, and the Bevy generator, `gp loop` and the other games' code stayed behind.

Two games have been ported so far. Donkey Kong (NES) plays at [gameprinter.intentius.workers.dev](https://gameprinter.intentius.workers.dev) from a ROM you load, and Tapper (arcade, MAME's `rbtapper`) has a model in progress. The checks run on Moss Hop, a small game we wrote in NES assembly for testing, which lives in this repo.

## What's in the repo

| Folder | What |
|---|---|
| `crates/` | 42 Rust crates: the `gp` command, the NES and arcade emulators, the model, simulation and interpreter, packages, the browser builds |
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

`just check` builds gp, assembles Moss Hop and runs formatting, clippy, the tests and the Moss Hop checks.

The tool knows no game by name, and none of its recipes reads a game's files. The checks of a game against its ROM are the games repo's: its justfile runs them from a game's folder with an installed gp (or `GP=path`), each saying plainly when a ROM is missing: `just check-game` (the game's model checked and in canonical form, `gp corpus score` with every clip exact, each MAME capture its `oracle/mame-captures.toml` names on gp-arcade, the arcade player against the native replay, the model in the page's player against the native run), or its check-all recipe, run at its top, for every game. The few of those that run a script or example of this repo find it through `GAMEPRINTER` (else `../gameprinter`); `just play-arcade-wasm` here builds the arcade player and its native examples for them. What is left here of the old by-hand checks needs no game: `just web-page check` (the public page, played headless), `just page-tests` (its `share.test.mjs` and `theme-line.test.mjs`) and `gp baseline check`.

Changes land on `main` only through `gp tool gate` ([docs/gate.md](docs/gate.md)).

ROMs never go in a repo. They live in `$GP_ROM_DIR` (`~/.gameprinter/roms` by default), and the git hook refuses them. Read the [ROM rules](site/src/content/docs/rom-rules.md) before you use one.

## Where the docs are

The docs site's pages, in `site/src/content/docs/`, are for people using the tool:

| Page | For |
|---|---|
| [install.md](site/src/content/docs/install.md) | Start here: installing `gp` and the gp plugin, `/gp:port`, connecting another agent, building `gp` |
| [bring.md](site/src/content/docs/bring.md) | Bringing in a game from a ROM you own |
| [improve.md](site/src/content/docs/improve.md) | Changing a published game and sending the change |
| [make.md](site/src/content/docs/make.md) | Making your own game, from Moss Hop or from nothing |
| [themes.md](site/src/content/docs/themes.md) | Themes and levels |
| [registry.md](site/src/content/docs/registry.md) | Running a registry of your own |
| [host-your-own.md](site/src/content/docs/host-your-own.md) | Hosting your own registry on GitHub and your own copy of the page ([docs/host-your-own.md](docs/host-your-own.md) has the details) |
| [contribute.md](site/src/content/docs/contribute.md) | Changing the tool |
| [rom-rules.md](site/src/content/docs/rom-rules.md) | What may and may not happen with a ROM |

The files in [docs/](docs/) are for contributors and agents. A good place to start is [CLI.md](CLI.md), every `gp` command, generated from the code. For taking a task and landing it, read [docs/workers.md](docs/workers.md) and [docs/gate.md](docs/gate.md); everyone who ports a ROM gets the same agent setup from the gp plugin for Claude Code ([docs/plugin.md](docs/plugin.md)), while other harnesses read the same tools and rules in [AGENTS.md](AGENTS.md). For the page and game modules, read [docs/modules.md](docs/modules.md), [docs/web.md](docs/web.md) and [docs/mechanic-page.md](docs/mechanic-page.md).

## Licence

The code is licensed under [Apache-2.0](LICENSE). Moss Hop is the only game in it; other games, themes and ROMs live elsewhere.
