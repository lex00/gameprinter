# Vendored code

Code from other projects that Game Printer builds on, imported by a script so every import can be reproduced and checked.

| Directory | What | Imported by | License |
|---|---|---|---|
| `tetanes-core/` | The NES emulator core that `crates/gp-nes` runs games on | `scripts/vendor/tetanes-core.sh` | MIT OR Apache-2.0 |

## Rules

- Each import script downloads a pinned release, checks its hash, drops anything that may not be committed here (such as test ROMs), adds the license files, and writes an `UPSTREAM.md` saying exactly what was taken.
- Our changes to vendored code are kept as one patch file next to it (`third_party/<name>.patch`), which the import script applies after importing. Next to that, `third_party/<name>.upstream.sha256` lists the SHA-256 of every file the import took, before the patch. So a new import keeps every change we made, including ones that only change behaviour and would build fine without them.
- `scripts/vendor/<name>.sh --check` confirms the vendored tree is exactly the import plus the patch, and that the patch file is what `--update-patch` would write. It works offline, by taking the patch back off a copy of the tree and comparing every file with its listed hash. `just check` runs it, and `scripts/test-vendor.sh` checks that it catches edits.
- Each change to vendored code is its own commit, saying why, with the code change and the regenerated patch file together. `git log -- third_party/<name>.patch` then shows the fork's whole story.
- Vendored crates are path dependencies, not workspace members (`exclude` in the root `Cargo.toml`). Their own tests and lints stay upstream's: some of their tests need files that aren't imported. Our tests of the behavior we rely on live in our crates.
- The repo holds only text, vendored code included. A binary data file a vendored crate compiles in is left out of the import (the script's `EXCLUDE`) and carried in the patch as a text file, which the crate's build script turns back into the same bytes; see the tetanes-core data files below.

## Changing vendored code

1. Edit the files under `third_party/<name>/`.
2. Run `scripts/vendor/<name>.sh --update-patch`. It downloads the pinned release (cached in `$GP_HOME/work/<name>`) to diff against, and rewrites the patch file.
3. Commit the change and the patch file together. For `tetanes-core`, read the runtime identity section below first.

Commits that changed vendored files without updating the patch (from a branch made before the patch file existed, say) fail `--check` once merged. Running `--update-patch` after the merge folds them in; commit the result.

## Updating to a new release

1. Change the version and pins at the top of the import script.
2. Run it. It imports the new release, rewrites the hash list, and applies the patch.
3. If some of the patch doesn't apply, the script says which files have a `.rej` file next to them. Make those changes by hand, delete the `.rej` files, and run the script with `--update-patch`.
4. Review the diff and commit the import, the hash list and the patch file together.
5. Run `just check` and `just docker-check`, then `just check-roms`: a new release changes the runtime identity, so stored recordings show as stale until they're re-recorded ([recording.md](../docs/recording.md)).

## tetanes-core and the runtime identity

Every recording names the runtime that made it, and `gp-nes` derives that name when it builds (`crates/gp-nes/build.rs`): the upstream version and commit from `tetanes-core.upstream.sha256`, a BLAKE3 hash of `tetanes-core.patch`, and `gp-nes`'s own `BEHAVIOUR_VERSION`. Any change to the patch file therefore changes the runtime identity, and replay treats recordings made before it as stale. That's intended for changes to emulation. A patch that can't change behaviour (a comment, an accessor or a hook nothing in a recording depends on) keeps `BEHAVIOUR_VERSION` as it is, and recordings made before it keep their checkpoints where each one round-trips under the new build ([recording.md](../docs/recording.md#when-the-runtime-changes)). Keep any state such a hook adds out of the save state (`#[serde(skip)]`), or the checkpoints won't round-trip.

## tetanes-core's data files

Upstream ships two binary files that tetanes-core compiles in with `include_bytes!`. The import leaves them out, and the patch adds each as a text file next to where it was:

| Text file | Replaces | Format |
|---|---|---|
| `ntscpalette.txt` | `ntscpalette.pal` (1,536 bytes), `Ppu::NTSC_PALETTE` | 512 lines of `emphasis colour rgb`, e.g. `0 15 B21B71`: the 64 PPU colours ($00-$3F, hex) under each of the 8 emphasis settings (PPUMASK's red, green and blue bits as 0-7), RGB in hex. Line order is the table's order, entry `emphasis * 64 + colour` |
| `game_db.txt` | `game_db.dat` (14,864 bytes), the cartridge database `Cart::lookup_info` searches | 2,683 lines of `crc=001388B3 region=Pal mapper=4 submapper=0`: the CRC32 of a ROM's PRG and CHR data (hex), the region (`Ntsc`, `Pal`, `Dendy` or `Auto`), the mapper and the submapper. Sorted by CRC, since the lookup is a binary search |

Lines starting with `#` are comments. `build.rs` reads both, writes `ntscpalette.pal` and `game_db.dat` to `OUT_DIR`, where the `include_bytes!` calls read them, and fails the build unless each has the SHA-256 of upstream's file (pinned in `build.rs`). For the palette that's the bytes in line order. For the database it's what `fs::save_version` writes for a `Vec<GameInfo>`: the `TETANES\x1a` magic, format version `1`, then the bincode (legacy config) encoding deflate-compressed by `flate2` at the default level. `flate2`'s default (Rust) backend reproduces upstream's compressed bytes exactly; a `flate2` or `miniz_oxide` update that compresses differently fails the build there, not at run time.

When a new upstream release changes either file, regenerate the text from it (decompress `game_db.dat` and decode the entries; read the palette three bytes at a time) and update the pinned hashes in `build.rs`.
