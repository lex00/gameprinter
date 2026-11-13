//! The tools: their names, descriptions, argument schemas and what each
//! does. docs/mcp.md is the reference; the names are the roadmap's with
//! `_` for `.` (`surface.describe` is `surface_describe`), since some MCP
//! clients allow only letters, digits, `_` and `-` in a tool name.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path as FsPath;

use gp_contract::{CONTRACT_FILE, Contract, Level};
use gp_interp::{Interp, Lifted};
use gp_model::{MODEL_FILE, Model};
use gp_runtime::{InputFrame, Runtime};
use schemars::JsonSchema;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::edit::{self, ContractOp, Edit};
use crate::{Config, Reply, model_check, run_gp};

/// The most frames a preview runs: ten minutes at 60 frames a second.
pub const MAX_PREVIEW_FRAMES: usize = 36_000;

/// The most frames a preview reads its paths at.
pub const MAX_SAMPLES: usize = 1_000;

/// The longest AGENTS.md `surface_describe` returns.
pub const MAX_RECIPES_BYTES: usize = 64 * 1024;

/// The most events a preview returns.
pub const MAX_EVENTS: usize = 1_000;

/// One tool as the server lists it.
pub struct ToolDef {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
    /// Changes nothing.
    pub read_only: bool,
    /// May replace what's there (a working copy's model or contract).
    pub destructive: bool,
    /// Calling it twice with the same arguments does what calling it once
    /// does.
    pub idempotent: bool,
}

fn schema<T: JsonSchema>() -> Value {
    let mut v = serde_json::to_value(schemars::schema_for!(T)).expect("a schema serializes");
    if let Some(o) = v.as_object_mut() {
        o.remove("$schema");
        o.remove("title");
    }
    v
}

