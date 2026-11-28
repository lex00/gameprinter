# Game Printer's tool repo. `just check` is the check for every change.

gp_home := env_var_or_default("GP_HOME", env_var("HOME") / ".gameprinter")
target := env_var_or_default("CARGO_TARGET_DIR", justfile_directory() / "target")
# Default dev, the checks' profile; the gate and a --rust worker set GP_PROFILE=fastdev.
profile := env_var_or_default("GP_PROFILE", "dev")
profile_dir := if profile == "dev" { "debug" } else { profile }
# GP_CHECK_BIN: the copy of gp that just check's lanes run, made once its
# builds are done, so a lane's cargo build or test (which writes another gp
# to target/, its features unified otherwise) never replaces the binary
# another lane is running.
gp := env_var_or_default("GP_CHECK_BIN", target / profile_dir / "gp")
mosshop_dir := target / "mosshop"
mosshop_rom := mosshop_dir / "mosshop.nes"
assets := gp_home / "work" / "assets" / "mosshop"

# Fails unless the ROM-blocking pre-commit hook is on in this clone.
_hook-enabled:
    @h="$(git config core.hooksPath)"; [ "$h" = ".githooks" ] || cmp -s "$h/pre-commit" .githooks/pre-commit || { echo "The ROM-blocking hook isn't enabled. Run: just setup" >&2; exit 1; }

# Turns on the ROM-blocking hook and installs the locked cargo.
setup:
    git config core.hooksPath .githooks
    mkdir -p {{quote(gp_home / "bin" / "locked")}}
    install -m 755 scripts/cargo/locked.py {{quote(gp_home / "bin" / "locked" / "cargo")}}

# Publishes this checkout's gp, optimized, at $GP_HOME/bin/gp: the first
# published build, before any gate has run. The gate publishes after that.
# The editor is built first: a gp is published only with its page's files
# (mechanic/ and editor/dist) beside it.
publish: editor
    cargo build -q --profile fastdev -p gp-cli
    {{quote(target / "fastdev" / "gp")}} self publish

build:
    cargo build -q --profile {{profile}} -p gp-cli

# A released gp for this machine, or for `--target TARGET` (a Rust target
# such as x86_64-unknown-linux-gnu or x86_64-pc-windows-msvc; default
# GP_RELEASE_TARGET, else this machine's): one binary that carries the files
# it reads at run time, so a maker needs no checkout, archived with its
# SHA-256 in OUT, which must be new or empty, as
# gp-VERSION-COMMIT-TARGET.tar.gz (.zip for Windows)
# (scripts/release/build.sh; site/src/content/docs/install.md). PAGE is the
# built public page to carry, default the newest that passed the release
# check. Another system's target needs its linker, so the release workflow
# builds each on its own runner, except the Intel Mac's, which an Apple
# silicon Mac builds too. Publishes nothing: no tag, no upload, no push.
#   just release-build OUT [--target TARGET] [PAGE]
release-build OUT *ARGS:
    scripts/release/build.sh {{quote(OUT)}} {{ARGS}}

# The release job's check of an archive, run on this machine
# (scripts/release/check-archive.sh): unpacked in an empty home with this
# checkout hidden, gp --version, gp self check --as-installed and gp game
# kit. An Intel Mac archive on an Apple silicon Mac runs under Rosetta.
# VERSION, given, is what gp --version must say.
#   just release-check OUT/gp-VERSION-COMMIT-TARGET.tar.gz [--version VERSION]
release-check ARCHIVE *ARGS:
    bash scripts/release/check-archive.sh {{quote(ARCHIVE)}} {{ARGS}}

# Start a release of gp (chant.config.ts, docs/release.md): dispatch
# .github/workflows/release.yml from main. It takes main's head once its
# checks on GitHub are green, builds gp on each platform's runner, cuts the
# release v<version>, stamps the version into crates/gp-cli/Cargo.toml and
# the plugin's manifests, and publishes the GitHub release (notes from
# CHANGELOG.md's "## gp VERSION" section, which must be there) and the
# Homebrew formula in lex00/tap. LABEL is auto (the next patch from the
# commits since the last release) or a version such as 0.2.0. It then waits
# for the run and, once it has passed, puts the released version on main
# (just release-on-main), since Claude Code's plugin marketplace reads the
# plugin from main. A failed run leaves main as it was. The owner's to run:
# an agent never starts a release.
#   just release
#   just release 0.2.0
release label="auto":
    #!/usr/bin/env bash
    set -euo pipefail
    before="$(gh run list --workflow release.yml --limit 1 --json databaseId --jq '.[0].databaseId // ""')"
    gh workflow run release.yml --ref main -f label={{quote(label)}}
    run=""
    for _ in $(seq 30); do
      run="$(gh run list --workflow release.yml --limit 1 --json databaseId --jq '.[0].databaseId // ""')"
      [ -n "$run" ] && [ "$run" != "$before" ] && break
      run=""; sleep 2
    done
    if [ -z "$run" ]; then
      echo "release: dispatched, but found no new run of release.yml; once it passes, run: just release-on-main latest" >&2
      exit 1
    fi
    echo "release: run $run; waiting for it (Ctrl-C leaves it running; then just release-on-main latest)"
    gh run watch "$run" --exit-status --interval 30 >/dev/null
    bash scripts/release/version-on-main.sh latest

# Put a released version on main (scripts/release/version-on-main.sh): stamp
# it into the files release.version names on a fresh worktree of origin/main
# and push that commit, so /plugin install gp@gameprinter gives the plugin of
# the gp just released. Local main follows it and is never rebased: with
# landings origin/main lacks, the commit goes on local main after them and
# nothing is pushed. LABEL is the version or latest; a version with no
# GitHub release is refused. `just release` runs it after a run passes. The
# owner's to run.
#   just release-on-main latest
#   just release-on-main 0.1.3
release-on-main label="latest":
    bash scripts/release/version-on-main.sh {{quote(label)}}

