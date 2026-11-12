//! The detectors on made-up runs: no game's data.

use super::*;

fn frame(i: u64) -> FrameFacts {
    FrameFacts {
        frame: i,
        play: true,
        mirroring: 0,
        counters: vec![0],
        ents: vec![Ent {
            kind: Some(0),
            player: true,
            slot: None,
            id: None,
            x: 100,
            y: 100,
        }],
        code: Some(Code::default()),
        ..FrameFacts::default()
    }
}

fn run(n: u64) -> Run {
    Run {
        frames: (0..n).map(frame).collect(),
        kinds: vec!["hero".into(), "walker".into()],
        counters: vec!["coins".into()],
        known: Known {
            oam_page: 2,
            ..Known::default()
        },
        ..Run::default()
    }
}

fn cell(cx: u8, cy: u8, before: u8, after: u8) -> CellChange {
    CellChange {
        nt: 0,
        cx,
        cy,
        before: [before; 4],
        after: [after; 4],
        writers: vec![],
    }
}

#[test]
fn cells_map_to_the_screen_by_mirroring() {
    // Vertical mirroring: physical table 1 is the right one.
    assert_eq!(cell_on_screen(1, 0, 2, 0, [256, 0]), Some((0, 32)));
    assert_eq!(cell_on_screen(1, 0, 2, 0, [0, 0]), None);
    // Horizontal mirroring: physical table 1 is the lower one.
    assert_eq!(cell_on_screen(1, 3, 0, 1, [0, 240]), Some((48, 0)));
    // Scrolled half a cell: the cell at the left edge isn't wholly shown.
    assert_eq!(cell_on_screen(0, 0, 0, 0, [8, 0]), None);
}

#[test]
fn runtime_cells_leave_out_redraws_streams_and_the_hud() {
    let mut r = run(10);
    r.known.hud_rows = 4;
    r.frames[2].cells.push(cell(3, 6, 1, 2));
    // The HUD's rows.
    r.frames[3].cells.push(cell(3, 1, 1, 2));
    // A column streamed in.
    r.frames[4]
        .cells
        .extend((0..8).map(|cy| cell(5, cy + 2, 1, 2)));
    // A redraw.
    r.frames[5]
        .cells
        .extend((0..15).flat_map(|cy| (0..4).map(move |cx| cell(cx, cy, 1, 2))));
    // Not in play.
    r.frames[6].play = false;
    r.frames[6].cells.push(cell(7, 7, 1, 2));
    let c = changes::runtime_cells(&r);
    assert_eq!(c.len(), 1);
    assert_eq!((c[0].frame, c[0].sx, c[0].sy), (2, 48, 96));
}

#[test]
fn unexplained_ram_is_what_neither_the_mapping_nor_a_cited_label_names() {
    let mut r = run(20);
    r.known.mapped.insert(0x10);
    r.known.ram = vec![
        RamLabel {
            name: "Cited".into(),
            addr: 0x20,
            size: 1,
        },
        RamLabel {
            name: "Timer".into(),
            addr: 0x30,
            size: 1,
        },
    ];
    r.known.cited.insert("Cited".into());
    for (i, f) in r.frames.iter_mut().enumerate() {
        f.ram_changed = vec![0x10, 0x20, 0x30];
        if i % 2 == 0 {
            // Past the label's own size: grouped under it.
            f.ram_changed.push(0x31);
        }
        // The stack and the sprite page are never reported.
        f.ram_changed.extend([0x1F0, 0x204]);
    }
    let rep = changes::detect(&r);
    let names: Vec<&str> = rep.ram.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, vec!["Timer"]);
    assert_eq!(rep.ram[0].addrs, vec![0x30, 0x31]);
    assert_eq!(rep.ram[0].play_changes, 20);
}

