# Evaluation of game-printer before the new repos (Step 1)

Written 2026-11-08 against `game-printer` at 9003564a. The live page is built from 6af9e8b3; the only commit after it changes ROADMAP.md. This moves to `gameprinter/docs/evaluation.md` when phase 0.2 creates the repo.

Nothing in `game-printer` was changed. `git status` shows only `scripts/web/deploy/.wrangler/`, which was there before this work. Not run: a full gate or corpus score, the page check, a build, or anything against a remote.

## 1. What game-printer is

A Rust workspace (toolchain 1.99.1) of 38 crates, 4 hook crates and one vendored emulator core (`third_party/tetanes-core`). It turns NES games into models, scores those models against the original, and plays them in a browser.

| Part | What it is |
|---|---|
| `gp` (`gp-cli`, about 207,000 lines) | About 255 commands, linking every other crate. It is one crate with one 4,862-line `main.rs` |
| Simulator and interpreter | `gp-sim` (19,500 lines), `gp-interp` (29,700), `gp-model` (15,700), `gp-present` (9,700), `gp-sound`, `gp-theme` |
| The original's runtime | `gp-nes` (19,600), on `tetanes-core` (36,800), held to Mesen by `gp-oracle` |
| Lift and packages | `gp-assets` (15,900, of which 6,696 is SMB and Mega Man 2 code), `gp-package` (1,052), `gp-evidence` (7,100) |
| Hooks | `gp-exts` links every game's hook crate into gp. Donkey Kong's is `templates/donkey-kong-oe/ext` (11,300 lines, 17 hooks) |
| Games | 11 template folders. Each has model, mapping, clips, evidence, kits, themes, and hooks for some |
| Public page | Static files built by `scripts/web/build-page.sh`, deployed by `scripts/web/deploy.sh` with wrangler 4.157.0 to the Worker `rivet-rodeo`. 268 files, 13.0 MB. `gp_web.wasm` is 3,594,830 bytes and `gp-web-lift.wasm` 3,875,990 |
| Checks | `just check` (about 40 steps), `just check-cycle`, `just check-release`, `just check-roms`. The page has `scripts/web/release-check.sh` |
| Merge gate and agents | `.githooks/merge-gate` needs a pass record from `scripts/merge-check.sh`. `gp model gate` lands template patches in a gate home (`$GP_HOME/gate`). Workers use `gp worker env`. The gate publishes `$GP_HOME/bin/gp` |
| Hosting | No git remote and no hosted CI today. Every check runs on this machine |

## 2. Where the design docs disagree

`migration.md` wins each of these. The ones marked "needs you" are also in section 6.

| # | Where | It says | migration.md says | Effect |
|---|---|---|---|---|
| 1 | packaging 6.1, choice 14 | One import commit holding `crates/`, `scripts/`, `docs/`, `editor/` and more | Crate by crate as a phase needs it; never the editor | Follow migration.md |
| 2 | packaging 6.1, choice 10 | No games repo | A games repo with CI | Follow migration.md |
| 3 | packaging 6.1 | No hosted CI is needed; nothing depends on a git host | Hosted CI jobs plus a runner on your machine | Needs you |
| 4 | packaging pick 17, choice 39 | First milestone plays on today's public page | A new page at its own address | Follow migration.md |
| 5 | publishing-paths and cli-gui headers | Old step names (M1, S3, S4, "T2 before the import") | O and T tracks hang off the phases; T2 is at 3.4 | Follow migration.md |
| 6 | packaging pick 2, choice 50 | The module kit stays inside each tool release, not on crates.io | T3: "the module kit on crates.io"; a hook crate builds "from crates.io alone" | Looks like a slip in migration.md. Can wait until T3, but needs you then |
| 7 | packaging 6.13 | Signatures are checked in `sshsig.js` with WebCrypto | 1.1: gp-package built to Wasm checks ids and signatures | Follow migration.md, which adds an ed25519 dependency to gp-package |
| 8 | packaging 6.11 | Version files are signed by a namespace key; the online signing key signs only index files | CI publishes "with the registry's online signing key" | CI would need both keys. Needs a decision at 2.2 |
| 9 | architecture.html | Phase 2's `connect-src` names the official domain only | 2.1 runs against staging | The page's policy must name staging in 2.1 and official from 2.2 |
| 10 | migration.md with itself | 1.3 brings "clips' presses" only | 3.3 gates "on the same clips", and 4.4 moves curves out of `clips.toml` | 1.3 can't drop `clips.toml` if 3.3 and 4.4 still need it. Needs you |
| 11 | packaging 6.2 example | `[accepts.levels]` names `open/level-kit/kit.toml`; JU `asset_map_sha256` is `eaa7fd...` | | No such kit exists in `templates/rivet-rodeo`. JU's real pin is `4a968562...`. Use measured values only |

