---
title: Host your game or theme
description: Put your theme, or a game you made, online on your own free Cloudflare account, and share the link.
prompt: |
  Read this page. Then help me put my theme online on a page of my own, step by step, with the host_your_own tool over gp mcp. Ask me before each step that writes to GitHub or Cloudflare. Use Cloudflare's own command-line tool, wrangler, through npx for the upload; the first time, run `npx wrangler login` and let me approve it in my browser.
---

The project's public page plays only the project's own games and themes. To share yours, you put up a page of your own on a free Cloudflare account. It plays the project's games with your theme added, and you share its link. Nothing of yours goes on the project's page, and nothing of the project's costs you anything.

## Tell your agent

The easy way is to ask your AI agent. Copy the prompt from the box at the top of this page, or say: "Help me host my theme on a page of my own with host_your_own." Over `gp mcp` the agent has a tool for this road, `host_your_own`. It tells the agent which step you're on and what comes next. The agent asks you before each step that writes to GitHub or Cloudflare.

## What you need

- `gp` ([Install gp](../install/)), git, and a GitHub account.
- A free Cloudflare account. You sign up at cloudflare.com; no card is needed for this.
- Node, for Cloudflare's command-line tool.
- For now, a checkout of gameprinter and of the games repo side by side, since building your page uses their scripts. A `gp` command for this is on the way.
- Your own ROM of the game, to make and play your theme. Building and uploading the page needs no ROM.

## 1. Your registry

Your theme lives in a registry of your own, which is just a public GitHub repo. The examples use a made-up account, `acme`. Its repo is `acme/harbor-registry` and its theme `acme/harbor-lights`, for Donkey Kong.

```sh
cd ~ && gh repo create acme/harbor-registry --public --clone
gp registry init git:$HOME/harbor-registry --name harbor-registry --namespace acme
gp registry publish git:$HOME/harbor-registry path/to/harbor-lights \
  --name acme/harbor-lights --version 0.1.0 --kind content \
  --title "Harbor Lights" --for lex00/donkey-kong
gp registry reindex git:$HOME/harbor-registry --keep acme/harbor-lights@0.1.0
gp registry check https://raw.githubusercontent.com/acme/harbor-registry/main \
  --for lex00/donkey-kong --theme acme/harbor-lights
```

Each of the first four commits to your repo and pushes. The last one reads your registry the way a page will. `gp registry host ~/harbor-registry --theme acme/harbor-lights` shows which steps you've done.

## 2. Your page, on Cloudflare

Cloudflare's command-line tool is called wrangler. There's nothing to install, since `npx` fetches the pinned version when the scripts run it. It only ever touches your own Cloudflare account. The first time, log in once:

```sh
npx wrangler login
```

It opens your browser for you to approve. Next, build your page in the games checkout with your registry as its registry of themes, using the gp you built in the gameprinter checkout beside it. Then upload it.

```sh
GP=../gameprinter/target/debug/gp \
GP_PAGE_THEMES_URL=https://raw.githubusercontent.com/acme/harbor-registry/main \
GP_PAGE_CONTENT=acme/harbor-lights \
  page/release-check.sh --no-roms --game donkey-kong
page/deploy.sh --first --worker harbor-page --subdomain acme
page/deploy.sh BUNDLE_DIR --no-roms --worker harbor-page --subdomain acme
```

- `--worker` is your page's name and `--subdomain` is your Cloudflare account's workers.dev subdomain. Cloudflare shows it under Workers. Your page's address is `https://WORKER.SUBDOMAIN.workers.dev`.
- `--first` runs once, to make the worker. After that, each upload goes up hidden and gets checked at its own preview address, and it goes live only when you type the worker's name to confirm.
- `BUNDLE_DIR` is the folder the release check prints on its last line.

The checks that need a ROM are skipped, each with a line saying so.

## 3. Share it

Your page plays the project's games and offers your theme beside them. A link to share looks like this one.

```
https://harbor-page.acme.workers.dev/p?g=lex00/donkey-kong&c=acme/harbor-lights
```

GitHub can take five minutes to show a file you just pushed. GitHub Pages can't host the page: it can't send the headers a game's own code needs, so Donkey Kong won't play there. Use Cloudflare.

## Only one game?

Hosting one game you made will take less than this: a page that carries its game in its own folder, with no separate registry repo. That's on the way. Until then, publish your game to your registry with `--kind game`, and build your page with `GP_PAGE_SOURCE_URL` set to your registry's address in place of `GP_PAGE_THEMES_URL`, so the page installs the game from your registry.

The repo's [docs/host-your-own.md](https://github.com/lex00/gameprinter/blob/main/docs/host-your-own.md) has the details.
