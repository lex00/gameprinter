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
    {{gp}} baseline check --rom {{quote(mosshop_rom)}} baselines/mosshop
    mkdir -p {{quote(assets)}}
    {{gp}} assets lift {{quote(mosshop_rom)}} --map assets/mosshop.toml -o {{quote(assets)}} --force
    {{gp}} assets pack {{quote(assets)}} --check {{quote(mosshop_rom)}}
    {{gp}} model check templates/mosshop --assets {{quote(assets)}}
    {{gp}} model fmt --check templates/mosshop
    just _interp-mosshop
    just play-web
    just play-web-arcade
    just mechanic-page
    {{gp}} template check templates/mosshop --rom {{quote(mosshop_rom)}}
    scripts/test-hook.sh
    python3 -I -B -m unittest discover -s scripts/romguard -q
    @echo "just check: passed"

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
            break
        fi
    done
    node scripts/web/check-mechanic.mjs --gp {{quote(gp)}} --rom {{quote(mosshop_rom)}} \
        --wasm {{quote(target / "play-web" / "gp_play.wasm")}} --browser "$browser" ${arcade[@]+"${arcade[@]}"}

_interp-mosshop:
    #!/usr/bin/env bash
    set -euo pipefail
    for f in baselines/mosshop/*.toml; do
        echo "$(basename "$f"): $({{quote(gp)}} interp compare templates/mosshop --rom {{quote(mosshop_rom)}} --baseline "$f" --assets {{quote(assets)}} | tail -1)"
    done

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
mame-record ROMSET LOG OUT SHOT_EVERY="30":
    GP_HOME={{quote(gp_home)}} scripts/oracle/mame-record.sh {{mame_image}} {{quote(ROMSET)}} {{quote(LOG)}} {{quote(OUT)}} {{quote(SHOT_EVERY)}}

# The page, built from the games repo beside this one.
page out="target/page":
    just build
    GP={{quote(gp)}} scripts/web/build-page.sh ../games/donkey-kong {{quote(out)}}

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
