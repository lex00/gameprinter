---
title: Install gp
description: Start here. Install gp and its plugin for Claude Code and port a game from your own ROM with /gp:port; other agents connect by hand.
---

You don't need anything installed to play; a browser is enough, plus your own ROM for a ported game such as Donkey Kong. To make something, you need `gp`, Game Printer's command-line tool.

A released `gp` is a single file with the pages and files it needs built in. You can make a theme without a copy of the gameprinter repo and without Rust.

## Start here

Four steps take you from nothing to porting a game from a ROM you own.

1. Install `gp`. On a Mac or Linux, Homebrew is the short way:

   ```sh
   brew install lex00/tap/gp
   gp --version
   ```

   With no Homebrew, [download a release](#download-it) instead. No release has a Windows build yet; on Windows, [build it from source](#build-it-from-source).

2. Add Game Printer's plugin to [Claude Code](https://claude.com/claude-code) by typing these two lines into a Claude Code session:

   ```sh
   /plugin marketplace add lex00/gameprinter
   /plugin install gp@gameprinter
   ```

   That gives the session gp's tools and skills along with the `/gp:port` command. The plugin finds `gp` by itself, so you never type its path. Everyone who ports a ROM gets this same setup, and two people porting one ROM end up with the same model.

3. Start the port with your own ROM:

   ```sh
   /gp:port ~/Downloads/my-game.nes
   ```

   The command copies the ROM into your ROM folder (`~/.gameprinter/roms`) and starts the game's folder. Running `/gp:port my-game` again later picks up from the next step. The ROM stays on your machine; the [ROM rules](../rom-rules/) say what may happen with it.

4. Open the page. The agent runs `gp mechanic start` when it needs you to play or watch. That starts `gp serve` and opens its page at `http://127.0.0.1:7710/` in your browser. On that page you record clips of the original and watch the model beside it. A landing or a paid call shows up there as a card and waits for your YES.

[Bring in a game](../bring/) goes through every step of a port. `/plugin update gp` and `brew upgrade gp` keep the two up to date, and a new session says when their versions differ.

Other agents connect to `gp mcp` [by hand](#connect-your-ai-agent), as Claude Desktop does. [AGENTS.md](https://github.com/lex00/gameprinter/blob/main/AGENTS.md) gives them the plugin's rules in words.

## Download it

A [release](https://github.com/lex00/gameprinter/releases) has an archive for each of these, and `SHA256SUMS`, the checksum of each:

| Machine | Archive |
|---|---|
| Mac with Apple silicon (M1 and later) | `gp-aarch64-apple-darwin.tar.gz` |
| Mac with an Intel processor | `gp-x86_64-apple-darwin.tar.gz` |
| Linux on an Intel or AMD 64-bit processor | `gp-x86_64-unknown-linux-gnu.tar.gz` |
| Linux on a 64-bit ARM processor | `gp-aarch64-unknown-linux-gnu.tar.gz` |

Windows has no archive yet. Releases since 0.1.2 leave it out, because the release tool can't yet read its settings from a Windows path; when it can, `gp-x86_64-pc-windows-msvc.zip` joins the table.

The first two releases, from before these names, have only the Apple silicon Mac archive, named `gp-VERSION-COMMIT-aarch64-apple-darwin.tar.gz` with its own `.sha256` file; for one of those on another machine, [build it from source](#build-it-from-source). The Linux builds are made on Ubuntu 24.04 and may not start on an older Linux (one with a glibc before 2.39). Run `uname -m` to see which Linux build you need: it prints `x86_64` or `aarch64`.

### With Homebrew

```sh
brew install lex00/tap/gp
gp --version
```

Homebrew installs it on a Mac or on Linux, checks the archive's checksum for you, and puts `gp` at `/opt/homebrew/bin/gp` on an Apple silicon Mac (`/usr/local/bin/gp` on an Intel one, `/home/linuxbrew/.linuxbrew/bin/gp` on Linux), which is the path you'll give an agent below. `brew upgrade gp` gets a newer release. The rest of this section does the same install by hand.

### By hand, on a Mac

Download the archive, `gp-aarch64-apple-darwin.tar.gz` (`gp-x86_64-apple-darwin.tar.gz` on an Intel Mac), and `SHA256SUMS` into the same folder, then in Terminal, in that folder:

```sh
shasum -a 256 -c --ignore-missing SHA256SUMS
```

It should print the archive's name and `OK`. If it says `FAILED`, the file isn't the one that was published: delete it and don't run it.

Then unpack it and put `gp` in a folder on your PATH:

```sh
tar -xzf gp-aarch64-apple-darwin.tar.gz
mkdir -p ~/.local/bin
mv gp-*-aarch64-apple-darwin/gp ~/.local/bin/gp
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc
export PATH="$HOME/.local/bin:$PATH"
gp --version
```

The archive also holds `LICENSE` (gp's, Apache-2.0) and `THIRD-PARTY.md`, the licences of the crates gp is built from.

### What a Mac says

This `gp` isn't signed with an Apple developer certificate yet. macOS marks files your browser downloads, and the first time you run an unsigned one it refuses. Terminal prints `killed`, and a window says Apple could not verify that "gp" is free of malware. Once the checksum says `OK`, there are two ways to tell macOS you trust the file:

- Open System Settings and go to Privacy & Security. Near the bottom is a line about "gp" with an Open Anyway button. Choose it, and run `gp --version` again.
- Or in Terminal: `xattr -d com.apple.quarantine ~/.local/bin/gp`

Files downloaded with `curl` in Terminal aren't marked, and macOS won't ask about them:

```sh
curl -fLO https://github.com/lex00/gameprinter/releases/latest/download/gp-aarch64-apple-darwin.tar.gz
curl -fLO https://github.com/lex00/gameprinter/releases/latest/download/SHA256SUMS
shasum -a 256 -c --ignore-missing SHA256SUMS
```

Don't skip the checksum, though: it's the last line above.

### On Linux

Download the archive for your processor and `SHA256SUMS` into one folder, then in a terminal there (with `aarch64` in place of `x86_64` on ARM):

```sh
sha256sum -c --ignore-missing SHA256SUMS
tar -xzf gp-x86_64-unknown-linux-gnu.tar.gz
mkdir -p ~/.local/bin
mv gp-*-x86_64-unknown-linux-gnu/gp ~/.local/bin/gp
gp --version
```

`sha256sum` prints `OK` after the archive's name when the file is the one that was published. Most Linux systems put `~/.local/bin` on the PATH once it exists. If `gp --version` says the command isn't found, add `export PATH="$HOME/.local/bin:$PATH"` to `~/.bashrc` (or your shell's startup file) and open a new terminal. Linux doesn't ask about unsigned programs the way a Mac does.

### On Windows

These steps wait on the first release with a zip ([Download it](#download-it) says why). Download `gp-x86_64-pc-windows-msvc.zip` and `SHA256SUMS` into one folder, such as Downloads. Open PowerShell there. Print the archive's checksum and the line of `SHA256SUMS` for it:

```powershell
(Get-FileHash .\gp-x86_64-pc-windows-msvc.zip -Algorithm SHA256).Hash
Select-String windows .\SHA256SUMS
```

The first should be the same string of letters and digits as the start of the line. PowerShell prints it in capitals and the file has it in lower case, which doesn't matter. If they differ, delete the file and don't run it.

Unpack the zip and add a folder for `gp.exe` to your PATH:

```powershell
Expand-Archive .\gp-x86_64-pc-windows-msvc.zip -DestinationPath .
New-Item -ItemType Directory -Force "$env:LOCALAPPDATA\Programs\gp"
Move-Item .\gp-*-x86_64-pc-windows-msvc\gp.exe "$env:LOCALAPPDATA\Programs\gp\gp.exe"
[Environment]::SetEnvironmentVariable("Path", [Environment]::GetEnvironmentVariable("Path", "User") + ";$env:LOCALAPPDATA\Programs\gp", "User")
```

Any PowerShell window you open after this finds `gp`; try `gp --version`. An agent needs the full path, `C:\Users\YOU\AppData\Local\Programs\gp\gp.exe`.

This `gp.exe` isn't signed either. The first time it runs, Windows may show a blue "Windows protected your PC" window (SmartScreen). If the checksum matched, choose More info, then Run anyway.

Nobody has made a theme with a Windows gp yet. It should handle what a theme needs (fetching a game, `gp theme look check`, `gp theme look play` in your browser), `gp mcp` for an agent, and `gp self check`. Fetching uses the `curl.exe` that comes with Windows 10 and 11. Building a game's own code (`gp game build`) doesn't work yet, because gp has no build sandbox for Windows, and `gp self check` tells you so. Commands that run the repo's build scripts, such as building the public page yourself or `gp game share` for a game with its own pictures, need a checkout and Git Bash. Your home folder is `C:\Users\YOU\.gameprinter`, and `GP_HOME` and `GP_ROM_DIR` work as on the other systems.

## Check what works

```sh
gp self check
```

This goes through everything `gp` reads or runs and says whether your machine has it. For anything missing, it says what needs it and how to get it. The last lines tell you what the machine is ready for: a theme, a game from a template, or a port. It installs and changes nothing. These are the pieces it looks at:

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

Themes are checked against the game's folder, so you'll want a copy of it. Put your ROM of the game in `~/.gameprinter/roms` first, then fetch the game from the public registry into a folder outside any git repo:

```sh
mkdir -p ~/.gameprinter/roms
gp game fetch lex00/donkey-kong@1.0.9 \
  --from https://raw.githubusercontent.com/lex00/gp-registry/main \
  --into ~/games/donkey-kong --rom-dir ~/.gameprinter/roms
```

gp reads the package's id from the version's `id` line in the registry's [index](https://github.com/lex00/gp-registry/tree/main/index/lex00/donkey-kong), checks every file against it, and prints the id it used. Leave off `@1.0.9` for the catalog's latest version, or pin an exact package by giving its id with `--id`. Then [start a theme](../themes/) with `--game ~/games/donkey-kong`, or set `GP_GAMES_DIR=~/games` and use `--game donkey-kong`.

## Connect your AI agent

In Claude Code, the gp plugin ([Start here](#start-here)) connects `gp` for you, and this section isn't needed. For any other agent, `gp mcp` gives it Game Printer's tools over MCP. The agent starts it; all you do is tell the agent where `gp` is. Give the full path (`~/.local/bin/gp` above is `/Users/YOU/.local/bin/gp`), because agent apps don't read your shell's PATH. Outside the gameprinter repo, `gp mcp` uses the templates built into the released `gp`. Have the agent read [AGENTS.md](https://github.com/lex00/gameprinter/blob/main/AGENTS.md) as well, since it holds the rules the plugin's hooks keep in Claude Code.

### Claude Code without the plugin

If you'd rather add only the MCP server to Claude Code, run this once, from any folder:

```sh
claude mcp add gp --scope user -- /Users/YOU/.local/bin/gp mcp
```

Then `/mcp` in Claude Code lists `gp` and its tools. This way has no `/gp:port` and no hooks.

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

`gp mcp` is a local server over stdio, with no network port. Give the client the command `/Users/YOU/.local/bin/gp` with the single argument `mcp`, and name the server `gp`, and its tools show up under `gp`. The repo's `docs/mcp.md` has more about the tools.

### What to ask your agent next

- Porting a game from your own ROM starts with `/gp:port` in Claude Code, or with [Bring in a game](../bring/) in another agent.
- To make a theme: [Themes and levels](../themes/) has a prompt to hand your agent.
- To put your game or theme online: [Host your game or theme](../host-your-own/) has one too. For the upload your agent uses wrangler, which is Cloudflare's own tool. It asks you to log in once.

### Hints

A command that is a step of a workflow says so first, on stderr. The hint names the step and the skill or MCP tool behind it, then what usually comes next. `gp theme sheet` starts with this:

```
hint: gp theme sheet belongs to the theme procedure, step 5 of 9, grids and prompts (skill theme-generation; MCP next). Next: paid art after the person's yes (step 6), then gp theme fit (step 7).
```

The workflows are the skills in the repo's `.claude/skills` and the pages on this site. stdout and `--json` output don't change. Set `GP_NO_HINTS=1` to turn the hints off.

## Build it from source

Do this if you want to change `gp` itself, or if there's no download for your machine yet.

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

`just setup` turns on the hook that refuses any commit holding ROM data. `just publish` builds an optimized `gp` and puts it in `~/.gameprinter/bin`; add the `export` line to your shell's startup file to keep it on your PATH. A `gp` built this way reads the page's files from the checkout. `just release-build OUT` builds one with them built in, like a download. Releases are made by the release workflow, which only the owner starts ([docs/release.md](https://github.com/lex00/gameprinter/blob/main/docs/release.md)).

Day-to-day work happens on a Mac, where it's built and tested. It also compiles for Linux and Windows, and the release workflow builds and checks the Linux ones. On Windows, run the repo's recipes from Git Bash. `gp game build` refuses there, since it has no sandbox.

## Where your files go

| Setting | Default | Holds |
|---|---|---|
| `GP_HOME` | `~/.gameprinter` | Built tools and working files, and the files a released `gp` unpacks the first time it needs them |
| `GP_ROM_DIR` | `~/.gameprinter/roms` | Your own ROMs, read by hash whatever they're called |
| `GP_GAMES_DIR` | none | A folder of game folders, so a game can be named by its name |
| `GP_NO_HINTS` | unset | Set to 1 to stop the workflow hint a command prints first on stderr |

Read the [ROM rules](../rom-rules/) before you use a ROM with the tool.