## 3. What comes in, by phase

Line counts are of the files today.

| Step | Comes in, from `game-printer` | Left behind |
|---|---|---|
| 0.2 tool | `gp-package`, `gp-evidence`, `gp-runtime`, `gp-model`, `gp-sim`, `gp-cycles`, `gp-sound`, `gp-sound-bank`, `gp-theme`, `gp-present`; `gp-assets` and `gp-interp` with the SMB and MM2 code cut out; `gp-web` and `gp-web-lift` with no hooks yet; `gp-nes` and `third_party/tetanes-core`; a new small `gp-cli`; `rust-toolchain.toml` | `gp-exts`, every hook crate, `gp-gen-bevy` whole, `gp-mcp`, `gp-host`, capture, track, sprites, gaps, explore, oracle and the rest |
| 0.2 Moss Hop | `templates/mosshop`, `games/mosshop`, `assets/mosshop.toml`, `baselines/mosshop`, `coverage/mosshop.toml`, `scripts/mosshop` | Every other template, map and baseline |
| 0.2 guards | `.githooks/pre-commit`, `binary-allowlist`, `scripts/romguard`, `scripts/test-hook.sh`, `gp package scan` | `merge-gate` waits for 3.4 |
| 0.3 games | The same hook and scans, with empty folders | Everything else |
| 1.1 | Parts of `theme_publish.rs` (2,008 lines) and `gp-package/src/index.rs`, grown into the registry core; `sshsig.js` can be read from branch `rtEight/download` (3fb585af) as a reference | Today's theme index format |
| 1.3 | `templates/donkey-kong-oe` less `evidence/`, `loop.toml`; `templates/rivet-rodeo` as `open/`; `scripts/rivet-rodeo/*`; `assets/donkey-kong-oe.toml`, `donkey-kong-ju.toml`, `rivet-rodeo.toml`; the derive step (`template_derive.rs`, 311 lines); the JU rule check (`scripts/check/ju_rom.py`). Measured at 237 files and 3.18 MB before dropping `loop.toml`, close to migration.md's estimate | Evidence, ROMs, lifted files, built modules, output |
| 2.1 | `scripts/web/public/*` (with `check-page.mjs`, `pictures.txt`), `scripts/web/page/*` less `bevy.html`, `build-page.sh`, `build-preview.sh`, `build-lift.sh`, `reuse.py`, `fetch-tools.sh`, `release-check.sh`, `scripts/inspector/fetch-browser.sh`, `_headers` | `scripts/web/bevy-web`, `build-bevy.sh`, Bevy's half of `parity.*` |
| 3.1 to 3.3 | `gate.rs` (4,105), `scoring_corpus.rs` (2,118), corpus code, `gp-wasm-check`, in the inventory's order | Whatever the inventory doesn't need |
| 3.4 | Worker and gate commands, `merge-check.sh`, `merge-gate` | The editor, `gp serve`'s 41,000 lines until T1 asks for parts |

Never brought in: Bevy and `gp-gen-bevy`'s generator, the editor, the suspended games' templates, maps and baselines, and the SMB family's level builders.

## 4. Parity baselines