/// A run in which code at three places asks for effect 4 through a queue
/// at $F0 that the engine (its writes flagged) clears the next frame, and
/// a note starts each time.
fn sound_run() -> Run {
    let mut r = run(200);
    r.constants = vec![
        Constant {
            name: "Sfx_Ping".into(),
            value: 4,
            near: vec!["SoundQueue".into()],
        },
        // The same value, used elsewhere: not this queue's effect.
        Constant {
            name: "Sfx_Other".into(),
            value: 4,
            near: vec![],
        },
    ];
    r.known.ram.push(RamLabel {
        name: "SoundQueue".into(),
        addr: 0xF0,
        size: 1,
    });
    for (n, at) in [20usize, 60, 100, 140].into_iter().enumerate() {
        let pc = 0x100 + n as u32 % 3;
        r.frames[at].code.as_mut().unwrap().ram.push(RamWrite {
            addr: 0xF0,
            value: 4,
            engine: false,
            pc,
            routine: pc,
        });
        r.frames[at + 1].code.as_mut().unwrap().ram.push(RamWrite {
            addr: 0xF0,
            value: 0,
            engine: true,
            pc: 0x900,
            routine: 0x900,
        });
        r.frames[at + 1].apu.push((0x4003, 0x08));
        // The coin counter goes up after each.
        for f in &mut r.frames[at + 2..] {
            f.counters[0] += 1;
        }
    }
    r
}

#[test]
fn a_sound_queue_is_found_and_its_requests_named() {
    let r = sound_run();
    let (ifaces, reqs) = sound::detect(&r);
    assert_eq!(ifaces.len(), 1);
    assert_eq!(ifaces[0].kind, "queue");
    assert_eq!(ifaces[0].addr, Some(0xF0));
    assert_eq!(reqs.len(), 4);
    assert!(reqs.iter().all(|q| q.name == "Sfx_Ping" && q.id == 4));
    let rep = changes::detect(&r);
    assert_eq!(rep.sound.len(), 1);
    assert_eq!(rep.sound[0].count, 4);
    assert!(
        rep.sound[0].suggests.starts_with("coins +1"),
        "{}",
        rep.sound[0].suggests
    );
}

#[test]
fn a_cited_effect_is_not_a_gap() {
    let mut r = sound_run();
    r.known.cited.insert("Sfx_Ping".into());
    assert!(changes::detect(&r).sound.is_empty());
}

#[test]
fn a_bump_from_below_is_cataloged_with_its_effects() {
    let mut r = run(400);
    // The player's box is the object at its position.
    for f in &mut r.frames {
        f.objects.push(Object {
            c: Cluster {
                x0: 100,
                y0: 100,
                x1: 116,
                y1: 116,
                palette: 0,
                tiles: vec![1],
            },
            sprites: vec![0],
        });
    }
    // Two blocks above the player's head, each hit once and restored 8
    // frames later, with a coin each time.
    for at in [50usize, 250] {
        for (j, f) in r.frames.iter_mut().enumerate() {
            if (at - 2..at).contains(&j) {
                f.ents[0].y = 100 + 2 * (at - j) as i32;
            }
        }
        r.frames[at].cells.push(cell(6, 5, 0x53, 0x24));
        r.frames[at + 8].cells.push(cell(6, 5, 0x24, 0x53));
        for f in &mut r.frames[at + 1..] {
            f.counters[0] += 1;
        }
    }
    let (cat, its) = interactive::catalog(&r);
    assert_eq!(its.iter().filter(|i| i.found_by == "cell").count(), 4);
    let k = cat
        .kinds
        .iter()
        .find(|k| k.kind == "cell 53535353 -> 24242424")
        .expect("the bump kind");
    assert_eq!(k.count, 2);
    assert_eq!(k.trigger.relation, "below");
    assert_eq!(k.effects.counters["coins"], (1, 2));
    assert_eq!(k.result.reverted, 2);
    assert_eq!(k.result.revert_frames, 8.0);
    assert_eq!(k.limits.max_hits, 2);
}

