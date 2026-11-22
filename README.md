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
just check-all # just check, then the checks that need ROMs, the games checkout or the network
just publish   # an optimized gp in ~/.gameprinter/bin
export PATH="$HOME/.gameprinter/bin:$PATH"
gp --help
```

`just check` builds gp, assembles Moss Hop and runs formatting, clippy, the tests and the Moss Hop checks.

`just check-all` (scripts/check/all.sh, most of an hour) runs `just check` and then the checks below, each one only when what it needs is on the machine. The tool knows no game by name, so it finds the games in the games checkout (`$GP_GAMES_DIR`, else `../games`) and reads what to check from each game's own files: each arcade game's model in the page's player against the native run (`just web-parity-arcade GAME`, step `web-parity-arcade-GAME`), the public page built and played headless with its lift's tests (`just web-page check`, which needs the games checkout beside the repo and the network) and its `share.test.mjs` and `theme-line.test.mjs`, `gp corpus score` on each game with a `corpus.toml` with every clip exact (`score-GAME`), each MAME capture a game's `oracle/mame-captures.toml` names on every frame (`mame-NAME`, gp-arcade's compare example, built in release), and `gp baseline check`. A check it can't run gets one line naming what's missing (`check-all: skipped score-GAME: no ROM GAME's corpus is for ...`), and with no games checkout the game checks are skipped in words. It exits 1 only when a check that ran failed; each check's output is in `target/check-all/`, and `CHECK_ALL_DRY=1` prints what it would run.

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