/// Every tool, in the order an agent uses them. A new tool is a `ToolDef`
/// here, an arm in [`call`] and its argument type; a test checks that every
/// listed tool has an arm.
pub fn list() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "next",
            title: "Where the port stands and what's next",
            description: "Where the port of a game stands (stands, one sentence for the person), the one next step and who does it (next.who: tools, agent or person; next.how says the page's button and the command), up to two other steps (also), what runs in the background and when to look again (running), anything that costs money or makes something public (waits_for_yes), and a clue when the person seems lost (clue). text is all of it as the terminal prints it. Read from files only, the same answer as `gp progress --next` and the status line on the page; it never starts work. Works before any game exists (it asks for the game's file) and on a finished port. With wait_s (at most 30) it waits for the answer to change, so a hand-off or an answer the person gives on the page reaches you: changed says whether it did.",
            input_schema: schema::<crate::next::NextArgs>(),
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolDef {
            name: "registry_search",
            title: "Search templates and sources",
            description: "Search the templates (game folders holding a model and no ROM data, ready to copy) under the repo's templates/ and in the games checkout ($GP_GAMES_DIR, or a games folder at, above or beside the folder the server started in), and the sources registered in sources/*.toml by words, platform, genre (a model extension such as platformer), entity roles (boss), systems (platformer.enemies) and mechanics from the mechanic index (enemy:contact.kick, or jump.coyote for any role), or ask for the templates inspired by one (inspired_by) or most like one (like, ranked by the share of mechanics in common). Each result lists its mechanics, its inspired_by provenance and the templates that took from it. Each template says what it needs before it can be copied and played: its lifted assets, lifted from the person's own ROM of the source (found by hash in $GP_ROM_DIR or given to the server) for a ported game, or built from its authored.toml for a game made here. template_fork does that step itself when it can.",
            input_schema: schema::<crate::registry::SearchArgs>(),
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolDef {
            name: "template_fork",
            title: "Copy a template",
            description: "Copy a template (a game's folder: its model, the rules as text, and nothing from a ROM) into a new copy for the person, which every other tool then names. template is the folder's name or the template's id, under the repo's templates/ or in the games checkout. The whole folder is copied, subfolders too, except build output, git's files and any ROM image; the template is never changed. A copy runs on its lifted assets (art, sound and levels): when they aren't there yet, the fork makes them if it can do so here and offline (built from authored.toml for a game made here, lifted from the person's ROM when one is found), and otherwise ready is false and next says the step. notes say what else the copy can't do yet.",
            input_schema: schema::<crate::fork::ForkArgs>(),
            read_only: false,
            destructive: false,
            idempotent: false,
        },
        ToolDef {
            name: "surface_describe",
            title: "Describe a copy",
            description: "The model's header, input actions, contract rules, every fact by model path with its value, the contract level that governs it and its evidence, and the slot catalog: each asset slot's kind, role, shape (sizes and format limits), the entities that draw from it, whether the simulation reads it, its status (original, stub or replaced), provenance and the checks a replacement must pass. Also the template's change recipes (`recipes`, its AGENTS.md) and the names of its scenario tests (`scenarios`), when the copy has them.",
            input_schema: schema::<CopyArgs>(),
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolDef {
            name: "model_edit",
            title: "Edit the model",
            description: "Set or remove parts of the model by concrete model path. All edits apply together, the model is rewritten in canonical form and checked with gp model check; if the check fails nothing is written. Returns the change by model path.",
            input_schema: schema::<EditArgs>(),
            read_only: false,
            destructive: true,
            idempotent: false,
        },
        ToolDef {
            name: "contract_edit",
            title: "Edit the fidelity contract",
            description: "Add, replace or remove contract rules, or change the default level. The contract is rewritten in canonical form and checked; if the check fails nothing is written. Returns the facts whose level changed.",
            input_schema: schema::<ContractArgs>(),
            read_only: false,
            destructive: true,
            idempotent: false,
        },
        ToolDef {
            name: "assets_generate",
            title: "Generate an asset for a slot",
            description: "Make a tiles or palettes asset for a slot, or for one entity's sprite in it, from the slot's shape and the entity's role, then place it with assets_place's checks. Generators: stub (numbered tiles, one hue per role), shapes (a shape per role at the sprite's size; a whole sheet is stub tiles with every sprite drawn in), recolor (indices or hues turned; keeps the current content's license). Deterministic: the same seed gives the same bytes. With place false it only returns the candidate. Every candidate is reviewed by the consistency critic (critic: advise by default, enforce to hold back a failing one, off to skip): palette coherence, size and role, contrast against the level's backdrop, style against the sheet's other art. With progressive true it fills every presentation slot in order (palettes, base sheets, then sprites by role), trying tries seeds per sprite and placing the best reviewed; plan_only returns the order.",
            input_schema: schema::<crate::generate::GenerateArgs>(),
            read_only: false,
            destructive: true,
            idempotent: true,
        },
        ToolDef {
            name: "assets_place",
            title: "Place an asset in a slot",
            description: "Replace a tiles slot's image or a palettes slot's palettes in the copy, or only what one entity's sprite uses (entity): its tiles, or its one palette. An image must be an indexed PNG of the slot's or the sprite's image_size with color indices 0 to 3 (surface_describe gives both); a palette is four NES color numbers, 0 to 63. It is packed, the model is checked and every baseline of the source is run before and after: the placement is refused, and nothing written, unless the state digests are the same. The slot's status, origin, license and digest go into the copy's assets/slots.toml; replacing part of a slot records both licenses.",
            input_schema: schema::<crate::place::PlaceArgs>(),
            read_only: false,
            destructive: true,
            idempotent: true,
        },
        ToolDef {
            name: "assets_edit",
            title: "Edit an entity's sprite or a level tile's look",
            description: "Edit an entity's sprite set or palette with deterministic semantic ops. ops run in order on each frame picked (frames: base, clip0.1, state.dead or all; the reply's sprite_set lists them): recolor (pairs of color indices, swapped at once, optionally in a region), flip, mirror, shift, outline, fill, pixels, copy (another entity's sprite or tiles of the sheet), load (a PNG an external editor saved after assets_export), match_style (take another entity's outline), tween (an in-between of two frames). frames also takes a clip's name (clip0) for all its frames; frames that share tiles must agree after the ops or the edit is refused, and also_changes lists other frames a changed tile draws. pictures puts whole frames in by rows; import reads back a folder assets_export wrote that an editor saved into (with the image format's checks), recorded as uploaded from that editor. palette_ops instead change the entity's palette colors (color, swap, hue, brightness, copy_from; same_role for every palette of its role). The model must know the sprite well enough to edit, or it is refused with what's missing. Each picture keeps its size and tiles. The result is reviewed by the consistency critic, checked against the hitbox and standing edge, drawn into the recorded frames it shows in with its readability against the backgrounds there (in_context), and placed with assets_place's checks. place false makes a proposal that accept places. Every edit has a seq in the copy's history: undo (newest, or seq when no later edit touched its tiles) any number of times, redo, accept or reject (seq). run names a live capture run, whose workspace panel shows each edit with accept and reject. tile instead of entity edits a level tile kind (a metatile, by its asset map name such as \"stone block\", <slot>[<index>] or its tiles in hex): ops (recolor, flip, mirror, shift, outline, fill, pixels, pictures) on its 16x16 picture of four tiles, palette_ops on the background palette the level draws it in (same_role for every palette of its role); also_changes names the other tile kinds, and also_changes_sprites the sprites, that draw a changed tile or palette; in_context crops the recorded frames around cells of the kind; proposals, accept, reject, undo and redo are the same.",
            input_schema: schema::<crate::asset_edit::EditArgs>(),
            read_only: false,
            destructive: true,
            idempotent: false,
        },
        ToolDef {
            name: "assets_history",
            title: "List a copy's art edits",
            description: "A copy's sprite and palette edits by seq with their status (proposed, placed, accepted, rejected, undone) and who decided: what a person accepted or rejected in the live panel. waiting lists proposals still open.",
            input_schema: schema::<crate::asset_edit::HistoryArgs>(),
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolDef {
            name: "assets_export",
            title: "Export a sprite for a pixel editor",
            description: "Write one entity's whole sprite set for an external pixel editor (Pixelorama by default, aseprite for Aseprite and LibreSprite, or png) by the image format config (docs/formats/image-config.md): a sheet or frame PNGs, the editor's palette file and the sidecars, in the copy's exchange/<entity>/set/, plus the sprite's own frame as one PNG and .gpl when it is a grid of tiles. Bring a save back with assets_edit import (the folder): every import and edit check runs, and the slot records it as uploaded from the editor.",
            input_schema: schema::<crate::exchange::ExportArgs>(),
            read_only: false,
            destructive: false,
            idempotent: true,
        },
        ToolDef {
            name: "sound_banks",
            title: "List the sound banks",
            description: "The sound banks there are to play captured music through: the faithful APU synthesizer (its pulse, triangle, noise and DMC voices) and every free SF2 SoundFont and SFZ file in $GP_SOUND_BANKS, $GP_HOME/sound-banks and the copy's sound/ folder, each with its licence and patches (SF2 presets by bank and program, SFZ regions by key range), and a one-second preview WAV of each patch (the first eight of a bank). Map an instrument or a role to a patch with sound_edit's remap op.",
            input_schema: schema::<crate::sound::BanksArgs>(),
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolDef {
            name: "sound_describe",
            title: "Describe the music and effects",
            description: "A copy's songs and sound effects, slot by slot: each song's channels and notes, its instruments with their roles (lead, bass, harmony, drums, sfx) and what each plays through now (the APU, a GM program or a bank's patch), where the notes came from (driver, log, import, edited) and how sure capture is, its tempo and loop, what the model binds it to (a level, a rule state, an event), and whether it's ready to edit (editable, with what's missing when not). Read this before sound_edit.",
            input_schema: schema::<crate::sound::DescribeArgs>(),
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolDef {
            name: "sound_edit",
            title: "Edit the music and effects",
            description: "Change how a copy's music and effects sound with deterministic ops, run in order: remap (an instrument or a role, in one song or all, to {kind: apu}, {kind: gm, program} or {kind: bank, bank, preset or program}, with a gain), role, transpose (semitones, a song or one channel), tempo (frames per row), mute and solo (channels), gain (a channel or a role), swap (a song becomes a level's or a rule state's music). A song an op names must be ready (sound_describe's editable): otherwise it is refused with what's missing. Every result is checked against the NES's channels; nothing is clamped. The notes go to the copy's sound/<slot>.songs.json and the mapping to sound/banks.toml, or with theme to that theme's restyled-audio layer, so a theme remasters the music without touching the simulation. Returns before and after WAV clips of the song; each placed edit goes into the copy's history and undo true puts back the last. With run, the edit is posted to a live capture run (kind sound_edit) for the person to accept or reject. place false previews without writing.",
            input_schema: schema::<crate::sound::EditArgs>(),
            read_only: false,
            destructive: true,
            idempotent: false,
        },
        ToolDef {
            name: "sound_preview",
            title: "Hear the game's sound in context",
            description: "Run the copy in the model interpreter on a baseline's input (or frames and holds) with its music and effects cued as the game cues them (the level's song, a rule state's song, each event's effect), played by the APU synthesizer with the copy's mix, into a WAV file. Returns the file, its length and the cues with their frames.",
            input_schema: schema::<crate::sound::PreviewArgs>(),
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolDef {
            name: "run_preview",
            title: "Preview a run",
            description: "Run the copy in the model interpreter on scripted input (frames and button holds, such as a@60-70) or a baseline's input, and return its events and its state at chosen paths and frames. No picture yet: the model has no renderer.",
            input_schema: schema::<PreviewArgs>(),
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolDef {
            name: "validate",
            title: "Validate against the original",
            description: "Check the model, then hold it to its fidelity contract with gp validate, the interpreter as the subject, against the original game's recorded runs in the repo's baselines/<source> (only Moss Hop has them today, so on another copy this step finds nothing to run). ok is false when a check fails; each check gives its measured and expected values. With scenarios true it also runs the copy's scenario tests (scenarios.toml) in parallel and returns the failing expectations under scenarios.",
            input_schema: schema::<ValidateArgs>(),
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolDef {
            name: "model_refine",
            title: "Refine the model from the baselines",
            description: "The refinement loop on the copy: record the source's baselines (or the named ones) with frame traces, then repeatedly validate, measure feel parameters from the recordings' tracks (walk and run speed, acceleration, jump speed and gravity, fall limit, enemy steps, periods and speeds), write each measured fact a failing check backs (with observed evidence) and each answered question, and ask the rest, until the checks pass or only questions remain. Returns the rounds (iterations), the facts written and the open questions. Needs the source ROM, so it works on a ported game's copy; a game made here has no ROM to measure.",
            input_schema: schema::<crate::refine::RefineArgs>(),
            read_only: false,
            destructive: true,
            idempotent: false,
        },
        ToolDef {
            name: "model_questions",
            title: "Questions for the person",
            description: "The copy's open questions from model_refine: each with its candidate answers and what would settle it. answers (ID=N) choose candidate N, written into the model as a stated fact with evidence. Ask the person; don't answer for them.",
            input_schema: schema::<crate::refine::QuestionsArgs>(),
            read_only: false,
            destructive: true,
            idempotent: true,
        },
        ToolDef {
            name: "show_explain",
            title: "What the person showed",
            description: "What the person showed you by playing the original in the workspace: each moment they marked (\"hit a brick\"), with what the original did in those frames that the model didn't (cell changes, interactions with their catalog kind, sound requests, counters, sprites, routines and RAM rare elsewhere) and what the model did on the same input. Reads the play session gp serve recorded and explained (default the newest). Detect only: it doesn't change the model; use it to answer the person or to decide what to edit.",
            input_schema: schema::<crate::show::ShowArgs>(),
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolDef {
            name: "build_publish",
            title: "Build for the browser",
            description: "Build the copy for the browser into a local directory that a page on this computer plays (there's no hosted runtime yet). The one target is web-preview, the interpreter preview. Writes manifest.json with the template, the model's digest, every slot's provenance from slots.toml, the licenses and each file's digest. Refused when the model check fails, a slot's files don't match their recorded digest, or any slot still holds an original asset of a commercial source. dry_run runs the checks only.",
            input_schema: schema::<crate::publish::PublishArgs>(),
            read_only: false,
            destructive: false,
            idempotent: true,
        },
        ToolDef {
            name: "capture_events",
            title: "Read a capture run",
            description: "A capture run's events after a sequence number (gp capture, docs/capture-live.md): each stage's end, the numbers that moved, the facts that differ, the failing checks, the questions asked, and the inputs the person and the agent sent, with who sent them. Also the questions still open and the inputs waiting for the next round. Pass the last_seq you have as after to read only what's new.",
            input_schema: schema::<crate::capture::EventsArgs>(),
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolDef {
            name: "requests",
            title: "What the person pointed at",
            description: "The person's point-and-tell requests in a run (docs/point-and-tell.md): they paused both games, boxed part of the screen (\"a brick\", \"the goomba\", \"the score\") and said what should happen there (\"this is missing\", \"wrong look\", \"should react when I hit it\", or their own words). Each comes with its frame, the region in both games, the things resolved there, and every reply so far. explain (N) fails today: it runs gp request explain, which this gp doesn't have. reply answers one yourself: plain words for a person who doesn't read code, with a proposal item from assets_edit (place false) or sound_edit for them to accept or reject, or status needs_mechanic and would_build when no tool can make the change yet.",
            input_schema: schema::<crate::requests::RequestsArgs>(),
            read_only: false,
            destructive: false,
            idempotent: false,
        },
        ToolDef {
            name: "moments",
            title: "The game's moments in a run",
            description: "The game's own moments in the recording the person watches in a capture run (docs/point-and-tell.md, \"Moments in words\"): each jump, landing, death, life lost, counter change and level change of the original, numbered by kind, with its frames and what the model does there on the same presses (\"jump 1 at frames 197 to 217 (3.3 s): the original goes up 24 px; the model 23.1 px, on the same frame\"). Give said with the person's words (\"refine the very first jump\", \"the second coin\") to get the moment they mean in found; for \"this jump\" give the request's frame too. Use refine_moment to refine it.",
            input_schema: schema::<crate::moments::MomentsArgs>(),
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolDef {
            name: "refine_moment",
            title: "Refine one moment",
            description: "Ask for the refinement loop on one of the game's moments (from moments: its kind and number): the person's workspace starts it beside the two games, with its windows in that moment's frames and only the numbers of the mechanic that owns that kind of moment (a jump's are platformer.jump's), then measures any value it keeps on every other moment of the kind and turns the ones it makes worse into questions. It changes the run's copy only as the loop does; tell the person in plain words what was asked.",
            input_schema: schema::<crate::moments::RefineMomentArgs>(),
            read_only: false,
            destructive: false,
            idempotent: false,
        },
        ToolDef {
            name: "watch_start",
            title: "Watch stage 0 on a ROM",
            description: "Start a watch session (gp watch start, docs/watch.md) on an iNES ROM: stage 0 reads the ROM, runs its own start clips writing each frame's novelty (code first run, the first code in each switched bank, edges, pointer values, the places the static reader stopped that a run went past), reads again with the runs, then searches for presses that reach code no run reached, writing each clip it keeps. It runs in the background; follow it with watch_events. The person can watch the same session in the page at ?watch=NAME.",
            input_schema: schema::<crate::watch::StartArgs>(),
            read_only: false,
            destructive: false,
            idempotent: false,
        },
        ToolDef {
            name: "watch_events",
            title: "Follow a watch session",
            description: "A watch session's events after a seq (pass the last_seq you have as after), optionally only some kinds (bank, stop.reached, clip.kept, fact, behaviour, watch, hook). wait_ms waits up to 30 s for new events while the session is being written, so calling it in a loop follows the session live. Each event from a run names its recipe (run) and frame, which watch_detail and watch_point take.",
            input_schema: schema::<crate::watch::EventsArgs>(),
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolDef {
            name: "watch_watch",
            title: "Watch addresses in a session",
            description: "Run the session's recipes again watching writes to addresses (addrs: `$F0`, `0x6F0`); each write comes back as a watch event with its value and the writing instruction (bank:pc), at most 200 per address per recipe. Follow with watch_events, kinds watch and rerun.end.",
            input_schema: schema::<crate::watch::WatchArgs>(),
            read_only: false,
            destructive: false,
            idempotent: false,
        },
        ToolDef {
            name: "watch_hook",
            title: "Hook instructions in a session",
            description: "Run the session's recipes again with instructions hooked (at: `01:8000`, `C27D`, `sub_C1FB`): each time one is about to run it comes back as a hook event with its frame and the registers (a, x, y, s, p), at most 200 per hook per recipe. The hooks are the cartridge shim's: no cycle of the run changes.",
            input_schema: schema::<crate::watch::WatchArgs>(),
            read_only: false,
            destructive: false,
            idempotent: false,
        },
        ToolDef {
            name: "watch_detail",
            title: "The instructions of a window",
            description: "Every instruction of frames A..B (or A..=B) of a recipe the session ran, as gp trace instructions prints them (L3, made by running the recipe again; never stored). The same as gp reach --detail for the same window. max_lines cuts the trace (default 200); lines and trace_hash describe the whole.",
            input_schema: schema::<crate::watch::DetailArgs>(),
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolDef {
            name: "watch_explore",
            title: "Search again in a session",
            description: "Start another search from the session's start clips (gp watch explore), aimed by default at the targets of the places the static reader still stops, with its own budget and seed. Its progress, kept clips and their novelty go into the same session; it ends with watch.end.",
            input_schema: schema::<crate::watch::ExploreArgs>(),
            read_only: false,
            destructive: false,
            idempotent: false,
        },
        ToolDef {
            name: "watch_point",
            title: "What is under a box of the screen",
            description: "The sprites under a box of the picture (rect: x, y, width, height) at a frame of a recipe, each with the instruction that last wrote its bytes in the sprite table the game copies to OAM, and the routine that instruction is in, with a line of words for each (\"sprite 3, written by sub_C623 (C643) at frame 600\"). The person's clicks on the page's game ask the same; view_now gives the last one.",
            input_schema: schema::<crate::watch::PointArgs>(),
            read_only: false,
            destructive: false,
            idempotent: true,
        },
        ToolDef {
            name: "view_now",
            title: "What the person's page shows",
            description: "What the watch page shows now (the recipe and frame it plays, what it focuses on) and the last point at the screen, the person's or yours, with its answer.",
            input_schema: schema::<crate::watch::ViewArgs>(),
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolDef {
            name: "capture_input",
            title: "Send an input to a capture run",
            description: "Send an answer, a pin (won't model), an unpin or a note to a capture run as the agent. A watched run starts its next round when an answer or pin arrives. Answer only with what the person chose; a note is for telling the person something in the run's log.",
            input_schema: schema::<crate::capture::InputArgs>(),
            read_only: false,
            destructive: false,
            idempotent: false,
        },
    ]
    .into_iter()
    .map(|mut t| {
        t.input_schema = crate::capture::with_run(t.input_schema);
        t
    })
    .collect()
}

/// Calls a tool. `None` for a tool that doesn't exist; arguments that
/// don't fit the schema are an error reply, so the agent can correct them.
pub fn call(config: &Config, name: &str, args: Value) -> Option<Reply> {
    // A call that names a capture run is logged in it; the capture tools'
    // own `run` is their argument.
    let mut args = args;
    let run = if name.starts_with("capture_")
        || matches!(name, "requests" | "moments" | "refine_moment")
    {
        None
    } else {
        crate::capture::split_run(&mut args)
    };
    let logged_args = args.clone();
    // assets_edit also tells the run of each edit itself (an asset_edit
    // input with its previews, docs/sprite-editing.md), so it keeps `run`.
    if name == "assets_edit"
        && let (Some(r), Some(o)) = (&run, args.as_object_mut())
    {
        o.insert("run".into(), json!(r));
    }
    let mut reply = call_tool(config, name, args)?;
    // A watch tool's call is logged in its session, for the person's page.
    if name.starts_with("watch_") || name == "view_now" {
        crate::watch::log_call(config, name, &logged_args, &reply);
    }
    if let Some(run) = run
        && let Some(why) = crate::capture::log_call(config, &run, name, &logged_args, &reply)
        && let Some(o) = reply.json.as_object_mut()
    {
        o.insert("capture_log".into(), json!(why));
    }
    Some(reply)
}

fn call_tool(config: &Config, name: &str, args: Value) -> Option<Reply> {
    fn with<T: DeserializeOwned>(args: Value, f: impl FnOnce(T) -> Result<Reply, String>) -> Reply {
        match serde_json::from_value::<T>(args) {
            Ok(a) => f(a).unwrap_or_else(Reply::error),
            Err(e) => Reply::error(format!("arguments: {e}")),
        }
    }
    // One call at a time on a working copy, held for the whole call.
    let _lock = args
        .get("copy")
        .and_then(Value::as_str)
        .filter(|c| gp_model::path::is_name(c))
        .map(crate::CopyLock::take);
    Some(match name {
        "next" => with(args, |a| crate::next::next(config, a)),
        "registry_search" => with(args, |a| crate::registry::search(config, a)),
        "template_fork" => with(args, |a| crate::fork::fork(config, a)),
        "surface_describe" => with(args, |a| describe(config, a)),
        "model_edit" => with(args, |a| model_edit(config, a)),
        "contract_edit" => with(args, |a| contract_edit(config, a)),
        "run_preview" => with(args, |a| preview(config, a)),
        "validate" => with(args, |a| validate(config, a)),
        "assets_generate" => with(args, |a| crate::generate::generate(config, a)),
        "assets_place" => with(args, |a| crate::place::place(config, a)),
        "assets_edit" => with(args, |a| crate::asset_edit::edit(config, a)),
        "assets_history" => with(args, |a| crate::asset_edit::history(config, a)),
        "assets_export" => with(args, |a| crate::exchange::export(config, a)),
        "sound_banks" => with(args, |a| crate::sound::banks(config, a)),
        "sound_describe" => with(args, |a| crate::sound::describe(config, a)),
        "sound_edit" => with(args, |a| crate::sound::edit(config, a)),
        "sound_preview" => with(args, |a| crate::sound::preview(config, a)),
        "build_publish" => with(args, |a| crate::publish::publish(config, a)),
        "model_refine" => with(args, |a| crate::refine::refine(config, a)),
        "model_questions" => with(args, |a| crate::refine::questions(config, a)),
        "watch_start" => with(args, |a| crate::watch::start(config, a)),
        "watch_events" => with(args, |a| crate::watch::events(config, a)),
        "watch_watch" | "watch_hook" => with(args, |a| crate::watch::rerun(config, a)),
        "watch_detail" => with(args, |a| crate::watch::detail(config, a)),
        "watch_explore" => with(args, |a| crate::watch::explore(config, a)),
        "watch_point" => with(args, |a| crate::watch::point(config, a)),
        "view_now" => with(args, |a| crate::watch::view_now(config, a)),
        "capture_events" => with(args, |a| crate::capture::events(config, a)),
        "capture_input" => with(args, |a| crate::capture::input(config, a)),

        "show_explain" => with(args, |a| crate::show::explain(config, a)),
        "requests" => with(args, |a| crate::requests::requests(config, a)),
        "moments" => with(args, |a| crate::moments::moments(config, a)),
        "refine_moment" => with(args, |a| crate::moments::refine_moment(config, a)),
        _ => return None,
    })
}

fn read(path: &FsPath) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))
}

fn write(path: &FsPath, text: &str) -> Result<(), String> {
    fs::write(path, text).map_err(|e| format!("writing {}: {e}", path.display()))
}

pub fn load_model(dir: &FsPath) -> Result<(String, Model), String> {
    let text = read(&dir.join(MODEL_FILE))?;
    let model = Model::parse(&text).map_err(|e| format!("{MODEL_FILE}: {e}"))?;
    Ok((text, model))
}

fn load_contract(dir: &FsPath) -> Result<Option<(String, Contract)>, String> {
    let path = dir.join(CONTRACT_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let text = read(&path)?;
    let c = Contract::parse(&text).map_err(|e| format!("{CONTRACT_FILE}: {e}"))?;
    Ok(Some((text, c)))
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CopyArgs {
    /// The copy's name, from template_fork.
    pub copy: String,
}

fn describe(config: &Config, a: CopyArgs) -> Result<Reply, String> {
    let dir = config.existing_copy(&a.copy)?;
    let (_, model) = load_model(&dir)?;
    let contract = load_contract(&dir)?.map(|(_, c)| c);
    let bound = contract.as_ref().map(|c| c.bind(&model));
    let tree = model.to_tree();
    let evidence: BTreeMap<_, _> = gp_model::validate::evidence_for(&model)
        .into_iter()
        .collect();
    let mut by_level: BTreeMap<&str, usize> = BTreeMap::new();
    let facts: Vec<Value> = gp_model::facts(&tree)
        .into_iter()
        .map(|(path, value)| {
            let mut fact = json!({
                "path": gp_model::path::display(&path),
                "value": value,
            });
            if let Some(b) = &bound {
                let r = b.resolve(&path, None);
                *by_level.entry(r.level.name()).or_insert(0) += 1;
                fact["level"] = json!(r.level.name());
                if !r.tolerance.is_empty() {
                    fact["tolerance"] = json!(r.tolerance);
                }
                if let Some(i) = r.rule {
                    fact["rule"] = json!(i);
                }
            }
            if let Some(Some(key)) = evidence.get(&path)
                && let Some(e) = model.evidence.get(key)
            {
                fact["evidence"] = json!({
                    "key": key,
                    "source": e.source.name(),
                    "confidence": e.confidence,
                });
            }
            fact
        })
        .collect();
    // The slot catalog: the shape needs the asset map, from the copy's
    // own assets or the lifted ones, and the status their slots.toml.
    let assets = config.assets_of(&dir, model.model.source.as_deref());
    let map = assets
        .as_ref()
        .and_then(|a| fs::read_to_string(a.join("assets.toml")).ok())
        .and_then(|t| gp_assets::AssetMap::parse(&t).ok());
    let state = match &assets {
        Some(a) => crate::catalog::State::read(a)?,
        None => None,
    };
    let slots = crate::catalog::records(
        &model,
        contract.as_ref(),
        map.as_ref(),
        state.as_ref(),
        assets.as_deref(),
    );
    let contract_json = contract.as_ref().map(|c| {
        json!({
            "default": c.contract.default,
            "description": c.contract.description,
            "rules": c.rules.iter().enumerate().map(|(i, r)| {
                let mut v = serde_json::to_value(r).expect("a rule serializes");
                v["index"] = json!(i);
                v
            }).collect::<Vec<_>>(),
            "facts_by_level": by_level,
        })
    });
    // The template's change recipes and scenario tests, as the template
    // wrote them (AGENTS.md and scenarios.toml, copied into each fork).
    let recipes = fs::read_to_string(dir.join("AGENTS.md"))
        .ok()
        .filter(|t| t.len() <= MAX_RECIPES_BYTES);
    let scenarios: Vec<Value> = fs::read_to_string(dir.join("scenarios.toml"))
        .ok()
        .and_then(|t| t.parse::<toml::Table>().ok())
        .and_then(|t| t.get("scenarios").and_then(toml::Value::as_table).cloned())
        .map(|t| {
            t.iter()
                .map(|(name, s)| {
                    json!({
                        "name": name,
                        "description": s.get("description").and_then(toml::Value::as_str),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Reply::ok(json!({
        "copy": a.copy,
        "recipes": recipes,
        "scenarios": scenarios,
        "model": {
            "name": model.model.name,
            "title": model.model.title,
            "source": model.model.source,
            "schema": model.model.schema,
            "extensions": model.model.extensions,
            "view": model.model.view,
            "systems": model.model.systems,
            "start_level": model.model.start_level,
        },
        "input": tree.get("input"),
        "entities": model.entities.keys().collect::<Vec<_>>(),
        "levels": model.levels.keys().collect::<Vec<_>>(),
        "rules": model.rules.keys().collect::<Vec<_>>(),
        "counters": model.counters.keys().collect::<Vec<_>>(),
        "slots": slots,
        "contract": contract_json,
        "facts": facts,
    })))
}

/// Evidence to record for every edited path.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceArg {
    /// `inferred` for an agent's own choice; `stated` when a person asked
    /// for this value (then give a ref).
    pub source: String,
    /// How sure, in percent.
    pub confidence: u8,
    /// Why the value is what it is.
    #[serde(default)]
    pub note: Option<String>,
    /// What supports it, each `{ "at": "..." }`.
    #[serde(default)]
    pub refs: Vec<Value>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditArgs {
    pub copy: String,
    /// The edits, applied in order and written together or not at all.
    pub edits: Vec<Edit>,
    /// Without it, an edited fact keeps the evidence that covered it.
    #[serde(default)]
    pub evidence: Option<EvidenceArg>,
}

fn model_edit(config: &Config, a: EditArgs) -> Result<Reply, String> {
    let dir = config.existing_copy(&a.copy)?;
    let (old_text, old) = load_model(&dir)?;
    if a.edits.is_empty() {
        return Err("no edits".into());
    }
    let mut tree = old.to_tree();
    for e in &a.edits {
        edit::apply(&mut tree, e)?;
    }
    // A removed part takes the evidence entries that named only it along.
    let dropped = edit::drop_dangling_evidence(&old.to_tree(), &mut tree);
    if let Some(ev) = &a.evidence {
        let mut entry = toml::Table::new();
        entry.insert("source".into(), toml::Value::String(ev.source.clone()));
        entry.insert(
            "confidence".into(),
            toml::Value::Integer(ev.confidence.into()),
        );
        if !ev.refs.is_empty() {
            entry.insert(
                "refs".into(),
                edit::to_toml(&Value::Array(ev.refs.clone()))?,
            );
        }
        if let Some(n) = &ev.note {
            entry.insert("note".into(), toml::Value::String(n.clone()));
        }
        let table = tree
            .as_table_mut()
            .expect("a model is a table")
            .entry("evidence")
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        for e in a.edits.iter().filter(|e| !e.remove) {
            let key = gp_model::Path::parse(&e.path)
                .map_err(|err| err.to_string())?
                .to_string();
            table
                .as_table_mut()
                .ok_or("the evidence isn't a table")?
                .insert(key, toml::Value::Table(entry.clone()));
        }
    }
    let text = toml::to_string(&tree).map_err(|e| e.to_string())?;
    let new = match Model::parse(&text) {
        Ok(m) => m,
        Err(e) => {
            return Ok(refused(json!([
                { "file": MODEL_FILE, "path": "", "message": e.to_string() }
            ])));
        }
    };
    let changes = gp_model::diff::diff(&old.to_tree(), &new.to_tree());
    let checked = write_checked(
        config,
        &dir,
        MODEL_FILE,
        &old_text,
        &new.to_toml(),
        new.model.source.as_deref(),
    )?;
    Ok(match checked {
        None => Reply::ok(json!({
            "ok": true,
            "written": true,
            "changes": changes,
            "evidence_dropped": dropped,
        })),
        Some(problems) => refused(problems),
    })
}

/// An edit gp model check refused: nothing was written.
fn refused(problems: Value) -> Reply {
    Reply::refused(json!({ "ok": false, "written": false, "problems": problems }))
}

/// Writes `new` to `dir/file` and runs gp model check. On failure the old
/// text goes back and the problems are returned.
fn write_checked(
    config: &Config,
    dir: &FsPath,
    file: &str,
    old: &str,
    new: &str,
    source: Option<&str>,
) -> Result<Option<Value>, String> {
    let path = dir.join(file);
    write(&path, new)?;
    let result = model_check(config, dir, source);
    match result {
        Ok((true, _)) => Ok(None),
        Ok((false, report)) => {
            write(&path, old)?;
            Ok(Some(report["problems"].clone()))
        }
        Err(e) => {
            write(&path, old)?;
            Err(e)
        }
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContractArgs {
    pub copy: String,
    /// Operations, applied in order and written together or not at all.
    pub ops: Vec<ContractOp>,
}

fn levels(model: &Model, c: &Contract) -> BTreeMap<String, Level> {
    let b = c.bind(model);
    gp_model::facts(&model.to_tree())
        .into_iter()
        .map(|(p, _)| (gp_model::path::display(&p), b.resolve(&p, None).level))
        .collect()
}

fn contract_edit(config: &Config, a: ContractArgs) -> Result<Reply, String> {
    let dir = config.existing_copy(&a.copy)?;
    let (_, model) = load_model(&dir)?;
    let Some((old_text, old)) = load_contract(&dir)? else {
        return Err(format!("the copy has no {CONTRACT_FILE}"));
    };
    let mut new = old.clone();
    edit::apply_contract(&mut new, &a.ops)?;
    // Round trip, so what's written is what the parser reads.
    let new = Contract::parse(&new.to_toml()).map_err(|e| e.to_string())?;
    let before = levels(&model, &old);
    let after = levels(&model, &new);
    let changed: Vec<Value> = after
        .iter()
        .filter(|(p, l)| before.get(*p) != Some(l))
        .map(|(p, l)| json!({ "path": p, "old": before.get(p), "new": l }))
        .collect();
    let checked = write_checked(
        config,
        &dir,
        CONTRACT_FILE,
        &old_text,
        &new.to_toml(),
        model.model.source.as_deref(),
    )?;
    if let Some(problems) = checked {
        return Ok(refused(problems));
    }
    Ok(Reply::ok(json!({
        "ok": true,
        "written": true,
        "rules": new.rules.iter().enumerate().map(|(i, r)| {
            let mut v = serde_json::to_value(r).expect("a rule serializes");
            v["index"] = json!(i);
            v
        }).collect::<Vec<_>>(),
        "default": new.contract.default,
        "levels_changed": changed,
    })))
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreviewArgs {
    pub copy: String,
    /// How many frames to run, when there's no baseline (default 60).
    #[serde(default)]
    pub frames: Option<usize>,
    /// Button holds, as in baselines: `a@60-70` holds A from frame 60 to
    /// 70, `right+b@10-` holds Right and B from frame 10 to the end.
    /// Frames count from 0.
    #[serde(default)]
    pub holds: Vec<String>,
    /// Instead of frames and holds, the input of a baseline of the model's
    /// source, by name: `jump-height` is baselines/<source>/jump-height.toml.
    #[serde(default)]
    pub baseline: Option<String>,
    /// State paths to read, such as `entities.player[0].y` or
    /// `counters.deaths`. Default: every value at the last frame.
    #[serde(default)]
    pub paths: Vec<String>,
    /// Frames to read the paths at, besides the last.
    #[serde(default)]
    pub at: Vec<u64>,
    /// Ask for a picture of the last frame. Not available yet; the reply
    /// says why.
    #[serde(default)]
    pub picture: bool,
}

pub fn input(config: &Config, model: &Model, a: &PreviewArgs) -> Result<Vec<InputFrame>, String> {
    let log = match &a.baseline {
        Some(name) => {
            if a.frames.is_some() || !a.holds.is_empty() {
                return Err("give a baseline, or frames and holds, not both".into());
            }
            let source = model
                .model
                .source
                .as_deref()
                .filter(|s| crate::is_source_name(s))
                .ok_or("the model names no source ID, so it has no baselines")?;
            if !crate::is_source_name(name) {
                return Err(format!("`{name}` isn't a baseline name"));
            }
            let path = config
                .repo
                .join("baselines")
                .join(source)
                .join(format!("{name}.toml"));
            let model_baseline =
                crate::model_baselines_dir(config, source).join(format!("{name}.toml"));
            if !path.is_file() && model_baseline.is_file() {
                gp_interp::baseline::ModelBaseline::load(&model_baseline)?.input()?
            } else {
                gp_evidence::baseline::Baseline::load(&path)
                    .map_err(|e| e.to_string())?
                    .input()
                    .map_err(|e| e.to_string())?
            }
        }
        None => {
            if a.frames.unwrap_or(0) > MAX_PREVIEW_FRAMES {
                return Err(format!(
                    "a preview runs at most {MAX_PREVIEW_FRAMES} frames"
                ));
            }
            let holds: Vec<&str> = a.holds.iter().map(String::as_str).collect();
            gp_evidence::input_log::generate(a.frames.unwrap_or(60), &holds)
                .map_err(|e| e.to_string())?
        }
    };
    if log.frames.len() > MAX_PREVIEW_FRAMES {
        return Err(format!(
            "{} frames is more than a preview runs ({MAX_PREVIEW_FRAMES})",
            log.frames.len()
        ));
    }
    Ok(log
        .frames
        .iter()
        .map(|f| InputFrame {
            buttons: f.buttons_word(),
            axes: Vec::new(),
            reset: f.reset,
        })
        .collect())
}

/// A runtime value as JSON: integers and flags as themselves, fixed-point
/// as an exact decimal string, as the model writes fractions.
pub fn value_json(v: &gp_runtime::Value) -> Value {
    use gp_runtime::Value as V;
    match v {
        V::Int(i) => json!(i),
        V::Fixed(raw) => json!(gp_sim::Fx::from_raw(*raw).to_string()),
        V::Flag(b) => json!(b),
        V::Text(t) => json!(t),
        V::Entity(e) => json!(e),
        V::Vec2(x, y) => json!([x, y]),
    }
}

pub fn read_paths(rt: &Interp, paths: &[String]) -> Result<Value, String> {
    let mut out = serde_json::Map::new();
    if paths.is_empty() {
        for (p, v) in gp_interp::state::leaves(rt.spec(), rt.world()) {
            out.insert(gp_model::path::display(&p), value_json(&v));
        }
    } else {
        for p in paths {
            for e in rt.query(p).map_err(|e| format!("`{p}`: {e}"))? {
                out.insert(e.path, value_json(&e.value));
            }
        }
    }
    Ok(Value::Object(out))
}

fn preview(config: &Config, a: PreviewArgs) -> Result<Reply, String> {
    let dir = config.existing_copy(&a.copy)?;
    let (_, model) = load_model(&dir)?;
    let frames = input(config, &model, &a)?;
    let assets = config
        .assets_of(&dir, model.model.source.as_deref())
        .ok_or("the model names no source, so start the server with --assets")?;
    let lifted = Lifted::read_dir(&assets, &model)
        .map_err(|e| format!("{e} (lift the model's assets with gp assets lift)"))?;
    let mut rt = Interp::load(&model, &lifted)?;
    let mut at = a.at.clone();
    at.sort_unstable();
    at.dedup();
    if at.len() > MAX_SAMPLES {
        return Err(format!("at most {MAX_SAMPLES} frames in `at`"));
    }
    if let Some(past) = at.iter().find(|f| **f > frames.len() as u64) {
        return Err(format!(
            "frame {past} in `at` is past the run's {} frames",
            frames.len()
        ));
    }
    let mut events = Vec::new();
    let mut dropped = 0usize;
    let mut samples = Vec::new();
    if at.first() == Some(&0) && !frames.is_empty() {
        samples.push(json!({ "frame": 0, "state": read_paths(&rt, &a.paths)? }));
    }
    for f in &frames {
        for e in rt.step(std::slice::from_ref(f))? {
            if e.kind == gp_interp::runtime::FRAME_KIND {
                continue;
            }
            if events.len() < MAX_EVENTS {
                events.push(json!({ "frame": e.frame, "kind": e.kind, "entity": e.entity }));
            } else {
                dropped += 1;
            }
        }
        let n = rt.frame();
        if at.binary_search(&n).is_ok() && n != frames.len() as u64 {
            samples.push(json!({ "frame": n, "state": read_paths(&rt, &a.paths)? }));
        }
    }
    samples.push(json!({ "frame": rt.frame(), "state": read_paths(&rt, &a.paths)? }));
    let mut out = json!({
        "frames": frames.len(),
        "state_hash": format!("{:016x}", rt.state_hash()),
        "events": events,
        "samples": samples,
    });
    if dropped > 0 {
        out["events_dropped"] = json!(dropped);
    }
    if a.picture {
        out["picture"] = json!(null);
        out["picture_note"] = json!(
            "no picture: the interpreter has no renderer yet (the runtime protocol's present interface isn't implemented), so a preview returns state and events only"
        );
    }
    Ok(Reply::ok(out))
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ValidateArgs {
    pub copy: String,
    /// Baselines of the model's source to validate against, by name.
    /// Default: all of baselines/<source>/.
    #[serde(default)]
    pub baselines: Vec<String>,
    /// Also run the copy's scenario tests (scenarios.toml, the template's
    /// plus any recorded) in parallel, and return them under `scenarios`.
    #[serde(default)]
    pub scenarios: bool,
    /// With `scenarios`: only these, by name.
    #[serde(default)]
    pub scenario_names: Vec<String>,
}

fn validate(config: &Config, a: ValidateArgs) -> Result<Reply, String> {
    let dir = config.existing_copy(&a.copy)?;
    let (_, model) = load_model(&dir)?;
    let source = model.model.source.as_deref();
    let (ok, report) = model_check(config, &dir, source)?;
    if !ok {
        return Ok(Reply::ok(json!({
            "ok": false,
            "stage": "model check",
            "problems": report["problems"],
        })));
    }
    let source =
        source.ok_or("the model names no source, so there's nothing to validate against")?;
    let base = config.repo.join("baselines").join(source);
    let against: Vec<std::path::PathBuf> = if a.baselines.is_empty() {
        vec![base]
    } else {
        a.baselines
            .iter()
            .map(|b| {
                if crate::is_source_name(b) {
                    Ok(base.join(format!("{b}.toml")))
                } else {
                    Err(format!("`{b}` isn't a baseline name"))
                }
            })
            .collect::<Result<_, _>>()?
    };
    let mut args: Vec<&std::ffi::OsStr> = vec![
        "validate".as_ref(),
        dir.as_os_str(),
        "--runtime".as_ref(),
        "interp".as_ref(),
        "--against".as_ref(),
    ];
    args.extend(against.iter().map(|p| p.as_os_str()));
    if let Some(r) = &config.rom {
        args.push("--rom".as_ref());
        args.push(r.as_os_str());
    }
    let assets = config.assets_of(&dir, Some(source));
    if let Some(a) = &assets {
        args.push("--assets".as_ref());
        args.push(a.as_os_str());
    }
    let (_, v) = run_gp(config, &args)?;
    let unchecked: BTreeMap<String, usize> = v["unchecked"]
        .as_object()
        .map(|o| {
            o.iter()
                .map(|(l, f)| (l.clone(), f.as_array().map_or(0, Vec::len)))
                .collect()
        })
        .unwrap_or_default();
    let failing: Vec<&Value> = v["checks"]
        .as_array()
        .map(|c| c.iter().filter(|c| c["status"] == "fail").collect())
        .unwrap_or_default();
    let scenarios = if a.scenarios {
        let req = crate::scenario::Request {
            copy: a.copy.clone(),
            template: false,
            names: a.scenario_names.clone(),
            jobs: None,
        };
        Some(match crate::scenario::run_all(config, &dir, &req) {
            // Failing checks only: the passing ones are noise to an agent.
            Ok(mut r) => {
                if let Some(rs) = r["results"].as_array_mut() {
                    for x in rs.iter_mut() {
                        if let Some(cs) = x["checks"].as_array_mut() {
                            cs.retain(|c| c["ok"] != true);
                        }
                    }
                }
                r
            }
            Err(e) => json!({ "ok": false, "error": e }),
        })
    } else {
        None
    };
    Ok(Reply::ok(json!({
        "scenarios": scenarios,
        "ok": v["ok"],
        "stage": "validate",
        "passed": v["passed"],
        "failed": v["failed"],
        "unexercised": v["unexercised"],
        "failing": failing,
        "checks": v["checks"],
        "unchecked_facts": unchecked,
        "recordings": v["recordings"].as_array().map(|r| r.iter().map(|r| r["name"].clone()).collect::<Vec<_>>()),
        "subject": v["subject"],
        "original": v["original"],
    })))
}