#[test]
fn a_constant_vertical_offset_is_systematic() {
    let mut r = run(30);
    for f in &mut r.frames {
        f.ents.push(Ent {
            kind: Some(1),
            player: false,
            slot: Some(0),
            id: Some(6),
            x: 40,
            y: 200,
        });
        // The original draws the walker 9 pixels lower than the model.
        f.objects.push(Object {
            c: Cluster {
                x0: 40,
                y0: 209,
                x1: 56,
                y1: 225,
                palette: 1,
                tiles: vec![2],
            },
            sprites: vec![1],
        });
    }
    let boxes = [
        None,
        Some(drawn::ModelBox {
            body: [16, 16],
            picture: [16, 16],
            origin: None,
        }),
    ];
    let rep = drawn::check(&r, &boxes);
    assert_eq!(rep.kinds.len(), 1);
    let k = &rep.kinds[0];
    assert_eq!(k.kind, "walker");
    assert_eq!(k.dy, [-9.0, 0.0]);
    assert_eq!(k.systematic, vec!["dy -9".to_owned()]);
    assert_eq!(rep.off(), 1);
    assert_eq!(
        (k.offset, k.offset_share, k.composite_frames),
        ([0.0, -9.0], 1.0, 0)
    );
    // A second walker beside it on some frames: the object is near both
    // boxes, so those frames don't measure the offset.
    let mut two = r.clone();
    for f in two.frames.iter_mut().take(10) {
        f.ents.push(Ent {
            kind: Some(1),
            player: false,
            slot: Some(1),
            id: Some(6),
            x: 48,
            y: 204,
        });
    }
    let rep = drawn::check(&two, &boxes);
    let k = &rep.kinds[0];
    assert_eq!(k.composite_frames, 10);
    assert_eq!(k.offset, [0.0, -9.0]);
}

#[test]
fn a_captured_frame_is_drawn_from_its_origin_and_measured_where_it_stands() {
    let mut r = run(30);
    for f in &mut r.frames {
        f.ents.push(Ent {
            kind: Some(1),
            player: false,
            slot: Some(0),
            id: Some(6),
            x: 40,
            y: 184,
        });
        // The original's frame is three rows with a blank top row, drawn
        // a line below its sprite table's y: 24 tall from y + 1, its art
        // in the bottom 16.
        f.objects.push(Object {
            c: Cluster {
                x0: 40,
                y0: 185,
                x1: 56,
                y1: 209,
                palette: 1,
                tiles: vec![0, 2],
            },
            sprites: vec![1],
        });
    }
    // A captured frame of the art, 9 below the position.
    let captured = [
        None,
        Some(drawn::ModelBox {
            body: [16, 16],
            picture: [16, 16],
            origin: Some([0, 9]),
        }),
    ];
    let k = &drawn::check(&r, &captured).kinds[0];
    assert_eq!(
        (k.offset, k.systematic.clone()),
        ([0.0, 0.0], vec!["dh -8".to_owned()])
    );
    // The same art stood on the body's bottom edge stands 9 above it.
    let standing = [
        None,
        Some(drawn::ModelBox {
            body: [16, 16],
            picture: [16, 16],
            origin: None,
        }),
    ];
    let k = &drawn::check(&r, &standing).kinds[0];
    assert_eq!(k.offset, [0.0, -9.0]);
}

#[test]
fn routines_that_run_and_nothing_cites_are_unexplained_code() {
    let mut r = run(10);
    r.known.rom = vec![
        RomLabel {
            name: "Cited".into(),
            prg: 0,
            addr: 0x8000,
            size: 16,
        },
        RomLabel {
            name: "Hidden".into(),
            prg: 16,
            addr: 0x8010,
            size: 16,
        },
    ];
    r.known.cited.insert("Cited".into());
    for f in &mut r.frames {
        let c = f.code.as_mut().unwrap();
        c.entered = vec![(0, 1), (16, 1)];
        c.ram.push(RamWrite {
            addr: 0x40,
            value: 1,
            engine: false,
            pc: 18,
            routine: 16,
        });
        f.ram_changed.push(0x40);
    }
    let rep = changes::detect(&r);
    assert_eq!(rep.code.len(), 1);
    assert_eq!(rep.code[0].name, "Hidden");
    assert_eq!(rep.code[0].frames, 10);
    assert_eq!(rep.code[0].caused, 10);
}

