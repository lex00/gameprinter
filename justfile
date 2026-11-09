# Game Printer's tool repo. `just check` is the check for every change.

gp_home := env_var_or_default("GP_HOME", env_var("HOME") / ".gameprinter")
target := env_var_or_default("CARGO_TARGET_DIR", justfile_directory() / "target")
gp := target / "debug" / "gp"
mosshop_dir := target / "mosshop"
mosshop_rom := mosshop_dir / "mosshop.nes"
assets := gp_home / "work" / "assets" / "mosshop"

# Fails unless the ROM-blocking pre-commit hook is on in this clone.
_hook-enabled:
    @[ "$(git config core.hooksPath)" = ".githooks" ] || { echo "The ROM-blocking hook isn't enabled. Run: just setup" >&2; exit 1; }

# Turns on the ROM-blocking hook.
setup:
    git config core.hooksPath .githooks

build:
    cargo build -q -p gp-cli

mosshop:
    scripts/mosshop/build.sh {{quote(mosshop_dir)}}

# Everything this repo holds, on Moss Hop alone. Exits non-zero at the first failure.
check: _hook-enabled build mosshop
    cargo fmt --all --check
    cargo clippy -q --workspace --all-targets -- -D warnings
    cargo test -q --workspace
    {{gp}} baseline check --rom {{quote(mosshop_rom)}} baselines/mosshop
    mkdir -p {{quote(assets)}}
    {{gp}} assets lift {{quote(mosshop_rom)}} --map assets/mosshop.toml -o {{quote(assets)}} --force
    {{gp}} assets pack {{quote(assets)}} --check {{quote(mosshop_rom)}}
    {{gp}} model check templates/mosshop --assets {{quote(assets)}}
    {{gp}} model fmt --check templates/mosshop
    just _interp-mosshop
    {{gp}} template check templates/mosshop --rom {{quote(mosshop_rom)}}
    scripts/test-hook.sh
    python3 -I -B -m unittest discover -s scripts/romguard -q
    @echo "just check: passed"

_interp-mosshop:
    #!/usr/bin/env bash
    set -euo pipefail
    for f in baselines/mosshop/*.toml; do
        echo "$(basename "$f"): $({{quote(gp)}} interp compare templates/mosshop --rom {{quote(mosshop_rom)}} --baseline "$f" --assets {{quote(assets)}} | tail -1)"
    done

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
