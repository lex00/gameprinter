# Releasing gp

gp is released with chant's release train, as INTENTIUS/choudoufu and behold are ([chant release](https://intentius.io/chant/cli/release/cut/)). `chant.config.ts` at the repo root says how; `.github/workflows/release.yml` is rendered from it by `just release-workflow` and is never edited by hand. The chant version is pinned in `scripts/release/chant.sh`; after changing the pin, render the workflow again.

Only the owner starts a release. An agent or a loop worker never runs `just release`, `chant release cut --yes` or `chant release publish`, and never pushes a release tag ([docs/workers.md](workers.md)).

## Starting one

1. Add a section to `CHANGELOG.md` for the version, `## gp 0.1.2` (a date may follow the version), with what changed, and land it on main.
2. `just release-plan` (or `just release-plan 0.2.0`) prints what the release would cut from this checkout's commit: the label, whether the commit is pushed and green, the CHANGELOG check, and what goes where. It is `chant release cut LABEL --dry-run`: it cuts, tags, pushes and publishes nothing.
3. `just release` (or `just release 0.2.0`) dispatches the workflow from main. `auto` takes the next version from the commits since the last release: gp's commits aren't conventional commits, so on 0.x that is the next patch. A minor or major release names its version.

## What the run does

| Job | What it does |
|---|---|
| gate | Takes main's head. Under `green: "forge-checks"` the cut refuses it unless every check GitHub has on it passed |
| build-darwin-arm64, build-darwin-amd64, build-linux-amd64, build-linux-arm64, build-windows-amd64 | Each on its own runner: `scripts/release/chant-build.sh TARGET` sets up Rust and cargo-about, builds the editor, stamps the version (`chant release stamp`), runs `scripts/release/build.sh`, and copies the archive to `dist/gp-TARGET.tar.gz` (`.zip` for Windows). It then unpacks it in an empty home and checks that `gp --version` says the release's version and that `gp self check` passes |
| cut | Runs `scripts/release/changelog.sh` (the release's `check`: CHANGELOG.md must have the version's section), records the five archives and `dist/SHA256SUMS`, and tags the commit `v<version>` |
| publish | Publishes the GitHub release with the archives and `SHA256SUMS`, its notes CHANGELOG.md's section, then the formula `Formula/gp.rb` in lex00/homebrew-tap (`brew install lex00/tap/gp`), rendered from `scripts/release/homebrew/gp.rb` with each archive's URL and SHA-256 |

The cut and the build jobs stamp the version into the files `release.version` names: `crates/gp-cli/Cargo.toml` (gp-cli's own `version` line), gp-cli's entry in `Cargo.lock`, and the plugin's `plugin/.claude-plugin/plugin.json` and `.claude-plugin/marketplace.json` once they are in the tree, so the plugin is always gp's version. No commit bumps a version: the committed versions stay where they are, and only a release's build carries the new one.

## Setup the owner does once

- A deploy key with write access on lex00/homebrew-tap, its private key in this repository's Actions secret `HOMEBREW_TAP_KEY`. The publish writes the formula with it over SSH.
- The workflow's own token makes the GitHub release; it needs nothing more.
- The first render: until it lands, `.github/workflows/release.yml` is the old hand-written one, which only builds and has no `label` input, so `just release` fails there and publishes nothing. `just release-workflow --force` renders it from `chant.config.ts` (chant refuses to overwrite a hand-written file without `--force`); land the rendered file through the tool gate. After that, `just release-workflow` with no flag renders it again.

`just release-build OUT` is the same build on a workstation, for this machine or `--target TARGET`, without the stamp; it carries the public page that last passed `just web-page-release`, which the runners' builds don't have.
