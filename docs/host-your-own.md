# Host your own registry and page

The owner hosts two things: the registry lex00/gp-registry, a GitHub repo of static files that holds the published games and themes, and the public page, https://gameprinter.intentius.workers.dev, a Cloudflare worker that installs games and themes from that registry. Anyone can host a registry of their own and a page of their own. A theme maker, say, can put their theme in a registry of their own and send a link that offers it on the public page, with no one's permission, and can later put up a page of their own too. Games always come from the page's own registry; a registry of yours adds themes, so it needs no copy of any game. A game you made, with its own pictures, goes in your registry and on a page of your own whose registry it is ("A game with its own pictures", below).

This page takes you from nothing to your own registry and page. The examples use a made-up GitHub account, `acme`, a registry repo `acme/harbor-registry` and a theme `acme/harbor-lights` for Donkey Kong.

## What you need

- `gp` (the [install page](../site/src/content/docs/install.md)), git, and a GitHub account. The `gh` command makes the repo for you, but the GitHub website does it too.
- Your own ROM of the game the theme is for, to make the theme and play it.
- For a page of your own: a checkout of this repo and of the games repo beside it, Node, and a Cloudflare account. No ROM is needed for that part.

## With your agent

Over `gp mcp`, the tool `host_your_own` walks the whole road. Ask your agent to host your theme yourself. Each answer says which step is now, what each step does, the commands for it and whether it writes to the outside world. The agent runs the commands through `gp_run` and calls `host_your_own` again after each one.

The steps that write to the outside world are asked first. Making a public repo, `gp registry init`, `grant`, `publish` and `reindex` on a `git:` target (each commits and pushes), and deploying a page all wait for your yes: on your page as a card with YES and NO while `gp serve` runs, else in the conversation. `gp registry host` and `gp registry check` only read and run at once.

On the command line the same road is `gp registry host`:

```sh
gp registry host ~/harbor-registry --theme acme/harbor-lights --game lex00/donkey-kong
```

It prints every step with `done`, `NOW` or `-` (not known from here), the commands, and the registry's address once the repo is on GitHub. It writes nothing; it asks git what GitHub holds.

## 1. A repo for the registry

Make an empty public repo on GitHub and clone it:

```sh
cd ~ && gh repo create acme/harbor-registry --public --clone
```

It has to be public: a player's browser reads the files from GitHub's raw host, https://raw.githubusercontent.com/acme/harbor-registry/main, with no login.

## 2. Make it a registry

```sh
gp registry init git:$HOME/harbor-registry --name harbor-registry --namespace acme
```