#[test]
fn a_routines_untaken_branch_is_unexercised() {
    let mut r = run(1);
    // LDA #1; CMP #3; BEQ +1; RTS; RTS (the target of the branch).
    r.prg = vec![0xA9, 0x01, 0xC9, 0x03, 0xF0, 0x01, 0x60, 0x60];
    r.prg.resize(0x8000, 0);
    r.executed = vec![0; 0x8000 / 64];
    for o in [0usize, 2, 4, 6] {
        r.executed[o / 64] |= 1 << (o % 64);
    }
    let l = RomLabel {
        name: "Check".into(),
        prg: 0,
        addr: 0x8000,
        size: 8,
    };
    r.known.rom = vec![l.clone()];
    let s = code::scan(&r, &l).unwrap();
    assert_eq!(s.unexercised, vec!["Check+7".to_owned()]);
    assert_eq!(s.compares, vec![3]);
}

#[test]
fn a_play_sound_routine_is_found_by_what_it_feeds() {
    let mut r = run(100);
    let push = 0x3C051;
    r.known.rom.push(RomLabel {
        name: "queue_sound".into(),
        prg: push,
        addr: 0xC051,
        size: 11,
    });
    r.watched = vec![push];
    r.calls = vec![CallStat {
        target: push,
        calls: 6,
        callers: 4,
        distinct_a: 3,
        reaches_apu: false,
        engine: false,
    }];
    // Each caller loads A with the ID just before the call.
    r.prg = vec![0xEA; 0x8000];
    for caller in [0x100usize, 0x105, 0x10A, 0x10F] {
        r.prg[caller - 2] = 0xA9;
    }
    for (n, at) in [10usize, 25, 40, 55, 70, 85].into_iter().enumerate() {
        let c = r.frames[at].code.as_mut().unwrap();
        let id = [0x26, 0x2f, 0x3a][n % 3];
        c.calls.push((push, 0x100 + 5 * (n as u32 % 4), id, false));
        // It puts the ID in a buffer that code elsewhere takes the next
        // frame.
        c.ram.push(RamWrite {
            addr: 0x580,
            value: id,
            engine: false,
            pc: push + 4,
            routine: push,
        });
        r.frames[at + 1].code.as_mut().unwrap().ram.push(RamWrite {
            addr: 0x580,
            value: 0,
            engine: false,
            pc: 0x3E000,
            routine: 0x3E000,
        });
    }
    let (ifaces, reqs) = sound::detect(&r);
    assert_eq!(ifaces.len(), 1, "{ifaces:?}");
    assert_eq!(ifaces[0].kind, "routine");
    assert_eq!(reqs.len(), 6);
    assert_eq!(reqs[0].id, 0x26);
    assert_eq!(reqs[0].name, "queue_sound 0x26");
    // One whose writes don't carry the ID asks for no ID of its own.
    let mut fixed = r.clone();
    for f in &mut fixed.frames {
        for w in &mut f.code.as_mut().unwrap().ram {
            if w.value != 0 {
                w.value = 0x40;
            }
        }
    }
    assert!(sound::detect(&fixed).0.is_empty());
    // A routine the engine calls too is a utility.
    for f in &mut r.frames {
        for c in &mut f.code.as_mut().unwrap().calls {
            c.3 = true;
        }
    }
    assert!(sound::detect(&r).0.is_empty());
}

