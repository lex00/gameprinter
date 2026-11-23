# Releasing gp

gp is released with chant's release train, as INTENTIUS/choudoufu and behold are ([chant release](https://intentius.io/chant/cli/release/cut/)). `chant.config.ts` at the repo root says how; `.github/workflows/release.yml` is rendered from it by `just release-workflow` and is never edited by hand. The chant version is pinned in `scripts/release/chant.sh`; after changing the pin, render the workflow again.

Only the owner starts a release. An agent or a loop worker never runs `just release`, `chant release cut --yes` or `chant release publish`, and never pushes a release tag ([docs/workers.md](workers.md)).

## Starting one

1. Add a section to `CHANGELOG.md` for the version, `## gp 0.1.2` (a date may follow the version), with what changed, and land it on main.
2. `just release-plan` (or `just release-plan 0.2.0`) prints what the release would cut from this checkout's commit: the label, whether the commit is pushed and green, the CHANGELOG check, and what goes where. It is `chant release cut LABEL --dry-run` (`scripts/release/plan.sh`): it cuts, tags, pushes and publishes nothing. The dry run records the archives as the cut does, from `dist/`, and only the run's build jobs write them, so for each one `dist/` lacks the script writes a stand-in, says so on its first line, and removes it (and the `dist/SHA256SUMS` the dry run writes) after. The digests it shows for those files are the stand-ins', not the release's; an archive already in `dist/` is used as it is.
3. `just release` (or `just release 0.2.0`) dispatches the workflow from main. `auto` takes the next version from the commits since the last release: gp's commits aren't conventional commits, so on 0.x that is the next patch. A minor or major release names its version.
4. `just release` then waits for the run. Once the run passes it puts the released version on main (below). A failed run leaves main alone. If the wait is cut short (Ctrl-C, a closed terminal), run `just release-on-main latest` once the run has passed.

## What the run does

| Job | What it does |
|---|---|
| gate | Takes main's head. Under `green: "forge-checks"` the cut refuses it unless every check GitHub has on it passed |
| build-darwin-arm64, build-darwin-amd64, build-linux-amd64, build-linux-arm64 | Each on its own runner: `scripts/release/chant-build.sh TARGET` sets up Rust and cargo-about, builds the editor, stamps the version (`chant release stamp`), runs `scripts/release/build.sh`, and copies the archive to `dist/gp-TARGET.tar.gz` (`.zip` for Windows). It then checks it with `scripts/release/check-archive.sh`: unpacked in an empty home with the checkout moved out of reach, `gp --version` must say the release's version, `gp self check` must pass and `gp game kit` must write the kit (below); the checkout is then put back. build-darwin-amd64 runs on the same Apple silicon runner as build-darwin-arm64 (`macos-15`): it cross-compiles for `x86_64-apple-darwin` and runs the check under Rosetta, which it installs first. Its own Intel runner, `macos-15-intel`, took 48 minutes where the others take about 20 (issue 47) |
| cut | Runs `scripts/release/changelog.sh` (the release's `check`: CHANGELOG.md must have the version's section), records the four archives and `dist/SHA256SUMS`, and tags the commit `v<version>` |
| publish | Publishes the GitHub release with the archives and `SHA256SUMS`, its notes CHANGELOG.md's section, then the formula `Formula/gp.rb` in lex00/homebrew-tap (`brew install lex00/tap/gp`), rendered from `scripts/release/homebrew/gp.rb` with each archive's URL and SHA-256 |

## The check of the built archive

