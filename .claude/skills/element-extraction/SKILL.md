---
name: element-extraction
description: The code-first procedure for extracting one game element (an enemy, object, projectile or mechanic) from a ROM into Game Printer's model, by walking from its assets to the code that runs it, listing the inputs that code reads, generating clips that take every branch, and letting the detectors state any gap. Use it when adding or fixing any element, when a clip differs on an element, or when a person reports something wrong about one. Don't ask a person what an element does; the code says.
---

# Element extraction, code first

Game Printer extracts a game one element at a time. An element is an entity kind (a barrel, a fire, a customer), a projectile, an object (a lift, a springboard) or a player mechanic (jump, skid). The original's code says what the element reads and does. Clips exist to check the model against that code, branch by branch. A person is asked only what a person can judge by watching.

The owner's words, 2026-10-30: "the code should have driven that clip and answered itself, and thats how it should work for any element in the game. work from code through to the asset and generate clips to validate behavior." The example was a lava ball's second leap in another game: its routine read the game's random register for each launch, and the model's leap read none. The procedure below finds that without asking.

This skill came from game-printer. GAME below is a game's package folder in the games checkout (`games/donkey-kong`, `games/tapper`), where game-printer had `templates/T`; RUN is a run's name as `gp corpus score --run` takes it. Each step names the tool that does it today, or what to do by hand until a tool exists. The goal is a deterministic `gp element` command that runs every step with no LLM ([docs/research/deterministic-inference.md](../../../docs/research/deterministic-inference.md)).

Most of the code-reading tools read the NES's 6502 code. On an arcade game (Tapper, a Z80 board) `gp element inputs`, `gp code listing`, `gp code tables` and `gp interp draws --find` work from the board's Z80 code; `gp model static`, `gp capture dense` and `carry`, and `gp mechanic leads` still refuse an arcade game in words; `gp mechanic probe` reads RAM and the mapping, so it works on both. Where a step's tool refuses, do it by hand and note it in the dossier.

## The steps

| Step | Question | Tool today | By hand until a tool exists |
|---|---|---|---|
| 1. Element to assets | Which sprites, frames, palettes and object IDs are this element? | The asset map (`GAME/assets/*.toml`), `model.toml`'s `present.*` paths, the mapping's slot kinds, the labels file (`gp labels show GAME`) | Read the asset map's slot names |
| 2. Assets to code | Which routines spawn, move, draw and remove it? | `gp element inputs --run GAME --clip ID --measure PATH`: the routine that wrote the element's byte on the clip's first wrong frame, by the labels. `gp code listing ROM --clips ...` lists each routine a clip ran. The graph script ([docs/research/notes/graphs/build_graph.py](../../../docs/research/notes/graphs/build_graph.py), edges sprite to routine and ID to routine, [docs/research/graphs.md](../../../docs/research/graphs.md)) for routines no clip has run; its full build needs `gp dataset export`, which isn't here yet | For an element no clip shows yet: search the labels for its name and its object ID's dispatch entry |
| 3. Code to inputs | What does each routine read: which RAM bytes, the random register, the frame counter, the player's position, timers, tile data? | `gp element inputs --run GAME --clip ID [--frame F] [--measure PATH \| --byte LABEL] [--json]`: the writer slice of the original's trace, each read with its label, role and value, the branches' tests, the earlier writes in the frame and one step back across frames, and the sentence ("ROUTINE wrote ... from LABEL"). [docs/element.md](../../../docs/element.md) has the output | Only where it says it gives up: code run from RAM, an unmodelled opcode, a hardware register, or a byte that "came with the clip's start" (then check what the start carries) |
| 4. Inputs to the model | Which of those inputs does the model's rule for this element read? | `gp model check GAME`, `gp model mechanics GAME --module M`, the mapping, the facts' code citations | Compare the step 3 table with the model's components and params for the element |
| 5. Branches to clips | Which clips make each input vary and take each branch? | `gp capture dense GAME` and `gp capture carry` (starts where the element is loaded), `gp interp scenarios`, the code-steered method ([docs/research/code-steered-clips.md](../../../docs/research/code-steered-clips.md)); `gp mechanic probe` for a probe kept as a recipe with its measures, and sweeps over a value or an asset's rows ([docs/contributing-mechanics.md](../../../docs/contributing-mechanics.md), "Probe recipes"). For one branch way from a clip's start: the way as the RAM test that decides it, a `gp mechanic probe --file` sweep from the start, `--save` into a new session, `gp clip score GAME SESSION:N`, `gp corpus import GAME` then `--write`, and `gp code listing ROM --only ID --routine ADDR` to check the way ([docs/formats/clips.md](../../../docs/formats/clips.md), "A new clip that takes a branch way"). On arcade, `gp clip presses GAME ID --from-session SESSION/N` keeps a clip as presses. Don't write your own replay or search script | Pick movie frames where the element is live, and starts the carry accepts; name each clip after the routine and branch |
| 6. Clips to differences | Where does the rebuild part, and on which measure? | `gp clip score GAME ID`, `gp corpus score GAME --run RUN --only ID`, `gp agent score --run GAME --model DIR --clip ID`, `gp interp compare`, `gp interp slots` | Read the first wrong frame and the measure |
| 7. Differences to the answer | Which input or branch does the original use that the model doesn't? | `gp solve GAME --run RUN --missing` (bodies, states), the solvers, `gp interp moments` | Write the sentence: "the original's ROUTINE reads INPUT; the model's RULE reads none" |
| 8. Answer to the fix | A general mechanic option or mapping row that reads that input, with the code cited | `gp model refine` or a clip agent on the page's REFINE tab, given the step 7 sentence; `gp agent attempt` and `gp agent score` for a worktree's try | Write the fix, then score it: better on its clips, none worse, and land it with `gp model gate GAME --patch FILE -m MSG` |