#[test]
fn a_focus_window_keeps_what_happened_in_it_and_ranks_by_rarity() {
    let mut r = run(200);
    r.frames[20].cells.push(cell(3, 6, 1, 2));
    r.frames[100].cells.push(cell(4, 6, 1, 2));
    for (i, f) in r.frames.iter_mut().enumerate() {
        let c = f.code.as_mut().unwrap();
        // Every frame: never rare.
        c.entered.push((0x10, 1));
        if i == 101 {
            // Only in the window: the block's handler.
            c.entered.push((0x1234, 1));
            f.ram_changed.push(0x42);
        }
        if i >= 102 {
            f.counters = vec![1];
        }
        // RAM that changes on every frame isn't rare.
        f.ram_changed.push(0x50);
    }
    // A sprite object far from the player in the window, and one on him.
    let obj = |x: i16, y: i16| Object {
        c: Cluster {
            x0: x,
            y0: y,
            x1: x + 8,
            y1: y + 8,
            palette: 1,
            tiles: vec![0x84],
        },
        sprites: vec![0],
    };
    r.frames[103].objects = vec![obj(10, 10), obj(98, 98)];
    let (cat, its) = interactive::catalog(&r);
    let (_, reqs) = sound::detect(&r);
    let found = focus::Found {
        catalog: &cat,
        interactions: &its,
        requests: &reqs,
    };
    let f = focus::window(&r, &found, 95, 110);
    assert_eq!((f.from, f.to, f.play_frames), (95, 110, 16));
    assert_eq!(f.cells.len(), 1);
    assert_eq!(f.cells[0].samples[0][0], 100);
    let kinds: Vec<&str> = f.interactions.iter().map(|h| h.kind.as_str()).collect();
    assert_eq!(kinds, vec!["cell 01010101 -> 02020202"]);
    assert_eq!(f.interactions[0].kind_count, 2);
    let routines: Vec<&str> = f.routines.iter().map(|x| x.name.as_str()).collect();
    assert_eq!(routines, vec!["sub_01234"]);
    assert_eq!(f.routines[0].elsewhere, 0.0);
    let ram: Vec<&str> = f.ram.iter().map(|x| x.name.as_str()).collect();
    assert_eq!(ram, vec!["$004x"]);
    assert_eq!(
        f.counters,
        vec![focus::Counter {
            name: "coins".into(),
            before: 0,
            after: 1
        }]
    );
    assert_eq!(f.sprites.len(), 1);
    assert_eq!(f.sprites[0].first, 103);
    assert!(
        f.lines()
            .iter()
            .any(|l| l.contains("counter coins 0 -> 1 (+1)"))
    );

    // A window before anything happened finds nothing.
    let quiet = focus::window(&r, &found, 40, 60);
    assert!(quiet.cells.is_empty() && quiet.routines.is_empty() && quiet.counters.is_empty());
}

#[test]
fn the_reference_sets_parse_and_name_their_interfaces() {
    for t in ["mosshop"] {
        let truth = score::Truth::parse(score::builtin(t).unwrap()).unwrap();
        assert_eq!(truth.template, t);
        for e in &truth.sound.effects {
            assert!(truth.sound.interfaces.contains(&e.via), "{t}: {}", e.name);
        }
        for el in &truth.elements {
            for s in &el.sounds {
                assert!(
                    truth.sound.effects.iter().any(|e| &e.name == s),
                    "{t}: {s} of {}",
                    el.name
                );
            }
        }
    }
}

#[test]
fn scoring_counts_found_items_against_the_set() {
    // A made-up game: one queue at $00F0 with two effects, one element.
    let truth = score::Truth::parse(
        r#"
schema = 0
template = "made-up"
[sound]
interfaces = ["SfxQueue"]
effects = [
  { via = "SfxQueue", id = 1, name = "Sfx_Ping" },
  { via = "SfxQueue", id = 2, name = "Sfx_Pong" },
]
[[element]]
name = "bell"
sounds = ["Sfx_Ping"]
[ram]
matter = ["BellTimer"]
bookkeeping = ["Frame"]
"#,
    )
    .unwrap();
    let mut r = run(40);
    r.known.ram = vec![RamLabel {
        name: "SfxQueue".into(),
        addr: 0xF0,
        size: 1,
    }];
    for i in [5usize, 15, 25] {
        r.frames[i].code.as_mut().unwrap().ram.push(RamWrite {
            addr: 0xF0,
            value: 1,
            engine: false,
            pc: 0x100,
            routine: 0x100,
        });
    }
    let changes = changes::Report {
        ram: vec![
            changes::RamItem {
                name: "Frame".into(),
                addrs: vec![1],
                play_changes: 40,
                changes: 40,
                play_share: 1.0,
                input_lift: 1.0,
                labelled: true,
                score: 0.0,
                event_lift: 1.0,
                per_frame: 0.0,
                engine_share: 0.0,
                writers: vec![],
                samples: vec![],
            },
            changes::RamItem {
                name: "BellTimer".into(),
                addrs: vec![2],
                play_changes: 3,
                changes: 3,
                play_share: 1.0,
                input_lift: 1.0,
                labelled: true,
                score: 0.0,
                event_lift: 1.0,
                per_frame: 0.0,
                engine_share: 0.0,
                writers: vec![],
                samples: vec![],
            },
        ],
        ..changes::Report::default()
    };
    let s = score::score(&r, &truth, &changes, &interactive::Catalog::default());
    let ram = s.get("ram").unwrap();
    assert_eq!((ram.true_positives, ram.false_positives), (1, 1));
    assert_eq!(ram.recall, 1.0);
    assert_eq!(s.ram_bookkeeping, 1);
    // The queue was written with Sfx_Ping three times: exercised, but the
    // sound detector found no interface here, so recall is 0.
    let sound = s.get("sound").unwrap();
    assert_eq!((sound.exercised, sound.recall), (1, 0.0));
    let el = s.get("interactive").unwrap();
    assert_eq!((el.exercised, el.recall), (1, 0.0));
}

