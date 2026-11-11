# Where the migration differs from migration.md

Each of these was the owner's decision, on 2026-11-08 or 2026-11-09.

| Step | The plan | What was done |
|---|---|---|
| 1, 2.2 | Registries on R2, with a domain | A public git repo, `lex00/rivetrodeo`, at its raw GitHub address. S3 staging and an empty official bucket also exist |
| 2.2 | An official root key, made by the owner in a ceremony, and an official registry | Closed without one. The live page and `rivetrodeo` trust the staging key at `~/.gameprinter/keys/staging`. `scripts/root-ceremony.sh` is there if that changes |
| 3.4 | Loops brought over | `gp loop` left out: neither landing used it |
| 4.3 | The old repo archived read-only | It has no remote, so its archive is the read-only bundle `~/Documents/archives/game-printer-9003564a.bundle`, with its full history. The working folder is unchanged |

Signing is off for `lex00/rivetrodeo`, by the owner's decision on 2026-11-09: there is one publisher, and signing shouldn't block work until there is another. The code and its tests stay, and a source opts in by having a root key in `scripts/web/sources.toml`. Hashes are still checked. `scripts/web/resign-registry.sh` is for a signed registry, whose list files expire after 7 days.

On 2026-11-11 the owner retired Rivet Rodeo, the open version, and replaced `lex00/rivetrodeo` with a new public, unsigned registry, `lex00/gp-registry`. The page moved to `nes-donkey-kong.intentius.workers.dev` and plays only a ROM the visitor loads, with the clay theme `lex00/turnips` (its own repo) installed beside the game. Later that day the owner retired the old page too: the Worker `rivet-rodeo` was deleted, and `lex00/rivetrodeo` was archived and then deleted, with its source taken out of `scripts/web/sources.toml`. Later still, Rivet Rodeo itself was deleted: `open/` and the paper theme went from the Donkey Kong package (1.0.9), and the page stopped filling what a JU ROM lacks from it. A JU ROM now plays JU's own game: no 50m and no stage-clear scene, with the assets only those use left blank.
