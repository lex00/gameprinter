---
title: Install gp
description: Download the gp command, check it, connect your AI agent to it, or build it from source.
---

`gp` is the command-line tool behind Game Printer. Playing a game takes a browser and nothing installed; a ported game such as Donkey Kong also asks for your own ROM of it, which stays in your browser. You need `gp` to make something.

A released `gp` is one file that carries the pages and files it needs, so making a theme takes no copy of the gameprinter repo and no Rust.

## Download it

A release has an archive for each of these:

| Machine | Archive name ends in |
|---|---|
| Mac with Apple silicon (M1 and later) | `aarch64-apple-darwin.tar.gz` |
| Linux on an Intel or AMD 64-bit processor | `x86_64-unknown-linux-gnu.tar.gz` |
| Linux on a 64-bit ARM processor | `aarch64-unknown-linux-gnu.tar.gz` |
| Windows 10 or 11, 64-bit Intel or AMD | `x86_64-pc-windows-msvc.zip` |

A release built before the Linux and Windows builds existed has only the Mac one; on another machine, [build it from source](#build-it-from-source). The Linux builds are made on Ubuntu 24.04, so they may not start on a Linux older than that (one with a glibc before 2.39). `uname -m` prints `x86_64` or `aarch64` to say which Linux one you need.

### With Homebrew

```sh
brew install lex00/tap/gp
gp --version
```

This is for a Mac; the Homebrew formula doesn't have the Linux builds yet. Homebrew checks the archive's checksum itself and puts `gp` at `/opt/homebrew/bin/gp`; that is the path to give an agent below. `brew upgrade gp` takes a newer release. The rest of this section is the same install by hand.

### By hand, on a Mac

Each archive comes with its checksum, the same name ending in `.sha256`. Download the archive, `gp-VERSION-COMMIT-aarch64-apple-darwin.tar.gz`, and its checksum into the same folder, then in Terminal, in that folder:

```sh
shasum -a 256 -c gp-*-aarch64-apple-darwin.tar.gz.sha256
```

It must print the archive's name and `OK`. A `FAILED` means the file isn't the one that was published, so delete it and don't run it.

Then unpack it and put `gp` in a folder on your PATH:

```sh
tar -xzf gp-*-aarch64-apple-darwin.tar.gz
mkdir -p ~/.local/bin
mv gp-*-aarch64-apple-darwin/gp ~/.local/bin/gp
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc
export PATH="$HOME/.local/bin:$PATH"
gp --version
```

The archive also holds `LICENSE` (gp's, Apache-2.0) and `THIRD-PARTY.md`, the licences of the crates gp is built from.

### What a Mac says

This `gp` isn't signed with an Apple developer certificate yet. A file a browser downloaded is marked as downloaded, and macOS refuses to run an unsigned one the first time: Terminal prints `killed` and a window says Apple could not verify that "gp" is free of malware. Once the checksum says `OK`, you can tell macOS you trust this file, in either of two ways:

- Open System Settings and go to Privacy & Security. Near the bottom is a line about "gp" with an Open Anyway button. Choose it, and run `gp --version` again.
- Or in Terminal: `xattr -d com.apple.quarantine ~/.local/bin/gp`

A file downloaded with `curl` in Terminal isn't marked, so macOS doesn't ask:

```sh
curl -fLO https://github.com/lex00/gameprinter/releases/download/TAG/gp-VERSION-COMMIT-aarch64-apple-darwin.tar.gz
curl -fLO https://github.com/lex00/gameprinter/releases/download/TAG/gp-VERSION-COMMIT-aarch64-apple-darwin.tar.gz.sha256
shasum -a 256 -c gp-*-aarch64-apple-darwin.tar.gz.sha256
```

The checksum is still the step that matters: it is how you know the file is the one published.

### On Linux

Download the archive for your processor and its `.sha256` into one folder, then in a terminal there (with `aarch64` in place of `x86_64` on ARM):

```sh
sha256sum -c gp-*-x86_64-unknown-linux-gnu.tar.gz.sha256
tar -xzf gp-*-x86_64-unknown-linux-gnu.tar.gz
mkdir -p ~/.local/bin
mv gp-*-x86_64-unknown-linux-gnu/gp ~/.local/bin/gp
gp --version
```

`sha256sum` answers `OK` after the archive's name when the file is the one published. Most Linux systems already have `~/.local/bin` on the PATH once it exists; if `gp --version` says the command isn't found, add `export PATH="$HOME/.local/bin:$PATH"` to `~/.bashrc` (or your shell's startup file) and open a new terminal. Linux doesn't ask about unsigned programs the way a Mac does.

### On Windows

Download `gp-VERSION-COMMIT-x86_64-pc-windows-msvc.zip` and its `.sha256` into one folder, such as Downloads. Open PowerShell there. Print the archive's checksum and compare it with the first word of the `.sha256` file:

```powershell
(Get-FileHash .\gp-*-x86_64-pc-windows-msvc.zip -Algorithm SHA256).Hash
Get-Content .\gp-*-x86_64-pc-windows-msvc.zip.sha256
```

The two must be the same string of letters and digits (PowerShell prints it in capitals, the file in small letters; that doesn't matter). If they differ, delete the file and don't run it.

Unpack the zip and add a folder for `gp.exe` to your PATH:

```powershell
Expand-Archive .\gp-*-x86_64-pc-windows-msvc.zip -DestinationPath .
New-Item -ItemType Directory -Force "$env:LOCALAPPDATA\Programs\gp"
Move-Item .\gp-*-x86_64-pc-windows-msvc\gp.exe "$env:LOCALAPPDATA\Programs\gp\gp.exe"
[Environment]::SetEnvironmentVariable("Path", [Environment]::GetEnvironmentVariable("Path", "User") + ";$env:LOCALAPPDATA\Programs\gp", "User")
```

A PowerShell window opened after this finds `gp`, which `gp --version` shows. An agent needs gp by its full path, `C:\Users\YOU\AppData\Local\Programs\gp\gp.exe`.

This `gp.exe` isn't signed either. The first time it runs, Windows may show a blue "Windows protected your PC" window (SmartScreen): choose More info, then Run anyway, once the checksum matched.

The Windows build is new: the release workflow builds it and runs `gp self check` on it, but no one has made a theme with it yet. It should run what a theme needs (fetching a game, `gp theme look check`, `gp theme look play` in your browser), `gp mcp` for an agent, and `gp self check`. Fetching uses the `curl.exe` that Windows 10 and 11 have. Building a game's own code (`gp game build`) doesn't: gp has no build sandbox for Windows yet, and `gp self check` says so. Commands that run the repo's build scripts (building the public page yourself, `gp game share` for a game with its own pictures) need a checkout and Git Bash. Your home folder is `C:\Users\YOU\.gameprinter`, and `GP_HOME` and `GP_ROM_DIR` work as on the other systems.

## Check what works

```sh
gp self check
```

For each thing `gp` reads or runs, it says whether this machine has it. A missing piece comes with what needs it and how to get it. The last lines say what this machine is ready for (a theme, a game from a template, a port). Nothing is installed or changed. This table lists the pieces it looks at.

| Piece | What needs it | Making a theme needs it? |
|---|---|---|
| Your ROM of the game, in `~/.gameprinter/roms` or `GP_ROM_DIR` | `gp theme look check` names the game's pictures from it and checks your files hold nothing of it; the page plays from it | Yes |
| curl | Reading the public registry: `gp game fetch`, `gp theme look play` | Yes (macOS and Windows have it) |
| A games folder (`GP_GAMES_DIR`) | Naming a game by name, such as `--game donkey-kong`; `gp serve` | No: give the game's folder instead |
| git, ssh-keygen | Publishing to a registry | Only to publish |
| Rust (rustup, the release gp names, and the `wasm32-unknown-unknown` target) | Building a game's own code: `gp game build`, and any command that opens a game with code on your machine | No: the page takes a published game's code from the registry |
| The build sandbox (part of macOS; bubblewrap on Linux; none on Windows yet) | `gp game build` | No |
| Node and a headless browser | `gp theme look play --shots`, pictures of your theme in the page | No: playing your theme in a browser doesn't need them |
| Docker with the MAME image | Recording an arcade game in MAME, for a port | No |

`gp self check` prints the exact `rustup` line to run when Rust is what's missing.

## Get a game to work on

A theme is checked against the game's folder. Put your ROM of the game in `~/.gameprinter/roms` first, then fetch the game from the public registry, outside any git repo:

```sh
mkdir -p ~/.gameprinter/roms
gp game fetch lex00/donkey-kong@1.0.9 \
  --from https://raw.githubusercontent.com/lex00/gp-registry/main \
  --into ~/games/donkey-kong --rom-dir ~/.gameprinter/roms
```

gp reads the package's id from the version's `id` line in the registry's [index](https://github.com/lex00/gp-registry/tree/main/index/lex00/donkey-kong), checks every file against it, and prints the id it used. Leave off `@1.0.9` to get the catalog's latest version. To pin an exact package, give its id with `--id`. Then [start a theme](../themes/) with `--game ~/games/donkey-kong`, or set `GP_GAMES_DIR=~/games` and use `--game donkey-kong`.

## Connect your AI agent

`gp mcp` gives an agent Game Printer's tools over MCP. The agent starts it itself; you only tell the agent where `gp` is. Use the full path to `gp` (`~/.local/bin/gp` above is `/Users/YOU/.local/bin/gp`), since an agent app doesn't read your shell's PATH. Outside the gameprinter repo, `gp mcp` uses the templates the released `gp` carries.

### Claude Code

Run this once, from any folder:

```sh
claude mcp add gp --scope user -- /Users/YOU/.local/bin/gp mcp
```

Then `/mcp` in Claude Code lists `gp` and its tools.

### Claude Desktop

In Claude Desktop's settings, the Developer page has Edit Config, which opens `claude_desktop_config.json` (on a Mac it is in `~/Library/Application Support/Claude/`, on Windows in `%APPDATA%\Claude\`). Add `gp` under `mcpServers` and restart Claude Desktop:

```json
{
  "mcpServers": {
    "gp": {
      "command": "/Users/YOU/.local/bin/gp",
      "args": ["mcp"],
      "env": { "GP_ROM_DIR": "/Users/YOU/roms" }
    }
  }
}
```

The `env` line is only needed when your ROMs aren't in `~/.gameprinter/roms`. On Windows, write the paths with doubled backslashes, as JSON needs: `"C:\\Users\\YOU\\AppData\\Local\\Programs\\gp\\gp.exe"`.

### Another MCP client

`gp mcp` is a local server over stdio (no network port). Give the client the command `/Users/YOU/.local/bin/gp` with the one argument `mcp`, and name the server `gp`; its tools then appear as `gp`'s. More about the tools is in the repo's `docs/mcp.md`.

## Build it from source

To change `gp` itself, or on a machine there's no download for yet.

| Tool | Why |
|---|---|
| [rustup](https://rustup.rs) | Installs the exact Rust version the repo names, on the first build |
| [just](https://github.com/casey/just) | Runs the repo's recipes |
| cc65 (Homebrew or apt) | Only to build Moss Hop, the test game |
| bubblewrap, on Linux | Only to build the custom code some games have (Donkey Kong does, Moss Hop doesn't), in a sandbox with no network |

```sh
git clone https://github.com/lex00/gameprinter
cd gameprinter
just setup
just publish
export PATH="$HOME/.gameprinter/bin:$PATH"
gp --help
```

`just setup` turns on the hook that refuses a commit holding ROM data. `just publish` builds an optimized `gp` and puts it in `~/.gameprinter/bin`. Add the `export` line to your shell's startup file to keep it. A `gp` built this way reads the page's files from the checkout; `just release` builds one that carries them, as a download does. It's built and tested on macOS. It compiles for Linux and Windows, and the release workflow builds and checks it on both, but day-to-day work happens on a Mac. On Windows, run the repo's recipes from Git Bash; `gp game build` there has no sandbox and refuses.

## Where your files go

| Setting | Default | Holds |
|---|---|---|
| `GP_HOME` | `~/.gameprinter` | Built tools and working files, and the files a released `gp` unpacks the first time it needs them |
| `GP_ROM_DIR` | `~/.gameprinter/roms` | Your own ROMs, read by hash whatever they're called |
| `GP_GAMES_DIR` | none | A folder of game folders, so a game can be named by its name |

Read the [ROM rules](../rom-rules/) before you use a ROM with the tool.
