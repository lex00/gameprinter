# Game Printer's tool repo. `just check` is the check for every change.

gp_home := env_var_or_default("GP_HOME", env_var("HOME") / ".gameprinter")
target := env_var_or_default("CARGO_TARGET_DIR", justfile_directory() / "target")
# Default dev, the checks' profile; the gate and a --rust worker set GP_PROFILE=fastdev.
profile := env_var_or_default("GP_PROFILE", "dev")
profile_dir := if profile == "dev" { "debug" } else { profile }
gp := target / profile_dir / "gp"
mosshop_dir := target / "mosshop"
mosshop_rom := mosshop_dir / "mosshop.nes"
assets := gp_home / "work" / "assets" / "mosshop"

# Fails unless the ROM-blocking pre-commit hook is on in this clone.
_hook-enabled:
    @[ "$(git config core.hooksPath)" = ".githooks" ] || { echo "The ROM-blocking hook isn't enabled. Run: just setup" >&2; exit 1; }

# Turns on the ROM-blocking hook and installs the locked cargo.
setup:
    git config core.hooksPath .githooks
    mkdir -p {{quote(gp_home / "bin" / "locked")}}
    install -m 755 scripts/cargo/locked.py {{quote(gp_home / "bin" / "locked" / "cargo")}}

# Publishes this checkout's gp, optimized, at $GP_HOME/bin/gp: the first
# published build, before any gate has run. The gate publishes after that.
publish:
    cargo build -q --profile fastdev -p gp-cli
    {{quote(target / "fastdev" / "gp")}} self publish

build:
    cargo build -q --profile {{profile}} -p gp-cli

mosshop:
    scripts/mosshop/build.sh {{quote(mosshop_dir)}}

# Everything this repo holds, on Moss Hop alone. Exits non-zero at the first failure.
check: _hook-enabled build mosshop
    cargo fmt --all --check
    cargo clippy -q --workspace --all-targets --profile {{profile}} -- -D warnings
    cargo test -q --workspace --profile {{profile}}
    {{gp}} self docs --check CLI.md
    {{gp}} self schemas --check schema
    {{gp}} baseline check --rom {{quote(mosshop_rom)}} baselines/mosshop
    {{gp}} baseline check baselines/gp-testrom
    mkdir -p {{quote(assets)}}
    {{gp}} assets lift {{quote(mosshop_rom)}} --map assets/mosshop.toml -o {{quote(assets)}} --force
    {{gp}} assets pack {{quote(assets)}} --check {{quote(mosshop_rom)}}
    {{gp}} model check templates/mosshop --assets {{quote(assets)}}
    {{gp}} model fmt --check templates/mosshop
    {{gp}} labels check templates/mosshop/mapping.toml
    just _interp-mosshop
    just play-web
    just play-web-arcade
    just editor
    just mechanic-page
    just web-parity
    {{gp}} template check templates/mosshop --rom {{quote(mosshop_rom)}}
    just originals
    scripts/test-hook.sh
    just check-vendor
    python3 -I -B -m unittest discover -s scripts/romguard -q
    just site-prose
    just check-roms
    @echo "just check: passed"

# Formats the Rust code.
fmt:
    cargo fmt --all

# Runs the Rust tests only.
test:
    cargo test --workspace --profile {{profile}}

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

# The ROM tier, on what this machine has, skipping with a notice what it
# hasn't: each NES ROM a game in the games checkout names boots in gp-nes as
# tetanes-core boots the file, and its junk copies record the same
# (gp-nes's registered_roms_boot_as_their_files_do); then, with Donkey Kong:
# Original Edition in $GP_ROM_DIR and the games checkout beside the repo
# ($GP_GAMES_DIR), Donkey Kong's module built from this tool's kit, and
# profiles_dk and clip_agree on it. CI has no ROM, so there it checks nothing.
check-roms:
    #!/usr/bin/env bash
    set -euo pipefail
    roms="${GP_ROM_DIR:-{{gp_home / "roms"}}}"
    games="${GP_GAMES_DIR:-{{justfile_directory() / ".." / "games"}}}"
    if [ ! -d "$roms" ]; then
        echo "check-roms: skipped, no ROM folder at $roms"
        exit 0
    fi
    export GP_ROM_DIR="$roms" GP_GAMES_DIR="$games"
    # Runs a test command, printing its notices and, on a failure, all of it.
    tier() {
        local out status=0
        out=$("$@" -- --ignored --nocapture 2>&1) || status=$?
        if [ "$status" -ne 0 ]; then echo "$out" >&2; exit "$status"; fi
        grep -E '^(profiles_dk|clip_agree):|identical|skipped' <<<"$out" || true
    }
    GP_CHECK_ROM_BOOT=1 tier cargo test -q -p gp-nes --profile {{profile}} --lib registered_roms_boot_as_their_files_do
    if [ ! -f "$games/donkey-kong/corpus.toml" ]; then
        echo "check-roms: profiles_dk and clip_agree skipped, no games checkout at $games"
        exit 0
    fi
    if ! {{quote(gp)}} --json roms scan 2>/dev/null | grep -q '"donkey-kong-oe"'; then
        echo "check-roms: profiles_dk and clip_agree skipped, no Donkey Kong: Original Edition in $roms"
        exit 0
    fi
    just _dk-module "$games"
    export GP_DK_MODULE={{quote(target / "modules" / "donkey-kong.wasm")}}
    tier cargo test -q -p gp-cli --profile {{profile}} --test profiles_dk --test clip_agree
    echo "check-roms: passed"

