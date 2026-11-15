# Registry list files

A registry's list files are the three files `gp registry reindex` rewrites and re-signs: `catalog.toml`, `by-game/NS/GAME.toml` and `yanks.toml`. Every other key is written once ([packages](../packages.md#where-packages-are-kept)). This page is the by-game file, which the page reads for a game's theme list, with its optional `recheck`. The Rust types are in `crates/gp-package/src/registry.rs` (`ByGame`, `ByGameLine`, `Recheck`); they are the definition. The format is `"1"`, the same as every registry file.

## by-game/NS/GAME.toml

```toml
format = "1"
registry = "gp-registry"
signed = "2026-11-15T05:09:12Z"
expires = "2026-11-22T05:09:12Z"

[[line]]
package = "friend/night"
version = "0.9.0"
id = "c07c62ff4668fe419c24ae0f3101c121ebbe3727592095234eeec9b789717b73"
title = "Night"

[line.recheck]
date = "2026-11-15"
result = "fail"
reason = "the page would leave it out of Settings, Theme for Donkey Kong, because its files have a mistake in them (not a look (no `gp-look 1` line))"
```

| Key | Means |
|---|---|
| `format`, `registry` | `"1"`, and the registry's name from registry.toml |
| `signed`, `expires` | When the file was written and when a reader holding the root stops trusting it, RFC 3339 UTC |
| `[[line]]` | One published content version for the game: `package` (NS/NAME), `version`, `id` (the package id) and `title` |
| `[line.recheck]` | Optional. The last recheck of that version for the game: `date` (UTC, `YYYY-MM-DD`), `result` (`pass` or `fail`) and `reason` (in words a player reads). Absent when no recheck was recorded |

### The recheck

`gp theme recheck GAME THEME --registry TARGET` writes it. GAME and THEME are package names (`lex00/donkey-kong`, `friend/night`). It finds the version of THEME the game's list offers, fetches it by its id, reads its files as the page's lift reads them over a ROM of the game on this machine (`--rom`, or one in `$GP_ROM_DIR`; the same reading `gp registry check --for GAME --theme THEME` does), and records the outcome in that version's line, replacing an earlier recheck. A problem the lift reports is a `fail`, with the problem's words as the reason; a lift that draws the theme is a `pass`. A recheck that can't run (no ROM, no game folder here) records nothing and says so. A failed recheck is recorded and the command exits 1. With `--dry-run` it prints the change and writes nothing; `--key` is the registry's signing key for a signed registry; on a `git:` target it commits "Record a recheck of NAME@VERSION for GAME".

The recheck is inside the by-game file, so the file's signature covers it: a signed registry's file is re-signed with the signing key when a recheck is written, and a reader with the root refuses a file whose recheck was changed after signing, as it refuses any other edit. `gp registry reindex` keeps each line's recheck when it rewrites the file, for the same package and id; a new version starts with none.

Readers:

| Reader | What it does with `recheck` |
|---|---|
| The page (gp-registry.wasm, `gp_web_registry::choose`) | Still offers the theme. A failed recheck comes with the offer (`recheck` in the list answer), and the page says "Theme that failed a recheck: TITLE, on DATE: REASON." in the line under the header and in About. A passed one isn't passed on |
| `gp registry check TARGET --for GAME` | Adds "its recheck on DATE failed: REASON" to the theme's line, and `recheck` to its JSON |
| An older gp or page | Passes over it. serde reads a table with keys it has no field for, and no registry type sets `deny_unknown_fields`; `an_older_reader_passes_over_the_recheck` reads such a line with the four-key line type from before |

The themes index of a ROM theme (`themes/index/GAME.toml`, [theme publishing](../theme-publishing.md#recheck)) is another file. `gp theme recheck` without `--registry` still writes only that index, and a failed recheck there still records nothing in it.

## Tests

| Test | What |
|---|---|
| `gp_package::registry::tests::a_recheck_is_recorded_in_the_by_game_line_and_a_reindex_keeps_it` | A made-up failed recheck written as `[line.recheck]`, read back, kept by a reindex, replaced by a pass; `maybe` and a version the list doesn't name refused |
| `gp_package::registry::tests::a_recheck_on_a_signed_registry_needs_the_signing_key_and_is_signed` | Refused with no key; signed with a key that isn't the registry's, the reader with the root refuses the file |
| `gp_package::registry::tests::an_older_reader_passes_over_the_recheck` | The line type from before reads a line with a recheck; today's reads one with a key from a newer gp |
| `gp_web_registry::tests::a_failed_recheck_comes_with_the_offer` | The page's list answer offers both themes, the failed one with its recheck, the passed one with none |
| `registry_recheck::tests::*` (gp-cli) | Findings come to pass, fail or nothing; a failed recheck recorded, a dry run writing nothing, and the check line's words |
