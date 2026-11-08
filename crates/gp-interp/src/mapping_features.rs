//! The features of the state mapping format this build knows
//! ([`gp_model::features`], docs/formats/state-mapping.md). Each optional
//! part added by schema steps 2 to 19 is one, with its schema kept as a
//! note.
//!
//! To add a part: add its row here, with a new name and `since = ""`, and
//! don't bump [`crate::mapping::SCHEMA`]. A breaking change bumps it
//! instead, once per release.

use gp_model::features::{Feature, Piece};

const fn mapping(
    name: &'static str,
    since: &'static str,
    about: &'static str,
    paths: &'static [&'static str],
) -> Feature {
    Feature {
        name,
        piece: Piece::Mapping,
        since,
        about,
        paths,
    }
}

/// Every feature of the state mapping, in the order they appeared.
pub const FEATURES: &[Feature] = &[
    mapping(
        "ticks",
        "schema 2",
        "`ticks.<counter>[i]` paths, the count toward a counter's `every` change. A field's path is a value with a name in it, which feature paths can't match, so this one is found only by being listed",
        &[],
    ),
    mapping(
        "field.case",
        "schema 3",
        "A field's cases: values that stand for a model state, or a start that only approximates it",
        &["field.case", "slots.field.case", "slots.group.field.case"],
    ),
    mapping(
        "field.case_flag",
        "",
        "A flag's case that gives it `true` or `false`, for a flag the bytes alone don't say. Found only by being listed",
        &[],
    ),
    mapping(
        "entity.jumping",
        "",
        "`entities.<kind>[i].jumping` as a loadable path for a kind with a jump: in a jump's air, so a start inside a jump goes on along its arc. Found only by being listed, as `entity.time` is",
        &[],
    ),
    mapping(
        "entity.time",
        "schema 3",
        "`entities.<kind>[i].time` as a loadable path. Found only by being listed, as `ticks` is",
        &[],
    ),
    mapping(
        "field.presentation",
        "schema 4",
        "A field's `presentation` flag: loaded, but not compared",
        &[
            "field.presentation",
            "slots.field.presentation",
            "slots.group.field.presentation",
        ],
    ),
    mapping(
        "slot.body_fields",
        "schema 5",
        "The slot fields `facing`, `state`, `home_x`, `home_y`, `path_back` and `wait`, loaded into a carried body",
        &[
            "slots.field.path=facing",
            "slots.field.path=state",
            "slots.field.path=home_x",
            "slots.field.path=home_y",
            "slots.field.path=path_back",
            "slots.field.path=wait",
            "slots.group.field.path=facing",
            "slots.group.field.path=state",
            "slots.group.field.path=home_x",
            "slots.group.field.path=home_y",
            "slots.group.field.path=path_back",
            "slots.group.field.path=wait",
        ],
    ),
    mapping(
        "field.whole",
        "schema 5",
        "`whole` with `frac`: the fraction loaded, whole parts compared",
        &[
            "field.whole",
            "slots.field.whole",
            "slots.group.field.whole",
        ],
    ),
    mapping(
        "entity.mechanic_paths",
        "schema 5",
        "An entity's `ext[0]` to `ext[3]`, `home_x`, `home_y`, `path_back` and `wait` as loadable paths. Found only by being listed, as `ticks` is",
        &[],
    ),
    mapping(
        "present.animation",
        "schema 6",
        "`present.<kind>[i].frame` and `.timer`, a body's animation, loaded into the screen measure's presentation. Found only by being listed, as `ticks` is",
        &[],
    ),
    mapping(
        "slots.rest",
        "schema 7",
        "The slots' `rest`, what an empty slot's last object left",
        &["slots.rest"],
    ),
    mapping(
        "camera.last_move",
        "schema 8",
        "`cameras.<name>.move_x` and `.move_y`, a camera's followed entity's last move. Found only by being listed, as `ticks` is",
        &[],
    ),
    mapping(
        "slots.group",
        "schema 9",
        "More object slots of their own, such as a game's projectiles",
        &["slots.group"],
    ),
    mapping(
        "slot_kind.up_down",
        "schema 9",
        "A slot kind's direction up and down as a measure",
        &["slots.kind.up_down", "slots.group.kind.up_down"],
    ),
    mapping(
        "input.held",
        "schema 9",
        "`input.<action>`, the buttons held on the start's frame. Found only by being listed, as `ticks` is",
        &[],
    ),
    mapping(
        "entity.seen_speed",
        "schema 10",
        "`entities.<kind>[i].seen_speed`, the speed the run last left before a wall. Found only by being listed, as `ticks` is",
        &[],
    ),
    mapping(
        "group.rest",
        "schema 11",
        "A slot group's `rest` and `turn`: a projectile slot's leftovers and whose turn it is",
        &["slots.group.rest", "slots.group.turn"],
    ),
    mapping(
        "facing.both",
        "schema 12",
        "The value `both` of an entity's `facing` and `moved`",
        &[
            "field.values=both",
            "slots.field.values=both",
            "slots.group.field.values=both",
        ],
    ),
    mapping(
        "field.shared",
        "schema 13",
        "A slot field's `shared`: bytes at one address for every slot",
        &[
            "slots.field.shared",
            "slots.group.field.shared",
            "field.shared",
        ],
    ),
    mapping(
        "cells",
        "schema 14",
        "Where the original keeps its level grid, so a translation knows which cells have been taken",
        &["cells"],
    ),
    mapping(
        "rest.y_frac",
        "schema 15",
        "A slot rest's `y_frac`, the fraction of the height a slot's last object left",
        &["slots.rest.path=y_frac"],
    ),
    mapping(
        "slots.held",
        "schema 15",
        "The slots' `held`: an object that keeps its slot after its life ends",
        &["slots.held", "slots.group.held"],
    ),
    mapping(
        "pending",
        "schema 16",
        "`[[pending]]`: changes to a counter the original holds back, loaded as the model's waiting changes",
        &["pending"],
    ),
    mapping(
        "slot.time_hp",
        "schema 17",
        "The slot fields `time` and `hp`",
        &[
            "slots.field.path=time",
            "slots.field.path=hp",
            "slots.group.field.path=time",
            "slots.group.field.path=hp",
        ],
    ),
    mapping(
        "field.names",
        "schema 17",
        "A field's `names`: values by number",
        &[
            "field.names",
            "slots.field.names",
            "slots.group.field.names",
        ],
    ),
    mapping(
        "slot_kind.refuse",
        "schema 17",
        "A slot kind's `refuse`: why a start can't carry it",
        &["slots.kind.refuse", "slots.group.kind.refuse"],
    ),
    mapping(
        "slot_kind.leave",
        "schema 17",
        "A slot kind's `leave`: why a start leaves it out on purpose",
        &["slots.kind.leave", "slots.group.kind.leave"],
    ),
    mapping(
        "field.wide_whole",
        "schema 18",
        "A plain whole number of 6 or 7 addresses, read as an integer. Found only by being listed: feature paths can't count a field's addresses",
        &[],
    ),
    mapping(
        "slot_kind.holds_slot",
        "schema 19",
        "A slot kind's `holds_slot`: a start carries the object and no run pairs or measures it",
        &["slots.kind.holds_slot", "slots.group.kind.holds_slot"],
    ),
    mapping(
        "slots.part_flag",
        "schema 19",
        "The flag bit of a slot whose ID means nothing",
        &["slots.part_flag", "slots.group.part_flag"],
    ),
    mapping(
        "ignore",
        "schema 21",
        "The `[[ignore]]` list: RAM bytes the model needs no field for, each with its role and why",
        &["ignore"],
    ),
    mapping(
        "slot.rest_word",
        "schema 22",
        "A slot rest's `word`, the slot's own word, read for a taken slot too",
        &["slots.rest.path=word", "slots.group.rest.path=word"],
    ),
    mapping(
        "cells.becomes",
        "",
        "The kind each byte of the level grid gives a cell of `[cells]`'s kinds: a used block's byte is a solid cell",
        &["cells.becomes"],
    ),
    mapping(
        "slots.field.stride",
        "",
        "A slot field's `stride`, its bytes from one slot to the next, for a game that keeps an object's values a record apart, as a sprite buffer does",
        &["slots.field.stride", "slots.group.field.stride"],
    ),
    mapping(
        "slots.no_id",
        "",
        "Slots with no `id`: every slot's ID is 0, for slots that hold one kind of object. Found only by being listed, as `ticks` is",
        &[],
    ),
    mapping(
        "slots.no_clock",
        "",
        "Slots with no `clock`: their bodies step once a frame, and a start leaves the model's frame count as it is, for a game that keeps no frame counter. Found only by being listed, as `ticks` is",
        &[],
    ),
    mapping(
        "slots.active_stride",
        "",
        "The slots' flag bytes a stride apart, as a slot field's",
        &["slots.active_stride", "slots.group.active_stride"],
    ),
    mapping(
        "slots.field.index",
        "",
        "A slot field whose value is the slot's own number",
        &[
            "slots.field.index",
            "slots.group.field.index",
            "slots.kind.field.index",
            "slots.group.kind.field.index",
        ],
    ),
    mapping(
        "slots.free",
        "",
        "The flag's value for an empty slot, instead of 0",
        &["slots.free", "slots.group.free"],
    ),
    mapping(
        "slots.vacant",
        "",
        "A condition under which the original's slots hold none of their objects and every slot reads free, for a scene the game draws in the slots' bytes; a group without one takes its slots'",
        &["slots.vacant", "slots.group.vacant"],
    ),
    mapping(
        "slots.vacant_from",
        "",
        "A condition under which the slots from a given one on read free, for a stage that uses fewer of a group's slots and the rest's bytes for something else",
        &["slots.vacant_from", "slots.group.vacant_from"],
    ),
    mapping(
        "draws",
        "",
        "`[[draws]]`, the original's random draw sites: the code address that reads a random byte, the byte, and the site's name, from which `gp interp draws` records a clip's random draws script",
        &["draws"],
    ),
    mapping(
        "draws.soft",
        "",
        "A `[[draws]]` site's `soft`: a byte read only for presentation, such as a sound driver's random pitch, whose draws the rebuild takes when it asks on the same frame and which never desync the script",
        &["draws.soft"],
    ),
    mapping(
        "draws.score",
        "",
        "`score_draws`: a clip's score gives the rebuild the original's random draws over the clip, recorded at the `[[draws]]` sites, as `gp interp compare --draws` does",
        &["score_draws"],
    ),
    mapping(
        "stretches.play",
        "",
        "`[stretches] play`: a condition over the original's state outside which a frame isn't play, so it ends a long run's exact stretch and isn't counted",
        &["stretches.play"],
    ),
    mapping(
        "stretches.state",
        "",
        "`[[stretches.state]]`: the original's states by name and condition, by which a lockstep lists its splits",
        &["stretches.state"],
    ),
    mapping(
        "scope.while",
        "",
        "`[scope] while`: a condition over the original's state that holds while a run is in the template's scope; a lockstep stops at the first frame it doesn't and counts the frames from there as out of scope",
        &["scope.while"],
    ),
    mapping(
        "scope.leaves",
        "",
        "`[scope] leaves`: selectors over the original's state, such as a stage byte; a run is out of the template's scope from the first frame one leaves the value it had on the first compared frame",
        &["scope.leaves"],
    ),
    mapping(
        "field.bcd",
        "",
        "A field's `bcd`: bytes of two decimal digits each, most significant first",
        &["field.bcd", "slots.field.bcd"],
    ),
    mapping(
        "field.case.unmeasured",
        "",
        "A case under which a field isn't compared: it means nothing then, though a translation loads it",
        &[
            "field.case.unmeasured",
            "slots.field.case.unmeasured",
            "slots.group.field.case.unmeasured",
        ],
    ),
    mapping(
        "slots.resync",
        "",
        "A resync carries the original's slot objects into the model as a start does",
        &["slots.resync"],
    ),
    mapping(
        "field.lookup",
        "",
        "A field's `lookup`: the model's number for each number the bytes hold",
        &[
            "field.lookup",
            "slots.field.lookup",
            "field.lookup_else",
            "slots.field.lookup_else",
        ],
    ),
    mapping(
        "field.case.ram",
        "",
        "A case's `ram`: other addresses the field reads while the case holds, for a value a game keeps once per player",
        &[
            "field.case.ram",
            "slots.field.case.ram",
            "slots.group.field.case.ram",
        ],
    ),
    mapping(
        "field.case.oam",
        "",
        "A case's `oam`: other sprite table bytes a field that reads OAM reads while the case holds, for a game that sends its sprites in another order on one stage",
        &["field.case.oam"],
    ),
    mapping(
        "field.case.nametable",
        "",
        "A case's `nametable`: other PPU addresses a field that reads the name tables reads while the case holds, for a game that draws a picture at another place on one stage",
        &["field.case.nametable"],
    ),
    mapping(
        "field.case.number",
        "",
        "A case that gives a number field a value of its own while it holds",
        &[
            "field.case.number",
            "slots.field.case.number",
            "slots.group.field.case.number",
        ],
    ),
    mapping(
        "field.whole_64",
        "",
        "A plain whole number of 8 addresses, all 64 bits of the model's number, as a 64-bit random register. Found only by being listed: feature paths can't count a field's addresses",
        &[],
    ),
    mapping(
        "align.read_at_nmi",
        "",
        "`read_at = \"nmi\"`: the original's RAM is read as the CPU takes the NMI after the frame's end, so a frame whose NMI handler runs past the end is read whole",
        &["read_at"],
    ),
    mapping(
        "field.nametable",
        "",
        "A field's `nametable`: name table cells by PPU address, read through the cartridge's mirroring, for a value a game keeps only on the screen",
        &["field.nametable"],
    ),
    mapping(
        "field.oam",
        "",
        "A field's `oam`: sprite memory bytes by index, the sprite table the last DMA sent",
        &["field.oam"],
    ),
    mapping(
        "field.oam_tile",
        "",
        "A field's `oam_tile`: a byte of the first sprite in OAM whose tile is one of a list, or a number of its own when none is",
        &["field.oam_tile"],
    ),
    mapping(
        "field.machine",
        "",
        "A field's `machine`: a selector of the original's runtime protocol whose number is the field's, for state the console keeps outside memory, such as gp-nes's `nmi.` selectors (where the code the next NMI stops goes on, its entry's CPU cycle and PPU dot)",
        &["field.machine"],
    ),
    mapping(
        "slots.group.name",
        "",
        "A slot group's name, by which a derived template's delta merges its groups over its base's",
        &["slots.group.name"],
    ),
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use super::FEATURES;
    use crate::mapping::Mapping;
    use gp_model::features::{is_name, used};

    #[test]
    fn names_are_unique_well_formed_and_not_the_models() {
        let mut seen = BTreeSet::new();
        for f in FEATURES {
            assert!(is_name(f.name), "{}", f.name);
            assert!(seen.insert(f.name), "{} is listed twice", f.name);
            assert!(
                !gp_model::feature_list::FEATURES
                    .iter()
                    .any(|m| m.name == f.name),
                "{} is a model feature's name too",
                f.name
            );
        }
    }

    #[test]
    fn templates_parse_and_derive_their_features() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates");
        let mut read = 0;
        for d in std::fs::read_dir(&root).unwrap().flatten() {
            let file = d.path().join(crate::mapping::MAPPING_FILE);
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            Mapping::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
            let tree = toml::Value::Table(text.parse().unwrap());
            let _ = used(&tree, FEATURES);
            read += 1;
        }
        assert!(read > 0, "no mappings under {}", root.display());
    }

    const BASE: &str = "schema = 20\nmodel = \"m\"\nsource = \"m\"\nalign = \"polled\"\n\n[[field]]\npath = \"entities.player[0].state\"\nram = [0x10]\nvalues = [\"small\", \"big\"]\n";

    #[test]
    fn a_bcd_field_reads_two_digits_a_byte() {
        let t = format!(
            "{BASE}\n[[field]]\npath = \"counters.score\"\nram = [0x25, 0x26, 0x27]\nbcd = true\n\n[[field]]\npath = \"counters.bonus\"\nram = [0x2E]\nbcd = true\nscale = 100\n"
        );
        let m = Mapping::parse(&t).unwrap();
        assert!(m.features.iter().any(|f| f == "field.bcd") || m.features.is_empty());
        let score = m
            .fields
            .iter()
            .find(|f| f.path == "counters.score")
            .unwrap();
        assert_eq!(
            score.value(&[0x01, 0x35, 0x00]),
            gp_runtime::Value::Fixed(13500 << 16)
        );
        let bonus = m
            .fields
            .iter()
            .find(|f| f.path == "counters.bonus")
            .unwrap();
        assert_eq!(bonus.value(&[0x47]), gp_runtime::Value::Fixed(4700 << 16));
        let looked = format!(
            "{BASE}\n[[field]]\npath = \"counters.wait\"\nram = [0x43]\nlookup = [[135, 32], [121, 19]]\n"
        );
        let m2 = Mapping::parse(&looked).unwrap();
        let wait = m2
            .fields
            .iter()
            .find(|f| f.path == "counters.wait")
            .unwrap();
        assert_eq!(wait.value(&[135]), gp_runtime::Value::Fixed(32 << 16));
        assert_eq!(wait.value(&[121]), gp_runtime::Value::Fixed(19 << 16));
        assert_eq!(
            wait.value(&[7]),
            gp_runtime::Value::Fixed(7 << 16),
            "unlisted: as it is"
        );
        let m3 =
            Mapping::parse(&looked.replace("[121, 19]]", "[121, 19]]\nlookup_else = 0")).unwrap();
        let wait = m3
            .fields
            .iter()
            .find(|f| f.path == "counters.wait")
            .unwrap();
        assert_eq!(
            wait.value(&[7]),
            gp_runtime::Value::Fixed(0),
            "unlisted: lookup_else"
        );
        let bad = t.replace("bcd = true\nscale", "bcd = true\ndigits = true\nscale");
        assert!(Mapping::parse(&bad).unwrap_err().contains("bcd"));
    }

    #[test]
    fn a_mapping_listing_an_unknown_feature_is_refused_by_name() {
        let t = BASE.replace("align", "features = [\"rope.swing\"]\nalign");
        let e = Mapping::parse(&t).unwrap_err();
        assert!(e.contains("`rope.swing`"), "{e}");
        assert!(e.contains("doesn't know"), "{e}");
    }

    #[test]
    fn a_mapping_listing_a_known_feature_is_accepted_and_must_list_what_it_uses() {
        let t = BASE.replace("align", "features = [\"ticks\"]\nalign");
        assert_eq!(Mapping::parse(&t).unwrap().features, ["ticks"]);
        let case = "[[field.case]]\nwhen = \"ram[0x0010] == 1\"\nvalue = \"big\"\n";
        // Without a list, a mapping's features are derived; with one, the
        // ones it uses must be on it.
        let derived = format!("{BASE}{case}");
        Mapping::parse(&derived).unwrap();
        let e = Mapping::parse(&format!("{t}{case}")).unwrap_err();
        assert!(e.contains("`field.case`"), "{e}");
        let listed = format!("{t}{case}").replace("[\"ticks\"]", "[\"ticks\", \"field.case\"]");
        Mapping::parse(&listed).unwrap();
    }
}