# What `just release` would cut from this checkout's commit: the label, the
# checks it would pass or refuse, and what would be published where. A dry
# run of chant release cut: cuts, tags, pushes and publishes nothing. Asks
# GitHub for the commit's checks with `gh auth token` when GH_TOKEN is unset.
# The archives come from the run's build jobs: for each one dist/ lacks, the
# dry run records a stand-in, removed after (scripts/release/plan.sh).
#   just release-plan
#   just release-plan 0.2.0
release-plan label="auto":
    bash scripts/release/plan.sh {{quote(label)}}

# Render .github/workflows/release.yml from chant.config.ts. Never edit that
# file by hand. The first render replaces the hand-written build-only file,
# which chant refuses to overwrite without --force.
#   just release-workflow [--force]
release-workflow *args:
    bash scripts/release/chant.sh release workflow {{args}}

mosshop:
    scripts/mosshop/build.sh {{quote(mosshop_dir)}}

# Everything this repo holds, on Moss Hop alone. Exits non-zero when any
# step fails. The tool gate sets GP_GATE_RUNS_TESTS=1 and GP_GATE_RUN (its
# pid) and runs the tests itself, once. After the steps everything else reads (fmt, Moss Hop's
# assets lifted and packed), the rest runs as lanes at once
# (scripts/check/parallel.sh): each lane's steps in order, each step timed
# (scripts/check/step.sh), each lane's output printed whole when it ends,
# and the first failing lane stops the others. Sub-recipes run with
# --no-deps: their dependencies (build, mosshop) ran once, here, so no lane
# rewrites the Moss Hop ROM another reads. The lanes run gp from a copy
# (target/check-bin/gp, as GP_CHECK_BIN), since the tests' cargo builds
# write a different gp to target/ while other lanes run it.
check: _hook-enabled build mosshop
    scripts/check/step.sh fmt cargo fmt --all --check
    mkdir -p {{quote(assets)}}
    scripts/check/step.sh assets-lift {{gp}} assets lift {{quote(mosshop_rom)}} --map assets/mosshop.toml -o {{quote(assets)}} --force
    scripts/check/step.sh assets-pack {{gp}} assets pack {{quote(assets)}} --check {{quote(mosshop_rom)}}
    mkdir -p {{quote(target / "check-bin")}}
    cp {{gp}} {{quote(target / "check-bin" / "gp.new")}}
    mv {{quote(target / "check-bin" / "gp.new")}} {{quote(target / "check-bin" / "gp")}}
    GP_CHECK_BIN={{quote(target / "check-bin" / "gp")}} GP_CHECK_LIFTED=1 scripts/check/parallel.sh {{quote(target / "check-lanes")}} \
        "rust=just --no-deps _check-rust" \
        "tests=just --no-deps _check-tests"
    @echo "just check: passed"

# The browser-heavy checks, left out of `just check` so a change lands in
# minutes: the editor, web parity, the player in a browser, the mechanic
# page and the public page's sections that need no ROM. CI runs them after `just check`; run them by hand when you change
# scripts/web, editor, mechanic or the play and web crates. No ROM, no games
# checkout: Moss Hop only.
check-web: build mosshop
    scripts/check/step.sh editor just --no-deps editor
    scripts/check/step.sh web-parity just --no-deps web-parity
    scripts/check/step.sh play-web just --no-deps play-web
    scripts/check/step.sh mechanic-page just --no-deps mechanic-page
    scripts/check/step.sh test-pages scripts/web/test-pages.sh {{quote(target / "test-pages")}}
    scripts/check/step.sh page-no-roms just --no-deps page-no-roms
    scripts/check/step.sh test-parallel scripts/check/test-parallel.sh {{quote(target / "test-parallel")}}
    scripts/check/step.sh check-vendor just --no-deps check-vendor
    @echo "just check-web: passed"

# check's lanes. The Rust lane: clippy, the generated files, Moss Hop's
# baselines and model, the originals, the repo's own script checks, and
# that the tools name no ported game (scripts/check/no-game-names.sh).
_check-rust:
    scripts/check/step.sh game-names bash scripts/check/no-game-names.sh
    scripts/check/step.sh clippy cargo clippy -q --workspace --all-targets --profile {{profile}} -- -D warnings
    scripts/check/step.sh cli-docs {{gp}} self docs --check CLI.md
    scripts/check/step.sh schemas {{gp}} self schemas --check schema
    scripts/check/step.sh baseline-mosshop {{gp}} baseline check --rom {{quote(mosshop_rom)}} baselines/mosshop
    scripts/check/step.sh baseline-testrom {{gp}} baseline check baselines/gp-testrom
    scripts/check/step.sh model-check {{gp}} model check templates/mosshop --assets {{quote(assets)}}
    scripts/check/step.sh model-fmt {{gp}} model fmt --check templates/mosshop
    scripts/check/step.sh labels {{gp}} labels check templates/mosshop/mapping.toml
    scripts/check/step.sh interp-mosshop just --no-deps _interp-mosshop
    scripts/check/step.sh template-check {{gp}} template check templates/mosshop --rom {{quote(mosshop_rom)}}
    scripts/check/step.sh originals just --no-deps originals
    scripts/check/step.sh test-hook scripts/test-hook.sh
    scripts/check/step.sh romguard python3 -I -B -m unittest discover -s scripts/romguard -q
    scripts/check/step.sh site-prose just --no-deps site-prose

