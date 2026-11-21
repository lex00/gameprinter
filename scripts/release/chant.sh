#!/usr/bin/env bash
# The chant that releases gp (chant.config.ts): the justfile's release
# recipes and the release build jobs run it. .github/workflows/release.yml
# runs the chant that rendered it, so after changing this pin run
# `just release-workflow` to render it again with the same one.
exec npx --yes @intentius/chant@0.124.110 "$@"