| Result to match | How it's produced | Current value | State |
|---|---|---|---|
| Page check | `scripts/web/release-check.sh` (`check-page.mjs`) | PASS on bundle `6af9e8b3-dirty-e487caca`: 24 sections, 360 ok or kept lines, no skipped ROM steps. From `$GP_HOME/work/page-release/last-release-check.log`, not rerun today | Good baseline. Sections listed in the log |
| Reference pictures | `scripts/web/public/pictures.txt`, compared by the page check | 13 moments: Rivet Rodeo 7, Original Edition 5, JU 1 | Gap: `oe-stand` and `oe-walk` are `pending`, so they are played but not compared. 11 are really compared |
| Bundle's game-data check | `gp template served` in `build-page.sh` | No game data: 268 files against the OE ROM, 76 lifted files, 565 pictures | Good |
| Package scans | `gp --json package scan templates/T` | `rewrite` empty for `rivet-rodeo` and `donkey-kong-oe` on the last release check | Good |
| OE lift hash | `gp assets lift ROM --map assets/donkey-kong-oe.toml -o OUT`, then the digest in docs/templates.md | `3471247780a92d9af74e2d7fd2713926229df71f2e94bb45162e5221bba129cc`, 76 files. Measured today and equal to the pin | Good |
| Rivet Rodeo lift hash | `gp template build templates/rivet-rodeo -o OUT` | `b4a9838176c0b8c8fac0a47ebb885172b9bbaca18fabab7b7330e306646f0cfa`, reported by the tool today and equal to the pin. A plain digest of the whole output folder gives another value (`b240d2...`), so compare the tool's value | Good, with that note |
| JU lift hash | No command or pin exists. The page lifts JU only in the browser | Measured today with the deployed `gp-web-lift.wasm`, in memory, as the page check does: 1,198 files, digest `df9ac62b56a0fa6a0058645c51a80aabcab16ad63c68565ca9e4449204534950`. Same way for OE: 1,217 files, `8f1a88c8bb8be6e47774ad60e9aeffb4371200a0de6e7a1f52081324042e4e61` | Gap: no pin. These digests are over everything the page lift writes, drawn looks included, so 1.3 compares file by file. Per-file list in `page-lift-hashes.json` beside this file |
| Donkey Kong gate scores | Evidence record; `gp corpus score` to make a new one | Last record: 111 of 111 exact, fidelity 1000 per mille, bars held 111, met 108. Model `d04d89047a45`, corpus 6.0.0 (`5ba9574ad558`), committed in dd4bb552 | Gap: this does not describe the tip. See risk 2 |
| Moss Hop | `gp template check`, `gp baseline check --rom ... baselines/mosshop` | `gp template check templates/mosshop` passes today. 8 baselines | Baselines not run today |

The reference tool for these is `game-printer/target/release/gp`, built 09:54 today, after 6af9e8b3's commit at 09:50. `$GP_HOME/bin/gp` is older (5833fe9b, 33 files of `crates/` behind), so it must not produce baselines.

## 5. Gaps and risks the docs don't cover