This writes `registry.toml`, the namespace's file and an empty yank list, commits them and pushes. With no keys the registry is unsigned: readers check every file against its hash, and the repo's write access is what keeps others from publishing in it ([registry-backends.md](registry-backends.md#signing-is-opt-in)). Add `--no-push` to any of these commands to look at the commit before it goes up.

## 3. Your namespace

`init --namespace acme` already granted it. To grant another later:

```sh
gp registry grant git:$HOME/harbor-registry acme
```

Without the grant, a page lists a package in that namespace and then fails to install it.

## 4. Your theme

```sh
gp registry publish git:$HOME/harbor-registry ~/.gameprinter/work/themes/harbor-lights \
  --name acme/harbor-lights --version 0.1.0 --kind content \
  --title "Harbor Lights" --for lex00/donkey-kong
gp registry reindex git:$HOME/harbor-registry --keep acme/harbor-lights@0.1.0
```

Every file in the folder goes in, hidden ones too, so publish a folder that holds only the theme. `publish` adds the version; `reindex` lists it in your registry's list for the game (`by-game/lex00/donkey-kong.toml`), which is what a page reads to offer it. The game itself isn't in your registry and needn't be.

## 5. Check it from outside

```sh
gp registry check https://raw.githubusercontent.com/acme/harbor-registry/main \
  --for lex00/donkey-kong --theme acme/harbor-lights
```

This reads the registry as a player's browser does: CORS, every file by its hash, and the game's theme list as the page reads it (with no game in the registry), saying whether your theme is offered and, if not, why. GitHub's raw host caches each file for five minutes, so a version you just pushed can take that long to show.

## 6. The project's page doesn't take your registry

The project's public page plays only its own registry's games and themes: a link's `r=` is not read there (the page is built without `GP_PAGE_LINK_REGISTRY=1`), and the page says so. Your themes go on a page of your own, next.

## 7. A page of your own

Build the page with your registry as its second registry of themes, check it, and put it up. Its games still come from the project's registry. The release check and the deploy are the games repo's (`page/` in lex00/games); they build the page with gameprinter's `scripts/web/build-page.sh` from a built gp's checkout (`GP=path/to/gameprinter/target/debug/gp`). You need checkouts of gameprinter and of the games repo, and Node. No ROM: `--no-roms` skips every check that needs one and says so in a line each.

```sh
cd games
GP=../gameprinter/target/debug/gp \
GP_PAGE_THEMES_URL=https://raw.githubusercontent.com/acme/harbor-registry/main \
GP_PAGE_CONTENT=acme/harbor-lights \
  page/release-check.sh --no-roms --game donkey-kong
```

`GP_PAGE_THEMES_URL` goes in the page's `page.json` as `themes`: with no `r=` in its link the page offers that registry's themes beside its own registry's, and the line under its header says so (`GP_PAGE_THEMES_ROOT` gives a signed registry's root key line). `GP_PAGE_CONTENT` is the theme the page opens in. `GP_PAGE_HOME` is the https address of your list of games: the page's header then has an "All games" link to it (a page built for the repo's own registry links to that registry's list, `home` in `scripts/web/sources.toml`; with neither, the header has no link). Nothing in the repo is edited. (`GP_PAGE_SOURCE_URL` replaces the page's own registry, where games come from, for someone hosting games too.) On a pass the check records the page's digest, marked as made without ROMs, and prints `bundle: DIR`. Only that folder, file for file as it passed, can be uploaded.

### On Cloudflare

Cloudflare Workers serves the folder with the page's own headers (`_headers`), which the page needs. Log in once with `npx wrangler login`. Your workers.dev subdomain is on the Workers page of your Cloudflare dashboard. In the games checkout:

```sh
page/deploy.sh --first --worker harbor-page --subdomain acme
page/deploy.sh BUNDLE_DIR --no-roms --worker harbor-page --subdomain acme
```

The first command makes the worker with a placeholder page, once. The second uploads the page as a version that isn't live, runs the page check on that version's own preview address, asks you to type the worker's name, and makes it live at https://harbor-page.acme.workers.dev. `--address URL` gives a whole address instead (a domain of your own), and `GP_PAGE_WORKER`, `GP_PAGE_SUBDOMAIN` and `GP_PAGE_ADDRESS` do the same as the flags. Left out, they are the owner's: `gameprinter` on `intentius`. `--list` stops before the upload with wrangler's own dry run. Leave out `BUNDLE_DIR` and `deploy.sh` runs the release check itself (`--no-roms` passes on to it); the upload is held to the digest either way ([web.md](web.md#deploying-the-public-page)).

### On GitHub Pages

```sh
scripts/web/pages.sh BUNDLE_DIR ~/acme-page
```

This writes the page into `~/acme-page/site/` and a workflow, `.github/workflows/pages.yml`, that publishes `site/` on each push to `main`. Make `~/acme-page` a repo, push it to GitHub, and set the repo's Settings, Pages, Source to "GitHub Actions". The page is then at https://acme.github.io/acme-page/. It refuses a folder no release check passed on, as `deploy.sh` does.

GitHub Pages can't send headers of the page's choosing, and the page needs some:

| Header | What breaks on Pages |
|---|---|
| Cross-Origin-Opener-Policy and Cross-Origin-Embedder-Policy | The page isn't cross-origin isolated, so it has no SharedArrayBuffer and can't run a game's own module with its limits. Every game with hooks (Donkey Kong, Tapper) is refused with "this page wasn't served with its isolation headers, so it can't run a game module with its limits". Only a game with no hooks, which plays in the page's own interpreter (one made with `gp game new`), plays |
| Content-Security-Policy | `pages.sh` puts the policy in each HTML file as a `<meta>`, which covers everything the page loads except `frame-ancestors`: on Pages any site can frame the page |
| Cache-Control | Pages caches each file for ten minutes, so for a while after a push a visitor can get a new `page.js` beside an old file |
| `/p` | Pages doesn't read `_redirects`; `pages.sh` copies `index.html` to `p.html`, which Pages serves at `/p` |

