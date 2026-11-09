---
title: Contribute to the tool
description: Change gp itself, check it, and open a pull request.
---

The tool is in [`lex00/gameprinter`](https://github.com/lex00/gameprinter). Changes arrive as pull requests. To change a game instead, see [Improve a game](../improve/).

## Make a change

```sh
gh repo fork lex00/gameprinter --clone
cd gameprinter
just setup
just check
```

`just check` is what every change has to pass. It builds the tool, runs the tests and checks everything against Moss Hop, so it needs no ROM from anyone. You'll need [cc65](../install/) for it.

Run the same check again after your change, then open a pull request. Commands in `gp` that change also change `CLI.md`: regenerate it with `gp self docs`.

## How changes land

Nothing lands on `main` except through a gate. A maintainer runs your patch through `gp tool gate`, which builds the tool, runs `just check` and the full test suite, then plays Donkey Kong's whole corpus through the old and new tool to confirm that no clip got worse.

You can run it yourself:

```sh
git diff > /tmp/fix.patch
git checkout -- .
gp tool gate --patch /tmp/fix.patch --onto main --dry-run -m "what you changed"
```

Without a ROM, the Donkey Kong step says so and passes without running. A maintainer runs it for you.

A patch can't change the commit hook or the ROM checks in `.githooks/` and `scripts/romguard/`. Only the owner edits those.

[The gates](https://github.com/lex00/gameprinter/blob/main/docs/gate.md) has the full detail.