1. Both Donkey Kong templates fail `gp template check` at the tip. `donkey-kong-oe`: its clips aren't corpus 6.0.0's (hash `a75809aab355`, recorded `5ba9574ad558`; 114 clips against 111, because 8fef94b2 added three `m50-ladder-fire` clips with no corpus bump). `rivet-rodeo`: `profiles.toml` is stale against what the derive gives, because the same commit changed Donkey Kong's profiles. `gp template extract` refuses for the same reason. The live page was built from this state, so page parity needs it as it is.
2. No score record matches the tip. Besides the three clips and the profiles change, two commits after the record touch the model or the simulator (edc2e7d1, c91dd6b7). Phase 3.3 needs a real baseline, made by one whole-corpus score of an exported copy at the import commit.
3. The page needs Bevy's crate now. `gp-web` and `gp-web-lift` use `gp_gen_bevy` for `present`, `looks`, `sound`, `bundle` and `spec_digest`. migration.md moves that code at 3.3, but 0.2 and 2.1 need it. The crate doesn't link Bevy itself, so I'd bring only those parts (about 3,600 lines) into `gp-present` at 0.2. That differs from migration.md's timing.
4. SMB and MM2 code is inside two crates phase 0.2 needs: `gp-assets` (6 files), `gp-interp` (`mm2.rs`, `smb.rs`, branches in `compile.rs` around lines 1964 to 2010 and 2325 to 2512), and `gp-gen-bevy/src/present.rs`. Cutting it out must leave the OE and Rivet Rodeo hashes unchanged, and the hashes above are the check.
5. No plan covers `gp-cli`. It links everything, so it can't be brought in as it is. 0.2 needs a new small `gp` that takes modules across one command at a time.
6. The two repos depend on each other until T3. Donkey Kong's hooks need `gp-sim` and `gp-cycles`, and the 2.1 page needs Donkey Kong's hooks. Before crates.io these are paths between sibling folders, and hosted CI must check out both. The tool repo must take the hook crate's path as a build input so it holds no game.
7. Both worlds would share `~/.game-printer`. New `gp` writing metrics, gate state or `$GP_HOME/bin/gp` (3.4) could replace the tool the live page's fixes depend on. The new world needs its own home until cutover.
8. A self-hosted runner holding ROMs must never run outside code. migration.md allows pull requests on the games repo, so the repo must be private or the runner must not run on pull requests.
9. `check-page.mjs` is tied to today's bundle layout and takes Playwright from `editor/node_modules`. Moving it means editing it. I'd keep the section names and picture hashes byte for byte and show the diff of the check.
10. Known page issues carry over: a restart that once took over 30 s and `paper-saves` retries (ROADMAP items 5). A section that retries is reported as a retry.
11. 3.2 asks for a rebuild on a second machine, and P2 for real phones. Neither is available to me. The Linux Docker image can stand in for the first if you agree.
12. No domain exists for staging or official. `packages.game-printer.org` in the docs is a placeholder.

## 6. Questions for you

1. Phase 0.1: do you adopt the research, whole or in part? ROADMAP.md says nothing starts until you do.
2. Git host and CI for the new repos. A: private GitHub repos with a self-hosted runner on this Mac for ROM jobs (recommended). B: no host, with local CI scripts, which means rewriting 0.2's and 0.3's checks.
3. The two template failures at the tip. A: import as they are so page parity holds, record them as known failures, and fix them as the first patches in `games` (recommended). B: pick an older commit, which won't match the live page.
4. `clips.toml` in 1.3. A: bring it unchanged, curves included, until 4.4 (recommended). B: move to the corpus plan at 1.3, against 3.3's "same clips".
5. Bringing `gp-gen-bevy`'s shared drawing code into `gp-present` at 0.2 instead of 3.3 (recommended, risk 3).
6. A separate home for the new world, such as `~/.gameprinter`, until cutover (recommended, risk 7).
7. One whole-corpus score of an exported copy of the import commit, to make the real baseline. It costs tens of minutes and writes nothing to `game-printer`. Now, or at 3.3 (recommended: at 3.3)?

## 7. Decisions, 2026-11-08

1. The research artifacts define the direction, so phase 0.1 is done.
2. Private GitHub repos under lex00, with a self-hosted runner on the user's Mac for ROM jobs.
3. Import the tip as it is; the two template failures are known failures and become the first patches in `games`.
4. `clips.toml` comes across unchanged until phase 4.4.
5. `gp-gen-bevy`'s shared drawing code comes in at 0.2. It is in a new crate, `gp-look`, not `gp-present`: it needs `gp-interp`, which depends on `gp-present`.
6. The new world's home is `~/.gameprinter` until cutover.
7. The whole-corpus Donkey Kong baseline is made at 3.3.
8. Donkey Kong stays one package with OE, JU and Rivet Rodeo. Moss Hop stays only as the tool's test fixture.