Use Cloudflare for a page that plays Donkey Kong or Tapper.

## A game with its own pictures

The road above is written for a theme, and a game made here (a copy of a template such as Moss Hop, docs/web.md "A game with its own pictures") takes most of it as it is. Steps 1 to 5 work for a game's package as for a theme's: the repo, `init`, `grant`, `publish` and `reindex`, and `gp registry check` take any folder and kind. Two things differ. A page installs games only from its own registry (page.json's `source`), never from a second one, so the page that plays your game must have your registry as its source. And step 7's page (`release-check.sh`) builds a ported game's page from the games repo; a game of your own is built and published by `gp game share`, which by itself writes into `$GP_HOME/work/share/` a registry under the namespace `local` and a page that reads it from its own address (`.`), so the link plays only where gp serve's `/share/` or `gp registry serve` puts the two folders together.

With the examples' account, the road for a game `my_hop`:

```sh
cd ~ && gh repo create acme/harbor-registry --public --clone
gp game share ~/.gameprinter/work/mcp/my_hop \
  --registry ~/harbor-registry --namespace acme \
  --source https://raw.githubusercontent.com/acme/harbor-registry/main \
  --page ~/harbor-page
```

`gp game share` makes `~/harbor-registry` a registry (unsigned, `acme` granted) if it isn't one yet, publishes the game into it as `acme/my-hop` at the next version, lists it, and writes the page into `~/harbor-page` with page.json's `source` set to the `--source` address. It writes both as folders: nothing is committed, pushed or uploaded. A registry that is there already has to grant the namespace (`gp registry grant ~/harbor-registry acme`). `--source` is an https address, since the page's policy reads no other address (`connect-src 'self' https:`); GitHub's raw host sends the CORS header the page's fetch needs.

Then the two writes to the outside world, which are yours to make:

```sh
git -C ~/harbor-registry add -A && git -C ~/harbor-registry commit -m "acme/my-hop" && git -C ~/harbor-registry push
gp registry check https://raw.githubusercontent.com/acme/harbor-registry/main
npx wrangler@4.157.0 deploy --assets ~/harbor-page --name harbor-page --compatibility-date 2026-11-01
```

Cloudflare sends the page's own `_headers` (written into the page folder by its build), which a game with a module needs; GitHub Pages can't (above). `deploy.sh` is the project page's road and uploads only a page the release check passed, which a shared game's page never went through, so the page goes up with wrangler itself. The link `https://harbor-page.acme.workers.dev/p?g=acme/my-hop` then plays the game for anyone, with nothing installed: the page comes from Cloudflare, the game from your registry on GitHub, each file checked against its hash.

Each share again is a new version in the same registry; push it and the page offers it without being deployed again. Signing is as for a theme: the registry is unsigned and the repo's write access keeps others from publishing in it.

`crates/gp-cli/src/game_share.rs`'s test `the_road_puts_a_game_where_a_page_on_another_address_finds_it` walks this road with no network: a made-up game into a registry folder and a page folder outside the home, the home taken away, the registry and the page served on two local addresses, and the page's own install code (`gp_web_registry::install_on`) finding the game at the address page.json names.

## Trying the road with no real account

Each piece runs against a stand-in, so it can be tried without GitHub or Cloudflare:

| Real | Stand-in |
|---|---|
| The GitHub repo | A local bare repo as the checkout's origin, and `gp registry host --github acme/harbor-registry` for the address it stands for (crates/gp-cli/tests/host_your_own.rs walks the road this way) |
| wrangler | `GP_WRANGLER` naming a script that prints what it is asked (the games repo's page/test-deploy.sh) |
| A GitHub Pages site | The `site/` folder served by any local server with no headers |
| A registry of themes by its address | The page check's `registry-by-address` section serves a copy of the release check's registry with the game taken out on a free port of this machine and answers an https address from it, so the page's https rule and its policy are the ones a visitor gets; the game comes from the page's own registry and the theme from the second (scripts/web/public/check-page.mjs) |
