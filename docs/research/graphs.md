# Graphs: what links Game Printer's data, and where to keep the links

Brought from game-printer's docs/research/ as written on 2026-11-12, with the prototype's paths changed for a package folder. The measurements are game-printer's, on Super Mario Bros. at `aa540854`; `gp dataset export`, `gp capture` runs and the loop it names are game-printer's commands, not gameprinter's.

Research for the user's question of 2026-10-30: "what kind of graphs are here. asset graph, graph of gameplay element relationships, graph of full extracted dataset. specifically i want to know if there is a case for using a graph database vs columnar or other storage types for this work ... a graph of the code starting from a spot related to its assets". Later the same day: "look into ladybug", "this should be something a data scientist would approve of", "be open to re-engineering this part now is the time for it", and "this research in turn should help drive the spec for game printer games". The last section is a draft package spec that follows from the recommendation.

Everything here was measured on 2026-10-30 on an Apple M5 Pro (18 cores, 64 GB, macOS 26.7.1) at commit `aa540854`, in a scratch venv (Python 3.14.6, DuckDB 2.0.0, Kuzu 0.11.3, LadybugDB 0.22.1, pyarrow 26.0.0). Nothing was installed globally except one thing by mistake: DuckDB's `INSTALL duckpgq` wrote the extension to `~/.duckdb/extensions/` before the scripts set `extension_directory` to the scratch folder. The real store and run were read only.

## Summary

| Question | Answer |
|---|---|
| Are there graphs in the work | Yes, six, and every one is already in the files: the asset map, the model, the mapping, the evidence refs, the clips' `holds`, the coverage maps, the score records and the commit log each hold one kind of edge. Nothing joins them yet except by hand |
| How big | Super Mario Bros. today: 7,834 nodes and 50,305 edges, 239 KB as two zstd Parquet files. 59% of the edges are clip-runs-routine edges from code coverage |
| Which questions get easier | Anything that walks more than two kinds of link: the code behind a fact, the clips a routine change affects, the mechanics that share a RAM byte, the walk from an enemy's frames to its code, RAM, fields and clips, and routines no fact covers. Today each is a one-off script or isn't asked |
| A graph database | Not now. On this graph DuckDB answered all six test questions in 0.5 to 4.4 ms warm, faster than LadybugDB on five of six. Graph engines win only on fixed-depth path matching at scale, which this data doesn't have. Kuzu, the obvious choice a year ago, was archived on 2025-10-10 and returned wrong answers on three of six questions here |
| What to do instead | Yes, as a derived edge list: `nodes.parquet` and `edges.parquet` written by `gp dataset export` beside today's tables, with a manifest of input hashes, queried with DuckDB's recursive SQL, walked in Rust with petgraph for the page and the clip picker. LadybugDB can read the same files when someone wants Cypher; nothing depends on it |
| Re-engineering | Considered: a graph-first store for provenance, code and evidence, with Parquet only for per-frame tables. Rejected for now on maintenance risk (a one-year-old fork of an archived engine), determinism (no byte-stable file) and fit (the queries are joins and closures that SQL handles). TOML stays the source of truth. The change worth making now is a typed node and edge schema, versioned like the other formats |
| Prototype | `docs/research/notes/graphs/build_graph.py` builds the graph in 0.3 s from the template, the `gp dataset export --hf` tables, the pinned disassembly and the coverage maps; two runs give byte-identical Parquet. `bench.py` runs the same six questions on DuckDB SQL, DuckPGQ, Kuzu and LadybugDB, first run and warm, at 1x and 100x |

## 1. The graphs in the work

All six share node kinds, so they are one graph seen from six places. The counts are from the prototype's SMB1 build (section 4), with the routine heuristic of `gp dataset export`: a routine is a label that a `jsr`, a `jmp` or a `.dw` table names.

### The asset graph

| Nodes | Count | Edges | Count | From |
|---|---|---|---|---|
| ROM data label (`EnemyGraphicsTable`) | 509 data labels in the code | asset `stored_at` label | 217 | `assets/smb1-ntsc-prg0.toml` (labels, column labels, entry labels) |
| Asset (`enemy_frames`, `palette_ground`, `area_objects`) | 97 | model slot `lifted_from` asset | 29 | `model.toml` `[assets.*].from` |
| Model slot (`sprites`, `sprite_tiles`, `metatiles_1`) | 31 | slot `built_from` asset | 2 | `sprites.toml` tables |
| Sprite frame (`sprites[43]`) | 64 | entity `drawn_with` frame, frame `in_slot` slot | 80, 64 | `entities.*.sprite` |
| Entity | 42 in the model | entity `uses_tiles` slot | 38 | `entities.*.sprite.slot` |
| Level | 37 in the model | level `spawns` entity | 78 | `levels.*.spawn` |

Sketch: `level:w1_1` -> `entity:goomba` -> `frame:sprites[43]`, `frame:sprites[44]` -> `slot:sprites` -> `asset:enemy_frames` -> `data:EnemyGraphicsTable` <- `routine:DrawEnemyObjRow` (reads it).

One link is missing in the files: which row of `enemy_frames` became which frame of the captured sprite set. `gp sprites capture` knows it and doesn't write it, so the walk from the goomba to its frames' bytes goes through the whole table.

### The gameplay graph

