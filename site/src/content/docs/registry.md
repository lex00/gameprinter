---
title: Run a registry
description: Publish games and themes to a registry of your own, in a folder or a git repo.
---

A registry is a set of static files that holds published games and themes. Each published version is a package: the folder's files, each named by a hash of its bytes, and a list of them, of kind `game` or `content` (a theme). You can run your own on anything that serves files.

## Make one and publish to it

```sh
gp registry init ~/my-registry --name my-games --namespace me
gp registry publish ~/my-registry path/to/my-theme \
  --name me/my-theme --version 1.0.0 --kind content \
  --title "My Theme" --for lex00/donkey-kong
gp registry reindex ~/my-registry me/my-theme@1.0.0
gp registry check ~/my-registry
```

`reindex` rewrites the lists readers use to find things. Given `--keep` (or no versions at all) it starts from what is listed now and adds the versions you name. Without `--keep` you name every version that should stay listed; one left out is refused unless `--drop` names it. The last version of a package becomes its latest.

## Namespaces, withdrawals and what the page offers

| Command | What it does |
|---|---|
| `gp registry grant TARGET NAMESPACE` | Lets packages named `NAMESPACE/...` install from the registry. Without it the page lists such a theme and its install fails |
| `gp registry grants TARGET` | Lists the namespaces granted |
| `gp registry yank TARGET NAME@VERSION --severity security --reason TEXT` | Withdraws a version: the page leaves a `security` one out and refuses to install it, and still offers a `notice` one with the reason shown. `--undo` takes it back |
| `gp registry check TARGET --for GAME [--theme NAME]` | Reads the game's theme list as the page does: each theme it offers, installed as it installs it, and each it leaves out with why. With `--theme`, whether that one is offered and if not why, exiting 1 when it isn't. A theme whose last recheck failed says when and why |
| `gp theme recheck GAME NAME --registry TARGET` | Reads a theme the registry offers for the game again, as the page reads it, over your ROM of the game, and records the day, pass or fail, and why in the theme's line of the game's list (`recheck`, signed with the list). The page still offers a theme whose recheck failed and says why. A reindex keeps it |

The page names a theme by the part of its package name after the namespace, so that part must start with a lowercase letter, hold only lowercase letters, digits, `-` and `_`, and not be `original`. Of two themes whose names end the same, only the one from the game's own namespace is listed (else the first by title). `gp registry publish` says, for each game a theme is for, whether the page will offer it once it is reindexed.

## Where it can live

| Target | Written as | Notes |
|---|---|---|
| A folder | A path | For testing |
| A git repo | `git:PATH` to a checkout | Each command commits and pushes. Published files can't be replaced |
| An S3 bucket | `s3://BUCKET` | Uses your AWS command-line login |

A page reads a registry over https, with CORS allowed. `gp registry check URL` tests a public address the way a browser would. To try one on your machine with a page you've built, use `gp registry serve DIR --page PAGE`.

[Where a registry can live](https://github.com/lex00/gameprinter/blob/main/docs/registry-backends.md) has the details.

## Signing

Signing is off unless you opt in. Every file is still checked against its hash, which catches damage, but anyone who can write to your registry can publish to it. Signing matters once you have other publishers. The same document covers how to turn it on.

## Send a theme or game to someone else's registry

`lex00/gp-registry` holds the project's own games and themes and doesn't take others'. To share a theme, [host it yourself](../host-your-own/). Another registry that takes submissions can be sent one with `gp registry submit`, naming its repo with `--to`:

```sh
gp registry submit path/to/my-theme --name me/my-theme --version 1.0.0 \
  --title "My Theme" --for lex00/donkey-kong --to OWNER/REGISTRY --dry-run
gp registry submit path/to/my-theme --name me/my-theme --version 1.0.0 \
  --title "My Theme" --for lex00/donkey-kong --to OWNER/REGISTRY
```

`gp registry submit` copies your folder without its hidden files, clones the registry, and on a branch of its own runs what the owner would run: it grants your namespace if the registry doesn't have it yet, publishes your package with every check `gp registry publish` runs, and lists it. Then it pushes the branch to your fork of the registry (it makes the fork if you have none) and opens a pull request. That registry's owner reads it and merges it or not. You need the GitHub command line, `gh`, logged in. `--fork OWNER/REPO` names your fork.

If a check fails, it stops there and tells you which check and why, in the check's own words. Nothing is pushed to your fork and no pull request is opened, so nothing changes anywhere but your own machine. Fix what it names and run it again with the same version: the version was never published, so it is still free. Sending the same version again later replaces the branch, and the pull request already open for it stays.

## Your theme on the public page

The public page also offers the themes of another registry when its link names that registry with `r=`. [Host your theme](../host-your-own/) is the whole road.