# The tests' lane: `just test`, unless a live tool gate run runs them
# itself: GP_GATE_RUNS_TESTS=1 with GP_GATE_RUN the pid of a running gate.
# GP_GATE_RUNS_TESTS=1 alone (left in a shell) is ignored, with a line.
_check-tests:
    #!/usr/bin/env bash
    if [ "${GP_GATE_RUNS_TESTS:-}" = 1 ] && [ -n "${GP_GATE_RUN:-}" ] && kill -0 "$GP_GATE_RUN" 2>/dev/null; then
        echo "just check: the tests are left to the gate's own run (gate pid $GP_GATE_RUN)"
        exit 0
    fi
    if [ "${GP_GATE_RUNS_TESTS:-}" = 1 ]; then
        echo "just check: GP_GATE_RUNS_TESTS=1 ignored, since GP_GATE_RUN names no running gate; the tests run here"
    fi
    exec scripts/check/step.sh test just --no-deps test

# Formats the Rust code.
fmt:
    cargo fmt --all

# Runs every Rust test, the slow ones too (GP_SLOW_TESTS=1, docs/workers.md).
# Each slow test (one that starts with `skip_slow_test()`) is one long test
# on one core, so each gets a run of its own beside one run of the rest.
# Every run is `--workspace` and runs what the `--no-run` built: a run with
# `-p`, `--lib` or `--test NAME` resolves other features or units and builds
# again (tetanes-core, gp-nes and up for `--lib`, syn and gp-arcade for a
# `--test NAME` of gp-cli's on an arcade game). A slow run is `--tests`, which builds nothing
# more and leaves out the doctests, so they compile and run once, in the
# rest. Each slow test's full path comes from `--tests -- --list` and runs
# with `--exact`; the rest skips the same paths with `--exact`. A slow test
# that lists as no test fails, and so does a run that doesn't add up: each
# slow run must pass exactly its tests, and the rest must filter out exactly
# those, so every test runs once. Each run's output is in
# TARGET/test-runs/; a failed run's is printed.
test:
    #!/usr/bin/env bash
    set -uo pipefail
    export GP_SLOW_TESTS=1
    t() { cargo test -q --workspace --profile {{profile}} "$@"; }
    t --no-run || exit
    slow=$(find crates -name '*.rs' -print0 | xargs -0 awk '
        FNR == 1 { f = "" }
        match($0, /fn [a-z0-9_]+\(/) { f = substr($0, RSTART + 3, RLENGTH - 4) }
        /^[ \t]*if ([a-z_]+::)?skip_slow_test\(\)/ && f != "" { print f }' | sort -u)
    logs={{quote(target / "test-runs")}}
    mkdir -p "$logs"
    t --tests -- --list > "$logs/list.log" 2>&1 || { cat "$logs/list.log" >&2; exit 1; }
    # Sums the numbers before $1 in libtest's `test result:` lines of log $2.
    sum() { sed -n "s/^test result: .* \([0-9][0-9]*\) $1.*/\1/p" "$2" | awk '{ s += $1 } END { print s + 0 }'; }
    skips=() total=0
    for n in $slow; do
        sed -n 's/: test$//p' "$logs/list.log" | grep -E "(^|::)$n\$" > "$logs/$n.want"
        count=$(grep -c . "$logs/$n.want")
        if [ "$count" -lt 1 ]; then
            echo "tests: the slow test $n is no test of cargo test --workspace --tests (TARGET/test-runs/list.log)" >&2
            exit 1
        fi
        for p in $(cat "$logs/$n.want"); do skips+=(--skip "$p"); done
        total=$((total + count))
    done
    t -- --exact ${skips[@]+"${skips[@]}"} > "$logs/rest.log" 2>&1 &
    pids=($!) runs=(rest)
    for n in $slow; do
        t --tests -- --exact $(cat "$logs/$n.want") > "$logs/$n.log" 2>&1 &
        pids+=($!) runs+=("$n")
    done
    failed=0
    for i in "${!pids[@]}"; do
        r=${runs[$i]}
        if wait "${pids[$i]}"; then
            if [ "$r" = rest ]; then
                got=$(sum "filtered out" "$logs/rest.log") want=$total what="filtered out"
            else
                got=$(sum passed "$logs/$r.log") want=$(grep -c . "$logs/$r.want") what=passed
            fi
            if [ "$got" = "$want" ]; then
                echo "tests: $r passed ($got $what)"
                continue
            fi
            echo "tests: $r ran, but $got tests $what, not $want; its output:" >&2
        else
            echo "tests: $r failed; its output:" >&2
        fi
        cat "$logs/$r.log" >&2
        failed=1
    done
    exit "$failed"

# Builds the docs site with its link and contrast checks, then the prose lint.
site:
    cd site && ([ -f node_modules/.package-lock.json ] || npm ci --silent --no-audit --no-fund) && npm run build --silent
    just site-prose

# Serves the site at http://localhost:4321 while editing.
site-dev:
    cd site && ([ -f node_modules/.package-lock.json ] || npm ci --silent --no-audit --no-fund) && npm run dev

# The site's prose lint (site/scripts/lint-prose.mjs, the `sentences` trope
# rules) on its pages and README.md: a ratchet that fails only when a
# file's count of a rule grows over site/scripts/prose-baseline.json. When
# counts drop, `node site/scripts/lint-prose.mjs --update-baseline`.
site-prose:
    cd site && ([ -f node_modules/.package-lock.json ] || npm ci --silent --no-audit --no-fund) && npm run --silent lint:prose && npm run --silent test:prose > /dev/null

# The public page's share and theme-line tests under node (no browser, no
# game).
page-tests:
    node --test scripts/web/public/share.test.mjs scripts/web/public/theme-line.test.mjs

# The checks of a game against its ROM (its module's profiles and clips,
# its corpus scored, its arcade player and web parity, its MAME captures)
# are the games repo's: its justfile runs them from a game's folder with an
# installed gp. The tool knows no game, so nothing here reads a game's files.

# The original templates (docs/originals.md): each built from its
# authored.toml into $GP_HOME/work, held to its pins, its model checked
# against the build and in canonical form, and every level's model baseline
# replayed in the interpreter. Under a second for the three.
originals_list := "badge-in signal-failure swing-state"

originals: build
    #!/usr/bin/env bash
    set -euo pipefail
    for t in {{originals_list}}; do
        {{gp}} template build "templates/$t" > /dev/null
        {{gp}} template check "templates/$t" --rom {{quote(gp_home / "work" / "images")}}/"$t.nes" > /dev/null
        {{gp}} model check "templates/$t" --assets {{quote(gp_home / "work" / "assets")}}/"$t" > /dev/null
        {{gp}} model fmt --check "templates/$t" > /dev/null
        {{gp}} interp baselines "templates/$t" > /dev/null
        echo "originals: $t ok"
    done

# Writes the generated files: CLI.md from gp's commands and schema/ from
# the Rust types that read each format (schema/README.md).
docs: build
    {{gp}} self docs CLI.md
    {{gp}} self schemas schema
    {{gp}} self shell
    {{gp}} self features > docs/features.md

# The editor (game-printer's editor/, served at / by `gp serve`): its
# packages from its own lock, its type check and tests, and its build into
# editor/dist.
editor:
    cd editor && npm ci --silent --no-audit --no-fund
    cd editor && npx vitest run --silent
    cd editor && npm run --silent build > /dev/null

# The mechanic page's live player: gp_play.wasm built into target/play-web,
# Moss Hop played in it under node with scripted buttons and a save point
# restored on the way, then the input log replayed natively; both builds
# must give the same frame hash digest.
play-web: mosshop
    #!/usr/bin/env bash
    set -euo pipefail
    scripts/web/build-play.sh {{quote(target / "play-web")}} > /dev/null
    work={{quote(gp_home / "work" / "play-web")}}
    mkdir -p "$work"
    web=$(node scripts/web/check-play.mjs {{quote(target / "play-web" / "gp_play.wasm")}} {{quote(mosshop_rom)}} "$work/played.log")
    cargo build -q --profile {{profile}} -p gp-play --example digest
    native=$({{quote(target / profile_dir / "examples" / "digest")}} {{quote(mosshop_rom)}} "$work/played.log")
    if [ "$web" != "$native" ]; then
        echo "play-web: gp_play.wasm's frame hashes ($web) differ from the native replay's ($native)" >&2
        exit 1
    fi
    echo "play-web: Moss Hop played in gp_play.wasm and replayed natively: every frame hash agrees ($web)"

# The arcade player alone, for a game's own check of it (the games repo's
# play-web-arcade): gp_arcade_play.wasm built into OUT (default
# target/play-web) and the native examples it is held to (gp-arcade-play's
# pictures, sounds and latches, in the profile's examples folder). Reads no
# game and no romset.
play-arcade-wasm OUT=(target / "play-web"):
    scripts/web/build-play.sh --arcade {{quote(OUT)}} > /dev/null
    cargo build -q --profile {{profile}} -p gp-arcade-play --example pictures --example sounds --example latches
    @echo "play-arcade-wasm: {{OUT}}/gp_arcade_play.wasm, and pictures, sounds and latches in {{target / profile_dir / "examples"}}"

# The mechanic page in a headless browser on Moss Hop, in a GP_HOME made
# for the run: the screen page at gp mechanic start's and gp serve's
# addresses, the first screen, the copies and their tabs
# (scripts/web/check-mechanic.mjs). It needs no games checkout. The arcade
# run on a game's own romset (check-mechanic.mjs --only arcade
# --arcade-game DIR --arcade-rom ZIP --arcade-wasm WASM) is the games
# repo's to run. The player is built into target/play-web-mech, its own
# folder, so it can run beside play-web, which writes target/play-web.
mechanic-page: build mosshop
    #!/usr/bin/env bash
    set -euo pipefail
    scripts/web/build-play.sh {{quote(target / "play-web-mech")}} > /dev/null
    if [ -z "${PLAYWRIGHT_FROM:-}" ] && [ ! -d scripts/web/node_modules/playwright-core ]; then
        npm ci --prefix scripts/web --silent --no-audit --no-fund
    fi
    browser=$(scripts/inspector/fetch-browser.sh {{quote(gp_home)}})
    node scripts/web/check-mechanic.mjs --gp {{quote(gp)}} --rom {{quote(mosshop_rom)}} \
        --wasm {{quote(target / "play-web-mech" / "gp_play.wasm")}} --browser "$browser" \
        --shots {{quote(gp_home / "work" / "mechanic-page")}}

# The public page with no ROM and no games checkout: built by
# scripts/web/build-page.sh for Own Hop, a Moss Hop copy that plays from its
# own pictures, from a registry folder of its own, and the sections of
# scripts/web/public/check-page.mjs that need no ROM run on it in a headless
# browser (scripts/web/check-page-no-roms.sh), with an empty games folder
# and an empty ROM folder in a GP_HOME made for the run. The sections that
# play a ROM are the games repo's release check's.
page-no-roms: build mosshop
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -z "${PLAYWRIGHT_FROM:-}" ] && [ ! -d scripts/web/node_modules/playwright-core ]; then
        npm ci --prefix scripts/web --silent --no-audit --no-fund
    fi
    browser=$(scripts/inspector/fetch-browser.sh {{quote(gp_home)}})
    GP={{quote(gp)}} bash scripts/web/check-page-no-roms.sh {{quote(target / "page-no-roms")}} "$browser" {{quote(mosshop_rom)}}

_interp-mosshop:
    #!/usr/bin/env bash
    set -euo pipefail
    for f in baselines/mosshop/*.toml; do
        echo "$(basename "$f"): $({{quote(gp)}} interp compare templates/mosshop --rom {{quote(mosshop_rom)}} --baseline "$f" --assets {{quote(assets)}} | tail -1)"
    done

# Moss Hop held to its contract (docs/validation.md): templates/mosshop
# against its baselines, the model in the interpreter. Nothing is kept
# (--no-save). The same as crates/gp-cli/tests/validate.rs's first test.
validate-mosshop: build mosshop _validate-mosshop

_validate-mosshop:
    mkdir -p {{quote(assets)}}
    {{gp}} assets lift {{quote(mosshop_rom)}} --map assets/mosshop.toml -o {{quote(assets)}} --force
    {{gp}} validate templates/mosshop --against baselines/mosshop --rom {{quote(mosshop_rom)}} --assets {{quote(assets)}} --no-save

# Moss Hop's facts read back from games/mosshop/mosshop.s and its lifted
# tables (docs/derivation.md), then a copy with its kill line made wrong:
# derive must find 208 again and ask about it. Writes only under
# $GP_HOME/work/mosshop. game-printer also measured the baselines'
# recordings here with `gp input-log generate`, which this recipe doesn't run.
derive-mosshop: build mosshop _derive-mosshop

_derive-mosshop:
    #!/usr/bin/env bash
    set -euo pipefail
    work={{quote(gp_home / "work" / "mosshop")}}
    mkdir -p "$work" {{quote(assets)}}
    {{gp}} assets lift {{quote(mosshop_rom)}} --map assets/mosshop.toml -o {{quote(assets)}} --force >/dev/null
    out=$({{gp}} model derive templates/mosshop --no-questions --assets {{quote(assets)}}) || { echo "$out"; exit 1; }
    echo "derive-mosshop: $(head -1 <<<"$out")"
    copy="$work/derive-copy"
    mkdir -p "$copy"
    cp templates/mosshop/model.toml "$copy/"
    : > "$copy/questions.toml"
    python3 -I -c 'import sys; p = sys.argv[1]; t = open(p).read().replace("kill_below = 208\n", "kill_below = 200\n", 1); open(p, "w").write(t)' "$copy/model.toml"
    out=$({{gp}} model derive "$copy" --from evidence --root .) || { echo "$out"; exit 1; }
    grep -q 'differ .*levels.main.kill_below .* -> 208' <<<"$out" || { echo "$out"; echo "derive-mosshop: the wrong kill_below wasn't found" >&2; exit 1; }
    grep -q 'id = "derive:levels.main.kill_below"' "$copy/questions.toml" || { echo "derive-mosshop: no question for kill_below" >&2; exit 1; }
    echo "derive-mosshop: the mutated copy: $(head -1 <<<"$out")"

# Stage 0's ROM dataset (docs/formats/rom-dataset.md) for every ROM on this
# machine: Moss Hop and the NES ROMs in $GP_ROM_DIR/nes, each written to
# $GP_HOME/work/datasets/NAME and never into the repo. One line a ROM.
dataset-all: mosshop
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build -q --profile fastdev -p gp-cli
    gp={{quote(target / "fastdev" / "gp")}}
    roms="${GP_ROM_DIR:-$HOME/roms}"
    out={{quote(gp_home / "work" / "datasets")}}
    one() { "$gp" dataset rom "$2" --out "$out/$1" | sed "s|^.* -> $out/||"; }
    one mosshop {{quote(mosshop_rom)}}
    for r in "$roms"/nes/*.nes; do [ -f "$r" ] && one "$(basename "$r" .nes | tr -c 'A-Za-z0-9\n' '-')" "$r"; done

# Laya, the open Jev-compatible decision model, served on 127.0.0.1:7720
# over Jev's wire format for `gp mechanic demo` and `gp mechanic say --judge`
# (docs/mechanic-page.md). Installs @receptron/laya under $GP_HOME/work/laya,
# outside the repo, unless LAYA_DIR names an install already made; its weights (about 1.7 GB)
# download on first use to ~/.cache/receptron-laya.
laya port="7720":
    #!/usr/bin/env bash
    set -euo pipefail
    dir="${LAYA_DIR:-{{gp_home / "work" / "laya"}}}"
    mkdir -p "$dir"
    if [ ! -d "$dir/node_modules/@receptron/laya" ]; then
        (cd "$dir" && { [ -f package.json ] || npm init -y >/dev/null; } && npm install --silent --no-audit --no-fund @receptron/laya)
    fi
    node scripts/laya/serve.mjs "$dir" {{port}}

# The MAME oracle (docs/oracle-mame.md): MAME in its own pinned image, run on
# an arcade romset with an input log, writing what each frame shows. An
# optional pair of recipes, not part of `just check`. The tag names Debian's
# MAME version, which docker/mame/Dockerfile pins.
mame_version := "0.276"
host_arch := if arch() == "aarch64" { "arm64" } else { "amd64" }
mame_image := "gameprinter-mame:" + mame_version + "-" + host_arch

# Builds the MAME oracle image, for this machine's architecture by default.
mame-build platform=("linux/" + host_arch):
    #!/usr/bin/env bash
    set -euo pipefail
    tag="gameprinter-mame:{{mame_version}}-{{replace(platform, "linux/", "")}}"
    start=$SECONDS
    docker build -q --platform {{platform}} -t "$tag" docker/mame > /dev/null
    version=$(docker run --rm --network none --platform {{platform}} --entrypoint cat "$tag" /MAME_VERSION)
    case "$version" in
        "{{mame_version}} "*|"{{mame_version}}") ;;
        *) echo "mame-build: the image holds MAME $version, the justfile says {{mame_version}}" >&2; exit 1 ;;
    esac
    echo "mame-build: $tag (MAME $version) in $((SECONDS - start))s"

# ROMSET is a MAME romset zip, mounted read-only for the run; OUT goes under
# $GP_HOME/work/oracle/; SHOT_EVERY 0 writes no PNGs.
# Plays an input log on a romset in the MAME image, writing every frame.
mame-record ROMSET LOG OUT SHOT_EVERY="30" WAV="0":
    GP_HOME={{quote(gp_home)}} GP_ORACLE_WAV={{quote(WAV)}} scripts/oracle/mame-record.sh {{mame_image}} {{quote(ROMSET)}} {{quote(LOG)}} {{quote(OUT)}} {{quote(SHOT_EVERY)}}

# The Mesen oracle (docs/oracle.md): Mesen 2 built from source at a pinned
# commit, in its own image, separate from the toolchain image. The tag
# names the commit's first 8 hex digits, as gp_oracle::mesen::image does.
# Optional, not part of `just check`.
mesen_image := "game-printer-mesen:b9fa69dd"

# Builds the Mesen image, for this machine's architecture by default.
mesen-build platform=("linux/" + host_arch):
    @echo "building the Mesen image for {{platform}} (from source: a few minutes, longer under emulation)"
    docker build -q --platform {{platform}} -t {{mesen_image}}-{{replace(platform, "linux/", "")}} docker/mesen

# Each baseline in gp-nes and in Mesen on both architectures, diffed, every class triaged in oracle/triage.toml.
check-mesen: build (mesen-build "linux/arm64") (mesen-build "linux/amd64")
    {{gp}} oracle check baselines/gp-testrom --platform linux/arm64 --platform linux/amd64

# The accuracy baseline (docs/test-roms.md): the standard NES test ROMs, downloaded at pinned hashes, run on gp-nes. Not part of check, since it needs the network once.
check-testroms *suites: build build-testroms
    {{gp}} testroms run {{suites}}

# Assemble our own test programs, every `built_from` source in test-roms/suites.toml (`gp testroms built` lists them), with ca65 into $GP_HOME/work/test-roms/. Each x.s is linked with the x.cfg beside it.
build-testroms: build _build-testroms

_build-testroms:
    #!/usr/bin/env bash
    set -euo pipefail
    command -v ca65 >/dev/null && command -v ld65 >/dev/null || { echo "build-testroms: needs ca65 and ld65 (cc65): brew install cc65, or apt-get install cc65" >&2; exit 2; }
    work={{quote(gp_home / "work" / "test-roms")}}
    built=$({{gp}} testroms built)
    [ -n "$built" ] || { echo "build-testroms: the manifest lists no programs to build" >&2; exit 2; }
    while IFS=$'\t' read -r path src; do
        mkdir -p "$work/$(dirname "$path")"
        ca65 -o "$work/${path%.nes}.o" "$src"
        ld65 -C "${src%.s}.cfg" -o "$work/$path" "$work/${path%.nes}.o"
    done <<< "$built"

# The page, built from the games repo beside this one (the game its
# page.toml names).
page out="target/page":
    just build
    GP={{quote(gp)}} scripts/web/build-page.sh "$(scripts/web/page-game.sh)" {{quote(out)}}

# The public page (docs/web.md, "The public page"), built from the games
# repo beside this one (the game its page.toml names) into $GP_HOME/work/page/ and checked to hold no game
# data; with `check`, also played headless (scripts/web/public/check-page.mjs,
# the ROM from $GP_ROM_DIR, its ROM steps skipped without it) and its lift
# run under Node (lift.test.mjs); `accept-pictures` writes the picture
# sections' new hashes into scripts/web/public/pictures.txt.
# Builds the public page; `check` plays it headless and runs the lift's tests.
web-page *args: build
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{args}}" in ""|check|accept-pictures) ;; *) echo "web-page: give nothing, check or accept-pictures" >&2; exit 2 ;; esac
    WEB_TOOLS=$(scripts/web/fetch-tools.sh {{quote(gp_home)}})
    export WEB_TOOLS
    out={{quote(gp_home / "work" / "page")}}
    # The deployed page's theme, as the release check builds it.
    export GP_PAGE_CONTENT="${GP_PAGE_CONTENT-lex00/turnips}"
    game=$(scripts/web/page-game.sh)
    GP={{quote(gp)}} scripts/web/build-page.sh "$game" "$out"
    [ -n "{{args}}" ] || exit 0
    browser=$(scripts/inspector/fetch-browser.sh {{quote(gp_home)}})
    if [ -z "${PLAYWRIGHT_FROM:-}" ] && [ ! -d scripts/web/node_modules/playwright-core ]; then
        npm ci --prefix scripts/web --silent --no-audit --no-fund
    fi
    shots={{quote(gp_home / "work" / "page-shots")}}
    case "{{args}}" in
      # Every section, the picture references among them, then the lift.
      check)
        node scripts/web/public/check-page.mjs --dist "$out" --browser "$browser" --shots "$shots"
        mkdir -p "$out.lift-test/lift" "$out.lift-test/work"
        cp -R "$out/game/lift/gp-web-lift.wasm" "$out.play/lift/"* "$out.lift-test/lift/"
        GP_WEB_LIFT_OUT="$out.lift-test" GP_WORKER_OUT="$out.lift-test/work" node --test scripts/web/public/lift.test.mjs ;;
      # The picture sections alone, their new hashes written into pictures.txt: look at
      # the pictures in $GP_HOME/work/page-shots/ (MOMENT-new.png) before committing it.
      accept-pictures) node scripts/web/public/check-page.mjs --dist "$out" --browser "$browser" --shots "$shots" --accept-pictures ;;
    esac

# The web build against the native interpreter on Moss Hop's baselines
# (docs/web.md, "Parity"): gp_web.wasm under Node and the interpreter built
# for this machine must give the same state hash on every frame. No ROM but
# Moss Hop. WEB_TOOLS, when set, has wasm-opt shrink the module first.
# Moss Hop's baselines in gp_web.wasm and the native interpreter, every frame's hash.
web-parity: build mosshop
    #!/usr/bin/env bash
    set -euo pipefail
    assets={{quote(assets)}}
    mkdir -p "$assets"
    # Under check the assets are lifted once, before the lanes (GP_CHECK_LIFTED=1).
    if [ -z "${GP_CHECK_LIFTED:-}" ]; then
        {{quote(gp)}} assets lift {{quote(mosshop_rom)}} --map assets/mosshop.toml -o "$assets" --force > /dev/null
    fi
    out={{quote(target / "web" / "mosshop")}}
    scripts/web/build-preview.sh "$out" templates/mosshop "$assets" > /dev/null
    scripts/web/parity.sh "$out" templates/mosshop "$assets" scripts/web/parity/mosshop.toml | grep -v '^{'

# Build a registry in a folder on this machine and the page that installs from it.
local-registry GAMES THEME:
    GP_HOME={{quote(gp_home)}} scripts/web/local-registry.sh {{quote(GAMES)}} {{quote(THEME)}}

# Serve them at http://127.0.0.1:8377/p?g=NAMESPACE/NAME (the page's game,
# as its package.toml names it) until stopped.
# A game's module, built in the sandbox from the kit (GAME is its package folder).
game-module GAME OUT:
    target/debug/gp game kit -o target/module-kit
    target/debug/gp game build {{quote(GAME)}} --kit target/module-kit -o {{quote(OUT)}}

local-play:
    target/debug/gp registry serve {{quote(gp_home)}}/registry --page target/page-local

# game-printer's recipe also graded `gp behave`, which isn't ported.
# An optimized build, since a large banked game takes about a minute.
# Stage 0 on Moss Hop and each ROM given, from its own starts with no clip
# file: routines read (gp model static ROM --unread), one line a ROM.
[positional-arguments]
stage0 *ROMS: mosshop
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build -q --profile fastdev -p gp-cli
    gp={{quote(target / "fastdev" / "gp")}}
    for r in {{quote(mosshop_rom)}} "$@"; do
        line=$("$gp" model static "$r" --unread | grep '^Unread:')
        echo "$r: ${line#Unread: }"
    done

# The wasmtime release the WASM checks expect.
wasmtime_version := "48.0.7"

# gp-wasm-check on the WASM platform (docs/platform-checks.md): the open-tier
# baseline (baselines/gp-testrom/buttons.toml, compiled in) natively and
# under wasmtime, then Moss Hop's baselines under wasmtime. Both must give
# the committed digests. Not part of `just check`: it builds gp-wasm-check
# for wasm32-wasip2 in release.
check-wasm: mosshop
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build --release -q -p gp-wasm-check
    field() { sed -n "s/^$1 //p" <<<"$2"; }
    native=$({{quote(target / "release" / "gp-wasm-check")}}) || { echo "$native"; echo "check-wasm: the native build doesn't match baselines/gp-testrom/buttons.toml" >&2; exit 1; }
    echo "check-wasm: native $(field digest "$native") at $(field fps "$native") frames/s"
    if ! command -v wasmtime >/dev/null; then
        echo "check-wasm: wasmtime not found, so the WASM platform can't be checked. Install it with: cargo install --locked wasmtime-cli@{{wasmtime_version}}" >&2
        exit 1
    fi
    version=$(wasmtime --version | cut -d' ' -f2)
    [ "$version" = "{{wasmtime_version}}" ] || echo "check-wasm: note, wasmtime $version is not the pinned {{wasmtime_version}}"
    cargo build --release -q -p gp-wasm-check --target wasm32-wasip2
    module={{quote(target / "wasm32-wasip2" / "release" / "gp-wasm-check.wasm")}}
    wasm=$(wasmtime run "$module") || { echo "$wasm"; echo "check-wasm: the wasm32-wasip2 build doesn't match baselines/gp-testrom/buttons.toml" >&2; exit 1; }
    echo "check-wasm: wasm   $(field digest "$wasm") at $(field fps "$wasm") frames/s (wasmtime $version)"
    files=()
    for f in baselines/mosshop/*.toml; do files+=("/baselines/$(basename "$f")"); done
    echo "check-wasm: baselines/mosshop under wasmtime $version"
    wasmtime run --dir {{quote(mosshop_dir)}}::/rom --dir baselines/mosshop::/baselines "$module" baseline /rom/mosshop.nes "${files[@]}"

# Every recording in the store ($GP_STORE, else $GP_HOME/store) replayed
# natively and under wasmtime by gp-wasm-check, each ROM found by hash in
# $GP_ROM_DIR: NES images, arcade romsets by their identity, and model
# recordings from the store alone. Both must replay every recording that
# runs identically. Reads the store and the ROMs, writes nothing.
check-wasm-roms:
    #!/usr/bin/env bash
    set -euo pipefail
    store="${GP_STORE:-{{gp_home / "store"}}}"
    roms="${GP_ROM_DIR:-{{gp_home / "roms"}}}"
    cargo build --release -q -p gp-wasm-check
    echo "check-wasm-roms: natively"
    {{quote(target / "release" / "gp-wasm-check")}} replay-all "$store" "$roms"
    if ! command -v wasmtime >/dev/null; then
        echo "check-wasm-roms: wasmtime not found. Install it with: cargo install --locked wasmtime-cli@{{wasmtime_version}}" >&2
        exit 1
    fi
    cargo build --release -q -p gp-wasm-check --target wasm32-wasip2
    echo "check-wasm-roms: under wasmtime $(wasmtime --version | cut -d' ' -f2)"
    wasmtime run --dir "$store"::/store --dir "$roms"::/roms {{quote(target / "wasm32-wasip2" / "release" / "gp-wasm-check.wasm")}} replay-all /store /roms

# The open-tier baseline and Moss Hop's baselines on an x86_64 build of
# gp-wasm-check, under Rosetta on an Apple Silicon Mac: the same digests as
# the native and WASM builds. macOS only.
check-x86_64: mosshop
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "$(uname -s)" != Darwin ]; then echo "check-x86_64: macOS only (Rosetta)"; exit 0; fi
    cargo build --release -q -p gp-wasm-check --target x86_64-apple-darwin
    bin={{quote(target / "x86_64-apple-darwin" / "release" / "gp-wasm-check")}}
    "$bin" | sed -n 's/^digest /check-x86_64: digest /p'
    "$bin" baseline {{quote(mosshop_rom)}} baselines/mosshop/*.toml

# The vendored emulator core is exactly its import plus our patch
# (third_party/README.md), and the check catches each kind of edit.
check-vendor:
    scripts/vendor/tetanes-core.sh --check
    scripts/test-vendor.sh

# Full sprite capture on Moss Hop (docs/capture.md, "Sprites"), end to
# end: two baseline inputs recorded into a store of its own under
# $GP_HOME/work/sprites-check, captured (Moss Hop's sprites agree with
# capture, so the proposed model must equal the template's).
sprites-mosshop: build mosshop
    #!/usr/bin/env bash
    set -euo pipefail
    work={{quote(gp_home / "work" / "sprites-check")}}
    mkdir -p "$work/store" {{quote(assets)}}
    {{gp}} assets lift {{quote(mosshop_rom)}} --map assets/mosshop.toml -o {{quote(assets)}} --force > /dev/null
    ids=()
    for b in finish-level stomp-then-pit; do
        {{gp}} input-log generate --baseline baselines/mosshop/$b.toml --out "$work/$b.log" >/dev/null
        id=$(GP_STORE="$work/store" {{gp}} --json record --rom {{quote(mosshop_rom)}} --input "$work/$b.log" --name "mosshop-$b" | python3 -I -c 'import json, sys; print(json.load(sys.stdin)["id"])')
        ids+=(--recording "$id")
    done
    GP_STORE="$work/store" {{gp}} sprites capture templates/mosshop "${ids[@]}"
    cmp templates/mosshop/model.toml {{quote(gp_home / "work" / "sprites" / "mosshop" / "model.toml")}} || { echo "sprites-mosshop: capture proposes a change to Moss Hop's model" >&2; exit 1; }

# Every version pin in the repo (the toolchain, crates, npm, the MAME
# image, wasmtime, the inspector's browser, tetanes-core) with its newest
# release, the newest the pinned toolchain can use, and what blocks the
# rest. Read only: it asks public indexes what exists and changes nothing.
# Needs the network except with --offline.
upgrades *args:
    python3 -I -B scripts/upgrades/report.py {{args}}

# gp-nes's criterion micro-benchmarks, each mean recorded as a metric
# (bench.criterion.*, docs/platform-checks.md).
bench:
    #!/usr/bin/env bash
    set -euo pipefail
    stamp={{quote(target / "criterion-started")}}
    mkdir -p {{quote(target)}}
    touch "$stamp"
    cargo bench -q -p gp-nes --bench hot_paths -- --quick
    python3 -I -B scripts/bench/criterion-metrics.py --since "$stamp" {{quote(target / "criterion")}} | while read -r name value; do
        {{gp}} metrics record --name "$name" --value "$value" --unit ns >/dev/null
    done

# Time a clean build and an incremental one, as metrics, and save cargo's
# per-crate timing report. Builds into its own target directory
# (target/timings, emptied first), so the usual one stays warm.
build-timings: build
    #!/usr/bin/env bash
    set -euo pipefail
    dir={{quote(target / "timings")}}
    rm -rf "$dir"
    ms() { python3 -I -c 'import time; print(int(time.monotonic() * 1000))'; }
    t0=$(ms)
    CARGO_TARGET_DIR="$dir" cargo build --workspace --all-targets --timings -q
    t1=$(ms)
    {{gp}} metrics record --name build.clean.duration --value $((t1 - t0)) --unit ms >/dev/null
    # One edited line in the emulator crate: what most work on gp-nes costs.
    touch crates/gp-nes/src/lib.rs
    t0=$(ms)
    CARGO_TARGET_DIR="$dir" cargo build --workspace --all-targets -q
    t1=$(ms)
    {{gp}} metrics record --name build.incremental.gp-nes.duration --value $((t1 - t0)) --unit ms >/dev/null
    mkdir -p {{quote(gp_home / "reports")}}
    cp "$dir/cargo-timings/cargo-timing.html" {{quote(gp_home / "reports" / "cargo-timing.html")}}
    python3 -I -B scripts/build-timings.py "$dir/cargo-timings/cargo-timing.html" | while read -r name value; do
        {{gp}} metrics record --name "$name" --value "$value" --unit ms >/dev/null
    done
    python3 -I -B scripts/build-timings.py --summary "$dir/cargo-timings/cargo-timing.html"
    echo "clean build and incremental gp-nes rebuild recorded; cargo's report: {{gp_home / "reports" / "cargo-timing.html"}}"

# A tree per agent, for when two or more agents edit and build at once
# (docs/agents.md, "A tree per agent"; scripts/trees/, macOS only).
# tree-new clones this checkout and its target into $GP_HOME/trees/NAME on
# branch tree/NAME, without the checkout's uncommitted changes.
tree-new NAME:
    scripts/trees/new.sh {{quote(NAME)}}

# Cherry-picks tree NAME's commits onto work here, in order, stopping at
# the first conflict with work left as it was before that pick.
tree-take NAME:
    scripts/trees/take.sh {{quote(NAME)}}

# Each tree: branch, base, commits ahead, clean or not, its own disk use,
# age; then what .done holds and the command that clears it.
tree-list:
    scripts/trees/list.sh

# Moves the stale incremental sessions of this checkout's target (or
# TARGET) into $GP_HOME/trees/.done, keeping the newest of each crate and
# any a build holds; --dry-run says what it would move. Deletes nothing.
tree-prune *ARGS:
    scripts/trees/prune.sh {{ARGS}}

# Moves a taken tree to $GP_HOME/trees/.done and deletes its branch, once
# every one of its commits' patches is on work.
tree-done NAME:
    scripts/trees/done.sh {{quote(NAME)}}