| Nodes | Count | Edges | Count | From |
|---|---|---|---|---|
| Mechanic part (`platformer.walk`, `smb1.platform`) | 40 used | entity `carries` mechanic | 124 | `mechanics.parquet` (`gp_interp::mechanics::catalog`) |
| Fact (a model path, `entities.goomba.contact.every`) | 5,469 (5,413 in `facts.parquet`, plus 56 paths the clips hold that aren't leaves there) | fact `part_of` mechanic | 2,868 | `facts.parquet` |
| | | fact `about` entity, level or slot | 5,261 | the path |
| Mapping field (`entities.mario[0].vx`, `slots[0].kind.goomba:facing`) | 161 | field `measures_for` mechanic, field `of_entity` entity | 27, 13 | `mapping.parquet` (`owner`) |
| Object ID (`0x06`) | 38 | entity `has_object_id`, ID `named` constant (`Goomba`) | 39, 21 | `[[slots.kind]]`, the disassembly's equates |
| Event, touch | not nodes yet | | | The model's `contact` tables and the event names in grid cells (`player.died`, `enemy.hurt_player`) |

Sketch: `entity:goomba` -> carries `mechanic:platformer.walk`, `mechanic:platformer.contact`, `mechanic:platformer.fall` and three more; `fact:entities.goomba.contact.every` -> part of `platformer.contact`. Who touches whom (stomp, hurt) lives inside each entity's `contact` table as options, so it is a property today, not an edge. Making touches edges (`goomba -hurts-> mario`, `mario -stomps-> goomba`) is a small addition to the builder.

### The code graph

| Nodes | Count | Edges | Count | From |
|---|---|---|---|---|
| Routine (`MoveNormalEnemy`, `EnemyGfxHandler`) | 577 | `calls` (jsr), `jumps_to` (jmp to another routine), `dispatches` (a `.dw` table after `jsr JumpEngine`) | 546, 217, 249 | SMBDIS.ASM at the pinned revision, addresses from ld65's debug file (`just smb-listing`) |
| RAM label (`Enemy_X_Position`, `Player_Y_Speed`) | 415 | routine `reads`, `writes` RAM | 1,670, 1,423 | The same, by addressing mode |
| Data label | 509 | routine `reads_data` | 514 | The same |
| Constant (`Goomba`) | 26 | routine `uses_const` | 112 | Immediate operands |
| | | RAM `mapped_to` field | 502 | `mapping.parquet` `ram` |
| | | fact `cites` routine | 4,001 | `facts.parquet` `routine` (the evidence ref's line) |
| Clip | 170 | clip `runs` routine | 29,728 | The original's code map over each clip (`gp capture dense`'s `coverage.json`, 169 clips of `smb-side-by-side-2`) |

Sketch: `routine:GameCoreRoutine` -calls-> `routine:EnemiesAndLoopsCore` -> ... -> `routine:MoveNormalEnemy` -writes-> `ram:Enemy_MovingDir` -mapped_to-> `field:slots[0].kind.goomba:facing`; `fact:entities.goomba.contact.every` -cites-> `routine:PlayerEnemyCollision`; `clip:gen.w1_1.start.t1.nothing` -runs-> `routine:MoveNormalEnemy`.

The edges are as good as two heuristics. A routine is the nearest jsr, jmp or table target above a line, so a label code only falls into (SMBDIS.ASM's `GameEngine`, under `GameCoreRoutine`) is not a routine, and the first prototype query that started there found nothing. Indexed and indirect operands are read by their base label, so `lda Enemy_X_Position,x` is a read of `Enemy_X_Position` whatever slot `x` holds. Coverage would fix the first (the routine that ran an address) and a trace would fix the second; both exist (`gp_nes::coverage`, `gp trace`).

### The evidence and provenance graph

| Nodes | Count today | Edges | From |
|---|---|---|---|
| Evidence entry, ref | 168 entries, 686 refs (564 into SMBDIS.ASM) | fact `covered_by` entry (most specific prefix), entry `refs` file:line | `[evidence."..."]` in `model.toml` |
| Score | 87 | score `of_clip`, `under_model` (`83dc2714...`), `under_corpus` (2.0.0) | `evidence/<model>/<corpus>.json` |
| Clip recipe | 87 kept, 169 in the run | clip `holds` mechanic, `holds_path` fact, `guards` mutant, `in_level` | `clips.toml` |
| Mutant | 29 | | `guards` |
| Attempt, grid cell | 57 attempt commits in `git log`, 10 matched by the builder's pattern | attempt `on_cell` cell, cell `in_level`, cell `about` entity | Commit subjects (`Attempt 152 on grid:entity:lift_down:v0@w2_4.start: ...`) |
| Commit, model version, corpus version, user tell | Not built | attempt `is` commit, commit `changes` fact (from `gp model diff`), tell `on` clip | git, `gp model diff`, the tells' records |

Sketch: `attempt:152` -> `cell:grid:entity:lift_down:v0@w2_4.start` -> `level:w2_4`, `entity:lift_down`; `score:83dc2714.../gen.w1_1.start.t1.nothing` -> `clip:...`, `model:83dc2714...`, `corpus:2.0.0`.

This graph grows with every round, and it is where a graph earns its keep: "which commits changed facts that the failing clips hold" is three joins over three sources that no table holds together.

### The cross-game graph

`gp dataset catalogue` already writes it as a table (hf-datasets.md, section 3): 46 mechanic parts over five models, six in all five games. As a graph it is `game -uses-> mechanic part`, `fact -part_of-> part`, `part -option-> value`. "Which games have a jump with 3 gravity bands" is a filter on `facts` where `mechanic = 'platformer.jump'` and the path ends in `bands`, then a count; no traversal is needed. The cross-game graph is bipartite and two hops deep, so it gains nothing from a graph engine.

### From an enemy's assets to its code and clips

The walk the user asked for, run on the prototype (question q4 in section 4), starting at the goomba:

| Step | Edge | Reached |
|---|---|---|
| 1 | `entity:goomba` -drawn_with-> | `frame:sprites[43]`, `frame:sprites[44]` (and `uses_tiles` `slot:sprite_tiles`) |
| 2 | frame -in_slot-> slot -built_from-> asset -stored_at-> data label <-reads_data- routine | `asset:enemy_frames`, `data:EnemyGraphicsTable`, `routine:DrawEnemyObjRow` (inside EnemyGfxHandler) |
| 3 | `entity:goomba` -has_object_id-> `objid:0x06` -named-> `const:Goomba` <-uses_const- routine | 9 routines that name a goomba: `EnemyGfxHandler`, `EnemyToBGCollisionDet`, `HandleGroupEnemies`, `MoveDefeatedEnemy`, `PlayerEnemyCollision`, `SetStun`, `ShellOrBlockDefeat`, `FlipBowserOver`, `ProcLoopCommand` |
| 4 | routine -writes-> RAM -mapped_to-> field | `SetStun` writes `Enemy_State`, mapped to `slots[0].kind.goomba:state`; `HandleGroupEnemies` writes `Enemy_X_Position`, mapped to `slots[0].kind.goomba:x` (and to every other kind's state and x, since the slots are shared) |
| 5 | routine <-runs- clip | 131 of the 169 clips run `EnemyGfxHandler` |
| 6 | fact -cites-> routine | `entities.goomba.contact.every` cites `PlayerEnemyCollision`, `despawn_behind` cites `OffscreenBoundsCheck` |

Steps 1, 3, 4 and 5 are question q4 and reach 297 nodes; steps 2 and 6 were run as separate lookups. The walk is wide, not deep: every step is one hop, and the width comes from shared routines (EnemyGfxHandler draws every enemy). A useful view for the page has to rank, not list: routines that name `Goomba` before routines that every enemy runs.

## 2. The questions

| Question | Walk | Answered today | Prototype |
|---|---|---|---|
| Which clips would a change to this routine affect | routine <-runs- clip | No. Every model change rescores the whole set (the score cache keys on model hash) | q1: one join, 77 clips run MoveNormalEnemy |
| What code backs this fact | fact -cites-> routine -calls/jumps/dispatches*-> | Partly: `facts.parquet` has `routine`, `gp model check` checks the ref's line. The callees aren't followed | q2: 38 routines behind the goomba's facts |
| Smallest clip set touching all routines that write X | RAM <-writes- routine <-runs- clip, then a set cover | The cover exists in Rust (`gp capture prune`, `gp capture dense --pick`) over model parts and code addresses, not over a RAM byte's writers | 21 routines write `Enemy_X_Position`; 11 run in some clip; 6 clips cover those 11 (greedy, 27 ms in Python). 10 writers run in no clip: `MoveBloober`, `MoveSwimmingCheepCheep`, `ProcBowserFlame` and seven more |
| Which mechanics share a RAM byte | mechanic <-part_of- fact -cites-> routine -writes-> RAM | No | q3: 76 RAM labels are written by routines cited for two or more mechanics; `Enemy_State` by 16 |
| What does this tell's clip touch | tell -on-> clip -runs-> routine, -holds-> part | No: tells name a clip and a frame; nothing follows them | Not built: tells aren't in the builder yet |
| Which assets does this enemy use and which routines draw them | q4 above | No | q4: 297 nodes |
| Which routines run in play but no fact covers (missing mechanics) | routines run by a clip, minus those cited or reached from a cited routine | Stage 5 of `gp progress` counts facts that cite code, not code that no fact cites | q5: 143 routines. Most are the area parser and block objects (`AreaParserCore`, `BrickWithCoins`), which the asset decoders model rather than facts, so the detector needs `asset -decoded_by-> routine` edges before it is useful |
| Cross-game: which games have a jump with 3 gravity bands | `facts` filter | Yes, `catalogue/data/facts.parquet` with DuckDB (hf-datasets.md) | Not a graph question |
| Every routine the engine reaches | routine -code*-> | No | q6: 390 routines from GameCoreRoutine, at most 10 calls deep |

The questions the work asks are reachability and joins, not pattern matching. The deepest closure is 10 hops; most questions are 1 to 4.

## 3. Storage options

Facts verified on 2026-10-30; sources at the end. "Embedded" means a library in our process with a file on disk.

| Option | Kind | Licence | Status | Rust | Python | Query language | Parquet | Determinism and versioning |
|---|---|---|---|---|---|---|---|---|
| DuckDB 2.0.0 (with recursive CTEs) | Embedded | MIT (the Rust crate; the core's licence file wasn't opened) | Active: 2.0.0 released 21 Oct 2026 | `duckdb` 1.20000.1 (2026-10-28) | `duckdb` | SQL, `WITH RECURSIVE`, `USING KEY` since 1.3 | Reads and writes natively | Results unordered without `ORDER BY`; threaded float aggregates can differ slightly. A logical checksum recipe exists; byte-identical database files weren't verified |
| DuckPGQ (DuckDB community extension) | Embedded | MIT | "A research project and a work in progress"; builds listed for DuckDB 1.4.4 to 2.0.0. On 2.0 it needs `SET active_grammar_extensions = ['duckpgq']` | Through `duckdb` | Through `duckdb` | SQL/PGQ (`GRAPH_TABLE`, `MATCH`) | DuckDB's | Shortest paths are ANY SHORTEST, which its docs call non-deterministic. No `OPTIONAL MATCH` |
| Kuzu 0.11.3 | Embedded | MIT | Archived on 2025-10-10; Apple agreed to buy the company on 2025-10-09 | `kuzu` 0.11.3, last | `kuzu` | Cypher | `COPY FROM` Parquet | Wrong results here on filtered recursive patterns (section 4); no fixes coming |
| LadybugDB 0.22.1 | Embedded | MIT | The Kuzu fork, active: ten releases from 29 Aug to 28 Oct 2026. Maintainers named only as "LadybugDB Developers"; no company or governance stated | `lbug` 0.22.1 (2026-10-28), optional `arrow` feature | `ladybug` (an older `real_ladybug` 0.15.3 from `lbugdb/lbug` also exists) | Cypher | `COPY FROM` Parquet, `COPY TO` Parquet | Byte-stable file not verified. UNION results came back in a different order run to run |
| SQLite (recursive CTEs) | Embedded | Public domain | Active: 3.54.1, 2026-10-16. Already in the workspace (`gp-index`, `gp-metrics`, rusqlite 0.41) | `rusqlite` | `sqlite3` | SQL, `WITH RECURSIVE` (UNION to stop cycles; no aggregates in the recursive step) | None native | Same as any SQL: order by hand |
| CozoDB 0.7.6 | Embedded or server | MPL-2.0 | Last crate 2023-12-11, last push 2024-12-04 | `cozo` | `pycozo` | Datalog | Not mentioned | Not checked: inactive |
| Neo4j 2026.10 Community | Server | GPLv3 | Active | `neo4rs` 0.9.0-rc.11, a Neo4j Labs project "not part of Neo4j's supported product lineup" | not checked | Cypher | Through import tools | A server to run, which AGENTS.md's "everything is local" allows but nothing else here needs |
| Memgraph 3.14 | Server | BSL (no embedding in or distribution to third parties), Apache-2.0 after the change date | Active | `rsmgclient` 4.0.0 | not checked | Cypher | Not checked | The BSL rules it out for anything that ships with `gp` |
| Oxigraph 0.5.13 | Embedded (RocksDB) or server | MIT or Apache-2.0 | Active, "in heavy development"; "SPARQL query evaluation has not been optimized yet" | `oxigraph` | `pyoxigraph` | SPARQL 1.1 | Not mentioned | Triples suit publishing (JSON-LD, Croissant is JSON-LD) more than analysis |
| petgraph 0.8.3 | In-memory library | MIT or Apache-2.0 | Active | `petgraph` | none | Rust API: Dijkstra, SCCs, toposort, dominators, `has_path_connecting`, PageRank | none | Deterministic if built in sorted order; nothing persists |
| Apache AGE 1.8 | Postgres extension | Apache-2.0 | Active | Postgres drivers | Postgres drivers | openCypher in SQL | Postgres's | A Postgres server |

The DuckPGQ paper (ten Wolde, Singh, Szarnyas and Boncz, CIDR 2023) found that path queries written as recursive SQL were "slow and brittle" in its tests on LDBC SNB graphs of scale factor 10 to 3000, and that DuckPGQ's vectorized path-finding beat Neo4j 4.4 there. Those graphs have billions of edges. DuckDB's USING KEY post (Bamberg and Grust, 2025-05-23) shows plain recursion growing to 605,859,791 rows on a 1,446-edge graph for all-pairs paths, against 19,213 with USING KEY. Neither result is about a 50,000-edge graph asked reachability questions, which is what we have; section 4 measures that case.

## 4. The prototype

### What it builds

`docs/research/notes/graphs/build_graph.py` reads, and only reads:

| Input | Gives |
|---|---|
| `templates/smb1-ntsc-prg0/model.toml`, `mapping.toml`, `sprites.toml`, `assets/smb1-ntsc-prg0.toml` in game-printer. In gameprinter, `--game` names a package folder in the games checkout (games/donkey-kong) and the script reads its `model.toml`, `mapping.toml`, `sprites.toml` when there is one, and `assets/SOURCE.toml` (`--source`, default the model's `source`) | Assets, slots, frames, entities, levels, object IDs |
| `gp dataset export --hf` output (`facts`, `mechanics`, `mapping`, `clips`, `scores` Parquet), made from the template with the release `gp` | Facts with their routine, mechanic uses, fields with RAM and owner, clip recipes with `holds` and `guards`, scores |
| `$GP_HOME/work/smbdis/download/SMBDIS.ASM` and `listing/smbdis.dbg` (pinned, never committed) | Routines, calls, jumps, tables, RAM reads and writes, data reads, constants |
| A scratch run's `generated/dense/coverage.json` | Each of 169 clips' code addresses on the original, mapped to routines |
| `git log` | Attempts and their grid cells |

It writes `nodes.parquet` (`id`, `kind`, `name`, `attrs` JSON), `edges.parquet` (`src`, `rel`, `dst`, `via`: the file or file:line the edge came from) and `manifest.json` (the input hashes, the repo commit, the counts and the output hashes). Every id is `<kind>:<name>`, rows are sorted, and zstd Parquet from pyarrow is byte-stable: two builds gave the same SHA-256 for both files. The build takes 0.34 s.

| Table | Rows | Size |
|---|---|---|
| nodes | 7,834 | 109 KB |
| edges | 50,305 | 124 KB |
| The five export tables it starts from | 6,062 | 532 KB |

What it doesn't yet have: tells, commits beyond attempts, model and corpus versions over time, touches as edges, events, and the frame-to-table-row link. Coverage is from a scratch run made earlier on 2026-10-30, not from the corpus at 2.0.0, so its 169 clips are the run's, not the kept 87; coverage depends only on the ROM and the presses, so it is right for those clips whatever the model.

### How it was benchmarked

`bench.py` loads the same two Parquet files into each engine's own database file, then runs each question in a fresh process: the first run after opening the file, and ten more (warm) whose results must equal the first as a set. The OS page cache was warm for every run (the files are small and were just written), so "first" means a cold engine, not a cold disk; clearing the page cache needs `sudo purge`, which wasn't run. DuckDB ran with its default thread count. Each engine's answer was compared with DuckDB's. DuckPGQ ran the three questions it expresses directly.

The scaled run is the same graph copied 100 times with each copy's ids suffixed (5,030,500 edges), so a question that starts from one node still touches one copy and a question over all facts touches all 100. It tests size, not shape: real games won't be 100 disjoint copies.

### Results

Milliseconds. Load is from the two Parquet files into a new database file.

| | DuckDB SQL | DuckPGQ | Kuzu 0.11.3 | LadybugDB 0.22.1 |
|---|---|---|---|---|
| Load | 115 | 2,365 (includes the extension download) | 456 | 595 |
| Database file | 1.5 MB | 3.0 MB | 6.8 MB | 4.6 MB |

| Question (rows) | DuckDB first / warm | DuckPGQ first / warm | Kuzu first / warm | LadybugDB first / warm |
|---|---|---|---|---|
| q1 impact of a routine change (77) | 1.9 / 0.49 | 18.3 / 1.55 | 8.2 / 0.73 | 6.4 / 0.40 |
| q2 code behind the goomba's facts (38) | 3.3 / 1.54 | not expressed | 15.1 / 3.49, wrong (39) | 15.4 / 4.54 |
| q3 mechanics sharing a RAM byte (76) | 5.1 / 3.06 | 23.2 / 8.91 | 17.9 / 7.06 | 20.0 / 10.67 |
| q4 the goomba's walk, steps 1, 3 to 5 (297) | 9.5 / 4.34 | not expressed | 27.5 / 16.17 | 35.1 / 26.89 |
| q5 routines no fact covers (143) | 5.9 / 2.65 | not expressed | 28.2 / 9.83, wrong (146) | 27.1 / 11.67 |
| q6 reach from GameCoreRoutine (390) | 3.5 / 1.53 | 23.6 / 11.56 | 12.4 / 2.33, wrong (393) | 12.7 / 2.52 |

At 100 times the size, DuckDB and LadybugDB only (Kuzu is archived; DuckPGQ was the slowest at 1x):

| Question | DuckDB first / warm | LadybugDB first / warm |
|---|---|---|
| Load (5.0 M edges) | 291 | 2,855 |
| Database file | 56 MB | 196 MB |
| q1 | 6.3 / 3.92 | 10.2 / 0.65 |
| q2 (3,800 rows) | 19.8 / 14.50 | 116.4 / 83.31 |
| q3 (7,600) | 45.4 / 38.46 | 429.3 / 350.84 |
| q4 | 28.4 / 15.78 | 42.1 / 30.88 |
| q5 (14,300) | 65.2 / 30.71 | 5,198 / 3,328 |
| q6 | 15.6 / 9.58 | 16.3 / 2.23 |

What the numbers say:

| Finding | Detail |
|---|---|
| At today's size every engine is fast enough | The slowest warm answer is 27 ms. Nothing here is a reason to choose an engine on speed |
| Point lookups and single-source walks favour the graph engine | LadybugDB's warm q1 and q6 stay near 0.5 and 2 ms at 100x, where DuckDB scans (4 and 10 ms). Its adjacency lists pay off when a query starts at one node |
| Set-at-a-time questions favour DuckDB | q2, q3 and q5 start from thousands of nodes. DuckDB's hash joins grow 5 to 12 times from 1x to 100x; LadybugDB's grow 18 to 285 times. q5 took 3.3 s warm on LadybugDB against 31 ms |
| Kuzu 0.11.3 is wrong on filtered recursion | `(a)-[:E* 1..12 (e, n | WHERE e.rel IN [...])]->(b)` reached 97 RAM and data nodes, which only the excluded edges (`reads`, `writes`, `reads_data`) lead to; with `SHORTEST` it reached 11 such nodes and missed 8 routines. LadybugDB 0.22.1 gave DuckDB's answer on every question. Kuzu won't be fixed |
| Order isn't stable in Cypher's UNION | Kuzu and LadybugDB returned q2 and q4 in a different order on different runs. Every query that feeds a hash or a file needs an explicit order; Cypher's UNION has no ORDER BY over the whole, so the caller sorts |
| DuckPGQ adds no capability here | Its quantified path `->{1,30}` on q6 was the slowest of the four, and its shortest-path semantics are non-deterministic by its own docs. Recursive SQL already does reachability |
| Kuzu's variable-length bound is 30 | `Upper bound of rel exceeds maximum: 30` on `*0..40`. Enough for SMB1's 10-deep closure; a configuration setting raises it |

The limits of the test: one game, one machine, a warm page cache, Python bindings (the Rust ones weren't tried), one query formulation per engine written by one person, and a synthetic scale-up. A Cypher expert could probably speed up q3 and q5 on LadybugDB. None of that changes the verdict, which rests on fit and risk, not speed.

### The same questions side by side

Each query runs on the same `nodes` and `edges` tables (`node(id, kind, name, attrs)`, `edges(src, rel, dst, via)`); the full set is in `bench.py`.

q1, the clips a change to MoveNormalEnemy affects:

```sql
-- DuckDB
SELECT src FROM edges
WHERE rel = 'runs' AND dst = 'routine:MoveNormalEnemy' ORDER BY src;

-- DuckPGQ (runs is its own edge table)
FROM GRAPH_TABLE (gp
  MATCH (c:node)-[e:runs]->(r:node WHERE r.id = 'routine:MoveNormalEnemy')
  COLUMNS (c.id AS src)) ORDER BY src;
```

```cypher
// LadybugDB
MATCH (c:Node)-[e:E {rel: 'runs'}]->(r:Node {id: 'routine:MoveNormalEnemy'})
RETURN c.id AS src ORDER BY src;
```

q3, the mechanics that share a RAM byte through the code their facts cite:

```sql
-- DuckDB
SELECT w.dst AS ram, count(DISTINCT p.dst) AS mechanics
FROM edges p
JOIN edges c ON c.src = p.src AND c.rel = 'cites'
JOIN edges w ON w.src = c.dst AND w.rel = 'writes'
WHERE p.rel = 'part_of'
GROUP BY w.dst HAVING count(DISTINCT p.dst) > 1 ORDER BY ram;

-- DuckPGQ
FROM GRAPH_TABLE (gp
  MATCH (m:node)<-[p:part_of]-(f:node)-[c:cites]->(r:node)-[w:writes]->(b:node)
  COLUMNS (b.id AS ram, m.id AS mech))
SELECT ram, count(DISTINCT mech) AS mechanics
GROUP BY ram HAVING count(DISTINCT mech) > 1 ORDER BY ram;
```

```cypher
// LadybugDB
MATCH (m:Node)<-[:E {rel: 'part_of'}]-(f:Node)-[:E {rel: 'cites'}]->(r:Node)-[:E {rel: 'writes'}]->(b:Node)
WITH b.id AS ram, count(DISTINCT m.id) AS mechanics WHERE mechanics > 1
RETURN ram, mechanics ORDER BY ram;
```

q6, every routine reachable from GameCoreRoutine:

```sql
-- DuckDB: UNION (not UNION ALL) stops at cycles
WITH RECURSIVE r(id) AS (
  SELECT 'routine:GameCoreRoutine'
  UNION
  SELECT e.dst FROM edges e JOIN r ON e.src = r.id
  WHERE e.rel IN ('calls', 'jumps_to', 'dispatches'))
SELECT id FROM r WHERE id <> 'routine:GameCoreRoutine' ORDER BY id;

-- DuckPGQ (code is an edge table of the three kinds)
FROM GRAPH_TABLE (gp
  MATCH (a:node WHERE a.id = 'routine:GameCoreRoutine')-[e:code]->{1,30}(b:node)
  COLUMNS (b.id AS id))
SELECT DISTINCT id WHERE id <> 'routine:GameCoreRoutine' ORDER BY id;
```

```cypher
// LadybugDB
MATCH (a:Node {id: 'routine:GameCoreRoutine'})-[:E* SHORTEST 1..30 (e, n | WHERE e.rel IN ['calls','jumps_to','dispatches'])]->(b:Node)
WHERE b.id <> 'routine:GameCoreRoutine'
RETURN DISTINCT b.id AS id ORDER BY id;
```

The Cypher is shorter for fixed paths (q3) and about the same for closures. The SQL has one advantage that matters here: it runs on the Parquet files where they are, with no load step and no second engine, and the same text runs in DuckDB-Wasm in the page.

## 5. Is there a case?

### Against a graph database

| Criterion | Graph-first store (LadybugDB) | Derived edge list in Parquet, DuckDB and petgraph |
|---|---|---|
| Fit | Cypher reads well for fixed paths. But the questions are closures and joins over sets, where it was slower | SQL handles every question asked; recursive CTEs cover the closures. petgraph gives the algorithms SQL lacks (dominators, SCCs, set cover inputs) in the Rust that already holds the model |
| Maintenance risk | A one-year-old fork of an engine its company abandoned when Apple bought it; anonymous maintainers; ten releases in two months, so the file format moves | DuckDB is already the recommended reader (datasets.md, hf-datasets.md); Parquet is the format `gp dataset export` writes with the `parquet` crate already in `gp-cli`. No new dependency for the build; DuckDB stays a reader |
| Determinism | The database file isn't shown to be byte-stable; UNION order varies; the derived graph would need its own hash | Sorted Parquet from a fixed writer is byte-stable (measured); the manifest hashes inputs and outputs, as score records already do |
| Size | 4.6 MB for a 239 KB graph (19 times); 196 MB at 100x | 239 KB; 56 MB in DuckDB's own file at 100x when loaded at all |
| What a data scientist expects | A graph store is a fine analysis tool, but not a distribution format: nobody `load_dataset`s a LadybugDB file | Parquet with a card and Croissant loads in pandas, polars, DuckDB and `datasets` in one line (data-publishing.md goal 5); an edge list imports into any of the graph engines in section 3 (LadybugDB's `COPY FROM`, DuckPGQ over the tables, petgraph from rows) |

A graph database would help a person exploring in Cypher, and LadybugDB can read the Parquet edge list with `COPY FROM` when someone wants that. It doesn't help the toolchain, so it shouldn't be in it.

### The recommendation: "yes, as a derived view"

| Layer | What | Format | Engine for queries |
|---|---|---|---|
| Source of truth | The model, mapping, contract, profiles, asset map, clip recipes, corpus, loop, sources | TOML, in git, canonical (`gp model fmt`) | `gp` parses it; nothing else writes it |
| Pinned inputs, never committed | The disassembly, the ROM, recordings, coverage maps, traces | `$GP_HOME/work/`, `$GP_STORE` | Rebuilt by scripts from hashes |
| Derived tables | facts, mechanics, mapping, clips, scores, measures, events, splits, windows | Parquet, sorted, uncompressed or zstd with fixed settings, one row group | DuckDB (Python, CLI, Wasm in the page), polars |
| Derived graph | `nodes`, `edges` with a typed schema (below) and a manifest | Parquet beside the tables | DuckDB recursive SQL for analysis; petgraph in Rust for `gp` commands and the page's API |
| Records | Score records, signed statements | JSON (in-toto), in `evidence/` | Read by `gp`; their hashes appear as `score` nodes |

The re-engineering this asks for is small and worth doing now, while the export is young:

| Change | Where |
|---|---|
| A graph export: `gp dataset export --graph` writes `nodes.parquet`, `edges.parquet` and the manifest, from the same Rust that writes facts (`crates/gp-cli/src/serve/dataset_hf.rs`), with the code edges read from the pinned download when it is there and left out (and said so) when it isn't, as the `routine` column is today | `gp-cli` |
| Typed ids and a fixed edge vocabulary, versioned as `graph` 0.1.0 with a JSON Schema in `schema/` | `schema/graph-0.1.0.json` |
| Every table's Parquet metadata names the model hash, corpus version, mapping schema, gp version and input hashes (most of it is there already: `gp.model`, `gp.version`) | `dataset.rs` |
| Score records gain `inputs`: the hashes of the derived graph and tables a score was made with, so a record says what graph it was checked against. No other change | `docs/formats/scoring-corpus.md` |
| The page's API gains `GET /api/graph/{template}/walk?from=entity:goomba&depth=N&rels=...` (petgraph over the edges, ranked by how specific a node is) and `GET /api/graph/{template}/affected?routine=...` | `gp serve` |
| Incremental rescoring keys on the edges: a change to facts that cite routine R rescores the clips that run R first, then the rest in the background | `gp capture` queue |

### The typed schema (draft, `graph` 0.1.0)

Nodes:

| Kind | Key | Columns beyond `id`, `kind`, `name` | Source |
|---|---|---|---|
| `routine` | label | `addr` (CPU), `line` (in the pinned file), `bank` | disassembly or source, with its pin |
| `ram` | label | `addr`, `size` | labels file |
| `data` | label | `addr`, `prg` offset | disassembly |
| `const` | name | `value` | disassembly equates |
| `asset` | asset name | `format`, `region` | asset map |
| `slot` | slot name | `kind` | model `[assets]` |
| `frame` | slot and row | | model sprites |
| `entity`, `level` | model name | `role` | model |
| `mechanic` | part name | `version`, `carrier`, `class` | mechanic records |
| `fact` | model path | `class`, `evidence` kind, `confidence` | `facts` |
| `field` | scope and path | `role`, `owner` | mapping |
| `objid` | value | | mapping slot kinds |
| `clip` | clip id | `kept`, `frames`, `recipe` hash | `clips.toml`, runs |
| `mutant` | name | | `clips.toml` guards |
| `score` | model hash, corpus hash, clip | `exact`, `permille` | `evidence/` |
| `attempt`, `commit`, `tell`, `cell` | number, sha, id, cell id | `date` | git, tells, grid |

Edges: `src`, `rel`, `dst`, `via` (file or file:line), plus `confidence` where the edge is a heuristic (`routine` by nearest label, RAM by base label). The vocabulary is the one in section 1; a reader rejects a `rel` it doesn't know. Provenance is per file, not per row: the manifest's `model`, `corpus`, `mapping_schema`, `gp`, `inputs` (name -> hash; the prototype uses BLAKE2b, the export would use BLAKE3 as the store does) and `outputs`.

## 6. How it could help analysis and the user

| Use | What it needs | Value |
|---|---|---|
| Click an enemy in the page and see its code, RAM, facts and clips | The walk endpoint; a ranked list, the specific before the shared (q4 is 297 nodes) | High for the user's "pause, point, tell" flow: a tell on a goomba can show which clips run its routines and which facts cite them, without RAM or routine names in the main view (they stay in the details, as the inspector's rule says) |
| Incremental rescoring | `runs` edges per kept clip, cached by recipe hash | High: a change to one hook rescores the 77 clips that run its routine first instead of all of them |
| Graph-driven clip picks | `runs` and `writes` edges; the set cover `gp capture prune` already has | Medium: "cover every writer of Enemy_X_Position" found 10 writers no clip runs, a list of clips to make |
| Missing mechanics | Routines run in play with no `cites` edge and none reached from one, minus routines an asset decoder covers | Medium after the asset-decoder edges exist; today 143 hits, mostly the area parser |
| Shared RAM as coupling | q3 | Medium: a byte written under 16 mechanics (`Enemy_State`) is where one fix breaks another; the agents could be warned before an attempt |
| Publishing | The two Parquet files with the dataset; code edges only as names (the same line hf-datasets.md draws for `routine`) | Medium: a RAM map with its writers and readers is what Data Crystal's pages hold by hand |

## 7. Draft: Game Printer package spec v0

This follows from section 5 and builds on datasets.md section 5 (the package layout), docs/formats/scoring-corpus.md, clips.md, loop.md and `schema/`. Open choices are marked "Open".

### Tiers and homes

| Tier | Ships | Home |
|---|---|---|
| 1. In the playable game | What the Bevy or web build needs to run: the generated code, the model's runtime data, assets lifted from the player's ROM at print time | The printed project (`print/`), never committed; built on the player's machine |
| 2. With the package | Recipes, records and derived tables that prove and describe it, with nothing from the ROM | Git, and a Hugging Face dataset repo per game when the user decides to publish |
| 3. Build time only | Everything made from the ROM or from runs | `$GP_HOME/work/`, `$GP_STORE`, run folders; regenerated, never published |

### The layout

| Path | Required | Tier | Format | Holds |
|---|---|---|---|---|
| `gp-package.toml` (today `template.toml`) | Yes | 2 | TOML | Manifest: format, id, version, title, licence, source keys |
| `sources.toml` | Yes for a ROM | 2 | TOML | ROM and patch by hash, URL, permission note. Never bytes |
| `model.toml` | Yes | 1 and 2 | TOML, canonical | The model; its hash names every score |
| `mapping.toml` | Yes for a ROM | 2 | TOML | RAM to model paths; build time for scoring, shipped as evidence |
| `fidelity.toml` | Yes for a ROM | 2 | TOML | The contract |
| `profiles.toml` | Optional | 1 | TOML | Physics profiles the game offers |
| `observe.toml` | Optional | 2 | TOML | What validation reads beyond the mapping |
| `sprites.toml` | Optional | 3 (a recipe for `gp sprites capture`), committed | TOML | Sprite table shapes |
| `clips.toml`, `clips/*.presses` | Required to publish evidence | 2 | TOML, presses text | Kept clip recipes, the player's own input to save points |
| `corpus.toml` | Required to publish a score | 2 | TOML | Corpus name, version, recipe hashes |
| `aimed.toml` | Optional | 3, committed | TOML | Clips aimed at fixes |
| `loop.toml` | Optional | 3, committed | TOML | The improvement loop's recipe |
| `evidence/<model>/<corpus>.json` | Optional | 2 | JSON (in-toto) | Score records, signatures |
| `ext/` | If the model has hooks | 1 | Rust crate | Hook code |
| `levels/`, `themes/`, `sprites/`, `sounds/` | Optional | 1 | Model level format, theme TOML, lifted formats | Original content by the package's authors |
| `README.md`, `LICENSE`, `NOTICE` | Yes (NOTICE for a commercial base) | 2 | Markdown with YAML card | The card |
| `data/*.parquet`, `croissant.json`, `gp-dataset.json` | No: derived | 2 when published, else 3 | Parquet, JSON-LD | facts, mechanics, mapping, clips, scores, `nodes`, `edges`, difficulty windows |
| `assets/<source>.toml` (in the repo today, per source) | Yes for a ROM | 3, committed | TOML | The asset map: ROM ranges to assets |
| Generated clips, the run's clip pool, starts, snapshots | Never | 3 | | Run folders |
| Code coverage, traces, recordings, frames | Never | 3 | | `$GP_STORE`; hashes may appear in records |
| Measures (per-frame) | Never for a commercial game until decided | 3 (2 for an open game) | Parquet | datasets.md section 7 |
| The ladder's measures (`gp progress --measure`) | Never | 3 | JSON | `$GP_HOME/work/progress/` |
| Attempts and tells | Attempts are commits; tells: Open | 2 as commit history; tells 3 until their record format is settled | git; signed records (community-review.md) | |
| RAM-role ignore list | Doesn't exist yet (progress.md) | 2 when made | TOML in `mapping.toml` | Bytes the model deliberately doesn't need |
| `print/`, `target/` | Never | 1 output | | The printed game |

### Formats and why

| Kind | Format | Why |
|---|---|---|
| Authored sources | TOML, canonical | Diffs one line a fact, merges in git, JSON Schema for editors (`.taplo.toml`); unchanged from today |
| Derived tables and the graph | Parquet, sorted, fixed writer settings | Byte-stable, column types and units in metadata, one-line loading everywhere |
| Records | JSON in-toto statements | Signable; datasets.md section 8 |
| Publishing metadata | README YAML card, Croissant JSON-LD | What the Hub and dataset search read |
| A graph store | None committed or published | A local convenience; LadybugDB or DuckDB can load the Parquet in under a second |

### Schemas

| Piece | Schema today | Version | Missing |
|---|---|---|---|
| Model core, platformer, contract, slots, theme, chunks, image config, capture events | JSON Schema from the Rust types, `schema/` | 0.4.22, 0.4.27, 0.2.0, ... | |
| Mapping | Parsed in `gp-interp`, versioned by `schema` (19) | 19 | A JSON Schema from its types |
| Profiles | `format = "0.2.0"` | 0.2.0 | JSON Schema |
| Clips | `format = "0.3.0"` in the doc (the SMB1 file says 0.2.0) | 0.3.0 | JSON Schema |
| Corpus | `format = "0.2.0"` | 0.2.0 | JSON Schema |
| Loop | `format = "0.1.0"` | 0.1.0 | JSON Schema |
| Asset map | `schema = 1` | 1 | JSON Schema |
| Dataset tables | Columns in hf-datasets.md and Croissant | `gp.dataset.hf_format` | A versioned table schema file per config |
| Graph nodes and edges | This report's draft | 0.1.0 (proposed) | All of it |

Each is versioned the way the formats are now: a reader takes its own major and minor and every older 0.x one, and a change says what readers must do.

### Derived artefacts and how they are rebuilt

| Artefact | Rebuilt by | From | Hash |
|---|---|---|---|
| Canonical model | `gp model fmt` | `model.toml` | Model hash (names scores) |
| Lifted assets | `gp assets lift` | ROM + asset map | Per file, local only |
| Clips from recipes | `gp capture regenerate`, `gp corpus verify` | ROM + `clips.toml` | Each clip's `curve`; the corpus hash |
| Score records | `gp capture fidelity` | Model + corpus + ROM | Record hash, signed |
| Dataset tables | `gp dataset export --hf` | The package (+ pinned download for `routine`) | Output hashes in `gp-dataset.json` |
| Graph | `gp dataset export --graph` (proposed) | The package, the pinned download, the corpus's coverage | Inputs and outputs in the graph manifest |
| Difficulty windows | `gp level difficulty` | Model + level | Not yet hashed: Open |

### Validation commands

| Command | Checks |
|---|---|
| `gp package check DIR` (proposed, datasets.md) | Required files present and parse, format versions readable, manifest hashes match, `clips.toml` and `sources.toml` name one ROM, every `evidence/` file names a model hash, nothing from the "Never" rows present (the pre-commit hook's content checks on the directory), derived files (when present) match their manifest |
| `gp model check` | Schema, evidence for every fact, refs to file:line and quotes, pinned downloads by hash |
| `gp corpus verify` | Every clip made again from the player's ROM matches its recipe hash and curve |
| `gp dataset export --check` (proposed) | The exported tables and graph rebuild to the hashes their manifest names |

### Where the build-time data could earn a place in the product

| Idea | Uses | Sharing line | Verdict |
|---|---|---|---|
| Attract mode or demo reels playing corpus clips' inputs in the rebuild | Clip presses (tier 2) played by the rebuild | Fine: presses are the player's or a TAS's (TASVideos, CC BY 2.0), the frames are the rebuild's | Earns a place: cheap, and it shows the rebuild playing the original's inputs |
| Practice mode: a ghost of a TAS or clip, "show me how this trick works" | Presses and the model's events (tier 2); difficulty windows for the timing | Fine, same reason | Earns a place, after attract mode; the windows make the "how" concrete |
| A fidelity badge or "about this build" screen | The score record and corpus version (tier 2) | Fine: numbers and hashes | Earns a place: small, and it is the claim the project makes |
| Level select with the difficulty heat strip | Difficulty windows (tier 2 once hashed) | Fine | Earns a place in a practice or level-select screen |
| In-game "report a difference" that writes a tell recipe | The clip recipe format plus a frame and a place (tier 2) | Fine if the tell holds presses and a frame number, never a frame image of the original | Earns a place, later: it needs the tell record format (community-review.md) |
| The graph view | Graph edges (tier 2) | Names only, no code text | Editor and inspector only, not the game |
| Coverage, traces, ladder measures | Tier 3 | They come from the ROM's runs | Stay build time |

### Hugging Face homes

| Home | Holds | Fit |
|---|---|---|
| A dataset repo per game | The corpus recipes, score records, facts, mechanics, mapping, clips, scores, nodes, edges, difficulty windows, and the card | Good: what hf-datasets.md built |
| A cross-game dataset repo | The mechanics catalogue (`taxonomy`, facts, mechanics) | Good |
| A model repo per game (the user's suggestion) | `model.toml`, `mapping.toml`, `profiles.toml`, a model card with the ROM hash, the score and the corpus version | Possible, but a poor fit today. Model repos are filtered by library and task; none applies to a TOML game model (`library_name` would be a new value). The older `model-index` metadata takes a task type, a dataset name and type and metric values, so "fidelity 77 on smb1-ntsc-prg0 2.0.0" can be written. The newer `.eval_results/` format names a Hub dataset that must be registered as a Benchmark (an allow-list in beta, with an `evaluation_framework` Hugging Face maintains), and can pin the dataset's revision, which is what a corpus version needs. Every library the Hub lists is machine learning, the closest being ML-Agents and Stable-Baselines3 for games; no non-ML precedent was found |
| Local only | Everything in tier 3 | |

Recommendation: dataset repos only, for now. Put the model files in the game's dataset repo, where the scores that name them are. Revisit a model repo when a corpus can register as a Benchmark, because then the score shows on a leaderboard with its corpus revision pinned. Nothing is uploaded either way until the user decides (data-publishing.md).

### Still undecided

| Choice | Default here |
|---|---|
| `gp-package.toml` or keep `template.toml` | Keep `template.toml` as an alias |
| Whether `assets/<source>.toml` moves into the package | Move it: it is the package's recipe for its own assets |
| Whether code edges (routine names, RAM labels) are published | Names only, as hf-datasets.md decides for `routine` |
| Zstd or uncompressed Parquet for byte stability across writers | Uncompressed, as `gp dataset export` writes today; zstd was stable with one writer here |
| Whether tells are tier 2 | Not until the record format and consent are settled |
| A model repo on the Hub | No, until a corpus can be a Benchmark |
| Coverage for kept clips stored per corpus version | Yes, by hash, in `$GP_STORE`; the `runs` edges are derived from it |

## Open questions

- Build the graph export now, or after the next dataset work? Default: after the `facts` export gains commits and tells, so the provenance graph is worth reading; the code and asset edges can go first, since the page's "click an enemy" view needs only them.
- Routines from coverage instead of the nearest-label heuristic? Default: yes when a recording is there (hf-datasets.md already recommends it); the heuristic missed `GameEngine` here.
- Asset-decoder edges (`asset -decoded_by-> routine`): from the asset map's labels and the routines that read them, which the builder already finds as `reads_data`. Default: add them before the missing-mechanic query is used.
- DuckDB in the Rust toolchain: not needed. `gp` writes Parquet and walks with petgraph; DuckDB stays the reader for people and the page (DuckDB-Wasm).

## Sources

- Kuzu repository, archived on Oct 10, 2025: https://github.com/kuzudb/kuzu ; crate: https://crates.io/api/v1/crates/kuzu ; PyPI: https://pypi.org/project/kuzu/
- BetaKit, "Apple strikes deal to acquire Canadian database software startup Kuzu" (2026-02-12): https://betakit.com/apple-strikes-deal-to-acquire-canadian-database-software-startup-kuzu/
- LadybugDB repository: https://github.com/LadybugDB/ladybug ; releases: https://github.com/LadybugDB/ladybug/releases ; dbdb.io entry: https://dbdb.io/db/ladybugdb ; crate `lbug`: https://crates.io/api/v1/crates/lbug ; PyPI: https://pypi.org/project/ladybug/ ; Parquet import: https://docs.ladybugdb.com/import/parquet ; export: https://docs.ladybugdb.com/export/ ; blog, 2026-03-01: https://9904c494.ladybug-blog.pages.dev/post/ladybug-spreading-its-wings/
- DuckDB releases: https://github.com/duckdb/duckdb/releases ; crate: https://crates.io/api/v1/crates/duckdb ; non-deterministic behaviour: https://duckdb.org/docs/2.0/operations_manual/non-deterministic_behavior ; database checksum: https://duckdb.org/docs/2.0/guides/snippets/database_checksum
- Bamberg and Grust, "USING KEY in Recursive CTEs", DuckDB blog, 2025-05-23: https://duckdb.org/2025/05/23/using-key.html
- DuckPGQ extension page: https://duckdb.org/community_extensions/extensions/duckpgq ; repository: https://github.com/cwida/duckpgq-extension ; SQL/PGQ docs: https://duckpgq-docs.pages.dev/documentation/sql_pgq
- ten Wolde, Singh, Szarnyas and Boncz, "DuckPGQ: Efficient Property Graph Queries in an analytical RDBMS", CIDR 2023: https://ir.cwi.nl/pub/32773/32773.pdf
- SQLite WITH clause: https://www.sqlite.org/lang_with.html ; release: https://www.sqlite.org/index.html ; rusqlite: https://crates.io/api/v1/crates/rusqlite
- CozoDB: https://github.com/cozodb/cozo ; https://crates.io/api/v1/crates/cozo
- Neo4j: https://github.com/neo4j/neo4j ; neo4rs: https://github.com/neo4j-labs/neo4rs
- Memgraph: https://github.com/memgraph/memgraph ; BSL text: https://raw.githubusercontent.com/memgraph/memgraph/master/licenses/BSL.txt ; rsmgclient: https://crates.io/api/v1/crates/rsmgclient
- Oxigraph: https://github.com/oxigraph/oxigraph ; https://crates.io/api/v1/crates/oxigraph ; Apache Jena: https://jena.apache.org/download/index.cgi
- petgraph: https://crates.io/api/v1/crates/petgraph ; algorithms: https://docs.rs/petgraph/latest/petgraph/algo/index.html
- Apache AGE: https://github.com/apache/age
- Hugging Face model cards, evaluation results (model-index): https://huggingface.co/docs/hub/model-cards ; eval results and benchmarks: https://huggingface.co/docs/hub/eval-results ; libraries: https://huggingface.co/docs/hub/models-libraries
