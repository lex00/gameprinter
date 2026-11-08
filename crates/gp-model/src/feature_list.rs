//! The features of the model core and the platformer extension this build
//! knows ([`crate::features`]). Each optional option or component added
//! since 0.4.0 is one, with the version it appeared in kept as a note.
//!
//! To add an option: add its row here, with a new name and `since = ""`,
//! and don't bump a version. A breaking change bumps the piece's version
//! instead, once per release.

use crate::features::{Feature, Piece};

const fn core(
    name: &'static str,
    since: &'static str,
    about: &'static str,
    paths: &'static [&'static str],
) -> Feature {
    Feature {
        name,
        piece: Piece::Core,
        since,
        about,
        paths,
    }
}

const fn plat(
    name: &'static str,
    since: &'static str,
    about: &'static str,
    paths: &'static [&'static str],
) -> Feature {
    Feature {
        name,
        piece: Piece::Platformer,
        since,
        about,
        paths,
    }
}

const fn contract(
    name: &'static str,
    about: &'static str,
    paths: &'static [&'static str],
) -> Feature {
    Feature {
        name,
        piece: Piece::Contract,
        since: "",
        about,
        paths,
    }
}

/// The features of the fidelity contract format, which `gp-contract`
/// reads; listed here beside the model's so `just docs` prints them.
pub const CONTRACT_FEATURES: &[Feature] = &[
    contract(
        "corpus.together",
        "`[corpus] together`: each clip of the scoring corpus at this share of frames together or more, straight through from its start (`gp corpus score`)",
        &["corpus.together"],
    ),
    contract(
        "corpus.longest_stretch",
        "`[corpus] longest_stretch`: each long run of the scoring corpus with a longest exact stretch of this many seconds or more in resync mode",
        &["corpus.longest_stretch"],
    ),
];

/// The features of the asset map (`gp-assets`, docs/assets.md), listed
/// here so `just docs` prints them. Its `schema` stays 1.
pub const ASSET_MAP_FEATURES: &[Feature] = &[
    Feature {
        name: "asset_map.content",
        piece: Piece::AssetMap,
        since: "",
        about: "`[[content]]`: each span's BLAKE3, length and a fingerprint of each 16 bytes, written by `gp assets hash`, so `gp assets find` finds the span in other dumps (a pointer table into a list, `relative`, by its values less the list's address)",
        paths: &["content"],
    },
    Feature {
        name: "asset_map.code",
        piece: Piece::AssetMap,
        since: "",
        about: "`[code]`: the fingerprints of the ROM's whole PRG, which `gp assets find` compares a ROM's program with to pick the model",
        paths: &["code"],
    },
];

