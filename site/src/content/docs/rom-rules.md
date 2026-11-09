---
title: ROM rules
description: What may and may not happen with a ROM when you use or contribute to Game Printer.
---

A commercial game's ROM belongs to its publisher. Game Printer is built so that its data never leaves your machine.

## The rules

- Use a copy you're entitled to: an official release that gives you the file, or a dump of a cartridge you own.
- Keep ROMs on your own machine. Never commit one, upload one, or add one to hosted CI.
- The same goes for anything made from a ROM: lifted art, memory dumps, recordings of frames, and ROM bytes written out as text.
- Never bypass the commit hook with `--no-verify`.
- Don't ask for a ROM, offer one, or link to one in an issue or pull request. We can't help you find one.

## What can be shared

| Shared | Stays on your machine |
|---|---|
| The model, as text | The ROM |
| Button presses for clips | Art and sound lifted from it |
| Addresses of where things are in a ROM | Memory and frame recordings |
| Hashes that identify the ROM | Per-frame score output |

## What enforces them

| Check | When | What it refuses |
|---|---|---|
| The commit hook | Every commit, after `just setup` | ROM files, ROM bytes in any form it can recognise, game images |
| `gp package scan` | Before a package is published | Any file holding data from a ROM the package names |
| The registry | On publish | Any file holding 32 or more bytes in a row from a named ROM |

The checks are a backstop. Following the rules is still on you.