#[test]
fn ram_ranks_bytes_that_change_with_events_above_bookkeeping() {
    let mut r = run(400);
    r.known.ram = ["Clock", "Bell", "EngineNote", "Noise"]
        .iter()
        .enumerate()
        .map(|(i, n)| RamLabel {
            name: (*n).into(),
            addr: 0x40 + i as u16 * 0x10,
            size: 1,
        })
        .collect();
    for i in 0..400usize {
        let f = &mut r.frames[i];
        // A clock that ticks every frame.
        f.ram_changed.push(0x40);
        // Every 20 frames the coin counter goes up, and the bell byte
        // changes with it.
        if i % 20 == 10 {
            f.counters = vec![(i / 20) as i64 + 1];
            f.ram_changed.push(0x50);
            // The engine's note byte too, but the engine writes it.
            f.ram_changed.push(0x60);
            f.code.as_mut().unwrap().ram.push(RamWrite {
                addr: 0x60,
                value: 1,
                engine: true,
                pc: 0x200,
                routine: 0x200,
            });
        } else {
            f.counters = vec![(i / 20) as i64 + i64::from(i % 20 > 10)];
        }
        // A byte that changes now and then, at random to the events.
        if i % 7 == 3 {
            f.ram_changed.push(0x70);
        }
    }
    let rep = changes::detect(&r);
    let names: Vec<&str> = rep.ram.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names[0], "Bell", "{names:?}");
    let pos = |n: &str| names.iter().position(|x| *x == n).unwrap();
    assert!(pos("Clock") > pos("Noise"), "{names:?}");
    assert!(pos("EngineNote") > pos("Bell"), "{names:?}");
}

#[test]
fn a_box_question_is_answered_by_the_cell_in_the_box_nearest_in_time() {
    let hit = |frame: u64, x: i32, found_by: &str| focus::Hit {
        kind: format!("k{frame}"),
        kind_count: 1,
        interaction: interactive::Interaction {
            frame,
            at: [x, 64],
            before: None,
            after: None,
            relation: "below".into(),
            player_v: [0, 0],
            counters: Default::default(),
            spawned: vec![],
            sound: vec![],
            requests: vec![],
            found_by: found_by.into(),
            routines: vec![],
            primary: None,
            requester: None,
            ram: vec![],
            reverted_after: None,
        },
    };
    let f = focus::Focus {
        from: 780,
        to: 900,
        play_frames: 120,
        // A ? block beside the box, the brick in it twice, a used block
        // beside it, and a sound with no cell.
        interactions: vec![
            hit(859, 112, "cell"),
            hit(840, 128, "cell"),
            hit(870, 128, "cell"),
            hit(873, 144, "cell"),
            hit(869, 128, "sound"),
        ],
        cells: vec![],
        requests: vec![],
        counters: vec![],
        sprites: vec![],
        routines: vec![],
        ram: vec![],
        has_code: true,
    };
    let a = focus::attribute(&f, Some([128, 64, 16, 16]), 16, 868);
    let frames: Vec<u64> = a.answer.iter().map(|h| h.interaction.frame).collect();
    assert_eq!(frames, vec![870]);
    let near: Vec<u64> = a.nearby.iter().map(|h| h.interaction.frame).collect();
    assert_eq!(near, vec![869, 873, 859, 840]);
}