# Donkey Kong's normal profile against retro on its own model and module
# (crates/gp-cli/tests/profiles_dk.rs, ROM tier).
profiles-dk: build (_dk-module (env_var_or_default("GP_GAMES_DIR", justfile_directory() / ".." / "games")))
    GP_DK_MODULE={{quote(target / "modules" / "donkey-kong.wasm")}} cargo test -q -p gp-cli --profile {{profile}} --test profiles_dk -- --ignored --nocapture

# gp clip score and gp corpus score agree on Donkey Kong's corpus clips
# (crates/gp-cli/tests/clip_agree.rs, ROM tier).
check-clip-agree-roms: build (_dk-module (env_var_or_default("GP_GAMES_DIR", justfile_directory() / ".." / "games")))
    GP_DK_MODULE={{quote(target / "modules" / "donkey-kong.wasm")}} cargo test -q -p gp-cli --profile {{profile}} --test clip_agree -- --ignored --nocapture

# Donkey Kong's module, built from this tool's kit into target/modules/.
_dk-module games: build
    {{quote(gp)}} game kit -o {{quote(target / "module-kit")}} > /dev/null
    {{quote(gp)}} game build {{quote(games / "donkey-kong")}} --kit {{quote(target / "module-kit")}} -o {{quote(target / "modules" / "donkey-kong.wasm")}} > /dev/null

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

# The arcade player: gp_arcade_play.wasm built into target/play-web, an
# rbtapper capture's input (scripts/oracle/inputs/tapper-first.input)
# played in it under node, drawing every frame, with a save point restored
# on the way, then the same log replayed natively; every frame's picture
# hash must agree, and with the MAME capture of that log on this machine
# ($GP_HOME/work/oracle/tapper-first-a), MAME's too. Needs the owner's
# romset ($GP_ARCADE_RBTAPPER, $GP_ROM_DIR/arcade/rbtapper.zip or
# ~/Downloads/rbtapper.zip) and skips without it, so it can't run in CI.
play-web-arcade:
    #!/usr/bin/env bash
    set -euo pipefail
    zip=""
    for z in "${GP_ARCADE_RBTAPPER:-}" "${GP_ROM_DIR:+$GP_ROM_DIR/arcade/rbtapper.zip}" "$HOME/Downloads/rbtapper.zip"; do
        if [ -n "$z" ] && [ -f "$z" ]; then zip="$z"; break; fi
    done
    if [ -z "$zip" ]; then
        echo "play-web-arcade: skipped, no rbtapper romset on this machine"
        exit 0
    fi
    log=scripts/oracle/inputs/tapper-first.input
    scripts/web/build-play.sh --arcade {{quote(target / "play-web")}} > /dev/null
    work={{quote(gp_home / "work" / "play-web")}}
    mkdir -p "$work"
    node scripts/web/check-play-arcade.mjs {{quote(target / "play-web" / "gp_arcade_play.wasm")}} "$zip" "$log" "$work/arcade-web.txt" > /dev/null
    cargo build -q --profile {{profile}} -p gp-arcade-play --example pictures
    {{quote(target / profile_dir / "examples" / "pictures")}} "$zip" "$log" > "$work/arcade-native.txt"
    frames=$(wc -l < "$work/arcade-web.txt" | tr -d ' ')
    if ! cmp -s "$work/arcade-web.txt" "$work/arcade-native.txt"; then
        first=$(diff <(cat -n "$work/arcade-web.txt") <(cat -n "$work/arcade-native.txt") | sed -n 2p)
        echo "play-web-arcade: gp_arcade_play.wasm's picture hashes differ from the native replay's, first at: $first" >&2
        exit 1
    fi
    mame="no MAME capture of it here"
    tsv={{quote(gp_home / "work" / "oracle" / "tapper-first-a" / "frames.tsv")}}
    if [ -f "$tsv" ]; then
        if cmp -s "$work/arcade-web.txt" <(tail -n +2 "$tsv" | cut -f9); then
            mame="and MAME's capture agrees on every frame too"
        else
            echo "play-web-arcade: the picture hashes differ from MAME's capture ($tsv)" >&2
            exit 1
        fi
    fi
    echo "play-web-arcade: tapper-first ($frames frames) played in gp_arcade_play.wasm and replayed natively: every frame's picture hash agrees, $mame"

