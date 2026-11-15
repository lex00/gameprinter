---
title: Host your theme
description: Put your theme in a registry of your own on GitHub, play it on the public page by a link, and put up a page of your own.
---

The public page installs games and themes from the project's registry, lex00/gp-registry. You can host your own registry of themes as a public GitHub repo and send a link that offers your theme from it on the public page. Games still come from the project's registry, so yours needs no copy of any game. Later you can put up a copy of the page too, on your own Cloudflare account.

Your AI agent can walk you through it: ask it to host your theme yourself. Over `gp mcp` it has a tool for this road, `host_your_own`, and it asks you before each step that writes to GitHub or Cloudflare. The examples below use a made-up account, `acme`, with a registry repo `acme/harbor-registry` and a theme `acme/harbor-lights` for Donkey Kong.

## Your registry

You need `gp` and git with a GitHub account. You also need your own ROM of the game.

```sh
cd ~ && gh repo create acme/harbor-registry --public --clone
gp registry init git:$HOME/harbor-registry --name harbor-registry --namespace acme
gp registry publish git:$HOME/harbor-registry path/to/harbor-lights \
  --name acme/harbor-lights --version 0.1.0 --kind content \
  --title "Harbor Lights" --for lex00/donkey-kong
gp registry reindex git:$HOME/harbor-registry --keep acme/harbor-lights@0.1.0
```

Each command commits to your repo and pushes. `gp registry host ~/harbor-registry --theme acme/harbor-lights` shows which steps are done and prints your registry's address.

## Play it on the public page

Add your registry's address to the link as `r=`:

```
https://gameprinter.intentius.workers.dev/p?g=lex00/donkey-kong&c=acme/harbor-lights&r=https://raw.githubusercontent.com/acme/harbor-registry/main
```

The page installs the game from its own registry as always and offers your theme beside its own themes. It says so under its header, checks every file of yours against its hash, and keeps `r=` in the links its Share button makes. It takes only an https address. GitHub can take five minutes to show a file you just pushed; `gp registry check` on the address reads it the way the page does.

## A page of your own

With checkouts of gameprinter and the games repo, Node and a Cloudflare account, and no ROM:

```sh
GP_PAGE_THEMES_URL=https://raw.githubusercontent.com/acme/harbor-registry/main \
GP_PAGE_CONTENT=acme/harbor-lights \
  scripts/web/release-check.sh --no-roms --game ../games/donkey-kong
scripts/web/deploy.sh --first --worker harbor-page --subdomain acme
scripts/web/deploy.sh BUNDLE_DIR --no-roms --worker harbor-page --subdomain acme
```

The checks that need a ROM are skipped, each with a line saying so, and only the page that passed is uploaded. GitHub Pages also works (`scripts/web/pages.sh`), but it can't send the headers the page needs to run a game's own code, so Donkey Kong and Tapper don't play there. The details are in the repo's [docs/host-your-own.md](https://github.com/lex00/gameprinter/blob/main/docs/host-your-own.md).