#[test]
fn cells_nothing_touched_are_scenery_not_interactive_kinds() {
    let mut r = run(60);
    for i in [10usize, 30, 50] {
        // Under the player (at 100, 100): touched.
        r.frames[i].cells.push(cell(6, 6, 1, 2));
        // Far from the player, nothing moves with it: an animation.
        r.frames[i + 2].cells.push(cell(1, 10, 3, 4));
    }
    let (cat, _) = interactive::catalog(&r);
    let kinds: Vec<&str> = cat.kinds.iter().map(|k| k.kind.as_str()).collect();
    let scenery: Vec<&str> = cat.scenery.iter().map(|k| k.kind.as_str()).collect();
    assert_eq!(kinds, vec!["cell 01010101 -> 02020202"]);
    assert_eq!(scenery, vec!["cell 03030303 -> 04040404"]);
}

#[test]
fn effect_ids_are_named_by_the_pointer_table_the_engine_reads() {
    // A made-up 32KB PRG: the engine at $8000 writes the APU and reads a
    // table at $8100 with `lda $8100,y`; entry n points at $9000 + 16n,
    // where a data label sits for each ID.
    let mut r = run(20);
    let mut prg = vec![0xEA; 0x8000];
    prg[0x10..0x13].copy_from_slice(&[0xB9, 0x00, 0x81]);
    for id in 0..12u16 {
        let p = 0x9000 + 16 * id;
        prg[0x100 + 2 * id as usize..0x102 + 2 * id as usize].copy_from_slice(&p.to_le_bytes());
        r.known.data.push(RomLabel {
            name: format!("snd_{id}"),
            prg: u32::from(p - 0x8000),
            addr: p,
            size: 16,
        });
    }
    r.prg = prg;
    for f in &mut r.frames {
        f.code.as_mut().unwrap().apu_pc = vec![0x20];
    }
    let engine = sound::Engine::of(&r);
    let ids: std::collections::BTreeSet<u8> = (1..11).collect();
    let names = sound::pointer_table_names(&r, &engine, &ids);
    assert_eq!(names.get(&3).map(String::as_str), Some("snd_3"));
    assert_eq!(names.len(), 10);
    // A queue of flags indexes no table.
    let flags: std::collections::BTreeSet<u8> = (0..8).map(|b| 1u8 << b).collect();
    assert!(sound::pointer_table_names(&r, &engine, &flags).is_empty());
}

#[test]
fn screens_where_the_player_code_doesnt_run_are_not_play() {
    let mut r = run(1000);
    for (i, f) in r.frames.iter_mut().enumerate() {
        let c = f.code.as_mut().unwrap();
        if i < 600 {
            // Play: the player's code moves it and writes its x.
            f.ents[0].x = 100 + (i as i32 % 50);
            c.entered.push((0x500, 1));
            c.ram.push(RamWrite {
                addr: 0x86,
                value: i as u8,
                engine: false,
                pc: 0x510,
                routine: 0x500,
            });
        } else {
            // A title screen borrows the byte now and then, and the mapped
            // x moves with it.
            f.ents[0].x = 100 + (i as i32 % 7);
            if i % 10 == 0 {
                c.ram.push(RamWrite {
                    addr: 0x86,
                    value: 1,
                    engine: false,
                    pc: 0x910,
                    routine: 0x900,
                });
            }
        }
    }
    let out = r.refine_play(&[0x86]);
    assert_eq!(r.play_frames(), 630, "took {out}");
    assert!(!r.frames[700].play && r.frames[100].play);
}
