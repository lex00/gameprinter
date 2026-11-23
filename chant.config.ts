// How gp is released, with chant's release train
// (https://intentius.io/chant/cli/release/cut/), as INTENTIUS/choudoufu and
// behold do.
//
// `just release` dispatches .github/workflows/release.yml from main, which
// `just release-workflow` renders from this file: never edit that file by
// hand. The run takes main's head once its checks on GitHub are green, has
// each build job below build gp on its own runner (scripts/release/
// chant-build.sh, which runs scripts/release/build.sh), cuts the release with
// the tag v<version>, and publishes the archives and SHA256SUMS to a GitHub
// release whose notes are CHANGELOG.md's section for the version, then the
// formula to the Homebrew tap lex00/tap. The cut stamps the version into the
// files release.version names, so no commit bumps a version before a
// release. Once the run passes, `just release` puts the released version on
// main (scripts/release/version-on-main.sh), since Claude Code's plugin
// marketplace reads the plugin from main; the train's after-release job
// can't, having only contents: read. `just release-plan`
// prints what the next release would cut, and cuts, tags and pushes nothing.
//
// Only the owner starts a release. An agent never runs `chant release cut
// --yes` or `chant release publish`, and never pushes a release tag.
import { existsSync } from "node:fs";

// The Rust target each runner builds, named as the release's asset.
const builds = [
  { name: "darwin-arm64", runner: "macos-15", target: "aarch64-apple-darwin" },
  // No Intel Mac build for now (the owner, 2026-11-22): its runner,
  // macos-15-intel, took 48 minutes where the others take about 20, and the
  // release waits for every build. Build gp from source there.
  { name: "linux-amd64", runner: "ubuntu-24.04", target: "x86_64-unknown-linux-gnu" },
  { name: "linux-arm64", runner: "ubuntu-24.04-arm", target: "aarch64-unknown-linux-gnu" },
  // Windows waits on chant loading chant.config.ts from a Windows path
  // (row chant-windows-config): its Node ESM loader is given D:\\... where it
  // needs a file:// URL, so chant release stamp fails there.
].map(({ name, runner, target }) => ({
  name,
  runner,
  build: `bash scripts/release/chant-build.sh ${target}`,
  outputs: [`dist/gp-${target}.${target.includes("windows") ? "zip" : "tar.gz"}`],
  // The runner's own rustup installs rust-toolchain.toml's Rust
  // (chant-build.sh); Node builds the editor.
  toolchains: [{ name: "node", version: "24.13.1" }],
}));

// The files that carry gp's version. The plugin's two (docs/plugin.md) are
// stamped once they are in the tree, so the plugin is always gp's version.
const plugin = [
  { path: "plugin/.claude-plugin/plugin.json", field: "version" },
  { path: ".claude-plugin/marketplace.json", field: "plugins[].version" },
].filter(({ path }) => existsSync(path));

export default {
  release: {
    tag: "v{version}",
    // v0.1.0 and v0.1.1 were released by hand, before chant.
    legacyTags: ["v"],
    green: "forge-checks",
    version: {
      files: [
        // gp-cli's own version only, not its path dependencies' requirements.
        { path: "crates/gp-cli/Cargo.toml", pattern: '^version = "([^"]+)"' },
        { path: "Cargo.lock", format: "cargo-lock", crates: ["gp-cli"] },
        ...plugin,
      ],
    },
    // Fails the cut, before anything is tagged, when CHANGELOG.md has no
    // section for the version.
    check: ["bash scripts/release/changelog.sh"],
    workflow: {
      trigger: "dispatch",
      file: ".github/workflows/release.yml",
      install: false,
      build: false,
      toolchains: ["node"],
      builds,
      env: { HOMEBREW_TAP_KEY: "${{ secrets.HOMEBREW_TAP_KEY }}" },
    },
    artifacts: [
      { name: "binaries", builtBy: builds.map((b) => b.name) },
      { name: "checksums", checksums: "dist/SHA256SUMS", of: ["binaries"] },
    ],
    publish: [
      {
        type: "forge",
        forge: "github",
        repo: "lex00/gameprinter",
        assets: ["artifact:binaries", "artifact:checksums"],
        sbom: false,
        notes: { file: "CHANGELOG.md", heading: "## gp {version}" },
      },
      // After the forge target, so the archives download when the formula
      // is revealed. A deploy key of lex00/homebrew-tap, in the repository
      // secret HOMEBREW_TAP_KEY, writes it.
      {
        type: "homebrew",
        tap: "lex00/homebrew-tap",
        formula: "Formula/gp.rb",
        template: "scripts/release/homebrew/gp.rb",
        assets: ["artifact:binaries"],
        url: "https://github.com/lex00/gameprinter/releases/download/{tag}/{name}",
        sshKeyEnv: "HOMEBREW_TAP_KEY",
      },
    ],
  },
};
