# Where the migration differs from migration.md

Each of these was the owner's decision, on 2026-11-08 or 2026-11-09.

| Step | The plan | What was done |
|---|---|---|
| 1, 2.2 | Registries on R2, with a domain | A public git repo, `lex00/rivetrodeo`, at its raw GitHub address. S3 staging and an empty official bucket also exist |
| 2.2 | An official root key, made by the owner in a ceremony, and an official registry | Closed without one. The live page and `rivetrodeo` trust the staging key at `~/.gameprinter/keys/staging`. `scripts/root-ceremony.sh` is there if that changes |
| 3.4 | Loops brought over | `gp loop` left out: neither landing used it |
| 4.3 | The old repo archived read-only | It has no remote, so its archive is the read-only bundle `~/Documents/archives/game-printer-9003564a.bundle`, with its full history. The working folder is unchanged |

The registry's list files expire after 7 days. `scripts/web/resign-registry.sh` signs them again.