game-printer's `gp agent brief` carried step 3's table into an agent's brief. It isn't here; give the agent `gp element inputs --json`'s output yourself.

## Rules

- Code before clips. Start from steps 1 to 4. A clip that wasn't chosen to vary an input or take a branch is a sample, not a check.
- Every number cites the code, by routine name and label. Never copy code text or ROM bytes into a repo.
- The answer is a sentence about inputs or branches, not "tune the number". If no input or branch differs, look at timing (which frame the routine runs on, every N frames, before or after the move) and start state (what the start carries).
- Fixes are general. An input the original reads becomes a mechanic option or mapping row any game can use.
- Don't ask a person what an element does. Ask only what a person judges by watching ([docs/agents.md](../../../docs/agents.md) and [docs/mechanic-page.md](../../../docs/mechanic-page.md) describe the questions a person answers by watching, on the page's CLIPS tab). If you are about to ask, a step above is missing; note which one.
- Log the result in the notes log of [docs/research/deterministic-inference.md](../../../docs/research/deterministic-inference.md): the element, the input or branch found, and which step's tool found it or would have.

## Done

An element, or a level made of elements, is done when every row here holds. The coordinator added the picture row on 2026-11-05, after 75m and 50m both shipped bodies drawn as boxes.

| Check | How |
|---|---|
| Its clips score | Each clip that takes one of its branches is at 100% together, or its gap is a step 7 sentence in the dossier. The quest log shows the gate's scores per row (`gp mechanic queue GAME`) |
| No body drawn as a box | `gp picture GAME --run RUN` (or `--clip ID` for each of the level's clips) reports no `fallback box (no sprite)` group: every kind it adds has a sprite, in the entity or in each state's `set`. The gate refuses a patch that adds a kind with no picture to a game whose kinds all have one ([docs/gate.md](../../../docs/gate.md), the pictures rule); a kind that is never seen is a carrier, `no_slot` and 1 by 1 |
| The model is canonical | `gp model fmt --check GAME` passes. The canonical form drops comments, so notes go in a `description` or an evidence `note` ([docs/model.md](../../../docs/model.md), "Canonical form") |
| The notes log has its row | As "Rules" says |

## Output: the element dossier

For each element, write one dossier while you work, in the work folder, not a repo: `$GP_HOME/work/elements/<game>/<element>.md` (`$GP_HOME` is `~/.gameprinter` by default).

| Section | Content |
|---|---|
| Assets | Slot names, object IDs, frame names |
| Routines | Name, role (spawn, move, draw, remove), called from |
| Inputs | Per routine: each label read, its role (random, timer, player, tile, own state): `gp element inputs --json`'s `bytes` |
| Model | The components and params that implement it, and which inputs they read |
| Clips | Each clip id, the routine and branch it takes, its score |
| Gaps | The step 7 sentences, open or fixed |

The dossier's tables are the shape of the future `gp element` output and of the graph's element view. Keep them in this form.

## Becoming deterministic

Each step's "by hand" column is a gap to close with a tool, in order: step 4 (model inputs from the spec), step 5 (clips per branch), step 7 (the input-difference detector). Step 3 has its tool: `gp element inputs`. When a step gets a tool, update its row here and remove its by-hand column entry.
