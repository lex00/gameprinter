---
title: Host your theme
description: Share your theme from a registry of your own on GitHub, with a link that plays it on the public page, and put up a page of your own if you like.
---

Made a theme? You can share it with a link that plays it on the public page. Your theme lives in a registry of your own, which is just a public GitHub repo. The games still come from the project's registry, lex00/gp-registry, so yours never needs a copy of a game. Later, if you want, you can put up your own copy of the page on your own Cloudflare account.

The easiest way is to ask your AI agent to help you host your theme yourself. It has a tool for this over `gp mcp`, `host_your_own`, and it asks you before each step that writes to GitHub or Cloudflare. The examples below use a made-up account, `acme`, with a registry repo `acme/harbor-registry` and a theme `acme/harbor-lights` for Donkey Kong.

## Your registry

You need `gp`, git and a GitHub account, and your own ROM of the game.

```sh
cd ~ && gh repo create acme/harbor-registry --public --clone
gp registry init git:$HOME/harbor-registry --name harbor-registry --namespace acme
gp registry publish git:$HOME/harbor-registry path/to/harbor-lights \
  --name acme/harbor-lights --version 0.1.0 --kind content \
  --title "Harbor Lights" --for lex00/donkey-kong
gp registry reindex git:$HOME/harbor-registry --keep acme/harbor-lights@0.1.0
```

Each of these commits to your repo and pushes. To see which steps you've done and get your registry's address, run `gp registry host ~/harbor-registry --theme acme/harbor-lights`.

## Play it on the public page

Add your registry's address to the link as `r=`:

```
https://gameprinter.intentius.workers.dev/p?g=lex00/donkey-kong&c=acme/harbor-lights&r=https://raw.githubusercontent.com/acme/harbor-registry/main
```

The page installs the game from its own registry as usual and offers your theme next to its own. A line under the header says where your theme came from. The page checks every file of yours against its hash, and its Share button keeps `r=` in the links it makes. The address has to be https. GitHub can take five minutes to show a file you just pushed; `gp registry check` on the address reads it the way the page will.

## A page of your own

For this you need checkouts of gameprinter and the games repo, Node, and a Cloudflare account. You don't need a ROM:

```sh
GP_PAGE_THEMES_URL=https://raw.githubusercontent.com/acme/harbor-registry/main \
GP_PAGE_CONTENT=acme/harbor-lights \
  scripts/web/release-check.sh --no-roms --game ../games/donkey-kong
scripts/web/deploy.sh --first --worker harbor-page --subdomain acme
scripts/web/deploy.sh BUNDLE_DIR --no-roms --worker harbor-page --subdomain acme
```

The checks that need a ROM are skipped, each with a line saying so, and only a page that passed gets uploaded. GitHub Pages works too (`scripts/web/pages.sh`), but it can't send the headers the page needs to run a game's own code, which means Donkey Kong and Tapper won't play there. The repo's [docs/host-your-own.md](https://github.com/lex00/gameprinter/blob/main/docs/host-your-own.md) has the details.