/// Every feature of the core and the platformer extension, core first,
/// each piece in the order the features appeared.
pub const FEATURES: &[Feature] = &[
    core(
        "sprite.cells",
        "0.4.1",
        "A sprite drawn from cells, each its own tile, flip and palette",
        &["entities.**.sprite.cells"],
    ),
    core(
        "sprite.colors",
        "0.4.1",
        "A sprite's own colors",
        &["entities.**.sprite.colors"],
    ),
    core(
        "sprite.clips",
        "0.4.1",
        "A sprite's animation clips",
        &["entities.**.sprite.clips"],
    ),
    core(
        "level.label",
        "0.4.1",
        "A level's label, drawn by the HUD",
        &["levels.*.present.label"],
    ),
    core(
        "present.cycles",
        "0.4.1",
        "Color cycles, for the whole game or a level",
        &["present.cycles", "levels.*.present.cycles"],
    ),
    core(
        "present.screen",
        "0.4.1",
        "The screen's size, aspect and overscan",
        &["present.screen"],
    ),
    core(
        "present.hud",
        "0.4.1",
        "The HUD: its tiles, font and fields",
        &["present.hud"],
    ),
    core(
        "asset.font",
        "0.4.1",
        "The asset kind `font`",
        &["assets.*.kind=font"],
    ),
    core(
        "asset.screen_text",
        "0.4.1",
        "The asset kind `screen-text`",
        &["assets.*.kind=screen-text"],
    ),
    core(
        "role.hud",
        "0.4.1",
        "The slot role `hud`",
        &["assets.*.role=hud"],
    ),
    core(
        "asset.sprite_set",
        "0.4.2",
        "The asset kind `sprite-set`, which a sprite's `frame` may name",
        &["assets.*.kind=sprite-set"],
    ),
    core(
        "clip.hold",
        "0.4.2",
        "A held clip, which needs no `every`",
        &["entities.**.sprite.clips.hold"],
    ),
    core(
        "sprite.blink",
        "0.4.2",
        "A sprite that blinks",
        &["entities.**.sprite.blink"],
    ),
    core(
        "clip.when_jumped",
        "0.4.2",
        "The clip condition `jumped`",
        &["entities.**.sprite.clips.when.jumped"],
    ),
    core(
        "clip.when_invulnerable",
        "0.4.2",
        "The clip condition `invulnerable`",
        &[
            "entities.**.sprite.clips.when.invulnerable",
            "entities.**.sprite.blink.when.invulnerable",
        ],
    ),
    core(
        "clip.when_climbing",
        "0.4.2",
        "The clip condition `climbing`",
        &["entities.**.sprite.clips.when.climbing"],
    ),
    core(
        "asset.songs",
        "0.4.3",
        "The asset kind `songs` (format `gp-songs`)",
        &["assets.*.kind=songs"],
    ),
    core(
        "asset.sound_effects",
        "0.4.3",
        "The asset kind `sound-effects`",
        &["assets.*.kind=sound-effects"],
    ),
    core(
        "slot.capture",
        "0.4.3",
        "A slot's capture recipe",
        &["assets.*.capture"],
    ),
    core(
        "present.sound",
        "0.4.3",
        "Music bound to levels and rule states, and effects to events",
        &["present.sound"],
    ),
    core(
        "camera.by_move",
        "0.4.4",
        "A camera that follows by its followed entity's move (`from`, `full`, `short_by`)",
        &["cameras.*.by_move"],
    ),
    core(
        "tile.blocks",
        "0.4.6",
        "The tile kinds `coin_block`, `item_block` and `brick`, told apart by a head hit",
        &[
            "levels.*.tile_kinds=coin_block",
            "levels.*.tile_kinds=item_block",
            "levels.*.tile_kinds=brick",
            "entities.**.jump.head.hits.tile=coin_block",
            "entities.**.jump.head.hits.tile=item_block",
            "entities.**.jump.head.hits.tile=brick",
            "entities.**.jump.head.hits.becomes=coin_block",
            "entities.**.jump.head.hits.becomes=item_block",
            "entities.**.jump.head.hits.becomes=brick",
        ],
    ),
    core(
        "hud.sprite",
        "0.4.7",
        "HUD sprites (`at`, `tiles`, `tile`, `palette`, `behind`)",
        &["present.hud.sprite"],
    ),
    core(
        "hud.field_sprite",
        "",
        "A HUD field drawn as sprites in a sprite palette (`sprite`), counted in the sprite layer: Moss Hop's coins and lives",
        &["present.hud.field.sprite"],
    ),
    core(
        "cycle.counter",
        "0.4.7",
        "A color cycle stepped by a counter, with its `shift`",
        &["**.cycles.counter", "**.cycles.shift"],
    ),
    core(
        "counter.runs_when_frozen",
        "0.4.8",
        "A counter whose `every` changes count on frozen frames too",
        &["counters.*.runs_when_frozen"],
    ),
    core(
        "clip.body_clock",
        "0.4.9",
        "A clip's `clock = \"body\"`, a timer per entity",
        &["entities.**.sprite.clips.clock=body"],
    ),
    core(
        "level.spawn_slots",
        "0.4.10",
        "Numbered spawn slots in a level",
        &["levels.*.spawn_slots"],
    ),
    core(
        "level.spawn_in_order",
        "0.4.10",
        "Spawns taken in order",
        &["levels.*.spawn_in_order"],
    ),
    core(
        "level.appear_still",
        "0.4.10",
        "Bodies that appear still",
        &["levels.*.appear_still"],
    ),
    core(
        "level.draw",
        "0.4.10",
        "A level's draw: spawns read in steps as the view moves",
        &["levels.*.draw"],
    ),
    core(
        "level.despawn_at",
        "0.4.10",
        "Where a level despawns bodies",
        &["levels.*.despawn_at"],
    ),
    core(
        "spawn.drawn",
        "0.4.10",
        "A spawn the level's draw reads",
        &["levels.*.spawn.drawn"],
    ),
    core(
        "entity.no_slot",
        "0.4.10",
        "An entity that takes no spawn slot",
        &["entities.**.no_slot"],
    ),
    core(
        "entity.despawn_behind",
        "0.4.10",
        "An entity's own despawn distance behind the view",
        &["entities.*.despawn_behind"],
    ),
    core(
        "camera.push_hold",
        "0.4.11",
        "A camera `by_move`'s push hold",
        &["cameras.*.by_move.push_hold"],
    ),
    core(
        "level.slot_rest",
        "0.4.11",
        "What a numbered slot's last body left, given to the next",
        &["levels.*.slot_rest"],
    ),
    core(
        "spawn.at_edge",
        "0.4.11",
        "A spawn placed at the view's edge",
        &["levels.*.spawn.at_edge"],
    ),
    core(
        "spawn.appear_moving",
        "0.4.11",
        "A spawn that appears moving",
        &["levels.*.spawn.appear_moving"],
    ),
    core(
        "state.stops",
        "0.4.12",
        "A state that freezes only the systems it names",
        &["**.states.*.stops"],
    ),
    core(
        "state.stops_at_once",
        "0.4.12",
        "Systems a state stops for the rest of the frame it's entered in",
        &["**.states.*.stops_at_once"],
    ),
    core(
        "state.holds",
        "0.4.12",
        "Counters a state holds",
        &["**.states.*.holds"],
    ),
    core(
        "input.keep_direction",
        "",
        "Pad directions latched as a game filters them: one still held keeps acting and new ones wait",
        &["input.keep_direction"],
    ),
    core(
        "state.step_word",
        "",
        "An entity state that steps the entity's step counter (mechanic word 0) every frame, frozen or not",
        &["**.states.*.step_word"],
    ),
    core(
        "state.stops_machines",
        "",
        "A state in which the entities' own machines wait",
        &["**.states.*.stops_machines"],
    ),
    core(
        "state.frees_at_once",
        "",
        "Systems that run for the rest of the frame on which a state is left",
        &["**.states.*.frees_at_once"],
    ),
    core(
        "transition.pressed",
        "",
        "A transition taken on a frame an input action is newly pressed",
        &["**.transition.*.pressed"],
    ),
    core(
        "input.turn",
        "",
        "Players taking turns each play from their own controller, by a counter",
        &["input.turn"],
    ),
    core(
        "stages",
        "",
        "The original's stages in order, a stand-in level playing each stage the model has no level for, a counter saying which",
        &["stages"],
    ),
    core(
        "stages.skip",
        "",
        "Stages the game passes over: the stage counter steps on past each, as a version of the game without that stage does",
        &["stages.skip"],
    ),
    core(
        "present.hud.field_when",
        "",
        "A HUD field drawn only while counter tests hold",
        &["present.hud.field.*.when"],
    ),
    core(
        "present.screens",
        "",
        "Whole screens shown in place of the level while a rule state holds, from update strings, with sprites a counter moves",
        &["present.screens"],
    ),
    core(
        "present.screens_when",
        "",
        "A screen shown in place of the level only while counters are in ranges",
        &["present.screens.when"],
    ),
    core(
        "present.sprite_order",
        "",
        "Each entity's place in the console's sprite table in a level, which decides the sprites the sprite limit drops on a crowded line",
        &["levels.*.present.sprite_order"],
    ),
    core(
        "present.effect_order",
        "",
        "Each effect's place in the console's sprite table in a level, by the event that starts it, which decides what it is painted over and under",
        &["levels.*.present.effect_order"],
    ),
    core(
        "present.sprites_off",
        "",
        "Rule states in which the console's sprite layer is off: no sprite shows",
        &["present.screen.sprites_off"],
    ),
    core(
        "present.hidden",
        "",
        "Bodies not drawn while a rule state holds and counters are in ranges: given entity types, the whole sprite layer, or the HUD",
        &["present.hidden"],
    ),
    core(
        "assets.screen_tiles",
        "",
        "A screen-text slot drawn with a tiles slot of its own where a screen shows it, in place of the level's: another pattern table or CHR bank",
        &["assets.*.tiles"],
    ),
    core(
        "assets.layout_tiles",
        "",
        "A level-objects slot filled from update strings drawn with a tiles slot of its own where a level's layout names it, in place of the level's: a stage the game draws with another CHR bank",
        &["assets.*.tiles"],
    ),
    core(
        "present.screens_blank",
        "",
        "A screen with no layout: the backdrop color alone, as a console with its picture off",
        &["present.screens.blank"],
    ),
    core(
        "present.screens_from_line",
        "",
        "A blank screen from a line down: the frame drawn as without it above the line, as a console whose picture is turned off partway down the frame",
        &["present.screens.from_line"],
    ),
    core(
        "present.screens_text",
        "",
        "Text on a screen, written into its name table in the HUD font's characters over its layout or blank: a turn screen's PLAYER I",
        &["present.screens.text"],
    ),
    core(
        "transition.held",
        "",
        "A transition taken on every frame an input action is held",
        &["**.transition.*.held"],
    ),
    core(
        "transition.word",
        "",
        "A timed transition of an entity's machine taken only while one of its mechanic words is in a range",
        &["**.transition.*.word"],
    ),
    core(
        "state.keep_time",
        "0.4.12",
        "A state that keeps its time",
        &["**.states.*.keep_time"],
    ),
    core(
        "state.seen_every",
        "0.4.12",
        "A state's `seen_every`",
        &["**.states.*.seen_every"],
    ),
    core(
        "transition.align",
        "0.4.12",
        "A transition's `align`",
        &["**.states.*.transition.align"],
    ),
    core(
        "camera.keep_in_view",
        "0.4.12",
        "A camera's bounds that keep its followed entity in view",
        &["cameras.*.keep_in_view"],
    ),
    core(
        "camera.keep_move",
        "0.4.12",
        "A camera `by_move`'s `keep_move`",
        &["cameras.*.by_move.keep_move"],
    ),
    core(
        "change.after",
        "0.4.13",
        "A counter change held back for some frames (`after`), and by whom (`of`)",
        &["counters.*.change.after", "counters.*.change.of"],
    ),
    core(
        "state.ground",
        "0.4.13",
        "An entity state's standing on entering it",
        &["**.states.*.ground"],
    ),
    core(
        "screen.sprite_limit",
        "0.4.14",
        "The console's sprites per line (`per_line`, `sprite_width`, `flicker`)",
        &["present.screen.sprite_limit"],
    ),
    core(
        "state.enter_speed",
        "0.4.15",
        "The speed entering an entity state sets",
        &["**.states.*.enter_speed"],
    ),
    core(
        "state.enter_keeps_fraction",
        "0.4.15",
        "An `enter_speed` that keeps the speed's fraction up and down",
        &["**.states.*.enter_keeps_fraction"],
    ),
    core(
        "tile.coin",
        "0.4.16",
        "The tile kind `coin`, which stops nothing",
        &[
            "levels.*.tile_kinds=coin",
            "entities.**.jump.pickup.becomes=coin",
        ],
    ),
    core(
        "camera.keep_in_view_touch",
        "0.4.17",
        "A first bound that acts at the bound, not only past it",
        &["cameras.*.keep_in_view_touch"],
    ),
    core(
        "sprite.offset",
        "0.4.18",
        "A sprite's picture placed by an offset from the entity",
        &["entities.**.sprite.offset"],
    ),
    core(
        "sprite.behind",
        "0.4.18",
        "A sprite drawn behind the level's picture",
        &["entities.**.sprite.behind"],
    ),
    core(
        "clip.speed_before",
        "0.4.18",
        "A `by_speed` clip that reads the speed of the frame before",
        &["entities.**.sprite.clips.speed_before"],
    ),
    core(
        "level.despawn_after_move",
        "0.4.19",
        "Despawning tested after the enemies move",
        &["levels.*.despawn_after_move"],
    ),
    core(
        "counter.shift",
        "0.4.20",
        "A counter that is a shift register",
        &["counters.*.shift"],
    ),
    core(
        "sprite.copy",
        "0.4.20",
        "A second picture of a body below the first (`below`, `wrap`, `above`)",
        &["entities.**.sprite.copy"],
    ),
    core(
        "sprite.replaced",
        "0.4.21",
        "A sprite drawn as the body it replaced, turned over",
        &["entities.**.sprite.replaced"],
    ),
    core(
        "sprite.leaves",
        "0.4.21",
        "The picture a successor with `replaced` draws",
        &["entities.**.sprite.leaves"],
    ),
    core(
        "sprite.front",
        "0.4.21",
        "A sprite drawn over every other body",
        &["entities.**.sprite.front"],
    ),
    core(
        "clip.when_word",
        "0.4.21",
        "A clip or blink condition on an `ext` hook word in a range",
        &[
            "entities.**.sprite.clips.when.word",
            "entities.**.sprite.blink.when.word",
        ],
    ),
    core(
        "clip.frame_offset",
        "0.4.21",
        "A clip frame's offset",
        &["entities.**.sprite.clips.frames.offset"],
    ),
    core(
        "entity.despawn_ahead",
        "0.4.22",
        "An entity's own despawn distance ahead of the view",
        &["entities.*.despawn_ahead"],
    ),
    core(
        "entity.extra_slots",
        "0.4.22",
        "An entity that takes more than one spawn slot",
        &["entities.*.extra_slots"],
    ),
    core(
        "camera.hold_with_counters",
        "0.4.23",
        "A camera `by_move`'s push hold counted with the counters system",
        &["cameras.*.by_move.hold_with_counters"],
    ),
    core(
        "entity.despawn_behind_wrap",
        "0.4.24",
        "A despawn limit that stands in for `despawn_behind` while the view's left edge is far enough into its page",
        &["entities.**.despawn_behind_wrap"],
    ),
    core(
        "level.hit_widths",
        "0.4.24",
        "The width an entity's touches use in a level",
        &["levels.*.hit_widths"],
    ),
    core(
        "level.scroll_stop",
        "0.4.24",
        "A drawn column that stops the scroll and removes some bodies",
        &["levels.*.scroll_stop"],
    ),
    core(
        "level.loop_back",
        "0.4.24",
        "A level that loops: a player not at a check's height goes back",
        &["levels.*.loop_back"],
    ),
    core(
        "transition.below",
        "",
        "An entity's transition taken once the entity is at a depth",
        &["**.states.*.transition.below"],
    ),
    core(
        "transition.align_to",
        "",
        "A transition taken on a frame a counter's clock comes round",
        &["**.states.*.transition.align_to"],
    ),
    core(
        "action.clear",
        "",
        "The action `clear:<entity>`, which takes every active body of an entity out of the level",
        &["**.states.*.transition.do=clear:*"],
    ),
    core(
        "action.respawn",
        "",
        "The action `respawn:<entity>`, which puts every active body of an entity back at its spawn's place",
        &["**.states.*.transition.do=respawn:*"],
    ),
    core(
        "action.set",
        "",
        "The action `set:<counter>:<value>`, which puts a counter at a value",
        &["**.states.*.transition.do=set:*"],
    ),
    core(
        "action.enter_level",
        "",
        "The action `enter-level`: `next-level` with every counter and clock kept and counting on its frame, and each player's mechanic words, for a game that draws the next stage and its objects before it sets the stage up",
        &["**.states.*.transition.do=enter-level"],
    ),
    core(
        "action.remove_body",
        "",
        "The action `remove-body:<counter>`, which takes out the body whose ID plus one a counter holds",
        &["**.states.*.transition.do=remove-body:*"],
    ),
    core(
        "action.add",
        "",
        "The action `add:<to>:<from>`, which adds one counter's value to another",
        &["**.states.*.transition.do=add:*"],
    ),
    core(
        "action.emit",
        "",
        "The action `emit:<event>`, which fires an event",
        &["**.states.*.transition.do=emit:*"],
    ),
    core(
        "change.if",
        "",
        "A counter change on an event made only while a counter condition holds",
        &["counters.*.change.if"],
    ),
    core(
        "counter.high_of",
        "",
        "A counter that keeps another's highest value, as a top score",
        &["counters.*.high_of"],
    ),
    core(
        "counter.restarts_clock",
        "",
        "A counter whose clocks start over when a level sets it",
        &["counters.*.restarts_clock"],
    ),
    core(
        "camera.keep_in_view_turn",
        "",
        "An entity in the air that reaches a keep_in_view bound turns around instead of stopping",
        &["cameras.*.keep_in_view_turn"],
    ),
    core(
        "camera.keep_in_view_rows",
        "",
        "Other keep_in_view bounds while the followed entity's row (its takeoff height in the air) is in a range, such as a wall short of the screen's edge on one girder",
        &["cameras.*.keep_in_view_rows"],
    ),
    core(
        "cost.frame",
        "",
        "The original's CPU cycles for a frame's work as terms over model state (counters at the frame's start or end, input, machine states, bodies of a kind), which hooks add to and read, for a generator stepped in the frame's spare time",
        &["cost"],
    ),
    core(
        "cost.code_walk",
        "",
        "An entity's `ext_tables` naming the original's code, lifted by the asset map as `code_*` tables, which its hook walks along the path the model's state decides (gp_cycles::walk) for a routine's cycles: the cycles come from the person's ROM through the recipe, and the hook names only the routine, each branch's outcome by its address and the draws it measures",
        &["entities.*.ext_tables=code_*"],
    ),
    core(
        "cost.frame_pad",
        "",
        "A cost term's test of an action on controller 1 as pressed this frame, before keep_direction, a turn's controller or a hook's held actions: the pad the original's controller routine reads",
        &["cost.terms.tests.pad", "cost.terms.per_tests.pad"],
    ),
    core(
        "entity.ext_rows",
        "",
        "An entity's `ext_rows`: tables its hook reads, each a `rows` asset slot whose rows' numbers in order are one table, so a hook can read by address a span the model lifts as rows",
        &["entities.*.ext_rows"],
    ),
    core(
        "entity.hook_first",
        "",
        "An entity whose ext hook runs at the start of the player's system, before the player moves",
        &["entities.**.hook_first"],
    ),
    core(
        "entity.hook_last",
        "",
        "An entity whose ext hook runs once at the end of every frame, after every system, frozen or not",
        &["entities.**.hook_last"],
    ),
    core(
        "entity.hook_frame_start",
        "",
        "An entity whose ext hook also runs at the start of every frame, before the first system, frozen or not",
        &["entities.**.hook_frame_start"],
    ),
    core(
        "entity.hook_through_stops",
        "",
        "An entity whose ext hook still runs on a frame a state's stops list its system",
        &["entities.**.hook_through_stops"],
    ),
    core(
        "level.tile_surfaces",
        "",
        "Tile values whose standing surface is a pixel row inside the cell, and bodies that follow it as they walk",
        &["levels.*.tile_surfaces"],
    ),
    core(
        "level.empty",
        "",
        "A level of empty cells with no grid and no layout, for a new game's skeleton before its level is lifted",
        &["levels.*.empty"],
    ),
    core(
        "level.camera.keep_in_view",
        "",
        "A level's own bounds for the followed entity in the view across, instead of the camera's",
        &["levels.*.camera.keep_in_view"],
    ),
    core(
        "level.camera.keep_in_view_rows",
        "",
        "A level's own keep_in_view bounds while the followed entity's row is in a range, as a camera's keep_in_view_rows",
        &["levels.*.camera.keep_in_view_rows"],
    ),
    core(
        "level.halfway",
        "",
        "A level's checkpoint, where restart-level starts it again once the view got there",
        &["levels.*.halfway"],
    ),
    core(
        "present.cells",
        "",
        "The pictures of cells the game changes, and a held cell's bump",
        &["present.cells"],
    ),
    core(
        "present.effects",
        "",
        "Short-lived pictures the model's events start",
        &["present.effects"],
    ),
    core(
        "clip.when_words",
        "",
        "A clip or blink condition on more than one `ext` hook word",
        &[
            "entities.**.sprite.clips.when.words",
            "entities.**.sprite.blink.when.words",
        ],
    ),
    core(
        "clip.when_counter",
        "",
        "A clip or blink condition on the model's counters in ranges",
        &[
            "entities.**.sprite.clips.when.counters",
            "entities.**.sprite.blink.when.counters",
        ],
    ),
    core(
        "clip.when_slot_counter",
        "",
        "A clip or blink condition on a counter named by the body's slot, in a range",
        &[
            "entities.**.sprite.clips.when.slot_counters",
            "entities.**.sprite.blink.when.slot_counters",
        ],
    ),
    core(
        "clip.when_x",
        "",
        "A clip, blink or behind condition on the body's x in whole pixels, in a range",
        &[
            "entities.**.sprite.clips.when.x_at_least",
            "entities.**.sprite.clips.when.x_below",
            "entities.**.sprite.blink.when.x_at_least",
            "entities.**.sprite.blink.when.x_below",
            "entities.**.sprite.behind_when.x_at_least",
            "entities.**.sprite.behind_when.x_below",
        ],
    ),
    core(
        "sprite.behind_when",
        "",
        "A sprite drawn behind the level's picture on the frames a condition holds",
        &["entities.**.sprite.behind_when"],
    ),
    core(
        "sprite.split",
        "",
        "A part of a sprite's picture held at the body's place while the rest is drawn moved by counters",
        &["entities.**.sprite.split"],
    ),
    core(
        "clip.frame_every",
        "",
        "A clip frame that shows for its own number of frames",
        &["entities.**.sprite.clips.frames.every"],
    ),
    core(
        "clip.frame_slot",
        "",
        "A clip frame read from a rows slot of its own",
        &["entities.**.sprite.clips.frames.slot"],
    ),
    core(
        "present.sprite_x_wraps",
        "",
        "Sprites left of the screen drawn at the right edge, as a console whose sprite x is a byte",
        &["present.screen.sprite_x_wraps"],
    ),
    core(
        "clip.frame_colors",
        "",
        "A clip frame drawn in colors of its own, from a palettes slot",
        &["entities.**.sprite.clips.frames.colors"],
    ),
    core(
        "sprite.name_table",
        "",
        "A sprite the game draws into the background's name table, counted as the background's",
        &["entities.**.sprite.name_table"],
    ),
    core(
        "sprite.flip_in_place",
        "",
        "A sprite mirrored where it is, as a console flips a sprite, not about its hitbox's middle",
        &["entities.**.sprite.flip_in_place"],
    ),
    core(
        "present.effects.until_counter",
        "",
        "An effect phase that ends the frame after a counter reads 0",
        &["present.effects.*.phases.*.until_counter"],
    ),
    core(
        "present.effects.until_counters",
        "",
        "An effect phase that ends the frame after the one of several counters highest as it started reads 0",
        &["present.effects.*.phases.*.until_counters"],
    ),
    core(
        "present.sound_silent_while",
        "",
        "Counter tests while which the game makes no sound though it asks for some, so a theme's sound per role is silent then too",
        &["present.sound.silent_while"],
    ),
    core(
        "present.sound_roles",
        "",
        "Sound requests named as sound roles, each cued by a counter, which a theme can replace with a file",
        &["present.sound.roles"],
    ),
    core(
        "present.effects.until_gone",
        "",
        "An effect phase that ends the frame its event's entity is gone",
        &["present.effects.*.phases.*.until_gone"],
    ),
    plat(
        "oscillate",
        "0.4.1",
        "Swinging around a center",
        &[
            "entities.*.oscillate",
            "entities.*.machine.states.*.set.oscillate",
        ],
    ),
    plat(
        "leap",
        "0.4.1",
        "Leaping again and again from a start",
        &["entities.*.leap", "entities.*.machine.states.*.set.leap"],
    ),
    plat(
        "shuttle",
        "0.4.1",
        "Back and forth by a counter, with a sway",
        &[
            "entities.*.shuttle",
            "entities.*.machine.states.*.set.shuttle",
        ],
    ),
    plat(
        "orbit",
        "0.4.1",
        "A turning pivot and the parts it carries",
        &["entities.*.orbit", "entities.*.machine.states.*.set.orbit"],
    ),
    plat(
        "orbit_part",
        "0.4.1",
        "A part an orbit carries",
        &[
            "entities.*.orbit_part",
            "entities.*.machine.states.*.set.orbit_part",
        ],
    ),
    plat(
        "leave_view",
        "0.4.1",
        "Leaving once off the view",
        &[
            "entities.*.leave_view",
            "entities.*.machine.states.*.set.leave_view",
        ],
    ),
    plat(
        "hop.pattern",
        "0.4.1",
        "Hop speeds in turn",
        &["entities.**.hop.pattern"],
    ),
    plat(
        "hop.every",
        "0.4.1",
        "A hop on a beat (`every`, `phase`)",
        &["entities.**.hop.every", "entities.**.hop.phase"],
    ),
    plat(
        "run.move_step",
        "0.4.2",
        "A run that moves by whole steps, the rest kept as `vx_frac`",
        &["entities.**.run.move_step"],
    ),
    plat(
        "run.cap_whole",
        "0.4.2",
        "A cap on the whole steps that keeps the fraction",
        &["entities.**.run.cap_whole"],
    ),
    plat(
        "run.skid_by_last_motion",
        "0.4.2",
        "`skid_by = \"last_motion\"`: the rate doubles while facing away from the last way moved",
        &["entities.**.run.skid_by=last_motion"],
    ),
    plat(
        "run.skid_turn_below",
        "0.4.2",
        "A slow skid that turns at once",
        &["entities.**.run.skid_turn_below"],
    ),
    plat(
        "run.decel_bands",
        "0.4.2",
        "Walking rates by speed",
        &["entities.**.run.decel_bands"],
    ),
    plat(
        "run.decel_hold_from",
        "0.4.2",
        "A hold of the fastest walking rate",
        &["entities.**.run.decel_hold_from"],
    ),
    plat(
        "run.run_hold_frames",
        "0.4.2",
        "Frames the run action counts after it's let go",
        &["entities.**.run.run_hold_frames"],
    ),
    plat(
        "run.run_with_motion",
        "0.4.2",
        "The run action counting only with motion",
        &["entities.**.run.run_with_motion"],
    ),
    plat(
        "max_fall_from_half",
        "0.4.3",
        "A fall limit that acts from one half past it, on `jump` and `fall`",
        &[
            "entities.**.jump.max_fall_from_half",
            "entities.**.fall.max_fall_from_half",
        ],
    ),
    plat(
        "run.takeoff_in_air",
        "0.4.3",
        "The frame a jump starts is a frame in the air",
        &["entities.**.run.takeoff_in_air"],
    ),
    plat(
        "run.face_on_ground",
        "0.4.3",
        "The facing changes only on the ground",
        &["entities.**.run.face_on_ground"],
    ),
    plat(
        "run.air_run_from",
        "0.4.3",
        "In the air the speed alone says whether the run counts",
        &["entities.**.run.air_run_from"],
    ),
    plat(
        "springboard",
        "0.4.4",
        "A body that holds a player standing on it and throws it up",
        &[
            "entities.*.springboard",
            "entities.*.machine.states.*.set.springboard",
        ],
    ),
    plat(
        "jump.no_gravity_on_ground",
        "0.4.5",
        "A frame that starts standing adds no gravity",
        &["entities.**.jump.no_gravity_on_ground"],
    ),
    plat(
        "jump.land_keeps_fraction",
        "0.4.5",
        "A landing keeps the height's fraction",
        &["entities.**.jump.land_keeps_fraction"],
    ),
    plat(
        "jump.land_depth",
        "0.4.5",
        "A landing only near a tile's top",
        &["entities.**.jump.land_depth"],
    ),
    plat(
        "jump.feet",
        "0.4.5",
        "Ground points",
        &["entities.**.jump.feet"],
    ),
    plat(
        "jump.head",
        "0.4.5",
        "A head point and its hits by tile kind",
        &["entities.**.jump.head"],
    ),
    plat(
        "run.walls",
        "0.4.5",
        "Wall points after the vertical move",
        &["entities.**.run.walls"],
    ),
    plat(
        "jump.keep_fall_gravity",
        "0.4.6",
        "A fall that isn't a jump takes the last jump's gravity",
        &["entities.**.jump.keep_fall_gravity"],
    ),
    plat(
        "jump.start_fall_gravity",
        "0.4.6",
        "The fall gravity before the first jump",
        &["entities.**.jump.start_fall_gravity"],
    ),
    plat(
        "head.hold",
        "0.4.6",
        "A head hit's cell that stays solid for some frames",
        &["entities.**.jump.head.hits.hold"],
    ),
    plat(
        "path.every",
        "0.4.7",
        "A path that moves only on some frames (`every`, `phase`)",
        &["entities.**.path.every", "entities.**.path.phase"],
    ),
    plat(
        "run.push_masks",
        "0.4.8",
        "A push that masks the next frame's held direction",
        &["entities.**.run.push_masks"],
    ),
    plat(
        "jump.collide_y",
        "0.4.8",
        "A band of heights where tiles are met",
        &["entities.**.jump.collide_y"],
    ),
    plat(
        "hop.land_first",
        "0.4.8",
        "A hop that lands before it moves",
        &["entities.**.hop.land_first"],
    ),
    plat(
        "walk.walls_at_top",
        "0.4.8",
        "A walk that meets walls at its tile hitbox's top row",
        &["entities.**.walk.walls_at_top"],
    ),
    plat(
        "contact.every",
        "0.4.9",
        "A touch tested only every few frames (`every`, `phase`)",
        &["entities.**.contact.every", "entities.**.contact.phase"],
    ),
    plat(
        "level.pit_event",
        "0.4.9",
        "A pit that fires an event instead of killing",
        &["levels.*.pit_event"],
    ),
    plat(
        "invincible",
        "0.4.10",
        "A player's invincibility while a state holds",
        &[
            "entities.*.invincible",
            "entities.*.machine.states.*.set.invincible",
        ],
    ),
    plat(
        "climb.up_cycle",
        "",
        "Whole pixels a climber moves up on each count of its step counter, shared with run.step_cycle",
        &["entities.**.climb.up_cycle"],
    ),
    plat(
        "climb.down_cycle",
        "",
        "Whole pixels a climber moves down on each count of its step counter",
        &["entities.**.climb.down_cycle"],
    ),
    plat(
        "climb.by_feet",
        "",
        "A climber held by the ladder under its feet: it stops at a broken ladder's top and stands on the surface it reaches",
        &["entities.**.climb.by_feet"],
    ),
    plat(
        "jump.ledge_fall",
        "",
        "A fall off a ledge that drops straight down at a fixed speed, and may kill on landing",
        &["entities.**.jump.ledge_fall"],
    ),
    plat(
        "jump.fatal_drop",
        "",
        "A jump that lands far enough below its takeoff kills the player",
        &["entities.**.jump.fatal_drop"],
    ),
    plat(
        "jump.fatal_drop_in_air",
        "",
        "A fatal drop measured from the height after the takeoff frame's move and tested on every frame in the air, before a landing; once it holds the takeoff word is marked and the landing kills",
        &["entities.**.jump.fatal_drop_in_air"],
    ),
    plat(
        "climb.down_through",
        "",
        "Climbing down that goes on through anything to the next surface row, its height wrapping, and starts at a ladder's foot",
        &["entities.**.climb.down_through"],
    ),
    plat(
        "climb.down_holds",
        "",
        "Down held while standing keeps a climber still instead of walking",
        &["entities.**.climb.down_holds"],
    ),
    plat(
        "power.locks",
        "",
        "A player's actions that do nothing while a counter holds",
        &["entities.*.locks"],
    ),
    plat(
        "contact.invincible",
        "0.4.10",
        "A touch that defeats while the player is invincible",
        &["entities.**.contact.invincible"],
    ),
    plat(
        "touch.defeat",
        "0.4.10",
        "The touch `defeat`: the entity leaves and `enemy.defeated` fires",
        &["entities.**.contact.otherwise=defeat"],
    ),
    plat(
        "bounce",
        "0.4.10",
        "A body that bounces off the ground",
        &[
            "entities.*.bounce",
            "entities.*.machine.states.*.set.bounce",
        ],
    ),
    plat(
        "hits.every",
        "0.4.10",
        "Hits tested only on some frames (`every`, `phase`)",
        &["entities.**.hits.every", "entities.**.hits.phase"],
    ),
    plat(
        "hits.move_first",
        "0.4.10",
        "A hitter that moves before its hits",
        &["entities.**.hits.move_first"],
    ),
    plat(
        "shoot.speed",
        "0.4.10",
        "A shot's own speeds (`vx`, `vy`)",
        &["entities.**.shoot.vx", "entities.**.shoot.vy"],
    ),
    plat(
        "shoot.unmirrored",
        "0.4.10",
        "A shot whose speed isn't turned by the shooter's facing",
        &["entities.**.shoot.unmirrored"],
    ),
    plat(
        "shoot.after_move",
        "0.4.10",
        "A shot fired after the shooter moves",
        &["entities.**.shoot.after_move"],
    ),
    plat(
        "shoot.whole",
        "0.4.10",
        "A shot placed at whole pixels",
        &["entities.**.shoot.whole"],
    ),
    plat(
        "fly.keep_speed",
        "0.4.10",
        "A flier that keeps its speed and may fall",
        &["entities.**.fly.keep_speed"],
    ),
    plat(
        "leave_view.far_margin",
        "0.4.10",
        "A far edge a body leaves the view at",
        &["entities.**.leave_view.far_margin"],
    ),
    plat(
        "path.wait_near_wrap",
        "0.4.10",
        "A path's wait near, measured across the level's wrap",
        &["entities.**.path.wait_near_wrap"],
    ),
    plat(
        "run.skid_turn_with",
        "0.4.11",
        "Actions that steer for the quick turn as a direction does",
        &["entities.**.run.skid_turn_with"],
    ),
    plat(
        "run.speed_before_walls",
        "0.4.11",
        "Speed tests that read the speed before a wall stopped the run",
        &["entities.**.run.speed_before_walls"],
    ),
    plat(
        "shoot.in_slots",
        "0.4.12",
        "Shots in numbered slots taken in turn",
        &["entities.**.shoot.in_slots"],
    ),
    plat(
        "goal.under",
        "0.4.13",
        "The height a player passes under a goal at",
        &["levels.*.goal.under"],
    ),
    plat(
        "contact.touch_edges",
        "0.4.14",
        "A touch that counts hitboxes meeting at their edges",
        &["entities.**.contact.touch_edges"],
    ),
    plat(
        "run.face_both",
        "0.4.15",
        "Both directions held as a facing of their own",
        &["entities.**.run.face_both"],
    ),
    plat(
        "run.off",
        "0.4.16",
        "A run a machine state turns off",
        &["entities.**.run.off"],
    ),
    plat(
        "leap.move_first",
        "0.4.17",
        "A leap that moves before gravity",
        &["entities.**.leap.move_first"],
    ),
    plat(
        "leap.max_fall_from_half",
        "0.4.17",
        "A leap's half-step fall limit",
        &["entities.**.leap.max_fall_from_half"],
    ),
    plat(
        "leap.beat",
        "0.4.17",
        "A leap's wait on a shared beat",
        &["entities.**.leap.beat"],
    ),
    plat(
        "leap.keep_fraction",
        "0.4.17",
        "A leap that keeps its height's fraction",
        &["entities.**.leap.keep_fraction"],
    ),
    plat(
        "jump.pickup",
        "0.4.18",
        "Points that take a tile by touching it",
        &["entities.**.jump.pickup"],
    ),
    plat(
        "fall.feet",
        "0.4.19",
        "A fall's ground columns",
        &["entities.**.fall.feet"],
    ),
    plat(
        "fall.stands_still",
        "0.4.19",
        "A body on the ground that only looks for it",
        &["entities.**.fall.stands_still"],
    ),
    plat(
        "orbit.place_on_appear",
        "0.4.20",
        "A pivot placed on the frame it appears",
        &["entities.**.orbit.place_on_appear"],
    ),
    plat(
        "contact.sides",
        "0.4.22",
        "An entity's sides that stop the player as a wall does",
        &["entities.**.contact.sides"],
    ),
    plat(
        "oscillate.starts_at_rest",
        "0.4.23",
        "A swing that takes no speed from its numbered slot",
        &["entities.**.oscillate.starts_at_rest"],
    ),
    plat(
        "oscillate.move_first",
        "0.4.23",
        "A swing that moves by its old speed before the force",
        &["entities.**.oscillate.move_first"],
    ),
    plat(
        "leave_view.bottom_margin",
        "0.4.24",
        "A bottom edge a body leaves at",
        &["entities.**.leave_view.bottom_margin"],
    ),
    plat(
        "contact.in_turn",
        "0.4.25",
        "A touch's `stops_at_once` that stops the enemies from the toucher's slot on",
        &["entities.**.contact.in_turn"],
    ),
    plat(
        "health.defeat",
        "0.4.25",
        "A defeat that leaves a body of `becomes` in the defeated one's place",
        &["entities.**.health.defeat"],
    ),
    plat(
        "leap.place_on_appear",
        "0.4.26",
        "A leap placed on the frame it appears",
        &["entities.**.leap.place_on_appear"],
    ),
    plat(
        "hop.land_while_stopped",
        "0.4.26",
        "A `land_first` hopper that lands while the enemies are stopped",
        &["entities.**.hop.land_while_stopped"],
    ),
    plat(
        "level.hard_spawns",
        "0.4.26",
        "A level's spawns for the harder loop",
        &["levels.*.hard_spawns"],
    ),
    plat(
        "walk.turn_after_move",
        "0.4.26",
        "A walker that moves into a wall and turns there",
        &["entities.**.walk.turn_after_move"],
    ),
    plat(
        "leap.draw",
        "0.4.26",
        "A leap whose speed and wait come from a counter's byte",
        &["entities.**.leap.draw"],
    ),
    plat(
        "contact.ride",
        "0.4.26",
        "A rider placed after the entity's move, with a second box below (`copy_below`, `wrap`, `above`)",
        &["entities.**.contact.ride"],
    ),
    plat(
        "shoot.vx_both",
        "0.4.27",
        "A shot's signed speed across from a shooter facing both ways",
        &["entities.**.shoot.vx_both"],
    ),
    plat(
        "stream",
        "0.4.27",
        "A level marker that sends an entity in past the view's last column",
        &[
            "entities.*.stream",
            "entities.*.machine.states.*.set.stream",
        ],
    ),
    plat(
        "contact.near",
        "0.4.27",
        "A touch tested by points near the entity (`within`, `right`)",
        &["entities.**.contact.near"],
    ),
    plat(
        "touch_points",
        "0.4.27",
        "The player's touch points, tested after the entity moves",
        &[
            "entities.*.touch_points",
            "entities.*.machine.states.*.set.touch_points",
        ],
    ),
    plat(
        "orbit.angle_in_slot",
        "0.4.27",
        "A pivot's angle kept in its slot",
        &["entities.**.orbit.angle_in_slot"],
    ),
    plat(
        "launcher",
        "0.4.28",
        "An object kind that fires an entity of its own on a timer",
        &[
            "entities.*.launcher",
            "entities.*.machine.states.*.set.launcher",
        ],
    ),
    plat(
        "walk.ledge_at",
        "0.4.29",
        "With `turn_at_ledges`, the ledge looked for after the move at this many pixels across",
        &["entities.**.walk.ledge_at"],
    ),
    plat(
        "walk.bump",
        "0.4.29",
        "Walkers whose hitboxes touch turn each other once",
        &["entities.**.walk.bump"],
    ),
    plat(
        "oscillate.rest_creep_every",
        "0.4.29",
        "A swing at rest that creeps on a beat",
        &["entities.**.oscillate.rest_creep_every"],
    ),
    plat(
        "contact.after_move",
        "0.4.29",
        "A touch tested after the entity's move in its own turn",
        &["entities.**.contact.after_move"],
    ),
    plat(
        "head.busy_while",
        "",
        "A head that hits every block as solid while a counter condition holds",
        &["entities.**.jump.head.busy_while"],
    ),
    plat(
        "head.above",
        "",
        "A head hit that changes the cell above the one hit and fires an event",
        &["entities.**.jump.head.hits.above"],
    ),
    plat(
        "run.step_cycle",
        "",
        "Whole-pixel walking steps by a per-body step counter, which the air and a landing hold also advance",
        &["entities.**.run.step_cycle"],
    ),
    plat(
        "climb.grab",
        "",
        "A climb that starts where a box from the climber meets one of the level's ladder boxes",
        &["entities.**.climb.grab", "entities.**.climb.grab_snap"],
    ),
    plat(
        "climb.lone_way",
        "",
        "Up or down starts or moves a climb only held alone, and on a ladder left or right held alone turns the climber",
        &["entities.**.climb.lone_way"],
    ),
    plat(
        "run.both_left",
        "",
        "Left and right held together turn the entity left and move it nowhere",
        &["entities.**.run.both_left"],
    ),
    plat(
        "climb.down_through_step",
        "",
        "A climb down through the floor only while the last move across stepped up a surface",
        &["entities.**.climb.down_through_step"],
    ),
    plat(
        "climb.top_test",
        "",
        "A climb from standing that asks the level's ladder tops and stops by the grab box, and a climb up that stops at a stop",
        &["entities.**.climb.top_test"],
    ),
    plat(
        "level.ladders",
        "",
        "A level's ladders as boxes, read from rows of bytes",
        &["levels.*.ladders"],
    ),
    plat(
        "level.ladder_tops",
        "",
        "A level's ladder tops as boxes, read as its ladders are, for a climber's top test",
        &["levels.*.ladder_tops"],
    ),
    plat(
        "level.ladder_stops",
        "",
        "A level's boxes where a climb is refused, read as its ladders are, for a climber's top test",
        &["levels.*.ladder_stops"],
    ),
    plat(
        "level.ladder_lists",
        "",
        "Ladder boxes read from a game's own list in a byte table: rows of x, y and a shape from an index up to an end mark, and the shapes from another index",
        &[
            "levels.*.ladders.places_at",
            "levels.*.ladders.shapes_at",
            "levels.*.ladders.end",
            "levels.*.ladder_tops.places_at",
            "levels.*.ladder_tops.shapes_at",
            "levels.*.ladder_tops.end",
            "levels.*.ladder_stops.places_at",
            "levels.*.ladder_stops.shapes_at",
            "levels.*.ladder_stops.end",
        ],
    ),
    plat(
        "level.ground",
        "",
        "A level's ground as one-row boxes read from runs of bytes, followed by a body's feet in place of the tile surfaces",
        &["levels.*.ground"],
    ),
    plat(
        "level.ground_land",
        "",
        "A jump (or a ledge fall, where it is) that lands on the level's ground boxes its feet meet within a few pixels above them, in the boxes' order, in place of the tiles",
        &["levels.*.ground.land"],
    ),
    plat(
        "level.pickups",
        "",
        "A level's groups of places read from a byte table that the player takes by touching them, on a frame it moved across or is in a jump, each kept in a counter, with an event",
        &["levels.*.pickups"],
    ),
    plat(
        "level.pickup_holes",
        "",
        "A taken pickup place is a hole: touching it out of a jump starts the player's fall off a ledge, which passes the ground box under it",
        &["levels.*.pickups.hole"],
    ),
    plat(
        "level.pickup_cells",
        "",
        "The level's cells a pickup place covers, from its byte table, which show an empty cell's picture once it is taken",
        &["levels.*.pickups.cells"],
    ),
    plat(
        "level.pickup_by_hook",
        "",
        "A pickup group whose places an ext hook takes in place of the engine, which gives its cells and counters",
        &["levels.*.pickups.by_hook"],
    ),
    plat(
        "level.ground_boxes",
        "",
        "A level's ground runs read as one box a row (x, y and a shape), with no step across or count",
        &["levels.*.ground.boxes"],
    ),
    plat(
        "level.ground_bodies",
        "",
        "Ground that moves: entities whose box is ground after the level's boxes, which a body standing on them rides, moved by their every step until a ground test meets no box",
        &["levels.*.ground.bodies"],
    ),
    plat(
        "level.ground_land_rising",
        "",
        "A jump that lands on a ground box its feet meet on any frame after its takeoff, rising too",
        &["levels.*.ground.land_rising"],
    ),
    plat(
        "run.air_step_cycle",
        "",
        "Whole-pixel steps across in the air, the way the body took off, by a counter of its own",
        &["entities.**.run.air_step_cycle"],
    ),
    plat(
        "run.lone_direction",
        "",
        "Left or right walks only held alone, with neither of the climb's up and down actions held",
        &["entities.**.run.lone_direction"],
    ),
    plat(
        "run.face_way",
        "",
        "The facing is a way kept from the pad: a takeoff turns it to a lone direction held, a landing hold and a level's restart keep it, and a turn at the view's edge in the air turns it",
        &["entities.**.run.face_way"],
    ),
    plat(
        "jump.takeoff_fraction",
        "",
        "The height's fraction a takeoff sets",
        &["entities.**.jump.takeoff_fraction"],
    ),
    plat(
        "jump.land_hold",
        "",
        "Frames after a jump's landing in which the body neither walks, jumps nor takes a ladder",
        &["entities.**.jump.land_hold"],
    ),
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use super::FEATURES;
    use crate::Model;
    use crate::features::{Piece, is_name};
    use crate::validate::version_at_least;

    #[test]
    fn names_are_unique_and_well_formed() {
        let mut seen = BTreeSet::new();
        for f in FEATURES {
            assert!(is_name(f.name), "{}", f.name);
            assert!(seen.insert(f.name), "{} is listed twice", f.name);
            assert!(!f.paths.is_empty(), "{} has no paths", f.name);
        }
    }

    fn properties(v: &serde_json::Value, out: &mut BTreeSet<String>) {
        match v {
            serde_json::Value::Object(o) => {
                if let Some(serde_json::Value::Object(p)) = o.get("properties") {
                    out.extend(p.keys().cloned());
                }
                for x in o.values() {
                    properties(x, out);
                }
            }
            serde_json::Value::Array(a) => a.iter().for_each(|x| properties(x, out)),
            _ => {}
        }
    }

    #[test]
    fn every_path_ends_in_a_field_of_the_schema() {
        let schema: serde_json::Value =
            serde_json::from_str(&crate::schema::model_platformer()).unwrap();
        let mut props = BTreeSet::new();
        properties(&schema, &mut props);
        for f in FEATURES {
            for p in f.paths {
                let last = p.split('.').rfind(|s| *s != "*" && *s != "**").unwrap();
                let key = last.split_once('=').map_or(last, |(k, _)| k);
                assert!(
                    props.contains(key),
                    "{}: `{key}` of {p} isn't a field",
                    f.name
                );
            }
        }
    }

    #[test]
    fn a_small_model_uses_no_features() {
        let m = Model::parse(crate::tests::SMALL).unwrap();
        assert!(m.features_used().is_empty(), "{:?}", m.features_used());
    }

    #[test]
    fn templates_derive_their_features() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates");
        let mut read = 0;
        for d in std::fs::read_dir(&root).unwrap().flatten() {
            let file = d.path().join(crate::MODEL_FILE);
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            let m = Model::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
            let used = m.features_used();
            read += 1;
            // The old validator gated most core options on `model.schema`,
            // so a template that names a patch uses no core feature newer
            // than it. A few core options (all presentation from 0.4.1 to
            // 0.4.3, the head hit's block kinds' paths, the counter shift
            // and the entity's despawn_ahead and extra_slots) were never
            // gated, so they're left out of this check.
            let ungated = [
                "sprite.cells",
                "sprite.colors",
                "sprite.clips",
                "level.label",
                "present.cycles",
                "present.screen",
                "present.hud",
                "asset.font",
                "asset.screen_text",
                "role.hud",
                "asset.sprite_set",
                "clip.hold",
                "sprite.blink",
                "clip.when_jumped",
                "clip.when_invulnerable",
                "clip.when_climbing",
                "asset.songs",
                "asset.sound_effects",
                "slot.capture",
                "present.sound",
                "counter.shift",
                "sprite.copy",
                "entity.despawn_ahead",
                "entity.extra_slots",
            ];
            let schema = &m.model.schema;
            if schema.split('.').count() == 3 {
                for f in FEATURES.iter().filter(|f| {
                    f.piece == Piece::Core && used.contains(f.name) && !ungated.contains(&f.name)
                }) {
                    assert!(
                        version_at_least(schema, f.since),
                        "{}: uses {} (core {}) and names core {schema}",
                        file.display(),
                        f.name,
                        f.since
                    );
                }
            }
        }
        assert!(read > 0, "no templates under {}", root.display());
    }
}
