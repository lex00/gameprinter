---
title: Contribute to the tool
description: Change gp itself, check it, and open a pull request.
---

The tool lives in [`lex00/gameprinter`](https://github.com/lex00/gameprinter), and changes come in as pull requests. If you want to change a game rather than the tool, see [Improve a game](../improve/).

## Make a change

```sh
gh repo fork lex00/gameprinter --clone
cd gameprinter
just setup
just check
```

Every change has to pass `just check`. It builds the tool, runs the tests and checks everything against Moss Hop, our own test game. It needs no ROM. You'll need [cc65](../install/) for it.

Run it again after your change, then open a pull request. If you changed a `gp` command, regenerate `CLI.md` with `gp self docs`.

## How changes land

Everything reaches `main` through a gate. A maintainer runs your patch through `gp tool gate`. It builds the tool and runs `just check` and the full test suite. `just check` plays Moss Hop, a small game built from source, through the new tool and holds it to its recorded baselines, so a change that breaks how games run fails there. It needs no ROM.

You can run it yourself:

```sh
git diff > /tmp/fix.patch
git checkout -- .
gp tool gate --patch /tmp/fix.patch --onto main --dry-run -m "what you changed"
```

Without a ROM, the Donkey Kong step says so and passes without running, and a maintainer runs it for you.

A patch can't change the commit hook or the ROM checks in `.githooks/` and `scripts/romguard/`. Only the owner edits those.

[The gates](https://github.com/lex00/gameprinter/blob/main/docs/gate.md) has the details.
