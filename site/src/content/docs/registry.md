---
title: Run a registry
description: Publish games and themes to a registry of your own, in a folder or a git repo.
---

A registry holds published games and themes as plain static files, and you can run one on anything that serves files. Each published version is a package of kind `game` or `content` (a theme). It holds the folder's files, each named by a hash of its bytes, and a list of them.

## Make one and publish to it

```sh
gp registry init ~/my-registry --name my-games --namespace me
gp registry publish ~/my-registry path/to/my-theme \
  --name me/my-theme --version 1.0.0 --kind content \
  --title "My Theme" --for lex00/donkey-kong
gp registry reindex ~/my-registry me/my-theme@1.0.0
gp registry check ~/my-registry
```

`reindex` rewrites the lists readers use to find things. With `--keep`, or with no versions at all, it starts from what's listed now and adds the versions you name. Without `--keep` you name every version that should stay listed, and leaving one out is refused unless `--drop` names it. A package's last version becomes its latest.

## Namespaces, withdrawals and what the page offers

| Command | What it does |
|---|---|
| `gp registry grant TARGET NAMESPACE` | Lets packages named `NAMESPACE/...` install from the registry. Without it the page lists such a theme and its install fails |
| `gp registry grants TARGET` | Lists the namespaces granted |
| `gp registry yank TARGET NAME@VERSION --severity security --reason TEXT` | Withdraws a version: the page leaves a `security` one out and refuses to install it, and still offers a `notice` one with the reason shown. `--undo` takes it back |
| `gp registry check TARGET --for GAME [--theme NAME]` | Reads the game's theme list as the page does: each theme it offers, installed as it installs it, and each it leaves out with why. With `--theme`, whether that one is offered and if not why, exiting 1 when it isn't. A theme whose last recheck failed says when and why |
| `gp theme recheck GAME NAME --registry TARGET` | Reads a theme the registry offers for the game again, as the page reads it, over your ROM of the game, and records the day, pass or fail, and why in the theme's line of the game's list (`recheck`, signed with the list). The page still offers a theme whose recheck failed and says why. A reindex keeps it |

The page calls a theme by the part of its package name after the namespace. That part has to start with a lowercase letter and use only lowercase letters, digits, `-` and `_`. It also can't be `original`. When two themes' names end the same way, the page lists only the one from the game's own namespace, or failing that the first by title. For each game a theme is for, `gp registry publish` tells you whether the page will offer it after a reindex.

## Where it can live

| Target | Written as | Notes |
|---|---|---|
| A folder | A path | For testing |
| A git repo | `git:PATH` to a checkout | Each command commits and pushes. Published files can't be replaced |
| An S3 bucket | `s3://BUCKET` | Uses your AWS command-line login |

Pages read a registry over https, with CORS allowed. `gp registry check URL` tests a public address the way a browser would. To try a registry on your machine with a page you've built, use `gp registry serve DIR --page PAGE`.

[Where a registry can live](https://github.com/lex00/gameprinter/blob/main/docs/registry-backends.md) has the details.

## Signing

Signing is off unless you turn it on. Every file is still checked against its hash, which catches damage, but anyone who can write to your registry can publish to it. Once other people publish to your registry, you'll want signing; the same document says how to turn it on.

## Send a theme or game to someone else's registry

`lex00/gp-registry` holds the project's own games and themes and doesn't take other people's. To share a theme, [host it yourself](../host-your-own/). If some other registry takes submissions, send yours with `gp registry submit`, naming that registry's repo with `--to`:

```sh
gp registry submit path/to/my-theme --name me/my-theme --version 1.0.0 \
  --title "My Theme" --for lex00/donkey-kong --to OWNER/REGISTRY --dry-run
gp registry submit path/to/my-theme --name me/my-theme --version 1.0.0 \
  --title "My Theme" --for lex00/donkey-kong --to OWNER/REGISTRY
```

`gp registry submit` copies your folder without its hidden files and clones the registry. On a branch of its own it then does what the registry's owner would do. It grants your namespace if the registry doesn't have it yet, publishes your package with every check `gp registry publish` runs, and lists it. Then it pushes the branch to your fork of the registry, making the fork if you don't have one, and opens a pull request for the owner to merge or turn down. You need the GitHub command line, `gh`, logged in. `--fork OWNER/REPO` names your fork.

If a check fails, it stops and tells you which check and why, in the check's own words. Nothing is pushed and no pull request is opened; only your own machine has changed. Fix what it names and run it again with the same version, which is still free because it was never published. Sending the same version again later replaces the branch and keeps the pull request that's already open.

## Your theme on the public page

The public page also offers another registry's themes when its link names that registry with `r=`. [Host your theme](../host-your-own/) walks through it.