A gp knows the path of the checkout it was built from (`CARGO_MANIFEST_DIR`, the runner's `/Users/runner/work/gameprinter/gameprinter` for a release), and some commands looked there for files. On the runner that path exists, so a check run there passes for a gp that fails on every other machine: 0.1.2's `gp game kit` read `crates/gp-web/Cargo.toml` under it, so `gp model gate` failed before any check for everyone who installed it (issue 46), and the build job's `gp self check` passed, since it only said that the files were carried and the checkout was beside it.

So the check is now run as a person runs gp. `scripts/release/check-archive.sh` (which `chant-build.sh` runs with `--move-checkout`) unpacks the archive into an empty folder with an empty `HOME` and `GP_HOME` and `GP_HIDE_BUILT_SOURCE=1`, moves the checkout away (`mv`, put back before the job uploads `dist/`), and runs:

- `gp --version`, which must say the release's version;
- `gp self check --as-installed`, which hides the checkout gp was built from for its whole run (`crate::repo::hide_built`, the same switch `GP_HIDE_BUILT_SOURCE=1` turns on for any command) and uses each thing the bundle carries, from the bundle alone: it opens the page's files (`mechanic/`, `editor/dist/`, `scripts/web/page/`, the two players), reads the templates (Moss Hop among them), makes the module kit by `gp game kit`'s own path, builds a module from that kit in the sandbox (the job installs Rust's wasm32 target, and bubblewrap on Linux), and starts the page's server on a free port and asks it for `/`. Each is a line, ready, failed with the path it tried, or skipped and why, and a failed one exits 1. The job passes `--strict` too, so a skipped use fails it as well: 0.1.3's module build from the kit panicked in gp-exts' build script on the kit's `[package.version]` table, and no runner or worker had ever run that step, since each skipped it for want of a toolchain or bwrap. Windows has no sandbox gp game build knows, so there the job leaves `--strict` off and the module build is named as skipped;
- `gp game kit -o DIR`, the command `gp model gate` runs first.

An Intel Mac archive checked on an Apple silicon Mac runs each of these under Rosetta (`arch -x86_64`). The module the self check builds is wasm, so the arm64 cargo it starts builds it as on an Intel Mac.

A failure stops the job, so the release is neither cut nor published. In `just check`, `self_check::tests::a_bundle_without_its_kit_fails_and_names_the_kit` runs the same self check on a small bundle with no kit and the switch on, beside the checkout: it exits 1 and names the kit.

A gp built from source (every gp the tool gate publishes on a developer's machine) carries no bundle, so checked as installed it fails every use. Without `--as-installed`, `gp self check` on such a gp uses the checkout it was built from in place of a bundle, by its build-time path: its first line is `built from source at PATH: checked against the checkout, not a bundle`, the page's files are read where gp reads them (`mechanic/` and `editor/dist/` from the pages published beside a `gp-COMMIT` build, else the checkout), the kit is made from the checkout's crates, and it exits 1 by what failed there. A checkout that is gone fails each use saying so. A released gp carries its bundle and is always checked from it, flag or not. `GP_HIDE_BUILT_SOURCE=1` also checks a source build as installed.

The job doesn't run `gp model gate` itself: the gate scores a game's clips on its ROM, and the runner has no ROM and no game. The gate is checked end to end in `just check` instead, by `crates/gp-cli/tests/gate_mosshop.rs`: it builds Moss Hop's ROM, makes the game with `gp game new` in a games repo of its own, saves two clips from `baselines/mosshop` with `gp clip presses`, and dry-runs a one-line `model.toml` patch through `gp model gate` from an empty `HOME`, `GP_HOME` and gate home, which must PASS. That test runs a dev build, which carries no bundle, so its kit comes from the checkout; the kit from the bundle is what the job's own `gp game kit` check covers.

The cut and the build jobs stamp the version into the files `release.version` names: `crates/gp-cli/Cargo.toml` (gp-cli's own `version` line), gp-cli's entry in `Cargo.lock`, and the plugin's `plugin/.claude-plugin/plugin.json` and `.claude-plugin/marketplace.json` once they are in the tree, so the plugin is always gp's version. No commit bumps a version before a release, and the run itself commits nothing.

## Main carries the released version

Claude Code's plugin marketplace reads `plugin/.claude-plugin/plugin.json` and `.claude-plugin/marketplace.json` from main, not from the release. If main kept the old version, `/plugin install gp@gameprinter` would install the previous release's plugin next to the new gp (this happened with 0.1.2, and 7e3f490c stamped it by hand). So after a run passes, `just release` runs `scripts/release/version-on-main.sh latest` (`just release-on-main LABEL` runs the same thing alone). The script works in a fresh worktree of origin/main. It runs `chant release stamp VERSION`, checks that only the four version files changed, commits "main carries VERSION", and pushes to main, trying a second time if main moved in between. A version with no GitHub release is refused, and if main already has the version nothing happens.

This runs from the owner's machine and not in the train because chant's `after-release` job has only `contents: read`, and `release.workflow.env` only reaches the cut and publish jobs, so the job has no credential that can push to main.

## Setup the owner does once

- A deploy key with write access on lex00/homebrew-tap, its private key in this repository's Actions secret `HOMEBREW_TAP_KEY`. The publish writes the formula with it over SSH.
- The workflow's own token makes the GitHub release; it needs nothing more.
- The first render: until it lands, `.github/workflows/release.yml` is the old hand-written one, which only builds and has no `label` input, so `just release` fails there and publishes nothing. `just release-workflow --force` renders it from `chant.config.ts` (chant refuses to overwrite a hand-written file without `--force`); land the rendered file through the tool gate. After that, `just release-workflow` with no flag renders it again.

## The build's times, and no cache

Each build job prints how long each of its steps took when it ends, however it ends: `chant-build.sh`'s own steps (setting up Rust, building cargo-about, building the editor, fetching the crates, stamping the version, `build.sh`, the check), then `build.sh`'s (copying the page's files, building the players, making the module kit, building gp, the archive). The same list is in the run's summary on GitHub, one block per build, so two runs can be compared there.

Nothing is cached between runs. chant renders the build jobs with checkout, the toolchains (each set up with caching off) and the build command, and has no field for a cache step: its release builds start from the commit and the toolchain, never from a cache an earlier run wrote, so a poisoned cache can't reach a release. A `run` step can't reach the Actions cache either, so `chant-build.sh` can't add one. A cache would need chant to add one to its build jobs.

## On a workstation

`just release-build OUT` is the same build on a workstation, for this machine or `--target TARGET`, without the stamp; it carries the public page that last passed `just web-page-release`, which the runners' builds don't have. `just release-check OUT/gp-VERSION-COMMIT-TARGET.tar.gz` then runs the job's check on it, `--strict` included, so the machine needs Rust with the wasm32 target and the sandbox; the checkout is hidden by `GP_HIDE_BUILT_SOURCE=1` and left where it is. An Apple silicon Mac builds and checks the Intel Mac archive too (`--target x86_64-apple-darwin` after `rustup target add x86_64-apple-darwin`; the check needs Rosetta, `softwareupdate --install-rosetta`).
