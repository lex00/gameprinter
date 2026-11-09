---
title: Install gp
description: Build the gp command from source on a Mac or Linux machine.
---

`gp` is the command-line tool behind Game Printer. You only need it to change or make something. Playing a game takes a browser and nothing else.

There are no downloads yet, so you build it from source. It's built and tested on macOS and Linux. Windows hasn't been tried.

## What you need

| Tool | Why |
|---|---|
| [rustup](https://rustup.rs) | Installs the exact Rust version the repo names, on the first build |
| [just](https://github.com/casey/just) | Runs the repo's recipes |
| cc65 (Homebrew or apt) | Only to build Moss Hop, the test game |
| bubblewrap, on Linux | Only to build a game's module, in a sandbox with no network |

## Build it

```sh
git clone https://github.com/lex00/gameprinter
cd gameprinter
just setup
just publish
export PATH="$HOME/.gameprinter/bin:$PATH"
gp --help
```

`just setup` turns on the hook that refuses a commit holding ROM data. `just publish` builds an optimized `gp` and puts it in `~/.gameprinter/bin`. Add the `export` line to your shell's startup file to keep it.

## Where your files go

| Setting | Default | Holds |
|---|---|---|
| `GP_HOME` | `~/.gameprinter` | Built tools and working files |
| `GP_ROM_DIR` | `~/.gameprinter/roms` | Your own ROMs, read by hash whatever they're called |

Read the [ROM rules](../rom-rules/) before you use a ROM with the tool.