# The mechanic page in a headless browser on Moss Hop, in a GP_HOME made
# for the run: play, record a clip with a note and hand it off, the agent's
# answer from gp mechanic say, FREE PLAY, and the page's message when the
# player isn't built (scripts/web/check-mechanic.mjs). Then, with the
# owner's rbtapper romset on this machine, the same on Tapper: coin, start,
# a clip, gp mechanic say, and ON THE MODEL's refusal; skipped without it.
mechanic-page: build mosshop
    #!/usr/bin/env bash
    set -euo pipefail
    scripts/web/build-play.sh {{quote(target / "play-web")}} > /dev/null
    if [ -z "${PLAYWRIGHT_FROM:-}" ] && [ ! -d scripts/web/node_modules/playwright-core ]; then
        npm ci --prefix scripts/web --silent --no-audit --no-fund
    fi
    browser=$(scripts/inspector/fetch-browser.sh {{quote(gp_home)}})
    # The arcade run, with the owner's rbtapper romset when it is here.
    arcade=()
    for z in "${GP_ARCADE_RBTAPPER:-}" "${GP_ROM_DIR:+$GP_ROM_DIR/arcade/rbtapper.zip}" "$HOME/Downloads/rbtapper.zip"; do
        if [ -n "$z" ] && [ -f "$z" ]; then
            scripts/web/build-play.sh --arcade {{quote(target / "play-web")}} > /dev/null
            arcade=(--arcade-rom "$z" --arcade-wasm {{quote(target / "play-web" / "gp_arcade_play.wasm")}})
            # The ROM set picker on Tapper: a games checkout with tapper/.
            for g in "${GP_GAMES_DIR:-}" ../games; do
                if [ -n "$g" ] && [ -f "$g/tapper/roms.toml" ]; then arcade+=(--arcade-games "$g"); break; fi
            done
            break
        fi
    done
    node scripts/web/check-mechanic.mjs --gp {{quote(gp)}} --rom {{quote(mosshop_rom)}} \
        --wasm {{quote(target / "play-web" / "gp_play.wasm")}} --browser "$browser" ${arcade[@]+"${arcade[@]}"} \
        --shots {{quote(gp_home / "work" / "mechanic-page")}}

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
# recordings here; that needs `gp input-log generate` (wave 9).
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
# Optional, not part of .
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

# The page, built from the games repo beside this one.
page out="target/page":
    just build
    GP={{quote(gp)}} scripts/web/build-page.sh ../games/donkey-kong {{quote(out)}}

# The public page (docs/web.md, "The public page"), built from the games
# repo beside this one into $GP_HOME/work/page/ and checked to hold no game
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
    GP={{quote(gp)}} scripts/web/build-page.sh ../games/donkey-kong "$out"
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

# The release check (docs/web.md, "Deploying the public page"): the page built at
# this tip, its game-data check, the headless check under its headers, the
# lift's tests, gp package scan on the game, and on a pass a record of the
# page's digest, which `just web-page-deploy` needs. `--full` reuses nothing.
# The page's release check, which a deploy needs a pass of.
web-page-release *args: build
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{args}}" in ""|--full) ;; *) echo "web-page-release: give nothing or --full" >&2; exit 2 ;; esac
    WEB_TOOLS=$(scripts/web/fetch-tools.sh {{quote(gp_home)}})
    export WEB_TOOLS
    GP={{quote(gp)}} scripts/web/release-check.sh {{args}}

# Runs the full release check (`--full`: nothing reused), shows what would
# upload, asks for the worker name, then runs `npx wrangler deploy --assets`
# (needs `npx wrangler login` once; docs/web.md). `just web-page-deploy list`
# stops before the upload, with wrangler's dry run.
# Deploys the page after the full release check and a typed confirmation.
web-page-deploy *args: build
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{args}}" in ""|list) ;; *) echo "web-page-deploy: give nothing or list" >&2; exit 2 ;; esac
    WEB_TOOLS=$(scripts/web/fetch-tools.sh {{quote(gp_home)}})
    export WEB_TOOLS
    flag=""; [ "{{args}}" = list ] && flag="--list"
    GP={{quote(gp)}} scripts/web/deploy.sh $flag

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
    {{quote(gp)}} assets lift {{quote(mosshop_rom)}} --map assets/mosshop.toml -o "$assets" --force > /dev/null
    out={{quote(target / "web" / "mosshop")}}
    scripts/web/build-preview.sh "$out" templates/mosshop "$assets" > /dev/null
    scripts/web/parity.sh "$out" templates/mosshop "$assets" scripts/web/parity/mosshop.toml | grep -v '^{'

# Build a registry in a folder on this machine and the page that installs from it.
local-registry:
    GP_HOME={{quote(gp_home)}} scripts/web/local-registry.sh

# Serve them at http://127.0.0.1:8377/p?g=lex00/donkey-kong until stopped.
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
# capture, so the proposed model must equal the template's), exported as a
# sheet with a palette and sidecar, and imported back with its checks.
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
    {{gp}} sprites export templates/mosshop --out "$work/export"
    {{gp}} sprites import templates/mosshop --from "$work/export" --out "$work/theme"

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
